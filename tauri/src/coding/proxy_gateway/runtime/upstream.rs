use super::debug_log::{log_upstream_request, log_upstream_response};
use super::http_io::{json_response, DebugHttpRequest, DebugHttpResponse};
use super::providers::{load_candidate_providers, UpstreamModelMapping, UpstreamProvider};
use super::routes::{build_target_url, match_gateway_route, split_request_target, GatewayRoute};
use super::GatewayRuntimeContext;
use crate::coding::proxy_gateway::model_health::{self, GatewayFailureKind, ModelHealthRegistry};
use crate::coding::proxy_gateway::types::{GatewayCliKey, ProviderModelHealthKey};
use crate::db::SqliteDbState;
use crate::http_client;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_ENCODING, AUTHORIZATION, CONNECTION, CONTENT_LENGTH,
    HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};
use serde_json::{json, Value};

#[derive(Debug)]
struct GatewayForwardError {
    message: String,
    kind: GatewayFailureKind,
    upstream_request_body: Option<Vec<u8>>,
}

impl GatewayForwardError {
    fn new(message: impl Into<String>, kind: GatewayFailureKind) -> Self {
        Self {
            message: message.into(),
            kind,
            upstream_request_body: None,
        }
    }
}

pub(super) async fn route_request(
    request: &DebugHttpRequest,
    context: &GatewayRuntimeContext,
) -> DebugHttpResponse {
    let (request_path, _) = split_request_target(&request.path);
    if request.method == "GET" && request_path == "/health" {
        return json_response(
            200,
            "OK",
            json!({"ok": true}),
            "health",
            None,
            "local health endpoint",
        );
    }

    let Some(route) = match_gateway_route(&request.path) else {
        return json_response(
            404,
            "Not Found",
            json!({"error": "not_found"}),
            "unknown",
            None,
            "no gateway route matched this path",
        );
    };

    let Some(db) = context.db.as_ref() else {
        return json_response(
            503,
            "Service Unavailable",
            json!({
                "error": "gateway_provider_state_missing",
                "message": "Proxy gateway was started without database access, so it cannot resolve upstream providers."
            }),
            route.route_name,
            None,
            "matched CLI gateway route, but runtime has no database handle",
        );
    };

    forward_to_upstream(request, db, context, &route).await
}

