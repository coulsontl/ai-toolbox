use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::DateTime;
use serde_json::{json, Value};
use walkdir::WalkDir;

use super::message_blocks::{
    message_from_blocks, text_block, thinking_block, tool_call_block, tool_result_block,
};
use super::{assign_missing_message_ids, SessionMessage, SessionMeta};

pub fn scan_sessions(root: &Path) -> Vec<SessionMeta> {
    scan_recent_sessions(root, usize::MAX)
}

pub fn scan_recent_sessions(root: &Path, limit: usize) -> Vec<SessionMeta> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut sessions = WalkDir::new(root)
        .min_depth(3)
        .max_depth(3)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "summary.json")
        .filter_map(|entry| parse_summary(entry.path()).ok())
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| right.last_active_at.cmp(&left.last_active_at));
    sessions.truncate(limit);
    sessions
}

fn parse_summary(path: &Path) -> Result<SessionMeta, String> {
    let value: Value = serde_json::from_str(&fs::read_to_string(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let session_dir = path
        .parent()
        .ok_or_else(|| "Missing Grok session directory".to_string())?;
    let session_id = value
        .pointer("/info/id")
        .and_then(Value::as_str)
        .or_else(|| session_dir.file_name().and_then(|name| name.to_str()))
        .unwrap_or_default()
        .to_string();
    let project_dir = value
        .pointer("/info/cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(SessionMeta {
        provider_id: "grok".to_string(),
        session_id: session_id.clone(),
        title: value
            .get("generated_title")
            .and_then(Value::as_str)
            .or_else(|| value.get("session_summary").and_then(Value::as_str))
            .map(str::to_string),
        summary: value
            .get("session_summary")
            .and_then(Value::as_str)
            .map(str::to_string),
        project_dir: project_dir.clone(),
        created_at: parse_timestamp(value.get("created_at")),
        last_active_at: parse_timestamp(
            value
                .get("last_active_at")
                .or_else(|| value.get("updated_at")),
        ),
        source_path: session_dir.to_string_lossy().to_string(),
        resume_command: Some(super::utils::build_resume_command(
            project_dir.as_deref(),
            &format!("grok --resume {session_id}"),
        )),
        runtime_source: None,
        runtime_distro: None,
    })
}

fn parse_timestamp(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_str)
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.timestamp_millis())
}

pub fn load_messages(session_dir: &Path) -> Result<Vec<SessionMessage>, String> {
    let path = session_dir.join("chat_history.jsonl");
    let file =
        fs::File::open(&path).map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
    let mut messages = Vec::new();
    let mut tool_names = HashMap::<String, String>::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let role = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        if role == "system" {
            continue;
        }
        let mut blocks = Vec::new();
        match role.as_str() {
            "reasoning" => {
                let summary = content_text(value.get("summary"));
                if !summary.trim().is_empty() {
                    blocks.push(thinking_block(summary));
                }
            }
            "assistant" => {
                let content = content_text(value.get("content"));
                if !content.trim().is_empty() {
                    blocks.push(text_block(content));
                }
                if let Some(tool_calls) = value.get("tool_calls").and_then(Value::as_array) {
                    for tool_call in tool_calls {
                        let tool_id = tool_call
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let tool_name = tool_call
                            .get("name")
                            .or_else(|| tool_call.pointer("/function/name"))
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        if let Some(tool_id) = tool_id.as_ref() {
                            tool_names.insert(tool_id.clone(), tool_name.clone());
                        }
                        let input = tool_call
                            .get("arguments")
                            .or_else(|| tool_call.pointer("/function/arguments"))
                            .map(parse_tool_value);
                        blocks.push(tool_call_block(tool_id, tool_name, input));
                    }
                }
            }
            "tool_result" => {
                let tool_id = value
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let tool_name = tool_id.as_ref().and_then(|id| tool_names.get(id)).cloned();
                let output = value.get("content").map(parse_tool_value);
                let is_error = value
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .or_else(|| value.get("error").map(|error| !error.is_null()));
                blocks.push(tool_result_block(tool_id, tool_name, output, is_error));
            }
            _ => {
                let content = content_text(value.get("content"));
                if !content.trim().is_empty() {
                    blocks.push(text_block(content));
                }
            }
        }
        if blocks.is_empty() {
            continue;
        }
        let mut message =
            message_from_blocks(role.clone(), parse_timestamp(value.get("ts")), blocks);
        message.id = value.get("id").and_then(Value::as_str).map(str::to_string);
        message.message_type = Some(role);
        message.model = value
            .get("model_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        messages.push(message);
    }
    assign_missing_message_ids(&mut messages, "grok");
    Ok(messages)
}

