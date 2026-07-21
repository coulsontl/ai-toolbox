use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;
use serde_json::{json, Value};
use tempfile::NamedTempFile;
use toml_edit::{value, DocumentMut, Item, Table};

use super::adapter;
use super::constants::{GROK_ENV_KEY, GROK_LOCAL_PROVIDER_ID};
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::open_code::shell_env;
use crate::coding::open_code::types::{
    OpenCodeAllApiHubProvider, OpenCodeAllApiHubProvidersResult,
    ResolveOpenCodeAllApiHubProvidersRequest,
};
use crate::coding::runtime_location;
use crate::coding::skills::commands::resync_all_skills_if_tool_path_changed;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::Emitter;

pub fn get_grok_default_root_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".grok"))
        .map_err(|_| "Failed to get home directory".to_string())
}

pub fn get_grok_root_dir_without_db() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var(GROK_ENV_KEY) {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    if let Some(path) = shell_env::get_env_from_shell_config(GROK_ENV_KEY) {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    get_grok_default_root_dir()
}

pub async fn get_grok_root_dir_from_db_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(runtime_location::get_grok_runtime_location_async(db)
        .await?
        .host_path)
}

pub async fn get_grok_config_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    runtime_location::get_grok_config_path_async(db).await
}

pub async fn get_grok_auth_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    runtime_location::get_grok_auth_path_async(db).await
}

pub async fn get_grok_prompt_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    runtime_location::get_grok_prompt_path_async(db).await
}

#[tauri::command]
pub async fn get_grok_root_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<GrokPathInfo, String> {
    let location = runtime_location::get_grok_runtime_location_async(state.db()).await?;
    Ok(GrokPathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

#[tauri::command]
pub async fn get_grok_config_dir_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    Ok(get_grok_root_dir_from_db_async(state.db())
        .await?
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub async fn get_grok_config_file_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    Ok(get_grok_config_path_async(state.db())
        .await?
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub async fn reveal_grok_config_folder(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<(), String> {
    let config_dir = get_grok_root_dir_from_db_async(state.db()).await?;
    fs::create_dir_all(&config_dir)
        .map_err(|error| format!("Failed to create Grok config directory: {error}"))?;

    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");

    command
        .arg(&config_dir)
        .spawn()
        .map_err(|error| format!("Failed to reveal Grok config directory: {error}"))?;
    Ok(())
}

#[tauri::command]
pub async fn fetch_grok_official_models(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<GrokOfficialModelsResponse, String> {
    match run_grok_models_command(state.db()).await {
        Ok(output) => {
            let models = parse_grok_models_output(&output);
            if models.is_empty() {
                Ok(bundled_grok_official_models("bundled-fallback"))
            } else {
                Ok(GrokOfficialModelsResponse {
                    total: models.len(),
                    models,
                    source: "cli".to_string(),
                    tier: "official".to_string(),
                })
            }
        }
        Err(_) => Ok(bundled_grok_official_models("bundled")),
    }
}

fn bundled_grok_official_models(source: &str) -> GrokOfficialModelsResponse {
    let models = ["grok-4.5", "grok-build"]
        .into_iter()
        .map(|model_id| GrokOfficialModel {
            id: model_id.to_string(),
            name: Some(model_id.to_string()),
            owned_by: Some("xai".to_string()),
            created: None,
        })
        .collect::<Vec<_>>();
    GrokOfficialModelsResponse {
        total: models.len(),
        models,
        source: source.to_string(),
        tier: "official".to_string(),
    }
}

fn parse_grok_models_output(output: &str) -> Vec<GrokOfficialModel> {
    let mut models = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut in_available_section = false;

    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.to_ascii_lowercase().starts_with("available models") {
            in_available_section = true;
            continue;
        }
        if !in_available_section {
            continue;
        }
        let entry = line
            .strip_prefix('-')
            .or_else(|| line.strip_prefix('*'))
            .map(str::trim)
            .unwrap_or("");
        if entry.is_empty() {
            continue;
        }
        let model_id = entry
            .split_whitespace()
            .next()
            .unwrap_or(entry)
            .trim()
            .to_string();
        if model_id.is_empty() || !seen.insert(model_id.clone()) {
            continue;
        }
        models.push(GrokOfficialModel {
            id: model_id.clone(),
            name: Some(model_id),
            owned_by: Some("xai".to_string()),
            created: None,
        });
    }
    models
}

async fn run_grok_models_command(db: &SqliteDbState) -> Result<String, String> {
    use crate::coding::cli_resolver::{
        apply_create_no_window_tokio, build_local_tokio_command, resolve_local_grok_program,
    };
    use crate::coding::runtime_location::RuntimeLocationMode;
    use tokio::process::Command;

    let location = runtime_location::get_grok_runtime_location_async(db).await?;
    let mut command = match location.mode {
        RuntimeLocationMode::LocalWindows => {
            let program = resolve_local_grok_program();
            let mut command = build_local_tokio_command(&program.path);
            command.arg("models").env("GROK_HOME", &location.host_path);
            command
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = location.wsl.as_ref().ok_or_else(|| {
                "Missing WSL runtime metadata for Grok models command".to_string()
            })?;
            let mut command = Command::new("wsl");
            apply_create_no_window_tokio(&mut command);
            command
                .args(["-d", &wsl.distro, "--exec", "env"])
                .arg(format!("GROK_HOME={}", wsl.linux_path))
                .args(["grok", "models"]);
            command
        }
    };
    let output = command
        .output()
        .await
        .map_err(|error| format!("Failed to run grok models: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() { stdout } else { stderr })
}

#[tauri::command]
pub async fn read_grok_settings(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<GrokSettings, String> {
    // Preview must show the exact live runtime files with no redaction.
    let db = state.db();
    let config_path = get_grok_config_path_async(db).await?;
    let auth_path = get_grok_auth_path_async(db).await?;
    let config = read_optional_text(&config_path)?;
    let auth = read_optional_json(&auth_path)?;
    Ok(GrokSettings { auth, config })
}

#[tauri::command]
pub async fn list_grok_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GrokProvider>, String> {
    let providers = list_grok_providers_for_db(state.db())?;
    if !providers.is_empty() {
        return Ok(providers);
    }

    match load_temp_grok_provider_from_file(state.db()).await {
        Ok(provider) => Ok(vec![provider]),
        Err(error) if error == "No local Grok provider config found" => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

pub fn list_grok_providers_for_db(db: &SqliteDbState) -> Result<Vec<GrokProvider>, String> {
    let order = provider_order()?;
    db.with_conn(|conn| db_list(conn, DbTable::GrokProvider, Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .map(adapter::provider_from_db_value)
                .collect()
        })
}

#[tauri::command]
pub async fn create_grok_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: GrokProviderInput,
) -> Result<GrokProvider, String> {
    validate_provider_settings(&provider.settings_config, &provider.category)?;
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let sort_index = match provider.sort_index {
        Some(value) => Some(value),
        None => Some(next_sort_index(db, DbTable::GrokProvider)?),
    };
    let content = GrokProviderContent {
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index,
        meta: provider.meta,
        is_applied: false,
        is_disabled: provider.is_disabled.unwrap_or(false),
        created_at: now.clone(),
        updated_at: now,
    };
    let id = db_new_id();
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::GrokProvider,
            &id,
            &adapter::provider_to_db_value(&content),
        )
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(provider_from_content(id, content))
}

#[tauri::command]
pub async fn update_grok_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: GrokProvider,
) -> Result<GrokProvider, String> {
    validate_provider_settings(&provider.settings_config, &provider.category)?;
    if provider.id == GROK_LOCAL_PROVIDER_ID {
        return Err("Local Grok provider must be saved before it can be updated".to_string());
    }
    let db = state.db();
    let existing = get_provider(db, &provider.id)?
        .ok_or_else(|| format!("Grok provider '{}' not found", provider.id))?;
    let previous_settings_config = existing.settings_config.clone();
    let previous_category = existing.category.clone();
    let content = GrokProviderContent {
        name: provider.name,
        category: provider.category,
        settings_config: provider.settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        meta: provider.meta,
        is_applied: existing.is_applied,
        is_disabled: provider.is_disabled,
        created_at: existing.created_at,
        updated_at: Local::now().to_rfc3339(),
    };
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::GrokProvider,
            &provider.id,
            &adapter::provider_to_db_value(&content),
        )
    })?;
    if content.is_applied {
        apply_grok_provider_to_file_with_previous_settings(
            db,
            &provider.id,
            Some(&previous_settings_config),
            Some(&previous_category),
            None,
        )
        .await?;
        emit_grok_sync(&app);
    }
    let _ = app.emit("config-changed", "window");
    Ok(provider_from_content(provider.id, content))
}

#[tauri::command]
pub async fn delete_grok_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    if id == GROK_LOCAL_PROVIDER_ID {
        return Err("Local Grok provider must be saved before it can be deleted".to_string());
    }
    let has_accounts = state.db().with_conn(|conn| {
        let accounts = db_list(conn, DbTable::GrokOfficialAccount, None)?;
        Ok(accounts
            .iter()
            .any(|account| account.get("provider_id").and_then(Value::as_str) == Some(id.as_str())))
    })?;
    if has_accounts {
        return Err("Delete the Grok official accounts before deleting this provider".to_string());
    }
    state
        .db()
        .with_conn(|conn| db_delete(conn, DbTable::GrokProvider, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn reorder_grok_providers(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    for (index, id) in ids.iter().enumerate() {
        state.db().with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::GrokProvider,
                id,
                &[
                    ("sort_index", json!(index as i64)),
                    ("updated_at", Value::String(now.clone())),
                ],
            )
            .map(|_| ())
        })?;
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_grok_provider_disabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
    disabled: bool,
) -> Result<(), String> {
    let provider =
        get_provider(state.db(), &id)?.ok_or_else(|| format!("Grok provider '{id}' not found"))?;
    if provider.is_applied && disabled {
        return Err("The applied Grok provider cannot be disabled".to_string());
    }
    state.db().with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::GrokProvider,
            &id,
            &[("is_disabled", Value::Bool(disabled))],
        )
        .map(|_| ())
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn select_grok_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    select_grok_provider_internal(state.inner(), &app, &id).await
}

pub async fn select_grok_provider_internal<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    id: &str,
) -> Result<(), String> {
    select_grok_provider_internal_with_sync(state, app, id, false, true).await
}

pub async fn select_grok_provider_internal_with_sync<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    id: &str,
    from_tray: bool,
    emit_events: bool,
) -> Result<(), String> {
    let provider =
        get_provider(state, id)?.ok_or_else(|| format!("Grok provider '{id}' not found"))?;
    if provider.is_disabled {
        return Err("Disabled Grok provider cannot be applied".to_string());
    }
    apply_grok_provider_to_file(state, id).await?;
    let now = Local::now().to_rfc3339();
    state.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::GrokProvider, Some(id), &now)
    })?;
    // Mirror Codex: only official runtime can own account "applied" tags.
    // Switching to custom/local must clear stale official-account applied markers.
    if provider.category == "official" {
        super::official_accounts::sync_grok_official_account_apply_status(state, id).await?;
    } else {
        super::official_accounts::clear_all_grok_official_account_apply_status(state).await?;
    }
    if emit_events {
        let _ = app.emit("config-changed", if from_tray { "tray" } else { "window" });
        emit_grok_sync(app);
    }
    Ok(())
}

