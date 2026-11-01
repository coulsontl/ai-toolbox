//! ZCode backend commands.
//!
//! ZCode has no single "active provider": every enabled provider shows up in
//! the model picker at once. "Apply" therefore means writing the provider rule,
//! enabling it, and pointing `defaultModelSelection` at it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::Local;
use serde_json::{json, Value};
use tauri::Emitter;
use tempfile::NamedTempFile;
use tokio::sync::Mutex as AsyncMutex;

use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{Cli, CliConfigKind, DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;

use super::constants::*;
use super::projection;
use super::types::{
    ConfigPathInfo, ZcodeCommonConfigInput, ZcodeConfigPreview, ZcodePreviewFile,
    ZcodePromptConfig, ZcodePromptConfigInput, ZcodeProviderInput,
};
use crate::coding::open_code::types::{
    OpenCodeAllApiHubProvider, OpenCodeAllApiHubProvidersResult,
    ResolveOpenCodeAllApiHubProvidersRequest,
};

/// Serializes every read-modify-write pass over `provider_config.json`.
///
/// Each pass builds the next document from its own read snapshot; two
/// concurrent writes would otherwise project from the same stale snapshot and
/// the later one would drop the earlier one's provider. Entry points hold this
/// across the whole "read disk -> write DB -> project" window; `_locked`
/// bodies must never acquire it (it is not reentrant).
static CONFIG_WRITE_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

fn provider_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::created_at(OrderDirection::Asc),
    ]))
}

fn prompt_order() -> Result<OrderSpec, String> {
    provider_order()
}

pub(crate) fn write_text_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("Failed to create temp file for {}: {error}", path.display()))?;
    temporary
        .write_all(content.as_bytes())
        .map_err(|error| format!("Failed to write temp file for {}: {error}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Failed to persist {}: {error}", path.display()))?;
    Ok(())
}

fn zcode_root_dir_from_db(state: &SqliteDbState) -> Result<Option<String>, String> {
    let record = state.with_conn(|conn| {
        crate::db::helpers::db_get(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Common), "common")
    })?;
    Ok(record
        .map(super::adapter::from_db_value_common)
        .and_then(|record| record.root_dir)
        .filter(|value| !value.trim().is_empty()))
}

/// Resolves the effective ZCode data root.
///
/// Delegates to the shared runtime-location chain so every caller agrees on the
/// same directory. Resolving here independently previously skipped the shell
/// profile and `setting.json.dataBaseDir` fallbacks, which meant the registry
/// was read and written at a path the ZCode runtime never looks at.
pub fn resolve_zcode_root_dir(state: &SqliteDbState) -> Result<PathBuf, String> {
    if let Some(root) = zcode_root_dir_from_db(state)? {
        return Ok(PathBuf::from(root));
    }
    Ok(crate::coding::runtime_location::resolve_zcode_root_dir_without_db())
}

pub fn zcode_provider_config_path(state: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(resolve_zcode_root_dir(state)?.join(ZCODE_PROVIDER_CONFIG_RELATIVE_PATH))
}

pub fn zcode_prompt_path(state: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(resolve_zcode_root_dir(state)?.join(ZCODE_PROMPT_FILE_NAME))
}

#[tauri::command]
pub async fn get_zcode_config_file_path(state: tauri::State<'_, SqliteDbState>) -> Result<String, String> {
    Ok(zcode_provider_config_path(&state)?.to_string_lossy().to_string())
}

/// All API Hub listings, straight through from the shared scanner.
///
/// The browser extension reports providers with endpoints and keys, which is
/// all ZCode needs — the conversion into ZCode's own shape happens on the
/// frontend, so there is nothing to adapt here.
#[tauri::command]
pub async fn list_zcode_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OpenCodeAllApiHubProvidersResult, String> {
    crate::coding::open_code::commands::list_opencode_all_api_hub_providers(state).await
}

#[tauri::command]
pub async fn resolve_zcode_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
    request: ResolveOpenCodeAllApiHubProvidersRequest,
) -> Result<Vec<OpenCodeAllApiHubProvider>, String> {
    crate::coding::open_code::commands::resolve_opencode_all_api_hub_providers(state, request).await
}

