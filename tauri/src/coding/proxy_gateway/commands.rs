use super::aggregate_draft;
use super::aggregate_naming::AggregateNamingMode;
use super::cli_proxy;
use super::listen::check_port_available;
use super::model_health;
use super::paths::ProxyGatewayPaths;
use super::pricing;
use super::provider_switch;
use super::request_log;
use super::runtime::ProxyGatewayState;
use super::session_import;
use super::settings;
use super::types::{
    DataSourceBreakdownInput, DataSourceBreakdownItem, GatewayAggregateBareModel,
    GatewayAggregateConfig, GatewayCliKey, GatewayCliTakeoverStatus,
    GatewayConnectivityTestRequest, GatewayConnectivityTestResponse, GatewayModelHealthItem,
    GatewayModelStats, GatewayPaginatedRequestLogs, GatewayProviderStats, GatewayProxyMode,
    GatewayRequestLogDetail, GatewayRequestLogFilters, GatewaySessionUsageImportInput,
    GatewaySessionUsageImportResult, GatewaySubagentCatalog, GatewayUsageSummary,
    GatewayUsageSummaryByCli, GatewayUsageTool, GatewayUsageTrendPoint, ModelPricing,
    ProxyGatewayHealthCheckResult, ProxyGatewayPortCheckInput, ProxyGatewayPortCheckResult,
    ProxyGatewayRequestLogListInput, ProxyGatewaySettings, ProxyGatewayStatus,
    ProxyGatewayStopPreflight,
};
use super::usage_stats;
use crate::db::helpers::db_list;
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::{model_pricing_seed, SqliteDbState};
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tauri::Emitter;

pub async fn proxy_gateway_start_if_enabled_on_startup(
    db_state: &SqliteDbState,
    sqlite_state: &SqliteDbState,
    gateway_state: &ProxyGatewayState,
    app: &tauri::AppHandle,
) -> Result<Option<ProxyGatewayStatus>, String> {
    let settings = settings::load_settings_from_sqlite_state(sqlite_state)?;
    if !settings.enabled_on_startup {
        return Ok(None);
    }
    let paths = proxy_gateway_paths(app)?;

    let mut manager = gateway_state
        .manager
        .lock()
        .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
    manager
        .start_with_context_and_app(settings, db_state.db().clone(), paths, app.clone())
        .map(Some)
}

