mod cache_injector;
mod compat;
mod connectivity_test;
mod content_encoding;
mod header_preserving_client;
mod http_io;
mod middleware;
mod observability;
mod pipeline;
mod providers;
mod routes;
mod side_stores;
mod thinking_budget;
mod upstream;

pub(crate) use self::connectivity_test::test_gateway_provider_model_connectivity;
#[cfg(test)]
pub(crate) use self::providers::ProviderAuthStrategy;
#[cfg(test)]
pub(crate) use self::providers::UpstreamModelMapping;
pub(crate) use self::providers::{
    clear_gateway_provider_selection_cache, load_candidate_providers,
    load_candidate_providers_with_settings_and_selection, provider_priority_entries,
    GatewayProviderSelection, UpstreamProvider,
};

#[cfg(test)]
use self::http_io::DebugHttpRequest;
use self::http_io::{
    decode_inbound_request_body, json_response, read_http_request, write_response,
};
#[cfg(test)]
use self::http_io::{find_header_end, header_value};
#[cfg(test)]
use self::providers::{codex_base_url_from_config, json_object_string};
#[cfg(test)]
use self::routes::{build_target_url, match_gateway_route};
#[cfg(test)]
use self::upstream::build_upstream_headers;
use self::upstream::route_request;
#[cfg(test)]
use self::upstream::{route_request_with_options, GatewayRequestOptions};
use super::listen::bind_gateway_listener;
use super::model_health::ModelHealthRegistry;
use super::paths::ProxyGatewayPaths;
#[cfg(test)]
use super::types::GatewayConnectivityTestRequest;
#[cfg(test)]
use super::types::ProviderGatewayMeta;
use super::types::{
    GatewayCliKey, GatewayStreamOutcome, ProxyGatewayHealthCheckResult, ProxyGatewaySettings,
    ProxyGatewayStatus,
};
use crate::db::SqliteDbState;
use chrono::Utc;
#[cfg(test)]
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, HOST};
use serde_json::json;
#[cfg(test)]
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::async_runtime::JoinHandle as TauriJoinHandle;
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream as TokioTcpStream};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
const MAX_CONCURRENT_CONNECTIONS: u32 = 128;
const PROVIDER_CACHE_TTL: Duration = Duration::from_secs(30);
const RESTART_BIND_RETRY_ATTEMPTS: u32 = 4;
const RESTART_BIND_RETRY_DELAY: Duration = Duration::from_millis(50);
const BUSY_RESPONSE_BODY: &[u8] = br#"{"error":"gateway_busy","message":"too many connections"}"#;

fn bind_gateway_listener_with_retry(
    settings: &ProxyGatewaySettings,
) -> Result<super::listen::BoundGatewayListener, String> {
    let mut last_error = None;
    for attempt in 0..RESTART_BIND_RETRY_ATTEMPTS {
        match bind_gateway_listener(settings) {
            Ok(bound) => return Ok(bound),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < RESTART_BIND_RETRY_ATTEMPTS {
                    std::thread::sleep(RESTART_BIND_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "Failed to bind gateway listener".to_string()))
}

#[derive(Default)]
pub struct ProxyGatewayState {
    pub manager: Mutex<ProxyGatewayManager>,
    pub provider_switch_lock: tokio::sync::Mutex<()>,
}

impl ProxyGatewayState {
    pub fn clear_provider_cache(&self) -> Result<(), String> {
        let manager = self
            .manager
            .lock()
            .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
        manager.clear_provider_cache()
    }
}

pub struct ProxyGatewayManager {
    runtime: Option<ProxyGatewayRuntime>,
    last_settings: ProxyGatewaySettings,
    last_error: Option<String>,
}

impl Default for ProxyGatewayManager {
    fn default() -> Self {
        Self {
            runtime: None,
            last_settings: ProxyGatewaySettings::default(),
            last_error: None,
        }
    }
}

impl ProxyGatewayManager {
    pub fn start(&mut self, settings: ProxyGatewaySettings) -> Result<ProxyGatewayStatus, String> {
        self.start_internal(
            settings.clone(),
            GatewayRuntimeContext::new(settings, None, None),
        )
    }

    pub fn start_with_db(
        &mut self,
        settings: ProxyGatewaySettings,
        db: SqliteDbState,
    ) -> Result<ProxyGatewayStatus, String> {
        self.start_internal(
            settings.clone(),
            GatewayRuntimeContext::new(settings, Some(db), None),
        )
    }

    pub fn start_with_context(
        &mut self,
        settings: ProxyGatewaySettings,
        db: SqliteDbState,
        paths: ProxyGatewayPaths,
    ) -> Result<ProxyGatewayStatus, String> {
        self.start_internal(
            settings.clone(),
            GatewayRuntimeContext::new(settings, Some(db), Some(paths)),
        )
    }

    pub fn start_with_context_and_app(
        &mut self,
        settings: ProxyGatewaySettings,
        db: SqliteDbState,
        paths: ProxyGatewayPaths,
        app_handle: AppHandle,
    ) -> Result<ProxyGatewayStatus, String> {
        self.start_internal(
            settings.clone(),
            GatewayRuntimeContext::new(settings, Some(db), Some(paths)).with_app_handle(app_handle),
        )
    }

    /// Hot-restart the running gateway runtime.
    ///
    /// Unlike stop, this does not enforce CLI takeover preflight and does not
    /// change CLI manifests. It rebuilds the listener/runtime context, keeps the
    /// previous listen host/port when possible, and resets blocking runtime state
    /// such as model health cooldowns, provider cache, and side stores.
    pub fn restart_with_context(
        &mut self,
        db: SqliteDbState,
        paths: ProxyGatewayPaths,
    ) -> Result<ProxyGatewayStatus, String> {
        self.restart_internal(db, paths, None)
    }

    pub fn restart_with_context_and_app(
        &mut self,
        db: SqliteDbState,
        paths: ProxyGatewayPaths,
        app_handle: AppHandle,
    ) -> Result<ProxyGatewayStatus, String> {
        self.restart_internal(db, paths, Some(app_handle))
    }

    fn restart_internal(
        &mut self,
        db: SqliteDbState,
        paths: ProxyGatewayPaths,
        app_handle: Option<AppHandle>,
    ) -> Result<ProxyGatewayStatus, String> {
        if self.runtime.is_none() {
            return Err("Gateway is not running".to_string());
        }

        let live_settings = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.live_settings())
            .unwrap_or_else(|| self.last_settings.clone());
        let listen_host = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.listen_host.clone())
            .unwrap_or_else(|| live_settings.listen_host.clone());
        let listen_port = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.listen_port)
            .unwrap_or(live_settings.listen_port);

        // Keep the user's original port_auto_select in live settings. Only the
        // restart bind path forces the current port so CLI origins do not drift.
        let restart_settings = ProxyGatewaySettings {
            listen_host: listen_host.clone(),
            listen_port,
            ..live_settings
        };
        let bind_settings = ProxyGatewaySettings {
            listen_host,
            listen_port,
            port_auto_select: false,
            ..restart_settings.clone()
        };

        if let Some(mut runtime) = self.runtime.take() {
            runtime.stop();
        }

        let bound = match bind_gateway_listener_with_retry(&bind_settings) {
            Ok(bound) => bound,
            Err(error) => {
                return Err(self.fail_restart_after_stop(restart_settings, error));
            }
        };

        let mut context = GatewayRuntimeContext::new_reset_health(
            restart_settings.clone(),
            Some(db),
            Some(paths),
        );
        if let Some(app_handle) = app_handle {
            context = context.with_app_handle(app_handle);
        }

        match ProxyGatewayRuntime::spawn(bound, context) {
            Ok(runtime) => {
                self.last_settings = ProxyGatewaySettings {
                    listen_host: runtime.listen_host.clone(),
                    listen_port: runtime.listen_port,
                    ..restart_settings
                };
                self.last_error = None;
                self.runtime = Some(runtime);
                Ok(self.status())
            }
            Err(error) => Err(self.fail_restart_after_stop(restart_settings, error)),
        }
    }

    fn fail_restart_after_stop(
        &mut self,
        restart_settings: ProxyGatewaySettings,
        error: impl Into<String>,
    ) -> String {
        let detail = error.into();
        let message = format!(
            "Gateway restart failed and the gateway is now stopped: {detail}. Start the gateway again, or restore CLI direct mode if takeover is still enabled."
        );
        self.last_settings = restart_settings;
        self.last_error = Some(message.clone());
        message
    }

    fn start_internal(
        &mut self,
        settings: ProxyGatewaySettings,
        context: GatewayRuntimeContext,
    ) -> Result<ProxyGatewayStatus, String> {
        if self.runtime.is_some() {
            return Ok(self.status());
        }

        let bound = match bind_gateway_listener(&settings) {
            Ok(bound) => bound,
            Err(error) => {
                self.last_error = Some(error.clone());
                return Err(error);
            }
        };

        let runtime = ProxyGatewayRuntime::spawn(bound, context)?;
        self.last_settings = ProxyGatewaySettings {
            listen_host: runtime.listen_host.clone(),
            listen_port: runtime.listen_port,
            ..settings
        };
        self.last_error = None;
        self.runtime = Some(runtime);
        Ok(self.status())
    }

    pub fn stop(&mut self) -> Result<ProxyGatewayStatus, String> {
        if let Some(mut runtime) = self.runtime.take() {
            runtime.stop();
        }
        Ok(self.status())
    }

    pub fn update_runtime_settings(
        &mut self,
        settings: ProxyGatewaySettings,
    ) -> Result<(), String> {
        self.last_settings = settings.clone();
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.update_settings(settings)?;
        }
        Ok(())
    }

    pub fn clear_provider_cache(&self) -> Result<(), String> {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.clear_provider_cache()?;
        }
        Ok(())
    }

    pub fn status(&self) -> ProxyGatewayStatus {
        match &self.runtime {
            Some(runtime) => {
                let requests_per_minute_by_cli = runtime.context.request_counts_by_cli();
                ProxyGatewayStatus {
                    running: true,
                    base_url: Some(runtime.base_url.clone()),
                    listen_host: runtime.listen_host.clone(),
                    listen_port: Some(runtime.listen_port),
                    active_connections: runtime.active_connections.load(Ordering::SeqCst),
                    requests_per_minute: requests_per_minute_by_cli.values().sum(),
                    requests_per_minute_by_cli,
                    last_error: None,
                }
            }
            None => ProxyGatewayStatus::stopped(&self.last_settings, self.last_error.clone()),
        }
    }

    pub fn health_check_address(&self) -> Result<SocketAddr, ProxyGatewayHealthCheckResult> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.addr)
            .ok_or_else(|| ProxyGatewayHealthCheckResult {
                ok: false,
                status_code: None,
                error: Some("Gateway is not running".to_string()),
            })
    }

    pub fn health_check(&self) -> ProxyGatewayHealthCheckResult {
        match self.health_check_address() {
            Ok(addr) => health_check_socket(addr),
            Err(result) => result,
        }
    }

    pub fn model_health_items(&self) -> Option<Vec<super::types::GatewayModelHealthItem>> {
        self.runtime
            .as_ref()
            .and_then(|runtime| runtime.context.health_items())
    }
}

