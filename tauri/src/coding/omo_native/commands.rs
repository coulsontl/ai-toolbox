//! OmO Native 命令层。
//!
//! 数据面：
//! - 统一配置 `~/.omo/omo.jsonc` 的 **`[native]` 块**（与插件版的 `[opencode]` 块同文件不同块）
//! - 引擎状态目录 `~/.omo/agent/`：`settings.json` / `models.json` / `auth.json` / `mcp.json`
//!
//! 铁律：只写 `[native]` 块，绝不碰 `[opencode]` 块与共享 base 键；写入必须保留注释。

use chrono::Local;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Emitter;

use super::constants;
use super::providers;
use super::types::{
    OmoNativeAgentsConfig, OmoNativeAgentsConfigContent, OmoNativeAgentsConfigInput,
    OmoNativeBuiltinModel, OmoNativeBuiltinProvider, OmoNativeCliInfo,
    OmoNativeDefaultSelection, OmoNativeModelSettingsInput, OmoNativePathInfo,
    OmoNativeRuntimeConfig, OmoNativeSettingsConfig, OmoNativeSettingsConfigInput,
};
use crate::coding::db_id::db_new_id;
use crate::coding::omo_jsonc_patch::{self, patch_top_level_block, remove_top_level_key};
use crate::coding::runtime_location;
use crate::db::helpers::{db_get, db_list, db_patch_fields, db_put, db_update_applied_status};
use crate::db::schema::{Cli, CliConfigKind, DbTable};
use crate::db::SqliteDbState;

/// 顶层控制键：不属于任何 harness，读取时永不折叠进生效视图。
const OMO_NATIVE_CONTROL_KEYS: [&str; 4] = ["$schema", "_migrations", "legacy_migrations", "profiles"];

// ============================================================================
// 路径与文件读写
// ============================================================================

/// 统一配置文件路径（`~/.omo/omo.jsonc`，回退 `.json`）。
pub(crate) async fn get_omo_native_config_path_async(
    db: &SqliteDbState,
) -> Result<PathBuf, String> {
    let location = runtime_location::get_omo_native_runtime_location_async(db).await?;
    // 引擎状态目录是 `<home>/.omo/agent`，统一配置在它的上一级。
    let config_dir = location
        .host_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| location.host_path.clone());
    Ok(resolve_unified_config_path_in_dir(&config_dir))
}

fn resolve_unified_config_path_in_dir(dir: &Path) -> PathBuf {
    let jsonc = dir.join(constants::OMO_NATIVE_CONFIG_FILE);
    let json = dir.join(constants::OMO_NATIVE_CONFIG_FILE_FALLBACK);
    if jsonc.exists() {
        jsonc
    } else if json.exists() {
        json
    } else {
        jsonc
    }
}

fn read_jsonc_object(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let parsed: Value = json5::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    Ok(Some(parsed))
}

fn read_file_optional(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// 读 `<agentDir>/auth.json` 的原文，**并把每个条目的 `key` 还原成真值**。
///
/// 文件里存的是 config value 语义的转义文本（`$$` / `$!`），编辑器要给人看
/// 转义前的真值——否则含 `$` 的密钥会以 `$$` 的样子显示，用户改了别的字段再
/// 保存又会多转义一层（`save_omo_native_auth_config` 会再转义一次）。
///
/// 解析失败时原样返回：这是给人看的预览文本，不该因为一个坏文件整个读不出来。
fn read_auth_content(agent_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(agent_dir.join(constants::OMO_NATIVE_AUTH_FILE)).ok()?;
    let Ok(mut parsed) = serde_json::from_str::<Value>(&raw) else {
        return Some(raw);
    };
    if let Some(map) = parsed.as_object_mut() {
        for entry in map.values_mut() {
            if let Some(object) = entry.as_object_mut() {
                if let Some(key) = object.get("key").and_then(Value::as_str) {
                    let unescaped = providers::unescape_literal_config_value(key);
                    object.insert("key".to_string(), Value::String(unescaped));
                }
            }
        }
    }
    Some(
        serde_json::to_string_pretty(&parsed)
            .map(|text| format!("{}\n", text))
            .unwrap_or(raw),
    )
}

/// 从 `settings.json` 里取默认模型选择的三个键（引擎 `docs/settings.md`）。
fn default_selection_from_settings(settings: &Value) -> OmoNativeDefaultSelection {
    let read = |key: &str| {
        settings
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    OmoNativeDefaultSelection {
        provider_key: read("defaultProvider"),
        model_id: read("defaultModel"),
        thinking_level: read("defaultThinkingLevel"),
    }
}

/// 读 `<agentDir>/settings.json`，不存在或不可解析时返回空对象。
///
/// 与 `models.json` 不同，这份文件**必须容忍 JSONC**（引擎自己也读 `.jsonc` 回退），
/// 所以走 json5 而不是 serde_json。
fn read_settings_object(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| json5::from_str::<Value>(&content).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// 折叠 Native 生效视图：共享 base 键 → `[senpi]`（legacy 拼写）→ `[native]`，后者胜。
///
/// 镜像上游 `packages/omo-config-core/src/loader/resolution.ts` 的
/// `resolveOmoConfigView`（不含 profile 层——本模块不管理 `profiles`）。
fn resolve_native_view(config: &Value) -> Value {
    let Some(obj) = config.as_object() else {
        return json!({});
    };
    let mut merged = Map::new();

    // 1. 共享 base 键（顶层除控制键与所有 harness 块之外的全部键）。
    for (key, value) in obj {
        if OMO_NATIVE_CONTROL_KEYS.contains(&key.as_str()) {
            continue;
        }
        if is_harness_block_key(key) {
            continue;
        }
        merged.insert(key.clone(), value.clone());
    }

    // 2. `[senpi]`（legacy 拼写）先折，`[native]` 后折所以后者胜。
    for block in [
        constants::OMO_NATIVE_HARNESS_BLOCK_LEGACY,
        constants::OMO_NATIVE_HARNESS_BLOCK,
    ] {
        if let Some(layer) = obj.get(block).and_then(Value::as_object) {
            for (key, value) in layer {
                merged.insert(key.clone(), value.clone());
            }
        }
    }

    Value::Object(merged)
}

fn is_harness_block_key(key: &str) -> bool {
    matches!(key, "[opencode]" | "[native]" | "[senpi]" | "[codex]")
}

/// 顶层键是不是「共享键」（不属于控制键、也不属于任何 harness 块）。
fn is_shared_top_level_key(name: &str) -> bool {
    !OMO_NATIVE_CONTROL_KEYS.contains(&name) && !is_harness_block_key(name)
}

fn shared_base_view(config: &Value) -> Value {
    let Some(obj) = config.as_object() else {
        return json!({});
    };
    let mut base = Map::new();
    for (key, value) in obj {
        if !is_shared_top_level_key(key) {
            continue;
        }
        base.insert(key.clone(), value.clone());
    }
    Value::Object(base)
}

// ============================================================================
// 路径信息与设置
// ============================================================================

#[tauri::command]
pub async fn get_omo_native_root_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OmoNativePathInfo, String> {
    let db = state.db();
    let location = runtime_location::get_omo_native_runtime_location_async(&db).await?;
    Ok(OmoNativePathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

#[tauri::command]
pub async fn get_omo_native_settings_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<OmoNativeSettingsConfig>, String> {
    let db = state.db();
    let value = db.with_conn(|conn| db_get(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Settings), "common"))?;
    Ok(value.map(super::adapter::settings_from_db_value))
}

#[tauri::command]
pub async fn save_omo_native_settings_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config: OmoNativeSettingsConfigInput,
) -> Result<(), String> {
    let db = state.db();
    let root_dir = if config.clear_root_dir {
        None
    } else {
        config
            .root_dir
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(str::to_string)
    };

    let value = super::adapter::settings_to_db_value(root_dir.as_deref());
    db.with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Settings), "common", &value))?;

    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "omo_native").await?;

    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());

    Ok(())
}

