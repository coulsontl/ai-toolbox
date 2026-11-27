//! Request-observability markers for native/hosted tools and gateway rewrites.
//!
//! Everything collected here is persisted **only** in the JSONL request detail
//! (`GatewayRequestLogDetail`). It deliberately never enters
//! `proxy_request_logs`: the request list reads that compact table, and adding
//! columns for these markers would require a migration so that a detail-only
//! diagnostic could be filtered on. The detail dialog and the export are the
//! only consumers.
//!
//! Collection must not depend on body storage: `store_request_body` and
//! `store_response_body` are off by default, so a design that inspected saved
//! bodies after the fact would always report "nothing" for most users. The
//! request side reuses the `Value` already parsed by
//! `build_upstream_body_for_provider`; the response side scans the body it is
//! already parsing, or a bounded tail scanner inside the streaming writer.

use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::coding::proxy_gateway::types::{GatewayNativeToolRecord, GatewayRequestRewrite};
use serde_json::Value;

/// Canonical tool kinds. A single vocabulary shared by every protocol keeps the
/// detail dialog from having to translate per-dialect names.
pub(super) const TOOL_WEB_SEARCH: &str = "web_search";
pub(super) const TOOL_IMAGE_GENERATION: &str = "image_generation";
pub(super) const TOOL_CODE_INTERPRETER: &str = "code_interpreter";
pub(super) const TOOL_COMPUTER_USE: &str = "computer_use";
pub(super) const TOOL_FILE_SEARCH: &str = "file_search";
pub(super) const TOOL_MCP: &str = "mcp";
pub(super) const TOOL_LOCAL_SHELL: &str = "local_shell";

/// Rewrite kinds recorded on the request side.
pub(super) const REWRITE_PROMPT_CACHE_KEY: &str = "prompt_cache_key";
pub(super) const REWRITE_SERVICE_TIER: &str = "service_tier";
pub(super) const REWRITE_CCH_STRIPPED: &str = "billing_cch_stripped";
pub(super) const REWRITE_CAPSULE_EXPANDED: &str = "compaction_capsule_expanded";
pub(super) const REWRITE_COMPACTION_SUMMARIZED: &str = "compaction_summarized";
pub(super) const REWRITE_NATIVE_TOOL_FILTERED: &str = "native_tool_filtered";
pub(super) const REWRITE_LOSSY_CONVERSION: &str = "lossy_conversion";
/// Plugin middleware stages, recorded by [`RequestObservations::merge_middleware_rewrites`].
pub(super) const REWRITE_PLUGIN_MIDDLEWARE_REQUEST: &str = "plugin_middleware_request";
pub(super) const REWRITE_PLUGIN_MIDDLEWARE_PROVIDER: &str = "plugin_middleware_provider";
pub(super) const REWRITE_PLUGIN_MIDDLEWARE_RESPONSE: &str = "plugin_middleware_response";
/// The plugin provider's host reported a wire request that differs from what
/// the gateway itself built (URL, headers or body).
pub(super) const REWRITE_PLUGIN_PROVIDER_FETCH: &str = "plugin_provider_fetch";

/// Bounds that keep a malformed or hostile request from bloating the JSONL
/// detail. Both lists are diagnostics, not a ledger.
pub(super) const MAX_TOOL_RECORDS: usize = 16;
pub(super) const MAX_REWRITE_RECORDS: usize = 32;
const MAX_DETAIL_CHARS: usize = 256;
/// Bytes carried across stream chunks so a marker split between two chunks is
/// still seen. Longest marker is `"image_generation_call"` (21 bytes).
const STREAM_TAIL_BYTES: usize = 96;

/// Runtime accumulator handed through request preparation and the response
/// path. `native_tools` is keyed by kind; `rewrites` is append-only and
/// capped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RequestObservations {
    pub(super) native_tools: Vec<GatewayNativeToolRecord>,
    pub(super) rewrites: Vec<GatewayRequestRewrite>,
}

