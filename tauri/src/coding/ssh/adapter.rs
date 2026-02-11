use serde_json::{json, Value};
use super::types::{SSHConnection, SSHFileMapping, SSHSyncConfig};
use super::super::db_id;
use chrono::Local;

// ============================================================================
// SSH Sync Config Adapter Functions
// ============================================================================

/// Convert database Value to SSHSyncConfig
pub fn config_from_db_value(
    value: Value,
    file_mappings: Vec<SSHFileMapping>,
    connections: Vec<SSHConnection>,
) -> SSHSyncConfig {
    SSHSyncConfig {
        enabled: value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        active_connection_id: value
            .get("active_connection_id")
            .or_else(|| value.get("activeConnectionId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        sync_mcp: value
            .get("sync_mcp")
            .or_else(|| value.get("syncMcp"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),   // 默认 true（向后兼容）
        sync_skills: value
            .get("sync_skills")
            .or_else(|| value.get("syncSkills"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),   // 默认 true（向后兼容）
        file_mappings,
        connections,
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

/// Convert SSHSyncConfig to database Value
pub fn config_to_db_value(config: &SSHSyncConfig) -> Value {
    json!({
        "enabled": config.enabled,
        "active_connection_id": config.active_connection_id,
        "sync_mcp": config.sync_mcp,
        "sync_skills": config.sync_skills,
        "last_sync_time": config.last_sync_time,
        "last_sync_status": config.last_sync_status,
        "last_sync_error": config.last_sync_error,
    })
}

// ============================================================================
// SSH Connection Adapter Functions
// ============================================================================

/// Convert database Value to SSHConnection
pub fn connection_from_db_value(value: Value) -> SSHConnection {
    let id = db_id::db_extract_id(&value);

    SSHConnection {
        id,
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        host: value
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        port: value
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(22) as u16,
        username: value
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        auth_method: value
            .get("auth_method")
            .or_else(|| value.get("authMethod"))
            .and_then(|v| v.as_str())
            .unwrap_or("key")
            .to_string(),
        password: value
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        private_key_path: value
            .get("private_key_path")
            .or_else(|| value.get("privateKeyPath"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        private_key_content: value
            .get("private_key_content")
            .or_else(|| value.get("privateKeyContent"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        passphrase: value
            .get("passphrase")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        sort_order: value
            .get("sort_order")
            .or_else(|| value.get("sortOrder"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
    }
}

/// Convert SSHConnection to database Value
pub fn connection_to_db_value(conn: &SSHConnection) -> Value {
    json!({
        "name": conn.name,
        "host": conn.host,
        "port": conn.port,
        "username": conn.username,
        "auth_method": conn.auth_method,
        "password": conn.password,
        "private_key_path": conn.private_key_path,
        "private_key_content": conn.private_key_content,
        "passphrase": conn.passphrase,
        "sort_order": conn.sort_order,
        "updated_at": Local::now().to_rfc3339(),
    })
}

// ============================================================================
// SSH File Mapping Adapter Functions
// ============================================================================

/// Convert database Value to SSHFileMapping
pub fn mapping_from_db_value(value: Value) -> SSHFileMapping {
    let id = db_id::db_extract_id(&value);

    SSHFileMapping {
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
        local_path: value
            .get("local_path")
            .or_else(|| value.get("localPath"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        remote_path: value
            .get("remote_path")
            .or_else(|| value.get("remotePath"))
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

/// Convert SSHFileMapping to database Value
pub fn mapping_to_db_value(mapping: &SSHFileMapping) -> Value {
    json!({
        "name": mapping.name,
        "module": mapping.module,
        "local_path": mapping.local_path,
        "remote_path": mapping.remote_path,
        "enabled": mapping.enabled,
        "is_pattern": mapping.is_pattern,
        "is_directory": mapping.is_directory,
        "updated_at": Local::now().to_rfc3339(),
    })
}
