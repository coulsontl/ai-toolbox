use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::Connection;
use uuid::Uuid;

use super::{backup, health, migrations, model_pricing_seed};

pub const SQLITE_MIGRATION_BACKUP_DIR: &str = "sqlite-migration-backups";

/// How many pre-migration snapshots to keep.
///
/// Each one is a full copy of the database, so on a real install the directory
/// grows by a database per upgrade and is never trimmed by anything else. Five
/// is enough to step back through a bad upgrade without keeping months of
/// near-identical copies.
const MIGRATION_BACKUP_KEEP: usize = 5;

/// The prefix every pre-migration snapshot carries.
///
/// Retention matches on this rather than on the directory, because the same
/// directory also holds `ai-toolbox-session-usage-rebuild-*` snapshots. Those
/// are taken before a destructive import, are rare, and are the only copy of
/// what the import replaced -- trimming them by count would delete the one
/// that matters just because a few upgrades happened afterwards.
const MIGRATION_BACKUP_PREFIX: &str = "ai-toolbox-schema-";

#[derive(Clone)]
pub struct SqliteDbState {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl SqliteDbState {
    pub fn open(db_path: PathBuf) -> Result<Self, String> {
        let mut conn = Connection::open(&db_path).map_err(|error| {
            format!(
                "Failed to open SQLite database {}: {error}",
                db_path.display()
            )
        })?;
        initialize_file_connection(&mut conn, &db_path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path,
        })
    }

    pub fn in_memory_for_test() -> Result<Self, String> {
        let mut conn = Connection::open_in_memory()
            .map_err(|error| format!("Failed to open in-memory SQLite database: {error}"))?;
        initialize_connection(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: PathBuf::from(":memory:"),
        })
    }

    pub fn with_conn<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| "SQLite connection mutex is poisoned".to_string())?;
        operation(&conn)
    }

    pub fn with_conn_mut<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| "SQLite connection mutex is poisoned".to_string())?;
        operation(&mut conn)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn db(&self) -> &Self {
        self
    }
}

pub fn initialize_connection(conn: &mut Connection) -> Result<(), String> {
    initialize_connection_inner(conn, None)
}

fn initialize_file_connection(conn: &mut Connection, db_path: &Path) -> Result<(), String> {
    initialize_connection_inner(conn, Some(db_path))
}

