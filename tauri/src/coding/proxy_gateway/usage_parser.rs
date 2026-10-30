use super::types::{normalize_service_tier, GatewayCliKey};
use serde_json::Value;

const MAX_SSE_USAGE_BUFFER_BYTES: usize = 256 * 1024;
const MAX_SSE_EVENT_BUFFER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    /// Upstream envelope id when available (Claude message id, OpenAI/Codex response id,
    /// Gemini responseId). Used to build stable usage request keys.
    pub envelope_id: Option<String>,
}

impl TokenUsage {
    pub fn total_tokens(&self) -> Option<u64> {
        let input = self.input_tokens.unwrap_or(0);
        let output = self.output_tokens.unwrap_or(0);
        let cache_read = self.cache_read_tokens.unwrap_or(0);
        let cache_creation = self.cache_creation_tokens.unwrap_or(0);
        let total = input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_creation);
        (total > 0).then_some(total)
    }

    pub(crate) fn merge_max(&mut self, other: TokenUsage) {
        self.input_tokens = max_option(self.input_tokens, other.input_tokens);
        self.output_tokens = max_option(self.output_tokens, other.output_tokens);
        self.cache_read_tokens = max_option(self.cache_read_tokens, other.cache_read_tokens);
        self.cache_creation_tokens =
            max_option(self.cache_creation_tokens, other.cache_creation_tokens);
        if self.envelope_id.is_none() {
            self.envelope_id = other.envelope_id;
        }
    }
}

/// Build a stable usage `request_id` from an upstream envelope id.
///
/// Claude keeps bare `SESSION:{id}` so proxy rows converge with session JSONL import.
/// Other CLIs scope by app + provider to avoid cross-provider envelope collisions.
pub fn stable_usage_request_id(
    cli_key: GatewayCliKey,
    provider_id: Option<&str>,
    envelope_id: Option<&str>,
    fallback: &str,
) -> String {
    let Some(envelope_id) = envelope_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return fallback.to_string();
    };
    match cli_key {
        GatewayCliKey::Claude => format!("SESSION:{envelope_id}"),
        other => {
            let provider = provider_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown");
            format!("SESSION:{}:{provider}:{envelope_id}", other.as_str())
        }
    }
}

/// Canonical service tier an upstream confirmed in a protocol envelope.
///
/// Accepts both a bare response object (`/service_tier`, Chat and non-streaming
/// Responses) and a Responses stream event that nests the snapshot under
/// `response` (`/response/service_tier`). Values the gateway does not bill —
/// including the explicit `default` an upstream that never implements the echo
/// returns — map to `None`, so callers fall back to the requested tier instead
/// of silently dropping the premium.
pub fn served_service_tier(body: &[u8]) -> Option<&'static str> {
    let value: Value = serde_json::from_slice(body).ok()?;
    service_tier_from_value(&value)
}

fn service_tier_from_value(value: &Value) -> Option<&'static str> {
    ["/service_tier", "/response/service_tier"]
        .iter()
        .find_map(|path| value.pointer(path)?.as_str())
        .and_then(normalize_service_tier)
}

#[derive(Debug)]
pub struct SseUsageCollector {
    buffer: Vec<u8>,
    next_flattened_scan_at: usize,
    discarding_oversized_block: bool,
    usage: TokenUsage,
    provider_type: Option<String>,
    /// The protocol-level terminal event kind observed in any SSE block pushed
    /// so far (first-wins: terminal events are mutually exclusive). Mirrors
    /// axonhub's `IsTerminalStreamEvent` so the gateway can decide "did the
    /// stream actually end" independent of the already-written 200 status code.
    /// Distinct from `GatewayStreamOutcome`: this is *what the upstream terminal
    /// said*, which feeds `classify_stream_outcome` together with write success.
    terminal_kind: Option<SseTerminalKind>,
    /// Last canonical service tier seen in the stream (last-wins: the terminal
    /// snapshot is the upstream's final word, while `response.created` can still
    /// carry the requested tier).
    service_tier: Option<&'static str>,
}

/// The protocol-level terminal event kind carried by an SSE block. Required so
/// `response.completed` + `response.status = "incomplete"` (the gateway's own
/// cross-protocol Responses-incomplete signal, see
/// `transformer/stream.rs:2351`) maps to `Incomplete`, not `Completed` — two
/// booleans (`terminal_event_seen` + `error_event_seen`) cannot express the four
/// distinct terminal outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseTerminalKind {
    /// `message_stop` / `[DONE]` / `response.completed` (normal) /
    /// non-empty `finish_reason` / `finishReason` / `speech.audio.done` /
    /// `transcript.text.done`.
    Success,
    /// `error` / `response.failed` / `response.completed`+`status=failed` /
    /// non-empty top-level `error` / nested `response.error` / `status=failed`.
    Failed,
    /// `response.incomplete` / `response.completed`+`status=incomplete` /
    /// `status=incomplete`.
    Incomplete,
    /// `response.cancelled` / `response.canceled` / `status=cancelled|canceled`.
    Canceled,
}

impl Default for SseUsageCollector {
    fn default() -> Self {
        Self::with_provider_type(None)
    }
}

impl SseUsageCollector {
    pub fn with_provider_type(provider_type: Option<&str>) -> Self {
        Self {
            buffer: Vec::new(),
            next_flattened_scan_at: MAX_SSE_USAGE_BUFFER_BYTES,
            discarding_oversized_block: false,
            usage: TokenUsage::default(),
            provider_type: provider_type
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            terminal_kind: None,
            service_tier: None,
        }
    }

    pub fn push_chunk(&mut self, cli_key: GatewayCliKey, chunk: &[u8]) {
        self.ingest_chunk(Some(cli_key), chunk);
    }

    pub fn finish(mut self, cli_key: GatewayCliKey) -> TokenUsage {
        if !self.buffer.is_empty() && !self.discarding_oversized_block {
            let block = std::mem::take(&mut self.buffer);
            self.observe_block(&block);
            if let Some(value) = parse_sse_data_block(&block) {
                self.merge_event(cli_key, &value);
            }
            // Degenerate flattened framing never forms a `\n\n` block, so the
            // trailing events above are invisible to `parse_sse_data_block`.
            // Scan them again with the flattened reader so terminal events and
            // usage delivered in the residual buffer are still captured.
            self.merge_flattened_usage(cli_key, &block);
        }
        self.usage
    }

    /// Whether a terminal event has been observed in any pushed SSE block.
    pub fn terminal_event_seen(&self) -> bool {
        self.terminal_kind.is_some()
    }

    /// Whether an error terminal event (`error` / `response.failed`) has been
    /// observed. These end the stream properly — the client received an explicit
    /// protocol error — but the request is a failure, not a success.
    pub fn error_event_seen(&self) -> bool {
        matches!(self.terminal_kind, Some(SseTerminalKind::Failed))
    }

    /// The terminal event kind observed so far, if any. Drives
    /// `classify_stream_outcome` so the four distinct terminal outcomes
    /// (Success / Failed / Incomplete / Canceled) map to the right
    /// `GatewayStreamOutcome` instead of collapsing into two booleans.
    pub fn terminal_kind(&self) -> Option<SseTerminalKind> {
        self.terminal_kind
    }

    /// Canonical service tier the upstream confirmed while streaming, if any.
    /// MUST be read before [`Self::finish`], which consumes the collector.
    pub fn service_tier(&self) -> Option<&'static str> {
        self.service_tier
    }

    /// Ingest a chunk for terminal-event tracking only, without a cli identity
    /// to merge usage for. Same block splitting as [`push_chunk`].
    pub fn observe_chunk(&mut self, chunk: &[u8]) {
        self.ingest_chunk(None, chunk);
    }

    fn ingest_chunk(&mut self, cli_key: Option<GatewayCliKey>, mut chunk: &[u8]) {
        let oversized_chunk = chunk.len() > MAX_SSE_USAGE_BUFFER_BYTES;
        while !chunk.is_empty() {
            let append_len = chunk
                .len()
                .min(MAX_SSE_USAGE_BUFFER_BYTES)
                .min(MAX_SSE_EVENT_BUFFER_BYTES - self.buffer.len());
            let mut search_from = self.buffer.len().saturating_sub(3);
            self.buffer.extend_from_slice(&chunk[..append_len]);
            chunk = &chunk[append_len..];

            while let Some((position, delimiter_len)) =
                find_sse_block_delimiter(&self.buffer[search_from..])
            {
                let block_end = search_from + position;
                let remainder = self.buffer.split_off(block_end + delimiter_len);
                let mut block = std::mem::replace(&mut self.buffer, remainder);
                block.truncate(block_end);
                if !self.discarding_oversized_block {
                    self.observe_block(&block);
                    if let Some(cli_key) = cli_key {
                        self.merge_block_usage(cli_key, &block);
                    }
                }
                self.discarding_oversized_block = false;
                self.next_flattened_scan_at = MAX_SSE_USAGE_BUFFER_BYTES;
                search_from = 0;
            }

            if self.discarding_oversized_block {
                let keep_from = self.buffer.len().saturating_sub(3);
                self.buffer.drain(..keep_from);
                continue;
            }

            if self.buffer.len() >= self.next_flattened_scan_at {
                self.flush_complete_flattened_events(cli_key);
                if self.buffer.len() >= MAX_SSE_EVENT_BUFFER_BYTES {
                    let keep_from = self.buffer.len().saturating_sub(3);
                    self.buffer.drain(..keep_from);
                    self.discarding_oversized_block = true;
                }
                self.next_flattened_scan_at = self
                    .buffer
                    .len()
                    .saturating_add(MAX_SSE_USAGE_BUFFER_BYTES)
                    .min(MAX_SSE_EVENT_BUFFER_BYTES);
            }
        }
        if oversized_chunk && !self.discarding_oversized_block {
            self.flush_complete_flattened_events(cli_key);
        }
    }

    fn flush_complete_flattened_events(&mut self, cli_key: Option<GatewayCliKey>) {
        let boundary = flattened_flush_boundary(&self.buffer);
        if boundary == 0 {
            return;
        }
        let remainder = self.buffer.split_off(boundary);
        let complete = std::mem::replace(&mut self.buffer, remainder);
        self.observe_bytes(cli_key, &complete);
    }

    /// Observe terminal + usage for bytes that bypass the bounded residual
    /// buffer. Complete blocks are observed individually; the undelimited
    /// remainder (partial block or flattened framing) goes through the
    /// flattened reader so nothing terminal-shaped is lost.
    fn observe_bytes(&mut self, cli_key: Option<GatewayCliKey>, bytes: &[u8]) {
        let mut rest = bytes;
        while let Some((position, delimiter_len)) = find_sse_block_delimiter(rest) {
            let block = &rest[..position];
            self.observe_block(block);
            if let Some(cli_key) = cli_key {
                self.merge_block_usage(cli_key, block);
            }
            rest = &rest[position + delimiter_len..];
        }
        if rest.is_empty() {
            return;
        }
        if let Some(cli_key) = cli_key {
            self.merge_flattened_usage(cli_key, rest);
        }
        if self.terminal_kind.is_none() {
            self.terminal_kind = sse_block_classify_terminal(rest);
        }
    }

    fn merge_block_usage(&mut self, cli_key: GatewayCliKey, bytes: &[u8]) {
        if let Some(value) = parse_sse_data_block(bytes) {
            self.merge_event(cli_key, &value);
        } else {
            self.merge_flattened_usage(cli_key, bytes);
        }
    }

    /// Merge usage from every flattened-framing event inside `bytes`.
    fn merge_flattened_usage(&mut self, cli_key: GatewayCliKey, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        for_each_flattened_sse_field(&text, |_event_name, data| {
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                self.merge_event(cli_key, &value);
            }
            false
        });
    }

    /// Update the terminal kind from a complete SSE block (first-wins).
    fn observe_block(&mut self, block: &[u8]) {
        if self.terminal_kind.is_none() {
            if let Some(kind) = sse_block_classify_terminal(block) {
                self.terminal_kind = Some(kind);
            }
        }
    }

    /// Drain any terminal marker still sitting in the residual buffer (a final
    /// SSE event that arrived without a trailing blank-line separator, so
    /// `push_chunk` could not extract it). Called after the stream loop ends so
    /// the verdict reflects a terminal event whose bytes were already written to
    /// the client inside an earlier chunk, even though the collector only parsed
    /// it on EOF. Does not consume the buffer — `finish` still merges any usage
    /// carried by that trailing block.
    pub fn drain_terminal(&mut self) {
        if self.buffer.is_empty() || self.terminal_kind.is_some() || self.discarding_oversized_block
        {
            return;
        }
        if let Some(kind) = sse_block_classify_terminal(&self.buffer) {
            self.terminal_kind = Some(kind);
        }
    }

    fn merge_event(&mut self, cli_key: GatewayCliKey, value: &Value) {
        if let Some(tier) = service_tier_from_value(value) {
            self.service_tier = Some(tier);
        }
        let mut parsed =
            from_json_response_with_provider_type(cli_key, value, self.provider_type.as_deref());
        if parsed.envelope_id.is_none() {
            parsed.envelope_id = extract_envelope_id(cli_key, value);
        }
        self.usage.merge_max(parsed);
    }
}

