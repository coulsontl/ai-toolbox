use chrono::Local;
use serde_json::Value;
use std::fs;
use std::path::Path;

use super::adapter;
use super::types::*;
use crate::coding::all_api_hub;
use crate::coding::db_id::{db_new_id, db_record_id};
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::db::DbState;
use tauri::Emitter;

const KNOWN_ENV_FIELDS: [&str; 8] = [
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_REASONING_MODEL",
];

fn get_claude_prompt_file_path() -> Result<std::path::PathBuf, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    Ok(Path::new(&home_dir).join(".claude").join("CLAUDE.md"))
}

async fn get_local_prompt_config() -> Result<Option<ClaudePromptConfig>, String> {
    let prompt_path = get_claude_prompt_file_path()?;
    let Some(prompt_content) = read_prompt_content_file(&prompt_path, "Claude Code")? else {
        return Ok(None);
    };

    let now = Local::now().to_rfc3339();
    Ok(Some(ClaudePromptConfig {
        id: "__local__".to_string(),
        name: "default".to_string(),
        content: prompt_content,
        is_applied: true,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }))
}

fn write_prompt_content_to_file(prompt_content: Option<&str>) -> Result<(), String> {
    let prompt_path = get_claude_prompt_file_path()?;
    write_prompt_content_file(&prompt_path, prompt_content, "Claude Code")
}

fn emit_prompt_sync_requests<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-claude", ());

    let _ = app.emit("ssh-sync-request-claude", ());
}

// ============================================================================
// Claude Code Provider Commands
// ============================================================================

/// List all Claude Code providers ordered by sort_index
#[tauri::command]
pub async fn list_claude_providers(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<ClaudeCodeProvider>, String> {
    let db = state.db();

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM claude_provider")
        .await
        .map_err(|e| format!("Failed to query providers: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if records.is_empty() {
                // Database is empty, try to load from local file as temporary provider
                if let Ok(temp_provider) = load_temp_provider_from_file().await {
                    return Ok(vec![temp_provider]);
                }
                Ok(Vec::new())
            } else {
                let mut result: Vec<ClaudeCodeProvider> = records
                    .into_iter()
                    .map(adapter::from_db_value_provider)
                    .collect();
                result.sort_by_key(|p| p.sort_index.unwrap_or(0));
                Ok(result)
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to deserialize providers: {}", e);
            // Try to load from local file as fallback
            if let Ok(temp_provider) = load_temp_provider_from_file().await {
                return Ok(vec![temp_provider]);
            }
            Ok(Vec::new())
        }
    }
}

/// Load a temporary provider from settings.json without writing to database
/// This is used when the database is empty and we want to show the local config
async fn load_temp_provider_from_file() -> Result<ClaudeCodeProvider, String> {
    let config_path_str = get_claude_config_path()?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Err("No settings file found".to_string());
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;

    let settings: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings file: {}", e))?;

    let settings_obj = match settings.as_object() {
        Some(obj) => obj,
        None => return Err("Invalid settings format".to_string()),
    };

    let env_obj = match settings_obj.get("env").and_then(|v| v.as_object()) {
        Some(env) => env,
        None => return Err("No env section in settings".to_string()),
    };

    // Build provider settings
    let mut provider_settings = serde_json::Map::new();
    let mut provider_env = serde_json::Map::new();

    // Extract known fields
    let api_key = env_obj
        .get("ANTHROPIC_AUTH_TOKEN")
        .or_else(|| env_obj.get("ANTHROPIC_API_KEY"));
    if let Some(key) = api_key {
        provider_env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), key.clone());
    }
    if let Some(base_url) = env_obj.get("ANTHROPIC_BASE_URL") {
        provider_env.insert("ANTHROPIC_BASE_URL".to_string(), base_url.clone());
    }
    provider_settings.insert("env".to_string(), serde_json::json!(provider_env));

    if let Some(model) = env_obj.get("ANTHROPIC_MODEL") {
        provider_settings.insert("model".to_string(), model.clone());
    }
    if let Some(haiku) = env_obj.get("ANTHROPIC_DEFAULT_HAIKU_MODEL") {
        provider_settings.insert("haikuModel".to_string(), haiku.clone());
    }
    if let Some(sonnet) = env_obj.get("ANTHROPIC_DEFAULT_SONNET_MODEL") {
        provider_settings.insert("sonnetModel".to_string(), sonnet.clone());
    }
    if let Some(opus) = env_obj.get("ANTHROPIC_DEFAULT_OPUS_MODEL") {
        provider_settings.insert("opusModel".to_string(), opus.clone());
    }
    if let Some(reasoning) = env_obj.get("ANTHROPIC_REASONING_MODEL") {
        provider_settings.insert("reasoningModel".to_string(), reasoning.clone());
    }

    let now = Local::now().to_rfc3339();
    Ok(ClaudeCodeProvider {
        id: "__local__".to_string(), // Special ID to indicate this is from local file
        name: "default".to_string(),
        category: "custom".to_string(),
        settings_config: serde_json::to_string(&provider_settings)
            .map_err(|e| format!("Failed to serialize: {}", e))?,
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Create a new Claude Code provider
#[tauri::command]
pub async fn create_claude_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider: ClaudeCodeProviderInput,
) -> Result<ClaudeCodeProvider, String> {
    let db = state.db();

    let now = Local::now().to_rfc3339();
    let content = ClaudeCodeProviderContent {
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        is_applied: false,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    // Create new provider - SurrealDB auto-generates record ID
    db.query("CREATE claude_provider CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    // Fetch the created record to get the auto-generated ID
    let result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM claude_provider ORDER BY created_at DESC LIMIT 1")
        .await
        .map_err(|e| format!("Failed to fetch created provider: {}", e))?
        .take(0);

    // Notify to refresh tray menu
    let _ = app.emit("config-changed", "window");

    match result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::from_db_value_provider(record.clone()))
            } else {
                Err("Failed to retrieve created provider".to_string())
            }
        }
        Err(e) => Err(format!("Failed to retrieve created provider: {}", e)),
    }
}

