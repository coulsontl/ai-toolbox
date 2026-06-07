use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::message_blocks::text_message;
use super::utils::{
    extract_text, join_safe_relative, parse_timestamp_to_ms, path_basename, read_head_tail_lines,
    strip_path_prefix, text_contains_query, truncate_summary,
};
use super::{SessionMessage, SessionMeta};

const PROVIDER_ID: &str = "openclaw";

pub fn scan_sessions(agents_root: &Path) -> Vec<SessionMeta> {
    if !agents_root.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    let agent_entries = match std::fs::read_dir(agents_root) {
        Ok(entries) => entries,
        Err(_) => return sessions,
    };

    for agent_entry in agent_entries.flatten() {
        let agent_path = agent_entry.path();
        if !agent_path.is_dir() {
            continue;
        }

        let sessions_dir = agent_path.join("sessions");
        if !sessions_dir.is_dir() {
            continue;
        }

        let session_entries = match std::fs::read_dir(&sessions_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in session_entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == "sessions.json")
                .unwrap_or(false)
            {
                continue;
            }

            if let Some(session) = parse_session(&path) {
                sessions.push(session);
            }
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

        if value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }

        let message = match value.get("message") {
            Some(message) => message,
            None => continue,
        };

        let raw_role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let role = match raw_role {
            "toolResult" => "tool".to_string(),
            other => other.to_string(),
        };

        let content = message.get("content").map(extract_text).unwrap_or_default();
        if content.trim().is_empty() {
            continue;
        }

        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        messages.push(text_message(role, content, ts));
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

        if value.get("type").and_then(Value::as_str) != Some("message") {
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
    let session_id = parse_session(path)
        .map(|session| session.session_id)
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(|value| value.to_string())
        })
        .ok_or_else(|| {
            format!(
                "Failed to determine OpenClaw session id from {}",
                path.display()
            )
        })?;
    let sessions_dir = path.parent().ok_or_else(|| {
        format!(
            "Failed to determine OpenClaw sessions directory for {}",
            path.display()
        )
    })?;

    std::fs::remove_file(path)
        .map_err(|error| format!("Failed to delete session file {}: {error}", path.display()))?;
    remove_session_store_entry(sessions_dir, &session_id, path)
}

pub fn export_native_snapshot(agents_root: &Path, session_path: &Path) -> Result<Value, String> {
    let relative_session_path = strip_path_prefix(agents_root, session_path).ok_or_else(|| {
        format!(
            "Session path {} is outside OpenClaw agents root {}",
            session_path.display(),
            agents_root.display()
        )
    })?;
    let session_file_content = std::fs::read_to_string(session_path).map_err(|error| {
        format!(
            "Failed to read OpenClaw session file {}: {error}",
            session_path.display()
        )
    })?;
    let agent_id = session_path
        .parent()
        .and_then(Path::parent)
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "Failed to determine OpenClaw agent id from {}",
                session_path.display()
            )
        })?;
    let session_id = parse_session(session_path)
        .map(|session| session.session_id)
        .or_else(|| {
            session_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|value| value.to_string())
        })
        .ok_or_else(|| {
            format!(
                "Failed to determine OpenClaw session id from {}",
                session_path.display()
            )
        })?;
    let (session_key, session_store_entry) =
        read_session_store_entry(agents_root, agent_id, &session_id, session_path)?;

    Ok(serde_json::json!({
        "agentId": agent_id,
        "relativeSessionPath": relative_session_path,
        "sessionFileContent": session_file_content,
        "sessionKey": session_key,
        "sessionStoreEntry": session_store_entry,
    }))
}

