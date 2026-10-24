use super::content_encoding::{
    decompress_body, get_content_encoding_from_pairs, is_supported_content_encoding,
};
use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::coding::proxy_gateway::types::{
    GatewayCliKey, GatewayProviderAttempt, GatewayStreamOutcome, ProxyGatewaySettings,
};
use crate::coding::proxy_gateway::usage_parser::{SseTerminalKind, SseUsageCollector, TokenUsage};
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use std::io::Write;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time;

pub(super) const MAX_REQUEST_HEADER_BYTES: usize = 64 * 1024;
pub(super) const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024;

pub(super) type DebugBodyStream =
    Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>;

/// Distinguish a client disconnect (the client is gone, so writing more would
/// be wasted) from other write errors. Once the terminal event has already been
/// delivered, a subsequent disconnect still counts as a successful stream.
fn is_client_disconnect_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::WriteZero
    )
}

/// Errors that mean the upstream body's own framing/decode layer gave up
/// mid-stream even though the bytes already yielded (and forwarded to the
/// client) are valid. These are demoted to a clean stream EOF (issue #318):
/// the terminal-event detector, not the decoder's failure, decides Completed
/// vs Incomplete from what was actually delivered.
///
/// Text sources, lowercase-matched because both upstream paths wrap the raw
/// library error with "Failed to read upstream response body: ":
/// - reqwest 0.12 maps every hyper body-frame error to `Kind::Decode`, whose
///   display is "error decoding response body"; its `bytes_stream()` yields
///   that text for any mid-body failure on the reqwest path (system/custom
///   proxy modes);
/// - the header-preserving path formats the raw `hyper::Error` display, so the
///   same class of failure surfaces as hyper's own strings: "error reading a
///   body from connection" (`Kind::Body`, includes strict chunked-decoder
///   rejections) and "connection closed before message completed"
///   (`Kind::IncompleteMessage`, upstream dropped the connection before the
///   chunked terminator).
/// Other errors (idle timeout, closed hyper channel) keep their hard-failure
/// handling so real transport problems stay visible.
fn is_demotable_stream_body_error(error: &str) -> bool {
    let lowered = error.to_ascii_lowercase();
    lowered.contains("error decoding response body")
        || lowered.contains("error reading a body from connection")
        || lowered.contains("connection closed before message completed")
}

/// Classify how a streaming response ended for the client, mirroring axonhub's
/// `writeSSEStreamEnd` three-way priority:
/// - `Failed` terminal delivered (`error` / `response.failed` /
///   `response.completed`+`status=failed`) -> `Failed`: the stream ended
///   properly but carried a protocol failure;
/// - success terminal delivered -> `Completed` (success even if the client
///   disconnects immediately afterwards);
/// - `Incomplete` / `Canceled` terminal delivered -> the matching outcome, so
///   the gateway's own cross-protocol `response.completed`+`status=incomplete`
///   is not collapsed into `Completed`;
/// - explicit stream error (idle timeout / upstream stream error) -> `Failed`;
/// - client disconnected before the terminal event -> `Canceled`;
/// - stream ended at EOF without a terminal event and without an error ->
///   `Incomplete` (a failed stream the client read as a truncated success).
fn classify_stream_outcome(
    write_result: &std::io::Result<()>,
    terminal_kind_delivered: Option<SseTerminalKind>,
    idle_timeout: bool,
    upstream_stream_error: bool,
) -> GatewayStreamOutcome {
    match terminal_kind_delivered {
        Some(SseTerminalKind::Failed) => GatewayStreamOutcome::Failed,
        Some(SseTerminalKind::Success) => GatewayStreamOutcome::Completed,
        Some(SseTerminalKind::Incomplete) => GatewayStreamOutcome::Incomplete,
        Some(SseTerminalKind::Canceled) => GatewayStreamOutcome::Canceled,
        None => {
            let client_disconnected = write_result
                .as_ref()
                .err()
                .is_some_and(is_client_disconnect_error);
            if idle_timeout || upstream_stream_error {
                GatewayStreamOutcome::Failed
            } else if client_disconnected {
                GatewayStreamOutcome::Canceled
            } else {
                GatewayStreamOutcome::Incomplete
            }
        }
    }
}

/// Render a protocol-dialect SSE error event so the client receives a clear
/// failure instead of a silently truncated chunked stream. The shape follows
/// the client's own protocol: Anthropic `event: error`, Responses
/// `response.failed`, Gemini's `{code,message,status}` envelope, and a generic
/// chat-compat error object for OpenAI Chat / unknown routes. Only meaningful
/// for SSE responses.
fn render_stream_error_event(
    source_protocol: Option<AiProtocol>,
    code: &str,
    message: &str,
) -> Vec<u8> {
    let body = match source_protocol {
        Some(AiProtocol::AnthropicMessages) => {
            let payload = serde_json::json!({
                "type": "error",
                "error": {
                    "type": "api_error",
                    "message": message,
                },
            });
            format!(
                "event: error\ndata: {}\n\n",
                serde_json::to_string(&payload).unwrap_or_default()
            )
        }
        Some(AiProtocol::OpenAiResponses) => {
            let payload = serde_json::json!({
                "type": "response.failed",
                "response": {
                    "id": "resp_gateway_stream_error",
                    "object": "response",
                    "status": "failed",
                    "error": {
                        "code": code,
                        "message": message,
                    },
                },
            });
            format!(
                "event: response.failed\ndata: {}\n\n",
                serde_json::to_string(&payload).unwrap_or_default()
            )
        }
        Some(AiProtocol::GeminiNative) => {
            // Gemini's native error envelope is `{"error":{"code":<numeric>,
            // "message":..,"status":..}}` (see `transformer::gemini::convert`).
            // Map the internal category string to an HTTP-like numeric code so
            // `gemini_stream_error` produces a valid Gemini status; otherwise it
            // falls back to 500 / INTERNAL.
            let gemini_code = match code {
                "stream_idle_timeout" => "408",
                _ => "500",
            };
            let payload = crate::coding::proxy_gateway::transformer::gemini_stream_error(
                gemini_code,
                message,
            );
            format!(
                "data: {}\n\n",
                serde_json::to_string(&payload).unwrap_or_default()
            )
        }
        // OpenAI Chat and unknown routes: a generic chat-compatible error object.
        _ => {
            let payload = serde_json::json!({
                "error": {
                    "message": message,
                    "type": "server_error",
                    "code": code,
                },
            });
            format!(
                "data: {}\n\n",
                serde_json::to_string(&payload).unwrap_or_default()
            )
        }
    };
    body.into_bytes()
}

