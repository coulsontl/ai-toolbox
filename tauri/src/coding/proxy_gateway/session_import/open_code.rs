use super::{
    parsers, persist_records, GatewaySessionUsageImportResult, SourceState, SqliteDbState,
};
use crate::coding::proxy_gateway::types::GatewayUsageTool;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

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
    // Recent native writes may only exist in WAL. Main-file mtime is not a
    // valid cursor; include the messages' watermark for each session.
    let mut sessions_query = source
        .prepare(
            "SELECT s.id, MAX(s.time_updated, COALESCE(MAX(m.time_updated), s.time_updated))
         FROM session s LEFT JOIN message m ON m.session_id = s.id
         GROUP BY s.id ORDER BY 2",
        )
        .map_err(|error| error.to_string())?;
    let sessions = sessions_query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut result = GatewaySessionUsageImportResult::default();
    for (session_id, watermark) in sessions {
        result.scanned_files += 1;
        let source_id = format!("opencode:session:{session_id}");
        let mut state = states.get(&source_id).cloned().unwrap_or_default();
        let parser_revision = parsers::revision(GatewayUsageTool::OpenCode);
        if state.parser_revision == parser_revision
            && state.modified_nanos == watermark.max(0) as u64
            && !state.pending
        {
            continue;
        }
        state.parser_revision = parser_revision;
        state.modified_nanos = watermark.max(0) as u64;
        state.pending = false;
        let mut query = source.prepare("SELECT id, data, time_created FROM message WHERE session_id = ?1 ORDER BY time_created")
            .map_err(|error| error.to_string())?;
        let messages = query
            .query_map([&session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut records = Vec::new();
        for message in messages {
            let (id, data, created_at) = message.map_err(|error| error.to_string())?;
            let mut value: serde_json::Value =
                serde_json::from_str(&data).map_err(|error| error.to_string())?;
            if value.get("role").and_then(serde_json::Value::as_str) != Some("assistant") {
                continue;
            }
            if value
                .pointer("/time/completed")
                .is_none_or(serde_json::Value::is_null)
            {
                state.pending = true;
                continue;
            }
            if let Some(object) = value.as_object_mut() {
                object.insert("id".to_string(), serde_json::Value::String(id));
            }
            if let Some(record) = parsers::parse_value(
                GatewayUsageTool::OpenCode,
                &value,
                &session_id,
                0,
                created_at / 1000,
            ) {
                records.push(record);
            }
        }
        super::adopt_known_records(&mut state, &mut records, states);
        let (state, changes) =
            persist_records(db, &source_id, state, records, claimed_proxies, now)?;
        states.insert(source_id, state);
        result.merge(changes);
    }
    Ok(result)
}
