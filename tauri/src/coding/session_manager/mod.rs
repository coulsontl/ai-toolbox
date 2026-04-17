mod claude_code;
mod codex;
mod open_claw;
mod open_code;
mod utils;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::coding::runtime_location::{
    build_windows_unc_path, expand_home_from_user_root, get_claude_runtime_location_async,
    get_codex_runtime_location_async, get_openclaw_runtime_location_async,
    get_opencode_runtime_location_async, RuntimeLocationInfo,
};
use crate::db::DbState;

const SESSION_CACHE_TTL: Duration = Duration::from_secs(15);
const MAX_SESSION_CACHE_ENTRIES: usize = 16;
const DEFAULT_SESSION_PATH_LIMIT: usize = 200;
const MAX_SESSION_PATH_LIMIT: usize = 500;
const EXPORT_SCHEMA_VERSION: u8 = 2;
const EXPORT_SCHEMA_NAME: &str = "ai-toolbox.session-export.v2";
const SNAPSHOT_FORMAT_CODEX: &str = "codex-jsonl";
const SNAPSHOT_FORMAT_CLAUDE_CODE: &str = "claudecode-project-session";
const SNAPSHOT_FORMAT_OPENCLAW: &str = "openclaw-agent-session";
const SNAPSHOT_FORMAT_OPENCODE: &str = "opencode-official-export";

#[derive(Debug, Clone)]
struct SessionCacheEntry {
    created_at: Instant,
    sessions: Vec<SessionMeta>,
}

static SESSION_LIST_CACHE: LazyLock<Mutex<HashMap<String, SessionCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<i64>,
    pub source_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListPage {
    pub items: Vec<SessionMeta>,
    pub page: u32,
    pub page_size: u32,
    pub total: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub meta: SessionMeta,
    pub messages: Vec<SessionMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeSnapshot {
    format: String,
    payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedSessionFile {
    version: u8,
    schema: String,
    tool: String,
    exported_at: String,
    meta: SessionMeta,
    normalized_messages: Vec<SessionMessage>,
    native_snapshot: NativeSnapshot,
}

#[derive(Debug, Clone)]
enum ToolSessionContext {
    Codex {
        sessions_root: PathBuf,
    },
    ClaudeCode {
        projects_root: PathBuf,
    },
    OpenClaw {
        agents_root: PathBuf,
    },
    OpenCode {
        runtime_location: RuntimeLocationInfo,
        config_path: PathBuf,
        data_root: PathBuf,
        state_root: PathBuf,
        sqlite_db_path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy)]
enum SessionTool {
    Codex,
    ClaudeCode,
    OpenClaw,
    OpenCode,
}

impl SessionTool {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "codex" => Ok(Self::Codex),
            "claudecode" | "claude_code" => Ok(Self::ClaudeCode),
            "openclaw" | "open_claw" => Ok(Self::OpenClaw),
            "opencode" | "open_code" => Ok(Self::OpenCode),
            _ => Err(format!("Unsupported session tool: {raw}")),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claudecode",
            Self::OpenClaw => "openclaw",
            Self::OpenCode => "opencode",
        }
    }
}

impl ToolSessionContext {
    fn cache_key(&self) -> String {
        match self {
            Self::Codex { sessions_root } => format!("codex:{}", sessions_root.display()),
            Self::ClaudeCode { projects_root } => {
                format!("claudecode:{}", projects_root.display())
            }
            Self::OpenClaw { agents_root } => format!("openclaw:{}", agents_root.display()),
            Self::OpenCode {
                runtime_location,
                config_path,
                data_root,
                state_root,
                sqlite_db_path,
            } => format!(
                "opencode:{}:{}:{}:{}:{}",
                runtime_location.host_path.display(),
                config_path.display(),
                data_root.display(),
                state_root.display(),
                sqlite_db_path.display()
            ),
        }
    }
}

#[tauri::command]
pub async fn list_tool_sessions(
    state: tauri::State<'_, DbState>,
    tool: String,
    query: Option<String>,
    path_filter: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
    force_refresh: Option<bool>,
) -> Result<SessionListPage, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let query = normalize_query(query);
    let path_filter = normalize_query(path_filter);
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(10).clamp(1, 50);
    let force_refresh = force_refresh.unwrap_or(false);
    let context = resolve_context(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || {
        list_sessions_blocking(
            context,
            query,
            path_filter,
            page as usize,
            page_size as usize,
            force_refresh,
        )
    })
    .await
    .map_err(|error| format!("Failed to list sessions: {error}"))?
}

#[tauri::command]
pub async fn list_tool_session_paths(
    state: tauri::State<'_, DbState>,
    tool: String,
    limit: Option<u32>,
    force_refresh: Option<bool>,
) -> Result<Vec<String>, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let limit = limit
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_SESSION_PATH_LIMIT)
        .clamp(1, MAX_SESSION_PATH_LIMIT);
    let force_refresh = force_refresh.unwrap_or(false);
    let context = resolve_context(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || {
        list_session_paths_blocking(context, limit, force_refresh)
    })
    .await
    .map_err(|error| format!("Failed to list session paths: {error}"))?
}

#[tauri::command]
pub async fn get_tool_session_detail(
    state: tauri::State<'_, DbState>,
    tool: String,
    source_path: String,
) -> Result<SessionDetail, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let context = resolve_context(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || get_session_detail_blocking(context, source_path))
        .await
        .map_err(|error| format!("Failed to load session detail: {error}"))?
}

#[tauri::command]
pub async fn delete_tool_session(
    state: tauri::State<'_, DbState>,
    tool: String,
    source_path: String,
) -> Result<(), String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let context = resolve_context(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || delete_session_blocking(context, source_path))
        .await
        .map_err(|error| format!("Failed to delete session: {error}"))?
}

#[tauri::command]
pub async fn export_tool_session(
    state: tauri::State<'_, DbState>,
    tool: String,
    source_path: String,
    export_path: String,
) -> Result<(), String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let context = resolve_context(&state.db(), session_tool).await?;
    let normalized_tool = session_tool.as_str().to_string();

    tauri::async_runtime::spawn_blocking(move || {
        export_session_blocking(context, normalized_tool, source_path, export_path)
    })
    .await
    .map_err(|error| format!("Failed to export session: {error}"))?
}

#[tauri::command]
pub async fn import_tool_session(
    state: tauri::State<'_, DbState>,
    tool: String,
    import_path: String,
) -> Result<(), String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let context = resolve_context(&state.db(), session_tool).await?;
    let normalized_tool = session_tool.as_str().to_string();

    tauri::async_runtime::spawn_blocking(move || {
        import_session_blocking(context, normalized_tool, import_path)
    })
    .await
    .map_err(|error| format!("Failed to import session: {error}"))?
}

#[tauri::command]
pub async fn rename_tool_session(
    state: tauri::State<'_, DbState>,
    tool: String,
    source_path: String,
    title: String,
) -> Result<(), String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let context = resolve_context(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || {
        rename_session_blocking(context, tool, source_path, title)
    })
    .await
    .map_err(|error| format!("Failed to rename session: {error}"))?
}

fn list_sessions_blocking(
    context: ToolSessionContext,
    query: Option<String>,
    path_filter: Option<String>,
    page: usize,
    page_size: usize,
    force_refresh: bool,
) -> Result<SessionListPage, String> {
    let sessions = get_cached_sessions(&context, force_refresh);
    let path_filtered_sessions = if let Some(path_filter_text) = path_filter.as_deref() {
        filter_sessions_by_path(sessions, path_filter_text)
    } else {
        sessions
    };
    let filtered_sessions = if let Some(query_text) = query.as_deref() {
        filter_sessions_by_query(&context, path_filtered_sessions, query_text)
    } else {
        path_filtered_sessions
    };

    let total = filtered_sessions.len();
    let start = page.saturating_sub(1) * page_size;
    let end = (start + page_size).min(total);
    let items = if start >= total {
        Vec::new()
    } else {
        filtered_sessions[start..end].to_vec()
    };

    Ok(SessionListPage {
        items,
        page: page as u32,
        page_size: page_size as u32,
        total,
        has_more: end < total,
    })
}

