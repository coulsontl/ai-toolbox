use chrono::Local;
use rusqlite::{Connection, OptionalExtension, ToSql};
use serde_json::{Map, Number, Value};

use super::schema::{sql_string_literal, DbTable, JsonFieldPath, OrderSpec, ValidatedTableName};

pub fn db_get(conn: &Connection, table: DbTable, id: &str) -> Result<Option<Value>, String> {
    db_get_from_table(conn, table.name(), id)
}

pub fn db_list(
    conn: &Connection,
    table: DbTable,
    order: Option<&OrderSpec>,
) -> Result<Vec<Value>, String> {
    let table_name = table.name();
    let sql = format!(
        "SELECT id, json(data) AS data_json, created_at, updated_at FROM {table_name}{}",
        order.map(OrderSpec::to_sql).unwrap_or_default()
    );
    query_rows(conn, &sql, &[])
}

pub fn db_put(conn: &Connection, table: DbTable, id: &str, data: &Value) -> Result<(), String> {
    db_put_into_table(conn, table.name(), id, data)
}

pub fn db_create(conn: &Connection, table: DbTable, data: &Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    db_put(conn, table, &id, data)?;
    db_get(conn, table, &id)?.ok_or_else(|| {
        format!(
            "Failed to read newly created record '{}' from {}",
            id,
            table.name()
        )
    })
}

pub fn db_delete(conn: &Connection, table: DbTable, id: &str) -> Result<bool, String> {
    let table_name = table.name();
    let affected = conn
        .execute(&format!("DELETE FROM {table_name} WHERE id = ?1"), [id])
        .map_err(|error| format!("Failed to delete from {table_name}: {error}"))?;
    Ok(affected > 0)
}

pub fn db_delete_all(conn: &Connection, table: DbTable) -> Result<usize, String> {
    let table_name = table.name();
    conn.execute(&format!("DELETE FROM {table_name}"), [])
        .map_err(|error| format!("Failed to delete all records from {table_name}: {error}"))
}

pub fn db_count(conn: &Connection, table: DbTable) -> Result<i64, String> {
    let table_name = table.name();
    conn.query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
        row.get(0)
    })
    .map_err(|error| format!("Failed to count records in {table_name}: {error}"))
}

pub fn db_query_by_field(
    conn: &Connection,
    table: DbTable,
    field_path: &JsonFieldPath,
    expected: &Value,
    order: Option<&OrderSpec>,
    limit: Option<usize>,
) -> Result<Vec<Value>, String> {
    let table_name = table.name();
    let field_expr = format!(
        "json_extract(data, {})",
        sql_string_literal(&field_path.to_sql_path())
    );
    let mut sql = format!(
        "SELECT id, json(data) AS data_json, created_at, updated_at FROM {table_name} WHERE "
    );

    match expected {
        Value::Null => {
            sql.push_str(&format!("{field_expr} IS NULL"));
            append_order_and_limit(&mut sql, order, limit);
            query_rows(conn, &sql, &[])
        }
        Value::Bool(value) => {
            let expected_integer = if *value { 1_i64 } else { 0_i64 };
            sql.push_str(&format!("CAST({field_expr} AS INTEGER) = ?1"));
            append_order_and_limit(&mut sql, order, limit);
            query_rows(conn, &sql, &[&expected_integer])
        }
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                sql.push_str(&format!("CAST({field_expr} AS INTEGER) = ?1"));
                append_order_and_limit(&mut sql, order, limit);
                query_rows(conn, &sql, &[&value])
            } else if let Some(value) = number.as_u64().and_then(|value| i64::try_from(value).ok())
            {
                sql.push_str(&format!("CAST({field_expr} AS INTEGER) = ?1"));
                append_order_and_limit(&mut sql, order, limit);
                query_rows(conn, &sql, &[&value])
            } else if let Some(value) = number.as_f64() {
                sql.push_str(&format!("CAST({field_expr} AS REAL) = ?1"));
                append_order_and_limit(&mut sql, order, limit);
                query_rows(conn, &sql, &[&value])
            } else {
                Err("Unsupported JSON number value".to_string())
            }
        }
        Value::String(value) => {
            sql.push_str(&format!("{field_expr} = ?1"));
            append_order_and_limit(&mut sql, order, limit);
            query_rows(conn, &sql, &[value])
        }
        Value::Array(_) | Value::Object(_) => {
            let expected_json = serde_json::to_string(expected).map_err(|error| {
                format!("Failed to serialize JSON field comparison value: {error}")
            })?;
            sql.push_str(&format!("json({field_expr}) = json(?1)"));
            append_order_and_limit(&mut sql, order, limit);
            query_rows(conn, &sql, &[&expected_json])
        }
    }
}

