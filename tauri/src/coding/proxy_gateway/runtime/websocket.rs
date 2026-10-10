//! Codex Responses WebSocket transport for single/failover providers.
//! Aggregate mode uses HTTP/SSE because its model is only known after the
//! downstream WebSocket handshake.

use super::compat::codex_remote_compaction::{
    self as codex_remote_compaction, CompactionStreamSummary, RemoteCompactionCompat,
};
use super::compat::provider_kind::ProviderBodyCompat;
use super::http_io::{
    empty_response, header_value, write_response, DebugHttpRequest, DebugHttpResponse,
};
use super::observability::record_gateway_observability_with_transport;
use super::providers::{GatewayProviderSelection, UpstreamProvider};
use super::routes::{match_gateway_route, GatewayRoute};
use super::{upstream, GatewayRuntimeContext, NEXT_REQUEST_ID};
use crate::coding::proxy_gateway::model_health::GatewayFailureKind;
use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::coding::proxy_gateway::types::{
    GatewayCliKey, GatewayProviderAttempt, GatewayProxyMode, GatewayRequestKind,
    GatewayRequestTransport, GatewayStreamOutcome, GatewayWebSocketMetadata, ProviderHealthKey,
    ProviderModelHealthKey, ProxyGatewaySettings,
};
use crate::coding::proxy_gateway::usage_parser::{
    classify_sse_event_fields, from_response_body, SseTerminalKind,
};
use crate::http_client;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use http::{HeaderMap, HeaderValue, Request, StatusCode};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{
    handshake::{client::generate_key, derive_accept_key, server},
    protocol::{Role, WebSocketConfig},
    Message,
};
use tokio_tungstenite::WebSocketStream;

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PENDING_REQUESTS: usize = 64;
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(50 * 60);
const CONNECTION_MAX_LIFETIME: Duration = Duration::from_secs(55 * 60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const WEBSOCKET_BETA: &str = "responses_websockets=2026-02-06";
const WEBSOCKET_DISABLED_REASON: &str =
    "Codex WebSocket support is disabled in gateway settings; use HTTP/SSE.";
const AGGREGATE_WEBSOCKET_REASON: &str =
    "Codex aggregate mode uses HTTP/SSE because the model is selected after the WebSocket handshake.";

fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .write_buffer_size(0)
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}

pub(super) fn is_upgrade_request(request: &DebugHttpRequest) -> bool {
    header_value(&request.headers, "upgrade")
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        || header_value(&request.headers, "sec-websocket-key").is_some()
}

fn is_codex_responses_route(route: &GatewayRoute) -> bool {
    route.cli_key == GatewayCliKey::Codex
        && matches!(
            route.forwarded_path.as_str(),
            "/v1/responses" | "/responses"
        )
}

fn fallback_reason(provider: &UpstreamProvider) -> Option<&'static str> {
    if provider.target_protocol != AiProtocol::OpenAiResponses {
        return Some("The selected provider requires protocol conversion; use HTTP/SSE.");
    }
    if ProviderBodyCompat::from_provider_meta(Some(&provider.meta), provider.target_protocol)
        == Some(ProviderBodyCompat::Copilot)
    {
        return Some("This provider selects its protocol per request; use HTTP/SSE.");
    }
    if provider.supports_websockets == Some(false) {
        return Some("WebSocket transport is disabled for the selected provider; use HTTP/SSE.");
    }
    None
}

struct HandshakeFailure {
    status: u16,
    upstream_status: Option<u16>,
    message: String,
    headers: HeaderMap,
    body: Vec<u8>,
    attempts: Vec<GatewayProviderAttempt>,
}

impl HandshakeFailure {
    fn local(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            upstream_status: None,
            message: message.into(),
            headers: HeaderMap::new(),
            body: Vec::new(),
            attempts: Vec::new(),
        }
    }
}

fn header_has_token(headers: &HeaderMap, name: &str, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
        })
}

fn validate_upstream_handshake(headers: &HeaderMap, key: &str) -> bool {
    header_has_token(headers, "connection", "upgrade")
        && header_has_token(headers, "upgrade", "websocket")
        && headers.get("sec-websocket-accept").and_then(|value| value.to_str().ok())
            == Some(derive_accept_key(key.as_bytes()).as_str())
        // No extensions/subprotocols were offered: accepting either would make
        // the uncompressed frame reader interpret a different wire protocol.
        && !headers.contains_key("sec-websocket-extensions")
        && !headers.contains_key("sec-websocket-protocol")
}

async fn connect_upstream(
    request: &DebugHttpRequest,
    provider: &UpstreamProvider,
    context: &GatewayRuntimeContext,
    route: &GatewayRoute,
) -> Result<(WebSocketStream<reqwest::Upgraded>, HeaderMap, String), HandshakeFailure> {
    if !context.settings_snapshot().codex_websocket_enabled {
        return Err(HandshakeFailure::local(426, WEBSOCKET_DISABLED_REASON));
    }
    let db = context
        .db
        .as_ref()
        .ok_or_else(|| HandshakeFailure::local(503, "Gateway database is unavailable"))?;
    let mut url = upstream::websocket_provider_url(provider, route)
        .map_err(|error| HandshakeFailure::local(502, error))?;
    let scheme = match url.scheme() {
        "ws" | "http" => "http",
        "wss" | "https" => "https",
        _ => {
            return Err(HandshakeFailure::local(
                502,
                "Unsupported WebSocket upstream URL",
            ))
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| HandshakeFailure::local(502, "Invalid upstream URL scheme"))?;
    let mut display_url = url.clone();
    let _ = display_url.set_scheme(if scheme == "https" { "wss" } else { "ws" });
    let client = http_client::client_websocket_handshake(db)
        .await
        .map_err(|error| HandshakeFailure::local(502, error))?;
    let mut headers = upstream::build_upstream_headers(request, provider, None)
        .map_err(|error| HandshakeFailure::local(400, error))?
        .map;
    let keys: Vec<_> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("sec-websocket-"))
        .cloned()
        .collect();
    for name in keys {
        headers.remove(name);
    }
    for name in [
        "content-length",
        "content-type",
        "content-encoding",
        "transfer-encoding",
        "host",
        "accept",
    ] {
        headers.remove(name);
    }
    let key = generate_key();
    headers.insert("connection", HeaderValue::from_static("Upgrade"));
    headers.insert("upgrade", HeaderValue::from_static("websocket"));
    headers.insert("sec-websocket-version", HeaderValue::from_static("13"));
    headers.insert(
        "sec-websocket-key",
        HeaderValue::from_str(&key).expect("generated WebSocket key"),
    );
    if !headers.contains_key("openai-beta") {
        headers.insert("openai-beta", HeaderValue::from_static(WEBSOCKET_BETA));
    }
    let handshake_timeout = Duration::from_secs(
        context
            .settings_snapshot()
            .effective_app_config(GatewayCliKey::Codex)
            .streaming_first_byte_timeout_secs
            .max(1),
    );
    let mut response = timeout(
        handshake_timeout,
        client
            .get(url)
            .version(http::Version::HTTP_11)
            .headers(headers)
            .send(),
    )
    .await
    .map_err(|_| HandshakeFailure::local(504, "WebSocket upstream handshake timed out"))?
    .map_err(|error| {
        HandshakeFailure::local(
            502,
            format!(
                "WebSocket upstream handshake failed: {}",
                error.without_url()
            ),
        )
    })?;
    let status = response.status();
    let headers = response.headers().clone();
    if status != StatusCode::SWITCHING_PROTOCOLS {
        let mut body = Vec::new();
        let _ = timeout(Duration::from_secs(5), async {
            while body.len() < 16 * 1024 {
                let Some(chunk) = response.chunk().await? else {
                    break;
                };
                body.extend_from_slice(&chunk[..chunk.len().min(16 * 1024 - body.len())]);
            }
            Ok::<_, reqwest::Error>(())
        })
        .await;
        let unsupported = matches!(status.as_u16(), 200..=399 | 404 | 405 | 426 | 501);
        return Err(HandshakeFailure {
            status: if unsupported { 426 } else { status.as_u16() },
            upstream_status: Some(status.as_u16()),
            message: if unsupported {
                "Upstream does not support a Responses WebSocket upgrade; use HTTP/SSE.".to_string()
            } else {
                format!("Upstream rejected the WebSocket handshake ({status})")
            },
            headers,
            body,
            attempts: Vec::new(),
        });
    }
    if !validate_upstream_handshake(&headers, &key) {
        let mut failure = HandshakeFailure::local(502, "Invalid upstream WebSocket handshake");
        failure.upstream_status = Some(101);
        return Err(failure);
    }
    let upgraded = response.upgrade().await.map_err(|error| {
        HandshakeFailure::local(
            502,
            format!(
                "Could not upgrade upstream connection: {}",
                error.without_url()
            ),
        )
    })?;
    Ok((
        WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(socket_config())).await,
        headers,
        display_url.to_string(),
    ))
}

