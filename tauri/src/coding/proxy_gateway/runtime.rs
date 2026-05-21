mod cache_injector;
mod debug_log;
mod header_preserving_client;
mod http_io;
mod observability;
mod providers;
mod routes;
mod thinking_budget;
mod upstream;

pub(crate) use self::providers::load_candidate_providers;

#[cfg(test)]
use self::debug_log::format_body_for_debug_log;
use self::debug_log::{log_gateway_decision, log_incoming_request, log_response};
#[cfg(test)]
use self::http_io::{find_header_end, header_value, DebugHttpRequest};
use self::http_io::{read_http_request, write_response};
#[cfg(test)]
use self::providers::{
    codex_base_url_from_config, json_object_string, UpstreamModelMapping, UpstreamProvider,
};
#[cfg(test)]
use self::routes::{build_target_url, match_gateway_route};
#[cfg(test)]
use self::upstream::build_upstream_headers;
use self::upstream::route_request;
use super::listen::bind_gateway_listener;
use super::model_health::ModelHealthRegistry;
use super::paths::ProxyGatewayPaths;
#[cfg(test)]
use super::types::ProviderGatewayMeta;
use super::types::{
    GatewayCliKey, ProxyGatewayHealthCheckResult, ProxyGatewaySettings, ProxyGatewayStatus,
};
use crate::db::SqliteDbState;
use chrono::Utc;
#[cfg(test)]
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, HOST};
#[cfg(test)]
use serde_json::{json, Value};
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
const PROVIDER_CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct ProxyGatewayState {
    pub manager: Mutex<ProxyGatewayManager>,
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

    pub fn status(&self) -> ProxyGatewayStatus {
        match &self.runtime {
            Some(runtime) => ProxyGatewayStatus {
                running: true,
                base_url: Some(runtime.base_url.clone()),
                listen_host: runtime.listen_host.clone(),
                listen_port: Some(runtime.listen_port),
                active_connections: runtime.active_connections.load(Ordering::SeqCst),
                last_error: None,
            },
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
        Ok(())
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
    health_registry: Option<Arc<Mutex<ModelHealthRegistry>>>,
    health_path: Option<PathBuf>,
    app_handle: Option<AppHandle>,
    provider_cache: Arc<Mutex<HashMap<GatewayCliKey, ProviderCacheEntry>>>,
}

#[derive(Clone)]
struct ProviderCacheEntry {
    loaded_at: Instant,
    providers: Vec<providers::UpstreamProvider>,
}

impl GatewayRuntimeContext {
    fn new(
        settings: ProxyGatewaySettings,
        db: Option<SqliteDbState>,
        paths: Option<ProxyGatewayPaths>,
    ) -> Self {
        let health_path = paths.as_ref().map(|paths| paths.model_health_path());
        let health_registry = health_path.as_ref().map(|path| {
            let registry =
                ModelHealthRegistry::load(path, settings.clone()).unwrap_or_else(|error| {
                    log::warn!("Failed to load proxy gateway model health at startup: {error}");
                    ModelHealthRegistry::new(settings.clone())
                });
            Arc::new(Mutex::new(registry))
        });
        Self {
            db,
            paths,
            settings: Arc::new(RwLock::new(settings)),
            active_connections: Arc::new(AtomicU32::new(0)),
            health_registry,
            health_path,
            app_handle: None,
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn with_app_handle(mut self, app_handle: AppHandle) -> Self {
        self.app_handle = Some(app_handle);
        self
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

    async fn load_candidate_providers(
        &self,
        db: &SqliteDbState,
        cli_key: GatewayCliKey,
    ) -> Result<Vec<providers::UpstreamProvider>, String> {
        let now = Instant::now();
        if let Ok(cache) = self.provider_cache.lock() {
            if let Some(entry) = cache.get(&cli_key) {
                if now.duration_since(entry.loaded_at) <= PROVIDER_CACHE_TTL {
                    return Ok(entry.providers.clone());
                }
            }
        }

        let providers = providers::load_candidate_providers(db, cli_key).await?;
        if let Ok(mut cache) = self.provider_cache.lock() {
            cache.insert(
                cli_key,
                ProviderCacheEntry {
                    loaded_at: now,
                    providers: providers.clone(),
                },
            );
        }
        Ok(providers)
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
                    if let Err(error) =
                        handle_connection(&mut stream, peer_addr, &request_context).await
                    {
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
    peer_addr: SocketAddr,
    context: &GatewayRuntimeContext,
) -> std::io::Result<()> {
    let Some(_active_connection) = ActiveConnectionGuard::try_new(
        context.active_connections.clone(),
        MAX_CONCURRENT_CONNECTIONS,
    ) else {
        write_busy_response(stream).await?;
        return Ok(());
    };
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst);
    let request = read_http_request(stream, request_id, peer_addr).await?;
    let started_at = Utc::now();
    let started_instant = Instant::now();
    let settings = context.settings_snapshot();
    log_incoming_request(&request, &settings);

    let mut response = route_request(&request, context).await;
    log_gateway_decision(&request, &response, &settings);
    let write_result = write_response(stream, &mut response, started_instant, &settings).await;
    let ended_at = Utc::now();
    if let Err(error) = &write_result {
        if response.error_category.is_none() {
            response.error_category = Some("client_write_failed".to_string());
        }
        response.note = format!("failed to write gateway response to client: {error}");
    }
    observability::record_gateway_observability(&request, &response, context, started_at, ended_at);
    log_response(&request, &response, &settings);
    write_result
}

async fn write_busy_response(stream: &mut TokioTcpStream) -> std::io::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: 57\r\nConnection: close\r\n\r\n{\"error\":\"gateway_busy\",\"message\":\"too many connections\"}",
        )
        .await?;
    stream.flush().await
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
    use crate::coding::proxy_gateway::request_log;
    use crate::coding::proxy_gateway::types::ProxyGatewayRequestLogListInput;
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
            peer_addr: "127.0.0.1:50000".parse().unwrap(),
            method: method.to_string(),
            path: path.to_string(),
            version: "HTTP/1.1".to_string(),
            first_line: format!("{method} {path} HTTP/1.1"),
            headers: vec![
                ("Host".to_string(), "127.0.0.1".to_string()),
                ("Authorization".to_string(), "Bearer gateway".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Content-Length".to_string(), body.len().to_string()),
            ],
            body: body.to_vec(),
            raw_len: body.len(),
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
data: {"type":"message_delta","usage":{"output_tokens":30,"cache_creation_input_tokens":5}}

"#,
                )
                .expect("write second chunk");
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

    fn insert_claude_provider(db: &SqliteDbState, data: Value) {
        db.with_conn(|conn| db_create(conn, DbTable::ClaudeProvider, &data).map(|_| ()))
            .expect("insert provider");
    }

    #[test]
    fn status_is_stopped_by_default() {
        let manager = ProxyGatewayManager::default();
        let status = manager.status();
        assert!(!status.running);
        assert_eq!(status.base_url, None);
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
    fn debug_body_log_omits_messages_field() {
        let body = br#"{"model":"debug-model","messages":[{"role":"user","content":"large"}],"metadata":{"messages":[1,2,3]}}"#;
        let formatted = format_body_for_debug_log(body);

        assert!(formatted.contains(r#""model": "debug-model""#));
        assert!(formatted.contains("[omitted messages array: 1 items]"));
        assert!(formatted.contains("[omitted messages array: 3 items]"));
        assert!(!formatted.contains("large"));
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
            sort_index: None,
            meta: ProviderGatewayMeta::default(),
            model_mapping: UpstreamModelMapping::default(),
        };
        let headers = build_upstream_headers(&request, &provider).unwrap();

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
    fn route_request_forwards_to_applied_claude_provider() {
        let (base_url, captured_rx) = start_test_upstream();
        let body =
            br#"{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"say hi"}]}"#;
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

        let context = GatewayRuntimeContext::new(ProxyGatewaySettings::default(), Some(db), None);
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
        assert!(captured_lower.contains("x-api-key: provider-key"));
        assert!(!captured_lower.contains("authorization: bearer gateway"));
        assert!(captured.contains(r#""model":"provider-sonnet""#));
        assert!(captured.contains(r#""content":"say hi""#));
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
