//! Tauri commands for the gateway plugin system.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use crate::db::SqliteDbState;

use super::bun_host::auth::{self, PluginAccountSummary};
use super::bun_host::{BunHostStatus, RuntimeLogEntry};
use super::bun_runtime::{BunPathProbe, BunRuntimeStatus};
use super::installer;
use super::market;
use super::middleware_rt::stats::MiddlewareStateView;
use super::page_bridge;
use super::page_storage;
use super::store;
use super::types::{
    empty_json_object, InstalledPluginRecord, MarketPluginListing, MarketSourceRecord,
    PluginAccountMetaRecord, PluginPageSession, PluginProviderOptionValue, PluginSettings,
    RegisteredPluginPage,
};
use super::updater::{self, PluginUpdateSummary};
use super::PluginManagerState;

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

fn provider_options_value(db: &SqliteDbState) -> Result<Value, String> {
    let mut map = serde_json::Map::new();
    for option in store::list_provider_options(db)? {
        map.insert(option.provider_id, option.options);
    }
    Ok(Value::Object(map))
}

/// Merge each provider's persisted option values into the host listing, so the
/// UI restores its switches after a page reload instead of assuming defaults.
fn attach_provider_option_values(db: &SqliteDbState, listing: Value) -> Result<Value, String> {
    let saved = store::list_provider_options(db)?
        .into_iter()
        .map(|option| (option.provider_id, option.options))
        .collect::<HashMap<_, _>>();
    let Some(rows) = listing.as_array() else {
        return Ok(listing);
    };
    let rows = rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            if let Some(object) = row.as_object_mut() {
                let provider_id = object
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let values = saved
                    .get(&provider_id)
                    .cloned()
                    .unwrap_or_else(empty_json_object);
                object.insert("optionValues".to_string(), values);
            }
            row
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(rows))
}

/// Attach the effective channel-level auto check-in switch to each provider
/// row. Resolution includes the legacy account-level fallback, so the UI can
/// show one switch per channel without a second request.
fn attach_provider_auto_checkin(db: &SqliteDbState, listing: Value) -> Result<Value, String> {
    let switches = store::resolve_provider_auto_checkin(db)?;
    let Some(rows) = listing.as_array() else {
        return Ok(listing);
    };
    let rows = rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            if let Some(object) = row.as_object_mut() {
                let provider_id = object.get("id").and_then(Value::as_str).unwrap_or_default();
                let enabled = switches.get(provider_id).copied().unwrap_or(false);
                object.insert("autoCheckin".to_string(), Value::Bool(enabled));
            }
            row
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(rows))
}

fn installed_by_package(
    db: &SqliteDbState,
) -> Result<HashMap<String, InstalledPluginRecord>, String> {
    Ok(store::list_installed_plugins(db)?
        .into_iter()
        .filter(|record| !record.package_name.is_empty())
        .map(|record| (record.package_name.clone(), record))
        .collect())
}

pub(crate) async fn reload_runtimes(manager: &PluginManagerState) {
    manager.gateway().clear_provider_cache();
    manager.middleware().reload().await;
    if let Err(error) = manager.page_server().sync_enabled_plugins(manager.db()) {
        log::warn!("revoking plugin page tokens after a plugin change failed: {error}");
    }
    if let Err(error) = manager.host().notify_plugins_changed().await {
        log::warn!("plugin host restart after a plugin change failed: {error}");
    }
}

/// Atomic reload shared by the desktop command and the control API.
///
/// Rescans installed manifests, recompiles middleware, re-syncs page tokens,
/// restarts the Bun host when provider plugins are present and finally emits
/// one `plugins-changed` event so open desktop pages refresh without a restart.
pub async fn reload_plugins(
    manager: &PluginManagerState,
    app: &AppHandle,
    id: Option<&str>,
) -> Result<Vec<InstalledPluginRecord>, String> {
    let _update_guard = manager.update_lock.lock().await;
    if let Some(id) = id {
        if store::get_installed_plugin(manager.db(), id)?.is_none() {
            return Err(format!("Plugin '{id}' is not installed"));
        }
    }
    let records = installer::refresh_installed_plugins(manager.db(), manager.paths())?;
    reload_runtimes(manager).await;
    let _ = app.emit("plugins-changed", json!({ "id": id }));
    Ok(records)
}

