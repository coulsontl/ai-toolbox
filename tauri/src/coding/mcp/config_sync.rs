//! MCP Configuration File Synchronization
//!
//! Handles reading/writing MCP server configurations to various tool config files.
//! Supports JSON/JSONC (unified with json5) and TOML formats.
//! Also handles format conversion for tools like OpenCode that use different schemas.

use std::path::PathBuf;

use serde_json::Value;

use super::format_configs::get_format_config;
use super::types::{McpServer, McpSyncDetail, now_ms};
use crate::coding::tools::{resolve_mcp_config_path, McpFormatConfig, RuntimeTool};

/// Sync an MCP server to a specific tool's config file
pub fn sync_server_to_tool(
    server: &McpServer,
    tool: &RuntimeTool,
) -> Result<McpSyncDetail, String> {
    let config_path = resolve_mcp_config_path(tool)
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;

    let format = tool.mcp_config_format.as_deref().unwrap_or("json");
    let field = tool.mcp_field.as_deref().unwrap_or("mcpServers");
    let format_config = get_format_config(&tool.key);

    match format {
        // json5 handles both standard JSON and JSONC (with comments, trailing commas)
        "json" | "jsonc" => sync_server_to_json(&config_path, server, field, format_config),
        "toml" => sync_server_to_toml(&config_path, server, field),
        _ => Err(format!("Unsupported config format: {}", format)),
    }
    .map(|_| McpSyncDetail {
        tool: tool.key.clone(),
        status: "ok".to_string(),
        synced_at: Some(now_ms()),
        error_message: None,
    })
    .map_err(|e| e.to_string())
}

/// Remove an MCP server from a specific tool's config file
pub fn remove_server_from_tool(
    server_name: &str,
    tool: &RuntimeTool,
) -> Result<(), String> {
    let config_path = resolve_mcp_config_path(tool)
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;

    let format = tool.mcp_config_format.as_deref().unwrap_or("json");
    let field = tool.mcp_field.as_deref().unwrap_or("mcpServers");

    match format {
        // json5 handles both standard JSON and JSONC (with comments, trailing commas)
        "json" | "jsonc" => remove_server_from_json(&config_path, server_name, field),
        "toml" => remove_server_from_toml(&config_path, server_name, field),
        _ => Err(format!("Unsupported config format: {}", format)),
    }
}

/// Sync server to JSON/JSONC config file (using json5 for parsing)
/// json5 is a superset of JSON that supports comments, trailing commas, etc.
fn sync_server_to_json(
    config_path: &PathBuf,
    server: &McpServer,
    field: &str,
    format_config: Option<&McpFormatConfig>,
) -> Result<(), String> {
    // Read existing config or create new (json5 handles both JSON and JSONC)
    let mut config: Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        let content = content.trim();
        if content.is_empty() {
            serde_json::json!({})
        } else {
            json5::from_str(content)
                .map_err(|e| format!("Failed to parse config file: {}", e))?
        }
    } else {
        serde_json::json!({})
    };

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }

    // Get or create the MCP servers field
    let mcp_servers = config
        .as_object_mut()
        .ok_or("Config is not a JSON object")?
        .entry(field)
        .or_insert(serde_json::json!({}));

    // Build server config based on type and format config
    let server_config = build_json_server_config(server, format_config)?;

    // Add/update server
    mcp_servers
        .as_object_mut()
        .ok_or(format!("{} is not a JSON object", field))?
        .insert(server.name.clone(), server_config);

    // Write back to file with pretty formatting
    // Note: json5 crate doesn't have serialization, so we write standard JSON
    // which is valid JSON5 (JSON is a subset of JSON5)
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(config_path, content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}

