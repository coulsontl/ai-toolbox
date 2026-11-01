use chrono::Local;
use rusqlite::{params_from_iter, Connection, OptionalExtension, ToSql};
use serde_json::{Map, Number, Value};

use super::schema::{
    sql_string_literal, DbTable, JsonFieldPath, OrderSpec, TableScope, ValidatedTableName,
};

/// `cli_key = ?1 AND kind = ?2` predicates for the scope's columns.
fn scope_predicates(scope: &TableScope) -> Vec<String> {
    scope
        .columns()
        .enumerate()
        .map(|(index, column)| format!("{column} = ?{}", index + 1))
        .collect()
}

fn where_clause(predicates: &[String]) -> String {
    if predicates.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", predicates.join(" AND "))
    }
}

/// Scope values in binding order.
///
/// These stay `&str` rather than `&dyn ToSql`: `ToSql` is implemented for `str`,
/// not for `&str`, so there is no `&'static str as &dyn ToSql` coercion to hand
/// to `query_row`. Callers bind them through [`params_from_iter`].
fn scope_values(scope: &TableScope) -> Vec<&'static str> {
    scope.values().collect()
}

fn scope_len(scope: &TableScope) -> usize {
    scope.bindings().count()
}

/// Scope params followed by the caller's own params, in binding order.
fn with_scope<'a>(
    scope: &'a TableScope,
    params: Vec<Box<dyn ToSql + 'a>>,
) -> Vec<Box<dyn ToSql + 'a>> {
    let mut all: Vec<Box<dyn ToSql + 'a>> = scope
        .values()
        .map(|value| Box::new(value) as Box<dyn ToSql>)
        .collect();
    all.extend(params);
    all
}

