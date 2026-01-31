use std::fs;
use std::path::Path;
use serde_json::Value;

use crate::db::DbState;
use super::adapter;
use super::types::*;
use tauri::Emitter;
use chrono::Local;

// ============================================================================
// Codex Config Path Commands
// ============================================================================

/// Get Codex config directory path (~/.codex/)
fn get_codex_config_dir() -> Result<std::path::PathBuf, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;
    
    Ok(Path::new(&home_dir).join(".codex"))
}

/// Get Codex auth.json path
fn get_codex_auth_path() -> Result<std::path::PathBuf, String> {
    Ok(get_codex_config_dir()?.join("auth.json"))
}

/// Get Codex config.toml path
fn get_codex_config_path() -> Result<std::path::PathBuf, String> {
    Ok(get_codex_config_dir()?.join("config.toml"))
}

/// Get Codex config directory path
#[tauri::command]
pub fn get_codex_config_dir_path() -> Result<String, String> {
    let config_dir = get_codex_config_dir()?;
    Ok(config_dir.to_string_lossy().to_string())
}

/// Get Codex config.toml file path
#[tauri::command]
pub fn get_codex_config_file_path() -> Result<String, String> {
    let config_path = get_codex_config_path()?;
    Ok(config_path.to_string_lossy().to_string())
}

/// Reveal Codex config folder in file explorer
#[tauri::command]
pub fn reveal_codex_config_folder() -> Result<(), String> {
    let config_dir = get_codex_config_dir()?;

    // Ensure directory exists
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create .codex directory: {}", e))?;
    }

    // Open in file explorer (platform-specific)
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}

// ============================================================================
// Codex Provider Commands
// ============================================================================

/// List all Codex providers ordered by sort_index
/// If database is empty, returns a temporary provider loaded from local config files
#[tauri::command]
pub async fn list_codex_providers(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<CodexProvider>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider")
        .await
        .map_err(|e| format!("Failed to query providers: {}", e))?
        .take(0);

match records_result {
        Ok(records) => {
            if records.is_empty() {
                // Database is empty, try to load from local files as temporary provider
                if let Ok(temp_provider) = load_temp_provider_from_files().await {
                    return Ok(vec![temp_provider]);
                }
                Ok(Vec::new())
            } else {
                let mut result: Vec<CodexProvider> = records
                    .into_iter()
                    .map(adapter::from_db_value_provider)
                    .collect();
                result.sort_by_key(|p| p.sort_index.unwrap_or(0));
                Ok(result)
            }
        }
        Err(e) => {
            eprintln!("Failed to deserialize providers: {}", e);
            // Try to load from local files as fallback
            if let Ok(temp_provider) = load_temp_provider_from_files().await {
                return Ok(vec![temp_provider]);
            }
            Ok(Vec::new())
        }
    }
}