#[tauri::command]
pub async fn proxy_gateway_get_settings(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<ProxyGatewaySettings, String> {
    settings::load_settings_from_sqlite_state(&sqlite_state)
}

#[tauri::command]
pub async fn proxy_gateway_get_privacy_settings(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<super::privacy::PrivacySettings, String> {
    super::privacy::load_settings(&sqlite_state)
}

#[tauri::command]
pub async fn proxy_gateway_update_privacy_settings(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    update: super::privacy::PrivacySettingsUpdate,
) -> Result<super::privacy::PrivacySettings, String> {
    // Serialize with start/restart and other policy updates. Publish only after persistence succeeds.
    let manager = gateway_state
        .manager
        .lock()
        .map_err(|_| "Proxy gateway manager lock poisoned")?;
    let (settings, policy) = super::privacy::update_settings(&sqlite_state, update)?;
    manager.update_privacy_policy(policy);
    Ok(settings)
}

#[tauri::command]
pub async fn proxy_gateway_preview_privacy(
    rules: super::privacy::PrivacyRules,
    text: String,
) -> Result<super::privacy::PrivacyPreview, String> {
    super::privacy::preview(rules, text)
}

#[tauri::command]
pub async fn proxy_gateway_update_settings(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    mut settings: ProxyGatewaySettings,
) -> Result<ProxyGatewaySettings, String> {
    // Normalize/validate before touching runtime so invalid expressions never
    // change live gateway behavior while the DB write fails.
    let running = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status().running
    };
    if running {
        settings.enabled_on_startup = true;
    }
    let saved = settings::save_settings(&sqlite_state, settings)?;
    if running {
        let mut manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.update_runtime_settings(saved.clone())?;
    }
    Ok(saved)
}

#[tauri::command]
pub async fn proxy_gateway_start(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    settings: Option<ProxyGatewaySettings>,
) -> Result<ProxyGatewayStatus, String> {
    let mut settings = match settings {
        Some(settings) => settings,
        None => settings::load_settings_from_sqlite_state(&sqlite_state)?,
    };
    let paths = proxy_gateway_paths(&app)?;
    let status = {
        let mut manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.start_with_context_and_app(
            settings.clone(),
            db_state.db().clone(),
            paths,
            app.clone(),
        )?
    };

    settings.enabled_on_startup = true;
    if let Err(error) = settings::save_settings(&sqlite_state, settings) {
        log::warn!("Failed to persist proxy gateway startup state after start: {error}");
    }

    if let Err(error) = app.emit("gateway-running-changed", status.running) {
        log::warn!("Failed to emit proxy gateway running status after start: {error}");
    }

    Ok(status)
}

#[tauri::command]
pub async fn proxy_gateway_stop(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<ProxyGatewayStatus, String> {
    let current_status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    let preflight = cli_proxy::stop_preflight(db_state.db(), &paths, &current_status).await;
    if !preflight.allowed {
        return Err(preflight.message.unwrap_or_else(|| {
            "Restore gateway-taken-over CLIs to direct mode before stopping the gateway".to_string()
        }));
    }

    let mut settings = settings::load_settings_from_sqlite_state(&sqlite_state)?;
    settings.enabled_on_startup = false;
    settings::save_settings(&sqlite_state, settings)?;

    let status = {
        let mut manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.stop()?
    };
    if let Err(error) = app.emit("gateway-running-changed", status.running) {
        log::warn!("Failed to emit proxy gateway running status after stop: {error}");
    }
    Ok(status)
}

/// Hot-restart the running gateway without CLI stop preflight.
///
/// Keeps takeover manifests and settings, rebuilds runtime state, and resets
/// blocking runtime health/cache so network-switch recovery does not require
/// disengaging CLIs first.
#[tauri::command]
pub async fn proxy_gateway_restart(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<ProxyGatewayStatus, String> {
    let paths = proxy_gateway_paths(&app)?;
    // Restart may stop the old runtime before rebinding. Always report the final
    // running bit so listeners refresh even when restart fails mid-way.
    let (restart_result, running) = {
        let mut manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        match manager.restart_with_context_and_app(db_state.db().clone(), paths, app.clone()) {
            Ok(status) => {
                let running = status.running;
                (Ok(status), running)
            }
            Err(error) => {
                let running = manager.status().running;
                (Err(error), running)
            }
        }
    };

    if restart_result.is_ok() {
        // Restart keeps the service running; preserve auto-restore marker.
        let mut settings = settings::load_settings_from_sqlite_state(&sqlite_state)?;
        if !settings.enabled_on_startup {
            settings.enabled_on_startup = true;
            if let Err(error) = settings::save_settings(&sqlite_state, settings) {
                log::warn!("Failed to persist proxy gateway startup state after restart: {error}");
            }
        }
    }

    if let Err(error) = app.emit("gateway-running-changed", running) {
        log::warn!("Failed to emit proxy gateway running status after restart: {error}");
    }

    restart_result
}

#[tauri::command]
pub fn proxy_gateway_status(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
) -> Result<ProxyGatewayStatus, String> {
    let manager = gateway_state
        .manager
        .lock()
        .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
    Ok(manager.status())
}

#[tauri::command]
pub async fn proxy_gateway_health_check(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
) -> Result<ProxyGatewayHealthCheckResult, String> {
    let addr = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        match manager.health_check_address() {
            Ok(addr) => addr,
            Err(result) => return Ok(result),
        }
    };
    Ok(crate::coding::proxy_gateway::runtime::health_check_socket_async(addr).await)
}

#[tauri::command]
pub fn proxy_gateway_check_port_available(
    input: ProxyGatewayPortCheckInput,
) -> Result<ProxyGatewayPortCheckResult, String> {
    check_port_available(input)
}

#[tauri::command]
pub async fn proxy_gateway_cli_statuses(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<Vec<GatewayCliTakeoverStatus>, String> {
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    Ok(cli_proxy::cli_takeover_statuses(db_state.db(), &paths, &status).await)
}

#[tauri::command]
pub async fn proxy_gateway_cli_status(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
) -> Result<GatewayCliTakeoverStatus, String> {
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    Ok(cli_proxy::cli_takeover_status(db_state.db(), &paths, cli_key, &status).await)
}

#[tauri::command]
pub async fn proxy_gateway_engage_single(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
    provider_id: String,
) -> Result<GatewayCliTakeoverStatus, String> {
    let _data_dir_transition = crate::app_paths::DATA_DIR_CHANGE_LOCK.lock().await;
    crate::app_paths::ensure_no_pending_data_dir_change()?;
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    let next_status =
        cli_proxy::engage_single_cli(db_state.db(), &paths, cli_key, &status, provider_id).await?;
    gateway_state.clear_provider_cache()?;
    emit_gateway_cli_wsl_sync_request(&app, cli_key);
    Ok(next_status)
}

#[tauri::command]
pub async fn proxy_gateway_engage_failover(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
) -> Result<GatewayCliTakeoverStatus, String> {
    let _data_dir_transition = crate::app_paths::DATA_DIR_CHANGE_LOCK.lock().await;
    crate::app_paths::ensure_no_pending_data_dir_change()?;
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    let next_status =
        cli_proxy::engage_failover_cli(db_state.db(), &paths, cli_key, &status).await?;
    gateway_state.clear_provider_cache()?;
    emit_gateway_cli_wsl_sync_request(&app, cli_key);
    Ok(next_status)
}

/// Engage aggregate mode for Codex: one model list across the selected sites.
///
/// `provider_ids` is the user's selected sites in display order; `separator` is
/// the string placed between the site id and the model name in the generated
/// catalog (default `.`).
#[tauri::command]
pub async fn proxy_gateway_engage_aggregate(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
    provider_ids: Vec<String>,
    separator: Option<String>,
    aliases: Option<BTreeMap<String, String>>,
    naming: Option<AggregateNamingMode>,
    cross_site_failover: Option<bool>,
    subagent_exposed_models: Option<Vec<String>>,
    subagent_model: Option<String>,
    subagent_reasoning_effort: Option<String>,
) -> Result<GatewayCliTakeoverStatus, String> {
    let _data_dir_transition = crate::app_paths::DATA_DIR_CHANGE_LOCK.lock().await;
    crate::app_paths::ensure_no_pending_data_dir_change()?;
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    let separator = separator
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| cli_proxy::manifest::AGGREGATE_DEFAULT_SEPARATOR.to_string());
    // Opt-in Codex `[agents]` defaults. Blank values mean "leave the user's
    // config alone", so an untouched form never writes these keys.
    let subagent_defaults =
        crate::coding::proxy_gateway::cli_proxy::manifest::AggregateSubagentDefaults {
            model: subagent_model,
            reasoning_effort: subagent_reasoning_effort,
        }
        .normalized();
    let next_status = cli_proxy::engage_aggregate_cli(
        db_state.db(),
        &paths,
        cli_key,
        &status,
        provider_ids,
        separator,
        aliases.unwrap_or_default(),
        naming.unwrap_or_default(),
        cross_site_failover.unwrap_or(false),
        subagent_exposed_models.unwrap_or_default(),
        subagent_defaults,
    )
    .await?;
    gateway_state.clear_provider_cache()?;
    emit_gateway_cli_wsl_sync_request(&app, cli_key);
    Ok(next_status)
}

/// Read the saved aggregate draft: the site selection the settings page shows
/// while aggregate mode is not engaged.
#[tauri::command]
pub async fn proxy_gateway_aggregate_draft(
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
) -> Result<Option<GatewayAggregateConfig>, String> {
    let paths = proxy_gateway_paths(&app)?;
    Ok(aggregate_draft::load_aggregate_draft(&paths, cli_key))
}

/// Persist the aggregate draft without engaging the mode.
///
/// This is not an engage command: the gateway does not have to be running and no
/// CLI runtime config is rewritten, so a user can prepare the site selection
/// first and enable it later.
#[tauri::command]
pub async fn proxy_gateway_save_aggregate_draft(
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
    provider_ids: Vec<String>,
    separator: Option<String>,
    aliases: Option<BTreeMap<String, String>>,
    naming: Option<AggregateNamingMode>,
    cross_site_failover: Option<bool>,
    subagent_exposed_models: Option<Vec<String>>,
) -> Result<GatewayAggregateConfig, String> {
    let paths = proxy_gateway_paths(&app)?;
    let separator =
        separator.unwrap_or_else(|| cli_proxy::manifest::AGGREGATE_DEFAULT_SEPARATOR.to_string());
    cli_proxy::save_aggregate_draft(
        db_state.db(),
        &paths,
        cli_key,
        GatewayAggregateConfig {
            provider_ids,
            separator,
            aliases: aliases.unwrap_or_default(),
            naming: naming.unwrap_or_default(),
            cross_site_failover: cross_site_failover.unwrap_or(false),
            subagent_exposed_models: subagent_exposed_models.unwrap_or_default(),
            // The draft stores the site selection; the `[agents]` defaults are
            // only written when the mode is actually engaged.
            subagent: None,
        },
    )
    .await
}

/// Read the aggregate catalog's programmable bare names for the settings drawer.
///
/// Read-only and local-only: it reads the manifest (or, when aggregate mode is
/// not engaged yet, the saved draft) plus the selected providers' declared
/// models, and the generated catalog file when one exists. It never contacts an
/// upstream endpoint and never writes.
///
/// `bare_models` is the full universe the selected sites declare, which is
/// deliberately *not* the same as `entries` (the bare names the catalog
/// currently publishes): narrowing the exposure set removes entries, and only
/// the universe keeps a removed name selectable again. The settings panel needs
/// this before the mode is engaged, because the exposure selection is part of
/// what the user has to make before it may engage at all.
///
#[tauri::command]
pub async fn proxy_gateway_subagent_catalog(
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
    provider_ids: Option<Vec<String>>,
) -> Result<GatewaySubagentCatalog, String> {
    let empty = |aggregate_mode: bool| GatewaySubagentCatalog {
        cli_key,
        aggregate_mode,
        entries: Vec::new(),
        bare_models: Vec::new(),
    };
    if cli_key != GatewayCliKey::Codex {
        return Ok(empty(false));
    }
    let paths = proxy_gateway_paths(&app)?;
    let manifest = cli_proxy::read_manifest_for_catalog(&paths, cli_key)?;
    let engaged = manifest
        .as_ref()
        .is_some_and(|manifest| manifest.enabled && manifest.mode == GatewayProxyMode::Aggregate);
    let active_provider_ids = manifest
        .as_ref()
        .and_then(|manifest| manifest.aggregate.as_ref())
        .map(|aggregate| aggregate.provider_ids.clone());
    // While aggregate mode is off the manifest may still carry a previous
    // selection (a disengage keeps the block), but a never-engaged install has
    // none. While engaged the manifest is authoritative. Otherwise use the
    // selection currently shown in the form when supplied, then the persisted
    // draft for older callers that only pass the CLI key.
    let provider_ids = if engaged {
        active_provider_ids.unwrap_or_default()
    } else if let Some(provider_ids) = provider_ids {
        provider_ids
    } else if let Some(draft) = aggregate_draft::load_aggregate_draft(&paths, cli_key) {
        draft.provider_ids
    } else {
        return Ok(empty(false));
    };
    let providers = super::runtime::load_candidate_providers(db_state.db(), cli_key).await?;
    // Universe: the models the selected sites declare, first site wins. This is
    // the same source the published catalog is built from, so the two can never
    // describe different name sets.
    let selected_sites = provider_ids
        .iter()
        .filter_map(|site_id| {
            providers
                .iter()
                .find(|provider| &provider.id == site_id)
                .map(|provider| (provider.id.clone(), provider.meta.declared_models.clone()))
        })
        .collect::<Vec<_>>();
    let universe = super::aggregate_naming::build_aggregate_bare_model_universe(&selected_sites);
    let describe = |site_id: &str, model: &str| GatewayAggregateBareModel {
        model: model.to_string(),
        provider_id: site_id.to_string(),
        provider_name: providers
            .iter()
            .find(|provider| provider.id == site_id)
            .map(|provider| provider.name.clone())
            .unwrap_or_default(),
    };
    // The bare names the generated catalog currently publishes, read from the
    // catalog file instead of re-derived: the exposure set (or a hand-edited
    // catalog) is what Codex actually sees. Names the universe does not know
    // are still reported, with no owning site.
    let published = read_generated_bare_model_names(db_state.db())
        .await
        .into_iter()
        .map(|model| {
            universe
                .iter()
                .find(|entry| entry.model == model)
                .map(|entry| describe(&entry.site_id, &entry.model))
                .unwrap_or(GatewayAggregateBareModel {
                    model,
                    provider_id: String::new(),
                    provider_name: String::new(),
                })
        })
        .collect::<Vec<_>>();
    // A legacy manifest whose selected sites cannot be resolved right now has no
    // universe; fall back to the published names so the drawer still lists what
    // can be selected instead of looking empty.
    let bare_models = if universe.is_empty() {
        published
            .iter()
            .map(|entry| GatewayAggregateBareModel {
                model: entry.model.clone(),
                provider_id: String::new(),
                provider_name: String::new(),
            })
            .collect::<Vec<_>>()
    } else {
        universe
            .iter()
            .map(|entry| describe(&entry.site_id, &entry.model))
            .collect::<Vec<_>>()
    };
    Ok(GatewaySubagentCatalog {
        cli_key,
        // Reports the *takeover* state, not whether a candidate list exists: the
        // panel shows its exposure editor before engaging and only wants to know
        // whether the described selection is the live one.
        aggregate_mode: engaged,
        entries: published,
        bare_models,
    })
}

/// Bare-name entries of the currently generated Codex catalog.
///
/// A bare name is addressable either way: the exposure set publishes the ticked
/// ones as promoted, picker-visible rows and the rest as `visibility = "hide"`
/// aliases. Both kinds name themselves (`display_name == slug`), while a
/// per-site row is always labelled `<site label> · <model>`. Relying on the
/// rows' own fields keeps this reader correct for every naming template,
/// including `model_only`, where a site slug and a bare name can be identical.
///
/// Best-effort: a missing or unreadable catalog file yields an empty list, so
/// browsing the drawer can never fail the settings page.
async fn read_generated_bare_model_names(db: &SqliteDbState) -> Vec<String> {
    use crate::coding::codex::constants::AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME;

    let Ok(config_dir) =
        crate::coding::codex::commands::get_codex_config_dir_from_db_async(db).await
    else {
        return Vec::new();
    };
    let Ok(content) =
        std::fs::read_to_string(config_dir.join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME))
    else {
        return Vec::new();
    };
    let Ok(catalog) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };
    catalog
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| {
            let slug = item.get("slug").and_then(Value::as_str);
            let display_name = item.get("display_name").and_then(Value::as_str);
            matches!((slug, display_name), (Some(slug), Some(name)) if slug == name)
        })
        .filter_map(|item| item.get("slug").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

#[tauri::command]
pub async fn proxy_gateway_disengage_failover(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
) -> Result<GatewayCliTakeoverStatus, String> {
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    let next_status =
        cli_proxy::disengage_failover_cli(db_state.db(), &paths, cli_key, &status).await?;
    gateway_state.clear_provider_cache()?;
    emit_gateway_cli_wsl_sync_request(&app, cli_key);
    Ok(next_status)
}

#[tauri::command]
pub async fn proxy_gateway_restore_cli_direct(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
) -> Result<GatewayCliTakeoverStatus, String> {
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    let next_status =
        cli_proxy::restore_cli_direct(db_state.db(), &paths, cli_key, &status).await?;
    gateway_state.clear_provider_cache()?;
    emit_gateway_cli_wsl_sync_request(&app, cli_key);
    Ok(next_status)
}

#[tauri::command]
pub async fn proxy_gateway_switch_primary_provider(
    app: tauri::AppHandle,
    cli_key: GatewayCliKey,
    provider_id: String,
) -> Result<GatewayCliTakeoverStatus, String> {
    provider_switch::apply_or_switch_provider(&app, cli_key, &provider_id, false).await
}

#[tauri::command]
pub async fn proxy_gateway_stop_preflight(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<ProxyGatewayStopPreflight, String> {
    let status = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.status()
    };
    let paths = proxy_gateway_paths(&app)?;
    Ok(cli_proxy::stop_preflight(db_state.db(), &paths, &status).await)
}

#[tauri::command]
pub fn proxy_gateway_request_logs(
    db_state: tauri::State<'_, SqliteDbState>,
    filters: Option<GatewayRequestLogFilters>,
    page: Option<u32>,
    page_size: Option<u32>,
    input: Option<ProxyGatewayRequestLogListInput>,
) -> Result<GatewayPaginatedRequestLogs, String> {
    let page_size = page_size
        .or_else(|| input.and_then(|input| input.limit.map(|limit| limit as u32)))
        .unwrap_or(20);
    usage_stats::request_logs(
        &db_state,
        &filters.unwrap_or_default(),
        page.unwrap_or(0),
        page_size,
        session_usage_enabled(&db_state)?,
    )
}

#[tauri::command]
pub async fn proxy_gateway_request_log_detail(
    app: tauri::AppHandle,
    db_state: tauri::State<'_, SqliteDbState>,
    trace_id: String,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    let db = db_state.inner().clone();
    // `app.path()` must run on the command thread; the file IO below is
    // offloaded to a blocking task so a huge residual JSONL scan can no longer
    // freeze the webview (issue #324).
    let paths = proxy_gateway_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        load_request_log_detail(&paths, &db, &trace_id).map(|opt| {
            opt.map(truncate_bodies_for_display)
                .map(sanitize_request_log_detail_for_display)
        })
    })
    .await
    .map_err(|join_error| format!("Gateway request detail load failed: {join_error}"))?
}