/// Whether the response is being delivered as an SSE stream, so an error event
/// rendered in the client's protocol dialect is meaningful.
fn response_is_sse(headers: &[(String, String)]) -> bool {
    header_value(headers, "content-type")
        .map(|value| value.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false)
}

#[derive(Clone)]
pub(super) struct SharedBodySnapshot {
    inner: Arc<Mutex<BodySnapshot>>,
}

struct BodySnapshot {
    body: Vec<u8>,
    total_bytes: u64,
    max_bytes: usize,
}

impl SharedBodySnapshot {
    pub(super) fn new(max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BodySnapshot {
                body: Vec::new(),
                total_bytes: 0,
                max_bytes,
            })),
        }
    }

    pub(super) fn push(&self, chunk: &[u8]) {
        let Ok(mut snapshot) = self.inner.lock() else {
            return;
        };
        snapshot.total_bytes = snapshot.total_bytes.saturating_add(chunk.len() as u64);
        if snapshot.max_bytes == 0 || snapshot.body.len() >= snapshot.max_bytes {
            return;
        }
        let remaining = snapshot.max_bytes.saturating_sub(snapshot.body.len());
        snapshot
            .body
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    pub(super) fn read(&self) -> (Vec<u8>, u64) {
        let Ok(snapshot) = self.inner.lock() else {
            return (Vec::new(), 0);
        };
        (snapshot.body.clone(), snapshot.total_bytes)
    }
}

#[derive(Debug)]
pub(super) struct DebugHttpRequest {
    pub(super) id: u64,
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

pub(super) struct DebugHttpResponse {
    pub(super) privacy: Option<crate::coding::proxy_gateway::privacy::PrivacyRequest>,
    pub(super) status_code: u16,
    pub(super) status_text: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) body_stream: Option<DebugBodyStream>,
    pub(super) response_body_bytes: u64,
    pub(super) token_usage: TokenUsage,
    pub(super) first_token_ms: Option<u64>,
    pub(super) is_streaming: bool,
    pub(super) cli_key: Option<GatewayCliKey>,
    pub(super) route_name: String,
    pub(super) provider_id: Option<String>,
    pub(super) provider_name: Option<String>,
    pub(super) provider_type: Option<String>,
    pub(super) cost_multiplier: Option<String>,
    pub(super) pricing_model_source: Option<String>,
    /// Provider-level extra multipliers for the Fast / Ultrafast service tiers,
    /// captured from the effective provider at request time. Observability picks
    /// one based on the tier actually billed.
    pub(super) fast_cost_multiplier: Option<String>,
    pub(super) ultrafast_cost_multiplier: Option<String>,
    pub(super) requested_model: Option<String>,
    pub(super) upstream_model_id: Option<String>,
    /// Canonical service tier (`fast` / `ultrafast`) the upstream confirmed in
    /// its own response envelope. `None` when the upstream never confirmed a
    /// premium tier, so billing falls back to the requested tier.
    pub(super) served_service_tier: Option<String>,
    /// Which side wrote the `compaction` item on a Codex remote compaction v2
    /// turn: `native` when the upstream answered with its own item, `fallback`
    /// when the trigger was offered and refused before the Gateway summarized,
    /// `simulated` when the target protocol could never have served it natively.
    /// `None` on every other turn, and on a compaction turn that failed before
    /// either side wrote anything.
    pub(super) compaction_mode: Option<String>,
    pub(super) upstream_request_body: Option<Vec<u8>>,
    /// Protocol of the final upstream attempt, independent of the client protocol.
    pub(super) target_protocol: Option<AiProtocol>,
    pub(super) upstream_response_body: Option<Vec<u8>>,
    pub(super) upstream_response_body_bytes: u64,
    pub(super) upstream_response_body_stream_snapshot: Option<SharedBodySnapshot>,
    /// Original HTTP status code the upstream returned for this response, before
    /// the gateway rewrites a failure into its own 502 envelope. Populated when the
    /// gateway substitutes a synthetic status (e.g. a 200 SSE stream that carried an
    /// error envelope becomes 502) so request detail can still surface the real code.
    pub(super) upstream_status_code: Option<u16>,
    pub(super) upstream_url: Option<String>,
    pub(super) error_category: Option<String>,
    pub(super) attempt_count: u32,
    pub(super) provider_attempt_count: u32,
    pub(super) provider_attempts: Vec<GatewayProviderAttempt>,
    pub(super) failover: bool,
    pub(super) note: String,
    /// Client-facing protocol of the request that produced this response. Used to
    /// render a protocol-dialect error event when the stream ends abnormally.
    pub(super) source_protocol: Option<AiProtocol>,
    /// How the streaming response actually ended for the client. Set by
    /// `write_streaming_body` from the terminal-event verdict; `NotStreaming`
    /// for non-streaming responses. Drives `success` in observability instead of
    /// the (already-sent) HTTP status code.
    pub(super) stream_outcome: GatewayStreamOutcome,
    /// Native-tool and rewrite markers for the detail dialog. Persisted only in
    /// the JSONL detail, never in the SQLite summary.
    pub(super) observations: super::observations::RequestObservations,
}

impl DebugHttpResponse {
    pub(super) fn upstream_response_body_snapshot(&self) -> Option<(Vec<u8>, u64)> {
        if let Some(body) = &self.upstream_response_body {
            return Some((body.clone(), self.upstream_response_body_bytes));
        }
        self.upstream_response_body_stream_snapshot
            .as_ref()
            .map(SharedBodySnapshot::read)
    }
}

