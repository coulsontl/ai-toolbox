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

pub const CLAUDE_PLUGINS_MAPPING_ID: &str = "claude-plugins";
pub const CODEX_PLUGINS_MAPPING_ID: &str = "codex-plugins";
/// Windows-only Codex plugin server binaries. The SSH/WSL targets are POSIX
/// systems, so uploading this directory only wastes hundreds of megabytes.
pub const CODEX_PLUGIN_APPSERVER_DIR: &str = ".plugin-appserver";

pub fn default_directory_excludes() -> Vec<String> {
    DEFAULT_DIRECTORY_EXCLUDES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

pub fn default_directory_excludes_for_mapping(mapping_id: &str) -> Vec<String> {
    let mut excludes = default_directory_excludes();
    if mapping_id == CLAUDE_PLUGINS_MAPPING_ID {
        excludes.retain(|name| name != "cache");
    }
    if mapping_id == CODEX_PLUGINS_MAPPING_ID {
        excludes.push(CODEX_PLUGIN_APPSERVER_DIR.to_string());
    }
    excludes
}

/// Excludes actually used for a mapping. Existing saved mappings may not have
/// the newer codex default, so enforce it here as well without rewriting the
/// user's stored exclude list.
pub fn effective_directory_excludes_for_mapping(
    mapping_id: &str,
    configured_excludes: &[String],
) -> Vec<String> {
    let mut excludes = normalize_directory_excludes(configured_excludes);
    if mapping_id == CODEX_PLUGINS_MAPPING_ID
        && !excludes
            .iter()
            .any(|name| name == CODEX_PLUGIN_APPSERVER_DIR)
    {
        excludes.push(CODEX_PLUGIN_APPSERVER_DIR.to_string());
    }
    excludes
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

pub fn matches_default_directory_excludes(excludes: &[String]) -> bool {
    normalize_directory_excludes(excludes) == default_directory_excludes()
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
    pub module: String, // "opencode" | "claude" | "codex" | "grok" | "openclaw" | "geminicli"
    pub local_path: String,
    pub remote_path: String,
    pub enabled: bool,
    pub is_pattern: bool,
    pub is_directory: bool,
    #[serde(default)]
    pub directory_excludes: Vec<String>,
    #[serde(default)]
    pub cleanup_paths: Vec<String>,
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
    /// Warnings from the most recent Skills sync run (kept foreign paths, etc.)
    #[serde(default)]
    pub last_sync_warnings: Vec<String>,
    /// When the remote login user is root, rewrite Claude's bypass permission
    /// mode in the uploaded settings copy to `auto`.
    #[serde(default)]
    pub claude_root_permission_fix: bool,
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
            last_sync_warnings: vec![],
            claude_root_permission_fix: false,
            module_statuses: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        default_directory_excludes, default_directory_excludes_for_mapping,
        effective_directory_excludes_for_mapping, matches_default_directory_excludes,
        normalize_directory_excludes, CLAUDE_PLUGINS_MAPPING_ID, CODEX_PLUGINS_MAPPING_ID,
        CODEX_PLUGIN_APPSERVER_DIR,
    };

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

    #[test]
    fn claude_plugins_default_excludes_keep_plugin_cache_available() {
        let excludes = default_directory_excludes_for_mapping(CLAUDE_PLUGINS_MAPPING_ID);

        assert!(excludes.contains(&".venv".to_string()));
        assert!(excludes.contains(&"node_modules".to_string()));
        assert!(!excludes.contains(&"cache".to_string()));
    }

    #[test]
    fn codex_plugins_default_excludes_skip_windows_only_appserver() {
        let excludes = default_directory_excludes_for_mapping(CODEX_PLUGINS_MAPPING_ID);

        assert!(excludes.contains(&"cache".to_string()));
        assert!(excludes.contains(&CODEX_PLUGIN_APPSERVER_DIR.to_string()));
    }

    #[test]
    fn effective_excludes_add_appserver_without_losing_custom_entries() {
        let configured = vec!["cache".to_string(), "my-cache".to_string()];

        let excludes =
            effective_directory_excludes_for_mapping(CODEX_PLUGINS_MAPPING_ID, &configured);

        assert_eq!(
            excludes,
            vec![
                "cache".to_string(),
                "my-cache".to_string(),
                CODEX_PLUGIN_APPSERVER_DIR.to_string()
            ]
        );
    }

    #[test]
    fn detects_normalized_default_directory_excludes() {
        let input = vec![
            ".git".to_string(),
            ".venv".to_string(),
            "venv".to_string(),
            "node_modules".to_string(),
            "__pycache__".to_string(),
            ".pytest_cache".to_string(),
            ".mypy_cache".to_string(),
            "cache/".to_string(),
        ];

        assert!(matches_default_directory_excludes(&input));
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
    /// Detected remote shell dialect: `posix`, `windows_cmd`,
    /// `windows_powershell`, or `unknown`.
    #[serde(default)]
    pub shell_kind: Option<String>,
    /// Whether the remote login user is root. Only probed for POSIX remotes.
    #[serde(default)]
    pub is_root: Option<bool>,
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
    #[serde(default)]
    pub last_sync_warnings: Vec<String>,
}
