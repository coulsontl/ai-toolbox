//! CodeBuddy Code CLI commands.
//!
//! Mirrors the WorkBuddy desktop commands, with three differences the CLI
//! actually has:
//!
//! - Its config root is `~/.codebuddy` (`CODEBUDDY_CONFIG_DIR` first).
//! - Prompt presets are `CODEBUDDY.md`, a normal switchable global prompt.
//! - It has a `settings.json` the user may want to inspect.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::adapter;
use super::constants::{
    CODEBUDDY_PROMPT_FILE, CODEBUDDY_SETTINGS_CONFIG_ID, CODEBUDDY_SETTINGS_FILE,
};
use super::paths;
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::workbuddy_family::models_json::{self, ModelsDocument};
use crate::coding::workbuddy_family::runtime_config::{build_runtime_config, FamilyRuntimeConfig};
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_put, db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use chrono::Local;
use tauri::{Emitter, Runtime};

/// Serializes every read-modify-write of `models.json`.
///
/// The CLI is not watching the file, but two AI Toolbox saves dispatched
/// concurrently each hold their own snapshot; without this lock the second one
/// would write back the first one's pre-save view.
fn models_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_models_write() -> std::sync::MutexGuard<'static, ()> {
    models_write_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn custom_config_dir_from_db(db: &SqliteDbState) -> Option<PathBuf> {
    db.with_conn(|conn| {
        db_get(
            conn,
            DbTable::CodebuddySettingsConfig,
            CODEBUDDY_SETTINGS_CONFIG_ID,
        )
    })
    .ok()
    .flatten()
    .and_then(|value| adapter::settings_from_db_value(value).config_dir)
    .filter(|path| !path.trim().is_empty())
    .map(PathBuf::from)
}

async fn config_root_from_db(db: &SqliteDbState) -> (PathBuf, String) {
    match custom_config_dir_from_db(db) {
        Some(path) => (path, "custom".to_string()),
        None => paths::resolve_path_without_db(),
    }
}

fn emit_config_changed<R: Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("config-changed", "window");
}

fn read_document(root: &Path) -> Result<ModelsDocument, String> {
    ModelsDocument::load(root)
}

// ============================================================================
// Path info / settings
// ============================================================================

