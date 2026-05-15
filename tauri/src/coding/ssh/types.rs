use serde::{Deserialize, Serialize};

// Re-use SyncResult and SyncProgress from wsl module
pub use super::super::wsl::{SyncProgress, SyncResult};
use crate::coding::runtime_location::WslDirectModuleStatus;

pub const DEFAULT_DIRECTORY_EXCLUDES: &[&str] = &[
    ".git",
    ".venv",
    "venv",
    "node_modules",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    "cache",
];

pub fn default_directory_excludes() -> Vec<String> {
    DEFAULT_DIRECTORY_EXCLUDES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

pub fn normalize_directory_excludes(excludes: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();

    for exclude in excludes {
        let name = exclude
            .trim()
            .trim_matches(|c| c == '/' || c == '\\')
            .trim();
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            continue;
        }
        if seen.insert(name.to_string()) {
            normalized.push(name.to_string());
        }
    }

    normalized
}

// ============================================================================
// SSH Connection Types
// ============================================================================

/// SSH connection preset
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SSHConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String, // "key" | "password" | "none"
    pub password: String,
    pub private_key_path: String,
    pub private_key_content: String,
    pub passphrase: String,
    pub sort_order: u32,
}

// ============================================================================
// SSH File Mapping Types
// ============================================================================

/// SSH file mapping (global, shared across all connections)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SSHFileMapping {
    pub id: String,
    pub name: String,
    pub module: String, // "opencode" | "claude" | "codex" | "openclaw" | "geminicli"
    pub local_path: String,
    pub remote_path: String,
    pub enabled: bool,
    pub is_pattern: bool,
    pub is_directory: bool,
    #[serde(default)]
    pub directory_excludes: Vec<String>,
}

// ============================================================================
// SSH Sync Config Types
// ============================================================================

/// SSH sync global configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SSHSyncConfig {
    pub enabled: bool,
    pub active_connection_id: String,
    // sync_mcp and sync_skills are always true (no UI to toggle them)
    pub sync_mcp: bool,
    pub sync_skills: bool,
    pub file_mappings: Vec<SSHFileMapping>,
    pub connections: Vec<SSHConnection>,
    pub last_sync_time: Option<String>,
    pub last_sync_status: String, // "success" | "error" | "never"
    pub last_sync_error: Option<String>,
    #[serde(default)]
    pub module_statuses: Vec<WslDirectModuleStatus>,
}

impl Default for SSHSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            active_connection_id: String::new(),
            sync_mcp: true,
            sync_skills: true,
            file_mappings: vec![],
            connections: vec![],
            last_sync_time: None,
            last_sync_status: "never".to_string(),
            last_sync_error: None,
            module_statuses: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{default_directory_excludes, normalize_directory_excludes};

    #[test]
    fn normalizes_directory_excludes_by_trimming_and_deduplicating() {
        let input = vec![
            " cache ".to_string(),
            "cache/".to_string(),
            "node_modules".to_string(),
            "nested/cache".to_string(),
            "".to_string(),
        ];

        assert_eq!(
            normalize_directory_excludes(&input),
            vec!["cache".to_string(), "node_modules".to_string()]
        );
    }

    #[test]
    fn default_directory_excludes_cover_common_generated_directories() {
        let excludes = default_directory_excludes();
        assert!(excludes.contains(&".git".to_string()));
        assert!(excludes.contains(&".venv".to_string()));
        assert!(excludes.contains(&"node_modules".to_string()));
        assert!(excludes.contains(&"cache".to_string()));
    }
}

// ============================================================================
// SSH Result Types
// ============================================================================

/// SSH connection test result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SSHConnectionResult {
    pub connected: bool,
    pub error: Option<String>,
    pub server_info: Option<String>,
}

/// SSH status result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SSHStatusResult {
    pub ssh_available: bool,
    pub active_connection_name: Option<String>,
    pub last_sync_time: Option<String>,
    pub last_sync_status: String,
    pub last_sync_error: Option<String>,
}
