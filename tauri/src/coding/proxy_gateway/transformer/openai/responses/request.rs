use crate::coding::proxy_gateway::transformer::llm::{
    ApiFormat, Message, MessageContent, Request, RequestType, TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
};
use crate::coding::proxy_gateway::transformer::shared::{
    should_emit_openai_request_metadata, stop_from_value, stop_to_value, tool_choice_from_openai,
    tool_choice_to_responses,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use super::shared::*;

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
    // Top-level reasoning.context (e.g. "all_turns"), distinct from input item context.
    if let Some(context) = body.pointer("/reasoning/context") {
        request.transformer_metadata.insert(
            RESPONSES_REQUEST_REASONING_CONTEXT_METADATA_KEY.to_string(),
            context.clone(),
        );
    }
    preserve_responses_transformer_metadata(
        &body,
        &mut request,
        "max_tool_calls",
        RESPONSES_MAX_TOOL_CALLS_METADATA_KEY,
    );
    preserve_responses_transformer_metadata(
        &body,
        &mut request,
        "prompt_cache_retention",
        RESPONSES_PROMPT_CACHE_RETENTION_METADATA_KEY,
    );
    preserve_responses_transformer_metadata(
        &body,
        &mut request,
        "truncation",
        RESPONSES_TRUNCATION_METADATA_KEY,
    );
    append_responses_input_to_messages(body.get("input"), &mut request.messages);
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        request.tools = tools.iter().filter_map(responses_tool_to_llm).collect();
    }
    attach_responses_raw_request_metadata(&body, &mut request);
    request
}

pub fn responses_compact_request_to_llm(body: Value) -> Request {
    let mut request = responses_request_to_llm(body);
    request.request_type = Some(RequestType::Compact);
    request.api_format = Some(ApiFormat::OpenAiResponsesCompact);
    request.stream = Some(false);
    request
}

pub fn llm_request_to_responses(request: Request) -> Value {
    let mut input = Vec::new();
    let mut instructions = Vec::new();
    let mut custom_tool_call_ids = HashMap::new();
    let raw_tools = request
        .transformer_metadata
        .get(RESPONSES_RAW_TOOLS_METADATA_KEY)
        .cloned();
    let raw_tool_choice = request
        .transformer_metadata
        .get(RESPONSES_RAW_TOOL_CHOICE_METADATA_KEY)
        .cloned();
    let raw_input_items = request
        .transformer_metadata
        .get(RESPONSES_RAW_INPUT_ITEMS_METADATA_KEY)
        .cloned();
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
    let max_tool_calls = responses_metadata_or_extra_body(
        &request,
        RESPONSES_MAX_TOOL_CALLS_METADATA_KEY,
        "max_tool_calls",
    );
    let prompt_cache_retention = responses_metadata_or_extra_body(
        &request,
        RESPONSES_PROMPT_CACHE_RETENTION_METADATA_KEY,
        "prompt_cache_retention",
    );
    let truncation =
        responses_metadata_or_extra_body(&request, RESPONSES_TRUNCATION_METADATA_KEY, "truncation");
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
    input = merge_raw_responses_fragments(input, raw_input_items.as_ref());
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
    // Merge effort + request-level context; do not overwrite context with effort-only object.
    let request_reasoning_context = request
        .transformer_metadata
        .get(RESPONSES_REQUEST_REASONING_CONTEXT_METADATA_KEY)
        .cloned();
    if request.reasoning_effort.is_some() || request_reasoning_context.is_some() {
        let mut reasoning = serde_json::Map::new();
        if let Some(effort) = request.reasoning_effort {
            reasoning.insert("effort".to_string(), json!(effort));
        }
        if let Some(context) = request_reasoning_context {
            reasoning.insert("context".to_string(), context);
        }
        body["reasoning"] = Value::Object(reasoning);
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
                        let mut tool_object = Map::new();
                        tool_object.insert("type".to_string(), json!("custom"));
                        tool_object.insert("name".to_string(), json!(custom.name));
                        if !custom.description.is_empty() {
                            tool_object
                                .insert("description".to_string(), json!(custom.description));
                        }
                        if let Some(format) = custom.format {
                            tool_object.insert("format".to_string(), json!(format));
                        }
                        Value::Object(tool_object)
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
                if !function.description.is_empty() {
                    tool_object.insert("description".to_string(), json!(function.description));
                }
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
    let tool_signatures = request
        .transformer_metadata
        .get(RESPONSES_TOOL_SIGNATURES_METADATA_KEY)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        });
    let mut tools = tools;
    let tools = merge_raw_responses_fragments_with_signatures(
        &mut tools,
        raw_tools.as_ref(),
        tool_signatures.as_deref(),
    );
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(raw_tool_choice) = raw_tool_choice {
        body["tool_choice"] = raw_tool_choice;
    } else if let Some(tool_choice) = tool_choice_to_responses(request.tool_choice) {
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
    if let Some(max_tool_calls) = max_tool_calls {
        body["max_tool_calls"] = max_tool_calls;
    }
    if let Some(prompt_cache_retention) = prompt_cache_retention {
        body["prompt_cache_retention"] = prompt_cache_retention;
    }
    if let Some(truncation) = truncation {
        body["truncation"] = truncation;
    }
    body
}

pub fn llm_request_to_responses_compact(request: Request) -> Value {
    let mut body = llm_request_to_responses(request);
    if let Some(object) = body.as_object_mut() {
        object.remove("stream");
    }
    body
}

