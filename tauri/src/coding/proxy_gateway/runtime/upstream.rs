use super::header_preserving_client::{
    append_preserved_header, send_header_preserving_request, HeaderPreservingResponse,
    PreservedHeader,
};
use super::http_io::{
    empty_response, json_response, DebugBodyStream, DebugHttpRequest, DebugHttpResponse,
    SharedBodySnapshot,
};
use super::providers::{ProviderAuthStrategy, UpstreamModelMapping, UpstreamProvider};
use super::routes::{build_target_url, match_gateway_route, split_request_target, GatewayRoute};
use super::GatewayRuntimeContext;
use super::{cache_injector, thinking_budget};
use crate::coding::proxy_gateway::model_health::{self, GatewayFailureKind};
use crate::coding::proxy_gateway::transformer::{
    convert_error_response_body, convert_request_body_with_context,
    convert_response_body_with_context, convert_sse_stream_with_context, AiProtocol,
    ConversionContext, ConversionRoute,
};
#[cfg(test)]
use crate::coding::proxy_gateway::types::ProviderGatewayMeta;
use crate::coding::proxy_gateway::types::{
    GatewayCliKey, GatewayFailoverEvent, GatewayProviderAttempt, GatewayProxyMode,
    ProviderModelHealthKey,
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
use std::borrow::Cow;
use std::net::IpAddr;
use std::time::Duration;
use tauri::Emitter;

const ONE_M_CONTEXT_MARKER: &str = "[1m]";
const ENCODED_ONE_M_CONTEXT_MARKER: &str = "%5b1m%5d";
const UNSUPPORTED_IMAGE_MARKER: &str = "[Unsupported Image]";

#[derive(Debug)]
struct GatewayForwardError {
    message: String,
    kind: GatewayFailureKind,
    upstream_request_body: Option<Vec<u8>>,
    upstream_response_body: Option<Vec<u8>>,
    upstream_response_body_bytes: u64,
}

impl GatewayForwardError {
    fn new(message: impl Into<String>, kind: GatewayFailureKind) -> Self {
        Self {
            message: message.into(),
            kind,
            upstream_request_body: None,
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
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

fn snapshot_response_stream(
    inner: DebugBodyStream,
    snapshot: SharedBodySnapshot,
) -> DebugBodyStream {
    Box::pin(inner.map(move |chunk_result| {
        if let Ok(chunk) = &chunk_result {
            snapshot.push(chunk);
        }
        chunk_result
    }))
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
                        upstream_response_body: None,
                        upstream_response_body_bytes: 0,
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
    let provider_candidates = match context.load_candidate_providers(db, route.cli_key).await {
        Ok(provider_candidates) if !provider_candidates.providers.is_empty() => provider_candidates,
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
    let apply_family_model_mapping = !provider_candidates
        .selection
        .as_ref()
        .is_some_and(|selection| selection.mode == GatewayProxyMode::Single);
    let providers = provider_candidates.providers;

    let settings = context.settings_snapshot();
    let app_config = settings.effective_app_config(route.cli_key);
    let upstream_response_snapshot_limit = settings
        .store_response_body
        .then(|| settings.log_max_body_size_kb.saturating_mul(1024) as usize);
    refresh_health_registry(context);
    let mut health_changed = false;
    let mut attempt_count = 0_u32;
    let mut retry_count = 0_u32;
    let mut attempted_provider_count = 0_u32;
    let mut last_failure_response = None;
    let mut provider_attempts = Vec::new();
    let mut skipped_by_health = Vec::new();
    let is_single_provider = providers.len() == 1;

    'providers: for provider in providers {
        let upstream_model_id = resolve_upstream_model_id(
            request,
            &requested_model,
            &provider,
            apply_family_model_mapping,
        );
        let health_key = ProviderModelHealthKey {
            cli_key: route.cli_key,
            provider_id: provider.id.clone(),
            upstream_model_id: upstream_model_id.clone(),
        };

        // 单渠道代理跳过健康过滤，始终尝试转发。
        if !is_single_provider && !is_model_available(context, &health_key) {
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
                upstream_response_snapshot_limit,
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
                                let failure_response = streaming_first_chunk_failure_response(
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
                                );
                                provider_attempts.push(provider_attempt_log(&failure_response));
                                last_failure_response = Some(failure_response);
                                if can_retry_current_provider(
                                    provider_retry_count,
                                    app_config.per_provider_retry_count,
                                    retry_count,
                                    app_config.max_retry_count,
                                ) {
                                    provider_retry_count = provider_retry_count.saturating_add(1);
                                    wait_before_retry(app_config.retry_interval_secs).await;
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
                            provider_attempts.push(provider_attempt_log(&response));
                            last_failure_response = Some(response);
                            if can_retry_current_provider(
                                provider_retry_count,
                                app_config.per_provider_retry_count,
                                retry_count,
                                app_config.max_retry_count,
                            ) {
                                provider_retry_count = provider_retry_count.saturating_add(1);
                                wait_before_retry(app_config.retry_interval_secs).await;
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
                    provider_attempts.push(provider_attempt_log(&response));
                    response.provider_attempts = provider_attempts;
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
                    response.upstream_response_body = error.upstream_response_body;
                    response.upstream_response_body_bytes = error.upstream_response_body_bytes;
                    response.error_category = Some(category.to_string());
                    response.attempt_count = attempt_count;
                    response.provider_attempt_count = provider_retry_count.saturating_add(1);
                    response.failover = attempted_provider_count > 1;
                    provider_attempts.push(provider_attempt_log(&response));
                    last_failure_response = Some(response);
                    if can_retry_current_provider(
                        provider_retry_count,
                        app_config.per_provider_retry_count,
                        retry_count,
                        app_config.max_retry_count,
                    ) {
                        provider_retry_count = provider_retry_count.saturating_add(1);
                        wait_before_retry(app_config.retry_interval_secs).await;
                        continue;
                    }
                    continue 'providers;
                }
            }
        }
    }

    save_health_registry_if_needed(context, health_changed);
    if let Some(mut response) = last_failure_response {
        response.provider_attempts = provider_attempts;
        return response;
    }

    let mut response = json_response(
        503,
        "Service Unavailable",
        json!({
            "error": "model_temporarily_unavailable",
            "message": "All provider candidates for this model are cooling down; no upstream request was attempted.",
            "skipped_providers": skipped_by_health,
        }),
        route.route_name,
        None,
        "all provider candidates are cooling down",
    );
    response.cli_key = Some(route.cli_key);
    response.requested_model = Some(requested_model);
    response.error_category = Some("cooling_down".to_string());
    response.provider_attempts = provider_attempts;
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
    upstream_response_snapshot_limit: Option<usize>,
) -> Result<DebugHttpResponse, GatewayForwardError> {
    let source_protocol = source_protocol_from_route(route);
    let conversion_route =
        source_protocol.and_then(|source_protocol| conversion_route(source_protocol, provider));
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
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })?;
    let prepared_upstream_body = build_upstream_body_for_provider(
        request,
        requested_model,
        upstream_model_id,
        false,
        cache_injection_enabled,
        route.cli_key,
        provider.target_protocol,
        conversion_route,
        provider.meta.provider_type.as_deref(),
        route_declares_streaming(route),
    )?;
    let upstream_body = prepared_upstream_body.body;
    let conversion_context = prepared_upstream_body.conversion_context;
    let upstream_body_snapshot = upstream_body.clone();
    let target_streaming = request_declares_streaming(request) || route_declares_streaming(route);
    let forwarded_path = upstream_forwarded_path(
        route,
        provider,
        conversion_route,
        upstream_model_id,
        target_streaming,
    );
    let upstream_url = build_provider_target_url(
        provider,
        forwarded_path.as_ref(),
        route.query.as_deref(),
        conversion_route,
        target_streaming,
    )
    .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::GatewayParse))?;

    let client =
        http_client::client_with_timeout_no_compression(db, non_streaming_timeout_secs.max(1))
            .await
            .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::Connection))?;
    let header_preserving_proxy = header_preserving_proxy(db).await;

    let response = send_request_once(
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
    let should_attempt_thinking_signature_rectifier = should_attempt_thinking_signature_rectifier(
        thinking_rectifier_enabled,
        request,
        route,
        response.headers(),
        status.as_u16(),
    );
    let should_attempt_thinking_budget_rectifier = should_attempt_thinking_budget_rectifier(
        thinking_budget_rectifier_enabled,
        provider.target_protocol,
        request,
        route,
        response.headers(),
        status.as_u16(),
    );
    let should_attempt_unsupported_media_rectifier =
        should_attempt_unsupported_media_rectifier(status.as_u16());
    if should_attempt_thinking_signature_rectifier
        || should_attempt_thinking_budget_rectifier
        || should_attempt_unsupported_media_rectifier
    {
        let status_code = status.as_u16();
        let status_text = status.canonical_reason().unwrap_or("Unknown").to_string();
        let mut response_headers = filtered_response_headers(response.headers());
        let body = response.bytes().await.map_err(|mut error| {
            error.upstream_request_body = Some(upstream_body_snapshot.clone());
            error
        })?;

        if should_attempt_thinking_signature_rectifier
            && should_rectify_thinking_signature(status_code, &body)
        {
            if let Some(rectified) = build_thinking_signature_rectified_upstream_body(
                request,
                requested_model,
                upstream_model_id,
                cache_injection_enabled,
                route,
                provider.target_protocol,
                conversion_route,
                provider.meta.provider_type.as_deref(),
                route_declares_streaming(route),
                &upstream_body_snapshot,
            )? {
                let rectified_body = rectified.body;
                let rectified_context = rectified.conversion_context;
                let response = send_request_once(
                    &client,
                    method.clone(),
                    &upstream_url,
                    headers.clone(),
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
                    conversion_route,
                    Some(rectified_context),
                    upstream_response_snapshot_limit,
                )
                .await;
            }
        }

        if should_attempt_thinking_budget_rectifier
            && thinking_budget::should_rectify_thinking_budget(status_code, &body)
        {
            if let Some(rectified_body) =
                thinking_budget::rectify_thinking_budget(&upstream_body_snapshot)
            {
                let response = send_request_once(
                    &client,
                    method.clone(),
                    &upstream_url,
                    headers.clone(),
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
                    conversion_route,
                    Some(conversion_context.clone()),
                    upstream_response_snapshot_limit,
                )
                .await;
            }
        }

        if should_attempt_unsupported_media_rectifier
            && should_rectify_unsupported_media(status_code, &body)
        {
            if let Some(rectified_body) =
                build_unsupported_media_rectified_body(&upstream_body_snapshot)?
            {
                let response = send_request_once(
                    &client,
                    method.clone(),
                    &upstream_url,
                    headers.clone(),
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
                    conversion_route,
                    Some(conversion_context.clone()),
                    upstream_response_snapshot_limit,
                )
                .await;
            }
        }
        let upstream_response_body = body.clone();
        let body = convert_buffered_error_body(conversion_route, body, &mut response_headers);
        let stored_upstream_response_body =
            (upstream_response_body != body).then_some(upstream_response_body);
        return Ok(buffered_gateway_response(
            status_code,
            status_text,
            response_headers,
            body,
            provider,
            route,
            upstream_body_snapshot,
            stored_upstream_response_body,
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
        conversion_route,
        Some(conversion_context),
        upstream_response_snapshot_limit,
    )
    .await
}

async fn send_request_once(
    client: &reqwest::Client,
    method: reqwest::Method,
    upstream_url: &reqwest::Url,
    headers: UpstreamHeaders,
    upstream_body: Vec<u8>,
    timeout_secs: u64,
    header_preserving_proxy: Option<Option<String>>,
) -> Result<UpstreamResponse, GatewayForwardError> {
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
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
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
    conversion_route: Option<ConversionRoute>,
    conversion_context: Option<ConversionContext>,
    upstream_response_snapshot_limit: Option<usize>,
) -> Result<DebugHttpResponse, GatewayForwardError> {
    let status = response.status();
    let mut response_headers = filtered_response_headers(response.headers());
    let should_stream = should_stream_response(request, route, response.headers(), status.as_u16());
    let response_conversion_route = conversion_route.map(ConversionRoute::reverse);

    if should_stream {
        if response_conversion_route.is_some() {
            set_response_content_type(&mut response_headers, "text/event-stream");
        }
        let upstream_response_body_stream_snapshot = response_conversion_route
            .is_some()
            .then(|| upstream_response_snapshot_limit)
            .flatten()
            .map(SharedBodySnapshot::new);
        let raw_body_stream = match upstream_response_body_stream_snapshot.clone() {
            Some(snapshot) => snapshot_response_stream(response.bytes_stream(), snapshot),
            None => response.bytes_stream(),
        };
        let body_stream = match response_conversion_route {
            Some(route) => {
                convert_sse_stream_with_context(route, raw_body_stream, conversion_context.clone())
            }
            None => raw_body_stream,
        };
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
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
            upstream_response_body_stream_snapshot,
            provider_type: provider.meta.provider_type.clone(),
            cost_multiplier: Some(provider.meta.cost_multiplier.clone()),
            pricing_model_source: Some(provider.meta.pricing_model_source.clone()),
            upstream_url: Some(upstream_url),
            error_category: None,
            attempt_count: 1,
            provider_attempt_count: 1,
            provider_attempts: Vec::new(),
            failover: false,
            note: format!(
                "streaming forwarded to provider id={} name={}",
                provider.id, provider.name
            ),
        };
        return Ok(gateway_response);
    }

    let upstream_response_body = response.bytes().await.map_err(|mut error| {
        error.upstream_request_body = Some(upstream_body_snapshot.clone());
        error
    })?;
    let upstream_response_body_bytes = upstream_response_body.len() as u64;
    let mut body = upstream_response_body.clone();
    if let Some(route) = response_conversion_route {
        if (200..400).contains(&status.as_u16()) {
            body = convert_response_body_with_context(route, &body, conversion_context.as_ref())
                .map_err(|error| GatewayForwardError {
                    message: error.to_string(),
                    kind: GatewayFailureKind::GatewayParse,
                    upstream_request_body: Some(upstream_body_snapshot.clone()),
                    upstream_response_body: Some(upstream_response_body.clone()),
                    upstream_response_body_bytes,
                })?;
            set_response_content_type(&mut response_headers, "application/json");
        } else {
            let converted_error_body = convert_error_response_body(route, &body);
            if converted_error_body != body {
                body = converted_error_body;
                set_response_content_type(&mut response_headers, "application/json");
            }
        }
    }
    let token_usage = from_response_body(provider.cli_key, &body);
    let stored_upstream_response_body =
        (upstream_response_body != body).then_some(upstream_response_body);
    let stored_upstream_response_body_bytes = stored_upstream_response_body
        .as_ref()
        .map(|_| upstream_response_body_bytes)
        .unwrap_or(0);

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
        upstream_response_body: stored_upstream_response_body,
        upstream_response_body_bytes: stored_upstream_response_body_bytes,
        upstream_response_body_stream_snapshot: None,
        provider_type: provider.meta.provider_type.clone(),
        cost_multiplier: Some(provider.meta.cost_multiplier.clone()),
        pricing_model_source: Some(provider.meta.pricing_model_source.clone()),
        upstream_url: Some(upstream_url),
        error_category: None,
        attempt_count: 1,
        provider_attempt_count: 1,
        provider_attempts: Vec::new(),
        failover: false,
        note: format!(
            "forwarded to provider id={} name={}",
            provider.id, provider.name
        ),
    };
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
    upstream_response_body: Option<Vec<u8>>,
    upstream_url: String,
) -> DebugHttpResponse {
    let token_usage = from_response_body(provider.cli_key, &body);
    let upstream_response_body_bytes = upstream_response_body
        .as_ref()
        .map(|body| body.len() as u64)
        .unwrap_or(0);
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
        upstream_response_body,
        upstream_response_body_bytes,
        upstream_response_body_stream_snapshot: None,
        upstream_url: Some(upstream_url),
        error_category: None,
        attempt_count: 1,
        provider_attempt_count: 1,
        provider_attempts: Vec::new(),
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
    if let Some((body, body_bytes)) = response.upstream_response_body_snapshot() {
        failure_response.upstream_response_body = Some(body);
        failure_response.upstream_response_body_bytes = body_bytes;
    }
    failure_response.error_category = Some(category.to_string());
    failure_response.attempt_count = attempt_count;
    failure_response.provider_attempt_count = provider_attempt_count;
    failure_response.failover = failover;
    failure_response
}