#[tauri::command]
pub async fn get_codebuddy_default_config_dir() -> Result<String, String> {
    Ok(paths::default_config_dir()?.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn get_codebuddy_config_dir_without_db() -> String {
    paths::resolve_path_without_db()
        .0
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub async fn get_codebuddy_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<CodebuddyPathInfo, String> {
    let (path, source) = config_root_from_db(&state).await;
    Ok(CodebuddyPathInfo {
        path: path.to_string_lossy().into_owned(),
        source,
    })
}

#[tauri::command]
pub async fn get_codebuddy_settings_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<CodebuddySettingsConfig>, String> {
    let record = state
        .with_conn(|conn| {
            db_get(
                conn,
                DbTable::CodebuddySettingsConfig,
                CODEBUDDY_SETTINGS_CONFIG_ID,
            )
        })
        .map_err(|error| format!("Failed to read CodeBuddy settings: {error}"))?;
    Ok(record.map(adapter::settings_from_db_value))
}

#[tauri::command]
pub async fn save_codebuddy_settings_config(
    state: tauri::State<'_, SqliteDbState>,
    input: CodebuddySettingsConfigInput,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let normalized = if input.clear_config_dir {
        None
    } else {
        input
            .config_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let expanded = crate::coding::path_expand::expand_local_path(value)?;
                Ok::<String, String>(
                    paths::normalize_selected_root(std::path::Path::new(&expanded))
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .transpose()?
    };

    if let Some(dir) = normalized.as_deref() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("Failed to create CodeBuddy config directory: {error}"))?;
    }

    state
        .with_conn(|conn| {
            db_put(
                conn,
                DbTable::CodebuddySettingsConfig,
                CODEBUDDY_SETTINGS_CONFIG_ID,
                &adapter::settings_to_db_value(normalized.as_deref()),
            )
        })
        .map_err(|error| format!("Failed to save CodeBuddy settings: {error}"))?;

    crate::coding::runtime_location::refresh_runtime_location_cache_for_module_async(
        &state,
        "codebuddy",
    )
    .await
    .ok();

    emit_config_changed(&app);
    Ok(())
}

/// The raw `settings.json` body, for the "other config" panel.
///
/// Returned verbatim rather than parsed: AI Toolbox does not own this file, and
/// a JSON round trip would drop key order and comments.
#[tauri::command]
pub async fn read_codebuddy_settings_file(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<String>, String> {
    let (root, _) = config_root_from_db(&state).await;
    let path = root.join(CODEBUDDY_SETTINGS_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))
}

// ============================================================================
// models.json
// ============================================================================

/// Returns `models.json` verbatim for the preview dialog.
///
/// Never rebuild the text from `FamilyRuntimeConfig`: the channel view drops
/// entries without a URL, reorders fields and loses product-owned keys, so a
/// round-tripped preview would show a file the user does not have.
#[tauri::command]
pub async fn read_codebuddy_models_file(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<String>, String> {
    let (root, _) = config_root_from_db(&state).await;
    let path = root.join(models_json::MODELS_FILE_NAME);
    if !path.is_file() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))
}

#[tauri::command]
pub async fn read_codebuddy_runtime_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<FamilyRuntimeConfig, String> {
    let (root, _) = config_root_from_db(&state).await;
    let document = read_document(&root)?;
    Ok(build_runtime_config(&root, &document))
}

#[tauri::command]
pub async fn save_codebuddy_model(
    state: tauri::State<'_, SqliteDbState>,
    input: CodebuddyModelInput,
    app: tauri::AppHandle,
) -> Result<FamilyRuntimeConfig, String> {
    let model_id = input.id.trim().to_string();
    if model_id.is_empty() {
        return Err("Model id is required".to_string());
    }
    if input.base_url.trim().is_empty() {
        return Err("Model endpoint is required".to_string());
    }
    crate::coding::workbuddy_family::ensure_models_file_direct(
        crate::coding::proxy_gateway::types::GatewayCliKey::Codebuddy,
    )?;

    let (root, _) = config_root_from_db(&state).await;
    let _guard = lock_models_write();

    let mut document = read_document(&root)?;
    // Renaming merges onto the entry the user edited so product-owned keys stay
    // attached and the row keeps its position; removing then appending would
    // drop both.
    document.upsert_model_renaming(
        models_json::build_model_entry(&input.as_entry_input()),
        input.previous_id.as_deref(),
    );
    document.save(&root)?;

    emit_config_changed(&app);
    Ok(build_runtime_config(&root, &document))
}

#[tauri::command]
pub async fn delete_codebuddy_model(
    state: tauri::State<'_, SqliteDbState>,
    model_id: String,
    app: tauri::AppHandle,
) -> Result<FamilyRuntimeConfig, String> {
    crate::coding::workbuddy_family::ensure_models_file_direct(
        crate::coding::proxy_gateway::types::GatewayCliKey::Codebuddy,
    )?;
    let (root, _) = config_root_from_db(&state).await;
    let _guard = lock_models_write();

    let mut document = read_document(&root)?;
    if !document.remove_model(model_id.trim()) {
        return Err(format!("Model '{model_id}' was not found"));
    }
    document.save(&root)?;

    emit_config_changed(&app);
    Ok(build_runtime_config(&root, &document))
}

/// Deletes every model entry of one channel.
#[tauri::command]
pub async fn delete_codebuddy_provider(
    state: tauri::State<'_, SqliteDbState>,
    base_url: String,
    api_key: String,
    app: tauri::AppHandle,
) -> Result<FamilyRuntimeConfig, String> {
    crate::coding::workbuddy_family::ensure_models_file_direct(
        crate::coding::proxy_gateway::types::GatewayCliKey::Codebuddy,
    )?;
    let (root, _) = config_root_from_db(&state).await;
    let _guard = lock_models_write();

    let mut document = read_document(&root)?;
    let members = document.models_for_channel(&base_url, &api_key);
    if members.is_empty() {
        return Err("This provider has no models to delete".to_string());
    }
    for entry in members {
        if let Some(id) = models_json::entry_id(&entry) {
            document.remove_model(&id);
        }
    }
    document.save(&root)?;

    emit_config_changed(&app);
    Ok(build_runtime_config(&root, &document))
}

// ============================================================================
// Prompt presets (`CODEBUDDY.md`)
// ============================================================================

fn prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn next_sort_index(db: &SqliteDbState) -> Result<i32, String> {
    db.with_conn(|conn| {
        Ok(db_max_i64(conn, DbTable::CodebuddyPromptConfig, &JsonFieldPath::new("sort_index")?)?
            .map(|value| value as i32 + 1)
            .unwrap_or(0))
    })
}

fn prompt_from_content(id: String, content: CodebuddyPromptConfigContent) -> CodebuddyPromptConfig {
    CodebuddyPromptConfig {
        id,
        name: content.name,
        content: content.content,
        is_applied: content.is_applied,
        sort_index: content.sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    }
}

fn get_prompt(db: &SqliteDbState, id: &str) -> Result<Option<CodebuddyPromptConfig>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CodebuddyPromptConfig, id))
        .map(|record| record.map(adapter::prompt_from_db_value))
}

