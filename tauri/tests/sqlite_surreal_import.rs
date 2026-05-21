use ai_toolbox_lib::db::helpers::{db_count, db_get, db_list};
use ai_toolbox_lib::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use ai_toolbox_lib::db::surreal_import::{
    import_all_known_tables_from_surreal_with_warnings, import_missing_known_tables_from_surreal,
    import_tables_from_surreal, MigrationPaths,
};
use ai_toolbox_lib::db::SqliteDbState;
use serde_json::json;
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;

async fn temp_surreal_db() -> (tempfile::TempDir, Surreal<surrealdb::engine::local::Db>) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let db_path = temp_dir.path().join("surreal");
    let db = Surreal::new::<SurrealKv>(db_path)
        .await
        .expect("open surreal");
    db.use_ns("ai_toolbox")
        .use_db("main")
        .await
        .expect("use ns db");
    (temp_dir, db)
}

#[tokio::test]
async fn imports_selected_surreal_tables_into_sqlite_jsonb() {
    let (_temp_dir, surreal) = temp_surreal_db().await;
    surreal
        .query("UPSERT settings:`app` CONTENT $data")
        .bind((
            "data",
            json!({
                "language": "en-US",
                "theme": "dark",
                "backup_image_assets_enabled": false
            }),
        ))
        .await
        .expect("write settings");
    surreal
        .query("UPSERT claude_provider:`provider-a` CONTENT $data")
        .bind((
            "data",
            json!({
                "name": "Provider A",
                "is_applied": true,
                "sort_index": 2,
                "settings_config": {"env": {"ANTHROPIC_API_KEY": "sk-test"}}
            }),
        ))
        .await
        .expect("write provider a");
    surreal
        .query("UPSERT claude_provider:`provider-b` CONTENT $data")
        .bind((
            "data",
            json!({
                "name": "Provider B",
                "is_applied": false,
                "sort_index": 1
            }),
        ))
        .await
        .expect("write provider b");

    let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");
    let report = import_tables_from_surreal(
        &sqlite_state,
        &surreal,
        &[DbTable::Settings, DbTable::ClaudeProvider],
    )
    .await
    .expect("import tables");

    assert_eq!(report.total_records(), 3);
    assert_eq!(report.tables.len(), 2);

    let settings = sqlite_state
        .with_conn(|conn| db_get(conn, DbTable::Settings, "app"))
        .expect("read settings")
        .expect("settings record");
    assert_eq!(
        settings.get("id").and_then(|value| value.as_str()),
        Some("app")
    );
    assert_eq!(
        settings.get("language").and_then(|value| value.as_str()),
        Some("en-US")
    );
    assert_eq!(
        settings
            .get("backup_image_assets_enabled")
            .and_then(|value| value.as_bool()),
        Some(false)
    );

    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc).expect("sort field"),
        OrderField::id(OrderDirection::Asc),
    ]);
    let providers = sqlite_state
        .with_conn(|conn| db_list(conn, DbTable::ClaudeProvider, Some(&order)))
        .expect("list providers");
    assert_eq!(providers.len(), 2);
    assert_eq!(
        providers[0].get("id").and_then(|value| value.as_str()),
        Some("provider-b")
    );
    assert_eq!(
        providers[1]
            .pointer("/settings_config/env/ANTHROPIC_API_KEY")
            .and_then(|value| value.as_str()),
        Some("sk-test")
    );
}

#[tokio::test]
async fn repeated_import_replaces_records_in_target_table() {
    let (_temp_dir, surreal) = temp_surreal_db().await;
    surreal
        .query("UPSERT skill:`first` CONTENT { name: 'First' }")
        .await
        .expect("write first");

    let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");
    import_tables_from_surreal(&sqlite_state, &surreal, &[DbTable::Skill])
        .await
        .expect("first import");
    let first_count = sqlite_state
        .with_conn(|conn| db_count(conn, DbTable::Skill))
        .expect("first count");
    assert_eq!(first_count, 1);

    surreal
        .query("DELETE skill:`first`")
        .await
        .expect("delete first");
    surreal
        .query("UPSERT skill:`second` CONTENT { name: 'Second' }")
        .await
        .expect("write second");

    import_tables_from_surreal(&sqlite_state, &surreal, &[DbTable::Skill])
        .await
        .expect("second import");
    let records = sqlite_state
        .with_conn(|conn| db_list(conn, DbTable::Skill, None))
        .expect("list skills");

    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].get("id").and_then(|value| value.as_str()),
        Some("second")
    );
}

#[tokio::test]
async fn missing_known_table_import_preserves_non_empty_sqlite_tables() {
    let (_temp_dir, surreal) = temp_surreal_db().await;
    surreal
        .query("UPSERT settings:`app` CONTENT { language: 'zh-CN' }")
        .await
        .expect("write surreal settings");
    surreal
        .query("UPSERT codex_provider:`surreal-provider` CONTENT { name: 'Surreal Provider' }")
        .await
        .expect("write surreal codex provider");

    let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");
    sqlite_state
        .with_conn(|conn| {
            ai_toolbox_lib::db::helpers::db_put(
                conn,
                DbTable::Settings,
                "app",
                &json!({ "language": "en-US" }),
            )
        })
        .expect("seed sqlite settings");

    let report = import_missing_known_tables_from_surreal(&sqlite_state, &surreal)
        .await
        .expect("import missing tables");
    assert!(report
        .tables
        .iter()
        .any(|table| { table.table == DbTable::CodexProvider.name() && table.surreal_count == 1 }));
    assert!(!report
        .tables
        .iter()
        .any(|table| table.table == DbTable::Settings.name()));

    let settings = sqlite_state
        .with_conn(|conn| db_get(conn, DbTable::Settings, "app"))
        .expect("read sqlite settings")
        .expect("settings record");
    assert_eq!(
        settings.get("language").and_then(|value| value.as_str()),
        Some("en-US")
    );

    let provider = sqlite_state
        .with_conn(|conn| db_get(conn, DbTable::CodexProvider, "surreal-provider"))
        .expect("read imported provider")
        .expect("provider record");
    assert_eq!(
        provider.get("name").and_then(|value| value.as_str()),
        Some("Surreal Provider")
    );
}

#[tokio::test]
async fn all_known_table_import_writes_warnings_for_empty_legacy_tables() {
    let (_surreal_temp_dir, surreal) = temp_surreal_db().await;
    surreal
        .query("UPSERT settings:`app` CONTENT { language: 'zh-CN' }")
        .await
        .expect("write surreal settings");
    let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");
    let app_data_dir = tempfile::tempdir().expect("app data dir");
    let paths = MigrationPaths::new(app_data_dir.path());

    let report =
        import_all_known_tables_from_surreal_with_warnings(&sqlite_state, &surreal, &paths)
            .await
            .expect("import all known tables");

    assert!(report
        .tables
        .iter()
        .any(|table| table.table == DbTable::Settings.name() && table.surreal_count == 1));
    let warnings = std::fs::read_to_string(&paths.migration_warnings).expect("warnings");
    assert!(warnings.contains("Legacy SurrealDB table"));
}