fn provider_attempt_log(response: &DebugHttpResponse) -> GatewayProviderAttempt {
    GatewayProviderAttempt {
        provider_id: response.provider_id.clone(),
        provider_name: response.provider_name.clone(),
        upstream_model_id: response.upstream_model_id.clone(),
        status_code: Some(response.status_code),
        success: (200..400).contains(&response.status_code) && response.error_category.is_none(),
        error_category: response.error_category.clone(),
        error_message: response
            .error_category
            .as_ref()
            .map(|_| response.note.clone())
            .filter(|message| !message.trim().is_empty()),
        attempt_count: response.provider_attempt_count.max(1),
        total_attempt_count: response.attempt_count.max(1),
    }
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

fn should_attempt_thinking_budget_rectifier(
    enabled: bool,
    target_protocol: AiProtocol,
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    headers: &HeaderMap,
    status_code: u16,
) -> bool {
    enabled
        && target_protocol == AiProtocol::AnthropicMessages
        && !should_stream_response(request, route, headers, status_code)
        && (400..500).contains(&status_code)
}

fn should_rectify_thinking_signature(status_code: u16, body: &[u8]) -> bool {
    if !(400..500).contains(&status_code) {
        return false;
    }
    let Some(message) = extract_error_message_from_body(body) else {
        return false;
    };
    let lower = message.to_ascii_lowercase();

    if lower.contains("invalid")
        && lower.contains("signature")
        && lower.contains("thinking")
        && lower.contains("block")
    {
        return true;
    }
    if lower.contains("thought signature")
        && (lower.contains("not valid") || lower.contains("invalid"))
    {
        return true;
    }
    if lower.contains("must start with a thinking block") {
        return true;
    }
    if lower.contains("expected")
        && (lower.contains("thinking") || lower.contains("redacted_thinking"))
        && lower.contains("found")
        && lower.contains("tool_use")
    {
        return true;
    }
    if lower.contains("signature") && lower.contains("field required") {
        return true;
    }
    if lower.contains("signature") && lower.contains("extra inputs are not permitted") {
        return true;
    }
    if (lower.contains("thinking") || lower.contains("redacted_thinking"))
        && lower.contains("cannot be modified")
    {
        return true;
    }
    lower.contains("非法请求")
        || lower.contains("illegal request")
        || lower.contains("invalid request")
}

fn should_attempt_unsupported_media_rectifier(status_code: u16) -> bool {
    matches!(status_code, 400 | 415 | 422 | 501)
}

fn should_rectify_unsupported_media(status_code: u16, body: &[u8]) -> bool {
    if !should_attempt_unsupported_media_rectifier(status_code) {
        return false;
    }
    let Some(message) = extract_error_message_from_body(body) else {
        return false;
    };
    let lower = message.to_ascii_lowercase();
    let mentions_media = [
        "image",
        "vision",
        "multimodal",
        "multi-modal",
        "modality",
        "modalities",
        "media",
        "attachment",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let unsupported_hint = [
        "unsupported",
        "not supported",
        "does not support",
        "doesn't support",
        "do not support",
        "don't support",
        "only supports text",
        "text only",
        "text-only",
        "invalid content type",
        "invalid message content",
        "unknown variant",
        "unknown content type",
        "unrecognized content type",
        "cannot process",
        "cannot handle",
        "can't process",
        "can't handle",
        "unable to process",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    unsupported_hint
        && (mentions_media
            || lower.contains("only supports text")
            || lower.contains("text only")
            || lower.contains("text-only"))
}

fn build_unsupported_media_rectified_body(
    body: &[u8],
) -> Result<Option<Vec<u8>>, GatewayForwardError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return Ok(None);
    };
    if !replace_unsupported_image_parts(&mut value) {
        return Ok(None);
    }
    serde_json::to_vec(&value)
        .map(Some)
        .map_err(|error| GatewayForwardError {
            message: format!("Failed to serialize unsupported media rectified body: {error}"),
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: None,
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })
}

fn replace_unsupported_image_parts(value: &mut Value) -> bool {
    if let Some(replacement) = unsupported_image_replacement(value) {
        *value = replacement;
        return true;
    }

    match value {
        Value::Object(object) => {
            let mut changed = false;
            for child in object.values_mut() {
                changed |= replace_unsupported_image_parts(child);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= replace_unsupported_image_parts(item);
            }
            changed
        }
        _ => false,
    }
}

fn unsupported_image_replacement(value: &Value) -> Option<Value> {
    let object = value.as_object()?;

    if let Some(content_type) = object.get("type").and_then(Value::as_str) {
        if matches!(content_type, "image" | "image_url") {
            return Some(unsupported_image_text_part(value, "text"));
        }
        if content_type == "input_image" {
            return Some(unsupported_image_text_part(value, "input_text"));
        }
    }

    if gemini_part_is_image(object.get("inlineData"))
        || gemini_part_is_image(object.get("fileData"))
        || gemini_part_is_image(object.get("inline_data"))
        || gemini_part_is_image(object.get("file_data"))
    {
        return Some(json!({ "text": UNSUPPORTED_IMAGE_MARKER }));
    }

    None
}

fn unsupported_image_text_part(original: &Value, content_type: &str) -> Value {
    let mut replacement = json!({
        "type": content_type,
        "text": UNSUPPORTED_IMAGE_MARKER,
    });
    if let Some(cache_control) = original.get("cache_control").cloned() {
        if let Value::Object(object) = &mut replacement {
            object.insert("cache_control".to_string(), cache_control);
        }
    }
    replacement
}

fn gemini_part_is_image(part: Option<&Value>) -> bool {
    part.and_then(Value::as_object)
        .and_then(|object| {
            object
                .get("mimeType")
                .or_else(|| object.get("mime_type"))
                .and_then(Value::as_str)
        })
        .is_some_and(|mime_type| mime_type.to_ascii_lowercase().starts_with("image/"))
}

fn extract_error_message_from_body(body: &[u8]) -> Option<String> {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        return extract_error_message_from_value(&value)
            .or_else(|| Some(value.to_string()))
            .filter(|message| !message.trim().is_empty());
    }

    std::str::from_utf8(body)
        .ok()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToString::to_string)
}

fn extract_error_message_from_value(value: &Value) -> Option<String> {
    if let Some(message) = value.as_str().filter(|message| !message.trim().is_empty()) {
        return Some(message.to_string());
    }

    for pointer in [
        "/error/message",
        "/message",
        "/detail",
        "/msg",
        "/status_msg",
        "/base_resp/status_msg",
    ] {
        if let Some(message) = value
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
        {
            return Some(message.to_string());
        }
    }

    value
        .get("error")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(ToString::to_string)
}

fn should_attempt_thinking_signature_rectifier(
    enabled: bool,
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    headers: &HeaderMap,
    status_code: u16,
) -> bool {
    enabled
        && route.cli_key == GatewayCliKey::Claude
        && !should_stream_response(request, route, headers, status_code)
        && (400..500).contains(&status_code)
}

fn convert_buffered_error_body(
    conversion_route: Option<ConversionRoute>,
    mut body: Vec<u8>,
    response_headers: &mut Vec<(String, String)>,
) -> Vec<u8> {
    if let Some(route) = conversion_route.map(ConversionRoute::reverse) {
        let converted_error_body = convert_error_response_body(route, &body);
        if converted_error_body != body {
            body = converted_error_body;
            set_response_content_type(response_headers, "application/json");
        }
    }
    body
}

fn can_retry_current_provider(
    provider_retry_count: u32,
    per_provider_retry_count: u32,
    retry_count: u32,
    max_retry_count: u32,
) -> bool {
    provider_retry_count < per_provider_retry_count && retry_count < max_retry_count
}

async fn wait_before_retry(retry_interval_secs: u64) {
    if retry_interval_secs > 0 {
        tokio::time::sleep(Duration::from_secs(retry_interval_secs)).await;
    }
}