pub async fn select_grok_provider_internal_without_events<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    id: &str,
) -> Result<(), String> {
    select_grok_provider_internal_with_sync(state, app, id, false, false).await
}

pub async fn select_grok_model_internal<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    model_key: &str,
) -> Result<(), String> {
    let normalized_model_key = model_key.trim();
    if normalized_model_key.is_empty() {
        return Err("Grok model key is required".to_string());
    }
    let provider =
        get_applied_provider(state)?.ok_or_else(|| "No applied Grok provider found".to_string())?;
    if provider.is_disabled {
        return Err("Disabled Grok provider cannot change models".to_string());
    }
    let mut settings: Value = serde_json::from_str(&provider.settings_config)
        .map_err(|error| format!("Invalid Grok provider settings JSON: {error}"))?;
    // Official providers do not persist a custom modelCatalog; they only store
    // defaultModelKey and project it to [models].default. Custom providers must
    // keep the selected key inside modelCatalog.models.
    let model_exists = settings
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models.iter().any(|model| {
                model
                    .get("key")
                    .or_else(|| model.get("model"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    == Some(normalized_model_key)
            })
        });
    if provider.category != "official" && !model_exists {
        return Err(format!(
            "Grok model '{normalized_model_key}' is not in the applied provider catalog"
        ));
    }
    settings["defaultModelKey"] = Value::String(normalized_model_key.to_string());
    let next_settings_config = serde_json::to_string(&settings)
        .map_err(|error| format!("Failed to serialize Grok provider settings: {error}"))?;
    validate_provider_settings(&next_settings_config, &provider.category)?;
    let updated_at = Local::now().to_rfc3339();
    state.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::GrokProvider,
            &provider.id,
            &[
                ("settings_config", Value::String(next_settings_config)),
                ("updated_at", Value::String(updated_at)),
            ],
        )
        .map(|_| ())
    })?;
    apply_grok_provider_to_file_with_previous_settings(
        state,
        &provider.id,
        Some(&provider.settings_config),
        Some(provider.category.as_str()),
        None,
    )
    .await?;
    let _ = app.emit("config-changed", "tray");
    emit_grok_sync(app);
    Ok(())
}

pub async fn apply_grok_provider_to_file(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let previous_common_config = get_common_config(db)?.map(|value| value.config);
    apply_grok_provider_to_file_with_previous_settings(
        db,
        provider_id,
        None,
        None,
        previous_common_config.as_deref(),
    )
    .await
}

