use super::types::{
    AntigravityCommonConfig, AntigravityOfficialAccount, AntigravityOfficialAccountContent,
    AntigravityPromptConfig, AntigravityPromptConfigContent, AntigravityProvider,
    AntigravityProviderContent,
};
use crate::coding::db_id::db_extract_id;
use chrono::Local;
use serde_json::{json, Value};

fn get_str_compat(value: &Value, snake_key: &str, camel_key: &str, default: &str) -> String {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

fn get_opt_str_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<String> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn get_i64_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<i32> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
}

fn get_bool_compat(value: &Value, snake_key: &str, camel_key: &str, default: bool) -> bool {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
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

pub(super) fn token_preview_from_snapshot(snapshot: &str, pointer: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(snapshot).ok()?;
    parsed
        .pointer(pointer)
        .and_then(Value::as_str)
        .and_then(mask_token_preview)
}

pub(super) fn token_expires_at_from_snapshot(snapshot: &str) -> Option<i64> {
    let parsed = serde_json::from_str::<Value>(snapshot).ok()?;
    parsed
        .pointer("/token/expiry")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
        .or_else(|| {
            parsed
                .pointer("/expiry_date")
                .and_then(Value::as_i64)
                .map(|value| value / 1000)
        })
}

pub fn from_db_value_provider(value: Value) -> AntigravityProvider {
    AntigravityProvider {
        id: db_extract_id(&value),
        name: get_str_compat(&value, "name", "name", "Unnamed Provider"),
        category: get_str_compat(&value, "category", "category", "custom"),
        settings_config: get_str_compat(&value, "settings_config", "settingsConfig", "{}"),
        source_provider_id: get_opt_str_compat(&value, "source_provider_id", "sourceProviderId"),
        website_url: get_opt_str_compat(&value, "website_url", "websiteUrl"),
        notes: get_opt_str_compat(&value, "notes", "notes"),
        icon: get_opt_str_compat(&value, "icon", "icon"),
        icon_color: get_opt_str_compat(&value, "icon_color", "iconColor"),
        sort_index: get_i64_compat(&value, "sort_index", "sortIndex"),
        meta: value.get("meta").cloned(),
        is_applied: get_bool_compat(&value, "is_applied", "isApplied", false),
        is_disabled: get_bool_compat(&value, "is_disabled", "isDisabled", false),
        created_at: get_str_compat(&value, "created_at", "createdAt", ""),
        updated_at: get_str_compat(&value, "updated_at", "updatedAt", ""),
    }
}

pub fn to_db_value_provider(content: &AntigravityProviderContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|error| {
        eprintln!(
            "Failed to serialize Antigravity provider content: {}",
            error
        );
        json!({})
    })
}

