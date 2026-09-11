use super::parsers::{native_record, number, read_jsonl, string, timestamp, ParsedSession};
use super::{GatewayUsageTool, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub(super) fn is_usage_file(tool: GatewayUsageTool, path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if tool == GatewayUsageTool::KimiCli {
        return name == "wire.jsonl";
    }
    name == "wire.jsonl"
        || (name.ends_with(".jsonl") && path.components().any(|part| part.as_os_str() == "trees"))
}

fn session_identity(path: &Path) -> String {
    if let Some(agents) = path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "agents"))
    {
        return agents
            .parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
    }
    path.parent()
        .and_then(Path::file_name)
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub(super) fn parse(
    tool: GatewayUsageTool,
    path: &Path,
    fallback: i64,
) -> Result<ParsedSession, String> {
    let mut session = session_identity(path);
    let mut default_agent = if path.file_name().is_some_and(|name| name == "wire.jsonl") {
        path.parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    } else {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    let mut records = BTreeMap::<String, SessionUsageRecord>::new();
    let mut occurrences = HashMap::<String, usize>::new();
    let mut steps = HashMap::<String, u64>::new();
    let pending = read_jsonl(path, |index, line| {
        if let Some(tree) = string(&line, &["/tree"]) {
            session = tree;
        }
        if let Some(branch) = string(&line, &["/branch"]) {
            default_agent = branch;
        }
        let value = line.pointer("/payload/data").unwrap_or(&line);
        if tool == GatewayUsageTool::Kimi
            && value.get("type").and_then(Value::as_str) == Some("usage.record")
        {
            let payload = value.get("record").unwrap_or(value);
            let Some(raw) = payload.get("usage") else {
                return;
            };
            if !raw.as_object().is_some_and(|fields| {
                fields.contains_key("inputOther") || fields.contains_key("output")
            }) {
                return;
            }
            let agent = string(value, &["/agentId"]).unwrap_or_else(|| default_agent.clone());
            let signature = format!(
                "{:x}",
                Sha256::digest(
                    serde_json::json!([
                        payload.get("time").or_else(|| payload.get("at")),
                        agent,
                        string(payload, &["/model", "/model/model"]),
                        raw,
                        value.get("usageScope")
                    ])
                    .to_string()
                    .as_bytes()
                )
            );
            let occurrence = occurrences.entry(signature.clone()).or_default();
            *occurrence += 1;
            let usage = TokenUsage {
                input_tokens: Some(number(raw, &["inputOther"])),
                output_tokens: Some(number(raw, &["output"])),
                cache_read_tokens: Some(number(raw, &["inputCacheRead"])),
                cache_creation_tokens: Some(number(raw, &["inputCacheCreation"])),
                envelope_id: string(payload, &["/messageId", "/responseId"]),
            };
            let mut record = native_record(
                tool,
                &session,
                &format!("{agent}:{signature}:{occurrence}"),
                string(payload, &["/model", "/model/model"]),
                usage,
                timestamp(payload)
                    .or_else(|| timestamp(&line))
                    .unwrap_or(fallback),
            );
            record.metadata.native_provider = string(payload, &["/provider", "/model/provider"]);
            records.insert(record.request_id.clone(), record);
        } else if tool == GatewayUsageTool::KimiCli {
            let envelope = value.get("message").unwrap_or(value);
            collect_python(
                envelope,
                &session,
                "main",
                index,
                timestamp(value).unwrap_or(fallback),
                &mut steps,
                &mut records,
                0,
            );
        }
    })?;
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

fn collect_python(
    value: &Value,
    session: &str,
    agent: &str,
    index: usize,
    created_at: i64,
    steps: &mut HashMap<String, u64>,
    records: &mut BTreeMap<String, SessionUsageRecord>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    let payload = value.get("payload").unwrap_or(value);
    if kind == "SubagentEvent" {
        if let Some(event) = payload.get("event") {
            let child = string(
                payload,
                &["/agent_id", "/parent_tool_call_id", "/task_tool_call_id"],
            )
            .unwrap_or_else(|| format!("{agent}:child"));
            collect_python(
                event,
                session,
                &child,
                index,
                created_at,
                steps,
                records,
                depth + 1,
            );
        }
        return;
    }
    if kind == "StepBegin" || kind == "StepRetry" {
        *steps.entry(agent.into()).or_default() += 1;
        return;
    }
    if kind != "StatusUpdate" {
        return;
    }
    let Some(raw) = payload.get("token_usage").filter(|value| value.is_object()) else {
        return;
    };
    let envelope_id = string(payload, &["/message_id"]);
    let identity = envelope_id.clone().unwrap_or_else(|| {
        format!(
            "{agent}:step:{}",
            steps.get(agent).copied().unwrap_or(index as u64)
        )
    });
    let usage = TokenUsage {
        input_tokens: Some(number(raw, &["input_other"])),
        output_tokens: Some(number(raw, &["output"])),
        cache_read_tokens: Some(number(raw, &["input_cache_read"])),
        cache_creation_tokens: Some(number(raw, &["input_cache_creation"])),
        envelope_id,
    };
    let mut record = native_record(
        GatewayUsageTool::KimiCli,
        session,
        &identity,
        string(payload, &["/model"]),
        usage,
        created_at,
    );
    if let Some(envelope) = &record.usage.envelope_id {
        record.request_id = format!("SESSION:kimi_cli:response:{envelope}");
    }
    record.metadata.native_provider = string(payload, &["/provider"]);
    records.insert(record.request_id.clone(), record);
}