/// Update an existing Claude Code provider
#[tauri::command]
pub async fn update_claude_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider: ClaudeCodeProvider,
) -> Result<ClaudeCodeProvider, String> {
    let db = state.db();

    // Use the id from frontend (pure string id without table prefix)
    let id = provider.id.clone();
    let now = Local::now().to_rfc3339();

    // Get existing record to preserve created_at
    let record_id = db_record_id("claude_provider", &id);
    let existing_result: Result<Vec<Value>, _> = db
        .query(&format!("SELECT * OMIT id FROM {} LIMIT 1", record_id))
        .await
        .map_err(|e| format!("Failed to query existing provider: {}", e))?
        .take(0);

    // Check if provider exists
    if let Ok(records) = &existing_result {
        if records.is_empty() {
            return Err(format!("Claude Code provider with ID '{}' not found", id));
        }
    }

    // Get created_at and is_disabled from existing record
    let (created_at, existing_is_disabled) = if !provider.created_at.is_empty() {
        (provider.created_at, false)
    } else if let Ok(records) = &existing_result {
        if let Some(record) = records.first() {
            let created = record
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or(&now)
                .to_string();
            let is_disabled = record
                .get("is_disabled")
                .or_else(|| record.get("isDisabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            (created, is_disabled)
        } else {
            (now.clone(), false)
        }
    } else {
        (now.clone(), false)
    };

    let content = ClaudeCodeProviderContent {
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        is_applied: provider.is_applied,
        is_disabled: existing_is_disabled,
        created_at,
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    // Use database id for update
    db.query(format!("UPDATE claude_provider:`{}` CONTENT $data", id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to update provider: {}", e))?;

    // 如果该配置当前是应用状态，立即重新写入到配置文件
    if content.is_applied {
        if let Err(e) = apply_config_to_file(&db, &id).await {
            eprintln!("Failed to auto-apply updated config: {}", e);
            // 不中断更新流程，只记录错误
        }
    }

    // Notify frontend and tray to refresh
    let _ = app.emit("config-changed", "window");

    Ok(ClaudeCodeProvider {
        id,
        name: content.name,
        category: content.category,
        settings_config: content.settings_config,
        source_provider_id: content.source_provider_id,
        website_url: content.website_url,
        notes: content.notes,
        icon: content.icon,
        icon_color: content.icon_color,
        sort_index: content.sort_index,
        is_applied: content.is_applied,
        is_disabled: content.is_disabled,
        created_at: content.created_at,
        updated_at: content.updated_at,
    })
}

/// Delete a Claude Code provider
#[tauri::command]
pub async fn delete_claude_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();

    db.query(format!("DELETE claude_provider:`{}`", id))
        .await
        .map_err(|e| format!("Failed to delete claude provider: {}", e))?;

    // Notify to refresh tray menu
    let _ = app.emit("config-changed", "window");

    Ok(())
}

/// Reorder Claude Code providers
#[tauri::command]
pub async fn reorder_claude_providers(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    for (index, id) in ids.iter().enumerate() {
        let record_id = db_record_id("claude_provider", id);
        db.query(&format!(
            "UPDATE {} SET sort_index = $index, updated_at = $now",
            record_id
        ))
        .bind(("index", index as i32))
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to update provider {}: {}", id, e))?;
    }

    Ok(())
}

/// Select a Claude Code provider (mark as applied in database, but not write to file)
/// This sets the provider as "current" using is_applied field
#[tauri::command]
pub async fn select_claude_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();

    let now = Local::now().to_rfc3339();

    // Mark all providers as not applied (only update the currently applied one)
    db.query(
        "UPDATE claude_provider SET is_applied = false, updated_at = $now WHERE is_applied = true",
    )
    .bind(("now", now.clone()))
    .await
    .map_err(|e| format!("Failed to reset applied status: {}", e))?;

    // Mark target provider as applied
    let record_id = db_record_id("claude_provider", &id);
    db.query(&format!(
        "UPDATE {} SET is_applied = true, updated_at = $now",
        record_id
    ))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to set applied status: {}", e))?;

    // Notify frontend to refresh
    let _ = app.emit("config-changed", "window");

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
    // Get the provider
    let record_id = db_record_id("claude_provider", provider_id);
    let provider_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
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

    // Check if provider is disabled
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider_id
        ));
    }

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
        // 兼容旧版本：优先使用 ANTHROPIC_AUTH_TOKEN，如果没有则使用 ANTHROPIC_API_KEY
        let api_key = env_config
            .get("ANTHROPIC_AUTH_TOKEN")
            .or_else(|| env_config.get("ANTHROPIC_API_KEY"))
            .and_then(|v| v.as_str());
        if let Some(key) = api_key {
            env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), serde_json::json!(key));
        }

        if let Some(base_url) = env_config
            .get("ANTHROPIC_BASE_URL")
            .and_then(|v| v.as_str())
        {
            env.insert(
                "ANTHROPIC_BASE_URL".to_string(),
                serde_json::json!(base_url),
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

    if let Some(reasoning) = provider_config.get("reasoningModel").and_then(|v| v.as_str()) {
        env.insert(
            "ANTHROPIC_REASONING_MODEL".to_string(),
            serde_json::json!(reasoning),
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
/// Toggle is_disabled status for a provider
#[tauri::command]
pub async fn toggle_claude_code_provider_disabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.db();

    // Update is_disabled field in database
    let now = Local::now().to_rfc3339();
    db.query(format!(
        "UPDATE claude_provider:`{}` SET is_disabled = $is_disabled, updated_at = $now",
        provider_id
    ))
    .bind(("is_disabled", is_disabled))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to toggle provider disabled status: {}", e))?;

    // If this provider is applied and now disabled, re-apply config to update files
    let toggle_record_id = db_record_id("claude_provider", &provider_id);
    let provider: Option<Value> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {}",
            toggle_record_id
        ))
        .await
        .map_err(|e| format!("Failed to query provider: {}", e))?
        .take(0)
        .map_err(|e| format!("Failed to parse provider: {}", e))?;

    if let Some(provider_value) = provider {
        let is_applied = provider_value
            .get("is_applied")
            .or_else(|| provider_value.get("isApplied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_applied {
            // Re-apply config to update files (will check is_disabled internally)
            apply_config_internal(&db, &app, &provider_id, false).await?;
        }
    }

    Ok(())
}

/// Apply Claude Code provider configuration to settings.json
#[tauri::command]
pub async fn apply_claude_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    let db = state.db();
    apply_config_internal(&db, &app, &provider_id, false).await
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

    // Mark all providers as not applied (only update the currently applied one)
    db.query(
        "UPDATE claude_provider SET is_applied = false, updated_at = $now WHERE is_applied = true",
    )
    .bind(("now", now.clone()))
    .await
    .map_err(|e| format!("Failed to reset applied status: {}", e))?;

    // Mark target provider as applied
    let apply_record_id = db_record_id("claude_provider", provider_id);
    db.query(&format!(
        "UPDATE {} SET is_applied = true, updated_at = $now",
        apply_record_id
    ))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to set applied status: {}", e))?;

    // Notify based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-claude", ());

    Ok(())
}

// ============================================================================
// Claude Prompt Config Commands
// ============================================================================

#[tauri::command]
pub async fn list_claude_prompt_configs(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<ClaudePromptConfig>, String> {
    let db = state.db();

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM claude_prompt_config")
        .await
        .map_err(|e| format!("Failed to query prompt configs: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if records.is_empty() {
                drop(db);
                if let Some(local_config) = get_local_prompt_config().await? {
                    return Ok(vec![local_config]);
                }
                return Ok(Vec::new());
            }

            let mut result: Vec<ClaudePromptConfig> = records
                .into_iter()
                .map(adapter::from_db_value_prompt)
                .collect();

            result.sort_by(|a, b| match (a.sort_index, b.sort_index) {
                (Some(ai), Some(bi)) => ai.cmp(&bi),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.cmp(&b.name),
            });

            Ok(result)
        }
        Err(e) => {
            eprintln!("Failed to deserialize Claude prompt configs: {}", e);
            drop(db);
            if let Some(local_config) = get_local_prompt_config().await? {
                return Ok(vec![local_config]);
            }
            Ok(Vec::new())
        }
    }
}

#[tauri::command]
pub async fn create_claude_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: ClaudePromptConfigInput,
) -> Result<ClaudePromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    let sort_index_result: Result<Vec<Value>, _> = db
        .query("SELECT sort_index FROM claude_prompt_config ORDER BY sort_index DESC LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query prompt sort index: {}", e))?
        .take(0);

    let next_sort_index = sort_index_result
        .ok()
        .and_then(|records| records.first().cloned())
        .and_then(|record| record.get("sort_index").and_then(|value| value.as_i64()))
        .map(|value| value as i32 + 1)
        .unwrap_or(0);

    let content = ClaudePromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_prompt(&content);
    let prompt_id = db_new_id();
    let record_id = db_record_id("claude_prompt_config", &prompt_id);

    db.query(&format!("CREATE {} CONTENT $data", record_id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create prompt config: {}", e))?;

    let records_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query created prompt config: {}", e))?
        .take(0);
    let created_config = match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::from_db_value_prompt(record.clone())
            } else {
                return Err("Failed to retrieve created prompt config".to_string());
            }
        }
        Err(e) => {
            return Err(format!(
                "Failed to deserialize created prompt config: {}",
                e
            ))
        }
    };

    let _ = app.emit("config-changed", "window");

    Ok(created_config)
}

