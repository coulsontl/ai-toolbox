//! The Bun process that runs provider plugins.
//!
//! One long-lived child speaks newline-delimited JSON over stdin/stdout. A
//! dedicated reader decodes every line and hands it to the waiting caller (or
//! to a fetch body's queue) without ever awaiting plugin business; a dedicated
//! writer owns stdin. The process starts lazily: nothing is downloaded or
//! spawned until a provider-capable plugin is actually called.
//!
//! Failure policy: an unexpected exit fails every in-flight request, then
//! blocks new starts for 1/2/4/8/16 seconds. Five consecutive crashes open the
//! circuit until a manual restart.

pub mod assets;
pub mod auth;
pub mod protocol;
pub mod stream;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::coding::cli_resolver::apply_create_no_window_tokio;
use crate::db::SqliteDbState;

use super::bun_runtime::BunRuntime;
use super::installer;
use super::paths::PluginPaths;
use super::store;
use super::types::PluginKind;

use protocol::{
    encode_abort, encode_request, parse_host_line, CapturedWireRequest, FetchHead, HostEvent,
    HostMessage, MAX_WIRE_LINE, STREAM_WINDOW,
};
use stream::{FetchAbort, FetchBodyStream, PluginFetchResponse};

const RPC_TIMEOUT: Duration = Duration::from_secs(120);
const INIT_TIMEOUT: Duration = Duration::from_secs(120);
const FETCH_HEAD_TIMEOUT: Duration = Duration::from_secs(120);
const SIGNIN_METHODS: [&str; 4] = ["authorize", "callback", "apiKey", "importFiles"];
/// Frames a fetch body may queue before the reader applies backpressure.
/// The credit window (512 KiB / 2 KiB overhead) bounds the host to ~256.
const CHUNK_QUEUE: usize = 512;
const BACKOFF_SECONDS: [u64; 5] = [1, 2, 4, 8, 16];
const MAX_CONSECUTIVE_CRASHES: u32 = 5;
const LOG_CAPACITY: usize = 500;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunHostStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub pending_requests: usize,
    pub crashes: u32,
    pub circuit_open: bool,
    pub retry_in_millis: u64,
    pub last_error: Option<String>,
    pub started_at: Option<String>,
    pub log_entries: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogEntry {
    pub seq: u64,
    pub at: String,
    pub source: String,
    pub level: String,
    pub message: String,
}

struct RuntimeLogRing {
    entries: StdMutex<VecDeque<RuntimeLogEntry>>,
    next_seq: AtomicU64,
}

impl RuntimeLogRing {
    fn new() -> Self {
        Self {
            entries: StdMutex::new(VecDeque::with_capacity(LOG_CAPACITY)),
            next_seq: AtomicU64::new(1),
        }
    }