/// 读取 Native 运行时配置：生效视图 + 各文件原文。
#[tauri::command]
pub async fn read_omo_native_runtime_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OmoNativeRuntimeConfig, String> {
    let db = state.db();
    let location = runtime_location::get_omo_native_runtime_location_async(&db).await?;
    let agent_dir = location.host_path.clone();
    let config_path = get_omo_native_config_path_async(&db).await?;

    let raw_config = read_file_optional(&config_path);
    let (effective, native_block, shared_base, parsed, parse_error) = match read_jsonc_object(
        &config_path,
    ) {
        Ok(Some(config)) => {
            let native_block = config
                .get(constants::OMO_NATIVE_HARNESS_BLOCK)
                .cloned()
                .unwrap_or_else(|| json!({}));
            (
                resolve_native_view(&config),
                native_block,
                shared_base_view(&config),
                true,
                None,
            )
        }
        Ok(None) => (json!({}), json!({}), json!({}), true, None),
        Err(error) => (json!({}), json!({}), json!({}), false, Some(error)),
    };

    let settings_path =
        agent_dir.join(providers::resolve_omo_native_settings_file_name(&agent_dir));
    let model_settings = default_selection_from_settings(&read_settings_object(&settings_path));

    Ok(OmoNativeRuntimeConfig {
        effective,
        native_block,
        shared_base,
        model_settings,
        config_path: config_path.to_string_lossy().to_string(),
        config_content: raw_config,
        parsed,
        parse_error,
        settings_content: read_file_optional(&settings_path),
        models_content: read_file_optional(&agent_dir.join(constants::OMO_NATIVE_MODELS_FILE)),
        mcp_content: read_file_optional(&agent_dir.join(constants::OMO_NATIVE_MCP_FILE)),
        auth_content: read_auth_content(&agent_dir),
    })
}

/// 保存 `<agentDir>/settings.json` 里的默认模型选择。
///
/// **按键局部更新**：只动 `defaultProvider` / `defaultModel` / `defaultThinkingLevel`
/// 三个键，其余（`packages` / `theme` / `modelThinkingLevels` 等引擎自己的状态）
/// 原样保留——整份覆盖会把引擎的运行时状态一起抹掉。
///
/// 每个字段：`Some("非空")` 写入，`Some("")` 删除该键（回落引擎默认），
/// `None` 表示这次不动它。
#[tauri::command]
pub async fn save_omo_native_model_settings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativeModelSettingsInput,
) -> Result<OmoNativeRuntimeConfig, String> {
    let db = state.db();
    save_omo_native_model_settings_internal(&db, &input).await?;
    let _ = app.emit("config-changed", "omo_native");
    read_omo_native_runtime_config(state).await
}

/// 托盘用的版本：接受泛型 `AppHandle<R>`（`#[tauri::command]` 只收具体的
/// `AppHandle`，托盘那边拿到的是 `AppHandle<R>`）。写盘逻辑与命令版共用。
pub async fn save_omo_native_model_settings_from_tray<R: tauri::Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    input: &OmoNativeModelSettingsInput,
) -> Result<(), String> {
    save_omo_native_model_settings_internal(db, input).await?;
    let _ = app.emit("config-changed", "omo_native");
    Ok(())
}

