use super::cli_proxy;
use super::listen::check_port_available;
use super::metrics;
use super::model_health;
use super::paths::ProxyGatewayPaths;
use super::request_log;
use super::runtime::ProxyGatewayState;
use super::settings;
use super::types::{
    GatewayCliKey, GatewayCliTakeoverStatus, GatewayModelHealthItem, GatewayRequestLogDetail,
    GatewayRequestLogSummary, MetricRollupItem, ProxyGatewayHealthCheckResult,
    ProxyGatewayPortCheckInput, ProxyGatewayPortCheckResult, ProxyGatewayRequestLogListInput,
    ProxyGatewaySettings, ProxyGatewayStatus, ProxyGatewayStopPreflight,
};
use crate::db::helpers::db_list;
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use serde_json::Value;
use std::collections::HashMap;
use tauri::Manager;

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
        .start_with_context(settings, db_state.db().clone(), paths)
        .map(Some)
}

#[tauri::command]
pub async fn proxy_gateway_get_settings(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<ProxyGatewaySettings, String> {
    settings::load_settings_from_sqlite_state(&sqlite_state)
}

#[tauri::command]
pub async fn proxy_gateway_update_settings(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    mut settings: ProxyGatewaySettings,
) -> Result<ProxyGatewaySettings, String> {
    {
        let mut manager = gateway_state
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        let running = manager.status().running;
        if running {
            settings.enabled_on_startup = true;
            manager.update_runtime_settings(settings.clone())?;
        }
    }
    settings::save_settings(&sqlite_state, settings)
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
        manager.start_with_context(settings.clone(), db_state.db().clone(), paths)?
    };

    settings.enabled_on_startup = true;
    if let Err(error) = settings::save_settings(&sqlite_state, settings) {
        log::warn!("Failed to persist proxy gateway startup state after start: {error}");
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

    let mut manager = gateway_state
        .manager
        .lock()
        .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
    manager.stop()
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
pub fn proxy_gateway_health_check(
    gateway_state: tauri::State<'_, ProxyGatewayState>,
) -> Result<ProxyGatewayHealthCheckResult, String> {
    let manager = gateway_state
        .manager
        .lock()
        .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
    Ok(manager.health_check())
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
pub async fn proxy_gateway_takeover_cli(
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
    cli_proxy::takeover_cli(db_state.db(), &paths, cli_key, &status).await
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
    cli_proxy::restore_cli_direct(db_state.db(), &paths, cli_key, &status).await
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
    app: tauri::AppHandle,
    input: ProxyGatewayRequestLogListInput,
) -> Result<Vec<GatewayRequestLogSummary>, String> {
    let paths = proxy_gateway_paths(&app)?;
    request_log::list_request_logs(&paths, input)
}

#[tauri::command]
pub fn proxy_gateway_request_log_detail(
    app: tauri::AppHandle,
    trace_id: String,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    let paths = proxy_gateway_paths(&app)?;
    request_log::get_request_log_detail(&paths, &trace_id)
}

#[tauri::command]
pub fn proxy_gateway_metric_rollups(
    app: tauri::AppHandle,
) -> Result<Vec<MetricRollupItem>, String> {
    let paths = proxy_gateway_paths(&app)?;
    metrics::list_metric_rollups(&paths)
}

#[tauri::command]
pub async fn proxy_gateway_model_health_entries(
    app: tauri::AppHandle,
    sqlite_state: tauri::State<'_, SqliteDbState>,
    db_state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GatewayModelHealthItem>, String> {
    let paths = proxy_gateway_paths(&app)?;
    let settings = settings::load_settings_from_sqlite_state(&sqlite_state)?;
    let mut items = model_health::list_model_health_items(&paths.model_health_path(), settings)?;
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

fn proxy_gateway_paths(app: &tauri::AppHandle) -> Result<ProxyGatewayPaths, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Failed to resolve app data directory: {error}"))?;
    Ok(ProxyGatewayPaths::new(app_data_dir))
}

async fn load_provider_name_map(
    db: &SqliteDbState,
) -> Result<HashMap<(GatewayCliKey, String), String>, String> {
    let mut provider_names = HashMap::new();
    for (cli_key, table) in [
        (GatewayCliKey::Claude, DbTable::ClaudeProvider),
        (GatewayCliKey::Codex, DbTable::CodexProvider),
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
    Ok(provider_names)
}
