use serde_json::{json, Value};
use super::types::{FileMapping, WSLSyncConfig};
use super::super::db_id;
use chrono::Local;

// ============================================================================
// WSL Sync Config Adapter Functions
// ============================================================================

/// Convert database Value to WSLSyncConfig
pub fn config_from_db_value(value: Value, file_mappings: Vec<FileMapping>) -> WSLSyncConfig {
    WSLSyncConfig {
        enabled: value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        distro: value
            .get("distro")
            .and_then(|v| v.as_str())
            .unwrap_or("Ubuntu")
            .to_string(),
        // sync_mcp and sync_skills are always true (no UI to toggle them)
        sync_mcp: true,
        sync_skills: true,
        file_mappings,
        last_sync_time: value
            .get("last_sync_time")
            .or_else(|| value.get("lastSyncTime"))
            .and_then(|v| v.as_str())
            .map(String::from),
        last_sync_status: value
            .get("last_sync_status")
            .or_else(|| value.get("lastSyncStatus"))
            .and_then(|v| v.as_str())
            .unwrap_or("never")
            .to_string(),
        last_sync_error: value
            .get("last_sync_error")
            .or_else(|| value.get("lastSyncError"))
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

/// Convert WSLSyncConfig to database Value
pub fn config_to_db_value(config: &WSLSyncConfig) -> Value {
    json!({
        "enabled": config.enabled,
        "distro": config.distro,
        "last_sync_time": config.last_sync_time,
        "last_sync_status": config.last_sync_status,
        "last_sync_error": config.last_sync_error,
    })
}

/// Convert database Value to FileMapping
pub fn mapping_from_db_value(value: Value) -> FileMapping {
    // Use db_extract_id to clean the SurrealDB record ID
    let id = db_id::db_extract_id(&value);

    FileMapping {
        id,
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        module: value
            .get("module")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        windows_path: value
            .get("windows_path")
            .or_else(|| value.get("windowsPath"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        wsl_path: value
            .get("wsl_path")
            .or_else(|| value.get("wslPath"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        enabled: value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        is_pattern: value
            .get("is_pattern")
            .or_else(|| value.get("isPattern"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        is_directory: value
            .get("is_directory")
            .or_else(|| value.get("isDirectory"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// Convert FileMapping to database Value
pub fn mapping_to_db_value(mapping: &FileMapping) -> Value {
    json!({
        "id": mapping.id,
        "name": mapping.name,
        "module": mapping.module,
        "windows_path": mapping.windows_path,
        "wsl_path": mapping.wsl_path,
        "enabled": mapping.enabled,
        "is_pattern": mapping.is_pattern,
        "is_directory": mapping.is_directory,
        "updated_at": Local::now().to_rfc3339(),
    })
}
