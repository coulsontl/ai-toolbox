use chrono::Local;
use std::fs;
use serde_json::Value;

use crate::db::DbState;
use crate::coding::db_id::db_record_id;
use super::adapter;
use super::types::*;
use tauri::Emitter;

/// Normalize agent key to lowercase for backward compatibility
fn normalize_agent_key(key: &str) -> String {
    key.to_lowercase()
}

/// Normalize all keys in an agents object to lowercase
fn normalize_agents_keys(agents: &mut Value) {
    if let Some(obj) = agents.as_object_mut() {
        let keys: Vec<String> = obj.keys().cloned().collect();
        for key in keys {
            let normalized = normalize_agent_key(&key);
            if normalized != key {
                if let Some(value) = obj.remove(&key) {
                    if obj.contains_key(&normalized) {
                        log::warn!("[OhMyOpenCode] Agent key conflict: '{}' normalized to '{}' which already exists, overwriting", key, normalized);
                    }
                    obj.insert(normalized, value);
                }
            }
        }
    }
}

// ============================================================================
// Oh My OpenCode Config Commands
// ============================================================================

/// List all oh-my-opencode configs ordered by name
#[tauri::command]
pub async fn list_oh_my_opencode_configs(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<OhMyOpenCodeConfig>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_config")
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

            let mut result: Vec<OhMyOpenCodeConfig> = records
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

/// Helper function to get oh-my-opencode config path
/// Priority: .jsonc (if exists) → .json (if exists) → default .jsonc
pub fn get_oh_my_opencode_config_path() -> Result<std::path::PathBuf, String> {
    let home_dir = dirs::home_dir()
        .ok_or("Failed to get home directory")?;

    let opencode_dir = home_dir.join(".config").join("opencode");

    // Check for .jsonc first, then .json
    let jsonc_path = opencode_dir.join("oh-my-opencode.jsonc");
    let json_path = opencode_dir.join("oh-my-opencode.json");

    if jsonc_path.exists() {
        Ok(jsonc_path)
    } else if json_path.exists() {
        Ok(json_path)
    } else {
        // Return default path for new file
        Ok(jsonc_path)
    }
}

/// Load a temporary config from local file without writing to database
/// This is used when the database is empty and we want to show the local config
/// Returns a config with id "__local__" to indicate it's from local file
fn load_temp_config_from_file() -> Result<OhMyOpenCodeConfig, String> {
    let config_path = get_oh_my_opencode_config_path()
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

    // 提取 agents 配置（并标准化键名为小写）
    let mut agents = json_value
        .get("agents")
        .and_then(|v| serde_json::from_value::<Value>(v.clone()).ok());
    if let Some(ref mut agents_value) = agents {
        normalize_agents_keys(agents_value);
    }

    let categories = json_value
        .get("categories")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // 提取 other_fields（除了 agents 和全局配置字段之外的所有字段）
    let mut other_fields = json_value.clone();
    if let Some(obj) = other_fields.as_object_mut() {
        obj.remove("agents");
        obj.remove("categories");
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
        obj.remove("disabled_skills");
        obj.remove("disabledSkills");
        obj.remove("lsp");
        obj.remove("experimental");
        obj.remove("background_task");
        obj.remove("backgroundTask");
        obj.remove("browser_automation_engine");
        obj.remove("browserAutomationEngine");
        obj.remove("claude_code");
        obj.remove("claudeCode");
    }

    let other_fields_value = if other_fields.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        None
    } else {
        Some(other_fields)
    };

    let now = Local::now().to_rfc3339();
    Ok(OhMyOpenCodeConfig {
        id: "__local__".to_string(), // Special ID to indicate this is from local file
        name: "default".to_string(),
        is_applied: true,
        is_disabled: false,
        agents,
        categories,
        other_fields: other_fields_value,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    })
}

