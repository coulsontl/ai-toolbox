use super::chat::openai_usage_to_llm;
use crate::coding::proxy_gateway::transformer::error::ProtocolConversionError;
use crate::coding::proxy_gateway::transformer::llm::{
    ApiFormat, Choice, Function, FunctionCall, ImageUrl, Message, MessageContent,
    MessageContentPart, Request, RequestType, Response, ResponseCustomToolCall, Tool, ToolCall,
    Usage, TOOL_TYPE_FUNCTION, TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
};
use crate::coding::proxy_gateway::transformer::shared::signature::{
    decode_signature_for, encode_signature, SignatureProvider,
};
use crate::coding::proxy_gateway::transformer::shared::{
    extract_error_code, extract_error_message, extract_error_param, extract_error_type,
    should_emit_openai_request_metadata, stop_from_value, stop_to_value, tool_choice_from_openai,
    tool_choice_to_responses,
};
use crate::coding::proxy_gateway::transformer::traits::{InboundTransformer, OutboundTransformer};
use crate::coding::proxy_gateway::transformer::types::AiProtocol;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub struct OpenAiResponsesInbound;
pub struct OpenAiResponsesOutbound;

const RESPONSES_INCLUDE_METADATA_KEY: &str = "openai_responses_include";

impl InboundTransformer for OpenAiResponsesInbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::OpenAiResponses
    }

    fn request_to_llm(&self, body: Value) -> Result<Request, ProtocolConversionError> {
        Ok(responses_request_to_llm(body))
    }

    fn response_from_llm(&self, response: Response) -> Result<Value, ProtocolConversionError> {
        Ok(llm_response_to_responses(response))
    }

    fn error_from_llm(&self, error: Value) -> Value {
        let message = extract_error_message(&error)
            .unwrap_or_else(|| "Protocol conversion error".to_string());
        json!({
            "error": {
                "message": message,
                "type": extract_error_type(&error).unwrap_or_else(|| "api_error".to_string()),
                "param": extract_error_param(&error).unwrap_or(Value::Null),
                "code": extract_error_code(&error).unwrap_or(Value::Null)
            }
        })
    }
}

impl OutboundTransformer for OpenAiResponsesOutbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::OpenAiResponses
    }

    fn request_from_llm(&self, request: Request) -> Result<Value, ProtocolConversionError> {
        Ok(llm_request_to_responses(request))
    }

    fn response_to_llm(&self, body: Value) -> Result<Response, ProtocolConversionError> {
        Ok(responses_response_to_llm(body))
    }

    fn error_to_llm(&self, error: Value) -> Value {
        error
    }
}

pub fn responses_request_to_llm(body: Value) -> Request {
    let mut request = Request {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        max_tokens: body
            .get("max_output_tokens")
            .or_else(|| body.get("max_tokens"))
            .and_then(Value::as_i64),
        reasoning_effort: body
            .pointer("/reasoning/effort")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        frequency_penalty: body.get("frequency_penalty").and_then(Value::as_f64),
        presence_penalty: body.get("presence_penalty").and_then(Value::as_f64),
        service_tier: body
            .get("service_tier")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        top_logprobs: body.get("top_logprobs").and_then(Value::as_i64),
        user: body
            .get("user")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        verbosity: body
            .pointer("/text/verbosity")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        stream: body.get("stream").and_then(Value::as_bool),
        previous_response_id: body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        stop: stop_from_value(body.get("stop")),
        tool_choice: tool_choice_from_openai(body.get("tool_choice")),
        parallel_tool_calls: body.get("parallel_tool_calls").and_then(Value::as_bool),
        response_format: body
            .pointer("/text/format")
            .cloned()
            .map(responses_format_to_chat),
        prompt_cache_key: body
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        metadata: metadata_from_value(body.get("metadata")),
        extra_body: body.get("extra_body").cloned(),
        request_type: Some(RequestType::Chat),
        api_format: Some(ApiFormat::OpenAiResponses),
        ..Default::default()
    };
    if let Some(instructions) = responses_instructions_text(body.get("instructions")) {
        if !instructions.is_empty() {
            request.messages.push(Message {
                role: "system".to_string(),
                content: MessageContent::Text(instructions),
                ..Default::default()
            });
        }
    }
    if let Some(include) = body.get("include") {
        request
            .transformer_metadata
            .insert(RESPONSES_INCLUDE_METADATA_KEY.to_string(), include.clone());
    }
    append_responses_input_to_messages(body.get("input"), &mut request.messages);
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        request.tools = tools.iter().filter_map(responses_tool_to_llm).collect();
    }
    request
}