/// 写入 `settings.json` 的默认模型三键（不含事件与回读）。
async fn save_omo_native_model_settings_internal(
    db: &SqliteDbState,
    input: &OmoNativeModelSettingsInput,
) -> Result<(), String> {
    let dir = providers::agent_dir_for(db).await?;
    let settings_path = dir.join(providers::resolve_omo_native_settings_file_name(&dir));
    let mut settings = read_settings_object(&settings_path);
    let settings_object = settings
        .as_object_mut()
        .ok_or_else(|| "settings.json is not a JSON object".to_string())?;

    let mut apply = |key: &str, value: Option<&str>| match value {
        Some(text) if !text.trim().is_empty() => {
            settings_object.insert(key.to_string(), json!(text.trim()));
        }
        Some(_) => {
            settings_object.remove(key);
        }
        None => {}
    };
    apply("defaultProvider", input.default_provider.as_deref());
    apply("defaultModel", input.default_model.as_deref());
    apply("defaultThinkingLevel", input.default_thinking_level.as_deref());

    // settings.json 里的 `packages` 等键不敏感，但同一份文件可能被用户手写过
    // token（引擎没有这个约定），跟随 mcp.json 一起收紧权限不会伤到谁。
    providers::write_json_object(&settings_path, &settings, true)
}

/// 保存「其他配置」：`omo.jsonc` 里**不属于任何 harness 块的顶层共享键**。
///
/// 只改这些键：`[native]` / `[opencode]` 两个块、控制键（`$schema` /
/// `_migrations` / `legacy_migrations` / `profiles`）一律原样保留，注释也保留
/// ——所以这里**不能**整份重写文件，要按顶层键做原地补丁。
///
/// 键的增删：传入里有的键覆盖，原来有、传入里没有的键删除（用户在编辑器里
/// 删掉一个键就是要删掉它）。
#[tauri::command]
pub async fn save_omo_native_other_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config: Value,
) -> Result<(), String> {
    if !config.is_object() {
        return Err("Other config must be a JSON object".to_string());
    }
    let db = state.db();
    let config_path = get_omo_native_config_path_async(&db).await?;
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    if !existing.trim().is_empty() && !matches!(json5::from_str::<Value>(&existing), Ok(Value::Object(_)))
    {
        return Err("omo.jsonc is not a JSON object".to_string());
    }

    let mut next = existing.clone();
    let incoming = config.as_object().expect("checked object above");

    // 先删：原来有、这次没有的共享键。
    let managed_now: Vec<String> = incoming.keys().cloned().collect();
    let existing_shared: Vec<String> = omo_jsonc_patch::top_level_key_names(&next)
        .into_iter()
        .filter(|name| {
            !OMO_NATIVE_CONTROL_KEYS.contains(&name.as_str()) && !is_harness_block_key(name)
        })
        .collect();
    for name in existing_shared {
        if !managed_now.contains(&name) {
            let (patched, _) = remove_top_level_key(&next, &name);
            next = patched;
        }
    }

    // 再写：逐键补丁，保留其余内容与注释。
    for (key, value) in incoming {
        if OMO_NATIVE_CONTROL_KEYS.contains(&key.as_str()) || is_harness_block_key(key) {
            return Err(format!("`{}` is managed elsewhere and cannot be set here", key));
        }
        let value_json = serde_json::to_string(value).map_err(|e| e.to_string())?;
        next = patch_top_level_block(&next, key, &value_json, &[]);
    }

    if next.trim().is_empty() {
        // 空文件没有意义，但也别留个 `{}` 垃圾——没有共享键时直接删文件。
        if config_path.exists() {
            fs::remove_file(&config_path).map_err(|e| format!("Failed to remove omo.jsonc: {}", e))?;
        }
    } else {
        fs::write(&config_path, next).map_err(|e| format!("Failed to write omo.jsonc: {}", e))?;
    }

    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());
    Ok(())
}

/// 保存整份 `<agentDir>/auth.json`。
///
/// 界面上「引擎内建渠道」的 `auth.json` 入口用它——本应用是配置管理器，密钥要能
/// 在应用内**查看并编辑**，而不是只丢一个「用资源管理器打开」的入口
/// （2026-10-07 用户要求）。
///
/// ⚠️ 这是**整份覆盖**，与 provider 弹窗的「按 key 局部更新」不同：用户在编辑器里
/// 删掉一个 provider 就是要删掉它。写入沿用 `auth.json` 的 0600 权限。
///
/// ⚠️ **转义成对**：`auth.json` 的 `key` 是 config value 语义（`$NAME` 插值、
/// `!cmd` 执行 shell、`$$` / `$!` 是字面量转义），所以磁盘上是转义后的文本。
/// 读取端（`read_omo_native_runtime_config` 的 `auth_content`）已经把 `key`
/// 还原成真值给编辑器看，这里必须再转义一次写回——否则每存一次就多一层 `$`。
#[tauri::command]
pub async fn save_omo_native_auth_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config: Value,
) -> Result<(), String> {
    if !config.is_object() {
        return Err("auth.json must be a JSON object".to_string());
    }
    let db = state.db();
    let dir = providers::agent_dir_for(&db).await?;
    let auth_path = dir.join(constants::OMO_NATIVE_AUTH_FILE);

    let escaped = Value::Object(
        config
            .as_object()
            .expect("checked object above")
            .iter()
            .map(|(key, entry)| {
                let mut entry = entry.clone();
                if let Some(object) = entry.as_object_mut() {
                    if let Some(raw) = object.get("key").and_then(Value::as_str) {
                        let escaped = providers::escape_literal_config_value(raw);
                        object.insert("key".to_string(), Value::String(escaped));
                    }
                }
                (key.clone(), entry)
            })
            .collect(),
    );
    providers::write_json_object(&auth_path, &escaped, true)?;

    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());
    Ok(())
}

// ============================================================================
// 二进制检测
// ============================================================================

