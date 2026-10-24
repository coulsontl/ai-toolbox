use super::types::{
    default_backup_file_filter_rules, default_sidebar_hidden_by_page, AppSettings,
    BackupCustomEntry, BackupEncryptionConfig, BackupFileFilterRule, GatewayUsageRangeSelection,
    S3Config, SessionContentFilter, SessionDetailFilters, SessionRoleFilter, WebDAVConfig,
};
/**
 * Settings Adapter Layer
 *
 * Provides fault-tolerant conversion between database JSON and Rust types.
 * This layer ensures backward compatibility and eliminates version conflicts.
 */
use serde_json::{json, Value};
use std::collections::HashMap;

/// Convert database JSON Value to AppSettings with fault tolerance
/// Missing fields will use default values, never panics
pub fn from_db_value(value: Value) -> AppSettings {
    AppSettings {
        // Empty means "never chosen" — the frontend resolves it against the OS
        // locale and the settings dropdown renders it as "System Default". It has
        // to stay empty rather than name a language: a row created without this key
        // is a real path, because the backup form creates the settings row as `{}`
        // and patches only the fields it owns (repository_settings.rs:386). A
        // concrete default here would read back as an explicit choice, pinning the
        // window and the tray to Chinese on every machine and leaving the dropdown
        // on 简体中文 with no way back to following the system.
        language: get_str(&value, "language", ""),
        current_module: get_str(&value, "current_module", "coding"),
        current_sub_tab: get_str(&value, "current_sub_tab", "opencode"),
        backup_type: get_str(&value, "backup_type", "local"),
        local_backup_path: get_str(&value, "local_backup_path", ""),

        webdav: get_webdav(&value),
        s3: get_s3(&value),
        backup_encryption: get_backup_encryption(&value),

        last_backup_time: get_opt_str(&value, "last_backup_time"),
        backup_image_assets_enabled: get_bool(&value, "backup_image_assets_enabled", true),
        backup_cli_config_files_enabled: get_bool(&value, "backup_cli_config_files_enabled", true),
        backup_custom_entries: get_backup_custom_entries(&value),
        launch_on_startup: get_bool(&value, "launch_on_startup", true),
        minimize_to_tray_on_close: get_bool(&value, "minimize_to_tray_on_close", true),
        start_minimized: get_bool(&value, "start_minimized", false),
        start_lightweight: get_bool(&value, "start_lightweight", false),
        lightweight_on_close: get_bool(&value, "lightweight_on_close", false),
        keep_computer_awake: get_bool(&value, "keep_computer_awake", false),
        proxy_mode: get_proxy_mode(&value),
        proxy_url: get_str(&value, "proxy_url", ""),
        theme: get_str(&value, "theme", "system"),
        auto_backup_enabled: get_bool(&value, "auto_backup_enabled", false),
        auto_backup_interval_days: get_u32(&value, "auto_backup_interval_days", 7),
        auto_backup_max_keep: get_u32(&value, "auto_backup_max_keep", 10),
        last_auto_backup_time: get_opt_str(&value, "last_auto_backup_time"),
        auto_check_update: get_bool(&value, "auto_check_update", true),
        receive_beta_updates: get_bool(&value, "receive_beta_updates", false),
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
        opencode_use_legacy_oh_my_config: get_bool(
            &value,
            "opencode_use_legacy_oh_my_config",
            false,
        ),
        opencode_omo_upgrade_confirmed: get_bool(&value, "opencode_omo_upgrade_confirmed", false),
        opencode_dual_write_reasoning_variant: get_bool(
            &value,
            "opencode_dual_write_reasoning_variant",
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
        claude_cli_launch_full_access: get_bool(&value, "claude_cli_launch_full_access", false),
        preferred_terminal: get_opt_str(&value, "preferred_terminal"),
        backup_file_filter_rules: get_backup_file_filter_rules(&value),
        cli_manual_paths: get_string_map(&value, "cli_manual_paths"),
        session_detail_filters: get_session_detail_filters(&value),
        provider_sort_modes: get_string_map(&value, "provider_sort_modes"),
        provider_last_used: get_string_map(&value, "provider_last_used"),
        provider_view_modes: get_string_map(&value, "provider_view_modes"),
        gateway_usage_range: get_gateway_usage_range(&value),
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

/// Read the persisted gateway statistics range. A row written before this field
/// existed reads back as the statistics page's own default (`today`) instead of
/// an empty preset, so the tray never has to guess what "no selection" means.
fn get_gateway_usage_range(value: &Value) -> GatewayUsageRangeSelection {
    let Some(block) = value.get("gateway_usage_range").and_then(|v| v.as_object()) else {
        return GatewayUsageRangeSelection::default();
    };
    let preset = block
        .get("preset")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|preset| !preset.is_empty())
        .unwrap_or("today");
    GatewayUsageRangeSelection {
        preset: preset.to_string(),
        start_date: block.get("start_date").and_then(|v| v.as_i64()),
        end_date: block.get("end_date").and_then(|v| v.as_i64()),
    }
}

fn get_string_map(value: &Value, key: &str) -> HashMap<String, String> {
    value
        .get(key)
        .and_then(|v| v.as_object())
        .map(|object| {
            object
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

// Extracts the session-detail filter toggles from the stored settings object.
// If the whole `session_detail_filters` key is absent (upgrading from a build
// that never persisted them) returns None so the workbench falls back to
// defaults. If the key exists but only part of the nested booleans is present,
// missing keys fall back to their per-type default (all visible) instead of
// dropping the whole block.
fn get_session_detail_filters(value: &Value) -> Option<SessionDetailFilters> {
    let block = value.get("session_detail_filters")?.as_object()?;

    let role_block = block
        .get("role_filter")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let role_filter = SessionRoleFilter {
        user: role_block
            .get("user")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        assistant: role_block
            .get("assistant")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
    };

    let content_block = block
        .get("content_filter")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let content_filter = SessionContentFilter {
        text: content_block
            .get("text")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        thinking: content_block
            .get("thinking")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        tool_call: content_block
            .get("tool_call")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        command: content_block
            .get("command")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
    };

    Some(SessionDetailFilters {
        role_filter,
        content_filter,
    })
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
    const PRE_OMP_DEFAULT_VISIBLE_TABS: &[&str] = &[
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
    // Previous default before Claude Desktop (claudedesktop) and Hermes were added.
    const PRE_DESKTOP_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "codex",
        "grok",
        "geminicli",
        "openclaw",
        "pi",
        "oh_my_pi",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];
    // Previous default before Kimi was added.
    const PRE_KIMI_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "claudedesktop",
        "codex",
        "grok",
        "geminicli",
        "openclaw",
        "pi",
        "oh_my_pi",
        "hermes",
        "dsh",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];
    // Previous default before Antigravity was added.
    const PRE_ANTIGRAVITY_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "claudedesktop",
        "codex",
        "grok",
        "geminicli",
        "kimi",
        "openclaw",
        "pi",
        "oh_my_pi",
        "hermes",
        "dsh",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];
    const CURRENT_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "claudedesktop",
        "codex",
        "grok",
        "geminicli",
        "antigravity",
        "zcode",
        "kimi",
        "openclaw",
        "pi",
        "oh_my_pi",
        "omo_native",
        "hermes",
        "dsh",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];
    // Snapshot of the default list immediately before ZCode was added. Users
    // still on this exact default get the new tab through the full replace
    // below; users with a custom order keep their own list untouched.
    const PRE_ZCODE_DEFAULT_VISIBLE_TABS: &[&str] = &[
        "opencode",
        "claudecode",
        "claudedesktop",
        "codex",
        "grok",
        "geminicli",
        "antigravity",
        "kimi",
        "openclaw",
        "pi",
        "oh_my_pi",
        "omo_native",
        "hermes",
        "dsh",
        "gateway",
        "image",
        "ssh",
        "wsl",
    ];

    if string_vec_matches(&tabs, PRE_GEMINI_REORDER_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_GATEWAY_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_PI_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_GROK_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_OMP_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_DESKTOP_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_KIMI_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_ANTIGRAVITY_DEFAULT_VISIBLE_TABS)
        || string_vec_matches(&tabs, PRE_ZCODE_DEFAULT_VISIBLE_TABS)
    {
        return CURRENT_DEFAULT_VISIBLE_TABS
            .iter()
            .map(|tab| (*tab).to_string())
            .collect();
    }

    // Custom order (does not match any historical baseline): respect the user's
    // tabs as-is, including any tabs they chose to hide. We intentionally do NOT
    // force-insert newly added default tabs here — that would re-add tabs the
    // user explicitly turned off in settings on every reload. Newly added tabs
    // surface via full-replace above only for users still on a historical default.
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

/// Backup encryption defaults to disabled for both missing and malformed records.
/// The credential reference is informational only and never carries the password.
fn get_backup_encryption(value: &Value) -> BackupEncryptionConfig {
    let default = BackupEncryptionConfig::default();
    match value.get("backup_encryption") {
        Some(encryption) => BackupEncryptionConfig {
            enabled: encryption
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            credential_ref: {
                let reference = get_str(encryption, "credential_ref", "");
                if reference.is_empty() {
                    default.credential_ref.clone()
                } else {
                    reference
                }
            },
        },
        None => default,
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
    // Start from the default map so every known page key is present, then overlay
    // whatever the DB actually stored. We intentionally do NOT filter by a
    // hardcoded page-key allowlist: doing so silently dropped newer pages
    // (claudedesktop/hermes/dsh/oh_my_pi) on every read, which made the
    // "hide sidebar" toggle reset after restart. Any boolean value stored in
    // the map is authoritative regardless of whether the key is in the
    // default set, so future pages persist without backend edits.
    let mut sidebar_hidden = default_sidebar_hidden_by_page();

    if let Some(sidebar_value) = value
        .get("sidebar_hidden_by_page")
        .and_then(|v| v.as_object())
    {
        for (page_key, page_value) in sidebar_value {
            if let Some(hidden) = page_value.as_bool() {
                sidebar_hidden.insert(page_key.clone(), hidden);
            }
        }
        return sidebar_hidden;
    }

    // Legacy shape: sidebar_visibility_by_page accepted either a bare boolean
    // or `{ "hidden": bool }`. Preserve that compatibility while still
    // iterating every key instead of an allowlist.
    let Some(legacy_sidebar_value) = value
        .get("sidebar_visibility_by_page")
        .and_then(|v| v.as_object())
    else {
        return sidebar_hidden;
    };

    for (page_key, page_value) in legacy_sidebar_value {
        let hidden = if let Some(boolean_value) = page_value.as_bool() {
            boolean_value
        } else {
            page_value
                .get("hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };

        sidebar_hidden.insert(page_key.clone(), hidden);
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
    fn a_row_without_a_language_reads_back_as_unset_rather_than_as_chinese() {
        // The backup form creates the settings row as `{}` and patches only the
        // fields it owns, so this is a real record shape, not a hypothetical.
        // Reading "zh-CN" here would make the frontend treat it as an explicit
        // choice and drop "System Default" from the dropdown for good.
        let settings = from_db_value(json!({}));

        assert_eq!(settings.language, "");
    }

    #[test]
    fn an_explicit_language_is_preserved() {
        let settings = from_db_value(json!({ "language": "en-US" }));

        assert_eq!(settings.language, "en-US");
    }

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
    fn backup_cli_config_files_enabled_defaults_to_true() {
        let settings = from_db_value(json!({}));

        assert!(settings.backup_cli_config_files_enabled);
    }

    #[test]
    fn backup_cli_config_files_enabled_preserves_explicit_false() {
        let settings = from_db_value(json!({
            "backup_cli_config_files_enabled": false,
        }));

        assert!(!settings.backup_cli_config_files_enabled);
    }

    #[test]
    fn backup_custom_entries_default_to_empty() {
        let settings = from_db_value(json!({}));

        assert!(settings.backup_custom_entries.is_empty());
    }

    #[test]
    fn backup_custom_entry_with_null_exclude_patterns_survives() {
        // An explicit null used to fail the whole entry's deserialization and
        // the filter_map silently dropped the user's custom backup entry.
        let settings = from_db_value(json!({
            "backup_custom_entries": [
                {
                    "id": "entry-1",
                    "name": "Hand edited",
                    "source_path": "~/somewhere",
                    "restore_path": "",
                    "entry_type": "directory",
                    "enabled": true,
                    "exclude_patterns": null
                }
            ],
        }));

        assert_eq!(settings.backup_custom_entries.len(), 1);
        assert!(settings.backup_custom_entries[0]
            .exclude_patterns
            .is_empty());
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
                },
                {
                    "id": "entry-2",
                    "name": "Memories",
                    "source_path": "~/.agents/memories",
                    "restore_path": "",
                    "entry_type": "directory",
                    "enabled": true,
                    "exclude_patterns": ["  ^local$ ", "", "(^|/)\\.git$"]
                }
            ],
        }));

        assert_eq!(settings.backup_custom_entries.len(), 2);
        assert_eq!(
            settings.backup_custom_entries[0].source_path,
            "~/.config/opencode/custom.json"
        );
        assert_eq!(settings.backup_custom_entries[0].restore_path, None);
        // Legacy records without exclude_patterns load with no patterns.
        assert!(settings.backup_custom_entries[0]
            .exclude_patterns
            .is_empty());
        assert_eq!(
            settings.backup_custom_entries[1].exclude_patterns,
            vec!["^local$".to_string(), "(^|/)\\.git$".to_string()]
        );
    }

    #[test]
    fn visible_tabs_default_places_gateway_before_image() {
        let settings = from_db_value(json!({}));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_pre_antigravity_default_is_migrated() {
        let settings = from_db_value(json!({
            // The historical default that predates Antigravity, not the current
            // baseline: the migration must recognize it and append the new tab.
            "visible_tabs": [
                "opencode",
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        }));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_pre_zcode_default_is_migrated() {
        let settings = from_db_value(json!({
            // The historical default that predates ZCode, not the current
            // baseline: the migration must recognize it and append the new tab.
            "visible_tabs": [
                "opencode",
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        }));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn custom_visible_tabs_keep_zcode_hidden() {
        // A user who reordered or hid tabs keeps their own list; the new tab is
        // deliberately not force-inserted for custom orders.
        let settings = from_db_value(json!({
            "visible_tabs": ["codex", "opencode", "gateway"],
        }));

        assert_eq!(settings.visible_tabs, vec!["codex", "opencode", "gateway"]);
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
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
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
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_custom_order_is_preserved() {
        // A custom order that does not match any historical baseline must be
        // returned as-is. We must not force-insert newly added default tabs,
        // because that would re-add tabs the user explicitly hid on reload.
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
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_pre_kimi_default_is_migrated() {
        let settings = from_db_value(json!({
            "visible_tabs": [
                "opencode",
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "openclaw",
                "pi",
                "oh_my_pi",
                "hermes",
                "dsh",
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
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
                "gateway",
                "image",
                "ssh",
                "wsl",
            ]
        );
    }

    #[test]
    fn visible_tabs_pre_omp_default_is_migrated() {
        let settings = from_db_value(json!({
            "visible_tabs": [
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
                "wsl"
            ],
        }));

        assert_eq!(
            settings.visible_tabs,
            vec![
                "opencode",
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "geminicli",
                "antigravity",
                "zcode",
                "kimi",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
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
    fn opencode_use_legacy_oh_my_config_defaults_to_false() {
        let settings = from_db_value(json!({}));

        assert!(!settings.opencode_use_legacy_oh_my_config);
    }

    #[test]
    fn opencode_use_legacy_oh_my_config_preserves_explicit_true() {
        let settings = from_db_value(json!({
            "opencode_use_legacy_oh_my_config": true,
        }));

        assert!(settings.opencode_use_legacy_oh_my_config);
    }

    #[test]
    fn opencode_omo_upgrade_confirmed_defaults_to_false() {
        let settings = from_db_value(json!({}));

        assert!(!settings.opencode_omo_upgrade_confirmed);
    }

    #[test]
    fn opencode_omo_upgrade_confirmed_preserves_explicit_true() {
        let settings = from_db_value(json!({
            "opencode_omo_upgrade_confirmed": true,
        }));

        assert!(settings.opencode_omo_upgrade_confirmed);
    }

    #[test]
    fn opencode_dual_write_reasoning_variant_defaults_to_false() {
        let settings = from_db_value(json!({}));

        assert!(!settings.opencode_dual_write_reasoning_variant);
    }

    #[test]
    fn opencode_dual_write_reasoning_variant_preserves_explicit_true() {
        let settings = from_db_value(json!({
            "opencode_dual_write_reasoning_variant": true,
        }));

        assert!(settings.opencode_dual_write_reasoning_variant);
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

    #[test]
    fn sidebar_hidden_by_page_preserves_all_known_pages() {
        // Regression: the reader used to only honor a 7-key allowlist, so
        // claudedesktop/hermes/dsh were dropped and oh_my_pi was pinned to its
        // default on every read. That made the "hide sidebar" toggle reset
        // after restart. Every known page must round-trip its stored value.
        let settings = from_db_value(json!({
            "sidebar_hidden_by_page": {
                "opencode": true,
                "claudecode": false,
                "claudedesktop": true,
                "codex": false,
                "grok": true,
                "geminicli": false,
                "kimi": true,
                "openclaw": true,
                "pi": false,
                "oh_my_pi": true,
                "omo_native": false,
                "hermes": true,
                "dsh": true
            }
        }));

        assert!(settings.sidebar_hidden_by_page.get("opencode").copied() == Some(true));
        assert!(settings.sidebar_hidden_by_page.get("claudecode").copied() == Some(false));
        assert!(
            settings
                .sidebar_hidden_by_page
                .get("claudedesktop")
                .copied()
                == Some(true)
        );
        assert!(settings.sidebar_hidden_by_page.get("codex").copied() == Some(false));
        assert!(settings.sidebar_hidden_by_page.get("grok").copied() == Some(true));
        assert!(settings.sidebar_hidden_by_page.get("geminicli").copied() == Some(false));
        assert!(settings.sidebar_hidden_by_page.get("kimi").copied() == Some(true));
        assert!(settings.sidebar_hidden_by_page.get("openclaw").copied() == Some(true));
        assert!(settings.sidebar_hidden_by_page.get("pi").copied() == Some(false));
        assert!(settings.sidebar_hidden_by_page.get("oh_my_pi").copied() == Some(true));
        assert!(settings.sidebar_hidden_by_page.get("omo_native").copied() == Some(false));
        assert!(settings.sidebar_hidden_by_page.get("hermes").copied() == Some(true));
        assert!(settings.sidebar_hidden_by_page.get("dsh").copied() == Some(true));
    }

    #[test]
    fn sidebar_hidden_by_page_unknown_key_is_preserved() {
        // A page key the backend default does not know about must still
        // survive the read, so forward-compatibility (a frontend adding a new
        // tab before the backend default list catches up) holds.
        let settings = from_db_value(json!({
            "sidebar_hidden_by_page": {
                "future_tool": true
            }
        }));

        assert!(settings.sidebar_hidden_by_page.get("future_tool").copied() == Some(true));
    }

    #[test]
    fn sidebar_hidden_by_page_legacy_visibility_shape_is_migrated() {
        // Legacy sidebar_visibility_by_page values may be bare booleans or
        // { "hidden": bool } objects. Both must still be honored for every
        // key present, not just an allowlist subset.
        let settings = from_db_value(json!({
            "sidebar_visibility_by_page": {
                "claudedesktop": { "hidden": true },
                "dsh": true
            }
        }));

        assert!(
            settings
                .sidebar_hidden_by_page
                .get("claudedesktop")
                .copied()
                == Some(true)
        );
        assert!(settings.sidebar_hidden_by_page.get("dsh").copied() == Some(true));
    }

    #[test]
    fn sidebar_hidden_by_page_defaults_to_all_visible() {
        let settings = from_db_value(json!({}));

        for page_key in [
            "opencode",
            "claudecode",
            "claudedesktop",
            "codex",
            "grok",
            "geminicli",
            "kimi",
            "openclaw",
            "pi",
            "oh_my_pi",
            "omo_native",
            "hermes",
            "dsh",
        ] {
            assert!(
                settings.sidebar_hidden_by_page.get(page_key).copied() == Some(false),
                "page {page_key} should default to visible",
            );
        }
    }
}