fn responses_instructions_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| part.as_str())
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            Some(text)
        }
        other => other.as_str().map(ToString::to_string),
    }
}

fn append_responses_input_to_messages(input: Option<&Value>, messages: &mut Vec<Message>) {
    match input {
        Some(Value::Array(items)) => {
            let mut index = 0;
            while index < items.len() {
                let item = &items[index];
                if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    let mut reasoning_message = responses_reasoning_message(item);
                    if items.get(index + 1).is_some_and(|following| {
                        merge_responses_following_item_into_reasoning_message(
                            &mut reasoning_message,
                            following,
                        )
                    }) {
                        messages.push(reasoning_message);
                        index += 2;
                    } else {
                        messages.push(reasoning_message);
                        index += 1;
                    }
                    continue;
                }
                append_responses_item_to_messages(item, messages);
                index += 1;
            }
        }
        Some(Value::String(text)) => messages.push(Message {
            role: "user".to_string(),
            content: MessageContent::Text(text.clone()),
            ..Default::default()
        }),
        Some(Value::Object(_)) => append_responses_item_to_messages(input.unwrap(), messages),
        _ => {}
    }
}

fn append_responses_item_to_messages(item: &Value, messages: &mut Vec<Message>) {
    match item.get("type").and_then(Value::as_str) {
        Some("function_call") | Some("custom_tool_call") => messages.push(Message {
            role: "assistant".to_string(),
            tool_calls: vec![responses_call_to_tool_call(item, 0)],
            ..Default::default()
        }),
        Some("function_call_output") | Some("custom_tool_call_output") => messages.push(Message {
            role: "tool".to_string(),
            tool_call_id: item
                .get("call_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content: MessageContent::Text(
                item.get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            ),
            ..Default::default()
        }),
        Some("reasoning") => messages.push(responses_reasoning_message(item)),
        Some("input_image") => {
            if let Some(part) = responses_input_image_part(item) {
                messages.push(Message {
                    role: "user".to_string(),
                    content: MessageContent::Parts(vec![part]),
                    ..Default::default()
                });
            }
        }
        _ => messages.push(responses_message_item_to_llm(item)),
    }
}

fn responses_reasoning_message(item: &Value) -> Message {
    let reasoning = responses_reasoning_text(item);
    Message {
        role: "assistant".to_string(),
        reasoning_content: reasoning.clone(),
        reasoning,
        reasoning_signature: item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .filter(|signature| !signature.is_empty())
            .map(|signature| encode_signature(SignatureProvider::OpenAiResponses, signature)),
        ..Default::default()
    }
}

fn merge_responses_following_item_into_reasoning_message(
    reasoning_message: &mut Message,
    following: &Value,
) -> bool {
    match following.get("type").and_then(Value::as_str) {
        Some("function_call") | Some("custom_tool_call") => {
            reasoning_message
                .tool_calls
                .push(responses_call_to_tool_call(following, 0));
            true
        }
        Some("message") | None => {
            if following.get("content").is_none() {
                return false;
            }
            let following_message = responses_message_item_to_llm(following);
            merge_message_into_reasoning_message(reasoning_message, following_message);
            true
        }
        _ => false,
    }
}