pub struct ProxyGatewayRuntime {
    addr: SocketAddr,
    listen_host: String,
    listen_port: u16,
    base_url: String,
    running: Arc<AtomicBool>,
    settings: Arc<RwLock<ProxyGatewaySettings>>,
    active_connections: Arc<AtomicU32>,
    context: GatewayRuntimeContext,
    task: Option<GatewayTaskHandle>,
    _owned_runtime: Option<tokio::runtime::Runtime>,
}

enum GatewayTaskHandle {
    Tauri(TauriJoinHandle<()>),
    Tokio(tokio::task::JoinHandle<()>),
}

impl GatewayTaskHandle {
    fn abort(self) {
        match self {
            Self::Tauri(task) => task.abort(),
            Self::Tokio(task) => task.abort(),
        }
    }
}

impl ProxyGatewayRuntime {
    fn live_settings(&self) -> ProxyGatewaySettings {
        self.settings
            .read()
            .map(|settings| settings.clone())
            .unwrap_or_else(|_| ProxyGatewaySettings {
                listen_host: self.listen_host.clone(),
                listen_port: self.listen_port,
                ..ProxyGatewaySettings::default()
            })
    }

    fn spawn(
        bound: super::listen::BoundGatewayListener,
        context: GatewayRuntimeContext,
    ) -> Result<Self, String> {
        let addr = bound
            .listener
            .local_addr()
            .map_err(|error| format!("Failed to read gateway listener address: {error}"))?;
        let running = Arc::new(AtomicBool::new(true));
        let server_running = running.clone();
        let settings = context.settings.clone();
        let active_connections = context.active_connections.clone();
        let runtime_context = context.clone();

        let (task, owned_runtime) = if tokio::runtime::Handle::try_current().is_ok() {
            let listener = TcpListener::from_std(bound.listener)
                .map_err(|error| format!("Failed to create async gateway listener: {error}"))?;
            (
                GatewayTaskHandle::Tauri(tauri::async_runtime::spawn(run_health_server(
                    listener,
                    server_running,
                    context,
                ))),
                None,
            )
        } else {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("proxy-gateway-runtime")
                .build()
                .map_err(|error| format!("Failed to create proxy gateway runtime: {error}"))?;
            let listener = {
                let _guard = runtime.enter();
                TcpListener::from_std(bound.listener)
                    .map_err(|error| format!("Failed to create async gateway listener: {error}"))?
            };
            let task = runtime.spawn(run_health_server(listener, server_running, context));
            (GatewayTaskHandle::Tokio(task), Some(runtime))
        };

        Ok(Self {
            addr,
            listen_host: bound.listen_host,
            listen_port: bound.listen_port,
            base_url: bound.base_url,
            running,
            settings,
            active_connections,
            context: runtime_context,
            task: Some(task),
            _owned_runtime: owned_runtime,
        })
    }

    fn update_settings(&self, settings: ProxyGatewaySettings) -> Result<(), String> {
        let mut live_settings = self
            .settings
            .write()
            .map_err(|_| "Proxy gateway settings lock poisoned".to_string())?;
        *live_settings = settings;
        self.context.update_health_settings(live_settings.clone());
        self.context.clear_provider_cache()?;
        Ok(())
    }

    fn clear_provider_cache(&self) -> Result<(), String> {
        clear_gateway_provider_selection_cache();
        self.context.clear_provider_cache()
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(100));
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.context.save_health_registry_now();
    }
}

impl Drop for ProxyGatewayRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
struct GatewayRuntimeContext {
    db: Option<SqliteDbState>,
    paths: Option<ProxyGatewayPaths>,
    settings: Arc<RwLock<ProxyGatewaySettings>>,
    active_connections: Arc<AtomicU32>,
    request_rate: Arc<Mutex<observability::RequestRateWindow>>,
    health_registry: Option<Arc<Mutex<ModelHealthRegistry>>>,
    health_path: Option<PathBuf>,
    app_handle: Option<AppHandle>,
    provider_cache: Arc<Mutex<HashMap<GatewayCliKey, ProviderCacheEntry>>>,
    side_stores: side_stores::GatewaySideStores,
}

#[derive(Clone)]
struct ProviderCacheEntry {
    loaded_at: Instant,
    selection: Option<providers::GatewayProviderSelection>,
    providers: Vec<providers::UpstreamProvider>,
}

#[derive(Clone)]
struct ProviderCandidates {
    selection: Option<providers::GatewayProviderSelection>,
    providers: Vec<providers::UpstreamProvider>,
}

impl GatewayRuntimeContext {
    fn new(
        settings: ProxyGatewaySettings,
        db: Option<SqliteDbState>,
        paths: Option<ProxyGatewayPaths>,
    ) -> Self {
        Self::new_with_health_mode(settings, db, paths, false)
    }

    fn new_reset_health(
        settings: ProxyGatewaySettings,
        db: Option<SqliteDbState>,
        paths: Option<ProxyGatewayPaths>,
    ) -> Self {
        Self::new_with_health_mode(settings, db, paths, true)
    }

    fn new_with_health_mode(
        settings: ProxyGatewaySettings,
        db: Option<SqliteDbState>,
        paths: Option<ProxyGatewayPaths>,
        reset_health: bool,
    ) -> Self {
        let health_path = paths.as_ref().map(|paths| paths.model_health_path());
        let health_registry = health_path.as_ref().map(|path| {
            let registry = if reset_health {
                let registry = ModelHealthRegistry::new(settings.clone());
                if let Err(error) = registry.save(path) {
                    log::warn!(
                        "Failed to reset proxy gateway model health during restart: {error}"
                    );
                }
                registry
            } else {
                ModelHealthRegistry::load(path, settings.clone()).unwrap_or_else(|error| {
                    log::warn!("Failed to load proxy gateway model health at startup: {error}");
                    ModelHealthRegistry::new(settings.clone())
                })
            };
            Arc::new(Mutex::new(registry))
        });
        Self {
            db,
            paths,
            settings: Arc::new(RwLock::new(settings)),
            active_connections: Arc::new(AtomicU32::new(0)),
            request_rate: Arc::new(Mutex::new(observability::RequestRateWindow::default())),
            health_registry,
            health_path,
            app_handle: None,
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            side_stores: side_stores::GatewaySideStores::default(),
        }
    }

    fn with_app_handle(mut self, app_handle: AppHandle) -> Self {
        self.app_handle = Some(app_handle);
        self
    }

    fn record_request_arrival(&self, cli_key: GatewayCliKey) {
        if let Ok(mut window) = self.request_rate.lock() {
            window.record(Instant::now(), cli_key);
        }
    }

