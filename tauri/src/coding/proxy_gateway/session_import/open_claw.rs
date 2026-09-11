use super::hermes::{columns, open_read_only};
use super::parsers::{native_record, number, reported_cost, string, timestamp, ParsedSession};
use super::{
    adopt_known_records, persist_records, GatewaySessionUsageImportResult, GatewayUsageTool,
    SessionUsageRecord, SourceState, SqliteDbState, TokenUsage,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Cursor};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(super) fn is_transcript(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if name.contains(".checkpoint.") || name.ends_with(".trajectory.jsonl") {
        return false;
    }
    path.components().any(|part| part.as_os_str() == "sessions")
        && (name.ends_with(".jsonl")
            || name.contains(".jsonl.deleted.")
            || name.contains(".jsonl.reset."))
}

pub(super) fn canonical_files(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut sqlite_sessions = HashMap::<PathBuf, HashSet<String>>::new();
    files
        .into_iter()
        .filter(|file| {
            let Some(sessions_dir) = file
                .ancestors()
                .find(|parent| parent.file_name().is_some_and(|name| name == "sessions"))
            else {
                return true;
            };
            let db_path = sessions_dir
                .parent()
                .unwrap_or(sessions_dir)
                .join("agent/openclaw-agent.sqlite");
            let ids = sqlite_sessions.entry(db_path.clone()).or_insert_with(|| {
                let Ok(source) = open_read_only(&db_path) else {
                    return HashSet::new();
                };
                let Ok(mut query) =
                    source.prepare("SELECT DISTINCT session_id FROM transcript_events")
                else {
                    return HashSet::new();
                };
                query
                    .query_map([], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| rows.filter_map(Result::ok).collect())
                    .unwrap_or_default()
            });
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            let session = name.split(".jsonl").next().unwrap_or("");
            !ids.contains(session)
        })
        .collect()
}