#[tauri::command]
pub async fn plugins_reload(
    manager: State<'_, PluginManagerState>,
    app: AppHandle,
    id: Option<String>,
) -> Result<Vec<InstalledPluginRecord>, String> {
    let manager = manager.inner().clone();
    reload_plugins(&manager, &app, id.as_deref()).await
}

/// Apply updates and notify open desktop pages. Shared by the Tauri command
/// and the Control API so both paths emit the same event.
pub async fn update_plugins(
    manager: &PluginManagerState,
    app: &AppHandle,
    plugin_id: Option<&str>,
) -> Result<PluginUpdateSummary, String> {
    let summary = updater::check_and_update_plugins(manager, plugin_id, true, true).await?;
    if summary.updated_count > 0 {
        let updated_ids: Vec<&str> = summary
            .items
            .iter()
            .filter(|item| item.status == "updated")
            .map(|item| item.plugin_id.as_str())
            .collect();
        let _ = app.emit(
            "plugins-updated",
            json!({
                "updatedCount": summary.updated_count,
                "updatedIds": updated_ids,
            }),
        );
    }
    Ok(summary)
}

#[tauri::command]
pub async fn plugins_check_updates(
    manager: State<'_, PluginManagerState>,
    plugin_id: Option<String>,
    record_check_time: Option<bool>,
) -> Result<PluginUpdateSummary, String> {
    let manager = manager.inner().clone();
    if record_check_time.unwrap_or(true) {
        updater::check_and_update_plugins(&manager, plugin_id.as_deref(), false, true).await
    } else {
        updater::check_plugins_read_only(&manager, plugin_id.as_deref()).await
    }
}

#[tauri::command]
pub async fn plugins_update_plugins(
    manager: State<'_, PluginManagerState>,
    app: AppHandle,
    plugin_id: Option<String>,
) -> Result<PluginUpdateSummary, String> {
    let manager = manager.inner().clone();
    update_plugins(&manager, &app, plugin_id.as_deref()).await
}

#[tauri::command]
pub fn plugins_list_installed(
    manager: State<'_, PluginManagerState>,
) -> Result<Vec<InstalledPluginRecord>, String> {
    installer::refresh_installed_plugins(manager.db(), manager.paths())
}

#[tauri::command]
pub async fn plugins_install(
    manager: State<'_, PluginManagerState>,
    spec: String,
    market_id: Option<String>,
) -> Result<InstalledPluginRecord, String> {
    let manager = manager.inner().clone();
    let _update_guard = manager.update_lock.lock().await;
    let record = installer::install_plugin(
        manager.db(),
        manager.paths(),
        manager.bun(),
        &spec,
        market_id,
    )
    .await?;
    reload_runtimes(&manager).await;
    Ok(record)
}

#[tauri::command]
pub async fn plugins_uninstall(
    manager: State<'_, PluginManagerState>,
    id: String,
    purge_data: Option<bool>,
) -> Result<(), String> {
    let manager = manager.inner().clone();
    let _update_guard = manager.update_lock.lock().await;
    installer::uninstall_plugin(manager.db(), manager.paths(), manager.bun(), &id).await?;
    let purge_result = if purge_data.unwrap_or(false) {
        // The record is gone already; a stale data folder is harmless, a
        // failed purge must still be reported — after the runtimes have been
        // reloaded, so the plugin can never keep serving with revoked tokens.
        page_storage::purge_plugin_data(manager.paths(), &id)
    } else {
        Ok(())
    };
    reload_runtimes(&manager).await;
    purge_result
}

#[tauri::command]
pub async fn plugins_set_enabled(
    manager: State<'_, PluginManagerState>,
    id: String,
    enabled: bool,
) -> Result<InstalledPluginRecord, String> {
    let manager = manager.inner().clone();
    let _update_guard = manager.update_lock.lock().await;
    let record = store::set_plugin_enabled(manager.db(), &id, enabled)?;
    reload_runtimes(&manager).await;
    Ok(record)
}

#[tauri::command]
pub async fn plugins_set_options(
    manager: State<'_, PluginManagerState>,
    id: String,
    options: Value,
) -> Result<InstalledPluginRecord, String> {
    let manager = manager.inner().clone();
    let _update_guard = manager.update_lock.lock().await;
    let record = store::set_plugin_options(manager.db(), &id, &options)?;
    reload_runtimes(&manager).await;
    Ok(record)
}

// ---- custom pages ----------------------------------------------------------

