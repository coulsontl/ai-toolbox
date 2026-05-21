use chrono::Local;
use serde_json::Value;
use std::fs;

use super::adapter;
use super::types::*;
use crate::coding::runtime_location;
use crate::db::helpers::{
    db_create, db_delete, db_get, db_list, db_patch_fields, db_put, db_query_by_bool,
    db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath};
use crate::db::SqliteDbState;
use tauri::Emitter;

pub const OH_MY_OPENAGENT_CONFIG_TABLE: &str = "oh_my_openagent_config";
pub const OH_MY_OPENAGENT_GLOBAL_CONFIG_TABLE: &str = "oh_my_openagent_global_config";

fn default_global_config() -> OhMyOpenAgentGlobalConfig {
    OhMyOpenAgentGlobalConfig {
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

fn list_configs_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<OhMyOpenAgentAgentsProfile>, String> {
    let mut configs = sqlite_state.with_conn(|conn| {
        db_list(conn, DbTable::OhMyOpenAgentConfig, None).map(|records| {
            records
                .into_iter()
                .map(adapter::from_db_value)
                .collect::<Vec<_>>()
        })
    })?;
    configs.sort_by(|a, b| match (a.sort_index, b.sort_index) {
        (Some(ai), Some(bi)) => ai.cmp(&bi),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.cmp(&b.name),
    });
    Ok(configs)
}

fn get_config_from_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
) -> Result<Option<OhMyOpenAgentAgentsProfile>, String> {
    sqlite_state.with_conn(|conn| {
        db_get(conn, DbTable::OhMyOpenAgentConfig, config_id)
            .map(|record| record.map(adapter::from_db_value))
    })
}

fn get_global_config_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Option<OhMyOpenAgentGlobalConfig>, String> {
    sqlite_state.with_conn(|conn| {
        db_get(conn, DbTable::OhMyOpenAgentGlobalConfig, "global")
            .map(|record| record.map(adapter::global_config_from_db_value))
    })
}

fn put_config_to_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
    data: &Value,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| db_put(conn, DbTable::OhMyOpenAgentConfig, config_id, data))
}

fn put_global_config_to_sqlite(sqlite_state: &SqliteDbState, data: &Value) -> Result<(), String> {
    sqlite_state.with_conn(|conn| db_put(conn, DbTable::OhMyOpenAgentGlobalConfig, "global", data))
}

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
                        log::warn!(
                            "[OhMyOpenAgent] Agent key conflict: '{}' normalized to '{}' which already exists, overwriting",
                            key,
                            normalized
                        );
                    }
                    obj.insert(normalized, value);
                }
            }
        }
    }
}

fn get_default_oh_my_openagent_dir() -> Result<std::path::PathBuf, String> {
    let home_dir = dirs::home_dir().ok_or("Failed to get home directory")?;
    Ok(home_dir.join(".config").join("opencode"))
}

fn get_default_oh_my_openagent_path_candidates() -> Result<Vec<std::path::PathBuf>, String> {
    let default_dir = get_default_oh_my_openagent_dir()?;
    Ok(vec![
        default_dir.join("oh-my-openagent.jsonc"),
        default_dir.join("oh-my-openagent.json"),
        default_dir.join("oh-my-opencode.jsonc"),
        default_dir.join("oh-my-opencode.json"),
    ])
}

async fn get_oh_my_openagent_config_path_and_source(
    db: &crate::db::SqliteDbState,
) -> Result<(std::path::PathBuf, &'static str), String> {
    let path = runtime_location::get_omo_config_path_async(db).await?;
    let default_candidates = get_default_oh_my_openagent_path_candidates()?;
    let source = if default_candidates
        .iter()
        .any(|candidate| candidate == &path)
    {
        "default"
    } else {
        "custom"
    };
    Ok((path, source))
}

// ============================================================================
// Oh My OpenAgent Config Commands
// ============================================================================

/// List all Oh My OpenAgent configs ordered by name
#[tauri::command]
pub async fn list_oh_my_openagent_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OhMyOpenAgentAgentsProfile>, String> {
    let db = state.db();

    let configs = list_configs_from_sqlite(db)?;
    if configs.is_empty() {
        if let Ok(temp_config) = load_temp_config_from_file(db).await {
            return Ok(vec![temp_config]);
        }
    }
    Ok(configs)
}

