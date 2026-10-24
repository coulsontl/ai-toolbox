use super::types::{
    GatewaySessionImportCli, GatewaySessionUsageImportInput, GatewaySessionUsageImportResult,
    GatewayUsageRecordedEvent, GatewayUsageTool, SessionUsageGranularity, SessionUsageMetadata,
};
use super::usage_parser::TokenUsage;
use super::usage_stats::{calculate_session_costs, format_decimal_cost};
use crate::db::SqliteDbState;
use chrono::Utc;
use futures_util::FutureExt;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use walkdir::WalkDir;

mod antigravity;
mod cmdcode;
mod codebuddy;
mod cost_reconciliation;
mod desktop;
mod devin;
mod dsh;
mod grok;
mod hermes;
mod kimi;
mod minimax;
mod musecode;
mod open_claw;
mod open_code;
mod parsers;
mod pi;
mod prime_agent;
mod qoder_cli;
mod qoder_ide;
mod qwencode;
mod reconciliation;
mod workbuddy_family;
mod zcode;
pub(super) fn mark_archived_contributions(conn: &Connection, cutoff: i64) -> Result<(), String> {
    reconciliation::mark_archived(conn, cutoff)
}

const SYNC_INTERVAL: Duration = Duration::from_secs(60);
const SESSION_SETTLE_SECONDS: i64 = 3;
const PROXY_MATCH_START_GRACE_SECONDS: i64 = 10;
const PROXY_MATCH_END_GRACE_SECONDS: i64 = 30;

#[derive(Clone, Debug)]
struct SessionUsageRecord {
    metadata: SessionUsageMetadata,
    request_id: String,
    legacy_request_ids: Vec<String>,
    cli_key: GatewayUsageTool,
    model: String,
    usage: TokenUsage,
    created_at: i64,
    session_id: String,
    reported_cost_usd: Option<String>,
    /// Working directory the CLI ran in, when the transcript records one. Proxy
    /// rows never carry it; usage-only tools without a readable cwd stay `None`
    /// and land in the "unknown project" bucket.
    project_path: Option<String>,
    /// Credits the provider billed for this call, when the transcript reports
    /// them. Only Qoder does today: its built-in routing writes zero tokens to
    /// disk and records Credits instead, so this is the only usage measure
    /// available there. Stored for every tool but not rendered yet.
    credits: Option<f64>,
    /// How long the model call took, when the transcript carries timestamps
    /// that bracket it. This is an estimate derived from local events, never a
    /// gateway HTTP measurement; `None` means the transcript has no usable
    /// start/end pair.
    duration_ms: Option<u64>,
}

impl SessionUsageRecord {
    fn legacy_fingerprint(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::json!([
                    self.model,
                    self.created_at,
                    self.usage.input_tokens.unwrap_or(0),
                    self.usage.output_tokens.unwrap_or(0),
                    self.usage.cache_read_tokens.unwrap_or(0),
                    self.usage.cache_creation_tokens.unwrap_or(0),
                    self.reported_cost_usd
                ])
                .to_string()
                .as_bytes()
            )
        )
    }
    fn fingerprint(&self) -> String {
        let value = serde_json::json!([
            self.model,
            self.created_at,
            self.usage.input_tokens.unwrap_or(0),
            self.usage.output_tokens.unwrap_or(0),
            self.usage.cache_read_tokens.unwrap_or(0),
            self.usage.cache_creation_tokens.unwrap_or(0),
            self.reported_cost_usd,
            self.metadata,
            self.project_path,
            self.credits,
            self.duration_ms,
        ]);
        format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
    }

    fn extra_tokens(&self) -> u64 {
        self.metadata
            .reported_total_tokens
            .unwrap_or(0)
            .saturating_sub(self.usage.total_tokens().unwrap_or(0))
    }

    fn call_count(&self) -> u64 {
        self.metadata.call_count.unwrap_or_else(|| {
            u64::from(self.metadata.granularity == SessionUsageGranularity::Request)
        })
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct ImportedRecord {
    fingerprint: String,
    envelope_id: Option<String>,
    matched_proxy_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accounting: Option<reconciliation::Contribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded_cost: Option<cost_reconciliation::RecordedCost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_repair: Option<cost_reconciliation::CostRepair>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    retired: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct SourceState {
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_reconciliation_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cost_reconciliation_sources: Vec<String>,
    cumulative_usage: Option<hermes::Snapshot>,
    parser_revision: u32,
    modified_nanos: u64,
    size: u64,
    pending: bool,
    records: BTreeMap<String, ImportedRecord>,
    codex_snapshots: Vec<parsers::CodexSnapshot>,
}

fn sync_mutex() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// A source that keeps failing for the same reason (a Codex child waiting for a
/// rollout that never lands, an unreadable transcript) would otherwise log on
/// every sync interval without ever changing state. Remember the last message
/// per source so a repeat stays quiet while a different failure still surfaces.
#[derive(Default)]
struct SourceFailureLog {
    seen: std::sync::Mutex<HashMap<String, String>>,
}

impl SourceFailureLog {
    /// Returns `true` the first time this exact failure is reported for a source.
    fn should_log(&self, source_id: &str, message: &str) -> bool {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if seen.get(source_id).is_some_and(|last| last == message) {
            return false;
        }
        seen.insert(source_id.to_string(), message.to_string());
        true
    }

    /// A source that imported again (or vanished) reports its next failure afresh.
    fn clear(&self, source_id: &str) {
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(source_id);
    }
}

fn source_failures() -> &'static SourceFailureLog {
    static LOG: OnceLock<SourceFailureLog> = OnceLock::new();
    LOG.get_or_init(SourceFailureLog::default)
}

/// Phase labels a rebuild reports to the UI. They mirror the steps of
/// [`rebuild_sources`] so a slow step is named rather than guessed from a
/// spinner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRebuildPhase {
    BackingUp,
    Clearing,
    ScanningFiles,
    ImportingFiles,
    ReconcileLateRows,
    RetireDesktopRecords,
    ReconcileCosts,
    RollupAndPrune,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionRebuildProgress {
    pub phase: SessionRebuildPhase,
    /// Files found by the scan; 0 until `ScanningFiles` finishes.
    pub total_files: u64,
    /// Files already visited by the import loop.
    pub processed_files: u64,
    pub inserted_records: u64,
    pub updated_records: u64,
    pub skipped_records: u64,
    pub failed_files: u64,
    /// RFC 3339. The UI renders elapsed time from it instead of trusting its
    /// own clock, so a reloaded page still shows the true duration.
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

impl SessionRebuildProgress {
    fn new() -> Self {
        Self {
            phase: SessionRebuildPhase::BackingUp,
            total_files: 0,
            processed_files: 0,
            inserted_records: 0,
            updated_records: 0,
            skipped_records: 0,
            failed_files: 0,
            started_at: Utc::now().to_rfc3339(),
            finished_at: None,
            error: None,
        }
    }

    fn is_finished(&self) -> bool {
        matches!(
            self.phase,
            SessionRebuildPhase::Done | SessionRebuildPhase::Failed
        )
    }
}

/// The one rebuild the app allows at a time.
///
/// A rebuild already serializes on [`sync_mutex`], so tracking a single task
/// matches what can actually run. Keeping it in memory is deliberate: a
/// restart means the task is gone, and the UI must not show a stale progress
/// it cannot reconcile with the database.
#[derive(Default)]
struct RebuildTask {
    progress: Option<SessionRebuildProgress>,
}

fn rebuild_task() -> &'static std::sync::Mutex<RebuildTask> {
    static TASK: OnceLock<std::sync::Mutex<RebuildTask>> = OnceLock::new();
    TASK.get_or_init(|| std::sync::Mutex::new(RebuildTask::default()))
}

fn update_rebuild_progress(update: impl FnOnce(&mut SessionRebuildProgress)) {
    let guard = rebuild_task();
    let mut task = guard
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(progress) = task.progress.as_mut() {
        // The finished snapshot is the record the UI reports to the user. The
        // 60-second background sync shares `sync_sources` and would otherwise
        // keep adding to its counters after the rebuild is over.
        if progress.is_finished() {
            return;
        }
        update(progress);
    }
}

/// Claims the single rebuild slot.
///
/// Split from [`start_rebuild`] so the overlap guard is testable without
/// spawning the async task: a second start must be rejected while the first
/// is still running, and accepted again once it finished.
fn claim_rebuild_slot(task: &mut RebuildTask) -> Result<(), String> {
    if task
        .progress
        .as_ref()
        .is_some_and(|progress| !progress.is_finished())
    {
        return Err("A session usage rebuild is already running".to_string());
    }
    task.progress = Some(SessionRebuildProgress::new());
    Ok(())
}

/// Snapshot of the current rebuild, or of a finished one that the UI has not
/// reported yet.
///
/// Reading the finished snapshot consumes it: the view unmounts on a tab
/// switch, and a snapshot that stayed readable would re-report "rebuild
/// finished" on every later visit. A running task is never consumed.
pub fn take_rebuild_progress() -> Option<SessionRebuildProgress> {
    let guard = rebuild_task();
    let mut task = guard
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let progress = task.progress.as_ref()?;
    if progress.is_finished() {
        task.progress.take()
    } else {
        task.progress.clone()
    }
}

/// Starts a rebuild in the background and returns immediately.
///
/// The command used to await the whole rebuild, which left the toolbar
/// spinning for tens of minutes with no phase, no progress and no way to
/// leave the page. The returned result is the progress record to poll; the
/// final figures land there when the task finishes.
pub fn start_rebuild(
    db: SqliteDbState,
    input: GatewaySessionUsageImportInput,
) -> Result<(), String> {
    {
        let guard = rebuild_task();
        let mut task = guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        claim_rebuild_slot(&mut task)?;
    }
    let app_handle = tauri::async_runtime::spawn(async move {
        // A panic must still free the single slot: `claim_rebuild_slot` only
        // accepts a finished progress record, and nothing else would ever
        // write one, so an unwind here would keep the slot claimed until the
        // app restarts.
        let result = AssertUnwindSafe(rebuild_session_usage(db.clone(), input))
            .catch_unwind()
            .await;
        match result {
            Ok(Ok(import_result)) => {
                update_rebuild_progress(|progress| {
                    progress.phase = SessionRebuildPhase::Done;
                    progress.inserted_records = import_result.inserted_records;
                    progress.updated_records = import_result.updated_records;
                    progress.skipped_records = import_result.skipped_records;
                    progress.failed_files = import_result.failed_files;
                    progress.processed_files = progress.total_files;
                    progress.finished_at = Some(Utc::now().to_rfc3339());
                });
            }
            Ok(Err(error)) => {
                update_rebuild_progress(|progress| {
                    progress.phase = SessionRebuildPhase::Failed;
                    progress.error = Some(error);
                    progress.finished_at = Some(Utc::now().to_rfc3339());
                });
            }
            Err(panic) => {
                let message = panic_payload_message(panic.as_ref());
                update_rebuild_progress(|progress| {
                    *progress = panicked_rebuild_progress(message);
                });
            }
        }
    });
    // The task owns its own handle; detaching is the point of this command.
    drop(app_handle);
    Ok(())
}

/// What a panicking rebuild task died of, for its failure record.
fn panic_payload_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown cause".to_string()
    }
}

