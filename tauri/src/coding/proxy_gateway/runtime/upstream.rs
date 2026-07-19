use super::compat::codex_responses_compact::{
    CodexResponsesCompactCompat, CODEX_RESPONSES_COMPACT_COMPAT_HEADER,
};
use super::header_preserving_client::{
    append_preserved_header, send_header_preserving_request, HeaderPreservingResponse,
    PreservedHeader,
};
use super::http_io::{
    empty_response, json_response, DebugBodyStream, DebugHttpRequest, DebugHttpResponse,
    SharedBodySnapshot,
};
use super::middleware::{
    BillingHeaderCchMiddleware, EnsureMaxTokensMiddleware, Middleware, PipelineContext,
};
use super::pipeline::Pipeline;
use super::providers::{ProviderAuthStrategy, UpstreamModelMapping, UpstreamProvider};
use super::routes::{build_target_url, match_gateway_route, split_request_target, GatewayRoute};
use super::side_stores::{
    record_gemini_sse_stream, record_responses_sse_stream, GeminiShadowSessionKey,
};
use super::GatewayRuntimeContext;
use super::{cache_injector, thinking_budget};
use crate::coding::proxy_gateway::model_health::{self, GatewayFailureKind};
use crate::coding::proxy_gateway::transformer::{
    check_lossy_conversion, convert_error_response_body, convert_request_body_with_context,
    convert_response_body_with_context, convert_sse_stream_with_context, AiProtocol,
    ConversionContext, ConversionRoute,
};
use crate::coding::proxy_gateway::types::{
    CodexChatReasoningMeta, GatewayCliKey, GatewayFailoverEvent, GatewayProviderAttempt,
    GatewayProxyMode, ProviderGatewayMeta, ProviderModelHealthKey,
};
use crate::coding::proxy_gateway::usage_parser::{
    from_response_body_with_provider_type, TokenUsage,
};
use crate::db::SqliteDbState;
use crate::http_client::{self, ProxyMode};
use futures_util::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONNECTION,
    CONTENT_LENGTH, CONTENT_TYPE, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use uuid::Uuid;

const ONE_M_CONTEXT_MARKER: &str = "[1m]";
const ENCODED_ONE_M_CONTEXT_MARKER: &str = "%5b1m%5d";
const UNSUPPORTED_IMAGE_MARKER: &str = "[Unsupported Image]";
const DEFAULT_COPILOT_WARMUP_MODEL: &str = "gpt-5-mini";
const DEFAULT_COPILOT_TOKEN_ENDPOINT: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_TOKEN_EXPIRY_BUFFER_SECS: i64 = 300;
const STREAM_SEMANTIC_PROBE_MAX_CHUNKS: usize = 32;
const STREAM_SEMANTIC_PROBE_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
struct CopilotTokenCacheEntry {
    token: String,
    expires_at: i64,
}

static COPILOT_TOKEN_CACHE: LazyLock<Mutex<HashMap<String, CopilotTokenCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Default)]
pub(super) struct GatewayRequestOptions {
    pub(super) provider_override_id: Option<String>,
    pub(super) disable_health_mutation: bool,
}

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

struct PreReadChunkStream {
    pending_chunks: VecDeque<Vec<u8>>,
    inner: super::http_io::DebugBodyStream,
}

impl Unpin for PreReadChunkStream {}

impl futures_util::Stream for PreReadChunkStream {
    type Item = Result<Vec<u8>, String>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let Some(chunk) = self.pending_chunks.pop_front() {
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
    let semantic_probe = response_is_sse_header_pairs(&response.headers);
    let mut pre_read_chunks = VecDeque::new();
    let mut probe = StreamingSemanticProbe::new(semantic_probe);
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
                let decision = probe.push_chunk(&chunk);
                pre_read_chunks.push_back(chunk);
                match decision {
                    StreamingProbeDecision::Meaningful => {
                        response.body_stream = Some(Box::pin(PreReadChunkStream {
                            pending_chunks: pre_read_chunks,
                            inner: body_stream,
                        }));
                        return Ok(());
                    }
                    StreamingProbeDecision::Continue => {
                        if probe.exceeded_limits() {
                            response.body_stream = Some(Box::pin(PreReadChunkStream {
                                pending_chunks: pre_read_chunks,
                                inner: body_stream,
                            }));
                            return Ok(());
                        }
                    }
                }
            }
            Some(Ok(_)) => continue,
            Some(Err(error)) => {
                return Err(GatewayForwardError::new(
                    error,
                    GatewayFailureKind::Connection,
                ));
            }
            None => {
                if pre_read_chunks.is_empty() {
                    return Err(GatewayForwardError::new(
                        "Upstream streaming response ended before first chunk",
                        GatewayFailureKind::Timeout,
                    ));
                }
                if probe.finish() == StreamingProbeDecision::Meaningful {
                    response.body_stream = Some(Box::pin(PreReadChunkStream {
                        pending_chunks: pre_read_chunks,
                        inner: body_stream,
                    }));
                    return Ok(());
                }
                return Err(GatewayForwardError::new(
                    "Upstream streaming response ended before meaningful content",
                    GatewayFailureKind::EmptyResponse,
                ));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingProbeDecision {
    Continue,
    Meaningful,
}

struct StreamingSemanticProbe {
    enabled: bool,
    buffer: Vec<u8>,
    chunk_count: usize,
    byte_count: usize,
}

impl StreamingSemanticProbe {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            buffer: Vec::new(),
            chunk_count: 0,
            byte_count: 0,
        }
    }

    fn push_chunk(&mut self, chunk: &[u8]) -> StreamingProbeDecision {
        self.chunk_count = self.chunk_count.saturating_add(1);
        self.byte_count = self.byte_count.saturating_add(chunk.len());
        if !self.enabled {
            return StreamingProbeDecision::Meaningful;
        }

        self.buffer.extend_from_slice(chunk);
        while let Some((end, delimiter_len)) = find_sse_block_delimiter(&self.buffer) {
            let block = self.buffer.drain(..end + delimiter_len).collect::<Vec<_>>();
            if sse_block_has_meaningful_content(&block) {
                return StreamingProbeDecision::Meaningful;
            }
        }
        StreamingProbeDecision::Continue
    }

    fn finish(&mut self) -> StreamingProbeDecision {
        if !self.enabled {
            return if self.byte_count > 0 {
                StreamingProbeDecision::Meaningful
            } else {
                StreamingProbeDecision::Continue
            };
        }
        if self.buffer.is_empty() {
            return StreamingProbeDecision::Continue;
        }
        let block = std::mem::take(&mut self.buffer);
        if sse_block_has_meaningful_content(&block) {
            StreamingProbeDecision::Meaningful
        } else {
            StreamingProbeDecision::Continue
        }
    }

    fn exceeded_limits(&self) -> bool {
        self.chunk_count >= STREAM_SEMANTIC_PROBE_MAX_CHUNKS
            || self.byte_count >= STREAM_SEMANTIC_PROBE_MAX_BYTES
    }
}

fn response_is_sse_header_pairs(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case(CONTENT_TYPE.as_str())
            && value.to_ascii_lowercase().contains("text/event-stream")
    })
}

fn sse_block_has_meaningful_content(block: &[u8]) -> bool {
    let block = String::from_utf8_lossy(block);
    let event_name = sse_event_name(&block);
    let Some(data) = sse_data_payload(&block) else {
        return false;
    };
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return true;
    };
    sse_value_has_meaningful_content(event_name.as_deref(), &value)
}

fn sse_event_name(block: &str) -> Option<String> {
    block.lines().find_map(|line| {
        line.strip_prefix("event:")
            .map(str::trim)
            .filter(|event| !event.is_empty())
            .map(ToString::to_string)
    })
}

fn sse_value_has_meaningful_content(event_name: Option<&str>, value: &Value) -> bool {
    if value.get("error").is_some()
        || event_name == Some("error")
        || value.get("type").and_then(Value::as_str) == Some("error")
    {
        return true;
    }

    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .or(event_name)
        .unwrap_or_default();
    match event_type {
        "response.created"
        | "response.in_progress"
        | "response.output_item.added"
        | "response.content_part.added"
        | "response.output_text.done"
        | "response.reasoning_summary_text.done"
        | "response.function_call_arguments.done"
        | "response.custom_tool_call_input.done"
        | "response.content_part.done"
        | "response.output_item.done"
        | "response.completed" => responses_sse_event_has_meaningful_content(event_type, value),
        "response.output_text.delta"
        | "response.reasoning_summary_text.delta"
        | "response.function_call_arguments.delta"
        | "response.custom_tool_call_input.delta" => {
            responses_sse_event_has_meaningful_content(event_type, value)
        }
        "message_start"
        | "content_block_start"
        | "content_block_delta"
        | "message_delta"
        | "message_stop"
        | "ping" => anthropic_sse_event_has_meaningful_content(event_type, value),
        _ => generic_sse_value_has_meaningful_content(value),
    }
}

fn responses_sse_event_has_meaningful_content(event_type: &str, value: &Value) -> bool {
    match event_type {
        "response.created" | "response.in_progress" | "response.content_part.added" => false,
        "response.output_text.delta" | "response.reasoning_summary_text.delta" => value
            .get("delta")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty()),
        "response.function_call_arguments.delta" => value
            .get("delta")
            .or_else(|| value.get("arguments"))
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty()),
        "response.custom_tool_call_input.delta" => value
            .get("delta")
            .or_else(|| value.get("input"))
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty()),
        "response.output_item.added" | "response.output_item.done" => value
            .get("item")
            .is_some_and(output_item_has_meaningful_content),
        "response.completed" => value
            .get("response")
            .and_then(|response| response.get("output"))
            .and_then(Value::as_array)
            .is_some_and(|output| output.iter().any(output_item_has_meaningful_content)),
        _ => false,
    }
}

fn anthropic_sse_event_has_meaningful_content(event_type: &str, value: &Value) -> bool {
    match event_type {
        "ping" | "message_start" | "message_stop" => false,
        "content_block_start" => value
            .get("content_block")
            .is_some_and(content_part_has_meaningful_content),
        "content_block_delta" => value
            .get("delta")
            .is_some_and(content_part_has_meaningful_content),
        "message_delta" => value
            .pointer("/delta/stop_reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| reason == "tool_use"),
        _ => false,
    }
}

fn generic_sse_value_has_meaningful_content(value: &Value) -> bool {
    if let Some(choices) = value.get("choices").and_then(Value::as_array) {
        return choices.iter().any(choice_has_meaningful_content);
    }
    if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
        return candidates.iter().any(candidate_has_meaningful_content);
    }
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        return content.iter().any(content_part_has_meaningful_content);
    }
    if let Some(output) = value.get("output").and_then(Value::as_array) {
        return output.iter().any(output_item_has_meaningful_content);
    }
    true
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
    route_request_with_options(request, context, &GatewayRequestOptions::default()).await
}

pub(super) async fn route_request_with_options(
    request: &DebugHttpRequest,
    context: &GatewayRuntimeContext,
    options: &GatewayRequestOptions,
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

    forward_to_upstream(request, db, context, &route, options).await
}

fn is_cli_route_probe(request: &DebugHttpRequest, route: &GatewayRoute) -> bool {
    if !matches!(request.method.as_str(), "GET" | "HEAD") {
        return false;
    }
    match route.cli_key {
        GatewayCliKey::Claude => route.forwarded_path == "/",
        GatewayCliKey::Codex => route.forwarded_path == "/v1",
        GatewayCliKey::Grok => route.forwarded_path == "/v1",
        GatewayCliKey::Gemini => route.forwarded_path == "/v1beta",
        GatewayCliKey::OpenCode => false,
    }
}

async fn forward_to_upstream(
    request: &DebugHttpRequest,
    db: &SqliteDbState,
    context: &GatewayRuntimeContext,
    route: &GatewayRoute,
    options: &GatewayRequestOptions,
) -> DebugHttpResponse {
    let requested_model =
        extract_requested_model(request, route).unwrap_or_else(|| "unknown".to_string());
    let mut provider_candidates = match context.load_candidate_providers(db, route.cli_key).await {
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
    if let Some(provider_override_id) = options.provider_override_id.as_deref() {
        provider_candidates
            .providers
            .retain(|provider| provider.id == provider_override_id);
        if provider_candidates.providers.is_empty() {
            let mut response = json_response(
                502,
                "Bad Gateway",
                json!({
                    "error": "gateway_provider_missing",
                    "message": format!(
                        "Provider '{}' is not an enabled Gateway candidate for {}",
                        provider_override_id,
                        route.cli_key.as_str(),
                    ),
                }),
                route.route_name,
                None,
                "test request specified a provider that is not available for this CLI route",
            );
            response.cli_key = Some(route.cli_key);
            response.requested_model = Some(requested_model);
            response.error_category = Some("provider_missing".to_string());
            return response;
        }
    }
    // Connectivity tests pin a provider and model; never rewrite those requests.
    let allow_provider_model_mapping = options.provider_override_id.is_none();
    // Claude family + Codex default-model rewrite only run in failover mode.
    // Grok always rewrites when allowed (CLI takeover hardcodes model=grok-build).
    let apply_failover_model_mapping = allow_provider_model_mapping
        && !provider_candidates
            .selection
            .as_ref()
            .is_some_and(|selection| selection.mode == GatewayProxyMode::Single);
    let providers = provider_candidates.providers;

    let settings = context.settings_snapshot();
    let app_config = settings.effective_app_config(route.cli_key);
    // Settings are normalized before they reach runtime; parse errors here only
    // cover corrupted in-memory state and should not be a normal save path.
    let retryable_status_codes =
        crate::coding::proxy_gateway::retryable_status::retryable_status_code_set(
            &settings.retryable_status_codes,
        )
        .unwrap_or_else(|_| {
            crate::coding::proxy_gateway::retryable_status::default_retryable_status_codes()
                .into_iter()
                .collect()
        });
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
            apply_failover_model_mapping,
            allow_provider_model_mapping,
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
                context,
                request,
                db,
                route,
                &provider,
                &requested_model,
                &upstream_model_id,
                settings.thinking_rectifier_enabled,
                settings.responses_encrypted_content_rectifier_enabled,
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
                                if !options.disable_health_mutation {
                                    health_changed |=
                                        record_health_failure(context, &health_key, failure_kind);
                                }
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
                                    failure_kind,
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

                    if let Some(failure_kind) = classify_empty_success_response(&response)
                        .or_else(|| classify_status_failure(response.status_code))
                    {
                        let category = model_health::classify_failure(failure_kind).category;
                        if failure_kind == GatewayFailureKind::EmptyResponse {
                            response = empty_success_failure_response(
                                route,
                                &provider,
                                &requested_model,
                                &upstream_model_id,
                                response,
                                attempt_count,
                                provider_retry_count.saturating_add(1),
                                attempted_provider_count > 1,
                            );
                        }
                        response.error_category = Some(category.to_string());
                        if !options.disable_health_mutation {
                            health_changed |=
                                record_health_failure(context, &health_key, failure_kind);
                        }
                        if should_retry_status_or_kind(
                            Some(response.status_code),
                            failure_kind,
                            &retryable_status_codes,
                        ) {
                            provider_attempts.push(provider_attempt_log(&response));
                            last_failure_response = Some(response);
                            if can_retry_current_provider(
                                failure_kind,
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
                        if !options.disable_health_mutation {
                            health_changed |= record_health_success(context, &health_key);
                        }
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
                    if error.kind == GatewayFailureKind::RequestSchema {
                        save_health_registry_if_needed(context, health_changed);
                        let mut response = local_request_schema_failure_response(
                            route,
                            &provider,
                            &requested_model,
                            &upstream_model_id,
                            error,
                            attempt_count,
                            provider_retry_count.saturating_add(1),
                            attempted_provider_count > 1,
                        );
                        response.provider_attempts = provider_attempts;
                        return response;
                    }
                    let category = model_health::classify_failure(error.kind).category;
                    if !options.disable_health_mutation {
                        health_changed |= record_health_failure(context, &health_key, error.kind);
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
                        error.kind,
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
    context: &GatewayRuntimeContext,
    request: &DebugHttpRequest,
    db: &SqliteDbState,
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    requested_model: &str,
    upstream_model_id: &str,
    thinking_rectifier_enabled: bool,
    responses_encrypted_content_rectifier_enabled: bool,
    thinking_budget_rectifier_enabled: bool,
    cache_injection_enabled: bool,
    non_streaming_timeout_secs: u64,
    upstream_response_snapshot_limit: Option<usize>,
) -> Result<DebugHttpResponse, GatewayForwardError> {
    let effective_upstream_model_id =
        effective_upstream_model_id_for_request(provider, upstream_model_id, request);
    let upstream_model_id = effective_upstream_model_id.as_ref();
    let mut effective_provider =
        effective_upstream_provider_for_request(provider, upstream_model_id);
    let provider = &effective_provider;
    let compact_compat = CodexResponsesCompactCompat::new(route, provider);
    let source_protocol = source_protocol_from_route(route);
    let conversion_route = compact_compat.conversion_route().or_else(|| {
        source_protocol.and_then(|source_protocol| conversion_route(source_protocol, provider))
    });
    if let Err(message) = compact_compat.validate_request(route, &request.body) {
        return Err(GatewayForwardError::new(
            message,
            GatewayFailureKind::RequestSchema,
        ));
    }
    let method = reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|error| {
        GatewayForwardError::new(
            format!("Invalid HTTP method '{}': {error}", request.method),
            GatewayFailureKind::RequestSchema,
        )
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
        Some(&provider.meta),
        Some(context),
        Some(provider),
        route_declares_streaming(route),
        compact_compat,
    )?;
    let mut upstream_body = prepared_upstream_body.body;
    let conversion_context = prepared_upstream_body.conversion_context;
    if should_filter_known_invalid_responses_ciphers(
        responses_encrypted_content_rectifier_enabled,
        provider.target_protocol,
        compact_compat,
    ) {
        strip_known_invalid_responses_ciphers(context, provider, &mut upstream_body)?;
    }
    let upstream_body_snapshot = upstream_body.clone();
    let client =
        http_client::client_with_timeout_no_compression(db, non_streaming_timeout_secs.max(1))
            .await
            .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::Connection))?;
    if let Some(copilot_token) = resolve_copilot_token_for_provider(&client, provider)
        .await
        .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::Connection))?
    {
        effective_provider.api_key = copilot_token;
        effective_provider.auth_strategy = ProviderAuthStrategy::Bearer;
    }
    let provider = &effective_provider;
    let headers =
        build_upstream_headers(request, provider, Some(&upstream_body)).map_err(|message| {
            GatewayForwardError {
                message,
                kind: GatewayFailureKind::GatewayParse,
                upstream_request_body: None,
                upstream_response_body: None,
                upstream_response_body_bytes: 0,
            }
        })?;
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
        upstream_model_id,
    )
    .map_err(|message| GatewayForwardError::new(message, GatewayFailureKind::GatewayParse))?;

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
    let should_attempt_responses_encrypted_content_rectifier =
        should_attempt_responses_encrypted_content_rectifier(
            responses_encrypted_content_rectifier_enabled,
            provider.target_protocol,
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
        || should_attempt_responses_encrypted_content_rectifier
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
                Some(&provider.meta),
                Some(context),
                Some(provider),
                route_declares_streaming(route),
                compact_compat,
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
                    Some(context),
                    compact_compat,
                )
                .await;
            }
        }

        if should_attempt_responses_encrypted_content_rectifier {
            let encrypted_content_error =
                detect_responses_encrypted_content_error(status_code, &body);
            if let Some(encrypted_content_error) = encrypted_content_error {
                context.side_stores.remember_rejected_responses_ciphers(
                    responses_cipher_provider_config_identity(provider),
                    &upstream_body_snapshot,
                    encrypted_content_error
                        .message
                        .as_deref()
                        .unwrap_or_default(),
                );
                if let Some(rectified_body) =
                    build_responses_encrypted_content_rectified_body(&upstream_body_snapshot)?
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
                        Some(context),
                        compact_compat,
                    )
                    .await;
                }
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
                    Some(context),
                    compact_compat,
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
                    Some(context),
                    compact_compat,
                )
                .await;
            }
        }
        let upstream_response_body = body.clone();
        let body = convert_buffered_error_body(
            conversion_route,
            compact_compat,
            body,
            &mut response_headers,
        );
        append_compact_compat_header(&mut response_headers, compact_compat);
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
        Some(context),
        compact_compat,
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
    context: Option<&GatewayRuntimeContext>,
    compact_compat: CodexResponsesCompactCompat,
) -> Result<DebugHttpResponse, GatewayForwardError> {
    let status = response.status();
    let mut response_headers = filtered_response_headers(response.headers());
    append_lossy_warning_header(&mut response_headers, conversion_context.as_ref());
    let provider_kind =
        ProviderBodyCompat::from_provider_meta(Some(&provider.meta), provider.target_protocol);
    let should_stream = should_stream_response(request, route, response.headers(), status.as_u16());
    let response_conversion_route = conversion_route.map(ConversionRoute::reverse);

    if let Some(aggregate_kind) = sse_aggregation_kind_for_non_streaming_client(
        request,
        route,
        provider,
        response.headers(),
        status.as_u16(),
    ) {
        let (upstream_response_body, mut body) =
            aggregate_sse_stream_for_non_streaming_client(aggregate_kind, response.bytes_stream())
                .await
                .map_err(|mut error| {
                    error.upstream_request_body = Some(upstream_body_snapshot.clone());
                    error
                })?;
        let upstream_response_body_bytes = upstream_response_body.len() as u64;
        let target_response_body = body.clone();
        if compact_compat.is_compact() {
            body = compact_compat
                .convert_response_body(&body, conversion_context.as_ref())
                .map_err(|error| GatewayForwardError {
                    message: error.to_string(),
                    kind: GatewayFailureKind::GatewayParse,
                    upstream_request_body: Some(upstream_body_snapshot.clone()),
                    upstream_response_body: Some(upstream_response_body.clone()),
                    upstream_response_body_bytes,
                })?;
        } else if let Some(route) = response_conversion_route {
            body = convert_response_body_with_context(route, &body, conversion_context.as_ref())
                .map_err(|error| GatewayForwardError {
                    message: error.to_string(),
                    kind: GatewayFailureKind::GatewayParse,
                    upstream_request_body: Some(upstream_body_snapshot.clone()),
                    upstream_response_body: Some(upstream_response_body.clone()),
                    upstream_response_body_bytes,
                })?;
        }
        set_response_content_type(&mut response_headers, "application/json");
        append_compact_compat_header(&mut response_headers, compact_compat);
        record_side_store_response(
            context,
            provider,
            request,
            &upstream_body_snapshot,
            &upstream_response_body,
            &target_response_body,
            &body,
            response_conversion_route,
        );
        let token_usage = from_response_body_with_provider_type(
            provider.cli_key,
            provider.meta.provider_type.as_deref(),
            &body,
        );

        return Ok(DebugHttpResponse {
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
            upstream_response_body: Some(upstream_response_body),
            upstream_response_body_bytes,
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
                "aggregated {} streaming response from provider id={} name={}",
                aggregate_kind.label(),
                provider.id,
                provider.name
            ),
        });
    }

    if should_stream {
        let converts_ollama_stream = (200..400).contains(&status.as_u16())
            && provider_kind == Some(ProviderBodyCompat::Ollama);
        if response_conversion_route.is_some() || converts_ollama_stream {
            set_response_content_type(&mut response_headers, "text/event-stream");
        }
        let upstream_response_body_stream_snapshot = (response_conversion_route.is_some()
            || converts_ollama_stream)
            .then(|| upstream_response_snapshot_limit)
            .flatten()
            .map(SharedBodySnapshot::new);
        let raw_body_stream = match upstream_response_body_stream_snapshot.clone() {
            Some(snapshot) => snapshot_response_stream(response.bytes_stream(), snapshot),
            None => response.bytes_stream(),
        };
        let raw_body_stream = maybe_record_gemini_sse_stream(
            raw_body_stream,
            context,
            provider,
            request,
            &upstream_body_snapshot,
        );
        let raw_body_stream =
            maybe_filter_bailian_openai_chat_sse_stream(raw_body_stream, provider);
        let raw_body_stream = maybe_filter_xai_openai_chat_sse_stream(raw_body_stream, provider);
        let raw_body_stream = if converts_ollama_stream {
            convert_ollama_ndjson_stream_to_openai_chat_sse(raw_body_stream)
        } else {
            raw_body_stream
        };
        let body_stream = match response_conversion_route {
            Some(route) => {
                convert_sse_stream_with_context(route, raw_body_stream, conversion_context.clone())
            }
            None => raw_body_stream,
        };
        let body_stream = maybe_record_codex_responses_sse_stream(
            body_stream,
            context,
            response_conversion_route,
        );
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
    if (200..400).contains(&status.as_u16()) && provider_kind == Some(ProviderBodyCompat::Ollama) {
        body = convert_ollama_chat_response_to_openai_chat(&body).map_err(|message| {
            GatewayForwardError {
                message,
                kind: GatewayFailureKind::GatewayParse,
                upstream_request_body: Some(upstream_body_snapshot.clone()),
                upstream_response_body: Some(upstream_response_body.clone()),
                upstream_response_body_bytes,
            }
        })?;
        set_response_content_type(&mut response_headers, "application/json");
    }
    if compact_compat.is_compact() && (200..400).contains(&status.as_u16()) {
        body = compact_compat
            .convert_response_body(&body, conversion_context.as_ref())
            .map_err(|error| GatewayForwardError {
                message: error.to_string(),
                kind: GatewayFailureKind::GatewayParse,
                upstream_request_body: Some(upstream_body_snapshot.clone()),
                upstream_response_body: Some(upstream_response_body.clone()),
                upstream_response_body_bytes,
            })?;
        set_response_content_type(&mut response_headers, "application/json");
    } else if compact_compat.is_compact() {
        let converted_error_body = compact_compat.convert_error_response_body(&body);
        if converted_error_body != body {
            body = converted_error_body;
            set_response_content_type(&mut response_headers, "application/json");
        }
    } else if let Some(route) = response_conversion_route {
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
    append_compact_compat_header(&mut response_headers, compact_compat);
    record_side_store_response(
        context,
        provider,
        request,
        &upstream_body_snapshot,
        &upstream_response_body,
        &upstream_response_body,
        &body,
        response_conversion_route,
    );
    let token_usage = from_response_body_with_provider_type(
        provider.cli_key,
        provider.meta.provider_type.as_deref(),
        &body,
    );
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SseAggregateKind {
    OpenAiResponses,
    OpenAiChat,
    AnthropicMessages,
    GeminiNative,
}

impl SseAggregateKind {
    fn label(self) -> &'static str {
        match self {
            Self::OpenAiResponses => "OpenAI Responses",
            Self::OpenAiChat => "OpenAI Chat",
            Self::AnthropicMessages => "Anthropic Messages",
            Self::GeminiNative => "Gemini Native",
        }
    }
}

fn sse_aggregation_kind_for_non_streaming_client(
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    headers: &HeaderMap,
    status_code: u16,
) -> Option<SseAggregateKind> {
    if !(200..400).contains(&status_code)
        || !response_is_sse(headers)
        || request_declares_streaming(request)
        || route_declares_streaming(route)
    {
        return None;
    }
    match provider.target_protocol {
        AiProtocol::OpenAiResponses => Some(SseAggregateKind::OpenAiResponses),
        AiProtocol::OpenAiChat => Some(SseAggregateKind::OpenAiChat),
        AiProtocol::AnthropicMessages => Some(SseAggregateKind::AnthropicMessages),
        AiProtocol::GeminiNative => Some(SseAggregateKind::GeminiNative),
    }
}

#[cfg(test)]
fn should_aggregate_openai_responses_sse_for_non_streaming_client(
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    headers: &HeaderMap,
    status_code: u16,
) -> bool {
    sse_aggregation_kind_for_non_streaming_client(request, route, provider, headers, status_code)
        == Some(SseAggregateKind::OpenAiResponses)
}

#[cfg(test)]
fn should_aggregate_openai_chat_sse_for_non_streaming_client(
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    headers: &HeaderMap,
    status_code: u16,
) -> bool {
    sse_aggregation_kind_for_non_streaming_client(request, route, provider, headers, status_code)
        == Some(SseAggregateKind::OpenAiChat)
}

async fn aggregate_sse_stream_for_non_streaming_client(
    kind: SseAggregateKind,
    stream: DebugBodyStream,
) -> Result<(Vec<u8>, Vec<u8>), GatewayForwardError> {
    match kind {
        SseAggregateKind::OpenAiResponses => aggregate_openai_responses_sse_stream(stream).await,
        SseAggregateKind::OpenAiChat => aggregate_openai_chat_sse_stream(stream).await,
        SseAggregateKind::AnthropicMessages => aggregate_anthropic_sse_stream(stream).await,
        SseAggregateKind::GeminiNative => aggregate_gemini_sse_stream(stream).await,
    }
}

async fn aggregate_openai_responses_sse_stream(
    mut stream: DebugBodyStream,
) -> Result<(Vec<u8>, Vec<u8>), GatewayForwardError> {
    let mut raw_body = Vec::new();
    let mut aggregate = OpenAiResponsesSseAggregate::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| GatewayForwardError::new(error, GatewayFailureKind::Connection))?;
        raw_body.extend_from_slice(&chunk);
        aggregate.push_chunk(&chunk);
    }
    let body = aggregate.finish();
    Ok((raw_body, body))
}

async fn aggregate_openai_chat_sse_stream(
    mut stream: DebugBodyStream,
) -> Result<(Vec<u8>, Vec<u8>), GatewayForwardError> {
    let mut raw_body = Vec::new();
    let mut aggregate = OpenAiChatSseAggregate::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| GatewayForwardError::new(error, GatewayFailureKind::Connection))?;
        raw_body.extend_from_slice(&chunk);
        aggregate.push_chunk(&chunk);
    }
    let body = aggregate.finish();
    Ok((raw_body, body))
}

async fn aggregate_anthropic_sse_stream(
    mut stream: DebugBodyStream,
) -> Result<(Vec<u8>, Vec<u8>), GatewayForwardError> {
    let mut raw_body = Vec::new();
    let mut aggregate = AnthropicSseAggregate::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| GatewayForwardError::new(error, GatewayFailureKind::Connection))?;
        raw_body.extend_from_slice(&chunk);
        aggregate.push_chunk(&chunk);
    }
    let body = aggregate.finish();
    Ok((raw_body, body))
}

async fn aggregate_gemini_sse_stream(
    mut stream: DebugBodyStream,
) -> Result<(Vec<u8>, Vec<u8>), GatewayForwardError> {
    let mut raw_body = Vec::new();
    let mut aggregate = GeminiSseAggregate::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| GatewayForwardError::new(error, GatewayFailureKind::Connection))?;
        raw_body.extend_from_slice(&chunk);
        aggregate.push_chunk(&chunk);
    }
    let body = aggregate.finish();
    Ok((raw_body, body))
}

fn convert_ollama_chat_response_to_openai_chat(body: &[u8]) -> Result<Vec<u8>, String> {
    let value = serde_json::from_slice::<Value>(body)
        .map_err(|error| format!("Failed to parse Ollama chat response body: {error}"))?;
    let response = ollama_chat_response_value_to_openai_chat(value);
    serde_json::to_vec(&response)
        .map_err(|error| format!("Failed to serialize Ollama chat response body: {error}"))
}

fn ollama_chat_response_value_to_openai_chat(value: Value) -> Value {
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let message = value.get("message").cloned().unwrap_or_else(|| json!({}));
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("assistant")
        .to_string();
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let thinking = message
        .get("thinking")
        .and_then(Value::as_str)
        .filter(|thinking| !thinking.trim().is_empty())
        .map(str::to_string);
    let finish_reason = ollama_done_reason_to_openai(
        value
            .get("done_reason")
            .and_then(Value::as_str)
            .unwrap_or("stop"),
    );

    let mut openai_message = serde_json::Map::new();
    openai_message.insert("role".to_string(), Value::String(role));
    openai_message.insert("content".to_string(), Value::String(content));
    if let Some(thinking) = thinking {
        openai_message.insert("reasoning_content".to_string(), Value::String(thinking));
    }

    json!({
        "id": "chatcmpl-ollama",
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": Value::Object(openai_message),
                "finish_reason": finish_reason
            }
        ],
        "usage": ollama_usage_value(&value)
    })
}

fn ollama_usage_value(value: &Value) -> Value {
    let prompt_tokens = value
        .get("prompt_eval_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = value.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
    json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": prompt_tokens + completion_tokens
    })
}