pub fn import_native_snapshot(
    agents_root: &Path,
    session_id: &str,
    snapshot: &Value,
) -> Result<PathBuf, String> {
    let relative_session_path = snapshot
        .get("relativeSessionPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "OpenClaw snapshot missing relativeSessionPath".to_string())?;
    let session_file_content = snapshot
        .get("sessionFileContent")
        .and_then(Value::as_str)
        .ok_or_else(|| "OpenClaw snapshot missing sessionFileContent".to_string())?;
    let agent_id = snapshot
        .get("agentId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "OpenClaw snapshot missing agentId".to_string())?;
    let session_key = snapshot
        .get("sessionKey")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "OpenClaw snapshot missing sessionKey".to_string())?;

    let target_path = join_safe_relative(agents_root, relative_session_path)?;
    if target_path.exists() {
        return Err(format!(
            "OpenClaw session file already exists: {}",
            target_path.display()
        ));
    }

    let parent_dir = target_path.parent().ok_or_else(|| {
        format!(
            "Failed to determine OpenClaw session parent directory for {}",
            target_path.display()
        )
    })?;
    std::fs::create_dir_all(parent_dir).map_err(|error| {
        format!(
            "Failed to create OpenClaw session directory {}: {error}",
            parent_dir.display()
        )
    })?;
    std::fs::write(&target_path, session_file_content).map_err(|error| {
        format!(
            "Failed to write OpenClaw session file {}: {error}",
            target_path.display()
        )
    })?;

    upsert_session_store_entry(
        agents_root,
        agent_id,
        session_key,
        session_id,
        &target_path,
        snapshot.get("sessionStoreEntry"),
    )?;

    Ok(target_path)
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 10, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut summary: Option<String> = None;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }

        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if event_type == "session" {
            if session_id.is_none() {
                session_id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|value| value.to_string());
            }
            if cwd.is_none() {
                cwd = value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(|value| value.to_string());
            }
            if let Some(timestamp) = value.get("timestamp").and_then(parse_timestamp_to_ms) {
                created_at.get_or_insert(timestamp);
            }
            continue;
        }

        if event_type == "message" && summary.is_none() {
            if let Some(message) = value.get("message") {
                let text = message.get("content").map(extract_text).unwrap_or_default();
                if !text.trim().is_empty() {
                    summary = Some(text);
                }
            }
        }
    }

    let mut last_active_at: Option<i64> = None;
    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if let Some(timestamp) = value.get("timestamp").and_then(parse_timestamp_to_ms) {
            last_active_at = Some(timestamp);
            break;
        }
    }

    let session_id = session_id.or_else(|| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .map(|value| value.to_string())
    })?;

    let title = cwd
        .as_deref()
        .and_then(path_basename)
        .map(|value| value.to_string());

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id,
        title,
        summary: summary.map(|text| truncate_summary(&text, 160)),
        project_dir: cwd,
        created_at,
        last_active_at,
        source_path: path.to_string_lossy().to_string(),
        resume_command: None,
    })
}

fn read_session_store_entry(
    agents_root: &Path,
    agent_id: &str,
    session_id: &str,
    session_path: &Path,
) -> Result<(String, Value), String> {
    let store_path = agents_root
        .join(agent_id)
        .join("sessions")
        .join("sessions.json");
    if !store_path.exists() {
        return Ok((
            format!("agent:{agent_id}:{session_id}"),
            serde_json::json!({
                "sessionId": session_id,
                "updatedAt": chrono::Utc::now().timestamp_millis(),
                "sessionFile": session_path.to_string_lossy().to_string(),
            }),
        ));
    }

    let data = std::fs::read_to_string(&store_path).map_err(|error| {
        format!(
            "Failed to read OpenClaw session store {}: {error}",
            store_path.display()
        )
    })?;
    let store_value: Value = serde_json::from_str(&data).map_err(|error| {
        format!(
            "Failed to parse OpenClaw session store {}: {error}",
            store_path.display()
        )
    })?;
    let store_map = store_value.as_object().ok_or_else(|| {
        format!(
            "Invalid OpenClaw session store structure: {}",
            store_path.display()
        )
    })?;

    let session_path_display = session_path.to_string_lossy().to_string();
    if let Some((key, entry)) = store_map.iter().find(|(_, entry)| {
        entry
            .get("sessionId")
            .and_then(Value::as_str)
            .map(|value| value == session_id)
            .unwrap_or(false)
            || entry
                .get("sessionFile")
                .and_then(Value::as_str)
                .map(|value| value == session_path_display)
                .unwrap_or(false)
    }) {
        return Ok((key.clone(), entry.clone()));
    }

    Ok((
        format!("agent:{agent_id}:{session_id}"),
        serde_json::json!({
            "sessionId": session_id,
            "updatedAt": chrono::Utc::now().timestamp_millis(),
            "sessionFile": session_path_display,
        }),
    ))
}

