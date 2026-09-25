use super::{
    parsers, persist_records, GatewaySessionUsageImportResult, SessionUsageRecord, SourceState,
    SqliteDbState,
};
use crate::coding::proxy_gateway::types::{
    GatewayUsageTool, SessionUsageGranularity, SessionUsageMetadata,
};
use crate::coding::proxy_gateway::usage_parser::TokenUsage;
use rusqlite::{params_from_iter, types::Value, Connection, OpenFlags, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// OpenCode 2.0 renamed the native session store: `session`/`message` became
/// `session_v2`/`session_message`. An upgraded database keeps both (the 2.0
/// migration copies the V1 rows, or renames `session` in place), so `session_v2`
/// marks the live store and every other database keeps using the V1 tables.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionSchema {
    V1,
    V2,
}

impl SessionSchema {
    fn detect(source: &Connection) -> Result<Self, String> {
        let found = source
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'session_v2'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        Ok(if found.is_some() { Self::V2 } else { Self::V1 })
    }
}

/// One native session plus its usage watermark. `fork_session_id` only exists in
/// the V2 store; V1 sessions never carry one.
struct SessionRow {
    id: String,
    fork_session_id: Option<String>,
    time_created: i64,
    watermark: i64,
}

/// The V2 session's own usage totals. Auxiliary calls that never get a message
/// row (title generation in particular) only show up here: on 2.0.15 a session
/// with one assistant step reports twice that step's tokens.
struct SessionAggregate {
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    model: Option<String>,
}

