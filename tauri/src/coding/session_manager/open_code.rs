use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Map, Value};

use super::utils::{parse_timestamp_to_ms, path_basename, text_contains_query, truncate_summary};
use super::{SessionMessage, SessionMeta};
use crate::coding::runtime_location::{RuntimeLocationInfo, RuntimeLocationMode};

const PROVIDER_ID: &str = "opencode";

fn format_command_context(
    runtime_location: &RuntimeLocationInfo,
    config_path: Option<&Path>,
    data_root: Option<&Path>,
    state_root: Option<&Path>,
    working_directory: Option<&Path>,
) -> String {
    let config_display = config_path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let data_root_display = data_root
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let state_root_display = state_root
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let working_directory_display = working_directory
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let runtime_mode = match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => "local",
        RuntimeLocationMode::WslDirect => "wsl",
    };

    format!(
        "runtime={runtime_mode}, runtime_path={}, config_path={config_display}, data_root={data_root_display}, state_root={state_root_display}, working_directory={working_directory_display}",
        runtime_location.host_path.display()
    )
}

fn summarize_command_output(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }

    const MAX_PREVIEW_CHARS: usize = 300;
    let mut preview = trimmed.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if trimmed.chars().count() > MAX_PREVIEW_CHARS {
        preview.push_str("...");
    }
    preview
}

fn build_missing_local_opencode_cli_message(details: &str) -> String {
    format!(
        "OpenCode 会话导入/导出需要先安装 OpenCode CLI，并确保当前系统环境可以找到 `opencode` 命令。详情: {details}"
    )
}

fn build_missing_wsl_opencode_cli_message(distro: &str, details: &str) -> String {
    format!(
        "OpenCode 会话导入/导出需要在 WSL 发行版 `{distro}` 中安装 OpenCode CLI，并确保 `opencode` 命令可用。详情: {details}"
    )
}

fn build_missing_local_opencode_spawn_message(
    error: &std::io::Error,
    command_name: &str,
    command_context: &str,
) -> String {
    let runtime_error = format!("Failed to run `{command_name}`: {error} ({command_context})");
    if error.kind() == std::io::ErrorKind::NotFound {
        build_missing_local_opencode_cli_message(&runtime_error)
    } else {
        runtime_error
    }
}

#[cfg(target_os = "windows")]
fn windows_command_path_priority(path: &Path) -> usize {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("exe") => 0,
        Some("cmd") => 1,
        Some("bat") => 2,
        Some("com") => 3,
        Some("ps1") => 4,
        _ => 5,
    }
}

fn parse_where_command_output(stdout: &str) -> Vec<PathBuf> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "windows")]
fn select_windows_opencode_command_path(paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .min_by_key(|path| windows_command_path_priority(path))
        .cloned()
}

#[cfg(not(target_os = "windows"))]
fn select_local_opencode_command_path(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.first().cloned()
}

#[cfg(target_os = "windows")]
fn select_local_opencode_command_path(paths: &[PathBuf]) -> Option<PathBuf> {
    select_windows_opencode_command_path(paths)
}

#[cfg(target_os = "windows")]
fn build_local_windows_opencode_command(program_path: &Path) -> Command {
    match program_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("cmd") | Some("bat") => {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(program_path);
            command
        }
        Some("ps1") => {
            let mut command = Command::new("powershell");
            command
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(program_path);
            command
        }
        _ => Command::new(program_path),
    }
}

#[cfg(target_os = "windows")]
fn build_local_opencode_command(program_path: &Path) -> Command {
    build_local_windows_opencode_command(program_path)
}

#[cfg(not(target_os = "windows"))]
fn build_local_opencode_command(program_path: &Path) -> Command {
    Command::new(program_path)
}

fn resolve_local_opencode_program() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let lookup_command = "where";

    #[cfg(not(target_os = "windows"))]
    let lookup_command = "which";

    let output = Command::new(lookup_command).arg("opencode").output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let candidates = parse_where_command_output(&stdout);
    select_local_opencode_command_path(&candidates)
}

#[cfg(target_os = "windows")]
fn build_fallback_local_opencode_command() -> Command {
    build_local_windows_opencode_command(Path::new("opencode"))
}

#[cfg(not(target_os = "windows"))]
fn build_fallback_local_opencode_command() -> Command {
    Command::new("opencode")
}

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

pub fn scan_messages_for_query(source_path: &str, query_lower: &str) -> Result<bool, String> {
    if source_path.starts_with("sqlite:") {
        return scan_messages_for_query_sqlite(source_path, query_lower);
    }

    scan_messages_for_query_json(Path::new(source_path), query_lower)
}

pub fn delete_session(source_path: &str) -> Result<(), String> {
    if source_path.starts_with("sqlite:") {
        let (database_path, session_id) = parse_sqlite_source(source_path)
            .ok_or_else(|| format!("Invalid SQLite source reference: {source_path}"))?;
        delete_session_from_sqlite(&database_path, &session_id)?;
        delete_session_json_artifacts(
            &database_path.parent().unwrap_or(Path::new("")),
            &session_id,
        )?;
        return Ok(());
    }

    let message_dir = Path::new(source_path);
    let session_id = message_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "Invalid OpenCode message directory: {}",
                message_dir.display()
            )
        })?
        .to_string();
    let storage_root = message_dir
        .parent()
        .and_then(|parent| parent.parent())
        .ok_or_else(|| {
            format!(
                "Cannot determine storage root from {}",
                message_dir.display()
            )
        })?;
    let data_root = storage_root.parent().ok_or_else(|| {
        format!(
            "Cannot determine OpenCode data root from {}",
            storage_root.display()
        )
    })?;

    delete_session_from_sqlite(&data_root.join("opencode.db"), &session_id)?;
    delete_session_json_artifacts(data_root, &session_id)
}

