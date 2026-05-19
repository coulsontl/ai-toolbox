use ai_toolbox_lib::db::surreal_import::{
    archive_legacy_database, cleanup_incomplete_sqlite_database, clear_migration_failure_state,
    detect_startup_migration_state, mark_sqlite_import_complete, read_migration_failure_state,
    record_migration_failure, write_migration_log, write_migration_warning, MigrationPaths,
    StartupMigrationState, LEGACY_DATABASE_DIR, SQLITE_DATABASE_FILE,
};

fn paths(temp_dir: &tempfile::TempDir) -> MigrationPaths {
    MigrationPaths::new(temp_dir.path())
}

#[test]
fn detects_new_install_when_no_database_exists() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);

    assert_eq!(
        detect_startup_migration_state(&paths),
        StartupMigrationState::NewInstall
    );
}

#[test]
fn detects_initial_surreal_import_when_only_legacy_database_exists() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);
    std::fs::create_dir(paths.app_data_dir.join(LEGACY_DATABASE_DIR)).expect("legacy dir");

    assert_eq!(
        detect_startup_migration_state(&paths),
        StartupMigrationState::NeedsSurrealImport
    );
}

#[test]
fn detects_incomplete_import_when_legacy_and_sqlite_exist_without_flag() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);
    std::fs::create_dir(&paths.legacy_database_dir).expect("legacy dir");
    std::fs::write(&paths.sqlite_database_file, b"partial").expect("sqlite file");

    assert_eq!(
        detect_startup_migration_state(&paths),
        StartupMigrationState::IncompleteImport
    );
}

#[test]
fn detects_legacy_archive_step_when_complete_flag_exists() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);
    std::fs::create_dir(&paths.legacy_database_dir).expect("legacy dir");
    std::fs::write(&paths.sqlite_database_file, b"sqlite").expect("sqlite file");
    mark_sqlite_import_complete(&paths).expect("complete flag");

    assert_eq!(
        detect_startup_migration_state(&paths),
        StartupMigrationState::NeedsLegacyArchive
    );
}

#[test]
fn detects_ready_when_only_sqlite_database_exists() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);
    std::fs::write(paths.app_data_dir.join(SQLITE_DATABASE_FILE), b"sqlite").expect("sqlite file");

    assert_eq!(
        detect_startup_migration_state(&paths),
        StartupMigrationState::Ready
    );
}

#[test]
fn cleanup_incomplete_sqlite_database_removes_db_wal_shm_and_flag() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);

    std::fs::write(&paths.sqlite_database_file, b"db").expect("db");
    std::fs::write(&paths.sqlite_wal_file, b"wal").expect("wal");
    std::fs::write(&paths.sqlite_shm_file, b"shm").expect("shm");
    mark_sqlite_import_complete(&paths).expect("flag");

    cleanup_incomplete_sqlite_database(&paths).expect("cleanup");

    assert!(!paths.sqlite_database_file.exists());
    assert!(!paths.sqlite_wal_file.exists());
    assert!(!paths.sqlite_shm_file.exists());
    assert!(!paths.complete_flag.exists());
}

#[test]
fn migration_log_and_warning_are_written_to_files() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);

    write_migration_log(&paths, "started").expect("write log");
    write_migration_log(&paths, "finished").expect("append log");
    write_migration_warning(&paths, "unknown empty table").expect("write warning");

    let log = std::fs::read_to_string(&paths.migration_log).expect("read log");
    let warning = std::fs::read_to_string(&paths.migration_warnings).expect("read warning");

    assert!(log.contains("started"));
    assert!(log.contains("finished"));
    assert!(warning.contains("unknown empty table"));
}

#[test]
fn archive_legacy_database_zips_directory_and_removes_original() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);
    std::fs::create_dir_all(paths.legacy_database_dir.join("kv")).expect("legacy dir");
    std::fs::write(paths.legacy_database_dir.join("kv").join("data"), "legacy")
        .expect("legacy file");
    mark_sqlite_import_complete(&paths).expect("complete flag");

    archive_legacy_database(&paths).expect("archive legacy database");

    assert!(!paths.legacy_database_dir.exists());
    assert!(!paths.complete_flag.exists());
    assert!(paths.legacy_archive.exists());

    let archive = std::fs::File::open(&paths.legacy_archive).expect("archive file");
    let mut zip = zip::ZipArchive::new(archive).expect("zip archive");
    let mut archived_file = zip.by_name("database/kv/data").expect("archived file");
    let mut content = String::new();
    use std::io::Read;
    archived_file
        .read_to_string(&mut content)
        .expect("read archived file");
    assert_eq!(content, "legacy");
}

#[test]
fn migration_failure_state_counts_consecutive_failures_and_can_clear() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let paths = paths(&temp_dir);

    let first = record_migration_failure(&paths, "first").expect("first failure");
    assert_eq!(first.consecutive_failures, 1);
    assert_eq!(first.last_error.as_deref(), Some("first"));

    let second = record_migration_failure(&paths, "second").expect("second failure");
    assert_eq!(second.consecutive_failures, 2);
    assert_eq!(second.last_error.as_deref(), Some("second"));

    assert_eq!(read_migration_failure_state(&paths).consecutive_failures, 2);

    clear_migration_failure_state(&paths).expect("clear failures");
    assert_eq!(read_migration_failure_state(&paths).consecutive_failures, 0);
}
