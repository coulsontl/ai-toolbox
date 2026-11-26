use super::gemini::{
    gemini_finish_to_openai_finish, gemini_stream_error, gemini_usage_to_llm, llm_usage_to_gemini,
    merge_gemini_function_call_part, synthesize_gemini_tool_id,
};
use super::kernel::ConversionContext;
use super::llm::Usage;
use super::llm::{TOOL_TYPE_FUNCTION, TOOL_TYPE_RESPONSES_CUSTOM_TOOL};
use super::openai::codex_tools::{
    custom_tool_input_from_chat_arguments, is_custom_tool_chat_name,
    response_tool_added_item_from_chat_name, response_tool_done_item_from_chat_name,
    response_tool_item_id_from_chat_name, CodexToolContext,
};
use super::shared::signature::{
    decode_signature_for, encode_signature, SignatureProvider, DEFAULT_GEMINI_THOUGHT_SIGNATURE,
};
use super::shared::{
    extract_reasoning_field_text, split_leading_think_block, strip_leading_think_open_tag,
};
use super::sse::{parse_sse_block, sse_done, sse_event, SseBuffer};
use super::types::{AiProtocol, ConversionRoute};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone)]
pub enum UnifiedStreamEvent {
    Start {
        id: String,
        model: String,
    },
    TextDelta(String),
    ReasoningDelta(String),
    ReasoningSignature {
        signature: String,
    },
    ToolCallSignature {
        index: usize,
        signature: String,
    },
    ToolCall {
        index: usize,
        id: String,
        tool_type: String,
        name: String,
        arguments: String,
    },
    RawAnthropicContentBlock {
        block: Value,
    },
    StreamError {
        code: String,
        message: String,
    },
    Finish {
        reason: Option<String>,
        usage: Option<Value>,
        /// Service tier the upstream confirmed in its terminal stream event.
        /// Carried across the IR so the client-facing stream still reveals it
        /// after conversion — the gateway's usage collector reads the tier off
        /// the converted stream, and the IR `Response` has no tier field.
        service_tier: Option<String>,
    },
}

#[derive(Debug)]
pub struct StreamKernel {
    route: ConversionRoute,
    source: SourceStreamState,
    target: TargetStreamState,
    sse_buffer: SseBuffer,
    terminated_by_error: bool,
    done_received: bool,
    /// Last service tier the upstream confirmed in a source stream event
    /// (last-wins, mirroring `SseUsageCollector`). Captured here because the
    /// tier never passes through the IR, and the client-facing stream is what
    /// the collector observes after conversion.
    confirmed_service_tier: Option<String>,
}

impl StreamKernel {
    #[allow(dead_code)]
    pub fn new(route: ConversionRoute) -> Self {
        Self::with_context(route, ConversionContext::default())
    }

