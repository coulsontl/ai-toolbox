use std::sync::{Arc, Mutex};

use ai_toolbox_lib::db::backup;
use ai_toolbox_lib::db::change_hook::{install_change_recorder, DbChangeAction};
use ai_toolbox_lib::db::health;
use ai_toolbox_lib::db::helpers::{
    db_count, db_create, db_delete, db_delete_all, db_get, db_list, db_max_i64, db_patch_fields,
    db_patch_where_bool, db_put, db_query_by_bool, db_query_by_field, db_transaction,
    db_update_applied_status,
};
use ai_toolbox_lib::db::migrations::{self, TARGET_SCHEMA_VERSION};
use ai_toolbox_lib::db::schema::{
    validate_identifier, DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec, ALL_TABLES,
};
use ai_toolbox_lib::db::sqlite_state::{initialize_connection, SqliteDbState};
use rusqlite::Connection;
use serde_json::{json, Value};

fn test_conn() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open in-memory sqlite");
    initialize_connection(&mut conn).expect("initialize sqlite");
    conn
}

#[test]
fn jsonb_probe_and_schema_migration_create_all_tables() {
    let conn = test_conn();

    let support = health::verify_jsonb_support(&conn).expect("jsonb support");
    assert_eq!(support.jsonb_type, "blob");
    assert_eq!(support.jsonb_valid, 1);

    let user_version = migrations::get_user_version(&conn).expect("user_version");
    assert_eq!(user_version, TARGET_SCHEMA_VERSION);

    for table in ALL_TABLES {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table.name()],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(exists, 1, "missing table {}", table.name());
    }

    health::quick_check(&conn).expect("quick_check");
}

#[test]
fn schema_migration_rejects_future_user_version() {
    let mut conn = Connection::open_in_memory().expect("open in-memory sqlite");
    migrations::set_user_version(&conn, TARGET_SCHEMA_VERSION + 1).expect("set user version");

    let error = migrations::run_all(&mut conn).expect_err("future schema must fail");
    assert!(error.contains("newer than supported"));
}

#[test]
fn put_get_stores_jsonb_blob_and_injects_metadata() {
    let conn = test_conn();
    let payload = json!({
        "language": "zh-CN",
        "theme": "dark",
        "unknown_field": {"kept": true}
    });

    db_put(&conn, DbTable::Settings, "app", &payload).expect("put settings");

    let storage_type: String = conn
        .query_row(
            "SELECT typeof(data) FROM settings WHERE id = 'app'",
            [],
            |row| row.get(0),
        )
        .expect("read storage type");
    assert_eq!(storage_type, "blob");

    let record = db_get(&conn, DbTable::Settings, "app")
        .expect("get settings")
        .expect("settings record");
    assert_eq!(record["id"], "app");
    assert_eq!(record["language"], "zh-CN");
    assert_eq!(record["unknown_field"]["kept"], true);
    assert!(record["created_at"].as_str().is_some());
    assert!(record["updated_at"].as_str().is_some());
}

#[test]
fn put_keeps_column_and_json_timestamps_aligned_on_update() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::Settings,
        "app",
        &json!({
            "language": "zh-CN",
            "created_at": "2026-01-01T00:00:00+00:00",
            "updated_at": "2026-01-01T00:00:00+00:00"
        }),
    )
    .expect("initial put");

    db_put(
        &conn,
        DbTable::Settings,
        "app",
        &json!({"language": "en-US"}),
    )
    .expect("update put");

    let (column_created_at, json_created_at, column_updated_at, json_updated_at): (
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT created_at, json_extract(data, '$.created_at'),
                    updated_at, json_extract(data, '$.updated_at')
             FROM settings WHERE id = 'app'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read timestamps");

    assert_eq!(column_created_at, "2026-01-01T00:00:00+00:00");
    assert_eq!(json_created_at, column_created_at);
    assert_eq!(json_updated_at, column_updated_at);
}

#[test]
fn put_derives_columns_from_numeric_json_timestamps_without_changing_json_type() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::ImageAsset,
        "asset-1",
        &json!({
            "role": "output",
            "mime_type": "image/png",
            "file_name": "asset.png",
            "relative_path": "images/asset.png",
            "bytes": 10,
            "created_at": 1_777_777_777_i64
        }),
    )
    .expect("put image asset");

    let (column_created_at, json_created_type): (String, String) = conn
        .query_row(
            "SELECT created_at, json_type(data, '$.created_at')
             FROM image_asset WHERE id = 'asset-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read numeric timestamp");
    let record = db_get(&conn, DbTable::ImageAsset, "asset-1")
        .expect("get image asset")
        .expect("image asset record");

    assert_eq!(column_created_at, "1777777777");
    assert_eq!(json_created_type, "integer");
    assert_eq!(record["created_at"].as_i64(), Some(1_777_777_777));
}

