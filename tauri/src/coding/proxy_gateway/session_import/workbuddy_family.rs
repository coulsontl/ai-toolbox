//! WorkBuddy and WorkBuddy international transcripts.
//!
//! Both editions write Claude-Code-shaped JSONL under
//! `~/.workbuddy{,-ai}/projects/**`. Each item carrying usage is one model
//! call. Three usage sources can appear on one item and they describe the same
//! call — `message.usage`, `providerData.usage` and `providerData.rawUsage` —
//! so one is selected (the first with non-zero input/output) and the cache
//! fields are read from whichever source carries them.
//!
//! `total_tokens` sometimes counts the prompt *inclusively* (`total == input
//! + output`), which marks `input` as containing the cached part. Only then is
//! the cache subtracted; an exclusive total leaves the buckets as they are.

use super::parsers::{
    native_record, reported_cost, session_duration_ms, timestamp, timestamp_millis, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageGranularity, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Reads one integer, tolerating numbers stored as strings.
fn count(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        if value.is_boolean() {
            return None;
        }
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|value| value.max(0.0) as u64))
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn detail_count(value: &Value, keys: &[&str]) -> u64 {
    match value {
        Value::Object(_) => count(value, keys).unwrap_or(0),
        Value::Array(items) => items
            .iter()
            .filter(|item| item.is_object())
            .map(|item| count(item, keys).unwrap_or(0))
            .sum(),
        _ => 0,
    }
}

/// The token/cost facts of one item, before de-duplication.
pub(super) struct UsageRecord {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub model: String,
    pub created_at: i64,
    pub session: String,
    pub identity: String,
    pub reported_cost: Option<String>,
}

