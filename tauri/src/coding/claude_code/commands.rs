use chrono::Local;
use std::fs;
use std::path::Path;
use serde_json::Value;

use crate::db::DbState;
use super::adapter;
use super::types::*;
use tauri::Emitter;

// ============================================================================
// Claude Code Provider Commands
// ============================================================================

/// List all Claude Code providers ordered by sort_index
#[tauri::command]
pub async fn list_claude_providers(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<ClaudeCodeProvider>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_provider")
        .await
        .map_err(|e| format!("Failed to query providers: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            let mut result: Vec<ClaudeCodeProvider> = records
                .into_iter()
                .map(adapter::from_db_value_provider)
                .collect();
            result.sort_by_key(|p| p.sort_index.unwrap_or(0));
            Ok(result)
        }
        Err(e) => {
            eprintln!("❌ Failed to deserialize providers: {}", e);
            Ok(Vec::new())
        }
    }
}

/// Create a new Claude Code provider
#[tauri::command]
pub async fn create_claude_provider(
    state: tauri::State<'_, DbState>,
    provider: ClaudeCodeProviderInput,
) -> Result<ClaudeCodeProvider, String> {
    let db = state.0.lock().await;

    // Check if ID already exists
    let provider_id = provider.id.clone();
    let check_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_provider WHERE provider_id = $id OR providerId = $id LIMIT 1")
        .bind(("id", provider_id.clone()))
        .await
        .map_err(|e| format!("Failed to check provider existence: {}", e))?
        .take(0);

    if let Ok(records) = check_result {
        if !records.is_empty() {
            return Err(format!(
                "Claude provider with ID '{}' already exists",
                provider.id
            ));
        }
    }

    let now = Local::now().to_rfc3339();
    let content = ClaudeCodeProviderContent {
        provider_id: provider.id.clone(),
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        is_current: false,
        is_applied: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    db.query(format!("CREATE claude_provider:`{}` CONTENT $data", provider.id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    Ok(ClaudeCodeProvider {
        id: content.provider_id,
        name: content.name,
        category: content.category,
        settings_config: content.settings_config,
        source_provider_id: content.source_provider_id,
        website_url: content.website_url,
        notes: content.notes,
        icon: content.icon,
        icon_color: content.icon_color,
        sort_index: content.sort_index,
        is_current: content.is_current,
        is_applied: content.is_applied,
        created_at: content.created_at,
        updated_at: content.updated_at,
    })
}

/// Update an existing Claude Code provider
#[tauri::command]
pub async fn update_claude_provider(
    state: tauri::State<'_, DbState>,
    provider: ClaudeCodeProvider,
) -> Result<ClaudeCodeProvider, String> {
    let db = state.0.lock().await;

    // Get existing record to preserve created_at if not provided
    let provider_id = provider.id.clone();
    let existing_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_provider WHERE provider_id = $id OR providerId = $id LIMIT 1")
        .bind(("id", provider_id.clone()))
        .await
        .map_err(|e| format!("Failed to query existing provider: {}", e))?
        .take(0);

    let now = Local::now().to_rfc3339();
    let created_at = if !provider.created_at.is_empty() {
        provider.created_at
    } else if let Ok(records) = existing_result {
        if let Some(record) = records.first() {
            record
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or(&now)
                .to_string()
        } else {
            return Err("Provider not found".to_string());
        }
    } else {
        return Err("Provider not found".to_string());
    };

    let content = ClaudeCodeProviderContent {
        provider_id: provider.id.clone(),
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        is_current: provider.is_current,
        is_applied: provider.is_applied,
        created_at,
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    db.query(format!("DELETE claude_provider:`{}`", provider.id))
        .await
        .map_err(|e| format!("Failed to delete old provider: {}", e))?;

    db.query(format!("CREATE claude_provider:`{}` CONTENT $data", provider.id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create updated provider: {}", e))?;

    // 如果该配置当前是应用状态，立即重新写入到配置文件
    if content.is_applied {
        if let Err(e) = apply_config_to_file(&db, &provider.id).await {
            eprintln!("Failed to auto-apply updated config: {}", e);
            // 不中断更新流程，只记录错误
        }
    }

    Ok(ClaudeCodeProvider {
        id: content.provider_id,
        name: content.name,
        category: content.category,
        settings_config: content.settings_config,
        source_provider_id: content.source_provider_id,
        website_url: content.website_url,
        notes: content.notes,
        icon: content.icon,
        icon_color: content.icon_color,
        sort_index: content.sort_index,
        is_current: content.is_current,
        is_applied: content.is_applied,
        created_at: content.created_at,
        updated_at: content.updated_at,
    })
}

/// Delete a Claude Code provider
#[tauri::command]
pub async fn delete_claude_provider(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query(format!("DELETE claude_provider:`{}`", id))
        .await
        .map_err(|e| format!("Failed to delete claude provider: {}", e))?;

    Ok(())
}

/// Select a Claude Code provider as current (deselect others)
#[tauri::command]
pub async fn select_claude_provider(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    let now = Local::now().to_rfc3339();

    // Deselect all providers
    db.query("UPDATE claude_provider SET is_current = false, updated_at = $now")
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to deselect providers: {}", e))?;

    // Select the target provider (support both snake_case and camelCase for backward compatibility)
    db.query("UPDATE claude_provider SET is_current = true, updated_at = $now WHERE provider_id = $id OR providerId = $id")
        .bind(("id", id))
        .bind(("now", now))
        .await
        .map_err(|e| format!("Failed to select provider: {}", e))?;

    Ok(())
}

/// Reorder Claude Code providers
#[tauri::command]
pub async fn reorder_claude_providers(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.0.lock().await;
    let now = Local::now().to_rfc3339();

    for (index, id) in ids.iter().enumerate() {
        db.query("UPDATE claude_provider SET sort_index = $index, updated_at = $now WHERE provider_id = $id OR providerId = $id")
            .bind(("index", index as i32))
            .bind(("now", now.clone()))
            .bind(("id", id.clone()))
            .await
            .map_err(|e| format!("Failed to update provider {}: {}", id, e))?;
    }

    Ok(())
}

// ============================================================================
// Claude Config File Commands
// ============================================================================

/// Get Claude config file path (~/.claude/settings.json)
#[tauri::command]
pub fn get_claude_config_path() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    let config_path = Path::new(&home_dir).join(".claude").join("settings.json");
    Ok(config_path.to_string_lossy().to_string())
}

/// Reveal Claude config folder in file explorer
#[tauri::command]
pub fn reveal_claude_config_folder() -> Result<(), String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    let config_dir = Path::new(&home_dir).join(".claude");

    // Ensure directory exists
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create .claude directory: {}", e))?;
    }

    // Open in file explorer (platform-specific)
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}

/// Read Claude settings.json file
#[tauri::command]
pub async fn read_claude_settings() -> Result<ClaudeSettings, String> {
    let config_path_str = get_claude_config_path()?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        // Return empty settings if file doesn't exist
        return Ok(ClaudeSettings {
            env: None,
            other: serde_json::Map::new(),
        });
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;

    let settings: ClaudeSettings = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings file: {}", e))?;

    Ok(settings)
}

/// 内部函数：将指定 provider 的配置应用到 settings.json（不改变数据库中的 is_applied 状态）
async fn apply_config_to_file(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_to_file_public(db, provider_id).await
}

/// Public version of apply_config_to_file for tray module
pub async fn apply_config_to_file_public(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {


    // Get the provider (support both snake_case and camelCase for backward compatibility)
    let provider_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_provider WHERE provider_id = $id OR providerId = $id LIMIT 1")
        .bind(("id", provider_id.to_string()))
        .await
        .map_err(|e| format!("Failed to query provider: {}", e))?
        .take(0);

    let provider = match provider_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::from_db_value_provider(record.clone())
            } else {
                return Err("Provider not found".to_string());
            }
        }
        Err(e) => {
            return Err(format!("Failed to deserialize provider: {}", e));
        }
    };

    // Parse provider settings_config
    let provider_config: serde_json::Value = serde_json::from_str(&provider.settings_config)
        .map_err(|e| format!("Failed to parse provider config: {}", e))?;

    // Get common config
    let common_config_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query common config: {}", e))?
        .take(0);

    let common_config: serde_json::Value = match common_config_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                let config = adapter::from_db_value_common(record.clone());
                serde_json::from_str(&config.config)
                    .map_err(|e| format!("Failed to parse common config: {}", e))?
            } else {
                serde_json::json!({})
            }
        }
        Err(_) => serde_json::json!({}),
    };

    // Build env section from provider config
    let mut env = serde_json::Map::new();

    // Get env section from provider config
    if let Some(env_config) = provider_config.get("env").and_then(|v| v.as_object()) {
        if let Some(api_key) = env_config.get("ANTHROPIC_API_KEY").and_then(|v| v.as_str()) {
            env.insert(
                "ANTHROPIC_API_KEY".to_string(),
                serde_json::json!(api_key),
            );
        }

        if let Some(base_url) = env_config.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()) {
            env.insert(
                "ANTHROPIC_BASE_URL".to_string(),
                serde_json::json!(base_url),
            );
        }

        if let Some(auth_token) = env_config
            .get("ANTHROPIC_AUTH_TOKEN")
            .and_then(|v| v.as_str())
        {
            env.insert(
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                serde_json::json!(auth_token),
            );
        }
    }

    if let Some(model) = provider_config.get("model").and_then(|v| v.as_str()) {
        env.insert("ANTHROPIC_MODEL".to_string(), serde_json::json!(model));
    }

    if let Some(haiku) = provider_config.get("haikuModel").and_then(|v| v.as_str()) {
        env.insert(
            "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
            serde_json::json!(haiku),
        );
    }

    if let Some(sonnet) = provider_config.get("sonnetModel").and_then(|v| v.as_str()) {
        env.insert(
            "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
            serde_json::json!(sonnet),
        );
    }

    if let Some(opus) = provider_config.get("opusModel").and_then(|v| v.as_str()) {
        env.insert(
            "ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(),
            serde_json::json!(opus),
        );
    }

    // Merge common config and provider env
    let mut final_settings = if let serde_json::Value::Object(map) = common_config {
        map
    } else {
        serde_json::Map::new()
    };

    // Get or create env from common config
    let mut merged_env = final_settings
        .get("env")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    // Merge provider env into common env (provider takes precedence)
    for (key, value) in env {
        merged_env.insert(key, value);
    }

    // Remove old env and insert merged env at the end (env should be at the bottom)
    final_settings.remove("env");
    final_settings.insert("env".to_string(), serde_json::json!(merged_env));

    // Write to settings.json
    let config_path_str = get_claude_config_path()?;
    let config_path = Path::new(&config_path_str);

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create .claude directory: {}", e))?;
        }
    }

    let json_content = serde_json::to_string_pretty(&final_settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    fs::write(config_path, json_content)
        .map_err(|e| format!("Failed to write settings file: {}", e))?;

    Ok(())
}
/// Apply Claude Code provider configuration to settings.json
#[tauri::command]
pub async fn apply_claude_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    apply_config_internal(&db, &app, &provider_id, false).await?;
    Ok(())
}

