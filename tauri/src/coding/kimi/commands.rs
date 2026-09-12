use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::Local;
use serde_json::{json, Value};
use tempfile::NamedTempFile;
use tokio::sync::Mutex as AsyncMutex;
use toml_edit::{value, DocumentMut, Item, Table};

use super::adapter;
use super::constants::{
    KIMI_CREDENTIALS_DIR, KIMI_DEFAULT_MODEL_MAX_CONTEXT_SIZE, KIMI_HOME_ENV_KEY,
    KIMI_LOCAL_PROVIDER_ID, KIMI_OFFICIAL_API_BASE_URL, KIMI_OFFICIAL_DEFAULT_MODEL_DISPLAY_NAME,
    KIMI_OFFICIAL_DEFAULT_MODEL_KEY, KIMI_PLUGINS_DIR,
};
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::open_code::shell_env;
use crate::coding::proxy_gateway::{cli_proxy, paths::ProxyGatewayPaths, types::GatewayCliKey};
use crate::coding::runtime_location;
use crate::coding::skills::commands::resync_all_skills_if_tool_path_changed;
use crate::db::helpers::{
    db_count, db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put,
    db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::Emitter;

/// Serializes all read-modify-write passes over the live config.toml. Each
/// pass builds the next document from its own read snapshot; two concurrent
/// applies would otherwise write back from the same stale snapshot and the
/// later write would drop the earlier one's projected `[providers.*]` /
/// `[models.*]` tables. Save/update entry points hold this lock across the
/// whole "capture previous snapshot -> DB write -> projection" window and
/// call the `_locked` bodies (`apply_kimi_provider_to_file_locked`,
/// `write_common_config_without_provider_locked`); those bodies must never
/// acquire the lock themselves (it is not reentrant). Single-shot atomic
/// writes (prompt files) have no read-modify-write window and stay outside
/// this lock.
static CONFIG_WRITE_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

pub(super) fn kimi_gateway_takeover_active<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> bool {
    let paths = ProxyGatewayPaths::new(crate::app_paths::resolved_data_dir());
    ensure_gateway_direct_for_paths(&paths).is_err()
}

/// Gate shared by every Kimi command that would rewrite `<root>/config.toml`:
/// during a gateway takeover the live file is owned by the proxy gateway and a
/// direct write would clobber (or be clobbered by) the takeover projection.
pub(super) fn ensure_gateway_direct_for_paths(paths: &ProxyGatewayPaths) -> Result<(), String> {
    if cli_proxy::provider_switch_locked_by_manifest(paths, GatewayCliKey::Kimi) {
        return Err(
            "当前 Kimi CLI 已由网关接管，请通过网关代理切换入口切换渠道，或先恢复直连".to_string(),
        );
    }
    Ok(())
}

pub(super) fn ensure_kimi_gateway_direct<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), String> {
    if kimi_gateway_takeover_active(app) {
        return Err(
            "当前 Kimi CLI 已由网关接管，请通过网关代理切换入口切换渠道，或先恢复直连".to_string(),
        );
    }
    Ok(())
}

pub fn get_kimi_default_root_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".kimi-code"))
        .map_err(|_| "Failed to get home directory".to_string())
}

pub fn get_kimi_root_dir_without_db() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var(KIMI_HOME_ENV_KEY) {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    if let Some(path) = shell_env::get_env_from_shell_config(KIMI_HOME_ENV_KEY) {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    get_kimi_default_root_dir()
}

pub async fn get_kimi_root_dir_from_db_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(runtime_location::get_kimi_runtime_location_async(db)
        .await?
        .host_path)
}

pub async fn get_kimi_config_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    runtime_location::get_kimi_config_path_async(db).await
}

pub async fn get_kimi_prompt_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    runtime_location::get_kimi_prompt_path_async(db).await
}

#[tauri::command]
pub async fn get_kimi_root_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<KimiPathInfo, String> {
    let location = runtime_location::get_kimi_runtime_location_async(state.db()).await?;
    Ok(KimiPathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

#[tauri::command]
pub async fn get_kimi_config_dir_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    Ok(get_kimi_root_dir_from_db_async(state.db())
        .await?
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub async fn get_kimi_config_file_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    Ok(get_kimi_config_path_async(state.db())
        .await?
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub async fn reveal_kimi_config_folder(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<(), String> {
    let config_dir = get_kimi_root_dir_from_db_async(state.db()).await?;
    fs::create_dir_all(&config_dir)
        .map_err(|error| format!("Failed to create Kimi config directory: {error}"))?;

    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");

    command
        .arg(&config_dir)
        .spawn()
        .map_err(|error| format!("Failed to reveal Kimi config directory: {error}"))?;
    Ok(())
}

#[tauri::command]
pub async fn read_kimi_settings(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<KimiSettings, String> {
    // Preview must show the exact live runtime files with no redaction.
    let db = state.db();
    let config_path = get_kimi_config_path_async(db).await?;
    let config = read_optional_text(&config_path).await?;
    Ok(KimiSettings { config })
}

#[tauri::command]
pub async fn list_kimi_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<KimiProvider>, String> {
    // Lazy-adopt the on-disk setup as a real row on the first listing so it
    // survives once the user adds their own providers. This stays in the
    // command body: `list_kimi_providers_for_db` also runs inside the
    // CONFIG_WRITE_LOCK window (`get_applied_provider`) and the import must
    // never run there (the lock is not reentrant).
    let db = state.db();
    let mut providers = list_kimi_providers_for_db(db)?;
    if providers.is_empty() {
        import_kimi_local_provider_from_files(db).await?;
        providers = list_kimi_providers_for_db(db)?;
    }
    if !providers.is_empty() {
        return Ok(providers);
    }

    match load_temp_kimi_provider_from_file(db).await {
        Ok(Some(provider)) => Ok(vec![provider]),
        Ok(None) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

pub fn list_kimi_providers_for_db(db: &SqliteDbState) -> Result<Vec<KimiProvider>, String> {
    let order = provider_order()?;
    db.with_conn(|conn| db_list(conn, DbTable::KimiProvider, Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .map(adapter::provider_from_db_value)
                .collect()
        })
}

#[tauri::command]
pub async fn create_kimi_provider<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    provider: KimiProviderInput,
) -> Result<KimiProvider, String> {
    validate_provider_settings(&provider.settings_config, &provider.category)?;
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let sort_index = match provider.sort_index {
        Some(value) => Some(value),
        None => Some(next_sort_index(db, DbTable::KimiProvider)?),
    };
    let content = KimiProviderContent {
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
            DbTable::KimiProvider,
            &id,
            &adapter::provider_to_db_value(&content),
        )
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(provider_from_content(id, content))
}

#[tauri::command]
pub async fn update_kimi_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: KimiProvider,
) -> Result<KimiProvider, String> {
    validate_provider_settings(&provider.settings_config, &provider.category)?;
    if provider.id == KIMI_LOCAL_PROVIDER_ID {
        return Err("Local Kimi provider must be saved before it can be updated".to_string());
    }
    let db = state.db();
    let existing = get_provider(db, &provider.id)?
        .ok_or_else(|| format!("Kimi provider '{}' not found", provider.id))?;
    // The update path must not bypass the "applied provider cannot be
    // disabled" boundary enforced by toggle_kimi_provider_disabled.
    if existing.is_applied && provider.is_disabled {
        return Err("The applied Kimi provider cannot be disabled".to_string());
    }
    if existing.is_applied {
        // Applied providers are re-projected into the live config.toml on save;
        // that rewrite must not happen while the gateway owns the file.
        ensure_kimi_gateway_direct(&app)?;
    }
    // Applied providers re-project the live config.toml; keep snapshot
    // capture, DB write and projection in one CONFIG_WRITE_LOCK window.
    let config_write_guard = if existing.is_applied {
        Some(CONFIG_WRITE_LOCK.lock().await)
    } else {
        None
    };
    let previous_settings_config = existing.settings_config.clone();
    let previous_category = existing.category.clone();
    let content = KimiProviderContent {
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
            DbTable::KimiProvider,
            &provider.id,
            &adapter::provider_to_db_value(&content),
        )
    })?;
    if content.is_applied {
        apply_kimi_provider_to_file_locked(
            db,
            &provider.id,
            Some(&previous_settings_config),
            Some(&previous_category),
            None,
        )
        .await?;
        emit_kimi_sync(&app);
    }
    drop(config_write_guard);
    let _ = app.emit("config-changed", "window");
    Ok(provider_from_content(provider.id, content))
}

#[tauri::command]
pub async fn delete_kimi_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    delete_kimi_provider_internal(state.inner(), &id).await?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub async fn delete_kimi_provider_internal(db: &SqliteDbState, id: &str) -> Result<(), String> {
    if id == KIMI_LOCAL_PROVIDER_ID {
        return Err("Local Kimi provider must be saved before it can be deleted".to_string());
    }
    let provider =
        get_provider(db, id)?.ok_or_else(|| format!("Kimi provider '{id}' not found"))?;
    // Deleting the applied provider would leave its projected [providers.*] /
    // [models.*] tables in config.toml without any applied snapshot to clean
    // them up (same rule as "the applied provider cannot be disabled").
    if provider.is_applied {
        return Err("The applied Kimi provider cannot be deleted".to_string());
    }
    let has_accounts = db.with_conn(|conn| {
        let accounts = db_list(conn, DbTable::KimiOfficialAccount, None)?;
        Ok(accounts
            .iter()
            .any(|account| account.get("provider_id").and_then(Value::as_str) == Some(id)))
    })?;
    if has_accounts {
        return Err("Delete the Kimi official accounts before deleting this provider".to_string());
    }
    db.with_conn(|conn| db_delete(conn, DbTable::KimiProvider, id).map(|_| ()))?;
    Ok(())
}

#[tauri::command]
pub async fn reorder_kimi_providers(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    for (index, id) in ids.iter().enumerate() {
        state.db().with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::KimiProvider,
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
pub async fn toggle_kimi_provider_disabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
    disabled: bool,
) -> Result<(), String> {
    let provider =
        get_provider(state.db(), &id)?.ok_or_else(|| format!("Kimi provider '{id}' not found"))?;
    if provider.is_applied && disabled {
        return Err("The applied Kimi provider cannot be disabled".to_string());
    }
    state.db().with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::KimiProvider,
            &id,
            &[("is_disabled", Value::Bool(disabled))],
        )
        .map(|_| ())
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn select_kimi_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    select_kimi_provider_internal(state.inner(), &app, &id).await
}

pub async fn select_kimi_provider_internal<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    id: &str,
) -> Result<(), String> {
    select_kimi_provider_internal_with_sync(state, app, id, false, true).await
}

pub async fn select_kimi_provider_internal_with_sync<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    id: &str,
    from_tray: bool,
    emit_events: bool,
) -> Result<(), String> {
    ensure_kimi_gateway_direct(app)?;
    let provider =
        get_provider(state, id)?.ok_or_else(|| format!("Kimi provider '{id}' not found"))?;
    if provider.is_disabled {
        return Err("Disabled Kimi provider cannot be applied".to_string());
    }
    apply_kimi_provider_to_file(state, id).await?;
    let now = Local::now().to_rfc3339();
    state.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiProvider, Some(id), &now)
    })?;
    if provider.category == "official" {
        super::official_accounts::sync_kimi_official_account_apply_status(state, id).await?;
    } else {
        super::official_accounts::clear_all_kimi_official_account_apply_status(state).await?;
    }
    if emit_events {
        let _ = app.emit("config-changed", if from_tray { "tray" } else { "window" });
        emit_kimi_sync(app);
    }
    record_last_used_best_effort(state, "kimi", id);
    Ok(())
}

