use super::super::llm::{NamedToolChoice, Stop, ToolChoice, ToolFunction};
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
    serde_json::from_str(raw).unwrap_or_else(|_| json!({}))
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
            if let Some(mode) = object.get("mode").and_then(Value::as_str) {
                return Some(ToolChoice::String(mode.to_string()));
            }
            let name = object
                .get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)?;
            Some(ToolChoice::Named(NamedToolChoice {
                choice_type: "function".to_string(),
                function: ToolFunction {
                    name: name.to_string(),
                },
            }))
        }
        _ => None,
    }
}

pub fn tool_choice_from_anthropic(value: Option<&Value>) -> Option<ToolChoice> {
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
        Some(ToolChoice::Named(named)) => Some(json!({
            "type": "tool",
            "name": named.function.name
        })),
        None => None,
    }
}

pub fn tool_choice_to_openai(choice: Option<ToolChoice>) -> Option<Value> {
    match choice {
        Some(ToolChoice::String(choice)) if !choice.is_empty() => Some(json!(if choice == "any" {
            "required"
        } else {
            choice.as_str()
        })),
        Some(ToolChoice::Named(named)) => Some(json!({
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
        Some(ToolChoice::Named(named)) => Some(json!({
            "type": "function",
            "name": named.function.name
        })),
        _ => None,
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
