//! Sessions for WorkBuddy desktop and CodeBuddy Code CLI.
//!
//! Both products run one engine and write the **same** transcript shape, so
//! this module serves both. Verified on real installs (WorkBuddy v5.8.3 and
//! CodeBuddy Code v2.178.1): `projects/<cwd-encoded>/<sessionId>.jsonl`, one
//! JSON object per line, `message.role` + `content[]` with block types
//! `input_text` / `output_text` / `reasoning_text` / `tool_use` / `tool_result`.
//!
//! Divergences that matter here:
//! - CodeBuddy also writes `sessions/`, but it stays empty; sessions are always
//!   the `projects/` transcripts, never that directory.
//! - A `<sessionId>/` directory holding `subagents/agent-*.jsonl` sits next to
//!   the transcript. Those are child runs, not list entries.
//! - `ai-title` carries the generated title; `session-meta` carries the id.
//!   Neither is guaranteed early in the file, so the head window is generous
//!   and a filename fallback covers stripped/rotated files.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use super::message_blocks::{
    message_from_blocks, text_block, thinking_block, tool_call_block, tool_result_block,
};
use super::utils::{
    build_resume_command, collect_recent_files_by_modified, extract_prompt_title_text,
    parse_timestamp_to_ms, quote_session_arg, read_head_tail_lines, text_contains_query,
    truncate_summary,
};
use super::{assign_missing_message_ids, SessionMessage, SessionMeta};

/// A transcript file is `<sessionId>.jsonl`; sibling artifacts use other
/// suffixes (`.file-rollback.ndjson`) and `subagents/` holds child runs.
fn is_session_transcript(path: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
        return false;
    }
    if path
        .components()
        .any(|component| component.as_os_str() == "subagents")
    {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    // Primary session ids are UUIDs. Anything else in `projects/` (rollback
    // logs, fixtures) is not a session and must not become a list row.
    looks_like_session_id(stem)
}

fn looks_like_session_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

/// The `cwd` the product encodes into the directory name. Kept only as a
/// last-resort label: every transcript item carries the real `cwd`.
fn project_from_directory_name(name: &str) -> Option<String> {
    // `c-Users-name-repo` -> `c:\Users\name\repo` on Windows; the encoding
    // replaces every separator with `-`, so the original separators are lost.
    // Reconstructing would be guesswork, so only recover the drive-less shape.
    let trimmed = name.strip_prefix('c')?;
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.trim_start_matches('-').replace('-', "\\"))
}

fn session_id_from_filename(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

/// Which product's `projects/` tree is being read. Both write the same shape,
/// so only the reported provider id differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyProduct {
    Workbuddy,
    Codebuddy,
}

impl FamilyProduct {
    pub fn provider_id(self) -> &'static str {
        match self {
            Self::Workbuddy => "workbuddy",
            Self::Codebuddy => "codebuddy",
        }
    }
}

pub fn scan_sessions(root: &Path, product: FamilyProduct) -> Vec<SessionMeta> {
    let mut sessions = scan_recent_sessions(root, usize::MAX, product);
    sessions.sort_by_key(|meta| std::cmp::Reverse(meta.last_active_at));
    sessions
}

/// Newest-first scan, stopping once `limit` transcripts parsed.
///
/// The quick path must reuse the shared early-stop helper rather than
/// collecting the whole tree: WorkBuddy projects can hold thousands of files
/// once subagent runs accumulate.
pub fn scan_recent_sessions(
    root: &Path,
    limit: usize,
    product: FamilyProduct,
) -> Vec<SessionMeta> {
    if limit == 0 || !root.is_dir() {
        return Vec::new();
    }
    let files = collect_recent_files_by_modified(root, limit, is_session_transcript);
    let mut sessions = Vec::new();
    for path in files {
        let Some(meta) = parse_session(&path, product) else {
            continue;
        };
        sessions.push(meta);
        if sessions.len() >= limit {
            break;
        }
    }
    sessions
}