/// Reads `message.usage`, `providerData.usage` and `providerData.rawUsage` from
/// one item. Returns `None` when none of them carries usage.
///
/// `codebuddy` also accepts a Credit-only row and keys de-duplication by the
/// provider's message id, matching the CLI's own accounting.
pub(super) fn usage_record(
    item: &Value,
    session: &str,
    line: usize,
    fallback: i64,
) -> Option<UsageRecord> {
    let message = item.get("message").filter(|value| value.is_object());
    let provider = item
        .get("providerData")
        .or_else(|| message.and_then(|message| message.get("providerData")))
        .filter(|value| value.is_object());
    let sources = [
        message.and_then(|message| message.get("usage")),
        provider.and_then(|provider| provider.get("usage")),
        provider.and_then(|provider| provider.get("rawUsage")),
    ]
    .into_iter()
    .flatten()
    .filter(|value| value.is_object() && value.as_object().is_some_and(|fields| !fields.is_empty()))
    .collect::<Vec<_>>();

    let credits = sources
        .iter()
        .filter_map(|source| {
            count(source, &["credit", "credits"]).map(|value| value as f64)
        })
        .fold(0.0f64, f64::max);

    let selected = sources.iter().find(|source| {
        let input = count(source, &["input_tokens", "inputTokens", "input", "prompt_tokens"])
            .unwrap_or(0);
        let output = count(
            source,
            &["output_tokens", "outputTokens", "output", "completion_tokens"],
        )
        .unwrap_or(0);
        input + output > 0
    });
    if selected.is_none() && credits <= 0.0 {
        return None;
    }
    let selected = selected.copied().or_else(|| sources.first().copied())?;
    let input_total = count(selected, &["input_tokens", "inputTokens", "input", "prompt_tokens"])
        .unwrap_or(0);
    let output = count(
        selected,
        &["output_tokens", "outputTokens", "output", "completion_tokens"],
    )
    .unwrap_or(0);

    let cache_read = sources
        .iter()
        .flat_map(|source| {
            [
                count(
                    source,
                    &[
                        "cache_read_input_tokens",
                        "cacheReadInputTokens",
                        "cache_read",
                        "cacheRead",
                        "cached_tokens",
                        "cachedTokens",
                    ],
                )
                .unwrap_or(0),
                count(source, &["prompt_cache_hit_tokens"]).unwrap_or(0),
                detail_count(
                    source.get("inputTokensDetails").unwrap_or(&Value::Null),
                    &["cached_tokens", "cachedTokens"],
                ),
                detail_count(
                    source.get("input_tokens_details").unwrap_or(&Value::Null),
                    &["cached_tokens", "cachedTokens"],
                ),
                detail_count(
                    source.get("prompt_tokens_details").unwrap_or(&Value::Null),
                    &["cached_tokens", "cachedTokens"],
                ),
            ]
        })
        .max()
        .unwrap_or(0);
    let cache_write = sources
        .iter()
        .filter_map(|source| {
            count(
                source,
                &[
                    "cache_creation_input_tokens",
                    "cacheCreationInputTokens",
                    "cache_write_input_tokens",
                    "cacheWriteInputTokens",
                    "prompt_cache_write_tokens",
                    "cache_write",
                    "cacheWrite",
                ],
            )
        })
        .max()
        .unwrap_or(0);
    // `total == input + output` means the total already contains the cache and
    // the input bucket is inclusive; subtract once and never twice.
    let inclusive = sources.iter().any(|source| {
        count(source, &["total_tokens", "totalTokens", "total"])
            .is_some_and(|total| total == input_total + output)
    });
    let (input, cache_read, cache_write) = if inclusive {
        let cache_read = cache_read.min(input_total);
        let cache_write = cache_write.min(input_total.saturating_sub(cache_read));
        (
            input_total.saturating_sub(cache_read).saturating_sub(cache_write),
            cache_read,
            cache_write,
        )
    } else {
        (input_total, cache_read, cache_write)
    };

    // `timestamp` scans *containers* (`pointer("/timestamp")` etc.), so it takes
    // `item` itself — passing `item.get("timestamp")` would hand it a bare
    // scalar and always miss. `message` is a second container to fall back on
    // because some builds stamp the timestamp on the nested message.
    let created_at = timestamp(item)
        .or_else(|| message.and_then(timestamp))
        .unwrap_or(fallback);
    let model = provider
        .and_then(|provider| {
            ["requestModelName", "requestModelId", "model"]
                .iter()
                .find_map(|key| provider.get(*key).and_then(Value::as_str))
        })
        .or_else(|| message.and_then(|message| message.get("model")).and_then(Value::as_str))
        .or_else(|| item.get("model").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();
    let message_id = provider
        .and_then(|provider| provider.get("messageId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let identity = message_id
        .as_ref()
        .map(|id| format!("message:{id}"))
        .unwrap_or_else(|| format!("line:{line}"));
    Some(UsageRecord {
        input,
        output,
        cache_read,
        cache_write,
        model,
        created_at,
        session: session.to_string(),
        identity,
        reported_cost: reported_cost(
            selected
                .get("cost")
                .or_else(|| selected.get("costUsd"))
                .or_else(|| selected.get("cost_usd")),
        ),
    })
}

/// The working directory WorkBuddy/CodeBuddy write on each item. Some builds
/// put it on the nested message instead, so both spellings are accepted.
fn project_of(item: &Value) -> Option<String> {
    super::parsers::string(item, &["/cwd", "/message/cwd", "/working_directory"])
}

/// WorkBuddy keeps its session id in `sessionId`, its project in `cwd`.
fn session_of(item: &Value, fallback: &str) -> String {
    item.get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub(super) fn parse(tool: GatewayUsageTool, path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    let mut session_fallback = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut items = Vec::new();
    let pending = super::parsers::read_jsonl(path, |line, value| {
        if value.get("type").and_then(Value::as_str) == Some("session") {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                session_fallback = id.to_string();
            }
        }
        items.push((line, value));
    })?;
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let mut session_project: Option<String> = None;
    // The tool result (or user turn) that sent the request; the next usage
    // item is written when the reply lands, so the pair brackets the call.
    let mut trigger_ms: Option<i64> = None;
    for (line, item) in items {
        let session = session_of(&item, &session_fallback);
        if item.get("type").and_then(Value::as_str) == Some("function_call_result")
            || (item.get("type").and_then(Value::as_str) == Some("message")
                && item.pointer("/message/role").and_then(Value::as_str) == Some("user"))
        {
            if let Some(at) = timestamp_millis(&item) {
                trigger_ms = Some(at);
            }
        }
        let Some(usage) = usage_record(&item, &session, line, fallback) else {
            continue;
        };
        let duration_ms = match (trigger_ms, timestamp_millis(&item)) {
            (Some(start), Some(end)) => session_duration_ms(start, end),
            _ => None,
        };
        trigger_ms = None;
        let mut record = native_record(
            tool,
            &usage.session,
            &usage.identity,
            Some(usage.model),
            TokenUsage {
                input_tokens: Some(usage.input),
                output_tokens: Some(usage.output),
                cache_read_tokens: Some(usage.cache_read),
                cache_creation_tokens: Some(usage.cache_write),
                ..Default::default()
            },
            usage.created_at,
        );
        record.duration_ms = duration_ms;
        record.metadata.granularity = SessionUsageGranularity::Request;
        record.metadata.call_count = Some(1);
        // The item that carries the usage often omits `cwd`; the session
        // header line and other items still carry the project, so a project
        // already seen in this file fills the gap.
        if let Some(project) = project_of(&item).or_else(|| session_project.clone()) {
            session_project = Some(project.clone());
            record.project_path = Some(project);
        }
        record.reported_cost_usd = usage.reported_cost;
        if record.reported_cost_usd.is_some() {
            record.metadata.cost_source = Some("reported".into());
        }
        // A duplicated message id is one call; the larger counters win, as the
        // CLI can rewrite an item with more complete usage.
        match records.get(&record.request_id) {
            Some(existing)
                if existing.usage.total_tokens().unwrap_or(0)
                    >= record.usage.total_tokens().unwrap_or(0) => {}
            _ => {
                records.insert(record.request_id.clone(), record);
            }
        }
    }
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