/// Resolve the Oh My OpenAgent config path with legacy filename compatibility.
/// Priority: canonical .jsonc/.json -> legacy .jsonc/.json -> canonical .jsonc default
pub async fn get_oh_my_openagent_config_path(
    db: &crate::db::SqliteDbState,
) -> Result<std::path::PathBuf, String> {
    let (config_path, _) = get_oh_my_openagent_config_path_and_source(db).await?;
    Ok(config_path)
}

/// Load a temporary config from local file without writing to database
/// This is used when the database is empty and we want to show the local config
/// Returns a config with id "__local__" to indicate it's from local file
async fn load_temp_config_from_file(
    db: &crate::db::SqliteDbState,
) -> Result<OhMyOpenAgentAgentsProfile, String> {
    let config_path = get_oh_my_openagent_config_path(db)
        .await
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

    let other_fields_value = if other_fields
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true)
    {
        None
    } else {
        Some(other_fields)
    };

    let now = Local::now().to_rfc3339();
    Ok(OhMyOpenAgentAgentsProfile {
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
async fn load_temp_global_config_from_file(
    db: &crate::db::SqliteDbState,
) -> Result<OhMyOpenAgentGlobalConfig, String> {
    let config_path = get_oh_my_openagent_config_path(db)
        .await
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

    let other_fields_value = if other_fields
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true)
    {
        None
    } else {
        Some(other_fields)
    };

    let now = Local::now().to_rfc3339();
    Ok(OhMyOpenAgentGlobalConfig {
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

/// Create a new Oh My OpenAgent config
#[tauri::command]
pub async fn create_oh_my_openagent_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenAgentAgentsProfileInput,
) -> Result<OhMyOpenAgentAgentsProfile, String> {
    let db = state.db();

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenAgentAgentsProfileContent {
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

    let created = db.with_conn(|conn| db_create(conn, DbTable::OhMyOpenAgentConfig, &json_data))?;
    let _ = app.emit("config-changed", "window");
    Ok(adapter::from_db_value(created))
}

/// Update an existing Oh My OpenAgent config
#[tauri::command]
#[allow(unused_variables)] // app 在 Windows 平台上用于 WSL 同步
pub async fn update_oh_my_openagent_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenAgentAgentsProfileInput,
) -> Result<OhMyOpenAgentAgentsProfile, String> {
    let db = state.db();
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;

    let existing_config = get_config_from_sqlite(db, &config_id)?
        .ok_or_else(|| format!("Oh My OpenAgent config with ID '{}' not found", config_id))?;
    let now = Local::now().to_rfc3339();
    let created_at = existing_config
        .created_at
        .clone()
        .unwrap_or_else(|| Local::now().to_rfc3339());

    let content = OhMyOpenAgentAgentsProfileContent {
        name: input.name,
        is_applied: existing_config.is_applied,
        is_disabled: existing_config.is_disabled,
        agents: input.agents,
        categories: input.categories,
        other_fields: input.other_fields,
        sort_index: existing_config.sort_index,
        created_at,
        updated_at: now,
    };

    let json_data = adapter::to_db_value(&content);
    put_config_to_sqlite(db, &config_id, &json_data)?;

    if existing_config.is_applied {
        if let Err(e) = apply_config_to_file(&db, &config_id).await {
            eprintln!("Failed to auto-apply updated config: {}", e);
        } else {
            #[cfg(target_os = "windows")]
            let _ = app.emit("wsl-sync-request-opencode", ());
        }
    }

    Ok(OhMyOpenAgentAgentsProfile {
        id: config_id,
        name: content.name,
        is_applied: existing_config.is_applied,
        is_disabled: content.is_disabled,
        agents: content.agents,
        categories: content.categories,
        other_fields: content.other_fields,
        sort_index: content.sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    })
}

/// Delete an Oh My OpenAgent config
#[tauri::command]
pub async fn delete_oh_my_openagent_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    db.with_conn(|conn| db_delete(conn, DbTable::OhMyOpenAgentConfig, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Clear the currently applied runtime config file without deleting the saved profile.
#[tauri::command]
pub async fn clear_oh_my_openagent_applied_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    if config_id == "__local__" {
        return Err("Local config must be saved before clearing the runtime file".to_string());
    }

    let db = state.db();

    let config = get_config_from_sqlite(db, &config_id)?
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;
    if !config.is_applied {
        return Err(format!("Config '{}' is not currently applied", config_id));
    }

    let config_path = get_oh_my_openagent_config_path(&db).await?;

    #[cfg(target_os = "windows")]
    crate::coding::wsl::remove_auto_synced_wsl_mapping_target(state.inner(), "opencode-oh-my")
        .await?;

    if config_path.exists() {
        fs::remove_file(&config_path)
            .map_err(|e| format!("Failed to remove config file: {}", e))?;
    }

    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::OhMyOpenAgentConfig, None, &now)
    })?;

    let _ = app.emit("config-changed", "window");

    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

/// 内部函数：将指定配置应用到配置文件（不改变数据库中的 is_applied 状态）
async fn apply_config_to_file(
    db: &crate::db::SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    apply_config_to_file_public(db, config_id).await
}

/// Public version of apply_config_to_file for tray module
pub async fn apply_config_to_file_public(
    db: &crate::db::SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    // Get the config from database using direct ID format (like ClaudeCode)
    let agents_profile = get_config_from_sqlite(db, config_id)?
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;

    // Check if config is disabled (P0-3 fix: Architect solution C)
    if agents_profile.is_disabled {
        return Err(format!(
            "Config '{}' is disabled and cannot be applied",
            config_id
        ));
    }

    // Get config path using unified function
    let config_path = get_oh_my_openagent_config_path(db).await?;

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create opencode config directory: {}", e))?;
        }
    }

    // 获取 Global Config
    let global_config = get_global_config_from_sqlite(db)?.unwrap_or_else(default_global_config);

    // 合并配置的优先级顺序（从低到高）：
    // 1. 全局配置的明确字段（最低优先级）
    // 2. 全局配置的 other_fields
    // 3. Agents Profile 的 agents
    // 4. Agents Profile 的 categories
    // 5. Agents Profile 的 other_fields（最高优先级，可以覆盖所有）

    let mut final_json = serde_json::Map::new();

    // 使用保存的 schema 或默认 schema
    let schema_url = global_config.schema.unwrap_or_else(|| {
        "https://raw.githubusercontent.com/code-yeongyu/oh-my-openagent/dev/assets/oh-my-opencode.schema.json".to_string()
    });
    final_json.insert("$schema".to_string(), serde_json::json!(schema_url));

    // 1. 先设置全局配置的明确字段（优先级最低）
    if let Some(sisyphus) = global_config.sisyphus_agent {
        final_json.insert("sisyphus_agent".to_string(), sisyphus);
    }
    if let Some(disabled_agents) = global_config.disabled_agents {
        final_json.insert(
            "disabled_agents".to_string(),
            serde_json::json!(disabled_agents),
        );
    }
    if let Some(disabled_mcps) = global_config.disabled_mcps {
        final_json.insert(
            "disabled_mcps".to_string(),
            serde_json::json!(disabled_mcps),
        );
    }
    if let Some(disabled_hooks) = global_config.disabled_hooks {
        final_json.insert(
            "disabled_hooks".to_string(),
            serde_json::json!(disabled_hooks),
        );
    }
    if let Some(disabled_skills) = global_config.disabled_skills {
        final_json.insert(
            "disabled_skills".to_string(),
            serde_json::json!(disabled_skills),
        );
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
        final_json.insert(
            "browser_automation_engine".to_string(),
            browser_automation_engine,
        );
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
/// Apply an Oh My OpenAgent config to the JSON file
#[tauri::command]
pub async fn apply_oh_my_openagent_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    apply_config_internal(&db, &app, &config_id, false).await?;
    Ok(())
}

/// Internal function to apply config: writes to file and updates database
/// This is the single source of truth for applying an Oh My OpenAgent config
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &crate::db::SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    // 应用配置到文件
    apply_config_to_file(db, config_id).await?;

    // Update database - set all configs to not applied, then set this one to applied
    let now = Local::now().to_rfc3339();

    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::OhMyOpenAgentConfig, Some(config_id), &now)
    })?;

    // Notify based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