/// Reads one transcript's metadata from its head and tail.
///
/// Returns `None` only when the file has no usable session id at all; a
/// transcript whose head is unreadable still lists via its filename so the user
/// is not silently missing a session.
fn parse_session(path: &Path, product: FamilyProduct) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 40, 40).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut title: Option<String> = None;
    let mut first_user_text: Option<String> = None;

    for line in &head {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let item_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        if session_id.is_none() {
            session_id = value
                .get("sessionId")
                .or_else(|| value.get("id").filter(|_| item_type == "session-meta"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        if project_dir.is_none() {
            project_dir = value
                .get("cwd")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string);
        }
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if title.is_none() && item_type == "ai-title" {
            title = value
                .get("aiTitle")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        if first_user_text.is_none() && item_type == "message" {
            if value.get("role").and_then(Value::as_str) == Some("user") {
                let text = value
                    .get("content")
                    .map(collect_text_from_blocks)
                    .unwrap_or_default();
                // The engine prepends a `<system-reminder>` block describing the
                // workspace; it is not what the user typed.
                let cleaned = strip_system_reminder(&text);
                if !cleaned.trim().is_empty() {
                    first_user_text = Some(cleaned);
                }
            }
        }
    }

    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;
    for line in tail.iter().rev() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let item_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if summary.is_none() && item_type == "message" {
            if value.get("role").and_then(Value::as_str) == Some("assistant") {
                let text = value
                    .get("content")
                    .map(collect_text_from_blocks)
                    .unwrap_or_default();
                if !text.trim().is_empty() {
                    summary = Some(text);
                }
            }
        }
        if last_active_at.is_some() && summary.is_some() {
            break;
        }
    }

    let session_id = session_id.or_else(|| session_id_from_filename(path))?;
    let title = title.or_else(|| {
        first_user_text
            .as_deref()
            .and_then(|text| extract_prompt_title_text(text, 80))
    });
    let project_dir = project_dir.or_else(|| {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .and_then(project_from_directory_name)
    });

    let provider_id = product.provider_id();
    Some(SessionMeta {
        provider_id: provider_id.to_string(),
        session_id: session_id.clone(),
        title,
        summary: summary.map(|text| truncate_summary(&text, 160)),
        project_dir: project_dir.clone(),
        created_at,
        last_active_at: last_active_at.or(created_at),
        source_path: path.to_string_lossy().to_string(),
        resume_command: Some(build_resume_command(
            project_dir.as_deref(),
            // Both products resume by session id: `codebuddy --resume <id>` for
            // the CLI. WorkBuddy desktop has no public resume flag, so the
            // desktop page passes its own command name through `provider_id`.
            &format!(
                "{} --resume {}",
                resume_cli_name(),
                quote_session_arg(&session_id)
            ),
        )),
        runtime_source: None,
        runtime_distro: None,
    })
}

/// Which CLI resumes a session of this family. The desktop app embeds the same
/// engine, so `codebuddy --resume` works for both.
fn resume_cli_name() -> &'static str {
    "codebuddy"
}

fn strip_system_reminder(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<system-reminder") {
        out.push_str(&rest[..start]);
        let Some(open_end) = rest[start..].find('>') else {
            rest = "";
            break;
        };
        let after_open = &rest[start + open_end + 1..];
        let Some(close) = after_open.find("</system-reminder>") else {
            rest = "";
            break;
        };
        rest = &after_open[close + "</system-reminder>".len()..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Flattens a `content[]` array into plain text, ignoring tool blocks.
fn collect_text_from_blocks(content: &Value) -> String {
    let Some(items) = content.as_array() else {
        return content.as_str().unwrap_or_default().to_string();
    };
    let mut parts = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "input_text" | "output_text" | "text" | "reasoning_text" => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            _ => {}
        }
    }
    parts.join("\n")
}