fn merge_message_into_reasoning_message(reasoning_message: &mut Message, message: Message) {
    if reasoning_message.id.is_empty() {
        reasoning_message.id = message.id;
    }
    if reasoning_message.content.is_empty() {
        reasoning_message.content = message.content;
    } else if !message.content.is_empty() {
        let mut parts = message_content_into_parts(std::mem::take(&mut reasoning_message.content));
        parts.extend(message_content_into_parts(message.content));
        reasoning_message.content = MessageContent::Parts(parts);
    }
    if reasoning_message.refusal.is_empty() {
        reasoning_message.refusal = message.refusal;
    }
    reasoning_message.annotations.extend(message.annotations);
}

fn message_content_into_parts(content: MessageContent) -> Vec<MessageContentPart> {
    match content {
        MessageContent::Text(text) if !text.is_empty() => vec![MessageContentPart {
            part_type: "text".to_string(),
            text: Some(text),
            ..Default::default()
        }],
        MessageContent::Parts(parts) => parts,
        _ => Vec::new(),
    }
}

fn responses_message_item_to_llm(item: &Value) -> Message {
    let role = item
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_string();
    let mut parts = Vec::new();
    let mut refusal = String::new();
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for part in content {
            match part.get("type").and_then(Value::as_str) {
                Some("input_text") | Some("output_text") => {
                    parts.push(MessageContentPart {
                        id: part
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        part_type: "text".to_string(),
                        text: part
                            .get("text")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        ..Default::default()
                    });
                }
                Some("input_image") => {
                    if let Some(part) = responses_input_image_part(part) {
                        parts.push(part);
                    }
                }
                Some("refusal") => {
                    refusal = part
                        .get("refusal")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                _ => {}
            }
        }
    }
    Message {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        role,
        content: MessageContent::Parts(parts),
        refusal,
        annotations: item
            .get("content")
            .and_then(Value::as_array)
            .map(|content| content_annotations(content))
            .unwrap_or_default(),
        ..Default::default()
    }
}

fn responses_input_image_part(item: &Value) -> Option<MessageContentPart> {
    item.get("image_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(|url| MessageContentPart {
            id: item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            part_type: "image_url".to_string(),
            image_url: Some(ImageUrl {
                url: url.to_string(),
                detail: item
                    .get("detail")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            }),
            ..Default::default()
        })
}

fn responses_tool_to_llm(tool: &Value) -> Option<Tool> {
    match tool.get("type").and_then(Value::as_str) {
        Some("function") => Some(Tool {
            tool_type: TOOL_TYPE_FUNCTION.to_string(),
            function: Some(Function {
                name: tool
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                parameters: tool.get("parameters").cloned(),
                strict: tool.get("strict").and_then(Value::as_bool),
                ..Default::default()
            }),
            ..Default::default()
        }),
        Some("custom") => Some(Tool {
            tool_type: TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string(),
            response_custom_tool: serde_json::from_value(tool.clone()).ok(),
            ..Default::default()
        }),
        _ => None,
    }
}