async fn apply_grok_provider_to_file_with_previous_settings(
    db: &SqliteDbState,
    provider_id: &str,
    previous_settings_config: Option<&str>,
    previous_category: Option<&str>,
    previous_common_config: Option<&str>,
) -> Result<(), String> {
    let provider = get_provider(db, provider_id)?
        .ok_or_else(|| format!("Grok provider '{provider_id}' not found"))?;
    let settings: Value = serde_json::from_str(&provider.settings_config)
        .map_err(|error| format!("Invalid Grok provider settings JSON: {error}"))?;
    let config_path = get_grok_config_path_async(db).await?;
    let current = read_optional_text(&config_path)?.unwrap_or_default();
    let mut document = if current.trim().is_empty() {
        DocumentMut::new()
    } else {
        current
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid live Grok config.toml: {error}"))?
    };

    // settings_config never stores category (it lives on the provider row). Callers must
    // pass previous_category; falling back to get_applied_provider also supplies it.
    // Defaulting to "custom" would wrongly require modelCatalog when cleaning official.
    //
    // Provider-owned [model.<key>] tables ARE the channel config. Apply always removes
    // previous catalog keys and rewrites next catalog — never "preserve user edits"
    // for those keys, or switching/saving channels leaves stale base_url/api_backend.
    // Truly local models (keys never in previous provider catalog) remain untouched.
    if let Some(previous_settings_config) = previous_settings_config {
        let previous_category = previous_category.unwrap_or("custom");
        remove_provider_model_tables(&mut document, previous_settings_config, previous_category)?;
        remove_previous_provider_config(&mut document, previous_settings_config)?;
    } else if let Some(previous) = get_applied_provider(db)? {
        remove_provider_model_tables(&mut document, &previous.settings_config, &previous.category)?;
        remove_previous_provider_config(&mut document, &previous.settings_config)?;
    }
    let common = get_common_config(db)?;
    let previous_common = previous_common_config
        .map(str::to_string)
        .or_else(|| common.as_ref().map(|value| value.config.clone()));
    if let Some(previous_common_config) = previous_common.as_deref() {
        remove_matching_unmanaged_config(&mut document, previous_common_config)?;
    }
    if let Some(common) = common {
        merge_common_config(&mut document, &common.config)?;
    }
    merge_provider_config(&mut document, &settings)?;
    project_provider_models(&mut document, &settings, &provider.category)?;
    write_text_atomic(&config_path, &document.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_grok_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<GrokCommonConfig>, String> {
    get_common_config(state.db())
}

#[tauri::command]
pub async fn extract_grok_common_config_from_current_file(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<GrokCommonConfig, String> {
    // Read only config.toml with timeout; do not touch auth.json.
    let path = get_grok_config_path_async(state.db()).await?;
    let current =
        crate::coding::file_io::read_text_file_with_timeout(path, "Grok config.toml").await?;
    let mut document = if current.trim().is_empty() {
        DocumentMut::new()
    } else {
        current
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid Grok config.toml: {error}"))?
    };
    document.remove("model");
    document.remove("mcp_servers");
    document.remove("plugins");
    document.remove("marketplace");
    if let Some(models) = document.get_mut("models").and_then(Item::as_table_mut) {
        models.remove("default");
        if models.is_empty() {
            document.remove("models");
        }
    }
    let existing = get_common_config(state.db())?;
    Ok(GrokCommonConfig {
        config: document.to_string(),
        root_dir: existing.and_then(|value| value.root_dir),
        updated_at: Local::now().to_rfc3339(),
    })
}

#[tauri::command]
pub async fn save_grok_common_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokCommonConfigInput,
) -> Result<(), String> {
    validate_common_config(&input.config)?;
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(db, "grok").await;
    let existing_common = get_common_config(db)?;
    let previous_common_config = existing_common.as_ref().map(|value| value.config.clone());
    let existing_root = existing_common.and_then(|value| value.root_dir);
    let root_dir = if input.clear_root_dir {
        None
    } else {
        input
            .root_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or(existing_root)
    };
    let value = adapter::common_to_db_value(&input.config, root_dir.as_deref());
    db.with_conn(|conn| db_put(conn, DbTable::GrokCommonConfig, "common", &value))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(db, "grok").await?;
    if let Some(provider) = get_applied_provider(db)? {
        apply_grok_provider_to_file_with_previous_settings(
            db,
            &provider.id,
            None,
            None,
            previous_common_config.as_deref(),
        )
        .await?;
    } else {
        let path = get_grok_config_path_async(db).await?;
        write_text_atomic(&path, &input.config)?;
    }
    resync_all_skills_if_tool_path_changed(
        app.clone(),
        state.inner(),
        "grok",
        previous_skills_path,
    )
    .await;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}

#[tauri::command]
pub async fn save_grok_local_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokLocalConfigInput,
) -> Result<(), String> {
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(db, "grok").await;
    let live_snapshot = load_local_grok_provider_snapshot(db).await?;
    let provider_input = input.provider;
    let settings_config = provider_input
        .as_ref()
        .map(|provider| provider.settings_config.clone())
        .unwrap_or_else(|| live_snapshot.settings_config.clone());
    let provider_category = provider_input
        .as_ref()
        .map(|provider| provider.category.clone())
        .unwrap_or_else(|| live_snapshot.category.clone());
    validate_provider_settings(&settings_config, &provider_category)?;
    let now = Local::now().to_rfc3339();
    let provider_content = GrokProviderContent {
        name: provider_input
            .as_ref()
            .map(|provider| provider.name.clone())
            .unwrap_or(live_snapshot.name),
        category: provider_category,
        settings_config,
        source_provider_id: provider_input
            .as_ref()
            .and_then(|provider| provider.source_provider_id.clone()),
        website_url: provider_input
            .as_ref()
            .and_then(|provider| provider.website_url.clone()),
        notes: provider_input
            .as_ref()
            .and_then(|provider| provider.notes.clone()),
        icon: provider_input
            .as_ref()
            .and_then(|provider| provider.icon.clone()),
        icon_color: provider_input
            .as_ref()
            .and_then(|provider| provider.icon_color.clone()),
        sort_index: provider_input
            .as_ref()
            .and_then(|provider| provider.sort_index)
            .or(Some(next_sort_index(db, DbTable::GrokProvider)?)),
        meta: provider_input
            .as_ref()
            .and_then(|provider| provider.meta.clone())
            .or(live_snapshot.meta),
        is_applied: true,
        is_disabled: provider_input
            .as_ref()
            .and_then(|provider| provider.is_disabled)
            .unwrap_or(false),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let provider_id = db_new_id();
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::GrokProvider,
            &provider_id,
            &adapter::provider_to_db_value(&provider_content),
        )
    })?;

    let existing_common = get_common_config(db)?;
    let previous_common_config = live_snapshot.common_config.clone();
    let common_config = input.common_config.unwrap_or(live_snapshot.common_config);
    validate_common_config(&common_config)?;
    let root_dir = if input.clear_root_dir {
        None
    } else {
        input
            .root_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| existing_common.and_then(|value| value.root_dir))
    };
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::GrokCommonConfig,
            "common",
            &adapter::common_to_db_value(&common_config, root_dir.as_deref()),
        )
    })?;
    runtime_location::refresh_runtime_location_cache_for_module_async(db, "grok").await?;
    apply_grok_provider_to_file_with_previous_settings(
        db,
        &provider_id,
        Some(&live_snapshot.settings_config),
        Some(live_snapshot.category.as_str()),
        Some(&previous_common_config),
    )
    .await?;
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::GrokProvider, Some(&provider_id), &now)
    })?;
    if provider_content.category == "official" {
        super::official_accounts::sync_grok_official_account_apply_status(db, &provider_id).await?;
    } else {
        super::official_accounts::clear_all_grok_official_account_apply_status(db).await?;
    }
    resync_all_skills_if_tool_path_changed(
        app.clone(),
        state.inner(),
        "grok",
        previous_skills_path,
    )
    .await;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}

#[tauri::command]
pub async fn list_grok_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OpenCodeAllApiHubProvidersResult, String> {
    crate::coding::open_code::commands::list_opencode_all_api_hub_providers(state).await
}

#[tauri::command]
pub async fn resolve_grok_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
    request: ResolveOpenCodeAllApiHubProvidersRequest,
) -> Result<Vec<OpenCodeAllApiHubProvider>, String> {
    crate::coding::open_code::commands::resolve_opencode_all_api_hub_providers(state, request).await
}

#[tauri::command]
pub async fn list_grok_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GrokPromptConfig>, String> {
    let order = prompt_order()?;
    let prompts = state
        .db()
        .with_conn(|conn| db_list(conn, DbTable::GrokPromptConfig, Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .map(adapter::prompt_from_db_value)
                .collect::<Vec<_>>()
        })?;
    if prompts.is_empty() {
        if let Some(local_config) = get_local_prompt_config(state.db()).await? {
            return Ok(vec![local_config]);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_grok_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPromptConfigInput,
) -> Result<GrokPromptConfig, String> {
    let now = Local::now().to_rfc3339();
    let content = GrokPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index(state.db(), DbTable::GrokPromptConfig)?),
        created_at: now.clone(),
        updated_at: now,
    };
    let id = db_new_id();
    state.db().with_conn(|conn| {
        db_put(
            conn,
            DbTable::GrokPromptConfig,
            &id,
            &adapter::prompt_to_db_value(&content),
        )
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(prompt_from_content(id, content))
}

#[tauri::command]
pub async fn update_grok_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPromptConfigInput,
) -> Result<GrokPromptConfig, String> {
    let id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let existing =
        get_prompt(state.db(), &id)?.ok_or_else(|| format!("Grok prompt '{id}' not found"))?;
    let content = GrokPromptConfigContent {
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
            DbTable::GrokPromptConfig,
            &id,
            &adapter::prompt_to_db_value(&content),
        )
    })?;
    if content.is_applied {
        write_text_atomic(
            &get_grok_prompt_path_async(state.db()).await?,
            &content.content,
        )?;
        emit_grok_sync(&app);
    }
    let _ = app.emit("config-changed", "window");
    Ok(prompt_from_content(id, content))
}