/// 判定本次握手是否属于“单渠道部署”，即是否可以跳过冷却/健康过滤。
///
/// 与 HTTP 通道 `runtime/upstream.rs` 的 `is_single_provider` 保持同一语义：
/// 只有候选数恰好为 1，且（不是聚合模式，或聚合模式打开了跨站故障转移）时，
/// 才把唯一候选无条件尝试。聚合模式 + 跨站开关关闭时候选被有意收窄到一个
/// 站点，此时冷却过滤必须继续生效，不能硬打冷却中的站点。
fn is_single_provider_deployment(
    candidate_count: usize,
    selection: Option<&GatewayProviderSelection>,
) -> bool {
    candidate_count == 1
        && selection
            .filter(|selection| selection.mode == GatewayProxyMode::Aggregate)
            .is_none_or(|selection| selection.cross_site_failover)
}

pub(super) async fn handle_upgrade(
    stream: &mut TcpStream,
    mut request: DebugHttpRequest,
    context: &GatewayRuntimeContext,
) -> io::Result<()> {
    let started_at = Utc::now();
    let started = Instant::now();
    let connection_id = uuid::Uuid::new_v4().to_string();
    let initial_frames = std::mem::take(&mut request.body);
    let mut handshake_request = Request::builder()
        .method(request.method.as_str())
        .uri(&request.path);
    for (name, value) in &request.headers {
        handshake_request = handshake_request.header(name, value);
    }
    let downstream_response = handshake_request
        .body(())
        .ok()
        .and_then(|request| server::create_response(&request).ok());
    let Some(mut downstream_response) = downstream_response else {
        return write_handshake_failure(
            stream,
            &request,
            context,
            &connection_id,
            started_at,
            started,
            None,
            HandshakeFailure::local(400, "Invalid WebSocket upgrade request"),
        )
        .await;
    };
    let Some(route) = match_gateway_route(&request.path).filter(is_codex_responses_route) else {
        return write_handshake_failure(
            stream,
            &request,
            context,
            &connection_id,
            started_at,
            started,
            None,
            HandshakeFailure::local(
                426,
                "WebSocket transport is supported only on the Codex Responses route",
            ),
        )
        .await;
    };
    if !context.settings_snapshot().codex_websocket_enabled {
        return write_handshake_failure(
            stream,
            &request,
            context,
            &connection_id,
            started_at,
            started,
            None,
            HandshakeFailure::local(426, WEBSOCKET_DISABLED_REASON),
        )
        .await;
    }
    let Some(db) = context.db.as_ref() else {
        return write_handshake_failure(
            stream,
            &request,
            context,
            &connection_id,
            started_at,
            started,
            None,
            HandshakeFailure::local(503, "Gateway database is unavailable"),
        )
        .await;
    };
    let candidates = match context
        .load_candidate_providers(db, GatewayCliKey::Codex)
        .await
    {
        Ok(candidates) => candidates,
        Err(error) => {
            return write_handshake_failure(
                stream,
                &request,
                context,
                &connection_id,
                started_at,
                started,
                None,
                HandshakeFailure::local(502, error),
            )
            .await
        }
    };
    if candidates
        .selection
        .as_ref()
        .is_some_and(|selection| selection.mode == GatewayProxyMode::Aggregate)
    {
        return write_handshake_failure(
            stream,
            &request,
            context,
            &connection_id,
            started_at,
            started,
            None,
            HandshakeFailure::local(426, AGGREGATE_WEBSOCKET_REASON),
        )
        .await;
    }
    let apply_mapping = !candidates
        .selection
        .as_ref()
        .is_some_and(|selection| selection.mode == GatewayProxyMode::Single);
    let settings = context.settings_snapshot();
    let app_config = settings.effective_app_config(GatewayCliKey::Codex);
    let is_single_provider =
        is_single_provider_deployment(candidates.providers.len(), candidates.selection.as_ref());
    upstream::refresh_health_registry(context);
    let mut last_failure = HandshakeFailure::local(503, "No available Codex provider");
    let mut last_provider = None;
    let mut shutdown = context.websocket_shutdown.subscribe();
    let mut attempts = Vec::new();
    let mut skipped_by_health = 0_u32;
    let retryable_status_codes =
        crate::coding::proxy_gateway::retryable_status::retryable_status_code_set(
            &settings.retryable_status_codes,
        )
        .unwrap_or_else(|_| {
            crate::coding::proxy_gateway::retryable_status::default_retryable_status_codes()
                .into_iter()
                .collect()
        });
    'providers: for provider in candidates.providers {
        if attempts.len() as u64 > u64::from(app_config.max_retry_count) {
            break;
        }
        // response.create (and its model) arrives only after the handshake.
        let provider_available = context
            .health_registry
            .as_ref()
            .and_then(|registry| registry.lock().ok())
            .map(|registry| {
                registry.is_provider_available(
                    &ProviderHealthKey {
                        cli_key: GatewayCliKey::Codex,
                        provider_id: provider.id.clone(),
                    },
                    Utc::now(),
                )
            })
            .unwrap_or(true);
        if !is_single_provider && !provider_available {
            skipped_by_health = skipped_by_health.saturating_add(1);
            continue;
        }
        if let Some(reason) = fallback_reason(&provider) {
            let mut failure = HandshakeFailure::local(426, reason);
            failure.attempts = attempts;
            return write_handshake_failure(
                stream,
                &request,
                context,
                &connection_id,
                started_at,
                started,
                Some(&provider),
                failure,
            )
            .await;
        }
        let mut provider_attempt_count = 0;
        loop {
            if attempts.len() as u64 > u64::from(app_config.max_retry_count) {
                break 'providers;
            }
            if *shutdown.borrow() {
                return Ok(());
            }
            if !attempts.is_empty() {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(app_config.retry_interval_secs)) => {},
                    _ = shutdown.changed() => return Ok(()),
                }
            }
            provider_attempt_count += 1;
            let connected = tokio::select! {
                result = connect_upstream(&request, &provider, context, &route) => result,
                _ = shutdown.changed() => return Ok(()),
            };
            attempts.push(GatewayProviderAttempt {
                provider_id: Some(provider.id.clone()),
                provider_name: Some(provider.name.clone()),
                upstream_model_id: None,
                status_code: Some(
                    connected
                        .as_ref()
                        .map(|_| 101)
                        .unwrap_or_else(|error| error.status),
                ),
                success: connected.is_ok(),
                error_category: connected.as_ref().err().map(|error| {
                    if error.status == 426 {
                        "websocket_fallback"
                    } else {
                        "websocket_handshake_failed"
                    }
                    .to_string()
                }),
                error_message: connected.as_ref().err().map(|error| error.message.clone()),
                attempt_count: provider_attempt_count,
                total_attempt_count: attempts.len() as u32 + 1,
            });
            match connected {
                Ok((upstream_socket, upstream_headers, upstream_url)) => {
                    // A settings save can finish while the upstream handshake is in flight.
                    // The client must still receive 426 before any downstream upgrade.
                    if !context.settings_snapshot().codex_websocket_enabled {
                        let mut failure = HandshakeFailure::local(426, WEBSOCKET_DISABLED_REASON);
                        failure.upstream_status = Some(101);
                        failure.attempts = attempts;
                        return write_handshake_failure(
                            stream,
                            &request,
                            context,
                            &connection_id,
                            started_at,
                            started,
                            Some(&provider),
                            failure,
                        )
                        .await;
                    }
                    for (name, value) in &upstream_headers {
                        if !matches!(
                            name.as_str(),
                            "connection" | "upgrade" | "content-length" | "transfer-encoding"
                        ) && !name.as_str().starts_with("sec-websocket-")
                        {
                            downstream_response
                                .headers_mut()
                                .append(name, value.clone());
                        }
                    }
                    let mut wire = Vec::new();
                    server::write_response(&mut wire, &downstream_response).map_err(io_error)?;
                    timeout(WRITE_TIMEOUT, async {
                        stream.write_all(&wire).await?;
                        stream.flush().await
                    })
                    .await
                    .map_err(|_| io_error("Client WebSocket handshake write timed out"))??;
                    let downstream = WebSocketStream::from_partially_read(
                        stream,
                        initial_frames,
                        Role::Server,
                        Some(socket_config()),
                    )
                    .await;
                    let response_headers = downstream_response
                        .headers()
                        .iter()
                        .filter_map(|(name, value)| {
                            value
                                .to_str()
                                .ok()
                                .map(|value| (name.to_string(), value.to_string()))
                        })
                        .collect();
                    return relay(
                        downstream,
                        upstream_socket,
                        &request,
                        provider,
                        context,
                        connection_id,
                        upstream_url,
                        response_headers,
                        apply_mapping,
                        attempts,
                    )
                    .await;
                }
                Err(failure) => {
                    let retryable =
                        failure.status != 426 && retryable_status_codes.contains(&failure.status);
                    let retry_current = upstream::can_retry_current_provider(
                        upstream::classify_status_failure(failure.status)
                            .unwrap_or(GatewayFailureKind::Connection),
                        provider_attempt_count - 1,
                        app_config.per_provider_retry_count,
                        attempts.len().saturating_sub(1) as u32,
                        app_config.max_retry_count,
                    );
                    last_failure = failure;
                    last_provider = Some(provider.clone());
                    if !retryable {
                        break 'providers;
                    }
                    if !retry_current {
                        break;
                    }
                }
            }
        }
    }
    // 所有候选都被健康过滤跳过且没有发起任何握手时，用与 HTTP 503
    // `cooling_down` 相同的语义说明原因，而不是笼统的“没有可用渠道”。
    if attempts.is_empty() && skipped_by_health > 0 {
        last_failure = HandshakeFailure::local(
            503,
            "All provider candidates are cooling down; no upstream handshake was attempted.",
        );
    }
    last_failure.attempts = attempts;
    write_handshake_failure(
        stream,
        &request,
        context,
        &connection_id,
        started_at,
        started,
        last_provider.as_ref(),
        last_failure,
    )
    .await
}