#[tauri::command]
pub async fn proxy_gateway_export_request_log_detail(
    app: tauri::AppHandle,
    db_state: tauri::State<'_, SqliteDbState>,
    trace_id: String,
    export_path: String,
) -> Result<(), String> {
    let db = db_state.inner().clone();
    let paths = proxy_gateway_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let detail = load_request_log_detail(&paths, &db, &trace_id)?
            .ok_or_else(|| "Gateway request detail not found".to_string())?;
        let export_json = build_request_log_detail_export(&detail);
        write_export_json(Path::new(&export_path), &export_json)
    })
    .await
    .map_err(|join_error| format!("Gateway request detail export failed: {join_error}"))?
}

fn load_request_log_detail(
    paths: &ProxyGatewayPaths,
    db_state: &SqliteDbState,
    trace_id: &str,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    if let Some((detail_file, detail_offset)) =
        usage_stats::request_log_location(db_state, trace_id)?
    {
        if let Some(detail) =
            request_log::get_request_log_detail_at(paths, &detail_file, detail_offset, trace_id)?
        {
            return Ok(Some(sanitize_request_log_detail_for_display(detail)));
        }
    }
    if let Some(detail) = request_log::get_request_log_detail(paths, trace_id)? {
        return Ok(Some(sanitize_request_log_detail_for_display(detail)));
    }
    usage_stats::request_log_detail_from_summary(db_state, trace_id)
        .map(|detail| detail.map(sanitize_request_log_detail_for_display))
}

