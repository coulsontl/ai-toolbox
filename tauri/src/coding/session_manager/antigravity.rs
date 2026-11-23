use std::collections::{HashMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use super::message_blocks::{
    message_from_blocks, text_block, thinking_block, tool_call_block, tool_result_block,
};
use super::utils::{
    build_resume_command, collect_recent_files_by_modified, extract_prompt_title_text,
    parse_timestamp_to_ms, text_contains_query, truncate_summary,
};
use super::{assign_missing_message_ids, SessionMessage, SessionMessageBlock, SessionMeta};

const PROVIDER_ID: &str = "antigravity";

/// Antigravity CLI (`agy`) session layout under `<cli_root>` (default
/// `~/.gemini/antigravity-cli`):
/// - `conversations/<uuid>.db` — SQLite resume carrier; the filename stem is
///   the conversation id accepted by `agy --conversation <uuid>`. Enumeration
///   is rooted here: a missing database means the CLI cannot resume the
///   session even if display logs remain.
/// - `brain/<uuid>/.system_generated/logs/transcript.jsonl` — rolling display
///   mirror, rewritten whole on compaction (not append-only).
/// - `brain/<uuid>/.system_generated/logs/transcript_full.jsonl` — nominally
///   complete mirror, also truncated at checkpoints. Read whichever is larger.
/// - `history.jsonl` — index lines of
///   `{display, timestamp, workspace, conversationId}`.
#[derive(Debug, Clone, Default)]
struct HistoryEntry {
    display: Option<String>,
    timestamp: Option<Value>,
    workspace: Option<String>,
}

pub fn scan_sessions(cli_root: &Path) -> Vec<SessionMeta> {
    let conversations_dir = cli_root.join("conversations");
    let Ok(entries) = std::fs::read_dir(&conversations_dir) else {
        return Vec::new();
    };

    let history = load_history_index(cli_root);
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_conversation_db(&path) {
            continue;
        }
        let Some(uuid) = conversation_uuid_from_db(&path) else {
            continue;
        };
        if let Some(meta) = build_session_meta(cli_root, &uuid, &history) {
            sessions.push(meta);
        }
    }
    sessions
}

pub fn scan_recent_sessions(cli_root: &Path, limit: usize) -> Vec<SessionMeta> {
    if limit == 0 {
        return Vec::new();
    }
    let brain_dir = cli_root.join("brain");
    if !brain_dir.is_dir() {
        return Vec::new();
    }

    let history = load_history_index(cli_root);
    let transcript_files =
        collect_recent_files_by_modified(&brain_dir, limit.saturating_mul(4).max(limit), |path| {
            is_transcript_candidate(path)
        });

    let mut sessions = Vec::new();
    let mut seen_uuids = HashSet::new();
    for path in transcript_files {
        let Some(uuid) = conversation_uuid_from_transcript(&path) else {
            continue;
        };
        if !seen_uuids.insert(uuid.clone()) {
            continue;
        }
        // Only surface sessions the CLI can actually resume.
        if !cli_root
            .join("conversations")
            .join(format!("{uuid}.db"))
            .is_file()
        {
            continue;
        }
        if let Some(meta) = build_session_meta(cli_root, &uuid, &history) {
            sessions.push(meta);
            if sessions.len() >= limit {
                break;
            }
        }
    }
    sessions
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let transcript_path = resolve_transcript_for_source(path).ok_or_else(|| {
        format!(
            "Antigravity session has no readable transcript for {}",
            path.display()
        )
    })?;
    let content = std::fs::read_to_string(&transcript_path).map_err(|error| {
        format!(
            "Failed to read Antigravity transcript {}: {error}",
            transcript_path.display()
        )
    })?;

    let mut result = Vec::new();
    let mut pending_tool_calls: VecDeque<(String, String)> = VecDeque::new();
    for record in parse_transcript_records(&content) {
        let Some(message) = message_from_record(&record, &mut pending_tool_calls) else {
            continue;
        };
        result.push(message);
    }

    assign_missing_message_ids(&mut result, PROVIDER_ID);
    Ok(result)
}

pub fn scan_messages_for_query(path: &Path, query_lower: &str) -> Result<bool, String> {
    let messages = load_messages(path)?;
    Ok(messages
        .iter()
        .any(|message| text_contains_query(&message.content, query_lower)))
}

