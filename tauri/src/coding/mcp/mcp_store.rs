//! MCP Server database operations
//!
//! Provides CRUD operations for MCP server management.
//! Uses backtick-escaped record references instead of type::thing() to avoid
//! UUID parsing issues across SurrealDB versions.

use serde_json::Value;

use crate::coding::db_id::{db_record_id, db_new_id};
use crate::DbState;
use super::adapter::{
    from_db_mcp_preferences, from_db_mcp_server, from_db_favorite_mcp, remove_sync_detail, set_sync_detail,
    to_clean_mcp_server_payload, to_mcp_preferences_payload,
};
use super::command_normalize;
use super::types::{McpPreferences, McpServer, McpSyncDetail, FavoriteMcp, now_ms};

// ==================== MCP Server CRUD ====================

/// Get all MCP servers ordered by sort_index
pub async fn get_mcp_servers(state: &DbState) -> Result<Vec<McpServer>, String> {
    let db = state.0.lock().await;

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM mcp_server ORDER BY sort_index ASC")
        .await
        .map_err(|e| format!("Failed to query MCP servers: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_mcp_server).collect())
}

/// Get a single MCP server by ID
pub async fn get_mcp_server_by_id(state: &DbState, server_id: &str) -> Result<Option<McpServer>, String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("mcp_server", server_id);

    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query MCP server: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.first().map(|r| from_db_mcp_server(r.clone())))
}

/// Get MCP server by name
pub async fn get_mcp_server_by_name(state: &DbState, name: &str) -> Result<Option<McpServer>, String> {
    let db = state.0.lock().await;
    let name_owned = name.to_string();

    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM mcp_server WHERE name = $name LIMIT 1",
        )
        .bind(("name", name_owned))
        .await
        .map_err(|e| format!("Failed to query MCP server by name: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.first().map(|r| from_db_mcp_server(r.clone())))
}

/// Create or update an MCP server
pub async fn upsert_mcp_server(state: &DbState, server: &McpServer) -> Result<String, String> {
    let db = state.0.lock().await;

    // Normalize server_config: remove cmd /c wrapper for database storage (only for stdio type)
    let normalized_config = if server.server_type == "stdio" {
        command_normalize::unwrap_cmd_c(&server.server_config)
    } else {
        server.server_config.clone()
    };

    if server.id.is_empty() {
        // Get max sort_index for new server
        let mut max_result = db
            .query("SELECT sort_index FROM mcp_server ORDER BY sort_index DESC LIMIT 1")
            .await
            .map_err(|e| format!("Failed to query max sort_index: {}", e))?;
        let max_records: Vec<Value> = max_result.take(0).map_err(|e| e.to_string())?;
        let max_index = max_records
            .first()
            .and_then(|v| v.get("sort_index"))
            .and_then(|v| v.as_i64())
            .unwrap_or(-1) as i32;

        // Create new server with sort_index = max + 1 and normalized config
        let mut new_server = server.clone();
        new_server.sort_index = max_index + 1;
        new_server.server_config = normalized_config;
        let payload = to_clean_mcp_server_payload(&new_server);

        let id = db_new_id();
        let record_id = db_record_id("mcp_server", &id);
        db.query(&format!("CREATE {} CONTENT $data", record_id))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to create MCP server: {}", e))?;
        Ok(id)
    } else {
        // Update existing server with normalized config
        let mut updated_server = server.clone();
        updated_server.server_config = normalized_config;
        let payload = to_clean_mcp_server_payload(&updated_server);
        let record_id = db_record_id("mcp_server", &server.id);
        db.query(&format!("UPDATE {} CONTENT $data", record_id))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to update MCP server: {}", e))?;
        Ok(server.id.clone())
    }
}

/// Delete an MCP server
pub async fn delete_mcp_server(state: &DbState, server_id: &str) -> Result<(), String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("mcp_server", server_id);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete MCP server: {}", e))?;

    Ok(())
}

/// Reorder MCP servers by updating sort_index for each server
pub async fn reorder_mcp_servers(state: &DbState, ids: &[String]) -> Result<(), String> {
    let db = state.0.lock().await;

    for (index, id) in ids.iter().enumerate() {
        let record_id = db_record_id("mcp_server", id);
        db.query(&format!("UPDATE {} SET sort_index = $index", record_id))
            .bind(("index", index as i32))
            .await
            .map_err(|e| format!("Failed to reorder MCP servers: {}", e))?;
    }

    Ok(())
}

// ==================== Sync Details Operations ====================

/// Update sync detail for a specific tool
pub async fn update_sync_detail(
    state: &DbState,
    server_id: &str,
    detail: &McpSyncDetail,
) -> Result<(), String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("mcp_server", server_id);

    // Get existing server
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| e.to_string())?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    let server = records
        .first()
        .map(|r| from_db_mcp_server(r.clone()))
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;

    // Update sync_details
    let new_sync_details = set_sync_detail(&server.sync_details, &detail.tool, detail);

    // Save updates
    db.query(&format!("UPDATE {} SET sync_details = $sync_details, updated_at = $updated_at", record_id))
        .bind(("sync_details", new_sync_details))
        .bind(("updated_at", now_ms()))
        .await
        .map_err(|e| format!("Failed to update sync detail: {}", e))?;

    Ok(())
}

