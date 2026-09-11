use super::{
    modified_nanos, parsers, save_state, session_files, source_identity, GatewayUsageTool,
    SessionUsageRecord, SourceState, SqliteDbState,
};
use crate::coding::proxy_gateway::usage_stats;
use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;

pub(super) fn supports(tool: GatewayUsageTool) -> bool {
    matches!(
        tool,
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop | GatewayUsageTool::Dsh
    )
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct RecordedCost {
    pub source: String,
    pub total_usd: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct CostRepair {
    pub previous_total_usd: String,
    pub repaired_total_usd: String,
}

pub(super) fn read_recorded_cost(
    conn: &Connection,
    request_id: &str,
) -> Result<Option<RecordedCost>, String> {
    conn.query_row(
        "SELECT total_cost_usd, json_extract(usage_metadata, '$.cost_source')
         FROM proxy_request_logs WHERE request_id = ?1 AND data_source = 'session' AND app_type IN ('claude','claude_desktop','dsh')",
        [request_id], |row| Ok(RecordedCost {
            total_usd: row.get(0)?, source: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        }),
    ).optional().map_err(|error| error.to_string())
}

struct Candidate {
    record: SessionUsageRecord,
    previous_cost: RecordedCost,
    replacement: Option<RecordedCost>,
}

struct SourceStamp {
    source_id: String,
    path: PathBuf,
    size: u64,
    modified: u64,
}

fn source_fingerprint(
    files: &[SourceStamp],
    relevant: &BTreeSet<String>,
    imported: &BTreeMap<&String, &SourceState>,
    prices: &[[String; 5]],
) -> Result<String, String> {
    let mut signature = Sha256::new();
    signature.update(b"native-cost-reconciliation-v2");
    signature.update(serde_json::to_vec(prices).map_err(|error| error.to_string())?);
    for file in files {
        // Inventory changes discover restored or newly imported history. Only
        // files with unresolved costs need content-change invalidation; normal
        // traffic must not reparse every historical Claude transcript.
        let stamp = relevant
            .contains(&file.source_id)
            .then_some((file.size, file.modified));
        signature.update(
            serde_json::to_vec(&(file.path.to_string_lossy(), stamp))
                .map_err(|error| error.to_string())?,
        );
    }
    for (source_id, state) in imported
        .iter()
        .filter(|(id, _)| relevant.contains(id.as_str()))
    {
        for (id, record) in &state.records {
            signature.update(
                serde_json::to_vec(&(
                    source_id,
                    id,
                    &record.fingerprint,
                    &record.envelope_id,
                    &record.matched_proxy_id,
                    record.retired,
                ))
                .map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(format!("{:x}", signature.finalize()))
}

fn amount(value: &str) -> Result<Decimal, String> {
    value
        .parse()
        .map_err(|error| format!("Invalid recorded session cost: {error}"))
}

fn replacement_cost(
    conn: &Connection,
    record: &SessionUsageRecord,
    previous: &RecordedCost,
) -> Option<RecordedCost> {
    if previous.source != "unavailable"
        || amount(&previous.total_usd).ok()? != Decimal::ZERO
        || !usage_stats::session_model_has_pricing(conn, &record.model)
    {
        return None;
    }
    let cost = usage_stats::calculate_session_costs(
        conn,
        &record.model,
        record.usage.input_tokens.unwrap_or(0),
        record.usage.output_tokens.unwrap_or(0),
        record.usage.cache_read_tokens.unwrap_or(0),
        record.usage.cache_creation_tokens.unwrap_or(0),
    );
    Some(RecordedCost {
        source: "model_pricing".into(),
        total_usd: usage_stats::format_decimal_cost(cost.total()),
    })
}

/// Retry missing estimates without reimporting usage or repricing known costs.
/// Old archived rows are repaired only when the complete tool/day/model bucket
/// is proven by native IDs, saved fingerprints, counters and its previous cost.
pub(super) fn reconcile_session_costs(
    db: &SqliteDbState,
    cli_key: GatewayUsageTool,
    sources: &[(GatewayUsageTool, PathBuf)],
    states: &mut HashMap<String, SourceState>,
) -> Result<u64, String> {
    let source_prefix = format!("{}:", cli_key.as_str());
    let cache_id = format!("cost-reconciliation:{}", cli_key.as_str());
    let mut files = sources
        .iter()
        .filter(|(tool, _)| *tool == cli_key)
        .flat_map(|(tool, root)| {
            session_files(*tool, root)
                .into_iter()
                .map(move |path| (*tool, path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| {
        left.0
            .as_str()
            .cmp(right.0.as_str())
            .then_with(|| left.1.cmp(&right.1))
    });
    files.dedup();
    if files.is_empty() {
        return Ok(0);
    }
    let files = files
        .into_iter()
        .map(|(tool, path)| {
            let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
            Ok(SourceStamp {
                source_id: format!("{}:{}", tool.as_str(), source_identity(tool, &path)),
                path,
                size: metadata.len(),
                modified: modified_nanos(&metadata),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let imported = states
        .iter()
        .filter(|(id, _)| id.starts_with(&source_prefix) && id.as_str() != cache_id)
        .collect::<BTreeMap<_, _>>();
    // A failed import has no committed usage to price. Do not turn its one
    // ledger failure into a second error while trying to save an empty cache.
    if imported.values().all(|state| state.records.is_empty()) {
        return Ok(0);
    }
    let prices = db.with_conn(|conn| {
        let mut query = conn.prepare("SELECT model_id, input_cost_per_million, output_cost_per_million, cache_read_cost_per_million, cache_creation_cost_per_million FROM model_pricing ORDER BY model_id").map_err(|error| error.to_string())?;
        let rows = query.query_map([], |row| Ok([row.get::<_, String>(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?])).map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
    })?;
    let old_relevant = states
        .get(&cache_id)
        .map(|state| state.cost_reconciliation_sources.iter().cloned().collect())
        .unwrap_or_default();
    let fingerprint = source_fingerprint(&files, &old_relevant, &imported, &prices)?;
    if states
        .get(&cache_id)
        .and_then(|state| state.cost_reconciliation_fingerprint.as_ref())
        == Some(&fingerprint)
    {
        return Ok(0);
    }
    let mut ledger = BTreeMap::<&String, Vec<&super::ImportedRecord>>::new();
    for state in imported.values() {
        for (id, record) in &state.records {
            ledger.entry(id).or_default().push(record);
        }
    }
    let mut relevant = BTreeSet::new();
    let mut price_matches = HashMap::new();
    let mut native = BTreeMap::new();
    for file in &files {
        let parsed =
            parsers::parse_file(cli_key, &file.path, (file.modified / 1_000_000_000) as i64)?;
        // Desktop audit fallbacks intentionally remain pending to discover a
        // later transcript. Their final result records can still be priced.
        if parsed.pending && cli_key != GatewayUsageTool::ClaudeDesktop {
            return Ok(0);
        }
        let present = parsed
            .records
            .iter()
            .map(|record| record.request_id.as_str())
            .collect::<BTreeSet<_>>();
        if states.get(&file.source_id).is_some_and(|state| {
            state.records.iter().any(|(id, previous)| {
                !previous.retired
                    && previous.matched_proxy_id.is_none()
                    && !present.contains(id.as_str())
                    && previous
                        .recorded_cost
                        .as_ref()
                        .is_none_or(|cost| cost.source == "unavailable")
            })
        }) {
            relevant.insert(file.source_id.clone());
        }
        for record in parsed.records {
            if let Some(previous) = ledger.get(&record.request_id) {
                if previous.iter().any(|previous| {
                    !previous.retired
                        && previous.matched_proxy_id.is_none()
                        && previous
                            .recorded_cost
                            .as_ref()
                            .is_none_or(|cost| cost.source == "unavailable")
                }) {
                    if !price_matches.contains_key(&record.model) {
                        let matched = db.with_conn(|conn| {
                            Ok((
                                usage_stats::session_model_has_pricing(conn, &record.model),
                                usage_stats::session_model_needs_short_date_fallback(
                                    conn,
                                    &record.model,
                                ),
                            ))
                        })?;
                        price_matches.insert(record.model.clone(), matched);
                    }
                    let (has_price, legacy_missing) = price_matches[&record.model];
                    if previous.iter().any(|previous| {
                        previous.recorded_cost.as_ref().map_or(
                            legacy_missing && record.reported_cost_usd.is_none(),
                            |cost| cost.source == "unavailable" && has_price,
                        )
                    }) {
                        relevant.insert(file.source_id.clone());
                    }
                }
            }
            native.insert(record.request_id.clone(), record);
        }
    }
    let fingerprint = source_fingerprint(&files, &relevant, &imported, &prices)?;
    let mut committed_states = HashMap::new();
    let mut repaired = HashMap::<String, (RecordedCost, CostRepair)>::new();
    db.with_conn_mut(|conn| {
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        let mut archives = BTreeMap::<(String, String, String), Vec<Candidate>>::new();
        for record in native.into_values() {
            let fingerprint = record.fingerprint();
            let legacy_fingerprint = record.legacy_fingerprint();
            let previous = ledger.get(&record.request_id).into_iter().flatten()
                .filter(|previous| !previous.retired && previous.matched_proxy_id.is_none()
                    && (previous.fingerprint == fingerprint || previous.fingerprint == legacy_fingerprint)
                    && (previous.envelope_id.is_none() || record.usage.envelope_id.is_none() || previous.envelope_id == record.usage.envelope_id))
                .max_by_key(|previous| previous.recorded_cost.is_some());
            let Some(previous) = previous else { continue; };
            if let Some(mut cost) = read_recorded_cost(&transaction, &record.request_id)? {
                if cost.source.is_empty() {
                    cost.source = previous.recorded_cost.as_ref().map(|known| known.source.clone()).unwrap_or_default();
                }
                if cost.source.is_empty() && record.reported_cost_usd.is_none()
                    && usage_stats::session_model_needs_short_date_fallback(&transaction, &record.model) {
                    cost.source = "unavailable".into();
                }
                if let Some(replacement) = replacement_cost(&transaction, &record, &cost) {
                    let breakdown = usage_stats::calculate_session_costs(&transaction, &record.model,
                        record.usage.input_tokens.unwrap_or(0), record.usage.output_tokens.unwrap_or(0),
                        record.usage.cache_read_tokens.unwrap_or(0), record.usage.cache_creation_tokens.unwrap_or(0));
                    transaction.execute(
                        "UPDATE proxy_request_logs SET input_cost_usd=?2, output_cost_usd=?3,
                         cache_read_cost_usd=?4, cache_creation_cost_usd=?5, total_cost_usd=?6,
                         usage_metadata=jsonb_set(COALESCE(usage_metadata, jsonb('{}')), '$.cost_source', 'model_pricing')
                         WHERE request_id=?1 AND data_source='session'",
                        params![record.request_id, usage_stats::format_decimal_cost(breakdown.input_cost_usd),
                            usage_stats::format_decimal_cost(breakdown.output_cost_usd), usage_stats::format_decimal_cost(breakdown.cache_read_cost_usd),
                            usage_stats::format_decimal_cost(breakdown.cache_creation_cost_usd), replacement.total_usd],
                    ).map_err(|error| error.to_string())?;
                    repaired.insert(record.request_id, (replacement.clone(), CostRepair {
                        previous_total_usd: cost.total_usd, repaired_total_usd: replacement.total_usd,
                    }));
                }
                continue;
            }
            let cost = previous.recorded_cost.clone().or_else(|| {
                // Legacy ledgers have no price provenance. Limit inference to
                // the proven MMDD matching defect, whose old resolver fails.
                (record.reported_cost_usd.is_none() && usage_stats::session_model_needs_short_date_fallback(&transaction, &record.model))
                    .then(|| RecordedCost { source: "unavailable".into(), total_usd: "0".into() })
            });
            let Some(cost) = cost else { continue; };
            let date = transaction.query_row("SELECT date(?1, 'unixepoch', 'localtime')", [record.created_at], |row| row.get::<_, String>(0)).map_err(|error| error.to_string())?;
            let replacement = replacement_cost(&transaction, &record, &cost);
            archives.entry((record.cli_key.as_str().to_string(), date, record.model.clone())).or_default().push(Candidate { record, previous_cost: cost, replacement });
        }
        for ((tool, date, model), candidates) in archives {
            if !candidates.iter().any(|candidate| candidate.replacement.is_some()) { continue; }
            let existing = transaction.query_row(
                "SELECT request_count,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,extra_tokens,total_cost_usd
                 FROM usage_daily_rollups WHERE date=?1 AND app_type=?3 AND provider_id='session' AND model=?2",
                params![date,model,tool], |row| Ok(([row.get::<_, i64>(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?], row.get::<_, String>(6)?)),
            ).optional().map_err(|error| error.to_string())?;
            let Some((counters, old_total)) = existing else { continue; };
            let mut expected = [0i64;6];
            let mut expected_cost = Decimal::ZERO;
            let mut repaired_cost = Decimal::ZERO;
            for candidate in &candidates {
                let record = &candidate.record;
                for (counter, value) in expected.iter_mut().zip([record.call_count(), record.usage.input_tokens.unwrap_or(0),
                    record.usage.output_tokens.unwrap_or(0), record.usage.cache_read_tokens.unwrap_or(0),
                    record.usage.cache_creation_tokens.unwrap_or(0), record.extra_tokens()]) { *counter += value as i64; }
                expected_cost += amount(&candidate.previous_cost.total_usd)?;
                repaired_cost += amount(&candidate.replacement.as_ref().unwrap_or(&candidate.previous_cost).total_usd)?;
            }
            // Rollups use SQLite REAL sums; compare at the stored USD scale.
            if counters != expected || amount(&old_total)?.round_dp(6) != expected_cost.round_dp(6) { continue; }
            transaction.execute("UPDATE usage_daily_rollups SET total_cost_usd=?3 WHERE date=?1 AND app_type=?4 AND provider_id='session' AND model=?2",
                params![date,model,usage_stats::format_decimal_cost(repaired_cost),tool]).map_err(|error| error.to_string())?;
            for candidate in candidates {
                if let Some(replacement) = candidate.replacement {
                    repaired.insert(candidate.record.request_id, (replacement.clone(), CostRepair {
                        previous_total_usd: candidate.previous_cost.total_usd, repaired_total_usd: replacement.total_usd,
                    }));
                }
            }
        }
        for (source_id,state) in states.iter().filter(|(id,_)| id.starts_with(&source_prefix)) {
            if !state.records.keys().any(|id| repaired.contains_key(id)) { continue; }
            let mut updated = state.clone();
            for (id, record) in &mut updated.records {
                if let Some((cost, repair)) = repaired.get(id) {
                    record.recorded_cost = Some(cost.clone());
                    record.cost_repair = Some(repair.clone());
                    if let Some(accounting) = &mut record.accounting { accounting.set_cost(&cost.total_usd); }
                }
            }
            save_state(&transaction, source_id, &updated)?;
            committed_states.insert(source_id.clone(), updated);
        }
        let cache = SourceState { cost_reconciliation_fingerprint: Some(fingerprint), cost_reconciliation_sources: relevant.into_iter().collect(), ..Default::default() };
        save_state(&transaction, &cache_id, &cache)?;
        committed_states.insert(cache_id, cache);
        transaction.commit().map_err(|error| error.to_string())
    })?;
    states.extend(committed_states);
    Ok(repaired.len() as u64)
}
