//! The in-process gateway middleware engine.
//!
//! Middleware plugins are plain ESM modules exporting `onRequest`,
//! `onProvider`, `onEvent`, `onResponse` and/or `onUsage`. They run inside the
//! gateway process (rquickjs) rather than in the Bun host: a hook on every
//! streamed event must cost microseconds, not a pipe round trip.
//!
//! Every hook invocation gets a fresh runtime. That costs a module compile per
//! call but keeps the sandbox simple and leak-free: a hook that throws, runs
//! out of time or corrupts the module cannot affect the next call. Limits are
//! the Magpie ones: 32 MiB / 512 KiB stack, 250 ms per request-shaped hook,
//! 50 ms per stream event and 1 s to load the module. All failures are
//! fail-open except `ctx.reject`, which turns the request away.
//!
//! Imports are restricted to relative files inside the plugin's own directory.
//! There is no `node_modules` resolution here: a plugin that needs packages
//! must be bundled by its author.

pub mod stats;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::{Duration, Instant};

use rquickjs::function::Opt;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Context, Ctx, Function, Module, Object, Runtime, Value};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use tokio::sync::mpsc;

use crate::db::SqliteDbState;

use super::installer;
use super::paths::PluginPaths;
use super::store;
use super::types::{InstalledPluginRecord, PluginKind};

use stats::{MiddlewareStateView, MiddlewareStats};

const MEMORY_LIMIT: usize = 32 * 1024 * 1024;
const STACK_LIMIT: usize = 512 * 1024;
const REQUEST_LIMIT: Duration = Duration::from_millis(250);
const EVENT_LIMIT: Duration = Duration::from_millis(50);
const USAGE_LIMIT: Duration = Duration::from_millis(250);
const LOAD_LIMIT: Duration = Duration::from_millis(1000);
const USAGE_QUEUE_CAPACITY: usize = 1024;

/// What a request's hooks are told of it, mirroring Magpie's `gwmw.Info`.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestInfo {
    pub protocol: String,
    pub model: String,
    pub stream: bool,
    pub path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<JsonValue>,
}

/// `ctx.state` carried across every hook of one request, per middleware.
///
/// Magpie gives each module a state object that survives `onRequest` →
/// `onProvider` → `onEvent` → `onResponse`; the hooks run in short-lived
/// runtimes here, so the object is serialized between calls.
#[derive(Clone, Debug, Default)]
pub struct MiddlewareState {
    values: Arc<StdMutex<BTreeMap<String, JsonValue>>>,
}

impl MiddlewareState {
    pub fn new() -> Self {
        Self::default()
    }

    fn plugin_state(&self, plugin_id: &str) -> JsonValue {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plugin_id)
            .cloned()
            .unwrap_or_else(|| json!({}))
    }

    fn set_plugin_state(&self, plugin_id: &str, value: JsonValue) {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(plugin_id.to_string(), value);
    }

    fn ctx_json(
        &self,
        info_json: &JsonValue,
        module: &LoadedMiddleware,
        extra: JsonValue,
    ) -> JsonValue {
        build_ctx(
            info_json,
            &module.options,
            self.plugin_state(&module.plugin_id),
            extra,
        )
    }
}

/// The provider a request is about to be sent to (`gwmw.To`).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTarget {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub account: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RequestOutcome {
    Continue(Vec<u8>),
    Reject { status: u16, message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventOutcome {
    Keep,
    Replace(Vec<u8>),
    Drop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hook {
    OnRequest,
    OnProvider,
    OnEvent,
    OnResponse,
    OnUsage,
}

impl Hook {
    fn name(self) -> &'static str {
        match self {
            Self::OnRequest => "onRequest",
            Self::OnProvider => "onProvider",
            Self::OnEvent => "onEvent",
            Self::OnResponse => "onResponse",
            Self::OnUsage => "onUsage",
        }
    }
}

#[derive(Clone, Debug, Default)]
struct HookSet {
    on_request: bool,
    on_provider: bool,
    on_event: bool,
    on_response: bool,
    on_usage: bool,
    /// `None` means every event; `Some([])` means no event.
    events: Option<Vec<String>>,
}

impl HookSet {
    fn any(&self) -> bool {
        self.on_request || self.on_provider || self.on_event || self.on_response || self.on_usage
    }

    fn wants_event(&self, name: &str) -> bool {
        match &self.events {
            None => true,
            Some(events) => events.iter().any(|event| event == name),
        }
    }

    fn names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for (enabled, name) in [
            (self.on_request, "onRequest"),
            (self.on_provider, "onProvider"),
            (self.on_event, "onEvent"),
            (self.on_response, "onResponse"),
            (self.on_usage, "onUsage"),
        ] {
            if enabled {
                names.push(name.to_string());
            }
        }
        names
    }
}

struct LoadedMiddleware {
    plugin_id: String,
    spec: String,
    name: String,
    entry: PathBuf,
    root: PathBuf,
    options: JsonValue,
    hooks: HookSet,
    stats: Arc<MiddlewareStats>,
}

struct LoadError {
    plugin_id: String,
    spec: String,
    name: String,
    path: String,
    error: String,
}

struct Snapshot {
    modules: Vec<Arc<LoadedMiddleware>>,
    errors: Vec<LoadError>,
    has_any: bool,
    has_usage: bool,
    has_request: bool,
    has_provider: bool,
    has_event: bool,
    has_response: bool,
}

impl Snapshot {
    fn empty() -> Self {
        Self {
            modules: Vec::new(),
            errors: Vec::new(),
            has_any: false,
            has_usage: false,
            has_request: false,
            has_provider: false,
            has_event: false,
            has_response: false,
        }
    }
}

/// Shared middleware state managed alongside the plugin manager.
pub struct MiddlewareRuntime {
    db: SqliteDbState,
    paths: PluginPaths,
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
    reload_lock: tokio::sync::Mutex<()>,
    usage_tx: mpsc::Sender<JsonValue>,
    usage_rx: StdMutex<Option<mpsc::Receiver<JsonValue>>>,
    usage_worker_started: AtomicBool,
}

impl MiddlewareRuntime {
    pub fn new(db: SqliteDbState, paths: PluginPaths) -> Self {
        let (usage_tx, usage_rx) = mpsc::channel(USAGE_QUEUE_CAPACITY);
        Self {
            db,
            paths,
            snapshot: Arc::new(RwLock::new(Arc::new(Snapshot::empty()))),
            reload_lock: tokio::sync::Mutex::new(()),
            usage_tx,
            usage_rx: StdMutex::new(Some(usage_rx)),
            usage_worker_started: AtomicBool::new(false),
        }
    }

    fn current(&self) -> Arc<Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Whether any enabled middleware wants a gateway hook. The gateway calls
    /// this before it parses a body, so a plugin-free installation pays
    /// nothing.
    pub fn has_active_middlewares(&self) -> bool {
        self.current().has_any
    }

    /// Per-hook presence checks. The gateway consults these before it pays
    /// for body parsing (`onRequest`), body copies (`onProvider`), response
    /// rewriting (`onResponse`) or SSE framing (`onEvent`).
    pub fn has_request_hooks(&self) -> bool {
        self.current().has_request
    }

    pub fn has_provider_hooks(&self) -> bool {
        self.current().has_provider
    }

    pub fn has_event_hooks(&self) -> bool {
        self.current().has_event
    }

    pub fn has_response_hooks(&self) -> bool {
        self.current().has_response
    }

    pub fn has_usage_hooks(&self) -> bool {
        self.current().has_usage
    }

