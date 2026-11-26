use super::parsers::{
    native_record, number, read_jsonl, session_duration_ms, session_first_token_ms, string,
    timestamp, timestamp_millis, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

// The artifact naming rule (which file is a session, which generation is live)
// is a dsh on-disk layout fact, so it lives in the dsh module and is shared
// with the session browser rather than duplicated here.
pub(super) use crate::coding::dsh::session_artifact::{generation, select_generations};

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    let mut session = path
        .parent()
        .and_then(Path::file_name)
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let mut version = generation(path).unwrap_or(0);
    let mut has_parent = false;
    let mut seed_boundary_seen = false;
    let mut last_settlement: Option<((u64, u64), String)> = None;
    let mut legacy_retries = HashMap::<(u64, u64), u64>::new();
    let mut project_path: Option<String> = None;
    // `step/start` brackets each individual model call keyed by `(turn, step)`.
    // Older logs also emit `turn/start`, `request/header` or `llm/retry-started`,
    // kept as a per-step and fallback start time.
    let mut step_starts = HashMap::<(u64, u64), i64>::new();
    let mut fallback_started_ms: Option<i64> = None;
    let pending = read_jsonl(path, |index, event| {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "session" {
            if let Some(id) = string(&event, &["/id"]) {
                session = id;
            }
            project_path = project_path
                .take()
                .or_else(|| string(&event, &["/cwd", "/working_directory"]));
            version = event
                .get("version")
                .and_then(Value::as_u64)
                .unwrap_or(u64::from(version)) as u32;
            has_parent = event
                .get("parentSession")
                .is_some_and(|value| !value.is_null());
            return;
        }
        if kind == "session/end-seed" {
            // Untagged v0 markers also appear on ordinary resume. They do not
            // undo paid history. Only a tagged inherited cut (or the first
            // legacy parent seed) proves that the prefix belongs to a parent.
            if event.pointer("/data/inherited").and_then(Value::as_bool) == Some(true)
                || (version < 2 && has_parent && !seed_boundary_seen)
            {
                records.clear();
            }
            seed_boundary_seen = true;
            last_settlement = None;
            legacy_retries.clear();
            step_starts.clear();
            fallback_started_ms = None;
            return;
        }
        let Some(data) = event.get("data") else {
            return;
        };
        let step = (number(data, &["turn"]), number(data, &["step"]));
        if matches!(
            kind,
            "step/start" | "llm/retry-started" | "turn/start" | "request/header"
        ) {
            if kind != "step/start" {
                last_settlement = None;
            }
            if let Some(at) = timestamp_millis(&event) {
                if data.get("turn").is_some() && data.get("step").is_some() {
                    step_starts.insert(step, at);
                }
                fallback_started_ms = Some(at);
            }
            if kind == "llm/retry-started" {
                *legacy_retries.entry(step).or_default() += 1;
            }
            return;
        }
        if !matches!(
            kind,
            "assistant/message" | "assistant/attempt" | "compaction/summary"
        ) {
            return;
        }
        // Surface replacement records replay old messages after compaction.
        if kind == "assistant/message"
            && event
                .get("surfaceOp")
                .is_some_and(|op| op.as_str() != Some("append"))
        {
            return;
        }
        let raw = data.get("usage").or_else(|| {
            data.get("stream")?
                .as_array()?
                .iter()
                .rev()
                .find_map(|chunk| {
                    (chunk.get("type").and_then(Value::as_str) == Some("usage"))
                        .then(|| chunk.get("usage"))
                        .flatten()
                })
        });
        let Some(raw) = raw.filter(|usage| usage.is_object()) else {
            return;
        };
        let usage = TokenUsage {
            input_tokens: Some(number(raw, &["inputTokens"])),
            output_tokens: Some(number(raw, &["outputTokens"])),
            cache_read_tokens: Some(number(raw, &["cacheReadTokens"])),
            cache_creation_tokens: Some(number(raw, &["cacheWriteTokens"])),
            cache_creation_1h_tokens: None,
            envelope_id: string(
                data,
                &[
                    "/message/source/replayState/responseId",
                    "/source/replayState/responseId",
                ],
            ),
        };
        let identity = if kind == "compaction/summary" {
            format!(
                "compaction:{}",
                string(data, &["/compactionId"]).unwrap_or_else(|| event
                    .get("seq")
                    .and_then(Value::as_u64)
                    .unwrap_or(index as u64)
                    .to_string())
            )
        } else {
            // v0 turn/step values restart during resumed runs. Message IDs
            // remain distinct. v2+ replaces only consecutive settlements of
            // one attempt, never every historical occurrence of a turn/step.
            let fresh = string(data, &["/message/id", "/id"]).unwrap_or_else(|| {
                format!(
                    "event-{}",
                    event
                        .get("seq")
                        .and_then(Value::as_u64)
                        .unwrap_or(index as u64)
                )
            });
            let identity = if version >= 2 {
                last_settlement
                    .as_ref()
                    .filter(|(previous, _)| *previous == step)
                    .map(|(_, identity)| identity.clone())
                    .unwrap_or(fresh)
            } else {
                fresh
            };
            last_settlement = Some((step, identity.clone()));
            identity
        };
        let mut record = native_record(
            GatewayUsageTool::Dsh,
            &session,
            &identity,
            string(data, &["/message/source/model", "/source/model", "/model"]),
            usage,
            timestamp(&event).unwrap_or(fallback),
        );
        // A failed attempt is followed by `llm/retry-started`, so the next
        // start replaces this one; only a settled message consumes the bracket.
        let started_ms = step_starts.get(&step).copied().or(fallback_started_ms);
        record.duration_ms = match (started_ms, timestamp_millis(&event)) {
            (Some(start), Some(end)) => session_duration_ms(start, end),
            _ => None,
        };
        record.first_token_ms = match (
            started_ms,
            first_stream_output_ms(data.get("stream")),
            record.duration_ms,
        ) {
            (Some(start), Some(first_output), Some(duration)) => {
                session_first_token_ms(start, first_output, duration)
            }
            _ => None,
        };
        if kind == "assistant/message" {
            step_starts.remove(&step);
            fallback_started_ms = None;
        }
        record.metadata.native_provider = string(
            data,
            &["/message/source/provider", "/source/provider", "/provider"],
        );
        record.project_path = project_path.clone();
        record.metadata.incomplete = kind == "assistant/attempt";
        // A running development build may already have archived the earlier
        // turn/step identities. Preserve only fingerprint-proven aliases so
        // the parser revision backfills missing history without charging the
        // previously imported suffix again.
        let legacy_identity = if kind == "compaction/summary" {
            format!(
                "compaction:{}",
                event
                    .get("seq")
                    .and_then(Value::as_u64)
                    .unwrap_or(index as u64)
            )
        } else {
            format!(
                "turn-{}-step-{}-attempt-{}",
                step.0,
                step.1,
                legacy_retries.get(&step).copied().unwrap_or(0)
            )
        };
        let legacy_id = format!("SESSION:dsh:{session}:{legacy_identity}");
        if legacy_id != record.request_id {
            record.legacy_request_ids.push(legacy_id);
        }
        records.insert(record.request_id.clone(), record);
    })?;
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

/// Earliest chunk timestamp in DSH's `data.stream` array (`time` for single
/// chunks, `time0` for batched chunks).
fn first_stream_output_ms(stream: Option<&Value>) -> Option<i64> {
    stream?
        .as_array()?
        .iter()
        .flat_map(|entry| [entry.get("time"), entry.get("time0")])
        .flatten()
        .filter_map(Value::as_i64)
        .min()
}