fn resolve_upstream_model_id(
    request: &DebugHttpRequest,
    requested_model: &str,
    provider: &UpstreamProvider,
    apply_family_model_mapping: bool,
) -> String {
    if provider.cli_key != GatewayCliKey::Claude || !apply_family_model_mapping {
        return strip_one_m_context_marker(requested_model).to_string();
    }

    let resolved_model = resolve_claude_upstream_model_id(
        requested_model,
        &provider.model_mapping,
        is_claude_reasoning_request(request, requested_model),
    )
    .unwrap_or_else(|| requested_model.to_string());
    strip_one_m_context_marker(&resolved_model).to_string()
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

#[cfg(test)]
fn build_upstream_body(
    request: &DebugHttpRequest,
    _requested_model: &str,
    upstream_model_id: &str,
    strip_thinking_for_retry: bool,
    cache_injection_enabled: bool,
    cli_key: GatewayCliKey,
    target_protocol: AiProtocol,
    conversion_route: Option<ConversionRoute>,
    route_streaming: bool,
) -> Result<Vec<u8>, GatewayForwardError> {
    build_upstream_body_for_provider(
        request,
        _requested_model,
        upstream_model_id,
        strip_thinking_for_retry,
        cache_injection_enabled,
        cli_key,
        target_protocol,
        conversion_route,
        None,
        route_streaming,
    )
    .map(|prepared| prepared.body)
}

#[derive(Debug, Clone)]
struct PreparedUpstreamBody {
    body: Vec<u8>,
    conversion_context: ConversionContext,
}

fn build_upstream_body_for_provider(
    request: &DebugHttpRequest,
    _requested_model: &str,
    upstream_model_id: &str,
    strip_thinking_for_retry: bool,
    cache_injection_enabled: bool,
    cli_key: GatewayCliKey,
    target_protocol: AiProtocol,
    conversion_route: Option<ConversionRoute>,
    provider_type: Option<&str>,
    route_streaming: bool,
) -> Result<PreparedUpstreamBody, GatewayForwardError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(&request.body) else {
        if let Some(route) = conversion_route {
            return convert_request_body_with_context(route, &request.body)
                .map(|converted| PreparedUpstreamBody {
                    body: converted.body,
                    conversion_context: converted.context,
                })
                .map_err(|error| GatewayForwardError {
                    message: error.to_string(),
                    kind: GatewayFailureKind::GatewayParse,
                    upstream_request_body: Some(request.body.clone()),
                    upstream_response_body: None,
                    upstream_response_body_bytes: 0,
                });
        }
        return Ok(PreparedUpstreamBody {
            body: request.body.clone(),
            conversion_context: ConversionContext::default(),
        });
    };
    let upstream_model_for_body = strip_one_m_context_marker(upstream_model_id);
    if let Some(model_value) = value.get_mut("model") {
        if model_value.is_string() {
            *model_value = Value::String(upstream_model_for_body.to_string());
        }
    } else if conversion_route.is_some_and(|route| route.source == AiProtocol::GeminiNative) {
        if let Value::Object(object) = &mut value {
            object.insert(
                "model".to_string(),
                Value::String(upstream_model_for_body.to_string()),
            );
        }
    }
    if route_streaming
        && conversion_route.is_some_and(|route| {
            route.source == AiProtocol::GeminiNative && route.target != AiProtocol::GeminiNative
        })
    {
        if let Value::Object(object) = &mut value {
            object.insert("stream".to_string(), Value::Bool(true));
        }
    }
    if cli_key == GatewayCliKey::Claude && strip_thinking_for_retry {
        strip_thinking_blocks(&mut value);
    }
    let rewritten_body = serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to rewrite upstream request model: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })?;
    let (upstream_body, conversion_context) = if let Some(route) = conversion_route {
        let converted =
            convert_request_body_with_context(route, &rewritten_body).map_err(|error| {
                GatewayForwardError {
                    message: error.to_string(),
                    kind: GatewayFailureKind::GatewayParse,
                    upstream_request_body: Some(rewritten_body.clone()),
                    upstream_response_body: None,
                    upstream_response_body_bytes: 0,
                }
            })?;
        (converted.body, converted.context)
    } else {
        (rewritten_body, ConversionContext::default())
    };
    let upstream_body = apply_outbound_adapter_compat_for_provider(
        upstream_body,
        conversion_route,
        target_protocol,
        provider_type,
    )?;
    if cache_injection_enabled && target_protocol == AiProtocol::AnthropicMessages {
        return inject_cache_control_into_body(upstream_body).map(|body| PreparedUpstreamBody {
            body,
            conversion_context,
        });
    }
    Ok(PreparedUpstreamBody {
        body: upstream_body,
        conversion_context,
    })
}

#[cfg(test)]
fn apply_outbound_adapter_compat(
    body: Vec<u8>,
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
) -> Result<Vec<u8>, GatewayForwardError> {
    apply_outbound_adapter_compat_for_provider(body, conversion_route, target_protocol, None)
}

fn apply_outbound_adapter_compat_for_provider(
    body: Vec<u8>,
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
    provider_type: Option<&str>,
) -> Result<Vec<u8>, GatewayForwardError> {
    let mut value =
        serde_json::from_slice::<Value>(&body).map_err(|error| GatewayForwardError {
            message: format!("Failed to parse upstream request body for outbound adapter: {error}"),
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: Some(body.clone()),
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })?;
    filter_private_outbound_fields(&mut value, false);
    let provider_kind = ProviderBodyCompat::from_provider_type(provider_type);

    apply_provider_body_compat_before_generic(&mut value, target_protocol, provider_kind);

    if let Some(route) = conversion_route {
        if target_protocol == AiProtocol::OpenAiChat {
            normalize_converted_openai_chat_for_provider_compat(&mut value, provider_kind);
        }
        if let Value::Object(object) = &mut value {
            if route.source != AiProtocol::GeminiNative {
                if target_protocol == AiProtocol::OpenAiChat
                    || target_protocol == AiProtocol::OpenAiResponses
                {
                    remove_tool_controls_without_tools(
                        object,
                        &["tool_choice", "parallel_tool_calls"],
                    );
                } else if target_protocol == AiProtocol::AnthropicMessages {
                    remove_tool_controls_without_tools(object, &["tool_choice"]);
                }
            }
            if target_protocol == AiProtocol::AnthropicMessages {
                remove_anthropic_thinking_when_tool_choice_forces_tool_use(object);
            }
        }
    }

    apply_provider_body_compat_after_generic(&mut value, target_protocol, provider_kind);

    serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to serialize outbound adapter request body: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderBodyCompat {
    DeepSeek,
    Moonshot,
    Zai,
    Doubao,
    Xai,
    Longcat,
    ModelScope,
    Bailian,
    Mimo,
}

impl ProviderBodyCompat {
    fn from_provider_type(provider_type: Option<&str>) -> Option<Self> {
        let normalized = provider_type?.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "deepseek" => Some(Self::DeepSeek),
            "moonshot" | "kimi" => Some(Self::Moonshot),
            "zai" | "zhipu" | "glm" | "chatglm" | "bigmodel" | "big-model" => Some(Self::Zai),
            "doubao" | "doubaoseed" | "doubao-seed" | "volces" => Some(Self::Doubao),
            "xai" | "x-ai" | "grok" => Some(Self::Xai),
            "longcat" => Some(Self::Longcat),
            "modelscope" | "model-scope" => Some(Self::ModelScope),
            "bailian" | "dashscope" | "aliyun" => Some(Self::Bailian),
            "mimo" | "xiaomimimo" | "xiaomi-mimo" => Some(Self::Mimo),
            _ => None,
        }
    }
}

fn apply_provider_body_compat_before_generic(
    value: &mut Value,
    target_protocol: AiProtocol,
    provider_kind: Option<ProviderBodyCompat>,
) {
    let Value::Object(object) = value else {
        return;
    };

    match target_protocol {
        AiProtocol::OpenAiChat => {
            apply_openai_chat_provider_body_compat_before_generic(object, provider_kind);
        }
        AiProtocol::OpenAiResponses => {
            apply_openai_responses_provider_body_compat(object, provider_kind);
        }
        AiProtocol::AnthropicMessages => {
            apply_anthropic_provider_body_compat(object, provider_kind);
        }
        AiProtocol::GeminiNative => {}
    }
}

fn apply_provider_body_compat_after_generic(
    value: &mut Value,
    target_protocol: AiProtocol,
    provider_kind: Option<ProviderBodyCompat>,
) {
    let Value::Object(object) = value else {
        return;
    };

    if target_protocol == AiProtocol::OpenAiChat {
        apply_openai_chat_provider_body_compat_after_generic(object, provider_kind);
    }
}

fn apply_openai_chat_provider_body_compat_before_generic(
    object: &mut serde_json::Map<String, Value>,
    provider_kind: Option<ProviderBodyCompat>,
) {
    match provider_kind {
        Some(ProviderBodyCompat::DeepSeek) => {
            convert_response_format_json_schema_to_json_object(object);
            apply_deepseek_openai_chat_thinking_compat(object);
            sanitize_openai_chat_tools(object);
            if let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) {
                sanitize_openai_chat_messages(messages);
            }
        }
        Some(ProviderBodyCompat::Moonshot) => {
            convert_response_format_json_schema_to_json_object(object);
            backfill_tool_call_reasoning_content(object);
        }
        Some(ProviderBodyCompat::Zai) => {
            convert_response_format_json_schema_to_json_object(object);
            extract_metadata_to_vendor_ids(object);
            ensure_vendor_request_id(object);
            force_tool_choice_auto(object);
            apply_zai_openai_chat_thinking_compat(object);
        }
        Some(ProviderBodyCompat::Doubao) => {
            extract_metadata_to_vendor_ids(object);
            ensure_vendor_request_id(object);
        }
        Some(ProviderBodyCompat::Xai) => {
            strip_xai_unsupported_openai_chat_fields(object);
        }
        Some(ProviderBodyCompat::ModelScope) => {
            object.remove("metadata");
        }
        Some(ProviderBodyCompat::Bailian) => {
            merge_consecutive_tool_call_messages(object);
        }
        Some(ProviderBodyCompat::Mimo) => {
            backfill_tool_call_reasoning_content(object);
        }
        Some(ProviderBodyCompat::Longcat) | None => {}
    }
}

fn apply_openai_chat_provider_body_compat_after_generic(
    object: &mut serde_json::Map<String, Value>,
    provider_kind: Option<ProviderBodyCompat>,
) {
    if provider_kind == Some(ProviderBodyCompat::Longcat) {
        normalize_longcat_message_content_arrays(object);
    }
}

fn apply_openai_responses_provider_body_compat(
    object: &mut serde_json::Map<String, Value>,
    provider_kind: Option<ProviderBodyCompat>,
) {
    if matches!(
        provider_kind,
        Some(ProviderBodyCompat::Doubao | ProviderBodyCompat::ModelScope)
    ) {
        object.remove("metadata");
    }
}

fn apply_anthropic_provider_body_compat(
    object: &mut serde_json::Map<String, Value>,
    provider_kind: Option<ProviderBodyCompat>,
) {
    if matches!(
        provider_kind,
        Some(
            ProviderBodyCompat::DeepSeek | ProviderBodyCompat::Moonshot | ProviderBodyCompat::Mimo
        )
    ) {
        normalize_anthropic_tool_thinking_history(object);
    }
    if provider_kind == Some(ProviderBodyCompat::DeepSeek) {
        strip_deepseek_disabled_thinking_effort(object);
    }
}

fn convert_response_format_json_schema_to_json_object(object: &mut serde_json::Map<String, Value>) {
    let Some(response_format) = object
        .get_mut("response_format")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if response_format.get("type").and_then(Value::as_str) != Some("json_schema") {
        return;
    }
    response_format.insert("type".to_string(), Value::String("json_object".to_string()));
    response_format.remove("json_schema");
}

fn apply_deepseek_openai_chat_thinking_compat(object: &mut serde_json::Map<String, Value>) {
    let thinking_disabled = object
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .is_some_and(is_reasoning_disabled_effort);
    object.insert(
        "thinking".to_string(),
        json!({ "type": if thinking_disabled { "disabled" } else { "enabled" } }),
    );

    if thinking_disabled {
        object.remove("reasoning_effort");
        return;
    }

    if let Some(effort) = object
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .and_then(map_deepseek_reasoning_effort)
    {
        object.insert(
            "reasoning_effort".to_string(),
            Value::String(effort.to_string()),
        );
    }

    if let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            let Some(message_object) = message.as_object_mut() else {
                continue;
            };
            if message_object.get("role").and_then(Value::as_str) == Some("assistant")
                && !message_object.contains_key("reasoning_content")
            {
                message_object.insert(
                    "reasoning_content".to_string(),
                    Value::String(String::new()),
                );
            }
        }
    }
}

fn apply_zai_openai_chat_thinking_compat(object: &mut serde_json::Map<String, Value>) {
    let Some(effort) = object.get("reasoning_effort").and_then(Value::as_str) else {
        return;
    };
    object.insert(
        "thinking".to_string(),
        json!({ "type": if is_reasoning_disabled_effort(effort) { "disabled" } else { "enabled" } }),
    );
}

fn is_reasoning_disabled_effort(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "none" | "off" | "disabled"
    )
}

fn map_deepseek_reasoning_effort(value: &str) -> Option<&'static str> {
    let normalized = value.trim().to_ascii_lowercase();
    if is_reasoning_disabled_effort(&normalized) {
        return None;
    }
    if matches!(normalized.as_str(), "max" | "xhigh") {
        Some("max")
    } else {
        Some("high")
    }
}