    /// Re-read installed plugins, compile-check their modules and swap the
    /// active set. Called at startup and after every plugin change.
    pub async fn reload(&self) -> Vec<MiddlewareStateView> {
        let _guard = self.reload_lock.lock().await;
        let records = match store::list_installed_plugins(&self.db) {
            Ok(records) => records,
            Err(error) => {
                log::warn!("failed to list plugins for the middleware runtime: {error}");
                return self.states();
            }
        };

        let mut modules = Vec::new();
        let mut errors = Vec::new();
        for record in records {
            if !record.enabled
                || !matches!(record.kind, PluginKind::Middleware | PluginKind::Both)
                || record.middleware_entry.trim().is_empty()
            {
                continue;
            }
            let Some((entry, root)) = entry_and_root(&self.paths, &record) else {
                errors.push(LoadError {
                    plugin_id: record.id.clone(),
                    spec: record.spec.clone(),
                    name: display_name(&record),
                    path: String::new(),
                    error: "The middleware entry file does not exist".to_string(),
                });
                continue;
            };

            let entry_for_discovery = entry.clone();
            let root_for_discovery = root.clone();
            let discovered = tokio::task::spawn_blocking(move || {
                discover_module(&entry_for_discovery, &root_for_discovery)
            })
            .await;
            match discovered {
                Ok(Ok(hooks)) => modules.push(Arc::new(LoadedMiddleware {
                    plugin_id: record.id.clone(),
                    spec: record.spec.clone(),
                    name: display_name(&record),
                    entry,
                    root,
                    options: record.options.clone(),
                    hooks,
                    stats: Arc::new(MiddlewareStats::default()),
                })),
                Ok(Err(error)) => errors.push(LoadError {
                    plugin_id: record.id.clone(),
                    spec: record.spec.clone(),
                    name: display_name(&record),
                    path: entry.to_string_lossy().to_string(),
                    error,
                }),
                Err(error) => errors.push(LoadError {
                    plugin_id: record.id.clone(),
                    spec: record.spec.clone(),
                    name: display_name(&record),
                    path: entry.to_string_lossy().to_string(),
                    error: format!("Middleware load task failed: {error}"),
                }),
            }
        }

        let has_any = modules.iter().any(|module| module.hooks.any());
        let has_usage = modules.iter().any(|module| module.hooks.on_usage);
        let has_request = modules.iter().any(|module| module.hooks.on_request);
        let has_provider = modules.iter().any(|module| module.hooks.on_provider);
        let has_event = modules.iter().any(|module| module.hooks.on_event);
        let has_response = modules.iter().any(|module| module.hooks.on_response);
        let snapshot = Snapshot {
            modules,
            errors,
            has_any,
            has_usage,
            has_request,
            has_provider,
            has_event,
            has_response,
        };
        log::info!(
            "gateway middleware reloaded: {} active, {} failed",
            snapshot.modules.len(),
            snapshot.errors.len()
        );
        *self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(snapshot);
        self.ensure_usage_worker();
        self.states()
    }

    pub fn states(&self) -> Vec<MiddlewareStateView> {
        let snapshot = self.current();
        let mut views = Vec::with_capacity(snapshot.modules.len() + snapshot.errors.len());
        for module in &snapshot.modules {
            views.push(MiddlewareStateView {
                plugin_id: module.plugin_id.clone(),
                spec: module.spec.clone(),
                name: module.name.clone(),
                path: module.entry.to_string_lossy().to_string(),
                hooks: module.hooks.names(),
                events: module.hooks.events.clone(),
                calls: module.stats.calls(),
                failures: module.stats.failures(),
                avg_micros: module.stats.average_micros(),
                last_error: module.stats.last_error(),
                load_error: None,
                usage_rows: module.stats.usage_rows(),
                usage_dropped: module.stats.usage_dropped(),
            });
        }
        for error in &snapshot.errors {
            views.push(MiddlewareStateView {
                plugin_id: error.plugin_id.clone(),
                spec: error.spec.clone(),
                name: error.name.clone(),
                path: error.path.clone(),
                hooks: Vec::new(),
                events: None,
                calls: 0,
                failures: 0,
                avg_micros: 0,
                last_error: None,
                load_error: Some(error.error.clone()),
                usage_rows: 0,
                usage_dropped: 0,
            });
        }
        views
    }

    // ---- hooks -------------------------------------------------------------

    /// Run `onRequest` on one agent body. A reject ends the chain; every other
    /// failure keeps the previous body (fail-open).
    pub async fn run_on_request(
        &self,
        body: &[u8],
        info: &RequestInfo,
        state: &MiddlewareState,
    ) -> RequestOutcome {
        let snapshot = self.current();
        let modules: Vec<Arc<LoadedMiddleware>> = snapshot
            .modules
            .iter()
            .filter(|module| module.hooks.on_request)
            .cloned()
            .collect();
        if modules.is_empty() {
            return RequestOutcome::Continue(body.to_vec());
        }
        let mut value: JsonValue = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(_) => return RequestOutcome::Continue(body.to_vec()),
        };
        if !value.is_object() {
            return RequestOutcome::Continue(body.to_vec());
        }
        let original_stream = value.get("stream").cloned();
        let info_json = serde_json::to_value(info).unwrap_or_else(|_| json!({}));

        for module in modules {
            let ctx_json = state.ctx_json(&info_json, &module, json!({}));
            match invoke(&module, Hook::OnRequest, &value, &ctx_json, REQUEST_LIMIT).await {
                Ok(output) => {
                    let JsHookOutput {
                        value: hook_value,
                        reject,
                        state: next_state,
                    } = output;
                    if let Some(next_state) = next_state {
                        state.set_plugin_state(&module.plugin_id, next_state);
                    }
                    if let Some(reject) = reject {
                        return RequestOutcome::Reject {
                            status: reject.status,
                            message: reject.message,
                        };
                    }
                    match hook_value {
                        JsHookValue::Undefined => {}
                        JsHookValue::Null => {
                            module.stats.record_failure(
                                "onRequest returned null; keeping the request as it was",
                            );
                        }
                        JsHookValue::Object(mut next) => {
                            protect_stream_field(&module, &original_stream, &mut next);
                            value = next;
                        }
                    }
                }
                Err(_) => {
                    // `invoke` already recorded the failure.
                }
            }
        }

