use std::{
    fs,
    sync::{Arc, Mutex},
};

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
    validate_identifier, Cli, CliConfigKind, DbTable, JsonFieldPath, OrderDirection, OrderField,
    OrderSpec, RelayKind, ALL_TABLES,
};
use ai_toolbox_lib::db::sqlite_state::{
    initialize_connection, SqliteDbState, SQLITE_MIGRATION_BACKUP_DIR,
};
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

    // The per-CLI indexes from v8/v9/v16/v23/v25/v32 no longer exist as such:
    // v33 folds those tables into the scope-prefixed indexes below. Asserting
    // the old names would pin the schema to a shape it no longer has.
    for index_name in [
        "idx_cli_provider_applied",
        "idx_cli_provider_sort",
        "idx_cli_provider_disabled",
        "idx_cli_official_account_applied",
        "idx_cli_official_account_sort",
        "idx_cli_official_account_provider",
        "idx_cli_config_applied",
        "idx_cli_config_sort",
        "idx_cli_config_key_kind",
        "idx_relay_entity_kind",
        "idx_relay_quota_point_kind_at",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index_name],
                |row| row.get(0),
            )
            .expect("query sqlite index");
        assert_eq!(exists, 1, "missing index {index_name}");
    }

    // Scope-first, not just present: an index on `is_applied` alone would let
    // one CLI's rows dominate the planner's choice for another CLI's query.
    for (index_name, leading_column) in [
        ("idx_cli_provider_applied", "cli_key"),
        ("idx_cli_official_account_provider", "cli_key"),
        ("idx_cli_config_applied", "cli_key"),
        ("idx_relay_entity_channel", "kind"),
    ] {
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index_name],
                |row| row.get(0),
            )
            .unwrap_or_else(|error| panic!("missing index {index_name}: {error}"));
        let first_column = sql
            .split_once('(')
            .and_then(|(_, rest)| rest.split(',').next())
            .map(|column| column.trim().to_string())
            .unwrap_or_default();
        assert_eq!(
            first_column, leading_column,
            "{index_name} must lead with {leading_column}; got {sql}"
        );
    }

    let proxy_request_log_columns = conn
        .prepare("PRAGMA table_info(proxy_request_logs)")
        .expect("prepare proxy_request_logs table info")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query proxy_request_logs columns")
        .map(|row| row.expect("read proxy_request_logs column"))
        .collect::<Vec<_>>();
    let has_pricing_model_source = proxy_request_log_columns
        .iter()
        .any(|column| column == "pricing_model_source");
    assert!(
        has_pricing_model_source,
        "missing proxy_request_logs.pricing_model_source"
    );
    for column in ["route_name", "method", "path"] {
        assert!(
            proxy_request_log_columns
                .iter()
                .any(|name| name.as_str() == column),
            "missing proxy_request_logs.{column}"
        );
    }

    health::quick_check(&conn).expect("quick_check");
}

