use super::{save_state, GatewayUsageTool, SessionUsageRecord, SourceState, SqliteDbState};
use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Exact saved contribution, also retained as the repair audit snapshot.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Contribution {
    date: String,
    model: String,
    tokens: [u64; 4],
    extra: u64,
    calls: u64,
    cost: String,
    pub archived: bool,
}

pub(super) fn read_contribution(
    conn: &Connection,
    request_id: &str,
) -> Result<Option<Contribution>, String> {
    conn.query_row(
        "SELECT date(created_at, 'unixepoch', 'localtime'), model, input_tokens, output_tokens,
                cache_read_tokens, cache_creation_tokens, extra_tokens, usage_request_count, total_cost_usd
         FROM proxy_request_logs WHERE request_id = ?1 AND data_source = 'session' AND app_type = 'claude_desktop'",
        [request_id], |row| Ok(Contribution {
            date: row.get(0)?, model: row.get(1)?,
            tokens: [row.get::<_, i64>(2)?.max(0) as u64, row.get::<_, i64>(3)?.max(0) as u64,
                row.get::<_, i64>(4)?.max(0) as u64, row.get::<_, i64>(5)?.max(0) as u64],
            extra: row.get::<_, i64>(6)?.max(0) as u64, calls: row.get::<_, i64>(7)?.max(0) as u64,
            cost: row.get(8)?, archived: false,
        }),
    ).optional().map_err(|error| error.to_string())
}

pub(super) fn mark_archived(conn: &Connection, cutoff: i64) -> Result<(), String> {
    let mut query = conn.prepare("SELECT request_id FROM proxy_request_logs WHERE data_source = 'session' AND app_type = 'claude_desktop' AND created_at < ?1").map_err(|error| error.to_string())?;
    let ids = query
        .query_map([cutoff], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| error.to_string())?;
    if ids.is_empty() {
        return Ok(());
    }
    let mut query = conn.prepare("SELECT id, json(data) FROM gateway_session_usage_state WHERE id LIKE 'claude_desktop:%'").map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (source_id, json) in rows {
        let mut state: SourceState =
            serde_json::from_str(&json).map_err(|error| error.to_string())?;
        let mut changed = false;
        for (id, record) in &mut state.records {
            if ids.contains(id) {
                if let Some(accounting) = &mut record.accounting {
                    accounting.archived = true;
                    changed = true;
                }
            }
        }
        if changed {
            save_state(conn, &source_id, &state)?;
        }
    }
    Ok(())
}

fn subtract(conn: &Connection, saved: &Contribution, require_exact: bool) -> Result<bool, String> {
    let existing = conn.query_row(
        "SELECT request_count, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, extra_tokens, total_cost_usd
         FROM usage_daily_rollups WHERE date = ?1 AND app_type = 'claude_desktop' AND provider_id = 'session' AND model = ?2",
        params![saved.date, saved.model], |row| Ok((row.get::<_, i64>(0)?,
            [row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?], row.get::<_, i64>(5)?, row.get::<_, String>(6)?)),
    ).optional().map_err(|error| error.to_string())?;
    let Some((calls, tokens, extra, cost)) = existing else {
        return Ok(false);
    };
    let cost = cost.parse::<Decimal>().map_err(|error| error.to_string())?;
    let removed_cost = saved
        .cost
        .parse::<Decimal>()
        .map_err(|error| error.to_string())?;
    let counts_fit = calls >= saved.calls as i64
        && extra >= saved.extra as i64
        && tokens
            .iter()
            .zip(saved.tokens)
            .all(|(have, remove)| *have >= remove as i64);
    let exact = calls == saved.calls as i64
        && extra == saved.extra as i64
        && tokens
            .iter()
            .zip(saved.tokens)
            .all(|(have, remove)| *have == remove as i64)
        && cost == removed_cost;
    if !counts_fit || cost + Decimal::new(1, 9) < removed_cost || (require_exact && !exact) {
        return Ok(false);
    }
    if exact {
        conn.execute("DELETE FROM usage_daily_rollups WHERE date = ?1 AND app_type = 'claude_desktop' AND provider_id = 'session' AND model = ?2", params![saved.date, saved.model])
            .map_err(|error| error.to_string())?;
    } else {
        conn.execute(
            "UPDATE usage_daily_rollups SET request_count = request_count - ?3, success_count = MAX(0, success_count - ?3),
                input_tokens = input_tokens - ?4, output_tokens = output_tokens - ?5,
                cache_read_tokens = cache_read_tokens - ?6, cache_creation_tokens = cache_creation_tokens - ?7,
                extra_tokens = extra_tokens - ?8, total_cost_usd = ?9
             WHERE date = ?1 AND app_type = 'claude_desktop' AND provider_id = 'session' AND model = ?2",
            params![saved.date, saved.model, saved.calls as i64, saved.tokens[0] as i64, saved.tokens[1] as i64,
                saved.tokens[2] as i64, saved.tokens[3] as i64, saved.extra as i64, (cost - removed_cost).max(Decimal::ZERO).to_string()],
        ).map_err(|error| error.to_string())?;
    }
    Ok(true)
}

