use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::helpers::{db_count, db_delete_all, db_put, db_transaction};
use super::schema::{DbTable, ALL_TABLES};
use super::sqlite_state::SqliteDbState;

pub const LEGACY_DATABASE_DIR: &str = "database";
pub const SQLITE_DATABASE_FILE: &str = "ai-toolbox.db";
pub const SQLITE_MIGRATION_COMPLETE_FLAG: &str = "sqlite-migration-complete.flag";
pub const LEGACY_DATABASE_ARCHIVE: &str = "database.migrated.zip";
pub const MIGRATION_LOG_FILE: &str = "migration.log";
pub const MIGRATION_WARNINGS_FILE: &str = "migration_warnings.log";
pub const MIGRATION_FAILURES_FILE: &str = "sqlite-migration-failures.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableImportReport {
    pub table: &'static str,
    pub surreal_count: usize,
    pub sqlite_count: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurrealImportReport {
    pub tables: Vec<TableImportReport>,
}

impl SurrealImportReport {
    pub fn total_records(&self) -> usize {
        self.tables.iter().map(|table| table.surreal_count).sum()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationPaths {
    pub app_data_dir: PathBuf,
    pub legacy_database_dir: PathBuf,
    pub sqlite_database_file: PathBuf,
    pub sqlite_wal_file: PathBuf,
    pub sqlite_shm_file: PathBuf,
    pub complete_flag: PathBuf,
    pub legacy_archive: PathBuf,
    pub migration_log: PathBuf,
    pub migration_warnings: PathBuf,
    pub migration_failures: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MigrationFailureState {
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_failed_at: Option<String>,
}

impl MigrationPaths {
    pub fn new(app_data_dir: impl AsRef<Path>) -> Self {
        let app_data_dir = app_data_dir.as_ref().to_path_buf();
        let sqlite_database_file = app_data_dir.join(SQLITE_DATABASE_FILE);

        Self {
            legacy_database_dir: app_data_dir.join(LEGACY_DATABASE_DIR),
            sqlite_wal_file: sqlite_database_file.with_extension("db-wal"),
            sqlite_shm_file: sqlite_database_file.with_extension("db-shm"),
            complete_flag: app_data_dir.join(SQLITE_MIGRATION_COMPLETE_FLAG),
            legacy_archive: app_data_dir.join(LEGACY_DATABASE_ARCHIVE),
            migration_log: app_data_dir.join(MIGRATION_LOG_FILE),
            migration_warnings: app_data_dir.join(MIGRATION_WARNINGS_FILE),
            migration_failures: app_data_dir.join(MIGRATION_FAILURES_FILE),
            sqlite_database_file,
            app_data_dir,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupMigrationState {
    NewInstall,
    NeedsSurrealImport,
    IncompleteImport,
    NeedsLegacyArchive,
    Ready,
}

pub fn detect_startup_migration_state(paths: &MigrationPaths) -> StartupMigrationState {
    let has_legacy_database = paths.legacy_database_dir.exists();
    let has_sqlite_database = paths.sqlite_database_file.exists();
    let has_complete_flag = paths.complete_flag.exists();

    match (has_legacy_database, has_sqlite_database, has_complete_flag) {
        (false, false, _) => StartupMigrationState::NewInstall,
        (true, false, _) => StartupMigrationState::NeedsSurrealImport,
        (true, true, false) => StartupMigrationState::IncompleteImport,
        (true, true, true) => StartupMigrationState::NeedsLegacyArchive,
        (false, true, _) => StartupMigrationState::Ready,
    }
}

pub fn cleanup_incomplete_sqlite_database(paths: &MigrationPaths) -> Result<(), String> {
    remove_file_if_exists(&paths.sqlite_database_file)?;
    remove_file_if_exists(&paths.sqlite_wal_file)?;
    remove_file_if_exists(&paths.sqlite_shm_file)?;
    remove_file_if_exists(&paths.complete_flag)?;
    Ok(())
}

pub fn mark_sqlite_import_complete(paths: &MigrationPaths) -> Result<(), String> {
    if let Some(parent) = paths.complete_flag.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create migration flag parent directory {}: {error}",
                parent.display()
            )
        })?;
    }

    fs::write(&paths.complete_flag, b"ok").map_err(|error| {
        format!(
            "Failed to write SQLite migration complete flag {}: {error}",
            paths.complete_flag.display()
        )
    })
}

pub fn archive_legacy_database(paths: &MigrationPaths) -> Result<(), String> {
    if !paths.legacy_database_dir.exists() {
        remove_file_if_exists(&paths.complete_flag)?;
        return Ok(());
    }

    if paths.legacy_archive.exists() {
        fs::remove_file(&paths.legacy_archive).map_err(|error| {
            format!(
                "Failed to replace legacy database archive {}: {error}",
                paths.legacy_archive.display()
            )
        })?;
    }

    if let Some(parent) = paths.legacy_archive.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create legacy database archive parent {}: {error}",
                parent.display()
            )
        })?;
    }

    let archive_file = fs::File::create(&paths.legacy_archive).map_err(|error| {
        format!(
            "Failed to create legacy database archive {}: {error}",
            paths.legacy_archive.display()
        )
    })?;
    let mut zip = ZipWriter::new(archive_file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    add_directory_to_zip(
        &mut zip,
        &paths.legacy_database_dir,
        &paths.legacy_database_dir,
        options,
    )?;

    zip.finish().map_err(|error| {
        format!(
            "Failed to finish legacy database archive {}: {error}",
            paths.legacy_archive.display()
        )
    })?;

    fs::remove_dir_all(&paths.legacy_database_dir).map_err(|error| {
        format!(
            "Failed to remove archived legacy database directory {}: {error}",
            paths.legacy_database_dir.display()
        )
    })?;
    remove_file_if_exists(&paths.complete_flag)?;
    Ok(())
}