#[test]
fn v17_migration_preserves_existing_request_logs_and_nullable_effort() {
    let mut conn = test_conn();
    conn.execute_batch(
        "ALTER TABLE proxy_request_logs DROP COLUMN reasoning_effort;
         INSERT INTO proxy_request_logs
             (request_id, provider_id, app_type, model, input_tokens, created_at)
         VALUES ('legacy-log', 'provider-a', 'codex', 'gpt-5', 120, 1780000000);",
    )
    .expect("seed v16 request log schema");
    migrations::set_user_version(&conn, 16).expect("set v16 schema version");

    migrations::run_all(&mut conn).expect("upgrade v16");
    migrations::run_all(&mut conn).expect("reopen upgraded schema");
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );
    let legacy: (String, i64, Option<String>) = conn.query_row(
        "SELECT model, input_tokens, reasoning_effort FROM proxy_request_logs WHERE request_id = 'legacy-log'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).expect("read legacy log after upgrade");
    assert_eq!(legacy, ("gpt-5".to_string(), 120, None));
    conn.execute(
        "UPDATE proxy_request_logs SET reasoning_effort = 'high' WHERE request_id = 'legacy-log'",
        [],
    )
    .expect("write new effort metadata");
    // Existing-column migrations must also tolerate a restored version marker.
    migrations::set_user_version(&conn, 16).unwrap();
    migrations::run_all(&mut conn).expect("repeat additive migration");
    let effort: String = conn
        .query_row(
            "SELECT reasoning_effort FROM proxy_request_logs WHERE request_id = 'legacy-log'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(effort, "high");
}

#[test]
fn v27_migration_preserves_existing_request_logs_and_nullable_service_tier() {
    let mut conn = test_conn();
    conn.execute_batch(
        "ALTER TABLE proxy_request_logs DROP COLUMN service_tier;
         INSERT INTO proxy_request_logs
             (request_id, provider_id, app_type, model, input_tokens, created_at)
         VALUES ('legacy-tier-log', 'provider-a', 'codex', 'gpt-5.6-sol', 120, 1780000000);",
    )
    .expect("seed v26 request log schema");
    migrations::set_user_version(&conn, 26).expect("set v26 schema version");

    migrations::run_all(&mut conn).expect("upgrade v26");
    migrations::run_all(&mut conn).expect("reopen upgraded schema");
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );
    let legacy: (String, Option<String>) = conn
        .query_row(
            "SELECT model, service_tier FROM proxy_request_logs WHERE request_id = 'legacy-tier-log'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read legacy log after upgrade");
    assert_eq!(legacy, ("gpt-5.6-sol".to_string(), None));
    conn.execute(
        "UPDATE proxy_request_logs SET service_tier = 'fast' WHERE request_id = 'legacy-tier-log'",
        [],
    )
    .expect("write new service tier metadata");
    // Existing-column migrations must also tolerate a restored version marker.
    migrations::set_user_version(&conn, 26).unwrap();
    migrations::run_all(&mut conn).expect("repeat additive migration");
    let service_tier: String = conn
        .query_row(
            "SELECT service_tier FROM proxy_request_logs WHERE request_id = 'legacy-tier-log'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(service_tier, "fast");
}

/// v28 adds the compaction label the same way v27 added the service tier: an
/// additive, nullable column that leaves existing rows untouched and tolerates
/// a repeated upgrade.
#[test]
fn v28_migration_preserves_existing_request_logs_and_nullable_compaction_mode() {
    let mut conn = test_conn();
    conn.execute_batch(
        "ALTER TABLE proxy_request_logs DROP COLUMN compaction_mode;
         INSERT INTO proxy_request_logs
             (request_id, provider_id, app_type, model, input_tokens, created_at)
         VALUES ('legacy-compaction-log', 'provider-a', 'codex', 'gpt-5.6-sol', 120, 1780000000);",
    )
    .expect("seed v27 request log schema");
    migrations::set_user_version(&conn, 27).expect("set v27 schema version");

    migrations::run_all(&mut conn).expect("upgrade v27");
    migrations::run_all(&mut conn).expect("reopen upgraded schema");
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );
    let legacy: Option<String> = conn
        .query_row(
            "SELECT compaction_mode FROM proxy_request_logs WHERE request_id = 'legacy-compaction-log'",
            [],
            |row| row.get(0),
        )
        .expect("read legacy log after upgrade");
    assert_eq!(legacy, None, "an old row must not claim a compaction mode");
    conn.execute(
        "UPDATE proxy_request_logs SET compaction_mode = 'native' WHERE request_id = 'legacy-compaction-log'",
        [],
    )
    .expect("write new compaction metadata");
    migrations::set_user_version(&conn, 27).unwrap();
    migrations::run_all(&mut conn).expect("repeat additive migration");
    let compaction_mode: String = conn
        .query_row(
            "SELECT compaction_mode FROM proxy_request_logs WHERE request_id = 'legacy-compaction-log'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(compaction_mode, "native");
}

/// v29 rebuilds `usage_daily_rollups`: the project column joins the primary key,
/// which SQLite cannot alter in place. Existing archived usage must survive the
/// table swap and keep its measurement columns, and a repeated upgrade must be
/// a no-op rather than losing the new column.
#[test]
fn v29_migration_rebuilds_rollups_with_the_project_column() {
    let mut conn = test_conn();
    // SQLite cannot drop a column that participates in a primary key, so undo
    // v29 by rebuilding both tables the way v29 itself does.
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_request_logs_project;
         DROP INDEX IF EXISTS idx_usage_rollups_project;
         ALTER TABLE proxy_request_logs DROP COLUMN project_path;

         CREATE TABLE usage_daily_rollups_v28 (
             date TEXT NOT NULL,
             app_type TEXT NOT NULL,
             provider_id TEXT NOT NULL,
             model TEXT NOT NULL,
             request_count INTEGER NOT NULL DEFAULT 0,
             success_count INTEGER NOT NULL DEFAULT 0,
             input_tokens INTEGER NOT NULL DEFAULT 0,
             output_tokens INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens INTEGER NOT NULL DEFAULT 0,
             cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
             total_cost_usd TEXT NOT NULL DEFAULT '0',
             avg_latency_ms INTEGER NOT NULL DEFAULT 0,
             latency_sample_count INTEGER,
             extra_tokens INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (date, app_type, provider_id, model)
         );
         DROP TABLE usage_daily_rollups;
         ALTER TABLE usage_daily_rollups_v28 RENAME TO usage_daily_rollups;

         INSERT INTO proxy_request_logs
             (request_id, provider_id, app_type, model, input_tokens, created_at)
         VALUES ('legacy-project-log', 'session', 'claude', 'claude-sonnet-4-6', 90, 1780000000);
         INSERT INTO usage_daily_rollups
             (date, app_type, provider_id, model, request_count, success_count,
              input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
              total_cost_usd, avg_latency_ms, latency_sample_count, extra_tokens)
         VALUES ('2026-09-01', 'claude', 'session', 'claude-sonnet-4-6', 3, 3,
                 90, 10, 5, 1, '0.42', 250, 3, 7);",
    )
    .expect("seed the pre-v29 schema");
    migrations::set_user_version(&conn, 28).expect("set v28 schema version");

    migrations::run_all(&mut conn).expect("upgrade v28");
    migrations::run_all(&mut conn).expect("reopen upgraded schema");
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );

    let archived = conn
        .query_row(
            "SELECT project_path, request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms,
                    latency_sample_count, extra_tokens
             FROM usage_daily_rollups WHERE provider_id = 'session'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .expect("read the migrated rollup");
    assert_eq!(
        archived,
        (
            "".to_string(),
            3,
            3,
            90,
            10,
            5,
            1,
            "0.42".to_string(),
            250,
            3,
            7
        ),
        "archived usage must keep every counter and land in the unknown-project bucket"
    );

    // The project is part of the primary key now: two projects on the same
    // day/model must be two rows instead of one collapsing onto the other.
    conn.execute_batch(
        "INSERT INTO usage_daily_rollups (date, app_type, provider_id, model, project_path, request_count)
         VALUES ('2026-09-01', 'claude', 'session', 'claude-sonnet-4-6', 'D:/repo-a', 2),
                ('2026-09-01', 'claude', 'session', 'claude-sonnet-4-6', 'D:/repo-b', 1);",
    )
    .expect("write per-project rollups");
    let per_project: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM usage_daily_rollups
             WHERE date = '2026-09-01' AND model = 'claude-sonnet-4-6'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(per_project, 3, "each project keeps its own rollup row");

    // Detail rows gain the column as NULL; the importer coalesces that to `''`.
    let legacy_project: Option<String> = conn
        .query_row(
            "SELECT project_path FROM proxy_request_logs WHERE request_id = 'legacy-project-log'",
            [],
            |row| row.get(0),
        )
        .expect("read legacy project");
    assert_eq!(legacy_project, None);

    migrations::set_user_version(&conn, 28).unwrap();
    migrations::run_all(&mut conn).expect("repeat the v29 rebuild");
    for project in ["D:/repo-a", "D:/repo-b", ""] {
        let preserved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM usage_daily_rollups
                 WHERE date = '2026-09-01' AND project_path = ?1",
                [project],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            preserved, 1,
            "a repeated upgrade must keep the {project:?} row intact"
        );
    }
}

