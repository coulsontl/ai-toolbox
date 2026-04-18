use super::types::{
    OhMyOpenCodeSlimConfig, OhMyOpenCodeSlimConfigContent, OhMyOpenCodeSlimFallbackConfig,
    OhMyOpenCodeSlimGlobalConfig, OhMyOpenCodeSlimGlobalConfigContent,
};
use crate::coding::db_id::db_extract_id;
use serde_json::{json, Map, Value};

// ============================================================================
// Helper Functions
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

/// Helper function to get bool with backward compatibility
pub fn get_bool_compat(value: &Value, snake_key: &str, camel_key: &str, default: bool) -> bool {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// Deep merge two JSON Values recursively
/// Overlay values will overwrite base values for the same keys
pub fn deep_merge_json(base: &mut Value, overlay: &Value) {
    if let (Some(base_obj), Some(overlay_obj)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay_obj {
            if let Some(base_value) = base_obj.get_mut(key) {
                if base_value.is_object() && value.is_object() {
                    deep_merge_json(base_value, value);
                } else {
                    *base_value = value.clone();
                }
            } else {
                base_obj.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Recursively remove empty objects and null values from a JSON value
/// This is useful for cleaning up config files before writing
pub fn clean_empty_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_key, v| {
                clean_empty_values(v);
                // 删除空对象和 null 值，保留空数组
                !(v.is_object() && v.as_object().unwrap().is_empty()) && !v.is_null()
            });
        }
        _ => {}
    }
}

fn get_u64_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<u64> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|v| v.as_u64())
}

pub fn parse_fallback_config_value(value: &Value) -> Option<OhMyOpenCodeSlimFallbackConfig> {
    let fallback_obj = value.as_object()?;
    let mut other_fields = std::collections::BTreeMap::new();
    for (key, raw_value) in fallback_obj {
        if matches!(
            key.as_str(),
            "enabled"
                | "timeout_ms"
                | "timeoutMs"
                | "retry_delay_ms"
                | "retryDelayMs"
                | "retry_on_empty"
                | "retryOnEmpty"
                | "chains"
        ) {
            continue;
        }
        other_fields.insert(key.clone(), raw_value.clone());
    }

    Some(OhMyOpenCodeSlimFallbackConfig {
        enabled: fallback_obj.get("enabled").and_then(|v| v.as_bool()),
        timeout_ms: get_u64_compat(value, "timeout_ms", "timeoutMs"),
        retry_delay_ms: get_u64_compat(value, "retry_delay_ms", "retryDelayMs"),
        retry_on_empty: value
            .get("retry_on_empty")
            .or_else(|| value.get("retryOnEmpty"))
            .and_then(|v| v.as_bool()),
        chains: value.get("chains").cloned(),
        other_fields,
    })
}

fn collect_legacy_agent_fallback_chains(agents: Option<&Value>) -> Option<Value> {
    let agents_obj = agents.and_then(|value| value.as_object())?;
    let mut chains = Map::new();

    for (agent_key, agent_value) in agents_obj {
        let agent_obj = match agent_value.as_object() {
            Some(agent_obj) => agent_obj,
            None => continue,
        };

        let fallback_value = match agent_obj.get("fallback_models") {
            Some(fallback_value) => fallback_value,
            None => continue,
        };

        let normalized_value = match fallback_value {
            Value::String(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    continue;
                }
                Value::Array(vec![Value::String(trimmed.to_string())])
            }
            Value::Array(items) => {
                let normalized_items: Vec<Value> = items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| Value::String(item.to_string()))
                    .collect();
                if normalized_items.is_empty() {
                    continue;
                }
                Value::Array(normalized_items)
            }
            _ => continue,
        };

        chains.insert(agent_key.clone(), normalized_value);
    }

    if chains.is_empty() {
        None
    } else {
        Some(Value::Object(chains))
    }
}

fn merge_fallback_chains_preserving_primary(primary: Value, secondary: Value) -> Value {
    match (primary, secondary) {
        (Value::Object(mut primary_obj), Value::Object(secondary_obj)) => {
            for (key, value) in secondary_obj {
                primary_obj.entry(key).or_insert(value);
            }
            Value::Object(primary_obj)
        }
        (primary_value, _) => primary_value,
    }
}

