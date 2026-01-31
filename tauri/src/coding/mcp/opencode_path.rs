//! OpenCode MCP Config Path Resolution
//!
//! Provides functions to get the OpenCode MCP configuration file path.
//! This reuses the path resolution logic from the OpenCode module.

use std::path::PathBuf;

/// Get OpenCode MCP config path synchronously (without database access)
///
/// Priority:
/// 1. Environment variable OPENCODE_CONFIG
/// 2. Shell configuration files
/// 3. Default path (~/.config/opencode/opencode.jsonc or .json)
///
/// Note: This is a simplified version that doesn't check the database
/// for custom config path, since MCP sync typically runs without async context.
pub fn get_opencode_mcp_config_path_sync() -> Option<PathBuf> {
    // 1. Check system environment variable
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Some(PathBuf::from(env_path));
        }
    }

    // 2. Check shell configuration files
    if let Some(shell_path) = crate::coding::open_code::shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            return Some(PathBuf::from(shell_path));
        }
    }

    // 3. Return default path
    get_default_opencode_config_path()
}

/// Get the default OpenCode config path
/// Checks for .jsonc first, then .json, then defaults to .jsonc for new files
fn get_default_opencode_config_path() -> Option<PathBuf> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;

    let config_dir = PathBuf::from(&home_dir).join(".config").join("opencode");

    // Check for .jsonc first, then .json
    let jsonc_path = config_dir.join("opencode.jsonc");
    let json_path = config_dir.join("opencode.json");

    if jsonc_path.exists() {
        Some(jsonc_path)
    } else if json_path.exists() {
        Some(json_path)
    } else {
        // Return default path for new file
        Some(jsonc_path)
    }
}
