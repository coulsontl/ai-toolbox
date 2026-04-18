use base64::Engine;
use chrono::Local;
use serde_json::Value;

use super::commands::infer_codex_provider_category_from_settings;
use super::types::{
    CodexCommonConfig, CodexOfficialAccount, CodexOfficialAccountContent, CodexPromptConfig,
    CodexPromptConfigContent, CodexProvider, CodexProviderContent,
};
use crate::coding::db_id::db_extract_id;

fn decode_jwt_expiration(value: &str) -> Option<i64> {
    let payload = value.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .or_else(|| {
            let padded = format!("{}{}", payload, "=".repeat((4 - payload.len() % 4) % 4));
            base64::engine::general_purpose::URL_SAFE
                .decode(padded)
                .ok()
        })?;
    serde_json::from_slice::<Value>(&decoded)
        .ok()?
        .get("exp")
        .and_then(Value::as_i64)
}

fn token_expires_at_from_snapshot(snapshot: &str) -> Option<i64> {
    let parsed = serde_json::from_str::<Value>(snapshot).ok()?;
    parsed
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .and_then(decode_jwt_expiration)
        .or_else(|| {
            parsed
                .pointer("/tokens/id_token")
                .and_then(Value::as_str)
                .and_then(decode_jwt_expiration)
        })
}

fn mask_token_preview(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let char_count = trimmed.chars().count();
    if char_count <= 12 {
        return Some(trimmed.to_string());
    }
    let head: String = trimmed.chars().take(6).collect();
    let tail: String = trimmed
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Some(format!("{head}...{tail}"))
}

fn token_preview_from_snapshot(snapshot: &str, pointer: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(snapshot).ok()?;
    parsed
        .pointer(pointer)
        .and_then(Value::as_str)
        .and_then(mask_token_preview)
}

// ============================================================================
// Provider Adapter Functions
// ============================================================================