#[tauri::command]
pub async fn get_zcode_root_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ConfigPathInfo, String> {
    let custom = zcode_root_dir_from_db(&state)?;
    let source = if custom.is_some() {
        "custom"
    } else if std::env::var(ZCODE_DATA_BASE_DIR_ENV)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        "env"
    } else {
        "default"
    };
    Ok(ConfigPathInfo {
        path: resolve_zcode_root_dir(&state)?.to_string_lossy().to_string(),
        source: source.to_string(),
    })
}

#[tauri::command]
pub async fn reveal_zcode_config_folder(state: tauri::State<'_, SqliteDbState>) -> Result<(), String> {
    let root = resolve_zcode_root_dir(&state)?;
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    tauri_plugin_opener::reveal_item_in_dir(&root).map_err(|error| error.to_string())
}

/// Reads the live provider registry so the page can show what ZCode sees.
#[tauri::command]
pub async fn read_zcode_settings(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Value, String> {
    let path = zcode_provider_config_path(&state)?;
    Ok(projection::read_provider_config_base(&path))
}

/// Whether the user has migrated to the new-generation registry.
///
/// When this is false ZCode still reads the legacy `config.json` provider map,
/// so applying a provider would silently have no effect.
#[tauri::command]
pub async fn get_zcode_generation_status(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<bool, String> {
    let path = zcode_provider_config_path(&state)?;
    Ok(projection::provider_config_exists(&path))
}

#[tauri::command]
pub async fn get_zcode_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Value, String> {
    let record = state.with_conn(|conn| {
        crate::db::helpers::db_get(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Common), "common")
    })?;
    match record {
        Some(value) => {
            // The stored record is snake_case, but the web layer reads camelCase
            // keys, so convert through the API DTO rather than serializing the
            // storage shape.
            let record = super::adapter::from_db_value_common(value);
            let parsed: super::types::ZcodeCommonConfig = record.into();
            Ok(serde_json::to_value(parsed).unwrap_or_else(|_| serde_json::json!({})))
        }
        None => Ok(serde_json::json!({
            "config": "{}",
            "rootDir": Value::Null,
            "officialAccountIndex": Value::Null,
            "updatedAt": ""
        })),
    }
}

/// Reads the common-config record, or an empty one when nothing is stored yet.
fn read_common_record(state: &SqliteDbState) -> Result<super::types::ZcodeCommonConfigRecord, String> {
    let record = state.with_conn(|conn| {
        crate::db::helpers::db_get(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Common), "common")
    })?;
    Ok(record
        .map(super::adapter::from_db_value_common)
        .unwrap_or(super::types::ZcodeCommonConfigRecord {
            id: "common".to_string(),
            config: "{}".to_string(),
            root_dir: None,
            official_account_index: None,
            updated_at: String::new(),
        }))
}

#[tauri::command]
pub async fn save_zcode_common_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ZcodeCommonConfigInput,
) -> Result<(), String> {
    let clear = input.clear_root_dir.unwrap_or(false);
    let root_dir = if clear {
        None
    } else {
        input.root_dir.as_deref().map(str::trim).filter(|v| !v.is_empty())
    };
    // Read-modify-write: the record also carries the official-account card's
    // position, and a blind put of just `config` + `root_dir` would erase it.
    let existing = read_common_record(&state)?;
    let payload = super::adapter::to_db_value_common(
        &input.config,
        root_dir,
        existing.official_account_index,
    );
    state.with_conn(|conn| {
        crate::db::helpers::db_put(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Common), "common", &payload)
    })?;
    crate::coding::runtime_location::refresh_runtime_location_cache_for_module_async(&state, "zcode")
        .await
        .ok();
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Records where the official-account card sits in the provider list.
///
/// Kept apart from `save_zcode_common_config` so a drag never rewrites the
/// config blob, and so the config modal never has to carry a field it does not
/// show. `index` counts the provider cards above the official-account card.
#[tauri::command]
pub async fn save_zcode_official_account_index(
    state: tauri::State<'_, SqliteDbState>,
    index: i64,
) -> Result<(), String> {
    let existing = read_common_record(&state)?;
    let payload = super::adapter::to_db_value_common(
        &existing.config,
        existing.root_dir.as_deref(),
        Some(index.max(0)),
    );
    state.with_conn(|conn| {
        crate::db::helpers::db_put(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Common), "common", &payload)
    })
}

