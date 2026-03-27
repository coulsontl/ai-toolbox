use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use super::utils::{parse_timestamp_to_ms, path_basename, truncate_summary};
use super::{SessionMessage, SessionMeta};

const PROVIDER_ID: &str = "opencode";

pub fn scan_sessions(data_root: &Path, sqlite_db_path: &Path) -> Vec<SessionMeta> {
    let json_sessions = scan_sessions_json(data_root);
    let sqlite_sessions = scan_sessions_sqlite(sqlite_db_path);

    if sqlite_sessions.is_empty() {
        return json_sessions;
    }
    if json_sessions.is_empty() {
        return sqlite_sessions;
    }

    let sqlite_ids: std::collections::HashSet<String> = sqlite_sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect();

    let mut merged = sqlite_sessions;
    for session in json_sessions {
        if !sqlite_ids.contains(&session.session_id) {
            merged.push(session);
        }
    }

    merged
}

pub fn load_messages(source_path: &str) -> Result<Vec<SessionMessage>, String> {
    if source_path.starts_with("sqlite:") {
        return load_messages_sqlite(source_path);
    }

    load_messages_json(Path::new(source_path))
}

fn scan_sessions_json(data_root: &Path) -> Vec<SessionMeta> {
    let storage_root = data_root.join("storage");
    let session_root = storage_root.join("session");
    if !session_root.exists() {
        return Vec::new();
    }

    let mut json_files = Vec::new();
    collect_json_files(&session_root, &mut json_files);

    json_files
        .into_iter()
        .filter_map(|path| parse_session(&storage_root, &path))
        .collect()
}

