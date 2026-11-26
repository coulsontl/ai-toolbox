use super::super::llm::{AllowedTools, NamedToolChoice, Stop, ToolChoice, ToolFunction};
use serde_json::{json, Value};

pub fn content_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => other.as_str().unwrap_or_default().to_string(),
        None => String::new(),
    }
}

pub fn message_parts(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(parts)) => parts.clone(),
        Some(Value::String(text)) => vec![json!({ "type": "text", "text": text })],
        _ => Vec::new(),
    }
}

pub fn json_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
    }
}

pub fn tool_arguments_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    if let Ok(value) = serde_json::from_str(trimmed) {
        return value;
    }
    if let Ok(value) = json5::from_str::<Value>(trimmed) {
        return value;
    }
    if let Ok(repaired) = repair_common_json(trimmed) {
        if let Ok(value) = serde_json::from_str(&repaired) {
            return value;
        }
    }
    Value::String(raw.to_string())
}

pub fn extract_reasoning_field_text(value: &Value) -> Option<String> {
    for field in ["reasoning_content", "reasoning", "reasoning_text"] {
        if let Some(text) = value
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }

    if let Some(reasoning) = value.get("reasoning") {
        for field in ["content", "text", "summary"] {
            if let Some(text) = reasoning
                .get(field)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                return Some(text.to_string());
            }
        }
    }

    value
        .get("reasoning_details")
        .and_then(extract_reasoning_details_text)
}

fn extract_reasoning_details_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then(|| text.to_string()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(extract_reasoning_detail_part_text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (!text.is_empty()).then_some(text)
        }
        Value::Object(_) => extract_reasoning_detail_part_text(value),
        _ => None,
    }
}

fn extract_reasoning_detail_part_text(value: &Value) -> Option<String> {
    for field in ["text", "content", "summary"] {
        if let Some(text) = value
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }

    let parts = value.get("parts").and_then(Value::as_array)?;
    let text = parts
        .iter()
        .filter_map(extract_reasoning_detail_part_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then_some(text)
}

pub fn split_leading_think_block(raw: &str) -> Option<(String, String)> {
    let (rest, close_tag) = strip_leading_think_open_tag_with_close(raw)?;
    let lower = rest.to_ascii_lowercase();
    let close_index = lower.find(close_tag)?;
    let reasoning = rest[..close_index].trim().to_string();
    let answer = rest[close_index + close_tag.len()..]
        .trim_start_matches(['\r', '\n', ' ', '\t'])
        .to_string();
    (!reasoning.is_empty() || !answer.is_empty()).then_some((reasoning, answer))
}

pub fn strip_leading_think_open_tag(raw: &str) -> Option<&str> {
    strip_leading_think_open_tag_with_close(raw).map(|(rest, _)| rest)
}

fn strip_leading_think_open_tag_with_close(raw: &str) -> Option<(&str, &'static str)> {
    let trimmed = raw.trim_start_matches(['\r', '\n', ' ', '\t']);
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<thinking>") {
        return Some((&trimmed["<thinking>".len()..], "</thinking>"));
    }
    if lower.starts_with("<think>") {
        return Some((&trimmed["<think>".len()..], "</think>"));
    }
    None
}

fn repair_common_json(raw: &str) -> Result<String, serde_json::Error> {
    let without_comments = strip_json_like_comments(raw);
    let without_trailing_commas = strip_trailing_json_commas(&without_comments);
    let single_quoted = normalize_single_quoted_json_strings(&without_trailing_commas);
    let quoted_keys = quote_unquoted_json_keys(&without_trailing_commas);
    let quoted_keys_after_single = quote_unquoted_json_keys(&single_quoted);

    for candidate in [
        without_trailing_commas.as_str(),
        single_quoted.as_str(),
        quoted_keys.as_str(),
        quoted_keys_after_single.as_str(),
    ] {
        if serde_json::from_str::<Value>(candidate).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    serde_json::from_str::<Value>(&without_trailing_commas)?;
    Ok(without_trailing_commas)
}

fn strip_json_like_comments(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' || next == '\r' {
                    output.push(next);
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = '\0';
            for next in chars.by_ref() {
                if previous == '*' && next == '/' {
                    break;
                }
                previous = next;
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn strip_trailing_json_commas(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }
        if ch == ',' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(next) if next.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }
        output.push(ch);
    }
    output
}