#[tauri::command]
pub async fn update_claude_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: ClaudePromptConfigInput,
) -> Result<ClaudePromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let record_id = db_record_id("claude_prompt_config", &config_id);

    let existing_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT created_at, is_applied, sort_index FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query prompt config: {}", e))?
        .take(0);

    let (created_at, is_applied, sort_index) = match existing_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                let created_at = record
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| Box::leak(Local::now().to_rfc3339().into_boxed_str()))
                    .to_string();
                let is_applied = record
                    .get("is_applied")
                    .or_else(|| record.get("isApplied"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let sort_index = record
                    .get("sort_index")
                    .or_else(|| record.get("sortIndex"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                (created_at, is_applied, sort_index)
            } else {
                return Err(format!("Prompt config '{}' not found", config_id));
            }
        }
        Err(e) => return Err(format!("Failed to deserialize prompt config: {}", e)),
    };

    let now = Local::now().to_rfc3339();
    let content = ClaudePromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied,
        sort_index,
        created_at,
        updated_at: now.clone(),
    };
    let json_data = adapter::to_db_value_prompt(&content);

    db.query(&format!("UPDATE {} CONTENT $data", record_id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to update prompt config: {}", e))?;

    drop(db);

    if is_applied {
        write_prompt_content_to_file(Some(input.content.as_str()))?;
        emit_prompt_sync_requests(&app);
    }

    let _ = app.emit("config-changed", "window");

    Ok(ClaudePromptConfig {
        id: config_id,
        name: content.name,
        content: content.content,
        is_applied,
        sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(now),
    })
}

#[tauri::command]
pub async fn delete_claude_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("claude_prompt_config", &id);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete prompt config: {}", e))?;

    drop(db);
    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub async fn apply_prompt_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    if config_id == "__local__" {
        let local_prompt = get_local_prompt_config()
            .await?
            .ok_or_else(|| "Local default prompt not found".to_string())?;
        write_prompt_content_to_file(Some(local_prompt.content.as_str()))?;

        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
        emit_prompt_sync_requests(app);

        return Ok(());
    }

    let db = state.db();
    let record_id = db_record_id("claude_prompt_config", config_id);
    let records_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query prompt config: {}", e))?
        .take(0);

    let prompt_config = match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::from_db_value_prompt(record.clone())
            } else {
                return Err(format!("Prompt config '{}' not found", config_id));
            }
        }
        Err(e) => return Err(format!("Failed to deserialize prompt config: {}", e)),
    };

    let now = Local::now().to_rfc3339();

    db.query("UPDATE claude_prompt_config SET is_applied = false, updated_at = $now WHERE is_applied = true")
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to clear prompt applied flags: {}", e))?;

    db.query(&format!(
        "UPDATE {} SET is_applied = true, updated_at = $now",
        record_id
    ))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to set prompt applied flag: {}", e))?;

    drop(db);

    write_prompt_content_to_file(Some(prompt_config.content.as_str()))?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);
    emit_prompt_sync_requests(app);

    Ok(())
}