/// A terminal record for a rebuild that died from a panic.
fn panicked_rebuild_progress(message: String) -> SessionRebuildProgress {
    let mut progress = SessionRebuildProgress::new();
    progress.phase = SessionRebuildPhase::Failed;
    progress.error = Some(format!("Session usage rebuild panicked: {message}"));
    progress.finished_at = Some(Utc::now().to_rfc3339());
    progress
}

/// Discard everything this collector derived for `cli_key` and re-import it.
///
/// Imported usage is a projection of transcripts that are still on disk, so a
/// parser fix can be applied by dropping the projection and rebuilding it —
/// which is the only way to repair rows already written under an older parser,
/// since the collector never rewrites a row it considers unchanged.
///
/// **Proxy rows are never touched**: they describe real requests the gateway
/// served and have no on-disk source to rebuild from. Only rows this collector
/// owns are removed — `data_source = 'session'` detail rows, the matching
/// `provider_id = 'session'` rollups, and the per-file ledger that decides what
/// has already been read.
///
/// The rebuild is guarded by a database backup because the transcripts are the
/// only remaining source: if a CLI has since deleted a session file, that
/// session's history is gone from the new import too.
pub async fn rebuild_session_usage(
    db: SqliteDbState,
    input: GatewaySessionUsageImportInput,
) -> Result<GatewaySessionUsageImportResult, String> {
    if !super::settings::load_settings_from_sqlite_state(&db)?.session_usage_enabled {
        return Ok(GatewaySessionUsageImportResult::default());
    }
    let guard = sync_mutex().lock().await;
    let cli_keys = import_cli_keys(input.cli_key);
    let sources = {
        let db = db.clone();
        let cli_keys = cli_keys.clone();
        tauri::async_runtime::spawn_blocking(move || {
            cli_keys
                .into_iter()
                .flat_map(|cli_key| {
                    default_session_roots(&db, cli_key)
                        .into_iter()
                        .map(move |root| (cli_key, root))
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| format!("Failed to resolve session roots for rebuild: {error}"))?
    };
    let result = {
        let db = db.clone();
        let now = Utc::now().timestamp();
        tauri::async_runtime::spawn_blocking(move || rebuild_sources(&db, &sources, &cli_keys, now))
            .await
            .map_err(|error| format!("Failed to rebuild local session usage: {error}"))?
    };
    drop(guard);
    result
}

/// Backup, clear and re-import for an explicit source list.
///
/// Split from [`rebuild_session_usage`] so a test can rebuild against fixture
/// roots instead of the machine's real CLI directories.
///
/// The rebuild refuses to run against an empty source set for a CLI that
/// already has session usage: `clear_imported_usage` deletes by `cli_key`
/// regardless of what the scan found, so a root that resolved to nothing (the
/// CLI home points at an unmounted WSL, the sessions directory was never
/// created) would wipe that CLI's imported history and import nothing back.
fn rebuild_sources(
    db: &SqliteDbState,
    sources: &[(GatewayUsageTool, PathBuf)],
    cli_keys: &[GatewayUsageTool],
    now: i64,
) -> Result<GatewaySessionUsageImportResult, String> {
    // Guard on the files the scan actually finds, not on the source list: a
    // root can resolve to a path that exists but holds nothing the parser
    // reads (unmounted WSL root, never-created sessions directory) — clearing
    // by cli_key regardless would wipe that CLI's imported history and import
    // nothing back.
    update_rebuild_progress(|progress| progress.phase = SessionRebuildPhase::ScanningFiles);
    let scanned: Vec<_> = sources
        .iter()
        .flat_map(|(cli_key, root)| {
            session_files(*cli_key, root)
                .into_iter()
                .map(move |path| (*cli_key, path))
        })
        .collect();
    if scanned.is_empty() {
        let existing = count_imported_usage(db, cli_keys)?;
        if existing > 0 {
            return Err(format!(
                "No session files found for the selected CLI(s), but {existing} imported \
                 session usage record(s) exist. Refusing to rebuild: an empty source set would \
                 delete them. Check the CLI's session directory (root override, WSL mount) and \
                 retry."
            ));
        }
        // Nothing to clear and nothing to import: the no-op keeps its meaning
        // rather than failing on a fresh installation.
        return Ok(GatewaySessionUsageImportResult::default());
    }
    update_rebuild_progress(|progress| {
        progress.total_files = scanned.len() as u64;
        progress.phase = SessionRebuildPhase::BackingUp;
    });
    create_rebuild_backup(db)?;
    update_rebuild_progress(|progress| progress.phase = SessionRebuildPhase::Clearing);
    clear_imported_usage(db, cli_keys)?;
    sync_sources(db, sources, now, true)
}

/// How many session-sourced detail rows the given CLIs currently hold.
fn count_imported_usage(db: &SqliteDbState, cli_keys: &[GatewayUsageTool]) -> Result<u64, String> {
    db.with_conn(|conn| {
        let mut total: u64 = 0;
        for cli_key in cli_keys {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM proxy_request_logs
                     WHERE data_source = 'session' AND app_type = ?1",
                    params![cli_key.as_str()],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            total += count.max(0) as u64;
        }
        Ok(total)
    })
}

fn create_rebuild_backup(db: &SqliteDbState) -> Result<(), String> {
    db.with_conn(|conn| {
        let db_path = db.db_path();
        if db_path.as_os_str() == ":memory:" {
            // An in-memory database (tests) has no file to back up and nothing
            // durable to protect.
            return Ok(());
        }
        let parent = db_path.parent().ok_or_else(|| {
            format!(
                "Failed to resolve SQLite database parent for {}",
                db_path.display()
            )
        })?;
        let backup_dir = parent.join(crate::db::sqlite_state::SQLITE_MIGRATION_BACKUP_DIR);
        fs::create_dir_all(&backup_dir).map_err(|error| {
            format!(
                "Failed to create rebuild backup directory {}: {error}",
                backup_dir.display()
            )
        })?;
        let backup_path = backup_dir.join(format!(
            "ai-toolbox-session-usage-rebuild-{}.db",
            Utc::now().format("%Y%m%d-%H%M%S")
        ));
        crate::db::backup::backup_to_path(conn, &backup_path)?;
        log::info!(
            "Created pre-rebuild SQLite backup at {}",
            backup_path.display()
        );
        Ok(())
    })
}

fn clear_imported_usage(db: &SqliteDbState, cli_keys: &[GatewayUsageTool]) -> Result<(), String> {
    db.with_conn_mut(|conn| {
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        for cli_key in cli_keys {
            let app_type = cli_key.as_str();
            transaction
                .execute(
                    "DELETE FROM proxy_request_logs
                     WHERE data_source = 'session' AND app_type = ?1",
                    params![app_type],
                )
                .map_err(|error| error.to_string())?;
            // Rollups carry no `data_source`; the collector's own rows are the
            // ones filed under the reserved `session` provider id.
            transaction
                .execute(
                    "DELETE FROM usage_daily_rollups
                     WHERE provider_id = 'session' AND app_type = ?1",
                    params![app_type],
                )
                .map_err(|error| error.to_string())?;
            // Preserve each ledger record's matched-proxy id across the clear:
            // the proxy detail rows may already be rolled up and pruned, and
            // `find_matching_proxy` only searches live rows. Without the
            // preserved ids a rebuild re-imports calls the (now archived)
            // proxy already paid for — double counting in the daily rollups.
            //
            // A state row is `<cli>:<identity>` and holds many records; the
            // marker re-groups them by source so the re-import finds each
            // match under the file it came from.
            let mut preserved: HashMap<String, serde_json::Map<String, serde_json::Value>> =
                HashMap::new();
            {
                let mut statement = transaction
                    .prepare(
                        "SELECT s.id, json(s.data)
                         FROM gateway_session_usage_state s
                         WHERE s.id LIKE ?1",
                    )
                    .map_err(|error| error.to_string())?;
                let rows = statement
                    .query_map(params![format!("{app_type}:%")], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|error| error.to_string())?;
                for row in rows {
                    let (id, data) = row.map_err(|error| error.to_string())?;
                    let Ok(state) = serde_json::from_str::<SourceState>(&data) else {
                        continue;
                    };
                    for (record_id, record) in state.records {
                        if let Some(proxy_id) = record.matched_proxy_id {
                            preserved
                                .entry(id.clone())
                                .or_default()
                                .insert(
                                    record_id,
                                    serde_json::json!({ "matched_proxy_id": proxy_id }),
                                );
                        }
                    }
                }
            }
            transaction
                .execute(
                    "DELETE FROM gateway_session_usage_state WHERE id LIKE ?1",
                    params![format!("{app_type}:%")],
                )
                .map_err(|error| error.to_string())?;
            let mut insert = transaction
                .prepare(
                    "INSERT OR REPLACE INTO gateway_session_usage_state (id, data, created_at, updated_at)
                     VALUES (?1, jsonb(?2), ?3, ?3)",
                )
                .map_err(|error| error.to_string())?;
            let timestamp = Utc::now().to_rfc3339();
            for (source_id, records) in preserved {
                // The marker is a minimal valid `SourceState` holding only the
                // matched-proxy facts. `sync_sources` folds its records into
                // the file's fresh state, so the re-import re-derives the
                // match (`retained_proxy_id` handles the archived-proxy case)
                // instead of inserting a second detail row.
                insert
                    .execute(
                        params![
                            format!("archived-match:{source_id}"),
                            serde_json::json!({ "records": records }).to_string(),
                            timestamp
                        ],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())
    })
}

pub async fn import_session_usage(
    db: SqliteDbState,
    input: GatewaySessionUsageImportInput,
) -> Result<GatewaySessionUsageImportResult, String> {
    // Single gating point shared by the Tauri command and the 60s background
    // scheduler. Skipping here leaves the per-file ledger snapshots frozen, so
    // re-enabling later rescans the files and picks up everything recorded
    // while the toggle was off (as long as the CLI files still exist).
    if !super::settings::load_settings_from_sqlite_state(&db)?.session_usage_enabled {
        return Ok(GatewaySessionUsageImportResult::default());
    }
    // Manual sync, page activation and the background timer share one writer.
    let guard = sync_mutex().lock().await;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let sources = import_cli_keys(input.cli_key)
            .into_iter()
            .flat_map(|cli_key| {
                default_session_roots(&db, cli_key)
                    .into_iter()
                    .map(move |root| (cli_key, root))
            })
            .collect::<Vec<_>>();
        sync_sources(&db, &sources, Utc::now().timestamp(), false)
    })
    .await
    .map_err(|error| format!("Failed to sync local session usage: {error}"))?;
    drop(guard);
    result
}

pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let Some(db) = app.try_state::<SqliteDbState>() else {
                return;
            };
            match import_session_usage(db.db().clone(), GatewaySessionUsageImportInput::default())
                .await
            {
                Ok(result) => notify_usage_changed(&app, &result),
                Err(error) => log::warn!("Local session usage sync failed: {error}"),
            }
        }
    });
}