/// Whether an SSE block carries a protocol-level terminal marker. Thin wrapper
/// over [`sse_block_classify_terminal`] kept for call sites that only need the
/// boolean "did the stream end" verdict.
pub fn sse_block_is_terminal(block: &[u8]) -> bool {
    sse_block_classify_terminal(block).is_some()
}

/// Whether an SSE block carries a non-success terminal event. Thin wrapper over
/// [`sse_block_classify_terminal`]: `error` / `response.failed` (and the
/// terminal-but-not-success `response.incomplete` / `response.cancelled`) end
/// the stream properly but mark a failure.
pub fn sse_block_is_error_event(block: &[u8]) -> bool {
    matches!(
        sse_block_classify_terminal(block),
        Some(SseTerminalKind::Failed | SseTerminalKind::Incomplete | SseTerminalKind::Canceled)
    )
}

/// Map a Responses `response.status` / top-level `status` string to a terminal
/// kind. This is the key to distinguishing the gateway's own cross-protocol
/// signals: `response.completed` + `status=incomplete` (see
/// `transformer/stream.rs:2351`) must read as `Incomplete`, not `Completed`.
/// `completed` / unknown return `None` so the caller can fall back to the
/// event-name decision (Success) or continue inspecting `response.error`.
fn classify_response_status(status: &str) -> Option<SseTerminalKind> {
    match status.trim() {
        "failed" => Some(SseTerminalKind::Failed),
        "incomplete" => Some(SseTerminalKind::Incomplete),
        "cancelled" | "canceled" => Some(SseTerminalKind::Canceled),
        _ => None,
    }
}

/// Whether a JSON value carries a non-null `error` field (top-level or nested
/// under `response`). Mirrors the `value.error` / `response.error` arms of
/// `upstream.rs` `gateway_json_reports_error`.
fn json_carries_error(value: &Value) -> bool {
    value.get("error").is_some_and(|error| !error.is_null())
        || value
            .get("response")
            .and_then(|response| response.get("error"))
            .is_some_and(|error| !error.is_null())
}

/// Classify a single complete SSE block into a terminal kind. The check is flat
/// (not dispatched by protocol) because `source_protocol_from_route` returns
/// `None` for unknown paths and compatible upstreams often omit the SSE
/// `event:` field. Semantically aligned with `upstream.rs:9746`
/// `gateway_json_reports_error` (top-level `error`/`status`/`type`, nested
/// `response.error`/`response.status`), but extended to distinguish the three
/// non-success terminal outcomes (`Failed` / `Incomplete` / `Canceled`) that two
/// booleans could not express.
///
/// When the standard line-based parse finds nothing, a flattened-framing
/// fallback scans the block for degenerate single-line SSE (see
/// [`for_each_flattened_sse_field`]) before giving up.
pub fn sse_block_classify_terminal(block: &[u8]) -> Option<SseTerminalKind> {
    let text = String::from_utf8_lossy(block);
    let mut event_name: Option<&str> = None;
    let mut data_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(event) = line.strip_prefix("event:") {
            event_name = Some(event.trim());
        } else if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.strip_prefix(' ').unwrap_or(data));
        }
    }

    // OpenAI Chat `[DONE]` sentinel.
    if data_lines.iter().any(|data| data.trim() == "[DONE]") {
        return Some(SseTerminalKind::Success);
    }

    if let Some(kind) = classify_sse_event_fields(event_name, &data_lines.join("\n")) {
        return Some(kind);
    }

    // Degenerate flattened framing: several upstream relays concatenate every
    // event onto one whitespace-separated line, so the line-based parse above
    // sees a single giant `event:` field and no data at all. Scan field pairs
    // instead; first terminal wins, mirroring the block-level first-wins rule.
    let mut flattened: Option<SseTerminalKind> = None;
    for_each_flattened_sse_field(&text, |event_name, data| {
        match classify_sse_event_fields(event_name, data) {
            Some(kind) => {
                flattened = Some(kind);
                true
            }
            None => false,
        }
    });
    flattened
}

/// Classify one `(event_name, data)` pair using the shared terminal rules. The
/// data is either a JSON payload span or the `[DONE]` sentinel.
pub(crate) fn classify_sse_event_fields(
    event_name: Option<&str>,
    data: &str,
) -> Option<SseTerminalKind> {
    if data.trim() == "[DONE]" {
        return Some(SseTerminalKind::Success);
    }

    // SSE `event:` field — take precedence over JSON when present.
    if let Some(event_name) = event_name {
        if let Some(kind) = classify_terminal_event_name(event_name) {
            return Some(kind);
        }
    }

    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return None;
    };

    // Fallback: JSON `type` field when `event:` is absent.
    if let Some(event_type) = value.get("type").and_then(Value::as_str) {
        if let Some(kind) = classify_terminal_event_name(event_type) {
            return Some(kind);
        }
    }

    // `response.completed` may carry a non-success `response.status` (the
    // gateway's own cross-protocol incomplete/cancelled/failed signal). Inspect
    // before treating it as a plain success terminal.
    if event_name
        .map(|name| name == "response.completed")
        .unwrap_or_else(|| {
            value
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| t == "response.completed")
        })
    {
        let status = value
            .get("response")
            .and_then(|r| r.get("status"))
            .and_then(Value::as_str)
            .or_else(|| value.get("status").and_then(Value::as_str));
        if let Some(kind) = status.and_then(classify_response_status) {
            return Some(kind);
        }
        return Some(SseTerminalKind::Success);
    }

    // OpenAI Chat completions: choices[].finish_reason non-empty.
    if let Some(choices) = value.get("choices").and_then(Value::as_array) {
        if choices.iter().any(|choice| {
            choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| !reason.is_empty())
        }) {
            return Some(SseTerminalKind::Success);
        }
    }

    // Gemini generateContent: candidates[].finishReason non-empty.
    if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
        if candidates.iter().any(|candidate| {
            candidate
                .get("finishReason")
                .and_then(Value::as_str)
                .is_some_and(|reason| !reason.is_empty())
        }) {
            return Some(SseTerminalKind::Success);
        }
    }

    // JSON fallback aligned with gateway_json_reports_error: a non-null error
    // envelope (top-level or nested) makes this a Failed terminal even when the
    // event name was absent or non-terminal.
    if json_carries_error(&value) {
        return Some(SseTerminalKind::Failed);
    }

    // Top-level / nested `status` string for non-Responses envelopes. Responses-API
    // delta/part events (e.g. `response.reasoning_summary_part.done`) carry an
    // item-level `status` that is NOT a stream-terminal signal — observed in the
    // wild: `"status":"incomplete"` on a reasoning-summary part, meaning "this part
    // is not the final one". Treating that as a stream Incomplete prematurely locks
    // the verdict before the real `response.completed` arrives, marking a healthy
    // 200 stream as an error (issue #318, 2026-09 regression).
    //
    // But only `status:"incomplete"` is observed as item-level noise on deltas;
    // `status:"failed"` / `status:"cancelled"` are stream-level terminal signals that
    // no standard Responses delta carries as an item status, so a non-standard
    // `response.*` event carrying `status:"failed"` (without an `error` field, which
    // the earlier `json_carries_error` check would have caught) is still a genuine
    // failure. Block only the `Incomplete` outcome on Responses deltas, and let
    // `Failed`/`Canceled` through so custom non-standard failure events are not
    // swallowed.
    let is_responses_delta = is_responses_delta_event(event_name, &value);
    let allow_status =
        |kind: SseTerminalKind| !is_responses_delta || kind != SseTerminalKind::Incomplete;
    if let Some(kind) = value
        .get("status")
        .and_then(Value::as_str)
        .and_then(classify_response_status)
        .filter(|&kind| allow_status(kind))
    {
        return Some(kind);
    }
    if let Some(kind) = value
        .get("response")
        .and_then(|r| r.get("status"))
        .and_then(Value::as_str)
        .and_then(classify_response_status)
        .filter(|&kind| allow_status(kind))
    {
        return Some(kind);
    }

    None
}