async fn forward_to_upstream(
    request: &DebugHttpRequest,
    db: &SqliteDbState,
    context: &GatewayRuntimeContext,
    route: &GatewayRoute,
) -> DebugHttpResponse {
    let requested_model =
        extract_requested_model(request, route).unwrap_or_else(|| "unknown".to_string());
    let providers = match load_candidate_providers(db, route.cli_key).await {
        Ok(providers) if !providers.is_empty() => providers,
        Ok(_) => {
            let mut response = json_response(
                502,
                "Bad Gateway",
                json!({
                    "error": "gateway_provider_missing",
                    "message": format!("No enabled provider for {}", route.cli_key.as_str()),
                }),
                route.route_name,
                None,
                "matched CLI gateway route, but no enabled upstream provider is configured",
            );
            response.cli_key = Some(route.cli_key);
            response.requested_model = Some(requested_model);
            response.error_category = Some("provider_missing".to_string());
            return response;
        }
        Err(error) => {
            let mut response = json_response(
                502,
                "Bad Gateway",
                json!({
                    "error": "gateway_provider_load_failed",
                    "message": error,
                }),
                route.route_name,
                None,
                "failed to resolve upstream provider candidates",
            );
            response.cli_key = Some(route.cli_key);
            response.requested_model = Some(requested_model);
            response.error_category = Some("provider_load_failed".to_string());
            return response;
        }
    };

    let settings = context.settings_snapshot();
    let mut health_registry = context.paths.as_ref().and_then(|paths| {
        match ModelHealthRegistry::load(&paths.model_health_path(), settings.clone()) {
            Ok(mut registry) => {
                registry.refresh_due_cooldowns(chrono::Utc::now());
                Some(registry)
            }
            Err(error) => {
                log::warn!("Failed to load proxy gateway model health: {error}");
                None
            }
        }
    });
    let mut health_changed = false;
    let mut attempt_count = 0_u32;
    let mut retry_count = 0_u32;
    let mut attempted_provider_count = 0_u32;
    let mut last_failure_response = None;
    let mut skipped_by_health = Vec::new();

    'providers: for provider in providers {
        let upstream_model_id = resolve_upstream_model_id(request, &requested_model, &provider);
        let health_key = ProviderModelHealthKey {
            cli_key: route.cli_key,
            provider_id: provider.id.clone(),
            upstream_model_id: upstream_model_id.clone(),
        };

        if health_registry
            .as_ref()
            .is_some_and(|registry| !registry.is_model_available(&health_key, chrono::Utc::now()))
        {
            skipped_by_health.push(provider.name.clone());
            continue;
        }

        attempted_provider_count = attempted_provider_count.saturating_add(1);
        let mut provider_retry_count = 0_u32;
        loop {
            if attempt_count > 0 && retry_count >= settings.max_retry_count {
                break 'providers;
            }

            attempt_count = attempt_count.saturating_add(1);
            if attempt_count > 1 {
                retry_count = retry_count.saturating_add(1);
            }

            match send_upstream_request(
                request,
                db,
                route,
                &provider,
                &requested_model,
                &upstream_model_id,
                settings.thinking_rectifier_enabled,
            )
            .await
            {
                Ok(mut response) => {
                    response.cli_key = Some(route.cli_key);
                    response.provider_id = Some(provider.id.clone());
                    response.provider_name = Some(provider.name.clone());
                    response.requested_model = Some(requested_model.clone());
                    response.upstream_model_id = Some(upstream_model_id.clone());
                    response.attempt_count = attempt_count;
                    response.provider_attempt_count = provider_retry_count.saturating_add(1);
                    response.failover = attempted_provider_count > 1;

                    if let Some(failure_kind) = classify_status_failure(response.status_code) {
                        let category = model_health::classify_failure(failure_kind).category;
                        response.error_category = Some(category.to_string());
                        if let Some(registry) = health_registry.as_mut() {
                            health_changed |= registry.record_failure(
                                &health_key,
                                failure_kind,
                                chrono::Utc::now(),
                            );
                        }
                        if should_retry_failure(failure_kind) {
                            last_failure_response = Some(response);
                            if can_retry_current_provider(
                                provider_retry_count,
                                settings.per_provider_retry_count,
                                retry_count,
                                settings.max_retry_count,
                            ) {
                                provider_retry_count = provider_retry_count.saturating_add(1);
                                continue;
                            }
                            continue 'providers;
                        }
                    } else if let Some(registry) = health_registry.as_mut() {
                        health_changed |= registry.record_success(&health_key);
                    }

                    save_health_registry_if_needed(
                        context,
                        health_registry.as_ref(),
                        health_changed,
                    );
                    return response;
                }
                Err(error) => {
                    let category = model_health::classify_failure(error.kind).category;
                    if let Some(registry) = health_registry.as_mut() {
                        health_changed |=
                            registry.record_failure(&health_key, error.kind, chrono::Utc::now());
                    }
                    let mut response = json_response(
                        502,
                        "Bad Gateway",
                        json!({
                            "error": "upstream_forward_failed",
                            "message": error.message,
                        }),
                        route.route_name,
                        None,
                        "upstream forwarding failed before a response was available",
                    );
                    response.cli_key = Some(route.cli_key);
                    response.provider_id = Some(provider.id.clone());
                    response.provider_name = Some(provider.name.clone());
                    response.requested_model = Some(requested_model.clone());
                    response.upstream_model_id = Some(health_key.upstream_model_id.clone());
                    response.upstream_request_body = error.upstream_request_body;
                    response.error_category = Some(category.to_string());
                    response.attempt_count = attempt_count;
                    response.provider_attempt_count = provider_retry_count.saturating_add(1);
                    response.failover = attempted_provider_count > 1;
                    last_failure_response = Some(response);
                    if can_retry_current_provider(
                        provider_retry_count,
                        settings.per_provider_retry_count,
                        retry_count,
                        settings.max_retry_count,
                    ) {
                        provider_retry_count = provider_retry_count.saturating_add(1);
                        continue;
                    }
                    continue 'providers;
                }
            }
        }
    }

    save_health_registry_if_needed(context, health_registry.as_ref(), health_changed);
    if let Some(response) = last_failure_response {
        return response;
    }

    let mut response = json_response(
        503,
        "Service Unavailable",
        json!({
            "error": "model_temporarily_unavailable",
            "message": "All provider candidates for this model are currently cooling down.",
            "skipped_providers": skipped_by_health,
        }),
        route.route_name,
        None,
        "all upstream provider candidates were skipped by model health",
    );
    response.cli_key = Some(route.cli_key);
    response.requested_model = Some(requested_model);
    response.error_category = Some("cooling_down".to_string());
    response
}

