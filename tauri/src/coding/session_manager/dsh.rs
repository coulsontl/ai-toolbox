//! Dsh session scanning/loading.
//!
//! dsh persists every session as a versioned JSONL artifact under its sessions
//! root (`<home>/sessions`):
//!   `<home>/sessions/<project-key>/<encoded-session-id>/session[.v<N>].jsonl[.zstd]`
//!
//! One session directory may hold several format generations side by side; the
//! numerically highest is the live one. That naming rule is owned by
//! `crate::coding::dsh::session_artifact` and must not be re-implemented here.
//!
//! The first line is a `{type:"session", version, id, createdAt, cwd...}`
//! header; every following line is a `StorageRecord` — either a full
//! `SessionEvent` (`{type, seq, time, data, surfaceOp?}`) or a packed chunk
//! row (`text-chunks` / `reasoning-chunks` / `tool-call-chunks`) that only
//! carries streaming deltas. zstd frames are the default encoding, so both
//! `.jsonl.zstd` and plain `.jsonl` artifacts are covered. All read paths
//! degrade silently to an empty list when the data is unreachable.
//!
//! Transcripts follow dsh's append-only surface semantics: a `user/message`
//! event only surfaces a real user prompt when `data.source.kind` is `user`
//! (injected context arrives as `agent-instructions` / `plugin` /
//! `skill-catalog` kinds); assistant reasoning renders as thinking blocks;
//! `tool/call` records pair with their `tool/result` through `callId` into
//! tool-execution cards. Events whose `surfaceOp` replaces earlier surface
//! nodes (compaction copies) stay model-only and are skipped.
//!
//! Limitations: the list title is the last `session/title` event inside the
//! bounded head window, falling back to the first user message when the log
//! carries none; a rename that lands deeper than that window is not picked up
//! (zstd frames offer no cheap read from the tail, so the head window is a
//! deliberate cost bound). Last-active is approximated by the artifact's file
//! mtime, and the tail timestamp is not read for zstd artifacts.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::message_blocks::{
    message_from_blocks, text_block, text_message, thinking_block, tool_call_block,
    tool_result_block, usage_from_value,
};
use super::utils::{extract_text, parse_timestamp_to_ms, text_contains_query, truncate_summary};
use super::{assign_missing_message_ids, SessionMessage, SessionMessageUsage, SessionMeta};
use crate::coding::dsh::session_artifact;

const PROVIDER_ID: &str = "dsh";
const TITLE_MAX_CHARS: usize = 80;
// Bound the scan pass to the header plus a handful of early events; titles and
// creation metadata never sit deeper than a session's opening events.
const HEAD_LINES: usize = 40;

/// A `tool/call` record awaiting its `tool/result`.
struct PendingToolCall {
    call_id: String,
    name: String,
    arguments: Option<Value>,
}

/// Open a session artifact for buffered text reading, decompressing zstd when
/// the file is a `.zstd` artifact.
fn open_session_reader(path: &Path) -> std::io::Result<Box<dyn BufRead>> {
    let file = File::open(path)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name.ends_with(".zstd") {
        let decoder = zstd::stream::read::Decoder::new(file)?;
        Ok(Box::new(BufReader::new(decoder)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

/// Read up to `max_lines` text lines from an artifact (header plus early events).
fn read_head_lines(path: &Path, max_lines: usize) -> std::io::Result<Vec<String>> {
    let mut reader = open_session_reader(path)?;
    let mut lines = Vec::new();
    let mut line = String::new();
    while lines.len() < max_lines {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        lines.push(line.clone());
    }
    Ok(lines)
}

/// Recursively collect the live session artifact of every session under `root`.
///
/// A session directory can hold several format generations side by side, so the
/// candidates are reduced to the highest generation per directory.
fn collect_session_artifacts(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if session_artifact::is_session_artifact(&path) {
                files.push(path);
            }
        }
    }
    session_artifact::select_generations(files)
}

fn file_modified_ms(path: &Path) -> Option<i64> {
    let modified = path.metadata().ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as i64,
    )
}

/// Whether a surface event entered the ordered surface as an append — the
/// human-transcript source material. Replacement copies (compaction) are
/// model-only; a missing marker degrades to append for leniency.
fn is_append_surface_op(surface_op: Option<&Value>) -> bool {
    match surface_op {
        None => true,
        Some(Value::String(op)) => op == "append",
        Some(_) => false,
    }
}

/// Event payload of an append-origin surface event of the given type.
fn append_event_data<'a>(event: &'a Value, event_type: &str) -> Option<&'a Value> {
    if event.get("type").and_then(Value::as_str) != Some(event_type) {
        return None;
    }
    if !is_append_surface_op(event.get("surfaceOp")) {
        return None;
    }
    event.get("data")
}