fn initialize_connection_inner(
    conn: &mut Connection,
    db_path_for_migration_backup: Option<&Path>,
) -> Result<(), String> {
    let current_version = migrations::ensure_supported_user_version(conn)?;

    conn.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(|error| format!("Failed to set SQLite busy timeout: {error}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA cache_size = -8000;",
    )
    .map_err(|error| format!("Failed to initialize SQLite PRAGMA settings: {error}"))?;

    health::verify_jsonb_support(conn)?;
    if let Some(db_path) = db_path_for_migration_backup {
        create_pre_migration_backup_if_needed(conn, db_path, current_version)?;
    }
    migrations::run_all(conn)?;
    let inserted_pricing_count = model_pricing_seed::ensure_seeded(conn)?;
    if inserted_pricing_count > 0 {
        log::info!(
            "[ModelPricing] Seeded {} missing pricing rows",
            inserted_pricing_count
        );
    }
    health::quick_check(conn)?;
    Ok(())
}

fn create_pre_migration_backup_if_needed(
    conn: &Connection,
    db_path: &Path,
    current_version: i32,
) -> Result<Option<PathBuf>, String> {
    if current_version <= 0 || current_version >= migrations::TARGET_SCHEMA_VERSION {
        return Ok(None);
    }

    let db_parent = db_path.parent().ok_or_else(|| {
        format!(
            "Failed to resolve SQLite database parent directory for {}",
            db_path.display()
        )
    })?;
    let backup_dir = db_parent.join(SQLITE_MIGRATION_BACKUP_DIR);
    fs::create_dir_all(&backup_dir).map_err(|error| {
        format!(
            "Failed to create SQLite migration backup directory {}: {error}",
            backup_dir.display()
        )
    })?;

    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let backup_path = backup_dir.join(format!(
        "ai-toolbox-schema-v{}-to-v{}-{}-{}.db",
        current_version,
        migrations::TARGET_SCHEMA_VERSION,
        timestamp,
        Uuid::new_v4().simple()
    ));

    backup::backup_to_path(conn, &backup_path).map_err(|error| {
        format!(
            "Failed to create pre-migration SQLite backup before upgrading schema v{} to v{}: {}",
            current_version,
            migrations::TARGET_SCHEMA_VERSION,
            error
        )
    })?;
    log::info!(
        "Created pre-migration SQLite backup before schema v{} -> v{}: {}",
        current_version,
        migrations::TARGET_SCHEMA_VERSION,
        backup_path.display()
    );

    if let Err(error) = prune_migration_backups(&backup_dir, MIGRATION_BACKUP_KEEP) {
        // Retention is housekeeping. A failed prune must never turn a
        // successful upgrade into a failed startup.
        log::warn!("Failed to prune old SQLite migration backups: {error}");
    }

    Ok(Some(backup_path))
}

/// Keep the newest `keep` pre-migration snapshots, delete the rest.
///
/// Ordering is by write time rather than by the version numbers in the name:
/// the name also carries a UUID, and a repaired or re-run migration can produce
/// an older-looking name last. mtime is what actually records arrival order.
///
/// Each snapshot is deleted together with its `-wal` / `-shm` sidecars. Those
/// only exist when something opened the file afterwards (our own read-only
/// inspection does), but a lone sidecar left behind would be a stray file that
/// no future scan accounts for.
fn prune_migration_backups(backup_dir: &Path, keep: usize) -> Result<usize, String> {
    let mut snapshots: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let entries = fs::read_dir(backup_dir).map_err(|error| {
        format!(
            "Failed to list migration backup directory {}: {error}",
            backup_dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("Failed to read entry in {}: {error}", backup_dir.display())
        })?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        // Only `<prefix>*.db`; the sidecars travel with their main file.
        if !name.starts_with(MIGRATION_BACKUP_PREFIX) || !name.ends_with(".db") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        snapshots.push((modified, path));
    }

    if snapshots.len() <= keep {
        return Ok(0);
    }
    snapshots.sort_by(|left, right| right.0.cmp(&left.0));

    let mut removed = 0;
    for (_, path) in snapshots.into_iter().skip(keep) {
        for suffix in ["", "-wal", "-shm"] {
            let target = PathBuf::from(format!("{}{suffix}", path.display()));
            match fs::remove_file(&target) {
                Ok(()) => {
                    if suffix.is_empty() {
                        removed += 1;
                    }
                }
                // A missing sidecar is the normal case, not a failure.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    log::warn!(
                        "Failed to remove old migration backup {}: {error}",
                        target.display()
                    );
                }
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(dir: &Path, name: &str, age_secs: u64) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"x").unwrap();
        let stamp = SystemTime::now() - Duration::from_secs(age_secs);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        path
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| {
                entry
                    .ok()?
                    .file_name()
                    .to_str()
                    .map(str::to_string)
            })
            .collect();
        out.sort();
        out
    }

    /// Above the limit: the oldest go, the newest `keep` stay.
    #[test]
    fn keeps_the_newest_snapshots_and_deletes_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        for (index, age) in [(1, 500), (2, 400), (3, 300), (4, 200), (5, 100)] {
            touch(
                dir,
                &format!("{MIGRATION_BACKUP_PREFIX}v{index}-to-v{}-stamp.db", index + 1),
                age,
            );
        }

        let removed = prune_migration_backups(dir, 3).unwrap();

        assert_eq!(removed, 2);
        let left = names(dir);
        assert_eq!(left.len(), 3, "three should remain: {left:?}");
        assert!(left.iter().all(|name| !name.contains("-to-v2-")));
        assert!(left.iter().all(|name| !name.contains("-to-v3-")));
    }

    /// The rebuild snapshots share the directory but not the lifecycle. They are
    /// the only copy of what a destructive import replaced, so a run of upgrades
    /// must not count them towards the three and sweep them away.
    #[test]
    fn rebuild_snapshots_are_never_counted_or_removed() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        for index in 1..=5 {
            touch(
                dir,
                &format!("{MIGRATION_BACKUP_PREFIX}v{index}-to-v{}.db", index + 1),
                (6 - index) as u64 * 100,
            );
        }
        let rebuild_old = touch(dir, "ai-toolbox-session-usage-rebuild-20260118-081336.db", 9_000);
        let rebuild_new = touch(dir, "ai-toolbox-session-usage-rebuild-20261018-094503.db", 50);

        prune_migration_backups(dir, 3).unwrap();

        let left = names(dir);
        assert!(
            left.contains(&"ai-toolbox-session-usage-rebuild-20260118-081336.db".to_string()),
            "an old rebuild snapshot must survive: {left:?}"
        );
        assert!(
            left.contains(&"ai-toolbox-session-usage-rebuild-20261018-094503.db".to_string()),
            "a new rebuild snapshot must survive: {left:?}"
        );
        assert!(
            rebuild_old.exists() && rebuild_new.exists(),
            "both rebuild snapshots should still exist"
        );
    }

    /// A snapshot deleted on its own would leave the sidecars behind as stray
    /// files nothing ever scans again.
    #[test]
    fn deleting_a_snapshot_takes_its_wal_and_shm_sidecars_with_it() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        let stale = touch(dir, &format!("{MIGRATION_BACKUP_PREFIX}v1-to-v2.db"), 500);
        touch(dir, &format!("{MIGRATION_BACKUP_PREFIX}v2-to-v3.db"), 400);
        touch(dir, &format!("{MIGRATION_BACKUP_PREFIX}v3-to-v4.db"), 300);
        fs::write(format!("{}-wal", stale.display()), b"").unwrap();
        fs::write(format!("{}-shm", stale.display()), b"").unwrap();

        prune_migration_backups(dir, 2).unwrap();

        let left = names(dir);
        assert!(
            !left.iter().any(|name| name.starts_with("ai-toolbox-schema-v1-to-v2")),
            "the snapshot and both sidecars should be gone: {left:?}"
        );
    }

    /// Re-running on an already-trimmed directory is a no-op, not an error.
    #[test]
    fn pruning_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        for index in 1..=4 {
            touch(dir, &format!("{MIGRATION_BACKUP_PREFIX}v{index}-to-v{}.db", index + 1), 10);
        }
        prune_migration_backups(dir, 3).unwrap();
        assert_eq!(prune_migration_backups(dir, 3).unwrap(), 0);
        assert_eq!(names(dir).len(), 3);
    }
}