fn ollama_done_reason_to_openai(reason: &str) -> &'static str {
    match reason.trim().to_ascii_lowercase().as_str() {
        "length" | "max_tokens" | "limit" => "length",
        _ => "stop",
    }
}

struct OllamaNdjsonSseState {
    inner: DebugBodyStream,
    buffer: Vec<u8>,
    pending: VecDeque<Vec<u8>>,
    sent_role: bool,
    done: bool,
}

fn convert_ollama_ndjson_stream_to_openai_chat_sse(stream: DebugBodyStream) -> DebugBodyStream {
    Box::pin(futures_util::stream::unfold(
        OllamaNdjsonSseState {
            inner: stream,
            buffer: Vec::new(),
            pending: VecDeque::new(),
            sent_role: false,
            done: false,
        },
        |mut state| async move {
            loop {
                if let Some(chunk) = state.pending.pop_front() {
                    return Some((Ok(chunk), state));
                }
                let Some(chunk) = state.inner.next().await else {
                    if !state.buffer.is_empty() {
                        let tail = std::mem::take(&mut state.buffer);
                        state.push_line(&tail);
                        continue;
                    }
                    if !state.done {
                        state.done = true;
                        return Some((Ok(b"data: [DONE]\n\n".to_vec()), state));
                    }
                    return None;
                };
                match chunk {
                    Ok(chunk) => {
                        state.buffer.extend_from_slice(&chunk);
                        while let Some(position) =
                            state.buffer.iter().position(|byte| *byte == b'\n')
                        {
                            let line = state.buffer.drain(..=position).collect::<Vec<_>>();
                            state.push_line(&line);
                        }
                    }
                    Err(error) => return Some((Err(error), state)),
                }
            }
        },
    ))
}

impl OllamaNdjsonSseState {
    fn push_line(&mut self, line: &[u8]) {
        let trimmed = trim_ascii_line(line);
        if trimmed.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(trimmed) else {
            return;
        };
        let model = value.get("model").and_then(Value::as_str).unwrap_or("");
        let mut delta = serde_json::Map::new();
        if !self.sent_role {
            delta.insert("role".to_string(), Value::String("assistant".to_string()));
            self.sent_role = true;
        }
        if let Some(content) = value
            .pointer("/message/content")
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
        {
            delta.insert("content".to_string(), Value::String(content.to_string()));
        }
        if let Some(thinking) = value
            .pointer("/message/thinking")
            .and_then(Value::as_str)
            .filter(|thinking| !thinking.is_empty())
        {
            delta.insert(
                "reasoning_content".to_string(),
                Value::String(thinking.to_string()),
            );
        }
        let done = value.get("done").and_then(Value::as_bool).unwrap_or(false);
        if !delta.is_empty() || !done {
            let finish_reason = if done {
                Some(ollama_done_reason_to_openai(
                    value
                        .get("done_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("stop"),
                ))
            } else {
                None
            };
            self.pending.push_back(openai_chat_sse_chunk(
                model,
                Value::Object(delta),
                finish_reason,
                None,
            ));
        }
        if done {
            self.done = true;
            if delta_is_empty_done_without_chunk(&value) {
                self.pending.push_back(openai_chat_sse_chunk(
                    model,
                    json!({}),
                    Some(ollama_done_reason_to_openai(
                        value
                            .get("done_reason")
                            .and_then(Value::as_str)
                            .unwrap_or("stop"),
                    )),
                    None,
                ));
            }
            if let Some(usage) = ollama_stream_usage_chunk(model, &value) {
                self.pending.push_back(usage);
            }
            self.pending.push_back(b"data: [DONE]\n\n".to_vec());
        }
    }
}

fn trim_ascii_line(line: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = line.len();
    while start < end && line[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &line[start..end]
}

fn delta_is_empty_done_without_chunk(value: &Value) -> bool {
    value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
        && value
            .pointer("/message/thinking")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
}

fn ollama_stream_usage_chunk(model: &str, value: &Value) -> Option<Vec<u8>> {
    let prompt_tokens = value.get("prompt_eval_count").and_then(Value::as_u64)?;
    let completion_tokens = value.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
    Some(openai_chat_sse_chunk(
        model,
        json!({}),
        None,
        Some(json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        })),
    ))
}

fn openai_chat_sse_chunk(
    model: &str,
    delta: Value,
    finish_reason: Option<&str>,
    usage: Option<Value>,
) -> Vec<u8> {
    let mut chunk = json!({
        "id": "chatcmpl-ollama",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": model,
        "choices": [
            {
                "index": 0,
                "delta": delta,
                "finish_reason": finish_reason
            }
        ]
    });
    if let Some(usage) = usage {
        if let Some(object) = chunk.as_object_mut() {
            object.insert("usage".to_string(), usage);
        }
    }
    let mut bytes = b"data: ".to_vec();
    bytes.extend_from_slice(
        serde_json::to_string(&chunk)
            .unwrap_or_else(|_| "{}".to_string())
            .as_bytes(),
    );
    bytes.extend_from_slice(b"\n\n");
    bytes
}

#[derive(Debug, Default)]
struct AnthropicSseAggregate {
    buffer: Vec<u8>,
    message: Option<Value>,
    content_blocks: HashMap<i64, AnthropicContentBlockAggregate>,
    stop_reason: Option<String>,
    stop_sequence: Option<Value>,
    usage: Option<Value>,
}

#[derive(Debug)]
struct AnthropicContentBlockAggregate {
    index: i64,
    block: Value,
    partial_json: String,
}

impl AnthropicSseAggregate {
    fn push_chunk(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        while let Some((end, delimiter_len)) = find_sse_block_delimiter(&self.buffer) {
            let block = self.buffer.drain(..end + delimiter_len).collect::<Vec<_>>();
            self.push_block(&block);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if !self.buffer.is_empty() {
            let tail = std::mem::take(&mut self.buffer);
            self.push_block(&tail);
        }
        let response = self.response_value();
        serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec())
    }

    fn push_block(&mut self, block: &[u8]) {
        let block_text = String::from_utf8_lossy(block);
        let Some(data) = sse_data_payload(&block_text) else {
            return;
        };
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(message) = value.get("message").cloned() {
                    if let Some(usage) = message.get("usage") {
                        merge_usage_object(&mut self.usage, usage);
                    }
                    self.message = Some(message);
                }
            }
            Some("content_block_start") => {
                let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
                let block = value
                    .get("content_block")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "text", "text": ""}));
                self.content_blocks
                    .insert(index, AnthropicContentBlockAggregate::new(index, block));
            }
            Some("content_block_delta") => {
                let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
                let delta = value.get("delta").unwrap_or(&Value::Null);
                self.push_delta(index, delta);
            }
            Some("message_delta") => {
                if let Some(delta) = value.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(Value::as_str) {
                        if !reason.trim().is_empty() {
                            self.stop_reason = Some(reason.to_string());
                        }
                    }
                    if let Some(sequence) = delta.get("stop_sequence") {
                        self.stop_sequence = Some(sequence.clone());
                    }
                }
                if let Some(usage) = value.get("usage") {
                    merge_usage_object(&mut self.usage, usage);
                }
            }
            _ => {}
        }
    }

    fn push_delta(&mut self, index: i64, delta: &Value) {
        if let Some(text) = delta.get("text").and_then(Value::as_str) {
            self.block_entry(index, "text")
                .append_string_field("text", text);
        }
        if let Some(thinking) = delta.get("thinking").and_then(Value::as_str) {
            self.block_entry(index, "thinking")
                .append_string_field("thinking", thinking);
        }
        if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
            self.block_entry(index, "thinking")
                .set_string_field("signature", signature);
        }
        if let Some(partial_json) = delta.get("partial_json").and_then(Value::as_str) {
            self.block_entry(index, "tool_use")
                .partial_json
                .push_str(partial_json);
        }
    }

    fn block_entry(&mut self, index: i64, block_type: &str) -> &mut AnthropicContentBlockAggregate {
        self.content_blocks.entry(index).or_insert_with(|| {
            let block = match block_type {
                "thinking" => json!({"type": "thinking", "thinking": ""}),
                "tool_use" => json!({"type": "tool_use", "id": "", "name": "", "input": {}}),
                _ => json!({"type": "text", "text": ""}),
            };
            AnthropicContentBlockAggregate::new(index, block)
        })
    }

    fn response_value(mut self) -> Value {
        let mut response = self.message.take().unwrap_or_else(|| {
            json!({
                "id": "msg_gateway_aggregated",
                "type": "message",
                "role": "assistant",
                "model": ""
            })
        });
        let mut blocks = self.content_blocks.into_values().collect::<Vec<_>>();
        blocks.sort_by_key(|block| block.index);
        let content = blocks
            .into_iter()
            .map(AnthropicContentBlockAggregate::into_block)
            .collect::<Vec<_>>();
        if let Some(object) = response.as_object_mut() {
            object
                .entry("type")
                .or_insert_with(|| Value::String("message".to_string()));
            object
                .entry("role")
                .or_insert_with(|| Value::String("assistant".to_string()));
            object.insert("content".to_string(), Value::Array(content));
            object.insert(
                "stop_reason".to_string(),
                self.stop_reason.map(Value::String).unwrap_or(Value::Null),
            );
            object.insert(
                "stop_sequence".to_string(),
                self.stop_sequence.unwrap_or(Value::Null),
            );
            if let Some(usage) = self.usage {
                object.insert("usage".to_string(), usage);
            }
        }
        response
    }
}

impl AnthropicContentBlockAggregate {
    fn new(index: i64, block: Value) -> Self {
        Self {
            index,
            block,
            partial_json: String::new(),
        }
    }

    fn append_string_field(&mut self, field: &str, value: &str) {
        let Some(object) = self.block.as_object_mut() else {
            return;
        };
        let current = object
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        object.insert(
            field.to_string(),
            Value::String(format!("{current}{value}")),
        );
    }

    fn set_string_field(&mut self, field: &str, value: &str) {
        if let Some(object) = self.block.as_object_mut() {
            object.insert(field.to_string(), Value::String(value.to_string()));
        }
    }

    fn into_block(mut self) -> Value {
        if !self.partial_json.is_empty() {
            let input = serde_json::from_str::<Value>(&self.partial_json)
                .unwrap_or_else(|_| Value::String(self.partial_json));
            if let Some(object) = self.block.as_object_mut() {
                object.insert("input".to_string(), input);
            }
        }
        self.block
    }
}

#[derive(Debug, Default)]
struct GeminiSseAggregate {
    buffer: Vec<u8>,
    response_id: Option<String>,
    model_version: Option<String>,
    usage_metadata: Option<Value>,
    prompt_feedback: Option<Value>,
    candidates: HashMap<i64, GeminiCandidateAggregate>,
}

#[derive(Debug, Default)]
struct GeminiCandidateAggregate {
    index: i64,
    role: Option<String>,
    parts: Vec<Value>,
    finish_reason: Option<String>,
    extra_fields: serde_json::Map<String, Value>,
}

impl GeminiSseAggregate {
    fn push_chunk(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        while let Some((end, delimiter_len)) = find_sse_block_delimiter(&self.buffer) {
            let block = self.buffer.drain(..end + delimiter_len).collect::<Vec<_>>();
            self.push_block(&block);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if !self.buffer.is_empty() {
            let tail = std::mem::take(&mut self.buffer);
            self.push_block(&tail);
        }
        let response = self.response_value();
        serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec())
    }

    fn push_block(&mut self, block: &[u8]) {
        let block_text = String::from_utf8_lossy(block);
        let Some(data) = sse_data_payload(&block_text) else {
            return;
        };
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        if self.response_id.is_none() {
            self.response_id = value
                .get("responseId")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if self.model_version.is_none() {
            self.model_version = value
                .get("modelVersion")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if let Some(usage) = value.get("usageMetadata") {
            self.usage_metadata = Some(usage.clone());
        }
        if let Some(prompt_feedback) = value.get("promptFeedback") {
            self.prompt_feedback = Some(prompt_feedback.clone());
        }
        let Some(candidates) = value.get("candidates").and_then(Value::as_array) else {
            return;
        };
        for candidate in candidates {
            self.push_candidate(candidate);
        }
    }

    fn push_candidate(&mut self, candidate: &Value) {
        let index = candidate.get("index").and_then(Value::as_i64).unwrap_or(0);
        let entry = self
            .candidates
            .entry(index)
            .or_insert_with(|| GeminiCandidateAggregate {
                index,
                ..GeminiCandidateAggregate::default()
            });
        if let Some(finish_reason) = candidate.get("finishReason").and_then(Value::as_str) {
            if !finish_reason.trim().is_empty() {
                entry.finish_reason = Some(finish_reason.to_string());
            }
        }
        if let Some(content) = candidate.get("content") {
            if let Some(role) = content.get("role").and_then(Value::as_str) {
                entry.role = Some(role.to_string());
            }
            if let Some(parts) = content.get("parts").and_then(Value::as_array) {
                for part in parts {
                    entry.merge_part(part);
                }
            }
        }
        if let Some(object) = candidate.as_object() {
            for (key, value) in object {
                if matches!(key.as_str(), "index" | "content" | "finishReason") {
                    continue;
                }
                entry.extra_fields.insert(key.clone(), value.clone());
            }
        }
    }

    fn response_value(self) -> Value {
        let mut candidates = self.candidates.into_values().collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.index);
        let mut response = serde_json::Map::new();
        if let Some(response_id) = self.response_id {
            response.insert("responseId".to_string(), Value::String(response_id));
        }
        if let Some(model_version) = self.model_version {
            response.insert("modelVersion".to_string(), Value::String(model_version));
        }
        response.insert(
            "candidates".to_string(),
            Value::Array(
                candidates
                    .into_iter()
                    .map(GeminiCandidateAggregate::into_value)
                    .collect(),
            ),
        );
        if let Some(usage) = self.usage_metadata {
            response.insert("usageMetadata".to_string(), usage);
        }
        if let Some(prompt_feedback) = self.prompt_feedback {
            response.insert("promptFeedback".to_string(), prompt_feedback);
        }
        Value::Object(response)
    }
}

impl GeminiCandidateAggregate {
    fn merge_part(&mut self, part: &Value) {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            let thought = part
                .get("thought")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(last) = self.parts.iter_mut().rev().find(|candidate_part| {
                candidate_part.get("text").is_some()
                    && candidate_part
                        .get("thought")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        == thought
            }) {
                append_json_string_field(last, "text", text);
                if let Some(signature) = part.get("thoughtSignature").cloned() {
                    if let Some(object) = last.as_object_mut() {
                        object.insert("thoughtSignature".to_string(), signature);
                    }
                }
                return;
            }
        }
        if let Some(function_call) = part.get("functionCall") {
            let name = function_call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some(existing) = self.parts.iter_mut().rev().find(|candidate_part| {
                candidate_part
                    .pointer("/functionCall/name")
                    .and_then(Value::as_str)
                    == Some(name)
            }) {
                merge_gemini_function_call(existing, function_call);
                return;
            }
        }
        self.parts.push(part.clone());
    }

    fn into_value(self) -> Value {
        let mut candidate = self.extra_fields;
        candidate.insert("index".to_string(), json!(self.index));
        candidate.insert(
            "content".to_string(),
            json!({
                "role": self.role.unwrap_or_else(|| "model".to_string()),
                "parts": self.parts
            }),
        );
        if let Some(finish_reason) = self.finish_reason {
            candidate.insert("finishReason".to_string(), Value::String(finish_reason));
        }
        Value::Object(candidate)
    }
}

fn append_json_string_field(value: &mut Value, field: &str, suffix: &str) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let current = object
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    object.insert(
        field.to_string(),
        Value::String(format!("{current}{suffix}")),
    );
}

fn merge_gemini_function_call(existing_part: &mut Value, incoming_function_call: &Value) {
    let Some(existing_call) = existing_part
        .get_mut("functionCall")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if let Some(id) = incoming_function_call.get("id").cloned() {
        existing_call.insert("id".to_string(), id);
    }
    if let Some(args) = incoming_function_call.get("args") {
        let existing_args = existing_call
            .entry("args".to_string())
            .or_insert_with(|| json!({}));
        merge_json_objects(existing_args, args);
    }
}

fn merge_json_objects(existing: &mut Value, incoming: &Value) {
    match (existing.as_object_mut(), incoming.as_object()) {
        (Some(existing_object), Some(incoming_object)) => {
            for (key, value) in incoming_object {
                existing_object.insert(key.clone(), value.clone());
            }
        }
        _ => {
            *existing = incoming.clone();
        }
    }
}

fn merge_usage_object(target: &mut Option<Value>, usage: &Value) {
    let Some(usage_object) = usage.as_object() else {
        *target = Some(usage.clone());
        return;
    };
    let target_value = target.get_or_insert_with(|| json!({}));
    let Some(target_object) = target_value.as_object_mut() else {
        *target = Some(usage.clone());
        return;
    };
    for (key, value) in usage_object {
        if let (Some(existing), Some(incoming)) = (
            target_object.get(key).and_then(Value::as_i64),
            value.as_i64(),
        ) {
            target_object.insert(key.clone(), json!(existing + incoming));
        } else {
            target_object.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Debug, Default)]
struct OpenAiChatSseAggregate {
    buffer: Vec<u8>,
    id: Option<String>,
    created: Option<i64>,
    model: Option<String>,
    system_fingerprint: Option<Value>,
    usage: Option<Value>,
    choices: HashMap<i64, OpenAiChatChoiceAggregate>,
}

#[derive(Debug, Default)]
struct OpenAiChatChoiceAggregate {
    index: i64,
    role: Option<String>,
    content: String,
    reasoning_content: String,
    finish_reason: Option<Value>,
    tool_calls: HashMap<i64, OpenAiChatToolCallAggregate>,
}

#[derive(Debug, Default)]
struct OpenAiChatToolCallAggregate {
    index: i64,
    id: Option<String>,
    call_type: Option<String>,
    function_name: Option<String>,
    arguments: String,
}

impl OpenAiChatSseAggregate {
    fn push_chunk(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        while let Some((end, delimiter_len)) = find_sse_block_delimiter(&self.buffer) {
            let block = self.buffer.drain(..end + delimiter_len).collect::<Vec<_>>();
            self.push_block(&block);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if !self.buffer.is_empty() {
            let tail = std::mem::take(&mut self.buffer);
            self.push_block(&tail);
        }
        let response = self.response_value();
        serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec())
    }

    fn push_block(&mut self, block: &[u8]) {
        let block_text = String::from_utf8_lossy(block);
        let Some(data) = sse_data_payload(&block_text) else {
            return;
        };
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        self.capture_top_level_fields(&value);
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(usage.clone());
        }
        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            return;
        };
        for choice in choices {
            self.push_choice(choice);
        }
    }

    fn capture_top_level_fields(&mut self, value: &Value) {
        if self.id.is_none() {
            self.id = value.get("id").and_then(Value::as_str).map(str::to_string);
        }
        if self.created.is_none() {
            self.created = value.get("created").and_then(Value::as_i64);
        }
        if self.model.is_none() {
            self.model = value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if self.system_fingerprint.is_none() {
            self.system_fingerprint = value.get("system_fingerprint").cloned();
        }
    }

    fn push_choice(&mut self, choice: &Value) {
        let index = choice.get("index").and_then(Value::as_i64).unwrap_or(0);
        let entry = self
            .choices
            .entry(index)
            .or_insert_with(|| OpenAiChatChoiceAggregate {
                index,
                ..OpenAiChatChoiceAggregate::default()
            });
        if let Some(finish_reason) = choice.get("finish_reason") {
            if !finish_reason.is_null()
                && finish_reason
                    .as_str()
                    .is_none_or(|value| !value.trim().is_empty())
            {
                entry.finish_reason = Some(finish_reason.clone());
            }
        }
        let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
            return;
        };
        if let Some(role) = delta.get("role").and_then(Value::as_str) {
            entry.role = Some(role.to_string());
        }
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            entry.content.push_str(content);
        }
        for field in ["reasoning_content", "reasoning"] {
            if let Some(reasoning) = delta.get(field).and_then(Value::as_str) {
                entry.reasoning_content.push_str(reasoning);
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                entry.push_tool_call(tool_call);
            }
        }
    }

    fn response_value(self) -> Value {
        let mut choices = self.choices.into_values().collect::<Vec<_>>();
        choices.sort_by_key(|choice| choice.index);
        let choices = choices
            .into_iter()
            .map(OpenAiChatChoiceAggregate::into_value)
            .collect::<Vec<_>>();
        let mut response = serde_json::Map::new();
        response.insert(
            "id".to_string(),
            Value::String(
                self.id
                    .unwrap_or_else(|| "chatcmpl_gateway_aggregated".to_string()),
            ),
        );
        response.insert(
            "object".to_string(),
            Value::String("chat.completion".to_string()),
        );
        response.insert("created".to_string(), json!(self.created.unwrap_or(0)));
        response.insert(
            "model".to_string(),
            Value::String(self.model.unwrap_or_default()),
        );
        response.insert("choices".to_string(), Value::Array(choices));
        if let Some(usage) = self.usage {
            response.insert("usage".to_string(), usage);
        }
        if let Some(system_fingerprint) = self.system_fingerprint {
            response.insert("system_fingerprint".to_string(), system_fingerprint);
        }
        Value::Object(response)
    }
}

impl OpenAiChatChoiceAggregate {
    fn push_tool_call(&mut self, value: &Value) {
        let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
        let entry = self
            .tool_calls
            .entry(index)
            .or_insert_with(|| OpenAiChatToolCallAggregate {
                index,
                ..OpenAiChatToolCallAggregate::default()
            });
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            entry.id = Some(id.to_string());
        }
        if let Some(call_type) = value.get("type").and_then(Value::as_str) {
            entry.call_type = Some(call_type.to_string());
        }
        if let Some(function) = value.get("function").and_then(Value::as_object) {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                entry.function_name = Some(name.to_string());
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                entry.arguments.push_str(arguments);
            }
        }
    }

    fn into_value(self) -> Value {
        let mut message = serde_json::Map::new();
        message.insert(
            "role".to_string(),
            Value::String(self.role.unwrap_or_else(|| "assistant".to_string())),
        );
        if self.content.is_empty() && !self.tool_calls.is_empty() {
            message.insert("content".to_string(), Value::Null);
        } else {
            message.insert("content".to_string(), Value::String(self.content));
        }
        if !self.reasoning_content.is_empty() {
            message.insert(
                "reasoning_content".to_string(),
                Value::String(self.reasoning_content),
            );
        }
        let mut tool_calls = self.tool_calls.into_values().collect::<Vec<_>>();
        tool_calls.sort_by_key(|tool_call| tool_call.index);
        if !tool_calls.is_empty() {
            message.insert(
                "tool_calls".to_string(),
                Value::Array(
                    tool_calls
                        .into_iter()
                        .map(OpenAiChatToolCallAggregate::into_value)
                        .collect(),
                ),
            );
        }
        json!({
            "index": self.index,
            "message": Value::Object(message),
            "finish_reason": self.finish_reason.unwrap_or(Value::Null)
        })
    }
}

impl OpenAiChatToolCallAggregate {
    fn into_value(self) -> Value {
        json!({
            "id": self.id.unwrap_or_else(|| format!("call_gateway_{}", self.index)),
            "type": self.call_type.unwrap_or_else(|| "function".to_string()),
            "function": {
                "name": self.function_name.unwrap_or_default(),
                "arguments": self.arguments
            }
        })
    }
}

#[derive(Debug, Default)]
struct OpenAiResponsesSseAggregate {
    buffer: Vec<u8>,
    response_base: Option<Value>,
    completed_response: Option<Value>,
    output_items: Vec<(i64, Value)>,
}

impl OpenAiResponsesSseAggregate {
    fn push_chunk(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        while let Some((end, delimiter_len)) = find_sse_block_delimiter(&self.buffer) {
            let block = self.buffer.drain(..end + delimiter_len).collect::<Vec<_>>();
            self.push_block(&block);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if !self.buffer.is_empty() {
            let tail = std::mem::take(&mut self.buffer);
            self.push_block(&tail);
        }
        let response = if let Some(response) = self.completed_response {
            response
        } else {
            self.fallback_response()
        };
        serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec())
    }

    fn push_block(&mut self, block: &[u8]) {
        let block_text = String::from_utf8_lossy(block);
        let Some(data) = sse_data_payload(&block_text) else {
            return;
        };
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("response.created") | Some("response.in_progress") => {
                if let Some(response) = value.get("response").cloned() {
                    self.response_base = Some(response);
                }
            }
            Some("response.output_item.done") => {
                if let Some(item) = value.get("item").cloned() {
                    let output_index = value
                        .get("output_index")
                        .and_then(Value::as_i64)
                        .unwrap_or(self.output_items.len() as i64);
                    self.output_items.push((output_index, item));
                }
            }
            Some("response.completed") | Some("response.failed") | Some("response.incomplete") => {
                if let Some(response) = value.get("response").cloned() {
                    self.completed_response = Some(response);
                }
            }
            _ => {}
        }
    }

    fn fallback_response(mut self) -> Value {
        let mut response = self.response_base.take().unwrap_or_else(|| {
            json!({
                "id": "resp_gateway_aggregated",
                "object": "response",
                "status": "completed"
            })
        });
        self.output_items.sort_by_key(|(index, _)| *index);
        let output = self
            .output_items
            .into_iter()
            .map(|(_, item)| item)
            .collect::<Vec<_>>();
        if let Some(object) = response.as_object_mut() {
            object.insert("status".to_string(), Value::String("completed".to_string()));
            object.insert("output".to_string(), Value::Array(output));
        }
        response
    }
}

fn maybe_record_gemini_sse_stream(
    stream: DebugBodyStream,
    context: Option<&GatewayRuntimeContext>,
    provider: &UpstreamProvider,
    request: &DebugHttpRequest,
    upstream_body: &[u8],
) -> DebugBodyStream {
    if provider.target_protocol != AiProtocol::GeminiNative {
        return stream;
    }
    let Some(context) = context else {
        return stream;
    };
    let body = serde_json::from_slice::<Value>(upstream_body).unwrap_or(Value::Null);
    let Some(key) = gemini_shadow_session_key(provider, request, &body) else {
        return stream;
    };
    record_gemini_sse_stream(stream, context.side_stores.gemini_shadow(), key)
}

fn append_lossy_warning_header(
    response_headers: &mut Vec<(String, String)>,
    conversion_context: Option<&ConversionContext>,
) {
    let Some(context) = conversion_context else {
        return;
    };
    if context.lossy_warnings.is_empty() {
        return;
    }
    response_headers.push((
        "X-Transformer-Lossy".to_string(),
        context.lossy_warnings.join(" | "),
    ));
}

fn append_compact_compat_header(
    response_headers: &mut Vec<(String, String)>,
    compact_compat: CodexResponsesCompactCompat,
) {
    if let Some(value) = compact_compat.header_value() {
        response_headers.push((
            CODEX_RESPONSES_COMPACT_COMPAT_HEADER.to_string(),
            value.to_string(),
        ));
    }
}

fn maybe_record_codex_responses_sse_stream(
    stream: DebugBodyStream,
    context: Option<&GatewayRuntimeContext>,
    response_conversion_route: Option<ConversionRoute>,
) -> DebugBodyStream {
    if !response_conversion_route.is_some_and(|route| route.target == AiProtocol::OpenAiResponses) {
        return stream;
    }
    let Some(context) = context else {
        return stream;
    };
    record_responses_sse_stream(stream, context.side_stores.codex_history())
}

fn maybe_filter_bailian_openai_chat_sse_stream(
    stream: DebugBodyStream,
    provider: &UpstreamProvider,
) -> DebugBodyStream {
    if provider.target_protocol != AiProtocol::OpenAiChat {
        return stream;
    }
    if ProviderBodyCompat::from_provider_type(
        provider.meta.provider_type.as_deref(),
        provider.target_protocol,
    ) != Some(ProviderBodyCompat::Bailian)
    {
        return stream;
    }
    filter_bailian_openai_chat_sse_stream(stream)
}

fn filter_bailian_openai_chat_sse_stream(stream: DebugBodyStream) -> DebugBodyStream {
    struct State {
        inner: DebugBodyStream,
        filter: BailianChatStreamFilterState,
        pending: VecDeque<Result<Vec<u8>, String>>,
        finished: bool,
    }

    Box::pin(futures_util::stream::unfold(
        State {
            inner: stream,
            filter: BailianChatStreamFilterState::default(),
            pending: VecDeque::new(),
            finished: false,
        },
        |mut state| async move {
            loop {
                if let Some(chunk) = state.pending.pop_front() {
                    return Some((chunk, state));
                }
                if state.finished {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(chunk)) => {
                        state
                            .pending
                            .extend(state.filter.push_chunk(&chunk).into_iter().map(Ok));
                    }
                    Some(Err(error)) => return Some((Err(error), state)),
                    None => {
                        state.finished = true;
                        state
                            .pending
                            .extend(state.filter.finish().into_iter().map(Ok));
                    }
                }
            }
        },
    ))
}

fn maybe_filter_xai_openai_chat_sse_stream(
    stream: DebugBodyStream,
    provider: &UpstreamProvider,
) -> DebugBodyStream {
    if provider.target_protocol != AiProtocol::OpenAiChat {
        return stream;
    }
    if ProviderBodyCompat::from_provider_type(
        provider.meta.provider_type.as_deref(),
        provider.target_protocol,
    ) != Some(ProviderBodyCompat::Xai)
    {
        return stream;
    }
    filter_xai_openai_chat_sse_stream(stream)
}

fn filter_xai_openai_chat_sse_stream(stream: DebugBodyStream) -> DebugBodyStream {
    struct State {
        inner: DebugBodyStream,
        filter: XaiChatStreamFilterState,
        pending: VecDeque<Result<Vec<u8>, String>>,
        finished: bool,
    }

    Box::pin(futures_util::stream::unfold(
        State {
            inner: stream,
            filter: XaiChatStreamFilterState::default(),
            pending: VecDeque::new(),
            finished: false,
        },
        |mut state| async move {
            loop {
                if let Some(chunk) = state.pending.pop_front() {
                    return Some((chunk, state));
                }
                if state.finished {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(chunk)) => {
                        state
                            .pending
                            .extend(state.filter.push_chunk(&chunk).into_iter().map(Ok));
                    }
                    Some(Err(error)) => return Some((Err(error), state)),
                    None => {
                        state.finished = true;
                        state
                            .pending
                            .extend(state.filter.finish().into_iter().map(Ok));
                    }
                }
            }
        },
    ))
}

#[derive(Debug, Default)]
struct XaiChatStreamFilterState {
    buffer: Vec<u8>,
}

impl XaiChatStreamFilterState {
    fn push_chunk(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some((end, delimiter_len)) = find_sse_block_delimiter(&self.buffer) {
            let block = self.buffer.drain(..end + delimiter_len).collect::<Vec<_>>();
            out.extend(self.filter_block(&block));
        }
        out
    }

    fn finish(&mut self) -> Vec<Vec<u8>> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let tail = std::mem::take(&mut self.buffer);
        self.filter_block(&tail)
    }

    fn filter_block(&self, block: &[u8]) -> Vec<Vec<u8>> {
        let block_text = String::from_utf8_lossy(block);
        let Some(data) = sse_data_payload(&block_text) else {
            return vec![block.to_vec()];
        };
        if data.trim() == "[DONE]" {
            return vec![block.to_vec()];
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return vec![block.to_vec()];
        };
        if is_xai_empty_chat_delta_chunk(&value) {
            return Vec::new();
        }

        vec![block.to_vec()]
    }
}

fn is_xai_empty_chat_delta_chunk(value: &Value) -> bool {
    if value.get("usage").is_some_and(|usage| !usage.is_null()) {
        return false;
    }
    let Some(choices) = value.get("choices").and_then(Value::as_array) else {
        return false;
    };
    if choices.is_empty() {
        return false;
    }

    choices.iter().all(|choice| {
        if choice
            .get("finish_reason")
            .is_some_and(|finish_reason| !finish_reason.is_null())
        {
            return false;
        }
        choice
            .get("delta")
            .and_then(Value::as_object)
            .is_some_and(|delta| delta.is_empty())
    })
}

#[derive(Debug, Default)]
struct BailianChatStreamFilterState {
    buffer: Vec<u8>,
    saw_tool_calls: bool,
    buffered_text: String,
    last_text_choice: i64,
    last_chunk_base: Option<Value>,
    tool_args: HashMap<(i64, i64), String>,
}