fn parse_tool_value(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| value.clone())
}

fn content_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

pub fn scan_messages_for_query(path: &Path, query: &str) -> Result<bool, String> {
    Ok(load_messages(path)?
        .iter()
        .any(|message| message.content.to_lowercase().contains(query)))
}

pub fn delete_session(sessions_root: &Path, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let session_id = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Invalid Grok session path".to_string())?;
    match build_grok_command(sessions_root)?.args(["sessions", "delete", session_id]).output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) if output.status.code() == Some(127) => fs::remove_dir_all(path).map_err(
            |error| {
                format!(
                    "Grok CLI is unavailable and the session directory could not be deleted: {error}"
                )
            },
        ),
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if error.is_empty() {
                format!("Grok CLI failed to delete session {session_id}")
            } else {
                format!("Grok CLI failed to delete session {session_id}: {error}")
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::remove_dir_all(path).map_err(|error| {
                format!(
                    "Grok CLI is unavailable and the session directory could not be deleted: {error}"
                )
            })
        }
        Err(error) => Err(format!("Failed to start Grok CLI for session deletion: {error}")),
    }
}

pub fn export_markdown(
    sessions_root: &Path,
    session_id: &str,
    output_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create Grok export directory: {error}"))?;
    }
    let command_output_path = grok_command_path(sessions_root, output_path)?;
    let output = build_grok_command(sessions_root)?
        .args(["export", session_id])
        .arg(command_output_path)
        .output()
        .map_err(|error| format!("Failed to start Grok Markdown export: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if error.is_empty() {
        format!("Grok CLI failed to export session {session_id} as Markdown")
    } else {
        format!("Grok CLI failed to export session {session_id} as Markdown: {error}")
    })
}

fn build_grok_command(sessions_root: &Path) -> Result<Command, String> {
    let grok_home = sessions_root
        .parent()
        .ok_or_else(|| "Grok sessions root has no parent directory".to_string())?;
    if let Some(wsl) =
        crate::coding::runtime_location::parse_wsl_unc_path(&sessions_root.to_string_lossy())
    {
        let linux_grok_home = Path::new(&wsl.linux_path)
            .parent()
            .map(|path| path.to_string_lossy().to_string())
            .ok_or_else(|| "Grok WSL sessions path has no parent directory".to_string())?;
        let mut command = Command::new("wsl");
        command.args(["-d", &wsl.distro, "--exec", "env"]);
        command.arg(format!("GROK_HOME={linux_grok_home}"));
        command.arg("grok");
        return Ok(command);
    }

    let program = crate::coding::cli_resolver::resolve_local_grok_program();
    let mut command = crate::coding::cli_resolver::build_local_std_command(&program.path);
    command.env("GROK_HOME", grok_home);
    Ok(command)
}

fn grok_command_path(sessions_root: &Path, path: &Path) -> Result<PathBuf, String> {
    let Some(runtime_wsl) =
        crate::coding::runtime_location::parse_wsl_unc_path(&sessions_root.to_string_lossy())
    else {
        return Ok(path.to_path_buf());
    };
    if let Some(output_wsl) =
        crate::coding::runtime_location::parse_wsl_unc_path(&path.to_string_lossy())
    {
        if !output_wsl.distro.eq_ignore_ascii_case(&runtime_wsl.distro) {
            return Err(format!(
                "Grok WSL export target belongs to distro '{}', but the session belongs to '{}'",
                output_wsl.distro, runtime_wsl.distro
            ));
        }
        return Ok(PathBuf::from(output_wsl.linux_path));
    }

    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with('/') {
        return Ok(PathBuf::from(normalized));
    }
    let bytes = normalized.as_bytes();
    if normalized.len() >= 2 && bytes[1] == b':' {
        let drive = normalized
            .chars()
            .next()
            .ok_or_else(|| format!("Invalid Grok export path: {}", path.display()))?
            .to_ascii_lowercase();
        return Ok(PathBuf::from(format!("/mnt/{drive}{}", &normalized[2..])));
    }

    Err(format!(
        "Failed to convert Grok export path for WSL: {}",
        path.display()
    ))
}