fn get_session_detail_blocking(
    context: ToolSessionContext,
    source_path: String,
) -> Result<SessionDetail, String> {
    let sessions = get_cached_sessions(&context, false);
    let meta = sessions
        .into_iter()
        .find(|session| match &context {
            ToolSessionContext::OpenCode { .. } => {
                open_code::same_session_source(&session.source_path, &source_path)
            }
            _ => session.source_path == source_path,
        })
        .ok_or_else(|| "Session not found".to_string())?;
    let messages = load_messages(&context, &meta.source_path)?;

    Ok(SessionDetail { meta, messages })
}

fn list_session_paths_blocking(
    context: ToolSessionContext,
    limit: usize,
    force_refresh: bool,
) -> Result<Vec<String>, String> {
    let sessions = get_cached_sessions(&context, force_refresh);
    let mut paths = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    for session in sessions {
        let Some(project_dir) = session.project_dir.as_deref() else {
            continue;
        };
        let normalized = project_dir.trim();
        if normalized.is_empty() {
            continue;
        }

        let dedupe_key = normalized.to_ascii_lowercase();
        if seen_paths.insert(dedupe_key) {
            paths.push(normalized.to_string());
        }

        if paths.len() >= limit {
            break;
        }
    }

    Ok(paths)
}

fn delete_session_blocking(context: ToolSessionContext, source_path: String) -> Result<(), String> {
    let session = get_cached_sessions(&context, true)
        .into_iter()
        .find(|item| match &context {
            ToolSessionContext::OpenCode { .. } => {
                open_code::same_session_source(&item.source_path, &source_path)
            }
            _ => item.source_path == source_path,
        })
        .ok_or_else(|| "Session not found".to_string())?;

    match &context {
        ToolSessionContext::Codex { .. } => {
            codex::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::ClaudeCode { .. } => {
            claude_code::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::OpenClaw { .. } => {
            open_claw::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::OpenCode { .. } => {
            open_code::delete_session(&session.source_path)?;
        }
    }

    invalidate_cache(&context);
    Ok(())
}

fn export_session_blocking(
    context: ToolSessionContext,
    tool: String,
    source_path: String,
    export_path: String,
) -> Result<(), String> {
    let session_detail = get_session_detail_blocking(context.clone(), source_path)?;
    let native_snapshot = build_native_snapshot(
        &session_detail.meta.source_path,
        &session_detail.meta,
        &session_detail.messages,
        &context,
    )?;
    let exported_file = ExportedSessionFile {
        version: EXPORT_SCHEMA_VERSION,
        schema: EXPORT_SCHEMA_NAME.to_string(),
        tool,
        exported_at: Utc::now().to_rfc3339(),
        meta: session_detail.meta,
        normalized_messages: session_detail.messages,
        native_snapshot,
    };
    let serialized = serde_json::to_string_pretty(&exported_file)
        .map_err(|error| format!("Failed to serialize session export: {error}"))?;

    let export_path_ref = Path::new(&export_path);
    if let Some(parent_dir) = export_path_ref.parent() {
        std::fs::create_dir_all(parent_dir).map_err(|error| {
            format!(
                "Failed to create export directory {}: {error}",
                parent_dir.display()
            )
        })?;
    }

    std::fs::write(export_path_ref, serialized).map_err(|error| {
        format!(
            "Failed to write exported session file {}: {error}",
            export_path_ref.display()
        )
    })?;

    Ok(())
}

fn import_session_blocking(
    context: ToolSessionContext,
    tool: String,
    import_path: String,
) -> Result<(), String> {
    let exported_file = read_exported_session_file(&import_path)?;
    validate_exported_session_file(&exported_file, &tool)?;

    let duplicate_exists = get_cached_sessions(&context, true)
        .into_iter()
        .any(|session| session.session_id == exported_file.meta.session_id);
    if duplicate_exists {
        return Err(format!(
            "Session {} already exists for {}",
            exported_file.meta.session_id, tool
        ));
    }

    match &context {
        ToolSessionContext::Codex { sessions_root } => {
            ensure_snapshot_format(&exported_file.native_snapshot, SNAPSHOT_FORMAT_CODEX)?;
            codex::import_native_snapshot(
                sessions_root,
                &exported_file.meta.session_id,
                &exported_file.native_snapshot.payload,
            )?;
        }
        ToolSessionContext::ClaudeCode { projects_root } => {
            ensure_snapshot_format(&exported_file.native_snapshot, SNAPSHOT_FORMAT_CLAUDE_CODE)?;
            claude_code::import_native_snapshot(
                projects_root,
                &exported_file.meta.session_id,
                &exported_file.native_snapshot.payload,
            )?;
        }
        ToolSessionContext::OpenClaw { .. } => {
            ensure_snapshot_format(&exported_file.native_snapshot, SNAPSHOT_FORMAT_OPENCLAW)?;
            if let ToolSessionContext::OpenClaw { agents_root } = &context {
                open_claw::import_native_snapshot(
                    agents_root,
                    &exported_file.meta.session_id,
                    &exported_file.native_snapshot.payload,
                )?;
            }
        }
        ToolSessionContext::OpenCode {
            runtime_location,
            config_path,
            data_root,
            state_root,
            ..
        } => {
            ensure_snapshot_format(&exported_file.native_snapshot, SNAPSHOT_FORMAT_OPENCODE)?;
            open_code::import_native_snapshot(
                &exported_file.native_snapshot.payload,
                Some(&exported_file.meta),
                Some(&exported_file.normalized_messages),
                exported_file.meta.project_dir.as_deref(),
                runtime_location,
                Some(config_path),
                Some(data_root),
                Some(state_root),
            )?;
        }
    }

    invalidate_cache(&context);
    Ok(())
}

fn rename_session_blocking(
    context: ToolSessionContext,
    _tool: String,
    source_path: String,
    title: String,
) -> Result<(), String> {
    match &context {
        ToolSessionContext::Codex { .. } => {
            codex::rename_session(&source_path, &title)?;
            invalidate_cache(&context);
            Ok(())
        }
        ToolSessionContext::OpenCode { .. } => {
            let session = get_cached_sessions(&context, true)
                .into_iter()
                .find(|item| open_code::same_session_source(&item.source_path, &source_path))
                .ok_or_else(|| "Session not found".to_string())?;
            open_code::rename_session(&session.source_path, &title)?;
            invalidate_cache(&context);
            Ok(())
        }
        _ => Err("This session tool does not support title editing".to_string()),
    }
}

fn build_native_snapshot(
    source_path: &str,
    meta: &SessionMeta,
    _messages: &[SessionMessage],
    context: &ToolSessionContext,
) -> Result<NativeSnapshot, String> {
    match context {
        ToolSessionContext::Codex { sessions_root } => Ok(NativeSnapshot {
            format: SNAPSHOT_FORMAT_CODEX.to_string(),
            payload: codex::export_native_snapshot(sessions_root, Path::new(source_path))?,
        }),
        ToolSessionContext::ClaudeCode { projects_root } => Ok(NativeSnapshot {
            format: SNAPSHOT_FORMAT_CLAUDE_CODE.to_string(),
            payload: claude_code::export_native_snapshot(projects_root, Path::new(source_path))?,
        }),
        ToolSessionContext::OpenClaw { agents_root } => Ok(NativeSnapshot {
            format: SNAPSHOT_FORMAT_OPENCLAW.to_string(),
            payload: open_claw::export_native_snapshot(agents_root, Path::new(source_path))?,
        }),
        ToolSessionContext::OpenCode {
            runtime_location,
            config_path,
            data_root,
            state_root,
            ..
        } => Ok(NativeSnapshot {
            format: SNAPSHOT_FORMAT_OPENCODE.to_string(),
            payload: open_code::export_native_snapshot(
                &meta.source_path,
                meta,
                _messages,
                runtime_location,
                Some(config_path),
                Some(data_root),
                Some(state_root),
            )?,
        }),
    }
}

fn read_exported_session_file(import_path: &str) -> Result<ExportedSessionFile, String> {
    let import_path_ref = Path::new(import_path);
    let data = std::fs::read_to_string(import_path_ref).map_err(|error| {
        format!(
            "Failed to read imported session file {}: {error}",
            import_path_ref.display()
        )
    })?;

    serde_json::from_str::<ExportedSessionFile>(&data).map_err(|error| {
        format!(
            "Invalid session export file {}: {error}",
            import_path_ref.display()
        )
    })
}

fn validate_exported_session_file(
    exported_file: &ExportedSessionFile,
    tool: &str,
) -> Result<(), String> {
    if exported_file.version != EXPORT_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported session export version: {}",
            exported_file.version
        ));
    }

    if exported_file.schema.trim() != EXPORT_SCHEMA_NAME {
        return Err(format!(
            "Unsupported session export schema: {}",
            exported_file.schema
        ));
    }

    let exported_tool = SessionTool::parse(exported_file.tool.trim())?
        .as_str()
        .to_string();

    if exported_tool != tool {
        return Err(format!(
            "Session export belongs to {}, but current tool is {}",
            exported_tool, tool
        ));
    }

    if exported_file.meta.session_id.trim().is_empty() {
        return Err("Session export is missing sessionId".to_string());
    }

    Ok(())
}

