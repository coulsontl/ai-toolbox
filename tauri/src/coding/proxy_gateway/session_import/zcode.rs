//! ZCode local usage, read out of the CLI's SQLite database.
//!
//! Every other tool here stores transcripts on disk; ZCode keeps one database
//! at `cli/db/db.sqlite` and records usage in dedicated tables rather than in
//! the conversation itself. `model_usage` is the one that matters: one row per
//! model request, already carrying the token counts and the model id, so no
//! parsing of message payloads is involved.
//!
//! Two columns decide what counts:
//!
//! - `attempt_index` — a retried request is a second row under the same
//!   `logical_request_id`. Only attempt 0 is billed, so only attempt 0 is read;
//!   counting the rest would double the totals.
//! - `status` — `running`, `cancelled` and `error` rows carry zero tokens.
//!   `completed` is the only status worth reading, and filtering on it also
//!   keeps a request that is *still in flight* out of the statistics, rather
//!   than importing it once with zeros and again with the real numbers.
//!
//! `input_tokens` is the provider's *prompt* count and **includes** cached
//! tokens (measured locally: input 95855 with cache_read 83840). The importer's
//! `input_tokens` means fresh, non-cached input, so the cache columns are
//! subtracted here; keeping both would bill the cached part twice.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;
use serde_json::Value;

use super::parsers::{native_record, session_duration_ms, ParsedSession};
use super::{GatewayUsageTool, SessionUsageRecord};
use crate::coding::proxy_gateway::usage_parser::TokenUsage;

/// Columns read from `model_usage`, in order. The project is a property of the
/// session row, so it comes from a correlated subquery; the `session` table is
/// optional (a trimmed or test database may not carry it), hence the fallback
/// variant that selects a literal instead.
fn usage_query(session_table_present: bool) -> String {
    let project = if session_table_present {
        "(SELECT s.directory FROM session s WHERE s.id = u.session_id)"
    } else {
        "NULL"
    };
    format!(
        "SELECT u.id, u.session_id, u.provider_id, u.model_id, u.started_at, \
         u.completed_at, u.input_tokens, u.output_tokens, u.cache_read_input_tokens, \
         u.cache_creation_input_tokens, u.raw_usage_json, {project} \
         FROM model_usage u \
         WHERE u.attempt_index = 0 AND u.status = 'completed' \
         ORDER BY u.started_at ASC"
    )
}

/// Reads every completed request out of one ZCode database.
///
/// The whole file is one "session" from the importer's point of view — the
/// per-row `session_id` is preserved on each record, so the statistics still
/// break down by conversation, but the file-level cursor is what tracks
/// incremental progress.
pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open ZCode database {}: {error}", path.display()))?;

    let session_table_present = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'session'",
            [],
            |_| Ok(()),
        )
        .is_ok();
    let mut statement = connection
        .prepare(&usage_query(session_table_present))
        .map_err(|error| format!("Failed to prepare ZCode usage query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(UsageRow {
                id: row.get(0)?,
                session_id: row.get(1)?,
                provider_id: row.get(2)?,
                model_id: row.get(3)?,
                started_at: row.get(4)?,
                completed_at: row.get(5)?,
                input_tokens: row.get(6)?,
                output_tokens: row.get(7)?,
                cache_read_tokens: row.get(8)?,
                cache_creation_tokens: row.get(9)?,
                raw_usage: row.get(10)?,
                project_path: row.get::<_, Option<String>>(11)?,
            })
        })
        .map_err(|error| format!("Failed to query ZCode usage: {error}"))?;

    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    for row in rows {
        let row = row.map_err(|error| format!("Failed to read ZCode usage row: {error}"))?;
        let record = row.into_record(fallback);
        records.insert(record.request_id.clone(), record);
    }

    Ok(ParsedSession {
        records: records.into_values().collect(),
        // ZCode records the model's own provider, not a gateway provider, so
        // there is nothing to reconcile against a proxied request.
        pending: false,
        ..Default::default()
    })
}

struct UsageRow {
    id: String,
    session_id: String,
    provider_id: String,
    model_id: String,
    started_at: i64,
    completed_at: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    raw_usage: Option<String>,
    project_path: Option<String>,
}