fn responses_call_to_tool_call(item: &Value, index: usize) -> ToolCall {
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function_call");
    if item_type == "custom_tool_call" {
        let call_id = item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let input = item
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return ToolCall {
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
        };
    }
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let arguments = sanitize_responses_function_arguments(
        &name,
        item.get("arguments")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    ToolCall {
        id: item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        tool_type: TOOL_TYPE_FUNCTION.to_string(),
        function: FunctionCall { name, arguments },
        index,
        ..Default::default()
    }
}

fn sanitize_responses_function_arguments(name: &str, arguments: &str) -> String {
    if name != "Read" || arguments.is_empty() {
        return arguments.to_string();
    }

    let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(arguments) else {
        return arguments.to_string();
    };
    if matches!(object.get("pages"), Some(Value::String(value)) if value.is_empty()) {
        object.remove("pages");
    }
    serde_json::to_string(&Value::Object(object)).unwrap_or_else(|_| arguments.to_string())
}

pub fn llm_request_to_responses(request: Request) -> Value {
    let mut input = Vec::new();
    let mut instructions = Vec::new();
    let mut custom_tool_call_ids = HashMap::new();
    let include = request
        .transformer_metadata
        .get(RESPONSES_INCLUDE_METADATA_KEY)
        .cloned()
        .or_else(|| {
            request
                .extra_body
                .as_ref()
                .and_then(|extra_body| extra_body.get("include").cloned())
        });
    for message in request.messages {
        if message.role == "system" || message.role == "developer" {
            if let MessageContent::Text(text) = message.content {
                if !text.is_empty() {
                    instructions.push(text);
                }
            }
            continue;
        }
        append_llm_message_as_responses_input(message, &mut input, &mut custom_tool_call_ids);
    }
    let mut body = json!({
        "model": request.model,
        "input": input,
    });
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions.join("\n\n"));
    }
    if let Some(max_tokens) = request.max_tokens.or(request.max_completion_tokens) {
        body["max_output_tokens"] = json!(max_tokens);
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
    if let Some(service_tier) = request.service_tier {
        body["service_tier"] = json!(service_tier);
    }
    if let Some(top_logprobs) = request.top_logprobs {
        body["top_logprobs"] = json!(top_logprobs);
    }
    if let Some(user) = request.user {
        body["user"] = json!(user);
    }
    if let Some(reasoning_effort) = request.reasoning_effort {
        body["reasoning"] = json!({ "effort": reasoning_effort });
    }
    if let Some(stream) = request.stream {
        body["stream"] = json!(stream);
    }
    if let Some(stop) = stop_to_value(request.stop) {
        body["stop"] = stop;
    }
    let tools = request
        .tools
        .into_iter()
        .filter_map(|tool| {
            if tool.tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
                return tool.response_custom_tool.and_then(|custom| {
                    (!custom.name.is_empty()).then(|| {
                        json!({
                            "type": "custom",
                            "name": custom.name,
                            "description": custom.description,
                            "format": custom.format
                        })
                    })
                });
            }
            if let Some(function) = tool.function {
                if function.name.is_empty() {
                    return None;
                }
                let strict = function.strict;
                let mut tool_object = Map::new();
                tool_object.insert("type".to_string(), json!("function"));
                tool_object.insert("name".to_string(), json!(function.name));
                tool_object.insert("description".to_string(), json!(function.description));
                tool_object.insert(
                    "parameters".to_string(),
                    responses_function_parameters(function.parameters, strict),
                );
                if let Some(strict) = strict {
                    tool_object.insert("strict".to_string(), json!(strict));
                }
                return Some(Value::Object(tool_object));
            }
            None
        })
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(tool_choice) = tool_choice_to_responses(request.tool_choice) {
        body["tool_choice"] = tool_choice;
    }
    if let Some(parallel_tool_calls) = request.parallel_tool_calls {
        body["parallel_tool_calls"] = json!(parallel_tool_calls);
    }
    if request.response_format.is_some() || request.verbosity.is_some() {
        let mut text = Map::new();
        if let Some(response_format) = request.response_format {
            text.insert(
                "format".to_string(),
                response_format_to_responses_format(response_format),
            );
        }
        if let Some(verbosity) = request.verbosity {
            text.insert("verbosity".to_string(), json!(verbosity));
        }
        body["text"] = Value::Object(text);
    }
    if let Some(prompt_cache_key) = request.prompt_cache_key {
        body["prompt_cache_key"] = json!(prompt_cache_key);
    }
    if should_emit_openai_request_metadata(request.api_format) && !request.metadata.is_empty() {
        body["metadata"] = json!(request.metadata);
    }
    if let Some(include) = include {
        body["include"] = include;
    }
    body
}

