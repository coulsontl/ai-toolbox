//! `/v1/gateway/*` routes. Start/stop keep the backend's takeover preflight;
//! restart reuses the dedicated hot-restart path.

use std::collections::HashMap;

use hyper::Method;
use serde_json::Value;
use tauri::Manager;

use super::approval::RiskLevel;
use super::error::{ApiError, ApiResult};
use super::redact::redact_value;
use super::routes::{approve_write, body_bool, body_optional_string, body_string};
use super::ServerContext;
use crate::coding::proxy_gateway::commands as gateway_commands;
use crate::coding::proxy_gateway::types::{
    GatewayCliKey, GatewayRequestLogFilters, GatewayUsageTool,
};
use crate::coding::proxy_gateway::ProxyGatewayState;
use crate::db::SqliteDbState;

pub async fn dispatch(
    context: &ServerContext,
    method: &Method,
    segments: &[String],
    query: &HashMap<String, String>,
    body: Value,
) -> ApiResult<Value> {
    let tail = segments.get(1).map(String::as_str).unwrap_or_default();
    match (method, tail) {
        (&Method::GET, "status") => {
            let status = gateway_commands::proxy_gateway_status(gateway_state(context))
                .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(status)?))
        }
        (&Method::POST, "start") => {
            approve_write(
                context,
                RiskLevel::L2,
                "gateway.start",
                "Start the local proxy gateway",
                None,
                false,
            )
            .await?;
            let status = gateway_commands::proxy_gateway_start(
                gateway_state(context),
                sqlite_state(context),
                sqlite_state(context),
                context.app.clone(),
                None,
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(status)?))
        }
        (&Method::POST, "stop") => {
            approve_write(
                context,
                RiskLevel::L2,
                "gateway.stop",
                "Stop the local proxy gateway (rejected while any CLI is taken over)",
                None,
                false,
            )
            .await?;
            let status = gateway_commands::proxy_gateway_stop(
                gateway_state(context),
                sqlite_state(context),
                sqlite_state(context),
                context.app.clone(),
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(status)?))
        }
        (&Method::POST, "restart") => {
            approve_write(
                context,
                RiskLevel::L1,
                "gateway.restart",
                "Restart the local proxy gateway runtime",
                None,
                false,
            )
            .await?;
            let status = gateway_commands::proxy_gateway_restart(
                gateway_state(context),
                sqlite_state(context),
                sqlite_state(context),
                context.app.clone(),
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(status)?))
        }
        (&Method::GET, "logs") => {
            let page = query
                .get("page")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0);
            let page_size = query
                .get("pageSize")
                .or_else(|| query.get("limit"))
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(20)
                .min(200);
            let mut filters = GatewayRequestLogFilters::default();
            filters.data_source = query.get("dataSource").cloned();
            filters.provider_name = query.get("providerName").cloned();
            filters.model = query.get("model").cloned();
            filters.status_code = query.get("statusCode").and_then(|value| value.parse().ok());
            if let Some(cli_key) = query.get("cliKey") {
                filters.cli_key = Some(
                    serde_json::from_value::<GatewayUsageTool>(Value::String(cli_key.clone()))
                        .map_err(|_| {
                            ApiError::bad_request(format!("Unknown gateway usage tool '{cli_key}'"))
                        })?,
                );
            }
            filters.only_failed = query
                .get("onlyFailed")
                .and_then(|value| value.parse::<bool>().ok());
            let logs = gateway_commands::proxy_gateway_request_logs(
                sqlite_state(context),
                Some(filters),
                Some(page),
                Some(page_size),
                None,
            )
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(logs)?))
        }
        (&Method::GET, "health") => {
            let items = gateway_commands::proxy_gateway_model_health_entries(
                context.app.clone(),
                gateway_state(context),
                sqlite_state(context),
                sqlite_state(context),
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(items)?))
        }
        (&Method::POST, "health" | "reset") => {
            if segments.get(2).map(String::as_str) != Some("reset") && tail != "reset" {
                return Err(ApiError::not_found("Unknown gateway health route"));
            }
            approve_write(
                context,
                RiskLevel::L1,
                "gateway.health.reset",
                "Clear model health cooldowns",
                None,
                false,
            )
            .await?;
            let result = gateway_commands::proxy_gateway_reset_model_health(gateway_state(context))
                .map_err(ApiError::from)?;
            Ok(result)
        }
        (&Method::POST, "takeover") => {
            let action = segments.get(2).map(String::as_str).unwrap_or_default();
            match action {
                "engage" => engage(context, body).await,
                "restore" => restore(context, body).await,
                _ => Err(ApiError::not_found("Unknown gateway takeover route")),
            }
        }
        _ => Err(ApiError::not_found(format!(
            "Unknown gateway route '/{}'",
            segments.join("/")
        ))),
    }
}

