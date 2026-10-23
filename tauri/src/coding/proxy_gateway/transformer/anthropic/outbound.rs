use super::super::error::ProtocolConversionError;
use super::super::llm::{Message, MessageContent, MessageContentPart, Request, ToolCall};
use super::super::llm::{
    TOOL_TYPE_ANTHROPIC_NATIVE, TOOL_TYPE_ANTHROPIC_WEB_SEARCH, TOOL_TYPE_FUNCTION,
};
use super::super::shared::signature::{decode_signature_for, SignatureProvider};
use super::super::shared::thinking_config::{anthropic_thinking_plan, AnthropicThinkingPlan};
use super::super::shared::tool_media::{
    inline_image_data, plan_tool_result_images, ToolImageScope, TOOL_RESULT_MEDIA_ATTACHED_MARKER,
};
use super::super::shared::{
    normalize_function_parameters_owned, stop_to_value, tool_arguments_value,
    tool_choice_to_anthropic,
};
use super::super::traits::OutboundTransformer;
use super::super::types::AiProtocol;
use super::inbound::{
    anthropic_request_to_llm, anthropic_response_to_llm, llm_response_to_anthropic,
    ANTHROPIC_ARRAY_INSTRUCTIONS_KEY, ANTHROPIC_MESSAGE_INDEX_KEY, ANTHROPIC_RAW_CONTENT_BLOCK_KEY,
    ANTHROPIC_RAW_TOOL_KEY,
};
use serde_json::{json, Value};

pub struct AnthropicOutbound;

impl OutboundTransformer for AnthropicOutbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::AnthropicMessages
    }

    fn request_from_llm(&self, request: Request) -> Result<Value, ProtocolConversionError> {
        Ok(llm_request_to_anthropic(request))
    }

    fn response_to_llm(
        &self,
        body: Value,
    ) -> Result<super::super::llm::Response, ProtocolConversionError> {
        Ok(anthropic_response_to_llm(body))
    }

    fn error_to_llm(&self, error: Value) -> Value {
        error
    }
}