pub fn load_messages(source: &Path, product: FamilyProduct) -> Result<Vec<SessionMessage>, String> {
    let file = fs::File::open(source)
        .map_err(|error| format!("Failed to open {}: {error}", source.display()))?;
    let mut messages = Vec::new();

    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let item_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);

        match item_type {
            "message" => {
                let role = value
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("assistant")
                    .to_string();
                let blocks = content_to_blocks(value.get("content"));
                if blocks.is_empty() {
                    continue;
                }
                let mut message = message_from_blocks(&role, ts, blocks);
                message.id = string_field(&value, "id");
                message.parent_id = string_field(&value, "parentId");
                message.model = value
                    .pointer("/providerData/model")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                messages.push(message);
            }
            "reasoning" => {
                // `content[]` is usually empty; the payload lives in
                // `rawContent[]` as `reasoning_text`.
                let text = collect_text_from_blocks(
                    value.get("rawContent").unwrap_or(&Value::Null),
                );
                if text.trim().is_empty() {
                    continue;
                }
                let mut message =
                    message_from_blocks("assistant", ts, vec![thinking_block(text)]);
                message.id = string_field(&value, "id");
                message.parent_id = string_field(&value, "parentId");
                messages.push(message);
            }
            "function_call" => {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let input = value.get("input").cloned().or_else(|| {
                    value
                        .get("arguments")
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                });
                let mut message = message_from_blocks(
                    "assistant",
                    ts,
                    vec![tool_call_block(
                        string_field(&value, "callId"),
                        name,
                        input,
                    )],
                );
                message.id = string_field(&value, "id");
                message.parent_id = string_field(&value, "parentId");
                messages.push(message);
            }
            "function_call_result" => {
                let output = value.get("output").cloned();
                let is_error = value
                    .get("isError")
                    .and_then(Value::as_bool)
                    .or_else(|| {
                        value
                            .get("status")
                            .and_then(Value::as_str)
                            .map(|status| status == "error")
                    });
                let name = value.get("name").and_then(Value::as_str).map(str::to_string);
                let mut message = message_from_blocks(
                    "tool",
                    ts,
                    vec![tool_result_block(
                        string_field(&value, "callId"),
                        name,
                        output,
                        is_error,
                    )],
                );
                message.id = string_field(&value, "id");
                message.parent_id = string_field(&value, "parentId");
                messages.push(message);
            }
            // `file-history-snapshot`, `credit-usage` and `session-meta` are
            // bookkeeping: they carry no conversation content.
            _ => {}
        }
    }

    assign_missing_message_ids(&mut messages, product.provider_id());
    Ok(messages)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn content_to_blocks(content: Option<&Value>) -> Vec<super::SessionMessageBlock> {
    let Some(content) = content else {
        return Vec::new();
    };
    let Some(items) = content.as_array() else {
        return content
            .as_str()
            .map(|text| vec![text_block(text)])
            .unwrap_or_default();
    };

    let mut blocks = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "input_text" | "output_text" | "text" => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        blocks.push(text_block(text));
                    }
                }
            }
            "reasoning_text" | "thinking" => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        blocks.push(thinking_block(text));
                    }
                }
            }
            "tool_use" => {
                blocks.push(tool_call_block(
                    string_field(item, "id"),
                    item.get("name").and_then(Value::as_str).unwrap_or("unknown"),
                    item.get("input").cloned(),
                ));
            }
            "tool_result" => {
                let is_error = item.get("is_error").and_then(Value::as_bool);
                blocks.push(tool_result_block(
                    string_field(item, "tool_use_id"),
                    item.get("name").and_then(Value::as_str).map(str::to_string),
                    item.get("content").cloned(),
                    is_error,
                ));
            }
            _ => {}
        }
    }
    blocks
}

/// Streaming text search; short-circuits on the first hit.
///
/// Field scope mirrors `load_messages`: adding a searchable field there means
/// adding it here too.
pub fn scan_messages_for_query(source: &Path, query_lower: &str) -> Result<bool, String> {
    if query_lower.is_empty() {
        return Ok(false);
    }
    let file = fs::File::open(source)
        .map_err(|error| format!("Failed to open {}: {error}", source.display()))?;

    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let searchable = match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" => collect_text_from_blocks(value.get("content").unwrap_or(&Value::Null)),
            "reasoning" => {
                collect_text_from_blocks(value.get("rawContent").unwrap_or(&Value::Null))
            }
            "function_call" => {
                let mut text = value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(input) = value.get("input") {
                    text.push('\n');
                    text.push_str(&input.to_string());
                }
                text
            }
            "function_call_result" => value
                .get("output")
                .map(|output| output.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        };
        if text_contains_query(&searchable, query_lower) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Deletes one session: its transcript, the sibling `<sessionId>/` directory
/// holding subagent runs, and the rollback log.
///
/// Deleting only the `.jsonl` leaves `subagents/` and the rollback log behind,
/// so the session can reappear as a partially-restored row.
pub fn delete_session(source: &Path) -> Result<(), String> {
    if !source.exists() {
        // Already gone: deleting twice is a converged success, not an error.
        return Ok(());
    }
    fs::remove_file(source)
        .map_err(|error| format!("Failed to delete {}: {error}", source.display()))?;

    let Some(parent) = source.parent() else {
        return Ok(());
    };
    let Some(stem) = source.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(());
    };

    let session_dir = parent.join(stem);
    if session_dir.is_dir() {
        fs::remove_dir_all(&session_dir).map_err(|error| {
            format!(
                "Failed to delete session directory {}: {error}",
                session_dir.display()
            )
        })?;
    }

    let rollback_log = parent.join(format!("{stem}.file-rollback.ndjson"));
    if rollback_log.is_file() {
        // Best-effort: losing the rollback log must not fail the delete.
        let _ = fs::remove_file(&rollback_log);
    }
    Ok(())
}

