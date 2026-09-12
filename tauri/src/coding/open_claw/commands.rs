use std::fs;
use std::path::Path;
use tauri::Emitter;

use super::adapter;
use super::types::*;
use crate::coding::all_api_hub;
use crate::coding::runtime_location;
use crate::coding::skills::commands::resync_all_skills_if_tool_path_changed;
use crate::db::helpers::{db_get, db_put};
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;

// ============================================================================
// Helper Functions
// ============================================================================

/// Get default config path: ~/.openclaw/openclaw.json
pub fn get_default_config_path_for_runtime() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    let config_path = Path::new(&home_dir).join(".openclaw").join("openclaw.json");

    Ok(config_path.to_string_lossy().to_string())
}

fn get_default_config_path() -> Result<String, String> {
    get_default_config_path_for_runtime()
}

/// Internal function to save config and emit events.
///
/// 走 JSON5 round-trip 写引擎:只对发生变化的顶层节做原位替换,
/// 保留其它节的注释与格式;带原子写、写入前备份与外部变更冲突检测。
pub async fn apply_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config: OpenClawConfig,
    from_tray: bool,
) -> Result<(), String> {
    use super::roundtrip;

    // 先读取配置路径(需消费 state),再读取保留数。
    let retain_count = backup_retain_count_from_state(&state);
    let config_path_str = get_openclaw_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
    }

    let app_data_dir = crate::app_paths::resolved_data_dir();
    let _ = app;
    let backup_dir = app_data_dir.join("backups").join("openclaw");

    let mut new_value =
        serde_json::to_value(&config).map_err(|e| format!("Failed to serialize config: {}", e))?;
    roundtrip::migrate_legacy_timeout(&mut new_value);

    let mut document = roundtrip::OpenClawConfigDocument::load(config_path)?;
    let old_value: serde_json::Value = match &document.original_source {
        Some(src) => json5::from_str(src)
            .map_err(|e| format!("Failed to parse current config as JSON5: {}", e))?,
        None => serde_json::json!({}),
    };
    document.apply_root_section_diff(&old_value, &new_value)?;
    let _outcome = document.save(&backup_dir, retain_count)?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("openclaw-config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-openclaw", ());

    Ok(())
}

/// 备份保留数:`settings.auto_backup_max_keep`(0 = 不限),读取失败按不限处理。
fn backup_retain_count_from_state(state: &SqliteDbState) -> usize {
    match crate::settings::store::load_settings_from_sqlite_state(state) {
        Ok(settings) if settings.auto_backup_max_keep > 0 => settings.auto_backup_max_keep as usize,
        _ => usize::MAX,
    }
}

/// Read and parse the config file, returning the OpenClawConfig
async fn read_and_parse_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OpenClawConfig, String> {
    let result = read_openclaw_config(state).await?;
    match result {
        ReadOpenClawConfigResult::Success { config } => Ok(config),
        ReadOpenClawConfigResult::NotFound { path: _ } => {
            // Return empty config for non-existent file
            Ok(OpenClawConfig {
                models: None,
                agents: None,
                env: None,
                tools: None,
                other: serde_json::Map::new(),
            })
        }
        ReadOpenClawConfigResult::ParseError { error, .. } => {
            Err(format!("Config parse error: {}", error))
        }
        ReadOpenClawConfigResult::Error { error } => Err(error),
    }
}

// ============================================================================
// Config Path Commands
// ============================================================================

/// Get OpenClaw config file path with priority: common config > default
#[tauri::command]
pub async fn get_openclaw_config_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    // 1. Check common config for custom path
    if let Some(common_config) = get_openclaw_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(custom_path);
            }
        }
    }

    // 2. Return default path
    get_default_config_path()
}

/// Get OpenClaw config path info including source
#[tauri::command]
pub async fn get_openclaw_config_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OpenClawConfigPathInfo, String> {
    // 1. Check common config for custom path
    if let Some(common_config) = get_openclaw_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(OpenClawConfigPathInfo {
                    path: custom_path,
                    source: "custom".to_string(),
                });
            }
        }
    }

    // 2. Return default path
    let default_path = get_default_config_path()?;
    Ok(OpenClawConfigPathInfo {
        path: default_path,
        source: "default".to_string(),
    })
}

// ============================================================================
// Config Read/Write Commands
// ============================================================================

/// Read OpenClaw configuration file with detailed result
#[tauri::command]
pub async fn read_openclaw_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ReadOpenClawConfigResult, String> {
    let config_path_str = get_openclaw_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Ok(ReadOpenClawConfigResult::NotFound {
            path: config_path_str,
        });
    }

    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            return Ok(ReadOpenClawConfigResult::Error {
                error: format!("Failed to read config file: {}", e),
            });
        }
    };

    match json5::from_str::<OpenClawConfig>(&content) {
        Ok(config) => Ok(ReadOpenClawConfigResult::Success { config }),
        Err(e) => {
            let preview = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };

            Ok(ReadOpenClawConfigResult::ParseError {
                path: config_path_str,
                error: e.to_string(),
                content_preview: Some(preview),
            })
        }
    }
}

