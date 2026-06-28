use super::error::ProtocolConversionError;
use super::llm::{
    ApiFormat, Choice, DocumentUrl, Function, FunctionCall, GoogleTools, ImageUrl, Message,
    MessageContent, MessageContentPart, Request, RequestType, Response, Stop, Tool, ToolCall,
    ToolChoice, Usage, TOOL_TYPE_FUNCTION, TOOL_TYPE_GOOGLE_CODE_EXECUTION,
    TOOL_TYPE_GOOGLE_SEARCH, TOOL_TYPE_GOOGLE_URL_CONTEXT,
};
use super::shared::{json_string, stop_from_value, tool_arguments_value, tool_choice_from_gemini};
use super::transformer::{InboundTransformer, OutboundTransformer};
use super::types::AiProtocol;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

const SYNTHETIC_GEMINI_TOOL_ID_PREFIX: &str = "gemini_synth_";

pub struct GeminiInbound;
pub struct GeminiOutbound;

impl InboundTransformer for GeminiInbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::GeminiNative
    }

    fn request_to_llm(&self, body: Value) -> Result<Request, ProtocolConversionError> {
        Ok(gemini_request_to_llm(body))
    }

    fn response_from_llm(&self, response: Response) -> Result<Value, ProtocolConversionError> {
        Ok(llm_response_to_gemini(response))
    }

    fn error_from_llm(&self, error: Value) -> Value {
        gemini_error(error)
    }
}

impl OutboundTransformer for GeminiOutbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::GeminiNative
    }

    fn request_from_llm(&self, request: Request) -> Result<Value, ProtocolConversionError> {
        Ok(llm_request_to_gemini(request))
    }

    fn response_to_llm(&self, body: Value) -> Result<Response, ProtocolConversionError> {
        Ok(gemini_response_to_llm(body))
    }

    fn error_to_llm(&self, error: Value) -> Value {
        error
    }
}

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
    if budget <= 0 {
        Some("none".to_string())
    } else if budget <= 1_024 {
        Some("low".to_string())
    } else if budget <= 8_192 {
        Some("medium".to_string())
    } else {
        Some("high".to_string())
    }
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
    if let Some(gemini_parts) = content.get("parts").and_then(Value::as_array) {
        for (index, part) in gemini_parts.iter().enumerate() {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        reasoning_chunks.push(text.to_string());
                    }
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
        ..Default::default()
    })
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
    if let Some(max_tokens) = request.max_tokens.or(request.max_completion_tokens) {
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
        if let Some(thinking_config) = gemini_thinking_config(reasoning_effort) {
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
                    generation_config.insert("responseSchema".to_string(), schema);
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
                Some(json!({
                    "name": function.name,
                    "description": function.description,
                    "parameters": function
                        .parameters_json_schema
                        .clone()
                        .or_else(|| function.parameters.clone())
                        .unwrap_or_else(|| json!({}))
                }))
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

fn gemini_thinking_config(reasoning_effort: &str) -> Option<Value> {
    match reasoning_effort.to_ascii_lowercase().as_str() {
        "none" => Some(json!({
            "includeThoughts": false,
            "thinkingBudget": 0
        })),
        "low" => Some(json!({
            "includeThoughts": true,
            "thinkingLevel": "low"
        })),
        "medium" => Some(json!({
            "includeThoughts": true,
            "thinkingLevel": "medium"
        })),
        "high" | "xhigh" | "max" => Some(json!({
            "includeThoughts": true,
            "thinkingLevel": "high"
        })),
        _ => None,
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
                "response": {
                    "content": match message.content {
                        MessageContent::Text(text) => text,
                        other => serde_json::to_string(&other).unwrap_or_default(),
                    }
                }
            }
        }));
        return json!({ "role": "user", "parts": parts });
    }
    if let Some(reasoning) = message
        .reasoning_content
        .as_deref()
        .or(message.reasoning.as_deref())
    {
        if !reasoning.is_empty() {
            parts.push(json!({ "text": reasoning, "thought": true }));
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
    for tool_call in message.tool_calls {
        let mut function_call = json!({
            "name": tool_call.function.name,
            "args": tool_arguments_value(&tool_call.function.arguments)
        });
        if !tool_call.id.starts_with(SYNTHETIC_GEMINI_TOOL_ID_PREFIX) && !tool_call.id.is_empty() {
            function_call["id"] = json!(tool_call.id);
        }
        parts.push(json!({ "functionCall": function_call }));
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
                "response": {
                    "content": match message.content {
                        MessageContent::Text(text) => text,
                        other => serde_json::to_string(&other).unwrap_or_default(),
                    }
                }
            }
        }]
    })
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
                        finish_reason: Some(gemini_finish_to_openai_finish(
                            candidate.get("finishReason").and_then(Value::as_str),
                            has_tool,
                        )),
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

fn gemini_finish_to_openai_finish(reason: Option<&str>, has_tool: bool) -> String {
    match reason {
        Some("MAX_TOKENS") => "length".to_string(),
        Some("SAFETY")
        | Some("RECITATION")
        | Some("SPII")
        | Some("BLOCKLIST")
        | Some("PROHIBITED_CONTENT") => "refusal".to_string(),
        _ if has_tool => "tool_calls".to_string(),
        _ => "stop".to_string(),
    }
}

fn openai_finish_to_gemini_finish(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length") | Some("max_tokens") => "MAX_TOKENS",
        Some("refusal") => "SAFETY",
        _ => "STOP",
    }
}

fn gemini_error(error: Value) -> Value {
    if error.get("error").is_some() {
        return error;
    }
    json!({
        "error": {
            "code": 500,
            "message": error.get("message").and_then(Value::as_str).unwrap_or("Protocol conversion error"),
            "status": "INTERNAL"
        }
    })
}
