use chrono::Local;
use serde_json::Value;
use std::path::Path;

use super::adapter;
use super::config_writer;
use super::constants::{COMMON_CONFIG_ID, OFFICIAL_PROVIDER_ID};
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::proxy_gateway::{
    cli_proxy, paths::ProxyGatewayPaths, settings as gateway_settings, types::GatewayCliKey,
};
use crate::db::helpers::{db_delete, db_get, db_list, db_put};
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::{Emitter, Manager, Runtime};

// ============================================================================
// SQLite CRUD wrappers
// ============================================================================

fn provider_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::single(OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?))
}

fn list_providers_from_sqlite(db: &SqliteDbState) -> Result<Vec<ClaudeDesktopProvider>, String> {
    let order = provider_order()?;
    db.with_conn(|conn| {
        Ok(db_list(conn, DbTable::ClaudeDesktopProvider, Some(&order))?
            .into_iter()
            .map(adapter::from_db_value_provider)
            .filter(|provider| provider.id != COMMON_CONFIG_ID)
            .collect())
    })
}

fn get_provider_from_sqlite(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<Option<ClaudeDesktopProvider>, String> {
    db.with_conn(|conn| {
        Ok(db_get(conn, DbTable::ClaudeDesktopProvider, provider_id)?
            .map(adapter::from_db_value_provider))
    })
}

fn put_provider_to_sqlite(
    db: &SqliteDbState,
    provider_id: &str,
    content: &ClaudeDesktopProviderContent,
) -> Result<(), String> {
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::ClaudeDesktopProvider,
            provider_id,
            &adapter::to_db_value_provider(content),
        )
    })
}

fn delete_provider_from_sqlite(db: &SqliteDbState, provider_id: &str) -> Result<(), String> {
    db.with_conn(|conn| db_delete(conn, DbTable::ClaudeDesktopProvider, provider_id).map(|_| ()))
}

fn get_common_from_sqlite(db: &SqliteDbState) -> Result<Option<ClaudeDesktopCommonConfig>, String> {
    db.with_conn(|conn| {
        Ok(
            db_get(conn, DbTable::ClaudeDesktopProvider, COMMON_CONFIG_ID)?
                .map(adapter::from_db_value_common),
        )
    })
}

fn put_common_to_sqlite(db: &SqliteDbState, config: &str) -> Result<(), String> {
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::ClaudeDesktopProvider,
            COMMON_CONFIG_ID,
            &adapter::to_db_value_common(config),
        )
    })
}

// ============================================================================
// Paths & Status Commands
// ============================================================================

/// Get the resolved Claude Desktop config paths for the current platform.
#[tauri::command]
pub async fn get_claude_desktop_paths() -> Result<ClaudeDesktopPathInfo, String> {
    Ok(config_writer::get_claude_desktop_path_info())
}

