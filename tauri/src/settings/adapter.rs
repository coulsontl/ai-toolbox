use super::types::{default_sidebar_hidden_by_page, AppSettings, S3Config, WebDAVConfig};
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
        visible_tabs: get_string_array(
            &value,
            "visible_tabs",
            &[
                "opencode",
                "claudecode",
                "codex",
                "openclaw",
                "image",
                "ssh",
                "wsl",
            ],
        ),
        sidebar_hidden_by_page: get_sidebar_hidden_by_page(&value),
    }
}

/// Convert AppSettings to database JSON Value
pub fn to_db_value(settings: &AppSettings) -> Value {
    // Use serde to serialize the entire structure
    // This ensures all types are properly converted
    serde_json::to_value(settings).unwrap_or_else(|e| {
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
        for page_key in ["opencode", "claudecode", "codex", "openclaw"] {
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

    for page_key in ["opencode", "claudecode", "codex", "openclaw"] {
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
}