    fn request_counts_by_cli(&self) -> HashMap<GatewayCliKey, u64> {
        self.request_rate
            .lock()
            .map(|mut window| window.counts_by_cli(Instant::now()))
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn requests_per_minute(&self) -> u64 {
        self.request_counts_by_cli().values().sum()
    }

    fn settings_snapshot(&self) -> ProxyGatewaySettings {
        self.settings
            .read()
            .map(|settings| settings.clone())
            .unwrap_or_else(|_| {
                let mut settings = ProxyGatewaySettings::default();
                settings.request_log_enabled = false;
                settings.metrics_enabled = false;
                settings.store_request_body = false;
                settings.store_headers = false;
                settings.store_response_body = false;
                settings
            })
    }

    fn health_items(&self) -> Option<Vec<super::types::GatewayModelHealthItem>> {
        let registry = self.health_registry.as_ref()?;
        let mut registry = registry.lock().ok()?;
        registry.refresh_due_cooldowns(Utc::now());
        Some(registry.health_items())
    }

    fn update_health_settings(&self, settings: ProxyGatewaySettings) {
        if let Some(registry) = self.health_registry.as_ref() {
            if let Ok(mut registry) = registry.lock() {
                registry.update_settings(settings);
            }
        }
    }

    fn clear_provider_cache(&self) -> Result<(), String> {
        let mut cache = self
            .provider_cache
            .lock()
            .map_err(|_| "Proxy gateway provider cache lock poisoned".to_string())?;
        cache.clear();
        Ok(())
    }

    async fn load_candidate_providers(
        &self,
        db: &SqliteDbState,
        cli_key: GatewayCliKey,
    ) -> Result<ProviderCandidates, String> {
        let now = Instant::now();
        let settings = self.settings_snapshot();
        let selection =
            providers::load_gateway_provider_selection_async(self.paths.as_ref(), cli_key).await?;
        if let Ok(cache) = self.provider_cache.lock() {
            if let Some(entry) = cache.get(&cli_key) {
                if now.duration_since(entry.loaded_at) <= PROVIDER_CACHE_TTL
                    && entry.selection == selection
                {
                    return Ok(ProviderCandidates {
                        selection: entry.selection.clone(),
                        providers: entry.providers.clone(),
                    });
                }
            }
        }

        let providers = providers::load_candidate_providers_with_settings_and_selection(
            db,
            cli_key,
            Some(&settings),
            selection.as_ref(),
        )
        .await?;
        if let Ok(mut cache) = self.provider_cache.lock() {
            cache.insert(
                cli_key,
                ProviderCacheEntry {
                    loaded_at: now,
                    selection: selection.clone(),
                    providers: providers.clone(),
                },
            );
        }
        Ok(ProviderCandidates {
            selection,
            providers,
        })
    }

    fn save_health_registry_async(&self) {
        let (Some(registry), Some(path)) =
            (self.health_registry.as_ref(), self.health_path.clone())
        else {
            return;
        };
        let Ok(snapshot) = registry.lock().map(|registry| registry.clone()) else {
            log::warn!("Failed to snapshot proxy gateway model health: lock poisoned");
            return;
        };
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = snapshot.save(&path) {
                log::warn!("Failed to flush proxy gateway model health: {error}");
            }
        });
    }

    fn save_health_registry_now(&self) {
        let (Some(registry), Some(path)) =
            (self.health_registry.as_ref(), self.health_path.as_ref())
        else {
            return;
        };
        let Ok(registry) = registry.lock() else {
            log::warn!("Failed to save proxy gateway model health: lock poisoned");
            return;
        };
        if let Err(error) = registry.save(path) {
            log::warn!("Failed to save proxy gateway model health: {error}");
        }
    }
}

struct ActiveConnectionGuard {
    counter: Arc<AtomicU32>,
}