fn extract_metadata_to_vendor_ids(object: &mut serde_json::Map<String, Value>) {
    let metadata = object.remove("metadata");
    let Some(metadata_object) = metadata.and_then(|metadata| metadata.as_object().cloned()) else {
        return;
    };
    for (metadata_key, vendor_key) in [("user_id", "user_id"), ("request_id", "request_id")] {
        if let Some(value) = metadata_object
            .get(metadata_key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            object.insert(vendor_key.to_string(), Value::String(value.to_string()));
        }
    }
}

fn ensure_vendor_request_id(object: &mut serde_json::Map<String, Value>) {
    let has_request_id = object
        .get("request_id")
        .and_then(Value::as_str)
        .is_some_and(|request_id| !request_id.trim().is_empty());
    if has_request_id {
        return;
    }

    let timestamp_ms = chrono::Utc::now().timestamp_millis();
    object.insert(
        "request_id".to_string(),
        Value::String(format!("req_{timestamp_ms}")),
    );
}

fn force_tool_choice_auto(object: &mut serde_json::Map<String, Value>) {
    if object.get("tool_choice").is_some() {
        object.insert("tool_choice".to_string(), Value::String("auto".to_string()));
    }
}

fn strip_xai_unsupported_openai_chat_fields(object: &mut serde_json::Map<String, Value>) {
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .map(|model| model.trim().to_ascii_lowercase());
    match model.as_deref() {
        Some("grok-4") => {
            for field in [
                "reasoning_effort",
                "presence_penalty",
                "frequency_penalty",
                "stop",
            ] {
                object.remove(field);
            }
        }
        Some("grok-3" | "grok-3-mini") => {
            for field in ["presence_penalty", "frequency_penalty", "stop"] {
                object.remove(field);
            }
        }
        _ => {}
    }
}

fn backfill_tool_call_reasoning_content(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            continue;
        };
        let is_assistant_tool_call = message_object.get("role").and_then(Value::as_str)
            == Some("assistant")
            && message_object
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|tool_calls| !tool_calls.is_empty());
        if !is_assistant_tool_call {
            continue;
        }
        let has_reasoning = message_object
            .get("reasoning_content")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty());
        if !has_reasoning {
            message_object.insert(
                "reasoning_content".to_string(),
                Value::String("tool call".to_string()),
            );
        }
    }
}

fn normalize_longcat_message_content_arrays(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            continue;
        };
        let content = message_object.remove("content");
        let normalized = match content {
            Some(Value::Array(parts)) => Value::Array(parts),
            Some(Value::String(text)) => json!([{ "type": "text", "text": text }]),
            Some(Value::Null) | None => json!([{ "type": "text", "text": "" }]),
            Some(Value::Object(object)) => Value::Array(vec![Value::Object(object)]),
            Some(other) => json!([{ "type": "text", "text": other.to_string() }]),
        };
        message_object.insert("content".to_string(), normalized);
    }
}

fn merge_consecutive_tool_call_messages(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    if messages.len() < 2 {
        return;
    }

    let mut merged_messages = Vec::with_capacity(messages.len());
    let mut pending: Option<Value> = None;

    for message in std::mem::take(messages) {
        if is_bailian_mergeable_tool_call_message(&message) {
            if let Some(pending_message) = pending.as_mut() {
                let additional_tool_calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if let Some(pending_tool_calls) = pending_message
                    .get_mut("tool_calls")
                    .and_then(Value::as_array_mut)
                {
                    pending_tool_calls.extend(additional_tool_calls);
                }
            } else {
                pending = Some(message);
            }
            continue;
        }

        if let Some(pending_message) = pending.take() {
            merged_messages.push(pending_message);
        }
        merged_messages.push(message);
    }

    if let Some(pending_message) = pending {
        merged_messages.push(pending_message);
    }

    *messages = merged_messages;
}

fn is_bailian_mergeable_tool_call_message(message: &Value) -> bool {
    let Some(object) = message.as_object() else {
        return false;
    };
    if object.get("role").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    if !object
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|tool_calls| !tool_calls.is_empty())
    {
        return false;
    }
    for field in [
        "tool_call_id",
        "name",
        "message_index",
        "tool_call_name",
        "tool_call_is_error",
        "refusal",
        "reasoning_content",
        "reasoning",
        "reasoning_signature",
        "redacted_reasoning_content",
        "cache_control",
    ] {
        if object
            .get(field)
            .is_some_and(|value| !value_is_empty_chat_payload(value))
        {
            return false;
        }
    }
    object
        .get("content")
        .is_none_or(value_is_empty_chat_payload)
}

fn normalize_anthropic_tool_thinking_history(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        if !content
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        {
            continue;
        }

        let mut has_thinking = false;
        for block in content.iter_mut() {
            match block.get("type").and_then(Value::as_str) {
                Some("thinking") => {
                    let has_non_empty_thinking = block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty());
                    if let Some(block_object) = block.as_object_mut() {
                        block_object.remove("signature");
                        if !has_non_empty_thinking {
                            block_object.insert(
                                "thinking".to_string(),
                                Value::String("tool call".to_string()),
                            );
                        }
                    }
                    has_thinking = true;
                }
                Some("redacted_thinking") => {
                    *block = json!({
                        "type": "thinking",
                        "thinking": "[redacted thinking]"
                    });
                    has_thinking = true;
                }
                _ => {}
            }
        }

        if !has_thinking {
            content.insert(
                0,
                json!({
                    "type": "thinking",
                    "thinking": "tool call"
                }),
            );
        }
    }
}

fn strip_deepseek_disabled_thinking_effort(object: &mut serde_json::Map<String, Value>) {
    let thinking_type = object
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str);
    if thinking_type != Some("disabled") {
        return;
    }

    let should_remove_output_config = object
        .get_mut("output_config")
        .and_then(Value::as_object_mut)
        .is_some_and(|output_config| {
            output_config.remove("effort");
            output_config.is_empty()
        });
    if should_remove_output_config {
        object.remove("output_config");
    }
    object.remove("reasoning_effort");
}

fn normalize_converted_openai_chat_for_provider_compat(
    value: &mut Value,
    provider_kind: Option<ProviderBodyCompat>,
) {
    let Value::Object(object) = value else {
        return;
    };

    for field in ["verbosity", "prompt_cache_key"] {
        object.remove(field);
    }
    if provider_kind != Some(ProviderBodyCompat::DeepSeek) {
        object.remove("reasoning_effort");
    }

    sanitize_openai_chat_tools(object);
    if let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) {
        sanitize_openai_chat_messages(messages);
    }
}

fn sanitize_openai_chat_tools(object: &mut serde_json::Map<String, Value>) {
    let mut should_remove_tools = false;
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        tools.retain(is_supported_openai_chat_tool);
        for tool in tools.iter_mut() {
            if let Value::Object(tool_object) = tool {
                tool_object.remove("response_custom_tool");
            }
        }
        should_remove_tools = tools.is_empty();
    }
    if should_remove_tools {
        object.remove("tools");
    }
}

fn is_supported_openai_chat_tool(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("function")
        && tool
            .pointer("/function/name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.trim().is_empty())
}

fn sanitize_openai_chat_messages(messages: &mut Vec<Value>) {
    let mut removed_tool_call_ids = Vec::new();
    let mut filtered_messages = Vec::with_capacity(messages.len());

    for mut message in std::mem::take(messages) {
        if is_removed_tool_result_message(&message, &removed_tool_call_ids) {
            continue;
        }

        if let Value::Object(object) = &mut message {
            if object.get("role").and_then(Value::as_str) == Some("system") {
                flatten_system_text_parts(object);
            }
            sanitize_openai_chat_tool_calls(object, &mut removed_tool_call_ids);
            if should_drop_empty_assistant_message(object) {
                continue;
            }
        }

        filtered_messages.push(message);
    }

    *messages = filtered_messages;
}

fn is_removed_tool_result_message(message: &Value, removed_tool_call_ids: &[String]) -> bool {
    message.get("role").and_then(Value::as_str) == Some("tool")
        && message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .is_some_and(|tool_call_id| {
                removed_tool_call_ids
                    .iter()
                    .any(|removed_id| removed_id == tool_call_id)
            })
}

fn flatten_system_text_parts(object: &mut serde_json::Map<String, Value>) {
    let Some(parts) = object.get("content").and_then(Value::as_array) else {
        return;
    };
    let texts = parts
        .iter()
        .filter_map(openai_chat_part_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    if !texts.is_empty() {
        object.insert("content".to_string(), Value::String(texts.join("\n\n")));
    }
}

fn openai_chat_part_text(part: &Value) -> Option<String> {
    if let Some(text) = part.as_str() {
        return Some(text.to_string());
    }
    let part_type = part.get("type").and_then(Value::as_str)?;
    if matches!(part_type, "text" | "input_text" | "output_text") {
        return part
            .get("text")
            .and_then(Value::as_str)
            .map(ToString::to_string);
    }
    None
}

fn sanitize_openai_chat_tool_calls(
    object: &mut serde_json::Map<String, Value>,
    removed_tool_call_ids: &mut Vec<String>,
) {
    let mut should_remove_tool_calls = false;
    if let Some(Value::Array(tool_calls)) = object.get_mut("tool_calls") {
        for tool_call in tool_calls.iter_mut() {
            normalize_openai_chat_tool_call_arguments(tool_call);
        }
        tool_calls.retain(|tool_call| {
            if is_supported_openai_chat_tool_call(tool_call) {
                return true;
            }
            record_removed_tool_call_ids(tool_call, removed_tool_call_ids);
            false
        });
        should_remove_tool_calls = tool_calls.is_empty();
    }
    if should_remove_tool_calls {
        object.remove("tool_calls");
    }
}

fn normalize_openai_chat_tool_call_arguments(tool_call: &mut Value) {
    let Some(function) = tool_call.get_mut("function").and_then(Value::as_object_mut) else {
        return;
    };
    let has_non_empty_arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .is_some_and(|arguments| !arguments.trim().is_empty());
    if !has_non_empty_arguments {
        function.insert("arguments".to_string(), Value::String("{}".to_string()));
    }
}

fn is_supported_openai_chat_tool_call(tool_call: &Value) -> bool {
    tool_call.get("type").and_then(Value::as_str) == Some("function")
        && tool_call.get("response_custom_tool_call").is_none()
        && tool_call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.trim().is_empty())
}

fn record_removed_tool_call_ids(tool_call: &Value, removed_tool_call_ids: &mut Vec<String>) {
    for pointer in ["/id", "/response_custom_tool_call/call_id"] {
        if let Some(id) = tool_call.pointer(pointer).and_then(Value::as_str) {
            if !removed_tool_call_ids
                .iter()
                .any(|removed_id| removed_id == id)
            {
                removed_tool_call_ids.push(id.to_string());
            }
        }
    }
}

fn should_drop_empty_assistant_message(object: &serde_json::Map<String, Value>) -> bool {
    if object.get("role").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    if object.get("tool_calls").is_some() {
        return false;
    }
    for field in ["content", "refusal", "reasoning_content", "reasoning"] {
        if object
            .get(field)
            .is_some_and(|value| !value_is_empty_chat_payload(value))
        {
            return false;
        }
    }
    true
}

fn value_is_empty_chat_payload(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

fn filter_private_outbound_fields(value: &mut Value, preserve_schema_name_keys: bool) {
    match value {
        Value::Object(object) => {
            if !preserve_schema_name_keys {
                object.retain(|key, _| !key.starts_with('_'));
            }
            for (key, child) in object {
                filter_private_outbound_fields(child, is_schema_name_map(key));
            }
        }
        Value::Array(items) => {
            for item in items {
                filter_private_outbound_fields(item, false);
            }
        }
        _ => {}
    }
}

fn is_schema_name_map(key: &str) -> bool {
    matches!(
        key,
        "properties" | "patternProperties" | "definitions" | "$defs"
    )
}

fn remove_tool_controls_without_tools(
    object: &mut serde_json::Map<String, Value>,
    control_fields: &[&str],
) {
    let has_tools = object
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty());
    if has_tools {
        return;
    }
    for field in control_fields {
        object.remove(*field);
    }
}

fn remove_anthropic_thinking_when_tool_choice_forces_tool_use(
    object: &mut serde_json::Map<String, Value>,
) {
    if anthropic_tool_choice_forces_tool_use(object.get("tool_choice")) {
        object.remove("thinking");
    }
}

fn anthropic_tool_choice_forces_tool_use(value: Option<&Value>) -> bool {
    matches!(
        value
            .and_then(Value::as_object)
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str),
        Some("any" | "tool")
    )
}

