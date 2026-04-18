//! Tauri commands for MCP Server management
//!
//! Provides the public API for the MCP feature.

use tauri::{AppHandle, Emitter, Runtime, State};

use super::adapter::parse_sync_details_dto;
use super::config_sync::{
    import_servers_from_path, import_servers_from_plugin_mcp_json, import_servers_from_tool_async,
    remove_server_from_tool_async, sync_server_to_tool_async,
    sync_server_to_tool_with_enabled_async,
};
use super::mcp_store;
use super::types::{
    now_ms, CreateMcpServerInput, FavoriteMcp, FavoriteMcpDto, FavoriteMcpInput,
    McpDiscoveredServerDto, McpImportResultDto, McpScanResultDto, McpServer, McpServerDto,
    McpSyncDetail, McpSyncResultDto, UpdateMcpServerInput,
};
use crate::coding::tools::{
    custom_store, get_mcp_runtime_tools, is_tool_installed_with_db_async,
    resolve_mcp_config_path_with_db_async, runtime_tool_by_key, to_runtime_tool_dto_with_db_async,
    CustomTool, RuntimeToolDto,
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
            timeout: s.timeout,
            sort_index: s.sort_index,
            created_at: s.created_at,
            updated_at: s.updated_at,
        })
        .collect())
}

/// Create a new MCP server
/// After creation, automatically sync to all enabled tools
#[tauri::command]
pub async fn mcp_create_server<R: Runtime>(
    app: AppHandle<R>,
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
        timeout: input.timeout,
        sort_index: 0, // Will be assigned by upsert
        created_at: now,
        updated_at: now,
    };

    let id = mcp_store::upsert_mcp_server(&state, &server).await?;

    // Sync to all enabled tools
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let db = state.db();
    for tool_key in &input.enabled_tools {
        if let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) {
            if is_tool_installed_with_db_async(&db, &tool).await {
                match sync_server_to_tool_async(&db, &server, &tool).await {
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

    // Sync disabled to opencode if the switch is ON and opencode is not in enabled_tools
    maybe_sync_disabled_to_opencode(&state, &server, &custom_tools).await;

    // Get the created server with sync details
    let created = mcp_store::get_mcp_server_by_id(&state, &id)
        .await?
        .ok_or("Failed to get created server")?;

    // Emit mcp-changed for WSL sync
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("mcp-changed", "window");

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
        timeout: created.timeout,
        sort_index: created.sort_index,
        created_at: created.created_at,
        updated_at: created.updated_at,
    })
}

