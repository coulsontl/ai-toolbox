//! MCP Configuration File Synchronization
//!
//! Handles reading/writing MCP server configurations to various tool config files.
//! Supports JSON/JSONC (unified with json5) and TOML formats.
//! Also handles format conversion for tools like OpenCode that use different schemas.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::command_normalize;
use super::format_configs::get_format_config;
use super::types::{now_ms, McpServer, McpSyncDetail};
use crate::coding::{
    expand_local_path, runtime_location,
    tools::{
        resolve_mcp_config_path_with_db, resolve_mcp_config_path_with_db_async, McpFormatConfig,
        RuntimeTool,
    },
};

const OMP_MCP_SCHEMA_URL: &str = "https://raw.githubusercontent.com/can1357/oh-my-pi/main/packages/coding-agent/src/config/mcp-schema.json";

/// Sync an MCP server to a specific tool's config file
pub fn sync_server_to_tool(
    db: &crate::db::SqliteDbState,
    server: &McpServer,
    tool: &RuntimeTool,
) -> Result<McpSyncDetail, String> {
    sync_server_to_tool_with_enabled(db, server, tool, true)
}

pub async fn sync_server_to_tool_async(
    db: &crate::db::SqliteDbState,
    server: &McpServer,
    tool: &RuntimeTool,
) -> Result<McpSyncDetail, String> {
    sync_server_to_tool_with_enabled_async(db, server, tool, true).await
}

/// Sync an MCP server to a specific tool's config file with explicit enabled state
pub fn sync_server_to_tool_with_enabled(
    db: &crate::db::SqliteDbState,
    server: &McpServer,
    tool: &RuntimeTool,
    enabled: bool,
) -> Result<McpSyncDetail, String> {
    let config_path = resolve_mcp_config_path_with_db(db, tool)
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;
    sync_server_to_path(tool, &config_path, server, enabled)
}

pub async fn sync_server_to_tool_with_enabled_async(
    db: &crate::db::SqliteDbState,
    server: &McpServer,
    tool: &RuntimeTool,
    enabled: bool,
) -> Result<McpSyncDetail, String> {
    let config_path = resolve_mcp_config_path_with_db_async(db, tool)
        .await
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;
    sync_server_to_path(tool, &config_path, server, enabled)
}

/// Remove an MCP server from a specific tool's config file
pub fn remove_server_from_tool(
    db: &crate::db::SqliteDbState,
    server_name: &str,
    tool: &RuntimeTool,
) -> Result<(), String> {
    let config_path = resolve_mcp_config_path_with_db(db, tool)
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;
    remove_server_from_path(tool, &config_path, server_name)
}

pub async fn remove_server_from_tool_async(
    db: &crate::db::SqliteDbState,
    server_name: &str,
    tool: &RuntimeTool,
) -> Result<(), String> {
    let config_path = resolve_mcp_config_path_with_db_async(db, tool)
        .await
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;
    remove_server_from_path(tool, &config_path, server_name)
}

/// Clone the server and apply `cmd /c` wrapping to its `server_config` when
/// `should_wrap` is true and the server is stdio. Non-stdio servers are returned
/// unchanged. Used by the yaml/cordis arms which don't thread `should_wrap_cmd`
/// through their adapter signatures.
fn wrap_server_for_target(server: &McpServer, should_wrap: bool) -> McpServer {
    if !should_wrap || server.server_type != "stdio" {
        return server.clone();
    }
    let mut wrapped = server.clone();
    wrapped.server_config = command_normalize::wrap_cmd_c_for_target(&wrapped.server_config, true);
    wrapped
}

/// Clone the server and expand `~/`, `$HOME`, `%APPDATA%`, etc. in its stdio
/// `command` and each `args` entry to absolute host paths via `expand_local_path`.
///
/// Non-stdio servers are returned unchanged. Pure-flag args (e.g. `-y`, `npx`)
/// are untouched — `expand_local_path` only substitutes when the value starts
/// with `~/` or contains a known env-var pattern.
fn expand_stdio_paths(server: &McpServer) -> McpServer {
    if server.server_type != "stdio" {
        return server.clone();
    }

    let mut expanded = server.clone();
    let Some(config) = expanded.server_config.as_object_mut() else {
        return expanded;
    };

    if let Some(cmd) = config.get("command").and_then(Value::as_str) {
        if let Ok(resolved) = expand_local_path(cmd) {
            config.insert("command".to_string(), Value::String(resolved));
        }
    }

    if let Some(args) = config.get("args").and_then(Value::as_array) {
        let new_args: Vec<Value> = args
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| expand_local_path(s).ok())
                    .map(Value::String)
                    .unwrap_or_else(|| v.clone())
            })
            .collect();
        config.insert("args".to_string(), Value::Array(new_args));
    }

    expanded
}

/// Earlier ai-toolbox versions synced Kimi MCP servers into the
/// `[mcp_servers]` table of config.toml, but Kimi Code CLI only reads
/// `<root>/mcp.json`. Remove the dead table whenever we touch kimi's MCP
/// config so stale entries do not linger. Best-effort hygiene: the delete is
/// idempotent, and a concurrent Kimi projection write (which preserves unknown
/// tables) would simply re-add it for the next sync to clean up.
fn remove_legacy_kimi_toml_mcp_table(mcp_config_path: &Path) {
    let Some(parent) = mcp_config_path.parent() else {
        return;
    };
    let config_toml_path = parent.join("config.toml");
    if !config_toml_path.exists() {
        return;
    }

    let cleanup = || -> Result<bool, String> {
        let content = std::fs::read_to_string(&config_toml_path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        let mut doc = content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("Failed to parse TOML config: {}", e))?;
        if doc
            .get("mcp_servers")
            .and_then(|item| item.as_table())
            .is_none()
        {
            return Ok(false);
        }
        doc.remove("mcp_servers");
        std::fs::write(&config_toml_path, doc.to_string())
            .map_err(|e| format!("Failed to write config file: {}", e))?;
        Ok(true)
    };

    match cleanup() {
        Ok(true) => log::info!(
            "Removed legacy [mcp_servers] table from {}",
            config_toml_path.display()
        ),
        Ok(false) => {}
        Err(e) => log::warn!(
            "Failed to remove legacy [mcp_servers] table from {}: {}",
            config_toml_path.display(),
            e
        ),
    }
}

