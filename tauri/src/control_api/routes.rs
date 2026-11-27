//! Top-level route table for `/v1/*`.

use std::collections::HashMap;

use hyper::Method;
use serde_json::{json, Value};

use super::approval::{require_approval, ApprovalContext, ApprovalRequestSpec, RiskLevel};
use super::discovery;
use super::error::{ApiError, ApiResult};
use super::redact::redact_value;
use super::{routes_cli, routes_gateway, routes_plugins, ServerContext};

pub async fn dispatch(
    context: &ServerContext,
    method: &Method,
    segments: &[String],
    query: &HashMap<String, String>,
    body: Value,
) -> ApiResult<Value> {
    let tail = segments.get(1).map(String::as_str).unwrap_or_default();
    match tail {
        "status" => {
            require_method(method, &[Method::GET])?;
            Ok(status_payload(context))
        }
        "clis" => routes_cli::dispatch(context, method, &segments[1..], query, body).await,
        "plugins" => routes_plugins::dispatch(context, method, &segments[1..], query, body).await,
        "gateway" => routes_gateway::dispatch(context, method, &segments[1..], query, body).await,
        _ => Err(ApiError::not_found(format!(
            "Unknown control API path '/{}'",
            segments.join("/")
        ))),
    }
}

fn require_method(method: &Method, allowed: &[Method]) -> ApiResult<()> {
    if allowed.iter().any(|candidate| candidate == method) {
        Ok(())
    } else {
        Err(ApiError::new(
            405,
            "method_not_allowed",
            format!("Method {method} is not allowed on this route"),
        ))
    }
}

pub fn status_payload(context: &ServerContext) -> Value {
    let settings = context.settings();
    let running = context.port() != 0;
    let discovery_path = discovery::default_discovery_path();
    json!({
        "app": "ai-toolbox",
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "enabled": settings.enabled,
        "running": running,
        "url": if running { Value::String(context.url()) } else { Value::Null },
        "discoveryFile": discovery_path.to_string_lossy(),
        "dataDir": crate::app_paths::resolved_data_dir().to_string_lossy(),
        "approvalMode": settings.approval_mode,
        "approvalTimeoutSecs": settings.approval_timeout_secs,
        "autoApproveLocalPluginReload": settings.auto_approve_local_plugin_reload,
    })
}

/// One place for write-route approval so the risk level cannot drift between
/// the HTTP layer and the desktop dialog.
pub async fn approve_write(
    context: &ServerContext,
    risk: RiskLevel,
    action: impl Into<String>,
    summary: impl Into<String>,
    target: Option<String>,
    is_local_plugin_reload: bool,
) -> ApiResult<()> {
    let settings = context.settings();
    let approval = ApprovalContext {
        app: &context.app,
        hub: &context.approval,
        settings,
    };
    require_approval(
        &approval,
        ApprovalRequestSpec {
            action: action.into(),
            summary: summary.into(),
            target,
            risk,
            is_local_plugin_reload,
        },
    )
    .await
}

pub fn redacted(value: Value) -> Value {
    let mut value = value;
    redact_value(&mut value);
    value
}

pub fn body_string(body: &Value, key: &str) -> ApiResult<String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ApiError::bad_request(format!("'{key}' is required")))
}

pub fn body_optional_string(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn body_bool(body: &Value, key: &str) -> Option<bool> {
    body.get(key).and_then(Value::as_bool)
}