fn scan_sessions_sqlite(sqlite_db_path: &Path) -> Vec<SessionMeta> {
    if !sqlite_db_path.exists() {
        return Vec::new();
    }

    let connection = match Connection::open_with_flags(
        sqlite_db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(_) => return Vec::new(),
    };

    let mut statement = match connection.prepare(
        "SELECT id, title, directory, time_created, time_updated FROM session ORDER BY time_updated DESC",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };

    let database_display = sqlite_db_path.display().to_string();
    let rows = match statement.query_map([], |row| {
        let session_id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let directory: String = row.get(2)?;
        let created_at: i64 = row.get(3)?;
        let last_active_at: i64 = row.get(4)?;
        Ok((session_id, title, directory, created_at, last_active_at))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    let mut sessions = Vec::new();
    for row in rows.flatten() {
        let (session_id, title, directory, created_at, last_active_at) = row;
        let display_title = if title.is_empty() {
            path_basename(&directory)
        } else {
            Some(title)
        };

        sessions.push(SessionMeta {
            provider_id: PROVIDER_ID.to_string(),
            session_id: session_id.clone(),
            title: display_title.clone(),
            summary: display_title,
            project_dir: if directory.is_empty() {
                None
            } else {
                Some(directory)
            },
            created_at: Some(created_at),
            last_active_at: Some(last_active_at),
            source_path: format!("sqlite:{}:{}", database_display, session_id),
            resume_command: Some(format!("opencode session resume {session_id}")),
        });
    }

    sessions
}

fn parse_sqlite_source(source: &str) -> Option<(PathBuf, String)> {
    let rest = source.strip_prefix("sqlite:")?;
    let separator = rest.rfind(":ses_")?;
    let database_path = PathBuf::from(&rest[..separator]);
    let session_id = rest[separator + 1..].to_string();
    Some((database_path, session_id))
}

fn load_messages_json(path: &Path) -> Result<Vec<SessionMessage>, String> {
    if !path.is_dir() {
        return Err(format!("Message directory not found: {}", path.display()));
    }

    let storage_root = path
        .parent()
        .and_then(|parent| parent.parent())
        .ok_or_else(|| "Cannot determine storage root from message path".to_string())?;

    let mut message_files = Vec::new();
    collect_json_files(path, &mut message_files);

    let mut entries: Vec<(i64, String, String, String)> = Vec::new();
    for message_path in &message_files {
        let data = match std::fs::read_to_string(message_path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let message_id = match value.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let role = value
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let created_at = value
            .get("time")
            .and_then(|time| time.get("created"))
            .and_then(parse_timestamp_to_ms)
            .unwrap_or(0);

        let part_dir = storage_root.join("part").join(&message_id);
        let content = collect_parts_text(&part_dir);
        if content.trim().is_empty() {
            continue;
        }

        entries.push((created_at, message_id, role, content));
    }

    entries.sort_by_key(|(timestamp, _, _, _)| *timestamp);

    Ok(entries
        .into_iter()
        .map(|(timestamp, _, role, content)| SessionMessage {
            role,
            content,
            ts: if timestamp > 0 { Some(timestamp) } else { None },
        })
        .collect())
}

fn load_messages_sqlite(source: &str) -> Result<Vec<SessionMessage>, String> {
    let (database_path, session_id) = parse_sqlite_source(source)
        .ok_or_else(|| format!("Invalid SQLite source reference: {source}"))?;

    let connection = Connection::open_with_flags(
        &database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;

    let mut message_statement = connection
        .prepare("SELECT id, time_created, data FROM message WHERE session_id = ?1 ORDER BY time_created ASC")
        .map_err(|error| format!("Failed to prepare message query: {error}"))?;
    let message_rows = message_statement
        .query_map([session_id.as_str()], |row| {
            let message_id: String = row.get(0)?;
            let timestamp: i64 = row.get(1)?;
            let data: String = row.get(2)?;
            Ok((message_id, timestamp, data))
        })
        .map_err(|error| format!("Failed to query messages: {error}"))?;

    let mut part_statement = connection
        .prepare(
            "SELECT message_id, data FROM part WHERE session_id = ?1 ORDER BY time_created ASC",
        )
        .map_err(|error| format!("Failed to prepare part query: {error}"))?;
    let part_rows = part_statement
        .query_map([session_id.as_str()], |row| {
            let message_id: String = row.get(0)?;
            let data: String = row.get(1)?;
            Ok((message_id, data))
        })
        .map_err(|error| format!("Failed to query parts: {error}"))?;

    let mut parts_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for row in part_rows.flatten() {
        let (message_id, data) = row;
        parts_map.entry(message_id).or_default().push(data);
    }

    let mut messages = Vec::new();
    for row in message_rows.flatten() {
        let (message_id, timestamp, data) = row;
        let message_value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let role = message_value
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let mut texts = Vec::new();
        if let Some(parts) = parts_map.get(&message_id) {
            for part_data in parts {
                let part_value: Value = match serde_json::from_str(part_data) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if let Some(text) = extract_part_text(&part_value) {
                    texts.push(text);
                }
            }
        }

        let content = texts.join("\n");
        if content.trim().is_empty() {
            continue;
        }

        messages.push(SessionMessage {
            role,
            content,
            ts: Some(timestamp),
        });
    }

    Ok(messages)
}

fn parse_session(storage_root: &Path, path: &Path) -> Option<SessionMeta> {
    let data = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&data).ok()?;

    let session_id = value.get("id").and_then(Value::as_str)?.to_string();
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let directory = value
        .get("directory")
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let created_at = value
        .get("time")
        .and_then(|time| time.get("created"))
        .and_then(parse_timestamp_to_ms);
    let last_active_at = value
        .get("time")
        .and_then(|time| time.get("updated"))
        .and_then(parse_timestamp_to_ms);

    let has_title = title.is_some();
    let display_title = title.or_else(|| {
        directory
            .as_deref()
            .and_then(path_basename)
            .map(|value| value.to_string())
    });

    let source_path = storage_root
        .join("message")
        .join(&session_id)
        .to_string_lossy()
        .to_string();
    let summary = if has_title {
        display_title.clone()
    } else {
        get_first_user_summary(storage_root, &session_id)
    };

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title: display_title,
        summary,
        project_dir: directory,
        created_at,
        last_active_at: last_active_at.or(created_at),
        source_path,
        resume_command: Some(format!("opencode session resume {session_id}")),
    })
}

fn get_first_user_summary(storage_root: &Path, session_id: &str) -> Option<String> {
    let message_dir = storage_root.join("message").join(session_id);
    if !message_dir.is_dir() {
        return None;
    }

    let mut message_files = Vec::new();
    collect_json_files(&message_dir, &mut message_files);

    let mut user_messages: Vec<(i64, String)> = Vec::new();
    for message_path in &message_files {
        let data = match std::fs::read_to_string(message_path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if value.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }

        let message_id = match value.get("id").and_then(Value::as_str) {
            Some(message_id) => message_id.to_string(),
            None => continue,
        };
        let timestamp = value
            .get("time")
            .and_then(|time| time.get("created"))
            .and_then(parse_timestamp_to_ms)
            .unwrap_or(0);

        user_messages.push((timestamp, message_id));
    }

    user_messages.sort_by_key(|(timestamp, _)| *timestamp);
    let (_, first_message_id) = user_messages.first()?;
    let part_dir = storage_root.join("part").join(first_message_id);
    let text = collect_parts_text(&part_dir);
    if text.trim().is_empty() {
        return None;
    }

    Some(truncate_summary(&text, 160))
}

fn extract_part_text(part_value: &Value) -> Option<String> {
    match part_value.get("type").and_then(Value::as_str) {
        Some("text") => part_value
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(|text| text.to_string()),
        Some("tool") => {
            let tool = part_value
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("[Tool: {tool}]"))
        }
        _ => None,
    }
}

fn collect_parts_text(part_dir: &Path) -> String {
    if !part_dir.is_dir() {
        return String::new();
    }

    let mut part_files = Vec::new();
    collect_json_files(part_dir, &mut part_files);
    part_files.sort();

    let mut texts = Vec::new();
    for part_path in &part_files {
        let data = match std::fs::read_to_string(part_path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(text) = extract_part_text(&value) {
            texts.push(text);
        }
    }

    texts.join("\n")
}

fn collect_json_files(root: &Path, files: &mut Vec<PathBuf>) {
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
            collect_json_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
}