fn append_llm_message_as_responses_input(
    message: Message,
    input: &mut Vec<Value>,
    custom_tool_call_ids: &mut HashMap<String, bool>,
) {
    if let Some(reasoning_item) = responses_reasoning_item_from_message(&message) {
        input.push(reasoning_item);
    }
    if message.role == "tool" {
        let call_id = message.tool_call_id.unwrap_or_default();
        let output_type = if custom_tool_call_ids.contains_key(&call_id) {
            "custom_tool_call_output"
        } else {
            "function_call_output"
        };
        input.push(json!({
            "type": output_type,
            "call_id": call_id,
            "output": match message.content {
                MessageContent::Text(text) => text,
                other => serde_json::to_string(&other).unwrap_or_default(),
            }
        }));
        return;
    }
    let role = message.role;
    let content = llm_content_to_responses_content(
        message.content,
        role == "assistant",
        message.annotations,
        message.refusal,
    );
    if !content.is_empty() {
        input.push(json!({
            "type": "message",
            "role": role,
            "content": content
        }));
    }
    for tool_call in message.tool_calls {
        if tool_call.tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
            custom_tool_call_ids.insert(tool_call.id.clone(), true);
        }
        input.push(tool_call_to_responses_item(tool_call));
    }
}

fn responses_reasoning_item_from_message(message: &Message) -> Option<Value> {
    let reasoning = message
        .reasoning_content
        .as_deref()
        .or(message.reasoning.as_deref())
        .filter(|reasoning| !reasoning.is_empty());
    let encrypted_content = message
        .reasoning_signature
        .as_deref()
        .and_then(|signature| decode_signature_for(SignatureProvider::OpenAiResponses, signature));
    if reasoning.is_none() && encrypted_content.is_none() {
        return None;
    }
    let mut item = json!({
        "type": "reasoning",
        "summary": reasoning
            .map(|reasoning| vec![json!({"type": "summary_text", "text": reasoning})])
            .unwrap_or_default()
    });
    if let Some(encrypted_content) = encrypted_content {
        item["encrypted_content"] = json!(encrypted_content);
    }
    Some(item)
}

fn llm_content_to_responses_content(
    content: MessageContent,
    assistant: bool,
    annotations: Vec<Value>,
    refusal: String,
) -> Vec<Value> {
    let text_type = if assistant {
        "output_text"
    } else {
        "input_text"
    };
    let mut result = match content {
        MessageContent::Text(text) => {
            vec![text_content_item(text_type, text, assistant, &annotations)]
        }
        MessageContent::Parts(parts) => parts
            .into_iter()
            .filter_map(|part| match part.part_type.as_str() {
                "text" | "input_text" | "output_text" => Some(text_content_item(
                    text_type,
                    part.text.unwrap_or_default(),
                    assistant,
                    &annotations,
                )),
                "image_url" | "input_image" => Some(json!({
                    "type": "input_image",
                    "image_url": part.image_url.map(|image| image.url).unwrap_or_default()
                })),
                _ => None,
            })
            .collect(),
        MessageContent::Empty => Vec::new(),
    };
    if assistant && !refusal.is_empty() {
        result.push(json!({ "type": "refusal", "refusal": refusal }));
    }
    result
}

fn tool_call_to_responses_item(tool_call: ToolCall) -> Value {
    if tool_call.tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
        let custom = tool_call
            .response_custom_tool_call
            .unwrap_or(ResponseCustomToolCall {
                call_id: tool_call.id.clone(),
                name: tool_call.function.name.clone(),
                input: tool_call.function.arguments.clone(),
            });
        let call_id = custom.call_id;
        return json!({
            "type": "custom_tool_call",
            "id": responses_custom_tool_call_item_id(&call_id),
            "call_id": call_id,
            "name": custom.name,
            "input": custom.input,
            "status": "completed"
        });
    }
    let call_id = tool_call.id;
    json!({
        "type": "function_call",
        "id": responses_function_call_item_id(&call_id),
        "call_id": call_id,
        "name": tool_call.function.name,
        "arguments": tool_call.function.arguments,
        "status": "completed"
    })
}