impl BailianChatStreamFilterState {
    fn push_chunk(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some((end, delimiter_len)) = find_sse_block_delimiter(&self.buffer) {
            let block = self.buffer.drain(..end + delimiter_len).collect::<Vec<_>>();
            out.extend(self.filter_block(&block));
        }
        out
    }

    fn finish(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if !self.buffer.is_empty() {
            let tail = std::mem::take(&mut self.buffer);
            out.extend(self.filter_block(&tail));
        }
        if !self.buffered_text.is_empty() {
            if let Some(base) = self.last_chunk_base.as_ref() {
                out.push(sse_data_json(&self.build_text_chunk(base)));
            }
            self.buffered_text.clear();
        }
        out
    }

    fn filter_block(&mut self, block: &[u8]) -> Vec<Vec<u8>> {
        let block_text = String::from_utf8_lossy(block);
        let Some(data) = sse_data_payload(&block_text) else {
            return vec![block.to_vec()];
        };
        if data.trim() == "[DONE]" {
            return vec![block.to_vec()];
        }
        let Ok(mut value) = serde_json::from_str::<Value>(&data) else {
            return vec![block.to_vec()];
        };
        let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) else {
            return vec![block.to_vec()];
        };

        for choice in choices.iter_mut() {
            let choice_index = choice.get("index").and_then(Value::as_i64).unwrap_or(0);
            if let Some(tool_calls) = choice
                .get_mut("delta")
                .and_then(Value::as_object_mut)
                .and_then(|delta| delta.get_mut("tool_calls"))
                .and_then(Value::as_array_mut)
            {
                if !tool_calls.is_empty() {
                    self.saw_tool_calls = true;
                    self.filter_tool_calls(choice_index, tool_calls);
                }
            }
        }

        if !self.saw_tool_calls {
            return vec![block.to_vec()];
        }

        let mut has_finish = false;
        for choice in choices.iter_mut() {
            let choice_index = choice.get("index").and_then(Value::as_i64).unwrap_or(0);
            if choice
                .get("finish_reason")
                .is_some_and(|reason| !reason.is_null())
            {
                has_finish = true;
            }
            if let Some(delta) = choice.get_mut("delta").and_then(Value::as_object_mut) {
                let text = extract_bailian_text_delta(delta);
                if !text.is_empty() {
                    self.last_text_choice = choice_index;
                    self.buffered_text.push_str(&text);
                }
            }
        }

        self.last_chunk_base = Some(value.clone());
        if has_finish && !self.buffered_text.is_empty() {
            let text_chunk = self.build_text_chunk(&value);
            self.buffered_text.clear();
            return vec![sse_data_json(&text_chunk), sse_data_json(&value)];
        }

        vec![sse_data_json(&value)]
    }

    fn filter_tool_calls(&mut self, choice_index: i64, tool_calls: &mut [Value]) {
        for (fallback_index, tool_call) in tool_calls.iter_mut().enumerate() {
            let call_index = tool_call
                .get("index")
                .and_then(Value::as_i64)
                .unwrap_or(fallback_index as i64);
            let Some(arguments) = tool_call
                .get_mut("function")
                .and_then(Value::as_object_mut)
                .and_then(|function| function.get_mut("arguments"))
                .and_then(|arguments| arguments.as_str())
                .map(str::to_string)
            else {
                continue;
            };
            if arguments.is_empty() {
                continue;
            }

            let key = (choice_index, call_index);
            let accumulated = self.tool_args.entry(key).or_default();
            if arguments.trim() == "{}" && !accumulated.trim().is_empty() {
                if let Some(function) = tool_call.get_mut("function").and_then(Value::as_object_mut)
                {
                    function.insert("arguments".to_string(), Value::String(String::new()));
                }
                continue;
            }
            accumulated.push_str(&arguments);
        }
    }

    fn build_text_chunk(&self, base: &Value) -> Value {
        let mut chunk = base.clone();
        let text = self.buffered_text.clone();
        chunk["choices"] = json!([{
            "index": self.last_text_choice,
            "delta": {
                "content": text
            }
        }]);
        if let Some(object) = chunk.as_object_mut() {
            object.remove("usage");
        }
        chunk
    }
}

fn find_sse_block_delimiter(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(lf), Some(crlf)) if crlf < lf => Some((crlf, 4)),
        (Some(lf), _) => Some((lf, 2)),
        (None, Some(crlf)) => Some((crlf, 4)),
        (None, None) => None,
    }
}

fn sse_data_payload(block: &str) -> Option<String> {
    let mut lines = Vec::new();
    for line in block.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        lines.push(data.trim_start().to_string());
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn sse_data_json(value: &Value) -> Vec<u8> {
    format!("data: {}\n\n", value).into_bytes()
}

fn extract_bailian_text_delta(delta: &mut serde_json::Map<String, Value>) -> String {
    let mut text = String::new();
    if let Some(content) = delta.remove("content") {
        match content {
            Value::String(value) => text.push_str(&value),
            Value::Array(parts) => {
                let mut kept = Vec::new();
                for part in parts {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                            text.push_str(part_text);
                        }
                    } else {
                        kept.push(part);
                    }
                }
                if !kept.is_empty() {
                    delta.insert("content".to_string(), Value::Array(kept));
                }
            }
            other => {
                delta.insert("content".to_string(), other);
            }
        }
    }
    text
}

fn record_side_store_response(
    context: Option<&GatewayRuntimeContext>,
    provider: &UpstreamProvider,
    request: &DebugHttpRequest,
    upstream_body: &[u8],
    upstream_response_body: &[u8],
    target_response_body: &[u8],
    final_body: &[u8],
    response_conversion_route: Option<ConversionRoute>,
) {
    let Some(context) = context else {
        return;
    };
    if provider.target_protocol == AiProtocol::GeminiNative {
        if let Ok(request_value) = serde_json::from_slice::<Value>(upstream_body) {
            if let Some(key) = gemini_shadow_session_key(provider, request, &request_value) {
                let response_value = serde_json::from_slice::<Value>(upstream_response_body)
                    .or_else(|_| serde_json::from_slice::<Value>(target_response_body));
                if let Ok(response_value) = response_value {
                    context
                        .side_stores
                        .record_gemini_response(key, &response_value);
                }
            }
        }
    }
    if response_conversion_route.is_some_and(|route| route.target == AiProtocol::OpenAiResponses) {
        if let Ok(response_value) = serde_json::from_slice::<Value>(final_body) {
            context.side_stores.record_codex_response(&response_value);
        }
    }
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
    let token_usage = from_response_body_with_provider_type(
        provider.cli_key,
        provider.meta.provider_type.as_deref(),
        &body,
    );
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

fn empty_success_failure_response(
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    requested_model: &str,
    upstream_model_id: &str,
    mut response: DebugHttpResponse,
    attempt_count: u32,
    provider_attempt_count: u32,
    failover: bool,
) -> DebugHttpResponse {
    let mut failure_response = json_response(
        502,
        "Bad Gateway",
        json!({
            "error": "upstream_empty_response",
            "message": "Upstream returned a successful response without meaningful content.",
        }),
        route.route_name,
        response.upstream_url.clone(),
        "upstream returned an empty successful response",
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
    failure_response.attempt_count = attempt_count;
    failure_response.provider_attempt_count = provider_attempt_count;
    failure_response.failover = failover;
    failure_response
}

fn local_request_schema_failure_response(
    route: &GatewayRoute,
    provider: &UpstreamProvider,
    requested_model: &str,
    upstream_model_id: &str,
    error: GatewayForwardError,
    attempt_count: u32,
    provider_attempt_count: u32,
    failover: bool,
) -> DebugHttpResponse {
    let message = error.message.clone();
    let body = CodexResponsesCompactCompat::new(route, provider)
        .request_schema_error_value(&message)
        .unwrap_or_else(|| {
            json!({
                "error": "gateway_request_schema_rejected",
                "message": message,
            })
        });
    let mut response = json_response(
        400,
        "Bad Request",
        body,
        route.route_name,
        None,
        "gateway rejected request before upstream forwarding",
    );
    append_compact_compat_header(
        &mut response.headers,
        CodexResponsesCompactCompat::new(route, provider),
    );
    response.cli_key = Some(route.cli_key);
    response.provider_id = Some(provider.id.clone());
    response.provider_name = Some(provider.name.clone());
    response.provider_type = provider.meta.provider_type.clone();
    response.cost_multiplier = Some(provider.meta.cost_multiplier.clone());
    response.pricing_model_source = Some(provider.meta.pricing_model_source.clone());
    response.requested_model = Some(requested_model.to_string());
    response.upstream_model_id = Some(upstream_model_id.to_string());
    response.upstream_request_body = error.upstream_request_body;
    response.upstream_response_body = error.upstream_response_body;
    response.upstream_response_body_bytes = error.upstream_response_body_bytes;
    response.error_category = Some("request_schema".to_string());
    response.attempt_count = attempt_count;
    response.provider_attempt_count = provider_attempt_count;
    response.failover = failover;
    response
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
    response_is_sse(headers)
        || request_declares_streaming(request)
        || route_declares_streaming(route)
}

fn response_is_sse(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"))
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

fn route_is_openai_responses_compact(route: &GatewayRoute) -> bool {
    super::compat::codex_responses_compact::is_codex_responses_compact_route(route)
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

fn should_attempt_responses_encrypted_content_rectifier(
    enabled: bool,
    target_protocol: AiProtocol,
    request: &DebugHttpRequest,
    route: &GatewayRoute,
    headers: &HeaderMap,
    status_code: u16,
) -> bool {
    enabled
        && target_protocol == AiProtocol::OpenAiResponses
        && !route_is_openai_responses_compact(route)
        && !should_stream_response(request, route, headers, status_code)
        && (400..500).contains(&status_code)
}

fn should_filter_known_invalid_responses_ciphers(
    enabled: bool,
    target_protocol: AiProtocol,
    compact_compat: CodexResponsesCompactCompat,
) -> bool {
    enabled && target_protocol == AiProtocol::OpenAiResponses && !compact_compat.is_compact()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResponsesEncryptedContentError {
    message: Option<String>,
}

fn detect_responses_encrypted_content_error(
    status_code: u16,
    body: &[u8],
) -> Option<ResponsesEncryptedContentError> {
    if !(400..500).contains(&status_code) {
        return None;
    }
    let parsed_body = serde_json::from_slice::<Value>(body).ok();
    let message = parsed_body
        .as_ref()
        .and_then(extract_error_message_from_value)
        .or_else(|| extract_error_message_from_body(body));
    let error_code = parsed_body
        .as_ref()
        .and_then(|value| string_at_error_or_top_level(value, "code"));
    if error_code.is_some_and(|code| code.eq_ignore_ascii_case("invalid_encrypted_content")) {
        return Some(ResponsesEncryptedContentError { message });
    }
    let error_param = parsed_body
        .as_ref()
        .and_then(|value| string_at_error_or_top_level(value, "param"));
    if error_param.is_some_and(|param| param.eq_ignore_ascii_case("encrypted_content")) {
        return Some(ResponsesEncryptedContentError { message });
    }

    let message = message?;
    let lower = message.to_ascii_lowercase();
    let mentions_encrypted_content =
        lower.contains("encrypted content") || lower.contains("encrypted_content");
    let verification_failed = lower.contains("could not be verified");
    let decryption_failed = lower.contains("could not be decrypted")
        || lower.contains("could not be decrypted or parsed");

    (mentions_encrypted_content && (verification_failed || decryption_failed)).then_some(
        ResponsesEncryptedContentError {
            message: Some(message),
        },
    )
}

fn string_at_error_or_top_level<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get("error")
        .and_then(|error| error.get(field))
        .or_else(|| value.get(field))
        .and_then(Value::as_str)
}

#[cfg(test)]
fn should_rectify_responses_encrypted_content(status_code: u16, body: &[u8]) -> bool {
    detect_responses_encrypted_content_error(status_code, body).is_some()
}

fn build_responses_encrypted_content_rectified_body(
    body: &[u8],
) -> Result<Option<Vec<u8>>, GatewayForwardError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return Ok(None);
    };
    let Some(input) = value.get_mut("input").and_then(Value::as_array_mut) else {
        return Ok(None);
    };
    let original_len = input.len();
    input.retain(|item| {
        let is_reasoning = item.get("type").and_then(Value::as_str) == Some("reasoning");
        let has_encrypted_content = item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .is_some_and(|encrypted_content| !encrypted_content.trim().is_empty());
        !(is_reasoning && has_encrypted_content)
    });
    if input.len() == original_len {
        return Ok(None);
    }

    serde_json::to_vec(&value)
        .map(Some)
        .map_err(|error| GatewayForwardError {
            message: format!(
                "Failed to serialize Responses encrypted-content rectified body: {error}"
            ),
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: None,
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })
}

fn strip_known_invalid_responses_ciphers(
    context: &GatewayRuntimeContext,
    provider: &UpstreamProvider,
    body: &mut Vec<u8>,
) -> Result<usize, GatewayForwardError> {
    let provider_config_identity = responses_cipher_provider_config_identity(provider);
    if !context
        .side_stores
        .has_known_invalid_responses_ciphers(provider_config_identity)
    {
        return Ok(0);
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return Ok(0);
    };
    let removed = context
        .side_stores
        .strip_known_invalid_responses_ciphers(provider_config_identity, &mut value);
    if removed == 0 {
        return Ok(0);
    }
    *body = serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!(
            "Failed to serialize Responses body after filtering known invalid encrypted content: {error}"
        ),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })?;
    Ok(removed)
}

fn responses_cipher_provider_config_identity(provider: &UpstreamProvider) -> [u8; 32] {
    fn update_component(hasher: &mut Sha256, value: &str) {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }

    let mut hasher = Sha256::new();
    update_component(&mut hasher, provider.cli_key.as_str());
    update_component(&mut hasher, &provider.id);
    update_component(&mut hasher, &provider.base_url);
    update_component(&mut hasher, provider.target_protocol.as_str());
    update_component(
        &mut hasher,
        if provider.is_full_url {
            "full_url"
        } else {
            "base_url"
        },
    );
    update_component(
        &mut hasher,
        provider.meta.provider_type.as_deref().unwrap_or_default(),
    );
    update_component(
        &mut hasher,
        provider.meta.api_format.as_deref().unwrap_or_default(),
    );
    if let Some(profile) = provider.meta.gateway_profile.as_ref() {
        update_component(&mut hasher, profile.tool.as_deref().unwrap_or_default());
        update_component(&mut hasher, &profile.profile_id);
        update_component(&mut hasher, &profile.endpoint_id);
    }
    update_component(&mut hasher, &provider.api_key);
    hasher.finalize().into()
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
    compact_compat: CodexResponsesCompactCompat,
    mut body: Vec<u8>,
    response_headers: &mut Vec<(String, String)>,
) -> Vec<u8> {
    if compact_compat.is_compact() {
        let converted_error_body = compact_compat.convert_error_response_body(&body);
        if converted_error_body != body {
            body = converted_error_body;
            set_response_content_type(response_headers, "application/json");
        }
    } else if let Some(route) = conversion_route.map(ConversionRoute::reverse) {
        let converted_error_body = convert_error_response_body(route, &body);
        if converted_error_body != body {
            body = converted_error_body;
            set_response_content_type(response_headers, "application/json");
        }
    }
    body
}

fn can_retry_current_provider(
    failure_kind: GatewayFailureKind,
    provider_retry_count: u32,
    per_provider_retry_count: u32,
    retry_count: u32,
    max_retry_count: u32,
) -> bool {
    failure_kind != GatewayFailureKind::Timeout
        && provider_retry_count < per_provider_retry_count
        && retry_count < max_retry_count
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
    apply_failover_model_mapping: bool,
    allow_provider_model_mapping: bool,
) -> String {
    match provider.cli_key {
        GatewayCliKey::Claude => {
            if !apply_failover_model_mapping {
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
        // Codex only rewrites in failover: CLI config already carries the real
        // P0 model in single mode. Without a default model, passthrough.
        GatewayCliKey::Codex => {
            if !apply_failover_model_mapping {
                return strip_one_m_context_marker(requested_model).to_string();
            }
            let resolved_model = provider
                .model_mapping
                .default_model
                .as_deref()
                .unwrap_or(requested_model);
            strip_one_m_context_marker(resolved_model).to_string()
        }
        // Grok takeover always writes model=grok-build into CLI config for both
        // single and failover. Rewrite whenever provider mapping is allowed so
        // the real provider default reaches upstream without restarting CLI.
        // Connectivity tests set provider_override and must keep the pinned model.
        GatewayCliKey::Grok => {
            if !allow_provider_model_mapping {
                return strip_one_m_context_marker(requested_model).to_string();
            }
            let resolved_model = provider
                .model_mapping
                .default_model
                .as_deref()
                .unwrap_or(requested_model);
            strip_one_m_context_marker(resolved_model).to_string()
        }
        GatewayCliKey::Gemini | GatewayCliKey::OpenCode => {
            strip_one_m_context_marker(requested_model).to_string()
        }
    }
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
    if normalized_model.contains("fable") {
        return model_mapping
            .fable_model
            .clone()
            .or_else(|| model_mapping.opus_model.clone())
            .or_else(|| model_mapping.default_model.clone());
    }
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
        None,
        None,
        route_streaming,
        CodexResponsesCompactCompat::none(),
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
    provider_meta: Option<&ProviderGatewayMeta>,
    context: Option<&GatewayRuntimeContext>,
    provider: Option<&UpstreamProvider>,
    route_streaming: bool,
    compact_compat: CodexResponsesCompactCompat,
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
    let pipeline = build_provider_pipeline(
        provider_meta,
        conversion_route,
        target_protocol,
        is_openai_legacy_completion_request(cli_key, &request.path),
    );
    let mut pipeline_context = build_pipeline_context(provider_meta, target_protocol);
    pipeline
        .run_inbound_request(&mut value, &mut pipeline_context)
        .map_err(|error| GatewayForwardError {
            message: format!("Gateway request pipeline inbound failed: {error}"),
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: Some(request.body.clone()),
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })?;
    if compact_compat.should_use_codex_chat_history()
        || conversion_route.is_some_and(should_enrich_codex_history_before_conversion)
    {
        if let Some(context) = context {
            context.side_stores.enrich_codex_request(&mut value);
        }
    }
    let lossy_warnings = if let Some(route) = conversion_route {
        check_lossy_conversion_policy(context, request, route, &value)?
    } else {
        Vec::new()
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
    let (mut upstream_body, mut conversion_context) = if compact_compat.is_compact() {
        compact_compat
            .convert_request_body(&rewritten_body)
            .map_err(|error| GatewayForwardError {
                message: error.to_string(),
                kind: GatewayFailureKind::GatewayParse,
                upstream_request_body: Some(rewritten_body.clone()),
                upstream_response_body: None,
                upstream_response_body_bytes: 0,
            })?
    } else if let Some(route) = conversion_route {
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
    conversion_context.lossy_warnings = lossy_warnings;
    if let Some(warning) = compact_compat.warning() {
        conversion_context.lossy_warnings.push(warning.to_string());
    }
    if target_protocol == AiProtocol::GeminiNative {
        if let (Some(context), Some(provider)) = (context, provider) {
            upstream_body =
                enrich_gemini_upstream_request(context, provider, request, &upstream_body)?;
        }
    }
    if target_protocol == AiProtocol::OpenAiResponses {
        upstream_body = apply_openai_responses_prompt_cache_key_fallback(request, &upstream_body)?;
    }
    let upstream_body = apply_provider_pipeline_outbound_body(
        upstream_body,
        &pipeline,
        &pipeline_context,
        request.body.clone(),
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

fn should_enrich_codex_history_before_conversion(route: ConversionRoute) -> bool {
    route.source == AiProtocol::OpenAiResponses
        && matches!(
            route.target,
            AiProtocol::OpenAiChat | AiProtocol::AnthropicMessages
        )
}

fn build_provider_pipeline(
    provider_meta: Option<&ProviderGatewayMeta>,
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
    skip_outbound_adapter: bool,
) -> Pipeline {
    let mut middleware: Vec<Arc<dyn Middleware>> = vec![
        Arc::new(OutboundAdapterCompatMiddleware {
            conversion_route,
            target_protocol,
            provider_meta: provider_meta.cloned(),
            skip: skip_outbound_adapter,
        }),
        Arc::new(BillingHeaderCchMiddleware),
    ];
    if let Some(default_max_tokens) = provider_meta
        .and_then(|meta| meta.default_max_tokens)
        .filter(|value| *value > 0)
    {
        middleware.push(Arc::new(EnsureMaxTokensMiddleware::new(default_max_tokens)));
    }
    Pipeline::new(middleware)
}

fn build_pipeline_context(
    provider_meta: Option<&ProviderGatewayMeta>,
    target_protocol: AiProtocol,
) -> PipelineContext {
    PipelineContext {
        provider_type: provider_meta.and_then(|meta| meta.provider_type.clone()),
        target_protocol: Some(target_protocol),
        ..PipelineContext::default()
    }
}

fn apply_provider_pipeline_outbound_body(
    body: Vec<u8>,
    pipeline: &Pipeline,
    pipeline_context: &PipelineContext,
    fallback_snapshot: Vec<u8>,
) -> Result<Vec<u8>, GatewayForwardError> {
    let mut value =
        serde_json::from_slice::<Value>(&body).map_err(|error| GatewayForwardError {
            message: format!(
                "Failed to parse upstream request body for provider pipeline: {error}"
            ),
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: Some(body.clone()),
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })?;
    pipeline
        .run_outbound_body(&mut value, pipeline_context)
        .map_err(|error| GatewayForwardError {
            message: format!("Gateway request pipeline outbound failed: {error}"),
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: Some(fallback_snapshot),
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })?;
    serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to serialize provider pipeline request body: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: Some(body),
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })
}

#[derive(Debug, Clone)]
struct OutboundAdapterCompatMiddleware {
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
    provider_meta: Option<ProviderGatewayMeta>,
    skip: bool,
}

impl Middleware for OutboundAdapterCompatMiddleware {
    fn on_outbound_body(&self, body: &mut Value, _ctx: &PipelineContext) -> Result<(), String> {
        if self.skip {
            return Ok(());
        }
        apply_outbound_adapter_compat_value(
            body,
            self.conversion_route,
            self.target_protocol,
            self.provider_meta.as_ref(),
        )
        .map_err(|error| error.message)
    }
}

fn enrich_gemini_upstream_request(
    context: &GatewayRuntimeContext,
    provider: &UpstreamProvider,
    request: &DebugHttpRequest,
    upstream_body: &[u8],
) -> Result<Vec<u8>, GatewayForwardError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(upstream_body) else {
        return Ok(upstream_body.to_vec());
    };
    let Some(key) = gemini_shadow_session_key(provider, request, &value) else {
        return Ok(upstream_body.to_vec());
    };
    let changed = context.side_stores.enrich_gemini_request(&key, &mut value);
    if changed == 0 {
        return Ok(upstream_body.to_vec());
    }
    serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to serialize Gemini shadow enriched request body: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: Some(upstream_body.to_vec()),
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })
}

fn check_lossy_conversion_policy(
    context: Option<&GatewayRuntimeContext>,
    request: &DebugHttpRequest,
    route: ConversionRoute,
    value: &Value,
) -> Result<Vec<String>, GatewayForwardError> {
    let issues = check_lossy_conversion(route, value);
    if issues.is_empty() {
        return Ok(Vec::new());
    }
    let warnings = issues
        .iter()
        .map(|issue| format!("{}: {}", issue.path, issue.message))
        .collect::<Vec<_>>();
    let enabled = context
        .map(|context| context.settings_snapshot().lossy_rejection_enabled)
        .unwrap_or(false);
    if !enabled || request_allows_lossy_conversion(request) {
        return Ok(warnings);
    }
    let message = warnings.join("; ");
    Err(GatewayForwardError {
        message: format!("Lossy protocol conversion rejected: {message}"),
        kind: GatewayFailureKind::RequestSchema,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })
}

fn request_allows_lossy_conversion(request: &DebugHttpRequest) -> bool {
    header_value_ci(&request.headers, "x-allow-lossy").is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes"
        )
    })
}

fn gemini_shadow_session_key(
    provider: &UpstreamProvider,
    request: &DebugHttpRequest,
    body: &Value,
) -> Option<GeminiShadowSessionKey> {
    gateway_session_id_hint(request, body)
        .map(|session_id| GeminiShadowSessionKey::new(provider.id.clone(), session_id))
}

fn gateway_session_id_hint(request: &DebugHttpRequest, body: &Value) -> Option<String> {
    for header_name in [
        "x-ai-toolbox-session-id",
        "x-session-id",
        "x-conversation-id",
        "chatgpt-conversation-id",
        "chatgpt-account-id",
    ] {
        if let Some(value) =
            header_value_ci(&request.headers, header_name).filter(|value| !value.trim().is_empty())
        {
            return Some(value.trim().to_string());
        }
    }
    for pointer in [
        "/metadata/session_id",
        "/metadata/conversation_id",
        "/extra_body/session_id",
        "/previous_response_id",
        "/cachedContent",
    ] {
        if let Some(value) = body
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn apply_openai_responses_prompt_cache_key_fallback(
    request: &DebugHttpRequest,
    body: &[u8],
) -> Result<Vec<u8>, GatewayForwardError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return Ok(body.to_vec());
    };
    let Some(session_id) = gateway_session_id_hint(request, &value) else {
        return Ok(body.to_vec());
    };
    let Value::Object(object) = &mut value else {
        return Ok(body.to_vec());
    };
    let has_prompt_cache_key = object
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    if has_prompt_cache_key {
        return Ok(body.to_vec());
    }
    object.insert("prompt_cache_key".to_string(), Value::String(session_id));
    serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to serialize Responses prompt_cache_key fallback body: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: Some(body.to_vec()),
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })
}

fn header_value_ci<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
fn apply_outbound_adapter_compat(
    body: Vec<u8>,
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
) -> Result<Vec<u8>, GatewayForwardError> {
    apply_outbound_adapter_compat_for_provider(body, conversion_route, target_protocol, None)
}

#[cfg(test)]
fn apply_outbound_adapter_compat_for_provider_type(
    body: Vec<u8>,
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
    provider_type: &str,
) -> Result<Vec<u8>, GatewayForwardError> {
    let meta = ProviderGatewayMeta {
        provider_type: Some(provider_type.to_string()),
        ..ProviderGatewayMeta::default()
    };
    apply_outbound_adapter_compat_for_provider(body, conversion_route, target_protocol, Some(&meta))
}

#[cfg(test)]
fn apply_outbound_adapter_compat_for_provider(
    body: Vec<u8>,
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
    provider_meta: Option<&ProviderGatewayMeta>,
) -> Result<Vec<u8>, GatewayForwardError> {
    let mut value =
        serde_json::from_slice::<Value>(&body).map_err(|error| GatewayForwardError {
            message: format!("Failed to parse upstream request body for outbound adapter: {error}"),
            kind: GatewayFailureKind::GatewayParse,
            upstream_request_body: Some(body.clone()),
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
        })?;
    apply_outbound_adapter_compat_value(
        &mut value,
        conversion_route,
        target_protocol,
        provider_meta,
    )?;

    serde_json::to_vec(&value).map_err(|error| GatewayForwardError {
        message: format!("Failed to serialize outbound adapter request body: {error}"),
        kind: GatewayFailureKind::GatewayParse,
        upstream_request_body: None,
        upstream_response_body: None,
        upstream_response_body_bytes: 0,
    })
}

