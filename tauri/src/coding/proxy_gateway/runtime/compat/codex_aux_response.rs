//! Response shaping for Codex's structured auxiliary calls.
//!
//! Codex asks for title / description / summary / prediction through a
//! Responses `text.format` JSON schema. A Chat, Gemini or Anthropic target may
//! never see that schema, so a perfectly good model answer can arrive as plain
//! text. The gateway wraps that text back into the object Codex reads; an answer
//! that is already the expected JSON is left untouched.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::codex_aux_task::CodexAuxTask;

#[derive(Debug)]
pub(crate) struct AuxShapingOutcome {
    pub body: Vec<u8>,
    /// Whether the gateway rewrote the model's answer.
    pub shaped: bool,
}

/// Wrap the model's answer into the task's expected JSON object.
///
/// Returns the original bytes when the answer already has the expected shape,
/// so an upstream that implements the schema is not rewritten pointlessly.
pub(crate) fn shape_aux_response(
    task: &CodexAuxTask,
    request_body: &[u8],
    converted_body: &[u8],
    upstream_body: &[u8],
) -> Result<AuxShapingOutcome, String> {
    let converted: Value = serde_json::from_slice(converted_body)
        .map_err(|error| format!("Failed to parse auxiliary response: {error}"))?;
    let text = extract_response_text(&converted).unwrap_or_default();

    let shaped_text = match task {
        CodexAuxTask::Title => shape_title(&text, request_body),
        CodexAuxTask::Description => shape_description(&text, request_body),
        CodexAuxTask::Summary => shape_summary(&text, request_body),
        CodexAuxTask::Prediction => shape_prediction(&text),
        _ => {
            return Ok(AuxShapingOutcome {
                body: converted_body.to_vec(),
                shaped: false,
            });
        }
    };

    if shaped_text.is_empty() {
        return Err("Auxiliary task returned no output text".to_string());
    }
    if text.trim() == shaped_text {
        return Ok(AuxShapingOutcome {
            body: converted_body.to_vec(),
            shaped: false,
        });
    }

    let envelope = build_aux_envelope(&converted, upstream_body, &shaped_text);
    let body = serde_json::to_vec(&envelope)
        .map_err(|error| format!("Failed to serialize auxiliary response: {error}"))?;
    Ok(AuxShapingOutcome { body, shaped: true })
}

/// Serialize the shaped body as the Responses SSE lifecycle Codex consumes.
pub(crate) fn build_aux_sse(body: &[u8], upstream_body: &[u8]) -> Result<String, String> {
    let mut response: Value = serde_json::from_slice(body)
        .map_err(|error| format!("Failed to parse auxiliary response for SSE: {error}"))?;
    ensure_response_envelope(&mut response, upstream_body);
    let Some(item) = response
        .get("output")
        .and_then(Value::as_array)
        .and_then(|output| output.iter().find(|item| is_message_item(item)))
        .cloned()
    else {
        return Err("Auxiliary task returned no assistant message".to_string());
    };
    let text = item_text(&item).unwrap_or_default();
    if text.is_empty() {
        return Err("Auxiliary task returned an empty assistant message".to_string());
    }

    let mut opened = response.clone();
    opened["status"] = Value::String("in_progress".to_string());
    opened["output"] = Value::Array(Vec::new());
    let mut in_progress_item = item.clone();
    in_progress_item["status"] = Value::String("in_progress".to_string());
    let mut completed_item = item;
    completed_item["status"] = Value::String("completed".to_string());

    let events = vec![
        (
            "response.created",
            serde_json::json!({ "type": "response.created", "response": opened }),
        ),
        (
            "response.in_progress",
            serde_json::json!({ "type": "response.in_progress", "response": opened }),
        ),
        (
            "response.output_item.added",
            serde_json::json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": in_progress_item,
            }),
        ),
        (
            "response.output_text.delta",
            serde_json::json!({
                "type": "response.output_text.delta",
                "delta": text,
                "item_id": completed_item.get("id").cloned().unwrap_or(Value::Null),
                "output_index": 0,
                "content_index": 0,
            }),
        ),
        (
            "response.output_text.done",
            serde_json::json!({
                "type": "response.output_text.done",
                "text": text,
                "item_id": completed_item.get("id").cloned().unwrap_or(Value::Null),
                "output_index": 0,
                "content_index": 0,
            }),
        ),
        (
            "response.output_item.done",
            serde_json::json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": completed_item,
            }),
        ),
        (
            "response.completed",
            serde_json::json!({ "type": "response.completed", "response": response }),
        ),
    ];

    let mut stream = String::new();
    for (name, payload) in events {
        stream.push_str("event: ");
        stream.push_str(name);
        stream.push_str("\ndata: ");
        stream.push_str(&payload.to_string());
        stream.push_str("\n\n");
    }
    Ok(stream)
}

