use super::super::error::ProtocolConversionError;
use super::super::llm::{
    ApiFormat, Choice, Function, FunctionCall, ImageUrl, Message, MessageContent,
    MessageContentPart, Request, RequestType, Response, Tool, ToolCall, Usage, TOOL_TYPE_FUNCTION,
};
use super::super::shared::signature::{decode_signature_for, encode_signature, SignatureProvider};
use super::super::shared::{
    content_text, extract_error_message, extract_error_type, json_string, message_parts,
    stop_from_value, tool_choice_from_anthropic,
};
use super::super::traits::InboundTransformer;
use super::super::types::AiProtocol;
use serde_json::{json, Value};
use std::collections::HashMap;

pub(crate) const ANTHROPIC_MESSAGE_INDEX_KEY: &str = "anthropic_message_index";
const ANTHROPIC_BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header:";

pub struct AnthropicInbound;

impl InboundTransformer for AnthropicInbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::AnthropicMessages
    }

    fn request_to_llm(&self, body: Value) -> Result<Request, ProtocolConversionError> {
        Ok(anthropic_request_to_llm(body))
    }

    fn response_from_llm(&self, response: Response) -> Result<Value, ProtocolConversionError> {
        Ok(llm_response_to_anthropic(response))
    }

    fn error_from_llm(&self, error: Value) -> Value {
        let message = extract_error_message(&error)
            .unwrap_or_else(|| "Protocol conversion error".to_string());
        json!({
            "type": "error",
            "error": {
                "type": extract_error_type(&error).unwrap_or_else(|| "api_error".to_string()),
                "message": message
            }
        })
    }
}

pub fn anthropic_request_to_llm(body: Value) -> Request {
    let mut request = Request {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        max_tokens: body.get("max_tokens").and_then(Value::as_i64),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        stream: body.get("stream").and_then(Value::as_bool),
        stop: stop_from_value(body.get("stop_sequences")),
        tool_choice: tool_choice_from_anthropic(body.get("tool_choice")),
        reasoning_effort: anthropic_reasoning_effort(&body).map(ToString::to_string),
        request_type: Some(RequestType::Chat),
        api_format: Some(ApiFormat::AnthropicMessages),
        ..Default::default()
    };

    if let Some(user_id) = body
        .pointer("/metadata/user_id")
        .and_then(Value::as_str)
        .filter(|user_id| !user_id.is_empty())
    {
        request
            .metadata
            .insert("user_id".to_string(), user_id.to_string());
    }

    if let Some(system) = body.get("system") {
        for text in anthropic_system_texts(system) {
            request.messages.push(Message {
                role: "system".to_string(),
                content: MessageContent::Text(text),
                ..Default::default()
            });
        }
    }

    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        for (message_index, message) in messages.iter().enumerate() {
            append_anthropic_message_to_llm(message, message_index, &mut request.messages);
        }
    }

    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        request.tools = tools
            .iter()
            .filter_map(|tool| {
                if tool.get("type").and_then(Value::as_str) == Some("BatchTool") {
                    return None;
                }
                let name = tool.get("name").and_then(Value::as_str)?;
                Some(Tool {
                    tool_type: TOOL_TYPE_FUNCTION.to_string(),
                    function: Some(Function {
                        name: name.to_string(),
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        parameters: tool.get("input_schema").cloned(),
                        ..Default::default()
                    }),
                    cache_control: tool.get("cache_control").cloned(),
                    ..Default::default()
                })
            })
            .collect();
    }

    request
}

fn anthropic_system_texts(system: &Value) -> Vec<String> {
    match system {
        Value::String(text) => strip_leading_anthropic_billing_header(text)
            .filter(|text| !text.is_empty())
            .map(|text| vec![text.to_string()])
            .unwrap_or_default(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
            })
            .filter_map(strip_leading_anthropic_billing_header)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
            .collect(),
        other => other
            .as_str()
            .and_then(strip_leading_anthropic_billing_header)
            .filter(|text| !text.is_empty())
            .map(|text| vec![text.to_string()])
            .unwrap_or_default(),
    }
}

