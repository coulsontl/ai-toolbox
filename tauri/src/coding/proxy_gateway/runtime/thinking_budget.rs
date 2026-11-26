use serde_json::Value;

const MAX_THINKING_BUDGET: u64 = 32_000;
const MAX_TOKENS_VALUE: u64 = 64_000;
const MIN_MAX_TOKENS_FOR_BUDGET: u64 = MAX_THINKING_BUDGET + 1;

pub(super) fn should_rectify_thinking_budget(status_code: u16, response_body: &[u8]) -> bool {
    if !(400..500).contains(&status_code) {
        return false;
    }
    let message = extract_error_text(response_body);
    let lower = message.to_ascii_lowercase();
    let has_budget_reference = lower.contains("budget_tokens") || lower.contains("budget tokens");
    let has_thinking_reference = lower.contains("thinking");
    let has_constraint = lower.contains("greater than or equal to 1024")
        || lower.contains(">= 1024")
        || (lower.contains("1024") && lower.contains("input should be"));
    has_budget_reference && has_thinking_reference && has_constraint
}

pub(super) fn rectify_thinking_budget(body: &[u8]) -> Option<Vec<u8>> {
    let mut value = serde_json::from_slice::<Value>(body).ok()?;
    if value
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        == Some("adaptive")
    {
        return None;
    }

    if !value.get("thinking").is_some_and(Value::is_object) {
        value["thinking"] = Value::Object(serde_json::Map::new());
    }
    let before_max_tokens = value.get("max_tokens").and_then(Value::as_u64);
    let thinking = value.get_mut("thinking")?.as_object_mut()?;
    let before_budget = thinking.get("budget_tokens").and_then(Value::as_u64);
    let before_type = thinking
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);

    thinking.insert("type".to_string(), Value::String("enabled".to_string()));
    thinking.insert(
        "budget_tokens".to_string(),
        Value::Number(MAX_THINKING_BUDGET.into()),
    );
    if before_max_tokens.is_none() || before_max_tokens < Some(MIN_MAX_TOKENS_FOR_BUDGET) {
        value["max_tokens"] = Value::Number(MAX_TOKENS_VALUE.into());
    }

    let changed = before_type.as_deref() != Some("enabled")
        || before_budget != Some(MAX_THINKING_BUDGET)
        || before_max_tokens.is_none()
        || before_max_tokens < Some(MIN_MAX_TOKENS_FOR_BUDGET);
    changed.then(|| serde_json::to_vec(&value).ok()).flatten()
}

/// Detect upstreams that refuse `thinking: {"type": "disabled"}` (e.g.
/// MiniMax-M3.1-Flash rejecting Claude Code auto-mode classifier requests,
/// cc-switch f5ab4dfa).
pub(super) fn should_rectify_thinking_disabled(status_code: u16, response_body: &[u8]) -> bool {
    if !(400..500).contains(&status_code) {
        return false;
    }
    let message = extract_error_text(response_body);
    let lower = message.to_ascii_lowercase();
    lower.contains("thinking")
        && lower.contains("disabled")
        && (lower.contains("not allowed") || lower.contains("requires adaptive thinking"))
}

/// Drop `thinking` when and only when `thinking.type == "disabled"`, letting
/// the upstream use its own default thinking policy.
pub(super) fn rectify_thinking_disabled(body: &[u8]) -> Option<Vec<u8>> {
    let mut value = serde_json::from_slice::<Value>(body).ok()?;
    let is_disabled = value
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        == Some("disabled");
    if !is_disabled {
        return None;
    }
    value.as_object_mut()?.remove("thinking");
    serde_json::to_vec(&value).ok()
}

fn extract_error_text(body: &[u8]) -> String {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return String::from_utf8_lossy(body).to_string();
    };
    [
        "/error/message",
        "/error",
        "/message",
        "/detail",
        "/details",
    ]
    .iter()
    .find_map(|path| value.pointer(path))
    .map(|value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    })
    .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_budget_1024_errors() {
        let body = br#"{"error":{"message":"thinking.budget_tokens: Input should be greater than or equal to 1024"}}"#;
        assert!(should_rectify_thinking_budget(400, body));
    }

    #[test]
    fn rectifies_budget_and_max_tokens() {
        let body = br#"{"thinking":{"type":"enabled","budget_tokens":512},"max_tokens":1024}"#;
        let next = rectify_thinking_budget(body).expect("rectified body");
        let value: Value = serde_json::from_slice(&next).unwrap();
        assert_eq!(value["thinking"]["budget_tokens"], MAX_THINKING_BUDGET);
        assert_eq!(value["max_tokens"], MAX_TOKENS_VALUE);
    }

    #[test]
    fn skips_adaptive_thinking() {
        let body = json!({"thinking":{"type":"adaptive","budget_tokens":512}})
            .to_string()
            .into_bytes();
        assert!(rectify_thinking_budget(&body).is_none());
    }

    #[test]
    fn detects_and_rectifies_rejected_disabled_thinking() {
        let err = br#"{"type":"error","error":{"type":"invalid_request_error","message":"invalid params, model \"MiniMax-M3.1-Flash\" requires adaptive thinking; thinking.type=\"disabled\" is not allowed (2013)"}}"#;
        assert!(should_rectify_thinking_disabled(400, err));
        assert!(!should_rectify_thinking_budget(400, err));

        let disabled_body = json!({
            "model": "MiniMax-M3.1-Flash",
            "max_tokens": 64,
            "thinking": {"type": "disabled"}
        })
        .to_string()
        .into_bytes();
        let rectified = rectify_thinking_disabled(&disabled_body).expect("should strip disabled");
        let parsed: Value = serde_json::from_slice(&rectified).unwrap();
        assert!(parsed.get("thinking").is_none());
        assert_eq!(parsed["max_tokens"], 64);

        let adaptive_body = json!({
            "model": "MiniMax-M3.1-Flash",
            "thinking": {"type": "adaptive"}
        })
        .to_string()
        .into_bytes();
        assert!(rectify_thinking_disabled(&adaptive_body).is_none());
    }
}