impl ActiveConnectionGuard {
    fn try_new(counter: Arc<AtomicU32>, max_connections: u32) -> Option<Self> {
        let mut current = counter.load(Ordering::SeqCst);
        loop {
            if current >= max_connections {
                return None;
            }
            match counter.compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Some(Self { counter }),
                Err(next_current) => current = next_current,
            }
        }
    }
}

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn run_health_server(
    listener: TcpListener,
    running: Arc<AtomicBool>,
    context: GatewayRuntimeContext,
) {
    while running.load(Ordering::SeqCst) {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                let request_context = context.clone();
                tokio::spawn(async move {
                    let mut stream = stream;
                    if let Err(error) = handle_connection(&mut stream, &request_context).await {
                        log::warn!(
                            "[proxy-gateway] request_error peer={} error={}",
                            peer_addr,
                            error
                        );
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

async fn handle_connection(
    stream: &mut TokioTcpStream,
    context: &GatewayRuntimeContext,
) -> std::io::Result<()> {
    let Some(_active_connection) = ActiveConnectionGuard::try_new(
        context.active_connections.clone(),
        MAX_CONCURRENT_CONNECTIONS,
    ) else {
        // Concurrent connection cap reached: respond 503 and close. This path
        // never enters request observability, so log it here so a saturated
        // gateway does not fail silently.
        let peer = stream
            .peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        log::warn!(
            "[proxy-gateway] connection rejected peer={} active={} max={}",
            peer,
            context.active_connections.load(Ordering::SeqCst),
            MAX_CONCURRENT_CONNECTIONS
        );
        write_busy_response(stream).await?;
        return Ok(());
    };
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst);
    let mut request = match read_http_request(stream, request_id).await {
        Ok(request) => request,
        Err(error) => {
            // Header/body read failure (2s header / 30s body timeout, oversize,
            // peer reset). No parsed request exists, so this can never enter
            // request observability; log a distinct reason here so these dead
            // connections are not only the generic accept-loop warn.
            let peer = stream
                .peer_addr()
                .map(|addr| addr.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            log::warn!(
                "[proxy-gateway] request_read_failed peer={} error={}",
                peer,
                error
            );
            return Err(error);
        }
    };
    let started_at = Utc::now();
    let started_instant = Instant::now();
    let settings = context.settings_snapshot();

    // Codex Desktop official-login clients may send zstd-compressed JSON bodies.
    // Decode before routing/JSON parsing so passthrough and conversion both see plain JSON.
    let mut response = match decode_inbound_request_body(&mut request) {
        Ok(()) => route_request(&request, context).await,
        Err(message) => {
            let mut response = json_response(
                400,
                "Bad Request",
                json!({
                    "error": "invalid_request",
                    "message": message,
                }),
                "request_decode",
                None,
                &message,
            );
            response.error_category = Some("invalid_request".to_string());
            response
        }
    };
    let write_result = write_response(stream, &mut response, started_instant, &settings).await;
    let ended_at = Utc::now();
    if let Err(error) = &write_result {
        // A stream that already delivered its terminal event succeeded; a later
        // write failure (the trailing `0\r\n\r\n` / final flush) must not attach
        // a failure category or overwrite the note — the request is a success.
        let stream_already_succeeded = response.stream_outcome == GatewayStreamOutcome::Completed;
        if response.error_category.is_none() && !stream_already_succeeded {
            response.error_category = Some("client_write_failed".to_string());
        }
        // `write_streaming_body` classifies stream failures itself (idle timeout,
        // upstream stream error, no-terminal-event, pre-terminal disconnect);
        // only fall back to the raw write error when it did not produce a richer
        // reason, and never for an already-succeeded stream.
        if !stream_already_succeeded
            && (response.stream_outcome == GatewayStreamOutcome::NotStreaming
                || response.note.trim().is_empty())
        {
            response.note = format!("failed to write gateway response to client: {error}");
        }
    }
    amend_health_after_stream(context, &response);
    observability::record_gateway_observability(&request, &response, context, started_at, ended_at);
    write_result
}

/// Reconcile provider health with the *actual* stream outcome. The first-chunk
/// probe (`upstream.rs`) records `record_health_success` as soon as the probe
/// passes — before `write_streaming_body` consumes the stream — so a mid-stream
/// idle timeout / upstream stream error / no-terminal-event / pre-terminal
/// disconnect is not reflected in health. Here, after the response is fully
/// written, correct the health record for non-success streaming outcomes.
/// `Completed` and `NotStreaming` are left alone (already settled);
/// `Canceled` (client disconnect) is health-neutral by design
/// (`GatewayFailureKind::ClientCancelled` scores 0).
fn amend_health_after_stream(
    context: &GatewayRuntimeContext,
    response: &self::http_io::DebugHttpResponse,
) {
    if !response.is_streaming {
        return;
    }
    let (Some(cli_key), Some(provider_id), Some(upstream_model_id)) = (
        response.cli_key,
        response.provider_id.as_deref(),
        response.upstream_model_id.as_deref(),
    ) else {
        return;
    };
    let health_key = super::types::ProviderModelHealthKey {
        cli_key,
        provider_id: provider_id.to_string(),
        upstream_model_id: upstream_model_id.to_string(),
    };
    let changed = match response.stream_outcome {
        GatewayStreamOutcome::Completed | GatewayStreamOutcome::NotStreaming => false,
        GatewayStreamOutcome::Incomplete => self::upstream::record_health_failure(
            context,
            &health_key,
            crate::coding::proxy_gateway::model_health::GatewayFailureKind::EmptyResponse,
        ),
        GatewayStreamOutcome::Failed => {
            let kind = match response.error_category.as_deref() {
                Some("stream_idle_timeout") => {
                    crate::coding::proxy_gateway::model_health::GatewayFailureKind::Timeout
                }
                _ => crate::coding::proxy_gateway::model_health::GatewayFailureKind::Upstream5xx,
            };
            self::upstream::record_health_failure(context, &health_key, kind)
        }
        GatewayStreamOutcome::Canceled => false,
    };
    if changed {
        context.save_health_registry_async();
    }
}

async fn write_busy_response(stream: &mut TokioTcpStream) -> std::io::Result<()> {
    stream.write_all(busy_response_headers().as_bytes()).await?;
    stream.write_all(BUSY_RESPONSE_BODY).await?;
    stream.flush().await
}

fn busy_response_headers() -> String {
    format!(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        BUSY_RESPONSE_BODY.len()
    )
}

fn health_check_socket(addr: SocketAddr) -> ProxyGatewayHealthCheckResult {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2));
    let Ok(mut stream) = stream else {
        return ProxyGatewayHealthCheckResult {
            ok: false,
            status_code: None,
            error: Some("Failed to connect to gateway health endpoint".to_string()),
        };
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let request = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if let Err(error) = stream.write_all(request) {
        return ProxyGatewayHealthCheckResult {
            ok: false,
            status_code: None,
            error: Some(format!("Failed to write health request: {error}")),
        };
    }

    let mut response = String::new();
    if let Err(error) = stream.read_to_string(&mut response) {
        return ProxyGatewayHealthCheckResult {
            ok: false,
            status_code: None,
            error: Some(format!("Failed to read health response: {error}")),
        };
    }

    let status_code = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok());

    ProxyGatewayHealthCheckResult {
        ok: status_code == Some(200),
        status_code,
        error: None,
    }
}

pub(crate) async fn health_check_socket_async(addr: SocketAddr) -> ProxyGatewayHealthCheckResult {
    let stream = tokio::time::timeout(Duration::from_secs(2), TokioTcpStream::connect(addr)).await;
    let Ok(Ok(mut stream)) = stream else {
        return ProxyGatewayHealthCheckResult {
            ok: false,
            status_code: None,
            error: Some("Failed to connect to gateway health endpoint".to_string()),
        };
    };

    let request = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if let Err(error) = stream.write_all(request).await {
        return ProxyGatewayHealthCheckResult {
            ok: false,
            status_code: None,
            error: Some(format!("Failed to write health request: {error}")),
        };
    }

    let mut response = String::new();
    if let Err(error) =
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut response))
            .await
            .unwrap_or_else(|_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timed out reading health response",
                ))
            })
    {
        return ProxyGatewayHealthCheckResult {
            ok: false,
            status_code: None,
            error: Some(format!("Failed to read health response: {error}")),
        };
    }

    let status_code = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok());

    ProxyGatewayHealthCheckResult {
        ok: status_code == Some(200),
        status_code,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::cli_proxy::manifest::CliProxyManifest;
    use crate::coding::proxy_gateway::model_health::GatewayFailureKind;
    use crate::coding::proxy_gateway::request_log;
    use crate::coding::proxy_gateway::types::{
        GatewayProxyMode, ProviderModelHealthKey, ProxyGatewayRequestLogListInput,
    };
    use crate::db::helpers::{db_create, db_put};
    use crate::db::schema::DbTable;
    use crate::db::SqliteDbState;
    use futures_util::StreamExt;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn next_available_port() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
        listener.local_addr().unwrap().port()
    }

    fn debug_request(method: &str, path: &str, body: &[u8]) -> DebugHttpRequest {
        DebugHttpRequest {
            id: 42,
            method: method.to_string(),
            path: path.to_string(),
            headers: vec![
                ("Host".to_string(), "127.0.0.1".to_string()),
                ("Authorization".to_string(), "Bearer gateway".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Content-Length".to_string(), body.len().to_string()),
            ],
            body: body.to_vec(),
        }
    }

    fn start_test_upstream() -> (String, mpsc::Receiver<String>) {
        start_test_upstream_with_response(200, "OK", br#"{"ok":true}"#)
    }

    fn start_test_upstream_with_response(
        status_code: u16,
        status_text: &'static str,
        body: &'static [u8],
    ) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind upstream");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept upstream");
            let raw = read_test_http_request(&mut stream);
            tx.send(raw).expect("send captured request");
            write!(
                stream,
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nX-Upstream-Test: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                status_code,
                status_text,
                body.len()
            )
            .expect("write upstream headers");
            stream.write_all(body).expect("write upstream body");
        });
        (base_url, rx)
    }

    fn start_test_streaming_upstream() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind upstream");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept upstream");
            let raw = read_test_http_request(&mut stream);
            tx.send(raw).expect("send captured request");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Upstream-Test: yes\r\nConnection: close\r\n\r\n"
            )
            .expect("write upstream headers");
            stream
                .write_all(
                    br#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":100,"cache_read_input_tokens":20}}}

"#,
                )
                .expect("write first chunk");
            stream.flush().expect("flush first chunk");
            thread::sleep(Duration::from_millis(50));
            stream
                .write_all(
                    br#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":null},"usage":{"output_tokens":30,"cache_creation_input_tokens":5}}

"#,
                )
                .expect("write second chunk");
            stream.flush().expect("flush second chunk");
            thread::sleep(Duration::from_millis(50));
            stream
                .write_all(
                    br#"event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}

"#,
                )
                .expect("write third chunk");
        });
        (base_url, rx)
    }

    fn start_test_streaming_upstream_with_body(
        body: &'static [u8],
    ) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind upstream");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept upstream");
            let raw = read_test_http_request(&mut stream);
            tx.send(raw).expect("send captured request");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Upstream-Test: converted-stream\r\nConnection: close\r\n\r\n"
            )
            .expect("write upstream headers");
            stream.write_all(body).expect("write upstream body");
            stream.flush().expect("flush upstream body");
        });
        (base_url, rx)
    }

    fn start_test_streaming_upstream_without_body() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind upstream");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept upstream");
            let raw = read_test_http_request(&mut stream);
            tx.send(raw).expect("send captured request");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Upstream-Test: empty\r\nConnection: close\r\n\r\n"
            )
            .expect("write upstream headers");
            stream.flush().expect("flush headers");
        });
        (base_url, rx)
    }

    fn read_test_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set read timeout");
        let mut raw = Vec::new();
        let mut header_end = None;
        let mut buffer = [0_u8; 1024];
        while header_end.is_none() {
            let read = stream.read(&mut buffer).expect("read headers");
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&buffer[..read]);
            header_end = find_header_end(&raw);
        }
        let header_end = header_end.unwrap_or(raw.len());
        let header_text = String::from_utf8_lossy(&raw[..header_end]).to_string();
        let headers: Vec<(String, String)> = header_text
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
            .collect();
        let content_length = header_value(&headers, "content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body_len = raw.len().saturating_sub(header_end);
        while body_len < content_length {
            let read = stream.read(&mut buffer).expect("read body");
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&buffer[..read]);
            body_len += read;
        }
        String::from_utf8_lossy(&raw).to_string()
    }

    async fn create_test_db() -> (tempfile::TempDir, SqliteDbState) {
        let dir = tempfile::tempdir().expect("temp db");
        let db = SqliteDbState::in_memory_for_test().expect("open test db");
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::Settings,
                "app",
                &json!({"proxy_mode": "direct"}),
            )
        })
        .expect("save app settings");
        (dir, db)
    }

    fn insert_claude_provider(db: &SqliteDbState, data: Value) -> String {
        let record = db
            .with_conn(|conn| db_create(conn, DbTable::ClaudeProvider, &data))
            .expect("insert provider");
        record
            .get("id")
            .and_then(Value::as_str)
            .expect("provider id")
            .to_string()
    }

    fn assert_logged_effort(
        request: &DebugHttpRequest,
        response: &http_io::DebugHttpResponse,
        context: &GatewayRuntimeContext,
        expected: &str,
    ) {
        let now = Utc::now();
        observability::record_gateway_observability(request, response, context, now, now);
        let logs = super::super::usage_stats::request_logs(
            context.db.as_ref().unwrap(),
            &super::super::types::GatewayRequestLogFilters::default(),
            0,
            10,
            true,
        )
        .unwrap();
        assert_eq!(logs.total, 1);
        assert_eq!(logs.data[0].reasoning_effort.as_deref(), Some(expected));
    }

    fn write_gateway_manifest(
        paths: &ProxyGatewayPaths,
        cli_key: GatewayCliKey,
        mode: GatewayProxyMode,
        primary_provider_id: &str,
    ) {
        let manifest = CliProxyManifest::new(
            cli_key,
            "http://127.0.0.1:37123".to_string(),
            "2026-05-31T00:00:00Z".to_string(),
            mode,
            primary_provider_id.to_string(),
        );
        let manifest_path = paths.manifest_path(cli_key);
        std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
            .expect("create manifest parent");
        let content = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
        std::fs::write(manifest_path, format!("{content}\n")).expect("write manifest");
    }

    #[test]
    fn status_is_stopped_by_default() {
        let manager = ProxyGatewayManager::default();
        let status = manager.status();
        assert!(!status.running);
        assert_eq!(status.base_url, None);
        assert_eq!(status.requests_per_minute, 0);
    }

    #[test]
    fn health_check_reports_not_running() {
        let manager = ProxyGatewayManager::default();
        let health = manager.health_check();
        assert!(!health.ok);
        assert_eq!(health.status_code, None);
    }

    #[test]
    fn start_exposes_health_endpoint_and_stop_releases_port() {
        let port = next_available_port();
        let mut manager = ProxyGatewayManager::default();
        let status = manager
            .start(ProxyGatewaySettings {
                listen_port: port,
                ..ProxyGatewaySettings::default()
            })
            .expect("start gateway");

        assert!(status.running);
        assert_eq!(status.listen_port, Some(port));
        assert_eq!(manager.health_check().status_code, Some(200));

        manager.stop().expect("stop gateway");
        assert!(!manager.status().running);

        let rebound = TcpListener::bind(("127.0.0.1", port));
        assert!(rebound.is_ok());
    }

    #[test]
    fn restart_keeps_same_port_and_resets_runtime_health() {
        let port = next_available_port();
        let (_db_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let app_dir = tempfile::tempdir().expect("temp app dir");
        let paths = ProxyGatewayPaths::new(app_dir.path());
        let health_path = paths.model_health_path();
        let mut manager = ProxyGatewayManager::default();

        manager
            .start_with_context(
                ProxyGatewaySettings {
                    listen_port: port,
                    port_auto_select: true,
                    ..ProxyGatewaySettings::default()
                },
                db.clone(),
                paths.clone(),
            )
            .expect("start gateway");

        send_gateway_message_request(port);
        assert_eq!(manager.status().requests_per_minute, 1);
        assert_eq!(
            manager
                .status()
                .requests_per_minute_by_cli
                .get(&GatewayCliKey::Claude),
            Some(&1)
        );

        // Seed a cooling health snapshot that ordinary start would reload.
        let mut seeded = ModelHealthRegistry::new(ProxyGatewaySettings::default());
        let cooling_key = ProviderModelHealthKey {
            cli_key: GatewayCliKey::Claude,
            provider_id: "provider-1".to_string(),
            upstream_model_id: "model-a".to_string(),
        };
        seeded.record_failure(&cooling_key, GatewayFailureKind::Connection, Utc::now());
        seeded.record_failure(&cooling_key, GatewayFailureKind::Connection, Utc::now());
        seeded.save(&health_path).expect("seed health");
        assert!(
            !ModelHealthRegistry::load(&health_path, ProxyGatewaySettings::default())
                .expect("load seeded health")
                .is_model_available(&cooling_key, Utc::now())
        );

        let restarted = manager
            .restart_with_context(db, paths.clone())
            .expect("restart gateway");

        assert!(restarted.running);
        assert_eq!(restarted.requests_per_minute, 0);
        assert!(restarted.requests_per_minute_by_cli.is_empty());
        assert_eq!(restarted.listen_port, Some(port));
        assert_eq!(manager.health_check().status_code, Some(200));
        assert!(
            ModelHealthRegistry::load(&health_path, ProxyGatewaySettings::default())
                .expect("load health after restart")
                .is_model_available(&cooling_key, Utc::now())
        );
        // Restart bind forces the current port, but live settings keep user auto-select.
        assert!(manager.last_settings.port_auto_select);

        manager.stop().expect("stop gateway");
    }

    #[test]
    fn restart_when_not_running_returns_error() {
        let mut manager = ProxyGatewayManager::default();
        let (_db_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let app_dir = tempfile::tempdir().expect("temp app dir");
        let paths = ProxyGatewayPaths::new(app_dir.path());

        let error = manager
            .restart_with_context(db, paths)
            .expect_err("restart while stopped");
        assert!(error.contains("not running"));
        assert!(!manager.status().running);
    }

    #[test]
    fn fail_restart_after_stop_sets_stopped_status_and_message() {
        let mut manager = ProxyGatewayManager::default();
        let settings = ProxyGatewaySettings {
            listen_host: "127.0.0.1".to_string(),
            listen_port: 18765,
            port_auto_select: true,
            ..ProxyGatewaySettings::default()
        };

        let message = manager.fail_restart_after_stop(settings.clone(), "port already in use");

        assert!(message.contains("now stopped"));
        assert!(message.contains("port already in use"));
        assert!(message.contains("Start the gateway again"));
        assert!(!manager.status().running);
        assert_eq!(
            manager.status().last_error.as_deref(),
            Some(message.as_str())
        );
        assert_eq!(manager.last_settings.listen_port, settings.listen_port);
        assert!(manager.last_settings.port_auto_select);
    }

    #[test]
    fn start_returns_current_status_when_already_running() {
        let port = next_available_port();
        let mut manager = ProxyGatewayManager::default();
        let first = manager
            .start(ProxyGatewaySettings {
                listen_port: port,
                ..ProxyGatewaySettings::default()
            })
            .expect("start gateway");
        let second = manager
            .start(ProxyGatewaySettings {
                listen_port: next_available_port(),
                ..ProxyGatewaySettings::default()
            })
            .expect("second start");

        assert_eq!(first.base_url, second.base_url);
        manager.stop().expect("stop gateway");
    }

    #[test]
    fn clear_provider_cache_removes_runtime_entries() {
        let mut manager = ProxyGatewayManager::default();
        manager
            .start(ProxyGatewaySettings {
                listen_port: next_available_port(),
                ..ProxyGatewaySettings::default()
            })
            .expect("start gateway");

        let runtime = manager.runtime.as_ref().expect("runtime");
        {
            let mut cache = runtime
                .context
                .provider_cache
                .lock()
                .expect("provider cache lock");
            cache.insert(
                GatewayCliKey::Claude,
                ProviderCacheEntry {
                    loaded_at: Instant::now(),
                    selection: None,
                    providers: Vec::new(),
                },
            );
            assert!(!cache.is_empty());
        }

        manager
            .clear_provider_cache()
            .expect("clear provider cache");

        let cache = manager
            .runtime
            .as_ref()
            .expect("runtime")
            .context
            .provider_cache
            .lock()
            .expect("provider cache lock");
        assert!(cache.is_empty());
        drop(cache);
        manager.stop().expect("stop gateway");
    }

    #[test]
    fn busy_response_headers_use_body_length() {
        let headers = busy_response_headers();

        assert!(headers.contains(&format!("Content-Length: {}", BUSY_RESPONSE_BODY.len())));
        assert!(headers.ends_with("\r\n\r\n"));
    }

    #[test]
    fn provider_route_reports_missing_db_when_started_without_db() {
        let port = next_available_port();
        let mut manager = ProxyGatewayManager::default();
        manager
            .start(ProxyGatewaySettings {
                listen_port: port,
                ..ProxyGatewaySettings::default()
            })
            .expect("start gateway");

        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect gateway");
        let body = r#"{"model":"debug-model","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = format!(
            "POST /anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");

        assert!(response.starts_with("HTTP/1.1 503 Service Unavailable"));
        assert!(response.contains("gateway_provider_state_missing"));
        manager.stop().expect("stop gateway");
    }

    #[test]
    fn running_gateway_applies_request_log_setting_updates() {
        let port = next_available_port();
        let (_db_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let app_dir = tempfile::tempdir().expect("temp app dir");
        let paths = ProxyGatewayPaths::new(app_dir.path());
        let mut manager = ProxyGatewayManager::default();

        manager
            .start_with_context(
                ProxyGatewaySettings {
                    listen_port: port,
                    request_log_enabled: true,
                    ..ProxyGatewaySettings::default()
                },
                db,
                paths.clone(),
            )
            .expect("start gateway");

        send_gateway_message_request(port);
        let summaries = request_log::list_request_logs(
            &paths,
            ProxyGatewayRequestLogListInput { limit: Some(10) },
        )
        .expect("list logs after first request");
        assert_eq!(summaries.len(), 1);

        manager
            .update_runtime_settings(ProxyGatewaySettings {
                listen_port: port,
                request_log_enabled: false,
                ..ProxyGatewaySettings::default()
            })
            .expect("update live settings");

        send_gateway_message_request(port);
        let summaries = request_log::list_request_logs(
            &paths,
            ProxyGatewayRequestLogListInput { limit: Some(10) },
        )
        .expect("list logs after disabled request");
        assert_eq!(summaries.len(), 1);

        manager.stop().expect("stop gateway");
    }

    fn send_gateway_message_request(port: u16) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect gateway");
        let body = r#"{"model":"debug-model","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = format!(
            "POST /anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        response
    }

    #[test]
    fn gateway_routes_strip_cli_prefixes() {
        let claude = match_gateway_route("/anthropic/v1/messages?beta=1").unwrap();
        assert_eq!(claude.cli_key, GatewayCliKey::Claude);
        assert_eq!(claude.forwarded_path, "/v1/messages");
        assert_eq!(claude.query.as_deref(), Some("beta=1"));

        let codex = match_gateway_route("/openai/v1/responses").unwrap();
        assert_eq!(codex.cli_key, GatewayCliKey::Codex);
        assert_eq!(codex.forwarded_path, "/v1/responses");

        let gemini = match_gateway_route("/gemini/v1beta/models/gemini:generateContent").unwrap();
        assert_eq!(gemini.cli_key, GatewayCliKey::Gemini);
        assert_eq!(
            gemini.forwarded_path,
            "/v1beta/models/gemini:generateContent"
        );

        assert!(match_gateway_route("/openai/v2/responses").is_none());
        assert!(match_gateway_route("/anthropic-extra/v1/messages").is_none());
    }

    #[test]
    fn build_target_url_deduplicates_version_paths() {
        assert_eq!(
            build_target_url("https://api.example.com/v1", "/v1/messages", Some("a=1"))
                .unwrap()
                .to_string(),
            "https://api.example.com/v1/messages?a=1"
        );
        assert_eq!(
            build_target_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "/v1beta/models/gemini:generateContent",
                None,
            )
            .unwrap()
            .to_string(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini:generateContent"
        );
    }

    #[test]
    fn provider_config_extractors_read_existing_shapes() {
        let claude_settings = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "claude-key",
                "ANTHROPIC_BASE_URL": "https://claude.example.com/v1"
            }
        });
        let env = claude_settings.get("env").and_then(Value::as_object);
        assert_eq!(
            json_object_string(env, "ANTHROPIC_AUTH_TOKEN").as_deref(),
            Some("claude-key")
        );

        let codex_toml = r#"
