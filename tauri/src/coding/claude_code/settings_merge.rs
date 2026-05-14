use serde_json::{Map, Value};

const PROTECTED_TOP_LEVEL_FIELDS: [&str; 3] = ["enabledPlugins", "extraKnownMarketplaces", "hooks"];

const PROVIDER_MODEL_FIELD_MAPPINGS: [(&str, &str); 5] = [
    ("model", "ANTHROPIC_MODEL"),
    ("haikuModel", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
    ("sonnetModel", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
    ("opusModel", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
    ("reasoningModel", "ANTHROPIC_REASONING_MODEL"),
];

fn is_provider_model_field(field_key: &str) -> bool {
    PROVIDER_MODEL_FIELD_MAPPINGS
        .iter()
        .any(|(provider_field, _)| provider_field == &field_key)
}

pub const KNOWN_ENV_FIELDS: [&str; 8] = [
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_REASONING_MODEL",
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

fn remove_previous_extra_settings(
    target: &mut Map<String, Value>,
    previous_extra: &Map<String, Value>,
) {
    for field_key in previous_extra.keys() {
        if field_key == "env" {
            continue;
        }

        target.remove(field_key);
    }

    let Some(previous_extra_env) = previous_extra.get("env").and_then(value_as_object) else {
        return;
    };

    let Some(target_env) = target.get_mut("env").and_then(Value::as_object_mut) else {
        return;
    };

    for field_key in previous_extra_env.keys() {
        target_env.remove(field_key);
    }

    if target_env.is_empty() {
        target.remove("env");
    }
}

fn sanitize_extra_settings_config(
    extra_settings_config: &Value,
    known_env_fields: &[&str],
) -> Result<Map<String, Value>, String> {
    let extra_settings_object = match extra_settings_config {
        Value::Object(object) => object,
        Value::Null => return Ok(Map::new()),
        _ => return Err("Claude extra settings must be a JSON object".to_string()),
    };

    let mut sanitized_extra_settings = extra_settings_object.clone();
    for protected_field in PROTECTED_TOP_LEVEL_FIELDS {
        sanitized_extra_settings.remove(protected_field);
    }

    for (provider_field, _) in PROVIDER_MODEL_FIELD_MAPPINGS {
        sanitized_extra_settings.remove(provider_field);
    }

    let mut remove_env = false;
    if let Some(env_value) = sanitized_extra_settings.get_mut("env") {
        match env_value {
            Value::Object(env_object) => {
                for known_env_field in known_env_fields {
                    env_object.remove(*known_env_field);
                }
                if env_object.is_empty() {
                    remove_env = true;
                }
            }
            Value::Null => {
                remove_env = true;
            }
            _ => return Err("Claude extra settings env must be a JSON object".to_string()),
        }
    }
    if remove_env {
        sanitized_extra_settings.remove("env");
    }

    Ok(sanitized_extra_settings)
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
    previous_extra_settings_config: Option<&Value>,
    next_extra_settings_config: Option<&Value>,
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
    let previous_extra_settings_object = match previous_extra_settings_config {
        Some(value) => sanitize_extra_settings_config(value, known_env_fields)?,
        None => sanitize_extra_settings_config(
            next_extra_settings_config.unwrap_or(&Value::Object(Map::new())),
            known_env_fields,
        )?,
    };
    let next_extra_settings_object = match next_extra_settings_config {
        Some(value) => sanitize_extra_settings_config(value, known_env_fields)?,
        None => Map::new(),
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

    if !previous_extra_settings_object.is_empty() {
        remove_previous_extra_settings(&mut merged_settings, &previous_extra_settings_object);
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

    for (field_key, field_value) in &next_extra_settings_object {
        if field_key == "env" {
            continue;
        }

        if PROTECTED_TOP_LEVEL_FIELDS.contains(&field_key.as_str()) {
            continue;
        }

        if is_provider_model_field(field_key) {
            continue;
        }

        if let Some(existing_value) = merged_settings.get_mut(field_key) {
            *existing_value = field_value.clone();
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

    if let Some(next_extra_env) = next_extra_settings_object
        .get("env")
        .and_then(value_as_object)
    {
        for (field_key, field_value) in next_extra_env {
            if !known_env_fields.contains(&field_key.as_str()) {
                merged_env.insert(field_key.clone(), field_value.clone());
            }
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
    use super::*;
    use serde_json::json;

    fn merge_with_extra(
        current_disk_settings: Value,
        previous_extra_settings_config: Option<Value>,
        next_extra_settings_config: Option<Value>,
    ) -> Value {
        merge_claude_settings_for_provider(
            Some(&current_disk_settings),
            Some(&json!({
                "statusLine": { "command": "old-common" },
                "env": { "COMMON_ENV": "old" }
            })),
            &json!({
                "statusLine": { "command": "common" },
                "env": { "COMMON_ENV": "common" }
            }),
            previous_extra_settings_config.as_ref(),
            next_extra_settings_config.as_ref(),
            &json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://provider.example.com",
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "model": "provider-model"
            }),
            &KNOWN_ENV_FIELDS,
        )
        .expect("settings merge should succeed")
    }

    #[test]
    fn extra_settings_override_common_but_not_provider_fields() {
        let merged = merge_with_extra(
            json!({
                "untouched": true,
                "statusLine": { "command": "old-common" },
                "env": {
                    "COMMON_ENV": "old",
                    "ANTHROPIC_BASE_URL": "https://old.example.com"
                }
            }),
            Some(json!({})),
            Some(json!({
                "statusLine": { "command": "extra" },
                "env": {
                    "COMMON_ENV": "extra",
                    "ANTHROPIC_BASE_URL": "https://extra.example.com",
                    "ANTHROPIC_AUTH_TOKEN": "extra-key"
                },
                "model": "extra-model"
            })),
        );

        assert_eq!(merged["untouched"], json!(true));
        assert_eq!(merged["statusLine"]["command"], json!("extra"));
        assert_eq!(merged["env"]["COMMON_ENV"], json!("extra"));
        assert_eq!(
            merged["env"]["ANTHROPIC_BASE_URL"],
            json!("https://provider.example.com")
        );
        assert_eq!(merged["env"]["ANTHROPIC_AUTH_TOKEN"], json!("provider-key"));
        assert_eq!(merged["env"]["ANTHROPIC_MODEL"], json!("provider-model"));
        assert!(merged.get("model").is_none());
    }

    #[test]
    fn previous_extra_settings_are_removed_when_next_provider_has_none() {
        let merged = merge_with_extra(
            json!({
                "statusLine": { "command": "old-common" },
                "extraOnly": { "enabled": true },
                "env": {
                    "COMMON_ENV": "old",
                    "EXTRA_ENV": "old-extra",
                    "ANTHROPIC_AUTH_TOKEN": "old-provider"
                }
            }),
            Some(json!({
                "extraOnly": { "enabled": true },
                "env": { "EXTRA_ENV": "old-extra" }
            })),
            Some(json!({})),
        );

        assert!(merged.get("extraOnly").is_none());
        assert!(merged["env"].get("EXTRA_ENV").is_none());
        assert_eq!(merged["statusLine"]["command"], json!("common"));
        assert_eq!(merged["env"]["COMMON_ENV"], json!("common"));
        assert_eq!(merged["env"]["ANTHROPIC_AUTH_TOKEN"], json!("provider-key"));
    }

    #[test]
    fn previous_extra_settings_are_removed_by_managed_key_even_if_disk_value_changed() {
        let merged = merge_with_extra(
            json!({
                "statusLine": { "command": "old-common" },
                "extraOnly": { "enabled": false, "userChanged": true },
                "env": {
                    "COMMON_ENV": "old",
                    "EXTRA_ENV": "manually-changed"
                }
            }),
            Some(json!({
                "extraOnly": { "enabled": true },
                "env": { "EXTRA_ENV": "old-extra" }
            })),
            Some(json!({})),
        );

        assert!(merged.get("extraOnly").is_none());
        assert!(merged["env"].get("EXTRA_ENV").is_none());
        assert_eq!(merged["env"]["COMMON_ENV"], json!("common"));
    }

    #[test]
    fn protected_top_level_fields_are_preserved_from_disk() {
        let merged = merge_with_extra(
            json!({
                "enabledPlugins": ["runtime-plugin"],
                "extraKnownMarketplaces": { "runtime": true },
                "hooks": { "PreToolUse": [] },
                "statusLine": { "command": "old-common" },
                "env": { "COMMON_ENV": "old" }
            }),
            Some(json!({
                "enabledPlugins": ["old-extra"],
                "hooks": { "Stop": [] }
            })),
            Some(json!({
                "enabledPlugins": ["new-extra"],
                "extraKnownMarketplaces": { "extra": true },
                "hooks": { "Stop": [] }
            })),
        );

        assert_eq!(merged["enabledPlugins"], json!(["runtime-plugin"]));
        assert_eq!(merged["extraKnownMarketplaces"], json!({ "runtime": true }));
        assert_eq!(merged["hooks"], json!({ "PreToolUse": [] }));
    }

    #[test]
    fn extra_settings_env_must_be_an_object() {
        let result = merge_claude_settings_for_provider(
            Some(&json!({})),
            None,
            &json!({}),
            None,
            Some(&json!({ "env": "invalid" })),
            &json!({}),
            &KNOWN_ENV_FIELDS,
        );

        assert!(result.is_err());
    }
}
