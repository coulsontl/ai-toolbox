//! Gateway-only provider records for the WorkBuddy family.
//!
//! The pages derive their channel list from `models.json`, which has no
//! provider ids and cannot carry persistable state. Gateway takeover needs
//! both: a stable id to record in the manifest and per-provider settings
//! (enabled flag, sort order, protocol overrides, billing meta) that must
//! survive a restart.
//!
//! So these rows are **not** a second source of truth for the page. They are
//! written by the gateway when it engages (one row per channel, keyed by the
//! same `(baseUrl, apiKey)` grouping the page uses) and read only by the
//! gateway's candidate loader. A `models.json` edit that adds a channel does not
//! create a row until the gateway is engaged again; a row whose channel
//! disappeared is skipped as a candidate rather than failing the whole list.

use serde_json::{json, Value};

use crate::coding::db_id::db_extract_id;

/// One provider record, in the shape the gateway's candidate loader expects.
///
/// Field names match every other `<cli>_provider` table so the shared adapter
/// paths (`provider_meta_from_record`, ordering, selection) keep working
/// unchanged.
#[derive(Debug, Clone)]
pub struct FamilyGatewayProvider {
    pub id: String,
    pub name: String,
    pub category: String,
    /// JSON carrying `baseUrl` / `apiKey` (the same pair the channel view uses)
    /// plus any protocol overrides.
    pub settings_config: String,
    pub sort_index: Option<i32>,
    pub meta: Option<Value>,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub fn provider_from_db_value(value: Value) -> FamilyGatewayProvider {
    FamilyGatewayProvider {
        id: db_extract_id(&value),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
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
        sort_index: value
            .get("sort_index")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
        meta: value.get("meta").cloned(),
        is_disabled: value
            .get("is_disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        created_at: value
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// Builds the DB payload for one channel.
///
/// `settings_config` keeps the raw base URL (without the `/chat/completions`
/// suffix the product stores) so the gateway's own URL builder produces the
/// request path, and keeps the key verbatim.
pub fn provider_payload(
    name: &str,
    base_url: &str,
    api_key: &str,
    sort_index: i32,
) -> Value {
    json!({
        "name": name,
        "category": "custom",
        "settings_config": json!({
            "baseUrl": base_url,
            "apiKey": api_key,
        })
        .to_string(),
        "sort_index": sort_index,
        "is_applied": false,
        "is_disabled": false,
    })
}

/// Reads `baseUrl` / `apiKey` back out of a stored `settings_config`.
pub fn channel_from_settings_config(settings_config: &str) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(settings_config).ok()?;
    let base_url = value
        .get("baseUrl")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let api_key = value.get("apiKey").and_then(Value::as_str)?.to_string();
    Some((base_url, api_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_payload_round_trips_its_channel_pair() {
        let payload = provider_payload(
            "OpenAI",
            "https://api.example.com/v1",
            "sk-test",
            3,
        );
        let settings = payload
            .get("settings_config")
            .and_then(Value::as_str)
            .expect("settings_config is a JSON string");
        assert_eq!(
            channel_from_settings_config(settings),
            Some((
                "https://api.example.com/v1".to_string(),
                "sk-test".to_string()
            ))
        );
    }

    #[test]
    fn a_disabled_flag_defaults_off_and_disabled_records_read_back() {
        let payload = provider_payload("n", "u", "k", 0);
        assert_eq!(
            payload.get("is_disabled").and_then(Value::as_bool),
            Some(false)
        );

        let mut value = payload.clone();
        value["id"] = json!("abc");
        value["is_disabled"] = json!(true);
        let provider = provider_from_db_value(value);
        assert!(provider.is_disabled);
        assert_eq!(provider.id, "abc");
    }

    #[test]
    fn a_settings_config_without_the_pair_is_rejected() {
        assert_eq!(channel_from_settings_config("{}"), None);
        assert_eq!(channel_from_settings_config("not json"), None);
    }
}