/// 修复损坏的 Codex provider 数据
/// This is used when the database is empty and we want to show the local config
async fn load_temp_provider_from_files() -> Result<CodexProvider, String> {
    let auth_path = get_codex_auth_path()?;
    let config_path = get_codex_config_path()?;

    if !auth_path.exists() && !config_path.exists() {
        return Err("No config files found".to_string());
    }

    // Read auth.json (optional)
    let auth: serde_json::Value = if auth_path.exists() {
        let auth_content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        serde_json::from_str(&auth_content)
            .map_err(|e| format!("Failed to parse auth.json: {}", e))?
    } else {
        serde_json::json!({})
    };

    // Read config.toml (optional)
    let config_toml = if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Build settings_config
    let settings = serde_json::json!({
        "auth": auth,
        "config": config_toml
    });

    let now = Local::now().to_rfc3339();
    Ok(CodexProvider {
        id: "__local__".to_string(), // Special ID to indicate this is from local files
        name: "本地配置".to_string(),
        category: "custom".to_string(),
        settings_config: serde_json::to_string(&settings).unwrap_or_default(),
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

/// 修复损坏的 Codex provider 数据
/// 删除所有 provider 记录，需要重新创建
#[tauri::command]
pub async fn repair_codex_providers(
    state: tauri::State<'_, DbState>,
) -> Result<String, String> {
    let db = state.0.lock().await;
    
    db.query("DELETE codex_provider")
        .await
        .map_err(|e| format!("Failed to delete providers: {}", e))?;
    
    Ok("All Codex providers have been deleted. Please recreate them.".to_string())
}

/// Create a new Codex provider
#[tauri::command]
pub async fn create_codex_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider: CodexProviderInput,
) -> Result<CodexProvider, String> {
    let db = state.0.lock().await;

    let now = Local::now().to_rfc3339();
    let content = CodexProviderContent {
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
        is_disabled: provider.is_disabled.unwrap_or(false),
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    // Create new provider - SurrealDB auto-generates record ID
    db.query("CREATE codex_provider CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    // Fetch the created record to get the auto-generated ID
    let result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider ORDER BY created_at DESC LIMIT 1")
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

/// Update an existing Codex provider
#[tauri::command]
pub async fn update_codex_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider: CodexProvider,
) -> Result<CodexProvider, String> {
    let db = state.0.lock().await;

    // Use the id from frontend (pure string id without table prefix)
    let id = provider.id.clone();
    let now = Local::now().to_rfc3339();

    // Get existing record to preserve created_at
    // Use type::thing to convert string id to Thing for proper comparison
    let existing_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM codex_provider WHERE id = type::thing('codex_provider', $id) LIMIT 1")
        .bind(("id", id.clone()))
        .await
        .map_err(|e| format!("Failed to query existing provider: {}", e))?
        .take(0);

    // Check if provider exists
    if let Ok(records) = &existing_result {
        if records.is_empty() {
            return Err(format!("Codex provider with ID '{}' not found", id));
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

    let content = CodexProviderContent {
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
    db.query(format!("UPDATE codex_provider:`{}` CONTENT $data", id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to update provider: {}", e))?;

    // If this provider is applied, re-apply to config file
    if content.is_applied {
        if let Err(e) = apply_config_to_file(&db, &id).await {
            eprintln!("Failed to auto-apply updated config: {}", e);
        }
    }

    // Notify frontend and tray to refresh
    let _ = app.emit("config-changed", "window");

        Ok(CodexProvider {
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

/// Delete a Codex provider
#[tauri::command]
pub async fn delete_codex_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query(format!("DELETE codex_provider:`{}`", id))
        .await
        .map_err(|e| format!("Failed to delete codex provider: {}", e))?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Reorder Codex providers
/// 使用 DELETE + CREATE 模式避免 SurrealDB MVCC 版本控制问题
#[tauri::command]
pub async fn reorder_codex_providers(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.0.lock().await;
    let now = Local::now().to_rfc3339();

    for (index, id) in ids.iter().enumerate() {
        // 首先获取现有记录
        let existing_result: Result<Vec<Value>, _> = db
            .query("SELECT *, type::string(id) as id FROM codex_provider WHERE id = type::thing('codex_provider', $id) LIMIT 1")
            .bind(("id", id.clone()))
            .await
            .map_err(|e| format!("Failed to query provider {}: {}", id, e))?
            .take(0);

        if let Ok(records) = existing_result {
            if let Some(record) = records.first() {
                // 构建更新后的内容
                let content = CodexProviderContent {
                    name: record.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    category: record.get("category").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    settings_config: record.get("settings_config").and_then(|v| v.as_str()).unwrap_or("{}").to_string(),
                    source_provider_id: record.get("source_provider_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    website_url: record.get("website_url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    notes: record.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    icon: record.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    icon_color: record.get("icon_color").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    sort_index: Some(index as i32),
                    is_applied: record.get("is_applied").and_then(|v| v.as_bool()).unwrap_or(false),
                    is_disabled: record
                        .get("is_disabled")
                        .or_else(|| record.get("isDisabled"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    created_at: record.get("created_at").and_then(|v| v.as_str()).unwrap_or(&now).to_string(),
                    updated_at: now.clone(),
                };

                let json_data = adapter::to_db_value_provider(&content);

                // Use Blind Write pattern with native ID format
                db.query(format!("UPDATE codex_provider:`{}` CONTENT $data", id))
                    .bind(("data", json_data))
                    .await
                    .map_err(|e| format!("Failed to update provider {}: {}", id, e))?;
            }
        }
    }

    Ok(())
}

/// Select a Codex provider (mark as applied in database)
/// 使用 DELETE + CREATE 模式避免 SurrealDB MVCC 版本控制问题
#[tauri::command]
pub async fn select_codex_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    update_is_applied_status(&db, &id).await?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Internal function: update is_applied status
/// Use UPDATE with WHERE to avoid SurrealDB MVCC version control issues
async fn update_is_applied_status(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    target_id: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    let target_id = target_id.to_string(); // Clone for bind

    // Clear current applied status (only update the currently applied one)
    db.query("UPDATE codex_provider SET is_applied = false, updated_at = $now WHERE is_applied = true")
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to clear applied status: {}", e))?;

    // Set target provider as applied
    db.query("UPDATE codex_provider SET is_applied = true, updated_at = $now WHERE id = type::thing('codex_provider', $id)")
        .bind(("id", target_id))
        .bind(("now", now))
        .await
        .map_err(|e| format!("Failed to set applied status: {}", e))?;

    Ok(())
}

// ============================================================================
// Codex Config File Commands
// ============================================================================

/// Internal function: apply provider config to files
async fn apply_config_to_file(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_to_file_public(db, provider_id).await
}

/// Public version for tray module
pub async fn apply_config_to_file_public(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {
    // Get the provider
    let provider_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider WHERE id = type::thing('codex_provider', $id) LIMIT 1")
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
        Err(e) => return Err(format!("Failed to deserialize provider: {}", e)),
    };

    // Check if provider is disabled
    if provider.is_disabled {
        return Err(format!("Provider '{}' is disabled and cannot be applied", provider_id));
    }

    // Parse provider settings_config
    let provider_config: serde_json::Value = serde_json::from_str(&provider.settings_config)
        .map_err(|e| format!("Failed to parse provider config: {}", e))?;

    // Get common config
    let common_config_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM codex_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query common config: {}", e))?
        .take(0);

    let common_toml: Option<String> = match common_config_result {
        Ok(records) => records.first().and_then(|r| {
            r.get("config").and_then(|v| v.as_str()).map(|s| s.to_string())
        }),
        Err(_) => None,
    };

    // Extract auth and config
    let auth = provider_config.get("auth").cloned().unwrap_or(serde_json::json!({}));
    let config_toml = provider_config
        .get("config")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Append common config to provider config
    let final_config = if let Some(common) = common_toml {
        if !common.trim().is_empty() {
            append_toml_configs(&config_toml, &common)?
        } else {
            config_toml
        }
    } else {
        config_toml
    };

    write_codex_config_files(&auth, &final_config)?;
    Ok(())
}

/// Append common TOML config to provider config (common is appended after provider)
fn append_toml_configs(provider: &str, common: &str) -> Result<String, String> {
    let provider_content = provider.trim();
    let common_content = common.trim();

    if provider_content.is_empty() {
        return Ok(common_content.to_string());
    }
    if common_content.is_empty() {
        return Ok(provider_content.to_string());
    }

    // Add a blank line between provider and common config
    Ok(format!("{}\n\n{}", provider_content, common_content))
}

/// Write auth.json and config.toml files
fn write_codex_config_files(auth: &serde_json::Value, config_toml: &str) -> Result<(), String> {
    let config_dir = get_codex_config_dir()?;

    // Ensure directory exists
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create .codex directory: {}", e))?;
    }

    // Write auth.json (full overwrite is OK for auth)
    let auth_path = config_dir.join("auth.json");
    let auth_content = serde_json::to_string_pretty(auth)
        .map_err(|e| format!("Failed to serialize auth: {}", e))?;
    fs::write(&auth_path, auth_content)
        .map_err(|e| format!("Failed to write auth.json: {}", e))?;

    // Write config.toml with partial update (preserve mcp_servers)
    let config_path = config_dir.join("config.toml");
    write_codex_config_toml_preserve_mcp(&config_path, config_toml)?;

    Ok(())
}

/// Write config.toml while preserving mcp_servers and other unrelated fields
fn write_codex_config_toml_preserve_mcp(config_path: &std::path::Path, new_config: &str) -> Result<(), String> {
    use toml_edit::DocumentMut;

    // Parse new config
    let new_doc: DocumentMut = if new_config.trim().is_empty() {
        DocumentMut::new()
    } else {
        new_config.parse()
            .map_err(|e| format!("Failed to parse new config: {}", e))?
    };

    // Read existing config (if exists)
    let mut existing_doc: DocumentMut = if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("Failed to read config.toml: {}", e))?;
        if content.trim().is_empty() {
            DocumentMut::new()
        } else {
            content.parse()
                .map_err(|e| format!("Failed to parse existing config.toml: {}", e))?
        }
    } else {
        DocumentMut::new()
    };

    // Preserve mcp_servers from existing config
    let preserved_mcp = existing_doc.get("mcp_servers").cloned();

    // Replace all fields from new config
    for (key, value) in new_doc.iter() {
        existing_doc[key] = value.clone();
    }

    // Restore preserved mcp_servers (if it was present and not in new config)
    if let Some(mcp) = preserved_mcp {
        if !new_doc.contains_key("mcp_servers") {
            existing_doc["mcp_servers"] = mcp;
        }
    }

    // Write back with #:schema none header
    let doc_content = existing_doc.to_string();
    let final_content = if doc_content.trim_start().starts_with("#:schema") {
        doc_content
    } else {
        format!("#:schema none\n{}", doc_content)
    };
    fs::write(config_path, final_content)
        .map_err(|e| format!("Failed to write config.toml: {}", e))?;

    Ok(())
}

/// Apply Codex config to files
#[tauri::command]
pub async fn apply_codex_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    apply_config_internal(&db, &app, &provider_id, false).await
}

/// Toggle is_disabled status for a provider
#[tauri::command]
pub async fn toggle_codex_provider_disabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Update is_disabled field in database
    let now = Local::now().to_rfc3339();
    db.query(format!(
        "UPDATE codex_provider:`{}` SET is_disabled = $is_disabled, updated_at = $now",
        provider_id
    ))
    .bind(("is_disabled", is_disabled))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to toggle provider disabled status: {}", e))?;

    // If this provider is applied and now disabled, re-apply config to update files
    let provider: Option<Value> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider WHERE id = type::thing('codex_provider', $id)")
        .bind(("id", provider_id.clone()))
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

/// Internal function to apply config
/// 使用 DELETE + CREATE 模式避免 SurrealDB MVCC 版本控制问题
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    // Apply config to files
    apply_config_to_file(db, provider_id).await?;

    // Update is_applied status using DELETE + CREATE pattern
    update_is_applied_status(db, provider_id).await?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-codex", ());

    Ok(())
}

/// Read current Codex settings from files
#[tauri::command]
pub async fn read_codex_settings() -> Result<CodexSettings, String> {
    let auth_path = get_codex_auth_path()?;
    let config_path = get_codex_config_path()?;

    let auth = if auth_path.exists() {
        let content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse auth.json: {}", e))?
    } else {
        None
    };

    let config = if config_path.exists() {
        Some(fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.toml: {}", e))?)
    } else {
        None
    };

    Ok(CodexSettings { auth, config })
}

// ============================================================================
// Codex Common Config Commands
// ============================================================================

/// Get Codex common config
/// If database is empty, returns empty config (Codex doesn't have common config in local files)
#[tauri::command]
pub async fn get_codex_common_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<CodexCommonConfig>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query common config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(Some(adapter::from_db_value_common(record.clone())))
            } else {
                // Database is empty, return None (Codex doesn't have common config in local files)
                Ok(None)
            }
        }
        Err(e) => {
            // 反序列化失败，删除旧数据以修复版本冲突
            eprintln!("⚠️ Codex common config has incompatible format, cleaning up: {}", e);
            let _ = db.query("DELETE codex_common_config:`common`").await;
            Ok(None)
        }
    }
}

/// Save Codex common config
#[tauri::command]
pub async fn save_codex_common_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Validate TOML if not empty
    if !config.trim().is_empty() {
        let _: toml::Table = toml::from_str(&config)
            .map_err(|e| format!("Invalid TOML: {}", e))?;
    }

    let json_data = adapter::to_db_value_common(&config);

    // Use UPSERT to handle both update and create
    db.query("UPSERT codex_common_config:`common` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Re-apply current provider config to write merged config to file
    let applied_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider WHERE is_applied = true LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query applied provider: {}", e))?
        .take(0);

    if let Ok(records) = applied_result {
        if let Some(record) = records.first() {
            let provider = adapter::from_db_value_provider(record.clone());
            if let Err(e) = apply_config_to_file(&db, &provider.id).await {
                eprintln!("Failed to re-apply config: {}", e);
            }
        }
    }

    // Emit config-changed event to notify frontend
    let _ = app.emit("config-changed", "window");

    Ok(())
}

/// Save local config (provider and/or common) into database
/// Input can include provider and/or commonConfig; missing parts will be loaded from local files
#[tauri::command]
pub async fn save_codex_local_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: CodexLocalConfigInput,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Load base provider from local files
    let base_provider = load_temp_provider_from_files().await?;

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
    let provider_is_disabled = provider_input
        .as_ref()
        .and_then(|p| p.is_disabled)
        .unwrap_or(false);

    let common_config = input.common_config.unwrap_or_default();

    let now = Local::now().to_rfc3339();
    let provider_content = CodexProviderContent {
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
        is_disabled: provider_is_disabled,
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_json = adapter::to_db_value_provider(&provider_content);
    db.query("CREATE codex_provider CONTENT $data")
        .bind(("data", provider_json))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    let common_json = adapter::to_db_value_common(&common_config);
    db.query("UPSERT codex_common_config:`common` CONTENT $data")
        .bind(("data", common_json))
        .await
        .map_err(|e| format!("Failed to save common config: {}", e))?;

    // Re-apply config to files using the newly created provider
    let created_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider ORDER BY created_at DESC LIMIT 1")
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
// Codex Initialization
// ============================================================================

/// Initialize Codex provider from existing config files
pub async fn init_codex_provider_from_settings(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(), String> {
    // Check if any providers exist by querying for one record
    let check_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM codex_provider LIMIT 1")
        .await
        .map_err(|e| format!("Failed to check providers: {}", e))?
        .take(0);

    let has_providers = match check_result {
        Ok(records) => !records.is_empty(),
        Err(_) => false,
    };

    if has_providers {
        return Ok(());
    }

    // Check if config files exist
    let auth_path = get_codex_auth_path()?;
    let config_path = get_codex_config_path()?;
    if !auth_path.exists() && !config_path.exists() {
        return Ok(());
    }

    // Read auth.json (optional)
    let auth: serde_json::Value = if auth_path.exists() {
        let auth_content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        serde_json::from_str(&auth_content)
            .map_err(|e| format!("Failed to parse auth.json: {}", e))?
    } else {
        serde_json::json!({})
    };

    // Read config.toml (optional)
    let config_toml = if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Build settings_config
    let settings = serde_json::json!({
        "auth": auth,
        "config": config_toml
    });

    let now = Local::now().to_rfc3339();
    let content = CodexProviderContent {
        name: "默认配置".to_string(),
        category: String::new(),
        settings_config: serde_json::to_string(&settings).unwrap_or_default(),
        source_provider_id: None,
        website_url: None,
        notes: Some("从配置文件自动导入".to_string()),
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
    db.query("CREATE codex_provider CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    println!("✅ Imported Codex settings as default provider");
    Ok(())
}