/// Remove server from JSON/JSONC config file (using json5 for parsing)
fn remove_server_from_json(
    config_path: &PathBuf,
    server_name: &str,
    field: &str,
) -> Result<(), String> {
    if !config_path.exists() {
        return Ok(()); // Nothing to remove
    }

    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    let content = content.trim();
    if content.is_empty() {
        return Ok(()); // Empty file, nothing to remove
    }
    let mut config: Value = json5::from_str(content)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    // Get the MCP servers field
    if let Some(mcp_servers) = config.get_mut(field) {
        if let Some(servers_obj) = mcp_servers.as_object_mut() {
            servers_obj.remove(server_name);
        }
    }

    // Write back to file
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(config_path, content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}

/// Sync server to TOML config file (using toml_edit for precise formatting)
fn sync_server_to_toml(
    config_path: &PathBuf,
    server: &McpServer,
    field: &str,
) -> Result<(), String> {
    use toml_edit::Item;

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }

    // Read existing config or create new document
    let mut doc = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        if content.trim().is_empty() {
            toml_edit::DocumentMut::new()
        } else {
            content.parse::<toml_edit::DocumentMut>()
                .map_err(|e| format!("Failed to parse TOML config: {}", e))?
        }
    } else {
        toml_edit::DocumentMut::new()
    };

    // Ensure the servers field exists
    if !doc.contains_key(field) {
        doc[field] = toml_edit::table();
    }

    // Build server config using toml_edit
    let server_table = build_toml_edit_server_config(server)?;

    // Add/update server
    doc[field][&server.name] = Item::Table(server_table);

    // Write back to file
    let content = doc.to_string();
    std::fs::write(config_path, content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}

/// Remove server from TOML config file (using toml_edit)
fn remove_server_from_toml(
    config_path: &PathBuf,
    server_name: &str,
    field: &str,
) -> Result<(), String> {
    if !config_path.exists() {
        return Ok(()); // Nothing to remove
    }

    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let mut doc = match content.parse::<toml_edit::DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return Ok(()), // Can't parse, nothing to remove
    };

    // Get the MCP servers field and remove the server
    if let Some(servers) = doc.get_mut(field).and_then(|s| s.as_table_mut()) {
        servers.remove(server_name);
    }

    // Write back to file
    let content = doc.to_string();
    std::fs::write(config_path, content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}

/// Build TOML server configuration using toml_edit (matches cc-switch format)
fn build_toml_edit_server_config(server: &McpServer) -> Result<toml_edit::Table, String> {
    use toml_edit::{Array, Item, Table};

    let mut t = Table::new();

    match server.server_type.as_str() {
        "stdio" => {
            let command = server.server_config
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("stdio server requires 'command' field")?;

            // Insert in order: type -> command -> args -> env
            t["type"] = toml_edit::value("stdio");
            t["command"] = toml_edit::value(command);

            // Build args array (inline format)
            if let Some(args) = server.server_config.get("args").and_then(|v| v.as_array()) {
                let mut arr = Array::default();
                for a in args.iter().filter_map(|x| x.as_str()) {
                    arr.push(a);
                }
                if !arr.is_empty() {
                    t["args"] = Item::Value(toml_edit::Value::Array(arr));
                }
            }

            // Build env as sub-table
            if let Some(env) = server.server_config.get("env").and_then(|v| v.as_object()) {
                let mut env_tbl = Table::new();
                for (k, v) in env.iter() {
                    if let Some(s) = v.as_str() {
                        env_tbl[&k[..]] = toml_edit::value(s);
                    }
                }
                if !env_tbl.is_empty() {
                    t["env"] = Item::Table(env_tbl);
                }
            }
        }
        "http" | "sse" => {
            let url = server.server_config
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or(format!("{} server requires 'url' field", server.server_type))?;

            // Insert in order: type -> url -> http_headers
            t["type"] = toml_edit::value(&server.server_type);
            t["url"] = toml_edit::value(url);

            // Build http_headers as sub-table (Codex uses http_headers, not headers)
            if let Some(headers) = server.server_config.get("headers").and_then(|v| v.as_object()) {
                let mut h_tbl = Table::new();
                for (k, v) in headers.iter() {
                    if let Some(s) = v.as_str() {
                        h_tbl[&k[..]] = toml_edit::value(s);
                    }
                }
                if !h_tbl.is_empty() {
                    t["http_headers"] = Item::Table(h_tbl);
                }
            }
        }
        _ => return Err(format!("Unknown server type: {}", server.server_type)),
    }

    Ok(t)
}