fn event_time_ms(event: &Value) -> Option<i64> {
    event.get("time").and_then(parse_timestamp_to_ms)
}

/// Surface text of a real user prompt: an append-origin `user/message` whose
/// `source.kind` is `user`. Injected context (agent-instructions, plugin,
/// skill-catalog) uses other kinds and never surfaces.
fn user_prompt_text(event: &Value) -> Option<String> {
    let data = append_event_data(event, "user/message")?;
    let kind = data.pointer("/source/kind").and_then(Value::as_str)?;
    if kind != "user" {
        return None;
    }
    let text = data.get("content").map(extract_text)?;
    if text.trim().is_empty() {
        return None;
    }
    Some(text)
}

/// Map dsh token-usage field names onto the shared usage reader.
fn dsh_usage_from_value(usage: &Value) -> Option<SessionMessageUsage> {
    let mut mapped = usage.clone();
    if let Some(fields) = mapped.as_object_mut() {
        if let Some(cache_read) = fields.remove("cacheReadTokens") {
            fields.insert("cacheReadInputTokens".to_string(), cache_read);
        }
    }
    usage_from_value(&mapped)
}

/// Build the assistant transcript message from an `assistant/message` event:
/// reasoning becomes a thinking block, text passes through, and `tool-call`
/// entries are skipped (the paired `tool/call` + `tool/result` events carry
/// the same invocations with richer pairing data).
fn assistant_message_from_event(event: &Value) -> Option<SessionMessage> {
    let data = append_event_data(event, "assistant/message")?;
    let message = data
        .get("message")
        .filter(|message| message.is_object())
        .unwrap_or(data);

    let mut blocks = Vec::new();
    if let Some(items) = message.get("content").and_then(Value::as_array) {
        for item in items {
            let text = match item.get("type").and_then(Value::as_str).unwrap_or("") {
                "reasoning" | "text" => item.get("text").and_then(Value::as_str),
                _ => None,
            };
            if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
                if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    blocks.push(thinking_block(text));
                } else {
                    blocks.push(text_block(text));
                }
            }
        }
    }
    if blocks.is_empty() {
        // An empty-content assistant/message only hosts step usage.
        return None;
    }

    let mut session_message = message_from_blocks("assistant", event_time_ms(event), blocks);
    session_message.model = message
        .pointer("/source/model")
        .and_then(Value::as_str)
        .map(str::to_string);
    session_message.usage = data.get("usage").and_then(dsh_usage_from_value);
    Some(session_message)
}

/// Record a model-requested tool invocation until its result arrives.
fn pending_tool_call_from_event(event: &Value) -> Option<PendingToolCall> {
    let data = event.get("data")?;
    Some(PendingToolCall {
        call_id: data.get("callId").and_then(Value::as_str)?.to_string(),
        name: data
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        arguments: data.get("arguments").and_then(parse_tool_arguments),
    })
}

/// Tool arguments arrive as the raw JSON string exactly as the model produced
/// it; keep unparsable payloads verbatim instead of dropping them.
fn parse_tool_arguments(arguments: &Value) -> Option<Value> {
    match arguments {
        Value::Null => None,
        Value::String(raw) => {
            Some(serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone())))
        }
        parsed => Some(parsed.clone()),
    }
}

