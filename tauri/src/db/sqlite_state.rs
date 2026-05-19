use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::{health, migrations};

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
        initialize_connection(&mut conn)?;
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
    migrations::run_all(conn)?;
    health::quick_check(conn)?;
    Ok(())
}
