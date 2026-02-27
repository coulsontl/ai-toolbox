use chrono::Local;
use std::fs;
use serde_json::Value;

use crate::db::DbState;
use crate::coding::db_id::db_record_id;
use super::adapter;
use super::types::*;
use tauri::Emitter;

// ============================================================================
// Oh My OpenCode Slim Config Commands
// ============================================================================

/// List all oh-my-opencode-slim configs ordered by name
#[tauri::command]
pub async fn list_oh_my_opencode_slim_configs(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<OhMyOpenCodeSlimConfig>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_slim_config")
        .await
        .map_err(|e| format!("Failed to query configs: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            // 如果数据库为空，尝试从本地配置文件加载临时配置（不写入数据库）
            if records.is_empty() {
                if let Ok(temp_config) = load_temp_config_from_file() {
                    return Ok(vec![temp_config]);
                }
            }

            let mut result: Vec<OhMyOpenCodeSlimConfig> = records
                .into_iter()
                .map(adapter::from_db_value)
                .collect();
            // Sort by sort_index (if set), then by name as fallback
            result.sort_by(|a, b| {
                match (a.sort_index, b.sort_index) {
                    (Some(ai), Some(bi)) => ai.cmp(&bi),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                }
            });
            Ok(result)
        }
        Err(e) => {
            eprintln!("Failed to deserialize configs: {}", e);
            // Try to load from local file as fallback
            if let Ok(temp_config) = load_temp_config_from_file() {
                return Ok(vec![temp_config]);
            }
            Ok(Vec::new())
        }
    }
}

/// Helper function to get oh-my-opencode-slim config path
/// omos 只支持 .json 格式（不支持 jsonc）
pub fn get_oh_my_opencode_slim_config_path() -> Result<std::path::PathBuf, String> {
    let home_dir = dirs::home_dir()
        .ok_or("Failed to get home directory")?;

    let opencode_dir = home_dir.join(".config").join("opencode");
    let json_path = opencode_dir.join("oh-my-opencode-slim.json");

    Ok(json_path)
}

/// Load a temporary config from local file without writing to database
/// This is used when the database is empty and we want to show the local config
/// Returns a config with id "__local__" to indicate it's from local file
fn load_temp_config_from_file() -> Result<OhMyOpenCodeSlimConfig, String> {
    let config_path = get_oh_my_opencode_slim_config_path()
        .map_err(|_| "Local config file not found".to_string())?;

    if !config_path.exists() {
        return Err("No config file found".to_string());
    }

    // 读取文件内容
    let file_content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read local config file: {}", e))?;

    // 解析 JSON（使用 json5 支持带注释的 JSONC 格式）
    let json_value: Value = json5::from_str(&file_content)
        .map_err(|e| format!("Failed to parse local config file: {}", e))?;

    // 提取 agents 配置
    let agents = json_value
        .get("agents")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // 提取 other_fields（除了 agents 和全局配置字段之外的所有字段）
    let mut other_fields = json_value.clone();
    if let Some(obj) = other_fields.as_object_mut() {
        obj.remove("agents");
        obj.remove("$schema");
        // 移除属于 Global Config 的字段
        obj.remove("sisyphus_agent");
        obj.remove("sisyphusAgent");
        obj.remove("disabled_agents");
        obj.remove("disabledAgents");
        obj.remove("disabled_mcps");
        obj.remove("disabledMcps");
        obj.remove("disabled_hooks");
        obj.remove("disabledHooks");
        obj.remove("lsp");
        obj.remove("experimental");
    }

    let other_fields_value = if other_fields.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        None
    } else {
        Some(other_fields)
    };

    let now = Local::now().to_rfc3339();
    Ok(OhMyOpenCodeSlimConfig {
        id: "__local__".to_string(), // Special ID to indicate this is from local file
        name: "default".to_string(),
        is_applied: true,
        is_disabled: false,
        agents,
        other_fields: other_fields_value,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    })
}