fn sync_server_to_path(
    tool: &RuntimeTool,
    config_path: &PathBuf,
    server: &McpServer,
    enabled: bool,
) -> Result<McpSyncDetail, String> {
    let format = tool.mcp_config_format.as_deref().unwrap_or("json");
    let field = tool.mcp_field.as_deref().unwrap_or("mcpServers");
    let format_config = get_format_config(&tool.key);
    let should_wrap_cmd = should_wrap_cmd_for_config_path(config_path);

    if tool.key == "kimi" {
        remove_legacy_kimi_toml_mcp_table(config_path);
    }

    // Expand `~/`, `$HOME`, `%APPDATA%`, etc. in stdio command/args to absolute
    // paths before writing to the local tool config file. CLI runners (Claude Code,
    // Codex, ...) generally do not expand these themselves, so we resolve them on
    // the host at write time. DB keeps the raw portable form. WSL sync is handled
    // separately and does not go through this path.
    let expanded = expand_stdio_paths(server);
    let server = &expanded;

    match format {
        // json5 handles both standard JSON and JSONC (with comments, trailing commas)
        "json" | "jsonc" => sync_server_to_json(
            config_path,
            server,
            field,
            format_config,
            enabled,
            &tool.key,
            should_wrap_cmd,
        ),
        "toml" => sync_server_to_toml(
            config_path,
            server,
            field,
            enabled,
            &tool.key,
            should_wrap_cmd,
        ),
        "yaml" => match tool.key.as_str() {
            "hermes" => {
                let wrapped = wrap_server_for_target(server, should_wrap_cmd);
                super::hermes_mcp::sync_server_to_hermes(config_path, &wrapped, enabled)
            }
            _ => Err(format!(
                "yaml format only supported for hermes, got tool {}",
                tool.key
            )),
        },
        "cordis" => match tool.key.as_str() {
            "dsh" => {
                let wrapped = wrap_server_for_target(server, should_wrap_cmd);
                super::cordis_patch::sync_server_to_cordis(config_path, &wrapped)
            }
            _ => Err(format!(
                "cordis format only supported for dsh, got tool {}",
                tool.key
            )),
        },
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

fn should_wrap_cmd_for_config_path(config_path: &Path) -> bool {
    cfg!(windows) && should_wrap_cmd_for_windows_config_path(config_path)
}

fn should_wrap_cmd_for_windows_config_path(config_path: &Path) -> bool {
    config_path
        .to_str()
        .map(|path| !runtime_location::is_wsl_unc_path(path))
        .unwrap_or(true)
}

fn remove_server_from_path(
    tool: &RuntimeTool,
    config_path: &PathBuf,
    server_name: &str,
) -> Result<(), String> {
    let format = tool.mcp_config_format.as_deref().unwrap_or("json");
    let field = tool.mcp_field.as_deref().unwrap_or("mcpServers");

    if tool.key == "kimi" {
        remove_legacy_kimi_toml_mcp_table(config_path);
    }

    match format {
        // json5 handles both standard JSON and JSONC (with comments, trailing commas)
        "json" | "jsonc" => remove_server_from_json(config_path, server_name, field),
        "toml" => remove_server_from_toml(config_path, server_name, field),
        "yaml" => match tool.key.as_str() {
            "hermes" => super::hermes_mcp::remove_server_from_hermes(config_path, server_name),
            _ => Ok(()),
        },
        "cordis" => match tool.key.as_str() {
            "dsh" => super::cordis_patch::remove_server_from_cordis(config_path, server_name),
            _ => Ok(()),
        },
        _ => Err(format!("Unsupported config format: {}", format)),
    }
}

/// Oh My Pi 的 mcp.json 顶层允许 `$schema`/`mcpServers`/`disabledServers`/`enabledServers`。
/// AI Toolbox 首次创建文件时写入官方 `$schema`,不覆盖已有顶层字段。
fn ensure_omp_mcp_schema(config: &mut Value, tool_key: &str) -> Result<(), String> {
    if tool_key != "oh_my_pi" {
        return Ok(());
    }
    config
        .as_object_mut()
        .ok_or_else(|| "OMP MCP config root must be a JSON object".to_string())?
        .entry("$schema".to_string())
        .or_insert_with(|| Value::String(OMP_MCP_SCHEMA_URL.to_string()));
    Ok(())
}

/// Sync server to JSON/JSONC config file (using json5 for parsing)
/// json5 is a superset of JSON that supports comments, trailing commas, etc.
fn sync_server_to_json(
    config_path: &PathBuf,
    server: &McpServer,
    field: &str,
    format_config: Option<&McpFormatConfig>,
    enabled: bool,
    tool_key: &str,
    should_wrap_cmd: bool,
) -> Result<(), String> {
    // Read existing config or create new (json5 handles both JSON and JSONC)
    let mut config: Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        let content = content.trim();
        if content.is_empty() {
            serde_json::json!({})
        } else {
            json5::from_str(content).map_err(|e| format!("Failed to parse config file: {}", e))?
        }
    } else {
        serde_json::json!({})
    };

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }

    // Get or create the MCP servers field, supporting nested paths like `mcp.servers`.
    let mcp_servers = ensure_json_object_path(&mut config, field)?;

    // Build server config based on type and format config
    let server_config =
        build_json_server_config(server, format_config, enabled, tool_key, should_wrap_cmd)?;

    // Add/update server
    mcp_servers
        .as_object_mut()
        .ok_or(format!("{} is not a JSON object", field))?
        .insert(server.name.clone(), server_config);

    ensure_omp_mcp_schema(&mut config, tool_key)?;

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
    let mut config: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse config file: {}", e))?;

    // Get the MCP servers field, supporting nested paths like `mcp.servers`.
    if let Some(mcp_servers) = get_json_value_by_path_mut(&mut config, field) {
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
    enabled: bool,
    tool_key: &str,
    should_wrap_cmd: bool,
) -> Result<(), String> {
    use toml_edit::Item;

    if field.contains('.') {
        return Err(format!(
            "Nested TOML MCP field paths are not supported: {}",
            field
        ));
    }

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
            content
                .parse::<toml_edit::DocumentMut>()
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
    let server_table = if tool_key == "grok" {
        build_grok_toml_server_config(server, enabled)?
    } else {
        build_toml_edit_server_config(server, should_wrap_cmd)?
    };

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
    if field.contains('.') {
        return Err(format!(
            "Nested TOML MCP field paths are not supported: {}",
            field
        ));
    }

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

/// Build TOML server configuration using toml_edit
fn build_toml_edit_server_config(
    server: &McpServer,
    should_wrap_cmd: bool,
) -> Result<toml_edit::Table, String> {
    use toml_edit::{Array, Item, Table};

    let mut t = Table::new();

    match server.server_type.as_str() {
        "stdio" => {
            let command = server
                .server_config
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("stdio server requires 'command' field")?;

            let args: Vec<String> = server
                .server_config
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let (final_command, final_args) = if should_wrap_cmd {
                use super::command_normalize;
                let temp_config = serde_json::json!({
                    "type": "stdio",
                    "command": command,
                    "args": args
                });
                let wrapped =
                    command_normalize::wrap_cmd_c_for_target(&temp_config, should_wrap_cmd);
                let cmd = wrapped
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or(command)
                    .to_string();
                let a: Vec<String> = wrapped
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or(args.clone());
                (cmd, a)
            } else {
                (command.to_string(), args)
            };

            // Codex/Kimi TOML schemas have no `type` key: Codex infers the
            // transport from `command`/`url` (RawMcpServerConfig), and Kimi
            // Code CLI discriminates on `transport` instead. Writing `type`
            // makes Codex report an unrecognized-setting warning.
            t["command"] = toml_edit::value(&final_command);

            // Build args array (inline format)
            if !final_args.is_empty() {
                let mut arr = Array::default();
                for a in &final_args {
                    arr.push(a.as_str());
                }
                t["args"] = Item::Value(toml_edit::Value::Array(arr));
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
            let url = server
                .server_config
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or(format!(
                    "{} server requires 'url' field",
                    server.server_type
                ))?;

            // See the stdio branch: no `type` key. Codex only has a
            // streamable_http transport inferred from `url` (no `sse`);
            // Kimi discriminates on `transport` and auto-detects `http`.
            t["url"] = toml_edit::value(url);

            // Build http_headers as sub-table (Codex uses http_headers, not headers)
            if let Some(headers) = server
                .server_config
                .get("headers")
                .and_then(|v| v.as_object())
            {
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

    // Codex supports the same second-based timeout keys as Grok.
    // Leave unset so Codex keeps its own defaults (startup 10s / tool 60s).
    copy_optional_toml_fields(
        &mut t,
        &server.server_config,
        &["startup_timeout_sec", "tool_timeout_sec"],
    )?;

    Ok(t)
}

/// Build the official Grok MCP schema. Grok does not use Codex's `type` or
/// `http_headers` fields and must keep npm commands unwrapped on every target.
fn build_grok_toml_server_config(
    server: &McpServer,
    enabled: bool,
) -> Result<toml_edit::Table, String> {
    use toml_edit::{Array, Item, Table};

    let mut table = Table::new();
    match server.server_type.as_str() {
        "stdio" => {
            let command = server
                .server_config
                .get("command")
                .and_then(Value::as_str)
                .ok_or("stdio server requires 'command' field")?;
            table["command"] = toml_edit::value(command);

            if let Some(args) = server.server_config.get("args").and_then(Value::as_array) {
                let mut array = Array::new();
                for argument in args.iter().filter_map(Value::as_str) {
                    array.push(argument);
                }
                table["args"] = Item::Value(toml_edit::Value::Array(array));
            }
            if let Some(env) = server.server_config.get("env").and_then(Value::as_object) {
                let mut env_table = Table::new();
                for (key, value) in env {
                    if let Some(value) = value.as_str() {
                        env_table[key] = toml_edit::value(value);
                    }
                }
                table["env"] = Item::Table(env_table);
            }
            copy_optional_toml_fields(
                &mut table,
                &server.server_config,
                &[
                    "cwd",
                    "startup_timeout_sec",
                    "tool_timeout_sec",
                    "tool_timeouts",
                ],
            )?;
        }
        "http" | "sse" => {
            let url = server
                .server_config
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{} server requires 'url' field", server.server_type))?;
            table["url"] = toml_edit::value(url);
            if let Some(headers) = server
                .server_config
                .get("headers")
                .and_then(Value::as_object)
            {
                let mut headers_table = Table::new();
                for (key, value) in headers {
                    if let Some(value) = value.as_str() {
                        headers_table[key] = toml_edit::value(value);
                    }
                }
                table["headers"] = Item::Table(headers_table);
            }
            copy_optional_toml_fields(
                &mut table,
                &server.server_config,
                &["bearer_token_env_var"],
            )?;
        }
        _ => return Err(format!("Unknown server type: {}", server.server_type)),
    }
    table["enabled"] = toml_edit::value(enabled);
    Ok(table)
}

fn copy_optional_toml_fields(
    table: &mut toml_edit::Table,
    config: &Value,
    field_names: &[&str],
) -> Result<(), String> {
    for field_name in field_names {
        if let Some(value) = config.get(*field_name).filter(|value| !value.is_null()) {
            table[field_name] = json_to_toml_item(value)?;
        }
    }
    Ok(())
}

fn json_to_toml_item(value: &Value) -> Result<toml_edit::Item, String> {
    let serialized = toml::to_string(&serde_json::json!({ "holder": value }))
        .map_err(|error| format!("Failed to serialize MCP TOML field: {error}"))?;
    let mut document = serialized
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("Failed to build MCP TOML field: {error}"))?;
    document
        .remove("holder")
        .ok_or_else(|| "Failed to build MCP TOML field".to_string())
}

/// Build JSON server configuration from McpServer
/// Applies format conversion if format_config is provided
fn build_json_server_config(
    server: &McpServer,
    format_config: Option<&McpFormatConfig>,
    enabled: bool,
    tool_key: &str,
    should_wrap_cmd: bool,
) -> Result<Value, String> {
    match server.server_type.as_str() {
        "stdio" => build_stdio_config(server, format_config, enabled, tool_key, should_wrap_cmd),
        "http" | "sse" => build_http_config(server, format_config, enabled, tool_key),
        _ => Err(format!("Unknown server type: {}", server.server_type)),
    }
}

fn detect_server_type_with_format_config(
    server_config: &Value,
    format_config: &McpFormatConfig,
) -> String {
    if let Some(tool_type) = server_config.get("type").and_then(|v| v.as_str()) {
        return format_config.map_type_from_tool(tool_type);
    }

    if server_config.get("command").is_some() {
        return "stdio".to_string();
    }

    if format_config.infer_remote_type_from_url_fields_when_type_missing {
        if let Some(server_type) = format_config.infer_remote_type_from_url_fields(server_config) {
            if server_type == "sse" {
                // When the tool omits `type`, treat a plain `url` as the generic HTTP fallback.
                return "http".to_string();
            }
            return server_type;
        }
        if ["httpUrl", "serverUrl", "url"]
            .iter()
            .any(|field| server_config.get(*field).is_some())
        {
            return "http".to_string();
        }
    }

    format_config.map_type_from_tool(format_config.default_tool_type)
}

fn extract_remote_url_with_format_config<'a>(
    server_config: &'a Value,
    format_config: &McpFormatConfig,
    server_type: &str,
) -> Option<&'a str> {
    let preferred_field = format_config.remote_url_field_for_type(server_type);

    if let Some(url) = server_config.get(preferred_field).and_then(|v| v.as_str()) {
        return Some(url);
    }

    if matches!(server_type, "http" | "sse") && preferred_field != "url" {
        for fallback_field in ["httpUrl", "serverUrl", "url"] {
            if fallback_field == preferred_field {
                continue;
            }
            if let Some(url) = server_config.get(fallback_field).and_then(|v| v.as_str()) {
                return Some(url);
            }
        }
    }

    None
}

/// Build stdio server configuration
fn build_stdio_config(
    server: &McpServer,
    format_config: Option<&McpFormatConfig>,
    enabled: bool,
    tool_key: &str,
    should_wrap_cmd: bool,
) -> Result<Value, String> {
    let command = server
        .server_config
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("stdio server requires 'command' field")?;

    let args: Vec<String> = server
        .server_config
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let env = server.server_config.get("env").cloned();

    if tool_key == "openclaw" {
        let mut result = server
            .server_config
            .as_object()
            .cloned()
            .unwrap_or_default();

        result.insert("type".to_string(), Value::String("stdio".to_string()));
        result.insert("command".to_string(), Value::String(command.to_string()));
        result.insert(
            "args".to_string(),
            Value::Array(args.into_iter().map(Value::String).collect()),
        );

        if let Some(env_val) = env {
            if env_val.is_object() && !env_val.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                result.insert("env".to_string(), env_val);
            } else {
                result.remove("env");
            }
        } else {
            result.remove("env");
        }

        return Ok(Value::Object(result));
    }

    if tool_key == "kimi" {
        return build_kimi_stdio_config(server, command, args, env, enabled, should_wrap_cmd);
    }

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

            let command_array = command_normalize::wrap_cmd_c_opencode_array_for_target(
                &command_array,
                should_wrap_cmd,
            );
            result.insert("command".to_string(), Value::Array(command_array));
        } else {
            // Standard command + args format with format_config
            // Build result first, then wrap for Windows
            let temp_result = serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
            });
            let temp_result =
                command_normalize::wrap_cmd_c_for_target(&temp_result, should_wrap_cmd);

            // Extract wrapped command and args
            let final_command = temp_result
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or(command);
            let final_args = temp_result
                .get("args")
                .cloned()
                .unwrap_or(Value::Array(vec![]));

            result.insert(
                "command".to_string(),
                Value::String(final_command.to_string()),
            );
            result.insert("args".to_string(), final_args);
        }

        // Add environment variables with the correct field name
        if let Some(env_val) = env {
            if env_val.is_object() && !env_val.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                result.insert(config.env_field.to_string(), env_val);
            }
        }

        // Add enabled field if required
        if config.requires_enabled {
            result.insert("enabled".to_string(), Value::Bool(enabled));
        }

        // Add timeout field if supported
        if config.supports_timeout {
            if let Some(timeout) = server.timeout {
                result.insert("timeout".to_string(), Value::Number(timeout.into()));
            }
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

        let result = command_normalize::wrap_cmd_c_for_target(&result, should_wrap_cmd);

        Ok(result)
    }
}