#[tauri::command]
pub async fn apply_claude_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_prompt_config_internal(state, &app, &config_id, false).await
}

#[tauri::command]
pub async fn reorder_claude_prompt_configs(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();

    for (index, id) in ids.iter().enumerate() {
        let record_id = db_record_id("claude_prompt_config", id);
        db.query(&format!("UPDATE {} SET sort_index = $index", record_id))
            .bind(("index", index as i32))
            .await
            .map_err(|e| format!("Failed to update prompt sort index: {}", e))?;
    }

    drop(db);
    let _ = app.emit("config-changed", "window");

    Ok(())
}

#[tauri::command]
pub async fn save_claude_local_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: ClaudePromptConfigInput,
) -> Result<ClaudePromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        get_local_prompt_config()
            .await?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };

    let created = create_claude_prompt_config(
        state.clone(),
        app.clone(),
        ClaudePromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;

    apply_prompt_config_internal(state.clone(), &app, &created.id, false).await?;

    let db = state.db();
    let record_id = db_record_id("claude_prompt_config", &created.id);
    let refreshed_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query saved local prompt config: {}", e))?
        .take(0);

    match refreshed_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::from_db_value_prompt(record.clone()))
            } else {
                Ok(created)
            }
        }
        Err(_) => Ok(created),
    }
}

