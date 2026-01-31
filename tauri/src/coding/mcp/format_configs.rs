//! MCP Format Configurations
//!
//! Defines format conversion rules for different tools.
//! Each tool may have its own configuration format for MCP servers.

use crate::coding::tools::McpFormatConfig;

/// OpenCode format configuration
///
/// OpenCode uses a different format than ai-toolbox's unified format:
/// - `stdio` -> `local`, `sse/http` -> `remote`
/// - `command` + `args` merged into `command: [...]`
/// - `env` -> `environment`
/// - Requires `enabled: true` field
pub const OPENCODE_FORMAT: McpFormatConfig = McpFormatConfig {
    type_mappings: &[
        ("stdio", "local"),
        ("sse", "remote"),
        ("http", "remote"),
    ],
    merge_command_args: true,
    env_field: "environment",
    requires_enabled: true,
    default_tool_type: "local",
};

/// Get the format config for a tool by key
pub fn get_format_config(tool_key: &str) -> Option<&'static McpFormatConfig> {
    match tool_key {
        "opencode" => Some(&OPENCODE_FORMAT),
        _ => None,
    }
}
