//! OmO Native 的 provider 管理。
//!
//! 数据面：`<agentDir>/models.json` 的 `providers` 记录 + `<agentDir>/auth.json` 的密钥。
//!
//! `models.json` **没有对外发布的 schema**，字段形状反推自上游
//! `packages/omo-native/bin/lib/setup-opencode-providers.js` 的 `convertProvider`：
//! provider 条目含 `baseUrl` / `api` / `headers` / `models[]`，模型条目含
//! `id` / `name` / `reasoning` / `input` / `contextWindow` / `maxTokens` / `upstreamModelId`。
//! 因此写入一律按 provider key 局部更新，保留其他 provider 与未知字段。

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::Emitter;

use super::constants;
use super::types::{OmoNativeMcpServer, OmoNativeProvider, OmoNativeProviderInput, OmoNativeSkill};
use crate::db::SqliteDbState;

// ============================================================================
// 文件路径
// ============================================================================

async fn agent_dir(db: &SqliteDbState) -> Result<std::path::PathBuf, String> {
    let location =
        crate::coding::runtime_location::get_omo_native_runtime_location_async(db).await?;
    Ok(location.host_path.clone())
}

/// `commands` 侧读 `auth.json` 用的同一份目录决议（见 `read_auth_provider_keys`）。
pub(super) async fn agent_dir_for(db: &SqliteDbState) -> Result<std::path::PathBuf, String> {
    agent_dir(db).await
}

/// The settings file the engine itself reads from `agent_dir`: upstream
/// (`setup-model-choices-import.js`) prefers `settings.jsonc` when it exists
/// and falls back to `settings.json`. Every read/write of the default-model
/// keys must go through this so the app and the engine never edit two
/// different files.
pub fn resolve_omo_native_settings_file_name(agent_dir: &Path) -> &'static str {
    if agent_dir
        .join(constants::OMO_NATIVE_SETTINGS_FILE_FALLBACK)
        .exists()
    {
        constants::OMO_NATIVE_SETTINGS_FILE_FALLBACK
    } else {
        constants::OMO_NATIVE_SETTINGS_FILE
    }
}

fn read_json_object(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let parsed: Value = json5::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    Ok(Some(parsed))
}

pub(super) fn write_json_object(path: &Path, value: &Value, private: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| format!("Failed to serialize {}: {}", path.display(), e))?;
    fs::write(path, format!("{}\n", text))
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    // auth.json 含密钥，收紧到 0600（与上游 auth-store.js 一致）。Windows 上该调用无实际作用。
    if private {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
    }
    Ok(())
}

/// 引擎把 `auth.json` 的值当「config value」解析：`$NAME` / `${NAME}` 插值、
/// `!cmd` 执行 shell、`$$` / `$!` 是字面量转义。写入用户密钥时必须先转义，
/// 否则含 `$` 的 key 会被引擎当变量展开。
///
/// 对应上游 `packages/omo-native/bin/lib/auth-store.js` 的 `literalConfigValue`。
pub fn escape_literal_config_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch == '$' || ch == '!' {
            out.push('$');
        }
        out.push(ch);
    }
    out
}

/// `escape_literal_config_value` 的逆运算：`$$` → `$`，`$!` → `!`。
///
/// `auth.json` 里存的是转义后的文本，回填到界面之前必须还原——否则含 `$` 的
/// 密钥会以 `$$` 的样子显示（本应用是配置管理器，界面上必须是被转义前的真值）。
pub fn unescape_literal_config_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            match chars.next() {
                Some(next @ ('$' | '!')) => out.push(next),
                // 孤立的 `$` 不是转义序列，原样保留。
                Some(next) => {
                    out.push('$');
                    out.push(next);
                }
                None => out.push('$'),
            }
            continue;
        }
        out.push(ch);
    }
    out
}

// ============================================================================
// 读取
// ============================================================================