impl UsageRow {
    fn into_record(self, fallback: i64) -> SessionUsageRecord {
        // Both timestamps are epoch milliseconds; `completed_at` is the closer
        // match to when the tokens were actually produced, and it is what tokei
        // bills against. `started_at` remains the fallback for old rows.
        let timestamp_ms = if self.completed_at > 0 {
            self.completed_at
        } else {
            self.started_at
        };
        let created_at = if timestamp_ms > 0 {
            timestamp_ms / 1000
        } else {
            fallback
        };

        // `input_tokens` includes the cached part, so the four buckets are
        // re-cut here: fresh = prompt - cache_read - cache_creation.
        let cache_read = self.cache_read_tokens.max(0) as u64;
        let cache_creation = self.cache_creation_tokens.max(0) as u64;
        let input = (self.input_tokens.max(0) as u64)
            .saturating_sub(cache_read)
            .saturating_sub(cache_creation);
        let mut record = native_record(
            GatewayUsageTool::Zcode,
            &self.session_id,
            &self.id,
            Some(self.model_id),
            TokenUsage {
                input_tokens: Some(input),
                output_tokens: Some(self.output_tokens.max(0) as u64),
                cache_read_tokens: Some(cache_read),
                cache_creation_tokens: Some(cache_creation),
                envelope_id: None,
            },
            created_at,
        );
        record.metadata.native_provider = Some(self.provider_id);
        record.metadata.reported_total_tokens = reported_total(&self.raw_usage);
        // `started_at` and `completed_at` bracket this one upstream request,
        // so the difference is the call's real elapsed time.
        record.duration_ms = session_duration_ms(self.started_at, self.completed_at);
        record.project_path = self.project_path.filter(|path| !path.trim().is_empty());
        record
    }
}

/// Reads the provider's own total out of `raw_usage_json`.
///
/// ZCode stores the upstream usage blob verbatim, and its shape varies by
/// provider, so this is best-effort: a missing or unfamiliar field just means
/// the total is estimated from the token columns instead.
fn reported_total(raw: &Option<String>) -> Option<u64> {
    let usage: Value = serde_json::from_str(raw.as_deref()?).ok()?;
    ["total_tokens", "totalTokens", "total"]
        .iter()
        .find_map(|key| usage.get(*key).and_then(Value::as_u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_with_usage_rows() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("db.sqlite");
        let connection = Connection::open(&path).expect("open");
        connection
            .execute_batch(
                "CREATE TABLE model_usage (
                    id TEXT PRIMARY KEY,
                    attempt_index INTEGER NOT NULL DEFAULT 0,
                    session_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    model_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    completed_at INTEGER NOT NULL DEFAULT 0,
                    input_tokens INTEGER NOT NULL DEFAULT 0,
                    output_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
                    raw_usage_json TEXT
                );",
            )
            .expect("schema");
        // One billed request, its retry, a cancelled one, and a running one.
        connection
            .execute_batch(
                r#"INSERT INTO model_usage VALUES
                   ('u1', 0, 'sess_a', 'account:zai', 'GLM-5.3', 'completed', 1791123438145, 1791123439999, 100, 20, 30, 0, '{"total_tokens": 150}'),
                   ('u2', 1, 'sess_a', 'account:zai', 'GLM-5.3', 'completed', 1791123438200, 0, 100, 20, 30, 0, NULL),
                   ('u3', 0, 'sess_b', 'account:zai', 'GLM-5.3', 'cancelled', 1791123439000, 0, 0, 0, 0, 0, NULL),
                   ('u4', 0, 'sess_b', 'account:zai', 'GLM-5.3', 'running',   1791123440000, 0, 0, 0, 0, 0, NULL);"#,
            )
            .expect("rows");
        directory
    }

    #[test]
    fn only_billed_completed_attempts_become_records() {
        let directory = database_with_usage_rows();
        let parsed = parse(&directory.path().join("db.sqlite"), 0).expect("parse");

        // The retry, the cancelled request and the in-flight one are all out.
        assert_eq!(parsed.records.len(), 1);
        let record = parsed.records.first().unwrap();
        assert_eq!(record.cli_key, GatewayUsageTool::Zcode);
        assert_eq!(record.session_id, "sess_a");
        assert_eq!(record.model, "GLM-5.3");
        // `input_tokens` includes the cached part: 100 - 30 read - 0 write.
        assert_eq!(record.usage.input_tokens, Some(70));
        assert_eq!(record.usage.cache_read_tokens, Some(30));
        assert_eq!(
            record.metadata.native_provider.as_deref(),
            Some("account:zai")
        );
        assert_eq!(record.metadata.reported_total_tokens, Some(150));
        // `completed_at` wins over `started_at`; milliseconds on disk, seconds
        // in the record.
        assert_eq!(record.created_at, 1791123439);
    }

    #[test]
    fn cache_tokens_cannot_push_fresh_input_below_zero() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("db.sqlite");
        let connection = Connection::open(&path).expect("open");
        connection
            .execute_batch(
                "CREATE TABLE model_usage (
                    id TEXT PRIMARY KEY,
                    attempt_index INTEGER NOT NULL DEFAULT 0,
                    session_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    model_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    completed_at INTEGER NOT NULL DEFAULT 0,
                    input_tokens INTEGER NOT NULL DEFAULT 0,
                    output_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
                    raw_usage_json TEXT
                );
                INSERT INTO model_usage VALUES
                  ('u1', 0, 'sess_a', 'account:zai', 'GLM-5.3', 'completed', 1791123438145, 0, 100, 20, 80, 50, NULL);",
            )
            .expect("schema");

        let parsed = parse(&path, 0).expect("parse");
        let record = parsed.records.first().unwrap();
        assert_eq!(record.usage.input_tokens, Some(0));
        assert_eq!(record.usage.cache_read_tokens, Some(80));
        assert_eq!(record.usage.cache_creation_tokens, Some(50));
    }

    #[test]
    fn a_database_without_the_usage_table_is_an_error_not_an_empty_read() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("db.sqlite");
        Connection::open(&path).expect("open");

        // A silent empty result would look like "no usage yet" and freeze the
        // file's cursor at zero forever.
        assert!(parse(&path, 0).is_err());
    }

    #[test]
    fn a_missing_database_is_an_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        assert!(parse(&directory.path().join("absent.sqlite"), 0).is_err());
    }
}
#[cfg(test)]
mod end_to_end_tests {
    use super::*;
    use crate::coding::proxy_gateway::{settings, types::GatewaySessionImportCli, usage_stats};
    use rusqlite::Connection;

