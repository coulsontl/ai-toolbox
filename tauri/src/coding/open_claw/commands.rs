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

/// Internal function to save config and emit events
pub async fn apply_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config: OpenClawConfig,
    from_tray: bool,
) -> Result<(), String> {
    let config_path_str = get_openclaw_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
    }

    // Serialize with pretty printing
    let json_content = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(config_path, json_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("openclaw-config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-openclaw", ());

    Ok(())
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

/// Backup OpenClaw configuration file
#[tauri::command]
pub async fn backup_openclaw_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    let config_path_str = get_openclaw_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Err("Config file does not exist".to_string());
    }

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_path_str = format!("{}.bak.{}", config_path_str, timestamp);

    fs::copy(config_path, &backup_path_str)
        .map_err(|e| format!("Failed to backup config file: {}", e))?;

    Ok(backup_path_str)
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
    agents.defaults = Some(defaults);
    config.agents = Some(agents);

    apply_config_internal(state, &app, config, false).await
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