/// Internal function to apply config: writes to file and updates database
/// This is the single source of truth for applying a Claude Code provider config
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    // 应用配置到文件
    apply_config_to_file(db, provider_id).await?;

    // Update provider's is_applied status
    let now = Local::now().to_rfc3339();

    // Mark all providers as not applied
    db.query("UPDATE claude_provider SET is_applied = false, updated_at = $now")
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to reset applied status: {}", e))?;

    // Mark target provider as applied (support both snake_case and camelCase for backward compatibility)
    db.query("UPDATE claude_provider SET is_applied = true, updated_at = $now WHERE provider_id = $id OR providerId = $id")
        .bind(("id", provider_id.to_string()))
        .bind(("now", now))
        .await
        .map_err(|e| format!("Failed to set applied status: {}", e))?;

    // Notify based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    Ok(())
}

// ============================================================================
// Claude Common Config Commands
// ============================================================================

/// Get Claude common config
#[tauri::command]
pub async fn get_claude_common_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<ClaudeCommonConfig>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query common config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(Some(adapter::from_db_value_common(record.clone())))
            } else {
                Ok(None)
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to deserialize common config: {}", e);
            Ok(None)
        }
    }
}

/// Save Claude common config
#[tauri::command]
pub async fn save_claude_common_config(
    state: tauri::State<'_, DbState>,
    config: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Validate JSON
    let _: serde_json::Value =
        serde_json::from_str(&config).map_err(|e| format!("Invalid JSON: {}", e))?;

    let json_data = adapter::to_db_value_common(&config);

    db.query("DELETE claude_common_config:`common`")
        .await
        .map_err(|e| format!("Failed to delete old common config: {}", e))?;

    db.query("CREATE claude_common_config:`common` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create common config: {}", e))?;

    // 查找当前应用的 provider，如果存在则重新应用到文件
    let applied_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_provider WHERE is_applied = true LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query applied provider: {}", e))?
        .take(0);

    if let Ok(records) = applied_result {
        if let Some(record) = records.first() {
            let applied_provider = adapter::from_db_value_provider(record.clone());
            // 重新应用配置到文件（不改变数据库中的 is_applied 状态）
            if let Err(e) = apply_config_to_file(&db, &applied_provider.id).await {
                eprintln!("Failed to auto-apply config after common config update: {}", e);
                // 不中断保存流程，只记录错误
            }
        }
    }

    Ok(())
}