pub fn db_query_by_bool(
    conn: &Connection,
    table: DbTable,
    field_path: &JsonFieldPath,
    expected: bool,
    order: Option<&OrderSpec>,
    limit: Option<usize>,
) -> Result<Vec<Value>, String> {
    db_query_by_field(
        conn,
        table,
        field_path,
        &Value::Bool(expected),
        order,
        limit,
    )
}

pub fn db_max_i64(
    conn: &Connection,
    table: DbTable,
    field_path: &JsonFieldPath,
) -> Result<Option<i64>, String> {
    let table_name = table.name();
    let path = sql_string_literal(&field_path.to_sql_path());
    let sql = format!("SELECT MAX(CAST(json_extract(data, {path}) AS INTEGER)) FROM {table_name}");
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(|error| format!("Failed to read max value from {table_name}: {error}"))
}

pub fn db_patch_fields(
    conn: &Connection,
    table: DbTable,
    id: &str,
    patch: &[(&str, Value)],
) -> Result<Option<Value>, String> {
    let Some(mut record) = db_get(conn, table, id)? else {
        return Ok(None);
    };

    for (path, value) in patch {
        let field_path = JsonFieldPath::new(path)?;
        set_json_path(&mut record, &field_path, value.clone())?;
    }

    if !patch.iter().any(|(path, _)| *path == "updated_at") {
        set_json_path(
            &mut record,
            &JsonFieldPath::new("updated_at")?,
            Value::String(now_string()),
        )?;
    }

    remove_helper_metadata(&mut record);
    db_put(conn, table, id, &record)?;
    db_get(conn, table, id)
}

pub fn db_patch_where_bool(
    conn: &Connection,
    table: DbTable,
    predicate_path: &JsonFieldPath,
    predicate_value: bool,
    patch: &[(&str, Value)],
) -> Result<usize, String> {
    let records = db_query_by_bool(conn, table, predicate_path, predicate_value, None, None)?;
    let mut changed = 0;

    for record in records {
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Record in {} is missing id", table.name()))?;
        if db_patch_fields(conn, table, id, patch)?.is_some() {
            changed += 1;
        }
    }

    Ok(changed)
}

pub fn db_update_applied_status(
    conn: &mut Connection,
    table: DbTable,
    target_id: Option<&str>,
    updated_at: &str,
) -> Result<(), String> {
    db_transaction(conn, |tx| {
        db_patch_where_bool(
            tx,
            table,
            &JsonFieldPath::new("is_applied")?,
            true,
            &[
                ("is_applied", Value::Bool(false)),
                ("updated_at", Value::String(updated_at.to_string())),
            ],
        )?;

        if let Some(target_id) = target_id {
            db_patch_fields(
                tx,
                table,
                target_id,
                &[
                    ("is_applied", Value::Bool(true)),
                    ("updated_at", Value::String(updated_at.to_string())),
                ],
            )?
            .ok_or_else(|| format!("Record '{}' not found in {}", target_id, table.name()))?;
        }

        Ok(())
    })
}

pub fn db_transaction<T>(
    conn: &mut Connection,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let tx = conn
        .transaction()
        .map_err(|error| format!("Failed to begin SQLite transaction: {error}"))?;

    match operation(&tx) {
        Ok(value) => {
            tx.commit()
                .map_err(|error| format!("Failed to commit SQLite transaction: {error}"))?;
            Ok(value)
        }
        Err(error) => {
            let _ = tx.rollback();
            Err(error)
        }
    }
}

pub fn db_get_from_validated_table(
    conn: &Connection,
    table: &ValidatedTableName,
    id: &str,
) -> Result<Option<Value>, String> {
    db_get_from_table(conn, table.as_str(), id)
}

pub fn db_put_into_validated_table(
    conn: &Connection,
    table: &ValidatedTableName,
    id: &str,
    data: &Value,
) -> Result<(), String> {
    db_put_into_table(conn, table.as_str(), id, data)
}

