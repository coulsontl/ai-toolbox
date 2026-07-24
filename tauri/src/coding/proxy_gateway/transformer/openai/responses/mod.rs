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
    normalize_function_parameters_owned, should_emit_openai_request_metadata, stop_from_value,
    stop_to_value, tool_choice_from_openai, tool_choice_to_responses,
};
use crate::coding::proxy_gateway::transformer::traits::{InboundTransformer, OutboundTransformer};
use crate::coding::proxy_gateway::transformer::types::AiProtocol;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub struct OpenAiResponsesInbound;
pub struct OpenAiResponsesOutbound;

const RESPONSES_INCLUDE_METADATA_KEY: &str = "openai_responses_include";
const RESPONSES_MAX_TOOL_CALLS_METADATA_KEY: &str = "openai_responses_max_tool_calls";
const RESPONSES_PROMPT_CACHE_RETENTION_METADATA_KEY: &str =
    "openai_responses_prompt_cache_retention";
const RESPONSES_TRUNCATION_METADATA_KEY: &str = "openai_responses_truncation";
const RESPONSES_COMPACTION_ENCRYPTED_CONTENT_METADATA_KEY: &str =
    "openai_responses_compaction_encrypted_content";
const RESPONSES_COMPACTION_CREATED_BY_METADATA_KEY: &str = "openai_responses_compaction_created_by";
const RESPONSES_RAW_TOOLS_METADATA_KEY: &str = "openai_responses_raw_tools";
const RESPONSES_TOOL_SIGNATURES_METADATA_KEY: &str = "openai_responses_tool_signatures";
const RESPONSES_RAW_TOOL_CHOICE_METADATA_KEY: &str = "openai_responses_raw_tool_choice";
const RESPONSES_RAW_INPUT_ITEMS_METADATA_KEY: &str = "openai_responses_raw_input_items";
/// Request top-level `reasoning.context` (e.g. `"all_turns"`), not input-item context.
const RESPONSES_REQUEST_REASONING_CONTEXT_METADATA_KEY: &str =
    "openai_responses_request_reasoning_context";

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

fn preserve_responses_transformer_metadata(
    body: &Value,
    request: &mut Request,
    field_name: &str,
    metadata_key: &str,
) {
    if let Some(value) = body.get(field_name) {
        request
            .transformer_metadata
            .insert(metadata_key.to_string(), value.clone());
    }
}

fn attach_responses_raw_request_metadata(body: &Value, request: &mut Request) {
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let raw_tools = tools
            .iter()
            .enumerate()
            .filter(|(_, tool)| !is_structurally_represented_responses_tool(tool))
            .map(|(index, tool)| raw_responses_fragment(index, tool.clone()))
            .collect::<Vec<_>>();
        if !raw_tools.is_empty() {
            request.transformer_metadata.insert(
                RESPONSES_RAW_TOOLS_METADATA_KEY.to_string(),
                Value::Array(raw_tools),
            );
        }

        let tool_signatures = tools
            .iter()
            .filter(|tool| is_structurally_represented_responses_tool(tool))
            .filter_map(responses_tool_signature)
            .map(Value::String)
            .collect::<Vec<_>>();
        if !tool_signatures.is_empty() {
            request.transformer_metadata.insert(
                RESPONSES_TOOL_SIGNATURES_METADATA_KEY.to_string(),
                Value::Array(tool_signatures),
            );
        }
    }

    if let Some(tool_choice) = body
        .get("tool_choice")
        .filter(|tool_choice| should_preserve_raw_responses_tool_choice(tool_choice))
    {
        request.transformer_metadata.insert(
            RESPONSES_RAW_TOOL_CHOICE_METADATA_KEY.to_string(),
            tool_choice.clone(),
        );
    }

    if let Some(input_items) = body.get("input").and_then(Value::as_array) {
        let raw_input_items = input_items
            .iter()
            .enumerate()
            .filter(|(_, item)| !is_structurally_represented_responses_input_item(item))
            .map(|(index, item)| raw_responses_fragment(index, item.clone()))
            .collect::<Vec<_>>();
        if !raw_input_items.is_empty() {
            request.transformer_metadata.insert(
                RESPONSES_RAW_INPUT_ITEMS_METADATA_KEY.to_string(),
                Value::Array(raw_input_items),
            );
        }
    }
}

fn raw_responses_fragment(index: usize, value: Value) -> Value {
    json!({
        "index": index,
        "value": value
    })
}

fn is_structurally_represented_responses_tool(tool: &Value) -> bool {
    matches!(
        tool.get("type").and_then(Value::as_str),
        Some("function" | "custom")
    )
}

fn responses_tool_signature(tool: &Value) -> Option<String> {
    match tool.get("type").and_then(Value::as_str)? {
        tool_type @ ("function" | "custom") => tool
            .get("name")
            .and_then(Value::as_str)
            .map(|name| format!("{tool_type}:{name}")),
        tool_type => Some(tool_type.to_string()),
    }
}

fn should_preserve_raw_responses_tool_choice(tool_choice: &Value) -> bool {
    match tool_choice {
        Value::Object(object) => {
            object.get("tools").is_some() || tool_choice_from_openai(Some(tool_choice)).is_none()
        }
        _ => false,
    }
}

