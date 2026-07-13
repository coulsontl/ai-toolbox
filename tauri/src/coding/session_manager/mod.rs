mod claude_code;
mod codex;
mod gemini_cli;
mod grok;
mod message_blocks;
mod open_claw;
mod open_code;
mod pi;
mod tool_normalizer;
mod utils;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::coding::runtime_location::{
    build_windows_unc_path, expand_home_from_user_root, get_claude_runtime_location_async,
    get_codex_runtime_location_async, get_gemini_cli_runtime_location_async,
    get_grok_runtime_location_async, get_openclaw_runtime_location_async,
    get_opencode_runtime_location_async, get_pi_runtime_location_async, RuntimeLocationInfo,
    RuntimeLocationMode, WslLocationInfo,
};
use crate::db::helpers::db_get;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;

const SESSION_CACHE_TTL: Duration = Duration::from_secs(15);
const MAX_SESSION_CACHE_ENTRIES: usize = 16;
const DEFAULT_SESSION_PATH_LIMIT: usize = 200;
const MAX_SESSION_PATH_LIMIT: usize = 500;
const EXPORT_SCHEMA_VERSION: u8 = 2;
const EXPORT_SCHEMA_NAME: &str = "ai-toolbox.session-export.v2";
const SNAPSHOT_FORMAT_CODEX: &str = "codex-jsonl";
const SNAPSHOT_FORMAT_CLAUDE_CODE: &str = "claudecode-project-session";
const SNAPSHOT_FORMAT_GEMINI_CLI: &str = "gemini-cli-session-json";
const SNAPSHOT_FORMAT_OPENCLAW: &str = "openclaw-agent-session";
const SNAPSHOT_FORMAT_OPENCODE: &str = "opencode-official-export";
const SNAPSHOT_FORMAT_PI: &str = "pi-session-jsonl";
const SNAPSHOT_FORMAT_GROK: &str = "grok-session-directory";
const GROK_NATIVE_EXPORT_SCHEMA: &str = "ai-toolbox.grok-native-snapshot.v1";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_distro: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<SessionMessageBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionMessageUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_sidechain: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageBlock {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

pub(super) fn assign_missing_message_ids(messages: &mut [SessionMessage], provider_id: &str) {
    for (index, message) in messages.iter_mut().enumerate() {
        if message
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        {
            continue;
        }

        message.id = Some(format!("{provider_id}-message-{index:06}"));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListPage {
    pub items: Vec<SessionMeta>,
    pub page: u32,
    pub page_size: u32,
    pub total: usize,
    pub has_more: bool,
    #[serde(default)]
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_state: Option<String>,
    #[serde(default)]
    pub meta_complete: bool,
    #[serde(default)]
    pub message_search_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_sources: Vec<SessionSourceOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSourceOption {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distro: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub meta: SessionMeta,
    pub messages: Vec<SessionMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSubagentMeta {
    pub id: String,
    pub source_path: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_message_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_time: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionFailure {
    pub source_path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteToolSessionsResult {
    pub deleted_count: usize,
    pub failed_items: Vec<DeleteSessionFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionItem {
    pub source_path: String,
    pub export_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionFailure {
    pub source_path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportToolSessionsResult {
    pub exported_count: usize,
    pub exported_items: Vec<ExportSessionItem>,
    pub failed_items: Vec<ExportSessionFailure>,
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
    GeminiCli {
        tmp_root: PathBuf,
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
    Pi {
        sessions_root: PathBuf,
    },
    Grok {
        sessions_root: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionRuntimeSource {
    Local,
    Wsl,
}

impl SessionRuntimeSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Wsl => "wsl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionSourceMode {
    All,
    Local,
    Wsl,
}

impl SessionSourceMode {
    fn parse(raw: Option<String>) -> Result<Self, String> {
        match raw
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("all")
        {
            "all" => Ok(Self::All),
            "local" => Ok(Self::Local),
            "wsl" => Ok(Self::Wsl),
            value => Err(format!("Unsupported session source mode: {value}")),
        }
    }

    fn accepts(self, source: SessionRuntimeSource) -> bool {
        matches!(self, Self::All)
            || matches!((self, source), (Self::Local, SessionRuntimeSource::Local))
            || matches!((self, source), (Self::Wsl, SessionRuntimeSource::Wsl))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionListLoadMode {
    Auto,
    CacheFirst,
    Full,
    Refresh,
}

impl SessionListLoadMode {
    fn parse(raw: Option<String>) -> Result<Self, String> {
        match raw
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("auto")
        {
            "auto" => Ok(Self::Auto),
            "cache-first" => Ok(Self::CacheFirst),
            "full" => Ok(Self::Full),
            "refresh" => Ok(Self::Refresh),
            value => Err(format!("Unsupported session list load mode: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionListCacheState {
    None,
    Quick,
    Stale,
    Fresh,
}

impl SessionListCacheState {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Quick => "quick",
            Self::Stale => "stale",
            Self::Fresh => "fresh",
        }
    }
}

#[derive(Debug, Clone)]
struct SessionContextEntry {
    context: ToolSessionContext,
    source: SessionRuntimeSource,
    distro: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionContextSet {
    entries: Vec<SessionContextEntry>,
    available_sources: Vec<SessionSourceOption>,
}

#[derive(Debug, Clone, Copy)]
enum SessionTool {
    Codex,
    ClaudeCode,
    GeminiCli,
    OpenClaw,
    OpenCode,
    Pi,
    Grok,
}

impl SessionTool {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "codex" => Ok(Self::Codex),
            "claudecode" | "claude_code" => Ok(Self::ClaudeCode),
            "geminicli" | "gemini_cli" | "gemini" => Ok(Self::GeminiCli),
            "openclaw" | "open_claw" => Ok(Self::OpenClaw),
            "opencode" | "open_code" => Ok(Self::OpenCode),
            "pi" => Ok(Self::Pi),
            "grok" => Ok(Self::Grok),
            _ => Err(format!("Unsupported session tool: {raw}")),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claudecode",
            Self::GeminiCli => "geminicli",
            Self::OpenClaw => "openclaw",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::Grok => "grok",
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
            Self::GeminiCli { tmp_root } => format!("geminicli:{}", tmp_root.display()),
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
            Self::Pi { sessions_root } => format!("pi:{}", sessions_root.display()),
            Self::Grok { sessions_root } => format!("grok:{}", sessions_root.display()),
        }
    }
}

#[tauri::command]
pub async fn list_tool_sessions(
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    query: Option<String>,
    path_filter: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
    force_refresh: Option<bool>,
    source_mode: Option<String>,
    load_mode: Option<String>,
) -> Result<SessionListPage, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let query = normalize_query(query);
    let path_filter = normalize_query(path_filter);
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(10).clamp(1, 50);
    let force_refresh = force_refresh.unwrap_or(false);
    let source_mode = SessionSourceMode::parse(source_mode)?;
    let load_mode = SessionListLoadMode::parse(load_mode)?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || {
        list_sessions_blocking(
            contexts,
            source_mode,
            query,
            path_filter,
            page as usize,
            page_size as usize,
            force_refresh,
            load_mode,
        )
    })
    .await
    .map_err(|error| format!("Failed to list sessions: {error}"))?
}

#[tauri::command]
pub async fn list_tool_session_paths(
    state: tauri::State<'_, SqliteDbState>,
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
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    source_path: String,
) -> Result<SessionDetail, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || get_session_detail_blocking(contexts, source_path))
        .await
        .map_err(|error| format!("Failed to load session detail: {error}"))?
}

#[tauri::command]
pub async fn list_tool_session_subagents(
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    source_path: String,
) -> Result<Vec<SessionSubagentMeta>, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || {
        list_session_subagents_blocking(contexts, source_path)
    })
    .await
    .map_err(|error| format!("Failed to list subagent sessions: {error}"))?
}

#[tauri::command]
pub async fn get_tool_subagent_session_detail(
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    parent_source_path: String,
    subagent_source_path: String,
) -> Result<SessionDetail, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || {
        get_subagent_session_detail_blocking(contexts, parent_source_path, subagent_source_path)
    })
    .await
    .map_err(|error| format!("Failed to load subagent session detail: {error}"))?
}

#[tauri::command]
pub async fn delete_tool_session(
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    source_path: String,
) -> Result<(), String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || delete_session_blocking(contexts, source_path))
        .await
        .map_err(|error| format!("Failed to delete session: {error}"))?
}

#[tauri::command]
pub async fn delete_tool_sessions(
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    source_paths: Vec<String>,
) -> Result<DeleteToolSessionsResult, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || delete_sessions_blocking(contexts, source_paths))
        .await
        .map_err(|error| format!("Failed to delete sessions: {error}"))
}

#[tauri::command]
pub async fn export_tool_session(
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    source_path: String,
    export_path: String,
    export_format: Option<String>,
) -> Result<(), String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;
    let normalized_tool = session_tool.as_str().to_string();

    tauri::async_runtime::spawn_blocking(move || {
        export_session_blocking(
            contexts,
            normalized_tool,
            source_path,
            export_path,
            export_format.as_deref().unwrap_or("ai_toolbox").to_string(),
        )
    })
    .await
    .map_err(|error| format!("Failed to export session: {error}"))?
}

#[tauri::command]
pub async fn export_tool_sessions(
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    source_paths: Vec<String>,
    export_dir: String,
    export_format: Option<String>,
) -> Result<ExportToolSessionsResult, String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;
    let normalized_tool = session_tool.as_str().to_string();

    tauri::async_runtime::spawn_blocking(move || {
        export_sessions_blocking(
            contexts,
            normalized_tool,
            source_paths,
            export_dir,
            export_format.as_deref().unwrap_or("ai_toolbox").to_string(),
        )
    })
    .await
    .map_err(|error| format!("Failed to export sessions: {error}"))?
}

#[tauri::command]
pub async fn import_tool_session(
    state: tauri::State<'_, SqliteDbState>,
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
    state: tauri::State<'_, SqliteDbState>,
    tool: String,
    source_path: String,
    title: String,
) -> Result<(), String> {
    let session_tool = SessionTool::parse(tool.trim())?;
    let contexts = resolve_session_contexts(&state.db(), session_tool).await?;

    tauri::async_runtime::spawn_blocking(move || {
        rename_session_blocking(contexts, tool, source_path, title)
    })
    .await
    .map_err(|error| format!("Failed to rename session: {error}"))?
}

#[derive(Debug, Clone)]
struct SessionWithContext {
    context_index: usize,
    meta: SessionMeta,
}

fn collect_sessions_with_context(
    contexts: &SessionContextSet,
    source_mode: SessionSourceMode,
    force_refresh: bool,
) -> Vec<SessionWithContext> {
    let mut sessions = Vec::new();

    for (context_index, entry) in contexts.entries.iter().enumerate() {
        if !source_mode.accepts(entry.source) {
            continue;
        }

        let scanned_sessions = get_cached_sessions(&entry.context, force_refresh);
        sessions.extend(
            scanned_sessions
                .into_iter()
                .map(|session| SessionWithContext {
                    context_index,
                    meta: annotate_session_source(session, entry),
                }),
        );
    }

    sessions
}

fn collect_recent_sessions_with_context(
    contexts: &SessionContextSet,
    source_mode: SessionSourceMode,
    limit: usize,
) -> Vec<SessionWithContext> {
    let mut sessions = Vec::new();

    for (context_index, entry) in contexts.entries.iter().enumerate() {
        if !source_mode.accepts(entry.source) {
            continue;
        }

        let recent_sessions = scan_recent_sessions(&entry.context, limit);
        sessions.extend(
            recent_sessions
                .into_iter()
                .map(|session| SessionWithContext {
                    context_index,
                    meta: annotate_session_source(session, entry),
                }),
        );
    }

    sessions
}

fn collect_fresh_cached_sessions_with_context(
    contexts: &SessionContextSet,
    source_mode: SessionSourceMode,
) -> Option<Vec<SessionWithContext>> {
    let mut sessions = Vec::new();

    for (context_index, entry) in contexts.entries.iter().enumerate() {
        if !source_mode.accepts(entry.source) {
            continue;
        }

        let cached_sessions = get_fresh_cached_sessions(&entry.context)?;
        sessions.extend(
            cached_sessions
                .into_iter()
                .map(|session| SessionWithContext {
                    context_index,
                    meta: annotate_session_source(session, entry),
                }),
        );
    }

    Some(sessions)
}

fn collect_any_cached_sessions_with_context(
    contexts: &SessionContextSet,
    source_mode: SessionSourceMode,
) -> (Vec<SessionWithContext>, bool, SessionListCacheState) {
    let mut sessions = Vec::new();
    let mut cache_state = SessionListCacheState::Fresh;
    let mut accepted_context_count = 0usize;
    let mut missing_context_count = 0usize;

    for (context_index, entry) in contexts.entries.iter().enumerate() {
        if !source_mode.accepts(entry.source) {
            continue;
        }
        accepted_context_count += 1;

        let Some((cached_sessions, context_cache_state)) = get_any_cached_sessions(&entry.context)
        else {
            missing_context_count += 1;
            continue;
        };
        if context_cache_state == SessionListCacheState::Stale {
            cache_state = SessionListCacheState::Stale;
        }
        sessions.extend(
            cached_sessions
                .into_iter()
                .map(|session| SessionWithContext {
                    context_index,
                    meta: annotate_session_source(session, entry),
                }),
        );
    }

    if accepted_context_count == 0 {
        return (sessions, false, SessionListCacheState::None);
    }

    if missing_context_count > 0 {
        cache_state = SessionListCacheState::Quick;
    }

    (sessions, missing_context_count > 0, cache_state)
}

fn collect_quick_local_sessions_with_context(
    contexts: &SessionContextSet,
    source_mode: SessionSourceMode,
    limit: usize,
) -> Vec<SessionWithContext> {
    let mut sessions = Vec::new();

    for (context_index, entry) in contexts.entries.iter().enumerate() {
        if !source_mode.accepts(entry.source) || entry.source != SessionRuntimeSource::Local {
            continue;
        }

        let recent_sessions = scan_recent_sessions(&entry.context, limit);
        sessions.extend(
            recent_sessions
                .into_iter()
                .map(|session| SessionWithContext {
                    context_index,
                    meta: annotate_session_source(session, entry),
                }),
        );
    }

    sessions
}

fn annotate_session_source(mut session: SessionMeta, entry: &SessionContextEntry) -> SessionMeta {
    session.runtime_source = Some(entry.source.as_str().to_string());
    session.runtime_distro = entry.distro.clone();
    session
}

fn find_session_with_context(
    contexts: &SessionContextSet,
    source_path: &str,
    force_refresh: bool,
) -> Result<(SessionContextEntry, SessionMeta), String> {
    for entry in &contexts.entries {
        if let Some(session) = get_cached_sessions(&entry.context, force_refresh)
            .into_iter()
            .find(|session| {
                matches_session_source(&entry.context, &session.source_path, source_path)
            })
        {
            return Ok((entry.clone(), annotate_session_source(session, entry)));
        }
    }

    Err("Session not found".to_string())
}

fn build_session_paths_from_contexts(sessions: &[SessionWithContext], limit: usize) -> Vec<String> {
    let metas: Vec<SessionMeta> = sessions
        .iter()
        .map(|session| session.meta.clone())
        .collect();
    build_session_paths(&metas, limit)
}

fn filter_sessions_by_path_with_context(
    sessions: Vec<SessionWithContext>,
    path_filter: &str,
) -> Vec<SessionWithContext> {
    let path_filter_lower = path_filter.to_ascii_lowercase();

    sessions
        .into_iter()
        .filter(|session| {
            session
                .meta
                .project_dir
                .as_deref()
                .map(|value| contains_query(value, &path_filter_lower))
                .unwrap_or(false)
        })
        .collect()
}

fn filter_sessions_by_query_with_context(
    contexts: &SessionContextSet,
    sessions: Vec<SessionWithContext>,
    query: &str,
    include_message_content: bool,
) -> (Vec<SessionWithContext>, bool) {
    let query_lower = query.to_lowercase();
    let exact_session_id_matches: Vec<SessionWithContext> = sessions
        .iter()
        .filter(|session| session_id_exact_matches_query(&session.meta, &query_lower))
        .cloned()
        .collect();
    if !exact_session_id_matches.is_empty() {
        return (exact_session_id_matches, true);
    }

    let filtered_sessions = sessions
        .into_iter()
        .filter(|session| {
            if meta_matches_query(&session.meta, &query_lower) {
                return true;
            }

            if !include_message_content {
                return false;
            }

            contexts
                .entries
                .get(session.context_index)
                .map(|entry| {
                    scan_session_content_for_query(
                        &entry.context,
                        &session.meta.source_path,
                        &query_lower,
                    )
                    .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .collect();

    (filtered_sessions, false)
}

fn list_sessions_blocking(
    contexts: SessionContextSet,
    source_mode: SessionSourceMode,
    query: Option<String>,
    path_filter: Option<String>,
    page: usize,
    page_size: usize,
    force_refresh: bool,
    load_mode: SessionListLoadMode,
) -> Result<SessionListPage, String> {
    let use_quick_initial_page =
        page == 1 && page_size <= 10 && query.is_none() && path_filter.is_none() && !force_refresh;
    let (mut sessions, partial, cache_state, meta_complete) = match load_mode {
        SessionListLoadMode::CacheFirst => {
            let (mut cached_sessions, cache_partial, cache_state) =
                collect_any_cached_sessions_with_context(&contexts, source_mode);
            if cached_sessions.is_empty() && cache_partial {
                cached_sessions =
                    collect_quick_local_sessions_with_context(&contexts, source_mode, page_size);
            }
            (cached_sessions, cache_partial, cache_state, !cache_partial)
        }
        SessionListLoadMode::Full | SessionListLoadMode::Refresh => (
            collect_sessions_with_context(
                &contexts,
                source_mode,
                force_refresh || load_mode == SessionListLoadMode::Refresh,
            ),
            false,
            SessionListCacheState::Fresh,
            true,
        ),
        SessionListLoadMode::Auto => {
            if use_quick_initial_page {
                match collect_fresh_cached_sessions_with_context(&contexts, source_mode) {
                    Some(cached_sessions) => {
                        (cached_sessions, false, SessionListCacheState::Fresh, true)
                    }
                    None => (
                        collect_recent_sessions_with_context(&contexts, source_mode, page_size),
                        true,
                        SessionListCacheState::Quick,
                        false,
                    ),
                }
            } else {
                (
                    collect_sessions_with_context(&contexts, source_mode, force_refresh),
                    false,
                    SessionListCacheState::Fresh,
                    true,
                )
            }
        }
    };
    let include_message_content = query.is_some()
        && matches!(
            load_mode,
            SessionListLoadMode::Auto | SessionListLoadMode::Full | SessionListLoadMode::Refresh
        );

    sessions.sort_by(|left, right| {
        let left_ts = left
            .meta
            .last_active_at
            .or(left.meta.created_at)
            .unwrap_or(0);
        let right_ts = right
            .meta
            .last_active_at
            .or(right.meta.created_at)
            .unwrap_or(0);
        right_ts.cmp(&left_ts)
    });

    let available_paths = build_session_paths_from_contexts(&sessions, DEFAULT_SESSION_PATH_LIMIT);
    let path_filtered_sessions = if let Some(path_filter_text) = path_filter.as_deref() {
        filter_sessions_by_path_with_context(sessions, path_filter_text)
    } else {
        sessions
    };
    let (filtered_sessions, exact_session_id_match) = if let Some(query_text) = query.as_deref() {
        filter_sessions_by_query_with_context(
            &contexts,
            path_filtered_sessions,
            query_text,
            include_message_content,
        )
    } else {
        (path_filtered_sessions, false)
    };
    let message_search_complete =
        query.is_none() || include_message_content || exact_session_id_match;

    let total = filtered_sessions.len();
    let items = filtered_sessions
        .iter()
        .map(|session| session.meta.clone())
        .collect();
    Ok(SessionListPage {
        items,
        page: page as u32,
        page_size: page_size as u32,
        total,
        has_more: false,
        partial,
        cache_state: Some(cache_state.as_str().to_string()),
        meta_complete,
        message_search_complete,
        available_paths: Some(available_paths),
        available_sources: contexts.available_sources,
    })
}

fn get_session_detail_blocking(
    contexts: SessionContextSet,
    source_path: String,
) -> Result<SessionDetail, String> {
    let (entry, meta) = find_session_with_context(&contexts, &source_path, false)?;
    let messages = load_messages(&entry.context, &meta.source_path)?;

    Ok(SessionDetail { meta, messages })
}

fn list_session_subagents_blocking(
    contexts: SessionContextSet,
    source_path: String,
) -> Result<Vec<SessionSubagentMeta>, String> {
    let (entry, meta) = find_session_with_context(&contexts, &source_path, false)?;
    let subagents = list_subagent_sessions(&entry.context, &meta.source_path);
    Ok(subagents)
}

fn get_subagent_session_detail_blocking(
    contexts: SessionContextSet,
    parent_source_path: String,
    subagent_source_path: String,
) -> Result<SessionDetail, String> {
    let (entry, parent) = find_session_with_context(&contexts, &parent_source_path, false)?;
    let subagent = list_subagent_sessions(&entry.context, &parent.source_path)
        .into_iter()
        .find(|item| item.source_path == subagent_source_path)
        .ok_or_else(|| "SubAgent session not found".to_string())?;

    let messages = load_messages(&entry.context, &subagent.source_path)?;
    let meta = SessionMeta {
        provider_id: parent.provider_id,
        session_id: subagent.id.clone(),
        title: Some(subagent.title),
        summary: subagent.summary,
        project_dir: parent.project_dir,
        created_at: subagent.first_message_time,
        last_active_at: subagent.last_message_time,
        source_path: subagent.source_path,
        resume_command: None,
        runtime_source: parent.runtime_source,
        runtime_distro: parent.runtime_distro,
    };

    Ok(SessionDetail { meta, messages })
}

fn list_session_paths_blocking(
    context: ToolSessionContext,
    limit: usize,
    force_refresh: bool,
) -> Result<Vec<String>, String> {
    let sessions = get_cached_sessions(&context, force_refresh);
    Ok(build_session_paths(&sessions, limit))
}

fn delete_session_blocking(contexts: SessionContextSet, source_path: String) -> Result<(), String> {
    match find_session_with_context(&contexts, &source_path, true) {
        Ok((entry, session)) => {
            delete_session_from_meta(&entry.context, &session)?;
            invalidate_cache(&entry.context);
            Ok(())
        }
        Err(error) => {
            let mut handled_by_opencode = false;
            for entry in &contexts.entries {
                if matches!(entry.context, ToolSessionContext::OpenCode { .. }) {
                    open_code::delete_session(&source_path)?;
                    invalidate_cache(&entry.context);
                    handled_by_opencode = true;
                }
            }

            if handled_by_opencode {
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

fn matches_session_source(
    context: &ToolSessionContext,
    session_source_path: &str,
    requested_source_path: &str,
) -> bool {
    match context {
        ToolSessionContext::OpenCode { .. } => {
            open_code::same_session_source(session_source_path, requested_source_path)
        }
        _ => session_source_path == requested_source_path,
    }
}

fn session_dedupe_key(context: &ToolSessionContext, source_path: &str) -> String {
    let source_key = match context {
        ToolSessionContext::OpenCode { .. } => open_code::session_source_key(source_path)
            .unwrap_or_else(|_| source_path.to_ascii_lowercase()),
        _ => source_path.to_ascii_lowercase(),
    };

    format!("{}:{}", context.cache_key(), source_key)
}

fn delete_session_from_meta(
    context: &ToolSessionContext,
    session: &SessionMeta,
) -> Result<(), String> {
    match context {
        ToolSessionContext::Codex { .. } => {
            codex::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::ClaudeCode { .. } => {
            claude_code::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::GeminiCli { .. } => {
            gemini_cli::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::OpenClaw { .. } => {
            open_claw::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::OpenCode { .. } => {
            open_code::delete_session(&session.source_path)?;
        }
        ToolSessionContext::Pi { .. } => {
            pi::delete_session(Path::new(&session.source_path))?;
        }
        ToolSessionContext::Grok { sessions_root } => {
            grok::delete_session(sessions_root, Path::new(&session.source_path))?;
        }
    }

    Ok(())
}

fn delete_sessions_blocking(
    contexts: SessionContextSet,
    source_paths: Vec<String>,
) -> DeleteToolSessionsResult {
    let mut deleted_count = 0usize;
    let mut failed_items = Vec::new();
    let mut seen_paths = HashSet::new();

    for source_path in source_paths {
        let trimmed_source_path = source_path.trim();
        if trimmed_source_path.is_empty() {
            continue;
        }

        let (entry, session) = match find_session_with_context(&contexts, trimmed_source_path, true)
        {
            Ok(found) => found,
            Err(error) => {
                failed_items.push(DeleteSessionFailure {
                    source_path: trimmed_source_path.to_string(),
                    error,
                });
                continue;
            }
        };

        let dedupe_key = session_dedupe_key(&entry.context, &session.source_path);
        if !seen_paths.insert(dedupe_key) {
            continue;
        }

        match delete_session_from_meta(&entry.context, &session) {
            Ok(()) => {
                deleted_count += 1;
                invalidate_cache(&entry.context);
            }
            Err(error) => {
                failed_items.push(DeleteSessionFailure {
                    source_path: trimmed_source_path.to_string(),
                    error,
                });
            }
        }
    }

    DeleteToolSessionsResult {
        deleted_count,
        failed_items,
    }
}

fn build_session_paths(sessions: &[SessionMeta], limit: usize) -> Vec<String> {
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

    paths
}

fn export_session_blocking(
    contexts: SessionContextSet,
    tool: String,
    source_path: String,
    export_path: String,
    export_format: String,
) -> Result<(), String> {
    let (entry, meta) = find_session_with_context(&contexts, &source_path, false)?;
    export_session_to_path(
        &entry.context,
        &tool,
        &meta,
        &export_format,
        Path::new(&export_path),
    )
}

fn export_sessions_blocking(
    contexts: SessionContextSet,
    tool: String,
    source_paths: Vec<String>,
    export_dir: String,
    export_format: String,
) -> Result<ExportToolSessionsResult, String> {
    let export_dir_ref = Path::new(&export_dir);
    std::fs::create_dir_all(export_dir_ref).map_err(|error| {
        format!(
            "Failed to create export directory {}: {error}",
            export_dir_ref.display()
        )
    })?;

    let mut exported_items = Vec::new();
    let mut failed_items = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut used_file_names = HashSet::new();

    for source_path in source_paths {
        let trimmed_source_path = source_path.trim();
        if trimmed_source_path.is_empty() {
            continue;
        }

        let (entry, session) =
            match find_session_with_context(&contexts, trimmed_source_path, false) {
                Ok(found) => found,
                Err(error) => {
                    failed_items.push(ExportSessionFailure {
                        source_path: trimmed_source_path.to_string(),
                        error,
                    });
                    continue;
                }
            };

        let dedupe_key = session_dedupe_key(&entry.context, &session.source_path);
        if !seen_paths.insert(dedupe_key) {
            continue;
        }

        let result = (|| -> Result<String, String> {
            let file_name = build_unique_export_file_name(
                &session,
                &tool,
                exported_items.len() + 1,
                &mut used_file_names,
                export_file_extension(&tool, &export_format)?,
            );
            let export_path = export_dir_ref.join(file_name);
            export_session_to_path(
                &entry.context,
                &tool,
                &session,
                &export_format,
                &export_path,
            )?;
            Ok(export_path.to_string_lossy().to_string())
        })();

        match result {
            Ok(export_path) => exported_items.push(ExportSessionItem {
                source_path: session.source_path.clone(),
                export_path,
            }),
            Err(error) => failed_items.push(ExportSessionFailure {
                source_path: trimmed_source_path.to_string(),
                error,
            }),
        }
    }

    Ok(ExportToolSessionsResult {
        exported_count: exported_items.len(),
        exported_items,
        failed_items,
    })
}

fn build_exported_session_file(
    context: &ToolSessionContext,
    tool: String,
    session_detail: SessionDetail,
) -> Result<ExportedSessionFile, String> {
    let native_snapshot = build_native_snapshot(
        &session_detail.meta.source_path,
        &session_detail.meta,
        &session_detail.messages,
        context,
    )?;
    Ok(ExportedSessionFile {
        version: EXPORT_SCHEMA_VERSION,
        schema: EXPORT_SCHEMA_NAME.to_string(),
        tool,
        exported_at: Utc::now().to_rfc3339(),
        meta: session_detail.meta,
        normalized_messages: session_detail.messages,
        native_snapshot,
    })
}

fn export_file_extension<'a>(tool: &str, export_format: &'a str) -> Result<&'a str, String> {
    match export_format {
        "ai_toolbox" => Ok("json"),
        "grok_markdown" if tool == "grok" => Ok("md"),
        "grok_native" if tool == "grok" => Ok("json"),
        "grok_markdown" | "grok_native" => {
            Err("Grok-specific export formats are only available for Grok sessions".to_string())
        }
        _ => Err(format!(
            "Unsupported session export format: {export_format}"
        )),
    }
}

fn export_session_to_path(
    context: &ToolSessionContext,
    tool: &str,
    meta: &SessionMeta,
    export_format: &str,
    export_path: &Path,
) -> Result<(), String> {
    export_file_extension(tool, export_format)?;
    match export_format {
        "ai_toolbox" => {
            let messages = load_messages(context, &meta.source_path)?;
            let session_detail = SessionDetail {
                meta: meta.clone(),
                messages,
            };
            let exported_file =
                build_exported_session_file(context, tool.to_string(), session_detail)?;
            write_exported_session_file(&exported_file, export_path)
        }
        "grok_markdown" => match context {
            ToolSessionContext::Grok { sessions_root } => {
                grok::export_markdown(sessions_root, &meta.session_id, export_path)
            }
            _ => Err("Grok Markdown export requires a Grok session context".to_string()),
        },
        "grok_native" => match context {
            ToolSessionContext::Grok { sessions_root } => {
                let payload =
                    grok::export_native_snapshot(sessions_root, Path::new(&meta.source_path))?;
                let exported = json!({
                    "schema": GROK_NATIVE_EXPORT_SCHEMA,
                    "tool": "grok",
                    "sessionId": meta.session_id,
                    "exportedAt": Utc::now().to_rfc3339(),
                    "nativeSnapshot": {
                        "format": SNAPSHOT_FORMAT_GROK,
                        "payload": payload,
                    }
                });
                write_json_value_to_path(&exported, export_path)
            }
            _ => Err("Grok native export requires a Grok session context".to_string()),
        },
        _ => Err(format!(
            "Unsupported session export format: {export_format}"
        )),
    }
}

fn write_json_value_to_path(value: &Value, path: &Path) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| format!("Failed to serialize session export: {error}"))?;
    if let Some(parent_dir) = path.parent() {
        std::fs::create_dir_all(parent_dir).map_err(|error| {
            format!(
                "Failed to create export directory {}: {error}",
                parent_dir.display()
            )
        })?;
    }
    std::fs::write(path, serialized).map_err(|error| {
        format!(
            "Failed to write exported session file {}: {error}",
            path.display()
        )
    })
}

fn write_exported_session_file(
    exported_file: &ExportedSessionFile,
    export_path_ref: &Path,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(&exported_file)
        .map_err(|error| format!("Failed to serialize session export: {error}"))?;

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

fn build_unique_export_file_name(
    meta: &SessionMeta,
    tool: &str,
    index: usize,
    used_file_names: &mut HashSet<String>,
    extension: &str,
) -> String {
    let title = meta
        .title
        .as_deref()
        .or(meta.summary.as_deref())
        .map(sanitize_export_file_component)
        .filter(|value| !value.is_empty());
    let session_id = sanitize_export_file_component(&meta.session_id);
    let base_name = match title {
        Some(title) => format!("{index:03}-{tool}-{title}-{session_id}"),
        None => format!("{index:03}-{tool}-{session_id}"),
    };
    let mut file_name = format!("{base_name}.{extension}");
    let mut suffix = 2usize;
    while !used_file_names.insert(file_name.to_ascii_lowercase()) {
        file_name = format!("{base_name}-{suffix}.{extension}");
        suffix += 1;
    }
    file_name
}

fn sanitize_export_file_component(value: &str) -> String {
    let mut sanitized = String::new();
    let mut last_was_separator = false;

    for character in value.chars() {
        let next = if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            last_was_separator = false;
            Some(character)
        } else if character.is_whitespace()
            || matches!(
                character,
                '.' | '/'
                    | '\\'
                    | ':'
                    | '*'
                    | '?'
                    | '"'
                    | '<'
                    | '>'
                    | '|'
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '{'
                    | '}'
            )
        {
            if last_was_separator {
                None
            } else {
                last_was_separator = true;
                Some('-')
            }
        } else if character.is_alphanumeric() {
            last_was_separator = false;
            Some(character)
        } else if last_was_separator {
            None
        } else {
            last_was_separator = true;
            Some('-')
        };

        if let Some(character) = next {
            sanitized.push(character);
        }

        if sanitized.len() >= 80 {
            break;
        }
    }

    sanitized.trim_matches('-').to_string()
}

fn import_session_blocking(
    context: ToolSessionContext,
    tool: String,
    import_path: String,
) -> Result<(), String> {
    if tool == "grok" && try_import_grok_native_snapshot(&context, &import_path)? {
        invalidate_cache(&context);
        return Ok(());
    }
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
        ToolSessionContext::GeminiCli { tmp_root } => {
            ensure_snapshot_format(&exported_file.native_snapshot, SNAPSHOT_FORMAT_GEMINI_CLI)?;
            gemini_cli::import_native_snapshot(
                tmp_root,
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
        ToolSessionContext::Pi { sessions_root } => {
            ensure_snapshot_format(&exported_file.native_snapshot, SNAPSHOT_FORMAT_PI)?;
            pi::import_native_snapshot(
                sessions_root,
                &exported_file.meta.session_id,
                &exported_file.native_snapshot.payload,
            )?;
        }
        ToolSessionContext::Grok { sessions_root } => {
            ensure_snapshot_format(&exported_file.native_snapshot, SNAPSHOT_FORMAT_GROK)?;
            grok::import_native_snapshot(
                sessions_root,
                &exported_file.meta.session_id,
                &exported_file.native_snapshot.payload,
            )?;
        }
    }

    invalidate_cache(&context);
    Ok(())
}

fn try_import_grok_native_snapshot(
    context: &ToolSessionContext,
    import_path: &str,
) -> Result<bool, String> {
    let data = std::fs::read_to_string(import_path)
        .map_err(|error| format!("Failed to read imported session file {import_path}: {error}"))?;
    let value: Value = serde_json::from_str(&data)
        .map_err(|error| format!("Invalid session export file {import_path}: {error}"))?;
    if value.get("schema").and_then(Value::as_str) != Some(GROK_NATIVE_EXPORT_SCHEMA) {
        return Ok(false);
    }
    let ToolSessionContext::Grok { sessions_root } = context else {
        return Err("Grok native snapshot requires a Grok session context".to_string());
    };
    if value.get("tool").and_then(Value::as_str) != Some("grok") {
        return Err("Invalid Grok native snapshot tool".to_string());
    }
    let session_id = value
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Grok native snapshot is missing sessionId".to_string())?;
    let native_snapshot = value
        .get("nativeSnapshot")
        .ok_or_else(|| "Grok native snapshot payload is missing".to_string())?;
    if native_snapshot.get("format").and_then(Value::as_str) != Some(SNAPSHOT_FORMAT_GROK) {
        return Err("Unexpected Grok native snapshot format".to_string());
    }
    let payload = native_snapshot
        .get("payload")
        .ok_or_else(|| "Grok native snapshot payload is missing".to_string())?;
    grok::import_native_snapshot(sessions_root, session_id, payload)?;
    Ok(true)
}

fn rename_session_blocking(
    contexts: SessionContextSet,
    _tool: String,
    source_path: String,
    title: String,
) -> Result<(), String> {
    let (entry, session) = find_session_with_context(&contexts, &source_path, true)?;
    let context = entry.context;
    match &context {
        ToolSessionContext::Codex { .. } => {
            codex::rename_session(&session.source_path, &title)?;
            invalidate_cache(&context);
            Ok(())
        }
        ToolSessionContext::OpenCode { .. } => {
            open_code::rename_session(&session.source_path, &title)?;
            invalidate_cache(&context);
            Ok(())
        }
        ToolSessionContext::Pi { .. } => {
            pi::rename_session(&session.source_path, &title)?;
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
        ToolSessionContext::GeminiCli { tmp_root } => Ok(NativeSnapshot {
            format: SNAPSHOT_FORMAT_GEMINI_CLI.to_string(),
            payload: gemini_cli::export_native_snapshot(tmp_root, Path::new(source_path))?,
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
        ToolSessionContext::Pi { sessions_root } => Ok(NativeSnapshot {
            format: SNAPSHOT_FORMAT_PI.to_string(),
            payload: pi::export_native_snapshot(sessions_root, Path::new(source_path))?,
        }),
        ToolSessionContext::Grok { sessions_root } => Ok(NativeSnapshot {
            format: SNAPSHOT_FORMAT_GROK.to_string(),
            payload: grok::export_native_snapshot(sessions_root, Path::new(source_path))?,
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
        ToolSessionContext::GeminiCli { tmp_root } => gemini_cli::scan_sessions(tmp_root),
        ToolSessionContext::OpenClaw { agents_root } => open_claw::scan_sessions(agents_root),
        ToolSessionContext::OpenCode {
            data_root,
            sqlite_db_path,
            ..
        } => open_code::scan_sessions(data_root, sqlite_db_path),
        ToolSessionContext::Pi { sessions_root } => pi::scan_sessions(sessions_root),
        ToolSessionContext::Grok { sessions_root } => grok::scan_sessions(sessions_root),
    };

    sessions.sort_by(|left, right| {
        let left_ts = left.last_active_at.or(left.created_at).unwrap_or(0);
        let right_ts = right.last_active_at.or(right.created_at).unwrap_or(0);
        right_ts.cmp(&left_ts)
    });
    sessions
}

fn scan_recent_sessions(context: &ToolSessionContext, limit: usize) -> Vec<SessionMeta> {
    let mut sessions = match context {
        ToolSessionContext::Codex { sessions_root } => {
            codex::scan_recent_sessions(sessions_root, limit)
        }
        ToolSessionContext::ClaudeCode { projects_root } => {
            claude_code::scan_recent_sessions(projects_root, limit)
        }
        ToolSessionContext::GeminiCli { tmp_root } => {
            gemini_cli::scan_recent_sessions(tmp_root, limit)
        }
        ToolSessionContext::OpenClaw { agents_root } => {
            open_claw::scan_recent_sessions(agents_root, limit)
        }
        ToolSessionContext::OpenCode {
            data_root,
            sqlite_db_path,
            ..
        } => open_code::scan_recent_sessions(data_root, sqlite_db_path, limit),
        ToolSessionContext::Pi { sessions_root } => pi::scan_recent_sessions(sessions_root, limit),
        ToolSessionContext::Grok { sessions_root } => {
            grok::scan_recent_sessions(sessions_root, limit)
        }
    };

    sessions.sort_by(|left, right| {
        let left_ts = left.last_active_at.or(left.created_at).unwrap_or(0);
        let right_ts = right.last_active_at.or(right.created_at).unwrap_or(0);
        right_ts.cmp(&left_ts)
    });
    sessions.truncate(limit);
    sessions
}

fn load_messages(
    context: &ToolSessionContext,
    source_path: &str,
) -> Result<Vec<SessionMessage>, String> {
    match context {
        ToolSessionContext::Codex { .. } => codex::load_messages(Path::new(source_path)),
        ToolSessionContext::ClaudeCode { .. } => claude_code::load_messages(Path::new(source_path)),
        ToolSessionContext::GeminiCli { .. } => gemini_cli::load_messages(Path::new(source_path)),
        ToolSessionContext::OpenClaw { .. } => open_claw::load_messages(Path::new(source_path)),
        ToolSessionContext::OpenCode { .. } => open_code::load_messages(source_path),
        ToolSessionContext::Pi { .. } => pi::load_messages(Path::new(source_path)),
        ToolSessionContext::Grok { .. } => grok::load_messages(Path::new(source_path)),
    }
}

fn list_subagent_sessions(
    context: &ToolSessionContext,
    source_path: &str,
) -> Vec<SessionSubagentMeta> {
    match context {
        ToolSessionContext::ClaudeCode { .. } => {
            claude_code::list_subagent_sessions(Path::new(source_path))
        }
        ToolSessionContext::GeminiCli { .. } => {
            gemini_cli::list_subagent_sessions(Path::new(source_path))
        }
        ToolSessionContext::Codex { .. }
        | ToolSessionContext::OpenClaw { .. }
        | ToolSessionContext::OpenCode { .. }
        | ToolSessionContext::Pi { .. }
        | ToolSessionContext::Grok { .. } => Vec::new(),
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

fn get_fresh_cached_sessions(context: &ToolSessionContext) -> Option<Vec<SessionMeta>> {
    let cache_key = context.cache_key();

    let Ok(cache) = SESSION_LIST_CACHE.lock() else {
        return None;
    };

    if let Some(entry) = cache.get(&cache_key) {
        if entry.created_at.elapsed() <= SESSION_CACHE_TTL {
            return Some(entry.sessions.clone());
        }
    }

    None
}

fn get_any_cached_sessions(
    context: &ToolSessionContext,
) -> Option<(Vec<SessionMeta>, SessionListCacheState)> {
    let cache_key = context.cache_key();

    let Ok(cache) = SESSION_LIST_CACHE.lock() else {
        return None;
    };

    let entry = cache.get(&cache_key)?;
    let cache_state = if entry.created_at.elapsed() <= SESSION_CACHE_TTL {
        SessionListCacheState::Fresh
    } else {
        SessionListCacheState::Stale
    };

    Some((entry.sessions.clone(), cache_state))
}

fn invalidate_cache(context: &ToolSessionContext) {
    let cache_key = context.cache_key();
    if let Ok(mut cache) = SESSION_LIST_CACHE.lock() {
        cache.remove(&cache_key);
    }
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
        ToolSessionContext::GeminiCli { .. } => {
            gemini_cli::scan_messages_for_query(Path::new(source_path), query_lower)
        }
        ToolSessionContext::OpenClaw { .. } => {
            open_claw::scan_messages_for_query(Path::new(source_path), query_lower)
        }
        ToolSessionContext::OpenCode { .. } => {
            open_code::scan_messages_for_query(source_path, query_lower)
        }
        ToolSessionContext::Pi { .. } => {
            pi::scan_messages_for_query(Path::new(source_path), query_lower)
        }
        ToolSessionContext::Grok { .. } => {
            grok::scan_messages_for_query(Path::new(source_path), query_lower)
        }
    }
}

fn meta_matches_query(session: &SessionMeta, query_lower: &str) -> bool {
    contains_query(&session.session_id, query_lower)
        || contains_query(&session.source_path, query_lower)
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
        || session
            .runtime_source
            .as_deref()
            .map(|value| contains_query(value, query_lower))
            .unwrap_or(false)
        || session
            .runtime_distro
            .as_deref()
            .map(|value| contains_query(value, query_lower))
            .unwrap_or(false)
}

fn session_id_exact_matches_query(session: &SessionMeta, query_lower: &str) -> bool {
    session.session_id.to_lowercase() == query_lower
}

fn contains_query(value: &str, query_lower: &str) -> bool {
    value.to_lowercase().contains(query_lower)
}

fn normalize_query(query: Option<String>) -> Option<String> {
    query
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn resolve_session_contexts(
    db: &crate::db::SqliteDbState,
    tool: SessionTool,
) -> Result<SessionContextSet, String> {
    let primary_context = resolve_context(db, tool).await?;
    let primary_entry = session_context_entry(primary_context);

    let mut entries = vec![primary_entry.clone()];

    if primary_entry.source == SessionRuntimeSource::Local {
        if let Some(wsl_entry) = resolve_wsl_sync_session_context(db, tool).await {
            entries.push(wsl_entry);
        }
    }

    Ok(session_context_set(entries))
}

fn session_context_entry(context: ToolSessionContext) -> SessionContextEntry {
    if let Some(wsl) = context_wsl_info(&context) {
        return SessionContextEntry {
            context,
            source: SessionRuntimeSource::Wsl,
            distro: Some(wsl.distro),
        };
    }

    SessionContextEntry {
        context,
        source: SessionRuntimeSource::Local,
        distro: None,
    }
}

fn context_wsl_info(context: &ToolSessionContext) -> Option<WslLocationInfo> {
    match context {
        ToolSessionContext::Codex { sessions_root } => path_wsl_info(sessions_root),
        ToolSessionContext::ClaudeCode { projects_root } => path_wsl_info(projects_root),
        ToolSessionContext::GeminiCli { tmp_root } => path_wsl_info(tmp_root),
        ToolSessionContext::OpenClaw { agents_root } => path_wsl_info(agents_root),
        ToolSessionContext::OpenCode {
            runtime_location, ..
        } => runtime_location
            .wsl
            .clone()
            .or_else(|| path_wsl_info(&runtime_location.host_path)),
        ToolSessionContext::Pi { sessions_root } => path_wsl_info(sessions_root),
        ToolSessionContext::Grok { sessions_root } => path_wsl_info(sessions_root),
    }
}

fn path_wsl_info(path: &Path) -> Option<WslLocationInfo> {
    path.to_str()
        .and_then(crate::coding::runtime_location::parse_wsl_unc_path)
}

async fn resolve_wsl_sync_session_context(
    db: &crate::db::SqliteDbState,
    tool: SessionTool,
) -> Option<SessionContextEntry> {
    let distro = enabled_wsl_distro(db)?;
    let linux_home = crate::coding::wsl::get_wsl_user_home(&distro).ok()?;
    let context = build_default_wsl_session_context(tool, &distro, &linux_home)?;

    Some(SessionContextEntry {
        context,
        source: SessionRuntimeSource::Wsl,
        distro: Some(distro),
    })
}

fn enabled_wsl_distro(db: &crate::db::SqliteDbState) -> Option<String> {
    let config = db
        .with_conn(|conn| db_get(conn, DbTable::WslSyncConfig, "config"))
        .ok()??;

    if config.get("enabled").and_then(Value::as_bool) != Some(true) {
        return None;
    }

    let configured_distro = config
        .get("distro")
        .and_then(Value::as_str)
        .unwrap_or_default();

    crate::coding::wsl::get_effective_distro(configured_distro).ok()
}

fn build_default_wsl_session_context(
    tool: SessionTool,
    distro: &str,
    linux_home: &str,
) -> Option<ToolSessionContext> {
    let linux_user_root = Some(linux_home.to_string());

    match tool {
        SessionTool::Codex => Some(ToolSessionContext::Codex {
            sessions_root: wsl_home_path(distro, linux_home, ".codex/sessions"),
        }),
        SessionTool::ClaudeCode => Some(ToolSessionContext::ClaudeCode {
            projects_root: wsl_home_path(distro, linux_home, ".claude/projects"),
        }),
        SessionTool::GeminiCli => Some(ToolSessionContext::GeminiCli {
            tmp_root: wsl_home_path(distro, linux_home, ".gemini/tmp"),
        }),
        SessionTool::OpenClaw => Some(ToolSessionContext::OpenClaw {
            agents_root: wsl_home_path(distro, linux_home, ".openclaw/agents"),
        }),
        SessionTool::OpenCode => {
            let config_linux_path = linux_join(linux_home, ".config/opencode/opencode.jsonc");
            let data_linux_path = linux_join(linux_home, ".local/share/opencode");
            let state_linux_path = linux_join(linux_home, ".local/state/opencode");
            let config_path = build_windows_unc_path(distro, &config_linux_path);
            let data_root = build_windows_unc_path(distro, &data_linux_path);
            let state_root = build_windows_unc_path(distro, &state_linux_path);
            Some(ToolSessionContext::OpenCode {
                runtime_location: RuntimeLocationInfo {
                    mode: RuntimeLocationMode::WslDirect,
                    source: "wsl_sync".to_string(),
                    host_path: config_path.clone(),
                    wsl: Some(WslLocationInfo {
                        distro: distro.to_string(),
                        linux_path: config_linux_path,
                        linux_user_root,
                    }),
                },
                config_path,
                sqlite_db_path: data_root.join("opencode.db"),
                data_root,
                state_root,
            })
        }
        SessionTool::Pi => Some(ToolSessionContext::Pi {
            sessions_root: wsl_home_path(distro, linux_home, ".pi/agent/sessions"),
        }),
        SessionTool::Grok => Some(ToolSessionContext::Grok {
            sessions_root: wsl_home_path(distro, linux_home, ".grok/sessions"),
        }),
    }
}

fn linux_join(root: &str, suffix: &str) -> String {
    format!(
        "{}/{}",
        root.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

fn wsl_home_path(distro: &str, linux_home: &str, suffix: &str) -> PathBuf {
    build_windows_unc_path(distro, &linux_join(linux_home, suffix))
}

fn build_available_sources(entries: &[SessionContextEntry]) -> Vec<SessionSourceOption> {
    let mut sources = Vec::new();
    let mut seen = HashSet::new();

    for source in [SessionRuntimeSource::Local, SessionRuntimeSource::Wsl] {
        let Some(entry) = entries.iter().find(|entry| entry.source == source) else {
            continue;
        };

        if seen.insert(source.as_str()) {
            sources.push(SessionSourceOption {
                source: source.as_str().to_string(),
                distro: entry.distro.clone(),
            });
        }
    }

    sources
}

fn session_context_set(entries: Vec<SessionContextEntry>) -> SessionContextSet {
    let available_sources = build_available_sources(&entries);
    SessionContextSet {
        entries,
        available_sources,
    }
}

#[cfg(test)]
fn single_context_set(context: ToolSessionContext) -> SessionContextSet {
    session_context_set(vec![session_context_entry(context)])
}

async fn resolve_context(
    db: &crate::db::SqliteDbState,
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
        SessionTool::GeminiCli => {
            let runtime_location = get_gemini_cli_runtime_location_async(db).await?;
            Ok(ToolSessionContext::GeminiCli {
                tmp_root: runtime_location.host_path.join("tmp"),
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
        SessionTool::Pi => {
            let runtime_location = get_pi_runtime_location_async(db).await?;
            let sessions_root = resolve_pi_sessions_root(&runtime_location)?;
            Ok(ToolSessionContext::Pi { sessions_root })
        }
        SessionTool::Grok => {
            let runtime_location = get_grok_runtime_location_async(db).await?;
            Ok(ToolSessionContext::Grok {
                sessions_root: runtime_location.host_path.join("sessions"),
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

fn resolve_pi_sessions_root(location: &RuntimeLocationInfo) -> Result<PathBuf, String> {
    const SESSION_DIR_ENV_KEY: &str = "PI_CODING_AGENT_SESSION_DIR";

    if let Ok(session_dir) = std::env::var(SESSION_DIR_ENV_KEY) {
        if !session_dir.trim().is_empty() {
            return resolve_pi_session_dir_value(location, session_dir.trim());
        }
    }

    let settings_path = location.host_path.join("settings.json");
    if let Ok(content) = std::fs::read_to_string(&settings_path) {
        if !content.trim().is_empty() {
            let settings: Value = serde_json::from_str(&content).map_err(|error| {
                format!(
                    "Failed to parse Pi settings for sessionDir {}: {error}",
                    settings_path.display()
                )
            })?;
            if let Some(session_dir) = settings
                .get("sessionDir")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return resolve_pi_session_dir_value(location, session_dir);
            }
        }
    }

    Ok(location.host_path.join("sessions"))
}

fn resolve_pi_session_dir_value(
    location: &RuntimeLocationInfo,
    session_dir: &str,
) -> Result<PathBuf, String> {
    if let Some(wsl) = &location.wsl {
        if is_windows_style_path(session_dir) {
            return Err(format!(
                "Pi sessionDir '{}' is a Windows-style path but the current Pi runtime is WSL Direct. Use a Linux path such as ~/.pi/agent/sessions or /home/<user>/sessions.",
                session_dir
            ));
        }

        let linux_session_dir = session_dir.replace('\\', "/");
        if linux_session_dir == "~" || linux_session_dir.starts_with("~/") {
            let linux_path =
                expand_home_from_user_root(wsl.linux_user_root.as_deref(), &linux_session_dir);
            return Ok(build_windows_unc_path(&wsl.distro, &linux_path));
        }
        if linux_session_dir.starts_with('/') {
            return Ok(build_windows_unc_path(&wsl.distro, &linux_session_dir));
        }

        let linux_path = format!(
            "{}/{}",
            wsl.linux_path.trim_end_matches('/'),
            linux_session_dir.trim_start_matches('/')
        );
        return Ok(build_windows_unc_path(&wsl.distro, &linux_path));
    }

    if session_dir == "~" || session_dir.starts_with("~/") || session_dir.starts_with("~\\") {
        let home = get_home_dir()?;
        let rest = session_dir
            .trim_start_matches('~')
            .trim_start_matches(['/', '\\']);
        return Ok(if rest.is_empty() {
            home
        } else {
            home.join(rest)
        });
    }

    let path = PathBuf::from(session_dir);
    if path.is_absolute() {
        return Ok(path);
    }

    Ok(location.host_path.join(path))
}

fn is_windows_style_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'))
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

    fn pi_wsl_location() -> RuntimeLocationInfo {
        RuntimeLocationInfo {
            mode: crate::coding::runtime_location::RuntimeLocationMode::WslDirect,
            source: "test".to_string(),
            host_path: PathBuf::from(r"\\wsl.localhost\Ubuntu\home\tester\.pi\agent"),
            wsl: Some(crate::coding::runtime_location::WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/tester/.pi/agent".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        }
    }

    #[test]
    fn resolve_pi_session_dir_value_wsl_expands_backslash_tilde() {
        let resolved =
            resolve_pi_session_dir_value(&pi_wsl_location(), r"~\sessions").expect("resolve");

        assert_eq!(
            resolved.to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\sessions"
        );
    }

    #[test]
    fn resolve_pi_session_dir_value_wsl_rejects_windows_drive_path() {
        let error = resolve_pi_session_dir_value(&pi_wsl_location(), r"D:\sessions")
            .expect_err("windows path should be rejected in WSL Direct");

        assert!(error.contains("Windows-style path"));
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

    fn normalize_test_path(value: &str) -> String {
        value.replace('\\', "/")
    }

    fn assert_project_dir_eq(actual: Option<&str>, expected: &Path) {
        let actual = actual.map(normalize_test_path);
        let expected = normalize_test_path(&expected.to_string_lossy());
        assert_eq!(actual.as_deref(), Some(expected.as_str()));
    }

    fn normalize_opencode_official_export_defaults(value: &mut Value) {
        let Some(info) = value.get_mut("info").and_then(Value::as_object_mut) else {
            return;
        };

        if info.get("cost") == Some(&json!(0)) {
            info.remove("cost");
        }

        let default_tokens = json!({
            "input": 0,
            "output": 0,
            "reasoning": 0,
            "cache": {
                "read": 0,
                "write": 0
            }
        });
        if info.get("tokens") == Some(&default_tokens) {
            info.remove("tokens");
        }

        if let Some(path) = info.get("path").and_then(Value::as_str) {
            let normalized_path = normalize_test_path(path);
            let normalized_path = normalized_path
                .find("ai-toolbox-session-manager-")
                .map(|marker_index| normalized_path[marker_index..].to_string())
                .unwrap_or(normalized_path);
            info.insert("path".to_string(), Value::String(normalized_path));
        }
    }

    #[test]
    fn query_filter_short_circuits_exact_session_id_before_content_scan() {
        let test_root = TestDir::new("exact-session-id-query");
        let exact_session_id = "exact-session-id";
        let content_match_path = test_root.path().join("content-match.jsonl");
        fs::write(
            &content_match_path,
            json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": exact_session_id
                        }
                    ]
                }
            })
            .to_string(),
        )
        .expect("write content match session");

        let contexts = SessionContextSet {
            entries: vec![SessionContextEntry {
                context: ToolSessionContext::Codex {
                    sessions_root: test_root.path().to_path_buf(),
                },
                source: SessionRuntimeSource::Local,
                distro: None,
            }],
            available_sources: Vec::new(),
        };
        let sessions = vec![
            SessionWithContext {
                context_index: 0,
                meta: SessionMeta {
                    provider_id: "codex".to_string(),
                    session_id: exact_session_id.to_string(),
                    title: None,
                    summary: None,
                    project_dir: None,
                    created_at: None,
                    last_active_at: None,
                    source_path: test_root
                        .path()
                        .join("missing-exact-session.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    resume_command: None,
                    runtime_source: None,
                    runtime_distro: None,
                },
            },
            SessionWithContext {
                context_index: 0,
                meta: SessionMeta {
                    provider_id: "codex".to_string(),
                    session_id: "content-match-session".to_string(),
                    title: None,
                    summary: None,
                    project_dir: None,
                    created_at: None,
                    last_active_at: None,
                    source_path: content_match_path.to_string_lossy().to_string(),
                    resume_command: None,
                    runtime_source: None,
                    runtime_distro: None,
                },
            },
        ];

        let (filtered, exact_session_id_match) =
            filter_sessions_by_query_with_context(&contexts, sessions, exact_session_id, true);

        assert!(exact_session_id_match);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].meta.session_id, exact_session_id);
    }

    #[test]
    fn cache_first_skips_uncached_wsl_context_for_initial_page() {
        let test_root = TestDir::new("cache-first-skips-uncached-wsl");
        let local_root = test_root.path().join("local").join("sessions");
        let wsl_root = test_root.path().join("wsl").join("sessions");
        let local_project_dir = test_root.path().join("local-project");
        let wsl_project_dir = test_root.path().join("wsl-project");

        write_text_file(
            &local_root
                .join("2026")
                .join("07")
                .join("04")
                .join("rollout-2026-07-04T08-00-00-local-session.jsonl"),
            &json!({
                "timestamp": "2026-07-04T08:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "local-session",
                    "timestamp": "2026-07-04T08:00:00Z",
                    "cwd": local_project_dir.to_string_lossy().to_string(),
                }
            })
            .to_string(),
        );
        write_text_file(
            &wsl_root
                .join("2026")
                .join("07")
                .join("04")
                .join("rollout-2026-07-04T08-01-00-wsl-session.jsonl"),
            &json!({
                "timestamp": "2026-07-04T08:01:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "wsl-session",
                    "timestamp": "2026-07-04T08:01:00Z",
                    "cwd": wsl_project_dir.to_string_lossy().to_string(),
                }
            })
            .to_string(),
        );

        let contexts = SessionContextSet {
            entries: vec![
                SessionContextEntry {
                    context: ToolSessionContext::Codex {
                        sessions_root: local_root,
                    },
                    source: SessionRuntimeSource::Local,
                    distro: None,
                },
                SessionContextEntry {
                    context: ToolSessionContext::Codex {
                        sessions_root: wsl_root,
                    },
                    source: SessionRuntimeSource::Wsl,
                    distro: Some("Debian".to_string()),
                },
            ],
            available_sources: Vec::new(),
        };
        let full_contexts = contexts.clone();

        let result = list_sessions_blocking(
            contexts,
            SessionSourceMode::All,
            None,
            None,
            1,
            10,
            false,
            SessionListLoadMode::CacheFirst,
        )
        .expect("cache-first list should succeed");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].session_id, "local-session");
        assert_eq!(result.items[0].runtime_source.as_deref(), Some("local"));
        assert!(result.partial);
        assert_eq!(result.cache_state.as_deref(), Some("quick"));
        assert!(!result.has_more);
        assert!(!result.meta_complete);
        assert!(result.message_search_complete);

        let full_result = list_sessions_blocking(
            full_contexts,
            SessionSourceMode::All,
            None,
            None,
            1,
            10,
            false,
            SessionListLoadMode::Full,
        )
        .expect("full list should succeed");

        let full_session_ids: Vec<&str> = full_result
            .items
            .iter()
            .map(|session| session.session_id.as_str())
            .collect();
        assert_eq!(full_session_ids, vec!["wsl-session", "local-session"]);
        assert!(!full_result.partial);
        assert!(!full_result.has_more);
        assert!(full_result.meta_complete);
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
    fn grok_native_standalone_export_import_round_trip() {
        let test_root = TestDir::new("grok-native-round-trip");
        let source_root = test_root.path().join("source").join("sessions");
        let source_session = source_root.join("encoded-project").join("grok-session-1");
        fs::create_dir_all(source_session.join("subagents")).expect("create source session");
        fs::write(
            source_session.join("summary.json"),
            r#"{"info":{"id":"grok-session-1","cwd":"/workspace/demo"},"generated_title":"Grok native round trip"}"#,
        )
        .expect("write Grok summary");
        fs::write(
            source_session.join("chat_history.jsonl"),
            "{\"type\":\"user\",\"content\":\"hello\"}\n",
        )
        .expect("write Grok history");
        fs::write(source_session.join("subagents/state.bin"), [0_u8, 255, 10])
            .expect("write Grok binary state");

        let source_context = ToolSessionContext::Grok {
            sessions_root: source_root.clone(),
        };
        let meta = grok::scan_sessions(&source_root)
            .into_iter()
            .next()
            .expect("scan Grok session");
        let export_path = test_root.path().join("grok-native.json");
        export_session_to_path(&source_context, "grok", &meta, "grok_native", &export_path)
            .expect("export Grok native snapshot");
        let exported = read_json_file(&export_path);
        assert_eq!(
            exported.get("schema").and_then(Value::as_str),
            Some(GROK_NATIVE_EXPORT_SCHEMA)
        );

        let target_root = test_root.path().join("target").join("sessions");
        fs::create_dir_all(&target_root).expect("create target sessions root");
        import_session_blocking(
            ToolSessionContext::Grok {
                sessions_root: target_root.clone(),
            },
            "grok".to_string(),
            export_path.to_string_lossy().to_string(),
        )
        .expect("import Grok native snapshot");

        assert_eq!(
            fs::read(target_root.join("encoded-project/grok-session-1/subagents/state.bin"))
                .expect("read imported binary state"),
            [0_u8, 255, 10]
        );
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
    fn delete_sessions_blocking_returns_partial_result_and_removes_existing_codex_sessions() {
        let test_root = TestDir::new("codex-bulk-delete");
        let project_dir = test_root.path().join("codex-project");
        fs::create_dir_all(&project_dir).expect("failed to create codex project dir");

        let codex_home = test_root.path().join("codex-home");
        let sessions_root = codex_home.join("sessions");
        let existing_session_path = sessions_root
            .join("2026")
            .join("04")
            .join("21")
            .join("rollout-2026-04-21T10-00-00-session-a.jsonl");
        let another_session_path = sessions_root
            .join("2026")
            .join("04")
            .join("21")
            .join("rollout-2026-04-21T10-05-00-session-b.jsonl");

        write_text_file(
            &existing_session_path,
            &json!({
                "timestamp": "2026-04-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-a",
                    "timestamp": "2026-04-21T10:00:00Z",
                    "cwd": project_dir.to_string_lossy().to_string(),
                }
            })
            .to_string(),
        );
        write_text_file(
            &another_session_path,
            &json!({
                "timestamp": "2026-04-21T10:05:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-b",
                    "timestamp": "2026-04-21T10:05:00Z",
                    "cwd": project_dir.to_string_lossy().to_string(),
                }
            })
            .to_string(),
        );

        let context = ToolSessionContext::Codex {
            sessions_root: sessions_root.clone(),
        };
        let missing_session_path = sessions_root
            .join("2026")
            .join("04")
            .join("21")
            .join("rollout-2026-04-21T10-10-00-session-missing.jsonl");

        let result = delete_sessions_blocking(
            single_context_set(context),
            vec![
                existing_session_path.to_string_lossy().to_string(),
                missing_session_path.to_string_lossy().to_string(),
                another_session_path.to_string_lossy().to_string(),
            ],
        );

        assert_eq!(result.deleted_count, 2);
        assert_eq!(result.failed_items.len(), 1);
        assert_eq!(
            result.failed_items[0].source_path,
            missing_session_path.to_string_lossy()
        );
        assert!(result.failed_items[0].error.contains("Session not found"));
        assert!(!existing_session_path.exists());
        assert!(!another_session_path.exists());
    }

    #[test]
    fn export_sessions_blocking_exports_selected_codex_sessions_with_partial_result() {
        let test_root = TestDir::new("codex-bulk-export");
        let project_dir = test_root.path().join("codex-project");
        fs::create_dir_all(&project_dir).expect("failed to create codex project dir");

        let sessions_root = test_root.path().join("codex-home").join("sessions");
        let first_session_path = sessions_root
            .join("2026")
            .join("04")
            .join("22")
            .join("rollout-2026-04-22T10-00-00-session-a.jsonl");
        let second_session_path = sessions_root
            .join("2026")
            .join("04")
            .join("22")
            .join("rollout-2026-04-22T10-05-00-session-b.jsonl");

        write_text_file(
            &first_session_path,
            &json!({
                "timestamp": "2026-04-22T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-a",
                    "timestamp": "2026-04-22T10:00:00Z",
                    "cwd": project_dir.to_string_lossy().to_string(),
                }
            })
            .to_string(),
        );
        write_text_file(
            &second_session_path,
            &json!({
                "timestamp": "2026-04-22T10:05:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-b",
                    "timestamp": "2026-04-22T10:05:00Z",
                    "cwd": project_dir.to_string_lossy().to_string(),
                }
            })
            .to_string(),
        );

        let missing_session_path = sessions_root
            .join("2026")
            .join("04")
            .join("22")
            .join("rollout-2026-04-22T10-10-00-session-missing.jsonl");
        let export_dir = test_root.path().join("exports");
        let context = ToolSessionContext::Codex {
            sessions_root: sessions_root.clone(),
        };

        let result = export_sessions_blocking(
            single_context_set(context),
            "codex".to_string(),
            vec![
                first_session_path.to_string_lossy().to_string(),
                missing_session_path.to_string_lossy().to_string(),
                second_session_path.to_string_lossy().to_string(),
            ],
            export_dir.to_string_lossy().to_string(),
            "ai_toolbox".to_string(),
        )
        .expect("bulk export should complete with partial result");

        assert_eq!(result.exported_count, 2);
        assert_eq!(result.exported_items.len(), 2);
        assert_eq!(result.failed_items.len(), 1);
        assert_eq!(
            result.failed_items[0].source_path,
            missing_session_path.to_string_lossy()
        );
        assert!(result.failed_items[0].error.contains("Session not found"));

        for exported_item in result.exported_items {
            let exported_file = read_json_file(Path::new(&exported_item.export_path));
            assert_eq!(
                exported_file.get("schema"),
                Some(&Value::String(EXPORT_SCHEMA_NAME.to_string()))
            );
            assert_eq!(
                exported_file.get("version"),
                Some(&Value::Number(serde_json::Number::from(
                    EXPORT_SCHEMA_VERSION
                )))
            );
            assert_eq!(
                exported_file.get("tool"),
                Some(&Value::String("codex".to_string()))
            );
            assert_eq!(
                exported_file.pointer("/nativeSnapshot/format"),
                Some(&Value::String(SNAPSHOT_FORMAT_CODEX.to_string()))
            );
        }
    }

    #[test]
    fn delete_session_blocking_deletes_opencode_orphan_message_path_without_prescan() {
        let test_root = TestDir::new("opencode-direct-delete");
        let open_code_env = OpenCodeEnv::new(test_root.path(), "opencode-direct-delete-env");
        let data_root = open_code_env.data_root();
        let storage_root = data_root.join("storage");
        let session_id = "ses_direct_delete_orphan";
        let message_id = "msg_direct_delete_orphan";
        let message_dir = storage_root.join("message").join(session_id);
        let message_file = message_dir.join(format!("{message_id}.json"));
        let part_file = storage_root
            .join("part")
            .join(message_id)
            .join("prt_direct_delete_orphan.json");

        fs::create_dir_all(&message_dir).expect("failed to create opencode message dir");
        if let Some(parent) = part_file.parent() {
            fs::create_dir_all(parent).expect("failed to create opencode part dir");
        }

        write_text_file(
            &message_file,
            &format!(r#"{{"id":"{message_id}","role":"user","time":{{"created":1}}}}"#),
        );
        write_text_file(&part_file, r#"{"type":"text","text":"delete me"}"#);

        let context = ToolSessionContext::OpenCode {
            runtime_location: RuntimeLocationInfo {
                mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
                source: "test".to_string(),
                host_path: open_code_env
                    .xdg_config_home
                    .join("opencode")
                    .join("opencode.jsonc"),
                wsl: None,
            },
            config_path: open_code_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            data_root: data_root.clone(),
            state_root: open_code_env.xdg_state_home.join("opencode"),
            sqlite_db_path: open_code_env.sqlite_db_path(),
        };

        delete_session_blocking(
            single_context_set(context),
            message_dir.to_string_lossy().to_string(),
        )
        .expect("opencode direct delete should succeed without prescan");

        assert!(
            !message_dir.exists(),
            "opencode message directory should be removed"
        );
        assert!(!part_file.exists(), "opencode part file should be removed");
    }

    #[test]
    fn delete_session_blocking_treats_missing_opencode_session_as_idempotent_success() {
        let test_root = TestDir::new("opencode-delete-missing");
        let open_code_env = OpenCodeEnv::new(test_root.path(), "opencode-delete-missing-env");
        let missing_message_dir = open_code_env
            .data_root()
            .join("storage")
            .join("message")
            .join("ses_missing_delete_target");

        let context = ToolSessionContext::OpenCode {
            runtime_location: RuntimeLocationInfo {
                mode: crate::coding::runtime_location::RuntimeLocationMode::LocalWindows,
                source: "test".to_string(),
                host_path: open_code_env
                    .xdg_config_home
                    .join("opencode")
                    .join("opencode.jsonc"),
                wsl: None,
            },
            config_path: open_code_env
                .xdg_config_home
                .join("opencode")
                .join("opencode.jsonc"),
            data_root: open_code_env.data_root(),
            state_root: open_code_env.xdg_state_home.join("opencode"),
            sqlite_db_path: open_code_env.sqlite_db_path(),
        };

        delete_session_blocking(
            single_context_set(context),
            missing_message_dir.to_string_lossy().to_string(),
        )
        .expect("missing opencode delete should remain idempotent");
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
                runtime_source: None,
                runtime_distro: None,
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
            single_context_set(export_context),
            "codex".to_string(),
            source_path.to_string_lossy().to_string(),
            export_file.to_string_lossy().to_string(),
            "ai_toolbox".to_string(),
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
        assert_project_dir_eq(imported_session.project_dir.as_deref(), &project_dir);
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
            single_context_set(export_context),
            "claudecode".to_string(),
            source_path.to_string_lossy().to_string(),
            export_file.to_string_lossy().to_string(),
            "ai_toolbox".to_string(),
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
        assert_project_dir_eq(imported_session.project_dir.as_deref(), &project_dir);

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
        let _env_lock = crate::coding::test_env::lock();
        let session_id = "ses_1234567890abABCDEFGHIJKLMN";
        let message_id = "msg_1234567890abABCDEFGHIJKLMN";
        let part_id = "prt_1234567890abABCDEFGHIJKLMN";
        let project_dir = test_root.join("opencode-project");
        fs::create_dir_all(&project_dir).expect("failed to create opencode project dir");
        let opencode_project_path = project_dir
            .strip_prefix(std::env::temp_dir())
            .unwrap_or(&project_dir)
            .to_string_lossy()
            .replace('\\', "/");

        let official_export_path = test_root.join("opencode-official-export.json");
        let official_export_json = json!({
            "info": {
                "id": session_id,
                "slug": "opencode-round-trip",
                "projectID": "global",
                "directory": project_dir.to_string_lossy().to_string(),
                "path": opencode_project_path,
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
            single_context_set(export_context),
            "opencode".to_string(),
            source_session.source_path.clone(),
            export_file.to_string_lossy().to_string(),
            "ai_toolbox".to_string(),
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
        let mut exported_official_export_json: Value =
            serde_json::from_str(exported_official_export_raw)
                .expect("parse exported official export raw json");
        let mut expected_official_export_json = official_export_json.clone();
        normalize_opencode_official_export_defaults(&mut exported_official_export_json);
        normalize_opencode_official_export_defaults(&mut expected_official_export_json);
        assert_eq!(exported_official_export_json, expected_official_export_json);

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
        assert_project_dir_eq(imported_session.project_dir.as_deref(), &project_dir);

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
        let _env_lock = crate::coding::test_env::lock();
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
        assert_project_dir_eq(imported_session.project_dir.as_deref(), &project_dir);

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
        let _env_lock = crate::coding::test_env::lock();
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
        let _env_lock = crate::coding::test_env::lock();
        let test_root = TestDir::new("opencode-truncated-raw-assistant-first-import");
        let session_id = "ses_1234567890abASSISTANTFIRST01";
        let project_dir = test_root.path().join("opencode-project");
        fs::create_dir_all(&project_dir).expect("failed to create opencode project dir");

        let export_file = test_root
            .path()
            .join("opencode-truncated-raw-assistant-first.json");
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
        let _env_lock = crate::coding::test_env::lock();
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
                runtime_source: None,
                runtime_distro: None,
            },
            &[SessionMessage {
                role: "user".to_string(),
                content: "OpenCode explicit env export".to_string(),
                ts: Some(1710000000000_i64),
                id: None,
                parent_id: None,
                message_type: None,
                blocks: Vec::new(),
                model: None,
                usage: None,
                duration_ms: None,
                cost_usd: None,
                is_sidechain: None,
                metadata: None,
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
        let candidates: Vec<PathBuf> = program
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect();

        #[cfg(target_os = "windows")]
        {
            candidates.into_iter().find(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "exe" | "cmd" | "bat"
                        )
                    })
                    .unwrap_or(false)
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            candidates.into_iter().next()
        }
    }

    fn skip_when_opencode_cli_missing(test_name: &str) -> bool {
        if resolve_test_opencode_command().is_some() {
            return false;
        }

        eprintln!("skip {test_name}: OpenCode CLI `opencode` is not available in PATH");
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