/// Resolves `~/.zcode/cli/config.json`.
///
/// This is the CLI's own settings file — MCP servers, hooks, plugins and
/// permission switches. It is entirely separate from `provider_config.json`,
/// so AI Toolbox edits it verbatim rather than projecting a subset like it
/// does for providers.
pub fn zcode_cli_config_path(state: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(resolve_zcode_root_dir(state)?.join(ZCODE_CLI_CONFIG_RELATIVE_PATH))
}

/// Resolves `~/.zcode/v2/setting.json`.
///
/// Unlike every other ZCode file this one does **not** follow the data root:
/// it is the bootstrap file that says where the data root is, so it is always
/// read from the home directory.
pub(crate) fn zcode_setting_path() -> Option<PathBuf> {
    Some(
        dirs::home_dir()?
            .join(ZCODE_DEFAULT_ROOT_DIR_NAME)
            .join(ZCODE_SETTING_RELATIVE_PATH),
    )
}

fn preview_file(path: PathBuf) -> ZcodePreviewFile {
    // A missing file is a normal state — the CLI creates `cli/config.json` on
    // first run — so it becomes `None` and the modal skips the tab.
    let content = std::fs::read_to_string(&path).ok();
    ZcodePreviewFile {
        path: path.to_string_lossy().to_string(),
        content,
    }
}

/// Every file ZCode reads, for the "preview config" modal.
///
/// One file is not the picture: `provider_config.json` holds the providers AI
/// Toolbox manages, `cli/config.json` holds the CLI's own settings, and
/// `setting.json` decides where everything else even lives. Previewing only the
/// first reads as though the others do not exist.
#[tauri::command]
pub async fn get_zcode_preview(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ZcodeConfigPreview, String> {
    let root_dir = resolve_zcode_root_dir(&state)?;
    // The legacy map is only part of the picture while the runtime still reads
    // it; on a migrated install it is a leftover, and showing it would suggest
    // it still matters.
    let legacy_config = (!projection::provider_config_exists(
        &root_dir.join(ZCODE_PROVIDER_CONFIG_RELATIVE_PATH),
    ))
    .then(|| preview_file(root_dir.join(ZCODE_LEGACY_CONFIG_RELATIVE_PATH)));

    Ok(ZcodeConfigPreview {
        provider_config: preview_file(root_dir.join(ZCODE_PROVIDER_CONFIG_RELATIVE_PATH)),
        legacy_config,
        cli_config: preview_file(root_dir.join(ZCODE_CLI_CONFIG_RELATIVE_PATH)),
        setting: zcode_setting_path()
            .map(preview_file)
            .unwrap_or(ZcodePreviewFile {
                path: String::new(),
                content: None,
            }),
    })
}

#[tauri::command]
pub async fn read_zcode_cli_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    let path = zcode_cli_config_path(&state)?;
    match crate::coding::file_io::read_optional_text_file_with_timeout(
        path,
        "ZCode cli/config.json",
    )
    .await?
    {
        Some(text) => Ok(text),
        // A missing file is a valid state: ZCode creates it on first run, and
        // the user may never have configured MCP servers.
        None => Ok(String::new()),
    }
}

#[tauri::command]
pub async fn save_zcode_cli_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config: String,
) -> Result<(), String> {
    // Reject malformed JSON before it reaches disk: ZCode ignores the whole
    // settings file when it fails schema validation, so a bad save would
    // silently drop the user's MCP servers on next launch.
    if !config.trim().is_empty() {
        serde_json::from_str::<Value>(&config)
            .map_err(|error| format!("Invalid JSON: {error}"))?;
    }

    let path = zcode_cli_config_path(&state)?;
    let content = if config.trim().is_empty() {
        String::new()
    } else {
        // Keep the file readable: the user edits it by hand too.
        let parsed: Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&parsed).map_err(|e| e.to_string())?
    };
    write_text_atomic(&path, &content)?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub fn list_zcode_providers_for_db(db: &SqliteDbState) -> Result<Vec<super::types::ZcodeProvider>, String> {
    let order = provider_order()?;
    db.with_conn(|conn| db_list(conn, DbTable::CliProvider(Cli::Zcode), Some(&order))).map(|values| {
        values
            .into_iter()
            .map(super::adapter::from_db_value_provider)
            .collect()
    })
}