/// Build JSON server configuration from McpServer
/// Applies format conversion if format_config is provided
fn build_json_server_config(server: &McpServer, format_config: Option<&McpFormatConfig>) -> Result<Value, String> {
    match server.server_type.as_str() {
        "stdio" => build_stdio_config(server, format_config),
        "http" | "sse" => build_http_config(server, format_config),
        _ => Err(format!("Unknown server type: {}", server.server_type)),
    }
}

/// Build stdio server configuration
fn build_stdio_config(server: &McpServer, format_config: Option<&McpFormatConfig>) -> Result<Value, String> {
    let command = server.server_config
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("stdio server requires 'command' field")?;

    let args: Vec<String> = server.server_config
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let env = server.server_config.get("env").cloned();

    // Apply format conversion if config is provided
    if let Some(config) = format_config {
        let mut result = serde_json::Map::new();

        // Map server type
        let mapped_type = config.map_type_to_tool("stdio");
        result.insert("type".to_string(), Value::String(mapped_type.to_string()));

        // Merge command and args if needed
        if config.merge_command_args {
            let mut command_array = vec![Value::String(command.to_string())];
            command_array.extend(args.into_iter().map(Value::String));
            result.insert("command".to_string(), Value::Array(command_array));
        } else {
            result.insert("command".to_string(), Value::String(command.to_string()));
            result.insert("args".to_string(), Value::Array(args.into_iter().map(Value::String).collect()));
        }

        // Add environment variables with the correct field name
        if let Some(env_val) = env {
            if env_val.is_object() && !env_val.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                result.insert(config.env_field.to_string(), env_val);
            }
        }

        // Add enabled field if required
        if config.requires_enabled {
            result.insert("enabled".to_string(), Value::Bool(true));
        }

        Ok(Value::Object(result))
    } else {
        // Standard format (Claude Code, Gemini CLI, etc.)
        let mut result = serde_json::json!({
            "type": "stdio",
            "command": command,
            "args": args,
        });

        if let Some(env_val) = env {
            if env_val.is_object() && !env_val.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                result["env"] = env_val;
            }
        }

        Ok(result)
    }
}

/// Build HTTP/SSE server configuration
fn build_http_config(server: &McpServer, format_config: Option<&McpFormatConfig>) -> Result<Value, String> {
    let url = server.server_config
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or(format!("{} server requires 'url' field", server.server_type))?;

    let headers = server.server_config.get("headers").cloned();

    // Apply format conversion if config is provided
    if let Some(config) = format_config {
        let mut result = serde_json::Map::new();

        // Map server type
        let mapped_type = config.map_type_to_tool(&server.server_type);
        result.insert("type".to_string(), Value::String(mapped_type.to_string()));
        result.insert("url".to_string(), Value::String(url.to_string()));

        if let Some(headers_val) = headers {
            if headers_val.is_object() && !headers_val.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                result.insert("headers".to_string(), headers_val);
            }
        }

        // Add enabled field if required
        if config.requires_enabled {
            result.insert("enabled".to_string(), Value::Bool(true));
        }

        Ok(Value::Object(result))
    } else {
        // Standard format (Claude Code, Gemini CLI, etc.)
        let mut result = serde_json::json!({
            "type": &server.server_type,
            "url": url,
        });

        if let Some(headers_val) = headers {
            if headers_val.is_object() && !headers_val.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                result["headers"] = headers_val;
            }
        }

        Ok(result)
    }
}

/// Import MCP servers from a tool's config file
pub fn import_servers_from_tool(tool: &RuntimeTool) -> Result<Vec<McpServer>, String> {
    let config_path = resolve_mcp_config_path(tool)
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;

    if !config_path.exists() {
        return Ok(vec![]);
    }

    let format = tool.mcp_config_format.as_deref().unwrap_or("json");
    let field = tool.mcp_field.as_deref().unwrap_or("mcpServers");
    let format_config = get_format_config(&tool.key);

    match format {
        // json5 handles both standard JSON and JSONC (with comments, trailing commas)
        "json" | "jsonc" => import_servers_from_json(&config_path, field, format_config),
        "toml" => import_servers_from_toml(&config_path, field),
        _ => Err(format!("Unsupported config format: {}", format)),
    }
}