#[test]
fn session_usage_migrations_preserve_rollups_and_ledger_on_repeat() {
    let mut conn = test_conn();
    conn.execute_batch(
        "DROP TABLE gateway_session_usage_state;
         ALTER TABLE usage_daily_rollups DROP COLUMN latency_sample_count;
         INSERT INTO usage_daily_rollups (date, app_type, provider_id, model, request_count, avg_latency_ms)
         VALUES ('2026-09-01', 'claude', 'provider-a', 'model-a', 4, 300),
                ('2026-09-01', 'claude', 'session', 'model-a', 2, 0);",
    ).unwrap();
    migrations::set_user_version(&conn, 17).unwrap();
    migrations::run_all(&mut conn).unwrap();
    let rows = conn
        .prepare(
            "SELECT provider_id, request_count, avg_latency_ms, latency_sample_count
         FROM usage_daily_rollups ORDER BY provider_id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("provider-a".into(), 4, 300, 4),
            ("session".into(), 2, 0, 0)
        ]
    );
    conn.execute(
        "INSERT INTO gateway_session_usage_state (id, data, created_at, updated_at)
         VALUES ('claude:session-a', jsonb(?1), '2026-09-08', '2026-09-08')",
        [r#"{"records":{"SESSION:message-a":{"fingerprint":"kept"}}}"#],
    )
    .unwrap();
    migrations::set_user_version(&conn, 17).unwrap();
    migrations::run_all(&mut conn).unwrap();
    let fingerprint: String = conn.query_row(
        "SELECT json_extract(data, '$.records.\"SESSION:message-a\".fingerprint') FROM gateway_session_usage_state",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(fingerprint, "kept");
}

#[test]
fn already_migrated_v18_without_latency_samples_is_repaired_with_backup() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = temp_dir.path().join("ai-toolbox.db");
    {
        let db = SqliteDbState::open(db_path.clone()).unwrap();
        db.with_conn(|conn| {
            conn.execute_batch(
                "ALTER TABLE usage_daily_rollups DROP COLUMN latency_sample_count;
                 INSERT INTO usage_daily_rollups (date, app_type, provider_id, model, request_count, avg_latency_ms, total_cost_usd)
                 VALUES ('2026-09-01', 'claude', 'provider-a', 'model-a', 4, 300, '0.012345'),
                        ('2026-09-01', 'claude', 'session', 'model-a', 2, 0, '0.006789');
                 INSERT INTO gateway_session_usage_state (id, data, created_at, updated_at)
                 VALUES ('existing-session', jsonb('{}'), '2026-09-01', '2026-09-01');",
            ).map_err(|error| error.to_string())?;
            migrations::set_user_version(conn, 18)
        }).unwrap();
    }
    // This database has already applied v18. Do not lower its version marker:
    // a new migration must cover the exact state seen by the running app.
    let db = SqliteDbState::open(db_path).unwrap();
    db.with_conn(|conn| {
        let proxy_samples: i64 = conn.query_row(
            "SELECT latency_sample_count FROM usage_daily_rollups WHERE provider_id = 'provider-a'",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let session_samples: i64 = conn.query_row(
            "SELECT latency_sample_count FROM usage_daily_rollups WHERE provider_id = 'session'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!((proxy_samples, session_samples), (4, 0));
        let state_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM gateway_session_usage_state",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state_count, 1);
        Ok(())
    })
    .unwrap();
    let backups = fs::read_dir(temp_dir.path().join(SQLITE_MIGRATION_BACKUP_DIR))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open(&backups[0]).unwrap();
    assert_eq!(migrations::get_user_version(&backup).unwrap(), 18);
    assert!(backup
        .prepare("SELECT latency_sample_count FROM usage_daily_rollups")
        .is_err());
    let old_costs: String = backup
        .query_row(
            "SELECT total_cost_usd FROM usage_daily_rollups WHERE provider_id = 'provider-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_costs, "0.012345");
}

#[test]
fn already_migrated_v18_with_latency_samples_preserves_existing_measurements() {
    let mut conn = test_conn();
    conn.execute_batch(
        "INSERT INTO usage_daily_rollups (date, app_type, provider_id, model, request_count, avg_latency_ms, latency_sample_count)
         VALUES ('2026-09-01', 'claude', 'provider-a', 'model-a', 4, 350, 2),
                ('2026-09-01', 'claude', 'session', 'model-a', 2, 0, 0);",
    ).unwrap();
    migrations::set_user_version(&conn, 18).unwrap();
    migrations::run_all(&mut conn).unwrap();
    let samples: (i64, i64) = conn.query_row(
        "SELECT avg_latency_ms, latency_sample_count FROM usage_daily_rollups WHERE provider_id = 'provider-a'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(samples, (350, 2));
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );
    migrations::run_all(&mut conn).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT latency_sample_count FROM usage_daily_rollups WHERE provider_id = 'session'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
}

#[test]
fn schema_migration_rejects_future_user_version() {
    let mut conn = Connection::open_in_memory().expect("open in-memory sqlite");
    migrations::set_user_version(&conn, TARGET_SCHEMA_VERSION + 1).expect("set user version");

    let error = migrations::run_all(&mut conn).expect_err("future schema must fail");
    assert!(migrations::is_future_schema_error(&error));
    assert!(error.contains("newer than supported"));

    let message = migrations::future_schema_user_message(&error);
    assert!(message.contains("不能回退到旧版本"));
    assert!(message.contains(&error));
}

#[test]
fn v21_websocket_upgrade_defaults_old_logs_to_http_without_changing_usage() {
    let mut conn = test_conn();
    conn.execute_batch(
        "DROP INDEX idx_request_logs_proxy_envelope;
         ALTER TABLE proxy_request_logs DROP COLUMN transport;
         ALTER TABLE proxy_request_logs DROP COLUMN request_kind;
         INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,input_tokens,created_at,usage_request_count)
         VALUES ('before-websocket','provider','codex','test-model',123,1780000000,2);
         INSERT INTO usage_daily_rollups (date,app_type,provider_id,model,request_count,input_tokens,total_cost_usd,latency_sample_count)
         VALUES ('2026-09-01','codex','provider','test-model',2,123,'0.012345',2);",
    ).unwrap();
    migrations::set_user_version(&conn, 20).unwrap();
    migrations::run_all(&mut conn).unwrap();
    migrations::run_all(&mut conn).unwrap();
    let old_row: (String, String, i64, i64) = conn.query_row(
        "SELECT transport,request_kind,input_tokens,usage_request_count FROM proxy_request_logs WHERE request_id='before-websocket'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert_eq!(old_row, ("http".to_string(), "request".to_string(), 123, 2));
    let archived: (i64, String, i64) = conn
        .query_row(
            "SELECT request_count,total_cost_usd,latency_sample_count FROM usage_daily_rollups",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(archived, (2, "0.012345".to_string(), 2));
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );
    health::quick_check(&conn).unwrap();
}

#[test]
fn v20_usage_metadata_upgrade_preserves_logs_rollups_and_import_ledger() {
    let mut conn = test_conn();
    conn.execute_batch(
        r#"ALTER TABLE proxy_request_logs DROP COLUMN usage_metadata;
         ALTER TABLE proxy_request_logs DROP COLUMN usage_request_count;
         ALTER TABLE proxy_request_logs DROP COLUMN extra_tokens;
         ALTER TABLE usage_daily_rollups DROP COLUMN extra_tokens;
         DROP INDEX idx_proxy_native_match;
         INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,input_tokens,created_at,data_source)
         VALUES ('old-session','session','claude','glm-5.2',0,1780000000,'session');
         INSERT INTO usage_daily_rollups (date,app_type,provider_id,model,request_count,input_tokens,total_cost_usd,latency_sample_count)
         VALUES ('2026-09-01','codex','provider','model',2,200,'0.012345',2);
         INSERT INTO gateway_session_usage_state (id,data,created_at,updated_at)
         VALUES ('claude:old',jsonb('{"records":{"old-session":{"fingerprint":"kept"}}}'),'before','before');"#,
    ).unwrap();
    migrations::set_user_version(&conn, 19).unwrap();
    migrations::run_all(&mut conn).unwrap();
    migrations::run_all(&mut conn).unwrap();
    let preserved: (i64, i64, Option<String>) = conn.query_row(
        "SELECT usage_request_count,extra_tokens,json(usage_metadata) FROM proxy_request_logs WHERE request_id='old-session'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(preserved, (1, 0, None));
    assert_eq!(
        conn.query_row(
            "SELECT total_cost_usd FROM usage_daily_rollups",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "0.012345"
    );
    assert_eq!(conn.query_row("SELECT json_extract(data,'$.records.old-session.fingerprint') FROM gateway_session_usage_state WHERE id='claude:old'", [], |row| row.get::<_, String>(0)).unwrap(), "kept");
    conn.execute("UPDATE proxy_request_logs SET usage_metadata=jsonb(?1),usage_request_count=3,extra_tokens=7", [r#"{"granularity":"turn","call_count":3,"incomplete":true}"#]).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT json_extract(usage_metadata,'$.call_count') FROM proxy_request_logs",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    health::quick_check(&conn).unwrap();
}

#[test]
fn sqlite_state_open_rejects_future_user_version_before_file_initialization() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = temp_dir.path().join("ai-toolbox.db");
    {
        let conn = Connection::open(&db_path).expect("open temp sqlite");
        migrations::set_user_version(&conn, TARGET_SCHEMA_VERSION + 1)
            .expect("set future user_version");
    }

    let error = match SqliteDbState::open(db_path.clone()) {
        Ok(_) => panic!("future schema must fail"),
        Err(error) => error,
    };
    assert!(migrations::is_future_schema_error(&error));

    let conn = Connection::open(&db_path).expect("reopen temp sqlite");
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("read journal mode");
    assert_ne!(journal_mode.to_ascii_lowercase(), "wal");

    let user_table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .expect("count user tables");
    assert_eq!(user_table_count, 0);
}

#[test]
fn sqlite_state_open_creates_pre_migration_backup_before_schema_upgrade() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = temp_dir.path().join("ai-toolbox.db");
    {
        let mut conn = Connection::open(&db_path).expect("open temp sqlite");
        initialize_connection(&mut conn).expect("initialize sqlite");
        db_put(
            &conn,
            DbTable::Settings,
            "app",
            &json!({"language": "zh-CN"}),
        )
        .expect("put settings");
        migrations::set_user_version(&conn, TARGET_SCHEMA_VERSION - 1)
            .expect("set old user_version");
    }

    let state = SqliteDbState::open(db_path.clone()).expect("open migrated sqlite");
    let migrated_version = state
        .with_conn(migrations::get_user_version)
        .expect("read migrated user_version");
    assert_eq!(migrated_version, TARGET_SCHEMA_VERSION);

    let backup_dir = temp_dir.path().join(SQLITE_MIGRATION_BACKUP_DIR);
    let backup_paths = fs::read_dir(&backup_dir)
        .expect("read migration backup dir")
        .map(|entry| entry.expect("backup entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("db"))
        .collect::<Vec<_>>();
    assert_eq!(backup_paths.len(), 1);

    let backup_conn = Connection::open(&backup_paths[0]).expect("open migration backup");
    let backup_version = migrations::get_user_version(&backup_conn).expect("backup user_version");
    assert_eq!(backup_version, TARGET_SCHEMA_VERSION - 1);

    let language: String = backup_conn
        .query_row(
            "SELECT json_extract(data, '$.language') FROM settings WHERE id = 'app'",
            [],
            |row| row.get(0),
        )
        .expect("read backup settings");
    assert_eq!(language, "zh-CN");
}

#[test]
fn future_backup_schema_message_is_user_facing_and_detectable() {
    let error = migrations::future_backup_schema_error(
        i64::from(TARGET_SCHEMA_VERSION + 1),
        i64::from(TARGET_SCHEMA_VERSION),
    );
    assert!(migrations::is_future_schema_error(&error));

    let message = migrations::future_backup_schema_user_message(&error);
    assert!(message.contains("这个备份由更高版本的 AI Toolbox 创建"));
    assert!(message.contains("当前版本无法恢复"));
    assert!(message.contains(&error));
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
        DbTable::CliProvider(Cli::Claude),
        "a",
        &json!({"name": "A", "sort_index": 2}),
    )
    .expect("put a");
    db_put(
        &conn,
        DbTable::CliProvider(Cli::Claude),
        "b",
        &json!({"name": "B", "sort_index": 1}),
    )
    .expect("put b");
    db_put(
        &conn,
        DbTable::CliProvider(Cli::Claude),
        "c",
        &json!({"name": "C", "sort_index": 1}),
    )
    .expect("put c");

    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc).expect("sort_index order"),
        OrderField::id(OrderDirection::Desc),
    ]);
    let records = db_list(&conn, DbTable::CliProvider(Cli::Claude), Some(&order)).expect("list");
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
        DbTable::CliOfficialAccount(Cli::Codex),
        "one",
        &json!({"provider_id": "p1", "is_applied": true, "sort_index": 3}),
    )
    .expect("put one");
    db_put(
        &conn,
        DbTable::CliOfficialAccount(Cli::Codex),
        "two",
        &json!({"provider_id": "p2", "is_applied": false, "sort_index": 9}),
    )
    .expect("put two");

    let applied = db_query_by_bool(
        &conn,
        DbTable::CliOfficialAccount(Cli::Codex),
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
        DbTable::CliOfficialAccount(Cli::Codex),
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
        DbTable::CliOfficialAccount(Cli::Codex),
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
        DbTable::CliProvider(Cli::Codex),
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
        DbTable::CliProvider(Cli::Codex),
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
        DbTable::CliProvider(Cli::Codex),
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
        DbTable::CliProvider(Cli::Codex),
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
        DbTable::CliProvider(Cli::Gemini),
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
        DbTable::CliProvider(Cli::Gemini),
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
        DbTable::CliConfig(Cli::Claude, CliConfigKind::Prompt),
        "old",
        &json!({"name": "old", "is_applied": true}),
    )
    .expect("put old");
    db_put(
        &conn,
        DbTable::CliConfig(Cli::Claude, CliConfigKind::Prompt),
        "new",
        &json!({"name": "new", "is_applied": true}),
    )
    .expect("put new");

    let changed = db_patch_where_bool(
        &conn,
        DbTable::CliConfig(Cli::Claude, CliConfigKind::Prompt),
        &JsonFieldPath::new("is_applied").expect("path"),
        true,
        &[("is_applied", json!(false))],
    )
    .expect("patch where");
    assert_eq!(changed, 2);

    let applied = db_query_by_bool(
        &conn,
        DbTable::CliConfig(Cli::Claude, CliConfigKind::Prompt),
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
        DbTable::CliProvider(Cli::Codex),
        "old",
        &json!({"name": "old", "is_applied": true}),
    )
    .expect("put old");
    db_put(
        &conn,
        DbTable::CliProvider(Cli::Codex),
        "new",
        &json!({"name": "new", "is_applied": false}),
    )
    .expect("put new");

    db_update_applied_status(
        &mut conn,
        DbTable::CliProvider(Cli::Codex),
        Some("new"),
        "2026-01-02T00:00:00+00:00",
    )
    .expect("switch applied");

    let applied = db_query_by_bool(
        &conn,
        DbTable::CliProvider(Cli::Codex),
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

#[test]
fn v29_adds_the_credits_columns_in_the_same_step_as_the_project_dimension() {
    let mut conn = test_conn();
    // Rebuild the pre-v29 shape: neither `project_path` nor `credits` exists.
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_request_logs_project;
         DROP INDEX IF EXISTS idx_usage_rollups_project;
         ALTER TABLE proxy_request_logs DROP COLUMN project_path;
         ALTER TABLE proxy_request_logs DROP COLUMN credits;

         CREATE TABLE usage_daily_rollups_v28 (
             date TEXT NOT NULL,
             app_type TEXT NOT NULL,
             provider_id TEXT NOT NULL,
             model TEXT NOT NULL,
             request_count INTEGER NOT NULL DEFAULT 0,
             success_count INTEGER NOT NULL DEFAULT 0,
             input_tokens INTEGER NOT NULL DEFAULT 0,
             output_tokens INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens INTEGER NOT NULL DEFAULT 0,
             cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
             total_cost_usd TEXT NOT NULL DEFAULT '0',
             avg_latency_ms INTEGER NOT NULL DEFAULT 0,
             latency_sample_count INTEGER,
             extra_tokens INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (date, app_type, provider_id, model)
         );
         DROP TABLE usage_daily_rollups;
         ALTER TABLE usage_daily_rollups_v28 RENAME TO usage_daily_rollups;

         INSERT INTO proxy_request_logs
             (request_id, provider_id, app_type, model, input_tokens, created_at)
         VALUES ('pre-v29-log', 'session', 'qodercli', 'qfmodel', 0, 1780000000);
         INSERT INTO usage_daily_rollups
             (date, app_type, provider_id, model, request_count, success_count,
              input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
              total_cost_usd, avg_latency_ms, latency_sample_count, extra_tokens)
         VALUES ('2026-10-17', 'qodercli', 'session', 'qfmodel', 7, 7,
                 0, 0, 0, 0, '0.3', 120, 7, 4);",
    )
    .expect("seed the pre-v29 schema");
    migrations::set_user_version(&conn, 28).expect("set v28 schema version");

    migrations::run_all(&mut conn).expect("upgrade v28");
    // A second open must be a no-op, not a duplicate-column error.
    migrations::run_all(&mut conn).expect("reopen upgraded schema");
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );

    // The rebuilt rollup carries both new columns, and the rebuild preserved
    // the counters it was seeded with.
    let (project, credits, requests, extra): (String, f64, i64, i64) = conn
        .query_row(
            "SELECT project_path, credits, request_count, extra_tokens
             FROM usage_daily_rollups WHERE provider_id = 'session'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read rebuilt rollup");
    assert_eq!(project, "", "old rows join the unknown-project bucket");
    assert_eq!(credits, 0.0);
    assert_eq!(requests, 7, "the upgrade must not rewrite counters");
    assert_eq!(extra, 4);

    // A detail row that predates the column reads as "no credits reported",
    // which stays distinguishable from a real zero.
    let log_credits: Option<f64> = conn
        .query_row(
            "SELECT credits FROM proxy_request_logs WHERE request_id = 'pre-v29-log'",
            [],
            |row| row.get(0),
        )
        .expect("read log credits");
    assert_eq!(log_credits, None);
}

#[test]
fn v30_migration_preserves_history_and_indexes_legacy_proxy_envelopes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("history.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        initialize_connection(&mut conn).unwrap();
        conn.execute_batch(
            "DROP INDEX idx_request_logs_proxy_envelope;
             INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, model, input_tokens, output_tokens, created_at, data_source, request_kind)
             VALUES ('SESSION:codex:provider:old-proxy', 'provider', 'codex', 'gpt-5', 101, 12, 1780000000, 'proxy', 'request'),
                    ('SESSION:codex:session:old-native', 'session', 'codex', 'gpt-5', 202, 24, 1780000000, 'session', 'request');"
        ).unwrap();
        migrations::set_user_version(&conn, 29).unwrap();
    }
    let db = SqliteDbState::open(path.clone()).unwrap();
    db.with_conn(|conn| {
        assert_eq!(migrations::get_user_version(conn).unwrap(), TARGET_SCHEMA_VERSION);
        let totals: (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), SUM(input_tokens), SUM(output_tokens) FROM proxy_request_logs", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(totals, (2, 303, 36));
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM proxy_request_logs INDEXED BY idx_request_logs_proxy_envelope WHERE COALESCE(data_source, 'proxy') = 'proxy' AND request_kind = 'request'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "legacy proxy rows are indexed; native rows are excluded");
        Ok(())
    }).unwrap();
    drop(db);
    let mut conn = Connection::open(path).unwrap();
    migrations::set_user_version(&conn, 29).unwrap();
    migrations::run_all(&mut conn).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        fs::read_dir(root.path().join(SQLITE_MIGRATION_BACKUP_DIR))
            .unwrap()
            .count(),
        1
    );
}