        match serde_json::to_vec(&value) {
            Ok(bytes) => RequestOutcome::Continue(bytes),
            Err(_) => RequestOutcome::Continue(body.to_vec()),
        }
    }

    /// Run `onProvider` for one provider try. `null`, a rejection, a throw or
    /// a timeout all keep the body as it was.
    pub async fn run_on_provider(
        &self,
        body: &[u8],
        info: &RequestInfo,
        target: &ProviderTarget,
        state: &MiddlewareState,
    ) -> Vec<u8> {
        let snapshot = self.current();
        let modules: Vec<Arc<LoadedMiddleware>> = snapshot
            .modules
            .iter()
            .filter(|module| module.hooks.on_provider)
            .cloned()
            .collect();
        if modules.is_empty() {
            return body.to_vec();
        }
        let mut value: JsonValue = match serde_json::from_slice::<JsonValue>(body) {
            Ok(value) if value.is_object() => value,
            _ => return body.to_vec(),
        };
        let request_json = serde_json::to_value(info).unwrap_or_else(|_| json!({}));
        let target_json = serde_json::to_value(target).unwrap_or_else(|_| json!({}));

        for module in modules {
            let ctx_json =
                state.ctx_json(&request_json, &module, json!({ "provider": target_json }));
            match invoke(&module, Hook::OnProvider, &value, &ctx_json, REQUEST_LIMIT).await {
                Ok(output) => {
                    if let Some(next_state) = output.state {
                        state.set_plugin_state(&module.plugin_id, next_state);
                    }
                    match output.value {
                        JsHookValue::Object(next) => value = next,
                        JsHookValue::Undefined => {}
                        JsHookValue::Null => module
                            .stats
                            .record_failure("onProvider returned null; a request can't be dropped"),
                    }
                }
                Err(_) => {}
            }
        }

        serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
    }

    /// Run `onResponse` on a whole JSON reply. `ctx.status` is the reply's
    /// status. `null`, a throw or a timeout keeps the body as it was.
    pub async fn run_on_response(
        &self,
        body: &[u8],
        info: &RequestInfo,
        status: u16,
        state: &MiddlewareState,
    ) -> Vec<u8> {
        let snapshot = self.current();
        let modules: Vec<Arc<LoadedMiddleware>> = snapshot
            .modules
            .iter()
            .filter(|module| module.hooks.on_response)
            .cloned()
            .collect();
        if modules.is_empty() {
            return body.to_vec();
        }
        let mut value: JsonValue = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(_) => return body.to_vec(),
        };
        let request_json = serde_json::to_value(info).unwrap_or_else(|_| json!({}));
        for module in modules {
            let ctx_json = state.ctx_json(&request_json, &module, json!({ "status": status }));
            match invoke(&module, Hook::OnResponse, &value, &ctx_json, REQUEST_LIMIT).await {
                Ok(output) => {
                    if let Some(next_state) = output.state {
                        state.set_plugin_state(&module.plugin_id, next_state);
                    }
                    match output.value {
                        JsHookValue::Object(next) => value = next,
                        JsHookValue::Undefined => {}
                        JsHookValue::Null => module.stats.record_failure(
                            "onResponse returned null; a whole reply can't be dropped",
                        ),
                    }
                }
                Err(_) => {}
            }
        }
        serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
    }

    /// Run `onEvent` for one stream event whose data is a JSON object.
    /// `events` on the module is the whitelist; absent means every event.
    pub async fn run_on_event(
        &self,
        event_name: &str,
        data: &[u8],
        info: &RequestInfo,
        state: &MiddlewareState,
    ) -> EventOutcome {
        let snapshot = self.current();
        let modules: Vec<Arc<LoadedMiddleware>> = snapshot
            .modules
            .iter()
            .filter(|module| module.hooks.on_event && module.hooks.wants_event(event_name))
            .cloned()
            .collect();
        if modules.is_empty() {
            return EventOutcome::Keep;
        }
        let mut value: JsonValue = match serde_json::from_slice::<JsonValue>(data) {
            Ok(value) if value.is_object() => value,
            _ => return EventOutcome::Keep,
        };
        let request_json = serde_json::to_value(info).unwrap_or_else(|_| json!({}));
        let mut changed = false;

        for module in modules {
            let ctx_json = state.ctx_json(&request_json, &module, json!({}));
            match invoke(&module, Hook::OnEvent, &value, &ctx_json, EVENT_LIMIT).await {
                Ok(output) => {
                    if let Some(next_state) = output.state {
                        state.set_plugin_state(&module.plugin_id, next_state);
                    }
                    match output.value {
                        JsHookValue::Undefined => {}
                        JsHookValue::Null => return EventOutcome::Drop,
                        JsHookValue::Object(next) => {
                            value = next;
                            changed = true;
                        }
                    }
                }
                Err(_) => {}
            }
        }

        if !changed {
            return EventOutcome::Keep;
        }
        match serde_json::to_vec(&value) {
            Ok(bytes) => EventOutcome::Replace(bytes),
            Err(_) => EventOutcome::Keep,
        }
    }

    /// Compile every `onEvent` module once into a session that lives as long
    /// as one SSE stream. `None` when no module wants stream events.
    pub async fn open_event_session(
        &self,
        info: &RequestInfo,
        state: &MiddlewareState,
    ) -> Option<EventSession> {
        let snapshot = self.current();
        let modules: Vec<Arc<LoadedMiddleware>> = snapshot
            .modules
            .iter()
            .filter(|module| module.hooks.on_event)
            .cloned()
            .collect();
        if modules.is_empty() {
            return None;
        }
        let info_json = serde_json::to_value(info).unwrap_or_else(|_| json!({}));
        let mut workers = Vec::with_capacity(modules.len());
        for module in modules {
            let (tx, rx) = mpsc::channel(EVENT_SESSION_QUEUE);
            let worker_module = module.clone();
            let worker_state = state.clone();
            let worker_info = info_json.clone();
            tokio::task::spawn_blocking(move || {
                run_event_session(worker_module, worker_info, worker_state, rx);
            });
            workers.push(EventWorker {
                tx,
                events: module.hooks.events.clone(),
            });
        }
        Some(EventSession { workers })
    }

    /// Queue one usage row. Never waits for a hook; a full queue drops the
    /// row and counts it on every middleware that wanted it.
    pub fn enqueue_on_usage(&self, row: JsonValue) -> bool {
        let snapshot = self.current();
        if !snapshot.has_usage {
            return true;
        }
        self.ensure_usage_worker();
        match self.usage_tx.try_send(row) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                for module in snapshot
                    .modules
                    .iter()
                    .filter(|module| module.hooks.on_usage)
                {
                    module.stats.record_usage_dropped();
                }
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn ensure_usage_worker(&self) {
        if self.usage_worker_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            self.usage_worker_started.store(false, Ordering::SeqCst);
            return;
        }
        let Some(receiver) = self
            .usage_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        else {
            return;
        };
        let snapshot = self.snapshot.clone();
        tokio::spawn(async move {
            let mut receiver = receiver;
            while let Some(row) = receiver.recv().await {
                let modules: Vec<Arc<LoadedMiddleware>> = snapshot
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .modules
                    .iter()
                    .filter(|module| module.hooks.on_usage)
                    .cloned()
                    .collect();
                for module in modules {
                    match invoke_usage(&module, &row).await {
                        Ok(()) => module.stats.record_usage_row(),
                        Err(_) => {}
                    }
                }
            }
        });
    }
}

fn display_name(record: &InstalledPluginRecord) -> String {
    if record.display_name.trim().is_empty() {
        record.package_name.clone()
    } else {
        record.display_name.clone()
    }
}

fn entry_and_root(
    paths: &PluginPaths,
    record: &InstalledPluginRecord,
) -> Option<(PathBuf, PathBuf)> {
    let entry_name = record.middleware_entry.trim();
    if entry_name.is_empty() {
        return None;
    }
    let target = installer::plugin_target_path(paths, record);
    if target.is_file() {
        let entry = target.canonicalize().ok()?;
        let root = entry.parent()?.to_path_buf();
        return Some((entry, root));
    }
    let entry = target.join(entry_name);
    if !entry.is_file() {
        return None;
    }
    let entry = entry.canonicalize().ok()?;
    let root = target.canonicalize().ok()?;
    Some((entry, root))
}

// ---- context helpers -------------------------------------------------------

fn build_ctx(
    info: &JsonValue,
    options: &JsonValue,
    state: JsonValue,
    extra: JsonValue,
) -> JsonValue {
    let mut ctx = match info {
        JsonValue::Object(map) => JsonValue::Object(map.clone()),
        _ => json!({}),
    };
    let Some(object) = ctx.as_object_mut() else {
        return ctx;
    };
    object.insert("options".to_string(), options.clone());
    object.insert("state".to_string(), state);
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    ctx
}

#[derive(Clone, Debug)]
struct Reject {
    status: u16,
    message: String,
}