pub fn merge_fallback_configs(
    primary: Option<OhMyOpenCodeSlimFallbackConfig>,
    secondary: Option<OhMyOpenCodeSlimFallbackConfig>,
) -> Option<OhMyOpenCodeSlimFallbackConfig> {
    match (primary, secondary) {
        (None, None) => None,
        (Some(primary), None) => Some(primary),
        (None, Some(secondary)) => Some(secondary),
        (Some(mut primary), Some(secondary)) => {
            if primary.enabled.is_none() {
                primary.enabled = secondary.enabled;
            }
            if primary.timeout_ms.is_none() {
                primary.timeout_ms = secondary.timeout_ms;
            }
            if primary.retry_delay_ms.is_none() {
                primary.retry_delay_ms = secondary.retry_delay_ms;
            }
            if primary.retry_on_empty.is_none() {
                primary.retry_on_empty = secondary.retry_on_empty;
            }

            let primary_chains = primary.chains.take();
            primary.chains = match (primary_chains, secondary.chains) {
                (Some(mut primary_chains), Some(secondary_chains)) => {
                    primary_chains =
                        merge_fallback_chains_preserving_primary(primary_chains, secondary_chains);
                    Some(primary_chains)
                }
                (Some(primary_chains), None) => Some(primary_chains),
                (None, Some(secondary_chains)) => Some(secondary_chains),
                (None, None) => None,
            };

            for (key, value) in secondary.other_fields {
                primary.other_fields.entry(key).or_insert(value);
            }

            Some(primary)
        }
    }
}

pub fn fallback_config_to_value(config: &OhMyOpenCodeSlimFallbackConfig) -> Option<Value> {
    let mut fallback_obj = Map::new();

    if let Some(enabled) = config.enabled {
        fallback_obj.insert("enabled".to_string(), Value::Bool(enabled));
    }
    if let Some(timeout_ms) = config.timeout_ms {
        fallback_obj.insert("timeoutMs".to_string(), Value::Number(timeout_ms.into()));
    }
    if let Some(retry_delay_ms) = config.retry_delay_ms {
        fallback_obj.insert(
            "retryDelayMs".to_string(),
            Value::Number(retry_delay_ms.into()),
        );
    }
    if let Some(retry_on_empty) = config.retry_on_empty {
        fallback_obj.insert("retry_on_empty".to_string(), Value::Bool(retry_on_empty));
    }
    if let Some(chains) = &config.chains {
        fallback_obj.insert("chains".to_string(), chains.clone());
    }
    for (key, value) in &config.other_fields {
        fallback_obj.insert(key.clone(), value.clone());
    }

    if fallback_obj.is_empty() {
        None
    } else {
        Some(Value::Object(fallback_obj))
    }
}

pub fn merge_fallback_values(primary: Option<Value>, secondary: Option<Value>) -> Option<Value> {
    match (primary, secondary) {
        (None, None) => None,
        (Some(primary_value), None) => Some(primary_value),
        (None, Some(secondary_value)) => Some(secondary_value),
        (Some(primary_value), Some(secondary_value)) => {
            match (
                parse_fallback_config_value(&primary_value),
                parse_fallback_config_value(&secondary_value),
            ) {
                (Some(primary_config), Some(secondary_config)) => {
                    merge_fallback_configs(Some(primary_config), Some(secondary_config))
                        .and_then(|merged_config| fallback_config_to_value(&merged_config))
                }
                _ => {
                    if primary_value.is_object() && secondary_value.is_object() {
                        let mut merged_value = secondary_value;
                        deep_merge_json(&mut merged_value, &primary_value);
                        Some(merged_value)
                    } else {
                        Some(primary_value)
                    }
                }
            }
        }
    }
}

// ============================================================================
// Adapter Functions
// ============================================================================