fn sanitize_request_log_detail_for_display(
    mut detail: GatewayRequestLogDetail,
) -> GatewayRequestLogDetail {
    detail.summary.path = request_log::redact_request_path(&detail.summary.path);
    detail.summary.upstream_url = detail
        .summary
        .upstream_url
        .as_deref()
        .map(request_log::redact_request_path);
    detail
}

/// Maximum body bytes shipped across IPC to the webview for the detail dialog.
/// A single Codex turn can carry hundreds of KiB of request/response body;
/// rendering all of it (and `CollapsiblePre`'s line-count scan over it) can
/// peg the webview. Exports still carry the full bodies for diagnosis.
const MAX_DISPLAY_BODY_BYTES: usize = 256 * 1024;

/// Truncate each request/response body to [`MAX_DISPLAY_BODY_BYTES`] with a
/// trailing marker, only on the UI display path. The export path keeps full
/// bodies so maintainers can still diagnose "failed request shows consumption".
fn truncate_bodies_for_display(mut detail: GatewayRequestLogDetail) -> GatewayRequestLogDetail {
    detail.request_body = truncate_body_text(detail.request_body.take());
    detail.upstream_request_body = truncate_body_text(detail.upstream_request_body.take());
    detail.response_body = truncate_body_text(detail.response_body.take());
    detail.upstream_response_body = truncate_body_text(detail.upstream_response_body.take());
    detail
}