/// Best-effort "recently used" marker for provider list sorting. Failures must
/// never break the provider apply flow itself.
fn record_last_used_best_effort(state: &SqliteDbState, module: &str, provider_id: &str) {
    if let Err(error) =
        crate::settings::provider_list_state::record_provider_last_used_in_sqlite_state(
            state,
            module,
            provider_id,
        )
    {
        log::warn!("Failed to record provider last-used for {module}:{provider_id}: {error}");
    }
}

pub async fn select_kimi_model_internal<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    model_key: &str,
) -> Result<(), String> {
    let normalized_model_key = model_key.trim();
    if normalized_model_key.is_empty() {
        return Err("Kimi model key is required".to_string());
    }
    // Model switching re-projects the live config.toml, so it is subject to
    // the same gate as every other entry that rewrites that file.
    ensure_kimi_gateway_direct(app)?;
    let provider =
        get_applied_provider(state)?.ok_or_else(|| "No applied Kimi provider found".to_string())?;
    if provider.is_disabled {
        return Err("Disabled Kimi provider cannot change models".to_string());
    }
    let mut settings: Value = serde_json::from_str(&provider.settings_config)
        .map_err(|error| format!("Invalid Kimi provider settings JSON: {error}"))?;
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
            "Kimi model '{normalized_model_key}' is not in the applied provider catalog"
        ));
    }
    settings["defaultModelKey"] = Value::String(normalized_model_key.to_string());
    let next_settings_config = serde_json::to_string(&settings)
        .map_err(|error| format!("Failed to serialize Kimi provider settings: {error}"))?;
    validate_provider_settings(&next_settings_config, &provider.category)?;
    let updated_at = Local::now().to_rfc3339();
    // DB patch and projection must be one CONFIG_WRITE_LOCK window, matching
    // every other entry that rewrites the live config.toml.
    let config_write_guard = CONFIG_WRITE_LOCK.lock().await;
    state.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::KimiProvider,
            &provider.id,
            &[
                ("settings_config", Value::String(next_settings_config)),
                ("updated_at", Value::String(updated_at)),
            ],
        )
        .map(|_| ())
    })?;
    apply_kimi_provider_to_file_locked(
        state,
        &provider.id,
        Some(&provider.settings_config),
        Some(provider.category.as_str()),
        None,
    )
    .await?;
    drop(config_write_guard);
    let _ = app.emit("config-changed", "tray");
    emit_kimi_sync(app);
    Ok(())
}

pub async fn apply_kimi_provider_to_file(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().await;
    let previous_common_config = get_common_config(db)?.map(|value| value.config);
    apply_kimi_provider_to_file_locked(
        db,
        provider_id,
        None,
        None,
        previous_common_config.as_deref(),
    )
    .await
}