/// Get Claude Desktop status from the on-disk meta + profile files.
/// State is judged from `appliedId` and the profile, not `deploymentMode`.
#[tauri::command]
pub async fn get_claude_desktop_status(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ClaudeDesktopStatus, String> {
    if !config_writer::is_supported_platform() {
        return Ok(ClaudeDesktopStatus {
            supported: false,
            configured: false,
            applied_id: None,
            profile_path: None,
            config_library_path: None,
            mode: None,
            actual_base_url: None,
        });
    }

    let paths = config_writer::current_platform_paths()?;
    let applied_id = config_writer::read_applied_id(&paths.meta_path);
    let configured =
        paths.profile_path.exists() || config_writer::meta_has_profile_entry(&paths.meta_path);

    let profile = config_writer::read_profile_json(&paths.profile_path);
    let actual_base_url = profile
        .get("inferenceGatewayBaseUrl")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mode = if let Some(applied_id) = applied_id.as_deref() {
        get_provider_from_sqlite(state.db(), applied_id)?
            .map(|provider| config_writer::provider_mode(provider.meta.as_ref()))
    } else {
        None
    };

    Ok(ClaudeDesktopStatus {
        supported: true,
        configured,
        applied_id,
        profile_path: Some(paths.profile_path.to_string_lossy().to_string()),
        config_library_path: Some(paths.config_library_path.to_string_lossy().to_string()),
        mode,
        actual_base_url,
    })
}

/// Read the current on-disk Claude Desktop 3P files for the preview modal.
#[tauri::command]
pub async fn get_claude_desktop_preview() -> Result<Value, String> {
    if !config_writer::is_supported_platform() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    let paths = config_writer::current_platform_paths()?;
    Ok(serde_json::json!({
        "normalConfig": config_writer::read_json_file_or_empty(&paths.normal_config_path).unwrap_or_else(|_| serde_json::json!({})),
        "threepConfig": config_writer::read_json_file_or_empty(&paths.threep_config_path).unwrap_or_else(|_| serde_json::json!({})),
        "profile": config_writer::read_profile_json(&paths.profile_path),
        "meta": config_writer::read_json_file_or_empty(&paths.meta_path).unwrap_or_else(|_| serde_json::json!({})),
    }))
}

// ============================================================================
// Provider CRUD Commands
// ============================================================================

/// List all Claude Desktop providers ordered by sort_index.
#[tauri::command]
pub async fn list_claude_desktop_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<ClaudeDesktopProvider>, String> {
    list_providers_from_sqlite(state.db())
}

/// Create a new Claude Desktop provider.
#[tauri::command]
pub async fn create_claude_desktop_provider<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    provider: ClaudeDesktopProviderInput,
) -> Result<ClaudeDesktopProvider, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let content = ClaudeDesktopProviderContent {
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
        is_applied: false,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_id = db_new_id();
    put_provider_to_sqlite(db, &provider_id, &content)?;

    let _ = app.emit("config-changed", "window");
    Ok(ClaudeDesktopProvider {
        id: provider_id,
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
    })
}

/// Update an existing Claude Desktop provider.
#[tauri::command]
pub async fn update_claude_desktop_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: ClaudeDesktopProvider,
) -> Result<ClaudeDesktopProvider, String> {
    let db = state.db();
    let id = provider.id.clone();
    let existing = get_provider_from_sqlite(db, &id)?
        .ok_or_else(|| format!("Claude Desktop provider with ID '{}' not found", id))?;

    let created_at = if !provider.created_at.trim().is_empty() {
        provider.created_at.clone()
    } else if existing.created_at.trim().is_empty() {
        Local::now().to_rfc3339()
    } else {
        existing.created_at
    };

    let now = Local::now().to_rfc3339();
    let content = ClaudeDesktopProviderContent {
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
        is_applied: provider.is_applied,
        is_disabled: existing.is_disabled,
        created_at,
        updated_at: now,
    };

    put_provider_to_sqlite(db, &id, &content)?;

    // If this provider is currently applied, rewrite the on-disk profile immediately.
    if content.is_applied {
        if let Err(error) = (|| -> Result<(), String> {
            ensure_claude_desktop_gateway_direct(&app)?;
            apply_provider_to_sqlite_provider(&db, &id)
        })() {
            eprintln!("Failed to auto-apply updated provider: {}", error);
        }
    }

    let _ = app.emit("config-changed", "window");
    Ok(ClaudeDesktopProvider {
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
    })
}

/// Toggle a Claude Desktop provider's disabled flag.
#[tauri::command]
pub async fn toggle_claude_desktop_provider_disabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.db();

    let provider = get_provider_from_sqlite(db, &provider_id)?.ok_or_else(|| {
        format!(
            "Claude Desktop provider with ID '{}' not found",
            provider_id
        )
    })?;
    let is_applied = provider.is_applied;
    let now = Local::now().to_rfc3339();
    let content = ClaudeDesktopProviderContent {
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
        is_applied: provider.is_applied,
        is_disabled,
        created_at: provider.created_at,
        updated_at: now,
    };
    put_provider_to_sqlite(db, &provider_id, &content)?;

    // If this provider is currently applied, rewrite the on-disk profile (the
    // apply path checks is_disabled internally and skips disabled providers).
    if is_applied {
        apply_config_internal(&db, &app, &provider_id, false).await?;
    }

    Ok(())
}

