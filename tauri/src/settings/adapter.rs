use super::types::{
    default_backup_file_filter_rules, default_sidebar_hidden_by_page, AppSettings,
    BackupCustomEntry, BackupFileFilterRule, S3Config, WebDAVConfig,
};
/**
 * Settings Adapter Layer
 *
 * Provides fault-tolerant conversion between database JSON and Rust types.
 * This layer ensures backward compatibility and eliminates version conflicts.
 */
use serde_json::{json, Value};

/// Convert database JSON Value to AppSettings with fault tolerance
/// Missing fields will use default values, never panics
pub fn from_db_value(value: Value) -> AppSettings {
    AppSettings {
        language: get_str(&value, "language", "zh-CN"),
        current_module: get_str(&value, "current_module", "coding"),
        current_sub_tab: get_str(&value, "current_sub_tab", "opencode"),
        backup_type: get_str(&value, "backup_type", "local"),
        local_backup_path: get_str(&value, "local_backup_path", ""),

        webdav: get_webdav(&value),
        s3: get_s3(&value),

        last_backup_time: get_opt_str(&value, "last_backup_time"),
        backup_image_assets_enabled: get_bool(&value, "backup_image_assets_enabled", true),
        backup_custom_entries: get_backup_custom_entries(&value),
        launch_on_startup: get_bool(&value, "launch_on_startup", true),
        minimize_to_tray_on_close: get_bool(&value, "minimize_to_tray_on_close", true),
        start_minimized: get_bool(&value, "start_minimized", false),
        proxy_mode: get_proxy_mode(&value),
        proxy_url: get_str(&value, "proxy_url", ""),
        theme: get_str(&value, "theme", "system"),
        auto_backup_enabled: get_bool(&value, "auto_backup_enabled", false),
        auto_backup_interval_days: get_u32(&value, "auto_backup_interval_days", 7),
        auto_backup_max_keep: get_u32(&value, "auto_backup_max_keep", 10),
        last_auto_backup_time: get_opt_str(&value, "last_auto_backup_time"),
        auto_check_update: get_bool(&value, "auto_check_update", true),
        visible_tabs: normalize_visible_tabs_order(get_string_array(
            &value,
            "visible_tabs",
            &[
                "opencode",
                "claudecode",
                "codex",
                "grok",
                "geminicli",
                "openclaw",
                "pi",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ],
        )),
        sidebar_hidden_by_page: get_sidebar_hidden_by_page(&value),
        opencode_allow_clear_applied_oh_my_config: get_bool(
            &value,
            "opencode_allow_clear_applied_oh_my_config",
            false,
        ),
        codex_preserve_official_auth_on_switch: get_bool(
            &value,
            "codex_preserve_official_auth_on_switch",
            false,
        ),
        codex_unified_session_history_enabled: get_bool(
            &value,
            "codex_unified_session_history_enabled",
            false,
        ),
        backup_file_filter_rules: get_backup_file_filter_rules(&value),
    }
}

/// Convert AppSettings to database JSON Value
pub fn to_db_value(settings: &AppSettings) -> Value {
    let mut normalized_settings = settings.clone();
    normalized_settings.backup_custom_entries = settings
        .backup_custom_entries
        .iter()
        .map(crate::settings::backup::utils::normalize_backup_custom_entry)
        .collect();

    // Use serde to serialize the entire structure
    // This ensures all types are properly converted
    serde_json::to_value(&normalized_settings).unwrap_or_else(|e| {
        eprintln!("Failed to serialize settings: {}", e);
        json!({})
    })
}

// Helper functions for safe field extraction

fn get_str(value: &Value, key: &str, default: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

fn get_opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn get_bool(value: &Value, key: &str, default: bool) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn get_proxy_mode(value: &Value) -> String {
    value
        .get("proxy_mode")
        .and_then(|v| v.as_str())
        .filter(|mode| matches!(*mode, "direct" | "custom" | "system"))
        .unwrap_or("system")
        .to_string()
}

fn get_u32(value: &Value, key: &str, default: u32) -> u32 {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(default)
}

fn get_string_array(value: &Value, key: &str, defaults: &[&str]) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| defaults.iter().map(|s| s.to_string()).collect())
}

fn normalize_visible_tabs_order(tabs: Vec<String>) -> Vec<String> {
    const PRE_GROK_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "codex",
        "geminicli",
        "openclaw",
        "pi",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];
    const PRE_GEMINI_REORDER_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "codex",
        "openclaw",
        "geminicli",
        "image",
        "ssh",
        "wsl",
    ];
    const PRE_GATEWAY_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "codex",
        "geminicli",
        "openclaw",
        "image",
        "ssh",
        "wsl",
    ];
    const PRE_PI_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "codex",
        "geminicli",
        "openclaw",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];
    const CURRENT_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "codex",
        "grok",
        "geminicli",
        "openclaw",
        "pi",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];

    if string_vec_matches(&tabs, PRE_GEMINI_REORDER_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_GATEWAY_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_PI_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_GROK_DEFAULT_VISIBLE_TABS)
    {
        return CURRENT_DEFAULT_VISIBLE_TABS
            .iter()
            .map(|tab| (*tab).to_string())
            .collect();
    }

    tabs
}