fn responses_function_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("fc") {
        call_id.to_string()
    } else if call_id.is_empty() {
        "fc_0".to_string()
    } else {
        format!("fc_{call_id}")
    }
}

fn responses_custom_tool_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("ctc") {
        call_id.to_string()
    } else if call_id.is_empty() {
        "ctc_0".to_string()
    } else {
        format!("ctc_{call_id}")
    }
}

pub fn responses_response_to_llm(body: Value) -> Response {
    let mut message = Message {
        role: "assistant".to_string(),
        ..Default::default()
    };
    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();
    if let Some(output) = body.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(content) = item.get("content").and_then(Value::as_array) {
                        for part in content {
                            match part.get("type").and_then(Value::as_str) {
                                Some("output_text") => {
                                    if let Some(annotations) =
                                        part.get("annotations").and_then(Value::as_array)
                                    {
                                        message.annotations.extend(annotations.iter().cloned());
                                    }
                                    parts.push(MessageContentPart {
                                        part_type: "text".to_string(),
                                        text: part
                                            .get("text")
                                            .and_then(Value::as_str)
                                            .map(ToString::to_string),
                                        ..Default::default()
                                    });
                                }
                                Some("refusal") => {
                                    message.refusal = part
                                        .get("refusal")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string();
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Some("function_call") | Some("custom_tool_call") => {
                    let index = tool_calls.len();
                    tool_calls.push(responses_call_to_tool_call(item, index));
                }
                Some("input_image") => {
                    if let Some(part) = responses_input_image_part(item) {
                        parts.push(part);
                    }
                }
                Some("reasoning") => {
                    let reasoning = responses_reasoning_text(item);
                    message.reasoning_content = reasoning.clone();
                    message.reasoning = reasoning;
                    message.reasoning_signature = item
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .filter(|signature| !signature.is_empty())
                        .map(|signature| {
                            encode_signature(SignatureProvider::OpenAiResponses, signature)
                        });
                }
                _ => {}
            }
        }
    }
    message.content = MessageContent::Parts(parts);
    message.tool_calls = tool_calls;
    let has_tool = !message.tool_calls.is_empty();
    Response {
        id: body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        object: "response".to_string(),
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created: body
            .get("created_at")
            .or_else(|| body.get("created"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        previous_response_id: body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        choices: vec![Choice {
            index: 0,
            message,
            finish_reason: Some(responses_status_to_finish(
                body.get("status").and_then(Value::as_str),
                has_tool,
            )),
            ..Default::default()
        }],
        usage: Some(responses_usage_to_llm(body.get("usage"))),
        ..Default::default()
    }
}

pub fn llm_response_to_responses(response: Response) -> Value {
    let previous_response_id = response.previous_response_id.clone();
    let choice = response.choices.first().cloned().unwrap_or_default();
    let mut output = Vec::new();
    if let Some(reasoning_item) = responses_reasoning_item_from_message(&choice.message) {
        output.push(reasoning_item);
    }
    if !choice.message.content.is_empty() {
        output.push(json!({
            "type": "message",
            "role": "assistant",
            "content": llm_content_to_responses_content(
                choice.message.content.clone(),
                true,
                choice.message.annotations.clone(),
                choice.message.refusal.clone()
            )
        }));
    } else if !choice.message.refusal.is_empty() {
        output.push(json!({
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "refusal",
                "refusal": choice.message.refusal
            }]
        }));
    }
    for tool_call in choice.message.tool_calls {
        output.push(tool_call_to_responses_item(tool_call));
    }
    let mut body = json!({
        "id": response.id,
        "object": "response",
        "created_at": response.created,
        "status": finish_to_responses_status(choice.finish_reason.as_deref()),
        "model": response.model,
        "output": output,
        "usage": usage_to_responses(response.usage.as_ref())
    });
    if let Some(previous_response_id) = previous_response_id {
        body["previous_response_id"] = json!(previous_response_id);
    }
    body
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

fn responses_format_to_chat(format: Value) -> Value {
    if format.get("type").and_then(Value::as_str) != Some("json_schema") {
        return format;
    }
    let mut result = Map::new();
    result.insert("type".to_string(), json!("json_schema"));

    let mut schema = Map::new();
    for key in ["name", "description", "schema", "strict"] {
        if let Some(value) = format.get(key) {
            schema.insert(key.to_string(), value.clone());
        }
    }
    result.insert("json_schema".to_string(), Value::Object(schema));
    Value::Object(result)
}

fn response_format_to_responses_format(response_format: Value) -> Value {
    if response_format.get("type").and_then(Value::as_str) != Some("json_schema") {
        return response_format;
    }
    if let Some(json_schema) = response_format
        .get("json_schema")
        .and_then(Value::as_object)
    {
        let mut result = Map::new();
        result.insert("type".to_string(), json!("json_schema"));
        for key in ["name", "description", "schema", "strict"] {
            if let Some(value) = json_schema.get(key) {
                result.insert(key.to_string(), value.clone());
            }
        }
        return Value::Object(result);
    }
    response_format
}

fn responses_function_parameters(parameters: Option<Value>, strict: Option<bool>) -> Value {
    let mut parameters = parameters.unwrap_or_else(|| json!({}));
    let Some(object) = parameters.as_object_mut() else {
        return parameters;
    };

    if object.get("type").and_then(Value::as_str) == Some("object")
        && !object.contains_key("properties")
    {
        object.insert("properties".to_string(), json!({}));
    }

    if strict == Some(true) {
        object.insert("additionalProperties".to_string(), json!(false));
        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            let mut required = object
                .get("required")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for key in properties.keys() {
                if !required.iter().any(|existing| existing == key) {
                    required.push(key.clone());
                }
            }
            if !required.is_empty() {
                object.insert("required".to_string(), json!(required));
            }
        }
    }

    parameters
}

fn text_content_item(
    text_type: &str,
    text: String,
    assistant: bool,
    annotations: &[Value],
) -> Value {
    let mut item = json!({
        "type": text_type,
        "text": text
    });
    if assistant && !annotations.is_empty() {
        item["annotations"] = json!(annotations);
    }
    item
}

fn content_annotations(content: &[Value]) -> Vec<Value> {
    content
        .iter()
        .flat_map(|part| {
            part.get("annotations")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

fn responses_reasoning_text(item: &Value) -> Option<String> {
    item.get("summary")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|summary| summary.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty())
}

fn responses_usage_to_llm(usage: Option<&Value>) -> Usage {
    openai_usage_to_llm(usage)
}

fn usage_to_responses(usage: Option<&Usage>) -> Value {
    let usage = usage.cloned().unwrap_or_default();
    json!({
        "input_tokens": usage.prompt_tokens,
        "output_tokens": usage.completion_tokens,
        "total_tokens": if usage.total_tokens == 0 {
            usage.prompt_tokens.saturating_add(usage.completion_tokens)
        } else {
            usage.total_tokens
        },
        "input_tokens_details": {
            "cached_tokens": usage.cached_tokens
        },
        "output_tokens_details": {
            "reasoning_tokens": usage.reasoning_tokens
        }
    })
}

fn responses_status_to_finish(status: Option<&str>, has_tool: bool) -> String {
    match status {
        Some("failed") => "error".to_string(),
        Some("incomplete") => "length".to_string(),
        Some("completed") if has_tool => "tool_calls".to_string(),
        Some("completed") => "stop".to_string(),
        _ => "stop".to_string(),
    }
}

fn finish_to_responses_status(reason: Option<&str>) -> &'static str {
    match reason {
        Some("error") => "failed",
        Some("length") | Some("max_tokens") => "incomplete",
        _ => "completed",
    }
}