fn apply_outbound_adapter_compat_value(
    value: &mut Value,
    conversion_route: Option<ConversionRoute>,
    target_protocol: AiProtocol,
    provider_meta: Option<&ProviderGatewayMeta>,
) -> Result<(), GatewayForwardError> {
    filter_private_outbound_fields(value, false);
    let provider_kind = ProviderBodyCompat::from_provider_meta(provider_meta, target_protocol);
    let reasoning_policy = ReasoningFieldPolicy::from_provider_meta(provider_meta);
    let inferred_codex_chat_reasoning;
    let codex_chat_reasoning =
        if let Some(config) = provider_meta.and_then(|meta| meta.codex_chat_reasoning.as_ref()) {
            Some(config)
        } else {
            inferred_codex_chat_reasoning =
                infer_codex_chat_reasoning_config(provider_meta, target_protocol, value);
            inferred_codex_chat_reasoning.as_ref()
        };

    apply_provider_body_compat_before_generic(value, target_protocol, provider_kind);
    if target_protocol == AiProtocol::OpenAiChat {
        apply_codex_chat_reasoning_config(value, codex_chat_reasoning);
    }

    if target_protocol == AiProtocol::OpenAiChat {
        normalize_openai_chat_for_provider_compat(
            value,
            provider_kind,
            should_preserve_chat_reasoning_effort(provider_kind, codex_chat_reasoning),
        );
    }

    if let Some(route) = conversion_route {
        if let Value::Object(object) = value {
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

    apply_provider_body_compat_after_generic(value, target_protocol, provider_kind);
    if target_protocol == AiProtocol::OpenAiChat {
        apply_openai_chat_reasoning_field_policy(value, reasoning_policy);
        apply_deepseek_openai_chat_reasoning_policy_after_generic(value, provider_kind);
    }
    apply_predictive_unsupported_media_policy(value, provider_meta);
    if provider_kind == Some(ProviderBodyCompat::Ollama)
        && target_protocol == AiProtocol::OpenAiChat
    {
        let converted = convert_openai_chat_request_to_ollama_chat(std::mem::take(value));
        *value = converted;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderBodyCompat {
    AnthropicBedrock,
    AnthropicVertex,
    DeepSeek,
    Moonshot,
    Zai,
    Doubao,
    Xai,
    Longcat,
    ModelScope,
    Bailian,
    Mimo,
    OpenRouter,
    GeminiVertex,
    CodexOfficial,
    Copilot,
    Ollama,
}

impl ProviderBodyCompat {
    fn from_provider_meta(
        meta: Option<&ProviderGatewayMeta>,
        target_protocol: AiProtocol,
    ) -> Option<Self> {
        let meta = meta?;
        Self::from_provider_type(meta.provider_type.as_deref(), target_protocol).or_else(|| {
            (target_protocol == AiProtocol::OpenAiChat
                && meta.api_format.as_deref().is_some_and(is_ollama_api_format))
            .then_some(Self::Ollama)
        })
    }

    fn from_provider_type(
        provider_type: Option<&str>,
        target_protocol: AiProtocol,
    ) -> Option<Self> {
        let normalized = provider_type?.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "bedrock" | "anthropic-bedrock" | "aws-bedrock"
                if target_protocol == AiProtocol::AnthropicMessages =>
            {
                Some(Self::AnthropicBedrock)
            }
            "vertex" | "anthropic-vertex" | "claude-vertex"
                if target_protocol == AiProtocol::AnthropicMessages =>
            {
                Some(Self::AnthropicVertex)
            }
            "deepseek" => Some(Self::DeepSeek),
            "moonshot" | "kimi" => Some(Self::Moonshot),
            "zai" | "zhipu" | "glm" | "chatglm" | "bigmodel" | "big-model" => Some(Self::Zai),
            "doubao" | "doubaoseed" | "doubao-seed" | "volces" => Some(Self::Doubao),
            "xai" | "x-ai" | "grok" => Some(Self::Xai),
            "longcat" => Some(Self::Longcat),
            "modelscope" | "model-scope" => Some(Self::ModelScope),
            "bailian" | "dashscope" | "aliyun" => Some(Self::Bailian),
            "mimo" | "xiaomimimo" | "xiaomi-mimo" => Some(Self::Mimo),
            "openrouter" | "open-router" => Some(Self::OpenRouter),
            "codex" | "openai-codex" | "chatgpt-codex" | "codex-official"
                if target_protocol == AiProtocol::OpenAiResponses =>
            {
                Some(Self::CodexOfficial)
            }
            "copilot" | "github-copilot" | "githubcopilot" => Some(Self::Copilot),
            "ollama" | "ollama-chat" | "ollamachat"
                if target_protocol == AiProtocol::OpenAiChat =>
            {
                Some(Self::Ollama)
            }
            "vertex" | "googlevertex" | "google-vertex" | "geminivertex" | "gemini-vertex" => {
                Some(Self::GeminiVertex)
            }
            _ => None,
        }
    }
}

fn is_ollama_api_format(value: &str) -> bool {
    matches!(
        value
            .trim()
            .to_ascii_lowercase()
            .replace(['/', '-'], "_")
            .as_str(),
        "ollama" | "ollama_chat"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnthropicPlatform {
    Direct,
    Bedrock,
    Vertex,
    LongCat,
}

impl AnthropicPlatform {
    fn from_provider(provider: &UpstreamProvider) -> Option<Self> {
        if provider.target_protocol != AiProtocol::AnthropicMessages {
            return None;
        }
        let normalized = provider
            .meta
            .provider_type
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase().replace(['_', ' '], "-"));
        match normalized.as_deref() {
            Some("bedrock" | "anthropic-bedrock" | "aws-bedrock") => Some(Self::Bedrock),
            Some("vertex" | "anthropic-vertex" | "claude-vertex") => Some(Self::Vertex),
            Some("longcat" | "long-cat") => Some(Self::LongCat),
            Some("anthropic" | "claude" | "direct" | "claude-code") => Some(Self::Direct),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReasoningFieldPolicy {
    ReasoningContent,
    Reasoning,
    None,
    All,
}

impl ReasoningFieldPolicy {
    fn from_provider_meta(meta: Option<&ProviderGatewayMeta>) -> Self {
        if let Some(policy) =
            Self::from_meta_value(meta.and_then(|meta| meta.reasoning_field.as_deref()))
        {
            return policy;
        }
        match meta
            .and_then(|meta| meta.provider_type.as_deref())
            .map(|value| {
                value
                    .trim()
                    .to_ascii_lowercase()
                    .replace('_', "")
                    .replace('-', "")
            })
            .as_deref()
        {
            Some("openrouter" | "nanogpt") => Self::Reasoning,
            _ => Self::ReasoningContent,
        }
    }

    fn from_meta_value(value: Option<&str>) -> Option<Self> {
        match value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase().replace('-', "_"))
            .as_deref()
        {
            Some("reasoning") => Some(Self::Reasoning),
            Some("none") => Some(Self::None),
            Some("all") => Some(Self::All),
            Some("content" | "reasoning_content") => Some(Self::ReasoningContent),
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
        AiProtocol::GeminiNative => {
            apply_gemini_provider_body_compat(object, provider_kind);
        }
    }
}

fn apply_gemini_provider_body_compat(
    object: &mut serde_json::Map<String, Value>,
    provider_kind: Option<ProviderBodyCompat>,
) {
    if provider_kind != Some(ProviderBodyCompat::GeminiVertex) {
        return;
    }
    clear_gemini_vertex_function_ids(object);
}

fn clear_gemini_vertex_function_ids(object: &mut serde_json::Map<String, Value>) {
    let Some(contents) = object.get_mut("contents").and_then(Value::as_array_mut) else {
        return;
    };
    for content in contents {
        let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts {
            if let Some(function_call) = part.get_mut("functionCall").and_then(Value::as_object_mut)
            {
                function_call.remove("id");
            }
            if let Some(function_response) = part
                .get_mut("functionResponse")
                .and_then(Value::as_object_mut)
            {
                function_response.remove("id");
            }
        }
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
            apply_doubao_openai_chat_thinking_compat(object);
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
        Some(ProviderBodyCompat::OpenRouter) => {
            apply_openrouter_openai_chat_reasoning_effort(object);
        }
        Some(ProviderBodyCompat::Copilot) => {
            apply_copilot_model_normalization(object);
            strip_copilot_openai_chat_thinking_blocks(object);
            sanitize_copilot_openai_chat_orphan_tool_messages(object);
        }
        Some(
            ProviderBodyCompat::AnthropicBedrock
            | ProviderBodyCompat::AnthropicVertex
            | ProviderBodyCompat::Longcat
            | ProviderBodyCompat::GeminiVertex
            | ProviderBodyCompat::CodexOfficial
            | ProviderBodyCompat::Ollama,
        )
        | None => {}
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
    if provider_kind == Some(ProviderBodyCompat::CodexOfficial) {
        apply_codex_official_responses_body_compat(object);
    }
    if provider_kind == Some(ProviderBodyCompat::Copilot) {
        apply_copilot_model_normalization(object);
        sanitize_copilot_responses_orphan_tool_outputs(object);
        normalize_copilot_responses_function_call_item_ids(object);
    }
    if matches!(
        provider_kind,
        Some(ProviderBodyCompat::Doubao | ProviderBodyCompat::ModelScope)
    ) {
        object.remove("metadata");
    }
}

fn apply_codex_official_responses_body_compat(object: &mut serde_json::Map<String, Value>) {
    object.insert("stream".to_string(), Value::Bool(true));
    object.insert("store".to_string(), Value::Bool(false));
    object.insert("parallel_tool_calls".to_string(), Value::Bool(true));
    object.remove("max_tokens");
    object.remove("max_completion_tokens");
    object.remove("metadata");

    let include_value = object
        .entry("include".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    match include_value {
        Value::Array(items) => {
            if !items
                .iter()
                .any(|item| item.as_str() == Some("reasoning.encrypted_content"))
            {
                items.push(Value::String("reasoning.encrypted_content".to_string()));
            }
        }
        _ => {
            *include_value = json!(["reasoning.encrypted_content"]);
        }
    }

    let reasoning = object
        .entry("reasoning".to_string())
        .or_insert_with(|| json!({}));
    if let Value::Object(reasoning_object) = reasoning {
        reasoning_object
            .entry("summary".to_string())
            .or_insert_with(|| Value::String("auto".to_string()));
    }
}

fn apply_copilot_model_normalization(object: &mut serde_json::Map<String, Value>) {
    let Some(model) = object
        .get("model")
        .and_then(Value::as_str)
        .and_then(normalize_to_copilot_model_id)
    else {
        return;
    };
    object.insert("model".to_string(), Value::String(model));
}

fn normalize_to_copilot_model_id(client_id: &str) -> Option<String> {
    let trimmed = client_id.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 8 || !bytes[..7].eq_ignore_ascii_case(b"claude-") {
        return None;
    }

    let has_one_m_suffix = ends_with_ascii_ci(bytes, b"[1m]") || ends_with_ascii_ci(bytes, b"-1m");
    if trimmed.contains('.') && !has_one_m_suffix {
        return None;
    }

    let (base, has_one_m_suffix) = split_copilot_one_m_suffix(trimmed);
    let stripped = strip_copilot_trailing_date(base);
    let dotted = dashes_to_dot_in_last_copilot_version(stripped);
    if dotted.is_none() && !has_one_m_suffix {
        return None;
    }

    let mut candidate = dotted.unwrap_or_else(|| stripped.to_string());
    if has_one_m_suffix {
        candidate.push_str("-1m");
    }
    (candidate != trimmed).then_some(candidate)
}

fn ends_with_ascii_ci(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len()
        && haystack[haystack.len() - needle.len()..].eq_ignore_ascii_case(needle)
}

fn split_copilot_one_m_suffix(id: &str) -> (&str, bool) {
    let bytes = id.as_bytes();
    if ends_with_ascii_ci(bytes, b"[1m]") {
        return (&id[..bytes.len() - 4], true);
    }
    if ends_with_ascii_ci(bytes, b"-1m") {
        return (&id[..bytes.len() - 3], true);
    }
    (id, false)
}

fn strip_copilot_trailing_date(id: &str) -> &str {
    let Some(last_dash) = id.rfind('-') else {
        return id;
    };
    let suffix = &id[last_dash + 1..];
    if suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        &id[..last_dash]
    } else {
        id
    }
}

fn dashes_to_dot_in_last_copilot_version(id: &str) -> Option<String> {
    let last_dash = id.rfind('-')?;
    let last_segment = &id[last_dash + 1..];
    if last_segment.is_empty() || !last_segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let head = &id[..last_dash];
    let prev_dash = head.rfind('-')?;
    let prev_segment = &head[prev_dash + 1..];
    if prev_segment.is_empty() || !prev_segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("{head}.{last_segment}"))
}

fn strip_copilot_openai_chat_thinking_blocks(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            continue;
        };
        if message_object.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(parts) = message_object
            .get_mut("content")
            .and_then(Value::as_array_mut)
        {
            parts.retain(|part| {
                !matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("thinking" | "redacted_thinking")
                )
            });
            if parts.is_empty() {
                message_object.insert("content".to_string(), Value::String(String::new()));
            }
        }
    }
}

fn sanitize_copilot_openai_chat_orphan_tool_messages(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    let mut previous_assistant_tool_call_ids: HashSet<String> = HashSet::new();
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            previous_assistant_tool_call_ids.clear();
            continue;
        };
        match message_object.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                previous_assistant_tool_call_ids = message_object
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|tool_calls| {
                        tool_calls
                            .iter()
                            .filter_map(|tool_call| tool_call.get("id").and_then(Value::as_str))
                            .filter(|id| !id.trim().is_empty())
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Some("tool") => {
                let tool_call_id = message_object
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if tool_call_id.is_empty()
                    || !previous_assistant_tool_call_ids.contains(&tool_call_id)
                {
                    let content = chat_message_content_to_text(message_object.get("content"));
                    message_object.insert("role".to_string(), Value::String("user".to_string()));
                    message_object.remove("tool_call_id");
                    message_object.insert(
                        "content".to_string(),
                        Value::String(format!("[Tool result for {tool_call_id}]: {content}")),
                    );
                }
                previous_assistant_tool_call_ids.clear();
            }
            _ => {
                previous_assistant_tool_call_ids.clear();
            }
        }
    }
}

fn sanitize_copilot_responses_orphan_tool_outputs(object: &mut serde_json::Map<String, Value>) {
    let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };

    let mut known_call_ids = HashSet::new();
    for item in input.iter() {
        if item.get("type").and_then(Value::as_str) == Some("function_call") {
            if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
                known_call_ids.insert(call_id.to_string());
            }
        }
    }

    for item in input {
        let Some(item_object) = item.as_object_mut() else {
            continue;
        };
        if item_object.get("type").and_then(Value::as_str) != Some("function_call_output") {
            continue;
        }
        let call_id = item_object
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !call_id.is_empty() && known_call_ids.contains(&call_id) {
            continue;
        }
        let output = chat_message_content_to_text(item_object.get("output"));
        item_object.clear();
        item_object.insert("type".to_string(), Value::String("message".to_string()));
        item_object.insert("role".to_string(), Value::String("user".to_string()));
        item_object.insert(
            "content".to_string(),
            json!([{
                "type": "input_text",
                "text": format!("[Tool result for {call_id}]: {output}")
            }]),
        );
    }
}

fn normalize_copilot_responses_function_call_item_ids(object: &mut serde_json::Map<String, Value>) {
    for field in ["input", "output"] {
        let Some(items) = object.get_mut(field).and_then(Value::as_array_mut) else {
            continue;
        };
        for item in items {
            let Some(item_object) = item.as_object_mut() else {
                continue;
            };
            if item_object.get("type").and_then(Value::as_str) != Some("function_call") {
                continue;
            }
            let Some(call_id) = item_object
                .get("call_id")
                .and_then(Value::as_str)
                .filter(|call_id| !call_id.trim().is_empty())
                .map(ToString::to_string)
            else {
                continue;
            };
            item_object.insert("id".to_string(), Value::String(call_id));
            if item_object
                .get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| name.trim().is_empty())
            {
                item_object.insert("name".to_string(), Value::String("function".to_string()));
            }
        }
    }
}

fn chat_message_content_to_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => value
            .as_str()
            .map(ToString::to_string)
            .unwrap_or_else(|| value.to_string()),
        None => String::new(),
    }
}

fn convert_openai_chat_request_to_ollama_chat(value: Value) -> Value {
    let Value::Object(mut object) = value else {
        return value;
    };
    let model = object
        .remove("model")
        .unwrap_or(Value::String(String::new()));
    let stream = object.remove("stream");
    let messages = object
        .remove("messages")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(openai_chat_message_to_ollama_message)
        .collect::<Vec<_>>();

    let mut ollama = serde_json::Map::new();
    ollama.insert("model".to_string(), model);
    ollama.insert("messages".to_string(), Value::Array(messages));
    if let Some(Value::Bool(stream)) = stream {
        ollama.insert("stream".to_string(), Value::Bool(stream));
    } else {
        ollama.insert("stream".to_string(), Value::Bool(false));
    }
    if let Some(options) = ollama_options_from_openai_chat(&mut object) {
        ollama.insert("options".to_string(), options);
    }
    if let Some(format) = ollama_format_from_response_format(object.remove("response_format")) {
        ollama.insert("format".to_string(), format);
    }
    Value::Object(ollama)
}

fn openai_chat_message_to_ollama_message(message: Value) -> Option<Value> {
    let Value::Object(mut message_object) = message else {
        return None;
    };
    let role = message_object
        .remove("role")
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_else(|| "user".to_string());
    let content = message_object.remove("content");
    let mut ollama_message = serde_json::Map::new();
    ollama_message.insert("role".to_string(), Value::String(role));
    ollama_message.insert(
        "content".to_string(),
        Value::String(chat_message_content_to_text(content.as_ref())),
    );
    let images = ollama_images_from_chat_content(content.as_ref());
    if !images.is_empty() {
        ollama_message.insert("images".to_string(), Value::Array(images));
    }
    if let Some(reasoning) = first_reasoning_field_text(&message_object) {
        ollama_message.insert("thinking".to_string(), Value::String(reasoning));
    }
    Some(Value::Object(ollama_message))
}

fn ollama_images_from_chat_content(content: Option<&Value>) -> Vec<Value> {
    let Some(Value::Array(parts)) = content else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| {
            let image_url = part.get("image_url")?;
            let url = match image_url {
                Value::String(url) => url.as_str(),
                Value::Object(object) => object.get("url").and_then(Value::as_str)?,
                _ => return None,
            };
            let trimmed = url.trim();
            if trimmed.is_empty() {
                return None;
            }
            let image = trimmed
                .strip_prefix("data:")
                .and_then(|data_url| data_url.split_once(',').map(|(_, data)| data))
                .unwrap_or(trimmed);
            (!image.trim().is_empty()).then(|| Value::String(image.to_string()))
        })
        .collect()
}

fn ollama_options_from_openai_chat(object: &mut serde_json::Map<String, Value>) -> Option<Value> {
    let mut options = serde_json::Map::new();
    for (source, target) in [
        ("temperature", "temperature"),
        ("top_p", "top_p"),
        ("top_k", "top_k"),
        ("max_tokens", "num_predict"),
        ("max_completion_tokens", "num_predict"),
    ] {
        if let Some(value) = object.remove(source) {
            options.insert(target.to_string(), value);
        }
    }
    if let Some(stop) = object.remove("stop").and_then(ollama_stop_value) {
        options.insert("stop".to_string(), stop);
    }
    (!options.is_empty()).then(|| Value::Object(options))
}

fn ollama_stop_value(value: Value) -> Option<Value> {
    match value {
        Value::String(stop) if !stop.trim().is_empty() => {
            Some(Value::Array(vec![Value::String(stop)]))
        }
        Value::Array(stops) => {
            let stops = stops
                .into_iter()
                .filter(|item| item.as_str().is_some_and(|text| !text.trim().is_empty()))
                .collect::<Vec<_>>();
            (!stops.is_empty()).then(|| Value::Array(stops))
        }
        _ => None,
    }
}

fn ollama_format_from_response_format(value: Option<Value>) -> Option<Value> {
    let Value::Object(mut object) = value? else {
        return None;
    };
    match object.get("type").and_then(Value::as_str) {
        Some("json_object") => Some(Value::String("json".to_string())),
        Some("json_schema") => object
            .remove("json_schema")
            .and_then(|value| value.get("schema").cloned())
            .or_else(|| object.remove("schema")),
        _ => None,
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
    if provider_kind.is_some_and(|kind| kind != ProviderBodyCompat::AnthropicBedrock) {
        filter_anthropic_native_tools(object);
    }
    match provider_kind {
        Some(ProviderBodyCompat::AnthropicBedrock) => {
            let has_web_search = object
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| tools.iter().any(is_anthropic_native_web_search_tool));
            object.insert(
                "anthropic_version".to_string(),
                Value::String("bedrock-2023-05-31".to_string()),
            );
            if has_web_search {
                object.insert(
                    "anthropic_beta".to_string(),
                    json!(["web-search-2025-03-05"]),
                );
            }
            object.remove("model");
            object.remove("stream");
        }
        Some(ProviderBodyCompat::AnthropicVertex) => {
            object.insert(
                "anthropic_version".to_string(),
                Value::String("vertex-2023-10-16".to_string()),
            );
        }
        _ => {}
    }
}

fn filter_anthropic_native_tools(object: &mut serde_json::Map<String, Value>) {
    let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    tools.retain(|tool| !is_anthropic_native_web_search_tool(tool));
    if tools.is_empty() {
        object.remove("tools");
    }
}

fn is_anthropic_native_web_search_tool(tool: &Value) -> bool {
    tool.get("type")
        .and_then(Value::as_str)
        .is_some_and(|tool_type| {
            matches!(
                tool_type,
                "web_search_20250305" | "web_search_2025_03_05" | "web_search"
            )
        })
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
        strip_openai_chat_assistant_reasoning_fields(object);
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

fn apply_doubao_openai_chat_thinking_compat(object: &mut serde_json::Map<String, Value>) {
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

fn apply_openrouter_openai_chat_reasoning_effort(object: &mut serde_json::Map<String, Value>) {
    let Some(effort) = object
        .remove("reasoning_effort")
        .and_then(|value| value.as_str().map(map_openrouter_reasoning_effort))
        .flatten()
    else {
        return;
    };

    match object.get_mut("reasoning").and_then(Value::as_object_mut) {
        Some(reasoning) => {
            reasoning.insert("effort".to_string(), Value::String(effort.to_string()));
        }
        None => {
            object.insert("reasoning".to_string(), json!({ "effort": effort }));
        }
    }
}

fn map_openrouter_reasoning_effort(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "max" | "xhigh" => Some("xhigh"),
        "high" => Some("high"),
        "medium" => Some("medium"),
        "low" => Some("low"),
        "minimal" => Some("minimal"),
        "none" | "off" | "disabled" => Some("none"),
        _ => None,
    }
}

fn apply_predictive_unsupported_media_policy(
    value: &mut Value,
    provider_meta: Option<&ProviderGatewayMeta>,
) {
    let Some(meta) = provider_meta else {
        return;
    };
    if !should_predictively_replace_images(value, meta) {
        return;
    }
    replace_unsupported_image_parts(value);
}

fn should_predictively_replace_images(value: &Value, meta: &ProviderGatewayMeta) -> bool {
    if !value_contains_image_parts(value) {
        return false;
    }

    match normalized_image_input_policy(meta.image_input_policy.as_deref()).as_deref() {
        Some("preserve" | "keep" | "vision" | "multimodal" | "image" | "images") => {
            return false;
        }
        Some("strip" | "replace" | "text_only" | "textonly" | "unsupported") => return true,
        Some("auto") | None => {}
        Some(_) => {}
    }

    let Some(model) = value
        .get("model")
        .and_then(Value::as_str)
        .map(normalize_model_id_for_media_policy)
        .filter(|model| !model.is_empty())
    else {
        return false;
    };

    if model_list_contains(&meta.image_capable_models, &model) {
        return false;
    }
    if model_list_contains(&meta.text_only_models, &model) {
        return true;
    }

    meta.allow_text_only_model_heuristic && known_text_only_model(&model)
}

fn normalized_image_input_policy(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase().replace(['-', ' '], "_"))
}

fn model_list_contains(models: &[String], normalized_model: &str) -> bool {
    models
        .iter()
        .map(|model| normalize_model_id_for_media_policy(model))
        .any(|model| {
            model == normalized_model || model.rsplit('/').next() == Some(normalized_model)
        })
}

fn known_text_only_model(model: &str) -> bool {
    let normalized = normalize_model_id_for_media_policy(model);
    let tail = normalized.rsplit('/').next().unwrap_or(normalized.as_str());

    const EXACT_TAILS: &[&str] = &[
        "ark-code-latest",
        "deepseek-chat",
        "deepseek-reasoner",
        "deepseek-v4-flash",
        "deepseek-v4-pro",
        "glm-5.1",
        "kat-coder",
        "kat-coder-pro",
        "kat-coder-pro v1",
        "kat-coder-pro v2",
        "kat-coder-pro-v1",
        "kat-coder-pro-v2",
        "ling-2.5-1t",
        "longcat-flash-chat",
        "mimo-v2.5-pro",
        "us.deepseek.r1-v1",
    ];
    const TAIL_PREFIXES: &[&str] = &["minimax-m2.7", "qwen3-coder", "step-3.5-flash"];

    EXACT_TAILS.contains(&tail) || TAIL_PREFIXES.iter().any(|prefix| tail.starts_with(prefix))
}

fn normalize_model_id_for_media_policy(model: &str) -> String {
    model
        .trim()
        .trim_end_matches("[1M]")
        .trim_end_matches("[1m]")
        .trim()
        .to_ascii_lowercase()
}

fn value_contains_image_parts(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            if unsupported_image_replacement(value).is_some() {
                return true;
            }
            object.values().any(value_contains_image_parts)
        }
        Value::Array(items) => items.iter().any(value_contains_image_parts),
        _ => false,
    }
}

fn normalize_deepseek_openai_chat_reasoning_history(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            continue;
        };
        if message_object.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if assistant_message_has_tool_calls(message_object) {
            let reasoning_text = first_reasoning_field_text(message_object)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "tool call".to_string());
            message_object.insert(
                "reasoning_content".to_string(),
                Value::String(reasoning_text),
            );
            message_object.remove("reasoning");
        } else {
            message_object.remove("reasoning_content");
            message_object.remove("reasoning");
        }
    }
}

fn strip_openai_chat_assistant_reasoning_fields(object: &mut serde_json::Map<String, Value>) {
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            continue;
        };
        if message_object.get("role").and_then(Value::as_str) == Some("assistant") {
            message_object.remove("reasoning_content");
            message_object.remove("reasoning");
        }
    }
}

fn assistant_message_has_tool_calls(message_object: &serde_json::Map<String, Value>) -> bool {
    message_object
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|tool_calls| !tool_calls.is_empty())
}

