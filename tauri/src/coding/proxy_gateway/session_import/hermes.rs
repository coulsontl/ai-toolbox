use super::parsers::native_record;
use super::{
    persist_records, GatewaySessionUsageImportResult, GatewayUsageTool, SessionUsageGranularity,
    SourceState, SqliteDbState, TokenUsage, SESSION_SETTLE_SECONDS,
};
use rusqlite::{Connection, OpenFlags};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Snapshot {
    tokens: [u64; 4],
    calls: u64,
    cost: String,
    last_seen: Option<i64>,
}

struct ModelUsage {
    session: String,
    model: String,
    provider: Option<String>,
    scope: String,
    first_seen: Option<i64>,
    snapshot: Snapshot,
    cost_source: String,
}

pub(super) fn open_read_only(path: &Path) -> Result<Connection, String> {
    let source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    source
        .busy_timeout(Duration::from_millis(200))
        .map_err(|error| error.to_string())?;
    source
        .execute_batch("PRAGMA query_only = ON; BEGIN DEFERRED")
        .map_err(|error| error.to_string())?;
    Ok(source)
}

pub(super) fn columns(source: &Connection, table: &str) -> Result<HashSet<String>, String> {
    // Table names are fixed adapter constants, never transcript values.
    let mut query = source
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let result = query
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| error.to_string());
    result
}

