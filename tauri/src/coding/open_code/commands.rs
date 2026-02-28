use indexmap::IndexMap;
use std::fs;
use std::path::Path;
use serde_json::Value;
use tauri::Emitter;

use super::adapter;
use super::types::*;
use crate::coding::db_id::db_record_id;
use crate::db::DbState;

// ============================================================================
// Helper Functions
// ============================================================================

// ============================================================================
// OpenCode Commands
// ============================================================================

/// Get OpenCode config file path with priority: common config > system env > shell config > default
#[tauri::command]
pub async fn get_opencode_config_path(state: tauri::State<'_, DbState>) -> Result<String, String> {
    // 1. Check common config (highest priority)
    if let Some(common_config) = get_opencode_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(custom_path);
            }
        }
    }
    
    // 2. Check system environment variable (second priority)
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Ok(env_path);
        }
    }
    
    // 3. Check shell configuration files (third priority)
    if let Some(shell_path) = super::shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            return Ok(shell_path);
        }
    }
    
    // 4. Return default path
    get_default_config_path()
}

/// Get OpenCode config path info including source
#[tauri::command]
pub async fn get_opencode_config_path_info(
    state: tauri::State<'_, DbState>,
) -> Result<ConfigPathInfo, String> {
    // 1. Check common config (highest priority)
    if let Some(common_config) = get_opencode_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(ConfigPathInfo {
                    path: custom_path,
                    source: "custom".to_string(),
                });
            }
        }
    }
    
    // 2. Check system environment variable (second priority)
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Ok(ConfigPathInfo {
                path: env_path,
                source: "env".to_string(),
            });
        }
    }
    
    // 3. Check shell configuration files (third priority)
    if let Some(shell_path) = super::shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            return Ok(ConfigPathInfo {
                path: shell_path,
                source: "shell".to_string(),
            });
        }
    }
    
    // 4. Return default path
    let default_path = get_default_config_path()?;
    Ok(ConfigPathInfo {
        path: default_path,
        source: "default".to_string(),
    })
}

/// Helper function to get default config path
/// Returns the actual config file path (checks .jsonc first, then .json)
pub fn get_default_config_path() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    let config_dir = Path::new(&home_dir).join(".config").join("opencode");

    // Check for .jsonc first, then .json
    let jsonc_path = config_dir.join("opencode.jsonc");
    let json_path = config_dir.join("opencode.json");

    if jsonc_path.exists() {
        Ok(jsonc_path.to_string_lossy().to_string())
    } else if json_path.exists() {
        Ok(json_path.to_string_lossy().to_string())
    } else {
        // Return default path for new file
        Ok(jsonc_path.to_string_lossy().to_string())
    }
}

/// Read OpenCode configuration file with detailed result
#[tauri::command]
pub async fn read_opencode_config(state: tauri::State<'_, DbState>) -> Result<ReadConfigResult, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Ok(ReadConfigResult::NotFound { path: config_path_str });
    }

    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => return Ok(ReadConfigResult::Error { error: format!("Failed to read config file: {}", e) }),
    };

    match json5::from_str::<OpenCodeConfig>(&content) {
        Ok(mut config) => {
            // Initialize provider if missing
            if config.provider.is_none() {
                config.provider = Some(IndexMap::<String, OpenCodeProvider>::new());
            }

            // Fill missing name fields with provider key
            // Fill missing npm fields with smart default based on provider key/name
            if let Some(ref mut providers) = config.provider {
                for (key, provider) in providers.iter_mut() {
                    if provider.name.is_none() {
                        provider.name = Some(key.clone());
                    }
                    if provider.npm.is_none() {
                        // Smart npm inference based on provider key or name (case-insensitive)
                        let key_lower = key.to_lowercase();
                        let name_lower = provider.name.as_ref().map(|n| n.to_lowercase()).unwrap_or_default();

                        let inferred_npm = if key_lower.contains("google") || key_lower.contains("gemini")
                            || name_lower.contains("google") || name_lower.contains("gemini")
                        {
                            "@ai-sdk/google"
                        } else if key_lower.contains("anthropic") || key_lower.contains("claude")
                            || name_lower.contains("anthropic") || name_lower.contains("claude")
                        {
                            "@ai-sdk/anthropic"
                        } else {
                            "@ai-sdk/openai-compatible"
                        };

                        provider.npm = Some(inferred_npm.to_string());
                    }
                }
            }

            Ok(ReadConfigResult::Success { config })
        }
        Err(e) => {
            // Truncate content preview to first 500 chars
            let preview = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };

            Ok(ReadConfigResult::ParseError {
                path: config_path_str,
                error: e.to_string(),
                content_preview: Some(preview),
            })
        }
    }
}