/// Reorder Oh My OpenAgent configs (by name for now)
#[tauri::command]
pub async fn reorder_oh_my_openagent_configs(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::OhMyOpenAgentConfig,
                id,
                &[("sort_index", serde_json::json!(index as i32))],
            )
            .map(|_| ())
        })?;
    }

    Ok(())
}

/// Toggle is_disabled status for a config
#[tauri::command]
pub async fn toggle_oh_my_openagent_config_disabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let config_value = db
        .with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::OhMyOpenAgentConfig,
                &config_id,
                &[
                    ("is_disabled", Value::Bool(is_disabled)),
                    ("updated_at", Value::String(now.clone())),
                ],
            )
        })?
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;

    let is_applied = adapter::get_bool_compat(&config_value, "is_applied", "isApplied", false);
    if is_applied {
        apply_config_internal(&db, &app, &config_id, false).await?;
    }

    Ok(())
}

/// Get Oh My OpenAgent config file path info
#[tauri::command]
pub async fn get_oh_my_openagent_config_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ConfigPathInfo, String> {
    let db = state.db();
    let (config_path, source) = get_oh_my_openagent_config_path_and_source(&db).await?;
    let path = config_path.to_string_lossy().to_string();

    Ok(ConfigPathInfo {
        path,
        source: source.to_string(),
    })
}

