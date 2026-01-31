//! Type definitions for MCP Server management
//!
//! Contains types for MCP server configuration and synchronization.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// MCP Server type
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum McpServerType {
    Stdio,
    Http,
    Sse,
}

impl Default for McpServerType {
    fn default() -> Self {
        McpServerType::Stdio
    }
}

impl McpServerType {
    pub fn as_str(&self) -> &'static str {
        match self {
            McpServerType::Stdio => "stdio",
            McpServerType::Http => "http",
            McpServerType::Sse => "sse",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "http" => McpServerType::Http,
            "sse" => McpServerType::Sse,
            _ => McpServerType::Stdio,
        }
    }
}

/// MCP Server configuration for stdio type
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StdioConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Value>,
}

/// MCP Server configuration for HTTP/SSE type
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct HttpConfig {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<Value>,
}

/// MCP Server record stored in SurrealDB
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub server_type: String,
    pub server_config: Value,
    pub enabled_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub sort_index: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// MCP Server sync detail for a specific tool
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpSyncDetail {
    pub tool: String,
    pub status: String,  // "ok" | "error" | "pending"
    pub synced_at: Option<i64>,
    pub error_message: Option<String>,
}

/// DTO for MCP Server (frontend display)
#[derive(Debug, Serialize)]
pub struct McpServerDto {
    pub id: String,
    pub name: String,
    pub server_type: String,
    pub server_config: Value,
    pub enabled_tools: Vec<String>,
    pub sync_details: Vec<McpSyncDetailDto>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub sort_index: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// DTO for MCP sync detail (frontend display)
#[derive(Debug, Serialize)]
pub struct McpSyncDetailDto {
    pub tool: String,
    pub status: String,
    pub synced_at: Option<i64>,
    pub error_message: Option<String>,
}

/// Input for creating a new MCP server
#[derive(Clone, Debug, Deserialize)]
pub struct CreateMcpServerInput {
    pub name: String,
    pub server_type: String,
    pub server_config: Value,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Input for updating an MCP server
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateMcpServerInput {
    pub name: Option<String>,
    pub server_type: Option<String>,
    pub server_config: Option<Value>,
    pub enabled_tools: Option<Vec<String>>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}

/// MCP preferences (singleton record)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpPreferences {
    pub id: String,
    pub show_in_tray: bool,
    #[serde(default)]
    pub preferred_tools: Vec<String>,
    pub updated_at: i64,
}

impl Default for McpPreferences {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            show_in_tray: false,
            preferred_tools: Vec::new(),
            updated_at: 0,
        }
    }
}

/// Sync result for a single tool
#[derive(Debug, Serialize)]
pub struct McpSyncResultDto {
    pub tool: String,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Import result
#[derive(Debug, Serialize)]
pub struct McpImportResultDto {
    pub servers_imported: i32,
    pub servers_skipped: i32,
    pub servers_duplicated: Vec<String>,  // Names of servers created with suffix due to config differences
    pub errors: Vec<String>,
}

/// Discovered MCP server info (for scan results)
#[derive(Debug, Serialize)]
pub struct McpDiscoveredServerDto {
    pub name: String,
    pub tool_key: String,
    pub tool_name: String,
    pub server_type: String,
}

/// Scan result for discovered MCP servers
#[derive(Debug, Serialize)]
pub struct McpScanResultDto {
    pub total_tools_scanned: i32,
    pub total_servers_found: i32,
    pub servers: Vec<McpDiscoveredServerDto>,
}

/// Helper function to get current timestamp in milliseconds
pub fn now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
