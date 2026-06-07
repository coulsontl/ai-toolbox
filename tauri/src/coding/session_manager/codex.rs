use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::message_blocks::{
    message_from_blocks, text_block, tool_call_block, tool_result_block, usage_from_value,
};
use super::utils::{
    build_resume_command, extract_prompt_title_text, extract_text, join_safe_relative,
    parse_timestamp_to_ms, path_basename, read_head_tail_lines, sanitize_path_segment,
    strip_path_prefix, text_contains_query, truncate_summary,
};
use super::{SessionMessage, SessionMeta};

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

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file = File::open(path).map_err(|error| format!("Failed to open session file: {error}"))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }

        let payload = match value.get("payload") {
            Some(payload) => payload,
            None => continue,
        };

        let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        let mut message = match payload_type {
            "message" => {
                let role = payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let content = payload.get("content").map(extract_text).unwrap_or_default();
                if content.trim().is_empty() {
                    continue;
                }
                message_from_blocks(role, ts, vec![text_block(content)])
            }
            "function_call" => {
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let tool_id = payload
                    .get("call_id")
                    .or_else(|| payload.get("callId"))
                    .or_else(|| payload.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let input = payload
                    .get("arguments")
                    .or_else(|| payload.get("input"))
                    .cloned();
                message_from_blocks(
                    "assistant",
                    ts,
                    vec![tool_call_block(tool_id, name.to_string(), input)],
                )
            }
            "function_call_output" => {
                let tool_id = payload
                    .get("call_id")
                    .or_else(|| payload.get("callId"))
                    .or_else(|| payload.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let output = payload.get("output").cloned();
                message_from_blocks(
                    "tool",
                    ts,
                    vec![tool_result_block(tool_id, None, output, None)],
                )
            }
            _ => continue,
        };

        message.id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        message.message_type = Some(payload_type.to_string());
        message.model = payload
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        message.usage = payload.get("usage").and_then(usage_from_value);
        if message.content.trim().is_empty() {
            continue;
        }
        messages.push(message);
    }

    Ok(messages)
}

pub fn scan_messages_for_query(path: &Path, query_lower: &str) -> Result<bool, String> {
    let file = File::open(path).map_err(|error| format!("Failed to open session file: {error}"))?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }

        let payload = match value.get("payload") {
            Some(payload) => payload,
            None => continue,
        };

        let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
        let text = match payload_type {
            "message" => payload.get("content").map(extract_text).unwrap_or_default(),
            "function_call" => payload
                .get("name")
                .and_then(Value::as_str)
                .map(|name| format!("[Tool: {name}]"))
                .unwrap_or_default(),
            "function_call_output" => payload
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            _ => continue,
        };

        if text_contains_query(&text, query_lower) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn delete_session(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path)
        .map_err(|error| format!("Failed to delete session file {}: {error}", path.display()))
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
    let session_file_content = std::fs::read_to_string(session_path).map_err(|error| {
        format!(
            "Failed to read Codex session file {}: {error}",
            session_path.display()
        )
    })?;
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

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 30, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut title: Option<String> = None;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
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
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
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
    use super::scan_sessions;

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
}