/// Delete a Claude Desktop provider.
#[tauri::command]
pub async fn delete_claude_desktop_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    delete_provider_from_sqlite(state.db(), &id)?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Reorder Claude Desktop providers.
#[tauri::command]
pub async fn reorder_claude_desktop_providers(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    for (index, id) in ids.iter().enumerate() {
        if let Some(mut provider) = get_provider_from_sqlite(db, id)? {
            provider.sort_index = Some(index as i32);
            provider.updated_at = now.clone();
            put_provider_to_sqlite(
                db,
                id,
                &ClaudeDesktopProviderContent {
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
                    is_applied: provider.is_applied,
                    is_disabled: provider.is_disabled,
                    created_at: provider.created_at,
                    updated_at: provider.updated_at,
                },
            )?;
        }
    }

    Ok(())
}

/// Mark a provider as applied in the database without writing to disk.
#[tauri::command]
pub async fn select_claude_desktop_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    for mut provider in list_providers_from_sqlite(db)? {
        let should_be_applied = provider.id == id;
        if provider.is_applied == should_be_applied {
            continue;
        }
        let provider_id = provider.id.clone();
        provider.is_applied = should_be_applied;
        provider.updated_at = now.clone();
        put_provider_to_sqlite(
            db,
            &provider_id,
            &ClaudeDesktopProviderContent {
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
                is_applied: provider.is_applied,
                is_disabled: provider.is_disabled,
                created_at: provider.created_at,
                updated_at: provider.updated_at,
            },
        )?;
    }

    let _ = app.emit("config-changed", "window");
    record_last_used_best_effort(&db, "claudedesktop", &id);
    Ok(())
}

// ============================================================================
// Apply Config
// ============================================================================

/// Apply a provider's configuration to the Claude Desktop files.
#[tauri::command]
pub async fn apply_claude_desktop_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    let db = state.db();
    apply_config_internal(&db, &app, &provider_id, false).await
}

/// True when `settings_config` is the empty-credentials shape the form writes
/// for an official channel provider (`{"env":{}}` or an env object without a
/// base URL / auth token). This distinguishes form-created official providers
/// (which restore to 1P on apply) from imported rows that happen to carry
/// `category="official"` but still have their own upstream credentials and must
/// go through the direct/proxy apply path.
fn official_restore_settings(settings_config: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(settings_config) else {
        return false;
    };
    let Some(env) = value.get("env").and_then(Value::as_object) else {
        // No env object at all (e.g. "{}") counts as empty-credentials.
        return true;
    };
    let has_credential = env
        .get("ANTHROPIC_BASE_URL")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.trim().is_empty())
        || env
            .get("ANTHROPIC_AUTH_TOKEN")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.trim().is_empty());
    !has_credential
}

/// Write a provider's config to the on-disk profile (no DB changes).
fn apply_provider_to_sqlite_provider(db: &SqliteDbState, provider_id: &str) -> Result<(), String> {
    let provider = get_provider_from_sqlite(db, provider_id)?
        .ok_or_else(|| format!("Provider not found: {provider_id}"))?;

    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider.name
        ));
    }

    let paths = config_writer::current_platform_paths()?;

    // Official restore is driven by the stable provider id, or by a provider
    // created through the form's official channel (category=official with empty
    // credentials). A user/imported provider whose category happens to be
    // "official" but still carries its own credentials is NOT treated as an
    // official restore — it goes through the direct/proxy apply path below.
    if provider.id == OFFICIAL_PROVIDER_ID
        || (provider.category == "official" && official_restore_settings(&provider.settings_config))
    {
        return config_writer::restore_official(&paths);
    }

    // Parse once up-front: routing decisions and direct apply both read the
    // settings (env may carry Claude Code style role models for imported rows).
    let parsed_settings = serde_json::from_str::<Value>(&provider.settings_config);
    let settings_ref = parsed_settings.as_ref().ok();
    let needs_routing = config_writer::has_routing_models(provider.meta.as_ref(), settings_ref);
    if needs_routing {
        // Route the 3P profile at the local gateway: the provider's model names
        // are not claude-safe, so Direct mode cannot be written and the gateway
        // performs the upstream mapping. Menu models are surfaced via
        // inferenceModels.
        let origin = gateway_origin_from_settings(db)?;
        let gateway_endpoint = format!("{origin}/claude-desktop");
        let model_specs =
            config_writer::desktop_proxy_model_specs(provider.meta.as_ref(), settings_ref);
        let model_specs = (!model_specs.is_empty()).then_some(model_specs.as_slice());
        return config_writer::apply_gateway_proxy_profile(
            &paths,
            &gateway_endpoint,
            "ai-toolbox-gateway",
            model_specs,
        );
    }

    let settings_config = serde_json::from_str::<Value>(&provider.settings_config)
        .map_err(|error| format!("Failed to parse provider config: {error}"))?;
    config_writer::apply_provider_to_paths(settings_config, provider.meta.as_ref(), &paths)
}