pub(super) fn retire_desktop_records(
    db: &SqliteDbState,
    states: &mut HashMap<String, SourceState>,
    records: Vec<SessionUsageRecord>,
) -> Result<u64, String> {
    let candidates = records
        .into_iter()
        .filter(|record| record.cli_key == GatewayUsageTool::ClaudeDesktop)
        .map(|record| (record.request_id.clone(), record))
        .collect::<BTreeMap<_, _>>();
    let mut candidates = candidates
        .into_values()
        .filter_map(|record| {
            let old = states
                .values()
                .filter_map(|state| state.records.get(&record.request_id))
                .find(|record| !record.retired)?
                .clone();
            (old.accounting.is_some()
                || old.fingerprint == record.legacy_fingerprint()
                || old.fingerprint == record.fingerprint())
            .then_some((record, old))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(0);
    }
    let mut repaired = HashMap::<String, Contribution>::new();
    db.with_conn_mut(|conn| {
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        let mut legacy_groups = BTreeMap::<String, Vec<(String, Contribution)>>::new();
        for (record, previous) in candidates.drain(..) {
            if previous.matched_proxy_id.is_some() { continue; }
            if let Some(saved) = read_contribution(&transaction, &record.request_id)? {
                transaction.execute("DELETE FROM proxy_request_logs WHERE request_id = ?1 AND data_source = 'session'", [&record.request_id]).map_err(|error| error.to_string())?;
                repaired.insert(record.request_id, saved);
            } else if let Some(saved) = previous.accounting.filter(|saved| saved.archived) {
                if subtract(&transaction, &saved, false)? { repaired.insert(record.request_id, saved); }
            } else if record.model == "unknown" && record.reported_cost_usd.is_none() {
                // Old ledgers saved only fingerprints. Their unpriced unknown
                // bucket is repairable only when the entire bucket reconciles.
                let date: String = transaction.query_row("SELECT date(?1, 'unixepoch', 'localtime')", [record.created_at], |row| row.get(0)).map_err(|error| error.to_string())?;
                let saved = Contribution { date: date.clone(), model: "unknown".into(),
                    tokens: [record.usage.input_tokens.unwrap_or(0), record.usage.output_tokens.unwrap_or(0), record.usage.cache_read_tokens.unwrap_or(0), record.usage.cache_creation_tokens.unwrap_or(0)],
                    calls: 1, cost: "0".into(), archived: true, ..Default::default() };
                legacy_groups.entry(date).or_default().push((record.request_id, saved));
            }
        }
        for group in legacy_groups.into_values() {
            let mut total = group[0].1.clone();
            for (_, contribution) in group.iter().skip(1) {
                total.calls += contribution.calls;
                for index in 0..4 { total.tokens[index] += contribution.tokens[index]; }
            }
            if subtract(&transaction, &total, true)? { repaired.extend(group); }
        }
        for (source_id, state) in states.iter() {
            let mut updated = state.clone();
            let mut changed = false;
            for (id, record) in &mut updated.records {
                if let Some(saved) = repaired.get(id) {
                    record.retired = true;
                    record.accounting = Some(saved.clone());
                    changed = true;
                }
            }
            if changed { save_state(&transaction, source_id, &updated)?; }
        }
        transaction.commit().map_err(|error| error.to_string())
    })?;
    for state in states.values_mut() {
        for (id, record) in &mut state.records {
            if let Some(saved) = repaired.get(id) {
                record.retired = true;
                record.accounting = Some(saved.clone());
            }
        }
    }
    Ok(repaired.len() as u64)
}
