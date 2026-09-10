use super::super::db_id;
use super::types::{normalize_directory_excludes, FileMapping, WSLSyncConfig};
use crate::coding::config_cleanup;
use chrono::Local;
use serde_json::{json, Value};

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

/// Convert WSLSyncConfig to database Value
pub fn config_to_db_value(config: &WSLSyncConfig) -> Value {
    json!({
        "enabled": config.enabled,
        "distro": config.distro,
    })
}

fn cleanup_paths_from_db_value(
    value: &Value,
    is_directory: bool,
    is_pattern: bool,
    wsl_path: &str,
    windows_path: &str,
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
        wsl_path,
        windows_path,
        &paths,
    )
    .unwrap_or_default()
}

fn directory_excludes_from_db_value(value: &Value, is_directory: bool) -> Vec<String> {
    if !is_directory {
        return vec![];
    }

    let raw_excludes = value
        .get("directory_excludes")
        .or_else(|| value.get("directoryExcludes"));
    let Some(items) = raw_excludes.and_then(|v| v.as_array()) else {
        return vec![];
    };

    let excludes = items
        .iter()
        .filter_map(|item| item.as_str().map(String::from))
        .collect::<Vec<_>>();
    normalize_directory_excludes(&excludes)
}

/// Convert database Value to FileMapping
pub fn mapping_from_db_value(value: Value) -> FileMapping {
    // Use db_extract_id to clean the SurrealDB record ID
    let id = db_id::db_extract_id(&value);
    let windows_path = value
        .get("windows_path")
        .or_else(|| value.get("windowsPath"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let wsl_path = value
        .get("wsl_path")
        .or_else(|| value.get("wslPath"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let is_pattern = value
        .get("is_pattern")
        .or_else(|| value.get("isPattern"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_directory = value
        .get("is_directory")
        .or_else(|| value.get("isDirectory"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let directory_excludes = directory_excludes_from_db_value(&value, is_directory);
    let cleanup_paths =
        cleanup_paths_from_db_value(&value, is_directory, is_pattern, &wsl_path, &windows_path);

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
        windows_path,
        wsl_path,
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

/// Convert FileMapping to database Value
pub fn mapping_to_db_value(mapping: &FileMapping) -> Value {
    let directory_excludes = if mapping.is_directory {
        normalize_directory_excludes(&mapping.directory_excludes)
    } else {
        vec![]
    };

    json!({
        "id": mapping.id,
        "name": mapping.name,
        "module": mapping.module,
        "windows_path": mapping.windows_path,
        "wsl_path": mapping.wsl_path,
        "enabled": mapping.enabled,
        "is_pattern": mapping.is_pattern,
        "is_directory": mapping.is_directory,
        "directory_excludes": directory_excludes,
        "cleanup_paths": config_cleanup::cleanup_paths_for_mapping(
            mapping.is_directory,
            mapping.is_pattern,
            &mapping.wsl_path,
            &mapping.windows_path,
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
    fn config_from_legacy_record_defaults_last_sync_warnings_to_empty() {
        let config = config_from_db_value(
            json!({
                "enabled": true,
                "last_sync_status": "success",
            }),
            vec![],
        );
        assert!(config.last_sync_warnings.is_empty());
    }

    #[test]
    fn config_from_db_value_reads_persisted_last_sync_warnings() {
        let config = config_from_db_value(
            json!({
                "enabled": true,
                "last_sync_status": "success",
                "last_sync_warnings": ["警告A", "警告B"],
            }),
            vec![],
        );
        assert_eq!(config.last_sync_warnings, vec!["警告A".to_string(), "警告B".to_string()]);
    }

    #[test]
    fn json_or_toml_file_mapping_persists_cleanup_paths() {
        let mapping = mapping_from_db_value(json!({
            "id": "wsl_file_mapping:claude-settings",
            "name": "Claude Code 设置",
            "module": "claude",
            "windows_path": "~/.claude/settings.json",
            "wsl_path": "~/.claude/settings.json",
            "enabled": true,
            "is_pattern": false,
            "is_directory": false,
            "cleanup_paths": [
                " $.env.HTTP_PROXY ",
                "$.env.HTTP_PROXY"
            ],
        }));

        assert_eq!(mapping.cleanup_paths, vec!["$.env.HTTP_PROXY".to_string()]);
        assert_eq!(
            mapping_to_db_value(&mapping)["cleanup_paths"],
            json!(["$.env.HTTP_PROXY"])
        );
    }

    #[test]
    fn unsupported_file_mapping_does_not_persist_cleanup_paths() {
        let mapping = mapping_from_db_value(json!({
            "id": "wsl_file_mapping:geminicli-env",
            "name": "Gemini CLI 环境变量",
            "module": "geminicli",
            "windows_path": "~/.gemini/.env",
            "wsl_path": "~/.gemini/.env",
            "enabled": true,
            "is_pattern": false,
            "is_directory": false,
            "cleanup_paths": ["$.env.HTTP_PROXY"],
        }));

        assert!(mapping.cleanup_paths.is_empty());
        assert_eq!(mapping_to_db_value(&mapping)["cleanup_paths"], json!([]));
    }

    #[test]
    fn directory_excludes_roundtrip_for_directory_mapping() {
        let mapping = mapping_from_db_value(json!({
            "id": "wsl_file_mapping:opencode-agents",
            "name": "OpenCode Agent 配置（agents）",
            "module": "opencode",
            "windows_path": "~/.config/opencode/agents",
            "wsl_path": "~/.config/opencode/agents",
            "enabled": true,
            "is_pattern": false,
            "is_directory": true,
            "directory_excludes": ["cache", ".venv"],
        }));

        assert_eq!(
            mapping.directory_excludes,
            vec!["cache".to_string(), ".venv".to_string()]
        );
        assert_eq!(
            mapping_to_db_value(&mapping)["directory_excludes"],
            json!(["cache", ".venv"])
        );
    }

    #[test]
    fn non_directory_mapping_does_not_persist_excludes() {
        let mapping = mapping_from_db_value(json!({
            "id": "wsl_file_mapping:opencode-plugins",
            "name": "OpenCode 插件文件",
            "module": "opencode",
            "windows_path": "~/.config/opencode/*.mjs",
            "wsl_path": "~/.config/opencode/",
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
