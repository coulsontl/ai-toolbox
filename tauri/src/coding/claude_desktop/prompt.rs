//! Claude Desktop global prompt config commands.
//!
//! Mirrors the `claude_code` prompt CRUD but writes the applied prompt file to
//! `AGENTS.md` under the Claude Desktop normal config directory (resolved via
//! `config_writer::current_platform_paths`), matching the desktop "config file
//! path" model instead of a CLI root directory.

use chrono::Local;
use serde_json::Value;
use std::path::PathBuf;

use super::adapter;
use super::config_writer;
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_put, db_update_applied_status,
};
use crate::db::schema::{Cli, CliConfigKind, DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::Emitter;

/// Resolve the applied prompt file path (`AGENTS.md`) inside the Claude Desktop
/// normal config directory.
fn get_claude_desktop_prompt_file_path() -> Result<PathBuf, String> {
    let paths = config_writer::current_platform_paths()?;
    Ok(paths
        .normal_config_path
        .parent()
        .unwrap_or_else(|| paths.normal_config_path.as_path())
        .join("AGENTS.md"))
}

fn desktop_prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn list_prompts_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<ClaudeDesktopPromptConfig>, String> {
    let order = desktop_prompt_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(
            db_list(conn, DbTable::CliConfig(Cli::ClaudeDesktop, CliConfigKind::Prompt), Some(&order))?
                .into_iter()
                .map(adapter::from_db_value_prompt)
                .collect(),
        )
    })
}

fn get_prompt_from_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
) -> Result<Option<ClaudeDesktopPromptConfig>, String> {
    sqlite_state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::CliConfig(Cli::ClaudeDesktop, CliConfigKind::Prompt), config_id)?
            .map(adapter::from_db_value_prompt))
    })
}

fn put_prompt_to_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
    content: &ClaudeDesktopPromptConfigContent,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CliConfig(Cli::ClaudeDesktop, CliConfigKind::Prompt),
            config_id,
            &adapter::to_db_value_prompt(content),
        )
    })
}

fn delete_prompt_from_sqlite(sqlite_state: &SqliteDbState, config_id: &str) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_delete(conn, DbTable::CliConfig(Cli::ClaudeDesktop, CliConfigKind::Prompt), config_id).map(|_| ())
    })
}

async fn get_local_prompt_config() -> Result<Option<ClaudeDesktopPromptConfig>, String> {
    let prompt_path = get_claude_desktop_prompt_file_path()?;
    let Some(prompt_content) = read_prompt_content_file(&prompt_path, "Claude Desktop")? else {
        return Ok(None);
    };

    let now = Local::now().to_rfc3339();
    Ok(Some(ClaudeDesktopPromptConfig {
        id: crate::coding::local_bridge::LOCAL_CONFIG_ID.to_string(),
        name: "default".to_string(),
        content: prompt_content,
        is_applied: true,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }))
}

async fn write_prompt_content_to_file(prompt_content: Option<&str>) -> Result<(), String> {
    let prompt_path = get_claude_desktop_prompt_file_path()?;
    write_prompt_content_file(&prompt_path, prompt_content, "Claude Desktop")
}

// ============================================================================
// Prompt Config Commands
// ============================================================================

/// List all Claude Desktop prompt configs; fall back to the local `AGENTS.md`
/// when no managed preset exists yet.
#[tauri::command]
pub async fn list_claude_desktop_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<ClaudeDesktopPromptConfig>, String> {
    let db = state.db();
    let records = list_prompts_from_sqlite(db)?;
    if records.is_empty() {
        if let Some(local_config) = get_local_prompt_config().await? {
            return Ok(vec![local_config]);
        }
    }
    Ok(records)
}

/// Create a new Claude Desktop prompt config.
#[tauri::command]
pub async fn create_claude_desktop_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ClaudeDesktopPromptConfigInput,
) -> Result<ClaudeDesktopPromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    let next_sort_index = db
        .with_conn(|conn| {
            db_max_i64(
                conn,
                DbTable::CliConfig(Cli::ClaudeDesktop, CliConfigKind::Prompt),
                &JsonFieldPath::new("sort_index")?,
            )
        })?
        .map(|value| value as i32 + 1)
        .unwrap_or(0);

    let content = ClaudeDesktopPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_prompt(&content);
    let prompt_id = db_new_id();

    put_prompt_to_sqlite(db, &prompt_id, &content)?;

    let created_config = adapter::from_db_value_prompt({
        let mut value = json_data;
        if let Some(object) = value.as_object_mut() {
            object.insert("id".to_string(), Value::String(prompt_id));
        }
        value
    });

    let _ = app.emit("config-changed", "window");

    Ok(created_config)
}

