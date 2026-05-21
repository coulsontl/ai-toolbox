use super::debug_log::{log_upstream_request, log_upstream_response};
use super::header_preserving_client::{
    append_preserved_header, send_header_preserving_request, HeaderPreservingResponse,
    PreservedHeader,
};
use super::http_io::{empty_response, json_response, DebugHttpRequest, DebugHttpResponse};
use super::providers::{load_candidate_providers, UpstreamModelMapping, UpstreamProvider};
use super::routes::{build_target_url, match_gateway_route, split_request_target, GatewayRoute};
use super::GatewayRuntimeContext;
use super::{cache_injector, thinking_budget};
use crate::coding::proxy_gateway::model_health::{self, GatewayFailureKind};
#[cfg(test)]
use crate::coding::proxy_gateway::types::ProviderGatewayMeta;
use crate::coding::proxy_gateway::types::{
    GatewayCliKey, GatewayFailoverEvent, ProviderModelHealthKey,
};
use crate::coding::proxy_gateway::usage_parser::{from_response_body, TokenUsage};
use crate::db::SqliteDbState;
use crate::http_client::{self, ProxyMode};
use futures_util::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_ENCODING, AUTHORIZATION, CONNECTION, CONTENT_LENGTH,
    CONTENT_TYPE, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING,
    UPGRADE,
};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::time::Duration;
use tauri::Emitter;

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

struct FirstChunkStream {
    first_chunk: Option<Vec<u8>>,
    inner: super::http_io::DebugBodyStream,
}

impl Unpin for FirstChunkStream {}

impl futures_util::Stream for FirstChunkStream {
    type Item = Result<Vec<u8>, String>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let Some(chunk) = self.first_chunk.take() {
            return std::task::Poll::Ready(Some(Ok(chunk)));
        }
        self.inner.as_mut().poll_next(cx)
    }
}

#[derive(Clone)]
pub(super) struct UpstreamHeaders {
    map: HeaderMap,
    preserved: Vec<PreservedHeader>,
}

#[cfg(test)]
impl UpstreamHeaders {
    pub(super) fn contains_key<K>(&self, key: K) -> bool
    where
        K: reqwest::header::AsHeaderName,
    {
        self.map.contains_key(key)
    }

    pub(super) fn get<K>(&self, key: K) -> Option<&HeaderValue>
    where
        K: reqwest::header::AsHeaderName,
    {
        self.map.get(key)
    }
}

enum UpstreamResponse {
    Reqwest(reqwest::Response),
    HeaderPreserving(HeaderPreservingResponse),
}

impl UpstreamResponse {
    fn status(&self) -> reqwest::StatusCode {
        match self {
            Self::Reqwest(response) => response.status(),
            Self::HeaderPreserving(response) => response.status(),
        }
    }

    fn headers(&self) -> &HeaderMap {
        match self {
            Self::Reqwest(response) => response.headers(),
            Self::HeaderPreserving(response) => response.headers(),
        }
    }

    async fn bytes(self) -> Result<Vec<u8>, GatewayForwardError> {
        match self {
            Self::Reqwest(response) => {
                response
                    .bytes()
                    .await
                    .map(|bytes| bytes.to_vec())
                    .map_err(|error| GatewayForwardError {
                        message: format!("Failed to read upstream response body: {error}"),
                        kind: classify_reqwest_error(&error),
                        upstream_request_body: None,
                    })
            }
            Self::HeaderPreserving(response) => response
                .bytes()
                .await
                .map_err(|error| GatewayForwardError::new(error, GatewayFailureKind::Connection)),
        }
    }

    fn bytes_stream(self) -> super::http_io::DebugBodyStream {
        match self {
            Self::Reqwest(response) => {
                let body_stream = response.bytes_stream().map(|chunk| {
                    chunk
                        .map(|bytes| bytes.to_vec())
                        .map_err(|error| format!("Failed to read upstream response body: {error}"))
                });
                Box::pin(body_stream)
            }
            Self::HeaderPreserving(response) => Box::pin(response.bytes_stream()),
        }
    }
}