    pub fn with_context(route: ConversionRoute, context: ConversionContext) -> Self {
        Self {
            route,
            source: SourceStreamState::default(),
            target: TargetStreamState::with_conversion_context(context),
            sse_buffer: SseBuffer::new(),
            terminated_by_error: false,
            done_received: false,
            confirmed_service_tier: None,
        }
    }

    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        if self.terminated_by_error || self.done_received {
            return Vec::new();
        }
        self.sse_buffer.push_bytes(chunk);
        let mut out = Vec::new();
        while let Some(block) = self.sse_buffer.next_block().map(str::to_owned) {
            out.extend(self.convert_block(&block));
            if self.terminated_by_error || self.done_received {
                self.sse_buffer.clear();
                break;
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<Vec<u8>> {
        if self.done_received {
            self.sse_buffer.clear();
            return Vec::new();
        }
        let tail = self.sse_buffer.take_tail();
        if tail.trim().is_empty() {
            let mut out = self.finish_source();
            out.extend(self.target.finish(self.target_protocol()));
            return out;
        }
        let mut out = self.convert_block(&tail);
        if !self.done_received {
            out.extend(self.finish_source());
            out.extend(self.target.finish(self.target_protocol()));
        }
        out
    }

    pub fn fail(&mut self, message: &str) -> Vec<Vec<u8>> {
        self.terminated_by_error = true;
        self.target.write(
            self.target_protocol(),
            UnifiedStreamEvent::StreamError {
                code: "stream_error".to_string(),
                message: if message.is_empty() {
                    "stream error".to_string()
                } else {
                    message.to_string()
                },
            },
        )
    }

    fn convert_block(&mut self, block: &str) -> Vec<Vec<u8>> {
        if self.terminated_by_error || self.done_received {
            return Vec::new();
        }
        let parsed = parse_sse_block(block);
        let target = self.target_protocol();
        if parsed.data.trim().is_empty() {
            if parsed.event.as_deref() == Some("error") {
                self.terminated_by_error = true;
                return self.target.write(
                    target,
                    UnifiedStreamEvent::StreamError {
                        code: "stream_error".to_string(),
                        message: "stream error".to_string(),
                    },
                );
            }
            return Vec::new();
        }
        if parsed.data.trim() == "[DONE]" {
            self.done_received = true;
            let mut out = self.finish_source();
            out.extend(self.target.finish(target));
            return out;
        }
        let Ok(value) = serde_json::from_str::<Value>(&parsed.data) else {
            return Vec::new();
        };
        if let Some((code, message)) = stream_error_from_value(parsed.event.as_deref(), &value) {
            self.terminated_by_error = true;
            return self
                .target
                .write(target, UnifiedStreamEvent::StreamError { code, message });
        }
        // Last-wins, same policy as `SseUsageCollector`: the terminal snapshot
        // is the upstream's final word on the served tier. Captured before the
        // source parse consumes the value.
        if let Some(tier) = confirmed_service_tier_from_value(&value) {
            self.confirmed_service_tier = Some(tier.to_string());
        }
        let source = self.source_protocol();
        let events = self.source.parse(source, parsed.event.as_deref(), value);
        if events
            .iter()
            .any(|event| matches!(event, UnifiedStreamEvent::StreamError { .. }))
        {
            self.terminated_by_error = true;
        }
        let confirmed_service_tier = self.confirmed_service_tier.clone();
        events
            .into_iter()
            .map(|mut event| {
                if let UnifiedStreamEvent::Finish { service_tier, .. } = &mut event {
                    *service_tier = confirmed_service_tier.clone();
                }
                event
            })
            .flat_map(|event| self.target.write(target, event))
            .collect()
    }

    fn source_protocol(&self) -> AiProtocol {
        self.route.source
    }

    fn target_protocol(&self) -> AiProtocol {
        self.route.target
    }

    fn finish_source(&mut self) -> Vec<Vec<u8>> {
        if self.terminated_by_error {
            return Vec::new();
        }
        let target = self.target_protocol();
        let confirmed_service_tier = self.confirmed_service_tier.clone();
        self.source
            .finish(self.source_protocol())
            .into_iter()
            .map(|mut event| {
                if let UnifiedStreamEvent::Finish { service_tier, .. } = &mut event {
                    *service_tier = confirmed_service_tier.clone();
                }
                event
            })
            .flat_map(|event| self.target.write(target, event))
            .collect()
    }
}

#[derive(Debug, Default)]
struct SourceStreamState {
    chat_tool_names: HashMap<usize, String>,
    chat_tool_ids: HashMap<usize, String>,
    /// Accumulated arguments per tool index (for ordered flush / late identity).
    chat_tool_arguments: HashMap<usize, String>,
    chat_tool_types: HashMap<usize, String>,
    /// Tracks synthesized indices when upstream Chat SSE omits `tool_calls[].index`.
    chat_tool_next_index: usize,
    chat_tool_last_index: Option<usize>,
    chat_tool_index_by_id: HashMap<String, usize>,
    /// Next tool index that may be first-emitted (CS#5310 ordered flush).
    next_tool_index_to_add: usize,
    /// Tools whose id/name are ready but waiting for earlier indices.
    ready_tool_indices: BTreeSet<usize>,
    /// Tools that already had their identity-open ToolCall emitted.
    chat_tool_opened: HashSet<usize>,
    chat_seen_tool_call: bool,
    chat_inline_think_mode: InlineThinkMode,
    chat_inline_think_buffer: String,
    anthropic_tool_by_block: HashMap<usize, SourceToolState>,
    responses_tool_by_item: HashMap<String, SourceToolState>,
    responses_seen_output: bool,
    gemini_accumulated_text: String,
    gemini_accumulated_reasoning: String,
    gemini_seen_tool_call: bool,
    gemini_next_tool_index: usize,
    gemini_pending_tool_parts: Vec<Value>,
    pending_chat_finish_reason: Option<String>,
    chat_emitted_finish: bool,
    pending_anthropic_usage: Option<Value>,
}

#[derive(Debug, Clone, Default)]
struct SourceToolState {
    index: usize,
    id: String,
    tool_type: String,
    name: String,
    arguments: String,
    /// Bytes of `arguments` already emitted as ToolCall fragments.
    emitted_arguments_len: usize,
    /// True after arguments.done / output_item.done flushed this tool.
    arguments_done: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum InlineThinkMode {
    #[default]
    Detecting,
    Text,
    Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThinkPrefixDecision {
    NeedMore,
    Reasoning,
    Text,
}

fn is_anthropic_provider_local_stream_block(block_type: &str) -> bool {
    matches!(
        block_type,
        "server_tool_use"
            | "web_search_tool_use"
            | "web_search_tool_result"
            | "mcp_tool_use"
            | "mcp_tool_result"
    )
}

/// Return the still-unsent suffix of tool arguments.
///
/// Third-party Responses-compatible upstreams may send a done snapshot that does
/// not equal the concatenated deltas. A raw `arguments[emitted_len..]` slice can
/// then land mid multi-byte character and panic (fatal under `panic = "abort"`).
/// When the offset is not a char boundary, re-emit the full current snapshot.
fn safe_unsent_argument_suffix(arguments: &str, emitted_len: usize) -> String {
    if emitted_len == 0 {
        return arguments.to_string();
    }
    if emitted_len >= arguments.len() {
        return String::new();
    }
    if arguments.is_char_boundary(emitted_len) {
        return arguments[emitted_len..].to_string();
    }
    // Mid-char offset after a diverged done snapshot: re-send full arguments.
    arguments.to_string()
}

impl SourceStreamState {
    fn parse(
        &mut self,
        source: AiProtocol,
        event_name: Option<&str>,
        value: Value,
    ) -> Vec<UnifiedStreamEvent> {
        match source {
            AiProtocol::OpenAiChat => self.parse_chat(value),
            AiProtocol::OpenAiResponses => self.parse_responses(event_name, value),
            AiProtocol::AnthropicMessages => self.parse_anthropic(event_name, value),
            AiProtocol::GeminiNative => self.parse_gemini(value),
        }
    }

    fn parse_chat(&mut self, value: Value) -> Vec<UnifiedStreamEvent> {
        let mut out = Vec::new();
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("chatcmpl_gateway")
            .to_string();
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let usage = trusted_chat_stream_usage(&value, choices.is_empty());

        if choices.is_empty() {
            if let (Some(reason), Some(usage)) = (self.pending_chat_finish_reason.take(), usage) {
                out.push(UnifiedStreamEvent::Finish {
                    reason: Some(reason),
                    usage: Some(usage),
                    service_tier: None,
                });
                self.chat_emitted_finish = true;
            }
            return out;
        }

        for choice in choices {
            let delta = choice.get("delta").unwrap_or(&Value::Null);
            if delta.get("role").and_then(Value::as_str) == Some("assistant") {
                out.push(UnifiedStreamEvent::Start {
                    id: id.clone(),
                    model: model.clone(),
                });
            }
            match delta.get("content") {
                Some(Value::String(text)) if !text.is_empty() => {
                    out.extend(self.parse_chat_content_delta(text));
                }
                Some(Value::Array(parts)) => {
                    for part in parts {
                        if part.get("type").and_then(Value::as_str) == Some("thinking") {
                            let thinking = extract_mistral_thinking_part_text(part);
                            if !thinking.is_empty() {
                                out.push(UnifiedStreamEvent::ReasoningDelta(thinking));
                            }
                        } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                out.extend(self.parse_chat_content_delta(text));
                            }
                        }
                    }
                }
                _ => {}
            }
            if let Some(reasoning) = extract_reasoning_field_text(delta) {
                if !reasoning.is_empty() {
                    out.push(UnifiedStreamEvent::ReasoningDelta(reasoning));
                }
            }
            if let Some(signature) = delta
                .get("reasoning_signature")
                .and_then(Value::as_str)
                .filter(|signature| !signature.is_empty())
            {
                out.push(UnifiedStreamEvent::ReasoningSignature {
                    signature: signature.to_string(),
                });
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                out.extend(self.flush_chat_inline_think_at_boundary());
                for tool_call in tool_calls {
                    let index = self.resolve_chat_tool_call_index(tool_call);
                    if tool_call.get("type").and_then(Value::as_str)
                        == Some(TOOL_TYPE_RESPONSES_CUSTOM_TOOL)
                    {
                        let custom = tool_call
                            .get("response_custom_tool_call")
                            .unwrap_or(&Value::Null);
                        if let Some(id) = custom
                            .get("call_id")
                            .or_else(|| tool_call.get("id"))
                            .and_then(Value::as_str)
                        {
                            self.chat_tool_ids.insert(index, id.to_string());
                        }
                        if let Some(name) = custom.get("name").and_then(Value::as_str) {
                            self.chat_tool_names.insert(index, name.to_string());
                        }
                        let input = custom
                            .get("input")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !input.is_empty() {
                            self.chat_tool_arguments
                                .entry(index)
                                .or_default()
                                .push_str(input);
                        }
                        self.chat_tool_types
                            .insert(index, TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string());
                        self.mark_chat_tool_ready(index);
                        out.extend(self.flush_ready_tool_calls());
                        self.chat_seen_tool_call = true;
                        continue;
                    }
                    let function = tool_call.get("function").unwrap_or(tool_call);
                    if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                        self.chat_tool_ids.insert(index, id.to_string());
                    }
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        self.chat_tool_names.insert(index, name.to_string());
                    }
                    let arguments = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !arguments.is_empty() {
                        self.chat_tool_arguments
                            .entry(index)
                            .or_default()
                            .push_str(arguments);
                    }
                    self.chat_tool_types
                        .insert(index, TOOL_TYPE_FUNCTION.to_string());
                    self.mark_chat_tool_ready(index);
                    out.extend(self.flush_ready_tool_calls());
                    self.chat_seen_tool_call = true;
                }
            }
            if let Some(function_call) = delta.get("function_call") {
                out.extend(self.flush_chat_inline_think_at_boundary());
                let index = 0;
                if let Some(id) = function_call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    self.chat_tool_ids.insert(index, id.to_string());
                }
                if let Some(name) = function_call
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                {
                    self.chat_tool_names.insert(index, name.to_string());
                }
                let arguments = function_call
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !arguments.is_empty() {
                    self.chat_tool_arguments
                        .entry(index)
                        .or_default()
                        .push_str(arguments);
                }
                self.chat_tool_types
                    .insert(index, TOOL_TYPE_FUNCTION.to_string());
                self.mark_chat_tool_ready(index);
                out.extend(self.flush_ready_tool_calls());
                if !self
                    .chat_tool_names
                    .get(&index)
                    .map(String::is_empty)
                    .unwrap_or(true)
                    || !arguments.is_empty()
                {
                    self.chat_seen_tool_call = true;
                }
            }
            if let Some(finish_reason) = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.trim().is_empty())
                .map(|reason| match reason {
                    "function_call" => "tool_calls",
                    "stop" if self.chat_seen_tool_call => "tool_calls",
                    "sensitive" => "content_filter",
                    other => other,
                })
                .map(ToString::to_string)
            {
                out.extend(self.flush_chat_inline_think_at_boundary());
                // Flush any remaining ready tools in order before finish.
                out.extend(self.flush_ready_tool_calls_force());
                if let Some(usage) = usage.clone() {
                    self.pending_chat_finish_reason = None;
                    out.push(UnifiedStreamEvent::Finish {
                        reason: Some(finish_reason),
                        usage: Some(usage),
                        service_tier: None,
                    });
                } else {
                    self.pending_chat_finish_reason = Some(finish_reason.clone());
                    out.push(UnifiedStreamEvent::Finish {
                        reason: Some(finish_reason),
                        usage: None,
                        service_tier: None,
                    });
                }
                self.chat_emitted_finish = true;
            }
        }
        out
    }

    fn finish(&mut self, source: AiProtocol) -> Vec<UnifiedStreamEvent> {
        match source {
            AiProtocol::OpenAiChat => self.finish_chat(),
            AiProtocol::OpenAiResponses => self.finish_responses(),
            AiProtocol::GeminiNative => self.flush_gemini_tool_calls(),
            AiProtocol::AnthropicMessages => Vec::new(),
        }
    }

    fn finish_responses(&mut self) -> Vec<UnifiedStreamEvent> {
        // Only flush tools that never got arguments.done / output_item.done.
        // Dense streams already emitted deltas (+ optional done suffix); re-emitting
        // full arguments here would concatenate duplicates into Chat/Anthropic.
        let mut keys = self
            .responses_tool_by_item
            .iter()
            .filter(|(_, state)| !state.arguments_done)
            .map(|(key, state)| (key.clone(), state.index))
            .collect::<Vec<_>>();
        keys.sort_by_key(|(_, index)| *index);
        let mut out = Vec::new();
        for (key, _) in keys {
            out.extend(self.emit_responses_tool_arguments_progress(&key, true));
        }
        self.responses_tool_by_item
            .retain(|_, state| !state.arguments_done);
        out
    }

    fn emit_responses_tool_arguments_progress(
        &mut self,
        key: &str,
        mark_done: bool,
    ) -> Vec<UnifiedStreamEvent> {
        let Some(state) = self.responses_tool_by_item.get_mut(key) else {
            return Vec::new();
        };
        if state.arguments_done && !mark_done {
            return Vec::new();
        }
        // Third-party Responses-compatible upstreams may send a done snapshot that does
        // not equal the concatenated deltas. Never slice on a raw byte offset into that
        // snapshot — mid-char panics abort the whole process under panic=abort.
        let suffix = safe_unsent_argument_suffix(&state.arguments, state.emitted_arguments_len);
        state.emitted_arguments_len = state.arguments.len();
        if mark_done {
            state.arguments_done = true;
        }
        let event = if suffix.is_empty() {
            None
        } else {
            Some(UnifiedStreamEvent::ToolCall {
                index: state.index,
                id: state.id.clone(),
                tool_type: state.tool_type.clone(),
                name: state.name.clone(),
                arguments: suffix,
            })
        };
        if mark_done {
            self.responses_tool_by_item.remove(key);
        }
        event.into_iter().collect()
    }

    fn finish_chat(&mut self) -> Vec<UnifiedStreamEvent> {
        let mut out = self.flush_chat_inline_think_at_boundary();
        out.extend(self.flush_ready_tool_calls_force());
        if !self.chat_emitted_finish {
            self.chat_emitted_finish = true;
            out.push(UnifiedStreamEvent::Finish {
                reason: Some(if self.chat_seen_tool_call {
                    "tool_calls".to_string()
                } else {
                    "stop".to_string()
                }),
                usage: None,
                service_tier: None,
            });
        }
        out
    }

    fn resolve_chat_tool_call_index(&mut self, tool_call: &Value) -> usize {
        let id = tool_call
            .pointer("/response_custom_tool_call/call_id")
            .or_else(|| tool_call.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        if let Some(explicit) = tool_call.get("index").and_then(Value::as_u64) {
            let index = explicit as usize;
            self.chat_tool_next_index = self.chat_tool_next_index.max(index.saturating_add(1));
            self.chat_tool_last_index = Some(index);
            if let Some(id) = id {
                self.chat_tool_index_by_id.insert(id.to_string(), index);
            }
            return index;
        }
        if let Some(id) = id {
            if let Some(&existing) = self.chat_tool_index_by_id.get(id) {
                self.chat_tool_last_index = Some(existing);
                return existing;
            }
            let index = self.chat_tool_next_index;
            self.chat_tool_next_index = self.chat_tool_next_index.saturating_add(1);
            self.chat_tool_index_by_id.insert(id.to_string(), index);
            self.chat_tool_last_index = Some(index);
            return index;
        }
        let has_name = tool_call
            .pointer("/response_custom_tool_call/name")
            .or_else(|| tool_call.pointer("/function/name"))
            .or_else(|| tool_call.get("name"))
            .and_then(Value::as_str)
            .is_some_and(|name| !name.is_empty());
        if let Some(last) = self.chat_tool_last_index {
            let last_has_name = self
                .chat_tool_names
                .get(&last)
                .is_some_and(|name| !name.is_empty());
            if !has_name || !last_has_name {
                return last;
            }
        }
        let index = self.chat_tool_next_index;
        self.chat_tool_next_index = self.chat_tool_next_index.saturating_add(1);
        self.chat_tool_last_index = Some(index);
        index
    }

    fn mark_chat_tool_ready(&mut self, index: usize) {
        // Already opened: argument-only deltas emit immediately via flush path.
        if self.chat_tool_opened.contains(&index) {
            self.ready_tool_indices.insert(index);
            return;
        }
        // First emit must wait for both name and real id. Synthetic ids are
        // reserved for finish-time fallback when a legacy stream never sends id.
        let has_name = self
            .chat_tool_names
            .get(&index)
            .is_some_and(|name| !name.is_empty());
        let has_id = self
            .chat_tool_ids
            .get(&index)
            .is_some_and(|id| !id.is_empty());
        if has_name && has_id {
            self.ready_tool_indices.insert(index);
        }
    }

    /// Emit tools with complete identity in ascending index order (CS#5310).
    /// After a tool is opened, further argument deltas for that index emit immediately.
    fn flush_ready_tool_calls(&mut self) -> Vec<UnifiedStreamEvent> {
        let mut out = Vec::new();
        loop {
            // Drain already-opened tools that have pending argument deltas (any order OK after open).
            let opened_pending: Vec<usize> = self
                .ready_tool_indices
                .iter()
                .copied()
                .filter(|index| self.chat_tool_opened.contains(index))
                .collect();
            for index in opened_pending {
                self.ready_tool_indices.remove(&index);
                if self
                    .chat_tool_arguments
                    .get(&index)
                    .is_some_and(|args| !args.is_empty())
                {
                    out.push(self.emit_chat_tool_call(index));
                }
            }

            if !self
                .ready_tool_indices
                .contains(&self.next_tool_index_to_add)
            {
                break;
            }
            let index = self.next_tool_index_to_add;
            self.ready_tool_indices.remove(&index);
            out.push(self.emit_chat_tool_call(index));
            self.chat_tool_opened.insert(index);
            self.next_tool_index_to_add = self.next_tool_index_to_add.saturating_add(1);
        }
        out
    }

    /// On finish: emit remaining ready tools sorted by index even if gaps remain.
    fn flush_ready_tool_calls_force(&mut self) -> Vec<UnifiedStreamEvent> {
        let mut out = self.flush_ready_tool_calls();
        for index in self.force_ready_chat_tool_indices() {
            self.ready_tool_indices.insert(index);
        }
        let remaining: Vec<usize> = self.ready_tool_indices.iter().copied().collect();
        for index in remaining {
            self.ready_tool_indices.remove(&index);
            out.push(self.emit_chat_tool_call(index));
            self.chat_tool_opened.insert(index);
            self.next_tool_index_to_add = self.next_tool_index_to_add.max(index.saturating_add(1));
        }
        out
    }

    fn force_ready_chat_tool_indices(&self) -> Vec<usize> {
        let mut indices = self
            .chat_tool_names
            .iter()
            .filter_map(|(index, name)| {
                (!name.is_empty() && !self.chat_tool_opened.contains(index)).then_some(*index)
            })
            .collect::<Vec<_>>();
        indices.sort_unstable();
        indices
    }

    fn emit_chat_tool_call(&mut self, index: usize) -> UnifiedStreamEvent {
        let arguments = self.chat_tool_arguments.remove(&index).unwrap_or_default();
        UnifiedStreamEvent::ToolCall {
            index,
            id: self
                .chat_tool_ids
                .get(&index)
                .filter(|id| !id.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("call_{index}")),
            tool_type: self
                .chat_tool_types
                .get(&index)
                .cloned()
                .unwrap_or_else(|| TOOL_TYPE_FUNCTION.to_string()),
            name: self
                .chat_tool_names
                .get(&index)
                .cloned()
                .unwrap_or_default(),
            arguments,
        }
    }

    fn parse_chat_content_delta(&mut self, delta: &str) -> Vec<UnifiedStreamEvent> {
        match self.chat_inline_think_mode {
            InlineThinkMode::Text => vec![UnifiedStreamEvent::TextDelta(delta.to_string())],
            InlineThinkMode::Detecting => {
                self.chat_inline_think_buffer.push_str(delta);
                match leading_think_prefix_decision(&self.chat_inline_think_buffer) {
                    ThinkPrefixDecision::NeedMore => Vec::new(),
                    ThinkPrefixDecision::Reasoning => {
                        self.chat_inline_think_mode = InlineThinkMode::Reasoning;
                        self.drain_complete_chat_inline_think()
                    }
                    ThinkPrefixDecision::Text => {
                        self.chat_inline_think_mode = InlineThinkMode::Text;
                        let text = std::mem::take(&mut self.chat_inline_think_buffer);
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![UnifiedStreamEvent::TextDelta(text)]
                        }
                    }
                }
            }
            InlineThinkMode::Reasoning => {
                self.chat_inline_think_buffer.push_str(delta);
                self.drain_complete_chat_inline_think()
            }
        }
    }

    fn drain_complete_chat_inline_think(&mut self) -> Vec<UnifiedStreamEvent> {
        let Some((reasoning, answer)) = split_leading_think_block(&self.chat_inline_think_buffer)
        else {
            return Vec::new();
        };
        self.chat_inline_think_mode = InlineThinkMode::Text;
        self.chat_inline_think_buffer.clear();

        let mut out = Vec::new();
        if !reasoning.is_empty() {
            out.push(UnifiedStreamEvent::ReasoningDelta(reasoning));
        }
        if !answer.is_empty() {
            out.push(UnifiedStreamEvent::TextDelta(answer));
        }
        out
    }

    fn flush_chat_inline_think_at_boundary(&mut self) -> Vec<UnifiedStreamEvent> {
        match self.chat_inline_think_mode {
            InlineThinkMode::Text => Vec::new(),
            InlineThinkMode::Detecting => {
                self.chat_inline_think_mode = InlineThinkMode::Text;
                let text = std::mem::take(&mut self.chat_inline_think_buffer);
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![UnifiedStreamEvent::TextDelta(text)]
                }
            }
            InlineThinkMode::Reasoning => {
                let buffered = std::mem::take(&mut self.chat_inline_think_buffer);
                self.chat_inline_think_mode = InlineThinkMode::Text;
                if let Some((reasoning, answer)) = split_leading_think_block(&buffered) {
                    let mut out = Vec::new();
                    if !reasoning.is_empty() {
                        out.push(UnifiedStreamEvent::ReasoningDelta(reasoning));
                    }
                    if !answer.is_empty() {
                        out.push(UnifiedStreamEvent::TextDelta(answer));
                    }
                    return out;
                }
                let reasoning = strip_leading_think_open_tag(&buffered)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or(buffered.trim());
                if reasoning.is_empty() {
                    Vec::new()
                } else {
                    vec![UnifiedStreamEvent::ReasoningDelta(reasoning.to_string())]
                }
            }
        }
    }

    fn parse_responses(
        &mut self,
        event_name: Option<&str>,
        value: Value,
    ) -> Vec<UnifiedStreamEvent> {
        let event_type = event_name
            .filter(|name| !name.is_empty())
            .or_else(|| value.get("type").and_then(Value::as_str))
            .unwrap_or_default();
        match event_type {
            "response.created" => {
                let response = value.get("response").unwrap_or(&value);
                vec![UnifiedStreamEvent::Start {
                    id: response
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("resp_gateway")
                        .to_string(),
                    model: response
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                }]
            }
            "response.output_text.delta" => value
                .get("delta")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(|text| {
                    self.responses_seen_output = true;
                    vec![UnifiedStreamEvent::TextDelta(text.to_string())]
                })
                .unwrap_or_default(),
            "response.reasoning_summary_text.delta" => value
                .get("delta")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(|text| {
                    self.responses_seen_output = true;
                    vec![UnifiedStreamEvent::ReasoningDelta(text.to_string())]
                })
                .unwrap_or_default(),
            "response.output_item.added" => {
                let item = value.get("item").unwrap_or(&value);
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if item_type == "reasoning" {
                    return item
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .filter(|signature| !signature.is_empty())
                        .map(|signature| {
                            self.responses_seen_output = true;
                            vec![UnifiedStreamEvent::ReasoningSignature {
                                signature: encode_signature(
                                    SignatureProvider::OpenAiResponses,
                                    signature,
                                ),
                            }]
                        })
                        .unwrap_or_default();
                }
                if item_type != "function_call" && item_type != "custom_tool_call" {
                    return Vec::new();
                }
                self.responses_seen_output = true;
                let key = item
                    .get("id")
                    .or_else(|| value.get("item_id"))
                    .or_else(|| item.get("call_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("call_0")
                    .to_string();
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let state = SourceToolState {
                    index,
                    id: item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or(&key)
                        .to_string(),
                    tool_type: if item_type == "custom_tool_call" {
                        TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string()
                    } else {
                        TOOL_TYPE_FUNCTION.to_string()
                    },
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: String::new(),
                    emitted_arguments_len: 0,
                    arguments_done: false,
                };
                // Emit tool header (id+name) on added for both function and custom tools so
                // sparse streams without deltas still surface the call to target writers.
                let event = UnifiedStreamEvent::ToolCall {
                    index: state.index,
                    id: state.id.clone(),
                    tool_type: state.tool_type.clone(),
                    name: state.name.clone(),
                    arguments: String::new(),
                };
                self.responses_tool_by_item.insert(key, state);
                vec![event]
            }
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
                let key = value
                    .get("item_id")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("call_id").and_then(Value::as_str))
                    .unwrap_or("call_0")
                    .to_string();
                let delta = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                {
                    let state = self
                        .responses_tool_by_item
                        .entry(key.clone())
                        .or_insert_with(|| SourceToolState {
                            index: value
                                .get("output_index")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as usize,
                            id: key.clone(),
                            tool_type: if event_name.unwrap_or_default()
                                == "response.custom_tool_call_input.delta"
                            {
                                TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string()
                            } else {
                                TOOL_TYPE_FUNCTION.to_string()
                            },
                            ..Default::default()
                        });
                    if state.arguments_done {
                        return Vec::new();
                    }
                    if !delta.is_empty() {
                        self.responses_seen_output = true;
                    }
                    state.arguments.push_str(&delta);
                }
                self.emit_responses_tool_arguments_progress(&key, false)
            }
            "response.function_call_arguments.done" | "response.custom_tool_call_input.done" => {
                let key = value
                    .get("item_id")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("call_id").and_then(Value::as_str))
                    .unwrap_or("call_0")
                    .to_string();
                if let Some(state) = self.responses_tool_by_item.get_mut(&key) {
                    if let Some(arguments) = value
                        .get("arguments")
                        .or_else(|| value.get("input"))
                        .and_then(Value::as_str)
                        .filter(|arguments| !arguments.is_empty())
                    {
                        // Prefer the complete snapshot when non-empty; never let an empty
                        // done snapshot erase previously streamed deltas.
                        state.arguments = arguments.to_string();
                    }
                }
                self.emit_responses_tool_arguments_progress(&key, true)
            }
            "response.output_item.done" => {
                let item = value.get("item").unwrap_or(&value);
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if item_type == "reasoning" {
                    return item
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .filter(|signature| !signature.is_empty())
                        .map(|signature| {
                            self.responses_seen_output = true;
                            vec![UnifiedStreamEvent::ReasoningSignature {
                                signature: encode_signature(
                                    SignatureProvider::OpenAiResponses,
                                    signature,
                                ),
                            }]
                        })
                        .unwrap_or_default();
                }
                if item_type == "function_call" || item_type == "custom_tool_call" {
                    let key = item
                        .get("id")
                        .or_else(|| value.get("item_id"))
                        .or_else(|| item.get("call_id"))
                        .and_then(Value::as_str)
                        .unwrap_or("call_0")
                        .to_string();
                    if let Some(state) = self.responses_tool_by_item.get_mut(&key) {
                        if let Some(arguments) = item
                            .get("arguments")
                            .or_else(|| item.get("input"))
                            .and_then(Value::as_str)
                        {
                            if !arguments.is_empty() {
                                state.arguments = arguments.to_string();
                            }
                        }
                        if let Some(name) = item.get("name").and_then(Value::as_str) {
                            if !name.is_empty() {
                                state.name = name.to_string();
                            }
                        }
                    }
                    return self.emit_responses_tool_arguments_progress(&key, true);
                }
                Vec::new()
            }
            "response.failed" => {
                let response = value.get("response").unwrap_or(&value);
                let error = response
                    .get("error")
                    .filter(|error| !error.is_null())
                    .unwrap_or(response);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|message| !message.trim().is_empty())
                    .map(ToString::to_string)
                    .or_else(|| error.as_str().map(ToString::to_string))
                    .unwrap_or_else(|| "Response failed".to_string());
                let code = stream_error_code_from_value(error.get("code"))
                    .or_else(|| stream_error_code_from_value(error.get("type")))
                    .unwrap_or_else(|| "response_error".to_string());
                vec![UnifiedStreamEvent::StreamError { code, message }]
            }
            "response.completed"
            | "response.cancelled"
            | "response.canceled"
            | "response.incomplete" => {
                let response = value.get("response").unwrap_or(&value);
                let output_items = response.get("output").and_then(Value::as_array);
                let has_tool_call = !self.responses_tool_by_item.is_empty()
                    || output_items
                        .map(|items| {
                            items.iter().any(|item| {
                                matches!(
                                    item.get("type").and_then(Value::as_str),
                                    Some("function_call") | Some("custom_tool_call")
                                )
                            })
                        })
                        .unwrap_or(false);
                let incomplete_reason = response
                    .pointer("/incomplete_details/reason")
                    .and_then(Value::as_str);
                let is_incomplete = event_type == "response.incomplete"
                    || response.get("status").and_then(Value::as_str) == Some("incomplete");
                if is_incomplete && incomplete_reason != Some("content_filter") {
                    let has_zero_usage = response
                        .get("usage")
                        .is_some_and(is_zero_chat_usage_placeholder);
                    let output_is_empty = output_items.is_none_or(|items| items.is_empty());
                    if has_zero_usage
                        && !self.responses_seen_output
                        && !has_tool_call
                        && output_is_empty
                    {
                        return vec![UnifiedStreamEvent::StreamError {
                            code: "incomplete_empty_response".to_string(),
                            message: "Upstream returned an incomplete empty response with zero token usage"
                                .to_string(),
                        }];
                    }
                }
                let reason = match event_type {
                    "response.cancelled" | "response.canceled" => Some("cancelled".to_string()),
                    "response.incomplete" => Some(if incomplete_reason == Some("content_filter") {
                        "content_filter".to_string()
                    } else {
                        "length".to_string()
                    }),
                    _ => response
                        .get("status")
                        .and_then(Value::as_str)
                        .map(|status| match status {
                            "failed" => "error",
                            "canceled" | "cancelled" => "cancelled",
                            "incomplete" => {
                                if incomplete_reason == Some("content_filter") {
                                    "content_filter"
                                } else {
                                    "length"
                                }
                            }
                            _ if has_tool_call => "tool_calls",
                            _ => "stop",
                        })
                        .map(ToString::to_string),
                };
                // completed + status=failed is still a protocol failure terminal.
                if reason.as_deref() == Some("error") {
                    let error = response
                        .get("error")
                        .filter(|error| !error.is_null())
                        .unwrap_or(response);
                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|message| !message.trim().is_empty())
                        .map(ToString::to_string)
                        .or_else(|| error.as_str().map(ToString::to_string))
                        .unwrap_or_else(|| "Response failed".to_string());
                    let code = stream_error_code_from_value(error.get("code"))
                        .or_else(|| stream_error_code_from_value(error.get("type")))
                        .unwrap_or_else(|| "response_error".to_string());
                    return vec![UnifiedStreamEvent::StreamError { code, message }];
                }
                vec![UnifiedStreamEvent::Finish {
                    reason,
                    usage: response.get("usage").cloned(),
                    service_tier: None,
                }]
            }
            _ => Vec::new(),
        }
    }

    fn parse_anthropic(
        &mut self,
        event_name: Option<&str>,
        value: Value,
    ) -> Vec<UnifiedStreamEvent> {
        match event_name
            .or_else(|| value.get("type").and_then(Value::as_str))
            .unwrap_or_default()
        {
            "message_start" => {
                let message = value.get("message").unwrap_or(&value);
                if let Some(usage) = message.get("usage").cloned() {
                    self.pending_anthropic_usage = Some(usage);
                }
                vec![UnifiedStreamEvent::Start {
                    id: message
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("msg_gateway")
                        .to_string(),
                    model: message
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                }]
            }
            "content_block_start" => {
                let block = value.get("content_block").unwrap_or(&Value::Null);
                if block
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(is_anthropic_provider_local_stream_block)
                {
                    return vec![UnifiedStreamEvent::RawAnthropicContentBlock {
                        block: block.clone(),
                    }];
                }
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    self.anthropic_tool_by_block.insert(
                        index,
                        SourceToolState {
                            index,
                            id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            tool_type: TOOL_TYPE_FUNCTION.to_string(),
                            name: block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            arguments: String::new(),
                            emitted_arguments_len: 0,
                            arguments_done: false,
                        },
                    );
                }
                Vec::new()
            }
            "content_block_delta" => {
                if let Some(text) = value.pointer("/delta/text").and_then(Value::as_str) {
                    return vec![UnifiedStreamEvent::TextDelta(text.to_string())];
                }
                if let Some(thinking) = value.pointer("/delta/thinking").and_then(Value::as_str) {
                    return vec![UnifiedStreamEvent::ReasoningDelta(thinking.to_string())];
                }
                if let Some(signature) = value.pointer("/delta/signature").and_then(Value::as_str) {
                    return vec![UnifiedStreamEvent::ReasoningSignature {
                        signature: encode_signature(SignatureProvider::Anthropic, signature),
                    }];
                }
                if let Some(partial_json) =
                    value.pointer("/delta/partial_json").and_then(Value::as_str)
                {
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    if let Some(state) = self.anthropic_tool_by_block.get_mut(&index) {
                        state.arguments.push_str(partial_json);
                        return vec![UnifiedStreamEvent::ToolCall {
                            index: state.index,
                            id: state.id.clone(),
                            tool_type: state.tool_type.clone(),
                            name: state.name.clone(),
                            arguments: partial_json.to_string(),
                        }];
                    }
                }
                Vec::new()
            }
            "message_delta" => {
                if let Some(usage) = value.get("usage") {
                    self.pending_anthropic_usage = Some(merge_anthropic_stream_usage(
                        self.pending_anthropic_usage.as_ref(),
                        usage,
                    ));
                }
                value
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                    .filter(|reason| !reason.trim().is_empty())
                    .map(|reason| {
                        let mapped_reason = match reason {
                            "max_tokens" | "model_context_window_exceeded" => "length",
                            "tool_use" => "tool_calls",
                            "refusal" => "refusal",
                            "pause_turn" => "pause_turn",
                            "stop_sequence" => "stop_sequence",
                            _ => "stop",
                        };
                        vec![UnifiedStreamEvent::Finish {
                            reason: Some(mapped_reason.to_string()),
                            usage: self
                                .pending_anthropic_usage
                                .take()
                                .map(|usage| anthropic_usage_value_to_unified(&usage)),
                            service_tier: None,
                        }]
                    })
                    .unwrap_or_default()
            }
            "message_stop" => self
                .pending_anthropic_usage
                .take()
                .map(|usage| {
                    vec![UnifiedStreamEvent::Finish {
                        reason: None,
                        usage: Some(anthropic_usage_value_to_unified(&usage)),
                        service_tier: None,
                    }]
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn parse_gemini(&mut self, value: Value) -> Vec<UnifiedStreamEvent> {
        if value
            .get("responseId")
            .and_then(Value::as_str)
            .is_some_and(|response_id| response_id.trim().is_empty())
        {
            return vec![UnifiedStreamEvent::StreamError {
                code: "invalid_response".to_string(),
                message: "Gemini stream responseId is empty".to_string(),
            }];
        }

        let mut out = Vec::new();
        out.push(UnifiedStreamEvent::Start {
            id: value
                .get("responseId")
                .and_then(Value::as_str)
                .unwrap_or("gemini_gateway")
                .to_string(),
            model: value
                .get("modelVersion")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });

        // Safety-only blocked streams: promptFeedback.blockReason without candidates.
        if let Some(block_reason) = value
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.trim().is_empty())
        {
            let message = value
                .pointer("/promptFeedback/blockReasonMessage")
                .and_then(Value::as_str)
                .filter(|message| !message.trim().is_empty())
                .unwrap_or("Response blocked by Gemini safety filters");
            out.push(UnifiedStreamEvent::TextDelta(format!(
                "[blocked:{block_reason}] {message}"
            )));
            out.push(UnifiedStreamEvent::Finish {
                reason: Some("refusal".to_string()),
                usage: gemini_stream_usage(value.get("usageMetadata")),
                service_tier: None,
            });
            return out;
        }

        if let Some(candidate) = value
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
        {
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
            {
                let visible_text = parts
                    .iter()
                    .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>();
                if !visible_text.is_empty() {
                    let delta = if visible_text.len() > self.gemini_accumulated_text.len()
                        && visible_text.starts_with(&self.gemini_accumulated_text)
                    {
                        visible_text[self.gemini_accumulated_text.len()..].to_string()
                    } else {
                        visible_text.clone()
                    };
                    if !delta.is_empty() {
                        out.push(UnifiedStreamEvent::TextDelta(delta));
                    }
                    self.gemini_accumulated_text = visible_text;
                }
                if let Some(signature) = parts
                    .iter()
                    .filter(|part| part.get("thought").and_then(Value::as_bool) == Some(true))
                    .find_map(gemini_part_thought_signature)
                {
                    out.push(UnifiedStreamEvent::ReasoningSignature {
                        signature: encode_signature(SignatureProvider::Gemini, signature),
                    });
                }
                let reasoning_text = parts
                    .iter()
                    .filter(|part| part.get("thought").and_then(Value::as_bool) == Some(true))
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>();
                if !reasoning_text.is_empty() {
                    let delta = if reasoning_text.len() > self.gemini_accumulated_reasoning.len()
                        && reasoning_text.starts_with(&self.gemini_accumulated_reasoning)
                    {
                        reasoning_text[self.gemini_accumulated_reasoning.len()..].to_string()
                    } else {
                        reasoning_text.clone()
                    };
                    if !delta.is_empty() {
                        out.push(UnifiedStreamEvent::ReasoningDelta(delta));
                    }
                    self.gemini_accumulated_reasoning = reasoning_text;
                }
                for part in parts {
                    if part.get("functionCall").is_none() {
                        continue;
                    }
                    self.gemini_seen_tool_call = true;
                    // Keep only tool snapshots until finish; text/reasoning
                    // continue streaming. Re-emitting a closed JSON args object
                    // would concatenate two objects in Chat/Responses clients.
                    merge_gemini_function_call_part(&mut self.gemini_pending_tool_parts, part);
                }
            }
            if let Some(finish_reason) = candidate
                .get("finishReason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.trim().is_empty())
            {
                out.extend(self.flush_gemini_tool_calls());
                out.push(UnifiedStreamEvent::Finish {
                    reason: gemini_finish_to_openai_finish(
                        Some(finish_reason),
                        self.gemini_seen_tool_call,
                    ),
                    usage: gemini_stream_usage(value.get("usageMetadata")),
                    service_tier: None,
                });
            }
        }
        if (out.is_empty() || matches!(out.as_slice(), [UnifiedStreamEvent::Start { .. }]))
            && value.get("usageMetadata").is_some()
            && value
                .get("candidates")
                .and_then(Value::as_array)
                .is_none_or(|candidates| candidates.is_empty())
        {
            out.extend(self.flush_gemini_tool_calls());
            out.push(UnifiedStreamEvent::Finish {
                reason: Some(
                    if self.gemini_seen_tool_call {
                        "tool_calls"
                    } else {
                        "stop"
                    }
                    .to_string(),
                ),
                usage: gemini_stream_usage(value.get("usageMetadata")),
                service_tier: None,
            });
        }
        out
    }

    fn flush_gemini_tool_calls(&mut self) -> Vec<UnifiedStreamEvent> {
        let mut events = Vec::new();
        for part in self.gemini_pending_tool_parts.drain(..) {
            let function_call = &part["functionCall"];
            let index = self.gemini_next_tool_index;
            self.gemini_next_tool_index += 1;
            if let Some(signature) = gemini_part_thought_signature(&part) {
                events.push(UnifiedStreamEvent::ToolCallSignature {
                    index,
                    signature: encode_signature(SignatureProvider::Gemini, signature),
                });
            }
            events.push(UnifiedStreamEvent::ToolCall {
                index,
                id: function_call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(synthesize_gemini_tool_id),
                tool_type: TOOL_TYPE_FUNCTION.to_string(),
                name: function_call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                arguments: function_call.get("args").unwrap_or(&json!({})).to_string(),
            });
        }
        events
    }
}

fn gemini_stream_usage(usage_metadata: Option<&Value>) -> Option<Value> {
    usage_metadata.map(|usage| llm_usage_to_unified_value(&gemini_usage_to_llm(Some(usage))))
}

fn llm_usage_to_unified_value(usage: &Usage) -> Value {
    let mut value = json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": if usage.total_tokens == 0 {
            usage.prompt_tokens.saturating_add(usage.completion_tokens)
        } else {
            usage.total_tokens
        },
    });
    if usage.cached_tokens > 0 || usage.cache_write_tokens > 0 {
        let mut details = json!({});
        if usage.cached_tokens > 0 {
            details["cached_tokens"] = json!(usage.cached_tokens);
        }
        if usage.cache_write_tokens > 0 {
            details["cache_write_tokens"] = json!(usage.cache_write_tokens);
        }
        value["prompt_tokens_details"] = details;
    }
    if usage.reasoning_tokens > 0 {
        value["completion_tokens_details"] = json!({
            "reasoning_tokens": usage.reasoning_tokens
        });
    }
    value
}

