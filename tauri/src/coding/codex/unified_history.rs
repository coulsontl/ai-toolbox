use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml_edit::{DocumentMut, Item};
use uuid::Uuid;

use super::history_sync;

pub(crate) const UNIFIED_HISTORY_PROVIDER_ID: &str = "custom";
const OFFICIAL_HISTORY_PROVIDER_ID: &str = "openai";
const CONFIG_FILE_NAME: &str = "config.toml";
const SESSIONS_DIR_NAME: &str = "sessions";
const ARCHIVED_SESSIONS_DIR_NAME: &str = "archived_sessions";
const MIGRATION_BACKUP_DIR_NAME: &str = "unified-session-history-v1";
const RESTORE_BACKUP_DIR_NAME: &str = "unified-session-history-restore-v1";
const LEDGER_FILE_NAME: &str = "ledger.json";
#[cfg(test)]
const STATE_DB_FILE_NAME: &str = "state_5.sqlite";
const WRITE_LOCK_RETRY_LIMIT: usize = 40;
const WRITE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(250);
const FILE_RETRY_LIMIT: usize = 20;
const FILE_RETRY_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifiedHistoryMigrationResult {
    pub migrated_session_files: usize,
    pub migrated_session_entries: usize,
    pub migrated_thread_rows: usize,
    pub rewritten_index_entries: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifiedHistoryRestoreResult {
    pub restored_session_files: usize,
    pub restored_session_entries: usize,
    pub restored_thread_rows: usize,
    pub rewritten_index_entries: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiedHistoryLedger {
    codex_home: String,
    created_at: String,
    source_provider_id: String,
    target_provider_id: String,
    session_ids: Vec<String>,
    thread_ids: Vec<String>,
}

fn parse_toml_document(config_toml: &str) -> Result<DocumentMut, String> {
    if config_toml.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        config_toml
            .parse::<DocumentMut>()
            .map_err(|error| format!("Failed to parse Codex config.toml: {error}"))
    }
}

fn unified_official_provider_table() -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table["name"] = toml_edit::value("OpenAI");
    table["requires_openai_auth"] = toml_edit::value(true);
    table["supports_websockets"] = toml_edit::value(true);
    table["wire_api"] = toml_edit::value("responses");
    table
}

fn table_matches_unified_official_provider(table: &toml_edit::Table) -> bool {
    table.len() == 4
        && table.get("name").and_then(|item| item.as_str()) == Some("OpenAI")
        && table
            .get("requires_openai_auth")
            .and_then(|item| item.as_bool())
            == Some(true)
        && table
            .get("supports_websockets")
            .and_then(|item| item.as_bool())
            == Some(true)
        && table.get("wire_api").and_then(|item| item.as_str()) == Some("responses")
}

pub(crate) fn inject_unified_session_history_config(config_toml: &str) -> Result<String, String> {
    let mut document = parse_toml_document(config_toml)?;

    if document.get("model_provider").is_some() {
        return Ok(config_toml.to_string());
    }

    let existing_custom_conflicts = document
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(UNIFIED_HISTORY_PROVIDER_ID))
        .and_then(|item| item.as_table())
        .is_some_and(|table| !table_matches_unified_official_provider(table));
    if existing_custom_conflicts {
        log::warn!(
            "Skip Codex unified session history injection because [model_providers.custom] already exists with a different shape"
        );
        return Ok(config_toml.to_string());
    }

    document["model_provider"] = toml_edit::value(UNIFIED_HISTORY_PROVIDER_ID);
    if document.get("model_providers").is_none() {
        let mut parent = toml_edit::Table::new();
        parent.set_implicit(true);
        document["model_providers"] = Item::Table(parent);
    }
    if let Some(providers) = document["model_providers"].as_table_mut() {
        if !providers.contains_key(UNIFIED_HISTORY_PROVIDER_ID) {
            providers.insert(
                UNIFIED_HISTORY_PROVIDER_ID,
                Item::Table(unified_official_provider_table()),
            );
        }
    }

    Ok(document.to_string())
}

pub(crate) fn strip_unified_session_history_config(config_toml: &str) -> Result<String, String> {
    if !config_toml.contains("model_provider") {
        return Ok(config_toml.to_string());
    }

    let mut document = parse_toml_document(config_toml)?;
    if document
        .get("model_provider")
        .and_then(|item| item.as_str())
        != Some(UNIFIED_HISTORY_PROVIDER_ID)
    {
        return Ok(config_toml.to_string());
    }

    let matches_injected = document
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(UNIFIED_HISTORY_PROVIDER_ID))
        .and_then(|item| item.as_table())
        .is_some_and(table_matches_unified_official_provider);
    if !matches_injected {
        return Ok(config_toml.to_string());
    }

    document.as_table_mut().remove("model_provider");
    let providers_empty = document["model_providers"]
        .as_table_mut()
        .map(|providers| {
            providers.remove(UNIFIED_HISTORY_PROVIDER_ID);
            providers.is_empty()
        })
        .unwrap_or(false);
    if providers_empty {
        document.as_table_mut().remove("model_providers");
    }

    Ok(document.to_string())
}

pub(crate) fn config_routes_to_unified_history(config_toml: &str) -> bool {
    parse_toml_document(config_toml)
        .ok()
        .and_then(|document| {
            document
                .get("model_provider")
                .and_then(|item| item.as_str())
                .map(|provider_id| provider_id.trim() == UNIFIED_HISTORY_PROVIDER_ID)
        })
        .unwrap_or(false)
}