fn upsert_session_store_entry(
    agents_root: &Path,
    agent_id: &str,
    session_key: &str,
    session_id: &str,
    target_path: &Path,
    existing_entry: Option<&Value>,
) -> Result<(), String> {
    let sessions_dir = agents_root.join(agent_id).join("sessions");
    std::fs::create_dir_all(&sessions_dir).map_err(|error| {
        format!(
            "Failed to create OpenClaw sessions directory {}: {error}",
            sessions_dir.display()
        )
    })?;
    let store_path = sessions_dir.join("sessions.json");
    let mut store_value = if store_path.exists() {
        let data = std::fs::read_to_string(&store_path).map_err(|error| {
            format!(
                "Failed to read OpenClaw session store {}: {error}",
                store_path.display()
            )
        })?;
        serde_json::from_str::<Value>(&data).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if !store_value.is_object() {
        store_value = serde_json::json!({});
    }

    let store_map = store_value.as_object_mut().ok_or_else(|| {
        format!(
            "Invalid OpenClaw session store structure: {}",
            store_path.display()
        )
    })?;
    if let Some(existing_key_entry) = store_map.get(session_key) {
        let existing_session_id = existing_key_entry
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !existing_session_id.is_empty() && existing_session_id != session_id {
            return Err(format!(
                "OpenClaw session key {} is already bound to session {}",
                session_key, existing_session_id
            ));
        }
    }

    let mut next_entry = existing_entry
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if !next_entry.is_object() {
        next_entry = serde_json::json!({});
    }
    let entry_map = next_entry
        .as_object_mut()
        .ok_or_else(|| "Invalid OpenClaw session store entry".to_string())?;
    entry_map.insert(
        "sessionId".to_string(),
        Value::String(session_id.to_string()),
    );
    entry_map.insert(
        "updatedAt".to_string(),
        Value::Number(serde_json::Number::from(
            chrono::Utc::now().timestamp_millis(),
        )),
    );
    entry_map.insert(
        "sessionFile".to_string(),
        Value::String(target_path.to_string_lossy().to_string()),
    );

    store_map.insert(session_key.to_string(), next_entry);

    let serialized = serde_json::to_string_pretty(&store_value).map_err(|error| {
        format!(
            "Failed to serialize OpenClaw session store {}: {error}",
            store_path.display()
        )
    })?;
    std::fs::write(&store_path, serialized).map_err(|error| {
        format!(
            "Failed to write OpenClaw session store {}: {error}",
            store_path.display()
        )
    })?;

    Ok(())
}

fn remove_session_store_entry(
    sessions_dir: &Path,
    session_id: &str,
    session_path: &Path,
) -> Result<(), String> {
    let store_path = sessions_dir.join("sessions.json");
    if !store_path.exists() {
        return Ok(());
    }

    let data = std::fs::read_to_string(&store_path).map_err(|error| {
        format!(
            "Failed to read OpenClaw session store {}: {error}",
            store_path.display()
        )
    })?;
    let mut store_value =
        serde_json::from_str::<Value>(&data).unwrap_or_else(|_| serde_json::json!({}));
    let Some(store_map) = store_value.as_object_mut() else {
        return Ok(());
    };

    let session_path_display = session_path.to_string_lossy().to_string();
    store_map.retain(|_, entry| {
        let same_session_id = entry
            .get("sessionId")
            .and_then(Value::as_str)
            .map(|value| value == session_id)
            .unwrap_or(false);
        let same_session_file = entry
            .get("sessionFile")
            .and_then(Value::as_str)
            .map(|value| value == session_path_display)
            .unwrap_or(false);
        !(same_session_id || same_session_file)
    });

    let serialized = serde_json::to_string_pretty(&store_value).map_err(|error| {
        format!(
            "Failed to serialize OpenClaw session store {}: {error}",
            store_path.display()
        )
    })?;
    std::fs::write(&store_path, serialized).map_err(|error| {
        format!(
            "Failed to write OpenClaw session store {}: {error}",
            store_path.display()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::delete_session;

    use std::fs;
    use std::path::{Path, PathBuf};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ai-toolbox-openclaw-session-{label}-{}",
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
    fn delete_session_removes_sessions_json_entry() {
        let test_dir = TestDir::new("delete-store-entry");
        let sessions_dir = test_dir.path().join("agent-a").join("sessions");
        fs::create_dir_all(&sessions_dir).expect("failed to create sessions dir");

        let session_path = sessions_dir.join("session-a.jsonl");
        fs::write(
            &session_path,
            "{\"type\":\"session\",\"id\":\"session-a\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-31T10:00:00Z\"}\n",
        )
        .expect("failed to write session file");

        let store_path = sessions_dir.join("sessions.json");
        fs::write(
            &store_path,
            format!(
                "{{\"agent:agent-a:session-a\":{{\"sessionId\":\"session-a\",\"sessionFile\":\"{}\"}}}}",
                session_path.to_string_lossy()
            ),
        )
        .expect("failed to write sessions store");

        delete_session(&session_path).expect("delete_session should succeed");

        let store_content =
            fs::read_to_string(&store_path).expect("failed to read sessions store after delete");
        assert!(
            !store_content.contains("\"session-a\""),
            "sessions.json should remove deleted session entry"
        );
    }
}