// ============================================================================
// Claude Common Config Commands
// ============================================================================

/// Get Claude common config
#[tauri::command]
pub async fn get_claude_common_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<ClaudeCommonConfig>, String> {
    let db = state.db();

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM claude_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query common config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(Some(adapter::from_db_value_common(record.clone())))
            } else {
                // Database is empty, try to load from local file
                if let Ok(temp_common) = load_temp_common_config_from_file().await {
                    Ok(Some(temp_common))
                } else {
                    Ok(None)
                }
            }
        }
        Err(e) => {
            // Try to load from local file as fallback
            if let Ok(temp_common) = load_temp_common_config_from_file().await {
                Ok(Some(temp_common))
            } else {
                // 反序列化失败，删除旧数据以修复版本冲突
                eprintln!(
                    "⚠️ Claude common config has incompatible format, cleaning up: {}",
                    e
                );
                let _ = db.query("DELETE claude_common_config:`common`").await;
                Ok(None)
            }
        }
    }
}

/// Load a temporary common config from settings.json without writing to database
/// This extracts non-env fields and unknown env fields from settings.json
async fn load_temp_common_config_from_file() -> Result<ClaudeCommonConfig, String> {
    let config_path_str = get_claude_config_path()?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Err("No settings file found".to_string());
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;

    let settings: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings file: {}", e))?;

    let settings_obj = match settings.as_object() {
        Some(obj) => obj,
        None => return Err("Invalid settings format".to_string()),
    };

    let mut common_config = serde_json::Map::new();

    // Add non-env fields to common config
    for (key, value) in settings_obj {
        if key != "env" {
            common_config.insert(key.clone(), value.clone());
        }
    }

    // Add unknown env fields to common config's env
    if let Some(env_obj) = settings_obj.get("env").and_then(|v| v.as_object()) {
        let mut common_env = serde_json::Map::new();
        for (key, value) in env_obj {
            if !KNOWN_ENV_FIELDS.contains(&key.as_str()) {
                common_env.insert(key.clone(), value.clone());
            }
        }
        if !common_env.is_empty() {
            common_config.insert("env".to_string(), serde_json::json!(common_env));
        }
    }

    let now = Local::now().to_rfc3339();
    Ok(ClaudeCommonConfig {
        config: serde_json::to_string(&common_config)
            .map_err(|e| format!("Failed to serialize: {}", e))?,
        updated_at: now,
    })
}