fn ensure_snapshot_format(snapshot: &NativeSnapshot, expected: &str) -> Result<(), String> {
    if snapshot.format == expected {
        return Ok(());
    }

    Err(format!(
        "Unexpected native snapshot format: expected {}, got {}",
        expected, snapshot.format
    ))
}

fn scan_sessions(context: &ToolSessionContext) -> Vec<SessionMeta> {
    let mut sessions = match context {
        ToolSessionContext::Codex { sessions_root } => codex::scan_sessions(sessions_root),
        ToolSessionContext::ClaudeCode { projects_root } => {
            claude_code::scan_sessions(projects_root)
        }
        ToolSessionContext::OpenClaw { agents_root } => open_claw::scan_sessions(agents_root),
        ToolSessionContext::OpenCode {
            data_root,
            sqlite_db_path,
            ..
        } => open_code::scan_sessions(data_root, sqlite_db_path),
    };

    sessions.sort_by(|left, right| {
        let left_ts = left.last_active_at.or(left.created_at).unwrap_or(0);
        let right_ts = right.last_active_at.or(right.created_at).unwrap_or(0);
        right_ts.cmp(&left_ts)
    });
    sessions
}

fn load_messages(
    context: &ToolSessionContext,
    source_path: &str,
) -> Result<Vec<SessionMessage>, String> {
    match context {
        ToolSessionContext::Codex { .. } => codex::load_messages(Path::new(source_path)),
        ToolSessionContext::ClaudeCode { .. } => claude_code::load_messages(Path::new(source_path)),
        ToolSessionContext::OpenClaw { .. } => open_claw::load_messages(Path::new(source_path)),
        ToolSessionContext::OpenCode { .. } => open_code::load_messages(source_path),
    }
}

fn get_cached_sessions(context: &ToolSessionContext, force_refresh: bool) -> Vec<SessionMeta> {
    let cache_key = context.cache_key();

    if let Ok(mut cache) = SESSION_LIST_CACHE.lock() {
        if force_refresh {
            cache.remove(&cache_key);
        } else if let Some(entry) = cache.get(&cache_key) {
            if entry.created_at.elapsed() <= SESSION_CACHE_TTL {
                return entry.sessions.clone();
            }

            cache.remove(&cache_key);
        }
    }

    let sessions = scan_sessions(context);

    if let Ok(mut cache) = SESSION_LIST_CACHE.lock() {
        cache.retain(|_, entry| entry.created_at.elapsed() <= SESSION_CACHE_TTL);

        if cache.len() >= MAX_SESSION_CACHE_ENTRIES {
            let oldest_key = cache
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(key, _)| key.clone());
            if let Some(oldest_key) = oldest_key {
                cache.remove(&oldest_key);
            }
        }

        cache.insert(
            cache_key,
            SessionCacheEntry {
                created_at: Instant::now(),
                sessions: sessions.clone(),
            },
        );
    }

    sessions
}

fn invalidate_cache(context: &ToolSessionContext) {
    let cache_key = context.cache_key();
    if let Ok(mut cache) = SESSION_LIST_CACHE.lock() {
        cache.remove(&cache_key);
    }
}

fn filter_sessions_by_query(
    context: &ToolSessionContext,
    sessions: Vec<SessionMeta>,
    query: &str,
) -> Vec<SessionMeta> {
    let query_lower = query.to_lowercase();

    sessions
        .into_iter()
        .filter(|session| {
            if meta_matches_query(session, &query_lower) {
                return true;
            }

            scan_session_content_for_query(context, &session.source_path, &query_lower)
                .unwrap_or(false)
        })
        .collect()
}

fn filter_sessions_by_path(sessions: Vec<SessionMeta>, path_filter: &str) -> Vec<SessionMeta> {
    let path_filter_lower = path_filter.to_ascii_lowercase();

    sessions
        .into_iter()
        .filter(|session| {
            session
                .project_dir
                .as_deref()
                .map(|value| contains_query(value, &path_filter_lower))
                .unwrap_or(false)
        })
        .collect()
}

fn scan_session_content_for_query(
    context: &ToolSessionContext,
    source_path: &str,
    query_lower: &str,
) -> Result<bool, String> {
    match context {
        ToolSessionContext::Codex { .. } => {
            codex::scan_messages_for_query(Path::new(source_path), query_lower)
        }
        ToolSessionContext::ClaudeCode { .. } => {
            claude_code::scan_messages_for_query(Path::new(source_path), query_lower)
        }
        ToolSessionContext::OpenClaw { .. } => {
            open_claw::scan_messages_for_query(Path::new(source_path), query_lower)
        }
        ToolSessionContext::OpenCode { .. } => {
            open_code::scan_messages_for_query(source_path, query_lower)
        }
    }
}

fn meta_matches_query(session: &SessionMeta, query_lower: &str) -> bool {
    contains_query(&session.session_id, query_lower)
        || session
            .title
            .as_deref()
            .map(|value| contains_query(value, query_lower))
            .unwrap_or(false)
        || session
            .summary
            .as_deref()
            .map(|value| contains_query(value, query_lower))
            .unwrap_or(false)
        || session
            .project_dir
            .as_deref()
            .map(|value| contains_query(value, query_lower))
            .unwrap_or(false)
}

fn contains_query(value: &str, query_lower: &str) -> bool {
    value.to_lowercase().contains(query_lower)
}