#[tauri::command]
pub fn plugins_list_pages(
    manager: State<'_, PluginManagerState>,
) -> Result<Vec<RegisteredPluginPage>, String> {
    page_bridge::list_registered_pages(manager.db(), manager.paths())
}

#[tauri::command]
pub async fn plugins_open_page(
    manager: State<'_, PluginManagerState>,
    plugin_id: String,
    page_id: Option<String>,
) -> Result<PluginPageSession, String> {
    let manager = manager.inner().clone();
    page_bridge::open_page(&manager, &plugin_id, page_id).await
}

#[tauri::command]
pub fn plugins_page_bridge_read(
    manager: State<'_, PluginManagerState>,
    plugin_id: String,
    scope: String,
    params: Option<Value>,
) -> Result<Value, String> {
    page_bridge::read_scope(
        manager.db(),
        manager.inner(),
        &plugin_id,
        &scope,
        params.as_ref(),
    )
}

#[tauri::command]
pub async fn plugins_page_bridge_set_options(
    manager: State<'_, PluginManagerState>,
    plugin_id: String,
    options: Value,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    let _update_guard = manager.update_lock.lock().await;
    let page_only = store::get_installed_plugin(manager.db(), &plugin_id)?
        .is_some_and(|record| record.is_page_only());
    let saved = page_bridge::set_page_options(manager.db(), &plugin_id, &options)?;
    if page_only {
        // A pure page plugin owns nothing in the Bun host; restarting it would
        // only churn a runtime the page never talks to.
        manager.gateway().clear_provider_cache();
        if let Err(error) = manager.page_server().sync_enabled_plugins(manager.db()) {
            log::warn!("revoking plugin page tokens after an option change failed: {error}");
        }
    } else {
        reload_runtimes(&manager).await;
    }
    Ok(saved)
}

#[tauri::command]
pub async fn plugins_close_page(
    manager: State<'_, PluginManagerState>,
    plugin_id: String,
    page_id: String,
) -> Result<(), String> {
    manager
        .page_server()
        .revoke_plugin_page(&plugin_id, &page_id);
    Ok(())
}

#[tauri::command]
pub async fn plugins_page_bridge_copy_image(data_base64: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || page_bridge::copy_image_base64(&data_base64))
        .await
        .map_err(|error| format!("Clipboard task failed: {error}"))?
}

#[tauri::command]
pub async fn plugins_page_bridge_save_image(
    data_base64: String,
    filename: Option<String>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        page_bridge::save_image_base64(&data_base64, filename.as_deref())
    })
    .await
    .map_err(|error| format!("Image save task failed: {error}"))?
}

#[tauri::command]
pub fn plugins_page_bridge_kv(
    manager: State<'_, PluginManagerState>,
    plugin_id: String,
    op: String,
    key: Option<String>,
    value: Option<Value>,
    prefix: Option<String>,
) -> Result<Value, String> {
    page_storage::kv_op(
        manager.db(),
        manager.paths(),
        &plugin_id,
        &op,
        key,
        value,
        prefix,
    )
}

#[tauri::command]
pub async fn plugins_page_bridge_files(
    manager: State<'_, PluginManagerState>,
    plugin_id: String,
    op: String,
    path: Option<String>,
    content: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    tokio::task::spawn_blocking(move || {
        page_storage::files_op(
            manager.db(),
            manager.paths(),
            &plugin_id,
            &op,
            path,
            content,
        )
    })
    .await
    .map_err(|error| format!("Plugin file task failed: {error}"))?
}

#[tauri::command]
pub fn plugins_list_markets(
    manager: State<'_, PluginManagerState>,
) -> Result<Vec<MarketSourceRecord>, String> {
    store::list_market_sources(manager.db())
}