/// Lock-free body of the projection pass. Callers that need the previous
/// snapshot capture, DB write and file rewrite to be one atomic window hold
/// `CONFIG_WRITE_LOCK` themselves and call this directly; simple callers go
/// through the locking wrapper above. Must never acquire the lock itself.
pub(super) async fn apply_kimi_provider_to_file_locked(
    db: &SqliteDbState,
    provider_id: &str,
    previous_settings_config: Option<&str>,
    previous_category: Option<&str>,
    previous_common_config: Option<&str>,
) -> Result<(), String> {
    let provider = get_provider(db, provider_id)?
        .ok_or_else(|| format!("Kimi provider '{provider_id}' not found"))?;
    let settings: Value = serde_json::from_str(&provider.settings_config)
        .map_err(|error| format!("Invalid Kimi provider settings JSON: {error}"))?;
    let config_path = get_kimi_config_path_async(db).await?;
    let current = read_optional_text(&config_path).await?.unwrap_or_default();
    let mut document = if current.trim().is_empty() {
        DocumentMut::new()
    } else {
        current
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid live Kimi config.toml: {error}"))?
    };

    // 前置快照清理: remove the previous provider's projected [providers.*] and
    // [models."<alias>"] tables before writing the next channel projection.
    if let Some(previous_settings_config) = previous_settings_config {
        let previous_category = previous_category.unwrap_or("custom");
        remove_previous_provider_tables(
            &mut document,
            previous_settings_config,
            previous_category,
        )?;
    } else if let Some(previous) = get_applied_provider(db)? {
        remove_previous_provider_tables(
            &mut document,
            &previous.settings_config,
            &previous.category,
        )?;
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
    migrate_deprecated_loop_control_fields(&mut document);
    write_text_atomic(&config_path, &document.to_string())?;
    Ok(())
}

/// When no provider is applied, saving the common config must not clobber the
/// live config.toml either: users may keep unmanaged `[providers]` / `[models]`
/// tables there. Reuse the provider projection merge semantics — drop the
/// previously managed common fields from the live document, then write the new
/// managed fields over it.
pub async fn write_common_config_without_provider(
    db: &SqliteDbState,
    previous_common_config: Option<&str>,
    common_config: &str,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().await;
    write_common_config_without_provider_locked(db, previous_common_config, common_config).await
}

/// Lock-free body; see `apply_kimi_provider_to_file_locked` for the contract.
pub(super) async fn write_common_config_without_provider_locked(
    db: &SqliteDbState,
    previous_common_config: Option<&str>,
    common_config: &str,
) -> Result<(), String> {
    let path = get_kimi_config_path_async(db).await?;
    let current = read_optional_text(&path).await?.unwrap_or_default();
    let mut document = if current.trim().is_empty() {
        DocumentMut::new()
    } else {
        current
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid live Kimi config.toml: {error}"))?
    };
    if let Some(previous_common_config) = previous_common_config {
        remove_matching_unmanaged_config(&mut document, previous_common_config)?;
    }
    merge_common_config(&mut document, common_config)?;
    migrate_deprecated_loop_control_fields(&mut document);
    write_text_atomic(&path, &document.to_string())?;
    Ok(())
}

/// Kimi CLI deprecated `loop_control.max_retries_per_step`; carrying it
/// forward verbatim makes every CLI run print a deprecation warning. Rename
/// it to `max_attempts_per_step`, or drop it when the new key already exists.
fn migrate_deprecated_loop_control_fields(document: &mut DocumentMut) {
    let Some(loop_control) = document
        .get_mut("loop_control")
        .and_then(Item::as_table_mut)
    else {
        return;
    };
    let deprecated = loop_control.remove("max_retries_per_step");
    if let Some(Item::Value(deprecated_value)) = deprecated {
        if loop_control.get("max_attempts_per_step").is_none() {
            loop_control.insert("max_attempts_per_step", Item::Value(deprecated_value));
        }
    }
}

#[tauri::command]
pub async fn get_kimi_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<KimiCommonConfig>, String> {
    get_common_config(state.db())
}

#[tauri::command]
pub async fn extract_kimi_common_config_from_current_file(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<KimiCommonConfig, String> {
    // Read only config.toml with timeout; do not touch credentials.
    let path = get_kimi_config_path_async(state.db()).await?;
    let current =
        crate::coding::file_io::read_text_file_with_timeout(path, "Kimi config.toml").await?;
    let mut document = if current.trim().is_empty() {
        DocumentMut::new()
    } else {
        current
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid Kimi config.toml: {error}"))?
    };
    document.remove("providers");
    document.remove("models");
    document.remove("default_model");
    let existing = get_common_config(state.db())?;
    Ok(KimiCommonConfig {
        config: document.to_string(),
        root_dir: existing.and_then(|value| value.root_dir),
        updated_at: Local::now().to_rfc3339(),
    })
}

#[tauri::command]
pub async fn save_kimi_common_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: KimiCommonConfigInput,
) -> Result<(), String> {
    ensure_kimi_gateway_direct(&app)?;
    validate_common_config(&input.config)?;
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(db, "kimi").await;
    // Previous-snapshot capture, DB write and live-file rewrite must be one
    // window: a concurrent save cleaning from the same stale snapshot would
    // leave the earlier save's managed fields behind (see CONFIG_WRITE_LOCK).
    let config_write_guard = CONFIG_WRITE_LOCK.lock().await;
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
    db.with_conn(|conn| db_put(conn, DbTable::KimiCommonConfig, "common", &value))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(db, "kimi").await?;
    if let Some(provider) = get_applied_provider(db)? {
        apply_kimi_provider_to_file_locked(
            db,
            &provider.id,
            None,
            None,
            previous_common_config.as_deref(),
        )
        .await?;
    } else {
        write_common_config_without_provider_locked(
            db,
            previous_common_config.as_deref(),
            &input.config,
        )
        .await?;
    }
    drop(config_write_guard);
    resync_all_skills_if_tool_path_changed(
        app.clone(),
        state.inner(),
        "kimi",
        previous_skills_path,
    )
    .await;
    let _ = app.emit("config-changed", "window");
    emit_kimi_sync(&app);
    Ok(())
}

#[tauri::command]
pub async fn save_kimi_local_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: KimiLocalConfigInput,
) -> Result<String, String> {
    // Returns the freshly created provider id so callers can re-engage the
    // gateway with the real record instead of the `__local__` projection id.
    // Adopting the local config rewrites the live config.toml; during a
    // gateway takeover that file is owned by the proxy gateway.
    ensure_kimi_gateway_direct(&app)?;
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(db, "kimi").await;
    // Live-snapshot capture, DB writes and the projection must be one window;
    // otherwise a concurrent save could clean from a stale snapshot.
    let config_write_guard = CONFIG_WRITE_LOCK.lock().await;
    let live_snapshot = load_local_kimi_provider_snapshot(db).await?;
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
    let previous_common_config = live_snapshot.common_config.clone();
    let common_config = input.common_config.unwrap_or(live_snapshot.common_config);
    // Validate before any DB write: failing late would leave an applied
    // provider row whose live config.toml was never projected.
    validate_common_config(&common_config)?;
    let now = Local::now().to_rfc3339();
    let provider_content = KimiProviderContent {
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
            .or(Some(next_sort_index(db, DbTable::KimiProvider)?)),
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
            DbTable::KimiProvider,
            &provider_id,
            &adapter::provider_to_db_value(&provider_content),
        )
    })?;

    let existing_common = get_common_config(db)?;
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
            DbTable::KimiCommonConfig,
            "common",
            &adapter::common_to_db_value(&common_config, root_dir.as_deref()),
        )
    })?;
    runtime_location::refresh_runtime_location_cache_for_module_async(db, "kimi").await?;
    apply_kimi_provider_to_file_locked(
        db,
        &provider_id,
        Some(&live_snapshot.settings_config),
        Some(live_snapshot.category.as_str()),
        Some(&previous_common_config),
    )
    .await?;
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiProvider, Some(&provider_id), &now)
    })?;
    drop(config_write_guard);
    if provider_content.category == "official" {
        super::official_accounts::sync_kimi_official_account_apply_status(db, &provider_id).await?;
    } else {
        super::official_accounts::clear_all_kimi_official_account_apply_status(db).await?;
    }
    resync_all_skills_if_tool_path_changed(
        app.clone(),
        state.inner(),
        "kimi",
        previous_skills_path,
    )
    .await;
    let _ = app.emit("config-changed", "window");
    emit_kimi_sync(&app);
    Ok(provider_id)
}

#[tauri::command]
pub async fn list_kimi_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<KimiPromptConfig>, String> {
    let order = prompt_order()?;
    let prompts = state
        .db()
        .with_conn(|conn| db_list(conn, DbTable::KimiPromptConfig, Some(&order)))
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
pub async fn create_kimi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: KimiPromptConfigInput,
) -> Result<KimiPromptConfig, String> {
    let now = Local::now().to_rfc3339();
    let content = KimiPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index(state.db(), DbTable::KimiPromptConfig)?),
        created_at: now.clone(),
        updated_at: now,
    };
    let id = db_new_id();
    state.db().with_conn(|conn| {
        db_put(
            conn,
            DbTable::KimiPromptConfig,
            &id,
            &adapter::prompt_to_db_value(&content),
        )
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(prompt_from_content(id, content))
}

#[tauri::command]
pub async fn update_kimi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: KimiPromptConfigInput,
) -> Result<KimiPromptConfig, String> {
    let id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let existing =
        get_prompt(state.db(), &id)?.ok_or_else(|| format!("Kimi prompt '{id}' not found"))?;
    let content = KimiPromptConfigContent {
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
            DbTable::KimiPromptConfig,
            &id,
            &adapter::prompt_to_db_value(&content),
        )
    })?;
    if content.is_applied {
        write_text_atomic(
            &get_kimi_prompt_path_async(state.db()).await?,
            &content.content,
        )?;
        emit_kimi_sync(&app);
    }
    let _ = app.emit("config-changed", "window");
    Ok(prompt_from_content(id, content))
}

#[tauri::command]
pub async fn delete_kimi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    // Only delete the DB prompt record. Keep the live AGENTS.md on disk.
    state
        .db()
        .with_conn(|conn| db_delete(conn, DbTable::KimiPromptConfig, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn save_kimi_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: KimiPromptConfigInput,
) -> Result<KimiPromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        get_local_prompt_config(state.db())
            .await?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };

    let created = create_kimi_prompt_config(
        state.clone(),
        app.clone(),
        KimiPromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;

    apply_kimi_prompt_config_internal(state.inner(), &app, &created.id).await?;
    Ok(get_prompt(state.db(), &created.id)?.unwrap_or(created))
}

