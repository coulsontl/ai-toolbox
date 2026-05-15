use serde::{Deserialize, Serialize};

use crate::coding::runtime_location::WslDirectModuleStatus;

// ============================================================================
// File Mapping Types
// ============================================================================

/// File mapping API response (camelCase for frontend)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMapping {
    pub id: String,
    pub name: String,
    pub module: String, // "opencode" | "claude" | "codex" | "openclaw" | "geminicli"
    pub windows_path: String,
    pub wsl_path: String,
    pub enabled: bool,
    pub is_pattern: bool,
    pub is_directory: bool,
}

// ============================================================================
// WSL Sync Config Types
// ============================================================================

/// WSL sync configuration API response (camelCase for frontend)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WSLSyncConfig {
    pub enabled: bool,
    pub distro: String,
    /// Sync MCP configuration to WSL (default: true)
    #[serde(default = "default_true")]
    pub sync_mcp: bool,
    /// Sync Skills to WSL (default: true)
    #[serde(default = "default_true")]
    pub sync_skills: bool,
    pub file_mappings: Vec<FileMapping>,
    pub last_sync_time: Option<String>,
    pub last_sync_status: String, // "success" | "error" | "never"
    pub last_sync_error: Option<String>,
    #[serde(default)]
    pub module_statuses: Vec<WslDirectModuleStatus>,
}

impl Default for WSLSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            distro: String::new(),
            sync_mcp: true,
            sync_skills: true,
            file_mappings: vec![],
            last_sync_time: None,
            last_sync_status: "never".to_string(),
            last_sync_error: None,
            module_statuses: vec![],
        }
    }
}

fn default_true() -> bool {
    true
}

// ============================================================================
// Sync Result Types
// ============================================================================

/// Result of a sync operation (API response)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub success: bool,
    pub synced_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub errors: Vec<String>,
}

/// WSL detection result (API response)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WSLErrorResult {
    pub available: bool,
    pub error: Option<String>,
}

/// WSL detection result with distros (API response)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WSLDetectResult {
    pub available: bool,
    pub distros: Vec<String>,
    pub error: Option<String>,
}

/// WSL status result (API response)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WSLStatusResult {
    pub wsl_available: bool,
    pub last_sync_time: Option<String>,
    pub last_sync_status: String,
    pub last_sync_error: Option<String>,
    #[serde(default)]
    pub module_statuses: Vec<WslDirectModuleStatus>,
}

// ============================================================================
// Sync Progress Types
// ============================================================================

/// Sync progress event payload (sent to frontend via Tauri events)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    /// Current phase: "files" | "mcp" | "skills"
    pub phase: String,
    /// Current item being processed
    pub current_item: String,
    /// Current item index (1-based)
    pub current: u32,
    /// Total items in this phase
    pub total: u32,
    /// Overall progress message
    pub message: String,
    /// Current file being uploaded within the current item, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
}