/// Mirrors the registry's `defaultModelSelection` into the DB `is_applied` flag.
///
/// The DB row id and the ZCode registry `providerId` are different namespaces,
/// so the row is located through its `settings_config` rather than by primary
/// key. A selection naming a provider AI Toolbox does not manage (a tray pick
/// of `account:`/`builtin:`, for example) clears every flag, which is correct:
/// no managed provider is the default in that case.
fn sync_applied_flag_with_selection(db: &SqliteDbState, provider_id: &str) -> Result<(), String> {
    let target_id = list_zcode_providers_for_db(db)?
        .into_iter()
        .find(|provider| {
            serde_json::from_str::<super::types::ZcodeSettingsConfig>(&provider.settings_config)
                .map(|settings| settings.provider_id == provider_id)
                .unwrap_or(false)
        })
        .map(|provider| provider.id);
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliProvider(Cli::Zcode), target_id.as_deref(), &now)
    })
}

#[tauri::command]
pub async fn list_zcode_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<super::types::ZcodeProvider>, String> {
    list_zcode_providers_for_db(state.inner())
}

fn next_sort_index(db: &SqliteDbState) -> Result<i64, String> {
    db.with_conn(|conn| {
        Ok(db_max_i64(conn, DbTable::CliProvider(Cli::Zcode), &JsonFieldPath::new("sort_index")?)?
            .map(|value| value + 1)
            .unwrap_or(0))
    })
}

/// Mirrors `buildZcodeProviderId` on the frontend: lowercase the name, collapse
/// every run of non-alphanumerics into one `-`, trim the ends.
pub fn zcode_provider_id_slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "provider".to_string()
    } else {
        slug.to_string()
    }
}

/// Derives the row id and settings for a create call that carried no row id.
///
/// Imports hand over a `settings_config` whose `providerId` is the registry
/// identity the row will be projected under. Reusing it as the row id keeps the
/// two ids identical, which `select_zcode_provider` and the tray both address
/// rows by — a generated row id would leave "set as default" unable to find the
/// registry entry and `sync_applied_flag_with_selection` unable to find the row.
/// An empty `providerId` is filled from the display name so an imported row is
/// usable without a manual edit; rows whose settings cannot be parsed keep the
/// generated-id fallback.
pub fn derive_zcode_row_id_and_settings(
    db: &SqliteDbState,
    provider: &ZcodeProviderInput,
) -> Result<(String, String), String> {
    let mut settings_config = provider.settings_config.clone();
    let mut row_id = match serde_json::from_str::<Value>(&settings_config) {
        Ok(mut settings) if settings.is_object() => {
            let existing_id = settings
                .get("providerId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!("custom:{}", zcode_provider_id_slug(&provider.name))
                });
            if projection::is_managed_provider_id(&existing_id)
                && !projection::is_reserved_provider_id(&existing_id)
            {
                settings
                    .as_object_mut()
                    .expect("checked object above")
                    .insert("providerId".to_string(), Value::String(existing_id.clone()));
                settings_config = serde_json::to_string(&settings)
                    .map_err(|error| format!("Failed to serialize ZCode provider settings: {error}"))?;
                existing_id
            } else {
                return Ok((crate::coding::db_id::db_new_id(), settings_config));
            }
        }
        _ => return Ok((crate::coding::db_id::db_new_id(), settings_config)),
    };
    // The registry id must be unique per row: re-importing a favorite whose
    // source row still exists would otherwise overwrite it through the shared
    // id. Suffix the copy and mirror the suffix into the settings it projects.
    let base = row_id.clone();
    let mut suffix = 2;
    loop {
        let taken = db
            .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Zcode), &row_id))?
            .is_some();
        if !taken {
            return Ok((row_id, settings_config));
        }
        row_id = format!("{base}-{suffix}");
        suffix += 1;
        if let Ok(mut settings) = serde_json::from_str::<Value>(&settings_config) {
            settings
                .as_object_mut()
                .expect("checked object above")
                .insert("providerId".to_string(), Value::String(row_id.clone()));
            settings_config = serde_json::to_string(&settings)
                .map_err(|error| format!("Failed to serialize ZCode provider settings: {error}"))?;
        }
    }
}

