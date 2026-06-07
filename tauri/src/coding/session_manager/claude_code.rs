use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::message_blocks::{
    message_from_blocks, redacted_thinking_block, text_block, thinking_block, tool_call_block,
    tool_result_block, unknown_block, usage_from_value,
};
use super::utils::{
    build_resume_command, extract_prompt_title_text, extract_text, join_safe_relative,
    parse_timestamp_to_ms, path_basename, read_head_tail_lines, sanitize_path_segment,
    strip_path_prefix, text_contains_query, truncate_summary,
};
use super::{SessionMessage, SessionMessageBlock, SessionMeta, SessionSubagentMeta};

const PROVIDER_ID: &str = "claudecode";

pub fn scan_sessions(root: &Path) -> Vec<SessionMeta> {
    let indexed_sessions = scan_sessions_from_index(root);
    let indexed_paths: std::collections::HashSet<String> = indexed_sessions
        .iter()
        .map(|session| session.source_path.clone())
        .collect();
    let mut jsonl_files = Vec::new();
    collect_jsonl_files(root, &mut jsonl_files);

    let mut sessions = indexed_sessions;
    for path in jsonl_files {
        let resolved_path = path.to_string_lossy().to_string();
        if indexed_paths.contains(&resolved_path) {
            continue;
        }
        if let Some(session) = parse_session(&path) {
            sessions.push(session);
        }
    }

    sessions
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

        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        let message = match value.get("message") {
            Some(message) => message,
            None => continue,
        };

        let mut role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        if role == "user" {
            if let Some(Value::Array(items)) = message.get("content") {
                let all_tool_results = !items.is_empty()
                    && items.iter().all(|item| {
                        item.get("type").and_then(Value::as_str) == Some("tool_result")
                    });
                if all_tool_results {
                    role = "tool".to_string();
                }
            }
        }

        let blocks = claude_content_blocks(message.get("content"));
        let content = super::message_blocks::flatten_blocks_for_content(&blocks);
        if content.trim().is_empty() {
            continue;
        }

        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        let mut session_message = message_from_blocks(role, ts, blocks);
        session_message.id = value
            .get("uuid")
            .or_else(|| message.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        session_message.parent_id = value
            .get("parentUuid")
            .or_else(|| value.get("parent_uuid"))
            .and_then(Value::as_str)
            .map(str::to_string);
        session_message.message_type = value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
        session_message.model = message
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        session_message.usage = message.get("usage").and_then(usage_from_value);
        session_message.duration_ms = value
            .get("durationMs")
            .or_else(|| value.get("duration_ms"))
            .and_then(Value::as_i64);
        session_message.cost_usd = value
            .get("costUSD")
            .or_else(|| value.get("cost_usd"))
            .and_then(Value::as_f64);
        session_message.is_sidechain = value
            .get("isSidechain")
            .or_else(|| value.get("is_sidechain"))
            .and_then(Value::as_bool);
        messages.push(session_message);
    }

    Ok(messages)
}

pub fn list_subagent_sessions(parent_session_path: &Path) -> Vec<SessionSubagentMeta> {
    let subagent_files = find_subagent_files(parent_session_path);
    subagent_files
        .into_iter()
        .map(|path| build_subagent_meta(&path))
        .collect()
}

fn claude_content_blocks(content: Option<&Value>) -> Vec<SessionMessageBlock> {
    match content {
        Some(Value::String(text)) => vec![text_block(text.clone())],
        Some(Value::Array(items)) => items.iter().filter_map(claude_content_item_block).collect(),
        Some(Value::Object(map)) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                vec![text_block(text.to_string())]
            } else {
                vec![unknown_block(
                    extract_text(&Value::Object(map.clone())),
                    Some(Value::Object(map.clone())),
                )]
            }
        }
        _ => Vec::new(),
    }
}