fn truncate_body_text(body: Option<String>) -> Option<String> {
    let text = body?;
    if text.len() <= MAX_DISPLAY_BODY_BYTES {
        return Some(text);
    }
    // Walk to the last char boundary at or below the byte cap so the cut never
    // splits a UTF-8 sequence.
    let cut = text
        .char_indices()
        .take_while(|(byte_index, _)| *byte_index < MAX_DISPLAY_BODY_BYTES)
        .last()
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(0);
    let mut truncated = String::with_capacity(cut + 128);
    truncated.push_str(&text[..cut]);
    truncated.push_str(&format!(
        "\n\n…[truncated: {} total bytes, {} shown — export the request for the full body]",
        text.len(),
        cut
    ));
    Some(truncated)
}

fn build_request_log_detail_export(detail: &GatewayRequestLogDetail) -> Value {
    let summary = &detail.summary;
    let requested_model = summary.requested_model.as_deref();
    let upstream_model = summary.upstream_model_id.as_deref();
    let mut summary_value = serde_json::to_value(summary).unwrap_or(Value::Null);
    let mut provider_attempts_value =
        serde_json::to_value(&detail.provider_attempts).unwrap_or(Value::Null);
    let mut websocket_value = serde_json::to_value(&detail.websocket).unwrap_or(Value::Null);
    redact_json_value(&mut summary_value);
    redact_json_value(&mut provider_attempts_value);
    redact_json_value(&mut websocket_value);
    serde_json::json!({
        "schema_version": 1,
        "exported_at": Utc::now().to_rfc3339(),
        "redaction": {
            "placeholder": "xxx",
            "note": "Authentication-like fields and header values are redacted during export."
        },
        "summary": summary_value,
        "provider_attempts": provider_attempts_value,
        "websocket": websocket_value,
        "privacy": detail.privacy,
        "request": {
            "headers": redact_header_map(detail.request_headers.as_ref()),
            "body_before_conversion": redact_body(detail.request_body.as_deref()),
            "body_after_conversion": redact_body(detail.upstream_request_body.as_deref())
        },
        "response": {
            "headers": redact_header_map(detail.response_headers.as_ref()),
            "body_before_conversion": redact_body(detail.upstream_response_body.as_deref()),
            "body_after_conversion": redact_body(detail.response_body.as_deref())
        },
        "routing": {
            "requested_model": requested_model,
            "upstream_model_id": upstream_model,
            "upstream_url": summary
                .upstream_url
                .as_deref()
                .map(request_log::redact_request_path)
                .and_then(|value| redact_text(Some(&value)))
        }
    })
}

fn write_export_json(export_path: &Path, export_json: &Value) -> Result<(), String> {
    if let Some(parent) = export_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create gateway request export directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let content = serde_json::to_string_pretty(export_json)
        .map_err(|error| format!("Failed to serialize gateway request export: {error}"))?;
    std::fs::write(export_path, format!("{content}\n")).map_err(|error| {
        format!(
            "Failed to write gateway request export {}: {error}",
            export_path.display()
        )
    })
}

fn redact_header_map(headers: Option<&BTreeMap<String, String>>) -> Value {
    match headers {
        Some(headers) => Value::Object(
            headers
                .iter()
                .map(|(name, value)| {
                    let redacted_value =
                        if request_log::is_sensitive_header(&name.to_ascii_lowercase()) {
                            Value::String("xxx".to_string())
                        } else {
                            Value::String(redact_text(Some(value)).unwrap_or_default())
                        };
                    (name.clone(), redacted_value)
                })
                .collect(),
        ),
        None => Value::Null,
    }
}

fn redact_body(body: Option<&str>) -> Value {
    let Some(body) = body else {
        return Value::Null;
    };
    match serde_json::from_str::<Value>(body) {
        Ok(mut value) => {
            redact_json_value(&mut value);
            value
        }
        Err(_) => Value::String(redact_text(Some(body)).unwrap_or_default()),
    }
}

fn redact_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, nested_value) in map.iter_mut() {
                if is_sensitive_key(key) {
                    *nested_value = Value::String("xxx".to_string());
                } else {
                    redact_json_value(nested_value);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_json_value(item);
            }
        }
        Value::String(text) => {
            *text = redact_auth_like_text(text);
        }
        _ => {}
    }
}

fn redact_text(text: Option<&str>) -> Option<String> {
    text.map(redact_auth_like_text)
}