/// Update an existing Claude Desktop prompt config; rewrite `AGENTS.md` if applied.
#[tauri::command]
pub async fn update_claude_desktop_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ClaudeDesktopPromptConfigInput,
) -> Result<ClaudeDesktopPromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let existing_prompt = get_prompt_from_sqlite(db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    let created_at = existing_prompt
        .created_at
        .clone()
        .unwrap_or_else(|| Local::now().to_rfc3339());
    let is_applied = existing_prompt.is_applied;
    let sort_index = existing_prompt.sort_index;

    let now = Local::now().to_rfc3339();
    let content = ClaudeDesktopPromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied,
        sort_index,
        created_at,
        updated_at: now.clone(),
    };
    put_prompt_to_sqlite(db, &config_id, &content)?;

    if is_applied {
        write_prompt_content_to_file(Some(input.content.as_str())).await?;
    }

    let _ = app.emit("config-changed", "window");

    Ok(ClaudeDesktopPromptConfig {
        id: config_id,
        name: content.name,
        content: content.content,
        is_applied,
        sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(now),
    })
}

/// Delete a Claude Desktop prompt config.
#[tauri::command]
pub async fn delete_claude_desktop_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    delete_prompt_from_sqlite(db, &id)?;

    let _ = db;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub async fn apply_prompt_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_prompt_config_internal_with_events(state, app, config_id, from_tray, true).await
}

async fn apply_prompt_config_internal_with_events<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
    emit_events: bool,
) -> Result<(), String> {
    if config_id == crate::coding::local_bridge::LOCAL_CONFIG_ID {
        let local_prompt = get_local_prompt_config()
            .await?
            .ok_or_else(|| "Local default prompt not found".to_string())?;
        write_prompt_content_to_file(Some(local_prompt.content.as_str())).await?;

        if emit_events {
            let payload = if from_tray { "tray" } else { "window" };
            let _ = app.emit("config-changed", payload);
        }

        return Ok(());
    }

    let db = state.db();
    let prompt_config = get_prompt_from_sqlite(db, config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    let now = Local::now().to_rfc3339();

    for mut prompt in list_prompts_from_sqlite(db)? {
        let should_be_applied = prompt.id == config_id;
        if prompt.is_applied == should_be_applied {
            continue;
        }
        let prompt_id = prompt.id.clone();
        prompt.is_applied = should_be_applied;
        let content = ClaudeDesktopPromptConfigContent {
            name: prompt.name,
            content: prompt.content,
            is_applied: prompt.is_applied,
            sort_index: prompt.sort_index,
            created_at: prompt.created_at.unwrap_or_else(|| now.clone()),
            updated_at: now.clone(),
        };
        put_prompt_to_sqlite(db, &prompt_id, &content)?;
    }

    write_prompt_content_to_file(Some(prompt_config.content.as_str())).await?;

    if emit_events {
        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
    }

    Ok(())
}

/// Apply a Claude Desktop prompt config (write `AGENTS.md` + update applied state).
#[tauri::command]
pub async fn apply_claude_desktop_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_prompt_config_internal(state, &app, &config_id, false).await
}

/// Disable the applied Claude Desktop prompt: clear every applied flag and
/// empty `AGENTS.md`, while keeping the DB record so it can be re-applied later.
#[tauri::command]
pub async fn disable_claude_desktop_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    get_prompt_from_sqlite(db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::ClaudeDesktop, CliConfigKind::Prompt), None, &now)
    })?;
    write_prompt_content_to_file(Some("")).await?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Reorder Claude Desktop prompt configs by a full ordered id list.
#[tauri::command]
pub async fn reorder_claude_desktop_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();

    let now = Local::now().to_rfc3339();
    for (index, id) in ids.iter().enumerate() {
        if let Some(prompt) = get_prompt_from_sqlite(db, id)? {
            let content = ClaudeDesktopPromptConfigContent {
                name: prompt.name,
                content: prompt.content,
                is_applied: prompt.is_applied,
                sort_index: Some(index as i32),
                created_at: prompt.created_at.unwrap_or_else(|| now.clone()),
                updated_at: prompt.updated_at.unwrap_or_else(|| now.clone()),
            };
            put_prompt_to_sqlite(db, id, &content)?;
        }
    }

    let _ = db;
    let _ = app.emit("config-changed", "window");

    Ok(())
}

/// Adopt the current local `AGENTS.md` as a managed preset and apply it.
#[tauri::command]
pub async fn save_claude_desktop_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ClaudeDesktopPromptConfigInput,
) -> Result<ClaudeDesktopPromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        get_local_prompt_config()
            .await?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };

    let created = create_claude_desktop_prompt_config(
        state.clone(),
        app.clone(),
        ClaudeDesktopPromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;

    apply_prompt_config_internal(state.clone(), &app, &created.id, false).await?;

    let db = state.db();
    Ok(get_prompt_from_sqlite(db, &created.id)?.unwrap_or(created))
}