#[tauri::command]
pub async fn apply_kimi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_kimi_prompt_config_internal(state.inner(), &app, &config_id).await
}

/// Disable semantics: clear every applied flag and empty the live `AGENTS.md`,
/// while keeping all DB records so any of them can be re-applied later.
pub async fn disable_kimi_prompt_runtime(state: &SqliteDbState) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    state.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiPromptConfig, None, &now)
    })?;
    write_text_atomic(&get_kimi_prompt_path_async(state).await?, "")?;
    Ok(())
}

/// Disable the applied Kimi prompt: clear every applied flag and empty the
/// live `AGENTS.md`, while keeping the DB record so it can be re-applied later.
#[tauri::command]
pub async fn disable_kimi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    get_prompt(db, &config_id)?.ok_or_else(|| format!("Kimi prompt '{config_id}' not found"))?;
    disable_kimi_prompt_runtime(db).await?;

    let _ = app.emit("config-changed", "window");
    emit_kimi_sync(&app);
    Ok(())
}

pub async fn apply_kimi_prompt_config_internal<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_kimi_prompt_config_internal_with_payload(state, app, config_id, "window").await
}

/// Tray-initiated prompt switches must emit the `tray` payload so the main
/// window reloads, matching `select_kimi_model_internal`.
pub(super) async fn apply_kimi_prompt_config_internal_from_tray<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_kimi_prompt_config_internal_with_payload(state, app, config_id, "tray").await
}

pub async fn apply_kimi_prompt_config_internal_without_events<R: tauri::Runtime>(
    state: &SqliteDbState,
    _app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    write_kimi_prompt_and_mark_applied(state, config_id).await
}

async fn apply_kimi_prompt_config_internal_with_payload<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    payload: &str,
) -> Result<(), String> {
    write_kimi_prompt_and_mark_applied(state, config_id).await?;
    let _ = app.emit("config-changed", payload);
    emit_kimi_sync(app);
    Ok(())
}

pub async fn write_kimi_prompt_and_mark_applied(
    state: &SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    let prompt = get_prompt(state, config_id)?
        .ok_or_else(|| format!("Kimi prompt '{config_id}' not found"))?;
    write_text_atomic(&get_kimi_prompt_path_async(state).await?, &prompt.content)?;
    let now = Local::now().to_rfc3339();
    state.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiPromptConfig, Some(config_id), &now)
    })?;
    Ok(())
}

#[tauri::command]
pub async fn reorder_kimi_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        state.db().with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::KimiPromptConfig,
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    Ok(())
}

struct LocalKimiProviderSnapshot {
    name: String,
    category: String,
    settings_config: String,
    common_config: String,
    meta: Option<Value>,
}

async fn load_local_kimi_provider_snapshot(
    db: &SqliteDbState,
) -> Result<LocalKimiProviderSnapshot, String> {
    let config_path = get_kimi_config_path_async(db).await?;
    let config_text = read_optional_text(&config_path).await?.unwrap_or_default();
    let has_credentials = has_local_kimi_credentials(db).await;
    parse_local_kimi_provider_snapshot(&config_text, has_credentials)
}

/// Official login state lives in `credentials/*.json`; config.toml alone cannot
/// distinguish an official setup because the official apply flow also projects
/// channel credentials into `[providers.<name>]`.
async fn has_local_kimi_credentials(db: &SqliteDbState) -> bool {
    let Ok(root_dir) = get_kimi_root_dir_from_db_async(db).await else {
        return false;
    };
    crate::coding::file_io::blocking_probe_with_timeout(move || dir_has_json_credentials(&root_dir))
        .await
        .unwrap_or(false)
}

fn dir_has_json_credentials(root_dir: &Path) -> bool {
    let credentials_dir = root_dir.join(KIMI_CREDENTIALS_DIR);
    let Ok(entries) = fs::read_dir(&credentials_dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    })
}

/// Credentials on disk are the strongest official signal; without them, an
/// empty `[providers]` table still means the default official channel.
fn determine_local_kimi_provider_category(
    has_credentials: bool,
    has_custom_providers: bool,
) -> &'static str {
    if has_credentials || !has_custom_providers {
        "official"
    } else {
        "custom"
    }
}

const KIMI_NO_LOCAL_PROVIDER_CONFIG_ERROR: &str = "No local Kimi provider config found";

fn parse_local_kimi_provider_snapshot(
    config_text: &str,
    has_credentials: bool,
) -> Result<LocalKimiProviderSnapshot, String> {
    if config_text.trim().is_empty() {
        return Err(KIMI_NO_LOCAL_PROVIDER_CONFIG_ERROR.to_string());
    }

    let document = config_text
        .parse::<DocumentMut>()
        .map_err(|error| format!("Invalid local Kimi config.toml: {error}"))?;
    let default_model_key = document
        .get("default_model")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let providers_table = document.get("providers").and_then(Item::as_table);
    let mut catalog_models = Vec::new();
    let mut provider_configs = serde_json::Map::new();

    if let Some(providers_table) = providers_table {
        for (provider_key, provider_item) in providers_table.iter() {
            let Some(provider_table) = provider_item.as_table() else {
                continue;
            };
            let mut provider_json = serde_json::Map::new();
            for (field, item) in provider_table.iter() {
                if let Ok(field_value) = toml_item_to_json(item) {
                    provider_json.insert(field.to_string(), field_value);
                }
            }
            provider_configs.insert(provider_key.to_string(), Value::Object(provider_json));
        }
    }

    let models_table = document.get("models").and_then(Item::as_table);
    let mut model_api_keys: Vec<Option<String>> = Vec::new();
    if let Some(models_table) = models_table {
        for (model_key, model_item) in models_table.iter() {
            if model_key == "default" {
                continue;
            }
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
                    "provider" => Some("provider"),
                    "display_name" => Some("displayName"),
                    "max_context_size" => Some("maxContextSize"),
                    "capabilities" => Some("capabilities"),
                    "support_efforts" => Some("supportEfforts"),
                    "default_effort" => Some("defaultEffort"),
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

    let category =
        determine_local_kimi_provider_category(has_credentials, !provider_configs.is_empty());
    let mut settings = json!({
        "auth": {},
        "config": "",
    });
    if !catalog_models.is_empty() {
        if let Some(default_model_key) = default_model_key.as_deref() {
            settings["defaultModelKey"] = Value::String(default_model_key.to_string());
        }
        settings["modelCatalog"] = json!({ "models": catalog_models });
    }
    if !provider_configs.is_empty() {
        settings["providerConfigs"] = Value::Object(provider_configs);
    }
    let shared_api_key = model_api_keys
        .first()
        .and_then(|first| first.as_ref())
        .filter(|_| {
            model_api_keys
                .iter()
                .all(|key| key.as_ref() == model_api_keys[0].as_ref())
        })
        .cloned();
    if let Some(api_key) = shared_api_key {
        settings["auth"] = json!({ "API_KEY": api_key });
    }
    let settings_config = serde_json::to_string(&settings)
        .map_err(|error| format!("Failed to serialize local Kimi provider: {error}"))?;
    validate_provider_settings(&settings_config, category)?;

    let mut common_document = document;
    common_document.remove("providers");
    common_document.remove("models");
    common_document.remove("default_model");

    Ok(LocalKimiProviderSnapshot {
        name: "Local Kimi".to_string(),
        category: category.to_string(),
        settings_config,
        common_config: common_document.to_string(),
        meta: None,
    })
}

async fn load_temp_kimi_provider_from_file(
    db: &SqliteDbState,
) -> Result<Option<KimiProvider>, String> {
    // No live config (file missing or empty) means there is nothing to adopt;
    // express that structurally instead of string-matching a parser error.
    let config_path = get_kimi_config_path_async(db).await?;
    let config_text = read_optional_text(&config_path).await?.unwrap_or_default();
    if config_text.trim().is_empty() {
        return Ok(None);
    }
    let snapshot = load_local_kimi_provider_snapshot(db).await?;
    let now = Local::now().to_rfc3339();
    Ok(Some(KimiProvider {
        id: KIMI_LOCAL_PROVIDER_ID.to_string(),
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
    }))
}

/// Persist the on-disk Kimi setup as a real provider row when the DB has none,
/// mirroring `import_codex_default_provider_from_local_files`
/// (codex/commands.rs). Returns the new provider id when a row was inserted.
/// DB-only: the live config.toml already IS this provider, so no re-projection,
/// gateway gate, or event emission happens here.
async fn import_kimi_local_provider_from_files(
    db: &SqliteDbState,
) -> Result<Option<String>, String> {
    if db.with_conn(|conn| db_count(conn, DbTable::KimiProvider))? > 0 {
        return Ok(None);
    }

    let snapshot = match load_local_kimi_provider_snapshot(db).await {
        Ok(snapshot) => snapshot,
        Err(error) if error == KIMI_NO_LOCAL_PROVIDER_CONFIG_ERROR => return Ok(None),
        Err(error) => return Err(error),
    };
    let now = Local::now().to_rfc3339();
    let content = KimiProviderContent {
        name: snapshot.name,
        category: snapshot.category,
        settings_config: snapshot.settings_config,
        source_provider_id: None,
        website_url: None,
        notes: Some("从配置文件自动导入".to_string()),
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: snapshot.meta,
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_id = db_new_id();
    let inserted = db.with_conn(|conn| {
        if db_count(conn, DbTable::KimiProvider)? > 0 {
            return Ok(false);
        }
        db_put(
            conn,
            DbTable::KimiProvider,
            &provider_id,
            &adapter::provider_to_db_value(&content),
        )?;
        Ok(true)
    })?;
    if !inserted {
        return Ok(None);
    }

    // Adopt the common config too so later applies take over the whole file
    // state; an existing row (root_dir semantics) is never overwritten.
    if get_common_config(db)?.is_none() {
        let common = adapter::common_to_db_value(&snapshot.common_config, None);
        db.with_conn(|conn| db_put(conn, DbTable::KimiCommonConfig, "common", &common))?;
    }
    Ok(Some(provider_id))
}

fn toml_item_to_json(item: &Item) -> Result<Value, String> {
    let mut document = DocumentMut::new();
    document.insert("holder", item.clone());
    let parsed: toml::Value = toml::from_str(&document.to_string())
        .map_err(|error| format!("Failed to parse Kimi model field: {error}"))?;
    let value = parsed
        .get("holder")
        .cloned()
        .ok_or_else(|| "Failed to read Kimi model field".to_string())?;
    serde_json::to_value(value)
        .map_err(|error| format!("Failed to convert Kimi model field: {error}"))
}

fn get_provider(db: &SqliteDbState, id: &str) -> Result<Option<KimiProvider>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::KimiProvider, id))
        .map(|value| value.map(adapter::provider_from_db_value))
}

