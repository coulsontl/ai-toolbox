//! MCP Command Normalization Module
//!
//! Handles cmd /c wrapper for Windows compatibility.
//!
//! ## Background
//! On Windows, commands like npx/npm/yarn/pnpm/node/bun/deno are actually .cmd batch files
//! and need to be executed via `cmd /c`. However:
//! - Database storage should be normalized (no cmd /c)
//! - Windows local sync needs cmd /c wrapper
//! - Mac/Linux/WSL don't need cmd /c
//!
//! ## Functions
//! - `unwrap_cmd_c`: Remove cmd /c wrapper (for database storage, import, WSL)
//! - `wrap_cmd_c`: Add cmd /c wrapper (for Windows local sync)
//! - `process_*`: Process entire config file content (for cross-platform backup restore)

use serde_json::{json, Value};

/// Commands that need cmd /c wrapper on Windows
const WINDOWS_WRAP_COMMANDS: &[&str] = &["npx", "npm", "yarn", "pnpm", "node", "bun", "deno"];

/// Check if a command needs cmd /c wrapper
fn needs_wrap(command: &str) -> bool {
    let cmd_name = std::path::Path::new(command)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(command);

    WINDOWS_WRAP_COMMANDS
        .iter()
        .any(|&c| cmd_name.eq_ignore_ascii_case(c))
}

/// Check if command is already wrapped with cmd /c
fn is_cmd_wrapped(command: &str, args: &[Value]) -> bool {
    if !command.eq_ignore_ascii_case("cmd") && !command.eq_ignore_ascii_case("cmd.exe") {
        return false;
    }

    // Check if first arg is /c
    args.first()
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("/c"))
        .unwrap_or(false)
}

// ============================================================================
// Single Server Config Processing
// ============================================================================

/// Remove cmd /c wrapper from server config (only for stdio type)
///
/// Input:  {"type": "stdio", "command": "cmd", "args": ["/c", "npx", "-y", "foo"]}
/// Output: {"type": "stdio", "command": "npx", "args": ["-y", "foo"]}
///
/// http/sse types are returned unchanged.
pub fn unwrap_cmd_c(server_config: &Value) -> Value {
    let Some(obj) = server_config.as_object() else {
        return server_config.clone();
    };

    // Only process stdio type
    let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");
    if server_type != "stdio" {
        return server_config.clone();
    }

    let command = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = obj
        .get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Check if wrapped with cmd /c
    if !is_cmd_wrapped(command, &args) {
        return server_config.clone();
    }

    // Unwrap: args[1] becomes command, args[2..] become new args
    if args.len() < 2 {
        return server_config.clone();
    }

    let new_command = args[1].as_str().unwrap_or("");
    let new_args: Vec<Value> = args[2..].to_vec();

    let mut result = obj.clone();
    result.insert("command".to_string(), json!(new_command));
    result.insert("args".to_string(), json!(new_args));

    Value::Object(result)
}

/// Add cmd /c wrapper to server config (only for stdio type, only on Windows)
///
/// Input:  {"type": "stdio", "command": "npx", "args": ["-y", "foo"]}
/// Output: {"type": "stdio", "command": "cmd", "args": ["/c", "npx", "-y", "foo"]}
///
/// On non-Windows, returns the input unchanged.
/// http/sse types are returned unchanged.
/// Commands not in WINDOWS_WRAP_COMMANDS are returned unchanged.
#[cfg(windows)]
pub fn wrap_cmd_c(server_config: &Value) -> Value {
    let Some(obj) = server_config.as_object() else {
        return server_config.clone();
    };

    // Only process stdio type
    let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");
    if server_type != "stdio" {
        return server_config.clone();
    }

    let command = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = obj
        .get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Already wrapped?
    if is_cmd_wrapped(command, &args) {
        return server_config.clone();
    }

    // Check if command needs wrapping
    if !needs_wrap(command) {
        return server_config.clone();
    }

    // Wrap: command becomes "cmd", args become ["/c", original_command, ...original_args]
    let mut new_args = vec![json!("/c"), json!(command)];
    new_args.extend(args);

    let mut result = obj.clone();
    result.insert("command".to_string(), json!("cmd"));
    result.insert("args".to_string(), Value::Array(new_args));

    Value::Object(result)
}

#[cfg(not(windows))]
pub fn wrap_cmd_c(server_config: &Value) -> Value {
    // On non-Windows, no wrapping needed
    server_config.clone()
}

// ============================================================================
// OpenCode Array Format Processing
// ============================================================================

/// Unwrap cmd /c from OpenCode command array format
///
/// Input:  ["cmd", "/c", "npx", "-y", "foo"]
/// Output: ["npx", "-y", "foo"]
pub fn unwrap_cmd_c_opencode_array(command_array: &[Value]) -> Vec<Value> {
    if command_array.len() < 3 {
        return command_array.to_vec();
    }

    let first = command_array[0].as_str().unwrap_or("");
    let second = command_array[1].as_str().unwrap_or("");

    if (first.eq_ignore_ascii_case("cmd") || first.eq_ignore_ascii_case("cmd.exe"))
        && second.eq_ignore_ascii_case("/c")
    {
        command_array[2..].to_vec()
    } else {
        command_array.to_vec()
    }
}

