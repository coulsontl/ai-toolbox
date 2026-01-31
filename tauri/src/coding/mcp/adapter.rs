//! Database adapters for MCP Server types
//!
//! Provides conversion between database records and Rust types.

use serde_json::Value;

use crate::coding::db_extract_id;
use super::types::{McpPreferences, McpServer, McpSyncDetail, McpSyncDetailDto};

/// Convert database record to McpServer struct
pub fn from_db_mcp_server(value: Value) -> McpServer {
    let enabled_tools: Vec<String> = value
        .get("enabled_tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let tags: Vec<String> = value
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let sync_details = value.get("sync_details").cloned().filter(|v| !v.is_null());

    McpServer {
        id: db_extract_id(&value),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        server_type: value
            .get("server_type")
            .and_then(|v| v.as_str())
            .unwrap_or("stdio")
            .to_string(),
        server_config: value
            .get("server_config")
            .cloned()
            .unwrap_or(serde_json::json!({})),
        enabled_tools,
        sync_details,
        description: value
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        tags,
        sort_index: value.get("sort_index").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        created_at: value.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0),
        updated_at: value.get("updated_at").and_then(|v| v.as_i64()).unwrap_or(0),
    }
}

/// Convert McpServer to clean database payload (without id)
pub fn to_clean_mcp_server_payload(server: &McpServer) -> Value {
    serde_json::json!({
        "name": server.name,
        "server_type": server.server_type,
        "server_config": server.server_config,
        "enabled_tools": server.enabled_tools,
        "sync_details": server.sync_details,
        "description": server.description,
        "tags": server.tags,
        "sort_index": server.sort_index,
        "created_at": server.created_at,
        "updated_at": server.updated_at,
    })
}

/// Parse sync details from McpServer's sync_details JSON
pub fn parse_sync_details(server: &McpServer) -> Vec<McpSyncDetail> {
    let Some(details) = &server.sync_details else {
        return Vec::new();
    };
    let Some(obj) = details.as_object() else {
        return Vec::new();
    };

    obj.iter()
        .map(|(tool_key, entry)| McpSyncDetail {
            tool: tool_key.clone(),
            status: entry
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_string(),
            synced_at: entry.get("synced_at").and_then(|v| v.as_i64()),
            error_message: entry
                .get("error_message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
        .collect()
}

/// Parse sync details to DTO format
pub fn parse_sync_details_dto(server: &McpServer) -> Vec<McpSyncDetailDto> {
    parse_sync_details(server)
        .into_iter()
        .map(|d| McpSyncDetailDto {
            tool: d.tool,
            status: d.status,
            synced_at: d.synced_at,
            error_message: d.error_message,
        })
        .collect()
}

/// Set a sync detail in sync_details JSON
pub fn set_sync_detail(existing: &Option<Value>, tool: &str, detail: &McpSyncDetail) -> Value {
    let mut obj = existing
        .as_ref()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    obj.insert(
        tool.to_string(),
        serde_json::json!({
            "status": detail.status,
            "synced_at": detail.synced_at,
            "error_message": detail.error_message,
        }),
    );

    Value::Object(obj)
}

/// Remove a tool from sync_details JSON
pub fn remove_sync_detail(existing: &Option<Value>, tool: &str) -> Value {
    let mut obj = existing
        .as_ref()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    obj.remove(tool);
    Value::Object(obj)
}

/// Convert database record to McpPreferences struct
pub fn from_db_mcp_preferences(value: Value) -> McpPreferences {
    McpPreferences {
        id: db_extract_id(&value),
        show_in_tray: value
            .get("show_in_tray")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        preferred_tools: value
            .get("preferred_tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        updated_at: value.get("updated_at").and_then(|v| v.as_i64()).unwrap_or(0),
    }
}

/// Convert McpPreferences to database payload
pub fn to_mcp_preferences_payload(prefs: &McpPreferences) -> Value {
    serde_json::json!({
        "show_in_tray": prefs.show_in_tray,
        "preferred_tools": prefs.preferred_tools,
        "updated_at": prefs.updated_at,
    })
}