/// Load a temporary global config from local file without writing to database
/// Returns a config with id "__local__" to indicate it's from local file
fn load_temp_global_config_from_file() -> Result<OhMyOpenCodeSlimGlobalConfig, String> {
    let config_path = get_oh_my_opencode_slim_config_path()
        .map_err(|_| "Local config file not found".to_string())?;

    if !config_path.exists() {
        return Err("No config file found".to_string());
    }

    let file_content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read local config file: {}", e))?;

    let json_value: Value = json5::from_str(&file_content)
        .map_err(|e| format!("Failed to parse local config file: {}", e))?;

    // 提取全局配置字段
    let sisyphus_agent = json_value
        .get("sisyphus_agent")
        .or_else(|| json_value.get("sisyphusAgent"))
        .cloned();

    let disabled_agents: Option<Vec<String>> = json_value
        .get("disabled_agents")
        .or_else(|| json_value.get("disabledAgents"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let disabled_mcps: Option<Vec<String>> = json_value
        .get("disabled_mcps")
        .or_else(|| json_value.get("disabledMcps"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let disabled_hooks: Option<Vec<String>> = json_value
        .get("disabled_hooks")
        .or_else(|| json_value.get("disabledHooks"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let lsp = json_value.get("lsp").cloned();
    let experimental = json_value.get("experimental").cloned();

    // 提取 other_fields（除了已知字段之外的所有字段）
    let mut other_fields = json_value.clone();
    if let Some(obj) = other_fields.as_object_mut() {
        obj.remove("agents");
        obj.remove("$schema");
        obj.remove("sisyphus_agent");
        obj.remove("sisyphusAgent");
        obj.remove("disabled_agents");
        obj.remove("disabledAgents");
        obj.remove("disabled_mcps");
        obj.remove("disabledMcps");
        obj.remove("disabled_hooks");
        obj.remove("disabledHooks");
        obj.remove("lsp");
        obj.remove("experimental");
    }

    let other_fields_value = if other_fields.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        None
    } else {
        Some(other_fields)
    };

    let now = Local::now().to_rfc3339();
    Ok(OhMyOpenCodeSlimGlobalConfig {
        id: "__local__".to_string(), // Special ID to indicate this is from local file
        sisyphus_agent,
        disabled_agents,
        disabled_mcps,
        disabled_hooks,
        lsp,
        experimental,
        other_fields: other_fields_value,
        updated_at: Some(now),
    })
}

/// Create a new oh-my-opencode-slim config
#[tauri::command]
pub async fn create_oh_my_opencode_slim_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimConfigInput,
) -> Result<OhMyOpenCodeSlimConfig, String> {
    let db = state.0.lock().await;

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenCodeSlimConfigContent {
        name: input.name.clone(),
        is_applied: false,
        is_disabled: false,
        agents: input.agents.clone(),
        other_fields: input.other_fields.clone(),
        sort_index: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let json_data = adapter::to_db_value(&content);

    db.query("CREATE oh_my_opencode_slim_config CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create config: {}", e))?;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_slim_config ORDER BY created_at DESC LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query new config: {}", e))?
        .take(0);

    let _ = app.emit("config-changed", "window");

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::from_db_value(record.clone()))
            } else {
                Err("Failed to retrieve created config".to_string())
            }
        }
        Err(e) => Err(format!("Failed to create config: {}", e)),
    }
}

