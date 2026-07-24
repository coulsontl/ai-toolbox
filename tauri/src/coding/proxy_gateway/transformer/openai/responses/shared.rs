use super::super::chat::openai_usage_to_llm;
use crate::coding::proxy_gateway::transformer::llm::{
    Function, FunctionCall, ImageUrl, Message, MessageContent, MessageContentPart, Request,
    ResponseCustomToolCall, Tool, ToolCall, Usage, TOOL_TYPE_FUNCTION,
    TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
};
use crate::coding::proxy_gateway::transformer::shared::signature::{
    decode_signature_for, encode_signature, SignatureProvider,
};
use crate::coding::proxy_gateway::transformer::shared::{
    normalize_function_parameters_owned, tool_choice_from_openai,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub(crate) const RESPONSES_INCLUDE_METADATA_KEY: &str = "openai_responses_include";
pub(crate) const RESPONSES_MAX_TOOL_CALLS_METADATA_KEY: &str = "openai_responses_max_tool_calls";
pub(crate) const RESPONSES_PROMPT_CACHE_RETENTION_METADATA_KEY: &str =
    "openai_responses_prompt_cache_retention";
pub(crate) const RESPONSES_TRUNCATION_METADATA_KEY: &str = "openai_responses_truncation";
pub(crate) const RESPONSES_COMPACTION_ENCRYPTED_CONTENT_METADATA_KEY: &str =
    "openai_responses_compaction_encrypted_content";
pub(crate) const RESPONSES_COMPACTION_CREATED_BY_METADATA_KEY: &str = "openai_responses_compaction_created_by";
pub(crate) const RESPONSES_RAW_TOOLS_METADATA_KEY: &str = "openai_responses_raw_tools";
pub(crate) const RESPONSES_TOOL_SIGNATURES_METADATA_KEY: &str = "openai_responses_tool_signatures";
pub(crate) const RESPONSES_RAW_TOOL_CHOICE_METADATA_KEY: &str = "openai_responses_raw_tool_choice";
pub(crate) const RESPONSES_RAW_INPUT_ITEMS_METADATA_KEY: &str = "openai_responses_raw_input_items";
/// Request top-level `reasoning.context` (e.g. `"all_turns"`), not input-item context.
pub(crate) const RESPONSES_REQUEST_REASONING_CONTEXT_METADATA_KEY: &str =
    "openai_responses_request_reasoning_context";


pub(super) fn preserve_responses_transformer_metadata(
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

pub(super) fn attach_responses_raw_request_metadata(body: &Value, request: &mut Request) {
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

pub(super) fn raw_responses_fragment(index: usize, value: Value) -> Value {
    json!({
        "index": index,
        "value": value
    })
}

pub(super) fn is_structurally_represented_responses_tool(tool: &Value) -> bool {
    matches!(
        tool.get("type").and_then(Value::as_str),
        Some("function" | "custom")
    )
}

pub(super) fn responses_tool_signature(tool: &Value) -> Option<String> {
    match tool.get("type").and_then(Value::as_str)? {
        tool_type @ ("function" | "custom") => tool
            .get("name")
            .and_then(Value::as_str)
            .map(|name| format!("{tool_type}:{name}")),
        tool_type => Some(tool_type.to_string()),
    }
}

pub(super) fn should_preserve_raw_responses_tool_choice(tool_choice: &Value) -> bool {
    match tool_choice {
        Value::Object(object) => {
            object.get("tools").is_some() || tool_choice_from_openai(Some(tool_choice)).is_none()
        }
        _ => false,
    }
}

pub(super) fn is_structurally_represented_responses_input_item(item: &Value) -> bool {
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

pub(super) fn responses_instructions_text(value: Option<&Value>) -> Option<String> {
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

pub(super) fn append_responses_input_to_messages(input: Option<&Value>, messages: &mut Vec<Message>) {
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
pub(super) fn attach_pending_reasoning_to_previous_assistant(
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

pub(super) fn responses_item_boundary_role(item: &Value) -> Option<String> {
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

pub(super) fn append_responses_item_to_messages(item: &Value, messages: &mut Vec<Message>) {
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

pub(super) fn responses_text_item_role(item: &Value) -> String {
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

pub(super) fn responses_value_to_message_content(value: &Value) -> MessageContent {
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

pub(super) fn responses_reasoning_message(item: &Value) -> Message {
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

pub(super) fn merge_responses_following_item_into_reasoning_message(
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

pub(super) fn merge_message_into_reasoning_message(reasoning_message: &mut Message, message: Message) {
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

pub(super) fn message_content_into_parts(content: MessageContent) -> Vec<MessageContentPart> {
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

pub(super) fn responses_message_item_to_llm(item: &Value) -> Message {
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

pub(super) fn responses_content_part_to_llm(part: &Value) -> Option<MessageContentPart> {
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

pub(super) fn responses_compaction_message(item: &Value) -> Message {
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

pub(super) fn responses_compaction_part(item: &Value) -> MessageContentPart {
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

pub(super) fn responses_input_image_part(item: &Value) -> Option<MessageContentPart> {
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

pub(super) fn responses_tool_to_llm(tool: &Value) -> Option<Tool> {
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

pub(super) fn responses_call_to_tool_call(item: &Value, index: usize) -> ToolCall {
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

pub(super) fn responses_string_or_compact_json(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub(super) fn sanitize_responses_function_arguments(name: &str, arguments: &str) -> String {
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

pub(super) fn merge_raw_responses_fragments(
    mut structured: Vec<Value>,
    raw_fragments: Option<&Value>,
) -> Vec<Value> {
    merge_raw_responses_fragments_with_signatures(&mut structured, raw_fragments, None)
}

/// Merge raw Responses fragments back by original index.
/// When `expected_signatures` is present (from request-scoped tool signature sidecar),
/// a raw tool that collides with a structured tool signature is dropped (fail-closed
/// for that fragment) instead of silently overwriting structured identity (T-3).
pub(crate) fn merge_raw_responses_fragments_with_signatures(
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
pub(super) fn raw_tool_compatible_with_signatures(raw_tool: &Value, signatures: &[String]) -> bool {
    let Some(raw_sig) = responses_tool_signature(raw_tool) else {
        return true;
    };
    // If the raw tool's signature is already represented in structured tools, drop it.
    !signatures.iter().any(|expected| expected == &raw_sig)
}

pub(super) fn raw_responses_fragment_parts(value: &Value) -> Option<(usize, Value)> {
    let index = value.get("index").and_then(Value::as_u64)? as usize;
    let raw_value = value.get("value")?.clone();
    Some((index, raw_value))
}

pub(super) fn responses_metadata_or_extra_body(
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

pub(super) fn append_llm_message_as_responses_input(
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

pub(super) fn has_responses_message_content(content: &MessageContent, refusal: &str, assistant: bool) -> bool {
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

pub(super) fn append_responses_message_content_items(
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

pub(super) fn flush_responses_message_content(role: &str, content: &mut Vec<Value>, input: &mut Vec<Value>) {
    if content.is_empty() {
        return;
    }
    input.push(json!({
        "type": "message",
        "role": role,
        "content": std::mem::take(content)
    }));
}

pub(super) fn responses_reasoning_item_from_message(message: &Message) -> Option<Value> {
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

pub(super) fn llm_content_to_responses_content(
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

pub(super) fn llm_content_part_to_responses_content(
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

pub(super) fn is_responses_compaction_type(part_type: &str) -> bool {
    matches!(part_type, "compaction" | "compaction_summary")
}

pub(super) fn responses_compaction_item_from_part(part: MessageContentPart) -> Value {
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

pub(super) fn tool_call_to_responses_item(tool_call: ToolCall) -> Value {
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

pub(super) fn responses_function_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("fc") {
        call_id.to_string()
    } else if call_id.is_empty() {
        "fc_0".to_string()
    } else {
        format!("fc_{call_id}")
    }
}

pub(super) fn responses_custom_tool_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("ctc") {
        call_id.to_string()
    } else if call_id.is_empty() {
        "ctc_0".to_string()
    } else {
        format!("ctc_{call_id}")
    }
}

pub(super) fn metadata_from_value(value: Option<&Value>) -> HashMap<String, String> {
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

pub(super) fn responses_format_to_chat(format: Value) -> Value {
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

pub(super) fn response_format_to_responses_format(response_format: Value) -> Value {
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

pub(super) fn responses_function_parameters(parameters: Option<Value>, strict: Option<bool>) -> Value {
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

pub(super) fn text_content_item(
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

pub(super) fn content_annotations(content: &[Value]) -> Vec<Value> {
    content.iter().flat_map(part_annotations).collect()
}

pub(super) fn content_annotations_from_value(content: &Value) -> Vec<Value> {
    if let Some(parts) = content.as_array() {
        return content_annotations(parts);
    }
    part_annotations(content)
}

pub(super) fn part_annotations(part: &Value) -> Vec<Value> {
    part.get("annotations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn responses_reasoning_text(item: &Value) -> Option<String> {
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

pub(super) fn responses_usage_to_llm(usage: Option<&Value>) -> Usage {
    openai_usage_to_llm(usage)
}

pub(super) fn usage_to_responses(usage: Option<&Usage>) -> Value {
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

pub(super) fn responses_status_to_finish(status: Option<&str>, has_tool: bool) -> String {
    match status {
        Some("failed") => "error".to_string(),
        Some("incomplete") => "length".to_string(),
        Some("completed") if has_tool => "tool_calls".to_string(),
        Some("completed") => "stop".to_string(),
        _ => "stop".to_string(),
    }
}

pub(super) fn finish_to_responses_status(reason: Option<&str>) -> &'static str {
    match reason {
        Some("error") => "failed",
        Some("length") | Some("max_tokens") => "incomplete",
        _ => "completed",
    }
}