/// Load a temporary global config from local file without writing to database
/// Returns a config with id "__local__" to indicate it's from local file
fn load_temp_global_config_from_file() -> Result<OhMyOpenCodeGlobalConfig, String> {
    let config_path = get_oh_my_opencode_config_path()
        .map_err(|_| "Local config file not found".to_string())?;

    if !config_path.exists() {
        return Err("No config file found".to_string());
    }

    let file_content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read local config file: {}", e))?;

    let json_value: Value = json5::from_str(&file_content)
        .map_err(|e| format!("Failed to parse local config file: {}", e))?;

    // 提取全局配置字段
    let schema = json_value
        .get("$schema")
        .and_then(|v| v.as_str())
        .map(String::from);

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

    let disabled_skills: Option<Vec<String>> = json_value
        .get("disabled_skills")
        .or_else(|| json_value.get("disabledSkills"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let lsp = json_value.get("lsp").cloned();
    let experimental = json_value.get("experimental").cloned();
    let background_task = json_value
        .get("background_task")
        .or_else(|| json_value.get("backgroundTask"))
        .cloned();
    let browser_automation_engine = json_value
        .get("browser_automation_engine")
        .or_else(|| json_value.get("browserAutomationEngine"))
        .cloned();
    let claude_code = json_value
        .get("claude_code")
        .or_else(|| json_value.get("claudeCode"))
        .cloned();

    // 提取 other_fields（除了已知字段之外的所有字段）
    let mut other_fields = json_value.clone();
    if let Some(obj) = other_fields.as_object_mut() {
        obj.remove("agents");
        obj.remove("categories");
        obj.remove("$schema");
        obj.remove("sisyphus_agent");
        obj.remove("sisyphusAgent");
        obj.remove("disabled_agents");
        obj.remove("disabledAgents");
        obj.remove("disabled_mcps");
        obj.remove("disabledMcps");
        obj.remove("disabled_hooks");
        obj.remove("disabledHooks");
        obj.remove("disabled_skills");
        obj.remove("disabledSkills");
        obj.remove("lsp");
        obj.remove("experimental");
        obj.remove("background_task");
        obj.remove("backgroundTask");
        obj.remove("browser_automation_engine");
        obj.remove("browserAutomationEngine");
        obj.remove("claude_code");
        obj.remove("claudeCode");
    }

    let other_fields_value = if other_fields.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        None
    } else {
        Some(other_fields)
    };

    let now = Local::now().to_rfc3339();
    Ok(OhMyOpenCodeGlobalConfig {
        id: "__local__".to_string(), // Special ID to indicate this is from local file
        schema,
        sisyphus_agent,
        disabled_agents,
        disabled_mcps,
        disabled_hooks,
        disabled_skills,
        lsp,
        experimental,
        background_task,
        browser_automation_engine,
        claude_code,
        other_fields: other_fields_value,
        updated_at: Some(now),
    })
}

/// Create a new oh-my-opencode config
#[tauri::command]
pub async fn create_oh_my_opencode_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeConfigInput,
) -> Result<OhMyOpenCodeConfig, String> {
    let db = state.0.lock().await;

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenCodeConfigContent {
        name: input.name.clone(),
        is_applied: false,
        is_disabled: false,
        agents: input.agents.clone(),
        categories: input.categories.clone(),
        other_fields: input.other_fields.clone(),
        sort_index: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let json_data = adapter::to_db_value(&content);

    // Use CREATE to let SurrealDB auto-generate ID (like ClaudeCode)
    db.query("CREATE oh_my_opencode_config CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create config: {}", e))?;

    // Fetch the created record to get the auto-generated ID
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_config ORDER BY created_at DESC LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query new config: {}", e))?
        .take(0);

    // Notify to refresh tray menu
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

/// Update an existing oh-my-opencode config
#[tauri::command]
#[allow(unused_variables)] // app 在 Windows 平台上用于 WSL 同步
pub async fn update_oh_my_opencode_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeConfigInput,
) -> Result<OhMyOpenCodeConfig, String> {
    let db = state.0.lock().await;

    // ID is required for update
    let config_id = input.id.ok_or_else(|| "ID is required for update".to_string())?;

    // Check if config exists using backtick-escaped record ref
    let record_id = db_record_id("oh_my_opencode_config", &config_id);
    let check_result: Result<Vec<Value>, _> = db
        .query(&format!("SELECT * FROM {} LIMIT 1", record_id))
        .await
        .map_err(|e| format!("Failed to check config existence: {}", e))?
        .take(0);

    if let Ok(records) = check_result {
        if records.is_empty() {
            return Err(format!(
                "Oh-my-opencode config with ID '{}' not found",
                config_id
            ));
        }
    }

    let now = Local::now().to_rfc3339();

    // Get the existing config to preserve created_at and is_applied
    // Use direct ID format like ClaudeCode does to avoid type::thing serialization issues
    // Only select the fields we need to avoid enum type issues
    let existing_result: Result<Vec<serde_json::Value>, _> = db
        .query(format!(
            "SELECT created_at, type::bool(is_applied) as is_applied, sort_index FROM oh_my_opencode_config:`{}` LIMIT 1",
            config_id
        ))
        .await
        .map_err(|e| format!("Failed to query config: {}", e))?
        .take(0);

    // Extract fields from the query result
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

    let content = OhMyOpenCodeConfigContent {
        name: input.name,
        is_applied: is_applied_value,
        is_disabled: is_disabled_value,
        agents: input.agents,
        categories: input.categories,
        other_fields: input.other_fields,
        sort_index: sort_index_value,
        created_at,
        updated_at: now,
    };

    let json_data = adapter::to_db_value(&content);

    // Inline JSON into query to avoid SurrealDB parameter binding serialization issues
    // This is necessary because SurrealDB may have enum<bool> type issues with is_applied field
    let json_str = serde_json::to_string(&json_data)
        .map_err(|e| format!("Failed to serialize json_data: {}", e))?;
    
    db.query(format!("UPDATE oh_my_opencode_config:`{}` CONTENT {}", config_id, json_str))
        .await
        .map_err(|e| format!("Failed to update config: {}", e))?;

    // 如果该配置当前是应用状态，立即重新写入到配置文件
    if is_applied_value {
        if let Err(e) = apply_config_to_file(&db, &config_id).await {
            eprintln!("Failed to auto-apply updated config: {}", e);
            // 不中断更新流程，只记录错误
        } else {
            // Trigger WSL sync via event (Windows only)
            #[cfg(target_os = "windows")]
            let _ = app.emit("wsl-sync-request-opencode", ());
        }
    }

    // Return the config we just wrote - no need to query it back
    // This avoids any potential enum serialization issues from SurrealDB
    Ok(OhMyOpenCodeConfig {
        id: config_id,
        name: content.name,
        is_applied: is_applied_value,
        is_disabled: content.is_disabled,
        agents: content.agents,
        categories: content.categories,
        other_fields: content.other_fields,
        sort_index: sort_index_value,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    })
}