/// Creates or updates a provider row. The registry projection happens in
/// [`save_zcode_provider`], which holds the write lock.
#[tauri::command]
pub async fn create_zcode_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: ZcodeProviderInput,
) -> Result<super::types::ZcodeProvider, String> {
    let _guard = CONFIG_WRITE_LOCK.lock().await;
    let db = state.inner();
    let (id, settings_config) = if provider.id.is_some() {
        (provider.id.clone().expect("checked above"), provider.settings_config.clone())
    } else {
        derive_zcode_row_id_and_settings(db, &provider)?
    };
    let now = Local::now().to_rfc3339();
    let sort_index = next_sort_index(db)?;
    let content = super::types::ZcodeProviderContent {
        name: provider.name,
        category: provider.category,
        settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index,
        meta: provider.meta,
        is_disabled: provider.is_disabled.unwrap_or(false),
    };
    let mut payload = super::adapter::to_db_value_provider(&content);
    payload["created_at"] = json!(now);
    payload["updated_at"] = json!(now);
    payload["is_applied"] = json!(false);
    db.with_conn(|conn| db_put(conn, DbTable::CliProvider(Cli::Zcode), &id, &payload))?;

    let saved = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Zcode), &id))?
        .map(super::adapter::from_db_value_provider)
        .ok_or_else(|| "Failed to read back the created ZCode provider".to_string())?;
    let _ = app.emit("config-changed", "window");
    Ok(saved)
}

#[tauri::command]
pub async fn update_zcode_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: super::types::ZcodeProvider,
) -> Result<super::types::ZcodeProvider, String> {
    let _guard = CONFIG_WRITE_LOCK.lock().await;
    let db = state.inner();
    let existing = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Zcode), &provider.id))?
        .ok_or_else(|| format!("ZCode provider '{}' not found", provider.id))?;
    let previous = super::adapter::from_db_value_provider(existing);

    let content = super::types::ZcodeProviderContent {
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
        is_disabled: provider.is_disabled,
    };
    let mut payload = super::adapter::to_db_value_provider(&content);
    payload["created_at"] = json!(previous.created_at);
    payload["updated_at"] = json!(Local::now().to_rfc3339());
    payload["is_applied"] = json!(provider.is_applied);
    db.with_conn(|conn| db_put(conn, DbTable::CliProvider(Cli::Zcode), &provider.id, &payload))?;

    let saved = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Zcode), &provider.id))?
        .map(super::adapter::from_db_value_provider)
        .ok_or_else(|| "Failed to read back the updated ZCode provider".to_string())?;
    let _ = app.emit("config-changed", "window");
    Ok(saved)
}

#[tauri::command]
pub async fn delete_zcode_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().await;
    let db = state.inner();
    let record = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Zcode), &id))?
        .ok_or_else(|| format!("ZCode provider '{id}' not found"))?;
    let provider = super::adapter::from_db_value_provider(record);
    let settings: Option<super::types::ZcodeSettingsConfig> =
        serde_json::from_str(&provider.settings_config).ok();

    // Remove the registry entry first: leaving a DB row without its projection
    // is recoverable, the reverse is not.
    if let Some(settings) = settings {
        if !projection::is_reserved_provider_id(&settings.provider_id) {
            let path = zcode_provider_config_path(db)?;
            let mut base = projection::read_provider_config_base(&path);
            projection::remove_managed_provider(&mut base, &settings.provider_id);
            // A selection pointing at the provider being removed would leave
            // ZCode unable to resolve its default model.
            projection::clear_default_model_selection_for(&mut base, &settings.provider_id);
            projection::atomic_write_json(&path, &base)?;
        }
    }
    db.with_conn(|conn| db_delete(conn, DbTable::CliProvider(Cli::Zcode), &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-zcode", ());
    Ok(())
}

#[tauri::command]
pub async fn reorder_zcode_providers(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.inner();
    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CliProvider(Cli::Zcode),
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_zcode_provider_disabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.inner();
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CliProvider(Cli::Zcode),
            &provider_id,
            &[("is_disabled", json!(is_disabled))],
        )
        .map(|_| ())
    })?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Writes one provider's rule and model rows into the registry.