fn first_reasoning_field_text(message_object: &serde_json::Map<String, Value>) -> Option<String> {
    for field in ["reasoning_content", "reasoning"] {
        if let Some(text) = message_object
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        {
            return Some(text.to_string());
        }
    }
    for field in ["reasoning_content", "reasoning"] {
        if let Some(text) = message_object.get(field).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

fn apply_openai_chat_reasoning_field_policy(value: &mut Value, policy: ReasoningFieldPolicy) {
    let Value::Object(object) = value else {
        return;
    };
    let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(message_object) = message.as_object_mut() else {
            continue;
        };
        if message_object.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let reasoning_text = first_reasoning_field_text(message_object);
        match policy {
            ReasoningFieldPolicy::ReasoningContent => {
                if let Some(text) = reasoning_text {
                    message_object.insert("reasoning_content".to_string(), Value::String(text));
                }
                message_object.remove("reasoning");
            }
            ReasoningFieldPolicy::Reasoning => {
                message_object.remove("reasoning_content");
                if let Some(text) = reasoning_text {
                    message_object.insert("reasoning".to_string(), Value::String(text));
                }
            }
            ReasoningFieldPolicy::None => {
                message_object.remove("reasoning_content");
                message_object.remove("reasoning");
            }
            ReasoningFieldPolicy::All => {
                if let Some(text) = reasoning_text {
                    message_object
                        .insert("reasoning_content".to_string(), Value::String(text.clone()));
                    message_object.insert("reasoning".to_string(), Value::String(text));
                }
            }
        }
    }
}

fn apply_deepseek_openai_chat_reasoning_policy_after_generic(
    value: &mut Value,
    provider_kind: Option<ProviderBodyCompat>,
) {
    if provider_kind != Some(ProviderBodyCompat::DeepSeek) {
        return;
    }
    let Value::Object(object) = value else {
        return;
    };
    let thinking_disabled = object
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|thinking_type| thinking_type == "disabled");
    if thinking_disabled {
        strip_openai_chat_assistant_reasoning_fields(object);
    } else {
        normalize_deepseek_openai_chat_reasoning_history(object);
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

fn normalize_openai_chat_for_provider_compat(
    value: &mut Value,
    provider_kind: Option<ProviderBodyCompat>,
    preserve_reasoning_effort: bool,
) {
    let Value::Object(object) = value else {
        return;
    };

    for field in ["verbosity", "prompt_cache_key"] {
        object.remove(field);
    }
    if provider_kind != Some(ProviderBodyCompat::DeepSeek) && !preserve_reasoning_effort {
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
            if object.get("role").and_then(Value::as_str) == Some("developer") {
                object.insert("role".to_string(), Value::String("system".to_string()));
            }
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

    *messages = collapse_openai_chat_system_messages_to_head(filtered_messages);
}

fn collapse_openai_chat_system_messages_to_head(messages: Vec<Value>) -> Vec<Value> {
    let mut system_chunks = Vec::new();
    let mut rest = Vec::with_capacity(messages.len());

    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            if let Some(content) = message.get("content").and_then(Value::as_str) {
                if !content.trim().is_empty() {
                    system_chunks.push(content.to_string());
                }
                continue;
            }
        }
        rest.push(message);
    }

    if system_chunks.is_empty() {
        return rest;
    }

    let mut normalized = Vec::with_capacity(rest.len() + 1);
    normalized.push(json!({
        "role": "system",
        "content": system_chunks.join("\n\n")
    }));
    normalized.extend(rest);
    normalized
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
            strip_google_thought_signature_extra_content(tool_call);
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

fn strip_google_thought_signature_extra_content(tool_call: &mut Value) {
    let Some(tool_call_object) = tool_call.as_object_mut() else {
        return;
    };

    for key in ["thought_signature", "thoughtSignature"] {
        if tool_call_object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|signature| !signature.trim().is_empty())
        {
            tool_call_object.remove(key);
        }
    }
    if google_object_has_signature(tool_call_object.get("google")) {
        tool_call_object.remove("google");
    }
    for key in ["extra_content", "extraContent"] {
        if google_extra_content_has_signature(tool_call_object.get(key)) {
            tool_call_object.remove(key);
        }
    }
    for key in ["extra_fields", "extraFields"] {
        let has_signature = tool_call_object
            .get(key)
            .and_then(Value::as_object)
            .and_then(|extra_fields| {
                extra_fields
                    .get("extra_content")
                    .or_else(|| extra_fields.get("extraContent"))
            })
            .is_some_and(|extra_content| google_extra_content_has_signature(Some(extra_content)));
        if has_signature {
            tool_call_object.remove(key);
        }
    }
}

fn google_extra_content_has_signature(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .and_then(|object| object.get("google"))
        .is_some_and(|google| google_object_has_signature(Some(google)))
}

fn google_object_has_signature(value: Option<&Value>) -> bool {
    value.and_then(Value::as_object).is_some_and(|google| {
        google
            .get("thought_signature")
            .or_else(|| google.get("thoughtSignature"))
            .and_then(Value::as_str)
            .is_some_and(|signature| !signature.trim().is_empty())
    })
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

fn apply_codex_chat_reasoning_config(value: &mut Value, config: Option<&CodexChatReasoningMeta>) {
    let Some(config) = config else {
        return;
    };
    let Value::Object(object) = value else {
        return;
    };
    let Some(reasoning_enabled) = codex_chat_reasoning_requested(object) else {
        return;
    };
    let supports_effort = config.supports_effort.unwrap_or(false);
    let supports_thinking = config.supports_thinking.unwrap_or(false) || supports_effort;

    if supports_thinking {
        match normalized_meta_string(config.thinking_param.as_deref())
            .unwrap_or_else(|| "thinking".to_string())
            .as_str()
        {
            "thinking" => {
                object.insert(
                    "thinking".to_string(),
                    json!({ "type": if reasoning_enabled { "enabled" } else { "disabled" } }),
                );
            }
            "enable_thinking" => {
                object.insert(
                    "enable_thinking".to_string(),
                    Value::Bool(reasoning_enabled),
                );
            }
            "reasoning_split" => {
                object.insert(
                    "reasoning_split".to_string(),
                    Value::Bool(reasoning_enabled),
                );
            }
            "none" | "" => {}
            _ => {}
        }
    }

    let effort_param = normalized_meta_string(config.effort_param.as_deref())
        .unwrap_or_else(|| "reasoning_effort".to_string());
    if !reasoning_enabled {
        object.remove("reasoning_effort");
        if effort_param == "reasoning.effort" {
            object.insert("reasoning".to_string(), json!({ "effort": "none" }));
        }
        return;
    }
    if !supports_effort {
        object.remove("reasoning_effort");
        return;
    }
    let Some(effort) = codex_chat_reasoning_effort(object) else {
        return;
    };
    let Some(mapped) = map_codex_chat_reasoning_effort(
        &effort,
        normalized_meta_string(config.effort_value_mode.as_deref()).as_deref(),
    ) else {
        object.remove("reasoning_effort");
        return;
    };
    match effort_param.as_str() {
        "reasoning_effort" => {
            object.insert(
                "reasoning_effort".to_string(),
                Value::String(mapped.to_string()),
            );
            object.remove("reasoning");
        }
        "reasoning.effort" => {
            object.remove("reasoning_effort");
            object.insert("reasoning".to_string(), json!({ "effort": mapped }));
        }
        _ => {
            object.remove("reasoning_effort");
        }
    }
}

fn infer_codex_chat_reasoning_config(
    provider_meta: Option<&ProviderGatewayMeta>,
    target_protocol: AiProtocol,
    value: &Value,
) -> Option<CodexChatReasoningMeta> {
    if target_protocol != AiProtocol::OpenAiChat {
        return None;
    }
    let provider_meta = provider_meta?;
    let object = value.as_object()?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let provider_type = provider_meta
        .provider_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let api_format = provider_meta
        .api_format
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let platform = format!("{provider_type} {api_format}");

    if platform.contains("openrouter") {
        return Some(CodexChatReasoningMeta {
            supports_thinking: Some(false),
            supports_effort: Some(true),
            thinking_param: Some("none".to_string()),
            effort_param: Some("reasoning.effort".to_string()),
            effort_value_mode: Some("openrouter".to_string()),
            output_format: Some("auto".to_string()),
        });
    }

    if platform.contains("siliconflow") {
        return Some(CodexChatReasoningMeta {
            supports_thinking: Some(true),
            supports_effort: Some(false),
            thinking_param: Some("enable_thinking".to_string()),
            effort_param: Some("none".to_string()),
            effort_value_mode: None,
            output_format: Some("reasoning_content".to_string()),
        });
    }

    if platform.contains("deepseek") {
        return Some(CodexChatReasoningMeta {
            supports_thinking: Some(true),
            supports_effort: Some(true),
            thinking_param: Some("thinking".to_string()),
            effort_param: Some("reasoning_effort".to_string()),
            effort_value_mode: Some("deepseek".to_string()),
            output_format: Some("reasoning_content".to_string()),
        });
    }

    if platform.contains("stepfun") {
        return Some(CodexChatReasoningMeta {
            supports_thinking: Some(true),
            supports_effort: Some(model.contains("2603")),
            thinking_param: Some("none".to_string()),
            effort_param: Some("reasoning_effort".to_string()),
            effort_value_mode: Some("low_high".to_string()),
            output_format: Some("reasoning".to_string()),
        });
    }

    if platform.contains("kimi") || platform.contains("moonshot") {
        return Some(thinking_only_codex_chat_reasoning_config(
            "thinking",
            "reasoning_content",
        ));
    }

    if platform.contains("glm")
        || platform.contains("zhipu")
        || platform.contains("z.ai")
        || platform.contains("zai")
    {
        return Some(thinking_only_codex_chat_reasoning_config(
            "thinking",
            "reasoning_content",
        ));
    }

    if platform.contains("qwen") || platform.contains("dashscope") || platform.contains("bailian") {
        return Some(thinking_only_codex_chat_reasoning_config(
            "enable_thinking",
            "reasoning_content",
        ));
    }

    if platform.contains("minimax") {
        return Some(thinking_only_codex_chat_reasoning_config(
            "reasoning_split",
            "reasoning_details",
        ));
    }

    if platform.contains("mimo") {
        return Some(thinking_only_codex_chat_reasoning_config(
            "thinking",
            "reasoning_content",
        ));
    }

    None
}

fn thinking_only_codex_chat_reasoning_config(
    thinking_param: &str,
    output_format: &str,
) -> CodexChatReasoningMeta {
    CodexChatReasoningMeta {
        supports_thinking: Some(true),
        supports_effort: Some(false),
        thinking_param: Some(thinking_param.to_string()),
        effort_param: Some("none".to_string()),
        effort_value_mode: None,
        output_format: Some(output_format.to_string()),
    }
}

fn should_preserve_chat_reasoning_effort(
    provider_kind: Option<ProviderBodyCompat>,
    config: Option<&CodexChatReasoningMeta>,
) -> bool {
    if provider_kind == Some(ProviderBodyCompat::DeepSeek) {
        return true;
    }
    let Some(config) = config else {
        return false;
    };
    config.supports_effort.unwrap_or(false)
        && normalized_meta_string(config.effort_param.as_deref()).as_deref()
            == Some("reasoning_effort")
}

fn codex_chat_reasoning_requested(object: &serde_json::Map<String, Value>) -> Option<bool> {
    if let Some(effort) = codex_chat_reasoning_effort(object) {
        return Some(!is_reasoning_disabled_effort(&effort));
    }
    object.get("reasoning").map(|value| !value.is_null())
}

fn codex_chat_reasoning_effort(object: &serde_json::Map<String, Value>) -> Option<String> {
    object
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .or_else(|| {
            object
                .get("reasoning")
                .and_then(|value| value.get("effort"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalized_meta_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn map_codex_chat_reasoning_effort(effort: &str, mode: Option<&str>) -> Option<&'static str> {
    let effort = effort.trim().to_ascii_lowercase();
    if is_reasoning_disabled_effort(&effort) {
        return None;
    }
    match mode.unwrap_or("passthrough") {
        "deepseek" => match effort.as_str() {
            "max" | "xhigh" => Some("max"),
            _ => Some("high"),
        },
        "low_high" => match effort.as_str() {
            "minimal" | "low" => Some("low"),
            _ => Some("high"),
        },
        "openrouter" => match effort.as_str() {
            "max" | "xhigh" => Some("xhigh"),
            "high" => Some("high"),
            "medium" => Some("medium"),
            "low" => Some("low"),
            "minimal" => Some("minimal"),
            _ => None,
        },
        _ => match effort.as_str() {
            "minimal" => Some("minimal"),
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "xhigh" => Some("xhigh"),
            "max" => Some("max"),
            _ => None,
        },
    }
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
    provider_meta: Option<&ProviderGatewayMeta>,
    context: Option<&GatewayRuntimeContext>,
    provider: Option<&UpstreamProvider>,
    route_streaming: bool,
    compact_compat: CodexResponsesCompactCompat,
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
        provider_meta,
        context,
        provider,
        route_streaming,
        compact_compat,
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
        GatewayCliKey::Codex | GatewayCliKey::Grok => {
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

fn effective_upstream_provider_for_request(
    provider: &UpstreamProvider,
    upstream_model_id: &str,
) -> UpstreamProvider {
    let mut effective = provider.clone();
    if ProviderBodyCompat::from_provider_meta(Some(&provider.meta), provider.target_protocol)
        == Some(ProviderBodyCompat::Copilot)
    {
        effective.target_protocol = if copilot_model_uses_responses_api(upstream_model_id) {
            AiProtocol::OpenAiResponses
        } else {
            AiProtocol::OpenAiChat
        };
    }
    effective
}

fn effective_upstream_model_id_for_request<'a>(
    provider: &UpstreamProvider,
    upstream_model_id: &'a str,
    request: &DebugHttpRequest,
) -> Cow<'a, str> {
    if should_downgrade_copilot_warmup_request(provider, request) {
        Cow::Borrowed(DEFAULT_COPILOT_WARMUP_MODEL)
    } else {
        Cow::Borrowed(upstream_model_id)
    }
}

fn should_downgrade_copilot_warmup_request(
    provider: &UpstreamProvider,
    request: &DebugHttpRequest,
) -> bool {
    if ProviderBodyCompat::from_provider_meta(Some(&provider.meta), provider.target_protocol)
        != Some(ProviderBodyCompat::Copilot)
    {
        return false;
    }
    if header_value_ci(&request.headers, "anthropic-beta").is_none() {
        return false;
    }
    let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
        return false;
    };
    if is_copilot_compact_request(&body) || copilot_initiator_from_body(Some(&body)) != "user" {
        return false;
    }
    body.get("tools")
        .and_then(Value::as_array)
        .is_none_or(|tools| tools.is_empty())
}

fn copilot_model_uses_responses_api(model: &str) -> bool {
    let normalized = strip_one_m_context_marker(model)
        .trim()
        .to_ascii_lowercase();
    let Some(rest) = normalized.strip_prefix("gpt-") else {
        return false;
    };
    let major_digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if major_digits.is_empty() {
        return false;
    }
    let Ok(major) = major_digits.parse::<u32>() else {
        return false;
    };
    major >= 5 && !normalized.starts_with("gpt-5-mini")
}

async fn resolve_copilot_token_for_provider(
    client: &reqwest::Client,
    provider: &UpstreamProvider,
) -> Result<Option<String>, String> {
    if !should_exchange_copilot_access_token(provider) {
        return Ok(None);
    }
    let access_token = normalized_copilot_access_token(provider.api_key.trim());
    let token =
        exchange_copilot_access_token(client, DEFAULT_COPILOT_TOKEN_ENDPOINT, &access_token)
            .await?;
    Ok(Some(token))
}

fn should_exchange_copilot_access_token(provider: &UpstreamProvider) -> bool {
    if ProviderBodyCompat::from_provider_type(
        provider.meta.provider_type.as_deref(),
        provider.target_protocol,
    ) != Some(ProviderBodyCompat::Copilot)
    {
        return false;
    }
    let token = normalized_copilot_access_token(provider.api_key.trim());
    if token.is_empty() {
        return false;
    }
    if provider
        .meta
        .api_key_field
        .as_deref()
        .is_some_and(is_copilot_github_token_field)
    {
        return true;
    }
    looks_like_github_access_token(&token)
}

fn normalized_copilot_access_token(value: &str) -> String {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("token ")
        .or_else(|| trimmed.strip_prefix("Token "))
        .or_else(|| trimmed.strip_prefix("Bearer "))
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn is_copilot_github_token_field(value: &str) -> bool {
    matches!(
        value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "_")
            .as_str(),
        "github_token"
            | "github_access_token"
            | "github_oauth"
            | "github_oauth_token"
            | "copilot_oauth"
            | "copilot_oauth_token"
            | "copilot_github_token"
    )
}

fn looks_like_github_access_token(value: &str) -> bool {
    let trimmed = value.trim();
    ["ghp_", "github_pat_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

async fn exchange_copilot_access_token(
    client: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
) -> Result<String, String> {
    let cache_key = copilot_token_cache_key(endpoint, access_token);
    if let Some(token) = cached_copilot_token(&cache_key) {
        return Ok(token);
    }

    let response = client
        .get(endpoint)
        .header(AUTHORIZATION, format!("token {access_token}"))
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format!("Copilot token exchange request failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Copilot token exchange returned non-success status: {status}"
        ));
    }

    let body = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Failed to parse Copilot token exchange response: {error}"))?;
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Copilot token exchange response did not contain token".to_string())?
        .to_string();
    let expires_at = body
        .get("expires_at")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "Copilot token exchange response did not contain expires_at".to_string())?;

    cache_copilot_token(cache_key, token.clone(), expires_at);
    Ok(token)
}

fn cached_copilot_token(cache_key: &str) -> Option<String> {
    let now = unix_timestamp_secs();
    let cache = COPILOT_TOKEN_CACHE.lock().ok()?;
    cache.get(cache_key).and_then(|entry| {
        if entry.expires_at > now.saturating_add(COPILOT_TOKEN_EXPIRY_BUFFER_SECS) {
            Some(entry.token.clone())
        } else {
            None
        }
    })
}

fn cache_copilot_token(cache_key: String, token: String, expires_at: i64) {
    if let Ok(mut cache) = COPILOT_TOKEN_CACHE.lock() {
        cache.insert(cache_key, CopilotTokenCacheEntry { token, expires_at });
    }
}

fn copilot_token_cache_key(endpoint: &str, access_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(endpoint.as_bytes());
    hasher.update(b"\0");
    hasher.update(access_token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

fn is_openai_legacy_completion_path(path: &str) -> bool {
    path == "/v1/completions" || path == "/completions"
}

fn is_openai_legacy_completion_request(cli_key: GatewayCliKey, request_target: &str) -> bool {
    if cli_key != GatewayCliKey::Codex {
        return false;
    }
    if let Some(route) = match_gateway_route(request_target) {
        return route.cli_key == GatewayCliKey::Codex
            && is_openai_legacy_completion_path(&route.forwarded_path);
    }
    let (path, _) = split_request_target(request_target);
    is_openai_legacy_completion_path(&path)
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
    if ProviderBodyCompat::from_provider_meta(Some(&provider.meta), provider.target_protocol)
        == Some(ProviderBodyCompat::Copilot)
    {
        return copilot_forwarded_path(route, provider);
    }

    if conversion_route.is_none() {
        if route.cli_key == GatewayCliKey::Gemini {
            return gemini_forwarded_path_for_provider(&route.forwarded_path, provider);
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
            &provider.base_url,
        )),
    }
}

fn copilot_forwarded_path<'a>(
    route: &'a GatewayRoute,
    provider: &'a UpstreamProvider,
) -> Cow<'a, str> {
    match provider.target_protocol {
        AiProtocol::OpenAiResponses => {
            if route_is_openai_responses_compact(route) {
                Cow::Borrowed("/responses/compact")
            } else {
                Cow::Borrowed("/responses")
            }
        }
        AiProtocol::OpenAiChat => Cow::Borrowed("/chat/completions"),
        _ => Cow::Borrowed(&route.forwarded_path),
    }
}

fn build_provider_target_url(
    provider: &UpstreamProvider,
    forwarded_path: &str,
    route_query: Option<&str>,
    conversion_route: Option<ConversionRoute>,
    target_streaming: bool,
    upstream_model_id: &str,
) -> Result<reqwest::Url, String> {
    if provider.is_full_url {
        let query = converted_route_query(route_query, conversion_route, target_streaming);
        return build_full_target_url(&provider.base_url, query.as_deref());
    }

    if is_deepseek_legacy_completion_forward(provider, forwarded_path) {
        return build_deepseek_completion_beta_url(&provider.base_url, route_query);
    }

    if ProviderBodyCompat::from_provider_meta(Some(&provider.meta), provider.target_protocol)
        == Some(ProviderBodyCompat::Ollama)
    {
        return build_ollama_chat_url(&provider.base_url, route_query);
    }

    if provider.target_protocol == AiProtocol::AnthropicMessages {
        if let Some(path) =
            anthropic_platform_forwarded_path(provider, upstream_model_id, target_streaming)
        {
            return build_target_url(&provider.base_url, &path, None);
        }
    }

    let query = if conversion_route.is_some() {
        converted_route_query(route_query, conversion_route, target_streaming)
    } else {
        route_query.map(str::to_string)
    };
    build_target_url(&provider.base_url, forwarded_path, query.as_deref())
}

fn is_deepseek_legacy_completion_forward(
    provider: &UpstreamProvider,
    forwarded_path: &str,
) -> bool {
    is_openai_legacy_completion_path(forwarded_path)
        && ProviderBodyCompat::from_provider_type(
            provider.meta.provider_type.as_deref(),
            provider.target_protocol,
        ) == Some(ProviderBodyCompat::DeepSeek)
}

fn build_deepseek_completion_beta_url(
    base_url: &str,
    route_query: Option<&str>,
) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|error| format!("Invalid upstream base URL '{}': {error}", base_url))?;
    let base_path = url.path().trim_end_matches('/');
    let base_path = base_path.strip_suffix("/v1").unwrap_or(base_path);
    let mut combined_path = String::new();
    combined_path.push_str(base_path.trim_end_matches('/'));
    combined_path.push_str("/beta/completions");
    if !combined_path.starts_with('/') {
        combined_path.insert(0, '/');
    }
    url.set_path(&combined_path);
    url.set_query(route_query);
    Ok(url)
}

fn build_ollama_chat_url(
    base_url: &str,
    route_query: Option<&str>,
) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|error| format!("Invalid upstream base URL '{}': {error}", base_url))?;
    let base_path = url.path().trim_end_matches('/');
    let base_path = base_path.strip_suffix("/v1").unwrap_or(base_path);
    let mut combined_path = String::new();
    combined_path.push_str(base_path.trim_end_matches('/'));
    combined_path.push_str("/api/chat");
    if !combined_path.starts_with('/') {
        combined_path.insert(0, '/');
    }
    url.set_path(&combined_path);
    url.set_query(route_query);
    Ok(url)
}

fn anthropic_platform_forwarded_path(
    provider: &UpstreamProvider,
    upstream_model_id: &str,
    target_streaming: bool,
) -> Option<String> {
    match AnthropicPlatform::from_provider(provider)? {
        AnthropicPlatform::Bedrock => {
            let model = upstream_model_id.trim().trim_start_matches("models/");
            let action = if target_streaming {
                "invoke-with-response-stream"
            } else {
                "invoke"
            };
            Some(format!("/model/{model}/{action}"))
        }
        AnthropicPlatform::Vertex => {
            let model = upstream_model_id.trim().trim_start_matches("models/");
            let action = if target_streaming {
                "streamRawPredict"
            } else {
                "rawPredict"
            };
            Some(format!("/publishers/anthropic/models/{model}:{action}"))
        }
        AnthropicPlatform::Direct | AnthropicPlatform::LongCat => None,
    }
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

fn gemini_forwarded_path_for_provider<'a>(
    path: &'a str,
    provider: &UpstreamProvider,
) -> Cow<'a, str> {
    let stripped_path = strip_one_m_context_marker_from_gemini_path(path);
    let api_version = gemini_api_version_from_base_url(&provider.base_url);
    rewrite_gemini_api_version(stripped_path, &api_version)
}

fn gemini_native_forwarded_path(
    model: &str,
    target_streaming: bool,
    provider_base_url: &str,
) -> String {
    let model = strip_one_m_context_marker(model)
        .trim()
        .strip_prefix("models/")
        .unwrap_or_else(|| strip_one_m_context_marker(model).trim());
    let action = if target_streaming {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    let api_version = gemini_api_version_from_base_url(provider_base_url);
    format!("/{api_version}/models/{model}:{action}")
}

fn gemini_api_version_from_base_url(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| {
            url.path_segments().and_then(|segments| {
                segments
                    .filter(|segment| {
                        matches!(
                            segment.to_ascii_lowercase().as_str(),
                            "v1" | "v1beta" | "v1alpha"
                        )
                    })
                    .last()
                    .map(str::to_string)
            })
        })
        .unwrap_or_else(|| "v1beta".to_string())
}

fn rewrite_gemini_api_version<'a>(path: Cow<'a, str>, api_version: &str) -> Cow<'a, str> {
    let Some((version_start, version_end)) = gemini_api_version_segment_bounds(&path) else {
        return path;
    };
    if &path[version_start..version_end] == api_version {
        return path;
    }

    let mut rewritten_path =
        String::with_capacity(path.len() + api_version.len() - (version_end - version_start));
    rewritten_path.push_str(&path[..version_start]);
    rewritten_path.push_str(api_version);
    rewritten_path.push_str(&path[version_end..]);
    Cow::Owned(rewritten_path)
}

fn gemini_api_version_segment_bounds(path: &str) -> Option<(usize, usize)> {
    let rest = path.strip_prefix('/')?;
    let version_len = rest.find('/').unwrap_or(rest.len());
    let version = &rest[..version_len];
    if !matches!(
        version.to_ascii_lowercase().as_str(),
        "v1" | "v1beta" | "v1alpha"
    ) {
        return None;
    }
    Some((1, 1 + version_len))
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
    upstream_body: Option<&[u8]>,
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
    inject_anthropic_platform_headers(provider, upstream_body, &mut headers, &mut preserved)?;
    inject_codex_official_headers(request, provider, &mut headers, &mut preserved)?;
    inject_copilot_headers(
        request,
        provider,
        upstream_body,
        &mut headers,
        &mut preserved,
    )?;
    Ok(UpstreamHeaders {
        map: headers,
        preserved,
    })
}

fn inject_codex_official_headers(
    request: &DebugHttpRequest,
    provider: &UpstreamProvider,
    headers: &mut HeaderMap,
    preserved: &mut Vec<PreservedHeader>,
) -> Result<(), String> {
    if ProviderBodyCompat::from_provider_type(
        provider.meta.provider_type.as_deref(),
        provider.target_protocol,
    ) != Some(ProviderBodyCompat::CodexOfficial)
    {
        return Ok(());
    }

    append_preserved_header(
        headers,
        preserved,
        ACCEPT.as_str(),
        HeaderValue::from_static("text/event-stream"),
    )?;

    if !headers.contains_key("originator") {
        append_preserved_header(
            headers,
            preserved,
            "Originator",
            HeaderValue::from_static("ai-toolbox"),
        )?;
    }

    if !headers.contains_key("session_id") {
        if let Some(session_id) = codex_session_id_from_request_headers(request) {
            let value = HeaderValue::from_str(&session_id)
                .map_err(|error| format!("Invalid Codex Session_id header value: {error}"))?;
            append_preserved_header(headers, preserved, "Session_id", value)?;
        }
    }

    Ok(())
}

fn codex_session_id_from_request_headers(request: &DebugHttpRequest) -> Option<String> {
    header_value_ci(&request.headers, "session_id")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            header_value_ci(&request.headers, "x-codex-turn-metadata")
                .and_then(extract_codex_session_id_from_turn_metadata)
        })
}

fn extract_codex_session_id_from_turn_metadata(raw: &str) -> Option<String> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
}

fn inject_copilot_headers(
    request: &DebugHttpRequest,
    provider: &UpstreamProvider,
    upstream_body: Option<&[u8]>,
    headers: &mut HeaderMap,
    preserved: &mut Vec<PreservedHeader>,
) -> Result<(), String> {
    if ProviderBodyCompat::from_provider_type(
        provider.meta.provider_type.as_deref(),
        provider.target_protocol,
    ) != Some(ProviderBodyCompat::Copilot)
    {
        return Ok(());
    }

    for name in COPILOT_MANAGED_HEADERS {
        remove_header_ci(headers, preserved, name);
    }

    append_preserved_header(
        headers,
        preserved,
        "Editor-Version",
        HeaderValue::from_static("vscode/1.95.0"),
    )?;
    append_preserved_header(
        headers,
        preserved,
        "Editor-Plugin-Version",
        HeaderValue::from_static("copilot-chat/0.38.2"),
    )?;
    append_preserved_header(
        headers,
        preserved,
        "User-Agent",
        HeaderValue::from_static("GitHubCopilotChat/0.38.2"),
    )?;
    append_preserved_header(
        headers,
        preserved,
        "Copilot-Integration-Id",
        HeaderValue::from_static("vscode-chat"),
    )?;
    append_preserved_header(
        headers,
        preserved,
        "Openai-Intent",
        HeaderValue::from_static("conversation-edits"),
    )?;
    append_preserved_header(
        headers,
        preserved,
        "X-Github-Api-Version",
        HeaderValue::from_static("2025-10-01"),
    )?;
    append_preserved_header(
        headers,
        preserved,
        "X-Vscode-User-Agent-Library-Version",
        HeaderValue::from_static("electron-fetch"),
    )?;

    let body = upstream_body.and_then(|body| serde_json::from_slice::<Value>(body).ok());
    if body.as_ref().is_some_and(copilot_body_has_vision_content) {
        append_preserved_header(
            headers,
            preserved,
            "Copilot-Vision-Request",
            HeaderValue::from_static("true"),
        )?;
    }

    let initiator = body
        .as_ref()
        .map(|body| copilot_initiator_from_body(Some(body)))
        .or_else(|| {
            header_value_ci(&request.headers, "x-initiator").and_then(normalize_copilot_initiator)
        })
        .unwrap_or("user");
    append_preserved_header(
        headers,
        preserved,
        "X-Initiator",
        HeaderValue::from_str(initiator)
            .map_err(|error| format!("Invalid Copilot X-Initiator header value: {error}"))?,
    )?;

    if body.as_ref().is_some_and(detect_copilot_subagent) {
        append_preserved_header(
            headers,
            preserved,
            "X-Interaction-Type",
            HeaderValue::from_static("conversation-subagent"),
        )?;
    }

    if let Some(body) = body.as_ref() {
        if let Some(session_id) = gateway_session_id_hint(request, body) {
            if let Some(interaction_id) = deterministic_copilot_interaction_id(&session_id) {
                append_preserved_header(
                    headers,
                    preserved,
                    "X-Interaction-Id",
                    HeaderValue::from_str(&interaction_id).map_err(|error| {
                        format!("Invalid Copilot X-Interaction-Id header value: {error}")
                    })?,
                )?;
            }
            let request_id = deterministic_copilot_request_id(body, &session_id);
            let request_value = HeaderValue::from_str(&request_id)
                .map_err(|error| format!("Invalid Copilot X-Request-Id header value: {error}"))?;
            append_preserved_header(headers, preserved, "X-Request-Id", request_value.clone())?;
            append_preserved_header(headers, preserved, "X-Agent-Task-Id", request_value)?;
        }
    }

    Ok(())
}

const COPILOT_MANAGED_HEADERS: &[&str] = &[
    "user-agent",
    "editor-version",
    "editor-plugin-version",
    "copilot-integration-id",
    "x-github-api-version",
    "openai-intent",
    "x-initiator",
    "x-interaction-type",
    "x-interaction-id",
    "x-vscode-user-agent-library-version",
    "x-request-id",
    "x-agent-task-id",
    "copilot-vision-request",
];

fn remove_header_ci(headers: &mut HeaderMap, preserved: &mut Vec<PreservedHeader>, name: &str) {
    if let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) {
        headers.remove(header_name);
    }
    preserved.retain(|header| !header.name.eq_ignore_ascii_case(name));
}

fn normalize_copilot_initiator(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "user" => Some("user"),
        "agent" => Some("agent"),
        _ => None,
    }
}

fn copilot_initiator_from_body(body: Option<&Value>) -> &'static str {
    let Some(body) = body else {
        return "user";
    };
    if detect_copilot_subagent(body) || is_copilot_compact_request(body) {
        return "agent";
    }
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        return copilot_initiator_from_chat_messages(messages);
    }
    if let Some(input) = body.get("input").and_then(Value::as_array) {
        return copilot_initiator_from_responses_input(input);
    }
    "user"
}

fn copilot_initiator_from_chat_messages(messages: &[Value]) -> &'static str {
    let Some(last_message) = messages.last() else {
        return "user";
    };
    match last_message.get("role").and_then(Value::as_str) {
        Some("tool") => "agent",
        Some("user") => {
            if chat_message_content_contains_tool_result_marker(last_message.get("content")) {
                "agent"
            } else {
                "user"
            }
        }
        Some(_) => "agent",
        None => "user",
    }
}

fn copilot_initiator_from_responses_input(input: &[Value]) -> &'static str {
    let Some(last_item) = input.last() else {
        return "user";
    };
    match last_item.get("type").and_then(Value::as_str) {
        Some("function_call_output" | "tool_result") => "agent",
        Some("message") => {
            if last_item.get("role").and_then(Value::as_str) == Some("user") {
                "user"
            } else {
                "agent"
            }
        }
        Some(_) => "agent",
        None => "user",
    }
}

fn chat_message_content_contains_tool_result_marker(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(text)) => text.contains("[Tool result for "),
        Some(Value::Array(parts)) => parts.iter().any(|part| {
            part.get("type").and_then(Value::as_str) == Some("tool_result")
                || part
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains("[Tool result for "))
        }),
        _ => false,
    }
}

fn is_copilot_compact_request(body: &Value) -> bool {
    let system_text = extract_copilot_system_text(body);
    if system_text
        .starts_with("You are a helpful AI assistant tasked with summarizing conversations")
    {
        return true;
    }
    let last_text = body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.last())
        .map(extract_copilot_message_text)
        .or_else(|| {
            body.get("input")
                .and_then(Value::as_array)
                .and_then(|items| items.last())
                .map(extract_copilot_message_text)
        })
        .unwrap_or_default();
    last_text.contains("CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.")
        || (last_text.contains("Pending Tasks:") && last_text.contains("Current Work:"))
}

fn detect_copilot_subagent(body: &Value) -> bool {
    if extract_copilot_system_text(body).contains("__SUBAGENT_MARKER__") {
        return true;
    }
    if body
        .pointer("/metadata/user_id")
        .and_then(Value::as_str)
        .is_some_and(|user_id| user_id.contains("_agent_"))
    {
        return true;
    }
    for field in ["messages", "input"] {
        let Some(items) = body.get(field).and_then(Value::as_array) else {
            continue;
        };
        if items
            .iter()
            .filter(|item| item.get("role").and_then(Value::as_str) == Some("user"))
            .any(|item| extract_copilot_message_text(item).contains("__SUBAGENT_MARKER__"))
        {
            return true;
        }
    }
    false
}

fn extract_copilot_system_text(body: &Value) -> String {
    match body.get("system").or_else(|| body.get("instructions")) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn extract_copilot_message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn copilot_body_has_vision_content(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("image_url")
                || object.get("image_url").is_some()
                || object
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|item_type| item_type == "input_image")
            {
                return true;
            }
            if object
                .get("url")
                .or_else(|| object.get("image_url"))
                .or_else(|| object.get("file_id"))
                .and_then(Value::as_str)
                .is_some_and(|text| text.starts_with("data:image/"))
            {
                return true;
            }
            object.values().any(copilot_body_has_vision_content)
        }
        Value::Array(items) => items.iter().any(copilot_body_has_vision_content),
        Value::String(text) => text.starts_with("data:image/"),
        _ => false,
    }
}

fn deterministic_copilot_interaction_id(session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"interaction:");
    hasher.update(session_id.as_bytes());
    Some(uuid_from_sha256(hasher.finalize().as_slice()))
}

fn deterministic_copilot_request_id(body: &Value, session_id: &str) -> String {
    if let Some(last_user_content) = find_last_copilot_user_content(body) {
        let mut hasher = Sha256::new();
        hasher.update(session_id.as_bytes());
        hasher.update(last_user_content.as_bytes());
        uuid_from_sha256(hasher.finalize().as_slice())
    } else {
        Uuid::new_v4().to_string()
    }
}

fn uuid_from_sha256(digest: &[u8]) -> String {
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn find_last_copilot_user_content(body: &Value) -> Option<String> {
    for field in ["messages", "input"] {
        let Some(items) = body.get(field).and_then(Value::as_array) else {
            continue;
        };
        for item in items.iter().rev() {
            if item.get("role").and_then(Value::as_str) != Some("user") {
                continue;
            }
            let Some(content) = item.get("content") else {
                continue;
            };
            if let Some(text) = content.as_str() {
                return Some(text.to_string());
            }
            if let Some(parts) = content.as_array() {
                let filtered = parts
                    .iter()
                    .filter(|part| part.get("type").and_then(Value::as_str) != Some("tool_result"))
                    .map(|part| {
                        let mut part = part.clone();
                        if let Some(object) = part.as_object_mut() {
                            object.remove("cache_control");
                        }
                        part
                    })
                    .collect::<Vec<_>>();
                if !filtered.is_empty() {
                    return serde_json::to_string(&filtered).ok();
                }
            }
        }
    }
    None
}

fn inject_anthropic_platform_headers(
    provider: &UpstreamProvider,
    upstream_body: Option<&[u8]>,
    headers: &mut HeaderMap,
    preserved: &mut Vec<PreservedHeader>,
) -> Result<(), String> {
    if provider.target_protocol != AiProtocol::AnthropicMessages
        || AnthropicPlatform::from_provider(provider)
            .is_some_and(|platform| !matches!(platform, AnthropicPlatform::Direct))
    {
        return Ok(());
    }
    if !upstream_body.is_some_and(anthropic_body_contains_native_web_search_tool) {
        return Ok(());
    }
    append_preserved_header(
        headers,
        preserved,
        "anthropic-beta",
        HeaderValue::from_static("web-search-2025-03-05"),
    )
}

fn anthropic_body_contains_native_web_search_tool(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("tools").and_then(Value::as_array).cloned())
        .is_some_and(|tools| tools.iter().any(is_anthropic_native_web_search_tool))
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
        let anthropic_version = anthropic_header_version(provider);
        append_preserved_header(
            headers,
            preserved,
            "anthropic-version",
            HeaderValue::from_static(anthropic_version),
        )?;
    }
    Ok(())
}

fn anthropic_header_version(provider: &UpstreamProvider) -> &'static str {
    match AnthropicPlatform::from_provider(provider) {
        Some(AnthropicPlatform::Bedrock) => "bedrock-2023-05-31",
        Some(AnthropicPlatform::Vertex) => "vertex-2023-10-16",
        _ => "2023-06-01",
    }
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

fn classify_empty_success_response(response: &DebugHttpResponse) -> Option<GatewayFailureKind> {
    if response.is_streaming || !(200..400).contains(&response.status_code) {
        return None;
    }
    (!response_body_has_meaningful_content(&response.body))
        .then_some(GatewayFailureKind::EmptyResponse)
}

fn response_body_has_meaningful_content(body: &[u8]) -> bool {
    if body.iter().all(u8::is_ascii_whitespace) {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return true;
    };
    json_value_has_meaningful_content(&value)
}

fn json_value_has_meaningful_content(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(json_value_has_meaningful_content),
        Value::Object(object) => {
            if object.get("error").is_some() {
                return true;
            }
            if let Some(choices) = object.get("choices").and_then(Value::as_array) {
                return choices.iter().any(choice_has_meaningful_content);
            }
            if let Some(output) = object.get("output").and_then(Value::as_array) {
                return output.iter().any(output_item_has_meaningful_content);
            }
            if let Some(content) = object.get("content").and_then(Value::as_array) {
                return content.iter().any(content_part_has_meaningful_content);
            }
            if let Some(candidates) = object.get("candidates").and_then(Value::as_array) {
                return candidates.iter().any(candidate_has_meaningful_content);
            }
            object.values().any(json_value_has_meaningful_content)
        }
    }
}

fn choice_has_meaningful_content(choice: &Value) -> bool {
    let message = choice
        .get("message")
        .or_else(|| choice.get("delta"))
        .unwrap_or(&Value::Null);
    message_has_meaningful_content(message)
}

fn message_has_meaningful_content(message: &Value) -> bool {
    if let Some(content) = message.get("content") {
        match content {
            Value::String(text) if !text.trim().is_empty() => return true,
            Value::Array(parts) if parts.iter().any(content_part_has_meaningful_content) => {
                return true;
            }
            _ => {}
        }
    }
    extract_reasoning_like_text(message).is_some()
        || message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tool_calls| !tool_calls.is_empty())
        || message.get("function_call").is_some()
        || message
            .get("refusal")
            .and_then(Value::as_str)
            .is_some_and(|refusal| !refusal.trim().is_empty())
}

fn output_item_has_meaningful_content(item: &Value) -> bool {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => item
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|content| content.iter().any(content_part_has_meaningful_content)),
        Some("reasoning") => {
            item.get("summary")
                .and_then(Value::as_array)
                .is_some_and(|summary| summary.iter().any(content_part_has_meaningful_content))
                || item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| !content.trim().is_empty())
        }
        Some("function_call") | Some("custom_tool_call") | Some("tool_search_call") => true,
        _ => json_value_has_meaningful_content(item),
    }
}

fn content_part_has_meaningful_content(part: &Value) -> bool {
    if let Some(text) = part
        .get("text")
        .or_else(|| part.get("content"))
        .or_else(|| part.get("thinking"))
        .or_else(|| part.get("delta"))
        .and_then(Value::as_str)
    {
        return !text.trim().is_empty();
    }
    matches!(
        part.get("type").and_then(Value::as_str),
        Some(
            "tool_use"
                | "function_call"
                | "custom_tool_call"
                | "image"
                | "input_image"
                | "output_image"
                | "document"
                | "refusal"
        )
    ) || part.get("functionCall").is_some()
        || part.get("inlineData").is_some()
        || part.get("fileData").is_some()
}

fn candidate_has_meaningful_content(candidate: &Value) -> bool {
    candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .is_some_and(|parts| parts.iter().any(content_part_has_meaningful_content))
}

fn extract_reasoning_like_text(value: &Value) -> Option<&str> {
    ["reasoning_content", "reasoning", "reasoning_details"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())
}