pub fn has_codex_unified_history_backup(codex_home: &Path) -> bool {
    collect_restore_ledger(codex_home)
        .map(|(session_ids, thread_ids)| !session_ids.is_empty() || !thread_ids.is_empty())
        .unwrap_or(false)
}

pub fn migrate_official_history_to_unified(
    codex_home: &Path,
) -> Result<CodexUnifiedHistoryMigrationResult, String> {
    let _guard = history_sync::lock_history_op();
    let start = Instant::now();
    let config_text = fs::read_to_string(codex_home.join(CONFIG_FILE_NAME)).unwrap_or_default();
    if !config_routes_to_unified_history(&config_text) {
        return Ok(CodexUnifiedHistoryMigrationResult {
            skipped_reason: Some("live_not_unified".to_string()),
            duration_ms: start.elapsed().as_millis(),
            ..empty_migration_result()
        });
    }

    let generation_root = backup_generation_root(codex_home, MIGRATION_BACKUP_DIR_NAME);
    let state_db_paths = codex_state_db_paths(codex_home, &config_text);
    let mut migrated_session_ids = BTreeSet::new();
    let mut migrated_session_files = 0;
    let mut migrated_session_entries = 0;

    for session_file in collect_session_jsonl_files(codex_home) {
        let rewrite = rewrite_session_file_provider(
            &session_file,
            codex_home,
            &generation_root,
            OFFICIAL_HISTORY_PROVIDER_ID,
            UNIFIED_HISTORY_PROVIDER_ID,
            None,
        )?;
        if rewrite.changed {
            migrated_session_files += 1;
            migrated_session_entries += rewrite.changed_entries;
            migrated_session_ids.extend(rewrite.session_ids);
        }
    }

    let mut migrated_thread_ids = BTreeSet::new();
    for db_path in &state_db_paths {
        if !db_path.exists() {
            continue;
        }
        migrated_thread_ids.extend(retry_sqlite_write(|| {
            migrate_state_db_provider(
                db_path,
                codex_home,
                &generation_root,
                OFFICIAL_HISTORY_PROVIDER_ID,
                UNIFIED_HISTORY_PROVIDER_ID,
                None,
            )
        })?);
    }
    let migrated_thread_rows = migrated_thread_ids.len();

    if migrated_session_ids.is_empty() && migrated_thread_ids.is_empty() {
        return Ok(CodexUnifiedHistoryMigrationResult {
            skipped_reason: Some("no_official_history".to_string()),
            duration_ms: start.elapsed().as_millis(),
            ..empty_migration_result()
        });
    }

    let ledger = UnifiedHistoryLedger {
        codex_home: canonical_dir_string(codex_home),
        created_at: Utc::now().to_rfc3339(),
        source_provider_id: OFFICIAL_HISTORY_PROVIDER_ID.to_string(),
        target_provider_id: UNIFIED_HISTORY_PROVIDER_ID.to_string(),
        session_ids: migrated_session_ids.into_iter().collect(),
        thread_ids: migrated_thread_ids.into_iter().collect(),
    };
    write_ledger(&generation_root, &ledger)?;
    let rewritten_index_entries =
        rebuild_session_index_for_state_db_paths(codex_home, &state_db_paths)?;

    Ok(CodexUnifiedHistoryMigrationResult {
        migrated_session_files,
        migrated_session_entries,
        migrated_thread_rows,
        rewritten_index_entries,
        backup_path: Some(generation_root.to_string_lossy().to_string()),
        skipped_reason: None,
        duration_ms: start.elapsed().as_millis(),
    })
}

pub fn restore_official_history_from_unified_backups(
    codex_home: &Path,
    unified_history_enabled: bool,
) -> Result<CodexUnifiedHistoryRestoreResult, String> {
    let _guard = history_sync::lock_history_op();
    let start = Instant::now();
    if unified_history_enabled {
        return Ok(CodexUnifiedHistoryRestoreResult {
            skipped_reason: Some("unify_toggle_on".to_string()),
            duration_ms: start.elapsed().as_millis(),
            ..empty_restore_result()
        });
    }

    let (session_ids, thread_ids) = collect_restore_ledger(codex_home)?;
    if session_ids.is_empty() && thread_ids.is_empty() {
        return Ok(CodexUnifiedHistoryRestoreResult {
            skipped_reason: Some("no_backup_ledger".to_string()),
            duration_ms: start.elapsed().as_millis(),
            ..empty_restore_result()
        });
    }

    let generation_root = backup_generation_root(codex_home, RESTORE_BACKUP_DIR_NAME);
    let config_text = fs::read_to_string(codex_home.join(CONFIG_FILE_NAME)).unwrap_or_default();
    let state_db_paths = codex_state_db_paths(codex_home, &config_text);
    let mut restored_session_files = 0;
    let mut restored_session_entries = 0;
    for session_file in collect_session_jsonl_files(codex_home) {
        let rewrite = rewrite_session_file_provider(
            &session_file,
            codex_home,
            &generation_root,
            UNIFIED_HISTORY_PROVIDER_ID,
            OFFICIAL_HISTORY_PROVIDER_ID,
            Some(&session_ids),
        )?;
        if rewrite.changed {
            restored_session_files += 1;
            restored_session_entries += rewrite.changed_entries;
        }
    }

    let mut restored_thread_ids = BTreeSet::new();
    for db_path in &state_db_paths {
        if !db_path.exists() {
            continue;
        }
        restored_thread_ids.extend(retry_sqlite_write(|| {
            migrate_state_db_provider(
                db_path,
                codex_home,
                &generation_root,
                UNIFIED_HISTORY_PROVIDER_ID,
                OFFICIAL_HISTORY_PROVIDER_ID,
                Some(&thread_ids),
            )
        })?);
    }
    let restored_thread_rows = restored_thread_ids.len();

    if restored_session_files == 0 && restored_thread_rows == 0 {
        return Ok(CodexUnifiedHistoryRestoreResult {
            skipped_reason: Some("nothing_to_restore".to_string()),
            duration_ms: start.elapsed().as_millis(),
            ..empty_restore_result()
        });
    }

    let rewritten_index_entries =
        rebuild_session_index_for_state_db_paths(codex_home, &state_db_paths)?;

    Ok(CodexUnifiedHistoryRestoreResult {
        restored_session_files,
        restored_session_entries,
        restored_thread_rows,
        rewritten_index_entries,
        backup_path: Some(generation_root.to_string_lossy().to_string()),
        skipped_reason: None,
        duration_ms: start.elapsed().as_millis(),
    })
}