#[tauri::command]
pub async fn delete_grok_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    // Only delete the DB prompt record.
    // Keep the live AGENTS.md on disk so deleting a saved prompt never wipes the local runtime file.
    // This matches Claude Code / OpenCode (DB-only delete) and avoids treating "delete record"
    // as "delete local prompt file".
    state
        .db()
        .with_conn(|conn| db_delete(conn, DbTable::GrokPromptConfig, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn save_grok_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPromptConfigInput,
) -> Result<GrokPromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        get_local_prompt_config(state.db())
            .await?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };

    let created = create_grok_prompt_config(
        state.clone(),
        app.clone(),
        GrokPromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;

    apply_grok_prompt_config_internal(state.inner(), &app, &created.id).await?;
    Ok(get_prompt(state.db(), &created.id)?.unwrap_or(created))
}

#[tauri::command]
pub async fn apply_grok_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_grok_prompt_config_internal(state.inner(), &app, &config_id).await
}

pub async fn apply_grok_prompt_config_internal<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_grok_prompt_config_internal_with_events(state, app, config_id, true).await
}

pub async fn apply_grok_prompt_config_internal_without_events<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_grok_prompt_config_internal_with_events(state, app, config_id, false).await
}

async fn apply_grok_prompt_config_internal_with_events<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    emit_events: bool,
) -> Result<(), String> {
    let prompt = get_prompt(state, config_id)?
        .ok_or_else(|| format!("Grok prompt '{config_id}' not found"))?;
    write_text_atomic(&get_grok_prompt_path_async(state).await?, &prompt.content)?;
    let now = Local::now().to_rfc3339();
    state.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::GrokPromptConfig, Some(config_id), &now)
    })?;
    if emit_events {
        let _ = app.emit("config-changed", "window");
        emit_grok_sync(app);
    }
    Ok(())
}

#[tauri::command]
pub async fn reorder_grok_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        state.db().with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::GrokPromptConfig,
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    Ok(())
}

struct LocalGrokProviderSnapshot {
    name: String,
    category: String,
    settings_config: String,
    common_config: String,
    meta: Option<Value>,
}

async fn load_local_grok_provider_snapshot(
    db: &SqliteDbState,
) -> Result<LocalGrokProviderSnapshot, String> {
    let config_path = get_grok_config_path_async(db).await?;
    let config_text = read_optional_text(&config_path)?.unwrap_or_default();
    parse_local_grok_provider_snapshot(&config_text)
}

fn parse_local_grok_provider_snapshot(
    config_text: &str,
) -> Result<LocalGrokProviderSnapshot, String> {
    if config_text.trim().is_empty() {
        return Err("No local Grok provider config found".to_string());
    }

    let document = config_text
        .parse::<DocumentMut>()
        .map_err(|error| format!("Invalid local Grok config.toml: {error}"))?;
    let default_model_key = document
        .get("models")
        .and_then(Item::as_table)
        .and_then(|models| models.get("default"))
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let model_tables = document.get("model").and_then(Item::as_table);
    let mut catalog_models = Vec::new();
    // Provider-level auth.API_KEY is the form field; live Grok stores api_key on each [model.*].
    // Lift a shared model-level key into auth so Local Grok edit/save can round-trip it.
    // Keep per-model keys out of modelCatalog/extraConfig. Only lift when every model has the same non-empty key.
    let mut model_api_keys: Vec<Option<String>> = Vec::new();

    if let Some(model_tables) = model_tables {
        for (model_key, model_item) in model_tables.iter() {
            let Some(model_table) = model_item.as_table() else {
                continue;
            };
            let mut catalog_model = serde_json::Map::new();
            catalog_model.insert("key".to_string(), Value::String(model_key.to_string()));
            let mut extra_config = serde_json::Map::new();
            let mut model_api_key: Option<String> = None;
            for (field, item) in model_table.iter() {
                if field == "api_key" {
                    model_api_key = item
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string);
                    continue;
                }
                let field_value = toml_item_to_json(item)?;
                let target_field = match field {
                    "model" => Some("model"),
                    "name" => Some("displayName"),
                    "description" => Some("description"),
                    "base_url" => Some("baseUrl"),
                    "api_backend" => Some("apiBackend"),
                    "env_key" => Some("envKey"),
                    "context_window" => Some("contextWindow"),
                    "max_completion_tokens" => Some("maxCompletionTokens"),
                    "temperature" => Some("temperature"),
                    "top_p" => Some("topP"),
                    "supports_backend_search" => Some("supportsBackendSearch"),
                    "supports_reasoning_effort" => Some("supportsReasoningEffort"),
                    "reasoning_effort" => Some("reasoningEffort"),
                    "stream_tool_calls" => Some("streamToolCalls"),
                    "max_retries" => Some("maxRetries"),
                    "inference_idle_timeout_secs" => Some("inferenceIdleTimeoutSecs"),
                    "extra_headers" => Some("extraHeaders"),
                    _ => None,
                };
                if let Some(target_field) = target_field {
                    catalog_model.insert(target_field.to_string(), field_value);
                } else {
                    extra_config.insert(field.to_string(), field_value);
                }
            }
            if !catalog_model.contains_key("model") {
                catalog_model.insert("model".to_string(), Value::String(model_key.to_string()));
            }
            if !extra_config.is_empty() {
                catalog_model.insert("extraConfig".to_string(), Value::Object(extra_config));
            }
            model_api_keys.push(model_api_key);
            catalog_models.push(Value::Object(catalog_model));
        }
    }

    let category = if catalog_models.is_empty() {
        "official"
    } else {
        "custom"
    };
    let shared_api_key = model_api_keys
        .first()
        .and_then(|first| first.as_ref())
        .filter(|_| {
            model_api_keys
                .iter()
                .all(|key| key.as_ref() == model_api_keys[0].as_ref())
        })
        .cloned();
    let mut settings = json!({
        "auth": {},
        "config": "",
    });
    if !catalog_models.is_empty() {
        if let Some(default_model_key) = default_model_key.as_deref() {
            settings["defaultModelKey"] = Value::String(default_model_key.to_string());
        }
        settings["modelCatalog"] = json!({ "models": catalog_models });
        if let Some(api_key) = shared_api_key {
            settings["auth"] = json!({ "API_KEY": api_key });
        }
    }
    let settings_config = serde_json::to_string(&settings)
        .map_err(|error| format!("Failed to serialize local Grok provider: {error}"))?;
    validate_provider_settings(&settings_config, category)?;

    let mut common_document = document;
    common_document.remove("model");
    common_document.remove("mcp_servers");
    common_document.remove("plugins");
    common_document.remove("marketplace");
    if let Some(models) = common_document
        .get_mut("models")
        .and_then(Item::as_table_mut)
    {
        models.remove("default");
        if models.is_empty() {
            common_document.remove("models");
        }
    }

    Ok(LocalGrokProviderSnapshot {
        name: "Local Grok".to_string(),
        category: category.to_string(),
        settings_config,
        common_config: common_document.to_string(),
        meta: None,
    })
}

async fn load_temp_grok_provider_from_file(db: &SqliteDbState) -> Result<GrokProvider, String> {
    let snapshot = load_local_grok_provider_snapshot(db).await?;
    let now = Local::now().to_rfc3339();
    Ok(GrokProvider {
        id: GROK_LOCAL_PROVIDER_ID.to_string(),
        name: snapshot.name,
        category: snapshot.category,
        settings_config: snapshot.settings_config,
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: snapshot.meta,
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

fn toml_item_to_json(item: &Item) -> Result<Value, String> {
    let mut document = DocumentMut::new();
    document.insert("holder", item.clone());
    let parsed: toml::Value = toml::from_str(&document.to_string())
        .map_err(|error| format!("Failed to parse Grok model field: {error}"))?;
    let value = parsed
        .get("holder")
        .cloned()
        .ok_or_else(|| "Failed to read Grok model field".to_string())?;
    serde_json::to_value(value)
        .map_err(|error| format!("Failed to convert Grok model field: {error}"))
}

fn get_provider(db: &SqliteDbState, id: &str) -> Result<Option<GrokProvider>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::GrokProvider, id))
        .map(|value| value.map(adapter::provider_from_db_value))
}