fn normalize_query(query: Option<String>) -> Option<String> {
    query
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn resolve_context(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: SessionTool,
) -> Result<ToolSessionContext, String> {
    match tool {
        SessionTool::Codex => {
            let runtime_location = get_codex_runtime_location_async(db).await?;
            Ok(ToolSessionContext::Codex {
                sessions_root: runtime_location.host_path.join("sessions"),
            })
        }
        SessionTool::ClaudeCode => {
            let runtime_location = get_claude_runtime_location_async(db).await?;
            Ok(ToolSessionContext::ClaudeCode {
                projects_root: runtime_location.host_path.join("projects"),
            })
        }
        SessionTool::OpenClaw => {
            let runtime_location = get_openclaw_runtime_location_async(db).await?;
            let config_dir = runtime_location
                .host_path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| "Failed to determine OpenClaw config directory".to_string())?;
            Ok(ToolSessionContext::OpenClaw {
                agents_root: config_dir.join("agents"),
            })
        }
        SessionTool::OpenCode => {
            let runtime_location = get_opencode_runtime_location_async(db).await?;
            let data_root = resolve_opencode_data_root(&runtime_location)?;
            let state_root = resolve_opencode_state_root(&runtime_location)?;
            Ok(ToolSessionContext::OpenCode {
                runtime_location: runtime_location.clone(),
                config_path: runtime_location.host_path,
                sqlite_db_path: data_root.join("opencode.db"),
                data_root,
                state_root,
            })
        }
    }
}

fn resolve_opencode_data_root(location: &RuntimeLocationInfo) -> Result<PathBuf, String> {
    if let Some(wsl) = &location.wsl {
        let linux_path =
            expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.local/share/opencode");
        return Ok(build_windows_unc_path(&wsl.distro, &linux_path));
    }

    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        if !data_home.trim().is_empty() {
            return Ok(PathBuf::from(data_home).join("opencode"));
        }
    }

    Ok(get_home_dir()?
        .join(".local")
        .join("share")
        .join("opencode"))
}

fn resolve_opencode_state_root(location: &RuntimeLocationInfo) -> Result<PathBuf, String> {
    if let Some(wsl) = &location.wsl {
        let linux_path =
            expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.local/state/opencode");
        return Ok(build_windows_unc_path(&wsl.distro, &linux_path));
    }

    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        if !state_home.trim().is_empty() {
            return Ok(PathBuf::from(state_home).join("opencode"));
        }
    }

    Ok(get_home_dir()?
        .join(".local")
        .join("state")
        .join("opencode"))
}

fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::process::Command;

    use serde_json::{json, Value};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ai-toolbox-session-manager-{label}-{}",
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

    struct EnvVarGuard {
        key: String,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: &Path) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key: key.to_string(),
                previous,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(&self.key, previous);
            } else {
                std::env::remove_var(&self.key);
            }
        }
    }

    struct OpenCodeEnv {
        home: PathBuf,
        xdg_data_home: PathBuf,
        xdg_cache_home: PathBuf,
        xdg_config_home: PathBuf,
        xdg_state_home: PathBuf,
    }

    impl OpenCodeEnv {
        fn new(root: &Path, name: &str) -> Self {
            let base = root.join(name);
            let home = base.join("home");
            let xdg_data_home = base.join("xdg-data");
            let xdg_cache_home = base.join("xdg-cache");
            let xdg_config_home = base.join("xdg-config");
            let xdg_state_home = base.join("xdg-state");

            fs::create_dir_all(&home).expect("failed to create opencode home");
            fs::create_dir_all(&xdg_data_home).expect("failed to create opencode data root");
            fs::create_dir_all(&xdg_cache_home).expect("failed to create opencode cache root");
            fs::create_dir_all(&xdg_config_home).expect("failed to create opencode config root");
            fs::create_dir_all(&xdg_state_home).expect("failed to create opencode state root");

            Self {
                home,
                xdg_data_home,
                xdg_cache_home,
                xdg_config_home,
                xdg_state_home,
            }
        }

        fn data_root(&self) -> PathBuf {
            self.xdg_data_home.join("opencode")
        }

        fn sqlite_db_path(&self) -> PathBuf {
            self.data_root().join("opencode.db")
        }

        fn apply_process_env(&self) -> Vec<EnvVarGuard> {
            vec![
                EnvVarGuard::set("HOME", &self.home),
                EnvVarGuard::set("XDG_DATA_HOME", &self.xdg_data_home),
                EnvVarGuard::set("XDG_CACHE_HOME", &self.xdg_cache_home),
                EnvVarGuard::set("XDG_CONFIG_HOME", &self.xdg_config_home),
                EnvVarGuard::set("XDG_STATE_HOME", &self.xdg_state_home),
                EnvVarGuard::set("OPENCODE_TEST_HOME", &self.home),
            ]
        }
    }

    #[test]
    fn round_trip_export_import_for_codex_claude_and_opencode() {
        let test_root = TestDir::new("round-trip");

        verify_codex_round_trip(test_root.path());
        verify_claude_code_round_trip(test_root.path());
        if skip_when_opencode_cli_missing("round_trip_export_import_for_codex_claude_and_opencode")
        {
            return;
        }
        verify_opencode_round_trip(test_root.path());
    }

    #[test]
    fn codex_round_trip_preserves_thread_name_index() {
        let test_root = TestDir::new("codex-thread-name");
        verify_codex_round_trip(test_root.path());
    }

    #[test]
    fn codex_rename_updates_session_index_and_scanned_title() {
        let test_root = TestDir::new("codex-rename");
        let session_id = "11111111-2222-3333-4444-555555555555";
        let original_thread_name = "Original Codex Session";
        let renamed_thread_name = "Renamed Codex Session";
        let project_dir = test_root.path().join("codex-project");
        fs::create_dir_all(&project_dir).expect("failed to create codex project dir");

        let codex_home = test_root.path().join("codex-home");
        let sessions_root = codex_home.join("sessions");
        let session_path = sessions_root
            .join("2026")
            .join("04")
            .join("04")
            .join(format!("rollout-2026-04-04T10-00-00-{session_id}.jsonl"));
        write_text_file(
            &session_path,
            &[
                json!({
                    "timestamp": "2026-04-04T10:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": session_id,
                        "timestamp": "2026-04-04T10:00:00Z",
                        "cwd": project_dir.to_string_lossy().to_string(),
                    }
                })
                .to_string(),
                json!({
                    "timestamp": "2026-04-04T10:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": "Codex rename prompt"
                            }
                        ]
                    }
                })
                .to_string(),
            ]
            .join("\n"),
        );
        write_text_file(
            &codex_home.join("session_index.jsonl"),
            &format!(
                "{{\"id\":\"{session_id}\",\"thread_name\":\"{original_thread_name}\",\"updated_at\":\"2026-04-04T10:01:00Z\"}}\n"
            ),
        );

        codex::rename_session(session_path.to_string_lossy().as_ref(), renamed_thread_name)
            .expect("codex rename should succeed");

        let scanned_session = codex::scan_sessions(&sessions_root)
            .into_iter()
            .find(|session| session.session_id == session_id)
            .expect("codex scanned session should exist");
        assert_eq!(scanned_session.title.as_deref(), Some(renamed_thread_name));

        let session_index_content = read_text_file(&codex_home.join("session_index.jsonl"));
        assert!(session_index_content.contains(original_thread_name));
        assert!(session_index_content.contains(renamed_thread_name));
        let last_line = session_index_content
            .lines()
            .last()
            .expect("session index should contain last line");
        let parsed_entry: Value =
            serde_json::from_str(last_line).expect("last session index line should be valid json");
        assert_eq!(
            parsed_entry.get("id").and_then(Value::as_str),
            Some(session_id)
        );
        assert_eq!(
            parsed_entry.get("thread_name").and_then(Value::as_str),
            Some(renamed_thread_name)
        );
    }

    #[test]
    fn validate_exported_session_file_accepts_tool_aliases() {
        let exported_file = ExportedSessionFile {
            version: EXPORT_SCHEMA_VERSION,
            schema: EXPORT_SCHEMA_NAME.to_string(),
            tool: "claude_code".to_string(),
            exported_at: "2026-03-31T00:00:00Z".to_string(),
            meta: SessionMeta {
                provider_id: "claudecode".to_string(),
                session_id: "session-1".to_string(),
                title: None,
                summary: None,
                project_dir: None,
                created_at: None,
                last_active_at: None,
                source_path: "/tmp/session.jsonl".to_string(),
                resume_command: None,
            },
            normalized_messages: Vec::new(),
            native_snapshot: NativeSnapshot {
                format: SNAPSHOT_FORMAT_CLAUDE_CODE.to_string(),
                payload: json!({}),
            },
        };

        let validation_result = validate_exported_session_file(&exported_file, "claudecode");

        assert!(validation_result.is_ok());
    }

    fn verify_codex_round_trip(test_root: &Path) {
        let session_id = "11111111-2222-3333-4444-555555555555";
        let thread_name = "Named Codex Session";
        let project_dir = test_root.join("codex-project");
        fs::create_dir_all(&project_dir).expect("failed to create codex project dir");

        let export_sessions_root = test_root.join("codex-export").join("sessions");
        let source_path = export_sessions_root
            .join("2026")
            .join("03")
            .join("31")
            .join(format!("rollout-2026-03-31T10-00-00-{session_id}.jsonl"));
        write_text_file(
            &source_path,
            &[
                json!({
                    "timestamp": "2026-03-31T10:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": session_id,
                        "timestamp": "2026-03-31T10:00:00Z",
                        "cwd": project_dir.to_string_lossy().to_string(),
                    }
                })
                .to_string(),
                json!({
                    "timestamp": "2026-03-31T10:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": "Codex round trip prompt"
                            }
                        ]
                    }
                })
                .to_string(),
                json!({
                    "timestamp": "2026-03-31T10:00:02Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [
                            {
                                "type": "output_text",
                                "text": "Codex round trip reply"
                            }
                        ]
                    }
                })
                .to_string(),
            ]
            .join("\n"),
        );
        write_text_file(
            &test_root.join("codex-export").join("session_index.jsonl"),
            &format!(
                "{{\"id\":\"{session_id}\",\"thread_name\":\"{thread_name}\",\"updated_at\":\"2026-03-31T10:05:00Z\"}}\n"
            ),
        );

        let export_file = test_root.join("codex-session-export.json");
        let export_context = ToolSessionContext::Codex {
            sessions_root: export_sessions_root.clone(),
        };
        export_session_blocking(
            export_context,
            "codex".to_string(),
            source_path.to_string_lossy().to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("codex export should succeed");

        let exported_file = read_json_file(&export_file);
        assert_eq!(
            exported_file.get("tool"),
            Some(&Value::String("codex".to_string()))
        );
        assert_eq!(
            exported_file.get("version"),
            Some(&Value::Number(serde_json::Number::from(2_u8)))
        );
        assert_eq!(
            exported_file.pointer("/nativeSnapshot/format"),
            Some(&Value::String("codex-jsonl".to_string()))
        );
        assert_eq!(
            exported_file.pointer("/nativeSnapshot/payload/sessionIndexEntry/thread_name"),
            Some(&Value::String(thread_name.to_string()))
        );

        let import_sessions_root = test_root.join("codex-import").join("sessions");
        fs::create_dir_all(&import_sessions_root).expect("failed to create codex import root");
        let import_context = ToolSessionContext::Codex {
            sessions_root: import_sessions_root.clone(),
        };
        import_session_blocking(
            import_context.clone(),
            "codex".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("codex import should succeed");

        let imported_sessions = codex::scan_sessions(&import_sessions_root);
        let imported_session = imported_sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .expect("codex imported session should exist");
        assert_eq!(
            imported_session.project_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );
        assert_eq!(imported_session.title.as_deref(), Some(thread_name));

        let imported_messages = codex::load_messages(Path::new(&imported_session.source_path))
            .expect("load codex messages");
        assert_eq!(imported_messages.len(), 2);
        assert_eq!(imported_messages[0].content, "Codex round trip prompt");
        assert_eq!(imported_messages[1].content, "Codex round trip reply");

        let imported_session_index =
            read_text_file(&test_root.join("codex-import").join("session_index.jsonl"));
        assert!(imported_session_index.contains(session_id));
        assert!(imported_session_index.contains(thread_name));

        let duplicate_error = import_session_blocking(
            import_context,
            "codex".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect_err("duplicate codex import should fail");
        assert!(duplicate_error.contains("already exists"));
    }

    fn verify_claude_code_round_trip(test_root: &Path) {
        let session_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let project_dir = test_root.join("claude-project");
        fs::create_dir_all(&project_dir).expect("failed to create claude project dir");

        let export_projects_root = test_root.join("claude-export").join("projects");
        let source_project_dir = export_projects_root.join("project-alpha");
        let source_path = source_project_dir.join(format!("{session_id}.jsonl"));
        write_text_file(
            &source_path,
            &[
                json!({
                    "parentUuid": Value::Null,
                    "isSidechain": false,
                    "userType": "external",
                    "cwd": project_dir.to_string_lossy().to_string(),
                    "sessionId": session_id,
                    "version": "2.1.39",
                    "type": "user",
                    "message": {
                        "role": "user",
                        "content": "Claude round trip prompt"
                    },
                    "uuid": "user-msg-1",
                    "timestamp": "2026-03-31T10:10:00Z"
                })
                .to_string(),
                json!({
                    "parentUuid": "user-msg-1",
                    "isSidechain": false,
                    "userType": "external",
                    "cwd": project_dir.to_string_lossy().to_string(),
                    "sessionId": session_id,
                    "version": "2.1.39",
                    "type": "assistant",
                    "message": {
                        "role": "assistant",
                        "content": "Claude round trip reply"
                    },
                    "uuid": "assistant-msg-1",
                    "timestamp": "2026-03-31T10:10:01Z"
                })
                .to_string(),
            ]
            .join("\n"),
        );

        let export_file = test_root.join("claude-session-export.json");
        let export_context = ToolSessionContext::ClaudeCode {
            projects_root: export_projects_root.clone(),
        };
        export_session_blocking(
            export_context,
            "claudecode".to_string(),
            source_path.to_string_lossy().to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("claude export should succeed");

        let exported_file = read_json_file(&export_file);
        assert_eq!(
            exported_file.get("tool"),
            Some(&Value::String("claudecode".to_string()))
        );
        assert_eq!(
            exported_file.pointer("/nativeSnapshot/format"),
            Some(&Value::String("claudecode-project-session".to_string()))
        );

        let import_projects_root = test_root.join("claude-import").join("projects");
        fs::create_dir_all(&import_projects_root).expect("failed to create claude import root");
        let import_context = ToolSessionContext::ClaudeCode {
            projects_root: import_projects_root.clone(),
        };
        import_session_blocking(
            import_context.clone(),
            "claudecode".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("claude import should succeed");

        let imported_sessions = claude_code::scan_sessions(&import_projects_root);
        let imported_session = imported_sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .expect("claude imported session should exist");
        assert_eq!(
            imported_session.project_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );

        let imported_messages =
            claude_code::load_messages(Path::new(&imported_session.source_path))
                .expect("load claude messages");
        assert_eq!(imported_messages.len(), 2);
        assert_eq!(imported_messages[0].content, "Claude round trip prompt");
        assert_eq!(imported_messages[1].content, "Claude round trip reply");

        let sessions_index_path = Path::new(&imported_session.source_path)
            .parent()
            .expect("claude imported project dir")
            .join("sessions-index.json");
        let sessions_index = read_json_file(&sessions_index_path);
        let entries = sessions_index
            .get("entries")
            .and_then(Value::as_array)
            .expect("claude sessions index entries");
        let imported_entry = entries
            .iter()
            .find(|entry| entry.get("sessionId").and_then(Value::as_str) == Some(session_id))
            .expect("claude sessions index should contain imported session");
        assert_eq!(
            imported_entry.get("fullPath").and_then(Value::as_str),
            Some(imported_session.source_path.as_str())
        );

        let duplicate_error = import_session_blocking(
            import_context,
            "claudecode".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect_err("duplicate claude import should fail");
        assert!(duplicate_error.contains("already exists"));
    }

    fn verify_opencode_round_trip(test_root: &Path) {
        let session_id = "ses_1234567890abABCDEFGHIJKLMN";
        let message_id = "msg_1234567890abABCDEFGHIJKLMN";
        let part_id = "prt_1234567890abABCDEFGHIJKLMN";
        let project_dir = test_root.join("opencode-project");
        fs::create_dir_all(&project_dir).expect("failed to create opencode project dir");

        let official_export_path = test_root.join("opencode-official-export.json");
        let official_export_json = json!({
            "info": {
                "id": session_id,
                "slug": "opencode-round-trip",
                "projectID": "global",
                "directory": project_dir.to_string_lossy().to_string(),
                "title": "OpenCode Round Trip",
                "version": "0.0.0",
                "time": {
                    "created": 1710000000000_i64,
                    "updated": 1710000005000_i64
                }
            },
            "messages": [
                {
                    "info": {
                        "id": message_id,
                        "sessionID": session_id,
                        "role": "user",
                        "time": {
                            "created": 1710000000000_i64
                        },
                        "agent": "build",
                        "model": {
                            "providerID": "openai",
                            "modelID": "gpt-5"
                        }
                    },
                    "parts": [
                        {
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": message_id,
                            "type": "text",
                            "text": "OpenCode round trip prompt"
                        }
                    ]
                }
            ]
        });
        write_text_file(
            &official_export_path,
            &serde_json::to_string_pretty(&official_export_json)
                .expect("serialize opencode official export"),
        );

        let export_env = OpenCodeEnv::new(test_root, "opencode-export-env");
        run_opencode_command(
            &export_env,
            &project_dir,
            &["import", official_export_path.to_string_lossy().as_ref()],
        );

        let export_data_root = export_env.data_root();
        let export_runtime_location = RuntimeLocationInfo {
            mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
            source: "test".to_string(),
            host_path: export_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            wsl: None,
        };
        let export_context = ToolSessionContext::OpenCode {
            runtime_location: export_runtime_location,
            config_path: export_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            data_root: export_data_root.clone(),
            state_root: export_env.xdg_state_home.join("opencode"),
            sqlite_db_path: export_env.sqlite_db_path(),
        };
        let source_session =
            open_code::scan_sessions(&export_data_root, &export_env.sqlite_db_path())
                .into_iter()
                .find(|session| session.session_id == session_id)
                .expect("opencode source session should exist");

        let export_file = test_root.join("opencode-session-export.json");
        let export_env_guards = export_env.apply_process_env();
        export_session_blocking(
            export_context,
            "opencode".to_string(),
            source_session.source_path.clone(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("opencode export should succeed");
        drop(export_env_guards);

        let exported_file = read_json_file(&export_file);
        assert_eq!(
            exported_file.get("tool"),
            Some(&Value::String("opencode".to_string()))
        );
        assert_eq!(
            exported_file.pointer("/nativeSnapshot/format"),
            Some(&Value::String("opencode-official-export".to_string()))
        );
        let exported_official_export_raw = exported_file
            .pointer("/nativeSnapshot/payload/officialExportRaw")
            .and_then(Value::as_str)
            .expect("opencode export should include officialExportRaw");
        let exported_official_export_json: Value = serde_json::from_str(exported_official_export_raw)
            .expect("parse exported official export raw json");
        assert_eq!(exported_official_export_json, official_export_json);

        let import_env = OpenCodeEnv::new(test_root, "opencode-import-env");
        let import_runtime_location = RuntimeLocationInfo {
            mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
            source: "test".to_string(),
            host_path: import_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            wsl: None,
        };
        let import_context = ToolSessionContext::OpenCode {
            runtime_location: import_runtime_location,
            config_path: import_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            data_root: import_env.data_root(),
            state_root: import_env.xdg_state_home.join("opencode"),
            sqlite_db_path: import_env.sqlite_db_path(),
        };
        let import_env_guards = import_env.apply_process_env();
        import_session_blocking(
            import_context.clone(),
            "opencode".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("opencode import should succeed");
        drop(import_env_guards);

        let imported_sessions =
            open_code::scan_sessions(&import_env.data_root(), &import_env.sqlite_db_path());
        let imported_session = imported_sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .expect("opencode imported session should exist");
        assert_eq!(
            imported_session.project_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );

        let imported_messages = open_code::load_messages(&imported_session.source_path)
            .expect("load opencode messages");
        assert_eq!(imported_messages.len(), 1);
        assert_eq!(imported_messages[0].content, "OpenCode round trip prompt");

        let exported_after_import =
            run_opencode_command(&import_env, &project_dir, &["export", session_id]);
        let exported_after_import_json: Value =
            serde_json::from_str(&exported_after_import).expect("parse opencode exported json");
        assert_eq!(
            exported_after_import_json
                .pointer("/info/id")
                .and_then(Value::as_str),
            Some(session_id)
        );
        assert_eq!(
            exported_after_import_json
                .pointer("/messages/0/parts/0/text")
                .and_then(Value::as_str),
            Some("OpenCode round trip prompt")
        );

        let duplicate_import_guards = import_env.apply_process_env();
        let duplicate_error = import_session_blocking(
            import_context,
            "opencode".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect_err("duplicate opencode import should fail");
        drop(duplicate_import_guards);
        assert!(duplicate_error.contains("already exists"));
    }

    #[test]
    fn opencode_import_accepts_raw_official_export_snapshot() {
        if skip_when_opencode_cli_missing("opencode_import_accepts_raw_official_export_snapshot") {
            return;
        }
        let test_root = TestDir::new("opencode-raw-import");
        let session_id = "ses_1234567890abRAWRAWRAWRAWRA";
        let message_id = "msg_1234567890abRAWRAWRAWRAWRA";
        let part_id = "prt_1234567890abRAWRAWRAWRAWRA";
        let project_dir = test_root.path().join("opencode-project");
        fs::create_dir_all(&project_dir).expect("failed to create opencode project dir");

        let official_export_json = json!({
            "info": {
                "id": session_id,
                "slug": "opencode-raw-import",
                "projectID": "global",
                "directory": project_dir.to_string_lossy().to_string(),
                "title": "OpenCode Raw Import",
                "version": "0.0.0",
                "time": {
                    "created": 1710000000000_i64,
                    "updated": 1710000005000_i64
                }
            },
            "messages": [
                {
                    "info": {
                        "id": message_id,
                        "sessionID": session_id,
                        "role": "user",
                        "time": {
                            "created": 1710000000000_i64
                        },
                        "agent": "build",
                        "model": {
                            "providerID": "openai",
                            "modelID": "gpt-5"
                        }
                    },
                    "parts": [
                        {
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": message_id,
                            "type": "text",
                            "text": "OpenCode raw import prompt"
                        }
                    ]
                }
            ]
        });
        let official_export_raw =
            serde_json::to_string_pretty(&official_export_json).expect("serialize raw export");

        let export_file = test_root.path().join("opencode-raw-export.json");
        write_text_file(
            &export_file,
            &serde_json::to_string_pretty(&json!({
                "version": EXPORT_SCHEMA_VERSION,
                "schema": EXPORT_SCHEMA_NAME,
                "tool": "opencode",
                "exportedAt": "2026-04-09T00:00:00Z",
                "meta": {
                    "providerId": "opencode",
                    "sessionId": session_id,
                    "title": "OpenCode Raw Import",
                    "summary": Value::Null,
                    "projectDir": project_dir.to_string_lossy().to_string(),
                    "createdAt": 1710000000000_i64,
                    "lastActiveAt": 1710000005000_i64,
                    "sourcePath": format!("sqlite:{}:{session_id}", test_root.path().join("unused.db").display()),
                    "resumeCommand": Value::Null
                },
                "normalizedMessages": [
                    {
                        "role": "user",
                        "content": "OpenCode raw import prompt",
                        "ts": 1710000000000_i64
                    }
                ],
                "nativeSnapshot": {
                    "format": SNAPSHOT_FORMAT_OPENCODE,
                    "payload": {
                        "sessionId": session_id,
                        "officialExportRaw": official_export_raw
                    }
                }
            }))
            .expect("serialize raw import exported session file"),
        );

        let import_env = OpenCodeEnv::new(test_root.path(), "opencode-raw-import-env");
        let import_context = ToolSessionContext::OpenCode {
            runtime_location: RuntimeLocationInfo {
                mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
                source: "test".to_string(),
                host_path: import_env
                    .xdg_config_home
                    .join("opencode")
                    .join("opencode.jsonc"),
                wsl: None,
            },
            config_path: import_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            data_root: import_env.data_root(),
            state_root: import_env.xdg_state_home.join("opencode"),
            sqlite_db_path: import_env.sqlite_db_path(),
        };

        let import_env_guards = import_env.apply_process_env();
        import_session_blocking(
            import_context,
            "opencode".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("raw official export import should succeed");
        drop(import_env_guards);

        let imported_sessions =
            open_code::scan_sessions(&import_env.data_root(), &import_env.sqlite_db_path());
        let imported_session = imported_sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .expect("opencode imported session should exist");
        assert_eq!(
            imported_session.project_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );

        let imported_messages = open_code::load_messages(&imported_session.source_path)
            .expect("load opencode raw-import messages");
        assert_eq!(imported_messages.len(), 1);
        assert_eq!(imported_messages[0].content, "OpenCode raw import prompt");
    }

    #[test]
    fn opencode_import_recovers_from_truncated_official_export_raw() {
        if skip_when_opencode_cli_missing(
            "opencode_import_recovers_from_truncated_official_export_raw",
        ) {
            return;
        }
        let test_root = TestDir::new("opencode-truncated-raw-import");
        let session_id = "ses_1234567890abTRUNCATEDRAW001";
        let project_dir = test_root.path().join("opencode-project");
        fs::create_dir_all(&project_dir).expect("failed to create opencode project dir");

        let export_file = test_root.path().join("opencode-truncated-raw-export.json");
        write_text_file(
            &export_file,
            &serde_json::to_string_pretty(&json!({
                "version": EXPORT_SCHEMA_VERSION,
                "schema": EXPORT_SCHEMA_NAME,
                "tool": "opencode",
                "exportedAt": "2026-04-09T00:00:00Z",
                "meta": {
                    "providerId": "opencode",
                    "sessionId": session_id,
                    "title": "Recovered Import",
                    "summary": "Recovered Import",
                    "projectDir": project_dir.to_string_lossy().to_string(),
                    "createdAt": 1710000000000_i64,
                    "lastActiveAt": 1710000005000_i64,
                    "sourcePath": format!("sqlite:{}:{session_id}", test_root.path().join("unused.db").display()),
                    "resumeCommand": Value::Null
                },
                "normalizedMessages": [
                    {
                        "role": "user",
                        "content": "Recovered import prompt",
                        "ts": 1710000000000_i64
                    },
                    {
                        "role": "assistant",
                        "content": "Recovered import answer",
                        "ts": 1710000005000_i64
                    }
                ],
                "nativeSnapshot": {
                    "format": SNAPSHOT_FORMAT_OPENCODE,
                    "payload": {
                        "sessionId": session_id,
                        "officialExportRaw": "{\n  \"info\": {\n    \"id\": \"ses_1234567890abTRUNCATEDRAW001\",\n    \"title\": \"Broken"
                    }
                }
            }))
            .expect("serialize truncated raw import exported session file"),
        );

        let import_env = OpenCodeEnv::new(test_root.path(), "opencode-truncated-raw-import-env");
        let import_context = ToolSessionContext::OpenCode {
            runtime_location: RuntimeLocationInfo {
                mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
                source: "test".to_string(),
                host_path: import_env
                    .xdg_config_home
                    .join("opencode")
                    .join("opencode.jsonc"),
                wsl: None,
            },
            config_path: import_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            data_root: import_env.data_root(),
            state_root: import_env.xdg_state_home.join("opencode"),
            sqlite_db_path: import_env.sqlite_db_path(),
        };

        let import_env_guards = import_env.apply_process_env();
        import_session_blocking(
            import_context,
            "opencode".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("truncated official export raw should be recovered during import");
        drop(import_env_guards);

        let imported_sessions =
            open_code::scan_sessions(&import_env.data_root(), &import_env.sqlite_db_path());
        let imported_session = imported_sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .expect("recovered opencode imported session should exist");
        assert_eq!(imported_session.title.as_deref(), Some("Recovered Import"));

        let imported_messages = open_code::load_messages(&imported_session.source_path)
            .expect("load recovered opencode messages");
        assert_eq!(imported_messages.len(), 2);
        assert_eq!(imported_messages[0].content, "Recovered import prompt");
        assert_eq!(imported_messages[1].content, "Recovered import answer");
    }

    #[test]
    fn opencode_import_recovers_truncated_raw_when_first_message_is_assistant() {
        if skip_when_opencode_cli_missing(
            "opencode_import_recovers_truncated_raw_when_first_message_is_assistant",
        ) {
            return;
        }
        let test_root = TestDir::new("opencode-truncated-raw-assistant-first-import");
        let session_id = "ses_1234567890abASSISTANTFIRST01";
        let project_dir = test_root.path().join("opencode-project");
        fs::create_dir_all(&project_dir).expect("failed to create opencode project dir");

        let export_file = test_root.path().join("opencode-truncated-raw-assistant-first.json");
        write_text_file(
            &export_file,
            &serde_json::to_string_pretty(&json!({
                "version": EXPORT_SCHEMA_VERSION,
                "schema": EXPORT_SCHEMA_NAME,
                "tool": "opencode",
                "exportedAt": "2026-04-11T00:00:00Z",
                "meta": {
                    "providerId": "opencode",
                    "sessionId": session_id,
                    "title": "Recovered Assistant First",
                    "summary": "Recovered Assistant First",
                    "projectDir": project_dir.to_string_lossy().to_string(),
                    "createdAt": 1710000100000_i64,
                    "lastActiveAt": 1710000105000_i64,
                    "sourcePath": format!("sqlite:{}:{session_id}", test_root.path().join("unused.db").display()),
                    "resumeCommand": Value::Null
                },
                "normalizedMessages": [
                    {
                        "role": "assistant",
                        "content": "Recovered answer without user parent",
                        "ts": 1710000105000_i64
                    }
                ],
                "nativeSnapshot": {
                    "format": SNAPSHOT_FORMAT_OPENCODE,
                    "payload": {
                        "sessionId": session_id,
                        "officialExportRaw": "{\n  \"info\": {\n    \"id\": \"ses_1234567890abASSISTANTFIRST01\",\n    \"title\": \"Broken"
                    }
                }
            }))
            .expect("serialize assistant-first truncated raw export"),
        );

        let import_env = OpenCodeEnv::new(
            test_root.path(),
            "opencode-truncated-raw-assistant-first-import-env",
        );
        let import_context = ToolSessionContext::OpenCode {
            runtime_location: RuntimeLocationInfo {
                mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
                source: "test".to_string(),
                host_path: import_env
                    .xdg_config_home
                    .join("opencode")
                    .join("opencode.jsonc"),
                wsl: None,
            },
            config_path: import_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            data_root: import_env.data_root(),
            state_root: import_env.xdg_state_home.join("opencode"),
            sqlite_db_path: import_env.sqlite_db_path(),
        };

        let import_env_guards = import_env.apply_process_env();
        import_session_blocking(
            import_context,
            "opencode".to_string(),
            export_file.to_string_lossy().to_string(),
        )
        .expect("assistant-first truncated raw should be recovered during import");
        drop(import_env_guards);

        let imported_sessions =
            open_code::scan_sessions(&import_env.data_root(), &import_env.sqlite_db_path());
        let imported_session = imported_sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .expect("assistant-first recovered session should exist");
        assert_eq!(
            imported_session.title.as_deref(),
            Some("Recovered Assistant First")
        );

        let imported_messages = open_code::load_messages(&imported_session.source_path)
            .expect("load assistant-first recovered opencode messages");
        assert_eq!(imported_messages.len(), 1);
        assert_eq!(
            imported_messages[0].content,
            "Recovered answer without user parent"
        );
    }

    #[test]
    fn opencode_export_uses_explicit_runtime_environment() {
        if skip_when_opencode_cli_missing("opencode_export_uses_explicit_runtime_environment") {
            return;
        }
        let test_root = TestDir::new("opencode-explicit-env");
        let session_id = "ses_1234567890abABCDEFGHIJKLMN";
        let message_id = "msg_1234567890abABCDEFGHIJKLMN";
        let part_id = "prt_1234567890abABCDEFGHIJKLMN";
        let project_dir = test_root.path().join("opencode-project");
        fs::create_dir_all(&project_dir).expect("failed to create opencode project dir");

        let official_export_path = test_root.path().join("opencode-official-export.json");
        let official_export_json = json!({
            "info": {
                "id": session_id,
                "slug": "opencode-explicit-env",
                "projectID": "global",
                "directory": project_dir.to_string_lossy().to_string(),
                "title": "OpenCode Explicit Env",
                "version": "0.0.0",
                "time": {
                    "created": 1710000000000_i64,
                    "updated": 1710000005000_i64
                }
            },
            "messages": [
                {
                    "info": {
                        "id": message_id,
                        "sessionID": session_id,
                        "role": "user",
                        "time": {
                            "created": 1710000000000_i64
                        },
                        "agent": "build",
                        "model": {
                            "providerID": "openai",
                            "modelID": "gpt-5"
                        }
                    },
                    "parts": [
                        {
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": message_id,
                            "type": "text",
                            "text": "OpenCode explicit env export"
                        }
                    ]
                }
            ]
        });
        write_text_file(
            &official_export_path,
            &serde_json::to_string_pretty(&official_export_json)
                .expect("serialize opencode official export"),
        );

        let source_env = OpenCodeEnv::new(test_root.path(), "source-env");
        run_opencode_command(
            &source_env,
            &project_dir,
            &["import", official_export_path.to_string_lossy().as_ref()],
        );

        let wrong_env = OpenCodeEnv::new(test_root.path(), "wrong-env");
        let wrong_env_guards = wrong_env.apply_process_env();
        let export_result = open_code::export_native_snapshot(
            &format!(
                "sqlite:{}:{}",
                source_env.sqlite_db_path().display(),
                session_id
            ),
            &SessionMeta {
                provider_id: "opencode".to_string(),
                session_id: session_id.to_string(),
                title: Some("OpenCode explicit env export".to_string()),
                summary: Some("OpenCode explicit env export".to_string()),
                project_dir: Some(project_dir.to_string_lossy().to_string()),
                created_at: Some(1710000000000_i64),
                last_active_at: Some(1710000005000_i64),
                source_path: format!(
                    "sqlite:{}:{}",
                    source_env.sqlite_db_path().display(),
                    session_id
                ),
                resume_command: Some(format!("opencode -s {session_id}")),
            },
            &[SessionMessage {
                role: "user".to_string(),
                content: "OpenCode explicit env export".to_string(),
                ts: Some(1710000000000_i64),
            }],
            &RuntimeLocationInfo {
                mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
                source: "test".to_string(),
                host_path: source_env
                    .xdg_config_home
                    .join("opencode")
                    .join("opencode.jsonc"),
                wsl: None,
            },
            Some(
                &source_env
                    .xdg_config_home
                    .join("opencode")
                    .join("opencode.jsonc"),
            ),
            Some(&source_env.data_root()),
            Some(&source_env.xdg_state_home.join("opencode")),
        )
        .expect("export should use explicit runtime environment");
        drop(wrong_env_guards);

        let official_export = export_result
            .get("officialExport")
            .expect("official export should exist when stdout is valid json");
        assert_eq!(
            official_export.pointer("/info/id").and_then(Value::as_str),
            Some(session_id)
        );
        assert_eq!(
            official_export
                .pointer("/messages/0/parts/0/text")
                .and_then(Value::as_str),
            Some("OpenCode explicit env export")
        );
        let raw_official_export = export_result
            .get("officialExportRaw")
            .and_then(Value::as_str)
            .expect("raw official export should exist");
        let raw_official_export_json: Value =
            serde_json::from_str(raw_official_export).expect("parse raw official export json");
        assert_eq!(
            raw_official_export_json
                .pointer("/info/id")
                .and_then(Value::as_str),
            Some(session_id)
        );
    }

    fn resolve_test_opencode_command() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        let lookup_command = "where";

        #[cfg(not(target_os = "windows"))]
        let lookup_command = "which";

        let output = Command::new(lookup_command).arg("opencode").output().ok()?;
        if !output.status.success() {
            return None;
        }

        let program = String::from_utf8(output.stdout).ok()?;
        let first_path = program
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())?;

        Some(PathBuf::from(first_path))
    }

    fn skip_when_opencode_cli_missing(test_name: &str) -> bool {
        if resolve_test_opencode_command().is_some() {
            return false;
        }

        eprintln!(
            "skip {test_name}: OpenCode CLI `opencode` is not available in PATH"
        );
        true
    }

    fn run_opencode_command(env: &OpenCodeEnv, current_dir: &Path, args: &[&str]) -> String {
        let program_path = resolve_test_opencode_command()
            .expect("opencode CLI should be available before running integration helper");
        let output = Command::new(&program_path)
            .args(args)
            .current_dir(current_dir)
            .env("HOME", &env.home)
            .env("XDG_DATA_HOME", &env.xdg_data_home)
            .env("XDG_CACHE_HOME", &env.xdg_cache_home)
            .env("XDG_CONFIG_HOME", &env.xdg_config_home)
            .env("XDG_STATE_HOME", &env.xdg_state_home)
            .env("OPENCODE_TEST_HOME", &env.home)
            .output()
            .expect("failed to run opencode command");

        if !output.status.success() {
            panic!(
                "opencode command failed: args={:?}, stdout={}, stderr={}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        String::from_utf8(output.stdout).expect("opencode stdout should be utf-8")
    }

    fn write_text_file(path: &Path, content: &str) {
        if let Some(parent_dir) = path.parent() {
            fs::create_dir_all(parent_dir).expect("failed to create parent directory");
        }
        fs::write(path, content).expect("failed to write test file");
    }

    fn read_text_file(path: &Path) -> String {
        fs::read_to_string(path).expect("failed to read text file")
    }

    fn read_json_file(path: &Path) -> Value {
        let data = fs::read_to_string(path).expect("failed to read json file");
        serde_json::from_str(&data).expect("failed to parse json file")
    }
}