#[tauri::command]
pub async fn plugins_add_market(
    manager: State<'_, PluginManagerState>,
    name: String,
    url: String,
) -> Result<MarketSourceRecord, String> {
    let manager = manager.inner().clone();
    let url = market::normalize_market_url(&url)?;
    let client = manager.http_client().await?;
    let listings = market::fetch_registry(&client, &url).await?;

    let id = uuid::Uuid::new_v4().simple().to_string();
    market::write_market_cache(manager.paths(), &id, &listings)?;

    let name = name.trim();
    let name = if name.is_empty() {
        url::Url::parse(&url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_else(|| "Custom market".to_string())
    } else {
        name.to_string()
    };

    let record = MarketSourceRecord {
        id,
        name,
        url,
        enabled: true,
        builtin: false,
        plugin_count: listings.len(),
        last_synced_at: Some(now_rfc3339()),
        last_error: None,
    };
    store::upsert_market_source(manager.db(), &record)
}

#[tauri::command]
pub async fn plugins_update_market(
    manager: State<'_, PluginManagerState>,
    id: String,
    name: Option<String>,
    url: Option<String>,
    enabled: Option<bool>,
) -> Result<MarketSourceRecord, String> {
    let manager = manager.inner().clone();
    let db = manager.db();
    let mut record = store::get_market_source(db, &id)?
        .ok_or_else(|| format!("Market source '{id}' does not exist"))?;

    if let Some(url) = url.as_deref().map(str::trim).filter(|url| !url.is_empty()) {
        let url = market::normalize_market_url(url)?;
        if url != record.url {
            let client = manager.http_client().await?;
            let listings = market::fetch_registry(&client, &url).await?;
            market::write_market_cache(manager.paths(), &record.id, &listings)?;
            record.url = url;
            record.plugin_count = listings.len();
            record.last_synced_at = Some(now_rfc3339());
            record.last_error = None;
        }
    }
    if let Some(name) = name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        record.name = name.to_string();
    }
    if let Some(enabled) = enabled {
        record.enabled = enabled;
    }

    store::upsert_market_source(db, &record)
}

#[tauri::command]
pub fn plugins_remove_market(
    manager: State<'_, PluginManagerState>,
    id: String,
) -> Result<bool, String> {
    store::delete_market_source(manager.db(), &id)
}

#[tauri::command]
pub async fn plugins_refresh_markets(
    manager: State<'_, PluginManagerState>,
) -> Result<Vec<MarketSourceRecord>, String> {
    let manager = manager.inner().clone();
    let db = manager.db();
    let client = manager.http_client().await?;

    let mut refreshed = Vec::new();
    for mut source in store::list_market_sources(db)? {
        if !source.enabled {
            refreshed.push(source);
            continue;
        }
        let outcome = market::refresh_market_source(&client, manager.paths(), &source).await;
        source.plugin_count = outcome.listings.len();
        source.last_synced_at = Some(now_rfc3339());
        source.last_error = outcome.last_error;
        refreshed.push(store::upsert_market_source(db, &source)?);
    }
    Ok(refreshed)
}

#[tauri::command]
pub async fn plugins_market_listings(
    manager: State<'_, PluginManagerState>,
    search: Option<String>,
    kind: Option<String>,
    market_id: Option<String>,
    include_npm_search: Option<bool>,
) -> Result<Vec<MarketPluginListing>, String> {
    let manager = manager.inner().clone();
    let db = manager.db();
    let settings = store::get_plugin_settings(db)?;
    let client = manager.http_client().await?;

    let selected: Vec<MarketSourceRecord> = store::list_market_sources(db)?
        .into_iter()
        .filter(|source| source.enabled)
        .filter(|source| {
            market_id
                .as_deref()
                .map(|wanted| source.id == wanted)
                .unwrap_or(true)
        })
        .collect();

    let mut markets: Vec<(String, Vec<market::MarketListing>)> = Vec::new();
    for mut source in selected {
        let listings = match market::read_market_cache(manager.paths(), &source.id) {
            Some(cached) => cached,
            None => {
                let outcome =
                    market::refresh_market_source(&client, manager.paths(), &source).await;
                source.plugin_count = outcome.listings.len();
                source.last_synced_at = Some(now_rfc3339());
                source.last_error = outcome.last_error;
                store::upsert_market_source(db, &source)?;
                outcome.listings
            }
        };
        markets.push((source.id, listings));
    }

    let installed = installed_by_package(db)?;
    let merged = market::merge_market_listings(&markets, &installed);
    let mut listings = market::filter_market_listings(merged, search.as_deref(), kind.as_deref());

    market::enrich_listings_with_npm_info(
        &client,
        &settings.npm_registry,
        &manager.paths().npm_cache_path(),
        &mut listings,
    )
    .await;

    if include_npm_search.unwrap_or(false) {
        if let Some(query) = search
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
        {
            match market::search_npm(&client, &settings.npm_registry, query).await {
                Ok(hits) => {
                    let hits = market::filter_market_listings(hits, None, kind.as_deref());
                    let mut known: HashSet<String> = listings
                        .iter()
                        .map(|listing| listing.package.clone())
                        .collect();
                    for mut hit in hits {
                        if !known.insert(hit.package.clone()) {
                            continue;
                        }
                        if let Some(record) = installed.get(&hit.package) {
                            hit.installed = true;
                            hit.installed_version = Some(record.version.clone());
                        }
                        listings.push(hit);
                    }
                }
                Err(error) => {
                    log::warn!("npm plugin search failed: {error}");
                }
            }
        }
    }

    Ok(listings)
}