/// Whether an SSE event is a Responses-API delta/part event whose top-level
/// `status` field is item-level (not a stream terminal). All Responses terminal
/// envelopes (`response.completed` / `response.incomplete` / `response.failed` /
/// `response.cancelled`) are classified earlier in
/// [`classify_sse_event_fields`], so a `response.`-prefixed event reaching the
/// `status` fallback is by construction a non-terminal delta whose `status`
/// (e.g. `"incomplete"` on `response.reasoning_summary_part.done`) must not be
/// treated as a stream outcome.
fn is_responses_delta_event(event_name: Option<&str>, value: &Value) -> bool {
    let name = event_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or_else(|| value.get("type").and_then(Value::as_str).map(str::trim))
        .filter(|name| !name.is_empty());
    match name {
        // `response.`-prefixed events other than the four terminal envelopes are
        // deltas/parts. The terminal envelopes never reach this function (they
        // return earlier), but list them explicitly so future event names under
        // `response.` cannot accidentally re-open the path.
        Some(name) => {
            name.starts_with("response.")
                && !matches!(
                    name,
                    "response.completed"
                        | "response.incomplete"
                        | "response.failed"
                        | "response.cancelled"
                        | "response.canceled"
                )
        }
        None => false,
    }
}

/// Map an SSE `event:` / JSON `type` name to a terminal kind. `response.completed`
/// deliberately returns `None` so the caller falls through to the
/// `response.status` inspection — the gateway emits
/// `response.completed` + `status=incomplete`/`cancelled`/`failed` for
/// cross-protocol non-success outcomes, and those must not collapse to Success.
fn classify_terminal_event_name(name: &str) -> Option<SseTerminalKind> {
    match name.trim() {
        "message_stop" | "speech.audio.done" | "transcript.text.done" => {
            Some(SseTerminalKind::Success)
        }
        "error" | "response.failed" => Some(SseTerminalKind::Failed),
        "response.incomplete" => Some(SseTerminalKind::Incomplete),
        "response.cancelled" | "response.canceled" => Some(SseTerminalKind::Canceled),
        "response.completed" => None,
        _ => None,
    }
}

/// Locate the next SSE field token (`event:` / `data:`) at or after `from`.
/// A token only counts when preceded by whitespace or the text start, so
/// `data:` inside URLs or identifiers is not mistaken for a field.
fn find_sse_field_token(text: &str, from: usize) -> Option<(usize, &'static str)> {
    let bytes = text.as_bytes();
    let mut cursor = from;
    while cursor <= bytes.len() {
        let event = text[cursor..].find("event:").map(|offset| cursor + offset);
        let data = text[cursor..].find("data:").map(|offset| cursor + offset);
        let (index, token) = match (event, data) {
            (Some(event_index), Some(data_index)) => {
                if event_index <= data_index {
                    (event_index, "event:")
                } else {
                    (data_index, "data:")
                }
            }
            (Some(event_index), None) => (event_index, "event:"),
            (None, Some(data_index)) => (data_index, "data:"),
            (None, None) => return None,
        };
        if index == 0 || bytes[index - 1].is_ascii_whitespace() {
            return Some((index, token));
        }
        cursor = index + 1;
    }
    None
}

/// Visit `(event_name, data)` pairs from text that may use degenerate
/// flattened SSE framing: multiple events concatenated onto one line and
/// separated by whitespace, with no blank-line delimiters (observed on Codex
/// mirror relays since 2026-08). The `data` value is either an exact JSON span
/// (parsed with a streaming deserializer so an ` event: ` sequence inside a
/// JSON string cannot split it) or the `[DONE]` sentinel. Visiting stops early
/// when the callback returns true.
///
/// [`SseUsageCollector`] (terminal + usage) and the gateway's first-chunk
/// semantic probe (`runtime::upstream`) share this reader; the probe must judge
/// content in flattened framing too, otherwise a live relay stream never looks
/// like it produced anything and dies on the probe bounds (issue #404).
pub(super) fn for_each_flattened_sse_field(
    text: &str,
    mut visit: impl FnMut(Option<&str>, &str) -> bool,
) {
    let bytes = text.as_bytes();
    let mut position = 0usize;
    let mut current_event: Option<String> = None;
    while let Some((index, token)) = find_sse_field_token(text, position) {
        let mut value_start = index + token.len();
        if bytes.get(value_start) == Some(&b' ') {
            value_start += 1;
        }
        if token == "event:" {
            // Event names never contain whitespace in any supported protocol.
            let name_end = bytes[value_start..]
                .iter()
                .position(u8::is_ascii_whitespace)
                .map_or(bytes.len(), |offset| value_start + offset);
            current_event = Some(text[value_start..name_end].to_string());
            position = name_end;
            continue;
        }
        let rest = &text[value_start..];
        let mut json_stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
        position = match json_stream.next() {
            Some(Ok(_value)) => {
                let json_end = value_start + json_stream.byte_offset();
                if visit(current_event.as_deref(), &text[value_start..json_end]) {
                    return;
                }
                json_end
            }
            Some(Err(error)) if error.is_eof() => break,
            _ => {
                let trimmed = rest.trim_start();
                if trimmed.starts_with("[DONE]") && visit(current_event.as_deref(), "[DONE]") {
                    return;
                }
                // Not JSON: skip this token and keep scanning.
                value_start
            }
        };
    }
}

/// Byte offset up to which an overflow flush may cut a buffered flattened-SSE
/// stream: the end of the last complete event JSON value. Everything after it
/// belongs to an event whose JSON is still incomplete, so it must stay buffered
/// until later chunks complete it — a flush cut anywhere else would split an
/// event into two unparseable halves and lose its usage and terminal verdict
/// (issue #318: a flush landing inside `response.completed`'s JSON lost both).
///
/// The first-chunk semantic probe uses the same boundary to drop the events it
/// has already judged, so its residual buffer is the same "still incomplete"
/// tail and each chunk costs O(new bytes) instead of rescanning the whole trace.
pub(super) fn flattened_flush_boundary(buffer: &[u8]) -> usize {
    // Operate on a UTF-8 string whose byte offsets stay aligned with the
    // original buffer. `String::from_utf8_lossy` would replace invalid bytes
    // with the 3-byte U+FFFD, making the lossy string longer than `buffer`;
    // a `boundary` computed on it could then exceed `buffer.len()` and panic
    // the caller's `&buffer[..boundary]` slice (issue #318 review finding).
    // Invalid UTF-8 in an SSE stream is itself malformed framing — flushing
    // nothing (boundary 0) and letting the bounded window retry later is the
    // safe fallback, matching `for_each_flattened_sse_field`'s lossy path
    // which never indexes back into the original bytes.
    let text = match std::str::from_utf8(buffer) {
        Ok(text) => text,
        Err(_) => return 0,
    };
    let bytes = text.as_bytes();
    let mut boundary = 0usize;
    let mut cursor = 0usize;
    while let Some((index, token)) = find_sse_field_token(text, cursor) {
        let mut value_start = index + token.len();
        if bytes.get(value_start) == Some(&b' ') {
            value_start += 1;
        }
        if token == "event:" {
            // Event names never contain whitespace in any supported protocol.
            cursor = bytes[value_start..]
                .iter()
                .position(u8::is_ascii_whitespace)
                .map_or(text.len(), |offset| value_start + offset);
            continue;
        }
        let mut json_stream =
            serde_json::Deserializer::from_str(&text[value_start..]).into_iter::<Value>();
        cursor = match json_stream.next() {
            Some(Ok(_value)) => {
                let json_end = value_start + json_stream.byte_offset();
                boundary = json_end;
                json_end
            }
            Some(Err(error)) if error.is_eof() => break,
            _ if text[value_start..].trim_start().starts_with("[DONE]") => {
                let leading = text[value_start..].len() - text[value_start..].trim_start().len();
                boundary = value_start + leading + "[DONE]".len();
                boundary
            }
            _ => value_start,
        };
    }
    boundary
}

pub fn from_response_body(cli_key: GatewayCliKey, body: &[u8]) -> TokenUsage {
    from_response_body_with_provider_type(cli_key, None, body)
}

pub fn from_response_body_with_provider_type(
    cli_key: GatewayCliKey,
    provider_type: Option<&str>,
    body: &[u8],
) -> TokenUsage {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        return from_json_response_with_provider_type(cli_key, &value, provider_type);
    }

    let mut collector = SseUsageCollector::with_provider_type(provider_type);
    collector.push_chunk(cli_key, body);
    collector.finish(cli_key)
}

fn from_json_response_with_provider_type(
    cli_key: GatewayCliKey,
    value: &Value,
    provider_type: Option<&str>,
) -> TokenUsage {
    let mut usage = match cli_key {
        GatewayCliKey::Claude | GatewayCliKey::ClaudeDesktop => claude_usage(value, provider_type),
        GatewayCliKey::Codex | GatewayCliKey::Grok | GatewayCliKey::OpenCode => openai_usage(value),
        // Moonshot-style OpenAI-compatible upstreams report the cached portion
        // as top-level `usage.cached_tokens` (no `prompt_tokens_details`). The
        // extra read path stays Kimi-only so the other OpenAI-compatible CLIs
        // keep the exact shared parsing behavior; the fresh-input subtraction
        // semantics come from the shared inclusive-input handling.
        GatewayCliKey::Kimi => openai_usage_with_extra_cache_read_paths(
            value,
            &["/usage/cached_tokens", "/response/usage/cached_tokens"],
        ),
        // OpenAI Chat, same as the other compatible CLIs; no extra cache path.
        GatewayCliKey::Workbuddy | GatewayCliKey::Codebuddy => openai_usage(value),
        GatewayCliKey::Gemini | GatewayCliKey::Antigravity => gemini_usage(value),
    };
    if usage.envelope_id.is_none() {
        usage.envelope_id = extract_envelope_id(cli_key, value);
    }
    usage
}

fn extract_envelope_id(cli_key: GatewayCliKey, value: &Value) -> Option<String> {
    let paths: &[&str] = match cli_key {
        GatewayCliKey::Claude | GatewayCliKey::ClaudeDesktop => &[
            "/message/id",
            "/id",
            "/response/id",
            "/message_id",
            "/messageId",
        ],
        GatewayCliKey::Workbuddy | GatewayCliKey::Codebuddy => {
            &["/id", "/response/id", "/responseId"]
        }
        GatewayCliKey::Gemini | GatewayCliKey::Antigravity => {
            &["/responseId", "/response/responseId", "/id"]
        }
        GatewayCliKey::Codex
        | GatewayCliKey::Grok
        | GatewayCliKey::Kimi
        | GatewayCliKey::OpenCode => &["/response/id", "/id", "/responseId"],
    };
    first_non_empty_string_at_paths(value, paths)
}