/// Convert database value to CodexProvider
pub fn from_db_value_provider(value: Value) -> CodexProvider {
    // Use common utility to extract and clean the record ID
    let id = db_extract_id(&value);
    let settings_config = value
        .get("settings_config")
        .and_then(|v| v.as_str())
        .unwrap_or("{}")
        .to_string();
    let inferred_category = serde_json::from_str::<Value>(&settings_config)
        .map(|parsed| infer_codex_provider_category_from_settings(&parsed))
        .unwrap_or_else(|_| "custom".to_string());
    let stored_category = value
        .get("category")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let category = match stored_category.as_deref() {
        Some("official") => "official".to_string(),
        Some("custom") if inferred_category == "official" => inferred_category,
        Some("custom") => "custom".to_string(),
        Some(other) => other.to_string(),
        None => inferred_category,
    };

    CodexProvider {
        id,
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        category,
        settings_config,
        source_provider_id: value
            .get("source_provider_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        website_url: value
            .get("website_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        notes: value
            .get("notes")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        icon: value
            .get("icon")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        icon_color: value
            .get("icon_color")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        sort_index: value
            .get("sort_index")
            .and_then(|v| v.as_i64())
            .map(|n| n as i32),
        is_applied: value
            .get("is_applied")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        is_disabled: value
            .get("is_disabled")
            .or_else(|| value.get("isDisabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        updated_at: value
            .get("updated_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// Convert CodexProviderContent to database value
pub fn to_db_value_provider(content: &CodexProviderContent) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("name".to_string(), Value::String(content.name.clone()));
    map.insert(
        "category".to_string(),
        Value::String(content.category.clone()),
    );
    map.insert(
        "settings_config".to_string(),
        Value::String(content.settings_config.clone()),
    );

    if let Some(ref source_id) = content.source_provider_id {
        map.insert(
            "source_provider_id".to_string(),
            Value::String(source_id.clone()),
        );
    }
    if let Some(ref url) = content.website_url {
        map.insert("website_url".to_string(), Value::String(url.clone()));
    }
    if let Some(ref notes) = content.notes {
        map.insert("notes".to_string(), Value::String(notes.clone()));
    }
    if let Some(ref icon) = content.icon {
        map.insert("icon".to_string(), Value::String(icon.clone()));
    }
    if let Some(ref color) = content.icon_color {
        map.insert("icon_color".to_string(), Value::String(color.clone()));
    }
    if let Some(index) = content.sort_index {
        map.insert("sort_index".to_string(), Value::Number(index.into()));
    }

    map.insert("is_applied".to_string(), Value::Bool(content.is_applied));
    map.insert("is_disabled".to_string(), Value::Bool(content.is_disabled));
    map.insert(
        "created_at".to_string(),
        Value::String(content.created_at.clone()),
    );
    map.insert(
        "updated_at".to_string(),
        Value::String(content.updated_at.clone()),
    );

    Value::Object(map)
}

// ============================================================================
// Official Account Adapter Functions
// ============================================================================

pub fn from_db_value_official_account(value: Value) -> CodexOfficialAccount {
    let id = db_extract_id(&value);
    let auth_snapshot = value
        .get("auth_snapshot")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    CodexOfficialAccount {
        id,
        provider_id: value
            .get("provider_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        kind: value
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("oauth")
            .to_string(),
        email: value
            .get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        auth_snapshot: Some(auth_snapshot.clone()),
        auth_mode: value
            .get("auth_mode")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        account_id: value
            .get("account_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        plan_type: value
            .get("plan_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        last_refresh: value
            .get("last_refresh")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        token_expires_at: token_expires_at_from_snapshot(&auth_snapshot),
        access_token_preview: token_preview_from_snapshot(&auth_snapshot, "/tokens/access_token"),
        refresh_token_preview: token_preview_from_snapshot(&auth_snapshot, "/tokens/refresh_token"),
        limit_short_label: value
            .get("limit_short_label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        limit_5h_text: value
            .get("limit_5h_text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        limit_weekly_text: value
            .get("limit_weekly_text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        limit_5h_reset_at: value.get("limit_5h_reset_at").and_then(|v| v.as_i64()),
        limit_weekly_reset_at: value.get("limit_weekly_reset_at").and_then(|v| v.as_i64()),
        last_limits_fetched_at: value
            .get("last_limits_fetched_at")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        last_error: value
            .get("last_error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        sort_index: value
            .get("sort_index")
            .and_then(|v| v.as_i64())
            .map(|n| n as i32),
        is_applied: value
            .get("is_applied")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        is_virtual: false,
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        updated_at: value
            .get("updated_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

pub fn to_db_value_official_account(content: &CodexOfficialAccountContent) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "provider_id".to_string(),
        Value::String(content.provider_id.clone()),
    );
    map.insert("name".to_string(), Value::String(content.name.clone()));
    map.insert("kind".to_string(), Value::String(content.kind.clone()));
    map.insert(
        "auth_snapshot".to_string(),
        Value::String(content.auth_snapshot.clone()),
    );

    if let Some(ref email) = content.email {
        map.insert("email".to_string(), Value::String(email.clone()));
    }
    if let Some(ref auth_mode) = content.auth_mode {
        map.insert("auth_mode".to_string(), Value::String(auth_mode.clone()));
    }
    if let Some(ref account_id) = content.account_id {
        map.insert("account_id".to_string(), Value::String(account_id.clone()));
    }
    if let Some(ref plan_type) = content.plan_type {
        map.insert("plan_type".to_string(), Value::String(plan_type.clone()));
    }
    if let Some(ref last_refresh) = content.last_refresh {
        map.insert(
            "last_refresh".to_string(),
            Value::String(last_refresh.clone()),
        );
    }
    if let Some(ref label) = content.limit_short_label {
        map.insert(
            "limit_short_label".to_string(),
            Value::String(label.clone()),
        );
    }
    if let Some(ref limit_5h_text) = content.limit_5h_text {
        map.insert(
            "limit_5h_text".to_string(),
            Value::String(limit_5h_text.clone()),
        );
    }
    if let Some(ref limit_weekly_text) = content.limit_weekly_text {
        map.insert(
            "limit_weekly_text".to_string(),
            Value::String(limit_weekly_text.clone()),
        );
    }
    if let Some(limit_5h_reset_at) = content.limit_5h_reset_at {
        map.insert(
            "limit_5h_reset_at".to_string(),
            Value::Number(limit_5h_reset_at.into()),
        );
    }
    if let Some(limit_weekly_reset_at) = content.limit_weekly_reset_at {
        map.insert(
            "limit_weekly_reset_at".to_string(),
            Value::Number(limit_weekly_reset_at.into()),
        );
    }
    if let Some(ref last_limits_fetched_at) = content.last_limits_fetched_at {
        map.insert(
            "last_limits_fetched_at".to_string(),
            Value::String(last_limits_fetched_at.clone()),
        );
    }
    if let Some(ref last_error) = content.last_error {
        map.insert("last_error".to_string(), Value::String(last_error.clone()));
    }
    if let Some(sort_index) = content.sort_index {
        map.insert("sort_index".to_string(), Value::Number(sort_index.into()));
    }

    map.insert("is_applied".to_string(), Value::Bool(content.is_applied));
    map.insert(
        "created_at".to_string(),
        Value::String(content.created_at.clone()),
    );
    map.insert(
        "updated_at".to_string(),
        Value::String(content.updated_at.clone()),
    );

    Value::Object(map)
}

// ============================================================================
// Common Config Adapter Functions
// ============================================================================

/// Convert database value to CodexCommonConfig
pub fn from_db_value_common(value: Value) -> CodexCommonConfig {
    let updated_at_value = value
        .get("updated_at")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    CodexCommonConfig {
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
        updated_at: updated_at_value.unwrap_or_else(|| Local::now().to_rfc3339()),
    }
}

/// Convert config string to database value
pub fn to_db_value_common(config: &str, root_dir: Option<&str>) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("config".to_string(), Value::String(config.to_string()));
    if let Some(root_dir) = root_dir.filter(|dir| !dir.trim().is_empty()) {
        map.insert("root_dir".to_string(), Value::String(root_dir.to_string()));
    }
    map.insert(
        "updated_at".to_string(),
        Value::String(Local::now().to_rfc3339()),
    );
    Value::Object(map)
}

// ============================================================================
// Prompt Adapter Functions
// ============================================================================

pub fn from_db_value_prompt(value: Value) -> CodexPromptConfig {
    let id = db_extract_id(&value);

    CodexPromptConfig {
        id,
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed Prompt")
            .to_string(),
        content: value
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        is_applied: value
            .get("is_applied")
            .or_else(|| value.get("isApplied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        sort_index: value
            .get("sort_index")
            .or_else(|| value.get("sortIndex"))
            .and_then(|v| v.as_i64())
            .map(|n| n as i32),
        created_at: value
            .get("created_at")
            .or_else(|| value.get("createdAt"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        updated_at: value
            .get("updated_at")
            .or_else(|| value.get("updatedAt"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    }
}

pub fn to_db_value_prompt(content: &CodexPromptConfigContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|e| {
        eprintln!("Failed to serialize Codex prompt content: {}", e);
        Value::Object(serde_json::Map::new())
    })
}
