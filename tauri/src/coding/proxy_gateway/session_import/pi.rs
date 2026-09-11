use super::parsers::{
    native_record, number, read_jsonl, reported_cost, string, timestamp, ParsedSession,
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
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let pending = read_jsonl(path, |index, value| {
        if value.get("type").and_then(Value::as_str) == Some("session") {
            if let Some(id) = string(&value, &["/id"]) {
                session = id;
            }
            parent = string(&value, &["/parentSession"]);
            return;
        }
        let record = if tool == GatewayUsageTool::OpenClaw {
            super::open_claw::parse_entry(&session, index, &value, fallback)
        } else {
            parse_entry(tool, &session, index, &value, fallback)
        };
        if let Some(record) = record {
            records.insert(record.request_id.clone(), record);
        }
    })?;
    if let Some(parent) = parent {
        let parent_path = resolve_parent_path(path, &parent);
        let inherited = parse_inner(tool, &parent_path, fallback, ancestors)?;
        // Forks copy short entry IDs and timestamps, and OMP clears cost only.
        // Match against the actual parent; an 8-character entry ID alone is
        // not globally unique. Preserve the original invocation's identity.
        for record in records.values_mut() {
            let entry_id = record.request_id.rsplit(':').next();
            if let Some(original) = inherited.records.iter().find(|original| {
                original.request_id.rsplit(':').next() == entry_id
                    && original.created_at == record.created_at
                    && original.model == record.model
                    && original.usage == record.usage
            }) {
                *record = original.clone();
            }
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
        string(payload, &["/model", "/modelId"]),
        usage,
        timestamp(entry)
            .or_else(|| timestamp(payload))
            .unwrap_or(fallback),
    );
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