fn first_non_empty_string_at_paths(value: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn claude_usage(value: &Value, provider_type: Option<&str>) -> TokenUsage {
    let usage = value.pointer("/usage").or_else(|| {
        value
            .pointer("/message/usage")
            .or_else(|| value.pointer("/delta/usage"))
    });
    let usage_value = usage.unwrap_or(value);
    let input_tokens = first_u64_at_paths(usage_value, &["/input_tokens", "/prompt_tokens"])
        .or_else(|| {
            first_u64_at_paths(
                value,
                &[
                    "/usage/input_tokens",
                    "/message/usage/input_tokens",
                    "/delta/usage/input_tokens",
                ],
            )
        });
    let signed_input_tokens = first_i64_at_paths(usage_value, &["/input_tokens", "/prompt_tokens"])
        .or_else(|| {
            first_i64_at_paths(
                value,
                &[
                    "/usage/input_tokens",
                    "/message/usage/input_tokens",
                    "/delta/usage/input_tokens",
                ],
            )
        });
    let cache_read_tokens = first_u64_at_paths(
        usage_value,
        &[
            "/cache_read_input_tokens",
            "/cache_read_tokens",
            "/cached_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usage/cache_read_input_tokens",
                "/usage/cached_tokens",
                "/message/usage/cache_read_input_tokens",
                "/message/usage/cached_tokens",
                "/delta/usage/cache_read_input_tokens",
                "/delta/usage/cached_tokens",
            ],
        )
    });
    let cache_creation_tokens = first_u64_at_paths(
        usage_value,
        &[
            "/cache_creation_input_tokens",
            "/cache_creation_tokens",
            "/cache_write_input_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usage/cache_creation_input_tokens",
                "/message/usage/cache_creation_input_tokens",
                "/delta/usage/cache_creation_input_tokens",
            ],
        )
    });
    let input_tokens = if is_moonshot_provider_type(provider_type) {
        moonshot_fresh_input_tokens(signed_input_tokens, input_tokens, cache_read_tokens)
    } else {
        input_tokens
    };

    TokenUsage {
        input_tokens,
        output_tokens: first_u64_at_paths(usage_value, &["/output_tokens", "/completion_tokens"])
            .or_else(|| {
                first_u64_at_paths(
                    value,
                    &[
                        "/usage/output_tokens",
                        "/message/usage/output_tokens",
                        "/delta/usage/output_tokens",
                    ],
                )
            }),
        cache_read_tokens,
        cache_creation_tokens,
        envelope_id: extract_envelope_id(GatewayCliKey::Claude, value),
    }
}

fn is_moonshot_provider_type(provider_type: Option<&str>) -> bool {
    provider_type
        .map(|value| value.trim().to_ascii_lowercase().replace('_', "-"))
        .is_some_and(|value| matches!(value.as_str(), "moonshot" | "kimi"))
}

fn moonshot_fresh_input_tokens(
    signed_input_tokens: Option<i64>,
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
) -> Option<u64> {
    let cache_read = cache_read_tokens.unwrap_or(0);
    if cache_read == 0 {
        return input_tokens;
    }
    if let Some(signed_input) = signed_input_tokens {
        if signed_input < 0 {
            return Some((signed_input + cache_read as i64).max(0) as u64);
        }
    }
    let input = input_tokens?;
    if input < cache_read {
        Some(input)
    } else {
        Some(input.saturating_sub(cache_read))
    }
}

fn openai_usage(value: &Value) -> TokenUsage {
    openai_usage_with_extra_cache_read_paths(value, &[])
}

/// `openai_usage` with additional envelope-rooted fallback paths for
/// `cache_read_tokens`. Kimi uses this for Moonshot-style top-level
/// `usage.cached_tokens`; the other OpenAI-compatible CLIs pass no extra
/// paths and keep the exact shared parsing behavior.
fn openai_usage_with_extra_cache_read_paths(
    value: &Value,
    extra_cache_read_paths: &[&str],
) -> TokenUsage {
    let usage = value
        .pointer("/usage")
        .or_else(|| value.pointer("/response/usage"))
        .unwrap_or(value);
    let raw_input_tokens =
        first_u64_at_paths(usage, &["/input_tokens", "/prompt_tokens"]).or_else(|| {
            first_u64_at_paths(
                value,
                &[
                    "/usage/input_tokens",
                    "/usage/prompt_tokens",
                    "/response/usage/input_tokens",
                    "/response/usage/prompt_tokens",
                ],
            )
        });
    let cache_read_tokens = first_u64_at_paths(
        usage,
        &[
            "/input_tokens_details/cached_tokens",
            "/prompt_tokens_details/cached_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usage/input_tokens_details/cached_tokens",
                "/usage/prompt_tokens_details/cached_tokens",
                "/response/usage/input_tokens_details/cached_tokens",
                "/response/usage/prompt_tokens_details/cached_tokens",
            ],
        )
    })
    .or_else(|| {
        if extra_cache_read_paths.is_empty() {
            None
        } else {
            first_u64_at_paths(value, extra_cache_read_paths)
        }
    });
    // Responses API cache writes (AxonHub 94704784) and Chat-compatible aliases.
    // OpenAI/Responses `input_tokens` / `prompt_tokens` are treated as cache-inclusive
    // totals: fresh input = raw - cache_read - cache_write. Providers that already
    // report fresh-only input would undercount if they also emit cache write details.
    let cache_creation_tokens = first_u64_at_paths(
        usage,
        &[
            "/input_tokens_details/cache_write_tokens",
            "/prompt_tokens_details/cache_write_tokens",
            "/input_tokens_details/cache_creation_tokens",
            "/prompt_tokens_details/cache_creation_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usage/input_tokens_details/cache_write_tokens",
                "/usage/prompt_tokens_details/cache_write_tokens",
                "/response/usage/input_tokens_details/cache_write_tokens",
                "/response/usage/prompt_tokens_details/cache_write_tokens",
                "/usage/input_tokens_details/cache_creation_tokens",
                "/usage/prompt_tokens_details/cache_creation_tokens",
                "/response/usage/input_tokens_details/cache_creation_tokens",
                "/response/usage/prompt_tokens_details/cache_creation_tokens",
            ],
        )
    });
    TokenUsage {
        input_tokens: subtract_cache_from_inclusive_input(
            raw_input_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        ),
        output_tokens: first_u64_at_paths(usage, &["/output_tokens", "/completion_tokens"])
            .or_else(|| {
                first_u64_at_paths(
                    value,
                    &[
                        "/usage/output_tokens",
                        "/usage/completion_tokens",
                        "/response/usage/output_tokens",
                        "/response/usage/completion_tokens",
                    ],
                )
            }),
        cache_read_tokens,
        cache_creation_tokens,
        envelope_id: extract_envelope_id(GatewayCliKey::Codex, value),
    }
}

fn gemini_usage(value: &Value) -> TokenUsage {
    let usage = value.pointer("/usageMetadata").unwrap_or(value);
    let input_tokens =
        first_u64_at_paths(usage, &["/promptTokenCount", "/prompt_tokens"]).or_else(|| {
            first_u64_at_paths(
                value,
                &[
                    "/usageMetadata/promptTokenCount",
                    "/response/usageMetadata/promptTokenCount",
                ],
            )
        });
    let output_tokens = first_u64_at_paths(
        usage,
        &[
            "/candidatesTokenCount",
            "/completion_tokens",
            "/output_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usageMetadata/candidatesTokenCount",
                "/response/usageMetadata/candidatesTokenCount",
            ],
        )
    });

    let cache_read_tokens =
        first_u64_at_paths(usage, &["/cachedContentTokenCount", "/cache_read_tokens"]).or_else(
            || {
                first_u64_at_paths(
                    value,
                    &[
                        "/usageMetadata/cachedContentTokenCount",
                        "/response/usageMetadata/cachedContentTokenCount",
                    ],
                )
            },
        );

    TokenUsage {
        input_tokens: subtract_cache_from_inclusive_input(input_tokens, cache_read_tokens, None),
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens: None,
        envelope_id: extract_envelope_id(GatewayCliKey::Gemini, value),
    }
}

fn first_u64_at_paths(value: &Value, paths: &[&str]) -> Option<u64> {
    paths
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_u64))
}

fn first_i64_at_paths(value: &Value, paths: &[&str]) -> Option<i64> {
    paths
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_i64))
}

fn subtract_cache_from_inclusive_input(
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
) -> Option<u64> {
    input_tokens.map(|tokens| {
        tokens
            .saturating_sub(cache_read_tokens.unwrap_or(0))
            .saturating_sub(cache_creation_tokens.unwrap_or(0))
    })
}

fn max_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
fn take_sse_block(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let (position, delimiter_len) = find_sse_block_delimiter(buffer)?;
    let block = buffer[..position].to_vec();
    buffer.drain(..position + delimiter_len);
    Some(block)
}

/// Locate the physically earliest SSE block delimiter (`\n\n` or `\r\n\r\n`)
/// and return its position plus length.
fn find_sse_block_delimiter(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| (index, 2));
    let crlf = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4));
    match (lf, crlf) {
        (Some(lf), Some(crlf)) if crlf.0 < lf.0 => Some(crlf),
        (Some(lf), _) => Some(lf),
        (None, crlf) => crlf,
    }
}