fn normalize_single_quoted_json_strings(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_double_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_double_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_double_string = true;
            output.push(ch);
            continue;
        }

        if ch == '\'' {
            output.push('"');
            let mut single_escaped = false;
            for next in chars.by_ref() {
                if single_escaped {
                    if next == '"' {
                        output.push('\\');
                    }
                    output.push(next);
                    single_escaped = false;
                    continue;
                }
                match next {
                    '\\' => single_escaped = true,
                    '\'' => {
                        output.push('"');
                        break;
                    }
                    '"' => {
                        output.push('\\');
                        output.push('"');
                    }
                    other => output.push(other),
                }
            }
            continue;
        }

        output.push(ch);
    }

    output
}

fn quote_unquoted_json_keys(raw: &str) -> String {
    let chars = raw.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(raw.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut expecting_key = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            expecting_key = false;
            index += 1;
            continue;
        }

        if ch == '{' || ch == ',' {
            expecting_key = true;
            output.push(ch);
            index += 1;
            continue;
        }

        if expecting_key && ch.is_whitespace() {
            output.push(ch);
            index += 1;
            continue;
        }

        if expecting_key && is_bare_json_key_start(ch) {
            let start = index;
            index += 1;
            while index < chars.len() && is_bare_json_key_char(chars[index]) {
                index += 1;
            }
            let mut lookahead = index;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if lookahead < chars.len() && chars[lookahead] == ':' {
                output.push('"');
                for key_char in &chars[start..index] {
                    output.push(*key_char);
                }
                output.push('"');
                expecting_key = false;
                continue;
            }
            for key_char in &chars[start..index] {
                output.push(*key_char);
            }
            expecting_key = false;
            continue;
        }

        if !ch.is_whitespace() {
            expecting_key = false;
        }
        output.push(ch);
        index += 1;
    }

    output
}

fn is_bare_json_key_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_bare_json_key_char(ch: char) -> bool {
    matches!(ch, '_' | '-' | '.') || ch.is_ascii_alphanumeric()
}

pub fn stop_from_value(value: Option<&Value>) -> Option<Stop> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(Stop::String(text.clone())),
        Some(Value::Array(items)) => {
            let stops = items
                .iter()
                .filter_map(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!stops.is_empty()).then_some(Stop::Multiple(stops))
        }
        _ => None,
    }
}

pub fn stop_to_value(stop: Option<Stop>) -> Option<Value> {
    match stop {
        Some(Stop::String(text)) if !text.is_empty() => Some(json!(text)),
        Some(Stop::Multiple(items)) if !items.is_empty() => Some(json!(items)),
        _ => None,
    }
}

pub fn tool_choice_from_openai(value: Option<&Value>) -> Option<ToolChoice> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(ToolChoice::String(text.clone())),
        Some(Value::Object(object)) => {
            // An `allowed_tools` choice carries the caller's permitted tool
            // subset: Chat nests mode/tools under `allowed_tools` while
            // Responses keeps them at the top level (AxonHub d5237439).
            // Keeping only the mode would silently lift the restriction.
            if object.get("type").and_then(Value::as_str) == Some("allowed_tools") {
                let source = object
                    .get("allowed_tools")
                    .and_then(Value::as_object)
                    .unwrap_or(object);
                return Some(ToolChoice::AllowedTools(AllowedTools {
                    mode: source
                        .get("mode")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    tools: source
                        .get("tools")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                }));
            }
            if let Some(mode) = object.get("mode").and_then(Value::as_str) {
                return Some(ToolChoice::String(mode.to_string()));
            }
            // Require a `type` (e.g. "function", "image_generation") and treat
            // `name` as optional so type-only tool choices survive a
            // cross-protocol trip instead of being dropped by the inbound
            // parser. The real `type` is preserved rather than hardcoded to
            // "function".
            let choice_type = object.get("type").and_then(Value::as_str)?;
            let name = object
                .get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| object.get("name"))
                .and_then(Value::as_str);
            Some(ToolChoice::Named(NamedToolChoice {
                choice_type: choice_type.to_string(),
                function: ToolFunction {
                    name: name.unwrap_or_default().to_string(),
                },
            }))
        }
        _ => None,
    }
}