/// Build HTTP/SSE server configuration
fn build_http_config(
    server: &McpServer,
    format_config: Option<&McpFormatConfig>,
    enabled: bool,
    tool_key: &str,
) -> Result<Value, String> {
    let url = server
        .server_config
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or(format!(
            "{} server requires 'url' field",
            server.server_type
        ))?;

    let headers = server.server_config.get("headers").cloned();

    if tool_key == "openclaw" {
        let mut result = server
            .server_config
            .as_object()
            .cloned()
            .unwrap_or_default();

        result.insert(
            "type".to_string(),
            Value::String(server.server_type.clone()),
        );
        result.insert("url".to_string(), Value::String(url.to_string()));

        if let Some(headers_val) = headers {
            if headers_val.is_object()
                && !headers_val
                    .as_object()
                    .map(|o| o.is_empty())
                    .unwrap_or(true)
            {
                result.insert("headers".to_string(), headers_val);
            } else {
                result.remove("headers");
            }
        } else {
            result.remove("headers");
        }

        return Ok(Value::Object(result));
    }

    if tool_key == "kimi" {
        return build_kimi_http_config(server, url, headers, enabled);
    }

    // Apply format conversion if config is provided
    if let Some(config) = format_config {
        let mut result = serde_json::Map::new();

        // Map server type
        let mapped_type = config.map_type_to_tool(&server.server_type);
        result.insert("type".to_string(), Value::String(mapped_type.to_string()));
        let url_field = config.remote_url_field_for_type(&server.server_type);
        result.insert(url_field.to_string(), Value::String(url.to_string()));

        if let Some(headers_val) = headers {
            if headers_val.is_object()
                && !headers_val
                    .as_object()
                    .map(|o| o.is_empty())
                    .unwrap_or(true)
            {
                result.insert("headers".to_string(), headers_val);
            }
        }

        // Add enabled field if required
        if config.requires_enabled {
            result.insert("enabled".to_string(), Value::Bool(enabled));
        }

        // Add timeout field if supported
        if config.supports_timeout {
            if let Some(timeout) = server.timeout {
                result.insert("timeout".to_string(), Value::Number(timeout.into()));
            }
        }

        Ok(Value::Object(result))
    } else {
        // Standard format (Claude Code, Gemini CLI, etc.)
        let mut result = serde_json::json!({
            "type": &server.server_type,
            "url": url,
        });

        if let Some(headers_val) = headers {
            if headers_val.is_object()
                && !headers_val
                    .as_object()
                    .map(|o| o.is_empty())
                    .unwrap_or(true)
            {
                result["headers"] = headers_val;
            }
        }

        Ok(result)
    }
}

/// Kimi Code CLI uses millisecond timeout fields (`startupTimeoutMs` /
/// `toolTimeoutMs`, clamped to 1..=2147483647); the central store keeps the
/// Codex/Grok-style second-based `startup_timeout_sec` / `tool_timeout_sec`.
const KIMI_MAX_TIMEOUT_MS: i64 = 2_147_483_647;

fn kimi_timeout_ms_value_from_seconds(config: &Value, field: &str) -> Option<Value> {
    let seconds = config.get(field)?.as_f64()?;
    let millis = (seconds * 1000.0).round() as i64;
    if !(1..=KIMI_MAX_TIMEOUT_MS).contains(&millis) {
        return None;
    }
    Some(Value::Number(millis.into()))
}

