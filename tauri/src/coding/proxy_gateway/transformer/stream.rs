use super::llm::{TOOL_TYPE_FUNCTION, TOOL_TYPE_RESPONSES_CUSTOM_TOOL};
use super::shared::signature::{
    decode_signature_for, encode_signature, SignatureProvider, DEFAULT_GEMINI_THOUGHT_SIGNATURE,
};
use super::sse::{append_utf8_safe, parse_sse_block, sse_done, sse_event, take_sse_block};
use super::types::{AiProtocol, ConversionRoute};
use serde_json::{json, Value};
use std::collections::HashMap;

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
    Finish {
        reason: Option<String>,
        usage: Option<Value>,
    },
}

#[derive(Debug, Default)]
pub struct StreamKernel {
    route: Option<ConversionRoute>,
    source: SourceStreamState,
    target: TargetStreamState,
    buffer: String,
    utf8_remainder: Vec<u8>,
}

impl StreamKernel {
    pub fn new(route: ConversionRoute) -> Self {
        Self {
            route: Some(route),
            ..Default::default()
        }
    }

    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, chunk);
        let mut out = Vec::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            out.extend(self.convert_block(&block));
        }
        out
    }

    pub fn finish(&mut self) -> Vec<Vec<u8>> {
        if self.buffer.trim().is_empty() {
            return self.target.finish(self.target_protocol());
        }
        let tail = std::mem::take(&mut self.buffer);
        let mut out = self.convert_block(&tail);
        out.extend(self.target.finish(self.target_protocol()));
        out
    }

    fn convert_block(&mut self, block: &str) -> Vec<Vec<u8>> {
        let parsed = parse_sse_block(block);
        if parsed.data.trim().is_empty() {
            return Vec::new();
        }
        if parsed.data.trim() == "[DONE]" {
            return self.target.finish(self.target_protocol());
        }
        let Ok(value) = serde_json::from_str::<Value>(&parsed.data) else {
            return Vec::new();
        };
        let source = self.source_protocol();
        let target = self.target_protocol();
        let events = self.source.parse(source, parsed.event.as_deref(), value);
        events
            .into_iter()
            .flat_map(|event| self.target.write(target, event))
            .collect()
    }

    fn source_protocol(&self) -> AiProtocol {
        self.route.expect("route must be set").source
    }

    fn target_protocol(&self) -> AiProtocol {
        self.route.expect("route must be set").target
    }
}

#[derive(Debug, Default)]
struct SourceStreamState {
    chat_tool_names: HashMap<usize, String>,
    chat_tool_ids: HashMap<usize, String>,
    anthropic_tool_by_block: HashMap<usize, SourceToolState>,
    responses_tool_by_item: HashMap<String, SourceToolState>,
    gemini_accumulated_text: String,
    gemini_accumulated_reasoning: String,
}