fn strip_leading_anthropic_billing_header(text: &str) -> Option<&str> {
    if !text.starts_with(ANTHROPIC_BILLING_HEADER_PREFIX) {
        return Some(text);
    }

    let bytes = text.as_bytes();
    let line_end = bytes
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')?;
    let mut rest_start = line_end + 1;
    if bytes[line_end] == b'\r' && bytes.get(line_end + 1) == Some(&b'\n') {
        rest_start += 1;
    }

    let rest = &text[rest_start..];
    Some(
        rest.strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .or_else(|| rest.strip_prefix('\r'))
            .unwrap_or(rest),
    )
}

fn anthropic_reasoning_effort(body: &Value) -> Option<&'static str> {
    if let Some(effort) = body
        .pointer("/output_config/effort")
        .and_then(Value::as_str)
    {
        return match effort {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "max" => Some("xhigh"),
            _ => None,
        };
    }

    let thinking = body.get("thinking")?;
    match thinking.get("type").and_then(Value::as_str) {
        Some("adaptive") => Some("xhigh"),
        Some("enabled") => {
            let budget = thinking.get("budget_tokens").and_then(Value::as_u64);
            match budget {
                Some(value) if value < 4_000 => Some("low"),
                Some(value) if value < 16_000 => Some("medium"),
                Some(_) | None => Some("high"),
            }
        }
        _ => None,
    }
}

fn append_anthropic_message_to_llm(message: &Value, message_index: usize, out: &mut Vec<Message>) {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_string();
    let parts = message_parts(message.get("content"));
    let mut llm_parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut reasoning_content = None;
    let mut reasoning = None;
    let mut reasoning_signature = None;
    let mut redacted_reasoning_content = None;

    for (index, part) in parts.iter().enumerate() {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        llm_parts.push(MessageContentPart {
                            part_type: "text".to_string(),
                            text: Some(text.to_string()),
                            cache_control: part.get("cache_control").cloned(),
                            ..Default::default()
                        });
                    }
                }
            }
            Some("thinking") => {
                if let Some(thinking) = part
                    .get("thinking")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    reasoning_content = Some(thinking.to_string());
                    reasoning = Some(thinking.to_string());
                }
                reasoning_signature = part
                    .get("signature")
                    .and_then(Value::as_str)
                    .filter(|signature| !signature.is_empty())
                    .map(|signature| encode_signature(SignatureProvider::Anthropic, signature));
            }
            Some("redacted_thinking") => {
                redacted_reasoning_content = part
                    .get("data")
                    .and_then(Value::as_str)
                    .filter(|data| !data.is_empty())
                    .map(ToString::to_string);
            }
            Some("image") => {
                if let Some(source) = part.get("source") {
                    let image_url = source
                        .get("url")
                        .and_then(Value::as_str)
                        .filter(|url| !url.is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| {
                            let media_type = source
                                .get("media_type")
                                .and_then(Value::as_str)
                                .unwrap_or("application/octet-stream");
                            let data = source
                                .get("data")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            format!("data:{media_type};base64,{data}")
                        });
                    llm_parts.push(MessageContentPart {
                        part_type: "image_url".to_string(),
                        image_url: Some(ImageUrl {
                            url: image_url,
                            detail: None,
                        }),
                        cache_control: part.get("cache_control").cloned(),
                        ..Default::default()
                    });
                }
            }
            Some("tool_use") => {
                tool_calls.push(ToolCall {
                    id: part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    tool_type: TOOL_TYPE_FUNCTION.to_string(),
                    function: FunctionCall {
                        name: part
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: json_string(part.get("input").unwrap_or(&json!({}))),
                    },
                    index,
                    cache_control: part.get("cache_control").cloned(),
                    ..Default::default()
                });
            }
            Some("tool_result") => {
                out.push(Message {
                    role: "tool".to_string(),
                    tool_call_id: part
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    tool_call_is_error: part.get("is_error").and_then(Value::as_bool),
                    content: anthropic_tool_result_content(part.get("content")),
                    cache_control: part.get("cache_control").cloned(),
                    transformer_metadata: anthropic_message_metadata(message_index),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }

    let mut result = Message {
        role,
        content: MessageContent::Parts(llm_parts),
        tool_calls,
        reasoning_content,
        reasoning,
        reasoning_signature,
        redacted_reasoning_content,
        transformer_metadata: anthropic_message_metadata(message_index),
        ..Default::default()
    };
    if result.content.is_empty() && !result.tool_calls.is_empty() {
        result.content = MessageContent::Text(String::new());
    }
    if !result.content.is_empty()
        || !result.tool_calls.is_empty()
        || result.reasoning_content.is_some()
        || result.reasoning_signature.is_some()
        || result.redacted_reasoning_content.is_some()
    {
        out.push(result);
    }
}

fn anthropic_message_metadata(message_index: usize) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    metadata.insert(
        ANTHROPIC_MESSAGE_INDEX_KEY.to_string(),
        json!(message_index),
    );
    metadata
}