/// Summarises each subagent run of one session for the detail sub-panel.
///
/// Subagent transcripts use the same item shape as the parent, so the prompt
/// text of the first user message is the only useful title.
pub fn list_subagent_sessions(source: &Path) -> Vec<super::SessionSubagentMeta> {
    find_subagent_files(source)
        .into_iter()
        .map(|path| build_subagent_meta(&path))
        .collect()
}

fn build_subagent_meta(path: &Path) -> super::SessionSubagentMeta {
    let (message_count, first_message_time, last_message_time, summary) =
        summarize_subagent(path);
    let file_stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("subagent")
        .to_string();
    let id = file_stem
        .strip_prefix("agent-")
        .unwrap_or(&file_stem)
        .to_string();
    let title = summary.clone().unwrap_or_else(|| id.clone());

    super::SessionSubagentMeta {
        id,
        source_path: path.to_string_lossy().to_string(),
        title,
        summary,
        subagent_type: None,
        message_count,
        first_message_time,
        last_message_time,
    }
}

fn summarize_subagent(path: &Path) -> (usize, Option<i64>, Option<i64>, Option<String>) {
    let Ok(file) = fs::File::open(path) else {
        return (0, None, None, None);
    };
    let mut message_count = 0usize;
    let mut first_message_time = None;
    let mut last_message_time = None;
    let mut summary = None;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        message_count += 1;
        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        if first_message_time.is_none() {
            first_message_time = ts;
        }
        if ts.is_some() {
            last_message_time = ts;
        }
        if summary.is_none() && value.get("role").and_then(Value::as_str) == Some("user") {
            let text = value
                .get("content")
                .map(collect_text_from_blocks)
                .unwrap_or_default();
            let cleaned = strip_system_reminder(&text);
            if !cleaned.trim().is_empty() {
                summary = Some(truncate_summary(&cleaned, 160));
            }
        }
    }
    (message_count, first_message_time, last_message_time, summary)
}

