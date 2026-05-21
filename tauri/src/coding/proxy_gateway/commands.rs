use super::cli_proxy;
use super::listen::check_port_available;
use super::model_health;
use super::paths::ProxyGatewayPaths;
use super::request_log;
use super::runtime::ProxyGatewayState;
use super::session_import;
use super::settings;
use super::types::{
    GatewayCliKey, GatewayCliTakeoverStatus, GatewayModelHealthItem, GatewayModelStats,
    GatewayPaginatedRequestLogs, GatewayProviderStats, GatewayRequestLogDetail,
    GatewayRequestLogFilters, GatewaySessionUsageImportInput, GatewaySessionUsageImportResult,
    GatewayUsageSummary, GatewayUsageSummaryByCli, GatewayUsageTrendPoint,
    ProxyGatewayHealthCheckResult, ProxyGatewayPortCheckInput, ProxyGatewayPortCheckResult,
    ProxyGatewayRequestLogListInput, ProxyGatewaySettings, ProxyGatewayStatus,
    ProxyGatewayStopPreflight,
};
use super::usage_stats;
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
    )
}

#[tauri::command]
pub fn proxy_gateway_request_log_detail(
    app: tauri::AppHandle,
    db_state: tauri::State<'_, SqliteDbState>,
    trace_id: String,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    let paths = proxy_gateway_paths(&app)?;
    if let Some((detail_file, detail_offset)) =
        usage_stats::request_log_location(&db_state, &trace_id)?
    {
        if let Some(detail) =
            request_log::get_request_log_detail_at(&paths, &detail_file, detail_offset, &trace_id)?
        {
            return Ok(Some(detail));
        }
    }
    if let Some(detail) = request_log::get_request_log_detail(&paths, &trace_id)? {
        return Ok(Some(detail));
    }
    usage_stats::request_log_detail_from_summary(&db_state, &trace_id)
}

#[tauri::command]
pub fn proxy_gateway_usage_summary(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
) -> Result<GatewayUsageSummary, String> {
    usage_stats::usage_summary(&db_state, start_date, end_date, cli_key)
}

#[tauri::command]
pub fn proxy_gateway_usage_summary_by_cli(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
) -> Result<Vec<GatewayUsageSummaryByCli>, String> {
    usage_stats::usage_summary_by_cli(&db_state, start_date, end_date)
}

#[tauri::command]
pub fn proxy_gateway_usage_trends(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
) -> Result<Vec<GatewayUsageTrendPoint>, String> {
    usage_stats::usage_trends(&db_state, start_date, end_date, cli_key)
}

#[tauri::command]
pub fn proxy_gateway_provider_stats(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
) -> Result<Vec<GatewayProviderStats>, String> {
    usage_stats::provider_stats(&db_state, start_date, end_date, cli_key)
}

#[tauri::command]
pub fn proxy_gateway_model_stats(
    db_state: tauri::State<'_, SqliteDbState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
) -> Result<Vec<GatewayModelStats>, String> {
    usage_stats::model_stats(&db_state, start_date, end_date, cli_key)
}

#[tauri::command]
pub async fn proxy_gateway_import_session_usage(
    db_state: tauri::State<'_, SqliteDbState>,
    input: GatewaySessionUsageImportInput,
) -> Result<GatewaySessionUsageImportResult, String> {
    session_import::import_session_usage(db_state.db().clone(), input).await
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
