use crate::coding::proxy_gateway::transformer::llm::{
    ApiFormat, Message, MessageContent, Request, RequestType, TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
};
use crate::coding::proxy_gateway::transformer::shared::{
    downgrade_instruction_message, instruction_hoist_plan, placement_for_api_format,
    should_emit_openai_request_metadata, stop_from_value, stop_to_value, tool_choice_from_openai,
    tool_choice_to_responses,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use super::shared::*;

/// The Responses API rejects `max_output_tokens` below 16 ("The number must be
/// `>= 16`"), while Anthropic clients legitimately send tiny probe budgets
/// (the Claude Desktop model-availability probe sends `max_tokens=1`).
/// Sub-floor budgets are clamped up to the minimum instead of failing the
/// whole request; `0` and other pass-through values keep their existing
/// semantics.
const RESPONSES_MIN_MAX_OUTPUT_TOKENS: i64 = 16;

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
    let raw_input_items =
        append_responses_input_to_messages(body.get("input"), &mut request.messages);
    if !raw_input_items.is_empty() {
        request.transformer_metadata.insert(
            RESPONSES_RAW_INPUT_ITEMS_METADATA_KEY.to_string(),
            Value::Array(raw_input_items),
        );
    }
    let mut raw_tools_combined: Vec<Value> = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Codex Responses Lite carries extra tool definitions inside `input[]`
    // items with `type == "additional_tools"` (cc-switch a35e5000). Hoist them
    // into `request.tools` so Chat/Anthropic/Gemini targets receive them and do
    // not turn the carrier into an empty developer message.
    if let Some(input_items) = body.get("input").and_then(Value::as_array) {
        let mut seen_names: std::collections::HashSet<String> = raw_tools_combined
            .iter()
            .filter_map(|tool| {
                tool.get("name")
                    .or_else(|| tool.pointer("/function/name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();
        for item in input_items {
            if item.get("type").and_then(Value::as_str) != Some("additional_tools") {
                continue;
            }
            if let Some(extra_tools) = item.get("tools").and_then(Value::as_array) {
                for tool in extra_tools {
                    let key = tool
                        .get("name")
                        .or_else(|| tool.pointer("/function/name"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    if let Some(name) = key {
                        if !seen_names.insert(name) {
                            continue;
                        }
                    }
                    raw_tools_combined.push(tool.clone());
                }
            }
        }
    }
    if !raw_tools_combined.is_empty() {
        request.tools = responses_tools_to_llm(&raw_tools_combined);
    }
    attach_responses_raw_request_metadata(&body, &raw_tools_combined, &mut request);
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
    let mut raw_input_items = request
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
    let mut message_input_offsets = Vec::with_capacity(request.messages.len() + 1);
    let hoist_instructions = instruction_hoist_plan(
        &request.messages,
        placement_for_api_format(request.api_format),
    );
    for (message_index, message) in request.messages.into_iter().enumerate() {
        message_input_offsets.push(input.len());
        if hoist_instructions[message_index] {
            if let MessageContent::Text(text) = message.content {
                if !text.is_empty() {
                    instructions.push(text);
                }
            }
            continue;
        }
        // Instruction messages written in place (Anthropic/Claude Code sources) are
        // downgraded to `user`: hoisting them would rewrite the prompt head every
        // turn and break upstream prefix caching (see `shared/system_messages.rs`).
        let message = downgrade_instruction_message(message);
        append_llm_message_as_responses_input(message, &mut input, &mut custom_tool_call_ids);
    }
    message_input_offsets.push(input.len());
    if let Some(fragments) = raw_input_items.as_mut().and_then(Value::as_array_mut) {
        for (raw_offset, fragment) in fragments.iter_mut().enumerate() {
            if let Some(input_offset) = fragment
                .get("message_index")
                .and_then(Value::as_u64)
                .and_then(|index| message_input_offsets.get(index as usize))
            {
                fragment["index"] = json!(input_offset + raw_offset);
            }
        }
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
        let max_tokens = if (1..RESPONSES_MIN_MAX_OUTPUT_TOKENS).contains(&max_tokens) {
            RESPONSES_MIN_MAX_OUTPUT_TOKENS
        } else {
            max_tokens
        };
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
    let default_non_strict_for_anthropic = request.api_format == Some(ApiFormat::AnthropicMessages);
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
                // Anthropic tools allow optional properties and are non-strict by
                // default. Responses upstreams auto-strictify when `strict` is
                // omitted, rejecting optional parameters (AxonHub 98b8195e /
                // cc-switch 02759c67). Explicitly emit `strict: false` when absent.
                let strict = function
                    .strict
                    .or_else(|| default_non_strict_for_anthropic.then_some(false));
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
        .and_then(|items| {
            items
                .iter()
                .map(|item| item.as_str().map(ToString::to_string))
                .collect::<Option<Vec<_>>>()
        });
    let mut tools = tools;
    let can_merge_raw_tools = raw_tools.is_none()
        || (request
            .transformer_metadata
            .get(RESPONSES_TOOL_SIGNATURES_COMPLETE_METADATA_KEY)
            .and_then(Value::as_bool)
            == Some(true)
            && tool_signatures.is_some());
    let tools = if raw_tools.is_some() && !can_merge_raw_tools {
        tools
    } else {
        merge_raw_responses_fragments_with_signatures(
            &mut tools,
            raw_tools.as_ref(),
            tool_signatures.as_deref(),
        )
    };
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
        let mut is_json_object_format = false;
        if let Some(response_format) = request.response_format {
            let format_value = response_format_to_responses_format(response_format);
            is_json_object_format =
                format_value.get("type").and_then(Value::as_str) == Some("json_object");
            text.insert("format".to_string(), format_value);
        }
        if let Some(verbosity) = request.verbosity {
            text.insert("verbosity".to_string(), json!(verbosity));
        }
        body["text"] = Value::Object(text);

        // OpenAI Responses validates `text.format.type == "json_object"` by
        // scanning `input` items (not `instructions`) for the word "json".
        // Insert a minimal developer reminder when none is present (AxonHub dd364d6a).
        if is_json_object_format {
            if let Some(input_items) = body.get_mut("input").and_then(Value::as_array_mut) {
                if !responses_input_mentions_json(input_items) {
                    input_items.insert(
                        0,
                        json!({
                            "type": "message",
                            "role": "developer",
                            "content": [{
                                "type": "input_text",
                                "text": "Respond in valid JSON format."
                            }]
                        }),
                    );
                }
            }
        }
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

fn responses_input_mentions_json(items: &[Value]) -> bool {
    items.iter().any(|item| {
        let is_message = item
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(|kind| kind == "message");
        if !is_message || item.get("role").and_then(Value::as_str) == Some("assistant") {
            return false;
        }
        match item.get("content") {
            Some(Value::String(text)) => text.to_ascii_lowercase().contains("json"),
            Some(Value::Array(parts)) => parts.iter().any(|part| {
                matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("input_text" | "text")
                ) && part
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.to_ascii_lowercase().contains("json"))
            }),
            _ => false,
        }
    })
}

pub fn llm_request_to_responses_compact(request: Request) -> Value {
    let mut body = llm_request_to_responses(request);
    if let Some(object) = body.as_object_mut() {
        object.remove("stream");
    }
    body
}