model_provider = "custom"

[model_providers.custom]
base_url = "https://openai.example.com/v1"
"#;
        assert_eq!(
            codex_base_url_from_config(codex_toml).as_deref(),
            Some("https://openai.example.com/v1")
        );

        let gemini_settings = json!({
            "env": {
                "GEMINI_API_KEY": "gemini-key",
                "GOOGLE_GEMINI_BASE_URL": "https://gemini.example.com/v1beta"
            }
        });
        let env = gemini_settings.get("env").and_then(Value::as_object);
        assert_eq!(
            json_object_string(env, "GOOGLE_GEMINI_BASE_URL").as_deref(),
            Some("https://gemini.example.com/v1beta")
        );
    }

    #[test]
    fn upstream_headers_strip_gateway_auth_and_inject_provider_auth() {
        let body = br#"{"model":"debug"}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);
        let provider = UpstreamProvider {
            cli_key: GatewayCliKey::Claude,
            id: "p1".to_string(),
            name: "Provider".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "real-key".to_string(),
            target_protocol:
                crate::coding::proxy_gateway::transformer::AiProtocol::AnthropicMessages,
            auth_strategy: ProviderAuthStrategy::AnthropicApiKey,
            is_full_url: false,
            sort_index: None,
            meta: ProviderGatewayMeta::default(),
            model_mapping: UpstreamModelMapping::default(),
        };
        let headers = build_upstream_headers(&request, &provider, None).unwrap();

        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(HOST));
        assert!(!headers.contains_key(CONTENT_LENGTH));
        assert_eq!(
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("real-key")
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok()),
            Some("2023-06-01")
        );
    }

    #[test]
    fn request_rate_counts_external_arrivals_but_not_local_probes_or_imported_usage() {
        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let context = GatewayRuntimeContext::new(
            ProxyGatewaySettings {
                request_log_enabled: false,
                metrics_enabled: false,
                ..Default::default()
            },
            Some(db.clone()),
            None,
        );
        db.with_conn(|conn| conn.execute(
            "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model, created_at, data_source)
             VALUES ('imported-session', 'provider-a', 'codex', 'gpt-5', ?1, 'session')",
            [Utc::now().timestamp()],
        ).map_err(|error| error.to_string())).unwrap();
        assert_eq!(context.requests_per_minute(), 0);
        for (method, path) in [
            ("GET", "/health"),
            ("HEAD", "/anthropic"),
            ("GET", "/openai/v1"),
            ("HEAD", "/grok/v1"),
            ("GET", "/kimi/v1"),
            ("HEAD", "/gemini/v1beta"),
            ("GET", "/claude-desktop/api/hello"),
            ("HEAD", "/anthropic/api/hello"),
            ("GET", "/not-a-gateway-route"),
        ] {
            tauri::async_runtime::block_on(route_request(
                &debug_request(method, path, b""),
                &context,
            ));
        }
        assert_eq!(context.requests_per_minute(), 0);
        let request = debug_request(
            "POST",
            "/anthropic/v1/messages",
            br#"{"model":"claude-sonnet-4-6","messages":[]}"#,
        );
        // Both fail before a provider can be selected, but they are real arrivals.
        tauri::async_runtime::block_on(route_request(&request, &context));
        tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(context.requests_per_minute(), 2);
        tauri::async_runtime::block_on(route_request(
            &debug_request("GET", "/openai/v1/models", b""),
            &context,
        ));
        assert_eq!(context.requests_per_minute(), 3);
        assert_eq!(
            context.request_counts_by_cli(),
            HashMap::from([(GatewayCliKey::Claude, 2), (GatewayCliKey::Codex, 1),])
        );
    }

    #[test]
    fn request_rate_includes_requests_waiting_for_an_upstream_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (arrived_tx, arrived_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let upstream_thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_test_http_request(&mut stream);
            arrived_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").unwrap();
        });
        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        insert_claude_provider(
            &db,
            json!({
                "name": "Waiting Upstream", "category": "custom",
                "settings_config": json!({"env": {"ANTHROPIC_BASE_URL": base_url, "ANTHROPIC_AUTH_TOKEN": "test-key"}}).to_string(),
                "extra_settings_config": "{}", "is_applied": true, "is_disabled": false,
            }),
        );
        let context = GatewayRuntimeContext::new(
            ProxyGatewaySettings {
                request_log_enabled: false,
                metrics_enabled: false,
                ..Default::default()
            },
            Some(db),
            None,
        );
        let request_context = context.clone();
        let task = tauri::async_runtime::spawn(async move {
            route_request(
                &debug_request(
                    "POST",
                    "/anthropic/v1/messages",
                    br#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"hi"}]}"#,
                ),
                &request_context,
            )
            .await
        });
        arrived_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("request reached upstream");
        assert_eq!(context.requests_per_minute(), 1);
        release_tx.send(()).unwrap();
        let response = tauri::async_runtime::block_on(task).unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(context.requests_per_minute(), 1);
        upstream_thread.join().unwrap();
    }

    #[test]
    fn route_request_forwards_to_applied_claude_provider() {
        let (base_url, captured_rx) = start_test_upstream();
        let body =
            br#"{"model":"claude-sonnet-4-6","reasoning_effort":"low","output_config":{"effort":"high"},"messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages?debug=1", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "sonnetModel": "provider-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Local Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                }),
            );
        });

        let log_dir = tempfile::tempdir().unwrap();
        let context = GatewayRuntimeContext::new(
            ProxyGatewaySettings::default(),
            Some(db),
            Some(ProxyGatewayPaths::new(log_dir.path())),
        );
        let response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, br#"{"ok":true}"#);
        assert!(response
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-upstream-test") && value == "yes"));

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        let captured_lower = captured.to_ascii_lowercase();
        assert!(captured.starts_with("POST /v1/messages?debug=1 HTTP/1.1"));
        assert!(captured_lower.contains("authorization: bearer provider-key"));
        assert!(!captured_lower.contains("authorization: bearer gateway"));
        assert!(captured.contains(r#""model":"provider-sonnet""#));
        assert!(captured.contains(r#""content":"say hi""#));
        assert_logged_effort(&request, &response, &context, "high");
    }

    #[test]
    fn copilot_failed_requests_keep_effective_protocol_and_effort() {
        for failure in ["empty", "stream", "connection"] {
            let (base_url, captured_rx) = match failure {
                "empty" => start_test_upstream_with_response(
                    200,
                    "OK",
                    br#"{"id":"resp_empty","object":"response","model":"gpt-5.4","status":"completed","output":[]}"#,
                ),
                "stream" => start_test_streaming_upstream_with_body(
                    b"event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_failed\",\"status\":\"failed\",\"error\":{\"code\":\"server_error\",\"message\":\"failed upstream\"}}}\n\n",
                ),
                _ => {
                    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                    let base_url = format!("http://{}", listener.local_addr().unwrap());
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let (mut stream, _) = listener.accept().unwrap();
                        tx.send(read_test_http_request(&mut stream)).unwrap();
                        // Close after reading the request, without returning HTTP headers.
                    });
                    (base_url, rx)
                }
            };
            let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
            insert_claude_provider(
                &db,
                json!({
                    "name": "Copilot dynamic target", "category": "custom",
                    "settings_config": json!({"env": {
                        "ANTHROPIC_BASE_URL": base_url, "ANTHROPIC_AUTH_TOKEN": "test-copilot-token"
                    }}).to_string(),
                    "extra_settings_config": "{}", "is_applied": true, "is_disabled": false,
                    "meta": {"providerType": "github_copilot", "apiFormat": "openai_chat"}
                }),
            );
            let log_dir = tempfile::tempdir().unwrap();
            let context = GatewayRuntimeContext::new(
                ProxyGatewaySettings {
                    max_retry_count: 0,
                    ..ProxyGatewaySettings::default()
                },
                Some(db),
                Some(ProxyGatewayPaths::new(log_dir.path())),
            );
            let body = serde_json::to_vec(&json!({
                "model": "gpt-5.4", "max_tokens": 128, "stream": failure == "stream",
                "output_config": {"effort": "high"},
                "messages": [{"role": "user", "content": "say hi"}]
            }))
            .unwrap();
            let request = debug_request("POST", "/anthropic/v1/messages", &body);
            let response = tauri::async_runtime::block_on(route_request(&request, &context));
            let captured = captured_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(
                captured.starts_with("POST /responses HTTP/1.1"),
                "{failure}: {captured}"
            );
            assert_eq!(response.status_code, 502, "{failure}");
            assert_eq!(
                response.target_protocol,
                Some(super::super::transformer::AiProtocol::OpenAiResponses),
                "{failure}"
            );
            assert_logged_effort(&request, &response, &context, "high");
        }
    }

    #[test]
    fn route_request_preserves_upstream_response_body_for_converted_response() {
        let upstream_body = br#"{"id":"resp_test","object":"response","created_at":1764561600,"model":"gpt-5","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"converted hello","annotations":[]}],"status":"completed"}],"status":"completed","usage":{"input_tokens":8,"input_tokens_details":{"cached_tokens":0},"output_tokens":2,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":10}}"#;
        let (base_url, captured_rx) = start_test_upstream_with_response(200, "OK", upstream_body);
        let body = br#"{"model":"claude-sonnet-4-6","max_tokens":128,"output_config":{"effort":"high"},"messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "OPENAI_API_KEY": "provider-key"
                },
                "sonnetModel": "gpt-5"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Responses Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                    "meta": {
                        "apiFormat": "openai_responses"
                    }
                }),
            );
        });

        let log_dir = tempfile::tempdir().unwrap();
        let context = GatewayRuntimeContext::new(
            ProxyGatewaySettings::default(),
            Some(db),
            Some(ProxyGatewayPaths::new(log_dir.path())),
        );
        let response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        let response_value: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            response_value.get("type").and_then(Value::as_str),
            Some("message")
        );
        assert_eq!(
            response_value
                .pointer("/content/0/text")
                .and_then(Value::as_str),
            Some("converted hello")
        );

        let (upstream_response_body, upstream_response_body_bytes) = response
            .upstream_response_body_snapshot()
            .expect("upstream response body snapshot");
        assert_eq!(upstream_response_body, upstream_body);
        assert_eq!(upstream_response_body_bytes, upstream_body.len() as u64);

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        let captured_lower = captured.to_ascii_lowercase();
        assert!(captured.starts_with("POST /v1/responses HTTP/1.1"));
        assert!(captured_lower.contains("authorization: bearer provider-key"));
        assert!(captured.contains(r#""model":"gpt-5""#));
        let upstream_request: Value =
            serde_json::from_slice(response.upstream_request_body.as_deref().unwrap()).unwrap();
        assert_eq!(
            upstream_request.pointer("/reasoning/effort"),
            Some(&json!("high"))
        );
        assert_logged_effort(&request, &response, &context, "high");
    }

    #[test]
    fn route_request_answers_claude_root_probe_locally() {
        let request = debug_request("HEAD", "/anthropic", b"");

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), None, None);
        let response = tauri::async_runtime::block_on(route_request(&request, &context));

        assert_eq!(response.status_code, 204);
        assert_eq!(response.body, Vec::<u8>::new());
        assert_eq!(response.cli_key, Some(GatewayCliKey::Claude));
        assert_eq!(response.provider_id, None);
        assert_eq!(response.requested_model, None);
        assert_eq!(response.attempt_count, 0);
    }

    #[test]
    fn route_request_answers_desktop_hello_probe_locally() {
        // Claude Desktop 3P pings `HEAD /api/hello` on the gateway base URL.
        // Must answer 200 locally without any provider configured — never
        // forward to an upstream that would 404.
        let request = debug_request("HEAD", "/claude-desktop/api/hello", b"");

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), None, None);
        let response = tauri::async_runtime::block_on(route_request(&request, &context));

        assert_eq!(response.status_code, 200);
        assert_eq!(response.cli_key, Some(GatewayCliKey::ClaudeDesktop));
        assert_eq!(response.provider_id, None);
        assert_eq!(response.attempt_count, 0);
    }

    #[test]
    fn route_request_answers_claude_code_hello_probe_locally() {
        // Claude Code also warms the gateway with `GET /api/hello`.
        let request = debug_request("GET", "/anthropic/api/hello", b"");

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), None, None);
        let response = tauri::async_runtime::block_on(route_request(&request, &context));

        assert_eq!(response.status_code, 200);
        assert_eq!(response.cli_key, Some(GatewayCliKey::Claude));
        assert_eq!(response.provider_id, None);
        assert_eq!(response.attempt_count, 0);
    }

    #[test]
    fn route_request_preserves_streaming_response_body_stream() {
        let (base_url, captured_rx) = start_test_streaming_upstream();
        let body = br#"{"model":"claude-sonnet-4-6","stream":true,"messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "sonnetModel": "provider-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Streaming Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                }),
            );
        });

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), None);
        let mut response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert!(response.is_streaming);
        assert!(response.body.is_empty());

        let mut body_stream = response.body_stream.take().expect("stream body");
        let first_chunk = tauri::async_runtime::block_on(body_stream.next())
            .expect("first stream chunk")
            .expect("first stream chunk ok");
        let second_chunk = tauri::async_runtime::block_on(body_stream.next())
            .expect("second stream chunk")
            .expect("second stream chunk ok");
        assert!(String::from_utf8_lossy(&first_chunk).contains("message_start"));
        assert!(String::from_utf8_lossy(&second_chunk).contains("message_delta"));

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        assert!(captured.contains(r#""stream":true"#));
    }

    #[test]
    fn route_request_preserves_upstream_response_body_for_converted_stream() {
        let upstream_body = br#"event: response.created
data: {"type":"response.created","response":{"id":"resp_stream","model":"gpt-4o"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"stream hello","item_id":"msg_1","output_index":0,"content_index":0}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_stream","status":"completed","usage":{"input_tokens":8,"output_tokens":2,"total_tokens":10}}}

"#;
        let (base_url, captured_rx) = start_test_streaming_upstream_with_body(upstream_body);
        let body = br#"{"model":"claude-sonnet-4-6","max_tokens":128,"stream":true,"messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "OPENAI_API_KEY": "provider-key"
                },
                "sonnetModel": "gpt-4o"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Responses Streaming Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                    "meta": {
                        "apiFormat": "openai_responses"
                    }
                }),
            );
        });

        let context = GatewayRuntimeContext::new(
            ProxyGatewaySettings {
                store_response_body: true,
                ..ProxyGatewaySettings::default()
            },
            Some(db),
            None,
        );
        let mut response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert!(response.is_streaming);
        assert!(response.body.is_empty());

        let mut body_stream = response.body_stream.take().expect("stream body");
        let mut converted_body = Vec::new();
        while let Some(chunk) = tauri::async_runtime::block_on(body_stream.next()) {
            converted_body.extend(chunk.expect("stream chunk ok"));
        }
        let converted_body = String::from_utf8_lossy(&converted_body);
        assert!(converted_body.contains("event: message_start"));
        assert!(converted_body.contains("content_block_delta"));
        assert!(converted_body.contains("stream hello"));
        assert!(!converted_body.contains("response.output_text.delta"));

        let (upstream_response_body, upstream_response_body_bytes) = response
            .upstream_response_body_snapshot()
            .expect("upstream response body snapshot");
        assert_eq!(upstream_response_body, upstream_body);
        assert_eq!(upstream_response_body_bytes, upstream_body.len() as u64);

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        let captured_lower = captured.to_ascii_lowercase();
        assert!(captured.starts_with("POST /v1/responses HTTP/1.1"));
        assert!(captured_lower.contains("authorization: bearer provider-key"));
        assert!(captured.contains(r#""stream":true"#));
    }

    #[test]
    fn route_request_returns_bad_gateway_when_streaming_first_chunk_is_missing() {
        let (base_url, captured_rx) = start_test_streaming_upstream_without_body();
        let body = br#"{"model":"claude-sonnet-4-6","stream":true,"messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "sonnetModel": "provider-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Empty Streaming Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                }),
            );
        });

        let context = GatewayRuntimeContext::new(
            ProxyGatewaySettings {
                streaming_first_byte_timeout_secs: 1,
                ..ProxyGatewaySettings::default()
            },
            Some(db),
            None,
        );
        let response = tauri::async_runtime::block_on(route_request(&request, &context));

        assert_eq!(response.status_code, 502);
        assert!(!response.is_streaming);
        assert_eq!(
            response.provider_name.as_deref(),
            Some("Empty Streaming Upstream")
        );
        assert_eq!(response.error_category.as_deref(), Some("timeout"));
        assert!(
            String::from_utf8_lossy(&response.body).contains("upstream_stream_first_chunk_failed")
        );

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        assert!(captured.contains(r#""stream":true"#));
    }

    #[test]
    fn route_request_fails_over_when_streaming_first_chunk_is_missing() {
        let (first_base_url, first_rx) = start_test_streaming_upstream_without_body();
        let (second_base_url, second_rx) = start_test_streaming_upstream();
        let body = br#"{"model":"claude-sonnet-4-6","stream":true,"messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let first_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": first_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "first-key"
                },
                "sonnetModel": "first-sonnet"
            })
            .to_string();
            let second_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": second_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "second-key"
                },
                "sonnetModel": "second-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Empty Streaming Upstream",
                    "category": "custom",
                    "settings_config": first_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 0,
                }),
            );
            insert_claude_provider(
                &db,
                json!({
                    "name": "Working Streaming Upstream",
                    "category": "custom",
                    "settings_config": second_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 1,
                }),
            );
        });

        let context = GatewayRuntimeContext::new(
            ProxyGatewaySettings {
                streaming_first_byte_timeout_secs: 1,
                ..ProxyGatewaySettings::default()
            },
            Some(db),
            None,
        );
        let mut response = tauri::async_runtime::block_on(route_request(&request, &context));

        assert_eq!(response.status_code, 200);
        assert_eq!(
            response.provider_name.as_deref(),
            Some("Working Streaming Upstream")
        );
        assert!(response.failover);
        assert!(response.is_streaming);
        let mut body_stream = response.body_stream.take().expect("stream body");
        let first_chunk = tauri::async_runtime::block_on(body_stream.next())
            .expect("first stream chunk")
            .expect("first stream chunk ok");
        assert!(String::from_utf8_lossy(&first_chunk).contains("message_start"));

        let first_captured = first_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured first upstream request");
        assert!(first_captured.contains(r#""model":"first-sonnet""#));
        let second_captured = second_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured second upstream request");
        assert!(second_captured.contains(r#""model":"second-sonnet""#));
    }

    #[test]
    fn route_request_fails_over_when_non_streaming_response_is_empty() {
        let (first_base_url, first_rx) = start_test_upstream_with_response(200, "OK", b"");
        let second_body = br#"{"id":"msg_ok","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let (second_base_url, second_rx) =
            start_test_upstream_with_response(200, "OK", second_body);
        let body = br#"{"model":"claude-sonnet-4-6","max_tokens":128,"messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let first_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": first_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "first-key"
                },
                "sonnetModel": "first-sonnet"
            })
            .to_string();
            let second_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": second_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "second-key"
                },
                "sonnetModel": "second-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Empty Non-streaming Upstream",
                    "category": "custom",
                    "settings_config": first_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 0,
                }),
            );
            insert_claude_provider(
                &db,
                json!({
                    "name": "Working Non-streaming Upstream",
                    "category": "custom",
                    "settings_config": second_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 1,
                }),
            );
        });

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), None);
        let response = tauri::async_runtime::block_on(route_request(&request, &context));

        assert_eq!(response.status_code, 200);
        assert_eq!(
            response.provider_name.as_deref(),
            Some("Working Non-streaming Upstream")
        );
        assert!(response.failover);
        assert!(!response.is_streaming);
        assert!(String::from_utf8_lossy(&response.body).contains("hello"));
        assert_eq!(response.provider_attempts.len(), 2);
        assert_eq!(
            response.provider_attempts[0].error_category.as_deref(),
            Some("empty_response")
        );

        let first_captured = first_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured first upstream request");
        assert!(first_captured.contains(r#""model":"first-sonnet""#));
        let second_captured = second_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured second upstream request");
        assert!(second_captured.contains(r#""model":"second-sonnet""#));
    }

    #[test]
    fn started_gateway_forwards_provider_route_with_database_context() {
        let (base_url, captured_rx) = start_test_upstream();
        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "sonnetModel": "provider-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Runtime Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                }),
            );
        });

        let port = next_available_port();
        let mut manager = ProxyGatewayManager::default();
        manager
            .start_with_db(
                ProxyGatewaySettings {
                    listen_port: port,
                    ..ProxyGatewaySettings::default()
                },
                db,
            )
            .expect("start gateway");

        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect gateway");
        let body =
            r#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = format!(
            "POST /anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains(r#"{"ok":true}"#));
        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        assert!(captured.starts_with("POST /v1/messages HTTP/1.1"));
        assert!(captured.contains(r#""model":"provider-sonnet""#));

        manager.stop().expect("stop gateway");
    }

    #[test]
    fn route_request_single_manifest_preserves_original_claude_model() {
        let (base_url, captured_rx) = start_test_upstream();
        let body =
            br#"{"model":"claude-opus-4-6[1M]","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let app_dir = tempfile::tempdir().expect("temp app dir");
        let paths = ProxyGatewayPaths::new(app_dir.path());
        let provider_id = tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "opusModel": "provider-opus[1m]"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Single Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                }),
            )
        });
        write_gateway_manifest(
            &paths,
            GatewayCliKey::Claude,
            GatewayProxyMode::Single,
            &provider_id,
        );

        let context =
            GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), Some(paths));
        let response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert_eq!(
            response.upstream_model_id.as_deref(),
            Some("claude-opus-4-6")
        );

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        assert!(captured.contains(r#""model":"claude-opus-4-6""#));
        assert!(!captured.contains("provider-opus"));
    }

    #[test]
    fn route_request_failover_manifest_with_single_provider_keeps_claude_family_mapping() {
        let (base_url, captured_rx) = start_test_upstream();
        let body =
            br#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let app_dir = tempfile::tempdir().expect("temp app dir");
        let paths = ProxyGatewayPaths::new(app_dir.path());
        let provider_id = tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "sonnetModel": "provider-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "P0 Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                }),
            )
        });
        write_gateway_manifest(
            &paths,
            GatewayCliKey::Claude,
            GatewayProxyMode::Failover,
            &provider_id,
        );

        let context =
            GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), Some(paths));
        let response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert_eq!(
            response.upstream_model_id.as_deref(),
            Some("provider-sonnet")
        );

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        assert!(captured.contains(r#""model":"provider-sonnet""#));
    }

    #[test]
    fn route_request_fails_over_to_next_provider_after_retryable_failure() {
        let (first_base_url, first_rx) =
            start_test_upstream_with_response(429, "Too Many Requests", br#"{"error":"limited"}"#);
        let (second_base_url, second_rx) = start_test_upstream();
        let body =
            br#"{"model":"claude-opus-4-7","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let first_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": first_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "first-key"
                },
                "opusModel": "first-opus"
            })
            .to_string();
            let second_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": second_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "second-key"
                },
                "opusModel": "second-opus"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "First Upstream",
                    "category": "custom",
                    "settings_config": first_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 0,
                }),
            );
            insert_claude_provider(
                &db,
                json!({
                    "name": "Second Upstream",
                    "category": "custom",
                    "settings_config": second_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 1,
                }),
            );
        });

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), None);
        let response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert_eq!(response.provider_name.as_deref(), Some("Second Upstream"));
        assert_eq!(response.requested_model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(response.upstream_model_id.as_deref(), Some("second-opus"));
        assert!(response.failover);
        assert_eq!(response.attempt_count, 2);
        assert_eq!(response.provider_attempt_count, 1);
        assert_eq!(context.requests_per_minute(), 1);

        let first_captured = first_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured first upstream request");
        assert!(first_captured.contains(r#""model":"first-opus""#));

        let second_captured = second_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured second upstream request");
        assert!(second_captured.contains(r#""model":"second-opus""#));
    }

    #[test]
    fn route_request_fails_over_to_next_provider_after_payment_required() {
        let (first_base_url, first_rx) = start_test_upstream_with_response(
            402,
            "Payment Required",
            br#"{"error":"insufficient_quota"}"#,
        );
        let (second_base_url, second_rx) = start_test_upstream();
        let body =
            br#"{"model":"claude-opus-4-7","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let first_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": first_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "first-key"
                },
                "opusModel": "first-opus"
            })
            .to_string();
            let second_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": second_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "second-key"
                },
                "opusModel": "second-opus"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "First Upstream",
                    "category": "custom",
                    "settings_config": first_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 0,
                }),
            );
            insert_claude_provider(
                &db,
                json!({
                    "name": "Second Upstream",
                    "category": "custom",
                    "settings_config": second_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 1,
                }),
            );
        });

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), None);
        let response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert_eq!(response.provider_name.as_deref(), Some("Second Upstream"));
        assert_eq!(response.requested_model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(response.upstream_model_id.as_deref(), Some("second-opus"));
        assert!(response.failover);
        assert_eq!(response.attempt_count, 2);
        assert_eq!(response.provider_attempt_count, 1);

        let first_captured = first_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured first upstream request");
        assert!(first_captured.contains(r#""model":"first-opus""#));

        let second_captured = second_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured second upstream request");
        assert!(second_captured.contains(r#""model":"second-opus""#));
    }

    #[test]
    fn gateway_connectivity_test_overrides_provider_without_failover() {
        let (_first_base_url, first_rx) =
            start_test_upstream_with_response(429, "Too Many Requests", br#"{"error":"limited"}"#);
        let chat_body = br#"{"id":"chatcmpl_test","object":"chat.completion","created":1764561600,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let (second_base_url, second_rx) = start_test_upstream_with_response(200, "OK", chat_body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let second_provider_id = tauri::async_runtime::block_on(async {
            let first_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:9",
                    "OPENAI_API_KEY": "first-key"
                },
                "apiFormat": "openai_chat"
            })
            .to_string();
            let second_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": second_base_url,
                    "OPENAI_API_KEY": "second-key"
                },
                "apiFormat": "openai_chat"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "First Chat Upstream",
                    "category": "custom",
                    "settings_config": first_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 0,
                    "meta": {
                        "apiFormat": "openai_chat"
                    }
                }),
            );
            insert_claude_provider(
                &db,
                json!({
                    "name": "Second Chat Upstream",
                    "category": "custom",
                    "settings_config": second_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 1,
                    "meta": {
                        "apiFormat": "openai_chat"
                    }
                }),
            )
        });

        let response = tauri::async_runtime::block_on(test_gateway_provider_model_connectivity(
            ProxyGatewaySettings::default(),
            db,
            GatewayConnectivityTestRequest {
                cli_key: GatewayCliKey::Claude,
                provider_id: second_provider_id,
                prompt: "say hi".to_string(),
                stream: Some(false),
                model_ids: vec!["gpt-4o".to_string()],
                timeout_secs: Some(3),
            },
        ))
        .expect("gateway connectivity test");

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].status, "success");
        assert!(first_rx.recv_timeout(Duration::from_millis(200)).is_err());

        let second_captured = second_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured second upstream request");
        let second_captured_lower = second_captured.to_ascii_lowercase();
        assert!(second_captured.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(second_captured_lower.contains("authorization: bearer second-key"));
        assert!(second_captured.contains(r#""model":"gpt-4o""#));
    }

    #[test]
    fn gateway_connectivity_options_do_not_mutate_model_health() {
        let (base_url, captured_rx) =
            start_test_upstream_with_response(429, "Too Many Requests", br#"{"error":"limited"}"#);
        let body =
            br#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        let app_dir = tempfile::tempdir().expect("temp app dir");
        let paths = ProxyGatewayPaths::new(app_dir.path());
        let provider_id = tauri::async_runtime::block_on(async {
            let settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": "provider-key"
                },
                "sonnetModel": "provider-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "Limited Upstream",
                    "category": "custom",
                    "settings_config": settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": true,
                    "is_disabled": false,
                }),
            )
        });

        let context =
            GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), Some(paths));
        let response = tauri::async_runtime::block_on(route_request_with_options(
            &request,
            &context,
            &GatewayRequestOptions {
                provider_override_id: Some(provider_id),
                disable_health_mutation: true,
            },
        ));

        assert_eq!(response.status_code, 429);
        assert_eq!(response.error_category.as_deref(), Some("rate_limit"));
        assert_eq!(context.health_items().unwrap_or_default().len(), 0);
        assert_eq!(context.requests_per_minute(), 0);

        let captured = captured_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured upstream request");
        assert!(captured.contains(r#""model":"claude-sonnet-4-6""#));
    }

    #[test]
    fn route_request_retries_bad_request_on_next_provider() {
        let (first_base_url, first_rx) =
            start_test_upstream_with_response(400, "Bad Request", br#"{"error":"schema"}"#);
        let (second_base_url, second_rx) = start_test_upstream();
        let body =
            br#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"say hi"}]}"#;
        let request = debug_request("POST", "/anthropic/v1/messages", body);

        let (_dir, db) = tauri::async_runtime::block_on(create_test_db());
        tauri::async_runtime::block_on(async {
            let first_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": first_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "first-key"
                },
                "sonnetModel": "first-sonnet"
            })
            .to_string();
            let second_settings_config = json!({
                "env": {
                    "ANTHROPIC_BASE_URL": second_base_url,
                    "ANTHROPIC_AUTH_TOKEN": "second-key"
                },
                "sonnetModel": "second-sonnet"
            })
            .to_string();
            insert_claude_provider(
                &db,
                json!({
                    "name": "First Upstream",
                    "category": "custom",
                    "settings_config": first_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 0,
                }),
            );
            insert_claude_provider(
                &db,
                json!({
                    "name": "Second Upstream",
                    "category": "custom",
                    "settings_config": second_settings_config,
                    "extra_settings_config": "{}",
                    "is_applied": false,
                    "is_disabled": false,
                    "sort_index": 1,
                }),
            );
        });

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), None);
        let response = tauri::async_runtime::block_on(route_request(&request, &context));
        assert_eq!(response.status_code, 200);
        assert_eq!(response.provider_name.as_deref(), Some("Second Upstream"));
        assert_eq!(response.error_category, None);
        assert!(response.failover);

        let first_captured = first_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured first upstream request");
        assert!(first_captured.contains(r#""model":"first-sonnet""#));

        let second_captured = second_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured second upstream request");
        assert!(second_captured.contains(r#""model":"second-sonnet""#));
    }
}
