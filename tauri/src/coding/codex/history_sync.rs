use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use chrono::{TimeZone, Utc};
use regex::Regex;
use rusqlite::backup::Backup;
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::official_accounts::auth_has_official_runtime;
use uuid::Uuid;

const STATE_DB_PREFIX: &str = "state_";
const STATE_DB_SUFFIX: &str = ".sqlite";
const CONFIG_FILE_NAME: &str = "config.toml";
const AUTH_FILE_NAME: &str = "auth.json";
const OFFICIAL_MODEL_PROVIDER_ID: &str = "openai";
const AI_TOOLBOX_LEGACY_PROVIDER_ID: &str = "ai-toolbox-gateway";
const AI_TOOLBOX_LAUNCH_PROVIDER_PREFIX: &str = "aitb-launch-";
const CODEX_SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";
const CODEX_SQLITE_HOME_CONFIG_KEY: &str = "sqlite_home";
const SESSION_INDEX_FILE_NAME: &str = "session_index.jsonl";
const SESSIONS_DIR_NAME: &str = "sessions";
const ARCHIVED_SESSIONS_DIR_NAME: &str = "archived_sessions";
const BACKUP_DIR_NAME: &str = "history_sync_backups";
const ROLLOUT_COMPRESSED_SUFFIX: &str = ".jsonl.zst";
const ZSTD_COMPRESSION_LEVEL: i32 = 3;
const SQLITE_LOCK_RETRY_LIMIT: usize = 40;
const SQLITE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(250);
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(500);
const SQLITE_BACKUP_STEP_DELAY: Duration = Duration::from_millis(50);
const FILE_RETRY_LIMIT: usize = 20;
const FILE_RETRY_DELAY: Duration = Duration::from_millis(100);
const SQLITE_LOCKED_USER_MESSAGE: &str = "Codex is writing history (the Codex state database is locked). Finish the current Codex response/session or close Codex in WSL/VS Code, then try again.";

const PROVIDER_KIND_CURRENT: &str = "current";
const PROVIDER_KIND_AI_TOOLBOX_LEGACY: &str = "ai_toolbox_legacy";
const PROVIDER_KIND_OFFICIAL: &str = "official";
const PROVIDER_KIND_MISSING: &str = "missing";
const PROVIDER_KIND_OTHER: &str = "other";

/// Serializes every operation that rewrites Codex history.
///
/// `codex resume`, the unified-history toggle, and a manual history sync can be
/// triggered from different commands at the same time. They share rollout
/// files and the same state databases, so they must not interleave.
static HISTORY_OP_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn lock_history_op() -> MutexGuard<'static, ()> {
    HISTORY_OP_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistorySyncStatus {
    pub codex_home: String,
    pub config_path: String,
    pub db_path: String,
    pub sessions_dir: String,
    pub session_index_path: String,
    pub backup_dir: String,
    pub current_provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    pub total_threads: usize,
    pub provider_mismatch_threads: usize,
    #[serde(default)]
    pub provider_breakdown: Vec<CodexHistoryProviderGroup>,
    pub model_mismatch_threads: usize,
    pub model_column_exists: bool,
    pub session_file_count: usize,
    pub session_meta_mismatch_count: usize,
    pub indexed_threads: usize,
    pub missing_session_index_entries: usize,
    pub backup_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_backup_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_sources: Vec<CodexHistorySourceOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_distro: Option<String>,
    pub has_work: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistorySourceOption {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distro: Option<String>,
}

/// One `model_provider` bucket in the selected Codex history source.
///
/// `provider_id` is `None` for threads whose database row is NULL/empty and for
/// rollout files that never recorded one. `kind` is one of `current`,
/// `ai_toolbox_legacy`, `official`, `missing`, `other`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistoryProviderGroup {
    pub provider_id: Option<String>,
    pub kind: String,
    pub thread_count: usize,
    pub session_file_count: usize,
}

/// Background-only probe for legacy AI Toolbox provider sessions.
///
/// This is deliberately separate from [`get_status`]: the page calls it on
/// every activation, so it must not spend 40 × 250 ms retrying a lock.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistoryLegacyHint {
    pub detected: bool,
    pub thread_count: usize,
    pub session_file_count: usize,
    pub provider_ids: Vec<String>,
}