pub(super) fn notify_usage_changed(
    app: &tauri::AppHandle,
    result: &GatewaySessionUsageImportResult,
) {
    let changed = result
        .inserted_records
        .saturating_add(result.updated_records);
    if changed == 0 {
        return;
    }
    let payload = GatewayUsageRecordedEvent {
        cli_key: None,
        trace_id: None,
        data_source: "session".to_string(),
        inserted_records: changed,
    };
    if let Err(error) = app.emit("usage-log-recorded", payload) {
        log::warn!("Failed to emit local session usage update: {error}");
    }
}

fn import_cli_keys(selection: GatewaySessionImportCli) -> Vec<GatewayUsageTool> {
    match selection {
        GatewaySessionImportCli::All => GatewayUsageTool::all(),
        GatewaySessionImportCli::Claude => vec![GatewayUsageTool::Claude],
        GatewaySessionImportCli::ClaudeDesktop => vec![GatewayUsageTool::ClaudeDesktop],
        GatewaySessionImportCli::Codex => vec![GatewayUsageTool::Codex],
        GatewaySessionImportCli::Grok => vec![GatewayUsageTool::Grok],
        GatewaySessionImportCli::Kimi => vec![GatewayUsageTool::Kimi],
        GatewaySessionImportCli::Gemini => vec![GatewayUsageTool::Gemini],
        GatewaySessionImportCli::Antigravity => vec![GatewayUsageTool::Antigravity],
        GatewaySessionImportCli::OpenCode => vec![GatewayUsageTool::OpenCode],
        GatewaySessionImportCli::Pi => vec![GatewayUsageTool::Pi],
        GatewaySessionImportCli::OhMyPi => vec![GatewayUsageTool::OhMyPi],
        GatewaySessionImportCli::OmoNative => vec![GatewayUsageTool::OmoNative],
        GatewaySessionImportCli::Dsh => vec![GatewayUsageTool::Dsh],
        GatewaySessionImportCli::Hermes => vec![GatewayUsageTool::Hermes],
        GatewaySessionImportCli::OpenClaw => vec![GatewayUsageTool::OpenClaw],
        GatewaySessionImportCli::KimiCli => vec![GatewayUsageTool::KimiCli],
        GatewaySessionImportCli::Zcode => vec![GatewayUsageTool::Zcode],
        GatewaySessionImportCli::Mimocode => vec![GatewayUsageTool::Mimocode],
        GatewaySessionImportCli::PrimeAgent => vec![GatewayUsageTool::PrimeAgent],
        GatewaySessionImportCli::Devin => vec![GatewayUsageTool::Devin],
        GatewaySessionImportCli::Musecode => vec![GatewayUsageTool::Musecode],
        GatewaySessionImportCli::Cmdcode => vec![GatewayUsageTool::Cmdcode],
        GatewaySessionImportCli::Qwencode => vec![GatewayUsageTool::Qwencode],
        GatewaySessionImportCli::Workbuddy => vec![GatewayUsageTool::Workbuddy],
        GatewaySessionImportCli::WorkbuddyAi => vec![GatewayUsageTool::WorkbuddyAi],
        GatewaySessionImportCli::Codebuddy => vec![GatewayUsageTool::Codebuddy],
        GatewaySessionImportCli::Minimax => vec![GatewayUsageTool::Minimax],
        GatewaySessionImportCli::QoderIde => vec![GatewayUsageTool::QoderIde],
        GatewaySessionImportCli::QoderIdeCn => vec![GatewayUsageTool::QoderIdeCn],
        GatewaySessionImportCli::QoderCli => vec![GatewayUsageTool::QoderCli],
        GatewaySessionImportCli::QoderCliCn => vec![GatewayUsageTool::QoderCliCn],
    }
}