/// v31 adds the compact "tools actually invoked" column. Old rows must keep
/// their history and read as `NULL` — "never inspected" is not "nothing ran" —
/// and the upgrade has to survive being applied to a database that already has
/// the column.
#[test]
fn v31_migration_adds_the_native_tools_column_without_touching_history() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("history.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        initialize_connection(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, model, input_tokens, output_tokens, created_at, data_source, request_kind)
             VALUES ('SESSION:codex:provider:old-proxy', 'provider', 'codex', 'gpt-5', 101, 12, 1780000000, 'proxy', 'request'),
                    ('SESSION:codex:session:old-native', 'session', 'codex', 'gpt-5', 202, 24, 1780000000, 'session', 'request');",
        )
        .unwrap();
        migrations::set_user_version(&conn, 30).unwrap();
    }
    let db = SqliteDbState::open(path.clone()).unwrap();
    db.with_conn(|conn| {
        assert_eq!(migrations::get_user_version(conn).unwrap(), TARGET_SCHEMA_VERSION);
        let totals: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(input_tokens), SUM(output_tokens),
                        SUM(CASE WHEN native_tools_used IS NULL THEN 1 ELSE 0 END)
                 FROM proxy_request_logs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(totals, (2, 303, 36, 2), "history is preserved and unlabelled");
        Ok(())
    })
    .unwrap();
    drop(db);
    // Re-running the migration must not fail on the now-present column.
    let mut conn = Connection::open(path).unwrap();
    migrations::set_user_version(&conn, 30).unwrap();
    migrations::run_all(&mut conn).unwrap();
    conn.execute(
        "UPDATE proxy_request_logs SET native_tools_used = ?1 WHERE request_id = ?2",
        rusqlite::params![r#"["web_search"]"#, "SESSION:codex:provider:old-proxy"],
    )
    .unwrap();
    let stored: String = conn
        .query_row(
            "SELECT native_tools_used FROM proxy_request_logs WHERE request_id = ?1",
            ["SESSION:codex:provider:old-proxy"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, r#"["web_search"]"#);
}

/// A database that already ran v31 (the gateway `native_tools_used` column)
/// but predates the WorkBuddy/CodeBuddy tables must still get them.
///
/// Version 31 was already released on `main` before these tables existed, so
/// the tables live in v32. Do not lower the version marker to make this pass:
/// that hides the exact state a shipping build leaves behind.
#[test]
fn v32_migration_adds_workbuddy_tables_to_an_already_v31_database() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("already-v31.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        initialize_connection(&mut conn).unwrap();
        // Reproduce the real v31 shape: the gateway column exists, the three
        // WorkBuddy/CodeBuddy tables do not.
        conn.execute_batch(
            "DROP TABLE IF EXISTS workbuddy_settings_config;
             DROP TABLE IF EXISTS codebuddy_settings_config;
             DROP TABLE IF EXISTS codebuddy_prompt_config;",
        )
        .unwrap();
        migrations::set_user_version(&conn, 31).unwrap();
    }

    let db = SqliteDbState::open(path.clone()).unwrap();
    db.with_conn(|conn| {
        assert_eq!(
            migrations::get_user_version(conn).unwrap(),
            TARGET_SCHEMA_VERSION
        );
        // v32's own tables are transient — v33 folds them into `cli_config`
        // and drops them. What this test still has to prove is that v32's *work*
        // survives into the consolidated shape: WorkBuddy and CodeBuddy each
        // land under their own `cli_key` rather than being lost in the fold.
        for (cli, kind) in [
            (Cli::Workbuddy, CliConfigKind::Settings),
            (Cli::Codebuddy, CliConfigKind::Settings),
            (Cli::Codebuddy, CliConfigKind::Prompt),
        ] {
            let table = DbTable::CliConfig(cli, kind);
            assert_eq!(table.name(), "cli_config");
            let count = db_count(conn, table)
                .unwrap_or_else(|error| panic!("{cli:?}/{kind:?} after v32: {error}"));
            assert_eq!(count, 0);
        }
        // The WorkBuddy/CodeBuddy provider tables folded too, under scopes that
        // must not collide with any other CLI's.
        assert_eq!(db_count(conn, DbTable::CliProvider(Cli::Workbuddy)).unwrap(), 0);
        assert_eq!(db_count(conn, DbTable::CliProvider(Cli::Codebuddy)).unwrap(), 0);
        assert_ne!(
            DbTable::CliProvider(Cli::Workbuddy).scope(),
            DbTable::CliProvider(Cli::Codebuddy).scope()
        );
        // The v31 column must survive: v32 must not replace that migration.
        let has_native_tools_used = conn
            .prepare("SELECT native_tools_used FROM proxy_request_logs")
            .is_ok();
        assert!(has_native_tools_used);
        Ok(())
    })
    .unwrap();
    drop(db);

    // Reopening is a no-op, not a duplicate-table error.
    let mut conn = Connection::open(&path).unwrap();
    migrations::run_all(&mut conn).unwrap();
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );
}