fn shape_title(text: &str, request_body: &[u8]) -> String {
    let shape = response_shape(request_body);
    let parsed = parse_json_object(text);
    let expected = parsed
        .as_ref()
        .and_then(|object| object.get("title"))
        .and_then(Value::as_str)
        .map(clean_title)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| clean_title(text));
    if expected.is_empty() {
        return String::new();
    }
    build_shaped_object("title", &expected, parsed.as_ref(), &shape)
}

fn shape_description(text: &str, request_body: &[u8]) -> String {
    let shape = response_shape(request_body);
    let parsed = parse_json_object(text);
    let expected = parsed
        .as_ref()
        .and_then(|object| object.get("description"))
        .and_then(Value::as_str)
        .map(clean_description)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| clean_description(text));
    if expected.is_empty() {
        return String::new();
    }
    build_shaped_object("description", &expected, parsed.as_ref(), &shape)
}

fn shape_summary(text: &str, request_body: &[u8]) -> String {
    let shape = response_shape(request_body);
    let parsed = parse_json_object(text);
    let expected = parsed
        .as_ref()
        .and_then(|object| object.get("summary"))
        .and_then(Value::as_str)
        .map(clean_summary)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| clean_summary(text));
    if expected.is_empty() {
        return String::new();
    }
    build_shaped_object("summary", &expected, parsed.as_ref(), &shape)
}

/// Prediction never accepts plain text: a model that talks instead of answering
/// would otherwise have its words inserted into the user's composer.
fn shape_prediction(text: &str) -> String {
    let parsed = parse_json_object(text);
    let suggestion = parsed
        .as_ref()
        .and_then(|object| object.get("suggestion"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match suggestion {
        Some(value) => serde_json::json!({ "suggestion": value }).to_string(),
        None => serde_json::json!({ "suggestion": null }).to_string(),
    }
}

fn build_shaped_object(
    expected_key: &str,
    expected_text: &str,
    parsed: Option<&Map<String, Value>>,
    shape: &AuxResponseShape,
) -> String {
    let mut output = Map::new();
    let mut keys = shape.required.clone();
    if !keys.iter().any(|key| key == expected_key) {
        keys.push(expected_key.to_string());
    }
    for key in keys {
        let value = if key == expected_key {
            expected_text.to_string()
        } else {
            parsed
                .and_then(|object| object.get(&key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| expected_text.to_string())
        };
        let clipped = clip_to_max_length(&value, shape.max_lengths.get(&key).copied());
        output.insert(key, Value::String(clipped));
    }
    Value::Object(output).to_string()
}

fn clip_to_max_length(value: &str, max_length: Option<usize>) -> String {
    match max_length {
        Some(max_length) => value.chars().take(max_length).collect(),
        None => value.to_string(),
    }
}

#[derive(Debug, Default)]
struct AuxResponseShape {
    required: Vec<String>,
    max_lengths: HashMap<String, usize>,
}

fn response_shape(request_body: &[u8]) -> AuxResponseShape {
    let Ok(request) = serde_json::from_slice::<Value>(request_body) else {
        return AuxResponseShape::default();
    };
    let Some(schema) = request.pointer("/text/format/schema") else {
        return AuxResponseShape::default();
    };
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let max_lengths = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter_map(|(key, property)| {
                    property
                        .get("maxLength")
                        .and_then(Value::as_u64)
                        .map(|length| (key.clone(), length as usize))
                })
                .collect()
        })
        .unwrap_or_default();
    AuxResponseShape {
        required,
        max_lengths,
    }
}

fn clean_title(text: &str) -> String {
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    strip_label(first_line, "title")
        .trim_matches(|character| character == '"' || character == '\'' || character == '`')
        .trim()
        .to_string()
}

fn clean_description(text: &str) -> String {
    let trimmed = text.trim();
    strip_label(trimmed, "description")
        .trim_matches(|character| character == '"' || character == '\'')
        .trim()
        .to_string()
}

fn clean_summary(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    strip_label(&joined, "summary")
        .trim_matches(|character| character == '"' || character == '\'')
        .trim()
        .to_string()
}

fn strip_label<'a>(text: &'a str, label: &str) -> &'a str {
    let trimmed = text.trim();
    let lowercase = trimmed.to_ascii_lowercase();
    let prefix = format!("{label}:");
    if lowercase.starts_with(&prefix) {
        trimmed[prefix.len()..].trim()
    } else {
        trimmed
    }
}

