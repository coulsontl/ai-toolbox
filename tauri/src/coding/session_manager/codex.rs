use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::codex_rollout;
use super::message_blocks::{
    message_from_blocks, text_block, thinking_block, tool_call_block, tool_result_block,
    usage_from_value,
};
use super::utils::{
    build_resume_command, collect_recent_files_by_modified, extract_prompt_title_text,
    extract_text, join_safe_relative, parse_timestamp_to_ms, path_basename, read_head_tail_lines,
    sanitize_path_segment, strip_path_prefix, text_contains_query, truncate_summary,
};
use super::{assign_missing_message_ids, SessionMessage, SessionMeta};

const PROVIDER_ID: &str = "codex";
const SESSION_INDEX_FILE_NAME: &str = "session_index.jsonl";

static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
        .unwrap()
});

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionIndexEntry {
    id: String,
    thread_name: String,
    #[serde(default)]
    updated_at: String,
}

pub fn scan_sessions(root: &Path) -> Vec<SessionMeta> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
    let files = dedupe_rollout_siblings(files);

    let thread_names = read_thread_names_by_session_id(root);

    files
        .into_iter()
        .filter_map(|path| parse_session(&path))
        .map(|mut session| {
            if let Some(thread_name) = thread_names.get(&session.session_id) {
                session.title = Some(thread_name.clone());
            }
            session
        })
        .collect()
}

pub fn scan_recent_sessions(root: &Path, limit: usize) -> Vec<SessionMeta> {
    if limit == 0 {
        return Vec::new();
    }

    let thread_names = read_thread_names_by_session_id(root);
    let files = dedupe_rollout_siblings(collect_recent_files_by_modified(
        root,
        limit.saturating_mul(3).max(limit),
        is_rollout_artifact_file,
    ));
    let mut sessions = Vec::new();
    for path in files {
        let Some(mut session) = parse_session(&path) else {
            continue;
        };
        if let Some(thread_name) = thread_names.get(&session.session_id) {
            session.title = Some(thread_name.clone());
        }
        sessions.push(session);
        if sessions.len() >= limit {
            break;
        }
    }

    sessions
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    // Keep the original contract: a session file that cannot be opened is an
    // error, not an empty conversation. Ancestors that vanish mid-lineage are
    // handled far more gently below.
    File::open(path).map_err(|error| format!("Failed to open session file: {error}"))?;

    let sessions_root = find_sessions_root(path);
    let mut messages = Vec::new();
    let mut current_model: Option<String> = None;
    let mut prev_token_usage = CodexTokenUsageTotals::default();

    visit_lineage_lines(sessions_root.as_deref(), path, |line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return true;
        };
        // One shared per-record path for reading and searching, so a record type
        // added for one is never silently missing from the other.
        codex_record_search_text(
            &value,
            &mut messages,
            &mut current_model,
            &mut prev_token_usage,
        );
        true
    });

    assign_missing_message_ids(&mut messages, PROVIDER_ID);
    Ok(messages)
}

pub fn scan_messages_for_query(path: &Path, query_lower: &str) -> Result<bool, String> {
    File::open(path).map_err(|error| format!("Failed to open session file: {error}"))?;

    let sessions_root = find_sessions_root(path);
    let mut messages = Vec::new();
    let mut current_model: Option<String> = None;
    let mut prev_token_usage = CodexTokenUsageTotals::default();
    let mut found = false;

    // A thread's earlier records live in its prefix rollouts, so searching only
    // the file we were handed would miss most of the conversation.
    visit_lineage_lines(sessions_root.as_deref(), path, |line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return true;
        };

        if let Some(text) = codex_record_search_text(
            &value,
            &mut messages,
            &mut current_model,
            &mut prev_token_usage,
        ) {
            if text_contains_query(&text, query_lower) {
                found = true;
                return false;
            }
        }
        if let Some(last_message) = messages.last() {
            if text_contains_query(&last_message.content, query_lower) {
                found = true;
                return false;
            }
        }
        true
    });

    Ok(found)
}

/// Feed every record of a session, oldest rollout first, to `visit`.
///
/// `visit` returns `false` to stop early. Reading the whole lineage rather than
/// the single requested file is what makes a `paginated` thread's conversation
/// complete: its newest rollout only holds the records since the last revert,
/// and the earlier ones stay in the prefix files it points at.
fn visit_lineage_lines(
    sessions_root: Option<&Path>,
    path: &Path,
    mut visit: impl FnMut(&str) -> bool,
) {
    let lineage = match sessions_root {
        Some(root) => codex_rollout::resolve_lineage(root, path),
        None => codex_rollout::Lineage::single(path.to_path_buf()),
    };

    for segment in &lineage.segments {
        if !visit_segment_lines(segment, &mut visit) {
            return;
        }
    }
}

/// Stream one lineage segment, honoring its cutoff. Returns `false` when `visit`
/// asked to stop.
fn visit_segment_lines(
    segment: &codex_rollout::LineageSegment,
    visit: &mut impl FnMut(&str) -> bool,
) -> bool {
    // A prefix rollout Codex has already compressed decodes transparently; the
    // cutoff offsets history_base records are plain-text offsets, so they apply
    // to the decoded stream.
    let Some(mut reader) = codex_rollout::open_rollout_reader(&segment.path, None) else {
        // An ancestor we cannot read costs us its records, not the whole session.
        return true;
    };
    let mut buffer: Vec<u8> = Vec::new();
    let mut consumed: u64 = 0;

    loop {
        buffer.clear();
        let read = match reader.read_until(b'\n', &mut buffer) {
            Ok(read) => read,
            Err(_) => return true,
        };
        if read == 0 {
            return true;
        }
        // The cutoff is "immediately after the last included record", so a record
        // starting at or past it belongs to the child, not to this segment.
        if segment
            .end_byte_offset
            .is_some_and(|limit| consumed >= limit)
        {
            return true;
        }
        consumed = consumed.saturating_add(read as u64);

        if !visit(&String::from_utf8_lossy(&buffer)) {
            return false;
        }
    }
}

#[derive(Default)]
struct CodexTokenUsageTotals {
    input_tokens: i64,
    output_tokens: i64,
    cached_input_tokens: i64,
}

fn codex_record_search_text(
    value: &Value,
    messages: &mut Vec<SessionMessage>,
    current_model: &mut Option<String>,
    prev_token_usage: &mut CodexTokenUsageTotals,
) -> Option<String> {
    let record_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
    match record_type {
        "turn_context" => {
            if let Some(model) = value
                .get("payload")
                .and_then(|payload| payload.get("model"))
                .and_then(Value::as_str)
            {
                *current_model = Some(model.to_string());
            }
            None
        }
        "response_item" => {
            let payload = value.get("payload")?;
            let message = codex_message_from_payload(payload, ts, current_model.as_deref())?;
            if message.content.trim().is_empty() {
                return None;
            }
            let text = message.content.clone();
            messages.push(message);
            Some(text)
        }
        "event_msg" => {
            let payload = value.get("payload")?;
            let previous_len = messages.len();
            apply_codex_event_message(messages, payload, ts, prev_token_usage);
            if messages.len() > previous_len {
                return messages.last().map(|message| message.content.clone());
            }
            None
        }
        "compacted" => {
            let message = codex_compacted_message(value.get("payload")?, ts);
            let text = message.content.clone();
            messages.push(message);
            Some(text)
        }
        _ => None,
    }
}