fn prompt_file_path(root: &Path) -> PathBuf {
    root.join(CODEBUDDY_PROMPT_FILE)
}

/// The live `CODEBUDDY.md` as a synthetic `__local__` preset.
fn local_prompt_config(root: &Path) -> Result<Option<CodebuddyPromptConfig>, String> {
    let content = read_prompt_content_file(&prompt_file_path(root), "CodeBuddy")?;
    Ok(content.map(|content| CodebuddyPromptConfig {
        id: crate::coding::local_bridge::LOCAL_CONFIG_ID.to_string(),
        name: crate::coding::local_bridge::LOCAL_CONFIG_NAME.to_string(),
        content,
        is_applied: true,
        sort_index: None,
        created_at: None,
        updated_at: None,
    }))
}

#[tauri::command]
pub async fn list_codebuddy_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<CodebuddyPromptConfig>, String> {
    let order = prompt_order()?;
    let prompts = state
        .db()
        .with_conn(|conn| db_list(conn, DbTable::CodebuddyPromptConfig, Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .map(adapter::prompt_from_db_value)
                .collect::<Vec<_>>()
        })?;
    if prompts.is_empty() {
        let (root, _) = config_root_from_db(state.db()).await;
        if let Some(local_config) = local_prompt_config(&root)? {
            return Ok(vec![local_config]);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_codebuddy_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodebuddyPromptConfigInput,
) -> Result<CodebuddyPromptConfig, String> {
    let now = Local::now().to_rfc3339();
    let content = CodebuddyPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index(state.db())?),
        created_at: now.clone(),
        updated_at: now,
    };
    let id = db_new_id();
    state.db().with_conn(|conn| {
        db_put(
            conn,
            DbTable::CodebuddyPromptConfig,
            &id,
            &adapter::prompt_to_db_value(&content),
        )
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(prompt_from_content(id, content))
}

#[tauri::command]
pub async fn update_codebuddy_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodebuddyPromptConfigInput,
) -> Result<CodebuddyPromptConfig, String> {
    let id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let existing = get_prompt(state.db(), &id)?
        .ok_or_else(|| format!("CodeBuddy prompt '{id}' not found"))?;
    let content = CodebuddyPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: existing.is_applied,
        sort_index: existing.sort_index,
        created_at: existing
            .created_at
            .unwrap_or_else(|| Local::now().to_rfc3339()),
        updated_at: Local::now().to_rfc3339(),
    };
    state.db().with_conn(|conn| {
        db_put(
            conn,
            DbTable::CodebuddyPromptConfig,
            &id,
            &adapter::prompt_to_db_value(&content),
        )
    })?;
    if content.is_applied {
        let (root, _) = config_root_from_db(state.db()).await;
        write_prompt_content_file(
            &prompt_file_path(&root),
            Some(&content.content),
            "CodeBuddy",
        )?;
    }
    let _ = app.emit("config-changed", "window");
    Ok(prompt_from_content(id, content))
}