/// Save Claude common config
#[tauri::command]
pub async fn save_claude_common_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config: String,
) -> Result<(), String> {
    let db = state.db();

    // Validate JSON
    let _: serde_json::Value =
        serde_json::from_str(&config).map_err(|e| format!("Invalid JSON: {}", e))?;

    let json_data = adapter::to_db_value_common(&config);

    // Use UPSERT to handle both update and create
    db.query("UPSERT claude_common_config:`common` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save common config: {}", e))?;

    // 查找当前应用的 provider，如果存在则重新应用到文件
    let applied_result: Result<Vec<Value>, _> = db
        .query(
            "SELECT *, type::string(id) as id FROM claude_provider WHERE is_applied = true LIMIT 1",
        )
        .await
        .map_err(|e| format!("Failed to query applied provider: {}", e))?
        .take(0);

    if let Ok(records) = applied_result {
        if let Some(record) = records.first() {
            let applied_provider = adapter::from_db_value_provider(record.clone());
            // 重新应用配置到文件（不改变数据库中的 is_applied 状态）
            if let Err(e) = apply_config_to_file(&db, &applied_provider.id).await {
                eprintln!(
                    "Failed to auto-apply config after common config update: {}",
                    e
                );
                // 不中断保存流程，只记录错误
            }
        }
    }

    // Notify frontend to refresh
    let _ = app.emit("config-changed", "window");

    Ok(())
}