fn parse_sse_data_block(block: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(block);
    let mut data_lines = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.strip_prefix(' ').unwrap_or(data);
        if data.trim() == "[DONE]" {
            return None;
        }
        data_lines.push(data);
    }
    if data_lines.is_empty() {
        return None;
    }
    let data = data_lines.join("\n");
    serde_json::from_str::<Value>(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_stream_usage_with_cache_tokens() {
        let body = br#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":120,"output_tokens":1,"cache_read_input_tokens":40,"cache_creation_input_tokens":8}}}

event: message_delta
data: {"type":"message_delta","usage":{"output_tokens":35}}

"#;

        let usage = from_response_body(GatewayCliKey::Claude, body);

        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.output_tokens, Some(35));
        assert_eq!(usage.cache_read_tokens, Some(40));
        assert_eq!(usage.cache_creation_tokens, Some(8));
        assert_eq!(usage.total_tokens(), Some(203));
    }

    #[test]
    fn parses_moonshot_anthropic_cached_tokens_as_cache_read() {
        let usage = from_response_body_with_provider_type(
            GatewayCliKey::Claude,
            Some("moonshot"),
            br#"{"usage":{"input_tokens":100,"output_tokens":10,"cached_tokens":80}}"#,
        );

        assert_eq!(usage.input_tokens, Some(20));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(110));
    }

    #[test]
    fn parses_kimi_moonshot_style_top_level_cached_tokens() {
        // Moonshot-style OpenAI-compatible upstreams report the cached portion
        // as top-level `usage.cached_tokens` without
        // `prompt_tokens_details.cached_tokens`. Input is treated as a
        // cache-inclusive total: fresh = raw - cached.
        let usage = from_response_body(
            GatewayCliKey::Kimi,
            br#"{"id":"chatcmpl-1","usage":{"prompt_tokens":100,"completion_tokens":10,"cached_tokens":80}}"#,
        );

        assert_eq!(usage.input_tokens, Some(20));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(110));
    }

    #[test]
    fn parses_kimi_nested_response_top_level_cached_tokens() {
        let usage = from_response_body(
            GatewayCliKey::Kimi,
            br#"{"response":{"id":"resp_1","usage":{"prompt_tokens":90,"completion_tokens":12,"cached_tokens":30}}}"#,
        );

        assert_eq!(usage.input_tokens, Some(60));
        assert_eq!(usage.output_tokens, Some(12));
        assert_eq!(usage.cache_read_tokens, Some(30));
    }

    #[test]
    fn non_kimi_openai_usage_ignores_top_level_cached_tokens() {
        let usage = from_response_body(
            GatewayCliKey::Codex,
            br#"{"id":"chatcmpl-1","usage":{"prompt_tokens":100,"completion_tokens":10,"cached_tokens":80}}"#,
        );

        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, None);
    }

    #[test]
    fn parses_moonshot_negative_input_cache_discount() {
        let usage = from_response_body_with_provider_type(
            GatewayCliKey::Claude,
            Some("kimi"),
            br#"{"usage":{"input_tokens":-40,"output_tokens":10,"cached_tokens":80}}"#,
        );

        // fresh = signed + 1×cache = -40 + 80 = 40; total = fresh+output+cache = 130
        assert_eq!(usage.input_tokens, Some(40));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(130));
    }

    #[test]
    fn parses_moonshot_positive_input_less_than_cache_as_fresh() {
        // Branch: positive input < cache → treat input as already-fresh (no subtract).
        let usage = from_response_body_with_provider_type(
            GatewayCliKey::Claude,
            Some("moonshot"),
            br#"{"usage":{"input_tokens":30,"output_tokens":5,"cached_tokens":80}}"#,
        );
        assert_eq!(usage.input_tokens, Some(30));
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(115));
    }

    #[test]
    fn default_anthropic_usage_keeps_input_tokens_as_fresh_input() {
        let usage = from_response_body_with_provider_type(
            GatewayCliKey::Claude,
            Some("anthropic"),
            br#"{"usage":{"input_tokens":100,"output_tokens":10,"cached_tokens":80}}"#,
        );

        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(190));
    }

    #[test]
    fn parses_openai_stream_usage_with_cached_prompt_tokens() {
        let body = br#"data: {"type":"response.completed","response":{"id":"resp_abc","usage":{"input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30}}}}

data: [DONE]

"#;

        let usage = from_response_body(GatewayCliKey::Codex, body);

        assert_eq!(usage.input_tokens, Some(60));
        assert_eq!(usage.output_tokens, Some(12));
        assert_eq!(usage.cache_read_tokens, Some(30));
        assert_eq!(usage.cache_creation_tokens, None);
        assert_eq!(usage.total_tokens(), Some(102));
        assert_eq!(usage.envelope_id.as_deref(), Some("resp_abc"));
    }

    #[test]
    fn parses_responses_cache_write_tokens_as_cache_creation() {
        // AxonHub 94704784: input_tokens_details.cache_write_tokens.
        let usage = from_response_body(
            GatewayCliKey::Codex,
            br#"{"id":"resp_cw","usage":{"input_tokens":100,"output_tokens":10,"input_tokens_details":{"cached_tokens":20,"cache_write_tokens":8}}}"#,
        );
        // Inclusive OpenAI/Responses semantics: fresh input = 100 - 20 - 8 = 72
        assert_eq!(usage.input_tokens, Some(72));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(20));
        assert_eq!(usage.cache_creation_tokens, Some(8));
        assert_eq!(usage.total_tokens(), Some(110));
        assert_eq!(usage.envelope_id.as_deref(), Some("resp_cw"));
    }

    #[test]
    fn parses_responses_cache_write_only_without_cache_read() {
        let usage = from_response_body(
            GatewayCliKey::Codex,
            br#"{"id":"resp_cw_only","usage":{"input_tokens":50,"output_tokens":5,"input_tokens_details":{"cache_write_tokens":12}}}"#,
        );
        assert_eq!(usage.input_tokens, Some(38));
        assert_eq!(usage.cache_read_tokens, None);
        assert_eq!(usage.cache_creation_tokens, Some(12));
        assert_eq!(usage.total_tokens(), Some(55));
    }

    #[test]
    fn stable_usage_request_id_scopes_non_claude_and_keeps_claude_bare() {
        assert_eq!(
            stable_usage_request_id(
                GatewayCliKey::Claude,
                Some("provider-a"),
                Some("msg_123"),
                "gw-fallback"
            ),
            "SESSION:msg_123"
        );
        assert_eq!(
            stable_usage_request_id(
                GatewayCliKey::Codex,
                Some("provider-a"),
                Some("resp_123"),
                "gw-fallback"
            ),
            "SESSION:codex:provider-a:resp_123"
        );
        assert_eq!(
            stable_usage_request_id(GatewayCliKey::Gemini, Some("p1"), None, "gw-fallback"),
            "gw-fallback"
        );
        assert_eq!(
            stable_usage_request_id(GatewayCliKey::Codex, Some("p1"), Some(""), "gw-fallback"),
            "gw-fallback"
        );
    }

    #[test]
    fn extracts_gemini_response_id_as_envelope() {
        let usage = from_response_body(
            GatewayCliKey::Gemini,
            br#"{"responseId":"gemini_1","usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":7}}"#,
        );
        assert_eq!(usage.envelope_id.as_deref(), Some("gemini_1"));
    }

    #[test]
    fn extracts_claude_message_id_as_envelope() {
        let usage = from_response_body(
            GatewayCliKey::Claude,
            br#"{"id":"msg_9","usage":{"input_tokens":1,"output_tokens":2}}"#,
        );
        assert_eq!(usage.envelope_id.as_deref(), Some("msg_9"));
        assert_eq!(
            stable_usage_request_id(
                GatewayCliKey::Claude,
                Some("p"),
                usage.envelope_id.as_deref(),
                "gw-x"
            ),
            "SESSION:msg_9"
        );
    }

    #[test]
    fn parses_gemini_json_usage_metadata() {
        let usage = from_response_body(
            GatewayCliKey::Gemini,
            br#"{"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":7,"cachedContentTokenCount":3}}"#,
        );

        assert_eq!(usage.input_tokens, Some(7));
        assert_eq!(usage.output_tokens, Some(7));
        assert_eq!(usage.cache_read_tokens, Some(3));
        assert_eq!(usage.total_tokens(), Some(17));
    }

    #[test]
    fn gemini_usage_does_not_infer_output_from_total_tokens() {
        let usage = from_response_body(
            GatewayCliKey::Gemini,
            br#"{"usageMetadata":{"promptTokenCount":10,"totalTokenCount":17}}"#,
        );

        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.total_tokens(), Some(10));
    }

    #[test]
    fn sse_usage_collector_keeps_buffer_bounded() {
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(
            GatewayCliKey::Claude,
            &vec![b'a'; MAX_SSE_EVENT_BUFFER_BYTES + 1],
        );
        assert!(collector.buffer.len() <= 3);
        assert!(collector.discarding_oversized_block);
        collector.drain_terminal();
        assert_eq!(collector.terminal_kind(), None);
        collector.push_chunk(
            GatewayCliKey::Claude,
            b"\n\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        assert!(!collector.discarding_oversized_block);
        assert_eq!(collector.terminal_kind(), Some(SseTerminalKind::Success));
    }

    #[test]
    fn served_service_tier_reads_bare_and_nested_envelopes() {
        // Chat / non-streaming Responses put the field at the top level.
        assert_eq!(
            served_service_tier(br#"{"service_tier":"priority"}"#),
            Some("fast")
        );
        assert_eq!(
            served_service_tier(br#"{"service_tier":"fast"}"#),
            Some("fast")
        );
        assert_eq!(
            served_service_tier(br#"{"service_tier":"ultrafast"}"#),
            Some("ultrafast")
        );
        // Responses stream events nest the response snapshot.
        assert_eq!(
            served_service_tier(
                br#"{"type":"response.completed","response":{"service_tier":"priority"}}"#
            ),
            Some("fast")
        );
        // Non-premium / unusable values never bill.
        assert_eq!(served_service_tier(br#"{"service_tier":"default"}"#), None);
        assert_eq!(served_service_tier(br#"{"service_tier":"auto"}"#), None);
        assert_eq!(served_service_tier(br#"{"service_tier":"flex"}"#), None);
        assert_eq!(served_service_tier(br#"{"service_tier":null}"#), None);
        assert_eq!(served_service_tier(br#"{"service_tier":1}"#), None);
        assert_eq!(
            served_service_tier(br#"{"response":{"service_tier":"scale"}}"#),
            None
        );
        assert_eq!(served_service_tier(b"not json"), None);
        assert_eq!(served_service_tier(br#"{"model":"gpt-5.6-sol"}"#), None);
    }

    #[test]
    fn sse_usage_collector_keeps_the_last_confirmed_service_tier() {
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(
            GatewayCliKey::Codex,
            b"data: {\"type\":\"response.created\",\"response\":{\"service_tier\":\"priority\"}}\n\n",
        );
        assert_eq!(collector.service_tier(), Some("fast"));
        collector.push_chunk(
            GatewayCliKey::Codex,
            b"data: {\"type\":\"response.completed\",\"response\":{\"service_tier\":\"ultrafast\"}}\n\n",
        );
        assert_eq!(collector.service_tier(), Some("ultrafast"));
        // A later unusable echo (`default`) must not erase the confirmed tier.
        collector.push_chunk(
            GatewayCliKey::Codex,
            b"data: {\"type\":\"response.completed\",\"response\":{\"service_tier\":\"default\"}}\n\n",
        );
        assert_eq!(collector.service_tier(), Some("ultrafast"));
        let _ = collector.finish(GatewayCliKey::Codex);
    }

    #[test]
    fn take_sse_block_consumes_physically_earliest_delimiter() {
        // LF event first, CRLF later: must not drain through the later CRLF.
        let mut buffer = b"data: {\"a\":1}\n\ndata: {\"b\":2}\r\n\r\n".to_vec();
        let first = take_sse_block(&mut buffer).expect("first block");
        assert_eq!(String::from_utf8_lossy(&first), "data: {\"a\":1}");
        let second = take_sse_block(&mut buffer).expect("second block");
        assert_eq!(String::from_utf8_lossy(&second), "data: {\"b\":2}");
        assert!(buffer.is_empty());

        // CRLF first, LF later.
        let mut buffer = b"data: {\"c\":3}\r\n\r\ndata: {\"d\":4}\n\n".to_vec();
        let first = take_sse_block(&mut buffer).expect("crlf-first block");
        assert_eq!(String::from_utf8_lossy(&first), "data: {\"c\":3}");
        let second = take_sse_block(&mut buffer).expect("lf-second block");
        assert_eq!(String::from_utf8_lossy(&second), "data: {\"d\":4}");
    }

    #[test]
    fn sse_usage_collector_parses_mixed_lf_crlf_events() {
        let mut collector = SseUsageCollector::default();
        // First event ends with LF, second with CRLF; wrong priority would merge both.
        collector.push_chunk(
            GatewayCliKey::Codex,
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_a\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
        );
        collector.push_chunk(
            GatewayCliKey::Codex,
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_b\",\"usage\":{\"input_tokens\":90,\"output_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":30}}}}\r\n\r\n",
        );
        let usage = collector.finish(GatewayCliKey::Codex);
        assert_eq!(usage.input_tokens, Some(60));
        assert_eq!(usage.output_tokens, Some(12));
        assert_eq!(usage.cache_read_tokens, Some(30));
        assert_eq!(usage.envelope_id.as_deref(), Some("resp_a"));
    }

    #[test]
    fn sse_block_is_terminal_detects_openai_done_sentinel() {
        assert!(sse_block_is_terminal(b"data: [DONE]\n\n"));
    }

    #[test]
    fn sse_block_is_terminal_detects_anthropic_message_stop_event_field() {
        assert!(sse_block_is_terminal(
            b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        ));
    }

    #[test]
    fn sse_block_is_terminal_detects_responses_completed_via_json_type_fallback() {
        // Compatible upstreams omit the SSE `event:` field; only JSON `type`.
        assert!(sse_block_is_terminal(
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_x\"}}\n\n"
        ));
    }

    #[test]
    fn sse_block_is_terminal_detects_openai_chat_finish_reason() {
        assert!(sse_block_is_terminal(
            b"data: {\"choices\":[{\"finish_reason\":\"stop\",\"delta\":{}}]}\n\n"
        ));
    }

    #[test]
    fn sse_block_is_terminal_ignores_null_finish_reason() {
        assert!(!sse_block_is_terminal(
            b"data: {\"choices\":[{\"finish_reason\":null,\"delta\":{\"content\":\"hi\"}}]}\n\n"
        ));
    }

    #[test]
    fn sse_block_is_terminal_detects_gemini_finish_reason() {
        assert!(sse_block_is_terminal(
            b"data: {\"candidates\":[{\"finishReason\":\"STOP\",\"content\":{\"parts\":[{\"text\":\"ok\"}]}}]}\n\n"
        ));
    }

    #[test]
    fn sse_block_is_terminal_detects_mid_stream_error_event() {
        assert!(sse_block_is_terminal(
            b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"overloaded\"}}\n\n"
        ));
    }

    #[test]
    fn sse_block_is_error_event_distinguishes_error_terminal_from_success_terminal() {
        // `error` / `response.failed` end the stream but mark a failure.
        assert!(sse_block_is_error_event(
            b"event: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"boom\"}}\n\n"
        ));
        assert!(sse_block_is_error_event(
            b"data: {\"type\":\"response.failed\",\"response\":{\"id\":\"r\",\"status\":\"failed\"}}\n\n"
        ));
        // `response.incomplete` / `response.cancelled` are terminal-but-not-success
        // (mirrors upstream.rs:488-494), so they count as error terminals.
        assert!(sse_block_is_error_event(
            b"data: {\"type\":\"response.incomplete\",\"response\":{\"id\":\"r\",\"status\":\"incomplete\"}}\n\n"
        ));
        assert!(sse_block_is_error_event(
            b"event: response.cancelled\ndata: {\"type\":\"response.cancelled\",\"response\":{\"id\":\"r\",\"status\":\"canceled\"}}\n\n"
        ));
        // `message_stop` / `response.completed` / `[DONE]` end the stream as a
        // success terminal, not an error terminal.
        assert!(!sse_block_is_error_event(
            b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        ));
        assert!(!sse_block_is_error_event(
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\"}}\n\n"
        ));
        assert!(!sse_block_is_error_event(b"data: [DONE]\n\n"));
    }

    #[test]
    fn sse_usage_collector_tracks_error_event_independently_from_terminal() {
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(
            GatewayCliKey::Claude,
            b"event: message_delta\ndata: {\"type\":\"message_delta\"}\n\n",
        );
        assert!(!collector.terminal_event_seen());
        assert!(!collector.error_event_seen());
        collector.push_chunk(
            GatewayCliKey::Claude,
            b"event: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"boom\"}}\n\n",
        );
        assert!(collector.terminal_event_seen());
        assert!(collector.error_event_seen());
    }

    #[test]
    fn sse_block_is_terminal_rejects_delta_events() {
        assert!(!sse_block_is_terminal(
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hello\"}}\n\n"
        ));
        assert!(!sse_block_is_terminal(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n"
        ));
    }

    #[test]
    fn sse_usage_collector_tracks_terminal_event_across_chunks() {
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(
            GatewayCliKey::Claude,
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n",
        );
        assert!(!collector.terminal_event_seen());
        collector.push_chunk(
            GatewayCliKey::Claude,
            b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        assert!(collector.terminal_event_seen());
    }

    #[test]
    fn sse_usage_collector_finish_detects_terminal_in_trailing_buffer() {
        // Terminal event arrives without a trailing blank-line separator, so it
        // stays in the buffer until finish() drains it.
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(GatewayCliKey::Codex, b"data: [DONE]");
        assert!(!collector.terminal_event_seen());
        let _ = collector.finish(GatewayCliKey::Codex);
        // finish() consumes self; assert via a fresh collector that sees it.
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(GatewayCliKey::Codex, b"data: [DONE]\n\n");
        assert!(collector.terminal_event_seen());
    }

    #[test]
    fn sse_usage_collector_drain_terminal_recovers_unseparated_final_event() {
        // A real stream often ends with the terminal event and immediate EOF,
        // no trailing blank line. push_chunk cannot extract the block, so the
        // verdict must still surface via drain_terminal — otherwise the stream
        // is misclassified as incomplete and the client gets a spurious error.
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(
            GatewayCliKey::Claude,
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n",
        );
        assert!(!collector.terminal_event_seen());
        // Final message_stop arrives without a trailing blank-line separator.
        collector.push_chunk(
            GatewayCliKey::Claude,
            b"event: message_stop\ndata: {\"type\":\"message_stop\"}",
        );
        assert!(
            !collector.terminal_event_seen(),
            "unseparated block must not be parsed by push_chunk"
        );
        collector.drain_terminal();
        assert!(
            collector.terminal_event_seen(),
            "drain_terminal must recover the terminal marker from the residual buffer"
        );
    }

    /// Realistic degenerate framing observed on Codex mirror relays (issue #318):
    /// every event concatenated onto one whitespace-separated line, including
    /// relay-injected `codex.*` events, with no blank-line delimiters at all.
    fn flattened_codex_relay_stream() -> String {
        [
            "event: codex.rate_limits data: {\"type\":\"codex.rate_limits\",\"rate_limits\":{\"allowed\":true}}",
            "event: codex.response.metadata data: {\"type\":\"codex.response.metadata\",\"headers\":{\"x-codex-safety-buffering-enabled\":\"true\"}}",
            "event: response.created data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_x\",\"status\":\"in_progress\"}}",
            "event: response.output_text.delta data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}",
            "event: response.completed data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_x\",\"status\":\"completed\",\"usage\":{\"input_tokens\":120,\"output_tokens\":40}}}",
        ]
        .join(" ")
    }

    #[test]
    fn sse_block_classify_terminal_flattened_codex_relay_stream_is_success() {
        assert_eq!(
            sse_block_classify_terminal(flattened_codex_relay_stream().as_bytes()),
            Some(SseTerminalKind::Success)
        );
    }

    #[test]
    fn sse_block_classify_terminal_flattened_stream_without_terminal_is_none() {
        let text = [
            "event: codex.rate_limits data: {\"type\":\"codex.rate_limits\",\"rate_limits\":{\"allowed\":true}}",
            "event: response.created data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_x\"}}",
            "event: response.output_text.delta data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}",
        ]
        .join(" ");
        assert_eq!(sse_block_classify_terminal(text.as_bytes()), None);
    }

    #[test]
    fn sse_block_classify_terminal_flattened_failed_terminal_is_failed() {
        let text = [
            "event: response.created data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_x\"}}",
            "event: response.failed data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_x\",\"error\":{\"message\":\"boom\"}}}",
        ]
        .join(" ");
        assert_eq!(
            sse_block_classify_terminal(text.as_bytes()),
            Some(SseTerminalKind::Failed)
        );
    }

    #[test]
    fn sse_block_classify_terminal_flattened_done_sentinel_is_success() {
        let text = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]} data: [DONE]";
        assert_eq!(
            sse_block_classify_terminal(text.as_bytes()),
            Some(SseTerminalKind::Success)
        );
    }

    #[test]
    fn sse_block_classify_terminal_flattened_json_with_embedded_event_marker_is_none() {
        // A JSON string value containing an ` event: ` sequence must not be
        // split: the streaming parser consumes the whole value, so the embedded
        // marker never produces a fake terminal.
        let text = "event: response.output_text.delta data: {\"type\":\"response.output_text.delta\",\"delta\":\"run event: response.completed data: {\\\"type\\\":\\\"response.completed\\\"}\"}";
        assert_eq!(sse_block_classify_terminal(text.as_bytes()), None);
    }

    #[test]
    fn sse_block_classify_terminal_flattened_completed_with_status_incomplete_is_incomplete() {
        let text = "event: response.completed data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"incomplete\"}}";
        assert_eq!(
            sse_block_classify_terminal(text.as_bytes()),
            Some(SseTerminalKind::Incomplete)
        );
    }

    #[test]
    fn large_terminal_event_survives_every_network_chunk_size() {
        let padding = "中文".repeat(90_000);
        let body = format!(
            "event: response.created data: {{\"type\":\"response.created\"}} event: response.completed data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_large\",\"status\":\"completed\",\"note\":\"{padding}\",\"usage\":{{\"input_tokens\":120,\"output_tokens\":40}}}}}}"
        );
        for chunk_size in [1024, 65536, 262144, 300000, body.len()] {
            let mut collector = SseUsageCollector::default();
            for chunk in body.as_bytes().chunks(chunk_size) {
                collector.push_chunk(GatewayCliKey::Codex, chunk);
            }
            collector.drain_terminal();
            assert_eq!(
                collector.terminal_kind(),
                Some(SseTerminalKind::Success),
                "chunk size {chunk_size}"
            );
            let usage = collector.finish(GatewayCliKey::Codex);
            assert_eq!(usage.input_tokens, Some(120), "chunk size {chunk_size}");
            assert_eq!(usage.output_tokens, Some(40), "chunk size {chunk_size}");
        }
    }

    #[test]
    fn flattened_events_with_a_final_blank_line_keep_usage() {
        for delimiter in ["\n\n", "\r\n\r\n"] {
            let body = format!(
                "event: response.created data: {{\"type\":\"response.created\"}} event: response.completed data: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\",\"usage\":{{\"input_tokens\":120,\"output_tokens\":40}}}}}}{delimiter}"
            );
            for chunk_size in [1, 13, body.len()] {
                let mut collector = SseUsageCollector::default();
                for chunk in body.as_bytes().chunks(chunk_size) {
                    collector.push_chunk(GatewayCliKey::Codex, chunk);
                }
                assert_eq!(collector.terminal_kind(), Some(SseTerminalKind::Success));
                let usage = collector.finish(GatewayCliKey::Codex);
                assert_eq!(usage.input_tokens, Some(120), "chunk size {chunk_size}");
                assert_eq!(usage.output_tokens, Some(40), "chunk size {chunk_size}");
            }
        }
    }

    #[test]
    fn partial_flattened_json_does_not_classify_quoted_field_tokens() {
        let partial = b"event: response.output_text.delta data: {\"type\":\"response.output_text.delta\",\"delta\":\"quoted event: error data: {} unfinished";
        assert_eq!(sse_block_classify_terminal(partial), None);
        assert_eq!(flattened_flush_boundary(partial), 0);
        let mut collector = SseUsageCollector::default();
        collector.observe_chunk(partial);
        collector.drain_terminal();
        assert_eq!(collector.terminal_kind(), None);
        collector.observe_chunk(
            b"\"} event: response.completed data: {\"type\":\"response.completed\"}",
        );
        collector.drain_terminal();
        assert_eq!(collector.terminal_kind(), Some(SseTerminalKind::Success));
    }

    #[test]
    fn flattened_stream_terminal_survives_oversized_single_chunk() {
        // Regression for issue #318: a flattened stream makes the outbound
        // pipeline buffer the whole body and emit it as one oversized final
        // chunk. The old code dropped oversized chunks, so the terminal verdict
        // was lost and every such 200 response was misclassified as incomplete.
        let padding = "x".repeat(MAX_SSE_USAGE_BUFFER_BYTES + 1024);
        let stream = format!(
            "{} {}",
            flattened_codex_relay_stream().replace("\"hello\"", &format!("\"{padding}\"")),
            "event: response.completed data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_x\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}"
        );
        let mut collector = SseUsageCollector::default();
        collector.observe_chunk(stream.as_bytes());
        assert_eq!(
            collector.terminal_kind(),
            Some(SseTerminalKind::Success),
            "oversized flattened chunk must still yield its terminal event"
        );
    }

    #[test]
    fn flattened_stream_usage_recovered_from_oversized_single_chunk() {
        let padding = "x".repeat(MAX_SSE_USAGE_BUFFER_BYTES + 1024);
        let stream = format!(
            "{} {}",
            flattened_codex_relay_stream().replace("\"hello\"", &format!("\"{padding}\"")),
            "event: response.completed data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_final\",\"status\":\"completed\",\"usage\":{\"input_tokens\":120,\"output_tokens\":40}}}"
        );
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(GatewayCliKey::Codex, stream.as_bytes());
        let usage = collector.finish(GatewayCliKey::Codex);
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.output_tokens, Some(40));
        // First envelope wins (`resp_x` from the head's own response.completed).
        assert_eq!(usage.envelope_id.as_deref(), Some("resp_x"));
    }

    #[test]
    fn flattened_stream_terminal_survives_buffer_overflow_before_trailing_events() {
        // Terminal lands mid-stream; a later oversized chunk overflows the
        // bounded buffer. The pre-discard observation must keep the verdict.
        let padding = "x".repeat(MAX_SSE_USAGE_BUFFER_BYTES);
        let head = flattened_codex_relay_stream();
        let tail = format!(
            "event: codex.event.balance data: {{\"type\":\"codex.event.balance\",\"note\":\"{padding}\"}}"
        );
        let mut collector = SseUsageCollector::default();
        collector.observe_chunk(head.as_bytes());
        assert!(
            collector.terminal_kind().is_none(),
            "a sub-cap flattened head stays in the residual buffer before drain"
        );
        collector.observe_chunk(tail.as_bytes());
        assert_eq!(
            collector.terminal_kind(),
            Some(SseTerminalKind::Success),
            "overflow pre-discard observation must keep the earlier terminal verdict"
        );
    }

    #[test]
    fn flattened_stream_classified_when_streamed_in_small_chunks() {
        // The same flattened stream delivered in small slices: blocks never
        // form, the buffer overflows repeatedly, and the terminal must still
        // surface by the time the collector is drained.
        let stream = flattened_codex_relay_stream();
        let mut collector = SseUsageCollector::default();
        for chunk in stream.as_bytes().chunks(64) {
            collector.observe_chunk(chunk);
        }
        assert!(
            collector.terminal_kind().is_none(),
            "no terminal verdict before drain for an unseparated residual"
        );
        collector.drain_terminal();
        assert_eq!(
            collector.terminal_kind(),
            Some(SseTerminalKind::Success),
            "drain_terminal must classify the flattened residual"
        );
    }

    #[test]
    fn flattened_stream_usage_merged_from_finish_residual() {
        let stream = flattened_codex_relay_stream();
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(GatewayCliKey::Codex, stream.as_bytes());
        let usage = collector.finish(GatewayCliKey::Codex);
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.output_tokens, Some(40));
        assert_eq!(usage.envelope_id.as_deref(), Some("resp_x"));
    }

    #[test]
    fn flattened_stream_terminal_found_mid_stream_before_long_tail() {
        // A flattened stream whose `response.completed` is followed by a long
        // relay tail: the tail must not push the terminal out of the window.
        let padding = "y".repeat(MAX_SSE_USAGE_BUFFER_BYTES + 4096);
        let tail = format!(
            "event: codex.event.balance data: {{\"type\":\"codex.event.balance\",\"note\":\"{padding}\"}}"
        );
        let stream = format!("{} {}", flattened_codex_relay_stream(), tail);
        let mut collector = SseUsageCollector::default();
        collector.observe_chunk(stream.as_bytes());
        assert_eq!(
            collector.terminal_kind(),
            Some(SseTerminalKind::Success),
            "terminal before an oversized tail must be found during in-place scan"
        );
    }

    #[test]
    fn flattened_stream_with_line_break_separators_is_classified() {
        // A milder relay quirk: events separated by a single newline but with no
        // blank-line delimiter between events. take_sse_block still never forms
        // a block; the flattened scanner must handle it because a field token at
        // a line start is preceded by whitespace.
        let stream = flattened_codex_relay_stream()
            .split(" event: ")
            .collect::<Vec<_>>()
            .join("\nevent: ");
        assert_eq!(
            sse_block_classify_terminal(stream.as_bytes()),
            Some(SseTerminalKind::Success)
        );
    }

    #[test]
    fn flattened_scanner_survives_adversarial_inputs() {
        // Degenerate or truncated inputs must classify without panicking; a
        // trailing `event:`/`data:` token with no value is simply ignored.
        let inputs: &[&[u8]] = &[
            b"",
            b"data:",
            b"data: ",
            b"event:",
            b"event: ",
            b"event: data: {}",
            b"data: {invalid json",
            b"data: {\"error\":",
            b"event: response.completed data: ",
            b"event: response.completed data: {\"type\":\"response.completed\"",
            b"\xff\xfe data: {}",
            b"data: {} data: [DONE] data: {",
        ];
        for input in inputs {
            let _ = sse_block_classify_terminal(input);
            let mut collector = SseUsageCollector::default();
            collector.push_chunk(GatewayCliKey::Codex, input);
            collector.observe_chunk(input);
            let _ = collector.finish(GatewayCliKey::Codex);
        }
        // Invalid UTF-8 before a genuine terminal still classifies (lossy read);
        // a truncated completed payload stays None instead of guessing Success.
        assert_eq!(
            sse_block_classify_terminal(b"\xff\xfe event: error data: {}"),
            Some(SseTerminalKind::Failed)
        );
        assert_eq!(
            sse_block_classify_terminal(
                b"event: response.completed data: {\"type\":\"response.completed\""
            ),
            None
        );
    }

    #[test]
    fn sse_block_classify_terminal_response_completed_with_status_failed_is_failed() {
        // The gateway's own cross-protocol signal: a `response.completed` event
        // whose payload carries `status=failed` must read as Failed, not Success.
        assert_eq!(
            sse_block_classify_terminal(
                b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"failed\",\"error\":{\"message\":\"boom\"}}}\n\n"
            ),
            Some(SseTerminalKind::Failed)
        );
    }

    #[test]
    fn sse_block_classify_terminal_response_completed_with_status_incomplete_is_incomplete() {
        // `transformer/stream.rs` emits `response.completed` + `status=incomplete`
        // for cross-protocol Responses-incomplete. This must NOT collapse to
        // Completed (the original bug) or Failed.
        assert_eq!(
            sse_block_classify_terminal(
                b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"incomplete\"}}\n\n"
            ),
            Some(SseTerminalKind::Incomplete)
        );
    }

    #[test]
    fn sse_block_classify_terminal_response_completed_with_status_cancelled_is_canceled() {
        assert_eq!(
            sse_block_classify_terminal(
                b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"cancelled\"}}\n\n"
            ),
            Some(SseTerminalKind::Canceled)
        );
    }

    #[test]
    fn sse_block_classify_terminal_response_incomplete_event_is_incomplete() {
        assert_eq!(
            sse_block_classify_terminal(
                b"event: response.incomplete\ndata: {\"type\":\"response.incomplete\",\"response\":{\"id\":\"r\",\"status\":\"incomplete\"}}\n\n"
            ),
            Some(SseTerminalKind::Incomplete)
        );
    }

    #[test]
    fn sse_block_classify_terminal_top_level_error_field_is_failed() {
        // Aligned with gateway_json_reports_error: a non-null top-level `error`
        // makes this a Failed terminal even with no terminal event name.
        assert_eq!(
            sse_block_classify_terminal(b"data: {\"error\":{\"message\":\"boom\"}}\n\n"),
            Some(SseTerminalKind::Failed)
        );
    }

    #[test]
    fn sse_block_classify_terminal_nested_response_error_is_failed() {
        assert_eq!(
            sse_block_classify_terminal(
                b"data: {\"response\":{\"error\":{\"message\":\"boom\"}}}\n\n"
            ),
            Some(SseTerminalKind::Failed)
        );
    }

    #[test]
    fn sse_block_classify_terminal_top_level_status_failed_is_failed() {
        assert_eq!(
            sse_block_classify_terminal(b"data: {\"status\":\"failed\"}\n\n"),
            Some(SseTerminalKind::Failed)
        );
    }

    #[test]
    fn sse_block_classify_terminal_nested_response_status_incomplete_is_incomplete() {
        assert_eq!(
            sse_block_classify_terminal(b"data: {\"response\":{\"status\":\"incomplete\"}}\n\n"),
            Some(SseTerminalKind::Incomplete)
        );
    }

    #[test]
    fn sse_block_classify_terminal_success_terminals_map_to_success() {
        assert_eq!(
            sse_block_classify_terminal(b"data: [DONE]\n\n"),
            Some(SseTerminalKind::Success)
        );
        assert_eq!(
            sse_block_classify_terminal(
                b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
            ),
            Some(SseTerminalKind::Success)
        );
        assert_eq!(
            sse_block_classify_terminal(
                b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\"}}\n\n"
            ),
            Some(SseTerminalKind::Success)
        );
    }

    #[test]
    fn sse_block_classify_terminal_delta_events_are_none() {
        assert_eq!(
            sse_block_classify_terminal(
                b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n"
            ),
            None
        );
    }

    /// Regression for issue #318 (2026-09-06): a `response.reasoning_summary_part.done`
    /// delta carries an item-level `"status":"incomplete"` (meaning "this reasoning
    /// part is not the final one"). That top-level `status` must NOT be treated as
    /// a stream-terminal Incomplete, otherwise a healthy stream is misclassified
    /// before the real `response.completed` arrives.
    #[test]
    fn sse_block_classify_terminal_ignores_responses_delta_status_incomplete() {
        let delta = b"event: response.reasoning_summary_part.done data: {\"type\":\"response.reasoning_summary_part.done\",\"status\":\"incomplete\",\"item_id\":\"rs_x\",\"output_index\":0,\"part\":{\"type\":\"summary_text\",\"text\":\"hi\"}}";
        assert_eq!(
            sse_block_classify_terminal(delta),
            None,
            "delta item-level status:incomplete must not be a terminal"
        );
        // Same delta without the `event:` field (JSON `type` fallback path).
        let delta_no_event =
            b"data: {\"type\":\"response.reasoning_summary_part.done\",\"status\":\"incomplete\"}";
        assert_eq!(
            sse_block_classify_terminal(delta_no_event),
            None,
            "JSON-only delta item-level status:incomplete must not be a terminal"
        );
        // A genuine top-level `status:incomplete` on a non-Responses envelope is
        // still recognized (the guard is scoped to `response.*` deltas only).
        assert_eq!(
            sse_block_classify_terminal(b"data: {\"status\":\"incomplete\"}\n\n"),
            Some(SseTerminalKind::Incomplete)
        );
        // A non-standard `response.*` delta carrying `status:"failed"` (no `error`
        // field) is still a genuine failure: only item-level `status:"incomplete"`
        // is suppressed on deltas, never Failed/Canceled.
        assert_eq!(
            sse_block_classify_terminal(
                b"event: response.custom_reject data: {\"type\":\"response.custom_reject\",\"status\":\"failed\",\"reason\":\"policy\"}"
            ),
            Some(SseTerminalKind::Failed)
        );
        assert_eq!(
            sse_block_classify_terminal(
                b"event: response.custom_cancel data: {\"type\":\"response.custom_cancel\",\"status\":\"cancelled\"}"
            ),
            Some(SseTerminalKind::Canceled)
        );
    }

    /// End-to-end guard: a delimiter-free stream larger than the bounded window
    /// whose terminal event straddles the 256 KiB overflow boundary (its JSON
    /// is ~97 KiB, so the overflow flush lands inside it) must still yield
    /// Success and its full usage. The aligned overflow flush
    /// ([`flattened_flush_boundary`]) retains the incomplete terminal JSON and
    /// buffers later chunks until it completes, so the collector itself
    /// recovers the verdict without help — `write_streaming_body`'s snapshot
    /// re-scan remains as a defensive backstop for degenerate streams whose
    /// incomplete tail alone would fill the window.
    #[test]
    fn oversized_completed_json_straddling_overflow_boundary_yields_success() {
        let delta = "event: response.custom_tool_call_input.delta data: {\"type\":\"response.custom_tool_call_input.delta\",\"delta\":\"x\",\"item_id\":\"ctc_1\"} ";
        let mut body = String::new();
        // Start the terminal near 184 KiB (like the real krill relay bodies),
        // and make its JSON ~97 KiB (padding inside a string field) so it
        // straddles the 256 KiB overflow boundary — the exact failure shape.
        while body.len() < 184 * 1024 {
            body.push_str(delta);
        }
        let padding = "x".repeat(97 * 1024);
        body.push_str("event: response.completed data: ");
        body.push_str(&format!(
            "{{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_1\",\"status\":\"completed\",\"note\":\"{padding}\",\"usage\":{{\"input_tokens\":100,\"output_tokens\":50,\"total_tokens\":150}}}},\"sequence_number\":9}}"
        ));
        let bytes = body.into_bytes();

        // The whole-body flattened scan reliably finds the terminal.
        assert_eq!(
            sse_block_classify_terminal(&bytes),
            Some(SseTerminalKind::Success)
        );

        // The bounded chunked collector recovers the verdict too: the overflow
        // flush cuts after the last complete event and keeps the incomplete
        // terminal JSON buffered until the trailing chunks finish it.
        let mut collector = SseUsageCollector::with_provider_type(None);
        for chunk in bytes.chunks(64 * 1024) {
            collector.observe_chunk(chunk);
        }
        collector.drain_terminal();
        assert_eq!(
            collector.terminal_kind(),
            Some(SseTerminalKind::Success),
            "aligned overflow flush must preserve the straddling terminal event"
        );

        // Usage rides in the same terminal JSON, so it must survive as well.
        let mut collector = SseUsageCollector::with_provider_type(None);
        for chunk in bytes.chunks(64 * 1024) {
            collector.push_chunk(GatewayCliKey::Codex, chunk);
        }
        let usage = collector.finish(GatewayCliKey::Codex);
        assert_eq!(
            usage.total_tokens(),
            Some(150),
            "usage inside the straddling terminal JSON must be recovered"
        );
    }

    /// The overflow flush must cut after the last complete event JSON, so a
    /// complete terminal is observed immediately while an incomplete trailing
    /// event stays buffered; a `data:` sequence inside a JSON string without
    /// leading whitespace is not a field token and must not become a cut point.
    #[test]
    fn flattened_flush_boundary_cuts_after_last_complete_event() {
        let buffer =
            b"event: a data: {\"x\":1} event: b data: {\"y\":2} event: response.completed data: {";
        let complete_prefix = "event: a data: {\"x\":1} event: b data: {\"y\":2}";
        assert_eq!(flattened_flush_boundary(buffer), complete_prefix.len());

        let tricky = b"event: a data: {\"s\":\"nodata: here\"} event: b data: {";
        let tricky_complete = "event: a data: {\"s\":\"nodata: here\"}";
        assert_eq!(
            flattened_flush_boundary(tricky),
            tricky_complete.len(),
            "boundary must skip the in-string non-whitespace-prefixed data: token"
        );

        // No complete event at all: nothing can be flushed safely.
        assert_eq!(flattened_flush_boundary(b"{\"just\":\"json bytes\"}"), 0);
        assert_eq!(flattened_flush_boundary(b"event: a data: {\"cut"), 0);
        assert_eq!(flattened_flush_boundary(b""), 0);
    }

    /// Regression: invalid UTF-8 in the buffered flattened stream must not
    /// panic `flattened_flush_boundary` (issue #318 review). The lossy-string
    /// path made the string longer than the original buffer, so a boundary
    /// computed on it could exceed `buffer.len()` and panic the caller's slice.
    /// Now invalid UTF-8 returns boundary 0 (flush nothing) and the overflow
    /// path observes the raw residual without slicing past its end.
    #[test]
    fn flattened_flush_boundary_invalid_utf8_does_not_panic() {
        let mut buffer: Vec<u8> = Vec::new();
        // A complete event followed by an invalid UTF-8 byte, sized past the
        // 256 KiB overflow threshold so the flush path runs.
        let event = b"event: a data: {\"x\":1} ";
        while buffer.len() < MAX_SSE_USAGE_BUFFER_BYTES {
            buffer.extend_from_slice(event);
        }
        buffer.push(0xff);
        buffer.push(0xfe);
        // No panic; boundary never exceeds the original buffer length.
        let boundary = flattened_flush_boundary(&buffer);
        assert!(
            boundary <= buffer.len(),
            "boundary {boundary} must not exceed buffer len {}",
            buffer.len()
        );

        // The chunked collector itself must not panic on the same input.
        let mut collector = SseUsageCollector::with_provider_type(None);
        collector.observe_chunk(&buffer);
        collector.drain_terminal();
    }
}