fn merge_anthropic_stream_usage(start: Option<&Value>, delta: &Value) -> Value {
    let mut merged = start.cloned().unwrap_or_else(|| json!({}));
    let Some(merged_object) = merged.as_object_mut() else {
        return delta.clone();
    };
    let Some(delta_object) = delta.as_object() else {
        return merged;
    };
    for (key, value) in delta_object {
        // Prefer message_start for input/cache fields; take latest non-null for output.
        let is_input_like = matches!(
            key.as_str(),
            "input_tokens"
                | "cache_creation_input_tokens"
                | "cache_read_input_tokens"
                | "cache_creation"
        );
        if is_input_like {
            if !merged_object.contains_key(key)
                || merged_object.get(key).is_some_and(Value::is_null)
            {
                merged_object.insert(key.clone(), value.clone());
            }
            continue;
        }
        if !value.is_null() {
            merged_object.insert(key.clone(), value.clone());
        }
    }
    merged
}

fn anthropic_usage_value_to_unified(usage: &Value) -> Value {
    let input = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    llm_usage_to_unified_value(&Usage {
        prompt_tokens: input.saturating_add(cached).saturating_add(cache_creation),
        completion_tokens: output,
        total_tokens: input
            .saturating_add(cached)
            .saturating_add(cache_creation)
            .saturating_add(output),
        cached_tokens: cached,
        cache_write_tokens: cache_creation,
        reasoning_tokens: 0,
    })
}