#[test]
fn create_generates_clean_id_and_count_delete_work() {
    let conn = test_conn();
    let created = db_create(
        &conn,
        DbTable::McpServer,
        &json!({"name": "local-server", "sort_index": 0}),
    )
    .expect("create mcp server");

    let id = created["id"].as_str().expect("id");
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|char| char.is_ascii_hexdigit()));
    assert_eq!(db_count(&conn, DbTable::McpServer).expect("count"), 1);

    assert!(db_delete(&conn, DbTable::McpServer, id).expect("delete"));
    assert_eq!(db_count(&conn, DbTable::McpServer).expect("count"), 0);
}

#[test]
fn list_orders_by_json_integer_then_column() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::ClaudeProvider,
        "a",
        &json!({"name": "A", "sort_index": 2}),
    )
    .expect("put a");
    db_put(
        &conn,
        DbTable::ClaudeProvider,
        "b",
        &json!({"name": "B", "sort_index": 1}),
    )
    .expect("put b");
    db_put(
        &conn,
        DbTable::ClaudeProvider,
        "c",
        &json!({"name": "C", "sort_index": 1}),
    )
    .expect("put c");

    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc).expect("sort_index order"),
        OrderField::id(OrderDirection::Desc),
    ]);
    let records = db_list(&conn, DbTable::ClaudeProvider, Some(&order)).expect("list");
    let ids: Vec<_> = records
        .iter()
        .map(|record| record["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["c", "b", "a"]);
}

#[test]
fn query_by_bool_string_number_and_max_work() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::CodexOfficialAccount,
        "one",
        &json!({"provider_id": "p1", "is_applied": true, "sort_index": 3}),
    )
    .expect("put one");
    db_put(
        &conn,
        DbTable::CodexOfficialAccount,
        "two",
        &json!({"provider_id": "p2", "is_applied": false, "sort_index": 9}),
    )
    .expect("put two");

    let applied = db_query_by_bool(
        &conn,
        DbTable::CodexOfficialAccount,
        &JsonFieldPath::new("is_applied").expect("path"),
        true,
        None,
        None,
    )
    .expect("query applied");
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0]["id"], "one");

    let provider_records = db_query_by_field(
        &conn,
        DbTable::CodexOfficialAccount,
        &JsonFieldPath::new("provider_id").expect("path"),
        &Value::String("p2".to_string()),
        None,
        Some(1),
    )
    .expect("query provider");
    assert_eq!(provider_records.len(), 1);
    assert_eq!(provider_records[0]["id"], "two");

    let max_sort_index = db_max_i64(
        &conn,
        DbTable::CodexOfficialAccount,
        &JsonFieldPath::new("sort_index").expect("path"),
    )
    .expect("max sort index");
    assert_eq!(max_sort_index, Some(9));
}

#[test]
fn query_by_field_supports_array_and_object_values() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::CodexProvider,
        "one",
        &json!({
            "name": "one",
            "tags": ["official", "fast"],
            "metadata": {"tier": "pro", "enabled": true}
        }),
    )
    .expect("put one");
    db_put(
        &conn,
        DbTable::CodexProvider,
        "two",
        &json!({
            "name": "two",
            "tags": ["third-party"],
            "metadata": {"tier": "free", "enabled": false}
        }),
    )
    .expect("put two");

    let tagged = db_query_by_field(
        &conn,
        DbTable::CodexProvider,
        &JsonFieldPath::new("tags").expect("tags path"),
        &json!(["official", "fast"]),
        None,
        None,
    )
    .expect("query array");
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0]["id"], "one");

    let metadata = db_query_by_field(
        &conn,
        DbTable::CodexProvider,
        &JsonFieldPath::new("metadata").expect("metadata path"),
        &json!({"tier": "free", "enabled": false}),
        None,
        None,
    )
    .expect("query object");
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[0]["id"], "two");
}

#[test]
fn patch_preserves_unknown_fields_and_supports_nested_paths() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::GeminiCliProvider,
        "provider",
        &json!({
            "name": "Gemini",
            "settings_config": {"env": {"OLD": "kept"}},
            "unknown": {"nested": true}
        }),
    )
    .expect("put provider");

    let patched = db_patch_fields(
        &conn,
        DbTable::GeminiCliProvider,
        "provider",
        &[
            ("settings_config.env.GEMINI_API_KEY", json!("secret")),
            ("is_applied", json!(true)),
        ],
    )
    .expect("patch")
    .expect("patched record");

    assert_eq!(patched["settings_config"]["env"]["OLD"], "kept");
    assert_eq!(
        patched["settings_config"]["env"]["GEMINI_API_KEY"],
        "secret"
    );
    assert_eq!(patched["unknown"]["nested"], true);
    assert_eq!(patched["is_applied"], true);
    assert!(patched["updated_at"].as_str().is_some());
}

#[test]
fn patch_where_bool_updates_all_matching_records() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::ClaudePromptConfig,
        "old",
        &json!({"name": "old", "is_applied": true}),
    )
    .expect("put old");
    db_put(
        &conn,
        DbTable::ClaudePromptConfig,
        "new",
        &json!({"name": "new", "is_applied": true}),
    )
    .expect("put new");

    let changed = db_patch_where_bool(
        &conn,
        DbTable::ClaudePromptConfig,
        &JsonFieldPath::new("is_applied").expect("path"),
        true,
        &[("is_applied", json!(false))],
    )
    .expect("patch where");
    assert_eq!(changed, 2);

    let applied = db_query_by_bool(
        &conn,
        DbTable::ClaudePromptConfig,
        &JsonFieldPath::new("is_applied").expect("path"),
        true,
        None,
        None,
    )
    .expect("query applied");
    assert!(applied.is_empty());
}

