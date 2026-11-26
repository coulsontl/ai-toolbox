use super::super::error::ProtocolConversionError;
use super::super::llm::{
    ApiFormat, Choice, Function, FunctionCall, ImageUrl, Message, MessageContent,
    MessageContentPart, Request, RequestType, Response, ResponseCustomTool, ResponseCustomToolCall,
    StreamOptions, Tool, ToolCall, Usage, TOOL_TYPE_FUNCTION, TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
};
use super::super::shared::tool_media::{
    cleaned_tool_result_text, plan_tool_result_images, ToolImage, ToolImageScope,
    TOOL_RESULT_MEDIA_MOVED_MARKER,
};
use super::super::shared::{
    content_text, extract_error_code, extract_error_message, extract_error_param,
    extract_error_type, extract_reasoning_field_text, normalize_chat_system_messages,
    normalize_function_parameters_owned, placement_for_api_format,
    should_emit_openai_request_metadata, split_leading_think_block, stop_from_value, stop_to_value,
    tool_choice_from_openai, tool_choice_to_openai,
};
use super::super::traits::{InboundTransformer, OutboundTransformer};
use super::super::types::AiProtocol;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub struct OpenAiChatInbound;
pub struct OpenAiChatOutbound;

const CHAT_CITATIONS_METADATA_KEY: &str = "citations";

impl InboundTransformer for OpenAiChatInbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::OpenAiChat
    }

    fn request_to_llm(&self, body: Value) -> Result<Request, ProtocolConversionError> {
        Ok(chat_request_to_llm(body))
    }

    fn response_from_llm(&self, response: Response) -> Result<Value, ProtocolConversionError> {
        Ok(llm_response_to_chat(response))
    }

    fn error_from_llm(&self, error: Value) -> Value {
        openai_error(error)
    }
}

impl OutboundTransformer for OpenAiChatOutbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::OpenAiChat
    }

    fn request_from_llm(&self, request: Request) -> Result<Value, ProtocolConversionError> {
        Ok(llm_request_to_chat(request))
    }

    fn response_to_llm(&self, body: Value) -> Result<Response, ProtocolConversionError> {
        Ok(chat_response_to_llm(body))
    }

    fn error_to_llm(&self, error: Value) -> Value {
        error
    }
}