/// Resolve the gateway origin from persisted `ProxyGatewaySettings`. Returns a
/// clear error when the gateway has no configured listen address yet.
fn gateway_origin_from_settings(db: &SqliteDbState) -> Result<String, String> {
    let settings = gateway_settings::load_settings_from_sqlite_state(db)?;
    let host = settings.listen_host.trim();
    if host.is_empty() || settings.listen_port == 0 {
        return Err(
            "本地代理网关尚未配置(缺少监听地址/端口),请先在网关设置中启动并配置网关".to_string(),
        );
    }
    Ok(format!("http://{host}:{}", settings.listen_port))
}

/// Whether the local gateway has taken over the Claude Desktop config files.
fn claude_desktop_gateway_takeover_active<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    app.path()
        .app_data_dir()
        .map(ProxyGatewayPaths::new)
        .map(|paths| {
            cli_proxy::provider_switch_locked_by_manifest(&paths, GatewayCliKey::ClaudeDesktop)
        })
        .unwrap_or(false)
}

/// Reject a plain (direct) apply while the gateway is taking over Claude
/// Desktop, so the profile files are not rewritten out from under the gateway.
fn ensure_claude_desktop_gateway_direct<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), String> {
    if claude_desktop_gateway_takeover_active(app) {
        return Err(
            "当前 Claude Desktop 已由网关接管，请通过网关代理切换入口切换渠道，或先恢复直连"
                .to_string(),
        );
    }
    Ok(())
}

/// Internal: apply + update `is_applied` in the database.
pub async fn apply_config_internal<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_config_internal_with_sync(db, app, provider_id, from_tray, true).await
}

pub async fn apply_config_internal_with_sync<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
    _emit_sync_request: bool,
) -> Result<(), String> {
    apply_config_internal_with_events(db, app, provider_id, from_tray, true, _emit_sync_request)
        .await
}

pub async fn apply_config_internal_without_events<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_internal_with_events(db, app, provider_id, false, false, false).await
}

async fn apply_config_internal_with_events<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
    emit_config_changed: bool,
    _emit_sync_request: bool,
) -> Result<(), String> {
    ensure_claude_desktop_gateway_direct(app)?;
    apply_provider_to_sqlite_provider(db, provider_id)?;

    // Update provider is_applied status.
    let now = Local::now().to_rfc3339();
    for mut provider in list_providers_from_sqlite(db)? {
        let should_be_applied = provider.id == provider_id;
        if provider.is_applied == should_be_applied {
            continue;
        }
        let current_id = provider.id.clone();
        provider.is_applied = should_be_applied;
        provider.updated_at = now.clone();
        put_provider_to_sqlite(
            db,
            &current_id,
            &ClaudeDesktopProviderContent {
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
                is_applied: provider.is_applied,
                is_disabled: provider.is_disabled,
                created_at: provider.created_at,
                updated_at: provider.updated_at,
            },
        )?;
    }

    if emit_config_changed {
        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
    }

    record_last_used_best_effort(db, "claudedesktop", provider_id);

    Ok(())
}