fn default_session_roots(db: &SqliteDbState, cli_key: GatewayUsageTool) -> Vec<PathBuf> {
    use crate::coding::runtime_location::*;
    let location = match cli_key {
        GatewayUsageTool::Claude => get_claude_runtime_location_sync(db).ok(),
        GatewayUsageTool::Codex => get_codex_runtime_location_sync(db).ok(),
        GatewayUsageTool::Grok => get_grok_runtime_location_sync(db).ok(),
        GatewayUsageTool::Kimi => get_kimi_runtime_location_sync(db).ok(),
        GatewayUsageTool::Gemini => get_gemini_cli_runtime_location_sync(db).ok(),
        GatewayUsageTool::Antigravity => get_antigravity_runtime_location_sync(db).ok(),
        GatewayUsageTool::OpenCode => get_opencode_runtime_location_sync(db).ok(),
        GatewayUsageTool::Pi => get_pi_runtime_location_sync(db).ok(),
        GatewayUsageTool::OhMyPi => get_oh_my_pi_runtime_location_sync(db).ok(),
        GatewayUsageTool::OmoNative => get_omo_native_runtime_location_sync(db).ok(),
        GatewayUsageTool::OpenClaw => get_openclaw_runtime_location_sync(db).ok(),
        GatewayUsageTool::Zcode => get_zcode_runtime_location_sync(db).ok(),
        _ => None,
    };
    let mut roots = Vec::new();
    let (home_name, suffix) = match cli_key {
        GatewayUsageTool::Claude => (".claude", "projects"),
        GatewayUsageTool::Codex => (".codex", "sessions"),
        GatewayUsageTool::Grok => (".grok", "sessions"),
        GatewayUsageTool::Kimi => (".kimi-code", "sessions"),
        // Antigravity keeps the transcript mirror in `brain/` and the real
        // token usage in `conversations/*.db`, both beside each other in the
        // profile directory. Every root below is that profile directory; the
        // CLI, the desktop app and the IDE each keep their own.
        GatewayUsageTool::Antigravity => {
            if let Some(location) = &location {
                roots.push(location.host_path.clone());
            }
            if let Some(home) = dirs::home_dir() {
                for profile in ["antigravity-cli", "antigravity", "antigravity-ide"] {
                    let root = home.join(".gemini").join(profile);
                    if !roots.contains(&root) {
                        roots.push(root);
                    }
                }
            }
            return roots;
        }
        GatewayUsageTool::Gemini => (".gemini", "tmp"),
        GatewayUsageTool::Pi => {
            if let Some(location) = &location {
                if let Ok(root) = crate::coding::session_manager::resolve_pi_sessions_root(location)
                {
                    roots.push(root);
                }
            }
            return roots;
        }
        GatewayUsageTool::OhMyPi => (".omp/agent", "sessions"),
        GatewayUsageTool::OmoNative => (".omo/agent", "sessions"),
        GatewayUsageTool::OpenClaw => {
            if let Some(location) = location {
                roots.push(
                    location
                        .host_path
                        .parent()
                        .unwrap_or(&location.host_path)
                        .join("agents"),
                );
            }
            return roots;
        }
        GatewayUsageTool::Dsh => {
            if let Ok(info) = crate::coding::dsh::get_dsh_root_path_info_from_db(db) {
                roots.push(PathBuf::from(info.path).join("sessions"));
            }
            return roots;
        }
        GatewayUsageTool::Hermes => {
            if let Ok(info) = crate::coding::hermes::get_hermes_root_path_info_from_db(db) {
                roots.push(PathBuf::from(info.path));
            }
            return roots;
        }
        GatewayUsageTool::KimiCli => {
            if let Some(root) = std::env::var_os("KIMI_SHARE_DIR") {
                roots.push(PathBuf::from(root).join("sessions"));
            } else if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".kimi/sessions"));
            }
            return roots;
        }
        GatewayUsageTool::Zcode => {
            // The CLI keeps one SQLite database rather than a transcript tree,
            // so the "root" is the database itself and the resolver for it
            // lives in the session manager.
            if let Some(location) = location {
                if let Ok(path) =
                    crate::coding::session_manager::resolve_zcode_cli_db_path(&location)
                {
                    roots.push(path);
                }
            }
            return roots;
        }
        GatewayUsageTool::OpenCode => {
            if let Some(location) = location {
                if let Ok(root) =
                    crate::coding::session_manager::resolve_opencode_data_root(&location)
                {
                    roots.push(root);
                }
            }
            return roots;
        }
        GatewayUsageTool::ClaudeDesktop => {
            if let Ok(paths) =
                crate::coding::claude_desktop::config_writer::current_platform_paths()
            {
                if let Some(root) = paths.config_library_path.parent() {
                    roots.push(root.join("local-agent-mode-sessions"));
                }
            }
            roots.extend(desktop::official_session_roots());
            return roots;
        }
        GatewayUsageTool::Mimocode => {
            roots.extend(mimocode_db_paths());
            return roots;
        }
        GatewayUsageTool::PrimeAgent => {
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".prime/agent"));
            }
            return roots;
        }
        GatewayUsageTool::Devin => {
            if let Some(home) = dirs::home_dir() {
                let path = home.join(".local/share/devin/cli/sessions.db");
                if path.is_file() {
                    roots.push(path);
                }
            }
            return roots;
        }
        GatewayUsageTool::Musecode => {
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".local/share/muse/sessions"));
            }
            return roots;
        }
        GatewayUsageTool::Cmdcode => {
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".commandcode/projects"));
            }
            return roots;
        }
        GatewayUsageTool::Qwencode => {
            roots.push(qwencode::root());
            return roots;
        }
        GatewayUsageTool::Workbuddy => {
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".workbuddy/projects"));
            }
            return roots;
        }
        GatewayUsageTool::WorkbuddyAi => {
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".workbuddy-ai/projects"));
            }
            return roots;
        }
        GatewayUsageTool::Codebuddy => {
            if let Some(home) = dirs::home_dir() {
                roots.push(home.join(".codebuddy/projects"));
            }
            return roots;
        }
        GatewayUsageTool::Minimax => {
            if let Some(home) = dirs::home_dir() {
                let path = home.join(".minimax/v2/sqlite/runtime-state.sqlite");
                if path.is_file() {
                    roots.push(path);
                }
            }
            return roots;
        }
        // The two IDE editions keep one database each; both are read through
        // the same parser but filed under separate tool keys.
        GatewayUsageTool::QoderIde | GatewayUsageTool::QoderIdeCn => {
            roots.extend(qoder_ide::database_paths(cli_key));
            return roots;
        }
        // Both CLI editions write the same transcript shape under their own
        // config root; the desktop app shares the same tree, so its sessions
        // are covered here and never counted twice.
        GatewayUsageTool::QoderCli => {
            roots.push(qoder_cli_root("QODER_CONFIG_DIR", ".qoder"));
            return roots;
        }
        GatewayUsageTool::QoderCliCn => {
            roots.push(qoder_cli_root("QODERCN_CONFIG_DIR", ".qoder-cn"));
            return roots;
        }
    };
    if let Some(location) = location {
        if cli_key == GatewayUsageTool::Kimi {
            roots.push(location.host_path.join("store"));
        }
        roots.push(location.host_path.join(suffix));
        // Grok's per-request token counts live in `logs/unified.jsonl`, not in
        // the session mirror tree; the mirror is kept for older installs.
        if cli_key == GatewayUsageTool::Grok {
            roots.push(location.host_path.join("logs"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        let default = home.join(home_name).join(suffix);
        if !roots.contains(&default) {
            roots.push(default);
        }
        // Gemini CLI keeps a second tree beside `tmp/`: the desktop/IDE
        // integration writes its sessions under `.gemini/gemini-cli`. The
        // session file matcher accepts both roots because it keys on the
        // `session-*` basename, not on the parent directory.
        if cli_key == GatewayUsageTool::Gemini {
            let extended = home.join(".gemini").join("gemini-cli");
            if !roots.contains(&extended) {
                roots.push(extended);
            }
        }
    }
    if cli_key == GatewayUsageTool::Codex {
        let archives = roots
            .iter()
            .filter_map(|root| root.parent().map(|parent| parent.join("archived_sessions")))
            .collect::<Vec<_>>();
        roots.extend(archives);
    }
    roots
}

/// Qoder CLI's transcript tree, read from `<config dir>/projects`.
///
/// Both builds ship the same code and read their config directory from the
/// environment: the international CLI honours `QODER_CONFIG_DIR`, the CN build
/// `QODERCN_CONFIG_DIR` (no underscore — verified in both shipped binaries).
/// Without the variable they fall back to `~/.qoder` and `~/.qoder-cn`.
fn qoder_cli_root(variable: &str, default: &str) -> PathBuf {
    if let Some(root) = std::env::var_os(variable) {
        return PathBuf::from(root).join("projects");
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(default)
        .join("projects")
}

/// MiMoCode keeps an OpenCode-derived store under a per-platform data
/// directory: `mimocode.db`, or the newest channel database
/// `mimocode-<channel>.db`.
fn mimocode_db_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("MIMOCODE_HOME") {
        dirs.push(PathBuf::from(home).join("data"));
    } else if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg).join("mimocode"));
    } else {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("mimocode"));
        }
        if let Some(roaming) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(roaming).join("mimocode"));
        }
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".local/share/mimocode"));
        }
    }
    for dir in dirs {
        let default = dir.join("mimocode.db");
        if default.is_file() {
            return vec![default];
        }
        let mut channels = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.starts_with("mimocode-") && name.ends_with(".db")
                            })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        channels.sort_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        });
        if let Some(active) = channels.pop() {
            return vec![active];
        }
    }
    Vec::new()
}

fn source_identity(cli_key: GatewayUsageTool, path: &Path) -> String {
    if matches!(
        cli_key,
        GatewayUsageTool::Pi
            | GatewayUsageTool::OhMyPi
            | GatewayUsageTool::OmoNative
            | GatewayUsageTool::Dsh
            | GatewayUsageTool::Grok
            | GatewayUsageTool::Kimi
            | GatewayUsageTool::KimiCli
            | GatewayUsageTool::ClaudeDesktop
            | GatewayUsageTool::OpenClaw
            | GatewayUsageTool::Antigravity
            | GatewayUsageTool::PrimeAgent
            | GatewayUsageTool::QoderCli
            | GatewayUsageTool::QoderCliCn
    ) {
        // Generic basenames (audit, updates, wire, session) occur in many
        // directories. The file cursor is physical; invocation IDs are native.
        return format!(
            "file:{:x}",
            Sha256::digest(path.to_string_lossy().as_bytes())
        );
    }
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    if stem.starts_with("agent-") {
        if let Some(parent_session) = path
            .ancestors()
            .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "subagents"))
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
        {
            return format!("{parent_session}:{stem}");
        }
    }
    if cli_key == GatewayUsageTool::Codex && stem.len() >= 36 {
        if let Some(suffix) = stem.get(stem.len() - 36..) {
            if uuid::Uuid::parse_str(suffix).is_ok() {
                return suffix.to_string();
            }
        }
        // Codex compresses rollouts in place (`x.jsonl` -> `x.jsonl.zst`, the
        // original then deleted). `Path::file_stem` only strips the LAST
        // extension, so `rollout-...-<uuid>.jsonl.zst` yields a stem ending in
        // `.jsonl` and the 36-char tail is never the UUID. Strip the known
        // double extension first, or the identity drifts on every compression
        // cycle (old ledger rows orphaned, file re-scanned as a new source).
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if let Some(zst_stem) = file_name.strip_suffix(".jsonl.zst") {
            if zst_stem.len() >= 36 {
                if let Some(suffix) = zst_stem.get(zst_stem.len() - 36..) {
                    if uuid::Uuid::parse_str(suffix).is_ok() {
                        return suffix.to_string();
                    }
                }
            }
        }
    }
    if matches!(
        stem,
        "chat_history" | "context" | "wire" | "messages" | "transcript"
    ) {
        let parent = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("unknown");
        return format!("{parent}:{stem}");
    }
    stem.to_string()
}

/// Tools whose root is one SQLite database rather than a transcript tree.
fn reads_sqlite_database(cli_key: GatewayUsageTool) -> bool {
    matches!(
        cli_key,
        GatewayUsageTool::Zcode
            | GatewayUsageTool::Devin
            | GatewayUsageTool::Minimax
            | GatewayUsageTool::Mimocode
            | GatewayUsageTool::QoderIde
            | GatewayUsageTool::QoderIdeCn
    )
}