pub fn rename_session(source_path: &str, next_title: &str) -> Result<(), String> {
    let normalized_title = next_title.trim();
    if normalized_title.is_empty() {
        return Err("Session title cannot be empty".to_string());
    }

    let (data_root, database_path, session_id) = if source_path.starts_with("sqlite:") {
        let (database_path, session_id) = parse_sqlite_source(source_path)
            .ok_or_else(|| format!("Invalid SQLite source reference: {source_path}"))?;
        let data_root = database_path.parent().ok_or_else(|| {
            format!(
                "Cannot determine OpenCode data root from {}",
                database_path.display()
            )
        })?;
        (data_root.to_path_buf(), database_path, session_id)
    } else {
        let message_dir = Path::new(source_path);
        let session_id = message_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "Invalid OpenCode message directory: {}",
                    message_dir.display()
                )
            })?
            .to_string();
        let storage_root = message_dir
            .parent()
            .and_then(|parent| parent.parent())
            .ok_or_else(|| {
                format!(
                    "Cannot determine storage root from {}",
                    message_dir.display()
                )
            })?;
        let data_root = storage_root.parent().ok_or_else(|| {
            format!(
                "Cannot determine OpenCode data root from {}",
                storage_root.display()
            )
        })?;
        (
            data_root.to_path_buf(),
            data_root.join("opencode.db"),
            session_id,
        )
    };

    update_session_title_in_sqlite(&database_path, &session_id, normalized_title)?;
    update_session_title_in_json(&data_root, &session_id, normalized_title)?;
    Ok(())
}