async fn send_upstream_request(
    request: &DebugHttpRequest,
    db: &SqliteDbState,
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    requested_model: &str,
    upstream_model_id: &str,
    thinking_rectifier_enabled: bool,
) -> Result<DebugHttpResponse, GatewayForwardError> {
    let upstream_url = build_target_url(
        &provider.base_url,
        &route.forwarded_path,
        route.query.as_deref(),
    )
    .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::GatewayParse))?;
    let method = reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|error| {
        GatewayForwardError::new(
            format!("Invalid HTTP method '{}': {error}", request.method),
            GatewayFailureKind::RequestSchema,
        )
    })?;
    let headers =
        build_upstream_headers(request, provider).map_err(|message| GatewayForwardError {
            message,
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: None,
        })?;
    let upstream_body = build_upstream_body(
        request,
        requested_model,
        upstream_model_id,
        thinking_rectifier_enabled,
    )?;
    let upstream_body_snapshot = upstream_body.clone();

    log_upstream_request(request, provider, &upstream_url, &headers, &upstream_body);

    let client = http_client::client_with_timeout_no_compression(db, 600)
        .await
        .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::Connection))?;
    let response = client
        .request(method, upstream_url.clone())
        .headers(headers)
        .body(upstream_body)
        .send()
        .await
        .map_err(|error| GatewayForwardError {
            message: format!("Failed to send upstream request: {error}"),
            kind: classify_reqwest_error(&error),
            upstream_request_body: Some(upstream_body_snapshot.clone()),
        })?;

    let status = response.status();
    let response_headers = filtered_response_headers(response.headers());
    let body = response
        .bytes()
        .await
        .map_err(|error| GatewayForwardError {
            message: format!("Failed to read upstream response body: {error}"),
            kind: classify_reqwest_error(&error),
            upstream_request_body: Some(upstream_body_snapshot.clone()),
        })?
        .to_vec();

    let gateway_response = DebugHttpResponse {
        status_code: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("Unknown").to_string(),
        headers: response_headers,
        body,
        cli_key: Some(provider.cli_key),
        route_name: route.route_name.to_string(),
        provider_id: Some(provider.id.clone()),
        provider_name: Some(provider.name.clone()),
        requested_model: None,
        upstream_model_id: None,
        upstream_request_body: Some(upstream_body_snapshot),
        upstream_url: Some(upstream_url.to_string()),
        error_category: None,
        attempt_count: 1,
        provider_attempt_count: 1,
        failover: false,
        note: format!(
            "forwarded to provider id={} name={}",
            provider.id, provider.name
        ),
    };
    log_upstream_response(request, &gateway_response);
    Ok(gateway_response)
}

fn can_retry_current_provider(
    provider_retry_count: u32,
    per_provider_retry_count: u32,
    retry_count: u32,
    max_retry_count: u32,
) -> bool {
    provider_retry_count < per_provider_retry_count && retry_count < max_retry_count
}

fn resolve_upstream_model_id(
    request: &DebugHttpRequest,
    requested_model: &str,
    provider: &UpstreamProvider,
) -> String {
    if provider.cli_key != GatewayCliKey::Claude {
        return requested_model.to_string();
    }

    resolve_claude_upstream_model_id(
        requested_model,
        &provider.model_mapping,
        is_claude_reasoning_request(request, requested_model),
    )
    .unwrap_or_else(|| requested_model.to_string())
}