    fn push(&self, source: &str, level: &str, message: &str) {
        let redacted = redact_secrets(message);
        let mut message: String = redacted.chars().take(8_000).collect();
        if message.trim().is_empty() {
            return;
        }
        if redacted.chars().count() > 8_000 {
            message.push_str("…");
        }
        let entry = RuntimeLogEntry {
            seq: self.next_seq.fetch_add(1, Ordering::SeqCst),
            at: chrono::Local::now().to_rfc3339(),
            source: source.to_string(),
            level: level.to_string(),
            message,
        };
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while entries.len() >= LOG_CAPACITY {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    fn snapshot(&self, limit: usize) -> Vec<RuntimeLogEntry> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let skip = entries.len().saturating_sub(limit);
        entries.iter().skip(skip).cloned().collect()
    }

    fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// What a fetch's `Head` frame delivers: the reply head itself plus whatever
/// outbound request the host captured for it.
struct FetchHeadReply {
    head: FetchHead,
    wire_request: Option<CapturedWireRequest>,
}

enum Pending {
    Call {
        method: String,
        tx: oneshot::Sender<Result<Value, String>>,
    },
    Head {
        method: String,
        tx: oneshot::Sender<Result<FetchHeadReply, String>>,
        chunk_tx: mpsc::Sender<Result<Bytes, String>>,
        finished: Arc<AtomicBool>,
        /// Arrives before the head when the caller asked for a capture.
        wire_request: Option<CapturedWireRequest>,
    },
    Stream {
        method: String,
        chunk_tx: mpsc::Sender<Result<Bytes, String>>,
        finished: Arc<AtomicBool>,
    },
}

impl Pending {
    fn method(&self) -> &str {
        match self {
            Self::Call { method, .. } | Self::Head { method, .. } | Self::Stream { method, .. } => {
                method
            }
        }
    }

    fn fail(self, message: &str) {
        match self {
            Self::Call { tx, .. } => {
                let _ = tx.send(Err(message.to_string()));
            }
            Self::Head { tx, .. } => {
                let _ = tx.send(Err(message.to_string()));
            }
            Self::Stream {
                chunk_tx, finished, ..
            } => {
                finished.store(true, Ordering::SeqCst);
                let _ = chunk_tx.try_send(Err(message.to_string()));
            }
        }
    }
}

struct HostProcess {
    child: Child,
    pid: u32,
    stdin_tx: mpsc::UnboundedSender<String>,
    generation: u64,
    alive: Arc<AtomicBool>,
    // The process is registered before its readers start, so a fast exit is
    // still attributed to this generation. Handles are filled in right after
    // the tasks are spawned.
    stdin_task: Option<tokio::task::JoinHandle<()>>,
    stdout_task: Option<tokio::task::JoinHandle<()>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for HostProcess {
    fn drop(&mut self) {
        for task in [
            self.stdin_task.take(),
            self.stdout_task.take(),
            self.stderr_task.take(),
        ]
        .into_iter()
        .flatten()
        {
            task.abort();
        }
        // `kill_on_drop` on the child takes care of the process itself.
    }
}

struct HostInner {
    process: Option<HostProcess>,
    generation: u64,
    crashes: u32,
    circuit_open: bool,
    backoff_until: Option<Instant>,
    last_error: Option<String>,
    started_at: Option<String>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
}

impl HostInner {
    fn new() -> Self {
        Self {
            process: None,
            generation: 0,
            crashes: 0,
            circuit_open: false,
            backoff_until: None,
            last_error: None,
            started_at: None,
            next_id: 1,
            pending: HashMap::new(),
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn stdin_sender(&self) -> Result<mpsc::UnboundedSender<String>, String> {
        self.process
            .as_ref()
            .filter(|process| process.alive.load(Ordering::SeqCst))
            .map(|process| process.stdin_tx.clone())
            .ok_or_else(|| "The plugin host is not running".to_string())
    }
}

/// The plugin provider host.
pub struct BunHost {
    db: SqliteDbState,
    paths: PluginPaths,
    bun: Arc<BunRuntime>,
    inner: Arc<Mutex<HostInner>>,
    start_lock: Mutex<()>,
    logs: Arc<RuntimeLogRing>,
}

impl BunHost {
    pub fn new(db: SqliteDbState, paths: PluginPaths, bun: Arc<BunRuntime>) -> Self {
        Self {
            db,
            paths,
            bun,
            inner: Arc::new(Mutex::new(HostInner::new())),
            start_lock: Mutex::new(()),
            logs: Arc::new(RuntimeLogRing::new()),
        }
    }

    // ---- lifecycle ---------------------------------------------------------

    pub async fn ensure_running(&self) -> Result<(), String> {
        let _guard = self.start_lock.lock().await;
        self.start_locked().await
    }

    async fn start_locked(&self) -> Result<(), String> {
        {
            let mut inner = self.inner.lock().await;
            if let Some(process) = inner.process.as_mut() {
                let exited = process.child.try_wait().ok().flatten();
                if exited.is_none() && process.alive.load(Ordering::SeqCst) {
                    return Ok(());
                }
            }
            if inner.circuit_open {
                return Err(format!(
                    "The plugin host crashed {MAX_CONSECUTIVE_CRASHES} times in a row; restart it from the plugins settings"
                ));
            }
            if let Some(until) = inner.backoff_until {
                let now = Instant::now();
                if now < until {
                    let remaining = (until - now).as_millis().max(1);
                    return Err(format!(
                        "The plugin host is restarting; retry in {remaining} ms"
                    ));
                }
            }
            // Any process still here is dead or being replaced; bump the
            // generation so its reader cannot report a second crash.
            inner.generation = inner.generation.wrapping_add(1);
            inner.process = None;
            inner.backoff_until = None;
        }

        let bun = self.bun.ensure().await?;
        self.paths.ensure_dirs()?;
        let runtime_files = assets::materialize(&self.paths.runtime_host_dir())?;
        let host_paths = assets::RuntimeHostPaths::new(&self.paths.runtime_host_dir());
        log::info!(
            "starting plugin host with Bun {} ({} files)",
            bun.version,
            runtime_files.files.len()
        );

        let items = self.host_plugin_items()?;
        let init_params = self.init_params(&host_paths.pi, items);

        let mut command = tokio::process::Command::new(&bun.path);
        command
            .arg("run")
            .arg("--preload")
            .arg(&host_paths.opens)
            .arg(&host_paths.host)
            .current_dir(self.paths.packages_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_remove("BUN_OPTIONS")
            .env_remove("NODE_OPTIONS")
            .env("DO_NOT_TRACK", "1")
            // Where a plugin's host code may keep persistent data.
            .env("AI_TOOLBOX_PLUGINS_DATA_DIR", self.paths.data_dir());
        apply_create_no_window_tokio(&mut command);

        let mut child = command.spawn().map_err(|error| {
            format!(
                "Failed to start the plugin host with '{}': {error}",
                bun.path
            )
        })?;
        let pid = child.id().unwrap_or(0);
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Plugin host stdin was not captured".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Plugin host stdout was not captured".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Plugin host stderr was not captured".to_string())?;

        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel();
        let alive = Arc::new(AtomicBool::new(true));
        let generation = {
            let inner = self.inner.lock().await;
            inner.generation
        };

        {
            let mut inner = self.inner.lock().await;
            inner.process = Some(HostProcess {
                child,
                pid,
                stdin_tx,
                generation,
                alive: alive.clone(),
                stdin_task: None,
                stdout_task: None,
                stderr_task: None,
            });
            inner.started_at = Some(chrono::Local::now().to_rfc3339());
            inner.last_error = None;
        }

        let stdin_task = tokio::spawn(write_stdin(
            stdin,
            stdin_rx,
            self.inner.clone(),
            self.logs.clone(),
            generation,
        ));
        let stdout_task = tokio::spawn(read_stdout(
            stdout,
            self.inner.clone(),
            self.logs.clone(),
            generation,
        ));
        let stderr_task = tokio::spawn(read_stderr(stderr, self.logs.clone()));
        {
            let mut inner = self.inner.lock().await;
            if let Some(process) = inner.process.as_mut() {
                if process.generation == generation {
                    process.stdin_task = Some(stdin_task);
                    process.stdout_task = Some(stdout_task);
                    process.stderr_task = Some(stderr_task);
                } else {
                    stdin_task.abort();
                    stdout_task.abort();
                    stderr_task.abort();
                }
            } else {
                stdin_task.abort();
                stdout_task.abort();
                stderr_task.abort();
            }
        }

        if !alive.load(Ordering::SeqCst) {
            // The reader already recorded the exit and its backoff.
            return Err("The plugin host exited immediately after starting".to_string());
        }

        match self.send_call("init", init_params, INIT_TIMEOUT).await {
            Ok(result) => {
                if let Some(plugins) = result.get("plugins").and_then(Value::as_array) {
                    for plugin in plugins {
                        if let Some(error) = plugin.get("error").and_then(Value::as_str) {
                            let spec = plugin
                                .get("spec")
                                .and_then(Value::as_str)
                                .unwrap_or("plugin");
                            self.logs
                                .push("event", "error", &format!("{spec}: {error}"));
                        }
                    }
                }
                let mut inner = self.inner.lock().await;
                inner.crashes = 0;
                Ok(())
            }
            Err(error) => {
                self.stop_with_error(&error).await;
                Err(error)
            }
        }
    }

    fn host_plugin_items(&self) -> Result<Vec<Value>, String> {
        let records = store::list_installed_plugins(&self.db)?;
        let mut items = Vec::new();
        for record in records {
            if !record.enabled || !record.has_provider_entry {
                continue;
            }
            // A page-only package has no host part; starting Bun for it would
            // load nothing and burn a runtime for a static folder.
            if record.is_page_only() {
                continue;
            }
            if !matches!(record.kind, PluginKind::Provider | PluginKind::Both) {
                continue;
            }
            let target = installer::plugin_target_path(&self.paths, &record);
            if !target.exists() {
                self.logs.push(
                    "event",
                    "warn",
                    &format!("{} is installed but its files are missing", record.spec),
                );
                continue;
            }
            items.push(json!({
                "spec": record.spec,
                "target": target.to_string_lossy(),
                "options": record.options,
            }));
        }
        Ok(items)
    }

    fn init_params(&self, pi_path: &PathBuf, items: Vec<Value>) -> Value {
        let options = provider_options_map(&self.db);
        json!({
            "authPath": self.paths.auth_path().to_string_lossy(),
            "modelsDevPath": "",
            "piPath": pi_path.to_string_lossy(),
            "directory": self.paths.packages_dir().to_string_lossy(),
            "config": {},
            "prefer": {},
            "options": options,
            "plugins": items,
            "loadMillis": 15_000,
            "listMillis": 15_000,
            "heavy": {
                "mb": 2048,
                "afterMillis": 60_000,
                "everyMillis": 60_000,
            },
        })
    }

    async fn stop_with_error(&self, error: &str) {
        let mut inner = self.inner.lock().await;
        inner.generation = inner.generation.wrapping_add(1);
        if let Some(process) = inner.process.take() {
            process.alive.store(false, Ordering::SeqCst);
        }
        inner.last_error = Some(error.to_string());
        // A start that fails during `init` is as unusable as a crash; count
        // it so an always-failing plugin set cannot retry forever.
        inner.crashes = inner.crashes.saturating_add(1);
        match next_backoff(inner.crashes) {
            Some(backoff) => inner.backoff_until = Some(Instant::now() + backoff),
            None => inner.circuit_open = true,
        }
        for (_, pending) in inner.pending.drain() {
            pending.fail(error);
        }
    }

    /// Kill the host and start it again. Used after plugin changes and by the
    /// settings pane.
    pub async fn restart(&self) -> Result<(), String> {
        let _guard = self.start_lock.lock().await;
        {
            let mut inner = self.inner.lock().await;
            inner.generation = inner.generation.wrapping_add(1);
            if let Some(process) = inner.process.take() {
                process.alive.store(false, Ordering::SeqCst);
            }
            for (_, pending) in inner.pending.drain() {
                pending.fail("The plugin host was restarted");
            }
            inner.crashes = 0;
            inner.circuit_open = false;
            inner.backoff_until = None;
            inner.last_error = None;
            inner.started_at = None;
        }
        self.start_locked().await
    }

    /// Restart only when the host is running: installing a plugin must not
    /// pay for a Bun download or start a host nobody asked for.
    pub async fn notify_plugins_changed(&self) -> Result<(), String> {
        let running = {
            let inner = self.inner.lock().await;
            inner
                .process
                .as_ref()
                .is_some_and(|process| process.alive.load(Ordering::SeqCst))
        };
        if running {
            self.restart().await
        } else {
            Ok(())
        }
    }

    pub async fn status(&self) -> BunHostStatus {
        let mut inner = self.inner.lock().await;
        let mut running = false;
        let mut pid = None;
        if let Some(process) = inner.process.as_mut() {
            let exited = process.child.try_wait().ok().flatten();
            if exited.is_none() && process.alive.load(Ordering::SeqCst) {
                running = true;
                pid = Some(process.pid);
            }
        }
        let retry_in_millis = inner
            .backoff_until
            .map(|until| until.saturating_duration_since(Instant::now()).as_millis() as u64)
            .unwrap_or(0);
        BunHostStatus {
            running,
            pid,
            pending_requests: inner.pending.len(),
            crashes: inner.crashes,
            circuit_open: inner.circuit_open,
            retry_in_millis,
            last_error: inner.last_error.clone(),
            started_at: inner.started_at.clone(),
            log_entries: self.logs.len(),
        }
    }

    // ---- RPC ---------------------------------------------------------------

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.ensure_running().await?;
        self.send_call(method, params, RPC_TIMEOUT).await
    }

    async fn send_call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let (id, receiver, stdin_tx) = {
            let mut inner = self.inner.lock().await;
            let stdin_tx = inner.stdin_sender()?;
            let id = inner.allocate_id();
            let (tx, rx) = oneshot::channel();
            inner.pending.insert(
                id,
                Pending::Call {
                    method: method.to_string(),
                    tx,
                },
            );
            (id, rx, stdin_tx)
        };

        if stdin_tx.send(encode_request(id, method, &params)).is_err() {
            self.inner.lock().await.pending.remove(&id);
            return Err("The plugin host stopped before the request was sent".to_string());
        }

        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(format!("The plugin host dropped the '{method}' request")),
            Err(_) => {
                self.inner.lock().await.pending.remove(&id);
                let _ = stdin_tx.send(encode_abort(id));
                Err(format!(
                    "The plugin host did not answer '{method}' within {} seconds",
                    timeout.as_secs()
                ))
            }
        }
    }

    // ---- streaming fetch ---------------------------------------------------

    pub async fn fetch(&self, params: Value) -> Result<PluginFetchResponse, String> {
        self.ensure_running().await?;

        let (id, head_rx, chunk_rx, chunk_tx, stdin_tx, finished) = {
            let mut inner = self.inner.lock().await;
            let stdin_tx = inner.stdin_sender()?;
            let id = inner.allocate_id();
            let (head_tx, head_rx) = oneshot::channel();
            let (chunk_tx, chunk_rx) = mpsc::channel(CHUNK_QUEUE);
            let finished = Arc::new(AtomicBool::new(false));
            inner.pending.insert(
                id,
                Pending::Head {
                    method: "fetch".to_string(),
                    tx: head_tx,
                    chunk_tx: chunk_tx.clone(),
                    finished: finished.clone(),
                    wire_request: None,
                },
            );
            (id, head_rx, chunk_rx, chunk_tx, stdin_tx, finished)
        };
        drop(chunk_tx);

        let mut params = params;
        if let Some(object) = params.as_object_mut() {
            object.insert("window".to_string(), json!(STREAM_WINDOW));
        }
        if stdin_tx.send(encode_request(id, "fetch", &params)).is_err() {
            self.inner.lock().await.pending.remove(&id);
            return Err("The plugin host stopped before the fetch was sent".to_string());
        }

        let abort_guard =
            FetchAbort::new(id, stdin_tx.clone(), finished.clone(), self.inner.clone());
        let reply = match tokio::time::timeout(FETCH_HEAD_TIMEOUT, head_rx).await {
            Ok(Ok(Ok(reply))) => reply,
            Ok(Ok(Err(error))) => {
                self.inner.lock().await.pending.remove(&id);
                return Err(error);
            }
            Ok(Err(_)) => {
                self.inner.lock().await.pending.remove(&id);
                return Err("The plugin host dropped the fetch".to_string());
            }
            Err(_) => {
                self.inner.lock().await.pending.remove(&id);
                return Err(format!(
                    "The plugin host did not answer the fetch within {} seconds",
                    FETCH_HEAD_TIMEOUT.as_secs()
                ));
            }
        };
        // The body owns the abort from here on.
        abort_guard.disarm();

        let body = Box::pin(FetchBodyStream::new(
            id,
            chunk_rx,
            stdin_tx,
            finished,
            self.inner.clone(),
        ));
        Ok(PluginFetchResponse {
            status: reply.head.status,
            headers: reply.head.headers,
            body,
            wire_request: reply.wire_request,
        })
    }

    // ---- provider-specific helpers ----------------------------------------

    pub async fn providers(&self, params: Value) -> Result<Value, String> {
        self.call("providers", params).await
    }

    pub async fn login_prompt(&self, params: Value) -> Result<Value, String> {
        self.call("prompt", params).await
    }

    pub async fn validate(&self, params: Value) -> Result<Value, String> {
        self.call("validate", params).await
    }

    pub async fn authorize(&self, params: Value) -> Result<Value, String> {
        self.call("authorize", params).await
    }

    pub async fn callback(&self, params: Value) -> Result<Value, String> {
        self.call("callback", params).await
    }

    pub async fn api_key(&self, params: Value) -> Result<Value, String> {
        self.call("apiKey", params).await
    }

    pub async fn import_files(&self, params: Value) -> Result<Value, String> {
        self.call("importFiles", params).await
    }

    pub async fn sign_out(&self, params: Value) -> Result<Value, String> {
        self.call("signOut", params).await
    }

    pub async fn usage(&self, params: Value) -> Result<Value, String> {
        self.call("usage", params).await
    }

    pub async fn checkin(&self, params: Value) -> Result<Value, String> {
        self.call("checkin", params).await
    }

    /// Renew one OAuth account if it is due (`keepSignedIn`). The host itself
    /// honours the plugin's `auth.keepAlive !== false` and does nothing when
    /// the token is not near expiry.
    pub async fn keep_signed_in(&self, params: Value) -> Result<Value, String> {
        self.call("keepSignedIn", params).await
    }

    pub async fn check(&self, params: Value) -> Result<Value, String> {
        self.call("check", params).await
    }

    /// Abort every sign-in step currently waiting. Returns how many were
    /// cancelled.
    pub async fn abort_signin(&self) -> usize {
        let mut inner = self.inner.lock().await;
        let ids: Vec<u64> = inner
            .pending
            .iter()
            .filter(|(_, pending)| SIGNIN_METHODS.contains(&pending.method()))
            .map(|(id, _)| *id)
            .collect();
        if ids.is_empty() {
            return 0;
        }
        let stdin_tx = inner
            .process
            .as_ref()
            .map(|process| process.stdin_tx.clone());
        for id in &ids {
            if let Some(stdin_tx) = &stdin_tx {
                let _ = stdin_tx.send(encode_abort(*id));
            }
            if let Some(pending) = inner.pending.remove(id) {
                pending.fail("The sign-in was cancelled");
            }
        }
        ids.len()
    }

    /// Push the current provider options to a running host without a
    /// restart: the plugin reads them on its next request. A host that is
    /// not running already reads them from the database at `init`.
    pub async fn set_provider_options(&self, options: Value) -> Result<(), String> {
        let running = {
            let inner = self.inner.lock().await;
            inner
                .process
                .as_ref()
                .is_some_and(|process| process.alive.load(Ordering::SeqCst))
        };
        if !running {
            return Ok(());
        }
        self.call("setProviderOptions", json!({ "options": options }))
            .await
            .map(|_| ())
    }

    // ---- logs --------------------------------------------------------------

    pub fn logs(&self, limit: usize) -> Vec<RuntimeLogEntry> {
        self.logs.snapshot(limit.clamp(1, LOG_CAPACITY))
    }

    pub fn clear_logs(&self) {
        self.logs.clear();
    }
}

/// Delay before the next automatic start, or `None` when the consecutive
/// crash count opens the circuit.
fn next_backoff(crashes: u32) -> Option<Duration> {
    if crashes >= MAX_CONSECUTIVE_CRASHES {
        return None;
    }
    let index = (crashes.saturating_sub(1) as usize).min(BACKOFF_SECONDS.len() - 1);
    Some(Duration::from_secs(BACKOFF_SECONDS[index]))
}

fn provider_options_map(db: &SqliteDbState) -> Value {
    let mut map = serde_json::Map::new();
    for option in store::list_provider_options(db).unwrap_or_default() {
        map.insert(option.provider_id, option.options);
    }
    Value::Object(map)
}

// ---- reader / writer -------------------------------------------------------

async fn write_stdin(
    mut stdin: ChildStdin,
    mut receiver: mpsc::UnboundedReceiver<String>,
    state: Arc<Mutex<HostInner>>,
    logs: Arc<RuntimeLogRing>,
    generation: u64,
) {
    while let Some(line) = receiver.recv().await {
        if stdin.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        if stdin.flush().await.is_err() {
            break;
        }
    }
    on_process_gone(&state, &logs, generation, "stdin closed").await;
}

async fn read_stdout(
    stdout: ChildStdout,
    state: Arc<Mutex<HostInner>>,
    logs: Arc<RuntimeLogRing>,
    generation: u64,
) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        match read_bounded_line(&mut reader, &mut line).await {
            Ok(Some(())) => {
                let text = String::from_utf8_lossy(&line);
                handle_host_line(&text, &state, &logs).await;
                line.clear();
            }
            Ok(None) => break,
            Err(error) => {
                logs.push("stdout", "error", &error);
                break;
            }
        }
    }
    on_process_gone(&state, &logs, generation, "stdout closed").await;
}

async fn read_stderr(stderr: tokio::process::ChildStderr, logs: Arc<RuntimeLogRing>) {
    let mut reader = BufReader::new(stderr);
    let mut line = Vec::new();
    loop {
        match read_bounded_line(&mut reader, &mut line).await {
            Ok(Some(())) => {
                let text = String::from_utf8_lossy(&line);
                let text = text.trim_end();
                if !text.is_empty() {
                    logs.push("stderr", "info", text);
                    log::debug!("plugin host stderr: {}", redact_secrets(text));
                }
                line.clear();
            }
            Ok(None) => break,
            Err(error) => {
                logs.push("stderr", "error", &error);
                break;
            }
        }
    }
    // A closed stderr alone is not a crash; the stdout reader decides.
}

/// Read one newline-terminated line without letting a host that never writes
/// a newline grow the buffer without bound.
async fn read_bounded_line(
    reader: &mut BufReader<impl tokio::io::AsyncRead + Unpin>,
    line: &mut Vec<u8>,
) -> Result<Option<()>, String> {
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|error| format!("Failed to read plugin host output: {error}"))?;
        if available.is_empty() {
            return Ok(None);
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                line.extend_from_slice(&available[..=index]);
                reader.consume(index + 1);
                if line.len() > MAX_WIRE_LINE {
                    return Err(format!(
                        "Plugin host wrote a line longer than {MAX_WIRE_LINE} bytes"
                    ));
                }
                return Ok(Some(()));
            }
            None => {
                line.extend_from_slice(available);
                let consumed = available.len();
                reader.consume(consumed);
                if line.len() > MAX_WIRE_LINE {
                    return Err(format!(
                        "Plugin host wrote a line longer than {MAX_WIRE_LINE} bytes"
                    ));
                }
            }
        }
    }
}