/// Remove sync detail for a specific tool
pub async fn delete_sync_detail(state: &DbState, server_id: &str, tool: &str) -> Result<(), String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("mcp_server", server_id);
    let tool_owned = tool.to_string();

    // Get existing server
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| e.to_string())?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    let Some(server) = records.first().map(|r| from_db_mcp_server(r.clone())) else {
        return Ok(()); // Server not found, nothing to delete
    };

    // Update sync_details
    let new_sync_details = remove_sync_detail(&server.sync_details, &tool_owned);

    // Save updates
    db.query(&format!("UPDATE {} SET sync_details = $sync_details, updated_at = $updated_at", record_id))
        .bind(("sync_details", new_sync_details))
        .bind(("updated_at", now_ms()))
        .await
        .map_err(|e| format!("Failed to delete sync detail: {}", e))?;

    Ok(())
}

/// Toggle a tool's enabled state for an MCP server
pub async fn toggle_tool_enabled(
    state: &DbState,
    server_id: &str,
    tool_key: &str,
) -> Result<bool, String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("mcp_server", server_id);

    // Get existing server
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| e.to_string())?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    let server = records
        .first()
        .map(|r| from_db_mcp_server(r.clone()))
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;

    // Toggle tool in enabled_tools
    let mut enabled_tools = server.enabled_tools.clone();
    let is_now_enabled = if enabled_tools.contains(&tool_key.to_string()) {
        enabled_tools.retain(|t| t != tool_key);
        false
    } else {
        enabled_tools.push(tool_key.to_string());
        true
    };

    // Save updates
    db.query(&format!("UPDATE {} SET enabled_tools = $enabled_tools, updated_at = $updated_at", record_id))
        .bind(("enabled_tools", enabled_tools))
        .bind(("updated_at", now_ms()))
        .await
        .map_err(|e| format!("Failed to toggle tool: {}", e))?;

    Ok(is_now_enabled)
}

// ==================== MCP Preferences ====================

/// Get MCP preferences (singleton record)
pub async fn get_mcp_preferences(state: &DbState) -> Result<McpPreferences, String> {
    let db = state.0.lock().await;

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM mcp_preferences:`default` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query MCP preferences: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;

    if let Some(record) = records.first() {
        Ok(from_db_mcp_preferences(record.clone()))
    } else {
        Ok(McpPreferences::default())
    }
}

/// Save MCP preferences (singleton record)
pub async fn save_mcp_preferences(state: &DbState, prefs: &McpPreferences) -> Result<(), String> {
    let db = state.0.lock().await;
    let payload = to_mcp_preferences_payload(prefs);

    db.query("UPSERT mcp_preferences:`default` CONTENT $data")
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to save MCP preferences: {}", e))?;

    Ok(())
}

// ==================== Favorite MCP CRUD ====================

/// Get all favorite MCP servers
pub async fn get_favorite_mcps(state: &DbState) -> Result<Vec<FavoriteMcp>, String> {
    let db = state.0.lock().await;

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM favorite_mcp ORDER BY created_at DESC")
        .await
        .map_err(|e| format!("Failed to query favorite MCPs: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_favorite_mcp).collect())
}

/// Get a favorite MCP by name
pub async fn get_favorite_mcp_by_name(state: &DbState, name: &str) -> Result<Option<FavoriteMcp>, String> {
    let db = state.0.lock().await;
    let name_owned = name.to_string();

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM favorite_mcp WHERE name = $name LIMIT 1")
        .bind(("name", name_owned))
        .await
        .map_err(|e| format!("Failed to query favorite MCP by name: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.first().map(|v| from_db_favorite_mcp(v.clone())))
}

/// Create or update a favorite MCP
pub async fn upsert_favorite_mcp(state: &DbState, fav: &FavoriteMcp) -> Result<String, String> {
    let db = state.0.lock().await;

    // Remove id field for database payload
    let mut payload = serde_json::to_value(fav).map_err(|e| e.to_string())?;
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("id");
    }

    if fav.id.is_empty() {
        // Create new
        let id = db_new_id();
        let record_id = db_record_id("favorite_mcp", &id);
        db.query(&format!("CREATE {} CONTENT $data", record_id))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to create favorite MCP: {}", e))?;
        Ok(id)
    } else {
        // Update existing
        let record_id = db_record_id("favorite_mcp", &fav.id);
        db.query(&format!("UPDATE {} CONTENT $data", record_id))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to update favorite MCP: {}", e))?;
        Ok(fav.id.clone())
    }
}

/// Delete a favorite MCP
pub async fn delete_favorite_mcp(state: &DbState, id: &str) -> Result<(), String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("favorite_mcp", id);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete favorite MCP: {}", e))?;

    Ok(())
}