/// Delete an oh-my-opencode config
#[tauri::command]
pub async fn delete_oh_my_opencode_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query(format!("DELETE oh_my_opencode_config:`{}`", id))
        .await
        .map_err(|e| format!("Failed to delete config: {}", e))?;

    // Notify to refresh tray menu
    let _ = app.emit("config-changed", "window");

    Ok(())
}

/// 内部函数：将指定配置应用到配置文件（不改变数据库中的 is_applied 状态）
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
    // Get the config from database using direct ID format (like ClaudeCode)
    let records_result: Result<Vec<Value>, _> = db
        .query(format!(
            "SELECT *, type::string(id) as id FROM oh_my_opencode_config:`{}` LIMIT 1",
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

    // Check if config is disabled (P0-3 fix: Architect solution C)
    if agents_profile.is_disabled {
        return Err(format!("Config '{}' is disabled and cannot be applied", config_id));
    }

    // Get config path using unified function
    let config_path = get_oh_my_opencode_config_path()?;

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create opencode config directory: {}", e))?;
        }
    }

    // 获取 Global Config
    let global_records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_global_config:`global` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query global config: {}", e))?
        .take(0);

    let global_config = match global_records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::global_config_from_db_value(record.clone())
            } else {
                // 使用默认空配置
                OhMyOpenCodeGlobalConfig {
                    id: "global".to_string(),
                    schema: None,
                    sisyphus_agent: None,
                    disabled_agents: None,
                    disabled_mcps: None,
                    disabled_hooks: None,
                    disabled_skills: None,
                    lsp: None,
                    experimental: None,
                    background_task: None,
                    browser_automation_engine: None,
                    claude_code: None,
                    other_fields: None,
                    updated_at: None,
                }
            }
        }
        Err(_) => {
            // 使用默认空配置
            OhMyOpenCodeGlobalConfig {
                id: "global".to_string(),
                schema: None,
                sisyphus_agent: None,
                disabled_agents: None,
                disabled_mcps: None,
                disabled_hooks: None,
                disabled_skills: None,
                lsp: None,
                experimental: None,
                background_task: None,
                browser_automation_engine: None,
                claude_code: None,
                other_fields: None,
                updated_at: None,
            }
        }
    };

    // 合并配置的优先级顺序（从低到高）：
    // 1. 全局配置的明确字段（最低优先级）
    // 2. 全局配置的 other_fields
    // 3. Agents Profile 的 agents
    // 4. Agents Profile 的 categories
    // 5. Agents Profile 的 other_fields（最高优先级，可以覆盖所有）

    let mut final_json = serde_json::Map::new();

    // 使用保存的 schema 或默认 schema
    let schema_url = global_config.schema
        .unwrap_or_else(|| "https://raw.githubusercontent.com/code-yeongyu/oh-my-opencode/master/assets/oh-my-opencode.schema.json".to_string());
    final_json.insert("$schema".to_string(), serde_json::json!(schema_url));

    // 1. 先设置全局配置的明确字段（优先级最低）
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
    if let Some(disabled_skills) = global_config.disabled_skills {
        final_json.insert("disabled_skills".to_string(), serde_json::json!(disabled_skills));
    }
    if let Some(lsp) = global_config.lsp {
        final_json.insert("lsp".to_string(), lsp);
    }
    if let Some(experimental) = global_config.experimental {
        final_json.insert("experimental".to_string(), experimental);
    }
    if let Some(background_task) = global_config.background_task {
        final_json.insert("background_task".to_string(), background_task);
    }
    if let Some(browser_automation_engine) = global_config.browser_automation_engine {
        final_json.insert("browser_automation_engine".to_string(), browser_automation_engine);
    }
    if let Some(claude_code) = global_config.claude_code {
        final_json.insert("claude_code".to_string(), claude_code);
    }

    // 2. 然后平铺全局配置的 other_fields（会覆盖上面的明确字段）
    if let Some(global_others) = global_config.other_fields {
        if let Some(others_obj) = global_others.as_object() {
            for (key, value) in others_obj {
                final_json.insert(key.clone(), value.clone());
            }
        }
    }

    // 3. 设置 Agents Profile 的 agents（会覆盖前面的 agents，并标准化键名为小写）
    if let Some(mut agents) = agents_profile.agents {
        normalize_agents_keys(&mut agents);
        final_json.insert("agents".to_string(), agents);
    }

    // 4. 设置 Agents Profile 的 categories（会覆盖前面的 categories）
    if let Some(categories) = agents_profile.categories {
        final_json.insert("categories".to_string(), categories);
    }

    // 5. 最后平铺 Agents Profile 的 other_fields（最高优先级，可以覆盖所有字段）
    if let Some(profile_others) = agents_profile.other_fields {
        if let Some(others_obj) = profile_others.as_object() {
            for (key, value) in others_obj {
                final_json.insert(key.clone(), value.clone());
            }
        }
    }

    let mut final_json = Value::Object(final_json);

    // 清理空值：删除空对象和空数组
    adapter::clean_empty_values(&mut final_json);

    // Write to file with pretty formatting
    let json_content = serde_json::to_string_pretty(&final_json)
        .map_err(|e| format!("Failed to serialize final config: {}", e))?;

    fs::write(&config_path, json_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}
