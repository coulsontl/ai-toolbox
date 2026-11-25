use super::parsers::{
    bounded_session_first_token_ms, json_millis, native_record, native_session_duration_ms,
    number, read_jsonl, reported_cost, session_duration_ms, string, timestamp, timestamp_millis,
    ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub(super) fn parse(
    tool: GatewayUsageTool,
    path: &Path,
    fallback: i64,
) -> Result<ParsedSession, String> {
    parse_inner(tool, path, fallback, &mut HashSet::new())
}

fn parse_inner(
    tool: GatewayUsageTool,
    path: &Path,
    fallback: i64,
    ancestors: &mut HashSet<PathBuf>,
) -> Result<ParsedSession, String> {
    if !ancestors.insert(path.to_path_buf()) || ancestors.len() > 32 {
        return Err("Cyclic Pi session ancestry".into());
    }
    let mut session = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut parent = None;
    let mut started_at = None;
    // Pi/OMP/Prime-style transcripts record the working directory on the
    // session header line.
    let mut project_path: Option<String> = None;
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let pending = read_jsonl(path, |index, value| {
        if value.get("type").and_then(Value::as_str) == Some("session") {
            if let Some(id) = string(&value, &["/id"]) {
                session = id;
            }
            parent = string(&value, &["/parentSession"]);
            project_path = project_path
                .take()
                .or_else(|| string(&value, &["/cwd", "/working_directory"]));
            if started_at.is_none() {
                started_at = timestamp(&value);
            }
            return;
        }
        let record = if tool == GatewayUsageTool::OpenClaw {
            super::open_claw::parse_entry(&session, index, &value, fallback)
        } else {
            parse_entry(tool, &session, index, &value, fallback)
        };
        if let Some(mut record) = record {
            if record.project_path.is_none() {
                record.project_path = project_path.clone();
            }
            records.insert(record.request_id.clone(), record);
        }
    })?;
    if let Some(parent) = parent {
        let parent_path = resolve_parent_path(path, &parent);
        // OMP records `parentSession` as a bare session id (not a path) since it
        // moved to modular session APIs, and its gc archives old sessions outside
        // the sessions root, so a forked transcript often has no readable parent.
        // Inherited entries keep the parent's entry IDs and timestamps.
        let mut inherited_ids = HashSet::new();
        let parent_readable = parent_path.is_file();
        if parent_readable {
            let inherited = parse_inner(tool, &parent_path, fallback, ancestors)?;
            // Forks copy short entry IDs and timestamps, and OMP clears cost only.
            // Match against the actual parent; an 8-character entry ID alone is
            // not globally unique. Preserve the original invocation's identity.
            for (id, record) in records.iter_mut() {
                let entry_id = record.request_id.rsplit(':').next();
                if let Some(original) = inherited.records.iter().find(|original| {
                    original.request_id.rsplit(':').next() == entry_id
                        && original.created_at == record.created_at
                        && original.model == record.model
                        && original.usage == record.usage
                }) {
                    *record = original.clone();
                    inherited_ids.insert(id.clone());
                }
            }
        } else {
            log::debug!(
                "Pi/OMP parent session {parent} for {} is unavailable; keeping only usage recorded after the fork",
                path.display()
            );
        }
        // A record that predates this session and was not proven against the
        // readable parent above belongs to an ancestor: that ancestor's own
        // transcript, where it still exists, is scanned under its own identity,
        // so importing it here would bill it twice. A chain whose oldest link is
        // gone stops proving its ancestors one level up, which is what this
        // drops. Without a start timestamp the prefix cannot be bounded at all,
        // so an unreadable parent leaves the file unimportable (its own calls
        // included) rather than backfilling unprovable history as new usage.
        if let Some(started_at) = started_at {
            records
                .retain(|id, record| inherited_ids.contains(id) || record.created_at > started_at);
        } else if !parent_readable {
            records.clear();
        }
    }
    ancestors.remove(path);
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

fn resolve_parent_path(source: &Path, parent: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    if parent.starts_with('/') {
        if let Some(std::path::Component::Prefix(prefix)) = source.components().next() {
            match prefix.kind() {
                std::path::Prefix::UNC(server, _) | std::path::Prefix::VerbatimUNC(server, _)
                    if server == "wsl.localhost" || server == "wsl$" =>
                {
                    return PathBuf::from(prefix.as_os_str())
                        .join(parent.trim_start_matches('/').replace('/', "\\"));
                }
                _ => {}
            }
        }
    }
    let parent = PathBuf::from(parent);
    if parent.is_absolute() {
        parent
    } else {
        source.parent().unwrap_or(Path::new(".")).join(parent)
    }
}

pub(super) fn parse_entry(
    tool: GatewayUsageTool,
    session: &str,
    index: usize,
    entry: &Value,
    fallback: i64,
) -> Option<SessionUsageRecord> {
    let kind = entry.get("type")?.as_str()?;
    let payload = match kind {
        "message" => {
            let message = entry.get("message")?;
            match message.get("role")?.as_str()? {
                "assistant" => message,
                // Task/subagent tools summarize child sessions that are scanned
                // independently. Other tools may report their own real usage.
                "toolResult"
                    if !matches!(
                        message.get("toolName").and_then(Value::as_str),
                        Some("task" | "subagent")
                    ) =>
                {
                    message
                }
                _ => return None,
            }
        }
        "compaction" | "branch_summary" | "model_usage" => entry,
        _ => return None,
    };
    let raw = payload.get("usage")?.as_object()?;
    if !["input", "output", "cacheRead", "cacheWrite"]
        .iter()
        .any(|key| raw.contains_key(*key))
    {
        return None;
    }
    let raw = payload.get("usage")?;
    let usage = TokenUsage {
        input_tokens: Some(number(raw, &["input"])),
        output_tokens: Some(number(raw, &["output"])),
        cache_read_tokens: Some(number(raw, &["cacheRead"])),
        cache_creation_tokens: Some(number(raw, &["cacheWrite"])),
        envelope_id: string(payload, &["/responseId", "/response_id"]),
    };
    let identity = string(entry, &["/id"]).unwrap_or_else(|| format!("entry-{index}"));
    let mut record = native_record(
        tool,
        session,
        &identity,
        string(payload, &["/model", "/modelId", "/message/model", "/message/modelId"]),
        usage,
        timestamp(entry)
            .or_else(|| timestamp(payload))
            .unwrap_or(fallback),
    );
    record.duration_ms = request_duration_ms(entry, payload);
    record.first_token_ms = record
        .duration_ms
        .and_then(|duration| request_first_token_ms(payload, duration));
    record.metadata.native_provider = string(payload, &["/provider"]);
    record.metadata.reported_total_tokens = raw.get("totalTokens").and_then(Value::as_u64);
    record.reported_cost_usd = reported_cost(raw.pointer("/cost/total"));
    if record.reported_cost_usd.is_some() {
        record.metadata.cost_source = Some("reported".into());
    }
    record.metadata.incomplete = matches!(
        payload.get("stopReason").and_then(Value::as_str),
        Some("error" | "aborted")
    );
    Some(record)
}

/// How long the call took, from either an explicit duration or the pair of
/// timestamps a Pi-compatible client writes.
///
/// `message.timestamp` is when the streaming request started (milliseconds),
/// while the outer entry's `timestamp` is when the reply was persisted; the
/// difference is the end-to-end time. Clients that record `duration` /
/// `ttft` (milliseconds) report the model's own generation time instead.
pub(super) fn request_duration_ms(entry: &Value, message: &Value) -> Option<u64> {
    if let Some(duration) = json_millis(message.get("duration"))
        .and_then(native_session_duration_ms)
    {
        return Some(duration);
    }
    session_duration_ms(
        timestamp_millis(message)?,
        timestamp_millis(entry)?,
    )
}

pub(super) fn request_first_token_ms(message: &Value, duration_ms: u64) -> Option<u64> {
    let ttft = json_millis(message.get("ttft"))?;
    bounded_session_first_token_ms(ttft, duration_ms)
}