///
/// Shared by the save command and the startup reapply pass, which must be able
/// to rebuild a registry entry that was edited away while the app was closed.
pub async fn project_zcode_provider_internal_without_events(
    db: &SqliteDbState,
    settings: &mut super::types::ZcodeSettingsConfig,
) -> Result<(), String> {
    let path = zcode_provider_config_path(db)?;
    if !projection::provider_config_exists(&path) {
        return Err(
            "ZCode has not migrated to the new provider registry yet. Start the ZCode desktop app once, then retry."
                .to_string(),
        );
    }
    if projection::is_reserved_provider_id(&settings.provider_id) {
        return Err(format!(
            "Provider id '{}' is reserved by ZCode and cannot be managed here.",
            settings.provider_id
        ));
    }
    if !projection::is_managed_provider_id(&settings.provider_id) {
        return Err(format!(
            "Provider id '{}' must start with '{}'. The registry file is shared with ZCode, so only ids in that namespace are rewritten.",
            settings.provider_id,
            super::constants::ZCODE_MANAGED_PROVIDER_ID_PREFIX
        ));
    }
    projection::apply_personal_model_ids(settings);

    let _guard = CONFIG_WRITE_LOCK.lock().await;
    let mut base = projection::read_provider_config_base(&path);
    projection::remove_managed_provider(&mut base, &settings.provider_id);
    projection::upsert_provider_rule(&mut base, projection::build_provider_rule(settings));
    projection::push_model_rules(&mut base, &settings.provider_id, &settings.models);
    projection::atomic_write_json(&path, &base)
}

/// Projects a stored provider into the registry.
///
/// The row is re-read rather than trusting the caller's `settings_config`, so
/// the registry can never diverge from what the provider list renders.
#[tauri::command]
pub async fn save_zcode_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: ZcodeProviderInput,
) -> Result<String, String> {
    let db = state.inner();
    let row_id = provider
        .id
        .as_deref()
        .ok_or_else(|| "ZCode provider id is required to project the registry".to_string())?;
    let stored = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Zcode), row_id))?
        .ok_or_else(|| format!("ZCode provider '{row_id}' not found"))?;
    let mut settings: super::types::ZcodeSettingsConfig =
        serde_json::from_str(&super::adapter::from_db_value_provider(stored).settings_config)
            .map_err(|error| format!("Invalid ZCode provider settings: {error}"))?;
    project_zcode_provider_internal_without_events(db, &mut settings).await?;

    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-zcode", ());
    Ok(settings.provider_id)
}

/// Removes a provider from the registry and from storage.
#[tauri::command]
pub async fn delete_zcode_provider_from_file(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    if projection::is_reserved_provider_id(&provider_id) {
        return Err(format!("Provider id '{provider_id}' is reserved by ZCode."));
    }
    // Held across read -> write so a concurrent save cannot project from a stale
    // snapshot and resurrect the provider this call is removing.
    let _guard = CONFIG_WRITE_LOCK.lock().await;
    let path = zcode_provider_config_path(&state)?;
    let mut base = projection::read_provider_config_base(&path);
    projection::remove_managed_provider(&mut base, &provider_id);
    projection::clear_default_model_selection_for(&mut base, &provider_id);
    projection::atomic_write_json(&path, &base)?;
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-zcode", ());
    Ok(())
}

/// Points `defaultModelSelection` at the given provider.
#[tauri::command]
pub async fn select_zcode_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    model_id: String,
) -> Result<(), String> {
    select_zcode_provider_internal_without_events(state.inner(), &provider_id, &model_id).await?;
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-zcode", ());
    Ok(())
}

