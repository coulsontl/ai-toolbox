use std::fs;
use std::path::Path;
use serde_json::Value;

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
fn get_default_config_path() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    let config_dir = Path::new(&home_dir).join(".config").join("opencode");

    // Check for .json first, then .jsonc
    let json_path = config_dir.join("opencode.json");
    let jsonc_path = config_dir.join("opencode.jsonc");

    if json_path.exists() {
        Ok(json_path.to_string_lossy().to_string())
    } else if jsonc_path.exists() {
        Ok(jsonc_path.to_string_lossy().to_string())
    } else {
        // Return default path for new file
        Ok(json_path.to_string_lossy().to_string())
    }
}

/// Read OpenCode configuration file
#[tauri::command]
pub async fn read_opencode_config(state: tauri::State<'_, DbState>) -> Result<Option<OpenCodeConfig>, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let mut config: OpenCodeConfig = json5::from_str(&content)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    // Fill missing name fields with provider key
    // Fill missing npm fields with smart default based on provider key/name
    for (key, provider) in config.provider.iter_mut() {
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

    Ok(Some(config))
}

/// Save OpenCode configuration file
#[tauri::command]
pub async fn save_opencode_config(state: tauri::State<'_, DbState>, config: OpenCodeConfig) -> Result<(), String> {
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
        .query("SELECT * OMIT id FROM opencode_common_config:`common` LIMIT 1")
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
            eprintln!("❌ Failed to deserialize opencode common config: {}", e);
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

    db.query("DELETE opencode_common_config:`common`")
        .await
        .map_err(|e| format!("Failed to delete old opencode common config: {}", e))?;

    db.query("CREATE opencode_common_config:`common` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create opencode common config: {}", e))?;

    Ok(())
}