/// Update an existing oh-my-opencode-slim config
#[tauri::command]
#[allow(unused_variables)]
pub async fn update_oh_my_opencode_slim_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimConfigInput,
) -> Result<OhMyOpenCodeSlimConfig, String> {
    let db = state.0.lock().await;

    let config_id = input.id.ok_or_else(|| "ID is required for update".to_string())?;

    let record_id = db_record_id("oh_my_opencode_slim_config", &config_id);
    let check_result: Result<Vec<Value>, _> = db
        .query(&format!("SELECT * FROM {} LIMIT 1", record_id))
        .await
        .map_err(|e| format!("Failed to check config existence: {}", e))?
        .take(0);

    if let Ok(records) = check_result {
        if records.is_empty() {
            return Err(format!(
                "Oh-my-opencode-slim config with ID '{}' not found",
                config_id
            ));
        }
    }

    let now = Local::now().to_rfc3339();

    let existing_result: Result<Vec<serde_json::Value>, _> = db
        .query(format!(
            "SELECT created_at, type::bool(is_applied) as is_applied, sort_index FROM oh_my_opencode_slim_config:`{}` LIMIT 1",
            config_id
        ))
        .await
        .map_err(|e| format!("Failed to query config: {}", e))?
        .take(0);

    let (is_applied_value, is_disabled_value, created_at, sort_index_value) = match existing_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                let is_applied = record
                    .get("is_applied")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let is_disabled = record
                    .get("is_disabled")
                    .or_else(|| record.get("isDisabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let created = record
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| Local::now().to_rfc3339());
                let sort_index = record
                    .get("sort_index")
                    .or_else(|| record.get("sortIndex"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                (is_applied, is_disabled, created, sort_index)
            } else {
                (false, false, Local::now().to_rfc3339(), None)
            }
        }
        Err(_) => {
            (false, false, Local::now().to_rfc3339(), None)
        }
    };

    let content = OhMyOpenCodeSlimConfigContent {
        name: input.name,
        is_applied: is_applied_value,
        is_disabled: is_disabled_value,
        agents: input.agents,
        other_fields: input.other_fields,
        sort_index: sort_index_value,
        created_at,
        updated_at: now,
    };

    let json_data = adapter::to_db_value(&content);

    let json_str = serde_json::to_string(&json_data)
        .map_err(|e| format!("Failed to serialize json_data: {}", e))?;

    db.query(format!("UPDATE oh_my_opencode_slim_config:`{}` CONTENT {}", config_id, json_str))
        .await
        .map_err(|e| format!("Failed to update config: {}", e))?;

    if is_applied_value {
        if let Err(e) = apply_config_to_file(&db, &config_id).await {
            eprintln!("Failed to auto-apply updated config: {}", e);
        } else {
            #[cfg(target_os = "windows")]
            let _ = app.emit("wsl-sync-request-opencode", ());
        }
    }

    Ok(OhMyOpenCodeSlimConfig {
        id: config_id,
        name: content.name,
        is_applied: is_applied_value,
        is_disabled: content.is_disabled,
        agents: content.agents,
        other_fields: content.other_fields,
        sort_index: sort_index_value,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    })
}

/// Delete an oh-my-opencode-slim config
#[tauri::command]
pub async fn delete_oh_my_opencode_slim_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query(format!("DELETE oh_my_opencode_slim_config:`{}`", id))
        .await
        .map_err(|e| format!("Failed to delete config: {}", e))?;

    let _ = app.emit("config-changed", "window");

    Ok(())
}

/// 内部函数：将指定配置应用到配置文件
async fn apply_config_to_file(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    config_id: &str,
) -> Result<(), String> {
    apply_config_to_file_public(db, config_id).await
}

