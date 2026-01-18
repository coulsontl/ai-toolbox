use indexmap::IndexMap;
use std::fs;
use std::path::Path;
use serde_json::Value;
use tauri::Emitter;

use super::adapter;
use super::types::*;
use crate::db::DbState;

// ============================================================================
// Helper Functions
// ============================================================================

/// Fields in model that should be removed if they are empty objects
const MODEL_EMPTY_OBJECT_FIELDS: &[&str] = &["options", "variants", "modalities"];

/// Recursively clean empty objects from the config
/// Specifically targets options, variants, modalities in models
fn clean_empty_objects(value: &mut Value) {
    if let Value::Object(map) = value {
        // Check if this is a provider section
        if let Some(Value::Object(providers)) = map.get_mut("provider") {
            for (_provider_key, provider_value) in providers.iter_mut() {
                if let Value::Object(provider) = provider_value {
                    // Check models in each provider
                    if let Some(Value::Object(models)) = provider.get_mut("models") {
                        for (_model_key, model_value) in models.iter_mut() {
                            if let Value::Object(model) = model_value {
                                // Remove empty object fields
                                for field in MODEL_EMPTY_OBJECT_FIELDS {
                                    if let Some(Value::Object(obj)) = model.get(*field) {
                                        if obj.is_empty() {
                                            model.remove(*field);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

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

    // Serialize to JSON Value first, then clean up empty objects
    let mut json_value = serde_json::to_value(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    // Clean up empty objects in models (options, variants, modalities)
    clean_empty_objects(&mut json_value);

    // Serialize with pretty printing
    let json_content = serde_json::to_string_pretty(&json_value)
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
