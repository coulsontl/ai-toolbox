//! Built-in tool configurations
//!
//! Contains static configuration for all supported AI coding tools.
//!
//! Path prefix conventions:
//! - `~/` - relative to user's home directory
//! - `%APPDATA%/` - relative to config directory (APPDATA on Windows, ~/.config on Linux/macOS)
//! - No prefix - absolute path

use super::types::BuiltinTool;

/// All built-in tool configurations
/// Each tool can support Skills, MCP, or both
pub const BUILTIN_TOOLS: &[BuiltinTool] = &[
    // Claude Code - supports both Skills and MCP
    BuiltinTool {
        key: "claude_code",
        display_name: "Claude Code",
        relative_skills_dir: Some("~/.claude/skills"),
        relative_detect_dir: Some("~/.claude"),
        mcp_config_path: Some("~/.claude.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Codex - supports both Skills and MCP
    BuiltinTool {
        key: "codex",
        display_name: "Codex",
        relative_skills_dir: Some("~/.codex/skills"),
        relative_detect_dir: Some("~/.codex"),
        mcp_config_path: Some("~/.codex/config.toml"),
        mcp_config_format: Some("toml"),
        mcp_field: Some("mcp_servers"),
    },
    // Gemini CLI - supports both Skills and MCP
    BuiltinTool {
        key: "gemini_cli",
        display_name: "Gemini CLI",
        relative_skills_dir: Some("~/.gemini/skills"),
        relative_detect_dir: Some("~/.gemini"),
        mcp_config_path: Some("~/.gemini/settings.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Cursor - supports both Skills and MCP
    BuiltinTool {
        key: "cursor",
        display_name: "Cursor",
        relative_skills_dir: Some("~/.cursor/skills"),
        relative_detect_dir: Some("~/.cursor"),
        mcp_config_path: Some("~/.cursor/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // OpenCode - supports both Skills and MCP
    BuiltinTool {
        key: "opencode",
        display_name: "OpenCode",
        relative_skills_dir: Some("~/.config/opencode/skill"),
        relative_detect_dir: Some("~/.config/opencode"),
        mcp_config_path: Some("~/.config/opencode/opencode.jsonc"), // Dynamic resolution in detection.rs
        mcp_config_format: Some("jsonc"),
        mcp_field: Some("mcp"),
    },
    // Antigravity - supports both Skills and MCP
    BuiltinTool {
        key: "antigravity",
        display_name: "Antigravity",
        relative_skills_dir: Some("~/.gemini/antigravity/skills"),
        relative_detect_dir: Some("~/.gemini/antigravity"),
        mcp_config_path: Some("~/.gemini/antigravity/mcp_config.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Amp - supports both Skills and MCP
    // MCP path uses VSCode plugin config path (%APPDATA%/Code/User/mcp.json)
    // Skills use home_dir: ~/.config/agents/skills
    BuiltinTool {
        key: "amp",
        display_name: "Amp",
        relative_skills_dir: Some("~/.config/agents/skills"),
        relative_detect_dir: Some("%APPDATA%/Code"),
        mcp_config_path: Some("%APPDATA%/Code/User/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("servers"),
    },
    // Kilo Code - supports both Skills and MCP
    // MCP path uses VSCode plugin config path
    // Skills use home_dir: ~/.kilocode/skills
    BuiltinTool {
        key: "kilo_code",
        display_name: "Kilo Code",
        relative_skills_dir: Some("~/.kilocode/skills"),
        relative_detect_dir: Some("%APPDATA%/Code/User/globalStorage/kilocode.kilo-code"),
        mcp_config_path: Some("%APPDATA%/Code/User/globalStorage/kilocode.kilo-code/settings/mcp_settings.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Roo Code - supports both Skills and MCP
    // MCP path uses VSCode plugin config path
    // Skills use home_dir: ~/.roo/skills
    BuiltinTool {
        key: "roo_code",
        display_name: "Roo Code",
        relative_skills_dir: Some("~/.roo/skills"),
        relative_detect_dir: Some("%APPDATA%/Code/User/globalStorage/rooveterinaryinc.roo-cline"),
        mcp_config_path: Some("%APPDATA%/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Goose - Skills only
    BuiltinTool {
        key: "goose",
        display_name: "Goose",
        relative_skills_dir: Some("~/.config/goose/skills"),
        relative_detect_dir: Some("~/.config/goose"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // GitHub Copilot - supports both Skills and MCP
    // MCP path uses VSCode plugin config path (same as Amp)
    BuiltinTool {
        key: "github_copilot",
        display_name: "GitHub Copilot",
        relative_skills_dir: Some("~/.copilot/skills"),
        relative_detect_dir: Some("%APPDATA%/Code"),
        mcp_config_path: Some("%APPDATA%/Code/User/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("servers"),
    },
    // Clawdbot - Skills only
    BuiltinTool {
        key: "clawdbot",
        display_name: "Clawdbot",
        relative_skills_dir: Some("~/.clawdbot/skills"),
        relative_detect_dir: Some("~/.clawdbot"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // Droid - supports both Skills and MCP
    BuiltinTool {
        key: "droid",
        display_name: "Droid",
        relative_skills_dir: Some("~/.factory/skills"),
        relative_detect_dir: Some("~/.factory"),
        mcp_config_path: Some("~/.factory/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Windsurf - supports both Skills and MCP
    BuiltinTool {
        key: "windsurf",
        display_name: "Windsurf",
        relative_skills_dir: Some("~/.codeium/windsurf/skills"),
        relative_detect_dir: Some("~/.codeium/windsurf"),
        mcp_config_path: Some("~/.codeium/mcp_config.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
];

/// Get all built-in tools
pub fn get_all_builtin_tools() -> &'static [BuiltinTool] {
    BUILTIN_TOOLS
}

/// Get built-in tools that support Skills
pub fn get_skills_builtin_tools() -> Vec<&'static BuiltinTool> {
    BUILTIN_TOOLS
        .iter()
        .filter(|t| t.relative_skills_dir.is_some())
        .collect()
}

/// Get built-in tools that support MCP
pub fn get_mcp_builtin_tools() -> Vec<&'static BuiltinTool> {
    BUILTIN_TOOLS
        .iter()
        .filter(|t| t.mcp_config_path.is_some())
        .collect()
}

/// Find a built-in tool by key
pub fn builtin_tool_by_key(key: &str) -> Option<&'static BuiltinTool> {
    BUILTIN_TOOLS.iter().find(|t| t.key == key)
}