pub fn delete_session(path: &Path) -> Result<(), String> {
    let Some((cli_root, uuid)) = session_identity_from_source(path) else {
        return Err(format!(
            "Failed to determine Antigravity session identity for {}",
            path.display()
        ));
    };

    let mut errors = Vec::new();

    // Resume carrier plus its SQLite sidecars. Best-effort: a live CLI may
    // hold the database open; the transcript/brain cleanup must still run.
    let db_path = cli_root.join("conversations").join(format!("{uuid}.db"));
    if let Err(error) = remove_file_if_exists(&db_path) {
        errors.push(error);
    }
    for suffix in ["-wal", "-shm"] {
        let sidecar = db_path.with_extension(format!("db{suffix}"));
        if let Err(error) = remove_file_if_exists(&sidecar) {
            errors.push(error);
        }
    }

    if let Err(error) = remove_dir_if_exists(&cli_root.join("brain").join(&uuid)) {
        errors.push(error);
    }
    log_cleanup_errors(errors);

    Ok(())
}

fn build_session_meta(
    cli_root: &Path,
    uuid: &str,
    history: &HashMap<String, HistoryEntry>,
) -> Option<SessionMeta> {
    // Skip empty/uninitialized conversation databases (e.g. 393KB blank DBs
    // created before any user turn) that have no readable transcript mirror;
    // otherwise they show up as bare UUIDs and fail when opening details.
    let transcript_path = pick_transcript(cli_root, uuid)?;
    let history_entry = history.get(uuid);

    let first_user_message = std::fs::read_to_string(&transcript_path)
        .ok()
        .and_then(|content| extract_first_user_text(&content));

    let title = history_entry
        .and_then(|entry| entry.display.clone())
        .map(|value| truncate_summary(&value, 80))
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            first_user_message
                .as_deref()
                .and_then(|text| extract_prompt_title_text(text, 80))
        })
        .unwrap_or_else(|| truncate_summary(uuid, 80));

    let project_dir = history_entry
        .and_then(|entry| entry.workspace.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let history_ts = history_entry
        .and_then(|entry| entry.timestamp.as_ref())
        .and_then(parse_timestamp_to_ms);
    let file_ts = file_modified_ms(&transcript_path)
        .or_else(|| file_modified_ms(&cli_root.join("conversations").join(format!("{uuid}.db"))));
    let created_at = history_ts.or(file_ts);
    let last_active_at = file_ts.or(history_ts);

    let source_path = transcript_path.to_string_lossy().to_string();

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: uuid.to_string(),
        title: Some(title.clone()),
        summary: Some(truncate_summary(&title, 160)),
        project_dir: project_dir.clone(),
        created_at,
        last_active_at,
        source_path,
        resume_command: Some(build_resume_command(
            project_dir.as_deref(),
            &format!("agy --conversation {uuid}"),
        )),
        runtime_source: None,
        runtime_distro: None,
    })
}

fn load_history_index(cli_root: &Path) -> HashMap<String, HistoryEntry> {
    let mut index = HashMap::new();
    let Ok(content) = std::fs::read_to_string(cli_root.join("history.jsonl")) else {
        return index;
    };

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(conversation_id) = record
            .get("conversationId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        index.insert(
            conversation_id.to_string(),
            HistoryEntry {
                display: record
                    .get("display")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                timestamp: record.get("timestamp").cloned(),
                workspace: record
                    .get("workspace")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            },
        );
    }
    index
}

/// Prefer the larger transcript mirror; either file may be truncated at
/// checkpoints, and both may be rewritten by compaction.
fn pick_transcript(cli_root: &Path, uuid: &str) -> Option<PathBuf> {
    let logs_dir = cli_root
        .join("brain")
        .join(uuid)
        .join(".system_generated")
        .join("logs");
    let mut best: Option<(PathBuf, u64)> = None;
    for file_name in ["transcript_full.jsonl", "transcript.jsonl"] {
        let path = logs_dir.join(file_name);
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let size = metadata.len();
        let replace = best
            .as_ref()
            .map(|(_, best_size)| size > *best_size)
            .unwrap_or(true);
        if replace {
            best = Some((path, size));
        }
    }
    best.map(|(path, _)| path)
}

fn resolve_transcript_for_source(source_path: &Path) -> Option<PathBuf> {
    let (cli_root, uuid) = session_identity_from_source(source_path)?;
    pick_transcript(&cli_root, &uuid)
}