/// Save local config (provider and/or common) into database
/// Input can include provider and/or commonConfig; missing parts will be loaded from settings.json
#[tauri::command]
pub async fn save_claude_local_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: ClaudeLocalConfigInput,
) -> Result<(), String> {
    let db = state.db();

    // Load base provider/common from local settings
    let base_provider = load_temp_provider_from_file().await?;
    let base_common = load_temp_common_config_from_file().await.ok();

    let provider_input = input.provider;
    let provider_name = provider_input
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or(base_provider.name);
    let provider_category = provider_input
        .as_ref()
        .map(|p| p.category.clone())
        .unwrap_or(base_provider.category);
    let provider_settings_config = provider_input
        .as_ref()
        .map(|p| p.settings_config.clone())
        .unwrap_or(base_provider.settings_config);
    let provider_source_id = provider_input
        .as_ref()
        .and_then(|p| p.source_provider_id.clone());
    let provider_notes = provider_input
        .as_ref()
        .and_then(|p| p.notes.clone())
        .or(base_provider.notes);
    let provider_sort_index = provider_input
        .as_ref()
        .and_then(|p| p.sort_index)
        .or(base_provider.sort_index);

    let common_config = if let Some(config) = input.common_config {
        // Validate JSON
        let _: serde_json::Value =
            serde_json::from_str(&config).map_err(|e| format!("Invalid JSON: {}", e))?;
        config
    } else if let Some(common) = base_common {
        common.config
    } else {
        "{}".to_string()
    };

    let now = Local::now().to_rfc3339();
    let provider_content = ClaudeCodeProviderContent {
        name: provider_name,
        category: provider_category,
        settings_config: provider_settings_config,
        source_provider_id: provider_source_id,
        website_url: None,
        notes: provider_notes,
        icon: None,
        icon_color: None,
        sort_index: provider_sort_index,
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_json = adapter::to_db_value_provider(&provider_content);
    db.query("CREATE claude_provider CONTENT $data")
        .bind(("data", provider_json))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    let common_json = adapter::to_db_value_common(&common_config);
    db.query("UPSERT claude_common_config:`common` CONTENT $data")
        .bind(("data", common_json))
        .await
        .map_err(|e| format!("Failed to save common config: {}", e))?;

    // Re-apply config to file using the newly created provider
    let created_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM claude_provider ORDER BY created_at DESC LIMIT 1")
        .await
        .map_err(|e| format!("Failed to fetch created provider: {}", e))?
        .take(0);
    if let Ok(records) = created_result {
        if let Some(record) = records.first() {
            let created_provider = adapter::from_db_value_provider(record.clone());
            if let Err(e) = apply_config_to_file(&db, &created_provider.id).await {
                eprintln!("Failed to apply config after local save: {}", e);
            }
        }
    }

    let _ = app.emit("config-changed", "window");
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

    Ok(std::path::Path::new(&home_dir)
        .join(".claude")
        .join("config.json"))
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

// ============================================================================
// Claude Code Initialization Commands
// ============================================================================

/// Known fields in provider settings config (env section)

/// Initialize Claude provider from settings.json if database is empty
/// This function reads the settings.json file and imports its configuration
/// as a default provider if no providers exist in the database.
pub async fn init_claude_provider_from_settings(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(), String> {
    // Check if any providers exist by querying for one record
    let check_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM claude_provider LIMIT 1")
        .await
        .map_err(|e| format!("Failed to check providers: {}", e))?
        .take(0);

    let has_providers = match check_result {
        Ok(records) => !records.is_empty(),
        Err(_) => false,
    };

    if has_providers {
        // Already have providers, skip initialization
        return Ok(());
    }

    // Read settings.json
    let config_path_str = get_claude_config_path()?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        // No settings file, nothing to import
        return Ok(());
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;

    let settings: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse settings file: {}", e))?;

    // Check if settings has env section with ANTHROPIC fields
    let settings_obj = match settings.as_object() {
        Some(obj) => obj,
        None => return Ok(()), // Not a valid object, skip
    };

    let env_obj = match settings_obj.get("env").and_then(|v| v.as_object()) {
        Some(env) => env,
        None => return Ok(()), // No env section, skip
    };

    // Check if there are any ANTHROPIC fields
    let has_anthropic_config = env_obj.keys().any(|k| k.starts_with("ANTHROPIC_"));
    if !has_anthropic_config {
        return Ok(()); // No ANTHROPIC config, skip
    }

    // Extract provider-specific fields from env
    let mut provider_env = serde_json::Map::new();
    let mut common_env = serde_json::Map::new();

    for (key, value) in env_obj {
        if KNOWN_ENV_FIELDS.contains(&key.as_str()) {
            provider_env.insert(key.clone(), value.clone());
        } else {
            common_env.insert(key.clone(), value.clone());
        }
    }

    // Extract other known provider fields and build provider settings
    let mut provider_settings = serde_json::Map::new();

    // Build env section for provider (convert ANTHROPIC_MODEL back to model, etc.)
    let mut provider_env_for_settings = serde_json::Map::new();
    // 兼容旧版本：优先使用 ANTHROPIC_AUTH_TOKEN，如果没有则使用 ANTHROPIC_API_KEY
    let api_key = provider_env
        .get("ANTHROPIC_AUTH_TOKEN")
        .or_else(|| provider_env.get("ANTHROPIC_API_KEY"));
    if let Some(key) = api_key {
        provider_env_for_settings.insert("ANTHROPIC_AUTH_TOKEN".to_string(), key.clone());
    }
    if let Some(base_url) = provider_env.get("ANTHROPIC_BASE_URL") {
        provider_env_for_settings.insert("ANTHROPIC_BASE_URL".to_string(), base_url.clone());
    }
    provider_settings.insert(
        "env".to_string(),
        serde_json::json!(provider_env_for_settings),
    );

    // Convert ANTHROPIC_MODEL -> model, etc.
    if let Some(model) = provider_env.get("ANTHROPIC_MODEL") {
        provider_settings.insert("model".to_string(), model.clone());
    }
    if let Some(haiku) = provider_env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL") {
        provider_settings.insert("haikuModel".to_string(), haiku.clone());
    }
    if let Some(sonnet) = provider_env.get("ANTHROPIC_DEFAULT_SONNET_MODEL") {
        provider_settings.insert("sonnetModel".to_string(), sonnet.clone());
    }
    if let Some(opus) = provider_env.get("ANTHROPIC_DEFAULT_OPUS_MODEL") {
        provider_settings.insert("opusModel".to_string(), opus.clone());
    }
    if let Some(reasoning) = provider_env.get("ANTHROPIC_REASONING_MODEL") {
        provider_settings.insert("reasoningModel".to_string(), reasoning.clone());
    }

    // Build common config with unknown fields
    let mut common_config = serde_json::Map::new();

    // Add non-env fields to common config
    for (key, value) in settings_obj {
        if key != "env" {
            common_config.insert(key.clone(), value.clone());
        }
    }

    // Add unknown env fields to common config's env
    if !common_env.is_empty() {
        common_config.insert("env".to_string(), serde_json::json!(common_env));
    }

    // Save common config if not empty
    if !common_config.is_empty() {
        let common_json = serde_json::to_string(&common_config)
            .map_err(|e| format!("Failed to serialize common config: {}", e))?;

        let common_db_data = adapter::to_db_value_common(&common_json);

        // Use UPSERT to create if not exists, update if exists
        db.query("UPSERT claude_common_config:`common` CONTENT $data")
            .bind(("data", common_db_data))
            .await
            .map_err(|e| format!("Failed to save common config: {}", e))?;
    }

    // Create default provider
    let now = Local::now().to_rfc3339();
    let provider_name = "默认配置";

    let content = ClaudeCodeProviderContent {
        name: provider_name.to_string(),
        category: String::new(),
        settings_config: serde_json::to_string(&provider_settings)
            .map_err(|e| format!("Failed to serialize provider settings: {}", e))?,
        source_provider_id: None,
        website_url: None,
        notes: Some("从 settings.json 自动导入".to_string()),
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    // Create new provider with auto-generated random ID
    db.query("CREATE claude_provider CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create default provider: {}", e))?;

    println!("✅ Imported Claude Code settings from settings.json as default provider");

    Ok(())
}

// ============================================================================
// Claude Code Onboarding Commands
// ============================================================================

/// Get the Claude MCP config path (~/.claude.json)
fn get_claude_mcp_config_path() -> Result<std::path::PathBuf, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    Ok(std::path::Path::new(&home_dir).join(".claude.json"))
}

/// Get Claude onboarding status
/// Returns true if hasCompletedOnboarding is set to true in ~/.claude.json
#[tauri::command]
pub async fn get_claude_onboarding_status() -> Result<bool, String> {
    let config_path = get_claude_mcp_config_path()?;

    if !config_path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    let status = value
        .get("hasCompletedOnboarding")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Ok(status)
}

/// Skip Claude Code initial setup confirmation
/// Writes hasCompletedOnboarding=true to ~/.claude.json
#[tauri::command]
pub async fn apply_claude_onboarding_skip() -> Result<bool, String> {
    let config_path = get_claude_mcp_config_path()?;

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
        }
    }

    // Read existing config or create empty object
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

    // Check if already set
    let already = obj
        .get("hasCompletedOnboarding")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if already {
        return Ok(false);
    }

    // Set hasCompletedOnboarding = true
    obj.insert(
        "hasCompletedOnboarding".to_string(),
        serde_json::Value::Bool(true),
    );

    // Write back to file
    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(&config_path, format!("{serialized}\n"))
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(true)
}