impl RequestObservations {
    /// Records the tools declared by `body`, interpreted as `protocol`.
    pub(super) fn observe_declared(&mut self, protocol: AiProtocol, body: &Value) {
        self.native_tools = declared_tools(protocol, body);
    }

    /// Marks whether each declared tool survived into the forwarded body.
    ///
    /// Only declared records are updated: a record created by observation of an
    /// actual call (`declared: false`) was never in the client's request, so
    /// "forwarded" is meaningless for it.
    pub(super) fn observe_forwarded(&mut self, protocol: AiProtocol, body: &Value) {
        let forwarded = declared_tools(protocol, body);
        for record in self.native_tools.iter_mut().filter(|record| record.declared) {
            record.forwarded = forwarded
                .iter()
                .any(|forwarded| forwarded.kind == record.kind);
        }
    }

    /// Marks the tools the upstream actually invoked in its reply.
    pub(super) fn observe_used(&mut self, protocol: AiProtocol, body: &Value) {
        for kind in used_tools(protocol, body) {
            self.mark_used(&kind, true);
        }
    }

    /// Streaming counterpart: markers seen in raw SSE bytes.
    pub(super) fn observe_used_markers(&mut self, kinds: &[&'static str]) {
        for kind in kinds {
            self.mark_used(kind, true);
        }
    }

    fn mark_used(&mut self, kind: &str, used: bool) {
        if let Some(record) = self
            .native_tools
            .iter_mut()
            .find(|record| record.kind == kind)
        {
            record.used = used;
            return;
        }
        if self.native_tools.len() >= MAX_TOOL_RECORDS {
            return;
        }
        // A tool call can be observed even when the client never declared it
        // (for example a provider-side default search). Keep it: the point of
        // the marker is what actually ran.
        self.native_tools.push(GatewayNativeToolRecord {
            kind: kind.to_string(),
            declared: false,
            forwarded: false,
            used,
        });
    }

    pub(super) fn push_rewrite(&mut self, kind: &str, detail: Option<String>) {
        if self.rewrites.len() >= MAX_REWRITE_RECORDS {
            return;
        }
        self.rewrites.push(GatewayRequestRewrite {
            kind: kind.to_string(),
            detail: detail.map(|value| truncate(value, MAX_DETAIL_CHARS)),
        });
    }

    /// Records `lossy_warnings` verbatim (already human-readable).
    pub(super) fn push_lossy_warnings(&mut self, warnings: &[String]) {
        for warning in warnings {
            self.push_rewrite(REWRITE_LOSSY_CONVERSION, Some(warning.clone()));
        }
    }

    /// Adds a `native_tool_filtered` entry once per tool that was declared but
    /// did not reach the upstream.
    pub(super) fn push_filtered_tools(&mut self) {
        let filtered: Vec<String> = self
            .native_tools
            .iter()
            .filter(|record| record.declared && !record.forwarded)
            .map(|record| record.kind.clone())
            .collect();
        for kind in filtered {
            self.push_rewrite(REWRITE_NATIVE_TOOL_FILTERED, Some(kind));
        }
    }

    /// Appends the plugin middleware stages that actually changed the payload.
    ///
    /// Called twice by design: once when the gateway response is assembled
    /// (request/provider/response stages already known) and once right before
    /// the detail is written, because a streaming `onEvent` middleware can only
    /// report its rewrites after `write_streaming_body` consumed the stream.
    /// Entries are deduplicated so the second merge never doubles a marker.
    pub(super) fn merge_middleware_rewrites(
        &mut self,
        request: &[String],
        provider: &[String],
        response: &[String],
    ) {
        for (kind, names) in [
            (REWRITE_PLUGIN_MIDDLEWARE_REQUEST, request),
            (REWRITE_PLUGIN_MIDDLEWARE_PROVIDER, provider),
            (REWRITE_PLUGIN_MIDDLEWARE_RESPONSE, response),
        ] {
            if names.is_empty() {
                continue;
            }
            // `push_rewrite` truncates the detail before storing it, so the
            // duplicate check must compare the same truncated form; otherwise a
            // long detail is inserted again on the second merge.
            let detail = truncate(names.join(", "), MAX_DETAIL_CHARS);
            if self
                .rewrites
                .iter()
                .any(|rewrite| rewrite.kind == kind && rewrite.detail.as_deref() == Some(&detail))
            {
                continue;
            }
            self.push_rewrite(kind, Some(detail));
        }
    }
}

fn truncate(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push('…');
    truncated
}

/// Tools **declared** in a request body, in canonical kinds.
///
/// The same table serves the forwarded check: the final upstream body is in the
/// target protocol, so calling this again with `target_protocol` answers "did
/// the declaration survive?".
pub(super) fn declared_tools(protocol: AiProtocol, body: &Value) -> Vec<GatewayNativeToolRecord> {
    let mut kinds: Vec<&'static str> = Vec::new();
    match protocol {
        AiProtocol::OpenAiResponses => {
            for tool in body
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(tool_type) = tool.get("type").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(kind) = responses_declared_kind(tool_type) {
                    kinds.push(kind);
                }
            }
        }
        AiProtocol::AnthropicMessages => {
            for tool in body
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(tool_type) = tool.get("type").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(kind) = anthropic_declared_kind(tool_type) {
                    kinds.push(kind);
                }
            }
        }
        AiProtocol::GeminiNative => {
            for tool in body
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for (key, kind) in [
                    ("googleSearch", TOOL_WEB_SEARCH),
                    ("google_search", TOOL_WEB_SEARCH),
                    ("codeExecution", TOOL_CODE_INTERPRETER),
                    ("code_execution", TOOL_CODE_INTERPRETER),
                    ("urlContext", TOOL_WEB_SEARCH),
                ] {
                    if tool.get(key).is_some() {
                        kinds.push(kind);
                    }
                }
            }
        }
        AiProtocol::OpenAiChat => {}
    }

    dedupe_records(kinds, |kind| GatewayNativeToolRecord {
        kind: kind.to_string(),
        declared: true,
        forwarded: false,
        used: false,
    })
}