/// Extract `(callId, output)` from a `tool/result` event's payload. A multipart
/// `content` array contributes every `tool-result` entry's text in order
/// (joined by a blank line); entries of other types are ignored.
fn tool_result_fields(event: &Value) -> Option<(String, Value)> {
    let data = append_event_data(event, "tool/result")?;
    let message = data.get("message")?;
    let content = message.get("content")?.as_array()?;
    let mut call_id: Option<String> = None;
    let mut parts: Vec<String> = Vec::new();
    for part in content {
        if part.get("type").and_then(Value::as_str) != Some("tool-result") {
            continue;
        }
        if call_id.is_none() {
            call_id = part
                .get("toolCallId")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        parts.push(part.get("content").map(extract_text).unwrap_or_default());
    }
    let call_id = call_id?;
    Some((call_id, Value::String(parts.join("\n\n"))))
}

/// Build one tool-execution message by pairing a `tool/result` with its
/// recorded `tool/call`; unpaired results degrade to a bare result block.
fn tool_execution_message(
    call: Option<&PendingToolCall>,
    call_id: String,
    output: Value,
    is_error: bool,
    ts: Option<i64>,
) -> SessionMessage {
    let mut blocks = Vec::new();
    if let Some(call) = call {
        blocks.push(tool_call_block(
            Some(call.call_id.clone()),
            call.name.clone(),
            call.arguments.clone(),
        ));
    }
    blocks.push(tool_result_block(
        Some(call_id),
        call.map(|call| call.name.clone()),
        Some(output),
        Some(is_error),
    ));
    message_from_blocks("tool", ts, blocks)
}

/// Replay a dsh session artifact into surface transcript messages.
fn load_transcript(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let mut reader = open_session_reader(path).map_err(|error| {
        format!(
            "Failed to open dsh session file {}: {error}",
            path.display()
        )
    })?;

    // A tool/call immediately emits a placeholder message at its file position;
    // the matching tool/result later merges its block back into that message,
    // so orphaned calls keep their chronological slot instead of drifting to
    // the end of the transcript.
    let mut messages = Vec::new();
    let mut pending_calls: HashMap<String, usize> = HashMap::new();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).map_err(|error| {
            format!(
                "Failed to read dsh session file {}: {error}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "user/message" => {
                if let Some(text) = user_prompt_text(&value) {
                    messages.push(text_message("user", text, event_time_ms(&value)));
                }
            }
            "assistant/message" => {
                if let Some(message) = assistant_message_from_event(&value) {
                    messages.push(message);
                }
            }
            "tool/call" => {
                if let Some(call) = pending_tool_call_from_event(&value) {
                    let index = messages.len();
                    messages.push(message_from_blocks(
                        "tool",
                        event_time_ms(&value),
                        vec![tool_call_block(
                            Some(call.call_id.clone()),
                            call.name,
                            call.arguments,
                        )],
                    ));
                    pending_calls.insert(call.call_id, index);
                }
            }
            "tool/result" => {
                let Some((call_id, output)) = tool_result_fields(&value) else {
                    continue;
                };
                let is_error = value.pointer("/data/error").is_some();
                let ts = event_time_ms(&value);
                match pending_calls.remove(&call_id) {
                    Some(index) => {
                        let existing = &mut messages[index];
                        let mut blocks = std::mem::take(&mut existing.blocks);
                        blocks.push(tool_result_block(
                            Some(call_id),
                            None,
                            Some(output),
                            Some(is_error),
                        ));
                        *existing = message_from_blocks("tool", ts, blocks);
                    }
                    None => {
                        messages.push(tool_execution_message(None, call_id, output, is_error, ts));
                    }
                }
            }
            _ => {}
        }
    }

    assign_missing_message_ids(&mut messages, PROVIDER_ID);
    Ok(messages)
}

/// Build the `SessionMeta` for one artifact from its header line plus early
/// events and file mtime.
fn parse_session_artifact(path: &Path) -> Option<SessionMeta> {
    let head = read_head_lines(path, HEAD_LINES).ok()?;

    let mut session_id: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut cwd: Option<String> = None;
    let mut first_user: Option<String> = None;
    let mut title_event: Option<String> = None;

    for raw_line in &head {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // One unparsable line must not hide the whole session: the artifact is
        // appended to by another process, so a torn tail line is expected.
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if event_type == "session" {
            if session_id.is_none() {
                session_id = value.get("id").and_then(Value::as_str).map(str::to_string);
            }
            if created_at.is_none() {
                created_at = value.get("createdAt").and_then(parse_timestamp_to_ms);
            }
            if cwd.is_none() {
                cwd = value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    .map(str::to_string);
            }
        }
        if event_type == "session/title" {
            // The last title event wins, mirroring upstream's `findLast`. An
            // explicit user rename is stored the same way (its source kind is
            // `user`), so it needs no special case here.
            if let Some(title) = value.pointer("/data/title").and_then(Value::as_str) {
                if !title.trim().is_empty() {
                    title_event = Some(truncate_summary(title, TITLE_MAX_CHARS).to_string());
                }
            }
        }
        if first_user.is_none() {
            if let Some(text) = user_prompt_text(&value) {
                first_user = Some(truncate_summary(&text, TITLE_MAX_CHARS).to_string());
            }
        }
    }

    let session_id = session_id?;
    let created_at = created_at.unwrap_or_else(|| file_modified_ms(path).unwrap_or(0));
    let last_active_at = file_modified_ms(path)
        .filter(|ts| *ts >= created_at)
        .or(Some(created_at));

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id,
        title: title_event.or_else(|| first_user.clone()),
        summary: first_user,
        project_dir: cwd,
        created_at: Some(created_at),
        last_active_at,
        source_path: path.to_string_lossy().to_string(),
        resume_command: None,
        runtime_source: None,
        runtime_distro: None,
    })
}

