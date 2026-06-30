use super::super::error::ProtocolConversionError;
use super::super::llm::{
    ApiFormat, Choice, Function, FunctionCall, ImageUrl, Message, MessageContent,
    MessageContentPart, Request, RequestType, Response, ResponseCustomTool, ResponseCustomToolCall,
    StreamOptions, Tool, ToolCall, Usage, TOOL_TYPE_FUNCTION, TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
};
use super::super::shared::{
    content_text, extract_error_message, should_emit_openai_request_metadata, stop_from_value,
    stop_to_value, tool_choice_from_openai, tool_choice_to_openai,
};
use super::super::traits::{InboundTransformer, OutboundTransformer};
use super::super::types::AiProtocol;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub struct OpenAiChatInbound;
pub struct OpenAiChatOutbound;

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
    let content = match message.get("content") {
        Some(Value::Array(parts)) => MessageContent::Parts(
            parts
                .iter()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("text") => Some(MessageContentPart {
                        part_type: "text".to_string(),
                        text: part
                            .get("text")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        ..Default::default()
                    }),
                    Some("image_url") => Some(MessageContentPart {
                        part_type: "image_url".to_string(),
                        image_url: part
                            .get("image_url")
                            .cloned()
                            .and_then(|value| serde_json::from_value::<ImageUrl>(value).ok()),
                        ..Default::default()
                    }),
                    _ => None,
                })
                .collect(),
        ),
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
    let reasoning = message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

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

pub fn llm_request_to_chat(request: Request) -> Value {
    let messages = request
        .messages
        .into_iter()
        .map(llm_message_to_chat)
        .collect::<Vec<_>>();
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
                function.parameters.unwrap_or_else(|| json!({})),
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

fn llm_message_to_chat(message: Message) -> Value {
    if message.role == "tool" {
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
    let mut result = json!({
        "role": message.role,
        "content": llm_content_to_chat_value(message.content),
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
                        "image_url": part.image_url
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
                            .map(|reason| {
                                if reason == "function_call" {
                                    "tool_calls"
                                } else {
                                    reason
                                }
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
    Response {
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
    }
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
    let choices = if response.choices.is_empty() {
        vec![Choice::default()]
    } else {
        response.choices
    };
    json!({
        "id": response.id,
        "object": "chat.completion",
        "created": response.created,
        "model": response.model,
        "choices": choices
            .into_iter()
            .map(|choice| {
                json!({
                    "index": choice.index,
                    "message": llm_message_to_chat(choice.message),
                    "finish_reason": choice.finish_reason.unwrap_or_else(|| "stop".to_string())
                })
            })
            .collect::<Vec<_>>(),
        "usage": usage_to_openai(response.usage.as_ref())
    })
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
        reasoning_tokens,
    }
}

pub fn usage_to_openai(usage: Option<&Usage>) -> Value {
    let usage = usage.cloned().unwrap_or_default();
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": if usage.total_tokens == 0 {
            usage.prompt_tokens.saturating_add(usage.completion_tokens)
        } else {
            usage.total_tokens
        },
        "prompt_tokens_details": {
            "cached_tokens": usage.cached_tokens
        },
        "completion_tokens_details": {
            "reasoning_tokens": usage.reasoning_tokens
        }
    })
}

fn openai_error(error: Value) -> Value {
    if error.get("error").is_some() {
        return error;
    }
    let message =
        extract_error_message(&error).unwrap_or_else(|| "Protocol conversion error".to_string());
    json!({
        "error": {
            "message": message,
            "type": "api_error",
            "param": Value::Null,
            "code": Value::Null
        }
    })
}
