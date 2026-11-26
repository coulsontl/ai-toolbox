//! Normalize function tool JSON Schema roots for strict OpenAI-compatible upstreams.
//!
//! Some Codex / Responses tools carry `parameters: null`, missing `type`, or
//! `type: null`. Strict providers (e.g. DeepSeek) require a root schema of
//! `{"type":"object","properties":{...}}`. Only the tool-root parameters /
//! input_schema object is touched — nested property schemas are not rewritten.
//!
//! Also hosts the shared Codex namespace tool naming helper used by Chat
//! conversion and native Responses passthrough flatten/restore.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// OpenAI Chat tool name length limit (also used for Responses namespace flatten).
pub const CHAT_TOOL_NAME_MAX_LEN: usize = 64;

/// Flatten a namespaced tool into a single Chat/Responses function name.
///
/// Short names use `{namespace}__{name}`; names longer than
/// [`CHAT_TOOL_NAME_MAX_LEN`] are truncated with a stable sha256 suffix so Chat
/// conversion and native Responses passthrough agree.
pub fn flatten_namespace_tool_name(namespace: &str, name: &str) -> String {
    let full_name = format!("{namespace}__{name}");
    if full_name.len() <= CHAT_TOOL_NAME_MAX_LEN {
        return full_name;
    }

    let hash = short_sha256_hex(full_name.as_bytes());
    let suffix = format!("__{hash}");
    let prefix_len = CHAT_TOOL_NAME_MAX_LEN.saturating_sub(suffix.len());
    let mut prefix = String::new();
    for ch in full_name.chars() {
        if prefix.len() + ch.len_utf8() > prefix_len {
            break;
        }
        prefix.push(ch);
    }
    format!("{prefix}{suffix}")
}

fn short_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Normalize a function tool's root `parameters` / `input_schema` value.
///
/// - Missing / null / non-object → `{"type":"object","properties":{}}`
/// - Object whose `type` is missing, null, or not `"object"` → force `type: "object"`
/// - Existing fields (including `oneOf` / `properties`) are preserved
/// - Idempotent when already `type: "object"`
pub fn normalize_function_parameters(parameters: Option<&Value>) -> Value {
    let mut params = match parameters {
        Some(Value::Object(object)) => Value::Object(object.clone()),
        _ => json!({"type": "object", "properties": {}}),
    };
    if let Some(object) = params.as_object_mut() {
        match object.get("type").and_then(Value::as_str) {
            Some("object") => {}
            _ => {
                object.insert("type".to_string(), json!("object"));
            }
        }
    }
    sanitize_schema_required_null_recursive(&mut params);
    params
}

/// Recursively remove invalid `required: null` (or non-array `required`) fields
/// emitted by some MCP/Codex schemas that strict OpenAI-compatible upstreams
/// reject with HTTP 400 (cc-switch d5dfa830). Preserves `additionalProperties`.
fn sanitize_schema_required_null_recursive(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(required) = object.get("required") {
                if !required.is_array() {
                    object.remove("required");
                }
            }
            for (key, child) in object {
                if matches!(
                    key.as_str(),
                    "properties" | "patternProperties" | "definitions" | "$defs"
                ) {
                    if let Some(map) = child.as_object_mut() {
                        for prop_schema in map.values_mut() {
                            sanitize_schema_required_null_recursive(prop_schema);
                        }
                    }
                } else if matches!(
                    key.as_str(),
                    "items" | "additionalProperties" | "anyOf" | "oneOf" | "allOf" | "not"
                ) {
                    sanitize_schema_required_null_recursive(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_schema_required_null_recursive(item);
            }
        }
        _ => {}
    }
}

/// Convenience for owned optional parameters (e.g. IR `Option<Value>`).
pub fn normalize_function_parameters_owned(parameters: Option<Value>) -> Value {
    match parameters {
        Some(value) => normalize_function_parameters(Some(&value)),
        None => normalize_function_parameters(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_or_missing_becomes_empty_object_schema() {
        assert_eq!(
            normalize_function_parameters(None),
            json!({"type": "object", "properties": {}})
        );
        assert_eq!(
            normalize_function_parameters(Some(&Value::Null)),
            json!({"type": "object", "properties": {}})
        );
    }

    #[test]
    fn non_object_becomes_empty_object_schema() {
        assert_eq!(
            normalize_function_parameters(Some(&json!(["x"]))),
            json!({"type": "object", "properties": {}})
        );
        assert_eq!(
            normalize_function_parameters(Some(&json!("string"))),
            json!({"type": "object", "properties": {}})
        );
    }

    #[test]
    fn forces_type_object_while_preserving_properties() {
        let input = json!({
            "type": null,
            "properties": {"city": {"type": "string"}}
        });
        let output = normalize_function_parameters(Some(&input));
        assert_eq!(output["type"], "object");
        assert_eq!(output["properties"]["city"]["type"], "string");
    }

    #[test]
    fn missing_type_forces_object_and_keeps_one_of() {
        let input = json!({
            "oneOf": [
                {"type": "object", "properties": {"a": {"type": "string"}}},
                {"type": "object", "properties": {"b": {"type": "number"}}}
            ]
        });
        let output = normalize_function_parameters(Some(&input));
        assert_eq!(output["type"], "object");
        assert_eq!(output["oneOf"].as_array().map(|items| items.len()), Some(2));
    }

    #[test]
    fn already_object_schema_is_idempotent() {
        let input = json!({
            "type": "object",
            "properties": {"q": {"type": "string"}},
            "required": ["q"]
        });
        assert_eq!(normalize_function_parameters(Some(&input)), input);
        assert_eq!(
            normalize_function_parameters(Some(&normalize_function_parameters(Some(&input)))),
            input
        );
    }

    #[test]
    fn owned_helper_matches_borrowed() {
        let input = json!({"type": null, "properties": {}});
        assert_eq!(
            normalize_function_parameters_owned(Some(input.clone())),
            normalize_function_parameters(Some(&input))
        );
        assert_eq!(
            normalize_function_parameters_owned(None),
            normalize_function_parameters(None)
        );
    }
}
