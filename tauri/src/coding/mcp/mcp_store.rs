//! MCP Server database operations
//!
//! Provides CRUD operations for MCP server management.
//! Uses backtick-escaped record references instead of type::thing() to avoid
//! UUID parsing issues across SurrealDB versions.

use serde_json::Value;

use super::adapter::{
    from_db_favorite_mcp, from_db_mcp_preferences, from_db_mcp_server, remove_sync_detail,
    set_sync_detail, to_clean_mcp_server_payload, to_mcp_preferences_payload,
};
use super::command_normalize;
use super::types::{now_ms, FavoriteMcp, McpPreferences, McpServer, McpSyncDetail};
use crate::coding::db_id::db_new_id;
use crate::db::helpers::{db_delete, db_get, db_list, db_max_i64, db_put, db_query_by_field};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::SqliteDbState;

// ==================== MCP Server CRUD ====================

/// Get all MCP servers ordered by sort_index
pub async fn get_mcp_servers(state: &SqliteDbState) -> Result<Vec<McpServer>, String> {
    state.with_conn(|conn| {
        let order = OrderSpec::new(vec![
            OrderField::json_integer("sort_index", OrderDirection::Asc)?,
            OrderField::id(OrderDirection::Asc),
        ]);
        let records = db_list(conn, DbTable::McpServer, Some(&order))?;
        Ok(records.into_iter().map(from_db_mcp_server).collect())
    })
}

/// Get a single MCP server by ID
pub async fn get_mcp_server_by_id(
    state: &SqliteDbState,
    server_id: &str,
) -> Result<Option<McpServer>, String> {
    state.with_conn(|conn| Ok(db_get(conn, DbTable::McpServer, server_id)?.map(from_db_mcp_server)))
}

/// Get MCP server by name
pub async fn get_mcp_server_by_name(
    state: &SqliteDbState,
    name: &str,
) -> Result<Option<McpServer>, String> {
    state.with_conn(|conn| {
        let records = db_query_by_field(
            conn,
            DbTable::McpServer,
            &JsonFieldPath::new("name")?,
            &Value::String(name.to_string()),
            None,
            Some(1),
        )?;
        Ok(records.into_iter().next().map(from_db_mcp_server))
    })
}

/// Create or update an MCP server
pub async fn upsert_mcp_server(
    state: &SqliteDbState,
    server: &McpServer,
) -> Result<String, String> {
    // Normalize server_config: remove cmd /c wrapper for database storage (only for stdio type)
    let normalized_config = if server.server_type == "stdio" {
        command_normalize::unwrap_cmd_c(&server.server_config)
    } else {
        server.server_config.clone()
    };

    let id = if server.id.is_empty() {
        db_new_id()
    } else {
        server.id.clone()
    };
    state.with_conn(|conn| {
        let mut sqlite_server = server.clone();
        sqlite_server.id = id.clone();
        sqlite_server.server_config = normalized_config;
        if server.id.is_empty() {
            let max_index =
                db_max_i64(conn, DbTable::McpServer, &JsonFieldPath::new("sort_index")?)?
                    .unwrap_or(-1) as i32;
            sqlite_server.sort_index = max_index + 1;
        }
        db_put(
            conn,
            DbTable::McpServer,
            &id,
            &to_clean_mcp_server_payload(&sqlite_server),
        )
    })?;
    Ok(id)
}

/// Delete an MCP server
pub async fn delete_mcp_server(state: &SqliteDbState, server_id: &str) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::McpServer, server_id).map(|_| ()))
}

/// Update user-managed metadata for an MCP server without touching sync state.
pub async fn update_mcp_server_metadata(
    state: &SqliteDbState,
    server_id: &str,
    user_group: Option<String>,
    user_note: Option<String>,
) -> Result<(), String> {
    if let Some(mut server) = get_mcp_server_by_id(state, server_id).await? {
        server.user_group = user_group;
        server.user_note = user_note;
        server.updated_at = now_ms();
        upsert_mcp_server(state, &server).await?;
    }
    Ok(())
}

/// Reorder MCP servers by updating sort_index for each server
pub async fn reorder_mcp_servers(state: &SqliteDbState, ids: &[String]) -> Result<(), String> {
    state.with_conn(|conn| {
        for (index, id) in ids.iter().enumerate() {
            if let Some(mut record) = db_get(conn, DbTable::McpServer, id)? {
                if let Some(object) = record.as_object_mut() {
                    object.insert(
                        "sort_index".to_string(),
                        Value::Number(serde_json::Number::from(index as i64)),
                    );
                }
                db_put(conn, DbTable::McpServer, id, &record)?;
            }
        }
        Ok(())
    })
}

// ==================== Sync Details Operations ====================

/// Update sync detail for a specific tool
pub async fn update_sync_detail(
    state: &SqliteDbState,
    server_id: &str,
    detail: &McpSyncDetail,
) -> Result<(), String> {
    let mut server = get_mcp_server_by_id(state, server_id)
        .await?
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;
    server.sync_details = Some(set_sync_detail(&server.sync_details, &detail.tool, detail));
    server.updated_at = now_ms();
    upsert_mcp_server(state, &server).await?;
    Ok(())
}

/// Remove sync detail for a specific tool
pub async fn delete_sync_detail(
    state: &SqliteDbState,
    server_id: &str,
    tool: &str,
) -> Result<(), String> {
    let Some(mut server) = get_mcp_server_by_id(state, server_id).await? else {
        return Ok(());
    };
    server.sync_details = Some(remove_sync_detail(&server.sync_details, tool));
    server.updated_at = now_ms();
    upsert_mcp_server(state, &server).await?;
    Ok(())
}

