use super::super::db_id;
use super::types::{
    default_directory_excludes_for_mapping, matches_default_directory_excludes,
    normalize_directory_excludes, SSHConnection, SSHFileMapping, SSHSyncConfig,
};
use crate::coding::config_cleanup;
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
        last_sync_warnings: value
            .get("last_sync_warnings")
            .or_else(|| value.get("lastSyncWarnings"))
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
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

fn directory_excludes_from_db_value(
    value: &Value,
    mapping_id: &str,
    is_directory: bool,
) -> Vec<String> {
    if !is_directory {
        return vec![];
    }

    let default_excludes = default_directory_excludes_for_mapping(mapping_id);
    let Some(raw_excludes) = value
        .get("directory_excludes")
        .or_else(|| value.get("directoryExcludes"))
    else {
        return default_excludes;
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

    let normalized_excludes = normalize_directory_excludes(&excludes);
    if normalized_excludes != default_excludes
        && matches_default_directory_excludes(&normalized_excludes)
    {
        return default_excludes;
    }

    normalized_excludes
}

fn cleanup_paths_from_db_value(
    value: &Value,
    is_directory: bool,
    is_pattern: bool,
    remote_path: &str,
    local_path: &str,
) -> Vec<String> {
    let paths = value
        .get("cleanup_paths")
        .or_else(|| value.get("cleanupPaths"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    config_cleanup::cleanup_paths_for_mapping(
        is_directory,
        is_pattern,
        remote_path,
        local_path,
        &paths,
    )
    .unwrap_or_default()
}

/// Convert database Value to SSHFileMapping
pub fn mapping_from_db_value(value: Value) -> SSHFileMapping {
    let id = db_id::db_extract_id(&value);
    let is_directory = value
        .get("is_directory")
        .or_else(|| value.get("isDirectory"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_pattern = value
        .get("is_pattern")
        .or_else(|| value.get("isPattern"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let local_path = value
        .get("local_path")
        .or_else(|| value.get("localPath"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let remote_path = value
        .get("remote_path")
        .or_else(|| value.get("remotePath"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let directory_excludes = directory_excludes_from_db_value(&value, &id, is_directory);
    let cleanup_paths =
        cleanup_paths_from_db_value(&value, is_directory, is_pattern, &remote_path, &local_path);

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
        local_path,
        remote_path,
        enabled: value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        is_pattern,
        is_directory,
        directory_excludes,
        cleanup_paths,
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
        "cleanup_paths": config_cleanup::cleanup_paths_for_mapping(
            mapping.is_directory,
            mapping.is_pattern,
            &mapping.remote_path,
            &mapping.local_path,
            &mapping.cleanup_paths
        ).unwrap_or_default(),
        "updated_at": Local::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::{config_from_db_value, mapping_from_db_value, mapping_to_db_value};
    use serde_json::json;

    #[test]
    fn ssh_config_from_legacy_record_defaults_last_sync_warnings_to_empty() {
        let config = config_from_db_value(
            json!({
                "enabled": true,
                "active_connection_id": "conn-1",
                "last_sync_status": "success",
            }),
            vec![],
            vec![],
        );
        assert!(config.last_sync_warnings.is_empty());
    }

    #[test]
    fn ssh_config_from_db_value_reads_persisted_last_sync_warnings() {
        let config = config_from_db_value(
            json!({
                "enabled": true,
                "active_connection_id": "conn-1",
                "last_sync_status": "success",
                "last_sync_warnings": ["kept foreign path"],
            }),
            vec![],
            vec![],
        );
        assert_eq!(config.last_sync_warnings, vec!["kept foreign path".to_string()]);
    }

    #[test]
    fn directory_mapping_without_saved_excludes_uses_generic_defaults() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:custom-dir",
            "name": "Custom Dir",
            "module": "claude",
            "local_path": "~/src",
            "remote_path": "~/src",
            "enabled": true,
            "is_pattern": false,
            "is_directory": true,
        }));

        assert!(mapping.directory_excludes.contains(&"cache".to_string()));
        assert!(mapping.directory_excludes.contains(&".venv".to_string()));
    }

    #[test]
    fn claude_plugins_without_saved_excludes_keep_plugin_cache_available() {
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

        assert!(mapping.directory_excludes.contains(&".venv".to_string()));
        assert!(!mapping.directory_excludes.contains(&"cache".to_string()));
    }

    #[test]
    fn claude_plugins_legacy_default_excludes_keep_plugin_cache_available() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:claude-plugins",
            "name": "Claude Code 插件目录",
            "module": "claude",
            "local_path": "~/.claude/plugins",
            "remote_path": "~/.claude/plugins",
            "enabled": true,
            "is_pattern": false,
            "is_directory": true,
            "directory_excludes": [
                ".git",
                ".venv",
                "venv",
                "node_modules",
                "__pycache__",
                ".pytest_cache",
                ".mypy_cache",
                "cache"
            ],
        }));

        assert!(mapping.directory_excludes.contains(&".venv".to_string()));
        assert!(!mapping.directory_excludes.contains(&"cache".to_string()));
    }

    #[test]
    fn claude_plugins_custom_excludes_are_preserved() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:claude-plugins",
            "name": "Claude Code 插件目录",
            "module": "claude",
            "local_path": "~/.claude/plugins",
            "remote_path": "~/.claude/plugins",
            "enabled": true,
            "is_pattern": false,
            "is_directory": true,
            "directory_excludes": ["cache"],
        }));

        assert_eq!(mapping.directory_excludes, vec!["cache".to_string()]);
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

    #[test]
    fn json_or_toml_file_mapping_persists_cleanup_paths() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:codex-config",
            "name": "Codex 配置",
            "module": "codex",
            "local_path": "~/.codex/config.toml",
            "remote_path": "~/.codex/config.toml",
            "enabled": true,
            "is_pattern": false,
            "is_directory": false,
            "cleanup_paths": [
                " $.mcp_servers.demo.env.HTTP_PROXY ",
                "$.mcp_servers.demo.env.HTTP_PROXY"
            ],
        }));

        assert_eq!(
            mapping.cleanup_paths,
            vec!["$.mcp_servers.demo.env.HTTP_PROXY".to_string()]
        );
        assert_eq!(
            mapping_to_db_value(&mapping)["cleanup_paths"],
            json!(["$.mcp_servers.demo.env.HTTP_PROXY"])
        );
    }

    #[test]
    fn unsupported_file_mapping_does_not_persist_cleanup_paths() {
        let mapping = mapping_from_db_value(json!({
            "id": "ssh_file_mapping:geminicli-env",
            "name": "Gemini CLI 环境变量",
            "module": "geminicli",
            "local_path": "~/.gemini/.env",
            "remote_path": "~/.gemini/.env",
            "enabled": true,
            "is_pattern": false,
            "is_directory": false,
            "cleanup_paths": ["$.env.HTTP_PROXY"],
        }));

        assert!(mapping.cleanup_paths.is_empty());
        assert_eq!(mapping_to_db_value(&mapping)["cleanup_paths"], json!([]));
    }
}