pub fn tool_choice_from_anthropic(value: Option<&Value>) -> Option<ToolChoice> {
    if let Some(Value::String(choice)) = value {
        return match choice.as_str() {
            "any" => Some(ToolChoice::String("required".to_string())),
            "auto" | "none" => Some(ToolChoice::String(choice.clone())),
            _ => None,
        };
    }

    let object = value.and_then(Value::as_object)?;
    match object.get("type").and_then(Value::as_str) {
        Some("tool") => object.get("name").and_then(Value::as_str).map(|name| {
            ToolChoice::Named(NamedToolChoice {
                choice_type: "function".to_string(),
                function: ToolFunction {
                    name: name.to_string(),
                },
            })
        }),
        Some("any") => Some(ToolChoice::String("required".to_string())),
        Some("auto") | Some("none") => object
            .get("type")
            .and_then(Value::as_str)
            .map(|choice| ToolChoice::String(choice.to_string())),
        _ => None,
    }
}

pub fn tool_choice_to_anthropic(choice: Option<ToolChoice>) -> Option<Value> {
    match choice {
        Some(ToolChoice::String(choice)) => Some(json!({
            "type": match choice.as_str() {
                "required" => "any",
                "any" => "any",
                "none" => "none",
                _ => "auto",
            }
        })),
        // Anthropic has no subset-aware tool choice; keep the caller's mode
        // and drop the permitted subset (AxonHub d5237439). A subset without a
        // mode carries no Anthropic instruction and is dropped.
        Some(ToolChoice::AllowedTools(allowed)) => allowed.mode.map(|mode| {
            json!({
                "type": match mode.as_str() {
                    "required" | "any" => "any",
                    "none" => "none",
                    _ => "auto",
                }
            })
        }),
        // Anthropic `tool` tool_choice requires a name; a type-only choice
        // (e.g. "image_generation") has no Anthropic equivalent and is dropped.
        Some(ToolChoice::Named(named)) if !named.function.name.is_empty() => Some(json!({
            "type": "tool",
            "name": named.function.name
        })),
        _ => None,
    }
}

pub fn tool_choice_to_openai(choice: Option<ToolChoice>) -> Option<Value> {
    match choice {
        Some(ToolChoice::String(choice)) if !choice.is_empty() => Some(json!(if choice == "any" {
            "required"
        } else {
            choice.as_str()
        })),
        Some(ToolChoice::AllowedTools(allowed)) => Some(allowed_tools_json(&allowed, true)),
        // OpenAI Chat tool_choice always needs `function.name`; a type-only
        // choice cannot be expressed in the Chat shape and is dropped.
        Some(ToolChoice::Named(named)) if !named.function.name.is_empty() => Some(json!({
            "type": "function",
            "function": {
                "name": named.function.name
            }
        })),
        _ => None,
    }
}

pub fn tool_choice_to_responses(choice: Option<ToolChoice>) -> Option<Value> {
    match choice {
        Some(ToolChoice::String(choice)) if !choice.is_empty() => Some(json!(if choice == "any" {
            "required"
        } else {
            choice.as_str()
        })),
        Some(ToolChoice::AllowedTools(allowed)) => Some(allowed_tools_json(&allowed, false)),
        // Preserve the real `type` and omit `name` when empty so type-only
        // Responses tool choices (e.g. "image_generation") round-trip cleanly.
        Some(ToolChoice::Named(named)) => {
            let mut tool_choice = serde_json::Map::new();
            tool_choice.insert("type".to_string(), json!(named.choice_type));
            if !named.function.name.is_empty() {
                tool_choice.insert("name".to_string(), json!(named.function.name));
            }
            Some(Value::Object(tool_choice))
        }
        _ => None,
    }
}

/// Rebuild an `allowed_tools` choice for its Chat or Responses wire shape.
///
/// Chat Completions nests `mode`/`tools` under `allowed_tools`; Responses
/// keeps them next to `type` (AxonHub d5237439).
fn allowed_tools_json(allowed: &AllowedTools, chat_shape: bool) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("type".to_string(), json!("allowed_tools"));
    let mut payload = serde_json::Map::new();
    if let Some(mode) = &allowed.mode {
        payload.insert("mode".to_string(), json!(mode));
    }
    if !allowed.tools.is_empty() {
        payload.insert(
            "tools".to_string(),
            Value::Array(
                allowed
                    .tools
                    .iter()
                    .map(|tool| allowed_tool_json(tool, chat_shape))
                    .collect(),
            ),
        );
    }
    if chat_shape {
        object.insert("allowed_tools".to_string(), Value::Object(payload));
    } else {
        object.extend(payload);
    }
    Value::Object(object)
}