fn responses_declared_kind(tool_type: &str) -> Option<&'static str> {
    match tool_type {
        "web_search" | "web_search_preview" => Some(TOOL_WEB_SEARCH),
        "image_generation" => Some(TOOL_IMAGE_GENERATION),
        "code_interpreter" => Some(TOOL_CODE_INTERPRETER),
        "computer_use_preview" | "computer" => Some(TOOL_COMPUTER_USE),
        "file_search" => Some(TOOL_FILE_SEARCH),
        "mcp" => Some(TOOL_MCP),
        "local_shell" => Some(TOOL_LOCAL_SHELL),
        _ => None,
    }
}

fn anthropic_declared_kind(tool_type: &str) -> Option<&'static str> {
    match tool_type {
        "web_search_20250305" | "web_search_2025_03_05" | "web_search" => Some(TOOL_WEB_SEARCH),
        "mcp" => Some(TOOL_MCP),
        _ => None,
    }
}

/// Tools the upstream **invoked**, read from a reply body in `protocol`.
///
/// Scanned on the upstream (target) body rather than the converted client body:
/// cross-protocol conversion drops hosted-tool markers by design, and the point
/// is what the upstream actually did.
pub(super) fn used_tools(protocol: AiProtocol, body: &Value) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    collect_used_markers(protocol, body, &mut kinds);
    kinds.sort_unstable();
    kinds.dedup();
    kinds
}