#[tauri::command]
pub fn plugins_get_settings(
    manager: State<'_, PluginManagerState>,
) -> Result<PluginSettings, String> {
    store::get_plugin_settings(manager.db())
}

#[tauri::command]
pub fn plugins_save_settings(
    manager: State<'_, PluginManagerState>,
    settings: PluginSettings,
) -> Result<PluginSettings, String> {
    let saved = store::save_plugin_settings(manager.db(), &settings)?;
    // The executable may have moved; make the next status re-probe it.
    manager.bun().invalidate();
    Ok(saved)
}

// ---- Bun runtime -----------------------------------------------------------

#[tauri::command]
pub async fn plugins_bun_status(
    manager: State<'_, PluginManagerState>,
) -> Result<BunRuntimeStatus, String> {
    let manager = manager.inner().clone();
    Ok(manager.bun().status().await)
}

#[tauri::command]
pub async fn plugins_bun_ensure(
    manager: State<'_, PluginManagerState>,
) -> Result<BunRuntimeStatus, String> {
    let manager = manager.inner().clone();
    manager.bun().ensure().await?;
    Ok(manager.bun().status().await)
}

#[tauri::command]
pub async fn plugins_bun_probe_path(
    manager: State<'_, PluginManagerState>,
    path: String,
) -> Result<BunPathProbe, String> {
    let manager = manager.inner().clone();
    Ok(manager.bun().probe_path(&path).await)
}

// ---- Bun host --------------------------------------------------------------

#[tauri::command]
pub async fn plugins_host_status(
    manager: State<'_, PluginManagerState>,
) -> Result<BunHostStatus, String> {
    let manager = manager.inner().clone();
    Ok(manager.host().status().await)
}

#[tauri::command]
pub async fn plugins_host_restart(
    manager: State<'_, PluginManagerState>,
) -> Result<BunHostStatus, String> {
    let manager = manager.inner().clone();
    manager.host().restart().await?;
    Ok(manager.host().status().await)
}

#[tauri::command]
pub fn plugins_runtime_logs(
    manager: State<'_, PluginManagerState>,
    limit: Option<usize>,
) -> Result<Vec<RuntimeLogEntry>, String> {
    Ok(manager.host().logs(limit.unwrap_or(200)))
}

#[tauri::command]
pub fn plugins_clear_runtime_logs(manager: State<'_, PluginManagerState>) -> Result<(), String> {
    manager.host().clear_logs();
    Ok(())
}

#[tauri::command]
pub fn plugins_middleware_states(
    manager: State<'_, PluginManagerState>,
) -> Result<Vec<MiddlewareStateView>, String> {
    Ok(manager.middleware().states())
}

// ---- provider plugins ------------------------------------------------------

#[tauri::command]
pub async fn plugins_list_providers(
    manager: State<'_, PluginManagerState>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    let listing = manager.host().providers(json!({})).await?;
    let listing = attach_provider_option_values(manager.db(), listing)?;
    attach_provider_auto_checkin(manager.db(), listing)
}

#[tauri::command]
pub async fn plugins_refresh_provider_models(
    manager: State<'_, PluginManagerState>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    // `setProviderOptions` is the host's only cache-invalidating call; with
    // the current options it just drops every cached models list, so the
    // following `providers` reaches each plugin again.
    let _ = manager
        .host()
        .set_provider_options(provider_options_value(manager.db())?)
        .await;
    let listing = manager.host().providers(json!({})).await?;
    let listing = attach_provider_option_values(manager.db(), listing)?;
    attach_provider_auto_checkin(manager.db(), listing)
}

#[tauri::command]
pub async fn plugins_login_prompt(
    manager: State<'_, PluginManagerState>,
    provider: String,
    method: Option<String>,
    inputs: Option<Value>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    manager
        .host()
        .login_prompt(json!({
            "provider": provider,
            "method": method,
            "inputs": inputs.unwrap_or_else(empty_json_object),
        }))
        .await
}