#[tauri::command]
pub async fn delete_codebuddy_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    // Only delete the DB record. Keep the live CODEBUDDY.md on disk.
    state
        .db()
        .with_conn(|conn| db_delete(conn, DbTable::CodebuddyPromptConfig, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Adopts the live `CODEBUDDY.md` as a new preset and applies it.
///
/// The shared prompt card calls this when the user edits the `__local__`
/// bridge row, which is not a DB record: the edit becomes a real preset the
/// user can then keep, rename or delete.
#[tauri::command]
pub async fn save_codebuddy_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodebuddyPromptConfigInput,
) -> Result<CodebuddyPromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        let (root, _) = config_root_from_db(state.db()).await;
        local_prompt_config(&root)?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };

    let created = create_codebuddy_prompt_config(
        state.clone(),
        app.clone(),
        CodebuddyPromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;

    apply_codebuddy_prompt_internal(state.db(), &app, &created.id).await?;
    Ok(get_prompt(state.db(), &created.id)?.unwrap_or(created))
}

#[tauri::command]
pub async fn apply_codebuddy_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_codebuddy_prompt_internal(state.db(), &app, &config_id).await
}

/// Marks one preset applied and writes it to `CODEBUDDY.md`.
async fn apply_codebuddy_prompt_internal(
    db: &SqliteDbState,
    app: &tauri::AppHandle,
    config_id: &str,
) -> Result<(), String> {
    apply_codebuddy_prompt_internal_without_events(db, config_id).await?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Post-restore re-apply path: writes `CODEBUDDY.md` without emitting
/// `config-changed`, so a startup recovery does not look like a user edit to
/// every other listener (tray rebuild, WSL/SSH sync).
pub async fn apply_codebuddy_prompt_internal_without_events(
    db: &SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    let prompt = get_prompt(db, config_id)?
        .ok_or_else(|| format!("CodeBuddy prompt '{config_id}' not found"))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CodebuddyPromptConfig, Some(config_id), &now)
    })?;
    let (root, _) = config_root_from_db(db).await;
    write_prompt_content_file(&prompt_file_path(&root), Some(&prompt.content), "CodeBuddy")?;
    Ok(())
}

/// Disable semantics: clear every applied flag and empty the live
/// `CODEBUDDY.md`, while keeping all DB records so any can be re-applied.
///
/// Takes the id of the record being disabled only to mirror the shared
/// `GlobalPromptApi.disableConfig(configId)` contract; the effect is global,
/// exactly like every other CLI's disable command.
#[tauri::command]
pub async fn disable_codebuddy_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    get_prompt(state.db(), &config_id)?
        .ok_or_else(|| format!("CodeBuddy prompt '{config_id}' not found"))?;
    let now = Local::now().to_rfc3339();
    // The id only gates the call (mirroring the shared `disableConfig(configId)`
    // contract); disabling is global, like every other CLI.
    state
        .db()
        .with_conn_mut(|conn| {
            db_update_applied_status(conn, DbTable::CodebuddyPromptConfig, None, &now)
        })?;
    let (root, _) = config_root_from_db(state.db()).await;
    write_prompt_content_file(&prompt_file_path(&root), None, "CodeBuddy")?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn reorder_codebuddy_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    state.db().with_conn(|conn| {
        for (index, id) in ids.iter().enumerate() {
            crate::db::helpers::db_patch_fields(
                conn,
                DbTable::CodebuddyPromptConfig,
                id,
                &[
                    ("sort_index", serde_json::json!(index as i32)),
                    ("updated_at", serde_json::json!(now)),
                ],
            )?;
        }
        Ok(())
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[cfg(test)]
mod tests;