pub fn llm_request_to_anthropic(request: Request) -> Value {
    let mut system_chunks = Vec::new();
    let mut messages = Vec::new();
    let preserve_system_array = request
        .transformer_metadata
        .get(ANTHROPIC_ARRAY_INSTRUCTIONS_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let request_messages = request.messages;
    let mut index = 0;
    while index < request_messages.len() {
        let message = &request_messages[index];
        // Every instruction message is merged into the top-level `system` field.
        // Anthropic Messages is never a conversion source here (same-protocol
        // requests pass through), so this path only serves Chat/Responses/Gemini
        // sources, whose instruction content is stable across turns.
        if message.role == "system" || message.role == "developer" {
            match &message.content {
                MessageContent::Text(text) if !text.is_empty() => system_chunks.push(text.clone()),
                MessageContent::Parts(parts) => {
                    for part in parts {
                        if let Some(text) = &part.text {
                            if !text.is_empty() {
                                system_chunks.push(text.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
            index += 1;
            continue;
        }

        if message.role == "tool" {
            let mut content = Vec::new();
            let group_index = anthropic_message_index(message);
            index = append_tool_result_group(&request_messages, index, &mut content);
            index = append_same_anthropic_user_content(
                &request_messages,
                index,
                group_index,
                &mut content,
            );
            messages.push(json!({
                "role": "user",
                "content": content
            }));
            continue;
        }

        let mut content = message_content_to_anthropic(&message.content);
        if let Some(reasoning) = message
            .reasoning_content
            .as_deref()
            .or(message.reasoning.as_deref())
        {
            content.insert(0, anthropic_thinking_block(message, reasoning));
        }
        if let Some(redacted) = message.redacted_reasoning_content.as_deref() {
            if !redacted.is_empty() {
                let insert_index = if content
                    .first()
                    .and_then(|part| part.get("type"))
                    .and_then(Value::as_str)
                    == Some("thinking")
                {
                    1
                } else {
                    0
                };
                content.insert(
                    insert_index,
                    json!({ "type": "redacted_thinking", "data": redacted }),
                );
            }
        }
        for tool_call in &message.tool_calls {
            content.push(tool_call_to_anthropic(tool_call));
        }
        messages.push(json!({
            "role": if message.role == "assistant" { "assistant" } else { "user" },
            "content": content
        }));

        index += 1;
        if message.role == "assistant" && !message.tool_calls.is_empty() {
            let mut tool_result_content = Vec::new();
            let tool_result_start = index;
            index = append_tool_result_group(&request_messages, index, &mut tool_result_content);
            if index > tool_result_start {
                let group_index = request_messages
                    .get(tool_result_start)
                    .and_then(anthropic_message_index);
                index = append_same_anthropic_user_content(
                    &request_messages,
                    index,
                    group_index,
                    &mut tool_result_content,
                );
                messages.push(json!({
                    "role": "user",
                    "content": tool_result_content
                }));
            }
        }
    }

    let resolved_max_tokens = request
        .max_tokens
        .or(request.max_completion_tokens)
        .unwrap_or(8192);
    let thinking_plan = request
        .reasoning_effort
        .as_deref()
        .and_then(|effort| anthropic_thinking_plan(&request.model, effort, resolved_max_tokens));

    let mut body = json!({
        "model": request.model,
        "messages": messages
    });
    if !system_chunks.is_empty() {
        body["system"] = if preserve_system_array {
            Value::Array(
                system_chunks
                    .into_iter()
                    .map(|text| json!({ "type": "text", "text": text }))
                    .collect(),
            )
        } else {
            json!(system_chunks.join("\n\n"))
        };
    }
    body["max_tokens"] = json!(resolved_max_tokens);
    if let Some(user_id) = request
        .metadata
        .get("user_id")
        .filter(|user_id| !user_id.is_empty())
    {
        body["metadata"] = json!({ "user_id": user_id });
    }
    apply_anthropic_thinking(&mut body, thinking_plan);
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(stream) = request.stream {
        body["stream"] = json!(stream);
    }
    if let Some(stop) = stop_to_value(request.stop) {
        body["stop_sequences"] = stop;
    }
    let tools = request
        .tools
        .into_iter()
        .filter_map(|tool| {
            if let Some(raw_tool) = tool.transformer_metadata.get(ANTHROPIC_RAW_TOOL_KEY) {
                return Some(raw_tool.clone());
            }
            if tool.tool_type == TOOL_TYPE_ANTHROPIC_WEB_SEARCH {
                let mut value = json!({
                    "type": "web_search_20250305",
                    "name": tool
                        .function
                        .as_ref()
                        .map(|function| function.name.as_str())
                        .filter(|name| !name.is_empty())
                        .unwrap_or("web_search")
                });
                if let Some(cache_control) = tool.cache_control {
                    value["cache_control"] = cache_control;
                }
                return Some(value);
            }
            if tool.tool_type == TOOL_TYPE_ANTHROPIC_NATIVE || tool.tool_type != TOOL_TYPE_FUNCTION
            {
                return None;
            }
            let function = tool.function?;
            if function.name.is_empty() {
                return None;
            }
            Some(json!({
                "name": function.name,
                "description": function.description,
                "input_schema": normalize_function_parameters_owned(function.parameters)
            }))
        })
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(tool_choice) = tool_choice_to_anthropic(request.tool_choice) {
        body["tool_choice"] = tool_choice;
    }
    body
}

/// Writes the thinking fields the model's generation actually accepts.
fn apply_anthropic_thinking(body: &mut Value, plan: Option<AnthropicThinkingPlan>) {
    match plan {
        Some(AnthropicThinkingPlan::Disabled) => {
            body["thinking"] = json!({ "type": "disabled" });
        }
        Some(AnthropicThinkingPlan::Budget { budget_tokens }) => {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget_tokens
            });
        }
        Some(AnthropicThinkingPlan::Adaptive { effort }) => {
            body["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
            body["output_config"]["effort"] = json!(effort);
        }
        Some(AnthropicThinkingPlan::EffortOnly { effort }) => {
            body["output_config"]["effort"] = json!(effort);
        }
        None => {}
    }
}

fn anthropic_thinking_block(message: &Message, reasoning: &str) -> Value {
    let mut block = json!({ "type": "thinking", "thinking": reasoning });
    if let Some(signature) = message
        .reasoning_signature
        .as_deref()
        .and_then(|signature| decode_signature_for(SignatureProvider::Anthropic, signature))
    {
        block["signature"] = json!(signature);
    }
    block
}

fn append_tool_result_group(
    messages: &[Message],
    mut index: usize,
    content: &mut Vec<Value>,
) -> usize {
    while let Some(message) = messages.get(index) {
        if message.role != "tool" {
            break;
        }
        content.push(tool_result_to_anthropic(message));
        index += 1;
    }
    index
}

fn append_same_anthropic_user_content(
    messages: &[Message],
    index: usize,
    group_index: Option<u64>,
    content: &mut Vec<Value>,
) -> usize {
    let Some(message) = messages.get(index) else {
        return index;
    };
    if message.role == "user"
        && group_index.is_some()
        && anthropic_message_index(message) == group_index
        && message.tool_calls.is_empty()
    {
        content.extend(message_content_to_anthropic(&message.content));
        return index + 1;
    }
    index
}

fn anthropic_message_index(message: &Message) -> Option<u64> {
    message
        .transformer_metadata
        .get(ANTHROPIC_MESSAGE_INDEX_KEY)
        .and_then(Value::as_u64)
}

fn tool_result_to_anthropic(message: &Message) -> Value {
    let mut result = json!({
        "type": "tool_result",
        "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
        "content": tool_result_content_to_anthropic(&message.content)
    });
    if let Some(is_error) = message.tool_call_is_error {
        result["is_error"] = json!(is_error);
    }
    if let Some(cache_control) = &message.cache_control {
        result["cache_control"] = cache_control.clone();
    }
    result
}

fn tool_result_content_to_anthropic(content: &MessageContent) -> Value {
    if let Some(plan) = plan_tool_result_images(
        content,
        ToolImageScope::AnyImage,
        TOOL_RESULT_MEDIA_ATTACHED_MARKER,
    ) {
        let mut blocks = Vec::new();
        append_cleaned_anthropic_tool_output(&plan.cleaned, &mut blocks);
        blocks.extend(plan.images.into_iter().filter_map(|image| {
            if let Some((media_type, data)) = inline_image_data(&image.url) {
                return Some(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data
                    }
                }));
            }
            Some(json!({
                "type": "image",
                "source": {
                    "type": "url",
                    "url": image.url
                }
            }))
        }));
        return Value::Array(blocks);
    }

    match content {
        MessageContent::Text(text) => Value::String(text.clone()),
        MessageContent::Parts(parts) => Value::Array(
            parts
                .iter()
                .filter_map(message_content_part_to_anthropic)
                .collect::<Vec<_>>(),
        ),
        MessageContent::Empty => Value::String(String::new()),
    }
}

fn append_cleaned_anthropic_tool_output(value: &Value, blocks: &mut Vec<Value>) {
    match value {
        Value::String(text) if !text.is_empty() => {
            blocks.push(json!({ "type": "text", "text": text }));
        }
        Value::Array(parts) => {
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text" | "input_text" | "output_text") => {
                        let Some(text) = part.get("text").and_then(Value::as_str) else {
                            continue;
                        };
                        let mut block = json!({ "type": "text", "text": text });
                        if let Some(cache_control) = part.get("cache_control") {
                            block["cache_control"] = cache_control.clone();
                        }
                        blocks.push(block);
                    }
                    _ => blocks.push(json!({
                        "type": "text",
                        "text": serde_json::to_string(part).unwrap_or_default()
                    })),
                }
            }
        }
        Value::Object(object)
            if matches!(
                object.get("type").and_then(Value::as_str),
                Some("text" | "input_text" | "output_text")
            ) =>
        {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                blocks.push(json!({ "type": "text", "text": text }));
            }
        }
        Value::Null | Value::String(_) => {}
        other => blocks.push(json!({
            "type": "text",
            "text": serde_json::to_string(other).unwrap_or_default()
        })),
    }
}