pub fn read_migration_failure_state(paths: &MigrationPaths) -> MigrationFailureState {
    fs::read_to_string(&paths.migration_failures)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub fn record_migration_failure(
    paths: &MigrationPaths,
    error: &str,
) -> Result<MigrationFailureState, String> {
    let mut state = read_migration_failure_state(paths);
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.last_error = Some(error.to_string());
    state.last_failed_at = Some(Local::now().to_rfc3339());

    if let Some(parent) = paths.migration_failures.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create migration failure parent directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let content = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("Failed to serialize migration failure state: {error}"))?;
    fs::write(&paths.migration_failures, content).map_err(|error| {
        format!(
            "Failed to write migration failure state {}: {error}",
            paths.migration_failures.display()
        )
    })?;
    Ok(state)
}

pub fn clear_migration_failure_state(paths: &MigrationPaths) -> Result<(), String> {
    remove_file_if_exists(&paths.migration_failures)
}

pub fn write_migration_log(paths: &MigrationPaths, message: &str) -> Result<(), String> {
    append_line(&paths.migration_log, message)
}

pub fn write_migration_warning(paths: &MigrationPaths, message: &str) -> Result<(), String> {
    append_line(&paths.migration_warnings, message)
}

pub async fn import_all_known_tables_from_surreal(
    sqlite_state: &SqliteDbState,
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<SurrealImportReport, String> {
    import_tables_from_surreal(sqlite_state, db, ALL_TABLES).await
}

pub async fn import_missing_known_tables_from_surreal(
    sqlite_state: &SqliteDbState,
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<SurrealImportReport, String> {
    let mut missing_tables = Vec::new();
    for table in ALL_TABLES {
        let sqlite_count = sqlite_state.with_conn(|conn| db_count(conn, *table))?;
        if sqlite_count == 0 {
            missing_tables.push(*table);
        }
    }

    import_tables_from_surreal(sqlite_state, db, &missing_tables).await
}

pub async fn import_tables_from_surreal(
    sqlite_state: &SqliteDbState,
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tables: &[DbTable],
) -> Result<SurrealImportReport, String> {
    let mut table_records = Vec::with_capacity(tables.len());
    for table in tables {
        let records = read_surreal_table(db, *table).await?;
        table_records.push((*table, records));
    }

    sqlite_state.with_conn_mut(|conn| {
        db_transaction(conn, |tx| {
            for (table, records) in &table_records {
                db_delete_all(tx, *table)?;
                for record in records {
                    let (id, payload) = normalize_surreal_record(*table, record)?;
                    db_put(tx, *table, &id, &payload)?;
                }
            }
            Ok(())
        })
    })?;

    let mut reports = Vec::with_capacity(table_records.len());
    for (table, records) in table_records {
        let sqlite_count = sqlite_state.with_conn(|conn| db_count(conn, table))?;
        if sqlite_count != records.len() as i64 {
            return Err(format!(
                "Migration count mismatch for {}: SurrealDB={}, SQLite={}",
                table.name(),
                records.len(),
                sqlite_count
            ));
        }
        reports.push(TableImportReport {
            table: table.name(),
            surreal_count: records.len(),
            sqlite_count,
        });
    }

    Ok(SurrealImportReport { tables: reports })
}

async fn read_surreal_table(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    table: DbTable,
) -> Result<Vec<Value>, String> {
    let mut result = db
        .query(format!(
            "SELECT *, type::string(id) AS id FROM {}",
            table.name()
        ))
        .await
        .map_err(|error| format!("Failed to query SurrealDB table {}: {error}", table.name()))?;

    result
        .take(0)
        .map_err(|error| format!("Failed to parse SurrealDB table {}: {error}", table.name()))
}

fn normalize_surreal_record(table: DbTable, record: &Value) -> Result<(String, Value), String> {
    let raw_id = record
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("SurrealDB record in {} is missing string id", table.name()))?;
    let id = clean_surreal_id(raw_id);
    if id.trim().is_empty() {
        return Err(format!("SurrealDB record in {} has empty id", table.name()));
    }

    let mut payload = record.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("id");
    } else {
        return Err(format!(
            "SurrealDB record in {} must be a JSON object",
            table.name()
        ));
    }

    Ok((id, payload))
}