fn resolve_claude_upstream_model_id(
    requested_model: &str,
    model_mapping: &UpstreamModelMapping,
    is_reasoning_request: bool,
) -> Option<String> {
    let normalized_model = requested_model.trim().to_ascii_lowercase();
    if is_reasoning_request
        || normalized_model.contains("reasoning")
        || normalized_model.contains("thinking")
    {
        return model_mapping
            .reasoning_model
            .clone()
            .or_else(|| family_model_fallback(&normalized_model, model_mapping));
    }
    family_model_fallback(&normalized_model, model_mapping)
}

fn family_model_fallback(
    normalized_model: &str,
    model_mapping: &UpstreamModelMapping,
) -> Option<String> {
    if normalized_model.contains("opus") {
        return model_mapping
            .opus_model
            .clone()
            .or_else(|| model_mapping.default_model.clone());
    }
    if normalized_model.contains("sonnet") {
        return model_mapping
            .sonnet_model
            .clone()
            .or_else(|| model_mapping.default_model.clone());
    }
    if normalized_model.contains("haiku") {
        return model_mapping
            .haiku_model
            .clone()
            .or_else(|| model_mapping.default_model.clone());
    }
    model_mapping.default_model.clone()
}

fn is_claude_reasoning_request(request: &DebugHttpRequest, requested_model: &str) -> bool {
    let normalized_model = requested_model.trim().to_ascii_lowercase();
    if normalized_model.contains("reasoning") || normalized_model.contains("thinking") {
        return true;
    }
    let Ok(value) = serde_json::from_slice::<Value>(&request.body) else {
        return false;
    };
    value
        .get("thinking")
        .filter(|thinking| !thinking.is_null() && *thinking != &Value::Bool(false))
        .is_some()
}

fn build_upstream_body(
    request: &DebugHttpRequest,
    requested_model: &str,
    upstream_model_id: &str,
    thinking_rectifier_enabled: bool,
) -> Result<Vec<u8>, GatewayForwardError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(&request.body) else {
        return Ok(request.body.clone());
    };
    let Some(model_value) = value.get_mut("model") else {
        return Ok(request.body.clone());
    };
    if !model_value.is_string() {
        return Ok(request.body.clone());
    }
    *model_value = Value::String(upstream_model_id.to_string());
    if thinking_rectifier_enabled && requested_model != upstream_model_id {
        strip_thinking_blocks(&mut value);
    }
    serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to rewrite upstream request model: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: None,
    })
}

fn strip_thinking_blocks(value: &mut Value) {
    if let Value::Object(object) = value {
        object.remove("thinking");
        object.remove("Thinking");
        if let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) {
            for message in messages {
                if let Some(content) = message.get_mut("content") {
                    strip_message_content(content);
                }
            }
        }
    }
}

fn strip_message_content(content: &mut Value) {
    match content {
        Value::Array(blocks) => {
            blocks.retain_mut(|block| {
                let should_remove =
                    block
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|block_type| {
                            matches!(block_type, "thinking" | "redacted_thinking")
                        });
                if should_remove {
                    false
                } else {
                    strip_direct_signature_field(block);
                    true
                }
            });
        }
        Value::Object(_) => strip_direct_signature_field(content),
        _ => {}
    }
}

fn strip_direct_signature_field(value: &mut Value) {
    if let Value::Object(object) = value {
        object.remove("signature");
    }
}

pub(super) fn build_upstream_headers(
    request: &DebugHttpRequest,
    provider: &UpstreamProvider,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers {
        if should_skip_forwarded_request_header(name) {
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("Invalid request header name '{}': {error}", name))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("Invalid request header value for '{}': {error}", name))?;
        headers.insert(header_name, header_value);
    }
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    inject_provider_auth(provider, &mut headers)?;
    Ok(headers)
}