fn codex_message_from_payload(
    payload: &Value,
    ts: Option<i64>,
    current_model: Option<&str>,
) -> Option<SessionMessage> {
    let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    let mut message = match payload_type {
        "message" => {
            let role = payload
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let content = payload.get("content").map(extract_text).unwrap_or_default();
            if content.trim().is_empty() {
                return None;
            }
            message_from_blocks(role, ts, vec![text_block(content)])
        }
        "local_shell_call" => {
            let tool_id = codex_tool_id(payload);
            let input = codex_local_shell_input(payload);
            message_from_blocks(
                "assistant",
                ts,
                vec![tool_call_block(tool_id, "Bash".to_string(), Some(input))],
            )
        }
        "function_call" | "custom_tool_call" => {
            let name = codex_tool_name(payload_type, payload);
            let tool_id = codex_tool_id(payload);
            let input = codex_tool_input(payload_type, &name, payload);
            message_from_blocks(
                "assistant",
                ts,
                vec![tool_call_block(tool_id, name, Some(input))],
            )
        }
        "web_search_call" => {
            let tool_id = codex_tool_id(payload);
            let input = codex_web_search_input(
                payload
                    .get("action")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
            );
            message_from_blocks(
                "assistant",
                ts,
                vec![tool_call_block(
                    tool_id,
                    "WebSearch".to_string(),
                    Some(input),
                )],
            )
        }
        "function_call_output" | "custom_tool_call_output" => {
            let tool_id = codex_tool_id(payload);
            let output = payload
                .get("output")
                .cloned()
                .map(normalize_codex_tool_output);
            message_from_blocks(
                "tool",
                ts,
                vec![tool_result_block(tool_id, None, output, None)],
            )
        }
        "reasoning" => {
            let content = codex_reasoning_text(payload);
            if content.trim().is_empty() {
                return None;
            }
            message_from_blocks("assistant", ts, vec![thinking_block(content)])
        }
        _ => return None,
    };

    message.id = payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string);
    message.message_type = Some(payload_type.to_string());
    message.model = payload
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| current_model.map(str::to_string));
    message.usage = payload.get("usage").and_then(usage_from_value);
    Some(message)
}

fn apply_codex_event_message(
    messages: &mut Vec<SessionMessage>,
    payload: &Value,
    ts: Option<i64>,
    prev_token_usage: &mut CodexTokenUsageTotals,
) -> bool {
    let event_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "user_message" | "agent_message" => true,
        "token_count" => {
            if let Some(delta_usage) = codex_token_delta_usage(payload, prev_token_usage) {
                if let Some(last_message) = messages
                    .iter_mut()
                    .rev()
                    .find(|message| message.role == "assistant" && message.usage.is_none())
                {
                    last_message.usage = Some(delta_usage);
                }
            }
            true
        }
        "agent_reasoning" => true,
        "context_compacted" => {
            let mut message =
                message_from_blocks("system", ts, vec![text_block("Context compacted")]);
            message.message_type = Some(event_type.to_string());
            messages.push(message);
            true
        }
        "task_started" | "task_complete" => {
            let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
            let text = if event_type == "task_started" {
                format!("[Task Started] turn: {turn_id}")
            } else {
                format!("[Task Completed] turn: {turn_id}")
            };
            let mut message = message_from_blocks("system", ts, vec![text_block(text)]);
            message.message_type = Some(event_type.to_string());
            message.metadata = Some(serde_json::json!({
                "turnId": turn_id,
            }));
            messages.push(message);
            true
        }
        "turn_aborted" => {
            let reason = payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
            let text = format!("[Turn Aborted] reason: {reason}, turn: {turn_id}");
            let mut message = message_from_blocks("system", ts, vec![text_block(text)]);
            message.message_type = Some(event_type.to_string());
            messages.push(message);
            true
        }
        _ => false,
    }
}

fn codex_compacted_message(payload: &Value, ts: Option<i64>) -> SessionMessage {
    let replacement_history_count = payload
        .get("replacement_history")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut message = message_from_blocks("system", ts, vec![text_block("Conversation compacted")]);
    message.message_type = Some("compacted".to_string());
    message.metadata = Some(serde_json::json!({
        "replacementHistoryCount": replacement_history_count,
    }));
    message
}

fn codex_token_delta_usage(
    payload: &Value,
    previous: &mut CodexTokenUsageTotals,
) -> Option<super::SessionMessageUsage> {
    let total_usage = payload
        .get("info")
        .and_then(|info| info.get("total_token_usage"))
        .or_else(|| {
            payload
                .get("info")
                .and_then(|info| info.get("last_token_usage"))
        })?;
    let input_tokens = number_field(total_usage, &["input_tokens", "inputTokens"])?;
    let output_tokens = number_field(total_usage, &["output_tokens", "outputTokens"])?;
    let cached_input_tokens =
        number_field(total_usage, &["cached_input_tokens", "cachedInputTokens"]).unwrap_or(0);

    let delta_input = input_tokens.saturating_sub(previous.input_tokens);
    let delta_output = output_tokens.saturating_sub(previous.output_tokens);
    let delta_cached = cached_input_tokens.saturating_sub(previous.cached_input_tokens);
    previous.input_tokens = input_tokens;
    previous.output_tokens = output_tokens;
    previous.cached_input_tokens = cached_input_tokens;

    let non_cached_input = delta_input.saturating_sub(delta_cached);
    Some(super::SessionMessageUsage {
        input_tokens: Some(non_cached_input),
        output_tokens: Some(delta_output),
        cache_creation_input_tokens: None,
        cache_read_input_tokens: Some(delta_cached),
    })
}

fn codex_tool_id(payload: &Value) -> Option<String> {
    payload
        .get("call_id")
        .or_else(|| payload.get("callId"))
        .or_else(|| payload.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn codex_tool_name(payload_type: &str, payload: &Value) -> String {
    let raw_name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match (payload_type, raw_name) {
        ("function_call", "exec_command" | "shell" | "write_stdin") => "Bash",
        _ => raw_name,
    }
    .to_string()
}

fn codex_tool_input(payload_type: &str, tool_name: &str, payload: &Value) -> Value {
    let mut input = if payload_type == "function_call" {
        parse_codex_tool_arguments(payload.get("arguments").or_else(|| payload.get("input")))
    } else {
        payload.get("input").cloned().unwrap_or(Value::Null)
    };

    if payload_type == "custom_tool_call" && !input.is_object() {
        input = if tool_name == "apply_patch" {
            serde_json::json!({ "patch": input.as_str().unwrap_or("") })
        } else {
            serde_json::json!({ "input": input })
        };
    }

    normalize_codex_tool_input(tool_name, input)
}

fn codex_local_shell_input(payload: &Value) -> Value {
    let command = payload
        .get("action")
        .and_then(|action| action.get("command"))
        .cloned()
        .unwrap_or(Value::Null);
    let command = match command {
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Value::String(command) => command,
        _ => String::new(),
    };
    serde_json::json!({ "command": command })
}

fn parse_codex_tool_arguments(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(raw)) => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::Object(Map::new()))
        }
        Some(value) if value.is_object() || value.is_array() => value.clone(),
        _ => Value::Object(Map::new()),
    }
}

fn normalize_codex_tool_input(tool_name: &str, input: Value) -> Value {
    let Value::Object(mut input_object) = input else {
        return input;
    };

    if tool_name == "Bash" {
        if !input_object.contains_key("command") {
            if let Some(cmd) = input_object.get("cmd").cloned() {
                let command = match cmd {
                    Value::Array(parts) => parts
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                    Value::String(command) => command,
                    _ => String::new(),
                };
                if !command.is_empty() {
                    input_object.insert("command".to_string(), Value::String(command));
                }
            }
        }
        if let Some(Value::Array(parts)) = input_object.get("command").cloned() {
            let command = parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            input_object.insert("command".to_string(), Value::String(command));
        }
    }

    Value::Object(input_object)
}

