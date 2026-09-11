//! MCP configuration sync to WSL
//!
//! Syncs MCP server configurations to WSL for all MCP-enabled tools:
//! - Claude Code: directly edit ~/.claude.json mcpServers field
//! - OpenCode/Codex/Gemini CLI/Pi: sync config files via file mappings

use log::info;
use serde_json::Value;
use std::collections::HashSet;
use tauri::{AppHandle, Emitter};

use super::adapter;
use super::commands::resolve_dynamic_paths_with_db;
use super::sync::{read_wsl_file, sync_mappings, windows_to_wsl_path, write_wsl_file};
use super::types::{FileMapping, SyncProgress, WSLSyncConfig};
use crate::coding::mcp::command_normalize;
use crate::coding::mcp::mcp_store;
use crate::coding::runtime_location;
use crate::db::helpers::{db_get, db_list};
use crate::db::schema::DbTable;
use crate::SqliteDbState;

/// Read WSL sync config directly from database (without tauri::State wrapper)
async fn get_wsl_config(state: &SqliteDbState) -> Result<WSLSyncConfig, String> {
    let db = state.db();
    let record = db.with_conn(|conn| db_get(conn, DbTable::WslSyncConfig, "config"))?;
    Ok(record
        .map(|value| adapter::config_from_db_value(value, vec![]))
        .unwrap_or_default())
}

/// Get file mappings from database
async fn get_file_mappings(state: &SqliteDbState) -> Result<Vec<FileMapping>, String> {
    let db = state.db();
    let mut records = db.with_conn(|conn| db_list(conn, DbTable::WslFileMapping, None))?;
    records.sort_by(|a, b| {
        let module_a = a.get("module").and_then(Value::as_str).unwrap_or_default();
        let module_b = b.get("module").and_then(Value::as_str).unwrap_or_default();
        let name_a = a.get("name").and_then(Value::as_str).unwrap_or_default();
        let name_b = b.get("name").and_then(Value::as_str).unwrap_or_default();
        module_a.cmp(module_b).then_with(|| name_a.cmp(name_b))
    });
    Ok(records
        .into_iter()
        .map(adapter::mapping_from_db_value)
        .collect())
}