fn get_applied_provider(db: &SqliteDbState) -> Result<Option<KimiProvider>, String> {
    Ok(list_kimi_providers_for_db(db)?
        .into_iter()
        .find(|provider| provider.is_applied))
}

fn get_common_config(db: &SqliteDbState) -> Result<Option<KimiCommonConfig>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::KimiCommonConfig, "common"))
        .map(|value| value.map(adapter::common_from_db_value))
}

fn get_prompt(db: &SqliteDbState, id: &str) -> Result<Option<KimiPromptConfig>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::KimiPromptConfig, id))
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

fn provider_from_content(id: String, content: KimiProviderContent) -> KimiProvider {
    KimiProvider {
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

fn prompt_from_content(id: String, content: KimiPromptConfigContent) -> KimiPromptConfig {
    KimiPromptConfig {
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
        .map_err(|error| format!("Invalid Kimi provider settings JSON: {error}"))?;
    if let Some(config) = settings.get("config").and_then(Value::as_str) {
        if !config.trim().is_empty() {
            let document = config
                .parse::<DocumentMut>()
                .map_err(|error| format!("Invalid Kimi provider TOML: {error}"))?;
            validate_unmanaged_kimi_config(&document, "provider")?;
        }
    }
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
        return Err("Kimi modelCatalog.models is required when defaultModelKey is set".to_string());
    }
    Ok(())
}

fn validate_common_config(config: &str) -> Result<(), String> {
    if config.trim().is_empty() {
        return Ok(());
    }
    let document = config
        .parse::<DocumentMut>()
        .map_err(|error| format!("Invalid Kimi common TOML: {error}"))?;
    validate_unmanaged_kimi_config(&document, "common")
}

fn validate_unmanaged_kimi_config(document: &DocumentMut, owner: &str) -> Result<(), String> {
    for protected in ["providers", "models", "default_model"] {
        if document.get(protected).is_some() {
            return Err(format!(
                "Kimi {owner} config cannot manage protected section [{protected}]"
            ));
        }
    }
    Ok(())
}

/// Remove the previous provider's projected `[providers.*]` and `[models."<alias>"]`
/// tables from the live document (前置快照清理).
fn remove_previous_provider_tables(
    document: &mut DocumentMut,
    settings_config: &str,
    category: &str,
) -> Result<(), String> {
    let settings: Value = serde_json::from_str(settings_config)
        .map_err(|error| format!("Invalid previous Kimi provider settings JSON: {error}"))?;
    // Provider key under [providers.<name>]. Official providers project to the
    // managed provider name; custom providers use their own key.
    let provider_keys = collect_provider_keys(&settings, category);
    for provider_key in provider_keys {
        if let Some(providers) = document.get_mut("providers").and_then(Item::as_table_mut) {
            providers.remove(&provider_key);
            if providers.is_empty() {
                document.remove("providers");
            }
        }
    }
    // Model aliases owned by this channel.
    let model_keys = settings
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
    if let Some(models) = document.get_mut("models").and_then(Item::as_table_like_mut) {
        for key in model_keys {
            models.remove(&key);
        }
        if models.is_empty() {
            document.remove("models");
        }
    }
    document.remove("default_model");
    Ok(())
}

fn collect_provider_keys(settings: &Value, category: &str) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(provider_configs) = settings.get("providerConfigs").and_then(Value::as_object) {
        keys.extend(provider_configs.keys().cloned());
    }
    if keys.is_empty() {
        // Fall back to managed default key for official/custom channels.
        let default_key = if category == "official" {
            "managed:kimi-code".to_string()
        } else {
            "custom".to_string()
        };
        keys.push(default_key);
    }
    keys
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
        .map_err(|error| format!("Invalid previous Kimi managed TOML: {error}"))?;
    for protected in ["providers", "models", "default_model"] {
        previous_document.remove(protected);
    }
    remove_matching_table_items(document.as_table_mut(), previous_document.as_table());
    Ok(())
}

fn remove_matching_table_items(target: &mut Table, previous: &Table) {
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

async fn get_local_prompt_config(db: &SqliteDbState) -> Result<Option<KimiPromptConfig>, String> {
    let prompt_path = get_kimi_prompt_path_async(db).await?;
    let Some(content) = read_optional_text(&prompt_path).await? else {
        return Ok(None);
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    let now = Local::now().to_rfc3339();
    Ok(Some(KimiPromptConfig {
        id: KIMI_LOCAL_PROVIDER_ID.to_string(),
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
        .map_err(|error| format!("Invalid stored Kimi common TOML: {error}"))?;
    let keys = common_document
        .iter()
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();
    for key in keys {
        if matches!(key.as_str(), "providers" | "models" | "default_model") {
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

/// Project the applied provider into `[providers.<name>]` and `[models."<alias>"]`,
/// then set `default_model`.
fn project_provider_models(
    document: &mut DocumentMut,
    settings: &Value,
    category: &str,
) -> Result<(), String> {
    let mut models = settings
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Resolve the default model key: explicit key → first catalog model → the
    // official fallback. Custom channels with no catalog must not project a
    // dangling `default_model` (the CLI refuses to resolve unknown keys).
    let explicit_default_model_key = settings
        .get("defaultModelKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let first_model_key = models.first().and_then(|model| {
        model
            .get("key")
            .or_else(|| model.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let default_model_key = explicit_default_model_key
        .or(first_model_key)
        .or_else(|| (category == "official").then(|| KIMI_OFFICIAL_DEFAULT_MODEL_KEY.to_string()));
    // Official channels keep no client-side catalog (credentials carry the
    // channel), but a projected `default_model` without a matching [models]
    // table is a dangling reference the CLI refuses to resolve — synthesize
    // the missing entry so the key always resolves.
    if category == "official" {
        if let Some(key) = default_model_key.as_deref() {
            let has_entry = models.iter().any(|model| {
                ["key", "model"].iter().any(|field| {
                    model.get(field).and_then(Value::as_str).map(str::trim) == Some(key)
                })
            });
            if !has_entry {
                models.push(json!({
                    "key": key,
                    "model": key.rsplit('/').next().unwrap_or(key),
                    "displayName": (key == KIMI_OFFICIAL_DEFAULT_MODEL_KEY)
                        .then_some(KIMI_OFFICIAL_DEFAULT_MODEL_DISPLAY_NAME),
                }));
            }
        }
    }
    match default_model_key {
        Some(key) => document["default_model"] = value(key),
        None => {
            document.remove("default_model");
        }
    }
    let models_root = document["models"].or_insert(Item::Table(Table::new()));
    let models_root = models_root
        .as_table_mut()
        .ok_or_else(|| "Live Kimi [models] must be a table".to_string())?;

    for model in models {
        let key = model
            .get("key")
            .or_else(|| model.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Kimi model entry requires key or model".to_string())?;
        let mut table = Table::new();
        insert_known_model_fields(&mut table, &model, category)?;
        if let Some(extra) = model.get("extraConfig").and_then(Value::as_object) {
            for (field, value) in extra {
                if !table.contains_key(field) {
                    table.insert(field, json_to_toml_item(value)?);
                }
            }
        }
        models_root.insert(key, Item::Table(table));
    }

    // Drop a fully empty [models] root (e.g. official provider with an empty
    // catalog after cleaning the previous projection); user-written model
    // tables keep the root non-empty and are preserved.
    if models_root.is_empty() {
        document.remove("models");
    }

    // Project [providers.<name>] from providerConfigs or defaults.
    let provider_keys = collect_provider_keys(settings, category);
    let api_key = settings
        .pointer("/auth/API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let providers_root = document["providers"].or_insert(Item::Table(Table::new()));
    let providers_root = providers_root
        .as_table_mut()
        .ok_or_else(|| "Live Kimi [providers] must be a table".to_string())?;
    for provider_key in provider_keys {
        let config = settings
            .get("providerConfigs")
            .and_then(|value| value.get(&provider_key));
        // Custom channels with no fields, credentials, or env would project a
        // contentless `[providers.*]` table; leave it out instead.
        if category != "official" && config.is_none() && api_key.is_none() {
            continue;
        }
        let mut table = Table::new();
        if let Some(config) = config {
            for (field, value) in config.as_object().into_iter().flatten() {
                if field == "api_key" || field == "env" {
                    continue;
                }
                table.insert(field, json_to_toml_item(value)?);
            }
        }
        // Defaults per channel.
        if category == "official" {
            table.insert("type", value("kimi"));
            table.insert("base_url", value(KIMI_OFFICIAL_API_BASE_URL));
        } else {
            table.insert("type", value("openai"));
        }
        if let Some(api_key) = api_key {
            table.insert("api_key", value(api_key));
        }
        if let Some(env) = settings
            .get("providerConfigs")
            .and_then(|value| value.get(&provider_key))
            .and_then(|value| value.get("env"))
        {
            if let Some(env_obj) = env.as_object() {
                let mut env_table = Table::new();
                for (field, value) in env_obj {
                    env_table.insert(field, json_to_toml_item(value)?);
                }
                if !env_table.is_empty() {
                    table.insert("env", Item::Table(env_table));
                }
            }
        }
        providers_root.insert(&provider_key, Item::Table(table));
    }
    let providers_empty = providers_root.is_empty();
    if providers_empty {
        document.remove("providers");
    }
    Ok(())
}

fn insert_known_model_fields(
    table: &mut Table,
    model: &Value,
    category: &str,
) -> Result<(), String> {
    const DEFAULT_MODEL_MAX_CONTEXT_SIZE: i64 = KIMI_DEFAULT_MODEL_MAX_CONTEXT_SIZE;
    let model_id = model
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(model_id) = model_id {
        table.insert("model", value(model_id));
    }
    let provider = model
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if category == "official" {
                "managed:kimi-code".to_string()
            } else {
                "custom".to_string()
            }
        });
    table.insert("provider", value(provider));
    if let Some(display_name) = model
        .get("displayName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        table.insert("display_name", value(display_name));
    }
    // Kimi CLI refuses to start a session unless every projected model defines
    // a positive `max_context_size`; fall back to 256k instead of omitting the
    // field (256k matches the conservative official kimi-for-coding value).
    let max_context_size = model
        .get("maxContextSize")
        .and_then(Value::as_i64)
        .filter(|size| *size > 0)
        .unwrap_or(DEFAULT_MODEL_MAX_CONTEXT_SIZE);
    table.insert("max_context_size", value(max_context_size));
    if let Some(capabilities) = model.get("capabilities").and_then(Value::as_array) {
        let mut arr = toml_edit::Array::new();
        for item in capabilities.iter().filter_map(Value::as_str) {
            arr.push(item);
        }
        table.insert(
            "capabilities",
            toml_edit::Item::Value(toml_edit::Value::Array(arr)),
        );
    }
    if let Some(support_efforts) = model.get("supportEfforts").and_then(Value::as_array) {
        let mut arr = toml_edit::Array::new();
        for item in support_efforts.iter().filter_map(Value::as_str) {
            arr.push(item);
        }
        table.insert(
            "support_efforts",
            toml_edit::Item::Value(toml_edit::Value::Array(arr)),
        );
    }
    if let Some(default_effort) = model
        .get("defaultEffort")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        table.insert("default_effort", value(default_effort));
    }
    Ok(())
}

fn json_to_toml_item(value: &Value) -> Result<Item, String> {
    // Build a real toml::Value so key escaping and nested tables are handled
    // by the serializer instead of hand-rolled string formatting.
    let toml_value = json_to_toml_value(value)?;
    let mut holder = toml::map::Map::new();
    holder.insert("holder".to_string(), toml_value);
    let text = toml::to_string(&holder)
        .map_err(|error| format!("Failed to serialize Kimi config value: {error}"))?;
    let document = text
        .parse::<DocumentMut>()
        .map_err(|error| format!("Failed to parse Kimi config value: {error}"))?;
    document
        .get("holder")
        .cloned()
        .ok_or_else(|| "Failed to build Kimi config value".to_string())
}

fn json_to_toml_value(value: &Value) -> Result<toml::Value, String> {
    match value {
        Value::Null => Ok(toml::Value::String(String::new())),
        Value::Bool(boolean) => Ok(toml::Value::Boolean(*boolean)),
        Value::Number(number) => number
            .as_i64()
            .map(toml::Value::Integer)
            .or_else(|| number.as_f64().map(toml::Value::Float))
            .ok_or_else(|| format!("Unsupported Kimi config number: {number}")),
        Value::String(text) => Ok(toml::Value::String(text.clone())),
        Value::Array(items) => Ok(toml::Value::Array(
            items
                .iter()
                .map(json_to_toml_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (key, item) in map {
                table.insert(key.clone(), json_to_toml_value(item)?);
            }
            Ok(toml::Value::Table(table))
        }
    }
}

/// WSL UNC / network roots can block `fs::read_to_string` for a long time;
/// route every live runtime file read through the shared timeout helper.
async fn read_optional_text(path: &Path) -> Result<Option<String>, String> {
    crate::coding::file_io::read_optional_text_file_with_timeout(
        path.to_path_buf(),
        "Kimi runtime file",
    )
    .await
}

fn write_text_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Failed to determine parent of {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Failed to create temp file in {}: {error}",
            parent.display()
        )
    })?;
    temp.write_all(content.as_bytes())
        .map_err(|error| format!("Failed to write temp file: {error}"))?;
    temp.flush()
        .map_err(|error| format!("Failed to flush temp file: {error}"))?;
    temp.persist(path)
        .map_err(|error| format!("Failed to persist {}: {error}", path.display()))?;
    Ok(())
}

pub(super) fn emit_kimi_sync<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    // Mirror the other root-dir CLIs: one per-tool event consumed by the
    // lib.rs WSL auto-sync listener. SSH sync stays manual and has no
    // event-driven auto-sync listener.
    let _ = app.emit("wsl-sync-request-kimi", ());
}

pub fn list_kimi_plugins_from_dir(plugins_dir: &Path) -> Vec<KimiPlugin> {
    let installed_json_path = plugins_dir.join("installed.json");
    if installed_json_path.is_file() {
        if let Ok(content) = fs::read_to_string(&installed_json_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&content) {
                if let Some(array) = value.as_array() {
                    return array
                        .iter()
                        .filter_map(|item| {
                            let name = item
                                .get("name")
                                .and_then(Value::as_str)
                                .or_else(|| item.get("id").and_then(Value::as_str))?
                                .to_string();
                            let version = item
                                .get("version")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            let description = item
                                .get("description")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            let enabled =
                                item.get("enabled").and_then(Value::as_bool).or_else(|| {
                                    item.get("status")
                                        .and_then(Value::as_str)
                                        .map(|s| s == "enabled")
                                });
                            Some(KimiPlugin {
                                name,
                                version,
                                description,
                                enabled,
                            })
                        })
                        .collect();
                } else if let Some(map) = value.as_object() {
                    let mut plugins = Vec::new();
                    for (key, item) in map {
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(key)
                            .to_string();
                        let version = item
                            .get("version")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let description = item
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let enabled = item.get("enabled").and_then(Value::as_bool).or_else(|| {
                            item.get("status")
                                .and_then(Value::as_str)
                                .map(|s| s == "enabled")
                        });
                        plugins.push(KimiPlugin {
                            name,
                            version,
                            description,
                            enabled,
                        });
                    }
                    return plugins;
                }
            }
        }
    }

    // Fallback: scan managed/ or plugins directory entries
    let scan_dir = if plugins_dir.join("managed").is_dir() {
        plugins_dir.join("managed")
    } else {
        plugins_dir.to_path_buf()
    };

    let Ok(entries) = fs::read_dir(&scan_dir) else {
        return Vec::new();
    };

    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "managed" || name == ".git" {
                continue;
            }
            // Check for plugin.json / package.json / manifest.json if exists
            let mut version = None;
            let mut description = None;
            for manifest_name in &["plugin.json", "package.json", "manifest.json"] {
                let manifest_path = path.join(manifest_name);
                if manifest_path.is_file() {
                    if let Ok(manifest_content) = fs::read_to_string(&manifest_path) {
                        if let Ok(manifest_json) = serde_json::from_str::<Value>(&manifest_content)
                        {
                            if version.is_none() {
                                version = manifest_json
                                    .get("version")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                            }
                            if description.is_none() {
                                description = manifest_json
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                            }
                        }
                    }
                }
            }
            plugins.push(KimiPlugin {
                name,
                version,
                description,
                enabled: Some(true),
            });
        }
    }
    plugins
}

#[tauri::command]
pub async fn list_kimi_plugins(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<KimiPlugin>, String> {
    let root_dir = get_kimi_root_dir_from_db_async(state.db()).await?;
    let plugins_dir = root_dir.join(KIMI_PLUGINS_DIR);
    tauri::async_runtime::spawn_blocking(move || list_kimi_plugins_from_dir(&plugins_dir))
        .await
        .map_err(|error| format!("Failed to list Kimi plugins: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(settings_json: &str) -> Value {
        serde_json::from_str(settings_json).expect("valid settings json")
    }

    fn render(document: &DocumentMut) -> String {
        document.to_string()
    }

    #[test]
    fn migrate_renames_deprecated_loop_control_max_retries() {
        // Rename when the new key is absent.
        let mut document = r#"
[loop_control]
max_retries_per_step = 3
[thinking]
enabled = true
"#
        .parse::<DocumentMut>()
        .expect("valid toml");
        migrate_deprecated_loop_control_fields(&mut document);
        let text = render(&document);
        assert!(!text.contains("max_retries_per_step"));
        assert!(text.contains("max_attempts_per_step = 3"));
        assert!(text.contains("[thinking]"));

        // Drop the deprecated key when the new key already exists.
        let mut document = r#"
[loop_control]
max_attempts_per_step = 10
max_retries_per_step = 3
reserved_context_size = 50000
"#
        .parse::<DocumentMut>()
        .expect("valid toml");
        migrate_deprecated_loop_control_fields(&mut document);
        let text = render(&document);
        assert!(!text.contains("max_retries_per_step"));
        assert!(text.contains("max_attempts_per_step = 10"));
        assert!(text.contains("reserved_context_size = 50000"));

        // No loop_control table: no-op.
        let mut document = r#"
[thinking]
enabled = true
"#
        .parse::<DocumentMut>()
        .expect("valid toml");
        migrate_deprecated_loop_control_fields(&mut document);
        assert!(render(&document).contains("[thinking]"));
    }

    #[test]
    fn project_defaults_positive_max_context_size_for_cli_hard_requirement() {
        // Kimi CLI refuses to start a session when a projected model lacks a
        // positive max_context_size; the projection must always emit one.
        let settings = parse(
            r#"{
                "defaultModelKey": "custom-model",
                "auth": { "API_KEY": "sk-test" },
                "providerConfigs": { "custom": { "type": "openai", "base_url": "https://relay.example.com/v1" } },
                "modelCatalog": { "models": [
                    { "key": "custom-model", "model": "custom-model", "provider": "custom", "displayName": "Model" }
                ]}
            }"#,
        );
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &settings, "custom").expect("project ok");
        let text = render(&document);
        assert!(text.contains("max_context_size = 262144"));

        // Explicit values (including official K3's 1M) are preserved.
        let explicit = parse(
            r#"{
                "modelCatalog": { "models": [
                    { "key": "k3", "model": "k3", "maxContextSize": 1048576 }
                ]}
            }"#,
        );
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &explicit, "custom").expect("project ok");
        assert!(render(&document).contains("max_context_size = 1048576"));

        // Non-positive overrides are replaced by the default too.
        let invalid = parse(
            r#"{
                "modelCatalog": { "models": [
                    { "key": "k3", "model": "k3", "maxContextSize": 0 }
                ]}
            }"#,
        );
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &invalid, "custom").expect("project ok");
        assert!(render(&document).contains("max_context_size = 262144"));
    }

    #[test]
    fn project_writes_providers_models_and_default_model() {
        let settings = parse(
            r#"{
                "defaultModelKey": "kimi-code/k3",
                "auth": { "API_KEY": "sk-test" },
                "providerConfigs": { "managed:kimi-code": { "type": "kimi", "base_url": "https://api.kimi.com/coding/v1" } },
                "modelCatalog": { "models": [
                    { "key": "kimi-code/k3", "model": "k3", "provider": "managed:kimi-code", "displayName": "K3" }
                ]}
            }"#,
        );
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &settings, "custom").expect("project ok");
        let text = render(&document);
        assert!(text.contains("default_model = \"kimi-code/k3\""));
        assert!(text.contains("[providers.\"managed:kimi-code\"]"));
        assert!(text.contains("[models.\"kimi-code/k3\"]"));
        assert!(text.contains("model = \"k3\""));
        assert!(text.contains("provider = \"managed:kimi-code\""));
        assert!(text.contains("api_key = \"sk-test\""));
    }

    #[test]
    fn remove_previous_provider_tables_cleans_old_projection() {
        let mut document = r#"
default_model = "old/alias"
[providers."old-provider"]
type = "openai"
[models."old/alias"]
model = "old"
provider = "old-provider"
[thinking]
enabled = true
"#
        .parse::<DocumentMut>()
        .expect("valid toml");
        let old_settings = parse(
            r#"{
                "defaultModelKey": "old/alias",
                "providerConfigs": { "old-provider": { "type": "openai" } },
                "modelCatalog": { "models": [ { "key": "old/alias", "model": "old" } ]}
            }"#,
        );
        remove_previous_provider_tables(&mut document, &old_settings.to_string(), "custom")
            .expect("clean ok");
        let text = render(&document);
        assert!(!text.contains("old-provider"));
        assert!(!text.contains("old/alias"));
        // Unmanaged section preserved.
        assert!(text.contains("[thinking]"));
    }

    #[test]
    fn preserves_unmanaged_fields_across_projection() {
        let mut document = r#"
[thinking]
enabled = true
effort = "high"
[[permission.rules]]
decision = "allow"
pattern = "Read"
"#
        .parse::<DocumentMut>()
        .expect("valid toml");
        let settings = parse(
            r#"{
                "defaultModelKey": "kimi-code/k3",
                "modelCatalog": { "models": [ { "key": "kimi-code/k3", "model": "k3" } ]}
            }"#,
        );
        project_provider_models(&mut document, &settings, "official").expect("project ok");
        let text = render(&document);
        assert!(text.contains("enabled = true"));
        assert!(text.contains("effort = \"high\""));
        assert!(text.contains("pattern = \"Read\""));
    }

    #[test]
    fn determine_local_category_credentials_outweigh_providers_table() {
        use super::determine_local_kimi_provider_category as determine;

        // Official login present: always official, even when [providers] exists
        // (the official apply flow projects channel credentials there too).
        assert_eq!(determine(true, true), "official");
        assert_eq!(determine(true, false), "official");
        // No credentials: empty [providers] falls back to the default official
        // channel; any custom provider means custom.
        assert_eq!(determine(false, false), "official");
        assert_eq!(determine(false, true), "custom");
    }

    #[test]
    fn parse_local_snapshot_official_login_with_providers_table_is_official() {
        // Regression: a user logged in via the official CLI (credentials/ on
        // disk) with a [providers] table projected by the official apply flow
        // must not be misread as a custom setup.
        let config = r#"
default_model = "kimi-code/k3"
[providers."kimi-official"]
type = "kimi"
[models."kimi-code/k3"]
model = "k3"
provider = "kimi-official"
"#;
        let snapshot =
            super::parse_local_kimi_provider_snapshot(config, true).expect("snapshot ok");
        assert_eq!(snapshot.category, "official");
    }

    #[test]
    fn parse_local_snapshot_custom_providers_without_credentials_is_custom() {
        let config = r#"
default_model = "custom/model"
[providers."my-relay"]
type = "openai"
base_url = "https://relay.example.com/v1"
[models."custom/model"]
model = "model"
provider = "my-relay"
api_key = "sk-test"
"#;
        let snapshot =
            super::parse_local_kimi_provider_snapshot(config, false).expect("snapshot ok");
        assert_eq!(snapshot.category, "custom");
    }

    #[test]
    fn parse_local_snapshot_empty_providers_without_credentials_is_official() {
        let config = "default_model = \"kimi-code/k3\"\n";
        let snapshot =
            super::parse_local_kimi_provider_snapshot(config, false).expect("snapshot ok");
        assert_eq!(snapshot.category, "official");
    }

    #[test]
    fn dir_has_json_credentials_detects_token_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(!super::dir_has_json_credentials(temp.path()));

        let credentials_dir = temp.path().join(super::KIMI_CREDENTIALS_DIR);
        fs::create_dir_all(&credentials_dir).expect("create dir");
        assert!(!super::dir_has_json_credentials(temp.path()));

        fs::write(credentials_dir.join("token.json"), "{}").expect("write token");
        assert!(super::dir_has_json_credentials(temp.path()));
    }

    #[test]
    fn gateway_direct_gate_blocks_on_enabled_kimi_manifest() {
        use crate::coding::proxy_gateway::cli_proxy::manifest::CliProxyManifest;
        use crate::coding::proxy_gateway::types::GatewayProxyMode;

        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProxyGatewayPaths::new(temp.path());
        assert!(super::ensure_gateway_direct_for_paths(&paths).is_ok());

        let manifest = CliProxyManifest::new(
            GatewayCliKey::Kimi,
            "http://127.0.0.1:37123".to_string(),
            "2026-08-30T00:00:00Z".to_string(),
            GatewayProxyMode::Single,
            "provider-1".to_string(),
        );
        let manifest_path = paths.manifest_path(GatewayCliKey::Kimi);
        fs::create_dir_all(manifest_path.parent().expect("manifest parent")).expect("mkdir");
        fs::write(
            &manifest_path,
            serde_json::to_string(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let error = super::ensure_gateway_direct_for_paths(&paths)
            .expect_err("takeover must block direct writes");
        assert!(error.contains("网关接管"), "unexpected error: {error}");
    }

    #[test]
    fn json_to_toml_item_handles_nested_objects_and_special_keys() {
        let value = json!({
            "weird key.1": "safe",
            "nested": { "inner": 1, "deeper": { "leaf": true } },
            "list": ["a", "b"],
            "n": 1.5,
            "nothing": null
        });
        let item = json_to_toml_item(&value).expect("convert ok");
        let mut document = DocumentMut::new();
        document.insert("field", item);
        let text = render(&document);
        assert!(text.contains("\"weird key.1\" = \"safe\""));
        assert!(text.contains("inner = 1"));
        assert!(text.contains("leaf = true"));
        assert!(text.contains("list = [\"a\", \"b\"]"));
        // Round-trip must parse back into the same structure.
        let parsed: toml::Value = toml::from_str(&text).expect("valid toml round trip");
        assert_eq!(
            parsed["field"]["nested"]["deeper"]["leaf"],
            toml::Value::Boolean(true)
        );
        assert_eq!(parsed["field"]["n"], toml::Value::Float(1.5));
        assert_eq!(
            parsed["field"]["nothing"],
            toml::Value::String(String::new())
        );
    }

    #[test]
    fn project_skips_dangling_default_model_for_empty_custom_provider() {
        // Custom channel with no catalog and no key must not project a
        // dangling default_model referencing the official alias.
        let settings = parse("{}");
        let mut document = r#"
default_model = "previous/alias"
[models."previous/alias"]
model = "old"
"#
        .parse::<DocumentMut>()
        .expect("valid toml");
        project_provider_models(&mut document, &settings, "custom").expect("project ok");
        let text = render(&document);
        assert!(!text.contains("default_model"), "text: {text}");
        assert!(!text.contains("[providers"), "text: {text}");
    }

    #[test]
    fn project_defaults_default_model_to_first_catalog_model() {
        // No explicit defaultModelKey: point at the first catalog model
        // instead of the official alias.
        let settings = parse(
            r#"{
                "modelCatalog": { "models": [ { "key": "relay/a", "model": "a" } ]}
            }"#,
        );
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &settings, "custom").expect("project ok");
        assert!(render(&document).contains("default_model = \"relay/a\""));
    }

    #[test]
    fn project_synthesizes_official_default_model_entry() {
        // Official channels keep no client-side catalog; the fallback default
        // must still get a [models] table or the CLI refuses to resolve it.
        let settings = parse("{}");
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &settings, "official").expect("project ok");
        let text = render(&document);
        assert!(
            text.contains("default_model = \"kimi-code/kimi-for-coding\""),
            "text: {text}"
        );
        assert!(
            text.contains("[models.\"kimi-code/kimi-for-coding\"]"),
            "text: {text}"
        );
        assert!(text.contains("model = \"kimi-for-coding\""));
        assert!(text.contains("provider = \"managed:kimi-code\""));
        assert!(text.contains("display_name = \"K2.7 Coding\""));
        assert!(text.contains("max_context_size = 262144"));
    }

    #[test]
    fn project_synthesizes_missing_official_catalog_entry() {
        // An explicit official defaultModelKey absent from the catalog must be
        // synthesized too (covers legacy rows with dangling keys).
        let settings = parse(r#"{ "defaultModelKey": "kimi-code/kimi-for-coding-highspeed" }"#);
        let mut document = DocumentMut::new();
        project_provider_models(&mut document, &settings, "official").expect("project ok");
        let text = render(&document);
        assert!(
            text.contains("default_model = \"kimi-code/kimi-for-coding-highspeed\""),
            "text: {text}"
        );
        assert!(
            text.contains("[models.\"kimi-code/kimi-for-coding-highspeed\"]"),
            "text: {text}"
        );
        assert!(text.contains("model = \"kimi-for-coding-highspeed\""));
    }

    #[test]
    fn list_kimi_plugins_reads_installed_json_or_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let plugins_dir = temp.path().join("plugins");
        fs::create_dir_all(&plugins_dir).expect("create dir");

        // Non-existent installed.json, empty dir
        let empty = list_kimi_plugins_from_dir(&plugins_dir);
        assert!(empty.is_empty());

        // With installed.json array
        let installed_json = r#"[
            { "name": "kimi-search", "version": "1.0.0", "description": "Web search plugin", "enabled": true },
            { "id": "kimi-code-helper", "version": "0.2.0", "status": "enabled" }
        ]"#;
        fs::write(plugins_dir.join("installed.json"), installed_json)
            .expect("write installed.json");
        let plugins = list_kimi_plugins_from_dir(&plugins_dir);
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].name, "kimi-search");
        assert_eq!(plugins[0].version.as_deref(), Some("1.0.0"));
        assert_eq!(plugins[0].description.as_deref(), Some("Web search plugin"));
        assert_eq!(plugins[0].enabled, Some(true));
        assert_eq!(plugins[1].name, "kimi-code-helper");
        assert_eq!(plugins[1].version.as_deref(), Some("0.2.0"));
        assert_eq!(plugins[1].enabled, Some(true));
    }
}