/// Inverse of [`kimi_timeout_ms_value_from_seconds`]: convert a Kimi
/// millisecond value back into the central second-based field. Whole seconds
/// stay integers so round trips keep the stored shape stable.
fn kimi_seconds_value_from_ms(ms: f64) -> Value {
    if ms % 1000.0 == 0.0 {
        Value::from((ms / 1000.0) as i64)
    } else {
        Value::from(ms / 1000.0)
    }
}

/// Build a Kimi Code CLI stdio entry for `<root>/mcp.json`.
///
/// Kimi's schema infers `transport: "stdio"` from the `command` field, so the
/// discriminator is omitted. `enabled` and the millisecond timeouts are written
/// explicitly; `cwd` is passed through when the central config carries one.
fn build_kimi_stdio_config(
    server: &McpServer,
    command: &str,
    args: Vec<String>,
    env: Option<Value>,
    enabled: bool,
    should_wrap_cmd: bool,
) -> Result<Value, String> {
    let temp_result = serde_json::json!({
        "command": command,
        "args": args,
    });
    let wrapped = command_normalize::wrap_cmd_c_for_target(&temp_result, should_wrap_cmd);

    let mut result = serde_json::Map::new();
    result.insert(
        "command".to_string(),
        wrapped
            .get("command")
            .cloned()
            .unwrap_or(Value::String(command.to_string())),
    );
    result.insert(
        "args".to_string(),
        wrapped
            .get("args")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    );

    if let Some(env_val) = env {
        if env_val.is_object() && !env_val.as_object().is_some_and(|o| o.is_empty()) {
            result.insert("env".to_string(), env_val);
        }
    }

    if let Some(cwd) = server
        .server_config
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.is_empty())
    {
        result.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }

    result.insert("enabled".to_string(), Value::Bool(enabled));
    for (target, source) in [
        ("startupTimeoutMs", "startup_timeout_sec"),
        ("toolTimeoutMs", "tool_timeout_sec"),
    ] {
        if let Some(ms) = kimi_timeout_ms_value_from_seconds(&server.server_config, source) {
            result.insert(target.to_string(), ms);
        }
    }

    Ok(Value::Object(result))
}

/// Build a Kimi Code CLI HTTP/SSE entry for `<root>/mcp.json`.
///
/// Kimi infers `transport: "http"` from `url`, but has no `url` -> `sse`
/// inference, so SSE entries must carry the explicit discriminator.
fn build_kimi_http_config(
    server: &McpServer,
    url: &str,
    headers: Option<Value>,
    enabled: bool,
) -> Result<Value, String> {
    let mut result = serde_json::Map::new();
    if server.server_type == "sse" {
        result.insert("transport".to_string(), Value::String("sse".to_string()));
    }
    result.insert("url".to_string(), Value::String(url.to_string()));

    if let Some(headers_val) = headers {
        if headers_val.is_object() && !headers_val.as_object().is_some_and(|o| o.is_empty()) {
            result.insert("headers".to_string(), headers_val);
        }
    }

    if let Some(bearer_env) = server
        .server_config
        .get("bearer_token_env_var")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    {
        result.insert(
            "bearerTokenEnvVar".to_string(),
            Value::String(bearer_env.to_string()),
        );
    }

    result.insert("enabled".to_string(), Value::Bool(enabled));
    for (target, source) in [
        ("startupTimeoutMs", "startup_timeout_sec"),
        ("toolTimeoutMs", "tool_timeout_sec"),
    ] {
        if let Some(ms) = kimi_timeout_ms_value_from_seconds(&server.server_config, source) {
            result.insert(target.to_string(), ms);
        }
    }

    Ok(Value::Object(result))
}

/// Import MCP servers from a tool's config file
pub fn import_servers_from_tool(
    db: &crate::db::SqliteDbState,
    tool: &RuntimeTool,
) -> Result<Vec<McpServer>, String> {
    let config_path = resolve_mcp_config_path_with_db(db, tool)
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;
    import_servers_from_path(tool, &config_path)
}

pub async fn import_servers_from_tool_async(
    db: &crate::db::SqliteDbState,
    tool: &RuntimeTool,
) -> Result<Vec<McpServer>, String> {
    let config_path = resolve_mcp_config_path_with_db_async(db, tool)
        .await
        .ok_or_else(|| format!("Tool {} does not support MCP", tool.key))?;
    import_servers_from_path(tool, &config_path)
}

pub(crate) fn import_servers_from_path(
    tool: &RuntimeTool,
    config_path: &PathBuf,
) -> Result<Vec<McpServer>, String> {
    if !config_path.exists() {
        return Ok(vec![]);
    }

    let format = tool.mcp_config_format.as_deref().unwrap_or("json");
    let field = tool.mcp_field.as_deref().unwrap_or("mcpServers");
    let format_config = get_format_config(&tool.key);

    match format {
        // json5 handles both standard JSON and JSONC (with comments, trailing commas)
        "json" | "jsonc" if tool.key == "kimi" => import_servers_from_kimi(config_path),
        "json" | "jsonc" => import_servers_from_json(config_path, field, format_config),
        "toml" => import_servers_from_toml(config_path, field, &tool.key),
        "yaml" => match tool.key.as_str() {
            "hermes" => super::hermes_mcp::import_servers_from_hermes(config_path),
            _ => Ok(vec![]),
        },
        "cordis" => match tool.key.as_str() {
            "dsh" => super::cordis_patch::import_servers_from_cordis(config_path),
            _ => Ok(vec![]),
        },
        _ => Err(format!("Unsupported config format: {}", format)),
    }
}

/// Import MCP servers from Kimi Code CLI's `<root>/mcp.json`.
///
/// Differences from the standard Claude-style parser:
/// - the transport discriminator is `transport` (with `command`/`url` fallback
///   inference), not `type`;
/// - millisecond timeouts (`startupTimeoutMs`/`toolTimeoutMs`) convert back to
///   the central second-based fields;
/// - `enabled`, `cwd`, and `bearerTokenEnvVar` are kept in `server_config` so
///   re-syncing does not drop them.
fn import_servers_from_kimi(config_path: &PathBuf) -> Result<Vec<McpServer>, String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    let content = content.trim();
    if content.is_empty() {
        return Ok(vec![]);
    }
    let config: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse config file: {}", e))?;

    let Some(servers_obj) = config.get("mcpServers").and_then(Value::as_object) else {
        return Ok(vec![]);
    };

    let now = now_ms();
    let mut servers = Vec::new();

    for (name, entry) in servers_obj {
        let Some(server) = parse_kimi_server_config(name, entry, now) else {
            continue;
        };
        servers.push(server);
    }

    Ok(servers)
}

fn kimi_transport_to_server_type(entry: &Value) -> Option<String> {
    if let Some(transport) = entry.get("transport").and_then(Value::as_str) {
        return match transport {
            "stdio" | "http" | "sse" => Some(transport.to_string()),
            _ => None,
        };
    }
    if entry.get("command").is_some() {
        return Some("stdio".to_string());
    }
    if entry.get("url").is_some() {
        return Some("http".to_string());
    }
    None
}

fn kimi_copy_seconds_timeouts(entry: &Value, unified: &mut serde_json::Map<String, Value>) {
    for (target, source) in [
        ("startup_timeout_sec", "startupTimeoutMs"),
        ("tool_timeout_sec", "toolTimeoutMs"),
    ] {
        if let Some(ms) = entry.get(source).and_then(Value::as_f64) {
            unified.insert(target.to_string(), kimi_seconds_value_from_ms(ms));
        }
    }
}