fn empty_migration_result() -> CodexUnifiedHistoryMigrationResult {
    CodexUnifiedHistoryMigrationResult {
        migrated_session_files: 0,
        migrated_session_entries: 0,
        migrated_thread_rows: 0,
        rewritten_index_entries: 0,
        backup_path: None,
        skipped_reason: None,
        duration_ms: 0,
    }
}

fn empty_restore_result() -> CodexUnifiedHistoryRestoreResult {
    CodexUnifiedHistoryRestoreResult {
        restored_session_files: 0,
        restored_session_entries: 0,
        restored_thread_rows: 0,
        rewritten_index_entries: 0,
        backup_path: None,
        skipped_reason: None,
        duration_ms: 0,
    }
}

#[derive(Debug, Default)]
struct SessionRewriteResult {
    changed: bool,
    changed_entries: usize,
    session_ids: BTreeSet<String>,
}

fn rewrite_session_file_provider(
    path: &Path,
    codex_home: &Path,
    backup_root: &Path,
    source_provider_id: &str,
    target_provider_id: &str,
    allowed_session_ids: Option<&BTreeSet<String>>,
) -> Result<SessionRewriteResult, String> {
    let metadata_before = fs::metadata(path)
        .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
    let modified_before = metadata_before.modified().ok();
    let len_before = metadata_before.len();
    let content = history_sync::read_session_text(path)?;

    let mut rewritten = String::with_capacity(content.len());
    let mut result = SessionRewriteResult::default();
    for segment in content.split_inclusive('\n') {
        let (line, newline) = segment
            .strip_suffix('\n')
            .map(|line| (line, "\n"))
            .unwrap_or((segment, ""));
        if let Some((next_line, session_id)) = rewrite_session_meta_line_provider(
            line,
            source_provider_id,
            target_provider_id,
            allowed_session_ids,
        )? {
            rewritten.push_str(&next_line);
            result.changed = true;
            result.changed_entries += 1;
            result.session_ids.insert(session_id);
        } else {
            rewritten.push_str(line);
        }
        rewritten.push_str(newline);
    }

    if !result.changed {
        return Ok(result);
    }

    ensure_file_unchanged(path, modified_before, len_before)?;
    backup_session_file(path, codex_home, backup_root)?;
    ensure_file_unchanged(path, modified_before, len_before)?;
    history_sync::write_session_text(path, &rewritten)?;
    Ok(result)
}

fn rewrite_session_meta_line_provider(
    line: &str,
    source_provider_id: &str,
    target_provider_id: &str,
    allowed_session_ids: Option<&BTreeSet<String>>,
) -> Result<Option<(String, String)>, String> {
    if !line.contains("\"session_meta\"") || !line.contains("\"model_provider\"") {
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
    let Some(current_provider) = payload.get("model_provider").and_then(Value::as_str) else {
        return Ok(None);
    };
    if current_provider != source_provider_id {
        return Ok(None);
    }
    let Some(session_id) = payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    else {
        return Ok(None);
    };
    if allowed_session_ids.is_some_and(|ids| !ids.contains(&session_id)) {
        return Ok(None);
    }

    payload.insert(
        "model_provider".to_string(),
        Value::String(target_provider_id.to_string()),
    );
    let next_line = serde_json::to_string(&value)
        .map_err(|error| format!("Failed to serialize Codex session metadata: {error}"))?;
    Ok(Some((next_line, session_id)))
}

fn collect_session_jsonl_files(codex_home: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_jsonl_files(&codex_home.join(SESSIONS_DIR_NAME), &mut files);
    collect_jsonl_files(&codex_home.join(ARCHIVED_SESSIONS_DIR_NAME), &mut files);
    files.sort();
    history_sync::dedupe_rollout_siblings(files)
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files);
            continue;
        }
        if history_sync::is_rollout_session_file(&path) {
            files.push(path);
        }
    }
}

fn codex_state_db_paths(codex_home: &Path, config_text: &str) -> Vec<PathBuf> {
    history_sync::discover_state_db_paths(codex_home, config_text)
}

