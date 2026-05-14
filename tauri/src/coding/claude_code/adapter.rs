use super::types::{
    ClaudeCodeProvider, ClaudeCodeProviderContent, ClaudeCommonConfig, ClaudePromptConfig,
    ClaudePromptConfigContent,
};
use crate::coding::db_id::db_extract_id;
use chrono::Local;
use serde_json::{json, Value};

// ============================================================================
// Provider Adapter Functions
// ============================================================================

/// Helper function to get string value with backward compatibility (camelCase and snake_case)
fn get_str_compat(value: &Value, snake_key: &str, camel_key: &str, default: &str) -> String {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

/// Helper function to get optional string with backward compatibility
fn get_opt_str_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<String> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Helper function to get i64 with backward compatibility
fn get_i64_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<i32> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
}

/// Helper function to get bool with backward compatibility
fn get_bool_compat(value: &Value, snake_key: &str, camel_key: &str, default: bool) -> bool {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// Convert database Value to ClaudeCodeProvider with fault tolerance
/// Supports both snake_case (new) and camelCase (legacy) field names
pub fn from_db_value_provider(value: Value) -> ClaudeCodeProvider {
    // Use common utility to extract and clean the record ID
    let id = db_extract_id(&value);

    ClaudeCodeProvider {
        id,
        name: get_str_compat(&value, "name", "name", "Unnamed Provider"),
        category: get_str_compat(&value, "category", "category", "other"),
        settings_config: get_str_compat(&value, "settings_config", "settingsConfig", "{}"),
        extra_settings_config: get_str_compat(
            &value,
            "extra_settings_config",
            "extraSettingsConfig",
            "{}",
        ),
        source_provider_id: get_opt_str_compat(&value, "source_provider_id", "sourceProviderId"),
        website_url: get_opt_str_compat(&value, "website_url", "websiteUrl"),
        notes: get_opt_str_compat(&value, "notes", "notes"),
        icon: get_opt_str_compat(&value, "icon", "icon"),
        icon_color: get_opt_str_compat(&value, "icon_color", "iconColor"),
        sort_index: get_i64_compat(&value, "sort_index", "sortIndex"),
        is_applied: get_bool_compat(&value, "is_applied", "isApplied", false),
        is_disabled: get_bool_compat(&value, "is_disabled", "isDisabled", false),
        created_at: get_str_compat(&value, "created_at", "createdAt", ""),
        updated_at: get_str_compat(&value, "updated_at", "updatedAt", ""),
    }
}

/// Convert ClaudeCodeProviderContent to database Value
pub fn to_db_value_provider(content: &ClaudeCodeProviderContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|e| {
        eprintln!("Failed to serialize provider content: {}", e);
        json!({})
    })
}

// ============================================================================
// Common Config Adapter Functions
// ============================================================================

/// Convert database Value to ClaudeCommonConfig with fault tolerance
/// Supports both snake_case (new) and camelCase (legacy) field names
pub fn from_db_value_common(value: Value) -> ClaudeCommonConfig {
    ClaudeCommonConfig {
        config: value
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        root_dir: value
            .get("root_dir")
            .or_else(|| value.get("rootDir"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
        updated_at: value
            .get("updated_at")
            .or_else(|| value.get("updatedAt"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                let now = Local::now().to_rfc3339();
                Box::leak(now.into_boxed_str())
            })
            .to_string(),
    }
}

/// Convert common config to database Value
pub fn to_db_value_common(config: &str, root_dir: Option<&str>) -> Value {
    let now = Local::now().to_rfc3339();
    let mut value = json!({
        "config": config,
        "updated_at": now
    });

    if let Some(root_dir) = root_dir.filter(|dir| !dir.trim().is_empty()) {
        value["root_dir"] = json!(root_dir);
    }

    value
}

// ============================================================================
// Prompt Adapter Functions
// ============================================================================

pub fn from_db_value_prompt(value: Value) -> ClaudePromptConfig {
    ClaudePromptConfig {
        id: db_extract_id(&value),
        name: get_str_compat(&value, "name", "name", "Unnamed Prompt"),
        content: get_str_compat(&value, "content", "content", ""),
        is_applied: get_bool_compat(&value, "is_applied", "isApplied", false),
        sort_index: get_i64_compat(&value, "sort_index", "sortIndex"),
        created_at: get_opt_str_compat(&value, "created_at", "createdAt"),
        updated_at: get_opt_str_compat(&value, "updated_at", "updatedAt"),
    }
}

pub fn to_db_value_prompt(content: &ClaudePromptConfigContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|e| {
        eprintln!("Failed to serialize Claude prompt content: {}", e);
        json!({})
    })
}
