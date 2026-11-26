use super::super::llm::{
    ApiFormat, Choice, DocumentUrl, Function, FunctionCall, GoogleTools, ImageUrl, Message,
    MessageContent, MessageContentPart, Request, RequestType, Response, Stop, Tool, ToolCall,
    ToolChoice, Usage, TOOL_TYPE_FUNCTION, TOOL_TYPE_GOOGLE_CODE_EXECUTION,
    TOOL_TYPE_GOOGLE_SEARCH, TOOL_TYPE_GOOGLE_URL_CONTEXT,
};
use super::super::shared::signature::{
    decode_signature_for, encode_signature, metadata_signature, metadata_signature_raw,
    SignatureProvider, DEFAULT_GEMINI_THOUGHT_SIGNATURE, GEMINI_THOUGHT_SIGNATURE_METADATA_KEY,
};
use super::super::shared::tool_media::{
    cleaned_tool_result_text, inline_image_data, plan_tool_result_images, ToolImageScope,
    ToolResultMediaPlan, TOOL_RESULT_MEDIA_ATTACHED_MARKER,
};
use super::super::shared::{
    budget_tokens_to_reasoning_effort, extract_error_code, extract_error_message,
    extract_error_type, instruction_hoist_plan, json_string, placement_for_api_format,
    reasoning_effort_to_budget_tokens, stop_from_value, tool_arguments_value,
    tool_choice_from_gemini,
};
use super::{synthesize_gemini_tool_id, SYNTHETIC_GEMINI_TOOL_ID_PREFIX};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn gemini_request_to_llm(mut body: Value) -> Request {
    let mut request = Request {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stream: body.get("stream").and_then(Value::as_bool),
        request_type: Some(RequestType::Chat),
        api_format: Some(ApiFormat::GeminiContents),
        ..Default::default()
    };
    if let Some(config) = body.get("generationConfig") {
        request.max_tokens = config.get("maxOutputTokens").and_then(Value::as_i64);
        request.temperature = config.get("temperature").and_then(Value::as_f64);
        request.top_p = config.get("topP").and_then(Value::as_f64);
        request.presence_penalty = config.get("presencePenalty").and_then(Value::as_f64);
        request.frequency_penalty = config.get("frequencyPenalty").and_then(Value::as_f64);
        request.seed = config.get("seed").and_then(Value::as_i64);
        request.stop = stop_from_value(config.get("stopSequences"));
        request.response_format = gemini_response_format(config);
        request.reasoning_effort = config
            .get("thinkingConfig")
            .and_then(reasoning_effort_from_gemini_thinking_config);
    }
    request.tool_choice =
        tool_choice_from_gemini(body.pointer("/toolConfig/functionCallingConfig"));
    if let Some(system) = gemini_parts_text(
        body.pointer("/systemInstruction/parts")
            .or_else(|| body.pointer("/system_instruction/parts")),
    ) {
        request.messages.push(Message {
            role: "system".to_string(),
            content: MessageContent::Text(system),
            ..Default::default()
        });
    }
    if let Some(contents) = body.get_mut("contents").and_then(Value::as_array_mut) {
        resolve_gemini_request_tool_ids(contents);
        for content in contents {
            request
                .messages
                .extend(gemini_request_content_to_llm_messages(content));
        }
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        request.tools = tools.iter().flat_map(gemini_tool_to_llm).collect();
    }
    request
}

fn resolve_gemini_request_tool_ids(contents: &mut [Value]) {
    let mut pending_calls: Vec<(String, String)> = Vec::new();
    for content in contents {
        let is_model_turn = content.get("role").and_then(Value::as_str) == Some("model");
        let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) else {
            pending_calls.clear();
            continue;
        };
        if is_model_turn
            || !parts
                .iter()
                .any(|part| part.get("functionResponse").is_some())
        {
            pending_calls.clear();
        }
        for part in parts.iter_mut() {
            let Some(call) = part.get_mut("functionCall").and_then(Value::as_object_mut) else {
                continue;
            };
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    let id = synthesize_gemini_tool_id();
                    call.insert("id".to_string(), json!(id));
                    id
                });
            let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
            pending_calls.push((name.to_string(), call_id));
        }

        // Native IDs take precedence even when a later result is explicit and
        // an earlier result omits its ID. Never let name matching consume it.
        for part in parts.iter() {
            if let Some(id) = part
                .pointer("/functionResponse/id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                pending_calls.retain(|(_, call_id)| call_id != id);
            }
        }
        for part in parts.iter_mut() {
            let Some(response) = part
                .get_mut("functionResponse")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            if response
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
            {
                continue;
            }
            let name = response
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some(index) = pending_calls
                .iter()
                .position(|(call_name, _)| call_name == name)
            {
                // Older Gemini history may have no IDs. Match each occurrence
                // once, in call order, rather than reusing the last same-name ID.
                let (_, call_id) = pending_calls.remove(index);
                response.insert("id".to_string(), json!(call_id));
            }
        }
    }
}

fn reasoning_effort_from_gemini_thinking_config(config: &Value) -> Option<String> {
    if config.get("includeThoughts").and_then(Value::as_bool) == Some(false) {
        return Some("none".to_string());
    }
    if let Some(level) = config.get("thinkingLevel").and_then(Value::as_str) {
        return match level.to_ascii_lowercase().as_str() {
            "none" => Some("none".to_string()),
            "minimal" | "low" => Some("low".to_string()),
            "medium" => Some("medium".to_string()),
            "high" => Some("high".to_string()),
            _ => None,
        };
    }
    let budget = config.get("thinkingBudget").and_then(Value::as_i64)?;
    Some(budget_tokens_to_reasoning_effort(budget).to_string())
}