fn codex_web_search_input(action: Value) -> Value {
    let Value::Object(action_object) = action else {
        return Value::Object(Map::new());
    };

    let mut input = Map::new();
    for key in ["query", "url", "pattern"] {
        if let Some(value) = action_object.get(key).and_then(Value::as_str) {
            input.insert("query".to_string(), Value::String(value.to_string()));
            break;
        }
    }
    if let Some(queries) = action_object.get("queries").cloned() {
        input.insert("queries".to_string(), queries);
    }
    if let Some(action_type) = action_object.get("type").and_then(Value::as_str) {
        input.insert(
            "action_type".to_string(),
            Value::String(action_type.to_string()),
        );
    }
    Value::Object(input)
}

fn normalize_codex_tool_output(output: Value) -> Value {
    let Value::String(raw) = output else {
        return output;
    };

    if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
        if let Some(inner_output) = parsed.get("output") {
            return inner_output.clone();
        }
    }
    if let Some((_, output)) = raw.split_once("\nOutput:\n") {
        return Value::String(output.to_string());
    }
    Value::String(raw)
}

fn codex_reasoning_text(payload: &Value) -> String {
    payload
        .get("summary")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn number_field(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(Value::as_i64)
        .or_else(|| {
            keys.iter()
                .find_map(|key| value.get(*key))
                .and_then(Value::as_u64)
                .map(|value| value as i64)
        })
}

/// The session id a rollout artifact carries.
///
/// Codex names rollouts `rollout-<timestamp>-<thread id>.jsonl`, so the filename
/// answers without reading the file; only a non-canonical name falls back to the
/// `session_meta` header. A compressed rollout is never parsed as text — its
/// filename keeps the ids Codex wrote into it.
fn artifact_session_id(path: &Path) -> Option<String> {
    if codex_rollout::is_compressed_rollout_path(path) {
        return infer_session_id_from_filename(path);
    }

    infer_session_id_from_filename(path).or_else(|| parse_session(path).map(|meta| meta.session_id))
}

/// Every rollout artifact of one logical session under `root` and its sibling
/// `archived_sessions/`.
///
/// Codex opens a fresh `rollout-*.jsonl` on resume while keeping the thread id in
/// `session_meta`, so one session owns several files spread over dated
/// directories. Identity is the id, not the file — and not the directory either:
/// archiving moves a thread's rollouts to `archived_sessions/`, and Codex
/// compresses cold ones to `.jsonl.zst` in place, so leaving either out would
/// resurrect the session the user just deleted.
fn session_artifacts(root: &Path, session_id: &str) -> Vec<PathBuf> {
    let mut roots = vec![root.to_path_buf()];
    if let Some(archived) = codex_rollout::archived_sessions_root(root) {
        roots.push(archived);
    }

    let mut artifacts = Vec::new();
    for root in roots {
        collect_session_artifacts(&root, session_id, &mut artifacts);
    }
    artifacts
}

fn collect_session_artifacts(root: &Path, session_id: &str, artifacts: &mut Vec<PathBuf>) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if !is_rollout_artifact_file(&path) {
                continue;
            }
            if artifact_session_id(&path).as_deref() == Some(session_id) {
                artifacts.push(path);
            }
        }
    }
}

/// Whether a file is a rollout in either representation Codex writes.
fn is_rollout_artifact_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
}

/// Delete a Codex session.
///
/// `path` is any one of the session's artifacts; every artifact of that logical
/// session is removed — across the dated `sessions/` tree, the sibling
/// `archived_sessions/` copy Codex keeps after an archive, and both the plain
/// and `.jsonl.zst` representations — so a resumed, archived or compressed
/// thread cannot reappear through a rollout file after the user deleted the
/// single row they were shown. This matches dsh (any generation deletes the
/// whole session directory) and Gemini CLI (every file carrying the session id
/// goes). Without a resolvable id or sessions root there is nothing to collapse
/// onto, so the requested file alone is removed.
pub fn delete_session(path: &Path) -> Result<(), String> {
    let artifacts = match (find_sessions_root(path), artifact_session_id(path)) {
        (Some(root), Some(session_id)) => {
            let mut artifacts = session_artifacts(&root, &session_id);
            if !artifacts.iter().any(|artifact| artifact.as_path() == path) {
                artifacts.push(path.to_path_buf());
            }
            artifacts
        }
        _ => vec![path.to_path_buf()],
    };

    ensure_no_external_references(path, &artifacts)?;

    let mut failures = Vec::new();
    for artifact in artifacts {
        match std::fs::remove_file(&artifact) {
            Ok(()) => {}
            // Deleting an already-gone artifact is convergence, not an error.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", artifact.display())),
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Failed to delete Codex session file(s): {}",
            failures.join(", ")
        ))
    }
}

/// Refuse to delete rollouts another thread still replays from.
///
/// A `paginated` thread can keep its history in an older rollout and point at it
/// with `history_base`. Removing that prefix would leave the referencing thread
/// unable to reconstruct its conversation, so the delete is rejected rather than
/// silently corrupting it. This is Codex's own
/// `ensure_no_external_references` guard, applied before its thread delete.
///
/// A rollout with no resolvable sessions root has no references to check, and one
/// whose own thread cannot be determined is left to the caller's existing
/// single-file fallback.
fn ensure_no_external_references(path: &Path, artifacts: &[PathBuf]) -> Result<(), String> {
    let (Some(sessions_root), Some(thread_id)) =
        (find_sessions_root(path), artifact_session_id(path))
    else {
        return Ok(());
    };

    let referenced =
        codex_rollout::rollout_ids_referenced_by_other_threads(&sessions_root, &thread_id);
    if referenced.is_empty() {
        return Ok(());
    }

    let blocked: Vec<String> = codex_rollout::rollout_ids_of(artifacts)
        .into_iter()
        .filter(|rollout_id| referenced.contains(rollout_id))
        .collect();
    if blocked.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Codex session {thread_id} cannot be deleted: {} rollout(s) are still used as history by other sessions",
        blocked.len()
    ))
}

pub fn rename_session(source_path: &str, next_title: &str) -> Result<(), String> {
    let normalized_title = next_title.trim();
    if normalized_title.is_empty() {
        return Err("Session title cannot be empty".to_string());
    }

    let session_path = Path::new(source_path);
    let session_id = parse_session(session_path)
        .map(|session| session.session_id)
        .or_else(|| infer_session_id_from_filename(session_path))
        .ok_or_else(|| {
            format!(
                "Failed to determine Codex session id from {}",
                session_path.display()
            )
        })?;
    let sessions_root = find_sessions_root(session_path).ok_or_else(|| {
        format!(
            "Failed to determine Codex sessions root from {}",
            session_path.display()
        )
    })?;

    append_thread_name(&sessions_root, &session_id, normalized_title)
}

pub fn export_native_snapshot(
    root: &Path,
    session_path: &Path,
) -> Result<serde_json::Value, String> {
    let session_id = parse_session(session_path)
        .map(|session| session.session_id)
        .or_else(|| infer_session_id_from_filename(session_path))
        .ok_or_else(|| {
            format!(
                "Failed to determine Codex session id from {}",
                session_path.display()
            )
        })?;
    let relative_session_path = strip_path_prefix(root, session_path).ok_or_else(|| {
        format!(
            "Session path {} is outside Codex session root {}",
            session_path.display(),
            root.display()
        )
    })?;
    let relative_session_path = relative_session_path
        .strip_suffix(".zst")
        .filter(|stem| stem.ends_with(".jsonl"))
        .unwrap_or(&relative_session_path)
        .to_string();
    let session_file_content = read_rollout_text(session_path)?;
    let session_index_entry = read_session_index_entry(root, &session_id);

    Ok(serde_json::json!({
        "relativeSessionPath": relative_session_path,
        "sessionFileContent": session_file_content,
        "sessionIndexEntry": session_index_entry,
    }))
}

