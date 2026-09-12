use super::types::{
    GatewaySessionImportCli, GatewaySessionUsageImportInput, GatewaySessionUsageImportResult,
    GatewayUsageRecordedEvent, GatewayUsageTool, SessionUsageGranularity, SessionUsageMetadata,
};
use super::usage_parser::TokenUsage;
use super::usage_stats::{calculate_session_costs, format_decimal_cost};
use crate::db::SqliteDbState;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use walkdir::WalkDir;

mod cost_reconciliation;
mod desktop;
mod dsh;
mod grok;
mod hermes;
mod kimi;
mod open_claw;
mod open_code;
mod parsers;
mod pi;
mod reconciliation;
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
            self.metadata
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
        sync_sources(&db, &sources, Utc::now().timestamp())
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
        GatewaySessionImportCli::OpenCode => vec![GatewayUsageTool::OpenCode],
        GatewaySessionImportCli::Pi => vec![GatewayUsageTool::Pi],
        GatewaySessionImportCli::OhMyPi => vec![GatewayUsageTool::OhMyPi],
        GatewaySessionImportCli::Dsh => vec![GatewayUsageTool::Dsh],
        GatewaySessionImportCli::Hermes => vec![GatewayUsageTool::Hermes],
        GatewaySessionImportCli::OpenClaw => vec![GatewayUsageTool::OpenClaw],
        GatewaySessionImportCli::KimiCli => vec![GatewayUsageTool::KimiCli],
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
        GatewayUsageTool::OpenCode => get_opencode_runtime_location_sync(db).ok(),
        GatewayUsageTool::Pi => get_pi_runtime_location_sync(db).ok(),
        GatewayUsageTool::OhMyPi => get_oh_my_pi_runtime_location_sync(db).ok(),
        GatewayUsageTool::OpenClaw => get_openclaw_runtime_location_sync(db).ok(),
        _ => None,
    };
    let mut roots = Vec::new();
    let (home_name, suffix) = match cli_key {
        GatewayUsageTool::Claude => (".claude", "projects"),
        GatewayUsageTool::Codex => (".codex", "sessions"),
        GatewayUsageTool::Grok => (".grok", "sessions"),
        GatewayUsageTool::Kimi => (".kimi-code", "sessions"),
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
    };
    if let Some(location) = location {
        if cli_key == GatewayUsageTool::Kimi {
            roots.push(location.host_path.join("store"));
        }
        roots.push(location.host_path.join(suffix));
    }
    if let Some(home) = dirs::home_dir() {
        let default = home.join(home_name).join(suffix);
        if !roots.contains(&default) {
            roots.push(default);
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

fn source_identity(cli_key: GatewayUsageTool, path: &Path) -> String {
    if matches!(
        cli_key,
        GatewayUsageTool::Pi
            | GatewayUsageTool::OhMyPi
            | GatewayUsageTool::Dsh
            | GatewayUsageTool::Grok
            | GatewayUsageTool::Kimi
            | GatewayUsageTool::KimiCli
            | GatewayUsageTool::ClaudeDesktop
            | GatewayUsageTool::OpenClaw
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

fn session_files(cli_key: GatewayUsageTool, root: &Path) -> Vec<PathBuf> {
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
                GatewayUsageTool::Codex => extension == "jsonl" && name.starts_with("rollout-"),
                GatewayUsageTool::Gemini => {
                    matches!(extension, "json" | "jsonl") && name.starts_with("session-")
                }
                GatewayUsageTool::Grok => {
                    name == "updates.jsonl"
                        && !path
                            .components()
                            .any(|part| part.as_os_str() == "subagents")
                }
                GatewayUsageTool::Kimi | GatewayUsageTool::KimiCli => {
                    kimi::is_usage_file(cli_key, &path)
                }
                GatewayUsageTool::Pi | GatewayUsageTool::OhMyPi => extension == "jsonl",
                GatewayUsageTool::Dsh => dsh::generation(&path).is_some(),
                GatewayUsageTool::Hermes => false,
                GatewayUsageTool::OpenClaw => open_claw::is_transcript(&path),
                GatewayUsageTool::OpenCode => {
                    extension == "json"
                        && path.strip_prefix(root).ok().is_some_and(|relative| {
                            relative.starts_with(Path::new("storage").join("message"))
                        })
                }
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

fn sync_sources(
    db: &SqliteDbState,
    sources: &[(GatewayUsageTool, PathBuf)],
    now: i64,
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
    let mut files = sources
        .iter()
        .flat_map(|(cli_key, root)| {
            session_files(*cli_key, root)
                .into_iter()
                .map(move |path| (*cli_key, path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.1.file_name().cmp(&right.1.file_name()));
    files.dedup();
    let codex_files = files
        .iter()
        .filter(|(cli_key, _)| *cli_key == GatewayUsageTool::Codex)
        .map(|(_, path)| (source_identity(GatewayUsageTool::Codex, path), path.clone()))
        .collect::<HashMap<_, _>>();
    let mut result = GatewaySessionUsageImportResult::default();
    let mut retired_records = Vec::new();
    let mut failed_tools = HashSet::new();
    for (cli_key, path) in files {
        result.scanned_files += 1;
        let source_id = format!("{}:{}", cli_key.as_str(), source_identity(cli_key, &path));
        let old_state = states.get(&source_id).cloned().unwrap_or_default();
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
                let parent = if let Some(parent_path) = codex_files.get(parent_id) {
                    parsers::parse_file(GatewayUsageTool::Codex, parent_path, fallback)?.snapshots
                } else if let Some(state) = states.get(&format!("codex:{parent_id}")) {
                    state.codex_snapshots.clone()
                } else {
                    return Err(format!(
                        "Parent Codex rollout {parent_id} is unavailable; deferred child usage"
                    ));
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
                states.insert(source_id, state);
                result.merge(changes);
            }
            Ok(None) => {}
            Err(error) => {
                result.failed_files += 1;
                failed_tools.insert(cli_key);
                log::warn!("Skipping local session usage {}: {error}", path.display());
            }
        }
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
            match hermes::sync_database(
                db,
                &root.join("state.db"),
                &mut states,
                &mut claimed_proxies,
                now,
            ) {
                Ok(changes) => result.merge(changes),
                Err(error) => {
                    result.failed_files += 1;
                    failed_tools.insert(*cli_key);
                    log::warn!("Hermes session usage sync failed: {error}");
                }
            }
        }
        if *cli_key == GatewayUsageTool::OpenCode {
            match open_code::sync_database(
                db,
                &root.join("opencode.db"),
                &mut states,
                &mut claimed_proxies,
                now,
            ) {
                Ok(changes) => result.merge(changes),
                Err(error) => {
                    result.failed_files += 1;
                    failed_tools.insert(*cli_key);
                    log::warn!("OpenCode session usage sync failed: {error}");
                }
            }
        }
    }
    result.updated_records += reconcile_late_proxy_rows(db, &mut states, &mut claimed_proxies)?;
    result.updated_records +=
        reconciliation::retire_desktop_records(db, &mut states, retired_records)?;
    for cli_key in GatewayUsageTool::all()
        .into_iter()
        // Do not reparse a failed source and count its failure twice. Other
        // tools can still reconcile, and this tool retries on the next sync.
        .filter(|tool| cost_reconciliation::supports(*tool) && !failed_tools.contains(tool))
    {
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
        let exact = conn
            .query_row(
                "SELECT request_id FROM proxy_request_logs
             WHERE COALESCE(data_source, 'proxy') = 'proxy' AND app_type = ?1
               AND (?3 IS NULL OR request_id = ?3)
               AND (request_id = 'SESSION:' || ?2
                    OR request_id = 'SESSION:' || app_type || ':' || provider_id || ':' || ?2)
             LIMIT 1",
                params![record.cli_key.as_str(), envelope_id, only_proxy_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if exact.is_some() {
            return Ok(exact);
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
           AND (?9 IS NULL OR request_id = ?9)
           AND (?10 IS NULL OR request_id NOT GLOB 'SESSION:*')
           AND status_code >= 200 AND status_code < 300
           AND (stream_outcome IS NULL OR stream_outcome = 'completed')
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
            usage_metadata, usage_request_count, extra_tokens
         ) VALUES (
            ?1, 'session', ?2, ?3, ?3, ?4, ?5, ?6, ?7,
            ?8, ?9, ?10, ?11, ?12, 0, NULL, 0, 200, NULL, ?13, 'session', 0, '1.0', ?14, 'session',
            jsonb(?15), ?16, ?17
         ) ON CONFLICT(request_id) DO UPDATE SET
            model = excluded.model, request_model = excluded.request_model,
            input_tokens = excluded.input_tokens, output_tokens = excluded.output_tokens,
            cache_read_tokens = excluded.cache_read_tokens, cache_creation_tokens = excluded.cache_creation_tokens,
            input_cost_usd = excluded.input_cost_usd, output_cost_usd = excluded.output_cost_usd,
            cache_read_cost_usd = excluded.cache_read_cost_usd, cache_creation_cost_usd = excluded.cache_creation_cost_usd,
            total_cost_usd = excluded.total_cost_usd, session_id = excluded.session_id, created_at = excluded.created_at,
            usage_metadata = excluded.usage_metadata, usage_request_count = excluded.usage_request_count,
            extra_tokens = excluded.extra_tokens
         WHERE proxy_request_logs.data_source = 'session'",
        params![record.request_id, record.cli_key.as_str(), record.model, input as i64, output as i64, read as i64, creation as i64,
            format_decimal_cost(costs.input_cost_usd), format_decimal_cost(costs.output_cost_usd),
            format_decimal_cost(costs.cache_read_cost_usd), format_decimal_cost(costs.cache_creation_cost_usd),
            total, record.session_id, record.created_at,
            serde_json::to_string(&metadata).map_err(|error| error.to_string())?,
            record.call_count() as i64, record.extra_tokens() as i64],
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
