use super::gemini::gemini_stream_error;
use super::kernel::ConversionContext;
use super::llm::{TOOL_TYPE_FUNCTION, TOOL_TYPE_RESPONSES_CUSTOM_TOOL};
use super::openai::codex_tools::{
    custom_tool_input_from_chat_arguments, is_custom_tool_chat_name,
    response_tool_added_item_from_chat_name, response_tool_done_item_from_chat_name,
    response_tool_item_id_from_chat_name, CodexToolContext,
};
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
    StreamError {
        code: String,
        message: String,
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
    #[allow(dead_code)]
    pub fn new(route: ConversionRoute) -> Self {
        Self::with_context(route, ConversionContext::default())
    }

    pub fn with_context(route: ConversionRoute, context: ConversionContext) -> Self {
        Self {
            route: Some(route),
            target: TargetStreamState::with_conversion_context(context),
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

    pub fn fail(&mut self, message: &str) -> Vec<Vec<u8>> {
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
        let parsed = parse_sse_block(block);
        let target = self.target_protocol();
        if parsed.data.trim().is_empty() {
            if parsed.event.as_deref() == Some("error") {
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
            return self.target.finish(target);
        }
        let Ok(value) = serde_json::from_str::<Value>(&parsed.data) else {
            return Vec::new();
        };
        if let Some((code, message)) = stream_error_from_value(parsed.event.as_deref(), &value) {
            return self
                .target
                .write(target, UnifiedStreamEvent::StreamError { code, message });
        }
        let source = self.source_protocol();
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
    pending_chat_finish_reason: Option<String>,
    pending_anthropic_usage: Option<Value>,
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
        let usage = value.get("usage").cloned();
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if choices.is_empty() {
            if let (Some(reason), Some(usage)) = (self.pending_chat_finish_reason.take(), usage) {
                out.push(UnifiedStreamEvent::Finish {
                    reason: Some(reason),
                    usage: Some(usage),
                });
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
            if let Some(finish_reason) = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.trim().is_empty())
                .map(|reason| {
                    if reason == "function_call" {
                        "tool_calls"
                    } else {
                        reason
                    }
                })
                .map(ToString::to_string)
            {
                if let Some(usage) = usage.clone() {
                    self.pending_chat_finish_reason = None;
                    out.push(UnifiedStreamEvent::Finish {
                        reason: Some(finish_reason),
                        usage: Some(usage),
                    });
                } else {
                    self.pending_chat_finish_reason = Some(finish_reason.clone());
                    out.push(UnifiedStreamEvent::Finish {
                        reason: Some(finish_reason),
                        usage: None,
                    });
                }
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
            "message_delta" => {
                if let Some(usage) = value.get("usage").cloned() {
                    self.pending_anthropic_usage = Some(usage);
                }
                value
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                    .filter(|reason| !reason.trim().is_empty())
                    .map(|reason| {
                        let mapped_reason = match reason {
                            "max_tokens" => "length",
                            "tool_use" => "tool_calls",
                            _ => "stop",
                        };
                        vec![UnifiedStreamEvent::Finish {
                            reason: Some(mapped_reason.to_string()),
                            usage: self.pending_anthropic_usage.take(),
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
                        usage: Some(usage),
                    }]
                })
                .unwrap_or_default(),
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
            if let Some(finish_reason) = candidate
                .get("finishReason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.trim().is_empty())
            {
                out.push(UnifiedStreamEvent::Finish {
                    reason: Some(
                        if finish_reason == "MAX_TOKENS" {
                            "length"
                        } else {
                            "stop"
                        }
                        .to_string(),
                    ),
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
    let prompt_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    json!({
        "input_tokens": prompt_tokens.saturating_sub(cached_tokens),
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": cached_tokens,
        "output_tokens": output_tokens
    })
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
    let reasoning_tokens = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .or_else(|| usage.pointer("/output_tokens_details/reasoning_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens,
        "input_tokens_details": {
            "cached_tokens": cached_tokens
        },
        "output_tokens_details": {
            "reasoning_tokens": reasoning_tokens
        }
    })
}

fn anthropic_stop_reason(reason: &str) -> &'static str {
    match reason {
        "length" | "max_tokens" => "max_tokens",
        "tool_calls" | "function_call" | "tool_use" => "tool_use",
        "refusal" => "refusal",
        _ => "end_turn",
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
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str))
        .or_else(|| value.get("code").and_then(Value::as_str))
        .filter(|code| !code.is_empty() && *code != "error")
        .unwrap_or("stream_error")
        .to_string();

    Some((code, message))
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
    pending_gemini_reasoning_signature: Option<String>,
    pending_gemini_tool_signatures: HashMap<usize, String>,
    pending_gemini_tools: HashMap<usize, TargetGeminiToolState>,
    gemini_seen_reasoning: bool,
    gemini_seen_tool: bool,
    gemini_emitted_signature: bool,
    emitted_gemini_finish: bool,
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
        self.write(
            target,
            UnifiedStreamEvent::Finish {
                reason: Some("stop".to_string()),
                usage: None,
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
            let response_item_id = tool.response_item_id.clone();
            let is_custom_tool = tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL
                || is_custom_tool_chat_name(&tool_name, self.codex_tool_context.as_ref());
            if is_custom_tool {
                let tool_input = custom_tool_input_from_chat_arguments(&tool_arguments);
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
                        "item": response_tool_done_item_from_chat_name(
                            &response_item_id,
                            "completed",
                            &tool_id,
                            &tool_name,
                            &tool_arguments,
                            self.codex_tool_context.as_ref()
                        )
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
                        "item": response_tool_done_item_from_chat_name(
                            &response_item_id,
                            "completed",
                            &tool_id,
                            &tool_name,
                            &tool_arguments,
                            self.codex_tool_context.as_ref()
                        )
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
        out.push(sse_event(
            Some("response.completed"),
            &json!({
                "type": "response.completed",
                "response": {
                    "id": self.id,
                    "object": "response",
                    "status": if finish_reason == "length" { "incomplete" } else { "completed" },
                    "model": self.model,
                    "output": self.completed_responses_output(),
                    "usage": chat_usage_to_responses(usage.as_ref())
                }
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
        response_tool_done_item_from_chat_name(
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
        )
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
        self.close_anthropic_tool_block(&mut out);
        self.open_anthropic_tools.clear();
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
                    out.push(sse_event(
                        Some("content_block_start"),
                        &json!({
                            "type": "content_block_start",
                            "index": block_index,
                            "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
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
            | UnifiedStreamEvent::StreamError { .. } => Vec::new(),
            UnifiedStreamEvent::ToolCall {
                index,
                id,
                tool_type,
                name,
                arguments,
            } => {
                let mut out = Vec::new();
                self.finish_responses_reasoning_item(&mut out);
                self.finish_responses_message_item(&mut out);
                out.extend(self.ensure_responses_start());
                if !self.seen_response_tools.contains_key(&index) {
                    let output_index = self.next_responses_output_index();
                    let response_item_id = response_tool_item_id_from_chat_name(
                        &id,
                        &name,
                        self.codex_tool_context.as_ref(),
                    );
                    let response_item = response_tool_added_item_from_chat_name(
                        &response_item_id,
                        "in_progress",
                        &id,
                        &name,
                        self.codex_tool_context.as_ref(),
                    );
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
                        let input = custom_tool_input_from_chat_arguments(&arguments);
                        out.push(sse_event(
                            Some("response.custom_tool_call_input.delta"),
                            &json!({
                                "type": "response.custom_tool_call_input.delta",
                                "item_id": item_id,
                                "output_index": output_index,
                                "delta": input
                            }),
                        ));
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
            UnifiedStreamEvent::Finish { reason, usage } => {
                self.finish_responses_response(reason, usage, false)
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
            UnifiedStreamEvent::StreamError { .. } => Vec::new(),
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
            UnifiedStreamEvent::Finish { reason, usage } => {
                if self.emitted_gemini_finish {
                    return Vec::new();
                }
                self.emitted_gemini_finish = true;
                let mut out = Vec::new();
                out.extend(self.flush_gemini_tool_calls(reason.as_deref() == Some("tool_calls")));
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
