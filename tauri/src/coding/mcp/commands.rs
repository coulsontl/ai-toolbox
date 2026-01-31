//! Tauri commands for MCP Server management
//!
//! Provides the public API for the MCP feature.

use tauri::{AppHandle, Emitter, Runtime, State};

use super::adapter::parse_sync_details_dto;
use super::config_sync::{import_servers_from_tool, remove_server_from_tool, sync_server_to_tool};
use super::mcp_store;
use super::types::{
    CreateMcpServerInput, McpDiscoveredServerDto, McpImportResultDto, McpScanResultDto, McpServer, McpServerDto,
    McpSyncDetail, McpSyncResultDto, UpdateMcpServerInput, now_ms,
};
use crate::coding::tools::{
    custom_store, get_mcp_runtime_tools, runtime_tool_by_key, RuntimeToolDto, is_tool_installed,
    to_runtime_tool_dto,
};
use crate::DbState;

// ==================== MCP Server CRUD ====================

/// List all MCP servers
#[tauri::command]
pub async fn mcp_list_servers(state: State<'_, DbState>) -> Result<Vec<McpServerDto>, String> {
    let servers = mcp_store::get_mcp_servers(&state).await?;

    Ok(servers
        .into_iter()
        .map(|s| McpServerDto {
            id: s.id.clone(),
            name: s.name.clone(),
            server_type: s.server_type.clone(),
            server_config: s.server_config.clone(),
            enabled_tools: s.enabled_tools.clone(),
            sync_details: parse_sync_details_dto(&s),
            description: s.description.clone(),
            tags: s.tags.clone(),
            sort_index: s.sort_index,
            created_at: s.created_at,
            updated_at: s.updated_at,
        })
        .collect())
}

/// Create a new MCP server
/// After creation, automatically sync to all enabled tools
#[tauri::command]
pub async fn mcp_create_server(
    state: State<'_, DbState>,
    input: CreateMcpServerInput,
) -> Result<McpServerDto, String> {
    let now = now_ms();
    let server = McpServer {
        id: String::new(), // Will be assigned by upsert
        name: input.name.clone(),
        server_type: input.server_type.clone(),
        server_config: input.server_config.clone(),
        enabled_tools: input.enabled_tools.clone(),
        sync_details: None,
        description: input.description,
        tags: input.tags,
        sort_index: 0, // Will be assigned by upsert
        created_at: now,
        updated_at: now,
    };

    let id = mcp_store::upsert_mcp_server(&state, &server).await?;

    // Sync to all enabled tools
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    for tool_key in &input.enabled_tools {
        if let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) {
            if is_tool_installed(&tool) {
                match sync_server_to_tool(&server, &tool) {
                    Ok(detail) => {
                        let _ = mcp_store::update_sync_detail(&state, &id, &detail).await;
                    }
                    Err(e) => {
                        let detail = McpSyncDetail {
                            tool: tool_key.clone(),
                            status: "error".to_string(),
                            synced_at: Some(now_ms()),
                            error_message: Some(e),
                        };
                        let _ = mcp_store::update_sync_detail(&state, &id, &detail).await;
                    }
                }
            }
        }
    }

    // Get the created server with sync details
    let created = mcp_store::get_mcp_server_by_id(&state, &id)
        .await?
        .ok_or("Failed to get created server")?;

    let sync_details = parse_sync_details_dto(&created);
    Ok(McpServerDto {
        id: created.id,
        name: created.name,
        server_type: created.server_type,
        server_config: created.server_config,
        enabled_tools: created.enabled_tools,
        sync_details,
        description: created.description,
        tags: created.tags,
        sort_index: created.sort_index,
        created_at: created.created_at,
        updated_at: created.updated_at,
    })
}

