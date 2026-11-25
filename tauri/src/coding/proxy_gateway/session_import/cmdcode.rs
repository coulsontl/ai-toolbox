//! Command Code session usage.
//!
//! Sessions live at `~/.commandcode/projects/<project>/<session-id>.jsonl`.
//! Two sibling files must be excluded: `hooks-audit-*.jsonl` and checkpoint
//! transcripts named `<session>.<n>.jsonl` (their stem contains a dot).
//!
//! Assistant rows carry `usage` either at the top level or under `message`,
//! with four independent buckets (`inputTokens` excludes the cache) and the
//! CLI's own `costUsd`. The same message can be copied into more than one
//! file, so the identity is a hash over its id/timestamp and the usage.

use super::parsers::{
    native_record, number, read_jsonl, session_duration_ms, string, timestamp, timestamp_millis,
    ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Session transcripts; excludes audit logs and checkpoint copies.
pub(super) fn is_session_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.ends_with(".jsonl") || name.starts_with("hooks-audit-") {
        return false;
    }
    let stem = name.trim_end_matches(".jsonl");
    !stem.contains('.')
}

fn cost(value: Option<&Value>) -> Option<String> {
    let value = value?.as_f64().filter(|value| value.is_finite())?;
    (0.0..=1_000_000.0)
        .contains(&value)
        .then(|| format!("{value:.6}"))
}

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    let mut session = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut model: Option<String> = None;
    let mut project_path: Option<String> = None;
    let mut seen_events = HashSet::<String>::new();
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let mut trigger_ms: Option<i64> = None;
    let pending = read_jsonl(path, |index, value| {
        match value.get("type").and_then(Value::as_str) {
            Some("session") => {
                if let Some(id) = string(&value, &["/id", "/sessionId"]) {
                    session = id;
                }
                if let Some(name) = string(&value, &["/model"]) {
                    model = Some(name);
                }
                if project_path.is_none() {
                    project_path = string(&value, &["/cwd", "/working_directory"]);
                }
                return;
            }
            Some("model_change") => {
                if let Some(name) = string(&value, &["/model"]) {
                    model = Some(name);
                }
                return;
            }
            _ => {}
        }
        let nested = value.get("message").filter(|value| value.is_object());
        let role = nested
            .and_then(|message| message.get("role"))
            .or_else(|| value.get("role"))
            .and_then(Value::as_str);
        if role == Some("user")
            || value.get("type").and_then(Value::as_str) == Some("user")
        {
            if let Some(at) = timestamp_millis(&value).or_else(|| nested.and_then(timestamp_millis))
            {
                trigger_ms = Some(at);
            }
            return;
        }
        if role != Some("assistant") {
            return;
        }
        let usage = value
            .get("usage")
            .filter(|usage| usage.is_object())
            .or_else(|| nested.and_then(|message| message.get("usage")))
            .filter(|usage| usage.is_object());
        let Some(usage) = usage else {
            return;
        };
        let input = number(usage, &["inputTokens"]);
        let output = number(usage, &["outputTokens"]);
        let cache_read = number(usage, &["cacheReadTokens"]);
        let cache_write = number(usage, &["cacheWriteTokens"]);
        let reported_cost = cost(usage.get("costUsd"));
        if input + output + cache_read + cache_write == 0 && reported_cost.is_none() {
            return;
        }
        let created_at = timestamp(&value)
            .or_else(|| nested.and_then(timestamp))
            .unwrap_or(fallback);
        let message_id = string(&value, &["/id", "/messageId"]).or_else(|| {
            nested.and_then(|message| string(message, &["/id", "/messageId"]))
        });
        // A message id plus its timestamp is stable across duplicated files; a
        // row without either still needs a per-file identity.
        let identity = match message_id {
            Some(id) => format!("{id}:{created_at}"),
            None => format!("line:{index}:{created_at}"),
        };
        let identity_hash = format!("{:x}", Sha256::digest(identity.as_bytes()));
        if !seen_events.insert(identity_hash.clone()) {
            return;
        }
        let mut record = native_record(
            GatewayUsageTool::Cmdcode,
            &session,
            &format!("event:{identity_hash}"),
            string(&value, &["/model"])
                .or_else(|| nested.and_then(|message| string(message, &["/model"])))
                .or_else(|| model.clone()),
            TokenUsage {
                input_tokens: Some(input),
                output_tokens: Some(output),
                cache_read_tokens: Some(cache_read),
                cache_creation_tokens: Some(cache_write),
                ..Default::default()
            },
            created_at,
        );
        record.duration_ms = match (
            trigger_ms.take(),
            timestamp_millis(&value).or_else(|| nested.and_then(timestamp_millis)),
        ) {
            (Some(start), Some(end)) => session_duration_ms(start, end),
            _ => None,
        };
        record.metadata.granularity = SessionUsageGranularity::Request;
        record.metadata.call_count = Some(1);
        record.project_path = project_path.clone();
        record.reported_cost_usd = reported_cost;
        if record.reported_cost_usd.is_some() {
            record.metadata.cost_source = Some("reported".into());
        }
        records.insert(record.request_id.clone(), record);
    })?;
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