/// A database at v32 must reach v33 with the consolidated tables built.
///
/// v33 is the consolidation step, so this test asserts the *shape it produces*
/// rather than the per-CLI tables it retires: the three shared tables exist with
/// their scope columns, the legacy tables are gone, and reopening is idempotent.
#[test]
fn v33_consolidates_the_per_cli_tables_on_an_already_v32_database() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("already-v32.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        initialize_connection(&mut conn).unwrap();
        // A real v32 database still has the per-CLI tables; build them the way
        // v32 left them so the migration has something to fold.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workbuddy_settings_config (
                 id TEXT PRIMARY KEY NOT NULL,
                 data BLOB NOT NULL CHECK (json_valid(data, 4)),
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );",
        )
        .unwrap();
        migrations::set_user_version(&conn, 32).unwrap();
    }

    let db = SqliteDbState::open(path.clone()).unwrap();
    db.with_conn(|conn| {
        assert_eq!(
            migrations::get_user_version(conn).unwrap(),
            TARGET_SCHEMA_VERSION
        );

        for table in ["cli_provider", "cli_official_account", "cli_config"] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table}"),
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| format!("{table} missing after v33: {error}"))?;
            assert_eq!(count, 0, "{table} should start empty on this fixture");
        }

        // Every folded table is gone; the shared ones remain.
        for retired in [
            "workbuddy_settings_config",
            "workbuddy_provider",
            "codebuddy_provider",
            "claude_provider",
            "grok_prompt_config",
            "oh_my_pi_agents_config",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [retired],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 0, "{retired} should have been dropped by v33");
        }
        Ok(())
    })
    .unwrap();
    drop(db);

    // Reopening is a no-op, not a duplicate-table error.
    let mut conn = Connection::open(&path).unwrap();
    migrations::run_all(&mut conn).unwrap();
    assert_eq!(
        migrations::get_user_version(&conn).unwrap(),
        TARGET_SCHEMA_VERSION
    );
}