/// Points `defaultModelSelection` at a provider, without emitting events.
///
/// Split out for the startup reapply pass, which runs before the window exists
/// and must not fire UI notifications.
pub async fn select_zcode_provider_internal_without_events(
    db: &SqliteDbState,
    provider_id: &str,
    model_id: &str,
) -> Result<(), String> {
    if projection::is_reserved_provider_id(provider_id) {
        return Err(format!("Provider id '{provider_id}' is reserved by ZCode."));
    }
    // Held across read -> write so a concurrent save cannot project from a stale
    // snapshot and lose this selection.
    let _guard = CONFIG_WRITE_LOCK.lock().await;
    let path = zcode_provider_config_path(db)?;
    let mut base = projection::read_provider_config_base(&path);
    let registry_id = if projection::list_provider_ids(&base).contains(&provider_id.to_string())
    {
        provider_id.to_string()
    } else {
        // The argument may be a DB row id instead of a registry id: rows created
        // by older imports used generated row ids while their settings carry the
        // registry identity. Resolve through the stored settings so "set as
        // default" keeps working for those rows.
        let stored = db
            .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Zcode), provider_id))?
            .ok_or_else(|| {
                format!("Provider '{provider_id}' is not present in the ZCode registry.")
            })?;
        let settings: super::types::ZcodeSettingsConfig =
            serde_json::from_str(&super::adapter::from_db_value_provider(stored).settings_config)
                .map_err(|error| format!("Invalid ZCode provider settings: {error}"))?;
        settings.provider_id
    };
    if !projection::list_provider_ids(&base).contains(&registry_id) {
        return Err(format!(
            "Provider '{provider_id}' is not present in the ZCode registry."
        ));
    }
    projection::set_default_model_selection(&mut base, &registry_id, model_id);
    projection::atomic_write_json(&path, &base)?;
    // The card's "default" badge and the startup reapply pass both read this
    // flag, so the registry write and the DB row must move together.
    sync_applied_flag_with_selection(db, &registry_id)
}

/// Lists provider templates from the installed ZCode built-in catalog.
///
/// The catalog path embeds the app version, so it is discovered at call time
/// rather than hardcoded; a bundled fallback keeps the UI usable when ZCode is
/// not installed.
#[tauri::command]
pub async fn list_zcode_provider_templates(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<super::types::ZcodeProviderTemplate>, String> {
    let root = resolve_zcode_root_dir(&state)?;
    Ok(super::templates::read_provider_templates(&root))
}

// ---------------------------------------------------------------------------
// Global prompt (`~/.zcode/AGENTS.md`)
// ---------------------------------------------------------------------------

async fn get_zcode_prompt_record(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<ZcodePromptConfig>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt), id))
        .map(|value| value.map(super::adapter::from_db_value_prompt))
}

/// Reads the prompt currently sitting in `AGENTS.md` as an unmanaged preset.
///
/// `__local__` is the bridge to a file this app does not own: it appears when
/// nothing stored is applied, so the user can see — and adopt — whatever wrote
/// that file (ZCode's own editor, or a previous run). An applied record's
/// content *is* the file, so listing both would show the same prompt twice.
/// Returns `None` for a missing or blank file.
async fn get_zcode_local_prompt_config(
    state: &SqliteDbState,
) -> Result<Option<ZcodePromptConfig>, String> {
    let Some(content) = crate::coding::prompt_file::read_prompt_content_file(
        &zcode_prompt_path(state)?,
        "ZCode",
    )?
    else {
        return Ok(None);
    };
    let now = Local::now().to_rfc3339();
    Ok(Some(ZcodePromptConfig {
        id: crate::coding::local_bridge::LOCAL_CONFIG_ID.to_string(),
        name: ZCODE_PROMPT_FILE_NAME.to_string(),
        content,
        is_applied: false,
        // Sorts ahead of every stored preset, which start at 0.
        sort_index: -1,
        created_at: now.clone(),
        updated_at: now,
    }))
}

#[tauri::command]
pub async fn list_zcode_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<ZcodePromptConfig>, String> {
    let order = prompt_order()?;
    let mut prompts = state
        .with_conn(|conn| db_list(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt), Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .map(super::adapter::from_db_value_prompt)
                .collect::<Vec<_>>()
        })?;
    if !prompts.iter().any(|prompt| prompt.is_applied) {
        if let Some(local_prompt) = get_zcode_local_prompt_config(state.inner()).await? {
            prompts.insert(0, local_prompt);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_zcode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ZcodePromptConfigInput,
) -> Result<ZcodePromptConfig, String> {
    let db = state.inner();
    let id = input.id.clone().unwrap_or_else(crate::coding::db_id::db_new_id);
    let now = Local::now().to_rfc3339();
    let sort_index = db.with_conn(|conn| {
        Ok(db_max_i64(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt), &JsonFieldPath::new("sort_index")?)?
            .map(|value| value + 1)
            .unwrap_or(0))
    })?;
    let payload = json!({
        "name": input.name,
        "content": input.content,
        "is_applied": false,
        "sort_index": sort_index,
        "created_at": now,
        "updated_at": now,
    });
    db.with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt), &id, &payload))?;
    let _ = app.emit("config-changed", "window");
    get_zcode_prompt_record(db, &id)
        .await?
        .ok_or_else(|| "Failed to read back the created ZCode prompt".to_string())
}