fn unified_value_to_usage(usage: &Value) -> Usage {
    let prompt_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens));
    let cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write_tokens = usage
        .pointer("/prompt_tokens_details/cache_write_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cache_write_tokens"))
        .or_else(|| usage.get("cache_write_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning_tokens = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .or_else(|| usage.pointer("/output_tokens_details/reasoning_tokens"))
        .or_else(|| usage.get("reasoning_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens,
        cache_write_tokens,
        reasoning_tokens,
    }
}

fn unified_usage_to_gemini_metadata(usage: Option<Value>) -> Option<Value> {
    usage.map(|value| llm_usage_to_gemini(Some(&unified_value_to_usage(&value))))
}

fn leading_think_prefix_decision(buffer: &str) -> ThinkPrefixDecision {
    let trimmed = buffer.trim_start();
    if trimmed.is_empty() {
        return ThinkPrefixDecision::NeedMore;
    }

    let normalized = trimmed.to_ascii_lowercase();
    if normalized.starts_with("<think>") || normalized.starts_with("<thinking>") {
        return ThinkPrefixDecision::Reasoning;
    }
    if "<think>".starts_with(&normalized) || "<thinking>".starts_with(&normalized) {
        return ThinkPrefixDecision::NeedMore;
    }
    ThinkPrefixDecision::Text
}

fn extract_mistral_thinking_part_text(part: &Value) -> String {
    if let Some(thinking) = part.get("thinking") {
        match thinking {
            Value::String(text) => return text.clone(),
            Value::Array(items) => {
                let mut out = String::new();
                for item in items {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        out.push_str(text);
                    } else if let Some(text) = item.as_str() {
                        out.push_str(text);
                    }
                }
                if !out.is_empty() {
                    return out;
                }
            }
            _ => {}
        }
    }
    part.get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn gemini_part_thought_signature(part: &Value) -> Option<&str> {
    part.get("thoughtSignature")
        .or_else(|| part.get("thought_signature"))
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
}

fn anthropic_start_usage() -> Value {
    json!({
        "input_tokens": 1,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "output_tokens": 1
    })
}

fn chat_usage_to_anthropic(usage: Option<&Value>) -> Value {
    let usage = usage.unwrap_or(&Value::Null);
    let parsed = unified_value_to_usage(usage);
    let cache_write = parsed.cache_write_tokens;
    let cached_tokens = parsed.cached_tokens;
    json!({
        "input_tokens": parsed.prompt_tokens.saturating_sub(cached_tokens).saturating_sub(cache_write),
        "cache_creation_input_tokens": cache_write,
        "cache_read_input_tokens": cached_tokens,
        "output_tokens": parsed.completion_tokens
    })
}

fn direct_custom_tool_item_id(call_id: &str) -> String {
    if call_id.starts_with("ctc") {
        call_id.to_string()
    } else if call_id.is_empty() {
        "ctc_0".to_string()
    } else {
        format!("ctc_{call_id}")
    }
}

fn direct_custom_tool_added_item(item_id: &str, status: &str, call_id: &str, name: &str) -> Value {
    json!({
        "id": item_id,
        "type": "custom_tool_call",
        "status": status,
        "call_id": call_id,
        "name": name,
        "input": ""
    })
}

fn direct_custom_tool_done_item(
    item_id: &str,
    status: &str,
    call_id: &str,
    name: &str,
    input: &str,
) -> Value {
    json!({
        "id": item_id,
        "type": "custom_tool_call",
        "status": status,
        "call_id": call_id,
        "name": name,
        "input": custom_tool_input_from_chat_arguments(input)
    })
}

fn trusted_chat_stream_usage(value: &Value, usage_only_chunk: bool) -> Option<Value> {
    let usage = value.get("usage").filter(|usage| !usage.is_null())?;
    if usage_only_chunk || !is_zero_chat_usage_placeholder(usage) {
        Some(usage.clone())
    } else {
        None
    }
}

fn is_zero_chat_usage_placeholder(usage: &Value) -> bool {
    let Some(object) = usage.as_object() else {
        return false;
    };
    if object.is_empty() {
        return false;
    }

    let known_token_paths = [
        "/prompt_tokens",
        "/input_tokens",
        "/completion_tokens",
        "/output_tokens",
        "/total_tokens",
        "/prompt_tokens_details/audio_tokens",
        "/prompt_tokens_details/cached_tokens",
        "/input_tokens_details/audio_tokens",
        "/input_tokens_details/cached_tokens",
        "/completion_tokens_details/audio_tokens",
        "/completion_tokens_details/reasoning_tokens",
        "/completion_tokens_details/accepted_prediction_tokens",
        "/completion_tokens_details/rejected_prediction_tokens",
        "/output_tokens_details/audio_tokens",
        "/output_tokens_details/reasoning_tokens",
        "/output_tokens_details/accepted_prediction_tokens",
        "/output_tokens_details/rejected_prediction_tokens",
    ];

    let mut saw_token_field = false;
    for path in known_token_paths {
        let Some(value) = usage.pointer(path) else {
            continue;
        };
        let Some(count) = value.as_u64() else {
            continue;
        };
        saw_token_field = true;
        if count > 0 {
            return false;
        }
    }

    saw_token_field
}

fn chat_usage_to_responses(usage: Option<&Value>) -> Value {
    let usage = usage.unwrap_or(&Value::Null);
    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    let cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write_tokens = usage
        .pointer("/prompt_tokens_details/cache_write_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cache_write_tokens"))
        .or_else(|| usage.pointer("/prompt_tokens_details/cache_creation_tokens"))
        .or_else(|| usage.pointer("/input_tokens_details/cache_creation_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning_tokens = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .or_else(|| usage.pointer("/output_tokens_details/reasoning_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let mut input_details = json!({
        "cached_tokens": cached_tokens
    });
    if cache_write_tokens > 0 {
        input_details["cache_write_tokens"] = json!(cache_write_tokens);
    }
    json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens,
        "input_tokens_details": input_details,
        "output_tokens_details": {
            "reasoning_tokens": reasoning_tokens
        }
    })
}

fn anthropic_stop_reason(reason: &str) -> &'static str {
    match reason {
        "length" | "max_tokens" | "model_context_window_exceeded" => "max_tokens",
        "tool_calls" | "function_call" | "tool_use" => "tool_use",
        "refusal" | "content_filter" | "sensitive" => "refusal",
        "pause_turn" => "pause_turn",
        "stop_sequence" => "stop_sequence",
        _ => "end_turn",
    }
}

