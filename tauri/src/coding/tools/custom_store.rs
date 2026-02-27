//! Custom tool database operations
//!
//! Provides CRUD operations for user-defined custom tools.

use serde_json::Value;

use crate::coding::db_extract_id;
use crate::coding::db_record_id;
use crate::DbState;
use super::types::CustomTool;

/// Convert database record to CustomTool struct
pub fn from_db_custom_tool(value: Value) -> CustomTool {
    let key = db_extract_id(&value);
    CustomTool {
        key,
        display_name: value
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        relative_skills_dir: value
            .get("relative_skills_dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        relative_detect_dir: value
            .get("relative_detect_dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        force_copy: value.get("force_copy").and_then(|v| v.as_bool()).unwrap_or(false),
        mcp_config_path: value
            .get("mcp_config_path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        mcp_config_format: value
            .get("mcp_config_format")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        mcp_field: value
            .get("mcp_field")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        created_at: value.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0),
    }
}

/// Get all custom tools
pub async fn get_custom_tools(state: &DbState) -> Result<Vec<CustomTool>, String> {
    let db = state.0.lock().await;

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM custom_tool ORDER BY display_name ASC")
        .await
        .map_err(|e| format!("Failed to query custom tools: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;

    // Filter out any malformed records
    Ok(records
        .into_iter()
        .filter_map(|v| {
            let tool = from_db_custom_tool(v);
            // Skip records with empty key (likely corrupted)
            if tool.key.is_empty() {
                None
            } else {
                Some(tool)
            }
        })
        .collect())
}

/// Get custom tools that support Skills (have valid relative_skills_dir, not a root directory)
pub async fn get_skills_custom_tools(state: &DbState) -> Result<Vec<CustomTool>, String> {
    let tools = get_custom_tools(state).await?;
    Ok(tools
        .into_iter()
        .filter(|t| {
            if let Some(ref dir) = t.relative_skills_dir {
                // Skip if the path is a root directory (home or appdata)
                !super::path_utils::is_root_directory(dir)
            } else {
                false
            }
        })
        .collect())
}

/// Get custom tools that support MCP (have valid mcp_config_path, not a root directory)
pub async fn get_mcp_custom_tools(state: &DbState) -> Result<Vec<CustomTool>, String> {
    let tools = get_custom_tools(state).await?;
    Ok(tools
        .into_iter()
        .filter(|t| {
            if let (Some(ref path), Some(_format), Some(_field)) =
                (&t.mcp_config_path, &t.mcp_config_format, &t.mcp_field)
            {
                // Skip if the path is a root directory (home or appdata)
                !super::path_utils::is_root_directory(path)
            } else {
                false
            }
        })
        .collect())
}

/// Get a custom tool by key
pub async fn get_custom_tool_by_key(state: &DbState, key: &str) -> Result<Option<CustomTool>, String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("custom_tool", key);

    let mut result = db
        .query(&format!("SELECT *, type::string(id) as id FROM {} LIMIT 1", record_id))
        .await
        .map_err(|e| format!("Failed to query custom tool: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;

    Ok(records.into_iter().next().map(from_db_custom_tool))
}

/// Save a custom tool (create or update), merging with existing fields
pub async fn save_custom_tool(state: &DbState, tool: &CustomTool) -> Result<(), String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("custom_tool", &tool.key);

    db.query(&format!("UPSERT {} SET display_name = $display_name, relative_skills_dir = $skills_dir, relative_detect_dir = $detect_dir, force_copy = $force_copy, mcp_config_path = $mcp_path, mcp_config_format = $mcp_format, mcp_field = $mcp_field, created_at = $created_at", record_id))
        .bind(("display_name", tool.display_name.clone()))
        .bind(("skills_dir", tool.relative_skills_dir.clone()))
        .bind(("detect_dir", tool.relative_detect_dir.clone()))
        .bind(("force_copy", tool.force_copy))
        .bind(("mcp_path", tool.mcp_config_path.clone()))
        .bind(("mcp_format", tool.mcp_config_format.clone()))
        .bind(("mcp_field", tool.mcp_field.clone()))
        .bind(("created_at", tool.created_at))
        .await
        .map_err(|e| format!("Failed to save custom tool: {}", e))?;

    Ok(())
}

/// Save only skills-related fields, preserving MCP fields if they exist
pub async fn save_custom_tool_skills_fields(
    state: &DbState,
    key: &str,
    display_name: &str,
    relative_skills_dir: Option<String>,
    relative_detect_dir: Option<String>,
    force_copy: bool,
    created_at: i64,
) -> Result<(), String> {
    // First check if the tool already exists
    let existing = get_custom_tool_by_key(state, key).await?;

    let db = state.0.lock().await;
    let record_id = db_record_id("custom_tool", key);

    // Preserve existing MCP fields
    let (mcp_path, mcp_format, mcp_field) = match existing {
        Some(e) => (e.mcp_config_path, e.mcp_config_format, e.mcp_field),
        None => (None, None, None),
    };

    db.query(&format!("UPSERT {} SET display_name = $display_name, relative_skills_dir = $skills_dir, relative_detect_dir = $detect_dir, force_copy = $force_copy, mcp_config_path = $mcp_path, mcp_config_format = $mcp_format, mcp_field = $mcp_field, created_at = $created_at", record_id))
        .bind(("display_name", display_name.to_string()))
        .bind(("skills_dir", relative_skills_dir))
        .bind(("detect_dir", relative_detect_dir))
        .bind(("force_copy", force_copy))
        .bind(("mcp_path", mcp_path))
        .bind(("mcp_format", mcp_format))
        .bind(("mcp_field", mcp_field))
        .bind(("created_at", created_at))
        .await
        .map_err(|e| format!("Failed to save custom tool: {}", e))?;

    Ok(())
}

/// Save only MCP-related fields, preserving Skills fields if they exist
pub async fn save_custom_tool_mcp_fields(
    state: &DbState,
    key: &str,
    display_name: &str,
    relative_detect_dir: Option<String>,
    mcp_config_path: Option<String>,
    mcp_config_format: Option<String>,
    mcp_field: Option<String>,
    created_at: i64,
) -> Result<(), String> {
    // First check if the tool already exists
    let existing = get_custom_tool_by_key(state, key).await?;

    let db = state.0.lock().await;
    let record_id = db_record_id("custom_tool", key);

    // Preserve existing skills fields
    let (skills_dir, detect_dir) = match existing {
        Some(e) => (e.relative_skills_dir, e.relative_detect_dir.or(relative_detect_dir)),
        None => (None, relative_detect_dir),
    };

    db.query(&format!("UPSERT {} SET display_name = $display_name, relative_skills_dir = $skills_dir, relative_detect_dir = $detect_dir, mcp_config_path = $mcp_path, mcp_config_format = $mcp_format, mcp_field = $mcp_field, created_at = $created_at", record_id))
        .bind(("display_name", display_name.to_string()))
        .bind(("skills_dir", skills_dir))
        .bind(("detect_dir", detect_dir))
        .bind(("mcp_path", mcp_config_path))
        .bind(("mcp_format", mcp_config_format))
        .bind(("mcp_field", mcp_field))
        .bind(("created_at", created_at))
        .await
        .map_err(|e| format!("Failed to save custom tool: {}", e))?;

    Ok(())
}

/// Delete a custom tool
pub async fn delete_custom_tool(state: &DbState, key: &str) -> Result<(), String> {
    let db = state.0.lock().await;
    let record_id = db_record_id("custom_tool", key);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete custom tool: {}", e))?;

    Ok(())
}

/// Check if a custom tool key conflicts with built-in tools
pub fn is_builtin_tool_key(key: &str) -> bool {
    super::builtin::builtin_tool_by_key(key).is_some()
}