pub(super) async fn read_http_request(
    stream: &mut TcpStream,
    request_id: u64,
) -> std::io::Result<DebugHttpRequest> {
    let mut raw = Vec::new();
    let mut header_end = None;
    let mut buffer = [0_u8; 8192];

    while header_end.is_none() {
        let read = time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timed out reading gateway request headers",
                )
            })??;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        if raw.len() > MAX_REQUEST_HEADER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Gateway request headers exceed the maximum allowed size",
            ));
        }
        header_end = find_header_end(&raw);
    }

    let header_end = header_end.unwrap_or(raw.len());
    let mut header_text = String::from_utf8_lossy(&raw[..header_end]).to_string();
    while header_text.ends_with('\n') || header_text.ends_with('\r') {
        header_text.pop();
    }

    let mut lines = header_text.lines();
    let first_line = lines.next().unwrap_or_default().trim().to_string();
    let mut first_parts = first_line.split_whitespace();
    let method = first_parts.next().unwrap_or_default().to_string();
    let path = first_parts.next().unwrap_or_default().to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();

    let content_length = header_value(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Gateway request body exceeds the maximum allowed size",
        ));
    }
    let body_start = header_end.min(raw.len());
    let mut body = raw[body_start..].to_vec();
    if body.len() > MAX_REQUEST_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Gateway request body exceeds the maximum allowed size",
        ));
    }
    while body.len() < content_length {
        let read = time::timeout(Duration::from_secs(30), stream.read(&mut buffer))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timed out reading gateway request body",
                )
            })??;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        body.extend_from_slice(&buffer[..read]);
        if body.len() > MAX_REQUEST_BODY_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Gateway request body exceeds the maximum allowed size",
            ));
        }
    }

    Ok(DebugHttpRequest {
        id: request_id,
        method,
        path,
        headers,
        body,
    })
}

/// Decode inbound compressed request bodies before JSON routing/conversion.
///
/// Codex Desktop official-login clients commonly send `Content-Encoding: zstd`.
/// After a successful decode, entity headers that no longer match the body are
/// stripped so later forwarding rebuilds length/encoding from the plain JSON.
pub(super) fn decode_inbound_request_body(request: &mut DebugHttpRequest) -> Result<(), String> {
    let Some(encoding) = get_content_encoding_from_pairs(&request.headers) else {
        return Ok(());
    };

    if !is_supported_content_encoding(&encoding) {
        return Err(format!("Unsupported request content-encoding: {encoding}"));
    }

    let decompressed = match decompress_body(&encoding, &request.body, MAX_REQUEST_BODY_BYTES) {
        Ok(Some(decompressed)) => decompressed,
        Ok(None) => {
            return Err(format!("Unsupported request content-encoding: {encoding}"));
        }
        Err(error) => {
            return Err(format!(
                "Failed to decompress request body ({encoding}): {error}"
            ));
        }
    };

    request.body = decompressed;
    request.headers.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("content-encoding")
            && !name.eq_ignore_ascii_case("content-length")
            && !name.eq_ignore_ascii_case("transfer-encoding")
    });
    Ok(())
}