fn is_structurally_represented_responses_input_item(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        None | Some(
            "message"
                | "input_text"
                | "input_image"
                | "function_call"
                | "function_call_output"
                | "custom_tool_call"
                | "custom_tool_call_output"
                | "reasoning"
                | "compaction"
                | "compaction_summary"
        )
    )
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
            let mut pending_trailing_reasoning: Option<Message> = None;
            while index < items.len() {
                let item = &items[index];
                if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    // Flush any previous trailing reasoning before starting a new one.
                    if let Some(pending) = pending_trailing_reasoning.take() {
                        attach_pending_reasoning_to_previous_assistant(messages, pending);
                    }
                    let mut reasoning_message = responses_reasoning_message(item);
                    if items.get(index + 1).is_some_and(|following| {
                        merge_responses_following_item_into_reasoning_message(
                            &mut reasoning_message,
                            following,
                        )
                    }) {
                        // Forward-merged with following item — not trailing.
                        messages.push(reasoning_message);
                        index += 2;
                    } else {
                        // May be trailing: hold until next non-reasoning boundary or end.
                        pending_trailing_reasoning = Some(reasoning_message);
                        index += 1;
                    }
                    continue;
                }

                // User (or other non-assistant) boundary: attach pending trailing to previous assistant.
                let item_role = responses_item_boundary_role(item);
                if item_role.as_deref() == Some("user") {
                    if let Some(pending) = pending_trailing_reasoning.take() {
                        attach_pending_reasoning_to_previous_assistant(messages, pending);
                    }
                } else if let Some(pending) = pending_trailing_reasoning.take() {
                    // Non-user item after bare reasoning: try attach, else emit standalone.
                    if !attach_pending_reasoning_to_previous_assistant(messages, pending.clone()) {
                        messages.push(pending);
                    }
                }

                append_responses_item_to_messages(item, messages);
                index += 1;
            }
            if let Some(pending) = pending_trailing_reasoning.take() {
                if !attach_pending_reasoning_to_previous_assistant(messages, pending.clone()) {
                    messages.push(pending);
                }
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

/// Attach trailing reasoning fields onto the last assistant message (append, not replace).
/// Returns true if attached.
fn attach_pending_reasoning_to_previous_assistant(
    messages: &mut [Message],
    pending: Message,
) -> bool {
    let Some(assistant) = messages.iter_mut().rev().find(|message| message.role == "assistant")
    else {
        return false;
    };
    if let Some(text) = pending.reasoning_content.filter(|text| !text.is_empty()) {
        match &mut assistant.reasoning_content {
            Some(existing) if !existing.is_empty() => {
                existing.push('\n');
                existing.push_str(&text);
            }
            _ => assistant.reasoning_content = Some(text),
        }
    }
    if let Some(text) = pending.reasoning.filter(|text| !text.is_empty()) {
        match &mut assistant.reasoning {
            Some(existing) if !existing.is_empty() => {
                existing.push('\n');
                existing.push_str(&text);
            }
            _ => assistant.reasoning = Some(text),
        }
    }
    if assistant.reasoning_signature.is_none() {
        assistant.reasoning_signature = pending.reasoning_signature;
    }
    for (key, value) in pending.transformer_metadata {
        assistant.transformer_metadata.entry(key).or_insert(value);
    }
    true
}

fn responses_item_boundary_role(item: &Value) -> Option<String> {
    match item.get("type").and_then(Value::as_str) {
        Some("message") | None => item
            .get("role")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        Some("input_text") | Some("input_image") => Some("user".to_string()),
        Some("function_call") | Some("custom_tool_call") | Some("output_text") => {
            Some("assistant".to_string())
        }
        Some("function_call_output") | Some("custom_tool_call_output") => Some("tool".to_string()),
        _ => None,
    }
}

fn append_responses_item_to_messages(item: &Value, messages: &mut Vec<Message>) {
    match item.get("type").and_then(Value::as_str) {
        Some("input_text") | Some("output_text") | Some("text") => messages.push(Message {
            role: responses_text_item_role(item),
            content: responses_value_to_message_content(item),
            annotations: part_annotations(item),
            ..Default::default()
        }),
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
            content: item
                .get("output")
                .map(responses_value_to_message_content)
                .unwrap_or_default(),
            ..Default::default()
        }),
        Some("reasoning") => messages.push(responses_reasoning_message(item)),
        Some("compaction") | Some("compaction_summary") => {
            messages.push(responses_compaction_message(item))
        }
        Some("input_image") => {
            if let Some(part) = responses_input_image_part(item) {
                messages.push(Message {
                    role: "user".to_string(),
                    content: MessageContent::Parts(vec![part]),
                    ..Default::default()
                });
            }
        }
        None | Some("message") => messages.push(responses_message_item_to_llm(item)),
        _ => {}
    }
}

fn responses_text_item_role(item: &Value) -> String {
    item.get("role")
        .and_then(Value::as_str)
        .filter(|role| !role.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if item.get("type").and_then(Value::as_str) == Some("output_text") {
                "assistant".to_string()
            } else {
                "user".to_string()
            }
        })
}