/// Which existing provider buckets the caller wants migrated.
///
/// Both fields being `None` preserves the pre-filter API behavior; any `Some`
/// value makes the operation strictly selection-based.
#[derive(Debug, Clone, Default)]
pub struct CodexHistorySyncSelection {
    pub source_providers: Option<Vec<String>>,
    pub include_missing_provider: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistoryBackupResult {
    pub backup_path: String,
    pub backup_dir: String,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistorySyncResult {
    pub status: CodexHistorySyncStatus,
    pub backup_path: String,
    pub updated_thread_rows: usize,
    pub updated_session_files: usize,
    pub failed_session_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_session_file_error: Option<String>,
    pub rewritten_index_entries: usize,
    pub missing_session_index_entries_before: usize,
    pub preserved_index_only_entries: usize,
    pub attempts: usize,
    pub lock_wait_ms: u128,
    pub duration_ms: u128,
    pub partial_success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistoryRestoreResult {
    pub restored_backup_path: String,
    pub safety_backup_path: String,
    pub restored_session_meta_files: usize,
    pub skipped_session_meta_files: usize,
    pub rewritten_index_entries: usize,
    pub attempts: usize,
    pub lock_wait_ms: u128,
    pub duration_ms: u128,
    pub status: CodexHistorySyncStatus,
}

#[derive(Debug, Clone)]
struct Paths {
    codex_home: PathBuf,
    config_path: PathBuf,
    db_path: PathBuf,
    sessions_dir: PathBuf,
    archived_sessions_dir: PathBuf,
    session_index_path: PathBuf,
    auth_path: PathBuf,
    backup_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct CurrentTarget {
    provider: String,
    model: Option<String>,
}

#[derive(Debug, Clone)]
struct SelectionPlan {
    providers: BTreeSet<String>,
    include_missing: bool,
}

#[derive(Debug, Clone)]
struct ThreadRow {
    id: String,
    title: Option<String>,
    updated_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    path: PathBuf,
    relative_path: String,
    model_provider: Option<String>,
    first_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionIndexEntry {
    id: String,
    thread_name: String,
    #[serde(default)]
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionMetaBackupEntry {
    relative_path: String,
    /// Kept for backups written before multi-line session metadata support.
    #[serde(default)]
    first_line: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    meta_lines: Vec<SessionMetaLineBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionMetaLineBackup {
    /// Zero-based line number in the decompressed rollout.
    line_index: usize,
    content: String,
}

#[derive(Debug, Clone)]
struct BackupOutcome {
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct RetryStats<T> {
    value: T,
    attempts: usize,
    lock_wait_ms: u128,
}

#[derive(Debug, Clone)]
struct IndexRebuildResult {
    rewritten_entries: usize,
    missing_entries_before: usize,
    preserved_index_only_entries: usize,
}

pub fn get_status(codex_home: &Path) -> Result<CodexHistorySyncStatus, String> {
    let paths = resolve_paths(codex_home)?;
    let target = read_current_target(&paths)?;
    // Codex may hold the state database while answering/autosaving (especially under WSL).
    // Retry read-side inspection so opening the history sync modal is not a hard fail.
    let db_snapshot = retry_sqlite_operation(|| read_status_db_snapshot(&paths, &target))?.value;
    let session_records = scan_session_records(&paths)?;
    let session_meta_mismatch_count = session_records
        .iter()
        .filter(|record| session_record_needs_sync(record, &target))
        .count();
    let provider_breakdown = build_provider_breakdown(
        &db_snapshot.provider_thread_counts,
        &session_records,
        &target,
    );
    let session_index_entries = read_session_index(&paths.session_index_path)?;
    let missing_session_index_entries = db_snapshot
        .visible_thread_ids
        .iter()
        .filter(|id| !session_index_entries.contains_key(*id))
        .count();
    let backups = list_backup_files(&paths.backup_dir)?;
    let latest_backup_path = backups
        .last()
        .map(|path| path.to_string_lossy().to_string());
    let has_work = db_snapshot.provider_mismatch_threads > 0
        || session_meta_mismatch_count > 0
        || missing_session_index_entries > 0;

    Ok(CodexHistorySyncStatus {
        codex_home: paths.codex_home.to_string_lossy().to_string(),
        config_path: paths.config_path.to_string_lossy().to_string(),
        db_path: paths.db_path.to_string_lossy().to_string(),
        sessions_dir: paths.sessions_dir.to_string_lossy().to_string(),
        session_index_path: paths.session_index_path.to_string_lossy().to_string(),
        backup_dir: paths.backup_dir.to_string_lossy().to_string(),
        current_provider: target.provider,
        current_model: target.model,
        total_threads: db_snapshot.total_threads,
        provider_mismatch_threads: db_snapshot.provider_mismatch_threads,
        provider_breakdown,
        // History sync intentionally preserves existing model values. Keep the field for
        // API compatibility, but treat model differences as informational only.
        model_mismatch_threads: 0,
        model_column_exists: db_snapshot.model_column_exists,
        session_file_count: session_records.len(),
        session_meta_mismatch_count,
        indexed_threads: session_index_entries.len(),
        missing_session_index_entries,
        backup_count: backups.len(),
        latest_backup_path,
        available_sources: Vec::new(),
        runtime_source: None,
        runtime_distro: None,
        has_work,
    })
}

/// Cheap legacy-provider probe used by the page badge.
///
/// One read attempt, one 500 ms busy timeout, no retry loop: a lock or a
/// transient I/O error degrades to "nothing detected" at the call site instead
/// of blocking the page.
/// A database on a Windows WSL UNC share cannot be locked through 9P: every
/// open waits out the busy timeout and then fails with "database is locked".
/// The background hint simply reports "nothing detected" instead of paying that
/// cost on each page activation.
fn is_windows_wsl_unc_path(path: &Path) -> bool {
    cfg!(windows)
        && path.to_str().is_some_and(|value| {
            crate::coding::runtime_location::parse_wsl_unc_path(value).is_some()
        })
}

fn empty_legacy_hint() -> CodexHistoryLegacyHint {
    CodexHistoryLegacyHint {
        detected: false,
        thread_count: 0,
        session_file_count: 0,
        provider_ids: Vec::new(),
    }
}

pub fn get_legacy_hint(codex_home: &Path) -> Result<CodexHistoryLegacyHint, String> {
    if is_windows_wsl_unc_path(codex_home) {
        return Ok(empty_legacy_hint());
    }
    let paths = resolve_paths(codex_home)?;
    if is_windows_wsl_unc_path(&paths.db_path) {
        return Ok(empty_legacy_hint());
    }
    let target = read_current_target(&paths)?;
    // State DB only. Scanning (and decompressing) every rollout on each page
    // activation would cost far more than the badge is worth.
    let provider_thread_counts = read_provider_thread_counts(&paths.db_path)?;

    let mut thread_count = 0;
    let mut provider_ids = BTreeSet::new();
    for (provider, count) in &provider_thread_counts {
        let Some(provider) = provider.as_deref() else {
            continue;
        };
        if provider == target.provider {
            continue;
        }
        if is_ai_toolbox_legacy_provider(provider) {
            thread_count += *count;
            provider_ids.insert(provider.to_string());
        }
    }

    Ok(CodexHistoryLegacyHint {
        detected: thread_count > 0,
        thread_count,
        session_file_count: 0,
        provider_ids: provider_ids.into_iter().collect(),
    })
}

pub fn backup(codex_home: &Path, label: &str) -> Result<CodexHistoryBackupResult, String> {
    let _guard = lock_history_op();
    let start = Instant::now();
    let paths = resolve_paths(codex_home)?;
    ensure_environment(&paths)?;
    let outcome = make_backup(&paths, label)?;
    Ok(CodexHistoryBackupResult {
        backup_path: outcome.path.to_string_lossy().to_string(),
        backup_dir: paths.backup_dir.to_string_lossy().to_string(),
        duration_ms: start.elapsed().as_millis(),
    })
}

pub fn sync(
    codex_home: &Path,
    selection: Option<&CodexHistorySyncSelection>,
) -> Result<CodexHistorySyncResult, String> {
    let _guard = lock_history_op();
    let start = Instant::now();
    let paths = resolve_paths(codex_home)?;
    let target = read_current_target(&paths)?;
    let selection_plan = build_selection_plan(selection, &target)?;
    ensure_environment(&paths)?;
    let backup = make_backup(&paths, "pre-sync")?;

    let db_sync =
        retry_sqlite_operation(|| sync_database(&paths.db_path, &target, selection_plan.as_ref()))?;
    let session_sync = sync_session_records(&paths, &target, selection_plan.as_ref())?;
    let index_result = retry_sqlite_operation(|| rebuild_session_index(&paths))?.value;
    let status = get_status(codex_home)?;

    Ok(CodexHistorySyncResult {
        status,
        backup_path: backup.path.to_string_lossy().to_string(),
        updated_thread_rows: db_sync.value,
        updated_session_files: session_sync.updated,
        failed_session_files: session_sync.failed,
        first_session_file_error: session_sync.first_error,
        rewritten_index_entries: index_result.rewritten_entries,
        missing_session_index_entries_before: index_result.missing_entries_before,
        preserved_index_only_entries: index_result.preserved_index_only_entries,
        attempts: db_sync.attempts,
        lock_wait_ms: db_sync.lock_wait_ms,
        duration_ms: start.elapsed().as_millis(),
        partial_success: session_sync.failed > 0,
    })
}

pub fn restore_latest(codex_home: &Path) -> Result<CodexHistoryRestoreResult, String> {
    let _guard = lock_history_op();
    let start = Instant::now();
    let paths = resolve_paths(codex_home)?;
    ensure_environment(&paths)?;
    let latest_backup = list_backup_files(&paths.backup_dir)?
        .last()
        .cloned()
        .ok_or_else(|| "No Codex history backup found".to_string())?;
    let safety_backup = make_backup(&paths, "pre-restore")?;
    let restore_db =
        retry_sqlite_operation(|| restore_database_from_backup(&paths.db_path, &latest_backup))?;
    let (restored_session_meta_files, skipped_session_meta_files) =
        restore_metadata_sidecars(&paths, &latest_backup)?;
    let index_result = retry_sqlite_operation(|| rebuild_session_index(&paths))?.value;
    let status = get_status(codex_home)?;

    Ok(CodexHistoryRestoreResult {
        restored_backup_path: latest_backup.to_string_lossy().to_string(),
        safety_backup_path: safety_backup.path.to_string_lossy().to_string(),
        restored_session_meta_files,
        skipped_session_meta_files,
        rewritten_index_entries: index_result.rewritten_entries,
        attempts: restore_db.attempts,
        lock_wait_ms: restore_db.lock_wait_ms,
        duration_ms: start.elapsed().as_millis(),
        status,
    })
}

fn resolve_paths(codex_home: &Path) -> Result<Paths, String> {
    let mut paths = base_paths(codex_home);
    paths.db_path = resolve_state_db_path(codex_home, &paths.config_path)?;
    Ok(paths)
}

fn base_paths(codex_home: &Path) -> Paths {
    let codex_home = codex_home.to_path_buf();
    Paths {
        config_path: codex_home.join(CONFIG_FILE_NAME),
        db_path: PathBuf::new(),
        sessions_dir: codex_home.join(SESSIONS_DIR_NAME),
        archived_sessions_dir: codex_home.join(ARCHIVED_SESSIONS_DIR_NAME),
        session_index_path: codex_home.join(SESSION_INDEX_FILE_NAME),
        auth_path: codex_home.join(AUTH_FILE_NAME),
        backup_dir: codex_home.join(BACKUP_DIR_NAME),
        codex_home,
    }
}

/// Locate Codex's state database.
///
/// Codex's file name carries a schema version (`state_<N>.sqlite`), so the whole
/// family is matched and the highest version wins. An explicitly configured
/// SQLite home (`config.toml`'s `sqlite_home`, then `CODEX_SQLITE_HOME`) takes
/// precedence over the Codex home itself, matching Codex's own `SqliteConfig`.
fn resolve_state_db_path(codex_home: &Path, config_path: &Path) -> Result<PathBuf, String> {
    let configured = sqlite_home_from_config(config_path).or_else(sqlite_home_from_env);
    let mut searched = Vec::new();
    for directory in configured
        .into_iter()
        .chain(std::iter::once(codex_home.to_path_buf()))
    {
        if searched.contains(&directory) {
            continue;
        }
        if let Some(path) = highest_state_db_in(&directory) {
            return Ok(path);
        }
        searched.push(directory);
    }
    Err(format!(
        "Could not find a Codex state database (state_<N>.sqlite) in: {}",
        searched
            .iter()
            .map(|directory| directory.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn highest_state_db_in(directory: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(directory).ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(version) = name
            .strip_prefix(STATE_DB_PREFIX)
            .and_then(|rest| rest.strip_suffix(STATE_DB_SUFFIX))
            .and_then(|version| version.parse::<u64>().ok())
        else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if best.as_ref().is_none_or(|(seen, _)| version > *seen) {
            best = Some((version, path));
        }
    }
    best.map(|(_, path)| path)
}

/// Every state database a Codex root can use, highest schema version each.
///
/// Unlike [`resolve_state_db_path`], this keeps both the Codex home database and
/// the configured SQLite-home database: unified history migrates every database
/// Codex might read. Directories without a matching file contribute nothing.
pub(crate) fn discover_state_db_paths(codex_home: &Path, config_text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = highest_state_db_in(codex_home) {
        push_unique_path(&mut paths, path);
    }
    let configured = sqlite_home_from_config_text(config_text).or_else(sqlite_home_from_env);
    if let Some(directory) = configured {
        if let Some(path) = highest_state_db_in(&directory) {
            push_unique_path(&mut paths, path);
        }
    }
    paths
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn sqlite_home_from_config(config_path: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(config_path).ok()?;
    sqlite_home_from_config_text(&text)
}

fn sqlite_home_from_config_text(config_text: &str) -> Option<PathBuf> {
    let value = config_text.parse::<toml::Value>().ok()?;
    let raw = value
        .get(CODEX_SQLITE_HOME_CONFIG_KEY)
        .and_then(toml::Value::as_str)?
        .trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

fn sqlite_home_from_env() -> Option<PathBuf> {
    let raw = std::env::var(CODEX_SQLITE_HOME_ENV).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

fn resolve_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~\\") {
        return dirs::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    PathBuf::from(raw)
}

fn ensure_environment(paths: &Paths) -> Result<(), String> {
    if !paths.config_path.exists() {
        return Err(format!(
            "Codex config.toml does not exist: {}",
            paths.config_path.display()
        ));
    }
    if !paths.db_path.exists() {
        return Err(format!(
            "Codex history database does not exist: {}",
            paths.db_path.display()
        ));
    }
    Ok(())
}

fn read_current_target(paths: &Paths) -> Result<CurrentTarget, String> {
    ensure_environment(paths)?;
    let config_text = fs::read_to_string(&paths.config_path)
        .map_err(|error| format!("Failed to read Codex config.toml: {error}"))?;
    let value = config_text
        .parse::<toml::Value>()
        .map_err(|error| format!("Failed to parse Codex config.toml: {error}"))?;
    let provider = match value
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    {
        Some(provider) => provider,
        None => read_official_provider_fallback(paths, &value)?
            .ok_or_else(|| "Could not find model_provider in Codex config.toml".to_string())?,
    };
    let model = value
        .get("model")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Ok(CurrentTarget { provider, model })
}

fn read_official_provider_fallback(
    paths: &Paths,
    config: &toml::Value,
) -> Result<Option<String>, String> {
    if !paths.auth_path.exists() {
        return Ok(None);
    }
    let auth_text = fs::read_to_string(&paths.auth_path)
        .map_err(|error| format!("Failed to read Codex auth.json: {error}"))?;
    let auth: Value = serde_json::from_str(&auth_text)
        .map_err(|error| format!("Failed to parse Codex auth.json: {error}"))?;
    if auth_has_official_runtime(&auth)
        && !auth_has_custom_api_key(&auth)
        && !config_has_custom_base_url(config)
    {
        Ok(Some(OFFICIAL_MODEL_PROVIDER_ID.to_string()))
    } else {
        Ok(None)
    }
}

fn auth_has_custom_api_key(auth: &Value) -> bool {
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn config_has_custom_base_url(config: &toml::Value) -> bool {
    toml_string_value(config.get("base_url")).is_some()
        || config
            .get("model_providers")
            .and_then(toml::Value::as_table)
            .is_some_and(|providers| {
                providers.values().any(|provider| {
                    provider
                        .as_table()
                        .and_then(|table| toml_string_value(table.get("base_url")))
                        .is_some()
                })
            })
}

fn toml_string_value(value: Option<&toml::Value>) -> Option<&str> {
    value
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Open a state database for SELECT-only use.
///
/// A read-only connection cannot reliably open a WAL database after Codex has
/// exited and cleaned up the `-shm` file, so this opens normally and relies on
/// callers issuing only reads. The busy timeout keeps a live Codex from turning
/// a harmless inspection into an instant failure.
pub(crate) fn open_history_read_connection(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path)
        .map_err(|error| format!("Failed to open Codex history database: {error}"))?;
    apply_sqlite_busy_timeout(&conn)?;
    Ok(conn)
}

fn open_read_connection(db_path: &Path) -> Result<Connection, String> {
    open_history_read_connection(db_path)
}

fn open_write_connection(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path)
        .map_err(|error| format!("Failed to open Codex history database: {error}"))?;
    apply_sqlite_busy_timeout(&conn)?;
    Ok(conn)
}

fn apply_sqlite_busy_timeout(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| format!("Failed to set Codex database busy timeout: {error}"))
}

/// Copy a state database through SQLite's online backup API.
///
/// `run_to_completion(c_int::MAX, …)` copies the whole WAL snapshot in one pass
/// instead of the default small page steps (a 40 MB database otherwise takes
/// minutes); a transient Busy/Locked step is retried after 50 ms.
pub(crate) fn copy_sqlite_database(
    source: &Connection,
    destination_path: &Path,
) -> Result<(), String> {
    let mut destination = Connection::open(destination_path)
        .map_err(|error| format!("Failed to open Codex history database copy: {error}"))?;
    let backup = Backup::new(source, &mut destination)
        .map_err(|error| format!("Failed to initialize Codex history database backup: {error}"))?;
    backup
        .run_to_completion(std::ffi::c_int::MAX, SQLITE_BACKUP_STEP_DELAY, None)
        .map_err(|error| format!("Failed to copy Codex history database: {error}"))?;
    Ok(())
}

#[derive(Debug, Clone)]
struct StatusDbSnapshot {
    total_threads: usize,
    provider_mismatch_threads: usize,
    provider_thread_counts: BTreeMap<Option<String>, usize>,
    model_column_exists: bool,
    visible_thread_ids: BTreeSet<String>,
}

fn read_status_db_snapshot(
    paths: &Paths,
    target: &CurrentTarget,
) -> Result<StatusDbSnapshot, String> {
    let columns = read_thread_columns(&paths.db_path)?;
    let thread_rows = read_visible_thread_rows(&paths.db_path, &columns)?;
    let total_threads = read_total_thread_count(&paths.db_path)?;
    let provider_thread_counts = read_provider_thread_counts(&paths.db_path)?;
    let provider_mismatch_threads = provider_thread_counts
        .iter()
        .filter(|(provider, _)| provider.as_deref() != Some(target.provider.as_str()))
        .map(|(_, count)| *count)
        .sum();
    Ok(StatusDbSnapshot {
        total_threads,
        provider_mismatch_threads,
        provider_thread_counts,
        model_column_exists: columns.contains("model"),
        visible_thread_ids: thread_rows.into_iter().map(|row| row.id).collect(),
    })
}

fn read_thread_columns(db_path: &Path) -> Result<BTreeSet<String>, String> {
    let conn = open_read_connection(db_path)?;
    let mut statement = conn
        .prepare("PRAGMA table_info(threads)")
        .map_err(|error| format!("Failed to inspect Codex threads table: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed to read Codex threads columns: {error}"))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row.map_err(|error| format!("Failed to read Codex column: {error}"))?);
    }
    if columns.is_empty() {
        return Err("Codex history database missing threads table".to_string());
    }
    Ok(columns)
}

fn read_total_thread_count(db_path: &Path) -> Result<usize, String> {
    let conn = open_read_connection(db_path)?;
    conn.query_row("SELECT COUNT(*) FROM threads", [], |row| {
        row.get::<_, i64>(0)
    })
    .map(|value| value.max(0) as usize)
    .map_err(|error| format!("Failed to count Codex threads: {error}"))
}

fn read_provider_thread_counts(db_path: &Path) -> Result<BTreeMap<Option<String>, usize>, String> {
    let conn = open_read_connection(db_path)?;
    let mut statement = conn
        .prepare("SELECT model_provider, COUNT(*) FROM threads GROUP BY model_provider")
        .map_err(|error| format!("Failed to inspect Codex provider distribution: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            let provider: Option<String> = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((provider, count))
        })
        .map_err(|error| format!("Failed to read Codex provider distribution: {error}"))?;
    let mut counts = BTreeMap::new();
    for row in rows {
        let (provider, count) =
            row.map_err(|error| format!("Failed to parse Codex provider distribution: {error}"))?;
        *counts
            .entry(normalize_provider_id(provider.as_deref()))
            .or_insert(0) += count.max(0) as usize;
    }
    Ok(counts)
}

fn normalize_provider_id(provider: Option<&str>) -> Option<String> {
    provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn is_ai_toolbox_legacy_provider(provider: &str) -> bool {
    provider == AI_TOOLBOX_LEGACY_PROVIDER_ID
        || provider.starts_with(AI_TOOLBOX_LAUNCH_PROVIDER_PREFIX)
}

fn classify_history_provider(provider: Option<&str>, current_provider: &str) -> &'static str {
    match provider {
        Some(provider) if provider == current_provider => PROVIDER_KIND_CURRENT,
        Some(provider) if is_ai_toolbox_legacy_provider(provider) => {
            PROVIDER_KIND_AI_TOOLBOX_LEGACY
        }
        Some(OFFICIAL_MODEL_PROVIDER_ID) => PROVIDER_KIND_OFFICIAL,
        Some(_) => PROVIDER_KIND_OTHER,
        None => PROVIDER_KIND_MISSING,
    }
}

fn provider_kind_rank(kind: &str) -> usize {
    match kind {
        PROVIDER_KIND_AI_TOOLBOX_LEGACY => 0,
        PROVIDER_KIND_MISSING => 1,
        PROVIDER_KIND_OTHER => 2,
        PROVIDER_KIND_OFFICIAL => 3,
        PROVIDER_KIND_CURRENT => 4,
        _ => 5,
    }
}

fn build_provider_breakdown(
    provider_thread_counts: &BTreeMap<Option<String>, usize>,
    session_records: &[SessionRecord],
    target: &CurrentTarget,
) -> Vec<CodexHistoryProviderGroup> {
    let mut counts: BTreeMap<Option<String>, (usize, usize)> = BTreeMap::new();
    for (provider, count) in provider_thread_counts {
        counts.entry(provider.clone()).or_default().0 += *count;
    }
    for record in session_records {
        counts.entry(record.model_provider.clone()).or_default().1 += 1;
    }

    let mut groups: Vec<CodexHistoryProviderGroup> = counts
        .into_iter()
        .map(
            |(provider_id, (thread_count, session_file_count))| CodexHistoryProviderGroup {
                kind: classify_history_provider(provider_id.as_deref(), &target.provider)
                    .to_string(),
                provider_id,
                thread_count,
                session_file_count,
            },
        )
        .collect();
    groups.sort_by(|left, right| {
        provider_kind_rank(&left.kind)
            .cmp(&provider_kind_rank(&right.kind))
            .then_with(|| left.provider_id.cmp(&right.provider_id))
    });
    groups
}

fn read_visible_thread_rows(
    db_path: &Path,
    columns: &BTreeSet<String>,
) -> Result<Vec<ThreadRow>, String> {
    let conn = open_read_connection(db_path)?;
    let title_expr = if columns.contains("title") {
        "title"
    } else {
        "NULL"
    };
    let updated_expr = if columns.contains("updated_at") {
        "updated_at"
    } else {
        "NULL"
    };
    let archived_filter = if columns.contains("archived") {
        " WHERE archived IS NULL OR archived = 0"
    } else {
        ""
    };
    let sql = format!("SELECT id, {title_expr}, {updated_expr} FROM threads{archived_filter}");
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| format!("Failed to read Codex threads: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(ThreadRow {
                id: row.get::<_, String>(0)?,
                title: row.get::<_, Option<String>>(1)?,
                updated_at: read_optional_i64(row, 2)?,
            })
        })
        .map_err(|error| format!("Failed to query Codex threads: {error}"))?;
    let mut threads = Vec::new();
    for row in rows {
        let thread = row.map_err(|error| format!("Failed to parse Codex thread: {error}"))?;
        if !thread.id.trim().is_empty() {
            threads.push(thread);
        }
    }
    Ok(threads)
}

fn read_optional_i64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<i64>> {
    match row.get_ref(index)? {
        rusqlite::types::ValueRef::Null => Ok(None),
        rusqlite::types::ValueRef::Integer(value) => Ok(Some(value)),
        rusqlite::types::ValueRef::Real(value) => Ok(Some(value as i64)),
        rusqlite::types::ValueRef::Text(value) => Ok(std::str::from_utf8(value)
            .ok()
            .and_then(|text| text.parse::<i64>().ok())),
        rusqlite::types::ValueRef::Blob(_) => Ok(None),
    }
}

fn build_selection_plan(
    selection: Option<&CodexHistorySyncSelection>,
    target: &CurrentTarget,
) -> Result<Option<SelectionPlan>, String> {
    let Some(selection) = selection else {
        return Ok(None);
    };

    let include_missing = selection.include_missing_provider.unwrap_or(false);
    let mut providers = BTreeSet::new();
    for provider in selection.source_providers.iter().flatten() {
        let provider = provider.trim();
        // The active provider is already routed correctly; keep it out of the
        // update set so "selected" always means "will be rewritten".
        if provider.is_empty() || provider == target.provider {
            continue;
        }
        providers.insert(provider.to_string());
    }

    if providers.is_empty() && !include_missing {
        return Err("No Codex history providers were selected for sync".to_string());
    }

    Ok(Some(SelectionPlan {
        providers,
        include_missing,
    }))
}

fn sync_database(
    db_path: &Path,
    target: &CurrentTarget,
    plan: Option<&SelectionPlan>,
) -> Result<usize, String> {
    let mut conn = open_write_connection(db_path)?;
    let transaction = conn
        .transaction()
        .map_err(|error| format!("Failed to begin Codex history sync transaction: {error}"))?;
    let updated = match plan {
        None => transaction
            .execute(
                "UPDATE threads SET model_provider = ?1 WHERE model_provider IS NULL OR model_provider <> ?1",
                [target.provider.as_str()],
            )
            .map_err(|error| format!("Failed to sync Codex history database: {error}"))?,
        Some(plan) => {
            let mut clauses = Vec::new();
            let mut params: Vec<SqlValue> = vec![SqlValue::Text(target.provider.clone())];
            if plan.include_missing {
                clauses.push("(model_provider IS NULL OR TRIM(model_provider) = '')".to_string());
            }
            if !plan.providers.is_empty() {
                let placeholders = vec!["?"; plan.providers.len()].join(", ");
                clauses.push(format!("TRIM(model_provider) IN ({placeholders})"));
                params.extend(plan.providers.iter().cloned().map(SqlValue::Text));
            }
            let sql = format!(
                "UPDATE threads SET model_provider = ?1 WHERE ({}) AND (model_provider IS NULL OR model_provider <> ?1)",
                clauses.join(" OR ")
            );
            transaction
                .execute(&sql, rusqlite::params_from_iter(params))
                .map_err(|error| format!("Failed to sync Codex history database: {error}"))?
        }
    };
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit Codex history sync: {error}"))?;
    let _ = open_write_connection(db_path)?.pragma_update(None, "wal_checkpoint", "PASSIVE");
    Ok(updated)
}

fn retry_sqlite_operation<T>(
    mut operation: impl FnMut() -> Result<T, String>,
) -> Result<RetryStats<T>, String> {
    let mut attempts = 0;
    let start = Instant::now();
    loop {
        attempts += 1;
        match operation() {
            Ok(value) => {
                return Ok(RetryStats {
                    value,
                    attempts,
                    lock_wait_ms: start.elapsed().as_millis(),
                });
            }
            Err(error) if is_sqlite_locked_error(&error) && attempts < SQLITE_LOCK_RETRY_LIMIT => {
                thread::sleep(SQLITE_LOCK_RETRY_DELAY);
            }
            Err(error) if is_sqlite_locked_error(&error) => {
                return Err(format_sqlite_locked_error(&error));
            }
            Err(error) => return Err(error),
        }
    }
}

fn format_sqlite_locked_error(error: &str) -> String {
    format!("{SQLITE_LOCKED_USER_MESSAGE} Last error: {error}")
}

fn is_sqlite_locked_error(error: &str) -> bool {
    let value = error.to_ascii_lowercase();
    value.contains("database is locked")
        || value.contains("database table is locked")
        || value.contains("database is busy")
        || value.contains("destination database is in use")
        || value.contains("locked")
}

fn scan_session_records(paths: &Paths) -> Result<Vec<SessionRecord>, String> {
    let mut files = Vec::new();
    collect_rollout_files(&paths.sessions_dir, &mut files)?;
    collect_rollout_files(&paths.archived_sessions_dir, &mut files)?;
    files.sort();
    let files = dedupe_rollout_siblings(files);
    let mut records = Vec::new();
    for path in files {
        if let Some(record) = read_session_record(paths, &path)? {
            records.push(record);
        }
    }
    Ok(records)
}

fn collect_rollout_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|error| {
        format!(
            "Failed to read Codex sessions dir {}: {error}",
            dir.display()
        )
    })?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Failed to read Codex session entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_rollout_files(&path, files)?;
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if file_name.starts_with("rollout-") && is_rollout_session_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

/// Whether a path is a rollout file in either representation (extension only).
pub(crate) fn is_rollout_session_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".jsonl") || name.ends_with(ROLLOUT_COMPRESSED_SUFFIX))
}

pub(crate) fn is_compressed_rollout(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(ROLLOUT_COMPRESSED_SUFFIX))
}

fn logical_rollout_path(path: &Path) -> PathBuf {
    if !is_compressed_rollout(path) {
        return path.to_path_buf();
    }
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return path.to_path_buf();
    };
    let Some(plain_name) = name.strip_suffix(".zst") else {
        return path.to_path_buf();
    };
    path.with_file_name(plain_name)
}

/// When both `x.jsonl` and `x.jsonl.zst` exist (mid-compression or a leftover
/// archive), only the plain file is authoritative: it is the one Codex writes
/// and it contains at least everything the compressed copy does.
pub(crate) fn dedupe_rollout_siblings(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let plain: BTreeSet<PathBuf> = files
        .iter()
        .filter(|path| !is_compressed_rollout(path))
        .cloned()
        .collect();
    files
        .into_iter()
        .filter(|path| !is_compressed_rollout(path) || !plain.contains(&logical_rollout_path(path)))
        .collect()
}

fn read_first_line(path: &Path) -> Result<Option<String>, String> {
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "Failed to open Codex session file {}: {error}",
            path.display()
        )
    })?;
    let mut first_line = String::new();
    let read = if is_compressed_rollout(path) {
        let decoder = zstd::stream::read::Decoder::new(file).map_err(|error| {
            format!(
                "Failed to decode Codex session file {}: {error}",
                path.display()
            )
        })?;
        BufReader::new(decoder)
            .read_line(&mut first_line)
            .map_err(|error| {
                format!(
                    "Failed to read Codex session file {}: {error}",
                    path.display()
                )
            })?
    } else {
        BufReader::new(file)
            .read_line(&mut first_line)
            .map_err(|error| {
                format!(
                    "Failed to read Codex session file {}: {error}",
                    path.display()
                )
            })?
    };
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(first_line.trim_end_matches(['\r', '\n']).to_string()))
}