/// Backup OpenCode configuration file by renaming it with .bak.{timestamp} suffix
#[tauri::command]
pub async fn backup_opencode_config(state: tauri::State<'_, DbState>) -> Result<String, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Err("Config file does not exist".to_string());
    }

    // Generate backup path with timestamp
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_path_str = format!("{}.bak.{}", config_path_str, timestamp);
    let backup_path = Path::new(&backup_path_str);

    // Rename the file to backup
    fs::rename(config_path, backup_path)
        .map_err(|e| format!("Failed to backup config file: {}", e))?;

    Ok(backup_path_str.to_string())
}

/// Save OpenCode configuration file
#[tauri::command]
pub async fn save_opencode_config<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle<R>,
    config: OpenCodeConfig,
) -> Result<(), String> {
    apply_config_internal(state, &app, config, false).await
}

/// Internal function to save config and emit events
pub async fn apply_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle<R>,
    config: OpenCodeConfig,
    from_tray: bool,
) -> Result<(), String> {
    let config_path_str = get_opencode_config_path(state).await?;
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

    // Notify based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

// ============================================================================
// OpenCode Common Config Commands
// ============================================================================

/// Get OpenCode common config
#[tauri::command]
pub async fn get_opencode_common_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<OpenCodeCommonConfig>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query opencode common config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(Some(adapter::from_db_value(record.clone())))
            } else {
                Ok(None)
            }
        }
        Err(e) => {
            // 反序列化失败，删除旧数据以修复版本冲突
            eprintln!("⚠️ OpenCode common config has incompatible format, cleaning up: {}", e);
            let _ = db.query("DELETE opencode_common_config:`common`").await;
            Ok(None)
        }
    }
}

/// Save OpenCode common config
#[tauri::command]
pub async fn save_opencode_common_config(
    state: tauri::State<'_, DbState>,
    config: OpenCodeCommonConfig,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let json_data = adapter::to_db_value(&config);

    // Use UPSERT to handle both update and create
    db.query("UPSERT opencode_common_config:`common` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save opencode common config: {}", e))?;

    Ok(())
}

// ============================================================================
// Free Models Commands
// ============================================================================

/// Get OpenCode free models from opencode channel
/// Returns free models where cost.input and cost.output are both 0
#[tauri::command]
pub async fn get_opencode_free_models(
    state: tauri::State<'_, DbState>,
    force_refresh: Option<bool>,
) -> Result<GetFreeModelsResponse, String> {
    let (free_models, from_cache, updated_at) = super::free_models::get_free_models(&state, force_refresh.unwrap_or(false)).await?;
    let total = free_models.len();

    Ok(GetFreeModelsResponse {
        free_models,
        total,
        from_cache,
        updated_at,
    })
}