fn responses_value_to_message_content(value: &Value) -> MessageContent {
    match value {
        Value::String(text) => MessageContent::Text(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(responses_content_part_to_llm)
                .collect::<Vec<_>>();
            if parts.len() != items.len() {
                return MessageContent::Text(responses_string_or_compact_json(value));
            }
            let has_annotations = items.iter().any(|item| {
                item.get("annotations")
                    .and_then(Value::as_array)
                    .is_some_and(|annotations| !annotations.is_empty())
            });
            if parts.len() == 1
                && parts[0].part_type == "text"
                && parts[0].id.is_empty()
                && !has_annotations
            {
                if let Some(text) = parts[0].text.clone() {
                    return MessageContent::Text(text);
                }
            }
            MessageContent::Parts(parts)
        }
        Value::Object(_) => {
            if value.get("text").is_some() || value.get("type").is_some() {
                if let Some(part) = responses_content_part_to_llm(value) {
                    return MessageContent::Parts(vec![part]);
                }
            }
            MessageContent::Text(value.to_string())
        }
        Value::Null => MessageContent::Empty,
        other => MessageContent::Text(other.to_string()),
    }
}

fn responses_reasoning_message(item: &Value) -> Message {
    let reasoning = responses_reasoning_text(item);
    let mut message = Message {
        role: "assistant".to_string(),
        reasoning_content: reasoning.clone(),
        reasoning,
        reasoning_signature: item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .filter(|signature| !signature.is_empty())
            .map(|signature| encode_signature(SignatureProvider::OpenAiResponses, signature)),
        ..Default::default()
    };
    // Preserve Responses-only reasoning.context for Responses↔Responses IR rebuild (T-1).
    if let Some(context) = item.get("context") {
        message.transformer_metadata.insert(
            "openai_responses_reasoning_context".to_string(),
            context.clone(),
        );
    }
    message
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
            // Only forward-merge assistant/output messages. User turns are a
            // trailing-reasoning boundary and must not absorb the reasoning item.
            let role = following
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("assistant");
            if role != "assistant" {
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
    let mut content = MessageContent::Empty;
    if let Some(content_value) = item.get("content") {
        if let Some(content_parts) = content_value.as_array() {
            for part in content_parts {
                if part.get("type").and_then(Value::as_str) == Some("refusal") {
                    refusal = part
                        .get("refusal")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    continue;
                }
                if let Some(part) = responses_content_part_to_llm(part) {
                    parts.push(part);
                }
            }
            content = MessageContent::Parts(parts);
        } else {
            content = responses_value_to_message_content(content_value);
        }
    }
    Message {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        role,
        content,
        refusal,
        annotations: item
            .get("content")
            .map(content_annotations_from_value)
            .unwrap_or_default(),
        ..Default::default()
    }
}

fn responses_content_part_to_llm(part: &Value) -> Option<MessageContentPart> {
    match part.get("type").and_then(Value::as_str) {
        Some("input_text") | Some("output_text") | Some("text") => Some(MessageContentPart {
            id: part
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            part_type: "text".to_string(),
            text: part
                .get("text")
                .or_else(|| part.get("content"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            ..Default::default()
        }),
        Some("input_image") => responses_input_image_part(part),
        Some("compaction") | Some("compaction_summary") => Some(responses_compaction_part(part)),
        _ => None,
    }
}

fn responses_compaction_message(item: &Value) -> Message {
    Message {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        role: "assistant".to_string(),
        content: MessageContent::Parts(vec![responses_compaction_part(item)]),
        ..Default::default()
    }
}

fn responses_compaction_part(item: &Value) -> MessageContentPart {
    let mut transformer_metadata = HashMap::new();
    if let Some(encrypted_content) = item.get("encrypted_content").and_then(Value::as_str) {
        transformer_metadata.insert(
            RESPONSES_COMPACTION_ENCRYPTED_CONTENT_METADATA_KEY.to_string(),
            json!(encrypted_content),
        );
    }
    if let Some(created_by) = item.get("created_by").and_then(Value::as_str) {
        transformer_metadata.insert(
            RESPONSES_COMPACTION_CREATED_BY_METADATA_KEY.to_string(),
            json!(created_by),
        );
    }
    MessageContentPart {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        part_type: item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("compaction")
            .to_string(),
        transformer_metadata,
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
            .map(responses_string_or_compact_json)
            .unwrap_or_default();
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
        &item
            .get("arguments")
            .map(responses_string_or_compact_json)
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

fn responses_string_or_compact_json(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
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

fn merge_raw_responses_fragments(
    mut structured: Vec<Value>,
    raw_fragments: Option<&Value>,
) -> Vec<Value> {
    merge_raw_responses_fragments_with_signatures(&mut structured, raw_fragments, None)
}

/// Merge raw Responses fragments back by original index.
/// When `expected_signatures` is present (from request-scoped tool signature sidecar),
/// a raw tool that collides with a structured tool signature is dropped (fail-closed
/// for that fragment) instead of silently overwriting structured identity (T-3).
fn merge_raw_responses_fragments_with_signatures(
    structured: &mut Vec<Value>,
    raw_fragments: Option<&Value>,
    expected_signatures: Option<&[String]>,
) -> Vec<Value> {
    let raw = raw_fragments
        .and_then(Value::as_array)
        .map(|items| {
            let max_merge_index = structured.len().saturating_add(items.len());
            items
                .iter()
                .filter_map(raw_responses_fragment_parts)
                .filter(|(index, _)| *index <= max_merge_index)
                .filter(|(_, value)| {
                    if let Some(signatures) = expected_signatures {
                        raw_tool_compatible_with_signatures(value, signatures)
                    } else {
                        true
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if raw.is_empty() {
        return std::mem::take(structured);
    }

    let mut total = structured.len() + raw.len();
    for (index, _) in &raw {
        total = total.max(index.saturating_add(1));
    }

    let mut raw_by_index = raw.into_iter().collect::<HashMap<_, _>>();
    let mut merged = Vec::with_capacity(total);
    let mut structured_iter = structured.drain(..);
    for index in 0..total {
        if let Some(value) = raw_by_index.remove(&index) {
            merged.push(value);
        } else if let Some(value) = structured_iter.next() {
            merged.push(value);
        }
    }
    merged.extend(structured_iter);
    merged.extend(
        raw_by_index
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>(),
    );
    merged
}

/// Drop raw tools that collide with a collected structured tool signature
/// (same type:name) — prevents incorrect merge of hosted/raw tools over function tools.
fn raw_tool_compatible_with_signatures(raw_tool: &Value, signatures: &[String]) -> bool {
    let Some(raw_sig) = responses_tool_signature(raw_tool) else {
        return true;
    };
    // If the raw tool's signature is already represented in structured tools, drop it.
    !signatures.iter().any(|expected| expected == &raw_sig)
}

fn raw_responses_fragment_parts(value: &Value) -> Option<(usize, Value)> {
    let index = value.get("index").and_then(Value::as_u64)? as usize;
    let raw_value = value.get("value")?.clone();
    Some((index, raw_value))
}

fn responses_metadata_or_extra_body(
    request: &Request,
    metadata_key: &str,
    extra_body_key: &str,
) -> Option<Value> {
    request
        .transformer_metadata
        .get(metadata_key)
        .cloned()
        .or_else(|| {
            request
                .extra_body
                .as_ref()
                .and_then(|extra_body| extra_body.get(extra_body_key).cloned())
        })
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
    let assistant = role == "assistant";
    let has_tool_calls = !message.tool_calls.is_empty();
    let has_message_content =
        has_responses_message_content(&message.content, &message.refusal, assistant);
    if !has_tool_calls || has_message_content {
        append_responses_message_content_items(
            role,
            message.content,
            message.annotations,
            message.refusal,
            input,
        );
    }
    for tool_call in message.tool_calls {
        if tool_call.tool_type == TOOL_TYPE_RESPONSES_CUSTOM_TOOL {
            custom_tool_call_ids.insert(tool_call.id.clone(), true);
        }
        input.push(tool_call_to_responses_item(tool_call));
    }
}

fn has_responses_message_content(content: &MessageContent, refusal: &str, assistant: bool) -> bool {
    if assistant && !refusal.is_empty() {
        return true;
    }
    match content {
        MessageContent::Text(text) => !text.is_empty(),
        MessageContent::Parts(parts) => parts.iter().any(|part| {
            is_responses_compaction_type(&part.part_type)
                || matches!(
                    part.part_type.as_str(),
                    "image_url" | "input_image" | "refusal"
                )
                || matches!(
                    part.part_type.as_str(),
                    "text" | "input_text" | "output_text"
                ) && part.text.as_deref().is_some_and(|text| !text.is_empty())
        }),
        MessageContent::Empty => false,
    }
}

fn append_responses_message_content_items(
    role: String,
    content: MessageContent,
    annotations: Vec<Value>,
    refusal: String,
    input: &mut Vec<Value>,
) {
    let assistant = role == "assistant";
    let mut pending_content = Vec::new();
    match content {
        MessageContent::Parts(parts) => {
            for part in parts {
                if is_responses_compaction_type(&part.part_type) {
                    flush_responses_message_content(&role, &mut pending_content, input);
                    input.push(responses_compaction_item_from_part(part));
                    continue;
                }
                if let Some(item) =
                    llm_content_part_to_responses_content(part, assistant, &annotations)
                {
                    pending_content.push(item);
                }
            }
        }
        other => pending_content.extend(llm_content_to_responses_content(
            other,
            assistant,
            annotations,
            String::new(),
        )),
    }
    if assistant && !refusal.is_empty() {
        pending_content.push(json!({ "type": "refusal", "refusal": refusal }));
    }
    flush_responses_message_content(&role, &mut pending_content, input);
}

fn flush_responses_message_content(role: &str, content: &mut Vec<Value>, input: &mut Vec<Value>) {
    if content.is_empty() {
        return;
    }
    input.push(json!({
        "type": "message",
        "role": role,
        "content": std::mem::take(content)
    }));
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
    if let Some(context) = message
        .transformer_metadata
        .get("openai_responses_reasoning_context")
    {
        item["context"] = context.clone();
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
            .filter_map(|part| llm_content_part_to_responses_content(part, assistant, &annotations))
            .collect(),
        MessageContent::Empty => Vec::new(),
    };
    if assistant && !refusal.is_empty() {
        result.push(json!({ "type": "refusal", "refusal": refusal }));
    }
    result
}

fn llm_content_part_to_responses_content(
    part: MessageContentPart,
    assistant: bool,
    annotations: &[Value],
) -> Option<Value> {
    let text_type = if assistant {
        "output_text"
    } else {
        "input_text"
    };
    match part.part_type.as_str() {
        "text" | "input_text" | "output_text" => Some(text_content_item(
            text_type,
            part.text.unwrap_or_default(),
            assistant,
            annotations,
        )),
        "image_url" | "input_image" => Some(json!({
            "type": "input_image",
            "image_url": part.image_url.map(|image| image.url).unwrap_or_default()
        })),
        _ => None,
    }
}

fn is_responses_compaction_type(part_type: &str) -> bool {
    matches!(part_type, "compaction" | "compaction_summary")
}

fn responses_compaction_item_from_part(part: MessageContentPart) -> Value {
    let mut item = Map::new();
    item.insert("type".to_string(), json!(part.part_type));
    if !part.id.is_empty() {
        item.insert("id".to_string(), json!(part.id));
    }
    if let Some(encrypted_content) = part
        .transformer_metadata
        .get(RESPONSES_COMPACTION_ENCRYPTED_CONTENT_METADATA_KEY)
        .and_then(Value::as_str)
    {
        item.insert("encrypted_content".to_string(), json!(encrypted_content));
    }
    if let Some(created_by) = part
        .transformer_metadata
        .get(RESPONSES_COMPACTION_CREATED_BY_METADATA_KEY)
        .and_then(Value::as_str)
    {
        item.insert("created_by".to_string(), json!(created_by));
    }
    Value::Object(item)
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
                    if let Some(content) = item.get("content") {
                        if let Some(content_array) = content.as_array() {
                            for part in content_array {
                                match part.get("type").and_then(Value::as_str) {
                                    Some("output_text") | Some("input_text") | Some("text") => {
                                        if let Some(annotations) =
                                            part.get("annotations").and_then(Value::as_array)
                                        {
                                            message.annotations.extend(annotations.iter().cloned());
                                        }
                                        if let Some(part) = responses_content_part_to_llm(part) {
                                            parts.push(part);
                                        }
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
                        } else {
                            message
                                .annotations
                                .extend(content_annotations_from_value(content));
                            match responses_value_to_message_content(content) {
                                MessageContent::Text(text) => parts.push(MessageContentPart {
                                    part_type: "text".to_string(),
                                    text: Some(text),
                                    ..Default::default()
                                }),
                                MessageContent::Parts(content_parts) => parts.extend(content_parts),
                                MessageContent::Empty => {}
                            }
                        }
                    }
                }
                Some("output_text") | Some("input_text") | Some("text") => {
                    message.annotations.extend(part_annotations(item));
                    if let Some(part) = responses_content_part_to_llm(item) {
                        parts.push(part);
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
                Some("compaction") | Some("compaction_summary") => {
                    parts.push(responses_compaction_part(item));
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

pub fn responses_compact_response_to_llm(body: Value) -> Response {
    let mut response = responses_response_to_llm(body);
    response.object = "response.compaction".to_string();
    response
}

pub fn llm_response_to_responses(response: Response) -> Value {
    let previous_response_id = response.previous_response_id.clone();
    let choice = response.choices.first().cloned().unwrap_or_default();
    let mut output = Vec::new();
    if let Some(reasoning_item) = responses_reasoning_item_from_message(&choice.message) {
        output.push(reasoning_item);
    }
    append_responses_message_content_items(
        "assistant".to_string(),
        choice.message.content.clone(),
        choice.message.annotations.clone(),
        choice.message.refusal.clone(),
        &mut output,
    );
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

pub fn llm_response_to_responses_compact(response: Response) -> Value {
    let mut body = llm_response_to_responses(response);
    if let Some(object) = body.as_object_mut() {
        object.insert("object".to_string(), json!("response.compaction"));
        if !object.contains_key("status") && !object.contains_key("error") {
            object.insert("status".to_string(), json!("completed"));
        }
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
    let mut parameters = normalize_function_parameters_owned(parameters);
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
    content.iter().flat_map(part_annotations).collect()
}

fn content_annotations_from_value(content: &Value) -> Vec<Value> {
    if let Some(parts) = content.as_array() {
        return content_annotations(parts);
    }
    part_annotations(content)
}

fn part_annotations(part: &Value) -> Vec<Value> {
    part.get("annotations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::transformer::llm::ResponseCustomTool;

    #[test]
    fn responses_request_accepts_raw_message_and_input_shapes() {
        let llm = responses_request_to_llm(json!({
            "model": "gpt-5",
            "input": [
                {"type": "input_text", "text": "standalone text"},
                {"type": "message", "role": "user", "content": "string content"},
                {
                    "type": "function_call",
                    "call_id": "call_lookup",
                    "name": "lookup",
                    "arguments": {"query": "rust"}
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_lookup",
                    "output": [{"type": "output_text", "text": "tool output"}]
                }
            ]
        }));

        assert_eq!(llm.messages.len(), 4);
        let first_parts = match &llm.messages[0].content {
            MessageContent::Parts(parts) => parts,
            other => panic!("expected standalone text part, got {other:?}"),
        };
        assert_eq!(first_parts[0].text.as_deref(), Some("standalone text"));
        assert_eq!(
            llm.messages[1].content,
            MessageContent::Text("string content".to_string())
        );
        assert_eq!(
            llm.messages[2].tool_calls[0].function.arguments,
            r#"{"query":"rust"}"#
        );
        assert_eq!(
            llm.messages[3].content,
            MessageContent::Text("tool output".to_string())
        );
    }

    #[test]
    fn responses_request_preserves_raw_json_array_tool_output() {
        let llm = responses_request_to_llm(json!({
            "model": "gpt-5",
            "input": [{
                "type": "function_call_output",
                "call_id": "call_lookup",
                "output": [{"value": 1}]
            }]
        }));

        assert_eq!(
            llm.messages[0].content,
            MessageContent::Text(r#"[{"value":1}]"#.to_string())
        );
    }

    #[test]
    fn responses_response_accepts_top_level_output_text() {
        let llm = responses_response_to_llm(json!({
            "id": "resp_text",
            "object": "response",
            "created_at": 123,
            "status": "completed",
            "model": "gpt-5",
            "output": [
                {"type": "output_text", "text": "hello", "annotations": [{"type": "url_citation"}]}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        }));

        let message = &llm.choices[0].message;
        let parts = match &message.content {
            MessageContent::Parts(parts) => parts,
            other => panic!("expected output text part, got {other:?}"),
        };
        assert_eq!(parts[0].text.as_deref(), Some("hello"));
        assert_eq!(message.annotations, vec![json!({"type": "url_citation"})]);
    }

    #[test]
    fn responses_response_accepts_message_content_object() {
        let llm = responses_response_to_llm(json!({
            "id": "resp_content_object",
            "object": "response",
            "created_at": 123,
            "status": "completed",
            "model": "gpt-5",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": {
                    "type": "output_text",
                    "text": "object text",
                    "annotations": [{"type": "url_citation"}]
                }
            }]
        }));

        let message = &llm.choices[0].message;
        let parts = match &message.content {
            MessageContent::Parts(parts) => parts,
            other => panic!("expected output text part, got {other:?}"),
        };
        assert_eq!(parts[0].text.as_deref(), Some("object text"));
        assert_eq!(message.annotations, vec![json!({"type": "url_citation"})]);
    }

    #[test]
    fn responses_tools_omit_empty_optional_fields() {
        let responses = llm_request_to_responses(Request {
            model: "gpt-5".to_string(),
            tools: vec![
                Tool {
                    tool_type: TOOL_TYPE_RESPONSES_CUSTOM_TOOL.to_string(),
                    response_custom_tool: Some(ResponseCustomTool {
                        name: "freeform".to_string(),
                        description: String::new(),
                        format: None,
                    }),
                    ..Default::default()
                },
                Tool {
                    tool_type: TOOL_TYPE_FUNCTION.to_string(),
                    function: Some(Function {
                        name: "lookup".to_string(),
                        description: String::new(),
                        parameters: Some(json!({"type": "object"})),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });

        assert!(responses["tools"][0].get("description").is_none());
        assert!(responses["tools"][0].get("format").is_none());
        assert!(responses["tools"][1].get("description").is_none());
    }

    #[test]
    fn responses_request_roundtrip_preserves_compaction_items() {
        let llm = responses_request_to_llm(json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "before"}]
                },
                {
                    "type": "compaction",
                    "id": "cmp_1",
                    "encrypted_content": "encrypted_compaction",
                    "created_by": "model"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "after"}]
                }
            ]
        }));

        assert_eq!(llm.messages.len(), 3);
        let compaction_part = match &llm.messages[1].content {
            MessageContent::Parts(parts) => &parts[0],
            other => panic!("expected compaction part, got {other:?}"),
        };
        assert_eq!(compaction_part.part_type, "compaction");
        assert_eq!(compaction_part.id, "cmp_1");
        assert_eq!(
            compaction_part
                .transformer_metadata
                .get(RESPONSES_COMPACTION_ENCRYPTED_CONTENT_METADATA_KEY),
            Some(&json!("encrypted_compaction"))
        );

        let converted = llm_request_to_responses(llm);
        let input = converted["input"].as_array().expect("responses input");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["content"][0]["text"], "before");
        assert_eq!(input[1]["type"], "compaction");
        assert_eq!(input[1]["id"], "cmp_1");
        assert_eq!(input[1]["encrypted_content"], "encrypted_compaction");
        assert_eq!(input[1]["created_by"], "model");
        assert_eq!(input[2]["type"], "message");
        assert_eq!(input[2]["content"][0]["text"], "after");
    }

    #[test]
    fn responses_compact_roundtrip_preserves_raw_only_request_fragments() {
        let raw_tool_choice = json!({
            "type": "allowed_tools",
            "mode": "auto",
            "tools": [{"type": "mcp", "server_label": "local"}]
        });
        let compact = responses_compact_request_to_llm(json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "before"}]
                },
                {
                    "type": "local_shell_call",
                    "id": "shell_1",
                    "call_id": "call_shell",
                    "status": "completed",
                    "action": {"command": "pwd"}
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "after"}]
                }
            ],
            "tools": [
                {"type": "function", "name": "known", "parameters": {"type": "object"}},
                {"type": "mcp", "server_label": "local", "server_url": "http://localhost:3000"},
                {"type": "custom", "name": "freeform"}
            ],
            "tool_choice": raw_tool_choice
        }));

        let converted = llm_request_to_responses_compact(compact);
        let input = converted["input"].as_array().expect("responses input");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[1]["type"], "local_shell_call");
        assert_eq!(input[1]["action"]["command"], "pwd");
        assert_eq!(input[2]["type"], "message");

        let tools = converted["tools"].as_array().expect("responses tools");
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[1]["type"], "mcp");
        assert_eq!(tools[1]["server_url"], "http://localhost:3000");
        assert_eq!(tools[2]["type"], "custom");
        assert_eq!(converted["tool_choice"], raw_tool_choice);
        assert!(converted.get("stream").is_none());
    }

    #[test]
    fn responses_response_roundtrip_preserves_compaction_summary() {
        let llm = responses_response_to_llm(json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 123,
            "status": "completed",
            "model": "gpt-5",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "summary before"}]
                },
                {
                    "type": "compaction_summary",
                    "id": "cmp_summary_1",
                    "encrypted_content": "encrypted_summary",
                    "created_by": "model"
                }
            ],
            "usage": {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3}
        }));

        let message = &llm.choices[0].message;
        let parts = match &message.content {
            MessageContent::Parts(parts) => parts,
            other => panic!("expected response parts, got {other:?}"),
        };
        assert_eq!(parts[0].part_type, "text");
        assert_eq!(parts[1].part_type, "compaction_summary");

        let converted = llm_response_to_responses(llm);
        let output = converted["output"].as_array().expect("responses output");
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["text"], "summary before");
        assert_eq!(output[1]["type"], "compaction_summary");
        assert_eq!(output[1]["id"], "cmp_summary_1");
        assert_eq!(output[1]["encrypted_content"], "encrypted_summary");
        assert_eq!(output[1]["created_by"], "model");
    }

    #[test]
    fn llm_response_to_responses_preserves_text_compaction_text_order() {
        let mut metadata = HashMap::new();
        metadata.insert(
            RESPONSES_COMPACTION_ENCRYPTED_CONTENT_METADATA_KEY.to_string(),
            json!("encrypted_mid"),
        );
        let response = Response {
            id: "resp_order".to_string(),
            model: "gpt-5".to_string(),
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: "assistant".to_string(),
                    content: MessageContent::Parts(vec![
                        MessageContentPart {
                            part_type: "text".to_string(),
                            text: Some("first".to_string()),
                            ..Default::default()
                        },
                        MessageContentPart {
                            id: "cmp_mid".to_string(),
                            part_type: "compaction".to_string(),
                            transformer_metadata: metadata,
                            ..Default::default()
                        },
                        MessageContentPart {
                            part_type: "text".to_string(),
                            text: Some("second".to_string()),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                },
                finish_reason: Some("stop".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let converted = llm_response_to_responses(response);
        let output = converted["output"].as_array().expect("responses output");
        assert_eq!(output.len(), 3);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["text"], "first");
        assert_eq!(output[1]["type"], "compaction");
        assert_eq!(output[1]["id"], "cmp_mid");
        assert_eq!(output[1]["encrypted_content"], "encrypted_mid");
        assert_eq!(output[2]["type"], "message");
        assert_eq!(output[2]["content"][0]["text"], "second");
    }

    #[test]
    fn trailing_reasoning_attaches_to_previous_assistant_before_user() {
        let body = json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "done"}]
                },
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "trailing thought"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "next"}]
                }
            ]
        });
        let request = responses_request_to_llm(body);
        let assistant = request
            .messages
            .iter()
            .find(|message| message.role == "assistant")
            .expect("assistant");
        assert_eq!(
            assistant.reasoning_content.as_deref(),
            Some("trailing thought")
        );
        assert!(
            !request
                .messages
                .iter()
                .any(|message| message.role == "assistant"
                    && message.content.is_empty()
                    && message.reasoning_content.as_deref() == Some("trailing thought")
                    && message.tool_calls.is_empty()),
            "trailing reasoning must not remain a standalone assistant message"
        );
        let user = request
            .messages
            .iter()
            .find(|message| message.role == "user")
            .expect("user");
        let user_has_next = match &user.content {
            MessageContent::Text(text) => text == "next",
            MessageContent::Parts(parts) => parts
                .iter()
                .any(|part| part.text.as_deref() == Some("next")),
            MessageContent::Empty => false,
        };
        assert!(user_has_next, "user message should carry next turn text");
    }

    #[test]
    fn trailing_reasoning_at_input_end_attaches_to_previous_assistant() {
        let body = json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "answer"}],
                },
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "end trailing"}],
                    "context": {"foo": "bar"}
                }
            ]
        });
        let request = responses_request_to_llm(body);
        assert_eq!(request.messages.len(), 1);
        assert_eq!(
            request.messages[0].reasoning_content.as_deref(),
            Some("end trailing")
        );
        assert_eq!(
            request.messages[0]
                .transformer_metadata
                .get("openai_responses_reasoning_context"),
            Some(&json!({"foo": "bar"}))
        );
    }

    #[test]
    fn forward_merge_still_combines_reasoning_with_following_function_call() {
        let body = json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "plan"}]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "lookup",
                    "arguments": "{}"
                }
            ]
        });
        let request = responses_request_to_llm(body);
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "assistant");
        assert_eq!(request.messages[0].reasoning_content.as_deref(), Some("plan"));
        assert_eq!(request.messages[0].tool_calls.len(), 1);
        assert_eq!(request.messages[0].tool_calls[0].id, "call_1");
    }

    #[test]
    fn trailing_reasoning_appends_to_assistant_that_already_has_tool_calls() {
        let body = json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "lookup",
                    "arguments": "{}"
                },
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "after tools"}]
                }
            ]
        });
        let request = responses_request_to_llm(body);
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "assistant");
        assert_eq!(request.messages[0].tool_calls.len(), 1);
        assert_eq!(
            request.messages[0].reasoning_content.as_deref(),
            Some("after tools")
        );
    }

    #[test]
    fn trailing_reasoning_appends_after_embedded_forward_merge() {
        // reasoning + following assistant message (forward merge), then another
        // trailing reasoning before user must append to that same assistant.
        let body = json!({
            "model": "gpt-5",
            "input": [
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "first"}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "visible"}]
                },
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "second trailing"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "next"}]
                }
            ]
        });
        let request = responses_request_to_llm(body);
        let assistant = request
            .messages
            .iter()
            .find(|message| message.role == "assistant")
            .expect("assistant");
        let reasoning = assistant.reasoning_content.as_deref().unwrap_or_default();
        assert!(
            reasoning.contains("first") && reasoning.contains("second trailing"),
            "expected both forward-merged and trailing reasoning, got {reasoning:?}"
        );
    }

    #[test]
    fn request_top_level_reasoning_context_roundtrips_without_clobbering_effort() {
        // Audit T-1: preserve request-level reasoning.context (e.g. "all_turns"),
        // not just input-item context. Outbound must merge effort + context.
        let body = json!({
            "model": "gpt-5",
            "reasoning": {
                "effort": "high",
                "context": "all_turns"
            },
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hi"}]
                }
            ]
        });
        let request = responses_request_to_llm(body);
        assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(
            request
                .transformer_metadata
                .get(RESPONSES_REQUEST_REASONING_CONTEXT_METADATA_KEY),
            Some(&json!("all_turns"))
        );

        let converted = llm_request_to_responses(request);
        assert_eq!(converted["reasoning"]["effort"], "high");
        assert_eq!(converted["reasoning"]["context"], "all_turns");
    }

    #[test]
    fn request_reasoning_context_only_without_effort_is_preserved() {
        let body = json!({
            "model": "gpt-5",
            "reasoning": { "context": "all_turns" },
            "input": "hello"
        });
        let request = responses_request_to_llm(body);
        assert!(request.reasoning_effort.is_none());
        let converted = llm_request_to_responses(request);
        assert!(converted["reasoning"].get("effort").is_none());
        assert_eq!(converted["reasoning"]["context"], "all_turns");
    }

    #[test]
    fn raw_tools_merge_drops_raw_tool_colliding_with_structured_signature() {
        // Structured function tool signature is collected on inbound; a raw tool
        // fragment with the same type:name must be dropped (T-3 fail-closed).
        let mut structured = vec![json!({
            "type": "function",
            "name": "lookup",
            "parameters": {"type": "object"}
        })];
        let raw_tools = json!([
            {
                "index": 0,
                "value": {
                    "type": "function",
                    "name": "lookup",
                    "description": "raw collision should drop"
                }
            },
            {
                "index": 1,
                "value": {
                    "type": "web_search"
                }
            }
        ]);
        let signatures = vec!["function:lookup".to_string()];
        let merged = merge_raw_responses_fragments_with_signatures(
            &mut structured,
            Some(&raw_tools),
            Some(signatures.as_slice()),
        );
        let names: Vec<String> = merged
            .iter()
            .filter_map(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| {
                        tool.get("type")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    })
            })
            .collect();
        assert!(
            names.iter().filter(|name| name.as_str() == "lookup").count() == 1,
            "colliding raw function:lookup must be dropped, got {names:?}"
        );
        assert!(
            names.iter().any(|name| name == "web_search"),
            "non-colliding raw tool should remain, got {names:?}"
        );
    }
}
