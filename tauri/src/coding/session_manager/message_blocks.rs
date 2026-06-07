use std::collections::HashMap;

use serde_json::Value;

use super::tool_normalizer::{infer_tool_status, infer_tool_variant, normalize_tool_name};
use super::{SessionMessage, SessionMessageBlock, SessionMessageUsage};

pub(super) fn text_block(text: impl Into<String>) -> SessionMessageBlock {
    block_with_text("text", text)
}

pub(super) fn thinking_block(text: impl Into<String>) -> SessionMessageBlock {
    let mut block = block_with_text("thinking", text);
    block.variant = Some("thinking".to_string());
    block.title = Some("Thinking".to_string());
    block
}

pub(super) fn redacted_thinking_block(text: impl Into<String>) -> SessionMessageBlock {
    let mut block = block_with_text("redacted_thinking", text);
    block.variant = Some("thinking".to_string());
    block.title = Some("Redacted thinking".to_string());
    block
}

pub(super) fn unknown_block(
    text: impl Into<String>,
    metadata: Option<Value>,
) -> SessionMessageBlock {
    let mut block = block_with_text("unknown", text);
    block.variant = Some("neutral".to_string());
    block.metadata = metadata;
    block
}

pub(super) fn tool_call_block(
    tool_id: Option<String>,
    tool_name: impl Into<String>,
    input: Option<Value>,
) -> SessionMessageBlock {
    let tool_name = tool_name.into();
    let normalized_tool_name = normalize_tool_name(&tool_name);
    let mut block = empty_block("tool_call");
    block.variant = Some(infer_tool_variant(&normalized_tool_name, &tool_name));
    block.tool_id = tool_id;
    block.tool_name = Some(tool_name);
    block.normalized_tool_name = Some(normalized_tool_name);
    block.status = Some("pending".to_string());
    block.input = input;
    block
}

pub(super) fn tool_result_block(
    tool_id: Option<String>,
    tool_name: Option<String>,
    output: Option<Value>,
    is_error: Option<bool>,
) -> SessionMessageBlock {
    let normalized_tool_name = tool_name
        .as_deref()
        .map(normalize_tool_name)
        .unwrap_or_else(|| "unknown".to_string());
    let mut block = empty_block("tool_result");
    block.variant = Some(infer_tool_variant(
        &normalized_tool_name,
        tool_name.as_deref().unwrap_or(""),
    ));
    block.tool_id = tool_id;
    block.tool_name = tool_name;
    block.normalized_tool_name = Some(normalized_tool_name);
    block.output = output;
    block.is_error = is_error;
    block.status = Some(infer_tool_status(&block));
    block
}

pub(super) fn message_from_blocks(
    role: impl Into<String>,
    ts: Option<i64>,
    blocks: Vec<SessionMessageBlock>,
) -> SessionMessage {
    let blocks = pair_tool_blocks(blocks);
    let content = flatten_blocks_for_content(&blocks);
    SessionMessage {
        role: role.into(),
        content,
        ts,
        id: None,
        parent_id: None,
        message_type: None,
        blocks,
        model: None,
        usage: None,
        duration_ms: None,
        cost_usd: None,
        is_sidechain: None,
        metadata: None,
    }
}

pub(super) fn text_message(
    role: impl Into<String>,
    content: impl Into<String>,
    ts: Option<i64>,
) -> SessionMessage {
    message_from_blocks(role, ts, vec![text_block(content)])
}

pub(super) fn usage_from_value(value: &Value) -> Option<SessionMessageUsage> {
    let usage = SessionMessageUsage {
        input_tokens: number_field(
            value,
            &[
                "input_tokens",
                "inputTokens",
                "prompt_tokens",
                "promptTokens",
            ],
        ),
        output_tokens: number_field(
            value,
            &[
                "output_tokens",
                "outputTokens",
                "completion_tokens",
                "completionTokens",
            ],
        ),
        cache_creation_input_tokens: number_field(
            value,
            &["cache_creation_input_tokens", "cacheCreationInputTokens"],
        ),
        cache_read_input_tokens: number_field(
            value,
            &["cache_read_input_tokens", "cacheReadInputTokens"],
        ),
    };

    if usage.input_tokens.is_none()
        && usage.output_tokens.is_none()
        && usage.cache_creation_input_tokens.is_none()
        && usage.cache_read_input_tokens.is_none()
    {
        None
    } else {
        Some(usage)
    }
}