/// Scan every dsh session artifact under `root` into `SessionMeta` entries.
pub fn scan_sessions(root: &Path) -> Vec<SessionMeta> {
    collect_session_artifacts(root)
        .into_iter()
        .filter_map(|path| parse_session_artifact(&path))
        .collect()
}

/// Scan recent sessions, newest first. Reuses the full scan and truncates.
pub fn scan_recent_sessions(root: &Path, limit: usize) -> Vec<SessionMeta> {
    if limit == 0 {
        return Vec::new();
    }

    let mut sessions = scan_sessions(root);
    sessions.sort_by(|left, right| {
        let left_ts = left.last_active_at.or(left.created_at).unwrap_or(0);
        let right_ts = right.last_active_at.or(right.created_at).unwrap_or(0);
        right_ts.cmp(&left_ts)
    });
    sessions.truncate(limit);
    sessions
}

/// Load the surface messages from a dsh session artifact.
pub fn load_messages(source: &str) -> Result<Vec<SessionMessage>, String> {
    load_transcript(Path::new(source))
}

/// Test whether a session artifact's transcript contains the given query.
pub fn scan_messages_for_query(source: &str, query_lower: &str) -> Result<bool, String> {
    if query_lower.is_empty() {
        return Ok(false);
    }
    let path = Path::new(source);
    let mut reader = open_session_reader(path).map_err(|error| {
        format!(
            "Failed to open dsh session file {}: {error}",
            path.display()
        )
    })?;

    // Lightweight streaming scan: extract the same searchable text the full
    // transcript would surface (user prompts, assistant text/reasoning, tool
    // names + arguments, tool result output) without building messages or
    // blocks. Field scope is intentionally kept in sync with load_transcript;
    // if a new event type becomes searchable there, mirror it here. First hit
    // short-circuits, which keeps large artifacts cheap to search.
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).map_err(|error| {
            format!(
                "Failed to read dsh session file {}: {error}",
                path.display()
            )
        })?;
        if read == 0 {
            return Ok(false);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let fragments: Vec<String> = match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "user/message" => user_prompt_text(&value)
                .map(|text| vec![text])
                .unwrap_or_default(),
            "assistant/message" => append_event_data(&value, "assistant/message")
                .and_then(|data| {
                    data.get("message")
                        .filter(|message| message.is_object())
                        .or(Some(data))
                })
                .map(|message| {
                    message
                        .get("content")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| {
                                    let kind = item.get("type").and_then(Value::as_str)?;
                                    (kind == "reasoning" || kind == "text")
                                        .then(|| item.get("text").and_then(Value::as_str))
                                        .flatten()
                                        .map(str::to_string)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default(),
            "tool/call" => value
                .get("data")
                .map(|data| {
                    let name = data.get("name").and_then(Value::as_str).unwrap_or("");
                    let arguments = data
                        .get("arguments")
                        .map(|value| value.to_string())
                        .unwrap_or_default();
                    vec![format!("{name} {arguments}")]
                })
                .unwrap_or_default(),
            "tool/result" => tool_result_fields(&value)
                .map(|(_, output)| vec![output.to_string()])
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        for fragment in fragments {
            if text_contains_query(&fragment, query_lower) {
                return Ok(true);
            }
        }
    }
}

/// Delete a dsh session by removing its owning artifact directory, guarded to
/// the artifact name (any generation) and a location under `root`.
pub fn delete_session(root: &Path, source: &str) -> Result<(), String> {
    let path = Path::new(source);
    // Any generation is a legitimate handle: deleting removes the whole session
    // directory, so every generation of that session goes with it.
    if !session_artifact::is_session_artifact(path) {
        return Err("Not a dsh session artifact".to_string());
    }
    let session_dir = path
        .parent()
        .ok_or_else(|| "Invalid dsh session directory".to_string())?;
    if !session_dir.starts_with(root) {
        return Err("dsh session directory is outside the sessions root".to_string());
    }
    std::fs::remove_dir_all(session_dir).map_err(|error| {
        format!(
            "Failed to delete dsh session {}: {error}",
            session_dir.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn zstd_bytes(payload: &[u8]) -> Vec<u8> {
        zstd::stream::encode_all(std::io::Cursor::new(payload), 0).expect("zstd encode")
    }

    /// Write one artifact under its own session directory, using the canonical
    /// v0 name.
    fn write_artifact(dir: &Path, session_id: &str, compressed: bool, events: &str) {
        let file_name = if compressed {
            "session.jsonl.zstd"
        } else {
            "session.jsonl"
        };
        write_artifact_named(dir, session_id, file_name, compressed, events);
    }

    /// Write one artifact with an explicit file name, so a test can place
    /// several format generations of one session side by side.
    fn write_artifact_named(
        dir: &Path,
        session_id: &str,
        file_name: &str,
        compressed: bool,
        events: &str,
    ) {
        let session_dir = dir.join(session_id);
        std::fs::create_dir_all(&session_dir).expect("create session dir");
        let payload = if compressed {
            zstd_bytes(events.as_bytes())
        } else {
            events.as_bytes().to_vec()
        };
        std::fs::write(session_dir.join(file_name), payload).expect("write artifact");
    }

    /// Realistic dsh v0 log: header, a real prompt, injected context, one
    /// assistant step with reasoning/text/tool-call, a paired call+result,
    /// and a compaction replacement copy that must stay hidden.
    fn sample_events(created: i64) -> String {
        format!(
            r##"{{"type":"session","version":0,"id":"s1","createdAt":{created},"cwd":"/home/user/proj","delegationDepth":0}}
{{"type":"user/message","seq":7,"time":5,"data":{{"content":[{{"type":"text","text":"Hello there"}}],"source":{{"kind":"user"}},"role":"user","id":"u1"}},"surfaceOp":"append"}}
{{"type":"user/message","seq":8,"time":6,"data":{{"content":[{{"type":"text","text":"# AGENTS.md instructions"}}],"source":{{"kind":"agent-instructions"}},"role":"user","id":"u2"}},"surfaceOp":"append"}}
{{"type":"assistant/message","seq":9,"time":9,"data":{{"turn":1,"step":1,"message":{{"role":"assistant","content":[{{"type":"reasoning","text":"thinking hard"}},{{"type":"text","text":"Hi back"}},{{"type":"tool-call","id":"call-1","name":"glob","arguments":"{{\"pattern\":\"*.rs\"}}"}}],"source":{{"kind":"model","provider":"p","model":"test-model"}},"id":"a1"}},"usage":{{"inputTokens":10,"outputTokens":2,"cacheReadTokens":4}}}},"surfaceOp":"append"}}
{{"type":"tool/call","seq":10,"time":10,"data":{{"turn":1,"step":1,"callId":"call-1","name":"glob","arguments":"{{\"pattern\":\"*.rs\"}}"}}}}
{{"type":"tool/result","seq":11,"time":11,"data":{{"turn":1,"step":1,"message":{{"role":"tool","content":[{{"type":"tool-result","toolCallId":"call-1","content":[{{"type":"text","text":"src/main.rs"}}]}}]}}}},"surfaceOp":"append"}}
{{"type":"user/message","seq":12,"time":12,"data":{{"content":[{{"type":"text","text":"compaction summary"}}],"source":{{"kind":"plugin","plugin":"dsh-compaction"}},"role":"user","id":"u3"}},"surfaceOp":{{"op":"replace","start":7,"end":11}}}}
"##
        )
    }

    #[test]
    fn scan_parses_zstd_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact(
            dir.path(),
            "proj-a",
            true,
            &sample_events(1_700_000_000_000),
        );
        let sessions = scan_sessions(dir.path());
        assert_eq!(sessions.len(), 1);
        let meta = &sessions[0];
        assert_eq!(meta.session_id, "s1");
        assert_eq!(meta.project_dir.as_deref(), Some("/home/user/proj"));
        assert_eq!(meta.created_at.unwrap(), 1_700_000_000_000);
        // Only the real user prompt qualifies as the title candidate.
        assert_eq!(meta.title.as_deref(), Some("Hello there"));
    }

    #[test]
    fn load_messages_extracts_surface_messages() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact(
            dir.path(),
            "proj-a",
            true,
            &sample_events(1_700_000_000_000),
        );
        let path = collect_session_artifacts(dir.path())[0].clone();
        let messages = load_messages(&path.to_string_lossy()).expect("load");

        // User prompt, assistant step, tool execution; the injected context
        // message and the compaction replacement copy never surface.
        assert_eq!(messages.len(), 3);

        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello there");
        assert!(messages[0].id.is_some());

        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].blocks.len(), 2);
        assert_eq!(messages[1].blocks[0].kind, "thinking");
        assert_eq!(messages[1].blocks[1].kind, "text");
        assert_eq!(messages[1].model.as_deref(), Some("test-model"));
        let usage = messages[1].usage.as_ref().expect("usage");
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(2));
        assert_eq!(usage.cache_read_input_tokens, Some(4));

        // The call/result pair merges into one tool-execution block.
        assert_eq!(messages[2].role, "tool");
        assert_eq!(messages[2].blocks.len(), 1);
        assert_eq!(messages[2].blocks[0].kind, "tool_execution");
        assert_eq!(messages[2].blocks[0].tool_name.as_deref(), Some("glob"));
        assert_eq!(messages[2].blocks[0].status.as_deref(), Some("success"));
        let rendered = serde_json::to_string(&messages).expect("render");
        assert!(rendered.contains("src/main.rs"));
        assert!(!rendered.contains("AGENTS.md instructions"));
        assert!(!rendered.contains("compaction summary"));
    }

    #[test]
    fn orphan_tool_call_surfaces_as_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let events = format!(
            r#"{{"type":"session","version":0,"id":"s1","createdAt":1,"cwd":"/p"}}
{{"type":"tool/call","seq":1,"time":2,"data":{{"turn":1,"step":1,"callId":"call-x","name":"bash","arguments":"{{\"command\":\"ls\"}}"}}}}
"#
        );
        write_artifact(dir.path(), "proj-a", false, &events);
        let path = collect_session_artifacts(dir.path())[0].clone();
        let messages = load_messages(&path.to_string_lossy()).expect("load");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "tool");
        assert_eq!(messages[0].blocks.len(), 1);
        assert_eq!(messages[0].blocks[0].kind, "tool_call");
        assert_eq!(messages[0].blocks[0].status.as_deref(), Some("pending"));
    }

    #[test]
    fn orphan_tool_call_keeps_its_file_position() {
        let dir = tempfile::tempdir().expect("tempdir");
        // call-x never resolves; the later user message must come after it,
        // not before it as with the old append-at-end behavior.
        let events = format!(
            r#"{{"type":"session","version":0,"id":"s1","createdAt":1,"cwd":"/p"}}
{{"type":"tool/call","seq":1,"time":2,"data":{{"turn":1,"step":1,"callId":"call-x","name":"bash","arguments":"{{\"command\":\"ls\"}}"}}}}
{{"type":"user/message","seq":2,"time":3,"data":{{"source":{{"kind":"user"}},"content":"follow up"}}}}
"#
        );
        write_artifact(dir.path(), "proj-a", false, &events);
        let path = collect_session_artifacts(dir.path())[0].clone();
        let messages = load_messages(&path.to_string_lossy()).expect("load");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "tool");
        assert_eq!(messages[0].ts, Some(2_000));
        assert_eq!(messages[0].blocks[0].status.as_deref(), Some("pending"));
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content, "follow up");
    }

    #[test]
    fn multipart_tool_result_joins_all_parts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let events = format!(
            r#"{{"type":"session","version":0,"id":"s1","createdAt":1,"cwd":"/p"}}
{{"type":"tool/call","seq":1,"time":2,"data":{{"turn":1,"step":1,"callId":"call-m","name":"read","arguments":"null"}}}}
{{"type":"tool/result","seq":2,"time":3,"data":{{"message":{{"content":[{{"type":"tool-result","toolCallId":"call-m","content":"first part"}},{{"type":"text","text":"annotation ignored"}},{{"type":"tool-result","toolCallId":"call-m","content":"second part"}}]}}}}}}
"#
        );
        write_artifact(dir.path(), "proj-a", false, &events);
        let path = collect_session_artifacts(dir.path())[0].clone();
        let messages = load_messages(&path.to_string_lossy()).expect("load");

        assert_eq!(messages.len(), 1);
        let rendered = serde_json::to_string(&messages).expect("render");
        assert!(rendered.contains("first part"));
        assert!(rendered.contains("second part"));
        assert!(!rendered.contains("annotation ignored"));
    }

    #[test]
    fn scan_messages_query_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact(
            dir.path(),
            "proj-a",
            false,
            &sample_events(1_700_000_000_000),
        );
        let path = collect_session_artifacts(dir.path())[0].clone();
        let source = path.to_string_lossy();
        assert!(scan_messages_for_query(&source, "hi back").expect("scan"));
        // Tool names surface through the [Tool: name] preview.
        assert!(scan_messages_for_query(&source, "glob").expect("scan"));
        // Injected context stays out of the search space.
        assert!(!scan_messages_for_query(&source, "agents.md instructions").expect("scan"));
        assert!(!scan_messages_for_query(&source, "zzz").expect("scan"));
    }

    #[test]
    fn delete_removes_session_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact(
            dir.path(),
            "proj-a",
            true,
            &sample_events(1_700_000_000_000),
        );
        let path = collect_session_artifacts(dir.path())[0].clone();
        delete_session(dir.path(), &path.to_string_lossy()).expect("delete");
        assert!(!path.parent().unwrap().exists());
        assert!(scan_sessions(dir.path()).is_empty());
    }

    #[test]
    fn tool_arguments_keep_unparsable_payload_verbatim() {
        let arguments = parse_tool_arguments(&json!("not json"));
        assert_eq!(arguments, Some(json!("not json")));
        let arguments = parse_tool_arguments(&json!("{\"a\":1}"));
        assert_eq!(arguments, Some(json!({ "a": 1 })));
    }

    /// A directory holding several generations yields exactly one session,
    /// resolved from the highest generation.
    #[test]
    fn scan_prefers_the_highest_generation_in_one_session_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact_named(
            dir.path(),
            "proj-a",
            "session.jsonl.zstd",
            true,
            &sample_events(1_600_000_000_000),
        );
        // A migrated successor: same session id, newer generation, its own
        // header creation metadata is the same because it is the same session.
        write_artifact_named(
            dir.path(),
            "proj-a",
            "session.v3.jsonl.zstd",
            true,
            &sample_events(1_700_000_000_000),
        );

        let artifacts = collect_session_artifacts(dir.path());
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].ends_with("session.v3.jsonl.zstd"));

        let sessions = scan_sessions(dir.path());
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].source_path.ends_with("session.v3.jsonl.zstd"));
        assert_eq!(sessions[0].created_at.unwrap(), 1_700_000_000_000);
    }

    /// A session whose only artifact is a versioned generation is visible.
    #[test]
    fn scan_finds_a_versioned_only_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact_named(
            dir.path(),
            "proj-a",
            "session.v3.jsonl",
            false,
            &sample_events(1_700_000_000_000),
        );

        let sessions = scan_sessions(dir.path());
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].source_path.ends_with("session.v3.jsonl"));
    }

    /// One unparsable line in the head window must not hide the session.
    #[test]
    fn scan_survives_an_unparsable_head_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let events = format!(
            "{}\n{{ truncated",
            sample_events(1_700_000_000_000).trim_end()
        );
        write_artifact(dir.path(), "proj-a", false, &events);

        let sessions = scan_sessions(dir.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("Hello there"));
    }

    /// Any generation is a valid delete handle; the whole session directory goes.
    #[test]
    fn delete_accepts_a_versioned_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact_named(
            dir.path(),
            "proj-a",
            "session.v3.jsonl.zstd",
            true,
            &sample_events(1_700_000_000_000),
        );
        let path = collect_session_artifacts(dir.path())[0].clone();
        delete_session(dir.path(), &path.to_string_lossy()).expect("delete");
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn delete_rejects_a_non_artifact_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_artifact(dir.path(), "proj-a", false, &sample_events(1));
        let stray = dir.path().join("proj-a").join("notes.txt");
        std::fs::write(&stray, "x").expect("write stray file");
        assert!(delete_session(dir.path(), &stray.to_string_lossy()).is_err());
        assert!(stray.exists());
    }

    /// The last `session/title` event wins; the summary keeps the first prompt.
    #[test]
    fn scan_reads_the_last_session_title_event() {
        let dir = tempfile::tempdir().expect("tempdir");
        let events = format!(
            r##"{}
{{"type":"session/title","seq":13,"time":13,"data":{{"title":"Generated title","messageSeqs":[7],"source":{{"kind":"provider","provider":"session-title-first-prompt-llm"}}}}}}
{{"type":"session/title","seq":14,"time":14,"data":{{"title":"Renamed by hand","messageSeqs":[7],"source":{{"kind":"user"}}}}}}
"##,
            sample_events(1_700_000_000_000).trim_end()
        );
        write_artifact(dir.path(), "proj-a", false, &events);

        let sessions = scan_sessions(dir.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("Renamed by hand"));
        assert_eq!(sessions[0].summary.as_deref(), Some("Hello there"));
    }

    /// An empty title event must not blank the title.
    #[test]
    fn scan_ignores_an_empty_title_event() {
        let dir = tempfile::tempdir().expect("tempdir");
        let events = format!(
            "{}\n{}",
            sample_events(1_700_000_000_000).trim_end(),
            r#"{"type":"session/title","seq":13,"time":13,"data":{"title":"   "}}"#
        );
        write_artifact(dir.path(), "proj-a", false, &events);

        let sessions = scan_sessions(dir.path());
        assert_eq!(sessions[0].title.as_deref(), Some("Hello there"));
    }

    /// Smoke test over a real artifact: point `DSH_SMOKE_SESSION` at a
    /// `session[.v<N>].jsonl[.zstd]` path and run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_artifact_smoke() {
        let Ok(source) = std::env::var("DSH_SMOKE_SESSION") else {
            return;
        };
        let messages = load_messages(&source).expect("load");
        let mut role_counts = std::collections::BTreeMap::new();
        for message in &messages {
            *role_counts.entry(message.role.as_str()).or_insert(0u32) += 1;
        }
        let tool_block_kinds: Vec<&str> = messages
            .iter()
            .flat_map(|message| message.blocks.iter())
            .filter_map(|block| {
                matches!(
                    block.kind.as_str(),
                    "tool_call" | "tool_result" | "tool_execution"
                )
                .then_some(block.kind.as_str())
            })
            .collect();
        println!("total messages: {}", messages.len());
        println!("role counts: {role_counts:?}");
        println!(
            "tool blocks: {} ({tool_block_kinds:?})",
            tool_block_kinds.len()
        );
    }

    /// Smoke test over a real sessions root: point `DSH_SMOKE_SESSIONS_ROOT` at
    /// a dsh `<home>/sessions` directory and run
    /// `cargo test --lib -- --ignored dsh_sessions`.
    ///
    /// The count assertion is the one that matters: a session directory whose
    /// selected artifact fails to parse would silently vanish from the list,
    /// and only comparing against the directory count catches that.
    #[test]
    #[ignore]
    fn real_sessions_root_smoke() {
        let Ok(root) = std::env::var("DSH_SMOKE_SESSIONS_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);

        // Ground truth: every session directory that holds an accepted
        // artifact, counted without the parser under test.
        let mut expected_dirs: Vec<PathBuf> = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if session_artifact::is_session_artifact(&path) {
                    expected_dirs.push(path.parent().unwrap_or(Path::new(".")).to_path_buf());
                }
            }
        }
        expected_dirs.sort();
        expected_dirs.dedup();

        let sessions = scan_sessions(&root);
        for meta in &sessions {
            println!("{} -> {}", meta.session_id, meta.source_path);
        }
        println!(
            "session dirs on disk: {}, sessions parsed: {}",
            expected_dirs.len(),
            sessions.len()
        );

        assert_eq!(
            sessions.len(),
            expected_dirs.len(),
            "every session directory must yield exactly one parsed session"
        );
        let mut source_dirs: Vec<&str> = sessions
            .iter()
            .filter_map(|meta| Path::new(&meta.source_path).parent())
            .filter_map(|dir| dir.to_str())
            .collect();
        source_dirs.sort_unstable();
        let before = source_dirs.len();
        source_dirs.dedup();
        assert_eq!(before, source_dirs.len(), "one artifact per session dir");
        let mut ids: Vec<&str> = sessions
            .iter()
            .map(|meta| meta.session_id.as_str())
            .collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "session ids must be unique");
    }
}