fn select_aggregate(
    source: &Connection,
    session_id: &str,
) -> Result<Option<SessionAggregate>, String> {
    let counter = |row: &rusqlite::Row, index: usize| -> Result<u64, rusqlite::Error> {
        Ok(row
            .get::<_, Option<i64>>(index)?
            .unwrap_or(0)
            .max(0)
            .unsigned_abs())
    };
    source
        .query_row(
            "SELECT tokens_input, tokens_output, tokens_reasoning, tokens_cache_read,
                    tokens_cache_write, model
             FROM session_v2 WHERE id = ?1",
            [session_id],
            |row| {
                Ok(SessionAggregate {
                    input: counter(row, 0)?,
                    output: counter(row, 1)?,
                    reasoning: counter(row, 2)?,
                    cache_read: counter(row, 3)?,
                    cache_write: counter(row, 4)?,
                    model: row
                        .get::<_, Option<String>>(5)?
                        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
                        .and_then(|value| {
                            value
                                .get("id")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string)
                        }),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

/// Usage the session aggregate holds beyond the per-call rows. It is recorded at
/// session granularity so it is never mistaken for another provider call, and it
/// is skipped for forks, whose aggregate can include the copied parent history
/// that the message reader deliberately drops.
fn session_difference_record(
    session: &SessionRow,
    aggregate: &SessionAggregate,
    records: &[SessionUsageRecord],
) -> Option<SessionUsageRecord> {
    let sum = |read: fn(&TokenUsage) -> Option<u64>| {
        records
            .iter()
            .map(|record| read(&record.usage).unwrap_or(0))
            .sum::<u64>()
    };
    let usage = TokenUsage {
        input_tokens: Some(
            aggregate
                .input
                .saturating_sub(sum(|usage| usage.input_tokens)),
        ),
        // The aggregate counts reasoning apart while every imported row folds it
        // into the output counters.
        output_tokens: Some(
            aggregate
                .output
                .saturating_add(aggregate.reasoning)
                .saturating_sub(sum(|usage| usage.output_tokens)),
        ),
        cache_read_tokens: Some(
            aggregate
                .cache_read
                .saturating_sub(sum(|usage| usage.cache_read_tokens)),
        ),
        cache_creation_tokens: Some(
            aggregate
                .cache_write
                .saturating_sub(sum(|usage| usage.cache_creation_tokens)),
        ),
        ..Default::default()
    };
    if usage.total_tokens().unwrap_or(0) == 0 {
        return None;
    }
    Some(SessionUsageRecord {
        metadata: SessionUsageMetadata {
            granularity: SessionUsageGranularity::Session,
            ..Default::default()
        },
        request_id: format!(
            "SESSION:{}:{}:aggregate",
            GatewayUsageTool::OpenCode.as_str(),
            session.id
        ),
        legacy_request_ids: Vec::new(),
        cli_key: GatewayUsageTool::OpenCode,
        // Native session metadata, never a guess from the current settings.
        model: aggregate
            .model
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        usage,
        created_at: session.watermark / 1000,
        session_id: session.id.clone(),
        reported_cost_usd: None,
    })
}

/// Recent native writes may only exist in WAL, so the cursor is derived from the
/// session row and its messages; the main file's mtime is not a valid watermark.
fn select_sessions(source: &Connection, schema: SessionSchema) -> Result<Vec<SessionRow>, String> {
    let sql = match schema {
        // V1 sessions never fork, so the boundary column stays a literal rather
        // than a requirement on the native table.
        SessionSchema::V1 => {
            "SELECT s.id, NULL, 0,
                    MAX(s.time_updated, COALESCE(MAX(m.time_updated), s.time_updated))
             FROM session s LEFT JOIN message m ON m.session_id = s.id
             GROUP BY s.id ORDER BY 4"
        }
        SessionSchema::V2 => {
            "SELECT s.id, s.fork_session_id, s.time_created,
                    MAX(s.time_updated, COALESCE(MAX(m.time_updated), s.time_updated))
             FROM session_v2 s LEFT JOIN session_message m ON m.session_id = s.id
             GROUP BY s.id ORDER BY 4"
        }
    };
    let mut statement = source.prepare(sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                fork_session_id: row.get(1)?,
                time_created: row.get(2)?,
                watermark: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

pub(super) fn sync_database(
    db: &SqliteDbState,
    path: &Path,
    states: &mut HashMap<String, SourceState>,
    claimed_proxies: &mut HashMap<String, String>,
    now: i64,
) -> Result<GatewaySessionUsageImportResult, String> {
    if !path.is_file() {
        return Ok(GatewaySessionUsageImportResult::default());
    }
    let source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    source
        .busy_timeout(Duration::from_millis(200))
        .map_err(|error| error.to_string())?;
    let schema = SessionSchema::detect(&source)?;
    let sessions = select_sessions(&source, schema)?;
    let mut result = GatewaySessionUsageImportResult::default();
    for session in sessions {
        result.scanned_files += 1;
        let source_id = format!("opencode:session:{}", session.id);
        let mut state = states.get(&source_id).cloned().unwrap_or_default();
        let parser_revision = parsers::revision(GatewayUsageTool::OpenCode);
        if state.parser_revision == parser_revision
            && state.modified_nanos == session.watermark.max(0) as u64
            && !state.pending
        {
            continue;
        }
        state.parser_revision = parser_revision;
        state.modified_nanos = session.watermark.max(0) as u64;
        state.pending = false;
        let parsed = read_session(&source, schema, &session)?;
        state.pending = parsed.pending;
        let mut records = parsed.records;
        super::adopt_known_records(&mut state, &mut records, states);
        let (state, changes) =
            persist_records(db, &source_id, state, records, claimed_proxies, now)?;
        states.insert(source_id, state);
        result.merge(changes);
    }
    Ok(result)
}

fn read_session(
    source: &Connection,
    schema: SessionSchema,
    session: &SessionRow,
) -> Result<parsers::ParsedSession, String> {
    // A fork copies the parent's messages into its own store. OpenCode's own
    // stats only count what the fork created after it branched, and so must the
    // import, or inherited history is counted twice.
    let fork_boundary = session
        .fork_session_id
        .as_ref()
        .map(|_| session.time_created);
    let (sql, args) = match schema {
        SessionSchema::V1 => (
            "SELECT id, data, time_created, NULL FROM message WHERE session_id = ?1 ORDER BY time_created",
            vec![Value::Text(session.id.clone())],
        ),
        // The V2 store keeps the message kind in its own column, and only an
        // assistant step carries per-call usage; the other kinds hold none, so
        // their provider calls are left to the session aggregate below.
        SessionSchema::V2 => (
            "SELECT id, data, time_created, type FROM session_message
             WHERE session_id = ?1 AND type = 'assistant'
               AND (?2 IS NULL OR time_created >= ?2)
             ORDER BY time_created",
            vec![
                Value::Text(session.id.clone()),
                fork_boundary.map_or(Value::Null, Value::Integer),
            ],
        ),
    };
    let mut query = source.prepare(sql).map_err(|error| error.to_string())?;
    let messages = query
        .query_map(params_from_iter(args), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut parsed = parsers::ParsedSession::default();
    for message in messages {
        let (id, data, created_at, message_type) = message.map_err(|error| error.to_string())?;
        let mut value: serde_json::Value =
            serde_json::from_str(&data).map_err(|error| error.to_string())?;
        // The V2 row's own JSON omits the message kind its schema declares, so
        // the reader passes the column value down before classifying the row.
        if let Some(object) = value.as_object_mut() {
            object.insert("id".to_string(), serde_json::Value::String(id));
            if let Some(message_type) = message_type {
                object.insert("type".to_string(), serde_json::Value::String(message_type));
            }
        }
        if !parsers::is_opencode_usage_row(&value) {
            continue;
        }
        if value
            .pointer("/time/completed")
            .is_none_or(serde_json::Value::is_null)
        {
            parsed.pending = true;
            continue;
        }
        if let Some(record) = parsers::parse_value(
            GatewayUsageTool::OpenCode,
            &value,
            &session.id,
            0,
            created_at / 1000,
        ) {
            parsed.records.push(record);
        }
    }
    // Only a settled, unforked session has an aggregate that the rows above are
    // supposed to add up to; anything still running is retried on a later sync.
    // A store without the aggregate columns still yields its per-call rows.
    if schema == SessionSchema::V2 && !parsed.pending && session.fork_session_id.is_none() {
        match select_aggregate(source, &session.id) {
            Ok(Some(aggregate)) => {
                if let Some(record) =
                    session_difference_record(session, &aggregate, &parsed.records)
                {
                    parsed.records.push(record);
                }
            }
            Ok(None) => {}
            Err(error) => log::warn!("OpenCode session aggregate is unavailable: {error}"),
        }
    }
    Ok(parsed)
}

pub(super) fn cost_records(path: &Path) -> Result<Vec<SessionUsageRecord>, String> {
    let source = super::hermes::open_read_only(path)?;
    let schema = SessionSchema::detect(&source)?;
    let mut records = Vec::new();
    for session in select_sessions(&source, schema)? {
        // Unfinished messages do not prevent pricing completed, committed ones.
        records.extend(read_session(&source, schema, &session)?.records);
    }
    Ok(records)
}
