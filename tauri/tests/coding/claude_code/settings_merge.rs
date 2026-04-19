use ai_toolbox_lib::coding::claude_code::settings_merge::{
    extract_provider_settings_for_storage, merge_claude_settings_for_provider, KNOWN_ENV_FIELDS,
    split_settings_into_provider_and_common, strip_claude_common_config_from_settings,
};
use serde_json::json;

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
fn split_settings_into_provider_and_common_maps_api_key_to_auth_token() {
    let settings_value = json!({
        "env": {
            "ANTHROPIC_API_KEY": "legacy-api-key",
            "ANTHROPIC_BASE_URL": "https://example.com"
        }
    });

    let (provider_settings, common_settings) =
        split_settings_into_provider_and_common(&settings_value, &KNOWN_ENV_FIELDS)
            .expect("split should succeed");

    assert_eq!(
        provider_settings.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
        Some(&json!("legacy-api-key"))
    );
    assert!(provider_settings.pointer("/env/ANTHROPIC_API_KEY").is_none());
    assert_eq!(common_settings, json!({}));
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