fn extract_response_text(response: &Value) -> Option<String> {
    if let Some(output) = response.get("output").and_then(Value::as_array) {
        for item in output {
            if let Some(text) = item_text(item) {
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
        }
    }
    response
        .get("output_text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

fn item_text(item: &Value) -> Option<String> {
    let content = item.get("content")?.as_array()?;
    let mut text = String::new();
    for part in content {
        if let Some(value) = part.get("text").and_then(Value::as_str) {
            text.push_str(value);
        }
    }
    (!text.is_empty()).then_some(text)
}

fn is_message_item(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("message")
}

fn parse_json_object(text: &str) -> Option<Map<String, Value>> {
    let trimmed = text.trim();
    let candidate = strip_code_fence(trimmed);
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(candidate) {
        return Some(object);
    }
    let start = candidate.find('{')?;
    let end = candidate.rfind('}')?;
    if end <= start {
        return None;
    }
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(&candidate[start..=end]) else {
        return None;
    };
    Some(object)
}

fn strip_code_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest
        .strip_prefix("json")
        .or_else(|| rest.strip_prefix("JSON"))
        .unwrap_or(rest);
    rest.trim_start()
        .strip_suffix("```")
        .map(str::trim)
        .unwrap_or(rest.trim())
}

fn build_aux_envelope(converted: &Value, upstream: &[u8], text: &str) -> Value {
    let upstream = serde_json::from_slice::<Value>(upstream).unwrap_or(Value::Null);
    let model = converted
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| upstream.get("model").and_then(Value::as_str))
        .unwrap_or("unknown");
    let id = converted
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(aux_response_id);
    let usage = converted
        .get("usage")
        .cloned()
        .unwrap_or_else(|| usage_from_upstream(&upstream));
    let message_id = format!("msg_{}", id.trim_start_matches("resp_"));
    serde_json::json!({
        "id": id,
        "object": "response",
        "created_at": converted
            .get("created_at")
            .cloned()
            .unwrap_or_else(|| Value::from(unix_timestamp())),
        "status": "completed",
        "model": model,
        "output": [{
            "id": message_id,
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": text,
                "annotations": []
            }]
        }],
        "usage": usage,
    })
}

fn ensure_response_envelope(response: &mut Value, upstream_body: &[u8]) {
    if response.get("output").and_then(Value::as_array).is_some() {
        return;
    }
    let text = response
        .get("output_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    *response = build_aux_envelope(response, upstream_body, &text);
}

fn usage_from_upstream(upstream: &Value) -> Value {
    let input_tokens = upstream
        .pointer("/usage/input_tokens")
        .or_else(|| upstream.pointer("/usage/prompt_tokens"))
        .or_else(|| upstream.pointer("/usageMetadata/promptTokenCount"))
        .and_then(Value::as_i64);
    let output_tokens = upstream
        .pointer("/usage/output_tokens")
        .or_else(|| upstream.pointer("/usage/completion_tokens"))
        .or_else(|| upstream.pointer("/usageMetadata/candidatesTokenCount"))
        .and_then(Value::as_i64);
    let total_tokens = upstream
        .pointer("/usage/total_tokens")
        .or_else(|| upstream.pointer("/usageMetadata/totalTokenCount"))
        .and_then(Value::as_i64);
    serde_json::json!({
        "input_tokens": input_tokens.unwrap_or(0),
        "output_tokens": output_tokens.unwrap_or(0),
        "total_tokens": total_tokens.unwrap_or_else(|| {
            input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0)
        }),
    })
}