fn clean_surreal_id(raw_id: &str) -> String {
    let without_prefix = if let Some(pos) = raw_id.find(':') {
        &raw_id[pos + 1..]
    } else {
        raw_id
    };

    without_prefix
        .trim_start_matches('⟨')
        .trim_end_matches('⟩')
        .trim_start_matches('`')
        .trim_end_matches('`')
        .to_string()
}

fn append_line(path: &Path, message: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create migration log parent directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut existing = if path.exists() {
        fs::read_to_string(path)
            .map_err(|error| format!("Failed to read migration log {}: {error}", path.display()))?
    } else {
        String::new()
    };
    existing.push_str(message);
    existing.push('\n');
    fs::write(path, existing)
        .map_err(|error| format!("Failed to write migration log {}: {error}", path.display()))
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Failed to remove {}: {error}", path.display())),
    }
}

fn add_directory_to_zip(
    zip: &mut ZipWriter<fs::File>,
    root: &Path,
    current: &Path,
    options: SimpleFileOptions,
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(current)
        .map_err(|error| format!("Failed to read directory {}: {error}", current.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|error| {
            format!(
                "Failed to build archive-relative path for {}: {error}",
                path.display()
            )
        })?;
        let archive_name = Path::new(LEGACY_DATABASE_DIR).join(relative);
        let archive_name = archive_name.to_string_lossy().replace('\\', "/");

        if path.is_dir() {
            if !archive_name.is_empty() {
                zip.add_directory(format!("{archive_name}/"), options)
                    .map_err(|error| {
                        format!("Failed to add directory {archive_name} to archive: {error}")
                    })?;
            }
            add_directory_to_zip(zip, root, &path, options)?;
        } else {
            zip.start_file(&archive_name, options).map_err(|error| {
                format!("Failed to add file {archive_name} to archive: {error}")
            })?;
            let mut file = fs::File::open(&path)
                .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)
                .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
            zip.write_all(&buffer).map_err(|error| {
                format!("Failed to write {archive_name} to legacy database archive: {error}")
            })?;
        }
    }

    Ok(())
}