fn parse_kimi_server_config(name: &str, entry: &Value, now: i64) -> Option<McpServer> {
    let server_type = kimi_transport_to_server_type(entry)?;

    let mut unified_config = serde_json::Map::new();
    match server_type.as_str() {
        "stdio" => {
            let command = entry.get("command").and_then(Value::as_str)?;
            unified_config.insert("command".to_string(), Value::String(command.to_string()));
            if let Some(args) = entry.get("args").and_then(Value::as_array) {
                if !args.is_empty() {
                    unified_config.insert("args".to_string(), Value::Array(args.clone()));
                }
            }
            if let Some(env_val) = entry.get("env") {
                if env_val.as_object().is_some_and(|o| !o.is_empty()) {
                    unified_config.insert("env".to_string(), env_val.clone());
                }
            }
            if let Some(cwd) = entry.get("cwd").and_then(Value::as_str) {
                if !cwd.is_empty() {
                    unified_config.insert("cwd".to_string(), Value::String(cwd.to_string()));
                }
            }
            // Normalize for database storage (strip any cmd /c wrapper).
            let Value::Object(unwrapped) =
                command_normalize::unwrap_cmd_c(&Value::Object(unified_config))
            else {
                return None;
            };
            unified_config = unwrapped;
        }
        "http" | "sse" => {
            let url = entry.get("url").and_then(Value::as_str)?;
            unified_config.insert("url".to_string(), Value::String(url.to_string()));
            if let Some(headers_val) = entry.get("headers") {
                if headers_val.as_object().is_some_and(|o| !o.is_empty()) {
                    unified_config.insert("headers".to_string(), headers_val.clone());
                }
            }
            if let Some(bearer_env) = entry.get("bearerTokenEnvVar").and_then(Value::as_str) {
                if !bearer_env.is_empty() {
                    unified_config.insert(
                        "bearer_token_env_var".to_string(),
                        Value::String(bearer_env.to_string()),
                    );
                }
            }
        }
        _ => return None,
    }

    if let Some(enabled) = entry.get("enabled").and_then(Value::as_bool) {
        unified_config.insert("enabled".to_string(), Value::Bool(enabled));
    }
    kimi_copy_seconds_timeouts(entry, &mut unified_config);

    Some(McpServer {
        id: String::new(),
        name: name.to_string(),
        server_type,
        server_config: Value::Object(unified_config),
        enabled_tools: vec![],
        sync_details: None,
        description: None,
        user_group: None,
        user_note: None,
        tags: vec![],
        timeout: None,
        sort_index: 0,
        management_enabled: true,
        disabled_previous_tools: Vec::new(),
        created_at: now,
        updated_at: now,
    })
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
    let config: Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse config file: {}", e))?;

    parse_mcp_servers_from_value(&config, field, format_config)
}

/// Parse MCP servers from a JSON Value
fn parse_mcp_servers_from_value(
    config: &Value,
    field: &str,
    format_config: Option<&McpFormatConfig>,
) -> Result<Vec<McpServer>, String> {
    let Some(mcp_servers) = get_json_value_by_path(config, field) else {
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
    let server_type = detect_server_type_with_format_config(server_config, format_config);

    // Build unified server_config
    let unified_config = if server_type == "stdio" {
        // Handle command array -> command + args conversion
        let command_val = server_config.get("command")?;

        let (command, args) = if format_config.merge_command_args {
            // Command is an array: ["npx", "-y", "pkg"] or ["cmd", "/c", "npx", "-y", "pkg"]
            if let Some(arr) = command_val.as_array() {
                if arr.is_empty() {
                    return None;
                }
                // Unwrap cmd /c if present (for OpenCode array format)
                let unwrapped = command_normalize::unwrap_cmd_c_opencode_array(arr);
                let cmd = unwrapped.first()?.as_str()?.to_string();
                let args: Vec<Value> = unwrapped[1..].iter().cloned().collect();
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
            let args = server_config
                .get("args")
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

        // Unwrap cmd /c for import (normalize for database storage)
        command_normalize::unwrap_cmd_c(&result)
    } else {
        // HTTP/SSE type
        let url =
            extract_remote_url_with_format_config(server_config, format_config, &server_type)?;
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
        user_group: None,
        user_note: None,
        tags: vec![],
        timeout: None,
        sort_index: 0,
        management_enabled: true,
        disabled_previous_tools: Vec::new(),
        created_at: now,
        updated_at: now,
    })
}

/// Parse standard server config (no format conversion needed)
/// Used by Claude Code, Gemini CLI, etc.
fn parse_standard_server_config(name: &str, server_config: &Value, now: i64) -> Option<McpServer> {
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
                "stdio" // Default to stdio
            }
        });

    // Unwrap cmd /c for import (normalize for database storage)
    let normalized_config = if server_type == "stdio" {
        command_normalize::unwrap_cmd_c(server_config)
    } else {
        server_config.clone()
    };

    Some(McpServer {
        id: String::new(),
        name: name.to_string(),
        server_type: server_type.to_string(),
        server_config: normalized_config,
        enabled_tools: vec![],
        sync_details: None,
        description: None,
        user_group: None,
        user_note: None,
        tags: vec![],
        timeout: None,
        sort_index: 0,
        management_enabled: true,
        disabled_previous_tools: Vec::new(),
        created_at: now,
        updated_at: now,
    })
}

/// Import MCP servers from a Claude Code plugin's `.mcp.json` file.
///
/// Plugin `.mcp.json` uses a flat format: `{ "server-name": { "type": "http", "url": "..." } }`
/// i.e. the root object IS the mcpServers map (no wrapper field).
pub fn import_servers_from_plugin_mcp_json(
    path: &std::path::Path,
) -> Result<Vec<McpServer>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read plugin .mcp.json: {}", e))?;
    let content = content.trim();
    if content.is_empty() {
        return Ok(vec![]);
    }

    let root: serde_json::Value =
        json5::from_str(content).map_err(|e| format!("Failed to parse plugin .mcp.json: {}", e))?;

    let Some(obj) = root.as_object() else {
        return Ok(vec![]);
    };

    let now = now_ms();
    let mut servers = Vec::new();

    for (name, server_config) in obj {
        // Reuse the standard parser (same format as Claude Code mcpServers entries)
        if let Some(server) = parse_standard_server_config(name, server_config, now) {
            servers.push(server);
        }
    }

    Ok(servers)
}

/// Import servers from TOML config file
fn import_servers_from_toml(
    config_path: &PathBuf,
    field: &str,
    tool_key: &str,
) -> Result<Vec<McpServer>, String> {
    if field.contains('.') {
        return Err(format!(
            "Nested TOML MCP field paths are not supported: {}",
            field
        ));
    }

    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    let content_trimmed = content.trim();
    if content_trimmed.is_empty() {
        return Ok(vec![]);
    }
    let config: toml::Table = content_trimmed
        .parse()
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

        // Grok intentionally omits `type`; command/url is the authoritative discriminator.
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
                    let arr: Vec<Value> = args
                        .iter()
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
                if tool_key == "grok" {
                    copy_optional_import_fields(
                        config_table,
                        &mut json_config,
                        &[
                            "cwd",
                            "enabled",
                            "startup_timeout_sec",
                            "tool_timeout_sec",
                            "tool_timeouts",
                        ],
                    );
                }
            }
            "http" | "sse" => {
                if let Some(url) = config_table.get("url").and_then(|v| v.as_str()) {
                    json_config.insert("url".into(), Value::String(url.to_string()));
                }
                let headers_tbl = if tool_key == "grok" {
                    config_table.get("headers").and_then(|v| v.as_table())
                } else {
                    config_table
                        .get("http_headers")
                        .and_then(|v| v.as_table())
                        .or_else(|| config_table.get("headers").and_then(|v| v.as_table()))
                };
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
                if tool_key == "grok" {
                    copy_optional_import_fields(
                        config_table,
                        &mut json_config,
                        &["bearer_token_env_var", "enabled"],
                    );
                }
            }
            _ => continue,
        }

        // Codex keeps second-based timeouts in server_config so re-sync does not drop them.
        if tool_key == "codex" {
            copy_optional_import_fields(
                config_table,
                &mut json_config,
                &["startup_timeout_sec", "tool_timeout_sec"],
            );
        }

        // Unwrap cmd /c for import (normalize for database storage)
        let normalized_config = if server_type == "stdio" {
            command_normalize::unwrap_cmd_c(&Value::Object(json_config))
        } else {
            Value::Object(json_config)
        };

        servers.push(McpServer {
            id: String::new(),
            name: name.clone(),
            server_type: server_type.to_string(),
            server_config: normalized_config,
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            user_group: None,
            user_note: None,
            tags: vec![],
            timeout: None,
            sort_index: 0,
            management_enabled: true,
            disabled_previous_tools: Vec::new(),
            created_at: now,
            updated_at: now,
        });
    }

    Ok(servers)
}

fn copy_optional_import_fields(
    source: &toml::Table,
    target: &mut serde_json::Map<String, Value>,
    field_names: &[&str],
) {
    for field_name in field_names {
        if let Some(value) = source.get(*field_name) {
            if let Ok(value) = serde_json::to_value(value) {
                target.insert((*field_name).to_string(), value);
            }
        }
    }
}

fn split_field_path(field: &str) -> Vec<&str> {
    field
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn get_json_value_by_path<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    let path = split_field_path(field);
    if path.is_empty() {
        return Some(value);
    }

    let mut current = value;
    for segment in path {
        current = current.get(segment)?;
    }
    Some(current)
}

fn get_json_value_by_path_mut<'a>(value: &'a mut Value, field: &str) -> Option<&'a mut Value> {
    let path = split_field_path(field);
    if path.is_empty() {
        return Some(value);
    }

    let mut current = value;
    for segment in path {
        current = current.get_mut(segment)?;
    }
    Some(current)
}