fn session_files(cli_key: GatewayUsageTool, root: &Path) -> Vec<PathBuf> {
    // These tools keep one SQLite database rather than a transcript tree, so
    // their "root" is that file. Without this the walk below silently returns
    // nothing and the tool looks like it has no usage.
    if reads_sqlite_database(cli_key) {
        return root
            .is_file()
            .then(|| vec![root.to_path_buf()])
            .unwrap_or_default();
    }
    if !root.is_dir() {
        return Vec::new();
    }
    let files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.into_path();
            let name = path.file_name()?.to_str()?;
            let extension = path.extension()?.to_str()?;
            let accepted = match cli_key {
                GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop => extension == "jsonl",
                // Codex rewrites cold rollouts as `<name>.jsonl.zst` and reads
                // them back transparently; both spellings carry the same usage.
                GatewayUsageTool::Codex => {
                    name.starts_with("rollout-")
                        && (extension == "jsonl" || name.ends_with(".jsonl.zst"))
                }
                GatewayUsageTool::Gemini => {
                    matches!(extension, "json" | "jsonl") && name.starts_with("session-")
                }
                GatewayUsageTool::Antigravity => antigravity::is_transcript(&path),
                GatewayUsageTool::Grok => {
                    (name == "updates.jsonl"
                        && !path
                            .components()
                            .any(|part| part.as_os_str() == "subagents"))
                        || grok::is_unified_log(&path)
                }
                GatewayUsageTool::Kimi | GatewayUsageTool::KimiCli => {
                    kimi::is_usage_file(cli_key, &path)
                }
                GatewayUsageTool::Pi | GatewayUsageTool::OhMyPi | GatewayUsageTool::OmoNative => {
                    extension == "jsonl"
                }
                GatewayUsageTool::Dsh => dsh::generation(&path).is_some(),
                GatewayUsageTool::Hermes => false,
                // A SQLite database, not a transcript tree; its reader is
                // chosen in `parse_file` and never reaches this walk.
                GatewayUsageTool::Zcode => false,
                GatewayUsageTool::OpenClaw => open_claw::is_transcript(&path),
                GatewayUsageTool::OpenCode => {
                    extension == "json"
                        && path.strip_prefix(root).ok().is_some_and(|relative| {
                            relative.starts_with(Path::new("storage").join("message"))
                        })
                }
                GatewayUsageTool::PrimeAgent => extension == "jsonl",
                GatewayUsageTool::Musecode => musecode::is_session_file(&path),
                GatewayUsageTool::Cmdcode => cmdcode::is_session_file(&path),
                GatewayUsageTool::Qwencode => qwencode::is_usage_file(&path),
                // Every `*.jsonl` under the project tree is a session, including
                // `<session>/subagents/*.jsonl` children.
                GatewayUsageTool::QoderCli | GatewayUsageTool::QoderCliCn => extension == "jsonl",
                GatewayUsageTool::Workbuddy
                | GatewayUsageTool::WorkbuddyAi
                | GatewayUsageTool::Codebuddy => extension == "jsonl",
                // SQLite tools are never reached by this walk.
                GatewayUsageTool::Devin
                | GatewayUsageTool::Minimax
                | GatewayUsageTool::Mimocode
                | GatewayUsageTool::QoderIde
                | GatewayUsageTool::QoderIdeCn => false,
            };
            accepted.then_some(path)
        })
        .collect();
    if cli_key == GatewayUsageTool::Dsh {
        dsh::select_generations(files)
    } else if cli_key == GatewayUsageTool::ClaudeDesktop {
        desktop::select_sources(files)
    } else if cli_key == GatewayUsageTool::OpenClaw {
        open_claw::canonical_files(files)
    } else if cli_key == GatewayUsageTool::Antigravity {
        antigravity::select_sources(files)
    } else if cli_key == GatewayUsageTool::PrimeAgent {
        prime_agent::canonical_files(files)
    } else {
        files
    }
}