enum JsHookValue {
    Undefined,
    Null,
    Object(JsonValue),
}

struct JsHookOutput {
    value: JsHookValue,
    reject: Option<Reject>,
    /// `ctx.state` after the hook, when the hook replaced or mutated it.
    state: Option<JsonValue>,
}

fn protect_stream_field(
    module: &LoadedMiddleware,
    original: &Option<JsonValue>,
    next: &mut JsonValue,
) {
    let Some(object) = next.as_object_mut() else {
        return;
    };
    let current = object.get("stream").cloned();
    if current == *original {
        return;
    }
    match original {
        Some(original) => {
            object.insert("stream".to_string(), original.clone());
        }
        None => {
            object.remove("stream");
        }
    }
    module
        .stats
        .record_failure("changed the top-level stream flag; kept the request's original value");
}

async fn invoke(
    module: &Arc<LoadedMiddleware>,
    hook: Hook,
    input: &JsonValue,
    ctx_json: &JsonValue,
    limit: Duration,
) -> Result<JsHookOutput, String> {
    let entry = module.entry.clone();
    let root = module.root.clone();
    let options = module.options.clone();
    let label = module.name.clone();
    let input_json = input.to_string();
    let ctx_json = ctx_json.to_string();
    let hook_name = hook.name();
    let started = Instant::now();
    let result = tokio::task::spawn_blocking(move || {
        run_js_hook(
            &entry,
            &root,
            &options,
            &label,
            hook_name,
            &input_json,
            &ctx_json,
            limit,
        )
    })
    .await;
    let micros = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    module.stats.record_call(micros);
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => {
            module.stats.record_failure(&error);
            Err(error)
        }
        Err(error) => {
            let message = format!("Middleware hook task failed: {error}");
            module.stats.record_failure(&message);
            Err(message)
        }
    }
}

async fn invoke_usage(module: &Arc<LoadedMiddleware>, row: &JsonValue) -> Result<(), String> {
    let entry = module.entry.clone();
    let root = module.root.clone();
    let options = module.options.clone();
    let label = module.name.clone();
    let row_json = row.to_string();
    let started = Instant::now();
    let result = tokio::task::spawn_blocking(move || {
        run_js_hook(
            &entry,
            &root,
            &options,
            &label,
            "onUsage",
            &row_json,
            "{}",
            USAGE_LIMIT,
        )
    })
    .await;
    let micros = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    match result {
        Ok(Ok(_)) => {
            module.stats.record_call(micros);
            Ok(())
        }
        Ok(Err(error)) => {
            module.stats.record_failure(&error);
            Err(error)
        }
        Err(error) => {
            let message = format!("onUsage task failed: {error}");
            module.stats.record_failure(&message);
            Err(message)
        }
    }
}

// ---- JavaScript execution --------------------------------------------------

struct PluginPathResolver {
    root: PathBuf,
}

impl PluginPathResolver {
    fn new(root: &Path) -> Self {
        Self {
            root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        }
    }
}

impl Resolver for PluginPathResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if !(name.starts_with("./") || name.starts_with("../")) {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "middleware may only import relative files inside its own plugin",
            ));
        }
        let base_dir = Path::new(base)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let joined = base_dir.join(name);
        let canonical = joined.canonicalize().map_err(|error| {
            rquickjs::Error::new_resolving_message(
                base,
                name,
                format!("cannot resolve '{}': {error}", joined.display()),
            )
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "import escapes the plugin directory",
            ));
        }
        if canonical
            .components()
            .any(|component| component.as_os_str().eq_ignore_ascii_case("node_modules"))
        {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "importing from node_modules is not supported; bundle the dependency",
            ));
        }
        Ok(canonical.to_string_lossy().to_string())
    }
}

struct PluginFileLoader;

impl Loader for PluginFileLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
        let source = std::fs::read_to_string(name).map_err(|error| {
            rquickjs::Error::new_loading_message(name, format!("cannot read module: {error}"))
        })?;
        Module::declare(ctx.clone(), name, source)
    }
}

fn prepare_runtime(root: &Path) -> Result<Runtime, String> {
    let runtime =
        Runtime::new().map_err(|error| format!("Failed to create JS runtime: {error}"))?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    runtime.set_loader(PluginPathResolver::new(root), PluginFileLoader);
    Ok(runtime)
}

/// A deadline the interrupt handler reads without touching the runtime.
///
/// `Runtime::set_interrupt_handler` locks the runtime's inner cell, and
/// `Context::with` holds that same lock for the whole closure, so the handler
/// is installed once before entering the context. Re-arming between the module
/// load and the hook only stores an atomic.
struct HookDeadline {
    started: Instant,
    deadline_millis: std::sync::atomic::AtomicU64,
    expired: AtomicBool,
}

impl HookDeadline {
    fn install(runtime: &Runtime) -> Arc<Self> {
        let deadline = Arc::new(Self {
            started: Instant::now(),
            deadline_millis: std::sync::atomic::AtomicU64::new(u64::MAX),
            expired: AtomicBool::new(false),
        });
        let handler = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let elapsed = handler.started.elapsed().as_millis() as u64;
            if elapsed >= handler.deadline_millis.load(Ordering::SeqCst) {
                handler.expired.store(true, Ordering::SeqCst);
                true
            } else {
                false
            }
        })));
        deadline
    }

    fn arm(&self, limit: Duration) {
        let now = self.started.elapsed().as_millis() as u64;
        let deadline = now
            .saturating_add(limit.as_millis() as u64)
            .saturating_add(1);
        self.deadline_millis.store(deadline, Ordering::SeqCst);
        self.expired.store(false, Ordering::SeqCst);
    }

    fn expired(&self) -> bool {
        self.expired.load(Ordering::SeqCst)
    }
}

fn load_module<'js>(
    ctx: &Ctx<'js>,
    entry: &Path,
) -> Result<Module<'js, rquickjs::module::Evaluated>, String> {
    let source = std::fs::read_to_string(entry)
        .map_err(|error| format!("Failed to read '{}': {error}", entry.display()))?;
    let (module, promise) =
        Module::declare(ctx.clone(), entry.to_string_lossy().to_string(), source)
            .map_err(|error| describe_js_error(ctx, error))?
            .eval()
            .map_err(|error| describe_js_error(ctx, error))?;
    promise
        .finish::<()>()
        .map_err(|error| describe_js_error(ctx, error))?;
    Ok(module)
}

fn discover_module(entry: &Path, root: &Path) -> Result<HookSet, String> {
    let runtime = prepare_runtime(root)?;
    let deadline = HookDeadline::install(&runtime);
    deadline.arm(LOAD_LIMIT);
    let context =
        Context::full(&runtime).map_err(|error| format!("Failed to create JS context: {error}"))?;
    let hooks = context.with(|ctx| -> Result<HookSet, String> {
        let module = load_module(&ctx, entry)?;
        if deadline.expired() {
            return Err(format!(
                "Middleware module load exceeded {} ms",
                LOAD_LIMIT.as_millis()
            ));
        }
        let namespace = module
            .namespace()
            .map_err(|error| describe_js_error(&ctx, error))?;
        let mut hooks = HookSet {
            on_request: export_is_function(&ctx, &namespace, "onRequest")?,
            on_provider: export_is_function(&ctx, &namespace, "onProvider")?,
            on_event: export_is_function(&ctx, &namespace, "onEvent")?,
            on_response: export_is_function(&ctx, &namespace, "onResponse")?,
            on_usage: export_is_function(&ctx, &namespace, "onUsage")?,
            events: export_events(&ctx, &namespace)?,
        };
        // A package may put the hooks on `default` instead of the namespace.
        if !hooks.any() {
            if let Ok(default) = namespace.get::<_, Value>("default") {
                if let Some(object) = default.into_object() {
                    hooks = HookSet {
                        on_request: object_is_function(&ctx, &object, "onRequest")?,
                        on_provider: object_is_function(&ctx, &object, "onProvider")?,
                        on_event: object_is_function(&ctx, &object, "onEvent")?,
                        on_response: object_is_function(&ctx, &object, "onResponse")?,
                        on_usage: object_is_function(&ctx, &object, "onUsage")?,
                        events: object_events(&ctx, &object)?,
                    };
                }
            }
        }
        if !hooks.any() {
            return Err(
                "Middleware exports none of onRequest, onProvider, onEvent, onResponse and onUsage"
                    .to_string(),
            );
        }
        Ok(hooks)
    })?;
    Ok(hooks)
}