fn db_get_from_table(
    conn: &Connection,
    table_name: &str,
    id: &str,
) -> Result<Option<Value>, String> {
    let sql = format!(
        "SELECT id, json(data) AS data_json, created_at, updated_at FROM {table_name} WHERE id = ?1 LIMIT 1"
    );

    let row = conn
        .query_row(&sql, [id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .optional()
        .map_err(|error| format!("Failed to read record from {table_name}: {error}"))?;

    row.map(row_tuple_to_value).transpose()
}

fn db_put_into_table(
    conn: &Connection,
    table_name: &str,
    id: &str,
    data: &Value,
) -> Result<(), String> {
    let now = now_string();
    let existing_created_at = conn
        .query_row(
            &format!("SELECT created_at FROM {table_name} WHERE id = ?1 LIMIT 1"),
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read existing timestamp from {table_name}: {error}"))?;
    let mut stored_data = data.clone();
    let stored_object = stored_data
        .as_object_mut()
        .ok_or_else(|| format!("SQLite JSONB payload for {table_name} must be an object"))?;
    let has_json_created_at = stored_object.contains_key("created_at");
    let has_json_updated_at = stored_object.contains_key("updated_at");
    let created_at = stored_object
        .get("created_at")
        .and_then(json_timestamp_to_column_string)
        .or(existing_created_at)
        .unwrap_or_else(|| now.clone());
    let updated_at = stored_object
        .get("updated_at")
        .and_then(json_timestamp_to_column_string)
        .unwrap_or_else(|| now.clone());
    if !has_json_created_at {
        stored_object.insert("created_at".to_string(), Value::String(created_at.clone()));
    }
    if !has_json_updated_at {
        stored_object.insert("updated_at".to_string(), Value::String(updated_at.clone()));
    }
    let data_json = serde_json::to_string(&stored_data)
        .map_err(|error| format!("Failed to serialize JSON payload for {table_name}: {error}"))?;

    conn.execute(
        &format!(
            "INSERT INTO {table_name} (id, data, created_at, updated_at)
             VALUES (?1, jsonb(?2), ?3, ?4)
	             ON CONFLICT(id) DO UPDATE SET
	               data = excluded.data,
	               created_at = excluded.created_at,
	               updated_at = excluded.updated_at"
        ),
        (id, data_json, created_at, updated_at),
    )
    .map_err(|error| format!("Failed to write record into {table_name}: {error}"))?;

    Ok(())
}

fn json_timestamp_to_column_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn query_rows(conn: &Connection, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<Value>, String> {
    let mut statement = conn
        .prepare(sql)
        .map_err(|error| format!("Failed to prepare SQLite query: {error}; sql={sql}"))?;
    let rows = statement
        .query_map(params, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("Failed to execute SQLite query: {error}; sql={sql}"))?;

    let mut records = Vec::new();
    for row in rows {
        records.push(row_tuple_to_value(
            row.map_err(|error| format!("Failed to read SQLite row: {error}"))?,
        )?);
    }

    Ok(records)
}

fn row_tuple_to_value(
    (id, data_json, created_at, updated_at): (String, String, String, String),
) -> Result<Value, String> {
    let mut value: Value = serde_json::from_str(&data_json)
        .map_err(|error| format!("Failed to parse SQLite JSON payload: {error}"))?;

    let object = value
        .as_object_mut()
        .ok_or_else(|| "SQLite JSONB record payload must be a JSON object".to_string())?;

    object.insert("id".to_string(), Value::String(id));
    object
        .entry("created_at".to_string())
        .or_insert(Value::String(created_at));
    object
        .entry("updated_at".to_string())
        .or_insert(Value::String(updated_at));

    Ok(value)
}

fn append_order_and_limit(sql: &mut String, order: Option<&OrderSpec>, limit: Option<usize>) {
    if let Some(order) = order {
        sql.push_str(&order.to_sql());
    }
    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
}

fn set_json_path(root: &mut Value, path: &JsonFieldPath, value: Value) -> Result<(), String> {
    let mut current = root;
    let last_index = path.segments().len() - 1;

    for (index, segment) in path.segments().iter().enumerate() {
        if index == last_index {
            let object = current.as_object_mut().ok_or_else(|| {
                format!(
                    "Cannot set JSON path '{}': parent is not an object",
                    path.to_sql_path()
                )
            })?;
            object.insert(segment.clone(), value);
            return Ok(());
        }

        let object = current.as_object_mut().ok_or_else(|| {
            format!(
                "Cannot set JSON path '{}': parent is not an object",
                path.to_sql_path()
            )
        })?;
        current = object
            .entry(segment.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }

    Ok(())
}

fn remove_helper_metadata(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        object.remove("id");
    }
}

fn now_string() -> String {
    Local::now().to_rfc3339()
}

#[allow(dead_code)]
fn value_to_number(value: i64) -> Value {
    Value::Number(Number::from(value))
}
