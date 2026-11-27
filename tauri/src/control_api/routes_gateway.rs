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
        (&Method::GET, "clis") => {
            let statuses = gateway_commands::proxy_gateway_cli_statuses(
                gateway_state(context),
                sqlite_state(context),
                context.app.clone(),
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(statuses)?))
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
                filters.cli_key = Some(gateway_usage_tool(cli_key)?);
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
            let result = gateway_commands::proxy_gateway_reset_model_health(
                &context.app,
                gateway_state(context).inner(),
            )
            .map_err(ApiError::from)?;
            Ok(redacted(result))
        }
        (&Method::POST, "takeover") => {
            let action = segments.get(2).map(String::as_str).unwrap_or_default();
            match action {
                "engage" => engage(context, body).await,
                "restore" => restore(context, body).await,
                "switch" => switch_primary(context, body).await,
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
            let gateway_cli = cli_key.parse_gateway_cli()?;
            // Missing fields fall back to the saved aggregate draft/manifest.
            // An omitted sub-flag must not silently reset the user's aliases,
            // naming mode, exposure selection or `[agents]` defaults.
            let existing = gateway_commands::proxy_gateway_aggregate_draft(
                context.app.clone(),
                gateway_cli,
            )
            .await
            .ok()
            .flatten();

            let provider_ids: Vec<String> = match body.get("providerIds").and_then(Value::as_array)
            {
                Some(items) => items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                None => existing
                    .as_ref()
                    .map(|draft| draft.provider_ids.clone())
                    .unwrap_or_default(),
            };
            if provider_ids.is_empty() {
                return Err(ApiError::bad_request(
                    "Aggregate mode requires at least one 'providerIds' entry",
                ));
            }
            let separator = body_optional_string(&body, "separator").or_else(|| {
                existing
                    .as_ref()
                    .map(|draft| draft.separator.clone())
            });
            let aliases = match body.get("aliases") {
                Some(value) => Some(
                    serde_json::from_value(value.clone()).map_err(|error| {
                        ApiError::bad_request(format!("Invalid aliases: {error}"))
                    })?,
                ),
                None => existing.as_ref().map(|draft| draft.aliases.clone()),
            };
            let naming = match body.get("naming") {
                Some(value) => serde_json::from_value(value.clone()).map(Some).map_err(
                    |error| ApiError::bad_request(format!("Invalid naming mode: {error}")),
                )?,
                None => existing.as_ref().map(|draft| draft.naming),
            };
            let cross_site_failover = body_bool(&body, "crossSiteFailover")
                .or_else(|| existing.as_ref().map(|draft| draft.cross_site_failover));
            let subagent_exposed_models = match body.get("subagentExposedModels") {
                Some(value) => Some(
                    serde_json::from_value::<Vec<String>>(value.clone()).map_err(|error| {
                        ApiError::bad_request(format!("Invalid subagentExposedModels: {error}"))
                    })?,
                ),
                None => existing
                    .as_ref()
                    .map(|draft| draft.subagent_exposed_models.clone()),
            };
            let subagent_model = body_optional_string(&body, "subagentModel").or_else(|| {
                existing
                    .as_ref()
                    .and_then(|draft| draft.subagent.as_ref())
                    .and_then(|subagent| subagent.model.clone())
            });
            let subagent_reasoning_effort =
                body_optional_string(&body, "subagentReasoningEffort").or_else(|| {
                    existing
                        .as_ref()
                        .and_then(|draft| draft.subagent.as_ref())
                        .and_then(|subagent| subagent.reasoning_effort.clone())
                });
            let status = gateway_commands::proxy_gateway_engage_aggregate(
                gateway_state(context),
                sqlite_state(context),
                context.app.clone(),
                gateway_cli,
                provider_ids,
                separator,
                aliases,
                naming,
                cross_site_failover,
                subagent_exposed_models,
                subagent_model,
                subagent_reasoning_effort,
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

/// Switch the primary provider of a CLI that is already managed by the
/// gateway. Reuses the desktop orchestration (restore -> apply -> re-engage)
/// so failover and aggregate manifests keep their configuration.
async fn switch_primary(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let cli_key = normalize_cli_key(&body_string(&body, "cliKey")?)?;
    let provider_id = body_string(&body, "providerId")?;
    approve_write(
        context,
        RiskLevel::L2,
        "gateway.takeover.switch",
        format!("Switch the gateway-managed provider of '{cli_key}' to '{provider_id}'"),
        Some(cli_key.clone()),
        false,
    )
    .await?;
    let status = gateway_commands::proxy_gateway_switch_primary_provider(
        context.app.clone(),
        cli_key.parse_gateway_cli()?,
        provider_id,
    )
    .await
    .map_err(ApiError::from)?;
    Ok(redacted(serde_json::to_value(status)?))
}

/// `aitb gateway logs --cli claudecode` uses the coding-tab spelling; usage
/// statistics and the log filter use the gateway tool spelling.
fn gateway_usage_tool(cli_key: &str) -> ApiResult<GatewayUsageTool> {
    let canonical = match cli_key {
        "claudecode" | "claude_code" => "claude",
        "claudedesktop" | "claude_desktop" => "claude_desktop",
        "geminicli" | "gemini_cli" => "gemini",
        other => other,
    };
    serde_json::from_value::<GatewayUsageTool>(Value::String(canonical.to_string()))
        .map_err(|_| ApiError::bad_request(format!("Unknown gateway usage tool '{cli_key}'")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_logs_cli_keys_map_to_the_usage_tool_spelling() {
        assert_eq!(
            gateway_usage_tool("claudecode").unwrap(),
            GatewayUsageTool::Claude
        );
        assert_eq!(
            gateway_usage_tool("claudedesktop").unwrap(),
            GatewayUsageTool::ClaudeDesktop
        );
        assert_eq!(
            gateway_usage_tool("geminicli").unwrap(),
            GatewayUsageTool::Gemini
        );
        assert_eq!(
            gateway_usage_tool("codex").unwrap(),
            GatewayUsageTool::Codex
        );
        assert_eq!(gateway_usage_tool("not-a-cli").unwrap_err().status, 400);
    }
}