async fn validate_streaming_first_chunk(
    response: &mut DebugHttpResponse,
    first_byte_timeout_secs: u64,
) -> Result<(), GatewayForwardError> {
    let Some(mut body_stream) = response.body_stream.take() else {
        return Ok(());
    };
    let timeout_duration = Duration::from_secs(first_byte_timeout_secs.max(1));
    loop {
        let next_chunk = tokio::time::timeout(timeout_duration, body_stream.next())
            .await
            .map_err(|_| {
                GatewayForwardError::new(
                    format!(
                        "Timed out waiting for upstream streaming first chunk after {} seconds",
                        timeout_duration.as_secs()
                    ),
                    GatewayFailureKind::Timeout,
                )
            })?;
        match next_chunk {
            Some(Ok(chunk)) if !chunk.is_empty() => {
                response.body_stream = Some(Box::pin(FirstChunkStream {
                    first_chunk: Some(chunk),
                    inner: body_stream,
                }));
                return Ok(());
            }
            Some(Ok(_)) => continue,
            Some(Err(error)) => {
                return Err(GatewayForwardError::new(
                    error,
                    GatewayFailureKind::Connection,
                ));
            }
            None => {
                return Err(GatewayForwardError::new(
                    "Upstream streaming response ended before first chunk",
                    GatewayFailureKind::Timeout,
                ));
            }
        }
    }
}