fn export_is_function(ctx: &Ctx<'_>, namespace: &Object<'_>, name: &str) -> Result<bool, String> {
    object_is_function(ctx, namespace, name)
}

fn object_is_function(ctx: &Ctx<'_>, object: &Object<'_>, name: &str) -> Result<bool, String> {
    match object.get::<_, Value>(name) {
        Ok(value) => Ok(value.is_function()),
        Err(error) => Err(describe_js_error(ctx, error)),
    }
}

fn export_events(ctx: &Ctx<'_>, namespace: &Object<'_>) -> Result<Option<Vec<String>>, String> {
    match namespace.get::<_, Value>("events") {
        Ok(value) if !value.is_undefined() => Ok(read_events(&ctx, value)),
        _ => Ok(None),
    }
}

fn object_events(ctx: &Ctx<'_>, object: &Object<'_>) -> Result<Option<Vec<String>>, String> {
    match object.get::<_, Value>("events") {
        Ok(value) if !value.is_undefined() => Ok(read_events(ctx, value)),
        _ => Ok(None),
    }
}

fn read_events(_ctx: &Ctx<'_>, value: Value<'_>) -> Option<Vec<String>> {
    let array = value.into_array()?;
    let mut events = Vec::new();
    for item in array.iter::<Value>() {
        if let Ok(value) = item {
            if let Some(text) = value.as_string() {
                events.push(text.to_string().unwrap_or_default());
            }
        }
    }
    Some(events)
}

fn hook_function<'js>(namespace: &Object<'js>, hook: &str) -> Result<Function<'js>, String> {
    if let Ok(value) = namespace.get::<_, Value>(hook) {
        if let Some(function) = value.into_function() {
            return Ok(function);
        }
    }
    if let Ok(default) = namespace.get::<_, Value>("default") {
        if let Some(object) = default.into_object() {
            if let Ok(value) = object.get::<_, Value>(hook) {
                if let Some(function) = value.into_function() {
                    return Ok(function);
                }
            }
        }
    }
    Err(format!("The middleware exports no {hook} function"))
}

/// How many console lines one hook invocation may write.
const CONSOLE_LINE_LIMIT: usize = 20;
const CONSOLE_LINE_CHARS: usize = 2_000;

/// Give the module a `console` object. Magpie middleware use it freely, and
/// without it a single `console.log` would throw `ReferenceError` and make an
/// otherwise healthy hook fail open. Lines are rate limited, truncated and
/// redacted before they reach the application log.
fn install_console(ctx: &Ctx<'_>, label: &str) {
    let budget = Rc::new(RefCell::new((0usize, label.to_string())));
    let sink = budget.clone();
    let Ok(emit) = Function::new(ctx.clone(), move |level: String, message: String| {
        let mut budget = sink.borrow_mut();
        if budget.0 >= CONSOLE_LINE_LIMIT {
            return;
        }
        budget.0 += 1;
        let line: String = message.chars().take(CONSOLE_LINE_CHARS).collect();
        let line = super::bun_host::redact_secrets(&line);
        log::debug!(
            "plugin middleware {} console.{}: {line}",
            budget.1,
            level.trim()
        );
    }) else {
        return;
    };
    let Ok(console) = Object::new(ctx.clone()) else {
        return;
    };
    if console.set("__emit", emit).is_err() {
        return;
    }
    if ctx.globals().set("console", console).is_err() {
        return;
    }
    let _ = ctx.eval::<(), _>(
        r#"
        (() => {
          const emit = globalThis.console.__emit;
          const text = (value) => {
            try {
              if (typeof value === "string") return value;
              if (value instanceof Error) return value.stack || value.message || String(value);
              if (typeof value === "object" && value !== null) return JSON.stringify(value);
              return String(value);
            } catch {
              return String(value);
            }
          };
          const make = (level) => (...args) => emit(level, args.map(text).join(" "));
          globalThis.console = {
            log: make("log"), info: make("info"), warn: make("warn"),
            error: make("error"), debug: make("debug"),
          };
        })();
        "#,
    );
}

fn run_js_hook(
    entry: &Path,
    root: &Path,
    options: &JsonValue,
    label: &str,
    hook_name: &str,
    input_json: &str,
    ctx_json: &str,
    limit: Duration,
) -> Result<JsHookOutput, String> {
    let runtime = prepare_runtime(root)?;
    let deadline = HookDeadline::install(&runtime);
    deadline.arm(LOAD_LIMIT);
    let context =
        Context::full(&runtime).map_err(|error| format!("Failed to create JS context: {error}"))?;

    context.with(|ctx| -> Result<JsHookOutput, String> {
        install_console(&ctx, label);
        let module = load_module(&ctx, entry)?;
        if deadline.expired() {
            return Err(format!(
                "Middleware module load exceeded {} ms",
                LOAD_LIMIT.as_millis()
            ));
        }
        // The module is loaded; the hook gets its own deadline.
        deadline.arm(limit);

        let namespace = module
            .namespace()
            .map_err(|error| describe_js_error(&ctx, error))?;
        let function = hook_function(&namespace, hook_name)?;
        let input: Value = ctx
            .json_parse(input_json)
            .map_err(|error| describe_js_error(&ctx, error))?;

        let reject_slot: Rc<RefCell<Option<Reject>>> = Rc::new(RefCell::new(None));
        let ctx_object: Object = if hook_name == Hook::OnUsage.name() {
            let options_value: Value = ctx
                .json_parse(options.to_string())
                .map_err(|error| describe_js_error(&ctx, error))?;
            let result: Value = function
                .call((input, options_value))
                .map_err(|error| describe_js_error(&ctx, error))?;
            return finish_hook_result(&ctx, &deadline, result, None, None);
        } else {
            let value: Value = ctx
                .json_parse(ctx_json)
                .map_err(|error| describe_js_error(&ctx, error))?;
            value
                .into_object()
                .ok_or_else(|| "Middleware ctx is not a JSON object".to_string())?
        };

        let slot = reject_slot.clone();
        let reject = Function::new(
            ctx.clone(),
            move |status: Opt<u16>, message: Opt<String>| {
                let status = status
                    .0
                    .filter(|status| (400..=599).contains(status))
                    .unwrap_or(403);
                let message = message
                    .0
                    .filter(|message| !message.trim().is_empty())
                    .unwrap_or_else(|| "turned away by a gateway middleware".to_string());
                let mut slot = slot.borrow_mut();
                if slot.is_none() {
                    *slot = Some(Reject { status, message });
                }
            },
        )
        .map_err(|error| describe_js_error(&ctx, error))?;
        ctx_object
            .set("reject", reject)
            .map_err(|error| describe_js_error(&ctx, error))?;

        let result: Value = function
            .call((input, ctx_object.clone()))
            .map_err(|error| describe_js_error(&ctx, error))?;
        let reject = reject_slot.borrow().clone();
        let state = read_ctx_state(&ctx, &ctx_object);
        finish_hook_result(&ctx, &deadline, result, reject, state)
    })
}