/// Update an existing MCP server
/// After update, automatically re-sync to all enabled tools
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_update_server(
    state: State<'_, DbState>,
    serverId: String,
    input: UpdateMcpServerInput,
) -> Result<McpServerDto, String> {
    let mut server = mcp_store::get_mcp_server_by_id(&state, &serverId)
        .await?
        .ok_or_else(|| format!("MCP server not found: {}", serverId))?;

    // Apply updates
    if let Some(name) = input.name {
        server.name = name;
    }
    if let Some(server_type) = input.server_type {
        server.server_type = server_type;
    }
    if let Some(server_config) = input.server_config {
        server.server_config = server_config;
    }
    if let Some(enabled_tools) = input.enabled_tools {
        server.enabled_tools = enabled_tools;
    }
    if let Some(description) = input.description {
        server.description = Some(description);
    }
    if let Some(tags) = input.tags {
        server.tags = tags;
    }
    server.updated_at = now_ms();

    mcp_store::upsert_mcp_server(&state, &server).await?;

    // Re-sync to all enabled tools
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    for tool_key in &server.enabled_tools {
        if let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) {
            if is_tool_installed(&tool) {
                match sync_server_to_tool(&server, &tool) {
                    Ok(detail) => {
                        let _ = mcp_store::update_sync_detail(&state, &serverId, &detail).await;
                    }
                    Err(e) => {
                        let detail = McpSyncDetail {
                            tool: tool_key.clone(),
                            status: "error".to_string(),
                            synced_at: Some(now_ms()),
                            error_message: Some(e),
                        };
                        let _ = mcp_store::update_sync_detail(&state, &serverId, &detail).await;
                    }
                }
            }
        }
    }

    // Get the updated server with sync details
    let updated = mcp_store::get_mcp_server_by_id(&state, &serverId)
        .await?
        .ok_or("Failed to get updated server")?;

    let sync_details = parse_sync_details_dto(&updated);
    Ok(McpServerDto {
        id: updated.id,
        name: updated.name,
        server_type: updated.server_type,
        server_config: updated.server_config,
        enabled_tools: updated.enabled_tools,
        sync_details,
        description: updated.description,
        tags: updated.tags,
        sort_index: updated.sort_index,
        created_at: updated.created_at,
        updated_at: updated.updated_at,
    })
}

/// Delete an MCP server
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_delete_server(
    state: State<'_, DbState>,
    serverId: String,
) -> Result<(), String> {
    // Get the server first to remove from tool configs
    if let Some(server) = mcp_store::get_mcp_server_by_id(&state, &serverId).await? {
        // Remove from all enabled tools' configs
        let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
        for tool_key in &server.enabled_tools {
            if let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) {
                let _ = remove_server_from_tool(&server.name, &tool);
            }
        }
    }

    mcp_store::delete_mcp_server(&state, &serverId).await
}

/// Toggle a tool's enabled state for an MCP server
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_toggle_tool<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    serverId: String,
    toolKey: String,
) -> Result<bool, String> {
    let is_enabled = mcp_store::toggle_tool_enabled(&state, &serverId, &toolKey).await?;

    // Get the server
    let server = mcp_store::get_mcp_server_by_id(&state, &serverId)
        .await?
        .ok_or_else(|| format!("MCP server not found: {}", serverId))?;

    // Get the tool
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    let tool = runtime_tool_by_key(&toolKey, &custom_tools)
        .ok_or_else(|| format!("Tool not found: {}", toolKey))?;

    // Sync or remove based on new state
    if is_enabled {
        // Sync to tool config
        match sync_server_to_tool(&server, &tool) {
            Ok(detail) => {
                mcp_store::update_sync_detail(&state, &serverId, &detail).await?;
            }
            Err(e) => {
                let detail = McpSyncDetail {
                    tool: toolKey.clone(),
                    status: "error".to_string(),
                    synced_at: Some(now_ms()),
                    error_message: Some(e.clone()),
                };
                mcp_store::update_sync_detail(&state, &serverId, &detail).await?;
                return Err(e);
            }
        }
    } else {
        // Remove from tool config
        let _ = remove_server_from_tool(&server.name, &tool);
        mcp_store::delete_sync_detail(&state, &serverId, &toolKey).await?;
    }

    // Emit config-changed event
    let _ = app.emit("config-changed", "window");

    Ok(is_enabled)
}