/// The rows themselves have to survive the fold, not just the tables.
///
/// This is the failure mode that matters: a v33 that builds the new schema but
/// copies nothing would pass every "table exists" assertion while silently
/// deleting every provider and preset the user ever configured.
#[test]
fn v33_moves_every_legacy_row_into_the_shared_tables_with_its_scope() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy-rows.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        // Stop at v32 rather than `initialize_connection`, which would run v33
        // and drop the very tables this fixture needs to seed. The gateway
        // tables (model_pricing, proxy_request_logs, ...) still have to exist
        // because the upgrade path seeds them on the way through.
        initialize_connection(&mut conn).unwrap();
        for table in ["cli_provider", "cli_official_account", "cli_config"] {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))
                .unwrap();
        }
        for table in migrations::legacy_tables_for_test() {
            conn.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                     id TEXT PRIMARY KEY NOT NULL,
                     data BLOB NOT NULL CHECK (json_valid(data, 4)),
                     created_at TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                 );"
            ))
            .unwrap();
        }
        // Two CLIs' providers, two CLIs' common configs (both with the literal
        // id `common` — the collision the composite key exists for), and a
        // Grok prompt preset with `is_applied` set.
        conn.execute_batch(
            r#"INSERT INTO claude_provider (id, data, created_at, updated_at)
                 VALUES ('p-claude', jsonb('{"name":"Claude One","sort_index":1}'), 't', 't');
               INSERT INTO codex_provider (id, data, created_at, updated_at)
                 VALUES ('p-codex', jsonb('{"name":"Codex One","sort_index":2}'), 't', 't');
               INSERT INTO claude_common_config (id, data, created_at, updated_at)
                 VALUES ('common', jsonb('{"config":"{\"claude\":true}"}'), 't', 't');
               INSERT INTO codex_common_config (id, data, created_at, updated_at)
                 VALUES ('common', jsonb('{"config":"{\"codex\":true}"}'), 't', 't');
               INSERT INTO grok_prompt_config (id, data, created_at, updated_at)
                 VALUES ('pr-grok', jsonb('{"name":"Grok Prompt","is_applied":true}'), 't', 't');"#,
        )
        .unwrap();
        migrations::set_user_version(&conn, 32).unwrap();
    }

    let db = SqliteDbState::open(path).unwrap();
    db.with_conn(|conn| {
        let providers = db_list(conn, DbTable::CliProvider(Cli::Claude), None).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["name"], "Claude One");

        let codex = db_list(conn, DbTable::CliProvider(Cli::Codex), None).unwrap();
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0]["name"], "Codex One");

        // The literal `common` id survives in both scopes instead of one
        // overwriting the other.
        let claude_common =
            db_get(conn, DbTable::CliConfig(Cli::Claude, CliConfigKind::Common), "common")
                .unwrap()
                .expect("claude common config");
        assert_eq!(claude_common["config"], "{\"claude\":true}");
        let codex_common =
            db_get(conn, DbTable::CliConfig(Cli::Codex, CliConfigKind::Common), "common")
                .unwrap()
                .expect("codex common config");
        assert_eq!(codex_common["config"], "{\"codex\":true}");

        // `is_applied` moved with the row, so the applied preset is still the
        // one the CLI resolves on startup.
        let applied = db_query_by_bool(
            conn,
            DbTable::CliConfig(Cli::Grok, CliConfigKind::Prompt),
            &JsonFieldPath::new("is_applied").unwrap(),
            true,
            None,
            None,
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0]["name"], "Grok Prompt");
        Ok(())
    })
    .unwrap();
}