fn should_skip_forwarded_request_header(name: &str) -> bool {
    [
        HOST.as_str(),
        CONTENT_LENGTH.as_str(),
        CONNECTION.as_str(),
        "keep-alive",
        "proxy-connection",
        PROXY_AUTHENTICATE.as_str(),
        PROXY_AUTHORIZATION.as_str(),
        TE.as_str(),
        TRAILER.as_str(),
        TRANSFER_ENCODING.as_str(),
        UPGRADE.as_str(),
        AUTHORIZATION.as_str(),
        "x-api-key",
        "x-goog-api-key",
        "x-goog-api-client",
    ]
    .iter()
    .any(|skip| name.eq_ignore_ascii_case(skip))
}

fn inject_provider_auth(
    provider: &UpstreamProvider,
    headers: &mut HeaderMap,
) -> Result<(), String> {
    match provider.cli_key {
        GatewayCliKey::Claude => {
            let value = HeaderValue::from_str(provider.api_key.trim())
                .map_err(|error| format!("Invalid Claude API key header value: {error}"))?;
            headers.insert("x-api-key", value);
            if !headers.contains_key("anthropic-version") {
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
        }
        GatewayCliKey::Codex => {
            let value = HeaderValue::from_str(&format!("Bearer {}", provider.api_key.trim()))
                .map_err(|error| format!("Invalid Codex Authorization header value: {error}"))?;
            headers.insert(AUTHORIZATION, value);
        }
        GatewayCliKey::Gemini => {
            let trimmed = provider.api_key.trim();
            let oauth_token = if trimmed.starts_with("ya29.") {
                Some(trimmed.to_string())
            } else if trimmed.starts_with('{') {
                serde_json::from_str::<Value>(trimmed)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("access_token")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
            } else {
                None
            };
            if let Some(token) = oauth_token {
                let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|error| {
                    format!("Invalid Gemini Authorization header value: {error}")
                })?;
                headers.insert(AUTHORIZATION, value);
                headers.insert(
                    "x-goog-api-client",
                    HeaderValue::from_static("GeminiCLI/1.0"),
                );
            } else {
                let value = HeaderValue::from_str(trimmed)
                    .map_err(|error| format!("Invalid Gemini API key header value: {error}"))?;
                headers.insert("x-goog-api-key", value);
            }
        }
        GatewayCliKey::OpenCode => {
            return Err("OpenCode adapter is intentionally out of scope".to_string())
        }
    }
    Ok(())
}

fn filtered_response_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            if should_skip_forwarded_response_header(name.as_str()) {
                return None;
            }
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn should_skip_forwarded_response_header(name: &str) -> bool {
    [
        CONTENT_LENGTH.as_str(),
        CONNECTION.as_str(),
        "keep-alive",
        "proxy-connection",
        PROXY_AUTHENTICATE.as_str(),
        PROXY_AUTHORIZATION.as_str(),
        TE.as_str(),
        TRAILER.as_str(),
        TRANSFER_ENCODING.as_str(),
        UPGRADE.as_str(),
    ]
    .iter()
    .any(|skip| name.eq_ignore_ascii_case(skip))
}

fn extract_requested_model(request: &DebugHttpRequest, route: &GatewayRoute) -> Option<String> {
    extract_model_from_json_body(&request.body).or_else(|| {
        if route.cli_key == GatewayCliKey::Gemini {
            extract_gemini_model_from_path(&route.forwarded_path)
        } else {
            None
        }
    })
}