fn classify_status_failure(status_code: u16) -> Option<GatewayFailureKind> {
    match status_code {
        200..=399 => None,
        400 => Some(GatewayFailureKind::UpstreamBadRequest),
        // 402 Payment Required is a provider-side billing/quota failure, not a client
        // request schema error. Treat it like auth so failover can switch providers.
        401 | 402 | 403 => Some(GatewayFailureKind::Auth),
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

/// Decide whether a failure may trigger same-provider retry or failover.
///
/// HTTP status-code failures honor `retryable_status_codes` from gateway settings.
/// Non-status failures (connection/timeout/empty 2xx) keep the historical kind rules.
fn should_retry_status_or_kind(
    status_code: Option<u16>,
    kind: GatewayFailureKind,
    retryable_status_codes: &std::collections::BTreeSet<u16>,
) -> bool {
    if kind == GatewayFailureKind::EmptyResponse {
        return should_retry_failure(kind);
    }
    if let Some(status) = status_code {
        if classify_status_failure(status).is_some() {
            return retryable_status_codes.contains(&status);
        }
    }
    should_retry_failure(kind)
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
    use crate::coding::proxy_gateway::types::ProxyGatewaySettings;

    fn debug_request(body: &[u8]) -> DebugHttpRequest {
        DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    fn debug_request_with_headers(body: &[u8], headers: Vec<(&str, &str)>) -> DebugHttpRequest {
        DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
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
                GatewayCliKey::Codex | GatewayCliKey::Grok => {
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
                GatewayCliKey::Codex | GatewayCliKey::Grok | GatewayCliKey::OpenCode => {
                    ProviderAuthStrategy::Bearer
                }
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

    fn responses_to_anthropic_route() -> ConversionRoute {
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages)
    }

    fn lossy_responses_value() -> Value {
        json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "code_interpreter_call",
                    "code": "print('not representable in Anthropic Messages')"
                }
            ]
        })
    }

    #[test]
    fn lossy_conversion_policy_allows_by_default() {
        let value = lossy_responses_value();
        let request = debug_request(serde_json::to_vec(&value).unwrap().as_slice());
        let context = GatewayRuntimeContext::new(
            crate::coding::proxy_gateway::types::ProxyGatewaySettings::default(),
            None,
            None,
        );

        let warnings = check_lossy_conversion_policy(
            Some(&context),
            &request,
            responses_to_anthropic_route(),
            &value,
        )
        .expect("lossy conversion should warn by default");

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("code_interpreter_call"));
    }

    #[test]
    fn lossy_conversion_policy_rejects_when_global_rejection_enabled() {
        let value = lossy_responses_value();
        let request = debug_request(serde_json::to_vec(&value).unwrap().as_slice());
        let context = GatewayRuntimeContext::new(
            crate::coding::proxy_gateway::types::ProxyGatewaySettings {
                lossy_rejection_enabled: true,
                ..crate::coding::proxy_gateway::types::ProxyGatewaySettings::default()
            },
            None,
            None,
        );

        let error = check_lossy_conversion_policy(
            Some(&context),
            &request,
            responses_to_anthropic_route(),
            &value,
        )
        .expect_err("enabled lossy rejection should reject lossy conversion");

        assert!(error.message.contains("Lossy protocol conversion rejected"));
        assert!(error.message.contains("code_interpreter_call"));
    }

    #[test]
    fn request_schema_failure_returns_client_error_without_provider_attempt() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let provider = provider_for_cli(GatewayCliKey::Codex);
        let response = local_request_schema_failure_response(
            &route,
            &provider,
            "gpt-5",
            "gpt-5",
            GatewayForwardError::new(
                "Lossy protocol conversion rejected: /input/0: code_interpreter_call",
                GatewayFailureKind::RequestSchema,
            ),
            1,
            1,
            false,
        );
        let body: Value = serde_json::from_slice(&response.body).unwrap();

        assert_eq!(response.status_code, 400);
        assert_eq!(response.status_text, "Bad Request");
        assert_eq!(response.error_category.as_deref(), Some("request_schema"));
        assert!(response.provider_attempts.is_empty());
        assert_eq!(body["error"], "gateway_request_schema_rejected");
        assert!(body["message"]
            .as_str()
            .is_some_and(|message| message.contains("Lossy protocol conversion rejected")));
    }

    #[test]
    fn compact_request_schema_failure_returns_openai_error_shape() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses/compact");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let response = local_request_schema_failure_response(
            &route,
            &provider,
            "gpt-5",
            "gpt-5",
            GatewayForwardError::new(
                "Codex Responses compact compatibility does not support streaming requests",
                GatewayFailureKind::RequestSchema,
            ),
            1,
            1,
            false,
        );
        let body: Value = serde_json::from_slice(&response.body).unwrap();

        assert_eq!(response.status_code, 400);
        assert_eq!(response.status_text, "Bad Request");
        assert_eq!(response.error_category.as_deref(), Some("request_schema"));
        assert_eq!(
            body.pointer("/error/message").and_then(Value::as_str),
            Some("Codex Responses compact compatibility does not support streaming requests")
        );
        assert_eq!(
            body.pointer("/error/type").and_then(Value::as_str),
            Some("invalid_request_error")
        );
        assert_eq!(
            body.pointer("/error/code").and_then(Value::as_str),
            Some("gateway_request_schema_rejected")
        );
    }

    #[test]
    fn lossy_conversion_policy_allows_request_header_bypass() {
        let value = lossy_responses_value();
        let mut request = debug_request(serde_json::to_vec(&value).unwrap().as_slice());
        request
            .headers
            .push(("X-Allow-Lossy".to_string(), "true".to_string()));
        let context = GatewayRuntimeContext::new(
            crate::coding::proxy_gateway::types::ProxyGatewaySettings {
                lossy_rejection_enabled: true,
                ..crate::coding::proxy_gateway::types::ProxyGatewaySettings::default()
            },
            None,
            None,
        );

        let warnings = check_lossy_conversion_policy(
            Some(&context),
            &request,
            responses_to_anthropic_route(),
            &value,
        )
        .expect("header bypass should allow lossy conversion");

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("code_interpreter_call"));
    }

    #[test]
    fn lossy_conversion_policy_allows_when_global_rejection_disabled() {
        let value = lossy_responses_value();
        let request = debug_request(serde_json::to_vec(&value).unwrap().as_slice());
        let context = GatewayRuntimeContext::new(
            crate::coding::proxy_gateway::types::ProxyGatewaySettings {
                lossy_rejection_enabled: false,
                ..crate::coding::proxy_gateway::types::ProxyGatewaySettings::default()
            },
            None,
            None,
        );

        let warnings = check_lossy_conversion_policy(
            Some(&context),
            &request,
            responses_to_anthropic_route(),
            &value,
        )
        .expect("disabled global rejection should allow lossy conversion");

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("code_interpreter_call"));
    }

    #[test]
    fn lossy_warning_header_is_appended_to_allowed_response() {
        let mut headers = Vec::new();
        let context = ConversionContext {
            lossy_warnings: vec!["/input/0: code_interpreter_call is lossy".to_string()],
            ..ConversionContext::default()
        };

        append_lossy_warning_header(&mut headers, Some(&context));

        assert_eq!(
            headers,
            vec![(
                "X-Transformer-Lossy".to_string(),
                "/input/0: code_interpreter_call is lossy".to_string()
            )]
        );
    }

    #[test]
    fn gemini_shadow_session_key_requires_real_session_hint() {
        let provider = provider_for_cli(GatewayCliKey::Gemini);
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}]}
            ]
        });
        let request_without_session = debug_request(serde_json::to_vec(&body).unwrap().as_slice());
        let request_with_session = debug_request_with_headers(
            serde_json::to_vec(&body).unwrap().as_slice(),
            vec![("X-Session-Id", "session-a")],
        );

        assert!(gemini_shadow_session_key(&provider, &request_without_session, &body).is_none());
        assert!(gemini_shadow_session_key(&provider, &request_with_session, &body).is_some());
    }

    #[test]
    fn gemini_shadow_records_from_aggregated_sse_json_body() {
        let context = GatewayRuntimeContext::new(
            crate::coding::proxy_gateway::types::ProxyGatewaySettings::default(),
            None,
            None,
        );
        let provider = provider_for_cli(GatewayCliKey::Gemini);
        let upstream_body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "call lookup"}]}
            ]
        });
        let request = debug_request_with_headers(
            serde_json::to_vec(&upstream_body).unwrap().as_slice(),
            vec![("X-Session-Id", "session-a")],
        );
        let raw_sse = br#"data: {"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"lookup","args":{"query":"rust"}},"thoughtSignature":"sig_1"}]}}]}"#;
        let aggregated_gemini_json = json!({
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "functionCall": {
                                    "name": "lookup",
                                    "args": {"query": "rust"}
                                },
                                "thoughtSignature": "sig_1"
                            }
                        ]
                    }
                }
            ]
        });

        record_side_store_response(
            Some(&context),
            &provider,
            &request,
            &serde_json::to_vec(&upstream_body).unwrap(),
            raw_sse,
            &serde_json::to_vec(&aggregated_gemini_json).unwrap(),
            &serde_json::to_vec(&aggregated_gemini_json).unwrap(),
            None,
        );

        let key = gemini_shadow_session_key(&provider, &request, &upstream_body).unwrap();
        let mut follow_up = json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [
                        {
                            "functionResponse": {
                                "name": "lookup",
                                "response": {"result": "ok"}
                            }
                        }
                    ]
                }
            ]
        });

        assert_eq!(
            context
                .side_stores
                .enrich_gemini_request(&key, &mut follow_up),
            1
        );
        assert_eq!(
            follow_up["contents"][0]["parts"][0]["thoughtSignature"],
            "sig_1"
        );
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
    fn responses_compact_direct_route_keeps_compact_endpoint() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses/compact");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiResponses,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let source_protocol = source_protocol_from_route(&route).unwrap();
        let conversion = conversion_route(source_protocol, &provider);
        let compact_compat = CodexResponsesCompactCompat::new(&route, &provider);

        assert!(route_is_openai_responses_compact(&route));
        assert!(compact_compat.is_compact());
        assert!(!compact_compat.is_fallback());
        assert!(conversion.is_none());
        assert_eq!(
            upstream_forwarded_path(&route, &provider, conversion, "gpt-5", false).as_ref(),
            "/v1/responses/compact"
        );
    }

    #[test]
    fn responses_compact_anthropic_fallback_rewrites_to_messages_path() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses/compact");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::AnthropicMessages,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let source_protocol = source_protocol_from_route(&route).unwrap();
        let generic_conversion = conversion_route(source_protocol, &provider);
        let compact_compat = CodexResponsesCompactCompat::new(&route, &provider);
        let compact_conversion = compact_compat.conversion_route();

        assert!(route_is_openai_responses_compact(&route));
        assert!(generic_conversion.is_some());
        assert!(compact_compat.is_fallback());
        assert_eq!(
            compact_conversion,
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::AnthropicMessages
            ))
        );
        assert_eq!(
            upstream_forwarded_path(
                &route,
                &provider,
                compact_conversion,
                "claude-sonnet",
                false
            )
            .as_ref(),
            "/v1/messages"
        );
    }

    #[test]
    fn responses_compact_chat_fallback_rewrites_to_chat_path() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses/compact");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let compact_compat = CodexResponsesCompactCompat::new(&route, &provider);
        let compact_conversion = compact_compat.conversion_route();

        assert!(compact_compat.is_fallback());
        assert_eq!(
            upstream_forwarded_path(&route, &provider, compact_conversion, "gpt-4o", false)
                .as_ref(),
            "/v1/chat/completions"
        );
    }

    #[test]
    fn responses_compact_gemini_fallback_rewrites_to_generate_content_path() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses/compact");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::GeminiNative,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let compact_compat = CodexResponsesCompactCompat::new(&route, &provider);
        let compact_conversion = compact_compat.conversion_route();

        assert!(compact_compat.is_fallback());
        assert_eq!(
            upstream_forwarded_path(
                &route,
                &provider,
                compact_conversion,
                "gemini-2.5-pro",
                false
            )
            .as_ref(),
            "/v1beta/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn deepseek_legacy_completion_route_uses_beta_path() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/completions");
        let provider = UpstreamProvider {
            base_url: "https://api.deepseek.com/v1".to_string(),
            target_protocol: AiProtocol::OpenAiChat,
            meta: ProviderGatewayMeta {
                provider_type: Some("deepseek".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let url = build_provider_target_url(
            &provider,
            route.forwarded_path.as_str(),
            Some("request_id=abc"),
            None,
            false,
            "deepseek-chat",
        )
        .unwrap();

        assert_eq!(
            url.as_str(),
            "https://api.deepseek.com/beta/completions?request_id=abc"
        );
    }

    #[test]
    fn deepseek_legacy_completion_body_skips_chat_adapter() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/completions".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"text-model","prompt":"hello","max_tokens":8}"#.to_vec(),
        };
        let meta = ProviderGatewayMeta {
            provider_type: Some("deepseek".to_string()),
            ..ProviderGatewayMeta::default()
        };
        let prepared = build_upstream_body_for_provider(
            &request,
            "text-model",
            "deepseek-chat",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::OpenAiChat,
            None,
            Some(&meta),
            None,
            None,
            false,
            CodexResponsesCompactCompat::none(),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&prepared.body).unwrap();

        assert_eq!(value["model"], "deepseek-chat");
        assert_eq!(value["prompt"], "hello");
        assert_eq!(value["max_tokens"], 8);
        assert!(value.get("thinking").is_none());
        assert!(value.get("messages").is_none());
    }

    #[test]
    fn provider_pipeline_caps_default_max_tokens_in_upstream_body() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/chat/completions".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"max_tokens":2048}"#.to_vec(),
        };
        let meta = ProviderGatewayMeta {
            default_max_tokens: Some(256),
            ..ProviderGatewayMeta::default()
        };

        let prepared = build_upstream_body_for_provider(
            &request,
            "gpt-4o",
            "gpt-4o",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::OpenAiChat,
            None,
            Some(&meta),
            None,
            None,
            false,
            CodexResponsesCompactCompat::none(),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&prepared.body).unwrap();

        assert_eq!(value["max_tokens"], 256);
    }

    #[test]
    fn provider_pipeline_strips_billing_cch_for_non_anthropic_target() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"claude-sonnet","max_tokens":128,"system":[{"type":"text","text":"x-anthropic-billing-header: cc_version=2.1.42; cch=abc;\n\nStable prompt"}],"messages":[{"role":"user","content":"hi"}]}"#.to_vec(),
        };

        let prepared = build_upstream_body_for_provider(
            &request,
            "claude-sonnet",
            "gpt-4o",
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::OpenAiChat,
            Some(ConversionRoute::new(
                AiProtocol::AnthropicMessages,
                AiProtocol::OpenAiChat,
            )),
            None,
            None,
            None,
            false,
            CodexResponsesCompactCompat::none(),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&prepared.body).unwrap();
        let system_content = value["messages"][0]["content"].as_str().unwrap();

        assert!(system_content.contains("Stable prompt"));
        assert!(!system_content.contains("x-anthropic-billing-header"));
        assert!(!system_content.contains("cch=abc"));
    }

    #[test]
    fn provider_pipeline_restores_billing_cch_for_anthropic_target() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"claude-sonnet","max_tokens":128,"system":[{"type":"text","text":"x-anthropic-billing-header: cc_version=2.1.42; cch=abc;\n\nStable prompt"}],"messages":[{"role":"user","content":"hi"}]}"#.to_vec(),
        };

        let prepared = build_upstream_body_for_provider(
            &request,
            "claude-sonnet",
            "claude-sonnet",
            false,
            false,
            GatewayCliKey::Claude,
            AiProtocol::AnthropicMessages,
            None,
            None,
            None,
            None,
            false,
            CodexResponsesCompactCompat::none(),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&prepared.body).unwrap();
        let system_text = value["system"][0]["text"].as_str().unwrap();

        assert!(system_text.contains("cc_version=2.1.42"));
        assert!(system_text.contains("cch=abc"));
    }

    #[test]
    fn responses_sse_auto_aggregate_applies_to_non_codex_provider() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/responses".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"gpt-5","input":"hi"}"#.to_vec(),
        };
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiResponses,
            meta: ProviderGatewayMeta {
                provider_type: Some("openrouter".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));

        assert!(
            should_aggregate_openai_responses_sse_for_non_streaming_client(
                &request, &route, &provider, &headers, 200,
            )
        );
    }

    #[test]
    fn responses_sse_auto_aggregate_skips_explicit_streaming_client() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/responses".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"gpt-5","input":"hi","stream":true}"#.to_vec(),
        };
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiResponses,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));

        assert!(
            !should_aggregate_openai_responses_sse_for_non_streaming_client(
                &request, &route, &provider, &headers, 200,
            )
        );
    }

    #[test]
    fn chat_sse_auto_aggregate_applies_to_non_streaming_client() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/chat/completions".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#.to_vec(),
        };
        let route = gateway_route(GatewayCliKey::Codex, "/v1/chat/completions");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));

        assert!(should_aggregate_openai_chat_sse_for_non_streaming_client(
            &request, &route, &provider, &headers, 200,
        ));
    }

    #[test]
    fn chat_sse_auto_aggregate_skips_explicit_streaming_client() {
        let request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/openai/v1/chat/completions".to_string(),
            headers: Vec::new(),
            body:
                br#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}"#
                    .to_vec(),
        };
        let route = gateway_route(GatewayCliKey::Codex, "/v1/chat/completions");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));

        assert!(!should_aggregate_openai_chat_sse_for_non_streaming_client(
            &request, &route, &provider, &headers, 200,
        ));
    }

    #[test]
    fn anthropic_and_gemini_sse_auto_aggregate_apply_to_non_streaming_client() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));

        let anthropic_request = DebugHttpRequest {
            id: 1,
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            headers: Vec::new(),
            body: br#"{"model":"claude-sonnet","max_tokens":128,"messages":[{"role":"user","content":"hi"}]}"#.to_vec(),
        };
        let anthropic_route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        let anthropic_provider = UpstreamProvider {
            target_protocol: AiProtocol::AnthropicMessages,
            ..provider_for_cli(GatewayCliKey::Claude)
        };
        assert_eq!(
            sse_aggregation_kind_for_non_streaming_client(
                &anthropic_request,
                &anthropic_route,
                &anthropic_provider,
                &headers,
                200,
            ),
            Some(SseAggregateKind::AnthropicMessages)
        );

        let gemini_request = DebugHttpRequest {
            id: 2,
            method: "POST".to_string(),
            path: "/gemini/v1beta/models/gemini-pro:generateContent".to_string(),
            headers: Vec::new(),
            body: br#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#.to_vec(),
        };
        let gemini_route = gateway_route(GatewayCliKey::Gemini, "/v1beta/models/m:generateContent");
        let gemini_provider = UpstreamProvider {
            target_protocol: AiProtocol::GeminiNative,
            ..provider_for_cli(GatewayCliKey::Gemini)
        };
        assert_eq!(
            sse_aggregation_kind_for_non_streaming_client(
                &gemini_request,
                &gemini_route,
                &gemini_provider,
                &headers,
                200,
            ),
            Some(SseAggregateKind::GeminiNative)
        );
    }

    #[test]
    fn full_url_provider_does_not_append_forwarded_path() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let provider = UpstreamProvider {
            base_url: "https://api.example.com/custom/chat?existing=1".to_string(),
            is_full_url: true,
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let url = build_provider_target_url(
            &provider,
            "/v1/chat/completions",
            route.query.as_deref(),
            None,
            false,
            "gpt-5",
        )
        .unwrap();

        assert_eq!(
            url.as_str(),
            "https://api.example.com/custom/chat?existing=1"
        );
    }

    #[test]
    fn anthropic_bedrock_provider_uses_model_invoke_path_and_version_header() {
        let route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::AnthropicMessages,
            auth_strategy: ProviderAuthStrategy::Bearer,
            base_url: "https://bedrock-runtime.us-east-1.amazonaws.com".to_string(),
            meta: ProviderGatewayMeta {
                provider_type: Some("bedrock".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::Claude)
        };
        let url = build_provider_target_url(
            &provider,
            "/v1/messages",
            route.query.as_deref(),
            None,
            true,
            "anthropic.claude-sonnet-4-20250514-v1:0",
        )
        .unwrap();
        let headers = build_upstream_headers(&debug_request(b"{}"), &provider, None).unwrap();

        assert_eq!(
            url.as_str(),
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-sonnet-4-20250514-v1:0/invoke-with-response-stream"
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok()),
            Some("bedrock-2023-05-31")
        );
        assert!(headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key("x-api-key"));
    }

    #[test]
    fn anthropic_bedrock_body_clears_model_and_stream() {
        let body = apply_outbound_adapter_compat_for_provider_type(
            br#"{
                "model":"claude-sonnet",
                "stream":true,
                "tools":[{"type":"web_search_20250305","name":"web_search"}],
                "messages":[{"role":"user","content":"hi"}]
            }"#
            .to_vec(),
            None,
            AiProtocol::AnthropicMessages,
            "bedrock",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(
            value.get("anthropic_version").and_then(Value::as_str),
            Some("bedrock-2023-05-31")
        );
        assert_eq!(value["anthropic_beta"][0], "web-search-2025-03-05");
        assert!(value.get("model").is_none());
        assert!(value.get("stream").is_none());
    }

    #[test]
    fn anthropic_direct_web_search_adds_beta_header() {
        let provider = provider_for_cli(GatewayCliKey::Claude);
        let body = br#"{
            "model":"claude-sonnet",
            "tools":[{"type":"web_search_20250305","name":"web_search"}],
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let headers = build_upstream_headers(&debug_request(b"{}"), &provider, Some(body)).unwrap();

        assert_eq!(
            headers
                .get("anthropic-beta")
                .and_then(|value| value.to_str().ok()),
            Some("web-search-2025-03-05")
        );
    }

    #[test]
    fn anthropic_vertex_filters_native_web_search_tool() {
        let body = apply_outbound_adapter_compat_for_provider_type(
            br#"{
                "model":"claude-sonnet",
                "tools":[
                    {"type":"web_search_20250305","name":"web_search"},
                    {"name":"read_file","input_schema":{"type":"object"}}
                ],
                "messages":[{"role":"user","content":"hi"}]
            }"#
            .to_vec(),
            None,
            AiProtocol::AnthropicMessages,
            "anthropic-vertex",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["tools"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["tools"][0]["name"], "read_file");
    }

    #[test]
    fn anthropic_vertex_provider_uses_raw_predict_path_and_version_header() {
        let route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::AnthropicMessages,
            auth_strategy: ProviderAuthStrategy::GoogleOAuth,
            base_url:
                "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1"
                    .to_string(),
            api_key: "ya29.token".to_string(),
            meta: ProviderGatewayMeta {
                provider_type: Some("anthropic-vertex".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::Claude)
        };
        let url = build_provider_target_url(
            &provider,
            "/v1/messages",
            route.query.as_deref(),
            None,
            false,
            "claude-sonnet-4",
        )
        .unwrap();
        let headers = build_upstream_headers(&debug_request(b"{}"), &provider, None).unwrap();

        assert_eq!(
            url.as_str(),
            "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/anthropic/models/claude-sonnet-4:rawPredict"
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok()),
            Some("vertex-2023-10-16")
        );
        assert!(headers.contains_key(AUTHORIZATION));
    }

    fn resolve_model(
        requested_model: &str,
        provider: &UpstreamProvider,
        apply_failover_model_mapping: bool,
    ) -> String {
        resolve_upstream_model_id(
            &debug_request(b"{}"),
            requested_model,
            provider,
            apply_failover_model_mapping,
            true,
        )
    }

    #[test]
    fn claude_model_mapping_uses_provider_specific_model_for_standard_name() {
        let provider = claude_provider(UpstreamModelMapping {
            sonnet_model: Some("provider-sonnet".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_model("claude-sonnet-4-6", &provider, true),
            "provider-sonnet"
        );
    }

    #[test]
    fn claude_model_mapping_uses_provider_specific_fable_model() {
        let provider = claude_provider(UpstreamModelMapping {
            fable_model: Some("provider-fable[1M]".to_string()),
            opus_model: Some("provider-opus".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_model("claude-fable-5", &provider, true),
            "provider-fable"
        );
    }

    #[test]
    fn claude_model_mapping_fable_falls_back_to_opus_when_unset() {
        let provider = claude_provider(UpstreamModelMapping {
            opus_model: Some("provider-opus".to_string()),
            default_model: Some("provider-default".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_model("claude-fable-5", &provider, true),
            "provider-opus"
        );
    }

    #[test]
    fn claude_model_mapping_falls_back_to_default_model_when_family_missing() {
        let provider = claude_provider(UpstreamModelMapping {
            default_model: Some("provider-default".to_string()),
            ..UpstreamModelMapping::default()
        });

        assert_eq!(
            resolve_model("claude-opus-4-7", &provider, true),
            "provider-default"
        );
    }

    #[test]
    fn claude_model_mapping_falls_back_to_standard_model_when_no_mapping_exists() {
        let provider = claude_provider(UpstreamModelMapping::default());

        assert_eq!(
            resolve_model("claude-opus-4-7", &provider, true),
            "claude-opus-4-7"
        );
    }

    #[test]
    fn claude_model_mapping_strips_one_m_marker_before_upstream() {
        let provider = claude_provider(UpstreamModelMapping::default());

        assert_eq!(
            resolve_model("claude-sonnet-4-6[1M]", &provider, true),
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
            resolve_model("claude-sonnet-4-6[1M]", &provider, true),
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
            resolve_model("claude-sonnet-4-6[1M]", &provider, false),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn codex_model_strips_one_m_marker_before_upstream() {
        let provider = provider_for_cli(GatewayCliKey::Codex);

        assert_eq!(
            resolve_model("gpt-5-codex[1M]", &provider, true),
            "gpt-5-codex"
        );
    }

    #[test]
    fn codex_failover_uses_provider_default_model() {
        let mut provider = provider_for_cli(GatewayCliKey::Codex);
        provider.model_mapping.default_model = Some("deepseek-v4-flash".to_string());

        assert_eq!(
            resolve_model("gpt-5.4-codex", &provider, true),
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn codex_single_mode_preserves_requested_model() {
        let mut provider = provider_for_cli(GatewayCliKey::Codex);
        provider.model_mapping.default_model = Some("deepseek-v4-flash".to_string());

        assert_eq!(
            resolve_model("gpt-5.4-codex", &provider, false),
            "gpt-5.4-codex"
        );
    }

    #[test]
    fn codex_failover_without_default_model_passthroughs_request() {
        let provider = provider_for_cli(GatewayCliKey::Codex);

        assert_eq!(
            resolve_model("gpt-5.4-codex", &provider, true),
            "gpt-5.4-codex"
        );
    }

    #[test]
    fn codex_failover_strips_one_m_from_default_model() {
        let mut provider = provider_for_cli(GatewayCliKey::Codex);
        provider.model_mapping.default_model = Some("provider-model[1M]".to_string());

        assert_eq!(
            resolve_model("gpt-5.4-codex", &provider, true),
            "provider-model"
        );
    }

    #[test]
    fn grok_failover_uses_provider_default_model() {
        let mut provider = provider_for_cli(GatewayCliKey::Grok);
        provider.model_mapping.default_model = Some("grok-3-mini".to_string());

        assert_eq!(
            resolve_model("grok-build", &provider, true),
            "grok-3-mini"
        );
    }

    #[test]
    fn grok_single_mode_also_uses_provider_default_model() {
        let mut provider = provider_for_cli(GatewayCliKey::Grok);
        provider.model_mapping.default_model = Some("grok-3-mini".to_string());

        // Grok CLI takeover hardcodes model=grok-build even in single mode.
        assert_eq!(
            resolve_model("grok-build", &provider, false),
            "grok-3-mini"
        );
    }

    #[test]
    fn grok_provider_override_preserves_requested_model() {
        let mut provider = provider_for_cli(GatewayCliKey::Grok);
        provider.model_mapping.default_model = Some("grok-3-mini".to_string());

        assert_eq!(
            resolve_upstream_model_id(
                &debug_request(b"{}"),
                "user-pinned-model",
                &provider,
                false,
                false,
            ),
            "user-pinned-model"
        );
    }

    #[test]
    fn gemini_model_strips_encoded_one_m_marker_before_upstream() {
        let provider = provider_for_cli(GatewayCliKey::Gemini);

        assert_eq!(
            resolve_model("gemini-2.5-pro%5B1M%5D", &provider, true),
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
            resolve_upstream_model_id(&request, "claude-sonnet-4-6", &provider, true, true),
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
    fn responses_prompt_cache_key_falls_back_to_session_header() {
        let request = debug_request_with_headers(
            br#"{
                "model":"gpt-5.1-codex-mini",
                "messages":[{"role":"user","content":"hi"}]
            }"#,
            vec![("X-Session-Id", "shared-session-123")],
        );

        let body = build_upstream_body(
            &request,
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-mini",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::OpenAiResponses,
            Some(ConversionRoute::new(
                AiProtocol::OpenAiChat,
                AiProtocol::OpenAiResponses,
            )),
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["prompt_cache_key"], "shared-session-123");
    }

    #[test]
    fn responses_prompt_cache_key_keeps_explicit_request_value() {
        let request = debug_request_with_headers(
            br#"{
                "model":"gpt-5.1-codex-mini",
                "messages":[{"role":"user","content":"hi"}],
                "prompt_cache_key":"explicit-cache-key"
            }"#,
            vec![("X-Session-Id", "shared-session-123")],
        );

        let body = build_upstream_body(
            &request,
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-mini",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::OpenAiResponses,
            Some(ConversionRoute::new(
                AiProtocol::OpenAiChat,
                AiProtocol::OpenAiResponses,
            )),
            false,
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["prompt_cache_key"], "explicit-cache-key");
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
        assert_eq!(messages[1]["reasoning_content"], "Need to edit one file.");
        assert!(messages[1].get("reasoning").is_none());
        assert!(messages[1].get("tool_calls").is_none());
    }

    #[test]
    fn outbound_adapter_normalizes_direct_chat_body_for_provider_compat() {
        let body = include_bytes!(
            "../transformer/fixtures/live_provider/openai_chat/codex-chat-identity-developer-empty-tool-arguments.request.json"
        );

        let body =
            apply_outbound_adapter_compat(body.to_vec(), None, AiProtocol::OpenAiChat).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("verbosity").is_none());
        assert!(value.get("reasoning_effort").is_none());
        assert!(value.get("prompt_cache_key").is_none());
        assert_eq!(value["tools"].as_array().unwrap().len(), 1);
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tool_choice"], "auto");
        assert_eq!(value["parallel_tool_calls"], true);

        let messages = value["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(
            messages[0]["content"],
            "You are Codex, a coding agent. Follow the user's instructions and use tools carefully.\n\nFollow project instructions and use tools carefully."
        );
        assert!(messages
            .iter()
            .all(|message| message.get("role").and_then(Value::as_str) != Some("developer")));
        assert_eq!(messages[2]["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(messages[2]["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn outbound_adapter_strips_google_thought_signature_from_openai_chat_tool_calls() {
        let body = br#"{
            "model":"gpt-4o",
            "messages":[
                {"role":"user","content":"use tools"},
                {
                    "role":"assistant",
                    "content":null,
                    "tool_calls":[
                        {
                            "id":"call_empty_google",
                            "type":"function",
                            "function":{"name":"keep","arguments":"{}"},
                            "extra_content":{"google":{}}
                        },
                        {
                            "id":"call_extra_content",
                            "type":"function",
                            "function":{"name":"strip_extra","arguments":"{}"},
                            "extra_content":{"google":{"thought_signature":"sig-extra"}}
                        },
                        {
                            "id":"call_extra_fields",
                            "type":"function",
                            "function":{"name":"strip_fields","arguments":"{}"},
                            "extra_fields":{"extra_content":{"google":{"thought_signature":"sig-fields"}}}
                        },
                        {
                            "id":"call_direct",
                            "type":"function",
                            "function":{"name":"strip_direct","arguments":"{}"},
                            "google":{"thought_signature":"sig-direct"},
                            "thought_signature":"sig-top"
                        }
                    ]
                }
            ]
        }"#;

        let body =
            apply_outbound_adapter_compat(body.to_vec(), None, AiProtocol::OpenAiChat).unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        let tool_calls = value["messages"][1]["tool_calls"].as_array().unwrap();

        assert!(tool_calls[0].get("extra_content").is_some());
        assert!(tool_calls[1].get("extra_content").is_none());
        assert!(tool_calls[2].get("extra_fields").is_none());
        assert!(tool_calls[3].get("google").is_none());
        assert!(tool_calls[3].get("thought_signature").is_none());
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
    fn codex_history_enriches_responses_follow_up_for_anthropic_target() {
        let context = GatewayRuntimeContext::new(
            crate::coding::proxy_gateway::types::ProxyGatewaySettings::default(),
            None,
            None,
        );
        context.side_stores.record_codex_response(&json!({
            "id": "resp_1",
            "output": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "read_file",
                "arguments": "{\"path\":\"README.md\"}",
                "status": "completed"
            }]
        }));
        let request = debug_request(
            br#"{
                "model":"gpt-5-codex",
                "previous_response_id":"resp_1",
                "input":[{
                    "type":"function_call_output",
                    "call_id":"call_1",
                    "output":"ok"
                }]
            }"#,
        );

        let prepared = build_upstream_body_for_provider(
            &request,
            "gpt-5-codex",
            "claude-sonnet-4-6",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::AnthropicMessages,
            Some(responses_to_anthropic_route()),
            None,
            Some(&context),
            None,
            false,
            CodexResponsesCompactCompat::none(),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&prepared.body).unwrap();

        assert_eq!(value["messages"][0]["role"], "assistant");
        assert_eq!(value["messages"][0]["content"][0]["type"], "tool_use");
        assert_eq!(value["messages"][0]["content"][0]["id"], "call_1");
        assert_eq!(value["messages"][0]["content"][0]["name"], "read_file");
        assert_eq!(
            value["messages"][0]["content"][0]["input"]["path"],
            "README.md"
        );
        assert_eq!(value["messages"][1]["role"], "user");
        assert_eq!(value["messages"][1]["content"][0]["type"], "tool_result");
        assert_eq!(value["messages"][1]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(value["messages"][1]["content"][0]["content"], "ok");
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
    fn outbound_adapter_strips_gemini_function_ids_for_vertex() {
        let body = br#"{
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "functionCall": {
                                "id": "call_weather",
                                "name": "get_weather",
                                "args": {"location": "Tokyo"}
                            }
                        }
                    ]
                },
                {
                    "role": "user",
                    "parts": [
                        {
                            "functionResponse": {
                                "id": "call_weather",
                                "name": "get_weather",
                                "response": {"ok": true}
                            }
                        }
                    ]
                }
            ]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::GeminiNative,
            "google-vertex",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value["contents"][0]["parts"][0]["functionCall"]
            .get("id")
            .is_none());
        assert_eq!(
            value["contents"][0]["parts"][0]["functionCall"]["name"],
            "get_weather"
        );
        assert!(value["contents"][1]["parts"][0]["functionResponse"]
            .get("id")
            .is_none());
        assert_eq!(
            value["contents"][1]["parts"][0]["functionResponse"]["response"]["ok"],
            true
        );
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
            ProviderBodyCompat::from_provider_type(Some(" DeepSeek "), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::DeepSeek)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("model_scope"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::ModelScope)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("glm"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::Zai)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("x-ai"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::Xai)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("aliyun"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::Bailian)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("xiaomi_mimo"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::Mimo)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("open_router"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::OpenRouter)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("google-vertex"), AiProtocol::GeminiNative),
            Some(ProviderBodyCompat::GeminiVertex)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(
                Some("openai_codex"),
                AiProtocol::OpenAiResponses
            ),
            Some(ProviderBodyCompat::CodexOfficial)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("github_copilot"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::Copilot)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("ollama"), AiProtocol::OpenAiChat),
            Some(ProviderBodyCompat::Ollama)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_meta(
                Some(&ProviderGatewayMeta {
                    api_format: Some("ollama/chat".to_string()),
                    ..ProviderGatewayMeta::default()
                }),
                AiProtocol::OpenAiChat
            ),
            Some(ProviderBodyCompat::Ollama)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("vertex"), AiProtocol::AnthropicMessages),
            Some(ProviderBodyCompat::AnthropicVertex)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("bedrock"), AiProtocol::AnthropicMessages),
            Some(ProviderBodyCompat::AnthropicBedrock)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("custom"), AiProtocol::OpenAiChat),
            None
        );
    }

    #[test]
    fn ollama_chat_url_uses_api_chat_and_strips_v1_base_suffix() {
        let route = gateway_route(GatewayCliKey::Codex, "/v1/chat/completions");
        let provider = UpstreamProvider {
            base_url: "http://localhost:11434/v1".to_string(),
            target_protocol: AiProtocol::OpenAiChat,
            meta: ProviderGatewayMeta {
                provider_type: Some("ollama".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::Codex)
        };
        let url = build_provider_target_url(
            &provider,
            &route.forwarded_path,
            Some("trace=1"),
            None,
            false,
            "llama3.2",
        )
        .unwrap();

        assert_eq!(url.as_str(), "http://localhost:11434/api/chat?trace=1");
    }

    #[test]
    fn ollama_body_compat_converts_openai_chat_request_shape() {
        let meta = ProviderGatewayMeta {
            provider_type: Some("ollama".to_string()),
            ..ProviderGatewayMeta::default()
        };
        let body = apply_outbound_adapter_compat_for_provider(
            br#"{
                "model":"llava",
                "stream":true,
                "temperature":0.2,
                "top_p":0.8,
                "max_tokens":128,
                "stop":["</s>"],
                "response_format":{"type":"json_object"},
                "messages":[
                    {"role":"user","content":[
                        {"type":"text","text":"OCR this image."},
                        {"type":"image_url","image_url":{"url":"data:image/png;base64,Zm9v"}}
                    ]},
                    {"role":"assistant","content":"answer","reasoning_content":"thinking"}
                ]
            }"#
            .to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["model"], "llava");
        assert_eq!(value["stream"], true);
        assert_eq!(value["format"], "json");
        assert_eq!(value["messages"][0]["content"], "OCR this image.");
        assert_eq!(value["messages"][0]["images"][0], "Zm9v");
        assert_eq!(value["messages"][1]["thinking"], "thinking");
        assert_eq!(value["options"]["temperature"], 0.2);
        assert_eq!(value["options"]["top_p"], 0.8);
        assert_eq!(value["options"]["num_predict"], 128);
        assert_eq!(value["options"]["stop"][0], "</s>");
    }

    #[test]
    fn ollama_json_response_converts_to_openai_chat_response() {
        let body = convert_ollama_chat_response_to_openai_chat(
            br#"{
                "model":"llama3.2",
                "message":{"role":"assistant","content":"hello","thinking":"plan"},
                "done":true,
                "done_reason":"stop",
                "prompt_eval_count":3,
                "eval_count":4
            }"#,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["object"], "chat.completion");
        assert_eq!(value["model"], "llama3.2");
        assert_eq!(value["choices"][0]["message"]["role"], "assistant");
        assert_eq!(value["choices"][0]["message"]["content"], "hello");
        assert_eq!(value["choices"][0]["message"]["reasoning_content"], "plan");
        assert_eq!(value["choices"][0]["finish_reason"], "stop");
        assert_eq!(value["usage"]["prompt_tokens"], 3);
        assert_eq!(value["usage"]["completion_tokens"], 4);
        assert_eq!(value["usage"]["total_tokens"], 7);
    }

    #[test]
    fn ollama_ndjson_stream_converts_to_openai_chat_sse() {
        let stream: DebugBodyStream = Box::pin(futures_util::stream::iter(vec![
            Ok::<Vec<u8>, String>(br#"{"model":"llama3.2","message":{"role":"assistant","content":"hel"},"done":false}
"#
            .to_vec()),
            Ok::<Vec<u8>, String>(br#"{"model":"llama3.2","message":{"role":"assistant","content":"lo","thinking":"plan"},"done":false}
{"model":"llama3.2","done":true,"done_reason":"stop","prompt_eval_count":2,"eval_count":3}
"#
            .to_vec()),
        ]));
        let mut converted = convert_ollama_ndjson_stream_to_openai_chat_sse(stream);
        let chunks = tauri::async_runtime::block_on(async move {
            let mut chunks = Vec::new();
            while let Some(chunk) = converted.next().await {
                chunks.push(String::from_utf8(chunk.unwrap()).unwrap());
            }
            chunks
        });
        let joined = chunks.join("");

        assert!(joined.contains("\"object\":\"chat.completion.chunk\""));
        assert!(joined.contains("\"content\":\"hel\""));
        assert!(joined.contains("\"content\":\"lo\""));
        assert!(joined.contains("\"reasoning_content\":\"plan\""));
        assert!(joined.contains("\"finish_reason\":\"stop\""));
        assert!(joined.contains("\"completion_tokens\":3"));
        assert!(joined.contains("\"prompt_tokens\":2"));
        assert!(joined.contains("\"total_tokens\":5"));
        assert!(joined.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn copilot_model_uses_responses_api_matches_axonhub_rule() {
        for model in [
            "gpt-5",
            "gpt-5.3",
            "gpt-5.4",
            "gpt-5.4-preview",
            "gpt-5.10",
            "gpt-6",
            "gpt-6.1",
            "gpt-6-preview",
            "gpt-5-codex[1M]",
        ] {
            assert!(
                copilot_model_uses_responses_api(model),
                "{model} should use Responses"
            );
        }
        for model in ["gpt-5-mini", "gpt-4o", "claude-sonnet-4.6"] {
            assert!(
                !copilot_model_uses_responses_api(model),
                "{model} should use Chat"
            );
        }
    }

    #[test]
    fn copilot_effective_provider_switches_chat_and_responses_by_model() {
        let provider = UpstreamProvider {
            base_url: "https://api.githubcopilot.com".to_string(),
            target_protocol: AiProtocol::OpenAiChat,
            meta: ProviderGatewayMeta {
                provider_type: Some("copilot".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::OpenCode)
        };
        let route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        let source_protocol = source_protocol_from_route(&route).unwrap();

        let responses_provider = effective_upstream_provider_for_request(&provider, "gpt-5.4");
        assert_eq!(
            responses_provider.target_protocol,
            AiProtocol::OpenAiResponses
        );
        let responses_conversion = conversion_route(source_protocol, &responses_provider);
        assert_eq!(
            upstream_forwarded_path(
                &route,
                &responses_provider,
                responses_conversion,
                "gpt-5.4",
                false
            )
            .as_ref(),
            "/responses"
        );
        let responses_url = build_provider_target_url(
            &responses_provider,
            "/responses",
            route.query.as_deref(),
            responses_conversion,
            false,
            "gpt-5.4",
        )
        .unwrap();
        assert_eq!(
            responses_url.as_str(),
            "https://api.githubcopilot.com/responses"
        );

        let chat_provider = effective_upstream_provider_for_request(&provider, "gpt-5-mini");
        assert_eq!(chat_provider.target_protocol, AiProtocol::OpenAiChat);
        let chat_conversion = conversion_route(source_protocol, &chat_provider);
        assert_eq!(
            upstream_forwarded_path(&route, &chat_provider, chat_conversion, "gpt-5-mini", false)
                .as_ref(),
            "/chat/completions"
        );
        let chat_url = build_provider_target_url(
            &chat_provider,
            "/chat/completions",
            route.query.as_deref(),
            chat_conversion,
            false,
            "gpt-5-mini",
        )
        .unwrap();
        assert_eq!(
            chat_url.as_str(),
            "https://api.githubcopilot.com/chat/completions"
        );
    }

    #[test]
    fn copilot_warmup_downgrades_model_before_route_selection() {
        let provider = UpstreamProvider {
            base_url: "https://api.githubcopilot.com".to_string(),
            target_protocol: AiProtocol::OpenAiChat,
            meta: ProviderGatewayMeta {
                provider_type: Some("copilot".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::OpenCode)
        };
        let request = debug_request_with_headers(
            br#"{
                "model":"claude-sonnet-4-6",
                "messages":[{"role":"user","content":"warmup"}]
            }"#,
            vec![("anthropic-beta", "claude-code-20250219")],
        );

        let model = effective_upstream_model_id_for_request(&provider, "gpt-5.4", &request);
        assert_eq!(model.as_ref(), DEFAULT_COPILOT_WARMUP_MODEL);
        let effective_provider = effective_upstream_provider_for_request(&provider, model.as_ref());
        assert_eq!(effective_provider.target_protocol, AiProtocol::OpenAiChat);
    }

    #[test]
    fn copilot_warmup_does_not_downgrade_tool_or_agent_requests() {
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            meta: ProviderGatewayMeta {
                provider_type: Some("copilot".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::OpenCode)
        };
        let with_tools = debug_request_with_headers(
            br#"{
                "model":"claude-sonnet-4-6",
                "tools":[{"name":"Read","input_schema":{}}],
                "messages":[{"role":"user","content":"warmup"}]
            }"#,
            vec![("anthropic-beta", "claude-code-20250219")],
        );
        let agent = debug_request_with_headers(
            br#"{
                "model":"claude-sonnet-4-6",
                "messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}]
            }"#,
            vec![("anthropic-beta", "claude-code-20250219")],
        );

        assert_eq!(
            effective_upstream_model_id_for_request(&provider, "gpt-5.4", &with_tools).as_ref(),
            "gpt-5.4"
        );
        assert_eq!(
            effective_upstream_model_id_for_request(&provider, "gpt-5.4", &agent).as_ref(),
            "gpt-5.4"
        );
    }

    #[test]
    fn copilot_token_exchange_detection_is_explicit_or_github_token_shaped() {
        let base_provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            meta: ProviderGatewayMeta {
                provider_type: Some("github_copilot".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::OpenCode)
        };

        let github_token_provider = UpstreamProvider {
            api_key: "gho_test_access_token".to_string(),
            ..base_provider.clone()
        };
        assert!(should_exchange_copilot_access_token(&github_token_provider));

        let explicit_field_provider = UpstreamProvider {
            api_key: "custom-token-shape".to_string(),
            meta: ProviderGatewayMeta {
                provider_type: Some("github_copilot".to_string()),
                api_key_field: Some("github_access_token".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..base_provider.clone()
        };
        assert!(should_exchange_copilot_access_token(
            &explicit_field_provider
        ));

        let raw_copilot_token_provider = UpstreamProvider {
            api_key: "copilot_token_can_still_be_used_directly".to_string(),
            ..base_provider
        };
        assert!(!should_exchange_copilot_access_token(
            &raw_copilot_token_provider
        ));
    }

    #[tokio::test]
    async fn copilot_token_exchange_sends_github_token_and_caches_response() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/copilot_internal/v2/token",
            listener.local_addr().unwrap()
        );
        let (request_sender, request_receiver) = mpsc::channel::<String>();
        let expires_at = unix_timestamp_secs() + 3600;
        let response_body =
            format!(r#"{{"token":"copilot_token_from_exchange","expires_at":{expires_at}}}"#);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 2048];
            let read_len = stream.read(&mut buffer).unwrap();
            request_sender
                .send(String::from_utf8_lossy(&buffer[..read_len]).to_string())
                .unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let access_token = format!("gho_{}", Uuid::new_v4().simple());
        let first = exchange_copilot_access_token(&client, &endpoint, &access_token)
            .await
            .unwrap();
        let second = exchange_copilot_access_token(&client, &endpoint, &access_token)
            .await
            .unwrap();

        assert_eq!(first, "copilot_token_from_exchange");
        assert_eq!(second, first);
        let request = request_receiver.recv().unwrap();
        assert!(request
            .to_ascii_lowercase()
            .contains(&format!("authorization: token {access_token}")));
        server.join().unwrap();
    }

    #[test]
    fn copilot_openai_chat_body_normalizes_model_and_sanitizes_orphan_tool_message() {
        let body = apply_outbound_adapter_compat_for_provider_type(
            br#"{
                "model":"claude-sonnet-4-6-20250929[1m]",
                "messages":[
                    {"role":"user","content":"run tool"},
                    {"role":"tool","tool_call_id":"missing","content":"orphan output"},
                    {"role":"assistant","content":[
                        {"type":"thinking","text":"hidden"},
                        {"type":"text","text":"visible"}
                    ]}
                ]
            }"#
            .to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "github_copilot",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["model"], "claude-sonnet-4.6-1m");
        assert_eq!(value["messages"][1]["role"], "user");
        assert!(value["messages"][1].get("tool_call_id").is_none());
        assert_eq!(
            value["messages"][1]["content"],
            "[Tool result for missing]: orphan output"
        );
        let assistant_parts = value["messages"][2]["content"].as_array().unwrap();
        assert_eq!(assistant_parts.len(), 1);
        assert_eq!(assistant_parts[0]["text"], "visible");
    }

    #[test]
    fn copilot_responses_body_normalizes_function_item_ids_and_orphan_outputs() {
        let body = apply_outbound_adapter_compat_for_provider_type(
            br#"{
                "model":"claude-haiku-4-5-20251001",
                "input":[
                    {"type":"function_call","id":"random","call_id":"call_ok","name":"","arguments":"{}"},
                    {"type":"function_call_output","call_id":"missing","output":"orphan result"}
                ],
                "output":[
                    {"type":"function_call","id":"random2","call_id":"call_out","arguments":"{}"}
                ]
            }"#
            .to_vec(),
            None,
            AiProtocol::OpenAiResponses,
            "github-copilot",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert_eq!(value["model"], "claude-haiku-4.5");
        assert_eq!(value["input"][0]["id"], "call_ok");
        assert_eq!(value["input"][0]["name"], "function");
        assert_eq!(value["input"][1]["type"], "message");
        assert_eq!(value["input"][1]["role"], "user");
        assert_eq!(
            value["input"][1]["content"][0]["text"],
            "[Tool result for missing]: orphan result"
        );
        assert_eq!(value["output"][0]["id"], "call_out");
        assert_eq!(value["output"][0]["name"], "function");
    }

    #[test]
    fn copilot_headers_override_forwarded_fingerprint_and_infer_agent_turn() {
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            auth_strategy: ProviderAuthStrategy::Bearer,
            meta: ProviderGatewayMeta {
                provider_type: Some("github_copilot".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::OpenCode)
        };
        let body = br#"{
            "model":"gpt-5",
            "metadata":{"session_id":"session-a"},
            "messages":[
                {"role":"user","content":"Read"},
                {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"done"}
            ]
        }"#;
        let request = debug_request_with_headers(
            body,
            vec![
                ("x-initiator", "user"),
                ("User-Agent", "old-agent"),
                ("x-request-id", "old-request"),
            ],
        );
        let headers = build_upstream_headers(&request, &provider, Some(body)).unwrap();

        assert_eq!(
            headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok()),
            Some("GitHubCopilotChat/0.38.2")
        );
        assert_eq!(
            headers
                .get("x-initiator")
                .and_then(|value| value.to_str().ok()),
            Some("agent")
        );
        assert!(headers.get("x-interaction-id").is_some());
        assert!(headers.get("x-request-id").is_some());
        assert!(headers.get("x-agent-task-id").is_some());

        let preserved_user_agents = headers
            .preserved
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("user-agent"))
            .count();
        assert_eq!(preserved_user_agents, 1);
    }

    #[test]
    fn copilot_headers_detect_compact_subagent_and_vision() {
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::OpenAiChat,
            auth_strategy: ProviderAuthStrategy::Bearer,
            meta: ProviderGatewayMeta {
                provider_type: Some("copilot".to_string()),
                ..ProviderGatewayMeta::default()
            },
            ..provider_for_cli(GatewayCliKey::OpenCode)
        };
        let body = br#"{
            "model":"gpt-5",
            "system":"You are a helpful AI assistant tasked with summarizing conversations",
            "metadata":{"user_id":"main_agent_sub"},
            "messages":[
                {"role":"user","content":[
                    {"type":"text","text":"__SUBAGENT_MARKER__"},
                    {"type":"image_url","image_url":{"url":"data:image/png;base64,abc"}}
                ]}
            ]
        }"#;
        let headers = build_upstream_headers(&debug_request(body), &provider, Some(body)).unwrap();

        assert_eq!(
            headers
                .get("x-initiator")
                .and_then(|value| value.to_str().ok()),
            Some("agent")
        );
        assert_eq!(
            headers
                .get("x-interaction-type")
                .and_then(|value| value.to_str().ok()),
            Some("conversation-subagent")
        );
        assert_eq!(
            headers
                .get("copilot-vision-request")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
    }

    #[test]
    fn provider_body_compat_codex_official_responses_forces_required_fields() {
        let body = json!({
            "model":"gpt-5.1-codex-mini",
            "input":"hi",
            "stream":false,
            "store":true,
            "parallel_tool_calls":false,
            "max_tokens":2048,
            "max_completion_tokens":4096,
            "metadata":{"session_id":"session-a"},
            "include":["file_search_call.results"],
            "reasoning":{"effort":"high"}
        });
        let body = apply_outbound_adapter_compat_for_provider_type(
            serde_json::to_vec(&body).unwrap(),
            None,
            AiProtocol::OpenAiResponses,
            "openai-codex",
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["stream"], true);
        assert_eq!(value["store"], false);
        assert_eq!(value["parallel_tool_calls"], true);
        assert!(value.get("max_tokens").is_none());
        assert!(value.get("max_completion_tokens").is_none());
        assert!(value.get("metadata").is_none());
        assert_eq!(
            value["include"],
            json!(["file_search_call.results", "reasoning.encrypted_content"])
        );
        assert_eq!(value["reasoning"]["effort"], "high");
        assert_eq!(value["reasoning"]["summary"], "auto");
    }

    #[test]
    fn openai_responses_sse_aggregation_only_for_non_streaming_clients() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let mut provider = provider_for_cli(GatewayCliKey::Codex);
        provider.meta.provider_type = Some("openai-codex".to_string());

        assert!(
            should_aggregate_openai_responses_sse_for_non_streaming_client(
                &debug_request(br#"{"model":"gpt-5.1-codex-mini","input":"hi","stream":false}"#),
                &route,
                &provider,
                &headers,
                200,
            )
        );
        assert!(
            !should_aggregate_openai_responses_sse_for_non_streaming_client(
                &debug_request(br#"{"model":"gpt-5.1-codex-mini","input":"hi","stream":true}"#),
                &route,
                &provider,
                &headers,
                200,
            )
        );

        provider.target_protocol = AiProtocol::OpenAiChat;
        assert!(
            !should_aggregate_openai_responses_sse_for_non_streaming_client(
                &debug_request(br#"{"model":"gpt-5.1-codex-mini","input":"hi","stream":false}"#),
                &route,
                &provider,
                &headers,
                200,
            )
        );
    }

    #[tokio::test]
    async fn streaming_first_chunk_validation_rejects_semantically_empty_sse() {
        let stream: DebugBodyStream = Box::pin(futures_util::stream::iter(vec![Ok(concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_empty\",\"object\":\"response\",\"status\":\"in_progress\",\"output\":[]}}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_empty\",\"object\":\"response\",\"status\":\"completed\",\"output\":[]}}\n\n"
        )
        .as_bytes()
        .to_vec())]));
        let mut response = DebugHttpResponse {
            status_code: 200,
            status_text: "OK".to_string(),
            headers: vec![(CONTENT_TYPE.to_string(), "text/event-stream".to_string())],
            body: Vec::new(),
            body_stream: Some(stream),
            response_body_bytes: 0,
            token_usage: TokenUsage::default(),
            first_token_ms: None,
            is_streaming: true,
            cli_key: None,
            route_name: "test".to_string(),
            provider_id: None,
            provider_name: None,
            provider_type: None,
            cost_multiplier: None,
            pricing_model_source: None,
            requested_model: None,
            upstream_model_id: None,
            upstream_request_body: None,
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
            upstream_response_body_stream_snapshot: None,
            upstream_url: None,
            error_category: None,
            attempt_count: 1,
            provider_attempt_count: 1,
            provider_attempts: Vec::new(),
            failover: false,
            note: String::new(),
        };

        let error = validate_streaming_first_chunk(&mut response, 1)
            .await
            .expect_err("semantic empty SSE should fail before commit");
        assert_eq!(error.kind, GatewayFailureKind::EmptyResponse);
        assert!(response.body_stream.is_none());
    }

    #[tokio::test]
    async fn streaming_first_chunk_validation_replays_preread_sse_chunks() {
        let created = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_ok\",\"object\":\"response\",\"status\":\"in_progress\",\"output\":[]}}\n\n"
        )
        .as_bytes()
        .to_vec();
        let delta = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0}\n\n"
        )
        .as_bytes()
        .to_vec();
        let stream: DebugBodyStream = Box::pin(futures_util::stream::iter(vec![
            Ok(created.clone()),
            Ok(delta.clone()),
        ]));
        let mut response = DebugHttpResponse {
            status_code: 200,
            status_text: "OK".to_string(),
            headers: vec![(CONTENT_TYPE.to_string(), "text/event-stream".to_string())],
            body: Vec::new(),
            body_stream: Some(stream),
            response_body_bytes: 0,
            token_usage: TokenUsage::default(),
            first_token_ms: None,
            is_streaming: true,
            cli_key: None,
            route_name: "test".to_string(),
            provider_id: None,
            provider_name: None,
            provider_type: None,
            cost_multiplier: None,
            pricing_model_source: None,
            requested_model: None,
            upstream_model_id: None,
            upstream_request_body: None,
            upstream_response_body: None,
            upstream_response_body_bytes: 0,
            upstream_response_body_stream_snapshot: None,
            upstream_url: None,
            error_category: None,
            attempt_count: 1,
            provider_attempt_count: 1,
            provider_attempts: Vec::new(),
            failover: false,
            note: String::new(),
        };

        validate_streaming_first_chunk(&mut response, 1)
            .await
            .expect("meaningful SSE should pass");
        let mut stream = response.body_stream.take().expect("replayed body stream");
        assert_eq!(stream.next().await.unwrap().unwrap(), created);
        assert_eq!(stream.next().await.unwrap().unwrap(), delta);
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn codex_official_sse_aggregate_uses_completed_response_json() {
        let sse = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"status\":\"in_progress\",\"output\":[]}}\n\n",
            "event: response.output_item.done\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"msg_1\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\"}]}}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"status\":\"completed\",\"model\":\"gpt-5.1-codex-mini\",\"output\":[{\"id\":\"msg_1\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\"}]}],\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"total_tokens\":5}}}\n\n"
        );
        let stream = Box::pin(futures_util::stream::iter(vec![Ok(sse
            .as_bytes()
            .to_vec())]));
        let (raw, body) =
            tauri::async_runtime::block_on(aggregate_openai_responses_sse_stream(stream))
                .expect("aggregate responses sse");
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(raw, sse.as_bytes());
        assert_eq!(value["id"], "resp_1");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["output"][0]["content"][0]["text"], "hello");
        assert_eq!(value["usage"]["total_tokens"], 5);
    }

    #[test]
    fn codex_official_sse_aggregate_falls_back_to_done_items() {
        let sse = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_2\",\"object\":\"response\",\"status\":\"in_progress\",\"output\":[]}}\n\n",
            "event: response.output_item.done\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"status\":\"completed\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"{}\"}}\n\n",
            "data: [DONE]\n\n"
        );
        let stream = Box::pin(futures_util::stream::iter(vec![Ok(sse
            .as_bytes()
            .to_vec())]));
        let (_, body) =
            tauri::async_runtime::block_on(aggregate_openai_responses_sse_stream(stream))
                .expect("aggregate responses sse");
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["id"], "resp_2");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["output"][0]["type"], "function_call");
        assert_eq!(value["output"][0]["call_id"], "call_1");
    }

    #[tokio::test]
    async fn openai_chat_sse_aggregate_builds_chat_completion_json() {
        let sse = concat!(
            "data: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":123,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"think \",\"content\":\"hel\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"query\\\":\"}}]}}]}\n\n",
            "data: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":123,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"done\",\"content\":\"lo\",\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"rust\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":5,\"total_tokens\":8}}\n\n",
            "data: [DONE]\n\n"
        );
        let stream = Box::pin(futures_util::stream::iter(vec![Ok(sse
            .as_bytes()
            .to_vec())]));

        let (raw, body) = aggregate_openai_chat_sse_stream(stream)
            .await
            .expect("aggregate chat sse");
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(raw, sse.as_bytes());
        assert_eq!(value["id"], "chatcmpl_1");
        assert_eq!(value["object"], "chat.completion");
        assert_eq!(value["created"], 123);
        assert_eq!(value["model"], "gpt-4o");
        assert_eq!(value["usage"]["total_tokens"], 8);
        assert_eq!(value["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(value["choices"][0]["message"]["role"], "assistant");
        assert_eq!(value["choices"][0]["message"]["content"], "hello");
        assert_eq!(
            value["choices"][0]["message"]["reasoning_content"],
            "think done"
        );
        assert_eq!(
            value["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "lookup"
        );
        assert_eq!(
            value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{\"query\":\"rust\"}"
        );
    }

    #[tokio::test]
    async fn anthropic_sse_aggregate_builds_message_json() {
        let sse = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet\",\"content\":[],\"usage\":{\"input_tokens\":4}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"think\"}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"lookup\",\"input\":{}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\":\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"rust\\\"}\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":7}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let stream = Box::pin(futures_util::stream::iter(vec![Ok(sse
            .as_bytes()
            .to_vec())]));

        let (raw, body) = aggregate_anthropic_sse_stream(stream)
            .await
            .expect("aggregate anthropic sse");
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(raw, sse.as_bytes());
        assert_eq!(value["id"], "msg_1");
        assert_eq!(value["type"], "message");
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["model"], "claude-sonnet");
        assert_eq!(value["content"][0]["type"], "thinking");
        assert_eq!(value["content"][0]["thinking"], "think");
        assert_eq!(value["content"][1]["text"], "hello");
        assert_eq!(value["content"][2]["type"], "tool_use");
        assert_eq!(value["content"][2]["input"]["query"], "rust");
        assert_eq!(value["stop_reason"], "tool_use");
        assert_eq!(value["usage"]["input_tokens"], 4);
        assert_eq!(value["usage"]["output_tokens"], 7);
    }

    #[tokio::test]
    async fn gemini_sse_aggregate_builds_generate_content_json() {
        let sse = concat!(
            "data: {\"responseId\":\"resp_1\",\"modelVersion\":\"gemini-2.5-flash\",\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"thought\":true,\"text\":\"think \"}]}}]}\n\n",
            "data: {\"responseId\":\"resp_1\",\"modelVersion\":\"gemini-2.5-flash\",\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"thought\":true,\"text\":\"done\",\"thoughtSignature\":\"sig_1\"},{\"text\":\"hel\"}]}}]}\n\n",
            "data: {\"responseId\":\"resp_1\",\"modelVersion\":\"gemini-2.5-flash\",\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"lo\"},{\"functionCall\":{\"name\":\"lookup\",\"args\":{\"query\":\"rust\"}}}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":5,\"totalTokenCount\":8}}\n\n"
        );
        let stream = Box::pin(futures_util::stream::iter(vec![Ok(sse
            .as_bytes()
            .to_vec())]));

        let (raw, body) = aggregate_gemini_sse_stream(stream)
            .await
            .expect("aggregate gemini sse");
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(raw, sse.as_bytes());
        assert_eq!(value["responseId"], "resp_1");
        assert_eq!(value["modelVersion"], "gemini-2.5-flash");
        assert_eq!(value["candidates"][0]["finishReason"], "STOP");
        assert_eq!(value["candidates"][0]["content"]["role"], "model");
        assert_eq!(
            value["candidates"][0]["content"]["parts"][0]["text"],
            "think done"
        );
        assert_eq!(
            value["candidates"][0]["content"]["parts"][0]["thoughtSignature"],
            "sig_1"
        );
        assert_eq!(
            value["candidates"][0]["content"]["parts"][1]["text"],
            "hello"
        );
        assert_eq!(
            value["candidates"][0]["content"]["parts"][2]["functionCall"]["args"]["query"],
            "rust"
        );
        assert_eq!(value["usageMetadata"]["totalTokenCount"], 8);
    }

    #[test]
    fn codex_official_headers_set_originator_accept_and_session() {
        let mut provider = provider_for_cli(GatewayCliKey::Codex);
        provider.meta.provider_type = Some("codex".to_string());
        let request = debug_request_with_headers(
            b"{}",
            vec![(
                "X-Codex-Turn-Metadata",
                r#"{"session_id":"session-from-turn"}"#,
            )],
        );

        let headers = build_upstream_headers(&request, &provider, None).unwrap();

        assert_eq!(
            headers.get(ACCEPT).and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        assert_eq!(
            headers
                .get("originator")
                .and_then(|value| value.to_str().ok()),
            Some("ai-toolbox")
        );
        assert_eq!(
            headers
                .get("session_id")
                .and_then(|value| value.to_str().ok()),
            Some("session-from-turn")
        );
        assert_eq!(
            headers
                .get("x-codex-turn-metadata")
                .and_then(|value| value.to_str().ok()),
            Some(r#"{"session_id":"session-from-turn"}"#)
        );
    }

    #[test]
    fn codex_official_headers_preserve_client_originator_and_session() {
        let mut provider = provider_for_cli(GatewayCliKey::Codex);
        provider.meta.provider_type = Some("chatgpt-codex".to_string());
        let request = debug_request_with_headers(
            b"{}",
            vec![
                ("Originator", "codex_cli_rs"),
                ("Session_id", "client-session"),
                ("Chatgpt-Account-Id", "account-1"),
            ],
        );

        let headers = build_upstream_headers(&request, &provider, None).unwrap();

        assert_eq!(
            headers
                .get("originator")
                .and_then(|value| value.to_str().ok()),
            Some("codex_cli_rs")
        );
        assert_eq!(
            headers
                .get("session_id")
                .and_then(|value| value.to_str().ok()),
            Some("client-session")
        );
        assert_eq!(
            headers
                .get("chatgpt-account-id")
                .and_then(|value| value.to_str().ok()),
            Some("account-1")
        );
    }

    #[test]
    fn codex_chat_reasoning_config_maps_deepseek_effort_and_thinking() {
        let body = br#"{
            "model":"deepseek-reasoner",
            "reasoning_effort":"xhigh",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("deepseek".to_string()),
            codex_chat_reasoning: Some(CodexChatReasoningMeta {
                supports_thinking: Some(true),
                supports_effort: Some(true),
                thinking_param: Some("thinking".to_string()),
                effort_param: Some("reasoning_effort".to_string()),
                effort_value_mode: Some("deepseek".to_string()),
                output_format: Some("reasoning_content".to_string()),
            }),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert_eq!(value["thinking"]["type"], "enabled");
        assert_eq!(value["reasoning_effort"], "max");
    }

    #[test]
    fn codex_chat_reasoning_config_maps_openrouter_effort_object() {
        let body = br#"{
            "model":"openrouter/model",
            "reasoning_effort":"max",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("openrouter".to_string()),
            reasoning_field: Some("reasoning".to_string()),
            codex_chat_reasoning: Some(CodexChatReasoningMeta {
                supports_thinking: Some(true),
                supports_effort: Some(true),
                thinking_param: Some("none".to_string()),
                effort_param: Some("reasoning.effort".to_string()),
                effort_value_mode: Some("openrouter".to_string()),
                output_format: Some("auto".to_string()),
            }),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert!(value.get("reasoning_effort").is_none());
        assert!(value.get("thinking").is_none());
        assert_eq!(value["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn codex_chat_reasoning_config_strips_effort_for_thinking_only_provider() {
        let body = br#"{
            "model":"qwen3-coder",
            "reasoning_effort":"high",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("dashscope".to_string()),
            codex_chat_reasoning: Some(CodexChatReasoningMeta {
                supports_thinking: Some(true),
                supports_effort: Some(false),
                thinking_param: Some("enable_thinking".to_string()),
                effort_param: Some("none".to_string()),
                effort_value_mode: None,
                output_format: Some("reasoning_content".to_string()),
            }),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert!(value.get("reasoning_effort").is_none());
        assert_eq!(value["enable_thinking"], true);
    }

    #[test]
    fn codex_chat_reasoning_infers_deepseek_without_explicit_meta() {
        let body = br#"{
            "model":"deepseek-v4-pro",
            "reasoning_effort":"xhigh",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("deepseek".to_string()),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert_eq!(value["thinking"]["type"], "enabled");
        assert_eq!(value["reasoning_effort"], "max");
    }

    #[test]
    fn codex_chat_reasoning_custom_deepseek_model_does_not_infer_provider_compat() {
        let body = br#"{
            "model":"deepseek-v4-flash",
            "reasoning_effort":"xhigh",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("custom".to_string()),
            api_format: Some("openai_chat".to_string()),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert!(value.get("thinking").is_none());
        assert!(value.get("reasoning_effort").is_none());
    }

    #[test]
    fn codex_chat_reasoning_infers_openrouter_platform_before_model() {
        let body = br#"{
            "model":"deepseek/deepseek-chat-v3.1",
            "reasoning_effort":"max",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("openrouter".to_string()),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert!(value.get("thinking").is_none());
        assert!(value.get("reasoning_effort").is_none());
        assert_eq!(value["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn codex_chat_reasoning_custom_qwen_model_does_not_infer_provider_compat() {
        let body = br#"{
            "model":"qwen3-coder",
            "reasoning_effort":"high",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("custom-openai".to_string()),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert!(value.get("enable_thinking").is_none());
        assert!(value.get("reasoning_effort").is_none());
    }

    #[test]
    fn codex_chat_reasoning_custom_provider_model_names_do_not_infer_provider_compat() {
        for (model, provider_specific_fields) in [
            (
                "MiniMax-M2.7",
                ["reasoning_split", "thinking", "enable_thinking"],
            ),
            (
                "mimo-v2.5-pro",
                ["thinking", "reasoning_split", "enable_thinking"],
            ),
            (
                "kimi-k2.7-code",
                ["thinking", "reasoning_split", "enable_thinking"],
            ),
        ] {
            let body = format!(
                r#"{{
                    "model":"{model}",
                    "reasoning_effort":"high",
                    "messages":[{{"role":"user","content":"hi"}}]
                }}"#
            );
            let meta = ProviderGatewayMeta {
                provider_type: Some("custom-openai".to_string()),
                api_format: Some("openai_chat".to_string()),
                ..ProviderGatewayMeta::default()
            };

            let converted = apply_outbound_adapter_compat_for_provider(
                body.into_bytes(),
                None,
                AiProtocol::OpenAiChat,
                Some(&meta),
            )
            .unwrap();
            let value: Value = serde_json::from_slice(&converted).unwrap();

            for field in provider_specific_fields {
                assert!(value.get(field).is_none(), "{model} should not set {field}");
            }
            assert!(
                value.get("reasoning_effort").is_none(),
                "{model} should not preserve generic reasoning_effort"
            );
        }
    }

    #[test]
    fn codex_chat_reasoning_explicit_meta_overrides_inference() {
        let body = br#"{
            "model":"deepseek-v4-pro",
            "reasoning_effort":"high",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("custom-openai".to_string()),
            codex_chat_reasoning: Some(CodexChatReasoningMeta {
                supports_thinking: Some(true),
                supports_effort: Some(false),
                thinking_param: Some("enable_thinking".to_string()),
                effort_param: Some("none".to_string()),
                effort_value_mode: None,
                output_format: Some("reasoning_content".to_string()),
            }),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&converted).unwrap();

        assert_eq!(value["enable_thinking"], true);
        assert!(value.get("thinking").is_none());
        assert!(value.get("reasoning_effort").is_none());
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
                    "content":"plain reply",
                    "reasoning_content":"plain hidden reasoning",
                    "reasoning":"plain hidden reasoning"
                },
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "deepseek",
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
            value["messages"][2]["tool_calls"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            value["messages"][2]["tool_calls"][0]["id"],
            "call_empty_args"
        );
        assert_eq!(
            value["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
        assert_eq!(value["messages"][2]["tool_calls"][1]["id"], "call_fn");
        assert!(value["messages"][1].get("reasoning_content").is_none());
        assert!(value["messages"][1].get("reasoning").is_none());
        assert_eq!(value["messages"][2]["reasoning_content"], "tool call");
        assert!(value["messages"][2].get("reasoning").is_none());
        assert_eq!(value["messages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn provider_body_compat_openai_chat_applies_reasoning_field_policy() {
        let body = br#"{
            "model":"openrouter/deepseek-reasoner",
            "messages":[
                {"role":"user","content":"hi"},
                {"role":"assistant","content":"ok","reasoning_content":"hidden"},
                {"role":"assistant","content":"ok","reasoning":"alternate"}
            ]
        }"#;
        let meta = ProviderGatewayMeta {
            provider_type: Some("openrouter".to_string()),
            ..ProviderGatewayMeta::default()
        };

        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&converted).unwrap();
        assert!(value["messages"][1].get("reasoning_content").is_none());
        assert_eq!(value["messages"][1]["reasoning"], "hidden");
        assert!(value["messages"][2].get("reasoning_content").is_none());
        assert_eq!(value["messages"][2]["reasoning"], "alternate");

        let meta = ProviderGatewayMeta {
            reasoning_field: Some("none".to_string()),
            ..ProviderGatewayMeta::default()
        };
        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&converted).unwrap();
        assert!(value["messages"][1].get("reasoning_content").is_none());
        assert!(value["messages"][1].get("reasoning").is_none());
        assert!(value["messages"][2].get("reasoning_content").is_none());
        assert!(value["messages"][2].get("reasoning").is_none());

        let meta = ProviderGatewayMeta {
            reasoning_field: Some("all".to_string()),
            ..ProviderGatewayMeta::default()
        };
        let converted = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&converted).unwrap();
        assert_eq!(value["messages"][1]["reasoning_content"], "hidden");
        assert_eq!(value["messages"][1]["reasoning"], "hidden");
        assert_eq!(value["messages"][2]["reasoning_content"], "alternate");
        assert_eq!(value["messages"][2]["reasoning"], "alternate");
    }

    #[test]
    fn provider_pipeline_runs_outbound_adapter_middleware() {
        let meta = ProviderGatewayMeta {
            reasoning_field: Some("reasoning".to_string()),
            ..ProviderGatewayMeta::default()
        };
        let pipeline_context = build_pipeline_context(Some(&meta), AiProtocol::OpenAiChat);
        let mut body = json!({
            "model": "openrouter/deepseek-reasoner",
            "messages": [
                {"role": "assistant", "content": "ok", "reasoning_content": "hidden"}
            ]
        });

        let pipeline = build_provider_pipeline(Some(&meta), None, AiProtocol::OpenAiChat, false);
        pipeline
            .run_outbound_body(&mut body, &pipeline_context)
            .unwrap();

        assert!(body["messages"][0].get("reasoning_content").is_none());
        assert_eq!(body["messages"][0]["reasoning"], "hidden");
    }

    #[test]
    fn provider_body_compat_openrouter_moves_reasoning_effort_to_reasoning_object() {
        let body = br#"{
            "model":"openrouter/deepseek-reasoner",
            "reasoning_effort":"max",
            "messages":[{"role":"user","content":"hi"}]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "openrouter",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("reasoning_effort").is_none());
        assert_eq!(value["reasoning"]["effort"], "xhigh");

        let body = br#"{
            "model":"openrouter/deepseek-reasoner",
            "reasoning_effort":"disabled",
            "reasoning":{"exclude":false},
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "open-router",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("reasoning_effort").is_none());
        assert_eq!(value["reasoning"]["effort"], "none");
        assert_eq!(value["reasoning"]["exclude"], false);
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "zhipu",
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
        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "glm",
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
            "reasoning_effort":"high",
            "metadata":{"user_id":"user-1"},
            "messages":[{"role":"user","content":"hi"}]
        }"#;

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "doubao",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();

        assert!(value.get("metadata").is_none());
        assert_eq!(value["user_id"], "user-1");
        assert_eq!(value["thinking"]["type"], "enabled");
        assert!(value.get("reasoning_effort").is_none());
        assert!(value["request_id"]
            .as_str()
            .is_some_and(|request_id| request_id.starts_with("req_")));

        let body = br#"{
            "model":"doubao-seed-code",
            "reasoning_effort":"none",
            "messages":[{"role":"user","content":"hi"}]
        }"#;
        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "doubao-seed",
        )
        .unwrap();
        let value = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(value["thinking"]["type"], "disabled");
        assert!(value.get("reasoning_effort").is_none());
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "grok",
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
    fn xai_stream_filter_drops_empty_delta_chunks() {
        let mut filter = XaiChatStreamFilterState::default();
        let chunks = filter.push_chunk(
            br#"data: {"id":"role_1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}

data: {"id":"empty_1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":null}]}