/// Recover `(cli_root, conversation uuid)` from either a
/// `conversations/<uuid>.db` path or a
/// `brain/<uuid>/.system_generated/logs/transcript*.jsonl` path.
fn session_identity_from_source(source_path: &Path) -> Option<(PathBuf, String)> {
    if let Some(uuid) = conversation_uuid_from_db(source_path) {
        let cli_root = source_path.parent()?.parent()?.to_path_buf();
        return Some((cli_root, uuid));
    }
    if let Some(uuid) = conversation_uuid_from_transcript(source_path) {
        // <cli_root>/brain/<uuid>/.system_generated/logs/<file>
        let cli_root = source_path
            .parent()?
            .parent()?
            .parent()?
            .parent()?
            .parent()?
            .to_path_buf();
        return Some((cli_root, uuid));
    }
    None
}

fn is_conversation_db(path: &Path) -> bool {
    path.is_file()
        && path.extension().and_then(|value| value.to_str()) == Some("db")
        && path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            == Some("conversations")
}

fn conversation_uuid_from_db(path: &Path) -> Option<String> {
    if !is_conversation_db(path) {
        return None;
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_transcript_candidate(path: &Path) -> bool {
    let is_transcript_file = matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some("transcript.jsonl") | Some("transcript_full.jsonl")
    );
    is_transcript_file && conversation_uuid_from_transcript(path).is_some()
}

fn conversation_uuid_from_transcript(path: &Path) -> Option<String> {
    if !matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some("transcript.jsonl") | Some("transcript_full.jsonl")
    ) {
        return None;
    }
    let logs_dir = path.parent()?;
    if logs_dir.file_name().and_then(|value| value.to_str()) != Some("logs") {
        return None;
    }
    let system_dir = logs_dir.parent()?;
    if system_dir.file_name().and_then(|value| value.to_str()) != Some(".system_generated") {
        return None;
    }
    let uuid_dir = system_dir.parent()?;
    if uuid_dir
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|value| value.to_str())
        != Some("brain")
    {
        return None;
    }
    uuid_dir
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_transcript_records(content: &str) -> Vec<Value> {
    let mut records = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // Compaction rewrites the file; a crash mid-flush can leave a
        // truncated final line — skip unparseable rows, never fail the read.
        if let Ok(record) = serde_json::from_str::<Value>(line) {
            records.push(record);
        }
    }
    // Order by step_index when present; the file is append-ordered but
    // compaction rewrites must not be trusted for ordering.
    records.sort_by_key(|record| {
        record
            .get("step_index")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX)
    });
    records
}

fn message_from_record(
    record: &Value,
    pending_tool_calls: &mut VecDeque<(String, String)>,
) -> Option<SessionMessage> {
    let source = record.get("source").and_then(Value::as_str)?;
    let role = match source {
        "USER_EXPLICIT" => {
            pending_tool_calls.clear();
            "user"
        }
        "MODEL" => "assistant",
        _ => return None,
    };

    let record_type = record.get("type").and_then(Value::as_str).unwrap_or("");
    let ts = record.get("created_at").and_then(parse_timestamp_to_ms);
    let step_index = record.get("step_index").and_then(Value::as_i64);

    // Antigravity CLI records tool execution outputs as subsequent
    // `source=MODEL, type=GENERIC` entries immediately following a
    // `PLANNER_RESPONSE` with `tool_calls`. Pair them in order so the shared
    // detail view merges each output into its tool card instead of leaving
    // calls stuck in `pending` and scattering raw tool output bubbles.
    if role == "assistant" && record_type == "GENERIC" {
        if let Some((tool_id, tool_name)) = pending_tool_calls.pop_front() {
            let output_text = record
                .get("content")
                .map(strip_machine_metadata)
                .unwrap_or_default();
            let is_error = record
                .get("status")
                .and_then(Value::as_str)
                .map(|status| matches!(status.to_ascii_uppercase().as_str(), "ERROR" | "FAILED"))
                .filter(|flag| *flag);
            let blocks = vec![tool_result_block(
                Some(tool_id),
                Some(tool_name),
                Some(Value::String(output_text)),
                is_error,
            )];
            let mut message = message_from_blocks("assistant", ts, blocks);
            message.id = step_index.map(|index| format!("{PROVIDER_ID}-step-{index}"));
            message.message_type = Some(record_type.to_string());
            return Some(message);
        }
    }

    let blocks = blocks_from_record(record, role, step_index, pending_tool_calls);
    if blocks.is_empty() {
        return None;
    }

    let mut message = message_from_blocks(role, ts, blocks);
    message.id = step_index.map(|index| format!("{PROVIDER_ID}-step-{index}"));
    if !record_type.is_empty() {
        message.message_type = Some(record_type.to_string());
    }
    Some(message)
}