// ============================================================================
// Claude Plugin Integration Commands
// ============================================================================

/// Get Claude plugin config path (~/.claude/config.json)
fn get_claude_plugin_config_path() -> Result<std::path::PathBuf, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    Ok(std::path::Path::new(&home_dir).join(".claude").join("config.json"))
}

/// Check if plugin config has primaryApiKey = "any"
fn is_plugin_config_enabled(content: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value) => value
            .get("primaryApiKey")
            .and_then(|v| v.as_str())
            .map(|val| val == "any")
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Get Claude plugin integration status
#[tauri::command]
pub async fn get_claude_plugin_status() -> Result<ClaudePluginStatus, String> {
    let config_path = get_claude_plugin_config_path()?;
    let has_config_file = config_path.exists();

    if !has_config_file {
        return Ok(ClaudePluginStatus {
            enabled: false,
            has_config_file: false,
        });
    }

    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let enabled = is_plugin_config_enabled(&content);

    Ok(ClaudePluginStatus {
        enabled,
        has_config_file: true,
    })
}

/// Apply Claude plugin configuration
#[tauri::command]
pub async fn apply_claude_plugin_config(enabled: bool) -> Result<bool, String> {
    let config_path = get_claude_plugin_config_path()?;

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create .claude directory: {}", e))?;
        }
    }

    // Read existing config or create empty
    let mut obj: serde_json::Map<String, serde_json::Value> = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };

    if enabled {
        // Set primaryApiKey = "any"
        let current = obj
            .get("primaryApiKey")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if current != "any" {
            obj.insert(
                "primaryApiKey".to_string(),
                serde_json::Value::String("any".to_string()),
            );
        }
    } else {
        // Remove primaryApiKey field
        obj.remove("primaryApiKey");
    }

    // Write back to file
    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(&config_path, format!("{serialized}\n"))
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(true)
}