#[tauri::command]
pub async fn plugins_login_validate(
    manager: State<'_, PluginManagerState>,
    provider: String,
    method: Option<String>,
    key: Option<String>,
    value: Option<Value>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    manager
        .host()
        .validate(json!({
            "provider": provider,
            "method": method,
            "key": key,
            "value": value.unwrap_or(Value::Null),
        }))
        .await
}

#[tauri::command]
pub async fn plugins_login_authorize(
    manager: State<'_, PluginManagerState>,
    provider: String,
    method: Option<String>,
    inputs: Option<Value>,
    account: Option<String>,
    proxy: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    manager
        .host()
        .authorize(json!({
            "provider": provider,
            "method": method,
            "inputs": inputs.unwrap_or_else(empty_json_object),
            "account": account,
            "proxy": proxy,
        }))
        .await
}

#[tauri::command]
pub async fn plugins_login_callback(
    manager: State<'_, PluginManagerState>,
    session: String,
    code: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    manager
        .host()
        .callback(json!({ "session": session, "code": code }))
        .await
}

#[tauri::command]
pub async fn plugins_login_api_key(
    manager: State<'_, PluginManagerState>,
    provider: String,
    method: Option<String>,
    inputs: Option<Value>,
    key: Option<String>,
    account: Option<String>,
    proxy: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    manager
        .host()
        .api_key(json!({
            "provider": provider,
            "method": method,
            "inputs": inputs.unwrap_or_else(empty_json_object),
            "key": key,
            "account": account,
            "proxy": proxy,
        }))
        .await
}

#[tauri::command]
pub async fn plugins_login_import_files(
    manager: State<'_, PluginManagerState>,
    provider: String,
    texts: Value,
    proxy: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    manager
        .host()
        .import_files(json!({ "provider": provider, "texts": texts, "proxy": proxy }))
        .await
}

#[tauri::command]
pub async fn plugins_login_cancel(manager: State<'_, PluginManagerState>) -> Result<usize, String> {
    let manager = manager.inner().clone();
    Ok(manager.host().abort_signin().await)
}

#[tauri::command]
pub async fn plugins_list_accounts(
    manager: State<'_, PluginManagerState>,
) -> Result<Vec<PluginAccountSummary>, String> {
    let manager = manager.inner().clone();
    tokio::task::spawn_blocking(move || auth::list_accounts(manager.db(), manager.paths()))
        .await
        .map_err(|error| format!("Plugin account task failed: {error}"))?
}

#[tauri::command]
pub async fn plugins_account_sign_out(
    manager: State<'_, PluginManagerState>,
    provider: String,
    account: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    manager
        .host()
        .sign_out(json!({ "provider": provider, "account": account }))
        .await
}

/// `host.js`'s `usageOf` normalizes an empty plugin reply into a fixed object
/// with every field present but empty (`{"plan":"","until":"",…,"windows":[]}`),
/// so `object.is_empty()` never fires. A snapshot is only worth persisting when
/// at least one field carries real content; `error` alone is not a snapshot.
fn usage_snapshot_has_meaningful_data(object: &serde_json::Map<String, Value>) -> bool {
    object.iter().any(|(key, value)| {
        if key == "error" {
            return false;
        }
        match value {
            Value::String(text) => !text.trim().is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(map) => !map.is_empty(),
            Value::Number(_) | Value::Bool(_) => true,
            Value::Null => false,
        }
    })
}

fn record_usage_snapshot(
    db: &SqliteDbState,
    provider: &str,
    account: Option<&str>,
    usage: &Value,
) -> Result<(), String> {
    let Some(object) = usage.as_object() else {
        return Ok(());
    };
    let has_error = object
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|message| !message.trim().is_empty());
    if has_error || !usage_snapshot_has_meaningful_data(object) {
        return Ok(());
    }
    let account_key = account
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(provider);
    let mut record = super::background_tasks::find_account_meta(db, account_key, provider)?;
    if record.provider_id.trim().is_empty() {
        record.provider_id = provider.to_string();
    }
    record.quota = usage.clone();
    record.quota_updated_at = Some(now_rfc3339());
    store::upsert_account_meta(db, &record)?;
    Ok(())
}

