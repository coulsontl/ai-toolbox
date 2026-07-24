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
use super::super::shared::{
    budget_tokens_to_reasoning_effort, extract_error_code, extract_error_message,
    extract_error_type, json_string, reasoning_effort_to_budget_tokens, stop_from_value,
    tool_arguments_value, tool_choice_from_gemini,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

const SYNTHETIC_GEMINI_TOOL_ID_PREFIX: &str = "gemini_synth_";

pub fn gemini_request_to_llm(body: Value) -> Request {
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
    if let Some(system) = gemini_parts_text(body.pointer("/systemInstruction/parts")) {
        request.messages.push(Message {
            role: "system".to_string(),
            content: MessageContent::Text(system),
            ..Default::default()
        });
    }
    if let Some(contents) = body.get("contents").and_then(Value::as_array) {
        let mut function_call_ids_by_name = HashMap::new();
        for content in contents {
            let message = gemini_content_to_llm(content, &function_call_ids_by_name);
            for tool_call in &message.tool_calls {
                if !tool_call.id.is_empty() && !tool_call.function.name.is_empty() {
                    function_call_ids_by_name
                        .insert(tool_call.function.name.clone(), tool_call.id.clone());
                }
            }
            request.messages.push(message);
        }
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        request.tools = tools.iter().flat_map(gemini_tool_to_llm).collect();
    }
    request
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

fn gemini_content_to_llm(
    content: &Value,
    function_call_ids_by_name: &HashMap<String, String>,
) -> Message {
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
                        .unwrap_or_else(|| format!("{SYNTHETIC_GEMINI_TOOL_ID_PREFIX}{index}")),
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
                    .or_else(|| function_call_ids_by_name.get(&function_name).cloned())
                    .or_else(|| (!function_name.is_empty()).then(|| function_name.clone()));
                tool_result = Some(Message {
                    role: "tool".to_string(),
                    tool_call_id,
                    tool_call_name: (!function_name.is_empty()).then_some(function_name),
                    content: MessageContent::Text(gemini_function_response_text(
                        function_response.get("response"),
                    )),
                    ..Default::default()
                });
            }
        }
    }
    let reasoning = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
    tool_result.unwrap_or(Message {
        role,
        content: MessageContent::Parts(parts),
        tool_calls,
        reasoning_content: reasoning.clone(),
        reasoning,
        reasoning_signature,
        ..Default::default()
    })
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

pub fn llm_request_to_gemini(request: Request) -> Value {
    let mut system_chunks = Vec::new();
    let mut contents = Vec::new();
    let mut tool_names_by_id = HashMap::new();
    for message in request.messages {
        if message.role == "system" || message.role == "developer" {
            if let MessageContent::Text(text) = message.content {
                if !text.is_empty() {
                    system_chunks.push(text);
                }
            }
            continue;
        }
        if message.role == "tool" {
            contents.push(llm_tool_message_to_gemini_content(
                message,
                &tool_names_by_id,
            ));
            continue;
        }
        for tool_call in &message.tool_calls {
            if !tool_call.id.is_empty() && !tool_call.function.name.is_empty() {
                tool_names_by_id.insert(tool_call.id.clone(), tool_call.function.name.clone());
            }
        }
        contents.push(llm_message_to_gemini_content(message));
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
    let budget_tokens = reasoning_effort_to_budget_tokens(reasoning_effort, max_tokens)?;
    if is_gemini_3_model(model) {
        return Some(json!({
            "includeThoughts": budget_tokens > 0,
            "thinkingLevel": gemini_3_thinking_level(reasoning_effort, budget_tokens)
        }));
    }
    Some(json!({
        "includeThoughts": budget_tokens > 0,
        "thinkingBudget": budget_tokens.min(24576)
    }))
}

fn is_gemini_3_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("gemini-3")
}

fn gemini_3_thinking_level(reasoning_effort: &str, budget_tokens: i64) -> &'static str {
    match reasoning_effort.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "disabled" => "none",
        "minimal" | "low" => "low",
        "medium" => "medium",
        "high" | "xhigh" | "extra_high" | "max" => "high",
        _ if budget_tokens <= 0 => "none",
        _ if budget_tokens <= 4096 => "low",
        _ if budget_tokens <= 10240 => "medium",
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
        parts.push(json!({
            "functionResponse": {
                "id": message.tool_call_id.clone().unwrap_or_default(),
                "name": message.tool_call_name.or(message.tool_call_id).unwrap_or_default(),
                "response": gemini_function_response_value(message.content)
            }
        }));
        return json!({ "role": "user", "parts": parts });
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

fn llm_tool_message_to_gemini_content(
    message: Message,
    tool_names_by_id: &HashMap<String, String>,
) -> Value {
    let tool_call_id = message.tool_call_id.clone().unwrap_or_default();
    let tool_call_name = message
        .tool_call_name
        .clone()
        .or_else(|| tool_names_by_id.get(&tool_call_id).cloned())
        .unwrap_or_else(|| tool_call_id.clone());
    json!({
        "role": "user",
        "parts": [{
            "functionResponse": {
                "id": tool_call_id,
                "name": tool_call_name,
                "response": gemini_function_response_value(message.content)
            }
        }]
    })
}

fn gemini_function_response_value(content: MessageContent) -> Value {
    let text = match content {
        MessageContent::Text(text) => text,
        other => serde_json::to_string(&other).unwrap_or_default(),
    };
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(&text) {
        return Value::Object(object);
    }
    json!({ "result": text })
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
                    let message = gemini_content_to_llm(
                        candidate.get("content").unwrap_or(&json!({})),
                        &HashMap::new(),
                    );
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
                message: gemini_content_to_llm(&json!({}), &HashMap::new()),
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

fn gemini_usage_to_llm(usage: Option<&Value>) -> Usage {
    let usage = usage.unwrap_or(&Value::Null);
    let prompt = usage
        .get("promptTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = usage
        .get("cachedContentTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let candidate_tokens = usage
        .get("candidatesTokenCount")
        .and_then(Value::as_u64)
        .or_else(|| {
            usage
                .get("totalTokenCount")
                .and_then(Value::as_u64)
                .map(|total| total.saturating_sub(prompt))
        })
        .unwrap_or(0);
    let reasoning = usage
        .get("thoughtsTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion = candidate_tokens.saturating_add(reasoning);
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: usage
            .get("totalTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| prompt.saturating_add(completion)),
        cached_tokens: cached,
        reasoning_tokens: reasoning,
    }
}

fn llm_usage_to_gemini(usage: Option<&Usage>) -> Value {
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

fn gemini_finish_to_openai_finish(reason: Option<&str>, has_tool: bool) -> Option<String> {
    let reason = reason.filter(|reason| !reason.trim().is_empty())?;
    match reason {
        "MAX_TOKENS" => Some("length".to_string()),
        "SAFETY" | "RECITATION" | "SPII" | "BLOCKLIST" | "PROHIBITED_CONTENT" => {
            Some("refusal".to_string())
        }
        _ if has_tool => Some("tool_calls".to_string()),
        _ => Some("stop".to_string()),
    }
}

fn openai_finish_to_gemini_finish(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length") | Some("max_tokens") => "MAX_TOKENS",
        Some("refusal") => "SAFETY",
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

pub(super) fn gemini_error_from_parts(message: String, kind: Option<String>, code: Option<Value>) -> Value {
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