fn anthropic_tool_result_content(content: Option<&Value>) -> MessageContent {
    match content {
        Some(Value::Array(parts)) => {
            let converted = parts
                .iter()
                .filter_map(anthropic_content_part_to_llm)
                .collect::<Vec<_>>();
            if converted.len() == 1
                && converted[0].part_type == "text"
                && converted[0].cache_control.is_none()
            {
                return MessageContent::Text(converted[0].text.clone().unwrap_or_default());
            }
            MessageContent::Parts(converted)
        }
        Some(Value::String(text)) => MessageContent::Text(text.clone()),
        Some(value) => MessageContent::Text(content_text(Some(value))),
        None => MessageContent::Text(String::new()),
    }
}

fn anthropic_content_part_to_llm(part: &Value) -> Option<MessageContentPart> {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => Some(MessageContentPart {
            part_type: "text".to_string(),
            text: part
                .get("text")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            cache_control: part.get("cache_control").cloned(),
            ..Default::default()
        }),
        Some("image") => {
            let source = part.get("source")?;
            let image_url = source
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    let media_type = source
                        .get("media_type")
                        .and_then(Value::as_str)
                        .unwrap_or("application/octet-stream");
                    let data = source
                        .get("data")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    format!("data:{media_type};base64,{data}")
                });
            Some(MessageContentPart {
                part_type: "image_url".to_string(),
                image_url: Some(ImageUrl {
                    url: image_url,
                    detail: None,
                }),
                cache_control: part.get("cache_control").cloned(),
                ..Default::default()
            })
        }
        _ => None,
    }
}

pub fn llm_response_to_anthropic(response: Response) -> Value {
    let choice = response.choices.first();
    let message = choice.map(|choice| &choice.message);
    let mut content = Vec::new();
    if let Some(message) = message {
        if let Some(reasoning) = message
            .reasoning_content
            .as_deref()
            .or(message.reasoning.as_deref())
        {
            if !reasoning.is_empty() {
                let mut thinking = json!({ "type": "thinking", "thinking": reasoning });
                if let Some(signature) =
                    message
                        .reasoning_signature
                        .as_deref()
                        .and_then(|signature| {
                            decode_signature_for(SignatureProvider::Anthropic, signature)
                        })
                {
                    thinking["signature"] = json!(signature);
                }
                content.push(thinking);
            }
        }
        if let Some(redacted) = message.redacted_reasoning_content.as_deref() {
            if !redacted.is_empty() {
                content.push(json!({ "type": "redacted_thinking", "data": redacted }));
            }
        }
        append_llm_content_as_anthropic(&message.content, &mut content);
        for tool_call in &message.tool_calls {
            content.push(json!({
                "type": "tool_use",
                "id": tool_call.id,
                "name": tool_call.function.name,
                "input": serde_json::from_str::<Value>(&tool_call.function.arguments).unwrap_or_else(|_| json!({}))
            }));
        }
    }

    json!({
        "id": response.id,
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": response.model,
        "stop_reason": choice
            .and_then(|choice| choice.finish_reason.as_deref())
            .map(openai_finish_to_anthropic_stop)
            .unwrap_or("end_turn"),
        "stop_sequence": Value::Null,
        "usage": usage_to_anthropic(response.usage.as_ref()),
    })
}