/// 检测 `omo` 二进制并读取版本。复用 `cli_resolver` 的手动路径覆盖与有界超时探测。
#[tauri::command]
pub async fn get_omo_native_cli_info() -> Result<OmoNativeCliInfo, String> {
    let program = crate::coding::cli_resolver::resolve_local_cli_by_name("omo");
    let Some(program) = program else {
        return Ok(OmoNativeCliInfo {
            found: false,
            path: None,
            version: None,
            version_number: None,
            engine_version: None,
            error: None,
        });
    };

    let path = program.path.display().to_string();
    match crate::coding::cli_resolver::probe_cli_version(&path).await {
        Ok(version) => {
            let (version_number, engine_version) = parse_omo_version(&version);
            Ok(OmoNativeCliInfo {
                found: true,
                path: Some(path),
                version: Some(version),
                version_number,
                engine_version,
                error: None,
            })
        }
        Err(error) => Ok(OmoNativeCliInfo {
            found: true,
            path: Some(path),
            version: None,
            version_number: None,
            engine_version: None,
            error: Some(error),
        }),
    }
}

/// 单个子命令的探测超时。`--list-models` 要枚举全部内建 provider，比 `--version` 慢。
const OMO_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// 跑一次 `omo <args...>` 并取 stdout（有界超时，进程随 drop 结束）。
async fn run_omo_command(program: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let mut command = crate::coding::cli_resolver::build_local_tokio_command(program);
    command.args(args).kill_on_drop(true);
    match tokio::time::timeout(OMO_PROBE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        Ok(Ok(output)) => Err(format!(
            "omo {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Ok(Err(error)) => Err(format!("omo {} failed: {error}", args.join(" "))),
        Err(_) => Err(format!("omo {} timed out", args.join(" "))),
    }
}

/// 解析 `omo --list-models` 的表格输出：`provider  model  context  max-out  thinking  images`。
///
/// 前两列是空格分隔的标识符，后四列是定宽列；用 `split_whitespace` 取前两列、
/// 从行尾倒着取后四列，避免 provider / model 名里的点号被误当分隔。
fn parse_list_models(raw: &str) -> Vec<(String, OmoNativeBuiltinModel)> {
    let parse_count = |token: &str| -> Option<i64> {
        let token = token.trim();
        let (number, multiplier) = match token.chars().last() {
            Some('K') => (&token[..token.len() - 1], 1_000_f64),
            Some('M') => (&token[..token.len() - 1], 1_000_000_f64),
            _ => (token, 1_f64),
        };
        number
            .parse::<f64>()
            .ok()
            .map(|value| (value * multiplier).round() as i64)
    };

    let mut models = Vec::new();
    for line in raw.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // provider + model + 4 个定宽列 = 至少 6 段。
        if fields.len() < 6 {
            continue;
        }
        let provider = fields[0];
        let model_id = fields[1];
        let tail = &fields[fields.len() - 4..];
        models.push((
            provider.to_string(),
            OmoNativeBuiltinModel {
                id: model_id.to_string(),
                context: parse_count(tail[0]),
                output: parse_count(tail[1]),
                thinking: tail[2].eq_ignore_ascii_case("yes"),
                images: tail[3].eq_ignore_ascii_case("yes"),
            },
        ));
    }
    models
}

/// 解析 `omo auth check --provider <p> --json` 的输出。
fn parse_auth_check(raw: &str) -> (bool, Option<String>, Option<String>) {
    let parsed: Value = match serde_json::from_str(raw.trim()) {
        Ok(value) => value,
        Err(_) => return (false, None, None),
    };
    let ready = parsed.get("status").and_then(Value::as_str) == Some("ready");
    let auth_type = parsed
        .get("authType")
        .and_then(Value::as_str)
        .map(str::to_string);
    let reason = parsed
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    (ready, auth_type, reason)
}

/// 同时跑几个 `omo auth check`。引擎只能逐个查，49 个串行要 30 秒以上；
/// 全部一起起会互相拖慢（实测反而更慢），12 路是实测的甜点。
const OMO_AUTH_CHECK_CONCURRENCY: usize = 12;

/// 并发查一批 provider 的凭据状态，只返回**已就绪**的那些。
///
/// ⚠️ **不要加 `--no-refresh`**：引擎对 `cursor-cli-oauth` 这类 provider 在
/// 不刷新时直接报 `provider_not_found`，会把一个真实可用的渠道误判成未配置。
async fn probe_ready_providers(
    program: &std::path::Path,
    candidate_ids: Vec<String>,
) -> Vec<(String, Option<String>)> {
    let mut ready: Vec<(String, Option<String>)> = Vec::new();
    for chunk in candidate_ids.chunks(OMO_AUTH_CHECK_CONCURRENCY) {
        let checks = chunk.iter().map(|provider_id| {
            let provider_id = provider_id.clone();
            async move {
                let output = run_omo_command(
                    program,
                    &["auth", "check", "--provider", &provider_id, "--json"],
                )
                .await
                .unwrap_or_default();
                let (is_ready, auth_type, _) = parse_auth_check(&output);
                (provider_id, is_ready, auth_type)
            }
        });
        for (provider_id, is_ready, auth_type) in futures_util::future::join_all(checks).await {
            if is_ready {
                ready.push((provider_id, auth_type));
            }
        }
    }
    ready
}

/// 读出 `auth.json` 的键。
///
/// 这是候选名单的**第二个来源**：`--list-models` 不列「目录按账号发现」的
/// provider（如原生 `cursor`），但用户可以在 `auth.json` 里配它——那种渠道配好
/// 凭据后会在引擎里生效，不能因为表格里没有就漏掉。
fn read_auth_provider_keys(agent_dir: &std::path::Path) -> Vec<String> {
    let path = agent_dir.join(constants::OMO_NATIVE_AUTH_FILE);
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(Value::Object(map)) = json5::from_str::<Value>(&content) else {
        return Vec::new();
    };
    map.keys().cloned().collect()
}

/// 列出**已配置凭据**的引擎内建 provider（官方认证渠道）及其内建模型。
///
/// 与 OpenCode 的「官方 Auth 认证渠道」同语义：**只有配好凭据的渠道才出现**。
/// 未配置的渠道在本模块里没有任何可做的事，列出来只是噪音。
///
/// 数据全部来自引擎自身，不是我们维护的名单：
/// - 候选 = `omo --list-models` 的 provider ∪ `auth.json` 的键，再过滤到内建；
/// - 就绪 = `omo auth check --provider <p> --json`（**已包含环境变量判定**：
///   只设 `ANTHROPIC_API_KEY` 而没写 `auth.json` 时同样报 ready）。
///
/// 代价：引擎没有批量接口，只能逐个查，实测 12 路并发约 8 秒。所以由页面加载时
/// 调用一次，而不是每次渲染都调。
#[tauri::command]
pub async fn list_omo_native_builtin_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OmoNativeBuiltinProvider>, String> {
    let Some(program) = crate::coding::cli_resolver::resolve_local_cli_by_name("omo") else {
        return Ok(Vec::new());
    };
    let program_path = program.path.clone();

    let list_output = match run_omo_command(&program_path, &["--list-models"]).await {
        Ok(output) => output,
        // 引擎不在或版本不支持时返回空列表，让界面走空态——不要把这个当成页面级错误。
        Err(error) => {
            eprintln!("Failed to list OmO models: {error}");
            return Ok(Vec::new());
        }
    };

    let mut models_by_provider: std::collections::BTreeMap<String, Vec<OmoNativeBuiltinModel>> =
        std::collections::BTreeMap::new();
    for (provider, model) in parse_list_models(&list_output) {
        models_by_provider.entry(provider).or_default().push(model);
    }

    let agent_dir = {
        let db = state.db();
        providers::agent_dir_for(&db).await?
    };
    let mut candidate_ids: Vec<String> = models_by_provider
        .keys()
        .cloned()
        .chain(read_auth_provider_keys(&agent_dir))
        .filter(|key| constants::is_builtin_provider(key))
        .collect();
    candidate_ids.sort();
    candidate_ids.dedup();

    let mut providers: Vec<OmoNativeBuiltinProvider> =
        probe_ready_providers(&program_path, candidate_ids)
            .await
            .into_iter()
            .map(|(key, auth_type)| {
                let mut models = models_by_provider.remove(&key).unwrap_or_default();
                models.sort_by(|left, right| left.id.cmp(&right.id));
                OmoNativeBuiltinProvider {
                    name: key.clone(),
                    oauth: constants::is_oauth_provider(&key),
                    ready: true,
                    auth_type,
                    reason: None,
                    id: key,
                    models,
                }
            })
            .collect();
    providers.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(providers)
}

/// 解析 `omo --version` 的输出。
/// 形如 `omo 5.1.19 (engine: senpi 2026.10.10; scheme nodef)`。
fn parse_omo_version(raw: &str) -> (Option<String>, Option<String>) {
    let version_number = raw
        .strip_prefix("omo ")
        .and_then(|rest| rest.split_whitespace().next())
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let engine_version = raw
        .split("engine:")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .map(str::trim)
        .and_then(|rest| rest.strip_prefix("senpi "))
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    (version_number, engine_version)
}

// ============================================================================
// Agent/Category 方案
// ============================================================================

fn list_agents_configs_from_sqlite(
    db: &SqliteDbState,
) -> Result<Vec<OmoNativeAgentsConfig>, String> {
    let mut configs: Vec<OmoNativeAgentsConfig> = db.with_conn(|conn| {
        Ok(db_list(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), None)?
            .into_iter()
            .map(super::adapter::agents_from_db_value)
            .collect::<Vec<_>>())
    })?;
    configs.sort_by(|a, b| match (a.sort_index, b.sort_index) {
        (Some(ai), Some(bi)) => ai.cmp(&bi),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.cmp(&b.name),
    });
    Ok(configs)
}