fn aux_response_id() -> String {
    format!("resp_aux_{}", unix_timestamp_nanos())
}

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn unix_timestamp_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_schema(schema: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "text": { "format": { "type": "json_schema", "name": "x", "schema": schema } }
        }))
        .expect("serialize request")
    }

    fn responses_body(text: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "status": "completed",
            "model": "test-model",
            "output": [{
                "id": "msg_1",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": text }]
            }],
            "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
        }))
        .expect("serialize response")
    }

    fn shaped_text(outcome: &AuxShapingOutcome) -> String {
        let value: Value = serde_json::from_slice(&outcome.body).expect("shaped body");
        extract_response_text(&value).expect("message text")
    }

    #[test]
    fn title_plain_text_is_wrapped() {
        let outcome = shape_aux_response(
            &CodexAuxTask::Title,
            &request_with_schema(serde_json::json!({
                "type": "object",
                "properties": { "title": { "type": "string", "maxLength": 20 } },
                "required": ["title"]
            })),
            &responses_body("A short title\nwith a second line"),
            b"",
        )
        .expect("shape");
        assert!(outcome.shaped);
        assert_eq!(shaped_text(&outcome), r#"{"title":"A short title"}"#);
    }

    #[test]
    fn title_schema_required_fields_are_filled_and_clipped() {
        let outcome = shape_aux_response(
            &CodexAuxTask::Title,
            &request_with_schema(serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "maxLength": 5 },
                    "description": { "type": "string", "maxLength": 8 }
                },
                "required": ["title", "description"]
            })),
            &responses_body("abcdefghij"),
            b"",
        )
        .expect("shape");
        assert_eq!(
            shaped_text(&outcome),
            r#"{"title":"abcde","description":"abcdefgh"}"#
        );
    }

    #[test]
    fn valid_json_is_left_untouched() {
        let body = responses_body(r#"{"title":"already shaped"}"#);
        let outcome = shape_aux_response(
            &CodexAuxTask::Title,
            &request_with_schema(serde_json::json!({
                "type": "object",
                "properties": { "title": { "type": "string" } },
                "required": ["title"]
            })),
            &body,
            b"",
        )
        .expect("shape");
        assert!(!outcome.shaped);
        assert_eq!(outcome.body, body);
    }

    #[test]
    fn description_and_summary_wrap_plain_text() {
        let description = shape_aux_response(
            &CodexAuxTask::Description,
            b"",
            &responses_body("  a useful description  "),
            b"",
        )
        .expect("shape");
        assert_eq!(
            shaped_text(&description),
            r#"{"description":"a useful description"}"#
        );

        let summary = shape_aux_response(
            &CodexAuxTask::Summary,
            b"",
            &responses_body("line one\nline two"),
            b"",
        )
        .expect("shape");
        assert_eq!(shaped_text(&summary), r#"{"summary":"line one line two"}"#);
    }

    #[test]
    fn prediction_never_accepts_plain_text() {
        let outcome = shape_aux_response(
            &CodexAuxTask::Prediction,
            b"",
            &responses_body("I think you will ask about tests"),
            b"",
        )
        .expect("shape");
        assert_eq!(shaped_text(&outcome), r#"{"suggestion":null}"#);

        let json = shape_aux_response(
            &CodexAuxTask::Prediction,
            b"",
            &responses_body(r#"{"suggestion":"run the tests"}"#),
            b"",
        )
        .expect("shape");
        assert_eq!(shaped_text(&json), r#"{"suggestion":"run the tests"}"#);
    }

    #[test]
    fn fenced_json_is_unwrapped_and_non_shaped_tasks_are_untouched() {
        let title = shape_aux_response(
            &CodexAuxTask::Title,
            b"",
            &responses_body("```json\n{\"title\":\"fenced\"}\n```"),
            b"",
        )
        .expect("shape fenced title");
        assert_eq!(shaped_text(&title), r#"{"title":"fenced"}"#);

        let memory_body = responses_body("plain memory text");
        let memory = shape_aux_response(&CodexAuxTask::Memory, b"", &memory_body, b"")
            .expect("memory is not shaped");
        assert!(!memory.shaped);
        assert_eq!(memory.body, memory_body);
    }

    #[test]
    fn empty_title_fails() {
        let error = shape_aux_response(&CodexAuxTask::Title, b"", &responses_body("   "), b"")
            .expect_err("empty title must fail");
        assert!(error.contains("no output text"));
    }

    #[test]
    fn sse_contains_the_full_lifecycle_and_the_shaped_text() {
        let body = responses_body(r#"{"title":"hello"}"#);
        let stream = build_aux_sse(&body, b"").expect("sse");
        assert!(stream.contains("event: response.created"));
        assert!(stream.contains("event: response.output_text.delta"));
        assert!(stream.contains("event: response.output_item.done"));
        assert!(stream.contains("event: response.completed"));
        // The shaped JSON is itself a string payload, so it is JSON-escaped in
        // each SSE `data:` line.
        assert!(stream.contains(r#"{\"title\":\"hello\"}"#));
    }
}
