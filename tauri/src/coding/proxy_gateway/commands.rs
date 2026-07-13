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
    DataSourceBreakdownInput, DataSourceBreakdownItem, GatewayCliKey, GatewayCliTakeoverStatus,
    GatewayConnectivityTestRequest, GatewayConnectivityTestResponse, GatewayModelHealthItem,
    GatewayModelStats, GatewayPaginatedRequestLogs, GatewayProviderStats, GatewayRequestLogDetail,
    GatewayRequestLogFilters, GatewaySessionUsageImportInput, GatewaySessionUsageImportResult,
    GatewayUsageRecordedEvent, GatewayUsageSummary, GatewayUsageSummaryByCli,
    GatewayUsageTrendPoint, ModelPricing, ProxyGatewayHealthCheckResult,
    ProxyGatewayPortCheckInput, ProxyGatewayPortCheckResult, ProxyGatewayRequestLogListInput,
    ProxyGatewaySettings, ProxyGatewayStatus, ProxyGatewayStopPreflight,
};
use super::usage_stats;
use crate::db::helpers::db_list;
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::{model_pricing_seed, SqliteDbState};
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tauri::{Emitter, Manager};

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
    )
}

#[tauri::command]
pub fn proxy_gateway_request_log_detail(
    app: tauri::AppHandle,
    db_state: tauri::State<'_, SqliteDbState>,
    trace_id: String,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    load_request_log_detail(&app, &db_state, &trace_id)
}

#[tauri::command]
pub fn proxy_gateway_export_request_log_detail(
    app: tauri::AppHandle,
    db_state: tauri::State<'_, SqliteDbState>,
    trace_id: String,
    export_path: String,
) -> Result<(), String> {
    let detail = load_request_log_detail(&app, &db_state, &trace_id)?
        .ok_or_else(|| "Gateway request detail not found".to_string())?;
    let export_json = build_request_log_detail_export(&detail);
    write_export_json(Path::new(&export_path), &export_json)
}

fn load_request_log_detail(
    app: &tauri::AppHandle,
    db_state: &SqliteDbState,
    trace_id: &str,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    let paths = proxy_gateway_paths(app)?;
    if let Some((detail_file, detail_offset)) =
        usage_stats::request_log_location(db_state, trace_id)?
    {
        if let Some(detail) =
            request_log::get_request_log_detail_at(&paths, &detail_file, detail_offset, trace_id)?
        {
            return Ok(Some(sanitize_request_log_detail_for_display(detail)));
        }
    }
    if let Some(detail) = request_log::get_request_log_detail(&paths, trace_id)? {
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

fn build_request_log_detail_export(detail: &GatewayRequestLogDetail) -> Value {
    let summary = &detail.summary;
    let requested_model = summary.requested_model.as_deref();
    let upstream_model = summary.upstream_model_id.as_deref();
    let mut summary_value = serde_json::to_value(summary).unwrap_or(Value::Null);
    let mut provider_attempts_value =
        serde_json::to_value(&detail.provider_attempts).unwrap_or(Value::Null);
    redact_json_value(&mut summary_value);
    redact_json_value(&mut provider_attempts_value);
    serde_json::json!({
        "schema_version": 1,
        "exported_at": Utc::now().to_rfc3339(),
        "redaction": {
            "placeholder": "xxx",
            "note": "Authentication-like fields and header values are redacted during export."
        },
        "summary": summary_value,
        "provider_attempts": provider_attempts_value,
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

    #[test]
    fn display_sanitization_redacts_path_and_upstream_url_queries() {
        let now = Utc::now();
        let detail = GatewayRequestLogDetail {
            summary: GatewayRequestLogSummary {
                trace_id: "trace-redact-display".to_string(),
                started_at: now,
                ended_at: now,
                cli_key: Some(GatewayCliKey::Gemini),
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
                upstream_url: Some(
                    "https://generativelanguage.googleapis.com/v1beta/models?key=secret&api-key=hyphen"
                        .to_string(),
                ),
                status_code: Some(200),
                success: true,
                error_category: None,
                error_message: None,
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

        let sanitized = sanitize_request_log_detail_for_display(detail);

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
pub fn proxy_gateway_data_source_breakdown(
    db_state: tauri::State<'_, SqliteDbState>,
    input: Option<DataSourceBreakdownInput>,
) -> Result<Vec<DataSourceBreakdownItem>, String> {
    usage_stats::data_source_breakdown(&db_state, input.unwrap_or_default())
}

#[tauri::command]
pub async fn proxy_gateway_import_session_usage(
    app: tauri::AppHandle,
    db_state: tauri::State<'_, SqliteDbState>,
    input: GatewaySessionUsageImportInput,
) -> Result<GatewaySessionUsageImportResult, String> {
    let result = session_import::import_session_usage(db_state.db().clone(), input).await?;
    if result.inserted_records > 0 {
        let payload = GatewayUsageRecordedEvent {
            cli_key: None,
            trace_id: None,
            data_source: "session".to_string(),
            inserted_records: result.inserted_records,
        };
        if let Err(error) = app.emit("usage-log-recorded", payload) {
            log::warn!("Failed to emit gateway session usage recorded event: {error}");
        }
    }
    Ok(result)
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

#[tauri::command]
pub async fn fetch_remote_model_pricing(
    db_state: tauri::State<'_, SqliteDbState>,
    url: String,
) -> Result<model_pricing_seed::ModelPricingSeedResult, String> {
    model_pricing_seed::fetch_remote_model_pricing(&db_state, url).await
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
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Failed to resolve app data directory: {error}"))?;
    Ok(ProxyGatewayPaths::new(app_data_dir))
}

fn emit_gateway_cli_wsl_sync_request(app: &tauri::AppHandle, cli_key: GatewayCliKey) {
    let event_name = match cli_key {
        GatewayCliKey::Claude => "wsl-sync-request-claude",
        GatewayCliKey::Codex => "wsl-sync-request-codex",
        GatewayCliKey::Grok => "wsl-sync-request-grok",
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
        (GatewayCliKey::Codex, DbTable::CodexProvider),
        (GatewayCliKey::Grok, DbTable::GrokProvider),
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