pub(super) fn flatten_blocks_for_content(blocks: &[SessionMessageBlock]) -> String {
    blocks
        .iter()
        .filter_map(block_to_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn pair_tool_blocks(blocks: Vec<SessionMessageBlock>) -> Vec<SessionMessageBlock> {
    let mut result = Vec::new();
    let mut pending_tool_call_indices: HashMap<String, usize> = HashMap::new();

    for block in blocks {
        if block.kind == "tool_call" {
            if let Some(tool_id) = block.tool_id.clone() {
                pending_tool_call_indices.insert(tool_id, result.len());
            }
            result.push(block);
            continue;
        }

        if block.kind == "tool_result" {
            if let Some(tool_id) = block.tool_id.as_deref() {
                if let Some(call_index) = pending_tool_call_indices.remove(tool_id) {
                    if let Some(call_block) = result.get_mut(call_index) {
                        call_block.kind = "tool_execution".to_string();
                        call_block.output = block.output.clone();
                        call_block.is_error = block.is_error;
                        call_block.status = block.status.clone();
                        call_block.status = Some(infer_tool_status(call_block));
                        if call_block.tool_name.is_none() {
                            call_block.tool_name = block.tool_name.clone();
                        }
                        if call_block.normalized_tool_name.is_none() {
                            call_block.normalized_tool_name = block.normalized_tool_name.clone();
                        }
                        continue;
                    }
                }
            }
        }

        result.push(block);
    }

    result
}

fn block_with_text(kind: &str, text: impl Into<String>) -> SessionMessageBlock {
    let mut block = empty_block(kind);
    block.text = Some(text.into());
    block
}

fn empty_block(kind: &str) -> SessionMessageBlock {
    SessionMessageBlock {
        kind: kind.to_string(),
        text: None,
        title: None,
        variant: None,
        language: None,
        tool_id: None,
        tool_name: None,
        normalized_tool_name: None,
        status: None,
        is_error: None,
        input: None,
        output: None,
        metadata: None,
    }
}

fn block_to_text(block: &SessionMessageBlock) -> Option<String> {
    match block.kind.as_str() {
        "tool_call" | "tool_execution" => {
            let name = block
                .tool_name
                .as_deref()
                .or(block.normalized_tool_name.as_deref())
                .unwrap_or("unknown");
            Some(format!("[Tool: {name}]"))
        }
        "tool_result" => block.output.as_ref().and_then(value_to_text),
        _ => block
            .text
            .clone()
            .or_else(|| block.output.as_ref().and_then(value_to_text)),
    }
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.to_string()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(value_to_text)
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if text.trim().is_empty() {
                Some(serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
            } else {
                Some(text)
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                return Some(text.to_string());
            }
            if let Some(content) = map.get("content") {
                if let Some(text) = value_to_text(content) {
                    return Some(text);
                }
            }
            Some(serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
        }
        _ => Some(value.to_string()),
    }
}

fn number_field(value: &Value, names: &[&str]) -> Option<i64> {
    names.iter().find_map(|name| {
        let value = value.get(*name)?;
        value
            .as_i64()
            .or_else(|| value.as_u64().map(|number| number as i64))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn pair_tool_blocks_merges_matching_call_and_result() {
        let blocks = pair_tool_blocks(vec![
            tool_call_block(
                Some("tool-1".to_string()),
                "Bash",
                Some(json!({ "command": "echo hi" })),
            ),
            tool_result_block(
                Some("tool-1".to_string()),
                Some("Bash".to_string()),
                Some(json!({ "stdout": "hi" })),
                None,
            ),
        ]);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "tool_execution");
        assert_eq!(blocks[0].normalized_tool_name.as_deref(), Some("bash"));
        assert_eq!(blocks[0].status.as_deref(), Some("success"));
    }

    #[test]
    fn pair_tool_blocks_does_not_merge_without_id() {
        let blocks = pair_tool_blocks(vec![
            tool_call_block(None, "Read", Some(json!({ "file_path": "Cargo.toml" }))),
            tool_result_block(None, Some("Read".to_string()), Some(json!("ok")), None),
        ]);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "tool_call");
        assert_eq!(blocks[1].kind, "tool_result");
    }

    #[test]
    fn flatten_blocks_keeps_text_and_tool_preview() {
        let message = message_from_blocks(
            "assistant",
            None,
            vec![
                text_block("hello"),
                tool_call_block(
                    Some("tool-1".to_string()),
                    "Grep",
                    Some(json!({ "pattern": "x" })),
                ),
            ],
        );

        assert!(message.content.contains("hello"));
        assert!(message.content.contains("[Tool: Grep]"));
    }
}