fn extract_model_from_json_body(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_gemini_model_from_path(path: &str) -> Option<String> {
    let marker = "/models/";
    let start = path.find(marker)? + marker.len();
    let model_part = &path[start..];
    let end = model_part
        .find(|ch| matches!(ch, ':' | '/' | '?'))
        .unwrap_or(model_part.len());
    Some(model_part[..end].trim().to_string()).filter(|value| !value.is_empty())
}

fn classify_reqwest_error(error: &reqwest::Error) -> GatewayFailureKind {
    if error.is_timeout() {
        GatewayFailureKind::Timeout
    } else {
        GatewayFailureKind::Connection
    }
}

fn classify_status_failure(status_code: u16) -> Option<GatewayFailureKind> {
    match status_code {
        200..=399 => None,
        400 => Some(GatewayFailureKind::UpstreamBadRequest),
        401 | 403 => Some(GatewayFailureKind::Auth),
        404 => Some(GatewayFailureKind::ModelNotFound),
        408 => Some(GatewayFailureKind::Timeout),
        429 => Some(GatewayFailureKind::RateLimit),
        500..=599 => Some(GatewayFailureKind::Upstream5xx),
        _ => Some(GatewayFailureKind::RequestSchema),
    }
}

fn should_retry_failure(kind: GatewayFailureKind) -> bool {
    !matches!(
        kind,
        GatewayFailureKind::RequestSchema
            | GatewayFailureKind::ClientCancelled
            | GatewayFailureKind::GatewayParse
    )
}

fn save_health_registry_if_needed(
    context: &GatewayRuntimeContext,
    registry: Option<&ModelHealthRegistry>,
    changed: bool,
) {
    if !changed {
        return;
    }
    let (Some(paths), Some(registry)) = (context.paths.as_ref(), registry) else {
        return;
    };
    if let Err(error) = registry.save(&paths.model_health_path()) {
        log::warn!("Failed to save proxy gateway model health: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn debug_request(body: &[u8]) -> DebugHttpRequest {
        DebugHttpRequest {
            id: 1,
            peer_addr: "127.0.0.1:50000".parse::<SocketAddr>().unwrap(),
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            version: "HTTP/1.1".to_string(),
            first_line: "POST /anthropic/v1/messages HTTP/1.1".to_string(),
            headers: Vec::new(),
            body: body.to_vec(),
            raw_len: body.len(),
        }
    }

    fn claude_provider(mapping: UpstreamModelMapping) -> UpstreamProvider {
        UpstreamProvider {
            cli_key: GatewayCliKey::Claude,
            id: "p1".to_string(),
            name: "Provider".to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: "key".to_string(),
            sort_index: Some(0),
            model_mapping: mapping,
        }
    }

    #[test]
    fn claude_model_mapping_uses_provider_specific_model_for_standard_name() {
        let provider = claude_provider(UpstreamModelMapping {
            sonnet_model: Some("provider-sonnet".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_upstream_model_id(&debug_request(b"{}"), "claude-sonnet-4-6", &provider),
            "provider-sonnet"
        );
    }

    #[test]
    fn claude_model_mapping_falls_back_to_default_model_when_family_missing() {
        let provider = claude_provider(UpstreamModelMapping {
            default_model: Some("provider-default".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_upstream_model_id(&debug_request(b"{}"), "claude-opus-4-7", &provider),
            "provider-default"
        );
    }

    #[test]
    fn claude_model_mapping_falls_back_to_standard_model_when_no_mapping_exists() {
        let provider = claude_provider(UpstreamModelMapping::default());

        assert_eq!(
            resolve_upstream_model_id(&debug_request(b"{}"), "claude-opus-4-7", &provider),
            "claude-opus-4-7"
        );
    }

    #[test]
    fn claude_reasoning_request_uses_reasoning_model() {
        let provider = claude_provider(UpstreamModelMapping {
            reasoning_model: Some("provider-reasoning".to_string()),
            sonnet_model: Some("provider-sonnet".to_string()),
            ..UpstreamModelMapping::default()
        });
        let request =
            debug_request(br#"{"model":"claude-sonnet-4-6","thinking":{"type":"enabled"}}"#);

        assert_eq!(
            resolve_upstream_model_id(&request, "claude-sonnet-4-6", &provider),
            "provider-reasoning"
        );
    }

    #[test]
    fn upstream_body_rewrites_json_model_only() {
        let request = debug_request(br#"{"model":"claude-sonnet-4-6","messages":[]}"#);

        let body =
            build_upstream_body(&request, "claude-sonnet-4-6", "provider-sonnet", true).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(
            value.get("model").and_then(Value::as_str),
            Some("provider-sonnet")
        );
        assert!(value.get("messages").is_some());
    }

    #[test]
    fn upstream_body_strips_thinking_blocks_when_model_remapped() {
        let request = debug_request(
            br#"{
                "model":"claude-sonnet-4-6",
                "thinking":{"type":"enabled","budget_tokens":1024},
                "messages":[
                    {
                        "role":"assistant",
                        "content":[
                            {"type":"thinking","thinking":"hidden","signature":"sig-a"},
                            {"type":"redacted_thinking","data":"hidden"},
                            {"type":"text","text":"visible","signature":"sig-b","meta":{"signature":"sig-c"}}
                        ]
                    }
                ]
            }"#,
        );

        let body =
            build_upstream_body(&request, "claude-sonnet-4-6", "deepseek-chat", true).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let content = value
            .pointer("/messages/0/content")
            .and_then(Value::as_array)
            .unwrap();

        assert_eq!(
            value.get("model").and_then(Value::as_str),
            Some("deepseek-chat")
        );
        assert!(value.get("thinking").is_none());
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert!(content[0].get("signature").is_none());
        assert_eq!(
            content[0]
                .pointer("/meta/signature")
                .and_then(Value::as_str),
            Some("sig-c")
        );
    }

    #[test]
    fn upstream_body_does_not_strip_nested_business_payload_messages() {
        let request = debug_request(
            br#"{
                "model":"claude-sonnet-4-6",
                "thinking":{"type":"enabled"},
                "metadata":{
                    "messages":[
                        {
                            "content":[
                                {"type":"thinking","thinking":"business data","signature":"keep-me"}
                            ]
                        }
                    ]
                },
                "messages":[
                    {
                        "role":"user",
                        "content":[
                            {
                                "type":"tool_result",
                                "content":{
                                    "messages":[
                                        {
                                            "content":[
                                                {"type":"thinking","thinking":"tool data","signature":"keep-tool"}
                                            ]
                                        }
                                    ]
                                },
                                "signature":"strip-direct"
                            }
                        ]
                    }
                ]
            }"#,
        );

        let body =
            build_upstream_body(&request, "claude-sonnet-4-6", "deepseek-chat", true).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("thinking").is_none());
        assert_eq!(
            value
                .pointer("/metadata/messages/0/content/0/signature")
                .and_then(Value::as_str),
            Some("keep-me")
        );
        assert_eq!(
            value
                .pointer("/messages/0/content/0/content/messages/0/content/0/signature")
                .and_then(Value::as_str),
            Some("keep-tool")
        );
        assert!(value.pointer("/messages/0/content/0/signature").is_none());
    }

    #[test]
    fn upstream_body_preserves_thinking_blocks_when_model_unchanged() {
        let request = debug_request(
            br#"{
                "model":"claude-sonnet-4-6",
                "thinking":{"type":"enabled"},
                "messages":[
                    {
                        "role":"assistant",
                        "content":[
                            {"type":"thinking","thinking":"hidden","signature":"sig-a"},
                            {"type":"text","text":"visible","signature":"sig-b"}
                        ]
                    }
                ]
            }"#,
        );

        let body =
            build_upstream_body(&request, "claude-sonnet-4-6", "claude-sonnet-4-6", true).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let content = value
            .pointer("/messages/0/content")
            .and_then(Value::as_array)
            .unwrap();

        assert!(value.get("thinking").is_some());
        assert_eq!(content.len(), 2);
        assert_eq!(
            content[0].get("type").and_then(Value::as_str),
            Some("thinking")
        );
        assert!(content[0].get("signature").is_some());
        assert!(content[1].get("signature").is_some());
    }

    #[test]
    fn upstream_bad_request_is_retryable_model_failure() {
        assert_eq!(
            classify_status_failure(400),
            Some(GatewayFailureKind::UpstreamBadRequest)
        );
        assert!(should_retry_failure(GatewayFailureKind::UpstreamBadRequest));
        let weight = model_health::classify_failure(GatewayFailureKind::UpstreamBadRequest);
        assert_eq!(weight.scope, model_health::FailureScope::Model);
        assert_eq!(weight.score, 1);
        assert_eq!(weight.category, "upstream_bad_request");
    }

    #[test]
    fn current_provider_retry_respects_per_provider_and_global_limits() {
        assert!(can_retry_current_provider(0, 1, 0, 3));
        assert!(!can_retry_current_provider(1, 1, 1, 3));
        assert!(!can_retry_current_provider(0, 1, 3, 3));
    }
}