/// Convert database Value to OhMyOpenCodeSlimConfig (AgentsProfile) with fault tolerance
pub fn from_db_value(value: Value) -> OhMyOpenCodeSlimConfig {
    let is_applied = get_bool_compat(&value, "is_applied", "isApplied", false);
    let is_disabled = get_bool_compat(&value, "is_disabled", "isDisabled", false);
    let raw_other_fields = value
        .get("other_fields")
        .or_else(|| value.get("otherFields"))
        .cloned();
    let fallback_from_value = value.get("fallback").and_then(parse_fallback_config_value);
    let fallback_from_other_fields = raw_other_fields
        .as_ref()
        .and_then(|other| other.get("fallback"))
        .and_then(parse_fallback_config_value);
    let fallback_from_legacy_agents = collect_legacy_agent_fallback_chains(value.get("agents"))
        .map(|chains| OhMyOpenCodeSlimFallbackConfig {
            enabled: None,
            timeout_ms: None,
            retry_delay_ms: None,
            retry_on_empty: None,
            chains: Some(chains),
            other_fields: std::collections::BTreeMap::new(),
        });
    let legacy_council = raw_other_fields
        .as_ref()
        .and_then(|other| other.get("council"))
        .cloned();
    let cleaned_other_fields = raw_other_fields.and_then(|mut other| {
        if let Some(map) = other.as_object_mut() {
            map.remove("council");
            map.remove("fallback");
            if map.is_empty() {
                return None;
            }
        }
        Some(other)
    });
    let sort_index = value
        .get("sort_index")
        .or_else(|| value.get("sortIndex"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    OhMyOpenCodeSlimConfig {
        id: db_extract_id(&value),
        name: get_str_compat(&value, "name", "name", "Unnamed Config"),
        is_applied,
        is_disabled,
        agents: value.get("agents").cloned(),
        council: value.get("council").cloned().or(legacy_council),
        fallback: merge_fallback_configs(
            merge_fallback_configs(fallback_from_value, fallback_from_other_fields),
            fallback_from_legacy_agents,
        ),
        other_fields: cleaned_other_fields,
        sort_index,
        created_at: get_opt_str_compat(&value, "created_at", "createdAt"),
        updated_at: get_opt_str_compat(&value, "updated_at", "updatedAt"),
    }
}

/// Convert OhMyOpenCodeSlimConfigContent to database Value
pub fn to_db_value(content: &OhMyOpenCodeSlimConfigContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|e| {
        eprintln!(
            "Failed to serialize oh-my-opencode-slim config content: {}",
            e
        );
        json!({})
    })
}

/// Helper function to safely convert Value to Option<Vec<String>>, handling SurrealDB types
fn safe_to_string_array(value: &Value) -> Option<Vec<String>> {
    match value {
        // Already an array of strings
        Value::Array(arr) => {
            let mut result = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    result.push(s.to_string());
                } else {
                    // Non-string item, try to convert
                    if let Ok(s) = serde_json::from_value(item.clone()) {
                        result.push(s);
                    } else {
                        return None;
                    }
                }
            }
            Some(result)
        }
        // SurrealDB enum - try to parse
        Value::String(s) if s.starts_with("enum(") => {
            // Try to extract the value from enum format
            let inner = s.trim_start_matches("enum(").trim_end_matches(')');
            Some(vec![inner.to_string()])
        }
        _ => {
            // Try generic conversion
            serde_json::from_value(value.clone()).ok()
        }
    }
}

/// Convert database Value to OhMyOpenCodeSlimGlobalConfig with fault tolerance
pub fn global_config_from_db_value(value: Value) -> OhMyOpenCodeSlimGlobalConfig {
    let raw_other_fields = value
        .get("other_fields")
        .or_else(|| value.get("otherFields"))
        .cloned();
    let legacy_council = raw_other_fields
        .as_ref()
        .and_then(|other| other.get("council"))
        .cloned();
    let cleaned_other_fields = raw_other_fields.and_then(|mut other| {
        if let Some(map) = other.as_object_mut() {
            map.remove("council");
            if map.is_empty() {
                return None;
            }
        }
        Some(other)
    });

    OhMyOpenCodeSlimGlobalConfig {
        id: db_extract_id(&value),
        sisyphus_agent: value
            .get("sisyphus_agent")
            .or_else(|| value.get("sisyphusAgent"))
            .cloned(),
        disabled_agents: value
            .get("disabled_agents")
            .or_else(|| value.get("disabledAgents"))
            .and_then(|v| safe_to_string_array(v)),
        disabled_mcps: value
            .get("disabled_mcps")
            .or_else(|| value.get("disabledMcps"))
            .and_then(|v| safe_to_string_array(v)),
        disabled_hooks: value
            .get("disabled_hooks")
            .or_else(|| value.get("disabledHooks"))
            .and_then(|v| safe_to_string_array(v)),
        lsp: value.get("lsp").cloned(),
        experimental: value.get("experimental").cloned(),
        council: value.get("council").cloned().or(legacy_council),
        other_fields: cleaned_other_fields,
        updated_at: get_opt_str_compat(&value, "updated_at", "updatedAt"),
    }
}

/// Convert OhMyOpenCodeSlimGlobalConfigContent to database Value
pub fn global_config_to_db_value(content: &OhMyOpenCodeSlimGlobalConfigContent) -> Value {
    serde_json::to_value(content).unwrap_or_else(|e| {
        eprintln!(
            "Failed to serialize oh-my-opencode-slim global config content: {}",
            e
        );
        json!({})
    })
}