async fn write_handshake_failure(
    stream: &mut TcpStream,
    request: &DebugHttpRequest,
    context: &GatewayRuntimeContext,
    connection_id: &str,
    started_at: DateTime<Utc>,
    started: Instant,
    provider: Option<&UpstreamProvider>,
    failure: HandshakeFailure,
) -> io::Result<()> {
    let fallback = failure.status == 426;
    let mut response = empty_response(
        failure.status,
        StatusCode::from_u16(failure.status)
            .ok()
            .and_then(|status| status.canonical_reason())
            .unwrap_or("Gateway error"),
        "openai-compatible",
        &failure.message,
    );
    response.cli_key = match_gateway_route(&request.path)
        .filter(is_codex_responses_route)
        .map(|_| GatewayCliKey::Codex);
    response
        .headers
        .push(("Content-Type".to_string(), "application/json".to_string()));
    response.body = serde_json::to_vec(&json!({"error":{"type":"gateway_error","code":if fallback {"websocket_fallback"} else {"websocket_handshake_failed"},"message":failure.message}})).unwrap_or_default();
    response.response_body_bytes = response.body.len() as u64;
    response.error_category = Some(
        if fallback {
            "websocket_fallback"
        } else {
            "websocket_handshake_failed"
        }
        .to_string(),
    );
    response.upstream_status_code = failure.upstream_status;
    response.provider_attempts = failure.attempts.clone();
    response.attempt_count = failure.attempts.len() as u32;
    response.provider_attempt_count = failure
        .attempts
        .last()
        .map_or(0, |attempt| attempt.attempt_count);
    response.failover = failure
        .attempts
        .iter()
        .filter_map(|attempt| attempt.provider_id.as_ref())
        .any(|id| Some(id.as_str()) != provider.map(|provider| provider.id.as_str()));
    if !failure.body.is_empty() {
        response.upstream_response_body_bytes = failure.body.len() as u64;
        response.upstream_response_body = Some(failure.body);
    }
    if let Some(retry_after) = failure
        .headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
    {
        response
            .headers
            .push(("Retry-After".to_string(), retry_after.to_string()));
    }
    if let Some(provider) = provider {
        response.provider_id = Some(provider.id.clone());
        response.provider_name = Some(provider.name.clone());
    }
    let result = write_response(stream, &mut response, started, &context.settings_snapshot()).await;
    record_gateway_observability_with_transport(
        request,
        &response,
        context,
        started_at,
        Utc::now(),
        GatewayRequestTransport::Websocket,
        GatewayRequestKind::WebsocketHandshake,
        Some(GatewayWebSocketMetadata {
            connection_id: connection_id.to_string(),
            handshake_status: failure.status,
            upstream_handshake_status: failure.upstream_status,
            fallback_reason: fallback.then_some(failure.message),
            handshake_attempts: failure.attempts,
            ..Default::default()
        }),
    );
    result
}