pub fn export_native_snapshot(
    source_path: &str,
    meta: &SessionMeta,
    messages: &[SessionMessage],
    runtime_location: &RuntimeLocationInfo,
    config_path: Option<&Path>,
    data_root: Option<&Path>,
    state_root: Option<&Path>,
) -> Result<Value, String> {
    let session_id = extract_session_id_from_source(source_path)?;
    let command_context =
        format_command_context(runtime_location, config_path, data_root, state_root, None);
    let mut command =
        build_opencode_command(runtime_location, config_path, data_root, state_root, None)?;
    command.arg("export").arg(&session_id);
    let output = command
        .output()
        .map_err(|error| match runtime_location.mode {
            RuntimeLocationMode::LocalWindows => build_missing_local_opencode_spawn_message(
                &error,
                &format!("opencode export {session_id}"),
                &command_context,
            ),
            RuntimeLocationMode::WslDirect => {
                let runtime_error = format!(
                    "Failed to run `opencode export {session_id}`: {error} ({command_context})"
                );
                let distro = runtime_location
                    .wsl
                    .as_ref()
                    .map(|wsl| wsl.distro.as_str())
                    .unwrap_or("unknown");
                build_missing_wsl_opencode_cli_message(distro, &runtime_error)
            }
        })?;

    if !output.status.success() {
        let stderr_preview = summarize_command_output(&output.stderr);
        let stdout_preview = summarize_command_output(&output.stdout);
        return Err(format!(
            "`opencode export {session_id}` failed with status {} ({command_context}). stderr: {}; stdout: {}",
            output.status, stderr_preview, stdout_preview
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("OpenCode export output is not valid UTF-8: {error}"))?;
    let exported_json = serde_json::from_str::<Value>(&stdout).ok().or_else(|| {
        Some(build_recovered_official_export(
            &session_id,
            Some(meta),
            Some(messages),
            meta.project_dir.as_deref(),
        ))
    });
    let mut payload = Map::new();
    payload.insert("sessionId".to_string(), Value::String(session_id));
    if let Some(exported_json) = exported_json {
        let exported_raw = serde_json::to_string_pretty(&exported_json).map_err(|error| {
            format!("Failed to serialize recovered OpenCode official export: {error}")
        })?;
        payload.insert("officialExportRaw".to_string(), Value::String(exported_raw));
        payload.insert("officialExport".to_string(), exported_json);
    } else {
        payload.insert("officialExportRaw".to_string(), Value::String(stdout));
    }

    Ok(Value::Object(payload))
}

pub fn import_native_snapshot(
    snapshot: &Value,
    meta: Option<&SessionMeta>,
    normalized_messages: Option<&[SessionMessage]>,
    preferred_project_dir: Option<&str>,
    runtime_location: &RuntimeLocationInfo,
    config_path: Option<&Path>,
    data_root: Option<&Path>,
    state_root: Option<&Path>,
) -> Result<(), String> {
    let session_id = extract_session_id_from_snapshot(snapshot)?;
    let serialized = if let Some(official_export) = snapshot.get("officialExport").cloned() {
        serde_json::to_string_pretty(&official_export)
            .map_err(|error| format!("Failed to serialize OpenCode official export: {error}"))?
    } else if let Some(official_export_raw) =
        snapshot.get("officialExportRaw").and_then(Value::as_str)
    {
        match serde_json::from_str::<Value>(official_export_raw) {
            Ok(_) => official_export_raw.to_string(),
            Err(_) => serde_json::to_string_pretty(&build_recovered_official_export(
                &session_id,
                meta,
                normalized_messages,
                preferred_project_dir,
            ))
            .map_err(|error| {
                format!("Failed to serialize recovered OpenCode official export: {error}")
            })?,
        }
    } else {
        serde_json::to_string_pretty(&build_recovered_official_export(
            &session_id,
            meta,
            normalized_messages,
            preferred_project_dir,
        ))
        .map_err(|error| {
            format!("Failed to serialize recovered OpenCode official export: {error}")
        })?
    };

    let temp_path = std::env::temp_dir().join(format!(
        "ai-toolbox-opencode-import-{}.json",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&temp_path, serialized).map_err(|error| {
        format!(
            "Failed to write temporary OpenCode import file {}: {error}",
            temp_path.display()
        )
    })?;

    let runtime_project_dir = preferred_project_dir
        .map(|project_dir| resolve_runtime_project_dir(runtime_location, project_dir))
        .transpose()?
        .flatten();
    let command_context = format_command_context(
        runtime_location,
        config_path,
        data_root,
        state_root,
        runtime_project_dir.as_deref(),
    );
    let mut command = build_opencode_command(
        runtime_location,
        config_path,
        data_root,
        state_root,
        runtime_project_dir.as_deref(),
    )?;
    let import_argument = match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => temp_path.to_string_lossy().to_string(),
        RuntimeLocationMode::WslDirect => convert_to_wsl_command_path(&temp_path)?,
    };
    command.arg("import").arg(import_argument);

    let output = command
        .output()
        .map_err(|error| match runtime_location.mode {
            RuntimeLocationMode::LocalWindows => build_missing_local_opencode_spawn_message(
                &error,
                "opencode import",
                &command_context,
            ),
            RuntimeLocationMode::WslDirect => {
                let runtime_error =
                    format!("Failed to run `opencode import`: {error} ({command_context})");
                let distro = runtime_location
                    .wsl
                    .as_ref()
                    .map(|wsl| wsl.distro.as_str())
                    .unwrap_or("unknown");
                build_missing_wsl_opencode_cli_message(distro, &runtime_error)
            }
        })?;
    let _ = std::fs::remove_file(&temp_path);

    if output.status.success() {
        let stderr_preview = summarize_command_output(&output.stderr);
        let stdout_preview = summarize_command_output(&output.stdout);
        if stderr_preview.contains("File not found:") || stdout_preview.contains("File not found:")
        {
            return Err(format!(
                "`opencode import` reported success but could not read the import file ({command_context}). stderr: {}; stdout: {}",
                stderr_preview, stdout_preview
            ));
        }
        if let Some(data_root) = data_root {
            ensure_imported_session_visible(data_root, &session_id, &command_context)?;
        }
        return Ok(());
    }

    let stderr_preview = summarize_command_output(&output.stderr);
    let stdout_preview = summarize_command_output(&output.stdout);
    Err(format!(
        "`opencode import` failed with status {} ({command_context}). stderr: {}; stdout: {}",
        output.status, stderr_preview, stdout_preview
    ))
}

fn extract_session_id_from_snapshot(snapshot: &Value) -> Result<String, String> {
    if let Some(session_id) = snapshot
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(session_id.to_string());
    }

    if let Some(session_id) = snapshot
        .pointer("/officialExport/info/id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(session_id.to_string());
    }

    if let Some(official_export_raw) = snapshot.get("officialExportRaw").and_then(Value::as_str) {
        let official_export_value: Value =
            serde_json::from_str(official_export_raw).map_err(|error| {
                format!("Failed to parse OpenCode official export raw JSON: {error}")
            })?;
        if let Some(session_id) = official_export_value
            .pointer("/info/id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(session_id.to_string());
        }
    }

    Err("OpenCode snapshot missing sessionId".to_string())
}

fn build_recovered_official_export(
    session_id: &str,
    meta: Option<&SessionMeta>,
    normalized_messages: Option<&[SessionMessage]>,
    preferred_project_dir: Option<&str>,
) -> Value {
    let project_dir = meta
        .and_then(|item| item.project_dir.as_deref())
        .or(preferred_project_dir)
        .unwrap_or("")
        .to_string();
    let created_at = meta
        .and_then(|item| item.created_at)
        .or_else(|| {
            normalized_messages
                .and_then(|messages| messages.iter().filter_map(|message| message.ts).min())
        })
        .unwrap_or(0);
    let updated_at = meta
        .and_then(|item| item.last_active_at)
        .or_else(|| meta.and_then(|item| item.created_at))
        .or_else(|| {
            normalized_messages
                .and_then(|messages| messages.iter().filter_map(|message| message.ts).max())
        })
        .unwrap_or(created_at);
    let title = meta
        .and_then(|item| item.title.as_deref())
        .or_else(|| meta.and_then(|item| item.summary.as_deref()))
        .unwrap_or("Imported Session")
        .to_string();

    let source_messages = normalized_messages.unwrap_or(&[]);
    let needs_leading_parent = source_messages
        .first()
        .map(|message| message.role == "assistant")
        .unwrap_or(false);
    let synthetic_parent_id = "msg_recovered_parent_000000".to_string();
    let synthetic_part_id = "prt_recovered_parent_000000".to_string();
    let synthetic_parent_ts = created_at.saturating_sub(1);

    let mut messages = Vec::new();
    if needs_leading_parent {
        messages.push(serde_json::json!({
            "info": {
                "id": synthetic_parent_id,
                "sessionID": session_id,
                "role": "user",
                "time": {
                    "created": synthetic_parent_ts
                },
                "agent": "imported",
                "model": {
                    "providerID": "imported",
                    "modelID": "recovered"
                }
            },
            "parts": [
                {
                    "id": synthetic_part_id,
                    "sessionID": session_id,
                    "messageID": "msg_recovered_parent_000000",
                    "type": "text",
                    "text": ""
                }
            ]
        }));
    }

    messages.extend(
        source_messages
            .iter()
            .enumerate()
            .map(|(message_index, message)| {
                let message_id = format!("msg_recovered_{message_index:06}");
                let part_id = format!("prt_recovered_{message_index:06}");
                let message_ts = message
                    .ts
                    .unwrap_or_else(|| created_at.saturating_add((message_index as i64) * 1000));
                let parent_message_id = if message_index > 0 {
                    Some(format!("msg_recovered_{:06}", message_index - 1))
                } else if needs_leading_parent {
                    Some("msg_recovered_parent_000000".to_string())
                } else {
                    None
                };
                let info_value = if message.role == "assistant" {
                    serde_json::json!({
                        "id": message_id,
                        "sessionID": session_id,
                        "role": message.role,
                        "time": {
                            "created": message_ts,
                            "completed": message_ts
                        },
                        "parentID": parent_message_id.unwrap_or_default(),
                        "modelID": "recovered",
                        "providerID": "imported",
                        "mode": "imported",
                        "agent": "imported",
                        "path": {
                            "cwd": project_dir,
                            "root": project_dir
                        },
                        "cost": 0,
                        "tokens": {
                            "total": 0,
                            "input": 0,
                            "output": 0,
                            "reasoning": 0,
                            "cache": {
                                "read": 0,
                                "write": 0
                            }
                        },
                        "variant": "recovered",
                        "finish": "recovered"
                    })
                } else {
                    serde_json::json!({
                        "id": message_id,
                        "sessionID": session_id,
                        "role": message.role,
                        "time": {
                            "created": message_ts
                        },
                        "agent": "imported",
                        "model": {
                            "providerID": "imported",
                            "modelID": "recovered"
                        }
                    })
                };

                serde_json::json!({
                    "info": info_value,
                    "parts": [
                        {
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": format!("msg_recovered_{message_index:06}"),
                            "type": "text",
                            "text": message.content
                        }
                    ]
                })
            }),
    );

    serde_json::json!({
        "info": {
            "id": session_id,
            "slug": "recovered-import",
            "projectID": "global",
            "directory": project_dir,
            "title": title,
            "version": "0.0.0",
            "time": {
                "created": created_at,
                "updated": updated_at
            }
        },
        "messages": messages
    })
}

fn ensure_imported_session_visible(
    data_root: &Path,
    session_id: &str,
    command_context: &str,
) -> Result<(), String> {
    const MAX_ATTEMPTS: usize = 5;
    const RETRY_DELAY: Duration = Duration::from_millis(120);

    let sqlite_db_path = data_root.join("opencode.db");
    for attempt_index in 0..MAX_ATTEMPTS {
        if scan_sessions(data_root, &sqlite_db_path)
            .into_iter()
            .any(|session| session.session_id == session_id)
        {
            return Ok(());
        }

        if attempt_index + 1 < MAX_ATTEMPTS {
            std::thread::sleep(RETRY_DELAY);
        }
    }

    Err(format!(
        "`opencode import` reported success but session `{session_id}` was not found after import ({command_context}). This usually means the OpenCode CLI did not persist the imported session on this platform."
    ))
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

fn extract_session_id_from_source(source_path: &str) -> Result<String, String> {
    if source_path.starts_with("sqlite:") {
        let (_, session_id) = parse_sqlite_source(source_path)
            .ok_or_else(|| format!("Invalid SQLite source reference: {source_path}"))?;
        return Ok(session_id);
    }

    Path::new(source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|value| value.to_string())
        .ok_or_else(|| format!("Invalid OpenCode message directory: {source_path}"))
}

pub fn same_session_source(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }

    match (
        extract_session_id_from_source(left),
        extract_session_id_from_source(right),
    ) {
        (Ok(left_session_id), Ok(right_session_id)) => left_session_id == right_session_id,
        _ => false,
    }
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
            resume_command: Some(format!("opencode -s {session_id}")),
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

fn delete_session_from_sqlite(database_path: &Path, session_id: &str) -> Result<(), String> {
    if !database_path.exists() {
        return Ok(());
    }

    let mut connection = Connection::open(database_path)
        .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;
    let transaction = connection.transaction().map_err(|error| {
        format!("Failed to start OpenCode session deletion transaction: {error}")
    })?;

    transaction
        .execute("DELETE FROM part WHERE session_id = ?1", [session_id])
        .map_err(|error| format!("Failed to delete OpenCode session parts: {error}"))?;
    transaction
        .execute("DELETE FROM message WHERE session_id = ?1", [session_id])
        .map_err(|error| format!("Failed to delete OpenCode session messages: {error}"))?;
    transaction
        .execute(
            "DELETE FROM session_share WHERE session_id = ?1",
            [session_id],
        )
        .map_err(|error| format!("Failed to delete OpenCode session shares: {error}"))?;
    transaction
        .execute("DELETE FROM session WHERE id = ?1", [session_id])
        .map_err(|error| format!("Failed to delete OpenCode session record: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("Failed to commit OpenCode session deletion: {error}"))?;

    Ok(())
}

fn update_session_title_in_sqlite(
    database_path: &Path,
    session_id: &str,
    next_title: &str,
) -> Result<(), String> {
    if !database_path.exists() {
        return Ok(());
    }

    let connection = Connection::open(database_path)
        .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;
    connection
        .execute(
            "UPDATE session SET title = ?1 WHERE id = ?2",
            [next_title, session_id],
        )
        .map_err(|error| format!("Failed to update OpenCode session title: {error}"))?;

    Ok(())
}

fn update_session_title_in_json(
    data_root: &Path,
    session_id: &str,
    next_title: &str,
) -> Result<(), String> {
    let storage_root = data_root.join("storage");
    let session_file = find_session_json_paths(&storage_root, session_id)
        .into_iter()
        .find(|path| {
            path.components()
                .any(|component| component.as_os_str() == OsStr::new("session"))
        });
    let Some(session_file) = session_file else {
        return Ok(());
    };

    let data = std::fs::read_to_string(&session_file).map_err(|error| {
        format!(
            "Failed to read OpenCode session file {}: {error}",
            session_file.display()
        )
    })?;
    let mut value: Value = serde_json::from_str(&data).map_err(|error| {
        format!(
            "Failed to parse OpenCode session file {}: {error}",
            session_file.display()
        )
    })?;
    let map = value
        .as_object_mut()
        .ok_or_else(|| format!("Invalid OpenCode session JSON: {}", session_file.display()))?;
    map.insert("title".to_string(), Value::String(next_title.to_string()));

    let serialized = serde_json::to_string_pretty(&value).map_err(|error| {
        format!(
            "Failed to serialize OpenCode session file {}: {error}",
            session_file.display()
        )
    })?;
    std::fs::write(&session_file, serialized).map_err(|error| {
        format!(
            "Failed to write OpenCode session file {}: {error}",
            session_file.display()
        )
    })?;

    Ok(())
}

fn delete_session_json_artifacts(data_root: &Path, session_id: &str) -> Result<(), String> {
    let storage_root = data_root.join("storage");
    let message_dir = storage_root.join("message").join(session_id);
    let session_files = find_session_json_paths(&storage_root, session_id);

    let mut message_ids = Vec::new();
    if message_dir.is_dir() {
        let mut message_files = Vec::new();
        collect_json_files(&message_dir, &mut message_files);

        for message_path in &message_files {
            let data = match std::fs::read_to_string(message_path) {
                Ok(data) => data,
                Err(_) => continue,
            };
            let value: Value = match serde_json::from_str(&data) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(message_id) = value.get("id").and_then(Value::as_str) {
                message_ids.push(message_id.to_string());
            }
        }
    }

    for session_file in session_files {
        if session_file.exists() {
            std::fs::remove_file(&session_file).map_err(|error| {
                format!(
                    "Failed to delete OpenCode session file {}: {error}",
                    session_file.display()
                )
            })?;
        }
    }

    if message_dir.exists() {
        std::fs::remove_dir_all(&message_dir).map_err(|error| {
            format!(
                "Failed to delete OpenCode message directory {}: {error}",
                message_dir.display()
            )
        })?;
    }

    for message_id in message_ids {
        let part_dir = storage_root.join("part").join(&message_id);
        if part_dir.exists() {
            std::fs::remove_dir_all(&part_dir).map_err(|error| {
                format!(
                    "Failed to delete OpenCode part directory {}: {error}",
                    part_dir.display()
                )
            })?;
        }
    }

    Ok(())
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

fn scan_messages_for_query_json(path: &Path, query_lower: &str) -> Result<bool, String> {
    if !path.is_dir() {
        return Err(format!("Message directory not found: {}", path.display()));
    }

    let storage_root = path
        .parent()
        .and_then(|parent| parent.parent())
        .ok_or_else(|| "Cannot determine storage root from message path".to_string())?;

    let mut message_files = Vec::new();
    collect_json_files(path, &mut message_files);

    for message_path in &message_files {
        let data = match std::fs::read_to_string(message_path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let Some(message_id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };

        let part_dir = storage_root.join("part").join(message_id);
        let content = collect_parts_text(&part_dir);
        if text_contains_query(&content, query_lower) {
            return Ok(true);
        }
    }

    Ok(false)
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

fn scan_messages_for_query_sqlite(source: &str, query_lower: &str) -> Result<bool, String> {
    let (database_path, session_id) = parse_sqlite_source(source)
        .ok_or_else(|| format!("Invalid SQLite source reference: {source}"))?;

    let connection = Connection::open_with_flags(
        &database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;

    let mut message_statement = connection
        .prepare("SELECT id FROM message WHERE session_id = ?1 ORDER BY time_created ASC")
        .map_err(|error| format!("Failed to prepare message query: {error}"))?;
    let message_rows = message_statement
        .query_map([session_id.as_str()], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Failed to query messages: {error}"))?;

    let mut part_statement = connection
        .prepare("SELECT data FROM part WHERE session_id = ?1 AND message_id = ?2 ORDER BY time_created ASC")
        .map_err(|error| format!("Failed to prepare part query: {error}"))?;

    for message_id in message_rows.flatten() {
        let part_rows = part_statement
            .query_map([session_id.as_str(), message_id.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("Failed to query parts: {error}"))?;

        let mut texts = Vec::new();
        for part_data in part_rows.flatten() {
            let part_value: Value = match serde_json::from_str(&part_data) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(text) = extract_part_text(&part_value) {
                texts.push(text);
            }
        }

        if text_contains_query(&texts.join("\n"), query_lower) {
            return Ok(true);
        }
    }

    Ok(false)
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
        resume_command: Some(format!("opencode -s {session_id}")),
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

fn find_session_json_paths(storage_root: &Path, session_id: &str) -> Vec<PathBuf> {
    if !storage_root.exists() {
        return Vec::new();
    }

    let mut session_files = Vec::new();
    collect_json_files(storage_root, &mut session_files);
    session_files
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == format!("{session_id}.json"))
                .unwrap_or(false)
        })
        .collect()
}

fn configure_opencode_command_env(
    command: &mut Command,
    config_path: Option<&Path>,
    data_root: Option<&Path>,
    state_root: Option<&Path>,
) {
    if let Some(config_path) = config_path {
        command.env("OPENCODE_CONFIG", config_path);

        let config_dir = config_path.parent();
        let config_root = config_dir.and_then(Path::parent);
        if let Some(config_root) = config_root {
            if config_dir.and_then(Path::file_name).and_then(OsStr::to_str) == Some("opencode") {
                command.env("XDG_CONFIG_HOME", config_root);
            }
        }
    }

    if let Some(data_root) = data_root {
        let data_dir = data_root.parent();
        if let Some(data_dir) = data_dir {
            if data_root.file_name().and_then(OsStr::to_str) == Some("opencode") {
                command.env("XDG_DATA_HOME", data_dir);
            }
        }
    }

    if let Some(state_root) = state_root {
        let state_dir = state_root.parent();
        if let Some(state_dir) = state_dir {
            if state_root.file_name().and_then(OsStr::to_str) == Some("opencode") {
                command.env("XDG_STATE_HOME", state_dir);
            }
        }
    }
}

fn path_to_linux_string(path: &Path) -> Option<String> {
    let path_string = path.to_string_lossy().to_string();
    crate::coding::runtime_location::parse_wsl_unc_path(&path_string).map(|wsl| wsl.linux_path)
}

fn convert_windows_path_to_wsl(path: &str) -> Result<String, String> {
    let normalized_path = path.replace('\\', "/");
    if normalized_path.starts_with('/') {
        return Ok(normalized_path);
    }

    let bytes = normalized_path.as_bytes();
    if normalized_path.len() >= 2 && bytes[1] == b':' {
        let drive_letter = normalized_path
            .chars()
            .next()
            .ok_or_else(|| format!("Invalid Windows path: {path}"))?
            .to_ascii_lowercase();
        return Ok(format!("/mnt/{}{}", drive_letter, &normalized_path[2..]));
    }

    Err(format!(
        "Failed to convert Windows path to WSL path: {path}"
    ))
}

fn convert_to_wsl_command_path(path: &Path) -> Result<String, String> {
    if let Some(linux_path) = path_to_linux_string(path) {
        return Ok(linux_path);
    }

    convert_windows_path_to_wsl(&path.to_string_lossy())
}

fn add_opencode_runtime_env_args(
    command: &mut Command,
    runtime_location: &RuntimeLocationInfo,
    config_path: Option<&Path>,
    data_root: Option<&Path>,
    state_root: Option<&Path>,
) -> Result<(), String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            if let Some(config_path) = config_path {
                command.env("OPENCODE_CONFIG", config_path);

                let config_dir = config_path.parent();
                let config_root = config_dir.and_then(Path::parent);
                if let Some(config_root) = config_root {
                    if config_dir.and_then(Path::file_name).and_then(OsStr::to_str)
                        == Some("opencode")
                    {
                        command.env("XDG_CONFIG_HOME", config_root);
                    }
                }
            }

            if let Some(data_root) = data_root {
                let data_dir = data_root.parent();
                if let Some(data_dir) = data_dir {
                    if data_root.file_name().and_then(OsStr::to_str) == Some("opencode") {
                        command.env("XDG_DATA_HOME", data_dir);
                    }
                }
            }

            if let Some(state_root) = state_root {
                let state_dir = state_root.parent();
                if let Some(state_dir) = state_dir {
                    if state_root.file_name().and_then(OsStr::to_str) == Some("opencode") {
                        command.env("XDG_STATE_HOME", state_dir);
                    }
                }
            }
        }
        RuntimeLocationMode::WslDirect => {
            if let Some(config_path) = config_path {
                let linux_config_path = path_to_linux_string(config_path).ok_or_else(|| {
                    format!(
                        "Failed to convert OpenCode config path to WSL path: {}",
                        config_path.display()
                    )
                })?;
                command.arg(format!("OPENCODE_CONFIG={linux_config_path}"));

                let config_dir = Path::new(&linux_config_path)
                    .parent()
                    .map(Path::to_path_buf);
                let config_root = config_dir.as_deref().and_then(Path::parent);
                if let Some(config_root) = config_root {
                    if config_dir
                        .as_deref()
                        .and_then(Path::file_name)
                        .and_then(OsStr::to_str)
                        == Some("opencode")
                    {
                        command.arg(format!("XDG_CONFIG_HOME={}", config_root.to_string_lossy()));
                    }
                }
            }

            if let Some(data_root) = data_root {
                let linux_data_root = path_to_linux_string(data_root).ok_or_else(|| {
                    format!(
                        "Failed to convert OpenCode data root to WSL path: {}",
                        data_root.display()
                    )
                })?;
                let data_root_path = Path::new(&linux_data_root);
                let data_dir = data_root_path.parent();
                if let Some(data_dir) = data_dir {
                    if data_root_path.file_name().and_then(OsStr::to_str) == Some("opencode") {
                        command.arg(format!("XDG_DATA_HOME={}", data_dir.to_string_lossy()));
                    }
                }
            }

            if let Some(state_root) = state_root {
                let linux_state_root = path_to_linux_string(state_root).ok_or_else(|| {
                    format!(
                        "Failed to convert OpenCode state root to WSL path: {}",
                        state_root.display()
                    )
                })?;
                let state_root_path = Path::new(&linux_state_root);
                let state_dir = state_root_path.parent();
                if let Some(state_dir) = state_dir {
                    if state_root_path.file_name().and_then(OsStr::to_str) == Some("opencode") {
                        command.arg(format!("XDG_STATE_HOME={}", state_dir.to_string_lossy()));
                    }
                }
            }
        }
    }

    Ok(())
}

fn build_opencode_command(
    runtime_location: &RuntimeLocationInfo,
    config_path: Option<&Path>,
    data_root: Option<&Path>,
    state_root: Option<&Path>,
    working_directory: Option<&Path>,
) -> Result<Command, String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            let mut command = match resolve_local_opencode_program() {
                Some(opencode_program) => build_local_opencode_command(&opencode_program),
                None => build_fallback_local_opencode_command(),
            };
            configure_opencode_command_env(&mut command, config_path, data_root, state_root);
            if let Some(working_directory) = working_directory {
                command.current_dir(working_directory);
            }
            Ok(command)
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = runtime_location
                .wsl
                .as_ref()
                .ok_or_else(|| "Missing WSL runtime metadata for OpenCode command".to_string())?;
            let mut command = Command::new("wsl");
            command.args(["-d", &wsl.distro]);
            if let Some(working_directory) = working_directory {
                let linux_working_directory = convert_to_wsl_command_path(working_directory)?;
                command.args(["--cd", &linux_working_directory]);
            }
            command.args(["--exec", "env"]);
            add_opencode_runtime_env_args(
                &mut command,
                runtime_location,
                config_path,
                data_root,
                state_root,
            )?;
            command.arg("opencode");
            Ok(command)
        }
    }
}

fn resolve_runtime_project_dir(
    runtime_location: &RuntimeLocationInfo,
    project_dir: &str,
) -> Result<Option<PathBuf>, String> {
    let trimmed_project_dir = project_dir.trim();
    if trimmed_project_dir.is_empty() {
        return Ok(None);
    }

    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            let project_path = Path::new(trimmed_project_dir);
            if !project_path.exists() || !project_path.is_dir() {
                return Ok(None);
            }

            Ok(Some(project_path.to_path_buf()))
        }
        RuntimeLocationMode::WslDirect => {
            if trimmed_project_dir.starts_with('/') {
                return Ok(Some(PathBuf::from(trimmed_project_dir)));
            }

            let project_path = Path::new(trimmed_project_dir);
            if !project_path.exists() || !project_path.is_dir() {
                return Ok(None);
            }

            Ok(Some(project_path.to_path_buf()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        delete_session_json_artifacts, ensure_imported_session_visible,
        extract_session_id_from_snapshot, find_session_json_paths, resolve_runtime_project_dir,
    };

    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::coding::runtime_location::RuntimeLocationInfo;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ai-toolbox-opencode-{label}-{}",
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

    #[cfg(target_os = "windows")]
    #[test]
    fn select_windows_opencode_command_path_prefers_cmd_over_extensionless() {
        let selected = super::select_windows_opencode_command_path(&[
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode"),
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode.cmd"),
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode.ps1"),
        ])
        .expect("expected selected path");

        assert_eq!(
            selected,
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\fnm\aliases\default\opencode.cmd")
        );
    }

    #[test]
    fn missing_local_opencode_spawn_message_preserves_missing_cli_hint() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let message = super::build_missing_local_opencode_spawn_message(
            &error,
            "opencode import",
            "runtime=local",
        );

        assert!(message.contains("OpenCode 会话导入/导出需要先安装 OpenCode CLI"));
        assert!(message.contains("opencode import"));
    }

    #[test]
    fn delete_session_json_artifacts_removes_nested_session_json() {
        let test_dir = TestDir::new("delete-session-json");
        let data_root = test_dir.path().join("data");
        let storage_root = data_root.join("storage");
        let session_id = "ses_delete_nested";
        let message_id = "msg_delete_nested";

        let session_file = storage_root
            .join("session")
            .join("global")
            .join(format!("{session_id}.json"));
        let message_file = storage_root
            .join("message")
            .join(session_id)
            .join(format!("{message_id}.json"));
        let part_file = storage_root
            .join("part")
            .join(message_id)
            .join("prt_delete_nested.json");

        if let Some(parent) = session_file.parent() {
            fs::create_dir_all(parent).expect("failed to create session dir");
        }
        if let Some(parent) = message_file.parent() {
            fs::create_dir_all(parent).expect("failed to create message dir");
        }
        if let Some(parent) = part_file.parent() {
            fs::create_dir_all(parent).expect("failed to create part dir");
        }

        fs::write(
            &session_file,
            format!(r#"{{"id":"{session_id}","directory":"/tmp/project"}}"#),
        )
        .expect("failed to write session file");
        fs::write(
            &message_file,
            format!(r#"{{"id":"{message_id}","role":"user","time":{{"created":1}}}}"#),
        )
        .expect("failed to write message file");
        fs::write(&part_file, r#"{"type":"text","text":"hello"}"#)
            .expect("failed to write part file");

        delete_session_json_artifacts(&data_root, session_id)
            .expect("delete_session_json_artifacts should succeed");

        assert!(
            !session_file.exists(),
            "nested session json should be removed"
        );
        assert!(
            !message_file.exists(),
            "message json should be removed with session artifacts"
        );
        assert!(
            !part_file.exists(),
            "part json should be removed with session artifacts"
        );
    }

    #[test]
    fn find_session_json_paths_includes_sidecar_storage_files() {
        let test_dir = TestDir::new("find-session-json-paths");
        let storage_root = test_dir.path().join("storage");
        let session_id = "ses_find_session_paths";

        let session_file = storage_root
            .join("session")
            .join("global")
            .join(format!("{session_id}.json"));
        let sidecar_file = storage_root
            .join("directory-readme")
            .join(format!("{session_id}.json"));

        if let Some(parent) = session_file.parent() {
            fs::create_dir_all(parent).expect("failed to create session dir");
        }
        if let Some(parent) = sidecar_file.parent() {
            fs::create_dir_all(parent).expect("failed to create sidecar dir");
        }

        fs::write(&session_file, "{}").expect("failed to write session file");
        fs::write(&sidecar_file, "{}").expect("failed to write sidecar file");

        let matched_paths = find_session_json_paths(&storage_root, session_id);

        assert_eq!(matched_paths.len(), 2);
        assert!(matched_paths.contains(&session_file));
        assert!(matched_paths.contains(&sidecar_file));
    }

    #[test]
    fn resolve_runtime_project_dir_accepts_linux_path_in_wsl_direct_mode() {
        let runtime_location = RuntimeLocationInfo {
            mode: crate::coding::runtime_location::RuntimeLocationMode::WslDirect,
            source: "test".to_string(),
            host_path: PathBuf::from(
                r"\\wsl.localhost\Ubuntu\home\tester\.config\opencode\opencode.jsonc",
            ),
            wsl: Some(crate::coding::runtime_location::WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/tester/.config/opencode/opencode.jsonc".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };

        let resolved = resolve_runtime_project_dir(&runtime_location, "/home/tester/project")
            .expect("linux path should be accepted in wsl direct mode");

        assert_eq!(resolved, Some(PathBuf::from("/home/tester/project")));
    }

    #[test]
    fn extract_session_id_from_snapshot_reads_raw_official_export() {
        let snapshot = serde_json::json!({
            "officialExportRaw": r#"{
                "info": {
                    "id": "ses_raw_import_target"
                }
            }"#
        });

        let session_id =
            extract_session_id_from_snapshot(&snapshot).expect("should read session id");

        assert_eq!(session_id, "ses_raw_import_target");
    }

    #[test]
    fn ensure_imported_session_visible_accepts_json_storage_session() {
        let test_dir = TestDir::new("ensure-import-visible-success");
        let data_root = test_dir.path().join("data");
        let session_id = "ses_import_visible_success";
        let session_file = data_root
            .join("storage")
            .join("session")
            .join("global")
            .join(format!("{session_id}.json"));

        if let Some(parent_dir) = session_file.parent() {
            fs::create_dir_all(parent_dir).expect("failed to create session parent directory");
        }
        fs::write(
            &session_file,
            format!(
                r#"{{
                    "id": "{session_id}",
                    "title": "Imported Session",
                    "directory": "/tmp/imported-project",
                    "time": {{
                        "created": 1710000000000,
                        "updated": 1710000000001
                    }}
                }}"#
            ),
        )
        .expect("failed to write session file");

        ensure_imported_session_visible(&data_root, session_id, "runtime=local")
            .expect("imported json session should be visible");
    }

    #[test]
    fn ensure_imported_session_visible_errors_when_session_is_missing() {
        let test_dir = TestDir::new("ensure-import-visible-missing");
        let data_root = test_dir.path().join("data");
        fs::create_dir_all(&data_root).expect("failed to create data root");

        let error =
            ensure_imported_session_visible(&data_root, "ses_import_missing", "runtime=local")
                .expect_err("missing session should return error");

        assert!(error.contains("ses_import_missing"));
        assert!(error.contains("reported success"));
    }
}