data: {"id":"content_1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]}

data: {"id":"finish_1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: {"id":"usage_1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}

data: [DONE]

"#,
        );
        let output = String::from_utf8(
            chunks
                .iter()
                .flat_map(|chunk| chunk.iter().copied())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let values = sse_json_values(chunks);

        assert!(!output.contains("empty_1"));
        assert!(output.contains("role_1"));
        assert!(output.contains("content_1"));
        assert!(output.contains("finish_1"));
        assert!(output.contains("usage_1"));
        assert!(output.contains("[DONE]"));
        assert_eq!(values.len(), 4);
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "longcat",
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "bailian",
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            "aliyun",
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
    fn bailian_stream_filter_buffers_text_after_tool_calls_until_finish() {
        let mut filter = BailianChatStreamFilterState::default();
        let out = filter.push_chunk(
            br#"data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"qwen3","choices":[{"index":0,"delta":{"role":"assistant"}}]}

data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"qwen3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]}}]}

data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"qwen3","choices":[{"index":0,"delta":{"content":"text after tool"}}]}

data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"qwen3","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
        );
        let values = sse_json_values(out);

        assert_eq!(values.len(), 5);
        assert_eq!(
            values[1]["choices"][0]["delta"]["tool_calls"][0]["id"],
            "call_1"
        );
        assert!(values[2]["choices"][0]["delta"].get("content").is_none());
        assert_eq!(
            values[3]["choices"][0]["delta"]["content"],
            "text after tool"
        );
        assert_eq!(values[4]["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn bailian_stream_filter_drops_duplicate_empty_tool_arguments() {
        let mut filter = BailianChatStreamFilterState::default();
        let out = filter.push_chunk(
            br#"data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"qwen3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]}}]}

data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"qwen3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}}]}

"#,
        );
        let values = sse_json_values(out);

        assert_eq!(
            values[0]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"README.md\"}"
        );
        assert_eq!(
            values[1]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            ""
        );
    }

    fn sse_json_values(chunks: Vec<Vec<u8>>) -> Vec<Value> {
        chunks
            .into_iter()
            .filter_map(|chunk| {
                let text = String::from_utf8(chunk).ok()?;
                let data = sse_data_payload(&text)?;
                serde_json::from_str::<Value>(&data).ok()
            })
            .collect()
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::AnthropicMessages,
            "moonshot",
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

        let body = apply_outbound_adapter_compat_for_provider_type(
            body.to_vec(),
            None,
            AiProtocol::AnthropicMessages,
            "deepseek",
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
            None,
            None,
            false,
            CodexResponsesCompactCompat::none(),
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
    fn responses_encrypted_content_rectifier_matches_only_verification_errors() {
        assert!(should_rectify_responses_encrypted_content(
            400,
            br#"{"error":{"message":"The encrypted content gAAAA... could not be verified. Reason: Encrypted content could not be decrypted or parsed."}}"#
        ));
        assert!(should_rectify_responses_encrypted_content(
            400,
            r#"{"error":{"message":"无法验证请求中的加密内容","type":"invalid_request_error","code":"invalid_encrypted_content"}}"#.as_bytes()
        ));
        assert!(should_rectify_responses_encrypted_content(
            400,
            r#"{"message":"暗号化された内容を検証できません","code":"invalid_encrypted_content"}"#
                .as_bytes()
        ));
        assert!(should_rectify_responses_encrypted_content(
            422,
            br#"{"error":{"message":"invalid request field","param":"encrypted_content"}}"#
        ));
        assert!(should_rectify_responses_encrypted_content(
            422,
            br#"{"detail":"encrypted_content could not be decrypted"}"#
        ));

        assert!(!should_rectify_responses_encrypted_content(
            400,
            r#"{"error":{"message":"当前模型 gpt-5.6-sol 负载已经达到上限，请稍后重试","type":"new_api_error","code":"get_channel_failed"}}"#.as_bytes()
        ));
        assert!(!should_rectify_responses_encrypted_content(
            400,
            r#"{"error":{"message":"请求无效","type":"invalid_request_error"}}"#.as_bytes()
        ));
        assert!(!should_rectify_responses_encrypted_content(
            400,
            br#"{"error":{"message":"Invalid JSON schema: strict must be a boolean"}}"#
        ));
        assert!(!should_rectify_responses_encrypted_content(
            500,
            br#"{"error":{"message":"The encrypted content could not be verified"}}"#
        ));
    }

    #[test]
    fn responses_encrypted_content_rectifier_removes_only_encrypted_reasoning_items() {
        let body = br#"{
            "model":"gpt-5.6-sol",
            "store":false,
            "include":["reasoning.encrypted_content","file_search_call.results"],
            "reasoning":{"effort":"xhigh","summary":"detailed"},
            "input":[
                {"type":"message","role":"user","content":[{"type":"input_text","text":"inspect"}]},
                {"type":"reasoning","summary":[{"type":"summary_text","text":"private plan"}],"encrypted_content":"gAAAA-old"},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"visible"}]},
                {"type":"reasoning","summary":[{"type":"summary_text","text":"public summary only"}]},
                {"type":"reasoning","summary":[],"encrypted_content":"   "},
                {"type":"function_call","call_id":"call_1","name":"shell","arguments":"{}"},
                {"type":"function_call_output","call_id":"call_1","output":"ok"},
                {"type":"custom_tool_call","call_id":"custom_1","name":"apply_patch","input":"patch"},
                {"type":"custom_tool_call_output","call_id":"custom_1","output":"done"}
            ]
        }"#;

        let rectified = build_responses_encrypted_content_rectified_body(body)
            .unwrap()
            .expect("encrypted reasoning should be removed");
        let value = serde_json::from_slice::<Value>(&rectified).unwrap();
        let input = value["input"].as_array().unwrap();

        assert_eq!(input.len(), 8);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[1]["content"][0]["text"], "visible");
        assert_eq!(input[2]["type"], "reasoning");
        assert_eq!(input[2]["summary"][0]["text"], "public summary only");
        assert_eq!(input[3]["type"], "reasoning");
        assert_eq!(input[3]["encrypted_content"], "   ");
        assert_eq!(input[4]["type"], "function_call");
        assert_eq!(input[5]["type"], "function_call_output");
        assert_eq!(input[6]["type"], "custom_tool_call");
        assert_eq!(input[7]["type"], "custom_tool_call_output");
        assert_eq!(value["store"], false);
        assert_eq!(
            value["include"],
            json!(["reasoning.encrypted_content", "file_search_call.results"])
        );
        assert_eq!(value["reasoning"]["effort"], "xhigh");
        assert_eq!(value["reasoning"]["summary"], "detailed");
        assert!(!value.to_string().contains("private plan"));
        assert!(!value.to_string().contains("gAAAA-old"));
    }

    #[test]
    fn responses_encrypted_content_negative_cache_filters_only_rejected_provider_ciphers() {
        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), None, None);
        let provider = provider_for_cli(GatewayCliKey::Codex);
        let rejected_body = serde_json::to_vec(&json!({
            "input": [{"type":"reasoning","encrypted_content":"cipher-old"}]
        }))
        .unwrap();
        context.side_stores.remember_rejected_responses_ciphers(
            responses_cipher_provider_config_identity(&provider),
            &rejected_body,
            "encrypted_content could not be decrypted",
        );

        let mut next_body = serde_json::to_vec(&json!({
            "model":"gpt-5",
            "input":[
                {"type":"reasoning","encrypted_content":"cipher-old"},
                {"type":"reasoning","encrypted_content":"cipher-new"},
                {"type":"reasoning","summary":[{"type":"summary_text","text":"keep"}]},
                {"type":"message","role":"user","content":"hello"}
            ]
        }))
        .unwrap();
        assert_eq!(
            strip_known_invalid_responses_ciphers(&context, &provider, &mut next_body).unwrap(),
            1
        );
        let next_value = serde_json::from_slice::<Value>(&next_body).unwrap();
        assert_eq!(next_value["input"].as_array().unwrap().len(), 3);
        assert_eq!(next_value["input"][0]["encrypted_content"], "cipher-new");
        assert_eq!(next_value["input"][1]["type"], "reasoning");
        assert_eq!(next_value["input"][2]["type"], "message");

        let mut other_provider = provider.clone();
        other_provider.id = "provider-b".to_string();
        let mut other_body = rejected_body.clone();
        assert_eq!(
            strip_known_invalid_responses_ciphers(&context, &other_provider, &mut other_body)
                .unwrap(),
            0
        );

        let mut reconfigured_provider = provider.clone();
        reconfigured_provider.base_url = "https://other.example.com/v1".to_string();
        let mut reconfigured_body = rejected_body;
        assert_eq!(
            strip_known_invalid_responses_ciphers(
                &context,
                &reconfigured_provider,
                &mut reconfigured_body,
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn chat_to_responses_conversion_does_not_forge_encrypted_content() {
        let encrypted_content = concat!("g", "AAAAABfixture-openai-responses");
        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-5.6-sol",
            "messages": [
                {"role": "user", "content": "inspect"},
                {
                    "role": "assistant",
                    "reasoning_content": "private plan",
                    "reasoning_signature": encrypted_content,
                    "content": "visible"
                },
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "shell", "arguments": "{}"}
                    }]
                },
                {"role": "tool", "tool_call_id": "call_1", "content": "ok"}
            ],
            "store": false,
            "include": ["reasoning.encrypted_content"],
            "reasoning": {"effort": "xhigh", "summary": "detailed"}
        }))
        .unwrap();
        let request = debug_request(&request_body);
        let conversion_route = Some(ConversionRoute::new(
            AiProtocol::OpenAiChat,
            AiProtocol::OpenAiResponses,
        ));
        let original_body = build_upstream_body(
            &request,
            "gpt-5.6-sol",
            "gpt-5.6-sol",
            false,
            false,
            GatewayCliKey::Codex,
            AiProtocol::OpenAiResponses,
            conversion_route,
            false,
        )
        .unwrap();
        let original = serde_json::from_slice::<Value>(&original_body).unwrap();

        let input = original["input"].as_array().unwrap();
        assert!(!original.to_string().contains(encrypted_content));
        assert!(!input
            .iter()
            .any(|item| item.get("encrypted_content").is_some()));
        assert!(original.to_string().contains("private plan"));
        assert!(original.to_string().contains("visible"));
        assert!(input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && item.get("call_id").and_then(Value::as_str) == Some("call_1")
        }));
        assert!(input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call_output")
                && item.get("call_id").and_then(Value::as_str) == Some("call_1")
        }));
        assert!(
            build_responses_encrypted_content_rectified_body(&original_body)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn responses_encrypted_content_rectifier_skips_unrelated_bodies() {
        assert!(build_responses_encrypted_content_rectified_body(
            br#"{"model":"gpt-5","input":[{"type":"message","role":"user","content":"hi"}]}"#
        )
        .unwrap()
        .is_none());
        assert!(
            build_responses_encrypted_content_rectified_body(b"not-json")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn responses_encrypted_content_rectifier_uses_its_independent_runtime_switch() {
        let request = debug_request(
            br#"{"model":"gpt-5","input":[{"type":"reasoning","encrypted_content":"gAAAA-old"}],"stream":true}"#,
        );
        let route = gateway_route(GatewayCliKey::Codex, "/v1/responses");
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let thinking_rectifier_enabled = false;
        let responses_encrypted_content_rectifier_enabled = true;
        assert!(!thinking_rectifier_enabled);
        assert!(should_attempt_responses_encrypted_content_rectifier(
            responses_encrypted_content_rectifier_enabled,
            AiProtocol::OpenAiResponses,
            &request,
            &route,
            &headers,
            400,
        ));
        let thinking_rectifier_enabled = true;
        let responses_encrypted_content_rectifier_enabled = false;
        assert!(thinking_rectifier_enabled);
        assert!(!should_attempt_responses_encrypted_content_rectifier(
            responses_encrypted_content_rectifier_enabled,
            AiProtocol::OpenAiResponses,
            &request,
            &route,
            &headers,
            400,
        ));
        assert!(!should_attempt_responses_encrypted_content_rectifier(
            true,
            AiProtocol::OpenAiChat,
            &request,
            &route,
            &headers,
            400,
        ));
        assert!(!should_attempt_responses_encrypted_content_rectifier(
            true,
            AiProtocol::OpenAiResponses,
            &request,
            &route,
            &headers,
            500,
        ));
        assert!(should_filter_known_invalid_responses_ciphers(
            true,
            AiProtocol::OpenAiResponses,
            CodexResponsesCompactCompat::none(),
        ));
        assert!(!should_filter_known_invalid_responses_ciphers(
            false,
            AiProtocol::OpenAiResponses,
            CodexResponsesCompactCompat::none(),
        ));
        assert!(!should_filter_known_invalid_responses_ciphers(
            true,
            AiProtocol::OpenAiChat,
            CodexResponsesCompactCompat::none(),
        ));

        let compact_route = gateway_route(GatewayCliKey::Codex, "/v1/responses/compact");
        let compact_provider = provider_for_cli(GatewayCliKey::Codex);
        let compact_compat = CodexResponsesCompactCompat::new(&compact_route, &compact_provider);
        assert!(!should_attempt_responses_encrypted_content_rectifier(
            true,
            AiProtocol::OpenAiResponses,
            &request,
            &compact_route,
            &headers,
            400,
        ));
        assert!(!should_filter_known_invalid_responses_ciphers(
            true,
            AiProtocol::OpenAiResponses,
            compact_compat,
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
    fn predictive_media_policy_replaces_images_for_explicit_text_only_model() {
        let meta = ProviderGatewayMeta {
            text_only_models: vec!["deepseek-chat".to_string()],
            ..ProviderGatewayMeta::default()
        };
        let body = br#"{
            "model":"deepseek-chat",
            "messages":[{"role":"user","content":[
                {"type":"text","text":"describe"},
                {"type":"image_url","image_url":{"url":"https://example.com/a.png"}}
            ]}]
        }"#;

        let next = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .expect("media policy should serialize");
        let value = serde_json::from_slice::<Value>(&next).unwrap();

        assert_eq!(value["messages"][0]["content"][1]["type"], "text");
        assert_eq!(
            value["messages"][0]["content"][1]["text"],
            UNSUPPORTED_IMAGE_MARKER
        );
    }

    #[test]
    fn predictive_media_policy_preserves_images_for_image_capable_model() {
        let meta = ProviderGatewayMeta {
            text_only_models: vec!["deepseek-chat".to_string()],
            image_capable_models: vec!["deepseek-chat".to_string()],
            allow_text_only_model_heuristic: true,
            ..ProviderGatewayMeta::default()
        };
        let body = br#"{
            "model":"deepseek-chat",
            "messages":[{"role":"user","content":[
                {"type":"image_url","image_url":{"url":"https://example.com/a.png"}}
            ]}]
        }"#;

        let next = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&meta),
        )
        .expect("media policy should serialize");
        let value = serde_json::from_slice::<Value>(&next).unwrap();

        assert_eq!(value["messages"][0]["content"][0]["type"], "image_url");
    }

    #[test]
    fn predictive_media_policy_uses_heuristic_only_when_enabled() {
        let disabled_meta = ProviderGatewayMeta {
            allow_text_only_model_heuristic: false,
            ..ProviderGatewayMeta::default()
        };
        let enabled_meta = ProviderGatewayMeta {
            allow_text_only_model_heuristic: true,
            ..ProviderGatewayMeta::default()
        };
        let body = br#"{
            "model":"qwen3-coder-plus",
            "messages":[{"role":"user","content":[
                {"type":"image_url","image_url":{"url":"https://example.com/a.png"}}
            ]}]
        }"#;

        let disabled = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&disabled_meta),
        )
        .expect("disabled heuristic should serialize");
        let disabled_value = serde_json::from_slice::<Value>(&disabled).unwrap();
        assert_eq!(
            disabled_value["messages"][0]["content"][0]["type"],
            "image_url"
        );

        let enabled = apply_outbound_adapter_compat_for_provider(
            body.to_vec(),
            None,
            AiProtocol::OpenAiChat,
            Some(&enabled_meta),
        )
        .expect("enabled heuristic should serialize");
        let enabled_value = serde_json::from_slice::<Value>(&enabled).unwrap();
        assert_eq!(enabled_value["messages"][0]["content"][0]["type"], "text");
        assert_eq!(
            enabled_value["messages"][0]["content"][0]["text"],
            UNSUPPORTED_IMAGE_MARKER
        );
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
    fn gemini_native_conversion_uses_provider_base_api_version() {
        let route = gateway_route(GatewayCliKey::Claude, "/v1/messages");
        let provider = UpstreamProvider {
            target_protocol: AiProtocol::GeminiNative,
            auth_strategy: ProviderAuthStrategy::GoogleApiKey,
            base_url: "https://generativelanguage.googleapis.com/v1".to_string(),
            ..provider_for_cli(GatewayCliKey::Claude)
        };
        let source_protocol = source_protocol_from_route(&route).unwrap();
        let conversion = conversion_route(source_protocol, &provider);

        assert_eq!(
            upstream_forwarded_path(&route, &provider, conversion, "gemini-2.5-pro", false)
                .as_ref(),
            "/v1/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn gemini_direct_route_rewrites_version_to_provider_base_url() {
        let route = gateway_route(
            GatewayCliKey::Gemini,
            "/v1beta/models/gemini-2.5-pro[1M]:generateContent",
        );
        let provider = UpstreamProvider {
            base_url: "https://generativelanguage.googleapis.com/v1".to_string(),
            ..provider_for_cli(GatewayCliKey::Gemini)
        };
        let forwarded_path = upstream_forwarded_path(&route, &provider, None, "ignored", false);
        let url = build_provider_target_url(
            &provider,
            forwarded_path.as_ref(),
            route.query.as_deref(),
            None,
            false,
            "gemini-2.5-pro",
        )
        .unwrap();

        assert_eq!(
            forwarded_path.as_ref(),
            "/v1/models/gemini-2.5-pro:generateContent"
        );
        assert_eq!(
            url.as_str(),
            "https://generativelanguage.googleapis.com/v1/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn gemini_route_accepts_v1_requests() {
        let route = match_gateway_route("/gemini/v1/models/gemini-2.5-pro:generateContent")
            .expect("Gemini v1 route should be accepted");

        assert_eq!(route.cli_key, GatewayCliKey::Gemini);
        assert_eq!(
            route.forwarded_path,
            "/v1/models/gemini-2.5-pro:generateContent"
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
            "gemini-2.5-flash",
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
    fn payment_required_is_retryable_provider_auth_failure() {
        assert_eq!(
            classify_status_failure(402),
            Some(GatewayFailureKind::Auth)
        );
        assert!(should_retry_failure(GatewayFailureKind::Auth));
        let weight = model_health::classify_failure(GatewayFailureKind::Auth);
        assert_eq!(weight.scope, model_health::FailureScope::Provider);
        assert_eq!(weight.score, 5);
        assert_eq!(weight.category, "auth");
    }

    #[test]
    fn current_provider_retry_respects_per_provider_and_global_limits() {
        assert!(can_retry_current_provider(
            GatewayFailureKind::Connection,
            0,
            1,
            0,
            3
        ));
        assert!(!can_retry_current_provider(
            GatewayFailureKind::Connection,
            1,
            1,
            1,
            3
        ));
        assert!(!can_retry_current_provider(
            GatewayFailureKind::Connection,
            0,
            1,
            3,
            3
        ));
        assert!(!can_retry_current_provider(
            GatewayFailureKind::Timeout,
            0,
            1,
            0,
            3
        ));
    }

    #[test]
    fn retryable_status_codes_gate_status_driven_retry() {
        let defaults =
            crate::coding::proxy_gateway::retryable_status::retryable_status_code_set(
                crate::coding::proxy_gateway::retryable_status::DEFAULT_RETRYABLE_STATUS_CODES_COMPACT,
            )
            .expect("default set");
        assert!(should_retry_status_or_kind(
            Some(400),
            GatewayFailureKind::UpstreamBadRequest,
            &defaults
        ));
        assert!(should_retry_status_or_kind(
            Some(429),
            GatewayFailureKind::RateLimit,
            &defaults
        ));
        assert!(!should_retry_status_or_kind(
            Some(422),
            GatewayFailureKind::RequestSchema,
            &defaults
        ));

        let conservative =
            crate::coding::proxy_gateway::retryable_status::retryable_status_code_set(
                "429,500-599",
            )
            .expect("conservative set");
        assert!(!should_retry_status_or_kind(
            Some(400),
            GatewayFailureKind::UpstreamBadRequest,
            &conservative
        ));
        assert!(should_retry_status_or_kind(
            Some(503),
            GatewayFailureKind::Upstream5xx,
            &conservative
        ));
        // Non-status failures stay on historical kind rules.
        assert!(should_retry_status_or_kind(
            None,
            GatewayFailureKind::Connection,
            &conservative
        ));
        assert!(should_retry_status_or_kind(
            Some(200),
            GatewayFailureKind::EmptyResponse,
            &conservative
        ));
    }
}