fn rebuild_session_index_for_state_db_paths(
    codex_home: &Path,
    state_db_paths: &[PathBuf],
) -> Result<usize, String> {
    for db_path in state_db_paths.iter().rev() {
        if !db_path.exists() {
            continue;
        }
        let conn = open_read_connection(db_path)?;
        if !threads_table_has_column(&conn, "id")? {
            return Ok(0);
        }
        return history_sync::rebuild_session_index_for_db(codex_home, db_path);
    }

    Ok(0)
}

fn migrate_state_db_provider(
    db_path: &Path,
    codex_home: &Path,
    backup_root: &Path,
    source_provider_id: &str,
    target_provider_id: &str,
    allowed_thread_ids: Option<&BTreeSet<String>>,
) -> Result<BTreeSet<String>, String> {
    let mut conn = open_write_connection(db_path)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(|error| format!("Failed to set Codex database busy timeout: {error}"))?;
    let thread_ids = read_thread_ids_for_provider(&conn, source_provider_id, allowed_thread_ids)?;
    if thread_ids.is_empty() {
        return Ok(thread_ids);
    }

    backup_state_db(db_path, codex_home, backup_root, &conn)?;
    let transaction = conn
        .transaction()
        .map_err(|error| format!("Failed to begin Codex unified history transaction: {error}"))?;
    for thread_id in &thread_ids {
        transaction
            .execute(
                "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
                [target_provider_id, thread_id.as_str(), source_provider_id],
            )
            .map_err(|error| format!("Failed to update Codex history thread: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit Codex unified history transaction: {error}"))?;
    let _ = open_write_connection(db_path)?.pragma_update(None, "wal_checkpoint", "PASSIVE");
    Ok(thread_ids)
}

fn read_thread_ids_for_provider(
    conn: &Connection,
    source_provider_id: &str,
    allowed_thread_ids: Option<&BTreeSet<String>>,
) -> Result<BTreeSet<String>, String> {
    if !threads_table_has_column(conn, "id")? || !threads_table_has_column(conn, "model_provider")?
    {
        return Ok(BTreeSet::new());
    }

    let mut statement = conn
        .prepare("SELECT id FROM threads WHERE model_provider = ?1")
        .map_err(|error| format!("Failed to query Codex history threads: {error}"))?;
    let rows = statement
        .query_map([source_provider_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Failed to read Codex history thread ids: {error}"))?;

    let mut thread_ids = BTreeSet::new();
    for row in rows {
        let thread_id =
            row.map_err(|error| format!("Failed to parse Codex history thread id: {error}"))?;
        if thread_id.trim().is_empty() {
            continue;
        }
        if allowed_thread_ids.is_some_and(|allowed| !allowed.contains(&thread_id)) {
            continue;
        }
        thread_ids.insert(thread_id);
    }
    Ok(thread_ids)
}

fn threads_table_has_column(conn: &Connection, column_name: &str) -> Result<bool, String> {
    let mut statement = conn
        .prepare("PRAGMA table_info(threads)")
        .map_err(|error| format!("Failed to inspect Codex history threads schema: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed to read Codex history threads schema: {error}"))?;

    for row in rows {
        let name =
            row.map_err(|error| format!("Failed to parse Codex history schema column: {error}"))?;
        if name == column_name {
            return Ok(true);
        }
    }

    Ok(false)
}

fn retry_sqlite_write<T>(mut operation: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_sqlite_locked_error(&error) && attempts < WRITE_LOCK_RETRY_LIMIT => {
                thread::sleep(WRITE_LOCK_RETRY_DELAY);
            }
            Err(error) if is_sqlite_locked_error(&error) => {
                return Err(format!(
                    "Codex is writing local history. Wait for the current response or autosave to finish, then try again. Last error: {error}"
                ));
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_sqlite_locked_error(error: &str) -> bool {
    let value = error.to_ascii_lowercase();
    value.contains("database is locked")
        || value.contains("database table is locked")
        || value.contains("database is busy")
        || value.contains("destination database is in use")
        || value.contains("locked")
}

fn open_write_connection(db_path: &Path) -> Result<Connection, String> {
    Connection::open(db_path)
        .map_err(|error| format!("Failed to open Codex history database: {error}"))
}

fn open_read_connection(db_path: &Path) -> Result<Connection, String> {
    history_sync::open_history_read_connection(db_path)
}

fn backup_state_db(
    db_path: &Path,
    codex_home: &Path,
    backup_root: &Path,
    source: &Connection,
) -> Result<(), String> {
    let backup_path = backup_state_db_path(db_path, codex_home, backup_root);
    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Codex unified history backup dir {}: {error}",
                parent.display()
            )
        })?;
    }
    history_sync::copy_sqlite_database(source, &backup_path)
}

fn backup_state_db_path(db_path: &Path, codex_home: &Path, backup_root: &Path) -> PathBuf {
    if let Ok(relative_path) = db_path.strip_prefix(codex_home) {
        return backup_root.join("state").join(relative_path);
    }

    let file_name = db_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("state.sqlite"));
    backup_root.join("state").join("external").join(file_name)
}