fn get_agents_config_from_sqlite(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<OmoNativeAgentsConfig>, String> {
    let value = db.with_conn(|conn| db_get(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), id))?;
    Ok(value.map(super::adapter::agents_from_db_value))
}

#[tauri::command]
pub async fn list_omo_native_agents_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OmoNativeAgentsConfig>, String> {
    let db = state.db();
    let configs = list_agents_configs_from_sqlite(&db)?;
    if configs.is_empty() {
        if let Ok(temp) = load_temp_agents_config_from_file(&db).await {
            return Ok(vec![temp]);
        }
    }
    Ok(configs)
}

/// 数据库为空时，从 `[native]` 块读一份 `__local__` 桥接态。
/// 它只是桥接态，不是可长期引用的真实记录 ID。
async fn load_temp_agents_config_from_file(
    db: &SqliteDbState,
) -> Result<OmoNativeAgentsConfig, String> {
    let config_path = get_omo_native_config_path_async(db).await?;
    let config = read_jsonc_object(&config_path)?.unwrap_or_else(|| json!({}));
    let block = config
        .get(constants::OMO_NATIVE_HARNESS_BLOCK)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let take = |key: &str| block.get(key).cloned();
    let other_fields = collect_other_native_keys(&block);

    Ok(OmoNativeAgentsConfig {
        id: crate::coding::local_bridge::LOCAL_CONFIG_ID.to_string(),
        name: crate::coding::local_bridge::LOCAL_CONFIG_NAME.to_string(),
        is_applied: false,
        is_disabled: false,
        agents: take("agents"),
        categories: take("categories"),
        model_profiles: take("model_profiles"),
        model_profile: block
            .get("model_profile")
            .and_then(Value::as_str)
            .map(str::to_string),
        task: take("task"),
        other_fields,
        sort_index: None,
        created_at: None,
        updated_at: None,
    })
}

/// 方案未接管的 `[native]` 合法键，保留下来避免保存时被吞掉。
fn collect_other_native_keys(block: &Map<String, Value>) -> Option<Value> {
    const MANAGED: [&str; 5] = [
        "agents",
        "categories",
        "model_profiles",
        "model_profile",
        "task",
    ];
    let mut other = Map::new();
    for (key, value) in block {
        if MANAGED.contains(&key.as_str()) {
            continue;
        }
        if !constants::OMO_NATIVE_BLOCK_KEYS.contains(&key.as_str()) {
            continue;
        }
        other.insert(key.clone(), value.clone());
    }
    if other.is_empty() {
        None
    } else {
        Some(Value::Object(other))
    }
}