struct PendingTurn {
    request: DebugHttpRequest,
    response: DebugHttpResponse,
    metadata: GatewayWebSocketMetadata,
    kind: GatewayRequestKind,
    started_at: DateTime<Utc>,
    started: Instant,
    last_event: Instant,
    received_event: bool,
    failure_kind: Option<GatewayFailureKind>,
    restore_map: HashMap<String, super::compat::xai_responses::NamespacedName>,
    privacy_restorer: Option<crate::coding::proxy_gateway::privacy::stream::EventRestorer>,
    /// Set when this turn asked the upstream for a continuation summary, in
    /// which case the upstream's own stream is consumed and replaced by a
    /// synthesized `compaction` item.
    compaction: Option<CompactionStreamSummary>,
}

impl PendingTurn {
    fn record_upstream_event(&mut self, bytes: &[u8], settings: &ProxyGatewaySettings) {
        self.received_event = true;
        self.last_event = Instant::now();
        self.response.upstream_response_body_bytes = self
            .response
            .upstream_response_body_bytes
            .saturating_add(bytes.len() as u64);
        snapshot_push(
            self.response
                .upstream_response_body
                .get_or_insert_with(Vec::new),
            bytes,
            settings,
        );
    }

    fn record_delivered_event(&mut self, bytes: &[u8], settings: &ProxyGatewaySettings) {
        self.response
            .first_token_ms
            .get_or_insert(self.started.elapsed().as_millis() as u64);
        self.response.response_body_bytes = self
            .response
            .response_body_bytes
            .saturating_add(bytes.len() as u64);
        snapshot_push(&mut self.response.body, bytes, settings);
    }

    fn record_upstream_error(&mut self, value: &Value) {
        self.metadata.error_status = value
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok());
        self.failure_kind = self
            .metadata
            .error_status
            .and_then(upstream::classify_status_failure)
            .or_else(|| {
                match value
                    .pointer("/error/type")
                    .or_else(|| value.pointer("/response/error/type"))
                    .and_then(Value::as_str)
                {
                    Some("invalid_request_error") => Some(GatewayFailureKind::UpstreamBadRequest),
                    Some("authentication_error") => Some(GatewayFailureKind::Auth),
                    Some("rate_limit_error") => Some(GatewayFailureKind::RateLimit),
                    _ => None,
                }
            });
        self.response.error_category = Some("upstream_error".to_string());
        self.response.note = value
            .pointer("/error/message")
            .or_else(|| value.pointer("/response/error/message"))
            .and_then(Value::as_str)
            .unwrap_or("Upstream response failed")
            .to_string();
    }

    fn memory_bytes(&self) -> usize {
        self.request.body.len()
            + self
                .response
                .upstream_request_body
                .as_ref()
                .map_or(0, Vec::len)
    }

    fn finish(
        mut self,
        context: &GatewayRuntimeContext,
        outcome: GatewayStreamOutcome,
        category: Option<&str>,
        message: &str,
        terminal: bool,
    ) {
        self.response.stream_outcome = outcome;
        if let Some(category) = category {
            self.response.error_category = Some(category.to_string());
        }
        if !message.is_empty() {
            self.response.note = message.to_string();
        }
        if self.response.note.is_empty() && !outcome.is_success() {
            self.response.note = format!("Upstream response ended with {}", outcome.as_str());
        }
        if self.response.token_usage.envelope_id.is_none() {
            self.response.token_usage.envelope_id = self.metadata.response_id.clone();
        }
        if self.kind == GatewayRequestKind::Request
            && self.response.upstream_model_id.is_some()
            && !self
                .response
                .privacy
                .as_ref()
                .is_some_and(|privacy| privacy.detail().failed)
        {
            let key = ProviderModelHealthKey {
                cli_key: GatewayCliKey::Codex,
                provider_id: self.response.provider_id.clone().unwrap_or_default(),
                upstream_model_id: self.response.upstream_model_id.clone().unwrap_or_default(),
            };
            let changed = match outcome {
                GatewayStreamOutcome::Completed => upstream::record_health_success(context, &key),
                GatewayStreamOutcome::Failed if category != Some("request_schema") => {
                    upstream::record_health_failure(
                        context,
                        &key,
                        self.failure_kind.unwrap_or_else(|| match category {
                            Some(
                                "stream_idle_timeout"
                                | "stream_first_byte_timeout"
                                | "websocket_peer_timeout",
                            ) => GatewayFailureKind::Timeout,
                            Some("upstream_disconnected") => GatewayFailureKind::Connection,
                            _ => GatewayFailureKind::Upstream5xx,
                        }),
                    )
                }
                GatewayStreamOutcome::Incomplete if !terminal => upstream::record_health_failure(
                    context,
                    &key,
                    GatewayFailureKind::EmptyResponse,
                ),
                _ => false,
            };
            if changed {
                context.save_health_registry_async();
            }
        }
        self.response.provider_attempts = vec![GatewayProviderAttempt {
            provider_id: self.response.provider_id.clone(),
            provider_name: self.response.provider_name.clone(),
            upstream_model_id: self.response.upstream_model_id.clone(),
            status_code: None,
            success: outcome.is_success(),
            error_category: self.response.error_category.clone(),
            error_message: (!outcome.is_success()).then(|| self.response.note.clone()),
            attempt_count: 1,
            total_attempt_count: 1,
        }];
        record_gateway_observability_with_transport(
            &self.request,
            &self.response,
            context,
            self.started_at,
            Utc::now(),
            GatewayRequestTransport::Websocket,
            self.kind,
            Some(self.metadata),
        );
    }
}

#[derive(Default)]
struct PendingTurns {
    lanes: HashMap<String, VecDeque<PendingTurn>>,
    completed: VecDeque<String>,
    protected_completed: std::collections::HashSet<String>,
}