/// Reorder MCP servers
#[tauri::command]
pub async fn mcp_reorder_servers(
    state: State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    mcp_store::reorder_mcp_servers(&state, &ids).await
}

// ==================== Sync Operations ====================

/// Sync all enabled servers to a specific tool
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_sync_to_tool<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    toolKey: String,
) -> Result<Vec<McpSyncResultDto>, String> {
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    let tool = runtime_tool_by_key(&toolKey, &custom_tools)
        .ok_or_else(|| format!("Tool not found: {}", toolKey))?;

    if !is_tool_installed(&tool) {
        return Err(format!("Tool {} is not installed", toolKey));
    }

    let servers = mcp_store::get_mcp_servers(&state).await?;
    let mut results = Vec::new();

    for server in servers {
        if !server.enabled_tools.contains(&toolKey) {
            continue;
        }

        match sync_server_to_tool(&server, &tool) {
            Ok(detail) => {
                mcp_store::update_sync_detail(&state, &server.id, &detail).await?;
                results.push(McpSyncResultDto {
                    tool: toolKey.clone(),
                    success: true,
                    error_message: None,
                });
            }
            Err(e) => {
                let detail = McpSyncDetail {
                    tool: toolKey.clone(),
                    status: "error".to_string(),
                    synced_at: Some(now_ms()),
                    error_message: Some(e.clone()),
                };
                mcp_store::update_sync_detail(&state, &server.id, &detail).await?;
                results.push(McpSyncResultDto {
                    tool: toolKey.clone(),
                    success: false,
                    error_message: Some(e),
                });
            }
        }
    }

    // Emit config-changed event
    let _ = app.emit("config-changed", "window");

    Ok(results)
}

/// Sync all servers to all enabled tools
#[tauri::command]
pub async fn mcp_sync_all<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
) -> Result<Vec<McpSyncResultDto>, String> {
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    let servers = mcp_store::get_mcp_servers(&state).await?;
    let mut results = Vec::new();

    for server in servers {
        for tool_key in &server.enabled_tools {
            let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) else {
                continue;
            };

            if !is_tool_installed(&tool) {
                continue;
            }

            match sync_server_to_tool(&server, &tool) {
                Ok(detail) => {
                    mcp_store::update_sync_detail(&state, &server.id, &detail).await?;
                    results.push(McpSyncResultDto {
                        tool: tool_key.clone(),
                        success: true,
                        error_message: None,
                    });
                }
                Err(e) => {
                    let detail = McpSyncDetail {
                        tool: tool_key.clone(),
                        status: "error".to_string(),
                        synced_at: Some(now_ms()),
                        error_message: Some(e.clone()),
                    };
                    mcp_store::update_sync_detail(&state, &server.id, &detail).await?;
                    results.push(McpSyncResultDto {
                        tool: tool_key.clone(),
                        success: false,
                        error_message: Some(e),
                    });
                }
            }
        }
    }

    // Emit config-changed event
    let _ = app.emit("config-changed", "window");

    Ok(results)
}