#[derive(Debug, Clone, Default)]
struct SourceToolState {
    index: usize,
    id: String,
    tool_type: String,
    name: String,
    arguments: String,
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
        for choice in value
            .get("choices")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let delta = choice.get("delta").unwrap_or(&Value::Null);
            if delta.get("role").and_then(Value::as_str) == Some("assistant") {
                out.push(UnifiedStreamEvent::Start {
                    id: id.clone(),
                    model: model.clone(),
                });
            }
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    out.push(UnifiedStreamEvent::TextDelta(text.to_string()));
                }
            }
            if let Some(reasoning) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(Value::as_str)
            {
                if !reasoning.is_empty() {
                    out.push(UnifiedStreamEvent::ReasoningDelta(reasoning.to_string()));
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
                for tool_call in tool_calls {
                    let index =
                        tool_call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
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
                        out.push(UnifiedStreamEvent::ToolCall {
                            index,
                            id: self
                                .chat_tool_ids
                                .get(&index)
                                .cloned()
                                .unwrap_or_else(|| format!("call_{index}")),
                            tool_type: TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string(),
                            name: self
                                .chat_tool_names
                                .get(&index)
                                .cloned()
                                .unwrap_or_default(),
                            arguments: input.to_string(),
                        });
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
                    out.push(UnifiedStreamEvent::ToolCall {
                        index,
                        id: self
                            .chat_tool_ids
                            .get(&index)
                            .cloned()
                            .unwrap_or_else(|| format!("call_{index}")),
                        name: self
                            .chat_tool_names
                            .get(&index)
                            .cloned()
                            .unwrap_or_default(),
                        tool_type: TOOL_TYPE_FUNCTION.to_string(),
                        arguments: arguments.to_string(),
                    });
                }
            }
            if let Some(function_call) = delta.get("function_call") {
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
                if !self
                    .chat_tool_names
                    .get(&index)
                    .map(String::is_empty)
                    .unwrap_or(true)
                    || !arguments.is_empty()
                {
                    out.push(UnifiedStreamEvent::ToolCall {
                        index,
                        id: self
                            .chat_tool_ids
                            .get(&index)
                            .cloned()
                            .unwrap_or_else(|| format!("call_{index}")),
                        tool_type: TOOL_TYPE_FUNCTION.to_string(),
                        name: self
                            .chat_tool_names
                            .get(&index)
                            .cloned()
                            .unwrap_or_default(),
                        arguments: arguments.to_string(),
                    });
                }
            }
            if choice
                .get("finish_reason")
                .is_some_and(|value| !value.is_null())
            {
                let finish_reason = choice
                    .get("finish_reason")
                    .and_then(Value::as_str)
                    .map(|reason| {
                        if reason == "function_call" {
                            "tool_calls"
                        } else {
                            reason
                        }
                    })
                    .map(ToString::to_string);
                out.push(UnifiedStreamEvent::Finish {
                    reason: finish_reason,
                    usage: value.get("usage").cloned(),
                });
            }
        }
        out
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
                .map(|text| vec![UnifiedStreamEvent::TextDelta(text.to_string())])
                .unwrap_or_default(),
            "response.reasoning_summary_text.delta" => value
                .get("delta")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(|text| vec![UnifiedStreamEvent::ReasoningDelta(text.to_string())])
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
                };
                let event = (state.tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL).then(|| {
                    UnifiedStreamEvent::ToolCall {
                        index: state.index,
                        id: state.id.clone(),
                        tool_type: state.tool_type.clone(),
                        name: state.name.clone(),
                        arguments: String::new(),
                    }
                });
                self.responses_tool_by_item.insert(key, state);
                event.into_iter().collect()
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
                let state = self
                    .responses_tool_by_item
                    .entry(key.clone())
                    .or_insert_with(|| SourceToolState {
                        index: value
                            .get("output_index")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as usize,
                        id: key,
                        tool_type: if event_name.unwrap_or_default()
                            == "response.custom_tool_call_input.delta"
                        {
                            TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string()
                        } else {
                            TOOL_TYPE_FUNCTION.to_string()
                        },
                        ..Default::default()
                    });
                state.arguments.push_str(&delta);
                vec![UnifiedStreamEvent::ToolCall {
                    index: state.index,
                    id: state.id.clone(),
                    tool_type: state.tool_type.clone(),
                    name: state.name.clone(),
                    arguments: delta,
                }]
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
                    {
                        state.arguments = arguments.to_string();
                    }
                }
                Vec::new()
            }
            "response.output_item.done" => {
                let item = value.get("item").unwrap_or(&value);
                if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    return item
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .filter(|signature| !signature.is_empty())
                        .map(|signature| {
                            vec![UnifiedStreamEvent::ReasoningSignature {
                                signature: encode_signature(
                                    SignatureProvider::OpenAiResponses,
                                    signature,
                                ),
                            }]
                        })
                        .unwrap_or_default();
                }
                Vec::new()
            }
            "response.completed" => {
                let response = value.get("response").unwrap_or(&value);
                let has_tool_call = !self.responses_tool_by_item.is_empty()
                    || response
                        .get("output")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items.iter().any(|item| {
                                matches!(
                                    item.get("type").and_then(Value::as_str),
                                    Some("function_call") | Some("custom_tool_call")
                                )
                            })
                        })
                        .unwrap_or(false);
                vec![UnifiedStreamEvent::Finish {
                    reason: response
                        .get("status")
                        .and_then(Value::as_str)
                        .map(|status| {
                            if status == "incomplete" {
                                "length"
                            } else if has_tool_call {
                                "tool_calls"
                            } else {
                                "stop"
                            }
                        })
                        .map(ToString::to_string),
                    usage: response.get("usage").cloned(),
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
            "message_delta" => vec![UnifiedStreamEvent::Finish {
                reason: value
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                    .map(|reason| match reason {
                        "max_tokens" => "length",
                        "tool_use" => "tool_calls",
                        _ => "stop",
                    })
                    .map(ToString::to_string),
                usage: value.get("usage").cloned(),
            }],
            "message_stop" => Vec::new(),
            _ => Vec::new(),
        }
    }

    fn parse_gemini(&mut self, value: Value) -> Vec<UnifiedStreamEvent> {
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
                    let delta = if visible_text.starts_with(&self.gemini_accumulated_text) {
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
                    let delta = if reasoning_text.starts_with(&self.gemini_accumulated_reasoning) {
                        reasoning_text[self.gemini_accumulated_reasoning.len()..].to_string()
                    } else {
                        reasoning_text.clone()
                    };
                    if !delta.is_empty() {
                        out.push(UnifiedStreamEvent::ReasoningDelta(delta));
                    }
                    self.gemini_accumulated_reasoning = reasoning_text;
                }
                for (index, part) in parts.iter().enumerate() {
                    let Some(function_call) = part.get("functionCall") else {
                        continue;
                    };
                    if let Some(signature) = gemini_part_thought_signature(part) {
                        out.push(UnifiedStreamEvent::ToolCallSignature {
                            index,
                            signature: encode_signature(SignatureProvider::Gemini, signature),
                        });
                    }
                    let id = function_call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("gemini_synth_{index}"));
                    let args = function_call
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    out.push(UnifiedStreamEvent::ToolCall {
                        index,
                        id,
                        tool_type: TOOL_TYPE_FUNCTION.to_string(),
                        name: function_call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: serde_json::to_string(&args).unwrap_or_default(),
                    });
                }
            }
            if candidate.get("finishReason").is_some() {
                out.push(UnifiedStreamEvent::Finish {
                    reason: candidate
                        .get("finishReason")
                        .and_then(Value::as_str)
                        .map(|reason| {
                            if reason == "MAX_TOKENS" {
                                "length"
                            } else {
                                "stop"
                            }
                        })
                        .map(ToString::to_string),
                    usage: value.get("usageMetadata").cloned(),
                });
            }
        }
        out
    }
}