fn build_agents_config_content(
    input: &OmoNativeAgentsConfigInput,
    now: &str,
) -> OmoNativeAgentsConfigContent {
    OmoNativeAgentsConfigContent {
        name: input.name.clone(),
        is_applied: input.is_applied,
        is_disabled: input.is_disabled,
        agents: input.agents.clone(),
        categories: input.categories.clone(),
        model_profiles: input.model_profiles.clone(),
        model_profile: input.model_profile.clone(),
        task: input.task.clone(),
        other_fields: input.other_fields.clone(),
        sort_index: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

#[tauri::command]
pub async fn create_omo_native_agents_config(
    state: tauri::State<'_, SqliteDbState>,
    config: OmoNativeAgentsConfigInput,
) -> Result<OmoNativeAgentsConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let id = input_or_new_id(&config.id);
    let mut content = build_agents_config_content(&config, &now);
    content.sort_index = Some(next_sort_index(&db)?);
    let value = super::adapter::agents_to_db_value(&content);
    db.with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), &id, &value))?;
    get_agents_config_from_sqlite(&db, &id)?
        .ok_or_else(|| "Failed to read back the created config".to_string())
}

#[tauri::command]
pub async fn update_omo_native_agents_config(
    state: tauri::State<'_, SqliteDbState>,
    config: OmoNativeAgentsConfigInput,
) -> Result<OmoNativeAgentsConfig, String> {
    let db = state.db();
    let id = config
        .id
        .clone()
        .ok_or_else(|| "Config id is required for update".to_string())?;
    let now = Local::now().to_rfc3339();

    let existing = db.with_conn(|conn| db_get(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), &id))?
        .ok_or_else(|| format!("Config '{}' not found", id))?;

    let mut content = build_agents_config_content(&config, &now);
    content.created_at = existing
        .get("created_at")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| now.clone());
    content.sort_index = existing
        .get("sort_index")
        .and_then(Value::as_i64)
        .map(|value| value as i32);

    let value = super::adapter::agents_to_db_value(&content);
    db.with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), &id, &value))?;

    // 已应用的方案改动后要重新落到文件。
    if existing
        .get("is_applied")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        write_agents_config_to_file(&db, &id).await?;
    }

    get_agents_config_from_sqlite(&db, &id)?
        .ok_or_else(|| "Failed to read back the updated config".to_string())
}

#[tauri::command]
pub async fn delete_omo_native_agents_config(
    state: tauri::State<'_, SqliteDbState>,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    db.with_conn(|conn| {
        conn.execute(
            &format!(
                "DELETE FROM {} WHERE id = ?1",
                DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents).name()
            ),
            [&config_id],
        )
        .map(|_| ())
        .map_err(|error| format!("Failed to delete config: {error}"))
    })
}

#[tauri::command]
pub async fn reorder_omo_native_agents_configs(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents),
                id,
                &[("sort_index", json!(index as i32))],
            )
            .map(|_| ())
        })?;
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_omo_native_agents_config_disabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let value = db
        .with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents),
                &config_id,
                &[
                    ("is_disabled", Value::Bool(is_disabled)),
                    ("updated_at", Value::String(now.clone())),
                ],
            )
        })?
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;

    let is_applied = value
        .get("is_applied")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if is_applied {
        if is_disabled {
            // 禁用已应用方案 = 撤回运行目录，不留「已禁用但仍生效」的悬挂状态。
            clear_agents_config_from_file(&db).await?;
            db.with_conn_mut(|conn| {
                db_update_applied_status(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), None, &now)
            })?;
        } else {
            write_agents_config_to_file(&db, &config_id).await?;
        }
    }

    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());
    Ok(())
}

#[tauri::command]
pub async fn apply_omo_native_agents_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    write_agents_config_to_file(&db, &config_id).await?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), Some(&config_id), &now)
    })?;
    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());
    Ok(())
}

#[tauri::command]
pub async fn clear_omo_native_applied_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    clear_agents_config_from_file(&db).await?;
    let now = Local::now().to_rfc3339();
    let _ = config_id;
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), None, &now)
    })?;
    let _ = app.emit("config-changed", "omo_native");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-omo-native", ());
    Ok(())
}

/// 把 `__local__` 桥接态收编进数据库。
#[tauri::command]
pub async fn save_omo_native_local_config(
    state: tauri::State<'_, SqliteDbState>,
    config: OmoNativeAgentsConfigInput,
) -> Result<OmoNativeAgentsConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let id = db_new_id();
    let mut content = build_agents_config_content(&config, &now);
    content.is_applied = false;
    content.sort_index = Some(next_sort_index(&db)?);
    let value = super::adapter::agents_to_db_value(&content);
    db.with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::OmoNative, CliConfigKind::Agents), &id, &value))?;
    get_agents_config_from_sqlite(&db, &id)?
        .ok_or_else(|| "Failed to read back the saved config".to_string())
}

fn input_or_new_id(id: &Option<String>) -> String {
    id.clone()
        .filter(|value| !value.trim().is_empty() && value != crate::coding::local_bridge::LOCAL_CONFIG_ID)
        .unwrap_or_else(db_new_id)
}

fn next_sort_index(db: &SqliteDbState) -> Result<i32, String> {
    let configs = list_agents_configs_from_sqlite(db)?;
    Ok(configs
        .iter()
        .filter_map(|config| config.sort_index)
        .max()
        .map(|value| value + 1)
        .unwrap_or(0))
}

// ============================================================================
// 写入 `[native]` 块
// ============================================================================

