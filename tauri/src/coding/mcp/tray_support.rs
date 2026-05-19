//! MCP Server Tray Support
//!
//! Provides data structures and functions for system tray menu integration.

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::mcp_store;
use crate::coding::tools::{custom_store, get_mcp_runtime_tools, is_tool_installed_with_db_async};
use crate::SqliteDbState;

/// Tray data for MCP servers section
#[derive(Debug)]
pub struct TrayMcpData {
    pub title: String,
    pub items: Vec<TrayMcpServerItem>,
}

/// Single MCP server item in tray menu
#[derive(Debug)]
pub struct TrayMcpServerItem {
    pub id: String,
    pub display_name: String,
    pub tools: Vec<TrayMcpToolItem>,
}

/// Tool toggle item within MCP server submenu
#[derive(Debug)]
pub struct TrayMcpToolItem {
    pub tool_key: String,
    pub display_name: String,
    pub is_enabled: bool,
    pub is_installed: bool,
}

/// Check if MCP section should be shown in tray
pub async fn is_mcp_enabled_for_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    let state = app.state::<SqliteDbState>();
    let prefs = mcp_store::get_mcp_preferences(&state)
        .await
        .unwrap_or_default();
    prefs.show_in_tray
}

/// Get MCP data for tray menu
pub async fn get_mcp_tray_data<R: Runtime>(app: &AppHandle<R>) -> Result<TrayMcpData, String> {
    let state = app.state::<SqliteDbState>();

    // Get all MCP servers
    let servers = mcp_store::get_mcp_servers(&state).await?;

    // Get custom tools for MCP tool list
    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let mcp_tools = get_mcp_runtime_tools(&custom_tools);
    let db = state.db();

    let mut items = Vec::new();

    for server in servers {
        let mut tools = Vec::new();

        for tool in &mcp_tools {
            let is_enabled = server.enabled_tools.contains(&tool.key);
            let is_installed = is_tool_installed_with_db_async(&db, tool).await;

            tools.push(TrayMcpToolItem {
                tool_key: tool.key.clone(),
                display_name: super::mcp_tool_display_name(&tool.key, &tool.display_name),
                is_enabled,
                is_installed,
            });
        }

        items.push(TrayMcpServerItem {
            id: server.id.clone(),
            display_name: server.name.clone(),
            tools,
        });
    }

    Ok(TrayMcpData {
        title: "──── MCP Servers ────".to_string(),
        items,
    })
}

/// Toggle MCP server's tool from tray menu
pub async fn apply_mcp_tool_toggle<R: Runtime>(
    app: &AppHandle<R>,
    server_id: &str,
    tool_key: &str,
) -> Result<(), String> {
    let state = app.state::<SqliteDbState>();

    // Toggle the tool
    let is_enabled = mcp_store::toggle_tool_enabled(&state, server_id, tool_key).await?;

    // Get the server and tool
    let server = mcp_store::get_mcp_server_by_id(&state, server_id)
        .await?
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;

    let custom_tools = custom_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let tool = crate::coding::tools::runtime_tool_by_key(tool_key, &custom_tools)
        .ok_or_else(|| format!("Tool not found: {}", tool_key))?;
    let db = state.db();

    // Sync or remove based on new state
    if is_enabled {
        match super::config_sync::sync_server_to_tool_async(&db, &server, &tool).await {
            Ok(detail) => {
                mcp_store::update_sync_detail(&state, server_id, &detail).await?;
            }
            Err(e) => {
                let detail = super::types::McpSyncDetail {
                    tool: tool_key.to_string(),
                    status: "error".to_string(),
                    synced_at: Some(super::types::now_ms()),
                    error_message: Some(e.clone()),
                };
                mcp_store::update_sync_detail(&state, server_id, &detail).await?;
                return Err(e);
            }
        }
    } else {
        let _ = super::config_sync::remove_server_from_tool_async(&db, &server.name, &tool).await;
        mcp_store::delete_sync_detail(&state, server_id, tool_key).await?;
    }

    // Emit config-changed event (from tray)
    let _ = app.emit("config-changed", "tray");
    let _ = app.emit("mcp-changed", "tray");

    Ok(())
}