fn ensure_json_object_path<'a>(value: &'a mut Value, field: &str) -> Result<&'a mut Value, String> {
    let path = split_field_path(field);
    if path.is_empty() {
        return Ok(value);
    }

    let mut current = value;
    for segment in path {
        let object = current
            .as_object_mut()
            .ok_or_else(|| format!("{} is not a JSON object", segment))?;
        current = object
            .entry(segment.to_string())
            .or_insert_with(|| serde_json::json!({}));
    }

    if !current.is_object() {
        return Err(format!("{} is not a JSON object", field));
    }

    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::mcp::format_configs::get_format_config;
    use serde_json::json;

    fn build_openclaw_stdio_server() -> McpServer {
        McpServer {
            id: String::new(),
            name: "gemini".to_string(),
            server_type: "stdio".to_string(),
            server_config: json!({
                "command": "node",
                "args": ["server.js"],
            }),
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            user_group: None,
            user_note: None,
            tags: vec![],
            timeout: None,
            sort_index: 0,
            management_enabled: true,
            disabled_previous_tools: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    fn build_http_server() -> McpServer {
        McpServer {
            id: String::new(),
            name: "remote".to_string(),
            server_type: "http".to_string(),
            server_config: json!({
                "url": "https://example.com/mcp",
                "headers": {
                    "Authorization": "Bearer token"
                }
            }),
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            user_group: None,
            user_note: None,
            tags: vec![],
            timeout: None,
            sort_index: 0,
            management_enabled: true,
            disabled_previous_tools: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    fn build_npx_stdio_server() -> McpServer {
        McpServer {
            id: String::new(),
            name: "fast-context".to_string(),
            server_type: "stdio".to_string(),
            server_config: json!({
                "command": "npx",
                "args": ["-y", "--prefer-online", "@sammysnake/fast-context-mcp"],
            }),
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            user_group: None,
            user_note: None,
            tags: vec![],
            timeout: None,
            sort_index: 0,
            management_enabled: true,
            disabled_previous_tools: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn build_openclaw_stdio_config_keeps_type_field() {
        let server = build_openclaw_stdio_server();

        let config = build_json_server_config(&server, None, true, "openclaw", false).unwrap();

        assert_eq!(config["type"], "stdio");
        assert_eq!(config["command"], "node");
        assert_eq!(config["args"], json!(["server.js"]));
    }

    #[test]
    fn windows_config_path_wrap_check_excludes_wsl_unc_paths() {
        assert!(!should_wrap_cmd_for_windows_config_path(Path::new(
            r"\\wsl.localhost\Arch\home\tester\.codex\config.toml"
        )));
        assert!(!should_wrap_cmd_for_windows_config_path(Path::new(
            r"\\wsl$\Ubuntu\home\tester\.claude.json"
        )));
        assert!(should_wrap_cmd_for_windows_config_path(Path::new(
            r"C:\Users\tester\.codex\config.toml"
        )));
    }

    #[test]
    fn codex_toml_config_skips_cmd_wrapper_for_wsl_target() {
        let server = build_npx_stdio_server();

        let table = build_toml_edit_server_config(&server, false).unwrap();

        assert_eq!(table["command"].as_str(), Some("npx"));
        assert_eq!(
            table["args"].as_array().map(|args| args
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()),
            Some(vec![
                "-y",
                "--prefer-online",
                "@sammysnake/fast-context-mcp",
            ])
        );
    }

    #[test]
    fn codex_toml_config_wraps_cmd_for_windows_target() {
        let server = build_npx_stdio_server();

        let table = build_toml_edit_server_config(&server, true).unwrap();

        assert_eq!(table["command"].as_str(), Some("cmd"));
        assert_eq!(
            table["args"].as_array().map(|args| args
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()),
            Some(vec![
                "/c",
                "npx",
                "-y",
                "--prefer-online",
                "@sammysnake/fast-context-mcp",
            ])
        );
    }

    #[test]
    fn codex_toml_config_writes_timeout_fields() {
        let mut server = build_npx_stdio_server();
        server.server_config["startup_timeout_sec"] = json!(120);
        server.server_config["tool_timeout_sec"] = json!(300);

        let table = build_toml_edit_server_config(&server, false).expect("build Codex MCP");

        assert!(table.get("type").is_none());
        assert_eq!(table["command"].as_str(), Some("npx"));
        assert_eq!(table["startup_timeout_sec"].as_integer(), Some(120));
        assert_eq!(table["tool_timeout_sec"].as_integer(), Some(300));
    }

    #[test]
    fn codex_toml_config_omits_timeout_fields_when_unset() {
        let server = build_npx_stdio_server();

        let table = build_toml_edit_server_config(&server, false).expect("build Codex MCP");

        assert!(table.get("startup_timeout_sec").is_none());
        assert!(table.get("tool_timeout_sec").is_none());
    }

    #[test]
    fn codex_toml_import_preserves_timeout_fields() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[mcp_servers.local]
type = "stdio"
command = "npx"
args = ["-y", "pkg"]
startup_timeout_sec = 120
tool_timeout_sec = 300

[mcp_servers.remote]
type = "http"
url = "https://example.com/mcp"
startup_timeout_sec = 25
tool_timeout_sec = 1800

[mcp_servers.remote.http_headers]
X-Test = "yes"
"#,
        )
        .expect("write fixture");

        let servers = import_servers_from_toml(&config_path, "mcp_servers", "codex")
            .expect("import Codex MCP");
        let local = servers
            .iter()
            .find(|server| server.name == "local")
            .expect("local server");
        assert_eq!(local.server_config["command"], "npx");
        assert_eq!(local.server_config["startup_timeout_sec"], 120);
        assert_eq!(local.server_config["tool_timeout_sec"], 300);
        let remote = servers
            .iter()
            .find(|server| server.name == "remote")
            .expect("remote server");
        assert_eq!(remote.server_config["url"], "https://example.com/mcp");
        assert_eq!(remote.server_config["headers"]["X-Test"], "yes");
        assert_eq!(remote.server_config["startup_timeout_sec"], 25);
        assert_eq!(remote.server_config["tool_timeout_sec"], 1800);
    }

    #[test]
    fn kimi_json_config_writes_official_fields_without_type() {
        let mut server = build_npx_stdio_server();
        server.server_config["env"] = json!({ "API_KEY": "k-123" });
        server.server_config["cwd"] = json!("/workspace");
        server.server_config["startup_timeout_sec"] = json!(120);
        server.server_config["tool_timeout_sec"] = json!(0.5);

        let config =
            build_json_server_config(&server, None, false, "kimi", false).expect("build Kimi MCP");

        // Kimi's schema has no `type` key and infers stdio from `command`.
        assert!(config.get("type").is_none());
        assert!(config.get("transport").is_none());
        assert_eq!(config["command"], "npx");
        assert_eq!(
            config["args"],
            json!(["-y", "--prefer-online", "@sammysnake/fast-context-mcp"])
        );
        assert_eq!(config["env"], json!({ "API_KEY": "k-123" }));
        assert_eq!(config["cwd"], "/workspace");
        assert_eq!(config["enabled"], false);
        // Central second-based timeouts convert to Kimi's millisecond fields.
        assert_eq!(config["startupTimeoutMs"], 120_000);
        assert_eq!(config["toolTimeoutMs"], 500);
    }

    #[test]
    fn kimi_json_http_omits_transport_but_sse_keeps_it() {
        let mut http = build_http_server();
        http.server_config["startup_timeout_sec"] = json!(30);

        let http_config =
            build_json_server_config(&http, None, true, "kimi", false).expect("build Kimi HTTP");

        // `url` alone already implies `transport: "http"`.
        assert!(http_config.get("transport").is_none());
        assert_eq!(http_config["url"], "https://example.com/mcp");
        assert_eq!(http_config["enabled"], true);
        assert_eq!(http_config["startupTimeoutMs"], 30_000);

        let mut sse = build_http_server();
        sse.server_type = "sse".to_string();
        let sse_config =
            build_json_server_config(&sse, None, true, "kimi", false).expect("build Kimi SSE");

        // Kimi has no url -> sse inference; the discriminator is required.
        assert_eq!(sse_config["transport"], "sse");
        assert_eq!(sse_config["url"], "https://example.com/mcp");
    }

    #[test]
    fn kimi_json_import_converts_transport_and_ms_timeouts() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("mcp.json");
        std::fs::write(
            &config_path,
            r#"
{
  "mcpServers": {
    "local": {
      "command": "cmd",
      "args": ["/c", "npx", "-y", "pkg"],
      "env": { "API_KEY": "k-123" },
      "cwd": "/workspace",
      "enabled": false,
      "startupTimeoutMs": 120000,
      "toolTimeoutMs": 12500
    },
    "remote": {
      "transport": "sse",
      "url": "https://example.com/mcp",
      "headers": { "X-Test": "yes" },
      "bearerTokenEnvVar": "MY_TOKEN_VAR"
    }
  }
}
"#,
        )
        .expect("write fixture");

        let servers = import_servers_from_kimi(&config_path).expect("import Kimi MCP");
        let local = servers
            .iter()
            .find(|server| server.name == "local")
            .expect("local server");
        assert_eq!(local.server_type, "stdio");
        assert_eq!(local.server_config["command"], "npx");
        assert_eq!(local.server_config["env"], json!({ "API_KEY": "k-123" }));
        assert_eq!(local.server_config["cwd"], "/workspace");
        assert_eq!(local.server_config["enabled"], false);
        assert_eq!(local.server_config["startup_timeout_sec"], 120);
        assert_eq!(local.server_config["tool_timeout_sec"], 12.5);

        let remote = servers
            .iter()
            .find(|server| server.name == "remote")
            .expect("remote server");
        assert_eq!(remote.server_type, "sse");
        assert_eq!(remote.server_config["url"], "https://example.com/mcp");
        assert_eq!(remote.server_config["headers"], json!({ "X-Test": "yes" }));
        assert_eq!(remote.server_config["bearer_token_env_var"], "MY_TOKEN_VAR");
    }

    #[test]
    fn kimi_cleanup_removes_legacy_toml_mcp_table() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mcp_path = temp_dir.path().join("mcp.json");
        let config_toml_path = temp_dir.path().join("config.toml");
        std::fs::write(
            &config_toml_path,
            r#"
model = "kimi-for-coding"

[mcp_servers.old-server]
command = "npx"
args = ["-y", "pkg"]

[providers.official]
api_key = "sk-test"
"#,
        )
        .expect("write fixture");

        remove_legacy_kimi_toml_mcp_table(&mcp_path);

        let cleaned = std::fs::read_to_string(&config_toml_path).expect("read cleaned config");
        assert!(!cleaned.contains("mcp_servers"));
        assert!(cleaned.contains("[providers.official]"));
        assert!(cleaned.contains("model = \"kimi-for-coding\""));
        // Idempotent: second run is a no-op.
        remove_legacy_kimi_toml_mcp_table(&mcp_path);
        assert_eq!(
            std::fs::read_to_string(&config_toml_path).expect("read cleaned config"),
            cleaned
        );
    }

    #[test]
    fn grok_toml_config_uses_official_fields_without_cmd_wrapper() {
        let mut server = build_npx_stdio_server();
        server.server_config["cwd"] = json!("/workspace");
        server.server_config["startup_timeout_sec"] = json!(30);
        server.server_config["tool_timeout_sec"] = json!(6000);
        server.server_config["tool_timeouts"] = json!({ "search": 45 });

        let table = build_grok_toml_server_config(&server, false).expect("build Grok MCP");

        assert!(table.get("type").is_none());
        assert_eq!(table["command"].as_str(), Some("npx"));
        assert_eq!(table["cwd"].as_str(), Some("/workspace"));
        assert_eq!(table["enabled"].as_bool(), Some(false));
        assert_eq!(table["startup_timeout_sec"].as_integer(), Some(30));
        assert_eq!(table["tool_timeout_sec"].as_integer(), Some(6000));
        assert_eq!(table["tool_timeouts"]["search"].as_integer(), Some(45));
    }

    #[test]
    fn grok_toml_import_preserves_remote_and_timeout_fields() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[mcp_servers.local]
command = "npx"
args = ["-y", "pkg"]
cwd = "/workspace"
enabled = false
startup_timeout_sec = 30
tool_timeout_sec = 6000

[mcp_servers.local.tool_timeouts]
search = 45

[mcp_servers.remote]
url = "https://example.com/mcp"
enabled = true
bearer_token_env_var = "MCP_TOKEN"

[mcp_servers.remote.headers]
X-Test = "yes"
"#,
        )
        .expect("write fixture");

        let servers =
            import_servers_from_toml(&config_path, "mcp_servers", "grok").expect("import Grok MCP");
        let local = servers
            .iter()
            .find(|server| server.name == "local")
            .expect("local server");
        assert_eq!(local.server_config["command"], "npx");
        assert_eq!(local.server_config["cwd"], "/workspace");
        assert_eq!(local.server_config["enabled"], false);
        assert_eq!(local.server_config["tool_timeouts"]["search"], 45);
        let remote = servers
            .iter()
            .find(|server| server.name == "remote")
            .expect("remote server");
        assert_eq!(remote.server_config["headers"]["X-Test"], "yes");
        assert_eq!(remote.server_config["bearer_token_env_var"], "MCP_TOKEN");
    }

    #[test]
    fn standard_json_config_skips_cmd_wrapper_for_wsl_target() {
        let server = build_npx_stdio_server();

        let config = build_json_server_config(&server, None, true, "claude_code", false).unwrap();

        assert_eq!(config["command"], "npx");
        assert_eq!(
            config["args"],
            json!(["-y", "--prefer-online", "@sammysnake/fast-context-mcp"])
        );
    }

    #[test]
    fn standard_json_config_wraps_cmd_for_windows_target() {
        let server = build_npx_stdio_server();

        let config = build_json_server_config(&server, None, true, "claude_code", true).unwrap();

        assert_eq!(config["command"], "cmd");
        assert_eq!(
            config["args"],
            json!([
                "/c",
                "npx",
                "-y",
                "--prefer-online",
                "@sammysnake/fast-context-mcp"
            ])
        );
    }

    #[test]
    fn opencode_array_config_skips_cmd_wrapper_for_wsl_target() {
        let server = build_npx_stdio_server();
        let format = get_format_config("opencode").expect("opencode format should exist");

        let config =
            build_json_server_config(&server, Some(format), true, "opencode", false).unwrap();

        assert_eq!(
            config["command"],
            json!([
                "npx",
                "-y",
                "--prefer-online",
                "@sammysnake/fast-context-mcp"
            ])
        );
    }

    #[test]
    fn opencode_array_config_wraps_cmd_for_windows_target() {
        let server = build_npx_stdio_server();
        let format = get_format_config("opencode").expect("opencode format should exist");

        let config =
            build_json_server_config(&server, Some(format), true, "opencode", true).unwrap();

        assert_eq!(
            config["command"],
            json!([
                "cmd",
                "/c",
                "npx",
                "-y",
                "--prefer-online",
                "@sammysnake/fast-context-mcp"
            ])
        );
    }

    #[test]
    fn parse_nested_openclaw_mcp_servers() {
        let config = json!({
            "mcp": {
                "servers": {
                    "gemini": {
                        "type": "stdio",
                        "command": "node",
                        "args": ["server.js"]
                    }
                }
            }
        });

        let servers = parse_mcp_servers_from_value(&config, "mcp.servers", None).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "gemini");
        assert_eq!(servers[0].server_type, "stdio");
        assert_eq!(servers[0].server_config["command"], "node");
        assert_eq!(servers[0].server_config["args"], json!(["server.js"]));
    }

    #[test]
    fn build_gemini_like_http_config_uses_http_url() {
        let server = build_http_server();
        let format = get_format_config("gemini_cli").expect("gemini_cli format should exist");

        let config =
            build_json_server_config(&server, Some(format), true, "gemini_cli", false).unwrap();

        assert_eq!(config["type"], "http");
        assert_eq!(config["httpUrl"], "https://example.com/mcp");
        assert!(config.get("url").is_none());
        assert_eq!(config["headers"]["Authorization"], "Bearer token");
    }

    #[test]
    fn build_antigravity_http_config_uses_server_url() {
        let server = build_http_server();
        let format = get_format_config("antigravity").expect("antigravity format should exist");

        let config =
            build_json_server_config(&server, Some(format), true, "antigravity", false).unwrap();

        assert_eq!(config["type"], "http");
        assert_eq!(config["serverUrl"], "https://example.com/mcp");
        assert!(config.get("httpUrl").is_none());
        assert!(config.get("url").is_none());
        assert_eq!(config["headers"]["Authorization"], "Bearer token");
    }

    #[test]
    fn antigravity_remote_servers_round_trip_through_runtime_files() {
        for tool_key in ["antigravity", "antigravity_cli"] {
            for server_type in ["http", "sse"] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("mcp_config.json");
                let original = json!({
                    "customSetting": true,
                    "mcpServers": { "untouched": { "command": "example", "args": [] } }
                });
                std::fs::write(&path, original.to_string()).unwrap();
                let tool =
                    RuntimeTool::from(crate::coding::tools::builtin_tool_by_key(tool_key).unwrap());
                let format = get_format_config(tool_key).unwrap();
                let mut server = build_http_server();
                server.server_type = server_type.to_string();

                for url in ["https://example.com/mcp", "https://updated.example.com/mcp"] {
                    server.server_config["url"] = json!(url);
                    sync_server_to_path(&tool, &path, &server, true).unwrap();
                    let written: Value =
                        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                    assert_eq!(written["mcpServers"]["remote"]["serverUrl"], url);
                    assert!(written["mcpServers"]["remote"].get("url").is_none());
                    let imported =
                        parse_mcp_servers_from_value(&written, "mcpServers", Some(format)).unwrap();
                    let remote = imported.iter().find(|item| item.name == "remote").unwrap();
                    assert_eq!(remote.server_type, server_type);
                    assert_eq!(remote.server_config, server.server_config);
                }

                remove_server_from_path(&tool, &path, "remote").unwrap();
                let remaining: Value =
                    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                assert_eq!(remaining, original);
            }
        }
    }

    #[test]
    fn antigravity_cli_imports_legacy_remote_url_fields() {
        let format = get_format_config("antigravity_cli").unwrap();
        for server_type in ["http", "sse"] {
            for field in ["url", "httpUrl", "serverUrl"] {
                let config = json!({ "mcpServers": { "remote": {
                    "type": server_type, (field): "https://example.com/mcp"
                } } });
                let imported =
                    parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();
                assert_eq!(imported.len(), 1);
                assert_eq!(imported[0].server_type, server_type);
                assert_eq!(imported[0].server_config["url"], "https://example.com/mcp");
            }
        }
    }

    #[test]
    fn build_standard_http_config_keeps_url_for_non_gemini_tools() {
        let server = build_http_server();

        let config = build_json_server_config(&server, None, true, "claude_code", false).unwrap();

        assert_eq!(config["type"], "http");
        assert_eq!(config["url"], "https://example.com/mcp");
        assert!(config.get("httpUrl").is_none());
    }

    #[test]
    fn parse_gemini_like_http_prefers_http_url() {
        let config = json!({
            "mcpServers": {
                "remote": {
                    "type": "http",
                    "httpUrl": "https://example.com/mcp",
                    "url": "https://legacy.example.com/mcp",
                    "headers": {
                        "Authorization": "Bearer token"
                    }
                }
            }
        });
        let format = get_format_config("qwen_code").expect("qwen_code format should exist");

        let servers = parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_type, "http");
        assert_eq!(servers[0].server_config["url"], "https://example.com/mcp");
        assert_eq!(
            servers[0].server_config["headers"]["Authorization"],
            "Bearer token"
        );
    }

    #[test]
    fn parse_gemini_like_http_falls_back_to_url() {
        let config = json!({
            "mcpServers": {
                "remote": {
                    "type": "http",
                    "url": "https://legacy.example.com/mcp"
                }
            }
        });
        let format = get_format_config("antigravity").expect("antigravity format should exist");

        let servers = parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_type, "http");
        assert_eq!(
            servers[0].server_config["url"],
            "https://legacy.example.com/mcp"
        );
    }

    #[test]
    fn parse_antigravity_http_prefers_server_url() {
        let config = json!({
            "mcpServers": {
                "remote": {
                    "type": "http",
                    "serverUrl": "https://example.com/mcp",
                    "httpUrl": "https://legacy.example.com/mcp",
                    "headers": {
                        "Authorization": "Bearer token"
                    }
                }
            }
        });
        let format = get_format_config("antigravity").expect("antigravity format should exist");

        let servers = parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_type, "http");
        assert_eq!(servers[0].server_config["url"], "https://example.com/mcp");
        assert_eq!(
            servers[0].server_config["headers"]["Authorization"],
            "Bearer token"
        );
    }

    #[test]
    fn parse_antigravity_http_falls_back_to_legacy_http_url() {
        let config = json!({
            "mcpServers": {
                "remote": {
                    "type": "http",
                    "httpUrl": "https://legacy.example.com/mcp"
                }
            }
        });
        let format = get_format_config("antigravity").expect("antigravity format should exist");

        let servers = parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_type, "http");
        assert_eq!(
            servers[0].server_config["url"],
            "https://legacy.example.com/mcp"
        );
    }

    #[test]
    fn parse_gemini_like_http_url_without_type_infers_http() {
        let config = json!({
            "mcpServers": {
                "remote": {
                    "httpUrl": "https://example.com/mcp"
                }
            }
        });
        let format = get_format_config("gemini_cli").expect("gemini_cli format should exist");

        let servers = parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_type, "http");
        assert_eq!(servers[0].server_config["url"], "https://example.com/mcp");
    }

    #[test]
    fn parse_gemini_like_url_without_type_falls_back_to_http() {
        let config = json!({
            "mcpServers": {
                "remote": {
                    "url": "https://example.com/sse"
                }
            }
        });
        let format = get_format_config("gemini_cli").expect("gemini_cli format should exist");

        let servers = parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_type, "http");
        assert_eq!(servers[0].server_config["url"], "https://example.com/sse");
    }

    #[test]
    fn parse_gemini_like_explicit_sse_type_keeps_sse() {
        let config = json!({
            "mcpServers": {
                "remote": {
                    "type": "sse",
                    "url": "https://example.com/sse"
                }
            }
        });
        let format = get_format_config("gemini_cli").expect("gemini_cli format should exist");

        let servers = parse_mcp_servers_from_value(&config, "mcpServers", Some(format)).unwrap();

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_type, "sse");
        assert_eq!(servers[0].server_config["url"], "https://example.com/sse");
    }

    /// Build a stdio McpServer with the given command and args (helper for
    /// path-expansion tests).
    fn build_stdio_server_with(command: &str, args: &[&str]) -> McpServer {
        McpServer {
            id: String::new(),
            name: "pathy".to_string(),
            server_type: "stdio".to_string(),
            server_config: json!({
                "command": command,
                "args": args,
            }),
            enabled_tools: vec![],
            sync_details: None,
            description: None,
            user_group: None,
            user_note: None,
            tags: vec![],
            timeout: None,
            sort_index: 0,
            management_enabled: true,
            disabled_previous_tools: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn expand_stdio_paths_expands_tilde_command_and_arg() {
        let server =
            build_stdio_server_with("~/.fastctx/bin/fastctx.exe", &["-c", "~/config/mcp.json"]);

        let expanded = expand_stdio_paths(&server);

        let expected_cmd = expand_local_path("~/.fastctx/bin/fastctx.exe").unwrap();
        assert_eq!(expanded.server_config["command"], json!(expected_cmd));
        assert_eq!(
            expanded.server_config["args"][0],
            json!("-c"),
            "pure flag args must not be touched"
        );
        let expected_arg = expand_local_path("~/config/mcp.json").unwrap();
        assert_eq!(expanded.server_config["args"][1], json!(expected_arg));
    }

    #[test]
    fn expand_stdio_paths_expands_appdata_placeholder() {
        // %APPDATA% only exists on Windows; on other platforms the literal is
        // preserved (env var unset), so assert conditionally.
        let server = build_stdio_server_with("%APPDATA%/mcp/server.exe", &[]);
        let expanded = expand_stdio_paths(&server);
        if let Ok(appdata) = std::env::var("APPDATA") {
            assert_eq!(
                expanded.server_config["command"],
                json!(format!("{appdata}/mcp/server.exe"))
            );
        } else {
            assert_eq!(
                expanded.server_config["command"],
                json!("%APPDATA%/mcp/server.exe")
            );
        }
    }

    #[test]
    fn expand_stdio_paths_leaves_absolute_command_unchanged() {
        let abs = if cfg!(windows) {
            r"C:\Users\someone\fastctx.exe"
        } else {
            "/usr/local/bin/fastctx"
        };
        let server = build_stdio_server_with(abs, &["-y"]);
        let expanded = expand_stdio_paths(&server);
        assert_eq!(expanded.server_config["command"], json!(abs));
        assert_eq!(expanded.server_config["args"], json!(["-y"]));
    }

    #[test]
    fn expand_stdio_paths_skips_non_stdio() {
        let server = build_http_server();
        let expanded = expand_stdio_paths(&server);
        assert_eq!(expanded.server_type, "http");
        assert_eq!(expanded.server_config["url"], "https://example.com/mcp");
    }

    #[test]
    fn expand_stdio_paths_leaves_pure_flags_untouched() {
        let server = build_stdio_server_with(
            "npx",
            &["-y", "--prefer-online", "@sammysnake/fast-context-mcp"],
        );
        let expanded = expand_stdio_paths(&server);
        assert_eq!(expanded.server_config["command"], json!("npx"));
        assert_eq!(
            expanded.server_config["args"],
            json!(["-y", "--prefer-online", "@sammysnake/fast-context-mcp"])
        );
    }
}
