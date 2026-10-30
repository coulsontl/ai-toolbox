use chrono::Local;
use serde_json::{json, Value};

use super::types::{
    CodebuddyPromptConfig, CodebuddyPromptConfigContent, CodebuddySettingsConfig,
};

pub fn settings_from_db_value(value: Value) -> CodebuddySettingsConfig {
    CodebuddySettingsConfig {
        config_dir: value
            .get("config_dir")
            .and_then(Value::as_str)
            .map(str::to_string),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| Local::now().to_rfc3339()),
    }
}

pub fn settings_to_db_value(config_dir: Option<&str>) -> Value {
    let now = Local::now().to_rfc3339();
    let mut value = json!({ "updated_at": now });
    if let Some(config_dir) = config_dir.filter(|dir| !dir.trim().is_empty()) {
        value["config_dir"] = json!(config_dir);
    }
    value
}

pub fn prompt_from_db_value(value: Value) -> CodebuddyPromptConfig {
    CodebuddyPromptConfig {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        content: value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        is_applied: value
            .get("is_applied")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        sort_index: value
            .get("sort_index")
            .and_then(Value::as_i64)
            .map(|index| index as i32),
        created_at: value
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

pub fn prompt_to_db_value(content: &CodebuddyPromptConfigContent) -> Value {
    json!({
        "name": content.name,
        "content": content.content,
        "is_applied": content.is_applied,
        "sort_index": content.sort_index,
        "created_at": content.created_at,
        "updated_at": content.updated_at,
    })
}