fn get_applied_provider(db: &SqliteDbState) -> Result<Option<GrokProvider>, String> {
    Ok(list_grok_providers_for_db(db)?
        .into_iter()
        .find(|provider| provider.is_applied))
}

fn get_common_config(db: &SqliteDbState) -> Result<Option<GrokCommonConfig>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::GrokCommonConfig, "common"))
        .map(|value| value.map(adapter::common_from_db_value))
}

fn get_prompt(db: &SqliteDbState, id: &str) -> Result<Option<GrokPromptConfig>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::GrokPromptConfig, id))
        .map(|value| value.map(adapter::prompt_from_db_value))
}

fn provider_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::created_at(OrderDirection::Asc),
    ]))
}

fn prompt_order() -> Result<OrderSpec, String> {
    provider_order()
}

fn next_sort_index(db: &SqliteDbState, table: DbTable) -> Result<i32, String> {
    db.with_conn(|conn| {
        Ok(db_max_i64(conn, table, &JsonFieldPath::new("sort_index")?)?
            .map(|value| value as i32 + 1)
            .unwrap_or(0))
    })
}

fn provider_from_content(id: String, content: GrokProviderContent) -> GrokProvider {
    GrokProvider {
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
        meta: content.meta,
        is_applied: content.is_applied,
        is_disabled: content.is_disabled,
        created_at: content.created_at,
        updated_at: content.updated_at,
    }
}

fn prompt_from_content(id: String, content: GrokPromptConfigContent) -> GrokPromptConfig {
    GrokPromptConfig {
        id,
        name: content.name,
        content: content.content,
        is_applied: content.is_applied,
        sort_index: content.sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    }
}