/// Recurses through the reply body looking for type-tagged items.
///
/// Only objects carrying a `type` are candidates for the Responses/Anthropic
/// tables; Gemini additionally keys off dedicated fields. A model that writes a
/// JSON example in its *message text* cannot trigger this: that text lands in a
/// string value, and strings are never descended into.
fn collect_used_markers(protocol: AiProtocol, value: &Value, kinds: &mut Vec<&'static str>) {
    match value {
        Value::Object(object) => {
            if let Some(item_type) = object.get("type").and_then(Value::as_str) {
                match protocol {
                    AiProtocol::OpenAiResponses => {
                        if let Some(kind) = responses_used_kind(item_type) {
                            kinds.push(kind);
                        }
                    }
                    AiProtocol::AnthropicMessages => {
                        if let Some(kind) = anthropic_used_kind(object, item_type) {
                            kinds.push(kind);
                        }
                    }
                    AiProtocol::GeminiNative | AiProtocol::OpenAiChat => {}
                }
            }
            // Gemini marks search in the candidate, code execution in parts.
            if protocol == AiProtocol::GeminiNative {
                if object.contains_key("groundingMetadata")
                    || object.contains_key("grounding_metadata")
                {
                    kinds.push(TOOL_WEB_SEARCH);
                }
                if object.contains_key("executableCode") || object.contains_key("executable_code") {
                    kinds.push(TOOL_CODE_INTERPRETER);
                }
                if object.contains_key("codeExecutionResult")
                    || object.contains_key("code_execution_result")
                {
                    kinds.push(TOOL_CODE_INTERPRETER);
                }
            }
            for child in object.values() {
                collect_used_markers(protocol, child, kinds);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_used_markers(protocol, item, kinds);
            }
        }
        _ => {}
    }
}

fn responses_used_kind(item_type: &str) -> Option<&'static str> {
    match item_type {
        "web_search_call" => Some(TOOL_WEB_SEARCH),
        "image_generation_call" => Some(TOOL_IMAGE_GENERATION),
        "code_interpreter_call" => Some(TOOL_CODE_INTERPRETER),
        "computer_call" => Some(TOOL_COMPUTER_USE),
        "file_search_call" => Some(TOOL_FILE_SEARCH),
        "mcp_call" | "mcp_list_tools" | "mcp_approval_request" => Some(TOOL_MCP),
        "local_shell_call" => Some(TOOL_LOCAL_SHELL),
        _ => None,
    }
}

fn anthropic_used_kind(
    object: &serde_json::Map<String, Value>,
    item_type: &str,
) -> Option<&'static str> {
    match item_type {
        "web_search_tool_result" => Some(TOOL_WEB_SEARCH),
        "mcp_tool_use" | "mcp_tool_result" => Some(TOOL_MCP),
        // `server_tool_use` is generic; the embedded name says which tool.
        "server_tool_use" => {
            object
                .get("name")
                .and_then(Value::as_str)
                .and_then(|name| match name {
                    "web_search" => Some(TOOL_WEB_SEARCH),
                    _ => None,
                })
        }
        _ => None,
    }
}