fn emit_failover_event_if_needed(
    context: &GatewayRuntimeContext,
    cli_key: GatewayCliKey,
    previous_response: Option<&DebugHttpResponse>,
    response: &DebugHttpResponse,
) {
    if !response.failover {
        return;
    }
    let Some(app_handle) = context.app_handle.as_ref() else {
        return;
    };
    let Some(previous_response) = previous_response else {
        return;
    };
    let (Some(from_provider_id), Some(to_provider_id)) = (
        previous_response.provider_id.clone(),
        response.provider_id.clone(),
    ) else {
        return;
    };
    let payload = GatewayFailoverEvent {
        cli_key,
        from_provider_id,
        from_provider_name: previous_response.provider_name.clone(),
        to_provider_id,
        to_provider_name: response.provider_name.clone(),
    };
    if let Err(error) = app_handle.emit("gateway-failover", payload) {
        log::warn!("Failed to emit gateway failover event: {error}");
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

    if is_cli_route_probe(request, &route) {
        let mut response = empty_response(
            204,
            "No Content",
            route.route_name,
            "local CLI route probe endpoint",
        );
        response.cli_key = Some(route.cli_key);
        return response;
    }

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

fn is_cli_route_probe(request: &DebugHttpRequest, route: &GatewayRoute) -> bool {
    if !matches!(request.method.as_str(), "GET" | "HEAD") {
        return false;
    }
    match route.cli_key {
        GatewayCliKey::Claude => route.forwarded_path == "/",
        GatewayCliKey::Codex => route.forwarded_path == "/v1",
        GatewayCliKey::Gemini => route.forwarded_path == "/v1beta",
        GatewayCliKey::OpenCode => false,
    }
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
    let app_config = settings.effective_app_config(route.cli_key);
    refresh_health_registry(context);
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

        if !is_model_available(context, &health_key) {
            skipped_by_health.push(provider.name.clone());
            continue;
        }

        attempted_provider_count = attempted_provider_count.saturating_add(1);
        let mut provider_retry_count = 0_u32;
        loop {
            if attempt_count > 0 && retry_count >= app_config.max_retry_count {
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
                settings.thinking_budget_rectifier_enabled,
                settings.cache_injection_enabled,
                app_config.non_streaming_timeout_secs,
            )
            .await
            {
                Ok(mut response) => {
                    response.cli_key = Some(route.cli_key);
                    response.provider_id = Some(provider.id.clone());
                    response.provider_name = Some(provider.name.clone());
                    response.provider_type = provider.meta.provider_type.clone();
                    response.cost_multiplier = Some(provider.meta.cost_multiplier.clone());
                    response.pricing_model_source =
                        Some(provider.meta.pricing_model_source.clone());
                    response.requested_model = Some(requested_model.clone());
                    response.upstream_model_id = Some(upstream_model_id.clone());
                    response.attempt_count = attempt_count;
                    response.provider_attempt_count = provider_retry_count.saturating_add(1);
                    response.failover = attempted_provider_count > 1;

                    if response.is_streaming {
                        match validate_streaming_first_chunk(
                            &mut response,
                            app_config.streaming_first_byte_timeout_secs,
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(error) => {
                                let failure_kind = error.kind;
                                let category =
                                    model_health::classify_failure(failure_kind).category;
                                health_changed |=
                                    record_health_failure(context, &health_key, failure_kind);
                                last_failure_response =
                                    Some(streaming_first_chunk_failure_response(
                                        route,
                                        &provider,
                                        &requested_model,
                                        &upstream_model_id,
                                        response,
                                        error,
                                        category,
                                        attempt_count,
                                        provider_retry_count.saturating_add(1),
                                        attempted_provider_count > 1,
                                    ));
                                if can_retry_current_provider(
                                    provider_retry_count,
                                    app_config.per_provider_retry_count,
                                    retry_count,
                                    app_config.max_retry_count,
                                ) {
                                    provider_retry_count = provider_retry_count.saturating_add(1);
                                    continue;
                                }
                                continue 'providers;
                            }
                        }
                    }

                    if let Some(failure_kind) = classify_status_failure(response.status_code) {
                        let category = model_health::classify_failure(failure_kind).category;
                        response.error_category = Some(category.to_string());
                        health_changed |= record_health_failure(context, &health_key, failure_kind);
                        if should_retry_failure(failure_kind) {
                            last_failure_response = Some(response);
                            if can_retry_current_provider(
                                provider_retry_count,
                                app_config.per_provider_retry_count,
                                retry_count,
                                app_config.max_retry_count,
                            ) {
                                provider_retry_count = provider_retry_count.saturating_add(1);
                                continue;
                            }
                            continue 'providers;
                        }
                    } else {
                        health_changed |= record_health_success(context, &health_key);
                    }

                    save_health_registry_if_needed(context, health_changed);
                    emit_failover_event_if_needed(
                        context,
                        route.cli_key,
                        last_failure_response.as_ref(),
                        &response,
                    );
                    return response;
                }
                Err(error) => {
                    let category = model_health::classify_failure(error.kind).category;
                    health_changed |= record_health_failure(context, &health_key, error.kind);
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
                    response.provider_type = provider.meta.provider_type.clone();
                    response.cost_multiplier = Some(provider.meta.cost_multiplier.clone());
                    response.pricing_model_source =
                        Some(provider.meta.pricing_model_source.clone());
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
                        app_config.per_provider_retry_count,
                        retry_count,
                        app_config.max_retry_count,
                    ) {
                        provider_retry_count = provider_retry_count.saturating_add(1);
                        continue;
                    }
                    continue 'providers;
                }
            }
        }
    }

    save_health_registry_if_needed(context, health_changed);
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
    thinking_budget_rectifier_enabled: bool,
    cache_injection_enabled: bool,
    non_streaming_timeout_secs: u64,
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
        cache_injection_enabled,
        route.cli_key,
    )?;
    let upstream_body_snapshot = upstream_body.clone();

    let client =
        http_client::client_with_timeout_no_compression(db, non_streaming_timeout_secs.max(1))
            .await
            .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::Connection))?;
    let header_preserving_proxy = header_preserving_proxy(db).await;

    let response = send_request_once(
        request,
        provider,
        &client,
        method.clone(),
        &upstream_url,
        headers.clone(),
        upstream_body.clone(),
        non_streaming_timeout_secs.max(1),
        header_preserving_proxy.clone(),
    )
    .await
    .map_err(|mut error| {
        error.upstream_request_body = Some(upstream_body_snapshot.clone());
        error
    })?;

    let status = response.status();
    if thinking_budget_rectifier_enabled
        && route.cli_key == GatewayCliKey::Claude
        && !should_stream_response(
            request,
            route,
            response.headers(),
            response.status().as_u16(),
        )
        && (400..500).contains(&status.as_u16())
    {
        let status_code = status.as_u16();
        let status_text = status.canonical_reason().unwrap_or("Unknown").to_string();
        let response_headers = filtered_response_headers(response.headers());
        let body = response.bytes().await.map_err(|mut error| {
            error.upstream_request_body = Some(upstream_body_snapshot.clone());
            error
        })?;
        if thinking_budget::should_rectify_thinking_budget(status_code, &body) {
            if let Some(rectified_body) =
                thinking_budget::rectify_thinking_budget(&upstream_body_snapshot)
            {
                let response = send_request_once(
                    request,
                    provider,
                    &client,
                    method,
                    &upstream_url,
                    headers,
                    rectified_body.clone(),
                    non_streaming_timeout_secs.max(1),
                    header_preserving_proxy.clone(),
                )
                .await
                .map_err(|mut error| {
                    error.upstream_request_body = Some(rectified_body.clone());
                    error
                })?;
                return build_gateway_response(
                    request,
                    route,
                    provider,
                    response,
                    rectified_body,
                    upstream_url.to_string(),
                )
                .await;
            }
        }
        return Ok(buffered_gateway_response(
            status_code,
            status_text,
            response_headers,
            body,
            provider,
            route,
            upstream_body_snapshot,
            upstream_url.to_string(),
        ));
    }

    build_gateway_response(
        request,
        route,
        provider,
        response,
        upstream_body_snapshot,
        upstream_url.to_string(),
    )
    .await
}