fn claude_content_item_block(item: &Value) -> Option<SessionMessageBlock> {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
    match item_type {
        "text" => item
            .get("text")
            .and_then(Value::as_str)
            .map(|text| text_block(text.to_string())),
        "thinking" => item
            .get("thinking")
            .or_else(|| item.get("text"))
            .and_then(Value::as_str)
            .map(|text| thinking_block(text.to_string())),
        "redacted_thinking" => Some(redacted_thinking_block(
            item.get("data")
                .and_then(Value::as_str)
                .unwrap_or("Redacted thinking")
                .to_string(),
        )),
        "tool_use" => {
            let tool_name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let tool_id = item.get("id").and_then(Value::as_str).map(str::to_string);
            let input = item.get("input").cloned();
            Some(tool_call_block(tool_id, tool_name, input))
        }
        "tool_result" => {
            let tool_id = item
                .get("tool_use_id")
                .or_else(|| item.get("toolUseId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let output = item.get("content").cloned().or_else(|| Some(item.clone()));
            let is_error = item
                .get("is_error")
                .or_else(|| item.get("isError"))
                .and_then(Value::as_bool);
            Some(tool_result_block(tool_id, None, output, is_error))
        }
        _ => {
            let text = extract_text(item);
            if text.trim().is_empty() {
                Some(unknown_block(item.to_string(), Some(item.clone())))
            } else {
                Some(unknown_block(text, Some(item.clone())))
            }
        }
    }
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

        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        let Some(message) = value.get("message") else {
            continue;
        };
        let content = message.get("content").map(extract_text).unwrap_or_default();
        if text_contains_query(&content, query_lower) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn delete_session(path: &Path) -> Result<(), String> {
    let session_id = infer_session_id_from_filename(path).ok_or_else(|| {
        format!(
            "Failed to infer Claude Code session id from {}",
            path.display()
        )
    })?;
    let project_dir = path.parent().ok_or_else(|| {
        format!(
            "Failed to determine Claude Code project directory for {}",
            path.display()
        )
    })?;

    std::fs::remove_file(path)
        .map_err(|error| format!("Failed to delete session file {}: {error}", path.display()))?;
    remove_session_from_index(project_dir, &session_id)
}

pub fn export_native_snapshot(root: &Path, session_path: &Path) -> Result<Value, String> {
    let session = parse_session(session_path).ok_or_else(|| {
        format!(
            "Failed to parse Claude Code session {}",
            session_path.display()
        )
    })?;
    let relative_session_path = strip_path_prefix(root, session_path).ok_or_else(|| {
        format!(
            "Session path {} is outside Claude Code projects root {}",
            session_path.display(),
            root.display()
        )
    })?;
    let session_file_content = std::fs::read_to_string(session_path).map_err(|error| {
        format!(
            "Failed to read Claude Code session file {}: {error}",
            session_path.display()
        )
    })?;
    let project_relative_dir = session_path
        .parent()
        .and_then(|project_dir| strip_path_prefix(root, project_dir))
        .ok_or_else(|| {
            format!(
                "Failed to determine Claude Code project directory for {}",
                session_path.display()
            )
        })?;

    Ok(serde_json::json!({
        "projectRelativeDir": project_relative_dir,
        "sessionFileName": session_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "relativeSessionPath": relative_session_path,
        "sessionFileContent": session_file_content,
        "indexEntry": {
            "sessionId": session.session_id,
            "projectPath": session.project_dir,
            "summary": session.summary,
            "created": session.created_at,
            "modified": session.last_active_at,
            "firstPrompt": session.title,
        }
    }))
}

pub fn import_native_snapshot(
    root: &Path,
    session_id: &str,
    snapshot: &Value,
) -> Result<PathBuf, String> {
    let project_relative_dir = snapshot
        .get("projectRelativeDir")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Claude Code snapshot missing projectRelativeDir".to_string())?;
    let session_file_name = snapshot
        .get("sessionFileName")
        .and_then(Value::as_str)
        .filter(|value| value.ends_with(".jsonl"))
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("{}.jsonl", sanitize_path_segment(session_id, "session")));
    let session_file_content = snapshot
        .get("sessionFileContent")
        .and_then(Value::as_str)
        .ok_or_else(|| "Claude Code snapshot missing sessionFileContent".to_string())?;
    let index_entry = snapshot
        .get("indexEntry")
        .cloned()
        .ok_or_else(|| "Claude Code snapshot missing indexEntry".to_string())?;

    let project_dir = join_safe_relative(root, project_relative_dir)?;
    std::fs::create_dir_all(&project_dir).map_err(|error| {
        format!(
            "Failed to create Claude Code project directory {}: {error}",
            project_dir.display()
        )
    })?;

    let target_path = project_dir.join(&session_file_name);
    if target_path.exists() {
        return Err(format!(
            "Claude Code session file already exists: {}",
            target_path.display()
        ));
    }

    std::fs::write(&target_path, session_file_content).map_err(|error| {
        format!(
            "Failed to write Claude Code session file {}: {error}",
            target_path.display()
        )
    })?;

    upsert_sessions_index(&project_dir, &target_path, &index_entry)?;
    Ok(target_path)
}

fn scan_sessions_from_index(root: &Path) -> Vec<SessionMeta> {
    if !root.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    let project_entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    for project_entry in project_entries.flatten() {
        let project_path = project_entry.path();
        if !project_path.is_dir() {
            continue;
        }

        let index_path = project_path.join("sessions-index.json");
        let data = match std::fs::read_to_string(&index_path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(entries) = value.get("entries").and_then(Value::as_array) else {
            continue;
        };

        for entry in entries {
            let Some(session) = parse_index_entry(entry, &project_path) else {
                continue;
            };
            sessions.push(session);
        }
    }

    sessions
}

fn parse_index_entry(entry: &Value, project_path: &Path) -> Option<SessionMeta> {
    let session_id = entry.get("sessionId").and_then(Value::as_str)?.to_string();
    let full_path = entry
        .get("fullPath")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .unwrap_or_else(|| project_path.join(format!("{session_id}.jsonl")));
    if !full_path.exists() || is_agent_session(&full_path) {
        return None;
    }

    let project_dir = entry
        .get("projectPath")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| Some(project_path.to_string_lossy().to_string()));
    let resume_project_dir = entry.get("projectPath").and_then(Value::as_str);
    let created_at = entry.get("created").and_then(parse_timestamp_to_ms);
    let last_active_at = entry
        .get("modified")
        .or_else(|| entry.get("fileMtime"))
        .and_then(parse_timestamp_to_ms)
        .or(created_at);
    let summary = entry
        .get("summary")
        .or_else(|| entry.get("firstPrompt"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| truncate_summary(value, 160));
    let title = summary.clone().or_else(|| {
        project_dir
            .as_deref()
            .and_then(path_basename)
            .map(|value| value.to_string())
    });

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary,
        project_dir,
        created_at,
        last_active_at,
        source_path: full_path.to_string_lossy().to_string(),
        resume_command: Some(build_resume_command(
            resume_project_dir,
            &format!("claude --resume {session_id}"),
        )),
    })
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    if is_agent_session(path) {
        return None;
    }

    let (head, tail) = read_head_tail_lines(path, 20, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut slug: Option<String> = None;
    let mut title: Option<String> = None;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if session_id.is_none() {
            session_id = value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
        }
        if project_dir.is_none() {
            project_dir = value
                .get("cwd")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
        }
        if slug.is_none() {
            slug = value
                .get("slug")
                .and_then(Value::as_str)
                .and_then(format_slug_title);
        }
        if title.is_none() {
            title = extract_user_prompt_title(&value);
        }
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
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
        if summary.is_none() {
            if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(message) = value.get("message") {
                let text = message.get("content").map(extract_text).unwrap_or_default();
                if !text.trim().is_empty() {
                    summary = Some(text);
                }
            }
        }

        if last_active_at.is_some() && summary.is_some() {
            break;
        }
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path))?;
    let title = title.or(slug).or_else(|| {
        project_dir
            .as_deref()
            .and_then(path_basename)
            .map(|value| value.to_string())
    });
    let resume_command = build_resume_command(
        project_dir.as_deref(),
        &format!("claude --resume {session_id}"),
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

fn is_agent_session(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("agent-"))
        .unwrap_or(false)
}

fn find_subagent_files(parent_session_path: &Path) -> Vec<PathBuf> {
    let Some(parent_dir) = parent_session_path.parent() else {
        return Vec::new();
    };
    let Some(parent_stem) = parent_session_path
        .file_stem()
        .and_then(|value| value.to_str())
    else {
        return Vec::new();
    };

    let candidate_dirs = [
        parent_dir.join(parent_stem).join("subagents"),
        parent_dir.join("subagents").join(parent_stem),
    ];
    let mut files = Vec::new();
    for dir in candidate_dirs {
        let Ok(metadata) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if file_metadata.file_type().is_symlink()
                || !file_metadata.is_file()
                || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    files
}

fn build_subagent_meta(path: &Path) -> SessionSubagentMeta {
    let (message_count, first_message_time, last_message_time, summary) =
        extract_subagent_metadata(path);
    let file_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("subagent")
        .to_string();
    let id = file_stem
        .strip_prefix("agent-")
        .unwrap_or(&file_stem)
        .to_string();
    let title = summary.clone().unwrap_or_else(|| id.clone());

    SessionSubagentMeta {
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

fn extract_subagent_metadata(path: &Path) -> (usize, Option<i64>, Option<i64>, Option<String>) {
    let Ok(file) = File::open(path) else {
        return (0, None, None, None);
    };
    let reader = BufReader::new(file);
    let mut message_count = 0usize;
    let mut first_message_time = None;
    let mut last_message_time = None;
    let mut summary = None;

    for line in reader.lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };

        message_count += 1;
        if let Some(timestamp) = value.get("timestamp").and_then(parse_timestamp_to_ms) {
            first_message_time.get_or_insert(timestamp);
            last_message_time = Some(timestamp);
        }
        if summary.is_none()
            && message.get("role").and_then(Value::as_str) == Some("user")
            && !is_tool_result_only_content(message.get("content"))
        {
            let text = message.get("content").map(extract_text).unwrap_or_default();
            summary = normalize_title_text(&text);
        }
    }

    (
        message_count,
        first_message_time,
        last_message_time,
        summary,
    )
}

fn infer_session_id_from_filename(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
}

fn extract_user_prompt_title(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }

    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    if is_tool_result_only_content(message.get("content")) {
        return None;
    }

    let text = message.get("content").map(extract_text).unwrap_or_default();
    normalize_title_text(&text)
}

fn is_tool_result_only_content(content: Option<&Value>) -> bool {
    let Some(Value::Array(items)) = content else {
        return false;
    };

    !items.is_empty()
        && items
            .iter()
            .all(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
}

fn normalize_title_text(text: &str) -> Option<String> {
    extract_prompt_title_text(text, 80)
}

fn format_slug_title(slug: &str) -> Option<String> {
    let normalized = slug
        .split('-')
        .filter(|segment| !segment.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if normalized.is_empty() {
        return None;
    }

    Some(normalized)
}

fn upsert_sessions_index(
    project_dir: &Path,
    session_path: &Path,
    index_entry: &Value,
) -> Result<(), String> {
    let index_path = project_dir.join("sessions-index.json");
    let mut root_value = if index_path.exists() {
        let data = std::fs::read_to_string(&index_path).map_err(|error| {
            format!(
                "Failed to read Claude Code sessions index {}: {error}",
                index_path.display()
            )
        })?;
        serde_json::from_str::<Value>(&data).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if !root_value.is_object() {
        root_value = serde_json::json!({});
    }

    let entries = root_value
        .as_object_mut()
        .and_then(|map| {
            map.entry("entries")
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
        })
        .ok_or_else(|| {
            format!(
                "Invalid Claude Code sessions index structure: {}",
                index_path.display()
            )
        })?;

    let mut next_entry = index_entry.clone();
    let next_entry_map = next_entry
        .as_object_mut()
        .ok_or_else(|| "Claude Code snapshot indexEntry must be an object".to_string())?;
    next_entry_map.insert(
        "fullPath".to_string(),
        Value::String(session_path.to_string_lossy().to_string()),
    );

    let session_id = next_entry_map
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Claude Code snapshot indexEntry missing sessionId".to_string())?
        .to_string();

    if let Some(existing_entry) = entries.iter_mut().find(|entry| {
        entry
            .get("sessionId")
            .and_then(Value::as_str)
            .map(|value| value == session_id)
            .unwrap_or(false)
    }) {
        *existing_entry = next_entry;
    } else {
        entries.push(next_entry);
    }

    let serialized = serde_json::to_string_pretty(&root_value).map_err(|error| {
        format!(
            "Failed to serialize Claude Code sessions index {}: {error}",
            index_path.display()
        )
    })?;
    std::fs::write(&index_path, serialized).map_err(|error| {
        format!(
            "Failed to write Claude Code sessions index {}: {error}",
            index_path.display()
        )
    })?;

    Ok(())
}

fn remove_session_from_index(project_dir: &Path, session_id: &str) -> Result<(), String> {
    let index_path = project_dir.join("sessions-index.json");
    if !index_path.exists() {
        return Ok(());
    }

    let data = std::fs::read_to_string(&index_path).map_err(|error| {
        format!(
            "Failed to read Claude Code sessions index {}: {error}",
            index_path.display()
        )
    })?;
    let mut root_value =
        serde_json::from_str::<Value>(&data).unwrap_or_else(|_| serde_json::json!({}));
    let Some(entries) = root_value
        .as_object_mut()
        .and_then(|map| map.get_mut("entries"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };

    entries.retain(|entry| {
        entry
            .get("sessionId")
            .and_then(Value::as_str)
            .map(|value| value != session_id)
            .unwrap_or(true)
    });

    let serialized = serde_json::to_string_pretty(&root_value).map_err(|error| {
        format!(
            "Failed to serialize Claude Code sessions index {}: {error}",
            index_path.display()
        )
    })?;
    std::fs::write(&index_path, serialized).map_err(|error| {
        format!(
            "Failed to write Claude Code sessions index {}: {error}",
            index_path.display()
        )
    })?;

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{delete_session, scan_sessions};

    use std::fs;
    use std::path::{Path, PathBuf};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ai-toolbox-claude-session-{label}-{}",
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
    fn delete_session_removes_sessions_index_entry() {
        let test_dir = TestDir::new("delete-index-entry");
        let project_dir = test_dir.path().join("project");
        fs::create_dir_all(&project_dir).expect("failed to create project dir");

        let session_path = project_dir.join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
        fs::write(
            &session_path,
            "{\"sessionId\":\"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\",\"cwd\":\"/tmp/project\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-31T10:00:00Z\"}\n",
        )
        .expect("failed to write session file");

        let index_path = project_dir.join("sessions-index.json");
        let index_value = serde_json::json!({
            "entries": [
                {
                    "sessionId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "fullPath": session_path.to_string_lossy().to_string(),
                }
            ]
        });
        fs::write(
            &index_path,
            serde_json::to_string(&index_value).expect("failed to serialize sessions index"),
        )
        .expect("failed to write sessions index");

        delete_session(&session_path).expect("delete_session should succeed");

        let index_content =
            fs::read_to_string(&index_path).expect("failed to read sessions index after delete");
        assert!(
            !index_content.contains("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
            "sessions-index should remove deleted session entry"
        );
    }

    #[test]
    fn scan_sessions_prefixes_jsonl_resume_with_cwd() {
        let test_dir = TestDir::new("resume-jsonl-cwd");
        let project_dir = test_dir.path().join("project");
        fs::create_dir_all(&project_dir).expect("failed to create project dir");

        let session_path = project_dir.join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
        fs::write(
            &session_path,
            "{\"sessionId\":\"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\",\"cwd\":\"/Users/tester/project with space\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-31T10:00:00Z\"}\n",
        )
        .expect("failed to write session file");

        let sessions = scan_sessions(test_dir.path());

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].resume_command.as_deref(),
            Some("cd '/Users/tester/project with space' && claude --resume aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
    }

    #[test]
    fn scan_sessions_uses_index_project_path_for_resume() {
        let test_dir = TestDir::new("resume-index-project-path");
        let project_dir = test_dir.path().join("project");
        fs::create_dir_all(&project_dir).expect("failed to create project dir");

        let session_path = project_dir.join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
        fs::write(
            &session_path,
            "{\"sessionId\":\"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\",\"cwd\":\"/tmp/ignored\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-31T10:00:00Z\"}\n",
        )
        .expect("failed to write session file");

        let index_path = project_dir.join("sessions-index.json");
        let index_value = serde_json::json!({
            "entries": [
                {
                    "sessionId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "fullPath": session_path.to_string_lossy().to_string(),
                    "projectPath": "D:/GitHub/project with space"
                }
            ]
        });
        fs::write(
            &index_path,
            serde_json::to_string(&index_value).expect("failed to serialize sessions index"),
        )
        .expect("failed to write sessions index");

        let sessions = scan_sessions(test_dir.path());

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].resume_command.as_deref(),
            Some("pushd \"D:/GitHub/project with space\" && claude --resume aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
    }

    #[test]
    fn scan_sessions_keeps_bare_index_resume_without_project_path() {
        let test_dir = TestDir::new("resume-index-no-project-path");
        let project_dir = test_dir.path().join("project-cache-dir");
        fs::create_dir_all(&project_dir).expect("failed to create project dir");

        let session_path = project_dir.join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
        fs::write(
            &session_path,
            "{\"sessionId\":\"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\",\"cwd\":\"/tmp/ignored\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-31T10:00:00Z\"}\n",
        )
        .expect("failed to write session file");

        let index_path = project_dir.join("sessions-index.json");
        let index_value = serde_json::json!({
            "entries": [
                {
                    "sessionId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "fullPath": session_path.to_string_lossy().to_string()
                }
            ]
        });
        fs::write(
            &index_path,
            serde_json::to_string(&index_value).expect("failed to serialize sessions index"),
        )
        .expect("failed to write sessions index");

        let sessions = scan_sessions(test_dir.path());

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].resume_command.as_deref(),
            Some("claude --resume aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
    }
}
