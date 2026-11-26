use crate::coding::proxy_gateway::transformer::llm::{
    Choice, Message, MessageContent, MessageContentPart, Response, ResponseError,
};
use crate::coding::proxy_gateway::transformer::shared::signature::{
    encode_signature, SignatureProvider,
};
use serde_json::{json, Value};

use super::shared::*;

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
                    if let Some(reasoning) = responses_reasoning_text(item) {
                        append_reasoning_text(&mut message.reasoning_content, &reasoning);
                        message.reasoning = message.reasoning_content.clone();
                    }
                    if let Some(signature) = item
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .filter(|signature| !signature.is_empty())
                        .map(|signature| {
                            encode_signature(SignatureProvider::OpenAiResponses, signature)
                        })
                    {
                        message.reasoning_signature = Some(signature);
                    }
                }
                _ => {}
            }
        }
    }
    message.content = MessageContent::Parts(parts);
    message.tool_calls = tool_calls;
    let has_tool = !message.tool_calls.is_empty();
    let status = body.get("status").and_then(Value::as_str);
    let incomplete_reason = body
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str);
    let finish_reason = responses_status_to_finish(status, incomplete_reason, has_tool);
    let error = if status == Some("failed") || finish_reason == "error" {
        Some(responses_error_to_llm(body.get("error")))
    } else {
        None
    };
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
            finish_reason: Some(finish_reason),
            ..Default::default()
        }],
        usage: Some(responses_usage_to_llm(body.get("usage"))),
        error,
        ..Default::default()
    }
}

fn responses_error_to_llm(error: Option<&Value>) -> ResponseError {
    let error = error.unwrap_or(&Value::Null);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| error.as_str().map(ToString::to_string))
        .unwrap_or_else(|| "Response failed".to_string());
    let error_type = error
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("api_error")
        .to_string();
    let code = error.get("code").and_then(|code| match code {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    });
    ResponseError {
        message,
        error_type,
        code,
    }
}

fn append_reasoning_text(target: &mut Option<String>, text: &str) {
    match target {
        Some(existing) if !existing.is_empty() => {
            existing.push('\n');
            existing.push_str(text);
        }
        _ => *target = Some(text.to_string()),
    }
}

pub fn responses_compact_response_to_llm(body: Value) -> Response {
    let mut response = responses_response_to_llm(body);
    response.object = "response.compaction".to_string();
    response
}

pub fn llm_response_to_responses(response: Response) -> Value {
    let previous_response_id = response.previous_response_id.clone();
    let response_error = response.error.clone();
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
    // Strict Responses upstreams require replayed output message IDs to carry
    // the `msg_` prefix (cc-switch 82c29d09).
    let base_msg_id = if response.id.is_empty() {
        "gateway".to_string()
    } else {
        response.id.clone()
    };
    for (idx, item) in output.iter_mut().enumerate() {
        if item.get("type").and_then(Value::as_str) == Some("message")
            && item.get("id").and_then(Value::as_str).is_none_or(|id| !id.starts_with("msg_"))
        {
            item["id"] = json!(format!("msg_{base_msg_id}_{idx}"));
        }
    }
    for tool_call in choice.message.tool_calls {
        output.push(tool_call_to_responses_item(tool_call));
    }
    let status = if response_error.is_some() {
        "failed"
    } else {
        finish_to_responses_status(choice.finish_reason.as_deref())
    };
    let mut body = json!({
        "id": response.id,
        "object": "response",
        "created_at": response.created,
        "status": status,
        "model": response.model,
        "output": output,
        "usage": usage_to_responses(response.usage.as_ref())
    });
    if response_error.is_none() {
        if let Some(details) =
            finish_to_responses_incomplete_details(choice.finish_reason.as_deref())
        {
            body["incomplete_details"] = details;
        }
    }
    if let Some(previous_response_id) = previous_response_id {
        body["previous_response_id"] = json!(previous_response_id);
    }
    if let Some(error) = response_error {
        let mut error_obj = json!({
            "message": error.message
        });
        if !error.error_type.is_empty() {
            error_obj["type"] = json!(error.error_type);
        }
        if let Some(code) = error.code {
            error_obj["code"] = json!(code);
        }
        body["error"] = error_obj;
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