/// Restore Claude Code initial setup confirmation
/// Removes hasCompletedOnboarding field from ~/.claude.json
#[tauri::command]
pub async fn clear_claude_onboarding_skip() -> Result<bool, String> {
    let config_path = get_claude_mcp_config_path()?;

    if !config_path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let mut obj: serde_json::Map<String, serde_json::Value> =
        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => return Ok(false),
        };

    // Check if field exists
    let existed = obj.remove("hasCompletedOnboarding").is_some();

    if !existed {
        return Ok(false);
    }

    // Write back to file
    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(&config_path, format!("{serialized}\n"))
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(true)
}

#[tauri::command]
pub async fn list_claude_all_api_hub_providers(
    state: tauri::State<'_, DbState>,
) -> Result<ClaudeAllApiHubProvidersResult, String> {
    let _ = state;
    let discovery = all_api_hub::list_provider_candidates()?;

    let providers = discovery
        .providers
        .iter()
        .map(|candidate| ClaudeAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            npm: Some(candidate.npm.clone()),
            base_url: candidate.base_url.clone(),
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
            provider_config: serde_json::to_value(all_api_hub::candidate_to_opencode_provider(
                candidate,
            ))
            .unwrap_or_else(|_| serde_json::json!({})),
        })
        .collect();

    Ok(ClaudeAllApiHubProvidersResult {
        found: discovery.found,
        profiles: discovery.profiles,
        providers,
        message: discovery.message,
    })
}

#[tauri::command]
pub async fn resolve_claude_all_api_hub_providers(
    state: tauri::State<'_, DbState>,
    request: ResolveClaudeAllApiHubProvidersRequest,
) -> Result<Vec<ClaudeAllApiHubProvider>, String> {
    let providers =
        all_api_hub::resolve_provider_candidates_with_keys(&state, &request.provider_ids).await?;

    Ok(providers
        .iter()
        .map(|candidate| ClaudeAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            npm: Some(candidate.npm.clone()),
            base_url: candidate.base_url.clone(),
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
            provider_config: serde_json::to_value(all_api_hub::candidate_to_opencode_provider(
                candidate,
            ))
            .unwrap_or_else(|_| serde_json::json!({})),
        })
        .collect())
}