async fn engage(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let cli_key = normalize_cli_key(&body_string(&body, "cliKey")?)?;
    let mode = body_optional_string(&body, "mode").unwrap_or_else(|| "single".to_string());
    approve_write(
        context,
        RiskLevel::L2,
        "gateway.takeover.engage",
        format!("Take over '{cli_key}' through the local proxy gateway ({mode})"),
        Some(cli_key.clone()),
        false,
    )
    .await?;

    match mode.as_str() {
        "single" => {
            let provider_id = body_string(&body, "providerId")?;
            let status = gateway_commands::proxy_gateway_engage_single(
                gateway_state(context),
                sqlite_state(context),
                context.app.clone(),
                cli_key.parse_gateway_cli()?,
                provider_id,
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(status)?))
        }
        "failover" => {
            let status = gateway_commands::proxy_gateway_engage_failover(
                gateway_state(context),
                sqlite_state(context),
                context.app.clone(),
                cli_key.parse_gateway_cli()?,
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(status)?))
        }
        "aggregate" => {
            let provider_ids: Vec<String> = body
                .get("providerIds")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let separator = body_optional_string(&body, "separator");
            let aliases = body
                .get("aliases")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| ApiError::bad_request(format!("Invalid aliases: {error}")))?;
            let naming = body
                .get("naming")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| ApiError::bad_request(format!("Invalid naming mode: {error}")))?;
            let cross_site_failover = body_bool(&body, "crossSiteFailover");
            let status = gateway_commands::proxy_gateway_engage_aggregate(
                gateway_state(context),
                sqlite_state(context),
                context.app.clone(),
                cli_key.parse_gateway_cli()?,
                provider_ids,
                separator,
                aliases,
                naming,
                cross_site_failover,
                None,
                None,
                None,
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(status)?))
        }
        other => Err(ApiError::bad_request(format!(
            "Unknown takeover mode '{other}'; use single, failover or aggregate"
        ))),
    }
}

async fn restore(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let cli_key = normalize_cli_key(&body_string(&body, "cliKey")?)?;
    approve_write(
        context,
        RiskLevel::L2,
        "gateway.takeover.restore",
        format!("Restore '{cli_key}' to direct CLI configuration"),
        Some(cli_key.clone()),
        false,
    )
    .await?;
    let status = gateway_commands::proxy_gateway_restore_cli_direct(
        gateway_state(context),
        sqlite_state(context),
        context.app.clone(),
        cli_key.parse_gateway_cli()?,
    )
    .await
    .map_err(ApiError::from)?;
    Ok(redacted(serde_json::to_value(status)?))
}

fn normalize_cli_key(raw: &str) -> ApiResult<String> {
    super::routes_cli::normalize_cli_key(raw)
        .map(str::to_string)
        .ok_or_else(|| ApiError::bad_request(format!("Unknown CLI key '{raw}'")))
}

trait ParseGatewayCli {
    fn parse_gateway_cli(&self) -> ApiResult<GatewayCliKey>;
}

impl ParseGatewayCli for String {
    fn parse_gateway_cli(&self) -> ApiResult<GatewayCliKey> {
        let canonical = match self.as_str() {
            "claudecode" => "claude",
            "claudedesktop" => "claude_desktop",
            "geminicli" => "gemini",
            other => other,
        };
        serde_json::from_value(Value::String(canonical.to_string()))
            .map_err(|_| ApiError::bad_request(format!("'{}' cannot be taken over", self)))
    }
}

fn gateway_state(context: &ServerContext) -> tauri::State<'_, ProxyGatewayState> {
    context.app.state::<ProxyGatewayState>()
}

fn sqlite_state(context: &ServerContext) -> tauri::State<'_, SqliteDbState> {
    context.app.state::<SqliteDbState>()
}

#[allow(dead_code)]
fn redacted(value: Value) -> Value {
    let mut value = value;
    redact_value(&mut value);
    value
}