/// Check if a local Oh My OpenAgent config file exists
#[tauri::command]
pub async fn check_oh_my_openagent_config_exists(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<bool, String> {
    let db = state.db();
    let config_path = get_oh_my_openagent_config_path(&db).await?;
    Ok(config_path.exists())
}

// ============================================================================
// Oh My OpenAgent Global Config Commands
// ============================================================================

/// Get Oh My OpenAgent global config (固定 ID 为 "global")
#[tauri::command]
pub async fn get_oh_my_openagent_global_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OhMyOpenAgentGlobalConfig, String> {
    let db = state.db();
    if let Some(config) = get_global_config_from_sqlite(db)? {
        return Ok(config);
    }
    if let Ok(temp_config) = load_temp_global_config_from_file(db).await {
        return Ok(temp_config);
    }
    Ok(default_global_config())
}

/// Save Oh My OpenAgent global config
#[tauri::command]
#[allow(unused_variables)] // app 在 Windows 平台上用于 WSL 同步
pub async fn save_oh_my_openagent_global_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenAgentGlobalConfigInput,
) -> Result<OhMyOpenAgentGlobalConfig, String> {
    let db = state.db();

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenAgentGlobalConfigContent {
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

    put_global_config_to_sqlite(db, &json_data)?;

    let applied_configs = db.with_conn(|conn| {
        db_query_by_bool(
            conn,
            DbTable::OhMyOpenAgentConfig,
            &JsonFieldPath::new("is_applied")?,
            true,
            None,
            Some(1),
        )
    })?;

    if let Some(record) = applied_configs.first() {
        let applied_config = adapter::from_db_value(record.clone());
        if apply_config_to_file(&db, &applied_config.id).await.is_ok() {
            #[cfg(target_os = "windows")]
            let _ = app.emit("wsl-sync-request-opencode", ());
        }
    }

    get_global_config_from_sqlite(db)?
        .ok_or_else(|| "Failed to retrieve saved global config".to_string())
}

/// Save local config (both Agents Profile and Global Config) into database
/// This is used when saving __local__ temporary config to database
/// Input can include config and/or globalConfig; missing parts will be loaded from local files
#[tauri::command]
pub async fn save_oh_my_openagent_local_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenAgentLocalConfigInput,
) -> Result<(), String> {
    let db = state.db();

    // Load base config from local files
    let base_config = load_temp_config_from_file(&db).await?;
    let base_global = load_temp_global_config_from_file(&db).await.ok();

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

    let config_content = OhMyOpenAgentAgentsProfileContent {
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
    let created_config_value =
        db.with_conn(|conn| db_create(conn, DbTable::OhMyOpenAgentConfig, &config_json))?;
    let created_config = adapter::from_db_value(created_config_value);

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
        .or_else(|| {
            base_global
                .as_ref()
                .and_then(|g| g.browser_automation_engine.clone())
        });
    let global_claude_code = global_input
        .as_ref()
        .and_then(|g| g.claude_code.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.claude_code.clone()));
    let global_other_fields = global_input
        .as_ref()
        .and_then(|g| g.other_fields.clone())
        .or_else(|| base_global.as_ref().and_then(|g| g.other_fields.clone()));

    let global_content = OhMyOpenAgentGlobalConfigContent {
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
    put_global_config_to_sqlite(db, &global_json)?;

    if let Err(e) = apply_config_to_file(&db, &created_config.id).await {
        eprintln!("Failed to apply config after local save: {}", e);
    } else {
        #[cfg(target_os = "windows")]
        let _ = app.emit("wsl-sync-request-opencode", ());
    }

    let _ = app.emit("config-changed", "window");
    Ok(())
}
