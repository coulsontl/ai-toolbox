//! Startup recovery state and commands.
//!
//! When the SQLite database schema is newer than the current
//! `TARGET_SCHEMA_VERSION`, the app cannot open the database. Instead of
//! crashing or opening a browser, the startup path manages a
//! [`StartupRecovery`] state containing the user-facing error message, shows
//! the main window, and returns early from `setup`. The frontend then queries
//! [`get_startup_recovery`] and, if set, renders a lightweight recovery
//! screen that auto-upgrades via `tauri-plugin-updater` (no DB access).
//!
//! All commands here are DB-free so they work in the recovery path.
//!
//! # Cross-boundary contract
//!
//! - `StartupRecovery` is **always** managed (with `None`) before
//!   `SqliteDbState::open`, so `get_startup_recovery` resolves on every boot.
//! - The frontend (`web/app/App.tsx`) calls `get_startup_recovery` **before**
//!   mounting the normal `<Providers>` tree, so none of the DB-dependent init
//!   effects (`initApp`/`initSettings`/`loadCached*`/`fetchRemote*`) run when
//!   in recovery mode.
//! - `check_for_updates`/`install_update` were refactored to take `AppHandle`
//!   and use `app.try_state::<SqliteDbState>()` (optional). In recovery mode
//!   they fall back to `http_client::create_client_with_env_proxy` / env-proxy
//!   and skip DB-backed proxy configuration.


use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Managed state carrying the recovery error message.
///
/// `message` is `Some(msg)` when the app booted into recovery mode (DB schema
/// too new), and `None` during a normal boot. Always managed (with `None`) so
/// [`get_startup_recovery`] resolves in both paths.
#[derive(Default)]
pub struct StartupRecovery {
    pub message: Arc<Mutex<Option<String>>>,
}

impl StartupRecovery {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Returns the recovery error message if the app booted into recovery mode,
/// otherwise `None` (normal boot). The frontend uses this to decide whether
/// to render the recovery update screen or the normal app.
#[tauri::command]
pub fn get_startup_recovery(state: tauri::State<'_, StartupRecovery>) -> Option<String> {
    state.message.lock().ok().and_then(|m| m.clone())
}

/// Exit the application from the recovery screen (e.g. user gives up after a
/// failed auto-update). Mirrors `tray::request_app_exit`: sets the
/// `APP_EXIT_REQUESTED` flag so shutdown hooks treat it as a user-requested
/// exit, then `app.exit(0)`.
#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    crate::APP_EXIT_REQUESTED.store(true, Ordering::SeqCst);
    app.exit(0);
}