#[tauri::command]
pub async fn plugins_account_check(
    manager: State<'_, PluginManagerState>,
    provider: String,
    account: Option<String>,
    proxy: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    let result = manager
        .host()
        .check(json!({ "provider": provider, "account": account, "proxy": proxy }))
        .await?;
    if let Some(usage) = result.get("usage") {
        if let Err(error) = record_usage_snapshot(manager.db(), &provider, account.as_deref(), usage)
        {
            log::warn!("saving the usage snapshot of {provider} failed: {error}");
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn plugins_account_usage(
    manager: State<'_, PluginManagerState>,
    provider: String,
    account: Option<String>,
    proxy: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    let result = manager
        .host()
        .usage(json!({ "provider": provider, "account": account, "proxy": proxy }))
        .await?;
    if let Err(error) = record_usage_snapshot(manager.db(), &provider, account.as_deref(), &result)
    {
        log::warn!("saving the usage snapshot of {provider} failed: {error}");
    }
    Ok(result)
}

#[tauri::command]
pub async fn plugins_account_checkin(
    manager: State<'_, PluginManagerState>,
    provider: String,
    account: Option<String>,
    proxy: Option<String>,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    let result = manager
        .host()
        .checkin(json!({ "provider": provider, "account": account, "proxy": proxy }))
        .await?;
    // A manual press also updates the account's card and the daily automatic
    // pass's bookkeeping, so it never asks the vendor twice the same day.
    if let Err(error) = super::background_tasks::record_checkin_result(
        manager.db(),
        &provider,
        account.as_deref(),
        &result,
    ) {
        log::warn!("saving the check-in result of {provider} failed: {error}");
    }
    Ok(result)
}

#[tauri::command]
pub fn plugins_account_set_auto_checkin(
    manager: State<'_, PluginManagerState>,
    account: String,
    plugin_id: String,
    provider: Option<String>,
    enabled: bool,
) -> Result<PluginAccountMetaRecord, String> {
    let existing = store::list_account_metas(manager.db())?
        .into_iter()
        .find(|meta| meta.id == account);
    let mut record = existing.unwrap_or_else(|| PluginAccountMetaRecord {
        id: account.clone(),
        plugin_id: plugin_id.clone(),
        ..PluginAccountMetaRecord::default()
    });
    record.auto_checkin = enabled;
    if let Some(provider) = provider.filter(|value| !value.trim().is_empty()) {
        record.provider_id = provider;
    }
    if record.plugin_id.trim().is_empty() {
        record.plugin_id = plugin_id;
    }
    store::upsert_account_meta(manager.db(), &record)
}

#[tauri::command]
pub fn plugins_set_provider_auto_checkin(
    manager: State<'_, PluginManagerState>,
    provider_id: String,
    enabled: bool,
) -> Result<bool, String> {
    store::set_provider_auto_checkin(manager.db(), &provider_id, enabled)
}

#[tauri::command]
pub fn plugins_account_set_enabled(
    manager: State<'_, PluginManagerState>,
    account: String,
    plugin_id: String,
    provider: Option<String>,
    display_name: Option<String>,
    enabled: bool,
) -> Result<PluginAccountMetaRecord, String> {
    let existing = store::list_account_metas(manager.db())?
        .into_iter()
        .find(|meta| meta.id == account);
    let mut record = existing.unwrap_or_else(|| PluginAccountMetaRecord {
        id: account.clone(),
        plugin_id: plugin_id.clone(),
        ..PluginAccountMetaRecord::default()
    });
    record.enabled = enabled;
    if let Some(provider) = provider.filter(|value| !value.trim().is_empty()) {
        record.provider_id = provider;
    }
    if let Some(display_name) = display_name {
        record.display_name = display_name;
    }
    if record.plugin_id.trim().is_empty() {
        record.plugin_id = plugin_id;
    }
    store::upsert_account_meta(manager.db(), &record)
}

#[tauri::command]
pub async fn plugins_set_provider_option(
    manager: State<'_, PluginManagerState>,
    provider_id: String,
    options: Value,
) -> Result<PluginProviderOptionValue, String> {
    let manager = manager.inner().clone();
    let record = store::set_provider_option(manager.db(), &provider_id, &options)?;
    manager.gateway().clear_provider_cache();

    if let Err(error) = manager
        .host()
        .set_provider_options(provider_options_value(manager.db())?)
        .await
    {
        log::warn!("pushing provider options to the plugin host failed: {error}");
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_listing_carries_saved_option_values() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        store::set_provider_option(&db, "demo", &json!({"fast": true})).unwrap();
        store::set_provider_option(&db, "other", &json!({"retries": 3})).unwrap();

        let listing = json!([
            {"id": "demo", "name": "Demo", "models": []},
            {"id": "unknown", "name": "Unknown", "models": []},
            "not-an-object",
        ]);
        let merged = attach_provider_option_values(&db, listing).unwrap();
        let rows = merged.as_array().unwrap();
        assert_eq!(rows[0]["optionValues"], json!({"fast": true}));
        assert_eq!(rows[1]["optionValues"], json!({}));
        assert_eq!(rows[2], json!("not-an-object"));
        assert_eq!(rows[0]["name"], "Demo");
    }

    #[test]
    fn provider_listing_without_saved_options_stays_empty() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let merged = attach_provider_option_values(&db, json!([{"id": "demo"}])).unwrap();
        assert_eq!(merged[0]["optionValues"], json!({}));
    }

    #[test]
    fn provider_listing_carries_the_channel_auto_checkin_switch() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        store::set_provider_auto_checkin(&db, "demo", true).unwrap();
        store::set_provider_auto_checkin(&db, "off", false).unwrap();

        let listing = json!([
            {"id": "demo", "name": "Demo"},
            {"id": "off", "name": "Off"},
            {"id": "unknown", "name": "Unknown"},
            "not-an-object",
        ]);
        let merged = attach_provider_auto_checkin(&db, listing).unwrap();
        let rows = merged.as_array().unwrap();
        assert_eq!(rows[0]["autoCheckin"], true);
        assert_eq!(rows[1]["autoCheckin"], false);
        assert_eq!(rows[2]["autoCheckin"], false);
        assert_eq!(rows[3], json!("not-an-object"));
    }

    #[test]
    fn legacy_account_auto_checkin_reaches_the_provider_listing() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        store::upsert_account_meta(
            &db,
            &PluginAccountMetaRecord {
                id: "demo#2".to_string(),
                plugin_id: "demo-plugin".to_string(),
                auto_checkin: true,
                ..PluginAccountMetaRecord::default()
            },
        )
        .unwrap();
        let merged = attach_provider_auto_checkin(&db, json!([{"id": "demo"}])).unwrap();
        assert_eq!(merged[0]["autoCheckin"], true);
    }

    #[test]
    fn usage_snapshot_persists_without_disabling_account_or_overwriting_with_errors() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        record_usage_snapshot(
            &db,
            "qoder",
            Some("qoder#1"),
            &json!({ "plan": "Pro", "windows": [{ "name": "Daily", "used": 25 }] }),
        )
        .unwrap();

        let metas = store::list_account_metas(&db).unwrap();
        assert_eq!(metas.len(), 1);
        assert!(metas[0].enabled);
        assert_eq!(metas[0].provider_id, "qoder");
        assert_eq!(metas[0].quota["plan"], "Pro");
        assert!(metas[0].quota_updated_at.is_some());

        // Error payloads must not clobber the last valid cached quota snapshot.
        record_usage_snapshot(
            &db,
            "qoder",
            Some("qoder#1"),
            &json!({ "error": "upstream timeout" }),
        )
        .unwrap();
        let metas = store::list_account_metas(&db).unwrap();
        assert_eq!(metas[0].quota["plan"], "Pro");
    }

    #[test]
    fn an_all_empty_normalized_usage_snapshot_keeps_the_previous_quota() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        record_usage_snapshot(
            &db,
            "qoder",
            Some("qoder#1"),
            &json!({ "plan": "Pro", "windows": [{ "name": "Daily", "used": 25 }] }),
        )
        .unwrap();
        let before = store::list_account_metas(&db).unwrap();
        let previous_updated_at = before[0].quota_updated_at.clone();
        assert!(previous_updated_at.is_some());

        // The Bun host normalizes a missing usage reply into this fixed object.
        record_usage_snapshot(
            &db,
            "qoder",
            Some("qoder#1"),
            &json!({
                "plan": "",
                "until": "",
                "renew": "",
                "balance": "",
                "error": "",
                "user": "",
                "signIn": "",
                "resets": null,
                "windows": []
            }),
        )
        .unwrap();

        let after = store::list_account_metas(&db).unwrap();
        assert_eq!(after[0].quota["plan"], "Pro");
        assert_eq!(after[0].quota_updated_at, previous_updated_at);
    }
}