/// Toggle a tool's enabled state for an MCP server
pub async fn toggle_tool_enabled(
    state: &SqliteDbState,
    server_id: &str,
    tool_key: &str,
) -> Result<bool, String> {
    let mut server = get_mcp_server_by_id(state, server_id)
        .await?
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;
    let mut enabled_tools = server.enabled_tools.clone();
    let is_now_enabled = if enabled_tools.contains(&tool_key.to_string()) {
        enabled_tools.retain(|tool| tool != tool_key);
        false
    } else {
        enabled_tools.push(tool_key.to_string());
        true
    };
    server.enabled_tools = enabled_tools;
    server.updated_at = now_ms();
    upsert_mcp_server(state, &server).await?;
    Ok(is_now_enabled)
}

// ==================== MCP Preferences ====================

/// Get MCP preferences (singleton record)
pub async fn get_mcp_preferences(state: &SqliteDbState) -> Result<McpPreferences, String> {
    state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::McpPreferences, "default")?
            .map(from_db_mcp_preferences)
            .unwrap_or_default())
    })
}

/// Save MCP preferences (singleton record)
pub async fn save_mcp_preferences(
    state: &SqliteDbState,
    prefs: &McpPreferences,
) -> Result<(), String> {
    state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::McpPreferences,
            "default",
            &to_mcp_preferences_payload(prefs),
        )
    })
}

// ==================== Favorite MCP CRUD ====================

/// Get all favorite MCP servers
pub async fn get_favorite_mcps(state: &SqliteDbState) -> Result<Vec<FavoriteMcp>, String> {
    state.with_conn(|conn| {
        let order = OrderSpec::new(vec![
            OrderField::json_integer("created_at", OrderDirection::Desc)?,
            OrderField::id(OrderDirection::Asc),
        ]);
        let records = db_list(conn, DbTable::FavoriteMcp, Some(&order))?;
        Ok(records.into_iter().map(from_db_favorite_mcp).collect())
    })
}

/// Get a favorite MCP by name
pub async fn get_favorite_mcp_by_name(
    state: &SqliteDbState,
    name: &str,
) -> Result<Option<FavoriteMcp>, String> {
    state.with_conn(|conn| {
        let records = db_query_by_field(
            conn,
            DbTable::FavoriteMcp,
            &JsonFieldPath::new("name")?,
            &Value::String(name.to_string()),
            None,
            Some(1),
        )?;
        Ok(records.into_iter().next().map(from_db_favorite_mcp))
    })
}

/// Create or update a favorite MCP
pub async fn upsert_favorite_mcp(
    state: &SqliteDbState,
    fav: &FavoriteMcp,
) -> Result<String, String> {
    let id = if fav.id.is_empty() {
        db_new_id()
    } else {
        fav.id.clone()
    };
    let mut payload = serde_json::to_value(fav).map_err(|e| e.to_string())?;
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("id");
    }
    state.with_conn(|conn| db_put(conn, DbTable::FavoriteMcp, &id, &payload))?;
    Ok(id)
}

/// Delete a favorite MCP
pub async fn delete_favorite_mcp(state: &SqliteDbState, id: &str) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::FavoriteMcp, id).map(|_| ()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn sqlite_mcp_store_round_trips_servers_preferences_and_favorites() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        let server = McpServer {
            id: String::new(),
            name: "Server A".to_string(),
            server_type: "stdio".to_string(),
            server_config: json!({"command": "cmd", "args": ["/c", "node"]}),
            enabled_tools: vec!["claude".to_string()],
            sync_details: None,
            description: None,
            sort_index: 0,
            created_at: 1,
            updated_at: 2,
            user_group: None,
            user_note: None,
            tags: Vec::new(),
            timeout: None,
        };
        let server_id = upsert_mcp_server(&sqlite_state, &server)
            .await
            .expect("upsert server");
        let saved_server = get_mcp_server_by_id(&sqlite_state, &server_id)
            .await
            .expect("read server")
            .expect("server exists");
        assert_eq!(saved_server.name, "Server A");
        assert_eq!(saved_server.sort_index, 0);

        let prefs = McpPreferences {
            id: "default".to_string(),
            show_in_tray: true,
            preferred_tools: vec!["claude".to_string()],
            favorites_initialized: true,
            sync_disabled_to_opencode: true,
            updated_at: 9,
        };
        save_mcp_preferences(&sqlite_state, &prefs)
            .await
            .expect("save preferences");
        let prefs = get_mcp_preferences(&sqlite_state)
            .await
            .expect("read preferences");
        assert!(prefs.show_in_tray);

        let favorite = FavoriteMcp {
            id: String::new(),
            name: "Favorite A".to_string(),
            server_type: "http".to_string(),
            server_config: json!({"url": "https://example.com/mcp"}),
            description: None,
            tags: Vec::new(),
            is_preset: false,
            created_at: 5,
            updated_at: 6,
        };
        let favorite_id = upsert_favorite_mcp(&sqlite_state, &favorite)
            .await
            .expect("save favorite");
        assert!(!favorite_id.is_empty());
        let favorites = get_favorite_mcps(&sqlite_state)
            .await
            .expect("read favorites");
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].name, "Favorite A");
    }
}