fn backup_session_file(path: &Path, codex_home: &Path, backup_root: &Path) -> Result<(), String> {
    let relative_path = path.strip_prefix(codex_home).map_err(|_| {
        format!(
            "Codex session path {} is outside Codex root {}",
            path.display(),
            codex_home.display()
        )
    })?;
    let backup_path = backup_root.join("jsonl").join(relative_path);
    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Codex unified history backup dir {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::copy(path, &backup_path).map(|_| ()).map_err(|error| {
        format!(
            "Failed to backup Codex session file {}: {error}",
            path.display()
        )
    })
}

fn ensure_file_unchanged(
    path: &Path,
    modified_before: Option<SystemTime>,
    len_before: u64,
) -> Result<(), String> {
    let metadata_after = fs::metadata(path)
        .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
    if metadata_after.modified().ok() != modified_before || metadata_after.len() != len_before {
        return Err(format!(
            "Codex session file changed during unified history migration: {}",
            path.display()
        ));
    }
    Ok(())
}

fn write_atomic_with_retry(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Failed to determine parent for {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Failed to create Codex unified history target dir {}: {error}",
            parent.display()
        )
    })?;
    let tmp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("codex-unified-history"),
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

fn collect_restore_ledger(
    codex_home: &Path,
) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
    let codex_home_key = canonical_dir_string(codex_home);
    let parent = backup_parent(codex_home, MIGRATION_BACKUP_DIR_NAME);
    let mut session_ids = BTreeSet::new();
    let mut thread_ids = BTreeSet::new();
    let Ok(entries) = fs::read_dir(parent) else {
        return Ok((session_ids, thread_ids));
    };

    for entry in entries.flatten() {
        let generation_root = entry.path();
        if !generation_root.is_dir() {
            continue;
        }

        let ledger_path = generation_root.join(LEDGER_FILE_NAME);
        match read_ledger(&ledger_path) {
            Ok(ledger) => {
                if ledger.codex_home != codex_home_key {
                    continue;
                }
                session_ids.extend(ledger.session_ids);
                thread_ids.extend(ledger.thread_ids);
                continue;
            }
            Err(_) => {
                let derived = collect_restore_ledger_from_backup_generation(&generation_root)?;
                session_ids.extend(derived.0);
                thread_ids.extend(derived.1);
                continue;
            }
        };
    }

    Ok((session_ids, thread_ids))
}

fn collect_restore_ledger_from_backup_generation(
    generation_root: &Path,
) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
    let mut session_ids = BTreeSet::new();
    let mut thread_ids = BTreeSet::new();

    let mut session_files = Vec::new();
    collect_jsonl_files(&generation_root.join("jsonl"), &mut session_files);
    session_files.sort();
    for session_file in history_sync::dedupe_rollout_siblings(session_files) {
        session_ids.extend(collect_backup_session_ids_for_provider(
            &session_file,
            OFFICIAL_HISTORY_PROVIDER_ID,
        )?);
    }

    let mut state_db_files = Vec::new();
    collect_sqlite_files(&generation_root.join("state"), &mut state_db_files);
    for db_path in state_db_files {
        let conn = match open_read_connection(&db_path) {
            Ok(conn) => conn,
            Err(error) => {
                log::warn!(
                    "Skip unreadable Codex unified history backup database {}: {error}",
                    db_path.display()
                );
                continue;
            }
        };
        thread_ids.extend(read_thread_ids_for_provider(
            &conn,
            OFFICIAL_HISTORY_PROVIDER_ID,
            None,
        )?);
    }

    Ok((session_ids, thread_ids))
}

fn collect_backup_session_ids_for_provider(
    path: &Path,
    provider_id: &str,
) -> Result<BTreeSet<String>, String> {
    let content = history_sync::read_session_text(path).map_err(|error| {
        format!(
            "Failed to read Codex unified history backup session file {}: {error}",
            path.display()
        )
    })?;
    let mut session_ids = BTreeSet::new();
    for line in content.lines() {
        if let Some(session_id) = read_session_meta_id_for_provider(line, provider_id)? {
            session_ids.insert(session_id);
        }
    }

    Ok(session_ids)
}

fn read_session_meta_id_for_provider(
    line: &str,
    provider_id: &str,
) -> Result<Option<String>, String> {
    if !line.contains("\"session_meta\"") || !line.contains("\"model_provider\"") {
        return Ok(None);
    }

    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let Some(payload) = value.get("payload").and_then(Value::as_object) else {
        return Ok(None);
    };
    if payload.get("model_provider").and_then(Value::as_str) != Some(provider_id) {
        return Ok(None);
    }

    Ok(payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string))
}

fn collect_sqlite_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sqlite_files(&path, files);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) == Some("sqlite") {
            files.push(path);
        }
    }
    files.sort();
}

fn read_ledger(path: &Path) -> Result<UnifiedHistoryLedger, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read Codex unified history ledger: {error}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse Codex unified history ledger: {error}"))
}

fn write_ledger(backup_root: &Path, ledger: &UnifiedHistoryLedger) -> Result<(), String> {
    fs::create_dir_all(backup_root).map_err(|error| {
        format!(
            "Failed to create Codex unified history backup dir {}: {error}",
            backup_root.display()
        )
    })?;
    let bytes = serde_json::to_vec_pretty(ledger)
        .map_err(|error| format!("Failed to serialize Codex unified history ledger: {error}"))?;
    write_atomic_with_retry(&backup_root.join(LEDGER_FILE_NAME), &bytes)
}