/// Public version of apply_config_to_file for tray module
pub async fn apply_config_to_file_public(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    config_id: &str,
) -> Result<(), String> {
    let records_result: Result<Vec<Value>, _> = db
        .query(format!(
            "SELECT *, type::string(id) as id FROM oh_my_opencode_slim_config:`{}` LIMIT 1",
            config_id
        ))
        .await
        .map_err(|e| format!("Failed to query config: {}", e))?
        .take(0);

    let agents_profile = match records_result {
        Ok(records) => {
            if records.is_empty() {
                return Err(format!("Config '{}' not found", config_id));
            }
            adapter::from_db_value(records[0].clone())
        }
        Err(e) => return Err(format!("Failed to get config: {}", e)),
    };

    // Check if config is disabled
    if agents_profile.is_disabled {
        return Err(format!("Config '{}' is disabled and cannot be applied", config_id));
    }

    let config_path = get_oh_my_opencode_slim_config_path()?;

    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create opencode config directory: {}", e))?;
        }
    }

    // 获取 Global Config
    let global_records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_slim_global_config:`global` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query global config: {}", e))?
        .take(0);

    let global_config = match global_records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::global_config_from_db_value(record.clone())
            } else {
                OhMyOpenCodeSlimGlobalConfig {
                    id: "global".to_string(),
                    sisyphus_agent: None,
                    disabled_agents: None,
                    disabled_mcps: None,
                    disabled_hooks: None,
                    lsp: None,
                    experimental: None,
                    other_fields: None,
                    updated_at: None,
                }
            }
        }
        Err(_) => {
            OhMyOpenCodeSlimGlobalConfig {
                id: "global".to_string(),
                sisyphus_agent: None,
                disabled_agents: None,
                disabled_mcps: None,
                disabled_hooks: None,
                lsp: None,
                experimental: None,
                other_fields: None,
                updated_at: None,
            }
        }
    };

    let mut final_json = serde_json::Map::new();

    // omos 不需要 $schema 字段

    if let Some(sisyphus) = global_config.sisyphus_agent {
        final_json.insert("sisyphus_agent".to_string(), sisyphus);
    }
    if let Some(disabled_agents) = global_config.disabled_agents {
        final_json.insert("disabled_agents".to_string(), serde_json::json!(disabled_agents));
    }
    if let Some(disabled_mcps) = global_config.disabled_mcps {
        final_json.insert("disabled_mcps".to_string(), serde_json::json!(disabled_mcps));
    }
    if let Some(disabled_hooks) = global_config.disabled_hooks {
        final_json.insert("disabled_hooks".to_string(), serde_json::json!(disabled_hooks));
    }
    if let Some(lsp) = global_config.lsp {
        final_json.insert("lsp".to_string(), lsp);
    }
    if let Some(experimental) = global_config.experimental {
        final_json.insert("experimental".to_string(), experimental);
    }

    if let Some(global_others) = global_config.other_fields {
        if let Some(others_obj) = global_others.as_object() {
            for (key, value) in others_obj {
                final_json.insert(key.clone(), value.clone());
            }
        }
    }

    if let Some(agents) = agents_profile.agents {
        final_json.insert("agents".to_string(), agents);
    }

    if let Some(profile_others) = agents_profile.other_fields {
        if let Some(others_obj) = profile_others.as_object() {
            for (key, value) in others_obj {
                final_json.insert(key.clone(), value.clone());
            }
        }
    }

    let mut final_json = Value::Object(final_json);

    adapter::clean_empty_values(&mut final_json);

    let json_content = serde_json::to_string_pretty(&final_json)
        .map_err(|e| format!("Failed to serialize final config: {}", e))?;

    fs::write(&config_path, json_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}

/// Apply an oh-my-opencode-slim config to the JSON file
#[tauri::command]
pub async fn apply_oh_my_opencode_slim_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    apply_config_internal(&db, &app, &config_id, false).await?;
    Ok(())
}

/// Internal function to apply config
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_config_to_file(db, config_id).await?;

    let now = Local::now().to_rfc3339();

    db.query("UPDATE oh_my_opencode_slim_config SET is_applied = false, updated_at = $now WHERE is_applied = true")
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to clear applied flags: {}", e))?;

    let record_id = db_record_id("oh_my_opencode_slim_config", config_id);
    db.query(&format!("UPDATE {} SET is_applied = true, updated_at = $now", record_id))
        .bind(("now", now))
        .await
        .map_err(|e| format!("Failed to update applied flag: {}", e))?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

/// Reorder oh-my-opencode-slim configs
#[tauri::command]
pub async fn reorder_oh_my_opencode_slim_configs(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.0.lock().await;

    for (index, id) in ids.iter().enumerate() {
        db.query(format!(
            "UPDATE oh_my_opencode_slim_config:`{}` SET sort_index = $index",
            id
        ))
        .bind(("index", index as i32))
        .await
        .map_err(|e| format!("Failed to update sort index: {}", e))?;
    }

    Ok(())
}

/// Get oh-my-opencode-slim config file path info
#[tauri::command]
pub async fn get_oh_my_opencode_slim_config_path_info() -> Result<ConfigPathInfo, String> {
    let config_path = get_oh_my_opencode_slim_config_path()?;
    let path = config_path.to_string_lossy().to_string();

    Ok(ConfigPathInfo {
        path,
        source: "default".to_string(),
    })
}

