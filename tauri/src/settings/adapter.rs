/**
 * Settings Adapter Layer
 * 
 * Provides fault-tolerant conversion between database JSON and Rust types.
 * This layer ensures backward compatibility and eliminates version conflicts.
 */

use serde_json::{json, Value};
use super::types::{AppSettings, WebDAVConfig, S3Config};

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
        launch_on_startup: get_bool(&value, "launch_on_startup", true),
        minimize_to_tray_on_close: get_bool(&value, "minimize_to_tray_on_close", true),
        proxy_url: get_str(&value, "proxy_url", ""),
        theme: get_str(&value, "theme", "system"),
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
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn get_bool(value: &Value, key: &str, default: bool) -> bool {
    value
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

fn get_webdav(value: &Value) -> WebDAVConfig {
    let webdav = value.get("webdav");
    
    if let Some(webdav) = webdav {
        WebDAVConfig {
            url: get_str(webdav, "url", ""),
            username: get_str(webdav, "username", ""),
            password: get_str(webdav, "password", ""),
            remote_path: get_str(webdav, "remote_path", ""),
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