pub fn import_native_snapshot(
    root: &Path,
    session_id: &str,
    snapshot: &serde_json::Value,
) -> Result<PathBuf, String> {
    let relative_session_path = snapshot
        .get("relativeSessionPath")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Codex snapshot missing relativeSessionPath".to_string())?;
    let session_file_content = snapshot
        .get("sessionFileContent")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Codex snapshot missing sessionFileContent".to_string())?;

    let fallback_file_name = format!(
        "{}.jsonl",
        sanitize_path_segment(session_id, "codex-session")
    );
    let normalized_relative_path = if relative_session_path.ends_with(".jsonl") {
        relative_session_path.to_string()
    } else {
        format!(
            "{}/{}",
            relative_session_path.trim_end_matches('/'),
            fallback_file_name
        )
    };

    let target_path = join_safe_relative(root, &normalized_relative_path)?;
    if target_path.exists() {
        return Err(format!(
            "Codex session file already exists: {}",
            target_path.display()
        ));
    }

    let parent_dir = target_path.parent().ok_or_else(|| {
        format!(
            "Failed to determine Codex session parent directory for {}",
            target_path.display()
        )
    })?;
    std::fs::create_dir_all(parent_dir).map_err(|error| {
        format!(
            "Failed to create Codex session directory {}: {error}",
            parent_dir.display()
        )
    })?;
    std::fs::write(&target_path, session_file_content).map_err(|error| {
        format!(
            "Failed to write Codex session file {}: {error}",
            target_path.display()
        )
    })?;
    append_session_index_entry(root, session_id, snapshot.get("sessionIndexEntry"))?;

    Ok(target_path)
}

fn read_thread_names_by_session_id(root: &Path) -> std::collections::HashMap<String, String> {
    let Some(session_index_path) = session_index_path(root) else {
        return std::collections::HashMap::new();
    };
    let data = match std::fs::read_to_string(session_index_path) {
        Ok(data) => data,
        Err(_) => return std::collections::HashMap::new(),
    };

    let mut thread_names = std::collections::HashMap::new();
    for line in data.lines() {
        let Ok(entry) = serde_json::from_str::<SessionIndexEntry>(line) else {
            continue;
        };
        let normalized_thread_name = entry.thread_name.trim();
        if entry.id.trim().is_empty() || normalized_thread_name.is_empty() {
            continue;
        }
        thread_names.insert(entry.id, normalized_thread_name.to_string());
    }

    thread_names
}

fn read_session_index_entry(root: &Path, session_id: &str) -> Option<SessionIndexEntry> {
    let Some(session_index_path) = session_index_path(root) else {
        return None;
    };
    let data = std::fs::read_to_string(session_index_path).ok()?;
    let mut latest_entry = None;

    for line in data.lines() {
        let Ok(entry) = serde_json::from_str::<SessionIndexEntry>(line) else {
            continue;
        };
        if entry.id == session_id && !entry.thread_name.trim().is_empty() {
            latest_entry = Some(entry);
        }
    }

    latest_entry
}

fn append_session_index_entry(
    root: &Path,
    session_id: &str,
    session_index_entry: Option<&Value>,
) -> Result<(), String> {
    let Some(session_index_entry) = session_index_entry.filter(|value| !value.is_null()) else {
        return Ok(());
    };

    let mut parsed_entry = serde_json::from_value::<SessionIndexEntry>(session_index_entry.clone())
        .map_err(|error| format!("Invalid Codex sessionIndexEntry: {error}"))?;
    if parsed_entry.id.trim().is_empty() {
        return Ok(());
    }
    if parsed_entry.id != session_id {
        return Err(format!(
            "Codex sessionIndexEntry id {} does not match session {}",
            parsed_entry.id, session_id
        ));
    }

    let normalized_thread_name = parsed_entry.thread_name.trim();
    if normalized_thread_name.is_empty() {
        return Ok(());
    }
    parsed_entry.thread_name = normalized_thread_name.to_string();

    let session_index_path = session_index_path(root)
        .ok_or_else(|| "Failed to determine Codex session index path".to_string())?;
    if let Some(parent_dir) = session_index_path.parent() {
        std::fs::create_dir_all(parent_dir).map_err(|error| {
            format!(
                "Failed to create Codex session index directory {}: {error}",
                parent_dir.display()
            )
        })?;
    }

    let mut serialized_entry = serde_json::to_string(&parsed_entry)
        .map_err(|error| format!("Failed to serialize Codex sessionIndexEntry: {error}"))?;
    serialized_entry.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session_index_path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, serialized_entry.as_bytes()))
        .map_err(|error| {
            format!(
                "Failed to write Codex session index {}: {error}",
                session_index_path.display()
            )
        })?;

    Ok(())
}

fn append_thread_name(root: &Path, session_id: &str, thread_name: &str) -> Result<(), String> {
    let normalized_thread_name = thread_name.trim();
    if normalized_thread_name.is_empty() {
        return Err("Session title cannot be empty".to_string());
    }

    let parsed_entry = SessionIndexEntry {
        id: session_id.to_string(),
        thread_name: normalized_thread_name.to_string(),
        updated_at: Utc::now().to_rfc3339(),
    };
    let session_index_path = session_index_path(root)
        .ok_or_else(|| "Failed to determine Codex session index path".to_string())?;
    if let Some(parent_dir) = session_index_path.parent() {
        std::fs::create_dir_all(parent_dir).map_err(|error| {
            format!(
                "Failed to create Codex session index directory {}: {error}",
                parent_dir.display()
            )
        })?;
    }

    let mut serialized_entry = serde_json::to_string(&parsed_entry)
        .map_err(|error| format!("Failed to serialize Codex sessionIndexEntry: {error}"))?;
    serialized_entry.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session_index_path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, serialized_entry.as_bytes()))
        .map_err(|error| {
            format!(
                "Failed to write Codex session index {}: {error}",
                session_index_path.display()
            )
        })?;

    Ok(())
}

const PARSE_HEAD_LINES: usize = 30;
const PARSE_TAIL_LINES: usize = 30;

fn read_rollout_text(path: &Path) -> Result<String, String> {
    if !codex_rollout::is_compressed_rollout_path(path) {
        return std::fs::read_to_string(path).map_err(|error| {
            format!(
                "Failed to read Codex session file {}: {error}",
                path.display()
            )
        });
    }

    let file = File::open(path).map_err(|error| {
        format!(
            "Failed to open Codex session file {}: {error}",
            path.display()
        )
    })?;
    let mut decoder = zstd::stream::read::Decoder::new(file).map_err(|error| {
        format!(
            "Failed to decode compressed Codex session file {}: {error}",
            path.display()
        )
    })?;
    let mut content = String::new();
    decoder.read_to_string(&mut content).map_err(|error| {
        format!(
            "Failed to read compressed Codex session file {}: {error}",
            path.display()
        )
    })?;
    Ok(content)
}

fn file_modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