/// Get provider models data by provider_id
/// Returns the complete model information for a specific provider
#[tauri::command]
pub async fn get_provider_models(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<Option<ProviderModelsData>, String> {
    super::free_models::get_provider_models_internal(&state, &provider_id).await
}

// ============================================================================
// Unified Models Commands
// ============================================================================

/// Get unified model list combining custom providers and official providers from auth.json
/// Returns all available models sorted by display name
#[tauri::command]
pub async fn get_opencode_unified_models(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<UnifiedModelOption>, String> {
    // Read auth.json to get official provider ids
    let auth_channels = super::free_models::read_auth_channels();

    // Read config to get custom providers
    let result = read_opencode_config(state.clone()).await?;
    let custom_providers = match result {
        ReadConfigResult::Success { config } => config.provider,
        _ => None,
    };

    // Get unified model list
    let models = super::free_models::get_unified_models(&state, custom_providers.as_ref(), &auth_channels).await;

    Ok(models)
}

// ============================================================================
// Official Auth Providers Commands
// ============================================================================

/// Get official auth providers data from auth.json
/// Returns providers split into standalone (not in custom config) and merged (models only)
#[tauri::command]
pub async fn get_opencode_auth_providers(
    state: tauri::State<'_, DbState>,
) -> Result<GetAuthProvidersResponse, String> {
    // Read config to get custom providers
    let result = read_opencode_config(state.clone()).await?;
    let custom_providers = match result {
        ReadConfigResult::Success { config } => config.provider,
        _ => None,
    };

    // Get auth providers data
    let response = super::free_models::get_auth_providers_data(&state, custom_providers.as_ref()).await;

    Ok(response)
}

// ============================================================================
// Favorite Plugin Commands
// ============================================================================

/// Default favorite plugins to initialize on first use
const DEFAULT_FAVORITE_PLUGINS: &[&str] = &[
    "oh-my-opencode@latest",
    "oh-my-opencode-slim",
    "opencode-antigravity-auth",
    "opencode-openai-codex-auth",
    "opencode-omit-max-tokens",
    "opencode-axonhub-tracing",
];

/// Initialize default favorite plugins if database is empty
async fn init_default_favorite_plugins(db: &surrealdb::Surreal<surrealdb::engine::local::Db>) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    for plugin_name in DEFAULT_FAVORITE_PLUGINS {
        let record_id = db_record_id("opencode_favorite_plugin", plugin_name);
        let query = format!(
            "INSERT IGNORE INTO opencode_favorite_plugin {{ id: {}, plugin_name: $plugin_name, created_at: $created_at }}",
            record_id
        );
        db.query(&query)
            .bind(("plugin_name", *plugin_name))
            .bind(("created_at", now.clone()))
            .await
            .map_err(|e| format!("Failed to initialize favorite plugin: {}", e))?;
    }

    Ok(())
}

/// List all favorite plugins
/// Auto-initializes default plugins if database is empty
#[tauri::command]
pub async fn list_opencode_favorite_plugins(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<OpenCodeFavoritePlugin>, String> {
    let db = state.0.lock().await;

    // Check if there are any records
    let count_result: Result<Vec<Value>, _> = db
        .query("SELECT count() FROM opencode_favorite_plugin GROUP ALL")
        .await
        .map_err(|e| format!("Failed to count favorite plugins: {}", e))?
        .take(0);

    let is_empty = match count_result {
        Ok(records) => {
            records.first()
                .and_then(|r| r.get("count"))
                .and_then(|c| c.as_i64())
                .unwrap_or(0) == 0
        }
        Err(_) => true,
    };

    // Initialize default plugins if empty
    if is_empty {
        init_default_favorite_plugins(&db).await?;
    }

    // Query all favorite plugins ordered by created_at
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_plugin ORDER BY created_at ASC")
        .await
        .map_err(|e| format!("Failed to query favorite plugins: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            let plugins: Vec<OpenCodeFavoritePlugin> = records
                .into_iter()
                .map(adapter::from_db_value_favorite_plugin)
                .collect();
            Ok(plugins)
        }
        Err(e) => Err(format!("Failed to deserialize favorite plugins: {}", e)),
    }
}

/// Add a favorite plugin
/// Returns the created plugin, or existing one if already exists
#[tauri::command]
pub async fn add_opencode_favorite_plugin(
    state: tauri::State<'_, DbState>,
    plugin_name: String,
) -> Result<OpenCodeFavoritePlugin, String> {
    let db = state.0.lock().await;
    let now = chrono::Local::now().to_rfc3339();

    // Use INSERT IGNORE to avoid duplicates
    let record_id = db_record_id("opencode_favorite_plugin", &plugin_name);
    let query = format!("INSERT IGNORE INTO opencode_favorite_plugin {{ id: {}, plugin_name: $plugin_name, created_at: $created_at }}", record_id);
    db.query(&query)
        .bind(("plugin_name", plugin_name.clone()))
        .bind(("created_at", now.clone()))
        .await
        .map_err(|e| format!("Failed to add favorite plugin: {}", e))?;

    // Fetch the record (either newly created or existing)
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_plugin WHERE plugin_name = $plugin_name LIMIT 1")
        .bind(("plugin_name", plugin_name))
        .await
        .map_err(|e| format!("Failed to fetch favorite plugin: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.into_iter().next() {
                Ok(adapter::from_db_value_favorite_plugin(record))
            } else {
                Err("Failed to find favorite plugin after insert".to_string())
            }
        }
        Err(e) => Err(format!("Failed to deserialize favorite plugin: {}", e)),
    }
}

/// Delete a favorite plugin by plugin name
#[tauri::command]
pub async fn delete_opencode_favorite_plugin(
    state: tauri::State<'_, DbState>,
    plugin_name: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE FROM opencode_favorite_plugin WHERE plugin_name = $plugin_name")
        .bind(("plugin_name", plugin_name))
        .await
        .map_err(|e| format!("Failed to delete favorite plugin: {}", e))?;

    Ok(())
}

// ============================================================================
// Favorite Provider Commands
// ============================================================================

/// Sync providers from config file to database
/// Only inserts providers that don't exist in database
async fn sync_providers_from_config(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    config: &OpenCodeConfig,
) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    if let Some(ref providers) = config.provider {
        for (provider_id, provider_config) in providers.iter() {
            // Extract npm and base_url from provider_config
            let npm = provider_config.npm.clone().unwrap_or_default();
            let base_url = provider_config
                .options
                .as_ref()
                .and_then(|o| o.base_url.clone())
                .unwrap_or_default();

            // Serialize provider_config to JSON
            let provider_config_json = serde_json::to_value(provider_config)
                .map_err(|e| format!("Failed to serialize provider config: {}", e))?;

            // Use INSERT IGNORE to only insert if not exists
            let record_id = db_record_id("opencode_favorite_provider", provider_id);
            db.query(&format!("INSERT IGNORE INTO opencode_favorite_provider {{ id: {}, provider_id: $provider_id, npm: $npm, base_url: $base_url, provider_config: $provider_config, created_at: $created_at, updated_at: $updated_at }}", record_id))
                .bind(("provider_id", provider_id.clone()))
                .bind(("npm", npm))
                .bind(("base_url", base_url))
                .bind(("provider_config", provider_config_json))
                .bind(("created_at", now.clone()))
                .bind(("updated_at", now.clone()))
                .await
                .map_err(|e| format!("Failed to sync favorite provider: {}", e))?;
        }
    }

    Ok(())
}