async fn handle_host_line(line: &str, state: &Arc<Mutex<HostInner>>, logs: &Arc<RuntimeLogRing>) {
    match parse_host_line(line) {
        HostMessage::Response { id, result } => {
            let pending = state.lock().await.pending.remove(&id);
            match pending {
                Some(Pending::Call { tx, .. }) => {
                    let _ = tx.send(Ok(result));
                }
                Some(Pending::Stream {
                    chunk_tx, finished, ..
                }) => {
                    finished.store(true, Ordering::SeqCst);
                    drop(chunk_tx);
                }
                Some(Pending::Head { tx, .. }) => {
                    let _ = tx.send(Err(
                        "The plugin host finished a fetch without sending a head".to_string(),
                    ));
                }
                None => {}
            }
        }
        HostMessage::Failure { id, message } => {
            let pending = state.lock().await.pending.remove(&id);
            if let Some(pending) = pending {
                pending.fail(&message);
            }
        }
        HostMessage::WireRequest { id, request } => {
            let mut inner = state.lock().await;
            // The frame is only useful between the fetch request and its
            // head; anything else (an aborted request, a late frame) is
            // dropped rather than failing the fetch.
            if let Some(Pending::Head { wire_request, .. }) = inner.pending.get_mut(&id) {
                *wire_request = Some(request);
            }
        }
        HostMessage::Head { id, head } => {
            let mut inner = state.lock().await;
            let pending = inner.pending.remove(&id);
            match pending {
                Some(Pending::Head {
                    method,
                    tx,
                    chunk_tx,
                    finished,
                    wire_request,
                }) => {
                    inner.pending.insert(
                        id,
                        Pending::Stream {
                            method,
                            chunk_tx,
                            finished,
                        },
                    );
                    drop(inner);
                    let _ = tx.send(Ok(FetchHeadReply { head, wire_request }));
                }
                Some(other) => {
                    drop(inner);
                    other.fail("The plugin host sent a head for a non-fetch request");
                }
                None => {}
            }
        }
        HostMessage::Chunk { id, data } => {
            let mut inner = state.lock().await;
            if let Some(Pending::Stream { chunk_tx, .. }) = inner.pending.get(&id) {
                if chunk_tx.try_send(Ok(Bytes::from(data))).is_err() {
                    // Should be impossible: the credit window bounds the host
                    // to far fewer frames than the queue holds. Failing the
                    // stream loudly is better than silently truncating it.
                    if let Some(pending) = inner.pending.remove(&id) {
                        pending.fail("The fetch body queue overflowed");
                    }
                }
            }
        }
        HostMessage::Event(event) => handle_host_event(event, logs),
        HostMessage::Invalid(message) => {
            logs.push("protocol", "warn", &message);
        }
    }
}

