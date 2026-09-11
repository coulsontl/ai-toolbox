use super::parsers::{native_record, number, read_jsonl, string, timestamp, ParsedSession};
use super::{GatewayUsageTool, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub(super) fn generation(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let name = name.strip_suffix(".zstd").unwrap_or(name);
    if name == "session.jsonl" {
        return Some(0);
    }
    let version = name.strip_prefix("session.v")?.strip_suffix(".jsonl")?;
    if version.starts_with('0') {
        return None;
    }
    version.parse().ok()
}

pub(super) fn select_generations(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut selected = BTreeMap::<PathBuf, PathBuf>::new();
    for file in files {
        let directory = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let replace = selected.get(&directory).is_none_or(|old| {
            (generation(&file), file.extension()) > (generation(old), old.extension())
        });
        if replace {
            selected.insert(directory, file);
        }
    }
    selected.into_values().collect()
}

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
    let pending = read_jsonl(path, |index, event| {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "session" {
            if let Some(id) = string(&event, &["/id"]) {
                session = id;
            }
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
            return;
        }
        let Some(data) = event.get("data") else {
            return;
        };
        let step = (number(data, &["turn"]), number(data, &["step"]));
        if matches!(kind, "llm/retry-started" | "turn/start" | "request/header") {
            last_settlement = None;
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
        record.metadata.native_provider = string(
            data,
            &["/message/source/provider", "/source/provider", "/provider"],
        );
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