fn read_ctx_state<'js>(ctx: &Ctx<'js>, ctx_object: &Object<'js>) -> Option<JsonValue> {
    let value = ctx_object.get::<_, Value>("state").ok()?;
    if value.is_undefined() {
        return None;
    }
    let json = ctx.json_stringify(value).ok()??;
    let text = json.to_string().ok()?;
    serde_json::from_str(&text).ok()
}

fn finish_hook_result<'js>(
    ctx: &Ctx<'js>,
    deadline: &HookDeadline,
    result: Value<'js>,
    reject: Option<Reject>,
    state: Option<JsonValue>,
) -> Result<JsHookOutput, String> {
    let result = if result.is_promise() {
        result
            .into_promise()
            .ok_or_else(|| "Middleware returned an unreadable promise".to_string())?
            .finish::<Value>()
            .map_err(|error| describe_js_error(ctx, error))?
    } else {
        result
    };
    if deadline.expired() {
        return Err("Middleware hook exceeded its time limit".to_string());
    }

    let value =
        if result.is_undefined() {
            JsHookValue::Undefined
        } else if result.is_null() {
            JsHookValue::Null
        } else if result.is_object() {
            let json = ctx
                .json_stringify(result)
                .map_err(|error| describe_js_error(ctx, error))?
                .ok_or_else(|| "Middleware returned a value that is not JSON".to_string())?;
            let text = json
                .to_string()
                .map_err(|error| format!("Middleware result is not readable: {error}"))?;
            JsHookValue::Object(serde_json::from_str(&text).map_err(|error| {
                format!("Middleware returned a value that is not JSON: {error}")
            })?)
        } else {
            return Err("Middleware returned neither an object, null nor undefined".to_string());
        };
    Ok(JsHookOutput {
        value,
        reject,
        state,
    })
}

// ---- streaming event session ----------------------------------------------

/// Frames a session may queue before a callback has to wait for its answer.
const EVENT_SESSION_QUEUE: usize = 8;

struct EventJob {
    data: String,
    respond: tokio::sync::oneshot::Sender<Result<Option<JsonValue>, String>>,
}

struct EventWorker {
    tx: mpsc::Sender<EventJob>,
    /// `None` = every event; `Some([])` = none.
    events: Option<Vec<String>>,
}

impl EventWorker {
    fn wants(&self, event_name: &str) -> bool {
        match &self.events {
            None => true,
            Some(events) => events.iter().any(|event| event == event_name),
        }
    }

    async fn call(&self, data: String) -> Result<Option<JsonValue>, String> {
        let (respond, response) = tokio::sync::oneshot::channel();
        self.tx
            .send(EventJob { data, respond })
            .await
            .map_err(|_| "The middleware event session has ended".to_string())?;
        response
            .await
            .map_err(|_| "The middleware event session dropped the frame".to_string())?
    }
}

/// One compiled `onEvent` runtime per middleware module, kept alive for the
/// whole SSE stream. Without this, a 1000-frame reply compiled every module
/// 1000 times; now it compiles once and re-arms the deadline per frame.
pub struct EventSession {
    workers: Vec<EventWorker>,
}

impl EventSession {
    /// Run one frame through every middleware that wants its event. Failures
    /// are fail-open; a `null` result drops the frame.
    pub async fn run_frame(&self, event_name: &str, data: &[u8]) -> EventOutcome {
        let Ok(mut value) = serde_json::from_slice::<JsonValue>(data) else {
            return EventOutcome::Keep;
        };
        if !value.is_object() {
            return EventOutcome::Keep;
        }
        let mut changed = false;
        for worker in &self.workers {
            if !worker.wants(event_name) {
                continue;
            }
            let Ok(text) = serde_json::to_string(&value) else {
                return EventOutcome::Keep;
            };
            match worker.call(text).await {
                Ok(None) => {}
                Ok(Some(JsonValue::Null)) => return EventOutcome::Drop,
                Ok(Some(next)) => {
                    value = next;
                    changed = true;
                }
                Err(_) => {}
            }
        }
        if !changed {
            return EventOutcome::Keep;
        }
        match serde_json::to_vec(&value) {
            Ok(bytes) => EventOutcome::Replace(bytes),
            Err(_) => EventOutcome::Keep,
        }
    }
}

/// Keep one module's runtime, module and context alive for one stream.
///
/// The worker runs until the session is dropped (the receiver closes) or a
/// hook times out. A timeout can leave QuickJS mid-exception, so the worker
/// rebuilds the runtime and continues with the next frame.
fn run_event_session(
    module: Arc<LoadedMiddleware>,
    info_json: JsonValue,
    state: MiddlewareState,
    mut jobs: mpsc::Receiver<EventJob>,
) {
    loop {
        let runtime = match prepare_runtime(&module.root) {
            Ok(runtime) => runtime,
            Err(error) => {
                fail_event_jobs(&mut jobs, &module, error);
                return;
            }
        };
        let deadline = HookDeadline::install(&runtime);
        deadline.arm(LOAD_LIMIT);
        let context = match Context::full(&runtime) {
            Ok(context) => context,
            Err(error) => {
                fail_event_jobs(
                    &mut jobs,
                    &module,
                    format!("Failed to create JS context: {error}"),
                );
                return;
            }
        };

        let mut restart = false;
        let setup = context.with(|ctx| -> Result<(), String> {
            let loaded = load_module(&ctx, &module.entry)?;
            if deadline.expired() {
                return Err(format!(
                    "Middleware module load exceeded {} ms",
                    LOAD_LIMIT.as_millis()
                ));
            }
            let namespace = loaded
                .namespace()
                .map_err(|error| describe_js_error(&ctx, error))?;
            let function = hook_function(&namespace, "onEvent")?;
            loop {
                let Some(job) = jobs.blocking_recv() else {
                    return Ok(());
                };
                deadline.arm(EVENT_LIMIT);
                match call_event_frame(&ctx, &function, &module, &info_json, &state, job.data) {
                    Ok(outcome) => {
                        let _ = job.respond.send(Ok(outcome));
                    }
                    Err(error) => {
                        module.stats.record_failure(&error);
                        let _ = job.respond.send(Err(error));
                    }
                }
                if deadline.expired() {
                    restart = true;
                    return Ok(());
                }
            }
        });
        match setup {
            Ok(()) if restart => continue,
            Ok(()) => return,
            Err(error) => {
                fail_event_jobs(&mut jobs, &module, error);
                return;
            }
        }
    }
}

fn fail_event_jobs(jobs: &mut mpsc::Receiver<EventJob>, module: &LoadedMiddleware, error: String) {
    module.stats.record_failure(&error);
    while let Some(job) = jobs.blocking_recv() {
        let _ = job.respond.send(Err(error.clone()));
    }
}