pub fn db_get(conn: &Connection, table: DbTable, id: &str) -> Result<Option<Value>, String> {
    let table_name = table.name();
    let scope = table.scope();
    let mut predicates = scope_predicates(&scope);
    predicates.push(format!("id = ?{}", predicates.len() + 1));
    let sql = format!(
        "SELECT id, json(data) AS data_json, created_at, updated_at FROM {table_name}{} LIMIT 1",
        where_clause(&predicates)
    );

    let mut params = scope_values(&scope);
    params.push(id);

    let row = conn
        .query_row(&sql, params_from_iter(params), |row| {
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

pub fn db_list(
    conn: &Connection,
    table: DbTable,
    order: Option<&OrderSpec>,
) -> Result<Vec<Value>, String> {
    let table_name = table.name();
    let scope = table.scope();
    let sql = format!(
        "SELECT id, json(data) AS data_json, created_at, updated_at FROM {table_name}{}{}",
        where_clause(&scope_predicates(&scope)),
        order.map(OrderSpec::to_sql).unwrap_or_default()
    );
    query_rows(conn, &sql, params_from_iter(scope_values(&scope)))
}

pub fn db_put(conn: &Connection, table: DbTable, id: &str, data: &Value) -> Result<(), String> {
    write_scoped_row(conn, table, id, data)
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
    let scope = table.scope();
    let mut predicates = scope_predicates(&scope);
    predicates.push(format!("id = ?{}", predicates.len() + 1));
    let sql = format!("DELETE FROM {table_name}{}", where_clause(&predicates));

    let mut params = scope_values(&scope);
    params.push(id);
    let affected = conn
        .execute(&sql, params_from_iter(params))
        .map_err(|error| format!("Failed to delete from {table_name}: {error}"))?;
    Ok(affected > 0)
}

/// Delete every row in this table's scope.
///
/// For a consolidated table this is `DELETE ... WHERE cli_key = ?`, never a
/// truncate: the scope is part of the table identity, so "delete all" of
/// `CliProvider(Codex)` must leave Kimi's providers alone.
pub fn db_delete_all(conn: &Connection, table: DbTable) -> Result<usize, String> {
    let table_name = table.name();
    let scope = table.scope();
    let sql = format!(
        "DELETE FROM {table_name}{}",
        where_clause(&scope_predicates(&scope))
    );
    conn.execute(&sql, params_from_iter(scope_values(&scope)))
        .map_err(|error| format!("Failed to delete all records from {table_name}: {error}"))
}

pub fn db_count(conn: &Connection, table: DbTable) -> Result<i64, String> {
    let table_name = table.name();
    let scope = table.scope();
    let sql = format!(
        "SELECT COUNT(*) FROM {table_name}{}",
        where_clause(&scope_predicates(&scope))
    );
    conn.query_row(&sql, params_from_iter(scope_values(&scope)), |row| row.get(0))
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
    let scope = table.scope();
    let field_expr = format!(
        "json_extract(data, {})",
        sql_string_literal(&field_path.to_sql_path())
    );
    // Scope params occupy ?1..?n, so the value predicate starts after them.
    let value_slot = scope_len(&scope) + 1;
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    let value_predicate = match expected {
        Value::Null => format!("{field_expr} IS NULL"),
        Value::Bool(value) => {
            params.push(Box::new(if *value { 1_i64 } else { 0_i64 }));
            format!("CAST({field_expr} AS INTEGER) = ?{value_slot}")
        }
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                params.push(Box::new(value));
                format!("CAST({field_expr} AS INTEGER) = ?{value_slot}")
            } else if let Some(value) = number.as_u64().and_then(|value| i64::try_from(value).ok())
            {
                params.push(Box::new(value));
                format!("CAST({field_expr} AS INTEGER) = ?{value_slot}")
            } else if let Some(value) = number.as_f64() {
                params.push(Box::new(value));
                format!("CAST({field_expr} AS REAL) = ?{value_slot}")
            } else {
                return Err("Unsupported JSON number value".to_string());
            }
        }
        Value::String(value) => {
            params.push(Box::new(value.clone()));
            format!("{field_expr} = ?{value_slot}")
        }
        Value::Array(_) | Value::Object(_) => {
            let expected_json = serde_json::to_string(expected).map_err(|error| {
                format!("Failed to serialize JSON field comparison value: {error}")
            })?;
            params.push(Box::new(expected_json));
            format!("json({field_expr}) = json(?{value_slot})")
        }
    };

    let mut predicates = scope_predicates(&scope);
    predicates.push(value_predicate);

    let mut sql = format!(
        "SELECT id, json(data) AS data_json, created_at, updated_at FROM {table_name}{}",
        where_clause(&predicates)
    );
    append_order_and_limit(&mut sql, order, limit);

    let bound = with_scope(&scope, params);
    query_rows(conn, &sql, params_from_iter(bound))
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
    let scope = table.scope();
    let path = sql_string_literal(&field_path.to_sql_path());
    let sql = format!(
        "SELECT MAX(CAST(json_extract(data, {path}) AS INTEGER)) FROM {table_name}{}",
        where_clause(&scope_predicates(&scope))
    );
    conn.query_row(&sql, params_from_iter(scope_values(&scope)), |row| row.get(0))
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
    let table_name = table.as_str();
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

pub fn db_put_into_validated_table(
    conn: &Connection,
    table: &ValidatedTableName,
    id: &str,
    data: &Value,
) -> Result<(), String> {
    write_row(conn, table.as_str(), &TableScope::NONE, id, data)
}

fn write_scoped_row(
    conn: &Connection,
    table: DbTable,
    id: &str,
    data: &Value,
) -> Result<(), String> {
    write_row(conn, table.name(), &table.scope(), id, data)
}

/// Insert or replace one row, writing the scope columns alongside the payload.
///
/// The scope columns are part of the conflict update on purpose: if the same id
/// is ever written under a different scope, the row has to follow the writer.
/// Leaving them alone would make the write silently vanish from the writer's
/// own scoped reads — a bug with no error and no trace.
fn write_row(
    conn: &Connection,
    table_name: &str,
    scope: &TableScope,
    id: &str,
    data: &Value,
) -> Result<(), String> {
    let now = now_string();
    let mut predicates = scope_predicates(scope);
    predicates.push(format!("id = ?{}", predicates.len() + 1));
    let mut lookup_params = scope_values(scope);
    lookup_params.push(id);
    let existing_created_at = conn
        .query_row(
            &format!(
                "SELECT created_at FROM {table_name}{} LIMIT 1",
                where_clause(&predicates)
            ),
            params_from_iter(lookup_params),
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

    let scope_columns: Vec<&str> = scope.columns().collect();
    let mut column_list: Vec<&str> = scope_columns.clone();
    column_list.extend(["id", "data", "created_at", "updated_at"]);

    let mut placeholders: Vec<String> = (1..=scope_columns.len())
        .map(|index| format!("?{index}"))
        .collect();
    placeholders.extend([
        format!("?{}", scope_columns.len() + 1),
        format!("jsonb(?{})", scope_columns.len() + 2),
        format!("?{}", scope_columns.len() + 3),
        format!("?{}", scope_columns.len() + 4),
    ]);

    let mut assignments: Vec<String> = scope_columns
        .iter()
        .map(|column| format!("{column} = excluded.{column}"))
        .collect();
    assignments.extend([
        "data = excluded.data".to_string(),
        "created_at = excluded.created_at".to_string(),
        "updated_at = excluded.updated_at".to_string(),
    ]);

    // The conflict target has to name the *actual* primary key: the consolidated
    // tables key on `(cli_key, id)` / `(cli_key, kind, id)` / `(kind, id)`, so
    // `ON CONFLICT(id)` would be rejected outright ("does not match any PRIMARY
    // KEY or UNIQUE constraint") — an error with no hint that the fix is the
    // column list.
    let mut conflict_columns: Vec<&str> = scope_columns.clone();
    conflict_columns.push("id");
    let sql = format!(
        "INSERT INTO {table_name} ({}) VALUES ({})
         ON CONFLICT({}) DO UPDATE SET {}",
        column_list.join(", "),
        placeholders.join(", "),
        conflict_columns.join(", "),
        assignments.join(", ")
    );

    let mut params: Vec<Box<dyn ToSql>> = scope_values(scope)
        .into_iter()
        .map(|value| Box::new(value) as Box<dyn ToSql>)
        .collect();

    params.push(Box::new(id));
    params.push(Box::new(data_json));
    params.push(Box::new(created_at));
    params.push(Box::new(updated_at));

    conn.execute(&sql, params_from_iter(params))
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

fn query_rows<P: rusqlite::Params>(conn: &Connection, sql: &str, params: P) -> Result<Vec<Value>, String> {
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