fn modified_nanos(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn load_states(db: &SqliteDbState) -> Result<HashMap<String, SourceState>, String> {
    db.with_conn(|conn| {
        let mut statement = conn
            .prepare("SELECT id, json(data) FROM gateway_session_usage_state")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let mut states = HashMap::new();
        for row in rows {
            let (id, value) = row.map_err(|error| error.to_string())?;
            states.insert(
                id,
                serde_json::from_str(&value)
                    .map_err(|error| format!("Invalid session sync state: {error}"))?,
            );
        }
        Ok(states)
    })
}

fn save_state(conn: &Connection, source_id: &str, state: &SourceState) -> Result<(), String> {
    let json = serde_json::to_string(state).map_err(|error| error.to_string())?;
    let timestamp = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO gateway_session_usage_state (id, data, created_at, updated_at)
         VALUES (?1, jsonb(?2), ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
        params![source_id, json, timestamp],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// The token-event timeline a Codex child inherited from its parent.
///
/// The child copied the parent's conversation as it stood at fork time. For a
/// `paginated` parent that conversation is spread over several rollout files
/// chained by `history_base`, so the timeline is the concatenation of every
/// page's events up to the byte offset its child recorded — reading only the
/// newest file would leave most of the copied history unmatched, and the child
/// would bill it again as its own work.
fn read_codex_parent_snapshots(
    parent_paths: &[PathBuf],
    fallback_timestamp: i64,
) -> Result<Vec<parsers::CodexSnapshot>, String> {
    use crate::coding::session_manager::codex_rollout;
    let mut snapshots = Vec::new();
    // Several index keys (thread id, rollout id, file stem) can point at the
    // same file, so a lineage must contribute each rollout exactly once. The
    // parent list is filename-sorted (base page first), so the FIRST time a
    // path arrives it may carry no upper bound (a base page resolved on its
    // own is `Lineage::single`) while a LATER arrival carries the bound its
    // child recorded. Keeping the first appearance verbatim made the whole
    // ancestor page read to EOF; merging the tighter bound is what the child
    // is actually allowed to see.
    let mut seen_rollouts: HashMap<std::path::PathBuf, Option<u64>> = HashMap::new();
    let keep_segment =
        |segment: codex_rollout::LineageSegment,
         seen: &mut HashMap<std::path::PathBuf, Option<u64>>| {
            match seen.get_mut(&segment.path) {
                // First sighting: record path + bound as-is.
                None => {
                    seen.insert(segment.path.clone(), segment.end_byte_offset);
                    true
                }
                // Repeat sighting: keep the segment only if it tightens the
                // bound (a bound appearing where there was none, or a smaller
                // bound). `None` twice means the same unlimited read.
                Some(recorded) => {
                    let tighter = match (*recorded, segment.end_byte_offset) {
                        (None, Some(_)) => true,
                        (Some(current), Some(next)) => next < current,
                        (_, None) => false,
                    };
                    if tighter {
                        *recorded = segment.end_byte_offset;
                    }
                    tighter
                }
            }
        };
    for parent_path in parent_paths {
        let lineage = match codex_rollout::sessions_root_of(parent_path) {
            Some(root) => codex_rollout::resolve_lineage(&root, parent_path),
            None => codex_rollout::Lineage::single(parent_path.clone()),
        };
        let unseen = codex_rollout::Lineage {
            segments: lineage
                .segments
                .into_iter()
                .filter(|segment| keep_segment(segment.clone(), &mut seen_rollouts))
                .collect(),
        };
        snapshots.extend(parsers::read_codex_lineage_snapshots(
            &unseen,
            fallback_timestamp,
        )?);
    }
    Ok(snapshots)
}

fn sync_sources(
    db: &SqliteDbState,
    sources: &[(GatewayUsageTool, PathBuf)],
    now: i64,
    reports_rebuild_progress: bool,
) -> Result<GatewaySessionUsageImportResult, String> {
    let mut states = load_states(db)?;
    let mut claimed_proxies = states
        .values()
        .flat_map(|state| state.records.iter())
        .filter_map(|(id, state)| {
            state
                .matched_proxy_id
                .as_ref()
                .map(|proxy_id| (proxy_id.clone(), id.clone()))
        })
        .collect::<HashMap<_, _>>();
    // Antigravity needs all of its profiles at once to dedupe a conversation
    // UUID that appears in more than one of them, so the first source triggers
    // one sync over the whole set.
    let antigravity_roots = sources
        .iter()
        .filter(|(cli_key, _)| *cli_key == GatewayUsageTool::Antigravity)
        .map(|(_, root)| root.clone())
        .collect::<Vec<_>>();
    let mut antigravity_synced = antigravity_roots.is_empty();
    let mut files = sources
        .iter()
        // Qoder IDE's database is read only by `qoder_ide::sync_database`
        // below. `session_files` still reports that database so cost
        // reconciliation can price it, but handing it to the generic
        // transcript loop parses binary as JSON and counts a failed file on
        // every sync. ZCode keeps going through `parse_file`, which has its own
        // database branch, so it stays in this loop.
        .filter(|(cli_key, _)| {
            !matches!(
                *cli_key,
                GatewayUsageTool::QoderIde | GatewayUsageTool::QoderIdeCn
            )
        })
        .flat_map(|(cli_key, root)| {
            session_files(*cli_key, root)
                .into_iter()
                .map(move |path| (*cli_key, path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.1.file_name().cmp(&right.1.file_name()));
    files.dedup();
    // A Codex child names its parent by *thread* id, but a paginated thread's
    // rollover file is named `...-<thread>_<page>` and several files can carry
    // one thread id. Index every spelling of every rollout — thread id and
    // rollout id — so a child resolves whichever one its metadata recorded, and
    // keep them grouped so the parent's whole lineage can be read, not just the
    // one file that happens to sort last.
    let mut codex_files = HashMap::<String, Vec<PathBuf>>::new();
    for (_, path) in files
        .iter()
        .filter(|(cli_key, _)| *cli_key == GatewayUsageTool::Codex)
    {
        let mut keys = vec![source_identity(GatewayUsageTool::Codex, path)];
        if let Some(name) = crate::coding::session_manager::codex_rollout::parse_rollout_path(path)
        {
            for key in [name.thread_id, name.rollout_id] {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        for key in keys {
            let paths = codex_files.entry(key).or_default();
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }
    let mut result = GatewaySessionUsageImportResult::default();
    let mut retired_records = Vec::new();
    let mut failed_tools = HashSet::new();
    // Only a rebuild reports through the rebuild progress panel. The 60-second
    // sync shares this function; while a freshly claimed rebuild still waits on
    // the sync mutex, an in-flight periodic sync would otherwise write its own
    // scan phase and counters into the new task's panel.
    macro_rules! report_progress {
        ($update:expr) => {
            if reports_rebuild_progress {
                update_rebuild_progress($update);
            }
        };
    }
    report_progress!(|progress| {
        progress.phase = SessionRebuildPhase::ImportingFiles;
        progress.total_files = files.len() as u64;
    });
    for (cli_key, path) in files {
        result.scanned_files += 1;
        let source_id = format!("{}:{}", cli_key.as_str(), source_identity(cli_key, &path));
        let mut old_state = states.get(&source_id).cloned().unwrap_or_default();
        // A rebuild preserved this source's matched-proxy ids under an
        // `archived-match:` marker (the ledger rows themselves were cleared).
        // Fold them back in so `persist_records` treats them as the record's
        // previous match — the archived proxy's contribution stays excluded
        // instead of being imported as a second session row.
        if let Some(marker) = states.get(&format!("archived-match:{source_id}")) {
            for (record_id, record) in &marker.records {
                old_state
                    .records
                    .entry(record_id.clone())
                    .or_insert_with(|| record.clone());
            }
        }
        let processed = (|| {
            let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
            let stamp = modified_nanos(&metadata);
            let parser_revision = parsers::revision(cli_key);
            if old_state.parser_revision == parser_revision
                && old_state.modified_nanos == stamp
                && old_state.size == metadata.len()
                && !old_state.pending
            {
                return Ok(None);
            }
            let fallback = (stamp / 1_000_000_000) as i64;
            let mut parsed = parsers::parse_file(cli_key, &path, fallback)?;
            retired_records.append(&mut parsed.retired_records);
            if let Some(parent_id) = &parsed.parent_thread_id {
                let parent = match codex_files.get(parent_id) {
                    Some(parent_paths) => read_codex_parent_snapshots(parent_paths, fallback)?,
                    None => match states.get(&format!("codex:{parent_id}")) {
                        Some(state) => state.codex_snapshots.clone(),
                        None => {
                            return Err(format!(
                            "Parent Codex rollout {parent_id} is unavailable; deferred child usage"
                        ))
                        }
                    },
                };
                parsers::exclude_codex_replay(&mut parsed, &parent);
            }
            let mut next_state = SourceState {
                cost_reconciliation_fingerprint: old_state.cost_reconciliation_fingerprint.clone(),
                cost_reconciliation_sources: old_state.cost_reconciliation_sources.clone(),
                cumulative_usage: old_state.cumulative_usage.clone(),
                parser_revision,
                modified_nanos: stamp,
                size: metadata.len(),
                pending: parsed.pending,
                records: old_state.records.clone(),
                codex_snapshots: parsed.snapshots,
            };
            adopt_known_records(&mut next_state, &mut parsed.records, &states);
            persist_records(
                db,
                &source_id,
                next_state,
                parsed.records,
                &mut claimed_proxies,
                now,
            )
            .map(Some)
        })();
        match processed {
            Ok(Some((state, changes))) => {
                source_failures().clear(&source_id);
                states.insert(source_id, state);
                result.merge(changes);
            }
            Ok(None) => {}
            Err(error) => {
                result.failed_files += 1;
                failed_tools.insert(cli_key);
                let message = format!("Skipping local session usage {}: {error}", path.display());
                if source_failures().should_log(&source_id, &message) {
                    log::warn!("{message}");
                }
            }
        }
        report_progress!(|progress| {
            progress.processed_files = progress.processed_files.saturating_add(1);
            progress.inserted_records = result.inserted_records;
            progress.updated_records = result.updated_records;
            progress.skipped_records = result.skipped_records;
            progress.failed_files = result.failed_files;
        });
    }
    for (cli_key, root) in sources {
        if *cli_key == GatewayUsageTool::OpenClaw {
            match open_claw::sync_databases(db, root, &mut states, &mut claimed_proxies, now) {
                Ok(changes) => result.merge(changes),
                Err(error) => {
                    result.failed_files += 1;
                    failed_tools.insert(*cli_key);
                    log::warn!("OpenClaw session usage sync failed: {error}");
                }
            }
        }
        if *cli_key == GatewayUsageTool::Hermes {
            // Hermes keeps one database per profile beside the root one; a
            // profile run is invisible if only `state.db` is opened.
            for (path, source_prefix) in hermes::database_paths(root) {
                match hermes::sync_database(
                    db,
                    &path,
                    &source_prefix,
                    &mut states,
                    &mut claimed_proxies,
                    now,
                ) {
                    Ok(changes) => result.merge(changes),
                    Err(error) => {
                        result.failed_files += 1;
                        failed_tools.insert(*cli_key);
                        log::warn!(
                            "Hermes session usage sync failed for {}: {error}",
                            path.display()
                        );
                    }
                }
            }
        }
        if *cli_key == GatewayUsageTool::Qwencode {
            // The per-request files and the session rollup must be merged
            // before import: the rollup only contributes what the request
            // files do not cover, which a per-file walk cannot see.
            match qwencode::sync_root(db, root, &mut states, &mut claimed_proxies, now) {
                Ok(changes) => result.merge(changes),
                Err(error) => {
                    result.failed_files += 1;
                    failed_tools.insert(*cli_key);
                    log::warn!("Qwen Code session usage sync failed: {error}");
                }
            }
        }
        if *cli_key == GatewayUsageTool::Antigravity && !antigravity_synced {
            // Every Antigravity profile is read in one call so the same
            // conversation UUID in several profiles is imported once.
            antigravity_synced = true;
            match antigravity::sync_databases(
                db,
                &antigravity_roots,
                &mut states,
                &mut claimed_proxies,
                now,
            ) {
                Ok(changes) => result.merge(changes),
                Err(error) => {
                    result.failed_files += 1;
                    failed_tools.insert(*cli_key);
                    log::warn!("Antigravity session usage sync failed: {error}");
                }
            }
        }
        if matches!(
            *cli_key,
            GatewayUsageTool::OpenCode
                | GatewayUsageTool::Mimocode
                | GatewayUsageTool::Devin
                | GatewayUsageTool::Minimax
                | GatewayUsageTool::QoderIde
                | GatewayUsageTool::QoderIdeCn
        ) {
            // These tools keep usage in SQLite rather than in a transcript
            // tree; the root resolver points at the database file itself.
            let (sync, label) = match *cli_key {
                // OpenCode's root is its data directory; MiMoCode's is the
                // database file itself.
                GatewayUsageTool::OpenCode => (
                    open_code::sync_database(
                        db,
                        *cli_key,
                        &root.join("opencode.db"),
                        &mut states,
                        &mut claimed_proxies,
                        now,
                    ),
                    "OpenCode",
                ),
                GatewayUsageTool::Mimocode => (
                    open_code::sync_database(
                        db,
                        *cli_key,
                        root,
                        &mut states,
                        &mut claimed_proxies,
                        now,
                    ),
                    "MiMoCode",
                ),
                GatewayUsageTool::Devin => (
                    devin::sync_database(db, root, &mut states, &mut claimed_proxies, now),
                    "Devin",
                ),
                GatewayUsageTool::QoderIde | GatewayUsageTool::QoderIdeCn => (
                    qoder_ide::sync_database(
                        db,
                        *cli_key,
                        root,
                        &mut states,
                        &mut claimed_proxies,
                        now,
                    ),
                    if *cli_key == GatewayUsageTool::QoderIde {
                        "Qoder IDE"
                    } else {
                        "Qoder CN IDE"
                    },
                ),
                _ => (
                    minimax::sync_database(db, root, &mut states, &mut claimed_proxies, now),
                    "MiniMax",
                ),
            };
            match sync {
                Ok(changes) => result.merge(changes),
                Err(error) => {
                    result.failed_files += 1;
                    failed_tools.insert(*cli_key);
                    log::warn!("{label} session usage sync failed: {error}");
                }
            }
        }
    }
    report_progress!(|progress| progress.phase = SessionRebuildPhase::ReconcileLateRows);
    result.updated_records += reconcile_late_proxy_rows(db, &mut states, &mut claimed_proxies)?;
    report_progress!(|progress| progress.phase = SessionRebuildPhase::RetireDesktopRecords);
    result.updated_records +=
        reconciliation::retire_desktop_records(db, &mut states, retired_records)?;
    for cli_key in GatewayUsageTool::all()
        .into_iter()
        // Do not reparse a failed source and count its failure twice. Other
        // tools can still reconcile, and this tool retries on the next sync.
        .filter(|tool| cost_reconciliation::supports(*tool) && !failed_tools.contains(tool))
    {
        report_progress!(|progress| progress.phase = SessionRebuildPhase::ReconcileCosts);
        match cost_reconciliation::reconcile_session_costs(db, cli_key, sources, &mut states) {
            Ok(updated) => result.updated_records += updated,
            Err(error) => {
                result.failed_files += 1;
                log::warn!(
                    "{} session cost reconciliation will retry: {error}",
                    cli_key.as_str()
                );
            }
        }
    }
    report_progress!(|progress| progress.phase = SessionRebuildPhase::RollupAndPrune);
    // Backfills can add old rows after the proxy writer's pruning throttle has
    // run. Archive them now, even when the gateway itself has never started.
    let maintenance = super::settings::load_settings_from_sqlite_state(db).and_then(|settings| {
        db.with_conn(|conn| {
            super::usage_stats::rollup_and_prune(conn, i64::from(settings.log_retention_days))
        })
    });
    if let Err(error) = maintenance {
        // Imported usage and its ledger have committed. Keep the successful
        // result/event even when retention maintenance must retry later.
        log::warn!("Local usage saved, but history maintenance failed: {error}");
    }
    Ok(result)
}

fn adopt_known_records(
    state: &mut SourceState,
    records: &mut [SessionUsageRecord],
    states: &HashMap<String, SourceState>,
) {
    let mut claimed_aliases = HashSet::new();
    for record in records {
        if record.cli_key == GatewayUsageTool::Dsh {
            let fingerprint = record.fingerprint();
            let legacy_fingerprint = record.legacy_fingerprint();
            record.legacy_request_ids.retain(|id| {
                if claimed_aliases.contains(id)
                    || states
                        .values()
                        .any(|source| source.records.get(id).is_some_and(|record| record.retired))
                {
                    return false;
                }
                if let Some(previous) = states
                    .values()
                    .filter_map(|source| source.records.get(id))
                    .find(|previous| {
                        (previous.fingerprint == fingerprint
                            || previous.fingerprint == legacy_fingerprint)
                            && (previous.envelope_id.is_none()
                                || record.usage.envelope_id.is_none()
                                || previous.envelope_id == record.usage.envelope_id)
                    })
                {
                    state
                        .records
                        .entry(id.clone())
                        .or_insert_with(|| previous.clone());
                    claimed_aliases.insert(id.clone());
                    true
                } else {
                    false
                }
            });
        }
        if !state.records.contains_key(&record.request_id) {
            let canonical = states
                .values()
                .find_map(|source| source.records.get(&record.request_id));
            if let Some(previous) = canonical {
                state
                    .records
                    .insert(record.request_id.clone(), previous.clone());
            }
        }
    }
}

fn persist_records(
    db: &SqliteDbState,
    source_id: &str,
    mut state: SourceState,
    records: Vec<SessionUsageRecord>,
    claimed_proxies: &mut HashMap<String, String>,
    now: i64,
) -> Result<(SourceState, GatewaySessionUsageImportResult), String> {
    let mut claims = HashMap::new();
    let mut released_claims = HashSet::new();
    let mut changes = GatewaySessionUsageImportResult::default();
    db.with_conn_mut(|conn| {
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        for record in records {
            changes.parsed_records += 1;
            let fingerprint = record.fingerprint();
            let canonical_previous = state.records.get(&record.request_id).cloned();
            let adopted_alias = (canonical_previous.is_none() && record.cli_key == GatewayUsageTool::Dsh)
                .then(|| record.legacy_request_ids.iter().find(|id| state.records.get(*id).is_some_and(|record| !record.retired)).cloned()).flatten();
            let previous = canonical_previous.clone().or_else(|| adopted_alias.as_ref().and_then(|id| state.records.get(id).cloned()));
            if previous.as_ref().is_some_and(|previous| previous.retired) {
                changes.skipped_records += 1;
                continue;
            }
            if record.created_at > now - SESSION_SETTLE_SECONDS {
                state.pending = true;
                continue;
            }
            for legacy_id in &record.legacy_request_ids {
                if legacy_id == &record.request_id {
                    continue;
                }
                // Several old path/line identities can describe one response.
                // Adopt one row, then remove its redundant snapshots atomically.
                // A known ledger entry may already be archived; never resurrect it.
                if canonical_previous.is_none() {
                    changes.updated_records += transaction.execute(
                        "UPDATE proxy_request_logs SET request_id = ?1
                         WHERE request_id = ?2 AND data_source = 'session' AND app_type = ?3
                           AND NOT EXISTS (SELECT 1 FROM proxy_request_logs WHERE request_id = ?1)",
                        params![record.request_id, legacy_id, record.cli_key.as_str()],
                    ).map_err(|error| error.to_string())? as u64;
                }
                changes.updated_records += transaction.execute(
                    "DELETE FROM proxy_request_logs
                     WHERE request_id = ?1 AND data_source = 'session' AND app_type = ?2",
                    params![legacy_id, record.cli_key.as_str()],
                ).map_err(|error| error.to_string())? as u64;
            }
            if previous.as_ref().is_some_and(|previous| {
                previous.fingerprint == fingerprint && previous.envelope_id == record.usage.envelope_id
            }) {
                state.records.insert(record.request_id.clone(), previous.unwrap());
                if let Some(id) = adopted_alias { state.records.get_mut(&id).unwrap().retired = true; }
                changes.skipped_records += 1;
                continue;
            }
            let existing_source: Option<String> = transaction.query_row(
                "SELECT COALESCE(data_source, 'proxy') FROM proxy_request_logs WHERE request_id = ?1",
                [&record.request_id], |row| row.get(0),
            ).optional().map_err(|error| error.to_string())?;
            let previous_proxy_id = previous.as_ref().and_then(|item| item.matched_proxy_id.as_ref());
            let mut rejected_previous_match = false;
            let retained_proxy_id = if let Some(proxy_id) = previous_proxy_id {
                if !super::usage_stats::request_exists(&transaction, proxy_id)? {
                    // The proxy may already be archived. Its saved match still
                    // prevents importing the same paid invocation a second time.
                    Some(proxy_id.clone())
                } else {
                    let matched = find_matching_proxy(&transaction, &record, Some(proxy_id))?;
                    if matched.is_none() {
                        rejected_previous_match = true;
                        if claimed_proxies.get(proxy_id) == Some(&record.request_id) {
                            released_claims.insert(proxy_id.clone());
                        }
                    }
                    matched
                }
            } else { None };
            let matched_proxy_id = match retained_proxy_id {
                Some(proxy_id) => Some(proxy_id),
                None => find_matching_proxy(&transaction, &record, None)?,
            }.filter(|proxy_id| {
                claims.get(proxy_id).or_else(|| claimed_proxies.get(proxy_id)
                    .filter(|_| !released_claims.contains(proxy_id)))
                    .is_none_or(|owner| owner == &record.request_id)
            });
            if existing_source.as_deref() == Some("proxy") || matched_proxy_id.is_some() {
                if existing_source.as_deref() == Some("session") {
                    transaction.execute(
                        "DELETE FROM proxy_request_logs WHERE request_id = ?1 AND data_source = 'session'",
                        [&record.request_id],
                    ).map_err(|error| error.to_string())?;
                    changes.updated_records += 1;
                }
                changes.skipped_records += 1;
            } else if previous.is_some() && existing_source.is_none() && !rejected_previous_match {
                // A processed row was pruned into a rollup (or explicitly
                // removed). Re-reading its transcript must not insert it again.
                changes.skipped_records += 1;
            } else {
                write_record(&transaction, &record)?;
                if existing_source.is_some() { changes.updated_records += 1; }
                else { changes.inserted_records += 1; }
            }
            if let Some(proxy_id) = &matched_proxy_id {
                claims.insert(proxy_id.clone(), record.request_id.clone());
            }
            let accounting = if record.cli_key == GatewayUsageTool::ClaudeDesktop {
                reconciliation::read_contribution(&transaction, &record.request_id)?
                    .or_else(|| previous.as_ref().and_then(|item| item.accounting.clone()))
            } else { None };
            let recorded_cost = if cost_reconciliation::supports(record.cli_key) {
                cost_reconciliation::read_recorded_cost(&transaction, &record.request_id)?
                    .or_else(|| previous.as_ref().and_then(|item| item.recorded_cost.clone()))
            } else { None };
            let cost_repair = previous.as_ref().and_then(|item| item.cost_repair.clone());
            state.records.insert(record.request_id.clone(), ImportedRecord {
                fingerprint, envelope_id: record.usage.envelope_id.clone(), matched_proxy_id,
                accounting, recorded_cost, cost_repair, retired: false,
            });
            if let Some(id) = adopted_alias { state.records.get_mut(&id).unwrap().retired = true; }
        }
        // The ledger and usage rows must commit together, including no-op rows
        // that are already represented by a gateway log.
        save_state(&transaction, source_id, &state)?;
        transaction.commit().map_err(|error| error.to_string())
    })?;
    for proxy_id in released_claims {
        claimed_proxies.remove(&proxy_id);
    }
    claimed_proxies.extend(claims);
    Ok((state, changes))
}

fn find_matching_proxy(
    conn: &Connection,
    record: &SessionUsageRecord,
    only_proxy_id: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(envelope_id) = &record.usage.envelope_id {
        // Two index-friendly lookups instead of one `OR` of concatenated
        // expressions: SQLite gives up the `request_id` primary-key index as
        // soon as a branch concatenates columns, and then every record scans
        // the whole history (measured ~180 ms per lookup on 60k rows, which
        // made a full rebuild take tens of minutes). Each half is now a
        // constant-time lookup.
        //
        // Claude writes the bare form; every other CLI scopes the id by app
        // and provider (`stable_usage_request_id`). Both forms are tried so
        // ids written by older versions still match.
        let bare = format!("SESSION:{envelope_id}");
        let exact = conn
            .query_row(
                "SELECT request_id FROM proxy_request_logs
                 WHERE request_id = ?1
                   AND COALESCE(data_source, 'proxy') = 'proxy'
                   AND app_type = ?2 AND request_kind = 'request'
                   AND (?3 IS NULL OR request_id = ?3)
                 LIMIT 1",
                params![bare, record.cli_key.as_str(), only_proxy_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if exact.is_some() {
            return Ok(exact);
        }
        // A range bounds the lookup to `SESSION:<cli>:...`; the equality on
        // the constructed id pins `app_type`, so no separate app filter is
        // needed (and leaving it out keeps the primary-key index in use).
        let prefix = format!("SESSION:{}:", record.cli_key.as_str());
        let upper = format!("SESSION:{};", record.cli_key.as_str());
        let scoped = conn
            .query_row(
                "SELECT request_id FROM proxy_request_logs
                 WHERE request_id >= ?1 AND request_id < ?2
                   AND request_id = 'SESSION:' || app_type || ':' || provider_id || ':' || ?3
                   AND COALESCE(data_source, 'proxy') = 'proxy'
                   AND request_kind = 'request'
                   AND (?4 IS NULL OR request_id = ?4)
                 LIMIT 1",
                params![prefix, upper, envelope_id, only_proxy_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if scoped.is_some() {
            return Ok(scoped);
        }
    }
    // Zero counters cannot identify an invocation. A known native envelope
    // must never match a different known proxy envelope just by equal usage.
    if record.model == "unknown"
        || record.usage.total_tokens().is_none()
        || record.metadata.granularity != SessionUsageGranularity::Request
    {
        return Ok(None);
    }
    let mut query = conn
        .prepare_cached(
            "SELECT request_id FROM proxy_request_logs
         WHERE COALESCE(data_source, 'proxy') = 'proxy' AND app_type = ?1
           AND request_kind = 'request'
           AND (?9 IS NULL OR request_id = ?9)
           AND (?10 IS NULL OR request_id NOT GLOB 'SESSION:*')
           AND (stream_outcome = 'completed' OR (stream_outcome IS NULL AND status_code >= 200 AND status_code < 300))
           AND (LOWER(model) = LOWER(?2) OR LOWER(request_model) = LOWER(?2))
           AND input_tokens = ?3 AND output_tokens = ?4
           AND cache_read_tokens = ?5 AND cache_creation_tokens = ?6
           AND ?7 BETWEEN created_at - (MAX(COALESCE(duration_ms, 0), 0) + 999) / 1000 - ?8
                      AND created_at + ?11
         LIMIT 2",
        )
        .map_err(|error| error.to_string())?;
    let matches = query
        .query_map(
            params![
                record.cli_key.as_str(),
                record.model,
                record.usage.input_tokens.unwrap_or(0) as i64,
                record.usage.output_tokens.unwrap_or(0) as i64,
                record.usage.cache_read_tokens.unwrap_or(0) as i64,
                record.usage.cache_creation_tokens.unwrap_or(0) as i64,
                record.created_at,
                PROXY_MATCH_START_GRACE_SECONDS,
                only_proxy_id,
                record.usage.envelope_id,
                PROXY_MATCH_END_GRACE_SECONDS,
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok((matches.len() == 1).then(|| matches[0].clone()))
}

fn write_record(conn: &Connection, record: &SessionUsageRecord) -> Result<(), String> {
    let input = record.usage.input_tokens.unwrap_or(0);
    let output = record.usage.output_tokens.unwrap_or(0);
    let read = record.usage.cache_read_tokens.unwrap_or(0);
    let creation = record.usage.cache_creation_tokens.unwrap_or(0);
    let costs = calculate_session_costs(conn, &record.model, input, output, read, creation);
    let total = record
        .reported_cost_usd
        .clone()
        .unwrap_or_else(|| format_decimal_cost(costs.total()));
    let mut metadata = record.metadata.clone();
    if metadata.cost_source.is_none() {
        metadata.cost_source = Some(
            if record.reported_cost_usd.is_some() {
                "reported"
            } else if super::usage_stats::session_model_has_pricing(conn, &record.model) {
                "model_pricing"
            } else {
                "unavailable"
            }
            .into(),
        );
    }
    conn.execute(
        "INSERT INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
            latency_ms, first_token_ms, duration_ms, status_code, error_message,
            session_id, provider_type, is_streaming, cost_multiplier, created_at, data_source,
            usage_metadata, usage_request_count, extra_tokens, project_path, credits
         ) VALUES (
            ?1, 'session', ?2, ?3, ?3, ?4, ?5, ?6, ?7,
            ?8, ?9, ?10, ?11, ?12, 0, NULL, ?20, 200, NULL, ?13, 'session', 0, '1.0', ?14, 'session',
            jsonb(?15), ?16, ?17, ?18, ?19
         ) ON CONFLICT(request_id) DO UPDATE SET
            model = excluded.model, request_model = excluded.request_model,
            input_tokens = excluded.input_tokens, output_tokens = excluded.output_tokens,
            cache_read_tokens = excluded.cache_read_tokens, cache_creation_tokens = excluded.cache_creation_tokens,
            input_cost_usd = excluded.input_cost_usd, output_cost_usd = excluded.output_cost_usd,
            cache_read_cost_usd = excluded.cache_read_cost_usd, cache_creation_cost_usd = excluded.cache_creation_cost_usd,
            total_cost_usd = excluded.total_cost_usd, session_id = excluded.session_id, created_at = excluded.created_at,
            usage_metadata = excluded.usage_metadata, usage_request_count = excluded.usage_request_count,
            extra_tokens = excluded.extra_tokens, project_path = excluded.project_path,
            credits = excluded.credits, duration_ms = excluded.duration_ms
         WHERE proxy_request_logs.data_source = 'session'",
        params![record.request_id, record.cli_key.as_str(), record.model, input as i64, output as i64, read as i64, creation as i64,
            format_decimal_cost(costs.input_cost_usd), format_decimal_cost(costs.output_cost_usd),
            format_decimal_cost(costs.cache_read_cost_usd), format_decimal_cost(costs.cache_creation_cost_usd),
            total, record.session_id, record.created_at,
            serde_json::to_string(&metadata).map_err(|error| error.to_string())?,
            record.call_count() as i64, record.extra_tokens() as i64,
            record.project_path, record.credits, record.duration_ms.map(|value| value as i64)],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn reconcile_late_proxy_rows(
    db: &SqliteDbState,
    states: &mut HashMap<String, SourceState>,
    claimed_proxies: &mut HashMap<String, String>,
) -> Result<u64, String> {
    let owners = states
        .iter()
        .flat_map(|(source_id, state)| {
            state
                .records
                .keys()
                .map(move |id| (id.clone(), source_id.clone()))
        })
        .collect::<HashMap<_, _>>();
    let rows = db.with_conn(|conn| {
        let mut query = conn.prepare(
            "SELECT request_id, app_type, model, input_tokens, output_tokens, cache_read_tokens,
                    cache_creation_tokens, created_at, session_id
             FROM proxy_request_logs WHERE data_source = 'session'
               AND COALESCE(json_extract(usage_metadata, '$.granularity'), 'request') = 'request'",
        ).map_err(|error| error.to_string())?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row.get::<_, i64>(4)?.max(0) as u64,
                    row.get::<_, i64>(5)?.max(0) as u64,
                    row.get::<_, i64>(6)?.max(0) as u64,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    })?;
    let mut changed = 0;
    for (id, cli, model, input, output, read, creation, created_at, session_id) in rows {
        let Some(source_id) = owners.get(&id) else {
            continue;
        };
        let Ok(cli_key) =
            serde_json::from_value::<GatewayUsageTool>(serde_json::Value::String(cli))
        else {
            continue;
        };
        let record = SessionUsageRecord {
            metadata: SessionUsageMetadata::default(),
            request_id: id.clone(),
            legacy_request_ids: Vec::new(),
            cli_key,
            model,
            created_at,
            session_id: session_id.unwrap_or_default(),
            usage: TokenUsage {
                input_tokens: Some(input),
                output_tokens: Some(output),
                cache_read_tokens: Some(read),
                cache_creation_tokens: Some(creation),
                envelope_id: states
                    .get(source_id)
                    .and_then(|state| state.records.get(&id))
                    .and_then(|record| record.envelope_id.clone()),
            },
            reported_cost_usd: None,
            // Only used to re-find a proxy match; neither the project nor the
            // duration enters the matcher and this row is not written back.
            project_path: None,
            credits: None,
            duration_ms: None,
        };
        let matched = db.with_conn(|conn| find_matching_proxy(conn, &record, None))?;
        let Some(proxy_id) = matched.filter(|proxy_id| !claimed_proxies.contains_key(proxy_id))
        else {
            continue;
        };
        let Some(mut state) = states.get(source_id).cloned() else {
            continue;
        };
        state.records.get_mut(&id).unwrap().matched_proxy_id = Some(proxy_id.clone());
        db.with_conn_mut(|conn| {
            let transaction = conn.transaction().map_err(|error| error.to_string())?;
            transaction.execute("DELETE FROM proxy_request_logs WHERE request_id = ?1 AND data_source = 'session'", [&id])
                .map_err(|error| error.to_string())?;
            save_state(&transaction, source_id, &state)?;
            transaction.commit().map_err(|error| error.to_string())
        })?;
        states.insert(source_id.clone(), state);
        claimed_proxies.insert(proxy_id, id);
        changed += 1;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests;