fn redact_auth_like_text(text: &str) -> String {
    let mut redact_next = false;
    text.split_whitespace()
        .map(|token| {
            if redact_next {
                redact_next = false;
                return "xxx".to_string();
            }
            let lower = token.to_ascii_lowercase();
            if lower == "bearer" || lower == "basic" {
                redact_next = true;
                return token.to_string();
            }
            if lower.starts_with("sk-")
                || lower.starts_with("sk_")
                || lower.starts_with("xai-")
                || lower.starts_with("ghp_")
                || lower.starts_with("gho_")
                || lower.starts_with("ghu_")
                || lower.starts_with("ghs_")
                || lower.starts_with("ghr_")
                || lower.starts_with("github_pat_")
            {
                "xxx".to_string()
            } else {
                redact_sensitive_text_assignments(token)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_sensitive_text_assignments(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    let mut redacted = String::with_capacity(token.len());
    let mut search_start = 0;

    while let Some((key_start, value_start, value_end)) =
        find_next_sensitive_assignment(token, &lower, search_start)
    {
        redacted.push_str(&token[search_start..value_start]);
        redacted.push_str("xxx");
        search_start = value_end;
        if search_start <= key_start {
            break;
        }
    }

    if search_start == 0 {
        token.to_string()
    } else {
        redacted.push_str(&token[search_start..]);
        redacted
    }
}

fn find_next_sensitive_assignment(
    token: &str,
    lower: &str,
    start: usize,
) -> Option<(usize, usize, usize)> {
    const SENSITIVE_QUERY_KEYS: &[&str] = &[
        "key",
        "api_key",
        "api-key",
        "apikey",
        "x-api-key",
        "access_token",
        "access-token",
        "refresh_token",
        "refresh-token",
        "client_secret",
        "client-secret",
        "token",
    ];

    let mut best_match: Option<(usize, usize, usize)> = None;
    for key in SENSITIVE_QUERY_KEYS {
        let assignment = format!("{key}=");
        let mut search_start = start;
        while search_start < lower.len() {
            let Some(relative_index) = lower[search_start..].find(&assignment) else {
                break;
            };
            let key_start = search_start + relative_index;
            let value_start = key_start + assignment.len();
            if is_sensitive_assignment_boundary(lower, key_start) {
                let value_end = sensitive_assignment_value_end(token, value_start);
                if value_end > value_start
                    && best_match
                        .map(|(best_key_start, _, _)| key_start < best_key_start)
                        .unwrap_or(true)
                {
                    best_match = Some((key_start, value_start, value_end));
                }
                break;
            }
            search_start = key_start + 1;
        }
    }

    best_match
}

fn is_sensitive_assignment_boundary(lower: &str, key_start: usize) -> bool {
    if key_start == 0 {
        return true;
    }
    matches!(
        lower.as_bytes()[key_start - 1],
        b'?' | b'&' | b';' | b'"' | b'\'' | b'(' | b'[' | b'{'
    )
}

fn sensitive_assignment_value_end(token: &str, value_start: usize) -> usize {
    token[value_start..]
        .char_indices()
        .find_map(|(offset, character)| {
            matches!(character, '&' | ';' | '#').then_some(value_start + offset)
        })
        .unwrap_or(token.len())
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    let separator_normalized = normalized
        .chars()
        .map(|character| match character {
            '-' | ' ' => '_',
            _ => character,
        })
        .collect::<String>();
    request_log::is_sensitive_header(&normalized)
        || request_log::is_sensitive_header(&separator_normalized)
        || normalized.contains("secret")
        || separator_normalized.contains("secret")
        || normalized.contains("password")
        || separator_normalized.contains("password")
        || normalized.contains("credential")
        || separator_normalized.contains("credential")
        || normalized == "token"
        || separator_normalized == "token"
        || normalized.contains("access_token")
        || separator_normalized.contains("access_token")
        || normalized.contains("refresh_token")
        || separator_normalized.contains("refresh_token")
        || normalized == "key"
        || separator_normalized == "key"
        || normalized == "apikey"
        || separator_normalized == "api_key"
        || separator_normalized == "apikey"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::types::GatewayRequestLogSummary;

    #[test]
    fn export_redaction_redacts_json_auth_fields() {
        let redacted = redact_body(Some(
            r#"{"api_key":"sk-test","api-key":"hyphen","client-secret":"clientSecretValue","nested":{"authorization":"Bearer real-token","content":"keep"}}"#,
        ));

        assert_eq!(redacted["api_key"], "xxx");
        assert_eq!(redacted["api-key"], "xxx");
        assert_eq!(redacted["client-secret"], "xxx");
        assert_eq!(redacted["nested"]["authorization"], "xxx");
        assert_eq!(redacted["nested"]["content"], "keep");
    }

    #[test]
    fn export_redaction_preserves_token_count_fields() {
        // The gateway request-log export is how maintainers diagnose "failed
        // request shows consumption" reports (issue #318). The token-count
        // fields contain the substring "token" but are metrics, not secrets,
        // so they must survive redaction alongside a genuine `access_token`.
        let redacted = redact_body(Some(
            r#"{"input_tokens":120,"output_tokens":40,"cache_read_tokens":8,"cache_creation_tokens":2,"total_tokens":170,"first_token_ms":312,"access_token":"sk-secret","csrf_token":"csrf-secret"}"#,
        ));

        assert_eq!(redacted["input_tokens"], 120);
        assert_eq!(redacted["output_tokens"], 40);
        assert_eq!(redacted["cache_read_tokens"], 8);
        assert_eq!(redacted["cache_creation_tokens"], 2);
        assert_eq!(redacted["total_tokens"], 170);
        assert_eq!(redacted["first_token_ms"], 312);
        assert_eq!(redacted["access_token"], "xxx");
        assert_eq!(redacted["csrf_token"], "xxx");
    }

    #[test]
    fn truncate_body_text_keeps_small_bodies_intact() {
        assert_eq!(truncate_body_text(None), None);
        assert_eq!(
            truncate_body_text(Some("small body".to_string())),
            Some("small body".to_string())
        );
    }

    #[test]
    fn truncate_body_text_caps_oversized_body_at_char_boundary() {
        // A multi-megabyte body must be capped for the webview and carry a
        // truncation marker; the cut must land on a UTF-8 boundary so the
        // displayed prefix is not split mid-codepoint.
        let body = "中".repeat(MAX_DISPLAY_BODY_BYTES); // each char is 3 bytes
        let total_bytes = body.len();
        let truncated = truncate_body_text(Some(body)).unwrap();
        assert!(truncated.len() < total_bytes);
        assert!(truncated.contains("[truncated"));
        // The cap is a char boundary here, so the prefix is valid UTF-8.
        let prefix_end = truncated
            .find("\n\n…[truncated")
            .expect("truncation marker present");
        assert!(std::str::from_utf8(&truncated.as_bytes()[..prefix_end]).is_ok());
        assert!(prefix_end <= MAX_DISPLAY_BODY_BYTES);
    }

    #[test]
    fn export_redaction_redacts_auth_like_text() {
        let redacted = redact_auth_like_text(
            "Authorization: Bearer sk-test https://example.test/v1?api_key=secret&api-key=hyphen&client-secret=clientSecretValue&alt=sse keep",
        );

        assert!(redacted.contains("Bearer xxx"));
        assert!(redacted.contains("api_key=xxx&api-key=xxx&client-secret=xxx&alt=sse"));
        assert!(redacted.contains("keep"));
        assert!(!redacted.contains("sk-test"));
        assert!(!redacted.contains("api_key=secret"));
        assert!(!redacted.contains("api-key=hyphen"));
        assert!(!redacted.contains("clientSecretValue"));
    }

    #[test]
    fn export_redaction_redacts_gemini_key_query_param() {
        let redacted = redact_auth_like_text(
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent?key=AIzaSecret&alt=sse",
        );

        assert_eq!(
            redacted,
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent?key=xxx&alt=sse"
        );
        assert!(!redacted.contains("AIzaSecret"));
    }

    #[test]
    fn export_redaction_does_not_redact_non_sensitive_key_substrings() {
        let redacted = redact_auth_like_text("https://example.test/search?monkey=value&alt=sse");

        assert_eq!(redacted, "https://example.test/search?monkey=value&alt=sse");
    }

    fn request_detail_fixture() -> GatewayRequestLogDetail {
        let now = Utc::now();
        let detail = GatewayRequestLogDetail {
            privacy: None,
            websocket: None,
            summary: GatewayRequestLogSummary {
                transport: Default::default(),
                request_kind: Default::default(),
                usage_metadata: None,
                data_source: None,
                trace_id: "trace-redact-display".to_string(),
                started_at: now,
                ended_at: now,
                cli_key: Some(GatewayCliKey::Gemini.into()),
                route_name: "gemini".to_string(),
                method: "GET".to_string(),
                path: "/gemini/v1beta/models?key=secret&api%5Fkey=encoded&api-key=hyphen&client-secret=clientSecretValue&alt=sse".to_string(),
                provider_id: Some("provider-a".to_string()),
                provider_name: Some("Provider A".to_string()),
                provider_type: None,
                cost_multiplier: None,
                pricing_model_source: None,
                requested_model: Some("unknown".to_string()),
                upstream_model_id: Some("unknown".to_string()),
                reasoning_effort: None,
                upstream_url: Some(
                    "https://generativelanguage.googleapis.com/v1beta/models?key=secret&api-key=hyphen"
                        .to_string(),
                ),
                status_code: Some(200),
                upstream_status_code: None,
                success: true,
                error_category: None,
                error_message: None,
                stream_outcome: None,
                duration_ms: 1,
                attempt_count: 1,
                total_attempt_count: 1,
                failover: false,
                input_tokens: Some(0),
                output_tokens: Some(0),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                total_tokens: Some(0),
                request_body_bytes: 0,
                response_body_bytes: 0,
                is_streaming: false,
                first_token_ms: None,
                detail_file: None,
                detail_offset: None,
            },
            request_headers: None,
            request_body: None,
            upstream_request_body: None,
            response_headers: None,
            upstream_response_body: None,
            response_body: None,
            provider_attempts: Vec::new(),
        };

        detail
    }

    #[test]
    fn websocket_export_includes_transport_lifecycle_and_handshake_details() {
        use super::super::types::{
            GatewayRequestTransport, GatewayStreamOutcome, GatewayWebSocketMetadata,
        };
        let mut detail = request_detail_fixture();
        detail.summary.transport = GatewayRequestTransport::Websocket;
        detail.summary.status_code = None;
        detail.summary.stream_outcome = Some(GatewayStreamOutcome::Completed);
        detail.websocket = Some(GatewayWebSocketMetadata {
            connection_id: "connection-export".to_string(),
            response_id: Some("response-export".to_string()),
            previous_response_id: Some("previous-export".to_string()),
            stream_id: Some("lane-export".to_string()),
            handshake_status: 101,
            upstream_handshake_status: Some(101),
            ..Default::default()
        });
        let exported = build_request_log_detail_export(&detail);
        assert_eq!(exported["summary"]["transport"], "websocket");
        assert!(exported["summary"]["status_code"].is_null());
        assert_eq!(exported["summary"]["stream_outcome"], "completed");
        assert_eq!(exported["websocket"]["connection_id"], "connection-export");
        assert_eq!(exported["websocket"]["response_id"], "response-export");
        assert_eq!(
            exported["websocket"]["previous_response_id"],
            "previous-export"
        );
        assert_eq!(exported["websocket"]["stream_id"], "lane-export");
        assert_eq!(exported["websocket"]["handshake_status"], 101);
        assert!(!exported.to_string().contains("key=secret"));
    }

    #[test]
    fn privacy_export_keeps_the_recorded_log_redaction_marker() {
        let mut detail = request_detail_fixture();
        detail.privacy = Some(super::super::privacy::PrivacyDetail {
            matched_values: 1,
            restored_values: 2,
            log_redacted: true,
            ..Default::default()
        });
        let exported = build_request_log_detail_export(&detail);
        assert_eq!(exported["privacy"]["log_redacted"], true);
        assert_eq!(exported["privacy"]["matched_values"], 1);
        assert_eq!(exported["privacy"]["restored_values"], 2);
        assert!(exported["privacy"].get("mapping").is_none());
    }

    #[test]
    fn display_sanitization_redacts_path_and_upstream_url_queries() {
        let sanitized = sanitize_request_log_detail_for_display(request_detail_fixture());

        assert_eq!(
            sanitized.summary.path,
            "/gemini/v1beta/models?key=xxx&api%5Fkey=xxx&api-key=xxx&client-secret=xxx&alt=sse"
        );
        assert_eq!(
            sanitized.summary.upstream_url.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/models?key=xxx&api-key=xxx")
        );
        assert!(!sanitized.summary.path.contains("hyphen"));
        assert!(!sanitized.summary.path.contains("clientSecretValue"));
        assert!(!sanitized
            .summary
            .upstream_url
            .as_deref()
            .unwrap_or_default()
            .contains("hyphen"));
    }
}

#[tauri::command]
pub fn proxy_gateway_usage_summary(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<GatewayUsageSummary, String> {
    let include_session = session_usage_enabled(&db_state)?;
    usage_stats::usage_summary(&db_state, start_date, end_date, cli_key, include_session)
}

#[tauri::command]
pub fn proxy_gateway_usage_summary_by_cli(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
) -> Result<Vec<GatewayUsageSummaryByCli>, String> {
    let include_session = session_usage_enabled(&db_state)?;
    usage_stats::usage_summary_by_cli(&db_state, start_date, end_date, include_session)
}

#[tauri::command]
pub fn proxy_gateway_usage_trends(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<Vec<GatewayUsageTrendPoint>, String> {
    let include_session = session_usage_enabled(&db_state)?;
    usage_stats::usage_trends(&db_state, start_date, end_date, cli_key, include_session)
}

#[tauri::command]
pub fn proxy_gateway_provider_stats(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<Vec<GatewayProviderStats>, String> {
    let include_session = session_usage_enabled(&db_state)?;
    usage_stats::provider_stats(&db_state, start_date, end_date, cli_key, include_session)
}

#[tauri::command]
pub fn proxy_gateway_model_stats(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<Vec<GatewayModelStats>, String> {
    let include_session = session_usage_enabled(&db_state)?;
    usage_stats::model_stats(&db_state, start_date, end_date, cli_key, include_session)
}

#[tauri::command]
pub fn proxy_gateway_data_source_breakdown(
    db_state: tauri::State<'_, SqliteDbState>,
    input: Option<DataSourceBreakdownInput>,
) -> Result<Vec<DataSourceBreakdownItem>, String> {
    let include_session = session_usage_enabled(&db_state)?;
    usage_stats::data_source_breakdown(&db_state, input.unwrap_or_default(), include_session)
}

#[tauri::command]
pub async fn proxy_gateway_import_session_usage(
    app: tauri::AppHandle,
    db_state: tauri::State<'_, SqliteDbState>,
    input: GatewaySessionUsageImportInput,
) -> Result<GatewaySessionUsageImportResult, String> {
    // Gating by `session_usage_enabled` lives inside `import_session_usage`, so
    // the 60s background scheduler and this command share one authority.
    let result = session_import::import_session_usage(db_state.db().clone(), input).await?;
    session_import::notify_usage_changed(&app, &result);
    Ok(result)
}

/// Reads the local-session-usage display toggle from gateway settings.
fn session_usage_enabled(db_state: &SqliteDbState) -> Result<bool, String> {
    Ok(settings::load_settings_from_sqlite_state(db_state)?.session_usage_enabled)
}

#[tauri::command]
pub fn get_model_pricing_list(
    db_state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<ModelPricing>, String> {
    pricing::get_model_pricing_list(&db_state)
}

#[tauri::command]
pub fn upsert_model_pricing(
    db_state: tauri::State<'_, SqliteDbState>,
    pricing: ModelPricing,
) -> Result<ModelPricing, String> {
    pricing::upsert_model_pricing(&db_state, pricing)
}

#[tauri::command]
pub fn delete_model_pricing(
    db_state: tauri::State<'_, SqliteDbState>,
    model_id: String,
) -> Result<(), String> {
    pricing::delete_model_pricing(&db_state, model_id)
}

/// Sync the official price list. Sources are owned by the backend (primary plus
/// mirror fallback), so callers no longer pass a URL — which also removes a
/// webview-controlled arbitrary fetch.
#[tauri::command]
pub async fn fetch_remote_model_pricing(
    db_state: tauri::State<'_, SqliteDbState>,
) -> Result<model_pricing_seed::ModelPricingSyncResult, String> {
    model_pricing_seed::fetch_remote_model_pricing(&db_state).await
}

#[tauri::command]
pub async fn proxy_gateway_model_health_entries(
    app: tauri::AppHandle,
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    db_state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GatewayModelHealthItem>, String> {
    let paths = proxy_gateway_paths(&app)?;
    let settings = settings::load_settings_from_sqlite_state(&sqlite_state)?;
    let runtime_items = {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.model_health_items()
    };
    let mut items = match runtime_items {
        Some(items) => items,
        None => model_health::list_model_health_items(&paths.model_health_path(), settings)?,
    };
    match load_provider_name_map(&db_state.db()).await {
        Ok(provider_names) => {
            for item in &mut items {
                item.provider_name = provider_names
                    .get(&(item.cli_key, item.provider_id.clone()))
                    .cloned();
            }
        }
        Err(error) => {
            log::warn!("Failed to load proxy gateway provider name map: {error}");
        }
    }
    Ok(items)
}

#[tauri::command]
pub async fn proxy_gateway_test_provider_model_connectivity(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    db_state: tauri::State<'_, SqliteDbState>,
    request: GatewayConnectivityTestRequest,
) -> Result<GatewayConnectivityTestResponse, String> {
    {
        let manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        if !manager.status().running {
            return Err(
                "Gateway is not running. Start Gateway before testing protocol-converted providers."
                    .to_string(),
            );
        }
    }

    let settings = settings::load_settings_from_sqlite_state(&db_state)?;
    super::runtime::test_gateway_provider_model_connectivity(
        settings,
        db_state.db().clone(),
        request,
    )
    .await
}

fn proxy_gateway_paths(app: &tauri::AppHandle) -> Result<ProxyGatewayPaths, String> {
    let app_data_dir = crate::app_paths::resolved_data_dir();
    let _ = app; // data dir is resolved from the bootstrap override cache
    Ok(ProxyGatewayPaths::new(app_data_dir))
}

fn emit_gateway_cli_wsl_sync_request(app: &tauri::AppHandle, cli_key: GatewayCliKey) {
    let event_name = match cli_key {
        GatewayCliKey::Claude => "wsl-sync-request-claude",
        GatewayCliKey::ClaudeDesktop => "wsl-sync-request-claudedesktop",
        GatewayCliKey::Codex => "wsl-sync-request-codex",
        GatewayCliKey::Grok => "wsl-sync-request-grok",
        GatewayCliKey::Kimi => "wsl-sync-request-kimi",
        GatewayCliKey::Gemini => "wsl-sync-request-geminicli",
        GatewayCliKey::OpenCode => return,
    };
    if let Err(error) = app.emit(event_name, ()) {
        log::warn!("Failed to emit {event_name} after gateway CLI config change: {error}");
    }
}

async fn load_provider_name_map(
    db: &SqliteDbState,
) -> Result<HashMap<(GatewayCliKey, String), String>, String> {
    let mut provider_names = HashMap::new();
    for (cli_key, table) in [
        (GatewayCliKey::Claude, DbTable::ClaudeProvider),
        (GatewayCliKey::ClaudeDesktop, DbTable::ClaudeDesktopProvider),
        (GatewayCliKey::Codex, DbTable::CodexProvider),
        (GatewayCliKey::Grok, DbTable::GrokProvider),
        (GatewayCliKey::Kimi, DbTable::KimiProvider),
        (GatewayCliKey::Gemini, DbTable::GeminiCliProvider),
    ] {
        let order = OrderSpec::single(OrderField::id(OrderDirection::Asc));
        let records = db.with_conn(|conn| db_list(conn, table, Some(&order)))?;
        for record in records {
            let id = record
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = record
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if !id.is_empty() {
                if let Some(name) = name {
                    provider_names.insert((cli_key, id), name);
                }
            }
        }
    }
    let order = OrderSpec::single(OrderField::id(OrderDirection::Asc));
    let records =
        db.with_conn(|conn| db_list(conn, DbTable::OpenCodeFavoriteProvider, Some(&order)))?;
    for record in records {
        let Some(provider_id) = record.get("provider_id").and_then(Value::as_str) else {
            continue;
        };
        let name = record
            .get("provider_config")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(name) = name {
            provider_names.insert((GatewayCliKey::OpenCode, provider_id.to_string()), name);
        }
    }
    Ok(provider_names)
}