fn call_event_frame<'js>(
    ctx: &Ctx<'js>,
    function: &Function<'js>,
    module: &LoadedMiddleware,
    info_json: &JsonValue,
    state: &MiddlewareState,
    data: String,
) -> Result<Option<JsonValue>, String> {
    let input: Value = ctx
        .json_parse(data)
        .map_err(|error| describe_js_error(ctx, error))?;
    let ctx_json = state.ctx_json(info_json, module, json!({})).to_string();
    let ctx_value: Value = ctx
        .json_parse(ctx_json)
        .map_err(|error| describe_js_error(ctx, error))?;
    let ctx_object = ctx_value
        .into_object()
        .ok_or_else(|| "Middleware ctx is not a JSON object".to_string())?;

    let started = Instant::now();
    let result: Value = function
        .call((input, ctx_object.clone()))
        .map_err(|error| describe_js_error(ctx, error))?;
    let result = if result.is_promise() {
        result
            .into_promise()
            .ok_or_else(|| "Middleware returned an unreadable promise".to_string())?
            .finish::<Value>()
            .map_err(|error| describe_js_error(ctx, error))?
    } else {
        result
    };
    module
        .stats
        .record_call(started.elapsed().as_micros().min(u64::MAX as u128) as u64);

    if let Some(next_state) = read_ctx_state(ctx, &ctx_object) {
        state.set_plugin_state(&module.plugin_id, next_state);
    }
    if result.is_undefined() {
        Ok(None)
    } else if result.is_null() {
        Ok(Some(JsonValue::Null))
    } else if result.is_object() {
        let json = ctx
            .json_stringify(result)
            .map_err(|error| describe_js_error(ctx, error))?
            .ok_or_else(|| "Middleware returned a value that is not JSON".to_string())?;
        let text = json
            .to_string()
            .map_err(|error| format!("Middleware result is not readable: {error}"))?;
        serde_json::from_str::<JsonValue>(&text)
            .map(Some)
            .map_err(|error| format!("Middleware returned a value that is not JSON: {error}"))
    } else {
        Err("Middleware returned neither an object, null nor undefined".to_string())
    }
}