pub(crate) fn read_session_text(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "Failed to read Codex session file {}: {error}",
            path.display()
        )
    })?;
    if !is_compressed_rollout(path) {
        return String::from_utf8(bytes).map_err(|error| {
            format!(
                "Failed to decode Codex session file {}: {error}",
                path.display()
            )
        });
    }
    let decoded = zstd::stream::decode_all(Cursor::new(bytes)).map_err(|error| {
        format!(
            "Failed to decompress Codex session file {}: {error}",
            path.display()
        )
    })?;
    String::from_utf8(decoded).map_err(|error| {
        format!(
            "Failed to decode Codex session file {}: {error}",
            path.display()
        )
    })
}

/// Every `session_meta` line in a rollout, with its absolute line number.
///
/// A thread can append another `session_meta` when it is resumed, so restoring
/// only line 0 would leave the later provider record stale.
fn read_session_meta_lines(path: &Path) -> Result<Vec<SessionMetaLineBackup>, String> {
    let text = read_session_text(path)?;
    let mut meta_lines = Vec::new();
    for (line_index, segment) in text.split_inclusive('\n').enumerate() {
        let (line, _) = split_line_segment(segment);
        if !line.contains("\"session_meta\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        meta_lines.push(SessionMetaLineBackup {
            line_index,
            content: line.to_string(),
        });
    }
    Ok(meta_lines)
}