pub(super) fn json_response(
    status_code: u16,
    status_text: &str,
    value: Value,
    route_name: &str,
    upstream_url: Option<String>,
    note: &str,
) -> DebugHttpResponse {
    let body = serde_json::to_vec(&value)
        .unwrap_or_else(|_| br#"{"error":"response_serialize_failed"}"#.to_vec());
    DebugHttpResponse {
        privacy: None,
        status_code,
        status_text: status_text.to_string(),
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        response_body_bytes: body.len() as u64,
        body,
        body_stream: None,
        token_usage: TokenUsage::default(),
        first_token_ms: None,
        is_streaming: false,
        cli_key: None,
        route_name: route_name.to_string(),
        provider_id: None,
        provider_name: None,
        provider_type: None,
        cost_multiplier: None,
        pricing_model_source: None,
        requested_model: None,
        upstream_model_id: None,
        served_service_tier: None,
        compaction_mode: None,
        fast_cost_multiplier: None,
        ultrafast_cost_multiplier: None,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
        upstream_response_body_stream_snapshot: None,
        upstream_status_code: None,
        upstream_url,
        error_category: None,
        attempt_count: 0,
        provider_attempt_count: 0,
        provider_attempts: Vec::new(),
        failover: false,
        note: note.to_string(),
        source_protocol: None,
        target_protocol: None,
        stream_outcome: GatewayStreamOutcome::NotStreaming,
        observations: Default::default(),
    }
}

pub(super) fn empty_response(
    status_code: u16,
    status_text: &str,
    route_name: &str,
    note: &str,
) -> DebugHttpResponse {
    DebugHttpResponse {
        privacy: None,
        status_code,
        status_text: status_text.to_string(),
        headers: Vec::new(),
        response_body_bytes: 0,
        body: Vec::new(),
        body_stream: None,
        token_usage: TokenUsage::default(),
        first_token_ms: None,
        is_streaming: false,
        cli_key: None,
        route_name: route_name.to_string(),
        provider_id: None,
        provider_name: None,
        provider_type: None,
        cost_multiplier: None,
        pricing_model_source: None,
        requested_model: None,
        upstream_model_id: None,
        served_service_tier: None,
        compaction_mode: None,
        fast_cost_multiplier: None,
        ultrafast_cost_multiplier: None,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
        upstream_response_body_stream_snapshot: None,
        upstream_status_code: None,
        upstream_url: None,
        error_category: None,
        attempt_count: 0,
        provider_attempt_count: 0,
        provider_attempts: Vec::new(),
        failover: false,
        note: note.to_string(),
        source_protocol: None,
        target_protocol: None,
        stream_outcome: GatewayStreamOutcome::NotStreaming,
        observations: Default::default(),
    }
}

pub(super) async fn write_response(
    stream: &mut TcpStream,
    response: &mut DebugHttpResponse,
    started_instant: Instant,
    settings: &ProxyGatewaySettings,
) -> std::io::Result<()> {
    let mut header = Vec::new();
    write!(
        &mut header,
        "HTTP/1.1 {} {}\r\n",
        response.status_code, response.status_text
    )?;
    let mut has_content_length = false;
    let mut has_connection = false;
    let streaming = response.body_stream.is_some();
    for (name, value) in &response.headers {
        if streaming
            && (name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("transfer-encoding"))
        {
            continue;
        }
        if name.eq_ignore_ascii_case("content-length") {
            has_content_length = true;
        }
        if name.eq_ignore_ascii_case("connection") {
            has_connection = true;
        }
        write!(&mut header, "{}: {}\r\n", name, value)?;
    }
    if streaming {
        write!(&mut header, "Transfer-Encoding: chunked\r\n")?;
    } else if !has_content_length {
        write!(&mut header, "Content-Length: {}\r\n", response.body.len())?;
    }
    if !has_connection {
        write!(&mut header, "Connection: close\r\n")?;
    }
    write!(&mut header, "\r\n")?;
    stream.write_all(&header).await?;
    if streaming {
        write_streaming_body(stream, response, started_instant, settings).await?;
    } else {
        stream.write_all(&response.body).await?;
    }
    stream.flush().await
}

async fn write_streaming_body(
    stream: &mut TcpStream,
    response: &mut DebugHttpResponse,
    started_instant: Instant,
    settings: &ProxyGatewaySettings,
) -> std::io::Result<()> {
    let mut body_stream = match response.body_stream.take() {
        Some(body_stream) => body_stream,
        None => return Ok(()),
    };
    // The collector drives both token usage (needs cli_key at merge time) and
    // terminal-event tracking (independent of cli_key). Create it unconditionally
    // so a stream without a cli identity still gets a correct verdict and
    // client-facing error event instead of being misclassified as incomplete.
    let mut usage_collector =
        SseUsageCollector::with_provider_type(response.provider_type.as_deref());
    // Hosted-tool markers are scanned on the bytes actually forwarded, so the
    // detail dialog can say whether the upstream really ran a native tool. This
    // is independent of body storage, which is off by default.
    //
    // The scan is a byte search, so it has to use the dialect those bytes are
    // in: the stream reaching this writer has already been converted to the
    // *client's* protocol when a conversion route applied. Scanning it with the
    // upstream dialect found nothing on every cross-protocol turn, which made
    // the flag flip with `stream` alone (the non-streaming path reads the
    // pre-conversion body in `target_protocol`).
    let mut used_tool_scanner = super::observations::UsedToolScanner::new(
        response
            .source_protocol
            .or(response.target_protocol)
            .unwrap_or(AiProtocol::OpenAiResponses),
    );
    response.response_body_bytes = 0;
    response.body.clear();
    let idle_timeout_secs = response
        .cli_key
        .map(|cli_key| {
            settings
                .effective_app_config(cli_key)
                .streaming_idle_timeout_secs
        })
        .unwrap_or(settings.streaming_idle_timeout_secs)
        .max(1);
    let idle_timeout = Duration::from_secs(idle_timeout_secs);
    let is_sse = response_is_sse(&response.headers);

    let mut write_result: std::io::Result<()> = Ok(());
    let mut client_write_failed = false;
    let mut terminal_kind_delivered: Option<SseTerminalKind> = None;
    let mut idle_timeout_hit = false;
    let mut upstream_stream_error = false;
    // Set when the upstream body's framing/decode layer gave up mid-stream (see
    // `is_demotable_stream_body_error`) even though the bytes already forwarded
    // to the client are valid. Suppresses the synthetic error-event injection
    // below so the already-forwarded bytes are not corrupted by a trailing error
    // envelope the client cannot parse.
    let mut demoted_body_decode_error = false;
    loop {
        let next_chunk = match time::timeout(idle_timeout, body_stream.next()).await {
            Ok(next_chunk) => next_chunk,
            Err(_) => {
                response.error_category = Some("stream_idle_timeout".to_string());
                response.note = format!(
                    "upstream streaming response was idle for {} seconds",
                    idle_timeout.as_secs()
                );
                idle_timeout_hit = true;
                write_result = Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    response.note.clone(),
                ));
                break;
            }
        };
        let Some(chunk_result) = next_chunk else {
            break;
        };
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(error) => {
                // reqwest 0.12 surfaces every hyper body-frame error as "error
                // decoding response body", and the header-preserving path surfaces
                // the raw hyper display; both mean the body framing layer rejected
                // the stream mid-flight while the bytes already forwarded stay
                // valid. Treating that as a hard failure injects a synthetic error
                // event after the real data, breaking the client's SSE decoder
                // (issue #318, where relay chunked framing was non-standard yet the
                // payload had already been yielded). Demote it to a clean stream
                // EOF instead: the terminal detector decides Completed vs
                // Incomplete from what was actually delivered.
                if is_demotable_stream_body_error(&error) {
                    log::warn!("demoting upstream body decode error to clean stream EOF: {error}");
                    demoted_body_decode_error = true;
                    // Keep the raw reason in the summary note so these rows stay
                    // distinguishable from a plain empty stream in request logs.
                    response.note =
                        format!("upstream body decode error demoted to clean stream EOF: {error}");
                    break;
                }
                upstream_stream_error = true;
                response.error_category = Some(
                    if error.starts_with("privacy_") {
                        "privacy_restore_failed"
                    } else {
                        "stream_error"
                    }
                    .to_string(),
                );
                response.note = format!("upstream streaming response error: {error}");
                write_result = Err(std::io::Error::new(std::io::ErrorKind::Other, error));
                break;
            }
        };
        if chunk.is_empty() {
            continue;
        }
        if response.first_token_ms.is_none() {
            response.first_token_ms = Some(
                started_instant
                    .elapsed()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            );
        }
        response.response_body_bytes = response
            .response_body_bytes
            .saturating_add(chunk.len() as u64);
        used_tool_scanner.push_chunk(&chunk);
        append_body_snapshot(response, &chunk, settings);
        let chunk_header = format!("{:X}\r\n", chunk.len());
        let chunk_write_result: std::io::Result<()> = async {
            stream.write_all(chunk_header.as_bytes()).await?;
            stream.write_all(&chunk).await?;
            stream.write_all(b"\r\n").await?;
            stream.flush().await
        }
        .await;
        if chunk_write_result.is_err() {
            usage_collector.drain_terminal();
            terminal_kind_delivered = usage_collector.terminal_kind();
        }
        match response.cli_key {
            Some(cli_key) => usage_collector.push_chunk(cli_key, &chunk),
            None => usage_collector.observe_chunk(&chunk),
        }
        if let Err(error) = chunk_write_result {
            client_write_failed = true;
            write_result = Err(error);
            break;
        }
        terminal_kind_delivered = usage_collector.terminal_kind();
    }

    if !client_write_failed && terminal_kind_delivered.is_none() {
        usage_collector.drain_terminal();
        terminal_kind_delivered = usage_collector.terminal_kind();
    }

    // A `Failed` terminal event (upstream `error` / `response.failed` /
    // `response.completed`+`status=failed` / non-null error envelope) ends the
    // stream properly but is a failure; without this the summary row would
    // carry a failed outcome with no category or message.
    if matches!(terminal_kind_delivered, Some(SseTerminalKind::Failed))
        && response.error_category.is_none()
    {
        response.error_category = Some("upstream_stream_error".to_string());
        response.note = "upstream stream delivered a non-success terminal event".to_string();
    }

    // A real client disconnect before the terminal event -> Canceled. Record the
    // disconnect reason explicitly so it is not lost behind the benign forwarding
    // note ("streaming forwarded to provider id=... name=..."); otherwise the
    // request log would show success=false with a note that reads like a success.
    // A disconnect *after* the terminal event is left untouched: the stream
    // already succeeded, and `runtime::handle_connection` must not attach a
    // failure category to it.
    let client_disconnected_pre_terminal = write_result
        .as_ref()
        .err()
        .is_some_and(is_client_disconnect_error)
        && terminal_kind_delivered.is_none();
    if client_disconnected_pre_terminal {
        if response.error_category.is_none() {
            response.error_category = Some("client_disconnected".to_string());
        }
        if response.note.trim().is_empty() {
            let err = write_result.as_ref().err().unwrap();
            response.note = format!("client disconnected before stream terminal event: {err}");
        }
    }

    let outcome = classify_stream_outcome(
        &write_result,
        terminal_kind_delivered,
        idle_timeout_hit,
        upstream_stream_error,
    );

    // When the stream did not end normally and the client is still reachable,
    // render a protocol-dialect error event so the client sees an explicit
    // failure instead of a silently truncated chunked stream. Skip when a
    // terminal event was already delivered (the client saw the upstream's own
    // terminal envelope) and when the client has disconnected (nobody left to
    // receive it).
    let client_disconnected = write_result
        .as_ref()
        .err()
        .is_some_and(is_client_disconnect_error);
    if terminal_kind_delivered.is_none()
        && !client_disconnected
        && is_sse
        && !demoted_body_decode_error
    {
        let (code, message) = match outcome {
            GatewayStreamOutcome::Failed => (
                response.error_category.as_deref().unwrap_or("stream_error"),
                response.note.clone(),
            ),
            _ => {
                response.error_category = Some("stream_incomplete".to_string());
                response.note = "stream ended without a terminal event".to_string();
                ("stream_incomplete", response.note.clone())
            }
        };
        let error_event = render_stream_error_event(response.source_protocol, code, &message);
        let error_header = format!("{:X}\r\n", error_event.len());
        // Best-effort: a failure here just means the client is gone, which the
        // final terminator write below will also detect.
        let _ = stream.write_all(error_header.as_bytes()).await;
        let _ = stream.write_all(&error_event).await;
        let _ = stream.write_all(b"\r\n").await;
        let _ = stream.flush().await;
    }

    // Close the chunked stream so the client reads a clean EOF instead of a
    // bare truncation. Skip when the client has already disconnected.
    if !client_disconnected && write_result.is_ok() {
        write_result = stream.write_all(b"0\r\n\r\n").await;
    } else if !client_disconnected {
        // An upstream-side error (idle timeout / stream error) still leaves the
        // client connected; send the terminator after the error event above.
        let _ = stream.write_all(b"0\r\n\r\n").await;
    }

    if let Some(cli_key) = response.cli_key {
        // The collector is consumed by `finish`, so read the confirmed tier first.
        // Keep a pre-read tier (a buffered reply re-rendered as SSE, e.g. a
        // remote-compaction summary, whose upstream body was already parsed
        // for the tier) when the stream itself carried no tier events.
        let streamed_tier = usage_collector.service_tier().map(ToString::to_string);
        if streamed_tier.is_some() || response.served_service_tier.is_none() {
            response.served_service_tier = streamed_tier;
        }
        response.token_usage = usage_collector.finish(cli_key);
    }
    response.stream_outcome = outcome;
    response
        .observations
        .observe_used_markers(used_tool_scanner.kinds());
    write_result
}