async fn send_request_once(
    request: &DebugHttpRequest,
    provider: &UpstreamProvider,
    client: &reqwest::Client,
    method: reqwest::Method,
    upstream_url: &reqwest::Url,
    headers: UpstreamHeaders,
    upstream_body: Vec<u8>,
    timeout_secs: u64,
    header_preserving_proxy: Option<Option<String>>,
) -> Result<UpstreamResponse, GatewayForwardError> {
    log_upstream_request(
        request,
        provider,
        upstream_url,
        &headers.map,
        &upstream_body,
    );
    if should_use_header_preserving_raw(upstream_url) {
        if let Some(proxy_url) = header_preserving_proxy {
            match send_header_preserving_request(
                upstream_url,
                method.clone(),
                &headers.preserved,
                upstream_body.clone(),
                Duration::from_secs(timeout_secs.max(1)),
                proxy_url.as_deref(),
            )
            .await
            {
                Ok(response) => return Ok(UpstreamResponse::HeaderPreserving(response)),
                Err(error) => {
                    log::warn!(
                    "Header-preserving upstream request failed; falling back to reqwest: {error}"
                );
                }
            }
        }
    }
    let response = client
        .request(method, upstream_url.clone())
        .headers(headers.map)
        .body(upstream_body)
        .send()
        .await
        .map_err(|error| GatewayForwardError {
            message: format!("Failed to send upstream request: {error}"),
            kind: classify_reqwest_error(&error),
            upstream_request_body: None,
        })?;
    Ok(UpstreamResponse::Reqwest(response))
}