/// Reads head/tail lines for session metadata parsing.
///
/// Compressed `.jsonl.zst` rollouts cannot seek to the tail without decoding the
/// entire file, so only a bounded head prefix is decoded and the file `mtime`
/// (preserved by Codex on compression) supplies `last_active_at`.
fn read_session_head_tail(path: &Path) -> Option<(Vec<String>, Vec<String>, Option<i64>)> {
    if !codex_rollout::is_compressed_rollout_path(path) {
        let (head, tail) = read_head_tail_lines(path, PARSE_HEAD_LINES, PARSE_TAIL_LINES).ok()?;
        return Some((head, tail, None));
    }

    let mut reader =
        codex_rollout::open_rollout_reader(path, Some(codex_rollout::HEAD_BYTE_LIMIT))?;
    let mut head = Vec::with_capacity(PARSE_HEAD_LINES);
    let mut buffer = Vec::new();
    let mut read_error = false;

    for _ in 0..PARSE_HEAD_LINES {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buffer);
                head.push(line.trim_end_matches(['\r', '\n']).to_string());
            }
            Err(_) => {
                read_error = true;
                break;
            }
        }
    }

    if head.is_empty() && read_error {
        return None;
    }

    Some((head, Vec::new(), file_modified_ms(path)))
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let (head, tail, fallback_last_active_at) = read_session_head_tail(path)?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut head_last_active_at: Option<i64> = None;
    let mut title: Option<String> = None;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if let Some(ts) = value.get("timestamp").and_then(parse_timestamp_to_ms) {
            created_at.get_or_insert(ts);
            head_last_active_at = Some(ts);
        }

        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(payload) = value.get("payload") {
                if session_id.is_none() {
                    session_id = payload
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|value| value.to_string());
                }
                if project_dir.is_none() {
                    project_dir = payload
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(|value| value.to_string());
                }
                if let Some(timestamp) = payload.get("timestamp").and_then(parse_timestamp_to_ms) {
                    created_at.get_or_insert(timestamp);
                    head_last_active_at.get_or_insert(timestamp);
                }
            }
        }

        if title.is_none() {
            title = extract_user_prompt_title(&value);
        }
    }

    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;

    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }

        if summary.is_none() && value.get("type").and_then(Value::as_str) == Some("response_item") {
            if let Some(payload) = value.get("payload") {
                if payload.get("type").and_then(Value::as_str) == Some("message") {
                    let text = payload.get("content").map(extract_text).unwrap_or_default();
                    if !text.trim().is_empty() {
                        summary = Some(text);
                    }
                }
            }
        }

        if last_active_at.is_some() && summary.is_some() {
            break;
        }
    }

    if last_active_at.is_none() && codex_rollout::is_compressed_rollout_path(path) {
        last_active_at = fallback_last_active_at.or(head_last_active_at);
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path))?;
    let title = title.or_else(|| {
        project_dir
            .as_deref()
            .and_then(path_basename)
            .map(|value| value.to_string())
    });
    let resume_command = build_resume_command(
        project_dir.as_deref(),
        &format!("codex resume {session_id}"),
    );

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary: summary.map(|text| truncate_summary(&text, 160)),
        project_dir,
        created_at,
        last_active_at,
        source_path: path.to_string_lossy().to_string(),
        resume_command: Some(resume_command),
        runtime_source: None,
        runtime_distro: None,
    })
}

fn extract_user_prompt_title(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }

    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }

    let text = payload.get("content").map(extract_text).unwrap_or_default();
    normalize_user_prompt_title(&text)
}

fn normalize_user_prompt_title(text: &str) -> Option<String> {
    extract_prompt_title_text(text, 80)
}

fn infer_session_id_from_filename(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy();
    UUID_RE
        .find(&file_name)
        .map(|matched| matched.as_str().to_string())
}

fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }

    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files);
        } else if is_rollout_artifact_file(&path) {
            files.push(path);
        }
    }
}

/// Drops `<name>.jsonl.zst` when its plain `<name>.jsonl` sibling is also present,
/// preserving the original order of the kept paths.
fn dedupe_rollout_siblings(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let plain_paths: HashSet<PathBuf> = files
        .iter()
        .filter(|path| !codex_rollout::is_compressed_rollout_path(path))
        .cloned()
        .collect();

    if plain_paths.is_empty() {
        return files;
    }

    files
        .into_iter()
        .filter(|path| {
            if !codex_rollout::is_compressed_rollout_path(path) {
                return true;
            }
            let plain_sibling = path.with_extension("");
            !plain_paths.contains(&plain_sibling)
        })
        .collect()
}

fn find_sessions_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some("sessions"))
        .map(Path::to_path_buf)
}

