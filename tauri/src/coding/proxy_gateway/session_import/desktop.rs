use super::parsers::{
    parse_generic_file, parse_value, read_jsonl, reported_cost, string, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageGranularity};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(super) fn official_session_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(target_os = "windows") {
        for variable in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(base) = std::env::var_os(variable) {
                roots.push(PathBuf::from(base).join("Claude/local-agent-mode-sessions"));
            }
        }
    } else if let Some(base) = dirs::config_dir() {
        roots.push(base.join("Claude/local-agent-mode-sessions"));
    }
    roots
}

pub(super) fn select_sources(files: Vec<PathBuf>) -> Vec<PathBuf> {
    files
        .into_iter()
        .filter(|path| {
            path.file_name().is_some_and(|name| name == "audit.jsonl")
                || path.components().any(|part| part.as_os_str() == "projects")
        })
        .collect()
}

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    if path.file_name().is_none_or(|name| name != "audit.jsonl") {
        let mut parsed = parse_generic_file(GatewayUsageTool::ClaudeDesktop, path, fallback)?;
        // A transcript contains assistant snapshots, not SDK result summaries.
        parsed
            .records
            .retain(|record| record.model != "<synthetic>");
        return Ok(parsed);
    }
    let root = path.parent().unwrap_or(Path::new("."));
    let mut covered = HashSet::new();
    for entry in WalkDir::new(root.join(".claude/projects"))
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "jsonl")
        {
            for record in
                parse_generic_file(GatewayUsageTool::ClaudeDesktop, entry.path(), fallback)?.records
            {
                covered.insert(record.session_id);
            }
        }
    }
    let mut records = BTreeMap::new();
    let mut retired = Vec::new();
    let mut session = root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let _pending = read_jsonl(path, |index, value| {
        if let Some(id) = string(&value, &["/session_id", "/sessionId"]) {
            session = id;
        }
        if value.get("type").and_then(Value::as_str) != Some("result") {
            return;
        }
        let legacy = parse_value(
            GatewayUsageTool::ClaudeDesktop,
            &value,
            &session,
            index,
            fallback,
        );
        if covered.contains(&session) {
            // The result is the sum of transcript messages, never another call.
            if let Some(record) = legacy {
                retired.push(record);
            }
            return;
        }
        if let Some(mut record) = legacy {
            // A result is a final SDK turn aggregate. Keep a single identity;
            // modelUsage is a breakdown of the same total, not extra calls.
            if let Some(models) = value
                .get("modelUsage")
                .and_then(Value::as_object)
                .filter(|models| models.len() == 1)
            {
                record.model = models.keys().next().unwrap().clone();
            }
            record.metadata.granularity = SessionUsageGranularity::Turn;
            record.metadata.call_count = value.get("modelCalls").and_then(Value::as_u64);
            record.reported_cost_usd = reported_cost(value.get("total_cost_usd"));
            if record.reported_cost_usd.is_some() {
                record.metadata.cost_source = Some("reported".into());
            }
            record.metadata.incomplete = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            records.insert(record.request_id.clone(), record);
        }
    })?;
    // Revisit audit fallback even without another audit append: a transcript
    // may become readable after this scan and supersede its aggregate rows.
    Ok(ParsedSession {
        records: records.into_values().collect(),
        retired_records: retired,
        pending: true,
        ..Default::default()
    })
}