/// Apply an oh-my-opencode config to the JSON file
#[tauri::command]
pub async fn apply_oh_my_opencode_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;
    apply_config_internal(&db, &app, &config_id, false).await?;
    Ok(())
}

/// Internal function to apply config: writes to file and updates database
/// This is the single source of truth for applying an Oh My OpenCode config
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    // 应用配置到文件
    apply_config_to_file(db, config_id).await?;

    // Update database - set all configs to not applied, then set this one to applied
    let now = Local::now().to_rfc3339();

    // Clear applied flag (only update the currently applied one)
    db.query("UPDATE oh_my_opencode_config SET is_applied = false, updated_at = $now WHERE is_applied = true")
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to clear applied flags: {}", e))?;

    // Set this config as applied using backtick-escaped record ref
    let record_id = db_record_id("oh_my_opencode_config", config_id);
    db.query(&format!("UPDATE {} SET is_applied = true, updated_at = $now", record_id))
        .bind(("now", now))
        .await
        .map_err(|e| format!("Failed to update applied flag: {}", e))?;

    // Notify based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

/// Reorder oh-my-opencode configs (by name for now)
#[tauri::command]
pub async fn reorder_oh_my_opencode_configs(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.0.lock().await;

    for (index, id) in ids.iter().enumerate() {
        db.query(format!(
            "UPDATE oh_my_opencode_config:`{}` SET sort_index = $index",
            id
        ))
        .bind(("index", index as i32))
        .await
        .map_err(|e| format!("Failed to update sort index: {}", e))?;
    }

    Ok(())
}