/// Import MCP servers from a tool's config file
/// After import, automatically sync to preferred tools (or all installed tools if no preference)
/// If a server with the same name exists but has different config, create with suffix
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_import_from_tool(
    state: State<'_, DbState>,
    toolKey: String,
) -> Result<McpImportResultDto, String> {
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    let tool = runtime_tool_by_key(&toolKey, &custom_tools)
        .ok_or_else(|| format!("Tool not found: {}", toolKey))?;

    let imported_servers = import_servers_from_tool(&tool)?;

    // Get target tools for sync: preferred tools or all installed MCP tools
    let prefs = mcp_store::get_mcp_preferences(&state).await?;
    let target_tools: Vec<String> = if !prefs.preferred_tools.is_empty() {
        // Use preferred tools, but only those that are installed
        prefs.preferred_tools
            .into_iter()
            .filter(|key| {
                runtime_tool_by_key(key, &custom_tools)
                    .map(|t| is_tool_installed(&t))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        // Use all installed MCP tools
        get_mcp_runtime_tools(&custom_tools)
            .into_iter()
            .filter(|t| is_tool_installed(t))
            .map(|t| t.key)
            .collect()
    };

    let mut servers_imported = 0;
    let mut servers_skipped = 0;
    let mut servers_duplicated = Vec::new();
    let mut errors = Vec::new();

    for mut server in imported_servers {
        // Check if server with same name already exists
        if let Some(existing) = mcp_store::get_mcp_server_by_name(&state, &server.name).await? {
            // Compare configurations
            if existing.server_type == server.server_type && existing.server_config == server.server_config {
                // Same config, skip
                servers_skipped += 1;
                continue;
            } else {
                // Different config, create with suffix
                let new_name = format!("{} ({})", server.name, tool.display_name);
                servers_duplicated.push(new_name.clone());
                server.name = new_name;
            }
        }

        // Enable the target tools
        server.enabled_tools = target_tools.clone();

        match mcp_store::upsert_mcp_server(&state, &server).await {
            Ok(server_id) => {
                servers_imported += 1;

                // Sync to each enabled tool
                for tool_key in &target_tools {
                    if let Some(target_tool) = runtime_tool_by_key(tool_key, &custom_tools) {
                        match sync_server_to_tool(&server, &target_tool) {
                            Ok(detail) => {
                                let _ = mcp_store::update_sync_detail(&state, &server_id, &detail).await;
                            }
                            Err(e) => {
                                let detail = McpSyncDetail {
                                    tool: tool_key.clone(),
                                    status: "error".to_string(),
                                    synced_at: Some(now_ms()),
                                    error_message: Some(e.clone()),
                                };
                                let _ = mcp_store::update_sync_detail(&state, &server_id, &detail).await;
                                errors.push(format!("Sync '{}' to {}: {}", server.name, tool_key, e));
                            }
                        }
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Failed to import '{}': {}", server.name, e));
            }
        }
    }

    Ok(McpImportResultDto {
        servers_imported,
        servers_skipped,
        servers_duplicated,
        errors,
    })
}

// ==================== Tools API ====================

/// Get all tools that support MCP
#[tauri::command]
pub async fn mcp_get_tools(state: State<'_, DbState>) -> Result<Vec<RuntimeToolDto>, String> {
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    let mcp_tools = get_mcp_runtime_tools(&custom_tools);

    Ok(mcp_tools
        .iter()
        .map(to_runtime_tool_dto)
        .collect())
}

/// Scan all installed MCP tools and return discovered servers (excluding already imported ones)
#[tauri::command]
pub async fn mcp_scan_servers(state: State<'_, DbState>) -> Result<McpScanResultDto, String> {
    let custom_tools = custom_store::get_custom_tools(&state).await.unwrap_or_default();
    let mcp_tools = get_mcp_runtime_tools(&custom_tools);

    // Get existing server names for filtering
    let existing_servers = mcp_store::get_mcp_servers(&state).await?;
    let existing_names: std::collections::HashSet<_> = existing_servers
        .iter()
        .map(|s| s.name.as_str())
        .collect();

    let mut total_tools_scanned = 0;
    let mut servers: Vec<McpDiscoveredServerDto> = Vec::new();

    for tool in &mcp_tools {
        if !is_tool_installed(tool) {
            continue;
        }

        total_tools_scanned += 1;

        // Try to import servers from this tool
        match import_servers_from_tool(tool) {
            Ok(imported) => {
                for server in imported {
                    // Skip servers that already exist in the database
                    if existing_names.contains(server.name.as_str()) {
                        continue;
                    }
                    servers.push(McpDiscoveredServerDto {
                        name: server.name,
                        tool_key: tool.key.clone(),
                        tool_name: tool.display_name.clone(),
                        server_type: server.server_type,
                    });
                }
            }
            Err(e) => {
                // Log error but continue scanning
                eprintln!("Failed to scan {}: {}", tool.key, e);
            }
        }
    }

    Ok(McpScanResultDto {
        total_tools_scanned,
        total_servers_found: servers.len() as i32,
        servers,
    })
}

// ==================== Preferences ====================

/// Get MCP show in tray setting
#[tauri::command]
pub async fn mcp_get_show_in_tray(state: State<'_, DbState>) -> Result<bool, String> {
    let prefs = mcp_store::get_mcp_preferences(&state).await?;
    Ok(prefs.show_in_tray)
}

/// Set MCP show in tray setting
#[tauri::command]
pub async fn mcp_set_show_in_tray(
    state: State<'_, DbState>,
    enabled: bool,
) -> Result<(), String> {
    let mut prefs = mcp_store::get_mcp_preferences(&state).await?;
    prefs.show_in_tray = enabled;
    prefs.updated_at = now_ms();
    mcp_store::save_mcp_preferences(&state, &prefs).await
}

/// Get MCP preferred tools
#[tauri::command]
pub async fn mcp_get_preferred_tools(state: State<'_, DbState>) -> Result<Vec<String>, String> {
    let prefs = mcp_store::get_mcp_preferences(&state).await?;
    Ok(prefs.preferred_tools)
}

/// Set MCP preferred tools
#[tauri::command]
pub async fn mcp_set_preferred_tools(
    state: State<'_, DbState>,
    tools: Vec<String>,
) -> Result<(), String> {
    let mut prefs = mcp_store::get_mcp_preferences(&state).await?;
    prefs.preferred_tools = tools;
    prefs.updated_at = now_ms();
    mcp_store::save_mcp_preferences(&state, &prefs).await
}

// ==================== Custom Tool Management ====================

/// Add or update a custom tool with MCP fields (preserves existing Skills fields)
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_add_custom_tool(
    state: State<'_, DbState>,
    key: String,
    displayName: String,
    relativeDetectDir: Option<String>,
    mcpConfigPath: String,
    mcpConfigFormat: String,
    mcpField: String,
) -> Result<(), String> {
    // Trim whitespace from all inputs
    let key = key.trim().to_string();
    let display_name = displayName.trim().to_string();
    let detect_dir = relativeDetectDir.map(|s| s.trim().trim_start_matches("~/").to_string());
    let mcp_path = mcpConfigPath.trim().trim_start_matches("~/").to_string();
    let mcp_format = mcpConfigFormat.trim().to_lowercase();
    let mcp_field_name = mcpField.trim().to_string();

    // Validate key format
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Key must contain only letters, numbers, and underscores".to_string());
    }

    // Validate mcp_format
    if mcp_format != "json" && mcp_format != "toml" {
        return Err("MCP config format must be 'json' or 'toml'".to_string());
    }

    // Check for duplicate with built-in tools
    if crate::coding::tools::builtin::builtin_tool_by_key(&key).is_some() {
        return Err(format!("Key '{}' conflicts with a built-in tool", key));
    }

    custom_store::save_custom_tool_mcp_fields(
        &state,
        &key,
        &display_name,
        detect_dir,
        Some(mcp_path),
        Some(mcp_format),
        Some(mcp_field_name),
        now_ms(),
    )
    .await
}

/// Remove a custom tool (only if it has no Skills fields, otherwise just clear MCP fields)
#[tauri::command]
pub async fn mcp_remove_custom_tool(state: State<'_, DbState>, key: String) -> Result<(), String> {
    // Get the existing tool
    let existing = custom_store::get_custom_tool_by_key(&state, &key).await?;

    if let Some(tool) = existing {
        // If tool has Skills fields, just clear MCP fields
        if tool.relative_skills_dir.is_some() {
            custom_store::save_custom_tool_mcp_fields(
                &state,
                &key,
                &tool.display_name,
                tool.relative_detect_dir.clone(),
                None,
                None,
                None,
                tool.created_at,
            )
            .await
        } else {
            // No Skills fields, delete completely
            custom_store::delete_custom_tool(&state, &key).await
        }
    } else {
        Err(format!("Custom tool '{}' not found", key))
    }
}