/// Subagent transcripts belonging to one session, for the detail sub-panel.
pub fn find_subagent_files(source: &Path) -> Vec<PathBuf> {
    let Some(parent) = source.parent() else {
        return Vec::new();
    };
    let Some(stem) = source.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };
    let subagents = parent.join(stem).join("subagents");
    if !subagents.is_dir() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = WalkDir::new(&subagents)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_transcript(dir: &Path, session_id: &str, lines: &[Value]) -> PathBuf {
        let projects = dir.join("projects").join("c-Users-test-repo");
        fs::create_dir_all(&projects).unwrap();
        let path = projects.join(format!("{session_id}.jsonl"));
        let mut file = fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        path
    }

    const SESSION_ID: &str = "f6290c12-d7cf-4fd4-87ee-54350c2561a6";

    #[test]
    fn a_session_is_listed_from_its_transcript_with_title_and_project() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_transcript(
            tmp.path(),
            SESSION_ID,
            &[
                serde_json::json!({
                    "type": "session-meta",
                    "id": "meta-1",
                    "sessionId": SESSION_ID,
                    "timestamp": 1_792_244_252_625i64,
                }),
                serde_json::json!({
                    "id": "msg-1",
                    "timestamp": 1_792_244_250_877i64,
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "explain the memory layout"}],
                    "sessionId": SESSION_ID,
                    "cwd": "c:\\Users\\MaRuJun\\.agents\\memories",
                }),
                serde_json::json!({
                    "id": "title-1",
                    "timestamp": 1_792_244_256_435i64,
                    "type": "ai-title",
                    "aiTitle": "探索记忆库结构设计",
                    "sessionId": SESSION_ID,
                }),
                serde_json::json!({
                    "id": "msg-2",
                    "timestamp": 1_792_244_263_759i64,
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "I will check the layout first."}],
                    "sessionId": SESSION_ID,
                }),
            ],
        );

        let sessions = scan_sessions(&tmp.path().join("projects"), FamilyProduct::Workbuddy);
        assert_eq!(sessions.len(), 1);
        let meta = &sessions[0];
        assert_eq!(meta.session_id, SESSION_ID);
        assert_eq!(meta.title.as_deref(), Some("探索记忆库结构设计"));
        assert_eq!(
            meta.project_dir.as_deref(),
            Some("c:\\Users\\MaRuJun\\.agents\\memories")
        );
        assert_eq!(meta.source_path, path.to_string_lossy());
        assert!(meta.summary.is_some());
        assert!(meta
            .resume_command
            .as_deref()
            .is_some_and(|command| command.contains("--resume")));
    }

    #[test]
    fn subagent_transcripts_and_rollback_logs_are_not_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_transcript(
            tmp.path(),
            SESSION_ID,
            &[serde_json::json!({
                "type": "message", "role": "user", "timestamp": 1i64,
                "content": [{"type": "input_text", "text": "hi"}],
            })],
        );
        let subagents = path.parent().unwrap().join(SESSION_ID).join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        // A child run has a UUID filename too; only its location excludes it.
        fs::write(
            subagents.join("agent-5c8de3ab-4fe3-4bc8-a7b0-0548d2b7d27f.jsonl"),
            "{\"type\":\"message\"}\n",
        )
        .unwrap();
        fs::write(
            path.parent()
                .unwrap()
                .join(format!("{SESSION_ID}.file-rollback.ndjson")),
            "{}\n",
        )
        .unwrap();

        let sessions = scan_sessions(&tmp.path().join("projects"), FamilyProduct::Workbuddy);
        assert_eq!(sessions.len(), 1, "only the transcript is a session");
        assert_eq!(sessions[0].session_id, SESSION_ID);
        assert_eq!(find_subagent_files(&path).len(), 1);
    }

    #[test]
    fn a_system_reminder_is_not_used_as_the_session_title() {
        let tmp = tempfile::tempdir().unwrap();
        write_transcript(
            tmp.path(),
            SESSION_ID,
            &[serde_json::json!({
                "type": "message",
                "role": "user",
                "timestamp": 1i64,
                "content": [{"type": "input_text", "text":
                    "<system-reminder data-role=\"user-context\">\nWorkspace Folder: x\n</system-reminder>\nreal question"}],
            })],
        );
        let sessions = scan_sessions(&tmp.path().join("projects"), FamilyProduct::Workbuddy);
        assert_eq!(sessions[0].title.as_deref(), Some("real question"));
    }

    #[test]
    fn detail_maps_every_conversation_item_type_to_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_transcript(
            tmp.path(),
            SESSION_ID,
            &[
                serde_json::json!({
                    "id": "m1", "timestamp": 1i64, "type": "message", "role": "user",
                    "content": [{"type": "input_text", "text": "read the file"}],
                }),
                serde_json::json!({
                    "id": "r1", "timestamp": 2i64, "type": "reasoning",
                    "content": [],
                    "rawContent": [{"type": "reasoning_text", "text": "thinking about it"}],
                }),
                serde_json::json!({
                    "id": "c1", "timestamp": 3i64, "type": "function_call",
                    "name": "Read", "callId": "call-1",
                    "input": {"file_path": "a.txt"},
                }),
                serde_json::json!({
                    "id": "cr1", "timestamp": 4i64, "type": "function_call_result",
                    "name": "Read", "callId": "call-1",
                    "output": {"type": "text", "text": "contents"},
                }),
                // Bookkeeping items must not surface as messages.
                serde_json::json!({"type": "file-history-snapshot", "timestamp": 5i64}),
                serde_json::json!({"type": "credit-usage", "timestamp": 6i64}),
            ],
        );

        let messages = load_messages(&path, FamilyProduct::Workbuddy).unwrap();
        assert_eq!(messages.len(), 4, "rollback/credit rows are not messages");
        assert!(messages.iter().all(|message| message.id.is_some()));

        let kinds: Vec<Vec<&str>> = messages
            .iter()
            .map(|message| message.blocks.iter().map(|block| block.kind.as_str()).collect())
            .collect();
        assert_eq!(kinds[0], vec!["text"]);
        assert_eq!(kinds[1], vec!["thinking"]);
        assert_eq!(kinds[2], vec!["tool_call"]);
        assert_eq!(kinds[3], vec!["tool_result"]);
    }

    #[test]
    fn search_matches_body_text_and_short_circuits_bookkeeping_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_transcript(
            tmp.path(),
            SESSION_ID,
            &[
                serde_json::json!({"type": "credit-usage", "timestamp": 1i64, "model": "needle"}),
                serde_json::json!({
                    "type": "message", "role": "user", "timestamp": 2i64,
                    "content": [{"type": "input_text", "text": "the needle is here"}],
                }),
            ],
        );
        assert!(scan_messages_for_query(&path, "needle").unwrap());
        assert!(!scan_messages_for_query(&path, "absent").unwrap());
    }

    #[test]
    fn deleting_a_session_removes_its_transcript_dir_and_rollback_log() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_transcript(
            tmp.path(),
            SESSION_ID,
            &[serde_json::json!({
                "type": "message", "role": "user", "timestamp": 1i64,
                "content": [{"type": "input_text", "text": "hi"}],
            })],
        );
        let parent = path.parent().unwrap().to_path_buf();
        let session_dir = parent.join(SESSION_ID);
        fs::create_dir_all(session_dir.join("subagents")).unwrap();
        fs::write(session_dir.join("subagents").join("agent-a.jsonl"), "{}\n").unwrap();
        let rollback = parent.join(format!("{SESSION_ID}.file-rollback.ndjson"));
        fs::write(&rollback, "{}\n").unwrap();

        delete_session(&path).unwrap();
        assert!(!path.exists());
        assert!(!session_dir.exists());
        assert!(!rollback.exists());

        // Deleting twice converges instead of erroring.
        delete_session(&path).unwrap();
    }

    /// Reads the developer's real `~/.workbuddy/projects` — read-only, never
    /// writes. Set `WORKBUDDY_SMOKE_PROJECTS` to run it:
    ///
    /// ```text
    /// WORKBUDDY_SMOKE_PROJECTS='C:\Users\you\.workbuddy\projects' \
    ///   cargo test --lib -- --ignored workbuddy_family_real_projects_smoke
    /// ```
    ///
    /// Fixtures cannot catch what only a real install produces: engine-written
    /// `session-meta`/`ai-title` placement, subagent directories, and the
    /// actual scale of a used profile.
    #[test]
    #[ignore]
    fn workbuddy_family_real_projects_smoke() {
        let Ok(raw) = std::env::var("WORKBUDDY_SMOKE_PROJECTS") else {
            eprintln!("WORKBUDDY_SMOKE_PROJECTS is unset; nothing to smoke");
            return;
        };
        let root = Path::new(&raw);
        assert!(root.is_dir(), "not a directory: {}", root.display());

        let sessions = scan_sessions(root, FamilyProduct::Workbuddy);
        assert!(!sessions.is_empty(), "real profile must expose at least one session");
        for meta in &sessions {
            assert!(!meta.session_id.is_empty());
            assert!(
                Path::new(&meta.source_path).is_file(),
                "listed source must exist: {}",
                meta.source_path
            );
        }

        // Detail + search must work on the largest transcript, which is the
        // case most likely to expose a streaming/parse defect.
        let mut largest = None;
        let mut largest_size = 0u64;
        for meta in &sessions {
            if let Ok(metadata) = fs::metadata(&meta.source_path) {
                if metadata.len() > largest_size {
                    largest_size = metadata.len();
                    largest = Some(meta);
                }
            }
        }
        let meta = largest.expect("largest session");
        let messages = load_messages(Path::new(&meta.source_path), FamilyProduct::Workbuddy)
            .expect("load real transcript");
        assert!(!messages.is_empty(), "real transcript must yield messages");
        assert!(messages.iter().all(|message| message.id.is_some()));
        let subagents = list_subagent_sessions(Path::new(&meta.source_path));
        eprintln!(
            "smoke: {} sessions, largest {} bytes -> {} messages, {} subagents",
            sessions.len(),
            largest_size,
            messages.len(),
            subagents.len()
        );
    }
}