pub fn chat_request_to_llm(body: Value) -> Request {
    let mut request = Request {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        max_completion_tokens: body.get("max_completion_tokens").and_then(Value::as_i64),
        max_tokens: body.get("max_tokens").and_then(Value::as_i64),
        reasoning_effort: body
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        frequency_penalty: body.get("frequency_penalty").and_then(Value::as_f64),
        presence_penalty: body.get("presence_penalty").and_then(Value::as_f64),
        seed: body.get("seed").and_then(Value::as_i64),
        service_tier: body
            .get("service_tier")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        logprobs: body.get("logprobs").and_then(Value::as_bool),
        top_logprobs: body.get("top_logprobs").and_then(Value::as_i64),
        user: body
            .get("user")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        logit_bias: body.get("logit_bias").cloned(),
        verbosity: body
            .get("verbosity")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        stream: body.get("stream").and_then(Value::as_bool),
        stream_options: body
            .pointer("/stream_options/include_usage")
            .and_then(Value::as_bool)
            .map(|include_usage| StreamOptions { include_usage }),
        stop: stop_from_value(body.get("stop")),
        tool_choice: tool_choice_from_openai(body.get("tool_choice")),
        parallel_tool_calls: body.get("parallel_tool_calls").and_then(Value::as_bool),
        response_format: body.get("response_format").cloned(),
        prompt_cache_key: body
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        metadata: metadata_from_value(body.get("metadata")),
        extra_body: body.get("extra_body").cloned(),
        request_type: Some(RequestType::Chat),
        api_format: Some(ApiFormat::OpenAiChatCompletions),
        ..Default::default()
    };
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        request.messages = messages.iter().map(chat_message_to_llm).collect();
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        request.tools = tools
            .iter()
            .filter_map(|tool| {
                if tool.get("type").and_then(Value::as_str) == Some(TOOL_TYPE_RESPONSES_CUSTOM_TOOL)
                {
                    return Some(Tool {
                        tool_type: TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string(),
                        function: tool.get("function").map(|function| Function {
                            name: function
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            ..Default::default()
                        }),
                        response_custom_tool: tool.get("response_custom_tool").cloned().and_then(
                            |value| serde_json::from_value::<ResponseCustomTool>(value).ok(),
                        ),
                        ..Default::default()
                    });
                }
                let function = tool.get("function")?;
                Some(Tool {
                    tool_type: TOOL_TYPE_FUNCTION.to_string(),
                    function: Some(Function {
                        name: function
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        description: function
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        parameters: function.get("parameters").cloned(),
                        strict: function.get("strict").and_then(Value::as_bool),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            })
            .collect();
    }
    request
}

fn chat_message_to_llm(message: &Value) -> Message {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_string();
    if role == "tool" {
        return Message {
            role,
            name: message
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            tool_call_id: message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content: MessageContent::Text(content_text(message.get("content"))),
            ..Default::default()
        };
    }
    let mut chunked_thinking_segments: Vec<String> = Vec::new();
    let content = match message.get("content") {
        Some(Value::Array(parts)) => {
            let mut extracted = Vec::new();
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => extracted.push(MessageContentPart {
                        part_type: "text".to_string(),
                        text: part
                            .get("text")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        ..Default::default()
                    }),
                    Some("image_url") => extracted.push(MessageContentPart {
                        part_type: "image_url".to_string(),
                        image_url: part
                            .get("image_url")
                            .cloned()
                            .and_then(|value| serde_json::from_value::<ImageUrl>(value).ok()),
                        ..Default::default()
                    }),
                    // Mistral Chat API returns chunked `content` arrays with
                    // `{"type":"thinking","thinking":[{"type":"text","text":...}]}`
                    // (cc-switch 0c1121ce). Extract into unified reasoning.
                    Some("thinking") => {
                        extract_chat_thinking_part_text(part, &mut chunked_thinking_segments);
                    }
                    _ => {}
                }
            }
            if extracted.is_empty() && !chunked_thinking_segments.is_empty() {
                MessageContent::Empty
            } else {
                MessageContent::Parts(extracted)
            }
        }
        Some(Value::String(text)) => MessageContent::Text(text.clone()),
        _ => MessageContent::Empty,
    };
    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, call) in calls.iter().enumerate() {
            if call.get("type").and_then(Value::as_str) == Some(TOOL_TYPE_RESPONSES_CUSTOM_TOOL) {
                let custom = call
                    .get("response_custom_tool_call")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<ResponseCustomToolCall>(value).ok());
                let call_id = custom
                    .as_ref()
                    .map(|custom| custom.call_id.as_str())
                    .or_else(|| call.get("id").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                let name = custom
                    .as_ref()
                    .map(|custom| custom.name.clone())
                    .unwrap_or_default();
                let input = custom
                    .as_ref()
                    .map(|custom| custom.input.clone())
                    .unwrap_or_default();
                tool_calls.push(ToolCall {
                    id: call_id.clone(),
                    tool_type: TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string(),
                    function: FunctionCall {
                        name: name.clone(),
                        arguments: input.clone(),
                    },
                    response_custom_tool_call: Some(ResponseCustomToolCall {
                        call_id,
                        name,
                        input,
                    }),
                    index,
                    ..Default::default()
                });
                continue;
            }
            let function = call.get("function").unwrap_or(call);
            tool_calls.push(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                tool_type: TOOL_TYPE_FUNCTION.to_string(),
                function: FunctionCall {
                    name: function
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                index,
                ..Default::default()
            });
        }
    }
    if let Some(function_call) = message.get("function_call") {
        let index = tool_calls.len();
        tool_calls.push(ToolCall {
            id: function_call
                .get("id")
                .or_else(|| message.get("id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("call_{index}")),
            tool_type: TOOL_TYPE_FUNCTION.to_string(),
            function: FunctionCall {
                name: function_call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                arguments: function_call
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            },
            index,
            ..Default::default()
        });
    }
    let explicit_reasoning = extract_reasoning_field_text(message).or_else(|| {
        (!chunked_thinking_segments.is_empty()).then(|| chunked_thinking_segments.join(""))
    });
    let (content, inline_reasoning) =
        split_chat_inline_think_content(content, explicit_reasoning.is_none());
    let reasoning = explicit_reasoning.or(inline_reasoning);

    Message {
        role,
        content,
        name: message
            .get("name")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        refusal: message
            .get("refusal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        tool_calls,
        annotations: message
            .get("annotations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        reasoning_content: reasoning.clone(),
        reasoning,
        ..Default::default()
    }
}

fn split_chat_inline_think_content(
    content: MessageContent,
    enabled: bool,
) -> (MessageContent, Option<String>) {
    if !enabled {
        return (content, None);
    }
    match content {
        MessageContent::Text(text) => {
            if let Some((reasoning, answer)) = split_leading_think_block(&text) {
                (MessageContent::Text(answer), Some(reasoning))
            } else {
                (MessageContent::Text(text), None)
            }
        }
        other => (other, None),
    }
}

pub fn llm_request_to_chat(request: Request) -> Value {
    let placement = placement_for_api_format(request.api_format);
    let messages = llm_messages_to_chat(request.messages, request.api_format);
    let messages = normalize_chat_system_messages(messages, placement);
    let mut body = json!({
        "model": request.model,
        "messages": messages,
    });
    if let Some(max_tokens) = request.max_tokens.or(request.max_completion_tokens) {
        let max_tokens_key = if uses_max_completion_tokens(&request.model) {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body[max_tokens_key] = json!(max_tokens);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(frequency_penalty) = request.frequency_penalty {
        body["frequency_penalty"] = json!(frequency_penalty);
    }
    if let Some(presence_penalty) = request.presence_penalty {
        body["presence_penalty"] = json!(presence_penalty);
    }
    if let Some(seed) = request.seed {
        body["seed"] = json!(seed);
    }
    if let Some(service_tier) = request.service_tier {
        body["service_tier"] = json!(service_tier);
    }
    if let Some(logprobs) = request.logprobs {
        body["logprobs"] = json!(logprobs);
    }
    if let Some(top_logprobs) = request.top_logprobs {
        body["top_logprobs"] = json!(top_logprobs);
    }
    if let Some(user) = request.user {
        body["user"] = json!(user);
    }
    if let Some(logit_bias) = request.logit_bias {
        body["logit_bias"] = logit_bias;
    }
    if let Some(verbosity) = request.verbosity {
        body["verbosity"] = json!(verbosity);
    }
    if let Some(reasoning_effort) = request.reasoning_effort {
        body["reasoning_effort"] = json!(reasoning_effort);
    }
    if let Some(stream) = request.stream {
        body["stream"] = json!(stream);
        if stream {
            let include_usage = request
                .stream_options
                .as_ref()
                .map(|options| options.include_usage)
                .unwrap_or(true);
            body["stream_options"] = json!({"include_usage": include_usage});
        }
    }
    if let Some(stop) = stop_to_value(request.stop) {
        body["stop"] = stop;
    }
    let tools = request
        .tools
        .into_iter()
        .filter_map(|tool| {
            if tool.tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                let custom = tool.response_custom_tool?;
                if custom.name.is_empty() {
                    return None;
                }
                return Some(json!({
                    "type": TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
                    "function": {
                        "name": tool
                            .function
                            .map(|function| function.name)
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| custom.name.clone())
                    },
                    "response_custom_tool": custom
                }));
            }
            let function = tool.function?;
            if function.name.is_empty() {
                return None;
            }
            let mut function_object = Map::new();
            function_object.insert("name".to_string(), json!(function.name));
            function_object.insert("description".to_string(), json!(function.description));
            function_object.insert(
                "parameters".to_string(),
                normalize_function_parameters_owned(function.parameters),
            );
            if let Some(strict) = function.strict {
                function_object.insert("strict".to_string(), json!(strict));
            }
            Some(json!({
                "type": "function",
                "function": Value::Object(function_object)
            }))
        })
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(tool_choice) = tool_choice_to_openai(request.tool_choice) {
        body["tool_choice"] = tool_choice;
    }
    if let Some(parallel_tool_calls) = request.parallel_tool_calls {
        body["parallel_tool_calls"] = json!(parallel_tool_calls);
    }
    if let Some(response_format) = request.response_format {
        body["response_format"] = response_format;
    }
    if let Some(prompt_cache_key) = request.prompt_cache_key {
        body["prompt_cache_key"] = json!(prompt_cache_key);
    }
    if should_emit_openai_request_metadata(request.api_format) && !request.metadata.is_empty() {
        body["metadata"] = json!(request.metadata);
    }
    if let Some(extra_body) = request.extra_body {
        body["extra_body"] = extra_body;
    }
    body
}

fn extract_chat_thinking_part_text(part: &Value, out: &mut Vec<String>) {
    if let Some(text) = part
        .get("thinking")
        .and_then(Value::as_str)
        .or_else(|| part.get("text").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
    {
        out.push(text.to_string());
        return;
    }
    if let Some(items) = part.get("thinking").and_then(Value::as_array) {
        for item in items {
            if let Some(text) = item
                .get("text")
                .or_else(|| item.get("content"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                out.push(text.to_string());
            }
        }
    }
}

fn llm_messages_to_chat(messages: Vec<Message>, api_format: Option<ApiFormat>) -> Vec<Value> {
    let mut converted = Vec::with_capacity(messages.len());
    let mut pending_media = Vec::new();
    let mut deferred_orphan_outputs: Vec<Value> = Vec::new();
    let mut seen_tool_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let has_any_assistant_tool_call = messages.iter().any(|m| !m.tool_calls.is_empty());
    let downgrade_unmatched_ids = matches!(api_format, Some(ApiFormat::OpenAiResponses));

    for message in messages {
        if message.role == "tool" {
            let call_id = message.tool_call_id.clone().unwrap_or_default();
            let is_orphan = call_id.trim().is_empty()
                || (downgrade_unmatched_ids
                    && has_any_assistant_tool_call
                    && !seen_tool_call_ids.contains(&call_id));
            if is_orphan {
                // Codex Desktop can emit call-less or orphaned tool outputs
                // (e.g. `create_thread` / `heartbeat`). Strict Chat upstreams
                // reject orphan `role: "tool"` with HTTP 400 (cc-switch 9055dd37).
                // Downgrade to a deferred user message flushed after the active
                // contiguous `role: "tool"` batch.
                if let Some(plan) = plan_tool_result_images(
                    &message.content,
                    ToolImageScope::AnyImage,
                    TOOL_RESULT_MEDIA_MOVED_MARKER,
                ) {
                    let label = if call_id.trim().is_empty() {
                        "[Tool output]".to_string()
                    } else {
                        format!("[Tool output for {call_id}]")
                    };
                    let text = cleaned_tool_result_text(&plan);
                    deferred_orphan_outputs.push(json!({
                        "role": "user",
                        "content": format!("{label}\n{text}")
                    }));
                    queue_chat_tool_media(&mut pending_media, &call_id, plan.images);
                } else {
                    let text = match message.content {
                        MessageContent::Text(text) => text,
                        other => serde_json::to_string(&other).unwrap_or_default(),
                    };
                    let label = if call_id.trim().is_empty() {
                        "[Tool output]".to_string()
                    } else {
                        format!("[Tool output for {call_id}]")
                    };
                    deferred_orphan_outputs.push(json!({
                        "role": "user",
                        "content": format!("{label}\n{text}")
                    }));
                }
                continue;
            }
            if let Some(plan) = plan_tool_result_images(
                &message.content,
                ToolImageScope::AnyImage,
                TOOL_RESULT_MEDIA_MOVED_MARKER,
            ) {
                let mut tool_message = json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": cleaned_tool_result_text(&plan)
                });
                if let Some(name) = message.name {
                    tool_message["name"] = json!(name);
                }
                converted.push(tool_message);
                queue_chat_tool_media(&mut pending_media, &call_id, plan.images);
                continue;
            }
        } else {
            flush_chat_tool_media(&mut converted, &mut pending_media);
            converted.extend(deferred_orphan_outputs.drain(..));
        }

        // Drop completely empty user messages (e.g. post-compaction media-only
        // turns that became empty `input_text`), which strict Chat upstreams
        // reject with "message content cannot be empty" (cc-switch f7002d1e).
        if message.role == "user" && is_empty_chat_user_message(&message) {
            continue;
        }

        for call in &message.tool_calls {
            if !call.id.is_empty() {
                seen_tool_call_ids.insert(call.id.clone());
            }
        }
        converted.push(llm_message_to_chat(message));
    }

    flush_chat_tool_media(&mut converted, &mut pending_media);
    converted.extend(deferred_orphan_outputs.drain(..));
    converted
}

fn is_empty_chat_user_message(message: &Message) -> bool {
    match &message.content {
        MessageContent::Empty => true,
        MessageContent::Text(text) => text.is_empty(),
        MessageContent::Parts(parts) => !parts.iter().any(|part| match part.part_type.as_str() {
            "text" | "input_text" | "output_text" => {
                part.text.as_deref().is_some_and(|text| !text.is_empty())
            }
            "image_url" | "input_image" => part
                .image_url
                .as_ref()
                .is_some_and(|image| !image.url.is_empty()),
            _ => false,
        }),
    }
}

fn normalize_chat_image_detail(detail: Option<String>) -> Option<String> {
    detail.map(|value| {
        if value.eq_ignore_ascii_case("original") {
            // `detail: "original"` is Responses-only; Chat upstreams reject it
            // with HTTP 400 (cc-switch 83a24dfb).
            "auto".to_string()
        } else {
            value
        }
    })
}

fn queue_chat_tool_media(pending_media: &mut Vec<Value>, call_id: &str, images: Vec<ToolImage>) {
    if images.is_empty() {
        return;
    }
    pending_media.push(json!({
        "type": "text",
        "text": format!("[ai-toolbox: media output of tool call {call_id}]")
    }));
    pending_media.extend(images.into_iter().map(|image| {
        let mut image_url = json!({ "url": image.url });
        if let Some(detail) = normalize_chat_image_detail(image.detail) {
            image_url["detail"] = json!(detail);
        }
        json!({
            "type": "image_url",
            "image_url": image_url
        })
    }));
}

fn flush_chat_tool_media(messages: &mut Vec<Value>, pending_media: &mut Vec<Value>) {
    if pending_media.is_empty() {
        return;
    }
    messages.push(json!({
        "role": "user",
        "content": std::mem::take(pending_media)
    }));
}

fn llm_message_to_chat(message: Message) -> Value {
    // Codex (OpenAI Responses) emits `developer` system instructions. OpenAI's own
    // Chat Completions accepts `developer`, but third-party OpenAI-compatible chat
    // providers (kimi/deepseek/qwen/glm, ...) only recognize `system`. Normalize to
    // `system` so the converted request stays usable by those upstreams.
    let role = if message.role.eq_ignore_ascii_case("developer") {
        "system".to_string()
    } else {
        message.role
    };
    if role == "tool" {
        let mut result = json!({
            "role": "tool",
            "tool_call_id": message.tool_call_id.unwrap_or_default(),
            "content": match message.content {
                MessageContent::Text(text) => text,
                other => serde_json::to_string(&other).unwrap_or_default(),
            }
        });
        if let Some(name) = message.name {
            result["name"] = json!(name);
        }
        return result;
    }
    let has_reasoning = message
        .reasoning_content
        .as_deref()
        .or(message.reasoning.as_deref())
        .is_some_and(|text| !text.is_empty());
    let mut content_value = llm_content_to_chat_value(message.content);
    // Strict OpenAI-compatible backends (e.g. llama.cpp/vLLM) reject
    // reasoning-only assistant messages with `"content": null` when no
    // `tool_calls` are present (AxonHub 250bd3bf).
    if role == "assistant"
        && message.tool_calls.is_empty()
        && has_reasoning
        && content_value.is_null()
    {
        content_value = json!("");
    }
    let mut result = json!({
        "role": role,
        "content": content_value,
    });
    if let Some(name) = message.name {
        result["name"] = json!(name);
    }
    if !message.refusal.is_empty() {
        result["refusal"] = json!(message.refusal);
    }
    if !message.annotations.is_empty() {
        result["annotations"] = json!(message.annotations);
    }
    if let Some(reasoning) = message.reasoning_content.or(message.reasoning) {
        result["reasoning_content"] = json!(reasoning);
    }
    if !message.tool_calls.is_empty() {
        result["tool_calls"] = json!(message
            .tool_calls
            .into_iter()
            .map(|call| {
                if call.tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                    let custom = call
                        .response_custom_tool_call
                        .unwrap_or(ResponseCustomToolCall {
                            call_id: call.id.clone(),
                            name: call.function.name.clone(),
                            input: call.function.arguments.clone(),
                        });
                    return json!({
                        "id": call.id,
                        "type": TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
                        "function": {
                            "name": ""
                        },
                        "response_custom_tool_call": custom
                    });
                }
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.function.name,
                        "arguments": call.function.arguments
                    }
                })
            })
            .collect::<Vec<_>>());
    }
    result
}

