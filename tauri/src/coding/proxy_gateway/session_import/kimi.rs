use super::parsers::{
    bounded_session_first_token_ms, json_millis, native_record, native_session_duration_ms,
    number, read_jsonl, session_duration_ms, string, timestamp, timestamp_millis, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageRecord, TokenUsage};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Default)]
struct KimiStepTiming {
    uuid: String,
    turn_step: Option<String>,
    request_ms: Option<i64>,
    response_ms: Option<i64>,
    has_tools: bool,
    usage_request_ids: Vec<String>,
}

#[derive(Default)]
struct KimiRequestTiming {
    duration_ms: Option<u64>,
    first_token_ms: Option<u64>,
}

/// Tracks per-agent Kimi Code loop steps (`step.begin` / `llm.request` /
/// `content.part` / `tool.call` / `step.end`) to resolve native
/// `llmFirstTokenLatencyMs` + `llmStreamDurationMs` (or fallback wall-clock
/// duration excluding tool execution).
#[derive(Default)]
struct KimiTimingIndex {
    active_steps: HashMap<String, KimiStepTiming>,
    by_message_id: HashMap<String, KimiRequestTiming>,
    by_request_id: HashMap<String, KimiRequestTiming>,
}

impl KimiTimingIndex {
    fn observe(&mut self, agent: &str, value: &Value, line: &Value) {
        let event_time = timestamp_millis(value).or_else(|| timestamp_millis(line));
        match value.get("type").and_then(Value::as_str) {
            Some("agent.switched" | "agent.turn.started") => {
                self.active_steps.remove(agent);
            }
            Some("llm.request") => {
                if value.get("kind").and_then(Value::as_str).is_some_and(|kind| kind != "loop") {
                    return;
                }
                if let Some(step) = self.active_steps.get_mut(agent) {
                    if let (Some(expected), Some(actual)) = (
                        step.turn_step.as_deref(),
                        value.get("turnStep").and_then(Value::as_str),
                    ) {
                        if expected != actual {
                            return;
                        }
                    }
                    step.request_ms = event_time;
                    step.response_ms = None;
                    step.has_tools = false;
                }
            }
            Some("context.append_loop_event") => {
                let Some(event) = value.get("event") else {
                    return;
                };
                match event.get("type").and_then(Value::as_str) {
                    Some("step.begin") => {
                        if let Some(uuid) = event.get("uuid").and_then(Value::as_str) {
                            let turn_step = event
                                .get("turnId")
                                .and_then(Value::as_str)
                                .zip(event.get("step").and_then(Value::as_u64))
                                .map(|(turn, step)| format!("{turn}.{step}"));
                            self.active_steps.insert(
                                agent.to_string(),
                                KimiStepTiming {
                                    uuid: uuid.to_string(),
                                    turn_step,
                                    ..Default::default()
                                },
                            );
                        }
                    }
                    Some("content.part" | "tool.call") => {
                        if let Some(step) = self.active_steps.get_mut(agent) {
                            if event.get("stepUuid").and_then(Value::as_str)
                                != Some(step.uuid.as_str())
                            {
                                return;
                            }
                            if event.get("type").and_then(Value::as_str) == Some("tool.call") {
                                step.has_tools = true;
                            } else {
                                step.response_ms = step.response_ms.max(event_time);
                            }
                        }
                    }
                    Some("step.end") => {
                        if self.active_steps.get(agent).is_some_and(|step| {
                            event.get("uuid").and_then(Value::as_str) != Some(step.uuid.as_str())
                        }) {
                            return;
                        }
                        let step = self.active_steps.remove(agent).unwrap_or_default();
                        let raw_first = json_millis(event.get("llmFirstTokenLatencyMs"));
                        let raw_stream = json_millis(event.get("llmStreamDurationMs"));
                        let native_duration = raw_first
                            .zip(raw_stream)
                            .and_then(|(first, stream)| {
                                native_session_duration_ms(first.saturating_add(stream))
                            });
                        let end_ms = step
                            .response_ms
                            .or_else(|| (!step.has_tools).then_some(event_time).flatten());
                        let duration_ms = native_duration.or_else(|| {
                            session_duration_ms(step.request_ms?, end_ms?)
                        });
                        let first_token_ms = duration_ms.and_then(|duration| {
                            bounded_session_first_token_ms(raw_first?, duration)
                        });
                        if duration_ms.is_none() {
                            return;
                        }
                        if let Some(message_id) = event
                            .get("messageId")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                        {
                            let entry = self
                                .by_message_id
                                .entry(message_id.to_string())
                                .or_default();
                            entry.duration_ms = duration_ms.or(entry.duration_ms);
                            entry.first_token_ms = first_token_ms.or(entry.first_token_ms);
                        }
                        for req_id in step.usage_request_ids {
                            let entry = self.by_request_id.entry(req_id).or_default();
                            entry.duration_ms = duration_ms.or(entry.duration_ms);
                            entry.first_token_ms = first_token_ms.or(entry.first_token_ms);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn bind_usage_record(&mut self, agent: &str, request_id: &str) {
        if let Some(step) = self.active_steps.get_mut(agent) {
            step.usage_request_ids.push(request_id.to_string());
        }
    }

    fn apply(&self, record: &mut SessionUsageRecord) {
        let timing = record
            .usage
            .envelope_id
            .as_deref()
            .and_then(|id| self.by_message_id.get(id))
            .or_else(|| self.by_request_id.get(&record.request_id));
        if let Some(timing) = timing {
            if record.duration_ms.is_none() {
                record.duration_ms = timing.duration_ms;
            }
            if record.first_token_ms.is_none() {
                record.first_token_ms = timing.first_token_ms;
            }
        }
    }
}

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

fn session_dir(path: &Path) -> &Path {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "agents"))
        .and_then(Path::parent)
        .or_else(|| path.parent())
        .unwrap_or(path)
}

/// Kimi Code records the working directory in the session's `state.json`
/// (`cwd`); older Python Kimi only has the `kimi.json` work-dir registry, keyed
/// by an MD5 of the project path, so the registry is reversed here.
fn project_of(tool: GatewayUsageTool, path: &Path) -> Option<String> {
    let directory = session_dir(path);
    for name in ["state.json", "summary.json"] {
        let Ok(text) = std::fs::read_to_string(directory.join(name)) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(project) = string(&value, &["/cwd", "/work_dir", "/working_directory"]) {
            return Some(project);
        }
    }
    if tool != GatewayUsageTool::KimiCli {
        return None;
    }
    // Legacy layout: <root>/sessions/<md5(project)>/<session>/wire.jsonl.
    let key = directory.parent()?.file_name()?.to_str()?;
    let root = directory.parent()?.parent()?.parent()?;
    let text = std::fs::read_to_string(root.join("kimi.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("work_dirs")?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .find(|project| format!("{:x}", md5::compute(project.as_bytes())) == key)
        .map(str::to_string)
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
    let mut python_step_starts = HashMap::<String, i64>::new();
    let mut timing_index = KimiTimingIndex::default();
    let project_path = project_of(tool, path);
    let pending = read_jsonl(path, |index, line| {
        if let Some(tree) = string(&line, &["/tree"]) {
            session = tree;
        }
        if let Some(branch) = string(&line, &["/branch"]) {
            default_agent = branch;
        }
        let value = line.pointer("/payload/data").unwrap_or(&line);
        if tool == GatewayUsageTool::Kimi {
            let agent = string(value, &["/agentId"]).unwrap_or_else(|| default_agent.clone());
            timing_index.observe(&agent, value, &line);
            if value.get("type").and_then(Value::as_str) == Some("usage.record") {
                let payload = value.get("record").unwrap_or(value);
                let Some(raw) = payload.get("usage") else {
                    return;
                };
                if !raw.as_object().is_some_and(|fields| {
                    fields.contains_key("inputOther") || fields.contains_key("output")
                }) {
                    return;
                }
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
                    cache_creation_1h_tokens: None,
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
                record.metadata.native_provider =
                    string(payload, &["/provider", "/model/provider"]);
                record.project_path = project_path.clone();
                timing_index.bind_usage_record(&agent, &record.request_id);
                records.insert(record.request_id.clone(), record);
            }
        } else if tool == GatewayUsageTool::KimiCli {
            let envelope = value.get("message").unwrap_or(value);
            collect_python(
                envelope,
                &session,
                "main",
                index,
                timestamp(value).unwrap_or(fallback),
                timestamp_millis(value),
                &mut steps,
                &mut python_step_starts,
                &mut records,
                0,
                &project_path,
            );
        }
    })?;
    if tool == GatewayUsageTool::Kimi {
        for record in records.values_mut() {
            timing_index.apply(record);
        }
    }
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_python(
    value: &Value,
    session: &str,
    agent: &str,
    index: usize,
    created_at: i64,
    event_ms: Option<i64>,
    steps: &mut HashMap<String, u64>,
    step_starts: &mut HashMap<String, i64>,
    records: &mut BTreeMap<String, SessionUsageRecord>,
    depth: usize,
    project_path: &Option<String>,
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
                event_ms,
                steps,
                step_starts,
                records,
                depth + 1,
                project_path,
            );
        }
        return;
    }
    if kind == "StepBegin" || kind == "StepRetry" {
        *steps.entry(agent.into()).or_default() += 1;
        if let Some(ms) = event_ms {
            step_starts.insert(agent.into(), ms);
        }
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
        cache_creation_1h_tokens: None,
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
    record.duration_ms = match (step_starts.remove(agent), event_ms) {
        (Some(start), Some(end)) => session_duration_ms(start, end),
        _ => None,
    };
    record.metadata.native_provider = string(payload, &["/provider"]);
    // The Python Kimi store keeps the work dir beside the wire log; the legacy
    // registry lookup happens once per file in `parse`, not per row.
    record.project_path = project_path.clone();
    records.insert(record.request_id.clone(), record);
}