/// Import servers from JSON/JSONC config file (using json5 for parsing)
fn import_servers_from_json(
    config_path: &PathBuf,
    field: &str,
    format_config: Option<&McpFormatConfig>,
) -> Result<Vec<McpServer>, String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    let content = content.trim();
    if content.is_empty() {
        return Ok(vec![]);
    }
    let config: Value = json5::from_str(content)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    parse_mcp_servers_from_value(&config, field, format_config)
}

/// Parse MCP servers from a JSON Value
fn parse_mcp_servers_from_value(
    config: &Value,
    field: &str,
    format_config: Option<&McpFormatConfig>,
) -> Result<Vec<McpServer>, String> {
    let Some(mcp_servers) = config.get(field) else {
        return Ok(vec![]);
    };

    let Some(servers_obj) = mcp_servers.as_object() else {
        return Ok(vec![]);
    };

    let now = now_ms();
    let mut servers = Vec::new();

    for (name, server_config) in servers_obj {
        // Parse the server with format conversion if needed
        if let Some(server) = parse_server_config(name, server_config, format_config, now) {
            servers.push(server);
        }
    }

    Ok(servers)
}

/// Parse a single server config, applying format conversion if needed
fn parse_server_config(
    name: &str,
    server_config: &Value,
    format_config: Option<&McpFormatConfig>,
    now: i64,
) -> Option<McpServer> {
    if let Some(config) = format_config {
        // Tool-specific format - convert to unified format
        parse_server_with_format_config(name, server_config, config, now)
    } else {
        // Standard format
        parse_standard_server_config(name, server_config, now)
    }
}

/// Parse server config with format conversion
fn parse_server_with_format_config(
    name: &str,
    server_config: &Value,
    format_config: &McpFormatConfig,
    now: i64,
) -> Option<McpServer> {
    // Get the tool-specific type and convert to unified type
    // Default to the format config's default type when type field is missing
    let tool_type = server_config
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or(format_config.default_tool_type);
    let server_type = format_config.map_type_from_tool(tool_type);

    // Build unified server_config
    let unified_config = if server_type == "stdio" {
        // Handle command array -> command + args conversion
        let command_val = server_config.get("command")?;

        let (command, args) = if format_config.merge_command_args {
            // Command is an array: ["npx", "-y", "pkg"]
            if let Some(arr) = command_val.as_array() {
                if arr.is_empty() {
                    return None;
                }
                let cmd = arr[0].as_str()?.to_string();
                let args: Vec<Value> = arr[1..].iter().cloned().collect();
                (cmd, args)
            } else if let Some(cmd) = command_val.as_str() {
                // Fallback: command is a string
                (cmd.to_string(), vec![])
            } else {
                return None;
            }
        } else {
            // Standard format: command is string, args is separate
            let cmd = command_val.as_str()?.to_string();
            let args = server_config.get("args")
                .and_then(|v| v.as_array())
                .map(|arr| arr.clone())
                .unwrap_or_default();
            (cmd, args)
        };

        // Get environment variables with the correct field name
        let env = server_config.get(format_config.env_field).cloned();

        let mut result = serde_json::json!({
            "command": command,
            "args": args,
        });
        if let Some(env_val) = env {
            if !env_val.is_null() {
                result["env"] = env_val;
            }
        }
        result
    } else {
        // HTTP/SSE type
        let url = server_config.get("url").and_then(|v| v.as_str())?;
        let headers = server_config.get("headers").cloned();

        let mut result = serde_json::json!({
            "url": url,
        });
        if let Some(headers_val) = headers {
            if !headers_val.is_null() {
                result["headers"] = headers_val;
            }
        }
        result
    };

    Some(McpServer {
        id: String::new(),
        name: name.to_string(),
        server_type: server_type.to_string(),
        server_config: unified_config,
        enabled_tools: vec![],
        sync_details: None,
        description: None,
        tags: vec![],
        sort_index: 0,
        created_at: now,
        updated_at: now,
    })
}