fn message_content_to_anthropic(content: &MessageContent) -> Vec<Value> {
    match content {
        MessageContent::Text(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![json!({ "type": "text", "text": text })]
            }
        }
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(message_content_part_to_anthropic)
            .collect(),
        MessageContent::Empty => Vec::new(),
    }
}

fn message_content_part_to_anthropic(part: &MessageContentPart) -> Option<Value> {
    if let Some(raw_block) = part
        .transformer_metadata
        .get(ANTHROPIC_RAW_CONTENT_BLOCK_KEY)
        .cloned()
    {
        return Some(raw_block);
    }
    match part.part_type.as_str() {
        "text" | "input_text" | "output_text" => {
            let text = part.text.as_deref()?;
            if text.is_empty() {
                return None;
            }
            let mut value = json!({ "type": "text", "text": text });
            if let Some(cache_control) = &part.cache_control {
                value["cache_control"] = cache_control.clone();
            }
            Some(value)
        }
        "image_url" | "input_image" => {
            let image = part.image_url.as_ref()?;
            let mut value = if let Some((media_type, data)) = image
                .url
                .strip_prefix("data:")
                .and_then(|rest| rest.split_once(";base64,"))
            {
                json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data
                    }
                })
            } else if !image.url.is_empty() {
                json!({
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": image.url
                    }
                })
            } else {
                return None;
            };
            if let Some(cache_control) = &part.cache_control {
                value["cache_control"] = cache_control.clone();
            }
            Some(value)
        }
        _ => None,
    }
}