fn gemini_tool_to_llm(tool: &Value) -> Vec<Tool> {
    let mut tools = Vec::new();
    if let Some(declarations) = tool.get("functionDeclarations").and_then(Value::as_array) {
        tools.extend(declarations.iter().filter_map(|declaration| {
            let name = declaration.get("name").and_then(Value::as_str)?;
            Some(Tool {
                tool_type: TOOL_TYPE_FUNCTION.to_string(),
                function: Some(Function {
                    name: name.to_string(),
                    description: declaration
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    parameters: declaration
                        .get("parametersJsonSchema")
                        .or_else(|| declaration.get("parameters"))
                        .cloned()
                        .map(normalize_gemini_schema),
                    ..Default::default()
                }),
                ..Default::default()
            })
        }));
    }
    if let Some(search) = tool.get("googleSearch") {
        tools.push(Tool {
            tool_type: TOOL_TYPE_GOOGLE_SEARCH.to_string(),
            google: Some(GoogleTools {
                search: Some(search.clone()),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    if let Some(code_execution) = tool.get("codeExecution") {
        tools.push(Tool {
            tool_type: TOOL_TYPE_GOOGLE_CODE_EXECUTION.to_string(),
            google: Some(GoogleTools {
                code_execution: Some(code_execution.clone()),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    if let Some(url_context) = tool.get("urlContext") {
        tools.push(Tool {
            tool_type: TOOL_TYPE_GOOGLE_URL_CONTEXT.to_string(),
            google: Some(GoogleTools {
                url_context: Some(url_context.clone()),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    tools
}

fn normalize_gemini_schema(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(normalize_gemini_schema)
                .collect::<Vec<_>>(),
        ),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    let normalized = if key == "type" {
                        value
                            .as_str()
                            .map(|text| json!(text.to_ascii_lowercase()))
                            .unwrap_or_else(|| normalize_gemini_schema(value))
                    } else {
                        normalize_gemini_schema(value)
                    };
                    (key, normalized)
                })
                .collect(),
        ),
        other => other,
    }
}

fn gemini_parts_text(parts: Option<&Value>) -> Option<String> {
    let text = parts
        .and_then(Value::as_array)?
        .iter()
        .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn gemini_request_content_to_llm_messages(content: &Value) -> Vec<Message> {
    let Some(parts) = content.get("parts").and_then(Value::as_array) else {
        return vec![gemini_content_to_llm(content)];
    };
    let function_response_count = parts
        .iter()
        .filter(|part| part.get("functionResponse").is_some())
        .count();
    if function_response_count <= 1 {
        return vec![gemini_content_to_llm(content)];
    }

    let mut messages = Vec::with_capacity(function_response_count);
    let mut segment_parts = Vec::new();
    for part in parts {
        if part.get("functionResponse").is_some() {
            if !segment_parts.is_empty() {
                messages.push(gemini_content_segment_to_llm(
                    content,
                    std::mem::take(&mut segment_parts),
                ));
            }
        }
        segment_parts.push(part.clone());
    }
    if !segment_parts.is_empty() {
        messages.push(gemini_content_segment_to_llm(content, segment_parts));
    }
    messages
}

fn gemini_content_segment_to_llm(content: &Value, parts: Vec<Value>) -> Message {
    let mut segment = content.clone();
    if let Some(object) = segment.as_object_mut() {
        object.insert("parts".to_string(), Value::Array(parts));
    }
    gemini_content_to_llm(&segment)
}

fn gemini_content_to_llm(content: &Value) -> Message {
    let role = match content.get("role").and_then(Value::as_str) {
        Some("model") => "assistant",
        _ => "user",
    }
    .to_string();
    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_result: Option<Message> = None;
    let mut reasoning_chunks = Vec::new();
    let mut reasoning_signature = None;
    if let Some(gemini_parts) = content.get("parts").and_then(Value::as_array) {
        for (index, part) in gemini_parts.iter().enumerate() {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        reasoning_chunks.push(text.to_string());
                    }
                }
                if let Some(signature) = gemini_part_thought_signature(part) {
                    reasoning_signature =
                        Some(encode_signature(SignatureProvider::Gemini, signature));
                }
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                parts.push(MessageContentPart {
                    part_type: "text".to_string(),
                    text: Some(text.to_string()),
                    ..Default::default()
                });
            }
            if let Some(inline_data) = part.get("inlineData").or_else(|| part.get("inline_data")) {
                let mime = inline_data
                    .get("mimeType")
                    .or_else(|| inline_data.get("mime_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("image/png");
                let data = inline_data
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                parts.push(gemini_media_part_to_llm(
                    mime,
                    format!("data:{mime};base64,{data}"),
                ));
            }
            if let Some(file_data) = part.get("fileData").or_else(|| part.get("file_data")) {
                let uri = file_data
                    .get("fileUri")
                    .or_else(|| file_data.get("file_uri"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !uri.is_empty() {
                    let mime = file_data
                        .get("mimeType")
                        .or_else(|| file_data.get("mime_type"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    parts.push(gemini_media_part_to_llm(mime, uri.to_string()));
                }
            }
            if let Some(function_call) = part.get("functionCall") {
                let mut transformer_metadata = HashMap::new();
                if let Some(signature) = gemini_part_thought_signature(part) {
                    let encoded = encode_signature(SignatureProvider::Gemini, signature);
                    transformer_metadata.insert(
                        GEMINI_THOUGHT_SIGNATURE_METADATA_KEY.to_string(),
                        metadata_signature(&encoded),
                    );
                    if reasoning_signature.is_none() && tool_calls.is_empty() {
                        reasoning_signature = Some(encoded);
                    }
                }
                tool_calls.push(ToolCall {
                    id: function_call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(synthesize_gemini_tool_id),
                    tool_type: TOOL_TYPE_FUNCTION.to_string(),
                    function: FunctionCall {
                        name: function_call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: json_string(function_call.get("args").unwrap_or(&json!({}))),
                    },
                    index,
                    transformer_metadata,
                    ..Default::default()
                });
            }
            if let Some(function_response) = part.get("functionResponse") {
                let function_name = function_response
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tool_call_id = function_response
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| (!function_name.is_empty()).then(|| function_name.clone()));
                tool_result = Some(Message {
                    role: "tool".to_string(),
                    tool_call_id,
                    tool_call_name: (!function_name.is_empty()).then_some(function_name),
                    content: gemini_function_response_content(function_response),
                    ..Default::default()
                });
            }
        }
    }
    let reasoning = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
    if let Some(mut tool_result) = tool_result {
        if !parts.is_empty() {
            let mut combined = match tool_result.content {
                MessageContent::Text(text) if !text.is_empty() => vec![MessageContentPart {
                    part_type: "text".to_string(),
                    text: Some(text),
                    ..Default::default()
                }],
                MessageContent::Parts(parts) => parts,
                _ => Vec::new(),
            };
            combined.extend(parts);
            tool_result.content = MessageContent::Parts(combined);
        }
        return tool_result;
    }
    Message {
        role,
        content: MessageContent::Parts(parts),
        tool_calls,
        reasoning_content: reasoning.clone(),
        reasoning,
        reasoning_signature,
        ..Default::default()
    }
}

fn gemini_part_thought_signature(part: &Value) -> Option<&str> {
    part.get("thoughtSignature")
        .or_else(|| part.get("thought_signature"))
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
}

fn gemini_media_part_to_llm(mime: &str, url: String) -> MessageContentPart {
    if is_gemini_document_mime(mime) {
        return MessageContentPart {
            part_type: "document".to_string(),
            document: Some(DocumentUrl {
                url,
                mime_type: mime.to_string(),
            }),
            ..Default::default()
        };
    }

    MessageContentPart {
        part_type: "image_url".to_string(),
        image_url: Some(ImageUrl { url, detail: None }),
        ..Default::default()
    }
}

fn is_gemini_document_mime(mime: &str) -> bool {
    let normalized = mime.to_ascii_lowercase();
    normalized.starts_with("application/pdf")
        || normalized.starts_with("application/msword")
        || normalized.starts_with("application/vnd.openxmlformats-officedocument")
        || normalized.starts_with("application/vnd.ms-")
        || normalized.starts_with("text/")
}

fn gemini_function_response_text(response: Option<&Value>) -> String {
    match response {
        Some(Value::String(text)) => text.clone(),
        Some(value) => value
            .get("content")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default()),
        None => String::new(),
    }
}

fn gemini_function_response_content(function_response: &Value) -> MessageContent {
    let response_text = gemini_function_response_text(function_response.get("response"));
    let Some(media_parts) = function_response.get("parts").and_then(Value::as_array) else {
        return MessageContent::Text(response_text);
    };

    let mut parts = Vec::new();
    if !response_text.is_empty() {
        parts.push(MessageContentPart {
            part_type: "text".to_string(),
            text: Some(response_text.clone()),
            ..Default::default()
        });
    }
    for part in media_parts {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            parts.push(MessageContentPart {
                part_type: "text".to_string(),
                text: Some(text.to_string()),
                ..Default::default()
            });
            continue;
        }
        if let Some(inline_data) = part.get("inlineData").or_else(|| part.get("inline_data")) {
            let mime = inline_data
                .get("mimeType")
                .or_else(|| inline_data.get("mime_type"))
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            let data = inline_data
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();
            parts.push(gemini_media_part_to_llm(
                mime,
                format!("data:{mime};base64,{data}"),
            ));
            continue;
        }
        if let Some(file_data) = part.get("fileData").or_else(|| part.get("file_data")) {
            let uri = file_data
                .get("fileUri")
                .or_else(|| file_data.get("file_uri"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !uri.is_empty() {
                let mime = file_data
                    .get("mimeType")
                    .or_else(|| file_data.get("mime_type"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                parts.push(gemini_media_part_to_llm(mime, uri.to_string()));
            }
        }
    }

    if parts.is_empty() {
        MessageContent::Text(response_text)
    } else {
        MessageContent::Parts(parts)
    }
}

pub fn llm_request_to_gemini(request: Request) -> Value {
    let mut system_chunks = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut tool_names_by_id = HashMap::new();
    let mut tool_call_order = HashMap::new();
    let mut pending_tool_results = Vec::new();
    let mut deferred_mid_system_messages: Vec<String> = Vec::new();
    let supports_multimodal_function_response = is_gemini_3_or_newer_model(&request.model);
    let placement = placement_for_api_format(request.api_format);
    let hoist_plan = instruction_hoist_plan(&request.messages, placement);
    let mut awaiting_tool_responses = false;
    for (msg_index, message) in request.messages.into_iter().enumerate() {
        if message.role == "tool" {
            pending_tool_results.push(message);
            continue;
        }
        if message.role == "system" || message.role == "developer" {
            let text = match message.content {
                MessageContent::Empty => String::new(),
                MessageContent::Text(text) => text,
                MessageContent::Parts(parts) => parts
                    .into_iter()
                    .filter_map(|part| part.text)
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            if text.is_empty() {
                continue;
            }
            if hoist_plan.get(msg_index).copied().unwrap_or(true) {
                system_chunks.push(text);
            } else if awaiting_tool_responses || !pending_tool_results.is_empty() {
                // Keep mid-conversation system reminders after functionResponse
                // so model(functionCall) -> user(functionResponse) stays adjacent
                // even when a reminder appears between parallel tool results
                // (cc-switch da830bfd).
                deferred_mid_system_messages.push(text);
            } else {
                contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": text }]
                }));
            }
            continue;
        }
        if !pending_tool_results.is_empty() {
            append_gemini_tool_result_batch(
                &mut contents,
                &mut pending_tool_results,
                &tool_names_by_id,
                &tool_call_order,
                supports_multimodal_function_response,
            );
            for text in deferred_mid_system_messages.drain(..) {
                contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": text }]
                }));
            }
        }
        if !deferred_mid_system_messages.is_empty() {
            for text in deferred_mid_system_messages.drain(..) {
                contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": text }]
                }));
            }
        }
        awaiting_tool_responses = !message.tool_calls.is_empty();
        for (index, tool_call) in message.tool_calls.iter().enumerate() {
            if !tool_call.id.is_empty() && !tool_call.function.name.is_empty() {
                tool_names_by_id.insert(tool_call.id.clone(), tool_call.function.name.clone());
                tool_call_order.insert(tool_call.id.clone(), index);
            }
        }
        contents.push(llm_message_to_gemini_content(message));
    }
    append_gemini_tool_result_batch(
        &mut contents,
        &mut pending_tool_results,
        &tool_names_by_id,
        &tool_call_order,
        supports_multimodal_function_response,
    );
    for text in deferred_mid_system_messages.drain(..) {
        contents.push(json!({
            "role": "user",
            "parts": [{ "text": text }]
        }));
    }
    let mut body = json!({
        "contents": contents
    });
    if !system_chunks.is_empty() {
        body["systemInstruction"] = json!({
            "parts": [{ "text": system_chunks.join("\n\n") }]
        });
    }
    let mut generation_config = Map::new();
    let resolved_max_tokens = request.max_tokens.or(request.max_completion_tokens);
    if let Some(max_tokens) = resolved_max_tokens {
        generation_config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        generation_config.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        generation_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(presence_penalty) = request.presence_penalty {
        generation_config.insert("presencePenalty".to_string(), json!(presence_penalty));
    }
    if let Some(frequency_penalty) = request.frequency_penalty {
        generation_config.insert("frequencyPenalty".to_string(), json!(frequency_penalty));
    }
    if let Some(seed) = request.seed {
        generation_config.insert("seed".to_string(), json!(seed));
    }
    if let Some(reasoning_effort) = &request.reasoning_effort {
        if let Some(thinking_config) =
            gemini_thinking_config(&request.model, reasoning_effort, resolved_max_tokens)
        {
            generation_config.insert("thinkingConfig".to_string(), thinking_config);
        }
    }
    if let Some(stop_sequences) = gemini_stop_sequences(request.stop) {
        generation_config.insert("stopSequences".to_string(), json!(stop_sequences));
    }
    if let Some(response_format) = request.response_format {
        match response_format.get("type").and_then(Value::as_str) {
            Some("json_schema") => {
                generation_config.insert("responseMimeType".to_string(), json!("application/json"));
                if let Some(schema) = response_format_schema_for_gemini(&response_format) {
                    generation_config.insert("responseJsonSchema".to_string(), schema);
                }
            }
            Some("json_object") => {
                generation_config.insert("responseMimeType".to_string(), json!("application/json"));
            }
            _ => {}
        }
    }
    if !generation_config.is_empty() {
        body["generationConfig"] = Value::Object(generation_config);
    }
    if let Some(tool_config) = gemini_tool_config(request.tool_choice) {
        body["toolConfig"] = tool_config;
    }
    if !request.tools.is_empty() {
        let mut tools = Vec::new();
        let function_declarations = request
            .tools
            .iter()
            .filter_map(|tool| {
                let function = tool.function.as_ref()?;
                let parameters = function
                    .parameters_json_schema
                    .clone()
                    .or_else(|| function.parameters.clone())
                    .map(gemini_function_parameters_value)
                    .unwrap_or_else(|| {
                        GeminiFunctionParameters::Schema(empty_gemini_function_parameters())
                    });
                let mut declaration = Map::new();
                declaration.insert("name".to_string(), json!(function.name));
                declaration.insert("description".to_string(), json!(function.description));
                match parameters {
                    GeminiFunctionParameters::Schema(schema) => {
                        declaration.insert("parameters".to_string(), schema);
                    }
                    GeminiFunctionParameters::JsonSchema(schema) => {
                        declaration.insert("parametersJsonSchema".to_string(), schema);
                    }
                }
                Some(Value::Object(declaration))
            })
            .collect::<Vec<_>>();
        if !function_declarations.is_empty() {
            tools.push(json!({ "functionDeclarations": function_declarations }));
        }
        for tool in request.tools {
            match tool.tool_type.as_str() {
                TOOL_TYPE_GOOGLE_SEARCH => tools.push(json!({
                    "googleSearch": tool
                        .google
                        .and_then(|google| google.search)
                        .unwrap_or_else(|| json!({}))
                })),
                TOOL_TYPE_GOOGLE_CODE_EXECUTION => tools.push(json!({
                    "codeExecution": tool
                        .google
                        .and_then(|google| google.code_execution)
                        .unwrap_or_else(|| json!({}))
                })),
                TOOL_TYPE_GOOGLE_URL_CONTEXT => tools.push(json!({
                    "urlContext": tool
                        .google
                        .and_then(|google| google.url_context)
                        .unwrap_or_else(|| json!({}))
                })),
                _ => {}
            }
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }
    body
}

enum GeminiFunctionParameters {
    Schema(Value),
    JsonSchema(Value),
}

fn gemini_function_parameters_value(parameters: Value) -> GeminiFunctionParameters {
    match parameters {
        Value::Object(object) if object.is_empty() => {
            GeminiFunctionParameters::Schema(empty_gemini_function_parameters())
        }
        other if requires_parameters_json_schema(&other) => {
            GeminiFunctionParameters::JsonSchema(strip_json_schema_meta_keywords(other))
        }
        other => GeminiFunctionParameters::Schema(other),
    }
}

fn empty_gemini_function_parameters() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn requires_parameters_json_schema(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(requires_parameters_json_schema),
        Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "$schema"
                    | "$ref"
                    | "$defs"
                    | "definitions"
                    | "additionalProperties"
                    | "unevaluatedProperties"
                    | "patternProperties"
                    | "oneOf"
                    | "allOf"
                    | "anyOf"
                    | "const"
                    | "not"
                    | "if"
                    | "then"
                    | "else"
                    | "dependentRequired"
                    | "dependentSchemas"
                    | "contains"
                    | "minContains"
                    | "maxContains"
                    | "prefixItems"
                    | "exclusiveMinimum"
                    | "exclusiveMaximum"
                    | "multipleOf"
                    | "examples"
            ) || requires_parameters_json_schema(value)
        }),
        _ => false,
    }
}

fn strip_json_schema_meta_keywords(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(strip_json_schema_meta_keywords)
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter_map(|(key, value)| {
                    (key != "$schema").then(|| (key, strip_json_schema_meta_keywords(value)))
                })
                .collect(),
        ),
        other => other,
    }
}

fn gemini_thinking_config(
    model: &str,
    reasoning_effort: &str,
    max_tokens: Option<i64>,
) -> Option<Value> {
    if should_skip_gemini_thinking_config(model) {
        return None;
    }
    let budget_tokens = reasoning_effort_to_budget_tokens(reasoning_effort, max_tokens)?;
    if is_gemini_3_or_newer_model(model) {
        return Some(json!({
            "includeThoughts": budget_tokens > 0,
            "thinkingLevel": gemini_3_thinking_level(model, reasoning_effort, budget_tokens)
        }));
    }
    let mut effective_budget = budget_tokens.min(24576);
    if is_gemini_2_5_pro_model(model) {
        // Gemini 2.5 Pro cannot disable thinking and rejects thinkingBudget < 128
        // (cc-switch 93d0bf18).
        effective_budget = effective_budget.max(128);
    }
    Some(json!({
        "includeThoughts": budget_tokens > 0,
        "thinkingBudget": effective_budget
    }))
}

fn normalized_gemini_model_tail(model: &str) -> String {
    model
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase()
}

fn should_skip_gemini_thinking_config(model: &str) -> bool {
    let tail = normalized_gemini_model_tail(model);
    if tail.is_empty() {
        return false;
    }
    // Non-thinking Gemini 1.x / 2.0 (non-thinking) and media/embedding/live variants reject thinkingConfig.
    if tail.contains("gemini-1.")
        || (tail.contains("gemini-2.0") && !tail.contains("thinking"))
    {
        return true;
    }
    if [
        "-image",
        "image-",
        "-audio",
        "-live",
        "embedding",
        "-tts",
        "imagen",
    ]
    .iter()
    .any(|marker| tail.contains(marker))
    {
        return true;
    }
    // Relay aliases that already bake a thinking level into the model ID
    // (e.g. `gemini-3-pro-high`, `gemini-2.5-pro-low`) must not also receive
    // an overriding `thinkingConfig` (cc-switch 93d0bf18).
    [
        "-minimal",
        "-low",
        "-medium",
        "-high",
        "-xhigh",
        "-max",
        "-none",
        "-off",
        "-nothinking",
        "-no-thinking",
    ]
    .iter()
    .any(|suffix| tail.ends_with(suffix))
}

fn is_gemini_3_or_newer_model(model: &str) -> bool {
    let tail = normalized_gemini_model_tail(model);
    tail.contains("gemini-3") || tail.contains("gemini-4")
}

fn is_gemini_2_5_pro_model(model: &str) -> bool {
    let tail = normalized_gemini_model_tail(model);
    tail.contains("gemini-2.5-pro")
}

fn gemini_3_thinking_level(model: &str, reasoning_effort: &str, budget_tokens: i64) -> &'static str {
    let tail = normalized_gemini_model_tail(model);
    let is_pro = tail.contains("-pro");
    let accepts_minimal = tail.contains("flash");
    match reasoning_effort.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "disabled" => {
            if accepts_minimal {
                "minimal"
            } else {
                "low"
            }
        }
        "minimal" => {
            if accepts_minimal {
                "minimal"
            } else {
                "low"
            }
        }
        "low" => "low",
        "medium" => {
            if is_pro {
                "high"
            } else {
                "medium"
            }
        }
        "high" | "xhigh" | "extra_high" | "max" => "high",
        _ if budget_tokens <= 0 => {
            if accepts_minimal {
                "minimal"
            } else {
                "low"
            }
        }
        _ if budget_tokens <= 4096 => "low",
        _ if budget_tokens <= 10240 => {
            if is_pro {
                "high"
            } else {
                "medium"
            }
        }
        _ => "high",
    }
}

fn gemini_response_format(config: &Value) -> Option<Value> {
    if let Some(schema) = config
        .get("responseJsonSchema")
        .or_else(|| config.get("responseSchema"))
        .cloned()
    {
        return Some(json!({
            "type": "json_schema",
            "json_schema": {
                "schema": schema
            }
        }));
    }

    let mime = config.get("responseMimeType").and_then(Value::as_str)?;
    (mime == "application/json").then(|| json!({ "type": "json_object" }))
}

fn response_format_schema_for_gemini(response_format: &Value) -> Option<Value> {
    response_format
        .pointer("/json_schema/schema")
        .or_else(|| response_format.get("schema"))
        .or_else(|| response_format.get("json_schema"))
        .cloned()
}

fn llm_message_to_gemini_content(message: Message) -> Value {
    let role = if message.role == "assistant" {
        "model"
    } else {
        "user"
    };
    let mut parts = Vec::new();
    if message.role == "tool" {
        return llm_tool_message_to_gemini_content(message, &HashMap::new(), false);
    }
    let message_signature = message
        .reasoning_signature
        .as_deref()
        .and_then(|signature| decode_signature_for(SignatureProvider::Gemini, signature));
    let has_tool_calls = !message.tool_calls.is_empty();
    let mut emitted_signature = false;

    if let Some(reasoning) = message
        .reasoning_content
        .as_deref()
        .or(message.reasoning.as_deref())
    {
        if !reasoning.is_empty() {
            let mut part = json!({ "text": reasoning, "thought": true });
            if !has_tool_calls {
                let signature = message_signature
                    .clone()
                    .unwrap_or_else(|| DEFAULT_GEMINI_THOUGHT_SIGNATURE.to_string());
                part["thoughtSignature"] = json!(signature);
                emitted_signature = true;
            }
            parts.push(part);
        }
    }
    match message.content {
        MessageContent::Text(text) => {
            if !text.is_empty() {
                parts.push(json!({ "text": text }));
            }
        }
        MessageContent::Parts(llm_parts) => {
            for part in llm_parts {
                match part.part_type.as_str() {
                    "text" | "input_text" | "output_text" => {
                        if let Some(text) = part.text {
                            parts.push(json!({ "text": text }));
                        }
                    }
                    "image_url" | "input_image" => {
                        if let Some(image) = part.image_url {
                            if let Some((mime, data)) = image
                                .url
                                .strip_prefix("data:")
                                .and_then(|rest| rest.split_once(";base64,"))
                            {
                                parts.push(json!({
                                    "inlineData": {
                                        "mimeType": mime,
                                        "data": data
                                    }
                                }));
                            } else if !image.url.is_empty() {
                                parts.push(json!({
                                    "fileData": {
                                        "fileUri": image.url
                                    }
                                }));
                            }
                        }
                    }
                    "document" => {
                        if let Some(document) = part.document {
                            if let Some((mime, data)) = document
                                .url
                                .strip_prefix("data:")
                                .and_then(|rest| rest.split_once(";base64,"))
                            {
                                parts.push(json!({
                                    "inlineData": {
                                        "mimeType": mime,
                                        "data": data
                                    }
                                }));
                            } else if !document.url.is_empty() {
                                parts.push(json!({
                                    "fileData": {
                                        "mimeType": document.mime_type,
                                        "fileUri": document.url
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
    let tool_calls = message.tool_calls;
    let has_valid_tool_signature = tool_calls.iter().any(|tool_call| {
        metadata_signature_raw(
            tool_call
                .transformer_metadata
                .get(GEMINI_THOUGHT_SIGNATURE_METADATA_KEY),
        )
        .and_then(|signature| decode_signature_for(SignatureProvider::Gemini, &signature))
        .is_some()
    });
    for (tool_position, tool_call) in tool_calls.into_iter().enumerate() {
        let mut function_call = json!({
            "name": tool_call.function.name,
            "args": tool_arguments_value(&tool_call.function.arguments)
        });
        if !tool_call.id.starts_with(SYNTHETIC_GEMINI_TOOL_ID_PREFIX) && !tool_call.id.is_empty() {
            function_call["id"] = json!(tool_call.id);
        }
        let mut part = json!({ "functionCall": function_call });
        let tool_signature = metadata_signature_raw(
            tool_call
                .transformer_metadata
                .get(GEMINI_THOUGHT_SIGNATURE_METADATA_KEY),
        )
        .and_then(|signature| decode_signature_for(SignatureProvider::Gemini, &signature));
        let fallback_signature =
            (tool_position == 0 && !emitted_signature && !has_valid_tool_signature).then(|| {
                message_signature
                    .clone()
                    .unwrap_or_else(|| DEFAULT_GEMINI_THOUGHT_SIGNATURE.to_string())
            });
        if let Some(signature) = tool_signature.or(fallback_signature) {
            part["thoughtSignature"] = json!(signature);
            emitted_signature = true;
        }
        parts.push(part);
    }
    json!({ "role": role, "parts": parts })
}

fn append_gemini_tool_result_batch(
    contents: &mut Vec<Value>,
    pending_results: &mut Vec<Message>,
    tool_names_by_id: &HashMap<String, String>,
    tool_call_order: &HashMap<String, usize>,
    supports_multimodal_function_response: bool,
) {
    if pending_results.is_empty() {
        return;
    }
    if pending_results.iter().any(|message| {
        message
            .tool_call_id
            .as_deref()
            .is_some_and(|id| id.starts_with(SYNTHETIC_GEMINI_TOOL_ID_PREFIX))
    }) {
        // IDs synthesized for older Gemini calls must be removed on replay.
        // Restore call order before removing them, or reversed completions of
        // the same function would silently exchange their results upstream.
        pending_results.sort_by_key(|message| {
            message
                .tool_call_id
                .as_ref()
                .and_then(|id| tool_call_order.get(id))
                .copied()
                .unwrap_or(usize::MAX)
        });
    }
    let mut parts = Vec::new();
    for message in pending_results.drain(..) {
        let mut content = llm_tool_message_to_gemini_content(
            message,
            tool_names_by_id,
            supports_multimodal_function_response,
        );
        if let Value::Array(result_parts) = content["parts"].take() {
            parts.extend(result_parts);
        }
    }
    contents.push(json!({"role": "user", "parts": parts}));
}

fn llm_tool_message_to_gemini_content(
    message: Message,
    tool_names_by_id: &HashMap<String, String>,
    supports_multimodal_function_response: bool,
) -> Value {
    let tool_call_id = message.tool_call_id.clone().unwrap_or_default();
    let tool_call_name = message
        .tool_call_name
        .clone()
        .or_else(|| tool_names_by_id.get(&tool_call_id).cloned())
        .unwrap_or_else(|| tool_call_id.clone());
    let media_plan = plan_tool_result_images(
        &message.content,
        ToolImageScope::InlineImage,
        TOOL_RESULT_MEDIA_ATTACHED_MARKER,
    );
    let (response, media_parts) = match media_plan {
        Some(plan) => {
            let response = gemini_function_response_value_from_media(&plan);
            let media_parts = plan
                .images
                .into_iter()
                .filter_map(|image| {
                    inline_image_data(&image.url).map(|(media_type, data)| {
                        json!({
                            "inlineData": {
                                "mimeType": media_type,
                                "data": data
                            }
                        })
                    })
                })
                .collect::<Vec<_>>();
            (response, media_parts)
        }
        None => (gemini_function_response_value(message.content), Vec::new()),
    };
    let mut function_response = json!({
        "name": tool_call_name,
        "response": response
    });
    if !tool_call_id.is_empty() && !tool_call_id.starts_with(SYNTHETIC_GEMINI_TOOL_ID_PREFIX) {
        function_response["id"] = json!(tool_call_id);
    }
    let mut parts = Vec::new();
    if supports_multimodal_function_response && !media_parts.is_empty() {
        function_response["parts"] = Value::Array(media_parts);
        parts.push(json!({ "functionResponse": function_response }));
    } else {
        parts.push(json!({ "functionResponse": function_response }));
        if !media_parts.is_empty() {
            parts.push(json!({
                "text": format!("[ai-toolbox: media output of tool call {tool_call_id}]")
            }));
            parts.extend(media_parts);
        }
    }
    json!({ "role": "user", "parts": parts })
}

fn gemini_function_response_value(content: MessageContent) -> Value {
    let text = match content {
        MessageContent::Text(text) => text,
        other => serde_json::to_string(&other).unwrap_or_default(),
    };
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(&text) {
        let candidate = Value::Object(object);
        // Gemini interprets `$ref` keys anywhere inside `functionResponse.response`
        // as multimodal part pointers and rejects JSON Schema/OpenAPI payloads
        // with HTTP 400 (AxonHub 59bb6734). Wrap as raw text instead.
        if !value_contains_json_ref_key(&candidate) {
            return candidate;
        }
    }
    json!({ "result": text })
}

fn gemini_function_response_value_from_media(plan: &ToolResultMediaPlan) -> Value {
    let text = cleaned_tool_result_text(plan);
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(&text) {
        let candidate = Value::Object(object);
        if !value_contains_json_ref_key(&candidate) {
            return candidate;
        }
    }
    json!({ "result": text })
}

fn value_contains_json_ref_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, child)| key == "$ref" || value_contains_json_ref_key(child)),
        Value::Array(items) => items.iter().any(value_contains_json_ref_key),
        _ => false,
    }
}

fn gemini_stop_sequences(stop: Option<Stop>) -> Option<Vec<String>> {
    match stop {
        Some(Stop::String(text)) if !text.is_empty() => Some(vec![text]),
        Some(Stop::Multiple(items)) if !items.is_empty() => Some(items),
        _ => None,
    }
}

fn gemini_tool_config(choice: Option<ToolChoice>) -> Option<Value> {
    match choice {
        Some(ToolChoice::String(choice)) => Some(json!({
            "functionCallingConfig": {
                "mode": match choice.as_str() {
                    "none" => "NONE",
                    "required" | "any" => "ANY",
                    _ => "AUTO",
                }
            }
        })),
        // Gemini has no subset-aware tool choice; keep the caller's mode and
        // drop the permitted subset (AxonHub d5237439). A subset without a
        // mode behaves like a type-only choice: ANY without pinned names.
        Some(ToolChoice::AllowedTools(allowed)) => Some(json!({
            "functionCallingConfig": {
                "mode": match allowed.mode.as_deref() {
                    Some("none") => "NONE",
                    Some("required") | Some("any") => "ANY",
                    Some("auto") => "AUTO",
                    _ => "ANY",
                }
            }
        })),
        // A type-only tool choice (e.g. "image_generation") has no function
        // name to constrain; emit ANY so Gemini still requires a tool call
        // without pinning to an empty/invalid function name.
        Some(ToolChoice::Named(named)) if named.function.name.is_empty() => Some(json!({
            "functionCallingConfig": {"mode": "ANY"}
        })),
        Some(ToolChoice::Named(named)) => Some(json!({
            "functionCallingConfig": {
                "mode": "ANY",
                "allowedFunctionNames": [named.function.name]
            }
        })),
        None => None,
    }
}

pub fn gemini_response_to_llm(body: Value) -> Response {
    if let Some(block_reason) = body
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        let message = Message {
            role: "assistant".to_string(),
            content: MessageContent::Text(format!(
                "Request blocked by Gemini safety filters: {block_reason}"
            )),
            ..Default::default()
        };
        return Response {
            id: body
                .get("responseId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            model: body
                .get("modelVersion")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            choices: vec![Choice {
                index: 0,
                message,
                finish_reason: Some("refusal".to_string()),
                ..Default::default()
            }],
            usage: Some(gemini_usage_to_llm(body.get("usageMetadata"))),
            ..Default::default()
        };
    }

    let candidates = body
        .get("candidates")
        .and_then(Value::as_array)
        .filter(|candidates| !candidates.is_empty())
        .map(|candidates| {
            candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let message =
                        gemini_content_to_llm(candidate.get("content").unwrap_or(&json!({})));
                    let has_tool = !message.tool_calls.is_empty();
                    Choice {
                        index,
                        message,
                        finish_reason: gemini_finish_to_openai_finish(
                            candidate.get("finishReason").and_then(Value::as_str),
                            has_tool,
                        ),
                        ..Default::default()
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![Choice {
                message: gemini_content_to_llm(&json!({})),
                finish_reason: Some("stop".to_string()),
                ..Default::default()
            }]
        });
    Response {
        id: body
            .get("responseId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        model: body
            .get("modelVersion")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        choices: candidates,
        usage: Some(gemini_usage_to_llm(body.get("usageMetadata"))),
        ..Default::default()
    }
}

pub fn llm_response_to_gemini(response: Response) -> Value {
    if let Some(error) = response.error.as_ref() {
        let code =
            error.code.as_ref().map(|code| json!(code)).or_else(|| {
                (!error.error_type.is_empty()).then(|| json!(error.error_type.clone()))
            });
        let kind = (!error.error_type.is_empty()).then(|| error.error_type.clone());
        return gemini_error_from_parts(error.message.clone(), kind, code);
    }
    let is_error_finish = response
        .choices
        .first()
        .and_then(|choice| choice.finish_reason.as_deref())
        == Some("error");
    if is_error_finish {
        return gemini_error_from_parts(
            "Response failed".to_string(),
            Some("api_error".to_string()),
            Some(json!("response_error")),
        );
    }

    let choices = if response.choices.is_empty() {
        vec![Choice::default()]
    } else {
        response.choices
    };
    json!({
        "responseId": response.id,
        "modelVersion": response.model,
        "candidates": choices
            .into_iter()
            .map(|choice| {
                json!({
                    "content": llm_message_to_gemini_content(choice.message),
                    "finishReason": openai_finish_to_gemini_finish(choice.finish_reason.as_deref())
                })
            })
            .collect::<Vec<_>>(),
        "usageMetadata": llm_usage_to_gemini(response.usage.as_ref())
    })
}

fn gemini_u64_field(usage: &Value, field: &str) -> Option<u64> {
    let value = usage.get(field)?;
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

pub(crate) fn gemini_usage_to_llm(usage: Option<&Value>) -> Usage {
    let usage = usage.unwrap_or(&Value::Null);
    let base_prompt = gemini_u64_field(usage, "promptTokenCount").unwrap_or(0);
    // Include server-side tool retrieval tokens (`toolUsePromptTokenCount`,
    // AxonHub 84533aba) and accept Vertex AI protobuf stringified int64 values.
    let tool_use_prompt = gemini_u64_field(usage, "toolUsePromptTokenCount").unwrap_or(0);
    let prompt = base_prompt.saturating_add(tool_use_prompt);
    let cached = gemini_u64_field(usage, "cachedContentTokenCount").unwrap_or(0);
    let candidate_tokens = gemini_u64_field(usage, "candidatesTokenCount")
        .or_else(|| {
            gemini_u64_field(usage, "totalTokenCount").map(|total| total.saturating_sub(prompt))
        })
        .unwrap_or(0);
    let reasoning = gemini_u64_field(usage, "thoughtsTokenCount").unwrap_or(0);
    let completion = candidate_tokens.saturating_add(reasoning);
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: gemini_u64_field(usage, "totalTokenCount")
            .unwrap_or_else(|| prompt.saturating_add(completion)),
        cached_tokens: cached,
        cache_write_tokens: 0,
        reasoning_tokens: reasoning,
    }
}

pub(crate) fn llm_usage_to_gemini(usage: Option<&Usage>) -> Value {
    let usage = usage.cloned().unwrap_or_default();
    json!({
        "promptTokenCount": usage.prompt_tokens,
        "cachedContentTokenCount": usage.cached_tokens,
        "candidatesTokenCount": usage.completion_tokens.saturating_sub(usage.reasoning_tokens),
        "totalTokenCount": if usage.total_tokens == 0 {
            usage.prompt_tokens.saturating_add(usage.completion_tokens)
        } else {
            usage.total_tokens
        },
        "thoughtsTokenCount": usage.reasoning_tokens
    })
}

pub(crate) fn gemini_finish_to_openai_finish(
    reason: Option<&str>,
    has_tool: bool,
) -> Option<String> {
    let reason = reason.filter(|reason| !reason.trim().is_empty())?;
    match reason {
        "MAX_TOKENS" => Some("length".to_string()),
        "SAFETY"
        | "RECITATION"
        | "SPII"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "LANGUAGE"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "IMAGE_RECITATION" => Some("refusal".to_string()),
        _ if has_tool => Some("tool_calls".to_string()),
        _ => Some("stop".to_string()),
    }
}

fn openai_finish_to_gemini_finish(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length") | Some("max_tokens") => "MAX_TOKENS",
        Some("refusal") | Some("content_filter") | Some("sensitive") => "SAFETY",
        _ => "STOP",
    }
}

pub(super) fn gemini_error(error: Value) -> Value {
    let message =
        extract_error_message(&error).unwrap_or_else(|| "Protocol conversion error".to_string());
    gemini_error_from_parts(
        message,
        extract_error_type(&error),
        extract_error_code(&error),
    )
}

pub(super) fn gemini_error_from_parts(
    message: String,
    kind: Option<String>,
    code: Option<Value>,
) -> Value {
    let status_code = gemini_error_status_code(code.as_ref(), kind.as_deref());
    let status = gemini_error_status(status_code, kind.as_deref());
    json!({
        "error": {
            "code": status_code,
            "message": message,
            "status": status
        }
    })
}

pub(super) fn gemini_error_status_code(code: Option<&Value>, kind: Option<&str>) -> u16 {
    if let Some(code) = code {
        if let Some(code) = code.as_u64().and_then(|code| u16::try_from(code).ok()) {
            return code;
        }
        if let Some(code_text) = code.as_str() {
            if let Ok(code) = code_text.parse::<u16>() {
                return code;
            }
            if let Some(code) = common_error_code_to_http_status(code_text) {
                return code;
            }
        }
    }
    kind.and_then(common_error_code_to_http_status)
        .unwrap_or(500)
}

pub(super) fn gemini_error_status(status_code: u16, kind: Option<&str>) -> &'static str {
    if let Some(kind) = kind.and_then(gemini_status_from_text) {
        return kind;
    }
    match status_code {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        409 => "ALREADY_EXISTS",
        429 => "RESOURCE_EXHAUSTED",
        500 => "INTERNAL",
        501 => "UNIMPLEMENTED",
        503 => "UNAVAILABLE",
        _ => "UNKNOWN",
    }
}

pub(super) fn common_error_code_to_http_status(code: &str) -> Option<u16> {
    match code {
        "INVALID_ARGUMENT" | "invalid_request_error" | "bad_request" => Some(400),
        "UNAUTHENTICATED" | "authentication_error" | "unauthorized" => Some(401),
        "PERMISSION_DENIED" | "permission_denied" | "forbidden" => Some(403),
        "NOT_FOUND" | "not_found" | "invalid_model_error" => Some(404),
        "ALREADY_EXISTS" | "already_exists" => Some(409),
        "RESOURCE_EXHAUSTED" | "rate_limit_error" | "rate_limit_exceeded" => Some(429),
        "INTERNAL" | "internal_error" | "internal_server_error" | "api_error" | "stream_error" => {
            Some(500)
        }
        "UNIMPLEMENTED" | "not_implemented" => Some(501),
        "UNAVAILABLE" | "service_unavailable" => Some(503),
        _ => None,
    }
}

pub(super) fn gemini_status_from_text(text: &str) -> Option<&'static str> {
    match text {
        "INVALID_ARGUMENT" => Some("INVALID_ARGUMENT"),
        "UNAUTHENTICATED" => Some("UNAUTHENTICATED"),
        "PERMISSION_DENIED" => Some("PERMISSION_DENIED"),
        "NOT_FOUND" => Some("NOT_FOUND"),
        "ALREADY_EXISTS" => Some("ALREADY_EXISTS"),
        "RESOURCE_EXHAUSTED" => Some("RESOURCE_EXHAUSTED"),
        "INTERNAL" => Some("INTERNAL"),
        "UNIMPLEMENTED" => Some("UNIMPLEMENTED"),
        "UNAVAILABLE" => Some("UNAVAILABLE"),
        "UNKNOWN" => Some("UNKNOWN"),
        _ => None,
    }
}