fn dedupe_records(
    kinds: Vec<&'static str>,
    build: impl Fn(&'static str) -> GatewayNativeToolRecord,
) -> Vec<GatewayNativeToolRecord> {
    let mut records = Vec::new();
    for kind in kinds {
        if records
            .iter()
            .any(|record: &GatewayNativeToolRecord| record.kind == kind)
        {
            continue;
        }
        if records.len() >= MAX_TOOL_RECORDS {
            break;
        }
        records.push(build(kind));
    }
    records
}

/// Streaming marker scanner.
///
/// SSE/WS bodies are forwarded chunk by chunk; this keeps a bounded tail so a
/// marker split across two chunks is still recognized. Scanning is a raw byte
/// search, not JSON parsing, so it is restricted to the dialect the upstream
/// actually speaks: a model that merely *writes* `"web_search_call"` inside an
/// assistant message must not be reported as having run one. Within a matching
/// dialect the marker can only appear as a JSON type value.
pub(super) struct UsedToolScanner {
    protocol: AiProtocol,
    tail: Vec<u8>,
    kinds: Vec<&'static str>,
}

impl UsedToolScanner {
    pub(super) fn new(protocol: AiProtocol) -> Self {
        Self {
            protocol,
            tail: Vec::new(),
            kinds: Vec::new(),
        }
    }

    pub(super) fn push_chunk(&mut self, chunk: &[u8]) {
        let mut haystack = Vec::with_capacity(self.tail.len() + chunk.len());
        haystack.extend_from_slice(&self.tail);
        haystack.extend_from_slice(chunk);
        for (protocol, marker, kind) in STREAM_MARKERS {
            if *protocol != self.protocol || self.kinds.contains(kind) {
                continue;
            }
            if contains_bytes(&haystack, marker.as_bytes()) {
                self.kinds.push(kind);
            }
        }
        let keep = STREAM_TAIL_BYTES.min(haystack.len());
        self.tail = haystack[haystack.len() - keep..].to_vec();
    }

    pub(super) fn kinds(&self) -> &[&'static str] {
        &self.kinds
    }
}

/// Markers per wire dialect. A marker only counts when the upstream speaks that
/// dialect; see [`UsedToolScanner`].
const STREAM_MARKERS: &[(AiProtocol, &str, &str)] = &[
    (AiProtocol::OpenAiResponses, "\"web_search_call\"", TOOL_WEB_SEARCH),
    (AiProtocol::OpenAiResponses, "\"image_generation_call\"", TOOL_IMAGE_GENERATION),
    (AiProtocol::OpenAiResponses, "\"code_interpreter_call\"", TOOL_CODE_INTERPRETER),
    (AiProtocol::OpenAiResponses, "\"computer_call\"", TOOL_COMPUTER_USE),
    (AiProtocol::OpenAiResponses, "\"file_search_call\"", TOOL_FILE_SEARCH),
    (AiProtocol::OpenAiResponses, "\"mcp_call\"", TOOL_MCP),
    (AiProtocol::OpenAiResponses, "\"mcp_list_tools\"", TOOL_MCP),
    (AiProtocol::OpenAiResponses, "\"local_shell_call\"", TOOL_LOCAL_SHELL),
    (AiProtocol::AnthropicMessages, "\"web_search_tool_result\"", TOOL_WEB_SEARCH),
    (AiProtocol::AnthropicMessages, "\"mcp_tool_use\"", TOOL_MCP),
    (AiProtocol::AnthropicMessages, "\"mcp_tool_result\"", TOOL_MCP),
    (AiProtocol::GeminiNative, "\"groundingMetadata\"", TOOL_WEB_SEARCH),
    (AiProtocol::GeminiNative, "\"grounding_metadata\"", TOOL_WEB_SEARCH),
    (AiProtocol::GeminiNative, "\"executableCode\"", TOOL_CODE_INTERPRETER),
    (AiProtocol::GeminiNative, "\"executable_code\"", TOOL_CODE_INTERPRETER),
    (AiProtocol::GeminiNative, "\"codeExecutionResult\"", TOOL_CODE_INTERPRETER),
    (AiProtocol::GeminiNative, "\"code_execution_result\"", TOOL_CODE_INTERPRETER),
];

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_and_forwarded_track_a_filtered_tool() {
        let original = json!({
            "tools": [
                {"type": "web_search"},
                {"type": "function", "name": "shell"}
            ]
        });
        let forwarded = json!({
            "tools": [
                {"type": "function", "name": "shell"}
            ]
        });
        let mut observations = RequestObservations::default();
        observations.observe_declared(AiProtocol::OpenAiResponses, &original);
        observations.observe_forwarded(AiProtocol::OpenAiResponses, &forwarded);
        observations.push_filtered_tools();

        assert_eq!(observations.native_tools.len(), 1);
        assert_eq!(observations.native_tools[0].kind, TOOL_WEB_SEARCH);
        assert!(observations.native_tools[0].declared);
        assert!(!observations.native_tools[0].forwarded);
        assert_eq!(observations.rewrites.len(), 1);
        assert_eq!(observations.rewrites[0].kind, REWRITE_NATIVE_TOOL_FILTERED);
        assert_eq!(
            observations.rewrites[0].detail.as_deref(),
            Some(TOOL_WEB_SEARCH)
        );
    }

    #[test]
    fn used_markers_are_read_for_each_protocol() {
        let responses = json!({
            "output": [{"type": "web_search_call"}, {"type": "message"}]
        });
        assert_eq!(
            used_tools(AiProtocol::OpenAiResponses, &responses),
            vec![TOOL_WEB_SEARCH]
        );

        let anthropic = json!({
            "content": [
                {"type": "server_tool_use", "name": "web_search"},
                {"type": "web_search_tool_result"}
            ]
        });
        assert_eq!(
            used_tools(AiProtocol::AnthropicMessages, &anthropic),
            vec![TOOL_WEB_SEARCH]
        );

        let gemini = json!({
            "candidates": [{
                "groundingMetadata": {"webSearchQueries": ["rust"]},
                "content": {"parts": [{"executableCode": {"language": "python"}}]}
            }]
        });
        let mut kinds = used_tools(AiProtocol::GeminiNative, &gemini);
        kinds.sort_unstable();
        assert_eq!(kinds, vec![TOOL_CODE_INTERPRETER, TOOL_WEB_SEARCH]);
    }

    #[test]
    fn an_undeclared_call_still_gets_a_record() {
        let reply = json!({"output": [{"type": "web_search_call"}]});
        let mut observations = RequestObservations::default();
        observations.observe_used(AiProtocol::OpenAiResponses, &reply);

        assert_eq!(observations.native_tools.len(), 1);
        assert!(!observations.native_tools[0].declared);
        assert!(observations.native_tools[0].used);
    }

    #[test]
    fn stream_scanner_sees_a_marker_split_across_chunks() {
        let mut scanner = UsedToolScanner::new(AiProtocol::OpenAiResponses);
        scanner.push_chunk(b"data: {\"type\":\"web_sea");
        scanner.push_chunk(b"rch_call\"}\n\n");

        assert_eq!(scanner.kinds(), &[TOOL_WEB_SEARCH]);
    }

    #[test]
    fn stream_scanner_ignores_another_dialects_marker() {
        // A Gemini reply that merely *mentions* `web_search_call` in text is not
        // a Responses item; scanning every dialect would report a false call.
        let mut scanner = UsedToolScanner::new(AiProtocol::GeminiNative);
        scanner.push_chunk(b"{\"text\":\"call \\\"web_search_call\\\" next\"}");

        assert!(scanner.kinds().is_empty());
    }

    #[test]
    fn rewrite_detail_is_truncated() {
        let mut observations = RequestObservations::default();
        observations.push_rewrite(REWRITE_LOSSY_CONVERSION, Some("x".repeat(400)));

        let detail = observations.rewrites[0].detail.as_deref().unwrap();
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS + 1);
    }

    #[test]
    fn middleware_rewrite_merge_compares_truncated_details() {
        let mut observations = RequestObservations::default();
        let names = vec!["x".repeat(MAX_DETAIL_CHARS + 50)];
        observations.merge_middleware_rewrites(&names, &[], &[]);
        // The stored detail is truncated, so the second (identical) merge must
        // recognize it instead of appending a duplicate.
        observations.merge_middleware_rewrites(&names, &[], &[]);

        assert_eq!(observations.rewrites.len(), 1);
        let detail = observations.rewrites[0].detail.as_deref().unwrap();
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS + 1);
    }

    #[test]
    fn rewrite_list_is_capped() {
        let mut observations = RequestObservations::default();
        for index in 0..(MAX_REWRITE_RECORDS + 5) {
            observations.push_rewrite(REWRITE_PROMPT_CACHE_KEY, Some(index.to_string()));
        }

        assert_eq!(observations.rewrites.len(), MAX_REWRITE_RECORDS);
    }
}