fn tool_call_to_anthropic(tool_call: &ToolCall) -> Value {
    let mut value = json!({
        "type": "tool_use",
        "id": tool_call.id,
        "name": tool_call.function.name,
        "input": tool_arguments_value(&tool_call.function.arguments)
    });
    if let Some(cache_control) = &tool_call.cache_control {
        value["cache_control"] = cache_control.clone();
    }
    value
}

#[allow(dead_code)]
pub fn roundtrip_request(body: Value) -> Value {
    llm_request_to_anthropic(anthropic_request_to_llm(body))
}

#[allow(dead_code)]
pub fn roundtrip_response(body: Value) -> Value {
    llm_response_to_anthropic(anthropic_response_to_llm(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_roundtrip_preserves_anthropic_system_array_shape() {
        let source = json!({
            "model": "claude-sonnet",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "System A"},
                {"type": "text", "text": "System B"}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let roundtripped = roundtrip_request(source);

        assert!(roundtripped["system"].is_array());
        assert_eq!(roundtripped["system"][0]["text"], "System A");
        assert_eq!(roundtripped["system"][1]["text"], "System B");
    }

    #[test]
    fn request_roundtrip_keeps_anthropic_system_string_shape() {
        let source = json!({
            "model": "claude-sonnet",
            "max_tokens": 1024,
            "system": "System A",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let roundtripped = roundtrip_request(source);

        assert_eq!(roundtripped["system"], "System A");
    }

    #[test]
    fn request_roundtrip_preserves_anthropic_native_web_search_tool() {
        let source = json!({
            "model": "claude-sonnet",
            "max_tokens": 1024,
            "tools": [{
                "type": "web_search_20250305",
                "name": "web_search",
                "max_uses": 3,
                "allowed_domains": ["example.com"],
                "user_location": {
                    "type": "approximate",
                    "country": "US"
                }
            }],
            "messages": [{"role": "user", "content": "search"}]
        });

        let roundtripped = roundtrip_request(source);

        assert_eq!(roundtripped["tools"][0]["type"], "web_search_20250305");
        assert_eq!(roundtripped["tools"][0]["max_uses"], 3);
        assert_eq!(
            roundtripped["tools"][0]["allowed_domains"][0],
            "example.com"
        );
        assert_eq!(roundtripped["tools"][0]["user_location"]["country"], "US");
    }

    #[test]
    fn response_roundtrip_preserves_anthropic_server_tool_blocks() {
        let source = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet",
            "content": [{
                "type": "server_tool_use",
                "id": "srv_1",
                "name": "web_search",
                "input": {"query": "rust"}
            }, {
                "type": "web_search_tool_result",
                "tool_use_id": "srv_1",
                "content": [{"type": "web_search_result", "title": "Rust"}]
            }, {
                "type": "text",
                "text": "done"
            }],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });

        let roundtripped = roundtrip_response(source);

        assert_eq!(roundtripped["content"][0]["type"], "server_tool_use");
        assert_eq!(roundtripped["content"][0]["input"]["query"], "rust");
        assert_eq!(roundtripped["content"][1]["type"], "web_search_tool_result");
        assert_eq!(roundtripped["content"][2]["text"], "done");
    }
}