pub(super) fn parse_entry(
    session: &str,
    index: usize,
    entry: &Value,
    fallback: i64,
) -> Option<SessionUsageRecord> {
    if entry.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = entry.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let raw = message.get("usage")?.as_object()?;
    if ![
        "input",
        "input_tokens",
        "inputTokens",
        "prompt_tokens",
        "output",
        "output_tokens",
        "outputTokens",
        "completion_tokens",
    ]
    .iter()
    .any(|key| raw.contains_key(*key))
    {
        return None;
    }
    let raw = message.get("usage")?;
    let cached = number(
        raw,
        &[
            "cacheRead",
            "cache_read",
            "cache_read_input_tokens",
            "cached_input_tokens",
            "cached",
            "cached_tokens",
        ],
    )
    .max(
        raw.pointer("/input_tokens_details/cached_tokens")
            .or_else(|| raw.pointer("/prompt_tokens_details/cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    );
    let written = number(
        raw,
        &[
            "cacheWrite",
            "cache_write",
            "cache_creation_input_tokens",
            "cache_write_input_tokens",
        ],
    )
    .max(
        raw.pointer("/input_tokens_details/cache_write_tokens")
            .or_else(|| raw.pointer("/prompt_tokens_details/cache_write_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    );
    let mut input = number(
        raw,
        &[
            "input",
            "inputTokens",
            "input_tokens",
            "promptTokens",
            "prompt_tokens",
            "prompt_n",
        ],
    );
    let direct = raw.get("input").is_some();
    if raw.get("cached_tokens").is_some()
        || raw.pointer("/input_tokens_details/cached_tokens").is_some()
        || raw
            .pointer("/prompt_tokens_details/cached_tokens")
            .is_some()
        || (!direct && (raw.get("cached_input_tokens").is_some() || raw.get("cached").is_some()))
    {
        input = input.saturating_sub(cached);
    }
    if !direct
        && (raw.get("cache_write_input_tokens").is_some()
            || raw
                .pointer("/input_tokens_details/cache_write_tokens")
                .is_some()
            || raw
                .pointer("/prompt_tokens_details/cache_write_tokens")
                .is_some())
    {
        input = input.saturating_sub(written);
    }
    let usage = TokenUsage {
        input_tokens: Some(input),
        output_tokens: Some(number(
            raw,
            &[
                "output",
                "outputTokens",
                "output_tokens",
                "completionTokens",
                "completion_tokens",
            ],
        )),
        cache_read_tokens: Some(cached),
        cache_creation_tokens: Some(written),
        envelope_id: string(message, &["/responseId", "/response_id"]),
    };
    let identity = string(entry, &["/id"]).unwrap_or_else(|| format!("entry-{index}"));
    let mut record = native_record(
        GatewayUsageTool::OpenClaw,
        session,
        &identity,
        string(message, &["/model"]),
        usage,
        timestamp(entry)
            .or_else(|| timestamp(message))
            .unwrap_or(fallback),
    );
    if let Some(envelope) = &record.usage.envelope_id {
        record.request_id = format!(
            "SESSION:openclaw:response:{}:{envelope}",
            string(message, &["/provider"]).unwrap_or_default()
        );
    }
    record.metadata.native_provider = string(message, &["/provider"]);
    record.metadata.reported_total_tokens = raw
        .get("totalTokens")
        .or_else(|| raw.get("total_tokens"))
        .or_else(|| raw.get("total"))
        .and_then(Value::as_u64);
    record.reported_cost_usd = reported_cost(raw.pointer("/cost/total"));
    if record.reported_cost_usd.is_some() {
        record.metadata.cost_source = Some("reported".into());
    }
    Some(record)
}

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    super::pi::parse(GatewayUsageTool::OpenClaw, path, fallback)
}

fn persist_snapshot(
    db: &SqliteDbState,
    source_id: &str,
    mut records: Vec<SessionUsageRecord>,
    states: &mut HashMap<String, SourceState>,
    claims: &mut HashMap<String, String>,
    now: i64,
) -> Result<GatewaySessionUsageImportResult, String> {
    let mut state = states.get(source_id).cloned().unwrap_or_default();
    state.pending = false;
    adopt_known_records(&mut state, &mut records, states);
    let (state, changes) = persist_records(db, source_id, state, records, claims, now)?;
    states.insert(source_id.into(), state);
    Ok(changes)
}

pub(super) fn sync_databases(
    db: &SqliteDbState,
    root: &Path,
    states: &mut HashMap<String, SourceState>,
    claims: &mut HashMap<String, String>,
    now: i64,
) -> Result<GatewaySessionUsageImportResult, String> {
    let mut result = GatewaySessionUsageImportResult::default();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "openclaw-agent.sqlite")
    {
        let source = open_read_only(entry.path())?;
        result.scanned_files += 1;
        let mut query = source.prepare("SELECT session_id, seq, event_json, created_at FROM transcript_events ORDER BY session_id, seq").map_err(|error| error.to_string())?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut sessions = BTreeMap::<String, BTreeMap<String, SessionUsageRecord>>::new();
        for row in rows {
            let (session, seq, json, time) = row.map_err(|error| error.to_string())?;
            let event = serde_json::from_str::<Value>(&json).map_err(|error| error.to_string())?;
            if let Some(record) = parse_entry(&session, seq as usize, &event, time / 1000) {
                sessions
                    .entry(session)
                    .or_default()
                    .insert(record.request_id.clone(), record);
            }
        }
        for (session, records) in sessions {
            result.merge(persist_snapshot(
                db,
                &format!("openclaw:sqlite:{session}"),
                records.into_values().collect(),
                states,
                claims,
                now,
            )?);
        }
        if !columns(&source, "session_transcript_archives")?.is_empty() {
            let mut query = source.prepare("SELECT session_id, generation, encoding, archive_blob, created_at FROM session_transcript_archives").map_err(|error| error.to_string())?;
            let rows = query
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (session, generation, encoding, blob, time) =
                    row.map_err(|error| error.to_string())?;
                let source_id = format!("openclaw:archive:{session}:{generation}");
                if states.get(&source_id).is_some_and(|state| !state.pending) {
                    continue;
                }
                let reader: Box<dyn BufRead> = match encoding.as_str() {
                    "identity" => Box::new(BufReader::new(Cursor::new(blob))),
                    "zstd" => Box::new(BufReader::new(
                        zstd::stream::read::Decoder::new(Cursor::new(blob))
                            .map_err(|error| error.to_string())?,
                    )),
                    _ => return Err(format!("Unsupported OpenClaw archive encoding: {encoding}")),
                };
                let mut records = BTreeMap::new();
                for (index, line) in reader.lines().enumerate() {
                    let event: Value =
                        serde_json::from_str(&line.map_err(|error| error.to_string())?)
                            .map_err(|error| error.to_string())?;
                    if let Some(record) = parse_entry(&session, index, &event, time / 1000) {
                        records.insert(record.request_id.clone(), record);
                    }
                }
                result.merge(persist_snapshot(
                    db,
                    &source_id,
                    records.into_values().collect(),
                    states,
                    claims,
                    now,
                )?);
            }
        }
    }
    Ok(result)
}