/// Check if local oh-my-opencode-slim config file exists
/// omos 只支持 .json 格式
#[tauri::command]
pub async fn check_oh_my_opencode_slim_config_exists() -> Result<bool, String> {
    let home_dir = dirs::home_dir()
        .ok_or("Failed to get home directory")?;

    let opencode_dir = home_dir.join(".config").join("opencode");
    let json_path = opencode_dir.join("oh-my-opencode-slim.json");

    Ok(json_path.exists())
}

// ============================================================================
// Oh My OpenCode Slim Global Config Commands
// ============================================================================

/// Get oh-my-opencode-slim global config
#[tauri::command]
pub async fn get_oh_my_opencode_slim_global_config(
    state: tauri::State<'_, DbState>,
) -> Result<OhMyOpenCodeSlimGlobalConfig, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_slim_global_config:`global` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query global config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::global_config_from_db_value(record.clone()))
            } else {
                // 数据库为空，尝试从本地文件加载临时配置（不写入数据库）
                if let Ok(temp_config) = load_temp_global_config_from_file() {
                    return Ok(temp_config);
                }

                // 返回默认配置
                Ok(OhMyOpenCodeSlimGlobalConfig {
                    id: "global".to_string(),
                    sisyphus_agent: None,
                    disabled_agents: None,
                    disabled_mcps: None,
                    disabled_hooks: None,
                    lsp: None,
                    experimental: None,
                    other_fields: None,
                    updated_at: None,
                })
            }
        }
        Err(e) => {
            eprintln!("Failed to get global config: {}", e);
            // Try to load from local file as fallback
            if let Ok(temp_config) = load_temp_global_config_from_file() {
                return Ok(temp_config);
            }
            // 返回默认配置
            Ok(OhMyOpenCodeSlimGlobalConfig {
                id: "global".to_string(),
                sisyphus_agent: None,
                disabled_agents: None,
                disabled_mcps: None,
                disabled_hooks: None,
                lsp: None,
                experimental: None,
                other_fields: None,
                updated_at: None,
            })
        }
    }
}

/// Save oh-my-opencode-slim global config
#[tauri::command]
#[allow(unused_variables)]
pub async fn save_oh_my_opencode_slim_global_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimGlobalConfigInput,
) -> Result<OhMyOpenCodeSlimGlobalConfig, String> {
    let db = state.0.lock().await;

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenCodeSlimGlobalConfigContent {
        sisyphus_agent: input.sisyphus_agent,
        disabled_agents: input.disabled_agents,
        disabled_mcps: input.disabled_mcps,
        disabled_hooks: input.disabled_hooks,
        lsp: input.lsp,
        experimental: input.experimental,
        other_fields: input.other_fields,
        updated_at: now.clone(),
    };

    let json_data = adapter::global_config_to_db_value(&content);

    db.query("UPSERT oh_my_opencode_slim_global_config:`global` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save global config: {}", e))?;

    let applied_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_slim_config WHERE is_applied = true LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query applied config: {}", e))?
        .take(0);

    if let Ok(records) = applied_result {
        if let Some(record) = records.first() {
            let applied_config = adapter::from_db_value(record.clone());
            if apply_config_to_file(&db, &applied_config.id).await.is_ok() {
                #[cfg(target_os = "windows")]
                let _ = app.emit("wsl-sync-request-opencode", ());
            }
        }
    }

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_slim_global_config:`global` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query saved global config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::global_config_from_db_value(record.clone()))
            } else {
                Err("Failed to retrieve saved global config".to_string())
            }
        }
        Err(e) => Err(format!("Failed to save global config: {}", e)),
    }
}

/// Toggle is_disabled status for a config
#[tauri::command]
pub async fn toggle_oh_my_opencode_slim_config_disabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Update is_disabled field in database
    let now = Local::now().to_rfc3339();
    db.query(format!(
        "UPDATE oh_my_opencode_slim_config:`{}` SET is_disabled = $is_disabled, updated_at = $now",
        config_id
    ))
    .bind(("is_disabled", is_disabled))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to toggle config disabled status: {}", e))?;

    // If this config is applied, re-apply config to update files
    let records_result: Result<Vec<Value>, _> = db
        .query(format!(
            "SELECT *, type::string(id) as id FROM oh_my_opencode_slim_config:`{}` LIMIT 1",
            config_id
        ))
        .await
        .map_err(|e| format!("Failed to query config: {}", e))?
        .take(0);

    if let Ok(records) = records_result {
        if let Some(config_value) = records.first() {
            let is_applied = adapter::get_bool_compat(config_value, "is_applied", "isApplied", false);
            if is_applied {
                // Re-apply config to update files (will check is_disabled internally)
                apply_config_internal(&db, &app, &config_id, false).await?;
            }
        }
    }

    Ok(())
}