/// Save OpenClaw configuration file (full replacement)
#[tauri::command]
pub async fn save_openclaw_config<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    config: OpenClawConfig,
) -> Result<(), String> {
    apply_config_internal(state, &app, config, false).await
}

/// Backup OpenClaw configuration file to app-data `backups/openclaw` dir
#[tauri::command]
pub async fn backup_openclaw_config<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
) -> Result<String, String> {
    let retain_count = backup_retain_count_from_state(&state);
    let config_path_str = get_openclaw_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Err("Config file does not exist".to_string());
    }

    let source = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let app_data_dir = crate::app_paths::resolved_data_dir();
    let _ = app;
    let backup_path = super::roundtrip::create_openclaw_backup(
        &source,
        &app_data_dir.join("backups").join("openclaw"),
        retain_count,
    )?;

    Ok(backup_path.display().to_string())
}

/// Scan OpenClaw config for health warnings (invalid profile, legacy timeout, …)
#[tauri::command]
pub async fn scan_openclaw_config_health(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OpenClawHealthWarning>, String> {
    let config_path_str = get_openclaw_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let mut warnings = super::roundtrip::scan_openclaw_health_from_source(&content);
    for warning in warnings.iter_mut() {
        if warning.code == "config_parse_failed" && warning.path.is_none() {
            warning.path = Some(config_path_str.clone());
        }
    }
    Ok(warnings)
}

// ============================================================================
// Common Config Commands (DB)
// ============================================================================

/// Get OpenClaw common config from database
#[tauri::command]
pub async fn get_openclaw_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<OpenClawCommonConfig>, String> {
    state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::OpenClawCommonConfig, "common")?.map(adapter::from_db_value))
    })
}

/// Save OpenClaw common config to database
#[tauri::command]
pub async fn save_openclaw_common_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config: OpenClawCommonConfig,
) -> Result<(), String> {
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(&db, "openclaw").await;

    let json_data = adapter::to_db_value(&config);
    db.with_conn(|conn| db_put(conn, DbTable::OpenClawCommonConfig, "common", &json_data))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "openclaw").await?;

    resync_all_skills_if_tool_path_changed(app, state.inner(), "openclaw", previous_skills_path)
        .await;

    Ok(())
}

// ============================================================================
// Agents Defaults Commands
// ============================================================================

/// Get agents.defaults from config file
#[tauri::command]
pub async fn get_openclaw_agents_defaults(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<OpenClawAgentsDefaults>, String> {
    let config = read_and_parse_config(state).await?;
    Ok(config.agents.and_then(|a| a.defaults))
}

/// Set agents.defaults in config file (read-modify-write)
#[tauri::command]
pub async fn set_openclaw_agents_defaults<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    defaults: OpenClawAgentsDefaults,
) -> Result<(), String> {
    let mut config = read_and_parse_config(state.clone()).await?;

    // Ensure agents section exists
    let mut agents = config.agents.unwrap_or(OpenClawAgentsSection {
        defaults: None,
        extra: std::collections::HashMap::new(),
    });
    let applied_model_primary = defaults
        .model
        .as_ref()
        .map(|model| model.primary.clone())
        .unwrap_or_default();
    agents.defaults = Some(defaults);
    config.agents = Some(agents);

    apply_config_internal(state.clone(), &app, config, false).await?;
    record_model_last_used(&state, &applied_model_primary);
    Ok(())
}

/// Record the "recently used" marker for the provider part of an OpenClaw
/// `model.primary` value (`providerId/modelId`). Best-effort: failures must
/// never break the config save flow itself.
pub fn record_model_last_used(state: &SqliteDbState, model_primary: &str) {
    let Some((provider_id, _)) = model_primary.rsplit_once('/') else {
        return;
    };
    if provider_id.is_empty() {
        return;
    }
    if let Err(error) =
        crate::settings::provider_list_state::record_provider_last_used_in_sqlite_state(
            state,
            "openclaw",
            provider_id,
        )
    {
        log::warn!("Failed to record provider last-used for openclaw:{provider_id}: {error}");
    }
}

// ============================================================================
// Env Commands
// ============================================================================

/// Get env section from config file
#[tauri::command]
pub async fn get_openclaw_env(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<OpenClawEnvConfig>, String> {
    let config = read_and_parse_config(state).await?;
    Ok(config.env)
}

/// Set env section in config file (read-modify-write)
#[tauri::command]
pub async fn set_openclaw_env<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    env: OpenClawEnvConfig,
) -> Result<(), String> {
    let mut config = read_and_parse_config(state.clone()).await?;
    config.env = Some(env);
    apply_config_internal(state, &app, config, false).await
}

// ============================================================================
// Tools Commands
// ============================================================================

/// Get tools section from config file
#[tauri::command]
pub async fn get_openclaw_tools(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<OpenClawToolsConfig>, String> {
    let config = read_and_parse_config(state).await?;
    Ok(config.tools)
}

/// Set tools section in config file (read-modify-write)
#[tauri::command]
pub async fn set_openclaw_tools<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    tools: OpenClawToolsConfig,
) -> Result<(), String> {
    let mut config = read_and_parse_config(state.clone()).await?;
    config.tools = Some(tools);
    apply_config_internal(state, &app, config, false).await
}

