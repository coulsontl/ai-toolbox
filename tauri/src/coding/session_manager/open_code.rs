use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value};

use super::message_blocks::{
    message_from_blocks, text_block, thinking_block, tool_call_block, tool_result_block,
};
use super::utils::{
    build_resume_command, collect_recent_files_by_modified, parse_timestamp_to_ms, path_basename,
    text_contains_query, truncate_summary,
};
use super::{
    assign_missing_message_ids, SessionMessage, SessionMessageBlock, SessionMessageUsage,
    SessionMeta,
};
use crate::coding::cli_resolver::{
    build_local_std_command, local_cli_missing_hint, resolve_local_opencode_program,
};
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
        "OpenCode 会话导入/导出需要先安装 OpenCode CLI，并确保当前系统环境可以找到 `opencode` 命令。详情: {details}。{}",
        local_cli_missing_hint("opencode")
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

pub fn scan_sessions(data_root: &Path, sqlite_db_path: &Path, v2: bool) -> Vec<SessionMeta> {
    if v2 {
        return scan_sessions_sqlite_with_limit(sqlite_db_path, None, true);
    }

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

pub fn scan_recent_sessions(
    data_root: &Path,
    sqlite_db_path: &Path,
    limit: usize,
    v2: bool,
) -> Vec<SessionMeta> {
    if limit == 0 {
        return Vec::new();
    }
    if v2 {
        return scan_sessions_sqlite_with_limit(sqlite_db_path, Some(limit), true);
    }

    let json_sessions = scan_recent_sessions_json(data_root, limit);
    let sqlite_sessions = scan_sessions_sqlite_limited(sqlite_db_path, limit);

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

    merged.sort_by(|left, right| {
        let left_ts = left.last_active_at.or(left.created_at).unwrap_or(0);
        let right_ts = right.last_active_at.or(right.created_at).unwrap_or(0);
        right_ts.cmp(&left_ts)
    });
    merged.truncate(limit);
    merged
}

pub fn load_messages(source_path: &str, v2: bool) -> Result<Vec<SessionMessage>, String> {
    if v2 {
        return load_messages_sqlite_v2(source_path);
    }
    if source_path.starts_with("sqlite:") {
        return load_messages_sqlite(source_path);
    }

    load_messages_json(Path::new(source_path))
}

pub fn scan_messages_for_query(
    source_path: &str,
    query_lower: &str,
    v2: bool,
) -> Result<bool, String> {
    if v2 {
        return scan_messages_for_query_sqlite_v2(source_path, query_lower);
    }
    if source_path.starts_with("sqlite:") {
        return scan_messages_for_query_sqlite(source_path, query_lower);
    }

    scan_messages_for_query_json(Path::new(source_path), query_lower)
}

pub fn delete_session(source_path: &str, v2: bool) -> Result<(), String> {
    if v2 {
        let (database_path, session_id) = parse_sqlite_source(source_path)
            .ok_or_else(|| format!("Invalid SQLite source reference: {source_path}"))?;
        delete_session_from_sqlite_v2(&database_path, &session_id)?;
        return Ok(());
    }

    if source_path.starts_with("sqlite:") {
        let (database_path, session_id) = parse_sqlite_source(source_path)
            .ok_or_else(|| format!("Invalid SQLite source reference: {source_path}"))?;
        delete_session_from_sqlite(&database_path, &session_id)?;
        delete_session_json_artifacts(
            &database_path.parent().unwrap_or(Path::new("")),
            &session_id,
        )?;
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

        delete_session_from_sqlite(&data_root.join("opencode.db"), &session_id)?;
        delete_session_json_artifacts(data_root, &session_id)?;
    }

    Ok(())
}

pub fn rename_session(source_path: &str, next_title: &str, v2: bool) -> Result<(), String> {
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

    if v2 {
        update_session_title_in_sqlite_v2(&database_path, &session_id, normalized_title)?;
    } else {
        update_session_title_in_sqlite(&database_path, &session_id, normalized_title)?;
        update_session_title_in_json(&data_root, &session_id, normalized_title)?;
    }
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
            ensure_imported_session_visible(
                data_root,
                &session_id,
                &command_context,
                config_path.is_some_and(crate::coding::open_code::v2_migration::is_active),
            )?;
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
    v2: bool,
) -> Result<(), String> {
    const MAX_ATTEMPTS: usize = 5;
    const RETRY_DELAY: Duration = Duration::from_millis(120);

    let sqlite_db_path = data_root.join("opencode.db");
    for attempt_index in 0..MAX_ATTEMPTS {
        if scan_sessions(data_root, &sqlite_db_path, v2)
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

fn scan_recent_sessions_json(data_root: &Path, limit: usize) -> Vec<SessionMeta> {
    let storage_root = data_root.join("storage");
    let session_root = storage_root.join("session");
    if limit == 0 || !session_root.exists() {
        return Vec::new();
    }

    let json_files = collect_recent_files_by_modified(
        &session_root,
        limit.saturating_mul(3).max(limit),
        |path| path.extension().and_then(|ext| ext.to_str()) == Some("json"),
    );

    let mut sessions = Vec::new();
    for path in json_files {
        if let Some(session) = parse_session(&storage_root, &path) {
            sessions.push(session);
            if sessions.len() >= limit {
                break;
            }
        }
    }

    sessions
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

pub fn session_source_key(source_path: &str) -> Result<String, String> {
    extract_session_id_from_source(source_path).map(|session_id| session_id.to_ascii_lowercase())
}

fn scan_sessions_sqlite(sqlite_db_path: &Path) -> Vec<SessionMeta> {
    scan_sessions_sqlite_with_limit(sqlite_db_path, None, false)
}

fn scan_sessions_sqlite_limited(sqlite_db_path: &Path, limit: usize) -> Vec<SessionMeta> {
    scan_sessions_sqlite_with_limit(sqlite_db_path, Some(limit), false)
}

fn scan_sessions_sqlite_with_limit(
    sqlite_db_path: &Path,
    limit: Option<usize>,
    v2: bool,
) -> Vec<SessionMeta> {
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

    let sql = match (v2, limit.is_some()) {
        (false, true) => {
            "SELECT id, title, directory, time_created, time_updated FROM session ORDER BY time_updated DESC LIMIT ?1"
        }
        (false, false) => {
            "SELECT id, title, directory, time_created, time_updated FROM session ORDER BY time_updated DESC"
        }
        (true, true) => {
            "SELECT id, COALESCE(title, ''), directory, time_created, time_updated FROM session_v2 WHERE parent_id IS NULL ORDER BY time_updated DESC LIMIT ?1"
        }
        (true, false) => {
            "SELECT id, COALESCE(title, ''), directory, time_created, time_updated FROM session_v2 WHERE parent_id IS NULL ORDER BY time_updated DESC"
        }
    };

    let mut statement = match connection.prepare(sql) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };

    let database_display = sqlite_db_path.display().to_string();
    let row_mapper = |row: &rusqlite::Row<'_>| {
        let session_id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let directory: String = row.get(2)?;
        let created_at: i64 = row.get(3)?;
        let last_active_at: i64 = row.get(4)?;
        Ok((session_id, title, directory, created_at, last_active_at))
    };

    let mut sessions = Vec::new();
    let rows = if let Some(limit) = limit {
        statement.query_map([limit as i64], row_mapper)
    } else {
        statement.query_map([], row_mapper)
    };

    let rows = match rows {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    for row in rows.flatten() {
        let (session_id, title, directory, created_at, last_active_at) = row;
        let display_title = if title.is_empty() {
            path_basename(&directory)
        } else {
            Some(title)
        };

        let project_dir = if directory.is_empty() {
            None
        } else {
            Some(directory)
        };

        sessions.push(SessionMeta {
            provider_id: PROVIDER_ID.to_string(),
            session_id: session_id.clone(),
            title: display_title.clone(),
            summary: display_title,
            project_dir: project_dir.clone(),
            created_at: Some(created_at),
            last_active_at: Some(last_active_at),
            source_path: format!("sqlite:{}:{}", database_display, session_id),
            resume_command: Some(build_resume_command(
                project_dir.as_deref(),
                &format!("opencode -s {session_id}"),
            )),
            runtime_source: None,
            runtime_distro: None,
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

fn delete_session_from_sqlite(database_path: &Path, session_id: &str) -> Result<bool, String> {
    if !database_path.exists() {
        return Ok(false);
    }

    let mut connection = Connection::open(database_path)
        .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;
    let transaction = connection.transaction().map_err(|error| {
        format!("Failed to start OpenCode session deletion transaction: {error}")
    })?;

    let deleted_part_rows = transaction
        .execute("DELETE FROM part WHERE session_id = ?1", [session_id])
        .map_err(|error| format!("Failed to delete OpenCode session parts: {error}"))?;
    let deleted_message_rows = transaction
        .execute("DELETE FROM message WHERE session_id = ?1", [session_id])
        .map_err(|error| format!("Failed to delete OpenCode session messages: {error}"))?;
    let deleted_share_rows = transaction
        .execute(
            "DELETE FROM session_share WHERE session_id = ?1",
            [session_id],
        )
        .map_err(|error| format!("Failed to delete OpenCode session shares: {error}"))?;
    let deleted_session_rows = transaction
        .execute("DELETE FROM session WHERE id = ?1", [session_id])
        .map_err(|error| format!("Failed to delete OpenCode session record: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("Failed to commit OpenCode session deletion: {error}"))?;

    Ok(deleted_part_rows > 0
        || deleted_message_rows > 0
        || deleted_share_rows > 0
        || deleted_session_rows > 0)
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

fn update_session_title_in_sqlite_v2(
    database_path: &Path,
    session_id: &str,
    next_title: &str,
) -> Result<(), String> {
    if !database_path.exists() {
        return Ok(());
    }

    let connection = Connection::open(database_path)
        .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;
    if !sqlite_table_exists(&connection, "session_v2")? {
        return Ok(());
    }
    connection
        .execute(
            "UPDATE session_v2 SET title = ?1 WHERE id = ?2",
            [next_title, session_id],
        )
        .map_err(|error| format!("Failed to update OpenCode V2 session title: {error}"))?;

    Ok(())
}

fn delete_session_from_sqlite_v2(database_path: &Path, session_id: &str) -> Result<bool, String> {
    if !database_path.exists() {
        return Ok(false);
    }

    let mut connection = Connection::open(database_path)
        .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;
    let transaction = connection.transaction().map_err(|error| {
        format!("Failed to start OpenCode session deletion transaction: {error}")
    })?;

    let mut deleted_rows = 0;
    // `session_share` belongs to the V1 `session` table. Do not touch it here.
    for table in [
        "session_message",
        "session_pending",
        "session_inbox",
        "instruction_entry",
        "instruction_state",
    ] {
        if !sqlite_table_exists(&transaction, table)? {
            continue;
        }
        deleted_rows += transaction
            .execute(
                &format!("DELETE FROM {table} WHERE session_id = ?1"),
                [session_id],
            )
            .map_err(|error| format!("Failed to delete OpenCode V2 {table} rows: {error}"))?;
    }
    if sqlite_table_exists(&transaction, "session_v2")? {
        deleted_rows += transaction
            .execute("DELETE FROM session_v2 WHERE id = ?1", [session_id])
            .map_err(|error| format!("Failed to delete OpenCode V2 session record: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("Failed to commit OpenCode V2 session deletion: {error}"))?;
    Ok(deleted_rows > 0)
}

fn sqlite_table_exists(connection: &Connection, table: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(|error| format!("Failed to inspect OpenCode database schema: {error}"))
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

fn delete_session_json_artifacts(data_root: &Path, session_id: &str) -> Result<bool, String> {
    let storage_root = data_root.join("storage");
    let message_dir = storage_root.join("message").join(session_id);
    let session_files = find_session_json_paths(&storage_root, session_id);
    let mut deleted_any = false;

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
            deleted_any = true;
        }
    }

    if message_dir.exists() {
        std::fs::remove_dir_all(&message_dir).map_err(|error| {
            format!(
                "Failed to delete OpenCode message directory {}: {error}",
                message_dir.display()
            )
        })?;
        deleted_any = true;
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
            deleted_any = true;
        }
    }

    Ok(deleted_any)
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

    let mut entries: Vec<(i64, SessionMessage)> = Vec::new();
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
        let blocks = collect_part_blocks(&part_dir);
        if blocks.is_empty() {
            continue;
        }

        let mut message = message_from_blocks(
            role,
            if created_at > 0 {
                Some(created_at)
            } else {
                None
            },
            blocks,
        );
        message.id = Some(message_id);
        message.parent_id = value
            .get("parentID")
            .or_else(|| value.get("parent_id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        message.model = value
            .get("modelID")
            .or_else(|| value.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string);
        message.usage = opencode_usage_from_tokens(value.get("tokens"));
        message.cost_usd = value.get("cost").and_then(Value::as_f64);

        entries.push((created_at, message));
    }

    entries.sort_by_key(|(timestamp, _)| *timestamp);
    let mut messages: Vec<SessionMessage> =
        entries.into_iter().map(|(_, message)| message).collect();
    assign_missing_message_ids(&mut messages, PROVIDER_ID);
    Ok(messages)
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

        let mut part_values = Vec::new();
        if let Some(parts) = parts_map.get(&message_id) {
            for part_data in parts {
                let part_value: Value = match serde_json::from_str(part_data) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                part_values.push(part_value);
            }
        }

        let blocks = opencode_blocks_from_parts(&part_values);
        if blocks.is_empty() {
            continue;
        }

        let mut message = message_from_blocks(role, Some(timestamp), blocks);
        message.id = Some(message_id);
        message.parent_id = message_value
            .get("parentID")
            .or_else(|| message_value.get("parent_id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        message.model = message_value
            .get("modelID")
            .or_else(|| message_value.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string);
        message.usage = opencode_usage_from_tokens(message_value.get("tokens"));
        message.cost_usd = message_value.get("cost").and_then(Value::as_f64);

        messages.push(message);
    }

    assign_missing_message_ids(&mut messages, PROVIDER_ID);
    Ok(messages)
}

fn load_messages_sqlite_v2(source: &str) -> Result<Vec<SessionMessage>, String> {
    let (database_path, session_id) = parse_sqlite_source(source)
        .ok_or_else(|| format!("Invalid SQLite source reference: {source}"))?;
    let connection = Connection::open_with_flags(
        &database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open OpenCode database: {error}"))?;
    if !sqlite_table_exists(&connection, "session_message")? {
        return Ok(Vec::new());
    }

    let mut statement = connection
        .prepare(
            "SELECT id, type, time_created, data FROM session_message WHERE session_id = ?1 ORDER BY seq ASC",
        )
        .map_err(|error| format!("Failed to prepare V2 message query: {error}"))?;
    let rows = statement
        .query_map([session_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("Failed to query V2 messages: {error}"))?;

    let mut messages = Vec::new();
    for row in rows {
        let (message_id, message_type, timestamp, data) =
            row.map_err(|error| format!("Failed to read V2 message: {error}"))?;
        let Some(value) = serde_json::from_str::<Value>(&data).ok() else {
            continue;
        };
        let Some(mut message) = message_from_v2_row(&message_type, timestamp, &value) else {
            continue;
        };
        message.id = Some(message_id);
        messages.push(message);
    }

    assign_missing_message_ids(&mut messages, PROVIDER_ID);
    Ok(messages)
}

fn scan_messages_for_query_sqlite_v2(source: &str, query_lower: &str) -> Result<bool, String> {
    let messages = load_messages_sqlite_v2(source)?;
    Ok(messages
        .iter()
        .any(|message| text_contains_query(&v2_message_search_text(message), query_lower)))
}

fn v2_message_search_text(message: &SessionMessage) -> String {
    let mut parts = Vec::new();
    if !message.content.trim().is_empty() {
        parts.push(message.content.clone());
    }
    for block in &message.blocks {
        if let Some(text) = block.text.as_deref().filter(|text| !text.trim().is_empty()) {
            parts.push(text.to_string());
        }
        if let Some(output) = block.output.as_ref().and_then(Value::as_str) {
            if !output.trim().is_empty() {
                parts.push(output.to_string());
            }
        }
        if let Some(input) = block.input.as_ref() {
            if let Ok(text) = serde_json::to_string(input) {
                if text != "null" && text != "{}" {
                    parts.push(text);
                }
            }
        }
    }
    parts.join("\n")
}

fn message_from_v2_row(
    message_type: &str,
    timestamp: i64,
    value: &Value,
) -> Option<SessionMessage> {
    let (role, blocks) = match message_type {
        "user" => ("user", v2_user_blocks(value)),
        "assistant" => (
            "assistant",
            opencode_blocks_from_parts(&v2_assistant_parts(value)),
        ),
        "system" | "synthetic" | "skill" => ("system", v2_text_blocks(value.get("text"), "")),
        "compaction" => ("system", v2_text_blocks(value.get("summary"), "[Summary] ")),
        "shell" => ("system", v2_shell_blocks(value)),
        _ => return None,
    };
    if blocks.is_empty() {
        return None;
    }

    let mut message = message_from_blocks(role, Some(timestamp), blocks);
    message.message_type = Some(message_type.to_string());
    if message_type == "assistant" {
        message.model = value.get("model").and_then(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| model.as_str())
                .map(str::to_string)
        });
        message.usage = opencode_usage_from_tokens(value.get("tokens"));
        message.cost_usd = value.get("cost").and_then(Value::as_f64);
    }
    Some(message)
}

fn v2_user_blocks(value: &Value) -> Vec<SessionMessageBlock> {
    let mut blocks = v2_text_blocks(value.get("text"), "");
    let names = value
        .get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|file| file.get("name").and_then(Value::as_str))
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !names.is_empty() {
        blocks.push(text_block(format!("Files: {}", names.join(", "))));
    }
    blocks
}

fn v2_text_blocks(value: Option<&Value>, prefix: &str) -> Vec<SessionMessageBlock> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| vec![text_block(format!("{prefix}{text}"))])
        .unwrap_or_default()
}

fn v2_shell_blocks(value: &Value) -> Vec<SessionMessageBlock> {
    let mut lines = Vec::new();
    if let Some(command) = value
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.is_empty())
    {
        lines.push(format!("$ {command}"));
    }
    if let Some(output) = value
        .get("output")
        .and_then(Value::as_str)
        .filter(|output| !output.trim().is_empty())
    {
        lines.push(output.to_string());
    }
    if lines.is_empty() {
        Vec::new()
    } else {
        vec![text_block(lines.join("\n"))]
    }
}

fn v2_assistant_parts(value: &Value) -> Vec<Value> {
    value
        .get("content")
        .and_then(Value::as_array)
        .map(|content| content.iter().cloned().map(adapt_v2_part).collect())
        .unwrap_or_default()
}

fn adapt_v2_part(mut part: Value) -> Value {
    if part.get("type").and_then(Value::as_str) != Some("tool") {
        return part;
    }

    if let Some(object) = part.as_object_mut() {
        if !object.contains_key("tool") {
            if let Some(name) = object.get("name").cloned() {
                object.insert("tool".to_string(), name);
            }
        }
        if let Some(state) = object.get_mut("state").and_then(Value::as_object_mut) {
            if !state.contains_key("output") {
                if let Some(text) = v2_tool_content_text(state.get("content")) {
                    state.insert("output".to_string(), Value::String(text));
                }
            }
            if let Some(error) = state.get("error").cloned() {
                if !error.is_string() {
                    if let Some(message) = error
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|message| !message.is_empty())
                    {
                        state.insert("error".to_string(), Value::String(message.to_string()));
                    }
                }
            }
        }
    }

    part
}

fn v2_tool_content_text(content: Option<&Value>) -> Option<String> {
    let parts = content?.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string)
                .or_else(|| {
                    part.get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.is_empty())
                        .map(|name| format!("file: {name}"))
                })
        })
        .collect::<Vec<_>>();
    if text.is_empty() {
        None
    } else {
        Some(text.join("\n"))
    }
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
    let resume_command =
        build_resume_command(directory.as_deref(), &format!("opencode -s {session_id}"));

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title: display_title,
        summary,
        project_dir: directory,
        created_at,
        last_active_at: last_active_at.or(created_at),
        source_path,
        resume_command: Some(resume_command),
        runtime_source: None,
        runtime_distro: None,
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

fn collect_part_blocks(part_dir: &Path) -> Vec<SessionMessageBlock> {
    if !part_dir.is_dir() {
        return Vec::new();
    }

    let mut part_files = Vec::new();
    collect_json_files(part_dir, &mut part_files);
    part_files.sort();

    let mut part_values = Vec::new();
    for part_path in &part_files {
        let data = match std::fs::read_to_string(part_path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        part_values.push(value);
    }

    opencode_blocks_from_parts(&part_values)
}

fn opencode_blocks_from_parts(parts: &[Value]) -> Vec<SessionMessageBlock> {
    let mut blocks = Vec::new();

    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = part
                    .get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                {
                    blocks.push(text_block(text.to_string()));
                }
            }
            Some("reasoning") => {
                if let Some(text) = part
                    .get("text")
                    .or_else(|| part.get("reasoning"))
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                {
                    blocks.push(thinking_block(text.to_string()));
                }
            }
            Some("tool") => {
                let raw_tool_name = part
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let tool_name = normalize_opencode_tool_name(raw_tool_name);
                let tool_id = part
                    .get("callID")
                    .or_else(|| part.get("id"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string);
                let input = part
                    .get("state")
                    .and_then(|state| state.get("input"))
                    .or_else(|| part.get("input"))
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                let input = normalize_opencode_tool_input(&tool_name, input);

                blocks.push(tool_call_block(
                    tool_id.clone(),
                    tool_name.clone(),
                    Some(input),
                ));

                let status = part
                    .get("state")
                    .and_then(|state| state.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if let Some((result, is_error)) =
                    extract_opencode_tool_result_from_state(part, status)
                {
                    blocks.push(tool_result_block(
                        tool_id,
                        Some(tool_name),
                        Some(result),
                        Some(is_error),
                    ));
                }
            }
            Some("step-finish") => {
                let reason = part
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("completed");
                let snapshot = part
                    .get("snapshot")
                    .and_then(Value::as_str)
                    .map(|value| value.chars().take(8).collect::<String>())
                    .unwrap_or_default();
                let text = if snapshot.is_empty() {
                    format!("OpenCode step: {reason}")
                } else {
                    format!("OpenCode step: {reason} ({snapshot})")
                };
                blocks.push(text_block(text));
            }
            Some("compaction") => {
                let text = part
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("[Context compacted]");
                blocks.push(text_block(format!("[Summary] {text}")));
            }
            Some("patch") => {
                if let Some(files) = part.get("files").and_then(Value::as_array) {
                    let file_list = files
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .collect::<Vec<_>>();
                    if !file_list.is_empty() {
                        blocks.push(text_block(format!(
                            "Modified files: {}",
                            file_list.join(", ")
                        )));
                    }
                }
            }
            _ => {}
        }
    }

    blocks
}

fn normalize_opencode_tool_name(name: &str) -> String {
    match name {
        "read" => "Read",
        "bash" => "Bash",
        "glob" => "Glob",
        "grep" => "Grep",
        "write" => "Write",
        "edit" => "Edit",
        "todowrite" => "TodoWrite",
        "webfetch" => "WebFetch",
        "task" | "call_omo_agent" => "Task",
        "websearch_web_search_exa"
        | "websearch_exa_web_search_exa"
        | "web_search"
        | "brave-search_brave_web_search" => "WebSearch",
        _ if name.starts_with("grep_") => "Grep",
        _ => name,
    }
    .to_string()
}

fn move_input_key(input: &mut Map<String, Value>, from: &str, to: &str) {
    if input.contains_key(to) {
        return;
    }
    if let Some(value) = input.remove(from) {
        input.insert(to.to_string(), value);
    }
}

fn normalize_opencode_tool_input(tool_name: &str, input: Value) -> Value {
    let Value::Object(mut input_object) = input else {
        return input;
    };

    move_input_key(&mut input_object, "filePath", "file_path");
    move_input_key(&mut input_object, "oldString", "old_string");
    move_input_key(&mut input_object, "newString", "new_string");
    move_input_key(&mut input_object, "replaceAll", "replace_all");
    move_input_key(&mut input_object, "runInBackground", "run_in_background");
    move_input_key(&mut input_object, "allowedDomains", "allowed_domains");
    move_input_key(&mut input_object, "blockedDomains", "blocked_domains");

    if tool_name == "Bash" {
        if let Some(Value::Array(command_parts)) = input_object.get("command").cloned() {
            let command = command_parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            input_object.insert("command".to_string(), Value::String(command));
        }
    }

    Value::Object(input_object)
}

fn extract_opencode_tool_result_from_state(part: &Value, status: &str) -> Option<(Value, bool)> {
    let state = part.get("state")?;
    match status {
        "completed" => {
            let output = state
                .get("output")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            Some((output, false))
        }
        "error" | "cancelled" => {
            let error = state
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    state.get("output").map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string())
                    })
                })
                .unwrap_or_else(|| format!("Tool execution failed: {status}"));
            Some((Value::String(error), true))
        }
        _ => None,
    }
}