// ---- second-round regression cases for the scope consolidation --------------
//
// These exist because of one specific failure mode: with fifty-five per-CLI
// tables, "forgetting a table" was a compile error. With three shared tables,
// forgetting a scope filter is a *silent* cross-CLI read or write. Every test
// below fails if a scope is dropped anywhere in the helper chain.

/// `db_list` must never return another CLI's rows.
#[test]
fn scoped_list_never_returns_another_cli_rows() {
    let conn = test_conn();
    for (cli, name) in [(Cli::Codex, "codex-a"), (Cli::Kimi, "kimi-a")] {
        db_put(
            &conn,
            DbTable::CliProvider(cli),
            &format!("{name}-id"),
            &json!({ "id": format!("{name}-id"), "name": name }),
        )
        .unwrap();
    }

    let codex = db_list(&conn, DbTable::CliProvider(Cli::Codex), None).unwrap();
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0]["name"], "codex-a");

    let kimi = db_list(&conn, DbTable::CliProvider(Cli::Kimi), None).unwrap();
    assert_eq!(kimi.len(), 1);
    assert_eq!(kimi[0]["name"], "kimi-a");

    // The physical table really does hold both: the split is the scope's doing,
    // not an empty table.
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM cli_provider", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total, 2);
}

/// `db_get` addresses a row *within* a scope, so a correct id under the wrong
/// CLI must read as absent rather than returning a foreign record.
#[test]
fn scoped_get_is_not_addressable_from_another_cli() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::CliProvider(Cli::Codex),
        "shared-id",
        &json!({ "id": "shared-id", "name": "codex-only" }),
    )
    .unwrap();

    assert!(db_get(&conn, DbTable::CliProvider(Cli::Codex), "shared-id")
        .unwrap()
        .is_some());
    assert!(db_get(&conn, DbTable::CliProvider(Cli::Grok), "shared-id")
        .unwrap()
        .is_none());
}

/// `db_delete` and `db_count` must honour the scope too. A delete that ignored
/// it would silently remove another CLI's provider while reporting success.
#[test]
fn scoped_delete_and_count_stay_inside_the_scope() {
    let conn = test_conn();
    for cli in [Cli::Codex, Cli::Kimi] {
        db_put(
            &conn,
            DbTable::CliProvider(cli),
            "same-id",
            &json!({ "id": "same-id", "name": cli.as_str() }),
        )
        .unwrap();
    }
    assert_eq!(
        db_count(&conn, DbTable::CliProvider(Cli::Codex)).unwrap(),
        1
    );
    assert_eq!(db_count(&conn, DbTable::CliProvider(Cli::Kimi)).unwrap(), 1);

    // Same id in both scopes: only Kimi's row may disappear.
    assert!(db_delete(&conn, DbTable::CliProvider(Cli::Kimi), "same-id").unwrap());
    assert_eq!(db_count(&conn, DbTable::CliProvider(Cli::Kimi)).unwrap(), 0);
    assert_eq!(
        db_count(&conn, DbTable::CliProvider(Cli::Codex)).unwrap(),
        1
    );
}

/// The single most dangerous helper: `db_update_applied_status` clears
/// `is_applied` across the whole *scope* before setting the target. Without the
/// scope it would clear every other CLI's applied preset, and startup would
/// then resolve a different provider with no error anywhere.
#[test]
fn applied_status_only_clears_inside_its_own_cli() {
    let mut conn = test_conn();
    for (cli, id) in [
        (Cli::Codex, "codex-applied"),
        (Cli::Kimi, "kimi-applied"),
        (Cli::Grok, "grok-applied"),
    ] {
        db_put(
            &conn,
            DbTable::CliProvider(cli),
            id,
            &json!({ "id": id, "name": id, "is_applied": true }),
        )
        .unwrap();
    }

    db_update_applied_status(
        &mut conn,
        DbTable::CliProvider(Cli::Kimi),
        Some("kimi-applied"),
        "2026-11-01T00:00:00Z",
    )
    .unwrap();

    let applied = |table| -> Vec<String> {
        db_query_by_bool(
            &conn,
            table,
            &JsonFieldPath::new("is_applied").unwrap(),
            true,
            None,
            None,
        )
        .unwrap()
        .into_iter()
        .map(|record| record["name"].as_str().unwrap().to_string())
        .collect()
    };

    assert_eq!(applied(DbTable::CliProvider(Cli::Kimi)), ["kimi-applied"]);
    assert_eq!(applied(DbTable::CliProvider(Cli::Codex)), ["codex-applied"]);
    assert_eq!(applied(DbTable::CliProvider(Cli::Grok)), ["grok-applied"]);
}