/// Toggle is_disabled status for a config
#[tauri::command]
pub async fn toggle_oh_my_opencode_config_disabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Update is_disabled field in database
    let now = Local::now().to_rfc3339();
    db.query(format!(
        "UPDATE oh_my_opencode_config:`{}` SET is_disabled = $is_disabled, updated_at = $now",
        config_id
    ))
    .bind(("is_disabled", is_disabled))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to toggle config disabled status: {}", e))?;

    // If this config is applied, re-apply config to update files
    let records_result: Result<Vec<Value>, _> = db
        .query(format!(
            "SELECT *, type::string(id) as id FROM oh_my_opencode_config:`{}` LIMIT 1",
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

/// Get oh-my-opencode config file path info
#[tauri::command]
pub async fn get_oh_my_opencode_config_path_info() -> Result<ConfigPathInfo, String> {
    let config_path = get_oh_my_opencode_config_path()?;
    let path = config_path.to_string_lossy().to_string();

    Ok(ConfigPathInfo {
        path,
        source: "default".to_string(),
    })
}

/// Check if local oh-my-opencode config file exists
/// Returns true if ~/.config/opencode/oh-my-opencode.jsonc or .json exists
#[tauri::command]
pub async fn check_oh_my_opencode_config_exists() -> Result<bool, String> {
    let home_dir = dirs::home_dir()
        .ok_or("Failed to get home directory")?;

    let opencode_dir = home_dir.join(".config").join("opencode");
    let jsonc_path = opencode_dir.join("oh-my-opencode.jsonc");
    let json_path = opencode_dir.join("oh-my-opencode.json");

    Ok(jsonc_path.exists() || json_path.exists())
}

// ============================================================================
// Oh My OpenCode Global Config Commands
// ============================================================================

/// Get oh-my-opencode global config (固定 ID 为 "global")
#[tauri::command]
pub async fn get_oh_my_opencode_global_config(
    state: tauri::State<'_, DbState>,
) -> Result<OhMyOpenCodeGlobalConfig, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_global_config:`global` LIMIT 1")
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
                Ok(OhMyOpenCodeGlobalConfig {
                    id: "global".to_string(),
                    schema: None,
                    sisyphus_agent: None,
                    disabled_agents: None,
                    disabled_mcps: None,
                    disabled_hooks: None,
                    disabled_skills: None,
                    lsp: None,
                    experimental: None,
                    background_task: None,
                    browser_automation_engine: None,
                    claude_code: None,
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
            Ok(OhMyOpenCodeGlobalConfig {
                id: "global".to_string(),
                schema: None,
                sisyphus_agent: None,
                disabled_agents: None,
                disabled_mcps: None,
                disabled_hooks: None,
                disabled_skills: None,
                lsp: None,
                experimental: None,
                background_task: None,
                browser_automation_engine: None,
                claude_code: None,
                other_fields: None,
                updated_at: None,
            })
        }
    }
}

/// Save oh-my-opencode global config
#[tauri::command]
#[allow(unused_variables)] // app 在 Windows 平台上用于 WSL 同步
pub async fn save_oh_my_opencode_global_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeGlobalConfigInput,
) -> Result<OhMyOpenCodeGlobalConfig, String> {
    let db = state.0.lock().await;

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenCodeGlobalConfigContent {
        schema: input.schema,
        sisyphus_agent: input.sisyphus_agent,
        disabled_agents: input.disabled_agents,
        disabled_mcps: input.disabled_mcps,
        disabled_hooks: input.disabled_hooks,
        disabled_skills: input.disabled_skills,
        lsp: input.lsp,
        experimental: input.experimental,
        background_task: input.background_task,
        browser_automation_engine: input.browser_automation_engine,
        claude_code: input.claude_code,
        other_fields: input.other_fields,
        updated_at: now.clone(),
    };

    let json_data = adapter::global_config_to_db_value(&content);

    // Use UPSERT to handle both update and create
    db.query("UPSERT oh_my_opencode_global_config:`global` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save global config: {}", e))?;

    // 查找当前应用的配置，如果存在则重新应用到文件
    let applied_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_config WHERE is_applied = true LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query applied config: {}", e))?
        .take(0);

    if let Ok(records) = applied_result {
        if let Some(record) = records.first() {
            let applied_config = adapter::from_db_value(record.clone());
            // 重新应用配置到文件（不改变数据库中的 is_applied 状态）
            if apply_config_to_file(&db, &applied_config.id).await.is_ok() {
                // Trigger WSL sync via event (Windows only)
                #[cfg(target_os = "windows")]
                let _ = app.emit("wsl-sync-request-opencode", ());
            }
        }
    }

    // 从数据库读取刚保存的配置
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_global_config:`global` LIMIT 1")
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