fn read_session_record(paths: &Paths, path: &Path) -> Result<Option<SessionRecord>, String> {
    let Some(normalized_first_line) = read_first_line(path)? else {
        return Ok(None);
    };
    let value: Value = match serde_json::from_str(&normalized_first_line) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let Some(payload) = value.get("payload").and_then(Value::as_object) else {
        return Ok(None);
    };
    if payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .is_none()
    {
        return Ok(None);
    }
    Ok(Some(SessionRecord {
        relative_path: relative_path(paths, path)?,
        path: path.to_path_buf(),
        model_provider: normalize_provider_id(
            payload.get("model_provider").and_then(Value::as_str),
        ),
        first_line: normalized_first_line,
    }))
}

fn relative_path(paths: &Paths, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(&paths.codex_home).map_err(|_| {
        format!(
            "Codex session path {} is outside Codex home {}",
            path.display(),
            paths.codex_home.display()
        )
    })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn session_record_needs_sync(record: &SessionRecord, target: &CurrentTarget) -> bool {
    record.model_provider.as_deref() != Some(target.provider.as_str())
}

struct SessionSyncResult {
    updated: usize,
    failed: usize,
    first_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

fn file_stamp(path: &Path) -> Result<FileStamp, String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "Failed to inspect Codex session file {}: {error}",
            path.display()
        )
    })?;
    Ok(FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

fn ensure_file_unchanged(path: &Path, before: FileStamp) -> Result<(), String> {
    let after = file_stamp(path)?;
    if after.modified != before.modified || after.len != before.len {
        return Err(format!(
            "Codex session file changed during history sync: {}",
            path.display()
        ));
    }
    Ok(())
}

fn sync_session_records(
    paths: &Paths,
    target: &CurrentTarget,
    plan: Option<&SelectionPlan>,
) -> Result<SessionSyncResult, String> {
    let records = scan_session_records(paths)?;
    let mut updated = 0;
    let mut failed = 0;
    let mut first_error = None;
    // Every rollout is inspected: a later `session_meta` line can still carry
    // a provider that differs from the first one, and the file-level provider
    // record only describes the first line.
    for record in records {
        match rewrite_session_meta_lines(&record.path, target, plan) {
            Ok(true) => updated += 1,
            Ok(false) => {}
            Err(error) => {
                failed += 1;
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    Ok(SessionSyncResult {
        updated,
        failed,
        first_error,
    })
}

fn rewrite_session_meta_lines(
    path: &Path,
    target: &CurrentTarget,
    plan: Option<&SelectionPlan>,
) -> Result<bool, String> {
    let stamp = file_stamp(path)?;
    let text = read_session_text(path)?;
    let mut rewritten = String::with_capacity(text.len());
    let mut changed = false;
    for segment in text.split_inclusive('\n') {
        let (line, newline) = split_line_segment(segment);
        match rewrite_session_meta_line(line, target, plan)? {
            Some(next_line) => {
                rewritten.push_str(&next_line);
                changed = true;
            }
            None => rewritten.push_str(line),
        }
        rewritten.push_str(newline);
    }
    if !changed {
        return Ok(false);
    }
    ensure_file_unchanged(path, stamp)?;
    write_session_text(path, &rewritten)?;
    Ok(true)
}

fn rewrite_session_meta_line(
    line: &str,
    target: &CurrentTarget,
    plan: Option<&SelectionPlan>,
) -> Result<Option<String>, String> {
    if !line.contains("\"session_meta\"") {
        return Ok(None);
    }
    let mut value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) else {
        return Ok(None);
    };

    let current_provider =
        normalize_provider_id(payload.get("model_provider").and_then(Value::as_str));
    if current_provider.as_deref() == Some(target.provider.as_str()) {
        return Ok(None);
    }
    let selected = match plan {
        None => true,
        Some(plan) => match current_provider.as_deref() {
            None => plan.include_missing,
            Some(provider) => plan.providers.contains(provider),
        },
    };
    if !selected {
        return Ok(None);
    }

    payload.insert(
        "model_provider".to_string(),
        Value::String(target.provider.clone()),
    );
    let next_line = serde_json::to_string(&value)
        .map_err(|error| format!("Failed to serialize Codex session metadata: {error}"))?;
    if next_line == line {
        return Ok(None);
    }
    Ok(Some(next_line))
}

pub(crate) fn write_session_text(path: &Path, text: &str) -> Result<(), String> {
    let modified = fs::metadata(path).and_then(|meta| meta.modified()).ok();
    if !is_compressed_rollout(path) {
        write_atomic_with_retry(path, text.as_bytes())?;
    } else {
        let compressed = zstd::stream::encode_all(text.as_bytes(), ZSTD_COMPRESSION_LEVEL)
            .map_err(|error| {
                format!(
                    "Failed to compress Codex session file {}: {error}",
                    path.display()
                )
            })?;
        write_atomic_with_retry(path, &compressed)?;
    }
    if let Some(modified) = modified {
        // `codex resume` and the sidebar sort by mtime, and Codex preserves it
        // when compressing. Rewriting metadata must not make an old session
        // look freshly active. Unix only needs a read handle; Windows needs a
        // writable one. A failed timestamp restore is not worth aborting.
        let file = if cfg!(windows) {
            fs::File::options().write(true).open(path)
        } else {
            fs::File::open(path)
        };
        let times = fs::FileTimes::new().set_modified(modified);
        if let Err(error) = file.and_then(|file| file.set_times(times)) {
            log::warn!("Failed to restore mtime of {}: {error}", path.display());
        }
    }
    Ok(())
}

fn split_line_segment(segment: &str) -> (&str, &str) {
    if let Some(stripped) = segment.strip_suffix('\n') {
        if let Some(line) = stripped.strip_suffix('\r') {
            (line, "\r\n")
        } else {
            (stripped, "\n")
        }
    } else {
        (segment, "")
    }
}

fn write_atomic_with_retry(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Failed to determine parent for {}", path.display()))?;
    let tmp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("codex-history-sync"),
        Uuid::new_v4().simple()
    ));
    let mut last_error = None;
    for attempt in 0..FILE_RETRY_LIMIT {
        let result = fs::write(&tmp_path, bytes).and_then(|_| fs::rename(&tmp_path, path));
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error.to_string());
                let _ = fs::remove_file(&tmp_path);
                if attempt + 1 < FILE_RETRY_LIMIT {
                    thread::sleep(FILE_RETRY_DELAY);
                }
            }
        }
    }
    Err(format!(
        "Failed to replace Codex file {}: {}",
        path.display(),
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn read_session_index(path: &Path) -> Result<BTreeMap<String, SessionIndexEntry>, String> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let data = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read Codex session index: {error}"))?;
    let mut entries = BTreeMap::new();
    for line in data.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<SessionIndexEntry>(trimmed) else {
            continue;
        };
        if entry.id.trim().is_empty() {
            continue;
        }
        entries.insert(entry.id.clone(), entry);
    }
    Ok(entries)
}