/// Canonical service tier a source-dialect stream event confirmed, if any.
///
/// Accepts the same spellings and shapes the gateway's own usage collector
/// reads (`/service_tier` bare Chat chunk, `/response/service_tier` Responses
/// event) but with a local normalization table: the transformer layer must not
/// depend on the runtime `types` module, and only OpenAI-family dialects carry
/// the field. `default` / unknown values yield `None` — an upstream that never
/// implements the echo is indistinguishable from one that served standard.
fn confirmed_service_tier_from_value(value: &Value) -> Option<&'static str> {
    let tier = ["/service_tier", "/response/service_tier"]
        .iter()
        .find_map(|path| value.pointer(path)?.as_str())?;
    match tier.trim() {
        "priority" | "fast" => Some("fast"),
        "ultrafast" => Some("ultrafast"),
        _ => None,
    }
}

fn stream_error_from_value(event_name: Option<&str>, value: &Value) -> Option<(String, String)> {
    let is_error_event = event_name == Some("error")
        || value.get("event").and_then(Value::as_str) == Some("error")
        || value.get("type").and_then(Value::as_str) == Some("error");
    let error = value
        .get("error")
        .filter(|error| !error.is_null())
        .or_else(|| {
            value
                .pointer("/data/error")
                .filter(|error| !error.is_null())
        });

    if !is_error_event && error.is_none() {
        return None;
    }

    let error = error.unwrap_or(value);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .filter(|message| !message.is_empty())
        .map(ToString::to_string)
        .or_else(|| error.as_str().map(ToString::to_string))
        .unwrap_or_else(|| {
            if error.is_object() || error.is_array() {
                error.to_string()
            } else {
                "stream error".to_string()
            }
        });
    let code = stream_error_code_from_value(error.get("code"))
        .or_else(|| stream_error_code_from_value(error.get("type")))
        .or_else(|| stream_error_code_from_value(value.get("code")))
        .unwrap_or_else(|| "stream_error".to_string());

    Some((code, message))
}