fn handle_host_event(event: HostEvent, logs: &Arc<RuntimeLogRing>) {
    match event {
        HostEvent::Log { level, message } => logs.push("event", &level, &message),
        HostEvent::Toast { level, message } => {
            logs.push("toast", &level, &format!("toast: {message}"))
        }
        HostEvent::Auth { provider, account } => logs.push(
            "event",
            "info",
            &format!("auth changed: {provider} {account}"),
        ),
        HostEvent::SignIn {
            provider,
            account,
            said,
        } => logs.push(
            "event",
            "warn",
            &format!("sign-in {said}: {provider} {account}"),
        ),
        HostEvent::Heavy { count } => {
            logs.push("event", "warn", &format!("plugin host holds {count} MiB"))
        }
        HostEvent::Other { kind } => logs.push("event", "debug", &format!("event: {kind}")),
    }
}

async fn on_process_gone(
    state: &Arc<Mutex<HostInner>>,
    logs: &Arc<RuntimeLogRing>,
    generation: u64,
    reason: &str,
) {
    let mut inner = state.lock().await;
    let was_alive = match inner.process.as_ref() {
        Some(process) if process.generation == generation => {
            process.alive.swap(false, Ordering::SeqCst)
        }
        _ => return,
    };
    // Both the stdout reader and the stdin writer may report the same exit;
    // only the first report counts as a crash.
    if !was_alive || inner.generation != generation {
        return;
    }
    let exit = inner
        .process
        .as_mut()
        .and_then(|process| process.child.try_wait().ok().flatten());
    inner.crashes = inner.crashes.saturating_add(1);
    let message = match exit {
        Some(status) => format!("plugin host exited ({status})"),
        None => format!("plugin host stopped: {reason}"),
    };
    logs.push("process", "error", &message);
    inner.last_error = Some(message.clone());
    match next_backoff(inner.crashes) {
        Some(backoff) => inner.backoff_until = Some(Instant::now() + backoff),
        None => {
            inner.circuit_open = true;
            logs.push(
                "process",
                "error",
                &format!(
                    "plugin host crashed {MAX_CONSECUTIVE_CRASHES} times in a row; stopping retries"
                ),
            );
        }
    }
    for (_, pending) in inner.pending.drain() {
        pending.fail(&message);
    }
}