    /// A ZCode database is one file, so the importer's file cursor is also the
    /// only thing standing between "read once" and "read every 60 seconds and
    /// double the totals". This drives the real `sync_sources` twice.
    #[test]
    fn a_zcode_database_is_imported_once_and_not_again() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("db.sqlite");
        let connection = Connection::open(&path).expect("open");
        connection
            .execute_batch(
                "CREATE TABLE model_usage (
                    id TEXT PRIMARY KEY,
                    attempt_index INTEGER NOT NULL DEFAULT 0,
                    session_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    model_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    completed_at INTEGER NOT NULL DEFAULT 0,
                    input_tokens INTEGER NOT NULL DEFAULT 0,
                    output_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
                    raw_usage_json TEXT
                );
                INSERT INTO model_usage VALUES
                  ('u1', 0, 'sess_a', 'account:zai', 'GLM-5.3', 'completed', 1791123438145, 0, 100, 20, 30, 0, NULL),
                  ('u2', 0, 'sess_a', 'account:zai', 'GLM-5.3', 'completed', 1791123439000, 0, 200, 40, 60, 0, NULL);",
            )
            .expect("schema");

        let db = crate::db::SqliteDbState::in_memory_for_test().expect("db");
        let sources = [(GatewayUsageTool::Zcode, path.clone())];

        let first =
            super::super::sync_sources(&db, &sources, 1_800_000_000, false).expect("first sync");
        assert_eq!(first.inserted_records, 2);

        let summary =
            usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Zcode), true)
                .expect("summary");
        assert_eq!(summary.total_requests, 2);

        // Unchanged file: the ledger must keep it out of the second pass.
        let second =
            super::super::sync_sources(&db, &sources, 1_800_000_060, false).expect("second sync");
        assert_eq!(second.inserted_records, 0);
        assert_eq!(second.updated_records, 0);
        let summary =
            usage_stats::usage_summary(&db, None, None, Some(GatewayUsageTool::Zcode), true)
                .expect("summary");
        assert_eq!(summary.total_requests, 2);
    }

    /// The display toggle is the one gate; a disabled import must be a no-op
    /// rather than an error, so a machine with no ZCode install is unaffected.
    #[test]
    fn the_usage_toggle_gates_the_import() {
        let db = crate::db::SqliteDbState::in_memory_for_test().expect("db");
        let mut current = settings::load_settings_from_sqlite_state(&db).expect("settings");
        current.session_usage_enabled = false;
        settings::save_settings_to_sqlite_state(&db, current).expect("save");

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("db.sqlite");
        Connection::open(&path).expect("open");

        let result = tauri::async_runtime::block_on(super::super::import_session_usage(
            db.clone(),
            crate::coding::proxy_gateway::types::GatewaySessionUsageImportInput {
                cli_key: GatewaySessionImportCli::Zcode,
            },
        ))
        .expect("import");
        assert_eq!(result.scanned_files, 0);
    }
}