fn build_thinking_signature_rectified_upstream_body(
    request: &DebugHttpRequest,
    requested_model: &str,
    upstream_model_id: &str,
    cache_injection_enabled: bool,
    route: &GatewayRoute,
    target_protocol: AiProtocol,
    conversion_route: Option<ConversionRoute>,
    provider_type: Option<&str>,
    route_streaming: bool,
    original_upstream_body: &[u8],
) -> Result<Option<PreparedUpstreamBody>, GatewayForwardError> {
    let prepared = build_upstream_body_for_provider(
        request,
        requested_model,
        upstream_model_id,
        true,
        cache_injection_enabled,
        route.cli_key,
        target_protocol,
        conversion_route,
        provider_type,
        route_streaming,
    )?;

    if prepared.body == original_upstream_body {
        Ok(None)
    } else {
        Ok(Some(prepared))
    }
}

fn inject_cache_control_into_body(body: Vec<u8>) -> Result<Vec<u8>, GatewayForwardError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(&body) else {
        return Ok(body);
    };
    if !cache_injector::inject_cache_control(&mut value) {
        return Ok(body);
    }
    serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to inject Anthropic cache_control into upstream request: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })
}

fn strip_one_m_context_marker(model: &str) -> &str {
    let trimmed = model.trim_end();
    let bytes = trimmed.as_bytes();
    for marker in [
        ONE_M_CONTEXT_MARKER.as_bytes(),
        ENCODED_ONE_M_CONTEXT_MARKER.as_bytes(),
    ] {
        if bytes.len() >= marker.len()
            && bytes[bytes.len() - marker.len()..].eq_ignore_ascii_case(marker)
        {
            return trimmed[..trimmed.len() - marker.len()].trim_end();
        }
    }
    model
}

fn source_protocol_from_route(route: &GatewayRoute) -> Option<AiProtocol> {
    match route.cli_key {
        GatewayCliKey::Claude => {
            if route.forwarded_path == "/v1/messages" || route.forwarded_path == "/messages" {
                Some(AiProtocol::AnthropicMessages)
            } else {
                None
            }
        }
        GatewayCliKey::Codex => {
            let path = route.forwarded_path.as_str();
            if path == "/v1/chat/completions" || path == "/chat/completions" {
                Some(AiProtocol::OpenAiChat)
            } else if path == "/v1/responses"
                || path == "/responses"
                || path == "/v1/responses/compact"
                || path == "/responses/compact"
            {
                Some(AiProtocol::OpenAiResponses)
            } else {
                None
            }
        }
        GatewayCliKey::Gemini => {
            if route.forwarded_path.contains(":generateContent")
                || route.forwarded_path.contains(":streamGenerateContent")
            {
                Some(AiProtocol::GeminiNative)
            } else {
                None
            }
        }
        GatewayCliKey::OpenCode => None,
    }
}

fn conversion_route(
    source_protocol: AiProtocol,
    provider: &UpstreamProvider,
) -> Option<ConversionRoute> {
    (source_protocol != provider.target_protocol).then_some(ConversionRoute::new(
        source_protocol,
        provider.target_protocol,
    ))
}

fn upstream_forwarded_path<'a>(
    route: &'a GatewayRoute,
    provider: &'a UpstreamProvider,
    conversion_route: Option<ConversionRoute>,
    upstream_model_id: &str,
    target_streaming: bool,
) -> Cow<'a, str> {
    if conversion_route.is_none() {
        if route.cli_key == GatewayCliKey::Gemini {
            return strip_one_m_context_marker_from_gemini_path(&route.forwarded_path);
        }
        return Cow::Borrowed(&route.forwarded_path);
    }

    match provider.target_protocol {
        AiProtocol::AnthropicMessages => Cow::Borrowed("/v1/messages"),
        AiProtocol::OpenAiResponses => Cow::Borrowed("/v1/responses"),
        AiProtocol::OpenAiChat => Cow::Borrowed("/v1/chat/completions"),
        AiProtocol::GeminiNative => Cow::Owned(gemini_native_forwarded_path(
            upstream_model_id,
            target_streaming,
        )),
    }
}

fn build_provider_target_url(
    provider: &UpstreamProvider,
    forwarded_path: &str,
    route_query: Option<&str>,
    conversion_route: Option<ConversionRoute>,
    target_streaming: bool,
) -> Result<reqwest::Url, String> {
    if provider.is_full_url {
        let query = converted_route_query(route_query, conversion_route, target_streaming);
        return build_full_target_url(&provider.base_url, query.as_deref());
    }

    let query = if conversion_route.is_some() {
        converted_route_query(route_query, conversion_route, target_streaming)
    } else {
        route_query.map(str::to_string)
    };
    build_target_url(&provider.base_url, forwarded_path, query.as_deref())
}

fn build_full_target_url(
    base_url: &str,
    route_query: Option<&str>,
) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|error| format!("Invalid upstream full URL '{}': {error}", base_url))?;
    if let Some(route_query) = route_query.filter(|query| !query.trim().is_empty()) {
        let merged_query = merge_query(url.query(), Some(route_query));
        url.set_query(merged_query.as_deref());
    }
    Ok(url)
}