impl PendingTurns {
    fn push(&mut self, lane: String, turn: PendingTurn) {
        self.lanes.entry(lane).or_default().push_back(turn);
    }
    fn pop(&mut self, lane: &str) -> Option<PendingTurn> {
        let queue = self.lanes.get_mut(lane)?;
        let turn = queue.pop_front();
        // Queue wait contributes to duration, but not the next turn's first-event timeout.
        if let Some(next) = queue.front_mut() {
            next.last_event = Instant::now();
        }
        if queue.is_empty() {
            self.lanes.remove(lane);
        }
        if let Some(id) = turn
            .as_ref()
            .and_then(|turn| turn.metadata.response_id.clone())
        {
            if turn
                .as_ref()
                .is_some_and(|turn| turn.response.privacy.is_some())
            {
                self.protected_completed.insert(id.clone());
            }
            self.completed.push_back(id);
            if self.completed.len() > 256 {
                if let Some(expired) = self.completed.pop_front() {
                    self.protected_completed.remove(&expired);
                }
            }
        }
        turn
    }
    fn event_lane(&mut self, value: &Value) -> Option<String> {
        let response_id = value
            .pointer("/response/id")
            .or_else(|| value.get("response_id"))
            .and_then(Value::as_str);
        if response_id.is_some_and(|id| self.completed.iter().any(|completed| completed == id)) {
            return None;
        }
        let lane = value
            .get("stream_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                response_id.and_then(|id| {
                    self.lanes.iter().find_map(|(lane, queue)| {
                        (queue.front()?.metadata.response_id.as_deref() == Some(id))
                            .then(|| lane.clone())
                    })
                })
            })
            .unwrap_or_default();
        let turn = self.lanes.get_mut(&lane)?.front_mut()?;
        if let Some(response_id) = response_id {
            if turn
                .metadata
                .response_id
                .as_deref()
                .is_some_and(|current| current != response_id)
            {
                return None;
            }
            turn.metadata.response_id = Some(response_id.to_string());
        }
        Some(lane)
    }
    fn full(&self, additional_bytes: usize) -> bool {
        self.lanes.values().map(VecDeque::len).sum::<usize>() >= MAX_PENDING_REQUESTS
            || self
                .lanes
                .values()
                .flatten()
                .map(PendingTurn::memory_bytes)
                .sum::<usize>()
                .saturating_add(additional_bytes)
                > MAX_PENDING_BYTES
    }
    fn finish_all(
        &mut self,
        context: &GatewayRuntimeContext,
        outcome: GatewayStreamOutcome,
        category: &str,
        message: &str,
    ) {
        for (_, turns) in self.lanes.drain() {
            for turn in turns {
                turn.finish(context, outcome, Some(category), message, false);
            }
        }
    }
}

/// Seals a finished upstream summary into the `compaction` response Codex reads.
///
/// Returns `None` when there is no usable summary — the upstream failed, was
/// truncated, returned nothing, or the capsule could not be sealed. Codex
/// installs the item as the whole conversation history, so none of those may be
/// papered over with placeholder text: the caller forwards the upstream's own
/// terminal instead, which Codex classifies and retries on its own terms.
fn build_websocket_compaction_response(
    compaction: &CompactionStreamSummary,
    model: &str,
) -> Option<Value> {
    let summary = compaction.finish().ok()?;
    let item = codex_remote_compaction::build_compaction_item(&summary, model)?;
    Some(codex_remote_compaction::build_websocket_compaction_response(
        &item,
        model,
        compaction.usage(),
    ))
}

async fn send_message<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: Message,
) -> io::Result<()> {
    timeout(WRITE_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket write timed out"))?
        .map_err(io_error)
}

async fn flush_socket<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) -> io::Result<()> {
    timeout(WRITE_TIMEOUT, socket.flush())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket flush timed out"))?
        .map_err(io_error)
}

async fn close_socket<S: AsyncRead + AsyncWrite + Unpin>(socket: &mut WebSocketStream<S>) {
    let _ = timeout(CLOSE_TIMEOUT, async {
        // SinkExt also flushes the reply queued by an incoming Close. The
        // inherent close(None) sends a new message, which fails in that state.
        if SinkExt::close(socket).await.is_err() {
            return;
        }
        // Sending Close does not finish the handshake. Drain frames already in
        // flight until the peer acknowledges it, without forwarding new work.
        // Dropping TCP earlier can turn our Close into a connection reset.
        while let Some(Ok(_)) = socket.next().await {}
    })
    .await;
}

fn io_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn snapshot_push(snapshot: &mut Vec<u8>, bytes: &[u8], settings: &ProxyGatewaySettings) {
    if !settings.store_response_body {
        snapshot.clear();
        return;
    }
    let limit =
        (settings.log_max_body_size_kb.saturating_mul(1024) as usize).min(MAX_MESSAGE_BYTES);
    snapshot.truncate(limit);
    if snapshot.len() < limit {
        if !snapshot.is_empty() {
            snapshot.push(b'\n');
        }
        snapshot.extend_from_slice(&bytes[..bytes.len().min(limit.saturating_sub(snapshot.len()))]);
    }
}

fn terminal_outcome(kind: SseTerminalKind) -> GatewayStreamOutcome {
    match kind {
        SseTerminalKind::Success => GatewayStreamOutcome::Completed,
        SseTerminalKind::Failed => GatewayStreamOutcome::Failed,
        SseTerminalKind::Incomplete => GatewayStreamOutcome::Incomplete,
        SseTerminalKind::Canceled => GatewayStreamOutcome::Canceled,
    }
}

/// The retryable code Codex maps to a retry and reconnects on. Reusing the code
/// its own connection-limit recovery uses keeps a model switch to a reconnect
/// instead of a collapsed turn.
const MODEL_LOCK_RETRY_CODE: &str = "websocket_connection_limit_reached";

/// The message a New API-family relay writes for a Responses WebSocket that is
/// locked to another model. It is the only marker: the `code` is the generic
/// `invalid_request` shared with several unrelated rejections, and a real
/// connection-limit error says something else.
const MODEL_LOCK_MESSAGE: &str = "responses websocket connection is locked to model";

fn is_upstream_model_lock_error(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("error")
        && value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains(MODEL_LOCK_MESSAGE))
}

/// Rewrite the lock rejection so Codex classifies it as retryable, keeping the
/// upstream message (it names the model the connection is stuck on).
fn rewrite_upstream_model_lock_error(text: &str, value: &Value) -> Vec<u8> {
    let mut rewritten = value.clone();
    if let Some(error) = rewritten.get_mut("error").and_then(Value::as_object_mut) {
        error.insert("code".into(), Value::String(MODEL_LOCK_RETRY_CODE.into()));
    }
    serde_json::to_vec(&rewritten).unwrap_or_else(|_| text.as_bytes().to_vec())
}