fn metadata_from_value(value: Option<&Value>) -> HashMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|text| (key.clone(), text.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn llm_content_to_chat_value(content: MessageContent) -> Value {
    match content {
        MessageContent::Text(text) => json!(text),
        MessageContent::Parts(parts) => json!(parts
            .into_iter()
            .filter_map(|part| {
                match part.part_type.as_str() {
                    "text" | "input_text" | "output_text" => Some(json!({
                        "type": "text",
                        "text": part.text.unwrap_or_default()
                    })),
                    "image_url" | "input_image" => Some(json!({
                        "type": "image_url",
                        "image_url": part.image_url.map(|mut image| {
                            image.detail = normalize_chat_image_detail(image.detail);
                            image
                        })
                    })),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()),
        MessageContent::Empty => Value::Null,
    }
}

pub fn chat_response_to_llm(body: Value) -> Response {
    let choices = body
        .get("choices")
        .and_then(Value::as_array)
        .filter(|choices| !choices.is_empty())
        .map(|choices| {
            choices
                .iter()
                .map(|choice| {
                    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
                    Choice {
                        index: choice.get("index").and_then(Value::as_u64).unwrap_or(0) as usize,
                        message: chat_message_to_llm(&message),
                        finish_reason: choice
                            .get("finish_reason")
                            .and_then(Value::as_str)
                            .map(|reason| match reason {
                                "function_call" => "tool_calls",
                                "sensitive" => "content_filter",
                                other => other,
                            })
                            .map(ToString::to_string),
                        ..Default::default()
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![Choice {
                message: chat_message_to_llm(&json!({})),
                ..Default::default()
            }]
        });
    let mut response = Response {
        id: body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        object: body
            .get("object")
            .and_then(Value::as_str)
            .unwrap_or("chat.completion")
            .to_string(),
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        choices,
        usage: Some(openai_usage_to_llm(body.get("usage"))),
        ..Default::default()
    };
    if let Some(citations) = body.get("citations").and_then(Value::as_array) {
        let citations = citations
            .iter()
            .filter_map(|citation| citation.as_str().filter(|text| !text.is_empty()))
            .map(|citation| json!(citation))
            .collect::<Vec<_>>();
        if !citations.is_empty() {
            response
                .transformer_metadata
                .insert(CHAT_CITATIONS_METADATA_KEY.to_string(), json!(citations));
        }
    }
    response
}

fn uses_max_completion_tokens(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    is_openai_o_series(&normalized)
        || normalized
            .strip_prefix("gpt-")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|ch| ch.is_ascii_digit() && ch >= '5')
}

fn is_openai_o_series(model: &str) -> bool {
    model.len() > 1
        && model.starts_with('o')
        && model
            .as_bytes()
            .get(1)
            .is_some_and(|byte| byte.is_ascii_digit())
}

pub fn llm_response_to_chat(response: Response) -> Value {
    if let Some(error) = response.error.as_ref() {
        return json!({
            "error": {
                "message": error.message,
                "type": if error.error_type.is_empty() {
                    "api_error".to_string()
                } else {
                    error.error_type.clone()
                },
                "code": error
                    .code
                    .clone()
                    .unwrap_or_else(|| "response_error".to_string())
            }
        });
    }
    let is_error_finish = response
        .choices
        .first()
        .and_then(|choice| choice.finish_reason.as_deref())
        == Some("error");
    if is_error_finish {
        return json!({
            "error": {
                "message": "Response failed",
                "type": "api_error",
                "code": "response_error"
            }
        });
    }

    let citations = response
        .transformer_metadata
        .get(CHAT_CITATIONS_METADATA_KEY)
        .and_then(Value::as_array)
        .filter(|citations| !citations.is_empty())
        .cloned();
    let choices = if response.choices.is_empty() {
        vec![Choice::default()]
    } else {
        response.choices
    };
    let mut body = json!({
        "id": response.id,
        "object": "chat.completion",
        "created": response.created,
        "model": response.model,
        "choices": choices
            .into_iter()
            .map(|choice| {
                let finish_reason = match choice.finish_reason.as_deref() {
                    Some("refusal") | Some("sensitive") => "content_filter".to_string(),
                    Some("pause_turn") | Some("stop_sequence") => "stop".to_string(),
                    Some(reason) if !reason.is_empty() => reason.to_string(),
                    _ => "stop".to_string(),
                };
                json!({
                    "index": choice.index,
                    "message": llm_message_to_chat(choice.message),
                    "finish_reason": finish_reason
                })
            })
            .collect::<Vec<_>>(),
        "usage": usage_to_openai(response.usage.as_ref())
    });
    if let Some(citations) = citations {
        body["citations"] = json!(citations);
    }
    body
}

pub fn openai_usage_to_llm(usage: Option<&Value>) -> Usage {
    let usage = usage.unwrap_or(&Value::Null);
    let prompt = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    // Responses API (AxonHub 94704784) and some Chat providers surface cache writes
    // under input/prompt token details.
    let cache_write = usage
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
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| prompt.saturating_add(completion)),
        cached_tokens: cached,
        cache_write_tokens: cache_write,
        reasoning_tokens,
    }
}

pub fn usage_to_openai(usage: Option<&Usage>) -> Value {
    let usage = usage.cloned().unwrap_or_default();
    let mut prompt_details = json!({
        "cached_tokens": usage.cached_tokens
    });
    if usage.cache_write_tokens > 0 {
        prompt_details["cache_write_tokens"] = json!(usage.cache_write_tokens);
    }
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": if usage.total_tokens == 0 {
            usage.prompt_tokens.saturating_add(usage.completion_tokens)
        } else {
            usage.total_tokens
        },
        "prompt_tokens_details": prompt_details,
        "completion_tokens_details": {
            "reasoning_tokens": usage.reasoning_tokens
        }
    })
}

fn openai_error(error: Value) -> Value {
    let message =
        extract_error_message(&error).unwrap_or_else(|| "Protocol conversion error".to_string());
    json!({
        "error": {
            "message": message,
            "type": extract_error_type(&error).unwrap_or_else(|| "api_error".to_string()),
            "param": extract_error_param(&error).unwrap_or(Value::Null),
            "code": extract_error_code(&error).unwrap_or(Value::Null)
        }
    })
}