/// List all favorite providers
/// Auto-syncs providers from config file (inserts only if not exists in database)
#[tauri::command]
pub async fn list_opencode_favorite_providers(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<OpenCodeFavoriteProvider>, String> {
    // First, get config path BEFORE locking db to avoid deadlock
    let config_path_str = get_opencode_config_path(state.clone()).await?;
    let config_path = std::path::Path::new(&config_path_str);

    // Read and parse config file
    let config_opt = if config_path.exists() {
        std::fs::read_to_string(config_path)
            .ok()
            .and_then(|content| json5::from_str::<OpenCodeConfig>(&content).ok())
    } else {
        None
    };

    // Now lock db and sync providers
    {
        let db = state.0.lock().await;

        if let Some(config) = config_opt {
            sync_providers_from_config(&db, &config).await?;
        }
    }

    // Query all favorite providers
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_provider ORDER BY created_at ASC")
        .await
        .map_err(|e| format!("Failed to query favorite providers: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            let providers: Vec<OpenCodeFavoriteProvider> = records
                .into_iter()
                .filter_map(adapter::from_db_value_favorite_provider)
                .collect();
            Ok(providers)
        }
        Err(e) => Err(format!("Failed to deserialize favorite providers: {}", e)),
    }
}

/// Upsert (create or update) a favorite provider
/// Called automatically when user adds/modifies a provider
#[tauri::command]
pub async fn upsert_opencode_favorite_provider(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    provider_config: OpenCodeProvider,
    diagnostics: Option<OpenCodeDiagnosticsConfig>,
) -> Result<OpenCodeFavoriteProvider, String> {
    let db = state.0.lock().await;
    let now = chrono::Local::now().to_rfc3339();

    // Extract npm and base_url from provider_config
    let npm = provider_config.npm.clone().unwrap_or_default();
    let base_url = provider_config
        .options
        .as_ref()
        .and_then(|o| o.base_url.clone())
        .unwrap_or_default();

    // Serialize provider_config to JSON
    let provider_config_json = serde_json::to_value(&provider_config)
        .map_err(|e| format!("Failed to serialize provider config: {}", e))?;

    // Read existing record to preserve created_at and diagnostics if not provided
    let existing_record: Option<OpenCodeFavoriteProvider> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_provider WHERE provider_id = $provider_id LIMIT 1")
        .bind(("provider_id", provider_id.clone()))
        .await
        .map_err(|e| format!("Failed to query favorite provider: {}", e))?
        .take::<Vec<Value>>(0)
        .ok()
        .and_then(|records| records.into_iter().next())
        .and_then(adapter::from_db_value_favorite_provider);

    let has_existing = existing_record.is_some();
    let created_at = existing_record
        .as_ref()
        .map(|record| record.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let diagnostics_to_save = diagnostics.or_else(|| existing_record.as_ref().and_then(|record| record.diagnostics.clone()));

    if has_existing {
        db.query("UPDATE opencode_favorite_provider SET npm = $npm, base_url = $base_url, provider_config = $provider_config, diagnostics = $diagnostics, updated_at = $updated_at WHERE provider_id = $provider_id")
            .bind(("provider_id", provider_id.clone()))
            .bind(("npm", npm))
            .bind(("base_url", base_url))
            .bind(("provider_config", provider_config_json))
            .bind(("diagnostics", diagnostics_to_save))
            .bind(("updated_at", now.clone()))
            .await
            .map_err(|e| format!("Failed to update favorite provider: {}", e))?;
    } else {
        let record_id = db_record_id("opencode_favorite_provider", &provider_id);
        db.query(&format!("INSERT INTO opencode_favorite_provider {{ id: {}, provider_id: $provider_id, npm: $npm, base_url: $base_url, provider_config: $provider_config, diagnostics: $diagnostics, created_at: $created_at, updated_at: $updated_at }}", record_id))
            .bind(("provider_id", provider_id.clone()))
            .bind(("npm", npm))
            .bind(("base_url", base_url))
            .bind(("provider_config", provider_config_json))
            .bind(("diagnostics", diagnostics_to_save))
            .bind(("created_at", created_at))
            .bind(("updated_at", now.clone()))
            .await
            .map_err(|e| format!("Failed to insert favorite provider: {}", e))?;
    }

    // Fetch and return the record
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_provider WHERE provider_id = $provider_id LIMIT 1")
        .bind(("provider_id", provider_id))
        .await
        .map_err(|e| format!("Failed to fetch favorite provider: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.into_iter().next() {
                adapter::from_db_value_favorite_provider(record)
                    .ok_or_else(|| "Failed to parse favorite provider".to_string())
            } else {
                Err("Failed to find favorite provider after upsert".to_string())
            }
        }
        Err(e) => Err(format!("Failed to deserialize favorite provider: {}", e)),
    }
}

/// Delete a favorite provider from database
#[tauri::command]
pub async fn delete_opencode_favorite_provider(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE FROM opencode_favorite_provider WHERE provider_id = $provider_id")
        .bind(("provider_id", provider_id))
        .await
        .map_err(|e| format!("Failed to delete favorite provider: {}", e))?;

    Ok(())
}