/// Sync MCP configuration to WSL (called on mcp-changed event)
pub async fn sync_mcp_to_wsl(state: &SqliteDbState, app: AppHandle) -> Result<(), String> {
    let config = get_wsl_config(state).await?;

    if !config.enabled || !config.sync_mcp {
        return Ok(());
    }

    // Get effective distro (auto-resolve if configured one doesn't exist)
    let distro = match super::sync::get_effective_distro(&config.distro) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("WSL MCP sync skipped: {}", e);
            let _ = app.emit("wsl-sync-warning", format!("WSL MCP 同步已跳过：{}", e));
            return Ok(());
        }
    };
    let direct_modules = runtime_location::get_wsl_direct_modules_async(state.db()).await?;

    // 收集所有错误
    let mut all_errors: Vec<String> = vec![];

    // Emit progress for MCP sync
    let _ = app.emit(
        "wsl-sync-progress",
        SyncProgress {
            phase: "mcp".to_string(),
            current_item: "Claude Code MCP".to_string(),
            current: 1,
            total: 2,
            message: "MCP 同步: Claude Code...".to_string(),
            current_file: None,
        },
    );

    // 1. Claude Code: directly modify WSL ~/.claude.json
    let servers = mcp_store::get_mcp_servers(state).await?;
    let claude_servers: Vec<_> = servers
        .iter()
        .filter(|s| s.enabled_tools.contains(&"claude_code".to_string()))
        .collect();

    if !direct_modules.contains("claude") {
        if let Err(e) = sync_mcp_to_wsl_claude(state, &distro, &claude_servers).await {
            log::warn!("Skipped claude.json MCP sync: {}", e);
            all_errors.push(format!("Claude Code: {}", e));
            let _ = app.emit(
                "wsl-sync-warning",
                format!(
                    "WSL ~/.claude.json 同步已跳过：文件解析失败，请检查该文件格式是否正确。({})",
                    e
                ),
            );
        }
    }

    // Emit progress for OpenCode/Codex/Grok/Gemini CLI/Pi
    let _ = app.emit(
        "wsl-sync-progress",
        SyncProgress {
            phase: "mcp".to_string(),
            current_item: "OpenCode/Codex/Grok/Gemini CLI/Pi MCP".to_string(),
            current: 2,
            total: 2,
            message: "MCP 同步: OpenCode/Codex/Grok/Gemini CLI/Pi...".to_string(),
            current_file: None,
        },
    );

    // 2. OpenCode/Codex/Grok/Gemini CLI/Pi: sync config files via file mappings
    match get_file_mappings(state).await {
        Ok(file_mappings) => {
            let mcp_mappings = filter_mcp_file_mappings(file_mappings, &direct_modules);

            if !mcp_mappings.is_empty() {
                let resolved = resolve_dynamic_paths_with_db(&state.db(), mcp_mappings).await;
                let result = sync_mappings(&resolved, &distro, None);
                if !result.errors.is_empty() {
                    let msg = result.errors.join("; ");
                    log::warn!("MCP file mapping sync errors: {}", msg);
                    all_errors.push(format!("OpenCode/Codex/Grok/Gemini CLI/Pi: {}", msg));
                    let _ = app.emit(
                        "wsl-sync-warning",
                        format!(
                            "OpenCode/Codex/Grok/Gemini CLI/Pi 配置同步部分失败：{}",
                            msg
                        ),
                    );
                }

                // Post-process: strip cmd /c from synced MCP config files (WSL is Linux, doesn't need it)
                // Only process files that actually contain MCP server configurations
                let synced_paths: std::collections::HashSet<String> = result
                    .synced_files
                    .iter()
                    .filter_map(|s| s.split(" -> ").nth(1).map(|p| p.to_string()))
                    .collect();
                for mapping in &resolved {
                    if mapping.enabled
                        && is_mapped_mcp_config_file(&mapping.id)
                        && synced_paths.contains(&mapping.wsl_path)
                    {
                        if let Err(e) = strip_cmd_c_from_wsl_mcp_file(
                            &distro,
                            &mapping.wsl_path,
                            &mapping.module,
                        ) {
                            log::warn!("Failed to strip cmd /c from {}: {}", mapping.wsl_path, e);
                        }
                    }
                }
            }
        }
        Err(e) => {
            log::warn!("Skipped OpenCode/Codex/Grok/Gemini CLI/Pi MCP sync: {}", e);
            all_errors.push(format!("OpenCode/Codex/Grok/Gemini CLI/Pi: {}", e));
            let _ = app.emit(
                "wsl-sync-warning",
                format!("OpenCode/Codex/Grok/Gemini CLI/Pi MCP 同步已跳过：{}", e),
            );
        }
    }

    info!(
        "MCP WSL sync completed: {} servers synced to claude_code",
        claude_servers.len()
    );

    // 根据真实结果更新状态
    let sync_result = super::types::SyncResult {
        success: all_errors.is_empty(),
        synced_files: vec![],
        skipped_files: vec![],
        errors: all_errors,
        warnings: vec![],
    };
    let _ = super::commands::update_sync_status(state, &sync_result).await;

    // Emit event for UI feedback
    let _ = app.emit("wsl-mcp-sync-completed", ());
    let _ = app.emit("wsl-sync-completed", &sync_result);

    Ok(())
}

/// Sync MCP servers to WSL Claude Code ~/.claude.json
async fn sync_mcp_to_wsl_claude(
    state: &SqliteDbState,
    distro: &str,
    servers: &[&crate::coding::mcp::types::McpServer],
) -> Result<(), String> {
    let db = state.db();
    let wsl_config_path = runtime_location::get_claude_wsl_claude_json_path_async(&db).await;

    // 1. Read existing WSL ~/.claude.json
    let existing_content = read_wsl_file(distro, wsl_config_path.as_str())?;

    // 2. Parse JSON, update mcpServers field
    let mut config: Value = if existing_content.trim().is_empty() {
        serde_json::json!({})
    } else {
        json5::from_str(&existing_content)
            .map_err(|e| format!("Failed to parse WSL claude.json: {}", e))?
    };

    // 3. Build mcpServers object
    let mut mcp_servers = serde_json::Map::new();
    for server in servers {
        let server_config = build_standard_server_config(server);
        mcp_servers.insert(server.name.clone(), server_config);
    }

    // 4. Update only the mcpServers field, preserve other fields
    config
        .as_object_mut()
        .ok_or("WSL claude.json is not a JSON object")?
        .insert("mcpServers".to_string(), Value::Object(mcp_servers));

    // 5. Write back to WSL
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    write_wsl_file(distro, wsl_config_path.as_str(), &content)?;

    Ok(())
}