fn blocks_from_record(
    record: &Value,
    role: &str,
    step_index: Option<i64>,
    pending_tool_calls: &mut VecDeque<(String, String)>,
) -> Vec<SessionMessageBlock> {
    let mut blocks = Vec::new();

    if let Some(thinking) = record
        .get("thinking")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        blocks.push(thinking_block(thinking.to_string()));
    }

    if role == "assistant" {
        blocks.extend(tool_blocks_from_record(
            record,
            step_index,
            pending_tool_calls,
        ));
    }

    let content = record.get("content").map(strip_machine_metadata);
    match content {
        Some(text) if !text.trim().is_empty() => {
            let normalized = if role == "user" {
                strip_user_request_wrapper(&text)
            } else {
                text
            };
            if !normalized.trim().is_empty() {
                blocks.push(text_block(normalized));
            }
        }
        _ => {}
    }

    blocks
}

fn tool_blocks_from_record(
    record: &Value,
    step_index: Option<i64>,
    pending_tool_calls: &mut VecDeque<(String, String)>,
) -> Vec<SessionMessageBlock> {
    let Some(calls) = record.get("tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };
    if !calls.is_empty() {
        // Start a fresh pairing window for this planner turn.
        pending_tool_calls.clear();
    }
    let mut blocks = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let name = call
            .get("name")
            .or_else(|| call.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let tool_id = call
            .get("id")
            .or_else(|| call.get("call_id"))
            .or_else(|| call.get("callId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| match step_index {
                Some(step) => format!("{PROVIDER_ID}-step-{step}-tool-{index}"),
                None => format!("{PROVIDER_ID}-tool-{index}"),
            });
        let input = call
            .get("input")
            .or_else(|| call.get("args"))
            .or_else(|| call.get("arguments"))
            .cloned();
        blocks.push(tool_call_block(
            Some(tool_id.clone()),
            name.to_string(),
            input,
        ));
        if let Some(result) = call.get("result").or_else(|| call.get("output")) {
            let text = match result {
                Value::String(text) => text.clone(),
                _ => result.to_string(),
            };
            if !text.trim().is_empty() {
                blocks.push(tool_result_block(
                    Some(tool_id),
                    Some(name.to_string()),
                    Some(Value::String(text)),
                    None,
                ));
                continue;
            }
        }
        pending_tool_calls.push_back((tool_id, name.to_string()));
    }
    blocks
}

/// User content shares one string field with machine metadata; strip the
/// metadata blocks so titles, search and copy output stay human-readable.
fn strip_machine_metadata(content: &Value) -> String {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(_) | Value::Object(_) => super::utils::extract_text(content),
        _ => String::new(),
    };
    let mut cleaned = text;
    for (open, close) in [
        ("<ADDITIONAL_METADATA>", "</ADDITIONAL_METADATA>"),
        ("<USER_SETTINGS_CHANGE>", "</USER_SETTINGS_CHANGE>"),
    ] {
        cleaned = strip_tagged_spans(&cleaned, open, close);
    }
    cleaned
}

fn strip_user_request_wrapper(text: &str) -> String {
    let trimmed = text.trim();
    let Some(inner) = trimmed
        .strip_prefix("<USER_REQUEST>")
        .and_then(|rest| rest.strip_suffix("</USER_REQUEST>"))
    else {
        return strip_tagged_spans(trimmed, "<USER_REQUEST>", "</USER_REQUEST>");
    };
    inner.trim().to_string()
}

fn strip_tagged_spans(text: &str, open: &str, close: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + open.len()..];
        if let Some(end) = after_open.find(close) {
            rest = &after_open[end + close.len()..];
        } else {
            // Unterminated tag (truncated flush): drop the tag and keep the tail.
            result.push_str(after_open);
            rest = "";
            break;
        }
    }
    result.push_str(rest);
    result
}

fn extract_first_user_text(content: &str) -> Option<String> {
    for record in parse_transcript_records(content) {
        if record.get("source").and_then(Value::as_str) != Some("USER_EXPLICIT") {
            continue;
        }
        let text = strip_user_request_wrapper(&strip_machine_metadata(
            record.get("content").unwrap_or(&Value::Null),
        ));
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('/') || trimmed.starts_with('?') {
            continue;
        }
        return Some(trimmed.to_string());
    }
    None
}

fn file_modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to delete Antigravity session file {}: {error}",
            path.display()
        )),
    }
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to delete Antigravity session directory {}: {error}",
            path.display()
        )),
    }
}