/// Best-effort "recently used" marker for provider list sorting. Failures must
/// never break the provider apply flow itself.
fn record_last_used_best_effort(db: &SqliteDbState, module: &str, provider_id: &str) {
    if let Err(error) =
        crate::settings::provider_list_state::record_provider_last_used_in_sqlite_state(
            db,
            module,
            provider_id,
        )
    {
        log::warn!("Failed to record provider last-used for {module}:{provider_id}: {error}");
    }
}

// ============================================================================
// Common Config Commands
// ============================================================================

fn load_common_from_disk(path: &Path, label: &str) -> Option<String> {
    if !path.exists() {
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(value) => serde_json::to_string_pretty(&value).ok(),
            Err(_) => Some(content),
        },
        Err(error) => {
            log::debug!("Failed to read {label} for common config: {error}");
            None
        }
    }
}

/// Get the Claude Desktop common (base) config. Prefers the DB copy and falls
/// back to the current on-disk config file.
#[tauri::command]
pub async fn get_claude_desktop_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<ClaudeDesktopCommonConfig>, String> {
    if let Some(config) = get_common_from_sqlite(state.db())? {
        return Ok(Some(config));
    }

    if config_writer::is_supported_platform() {
        if let Ok(paths) = config_writer::current_platform_paths() {
            let disk = load_common_from_disk(&paths.threep_config_path, "Claude Desktop config")
                .or_else(|| {
                    load_common_from_disk(&paths.normal_config_path, "Claude Desktop config")
                });
            if let Some(disk_config) = disk {
                return Ok(Some(ClaudeDesktopCommonConfig {
                    config: disk_config,
                    updated_at: Local::now().to_rfc3339(),
                }));
            }
        }
    }

    Ok(None)
}

/// Save the Claude Desktop common (base) config and re-apply the applied provider.
#[tauri::command]
pub async fn save_claude_desktop_common_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: ClaudeDesktopCommonConfigInput,
) -> Result<(), String> {
    // Validate JSON.
    serde_json::from_str::<Value>(&input.config)
        .map_err(|error| format!("Invalid JSON: {error}"))?;

    let db = state.db();
    put_common_to_sqlite(db, &input.config)?;

    // Re-apply the currently applied provider so the base config stays consistent.
    if let Some(applied) = list_providers_from_sqlite(db)?
        .into_iter()
        .find(|provider| provider.is_applied)
    {
        if let Err(error) = (|| -> Result<(), String> {
            ensure_claude_desktop_gateway_direct(&app)?;
            apply_provider_to_sqlite_provider(&db, &applied.id)
        })() {
            eprintln!(
                "Failed to auto-apply config after common config update: {}",
                error
            );
        }
    }

    let _ = app.emit("config-changed", "window");
    Ok(())
}

// ============================================================================
// Import & Official Seed
// ============================================================================