/// 把方案的受管键合并成 `[native]` 块的值。
fn build_native_block(config: &OmoNativeAgentsConfig) -> Value {
    let mut block = Map::new();
    if let Some(other) = config.other_fields.as_ref().and_then(Value::as_object) {
        for (key, value) in other {
            if constants::OMO_NATIVE_BLOCK_KEYS.contains(&key.as_str()) {
                block.insert(key.clone(), value.clone());
            }
        }
    }
    for (key, value) in [
        ("agents", config.agents.clone()),
        ("categories", config.categories.clone()),
        ("model_profiles", config.model_profiles.clone()),
        ("task", config.task.clone()),
    ] {
        match value {
            Some(value) => {
                block.insert(key.to_string(), value);
            }
            None => {
                block.remove(key);
            }
        }
    }
    match config.model_profile.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            block.insert("model_profile".to_string(), json!(value));
        }
        _ => {
            block.remove("model_profile");
        }
    }
    Value::Object(block)
}

pub(crate) async fn write_agents_config_to_file(
    db: &SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    let config = get_agents_config_from_sqlite(db, config_id)?
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;
    let block = build_native_block(&config);
    write_native_block_to_config(db, &block).await
}

/// 只移除 `[native]` 块，保留 `[opencode]` 块、共享键与注释。
///
/// 即使删块后只剩控制键（`$schema` / `_migrations`）也**不删文件**：那些控制键属于
/// 插件版（`_migrations` 的标记就是插件版 stamp 的），删掉等于替插件版做决定。
/// 这与 `oh_my_openagent::remove_opencode_block` 的「只剩控制键就删文件」刻意不同。
async fn clear_agents_config_from_file(db: &SqliteDbState) -> Result<(), String> {
    let config_path = get_omo_native_config_path_async(db).await?;
    if !config_path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read omo.jsonc: {}", e))?;
    if json5::from_str::<Value>(&content).is_err() {
        return Ok(());
    }

    let (patched, found) = remove_top_level_key(&content, constants::OMO_NATIVE_HARNESS_BLOCK);
    if !found {
        return Ok(());
    }

    fs::write(&config_path, patched).map_err(|e| format!("Failed to write omo.jsonc: {}", e))?;
    Ok(())
}