/// Save local config (both Agents Profile and Global Config) into database
/// This is used when saving __local__ temporary config to database
/// Input can include config and/or globalConfig; missing parts will be loaded from local files
#[tauri::command]
pub async fn save_oh_my_opencode_slim_local_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimLocalConfigInput,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Load base config from local files
    let base_config = load_temp_config_from_file()?;
    let base_global = load_temp_global_config_from_file().ok();

    let now = Local::now().to_rfc3339();

    // Build Agents Profile content
    let config_input = input.config;
    let config_name = config_input
        .as_ref()
        .map(|c| c.name.clone())
        .unwrap_or(base_config.name);
    let config_agents = config_input
        .as_ref()
        .and_then(|c| c.agents.clone())
        .or(base_config.agents);
    let config_other_fields = config_input
        .as_ref()
        .and_then(|c| c.other_fields.clone())
        .or(base_config.other_fields);

    let config_content = OhMyOpenCodeSlimConfigContent {
        name: config_name,
        is_applied: true,
        is_disabled: false,
        agents: config_agents,
        other_fields: config_other_fields,
        sort_index: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let config_json = adapter::to_db_value(&config_content);
    db.query("CREATE oh_my_opencode_slim_config CONTENT $data")
        .bind(("data", config_json))
        .await
        .map_err(|e| format!("Failed to create config: {}", e))?;

    // Build Global Config content
    let global_input = input.global_config;
    let global_sisyphus_agent = global_input
        .as_ref()
        .and_then(|g| g.sisyphus_agent.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.sisyphus_agent.clone()));
    let global_disabled_agents = global_input
        .as_ref()
        .and_then(|g| g.disabled_agents.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.disabled_agents.clone()));
    let global_disabled_mcps = global_input
        .as_ref()
        .and_then(|g| g.disabled_mcps.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.disabled_mcps.clone()));
    let global_disabled_hooks = global_input
        .as_ref()
        .and_then(|g| g.disabled_hooks.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.disabled_hooks.clone()));
    let global_lsp = global_input
        .as_ref()
        .and_then(|g| g.lsp.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.lsp.clone()));
    let global_experimental = global_input
        .as_ref()
        .and_then(|g| g.experimental.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.experimental.clone()));
    let global_other_fields = global_input
        .as_ref()
        .and_then(|g| g.other_fields.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.other_fields.clone()));

    let global_content = OhMyOpenCodeSlimGlobalConfigContent {
        sisyphus_agent: global_sisyphus_agent,
        disabled_agents: global_disabled_agents,
        disabled_mcps: global_disabled_mcps,
        disabled_hooks: global_disabled_hooks,
        lsp: global_lsp,
        experimental: global_experimental,
        other_fields: global_other_fields,
        updated_at: now,
    };

    let global_json = adapter::global_config_to_db_value(&global_content);
    db.query("UPSERT oh_my_opencode_slim_global_config:`global` CONTENT $data")
        .bind(("data", global_json))
        .await
        .map_err(|e| format!("Failed to save global config: {}", e))?;

    // Re-apply config to files using the newly created config
    let created_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_slim_config ORDER BY created_at DESC LIMIT 1")
        .await
        .map_err(|e| format!("Failed to fetch created config: {}", e))?
        .take(0);

    if let Ok(records) = created_result {
        if let Some(record) = records.first() {
            let created_config = adapter::from_db_value(record.clone());
            if let Err(e) = apply_config_to_file(&db, &created_config.id).await {
                eprintln!("Failed to apply config after local save: {}", e);
            }
        }
    }

    let _ = app.emit("config-changed", "window");
    Ok(())
}
