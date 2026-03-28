use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::coding::{db_clean_id, db_record_id};
use crate::db_migration::{
    load_table_records, mark_migration_applied, migration_record_id, MigrationOutcome,
};

const MCP_SERVER_TABLE: &str = "mcp_server";
const MCP_PREFERENCES_TABLE: &str = "mcp_preferences";
pub const MIGRATION_ID: &str = "mcp_github_copilot_key_v1";

fn normalize_tool_key(key: &str) -> &str {
    match key {
        "github_copilot_intellij" => "github_copilot",
        _ => key,
    }
}

fn normalize_tool_list(value: &Value) -> Option<Vec<String>> {
    let arr = value.as_array()?;
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for item in arr {
        let Some(key) = item.as_str() else {
            continue;
        };
        let normalized_key = normalize_tool_key(key).to_string();
        if seen.insert(normalized_key.clone()) {
            normalized.push(normalized_key);
        }
    }

    Some(normalized)
}

fn merge_sync_detail_values(existing: &mut Value, incoming: &Value) {
    let Some(existing_obj) = existing.as_object_mut() else {
        return;
    };
    let Some(incoming_obj) = incoming.as_object() else {
        return;
    };

    for (key, value) in incoming_obj {
        let should_insert = existing_obj
            .get(key)
            .map(|existing_value| existing_value.is_null())
            .unwrap_or(true);
        if should_insert {
            existing_obj.insert(key.clone(), value.clone());
        }
    }
}

fn is_canonical_tool_key(tool_key: &str, normalized_key: &str) -> bool {
    tool_key == normalized_key
}

fn normalize_sync_details_value(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut normalized = Map::new();

    for (tool_key, detail) in obj {
        let normalized_key = normalize_tool_key(tool_key).to_string();
        if let Some(existing) = normalized.get_mut(&normalized_key) {
            if is_canonical_tool_key(tool_key, &normalized_key) {
                let mut canonical_detail = detail.clone();
                merge_sync_detail_values(&mut canonical_detail, existing);
                *existing = canonical_detail;
            } else {
                merge_sync_detail_values(existing, detail);
            }
        } else {
            normalized.insert(normalized_key, detail.clone());
        }
    }

    Some(Value::Object(normalized))
}

fn normalize_mcp_server_payload(record: &Value) -> Result<Option<Value>, String> {
    let record_id = record
        .get("id")
        .and_then(|value| value.as_str())
        .map(db_clean_id)
        .ok_or_else(|| "Failed to extract MCP server record id during migration".to_string())?;

    let mut payload = record.clone();
    let payload_obj = payload
        .as_object_mut()
        .ok_or_else(|| format!("Expected object payload for {}:{}", MCP_SERVER_TABLE, record_id))?;
    payload_obj.remove("id");

    let mut changed = false;

    if let Some(enabled_tools_value) = payload_obj.get("enabled_tools") {
        if let Some(normalized) = normalize_tool_list(enabled_tools_value) {
            let new_value = serde_json::json!(normalized);
            if new_value != *enabled_tools_value {
                payload_obj.insert("enabled_tools".to_string(), new_value);
                changed = true;
            }
        }
    }

    if let Some(sync_details_value) = payload_obj.get("sync_details") {
        if let Some(normalized) = normalize_sync_details_value(sync_details_value) {
            if normalized != *sync_details_value {
                payload_obj.insert("sync_details".to_string(), normalized);
                changed = true;
            }
        }
    }

    Ok(changed.then_some(payload))
}

fn normalize_mcp_preferences_payload(record: &Value) -> Result<Option<Value>, String> {
    let record_id = record
        .get("id")
        .and_then(|value| value.as_str())
        .map(db_clean_id)
        .ok_or_else(|| "Failed to extract MCP preferences record id during migration".to_string())?;

    let mut payload = record.clone();
    let payload_obj = payload.as_object_mut().ok_or_else(|| {
        format!(
            "Expected object payload for {}:{}",
            MCP_PREFERENCES_TABLE, record_id
        )
    })?;
    payload_obj.remove("id");

    let mut changed = false;

    if let Some(preferred_tools_value) = payload_obj.get("preferred_tools") {
        if let Some(normalized) = normalize_tool_list(preferred_tools_value) {
            let new_value = serde_json::json!(normalized);
            if new_value != *preferred_tools_value {
                payload_obj.insert("preferred_tools".to_string(), new_value);
                changed = true;
            }
        }
    }

    Ok(changed.then_some(payload))
}

