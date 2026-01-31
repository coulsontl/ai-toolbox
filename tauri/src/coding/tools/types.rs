//! Type definitions for the shared tools module
//!
//! Contains types used by both Skills and MCP features.

use serde::{Deserialize, Serialize};

/// Built-in tool configuration (compile-time constants)
/// Contains both Skills and MCP related configuration
#[derive(Clone, Debug)]
pub struct BuiltinTool {
    pub key: &'static str,
    pub display_name: &'static str,
    // Skills related (optional)
    pub relative_skills_dir: Option<&'static str>,
    pub relative_detect_dir: Option<&'static str>,
    // MCP related (optional)
    pub mcp_config_path: Option<&'static str>,
    pub mcp_config_format: Option<&'static str>, // "json" | "toml"
    pub mcp_field: Option<&'static str>,         // field name in config file
}

/// Custom tool defined by user (database storage)
/// Supports both Skills and MCP configurations
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomTool {
    pub key: String,
    pub display_name: String,
    // Skills related (optional)
    pub relative_skills_dir: Option<String>,
    pub relative_detect_dir: Option<String>,
    // MCP related (optional)
    pub mcp_config_path: Option<String>,
    pub mcp_config_format: Option<String>,
    pub mcp_field: Option<String>,
    pub created_at: i64,
}

/// Runtime tool adapter (unified interface for built-in and custom tools)
#[derive(Clone, Debug, Serialize)]
pub struct RuntimeTool {
    pub key: String,
    pub display_name: String,
    pub is_custom: bool,
    // Skills related
    pub relative_skills_dir: Option<String>,
    pub relative_detect_dir: Option<String>,
    // MCP related
    pub mcp_config_path: Option<String>,
    pub mcp_config_format: Option<String>,
    pub mcp_field: Option<String>,
}

impl From<&BuiltinTool> for RuntimeTool {
    fn from(tool: &BuiltinTool) -> Self {
        RuntimeTool {
            key: tool.key.to_string(),
            display_name: tool.display_name.to_string(),
            is_custom: false,
            relative_skills_dir: tool.relative_skills_dir.map(|s| s.to_string()),
            relative_detect_dir: tool.relative_detect_dir.map(|s| s.to_string()),
            mcp_config_path: tool.mcp_config_path.map(|s| s.to_string()),
            mcp_config_format: tool.mcp_config_format.map(|s| s.to_string()),
            mcp_field: tool.mcp_field.map(|s| s.to_string()),
        }
    }
}

impl From<&CustomTool> for RuntimeTool {
    fn from(tool: &CustomTool) -> Self {
        RuntimeTool {
            key: tool.key.clone(),
            display_name: tool.display_name.clone(),
            is_custom: true,
            relative_skills_dir: tool.relative_skills_dir.clone(),
            relative_detect_dir: tool.relative_detect_dir.clone(),
            mcp_config_path: tool.mcp_config_path.clone(),
            mcp_config_format: tool.mcp_config_format.clone(),
            mcp_field: tool.mcp_field.clone(),
        }
    }
}

/// DTO for custom tool (frontend display)
#[derive(Debug, Serialize)]
pub struct CustomToolDto {
    pub key: String,
    pub display_name: String,
    pub relative_skills_dir: Option<String>,
    pub relative_detect_dir: Option<String>,
    pub mcp_config_path: Option<String>,
    pub mcp_config_format: Option<String>,
    pub mcp_field: Option<String>,
    pub created_at: i64,
}

impl From<CustomTool> for CustomToolDto {
    fn from(tool: CustomTool) -> Self {
        CustomToolDto {
            key: tool.key,
            display_name: tool.display_name,
            relative_skills_dir: tool.relative_skills_dir,
            relative_detect_dir: tool.relative_detect_dir,
            mcp_config_path: tool.mcp_config_path,
            mcp_config_format: tool.mcp_config_format,
            mcp_field: tool.mcp_field,
            created_at: tool.created_at,
        }
    }
}

/// DTO for runtime tool (frontend display)
#[derive(Debug, Serialize)]
pub struct RuntimeToolDto {
    pub key: String,
    pub display_name: String,
    pub is_custom: bool,
    pub installed: bool,
    // Skills related
    pub relative_skills_dir: Option<String>,
    pub skills_path: Option<String>,
    pub supports_skills: bool,
    // MCP related
    pub mcp_config_path: Option<String>,
    pub mcp_config_format: Option<String>,
    pub mcp_field: Option<String>,
    pub supports_mcp: bool,
}

/// Tool detection result
#[derive(Debug, Serialize)]
pub struct ToolDetectionDto {
    pub key: String,
    pub display_name: String,
    pub installed: bool,
    pub supports_skills: bool,
    pub supports_mcp: bool,
}

/// MCP format configuration for different tools
/// Defines how to convert between ai-toolbox's unified format and tool-specific formats
#[derive(Clone, Debug)]
pub struct McpFormatConfig {
    /// Type mappings (e.g., "stdio" -> "local", "sse" -> "remote")
    pub type_mappings: &'static [(&'static str, &'static str)],
    /// Whether to merge command and args into a single array
    pub merge_command_args: bool,
    /// Field name for environment variables ("env" or "environment")
    pub env_field: &'static str,
    /// Whether the format requires an "enabled" field
    pub requires_enabled: bool,
    /// Default tool type when type field is missing (e.g., "local" for OpenCode)
    pub default_tool_type: &'static str,
}

impl McpFormatConfig {
    /// Map a server type from unified format to tool format
    pub fn map_type_to_tool(&self, server_type: &str) -> String {
        for (from, to) in self.type_mappings {
            if *from == server_type {
                return to.to_string();
            }
        }
        server_type.to_string()
    }

    /// Map a server type from tool format to unified format
    pub fn map_type_from_tool(&self, tool_type: &str) -> String {
        for (from, to) in self.type_mappings {
            if *to == tool_type {
                return from.to_string();
            }
        }
        tool_type.to_string()
    }
}

/// Helper function to get current timestamp in milliseconds
pub fn now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