/// Rewrite one `allowed_tools` selector between the Chat
/// `{type, function:{name}}` and Responses `{type, name}` shapes. Selectors
/// without a function name (for example MCP `server_label`) have no shape
/// difference and are forwarded exactly as they arrived.
fn allowed_tool_json(tool: &Value, chat_shape: bool) -> Value {
    let Some(object) = tool.as_object() else {
        return tool.clone();
    };
    let name = object
        .get("function")
        .and_then(|function| function.get("name"))
        .or_else(|| object.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty());
    let Some(name) = name else {
        return tool.clone();
    };
    let tool_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if chat_shape {
        json!({ "type": tool_type, "function": { "name": name } })
    } else {
        json!({ "type": tool_type, "name": name })
    }
}
pub fn tool_choice_from_gemini(value: Option<&Value>) -> Option<ToolChoice> {
    let config = value?;
    let mode = config.get("mode").and_then(Value::as_str);
    let allowed = config
        .get("allowedFunctionNames")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if mode == Some("ANY") {
        if allowed.len() == 1 {
            return Some(ToolChoice::Named(NamedToolChoice {
                choice_type: "function".to_string(),
                function: ToolFunction {
                    name: allowed[0].to_string(),
                },
            }));
        }
        if allowed.len() > 1 {
            return Some(ToolChoice::String("required".to_string()));
        }
    }
    match mode {
        Some("NONE") => Some(ToolChoice::String("none".to_string())),
        Some("ANY") => Some(ToolChoice::String("required".to_string())),
        Some("AUTO") => Some(ToolChoice::String("auto".to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_arguments_value_repairs_common_json() {
        assert_eq!(
            tool_arguments_value(r#"{"path":"a",}"#),
            json!({"path": "a"})
        );
        assert_eq!(
            tool_arguments_value(r#"{path:'README.md'}"#),
            json!({"path": "README.md"})
        );
        assert_eq!(
            tool_arguments_value(r#"{'path':'README.md',}"#),
            json!({"path": "README.md"})
        );
        assert_eq!(
            tool_arguments_value(
                r#"{
                    // keep path
                    "path": "a"
                }"#
            ),
            json!({"path": "a"})
        );
        assert_eq!(
            tool_arguments_value(
                r#"{
                    /* json5 block comment */
                    path: 'README.md',
                    flags: ['read', 'cache',],
                    nested: { dryRun: true },
                    count: +1,
                }"#
            ),
            json!({
                "path": "README.md",
                "flags": ["read", "cache"],
                "nested": {"dryRun": true},
                "count": 1
            })
        );
    }

    #[test]
    fn tool_arguments_value_preserves_unrepairable_arguments() {
        assert_eq!(
            tool_arguments_value(r#"{"path":"README.md""#),
            Value::String(r#"{"path":"README.md""#.to_string())
        );
    }

    #[test]
    fn tool_choice_from_openai_preserves_type_only_choice() {
        // type-only choice (e.g. image_generation) survives without a name
        let named = match tool_choice_from_openai(Some(&json!({"type": "image_generation"}))) {
            Some(ToolChoice::Named(named)) => named,
            other => panic!("expected Named, got {other:?}"),
        };
        assert_eq!(named.choice_type, "image_generation");
        assert_eq!(named.function.name, "");

        // type + name is preserved with the real type (not hardcoded "function")
        let named = match tool_choice_from_openai(Some(&json!({
            "type": "function", "name": "get_weather"
        }))) {
            Some(ToolChoice::Named(named)) => named,
            other => panic!("expected Named, got {other:?}"),
        };
        assert_eq!(named.choice_type, "function");
        assert_eq!(named.function.name, "get_weather");

        // Chat shape: function.name nested under "function"
        let named = match tool_choice_from_openai(Some(&json!({
            "type": "function", "function": {"name": "get_weather"}
        }))) {
            Some(ToolChoice::Named(named)) => named,
            other => panic!("expected Named, got {other:?}"),
        };
        assert_eq!(named.function.name, "get_weather");

        // no type at all -> None (cannot form a Named choice without a type)
        assert_eq!(tool_choice_from_openai(Some(&json!({"name": "x"}))), None);
    }

    #[test]
    fn tool_choice_to_responses_omits_empty_name_and_preserves_type() {
        // type-only -> {"type": "image_generation"} (no name key)
        assert_eq!(
            tool_choice_to_responses(Some(ToolChoice::Named(NamedToolChoice {
                choice_type: "image_generation".to_string(),
                function: ToolFunction::default(),
            }))),
            Some(json!({"type": "image_generation"}))
        );

        // type + name -> both keys, real type preserved
        assert_eq!(
            tool_choice_to_responses(Some(ToolChoice::Named(NamedToolChoice {
                choice_type: "function".to_string(),
                function: ToolFunction {
                    name: "get_weather".to_string(),
                },
            }))),
            Some(json!({"type": "function", "name": "get_weather"}))
        );
    }

    #[test]
    fn tool_choice_from_openai_preserves_allowed_tools_subsets() {
        // Chat Completions nests mode/tools under `allowed_tools`.
        let chat = json!({
            "type": "allowed_tools",
            "allowed_tools": {
                "mode": "required",
                "tools": [{"type": "function", "function": {"name": "tool_a"}}]
            }
        });
        let allowed = match tool_choice_from_openai(Some(&chat)) {
            Some(ToolChoice::AllowedTools(allowed)) => allowed,
            other => panic!("expected AllowedTools, got {other:?}"),
        };
        assert_eq!(allowed.mode.as_deref(), Some("required"));
        assert_eq!(
            allowed.tools,
            vec![json!({"type": "function", "function": {"name": "tool_a"}})]
        );

        // Responses keeps mode/tools at the top level next to `type`.
        let responses = json!({
            "type": "allowed_tools",
            "mode": "auto",
            "tools": [{"type": "mcp", "server_label": "local"}]
        });
        let allowed = match tool_choice_from_openai(Some(&responses)) {
            Some(ToolChoice::AllowedTools(allowed)) => allowed,
            other => panic!("expected AllowedTools, got {other:?}"),
        };
        assert_eq!(allowed.mode.as_deref(), Some("auto"));
        assert_eq!(
            allowed.tools,
            vec![json!({"type": "mcp", "server_label": "local"})]
        );

        // A bare mode without `type` keeps the existing string mapping.
        assert_eq!(
            tool_choice_from_openai(Some(&json!({"mode": "auto"}))),
            Some(ToolChoice::String("auto".to_string()))
        );
    }

    #[test]
    fn tool_choice_allowed_tools_rebuilds_each_wire_shape() {
        let choice = ToolChoice::AllowedTools(AllowedTools {
            mode: Some("required".to_string()),
            tools: vec![
                json!({"type": "function", "name": "tool_a"}),
                json!({"type": "mcp", "server_label": "local"}),
            ],
        });

        // Chat nests the subset and wraps function selectors under `function`.
        assert_eq!(
            tool_choice_to_openai(Some(choice.clone())),
            Some(json!({
                "type": "allowed_tools",
                "allowed_tools": {
                    "mode": "required",
                    "tools": [
                        {"type": "function", "function": {"name": "tool_a"}},
                        {"type": "mcp", "server_label": "local"}
                    ]
                }
            }))
        );

        // Responses keeps mode/tools flat and names the selector directly.
        assert_eq!(
            tool_choice_to_responses(Some(choice.clone())),
            Some(json!({
                "type": "allowed_tools",
                "mode": "required",
                "tools": [
                    {"type": "function", "name": "tool_a"},
                    {"type": "mcp", "server_label": "local"}
                ]
            }))
        );

        // Anthropic has no subset shape; only the caller's mode survives.
        assert_eq!(
            tool_choice_to_anthropic(Some(choice)),
            Some(json!({"type": "any"}))
        );
    }
    #[test]
    fn tool_choice_to_openai_and_anthropic_drop_type_only_choice() {
        let type_only = ToolChoice::Named(NamedToolChoice {
            choice_type: "image_generation".to_string(),
            function: ToolFunction::default(),
        });
        // Chat and Anthropic cannot express a type-only choice -> dropped
        assert_eq!(tool_choice_to_openai(Some(type_only.clone())), None);
        assert_eq!(tool_choice_to_anthropic(Some(type_only)), None);
    }
}