#[cfg(test)]
mod tests {
    use super::{
        fallback_config_to_value, from_db_value, global_config_from_db_value,
        merge_fallback_values, parse_fallback_config_value,
    };
    use crate::coding::oh_my_opencode_slim::types::OhMyOpenCodeSlimFallbackConfig;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn from_db_value_merges_fallback_sources_without_overwriting_primary_chains() {
        let config = from_db_value(json!({
            "id": "oh_my_opencode_slim_profile:test-profile",
            "name": "Test Profile",
            "agents": {
                "oracle": {
                    "model": "gpt-5.4",
                    "fallback_models": ["legacy-oracle"]
                },
                "orchestrator": {
                    "fallback_models": "legacy-orchestrator"
                }
            },
            "fallback": {
                "enabled": true,
                "timeoutMs": 1200,
                "retryDelayMs": 80,
                "retry_on_empty": true,
                "chains": {
                    "oracle": ["top-oracle"]
                },
                "strategy": "prefer-top-level"
            },
            "other_fields": {
                "fallback": {
                    "enabled": false,
                    "chains": {
                        "fixer": ["other-fixer"],
                        "oracle": ["other-oracle"]
                    }
                },
                "theme": "compact"
            }
        }));

        let fallback = config.fallback.expect("fallback should be extracted");
        assert_eq!(config.id, "test-profile");
        assert_eq!(fallback.enabled, Some(true));
        assert_eq!(fallback.timeout_ms, Some(1200));
        assert_eq!(fallback.retry_delay_ms, Some(80));
        assert_eq!(fallback.retry_on_empty, Some(true));
        assert_eq!(
            fallback.chains,
            Some(json!({
                "oracle": ["top-oracle"],
                "fixer": ["other-fixer"],
                "orchestrator": ["legacy-orchestrator"]
            }))
        );
        assert_eq!(
            fallback.other_fields.get("strategy"),
            Some(&json!("prefer-top-level"))
        );
        assert_eq!(config.other_fields, Some(json!({ "theme": "compact" })));
    }

    #[test]
    fn fallback_config_roundtrips_string_and_array_chain_shapes() {
        let fallback_value = fallback_config_to_value(&OhMyOpenCodeSlimFallbackConfig {
            enabled: Some(false),
            timeout_ms: Some(1500),
            retry_delay_ms: Some(90),
            retry_on_empty: Some(true),
            chains: Some(json!({
                "oracle": "gpt-5.4-mini",
                "fixer": ["gpt-5.4", "gpt-5.4-mini"]
            })),
            other_fields: BTreeMap::from([("strategy".to_string(), json!("aggressive"))]),
        })
        .expect("fallback should serialize");

        assert_eq!(
            fallback_value,
            json!({
                "enabled": false,
                "timeoutMs": 1500,
                "retryDelayMs": 90,
                "retry_on_empty": true,
                "chains": {
                    "oracle": "gpt-5.4-mini",
                    "fixer": ["gpt-5.4", "gpt-5.4-mini"]
                },
                "strategy": "aggressive"
            })
        );

        let parsed = parse_fallback_config_value(&fallback_value).expect("fallback should parse");
        assert_eq!(parsed.enabled, Some(false));
        assert_eq!(parsed.timeout_ms, Some(1500));
        assert_eq!(parsed.retry_delay_ms, Some(90));
        assert_eq!(parsed.retry_on_empty, Some(true));
        assert_eq!(
            parsed.chains,
            Some(json!({
                "oracle": "gpt-5.4-mini",
                "fixer": ["gpt-5.4", "gpt-5.4-mini"]
            }))
        );
        assert_eq!(
            parsed.other_fields.get("strategy"),
            Some(&json!("aggressive"))
        );
    }

    #[test]
    fn global_config_keeps_fallback_inside_other_fields() {
        let config = global_config_from_db_value(json!({
            "id": "oh_my_opencode_slim_global:global",
            "other_fields": {
                "fallback": {
                    "enabled": true,
                    "chains": {
                        "oracle": ["gpt-5.4"]
                    }
                },
                "theme": "compact"
            }
        }));

        assert_eq!(config.id, "global");
        assert_eq!(
            config.other_fields,
            Some(json!({
                "fallback": {
                    "enabled": true,
                    "chains": {
                        "oracle": ["gpt-5.4"]
                    }
                },
                "theme": "compact"
            }))
        );
    }

    #[test]
    fn merge_fallback_values_keeps_global_settings_when_profile_only_overrides_chains() {
        let merged_fallback = merge_fallback_values(
            Some(json!({
                "chains": {
                    "oracle": ["profile-oracle"]
                }
            })),
            Some(json!({
                "enabled": true,
                "timeout_ms": 1500,
                "strategy": "shared",
                "chains": {
                    "fixer": ["global-fixer"]
                }
            })),
        )
        .expect("fallback should merge");

        assert_eq!(
            merged_fallback,
            json!({
                "enabled": true,
                "timeoutMs": 1500,
                "strategy": "shared",
                "chains": {
                    "oracle": ["profile-oracle"],
                    "fixer": ["global-fixer"]
                }
            })
        );
    }
}
