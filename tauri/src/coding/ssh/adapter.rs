use super::super::db_id;
use super::types::{
    default_directory_excludes, normalize_directory_excludes, SSHConnection, SSHFileMapping,
    SSHSyncConfig,
};
use chrono::Local;
use serde_json::{json, Value};

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
        // sync_mcp and sync_skills are always true (no UI to toggle them)
        sync_mcp: true,
        sync_skills: true,
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
        module_statuses: vec![],
    }
}

/// Convert SSHSyncConfig to database Value
pub fn config_to_db_value(config: &SSHSyncConfig) -> Value {
    json!({
        "enabled": config.enabled,
        "active_connection_id": config.active_connection_id,
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
        port: value.get("port").and_then(|v| v.as_u64()).unwrap_or(22) as u16,
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

fn directory_excludes_from_db_value(value: &Value, is_directory: bool) -> Vec<String> {
    if !is_directory {
        return vec![];
    }

    let Some(raw_excludes) = value
        .get("directory_excludes")
        .or_else(|| value.get("directoryExcludes"))
    else {
        return default_directory_excludes();
    };

    let excludes = raw_excludes
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    normalize_directory_excludes(&excludes)
}

/// Convert database Value to SSHFileMapping
pub fn mapping_from_db_value(value: Value) -> SSHFileMapping {
    let id = db_id::db_extract_id(&value);
    let is_directory = value
        .get("is_directory")
        .or_else(|| value.get("isDirectory"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let directory_excludes = directory_excludes_from_db_value(&value, is_directory);

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
        is_directory,
        directory_excludes,
    }
}

/// Convert SSHFileMapping to database Value
pub fn mapping_to_db_value(mapping: &SSHFileMapping) -> Value {
    let directory_excludes = if mapping.is_directory {
        normalize_directory_excludes(&mapping.directory_excludes)
    } else {
        vec![]
    };

    json!({
        "name": mapping.name,
        "module": mapping.module,
        "local_path": mapping.local_path,
        "remote_path": mapping.remote_path,
        "enabled": mapping.enabled,
        "is_pattern": mapping.is_pattern,
        "is_directory": mapping.is_directory,
        "directory_excludes": directory_excludes,
        "updated_at": Local::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::{mapping_from_db_value, mapping_to_db_value};
    use serde_json::json;

    #[test]
    fn directory_mapping_without_saved_excludes_uses_defaults() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:claude-plugins",
            "name": "Claude Code 插件目录",
            "module": "claude",
            "local_path": "~/.claude/plugins",
            "remote_path": "~/.claude/plugins",
            "enabled": true,
            "is_pattern": false,
            "is_directory": true,
        }));

        assert!(mapping.directory_excludes.contains(&"cache".to_string()));
        assert!(mapping.directory_excludes.contains(&".venv".to_string()));
    }

    #[test]
    fn explicit_empty_directory_excludes_are_preserved() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:custom-dir",
            "name": "Custom Dir",
            "module": "claude",
            "local_path": "~/src",
            "remote_path": "~/src",
            "enabled": true,
            "is_pattern": false,
            "is_directory": true,
            "directory_excludes": [],
        }));

        assert!(mapping.directory_excludes.is_empty());
    }

    #[test]
    fn non_directory_mapping_does_not_persist_excludes() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:opencode-plugins",
            "name": "OpenCode 插件文件",
            "module": "opencode",
            "local_path": "~/.config/opencode/*.mjs",
            "remote_path": "~/.config/opencode/",
            "enabled": true,
            "is_pattern": true,
            "is_directory": false,
            "directory_excludes": ["cache"],
        }));

        assert!(mapping.directory_excludes.is_empty());
        assert_eq!(
            mapping_to_db_value(&mapping)["directory_excludes"],
            json!([])
        );
    }
}