fn session_index_path(root: &Path) -> Option<PathBuf> {
    root.parent()
        .map(|codex_home| codex_home.join(SESSION_INDEX_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::{delete_session, load_messages, scan_messages_for_query, scan_sessions};

    use std::fs;
    use std::path::{Path, PathBuf};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ai-toolbox-codex-session-{label}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&path).expect("failed to create test directory");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_session_meta(path: &Path, session_id: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create session directory");
        }
        fs::write(
            path,
            serde_json::json!({
                "timestamp": "2026-09-17T10:00:00Z",
                "type": "session_meta",
                "payload": { "id": session_id, "cwd": "/tmp/project" }
            })
            .to_string(),
        )
        .expect("failed to write session file");
    }

    /// Codex keeps one thread in several rollout files across dated directories
    /// (a fresh file per resume, all carrying the thread id). Deleting the row the
    /// list shows must take every artifact of that session, or the older rollouts
    /// just reappear as the same session.
    #[test]
    fn delete_session_removes_every_rollout_of_one_thread() {
        let test_dir = TestDir::new("delete-shared-thread");
        let sessions_root = test_dir.path().join("sessions");
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";
        let other_id = "01a08e11-2c7d-7b55-8e10-4a9c77d20453";

        let superseded = sessions_root
            .join("2026")
            .join("09")
            .join("11")
            .join(format!("rollout-2026-09-11T10-00-00-{thread_id}.jsonl"));
        let live = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!("rollout-2026-09-17T09-00-00-{thread_id}.jsonl"));
        let unrelated = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!("rollout-2026-09-17T10-00-00-{other_id}.jsonl"));

        write_session_meta(&superseded, thread_id);
        write_session_meta(&live, thread_id);
        write_session_meta(&unrelated, other_id);

        // Any artifact is a valid handle: deleting through the superseded file
        // removes the live one too.
        super::delete_session(&superseded).expect("delete should succeed");

        assert!(!superseded.exists());
        assert!(!live.exists());
        assert!(unrelated.exists());

        // Repeated deletion converges instead of failing.
        super::delete_session(&live).expect("repeated delete should converge");
    }

    #[test]
    fn load_messages_keeps_initial_developer_input_text() {
        let test_dir = TestDir::new("developer-input-text");
        let session_path = test_dir.path().join("rollout.jsonl");
        let content = [
            serde_json::json!({
                "timestamp": "2026-06-07T09:11:00Z",
                "type": "response_item",
                "payload": {
                    "id": "dev-1",
                    "type": "message",
                    "role": "developer",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "Filesystem sandboxing defines which files can be read or written."
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:11:01Z",
                "type": "response_item",
                "payload": {
                    "id": "dev-2",
                    "type": "message",
                    "role": "developer",
                    "content": [
                        {
                            "type": "input_text",
                            "text": r#"# AGENTS.md instructions for D:\GitHub\sample-workspace"#
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:11:02Z",
                "type": "response_item",
                "payload": {
                    "id": "user-1",
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": r#"Inspect D:\GitHub\sample-workspace source"#
                        }
                    ]
                }
            })
            .to_string(),
        ]
        .join("\n");
        fs::write(&session_path, content).expect("failed to write Codex session");

        let messages = load_messages(&session_path).expect("load Codex messages");

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "developer");
        assert!(messages[0]
            .content
            .contains("Filesystem sandboxing defines"));
        assert_eq!(messages[1].role, "developer");
        assert!(messages[1].content.contains("AGENTS.md instructions"));
        assert_eq!(messages[2].role, "user");
        assert!(messages[2]
            .content
            .contains(r#"D:\GitHub\sample-workspace"#));
    }

    /// A `session_meta` record in the shape Codex writes.
    fn session_meta_record(
        thread_id: &str,
        history_mode: &str,
        history_base: Option<serde_json::Value>,
    ) -> String {
        let mut payload = serde_json::json!({
            "id": thread_id,
            "cwd": "/tmp/project",
            "history_mode": history_mode,
        });
        if let Some(base) = history_base {
            payload["history_base"] = base;
        }
        serde_json::json!({
            "timestamp": "2026-09-17T10:00:00Z",
            "type": "session_meta",
            "payload": payload,
        })
        .to_string()
    }

    /// A `response_item` user message, which `load_messages` surfaces as text.
    fn user_record(text: &str) -> String {
        serde_json::json!({
            "timestamp": "2026-09-17T10:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": text }],
            }
        })
        .to_string()
    }

    /// Writes a rollout from pre-built records. Returns the byte offset after each
    /// record, which is the value a child records as its cutoff.
    fn write_lineage_rollout(path: &Path, lines: &[String]) -> Vec<u64> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create rollout directory");
        }
        let mut content = String::new();
        let mut offsets = Vec::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
            offsets.push(content.len() as u64);
        }
        fs::write(path, content).expect("failed to write rollout");
        offsets
    }

    /// Writes a rollout in Codex's cold representation (`<name>.jsonl.zst`).
    ///
    /// The byte offsets a child records stay plain-text offsets, so the same
    /// records can be written compressed and still honor a lineage cutoff.
    fn write_compressed_rollout(path: &Path, lines: &[String]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create rollout directory");
        }
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        let compressed =
            zstd::stream::encode_all(content.as_bytes(), 3).expect("failed to compress rollout");
        fs::write(path, compressed).expect("failed to write compressed rollout");
    }

    fn codex_lineage_fixture(test_dir: &TestDir) -> (PathBuf, PathBuf, PathBuf) {
        let sessions_root = test_dir.path().join("sessions");
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";
        let child_rollout_id = "01a08e80-1111-7c31-9a20-6d3f11b91882";
        let prefix = sessions_root
            .join("2026")
            .join("09")
            .join("11")
            .join(format!("rollout-2026-09-11T10-00-00-{thread_id}.jsonl"));
        let child = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{thread_id}_{child_rollout_id}.jsonl"
            ));
        (sessions_root, prefix, child)
    }

    /// A `paginated` thread keeps its earlier records in prefix rollouts and points
    /// at them with `history_base` instead of copying them, so reading only the
    /// file the client named would show one segment of the conversation.
    #[test]
    fn load_messages_stitches_a_paginated_lineage() {
        let test_dir = TestDir::new("lineage-stitch");
        let (_sessions_root, prefix, child) = codex_lineage_fixture(&test_dir);
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";

        let offsets = write_lineage_rollout(
            &prefix,
            &[
                session_meta_record(thread_id, "paginated", None),
                user_record("earlier turn"),
            ],
        );
        write_lineage_rollout(
            &child,
            &[
                session_meta_record(
                    thread_id,
                    "paginated",
                    Some(serde_json::json!({
                        "thread_id": thread_id,
                        "end_ordinal_exclusive": 2,
                        "end_byte_offset": offsets[1],
                    })),
                ),
                user_record("later turn"),
            ],
        );

        let messages = load_messages(&child).expect("load Codex messages");

        let texts: Vec<&str> = messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(texts, vec!["earlier turn", "later turn"]);
    }

    /// Only `paginated` threads keep rollouts. A `legacy` child must not pull in
    /// another file's records even when both carry the same thread id.
    #[test]
    fn load_messages_does_not_stitch_a_legacy_thread() {
        let test_dir = TestDir::new("lineage-legacy");
        let (_sessions_root, prefix, child) = codex_lineage_fixture(&test_dir);
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";

        write_lineage_rollout(
            &prefix,
            &[
                session_meta_record(thread_id, "legacy", None),
                user_record("stale record"),
            ],
        );
        write_lineage_rollout(
            &child,
            &[
                session_meta_record(thread_id, "legacy", None),
                user_record("current record"),
            ],
        );

        let messages = load_messages(&child).expect("load Codex messages");

        let texts: Vec<&str> = messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(texts, vec!["current record"]);
    }

    /// The cutoff is "immediately after the last included prefix record", so a
    /// record the child did not depend on must not be replayed.
    #[test]
    fn load_messages_honors_a_segment_cutoff() {
        let test_dir = TestDir::new("lineage-cutoff");
        let (_sessions_root, prefix, child) = codex_lineage_fixture(&test_dir);
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";

        let offsets = write_lineage_rollout(
            &prefix,
            &[
                session_meta_record(thread_id, "paginated", None),
                user_record("included record"),
                user_record("record past the cutoff"),
            ],
        );
        write_lineage_rollout(
            &child,
            &[
                session_meta_record(
                    thread_id,
                    "paginated",
                    Some(serde_json::json!({
                        "thread_id": thread_id,
                        "end_ordinal_exclusive": 2,
                        // Right after "included record".
                        "end_byte_offset": offsets[1],
                    })),
                ),
                user_record("child record"),
            ],
        );

        let messages = load_messages(&child).expect("load Codex messages");

        let texts: Vec<&str> = messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(texts, vec!["included record", "child record"]);
    }

    /// Search must reach the prefix segments too, or a query would miss most of a
    /// paginated thread's history.
    #[test]
    fn scan_messages_for_query_reaches_prefix_segments() {
        let test_dir = TestDir::new("lineage-search");
        let (_sessions_root, prefix, child) = codex_lineage_fixture(&test_dir);
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";

        let offsets = write_lineage_rollout(
            &prefix,
            &[
                session_meta_record(thread_id, "paginated", None),
                user_record("the needle lives here"),
            ],
        );
        write_lineage_rollout(
            &child,
            &[
                session_meta_record(
                    thread_id,
                    "paginated",
                    Some(serde_json::json!({
                        "thread_id": thread_id,
                        "end_ordinal_exclusive": 2,
                        "end_byte_offset": offsets[1],
                    })),
                ),
                user_record("nothing relevant here"),
            ],
        );

        assert!(
            scan_messages_for_query(&child, "needle").expect("search should run"),
            "a prefix segment's records are part of the session"
        );
        assert!(!scan_messages_for_query(&child, "absent").expect("search should run"));
    }

    /// Another thread can keep its history in a rollout we are about to delete.
    /// Removing it would break that thread's replay, so the delete is refused.
    #[test]
    fn delete_session_refuses_when_another_thread_references_a_rollout() {
        let test_dir = TestDir::new("delete-external-reference");
        let sessions_root = test_dir.path().join("sessions");
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";
        let other_id = "01a08e11-2c7d-7b55-8e10-4a9c77d20453";

        let target = sessions_root
            .join("2026")
            .join("09")
            .join("11")
            .join(format!("rollout-2026-09-11T10-00-00-{thread_id}.jsonl"));
        // The other thread's rollout names ours as its history base.
        let dependent = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!("rollout-2026-09-17T09-00-00-{other_id}.jsonl"));

        write_lineage_rollout(
            &target,
            &[
                session_meta_record(thread_id, "paginated", None),
                user_record("history another thread depends on"),
            ],
        );
        write_lineage_rollout(
            &dependent,
            &[
                session_meta_record(
                    other_id,
                    "paginated",
                    Some(serde_json::json!({
                        "thread_id": thread_id,
                        "end_ordinal_exclusive": 2,
                        "end_byte_offset": 128,
                    })),
                ),
                user_record("dependent thread record"),
            ],
        );

        let error = delete_session(&target).expect_err("delete should be refused");
        assert!(
            error.contains("still used as history"),
            "unexpected error: {error}"
        );
        assert!(
            target.exists(),
            "a refused delete must leave the rollout in place"
        );
    }

    /// Codex archives a thread by moving its rollouts to `archived_sessions/`
    /// and compresses cold ones to `<name>.jsonl.zst`. Deleting the row the list
    /// shows must take those copies too, or the archived thread keeps the
    /// session alive and it reappears on the next resume.
    #[test]
    fn delete_session_removes_archived_and_compressed_copies() {
        let test_dir = TestDir::new("delete-archived-copies");
        let codex_home = test_dir.path();
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";

        let live = codex_home
            .join("sessions")
            .join("2026")
            .join("09")
            .join("17")
            .join(format!("rollout-2026-09-17T09-00-00-{thread_id}.jsonl"));
        write_lineage_rollout(&live, &[session_meta_record(thread_id, "legacy", None)]);

        let archived_plain = codex_home
            .join("archived_sessions")
            .join("2026")
            .join("09")
            .join("11")
            .join(format!("rollout-2026-09-11T10-00-00-{thread_id}.jsonl"));
        write_lineage_rollout(
            &archived_plain,
            &[session_meta_record(thread_id, "legacy", None)],
        );

        let archived_compressed = codex_home
            .join("archived_sessions")
            .join(format!("rollout-2026-09-10T10-00-00-{thread_id}.jsonl.zst"));
        write_compressed_rollout(
            &archived_compressed,
            &[session_meta_record(thread_id, "legacy", None)],
        );

        delete_session(&live).expect("delete should succeed");

        assert!(!live.exists());
        assert!(!archived_plain.exists(), "the archived copy must go too");
        assert!(
            !archived_compressed.exists(),
            "the compressed archived copy must go too"
        );
    }

    /// A compressed rollout still references its prefix — Codex reads it through
    /// a decoder, so the delete guard has to as well.
    #[test]
    fn delete_session_refuses_when_a_compressed_rollout_references_a_target() {
        let test_dir = TestDir::new("delete-compressed-reference");
        let codex_home = test_dir.path();
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";
        let other_id = "01a08e11-2c7d-7b55-8e10-4a9c77d20453";

        let target = codex_home
            .join("sessions")
            .join("2026")
            .join("09")
            .join("11")
            .join(format!("rollout-2026-09-11T10-00-00-{thread_id}.jsonl"));
        write_lineage_rollout(
            &target,
            &[
                session_meta_record(thread_id, "paginated", None),
                user_record("history another thread depends on"),
            ],
        );

        let dependent = codex_home
            .join("archived_sessions")
            .join(format!("rollout-2026-09-17T09-00-00-{other_id}.jsonl.zst"));
        write_compressed_rollout(
            &dependent,
            &[
                session_meta_record(
                    other_id,
                    "paginated",
                    Some(serde_json::json!({
                        "thread_id": thread_id,
                        "end_ordinal_exclusive": 2,
                        "end_byte_offset": 128,
                    })),
                ),
                user_record("dependent thread record"),
            ],
        );

        let error = delete_session(&target).expect_err("delete should be refused");
        assert!(
            error.contains("still used as history"),
            "unexpected error: {error}"
        );
        assert!(
            target.exists(),
            "a refused delete must leave the rollout in place"
        );
    }

    /// A prefix rollout Codex already compressed is still part of the lineage:
    /// the decoded stream honors the same plain-text cutoff.
    #[test]
    fn load_messages_stitches_a_compressed_paginated_lineage() {
        let test_dir = TestDir::new("lineage-compressed-prefix");
        let (_sessions_root, prefix, child) = codex_lineage_fixture(&test_dir);
        let thread_id = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";

        let prefix_records = [
            session_meta_record(thread_id, "paginated", None),
            user_record("earlier turn"),
        ];
        let mut plain = String::new();
        for record in &prefix_records {
            plain.push_str(record);
            plain.push('\n');
        }
        let cutoff = plain.len() as u64;
        // Only the compressed representation exists on disk, as it is after a
        // cold-rollout compression pass.
        write_compressed_rollout(&prefix.with_extension("jsonl.zst"), &prefix_records);

        write_lineage_rollout(
            &child,
            &[
                session_meta_record(
                    thread_id,
                    "paginated",
                    Some(serde_json::json!({
                        "thread_id": thread_id,
                        "end_ordinal_exclusive": 2,
                        "end_byte_offset": cutoff,
                    })),
                ),
                user_record("later turn"),
            ],
        );

        let messages = load_messages(&child).expect("load Codex messages");
        let texts: Vec<&str> = messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(texts, vec!["earlier turn", "later turn"]);
    }

    #[test]
    fn load_messages_structures_codex_tools_reasoning_and_events() {
        let test_dir = TestDir::new("structured-events");
        let session_path = test_dir.path().join("rollout.jsonl");
        let content = [
            serde_json::json!({
                "timestamp": "2026-06-07T09:12:00Z",
                "type": "turn_context",
                "payload": {
                    "model": "gpt-5"
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:12:01Z",
                "type": "response_item",
                "payload": {
                    "id": "reasoning-1",
                    "type": "reasoning",
                    "summary": [
                        { "text": "Need to inspect the command output." }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:12:02Z",
                "type": "response_item",
                "payload": {
                    "id": "shell-1",
                    "type": "local_shell_call",
                    "call_id": "tool_shell_1",
                    "action": {
                        "command": ["rg", "needle"]
                    }
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:12:03Z",
                "type": "response_item",
                "payload": {
                    "id": "shell-output-1",
                    "type": "function_call_output",
                    "call_id": "tool_shell_1",
                    "output": "{\"output\":\"done\"}"
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:12:04Z",
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 100,
                            "output_tokens": 20,
                            "cached_input_tokens": 10
                        }
                    }
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:12:05Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_started",
                    "turn_id": "turn-1"
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:12:06Z",
                "type": "compacted",
                "payload": {
                    "replacement_history": [{ "id": "older" }]
                }
            })
            .to_string(),
        ]
        .join("\n");
        fs::write(&session_path, content).expect("failed to write Codex session");

        let messages = load_messages(&session_path).expect("load Codex messages");

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].blocks[0].kind, "thinking");
        assert_eq!(messages[0].model.as_deref(), Some("gpt-5"));

        let shell_block = &messages[1].blocks[0];
        assert_eq!(shell_block.kind, "tool_call");
        assert_eq!(shell_block.tool_name.as_deref(), Some("Bash"));
        assert_eq!(shell_block.normalized_tool_name.as_deref(), Some("bash"));
        assert_eq!(
            shell_block
                .input
                .as_ref()
                .and_then(|input| input.get("command"))
                .and_then(serde_json::Value::as_str),
            Some("rg needle")
        );
        assert_eq!(
            messages[1]
                .usage
                .as_ref()
                .and_then(|usage| usage.input_tokens),
            Some(90)
        );
        assert_eq!(
            messages[1]
                .usage
                .as_ref()
                .and_then(|usage| usage.cache_read_input_tokens),
            Some(10)
        );

        let result_block = &messages[2].blocks[0];
        assert_eq!(result_block.kind, "tool_result");
        assert_eq!(
            result_block
                .output
                .as_ref()
                .and_then(serde_json::Value::as_str),
            Some("done")
        );

        assert_eq!(messages[3].role, "system");
        assert_eq!(messages[3].message_type.as_deref(), Some("task_started"));
        assert!(messages[3].content.contains("[Task Started]"));

        assert_eq!(messages[4].role, "system");
        assert_eq!(messages[4].content, "Conversation compacted");
        assert_eq!(
            messages[4]
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("replacementHistoryCount"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn load_messages_ignores_duplicate_codex_reasoning_event() {
        let test_dir = TestDir::new("duplicate-reasoning-event");
        let session_path = test_dir.path().join("rollout.jsonl");
        let content = [
            serde_json::json!({
                "timestamp": "2026-06-07T09:13:00Z",
                "type": "event_msg",
                "payload": {
                    "type": "agent_reasoning",
                    "text": "Need to inspect the command output."
                }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-06-07T09:13:01Z",
                "type": "response_item",
                "payload": {
                    "id": "reasoning-1",
                    "type": "reasoning",
                    "summary": [
                        { "text": "Need to inspect the command output." }
                    ]
                }
            })
            .to_string(),
        ]
        .join("\n");
        fs::write(&session_path, content).expect("failed to write Codex session");

        let messages = load_messages(&session_path).expect("load Codex messages");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].blocks[0].kind, "thinking");
        assert_eq!(messages[0].content, "Need to inspect the command output.");
        assert_eq!(messages[0].id.as_deref(), Some("reasoning-1"));
    }

    #[test]
    fn scan_sessions_prefixes_resume_with_project_directory() {
        let test_dir = TestDir::new("resume-project-dir");
        let sessions_root = test_dir.path().join("sessions");
        let session_id = "11111111-2222-3333-4444-555555555555";
        let session_path = sessions_root
            .join("2026")
            .join("05")
            .join("16")
            .join(format!("rollout-2026-05-16T10-00-00-{session_id}.jsonl"));
        if let Some(parent) = session_path.parent() {
            fs::create_dir_all(parent).expect("failed to create session parent");
        }
        fs::write(
            &session_path,
            serde_json::json!({
                "timestamp": "2026-05-16T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": session_id,
                    "timestamp": "2026-05-16T10:00:00Z",
                    "cwd": "D:/GitHub/project with space"
                }
            })
            .to_string(),
        )
        .expect("failed to write session");

        let sessions = scan_sessions(&sessions_root);

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].resume_command.as_deref(),
            Some("pushd \"D:/GitHub/project with space\" && codex resume 11111111-2222-3333-4444-555555555555")
        );
    }

    #[test]
    fn scan_sessions_and_recent_discover_compressed_rollouts_and_dedupe_siblings() {
        let test_dir = TestDir::new("compressed-scan");
        let sessions_root = test_dir.path().join("sessions");
        let compressed_id = "22222222-3333-4444-5555-666666666666";
        let sibling_id = "33333333-4444-5555-6666-777777777777";
        let corrupt_id = "44444444-5555-6666-7777-888888888888";

        let compressed_path = sessions_root
            .join("2026")
            .join("11")
            .join("01")
            .join(format!(
                "rollout-2026-11-01T10-00-00-{compressed_id}.jsonl.zst"
            ));
        write_compressed_rollout(
            &compressed_path,
            &[
                session_meta_record(compressed_id, "legacy", None),
                user_record("Compressed rollout question"),
            ],
        );

        let sibling_plain = sessions_root
            .join("2026")
            .join("11")
            .join("02")
            .join(format!("rollout-2026-11-02T10-00-00-{sibling_id}.jsonl"));
        let sibling_compressed = sessions_root
            .join("2026")
            .join("11")
            .join("02")
            .join(format!(
                "rollout-2026-11-02T10-00-00-{sibling_id}.jsonl.zst"
            ));
        write_lineage_rollout(
            &sibling_plain,
            &[
                session_meta_record(sibling_id, "legacy", None),
                user_record("Plain sibling wins"),
            ],
        );
        write_compressed_rollout(
            &sibling_compressed,
            &[
                session_meta_record(sibling_id, "legacy", None),
                user_record("Compressed sibling should be skipped"),
            ],
        );

        let corrupt_path = sessions_root
            .join("2026")
            .join("11")
            .join("03")
            .join(format!(
                "rollout-2026-11-03T10-00-00-{corrupt_id}.jsonl.zst"
            ));
        fs::create_dir_all(corrupt_path.parent().unwrap()).expect("create corrupt dir");
        fs::write(&corrupt_path, b"not-valid-zstd-bytes").expect("write corrupt zst");

        let sessions = scan_sessions(&sessions_root);
        assert_eq!(sessions.len(), 2);

        let compressed_meta = sessions
            .iter()
            .find(|session| session.session_id == compressed_id)
            .expect("compressed rollout should be listed");
        assert_eq!(
            compressed_meta.source_path,
            compressed_path.to_string_lossy()
        );
        assert_eq!(
            compressed_meta.title.as_deref(),
            Some("Compressed rollout question")
        );
        assert_eq!(compressed_meta.project_dir.as_deref(), Some("/tmp/project"));
        assert!(compressed_meta.created_at.is_some());
        assert!(compressed_meta.last_active_at.is_some());

        let sibling_meta = sessions
            .iter()
            .find(|session| session.session_id == sibling_id)
            .expect("sibling rollout should be listed once");
        assert_eq!(sibling_meta.source_path, sibling_plain.to_string_lossy());
        assert_eq!(sibling_meta.title.as_deref(), Some("Plain sibling wins"));

        let recent = super::scan_recent_sessions(&sessions_root, 10);
        assert_eq!(recent.len(), 2);
        assert!(recent
            .iter()
            .any(|session| session.session_id == compressed_id));
        assert!(recent
            .iter()
            .any(|session| session.source_path == sibling_plain.to_string_lossy()));
    }

    #[test]
    fn compressed_rollout_supports_messages_search_rename_and_snapshot_roundtrip() {
        let test_dir = TestDir::new("compressed-roundtrip");
        let sessions_root = test_dir.path().join("sessions");
        let session_id = "55555555-6666-7777-8888-999999999999";
        let compressed_path = sessions_root
            .join("2026")
            .join("11")
            .join("05")
            .join(format!(
                "rollout-2026-11-05T10-00-00-{session_id}.jsonl.zst"
            ));
        write_compressed_rollout(
            &compressed_path,
            &[
                session_meta_record(session_id, "legacy", None),
                user_record("Find the zstd needle in this session"),
            ],
        );

        let messages = load_messages(&compressed_path).expect("load compressed messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Find the zstd needle in this session");

        assert!(scan_messages_for_query(&compressed_path, "zstd needle")
            .expect("query compressed rollout"));

        super::rename_session(
            compressed_path.to_string_lossy().as_ref(),
            "Renamed Compressed Thread",
        )
        .expect("rename compressed rollout");
        let sessions = scan_sessions(&sessions_root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].title.as_deref(),
            Some("Renamed Compressed Thread")
        );

        let snapshot = super::export_native_snapshot(&sessions_root, &compressed_path)
            .expect("export compressed rollout snapshot");
        assert_eq!(
            snapshot
                .get("relativeSessionPath")
                .and_then(serde_json::Value::as_str),
            Some(
                "2026/11/05/rollout-2026-11-05T10-00-00-55555555-6666-7777-8888-999999999999.jsonl"
            )
        );
        assert!(snapshot
            .get("sessionFileContent")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|content| content.contains("Find the zstd needle in this session")));

        let import_dir = TestDir::new("compressed-import");
        let import_sessions_root = import_dir.path().join("sessions");
        let imported_path =
            super::import_native_snapshot(&import_sessions_root, session_id, &snapshot)
                .expect("import plain snapshot exported from compressed rollout");
        assert_eq!(
            imported_path.extension().and_then(|ext| ext.to_str()),
            Some("jsonl")
        );
        let imported_sessions = scan_sessions(&import_sessions_root);
        assert_eq!(imported_sessions.len(), 1);
        assert_eq!(
            imported_sessions[0].title.as_deref(),
            Some("Renamed Compressed Thread")
        );
        let imported_messages = load_messages(&imported_path).expect("load imported messages");
        assert_eq!(imported_messages.len(), 1);
        assert_eq!(
            imported_messages[0].content,
            "Find the zstd needle in this session"
        );
    }
}