fn read_usage(source: &Connection, table: &str) -> Result<Vec<ModelUsage>, String> {
    let available = columns(source, table)?;
    if available.is_empty() {
        return Ok(Vec::new());
    }
    let field = |name: &str, fallback: &str| {
        if available.contains(name) {
            format!("COALESCE({name}, {fallback})")
        } else {
            fallback.to_string()
        }
    };
    let model_table = table == "session_model_usage";
    let session = if model_table { "session_id" } else { "id" };
    let model = if model_table {
        field("model", "'unknown'")
    } else {
        "'unknown'".into()
    };
    let first_seen = field(
        if model_table {
            "first_seen"
        } else {
            "started_at"
        },
        "NULL",
    );
    let last_seen = field(if model_table { "last_seen" } else { "ended_at" }, "NULL");
    let names = [
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "api_call_count",
        "actual_cost_usd",
        "estimated_cost_usd",
        "billing_provider",
        "billing_base_url",
        "billing_mode",
        "task",
        "cost_status",
    ];
    let expressions = names
        .iter()
        .enumerate()
        .map(|(index, name)| field(name, if index >= 7 { "''" } else { "0" }))
        .collect::<Vec<_>>();
    let sql = format!(
        "SELECT {session}, {model}, {first_seen}, {last_seen}, {} FROM {table}",
        expressions.join(", ")
    );
    let mut query = source.prepare(&sql).map_err(|error| error.to_string())?;
    let result = query
        .query_map([], |row| {
            let actual: f64 = row.get(9)?;
            let estimated: f64 = row.get(10)?;
            let status: String = row.get(15)?;
            let is_actual =
                actual > 0.0 || matches!(status.as_str(), "actual" | "reported" | "known");
            let provider: String = row.get(11)?;
            let route: String = row.get(12)?;
            let mode: String = row.get(13)?;
            let task: String = row.get(14)?;
            let scope = if model_table {
                format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::json!([provider, route, mode, task])
                            .to_string()
                            .as_bytes()
                    )
                )
            } else {
                "legacy".into()
            };
            Ok(ModelUsage {
                session: row.get(0)?,
                model: row.get(1)?,
                provider: (!provider.is_empty()).then_some(provider),
                scope,
                first_seen: row.get::<_, Option<f64>>(2)?.map(|value| value as i64),
                snapshot: Snapshot {
                    tokens: [
                        row.get::<_, i64>(4)?.max(0) as u64,
                        row.get::<_, i64>(5)?.max(0) as u64,
                        row.get::<_, i64>(6)?.max(0) as u64,
                        row.get::<_, i64>(7)?.max(0) as u64,
                    ],
                    calls: row.get::<_, i64>(8)?.max(0) as u64,
                    cost: Decimal::from_f64_retain(if is_actual { actual } else { estimated })
                        .unwrap_or_default()
                        .to_string(),
                    last_seen: row.get::<_, Option<f64>>(3)?.map(|value| value as i64),
                },
                cost_source: if is_actual {
                    "reported"
                } else {
                    "native_estimate"
                }
                .into(),
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string());
    result
}

pub(super) fn sync_database(
    db: &SqliteDbState,
    path: &Path,
    states: &mut HashMap<String, SourceState>,
    claims: &mut HashMap<String, String>,
    now: i64,
) -> Result<GatewaySessionUsageImportResult, String> {
    if !path.is_file() {
        return Ok(Default::default());
    }
    let source = open_read_only(path)?;
    let mut rows = read_usage(&source, "session_model_usage")?;
    rows.retain(|row| {
        row.snapshot.calls > 0
            || row.snapshot.tokens.iter().any(|value| *value > 0)
            || row.snapshot.cost.parse::<Decimal>().unwrap_or_default() != Decimal::ZERO
    });
    let mut continued_legacy = BTreeMap::<String, ModelUsage>::new();
    rows.retain(|row| {
        let old_source = format!("hermes:{}:unknown:legacy", row.session);
        if !states
            .get(&old_source)
            .is_some_and(|state| state.cumulative_usage.is_some())
        {
            return true;
        }
        // A pre-upgrade cumulative total has no historical model allocation.
        // Continue that session's total identity when richer rows appear;
        // importing every model's full history would charge the baseline again.
        let aggregate = continued_legacy
            .entry(row.session.clone())
            .or_insert_with(|| ModelUsage {
                session: row.session.clone(),
                model: "unknown".into(),
                provider: None,
                scope: "legacy".into(),
                first_seen: row.first_seen,
                snapshot: Snapshot {
                    cost: "0".into(),
                    ..Default::default()
                },
                cost_source: row.cost_source.clone(),
            });
        for index in 0..4 {
            aggregate.snapshot.tokens[index] += row.snapshot.tokens[index];
        }
        aggregate.snapshot.calls += row.snapshot.calls;
        aggregate.snapshot.cost = (aggregate
            .snapshot
            .cost
            .parse::<Decimal>()
            .unwrap_or_default()
            + row.snapshot.cost.parse::<Decimal>().unwrap_or_default())
        .to_string();
        aggregate.snapshot.last_seen = aggregate.snapshot.last_seen.max(row.snapshot.last_seen);
        aggregate.first_seen = aggregate.first_seen.min(row.first_seen);
        if aggregate.cost_source != row.cost_source {
            aggregate.cost_source = "native_estimate".into();
        }
        false
    });
    rows.extend(continued_legacy.into_values());
    let model_sessions = rows
        .iter()
        .map(|row| row.session.clone())
        .collect::<HashSet<_>>();
    rows.extend(
        read_usage(&source, "sessions")?
            .into_iter()
            .filter(|row| !model_sessions.contains(&row.session)),
    );
    let mut result = GatewaySessionUsageImportResult {
        scanned_files: 1,
        ..Default::default()
    };
    for mut row in rows {
        let source_id = format!("hermes:{}:{}:{}", row.session, row.model, row.scope);
        let mut state = states.get(&source_id).cloned().unwrap_or_default();
        let previous = state.cumulative_usage.clone().unwrap_or_default();
        if row
            .snapshot
            .last_seen
            .is_some_and(|time| time > now - SESSION_SETTLE_SECONDS)
        {
            continue;
        }
        let mut delta = [0; 4];
        for (index, value) in row.snapshot.tokens.iter_mut().enumerate() {
            *value = (*value).max(previous.tokens[index]);
            delta[index] = value.saturating_sub(previous.tokens[index]);
        }
        row.snapshot.calls = row.snapshot.calls.max(previous.calls);
        let calls = row.snapshot.calls - previous.calls;
        let cost = row.snapshot.cost.parse::<Decimal>().unwrap_or_default()
            - previous.cost.parse::<Decimal>().unwrap_or_default();
        if delta.iter().all(|value| *value == 0) && calls == 0 && cost.is_zero() {
            continue;
        }
        // Two models can have identical counters. Cost corrections can also
        // return to a previous snapshot without changing last_seen. Identify
        // each committed delta by its full scope and retained ledger sequence.
        let identity = format!(
            "delta:{:x}:{}",
            Sha256::digest(source_id.as_bytes()),
            state.records.len()
        );
        let mut record = native_record(
            GatewayUsageTool::Hermes,
            &row.session,
            &identity,
            Some(row.model),
            TokenUsage {
                input_tokens: Some(delta[0]),
                output_tokens: Some(delta[1]),
                cache_read_tokens: Some(delta[2]),
                cache_creation_tokens: Some(delta[3]),
                ..Default::default()
            },
            row.snapshot
                .last_seen
                .unwrap_or(now - SESSION_SETTLE_SECONDS),
        );
        record.metadata.granularity = SessionUsageGranularity::Session;
        record.metadata.call_count = Some(calls);
        record.metadata.native_provider = row.provider;
        record.metadata.window_start = previous.last_seen.or(row.first_seen);
        record.metadata.window_end = row.snapshot.last_seen;
        record.metadata.cost_source = Some(row.cost_source);
        // An estimate can later be replaced by a smaller actual cost. The
        // cumulative adjustment is signed; it must not charge both values.
        record.reported_cost_usd = Some(cost.to_string());
        state.cumulative_usage = Some(row.snapshot);
        let (state, changes) = persist_records(db, &source_id, state, vec![record], claims, now)?;
        states.insert(source_id, state);
        result.merge(changes);
    }
    Ok(result)
}