async fn relay<S: AsyncRead + AsyncWrite + Unpin, U: AsyncRead + AsyncWrite + Unpin>(
    mut downstream: WebSocketStream<S>,
    mut upstream_socket: WebSocketStream<U>,
    handshake: &DebugHttpRequest,
    provider: UpstreamProvider,
    context: &GatewayRuntimeContext,
    connection_id: String,
    upstream_url: String,
    response_headers: Vec<(String, String)>,
    apply_mapping: bool,
    handshake_attempts: Vec<GatewayProviderAttempt>,
) -> io::Result<()> {
    let mut pending = PendingTurns::default();
    let mut shutdown = context.websocket_shutdown.subscribe();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.tick().await;
    let mut deadline_check = tokio::time::interval(Duration::from_secs(1));
    let connected_at = Instant::now();
    let mut last_data = connected_at;
    let mut last_upstream_frame = connected_at;
    let mut last_downstream_frame = connected_at;
    let (outcome, category, note) = loop {
        // Keep in-flight generations and their usage intact. Once the connection
        // is idle, close it so the next handshake observes the HTTP fallback gate.
        if pending.lanes.is_empty() && !context.settings_snapshot().codex_websocket_enabled {
            break (
                GatewayStreamOutcome::Canceled,
                "websocket_disabled",
                WEBSOCKET_DISABLED_REASON,
            );
        }
        if *shutdown.borrow() {
            break (
                GatewayStreamOutcome::Canceled,
                "gateway_stopped",
                "Gateway stopped or restarted",
            );
        }
        tokio::select! {
            _ = shutdown.changed() => break (GatewayStreamOutcome::Canceled, "gateway_stopped", "Gateway stopped or restarted"),
            _ = deadline_check.tick() => {
                let settings = context.settings_snapshot();
                let app = settings.effective_app_config(GatewayCliKey::Codex);
                let timed_out = pending
                    .lanes
                    .values()
                    .filter_map(|queue| queue.front())
                    .find(|turn| {
                        let seconds = if turn.received_event {
                            app.streaming_idle_timeout_secs
                        } else {
                            app.streaming_first_byte_timeout_secs
                        };
                        turn.last_event.elapsed() >= Duration::from_secs(seconds.max(1))
                    });
                if let Some(turn) = timed_out {
                    break (
                        GatewayStreamOutcome::Failed,
                        if turn.received_event {
                            "stream_idle_timeout"
                        } else {
                            "stream_first_byte_timeout"
                        },
                        "Responses WebSocket request timed out",
                    );
                }
                if last_upstream_frame.elapsed() >= Duration::from_secs(90) {
                    break (
                        GatewayStreamOutcome::Failed,
                        "websocket_peer_timeout",
                        "Upstream WebSocket stopped responding",
                    );
                }
                if last_downstream_frame.elapsed() >= Duration::from_secs(90) {
                    break (
                        GatewayStreamOutcome::Canceled,
                        "client_disconnected",
                        "Client WebSocket stopped responding",
                    );
                }
                if last_data.elapsed() >= CONNECTION_IDLE_TIMEOUT
                    || connected_at.elapsed() >= CONNECTION_MAX_LIFETIME
                {
                    break (
                        GatewayStreamOutcome::Canceled,
                        "websocket_connection_expired",
                        "WebSocket connection expired; reconnect to continue",
                    );
                }
            }
            _ = heartbeat.tick() => {
                if send_message(&mut downstream, Message::Ping(Vec::new().into()))
                    .await
                    .is_err()
                {
                    break (
                        GatewayStreamOutcome::Canceled,
                        "client_disconnected",
                        "Client disconnected",
                    );
                }
                if send_message(&mut upstream_socket, Message::Ping(Vec::new().into()))
                    .await
                    .is_err()
                {
                    break (
                        GatewayStreamOutcome::Failed,
                        "upstream_disconnected",
                        "Upstream disconnected",
                    );
                }
            }
            message = downstream.next() => {
                // The setting may change while an idle connection awaits a frame.
                if pending.lanes.is_empty() && !context.settings_snapshot().codex_websocket_enabled {
                    break (GatewayStreamOutcome::Canceled, "websocket_disabled", WEBSOCKET_DISABLED_REASON);
                }
                last_downstream_frame = Instant::now();
                match message {
                    Some(Ok(Message::Text(text))) => {
                        last_data = Instant::now();
                        let value = serde_json::from_str::<Value>(&text).ok();
                        let event_type = value
                            .as_ref()
                            .and_then(|value| value.get("type"))
                            .and_then(Value::as_str);
                        if event_type != Some("response.create") {
                            if context.privacy.enabled() && !value.as_ref().is_some_and(|value| {
                                event_type == Some("response.cancel") && value.as_object().is_some_and(|object| object.keys().all(|key| matches!(key.as_str(), "type" | "response_id" | "stream_id" | "event_id")))
                            }) {
                                let payload = json!({"type":"error","status":400,"error":{"type":"invalid_request_error","code":"privacy_unsupported_event","message":"Privacy protection supports response.create and response.cancel events"}});
                                if send_message(&mut downstream, Message::Text(payload.to_string().into())).await.is_err() {
                                    break (GatewayStreamOutcome::Canceled, "client_disconnected", "Client disconnected");
                                }
                                continue;
                            }
                            // Preserve forward-compatible control events; they are not new model requests.
                            if send_message(&mut upstream_socket, Message::Text(text))
                                .await
                                .is_err()
                            {
                                break (
                                    GatewayStreamOutcome::Failed,
                                    "upstream_disconnected",
                                    "Could not send WebSocket control event",
                                );
                            }
                            continue;
                        }
                        let value = value.expect("response.create was parsed");
                        let started_at = Utc::now();
                        let started = Instant::now();
                        let warmup = value.get("generate").and_then(Value::as_bool) == Some(false);
                        if !warmup {
                            context.record_request_arrival(GatewayCliKey::Codex);
                        }
                        let request = DebugHttpRequest {
                            id: NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst),
                            method: "WS".to_string(),
                            path: handshake.path.clone(),
                            headers: handshake.headers.clone(),
                            body: text.as_bytes().to_vec(),
                        };
                        let connection_scope = format!("{}:{}", connection_id, value.get("stream_id").and_then(Value::as_str).unwrap_or_default());
                        let privacy = context.privacy.begin("codex", &request.headers, &request.body, Some(&connection_scope));
                        let prepared = privacy.as_ref().map_err(Clone::clone).and_then(|privacy| {
                            upstream::prepare_websocket_request(&request, &provider, context, apply_mapping)
                                .and_then(|(body, model, map)| {
                                    let body = if let Some(privacy) = privacy {
                                        privacy.prepare(&body, &provider.id).inspect_err(|_| privacy.fail())?
                                    } else { body };
                                    Ok((body, model, map))
                                })
                        });
                        let mut response = empty_response(0, "", "openai-compatible", "");
                        response.cli_key = Some(GatewayCliKey::Codex);
                        response.provider_id = Some(provider.id.clone());
                        response.provider_name = Some(provider.name.clone());
                        response.provider_type = provider.meta.provider_type.clone();
                        response.cost_multiplier = Some(provider.meta.cost_multiplier.clone());
                        response.pricing_model_source = Some(provider.meta.pricing_model_source.clone());
                        response.requested_model = value
                            .get("model")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        response.target_protocol = Some(AiProtocol::OpenAiResponses);
                        response.source_protocol = Some(AiProtocol::OpenAiResponses);
                        response.upstream_url = Some(upstream_url.clone());
                        response.headers = response_headers.clone();
                        response.is_streaming = true;
                        response.attempt_count = 1;
                        response.provider_attempt_count = 1;
                        response.privacy = privacy.ok().flatten();
                        if let Some(privacy) = &response.privacy { privacy.set_usage_provider(response.provider_type.clone()); }
                        let privacy_restorer = response.privacy.clone().map(crate::coding::proxy_gateway::privacy::stream::EventRestorer::new);
                        let stream_id = value
                            .get("stream_id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let lane = stream_id.clone().unwrap_or_default();
                        let mut turn = PendingTurn {
                            request,
                            response,
                            metadata: GatewayWebSocketMetadata {
                                connection_id: connection_id.clone(),
                                stream_id,
                                previous_response_id: value
                                    .get("previous_response_id")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                event_type: Some("response.create".to_string()),
                                handshake_status: 101,
                                upstream_handshake_status: Some(101),
                                handshake_attempts: handshake_attempts.clone(),
                                ..Default::default()
                            },
                            kind: if warmup {
                                GatewayRequestKind::WebsocketWarmup
                            } else {
                                GatewayRequestKind::Request
                            },
                            started_at,
                            started,
                            last_event: started,
                            received_event: false,
                            failure_kind: None,
                            restore_map: HashMap::new(),
                            privacy_restorer,
                            compaction: RemoteCompactionCompat::resolve_websocket(&value)
                                .is_remote_compaction()
                                .then(CompactionStreamSummary::default),
                        };
                        let prepared = prepared.and_then(|(body, model, map)| {
                            if pending.full(turn.request.body.len().saturating_add(body.len())) {
                                return Err("Too many pending WebSocket requests".to_string());
                            }
                            Ok((body, model, map))
                        });
                        match prepared {
                            Ok((body, model, map)) => {
                                turn.response.upstream_model_id = Some(model);
                                turn.response.upstream_request_body = Some(body.clone());
                                turn.restore_map = map;
                                if send_message(
                                    &mut upstream_socket,
                                    Message::Text(String::from_utf8(body).map_err(io_error)?.into()),
                                )
                                .await
                                .is_err()
                                {
                                    turn.finish(
                                        context,
                                        GatewayStreamOutcome::Failed,
                                        Some("upstream_disconnected"),
                                        "Could not send response.create",
                                        false,
                                    );
                                    break (
                                        GatewayStreamOutcome::Failed,
                                        "upstream_disconnected",
                                        "Upstream disconnected",
                                    );
                                }
                                pending.push(lane, turn);
                            }
                            Err(error) => {
                                turn.response.upstream_url = None;
                                let mut payload = json!({"type":"error","status":400,"error":{"type":"invalid_request_error","code":"invalid_request","message":error}});
                                if let Some(stream_id) = &turn.metadata.stream_id {
                                    payload["stream_id"] = json!(stream_id);
                                }
                                let bytes = serde_json::to_vec(&payload).map_err(io_error)?;
                                let sent =
                                    send_message(&mut downstream, Message::Text(payload.to_string().into()))
                                        .await;
                                if sent.is_ok() {
                                    turn.record_delivered_event(&bytes, &context.settings_snapshot());
                                    turn.metadata.error_status = Some(400);
                                    turn.finish(
                                        context,
                                        GatewayStreamOutcome::Failed,
                                        Some("request_schema"),
                                        &error,
                                        true,
                                    );
                                } else {
                                    turn.finish(
                                        context,
                                        GatewayStreamOutcome::Canceled,
                                        Some("client_disconnected"),
                                        "Could not deliver request validation error to client",
                                        false,
                                    );
                                    break (
                                        GatewayStreamOutcome::Canceled,
                                        "client_disconnected",
                                        "Client disconnected",
                                    );
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        if flush_socket(&mut downstream).await.is_err() {
                            break (
                                GatewayStreamOutcome::Canceled,
                                "client_disconnected",
                                "Client disconnected",
                            );
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        let _ = send_message(&mut upstream_socket, Message::Close(frame)).await;
                        break (
                            GatewayStreamOutcome::Canceled,
                            "client_disconnected",
                            "Client closed the WebSocket",
                        );
                    }
                    Some(Ok(other)) => {
                        if context.privacy.enabled() {
                            break (GatewayStreamOutcome::Canceled, "privacy_unsupported_event", "Binary data is unsupported while privacy protection is enabled");
                        }
                        if send_message(&mut upstream_socket, other).await.is_err() {
                            break (
                                GatewayStreamOutcome::Failed,
                                "upstream_disconnected",
                                "Upstream disconnected",
                            );
                        }
                    }
                    Some(Err(_)) | None => {
                        break (
                            GatewayStreamOutcome::Canceled,
                            "client_disconnected",
                            "Client disconnected before the response finished",
                        )
                    }
                }
            }
            message = upstream_socket.next() => {
                last_upstream_frame = Instant::now();
                match message {
                    Some(Ok(Message::Text(text))) => {
                        last_data = Instant::now();
                        let value = serde_json::from_str::<Value>(&text).ok();
                        let lane = value.as_ref().and_then(|value| pending.event_lane(value));
                        // The relay pins a Responses WebSocket to the model of its
                        // first successful turn and rejects a later model switch
                        // with a 400. Codex reuses one socket for a whole session,
                        // so the rejection must become a retry plus a fresh
                        // connection rather than a dead turn; see the rewrite below.
                        let model_locked = value
                            .as_ref()
                            .is_some_and(is_upstream_model_lock_error);
                        // Late duplicate terminals must not consume or leak into another protected turn.
                        if value.as_ref().and_then(|value| value.pointer("/response/id").or_else(|| value.get("response_id"))).and_then(Value::as_str)
                            .is_some_and(|id| pending.completed.iter().any(|completed| completed == id)
                                && (context.privacy.enabled() || pending.protected_completed.contains(id))) {
                            continue;
                        }
                        // Determine scope before popping a default-lane terminal.
                        let connection_error = lane.is_none()
                            && value.as_ref().is_some_and(|value| {
                                value.get("type").and_then(Value::as_str) == Some("error")
                                    && value.get("stream_id").is_none()
                                    && value.get("response_id").is_none()
                            });
                        let mut terminal = classify_sse_event_fields(None, &text);
                        let settings = context.settings_snapshot();
                        let mut outgoing = match &value {
                            Some(value) if model_locked => {
                                rewrite_upstream_model_lock_error(&text, value)
                            }
                            _ => text.as_bytes().to_vec(),
                        };
                        let mut restored_messages = None;
                        if let Some(turn) = lane
                            .as_ref()
                            .and_then(|lane| pending.lanes.get_mut(lane))
                            .and_then(|queue| queue.front_mut())
                        {
                            turn.record_upstream_event(text.as_bytes(), &settings);
                            turn.response
                                .token_usage
                                .merge_max(from_response_body(GatewayCliKey::Codex, text.as_bytes()));
                            if terminal != Some(SseTerminalKind::Failed) {
                                outgoing =
                                    upstream::restore_xai_namespace_json_body(&outgoing, &turn.restore_map);
                            }
                            if terminal == Some(SseTerminalKind::Failed) {
                                if let Some(value) = &value {
                                    turn.record_upstream_error(value);
                                    if model_locked {
                                        // The refusal is about the connection, not
                                        // the model: keep it off the health score and
                                        // name it for what it is in the log.
                                        turn.failure_kind =
                                            Some(GatewayFailureKind::UpstreamModelLocked);
                                        turn.response.error_category =
                                            Some("websocket_model_locked".to_string());
                                    }
                                }
                            }
                            if let Some(restorer) = &mut turn.privacy_restorer {
                                restored_messages = Some(match restorer.push_json(&outgoing) {
                                    Ok(messages) => messages,
                                    Err(error) => {
                                        turn.failure_kind = Some(GatewayFailureKind::RequestSchema);
                                        turn.response.error_category = Some("privacy_restore_failed".into());
                                        turn.response.note = error.clone();
                                        vec![json!({"type":"response.failed","stream_id":turn.metadata.stream_id,"response":{"id":turn.metadata.response_id,"status":"failed","error":{"code":"privacy_restore_failed","message":error}}}).to_string().into_bytes()]
                                    }
                                });
                            }
                            if let (Some(compaction), Some(value)) = (&mut turn.compaction, &value) {
                                compaction.push(value);
                            }
                        }
                        if connection_error {
                            for turn in pending.lanes.values_mut().flatten() {
                                turn.record_upstream_event(text.as_bytes(), &settings);
                                if let Some(value) = &value {
                                    turn.record_upstream_error(value);
                                }
                            }
                        }
                        let privacy_unscoped = lane.is_none() && (context.privacy.enabled()
                            || pending.lanes.values().flatten().any(|turn| turn.privacy_restorer.is_some()));
                        if privacy_unscoped {
                            outgoing = json!({"type":"error","status":502,"error":{"code":"privacy_unscoped_event","message":"Could not associate an upstream event with its privacy mapping"}}).to_string().into_bytes();
                            for turn in pending.lanes.values_mut().flatten() {
                                if let Some(privacy) = &turn.response.privacy { privacy.fail(); }
                            }
                        }
                        // A compaction turn's upstream frames are consumed rather
                        // than forwarded. Two reasons: Codex needs exactly one
                        // `compaction` item, which cannot be built until the
                        // summary is complete; and streaming the raw deltas would
                        // put the summary on the wire in the clear, defeating the
                        // envelope this path exists to produce.
                        //
                        // Two cases fall back to the upstream's own frames:
                        // a summary that is not finished yet (nothing to emit),
                        // and one that failed, was truncated, or came back empty.
                        // The latter must not be papered over — Codex installs the
                        // item as the whole conversation history, and it classifies
                        // `response.failed` by error code to retry the retryable
                        // ones.
                        enum CompactionOutcome {
                            Consumed,
                            Synthesized(Vec<String>),
                            Forwarded,
                        }
                        let compaction_outcome = match lane
                            .as_ref()
                            .and_then(|lane| pending.lanes.get(lane))
                            .and_then(|queue| queue.front())
                            .and_then(|turn| turn.compaction.as_ref())
                        {
                            None => CompactionOutcome::Forwarded,
                            Some(compaction) if !compaction.is_terminal() => {
                                CompactionOutcome::Consumed
                            }
                            Some(_) => lane
                                .as_ref()
                                .and_then(|lane| pending.lanes.get_mut(lane))
                                .and_then(|queue| queue.front_mut())
                                .and_then(|turn| {
                                    let compaction = turn.compaction.take()?;
                                    let model = turn
                                        .response
                                        .upstream_model_id
                                        .clone()
                                        .unwrap_or_default();
                                    build_websocket_compaction_response(&compaction, &model)
                                })
                                .map(|response| {
                                    CompactionOutcome::Synthesized(
                                        codex_remote_compaction::build_compaction_frames(&response),
                                    )
                                })
                                .unwrap_or(CompactionOutcome::Forwarded),
                        };
                        if matches!(compaction_outcome, CompactionOutcome::Synthesized(_)) {
                            // The upstream's own terminal is replaced, so the turn
                            // is closed by the synthesized `response.completed`.
                            terminal = Some(SseTerminalKind::Success);
                        }
                        let mut sent = Ok(());
                        let mut delivered_terminal = None;
                        let privacy_transformed = restored_messages.is_some();
                        let passthrough = restored_messages.is_none().then(|| outgoing.clone());
                        let forwarded = match compaction_outcome {
                            CompactionOutcome::Synthesized(frames) => frames
                                .into_iter()
                                .map(|frame| frame.into_bytes())
                                .collect::<Vec<_>>(),
                            CompactionOutcome::Consumed => Vec::new(),
                            CompactionOutcome::Forwarded => {
                                restored_messages.into_iter().flatten().chain(passthrough).collect()
                            }
                        };
                        for message in forwarded {
                            sent = send_message(&mut downstream, Message::Text(String::from_utf8(message.clone()).map_err(io_error)?.into())).await;
                            if sent.is_err() { break; }
                            delivered_terminal = if privacy_transformed {
                                classify_sse_event_fields(None, std::str::from_utf8(&message).unwrap_or_default()).or(delivered_terminal)
                            } else { terminal };
                            if let Some(lane) = &lane {
                                if let Some(turn) = pending
                                    .lanes
                                    .get_mut(lane)
                                    .and_then(|queue| queue.front_mut())
                                {
                                    turn.record_delivered_event(&message, &settings);
                                }
                            }
                        }
                        if sent.is_ok() {
                            if let (Some(lane), Some(terminal)) = (lane, delivered_terminal) {
                                if let Some(turn) = pending.pop(&lane) { turn.finish(context, terminal_outcome(terminal), None, "", true); }
                            }
                        }
                        if sent.is_err() {
                            break (
                                GatewayStreamOutcome::Canceled,
                                "client_disconnected",
                                "Could not deliver WebSocket response to client",
                            );
                        }
                        if model_locked {
                            // Codex reuses one socket for a whole session, so the
                            // retry it just queued would land on the same locked
                            // connection. Close it and let the retry dial again.
                            break (
                                GatewayStreamOutcome::Failed,
                                "websocket_model_locked",
                                "Upstream WebSocket is locked to another model; reconnecting",
                            );
                        }
                        if privacy_unscoped {
                            break (GatewayStreamOutcome::Failed, "privacy_unscoped_event", "Could not associate an upstream event with its privacy mapping");
                        }
                        if connection_error {
                            // An unscoped error while named lanes are pending is a connection error.
                            for turn in pending.lanes.values_mut().flatten() {
                                turn.record_delivered_event(&outgoing, &settings);
                            }
                            break (GatewayStreamOutcome::Failed, "upstream_error", "");
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        if flush_socket(&mut upstream_socket).await.is_err() {
                            break (
                                GatewayStreamOutcome::Failed,
                                "upstream_disconnected",
                                "Upstream disconnected",
                            );
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        let _ = send_message(&mut downstream, Message::Close(frame)).await;
                        break (
                            GatewayStreamOutcome::Incomplete,
                            "stream_incomplete",
                            "Upstream closed before the response terminal event",
                        );
                    }
                    Some(Ok(other)) => {
                        if context.privacy.enabled() || pending.lanes.values().flatten().any(|turn| turn.privacy_restorer.is_some()) {
                            for turn in pending.lanes.values_mut().flatten() {
                                if let Some(privacy) = &turn.response.privacy { privacy.fail(); }
                            }
                            break (GatewayStreamOutcome::Failed, "privacy_unsupported_event", "Cannot restore binary upstream data");
                        }
                        if send_message(&mut downstream, other).await.is_err() {
                            break (
                                GatewayStreamOutcome::Canceled,
                                "client_disconnected",
                                "Client disconnected",
                            );
                        }
                    }
                    Some(Err(_)) | None => {
                        break (
                            GatewayStreamOutcome::Incomplete,
                            "stream_incomplete",
                            "Upstream disconnected before the response terminal event",
                        )
                    }
                }
            }
        }
    };
    pending.finish_all(context, outcome, category, note);
    tokio::join!(
        close_socket(&mut downstream),
        close_socket(&mut upstream_socket)
    );
    Ok(())
}

#[cfg(test)]
mod tests;