fn describe_js_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if ctx.has_exception() {
        let caught = ctx.catch();
        let text = if let Some(text) = caught.as_string() {
            text.to_string().unwrap_or_default()
        } else if let Some(exception) = caught.as_exception() {
            exception
                .message()
                .unwrap_or_else(|| "JavaScript exception".to_string())
        } else {
            "JavaScript exception".to_string()
        };
        format!("JavaScript error: {text}")
    } else {
        format!("JavaScript error: {error}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::types::InstalledPluginRecord;
    use serde_json::json;

    fn write_plugin(root: &Path, name: &str, source: &str) -> PathBuf {
        let directory = root.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("package.json"),
            json!({
                "name": name,
                "magpie": { "middleware": "index.middleware.js" },
            })
            .to_string(),
        )
        .unwrap();
        let entry = directory.join("index.middleware.js");
        std::fs::write(&entry, source).unwrap();
        entry
    }

    fn install_record(db: &SqliteDbState, entry: &Path, options: JsonValue) {
        let record = InstalledPluginRecord {
            id: "demo".to_string(),
            spec: "demo".to_string(),
            package_name: "demo".to_string(),
            display_name: "Demo".to_string(),
            version: "1.0.0".to_string(),
            kind: PluginKind::Middleware,
            enabled: true,
            source_type: "local".to_string(),
            local_path: entry.parent().unwrap().to_string_lossy().to_string(),
            middleware_entry: entry.file_name().unwrap().to_string_lossy().to_string(),
            options,
            ..InstalledPluginRecord::default()
        };
        store::upsert_installed_plugin(db, &record).unwrap();
    }

    fn runtime_with(root: &Path, entry: &Path, source: &str) -> (SqliteDbState, MiddlewareRuntime) {
        std::fs::create_dir_all(root).unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        std::fs::write(entry, source).unwrap();
        install_record(&db, entry, json!({}));
        let paths = PluginPaths::new(root);
        paths.ensure_dirs().unwrap();
        let runtime = MiddlewareRuntime::new(db.clone(), paths);
        (db, runtime)
    }

    fn info() -> RequestInfo {
        RequestInfo {
            protocol: "anthropic".to_string(),
            model: "claude".to_string(),
            stream: true,
            path: "/v1/messages".to_string(),
            agent: "claude".to_string(),
            caller: None,
        }
    }

    #[tokio::test]
    async fn on_request_rewrites_the_body() {
        let root = tempfile::tempdir().unwrap();
        let entry = write_plugin(
            root.path(),
            "rewrite",
            r#"export function onRequest(body, ctx) { body.model = "rewritten"; return body }"#,
        );
        let (_db, runtime) = runtime_with(
            root.path().join("data").as_path(),
            &entry,
            r#"export function onRequest(body, ctx) { body.model = "rewritten"; return body }"#,
        );
        runtime.reload().await;
        assert!(runtime.has_active_middlewares());
        let outcome = runtime
            .run_on_request(
                br#"{"model":"original","stream":true}"#,
                &info(),
                &MiddlewareState::new(),
            )
            .await;
        match outcome {
            RequestOutcome::Continue(body) => {
                let value: JsonValue = serde_json::from_slice(&body).unwrap();
                assert_eq!(value["model"], "rewritten");
                assert_eq!(value["stream"], true);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[tokio::test]
    async fn ctx_reject_turns_the_request_away() {
        let root = tempfile::tempdir().unwrap();
        let entry = write_plugin(
            root.path(),
            "guard",
            r#"export function onRequest(body, ctx) { ctx.reject(451, "blocked"); return undefined }"#,
        );
        let (_db, runtime) = runtime_with(
            root.path().join("data").as_path(),
            &entry,
            r#"export function onRequest(body, ctx) { ctx.reject(451, "blocked"); return undefined }"#,
        );
        runtime.reload().await;
        let outcome = runtime
            .run_on_request(br#"{"model":"m"}"#, &info(), &MiddlewareState::new())
            .await;
        assert_eq!(
            outcome,
            RequestOutcome::Reject {
                status: 451,
                message: "blocked".to_string()
            }
        );
    }

    #[tokio::test]
    async fn a_plugin_cannot_change_the_stream_flag() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"export function onRequest(body) { body.stream = false; body.model = "k"; return body }"#;
        let entry = write_plugin(root.path(), "stream-guard", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;
        let outcome = runtime
            .run_on_request(
                br#"{"model":"m","stream":true}"#,
                &info(),
                &MiddlewareState::new(),
            )
            .await;
        match outcome {
            RequestOutcome::Continue(body) => {
                let value: JsonValue = serde_json::from_slice(&body).unwrap();
                assert_eq!(value["stream"], true);
                assert_eq!(value["model"], "k");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_infinite_loop_is_interrupted_and_fails_open() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"export function onRequest() { while (true) {} }"#;
        let entry = write_plugin(root.path(), "spin", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;
        let started = Instant::now();
        let outcome = runtime
            .run_on_request(
                br#"{"model":"m","stream":true}"#,
                &info(),
                &MiddlewareState::new(),
            )
            .await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            outcome,
            RequestOutcome::Continue(br#"{"model":"m","stream":true}"#.to_vec())
        );
        let states = runtime.states();
        assert_eq!(states[0].failures, 1);
        assert!(states[0].last_error.is_some());
    }

    #[tokio::test]
    async fn on_provider_replaces_only_the_try_body() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"export function onProvider(body, ctx) { body.vendor = ctx.provider.id; return body }"#;
        let entry = write_plugin(root.path(), "vendor", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;
        let target = ProviderTarget {
            id: "vendor-a".to_string(),
            name: "Vendor A".to_string(),
            account: String::new(),
            model: "m".to_string(),
        };
        let body = runtime
            .run_on_provider(
                br#"{"model":"m"}"#,
                &info(),
                &target,
                &MiddlewareState::new(),
            )
            .await;
        let value: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["vendor"], "vendor-a");
    }

    #[tokio::test]
    async fn on_event_respects_the_whitelist_and_can_drop() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"
            export const events = ["content_block_delta"]
            export function onEvent(data) {
                if (data.drop) return null
                data.touched = true
                return data
            }
        "#;
        let entry = write_plugin(root.path(), "events", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;

        assert_eq!(
            runtime
                .run_on_event(
                    "message_start",
                    br#"{"a":1}"#,
                    &info(),
                    &MiddlewareState::new()
                )
                .await,
            EventOutcome::Keep
        );
        match runtime
            .run_on_event(
                "content_block_delta",
                br#"{"a":1}"#,
                &info(),
                &MiddlewareState::new(),
            )
            .await
        {
            EventOutcome::Replace(body) => {
                let value: JsonValue = serde_json::from_slice(&body).unwrap();
                assert_eq!(value["touched"], true);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert_eq!(
            runtime
                .run_on_event(
                    "content_block_delta",
                    br#"{"drop":true}"#,
                    &info(),
                    &MiddlewareState::new()
                )
                .await,
            EventOutcome::Drop
        );
    }

    #[tokio::test]
    async fn ctx_state_survives_between_hooks_of_one_request() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"
            export function onRequest(body, ctx) {
                ctx.state.n = (ctx.state.n || 0) + 1
                return body
            }
            export function onProvider(body, ctx) {
                body.seen = ctx.state.n
                ctx.state.n += 1
                return body
            }
            export function onResponse(body, ctx) {
                body.n = ctx.state.n
                return body
            }
        "#;
        let entry = write_plugin(root.path(), "stateful", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;

        let state = MiddlewareState::new();
        runtime
            .run_on_request(br#"{"model":"m"}"#, &info(), &state)
            .await;
        let target = ProviderTarget {
            id: "vendor".to_string(),
            ..ProviderTarget::default()
        };
        let body = runtime
            .run_on_provider(br#"{"model":"m"}"#, &info(), &target, &state)
            .await;
        let value: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["seen"], 1);

        let body = runtime
            .run_on_response(br#"{"ok":true}"#, &info(), 200, &state)
            .await;
        let value: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["n"], 2);

        // A different request starts from an empty state.
        let fresh = MiddlewareState::new();
        let body = runtime
            .run_on_response(br#"{}"#, &info(), 200, &fresh)
            .await;
        let value: JsonValue = serde_json::from_slice(&body).unwrap();
        assert!(value.get("n").is_none());
    }

    #[tokio::test]
    async fn console_output_does_not_break_a_hook() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"
            export function onRequest(body) {
                console.log("hello", "world")
                console.warn({ level: "warn" }, 2)
                console.error(new Error("boom"))
                return body
            }
        "#;
        let entry = write_plugin(root.path(), "chatty", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;
        let outcome = runtime
            .run_on_request(br#"{"model":"m"}"#, &info(), &MiddlewareState::new())
            .await;
        assert!(matches!(outcome, RequestOutcome::Continue(_)));
        let states = runtime.states();
        assert_eq!(states[0].failures, 0);
        assert_eq!(states[0].calls, 1);
    }

    #[tokio::test]
    async fn an_event_session_reuses_the_runtime_and_state_per_frame() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"
            export function onEvent(data, ctx) {
                ctx.state.n = (ctx.state.n || 0) + 1
                data.n = ctx.state.n
                return data
            }
            export function onResponse(body, ctx) {
                body.n = ctx.state.n
                return body
            }
        "#;
        let entry = write_plugin(root.path(), "stream-state", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;

        let state = MiddlewareState::new();
        let session = runtime
            .open_event_session(&info(), &state)
            .await
            .expect("event session");
        for expected in 1..=3 {
            match session.run_frame("message", br#"{"a":1}"#).await {
                EventOutcome::Replace(body) => {
                    let value: JsonValue = serde_json::from_slice(&body).unwrap();
                    assert_eq!(value["n"], expected);
                }
                other => panic!("unexpected outcome: {other:?}"),
            }
        }
        assert_eq!(runtime.states()[0].calls, 3);

        // onResponse sees what onEvent kept, through the same state.
        let body = runtime
            .run_on_response(br#"{}"#, &info(), 200, &state)
            .await;
        let value: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["n"], 3);
    }

    #[tokio::test]
    async fn on_usage_is_queued_and_counted() {
        let root = tempfile::tempdir().unwrap();
        let source = r#"export function onUsage(row, options) { }"#;
        let entry = write_plugin(root.path(), "usage", source);
        let (_db, runtime) = runtime_with(root.path().join("data").as_path(), &entry, source);
        runtime.reload().await;
        assert!(runtime.enqueue_on_usage(json!({ "model": "m", "tokens": 3 })));

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let states = runtime.states();
            if states[0].usage_rows == 1 {
                break;
            }
            assert!(Instant::now() < deadline, "onUsage never ran");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_plugin_without_hooks_is_reported_as_a_load_error() {
        let root = tempfile::tempdir().unwrap();
        let entry = write_plugin(root.path(), "empty", r#"export const nothing = 1"#);
        let (_db, runtime) = runtime_with(
            root.path().join("data").as_path(),
            &entry,
            r#"export const nothing = 1"#,
        );
        let states = runtime.reload().await;
        assert!(!runtime.has_active_middlewares());
        assert_eq!(states.len(), 1);
        assert!(states[0].load_error.is_some());
    }

    #[tokio::test]
    async fn relative_imports_inside_the_plugin_load() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("relative");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("package.json"),
            json!({
                "name": "relative",
                "magpie": { "middleware": "index.middleware.js" },
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            directory.join("shared.js"),
            "export const tag = 'from-shared'",
        )
        .unwrap();
        let entry = directory.join("index.middleware.js");
        std::fs::write(
            &entry,
            "import { tag } from './shared.js'; export function onRequest(body) { body.tag = tag; return body }",
        )
        .unwrap();

        let db = SqliteDbState::in_memory_for_test().unwrap();
        install_record(&db, &entry, json!({}));
        let paths = PluginPaths::new(root.path().join("data"));
        paths.ensure_dirs().unwrap();
        let runtime = MiddlewareRuntime::new(db, paths);
        runtime.reload().await;

        let outcome = runtime
            .run_on_request(br#"{"model":"m"}"#, &info(), &MiddlewareState::new())
            .await;
        match outcome {
            RequestOutcome::Continue(body) => {
                let value: JsonValue = serde_json::from_slice(&body).unwrap();
                assert_eq!(value["tag"], "from-shared");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[tokio::test]
    async fn imports_outside_the_plugin_are_refused() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            discover_module(Path::new("definitely-missing.middleware.js"), root.path()).is_err()
        );

        let entry = write_plugin(
            root.path(),
            "importer",
            r#"import "../../outside.js"; export function onRequest(body) { return body }"#,
        );
        let error = discover_module(&entry, &root.path().join("importer")).unwrap_err();
        assert!(
            error.contains("escapes") || error.contains("resolve") || error.contains("JavaScript"),
            "{error}"
        );
    }

    #[test]
    fn stream_field_protection_keeps_the_original() {
        let module = LoadedMiddleware {
            plugin_id: "p".to_string(),
            spec: "p".to_string(),
            name: "p".to_string(),
            entry: PathBuf::from("/tmp/p.js"),
            root: PathBuf::from("/tmp"),
            options: json!({}),
            hooks: HookSet::default(),
            stats: Arc::new(MiddlewareStats::default()),
        };
        let mut next = json!({ "stream": false, "model": "x" });
        protect_stream_field(&module, &Some(json!(true)), &mut next);
        assert_eq!(next["stream"], true);
        assert_eq!(next["model"], "x");
        assert_eq!(module.stats.failures(), 1);
    }
}