fn write_session_index(path: &Path, entries: &[SessionIndexEntry]) -> Result<(), String> {
    let mut content = String::new();
    for entry in entries {
        let serialized = serde_json::to_string(entry)
            .map_err(|error| format!("Failed to serialize Codex session index: {error}"))?;
        content.push_str(&serialized);
        content.push('\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Codex session index directory {}: {error}",
                parent.display()
            )
        })?;
    }
    write_atomic_with_retry(path, content.as_bytes())
}

fn rebuild_session_index(paths: &Paths) -> Result<IndexRebuildResult, String> {
    let columns = read_thread_columns(&paths.db_path)?;
    let thread_rows = read_visible_thread_rows(&paths.db_path, &columns)?;
    let existing = read_session_index(&paths.session_index_path)?;
    let visible_thread_ids: BTreeSet<String> =
        thread_rows.iter().map(|row| row.id.clone()).collect();
    let missing_entries_before = visible_thread_ids
        .iter()
        .filter(|id| !existing.contains_key(*id))
        .count();
    let mut entries = Vec::new();
    for thread in &thread_rows {
        let existing_entry = existing.get(&thread.id);
        let thread_name = existing_entry
            .map(|entry| entry.thread_name.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| thread.title.as_ref().map(|value| value.trim().to_string()))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| thread.id.clone());
        let updated_at = existing_entry
            .map(|entry| entry.updated_at.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| thread.updated_at.and_then(format_unix_seconds))
            .unwrap_or_default();
        entries.push(SessionIndexEntry {
            id: thread.id.clone(),
            thread_name,
            updated_at,
        });
    }
    let mut preserved_index_only_entries = 0;
    for (id, entry) in &existing {
        if visible_thread_ids.contains(id) {
            continue;
        }
        preserved_index_only_entries += 1;
        entries.push(entry.clone());
    }
    entries.sort_by(|left, right| {
        left.updated_at
            .cmp(&right.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    write_session_index(&paths.session_index_path, &entries)?;
    Ok(IndexRebuildResult {
        rewritten_entries: entries.len(),
        missing_entries_before,
        preserved_index_only_entries,
    })
}

pub(crate) fn rebuild_session_index_for_db(
    codex_home: &Path,
    db_path: &Path,
) -> Result<usize, String> {
    if !db_path.exists() {
        return Ok(0);
    }

    let mut paths = base_paths(codex_home);
    paths.db_path = db_path.to_path_buf();
    rebuild_session_index(&paths).map(|result| result.rewritten_entries)
}

fn format_unix_seconds(value: i64) -> Option<String> {
    Utc.timestamp_opt(value, 0)
        .single()
        .map(|value| value.to_rfc3339())
}

fn make_backup(paths: &Paths, label: &str) -> Result<BackupOutcome, String> {
    ensure_environment(paths)?;
    fs::create_dir_all(&paths.backup_dir).map_err(|error| {
        format!(
            "Failed to create Codex history backup directory {}: {error}",
            paths.backup_dir.display()
        )
    })?;
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let db_file_name = paths
        .db_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state.sqlite");
    let backup_path = paths.backup_dir.join(format!(
        "{db_file_name}.{label}.{timestamp}.{}.bak",
        Uuid::new_v4().simple()
    ));
    backup_database_to_path(&paths.db_path, &backup_path)?;
    snapshot_metadata(paths, &backup_path)?;
    Ok(BackupOutcome { path: backup_path })
}

fn backup_database_to_path(db_path: &Path, backup_path: &Path) -> Result<(), String> {
    retry_sqlite_operation(|| {
        let conn = open_read_connection(db_path)?;
        copy_sqlite_database(&conn, backup_path)
    })
    .map(|_| ())
}

fn snapshot_metadata(paths: &Paths, backup_path: &Path) -> Result<(), String> {
    if paths.session_index_path.exists() {
        let index_text = fs::read_to_string(&paths.session_index_path)
            .map_err(|error| format!("Failed to read Codex session index for backup: {error}"))?;
        fs::write(session_index_backup_path(backup_path), index_text)
            .map_err(|error| format!("Failed to write Codex session index backup: {error}"))?;
    }
    let records = scan_session_records(paths)?;
    let mut entries = Vec::with_capacity(records.len());
    for record in records {
        entries.push(SessionMetaBackupEntry {
            meta_lines: read_session_meta_lines(&record.path)?,
            relative_path: record.relative_path,
            first_line: record.first_line,
        });
    }
    let content = serde_json::to_string_pretty(&entries)
        .map_err(|error| format!("Failed to serialize Codex session metadata backup: {error}"))?;
    fs::write(session_meta_backup_path(backup_path), content)
        .map_err(|error| format!("Failed to write Codex session metadata backup: {error}"))?;
    Ok(())
}

fn session_index_backup_path(backup_path: &Path) -> PathBuf {
    backup_path.with_file_name(format!(
        "{}.session_index.jsonl",
        backup_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ))
}

fn session_meta_backup_path(backup_path: &Path) -> PathBuf {
    backup_path.with_file_name(format!(
        "{}.session_meta.json",
        backup_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ))
}

fn restore_database_from_backup(db_path: &Path, backup_path: &Path) -> Result<(), String> {
    if !backup_path.exists() {
        return Err(format!(
            "Codex history backup does not exist: {}",
            backup_path.display()
        ));
    }
    // Caller already wraps this in retry_sqlite_operation for lock wait stats.
    let source = open_read_connection(backup_path)?;
    copy_sqlite_database(&source, db_path)
}

fn restore_metadata_sidecars(paths: &Paths, backup_path: &Path) -> Result<(usize, usize), String> {
    let index_backup = session_index_backup_path(backup_path);
    if index_backup.exists() {
        let text = fs::read(&index_backup)
            .map_err(|error| format!("Failed to read Codex session index backup: {error}"))?;
        write_atomic_with_retry(&paths.session_index_path, &text)?;
    }
    let meta_backup = session_meta_backup_path(backup_path);
    if !meta_backup.exists() {
        return Ok((0, 0));
    }
    let content = fs::read_to_string(&meta_backup)
        .map_err(|error| format!("Failed to read Codex session metadata backup: {error}"))?;
    let entries: Vec<SessionMetaBackupEntry> = serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse Codex session metadata backup: {error}"))?;
    let mut restored = 0;
    let mut skipped = 0;
    for entry in entries {
        let Ok(path) = join_safe_relative(&paths.codex_home, &entry.relative_path) else {
            skipped += 1;
            continue;
        };
        if !path.exists() {
            skipped += 1;
            continue;
        }
        if restore_session_meta_entry(&path, &entry)? {
            restored += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((restored, skipped))
}

fn restore_session_meta_entry(path: &Path, entry: &SessionMetaBackupEntry) -> Result<bool, String> {
    let stamp = file_stamp(path)?;
    let text = read_session_text(path)?;
    let mut segments: Vec<String> = text
        .split_inclusive('\n')
        .map(ToString::to_string)
        .collect();
    let mut changed = false;

    if !entry.meta_lines.is_empty() {
        for meta_line in &entry.meta_lines {
            let Some(segment) = segments.get_mut(meta_line.line_index) else {
                continue;
            };
            let (_, newline) = split_line_segment(segment);
            let next = format!("{}{}", meta_line.content, newline);
            if *segment != next {
                *segment = next;
                changed = true;
            }
        }
    } else if !entry.first_line.is_empty() {
        if let Some(segment) = segments.get_mut(0) {
            let (_, newline) = split_line_segment(segment);
            let next = format!("{}{}", entry.first_line, newline);
            if *segment != next {
                *segment = next;
                changed = true;
            }
        }
    }

    if !changed {
        // Old backups store only the first line; a file that already matches
        // still counts as restored (nothing to rewrite).
        return Ok(!entry.meta_lines.is_empty() || !entry.first_line.is_empty());
    }
    ensure_file_unchanged(path, stamp)?;
    write_session_text(path, &segments.concat())?;
    Ok(true)
}

fn join_safe_relative(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative_path).components() {
        match component {
            std::path::Component::Normal(value) => path.push(value),
            std::path::Component::CurDir => {}
            _ => {
                return Err(format!(
                    "Unsafe Codex backup relative path: {relative_path}"
                ))
            }
        }
    }
    Ok(path)
}

fn list_backup_files(backup_dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }
    let re = Regex::new(r"^state_\d+\.sqlite\..*\.bak$").unwrap();
    let mut entries = Vec::new();
    for entry in fs::read_dir(backup_dir).map_err(|error| {
        format!(
            "Failed to read Codex history backup directory {}: {error}",
            backup_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("Failed to read Codex backup entry: {error}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if re.is_match(name) {
            entries.push(path);
        }
    }
    entries.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct TestCodexHistory {
        _temp_dir: TempDir,
        root: PathBuf,
    }

    impl TestCodexHistory {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let root = temp_dir.path().join("codex-home");
            fs::create_dir_all(root.join("sessions/2026/05/21")).expect("create sessions");
            fs::write(
                root.join("config.toml"),
                r#"model_provider = "new-provider"
model = "gpt-new"
"#,
            )
            .expect("write config");
            Self {
                _temp_dir: temp_dir,
                root,
            }
        }

        fn create_db(&self, with_model: bool) {
            let conn = Connection::open(self.root.join("state_5.sqlite")).expect("open db");
            if with_model {
                conn.execute_batch(
                    r#"
                    CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        model_provider TEXT,
                        model TEXT,
                        title TEXT,
                        updated_at INTEGER,
                        archived INTEGER
                    );
                    INSERT INTO threads (id, model_provider, model, title, updated_at, archived)
                    VALUES
                        ('11111111-1111-1111-1111-111111111111', 'old-provider', 'gpt-old', 'Old thread', 1700000000, 0),
                        ('22222222-2222-2222-2222-222222222222', 'new-provider', 'gpt-old', 'Model old', 1700000100, 0),
                        ('33333333-3333-3333-3333-333333333333', 'new-provider', 'gpt-new', 'Archived', 1700000200, 1),
                        ('44444444-4444-4444-4444-444444444444', 'old-provider', 'gpt-other', 'Other model', 1700000300, 0);
                    "#,
                )
                .expect("create model db");
            } else {
                conn.execute_batch(
                    r#"
                    CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        model_provider TEXT,
                        title TEXT,
                        updated_at INTEGER
                    );
                    INSERT INTO threads (id, model_provider, title, updated_at)
                    VALUES ('11111111-1111-1111-1111-111111111111', 'old-provider', 'Old thread', 1700000000);
                    "#,
                )
                .expect("create legacy db");
            }
        }

        fn write_session(&self, id: &str, provider: &str, model: &str) -> PathBuf {
            let path = self
                .root
                .join("sessions/2026/05/21")
                .join(format!("rollout-2026-05-21T00-00-00-{id}.jsonl"));
            fs::write(
                &path,
                format!(
                    "{}\n{}\n",
                    serde_json::json!({
                        "type": "session_meta",
                        "payload": {
                            "id": id,
                            "cwd": "/tmp/project",
                            "model_provider": provider,
                            "model": model
                        }
                    }),
                    serde_json::json!({
                        "type": "response_item",
                        "payload": {"type": "message", "role": "user", "content": "hello"}
                    })
                ),
            )
            .expect("write session");
            path
        }
    }

    fn create_threads_db(path: &Path, providers: &[Option<&str>]) {
        let conn = Connection::open(path).expect("open db");
        conn.execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                model TEXT,
                title TEXT,
                updated_at INTEGER,
                archived INTEGER
            );
            "#,
        )
        .expect("create threads table");
        for (index, provider) in providers.iter().enumerate() {
            conn.execute(
                "INSERT INTO threads (id, model_provider, model, title, updated_at, archived)
                 VALUES (?1, ?2, NULL, NULL, 0, 0)",
                rusqlite::params![format!("{index:08}-1111-1111-1111-111111111111"), *provider],
            )
            .expect("insert thread");
        }
    }

    fn write_rollout(
        root: &Path,
        relative_dir: &str,
        id: &str,
        provider: Option<&str>,
        compressed: bool,
    ) -> PathBuf {
        let dir = root.join(relative_dir);
        fs::create_dir_all(&dir).expect("create rollout dir");
        let mut payload = serde_json::json!({
            "id": id,
            "cwd": "/tmp/project",
            "model": "gpt-old"
        });
        if let Some(provider) = provider {
            payload["model_provider"] = serde_json::json!(provider);
        }
        let text = format!(
            "{}\n{}\n",
            serde_json::json!({"type": "session_meta", "payload": payload}),
            serde_json::json!({
                "type": "response_item",
                "payload": {"type": "message", "role": "user", "content": "hello"}
            })
        );
        let path = dir.join(format!(
            "rollout-2026-05-21T00-00-00-{id}.jsonl{}",
            if compressed { ".zst" } else { "" }
        ));
        if compressed {
            fs::write(
                &path,
                zstd::stream::encode_all(text.as_bytes(), 3).expect("compress rollout"),
            )
            .expect("write compressed rollout");
        } else {
            fs::write(&path, text).expect("write rollout");
        }
        path
    }

    fn read_rollout_text(path: &Path) -> String {
        let bytes = fs::read(path).expect("read rollout bytes");
        if is_compressed_rollout(path) {
            let decoded = zstd::stream::decode_all(Cursor::new(bytes)).expect("decompress");
            String::from_utf8(decoded).expect("utf8 rollout")
        } else {
            String::from_utf8(bytes).expect("utf8 rollout")
        }
    }

    fn provider_of(path: &Path) -> Option<String> {
        let text = read_rollout_text(path);
        let first_line = text.lines().next().expect("first line");
        let value: Value = serde_json::from_str(first_line).expect("session meta json");
        value
            .get("payload")
            .and_then(|payload| payload.get("model_provider"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
    }

    fn session_meta_providers(path: &Path) -> Vec<String> {
        read_rollout_text(path)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| value.get("type").and_then(Value::as_str) == Some("session_meta"))
            .filter_map(|value| {
                value
                    .get("payload")
                    .and_then(|payload| payload.get("model_provider"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect()
    }

    fn set_rollout_mtime(path: &Path, seconds: u64) {
        let file = if cfg!(windows) {
            fs::File::options().write(true).open(path)
        } else {
            fs::File::open(path)
        };
        file.expect("open rollout for mtime")
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
            )
            .expect("set rollout mtime");
    }

    fn rollout_mtime(path: &Path) -> SystemTime {
        fs::metadata(path)
            .expect("rollout metadata")
            .modified()
            .expect("rollout modified time")
    }

    #[test]
    fn sqlite_home_config_wins_and_newest_state_db_is_used() {
        let env = TestCodexHistory::new();
        let sqlite_home = env.root.join("custom-sqlite");
        fs::create_dir_all(&sqlite_home).expect("create sqlite home");
        create_threads_db(
            &sqlite_home.join("state_4.sqlite"),
            &[Some("ai-toolbox-gateway")],
        );
        create_threads_db(&sqlite_home.join("state_6.sqlite"), &[Some("old-provider")]);
        // A newer database in the Codex home must not win over an explicitly
        // configured SQLite home.
        create_threads_db(&env.root.join("state_9.sqlite"), &[Some("new-provider")]);
        fs::write(
            env.root.join("config.toml"),
            format!(
                "model_provider = \"new-provider\"\nsqlite_home = {:?}\n",
                sqlite_home.to_string_lossy()
            ),
        )
        .expect("write config");

        let status = get_status(&env.root).expect("status");

        assert_eq!(
            PathBuf::from(&status.db_path),
            sqlite_home.join("state_6.sqlite")
        );
        assert_eq!(status.total_threads, 1);

        let backup = backup(&env.root, "manual").expect("backup");
        assert!(backup.backup_path.contains("state_6.sqlite.manual"));
    }

    #[test]
    fn status_and_sync_cover_archived_and_compressed_rollouts() {
        let env = TestCodexHistory::new();
        env.create_db(true);
        let compressed_path = write_rollout(
            &env.root,
            "archived_sessions/2026/05/21",
            "aaaaaaaa-1111-1111-1111-111111111111",
            Some("old-provider"),
            true,
        );

        let status = get_status(&env.root).expect("status");
        assert_eq!(status.session_file_count, 1);
        assert_eq!(status.session_meta_mismatch_count, 1);
        let legacy_group = status
            .provider_breakdown
            .iter()
            .find(|group| group.provider_id.as_deref() == Some("old-provider"))
            .expect("legacy group");
        assert_eq!(legacy_group.kind, "other");
        assert_eq!(legacy_group.thread_count, 2);
        assert_eq!(legacy_group.session_file_count, 1);

        let selection = CodexHistorySyncSelection {
            source_providers: Some(vec!["old-provider".to_string()]),
            include_missing_provider: Some(false),
        };
        let result = sync(&env.root, Some(&selection)).expect("sync selected");
        assert_eq!(result.updated_thread_rows, 2);
        assert_eq!(result.updated_session_files, 1);
        assert_eq!(
            provider_of(&compressed_path).as_deref(),
            Some("new-provider")
        );
        assert!(read_rollout_text(&compressed_path).contains("hello"));

        let restored = restore_latest(&env.root).expect("restore");
        assert_eq!(restored.restored_session_meta_files, 1);
        assert_eq!(
            provider_of(&compressed_path).as_deref(),
            Some("old-provider")
        );
    }

    #[test]
    fn provider_breakdown_classifies_and_sorts_groups() {
        let env = TestCodexHistory::new();
        create_threads_db(
            &env.root.join("state_5.sqlite"),
            &[
                Some("aitb-launch-abc"),
                Some("ai-toolbox-gateway"),
                None,
                Some("openai"),
                Some("third-party"),
                Some("new-provider"),
            ],
        );
        write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "aaaaaaaa-1111-1111-1111-111111111111",
            Some("ai-toolbox-gateway"),
            false,
        );
        write_rollout(
            &env.root,
            "archived_sessions/2026/05/21",
            "bbbbbbbb-1111-1111-1111-111111111111",
            None,
            false,
        );
        write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "cccccccc-1111-1111-1111-111111111111",
            Some("openai"),
            false,
        );
        write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "dddddddd-1111-1111-1111-111111111111",
            Some("new-provider"),
            false,
        );

        let status = get_status(&env.root).expect("status");
        let summary: Vec<(Option<&str>, &str, usize, usize)> = status
            .provider_breakdown
            .iter()
            .map(|group| {
                (
                    group.provider_id.as_deref(),
                    group.kind.as_str(),
                    group.thread_count,
                    group.session_file_count,
                )
            })
            .collect();

        assert_eq!(
            summary,
            vec![
                (Some("ai-toolbox-gateway"), "ai_toolbox_legacy", 1, 1),
                (Some("aitb-launch-abc"), "ai_toolbox_legacy", 1, 0),
                (None, "missing", 1, 1),
                (Some("third-party"), "other", 1, 0),
                (Some("openai"), "official", 1, 1),
                (Some("new-provider"), "current", 1, 1),
            ]
        );
    }

    #[test]
    fn targeted_sync_only_rewrites_selected_providers() {
        let env = TestCodexHistory::new();
        create_threads_db(
            &env.root.join("state_5.sqlite"),
            &[
                Some("new-provider"),
                Some("ai-toolbox-gateway"),
                Some("openai"),
                None,
                Some("third-party"),
            ],
        );
        let current_path = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "aaaaaaaa-1111-1111-1111-111111111111",
            Some("new-provider"),
            false,
        );
        let legacy_path = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "bbbbbbbb-1111-1111-1111-111111111111",
            Some("ai-toolbox-gateway"),
            false,
        );
        let official_path = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "cccccccc-1111-1111-1111-111111111111",
            Some("openai"),
            false,
        );
        let missing_path = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "dddddddd-1111-1111-1111-111111111111",
            None,
            false,
        );
        let other_path = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "eeeeeeee-1111-1111-1111-111111111111",
            Some("third-party"),
            false,
        );

        let selection = CodexHistorySyncSelection {
            source_providers: Some(vec!["ai-toolbox-gateway".to_string()]),
            include_missing_provider: Some(false),
        };
        let result = sync(&env.root, Some(&selection)).expect("sync legacy only");
        assert_eq!(result.updated_thread_rows, 1);
        assert_eq!(result.updated_session_files, 1);

        assert_eq!(provider_of(&legacy_path).as_deref(), Some("new-provider"));
        assert_eq!(provider_of(&official_path).as_deref(), Some("openai"));
        assert_eq!(provider_of(&missing_path), None);
        assert_eq!(provider_of(&other_path).as_deref(), Some("third-party"));
        assert_eq!(provider_of(&current_path).as_deref(), Some("new-provider"));
        let conn = Connection::open(env.root.join("state_5.sqlite")).expect("open db");
        let official_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE model_provider = 'openai'",
                [],
                |row| row.get(0),
            )
            .expect("count official");
        assert_eq!(official_rows, 1);

        let missing_only = CodexHistorySyncSelection {
            source_providers: Some(Vec::new()),
            include_missing_provider: Some(true),
        };
        let result = sync(&env.root, Some(&missing_only)).expect("sync missing only");
        assert_eq!(result.updated_thread_rows, 1);
        assert_eq!(result.updated_session_files, 1);
        assert_eq!(provider_of(&missing_path).as_deref(), Some("new-provider"));
        assert_eq!(provider_of(&official_path).as_deref(), Some("openai"));

        let empty_selection = CodexHistorySyncSelection {
            source_providers: Some(Vec::new()),
            include_missing_provider: Some(false),
        };
        let error = sync(&env.root, Some(&empty_selection)).expect_err("empty selection");
        assert!(error.contains("No Codex history providers were selected"));

        let no_match_selection = CodexHistorySyncSelection {
            source_providers: Some(vec!["not-present".to_string()]),
            include_missing_provider: Some(false),
        };
        let result = sync(&env.root, Some(&no_match_selection)).expect("no matching rows");
        assert_eq!(result.updated_thread_rows, 0);
        assert_eq!(result.updated_session_files, 0);
    }

    #[test]
    fn backup_listing_accepts_legacy_and_current_state_db_names() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let names = [
            "state_5.sqlite.pre-sync.20260101-000000.aaaaaaaa.bak",
            "state_6.sqlite.manual.20260102-000000.bbbbbbbb.bak",
            "state_6.sqlite.manual.20260102-000000.bbbbbbbb.bak.session_meta.json",
            "unrelated.bak",
        ];
        for name in names {
            fs::write(temp_dir.path().join(name), b"backup").expect("write backup");
        }

        let backups = list_backup_files(temp_dir.path()).expect("list backups");
        let listed: BTreeSet<String> = backups
            .iter()
            .filter_map(|path| path.file_name().and_then(|value| value.to_str()))
            .map(ToString::to_string)
            .collect();

        assert_eq!(listed.len(), 2);
        assert!(listed.contains("state_5.sqlite.pre-sync.20260101-000000.aaaaaaaa.bak"));
        assert!(listed.contains("state_6.sqlite.manual.20260102-000000.bbbbbbbb.bak"));
    }

    #[test]
    fn legacy_hint_reports_only_ai_toolbox_legacy_sessions() {
        let env = TestCodexHistory::new();
        create_threads_db(
            &env.root.join("state_5.sqlite"),
            &[
                Some("ai-toolbox-gateway"),
                Some("ai-toolbox-gateway"),
                Some("aitb-launch-abc"),
                Some("openai"),
                Some("new-provider"),
            ],
        );
        write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "aaaaaaaa-1111-1111-1111-111111111111",
            Some("ai-toolbox-gateway"),
            false,
        );
        write_rollout(
            &env.root,
            "archived_sessions/2026/05/21",
            "bbbbbbbb-1111-1111-1111-111111111111",
            Some("aitb-launch-abc"),
            true,
        );
        write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "cccccccc-1111-1111-1111-111111111111",
            Some("openai"),
            false,
        );

        let hint = get_legacy_hint(&env.root).expect("hint");

        assert!(hint.detected);
        assert_eq!(hint.thread_count, 3);
        // The hint is database-only: rollout files are never scanned, so a
        // Codex home with no sessions/ directory still reports the same counts.
        assert_eq!(hint.session_file_count, 0);
        assert_eq!(
            hint.provider_ids,
            vec![
                "ai-toolbox-gateway".to_string(),
                "aitb-launch-abc".to_string()
            ]
        );
    }

    #[test]
    fn sync_updates_provider_only_and_preserves_existing_models() {
        let env = TestCodexHistory::new();
        env.create_db(true);
        let session_path = env.write_session(
            "11111111-1111-1111-1111-111111111111",
            "old-provider",
            "gpt-old",
        );

        let status = get_status(&env.root).expect("status");
        assert_eq!(status.model_mismatch_threads, 0);
        assert!(status.has_work);

        let result = sync(&env.root, None).expect("sync");

        assert_eq!(result.updated_thread_rows, 2);
        assert_eq!(result.updated_session_files, 1);
        assert!(result.backup_path.contains("pre-sync"));
        let conn = Connection::open(env.root.join("state_5.sqlite")).expect("open db");
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT model_provider, model FROM threads WHERE archived = 0 ORDER BY id")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            vec![
                ("new-provider".to_string(), "gpt-old".to_string()),
                ("new-provider".to_string(), "gpt-old".to_string()),
                ("new-provider".to_string(), "gpt-other".to_string()),
            ]
        );
        let content = fs::read_to_string(session_path).expect("read session");
        let first_line = content.lines().next().unwrap();
        assert!(first_line.contains("new-provider"));
        assert!(first_line.contains("gpt-old"));
        assert!(!first_line.contains("gpt-new"));
        assert!(content.contains("hello"));
    }

    #[test]
    fn sync_legacy_schema_updates_provider_only() {
        let env = TestCodexHistory::new();
        env.create_db(false);

        let result = sync(&env.root, None).expect("sync legacy");

        assert_eq!(result.updated_thread_rows, 1);
        let conn = Connection::open(env.root.join("state_5.sqlite")).expect("open db");
        let provider: String = conn
            .query_row("SELECT model_provider FROM threads", [], |row| row.get(0))
            .expect("provider");
        assert_eq!(provider, "new-provider");
    }

    #[test]
    fn status_ignores_model_only_differences() {
        let env = TestCodexHistory::new();
        let conn = Connection::open(env.root.join("state_5.sqlite")).expect("open db");
        conn.execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                model TEXT,
                title TEXT,
                updated_at INTEGER,
                archived INTEGER
            );
            INSERT INTO threads (id, model_provider, model, title, updated_at, archived)
            VALUES ('22222222-2222-2222-2222-222222222222', 'new-provider', 'gpt-old', 'Model old', 1700000100, 0);
            "#,
        )
        .expect("create db");
        env.write_session(
            "22222222-2222-2222-2222-222222222222",
            "new-provider",
            "gpt-old",
        );
        fs::write(
            env.root.join("session_index.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({
                    "id": "22222222-2222-2222-2222-222222222222",
                    "thread_name": "Model old",
                    "updated_at": "2026-05-21T00:00:00Z"
                })
            ),
        )
        .expect("write index");

        let status = get_status(&env.root).expect("status");

        assert_eq!(status.provider_mismatch_threads, 0);
        assert_eq!(status.model_mismatch_threads, 0);
        assert_eq!(status.session_meta_mismatch_count, 0);
        assert!(!status.has_work);
    }

    #[test]
    fn official_auth_without_config_provider_uses_official_fallback() {
        let env = TestCodexHistory::new();
        fs::write(env.root.join("config.toml"), "model = \"gpt-new\"\n").expect("write config");
        fs::write(
            env.root.join("auth.json"),
            serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": "access-token",
                    "refresh_token": "refresh-token"
                }
            })
            .to_string(),
        )
        .expect("write auth");
        env.create_db(false);

        let status = get_status(&env.root).expect("status");

        assert_eq!(status.current_provider, OFFICIAL_MODEL_PROVIDER_ID);
        assert!(status.has_work);
    }

    #[test]
    fn api_key_auth_without_config_provider_does_not_use_official_fallback() {
        let env = TestCodexHistory::new();
        fs::write(env.root.join("config.toml"), "model = \"gpt-new\"\n").expect("write config");
        fs::write(
            env.root.join("auth.json"),
            serde_json::json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "sk-test"
            })
            .to_string(),
        )
        .expect("write auth");
        env.create_db(false);

        let error = get_status(&env.root).expect_err("status should fail");

        assert!(error.contains("Could not find model_provider"));
    }

    #[test]
    fn sqlite_locked_error_message_is_actionable() {
        let message =
            format_sqlite_locked_error("Failed to inspect Codex threads table: database is locked");
        assert!(message.contains("the Codex state database is locked"));
        assert!(message.contains("Finish the current Codex response/session"));
        assert!(message.contains("database is locked"));
        assert!(is_sqlite_locked_error(
            "Failed to inspect Codex threads table: database is locked"
        ));
    }

    #[test]
    fn retry_sqlite_operation_recovers_from_transient_lock() {
        let mut calls = 0usize;
        let result = retry_sqlite_operation(|| {
            calls += 1;
            if calls < 3 {
                Err("Failed to inspect Codex threads table: database is locked".to_string())
            } else {
                Ok(42)
            }
        })
        .expect("retry should recover");
        assert_eq!(result.value, 42);
        assert_eq!(result.attempts, 3);
        assert!(result.lock_wait_ms > 0);
    }

    #[test]
    fn restore_latest_restores_database_and_session_metadata() {
        let env = TestCodexHistory::new();
        env.create_db(true);
        let session_path = env.write_session(
            "11111111-1111-1111-1111-111111111111",
            "old-provider",
            "gpt-old",
        );
        backup(&env.root, "manual").expect("backup");
        sync(&env.root, None).expect("sync");

        let restored = restore_latest(&env.root).expect("restore");

        assert!(restored.restored_backup_path.contains("pre-sync"));
        let conn = Connection::open(env.root.join("state_5.sqlite")).expect("open db");
        let provider: String = conn
            .query_row(
                "SELECT model_provider FROM threads WHERE id = '11111111-1111-1111-1111-111111111111'",
                [],
                |row| row.get(0),
            )
            .expect("provider");
        assert_eq!(provider, "old-provider");
        let first_line = fs::read_to_string(session_path)
            .expect("read session")
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert!(first_line.contains("old-provider"));
        assert!(restored.safety_backup_path.contains("pre-restore"));
    }

    #[test]
    fn sync_and_restore_preserve_rollout_mtime_for_plain_and_compressed_files() {
        let env = TestCodexHistory::new();
        env.create_db(false);
        let plain = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "11111111-1111-1111-1111-111111111111",
            Some("old-provider"),
            false,
        );
        let compressed = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            "22222222-2222-2222-2222-222222222222",
            Some("old-provider"),
            true,
        );
        let stamp = 1_600_000_000_u64;
        let expected_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(stamp);
        set_rollout_mtime(&plain, stamp);
        set_rollout_mtime(&compressed, stamp);

        let result = sync(&env.root, None).expect("sync");

        assert_eq!(result.updated_session_files, 2);
        assert_eq!(provider_of(&plain).as_deref(), Some("new-provider"));
        assert_eq!(provider_of(&compressed).as_deref(), Some("new-provider"));
        assert_eq!(rollout_mtime(&plain), expected_mtime);
        assert_eq!(rollout_mtime(&compressed), expected_mtime);

        restore_latest(&env.root).expect("restore");

        assert_eq!(provider_of(&plain).as_deref(), Some("old-provider"));
        assert_eq!(provider_of(&compressed).as_deref(), Some("old-provider"));
        assert_eq!(rollout_mtime(&plain), expected_mtime);
        assert_eq!(rollout_mtime(&compressed), expected_mtime);
    }

    #[test]
    fn sync_rewrites_every_session_meta_line_and_restore_puts_them_back() {
        let env = TestCodexHistory::new();
        env.create_db(false);
        let dir = env.root.join("sessions/2026/05/21");
        let path = dir.join("rollout-2026-05-21T00-00-00-multi.jsonl");
        let meta_line = |provider: &str| {
            serde_json::json!({
                "type": "session_meta",
                "payload": {
                    "id": "11111111-1111-1111-1111-111111111111",
                    "cwd": "/tmp/project",
                    "model_provider": provider,
                    "model": "gpt-old"
                }
            })
            .to_string()
        };
        let response_line = serde_json::json!({
            "type": "response_item",
            "payload": {"type": "message", "role": "user", "content": "hello"}
        })
        .to_string();
        fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n",
                meta_line("old-provider"),
                response_line,
                meta_line("old-provider")
            ),
        )
        .expect("write resumed rollout");

        sync(&env.root, None).expect("sync");
        assert_eq!(
            session_meta_providers(&path),
            vec!["new-provider".to_string(), "new-provider".to_string()]
        );

        restore_latest(&env.root).expect("restore");
        assert_eq!(
            session_meta_providers(&path),
            vec!["old-provider".to_string(), "old-provider".to_string()]
        );
    }

    #[test]
    fn sync_dedupes_plain_and_compressed_rollout_siblings() {
        let env = TestCodexHistory::new();
        env.create_db(false);
        let id = "11111111-1111-1111-1111-111111111111";
        let plain = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            id,
            Some("old-provider"),
            false,
        );
        let compressed = write_rollout(
            &env.root,
            "sessions/2026/05/21",
            id,
            Some("old-provider"),
            true,
        );
        assert_eq!(
            plain.with_file_name(format!(
                "{}.zst",
                plain.file_name().unwrap().to_string_lossy()
            )),
            compressed
        );
        assert_eq!(get_status(&env.root).expect("status").session_file_count, 1);

        let result = sync(&env.root, None).expect("sync");

        assert_eq!(result.updated_session_files, 1);
        assert_eq!(provider_of(&plain).as_deref(), Some("new-provider"));
        // The sibling archive is never the authoritative copy and is left alone.
        assert_eq!(provider_of(&compressed).as_deref(), Some("old-provider"));
    }

    #[test]
    fn read_connection_recovers_wal_database_without_shm() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let source_path = temp_dir.path().join("source.sqlite");
        let source = Connection::open(&source_path).expect("open source db");
        source
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT);
                 INSERT INTO threads (id, model_provider) VALUES ('t1', 'old-provider');",
            )
            .expect("seed wal db");

        let copy_dir = temp_dir.path().join("copy");
        fs::create_dir_all(&copy_dir).expect("create copy dir");
        let copied_path = copy_dir.join("state_5.sqlite");
        fs::copy(&source_path, &copied_path).expect("copy main db");
        let source_wal = PathBuf::from(format!("{}-wal", source_path.to_string_lossy()));
        let copied_wal = PathBuf::from(format!("{}-wal", copied_path.to_string_lossy()));
        assert!(source_wal.exists(), "wal file should exist while open");
        fs::copy(&source_wal, &copied_wal).expect("copy wal");
        let copied_shm = PathBuf::from(format!("{}-shm", copied_path.to_string_lossy()));
        assert!(!copied_shm.exists(), "copy deliberately lacks -shm");

        let counts = read_provider_thread_counts(&copied_path).expect("read wal db without shm");

        assert_eq!(counts.get(&Some("old-provider".to_string())), Some(&1));
        drop(source);
    }

    #[cfg(windows)]
    #[test]
    fn legacy_hint_skips_windows_wsl_unc_paths() {
        for path in [
            r"\\wsl.localhost\Ubuntu\home\me\.codex",
            r"\\wsl$\Ubuntu\home\me\.codex",
        ] {
            let hint = get_legacy_hint(Path::new(path)).expect("hint should not touch disk");
            assert!(!hint.detected);
            assert_eq!(hint.thread_count, 0);
            assert_eq!(hint.session_file_count, 0);
        }
    }
}