/// Update an existing MCP server
/// After update, automatically re-sync to all enabled tools
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_update_server<R: Runtime>(
    app: AppHandle<R>,
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
    server.timeout = input.timeout;
    server.updated_at = now_ms();

    mcp_store::upsert_mcp_server(&state, &server).await?;

    // Re-sync to all enabled tools
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let db = state.db();
    for tool_key in &server.enabled_tools {
        if let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) {
            if is_tool_installed_with_db_async(&db, &tool).await {
                match sync_server_to_tool_async(&db, &server, &tool).await {
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

    // Sync disabled to opencode if the switch is ON and opencode is not in enabled_tools
    maybe_sync_disabled_to_opencode(&state, &server, &custom_tools).await;

    // Get the updated server with sync details
    let updated = mcp_store::get_mcp_server_by_id(&state, &serverId)
        .await?
        .ok_or("Failed to get updated server")?;

    // Emit mcp-changed for WSL sync
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("mcp-changed", "window");

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
        timeout: updated.timeout,
        sort_index: updated.sort_index,
        created_at: updated.created_at,
        updated_at: updated.updated_at,
    })
}

/// Delete an MCP server
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_delete_server<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    serverId: String,
) -> Result<(), String> {
    // Get the server first to remove from tool configs
    if let Some(server) = mcp_store::get_mcp_server_by_id(&state, &serverId).await? {
        // Remove from all enabled tools' configs
        let custom_tools = custom_store::get_custom_tools(&state)
            .await
            .unwrap_or_default();
        let db = state.db();
        for tool_key in &server.enabled_tools {
            if let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) {
                let _ = remove_server_from_tool_async(&db, &server.name, &tool).await;
            }
        }
        // Also remove from opencode if sync_disabled is ON
        maybe_remove_disabled_from_opencode(&state, &server, &custom_tools).await;
    }

    mcp_store::delete_mcp_server(&state, &serverId).await?;

    // Emit mcp-changed for WSL sync
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("mcp-changed", "window");

    Ok(())
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
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let db = state.db();
    let tool = runtime_tool_by_key(&toolKey, &custom_tools)
        .ok_or_else(|| format!("Tool not found: {}", toolKey))?;

    // Sync or remove based on new state
    if is_enabled {
        // Sync to tool config
        match sync_server_to_tool_async(&db, &server, &tool).await {
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
        // Remove from tool config (or write as disabled for opencode)
        if toolKey == "opencode" {
            let prefs = mcp_store::get_mcp_preferences(&state)
                .await
                .unwrap_or_default();
            if prefs.sync_disabled_to_opencode {
                // Write with enabled=false instead of removing
                let _ = sync_server_to_tool_with_enabled_async(&db, &server, &tool, false).await;
            } else {
                let _ = remove_server_from_tool_async(&db, &server.name, &tool).await;
            }
        } else {
            let _ = remove_server_from_tool_async(&db, &server.name, &tool).await;
        }
        mcp_store::delete_sync_detail(&state, &serverId, &toolKey).await?;
    }

    // Emit config-changed and mcp-changed events
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("mcp-changed", "window");

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
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let tool = runtime_tool_by_key(&toolKey, &custom_tools)
        .ok_or_else(|| format!("Tool not found: {}", toolKey))?;

    let db = state.db();
    if !is_tool_installed_with_db_async(&db, &tool).await {
        return Err(format!("Tool {} is not installed", toolKey));
    }

    let servers = mcp_store::get_mcp_servers(&state).await?;
    let mut results = Vec::new();

    for server in servers {
        if !server.enabled_tools.contains(&toolKey) {
            continue;
        }

        match sync_server_to_tool_async(&db, &server, &tool).await {
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

    // Emit config-changed and mcp-changed events
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("mcp-changed", "window");

    Ok(results)
}

/// Sync all servers to all enabled tools
#[tauri::command]
pub async fn mcp_sync_all<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
) -> Result<Vec<McpSyncResultDto>, String> {
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let db = state.db();
    let servers = mcp_store::get_mcp_servers(&state).await?;
    let mut results = Vec::new();

    for server in servers {
        for tool_key in &server.enabled_tools {
            let Some(tool) = runtime_tool_by_key(tool_key, &custom_tools) else {
                continue;
            };

            if !is_tool_installed_with_db_async(&db, &tool).await {
                continue;
            }

            match sync_server_to_tool_async(&db, &server, &tool).await {
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

    // Also sync disabled servers to opencode if switch is ON
    let prefs = mcp_store::get_mcp_preferences(&state)
        .await
        .unwrap_or_default();
    if prefs.sync_disabled_to_opencode {
        let all_servers = mcp_store::get_mcp_servers(&state).await.unwrap_or_default();
        sync_opencode_disabled(&db, &all_servers, &custom_tools).await;
    }

    // Emit config-changed and mcp-changed events
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("mcp-changed", "window");

    Ok(results)
}

/// Import MCP servers from a tool's config file
/// After import, automatically sync to specified tools (or preferred tools if not specified)
/// If a server with the same name exists but has different config, create with suffix
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_import_from_tool(
    state: State<'_, DbState>,
    toolKey: String,
    enabledTools: Option<Vec<String>>,
) -> Result<McpImportResultDto, String> {
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();

    // Resolve imported servers: either from a plugin or a standard tool
    let (imported_servers, source_display_name) =
        if let Some(plugin_id) = toolKey.strip_prefix("plugin::") {
            // Plugin source: find the plugin and read its .mcp.json
            let plugins =
                crate::coding::tools::claude_plugins::get_installed_plugins(&state.db()).await;
            let plugin = plugins
                .iter()
                .find(|p| p.plugin_id == plugin_id)
                .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?;
            let mcp_json_path = plugin.install_path.join(".mcp.json");
            let servers = import_servers_from_plugin_mcp_json(&mcp_json_path)?;
            (servers, format!("Plugin: {}", plugin.display_name))
        } else {
            // Standard tool source
            let tool = runtime_tool_by_key(&toolKey, &custom_tools)
                .ok_or_else(|| format!("Tool not found: {}", toolKey))?;
            let servers = import_servers_from_tool_async(&state.db(), &tool).await?;
            (
                servers,
                super::mcp_tool_display_name(&tool.key, &tool.display_name),
            )
        };

    // Get target tools for sync: use enabledTools if provided, otherwise use preferred tools or all installed MCP tools
    let target_tools: Vec<String> = if let Some(enabled) = enabledTools {
        // Use provided enabled tools, but only those that are installed
        {
            let mut installed_tool_keys = Vec::new();
            for key in enabled {
                let Some(tool) = runtime_tool_by_key(&key, &custom_tools) else {
                    continue;
                };
                if is_tool_installed_with_db_async(&state.db(), &tool).await {
                    installed_tool_keys.push(key);
                }
            }
            installed_tool_keys
        }
    } else {
        // Fall back to preferred tools or all installed MCP tools
        let prefs = mcp_store::get_mcp_preferences(&state).await?;
        if !prefs.preferred_tools.is_empty() {
            // Use preferred tools, but only those that are installed
            {
                let mut installed_tool_keys = Vec::new();
                for key in prefs.preferred_tools {
                    let Some(tool) = runtime_tool_by_key(&key, &custom_tools) else {
                        continue;
                    };
                    if is_tool_installed_with_db_async(&state.db(), &tool).await {
                        installed_tool_keys.push(key);
                    }
                }
                installed_tool_keys
            }
        } else {
            // Use all installed MCP tools
            {
                let mut installed_tool_keys = Vec::new();
                for tool in get_mcp_runtime_tools(&custom_tools) {
                    if is_tool_installed_with_db_async(&state.db(), &tool).await {
                        installed_tool_keys.push(tool.key);
                    }
                }
                installed_tool_keys
            }
        }
    };

    let mut servers_imported = 0;
    let mut servers_skipped = 0;
    let mut servers_duplicated = Vec::new();
    let mut errors = Vec::new();

    for mut server in imported_servers {
        // Check if server with same name already exists
        if let Some(existing) = mcp_store::get_mcp_server_by_name(&state, &server.name).await? {
            // Compare configurations
            if existing.server_type == server.server_type
                && existing.server_config == server.server_config
            {
                // Same config, skip
                servers_skipped += 1;
                continue;
            } else {
                // Different config, create with suffix
                let new_name = format!("{} ({})", server.name, source_display_name);
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
                        match sync_server_to_tool_async(&state.db(), &server, &target_tool).await {
                            Ok(detail) => {
                                let _ = mcp_store::update_sync_detail(&state, &server_id, &detail)
                                    .await;
                            }
                            Err(e) => {
                                let detail = McpSyncDetail {
                                    tool: tool_key.clone(),
                                    status: "error".to_string(),
                                    synced_at: Some(now_ms()),
                                    error_message: Some(e.clone()),
                                };
                                let _ = mcp_store::update_sync_detail(&state, &server_id, &detail)
                                    .await;
                                errors
                                    .push(format!("Sync '{}' to {}: {}", server.name, tool_key, e));
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
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let mcp_tools = get_mcp_runtime_tools(&custom_tools);
    let db = state.db();

    let mut tool_dtos = Vec::with_capacity(mcp_tools.len());
    for tool in &mcp_tools {
        let mut tool_dto = to_runtime_tool_dto_with_db_async(&db, tool).await;
        tool_dto.display_name = super::mcp_tool_display_name(&tool.key, &tool_dto.display_name);
        tool_dtos.push(tool_dto);
    }

    Ok(tool_dtos)
}

/// Scan all installed MCP tools and return discovered servers (excluding already imported ones)
#[tauri::command]
pub async fn mcp_scan_servers(state: State<'_, DbState>) -> Result<McpScanResultDto, String> {
    // Add 30 second timeout to prevent hanging
    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        mcp_scan_servers_inner(&state),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            Err("Scan timed out after 30 seconds. Please check your custom tool paths.".to_string())
        }
    }
}

async fn mcp_scan_servers_inner(state: &DbState) -> Result<McpScanResultDto, String> {
    let custom_tools = custom_store::get_custom_tools(state)
        .await
        .unwrap_or_default();
    let mcp_tools = get_mcp_runtime_tools(&custom_tools);
    let scan_db = state.db();

    // Get existing server names for filtering
    let existing_servers = mcp_store::get_mcp_servers(state).await?;
    let existing_names: std::collections::HashSet<String> =
        existing_servers.iter().map(|s| s.name.clone()).collect();
    let claude_plugins =
        crate::coding::tools::claude_plugins::get_installed_plugins(&scan_db).await;

    let mut scan_targets = Vec::new();
    for tool in &mcp_tools {
        if !is_tool_installed_with_db_async(&scan_db, tool).await {
            continue;
        }

        let Some(config_path) = resolve_mcp_config_path_with_db_async(&scan_db, tool).await else {
            continue;
        };

        if !config_path.exists() {
            continue;
        }

        scan_targets.push((tool.clone(), config_path));
    }

    // Run the blocking file system operations in a dedicated thread pool
    // to avoid blocking the tokio async runtime
    let scan_result = tokio::task::spawn_blocking(move || {
        let mut total_tools_scanned = 0;
        let mut servers: Vec<McpDiscoveredServerDto> = Vec::new();

        for (tool, config_path) in &scan_targets {
            eprintln!("[DEBUG][mcp_scan_servers] scanning tool: {}", tool.key);
            total_tools_scanned += 1;

            // Try to import servers from this tool
            match import_servers_from_path(tool, config_path) {
                Ok(imported) => {
                    eprintln!(
                        "[DEBUG][mcp_scan_servers] {} imported {} servers",
                        tool.key,
                        imported.len()
                    );
                    for server in imported {
                        // Skip servers that already exist in the database
                        if existing_names.contains(&server.name) {
                            continue;
                        }
                        servers.push(McpDiscoveredServerDto {
                            name: server.name,
                            tool_key: tool.key.clone(),
                            tool_name: super::mcp_tool_display_name(&tool.key, &tool.display_name),
                            server_type: server.server_type,
                            server_config: server.server_config,
                        });
                    }
                }
                Err(e) => {
                    // Log error but continue scanning
                    eprintln!("Failed to scan {}: {}", tool.key, e);
                }
            }
        }

        // Scan Claude Code plugins for MCP servers
        for plugin in &claude_plugins {
            let mcp_json_path = plugin.install_path.join(".mcp.json");
            if !mcp_json_path.exists() {
                continue;
            }

            let tool_key = format!("plugin::{}", plugin.plugin_id);
            let tool_name = format!("Plugin: {}", plugin.display_name);
            total_tools_scanned += 1;

            match import_servers_from_plugin_mcp_json(&mcp_json_path) {
                Ok(imported) => {
                    for server in imported {
                        if existing_names.contains(&server.name) {
                            continue;
                        }
                        servers.push(McpDiscoveredServerDto {
                            name: server.name,
                            tool_key: tool_key.clone(),
                            tool_name: tool_name.clone(),
                            server_type: server.server_type,
                            server_config: server.server_config,
                        });
                    }
                }
                Err(e) => {
                    eprintln!("Failed to scan plugin {}: {}", plugin.plugin_id, e);
                }
            }
        }

        McpScanResultDto {
            total_tools_scanned,
            total_servers_found: servers.len() as i32,
            servers,
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?;

    Ok(scan_result)
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
pub async fn mcp_set_show_in_tray(state: State<'_, DbState>, enabled: bool) -> Result<(), String> {
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

/// Get sync disabled to opencode setting
#[tauri::command]
pub async fn mcp_get_sync_disabled_to_opencode(state: State<'_, DbState>) -> Result<bool, String> {
    let prefs = mcp_store::get_mcp_preferences(&state).await?;
    Ok(prefs.sync_disabled_to_opencode)
}

/// Set sync disabled to opencode setting
/// When toggled ON: sync all unlinked MCP servers to opencode with enabled=false
/// When toggled OFF: remove disabled entries from opencode config
#[tauri::command]
pub async fn mcp_set_sync_disabled_to_opencode<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    enabled: bool,
) -> Result<(), String> {
    let mut prefs = mcp_store::get_mcp_preferences(&state).await?;
    prefs.sync_disabled_to_opencode = enabled;
    prefs.updated_at = now_ms();
    mcp_store::save_mcp_preferences(&state, &prefs).await?;

    let servers = mcp_store::get_mcp_servers(&state).await?;
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let db = state.db();

    if enabled {
        sync_opencode_disabled(&db, &servers, &custom_tools).await;
    } else {
        cleanup_opencode_disabled(&db, &servers, &custom_tools).await;
    }

    let _ = app.emit("config-changed", "window");
    let _ = app.emit("mcp-changed", "window");

    Ok(())
}

/// If sync_disabled_to_opencode is ON and server is not linked to opencode,
/// write it to opencode config with enabled=false.
async fn maybe_sync_disabled_to_opencode(
    state: &DbState,
    server: &McpServer,
    custom_tools: &[CustomTool],
) {
    let prefs = mcp_store::get_mcp_preferences(state)
        .await
        .unwrap_or_default();
    if !prefs.sync_disabled_to_opencode || server.enabled_tools.contains(&"opencode".to_string()) {
        return;
    }
    if let Some(tool) = runtime_tool_by_key("opencode", custom_tools) {
        let db = state.db();
        if is_tool_installed_with_db_async(&db, &tool).await {
            let _ = sync_server_to_tool_with_enabled_async(&db, server, &tool, false).await;
        }
    }
}

/// If sync_disabled_to_opencode is ON and server is not linked to opencode,
/// remove it from opencode config (used when deleting a server).
async fn maybe_remove_disabled_from_opencode(
    state: &DbState,
    server: &McpServer,
    custom_tools: &[CustomTool],
) {
    let prefs = mcp_store::get_mcp_preferences(state)
        .await
        .unwrap_or_default();
    if !prefs.sync_disabled_to_opencode || server.enabled_tools.contains(&"opencode".to_string()) {
        return;
    }
    if let Some(tool) = runtime_tool_by_key("opencode", custom_tools) {
        let db = state.db();
        let _ = remove_server_from_tool_async(&db, &server.name, &tool).await;
    }
}

/// Helper: Sync all MCP servers NOT linked to opencode as disabled (enabled=false) in opencode config
async fn sync_opencode_disabled(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    servers: &[McpServer],
    custom_tools: &[CustomTool],
) {
    let Some(tool) = runtime_tool_by_key("opencode", custom_tools) else {
        return;
    };
    if !is_tool_installed_with_db_async(db, &tool).await {
        return;
    }
    for server in servers {
        if !server.enabled_tools.contains(&"opencode".to_string()) {
            let _ = sync_server_to_tool_with_enabled_async(db, server, &tool, false).await;
        }
    }
}

/// Helper: Remove all MCP servers NOT linked to opencode from opencode config
async fn cleanup_opencode_disabled(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    servers: &[McpServer],
    custom_tools: &[CustomTool],
) {
    let Some(tool) = runtime_tool_by_key("opencode", custom_tools) else {
        return;
    };
    for server in servers {
        if !server.enabled_tools.contains(&"opencode".to_string()) {
            let _ = remove_server_from_tool_async(db, &server.name, &tool).await;
        }
    }
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
    use crate::coding::tools::path_utils::{normalize_path, to_storage_path};

    // Trim whitespace from all inputs
    let key = key.trim().to_string();
    let display_name = displayName.trim().to_string();
    let mcp_format = mcpConfigFormat.trim().to_lowercase();
    let mcp_field_name = mcpField.trim().to_string();

    // Normalize the MCP config path
    let normalized_mcp_path = normalize_path(mcpConfigPath.trim());
    let mcp_path = to_storage_path(&normalized_mcp_path);

    // Normalize the detect dir if provided
    let detect_dir = relativeDetectDir.map(|s| {
        let normalized = normalize_path(s.trim());
        to_storage_path(&normalized)
    });

    // Validate key format
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Key must contain only letters, numbers, and underscores".to_string());
    }

    // Validate mcp_format
    if mcp_format != "json" && mcp_format != "toml" && mcp_format != "jsonc" {
        return Err("MCP config format must be 'json', 'jsonc' or 'toml'".to_string());
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

// ==================== Favorite MCP ====================

/// List all favorite MCPs
#[tauri::command]
pub async fn mcp_list_favorites(state: State<'_, DbState>) -> Result<Vec<FavoriteMcpDto>, String> {
    let favorites = mcp_store::get_favorite_mcps(&state).await?;

    Ok(favorites
        .into_iter()
        .map(|f| FavoriteMcpDto {
            id: f.id,
            name: f.name,
            server_type: f.server_type,
            server_config: f.server_config,
            description: f.description,
            tags: f.tags,
            is_preset: f.is_preset,
            created_at: f.created_at,
            updated_at: f.updated_at,
        })
        .collect())
}

/// Create or update a favorite MCP (upsert by name)
#[tauri::command]
pub async fn mcp_upsert_favorite(
    state: State<'_, DbState>,
    input: FavoriteMcpInput,
) -> Result<FavoriteMcpDto, String> {
    let now = now_ms();

    // Check if a favorite with the same name exists
    let existing = mcp_store::get_favorite_mcp_by_name(&state, &input.name).await?;

    let fav = if let Some(existing) = existing {
        // Update existing
        FavoriteMcp {
            id: existing.id,
            name: input.name,
            server_type: input.server_type,
            server_config: input.server_config,
            description: input.description,
            tags: input.tags,
            is_preset: false,
            created_at: existing.created_at,
            updated_at: now,
        }
    } else {
        // Create new
        FavoriteMcp {
            id: String::new(),
            name: input.name,
            server_type: input.server_type,
            server_config: input.server_config,
            description: input.description,
            tags: input.tags,
            is_preset: false,
            created_at: now,
            updated_at: now,
        }
    };

    let id = mcp_store::upsert_favorite_mcp(&state, &fav).await?;

    Ok(FavoriteMcpDto {
        id,
        name: fav.name,
        server_type: fav.server_type,
        server_config: fav.server_config,
        description: fav.description,
        tags: fav.tags,
        is_preset: fav.is_preset,
        created_at: fav.created_at,
        updated_at: fav.updated_at,
    })
}

/// Delete a favorite MCP
#[tauri::command]
#[allow(non_snake_case)]
pub async fn mcp_delete_favorite(
    state: State<'_, DbState>,
    favoriteId: String,
) -> Result<(), String> {
    mcp_store::delete_favorite_mcp(&state, &favoriteId).await
}

/// Default favorite MCP presets seeded into a user's library.
const DEFAULT_FAVORITE_MCP_PRESETS: &[(&str, &str, &str)] = &[
    (
        "mcp-server-fetch",
        "stdio",
        r#"{"command":"uvx","args":["mcp-server-fetch"]}"#,
    ),
    (
        "@modelcontextprotocol/server-time",
        "stdio",
        r#"{"command":"npx","args":["-y","@modelcontextprotocol/server-time"]}"#,
    ),
    (
        "@modelcontextprotocol/server-memory",
        "stdio",
        r#"{"command":"npx","args":["-y","@modelcontextprotocol/server-memory"]}"#,
    ),
    (
        "@modelcontextprotocol/server-sequential-thinking",
        "stdio",
        r#"{"command":"npx","args":["-y","@modelcontextprotocol/server-sequential-thinking"]}"#,
    ),
    (
        "@upstash/context7-mcp",
        "stdio",
        r#"{"command":"npx","args":["-y","@upstash/context7-mcp"]}"#,
    ),
    (
        "chrome-devtools",
        "stdio",
        r#"{"command":"npx","args":["-y","chrome-devtools-mcp@latest"]}"#,
    ),
    (
        "playwright",
        "stdio",
        r#"{"command":"npx","args":["@playwright/mcp@latest"]}"#,
    ),
];

#[tauri::command]
pub async fn mcp_init_default_favorites(state: State<'_, DbState>) -> Result<usize, String> {
    let prefs = mcp_store::get_mcp_preferences(&state).await?;
    let now = now_ms();
    let mut inserted_count = 0;

    for (name, server_type, config_json) in DEFAULT_FAVORITE_MCP_PRESETS {
        if mcp_store::get_favorite_mcp_by_name(&state, name)
            .await?
            .is_some()
        {
            continue;
        }

        let server_config: serde_json::Value = serde_json::from_str(config_json)
            .map_err(|e| format!("Invalid preset config: {}", e))?;

        let fav = FavoriteMcp {
            id: String::new(),
            name: name.to_string(),
            server_type: server_type.to_string(),
            server_config,
            description: None,
            tags: vec![],
            is_preset: true,
            created_at: now,
            updated_at: now,
        };
        mcp_store::upsert_favorite_mcp(&state, &fav).await?;
        inserted_count += 1;
    }

    let mut prefs = prefs;
    prefs.favorites_initialized = true;
    prefs.updated_at = now;
    mcp_store::save_mcp_preferences(&state, &prefs).await?;

    Ok(inserted_count)
}