async fn build_gateway_response(
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    response: UpstreamResponse,
    upstream_body_snapshot: Vec<u8>,
    upstream_url: String,
) -> Result<DebugHttpResponse, GatewayForwardError> {
    let status = response.status();
    let response_headers = filtered_response_headers(response.headers());
    let should_stream = should_stream_response(request, route, response.headers(), status.as_u16());

    if should_stream {
        let body_stream = response.bytes_stream();
        let gateway_response = DebugHttpResponse {
            status_code: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("Unknown").to_string(),
            headers: response_headers,
            body: Vec::new(),
            body_stream: Some(Box::pin(body_stream)),
            response_body_bytes: 0,
            token_usage: TokenUsage::default(),
            first_token_ms: None,
            is_streaming: true,
            cli_key: Some(provider.cli_key),
            route_name: route.route_name.to_string(),
            provider_id: Some(provider.id.clone()),
            provider_name: Some(provider.name.clone()),
            requested_model: None,
            upstream_model_id: None,
            upstream_request_body: Some(upstream_body_snapshot),
            provider_type: provider.meta.provider_type.clone(),
            cost_multiplier: Some(provider.meta.cost_multiplier.clone()),
            pricing_model_source: Some(provider.meta.pricing_model_source.clone()),
            upstream_url: Some(upstream_url),
            error_category: None,
            attempt_count: 1,
            provider_attempt_count: 1,
            failover: false,
            note: format!(
                "streaming forwarded to provider id={} name={}",
                provider.id, provider.name
            ),
        };
        log_upstream_response(request, &gateway_response);
        return Ok(gateway_response);
    }

    let body = response.bytes().await.map_err(|mut error| {
        error.upstream_request_body = Some(upstream_body_snapshot.clone());
        error
    })?;
    let token_usage = from_response_body(provider.cli_key, &body);

    let gateway_response = DebugHttpResponse {
        status_code: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("Unknown").to_string(),
        headers: response_headers,
        response_body_bytes: body.len() as u64,
        body,
        body_stream: None,
        token_usage,
        first_token_ms: None,
        is_streaming: false,
        cli_key: Some(provider.cli_key),
        route_name: route.route_name.to_string(),
        provider_id: Some(provider.id.clone()),
        provider_name: Some(provider.name.clone()),
        requested_model: None,
        upstream_model_id: None,
        upstream_request_body: Some(upstream_body_snapshot),
        provider_type: provider.meta.provider_type.clone(),
        cost_multiplier: Some(provider.meta.cost_multiplier.clone()),
        pricing_model_source: Some(provider.meta.pricing_model_source.clone()),
        upstream_url: Some(upstream_url),
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

fn buffered_gateway_response(
    status_code: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    provider: &UpstreamProvider,
    route: &GatewayRoute,
    upstream_body_snapshot: Vec<u8>,
    upstream_url: String,
) -> DebugHttpResponse {
    let token_usage = from_response_body(provider.cli_key, &body);
    DebugHttpResponse {
        status_code,
        status_text,
        headers,
        response_body_bytes: body.len() as u64,
        body,
        body_stream: None,
        token_usage,
        first_token_ms: None,
        is_streaming: false,
        cli_key: Some(provider.cli_key),
        route_name: route.route_name.to_string(),
        provider_id: Some(provider.id.clone()),
        provider_name: Some(provider.name.clone()),
        provider_type: provider.meta.provider_type.clone(),
        cost_multiplier: Some(provider.meta.cost_multiplier.clone()),
        pricing_model_source: Some(provider.meta.pricing_model_source.clone()),
        requested_model: None,
        upstream_model_id: None,
        upstream_request_body: Some(upstream_body_snapshot),
        upstream_url: Some(upstream_url),
        error_category: None,
        attempt_count: 1,
        provider_attempt_count: 1,
        failover: false,
        note: format!(
            "forwarded to provider id={} name={}",
            provider.id, provider.name
        ),
    }
}

fn streaming_first_chunk_failure_response(
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    requested_model: &str,
    upstream_model_id: &str,
    mut response: DebugHttpResponse,
    error: GatewayForwardError,
    category: &str,
    attempt_count: u32,
    provider_attempt_count: u32,
    failover: bool,
) -> DebugHttpResponse {
    let mut failure_response = json_response(
        502,
        "Bad Gateway",
        json!({
            "error": "upstream_stream_first_chunk_failed",
            "message": error.message,
        }),
        route.route_name,
        response.upstream_url.clone(),
        "upstream streaming failed before first chunk",
    );
    failure_response.cli_key = Some(route.cli_key);
    failure_response.provider_id = Some(provider.id.clone());
    failure_response.provider_name = Some(provider.name.clone());
    failure_response.provider_type = provider.meta.provider_type.clone();
    failure_response.cost_multiplier = Some(provider.meta.cost_multiplier.clone());
    failure_response.pricing_model_source = Some(provider.meta.pricing_model_source.clone());
    failure_response.requested_model = Some(requested_model.to_string());
    failure_response.upstream_model_id = Some(upstream_model_id.to_string());
    failure_response.upstream_request_body = response.upstream_request_body.take();
    failure_response.error_category = Some(category.to_string());
    failure_response.attempt_count = attempt_count;
    failure_response.provider_attempt_count = provider_attempt_count;
    failure_response.failover = failover;
    failure_response
}

fn should_stream_response(
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    headers: &HeaderMap,
    status_code: u16,
) -> bool {
    if !(200..400).contains(&status_code) {
        return false;
    }
    let response_is_sse = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
    response_is_sse || request_declares_streaming(request) || route_declares_streaming(route)
}

fn request_declares_streaming(request: &DebugHttpRequest) -> bool {
    serde_json::from_slice::<Value>(&request.body)
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn route_declares_streaming(route: &GatewayRoute) -> bool {
    route.cli_key == GatewayCliKey::Gemini
        && (route.forwarded_path.contains(":streamGenerateContent")
            || route
                .query
                .as_deref()
                .is_some_and(|query| query.contains("alt=sse")))
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
    cache_injection_enabled: bool,
    cli_key: GatewayCliKey,
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
    if cache_injection_enabled && cli_key == GatewayCliKey::Claude {
        cache_injector::inject_cache_control(&mut value);
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
) -> Result<UpstreamHeaders, String> {
    let mut headers = HeaderMap::new();
    let mut preserved = Vec::new();
    for (name, value) in &request.headers {
        if should_skip_forwarded_request_header(name) {
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("Invalid request header name '{}': {error}", name))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("Invalid request header value for '{}': {error}", name))?;
        headers.insert(header_name, header_value.clone());
        preserved.push(PreservedHeader {
            name: name.clone(),
            value: header_value,
        });
    }
    append_preserved_header(
        &mut headers,
        &mut preserved,
        ACCEPT_ENCODING.as_str(),
        HeaderValue::from_static("identity"),
    )?;
    inject_provider_auth(provider, &mut headers, &mut preserved)?;
    Ok(UpstreamHeaders {
        map: headers,
        preserved,
    })
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
        ACCEPT_ENCODING.as_str(),
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
    preserved: &mut Vec<PreservedHeader>,
) -> Result<(), String> {
    match provider.cli_key {
        GatewayCliKey::Claude => {
            let value = HeaderValue::from_str(provider.api_key.trim())
                .map_err(|error| format!("Invalid Claude API key header value: {error}"))?;
            append_preserved_header(headers, preserved, "x-api-key", value)?;
            if !headers.contains_key("anthropic-version") {
                append_preserved_header(
                    headers,
                    preserved,
                    "anthropic-version",
                    HeaderValue::from_static("2023-06-01"),
                )?;
            }
        }
        GatewayCliKey::Codex => {
            let value = HeaderValue::from_str(&format!("Bearer {}", provider.api_key.trim()))
                .map_err(|error| format!("Invalid Codex Authorization header value: {error}"))?;
            append_preserved_header(headers, preserved, AUTHORIZATION.as_str(), value)?;
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
                append_preserved_header(headers, preserved, AUTHORIZATION.as_str(), value)?;
                append_preserved_header(
                    headers,
                    preserved,
                    "x-goog-api-client",
                    HeaderValue::from_static("GeminiCLI/1.0"),
                )?;
            } else {
                let value = HeaderValue::from_str(trimmed)
                    .map_err(|error| format!("Invalid Gemini API key header value: {error}"))?;
                append_preserved_header(headers, preserved, "x-goog-api-key", value)?;
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

async fn header_preserving_proxy(db: &SqliteDbState) -> Option<Option<String>> {
    let Ok((proxy_mode, proxy_url)) = http_client::get_proxy_from_settings(db).await else {
        return Some(None);
    };
    match proxy_mode {
        ProxyMode::Direct => Some(None),
        ProxyMode::Custom => {
            let normalized = normalize_proxy_url_for_header_preserving_path(&proxy_url)?;
            Some(Some(normalized))
        }
        // System proxy detection is platform-specific. Keep the existing reqwest path so
        // users who rely on OS proxy settings do not get silently bypassed.
        ProxyMode::System => None,
    }
}

fn normalize_proxy_url_for_header_preserving_path(proxy_url: &str) -> Option<String> {
    let trimmed = proxy_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    if normalized.starts_with("http://") || normalized.starts_with("https://") {
        Some(normalized)
    } else {
        None
    }
}

fn should_use_header_preserving_raw(upstream_url: &reqwest::Url) -> bool {
    let Some(host) = upstream_url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return false;
    }
    if host
        .parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
    {
        return false;
    }
    true
}

fn refresh_health_registry(context: &GatewayRuntimeContext) {
    let Some(registry) = context.health_registry.as_ref() else {
        return;
    };
    if let Ok(mut registry) = registry.lock() {
        registry.refresh_due_cooldowns(chrono::Utc::now());
    }
}

fn is_model_available(
    context: &GatewayRuntimeContext,
    health_key: &ProviderModelHealthKey,
) -> bool {
    let Some(registry) = context.health_registry.as_ref() else {
        return true;
    };
    registry
        .lock()
        .map(|registry| registry.is_model_available(health_key, chrono::Utc::now()))
        .unwrap_or(true)
}

fn record_health_failure(
    context: &GatewayRuntimeContext,
    health_key: &ProviderModelHealthKey,
    kind: GatewayFailureKind,
) -> bool {
    let Some(registry) = context.health_registry.as_ref() else {
        return false;
    };
    registry
        .lock()
        .map(|mut registry| registry.record_failure(health_key, kind, chrono::Utc::now()))
        .unwrap_or(false)
}

fn record_health_success(
    context: &GatewayRuntimeContext,
    health_key: &ProviderModelHealthKey,
) -> bool {
    let Some(registry) = context.health_registry.as_ref() else {
        return false;
    };
    registry
        .lock()
        .map(|mut registry| registry.record_success(health_key))
        .unwrap_or(false)
}

fn save_health_registry_if_needed(context: &GatewayRuntimeContext, changed: bool) {
    if !changed {
        return;
    }
    context.save_health_registry_async();
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
            meta: ProviderGatewayMeta::default(),
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

        let body = build_upstream_body(
            &request,
            "claude-sonnet-4-6",
            "provider-sonnet",
            true,
            false,
            GatewayCliKey::Claude,
        )
        .unwrap();
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

        let body = build_upstream_body(
            &request,
            "claude-sonnet-4-6",
            "deepseek-chat",
            true,
            false,
            GatewayCliKey::Claude,
        )
        .unwrap();
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

        let body = build_upstream_body(
            &request,
            "claude-sonnet-4-6",
            "deepseek-chat",
            true,
            false,
            GatewayCliKey::Claude,
        )
        .unwrap();
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

        let body = build_upstream_body(
            &request,
            "claude-sonnet-4-6",
            "claude-sonnet-4-6",
            true,
            false,
            GatewayCliKey::Claude,
        )
        .unwrap();
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