#[test]
fn update_applied_status_switches_records_inside_one_transaction() {
    let mut conn = test_conn();
    db_put(
        &conn,
        DbTable::CodexProvider,
        "old",
        &json!({"name": "old", "is_applied": true}),
    )
    .expect("put old");
    db_put(
        &conn,
        DbTable::CodexProvider,
        "new",
        &json!({"name": "new", "is_applied": false}),
    )
    .expect("put new");

    db_update_applied_status(
        &mut conn,
        DbTable::CodexProvider,
        Some("new"),
        "2026-01-02T00:00:00+00:00",
    )
    .expect("switch applied");

    let applied = db_query_by_bool(
        &conn,
        DbTable::CodexProvider,
        &JsonFieldPath::new("is_applied").expect("path"),
        true,
        None,
        None,
    )
    .expect("query applied");

    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0]["id"], "new");
    assert_eq!(applied[0]["updated_at"], "2026-01-02T00:00:00+00:00");
}

#[test]
fn transaction_rolls_back_on_error() {
    let mut conn = test_conn();

    let result: Result<(), String> = db_transaction(&mut conn, |tx| {
        db_put(
            tx,
            DbTable::Skill,
            "skill",
            &json!({"name": "rollback", "sort_index": 1}),
        )?;
        Err("forced failure".to_string())
    });

    assert!(result.is_err());
    assert_eq!(db_count(&conn, DbTable::Skill).expect("count"), 0);
}

#[test]
fn invalid_identifier_and_json_path_are_rejected() {
    assert!(validate_identifier("valid_name_1").is_ok());
    assert!(validate_identifier("1invalid").is_err());
    assert!(validate_identifier("bad-name").is_err());
    assert!(JsonFieldPath::new("settings_config.env.API_KEY").is_ok());
    assert!(JsonFieldPath::new("settings-config.env").is_err());
    assert!(OrderField::json_integer("sort_index;DROP", OrderDirection::Asc).is_err());
}

#[test]
fn delete_all_clears_only_target_table() {
    let conn = test_conn();
    db_put(&conn, DbTable::SkillGroup, "group", &json!({"name": "G"})).expect("put group");
    db_put(&conn, DbTable::Skill, "skill", &json!({"name": "S"})).expect("put skill");

    let deleted = db_delete_all(&conn, DbTable::SkillGroup).expect("delete all groups");
    assert_eq!(deleted, 1);
    assert_eq!(
        db_count(&conn, DbTable::SkillGroup).expect("count groups"),
        0
    );
    assert_eq!(db_count(&conn, DbTable::Skill).expect("count skills"), 1);
}

#[test]
fn sqlite_state_limits_lock_scope_with_closure_api() {
    let state = SqliteDbState::in_memory_for_test().expect("sqlite state");

    state
        .with_conn(|conn| {
            db_put(
                conn,
                DbTable::Settings,
                "app",
                &json!({"language": "zh-CN"}),
            )
        })
        .expect("write through state");

    let language = state
        .with_conn(|conn| {
            let record = db_get(conn, DbTable::Settings, "app")?
                .ok_or_else(|| "missing settings".to_string())?;
            Ok(record["language"].as_str().unwrap().to_string())
        })
        .expect("read through state");

    assert_eq!(language, "zh-CN");
    assert_eq!(state.db_path().to_string_lossy(), ":memory:");
}

#[test]
fn backup_api_creates_readable_sqlite_file() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::Settings,
        "app",
        &json!({"language": "zh-CN"}),
    )
    .expect("put settings");

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let backup_path = temp_dir.path().join("ai-toolbox.db");
    backup::backup_to_path(&conn, &backup_path).expect("backup");

    let backup_conn = Connection::open(&backup_path).expect("open backup");
    let language: String = backup_conn
        .query_row(
            "SELECT json_extract(data, '$.language') FROM settings WHERE id = 'app'",
            [],
            |row| row.get(0),
        )
        .expect("read backup");
    assert_eq!(language, "zh-CN");
}

#[test]
fn update_hook_records_changed_table_names() {
    let conn = test_conn();
    let changes = Arc::new(Mutex::new(Vec::new()));
    install_change_recorder(&conn, changes.clone()).expect("install hook");

    db_put(
        &conn,
        DbTable::Settings,
        "app",
        &json!({"language": "zh-CN"}),
    )
    .expect("put settings");
    db_delete(&conn, DbTable::Settings, "app").expect("delete settings");

    let changes = changes.lock().expect("changes lock");
    assert!(changes
        .iter()
        .any(|change| change.table == "settings" && change.action == DbChangeAction::Insert));
    assert!(changes
        .iter()
        .any(|change| change.table == "settings" && change.action == DbChangeAction::Delete));
}