fn string_vec_matches(values: &[String], expected: &[&str]) -> bool {
    values.len() == expected.len()
        && values
            .iter()
            .zip(expected.iter())
            .all(|(value, expected)| value == expected)
}

fn get_backup_custom_entries(value: &Value) -> Vec<BackupCustomEntry> {
    value
        .get("backup_custom_entries")
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    serde_json::from_value::<BackupCustomEntry>(entry.clone())
                        .ok()
                        .map(|entry| {
                            crate::settings::backup::utils::normalize_backup_custom_entry(&entry)
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn get_webdav(value: &Value) -> WebDAVConfig {
    let webdav = value.get("webdav");

    if let Some(webdav) = webdav {
        WebDAVConfig {
            url: get_str(webdav, "url", ""),
            username: get_str(webdav, "username", ""),
            password: get_str(webdav, "password", ""),
            remote_path: get_str(webdav, "remote_path", ""),
            host_label: get_str(webdav, "host_label", ""),
        }
    } else {
        WebDAVConfig::default()
    }
}

fn get_s3(value: &Value) -> S3Config {
    let s3 = value.get("s3");

    if let Some(s3) = s3 {
        S3Config {
            access_key: get_str(s3, "access_key", ""),
            secret_key: get_str(s3, "secret_key", ""),
            bucket: get_str(s3, "bucket", ""),
            region: get_str(s3, "region", ""),
            prefix: get_str(s3, "prefix", ""),
            endpoint_url: get_str(s3, "endpoint_url", ""),
            force_path_style: s3
                .get("force_path_style")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            public_domain: get_str(s3, "public_domain", ""),
        }
    } else {
        S3Config::default()
    }
}

fn get_sidebar_hidden_by_page(value: &Value) -> std::collections::HashMap<String, bool> {
    let mut sidebar_hidden = default_sidebar_hidden_by_page();

    if let Some(sidebar_value) = value
        .get("sidebar_hidden_by_page")
        .and_then(|v| v.as_object())
    {
        for page_key in [
            "opencode",
            "claudecode",
            "codex",
            "grok",
            "openclaw",
            "geminicli",
            "pi",
        ] {
            let Some(page_value) = sidebar_value.get(page_key).and_then(|v| v.as_bool()) else {
                continue;
            };
            sidebar_hidden.insert(page_key.to_string(), page_value);
        }
        return sidebar_hidden;
    }

    let Some(legacy_sidebar_value) = value
        .get("sidebar_visibility_by_page")
        .and_then(|v| v.as_object())
    else {
        return sidebar_hidden;
    };

    for page_key in [
        "opencode",
        "claudecode",
        "codex",
        "grok",
        "openclaw",
        "geminicli",
        "pi",
    ] {
        let Some(page_value) = legacy_sidebar_value.get(page_key) else {
            continue;
        };

        let hidden = if let Some(boolean_value) = page_value.as_bool() {
            boolean_value
        } else {
            page_value
                .get("hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };

        sidebar_hidden.insert(page_key.to_string(), hidden);
    }

    sidebar_hidden
}

fn get_backup_file_filter_rules(value: &Value) -> Vec<BackupFileFilterRule> {
    value
        .get("backup_file_filter_rules")
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    serde_json::from_value::<BackupFileFilterRule>(entry.clone()).ok()
                })
                .collect()
        })
        .unwrap_or_else(default_backup_file_filter_rules)
}

#[cfg(test)]
mod tests {
    use super::from_db_value;
    use serde_json::json;

    #[test]
    fn backup_image_assets_enabled_defaults_to_true() {
        let settings = from_db_value(json!({}));

        assert!(settings.backup_image_assets_enabled);
    }

    #[test]
    fn backup_image_assets_enabled_preserves_explicit_false() {
        let settings = from_db_value(json!({
            "backup_image_assets_enabled": false,
        }));

        assert!(!settings.backup_image_assets_enabled);
    }

    #[test]
    fn backup_custom_entries_default_to_empty() {
        let settings = from_db_value(json!({}));

        assert!(settings.backup_custom_entries.is_empty());
    }

    #[test]
    fn backup_custom_entries_are_loaded_and_normalized() {
        let settings = from_db_value(json!({
            "backup_custom_entries": [
                {
                    "id": "entry-1",
                    "name": "OpenCode custom",
                    "source_path": "~\\.config\\opencode\\custom.json",
                    "restore_path": "",
                    "entry_type": "file",
                    "enabled": true
                }
            ],
        }));

        assert_eq!(settings.backup_custom_entries.len(), 1);
        assert_eq!(
            settings.backup_custom_entries[0].source_path,
            "~/.config/opencode/custom.json"
        );
        assert_eq!(settings.backup_custom_entries[0].restore_path, None);
    }

    #[test]
    fn visible_tabs_default_places_gateway_before_image() {
        let settings = from_db_value(json!({}));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "codex",
                "grok",
                "geminicli",
                "openclaw",
                "pi",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_pre_gemini_reorder_default_is_migrated() {
        let settings = from_db_value(json!({
            "visible_tabs": [
                "opencode",
                "claudecode",
                "codex",
                "openclaw",
                "geminicli",
                "image",
                "ssh",
                "wsl"
            ],
        }));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "codex",
                "grok",
                "geminicli",
                "openclaw",
                "pi",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_pre_gateway_default_is_migrated() {
        let settings = from_db_value(json!({
            "visible_tabs": [
                "opencode",
                "claudecode",
                "codex",
                "geminicli",
                "openclaw",
                "image",
                "ssh",
                "wsl"
            ],
        }));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "codex",
                "grok",
                "geminicli",
                "openclaw",
                "pi",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_custom_order_is_preserved() {
        let settings = from_db_value(json!({
            "visible_tabs": [
                "codex",
                "opencode",
                "geminicli",
                "claudecode",
                "openclaw",
                "image"
            ],
        }));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "codex",
                "opencode",
                "geminicli",
                "claudecode",
                "openclaw",
                "image",
            ]
        );
    }

    #[test]
    fn visible_tabs_pre_grok_default_is_migrated() {
        let settings = from_db_value(json!({
            "visible_tabs": [
                "opencode",
                "claudecode",
                "codex",
                "geminicli",
                "openclaw",
                "pi",
                "gateway",
                "image",
                "ssh",
                "wsl"
            ],
        }));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "codex",
                "grok",
                "geminicli",
                "openclaw",
                "pi",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn opencode_allow_clear_applied_oh_my_config_defaults_to_false() {
        let settings = from_db_value(json!({}));

        assert!(!settings.opencode_allow_clear_applied_oh_my_config);
    }

    #[test]
    fn opencode_allow_clear_applied_oh_my_config_preserves_explicit_true() {
        let settings = from_db_value(json!({
            "opencode_allow_clear_applied_oh_my_config": true,
        }));

        assert!(settings.opencode_allow_clear_applied_oh_my_config);
    }

    #[test]
    fn codex_preserve_official_auth_on_switch_defaults_to_false() {
        let settings = from_db_value(json!({}));

        assert!(!settings.codex_preserve_official_auth_on_switch);
    }

    #[test]
    fn codex_preserve_official_auth_on_switch_preserves_explicit_true() {
        let settings = from_db_value(json!({
            "codex_preserve_official_auth_on_switch": true,
        }));

        assert!(settings.codex_preserve_official_auth_on_switch);
    }

    #[test]
    fn codex_unified_session_history_enabled_defaults_to_false() {
        let settings = from_db_value(json!({}));

        assert!(!settings.codex_unified_session_history_enabled);
    }

    #[test]
    fn codex_unified_session_history_enabled_preserves_explicit_true() {
        let settings = from_db_value(json!({
            "codex_unified_session_history_enabled": true,
        }));

        assert!(settings.codex_unified_session_history_enabled);
    }

    #[test]
    fn backup_file_filter_rules_defaults_to_empty() {
        let settings = from_db_value(json!({}));

        assert!(settings.backup_file_filter_rules.is_empty());
    }

    #[test]
    fn backup_file_filter_rules_preserves_custom_rules() {
        let settings = from_db_value(json!({
            "backup_file_filter_rules": [
                { "tool": "opencode", "file_path": "~/.local/share/opencode/auth.json" },
                { "tool": "claude", "file_path": "~/.claude/settings.json" }
            ],
        }));

        assert_eq!(settings.backup_file_filter_rules.len(), 2);
        assert_eq!(
            settings.backup_file_filter_rules[0].file_path,
            "~/.local/share/opencode/auth.json"
        );
        assert_eq!(settings.backup_file_filter_rules[1].tool, "claude");
        assert_eq!(
            settings.backup_file_filter_rules[1].file_path,
            "~/.claude/settings.json"
        );
    }

    #[test]
    fn backup_file_filter_rules_empty_array_disables_all() {
        let settings = from_db_value(json!({
            "backup_file_filter_rules": [],
        }));

        assert!(settings.backup_file_filter_rules.is_empty());
    }

    #[test]
    fn backup_file_filter_rules_invalid_entries_are_ignored() {
        let settings = from_db_value(json!({
            "backup_file_filter_rules": [
                { "invalid": "data" },
                { "another": "invalid" }
            ],
        }));

        assert!(settings.backup_file_filter_rules.is_empty());
    }

    #[test]
    fn backup_file_filter_rules_mixed_valid_invalid_entries() {
        let settings = from_db_value(json!({
            "backup_file_filter_rules": [
                { "tool": "opencode", "file_path": "~/.local/share/opencode/auth.json" },
                { "invalid": "data" }
            ],
        }));

        // Should keep only valid entries
        assert_eq!(settings.backup_file_filter_rules.len(), 1);
        assert_eq!(
            settings.backup_file_filter_rules[0].file_path,
            "~/.local/share/opencode/auth.json"
        );
    }
}