fn backup_parent(codex_home: &Path, dir_name: &str) -> PathBuf {
    codex_home.join("history_sync_backups").join(dir_name)
}

fn backup_generation_root(codex_home: &Path, dir_name: &str) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    backup_parent(codex_home, dir_name).join(format!("{}-{}", timestamp, Uuid::new_v4().simple()))
}

fn canonical_dir_string(dir: &Path) -> String {
    fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn injects_official_custom_bucket_when_config_has_no_provider() {
        let injected = inject_unified_session_history_config("model = \"gpt-5\"\n")
            .expect("inject unified history");
        let document = injected.parse::<DocumentMut>().expect("parse injected");

        assert_eq!(
            document
                .get("model_provider")
                .and_then(|item| item.as_str()),
            Some(UNIFIED_HISTORY_PROVIDER_ID)
        );
        let custom = document["model_providers"][UNIFIED_HISTORY_PROVIDER_ID]
            .as_table()
            .expect("custom provider table");
        assert!(table_matches_unified_official_provider(custom));
    }

    #[test]
    fn inject_does_not_override_explicit_provider_or_conflicting_custom_table() {
        let explicit = "model_provider = \"openai\"\n";
        assert_eq!(
            inject_unified_session_history_config(explicit).expect("inject explicit"),
            explicit
        );

        let conflicting = r#"model = "gpt-5"

[model_providers.custom]
name = "Third Party"
base_url = "https://example.com/v1"
"#;
        assert_eq!(
            inject_unified_session_history_config(conflicting).expect("inject conflict"),
            conflicting
        );
    }

    #[test]
    fn strip_removes_only_exact_injected_bucket() {
        let injected = inject_unified_session_history_config("").expect("inject");
        let stripped = strip_unified_session_history_config(&injected).expect("strip");
        assert!(!stripped.contains("model_provider"));
        assert!(!stripped.contains("model_providers"));

        let third_party = r#"model_provider = "custom"

[model_providers.custom]
name = "Third Party"
base_url = "https://example.com/v1"
"#;
        assert_eq!(
            strip_unified_session_history_config(third_party).expect("strip third party"),
            third_party
        );
    }

    #[test]
    fn migrates_and_restores_only_ledgered_official_history() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path();
        fs::write(
            codex_home.join(CONFIG_FILE_NAME),
            inject_unified_session_history_config("").expect("inject"),
        )
        .expect("write config");

        let sessions_dir = codex_home.join(SESSIONS_DIR_NAME).join("2026").join("06");
        fs::create_dir_all(&sessions_dir).expect("create sessions");
        let official_session = sessions_dir.join("rollout-official.jsonl");
        fs::write(
            &official_session,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"openai\"}}\n{\"type\":\"response_item\",\"payload\":{\"text\":\"hello\"}}\n",
        )
        .expect("write official session");
        let custom_session = sessions_dir.join("rollout-custom.jsonl");
        fs::write(
            &custom_session,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s2\",\"model_provider\":\"custom\"}}\n",
        )
        .expect("write custom session");

        let db_path = codex_home.join(STATE_DB_FILE_NAME);
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "
            CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, updated_at INTEGER, model_provider TEXT);
            INSERT INTO threads (id, title, updated_at, model_provider) VALUES
                ('s1', 'official', 1, 'openai'),
                ('s2', 'custom', 2, 'custom');
            ",
        )
        .expect("seed db");
        drop(conn);

        let migration =
            migrate_official_history_to_unified(codex_home).expect("migrate official history");
        assert_eq!(migration.migrated_session_files, 1);
        assert_eq!(migration.migrated_thread_rows, 1);
        assert!(has_codex_unified_history_backup(codex_home));
        let migrated_official = fs::read_to_string(&official_session).expect("read official");
        assert!(migrated_official.contains("\"model_provider\":\"custom\""));
        let conn = Connection::open(&db_path).expect("open db after migrate");
        let openai_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE model_provider = 'openai'",
                [],
                |row| row.get(0),
            )
            .expect("count openai");
        assert_eq!(openai_count, 0);
        drop(conn);

        let on_period_session = sessions_dir.join("rollout-on-period.jsonl");
        fs::write(
            &on_period_session,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s3\",\"model_provider\":\"custom\"}}\n",
        )
        .expect("write on-period session");
        let conn = Connection::open(&db_path).expect("open db add on-period");
        conn.execute(
            "INSERT INTO threads (id, title, updated_at, model_provider) VALUES ('s3', 'new', 3, 'custom')",
            [],
        )
        .expect("insert on-period");
        drop(conn);

        let restore = restore_official_history_from_unified_backups(codex_home, false)
            .expect("restore official history");
        assert_eq!(restore.restored_session_files, 1);
        assert_eq!(restore.restored_thread_rows, 1);
        let restored_official = fs::read_to_string(&official_session).expect("read restored");
        assert!(restored_official.contains("\"model_provider\":\"openai\""));
        let on_period = fs::read_to_string(&on_period_session).expect("read on-period");
        assert!(on_period.contains("\"model_provider\":\"custom\""));
        let conn = Connection::open(&db_path).expect("open db after restore");
        let provider_of = |id: &str| -> String {
            conn.query_row(
                "SELECT model_provider FROM threads WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("provider")
        };
        assert_eq!(provider_of("s1"), "openai");
        assert_eq!(provider_of("s2"), "custom");
        assert_eq!(provider_of("s3"), "custom");
    }

    #[test]
    fn migration_uses_config_sqlite_home_state_db() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path().join(".codex");
        let sqlite_home = temp_dir.path().join("sqlite-home");
        fs::create_dir_all(&codex_home).expect("create codex home");
        fs::create_dir_all(&sqlite_home).expect("create sqlite home");
        let sqlite_home_toml = sqlite_home.to_string_lossy().replace('\\', "\\\\");
        let config = format!(
            "sqlite_home = \"{}\"\n{}",
            sqlite_home_toml,
            inject_unified_session_history_config("").expect("inject")
        );
        fs::write(codex_home.join(CONFIG_FILE_NAME), config).expect("write config");

        let sessions_dir = codex_home.join(SESSIONS_DIR_NAME).join("2026").join("06");
        fs::create_dir_all(&sessions_dir).expect("create sessions");
        let official_session = sessions_dir.join("rollout-official.jsonl");
        fs::write(
            &official_session,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"openai\"}}\n",
        )
        .expect("write official session");

        let db_path = sqlite_home.join(STATE_DB_FILE_NAME);
        let conn = Connection::open(&db_path).expect("open external db");
        conn.execute_batch(
            "
            CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, updated_at INTEGER, model_provider TEXT);
            INSERT INTO threads (id, title, updated_at, model_provider) VALUES
                ('s1', 'official', 1, 'openai');
            ",
        )
        .expect("seed external db");
        drop(conn);

        let migration =
            migrate_official_history_to_unified(&codex_home).expect("migrate external state db");
        assert_eq!(migration.migrated_thread_rows, 1);
        let conn = Connection::open(&db_path).expect("open external db after migrate");
        let migrated_provider: String = conn
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .expect("provider after migrate");
        assert_eq!(migrated_provider, "custom");
        drop(conn);

        let restore = restore_official_history_from_unified_backups(&codex_home, false)
            .expect("restore external state db");
        assert_eq!(restore.restored_thread_rows, 1);
        let conn = Connection::open(&db_path).expect("open external db after restore");
        let restored_provider: String = conn
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .expect("provider after restore");
        assert_eq!(restored_provider, "openai");
    }

    #[test]
    fn migration_ignores_state_db_without_threads_schema() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path();
        fs::write(
            codex_home.join(CONFIG_FILE_NAME),
            inject_unified_session_history_config("").expect("inject"),
        )
        .expect("write config");

        let sessions_dir = codex_home.join(SESSIONS_DIR_NAME).join("2026").join("06");
        fs::create_dir_all(&sessions_dir).expect("create sessions");
        let official_session = sessions_dir.join("rollout-official.jsonl");
        fs::write(
            &official_session,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"openai\"}}\n",
        )
        .expect("write official session");

        let conn = Connection::open(codex_home.join(STATE_DB_FILE_NAME)).expect("open db");
        conn.execute_batch("CREATE TABLE unrelated (id TEXT PRIMARY KEY);")
            .expect("seed unrelated schema");
        drop(conn);

        let migration = migrate_official_history_to_unified(codex_home)
            .expect("migrate with unrelated state db schema");
        assert_eq!(migration.migrated_session_files, 1);
        assert_eq!(migration.migrated_thread_rows, 0);
    }

    #[test]
    fn restore_can_recover_from_backups_without_ledger() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path();
        fs::write(
            codex_home.join(CONFIG_FILE_NAME),
            inject_unified_session_history_config("").expect("inject"),
        )
        .expect("write config");

        let sessions_dir = codex_home.join(SESSIONS_DIR_NAME).join("2026").join("06");
        fs::create_dir_all(&sessions_dir).expect("create sessions");
        let official_session = sessions_dir.join("rollout-official.jsonl");
        fs::write(
            &official_session,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"openai\"}}\n",
        )
        .expect("write official session");

        let db_path = codex_home.join(STATE_DB_FILE_NAME);
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "
            CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, updated_at INTEGER, model_provider TEXT);
            INSERT INTO threads (id, title, updated_at, model_provider) VALUES
                ('s1', 'official', 1, 'openai');
            ",
        )
        .expect("seed db");
        drop(conn);

        migrate_official_history_to_unified(codex_home).expect("migrate official history");
        let parent = backup_parent(codex_home, MIGRATION_BACKUP_DIR_NAME);
        for entry in fs::read_dir(parent).expect("read backups").flatten() {
            let _ = fs::remove_file(entry.path().join(LEDGER_FILE_NAME));
        }

        assert!(has_codex_unified_history_backup(codex_home));
        let restore = restore_official_history_from_unified_backups(codex_home, false)
            .expect("restore without ledger");
        assert_eq!(restore.restored_session_files, 1);
        assert_eq!(restore.restored_thread_rows, 1);
    }

    fn set_session_mtime(path: &Path, seconds: u64) {
        let file = if cfg!(windows) {
            fs::File::options().write(true).open(path)
        } else {
            fs::File::open(path)
        };
        file.expect("open session for mtime")
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
            )
            .expect("set session mtime");
    }

    fn session_mtime(path: &Path) -> SystemTime {
        fs::metadata(path)
            .expect("session metadata")
            .modified()
            .expect("session modified time")
    }

    fn write_compressed_session(path: &Path, text: &str) {
        fs::write(
            path,
            zstd::stream::encode_all(text.as_bytes(), 3).expect("compress session"),
        )
        .expect("write compressed session");
    }

    #[test]
    fn migration_uses_highest_state_db_version() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path();
        fs::write(
            codex_home.join(CONFIG_FILE_NAME),
            inject_unified_session_history_config("").expect("inject"),
        )
        .expect("write config");

        let seed_db = |name: &str, id: &str| {
            let conn = Connection::open(codex_home.join(name)).expect("open db");
            conn.execute_batch(&format!(
                "
                CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, updated_at INTEGER, model_provider TEXT);
                INSERT INTO threads (id, title, updated_at, model_provider) VALUES
                    ('{id}', 'official', 1, 'openai');
                "
            ))
            .expect("seed db");
        };
        seed_db("state_5.sqlite", "s5");
        seed_db("state_6.sqlite", "s6");

        let migration =
            migrate_official_history_to_unified(codex_home).expect("migrate highest state db");

        assert_eq!(migration.migrated_thread_rows, 1);
        let provider_of = |db_name: &str, id: &str| -> String {
            let conn = Connection::open(codex_home.join(db_name)).expect("open db");
            conn.query_row(
                "SELECT model_provider FROM threads WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("provider")
        };
        assert_eq!(provider_of("state_6.sqlite", "s6"), "custom");
        assert_eq!(provider_of("state_5.sqlite", "s5"), "openai");
    }

    #[test]
    fn migrates_and_restores_compressed_rollout_preserving_mtime() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path();
        fs::write(
            codex_home.join(CONFIG_FILE_NAME),
            inject_unified_session_history_config("").expect("inject"),
        )
        .expect("write config");

        let sessions_dir = codex_home.join(SESSIONS_DIR_NAME).join("2026").join("06");
        fs::create_dir_all(&sessions_dir).expect("create sessions");
        let session_path = sessions_dir.join("rollout-compressed.jsonl.zst");
        write_compressed_session(
            &session_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"openai\"}}\n{\"type\":\"response_item\",\"payload\":{\"text\":\"hello\"}}\n",
        );
        let stamp = 1_600_000_000_u64;
        let expected_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(stamp);
        set_session_mtime(&session_path, stamp);

        let db_path = codex_home.join(STATE_DB_FILE_NAME);
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "
            CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, updated_at INTEGER, model_provider TEXT);
            INSERT INTO threads (id, title, updated_at, model_provider) VALUES
                ('s1', 'official', 1, 'openai');
            ",
        )
        .expect("seed db");
        drop(conn);

        let migration =
            migrate_official_history_to_unified(codex_home).expect("migrate compressed rollout");
        assert_eq!(migration.migrated_session_files, 1);
        assert!(session_path.exists());
        assert!(!session_path
            .with_file_name("rollout-compressed.jsonl")
            .exists());
        let migrated = history_sync::read_session_text(&session_path).expect("read migrated");
        assert!(migrated.contains("\"model_provider\":\"custom\""));
        assert_eq!(session_mtime(&session_path), expected_mtime);

        let restore = restore_official_history_from_unified_backups(codex_home, false)
            .expect("restore compressed rollout");
        assert_eq!(restore.restored_session_files, 1);
        let restored = history_sync::read_session_text(&session_path).expect("read restored");
        assert!(restored.contains("\"model_provider\":\"openai\""));
        assert_eq!(session_mtime(&session_path), expected_mtime);
    }

    #[test]
    fn external_state_db_backup_keeps_actual_file_name() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path().join(".codex");
        let db_path = temp_dir.path().join("sqlite-home").join("state_7.sqlite");

        let backup_path = backup_state_db_path(&db_path, &codex_home, temp_dir.path());

        assert_eq!(
            backup_path,
            temp_dir
                .path()
                .join("state")
                .join("external")
                .join("state_7.sqlite")
        );
    }

    #[test]
    fn migration_dedupes_plain_and_compressed_rollout_siblings() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let codex_home = temp_dir.path();
        fs::write(
            codex_home.join(CONFIG_FILE_NAME),
            inject_unified_session_history_config("").expect("inject"),
        )
        .expect("write config");

        let sessions_dir = codex_home.join(SESSIONS_DIR_NAME).join("2026").join("06");
        fs::create_dir_all(&sessions_dir).expect("create sessions");
        let session_text = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"openai\"}}\n";
        let plain_path = sessions_dir.join("rollout-sibling.jsonl");
        let compressed_path = sessions_dir.join("rollout-sibling.jsonl.zst");
        fs::write(&plain_path, session_text).expect("write plain session");
        write_compressed_session(&compressed_path, session_text);

        let db_path = codex_home.join(STATE_DB_FILE_NAME);
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "
            CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, updated_at INTEGER, model_provider TEXT);
            INSERT INTO threads (id, title, updated_at, model_provider) VALUES
                ('s1', 'official', 1, 'openai');
            ",
        )
        .expect("seed db");
        drop(conn);

        let migration =
            migrate_official_history_to_unified(codex_home).expect("migrate sibling rollouts");

        assert_eq!(migration.migrated_session_files, 1);
        let plain = fs::read_to_string(&plain_path).expect("read plain session");
        assert!(plain.contains("\"model_provider\":\"custom\""));
        let compressed =
            history_sync::read_session_text(&compressed_path).expect("read compressed sibling");
        assert!(compressed.contains("\"model_provider\":\"openai\""));
    }
}