/// `cli_config` keys on `(cli_key, kind, id)`. Six CLIs' common configs and two
/// CLIs' globals all use a literal id, so `id` alone would collide and one
/// CLI's config would overwrite another's.
#[test]
fn literal_config_ids_coexist_across_clis_and_kinds() {
    let conn = test_conn();
    for (cli, payload) in [(Cli::Claude, "claude"), (Cli::Codex, "codex")] {
        db_put(
            &conn,
            DbTable::CliConfig(cli, CliConfigKind::Common),
            "common",
            &json!({ "id": "common", "config": payload }),
        )
        .unwrap();
    }
    for (cli, payload) in [
        (Cli::OhMyOpenAgent, "openagent"),
        (Cli::OhMyOpenCodeSlim, "slim"),
    ] {
        db_put(
            &conn,
            DbTable::CliConfig(cli, CliConfigKind::Global),
            "global",
            &json!({ "id": "global", "config": payload }),
        )
        .unwrap();
    }

    for (cli, expected) in [(Cli::Claude, "claude"), (Cli::Codex, "codex")] {
        let record = db_get(&conn, DbTable::CliConfig(cli, CliConfigKind::Common), "common")
            .unwrap()
            .expect("common config must survive its sibling");
        assert_eq!(record["config"], expected);
    }
    for (cli, expected) in [
        (Cli::OhMyOpenAgent, "openagent"),
        (Cli::OhMyOpenCodeSlim, "slim"),
    ] {
        let record = db_get(&conn, DbTable::CliConfig(cli, CliConfigKind::Global), "global")
            .unwrap()
            .expect("global config must survive its sibling");
        assert_eq!(record["config"], expected);
    }

    // Same cli_key and id, different kind, must also stay separate.
    db_put(
        &conn,
        DbTable::CliConfig(Cli::Claude, CliConfigKind::Prompt),
        "common",
        &json!({ "id": "common", "content": "prompt body" }),
    )
    .unwrap();
    let common = db_get(
        &conn,
        DbTable::CliConfig(Cli::Claude, CliConfigKind::Common),
        "common",
    )
    .unwrap()
    .unwrap();
    let prompt = db_get(
        &conn,
        DbTable::CliConfig(Cli::Claude, CliConfigKind::Prompt),
        "common",
    )
    .unwrap()
    .unwrap();
    assert_eq!(common["config"], "claude");
    assert_eq!(prompt["content"], "prompt body");
}

/// Rewriting a record must update it in place, not insert a second scoped row
/// or lose the original `created_at`.
#[test]
fn rewriting_a_scoped_record_updates_in_place() {
    let conn = test_conn();
    let table = DbTable::CliProvider(Cli::Codex);
    db_put(&conn, table, "p1", &json!({ "id": "p1", "name": "first" })).unwrap();
    let created = db_get(&conn, table, "p1").unwrap().unwrap()["created_at"].clone();

    db_put(&conn, table, "p1", &json!({ "id": "p1", "name": "second" })).unwrap();

    let rows = db_list(&conn, table, None).unwrap();
    assert_eq!(rows.len(), 1, "re-put must not create a duplicate row");
    assert_eq!(rows[0]["name"], "second");
    assert_eq!(
        rows[0]["created_at"], created,
        "created_at must survive a rewrite"
    );
    assert_eq!(db_count(&conn, DbTable::CliProvider(Cli::Kimi)).unwrap(), 0);
}

/// `relay_entity` holds nine kinds in one table; a kind scope is what keeps
/// channels and check-in records apart.
#[test]
fn relay_kinds_do_not_see_each_other() {
    let conn = test_conn();
    db_put(
        &conn,
        DbTable::RelayEntity(RelayKind::Channel),
        "e1",
        &json!({ "id": "e1", "name": "a channel" }),
    )
    .unwrap();
    db_put(
        &conn,
        DbTable::RelayEntity(RelayKind::Model),
        "e2",
        &json!({ "id": "e2", "model_id": "gpt-5.5", "channel_id": "e1" }),
    )
    .unwrap();
    db_put(
        &conn,
        DbTable::RelayEntity(RelayKind::CheckinRecord),
        "e3",
        &json!({ "id": "e3", "ts": 1 }),
    )
    .unwrap();

    assert_eq!(
        db_list(&conn, DbTable::RelayEntity(RelayKind::Channel), None)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db_list(&conn, DbTable::RelayEntity(RelayKind::Model), None)
            .unwrap()
            .len(),
        1
    );
    assert!(db_list(&conn, DbTable::RelayEntity(RelayKind::Plugin), None)
        .unwrap()
        .is_empty());
}

// ---- schema parity: fresh install vs upgrade --------------------------------
//
// A fresh install runs `migrate_v1` (legacy per-CLI tables) and then `v33`
// folds them; an existing install skips v1 and only runs v33. If the two paths
// disagree — v1 building a shape that v33's `CREATE TABLE IF NOT EXISTS` then
// skips, or v1 creating an index v33 never recreates — the divergence stays
// invisible until a query hits a missing column on one of them. That is exactly
// the class of bug the first draft of this migration had.

/// Sorted `table(column:type:notnull)` plus sorted index DDL, so the comparison
/// does not depend on creation order.
fn schema_fingerprint(conn: &Connection) -> (Vec<String>, Vec<String>) {
    let table_names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let mut tables: Vec<String> = table_names
        .into_iter()
        .map(|table| {
            let columns: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| {
                    Ok(format!(
                        "{}:{}:{}",
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(5)?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            format!("{table}({})", columns.join(","))
        })
        .collect();
    tables.sort();

    let mut indexes: Vec<String> = conn
        .prepare(
            "SELECT sql FROM sqlite_master
             WHERE type='index' AND sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    indexes.sort();

    (tables, indexes)
}

#[test]
fn a_fresh_database_matches_an_upgraded_one() {
    let fresh = test_conn();
    let fresh_fingerprint = schema_fingerprint(&fresh);

    // Reproduce an installation that reached v32 without ever seeing the
    // consolidation: legacy tables present, consolidated tables absent.
    let upgraded = test_conn();
    for table in migrations::legacy_tables_for_test() {
        upgraded
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                     id TEXT PRIMARY KEY NOT NULL,
                     data BLOB NOT NULL CHECK (json_valid(data, 4)),
                     created_at TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                 );"
            ))
            .unwrap();
    }
    for table in ["cli_provider", "cli_official_account", "cli_config"] {
        upgraded
            .execute_batch(&format!("DROP TABLE IF EXISTS {table};"))
            .unwrap();
    }
    migrations::set_user_version(&upgraded, 32).unwrap();
    let mut upgraded = upgraded;
    migrations::run_all(&mut upgraded).unwrap();

    let upgraded_fingerprint = schema_fingerprint(&upgraded);

    assert_eq!(
        fresh_fingerprint.0, upgraded_fingerprint.0,
        "fresh and upgraded databases disagree on tables/columns"
    );
    assert_eq!(
        fresh_fingerprint.1, upgraded_fingerprint.1,
        "fresh and upgraded databases disagree on indexes"
    );
}
