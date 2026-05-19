use chrono::Local;
use serde_json::Value;
use std::fs;

use super::adapter;
use super::types::*;
use crate::coding::runtime_location;
use crate::db::helpers::{
    db_create, db_delete, db_get, db_list, db_patch_fields, db_patch_where_bool, db_put,
    db_query_by_bool,
};
use crate::db::schema::{DbTable, JsonFieldPath};
use crate::db::SqliteDbState;
use tauri::Emitter;

fn default_global_config() -> OhMyOpenCodeSlimGlobalConfig {
    OhMyOpenCodeSlimGlobalConfig {
        id: "global".to_string(),
        sisyphus_agent: None,
        disabled_agents: None,
        disabled_mcps: None,
        disabled_hooks: None,
        lsp: None,
        experimental: None,
        council: None,
        other_fields: None,
        updated_at: None,
    }
}

fn list_configs_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<OhMyOpenCodeSlimConfig>, String> {
    let mut configs = sqlite_state.with_conn(|conn| {
        db_list(conn, DbTable::OhMyOpenCodeSlimConfig, None).map(|records| {
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
) -> Result<Option<OhMyOpenCodeSlimConfig>, String> {
    sqlite_state.with_conn(|conn| {
        db_get(conn, DbTable::OhMyOpenCodeSlimConfig, config_id)
            .map(|record| record.map(adapter::from_db_value))
    })
}

fn get_global_config_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Option<OhMyOpenCodeSlimGlobalConfig>, String> {
    sqlite_state.with_conn(|conn| {
        db_get(conn, DbTable::OhMyOpenCodeSlimGlobalConfig, "global")
            .map(|record| record.map(adapter::global_config_from_db_value))
    })
}

fn put_config_to_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
    data: &Value,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| db_put(conn, DbTable::OhMyOpenCodeSlimConfig, config_id, data))
}

fn put_global_config_to_sqlite(sqlite_state: &SqliteDbState, data: &Value) -> Result<(), String> {
    sqlite_state
        .with_conn(|conn| db_put(conn, DbTable::OhMyOpenCodeSlimGlobalConfig, "global", data))
}

fn get_default_oh_my_opencode_slim_dir() -> Result<std::path::PathBuf, String> {
    let home_dir = dirs::home_dir().ok_or("Failed to get home directory")?;
    Ok(home_dir.join(".config").join("opencode"))
}

async fn get_oh_my_opencode_slim_config_path_and_source(
    db: &crate::db::SqliteDbState,
) -> Result<(std::path::PathBuf, &'static str), String> {
    let path = runtime_location::get_omos_config_path_async(db).await?;
    let source = if path.parent() == Some(get_default_oh_my_opencode_slim_dir()?.as_path()) {
        "default"
    } else {
        "custom"
    };
    Ok((path, source))
}

// ============================================================================
// Oh My OpenCode Slim Config Commands
// ============================================================================

/// List all oh-my-opencode-slim configs ordered by name
#[tauri::command]
pub async fn list_oh_my_opencode_slim_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OhMyOpenCodeSlimConfig>, String> {
    let db = state.db();

    let configs = list_configs_from_sqlite(db)?;
    if configs.is_empty() {
        if let Ok(temp_config) = load_temp_config_from_file(db).await {
            return Ok(vec![temp_config]);
        }
    }
    Ok(configs)
}

/// Helper function to get oh-my-opencode-slim config path
/// omos 只支持 .json 格式（不支持 jsonc）
pub async fn get_oh_my_opencode_slim_config_path(
    db: &crate::db::SqliteDbState,
) -> Result<std::path::PathBuf, String> {
    let (config_path, _) = get_oh_my_opencode_slim_config_path_and_source(db).await?;
    Ok(config_path)
}

/// Load a temporary config from local file without writing to database
/// This is used when the database is empty and we want to show the local config
/// Returns a config with id "__local__" to indicate it's from local file
async fn load_temp_config_from_file(
    db: &crate::db::SqliteDbState,
) -> Result<OhMyOpenCodeSlimConfig, String> {
    let config_path = get_oh_my_opencode_slim_config_path(db)
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
        obj.remove("council");
        obj.remove("fallback");
        obj.remove("preset");
        obj.remove("presets");
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
    let agents = adapter::resolve_slim_agents_from_config_value(&json_value);
    Ok(adapter::from_db_value(serde_json::json!({
        "id": "__local__",
        "name": "default",
        "is_applied": true,
        "is_disabled": false,
        "agents": agents,
        "council": json_value.get("council").cloned(),
        "fallback": json_value.get("fallback").cloned(),
        "other_fields": other_fields_value,
        "created_at": now.clone(),
        "updated_at": now,
    })))
}

/// Load a temporary global config from local file without writing to database
/// Returns a config with id "__local__" to indicate it's from local file
async fn load_temp_global_config_from_file(
    db: &crate::db::SqliteDbState,
) -> Result<OhMyOpenCodeSlimGlobalConfig, String> {
    let config_path = get_oh_my_opencode_slim_config_path(db)
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
    let council = json_value.get("council").cloned();

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
        obj.remove("council");
        obj.remove("preset");
        obj.remove("presets");
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
    Ok(OhMyOpenCodeSlimGlobalConfig {
        id: "__local__".to_string(), // Special ID to indicate this is from local file
        sisyphus_agent,
        disabled_agents,
        disabled_mcps,
        disabled_hooks,
        lsp,
        experimental,
        council,
        other_fields: other_fields_value,
        updated_at: Some(now),
    })
}

/// Create a new oh-my-opencode-slim config
#[tauri::command]
pub async fn create_oh_my_opencode_slim_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimConfigInput,
) -> Result<OhMyOpenCodeSlimConfig, String> {
    let db = state.db();
    let sanitized_agents = input
        .agents
        .clone()
        .map(adapter::strip_legacy_fallback_models_from_agents);

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenCodeSlimConfigContent {
        name: input.name.clone(),
        is_applied: false,
        is_disabled: false,
        agents: sanitized_agents,
        council: input.council.clone(),
        fallback: input.fallback.clone(),
        other_fields: input.other_fields.clone(),
        sort_index: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let json_data = adapter::to_db_value(&content);

    let created =
        db.with_conn(|conn| db_create(conn, DbTable::OhMyOpenCodeSlimConfig, &json_data))?;
    let _ = app.emit("config-changed", "window");
    Ok(adapter::from_db_value(created))
}

/// Update an existing oh-my-opencode-slim config
#[tauri::command]
#[allow(unused_variables)]
pub async fn update_oh_my_opencode_slim_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimConfigInput,
) -> Result<OhMyOpenCodeSlimConfig, String> {
    let db = state.db();

    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;

    let existing_config = get_config_from_sqlite(db, &config_id)?.ok_or_else(|| {
        format!(
            "Oh-my-opencode-slim config with ID '{}' not found",
            config_id
        )
    })?;
    let now = Local::now().to_rfc3339();
    let sanitized_agents = input
        .agents
        .clone()
        .map(adapter::strip_legacy_fallback_models_from_agents);
    let created_at = existing_config
        .created_at
        .clone()
        .unwrap_or_else(|| Local::now().to_rfc3339());

    let content = OhMyOpenCodeSlimConfigContent {
        name: input.name,
        is_applied: existing_config.is_applied,
        is_disabled: existing_config.is_disabled,
        agents: sanitized_agents,
        council: input.council,
        fallback: input.fallback,
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

    Ok(OhMyOpenCodeSlimConfig {
        id: config_id,
        name: content.name,
        is_applied: existing_config.is_applied,
        is_disabled: content.is_disabled,
        agents: content.agents,
        council: content.council,
        fallback: content.fallback,
        other_fields: content.other_fields,
        sort_index: content.sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    })
}

/// Delete an oh-my-opencode-slim config
#[tauri::command]
pub async fn delete_oh_my_opencode_slim_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();

    db.with_conn(|conn| db_delete(conn, DbTable::OhMyOpenCodeSlimConfig, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Clear the currently applied runtime config file without deleting the saved profile.
#[tauri::command]
pub async fn clear_oh_my_opencode_slim_applied_config(
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

    let config_path = get_oh_my_opencode_slim_config_path(&db).await?;

    #[cfg(target_os = "windows")]
    crate::coding::wsl::remove_auto_synced_wsl_mapping_target(state.inner(), "opencode-oh-my-slim")
        .await?;

    if config_path.exists() {
        fs::remove_file(&config_path)
            .map_err(|e| format!("Failed to remove config file: {}", e))?;
    }

    let now = Local::now().to_rfc3339();
    db.with_conn(|conn| {
        db_patch_where_bool(
            conn,
            DbTable::OhMyOpenCodeSlimConfig,
            &JsonFieldPath::new("is_applied")?,
            true,
            &[
                ("is_applied", Value::Bool(false)),
                ("updated_at", Value::String(now.clone())),
            ],
        )
        .map(|_| ())
    })?;

    let _ = app.emit("config-changed", "window");

    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

/// 内部函数：将指定配置应用到配置文件
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
    let agents_profile = get_config_from_sqlite(db, config_id)?
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;

    // Check if config is disabled
    if agents_profile.is_disabled {
        return Err(format!(
            "Config '{}' is disabled and cannot be applied",
            config_id
        ));
    }

    let config_path = get_oh_my_opencode_slim_config_path(db).await?;

    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create opencode config directory: {}", e))?;
        }
    }

    // 获取 Global Config
    let global_config = get_global_config_from_sqlite(db)?.unwrap_or_else(default_global_config);

    let mut final_json = serde_json::Map::new();

    // omos 不需要 $schema 字段

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
    if let Some(lsp) = global_config.lsp {
        final_json.insert("lsp".to_string(), lsp);
    }
    if let Some(experimental) = global_config.experimental {
        final_json.insert("experimental".to_string(), experimental);
    }
    if let Some(council) = global_config.council {
        final_json.insert("council".to_string(), council);
    }

    if let Some(global_others) = global_config.other_fields {
        if let Some(others_obj) = global_others.as_object() {
            for (key, value) in others_obj {
                if key == "council" {
                    continue;
                }
                final_json.insert(key.clone(), value.clone());
            }
        }
    }

    if let Some(agents) = agents_profile.agents {
        final_json.insert(
            "agents".to_string(),
            adapter::strip_legacy_fallback_models_from_agents(agents),
        );
    }
    if let Some(profile_council) = agents_profile.council {
        final_json.insert("council".to_string(), profile_council);
    }
    let existing_global_fallback = final_json.get("fallback").cloned();
    let profile_fallback = agents_profile
        .fallback
        .and_then(|fallback| adapter::fallback_config_to_value(&fallback));
    if let Some(merged_fallback) =
        adapter::merge_fallback_values(profile_fallback, existing_global_fallback)
    {
        final_json.insert("fallback".to_string(), merged_fallback);
    }

    if let Some(profile_others) = agents_profile.other_fields {
        if let Some(others_obj) = profile_others.as_object() {
            for (key, value) in others_obj {
                if key == "council" || key == "fallback" {
                    continue;
                }
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
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    apply_config_internal(&db, &app, &config_id, false).await?;
    Ok(())
}

/// Internal function to apply config
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &crate::db::SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_config_to_file(db, config_id).await?;

    let now = Local::now().to_rfc3339();

    let applied_field = JsonFieldPath::new("is_applied")?;
    db.with_conn(|conn| {
        db_patch_where_bool(
            conn,
            DbTable::OhMyOpenCodeSlimConfig,
            &applied_field,
            true,
            &[
                ("is_applied", Value::Bool(false)),
                ("updated_at", Value::String(now.clone())),
            ],
        )?;
        db_patch_fields(
            conn,
            DbTable::OhMyOpenCodeSlimConfig,
            config_id,
            &[
                ("is_applied", Value::Bool(true)),
                ("updated_at", Value::String(now.clone())),
            ],
        )?
        .ok_or_else(|| format!("Config '{}' not found", config_id))?;
        Ok(())
    })?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

/// Reorder oh-my-opencode-slim configs
#[tauri::command]
pub async fn reorder_oh_my_opencode_slim_configs(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();

    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::OhMyOpenCodeSlimConfig,
                id,
                &[("sort_index", serde_json::json!(index as i32))],
            )
            .map(|_| ())
        })?;
    }

    Ok(())
}

/// Get oh-my-opencode-slim config file path info
#[tauri::command]
pub async fn get_oh_my_opencode_slim_config_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ConfigPathInfo, String> {
    let db = state.db();
    let (config_path, source) = get_oh_my_opencode_slim_config_path_and_source(&db).await?;
    let path = config_path.to_string_lossy().to_string();

    Ok(ConfigPathInfo {
        path,
        source: source.to_string(),
    })
}

/// Check if local oh-my-opencode-slim config file exists
#[tauri::command]
pub async fn check_oh_my_opencode_slim_config_exists(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<bool, String> {
    let db = state.db();
    let config_path = get_oh_my_opencode_slim_config_path(&db).await?;
    Ok(config_path.exists())
}

// ============================================================================
// Oh My OpenCode Slim Global Config Commands
// ============================================================================

/// Get oh-my-opencode-slim global config
#[tauri::command]
pub async fn get_oh_my_opencode_slim_global_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OhMyOpenCodeSlimGlobalConfig, String> {
    let db = state.db();
    if let Some(config) = get_global_config_from_sqlite(db)? {
        return Ok(config);
    }
    if let Ok(temp_config) = load_temp_global_config_from_file(db).await {
        return Ok(temp_config);
    }
    Ok(default_global_config())
}

/// Save oh-my-opencode-slim global config
#[tauri::command]
#[allow(unused_variables)]
pub async fn save_oh_my_opencode_slim_global_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimGlobalConfigInput,
) -> Result<OhMyOpenCodeSlimGlobalConfig, String> {
    let db = state.db();

    let now = Local::now().to_rfc3339();
    let content = OhMyOpenCodeSlimGlobalConfigContent {
        sisyphus_agent: input.sisyphus_agent,
        disabled_agents: input.disabled_agents,
        disabled_mcps: input.disabled_mcps,
        disabled_hooks: input.disabled_hooks,
        lsp: input.lsp,
        experimental: input.experimental,
        council: input.council,
        other_fields: input.other_fields,
        updated_at: now.clone(),
    };

    let json_data = adapter::global_config_to_db_value(&content);

    put_global_config_to_sqlite(db, &json_data)?;

    let applied_configs = db.with_conn(|conn| {
        db_query_by_bool(
            conn,
            DbTable::OhMyOpenCodeSlimConfig,
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

/// Toggle is_disabled status for a config
#[tauri::command]
pub async fn toggle_oh_my_opencode_slim_config_disabled(
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
                DbTable::OhMyOpenCodeSlimConfig,
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

/// Save local config (both Agents Profile and Global Config) into database
/// This is used when saving __local__ temporary config to database
/// Input can include config and/or globalConfig; missing parts will be loaded from local files
#[tauri::command]
pub async fn save_oh_my_opencode_slim_local_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OhMyOpenCodeSlimLocalConfigInput,
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
        .map(|config| config.name.clone())
        .unwrap_or(base_config.name);
    let config_agents = if let Some(config) = config_input.as_ref() {
        config
            .agents
            .clone()
            .map(adapter::strip_legacy_fallback_models_from_agents)
    } else {
        base_config
            .agents
            .map(adapter::strip_legacy_fallback_models_from_agents)
    };
    let config_council = if let Some(config) = config_input.as_ref() {
        config.council.clone()
    } else {
        base_config.council
    };
    let config_fallback = if let Some(config) = config_input.as_ref() {
        config.fallback.clone()
    } else {
        base_config.fallback
    };
    let config_other_fields = if let Some(config) = config_input.as_ref() {
        config.other_fields.clone()
    } else {
        base_config.other_fields
    };

    let config_content = OhMyOpenCodeSlimConfigContent {
        name: config_name,
        is_applied: true,
        is_disabled: false,
        agents: config_agents,
        council: config_council,
        fallback: config_fallback,
        other_fields: config_other_fields,
        sort_index: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let config_json = adapter::to_db_value(&config_content);
    let created_config_value =
        db.with_conn(|conn| db_create(conn, DbTable::OhMyOpenCodeSlimConfig, &config_json))?;
    let created_config = adapter::from_db_value(created_config_value);

    // Build Global Config content
    let global_input = input.global_config;
    let global_sisyphus_agent = if let Some(global) = global_input.as_ref() {
        global.sisyphus_agent.clone()
    } else {
        base_global
            .as_ref()
            .and_then(|global| global.sisyphus_agent.clone())
    };
    let global_disabled_agents = if let Some(global) = global_input.as_ref() {
        global.disabled_agents.clone()
    } else {
        base_global
            .as_ref()
            .and_then(|global| global.disabled_agents.clone())
    };
    let global_disabled_mcps = if let Some(global) = global_input.as_ref() {
        global.disabled_mcps.clone()
    } else {
        base_global
            .as_ref()
            .and_then(|global| global.disabled_mcps.clone())
    };
    let global_disabled_hooks = if let Some(global) = global_input.as_ref() {
        global.disabled_hooks.clone()
    } else {
        base_global
            .as_ref()
            .and_then(|global| global.disabled_hooks.clone())
    };
    let global_lsp = if let Some(global) = global_input.as_ref() {
        global.lsp.clone()
    } else {
        base_global.as_ref().and_then(|global| global.lsp.clone())
    };
    let global_experimental = if let Some(global) = global_input.as_ref() {
        global.experimental.clone()
    } else {
        base_global
            .as_ref()
            .and_then(|global| global.experimental.clone())
    };
    let global_other_fields = if let Some(global) = global_input.as_ref() {
        global.other_fields.clone()
    } else {
        base_global
            .as_ref()
            .and_then(|global| global.other_fields.clone())
    };
    let global_council = if let Some(global) = global_input.as_ref() {
        global.council.clone()
    } else {
        base_global
            .as_ref()
            .and_then(|global| global.council.clone())
    };

    let global_content = OhMyOpenCodeSlimGlobalConfigContent {
        sisyphus_agent: global_sisyphus_agent,
        disabled_agents: global_disabled_agents,
        disabled_mcps: global_disabled_mcps,
        disabled_hooks: global_disabled_hooks,
        lsp: global_lsp,
        experimental: global_experimental,
        council: global_council,
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