fn opencode_usage_from_tokens(tokens: Option<&Value>) -> Option<SessionMessageUsage> {
    let tokens = tokens?;
    let usage = SessionMessageUsage {
        input_tokens: tokens
            .get("input")
            .or_else(|| tokens.get("input_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as i64),
        output_tokens: tokens
            .get("output")
            .or_else(|| tokens.get("output_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as i64),
        cache_creation_input_tokens: tokens
            .get("cache")
            .and_then(|cache| cache.get("write"))
            .or_else(|| tokens.get("cache_creation_input_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as i64),
        cache_read_input_tokens: tokens
            .get("cache")
            .and_then(|cache| cache.get("read"))
            .or_else(|| tokens.get("cache_read_input_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as i64),
    };

    if usage.input_tokens.is_none()
        && usage.output_tokens.is_none()
        && usage.cache_creation_input_tokens.is_none()
        && usage.cache_read_input_tokens.is_none()
    {
        None
    } else {
        Some(usage)
    }
}

fn extract_part_text(part_value: &Value) -> Option<String> {
    let blocks = opencode_blocks_from_parts(std::slice::from_ref(part_value));
    if blocks.is_empty() {
        return None;
    }

    let message = message_from_blocks("assistant", None, blocks);
    if message.content.trim().is_empty() {
        None
    } else {
        Some(message.content)
    }
}

fn collect_parts_text(part_dir: &Path) -> String {
    if !part_dir.is_dir() {
        return String::new();
    }

    let blocks = collect_part_blocks(part_dir);
    if blocks.is_empty() {
        return String::new();
    }

    message_from_blocks("assistant", None, blocks).content
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
            let opencode_program = resolve_local_opencode_program();
            let mut command = build_local_std_command(&opencode_program.path);
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
        extract_session_id_from_snapshot, find_session_json_paths, load_messages,
        resolve_runtime_project_dir,
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
    fn scan_sessions_json_prefixes_resume_with_project_directory() {
        let test_dir = TestDir::new("resume-json-directory");
        let data_root = test_dir.path().join("data");
        let storage_root = data_root.join("storage");
        let session_id = "ses_resume_json";
        let session_file = storage_root
            .join("session")
            .join("global")
            .join(format!("{session_id}.json"));

        if let Some(parent) = session_file.parent() {
            fs::create_dir_all(parent).expect("failed to create session dir");
        }
        fs::write(
            &session_file,
            format!(
                r#"{{
                    "id": "{session_id}",
                    "directory": "/Users/tester/project with space",
                    "time": {{
                        "created": 1710000000000,
                        "updated": 1710000000001
                    }}
                }}"#
            ),
        )
        .expect("failed to write session file");

        let sessions = super::scan_sessions(&data_root, &data_root.join("opencode.db"), false);

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].resume_command.as_deref(),
            Some("cd '/Users/tester/project with space' && opencode -s ses_resume_json")
        );
    }

    #[test]
    fn scan_sessions_json_keeps_bare_resume_without_project_directory() {
        let test_dir = TestDir::new("resume-json-no-directory");
        let data_root = test_dir.path().join("data");
        let storage_root = data_root.join("storage");
        let session_id = "ses_resume_json_no_directory";
        let session_file = storage_root
            .join("session")
            .join("global")
            .join(format!("{session_id}.json"));

        if let Some(parent) = session_file.parent() {
            fs::create_dir_all(parent).expect("failed to create session dir");
        }
        fs::write(
            &session_file,
            format!(
                r#"{{
                    "id": "{session_id}",
                    "time": {{
                        "created": 1710000000000,
                        "updated": 1710000000001
                    }}
                }}"#
            ),
        )
        .expect("failed to write session file");

        let sessions = super::scan_sessions(&data_root, &data_root.join("opencode.db"), false);

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].resume_command.as_deref(),
            Some("opencode -s ses_resume_json_no_directory")
        );
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
    fn load_messages_json_structures_opencode_tool_parts() {
        let test_dir = TestDir::new("opencode-structured-parts");
        let storage_root = test_dir.path().join("storage");
        let session_id = "ses_structured_parts";
        let message_id = "msg_structured_parts";

        let message_dir = storage_root.join("message").join(session_id);
        let part_dir = storage_root.join("part").join(message_id);
        fs::create_dir_all(&message_dir).expect("failed to create message dir");
        fs::create_dir_all(&part_dir).expect("failed to create part dir");

        fs::write(
            message_dir.join(format!("{message_id}.json")),
            format!(
                r#"{{
                    "id":"{message_id}",
                    "role":"assistant",
                    "modelID":"anthropic/claude-sonnet-4",
                    "time":{{"created":1700000000000}},
                    "tokens":{{"input":12,"output":5,"cache":{{"read":3,"write":2}}}},
                    "cost":0.0012
                }}"#
            ),
        )
        .expect("failed to write message file");
        fs::write(
            part_dir.join("001-text.json"),
            r#"{"type":"text","text":"checking file"}"#,
        )
        .expect("failed to write text part");
        fs::write(
            part_dir.join("002-reasoning.json"),
            r#"{"type":"reasoning","text":"need to edit"}"#,
        )
        .expect("failed to write reasoning part");
        fs::write(
            part_dir.join("003-tool.json"),
            r#"{
                "type":"tool",
                "tool":"edit",
                "callID":"tool_opencode_edit",
                "state":{
                    "status":"completed",
                    "input":{
                        "filePath":"D:\\GitHub\\ai-toolbox\\src\\main.ts",
                        "oldString":"before",
                        "newString":"after"
                    },
                    "output":"updated"
                }
            }"#,
        )
        .expect("failed to write tool part");

        let messages =
            load_messages(&message_dir.to_string_lossy(), false).expect("load OpenCode messages");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id.as_deref(), Some(message_id));
        assert_eq!(
            messages[0].model.as_deref(),
            Some("anthropic/claude-sonnet-4")
        );
        assert_eq!(
            messages[0]
                .usage
                .as_ref()
                .and_then(|usage| usage.input_tokens),
            Some(12)
        );
        assert_eq!(
            messages[0]
                .usage
                .as_ref()
                .and_then(|usage| usage.cache_read_input_tokens),
            Some(3)
        );
        assert_eq!(messages[0].blocks.len(), 3);
        assert_eq!(messages[0].blocks[0].kind, "text");
        assert_eq!(messages[0].blocks[1].kind, "thinking");

        let tool_block = &messages[0].blocks[2];
        assert_eq!(tool_block.kind, "tool_execution");
        assert_eq!(tool_block.tool_id.as_deref(), Some("tool_opencode_edit"));
        assert_eq!(tool_block.tool_name.as_deref(), Some("Edit"));
        assert_eq!(tool_block.normalized_tool_name.as_deref(), Some("edit"));
        assert_eq!(tool_block.status.as_deref(), Some("success"));
        assert_eq!(
            tool_block
                .input
                .as_ref()
                .and_then(|input| input.get("file_path"))
                .and_then(serde_json::Value::as_str),
            Some("D:\\GitHub\\ai-toolbox\\src\\main.ts")
        );
        assert_eq!(
            tool_block
                .output
                .as_ref()
                .and_then(serde_json::Value::as_str),
            Some("updated")
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

        ensure_imported_session_visible(&data_root, session_id, "runtime=local", false)
            .expect("imported json session should be visible");
    }

    #[test]
    fn ensure_imported_session_visible_errors_when_session_is_missing() {
        let test_dir = TestDir::new("ensure-import-visible-missing");
        let data_root = test_dir.path().join("data");
        fs::create_dir_all(&data_root).expect("failed to create data root");

        let error = ensure_imported_session_visible(
            &data_root,
            "ses_import_missing",
            "runtime=local",
            false,
        )
        .expect_err("missing session should return error");

        assert!(error.contains("ses_import_missing"));
        assert!(error.contains("reported success"));
    }

    #[test]
    fn v2_switch_reads_session_v2_and_leaves_v1_rows_alone() {
        let test_dir = TestDir::new("opencode-v2-sessions");
        let database_path = test_dir.path().join("opencode.db");
        let connection = rusqlite::Connection::open(&database_path).expect("open db");
        connection
            .execute_batch(
                "
                CREATE TABLE session (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    directory TEXT NOT NULL,
                    time_created INTEGER NOT NULL,
                    time_updated INTEGER NOT NULL
                );
                CREATE TABLE message (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    time_created INTEGER NOT NULL,
                    data TEXT NOT NULL
                );
                CREATE TABLE part (
                    id TEXT PRIMARY KEY,
                    message_id TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    time_created INTEGER NOT NULL,
                    data TEXT NOT NULL
                );
                CREATE TABLE session_v2 (
                    id TEXT PRIMARY KEY,
                    parent_id TEXT,
                    title TEXT,
                    directory TEXT NOT NULL,
                    time_created INTEGER NOT NULL,
                    time_updated INTEGER NOT NULL
                );
                CREATE TABLE session_message (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    type TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    time_created INTEGER NOT NULL,
                    data TEXT NOT NULL
                );
                CREATE TABLE session_share (
                    session_id TEXT NOT NULL,
                    id TEXT NOT NULL
                );
                INSERT INTO session VALUES ('ses_v1_only', 'V1 only', 'D:/legacy', 1, 10);
                INSERT INTO session_v2 VALUES ('ses_v2_root', NULL, 'V2 root', 'D:/project', 2, 30);
                INSERT INTO session_v2 VALUES ('ses_v2_child', 'ses_v2_root', 'child', 'D:/project', 3, 40);
                INSERT INTO session_v2 VALUES ('ses_v2_untitled', NULL, NULL, 'D:/untitled', 4, 20);
                INSERT INTO session_share VALUES ('ses_v2_root', 'share_v1');
                INSERT INTO session_message VALUES (
                    'msg_user', 'ses_v2_root', 'user', 0, 2,
                    '{\"text\":\"inspect the tests\"}'
                );
                INSERT INTO session_message VALUES (
                    'msg_assistant', 'ses_v2_root', 'assistant', 1, 3,
                    '{\"model\":{\"id\":\"gpt-5\",\"providerID\":\"openai\"},\"content\":[{\"type\":\"text\",\"text\":\"looking\"},{\"type\":\"tool\",\"id\":\"call_1\",\"name\":\"read\",\"state\":{\"status\":\"completed\",\"input\":{\"filePath\":\"src/main.ts\"},\"content\":[{\"type\":\"text\",\"text\":\"file contents\"}]}}],\"tokens\":{\"input\":4,\"output\":2,\"cache\":{\"read\":1,\"write\":0}},\"cost\":0.02}'
                );
                ",
            )
            .expect("seed opencode db");
        drop(connection);

        let v1_sessions = super::scan_sessions(test_dir.path(), &database_path, false);
        assert_eq!(
            v1_sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ses_v1_only"]
        );

        let v2_sessions = super::scan_sessions(test_dir.path(), &database_path, true);
        let v2_ids = v2_sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(v2_ids, vec!["ses_v2_root", "ses_v2_untitled"]);
        assert_eq!(v2_sessions[1].title.as_deref(), Some("untitled"));
        assert!(v2_sessions[0].source_path.starts_with("sqlite:"));

        let source = v2_sessions[0].source_path.clone();
        let messages = super::load_messages(&source, true).expect("load v2 messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert!(messages[0].content.contains("inspect the tests"));
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].model.as_deref(), Some("gpt-5"));
        assert!(messages[1].content.contains("looking"));
        assert!(messages[1].blocks.iter().any(|block| {
            block.kind == "tool_execution"
                && block.output.as_ref().and_then(serde_json::Value::as_str)
                    == Some("file contents")
        }));
        assert_eq!(
            super::scan_messages_for_query(&source, "looking", true).expect("search"),
            true
        );
        assert_eq!(
            super::scan_messages_for_query(&source, "file contents", true)
                .expect("search tool output"),
            true
        );
        assert_eq!(
            super::scan_messages_for_query(&source, "src/main.ts", true)
                .expect("search tool input"),
            true
        );
        assert_eq!(
            super::scan_messages_for_query(&source, "file contents", false).expect("v1 search"),
            false
        );

        super::rename_session(&source, "renamed v2", true).expect("rename v2");
        let renamed = super::scan_sessions(test_dir.path(), &database_path, true);
        assert_eq!(renamed[0].title.as_deref(), Some("renamed v2"));
        let v1_after_rename = super::scan_sessions(test_dir.path(), &database_path, false);
        assert_eq!(v1_after_rename[0].title.as_deref(), Some("V1 only"));

        super::delete_session(&source, true).expect("delete v2");
        let remaining = super::scan_sessions(test_dir.path(), &database_path, true);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].session_id, "ses_v2_untitled");
        let v1_after_delete = super::scan_sessions(test_dir.path(), &database_path, false);
        assert_eq!(v1_after_delete[0].session_id, "ses_v1_only");
        let share_count: i64 = rusqlite::Connection::open(&database_path)
            .expect("reopen db")
            .query_row("SELECT COUNT(*) FROM session_share", [], |row| row.get(0))
            .expect("count shares");
        assert_eq!(share_count, 1);
    }
}