fn append_body_snapshot(
    response: &mut DebugHttpResponse,
    chunk: &[u8],
    settings: &ProxyGatewaySettings,
) {
    if !settings.store_response_body {
        return;
    }
    let max_bytes = settings.log_max_body_size_kb.saturating_mul(1024) as usize;
    if max_bytes == 0 || response.body.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes.saturating_sub(response.body.len());
    response
        .body
        .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
}

pub(super) fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            raw.windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })
}

pub(super) fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_inbound_request_body_decompresses_zstd_and_strips_entity_headers() {
        let payload = br#"{"model":"gpt-5","input":[{"role":"user","content":"hi"}]}"#;
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(&payload[..]), 0).unwrap();
        let mut request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/responses".to_string(),
            headers: vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("content-encoding".to_string(), "zstd".to_string()),
                ("content-length".to_string(), compressed.len().to_string()),
            ],
            body: compressed,
        };

        decode_inbound_request_body(&mut request).unwrap();

        assert_eq!(request.body, payload);
        assert!(header_value(&request.headers, "content-encoding").is_none());
        assert!(header_value(&request.headers, "content-length").is_none());
        assert_eq!(
            header_value(&request.headers, "content-type"),
            Some("application/json")
        );
    }

    #[test]
    fn decode_inbound_request_body_rejects_unsupported_encoding() {
        let mut request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/responses".to_string(),
            headers: vec![("content-encoding".to_string(), "snappy".to_string())],
            body: b"\x00\x01\x02".to_vec(),
        };

        let error = decode_inbound_request_body(&mut request).unwrap_err();
        assert!(error.contains("Unsupported request content-encoding: snappy"));
        assert_eq!(request.body, b"\x00\x01\x02");
    }

    #[test]
    fn decode_inbound_request_body_rejects_corrupt_zstd() {
        let mut request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/responses".to_string(),
            headers: vec![("content-encoding".to_string(), "zstd".to_string())],
            body: b"not-valid-zstd".to_vec(),
        };

        let error = decode_inbound_request_body(&mut request).unwrap_err();
        assert!(error.contains("Failed to decompress request body (zstd)"));
    }

    fn io_error(kind: std::io::ErrorKind, message: &str) -> std::io::Error {
        std::io::Error::new(kind, message.to_string())
    }

    #[test]
    fn classify_stream_outcome_completed_wins_over_later_disconnect() {
        // Terminal event already delivered -> Completed even if the client then
        // resets the connection (common: client closes right after message_stop).
        let outcome = classify_stream_outcome(
            &Err(io_error(std::io::ErrorKind::BrokenPipe, "broken pipe")),
            Some(SseTerminalKind::Success),
            false,
            false,
        );
        assert_eq!(outcome, GatewayStreamOutcome::Completed);
    }

    #[test]
    fn classify_stream_outcome_error_terminal_event_is_failed() {
        // An `error` / `response.failed` event ends the stream properly but the
        // request is a failure, not a success — even though delivery succeeded.
        let outcome = classify_stream_outcome(&Ok(()), Some(SseTerminalKind::Failed), false, false);
        assert_eq!(outcome, GatewayStreamOutcome::Failed);
    }

    #[test]
    fn classify_stream_outcome_incomplete_terminal_is_incomplete() {
        // `response.completed` + `status=incomplete` (the gateway's own
        // cross-protocol signal) is a terminal-but-incomplete outcome, not a
        // success and not a hard failure.
        let outcome =
            classify_stream_outcome(&Ok(()), Some(SseTerminalKind::Incomplete), false, false);
        assert_eq!(outcome, GatewayStreamOutcome::Incomplete);
    }

    #[test]
    fn classify_stream_outcome_canceled_terminal_is_canceled() {
        // `response.cancelled` delivered to the client is a canceled outcome.
        let outcome =
            classify_stream_outcome(&Ok(()), Some(SseTerminalKind::Canceled), false, false);
        assert_eq!(outcome, GatewayStreamOutcome::Canceled);
    }

    #[test]
    fn classify_stream_outcome_idle_timeout_is_failed() {
        let outcome = classify_stream_outcome(
            &Err(io_error(std::io::ErrorKind::TimedOut, "idle")),
            None,
            true,
            false,
        );
        assert_eq!(outcome, GatewayStreamOutcome::Failed);
    }

    #[test]
    fn classify_stream_outcome_disconnect_before_terminal_is_canceled() {
        let outcome = classify_stream_outcome(
            &Err(io_error(std::io::ErrorKind::ConnectionReset, "reset")),
            None,
            false,
            false,
        );
        assert_eq!(outcome, GatewayStreamOutcome::Canceled);
    }

    #[test]
    fn classify_stream_outcome_eof_without_terminal_is_incomplete() {
        let outcome = classify_stream_outcome(&Ok(()), None, false, false);
        assert_eq!(outcome, GatewayStreamOutcome::Incomplete);
    }

    #[test]
    fn classify_stream_outcome_non_disconnect_write_error_is_incomplete() {
        // A write error that is not a client disconnect (e.g. local disk/socket
        // issue) without a terminal event is still an incomplete stream.
        let outcome = classify_stream_outcome(
            &Err(io_error(std::io::ErrorKind::Other, "weird")),
            None,
            false,
            false,
        );
        assert_eq!(outcome, GatewayStreamOutcome::Incomplete);
    }

    #[test]
    fn render_stream_error_event_matches_client_protocol_dialect() {
        let anthropic = render_stream_error_event(Some(AiProtocol::AnthropicMessages), "x", "boom");
        let text = String::from_utf8_lossy(&anthropic);
        assert!(text.starts_with("event: error\n"));
        assert!(text.contains("\"type\":\"api_error\""));

        let responses = render_stream_error_event(Some(AiProtocol::OpenAiResponses), "x", "boom");
        let text = String::from_utf8_lossy(&responses);
        assert!(text.starts_with("event: response.failed\n"));
        assert!(text.contains("\"status\":\"failed\""));

        let generic = render_stream_error_event(None, "x", "boom");
        let text = String::from_utf8_lossy(&generic);
        assert!(text.starts_with("data: "));
        assert!(text.contains("\"error\""));

        let chat = render_stream_error_event(Some(AiProtocol::OpenAiChat), "x", "boom");
        assert_eq!(chat, generic);

        let gemini = render_stream_error_event(
            Some(AiProtocol::GeminiNative),
            "stream_idle_timeout",
            "boom",
        );
        let text = String::from_utf8_lossy(&gemini);
        assert!(text.starts_with("data: "));
        // Gemini error envelope: numeric code + message + status, not the
        // generic `type`/string-code chat shape.
        assert!(text.contains("\"code\":408"));
        assert!(text.contains("\"message\":\"boom\""));
        assert!(text.contains("\"status\""));
        assert!(!text.contains("\"type\":\"server_error\""));
    }

    #[test]
    fn response_is_sse_requires_event_stream_content_type() {
        assert!(response_is_sse(&[(
            "Content-Type".to_string(),
            "text/event-stream".to_string(),
        )]));
        assert!(response_is_sse(&[(
            "content-type".to_string(),
            "text/event-stream; charset=utf-8".to_string(),
        )]));
        assert!(!response_is_sse(&[(
            "Content-Type".to_string(),
            "application/json".to_string(),
        )]));
        assert!(!response_is_sse(&[]));
    }

    #[test]
    fn demotable_stream_body_error_covers_both_upstream_http_paths() {
        // reqwest 0.12 wraps every hyper body-frame error as Kind::Decode.
        assert!(is_demotable_stream_body_error(
            "Failed to read upstream response body: error decoding response body"
        ));
        // Header-preserving path surfaces the raw hyper display.
        assert!(is_demotable_stream_body_error(
            "Failed to read upstream response body: error reading a body from connection"
        ));
        assert!(is_demotable_stream_body_error(
            "Failed to read upstream response body: connection closed before message completed"
        ));
        // Matching is case-insensitive and prefix-tolerant.
        assert!(is_demotable_stream_body_error(
            "Failed to read upstream response body: Error Decoding Response Body"
        ));
        // Non-decode transport errors keep hard-failure handling.
        assert!(!is_demotable_stream_body_error(
            "Timed out waiting for upstream stream chunk after 60 seconds"
        ));
        assert!(!is_demotable_stream_body_error(
            "Failed to read upstream response body: channel closed"
        ));
        assert!(!is_demotable_stream_body_error("upstream exploded"));
    }

    fn test_streaming_response(chunks: Vec<Result<Vec<u8>, String>>) -> DebugHttpResponse {
        DebugHttpResponse {
            privacy: None,
            status_code: 200,
            status_text: "OK".to_string(),
            headers: vec![("Content-Type".to_string(), "text/event-stream".to_string())],
            body: Vec::new(),
            body_stream: Some(Box::pin(futures_util::stream::iter(chunks))),
            response_body_bytes: 0,
            token_usage: TokenUsage::default(),
            first_token_ms: None,
            is_streaming: true,
            cli_key: Some(GatewayCliKey::Codex),
            route_name: "openai-compatible".to_string(),
            provider_id: Some("provider-1".to_string()),
            provider_name: Some("Provider One".to_string()),
            provider_type: None,
            cost_multiplier: None,
            pricing_model_source: None,
            requested_model: None,
            upstream_model_id: None,
            served_service_tier: None,
            compaction_mode: None,
            fast_cost_multiplier: None,
            ultrafast_cost_multiplier: None,
            upstream_request_body: None,
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
            upstream_response_body_stream_snapshot: None,
            upstream_status_code: None,
            upstream_url: None,
            error_category: None,
            attempt_count: 1,
            provider_attempt_count: 1,
            provider_attempts: Vec::new(),
            failover: false,
            source_protocol: Some(AiProtocol::OpenAiResponses),
            target_protocol: Some(AiProtocol::OpenAiResponses),
            stream_outcome: GatewayStreamOutcome::NotStreaming,
            observations: Default::default(),
            note: "streaming forwarded to provider id=provider-1 name=Provider One".to_string(),
        }
    }

    /// Loopback client/server pair: `write_streaming_body` writes to the client
    /// half while the spawned task collects everything the server half receives,
    /// so tests can assert the exact bytes that reached the client.
    async fn connect_write_pair() -> (TcpStream, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let reader = tokio::spawn(async move {
            let mut server = server;
            let mut collected = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                match server.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => collected.extend_from_slice(&buffer[..read]),
                }
            }
            collected
        });
        (client, reader)
    }

    async fn run_stream_test(
        chunks: Vec<Result<Vec<u8>, String>>,
    ) -> (DebugHttpResponse, std::io::Result<()>, String) {
        run_stream_test_with_settings(chunks, ProxyGatewaySettings::default()).await
    }

    async fn run_stream_test_with_settings(
        chunks: Vec<Result<Vec<u8>, String>>,
        settings: ProxyGatewaySettings,
    ) -> (DebugHttpResponse, std::io::Result<()>, String) {
        let (mut client, reader) = connect_write_pair().await;
        let mut response = test_streaming_response(chunks);
        let result =
            write_streaming_body(&mut client, &mut response, Instant::now(), &settings).await;
        drop(client);
        let received = String::from_utf8_lossy(&reader.await.unwrap()).to_string();
        (response, result, received)
    }

    /// The scanner must read the forwarded bytes in *their* dialect. On a
    /// cross-protocol turn those bytes are already the client's protocol
    /// (`source_protocol`), not the upstream's (`target_protocol`); scanning
    /// with the latter found no marker and left `used` empty.
    #[tokio::test]
    async fn streamed_tool_markers_are_scanned_in_the_forwarded_dialect() {
        let body = "event: content_block_delta data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"web_search_tool_result\"}} event: message_stop data: {\"type\":\"message_stop\"}";
        let (mut client, reader) = connect_write_pair().await;
        let mut response = test_streaming_response(vec![Ok(body.as_bytes().to_vec())]);
        // Upstream spoke Responses; the forwarded stream is Anthropic.
        response.target_protocol = Some(AiProtocol::OpenAiResponses);
        response.source_protocol = Some(AiProtocol::AnthropicMessages);

        let result = write_streaming_body(
            &mut client,
            &mut response,
            Instant::now(),
            &ProxyGatewaySettings::default(),
        )
        .await;
        drop(client);
        let _ = reader.await.unwrap();

        assert!(result.is_ok());
        assert!(
            response
                .observations
                .native_tools
                .iter()
                .any(|tool| tool.used
                    && tool.kind
                        == crate::coding::proxy_gateway::runtime::observations::TOOL_WEB_SEARCH),
            "the Anthropic marker in the forwarded stream should have been seen"
        );
    }

    #[tokio::test]
    async fn large_flattened_terminal_reaches_client_independently_of_body_logging() {
        let padding = "x".repeat(540 * 1024);
        let body = format!(
            "event: response.reasoning_summary_part.done data: {{\"type\":\"response.reasoning_summary_part.done\",\"status\":\"incomplete\"}} event: response.completed data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_large\",\"status\":\"completed\",\"note\":\"{padding}\",\"usage\":{{\"input_tokens\":120,\"output_tokens\":40}}}}}}"
        );
        for store_body in [false, true] {
            for chunk_size in [1024, 65536, 300000] {
                let chunks: Vec<_> = body
                    .as_bytes()
                    .chunks(chunk_size)
                    .map(|chunk| Ok(chunk.to_vec()))
                    .collect();
                let mut expected_wire = String::new();
                for chunk in body.as_bytes().chunks(chunk_size) {
                    expected_wire.push_str(&format!(
                        "{:X}\r\n{}\r\n",
                        chunk.len(),
                        std::str::from_utf8(chunk).unwrap()
                    ));
                }
                expected_wire.push_str("0\r\n\r\n");
                let settings = ProxyGatewaySettings {
                    store_response_body: store_body,
                    log_max_body_size_kb: 1,
                    ..ProxyGatewaySettings::default()
                };
                let (response, result, received) =
                    run_stream_test_with_settings(chunks, settings).await;
                assert!(result.is_ok());
                assert_eq!(
                    response.stream_outcome,
                    GatewayStreamOutcome::Completed,
                    "store_body={store_body}, chunk_size={chunk_size}"
                );
                assert_eq!(response.token_usage.input_tokens, Some(120));
                assert_eq!(response.token_usage.output_tokens, Some(40));
                assert_eq!(received, expected_wire);
                assert!(response.body.len() <= 1024);
            }
        }
    }

    #[tokio::test]
    async fn terminal_in_failed_write_is_not_counted_as_delivered() {
        for delimiter in ["", "\n\n"] {
            let (mut client, reader) = connect_write_pair().await;
            client.shutdown().await.unwrap();
            let terminal = format!(
                "event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\",\"usage\":{{\"input_tokens\":100,\"output_tokens\":50}}}}}}{delimiter}"
            );
            let mut response = test_streaming_response(vec![Ok(terminal.into_bytes())]);
            let result = write_streaming_body(
                &mut client,
                &mut response,
                Instant::now(),
                &ProxyGatewaySettings::default(),
            )
            .await;
            drop(client);
            assert!(result.is_err());
            assert_ne!(response.stream_outcome, GatewayStreamOutcome::Completed);
            assert_eq!(response.token_usage.total_tokens(), Some(150));
            assert!(reader.await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn charged_200_stream_keeps_real_non_success_terminal_outcomes() {
        for (status, expected) in [
            ("failed", GatewayStreamOutcome::Failed),
            ("incomplete", GatewayStreamOutcome::Incomplete),
            ("canceled", GatewayStreamOutcome::Canceled),
        ] {
            let terminal = format!(
                "event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"{status}\",\"usage\":{{\"input_tokens\":100,\"output_tokens\":50}}}}}}\n\n"
            );
            let (response, result, received) =
                run_stream_test(vec![Ok(terminal.into_bytes())]).await;
            assert!(result.is_ok());
            assert_eq!(response.status_code, 200);
            assert_eq!(response.stream_outcome, expected);
            assert_eq!(response.token_usage.total_tokens(), Some(150));
            assert!(received.contains(status));
        }
    }

    #[tokio::test]
    async fn late_transport_error_does_not_erase_delivered_terminal_tail() {
        let (response, result, received) = run_stream_test(vec![
            Ok(b"event: response.completed data: {\"type\":\"response.completed\"}".to_vec()),
            Err("upstream transport failed after completion".to_string()),
        ])
        .await;
        assert!(result.is_err());
        assert_eq!(response.stream_outcome, GatewayStreamOutcome::Completed);
        assert!(received.contains("response.completed"));
        assert!(!received.contains("response.failed"));
    }

    #[tokio::test]
    async fn stream_body_decode_error_demotes_to_incomplete_without_synthetic_event() {
        let (response, result, received) = run_stream_test(vec![
            Ok(b"data: {\"type\":\"response.created\"}\n\n".to_vec()),
            Ok(b"data: {\"type\":\"response.in_progress\"}\n\n".to_vec()),
            // reqwest-path text: every mid-body failure surfaces as this string.
            Err("Failed to read upstream response body: error decoding response body".to_string()),
        ])
        .await;

        assert!(result.is_ok());
        // No terminal event was delivered -> still Incomplete (marked red), but
        // the client stream was not corrupted with a synthetic error envelope.
        assert_eq!(response.stream_outcome, GatewayStreamOutcome::Incomplete);
        assert_eq!(response.error_category, None);
        assert!(response.note.contains("upstream body decode error demoted"));
        assert!(received.contains("response.created"));
        assert!(!received.contains("response.failed"));
        assert!(!received.contains("stream_incomplete"));
        assert!(received.ends_with("0\r\n\r\n"));
    }

    #[tokio::test]
    async fn stream_body_decode_error_after_terminal_tail_completes() {
        let (response, result, received) = run_stream_test(vec![
            Ok(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n".to_vec()),
            // Header-preserving-path text: raw hyper display for Kind::Body.
            // The terminal event has no trailing blank line, so only
            // `drain_terminal` can recover it after the demoted EOF.
            Ok(b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}".to_vec()),
            Err("Failed to read upstream response body: error reading a body from connection".to_string()),
        ])
        .await;

        assert!(result.is_ok());
        // The terminal event was actually delivered -> the request is green even
        // though the body decoder gave up on the framing afterwards.
        assert_eq!(response.stream_outcome, GatewayStreamOutcome::Completed);
        assert!(response.note.contains("upstream body decode error demoted"));
        assert!(received.contains("response.completed"));
        assert!(!received.contains("response.failed"));
        assert!(received.ends_with("0\r\n\r\n"));
    }

    #[tokio::test]
    async fn stream_non_decode_error_keeps_hard_failure_and_synthetic_event() {
        let (response, result, received) = run_stream_test(vec![
            Ok(b"data: {\"type\":\"response.created\"}\n\n".to_vec()),
            Err("Failed to read upstream response body: some other transport failure".to_string()),
        ])
        .await;

        assert!(result.is_err());
        assert_eq!(response.stream_outcome, GatewayStreamOutcome::Failed);
        assert_eq!(response.error_category.as_deref(), Some("stream_error"));
        assert!(response.note.contains("some other transport failure"));
        // Existing behavior for non-decode errors: a protocol-dialect error event
        // is still injected so the client sees an explicit failure.
        assert!(received.contains("event: response.failed"));
        assert!(received.ends_with("0\r\n\r\n"));
    }
}
