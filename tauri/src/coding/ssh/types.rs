use serde::{Deserialize, Serialize};

// Re-use SyncResult and SyncProgress from wsl module
pub use super::super::wsl::{SyncResult, SyncProgress};

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
    pub auth_method: String, // "key" | "password"
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
    pub module: String, // "opencode" | "claude" | "codex"
    pub local_path: String,
    pub remote_path: String,
    pub enabled: bool,
    pub is_pattern: bool,
    pub is_directory: bool,
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
    /// Sync MCP configuration to SSH (default: true)
    #[serde(default = "default_true")]
    pub sync_mcp: bool,
    /// Sync Skills to SSH (default: true)
    #[serde(default = "default_true")]
    pub sync_skills: bool,
    pub file_mappings: Vec<SSHFileMapping>,
    pub connections: Vec<SSHConnection>,
    pub last_sync_time: Option<String>,
    pub last_sync_status: String, // "success" | "error" | "never"
    pub last_sync_error: Option<String>,
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
        }
    }
}

fn default_true() -> bool {
    true
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