/// Build standard JSON server config for Claude Code format
/// Note: Database stores normalized config (no cmd /c), but we add a safeguard here
fn build_standard_server_config(server: &crate::coding::mcp::types::McpServer) -> Value {
    match server.server_type.as_str() {
        "stdio" => {
            let command = server
                .server_config
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args: Vec<Value> = server
                .server_config
                .get("args")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let env = server.server_config.get("env").cloned();

            let mut result = serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
            });

            if let Some(env_val) = env {
                if env_val.is_object() && !env_val.as_object().map(|o| o.is_empty()).unwrap_or(true)
                {
                    result["env"] = env_val;
                }
            }

            // 1. Safeguard: ensure no cmd /c for WSL (database should already be
            //    normalized). This may unwrap `cmd /c <exe>` into `<exe>`.
            let mut result = command_normalize::unwrap_cmd_c(&result);

            // 2. WSL can run Windows exes via /mnt; convert the (now real) command's
            //    Windows full path (or ~/%APPDATA%, expanded on the host first) to
            //    /mnt/c/... so the WSL-side CLI can spawn it. Must run AFTER the
            //    cmd /c unwrap, otherwise the real exe buried in args is missed.
            //    Bare command names (npx, node) and already-Linux paths pass through.
            if let Some(cmd) = result.get("command").and_then(|v| v.as_str()) {
                let mapped = windows_to_wsl_path(cmd).unwrap_or_else(|_| cmd.to_string());
                if mapped != cmd {
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("command".to_string(), Value::String(mapped));
                    }
                }
            }

            result
        }
        "http" | "sse" => {
            let url = server
                .server_config
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let headers = server.server_config.get("headers").cloned();

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

            result
        }
        _ => server.server_config.clone(),
    }
}

/// Check whether a file mapping is part of the MCP-specific sync path.
/// Claude Code is handled by direct ~/.claude.json writes, not file mappings.
fn is_mapped_mcp_config_file(mapping_id: &str) -> bool {
    matches!(
        mapping_id,
        "opencode-main"
            | "opencode-oh-my"
            | "codex-config"
            | "grok-config"
            | "geminicli-settings"
            | "pi-mcp"
            | "omp-mcp"
            | "hermes-config"
            | "dsh-mcp"
    )
}

fn filter_mcp_file_mappings(
    mappings: Vec<FileMapping>,
    direct_modules: &HashSet<String>,
) -> Vec<FileMapping> {
    mappings
        .into_iter()
        .filter(|mapping| mapping.enabled && is_mapped_mcp_config_file(&mapping.id))
        .filter(|mapping| !direct_modules.contains(&mapping.module))
        .collect()
}