fn append_llm_content_as_anthropic(content: &MessageContent, out: &mut Vec<Value>) {
    match content {
        MessageContent::Text(text) => {
            if !text.is_empty() {
                out.push(json!({ "type": "text", "text": text }));
            }
        }
        MessageContent::Parts(parts) => {
            for part in parts {
                match part.part_type.as_str() {
                    "text" | "input_text" | "output_text" => {
                        if let Some(text) = &part.text {
                            if !text.is_empty() {
                                out.push(json!({ "type": "text", "text": text }));
                            }
                        }
                    }
                    "image_url" | "input_image" => {
                        if let Some(image) = &part.image_url {
                            if let Some((media_type, data)) = image
                                .url
                                .strip_prefix("data:")
                                .and_then(|rest| rest.split_once(";base64,"))
                            {
                                out.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": data
                                    }
                                }));
                            } else if !image.url.is_empty() {
                                out.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "url",
                                        "url": image.url
                                    }
                                }));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        MessageContent::Empty => {}
    }
}

fn usage_to_anthropic(usage: Option<&Usage>) -> Value {
    let usage = usage.cloned().unwrap_or_default();
    json!({
        "input_tokens": usage.prompt_tokens.saturating_sub(usage.cached_tokens),
        "output_tokens": usage.completion_tokens,
        "cache_read_input_tokens": usage.cached_tokens,
    })
}

fn openai_finish_to_anthropic_stop(reason: &str) -> &'static str {
    match reason {
        "length" | "max_tokens" => "max_tokens",
        "tool_calls" | "function_call" | "tool_use" => "tool_use",
        "refusal" => "refusal",
        _ => "end_turn",
    }
}

pub fn anthropic_response_to_llm(body: Value) -> Response {
    let mut message = Message {
        role: "assistant".to_string(),
        ..Default::default()
    };
    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();

    if let Some(content) = body.get("content").and_then(Value::as_array) {
        for (index, block) in content.iter().enumerate() {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        parts.push(MessageContentPart {
                            part_type: "text".to_string(),
                            text: Some(text.to_string()),
                            ..Default::default()
                        });
                    }
                }
                Some("thinking") => {
                    let reasoning = block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    message.reasoning_content = reasoning.clone();
                    message.reasoning = reasoning;
                    message.reasoning_signature = block
                        .get("signature")
                        .and_then(Value::as_str)
                        .filter(|signature| !signature.is_empty())
                        .map(|signature| encode_signature(SignatureProvider::Anthropic, signature));
                }
                Some("redacted_thinking") => {
                    message.redacted_reasoning_content = block
                        .get("data")
                        .and_then(Value::as_str)
                        .filter(|data| !data.is_empty())
                        .map(ToString::to_string);
                }
                Some("tool_use") => tool_calls.push(ToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    tool_type: TOOL_TYPE_FUNCTION.to_string(),
                    function: FunctionCall {
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: json_string(block.get("input").unwrap_or(&json!({}))),
                    },
                    index,
                    ..Default::default()
                }),
                _ => {}
            }
        }
    }
    message.content = MessageContent::Parts(parts);
    message.tool_calls = tool_calls;
    Response {
        id: body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        object: "chat.completion".to_string(),
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        choices: vec![Choice {
            index: 0,
            message,
            finish_reason: body
                .get("stop_reason")
                .and_then(Value::as_str)
                .map(anthropic_stop_to_openai_finish)
                .map(ToString::to_string),
            ..Default::default()
        }],
        usage: Some(anthropic_usage_to_llm(body.get("usage"))),
        ..Default::default()
    }
}

fn anthropic_usage_to_llm(usage: Option<&Value>) -> Usage {
    let usage = usage.unwrap_or(&Value::Null);
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
    Usage {
        prompt_tokens: input.saturating_add(cached).saturating_add(cache_creation),
        completion_tokens: output,
        total_tokens: input
            .saturating_add(cached)
            .saturating_add(cache_creation)
            .saturating_add(output),
        cached_tokens: cached,
        ..Default::default()
    }
}

fn anthropic_stop_to_openai_finish(reason: &str) -> &'static str {
    match reason {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    }
}