fn stream_error_code_from_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => {
            let code = text.trim();
            (!code.is_empty() && code != "error").then(|| code.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[derive(Debug, Default)]
struct TargetStreamState {
    sent_start: bool,
    finished: bool,
    id: String,
    model: String,
    next_anthropic_index: usize,
    open_anthropic_text: Option<usize>,
    open_anthropic_reasoning: Option<usize>,
    pending_anthropic_reasoning_signature: Option<String>,
    open_anthropic_tools: HashMap<usize, TargetAnthropicToolState>,
    open_anthropic_current_tool_index: Option<usize>,
    pending_anthropic_stop_reason: Option<String>,
    /// Tracks Chat target tool indices already opened (`value = name_emitted`)
    /// so subsequent argument deltas omit repeated `id`/`type`/`name`.
    emitted_chat_tools: HashMap<usize, bool>,
    seen_response_tools: HashMap<usize, TargetResponseToolState>,
    responses_next_output_index: usize,
    responses_reasoning_started: bool,
    responses_reasoning_done: bool,
    responses_reasoning_output_index: Option<usize>,
    responses_reasoning_summary_part_started: bool,
    responses_reasoning_summary: String,
    responses_reasoning_encrypted_content: Option<String>,
    responses_message_output_index: Option<usize>,
    responses_message_done: bool,
    responses_message_text: String,
    pending_responses_finish_reason: Option<String>,
    pending_responses_encrypted_content: Option<String>,
    /// Service tier the source stream confirmed, staged for the terminal
    /// `response.completed` snapshot this target renders.
    pending_responses_service_tier: Option<String>,
    pending_gemini_reasoning_signature: Option<String>,
    pending_gemini_tool_signatures: HashMap<usize, String>,
    pending_gemini_tools: HashMap<usize, TargetGeminiToolState>,
    pending_gemini_finish_reason: Option<String>,
    gemini_seen_reasoning: bool,
    gemini_seen_tool: bool,
    gemini_emitted_signature: bool,
    codex_tool_context: Option<CodexToolContext>,
}

#[derive(Debug, Clone, Default)]
struct TargetAnthropicToolState {
    block_index: usize,
}

#[derive(Debug, Clone, Default)]
struct TargetResponseToolState {
    id: String,
    output_index: usize,
    tool_type: String,
    name: String,
    response_item_id: String,
    response_item_name: String,
    arguments: String,
    reasoning_content: String,
    done: bool,
}

#[derive(Debug, Clone, Default)]
struct TargetGeminiToolState {
    id: String,
    name: String,
    arguments: String,
}

impl TargetStreamState {
    fn with_conversion_context(context: ConversionContext) -> Self {
        Self {
            codex_tool_context: context.codex_tool_context,
            ..Default::default()
        }
    }

    fn write(&mut self, target: AiProtocol, event: UnifiedStreamEvent) -> Vec<Vec<u8>> {
        if let UnifiedStreamEvent::StreamError { code, message } = event {
            return self.write_stream_error(target, code, message);
        }
        match target {
            AiProtocol::AnthropicMessages => self.write_anthropic(event),
            AiProtocol::OpenAiChat => self.write_chat(event),
            AiProtocol::OpenAiResponses => self.write_responses(event),
            AiProtocol::GeminiNative => self.write_gemini(event),
        }
    }

    fn finish(&mut self, target: AiProtocol) -> Vec<Vec<u8>> {
        if self.finished {
            return Vec::new();
        }
        if target == AiProtocol::AnthropicMessages {
            let reason = if self.pending_anthropic_stop_reason.is_some() {
                None
            } else {
                Some("stop".to_string())
            };
            return self.finish_anthropic_message(reason, None, true);
        }
        if target == AiProtocol::OpenAiResponses {
            let reason = if self.pending_responses_finish_reason.is_some() {
                None
            } else {
                Some("stop".to_string())
            };
            return self.finish_responses_response(reason, None, true);
        }
        if target == AiProtocol::GeminiNative {
            return self.finish_gemini_response(None, None, true);
        }
        self.write(
            target,
            UnifiedStreamEvent::Finish {
                reason: Some("stop".to_string()),
                usage: None,
                service_tier: None,
            },
        )
    }

    fn write_stream_error(
        &mut self,
        target: AiProtocol,
        code: String,
        message: String,
    ) -> Vec<Vec<u8>> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        match target {
            AiProtocol::OpenAiResponses => self.write_responses_stream_error(code, message),
            AiProtocol::AnthropicMessages => vec![sse_event(
                Some("error"),
                &json!({
                    "type": "error",
                    "error": {
                        "type": code,
                        "message": message
                    }
                }),
            )],
            AiProtocol::OpenAiChat => vec![sse_event(
                None,
                &json!({
                    "error": {
                        "message": message,
                        "type": code,
                        "code": code
                    }
                }),
            )],
            AiProtocol::GeminiNative => {
                vec![sse_event(None, &gemini_stream_error(&code, &message))]
            }
        }
    }

    fn write_responses_stream_error(&mut self, code: String, message: String) -> Vec<Vec<u8>> {
        if !self.sent_start {
            return vec![sse_event(
                Some("error"),
                &json!({
                    "type": "error",
                    "code": code,
                    "message": message
                }),
            )];
        }
        vec![sse_event(
            Some("response.failed"),
            &json!({
                "type": "response.failed",
                "response": {
                    "id": self.id,
                    "object": "response",
                    "status": "failed",
                    "model": self.model,
                    "output": self.completed_responses_output(),
                    "error": {
                        "type": "server_error",
                        "code": code,
                        "message": message
                    }
                }
            }),
        )]
    }

    fn remember_start(&mut self, id: String, model: String) {
        if !id.is_empty() {
            self.id = id;
        }
        if !model.is_empty() {
            self.model = model;
        }
        self.sent_start = true;
    }

    fn ensure_anthropic_start(&mut self) -> Option<Vec<u8>> {
        if self.sent_start {
            return None;
        }
        self.remember_start(String::new(), String::new());
        Some(sse_event(
            Some("message_start"),
            &json!({
                "type": "message_start",
                "message": {
                    "id": self.id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": anthropic_start_usage()
                }
            }),
        ))
    }

    fn ensure_chat_start(&mut self) -> Vec<Vec<u8>> {
        if self.sent_start {
            return Vec::new();
        }
        self.remember_start(String::new(), String::new());
        vec![self.chat_chunk(json!({"role": "assistant"}), None)]
    }

    fn responses_start_events(&self) -> Vec<Vec<u8>> {
        let response = json!({
            "id": self.id,
            "object": "response",
            "status": "in_progress",
            "model": self.model,
            "output": []
        });
        vec![
            sse_event(
                Some("response.created"),
                &json!({
                    "type": "response.created",
                    "response": response
                }),
            ),
            sse_event(
                Some("response.in_progress"),
                &json!({
                    "type": "response.in_progress",
                    "response": response
                }),
            ),
        ]
    }

    fn ensure_responses_start(&mut self) -> Vec<Vec<u8>> {
        if self.sent_start {
            return Vec::new();
        }
        self.remember_start(String::new(), String::new());
        self.responses_start_events()
    }

    fn next_responses_output_index(&mut self) -> usize {
        let output_index = self.responses_next_output_index;
        self.responses_next_output_index += 1;
        output_index
    }

    fn responses_reasoning_item_id(&self) -> String {
        format!(
            "reasoning_{}",
            self.responses_reasoning_output_index.unwrap_or_default()
        )
    }

    fn responses_message_item_id(&self) -> String {
        let output_index = self.responses_message_output_index.unwrap_or_default();
        if self.id.is_empty() {
            format!("msg_gateway_{output_index}")
        } else {
            format!("msg_{}_{output_index}", self.id)
        }
    }

    fn ensure_responses_reasoning_item(&mut self, out: &mut Vec<Vec<u8>>) {
        out.extend(self.ensure_responses_start());
        if self.responses_reasoning_output_index.is_some() {
            return;
        }
        let output_index = self.next_responses_output_index();
        self.responses_reasoning_output_index = Some(output_index);
        self.responses_reasoning_started = true;
        let item_id = self.responses_reasoning_item_id();
        out.push(sse_event(
            Some("response.output_item.added"),
            &json!({
                "type": "response.output_item.added",
                "output_index": output_index,
                "item": {
                    "id": item_id,
                    "type": "reasoning",
                    "status": "in_progress",
                    "summary": []
                }
            }),
        ));
    }

    fn finish_responses_reasoning_item(&mut self, out: &mut Vec<Vec<u8>>) {
        if self.responses_reasoning_done
            || (!self.responses_reasoning_started
                && self.pending_responses_encrypted_content.is_none())
        {
            return;
        }
        self.ensure_responses_reasoning_item(out);
        let summary = if self.responses_reasoning_summary.is_empty() {
            Vec::new()
        } else {
            vec![json!({
                "type": "summary_text",
                "text": self.responses_reasoning_summary
            })]
        };
        let mut item = json!({
            "id": self.responses_reasoning_item_id(),
            "type": "reasoning",
            "status": "completed",
            "summary": summary
        });
        if let Some(encrypted_content) = self.pending_responses_encrypted_content.take() {
            self.responses_reasoning_encrypted_content = Some(encrypted_content.clone());
            item["encrypted_content"] = json!(encrypted_content);
        } else if let Some(encrypted_content) = &self.responses_reasoning_encrypted_content {
            item["encrypted_content"] = json!(encrypted_content);
        }
        let output_index = self.responses_reasoning_output_index.unwrap_or_default();
        if self.responses_reasoning_summary_part_started {
            out.push(sse_event(
                Some("response.reasoning_summary_text.done"),
                &json!({
                    "type": "response.reasoning_summary_text.done",
                    "item_id": self.responses_reasoning_item_id(),
                    "output_index": output_index,
                    "summary_index": 0,
                    "text": self.responses_reasoning_summary
                }),
            ));
            out.push(sse_event(
                Some("response.reasoning_summary_part.done"),
                &json!({
                    "type": "response.reasoning_summary_part.done",
                    "item_id": self.responses_reasoning_item_id(),
                    "output_index": output_index,
                    "summary_index": 0,
                    "part": {
                        "type": "summary_text",
                        "text": self.responses_reasoning_summary
                    }
                }),
            ));
        }
        out.push(sse_event(
            Some("response.output_item.done"),
            &json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": item
            }),
        ));
        self.responses_reasoning_done = true;
        self.responses_reasoning_summary_part_started = false;
    }

    fn ensure_responses_message_item(&mut self, out: &mut Vec<Vec<u8>>) -> (String, usize) {
        out.extend(self.ensure_responses_start());
        if self.responses_message_output_index.is_none() {
            let output_index = self.next_responses_output_index();
            self.responses_message_output_index = Some(output_index);
            let item_id = self.responses_message_item_id();
            out.push(sse_event(
                Some("response.output_item.added"),
                &json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": item_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            ));
            out.push(sse_event(
                Some("response.content_part.added"),
                &json!({
                    "type": "response.content_part.added",
                    "item_id": item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": {
                        "type": "output_text",
                        "annotations": [],
                        "text": ""
                    }
                }),
            ));
        }
        (
            self.responses_message_item_id(),
            self.responses_message_output_index.unwrap_or_default(),
        )
    }

    fn finish_responses_message_item(&mut self, out: &mut Vec<Vec<u8>>) {
        if self.responses_message_done {
            return;
        }
        let Some(output_index) = self.responses_message_output_index else {
            return;
        };
        let item_id = self.responses_message_item_id();
        let content_part = json!({
            "type": "output_text",
            "annotations": [],
            "text": self.responses_message_text
        });
        out.push(sse_event(
            Some("response.output_text.done"),
            &json!({
                "type": "response.output_text.done",
                "item_id": item_id,
                "output_index": output_index,
                "content_index": 0,
                "text": self.responses_message_text
            }),
        ));
        out.push(sse_event(
            Some("response.content_part.done"),
            &json!({
                "type": "response.content_part.done",
                "item_id": item_id,
                "output_index": output_index,
                "content_index": 0,
                "part": content_part
            }),
        ));
        out.push(sse_event(
            Some("response.output_item.done"),
            &json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": self.responses_message_output_item()
            }),
        ));
        self.responses_message_done = true;
    }

    fn finish_responses_tool_items(&mut self, out: &mut Vec<Vec<u8>>) {
        let mut tools = self
            .seen_response_tools
            .iter()
            .filter_map(|(index, tool)| (!tool.done).then_some((*index, tool.output_index)))
            .collect::<Vec<_>>();
        tools.sort_by_key(|(_, output_index)| *output_index);

        for (index, _) in tools {
            let Some(tool) = self.seen_response_tools.get_mut(&index) else {
                continue;
            };
            tool.done = true;
            let tool_id = tool.id.clone();
            let output_index = tool.output_index;
            let tool_type = tool.tool_type.clone();
            let tool_name = tool.name.clone();
            let tool_arguments = tool.arguments.clone();
            let tool_reasoning_content = tool.reasoning_content.clone();
            let response_item_id = tool.response_item_id.clone();
            let mut done_item = if tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                direct_custom_tool_done_item(
                    &response_item_id,
                    "completed",
                    &tool_id,
                    &tool_name,
                    &tool_arguments,
                )
            } else {
                response_tool_done_item_from_chat_name(
                    &response_item_id,
                    "completed",
                    &tool_id,
                    &tool_name,
                    &tool_arguments,
                    self.codex_tool_context.as_ref(),
                )
            };
            if !tool_reasoning_content.trim().is_empty() {
                done_item["reasoning_content"] = json!(tool_reasoning_content);
            }
            let is_custom_tool = tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL
                || is_custom_tool_chat_name(&tool_name, self.codex_tool_context.as_ref());
            if is_custom_tool {
                let tool_input = custom_tool_input_from_chat_arguments(&tool_arguments);
                // Emit one full-input delta then done (flush semantics; no per-fragment unpack).
                if !tool_input.is_empty() {
                    out.push(sse_event(
                        Some("response.custom_tool_call_input.delta"),
                        &json!({
                            "type": "response.custom_tool_call_input.delta",
                            "item_id": response_item_id.clone(),
                            "output_index": output_index,
                            "delta": tool_input
                        }),
                    ));
                }
                out.push(sse_event(
                    Some("response.custom_tool_call_input.done"),
                    &json!({
                        "type": "response.custom_tool_call_input.done",
                        "item_id": response_item_id.clone(),
                        "output_index": output_index,
                        "input": tool_input
                    }),
                ));
                out.push(sse_event(
                    Some("response.output_item.done"),
                    &json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": done_item
                    }),
                ));
            } else {
                out.push(sse_event(
                    Some("response.function_call_arguments.done"),
                    &json!({
                        "type": "response.function_call_arguments.done",
                        "item_id": response_item_id.clone(),
                        "output_index": output_index,
                        "arguments": tool_arguments.clone()
                    }),
                ));
                out.push(sse_event(
                    Some("response.output_item.done"),
                    &json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": done_item
                    }),
                ));
            }
        }
    }

    fn finish_responses_response(
        &mut self,
        reason: Option<String>,
        usage: Option<Value>,
        force: bool,
    ) -> Vec<Vec<u8>> {
        if self.finished {
            return Vec::new();
        }

        let mut out = Vec::new();
        out.extend(self.ensure_responses_start());
        if let Some(reason) = reason {
            self.pending_responses_finish_reason = Some(reason);
        }
        self.finish_responses_reasoning_item(&mut out);
        self.finish_responses_message_item(&mut out);
        self.finish_responses_tool_items(&mut out);

        if usage.is_none() && !force {
            return out;
        }

        self.finished = true;
        let finish_reason = self
            .pending_responses_finish_reason
            .take()
            .unwrap_or_else(|| "stop".to_string());
        if finish_reason == "error" {
            let mut response = json!({
                "id": self.id,
                "object": "response",
                "status": "failed",
                "model": self.model,
                "output": self.completed_responses_output(),
                "error": {
                    "type": "server_error",
                    "code": "response_error",
                    "message": "Response failed"
                }
            });
            if let Some(usage) = usage.as_ref() {
                response["usage"] = chat_usage_to_responses(Some(usage));
            }
            out.push(sse_event(
                Some("response.failed"),
                &json!({
                    "type": "response.failed",
                    "response": response
                }),
            ));
            return out;
        }
        if finish_reason == "cancelled" || finish_reason == "canceled" {
            let mut response = json!({
                "id": self.id,
                "object": "response",
                "status": "canceled",
                "model": self.model,
                "output": self.completed_responses_output()
            });
            if let Some(usage) = usage.as_ref() {
                response["usage"] = chat_usage_to_responses(Some(usage));
            }
            out.push(sse_event(
                Some("response.cancelled"),
                &json!({
                    "type": "response.cancelled",
                    "response": response
                }),
            ));
            return out;
        }
        let incomplete_reason = match finish_reason.as_str() {
            "length" | "max_tokens" | "model_context_window_exceeded" => Some("max_output_tokens"),
            "content_filter" | "refusal" | "sensitive" => Some("content_filter"),
            _ => None,
        };
        let mut response = json!({
            "id": self.id,
            "object": "response",
            "status": if incomplete_reason.is_some() { "incomplete" } else { "completed" },
            "model": self.model,
            "output": self.completed_responses_output()
        });
        if let Some(reason) = incomplete_reason {
            response["incomplete_details"] = json!({ "reason": reason });
        }
        if let Some(usage) = usage.as_ref() {
            response["usage"] = chat_usage_to_responses(Some(usage));
        }
        if let Some(service_tier) = self.pending_responses_service_tier.as_ref() {
            // Re-render the source dialect's tier echo so the gateway's usage
            // collector (which observes this converted stream) still bills the
            // confirmed tier.
            response["service_tier"] = json!(service_tier);
        }
        out.push(sse_event(
            Some("response.completed"),
            &json!({
                "type": "response.completed",
                "response": response
            }),
        ));
        out
    }

    fn responses_reasoning_output_item(&self) -> Option<Value> {
        self.responses_reasoning_output_index.map(|_| {
            let summary = if self.responses_reasoning_summary.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "type": "summary_text",
                    "text": self.responses_reasoning_summary
                })]
            };
            let mut item = json!({
                "id": self.responses_reasoning_item_id(),
                "type": "reasoning",
                "status": if self.responses_reasoning_done { "completed" } else { "in_progress" },
                "summary": summary
            });
            if let Some(encrypted_content) = &self.responses_reasoning_encrypted_content {
                item["encrypted_content"] = json!(encrypted_content);
            } else if let Some(encrypted_content) = &self.pending_responses_encrypted_content {
                item["encrypted_content"] = json!(encrypted_content);
            }
            item
        })
    }

    fn responses_message_output_item(&self) -> Value {
        json!({
            "id": self.responses_message_item_id(),
            "type": "message",
            "status": if self.responses_message_done { "completed" } else { "in_progress" },
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "annotations": [],
                "text": self.responses_message_text
            }]
        })
    }

    fn responses_tool_output_item(&self, tool: &TargetResponseToolState) -> Value {
        let mut item = response_tool_done_item_from_chat_name(
            &tool.response_item_id,
            if tool.done {
                "completed"
            } else {
                "in_progress"
            },
            &tool.id,
            &tool.name,
            &tool.arguments,
            self.codex_tool_context.as_ref(),
        );
        if !tool.reasoning_content.trim().is_empty() {
            item["reasoning_content"] = json!(tool.reasoning_content);
        }
        item
    }

    fn append_reasoning_to_active_response_tools(&mut self, text: &str) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let mut appended = false;
        for tool in self
            .seen_response_tools
            .values_mut()
            .filter(|tool| !tool.done)
        {
            if tool.reasoning_content.is_empty() {
                tool.reasoning_content = text.trim_start().to_string();
            } else {
                tool.reasoning_content.push_str(text);
            }
            appended = true;
        }
        appended
    }

    fn completed_responses_output(&self) -> Vec<Value> {
        let mut output_items = Vec::new();
        if let Some(output_index) = self.responses_reasoning_output_index {
            if let Some(item) = self.responses_reasoning_output_item() {
                output_items.push((output_index, item));
            }
        }
        if let Some(output_index) = self.responses_message_output_index {
            output_items.push((output_index, self.responses_message_output_item()));
        }
        for tool in self.seen_response_tools.values() {
            output_items.push((tool.output_index, self.responses_tool_output_item(tool)));
        }
        output_items.sort_by_key(|(output_index, _)| *output_index);
        output_items
            .into_iter()
            .map(|(_, item)| item)
            .collect::<Vec<_>>()
    }

    fn close_anthropic_text_block(&mut self, out: &mut Vec<Vec<u8>>) {
        if let Some(index) = self.open_anthropic_text.take() {
            out.push(sse_event(
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": index}),
            ));
        }
    }

    fn close_anthropic_reasoning_block(&mut self, out: &mut Vec<Vec<u8>>) {
        if let Some(index) = self.open_anthropic_reasoning.take() {
            if let Some(signature) = self.pending_anthropic_reasoning_signature.take() {
                out.push(sse_event(
                    Some("content_block_delta"),
                    &json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "signature_delta", "signature": signature}
                    }),
                ));
            }
            out.push(sse_event(
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": index}),
            ));
        }
    }

    fn close_anthropic_tool_block(&mut self, out: &mut Vec<Vec<u8>>) {
        let Some(index) = self.open_anthropic_current_tool_index.take() else {
            return;
        };
        if let Some(state) = self.open_anthropic_tools.remove(&index) {
            out.push(sse_event(
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": state.block_index}),
            ));
        }
    }

    fn close_all_anthropic_tool_blocks(&mut self, out: &mut Vec<Vec<u8>>) {
        self.open_anthropic_current_tool_index = None;
        if self.open_anthropic_tools.is_empty() {
            return;
        }
        let mut remaining = self
            .open_anthropic_tools
            .drain()
            .map(|(_, state)| state.block_index)
            .collect::<Vec<_>>();
        remaining.sort_unstable();
        for block_index in remaining {
            out.push(sse_event(
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": block_index}),
            ));
        }
    }

    fn finish_anthropic_message(
        &mut self,
        reason: Option<String>,
        usage: Option<Value>,
        force: bool,
    ) -> Vec<Vec<u8>> {
        if self.finished {
            return Vec::new();
        }

        let mut out = Vec::new();
        if let Some(start) = self.ensure_anthropic_start() {
            out.push(start);
        }
        if let Some(reason) = reason.as_deref() {
            self.pending_anthropic_stop_reason = Some(anthropic_stop_reason(reason).to_string());
        }

        self.close_anthropic_reasoning_block(&mut out);
        self.close_anthropic_text_block(&mut out);
        self.close_all_anthropic_tool_blocks(&mut out);
        self.flush_pending_anthropic_signature_block(&mut out);

        if usage.is_none() && !force {
            return out;
        }

        self.finished = true;
        let stop_reason = self
            .pending_anthropic_stop_reason
            .take()
            .unwrap_or_else(|| "end_turn".to_string());
        out.push(sse_event(
            Some("message_delta"),
            &json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": stop_reason,
                    "stop_sequence": Value::Null
                },
                "usage": chat_usage_to_anthropic(usage.as_ref())
            }),
        ));
        out.push(sse_event(
            Some("message_stop"),
            &json!({"type": "message_stop"}),
        ));
        out
    }

    fn flush_pending_anthropic_signature_block(&mut self, out: &mut Vec<Vec<u8>>) {
        let Some(signature) = self.pending_anthropic_reasoning_signature.take() else {
            return;
        };
        let index = self.next_anthropic_index;
        self.next_anthropic_index += 1;
        out.push(sse_event(
            Some("content_block_start"),
            &json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {"type": "thinking", "thinking": ""}
            }),
        ));
        out.push(sse_event(
            Some("content_block_delta"),
            &json!({
                "type": "content_block_delta",
                "index": index,
                "delta": {"type": "signature_delta", "signature": signature}
            }),
        ));
        out.push(sse_event(
            Some("content_block_stop"),
            &json!({"type": "content_block_stop", "index": index}),
        ));
    }

    fn write_anthropic(&mut self, event: UnifiedStreamEvent) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        match event {
            UnifiedStreamEvent::Start { id, model } => {
                if !self.sent_start {
                    self.remember_start(id, model);
                    out.push(sse_event(
                        Some("message_start"),
                        &json!({
                            "type": "message_start",
                            "message": {
                                "id": self.id,
                                "type": "message",
                                "role": "assistant",
                                "model": self.model,
                                "content": [],
                                "stop_reason": Value::Null,
                                "stop_sequence": Value::Null,
                                "usage": anthropic_start_usage()
                            }
                        }),
                    ));
                }
            }
            UnifiedStreamEvent::TextDelta(text) => {
                if let Some(start) = self.ensure_anthropic_start() {
                    out.push(start);
                }
                self.close_anthropic_tool_block(&mut out);
                self.close_anthropic_reasoning_block(&mut out);
                if self.open_anthropic_text.is_none() {
                    self.flush_pending_anthropic_signature_block(&mut out);
                }
                if self.open_anthropic_text.is_none() {
                    let index = self.next_anthropic_index;
                    self.next_anthropic_index += 1;
                    self.open_anthropic_text = Some(index);
                    out.push(sse_event(
                        Some("content_block_start"),
                        &json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {"type": "text", "text": ""}
                        }),
                    ));
                }
                let index = self.open_anthropic_text.unwrap_or(0);
                out.push(sse_event(
                    Some("content_block_delta"),
                    &json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "text_delta", "text": text}
                    }),
                ));
            }
            UnifiedStreamEvent::ReasoningDelta(text) => {
                if let Some(start) = self.ensure_anthropic_start() {
                    out.push(start);
                }
                self.close_anthropic_tool_block(&mut out);
                self.close_anthropic_text_block(&mut out);
                if self.open_anthropic_reasoning.is_none() {
                    let index = self.next_anthropic_index;
                    self.next_anthropic_index += 1;
                    self.open_anthropic_reasoning = Some(index);
                    out.push(sse_event(
                        Some("content_block_start"),
                        &json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {"type": "thinking", "thinking": ""}
                        }),
                    ));
                }
                let index = self.open_anthropic_reasoning.unwrap_or(0);
                out.push(sse_event(
                    Some("content_block_delta"),
                    &json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "thinking_delta", "thinking": text}
                    }),
                ));
            }
            UnifiedStreamEvent::ReasoningSignature { signature } => {
                if let Some(signature) =
                    decode_signature_for(SignatureProvider::Anthropic, &signature)
                {
                    self.pending_anthropic_reasoning_signature = Some(signature);
                }
            }
            UnifiedStreamEvent::ToolCallSignature { .. }
            | UnifiedStreamEvent::StreamError { .. } => {}
            UnifiedStreamEvent::RawAnthropicContentBlock { block } => {
                if let Some(start) = self.ensure_anthropic_start() {
                    out.push(start);
                }
                self.close_anthropic_text_block(&mut out);
                self.close_anthropic_reasoning_block(&mut out);
                self.close_anthropic_tool_block(&mut out);
                self.flush_pending_anthropic_signature_block(&mut out);
                let index = self.next_anthropic_index;
                self.next_anthropic_index += 1;
                out.push(sse_event(
                    Some("content_block_start"),
                    &json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": block
                    }),
                ));
                out.push(sse_event(
                    Some("content_block_stop"),
                    &json!({"type": "content_block_stop", "index": index}),
                ));
            }
            UnifiedStreamEvent::ToolCall {
                index,
                id,
                tool_type: _,
                name,
                arguments,
            } => {
                if let Some(start) = self.ensure_anthropic_start() {
                    out.push(start);
                }
                self.close_anthropic_text_block(&mut out);
                self.close_anthropic_reasoning_block(&mut out);
                let is_new_tool = !self.open_anthropic_tools.contains_key(&index);
                if is_new_tool {
                    self.close_anthropic_tool_block(&mut out);
                }
                self.flush_pending_anthropic_signature_block(&mut out);
                if is_new_tool {
                    let block_index = self.next_anthropic_index;
                    self.next_anthropic_index += 1;
                    self.open_anthropic_tools
                        .insert(index, TargetAnthropicToolState { block_index });
                    self.open_anthropic_current_tool_index = Some(index);
                    let normalized_id = super::anthropic::normalize_anthropic_tool_use_id(&id);
                    out.push(sse_event(
                        Some("content_block_start"),
                        &json!({
                            "type": "content_block_start",
                            "index": block_index,
                            "content_block": {"type": "tool_use", "id": normalized_id, "name": name, "input": {}}
                        }),
                    ));
                }
                let block_index = self
                    .open_anthropic_tools
                    .get(&index)
                    .map(|state| state.block_index)
                    .unwrap_or(0);
                if !arguments.is_empty() {
                    out.push(sse_event(
                        Some("content_block_delta"),
                        &json!({
                            "type": "content_block_delta",
                            "index": block_index,
                            "delta": {"type": "input_json_delta", "partial_json": arguments}
                        }),
                    ));
                }
            }
            UnifiedStreamEvent::Finish {
                reason,
                usage,
                service_tier: _,
            } => {
                if reason.as_deref() == Some("error") {
                    return self.write_stream_error(
                        AiProtocol::AnthropicMessages,
                        "response_error".to_string(),
                        "Response failed".to_string(),
                    );
                }
                // Anthropic has no `service_tier` response field to render it
                // into; the tier is OpenAI-family only.
                return self.finish_anthropic_message(reason, usage, false);
            }
        }
        out
    }

    fn write_chat(&mut self, event: UnifiedStreamEvent) -> Vec<Vec<u8>> {
        match event {
            UnifiedStreamEvent::Start { id, model } => {
                if self.sent_start {
                    return Vec::new();
                }
                self.remember_start(id, model);
                vec![self.chat_chunk(json!({"role": "assistant"}), None)]
            }
            UnifiedStreamEvent::TextDelta(text) => {
                let mut out = self.ensure_chat_start();
                out.push(self.chat_chunk(json!({"content": text}), None));
                out
            }
            UnifiedStreamEvent::ReasoningDelta(text) => {
                let mut out = self.ensure_chat_start();
                out.push(self.chat_chunk(json!({"reasoning_content": text}), None));
                out
            }
            UnifiedStreamEvent::ReasoningSignature { .. }
            | UnifiedStreamEvent::ToolCallSignature { .. }
            | UnifiedStreamEvent::RawAnthropicContentBlock { .. }
            | UnifiedStreamEvent::StreamError { .. } => Vec::new(),
            UnifiedStreamEvent::ToolCall {
                index,
                id,
                tool_type,
                name,
                arguments,
            } => {
                let mut out = self.ensure_chat_start();
                if tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                    let mut custom = json!({
                        "call_id": id.clone(),
                        "name": name.clone()
                    });
                    if !arguments.is_empty() {
                        custom["input"] = json!(arguments);
                    }
                    out.push(self.chat_chunk(
                        json!({
                            "tool_calls": [{
                                "index": index,
                                "id": id,
                                "type": TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
                                "function": {"name": ""},
                                "response_custom_tool_call": custom
                            }]
                        }),
                        None,
                    ));
                    return out;
                }
                let is_first_chunk = !self.emitted_chat_tools.contains_key(&index);
                let sent_name = self.emitted_chat_tools.entry(index).or_insert(false);
                let mut function = json!({"arguments": arguments});
                if !name.is_empty() && !*sent_name {
                    function["name"] = json!(name);
                    *sent_name = true;
                }
                let mut tool_call_entry = json!({
                    "index": index,
                    "function": function
                });
                if is_first_chunk {
                    if !id.is_empty() {
                        tool_call_entry["id"] = json!(id);
                    }
                    tool_call_entry["type"] = json!("function");
                }
                out.push(self.chat_chunk(
                    json!({
                        "tool_calls": [tool_call_entry]
                    }),
                    None,
                ));
                out
            }
            UnifiedStreamEvent::Finish {
                reason,
                usage,
                service_tier,
            } => {
                if reason.as_deref() == Some("error") {
                    return self.write_stream_error(
                        AiProtocol::OpenAiChat,
                        "response_error".to_string(),
                        "Response failed".to_string(),
                    );
                }
                if self.finished {
                    return Vec::new();
                }
                let mut out = self.ensure_chat_start();
                self.finished = true;
                // OpenAI include_usage wire: usage-only chunk with empty choices before finish.
                // Normalize source-protocol usage keys (input_tokens/output_tokens) into Chat shape.
                // The tier echo rides on this top-level chunk (the same place
                // OpenAI puts it), re-rendered from the source dialect so the
                // gateway's usage collector — which observes the converted
                // stream — still bills the confirmed tier.
                let mut usage_chunk = json!({
                    "id": if self.id.is_empty() { "chatcmpl_gateway" } else { &self.id },
                    "object": "chat.completion.chunk",
                    "model": self.model,
                    "choices": []
                });
                if let Some(usage) = usage.as_ref() {
                    usage_chunk["usage"] =
                        llm_usage_to_unified_value(&unified_value_to_usage(usage));
                }
                if let Some(service_tier) = service_tier.as_ref() {
                    usage_chunk["service_tier"] = json!(service_tier);
                }
                if usage.is_some() || service_tier.is_some() {
                    out.push(sse_event(None, &usage_chunk));
                }
                out.push(self.chat_chunk(
                    json!({}),
                    Some(match reason.as_deref() {
                        Some("length") => "length",
                        Some("tool_calls") => "tool_calls",
                        Some("refusal") | Some("content_filter") | Some("sensitive") => {
                            "content_filter"
                        }
                        Some("cancelled") | Some("canceled") => "cancelled",
                        _ => "stop",
                    }),
                ));
                out.push(sse_done());
                out
            }
        }
    }

    fn chat_chunk(&self, delta: Value, finish_reason: Option<&str>) -> Vec<u8> {
        sse_event(
            None,
            &json!({
                "id": if self.id.is_empty() { "chatcmpl_gateway" } else { &self.id },
                "object": "chat.completion.chunk",
                "model": self.model,
                "choices": [{
                    "index": 0,
                    "delta": delta,
                    "finish_reason": finish_reason
                }]
            }),
        )
    }

    fn write_responses(&mut self, event: UnifiedStreamEvent) -> Vec<Vec<u8>> {
        match event {
            UnifiedStreamEvent::Start { id, model } => {
                if self.sent_start {
                    return Vec::new();
                }
                self.remember_start(id, model);
                self.responses_start_events()
            }
            UnifiedStreamEvent::TextDelta(text) => {
                let mut out = Vec::new();
                self.finish_responses_reasoning_item(&mut out);
                let (item_id, output_index) = self.ensure_responses_message_item(&mut out);
                self.responses_message_text.push_str(&text);
                out.push(sse_event(
                    Some("response.output_text.delta"),
                    &json!({
                        "type": "response.output_text.delta",
                        "delta": text,
                        "item_id": item_id,
                        "output_index": output_index,
                        "content_index": 0
                    }),
                ));
                out
            }
            UnifiedStreamEvent::ReasoningDelta(text) => {
                if self.append_reasoning_to_active_response_tools(&text) {
                    return Vec::new();
                }
                let mut out = Vec::new();
                self.ensure_responses_reasoning_item(&mut out);
                self.responses_reasoning_summary.push_str(&text);
                let item_id = self.responses_reasoning_item_id();
                let output_index = self.responses_reasoning_output_index.unwrap_or_default();
                if !self.responses_reasoning_summary_part_started {
                    self.responses_reasoning_summary_part_started = true;
                    out.push(sse_event(
                        Some("response.reasoning_summary_part.added"),
                        &json!({
                            "type": "response.reasoning_summary_part.added",
                            "item_id": item_id,
                            "output_index": output_index,
                            "summary_index": 0,
                            "part": {
                                "type": "summary_text"
                            }
                        }),
                    ));
                }
                out.push(sse_event(
                    Some("response.reasoning_summary_text.delta"),
                    &json!({
                        "type": "response.reasoning_summary_text.delta",
                        "delta": text,
                        "item_id": item_id,
                        "output_index": output_index,
                        "summary_index": 0
                    }),
                ));
                out
            }
            UnifiedStreamEvent::ReasoningSignature { signature } => {
                let Some(encrypted_content) =
                    decode_signature_for(SignatureProvider::OpenAiResponses, &signature)
                else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                self.pending_responses_encrypted_content = Some(encrypted_content);
                self.ensure_responses_reasoning_item(&mut out);
                out
            }
            UnifiedStreamEvent::ToolCallSignature { .. }
            | UnifiedStreamEvent::RawAnthropicContentBlock { .. }
            | UnifiedStreamEvent::StreamError { .. } => Vec::new(),
            UnifiedStreamEvent::ToolCall {
                index,
                id,
                tool_type,
                name,
                arguments,
            } => {
                let mut out = Vec::new();
                let reasoning_for_tool = self.responses_reasoning_summary.clone();
                self.finish_responses_reasoning_item(&mut out);
                self.finish_responses_message_item(&mut out);
                out.extend(self.ensure_responses_start());
                if !self.seen_response_tools.contains_key(&index) {
                    let output_index = self.next_responses_output_index();
                    let response_item_id = if tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                        direct_custom_tool_item_id(&id)
                    } else {
                        response_tool_item_id_from_chat_name(
                            &id,
                            &name,
                            self.codex_tool_context.as_ref(),
                        )
                    };
                    let response_item = if tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                        direct_custom_tool_added_item(&response_item_id, "in_progress", &id, &name)
                    } else {
                        response_tool_added_item_from_chat_name(
                            &response_item_id,
                            "in_progress",
                            &id,
                            &name,
                            self.codex_tool_context.as_ref(),
                        )
                    };
                    self.seen_response_tools.insert(
                        index,
                        TargetResponseToolState {
                            id: id.clone(),
                            output_index,
                            tool_type: tool_type.clone(),
                            name: name.clone(),
                            response_item_id: response_item_id.clone(),
                            response_item_name: name.clone(),
                            arguments: String::new(),
                            reasoning_content: reasoning_for_tool,
                            done: false,
                        },
                    );
                    out.push(sse_event(
                        Some("response.output_item.added"),
                        &json!({
                            "type": "response.output_item.added",
                            "output_index": output_index,
                            "item": response_item
                        }),
                    ));
                }
                let mut item_id = id.clone();
                let mut output_index = 0;
                let mut state_tool_type = tool_type.clone();
                let mut state_name = name.clone();
                if let Some(state) = self.seen_response_tools.get_mut(&index) {
                    if !id.is_empty() {
                        state.id = id.clone();
                    }
                    if !name.is_empty() {
                        state.name = name.clone();
                        state.response_item_name = name.clone();
                    }
                    state.arguments.push_str(&arguments);
                    item_id = state.response_item_id.clone();
                    output_index = state.output_index;
                    state_tool_type = state.tool_type.clone();
                    state_name = state.response_item_name.clone();
                }
                if !arguments.is_empty() {
                    if state_tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL
                        || is_custom_tool_chat_name(&state_name, self.codex_tool_context.as_ref())
                    {
                        // Custom tool args need complete JSON to unpack. Accumulate only;
                        // emit a single delta+done from finish_responses_tool_items.
                        let _ = (item_id, output_index);
                    } else {
                        out.push(sse_event(
                            Some("response.function_call_arguments.delta"),
                            &json!({
                                "type": "response.function_call_arguments.delta",
                                "item_id": item_id,
                                "output_index": output_index,
                                "delta": arguments
                            }),
                        ));
                    }
                }
                out
            }
            UnifiedStreamEvent::Finish {
                reason,
                usage,
                service_tier,
            } => {
                self.pending_responses_service_tier = service_tier;
                self.finish_responses_response(reason, usage, false)
            }
        }
    }

    fn write_gemini(&mut self, event: UnifiedStreamEvent) -> Vec<Vec<u8>> {
        if self.finished {
            return Vec::new();
        }
        match event {
            UnifiedStreamEvent::Start { id, model } => {
                if self.sent_start {
                    return Vec::new();
                }
                self.remember_start(id, model);
                Vec::new()
            }
            UnifiedStreamEvent::TextDelta(text) => {
                vec![self.gemini_chunk(vec![json!({"text": text})], None, None)]
            }
            UnifiedStreamEvent::ReasoningDelta(text) => {
                self.gemini_seen_reasoning = true;
                let mut part = json!({"text": text, "thought": true});
                if !self.gemini_seen_tool && !self.gemini_emitted_signature {
                    if let Some(signature) = self.pending_gemini_reasoning_signature.take() {
                        part["thoughtSignature"] = json!(signature);
                        self.gemini_emitted_signature = true;
                    }
                }
                vec![self.gemini_chunk(vec![part], None, None)]
            }
            UnifiedStreamEvent::ReasoningSignature { signature } => {
                if let Some(signature) = decode_signature_for(SignatureProvider::Gemini, &signature)
                {
                    self.pending_gemini_reasoning_signature = Some(signature);
                }
                Vec::new()
            }
            UnifiedStreamEvent::ToolCallSignature { index, signature } => {
                if let Some(signature) = decode_signature_for(SignatureProvider::Gemini, &signature)
                {
                    self.pending_gemini_tool_signatures.insert(index, signature);
                }
                Vec::new()
            }
            UnifiedStreamEvent::RawAnthropicContentBlock { .. }
            | UnifiedStreamEvent::StreamError { .. } => Vec::new(),
            UnifiedStreamEvent::ToolCall {
                index,
                id,
                name,
                arguments,
                ..
            } => {
                let tool = self.pending_gemini_tools.entry(index).or_default();
                if !id.is_empty() {
                    tool.id = id;
                }
                if !name.is_empty() {
                    tool.name = name;
                }
                tool.arguments.push_str(&arguments);
                self.flush_gemini_tool_calls(false)
            }
            UnifiedStreamEvent::Finish {
                reason,
                usage,
                service_tier: _,
            } => {
                // Gemini has no `service_tier` response field to render it
                // into; the tier is OpenAI-family only.
                self.finish_gemini_response(reason, usage, false)
            }
        }
    }

    fn finish_gemini_response(
        &mut self,
        reason: Option<String>,
        usage: Option<Value>,
        force: bool,
    ) -> Vec<Vec<u8>> {
        if self.finished {
            return Vec::new();
        }
        if reason.as_deref() == Some("error") {
            return self.write_stream_error(
                AiProtocol::GeminiNative,
                "response_error".to_string(),
                "Response failed".to_string(),
            );
        }
        if reason.is_some() {
            self.pending_gemini_finish_reason = reason;
        }
        let mut out = self.flush_gemini_tool_calls(
            self.pending_gemini_finish_reason.as_deref() == Some("tool_calls"),
        );
        // Chat providers may send their final usage after finish_reason.
        if usage.is_none() && !force {
            return out;
        }
        self.finished = true;
        if self.gemini_seen_reasoning && !self.gemini_seen_tool && !self.gemini_emitted_signature {
            let signature = self
                .pending_gemini_reasoning_signature
                .take()
                .unwrap_or_else(|| DEFAULT_GEMINI_THOUGHT_SIGNATURE.to_string());
            out.push(self.gemini_chunk(
                vec![json!({
                    "text": "",
                    "thought": true,
                    "thoughtSignature": signature
                })],
                None,
                None,
            ));
            self.gemini_emitted_signature = true;
        }
        let finish_reason = match self.pending_gemini_finish_reason.take().as_deref() {
            Some("length") => "MAX_TOKENS",
            Some("refusal") | Some("content_filter") | Some("sensitive") => "SAFETY",
            _ => "STOP",
        };
        out.push(self.gemini_chunk(
            Vec::new(),
            Some(finish_reason),
            unified_usage_to_gemini_metadata(usage),
        ));
        out
    }

    fn flush_gemini_tool_calls(&mut self, force_all: bool) -> Vec<Vec<u8>> {
        let mut tool_indexes = self
            .pending_gemini_tools
            .iter()
            .filter_map(|(index, tool)| {
                self.gemini_tool_arguments_value(tool, force_all)
                    .map(|_| *index)
            })
            .collect::<Vec<_>>();
        tool_indexes.sort_unstable();

        let mut parts = Vec::new();
        for index in tool_indexes {
            let Some(tool) = self.pending_gemini_tools.remove(&index) else {
                continue;
            };
            let Some(args) = self.gemini_tool_arguments_value(&tool, force_all) else {
                continue;
            };
            let mut part = json!({
                "functionCall": {
                    "id": tool.id,
                    "name": tool.name,
                    "args": args
                }
            });
            let signature = self
                .pending_gemini_tool_signatures
                .remove(&index)
                .or_else(|| self.pending_gemini_reasoning_signature.take())
                .or_else(|| {
                    (!self.gemini_emitted_signature)
                        .then(|| DEFAULT_GEMINI_THOUGHT_SIGNATURE.to_string())
                });
            if let Some(signature) = signature {
                part["thoughtSignature"] = json!(signature);
                self.gemini_emitted_signature = true;
            }
            self.gemini_seen_tool = true;
            parts.push(part);
        }

        if parts.is_empty() {
            Vec::new()
        } else {
            vec![self.gemini_chunk(parts, None, None)]
        }
    }

    fn gemini_tool_arguments_value(
        &self,
        tool: &TargetGeminiToolState,
        force_all: bool,
    ) -> Option<Value> {
        if tool.name.is_empty() {
            return None;
        }
        let arguments = tool.arguments.trim();
        if arguments.is_empty() {
            return force_all.then(|| json!({}));
        }
        serde_json::from_str::<Value>(arguments)
            .ok()
            .or_else(|| force_all.then(|| json!({})))
    }

    fn gemini_chunk(
        &self,
        parts: Vec<Value>,
        finish_reason: Option<&str>,
        usage: Option<Value>,
    ) -> Vec<u8> {
        let mut candidate = json!({
            "content": {
                "role": "model",
                "parts": parts
            }
        });
        if let Some(finish_reason) = finish_reason {
            candidate["finishReason"] = json!(finish_reason);
        }
        let mut payload = json!({
            "responseId": self.id,
            "modelVersion": self.model,
            "candidates": [candidate]
        });
        if let Some(usage) = usage {
            payload["usageMetadata"] = usage;
        }
        sse_event(None, &payload)
    }
}