/// Strip cmd /c from WSL MCP config file after sync.
/// Selects the correct parser based on file extension rather than module name,
/// so that JSON files are not accidentally parsed as TOML.
fn strip_cmd_c_from_wsl_mcp_file(distro: &str, wsl_path: &str, module: &str) -> Result<(), String> {
    let content = read_wsl_file(distro, wsl_path)?;
    if content.trim().is_empty() {
        return Ok(());
    }

    // Convert Windows full-path (or ~/%APPDATA%, expanded on host) stdio
    // commands to /mnt/c/... so WSL can spawn the Windows exe. Bare command
    // names and Linux paths pass through unchanged.
    let to_wsl = |s: &str| windows_to_wsl_path(s).unwrap_or_else(|_| s.to_string());

    let processed = match module {
        "opencode" => command_normalize::process_opencode_json(&content, false, &to_wsl)?,
        "codex" => {
            // Determine parser by file extension: only .toml files use TOML parser
            if wsl_path.ends_with(".toml") {
                command_normalize::process_codex_toml(&content, false, &to_wsl)?
            } else {
                // JSON files in codex module (e.g. auth.json) should not be processed
                return Ok(());
            }
        }
        // Grok carries MCP servers in config.toml with the same mcp_servers /
        // stdio structure as Codex, but never wraps cmd /c. Reuse the Codex TOML
        // parser: the cmd /c strip is a no-op for Grok, and the path transform
        // converts any Windows full-path command to /mnt for the WSL target.
        "grok" => command_normalize::process_codex_toml(&content, false, &to_wsl)?,
        "geminicli" | "pi" | "oh_my_pi" => {
            command_normalize::process_claude_json(&content, false, &to_wsl)?
        }
        // Hermes mcp_servers lives in YAML; dsh uses the cordis patch DSL
        // (also YAML). Both carry `cmd /c` on Windows and need it stripped
        // for the Linux-side WSL target.
        "hermes" => command_normalize::process_hermes_yaml_mcp_servers(&content, &to_wsl)?,
        "dsh" => command_normalize::process_cordis_patch_yaml(&content, &to_wsl)?,
        _ => return Ok(()),
    };

    // Only write back if content changed
    if processed != content {
        write_wsl_file(distro, wsl_path, &processed)?;
        info!("Stripped cmd /c from WSL MCP config: {}", wsl_path);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_standard_server_config, filter_mcp_file_mappings, is_mapped_mcp_config_file,
    };
    use crate::coding::wsl::wsl_get_default_mappings;
    use std::collections::HashSet;

    #[test]
    fn recognizes_gemini_cli_settings_as_mcp_config_file() {
        assert!(is_mapped_mcp_config_file("geminicli-settings"));
    }

    #[test]
    fn recognizes_pi_mcp_as_mcp_config_file() {
        assert!(is_mapped_mcp_config_file("pi-mcp"));
    }

    #[test]
    fn excludes_gemini_cli_non_mcp_file_mappings() {
        assert!(!is_mapped_mcp_config_file("geminicli-env"));
        assert!(!is_mapped_mcp_config_file("geminicli-prompt"));
        assert!(!is_mapped_mcp_config_file("geminicli-oauth"));
    }

    #[test]
    fn excludes_pi_non_mcp_file_mappings() {
        assert!(!is_mapped_mcp_config_file("pi-settings"));
        assert!(!is_mapped_mcp_config_file("pi-auth"));
        assert!(!is_mapped_mcp_config_file("pi-prompt"));
    }

    #[test]
    fn skips_direct_mcp_mappings_and_preserves_local_modules() {
        for module in ["pi", "oh_my_pi", "grok", "hermes", "dsh"] {
            let direct_modules = HashSet::from([module.to_string()]);
            let mappings = filter_mcp_file_mappings(wsl_get_default_mappings(), &direct_modules);
            assert!(mappings.iter().all(|mapping| mapping.module != module));
            assert!(mappings.iter().any(|mapping| mapping.id == "codex-config"));
        }
    }

    #[test]
    fn local_hermes_and_dsh_sync_only_enabled_mcp_files() {
        let mut mappings = wsl_get_default_mappings();
        let selected = filter_mcp_file_mappings(mappings.clone(), &HashSet::new());
        for id in ["hermes-config", "dsh-mcp"] {
            assert!(selected.iter().any(|mapping| mapping.id == id));
        }
        for id in [
            "hermes-prompt",
            "dsh-config",
            "dsh-credentials",
            "dsh-prompt",
        ] {
            assert!(selected.iter().all(|mapping| mapping.id != id));
        }

        mappings
            .iter_mut()
            .find(|mapping| mapping.id == "dsh-mcp")
            .unwrap()
            .enabled = false;
        let selected = filter_mcp_file_mappings(mappings, &HashSet::new());
        assert!(selected.iter().all(|mapping| mapping.id != "dsh-mcp"));
        assert!(selected.iter().any(|mapping| mapping.id == "hermes-config"));
    }

    fn make_stdio_mcp_server(command: &str, args: &[&str]) -> crate::coding::mcp::types::McpServer {
        crate::coding::mcp::types::McpServer {
            id: String::new(),
            name: "fs".to_string(),
            server_type: "stdio".to_string(),
            server_config: serde_json::json!({
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
    fn build_standard_server_config_converts_full_path_command_to_mnt() {
        let server = make_stdio_mcp_server("C:\\Users\\x\\.fastctx\\bin\\fastctx.exe", &["-y"]);
        let config = build_standard_server_config(&server);
        assert_eq!(config["command"], "/mnt/c/Users/x/.fastctx/bin/fastctx.exe");
        // args untouched
        assert_eq!(config["args"], serde_json::json!(["-y"]));
    }

    #[test]
    fn build_standard_server_config_unwraps_cmd_c_then_converts() {
        // cmd /c wrapping a full path: the real exe must be unwrapped FIRST and
        // then converted to /mnt. Regression test for the ordering bug where the
        // transform ran before the unwrap and missed the exe buried in args.
        let server = make_stdio_mcp_server("cmd", &["/c", "C:\\x.exe"]);
        let config = build_standard_server_config(&server);
        assert_eq!(config["command"], "/mnt/c/x.exe");
        assert!(config["args"].as_array().unwrap().is_empty());
    }

    #[test]
    fn build_standard_server_config_leaves_bare_command_unchanged() {
        let server = make_stdio_mcp_server("npx", &["-y", "pkg"]);
        let config = build_standard_server_config(&server);
        assert_eq!(config["command"], "npx");
        assert_eq!(config["args"], serde_json::json!(["-y", "pkg"]));
    }
}
