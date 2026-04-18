use serde_json::{Map, Value};

const PROTECTED_TOP_LEVEL_FIELDS: [&str; 3] = ["enabledPlugins", "extraKnownMarketplaces", "hooks"];

const PROVIDER_MODEL_FIELD_MAPPINGS: [(&str, &str); 5] = [
    ("model", "ANTHROPIC_MODEL"),
    ("haikuModel", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
    ("sonnetModel", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
    ("opusModel", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
    ("reasoningModel", "ANTHROPIC_REASONING_MODEL"),
];

fn value_as_object(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

fn merge_json_value_preserving_existing(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(target_map), Value::Object(source_map)) => {
            for (key, source_value) in source_map {
                match target_map.get_mut(key) {
                    Some(target_value) => {
                        merge_json_value_preserving_existing(target_value, source_value)
                    }
                    None => {
                        target_map.insert(key.clone(), source_value.clone());
                    }
                }
            }
        }
        (target_value, source_value) => {
            *target_value = source_value.clone();
        }
    }
}

fn json_is_subset(target: &Value, source: &Value) -> bool {
    match source {
        Value::Object(source_map) => {
            let Some(target_map) = target.as_object() else {
                return false;
            };
            source_map.iter().all(|(key, source_value)| {
                target_map
                    .get(key)
                    .is_some_and(|target_value| json_is_subset(target_value, source_value))
            })
        }
        Value::Array(source_array) => {
            let Some(target_array) = target.as_array() else {
                return false;
            };
            json_array_contains_subset(target_array, source_array)
        }
        _ => target == source,
    }
}

fn json_array_contains_subset(target_array: &[Value], source_array: &[Value]) -> bool {
    let mut matched = vec![false; target_array.len()];

    source_array.iter().all(|source_item| {
        if let Some((index, _)) = target_array
            .iter()
            .enumerate()
            .find(|(index, target_item)| {
                !matched[*index] && json_is_subset(target_item, source_item)
            })
        {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn json_remove_array_items(target_array: &mut Vec<Value>, source_array: &[Value]) {
    for source_item in source_array {
        if let Some(index) = target_array
            .iter()
            .position(|target_item| json_is_subset(target_item, source_item))
        {
            target_array.remove(index);
        }
    }
}

fn json_deep_remove(target: &mut Value, source: &Value) {
    let (Some(target_map), Some(source_map)) = (target.as_object_mut(), source.as_object()) else {
        return;
    };

    for (key, source_value) in source_map {
        let mut remove_key = false;

        if let Some(target_value) = target_map.get_mut(key) {
            if source_value.is_object() && target_value.is_object() {
                json_deep_remove(target_value, source_value);
                remove_key = target_value.as_object().is_some_and(|obj| obj.is_empty());
            } else if let (Some(target_array), Some(source_array)) =
                (target_value.as_array_mut(), source_value.as_array())
            {
                json_remove_array_items(target_array, source_array);
                remove_key = target_array.is_empty();
            } else if json_is_subset(target_value, source_value) {
                remove_key = true;
            }
        }

        if remove_key {
            target_map.remove(key);
        }
    }
}

pub fn parse_json_object(raw_json: &str) -> Result<Map<String, Value>, String> {
    if raw_json.trim().is_empty() {
        return Ok(Map::new());
    }

    match serde_json::from_str::<Value>(raw_json)
        .map_err(|error| format!("Failed to parse JSON object: {}", error))?
    {
        Value::Object(object) => Ok(object),
        _ => Err("Expected JSON object".to_string()),
    }
}

pub fn strip_claude_common_config_from_settings(
    settings_value: &Value,
    common_config: &Value,
) -> Result<Value, String> {
    let _ = settings_value
        .as_object()
        .ok_or_else(|| "Claude settings must be a JSON object".to_string())?;

    let common_config_object = match common_config {
        Value::Object(object) => object,
        Value::Null => return Ok(settings_value.clone()),
        _ => return Err("Claude common config must be a JSON object".to_string()),
    };

    let mut sanitized_common_config = common_config_object.clone();
    for protected_field in PROTECTED_TOP_LEVEL_FIELDS {
        sanitized_common_config.remove(protected_field);
    }

    if sanitized_common_config.is_empty() {
        return Ok(settings_value.clone());
    }

    let mut stripped_settings = settings_value.clone();
    json_deep_remove(
        &mut stripped_settings,
        &Value::Object(sanitized_common_config),
    );
    Ok(stripped_settings)
}

pub fn extract_provider_settings_for_storage(
    settings_value: &Value,
    common_config: Option<&Value>,
    known_env_fields: &[&str],
) -> Result<Value, String> {
    let provider_source_settings = if let Some(common_config_value) = common_config {
        strip_claude_common_config_from_settings(settings_value, common_config_value)?
    } else {
        settings_value.clone()
    };

    let (provider_settings, _) =
        split_settings_into_provider_and_common(&provider_source_settings, known_env_fields)?;
    Ok(provider_settings)
}

pub fn build_provider_managed_env(
    provider_config: &Value,
    known_env_fields: &[&str],
) -> Map<String, Value> {
    let mut managed_env = Map::new();

    if let Some(provider_env) = provider_config.get("env").and_then(value_as_object) {
        let api_key_value = provider_env
            .get("ANTHROPIC_AUTH_TOKEN")
            .or_else(|| provider_env.get("ANTHROPIC_API_KEY"));
        if let Some(api_key_value) = api_key_value {
            managed_env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), api_key_value.clone());
        }

        if let Some(base_url_value) = provider_env.get("ANTHROPIC_BASE_URL") {
            managed_env.insert("ANTHROPIC_BASE_URL".to_string(), base_url_value.clone());
        }
    }

    for (provider_field, env_field) in PROVIDER_MODEL_FIELD_MAPPINGS {
        if let Some(field_value) = provider_config.get(provider_field) {
            managed_env.insert(env_field.to_string(), field_value.clone());
        }
    }

    managed_env.retain(|key, value| {
        known_env_fields.contains(&key.as_str())
            && !value.is_null()
            && !value.as_str().is_some_and(str::is_empty)
    });

    managed_env
}

pub fn merge_claude_settings_for_provider(
    current_disk_settings: Option<&Value>,
    previous_common_config: Option<&Value>,
    next_common_config: &Value,
    provider_config: &Value,
    known_env_fields: &[&str],
) -> Result<Value, String> {
    let current_settings_object = match current_disk_settings {
        Some(Value::Object(object)) => object.clone(),
        Some(_) => return Err("Current Claude settings must be a JSON object".to_string()),
        None => Map::new(),
    };

    let next_common_config_object = match next_common_config {
        Value::Object(object) => object.clone(),
        Value::Null => Map::new(),
        _ => return Err("Claude common config must be a JSON object".to_string()),
    };
    let previous_common_config_object = match previous_common_config {
        Some(Value::Object(object)) => object.clone(),
        Some(Value::Null) => Map::new(),
        Some(_) => return Err("Previous Claude common config must be a JSON object".to_string()),
        None => next_common_config_object.clone(),
    };

    let mut merged_settings = current_settings_object;

    for field_key in previous_common_config_object.keys() {
        if field_key == "env" {
            continue;
        }

        if PROTECTED_TOP_LEVEL_FIELDS.contains(&field_key.as_str()) {
            continue;
        }

        if !next_common_config_object.contains_key(field_key) {
            merged_settings.remove(field_key);
        }
    }

    for (field_key, field_value) in &next_common_config_object {
        if field_key == "env" {
            continue;
        }

        if PROTECTED_TOP_LEVEL_FIELDS.contains(&field_key.as_str()) {
            continue;
        }

        if let Some(existing_value) = merged_settings.get_mut(field_key) {
            merge_json_value_preserving_existing(existing_value, field_value);
        } else {
            merged_settings.insert(field_key.clone(), field_value.clone());
        }
    }

    let mut merged_env = merged_settings
        .get("env")
        .and_then(value_as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(previous_common_env) = previous_common_config_object
        .get("env")
        .and_then(value_as_object)
    {
        for field_key in previous_common_env.keys() {
            if !known_env_fields.contains(&field_key.as_str()) {
                merged_env.remove(field_key);
            }
        }
    }

    if let Some(next_common_env) = next_common_config_object
        .get("env")
        .and_then(value_as_object)
    {
        for (field_key, field_value) in next_common_env {
            merged_env.insert(field_key.clone(), field_value.clone());
        }
    }

    for known_env_field in known_env_fields {
        merged_env.remove(*known_env_field);
    }

    for (field_key, field_value) in build_provider_managed_env(provider_config, known_env_fields) {
        merged_env.insert(field_key, field_value);
    }

    if merged_env.is_empty() {
        merged_settings.remove("env");
    } else {
        merged_settings.insert("env".to_string(), Value::Object(merged_env));
    }

    Ok(Value::Object(merged_settings))
}

pub fn split_settings_into_provider_and_common(
    settings_value: &Value,
    known_env_fields: &[&str],
) -> Result<(Value, Value), String> {
    let settings_object = settings_value
        .as_object()
        .ok_or_else(|| "Claude settings must be a JSON object".to_string())?;

    let mut provider_env = Map::new();
    let mut common_env = Map::new();

    if let Some(env_object) = settings_object.get("env").and_then(value_as_object) {
        for (field_key, field_value) in env_object {
            if known_env_fields.contains(&field_key.as_str()) {
                provider_env.insert(field_key.clone(), field_value.clone());
            } else {
                common_env.insert(field_key.clone(), field_value.clone());
            }
        }
    }

    let mut provider_settings = Map::new();
    let mut provider_settings_env = Map::new();

    let api_key_value = provider_env
        .get("ANTHROPIC_AUTH_TOKEN")
        .or_else(|| provider_env.get("ANTHROPIC_API_KEY"));
    if let Some(api_key_value) = api_key_value {
        provider_settings_env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), api_key_value.clone());
    }
    if let Some(base_url_value) = provider_env.get("ANTHROPIC_BASE_URL") {
        provider_settings_env.insert("ANTHROPIC_BASE_URL".to_string(), base_url_value.clone());
    }
    if !provider_settings_env.is_empty() {
        provider_settings.insert("env".to_string(), Value::Object(provider_settings_env));
    }

    for (provider_field, env_field) in PROVIDER_MODEL_FIELD_MAPPINGS {
        if let Some(field_value) = provider_env.get(env_field) {
            provider_settings.insert(provider_field.to_string(), field_value.clone());
        }
    }

    for (provider_field, _) in PROVIDER_MODEL_FIELD_MAPPINGS {
        if let Some(field_value) = settings_object.get(provider_field) {
            provider_settings.insert(provider_field.to_string(), field_value.clone());
        }
    }

    let mut common_settings = Map::new();
    for (field_key, field_value) in settings_object {
        if field_key == "env" {
            continue;
        }
        if PROVIDER_MODEL_FIELD_MAPPINGS
            .iter()
            .any(|(provider_field, _)| provider_field == field_key)
        {
            continue;
        }
        if PROTECTED_TOP_LEVEL_FIELDS.contains(&field_key.as_str()) {
            continue;
        }
        common_settings.insert(field_key.clone(), field_value.clone());
    }

    if !common_env.is_empty() {
        common_settings.insert("env".to_string(), Value::Object(common_env));
    }

    Ok((
        Value::Object(provider_settings),
        Value::Object(common_settings),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        extract_provider_settings_for_storage, merge_claude_settings_for_provider,
        split_settings_into_provider_and_common, strip_claude_common_config_from_settings,
    };
    use serde_json::json;

    const KNOWN_ENV_FIELDS: [&str; 6] = [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_REASONING_MODEL",
    ];

    #[test]
    fn merge_preserves_existing_nested_status_line_details() {
        let current_disk_settings = json!({
            "statusLine": {
                "command": "ccline",
                "type": "command",
                "padding": 2
            },
            "enabledPlugins": ["jarrodwatts/claude-hud"],
            "skipWebFetchPreflight": true,
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "old-token",
                "ANTHROPIC_BASE_URL": "https://old.example.com",
                "CLAUDE_CODE_ENABLE_TELEMETRY": false
            }
        });
        let previous_common_config = json!({
            "statusLine": {},
            "skipWebFetchPreflight": true
        });
        let next_common_config = json!({
            "statusLine": {},
            "skipWebFetchPreflight": false
        });
        let provider_config = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "new-token",
                "ANTHROPIC_BASE_URL": "https://new.example.com"
            },
            "model": "claude-sonnet-4-5"
        });

        let merged_settings = merge_claude_settings_for_provider(
            Some(&current_disk_settings),
            Some(&previous_common_config),
            &next_common_config,
            &provider_config,
            &KNOWN_ENV_FIELDS,
        )
        .expect("merge should succeed");

        assert_eq!(
            merged_settings.get("statusLine"),
            current_disk_settings.get("statusLine")
        );
        assert_eq!(
            merged_settings.get("enabledPlugins"),
            current_disk_settings.get("enabledPlugins")
        );
        assert_eq!(
            merged_settings.get("skipWebFetchPreflight"),
            Some(&json!(false))
        );
        assert_eq!(
            merged_settings.pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY"),
            Some(&json!(false))
        );
        assert_eq!(
            merged_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
            Some(&json!("new-token"))
        );
        assert_eq!(
            merged_settings.pointer("/env/ANTHROPIC_BASE_URL"),
            Some(&json!("https://new.example.com"))
        );
        assert_eq!(
            merged_settings.pointer("/env/ANTHROPIC_MODEL"),
            Some(&json!("claude-sonnet-4-5"))
        );
    }

    #[test]
    fn merge_removes_deleted_top_level_status_line_key() {
        let current_disk_settings = json!({
            "statusLine": {
                "command": "ccline",
                "type": "command"
            },
            "skipWebFetchPreflight": true
        });
        let previous_common_config = json!({
            "statusLine": {},
            "skipWebFetchPreflight": true
        });
        let next_common_config = json!({
            "skipWebFetchPreflight": false
        });

        let merged_settings = merge_claude_settings_for_provider(
            Some(&current_disk_settings),
            Some(&previous_common_config),
            &next_common_config,
            &json!({}),
            &KNOWN_ENV_FIELDS,
        )
        .expect("merge should succeed");

        assert!(merged_settings.get("statusLine").is_none());
        assert_eq!(
            merged_settings.get("skipWebFetchPreflight"),
            Some(&json!(false))
        );
    }

    #[test]
    fn split_excludes_runtime_owned_fields_but_keeps_status_line_in_common_config() {
        let settings_value = json!({
            "statusLine": {
                "command": "ccline",
                "type": "command"
            },
            "enabledPlugins": ["jarrodwatts/claude-hud"],
            "hooks": {
                "preToolUse": []
            },
            "skipWebFetchPreflight": true,
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "token",
                "ANTHROPIC_BASE_URL": "https://example.com",
                "CLAUDE_CODE_ENABLE_TELEMETRY": false
            }
        });

        let (provider_settings, common_settings) =
            split_settings_into_provider_and_common(&settings_value, &KNOWN_ENV_FIELDS)
                .expect("split should succeed");

        assert_eq!(
            provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
            Some(&json!("token"))
        );
        assert_eq!(
            provider_settings.pointer("/env/ANTHROPIC_BASE_URL"),
            Some(&json!("https://example.com"))
        );

        assert_eq!(
            common_settings.get("skipWebFetchPreflight"),
            Some(&json!(true))
        );
        assert_eq!(
            common_settings.get("statusLine"),
            settings_value.get("statusLine")
        );
        assert!(common_settings.get("enabledPlugins").is_none());
        assert!(common_settings.get("hooks").is_none());
        assert_eq!(
            common_settings.pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY"),
            Some(&json!(false))
        );
    }

    #[test]
    fn strip_common_config_preserves_status_line_details_for_empty_object_marker() {
        let settings_value = json!({
            "statusLine": {
                "command": "ccline",
                "type": "command",
                "padding": 2
            },
            "skipWebFetchPreflight": true
        });
        let common_config = json!({
            "statusLine": {},
            "skipWebFetchPreflight": true
        });

        let stripped = strip_claude_common_config_from_settings(&settings_value, &common_config)
            .expect("strip should succeed");

        assert_eq!(stripped.get("statusLine"), settings_value.get("statusLine"));
        assert!(stripped.get("skipWebFetchPreflight").is_none());
    }

    #[test]
    fn strip_common_config_ignores_protected_runtime_owned_fields() {
        let settings_value = json!({
            "enabledPlugins": {
                "claude-hud": true
            },
            "hooks": {
                "preToolUse": []
            },
            "statusLine": {
                "command": "ccline"
            }
        });
        let common_config = json!({
            "enabledPlugins": {},
            "hooks": {},
            "statusLine": {}
        });

        let stripped = strip_claude_common_config_from_settings(&settings_value, &common_config)
            .expect("strip should succeed");

        assert_eq!(
            stripped.get("enabledPlugins"),
            settings_value.get("enabledPlugins")
        );
        assert_eq!(stripped.get("hooks"), settings_value.get("hooks"));
        assert_eq!(stripped.get("statusLine"), settings_value.get("statusLine"));
    }

    #[test]
    fn extract_provider_settings_for_storage_drops_common_fields_after_strip() {
        let settings_value = json!({
            "statusLine": {
                "command": "ccline",
                "type": "command",
                "padding": 2
            },
            "skipWebFetchPreflight": true,
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "token",
                "ANTHROPIC_BASE_URL": "https://example.com",
                "ANTHROPIC_REASONING_MODEL": "claude-reasoning",
                "CLAUDE_CODE_ENABLE_TELEMETRY": false
            }
        });
        let common_config = json!({
            "statusLine": {},
            "skipWebFetchPreflight": true,
            "env": {
                "CLAUDE_CODE_ENABLE_TELEMETRY": false
            }
        });

        let provider_settings = extract_provider_settings_for_storage(
            &settings_value,
            Some(&common_config),
            &KNOWN_ENV_FIELDS,
        )
        .expect("extract should succeed");

        assert!(provider_settings.get("statusLine").is_none());
        assert!(provider_settings.get("skipWebFetchPreflight").is_none());
        assert_eq!(
            provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
            Some(&json!("token"))
        );
        assert_eq!(
            provider_settings.pointer("/env/ANTHROPIC_BASE_URL"),
            Some(&json!("https://example.com"))
        );
        assert_eq!(
            provider_settings.get("reasoningModel"),
            Some(&json!("claude-reasoning"))
        );
        assert!(provider_settings
            .pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY")
            .is_none());
    }

    #[test]
    fn extract_provider_settings_for_storage_keeps_top_level_model_fields_from_form_payload() {
        let settings_value = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "token",
                "ANTHROPIC_BASE_URL": "https://example.com"
            },
            "model": "claude-sonnet-4-5",
            "haikuModel": "claude-3-5-haiku",
            "sonnetModel": "claude-3-7-sonnet",
            "opusModel": "claude-3-opus",
            "reasoningModel": "claude-3-7-thinking",
            "statusLine": {
                "type": "command"
            }
        });

        let provider_settings =
            extract_provider_settings_for_storage(&settings_value, None, &KNOWN_ENV_FIELDS)
                .expect("extract should succeed");

        assert_eq!(
            provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
            Some(&json!("token"))
        );
        assert_eq!(
            provider_settings.pointer("/env/ANTHROPIC_BASE_URL"),
            Some(&json!("https://example.com"))
        );
        assert_eq!(
            provider_settings.get("model"),
            Some(&json!("claude-sonnet-4-5"))
        );
        assert_eq!(
            provider_settings.get("haikuModel"),
            Some(&json!("claude-3-5-haiku"))
        );
        assert_eq!(
            provider_settings.get("sonnetModel"),
            Some(&json!("claude-3-7-sonnet"))
        );
        assert_eq!(
            provider_settings.get("opusModel"),
            Some(&json!("claude-3-opus"))
        );
        assert_eq!(
            provider_settings.get("reasoningModel"),
            Some(&json!("claude-3-7-thinking"))
        );
        assert!(provider_settings.get("statusLine").is_none());
    }

    #[test]
    fn split_settings_into_provider_and_common_keeps_model_fields_out_of_common_config() {
        let settings_value = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "token",
                "ANTHROPIC_BASE_URL": "https://example.com"
            },
            "model": "claude-sonnet-4-5",
            "reasoningModel": "claude-3-7-thinking",
            "skipWebFetchPreflight": true
        });

        let (provider_settings, common_settings) =
            split_settings_into_provider_and_common(&settings_value, &KNOWN_ENV_FIELDS)
                .expect("split should succeed");

        assert_eq!(
            provider_settings.get("model"),
            Some(&json!("claude-sonnet-4-5"))
        );
        assert_eq!(
            provider_settings.get("reasoningModel"),
            Some(&json!("claude-3-7-thinking"))
        );
        assert!(common_settings.get("model").is_none());
        assert!(common_settings.get("reasoningModel").is_none());
        assert_eq!(
            common_settings.get("skipWebFetchPreflight"),
            Some(&json!(true))
        );
    }
}
