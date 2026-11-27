//! Local Agent Control API.
//!
//! A loopback-only HTTP control plane for coding agents. It reuses the
//! existing Tauri command implementations (and therefore their validation,
//! backups, event broadcasts and tray refreshes) instead of reimplementing
//! per-CLI config writers. See `AGENTS.md` in this directory for the contract.

pub mod approval;
pub mod cli_dispatch;
pub mod commands;
pub mod discovery;
pub mod error;
pub mod redact;
pub mod routes;
pub mod routes_cli;
pub mod routes_gateway;
pub mod routes_plugins;
pub mod server;
pub mod settings;

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tauri::AppHandle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::db::SqliteDbState;

use approval::ApprovalHub;
use settings::ControlApiSettings;

/// Shared data every HTTP request needs. Held in an `Arc` so connection tasks
/// do not borrow Tauri managed state.
pub struct ServerContext {
    pub app: AppHandle,
    pub db: SqliteDbState,
    pub settings: RwLock<ControlApiSettings>,
    pub approval: ApprovalHub,
    pub port: AtomicU16,
    pub started_at: String,
}

impl ServerContext {
    pub fn settings(&self) -> ControlApiSettings {
        self.settings
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn update_settings(&self, settings: ControlApiSettings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
    }

    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }
}

struct ServerRuntime {
    token: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

/// Tauri-managed state. `runtime` owns the listener task and is swapped by the
/// settings toggle without restarting the app.
pub struct ControlApiState {
    pub context: Arc<ServerContext>,
    runtime: Mutex<Option<ServerRuntime>>,
}

impl ControlApiState {
    pub fn new(app: AppHandle, db: SqliteDbState, settings: ControlApiSettings) -> Self {
        let context = Arc::new(ServerContext {
            app,
            db,
            settings: RwLock::new(settings),
            approval: ApprovalHub::new(),
            port: AtomicU16::new(0),
            started_at: chrono::Local::now().to_rfc3339(),
        });
        Self {
            context,
            runtime: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.runtime
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    pub fn token(&self) -> Option<String> {
        self.runtime
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|runtime| runtime.token.clone()))
    }

    pub fn take_runtime(&self) -> Option<(Option<oneshot::Sender<()>>, Option<JoinHandle<()>>)> {
        self.runtime
            .lock()
            .ok()
            .and_then(|mut guard| guard.take().map(|runtime| (runtime.shutdown, runtime.task)))
    }

    pub fn store_runtime(
        &self,
        token: String,
        shutdown: oneshot::Sender<()>,
        task: JoinHandle<()>,
    ) {
        if let Ok(mut guard) = self.runtime.lock() {
            *guard = Some(ServerRuntime {
                token,
                shutdown: Some(shutdown),
                task: Some(task),
            });
        }
    }
}

/// Starts the listener when enabled. Returns `true` when this call started it.
pub async fn start_if_enabled(state: &ControlApiState) -> Result<bool, String> {
    let settings = state.context.settings();
    if !settings.enabled || state.is_running() {
        return Ok(false);
    }
    server::start(state).await?;
    Ok(true)
}

/// Stops the listener and removes the discovery file.
///
/// Pending approvals are rejected and expired first: their HTTP handlers would
/// otherwise wait for the full timeout against a listener that no longer
/// exists, and the desktop modal would keep offering a dead request.
pub async fn stop(state: &ControlApiState) -> Result<(), String> {
    cancel_pending_approvals(&state.context, "control_api_stopped");
    server::stop(state).await
}

/// Rejects every parked approval and broadcasts the expiry event.
pub fn cancel_pending_approvals(context: &ServerContext, reason: &str) -> usize {
    use tauri::Emitter;
    let cancelled = context.approval.cancel_all();
    let count = cancelled.len();
    for info in cancelled {
        let _ = context.app.emit(
            approval::APPROVAL_EXPIRED_EVENT,
            serde_json::json!({"requestId": info.request_id, "reason": reason}),
        );
    }
    count
}

/// App-exit hook: stop accepting connections and delete our discovery file.
pub fn shutdown_on_exit(app: &AppHandle) {
    use tauri::Manager;
    if let Some(state) = app.try_state::<ControlApiState>() {
        // The runtime is dropped synchronously; the listener sees the closed
        // oneshot and exits. We still remove the file immediately so a restart
        // cannot read a token that no listener serves.
        let path = discovery::default_discovery_path();
        discovery::remove_discovery_file_if_owned(&path, std::process::id());
        if let Some((shutdown, task)) = state.take_runtime() {
            drop(shutdown);
            if let Some(task) = task {
                task.abort();
            }
        }
    }
}

/// Tauri IPC dispatcher for `control_api_*` commands. Kept out of the root
/// `generate_handler!` match for the same Windows debug-stack reason as
/// `plugins::invoke_handler`.
#[inline(never)]
pub fn invoke_handler(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        commands::control_api_get_settings,
        commands::control_api_save_settings,
        commands::control_api_status,
        commands::control_api_resolve_approval,
        commands::control_api_list_pending_approvals,
        commands::control_api_cli_status,
        commands::control_api_install_cli,
        commands::control_api_uninstall_cli,
    ];
    handler(invoke)
}