/// 一条 provider 记录的密钥。
///
/// 两个文件都存得下密钥，**谁先看取决于这个 provider 是不是引擎内建**——
/// 每个 provider 的「写入落点」就是它的读取优先处：
/// - **自定义 provider**：`models.json` 的 `apiKey` 优先（本模块的写入位置），
///   `auth.json` 兜底。早先版本把自定义 provider 写进了 `auth.json`，那些记录
///   还在，只看 `models.json` 会把它们显示成「没存过密钥」。
/// - **内建 provider**：`auth.json` 优先——那是 `/login` 的落点、引擎真正读的地方；
///   它的 `models.json` 覆盖条目里的 `apiKey` 只是兜底。
///
/// ⚠️ 引擎**永远**优先 `auth.json`（`omo auth print-api-key` 实测），所以自定义
/// provider 保存时必须清掉同名 `auth.json` 条目，否则旧值会盖住用户刚改的新值。
///
/// 两处的值都是**转义过的 config value**（`$$` / `$!`，见 `escape_literal_config_value`），
/// 回填界面之前要还原。
fn provider_api_key(
    auth: Option<&Value>,
    key: &str,
    config: &Value,
    builtin: bool,
) -> Option<String> {
    let from_auth = auth
        .and_then(|value| value.get(key))
        .and_then(|entry| entry.get("key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(unescape_literal_config_value);
    let from_config = config
        .get("apiKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(unescape_literal_config_value);

    if builtin {
        from_auth.or(from_config)
    } else {
        from_config.or(from_auth)
    }
}

/// 一个 provider 在 `models.json` 条目里的模型 id 列表（没有目录则空）。
///
/// 纯内建 provider（`models.json` 里没有条目）返回空——它们的目录要跑
/// `omo --list-models` 才有，托盘刷新等不起那次探测。
pub fn model_ids_from_config(config: &Value) -> Vec<String> {
    config
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 界面/托盘用的显示名：`models.json` 的 `name`，缺省回落到 key。
pub fn display_name_for(provider: &OmoNativeProvider) -> String {
    provider
        .config
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&provider.key)
        .to_string()
}

/// 列出 provider：内建名单 + `models.json` 自定义 provider + 密钥状态。
#[tauri::command]
pub async fn list_omo_native_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OmoNativeProvider>, String> {
    let db = state.db();
    let dir = agent_dir(&db).await?;
    let models = read_json_object(&dir.join(constants::OMO_NATIVE_MODELS_FILE))?;
    let auth = read_json_object(&dir.join(constants::OMO_NATIVE_AUTH_FILE))?;

    let custom_providers = models
        .as_ref()
        .and_then(|value| value.get("providers"))
        .and_then(Value::as_object);
    let auth_ref = auth.as_ref();

    let mut providers: Vec<OmoNativeProvider> = Vec::new();

    // 纯内建 provider（`models.json` 里没有条目）：只用于「另有 N 个引擎内建」计数，
    // 不出现在自定义列表里，所以它们的相对顺序不影响界面。
    let is_overridden = |key: &str| {
        custom_providers
            .map(|map| map.contains_key(key))
            .unwrap_or(false)
    };
    for key in constants::OMO_NATIVE_BUILTIN_PROVIDERS {
        if is_overridden(key) {
            continue;
        }
        let api_key = provider_api_key(auth_ref, key, &json!({}), true);
        providers.push(OmoNativeProvider {
            key: key.to_string(),
            builtin: true,
            oauth: constants::is_oauth_provider(key),
            config: json!({}),
            has_key: api_key.is_some(),
            api_key,
            custom: false,
        });
    }

    // `models.json` 里的条目（含内建 provider 的覆盖条目）：**保持文件顺序**。
    // 这个顺序就是拖拽结果，也是排序模式里 `custom` 的含义。
    // ⚠️ **不要再按 key 排序**——那会让用户拖完一刷新就跳回字母序，
    // 表现成「拖拽保存了但没生效」（2026-10-07 修）。
    if let Some(map) = custom_providers {
        for (key, config) in map {
            let builtin = constants::is_builtin_provider(key);
            let api_key = provider_api_key(auth_ref, key, config, builtin);
            providers.push(OmoNativeProvider {
                key: key.clone(),
                builtin,
                oauth: constants::is_oauth_provider(key),
                config: config.clone(),
                has_key: api_key.is_some(),
                api_key,
                custom: true,
            });
        }
    }

    Ok(providers)
}

// ============================================================================
// 写入
// ============================================================================

/// 写入（或更新）一个 provider 到 `models.json`，密钥写在该条目的 `apiKey` 上。
///
/// 按 provider key 局部更新，保留其他 provider 与未知字段。
///
/// **密钥归 `models.json` 的 `apiKey`**：`auth.json` 是引擎 `/login` 与内建渠道的
/// 落点，自定义 provider 混进去会让两边看起来是同一类凭据（2026-10-07 用户报的
/// 混淆）。写入的同时会**删掉 `auth.json` 里的同名条目**——引擎优先读 `auth.json`，
/// 留着旧值会一直盖住用户刚改的新值（见 `provider_api_key` 的说明）。
#[tauri::command]
pub async fn save_omo_native_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmoNativeProviderInput,
) -> Result<(), String> {
    let db = state.db();
    let key = input.key.trim();
    if key.is_empty() {
        return Err("Provider key cannot be empty".to_string());
    }
    if !input.config.is_object() {
        return Err("Provider config must be a JSON object".to_string());
    }

    let dir = agent_dir(&db).await?;
    upsert_omo_native_provider_at(&dir, key, input.config, input.api_key.as_deref())?;
    let _ = app.emit("config-changed", "omo_native");
    Ok(())
}

/// Path-based provider upsert shared by the Tauri command and the gateway
/// plugin integration: writes `models.json.providers.<key>` and clears the same
/// key from `auth.json` so the engine's auth-first lookup cannot shadow it.
pub(crate) fn upsert_omo_native_provider_at(
    dir: &Path,
    key: &str,
    config: Value,
    api_key: Option<&str>,
) -> Result<(), String> {
    // --- models.json ---
    let models_path = dir.join(constants::OMO_NATIVE_MODELS_FILE);
    let mut models = read_json_object(&models_path)?.unwrap_or_else(|| json!({}));
    if !models.is_object() {
        models = json!({});
    }
    let providers = models
        .as_object_mut()
        .expect("models is an object")
        .entry("providers".to_string())
        .or_insert_with(|| json!({}));
    if !providers.is_object() {
        *providers = json!({});
    }

    let mut config_to_store = config.clone();
    // 显式传入的 key 优先（空串表示不动）；没传就沿用已有的 `apiKey`
    // ——包括从 `auth.json` 迁移过来的那个。
    let api_key_to_store = match api_key {
        Some(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(_) => None,
        None => provider_api_key(
            read_json_object(&dir.join(constants::OMO_NATIVE_AUTH_FILE))?.as_ref(),
            key,
            &config,
            constants::is_builtin_provider(key),
        ),
    };
    match api_key_to_store {
        Some(api_key) => {
            // 引擎把 `models.json` 的 `apiKey` 当 config value 解析（`docs/models.md`
            // 的 value resolution），含 `$` 的密钥不转义会被当变量展开。
            config_to_store
                .as_object_mut()
                .expect("checked object above")
                .insert(
                    "apiKey".to_string(),
                    json!(escape_literal_config_value(&api_key)),
                );
        }
        // 用户清空了密钥就是要清空它，不能留一个空串当「已保存」。
        None => {
            config_to_store
                .as_object_mut()
                .expect("checked object above")
                .remove("apiKey");
        }
    }
    let wrote_key = config_to_store.get("apiKey").is_some();
    providers
        .as_object_mut()
        .expect("providers is an object")
        .insert(key.to_string(), config_to_store);
    write_json_object(&models_path, &models, false)?;

    // --- auth.json：只在**刚把密钥写进 models.json** 时清同名条目 ---
    //
    // 两件事：
    // 1. **迁移**：本模块早先往 `auth.json` 写的自定义 provider 还留在那儿，
    //    不清掉就会一直出现在「引擎内建渠道」的 auth.json 编辑器里（用户报的混淆）。
    // 2. **防遮蔽**：引擎优先读 `auth.json`，旧值会盖住用户刚改的新值。
    //
    // ⚠️ **内建 provider 不动**：`anthropic` 这类既在内建名单里、又可能被用户在
    // `models.json` 里写覆盖条目。它们在 `auth.json` 的条目是 `/login` 的真实凭据
    // （`type: "api"`），删掉等于注销账号。内建渠道的凭据本来就该在 auth.json 里改。
    let auth_path = dir.join(constants::OMO_NATIVE_AUTH_FILE);
    if wrote_key && !constants::is_builtin_provider(key) {
        if let Some(mut auth) = read_json_object(&auth_path)? {
            let removed = auth
                .as_object_mut()
                .map(|map| map.remove(key).is_some())
                .unwrap_or(false);
            if removed {
                write_json_object(&auth_path, &auth, true)?;
            }
        }
    }
    Ok(())
}

/// Read one `models.json.providers.<key>` entry for in-place model refreshes.
pub(crate) fn read_omo_native_provider_at(dir: &Path, key: &str) -> Result<Option<Value>, String> {
    Ok(
        read_json_object(&dir.join(constants::OMO_NATIVE_MODELS_FILE))?.and_then(|models| {
            models
                .get("providers")
                .and_then(|providers| providers.get(key))
                .cloned()
        }),
    )
}

/// Resolve the engine's agent directory from a plain DB handle.
pub(crate) async fn omo_native_agent_dir_from_db(
    db: &SqliteDbState,
) -> Result<std::path::PathBuf, String> {
    agent_dir(db).await
}

/// 按给定 key 顺序重排 `providers` 对象（纯逻辑，便于单测）。
///
/// `providers` 是 JSON **对象**，靠 `serde_json` 的 `preserve_order` 保序：
/// 重建一遍即是新的顺序。只重排列表里出现的 key；未出现的保持原相对顺序、
/// 追加在后（前端只提交它展示的那些）。
fn reorder_provider_map(map: &mut serde_json::Map<String, Value>, keys: &[String]) {
    // `iter()` 按插入序返回；先取快照再重建，避免边遍历边改。
    let mut remaining: Vec<(String, Value)> = map
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut ordered: Vec<(String, Value)> = Vec::new();
    for key in keys {
        if let Some(index) = remaining.iter().position(|(existing, _)| existing == key) {
            ordered.push(remaining.remove(index));
        }
    }
    ordered.extend(remaining);

    map.clear();
    for (key, value) in ordered {
        map.insert(key, value);
    }
}

/// 按给定 key 顺序重排 `models.json` 里的 provider（拖拽排序的落盘动作）。
#[tauri::command]
pub async fn reorder_omo_native_providers(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    keys: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    let dir = agent_dir(&db).await?;
    let models_path = dir.join(constants::OMO_NATIVE_MODELS_FILE);
    let Some(mut models) = read_json_object(&models_path)? else {
        return Ok(());
    };
    let Some(provider_map) = models.get_mut("providers").and_then(Value::as_object_mut) else {
        return Ok(());
    };

    reorder_provider_map(provider_map, &keys);
    write_json_object(&models_path, &models, false)?;

    let _ = app.emit("config-changed", "omo_native");
    Ok(())
}

/// 删除 `models.json` 里的 provider 条目（可选一并删除 `auth.json` 的密钥）。
#[tauri::command]
pub async fn delete_omo_native_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_key: String,
    remove_key: bool,
) -> Result<(), String> {
    let db = state.db();
    let dir = agent_dir(&db).await?;

    let models_path = dir.join(constants::OMO_NATIVE_MODELS_FILE);
    if let Some(mut models) = read_json_object(&models_path)? {
        if let Some(providers) = models.get_mut("providers").and_then(Value::as_object_mut) {
            providers.remove(&provider_key);
        }
        write_json_object(&models_path, &models, false)?;
    }

    if remove_key {
        let auth_path = dir.join(constants::OMO_NATIVE_AUTH_FILE);
        if let Some(mut auth) = read_json_object(&auth_path)? {
            if let Some(map) = auth.as_object_mut() {
                map.remove(&provider_key);
            }
            write_json_object(&auth_path, &auth, true)?;
        }
    }

    let _ = app.emit("config-changed", "omo_native");
    Ok(())
}

// ============================================================================
// MCP
// ============================================================================

/// 列出 `<agentDir>/mcp.json` 的 `mcpServers`。
#[tauri::command]
pub async fn list_omo_native_mcp_servers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OmoNativeMcpServer>, String> {
    let db = state.db();
    let dir = agent_dir(&db).await?;
    let mcp = read_json_object(&dir.join(constants::OMO_NATIVE_MCP_FILE))?;
    let servers = mcp
        .as_ref()
        .and_then(|value| value.get("mcpServers"))
        .and_then(Value::as_object);

    let mut result: Vec<OmoNativeMcpServer> = servers
        .map(|map| {
            map.iter()
                .map(|(name, config)| OmoNativeMcpServer {
                    name: name.clone(),
                    config: config.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

/// 写入一个 MCP server 到 `<agentDir>/mcp.json`，保留其他 server 与未知字段。
#[tauri::command]
pub async fn save_omo_native_mcp_server(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    name: String,
    config: Value,
) -> Result<(), String> {
    let db = state.db();
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("MCP server name cannot be empty".to_string());
    }
    if !config.is_object() {
        return Err("MCP server config must be a JSON object".to_string());
    }

    let dir = agent_dir(&db).await?;
    let mcp_path = dir.join(constants::OMO_NATIVE_MCP_FILE);
    let mut mcp = read_json_object(&mcp_path)?.unwrap_or_else(|| json!({}));
    if !mcp.is_object() {
        mcp = json!({});
    }
    let servers = mcp
        .as_object_mut()
        .expect("mcp is an object")
        .entry("mcpServers".to_string())
        .or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    servers
        .as_object_mut()
        .expect("servers is an object")
        .insert(trimmed.to_string(), config);

    // mcp.json 的 env/headers 可能含 token，与上游一致收紧权限。
    write_json_object(&mcp_path, &mcp, true)?;
    let _ = app.emit("config-changed", "omo_native");
    Ok(())
}

#[tauri::command]
pub async fn delete_omo_native_mcp_server(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<(), String> {
    let db = state.db();
    let dir = agent_dir(&db).await?;
    let mcp_path = dir.join(constants::OMO_NATIVE_MCP_FILE);
    if let Some(mut mcp) = read_json_object(&mcp_path)? {
        if let Some(servers) = mcp.get_mut("mcpServers").and_then(Value::as_object_mut) {
            servers.remove(&name);
        }
        write_json_object(&mcp_path, &mcp, true)?;
    }
    let _ = app.emit("config-changed", "omo_native");
    Ok(())
}

// ============================================================================
// Skills
// ============================================================================

/// 列出 `<agentDir>/skills/` 下的 skill 目录。
#[tauri::command]
pub async fn list_omo_native_skills(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OmoNativeSkill>, String> {
    let db = state.db();
    let dir = agent_dir(&db).await?.join(constants::OMO_NATIVE_SKILLS_DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries =
        fs::read_dir(&dir).map_err(|e| format!("Failed to read skills directory: {}", e))?;
    let mut skills: Vec<OmoNativeSkill> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        skills.push(OmoNativeSkill {
            description: read_skill_description(&path),
            name,
            path: path.to_string_lossy().to_string(),
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// 从 `SKILL.md` 的 YAML frontmatter 读 description（失败返回 None，不影响列表）。
fn read_skill_description(skill_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(skill_dir.join("SKILL.md")).ok()?;
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_file_prefers_the_engine_jsonc_fallback() {
        // Upstream reads `settings.jsonc` when it exists; without this
        // resolution the app would keep patching `settings.json` while the
        // engine reads a different file.
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            resolve_omo_native_settings_file_name(dir.path()),
            "settings.json",
            "no file on disk yet: writes start in the plain json file"
        );
        std::fs::write(dir.path().join("settings.jsonc"), "{}").expect("write jsonc");
        assert_eq!(
            resolve_omo_native_settings_file_name(dir.path()),
            "settings.jsonc"
        );
    }

    #[test]
    fn escape_doubles_dollar_and_bang() {
        assert_eq!(escape_literal_config_value("sk-abc"), "sk-abc");
        assert_eq!(escape_literal_config_value("a$b"), "a$$b");
        assert_eq!(escape_literal_config_value("a!b"), "a$!b");
        assert_eq!(escape_literal_config_value("${ENV}"), "$${ENV}");
        // 幂等性检查：转义后的字符串再转义会继续翻倍——所以只能转义一次。
        assert_eq!(escape_literal_config_value("a$$b"), "a$$$$b");
    }

    #[test]
    fn escape_leaves_other_chars_untouched() {
        assert_eq!(
            escape_literal_config_value("key-with_underscore.and/slash:colon"),
            "key-with_underscore.and/slash:colon"
        );
    }

    /// 拖拽排序要把 `providers` 重建成给定顺序——键序就是存储顺序。
    fn map_with_keys(keys: &[&str]) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        for key in keys {
            map.insert((*key).to_string(), json!({ "name": key }));
        }
        map
    }

    fn keys_of(map: &serde_json::Map<String, Value>) -> Vec<String> {
        map.keys().cloned().collect()
    }

    #[test]
    fn reorder_applies_the_given_order() {
        let mut map = map_with_keys(&["a", "b", "c"]);

        reorder_provider_map(
            &mut map,
            &["c".to_string(), "a".to_string(), "b".to_string()],
        );

        assert_eq!(keys_of(&map), vec!["c", "a", "b"]);
        // 值跟着键走，不能只搬键名。
        assert_eq!(map["c"]["name"], json!("c"));
    }

    #[test]
    fn reorder_keeps_unlisted_entries_in_order_at_the_end() {
        let mut map = map_with_keys(&["a", "b", "c", "d"]);

        reorder_provider_map(&mut map, &["d".to_string(), "b".to_string()]);

        // 只提到前面的两个；未列出的保持原相对顺序追加在后。
        assert_eq!(keys_of(&map), vec!["d", "b", "a", "c"]);
    }

    #[test]
    fn reorder_ignores_keys_that_are_not_present() {
        let mut map = map_with_keys(&["a", "b"]);

        reorder_provider_map(&mut map, &["ghost".to_string(), "b".to_string()]);

        assert_eq!(keys_of(&map), vec!["b", "a"]);
    }

    #[test]
    fn reorder_with_no_keys_leaves_the_map_untouched() {
        let mut map = map_with_keys(&["a", "b", "c"]);

        reorder_provider_map(&mut map, &[]);

        assert_eq!(keys_of(&map), vec!["a", "b", "c"]);
    }

    /// 自定义 provider 的密钥只写在 `auth.json` 时也要读得出来——本模块早先
    /// 就写在那儿，只看 `models.json` 会把它显示成「没存过密钥」。
    #[test]
    fn custom_credential_falls_back_to_auth_json() {
        let auth = json!({ "shangtang": { "type": "api_key", "key": "sk-from-auth" } });
        assert_eq!(
            provider_api_key(Some(&auth), "shangtang", &json!({}), false).as_deref(),
            Some("sk-from-auth"),
        );
    }

    #[test]
    fn custom_credential_is_read_from_models_json_api_key() {
        assert_eq!(
            provider_api_key(
                None,
                "shangtang",
                &json!({ "apiKey": "sk-from-models" }),
                false
            )
            .as_deref(),
            Some("sk-from-models"),
        );
    }

    /// `models.json` 是自定义 provider 的写入位置，所以它赢；`auth.json` 只作兜底。
    ///
    /// ⚠️ 引擎自己的优先级是反的（`auth.json` 赢），所以保存时必须清掉同名
    /// `auth.json` 条目，见 `save_omo_native_provider`。
    #[test]
    fn custom_models_json_api_key_wins_over_auth_json() {
        let auth = json!({ "p": { "key": "from-auth" } });
        assert_eq!(
            provider_api_key(Some(&auth), "p", &json!({ "apiKey": "from-models" }), false)
                .as_deref(),
            Some("from-models"),
        );
    }

    /// 内建 provider 反过来：`auth.json` 是 `/login` 的落点，引擎真正读的是它。
    #[test]
    fn builtin_credential_prefers_auth_json() {
        let auth = json!({ "anthropic": { "type": "api", "key": "from-login" } });
        assert_eq!(
            provider_api_key(
                Some(&auth),
                "anthropic",
                &json!({ "apiKey": "from-models" }),
                true,
            )
            .as_deref(),
            Some("from-login"),
        );
    }

    #[test]
    fn builtin_credential_falls_back_to_models_json_api_key() {
        assert_eq!(
            provider_api_key(None, "anthropic", &json!({ "apiKey": "from-models" }), true)
                .as_deref(),
            Some("from-models"),
        );
    }

    /// 存的是转义后的 config value，界面要拿到转义前的真值——否则含 `$` 的密钥
    /// 会以 `$$` 的样子显示。
    #[test]
    fn stored_config_values_are_unescaped_before_display() {
        let auth = json!({ "p": { "key": "sk-a$$b" } });
        assert_eq!(
            provider_api_key(Some(&auth), "p", &json!({}), false).as_deref(),
            Some("sk-a$b"),
        );
        assert_eq!(
            provider_api_key(None, "p", &json!({ "apiKey": "sk-$!bang" }), false).as_deref(),
            Some("sk-!bang"),
        );
    }

    #[test]
    fn blank_or_missing_api_key_is_not_a_credential() {
        assert_eq!(provider_api_key(None, "p", &json!({}), false), None);
        assert_eq!(
            provider_api_key(None, "p", &json!({ "apiKey": "" }), false),
            None
        );
        assert_eq!(
            provider_api_key(None, "p", &json!({ "apiKey": "   " }), false),
            None
        );
        // 非字符串的 apiKey 不能当成凭据（引擎也不会读它）。
        assert_eq!(
            provider_api_key(None, "p", &json!({ "apiKey": 42 }), false),
            None
        );
    }

    #[test]
    fn credential_lookup_is_per_provider() {
        let auth = json!({ "other": { "key": "sk-other" } });
        assert_eq!(
            provider_api_key(Some(&auth), "shangtang", &json!({}), false),
            None
        );
    }

    #[test]
    fn unescape_reverses_escape() {
        assert_eq!(unescape_literal_config_value("sk-abc"), "sk-abc");
        assert_eq!(unescape_literal_config_value("a$$b"), "a$b");
        assert_eq!(unescape_literal_config_value("a$!b"), "a!b");
        assert_eq!(unescape_literal_config_value("$${ENV}"), "${ENV}");
        // 孤立的 `$` 不是转义序列，原样保留。
        assert_eq!(unescape_literal_config_value("a$b"), "a$b");
        assert_eq!(unescape_literal_config_value("trailing$"), "trailing$");
        // 与转义互逆（转义是幂等的反面：只能转义一次，所以往返只跑一轮）。
        for raw in ["sk-plain", "a$b", "a!b", "${ENV}", "$$", "!!", "$!"] {
            assert_eq!(
                unescape_literal_config_value(&escape_literal_config_value(raw)),
                raw
            );
        }
    }
}