/// Parse standard server config (no format conversion needed)
/// Used by Claude Code, Gemini CLI, etc.
fn parse_standard_server_config(
    name: &str,
    server_config: &Value,
    now: i64,
) -> Option<McpServer> {
    // Detect server type: check explicit "type" field first, fall back to field presence
    let server_type = server_config
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            if server_config.get("command").is_some() {
                "stdio"
            } else if server_config.get("url").is_some() {
                "http"
            } else {
                "stdio" // Default to stdio (matching cc-switch)
            }
        });

    Some(McpServer {
        id: String::new(),
        name: name.to_string(),
        server_type: server_type.to_string(),
        server_config: server_config.clone(),
        enabled_tools: vec![],
        sync_details: None,
        description: None,
        tags: vec![],
        sort_index: 0,
        created_at: now,
        updated_at: now,
    })
}

/// Import servers from TOML config file
fn import_servers_from_toml(config_path: &PathBuf, field: &str) -> Result<Vec<McpServer>, String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    let content_trimmed = content.trim();
    if content_trimmed.is_empty() {
        return Ok(vec![]);
    }
    let config: toml::Table = content_trimmed.parse()
        .map_err(|e| format!("Failed to parse TOML config: {}", e))?;

    let Some(toml::Value::Table(servers_table)) = config.get(field) else {
        return Ok(vec![]);
    };

    let now = now_ms();
    let mut servers = Vec::new();

    for (name, server_config) in servers_table {
        let toml::Value::Table(config_table) = server_config else {
            continue;
        };

        // Detect server type: use "type" field, default to "stdio" (matching cc-switch)
        let server_type = config_table
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                if config_table.get("command").is_some() {
                    "stdio"
                } else if config_table.get("url").is_some() {
                    "http"
                } else {
                    "stdio"
                }
            });

        // Convert TOML to JSON for unified storage
        let mut json_config = serde_json::Map::new();

        match server_type {
            "stdio" => {
                if let Some(cmd) = config_table.get("command").and_then(|v| v.as_str()) {
                    json_config.insert("command".into(), Value::String(cmd.to_string()));
                }
                if let Some(args) = config_table.get("args").and_then(|v| v.as_array()) {
                    let arr: Vec<Value> = args.iter()
                        .filter_map(|x| x.as_str().map(|s| Value::String(s.to_string())))
                        .collect();
                    if !arr.is_empty() {
                        json_config.insert("args".into(), Value::Array(arr));
                    }
                }
                if let Some(toml::Value::Table(env_tbl)) = config_table.get("env") {
                    let mut env_json = serde_json::Map::new();
                    for (k, v) in env_tbl {
                        if let Some(s) = v.as_str() {
                            env_json.insert(k.clone(), Value::String(s.to_string()));
                        }
                    }
                    if !env_json.is_empty() {
                        json_config.insert("env".into(), Value::Object(env_json));
                    }
                }
            }
            "http" | "sse" => {
                if let Some(url) = config_table.get("url").and_then(|v| v.as_str()) {
                    json_config.insert("url".into(), Value::String(url.to_string()));
                }
                // Read from http_headers (Codex format) or headers (legacy), prefer http_headers
                let headers_tbl = config_table.get("http_headers")
                    .and_then(|v| v.as_table())
                    .or_else(|| config_table.get("headers").and_then(|v| v.as_table()));
                if let Some(h_tbl) = headers_tbl {
                    let mut headers_json = serde_json::Map::new();
                    for (k, v) in h_tbl {
                        if let Some(s) = v.as_str() {
                            headers_json.insert(k.clone(), Value::String(s.to_string()));
                        }
                    }
                    if !headers_json.is_empty() {
                        json_config.insert("headers".into(), Value::Object(headers_json));
                    }
                }
            }
            _ => continue,
        }

        servers.push(McpServer {
            id: String::new(),
            name: name.clone(),
            server_type: server_type.to_string(),
            server_config: Value::Object(json_config),
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            tags: vec![],
            sort_index: 0,
            created_at: now,
            updated_at: now,
        });
    }

    Ok(servers)
}