fn log_cleanup_errors(errors: Vec<String>) {
    for error in errors {
        eprintln!("Antigravity session artifact cleanup warning: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ai-toolbox-antigravity-session-{label}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&path).expect("create test dir");
            Self { path }
        }

        fn cli_root(&self) -> PathBuf {
            self.path.join("antigravity-cli")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    const TEST_UUID: &str = "11111111-2222-4333-8444-555555555555";

    fn write_session_tree(cli_root: &Path) {
        let conversations_dir = cli_root.join("conversations");
        fs::create_dir_all(&conversations_dir).expect("create conversations dir");
        // The database content is opaque protobuf; scanners only need the file.
        fs::write(
            conversations_dir.join(format!("{TEST_UUID}.db")),
            b"fixture",
        )
        .expect("write db");

        let logs_dir = cli_root
            .join("brain")
            .join(TEST_UUID)
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs_dir).expect("create logs dir");
        let transcript = r#"{"step_index":0,"source":"SYSTEM","type":"EPHEMERAL_MESSAGE","status":"DONE","created_at":"2026-09-20T10:00:00Z","content":"boot"}
{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-20T10:00:01Z","content":"<USER_REQUEST>fix the login bug</USER_REQUEST>"}
{"step_index":2,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-20T10:00:02Z","content":"On it.","thinking":"check auth flow","tool_calls":[{"id":"call-1","name":"read_file","input":{"path":"login.ts"},"result":"file body"}]}
{"step_index":3,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-20T10:00:03Z","content":"<USER_REQUEST>also update tests</USER_REQUEST><ADDITIONAL_METADATA>{\"cwd\":\"/repo\"}</ADDITIONAL_METADATA>"}
not-json-garbage-line
"#;
        // transcript_full.jsonl is larger so it wins the mirror pick.
        fs::write(logs_dir.join("transcript.jsonl"), "{\"step_index\":0}\n")
            .expect("write transcript");
        fs::write(logs_dir.join("transcript_full.jsonl"), transcript)
            .expect("write transcript_full");

        fs::write(
            cli_root.join("history.jsonl"),
            format!(
                "{{\"display\":\"login bug session\",\"timestamp\":\"2026-09-20T10:00:03Z\",\"workspace\":\"/repo/demo\",\"conversationId\":\"{TEST_UUID}\"}}\n"
            ),
        )
        .expect("write history");
    }

    #[test]
    fn scan_lists_session_with_resume_command() {
        let test_dir = TestDir::new("scan");
        let cli_root = test_dir.cli_root();
        write_session_tree(&cli_root);

        let sessions = scan_sessions(&cli_root);
        assert_eq!(sessions.len(), 1);
        let meta = &sessions[0];
        assert_eq!(meta.session_id, TEST_UUID);
        assert_eq!(meta.provider_id, "antigravity");
        assert_eq!(meta.title.as_deref(), Some("login bug session"));
        assert_eq!(meta.project_dir.as_deref(), Some("/repo/demo"));
        let resume = meta.resume_command.as_deref().expect("resume command");
        assert!(
            resume.ends_with("agy --conversation 11111111-2222-4333-8444-555555555555"),
            "unexpected resume command: {resume}"
        );
        assert!(
            resume.contains("/repo/demo"),
            "resume command should carry the workspace dir: {resume}"
        );
        assert!(meta.source_path.ends_with("transcript_full.jsonl"));
    }

    #[test]
    fn load_messages_maps_roles_and_blocks() {
        let test_dir = TestDir::new("load");
        let cli_root = test_dir.cli_root();
        write_session_tree(&cli_root);

        let sessions = scan_sessions(&cli_root);
        let messages = load_messages(Path::new(&sessions[0].source_path)).expect("load messages");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "fix the login bug");
        assert_eq!(messages[1].role, "assistant");
        assert!(messages[1].content.contains("On it."));
        assert!(messages[1].content.contains("check auth flow"));
        assert_eq!(messages[1].blocks.len(), 3);
        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].content, "also update tests");
        assert!(messages.iter().all(|message| message.id.is_some()));
    }

    #[test]
    fn scan_recent_prefers_transcript_candidates() {
        let test_dir = TestDir::new("recent");
        let cli_root = test_dir.cli_root();
        write_session_tree(&cli_root);

        let sessions = scan_recent_sessions(&cli_root, 10);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, TEST_UUID);
    }

    #[test]
    fn scan_messages_for_query_matches_user_text() {
        let test_dir = TestDir::new("query");
        let cli_root = test_dir.cli_root();
        write_session_tree(&cli_root);

        let sessions = scan_sessions(&cli_root);
        let source = Path::new(&sessions[0].source_path);
        assert!(scan_messages_for_query(source, "login bug").expect("query"));
        assert!(!scan_messages_for_query(source, "no-such-phrase").expect("query"));
    }

    #[test]
    fn delete_session_removes_db_and_brain() {
        let test_dir = TestDir::new("delete");
        let cli_root = test_dir.cli_root();
        write_session_tree(&cli_root);

        let sessions = scan_sessions(&cli_root);
        assert_eq!(sessions.len(), 1);
        delete_session(Path::new(&sessions[0].source_path)).expect("delete");
        assert_eq!(scan_sessions(&cli_root).len(), 0);
        assert!(!cli_root.join("brain").join(TEST_UUID).exists());
        // Deleting again stays idempotent.
        delete_session(Path::new(&sessions[0].source_path)).expect("re-delete");
    }

    #[test]
    fn scan_skips_blank_conversation_db_without_transcript() {
        let test_dir = TestDir::new("blank-db");
        let cli_root = test_dir.cli_root();
        write_session_tree(&cli_root);

        // Blank 393KB conversation db created without any brain transcript.
        fs::write(
            cli_root
                .join("conversations")
                .join("3925aab9-93f9-40d4-83b7-683df02808f2.db"),
            b"blank-db",
        )
        .expect("write blank db");

        let sessions = scan_sessions(&cli_root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, TEST_UUID);
    }

    #[test]
    fn load_messages_pairs_generic_tool_outputs_with_planner_tool_calls() {
        let test_dir = TestDir::new("generic-pair");
        let cli_root = test_dir.cli_root();
        let conversations_dir = cli_root.join("conversations");
        let logs_dir = cli_root
            .join("brain")
            .join(TEST_UUID)
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&conversations_dir).unwrap();
        fs::create_dir_all(&logs_dir).unwrap();
        fs::write(conversations_dir.join(format!("{TEST_UUID}.db")), b"db").unwrap();

        let transcript = concat!(
            "{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"read_file\",\"args\":{\"Path\":\"a.rs\"}},{\"name\":\"edit_file\",\"args\":{\"Path\":\"a.rs\",\"Target\":\"old\",\"Replacement\":\"new\"}}]}\n",
            "{\"step_index\":2,\"source\":\"MODEL\",\"type\":\"GENERIC\",\"status\":\"DONE\",\"content\":\"file contents of a.rs\"}\n",
            "{\"step_index\":3,\"source\":\"MODEL\",\"type\":\"GENERIC\",\"status\":\"DONE\",\"content\":\"Made 1 replacement\"}\n",
            "{\"step_index\":4,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"read_file\",\"args\":{\"Path\":\"b.rs\"}}]}\n",
            "{\"step_index\":5,\"source\":\"MODEL\",\"type\":\"GENERIC\",\"status\":\"ERROR\",\"content\":\"file not found\"}\n",
        );
        let transcript_path = logs_dir.join("transcript_full.jsonl");
        fs::write(&transcript_path, transcript).unwrap();

        let messages = load_messages(&transcript_path).expect("load messages");
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].blocks[0].tool_id.as_deref(), Some("antigravity-step-1-tool-0"));
        assert_eq!(messages[0].blocks[1].tool_id.as_deref(), Some("antigravity-step-1-tool-1"));
        assert_eq!(messages[1].blocks[0].kind, "tool_result");
        assert_eq!(messages[1].blocks[0].tool_id.as_deref(), Some("antigravity-step-1-tool-0"));
        assert_eq!(messages[1].blocks[0].tool_name.as_deref(), Some("read_file"));
        assert_eq!(messages[2].blocks[0].kind, "tool_result");
        assert_eq!(messages[2].blocks[0].tool_id.as_deref(), Some("antigravity-step-1-tool-1"));
        assert_eq!(messages[2].blocks[0].tool_name.as_deref(), Some("edit_file"));
        // Next turn gets distinct step-scoped tool_id and propagates ERROR status.
        assert_eq!(messages[3].blocks[0].tool_id.as_deref(), Some("antigravity-step-4-tool-0"));
        assert_eq!(messages[4].blocks[0].tool_id.as_deref(), Some("antigravity-step-4-tool-0"));
        assert_eq!(messages[4].blocks[0].is_error, Some(true));
    }
}