/// Wrap cmd /c for OpenCode command array format (Windows only)
///
/// Input:  ["npx", "-y", "foo"]
/// Output: ["cmd", "/c", "npx", "-y", "foo"]
#[cfg(windows)]
pub fn wrap_cmd_c_opencode_array(command_array: &[Value]) -> Vec<Value> {
    if command_array.is_empty() {
        return command_array.to_vec();
    }

    let first = command_array[0].as_str().unwrap_or("");

    // Already wrapped?
    if first.eq_ignore_ascii_case("cmd") || first.eq_ignore_ascii_case("cmd.exe") {
        return command_array.to_vec();
    }

    // Check if needs wrapping
    if !needs_wrap(first) {
        return command_array.to_vec();
    }

    let mut result = vec![json!("cmd"), json!("/c")];
    result.extend(command_array.iter().cloned());
    result
}

#[cfg(not(windows))]
pub fn wrap_cmd_c_opencode_array(command_array: &[Value]) -> Vec<Value> {
    command_array.to_vec()
}

// ============================================================================
// Full Config File Processing (for backup restore and WSL sync)
// ============================================================================

/// Process Claude JSON config file content
///
/// - wrap=true: Add cmd /c (restore to Windows)
/// - wrap=false: Remove cmd /c (restore to Mac/Linux/WSL)
pub fn process_claude_json(content: &str, wrap: bool) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    let mut root: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse Claude JSON: {}", e))?;

    // Process mcpServers field
    if let Some(mcp_servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        for (_name, server_config) in mcp_servers.iter_mut() {
            let processed = if wrap {
                wrap_cmd_c(server_config)
            } else {
                unwrap_cmd_c(server_config)
            };
            *server_config = processed;
        }
    }

    serde_json::to_string_pretty(&root).map_err(|e| format!("Failed to serialize JSON: {}", e))
}

/// Process OpenCode JSON/JSONC config file content
///
/// OpenCode format: type=local, command=array
pub fn process_opencode_json(content: &str, wrap: bool) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    let mut root: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse OpenCode JSON: {}", e))?;

    // Process mcp.servers or mcp (depending on format)
    // OpenCode uses "mcp" field which can be an object with server configs
    if let Some(mcp) = root.get_mut("mcp").and_then(|v| v.as_object_mut()) {
        for (name, server_config) in mcp.iter_mut() {
            // Skip non-object entries and special fields
            if name == "enabled" || name == "disabled" {
                continue;
            }

            let Some(obj) = server_config.as_object_mut() else {
                continue;
            };

            // Only process local type (equivalent to stdio)
            let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("local");
            if server_type != "local" {
                continue;
            }

            // OpenCode uses command as array
            if let Some(cmd_arr) = obj.get("command").and_then(|v| v.as_array()).cloned() {
                let processed = if wrap {
                    wrap_cmd_c_opencode_array(&cmd_arr)
                } else {
                    unwrap_cmd_c_opencode_array(&cmd_arr)
                };
                obj.insert("command".to_string(), Value::Array(processed));
            }
        }
    }

    serde_json::to_string_pretty(&root).map_err(|e| format!("Failed to serialize JSON: {}", e))
}

/// Process Codex TOML config file content
///
/// Codex format: [mcp_servers.name] with command and args fields
pub fn process_codex_toml(content: &str, wrap: bool) -> Result<String, String> {
    if content.trim().is_empty() {
        return Ok(content.to_string());
    }

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| format!("Failed to parse Codex TOML: {}", e))?;

    // Process mcp_servers table
    if let Some(mcp_servers) = doc.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
        for (_name, server_item) in mcp_servers.iter_mut() {
            let Some(server) = server_item.as_table_mut() else {
                continue;
            };

            // Only process stdio type
            let server_type = server
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("stdio");
            if server_type != "stdio" {
                continue;
            }

            let command = server
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args: Vec<String> = server
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if wrap {
                // Wrap cmd /c
                if !command.eq_ignore_ascii_case("cmd")
                    && !command.eq_ignore_ascii_case("cmd.exe")
                    && needs_wrap(&command)
                {
                    server["command"] = toml_edit::value("cmd");
                    let mut new_args = toml_edit::Array::new();
                    new_args.push("/c");
                    new_args.push(&command);
                    for arg in &args {
                        new_args.push(arg.as_str());
                    }
                    server["args"] = toml_edit::value(new_args);
                }
            } else {
                // Unwrap cmd /c
                if (command.eq_ignore_ascii_case("cmd") || command.eq_ignore_ascii_case("cmd.exe"))
                    && args
                        .first()
                        .map(|s| s.eq_ignore_ascii_case("/c"))
                        .unwrap_or(false)
                    && args.len() >= 2
                {
                    server["command"] = toml_edit::value(&args[1]);
                    let mut new_args = toml_edit::Array::new();
                    for arg in &args[2..] {
                        new_args.push(arg.as_str());
                    }
                    server["args"] = toml_edit::value(new_args);
                }
            }
        }
    }

    Ok(doc.to_string())
}