// ---- redaction -------------------------------------------------------------

struct RedactionRule {
    pattern: Regex,
    replacement: &'static str,
}

fn redaction_rules() -> &'static [RedactionRule] {
    static RULES: OnceLock<Vec<RedactionRule>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (
                r#"(?i)(authorization\s*:\s*bearer\s+)[^\s"',}]+"#,
                "$1***",
            ),
            (
                r#"(?i)("(?:access|refresh|api[_-]?key|secret|password|token|key)"\s*:\s*")[^"]*(")"#,
                "$1***$2",
            ),
            (
                r"(?i)\b(sk-|ghp_|github_pat_|xox[abp]-)[A-Za-z0-9_-]{8,}",
                "$1***",
            ),
            (
                r#"(?i)\b(api[_-]?key|access[_-]?token|refresh[_-]?token|secret|password)\s*[:=]\s*[^\s,;"'}]+"#,
                "$1=***",
            ),
        ]
        .into_iter()
        .filter_map(|(pattern, replacement)| {
            Regex::new(pattern)
                .ok()
                .map(|pattern| RedactionRule {
                    pattern,
                    replacement,
                })
        })
        .collect()
    })
}

pub fn redact_secrets(text: &str) -> String {
    let mut redacted = text.to_string();
    for rule in redaction_rules() {
        redacted = rule
            .pattern
            .replace_all(&redacted, rule.replacement)
            .into_owned();
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() -> (tempfile::TempDir, BunHost) {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let paths = PluginPaths::new(root.path());
        let bun = Arc::new(BunRuntime::new(db.clone(), paths.clone()));
        (root, BunHost::new(db, paths, bun))
    }

    #[test]
    fn redacts_common_credential_shapes() {
        let text = concat!(
            "authorization: Bearer abcdefghijklmnop\n",
            "{\"access\":\"tok_123456\",\"refresh\":\"ref_98765\"}\n",
            "using sk-1234567890abcdef and ghp_abcdefghijklmnop\n",
            "api_key=super-secret-value\n"
        );
        let redacted = redact_secrets(text);
        for secret in [
            "abcdefghijklmnop",
            "tok_123456",
            "ref_98765",
            "sk-1234567890abcdef",
            "ghp_abcdefghijklmnop",
            "super-secret-value",
        ] {
            assert!(!redacted.contains(secret), "{secret} leaked in {redacted}");
        }
        assert!(redacted.contains("***"));
    }

    #[test]
    fn log_ring_keeps_the_latest_entries_and_caps_itself() {
        let ring = RuntimeLogRing::new();
        for index in 0..(LOG_CAPACITY + 25) {
            ring.push("test", "info", &format!("line {index}"));
        }
        assert_eq!(ring.len(), LOG_CAPACITY);
        let entries = ring.snapshot(3);
        assert_eq!(entries.len(), 3);
        assert!(entries[2].message.ends_with("line 524"));
        ring.clear();
        assert_eq!(ring.len(), 0);
    }

    #[tokio::test]
    async fn a_never_started_host_reports_its_state_and_fails_calls_fast() {
        let (_root, host) = test_host();
        let status = host.status().await;
        assert!(!status.running);
        assert_eq!(status.pending_requests, 0);
        assert_eq!(host.logs(10).len(), 0);
        host.clear_logs();
    }

    #[tokio::test]
    async fn abort_signin_with_nothing_pending_is_a_no_op() {
        let (_root, host) = test_host();
        assert_eq!(host.abort_signin().await, 0);
    }

    #[test]
    fn backoff_grows_then_opens_the_circuit() {
        assert_eq!(next_backoff(1), Some(Duration::from_secs(1)));
        assert_eq!(next_backoff(2), Some(Duration::from_secs(2)));
        assert_eq!(next_backoff(3), Some(Duration::from_secs(4)));
        assert_eq!(next_backoff(4), Some(Duration::from_secs(8)));
        assert_eq!(next_backoff(5), None);
        assert_eq!(next_backoff(9), None);
    }

    async fn stream_pending(
        state: &Arc<Mutex<HostInner>>,
        id: u64,
    ) -> (mpsc::Receiver<Result<Bytes, String>>, Arc<AtomicBool>) {
        let (chunk_tx, chunk_rx) = mpsc::channel(4);
        let finished = Arc::new(AtomicBool::new(false));
        state.lock().await.pending.insert(
            id,
            Pending::Stream {
                method: "fetch".to_string(),
                chunk_tx,
                finished: finished.clone(),
            },
        );
        (chunk_rx, finished)
    }

    #[tokio::test]
    async fn host_lines_route_a_streamed_reply() {
        let state = Arc::new(Mutex::new(HostInner::new()));
        let logs = Arc::new(RuntimeLogRing::new());
        let (head_tx, head_rx) = oneshot::channel();
        let (chunk_tx, mut chunk_rx) = mpsc::channel(4);
        let finished = Arc::new(AtomicBool::new(false));
        state.lock().await.pending.insert(
            9,
            Pending::Head {
                method: "fetch".to_string(),
                tx: head_tx,
                chunk_tx,
                finished: finished.clone(),
                wire_request: None,
            },
        );

        handle_host_line(
            r#"{"id":9,"event":"head","status":200,"headers":{"content-type":"text/event-stream"}}"#,
            &state,
            &logs,
        )
        .await;
        let reply = head_rx.await.unwrap().unwrap();
        assert_eq!(reply.head.status, 200);
        assert_eq!(reply.head.headers[0].0, "content-type");
        assert!(reply.wire_request.is_none());

        handle_host_line(
            r#"{"id":9,"event":"chunk","data":"aGVsbG8="}"#,
            &state,
            &logs,
        )
        .await;
        let chunk = chunk_rx.recv().await.unwrap().unwrap();
        assert_eq!(&chunk[..], b"hello");

        handle_host_line(r#"{"id":9,"result":null}"#, &state, &logs).await;
        assert!(chunk_rx.recv().await.is_none());
        assert!(finished.load(Ordering::SeqCst));
        assert!(state.lock().await.pending.is_empty());
    }

    #[tokio::test]
    async fn a_captured_wire_request_rides_along_with_the_head() {
        let state = Arc::new(Mutex::new(HostInner::new()));
        let logs = Arc::new(RuntimeLogRing::new());
        let (head_tx, head_rx) = oneshot::channel();
        let (chunk_tx, _chunk_rx) = mpsc::channel(4);
        let finished = Arc::new(AtomicBool::new(false));
        state.lock().await.pending.insert(
            12,
            Pending::Head {
                method: "fetch".to_string(),
                tx: head_tx,
                chunk_tx,
                finished,
                wire_request: None,
            },
        );

        handle_host_line(
            r#"{"id":12,"event":"wire_request","url":"https://api.example.com/v1/messages","method":"POST","headers":{"content-type":"application/json"},"body":"aGVsbG8=","bodyBytes":5,"truncated":false}"#,
            &state,
            &logs,
        )
        .await;
        handle_host_line(
            r#"{"id":12,"event":"head","status":200,"headers":{}}"#,
            &state,
            &logs,
        )
        .await;

        let reply = head_rx.await.unwrap().unwrap();
        let wire = reply.wire_request.expect("captured request");
        assert_eq!(wire.url, "https://api.example.com/v1/messages");
        assert_eq!(wire.method, "POST");
        assert_eq!(&wire.body.unwrap()[..], b"hello");
        assert_eq!(wire.body_bytes, 5);
    }

    #[tokio::test]
    async fn a_wire_request_without_a_pending_head_is_ignored() {
        let state = Arc::new(Mutex::new(HostInner::new()));
        let logs = Arc::new(RuntimeLogRing::new());
        handle_host_line(
            r#"{"id":99,"event":"wire_request","url":"https://api.example.com/v1/messages","method":"POST"}"#,
            &state,
            &logs,
        )
        .await;
        assert!(state.lock().await.pending.is_empty());
        assert!(logs.snapshot(10).is_empty());
    }

    #[tokio::test]
    async fn a_host_error_fails_the_fetch_body() {
        let state = Arc::new(Mutex::new(HostInner::new()));
        let logs = Arc::new(RuntimeLogRing::new());
        let (mut chunk_rx, finished) = stream_pending(&state, 11).await;

        handle_host_line(
            r#"{"id":11,"error":{"message":"upstream down"}}"#,
            &state,
            &logs,
        )
        .await;

        let item = chunk_rx.recv().await.unwrap();
        assert_eq!(item.unwrap_err(), "upstream down");
        assert!(chunk_rx.recv().await.is_none());
        assert!(finished.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn an_invalid_line_is_logged_and_skipped() {
        let state = Arc::new(Mutex::new(HostInner::new()));
        let logs = Arc::new(RuntimeLogRing::new());
        handle_host_line("this is not json", &state, &logs).await;
        let entries = logs.snapshot(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, "warn");
        assert!(entries[0].message.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn an_unterminated_line_past_the_wire_limit_is_refused() {
        let data = vec![b'a'; MAX_WIRE_LINE + 16];
        let mut reader = BufReader::new(data.as_slice());
        let mut line = Vec::new();
        let error = read_bounded_line(&mut reader, &mut line).await.unwrap_err();
        assert!(error.contains("longer than"), "{error}");
    }

    #[test]
    fn pending_failures_reach_the_waiting_caller() {
        let (tx, mut rx) = oneshot::channel();
        let pending = Pending::Call {
            method: "providers".to_string(),
            tx,
        };
        pending.fail("boom");
        let result = rx.try_recv().unwrap();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "boom");
    }
}