/// Import Claude Code providers into the Claude Desktop provider table so users
/// can reuse their base-url / token channels across both tools.
#[tauri::command]
pub async fn import_claude_desktop_providers_from_claude(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<usize, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    let claude_providers = db.with_conn(|conn| {
        Ok(db_list(conn, DbTable::ClaudeProvider, None)?
            .into_iter()
            .map(crate::coding::claude_code::adapter::from_db_value_provider)
            .collect::<Vec<_>>())
    })?;

    let existing_source_ids: std::collections::HashSet<String> = list_providers_from_sqlite(db)?
        .into_iter()
        .filter_map(|provider| provider.source_provider_id)
        .collect();

    let mut imported = 0usize;
    for claude in claude_providers {
        if claude.id == "__local__" {
            continue;
        }
        let source_id = format!("claude:{}", claude.id);
        if existing_source_ids.contains(&source_id) {
            continue;
        }

        let content = ClaudeDesktopProviderContent {
            name: claude.name,
            category: claude.category,
            settings_config: claude.settings_config,
            source_provider_id: Some(source_id.clone()),
            website_url: claude.website_url,
            notes: claude.notes,
            icon: claude.icon,
            icon_color: claude.icon_color,
            sort_index: None,
            meta: claude.meta,
            is_applied: false,
            is_disabled: false,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        let provider_id = db_new_id();
        put_provider_to_sqlite(db, &provider_id, &content)?;
        imported += 1;
    }

    let _ = app.emit("config-changed", "window");
    Ok(imported)
}

/// Ensure a seeded "Claude Desktop Official" provider exists; create it if absent.
#[tauri::command]
pub async fn ensure_claude_desktop_official_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<ClaudeDesktopProvider, String> {
    let db = state.db();
    if let Some(existing) = get_provider_from_sqlite(db, OFFICIAL_PROVIDER_ID)? {
        return Ok(existing);
    }

    let now = Local::now().to_rfc3339();
    let content = ClaudeDesktopProviderContent {
        name: "Claude Desktop Official".to_string(),
        category: "official".to_string(),
        settings_config: "{\"env\":{}}".to_string(),
        source_provider_id: Some(OFFICIAL_PROVIDER_ID.to_string()),
        website_url: Some("https://claude.ai/download".to_string()),
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: Some(serde_json::json!({ "claude_desktop_mode": "direct" })),
        is_applied: false,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    put_provider_to_sqlite(db, OFFICIAL_PROVIDER_ID, &content)?;
    let _ = app.emit("config-changed", "window");

    Ok(ClaudeDesktopProvider {
        id: OFFICIAL_PROVIDER_ID.to_string(),
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
    })
}

// ============================================================================
// All API Hub import (Claude Desktop)
// ============================================================================

#[tauri::command]
pub async fn list_claude_desktop_all_api_hub_providers(
) -> Result<crate::coding::all_api_hub::AllApiHubProvidersResult, String> {
    let discovery = crate::coding::all_api_hub::list_provider_candidates()?;
    let providers = crate::coding::all_api_hub::build_all_api_hub_items(
        &discovery.providers,
        crate::coding::all_api_hub::candidate_to_claude_desktop_settings,
    );
    Ok(crate::coding::all_api_hub::AllApiHubProvidersResult {
        found: discovery.found,
        profiles: discovery.profiles,
        providers,
        message: discovery.message,
    })
}

#[tauri::command]
pub async fn resolve_claude_desktop_all_api_hub_providers(
    state: tauri::State<'_, crate::db::SqliteDbState>,
    request: crate::coding::all_api_hub::AllApiHubResolveRequest,
) -> Result<Vec<crate::coding::all_api_hub::AllApiHubProviderItem>, String> {
    let providers = crate::coding::all_api_hub::resolve_provider_candidates_with_keys(
        &state,
        &request.provider_ids,
    )
    .await?;
    Ok(crate::coding::all_api_hub::build_all_api_hub_items(
        &providers,
        crate::coding::all_api_hub::candidate_to_claude_desktop_settings,
    ))
}

#[cfg(test)]
mod tests {
    use super::official_restore_settings;

    #[test]
    fn official_restore_settings_for_empty_env() {
        // Form official mode writes exactly `{"env":{}}`.
        assert!(official_restore_settings(r#"{"env":{}}"#));
        // A bare object with no env at all also counts as empty-credentials.
        assert!(official_restore_settings(r#"{}"#));
    }

    #[test]
    fn official_restore_settings_rejects_providers_with_credentials() {
        // A custom provider carrying base url + token must NOT be treated as an
        // official restore target, even if category were "official".
        let with_creds =
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://x","ANTHROPIC_AUTH_TOKEN":"sk"}}"#;
        assert!(!official_restore_settings(with_creds));

        // Only base url present is still a credential -> not official restore.
        let base_only = r#"{"env":{"ANTHROPIC_BASE_URL":"https://x"}}"#;
        assert!(!official_restore_settings(base_only));

        // Whitespace-only credential is treated as absent.
        let blank = r#"{"env":{"ANTHROPIC_BASE_URL":"   "}}"#;
        assert!(official_restore_settings(blank));
    }

    #[test]
    fn official_restore_settings_handles_invalid_json() {
        assert!(!official_restore_settings("not json"));
    }
}