fn build_update_statement(table_name: &str, record_id: &str, payload: &Value) -> Result<String, String> {
    let payload_json = serde_json::to_string(payload).map_err(|error| {
        format!(
            "Failed to serialize migration payload for {}:{}: {}",
            table_name, record_id, error
        )
    })?;

    Ok(format!(
        "UPDATE {} CONTENT {}",
        db_record_id(table_name, record_id),
        payload_json
    ))
}

pub fn run_migration(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<MigrationOutcome, String>> + Send + '_>> {
    Box::pin(async move {
        let mcp_server_records = load_table_records(db, MCP_SERVER_TABLE).await?;
        let mcp_preferences_records = load_table_records(db, MCP_PREFERENCES_TABLE).await?;

        let mut statements = Vec::new();

        for record in &mcp_server_records {
            let record_id = record
                .get("id")
                .and_then(|value| value.as_str())
                .map(db_clean_id)
                .ok_or_else(|| {
                    "Failed to extract MCP server record id when preparing migration".to_string()
                })?;

            if let Some(payload) = normalize_mcp_server_payload(record)? {
                statements.push(build_update_statement(MCP_SERVER_TABLE, &record_id, &payload)?);
            }
        }

        for record in &mcp_preferences_records {
            let record_id = record
                .get("id")
                .and_then(|value| value.as_str())
                .map(db_clean_id)
                .ok_or_else(|| {
                    "Failed to extract MCP preferences record id when preparing migration"
                        .to_string()
                })?;

            if let Some(payload) = normalize_mcp_preferences_payload(record)? {
                statements.push(build_update_statement(
                    MCP_PREFERENCES_TABLE,
                    &record_id,
                    &payload,
                )?);
            }
        }

        if statements.is_empty() {
            mark_migration_applied(db, MIGRATION_ID, "skipped_noop").await?;
            return Ok(MigrationOutcome::SkippedNoOp);
        }

        let mut transaction = String::from("BEGIN TRANSACTION;\n");
        for statement in &statements {
            transaction.push_str(statement);
            transaction.push_str(";\n");
        }
        transaction.push_str(&format!(
            "UPSERT {} CONTENT {{ migration_id: '{}', status: 'applied', applied_at: time::now() }};\n",
            migration_record_id(MIGRATION_ID),
            MIGRATION_ID
        ));
        transaction.push_str("COMMIT TRANSACTION;");

        db.query(transaction).await.map_err(|error| {
            format!(
                "Failed to apply database migration '{}': {}",
                MIGRATION_ID, error
            )
        })?;

        Ok(MigrationOutcome::Applied)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_normalize_tool_list_dedupes_legacy_github_copilot_key() {
        let value = json!(["github_copilot_intellij", "github_copilot", "codex"]);
        let normalized = normalize_tool_list(&value).unwrap();
        assert_eq!(normalized, vec!["github_copilot", "codex"]);
    }

    #[test]
    fn test_normalize_sync_details_value_merges_legacy_key_into_canonical() {
        let value = json!({
            "github_copilot_intellij": {
                "status": "ok",
                "synced_at": 1,
                "error_message": null
            },
            "github_copilot": {
                "status": "pending"
            }
        });

        let normalized = normalize_sync_details_value(&value).unwrap();
        assert!(normalized.get("github_copilot_intellij").is_none());
        assert_eq!(normalized["github_copilot"]["status"], "pending");
        assert_eq!(normalized["github_copilot"]["synced_at"], 1);
        assert!(normalized["github_copilot"]["error_message"].is_null());
    }
}