fn converted_route_query(
    route_query: Option<&str>,
    conversion_route: Option<ConversionRoute>,
    target_streaming: bool,
) -> Option<String> {
    let mut params: Vec<String> = route_query
        .into_iter()
        .flat_map(|query| query.split('&'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| !pair.starts_with("beta="))
        .filter(|pair| {
            !conversion_route.is_some_and(|route| {
                route.source == AiProtocol::GeminiNative
                    && route.target != AiProtocol::GeminiNative
                    && (pair.eq_ignore_ascii_case("alt=sse") || pair.starts_with("key="))
            })
        })
        .map(str::to_string)
        .collect();
    if conversion_route.is_some_and(|route| route.target == AiProtocol::GeminiNative)
        && target_streaming
        && !params
            .iter()
            .any(|pair| pair.eq_ignore_ascii_case("alt=sse"))
    {
        params.push("alt=sse".to_string());
    }
    (!params.is_empty()).then(|| params.join("&"))
}

fn merge_query(base_query: Option<&str>, extra_query: Option<&str>) -> Option<String> {
    let params: Vec<String> = base_query
        .into_iter()
        .chain(extra_query)
        .flat_map(|query| query.split('&'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| !pair.starts_with("beta="))
        .map(str::to_string)
        .collect();
    (!params.is_empty()).then(|| params.join("&"))
}

fn strip_one_m_context_marker_from_gemini_path(path: &str) -> Cow<'_, str> {
    let Some((model_start, model_end)) = gemini_model_segment_bounds(path) else {
        return Cow::Borrowed(path);
    };
    let model = &path[model_start..model_end];
    let stripped_model = strip_one_m_context_marker(model);
    if stripped_model == model {
        return Cow::Borrowed(path);
    }

    let mut rewritten_path =
        String::with_capacity(path.len() - (model.len() - stripped_model.len()));
    rewritten_path.push_str(&path[..model_start]);
    rewritten_path.push_str(stripped_model);
    rewritten_path.push_str(&path[model_end..]);
    Cow::Owned(rewritten_path)
}

fn gemini_native_forwarded_path(model: &str, target_streaming: bool) -> String {
    let model = strip_one_m_context_marker(model)
        .trim()
        .strip_prefix("models/")
        .unwrap_or_else(|| strip_one_m_context_marker(model).trim());
    let action = if target_streaming {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    format!("/v1beta/models/{model}:{action}")
}

fn gemini_model_segment_bounds(path: &str) -> Option<(usize, usize)> {
    let marker = "/models/";
    let model_start = path.find(marker)? + marker.len();
    let model_part = &path[model_start..];
    let model_len = model_part
        .find(|ch| matches!(ch, ':' | '/' | '?'))
        .unwrap_or(model_part.len());
    if model_len == 0 {
        return None;
    }
    Some((model_start, model_start + model_len))
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
    match provider.auth_strategy {
        ProviderAuthStrategy::AnthropicApiKey => {
            let value = HeaderValue::from_str(provider.api_key.trim())
                .map_err(|error| format!("Invalid Claude API key header value: {error}"))?;
            append_preserved_header(headers, preserved, "x-api-key", value)?;
        }
        ProviderAuthStrategy::Bearer => {
            let value = HeaderValue::from_str(&format!("Bearer {}", provider.api_key.trim()))
                .map_err(|error| format!("Invalid Authorization header value: {error}"))?;
            append_preserved_header(headers, preserved, AUTHORIZATION.as_str(), value)?;
        }
        ProviderAuthStrategy::GoogleApiKey => {
            let value = HeaderValue::from_str(provider.api_key.trim())
                .map_err(|error| format!("Invalid Google API key header value: {error}"))?;
            append_preserved_header(headers, preserved, "x-goog-api-key", value)?;
        }
        ProviderAuthStrategy::GoogleOAuth => {
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
            let token = oauth_token.unwrap_or_else(|| trimmed.to_string());
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|error| format!("Invalid Google OAuth header value: {error}"))?;
            append_preserved_header(headers, preserved, AUTHORIZATION.as_str(), value)?;
            append_preserved_header(
                headers,
                preserved,
                "x-goog-api-client",
                HeaderValue::from_static("GeminiCLI/1.0"),
            )?;
        }
    }
    if provider.target_protocol == AiProtocol::AnthropicMessages
        && !headers.contains_key("anthropic-version")
    {
        append_preserved_header(
            headers,
            preserved,
            "anthropic-version",
            HeaderValue::from_static("2023-06-01"),
        )?;
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

fn set_response_content_type(headers: &mut Vec<(String, String)>, content_type: &str) {
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case(CONTENT_TYPE.as_str()));
    headers.push((CONTENT_TYPE.as_str().to_string(), content_type.to_string()));
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

    fn debug_request(body: &[u8]) -> DebugHttpRequest {
        DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    fn claude_provider(mapping: UpstreamModelMapping) -> UpstreamProvider {
        UpstreamProvider {
            cli_key: GatewayCliKey::Claude,
            id: "p1".to_string(),
            name: "Provider".to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: "key".to_string(),
            target_protocol:
                crate::coding::proxy_gateway::transformer::AiProtocol::AnthropicMessages,
            auth_strategy: ProviderAuthStrategy::AnthropicApiKey,
            is_full_url: false,
            sort_index: Some(0),
            meta: ProviderGatewayMeta::default(),
            model_mapping: mapping,
        }
    }

    fn provider_for_cli(cli_key: GatewayCliKey) -> UpstreamProvider {
        UpstreamProvider {
            cli_key,
            id: "p1".to_string(),
            name: "Provider".to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: "key".to_string(),
            target_protocol: match cli_key {
                GatewayCliKey::Claude => {
                    crate::coding::proxy_gateway::transformer::AiProtocol::AnthropicMessages
                }
                GatewayCliKey::Codex => {
                    crate::coding::proxy_gateway::transformer::AiProtocol::OpenAiResponses
                }
                GatewayCliKey::Gemini => {
                    crate::coding::proxy_gateway::transformer::AiProtocol::GeminiNative
                }
                GatewayCliKey::OpenCode => {
                    crate::coding::proxy_gateway::transformer::AiProtocol::OpenAiChat
                }
            },
            auth_strategy: match cli_key {
                GatewayCliKey::Claude => ProviderAuthStrategy::AnthropicApiKey,
                GatewayCliKey::Codex | GatewayCliKey::OpenCode => ProviderAuthStrategy::Bearer,
                GatewayCliKey::Gemini => ProviderAuthStrategy::GoogleApiKey,
            },
            is_full_url: false,
            sort_index: Some(0),
            meta: ProviderGatewayMeta::default(),
            model_mapping: UpstreamModelMapping::default(),
        }
    }

    fn gateway_route(cli_key: GatewayCliKey, forwarded_path: &str) -> GatewayRoute {
        GatewayRoute {
            cli_key,
            route_name: "test",
            forwarded_path: forwarded_path.to_string(),
            query: None,
        }
    }

    #[test]
    fn conversion_route_rewrites_codex_responses_to_anthropic_messages_path() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let provider = UpstreamProvider {
            cli_key: GatewayCliKey::Codex,
            id: "p1".to_string(),
            name: "Provider".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "key".to_string(),
            target_protocol:
                crate::coding::proxy_gateway::transformer::AiProtocol::AnthropicMessages,
            auth_strategy: ProviderAuthStrategy::AnthropicApiKey,
            is_full_url: false,
            sort_index: Some(0),
            meta: ProviderGatewayMeta::default(),
            model_mapping: UpstreamModelMapping::default(),
        };
        let source_protocol = source_protocol_from_route(&route).unwrap();
        let conversion = conversion_route(source_protocol, &provider);

        assert_eq!(
            upstream_forwarded_path(&route, &provider, conversion, "claude-sonnet", false).as_ref(),
            "/v1/messages"
        );
    }

    #[test]
    fn direct_route_keeps_original_forwarded_path() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let provider = provider_for_cli(GatewayCliKey::Codex);

        assert_eq!(
            upstream_forwarded_path(&route, &provider, None, "gpt-5", false).as_ref(),
            "/v1/responses"
        );
    }

    #[test]
    fn claude_model_mapping_uses_provider_specific_model_for_standard_name() {
        let provider = claude_provider(UpstreamModelMapping {
            sonnet_model: Some("provider-sonnet".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_upstream_model_id(&debug_request(b"{}"), "claude-sonnet-4-6", &provider, true),
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
            resolve_upstream_model_id(&debug_request(b"{}"), "claude-opus-4-7", &provider, true),
            "provider-default"
        );
    }

    #[test]
    fn claude_model_mapping_falls_back_to_standard_model_when_no_mapping_exists() {
        let provider = claude_provider(UpstreamModelMapping::default());

        assert_eq!(
            resolve_upstream_model_id(&debug_request(b"{}"), "claude-opus-4-7", &provider, true),
            "claude-opus-4-7"
        );
    }

    #[test]
    fn claude_model_mapping_strips_one_m_marker_before_upstream() {
        let provider = claude_provider(UpstreamModelMapping::default());

        assert_eq!(
            resolve_upstream_model_id(
                &debug_request(b"{}"),
                "claude-sonnet-4-6[1M]",
                &provider,
                true,
            ),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn claude_model_mapping_strips_one_m_marker_after_provider_mapping() {
        let provider = claude_provider(UpstreamModelMapping {
            sonnet_model: Some("provider-sonnet[1m]".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_upstream_model_id(
                &debug_request(b"{}"),
                "claude-sonnet-4-6[1M]",
                &provider,
                true,
            ),
            "provider-sonnet"
        );
    }

    #[test]
    fn claude_disabled_family_mapping_preserves_original_model_but_strips_one_m_marker() {
        let provider = claude_provider(UpstreamModelMapping {
            sonnet_model: Some("provider-sonnet[1m]".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_upstream_model_id(
                &debug_request(b"{}"),
                "claude-sonnet-4-6[1M]",
                &provider,
                false,
            ),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn codex_model_strips_one_m_marker_before_upstream() {
        let provider = provider_for_cli(GatewayCliKey::Codex);

        assert_eq!(
            resolve_upstream_model_id(&debug_request(b"{}"), "gpt-5-codex[1M]", &provider, true),
            "gpt-5-codex"
        );
    }

    #[test]
    fn gemini_model_strips_encoded_one_m_marker_before_upstream() {
        let provider = provider_for_cli(GatewayCliKey::Gemini);

        assert_eq!(
            resolve_upstream_model_id(
                &debug_request(b"{}"),
                "gemini-2.5-pro%5B1M%5D",
                &provider,
                true,
            ),
            "gemini-2.5-pro"
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
            resolve_upstream_model_id(&request, "claude-sonnet-4-6", &provider, true),
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
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::AnthropicMessages,
            None,
            false,
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
    fn upstream_body_preserves_thinking_blocks_when_model_remapped_normally() {
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
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::AnthropicMessages,
            None,
            false,
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
        assert!(value.get("thinking").is_some());
        assert_eq!(content.len(), 3);
        assert_eq!(
            content[0].get("type").and_then(Value::as_str),
            Some("thinking")
        );
        assert_eq!(
            content[1].get("type").and_then(Value::as_str),
            Some("redacted_thinking")
        );
        assert_eq!(content[2].get("type").and_then(Value::as_str), Some("text"));
        assert!(content[0].get("signature").is_some());
        assert!(content[2].get("signature").is_some());
        assert_eq!(
            content[2]
                .pointer("/meta/signature")
                .and_then(Value::as_str),
            Some("sig-c")
        );
    }

    #[test]
    fn upstream_body_preserves_thinking_reasoning_when_protocol_converted_normally() {
        let request = debug_request(
            br#"{
                "model":"claude-sonnet-4-6",
                "thinking":{"type":"enabled","budget_tokens":1024},
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
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::OpenAiChat,
            Some(ConversionRoute::new(
                AiProtocol::AnthropicMessages,
                AiProtocol::OpenAiChat,
            )),
            false,
        )
        .unwrap();
        let body_text = String::from_utf8(body.clone()).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(
            value.get("model").and_then(Value::as_str),
            Some("claude-sonnet-4-6")
        );
        assert!(value.get("reasoning_effort").is_none());
        assert_eq!(value["messages"][0]["reasoning_content"], "hidden");
        assert!(body_text.contains("visible"));
        assert!(body_text.contains("hidden"));
        assert!(!body_text.contains("sig-a"));
    }

    #[test]
    fn outbound_adapter_drops_chat_to_responses_tool_controls_without_valid_tools() {
        let request = debug_request(
            br#"{
                "model":"gpt-5.1-codex-mini",
                "messages":[{"role":"user","content":"hi"}],
                "tools":[{"type":"function","function":{"description":"missing name"}}],
                "tool_choice":"auto",
                "parallel_tool_calls":true
            }"#,
        );

        let body = build_upstream_body(
            &request,
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-mini",
            false,
            false,
            GatewayCliKey::OpenCode,
            AiProtocol::OpenAiResponses,
            Some(ConversionRoute::new(
                AiProtocol::OpenAiChat,
                AiProtocol::OpenAiResponses,
            )),
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("tools").is_none());
        assert!(value.get("tool_choice").is_none());
        assert!(value.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn outbound_adapter_drops_responses_to_chat_tool_controls_without_valid_tools() {
        let request = debug_request(
            br#"{
                "model":"gpt-5.1-codex-mini",
                "input":"hi",
                "tools":[{"type":"function","description":"missing name"}],
                "tool_choice":"auto",
                "parallel_tool_calls":true
            }"#,
        );

        let body = build_upstream_body(
            &request,
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-mini",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::OpenAiChat,
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::OpenAiChat,
            )),
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("tools").is_none());
        assert!(value.get("tool_choice").is_none());
        assert!(value.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn outbound_adapter_sanitizes_converted_responses_body_for_chat_provider_compat() {
        let body = br#"{
            "model":"kimi-k2.7-code",
            "messages":[
                {
                    "role":"system",
                    "content":[
                        {"type":"text","text":"You are Codex."},
                        {"type":"text","text":"Use tools carefully."}
                    ]
                },
                {"role":"user","content":"hi"},
                {
                    "role":"assistant",
                    "content":null,
                    "tool_calls":[
                        {
                            "id":"call_custom",
                            "type":"responses_custom_tool",
                            "function":{"name":""},
                            "response_custom_tool_call":{
                                "call_id":"call_custom",
                                "name":"apply_patch",
                                "input":"*** Begin Patch"
                            }
                        },
                        {
                            "id":"call_fn",
                            "type":"function",
                            "function":{"name":"exec_command","arguments":""}
                        }
                    ]
                },
                {
                    "role":"tool",
                    "tool_call_id":"call_custom",
                    "content":"patched"
                }
            ],
            "verbosity":"high",
            "reasoning_effort":"high",
            "prompt_cache_key":"cache-key",
            "stream":true,
            "stream_options":{"include_usage":true},
            "tools":[
                {
                    "type":"function",
                    "function":{"name":"exec_command","parameters":{}}
                },
                {
                    "type":"responses_custom_tool",
                    "function":{"name":"apply_patch"},
                    "response_custom_tool":{"name":"apply_patch"}
                }
            ],
            "tool_choice":"auto",
            "parallel_tool_calls":true
        }"#;

        let body = apply_outbound_adapter_compat(
            body.to_vec(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::OpenAiChat,
            )),
            AiProtocol::OpenAiChat,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("verbosity").is_none());
        assert!(value.get("reasoning_effort").is_none());
        assert!(value.get("prompt_cache_key").is_none());
        assert_eq!(
            value["messages"][0]["content"],
            "You are Codex.\n\nUse tools carefully."
        );
        assert_eq!(value["tools"].as_array().unwrap().len(), 1);
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(
            value["messages"][2]["tool_calls"].as_array().unwrap().len(),
            1
        );
        assert_eq!(value["messages"][2]["tool_calls"][0]["type"], "function");
        assert_eq!(
            value["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
        assert_eq!(value["messages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn outbound_adapter_drops_empty_assistant_after_custom_tool_filtering() {
        let body = br#"{
            "model":"kimi-k2.7-code",
            "messages":[
                {"role":"user","content":"apply this patch"},
                {
                    "role":"assistant",
                    "content":null,
                    "tool_calls":[
                        {
                            "id":"call_custom",
                            "type":"responses_custom_tool",
                            "function":{"name":""},
                            "response_custom_tool_call":{
                                "call_id":"call_custom",
                                "name":"apply_patch",
                                "input":"*** Begin Patch"
                            }
                        }
                    ]
                },
                {
                    "role":"tool",
                    "tool_call_id":"call_custom",
                    "content":"patched"
                }
            ],
            "tools":[
                {
                    "type":"responses_custom_tool",
                    "function":{"name":"apply_patch"},
                    "response_custom_tool":{"name":"apply_patch"}
                }
            ],
            "tool_choice":"auto",
            "parallel_tool_calls":true
        }"#;

        let body = apply_outbound_adapter_compat(
            body.to_vec(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::OpenAiChat,
            )),
            AiProtocol::OpenAiChat,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("tools").is_none());
        assert!(value.get("tool_choice").is_none());
        assert!(value.get("parallel_tool_calls").is_none());
        let messages = value["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn outbound_adapter_preserves_visible_assistant_after_custom_tool_filtering() {
        let body = br#"{
            "model":"kimi-k2.7-code",
            "messages":[
                {"role":"user","content":"apply this patch"},
                {
                    "role":"assistant",
                    "content":"I'll update that.",
                    "tool_calls":[
                        {
                            "id":"call_custom",
                            "type":"responses_custom_tool",
                            "function":{"name":""},
                            "response_custom_tool_call":{
                                "call_id":"call_custom",
                                "name":"apply_patch",
                                "input":"*** Begin Patch"
                            }
                        }
                    ]
                },
                {
                    "role":"tool",
                    "tool_call_id":"call_custom",
                    "content":"patched"
                }
            ]
        }"#;

        let body = apply_outbound_adapter_compat(
            body.to_vec(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::OpenAiChat,
            )),
            AiProtocol::OpenAiChat,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let messages = value["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "I'll update that.");
        assert!(messages[1].get("tool_calls").is_none());
    }

    #[test]
    fn outbound_adapter_preserves_reasoning_assistant_after_custom_tool_filtering() {
        let body = br#"{
            "model":"kimi-k2.7-code",
            "messages":[
                {"role":"user","content":"think and patch"},
                {
                    "role":"assistant",
                    "content":null,
                    "reasoning":"Need to edit one file.",
                    "tool_calls":[
                        {
                            "id":"call_custom",
                            "type":"responses_custom_tool",
                            "function":{"name":""},
                            "response_custom_tool_call":{
                                "call_id":"call_custom",
                                "name":"apply_patch",
                                "input":"*** Begin Patch"
                            }
                        }
                    ]
                },
                {
                    "role":"tool",
                    "tool_call_id":"call_custom",
                    "content":"patched"
                }
            ]
        }"#;

        let body = apply_outbound_adapter_compat(
            body.to_vec(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::OpenAiChat,
            )),
            AiProtocol::OpenAiChat,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let messages = value["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["reasoning"], "Need to edit one file.");
        assert!(messages[1].get("tool_calls").is_none());
    }

    #[test]
    fn outbound_adapter_preserves_direct_chat_extensions_without_conversion() {
        let body = br#"{
            "model":"kimi-k2.7-code",
            "_internal":"drop",
            "messages":[{"role":"user","content":"hi"}],
            "verbosity":"high",
            "reasoning_effort":"high",
            "prompt_cache_key":"cache-key",
            "tools":[
                {
                    "type":"responses_custom_tool",
                    "function":{"name":"apply_patch"},
                    "response_custom_tool":{"name":"apply_patch"}
                }
            ],
            "tool_choice":"auto",
            "parallel_tool_calls":true
        }"#;

        let body =
            apply_outbound_adapter_compat(body.to_vec(), None, AiProtocol::OpenAiChat).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("_internal").is_none());
        assert_eq!(value["verbosity"], "high");
        assert_eq!(value["reasoning_effort"], "high");
        assert_eq!(value["prompt_cache_key"], "cache-key");
        assert_eq!(value["tools"][0]["type"], "responses_custom_tool");
        assert_eq!(value["tool_choice"], "auto");
        assert_eq!(value["parallel_tool_calls"], true);
    }

    #[test]
    fn outbound_adapter_preserves_function_tools_and_named_tool_choice() {
        let body = br#"{
            "model":"kimi-k2.7-code",
            "messages":[{"role":"user","content":"hi"}],
            "tools":[
                {
                    "type":"function",
                    "function":{"name":"lookup","parameters":{"type":"object"}},
                    "response_custom_tool":{"name":"should_not_leak"}
                }
            ],
            "tool_choice":{"type":"function","function":{"name":"lookup"}},
            "parallel_tool_calls":true
        }"#;

        let body = apply_outbound_adapter_compat(
            body.to_vec(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::OpenAiChat,
            )),
            AiProtocol::OpenAiChat,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["tools"].as_array().unwrap().len(), 1);
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tools"][0]["function"]["name"], "lookup");
        assert!(value["tools"][0].get("response_custom_tool").is_none());
        assert_eq!(value["tool_choice"]["function"]["name"], "lookup");
        assert_eq!(value["parallel_tool_calls"], true);
    }

    #[test]
    fn outbound_adapter_removes_anthropic_thinking_when_tool_choice_forces_tool_use() {
        let body = br#"{
            "model":"claude-sonnet-4-5",
            "max_tokens":8192,
            "messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}],
            "thinking":{"type":"enabled","budget_tokens":30000},
            "tools":[{"name":"lookup","input_schema":{"type":"object"}}],
            "tool_choice":{"type":"any"}
        }"#;

        let body = apply_outbound_adapter_compat(
            body.to_vec(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiChat,
                AiProtocol::AnthropicMessages,
            )),
            AiProtocol::AnthropicMessages,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("thinking").is_none());
        assert_eq!(value["tool_choice"]["type"], "any");
    }

    #[test]
    fn outbound_adapter_drops_chat_to_anthropic_tool_choice_without_valid_tools() {
        let request = debug_request(
            br#"{
                "model":"gpt-5.1-codex-mini",
                "messages":[{"role":"user","content":"hi"}],
                "tools":[{"type":"function","function":{"description":"missing name"}}],
                "tool_choice":"required"
            }"#,
        );

        let body = build_upstream_body(
            &request,
            "gpt-5.1-codex-mini",
            "claude-sonnet-4-6",
            false,
            false,
            GatewayCliKey::OpenCode,
            AiProtocol::AnthropicMessages,
            Some(ConversionRoute::new(
                AiProtocol::OpenAiChat,
                AiProtocol::AnthropicMessages,
            )),
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("tools").is_none());
        assert!(value.get("tool_choice").is_none());
    }

    #[test]
    fn outbound_adapter_preserves_gemini_derived_tool_choice_without_tools() {
        let request = debug_request(
            br#"{
                "contents":[{"role":"user","parts":[{"text":"hi"}]}],
                "toolConfig":{"functionCallingConfig":{"mode":"AUTO"}}
            }"#,
        );

        let body = build_upstream_body(
            &request,
            "gemini-2.5-flash",
            "gpt-5.1-codex-mini",
            false,
            false,
            GatewayCliKey::Gemini,
            AiProtocol::OpenAiChat,
            Some(ConversionRoute::new(
                AiProtocol::GeminiNative,
                AiProtocol::OpenAiChat,
            )),
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("tools").is_none());
        assert_eq!(value["tool_choice"], "auto");
    }

    #[test]
    fn outbound_adapter_filters_private_fields_on_direct_json_body() {
        let request = debug_request(
            br#"{
                "model":"gpt-5.1-codex-mini",
                "_internal":"drop",
                "messages":[
                    {
                        "role":"user",
                        "content":"hi",
                        "_debug":"drop"
                    }
                ],
                "tools":[
                    {
                        "type":"function",
                        "function":{
                            "name":"lookup",
                            "_internal":"drop",
                            "parameters":{
                                "type":"object",
                                "_debug":"drop",
                                "properties":{
                                    "_id":{
                                        "type":"string",
                                        "_internal_note":"drop",
                                        "properties":{
                                            "_nested":{"type":"string"}
                                        }
                                    }
                                },
                                "$defs":{
                                    "_shared":{
                                        "type":"object",
                                        "_internal_note":"drop"
                                    }
                                }
                            }
                        }
                    }
                ]
            }"#,
        );

        let body = build_upstream_body(
            &request,
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-mini",
            false,
            false,
            GatewayCliKey::OpenCode,
            AiProtocol::OpenAiChat,
            None,
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("_internal").is_none());
        assert!(value.pointer("/messages/0/_debug").is_none());
        assert!(value.pointer("/tools/0/function/_internal").is_none());
        assert!(value
            .pointer("/tools/0/function/parameters/_debug")
            .is_none());
        assert!(value
            .pointer("/tools/0/function/parameters/properties/_id")
            .is_some());
        assert!(value
            .pointer("/tools/0/function/parameters/properties/_id/_internal_note")
            .is_none());
        assert!(value
            .pointer("/tools/0/function/parameters/properties/_id/properties/_nested")
            .is_some());
        assert!(value
            .pointer("/tools/0/function/parameters/$defs/_shared")
            .is_some());
        assert!(value
            .pointer("/tools/0/function/parameters/$defs/_shared/_internal_note")
            .is_none());
    }

    #[test]
    fn provider_body_compat_detects_canonical_provider_type_aliases() {
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some(" DeepSeek ")),
            Some(ProviderBodyCompat::DeepSeek)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("model_scope")),
            Some(ProviderBodyCompat::ModelScope)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("glm")),
            Some(ProviderBodyCompat::Zai)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("x-ai")),
            Some(ProviderBodyCompat::Xai)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("aliyun")),
            Some(ProviderBodyCompat::Bailian)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("xiaomi_mimo")),
            Some(ProviderBodyCompat::Mimo)
        );
        assert_eq!(ProviderBodyCompat::from_provider_type(Some("custom")), None);
    }

    #[test]
    fn provider_body_compat_deepseek_chat_rewrites_json_schema_thinking_and_custom_tools() {
        let body = br#"{
            "model":"deepseek-v4-pro",
            "reasoning_effort":"medium",
            "response_format":{
                "type":"json_schema",
                "json_schema":{"name":"answer","schema":{"type":"object"}}
            },
            "messages":[
                {"role":"user","content":"hi"},
                {
                    "role":"assistant",
                    "content":null,
                    "tool_calls":[
                        {
                            "id":"call_custom",
                            "type":"responses_custom_tool",
                            "function":{"name":""},
                            "response_custom_tool_call":{"call_id":"call_custom","name":"patch","input":"x"}
                        },
                        {
                            "id":"call_empty_args",
                            "type":"function",
                            "function":{"name":"exec_command","arguments":""}
                        },
                        {
                            "id":"call_fn",
                            "type":"function",
                            "function":{"name":"lookup","arguments":"{}"}
                        }
                    ]
                },
                {"role":"tool","tool_call_id":"call_custom","content":"patched"}
            ],
            "tools":[
                {"type":"responses_custom_tool","function":{"name":"patch"},"response_custom_tool":{"name":"patch"}},
                {"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}
            ],
            "tool_choice":"auto"
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("deepseek"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["response_format"]["type"], "json_object");
        assert!(value["response_format"].get("json_schema").is_none());
        assert_eq!(value["thinking"]["type"], "enabled");
        assert_eq!(value["reasoning_effort"], "high");
        assert_eq!(value["tools"].as_array().unwrap().len(), 1);
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(
            value["messages"][1]["tool_calls"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            value["messages"][1]["tool_calls"][0]["id"],
            "call_empty_args"
        );
        assert_eq!(
            value["messages"][1]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
        assert_eq!(value["messages"][1]["tool_calls"][1]["id"], "call_fn");
        assert_eq!(value["messages"][1]["reasoning_content"], "");
        assert_eq!(value["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn provider_body_compat_zai_chat_moves_metadata_and_forces_auto_tool_choice() {
        let body = br#"{
            "model":"glm-4.7",
            "reasoning_effort":"none",
            "metadata":{"user_id":"user-1","request_id":"req-1","ignored":"x"},
            "response_format":{"type":"json_schema","json_schema":{"name":"x","schema":{}}},
            "tool_choice":{"type":"function","function":{"name":"lookup"}},
            "messages":[{"role":"user","content":"hi"}]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("zhipu"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("metadata").is_none());
        assert_eq!(value["user_id"], "user-1");
        assert_eq!(value["request_id"], "req-1");
        assert_eq!(value["tool_choice"], "auto");
        assert_eq!(value["thinking"]["type"], "disabled");
        assert_eq!(value["response_format"]["type"], "json_object");
        assert!(value["response_format"].get("json_schema").is_none());

        let body = br#"{
            "model":"glm-4.7",
            "metadata":{"user_id":"user-1"},
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("glm"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        assert!(value["request_id"]
            .as_str()
            .is_some_and(|request_id| request_id.starts_with("req_")));
    }

    #[test]
    fn provider_body_compat_doubao_chat_extracts_metadata_and_generates_request_id() {
        let body = br#"{
            "model":"doubao-seed-code",
            "metadata":{"user_id":"user-1"},
            "messages":[{"role":"user","content":"hi"}]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("doubao"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("metadata").is_none());
        assert_eq!(value["user_id"], "user-1");
        assert!(value["request_id"]
            .as_str()
            .is_some_and(|request_id| request_id.starts_with("req_")));
    }

    #[test]
    fn provider_body_compat_xai_chat_strips_model_specific_unsupported_fields() {
        let body = br#"{
            "model":"grok-4",
            "reasoning_effort":"high",
            "presence_penalty":0.1,
            "frequency_penalty":0.2,
            "stop":["END"],
            "temperature":0.5,
            "messages":[{"role":"user","content":"hi"}]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("grok"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("reasoning_effort").is_none());
        assert!(value.get("presence_penalty").is_none());
        assert!(value.get("frequency_penalty").is_none());
        assert!(value.get("stop").is_none());
        assert_eq!(value["temperature"], 0.5);
    }

    #[test]
    fn provider_body_compat_longcat_chat_forces_message_content_arrays() {
        let body = br#"{
            "model":"longcat-flash",
            "messages":[
                {"role":"system","content":"rules"},
                {"role":"user","content":null},
                {"role":"assistant","content":{"type":"text","text":"ok"}},
                {"role":"user","content":42}
            ]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("longcat"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["messages"][0]["content"][0]["text"], "rules");
        assert_eq!(value["messages"][1]["content"][0]["text"], "");
        assert_eq!(value["messages"][2]["content"][0]["type"], "text");
        assert_eq!(value["messages"][2]["content"][0]["text"], "ok");
        assert_eq!(value["messages"][3]["content"][0]["text"], "42");
    }

    #[test]
    fn provider_body_compat_bailian_chat_merges_consecutive_tool_call_messages() {
        let body = br#"{
            "model":"qwen3-coder",
            "messages":[
                {"role":"user","content":"use tools"},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call_a","type":"function","function":{"name":"a","arguments":"{}"}}]},
                {"role":"assistant","content":"","tool_calls":[{"id":"call_b","type":"function","function":{"name":"b","arguments":"{}"}}]},
                {"role":"assistant","content":"done","tool_calls":[{"id":"call_c","type":"function","function":{"name":"c","arguments":"{}"}}]}
            ]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("bailian"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let messages = value["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_a");
        assert_eq!(messages[1]["tool_calls"][1]["id"], "call_b");
        assert_eq!(messages[2]["content"], "done");
        assert_eq!(messages[2]["tool_calls"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn provider_body_compat_bailian_keeps_tool_call_messages_with_side_effect_fields() {
        let body = br#"{
            "model":"qwen3-coder",
            "messages":[
                {"role":"assistant","content":null,"message_index":1,"tool_calls":[{"id":"call_a","type":"function","function":{"name":"a","arguments":"{}"}}]},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call_b","type":"function","function":{"name":"b","arguments":"{}"}}]}
            ]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some("aliyun"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let messages = value["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["message_index"], 1);
        assert_eq!(messages[0]["tool_calls"][0]["id"], "call_a");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_b");
    }

    #[test]
    fn provider_body_compat_anthropic_reasoning_vendor_normalizes_tool_thinking_history() {
        let body = br#"{
            "model":"kimi-for-coding",
            "messages":[
                {
                    "role":"assistant",
                    "content":[
                        {"type":"tool_use","id":"call_a","name":"read","input":{}}
                    ]
                },
                {
                    "role":"assistant",
                    "content":[
                        {"type":"redacted_thinking","data":"opaque"},
                        {"type":"tool_use","id":"call_b","name":"write","input":{}}
                    ]
                },
                {
                    "role":"assistant",
                    "content":[
                        {"type":"thinking","thinking":"Need to inspect.","signature":"sig"},
                        {"type":"tool_use","id":"call_c","name":"grep","input":{}}
                    ]
                }
            ]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::AnthropicMessages,
            Some("moonshot"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["messages"][0]["content"][0]["type"], "thinking");
        assert_eq!(value["messages"][0]["content"][0]["thinking"], "tool call");
        assert_eq!(value["messages"][1]["content"][0]["type"], "thinking");
        assert_eq!(
            value["messages"][1]["content"][0]["thinking"],
            "[redacted thinking]"
        );
        assert_eq!(
            value["messages"][2]["content"][0]["thinking"],
            "Need to inspect."
        );
        assert!(value["messages"][2]["content"][0]
            .get("signature")
            .is_none());
    }

    #[test]
    fn provider_body_compat_deepseek_anthropic_disabled_thinking_strips_effort_fields() {
        let body = br#"{
            "model":"deepseek-v4-pro",
            "thinking":{"type":"disabled"},
            "output_config":{"effort":"max","temperature":0.2},
            "reasoning_effort":"high",
            "messages":[{"role":"user","content":"hi"}]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::AnthropicMessages,
            Some("deepseek"),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["thinking"]["type"], "disabled");
        assert!(value.get("reasoning_effort").is_none());
        assert_eq!(value["output_config"]["temperature"], 0.2);
        assert!(value["output_config"].get("effort").is_none());
    }

    #[test]
    fn thinking_signature_rectifier_strips_top_level_messages_only() {
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
            AiProtocol::AnthropicMessages,
            None,
            false,
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
    fn thinking_signature_rectifier_rebuilds_converted_body_after_matching_error() {
        let request = debug_request(
            br#"{
                "model":"gpt-5",
                "thinking":{"type":"enabled","budget_tokens":1024},
                "output_config":{"effort":"high"},
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
        let route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        let conversion_route = Some(ConversionRoute::new(
            AiProtocol::AnthropicMessages,
            AiProtocol::OpenAiResponses,
        ));
        let original_body = build_upstream_body(
            &request,
            "gpt-5",
            "gpt-5",
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::OpenAiResponses,
            conversion_route,
            false,
        )
        .unwrap();
        let rectified_body = build_thinking_signature_rectified_upstream_body(
            &request,
            "gpt-5",
            "gpt-5",
            false,
            &route,
            AiProtocol::OpenAiResponses,
            conversion_route,
            None,
            false,
            &original_body,
        )
        .unwrap()
        .expect("thinking/signature cleanup should change converted body");
        let rectified_body = rectified_body.body;
        let original = serde_json::from_slice::<Value>(&original_body).unwrap();
        let rectified = serde_json::from_slice::<Value>(&rectified_body).unwrap();

        assert_eq!(original["reasoning"]["effort"], "high");
        assert_eq!(rectified["reasoning"]["effort"], "high");
        assert!(
            original.to_string().contains("hidden"),
            "normal conversion should preserve thinking text"
        );
        assert!(
            !rectified.to_string().contains("hidden"),
            "rectifier retry should remove thinking text"
        );
        assert!(
            !rectified.to_string().contains("sig-a"),
            "rectifier retry should remove thinking signatures"
        );
        assert!(rectified.to_string().contains("visible"));
    }

    #[test]
    fn thinking_signature_rectifier_matches_only_thinking_signature_errors() {
        assert!(should_rectify_thinking_signature(
            400,
            br#"{"error":{"message":"Invalid 'signature' in 'thinking' block"}}"#
        ));
        assert!(should_rectify_thinking_signature(
            400,
            br#"{"detail":"Expected `thinking` or `redacted_thinking`, but found `tool_use`"}"#
        ));
        assert!(should_rectify_thinking_signature(
            400,
            br#"{"base_resp":{"status_msg":"Thought signature is not valid"}}"#
        ));

        assert!(!should_rectify_thinking_signature(
            400,
            br#"{"error":{"message":"Invalid JSON schema: strict must be a boolean"}}"#
        ));
        assert!(!should_rectify_thinking_signature(
            500,
            br#"{"error":{"message":"Invalid 'signature' in 'thinking' block"}}"#
        ));
    }

    #[test]
    fn unsupported_media_rectifier_matches_only_media_compat_errors() {
        assert!(should_rectify_unsupported_media(
            400,
            br#"{"error":{"message":"This model does not support image input"}}"#
        ));
        assert!(should_rectify_unsupported_media(
            415,
            br#"{"detail":"invalid content type for attachment"}"#
        ));
        assert!(should_rectify_unsupported_media(
            422,
            br#"{"message":"model is text-only"}"#
        ));

        assert!(!should_rectify_unsupported_media(
            500,
            br#"{"error":{"message":"This model does not support image input"}}"#
        ));
        assert!(!should_rectify_unsupported_media(
            400,
            br#"{"error":{"message":"Invalid JSON schema: strict must be a boolean"}}"#
        ));
    }

    #[test]
    fn unsupported_media_rectifier_replaces_images_and_preserves_cache_control() {
        let rectified = build_unsupported_media_rectified_body(
            br#"{
                "messages":[
                    {
                        "role":"user",
                        "content":[
                            {
                                "type":"image",
                                "source":{"type":"base64","media_type":"image/png","data":"aaa"},
                                "cache_control":{"type":"ephemeral"}
                            },
                            {"type":"image_url","image_url":{"url":"https://example.com/a.png"}},
                            {"type":"input_image","image_url":"https://example.com/b.png"}
                        ]
                    }
                ],
                "contents":[
                    {
                        "parts":[
                            {"inlineData":{"mimeType":"image/png","data":"aaa"}},
                            {"fileData":{"mimeType":"application/pdf","fileUri":"file.pdf"}},
                            {"fileData":{"mimeType":"image/jpeg","fileUri":"file.jpg"}}
                        ]
                    }
                ]
            }"#,
        )
        .unwrap()
        .expect("image blocks should be replaced");
        let value = serde_json::from_slice::<Value>(&rectified).unwrap();

        assert_eq!(value["messages"][0]["content"][0]["type"], "text");
        assert_eq!(
            value["messages"][0]["content"][0]["text"],
            UNSUPPORTED_IMAGE_MARKER
        );
        assert_eq!(
            value["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(value["messages"][0]["content"][1]["type"], "text");
        assert_eq!(value["messages"][0]["content"][2]["type"], "input_text");
        assert_eq!(
            value["contents"][0]["parts"][0]["text"],
            UNSUPPORTED_IMAGE_MARKER
        );
        assert!(value["contents"][0]["parts"][1].get("fileData").is_some());
        assert_eq!(
            value["contents"][0]["parts"][2]["text"],
            UNSUPPORTED_IMAGE_MARKER
        );
    }

    #[test]
    fn unsupported_media_rectifier_skips_body_without_images() {
        assert!(build_unsupported_media_rectified_body(
            br#"{"messages":[{"role":"user","content":"hi"}]}"#
        )
        .unwrap()
        .is_none());
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
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::AnthropicMessages,
            None,
            false,
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
    fn upstream_body_strips_one_m_marker_without_treating_it_as_model_remap() {
        let request = debug_request(
            br#"{
                "model":"claude-sonnet-4-6[1M]",
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
            "claude-sonnet-4-6[1M]",
            "claude-sonnet-4-6[1M]",
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::AnthropicMessages,
            None,
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let content = value
            .pointer("/messages/0/content")
            .and_then(Value::as_array)
            .unwrap();

        assert_eq!(
            value.get("model").and_then(Value::as_str),
            Some("claude-sonnet-4-6")
        );
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
    fn codex_upstream_body_strips_one_m_marker_without_claude_thinking_rectifier() {
        let request = debug_request(
            br#"{
                "model":"gpt-5-codex[1M]",
                "thinking":{"type":"enabled"},
                "messages":[
                    {
                        "role":"assistant",
                        "content":[{"type":"thinking","thinking":"codex-owned"}]
                    }
                ]
            }"#,
        );

        let body = build_upstream_body(
            &request,
            "gpt-5-codex[1M]",
            "gpt-5-codex",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::OpenAiResponses,
            None,
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(
            value.get("model").and_then(Value::as_str),
            Some("gpt-5-codex")
        );
        assert!(value.get("thinking").is_some());
        assert_eq!(
            value
                .pointer("/messages/0/content/0/type")
                .and_then(Value::as_str),
            Some("thinking")
        );
    }

    #[test]
    fn gemini_forwarded_path_strips_one_m_marker_from_model_segment() {
        let route = gateway_route(
            GatewayCliKey::Gemini,
            "/v1beta/models/gemini-2.5-pro[1M]:generateContent",
        );
        let provider = provider_for_cli(GatewayCliKey::Gemini);

        assert_eq!(
            upstream_forwarded_path(&route, &provider, None, "gemini-2.5-pro", false),
            "/v1beta/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn gemini_forwarded_path_strips_encoded_one_m_marker_from_model_segment() {
        let route = gateway_route(
            GatewayCliKey::Gemini,
            "/v1beta/models/gemini-2.5-pro%5B1M%5D:streamGenerateContent",
        );
        let provider = provider_for_cli(GatewayCliKey::Gemini);

        assert_eq!(
            upstream_forwarded_path(&route, &provider, None, "gemini-2.5-pro", false),
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent"
        );
    }

    #[test]
    fn conversion_route_rewrites_claude_to_gemini_native_generate_content_path() {
        let route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::GeminiNative,
            auth_strategy: ProviderAuthStrategy::GoogleApiKey,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            ..provider_for_cli(GatewayCliKey::Claude)
        };
        let source_protocol = source_protocol_from_route(&route).unwrap();
        let conversion = conversion_route(source_protocol, &provider);

        assert_eq!(
            upstream_forwarded_path(
                &route,
                &provider,
                conversion,
                "models/gemini-2.5-pro[1M]",
                false,
            )
            .as_ref(),
            "/v1beta/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn conversion_route_rewrites_claude_to_gemini_native_streaming_path_and_query() {
        let mut route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        route.query = Some("beta=true&x-id=1".to_string());
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::GeminiNative,
            auth_strategy: ProviderAuthStrategy::GoogleApiKey,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            ..provider_for_cli(GatewayCliKey::Claude)
        };
        let source_protocol = source_protocol_from_route(&route).unwrap();
        let conversion = conversion_route(source_protocol, &provider);
        let forwarded_path =
            upstream_forwarded_path(&route, &provider, conversion, "gemini-2.5-flash", true);
        let url = build_provider_target_url(
            &provider,
            forwarded_path.as_ref(),
            route.query.as_deref(),
            conversion,
            true,
        )
        .unwrap();

        assert_eq!(
            forwarded_path.as_ref(),
            "/v1beta/models/gemini-2.5-flash:streamGenerateContent"
        );
        assert_eq!(
            url.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?x-id=1&alt=sse"
        );
    }

    #[test]
    fn gemini_stream_route_to_anthropic_sets_target_stream_flag_and_model() {
        let request = debug_request(br#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#);
        let body = build_upstream_body(
            &request,
            "gemini-2.5-pro",
            "claude-sonnet-4-6",
            false,
            false,
            GatewayCliKey::Gemini,
            AiProtocol::AnthropicMessages,
            Some(ConversionRoute::new(
                AiProtocol::GeminiNative,
                AiProtocol::AnthropicMessages,
            )),
            true,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["model"], "claude-sonnet-4-6");
        assert_eq!(value["stream"], true);
        assert_eq!(value["messages"][0]["content"][0]["text"], "hi");
    }

    #[test]
    fn cache_injection_applies_after_codex_to_anthropic_conversion() {
        let request = debug_request(
            br#"{
                "model":"gpt-5-codex",
                "input":[
                    {
                        "role":"user",
                        "content":[{"type":"input_text","text":"hi"}]
                    }
                ]
            }"#,
        );

        let body = build_upstream_body(
            &request,
            "gpt-5-codex",
            "claude-sonnet-4-6",
            false,
            true,
            GatewayCliKey::Codex,
            AiProtocol::AnthropicMessages,
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::AnthropicMessages,
            )),
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["model"], "claude-sonnet-4-6");
        assert_eq!(value["messages"][0]["content"][0]["text"], "hi");
        assert_eq!(
            value["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn thinking_budget_rectifier_targets_anthropic_protocol_even_for_converted_routes() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let headers = HeaderMap::new();
        let request = debug_request(br#"{"model":"gpt-5-codex","input":"hi"}"#);

        assert!(should_attempt_thinking_budget_rectifier(
            true,
            AiProtocol::AnthropicMessages,
            &request,
            &route,
            &headers,
            400,
        ));
        assert!(!should_attempt_thinking_budget_rectifier(
            true,
            AiProtocol::OpenAiResponses,
            &request,
            &route,
            &headers,
            400,
        ));
        assert!(!should_attempt_thinking_budget_rectifier(
            false,
            AiProtocol::AnthropicMessages,
            &request,
            &route,
            &headers,
            400,
        ));
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