fn validate_provider_settings(settings_config: &str, category: &str) -> Result<(), String> {
    let settings: Value = serde_json::from_str(settings_config)
        .map_err(|error| format!("Invalid Grok provider settings JSON: {error}"))?;
    if let Some(config) = settings.get("config").and_then(Value::as_str) {
        if !config.trim().is_empty() {
            let document = config
                .parse::<DocumentMut>()
                .map_err(|error| format!("Invalid Grok provider TOML: {error}"))?;
            validate_unmanaged_grok_config(&document, "provider")?;
        }
    }
    // Official providers only store defaultModelKey and project it to [models].default.
    // They must NOT require modelCatalog. Custom providers need catalog entries when a
    // default model key is set so apply can write [model.<key>] tables.
    let has_default_model_key = settings
        .get("defaultModelKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_model_catalog = settings
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .is_some_and(|models| !models.is_empty());
    if category != "official" && has_default_model_key && !has_model_catalog {
        return Err("Grok modelCatalog.models is required when defaultModelKey is set".to_string());
    }
    Ok(())
}

fn validate_common_config(config: &str) -> Result<(), String> {
    if config.trim().is_empty() {
        return Ok(());
    }
    let document = config
        .parse::<DocumentMut>()
        .map_err(|error| format!("Invalid Grok common TOML: {error}"))?;
    validate_unmanaged_grok_config(&document, "common")
}

fn validate_unmanaged_grok_config(document: &DocumentMut, owner: &str) -> Result<(), String> {
    for protected in ["model", "mcp_servers", "plugins", "marketplace"] {
        if document.get(protected).is_some() {
            return Err(format!(
                "Grok {owner} config cannot manage protected section [{protected}]"
            ));
        }
    }
    if document
        .get("models")
        .and_then(Item::as_table)
        .is_some_and(|models| models.contains_key("default"))
    {
        return Err(format!(
            "Grok {owner} config cannot manage protected field [models].default"
        ));
    }
    Ok(())
}

fn remove_provider_model_tables(
    document: &mut DocumentMut,
    settings_config: &str,
    category: &str,
) -> Result<(), String> {
    let settings: Value = serde_json::from_str(settings_config)
        .map_err(|error| format!("Invalid previous Grok provider settings JSON: {error}"))?;
    // Official providers do not own [model.*] tables; only [models].default.
    if category == "official" {
        return Ok(());
    }
    // Always force-remove previous provider catalog keys. [model.<key>] is channel config;
    // switching/saving must not keep "user-edited" tables for those keys.
    let keys = settings
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            model
                .get("key")
                .or_else(|| model.get("model"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    if let Some(model_tables) = document.get_mut("model").and_then(Item::as_table_mut) {
        for key in keys {
            model_tables.remove(&key);
        }
        if model_tables.is_empty() {
            document.remove("model");
        }
    }
    Ok(())
}

fn remove_previous_provider_config(
    document: &mut DocumentMut,
    settings_config: &str,
) -> Result<(), String> {
    let settings: Value = serde_json::from_str(settings_config)
        .map_err(|error| format!("Invalid previous Grok provider settings JSON: {error}"))?;
    let config = settings
        .get("config")
        .and_then(Value::as_str)
        .unwrap_or_default();
    remove_matching_unmanaged_config(document, config)
}

fn remove_matching_unmanaged_config(
    document: &mut DocumentMut,
    previous_config: &str,
) -> Result<(), String> {
    if previous_config.trim().is_empty() {
        return Ok(());
    }
    let mut previous_document = previous_config
        .parse::<DocumentMut>()
        .map_err(|error| format!("Invalid previous Grok managed TOML: {error}"))?;
    for protected in ["model", "mcp_servers", "plugins", "marketplace"] {
        previous_document.remove(protected);
    }
    if let Some(models) = previous_document
        .get_mut("models")
        .and_then(Item::as_table_mut)
    {
        models.remove("default");
        if models.is_empty() {
            previous_document.remove("models");
        }
    }
    remove_matching_table_items(document.as_table_mut(), previous_document.as_table());
    Ok(())
}

fn remove_matching_table_items(target: &mut Table, previous: &Table) {
    // Codex-style aggressive removal: once a field was managed, remove it on the
    // next apply even if the live value diverged from the previous managed value.
    // User-edited [model.*] tables are handled separately by remove_provider_model_tables.
    let previous_keys = previous
        .iter()
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();
    for key in previous_keys {
        let Some(previous_item) = previous.get(&key) else {
            continue;
        };
        let remove_key = match (target.get_mut(&key), previous_item.as_table()) {
            (Some(target_item), Some(previous_table)) => {
                if let Some(target_table) = target_item.as_table_mut() {
                    remove_matching_table_items(target_table, previous_table);
                    target_table.is_empty()
                } else {
                    true
                }
            }
            (Some(_target_item), None) => true,
            (None, _) => false,
        };
        if remove_key {
            target.remove(&key);
        }
    }
}

async fn get_local_prompt_config(db: &SqliteDbState) -> Result<Option<GrokPromptConfig>, String> {
    let prompt_path = get_grok_prompt_path_async(db).await?;
    let Some(content) = read_optional_text(&prompt_path)? else {
        return Ok(None);
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    let now = Local::now().to_rfc3339();
    Ok(Some(GrokPromptConfig {
        id: GROK_LOCAL_PROVIDER_ID.to_string(),
        name: "default".to_string(),
        content,
        is_applied: true,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }))
}

fn merge_common_config(document: &mut DocumentMut, common: &str) -> Result<(), String> {
    if common.trim().is_empty() {
        return Ok(());
    }
    let mut common_document = common
        .parse::<DocumentMut>()
        .map_err(|error| format!("Invalid stored Grok common TOML: {error}"))?;
    let keys = common_document
        .iter()
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();
    for key in keys {
        if matches!(
            key.as_str(),
            "model" | "mcp_servers" | "plugins" | "marketplace"
        ) {
            continue;
        }
        if key == "models" {
            let Some(source_models) = common_document
                .get_mut("models")
                .and_then(Item::as_table_mut)
            else {
                continue;
            };
            let target_models = document["models"].or_insert(Item::Table(Table::new()));
            let target_models = target_models
                .as_table_mut()
                .ok_or_else(|| "Live Grok [models] must be a table".to_string())?;
            let model_keys = source_models
                .iter()
                .map(|(field, _)| field.to_string())
                .collect::<Vec<_>>();
            for field in model_keys {
                if field != "default" {
                    if let Some(item) = source_models.remove(&field) {
                        target_models.insert(&field, item);
                    }
                }
            }
            continue;
        }
        if let Some(item) = common_document.remove(&key) {
            document.insert(&key, item);
        }
    }
    Ok(())
}

fn merge_provider_config(document: &mut DocumentMut, settings: &Value) -> Result<(), String> {
    let Some(config) = settings.get("config").and_then(Value::as_str) else {
        return Ok(());
    };
    merge_common_config(document, config)
}

fn project_provider_models(
    document: &mut DocumentMut,
    settings: &Value,
    category: &str,
) -> Result<(), String> {
    let default_model_key = settings
        .get("defaultModelKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("grok-4.5");
    document["models"]["default"] = value(default_model_key);
    if category == "official" {
        return Ok(());
    }
    let api_key = settings
        .pointer("/auth/API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let models = settings
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .ok_or_else(|| "Grok custom provider requires modelCatalog.models".to_string())?;
    let model_root = document["model"].or_insert(Item::Table(Table::new()));
    let model_root = model_root
        .as_table_mut()
        .ok_or_else(|| "Live Grok [model] must be a table".to_string())?;
    for model in models {
        let key = model
            .get("key")
            .or_else(|| model.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Grok model entry requires key or model".to_string())?;
        let mut table = Table::new();
        insert_known_model_fields(&mut table, model, api_key)?;
        if let Some(extra) = model.get("extraConfig").and_then(Value::as_object) {
            for (field, value) in extra {
                if !table.contains_key(field) {
                    table.insert(field, json_to_toml_item(value)?);
                }
            }
        }
        model_root.insert(key, Item::Table(table));
    }
    Ok(())
}

fn insert_known_model_fields(
    table: &mut Table,
    model: &Value,
    fallback_api_key: Option<&str>,
) -> Result<(), String> {
    let mappings = [
        ("model", "model"),
        ("displayName", "name"),
        ("description", "description"),
        ("baseUrl", "base_url"),
        ("apiBackend", "api_backend"),
        ("envKey", "env_key"),
        ("contextWindow", "context_window"),
        ("maxCompletionTokens", "max_completion_tokens"),
        ("temperature", "temperature"),
        ("topP", "top_p"),
        ("supportsBackendSearch", "supports_backend_search"),
        ("supportsReasoningEffort", "supports_reasoning_effort"),
        ("reasoningEffort", "reasoning_effort"),
        ("streamToolCalls", "stream_tool_calls"),
        ("maxRetries", "max_retries"),
        ("inferenceIdleTimeoutSecs", "inference_idle_timeout_secs"),
        ("extraHeaders", "extra_headers"),
    ];
    for (json_key, toml_key) in mappings {
        if let Some(value) = model.get(json_key).filter(|value| !value.is_null()) {
            table.insert(toml_key, json_to_toml_item(value)?);
        }
    }
    let explicit_api_key = model
        .get("apiKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(api_key) = explicit_api_key.or(fallback_api_key) {
        table.insert("api_key", value(api_key));
    }
    Ok(())
}

fn json_to_toml_item(value: &Value) -> Result<Item, String> {
    let serialized = toml::to_string(&json!({ "holder": value }))
        .map_err(|error| format!("Failed to serialize Grok model field: {error}"))?;
    let mut document = serialized
        .parse::<DocumentMut>()
        .map_err(|error| format!("Failed to build Grok model TOML field: {error}"))?;
    document
        .remove("holder")
        .ok_or_else(|| "Failed to build Grok model TOML field".to_string())
}

fn read_optional_text(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))
}

fn read_optional_json(path: &Path) -> Result<Option<Value>, String> {
    let Some(content) = read_optional_text(path)? else {
        return Ok(None);
    };
    if content.trim().is_empty() {
        return Ok(Some(json!({})));
    }
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))
}

fn write_text_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("Failed to create temp file for {}: {error}", path.display()))?;
    temporary
        .write_all(content.as_bytes())
        .map_err(|error| format!("Failed to write temp file for {}: {error}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Failed to replace {}: {}", path.display(), error.error))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn emit_grok_sync<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("wsl-sync-request-grok", ());
}

#[cfg(not(target_os = "windows"))]
fn emit_grok_sync<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_model_fields_without_losing_false_or_extra_config() {
        let settings = json!({
            "defaultModelKey": "grok-test",
            "auth": { "API_KEY": "secret" },
            "modelCatalog": { "models": [{
                "key": "grok-test",
                "model": "grok-test-upstream",
                "baseUrl": "https://api.example.com/v1",
                "apiBackend": "responses",
                "envKey": "XAI_API_KEY",
                "contextWindow": 131072,
                "maxCompletionTokens": 16384,
                "temperature": 0.25,
                "topP": 0.9,
                "supportsBackendSearch": false,
                "supportsReasoningEffort": true,
                "reasoningEffort": "high",
                "streamToolCalls": false,
                "maxRetries": 4,
                "inferenceIdleTimeoutSecs": 120,
                "extraHeaders": { "X-Test": "yes" },
                "extraConfig": { "unknown_flag": false, "tool_timeouts": { "search": 30 } }
            }]}
        });
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &settings, "custom").expect("project models");
        let text = document.to_string();
        assert!(text.contains("default = \"grok-test\""));
        assert!(text.contains("env_key = \"XAI_API_KEY\""));
        assert!(text.contains("context_window = 131072"));
        assert!(text.contains("max_completion_tokens = 16384"));
        assert!(text.contains("temperature = 0.25"));
        assert!(text.contains("top_p = 0.9"));
        assert!(text.contains("supports_backend_search = false"));
        assert!(text.contains("supports_reasoning_effort = true"));
        assert!(text.contains("reasoning_effort = \"high\""));
        assert!(text.contains("stream_tool_calls = false"));
        assert!(text.contains("max_retries = 4"));
        assert!(text.contains("inference_idle_timeout_secs = 120"));
        assert!(text.contains("extra_headers"));
        assert!(text.contains("unknown_flag = false"));
        assert!(text.contains("tool_timeouts"));
    }

    #[test]
    fn common_merge_preserves_provider_default_and_protected_sections() {
        let mut document: DocumentMut = r#"
[models]
default = "provider-model"
[mcp_servers.keep]
command = "npx"
[plugins]
enabled = true
"#
        .parse()
        .expect("parse live");
        merge_common_config(
            &mut document,
            r#"
[models]
default = "must-not-win"
fallback = "grok-build"
[ui]
theme = "dark"
"#,
        )
        .expect("merge common");
        assert_eq!(
            document["models"]["default"].as_str(),
            Some("provider-model")
        );
        assert_eq!(document["models"]["fallback"].as_str(), Some("grok-build"));
        assert!(document.get("mcp_servers").is_some());
        assert!(document.get("plugins").is_some());
    }

    #[test]
    fn provider_extra_config_merges_without_overwriting_managed_models() {
        let mut document: DocumentMut = r#"
[models]
default = "managed-model"

[model.managed-model]
model = "upstream-model"

[mcp_servers.keep]
command = "npx"
"#
        .parse()
        .expect("parse live");
        let settings = json!({
            "config": "[ui]\nsimple_mode = true\n\n[features]\ntelemetry = false"
        });

        merge_provider_config(&mut document, &settings).expect("merge provider config");

        assert_eq!(
            document["models"]["default"].as_str(),
            Some("managed-model")
        );
        assert_eq!(
            document["model"]["managed-model"]["model"].as_str(),
            Some("upstream-model")
        );
        assert_eq!(document["ui"]["simple_mode"].as_bool(), Some(true));
        assert_eq!(document["features"]["telemetry"].as_bool(), Some(false));
        assert!(document.get("mcp_servers").is_some());
    }

    #[test]
    fn common_update_aggressively_removes_previously_managed_fields() {
        let previous_common = r#"
[features]
telemetry = false
codebase_indexing = false

[telemetry]
trace_upload = false

[harness]
disable_codebase_upload = true
"#;
        let mut document: DocumentMut = r#"
[features]
telemetry = false
codebase_indexing = false
user_feature = true

[telemetry]
trace_upload = true
user_trace = "keep"

[harness]
disable_codebase_upload = true
user_upload = false
"#
        .parse()
        .expect("parse live config");

        remove_matching_unmanaged_config(&mut document, previous_common)
            .expect("remove previous common fields");

        assert!(document["features"].get("telemetry").is_none());
        assert!(document["features"].get("codebase_indexing").is_none());
        assert_eq!(document["features"]["user_feature"].as_bool(), Some(true));
        // Aggressive strategy: previously managed leaf is removed even if live value changed.
        assert!(document["telemetry"].get("trace_upload").is_none());
        assert_eq!(document["telemetry"]["user_trace"].as_str(), Some("keep"));
        assert!(document["harness"].get("disable_codebase_upload").is_none());
        assert_eq!(document["harness"]["user_upload"].as_bool(), Some(false));
    }

    #[test]
    fn provider_update_aggressively_removes_cleared_previous_advanced_config() {
        let previous_settings = json!({
            "config": "[ui]\nsimple_mode = true\nkeep = false"
        });
        let mut document: DocumentMut = r#"
[ui]
simple_mode = true
keep = true
runtime_owned = "preserve"
"#
        .parse()
        .expect("parse live config");

        remove_previous_provider_config(&mut document, &previous_settings.to_string())
            .expect("remove previous provider config");

        assert!(document["ui"].get("simple_mode").is_none());
        // Aggressive strategy: previously managed `keep` is removed even after user edits.
        assert!(document["ui"].get("keep").is_none());
        assert_eq!(document["ui"]["runtime_owned"].as_str(), Some("preserve"));
    }

    #[test]
    fn applied_provider_update_replaces_previous_models_and_advanced_config() {
        let previous_settings = json!({
            "config": "[ui]\nsimple_mode = true",
            "defaultModelKey": "old-model",
            "modelCatalog": { "models": [{
                "key": "old-model",
                "model": "old-upstream",
                "baseUrl": "https://old.example.com/v1",
                "apiBackend": "responses"
            }]}
        });
        let next_settings = json!({
            "config": "",
            "defaultModelKey": "new-model",
            "modelCatalog": { "models": [{
                "key": "new-model",
                "model": "new-upstream",
                "baseUrl": "https://new.example.com/v1",
                "apiBackend": "chat_completions"
            }]}
        });
        let mut document = DocumentMut::new();
        merge_provider_config(&mut document, &previous_settings).expect("merge previous config");
        project_provider_models(&mut document, &previous_settings, "custom")
            .expect("project previous models");
        document["ui"]["runtime_owned"] = value("preserve");

        remove_provider_model_tables(&mut document, &previous_settings.to_string(), "custom")
            .expect("remove previous models");
        remove_previous_provider_config(&mut document, &previous_settings.to_string())
            .expect("remove previous config");
        merge_provider_config(&mut document, &next_settings).expect("merge next config");
        project_provider_models(&mut document, &next_settings, "custom")
            .expect("project next models");

        assert!(document
            .get("model")
            .and_then(Item::as_table)
            .is_some_and(|models| !models.contains_key("old-model")));
        assert_eq!(
            document["model"]["new-model"]["model"].as_str(),
            Some("new-upstream")
        );
        assert_eq!(
            document["model"]["new-model"]["base_url"].as_str(),
            Some("https://new.example.com/v1")
        );
        assert_eq!(document["models"]["default"].as_str(), Some("new-model"));
        assert!(document["ui"].get("simple_mode").is_none());
        assert_eq!(document["ui"]["runtime_owned"].as_str(), Some("preserve"));
    }

    #[test]
    fn parse_grok_models_output_reads_available_list() {
        let output = r#"
Model 'custom' is using its own API key.

Default model: custom

Available models:
  - grok-4.5
  * custom (default)
"#;
        let models = parse_grok_models_output(output);
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["grok-4.5", "custom"]
        );
    }

    #[test]
    fn provider_config_rejects_managed_model_sections() {
        let settings = json!({
            "config": "[models]\ndefault = \"must-not-be-managed-here\""
        });
        let error = validate_provider_settings(&settings.to_string(), "custom")
            .expect_err("managed default model must be rejected");
        assert!(error.contains("[models].default"));
    }

    #[test]
    fn official_provider_allows_default_model_key_without_catalog() {
        let settings = json!({
            "auth": {},
            "config": "",
            "defaultModelKey": "grok-4.5"
        });
        validate_provider_settings(&settings.to_string(), "official")
            .expect("official providers only need defaultModelKey");
    }

    #[test]
    fn custom_provider_requires_catalog_when_default_model_key_is_set() {
        let settings = json!({
            "auth": { "API_KEY": "secret" },
            "config": "",
            "defaultModelKey": "custom"
        });
        let error = validate_provider_settings(&settings.to_string(), "custom")
            .expect_err("custom providers need modelCatalog with defaultModelKey");
        assert!(error.contains("modelCatalog.models"));
    }

    #[test]
    fn previous_provider_cleanup_force_removes_projected_models_even_if_edited() {
        let settings = json!({
            "auth": { "API_KEY": "secret" },
            "defaultModelKey": "managed",
            "modelCatalog": { "models": [{
                "key": "managed",
                "model": "upstream-model",
                "baseUrl": "https://api.example.com/v1",
                "apiBackend": "responses"
            }]}
        });
        let mut unchanged = DocumentMut::new();
        project_provider_models(&mut unchanged, &settings, "custom").expect("project provider");
        remove_provider_model_tables(&mut unchanged, &settings.to_string(), "custom")
            .expect("remove unchanged projection");
        assert!(unchanged.get("model").is_none());

        let mut edited = DocumentMut::new();
        project_provider_models(&mut edited, &settings, "custom").expect("project provider");
        edited["model"]["managed"]["name"] = value("User override");
        // Even "edited" catalog tables are channel config and must be removed on apply.
        remove_provider_model_tables(&mut edited, &settings.to_string(), "custom")
            .expect("remove edited projection");
        assert!(edited.get("model").is_none());
    }

    #[test]
    fn next_provider_overwrites_same_key_from_previous_channel() {
        // [model.custom]/[model.managed] is provider-owned channel config.
        // Switching providers with the same key must rewrite fields, not keep old ones.
        let previous_settings = json!({
            "defaultModelKey": "managed",
            "modelCatalog": { "models": [{
                "key": "managed",
                "model": "previous-upstream",
                "baseUrl": "https://previous.example.com/v1",
                "apiBackend": "responses"
            }]}
        });
        let next_settings = json!({
            "defaultModelKey": "managed",
            "modelCatalog": { "models": [{
                "key": "managed",
                "model": "next-upstream",
                "baseUrl": "https://next.example.com/v1",
                "apiBackend": "chat_completions"
            }]}
        });
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &previous_settings, "custom")
            .expect("project previous provider");
        document["model"]["managed"]["name"] = value("User override");
        // Unrelated local model key must survive cleanup of previous catalog keys.
        document["model"]["user-local"]["model"] = value("keep-me");

        remove_provider_model_tables(&mut document, &previous_settings.to_string(), "custom")
            .expect("remove previous catalog keys");
        project_provider_models(&mut document, &next_settings, "custom")
            .expect("project next provider");

        assert_eq!(
            document["model"]["managed"]["model"].as_str(),
            Some("next-upstream")
        );
        assert_eq!(
            document["model"]["managed"]["base_url"].as_str(),
            Some("https://next.example.com/v1")
        );
        assert_eq!(
            document["model"]["managed"]["api_backend"].as_str(),
            Some("chat_completions")
        );
        assert!(document["model"]["managed"].get("name").is_none());
        assert_eq!(
            document["model"]["user-local"]["model"].as_str(),
            Some("keep-me")
        );
        assert_eq!(document["models"]["default"].as_str(), Some("managed"));
    }

    #[test]
    fn local_provider_model_key_change_removes_old_projection_before_writing_new_one() {
        let previous_settings = json!({
            "defaultModelKey": "old-model",
            "modelCatalog": { "models": [{
                "key": "old-model",
                "model": "old-upstream",
                "baseUrl": "https://old.example.com/v1",
                "apiBackend": "responses"
            }]}
        });
        let next_settings = json!({
            "defaultModelKey": "new-model",
            "modelCatalog": { "models": [{
                "key": "new-model",
                "model": "new-upstream",
                "baseUrl": "https://new.example.com/v1",
                "apiBackend": "chat_completions"
            }]}
        });
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &previous_settings, "custom")
            .expect("project previous provider");

        remove_provider_model_tables(&mut document, &previous_settings.to_string(), "custom")
            .expect("remove previous provider");
        project_provider_models(&mut document, &next_settings, "custom")
            .expect("project next provider");

        assert!(document
            .get("model")
            .and_then(Item::as_table)
            .is_some_and(|models| !models.contains_key("old-model")));
        assert_eq!(
            document["model"]["new-model"]["model"].as_str(),
            Some("new-upstream")
        );
        assert_eq!(document["models"]["default"].as_str(), Some("new-model"));
    }

    #[test]
    fn removing_official_provider_models_does_not_require_catalog() {
        // Official settings only store defaultModelKey; cleanup must not treat them as custom.
        let official_settings = json!({
            "auth": {},
            "config": "",
            "defaultModelKey": "grok-4.5"
        });
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &official_settings, "official")
            .expect("project official");
        document["model"]["user-kept"]["model"] = value("keep-me");

        remove_provider_model_tables(&mut document, &official_settings.to_string(), "official")
            .expect("official cleanup must not require modelCatalog");
        // Official never owns [model.*], so user-local tables stay untouched.
        assert_eq!(
            document["model"]["user-kept"]["model"].as_str(),
            Some("keep-me")
        );
        assert_eq!(document["models"]["default"].as_str(), Some("grok-4.5"));
    }

    #[test]
    fn applying_official_removes_previous_custom_models_even_if_edited() {
        // Custom channel left [model.custom] with user-visible fields. Switching to official
        // must drop those tables so base_url/api_key do not stick around under OAuth mode.
        let previous_settings = json!({
            "auth": { "API_KEY": "secret" },
            "defaultModelKey": "custom",
            "modelCatalog": { "models": [{
                "key": "custom",
                "model": "grok-4.5",
                "baseUrl": "http://192.0.2.10/v1",
                "apiBackend": "responses"
            }]}
        });
        let official_settings = json!({
            "auth": {},
            "config": "",
            "defaultModelKey": "grok-4.5"
        });
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &previous_settings, "custom")
            .expect("project custom");
        document["model"]["custom"]["reasoning_efforts"] =
            toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::from_iter([
                "low", "medium", "high",
            ])));

        remove_provider_model_tables(&mut document, &previous_settings.to_string(), "custom")
            .expect("remove previous custom models");
        project_provider_models(&mut document, &official_settings, "official")
            .expect("project official");

        assert!(document.get("model").is_none());
        assert_eq!(document["models"]["default"].as_str(), Some("grok-4.5"));
    }

    #[test]
    fn local_provider_snapshot_lifts_shared_model_api_key_into_auth() {
        let snapshot = parse_local_grok_provider_snapshot(
            r#"
[models]
default = "private-grok"
fallback = "grok-build"

[model.private-grok]
model = "grok-4"
name = "Private Grok"
base_url = "https://api.example.com/v1"
api_backend = "responses"
api_key = "secret"
supports_backend_search = false
unknown_flag = true

[features]
telemetry = false

[mcp_servers.keep]
command = "npx"
"#,
        )
        .expect("parse local snapshot");

        assert_eq!(snapshot.category, "custom");
        let settings: Value =
            serde_json::from_str(&snapshot.settings_config).expect("parse provider settings");
        assert_eq!(settings["defaultModelKey"], "private-grok");
        assert_eq!(settings["auth"]["API_KEY"], "secret");
        assert_eq!(
            settings["modelCatalog"]["models"][0]["baseUrl"],
            "https://api.example.com/v1"
        );
        assert_eq!(
            settings["modelCatalog"]["models"][0]["supportsBackendSearch"],
            false
        );
        assert_eq!(
            settings["modelCatalog"]["models"][0]["extraConfig"]["unknown_flag"],
            true
        );
        assert!(settings["modelCatalog"]["models"][0]
            .get("apiKey")
            .is_none());
        assert!(!snapshot.settings_config.contains("access_token"));

        let common: DocumentMut = snapshot.common_config.parse().expect("parse common");
        assert_eq!(common["models"]["fallback"].as_str(), Some("grok-build"));
        assert!(common
            .get("models")
            .and_then(Item::as_table)
            .is_some_and(|models| !models.contains_key("default")));
        assert_eq!(common["features"]["telemetry"].as_bool(), Some(false));
        assert!(common.get("model").is_none());
        assert!(common.get("mcp_servers").is_none());
    }

    #[test]
    fn local_provider_snapshot_does_not_lift_divergent_model_api_keys() {
        let snapshot = parse_local_grok_provider_snapshot(
            r#"
[models]
default = "model-a"

[model.model-a]
model = "a"
api_key = "secret-a"

[model.model-b]
model = "b"
api_key = "secret-b"
"#,
        )
        .expect("parse local snapshot");

        let settings: Value =
            serde_json::from_str(&snapshot.settings_config).expect("parse provider settings");
        assert_eq!(settings["auth"], json!({}));
        assert!(settings["modelCatalog"]["models"][0]
            .get("apiKey")
            .is_none());
        assert!(settings["modelCatalog"]["models"][1]
            .get("apiKey")
            .is_none());
        assert!(!snapshot.settings_config.contains("secret-a"));
        assert!(!snapshot.settings_config.contains("secret-b"));
    }

    #[test]
    fn local_provider_snapshot_does_not_lift_partial_model_api_keys() {
        let snapshot = parse_local_grok_provider_snapshot(
            r#"
[models]
default = "model-a"

[model.model-a]
model = "a"
api_key = "secret"

[model.model-b]
model = "b"
"#,
        )
        .expect("parse local snapshot");

        let settings: Value =
            serde_json::from_str(&snapshot.settings_config).expect("parse provider settings");
        assert_eq!(settings["auth"], json!({}));
        assert!(!snapshot.settings_config.contains("secret"));
    }

    #[test]
    fn local_official_snapshot_keeps_default_model_without_custom_catalog() {
        let snapshot = parse_local_grok_provider_snapshot(
            r#"
[models]
default = "grok-build"

[telemetry]
trace_upload = false
"#,
        )
        .expect("parse official local snapshot");

        assert_eq!(snapshot.category, "official");
        let settings: Value =
            serde_json::from_str(&snapshot.settings_config).expect("parse provider settings");
        assert!(settings.get("defaultModelKey").is_none());
        assert!(settings.get("modelCatalog").is_none());
        assert!(snapshot.common_config.contains("trace_upload = false"));
    }
}