pub fn from_db_value_official_account(value: Value) -> AntigravityOfficialAccount {
    let auth_snapshot = get_str_compat(&value, "auth_snapshot", "authSnapshot", "");

    AntigravityOfficialAccount {
        id: db_extract_id(&value),
        provider_id: get_str_compat(&value, "provider_id", "providerId", ""),
        name: get_str_compat(&value, "name", "name", ""),
        kind: get_str_compat(&value, "kind", "kind", "oauth"),
        email: get_opt_str_compat(&value, "email", "email"),
        auth_snapshot: Some(auth_snapshot.clone()),
        auth_mode: get_opt_str_compat(&value, "auth_mode", "authMode"),
        account_id: get_opt_str_compat(&value, "account_id", "accountId"),
        project_id: get_opt_str_compat(&value, "project_id", "projectId"),
        plan_type: get_opt_str_compat(&value, "plan_type", "planType"),
        last_refresh: get_opt_str_compat(&value, "last_refresh", "lastRefresh"),
        token_expires_at: token_expires_at_from_snapshot(&auth_snapshot),
        access_token_preview: token_preview_from_snapshot(&auth_snapshot, "/token/access_token")
            .or_else(|| token_preview_from_snapshot(&auth_snapshot, "/access_token")),
        refresh_token_preview: token_preview_from_snapshot(&auth_snapshot, "/token/refresh_token")
            .or_else(|| token_preview_from_snapshot(&auth_snapshot, "/refresh_token")),
        limit_short_label: get_opt_str_compat(&value, "limit_short_label", "limitShortLabel"),
        limit_5h_text: get_opt_str_compat(&value, "limit_5h_text", "limit5hText"),
        limit_weekly_text: get_opt_str_compat(&value, "limit_weekly_text", "limitWeeklyText"),
        limit_5h_reset_at: value
            .get("limit_5h_reset_at")
            .or_else(|| value.get("limit5hResetAt"))
            .and_then(Value::as_i64),
        limit_weekly_reset_at: value
            .get("limit_weekly_reset_at")
            .or_else(|| value.get("limitWeeklyResetAt"))
            .and_then(Value::as_i64),
        last_limits_fetched_at: get_opt_str_compat(
            &value,
            "last_limits_fetched_at",
            "lastLimitsFetchedAt",
        ),
        last_error: get_opt_str_compat(&value, "last_error", "lastError"),
        sort_index: get_i64_compat(&value, "sort_index", "sortIndex"),
        is_applied: get_bool_compat(&value, "is_applied", "isApplied", false),
        is_virtual: false,
        created_at: get_str_compat(&value, "created_at", "createdAt", ""),
        updated_at: get_str_compat(&value, "updated_at", "updatedAt", ""),
    }
}

pub fn to_db_value_official_account(content: &AntigravityOfficialAccountContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|error| {
        eprintln!(
            "Failed to serialize Antigravity official account content: {}",
            error
        );
        json!({})
    })
}

pub(super) fn merge_local_default_snapshot(
    previous: Option<Value>,
    provider_id: &str,
    snapshot: &str,
    now: &str,
) -> Value {
    let mut record = previous.unwrap_or_else(|| {
        json!({
            "name": "Antigravity CLI 默认配置", "kind": "local",
            "sort_index": i32::MIN, "is_applied": false, "created_at": now,
        })
    });
    record["provider_id"] = json!(provider_id);
    record["auth_snapshot"] = json!(snapshot);
    // Distinguish a confirmed logged-out default from an ambiguous legacy empty value.
    record["snapshot_captured_from_default"] = json!(true);
    record["updated_at"] = json!(now);
    record
}

pub fn from_db_value_common(value: Value) -> AntigravityCommonConfig {
    AntigravityCommonConfig {
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
            .map(str::to_string)
            .unwrap_or_else(|| Local::now().to_rfc3339()),
    }
}

pub fn to_db_value_common(config: &str, root_dir: Option<&str>) -> Value {
    let mut value = json!({
        "config": config,
        "updated_at": Local::now().to_rfc3339(),
    });

    if let Some(root_dir) = root_dir.filter(|dir| !dir.trim().is_empty()) {
        value["root_dir"] = json!(root_dir);
    }

    value
}

pub fn from_db_value_prompt(value: Value) -> AntigravityPromptConfig {
    AntigravityPromptConfig {
        id: db_extract_id(&value),
        name: get_str_compat(&value, "name", "name", "Unnamed Prompt"),
        content: get_str_compat(&value, "content", "content", ""),
        target_file: get_opt_str_compat(&value, "target_file", "targetFile"),
        is_applied: get_bool_compat(&value, "is_applied", "isApplied", false),
        sort_index: get_i64_compat(&value, "sort_index", "sortIndex"),
        created_at: get_opt_str_compat(&value, "created_at", "createdAt"),
        updated_at: get_opt_str_compat(&value, "updated_at", "updatedAt"),
    }
}

pub fn to_db_value_prompt(content: &AntigravityPromptConfigContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|error| {
        eprintln!("Failed to serialize Antigravity prompt content: {}", error);
        json!({})
    })
}
