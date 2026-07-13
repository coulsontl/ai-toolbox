use chrono::Local;
use serde_json::{json, Map, Value};

use super::types::{
    GrokCommonConfig, GrokPromptConfig, GrokPromptConfigContent, GrokProvider, GrokProviderContent,
};
use crate::coding::db_id::db_extract_id;

pub fn provider_from_db_value(value: Value) -> GrokProvider {
    GrokProvider {
        id: db_extract_id(&value),
        name: string_field(&value, "name"),
        category: value
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("custom")
            .to_string(),
        settings_config: value
            .get("settings_config")
            .and_then(Value::as_str)
            .unwrap_or("{}")
            .to_string(),
        source_provider_id: optional_string(&value, "source_provider_id"),
        website_url: optional_string(&value, "website_url"),
        notes: optional_string(&value, "notes"),
        icon: optional_string(&value, "icon"),
        icon_color: optional_string(&value, "icon_color"),
        sort_index: value
            .get("sort_index")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        meta: value.get("meta").cloned(),
        is_applied: value
            .get("is_applied")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_disabled: value
            .get("is_disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        created_at: string_field(&value, "created_at"),
        updated_at: string_field(&value, "updated_at"),
    }
}

pub fn provider_to_db_value(content: &GrokProviderContent) -> Value {
    let mut value = serde_json::to_value(content).unwrap_or_else(|_| json!({}));
    if let Value::Object(map) = &mut value {
        map.retain(|_, value| !value.is_null());
    }
    value
}

pub fn common_from_db_value(value: Value) -> GrokCommonConfig {
    GrokCommonConfig {
        config: value
            .get("config")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        root_dir: optional_string(&value, "root_dir"),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| Local::now().to_rfc3339()),
    }
}

pub fn common_to_db_value(config: &str, root_dir: Option<&str>) -> Value {
    let mut map = Map::new();
    map.insert("config".to_string(), Value::String(config.to_string()));
    if let Some(root_dir) = root_dir.filter(|value| !value.trim().is_empty()) {
        map.insert("root_dir".to_string(), Value::String(root_dir.to_string()));
    }
    map.insert(
        "updated_at".to_string(),
        Value::String(Local::now().to_rfc3339()),
    );
    Value::Object(map)
}

pub fn prompt_from_db_value(value: Value) -> GrokPromptConfig {
    GrokPromptConfig {
        id: db_extract_id(&value),
        name: string_field(&value, "name"),
        content: string_field(&value, "content"),
        is_applied: value
            .get("is_applied")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        sort_index: value
            .get("sort_index")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        created_at: optional_string(&value, "created_at"),
        updated_at: optional_string(&value, "updated_at"),
    }
}

pub fn prompt_to_db_value(content: &GrokPromptConfigContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|_| json!({}))
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}