#[tauri::command]
pub async fn update_zcode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ZcodePromptConfigInput,
) -> Result<ZcodePromptConfig, String> {
    let db = state.inner();
    let id = input
        .id
        .clone()
        .ok_or_else(|| "ZCode prompt id is required for update".to_string())?;
    let existing = get_zcode_prompt_record(db, &id)
        .await?
        .ok_or_else(|| format!("ZCode prompt '{id}' not found"))?;
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt),
            &id,
            &[
                ("name", json!(input.name)),
                ("content", json!(input.content)),
                ("updated_at", json!(Local::now().to_rfc3339())),
            ],
        )
        .map(|_| ())
    })?;
    // An applied prompt must keep the live file in step with its record.
    if existing.is_applied {
        write_text_atomic(&zcode_prompt_path(db)?, &input.content)?;
        let _ = app.emit("wsl-sync-request-zcode", ());
    }
    let _ = app.emit("config-changed", "window");
    get_zcode_prompt_record(db, &id)
        .await?
        .ok_or_else(|| format!("ZCode prompt '{id}' disappeared after update"))
}

/// Deletes the stored record only; the live `AGENTS.md` is left untouched so a
/// user's current rules are never silently dropped.
#[tauri::command]
pub async fn delete_zcode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt), &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn apply_zcode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_zcode_prompt_config_internal_without_events(state.inner(), &config_id).await?;
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-zcode", ());
    Ok(())
}

/// Writes a prompt config to `AGENTS.md`, without emitting events.
///
/// Split out for the startup reapply pass.
pub async fn apply_zcode_prompt_config_internal_without_events(
    db: &SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    let prompt = get_zcode_prompt_record(db, config_id)
        .await?
        .ok_or_else(|| format!("ZCode prompt '{config_id}' not found"))?;
    write_text_atomic(&zcode_prompt_path(db)?, &prompt.content)?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt), Some(config_id), &now)
    })?;
    Ok(())
}

/// Clears every applied flag and empties the live file while keeping records.
#[tauri::command]
pub async fn disable_zcode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.inner();
    get_zcode_prompt_record(db, &config_id)
        .await?
        .ok_or_else(|| format!("ZCode prompt '{config_id}' not found"))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt), None, &now)
    })?;
    write_text_atomic(&zcode_prompt_path(db)?, "")?;
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-zcode", ());
    Ok(())
}

#[tauri::command]
pub async fn reorder_zcode_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        state.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CliConfig(Cli::Zcode, CliConfigKind::Prompt),
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    Ok(())
}

/// Adopts the current live `AGENTS.md` as a stored prompt record, then applies
/// it.
///
/// Saving the local card has to leave both halves in step: the record holds the
/// edited content, and the file the CLI actually reads gets it too. Creating
/// the record alone would keep the edit out of `AGENTS.md` until a separate
/// apply — the content the user just typed would appear saved but not be live.
/// A blank edit falls back to whatever the file holds now rather than adopting
/// an empty prompt.
#[tauri::command]
pub async fn save_zcode_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ZcodePromptConfigInput,
) -> Result<ZcodePromptConfig, String> {
    let db = state.inner();
    let content = if input.content.trim().is_empty() {
        get_zcode_local_prompt_config(db)
            .await?
            .map(|prompt| prompt.content)
            .unwrap_or_default()
    } else {
        input.content
    };
    let created = create_zcode_prompt_config(
        state.clone(),
        app.clone(),
        ZcodePromptConfigInput {
            id: None,
            name: input.name,
            content,
        },
    )
    .await?;
    apply_zcode_prompt_config_internal_without_events(db, &created.id).await?;
    // `create_*` already emitted `config-changed`; the file itself moved, so
    // the WSL side needs a nudge as well.
    let _ = app.emit("wsl-sync-request-zcode", ());
    get_zcode_prompt_record(db, &created.id)
        .await?
        .ok_or_else(|| "Failed to read back the adopted ZCode prompt".to_string())
}