fn gemini_part_thought_signature(part: &Value) -> Option<&str> {
    part.get("thoughtSignature")
        .or_else(|| part.get("thought_signature"))
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
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
    seen_response_tools: HashMap<usize, TargetResponseToolState>,
    responses_reasoning_started: bool,
    responses_reasoning_done: bool,
    responses_reasoning_summary: String,
    pending_responses_encrypted_content: Option<String>,
    pending_gemini_reasoning_signature: Option<String>,
    pending_gemini_tool_signatures: HashMap<usize, String>,
    gemini_seen_reasoning: bool,
    gemini_seen_tool: bool,
    gemini_emitted_signature: bool,
    emitted_gemini_finish: bool,
}

#[derive(Debug, Clone, Default)]
struct TargetAnthropicToolState {
    block_index: usize,
}

#[derive(Debug, Clone, Default)]
struct TargetResponseToolState {
    id: String,
    tool_type: String,
    name: String,
    arguments: String,
}

impl TargetStreamState {
    fn write(&mut self, target: AiProtocol, event: UnifiedStreamEvent) -> Vec<Vec<u8>> {
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
        self.write(
            target,
            UnifiedStreamEvent::Finish {
                reason: Some("stop".to_string()),
                usage: None,
            },
        )
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
                    "usage": {"input_tokens": 0, "output_tokens": 0}
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

    fn ensure_responses_start(&mut self) -> Option<Vec<u8>> {
        if self.sent_start {
            return None;
        }
        self.remember_start(String::new(), String::new());
        Some(sse_event(
            Some("response.created"),
            &json!({
                "type": "response.created",
                "response": {
                    "id": self.id,
                    "object": "response",
                    "status": "in_progress",
                    "model": self.model,
                    "output": []
                }
            }),
        ))
    }

    fn ensure_responses_reasoning_item(&mut self, out: &mut Vec<Vec<u8>>) {
        if let Some(start) = self.ensure_responses_start() {
            out.push(start);
        }
        if self.responses_reasoning_started {
            return;
        }
        self.responses_reasoning_started = true;
        out.push(sse_event(
            Some("response.output_item.added"),
            &json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "id": "reasoning_0",
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
            "id": "reasoning_0",
            "type": "reasoning",
            "status": "completed",
            "summary": summary
        });
        if let Some(encrypted_content) = self.pending_responses_encrypted_content.take() {
            item["encrypted_content"] = json!(encrypted_content);
        }
        out.push(sse_event(
            Some("response.output_item.done"),
            &json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": item
            }),
        ));
        self.responses_reasoning_done = true;
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
                                "usage": {"input_tokens": 0, "output_tokens": 0}
                            }
                        }),
                    ));
                }
            }
            UnifiedStreamEvent::TextDelta(text) => {
                if let Some(start) = self.ensure_anthropic_start() {
                    out.push(start);
                }
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
            UnifiedStreamEvent::ToolCallSignature { .. } => {}
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
                self.flush_pending_anthropic_signature_block(&mut out);
                if !self.open_anthropic_tools.contains_key(&index) {
                    let block_index = self.next_anthropic_index;
                    self.next_anthropic_index += 1;
                    self.open_anthropic_tools
                        .insert(index, TargetAnthropicToolState { block_index });
                    out.push(sse_event(
                        Some("content_block_start"),
                        &json!({
                            "type": "content_block_start",
                            "index": block_index,
                            "content_block": {"type": "tool_use", "id": id, "name": name}
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
            UnifiedStreamEvent::Finish { reason, usage } => {
                if self.finished {
                    return Vec::new();
                }
                if let Some(start) = self.ensure_anthropic_start() {
                    out.push(start);
                }
                self.finished = true;
                self.close_anthropic_reasoning_block(&mut out);
                self.close_anthropic_text_block(&mut out);
                let mut tool_blocks = self
                    .open_anthropic_tools
                    .values()
                    .map(|state| state.block_index)
                    .collect::<Vec<_>>();
                tool_blocks.sort_unstable();
                for block_index in tool_blocks {
                    out.push(sse_event(
                        Some("content_block_stop"),
                        &json!({"type": "content_block_stop", "index": block_index}),
                    ));
                }
                self.open_anthropic_tools.clear();
                self.flush_pending_anthropic_signature_block(&mut out);
                out.push(sse_event(
                    Some("message_delta"),
                    &json!({
                        "type": "message_delta",
                        "delta": {
                            "stop_reason": match reason.as_deref() {
                                Some("length") => "max_tokens",
                                Some("tool_calls") => "tool_use",
                                Some("refusal") => "refusal",
                                _ => "end_turn",
                            },
                            "stop_sequence": Value::Null
                        },
                        "usage": usage.unwrap_or_else(|| json!({"output_tokens": 0}))
                    }),
                ));
                out.push(sse_event(
                    Some("message_stop"),
                    &json!({"type": "message_stop"}),
                ));
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
            | UnifiedStreamEvent::ToolCallSignature { .. } => Vec::new(),
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
                let mut function = json!({"arguments": arguments});
                if !name.is_empty() {
                    function["name"] = json!(name);
                }
                out.push(self.chat_chunk(
                    json!({
                        "tool_calls": [{
                            "index": index,
                            "id": id,
                            "type": "function",
                            "function": function
                        }]
                    }),
                    None,
                ));
                out
            }
            UnifiedStreamEvent::Finish { reason, .. } => {
                if self.finished {
                    return Vec::new();
                }
                let mut out = self.ensure_chat_start();
                self.finished = true;
                out.push(self.chat_chunk(
                    json!({}),
                    Some(match reason.as_deref() {
                        Some("length") => "length",
                        Some("tool_calls") => "tool_calls",
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
                vec![sse_event(
                    Some("response.created"),
                    &json!({
                        "type": "response.created",
                        "response": {
                            "id": self.id,
                            "object": "response",
                            "status": "in_progress",
                            "model": self.model,
                            "output": []
                        }
                    }),
                )]
            }
            UnifiedStreamEvent::TextDelta(text) => {
                let mut out = Vec::new();
                if let Some(start) = self.ensure_responses_start() {
                    out.push(start);
                }
                out.push(sse_event(
                    Some("response.output_text.delta"),
                    &json!({
                        "type": "response.output_text.delta",
                        "delta": text,
                        "item_id": self.id,
                        "output_index": 0,
                        "content_index": 0
                    }),
                ));
                out
            }
            UnifiedStreamEvent::ReasoningDelta(text) => {
                let mut out = Vec::new();
                self.ensure_responses_reasoning_item(&mut out);
                self.responses_reasoning_summary.push_str(&text);
                out.push(sse_event(
                    Some("response.reasoning_summary_text.delta"),
                    &json!({
                        "type": "response.reasoning_summary_text.delta",
                        "delta": text,
                        "item_id": "reasoning_0",
                        "output_index": 0,
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
            UnifiedStreamEvent::ToolCallSignature { .. } => Vec::new(),
            UnifiedStreamEvent::ToolCall {
                index,
                id,
                tool_type,
                name,
                arguments,
            } => {
                let mut out = Vec::new();
                if let Some(start) = self.ensure_responses_start() {
                    out.push(start);
                }
                if !self.seen_response_tools.contains_key(&index) {
                    self.seen_response_tools.insert(
                        index,
                        TargetResponseToolState {
                            id: id.clone(),
                            tool_type: tool_type.clone(),
                            name: name.clone(),
                            arguments: String::new(),
                        },
                    );
                    let item = if tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                        json!({
                            "type": "custom_tool_call",
                            "status": "in_progress",
                            "call_id": id.clone(),
                            "name": name.clone(),
                            "input": ""
                        })
                    } else {
                        json!({
                            "id": id.clone(),
                            "type": "function_call",
                            "status": "in_progress",
                            "call_id": id.clone(),
                            "name": name.clone()
                        })
                    };
                    out.push(sse_event(
                        Some("response.output_item.added"),
                        &json!({
                            "type": "response.output_item.added",
                            "output_index": index,
                            "item": item
                        }),
                    ));
                }
                if let Some(state) = self.seen_response_tools.get_mut(&index) {
                    if !id.is_empty() {
                        state.id = id.clone();
                    }
                    if !name.is_empty() {
                        state.name = name.clone();
                    }
                    state.arguments.push_str(&arguments);
                }
                if !arguments.is_empty() {
                    if tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                        out.push(sse_event(
                            Some("response.custom_tool_call_input.delta"),
                            &json!({
                                "type": "response.custom_tool_call_input.delta",
                                "item_id": id,
                                "output_index": index,
                                "delta": arguments
                            }),
                        ));
                    } else {
                        out.push(sse_event(
                            Some("response.function_call_arguments.delta"),
                            &json!({
                                "type": "response.function_call_arguments.delta",
                                "item_id": id,
                                "output_index": index,
                                "delta": arguments
                            }),
                        ));
                    }
                }
                out
            }
            UnifiedStreamEvent::Finish { reason, usage } => {
                if self.finished {
                    return Vec::new();
                }
                let mut out = Vec::new();
                if let Some(start) = self.ensure_responses_start() {
                    out.push(start);
                }
                self.finished = true;
                self.finish_responses_reasoning_item(&mut out);
                for (index, tool) in self.seen_response_tools.clone() {
                    let tool_id = tool.id;
                    let tool_type = tool.tool_type;
                    let tool_name = tool.name;
                    let tool_arguments = tool.arguments;
                    if tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                        out.push(sse_event(
                            Some("response.custom_tool_call_input.done"),
                            &json!({
                                "type": "response.custom_tool_call_input.done",
                                "item_id": tool_id.clone(),
                                "output_index": index,
                                "input": tool_arguments.clone()
                            }),
                        ));
                        out.push(sse_event(
                            Some("response.output_item.done"),
                            &json!({
                                "type": "response.output_item.done",
                                "output_index": index,
                                "item": {
                                    "type": "custom_tool_call",
                                    "status": "completed",
                                    "call_id": tool_id,
                                    "name": tool_name,
                                    "input": tool_arguments
                                }
                            }),
                        ));
                    } else {
                        out.push(sse_event(
                            Some("response.function_call_arguments.done"),
                            &json!({
                                "type": "response.function_call_arguments.done",
                                "item_id": tool_id.clone(),
                                "output_index": index,
                                "arguments": tool_arguments.clone()
                            }),
                        ));
                        out.push(sse_event(
                            Some("response.output_item.done"),
                            &json!({
                                "type": "response.output_item.done",
                                "output_index": index,
                                "item": {
                                    "id": tool_id.clone(),
                                    "type": "function_call",
                                    "status": "completed",
                                    "call_id": tool_id,
                                    "name": tool_name,
                                    "arguments": tool_arguments
                                }
                            }),
                        ));
                    }
                }
                out.push(sse_event(
                    Some("response.completed"),
                    &json!({
                        "type": "response.completed",
                        "response": {
                            "id": self.id,
                            "object": "response",
                            "status": if reason.as_deref() == Some("length") { "incomplete" } else { "completed" },
                            "model": self.model,
                            "output": [],
                            "usage": usage
                        }
                    }),
                ));
                out
            }
        }
    }

    fn write_gemini(&mut self, event: UnifiedStreamEvent) -> Vec<Vec<u8>> {
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
            UnifiedStreamEvent::ToolCall {
                index,
                id,
                name,
                arguments,
                ..
            } => {
                self.gemini_seen_tool = true;
                let mut part = json!({
                    "functionCall": {
                        "id": id,
                        "name": name,
                        "args": serde_json::from_str::<Value>(&arguments).unwrap_or_else(|_| json!({}))
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
                vec![self.gemini_chunk(vec![part], None, None)]
            }
            UnifiedStreamEvent::Finish { reason, usage } => {
                if self.emitted_gemini_finish {
                    return Vec::new();
                }
                self.emitted_gemini_finish = true;
                let mut out = Vec::new();
                if self.gemini_seen_reasoning
                    && !self.gemini_seen_tool
                    && !self.gemini_emitted_signature
                {
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
                out.push(self.gemini_chunk(
                    Vec::new(),
                    Some(if reason.as_deref() == Some("length") {
                        "MAX_TOKENS"
                    } else {
                        "STOP"
                    }),
                    usage,
                ));
                out
            }
        }
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