#[tauri::command]
pub async fn list_openclaw_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OpenClawAllApiHubProvidersResult, String> {
    let _ = state;
    let discovery = all_api_hub::list_provider_candidates()?;

    let providers = discovery
        .providers
        .iter()
        .map(|candidate| OpenClawAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            base_url: candidate.base_url.clone(),
            api_protocol: candidate.api_protocol.clone(),
            requires_browser_open: candidate
                .auth_type
                .as_deref()
                .map(|value| value.trim().eq_ignore_ascii_case("cookie"))
                .unwrap_or(false),
            is_disabled: candidate.is_disabled,
            has_api_key: candidate
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            api_key_preview: candidate
                .api_key
                .as_ref()
                .map(|value| all_api_hub::mask_api_key_preview(value)),
            balance_usd: candidate.balance_usd,
            balance_cny: candidate.balance_cny,
            site_name: candidate.site_name.clone(),
            site_type: candidate.site_type.clone(),
            account_label: candidate.account_label.clone(),
            source_profile_name: candidate.source_profile_name.clone(),
            source_extension_id: candidate.source_extension_id.clone(),
            config: all_api_hub::candidate_to_openclaw_provider(candidate),
        })
        .collect();

    Ok(OpenClawAllApiHubProvidersResult {
        found: discovery.found,
        profiles: discovery.profiles,
        providers,
        message: discovery.message,
    })
}

#[tauri::command]
pub async fn resolve_openclaw_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
    request: ResolveOpenClawAllApiHubProvidersRequest,
) -> Result<Vec<OpenClawAllApiHubProvider>, String> {
    let providers =
        all_api_hub::resolve_provider_candidates_with_keys(&state, &request.provider_ids).await?;

    Ok(providers
        .iter()
        .map(|candidate| OpenClawAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            base_url: candidate.base_url.clone(),
            api_protocol: candidate.api_protocol.clone(),
            requires_browser_open: candidate
                .auth_type
                .as_deref()
                .map(|value| value.trim().eq_ignore_ascii_case("cookie"))
                .unwrap_or(false),
            is_disabled: candidate.is_disabled,
            has_api_key: candidate
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            api_key_preview: candidate
                .api_key
                .as_ref()
                .map(|value| all_api_hub::mask_api_key_preview(value)),
            balance_usd: candidate.balance_usd,
            balance_cny: candidate.balance_cny,
            site_name: candidate.site_name.clone(),
            site_type: candidate.site_type.clone(),
            account_label: candidate.account_label.clone(),
            source_profile_name: candidate.source_profile_name.clone(),
            source_extension_id: candidate.source_extension_id.clone(),
            config: all_api_hub::candidate_to_openclaw_provider(candidate),
        })
        .collect())
}

// ============================================================================
// OpenClaw Control UI
// ============================================================================

/// 从 openclaw.json 读取顶层 `gateway.port`(供 Web UI 端口解析;任何读取/解析失败容错为 None)。
async fn read_openclaw_gateway_port(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<u16>, String> {
    let config_path_str = get_openclaw_config_path(state).await?;

    // 该路径可解析为 WSL Direct 的 UNC,不可达时会长时间阻塞 async 运行时;
    // 走 `file_io` 的 spawn_blocking + 超时读取(见 coding/AGENTS.md)。
    // 保持原容错契约:任何读取/解析失败(含超时)都视为无端口可解析。
    let content = match crate::coding::file_io::read_optional_text_file_with_timeout(
        std::path::PathBuf::from(&config_path_str),
        "openclaw gateway port",
    )
    .await
    {
        Ok(Some(c)) => c,
        Ok(None) | Err(_) => return Ok(None),
    };
    let Ok(value) = json5::from_str::<serde_json::Value>(&content) else {
        return Ok(None);
    };

    Ok(value
        .get("gateway")
        .and_then(|gateway| gateway.get("port"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok()))
}

/// 打开 OpenClaw Control UI:解析端口(env > openclaw.json gateway.port > 默认)并探测
/// 在线后用系统浏览器打开。服务离线返回 `Err`,前端据此引导用户启动 gateway。
#[tauri::command]
pub async fn open_openclaw_web_ui(
    state: tauri::State<'_, SqliteDbState>,
    path: Option<String>,
) -> Result<(), String> {
    use super::web_ui;

    let config_port = read_openclaw_gateway_port(state).await?;
    let port = web_ui::resolve_web_port(config_port);
    if !web_ui::probe_web_up(port).await {
        return Err("OpenClaw Web UI (Control UI) 未运行,请先启动 openclaw gateway".to_string());
    }
    web_ui::open_browser(port, path.as_deref())
}

/// 在用户终端里非阻塞启动 `openclaw gateway`(OpenClaw 常驻服务)。
#[tauri::command]
pub async fn launch_openclaw_gateway() -> Result<(), String> {
    use super::web_ui;

    web_ui::launch_gateway_in_terminal()
}