pub fn export_native_snapshot(root: &Path, session_path: &Path) -> Result<Value, String> {
    let relative = session_path
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let mut files = serde_json::Map::new();
    for entry in WalkDir::new(session_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let name = entry
            .path()
            .strip_prefix(session_path)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(entry.path())
            .map_err(|e| format!("Failed to read {}: {e}", entry.path().display()))?;
        let content = match String::from_utf8(bytes.clone()) {
            Ok(text) => Value::String(text),
            Err(_) => json!({
                "encoding": "base64",
                "data": BASE64_STANDARD.encode(bytes),
            }),
        };
        files.insert(name, content);
    }
    Ok(json!({ "relativeDir": relative, "files": files }))
}

pub fn import_native_snapshot(
    root: &Path,
    session_id: &str,
    payload: &Value,
) -> Result<(), String> {
    let relative = payload
        .get("relativeDir")
        .and_then(Value::as_str)
        .unwrap_or(session_id);
    let relative = safe_relative_path(relative)?;
    reject_symlink_components(root, &relative)?;
    let target = root.join(relative);
    if fs::symlink_metadata(&target).is_ok() {
        return Err(format!("Grok session {session_id} already exists"));
    }
    let files = payload
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| "Invalid Grok session snapshot".to_string())?;
    for (name, content) in files {
        let relative_file = safe_relative_path(name)?;
        reject_symlink_components(&target, &relative_file)?;
        let path = target.join(relative_file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let bytes = if let Some(text) = content.as_str() {
            text.as_bytes().to_vec()
        } else if content.get("encoding").and_then(Value::as_str) == Some("base64") {
            let encoded = content
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "Invalid Grok base64 snapshot file".to_string())?;
            BASE64_STANDARD
                .decode(encoded)
                .map_err(|e| format!("Invalid Grok base64 snapshot file: {e}"))?
        } else {
            return Err(format!("Invalid Grok snapshot file payload: {name}"));
        };
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), String> {
    if fs::symlink_metadata(root)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "Grok session import root cannot be a symlink: {}",
            root.display()
        ));
    }

    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Grok session import path contains a symlink: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "Failed to inspect Grok session import path {}: {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || value.contains(':')
        || value
            .split(['/', '\\'])
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err("Invalid absolute or empty Grok snapshot path".to_string());
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Invalid Grok snapshot path traversal".to_string());
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_command_uses_target_distro_and_linux_paths() {
        let sessions_root = Path::new(r"\\wsl.localhost\Ubuntu\home\tester\.grok\sessions");
        let command = build_grok_command(sessions_root).expect("build WSL Grok command");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(command.get_program().to_string_lossy(), "wsl");
        assert_eq!(
            args,
            vec![
                "-d",
                "Ubuntu",
                "--exec",
                "env",
                "GROK_HOME=/home/tester/.grok",
                "grok",
            ]
        );
        assert_eq!(
            grok_command_path(sessions_root, Path::new(r"D:\Exports\session.md"))
                .expect("convert export path"),
            PathBuf::from("/mnt/d/Exports/session.md")
        );
        assert!(grok_command_path(
            sessions_root,
            Path::new(r"\\wsl.localhost\Debian\home\tester\session.md"),
        )
        .is_err());
    }

    #[test]
    fn scans_summary_and_loads_chat_history() {
        let temp = tempfile::tempdir().unwrap();
        let session_dir = temp.path().join("encoded-cwd/session-1");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("summary.json"), r#"{"info":{"id":"session-1","cwd":"/workspace/demo"},"generated_title":"Fix tests","session_summary":"Fix tests safely","created_at":"2026-07-12T03:14:16Z","last_active_at":"2026-07-12T03:15:29Z"}"#).unwrap();
        fs::write(
            session_dir.join("chat_history.jsonl"),
            concat!(
                "{\"type\":\"system\",\"content\":\"hidden\"}\n",
                "{\"type\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}\n",
                "{\"type\":\"assistant\",\"content\":\"world\"}\n"
            ),
        )
        .unwrap();
        let sessions = scan_sessions(temp.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "session-1");
        assert_eq!(sessions[0].project_dir.as_deref(), Some("/workspace/demo"));
        assert_eq!(
            sessions[0].resume_command.as_deref(),
            Some("cd /workspace/demo && grok --resume session-1")
        );
        let messages = load_messages(&session_dir).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec!["hello", "world"]
        );
        assert!(messages.iter().all(|message| message.id.is_some()));
    }

    #[test]
    fn native_snapshot_round_trip_preserves_session_files() {
        let source = tempfile::tempdir().unwrap();
        let session_dir = source.path().join("project/session-2");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("summary.json"),
            "{\"info\":{\"id\":\"session-2\"}}",
        )
        .unwrap();
        fs::write(
            session_dir.join("chat_history.jsonl"),
            "{\"type\":\"user\",\"content\":\"hello\"}\n",
        )
        .unwrap();
        fs::create_dir_all(session_dir.join("subagents/agent-1")).unwrap();
        fs::write(
            session_dir.join("subagents/agent-1/state.bin"),
            [0_u8, 159, 146, 150, 255],
        )
        .unwrap();
        fs::write(session_dir.join("rewind.json"), "{\"checkpoint\":1}").unwrap();
        let snapshot = export_native_snapshot(source.path(), &session_dir).unwrap();
        let target = tempfile::tempdir().unwrap();
        import_native_snapshot(target.path(), "session-2", &snapshot).unwrap();
        assert_eq!(
            fs::read_to_string(target.path().join("project/session-2/chat_history.jsonl")).unwrap(),
            "{\"type\":\"user\",\"content\":\"hello\"}\n"
        );
        assert_eq!(
            fs::read(
                target
                    .path()
                    .join("project/session-2/subagents/agent-1/state.bin")
            )
            .unwrap(),
            [0_u8, 159, 146, 150, 255]
        );
        assert_eq!(
            fs::read_to_string(target.path().join("project/session-2/rewind.json")).unwrap(),
            "{\"checkpoint\":1}"
        );
    }

    #[test]
    fn loads_reasoning_tool_calls_and_tool_results() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("chat_history.jsonl"),
            concat!(
                "{\"type\":\"reasoning\",\"summary\":[{\"text\":\"inspect files\"}]}\n",
                "{\"type\":\"assistant\",\"content\":\"\",\"tool_calls\":[{\"id\":\"call-1\",\"name\":\"run_terminal_command\",\"arguments\":\"{\\\"command\\\":\\\"pwd\\\"}\"}]}\n",
                "{\"type\":\"tool_result\",\"tool_call_id\":\"call-1\",\"content\":\"/workspace\"}\n"
            ),
        )
        .unwrap();

        let messages = load_messages(temp.path()).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].blocks[0].kind, "thinking");
        assert_eq!(messages[1].blocks[0].kind, "tool_call");
        assert_eq!(
            messages[1].blocks[0].tool_name.as_deref(),
            Some("run_terminal_command")
        );
        assert_eq!(messages[2].blocks[0].kind, "tool_result");
        assert_eq!(
            messages[2].blocks[0].tool_name.as_deref(),
            Some("run_terminal_command")
        );
    }

    #[test]
    fn native_snapshot_import_rejects_path_traversal() {
        let target = tempfile::tempdir().unwrap();
        let payload = json!({
            "relativeDir": "../escape",
            "files": { "summary.json": "{}" }
        });
        assert!(import_native_snapshot(target.path(), "session", &payload).is_err());

        let payload = json!({
            "relativeDir": "project/session",
            "files": { "../../escape.json": "{}" }
        });
        assert!(import_native_snapshot(target.path(), "session", &payload).is_err());
        assert!(!target.path().join("escape.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn native_snapshot_import_rejects_symlink_components() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("encoded-project")).unwrap();
        let payload = json!({
            "relativeDir": "encoded-project/session-1",
            "files": { "summary.json": "{}" }
        });

        let error = import_native_snapshot(&root, "session-1", &payload).unwrap_err();
        assert!(error.contains("symlink"));
        assert!(!outside.join("session-1/summary.json").exists());
    }
}