/// Save local config (both Agents Profile and Global Config) into database
/// This is used when saving __local__ temporary config to database
/// Input can include config and/or globalConfig; missing parts will be loaded from local files
#[tauri::command]
pub async fn save_oh_my_opencode_local_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeLocalConfigInput,
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
    let config_categories = config_input
        .as_ref()
        .and_then(|c| c.categories.clone())
        .or(base_config.categories);
    let config_other_fields = config_input
        .as_ref()
        .and_then(|c| c.other_fields.clone())
        .or(base_config.other_fields);

    let config_content = OhMyOpenCodeConfigContent {
        name: config_name,
        is_applied: true,
        is_disabled: false,
        agents: config_agents,
        categories: config_categories,
        other_fields: config_other_fields,
        sort_index: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let config_json = adapter::to_db_value(&config_content);
    db.query("CREATE oh_my_opencode_config CONTENT $data")
        .bind(("data", config_json))
        .await
        .map_err(|e| format!("Failed to create config: {}", e))?;

    // Build Global Config content
    let global_input = input.global_config;
    let global_schema = global_input
        .as_ref()
        .and_then(|g| g.schema.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.schema.clone()));
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
    let global_disabled_skills = global_input
        .as_ref()
        .and_then(|g| g.disabled_skills.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.disabled_skills.clone()));
    let global_lsp = global_input
        .as_ref()
        .and_then(|g| g.lsp.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.lsp.clone()));
    let global_experimental = global_input
        .as_ref()
        .and_then(|g| g.experimental.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.experimental.clone()));
    let global_background_task = global_input
        .as_ref()
        .and_then(|g| g.background_task.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.background_task.clone()));
    let global_browser_automation_engine = global_input
        .as_ref()
        .and_then(|g| g.browser_automation_engine.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.browser_automation_engine.clone()));
    let global_claude_code = global_input
        .as_ref()
        .and_then(|g| g.claude_code.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.claude_code.clone()));
    let global_other_fields = global_input
        .as_ref()
        .and_then(|g| g.other_fields.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.other_fields.clone()));

    let global_content = OhMyOpenCodeGlobalConfigContent {
        schema: global_schema,
        sisyphus_agent: global_sisyphus_agent,
        disabled_agents: global_disabled_agents,
        disabled_mcps: global_disabled_mcps,
        disabled_hooks: global_disabled_hooks,
        disabled_skills: global_disabled_skills,
        lsp: global_lsp,
        experimental: global_experimental,
        background_task: global_background_task,
        browser_automation_engine: global_browser_automation_engine,
        claude_code: global_claude_code,
        other_fields: global_other_fields,
        updated_at: now,
    };

    let global_json = adapter::global_config_to_db_value(&global_content);
    db.query("UPSERT oh_my_opencode_global_config:`global` CONTENT $data")
        .bind(("data", global_json))
        .await
        .map_err(|e| format!("Failed to save global config: {}", e))?;

    // Re-apply config to files using the newly created config
    let created_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_config ORDER BY created_at DESC LIMIT 1")
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