/// 把 `[native]` 块写进统一配置，保留其他顶层块与全部注释。
async fn write_native_block_to_config(
    db: &SqliteDbState,
    block: &Value,
) -> Result<(), String> {
    let config_path = get_omo_native_config_path_async(db).await?;
    let block_json = serde_json::to_string_pretty(block)
        .map_err(|e| format!("Failed to serialize [native] block: {}", e))?;

    if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read omo.jsonc: {}", e))?;
        if matches!(json5::from_str::<Value>(&content), Ok(Value::Object(_))) {
            let patched = patch_top_level_block(
                &content,
                constants::OMO_NATIVE_HARNESS_BLOCK,
                &block_json,
                &[],
            );
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;
            }
            fs::write(&config_path, patched)
                .map_err(|e| format!("Failed to write omo.jsonc: {}", e))?;
            return Ok(());
        }
    }

    let top = json!({
        constants::OMO_NATIVE_HARNESS_BLOCK: block,
    });
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }
    let text = serde_json::to_string_pretty(&top)
        .map_err(|e| format!("Failed to serialize omo.jsonc: {}", e))?;
    fs::write(&config_path, format!("{}\n", text))
        .map_err(|e| format!("Failed to write omo.jsonc: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_view_folds_shared_base_then_blocks() {
        let config = json!({
            "agents": { "shared": { "model": "a/b" } },
            "categories": { "shared-only": {} },
            "[opencode]": { "agents": { "sisyphus": {} } },
            "[native]": { "agents": { "explore": {} } },
            "$schema": "x",
            "profiles": {}
        });
        let view = resolve_native_view(&config);
        // 共享 base 的 categories 保留。
        assert!(view["categories"]["shared-only"].is_object());
        // `[native]` 的 agents 覆盖共享 base 的 agents（块胜）。
        assert!(view["agents"]["explore"].is_object());
        assert!(view["agents"]["shared"].is_null());
        // `[opencode]` 的 agents 不得泄漏进 Native 视图。
        assert!(view["agents"]["sisyphus"].is_null());
        // 控制键不进入视图。
        assert!(view.get("$schema").is_none());
        assert!(view.get("profiles").is_none());
    }

    #[test]
    fn legacy_senpi_block_folds_before_native() {
        let config = json!({
            "[senpi]": { "agents": { "from-senpi": {} }, "task": { "a": 1 } },
            "[native]": { "agents": { "from-native": {} } }
        });
        let view = resolve_native_view(&config);
        // 同名块内键：native 胜。
        assert!(view["agents"]["from-native"].is_object());
        assert!(view["agents"]["from-senpi"].is_null());
        // 只有 senpi 设过的键仍然生效。
        assert_eq!(view["task"]["a"], json!(1));
    }

    #[test]
    fn build_native_block_keeps_managed_keys_and_other_fields() {
        let config = OmoNativeAgentsConfig {
            id: "x".into(),
            name: "n".into(),
            is_applied: false,
            is_disabled: false,
            agents: Some(json!({ "explore": { "model": "a/b" } })),
            categories: None,
            model_profiles: Some(json!({ "office": { "models": ["a/b"] } })),
            model_profile: Some("office".into()),
            task: None,
            other_fields: Some(json!({ "memory": { "enabled": true }, "bogus": 1 })),
            sort_index: None,
            created_at: None,
            updated_at: None,
        };
        let block = build_native_block(&config);
        assert!(block["agents"]["explore"].is_object());
        assert_eq!(block["model_profile"], json!("office"));
        // other_fields 里的合法 Native 键保留。
        assert_eq!(block["memory"]["enabled"], json!(true));
        // 非法键被丢弃（写进去会触发上游 unknown-key）。
        assert!(block.get("bogus").is_none());
        // 未设置的受管键不出现。
        assert!(block.get("categories").is_none());
    }

    #[test]
    fn build_native_block_drops_empty_model_profile() {
        let config = OmoNativeAgentsConfig {
            id: "x".into(),
            name: "n".into(),
            is_applied: false,
            is_disabled: false,
            agents: None,
            categories: None,
            model_profiles: None,
            model_profile: Some("   ".into()),
            task: None,
            other_fields: None,
            sort_index: None,
            created_at: None,
            updated_at: None,
        };
        let block = build_native_block(&config);
        assert!(block.get("model_profile").is_none());
    }

    /// 隔离守护：写入 `[native]` 块绝不能扰动同一文件里的 `[opencode]` 块、
    /// 共享键与注释。这是本模块与 opencode tab 的 OMO 区块共存的前提。
    #[test]
    fn writing_native_block_leaves_opencode_block_and_comments_intact() {
        // 结构仿照 omo.jsonc 的实际形状（插件版写入的 [opencode] 块 + 共享键）。
        // 渠道名用占位符：测试数据不得出现开发者的真实配置。
        let raw = r#"{
  "_migrations": ["2026-07-opencode-config-unification","2026-08-reasoning-unification"],
  "$schema": "https://example/omo.schema.json",
  "codegraph": { "daemon": true }, // 共享键，必须原样保留
  "[opencode]": {
    "agents": {
      "sisyphus": { "models": ["example-provider/glm-5.2"] }
    },
    "claude_code": { "commands": true }
  }
}"#;

        let block = json!({
            "agents": { "plan-reviewer": { "model": "example-provider/glm-5.2" } },
            "categories": { "deep-low": { "model": "example-provider/glm-5.2" } }
        });
        let block_json = serde_json::to_string_pretty(&block).unwrap();
        let patched = patch_top_level_block(raw, "[native]", &block_json, &[]);

        // 1) 新增的 [native] 块内容正确。
        let obj: Value = json5::from_str(&patched).unwrap();
        assert_eq!(
            obj["[native]"]["agents"]["plan-reviewer"]["model"],
            json!("example-provider/glm-5.2")
        );
        assert_eq!(
            obj["[native]"]["categories"]["deep-low"]["model"],
            json!("example-provider/glm-5.2")
        );

        // 2) [opencode] 块逐字未变。
        assert_eq!(
            obj["[opencode]"],
            json!({
                "agents": { "sisyphus": { "models": ["example-provider/glm-5.2"] } },
                "claude_code": { "commands": true }
            })
        );

        // 3) 共享键与控制键未变。
        assert_eq!(obj["codegraph"]["daemon"], json!(true));
        assert_eq!(
            obj["_migrations"],
            json!([
                "2026-07-opencode-config-unification",
                "2026-08-reasoning-unification"
            ])
        );
        assert_eq!(obj["$schema"], json!("https://example/omo.schema.json"));

        // 4) 注释保留。
        assert!(patched.contains("// 共享键，必须原样保留"));
    }

    /// 反向守护：清除 `[native]` 块后，`[opencode]` 块与共享键仍在。
    #[test]
    fn clearing_native_block_leaves_opencode_block_intact() {
        let raw = r#"{
  "codegraph": { "daemon": true },
  "[native]": { "agents": {} },
  "[opencode]": { "agents": { "sisyphus": {} } }
}"#;
        let (patched, found) =
            remove_top_level_key(raw, constants::OMO_NATIVE_HARNESS_BLOCK);
        assert!(found);

        let obj: Value = json5::from_str(&patched).unwrap();
        assert!(obj.get(constants::OMO_NATIVE_HARNESS_BLOCK).is_none());
        assert_eq!(obj["[opencode]"]["agents"]["sisyphus"], json!({}));
        assert_eq!(obj["codegraph"]["daemon"], json!(true));
    }

    /// 幂等：连续写两次同一个块，结果一致（不会重复插入或叠加）。
    #[test]
    fn writing_native_block_twice_is_idempotent() {
        let raw = r#"{ "[opencode]": { "plugin": true } }"#;
        let block_json = r#"{ "agents": { "explore": {} } }"#;
        let once = patch_top_level_block(raw, "[native]", block_json, &[]);
        let twice = patch_top_level_block(&once, "[native]", block_json, &[]);
        assert_eq!(once, twice);
        let obj: Value = json5::from_str(&twice).unwrap();
        assert_eq!(obj["[native]"]["agents"]["explore"], json!({}));
        assert_eq!(obj["[opencode]"]["plugin"], json!(true));
    }

    #[test]
    fn parse_omo_version_reads_product_and_engine() {
        let (product, engine) =
            parse_omo_version("omo 5.1.19 (engine: senpi 2026.10.10; scheme nodef)");
        assert_eq!(product.as_deref(), Some("5.1.19"));
        assert_eq!(engine.as_deref(), Some("2026.10.10"));
    }

    #[test]
    fn parse_omo_version_tolerates_unexpected_shapes() {
        let (product, engine) = parse_omo_version("something else entirely");
        assert!(product.is_none());
        assert!(engine.is_none());

        // 只有产品版本、没有 engine 段时也不能炸。
        let (product, engine) = parse_omo_version("omo 5.1.19");
        assert_eq!(product.as_deref(), Some("5.1.19"));
        assert!(engine.is_none());
    }

    #[test]
    fn collect_other_native_keys_keeps_only_legal_block_keys() {
        let mut block = Map::new();
        block.insert("memory".to_string(), json!({}));
        block.insert("telemetry".to_string(), json!({ "enabled": false }));
        block.insert("agents".to_string(), json!({}));
        block.insert("not_a_native_key".to_string(), json!(1));
        let other = collect_other_native_keys(&block).unwrap();
        assert!(other["memory"].is_object());
        assert_eq!(other["telemetry"]["enabled"], json!(false));
        assert!(other.get("agents").is_none());
        assert!(other.get("not_a_native_key").is_none());
    }
}
