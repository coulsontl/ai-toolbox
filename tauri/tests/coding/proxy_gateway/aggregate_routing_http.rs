use ai_toolbox_lib::coding::proxy_gateway::{
    aggregate_naming::{AggregateNamingMode, AggregateSlugEntry},
    cli_proxy::manifest::CliProxyManifest,
    model_health::{GatewayFailureKind, ModelHealthRegistry},
    paths::ProxyGatewayPaths,
    types::{GatewayCliKey, GatewayProxyMode, ProviderModelHealthKey, ProxyGatewaySettings},
    ProxyGatewayState,
};
use ai_toolbox_lib::db::{helpers::db_put, schema::DbTable, SqliteDbState};
use ai_toolbox_lib::http_client;
use chrono::Utc;
use serde_json::{json, Value};
use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

struct RunningAggregateGateway {
    state: ProxyGatewayState,
    url: String,
    _directory: TempDir,
}

impl RunningAggregateGateway {
    fn new(
        providers: &[(&str, &str, &str, &[&str])],
        mode: GatewayProxyMode,
        selected_sites: &[&str],
    ) -> Self {
        // Legacy callers keep the historical "sites backing each other up"
        // behavior; the gate has its own dedicated cases below.
        Self::new_with_slug_table(
            providers,
            mode,
            selected_sites,
            AggregateNamingMode::default(),
            Vec::new(),
            true,
            &[],
        )
    }

    /// Same gateway, with the slug table the engage path would have persisted.
    fn new_with_slug_table(
        providers: &[(&str, &str, &str, &[&str])],
        mode: GatewayProxyMode,
        selected_sites: &[&str],
        naming: AggregateNamingMode,
        slug_table: Vec<AggregateSlugEntry>,
        cross_site_failover: bool,
        cooling_sites: &[&str],
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        db.with_conn(|connection| {
            db_put(
                connection,
                DbTable::Settings,
                "app",
                &json!({"proxy_mode": "direct"}),
            )?;
            for (index, (id, name, upstream_url, models)) in providers.iter().enumerate() {
                db_put(
                    connection,
                    DbTable::CodexProvider,
                    id,
                    &codex_provider_record(name, upstream_url, models, index as i64),
                )?;
            }
            Ok::<_, String>(())
        })
        .unwrap();

        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let paths = ProxyGatewayPaths::new(directory.path());
        let mut manifest = CliProxyManifest::new(
            GatewayCliKey::Codex,
            format!("http://127.0.0.1:{port}"),
            "2026-09-15T00:00:00Z".to_string(),
            mode,
            selected_sites
                .first()
                .copied()
                .unwrap_or("siteA")
                .to_string(),
        );
        if mode == GatewayProxyMode::Aggregate {
            manifest = manifest.with_aggregate(
                selected_sites
                    .iter()
                    .map(|site| (*site).to_string())
                    .collect(),
                ".".to_string(),
                std::collections::BTreeMap::new(),
                naming,
                cross_site_failover,
                slug_table,
            );
        }
        fs::create_dir_all(paths.manifest_path(GatewayCliKey::Codex).parent().unwrap()).unwrap();
        fs::write(
            paths.manifest_path(GatewayCliKey::Codex),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let settings = ProxyGatewaySettings {
            listen_port: port,
            port_auto_select: true,
            max_retry_count: 1,
            per_provider_retry_count: 0,
            retry_interval_secs: 0,
            non_streaming_timeout_secs: 1,
            streaming_first_byte_timeout_secs: 1,
            streaming_idle_timeout_secs: 1,
            request_log_enabled: false,
            metrics_enabled: false,
            store_request_body: false,
            store_headers: false,
            store_response_body: false,
            ..ProxyGatewaySettings::default()
        };
        // Seed the persisted health snapshot *before* the runtime loads it, so a
        // request that must be refused by the cooldown filter never reaches the
        // upstream socket. `Auth` is provider-scoped (score 5 = the default
        // threshold), i.e. one call cools the whole site.
        if !cooling_sites.is_empty() {
            let mut registry = ModelHealthRegistry::new(settings.clone());
            let now = Utc::now();
            for site_id in cooling_sites {
                registry.record_failure(
                    &ProviderModelHealthKey {
                        cli_key: GatewayCliKey::Codex,
                        provider_id: (*site_id).to_string(),
                        upstream_model_id: String::new(),
                    },
                    GatewayFailureKind::Auth,
                    now,
                );
            }
            registry.save(&paths.model_health_path()).unwrap();
        }
        let state = ProxyGatewayState::default();
        let status = state
            .manager
            .lock()
            .unwrap()
            .start_with_context(settings, db, paths)
            .unwrap();
        Self {
            state,
            url: format!(
                "http://127.0.0.1:{}/openai/v1/responses",
                status.listen_port.expect("bound gateway port")
            ),
            _directory: directory,
        }
    }
}

impl Drop for RunningAggregateGateway {
    fn drop(&mut self) {
        let _ = self.state.manager.lock().unwrap().stop();
    }
}

fn codex_provider_record(
    name: &str,
    upstream_url: &str,
    models: &[&str],
    sort_index: i64,
) -> Value {
    let config = format!(
        "model = \"fixture-model\"\nmodel_provider = \"fixture\"\n\
         [model_providers.fixture]\nbase_url = \"{upstream_url}/v1\"\nwire_api = \"responses\"\n"
    );
    json!({
        "name": name,
        "category": "custom",
        "settings_config": serde_json::to_string(&json!({
            "config": config,
            "auth": {"OPENAI_API_KEY": "fixture-key"},
            "modelCatalog": {
                "models": models.iter().map(|model| json!({"model": model})).collect::<Vec<_>>()
            }
        })).unwrap(),
        "meta": {"apiFormat": "openai_responses", "providerType": "custom"},
        "sort_index": sort_index,
        "is_applied": true,
        "is_disabled": false,
    })
}

fn responses_request_for_model(model: &str) -> Value {
    json!({
        "model": model,
        "input": [{"role": "user", "content": "say hello"}],
    })
}

fn responses_success(model: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": format!("resp_{model}"),
        "object": "response",
        "created_at": 1,
        "model": model,
        "output": [{
            "type": "message",
            "id": "msg_fixture",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}],
        }],
        "status": "completed",
        "usage": {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5},
    }))
    .unwrap()
}

fn model_not_found_response(model: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "error": {
            "type": "invalid_request_error",
            "code": "model_not_found",
            "message": format!("model {model} not found"),
        }
    }))
    .unwrap()
}

async fn read_http_json(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let size = socket.read(&mut buffer).await.unwrap();
        assert!(size > 0, "upstream request ended before headers");
        bytes.extend_from_slice(&buffer[..size]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let content_length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .expect("upstream request must have Content-Length")
        .1
        .trim()
        .parse::<usize>()
        .unwrap();
    while bytes.len() < header_end + content_length {
        let size = socket.read(&mut buffer).await.unwrap();
        assert!(size > 0, "upstream request ended before body");
        bytes.extend_from_slice(&buffer[..size]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
}

async fn write_http_json(socket: &mut TcpStream, status: u16, body: &[u8]) {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    socket.shutdown().await.unwrap();
}

async fn write_http_sse(socket: &mut TcpStream, body: &[u8]) {
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    socket.shutdown().await.unwrap();
}

/// Capture at most one real upstream request. Returning `None` means the
/// listener saw no request before the caller aborted the task.
fn spawn_upstream_capture(
    listener: TcpListener,
    status: u16,
    response_body: Vec<u8>,
) -> JoinHandle<Option<Value>> {
    tokio::spawn(async move {
        let Ok(Ok((mut socket, _))) =
            tokio::time::timeout(REQUEST_TIMEOUT, listener.accept()).await
        else {
            return None;
        };
        let body = read_http_json(&mut socket).await;
        write_http_json(&mut socket, status, &response_body).await;
        Some(body)
    })
}

async fn abort_if_idle(mut task: JoinHandle<Option<Value>>) -> Option<Value> {
    match tokio::time::timeout(Duration::from_millis(250), &mut task).await {
        Ok(Ok(captured)) => captured,
        Ok(Err(error)) => panic!("upstream capture task failed: {error}"),
        Err(_) => {
            task.abort();
            let _ = task.await;
            None
        }
    }
}

/// Accept connections for a bounded window and count them.
///
/// A stronger form of `abort_if_idle`: it proves how many times a site was
/// contacted (expected: zero), instead of only that nothing happened to arrive
/// within one short window.
fn spawn_upstream_counter(listener: TcpListener) -> (Arc<AtomicUsize>, JoinHandle<()>) {
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    let task = tokio::spawn(async move {
        loop {
            let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(REQUEST_TIMEOUT, listener.accept()).await
            else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = read_http_json(&mut socket).await;
            write_http_json(&mut socket, 200, &responses_success("modelX", "unexpected")).await;
        }
    });
    (count, task)
}

/// Wait past the point where a cross-site attempt would have been observable.
async fn settle_cross_site_window() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

async fn send_request(gateway_url: &str, model: &str) -> (reqwest::StatusCode, Vec<u8>) {
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(gateway_url)
        .json(&responses_request_for_model(model))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap().to_vec();
    (status, bytes)
}

async fn send_streaming_request(gateway_url: &str, model: &str) -> (reqwest::StatusCode, Vec<u8>) {
    let client = http_client::create_client_no_proxy(10).unwrap();
    let mut request_body = responses_request_for_model(model);
    request_body["stream"] = json!(true);
    let response = client
        .post(gateway_url)
        .json(&request_body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap().to_vec();
    (status, bytes)
}

#[tokio::test]
async fn aggregate_site_model_prefix_routes_only_to_site_a_and_strips_prefix() {
    let site_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a.local_addr().unwrap());
    let site_b_url = format!("http://{}", site_b.local_addr().unwrap());
    let site_a_task = spawn_upstream_capture(site_a, 200, responses_success("modelX", "from A"));
    let site_b_task =
        spawn_upstream_capture(site_b, 500, responses_success("modelX", "unexpected B"));

    let gateway = RunningAggregateGateway::new(
        &[
            ("siteA", "Site A", &site_a_url, &["modelX"]),
            ("siteB", "Site B", &site_b_url, &["modelX"]),
        ],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
    );
    let (status, bytes) = send_request(&gateway.url, "siteA.modelX").await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["model"], "modelX");
    assert_eq!(response["output"][0]["content"][0]["text"], "from A");

    let captured_a = tokio::time::timeout(REQUEST_TIMEOUT, site_a_task)
        .await
        .expect("site A request should arrive")
        .unwrap()
        .expect("site A request should be captured");
    assert_eq!(captured_a["model"], "modelX");
    assert_eq!(abort_if_idle(site_b_task).await, None);
}

#[tokio::test]
async fn aggregate_site_model_prefix_routes_only_to_site_b_and_strips_prefix() {
    let site_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a.local_addr().unwrap());
    let site_b_url = format!("http://{}", site_b.local_addr().unwrap());
    let site_a_task =
        spawn_upstream_capture(site_a, 500, responses_success("modelY", "unexpected A"));
    let site_b_task = spawn_upstream_capture(site_b, 200, responses_success("modelY", "from B"));

    // 跨站故障转移关闭时，点名站点仍然照常服务：闸门只禁止"换站"，不禁止
    // "访问 slug 点名的站点"。这是关闭开关后的正常路径。
    let gateway = RunningAggregateGateway::new_with_slug_table(
        &[
            ("siteA", "Site A", &site_a_url, &["modelX"]),
            ("siteB", "Site B", &site_b_url, &["modelY"]),
        ],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
        AggregateNamingMode::default(),
        Vec::new(),
        false,
        &[],
    );
    let (status, bytes) = send_request(&gateway.url, "siteB.modelY").await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["model"], "modelY");
    assert_eq!(response["output"][0]["content"][0]["text"], "from B");

    assert_eq!(abort_if_idle(site_a_task).await, None);
    let captured_b = tokio::time::timeout(REQUEST_TIMEOUT, site_b_task)
        .await
        .expect("site B request should arrive")
        .unwrap()
        .expect("site B request should be captured");
    assert_eq!(captured_b["model"], "modelY");
}

#[tokio::test]
async fn aggregate_model_not_found_on_site_a_fails_over_to_site_b_with_same_model() {
    let site_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a.local_addr().unwrap());
    let site_b_url = format!("http://{}", site_b.local_addr().unwrap());
    let site_a_task = spawn_upstream_capture(site_a, 404, model_not_found_response("modelX"));
    let site_b_task =
        spawn_upstream_capture(site_b, 200, responses_success("modelX", "fallback B"));

    // 跨站故障转移开启：这是它的历史契约，站点 A 失败时站点 B 顶上。
    let gateway = RunningAggregateGateway::new_with_slug_table(
        &[
            ("siteA", "Site A", &site_a_url, &["modelX"]),
            ("siteB", "Site B", &site_b_url, &["modelX"]),
        ],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
        AggregateNamingMode::default(),
        Vec::new(),
        true,
        &[],
    );
    let (status, bytes) = send_request(&gateway.url, "siteA.modelX").await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["model"], "modelX");
    assert_eq!(response["output"][0]["content"][0]["text"], "fallback B");

    let captured_a = tokio::time::timeout(REQUEST_TIMEOUT, site_a_task)
        .await
        .expect("site A request should arrive")
        .unwrap()
        .expect("site A request should be captured");
    assert_eq!(captured_a["model"], "modelX");
    let captured_b = tokio::time::timeout(REQUEST_TIMEOUT, site_b_task)
        .await
        .expect("site B fallback request should arrive")
        .unwrap()
        .expect("site B request should be captured");
    assert_eq!(captured_b["model"], "modelX");
}

#[tokio::test]
async fn aggregate_unknown_site_prefix_returns_404_without_calling_any_upstream() {
    let site_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a.local_addr().unwrap());
    let site_b_url = format!("http://{}", site_b.local_addr().unwrap());
    let site_a_task = spawn_upstream_capture(
        site_a,
        500,
        responses_success("unknown.modelX", "unexpected A"),
    );
    let site_b_task = spawn_upstream_capture(
        site_b,
        500,
        responses_success("unknown.modelX", "unexpected B"),
    );

    let gateway = RunningAggregateGateway::new(
        &[
            ("siteA", "Site A", &site_a_url, &["modelX"]),
            ("siteB", "Site B", &site_b_url, &["modelX"]),
        ],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
    );
    let (status, bytes) = send_request(&gateway.url, "unknown.modelX").await;
    assert_eq!(status, 404, "{}", String::from_utf8_lossy(&bytes));

    assert_eq!(abort_if_idle(site_a_task).await, None);
    assert_eq!(abort_if_idle(site_b_task).await, None);
}

#[tokio::test]
async fn aggregate_replays_the_persisted_table_when_a_site_is_no_longer_enabled() {
    let site_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b_url = format!("http://{}", site_b.local_addr().unwrap());
    let site_b_task = spawn_upstream_capture(site_b, 200, responses_success("modelX", "from B"));

    // The manifest was engaged with siteA + siteB under `model_only`, so it
    // published modelX (siteA) and modelX#2 (siteB). siteA is no longer an
    // enabled candidate: rebuilding the table from the live candidates would
    // renumber modelX#2 down onto siteB, so a slug the user already had in
    // their Codex list would change meaning. The persisted table wins instead.
    let gateway = RunningAggregateGateway::new_with_slug_table(
        &[("siteB", "Site B", &site_b_url, &["modelX"])],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
        AggregateNamingMode::ModelOnly,
        vec![
            AggregateSlugEntry {
                site_id: "siteA".to_string(),
                upstream_model: "modelX".to_string(),
                slug: "modelX".to_string(),
            },
            AggregateSlugEntry {
                site_id: "siteB".to_string(),
                upstream_model: "modelX".to_string(),
                slug: "modelX#2".to_string(),
            },
        ],
        true,
        &[],
    );

    let (status, bytes) = send_request(&gateway.url, "modelX#2").await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["output"][0]["content"][0]["text"], "from B");

    let captured = tokio::time::timeout(REQUEST_TIMEOUT, site_b_task)
        .await
        .expect("site B request should arrive")
        .unwrap()
        .expect("site B request should be captured");
    // The slug resolved back to the upstream model id before forwarding.
    assert_eq!(captured["model"], "modelX");
}

/// The persisted table still names siteA for `modelX`; with the gate **off** that
/// slug has no live site behind it, so the request must 404 instead of being
/// handed to siteB just because it declares the same model.
#[tokio::test]
async fn aggregate_gate_off_serves_a_disabled_sites_slug_only_from_the_named_site() {
    let site_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b_url = format!("http://{}", site_b.local_addr().unwrap());
    let site_b_task = spawn_upstream_capture(site_b, 200, responses_success("modelX", "from B"));

    let gateway = RunningAggregateGateway::new_with_slug_table(
        &[("siteB", "Site B", &site_b_url, &["modelX"])],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
        AggregateNamingMode::ModelOnly,
        vec![
            AggregateSlugEntry {
                site_id: "siteA".to_string(),
                upstream_model: "modelX".to_string(),
                slug: "modelX".to_string(),
            },
            AggregateSlugEntry {
                site_id: "siteB".to_string(),
                upstream_model: "modelX".to_string(),
                slug: "modelX#2".to_string(),
            },
        ],
        false,
        &[],
    );

    let (status, bytes) = send_request(&gateway.url, "modelX").await;
    assert_eq!(status, 404, "{}", String::from_utf8_lossy(&bytes));
    // With the gate off the unnamed-model branch refuses instead of silently
    // picking a site, so the body is the "no enabled site declares" envelope.
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["error"], "gateway_aggregate_model_unknown");

    assert_eq!(abort_if_idle(site_b_task).await, None);
}

// ---- 跨站故障转移开关 (cross_site_failover) ---------------------------------

#[tokio::test]
async fn aggregate_gate_off_keeps_a_failing_site_a_request_on_site_a() {
    let site_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a.local_addr().unwrap());
    let site_a_task = spawn_upstream_capture(site_a, 404, model_not_found_response("modelX"));

    // Site B must not receive *any* request, not merely "no request within one
    // short window": it runs for the whole test and counts every connection.
    let site_b_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b_url = format!("http://{}", site_b_listener.local_addr().unwrap());
    let (site_b_hits, site_b_counter) = spawn_upstream_counter(site_b_listener);

    let gateway = RunningAggregateGateway::new_with_slug_table(
        &[
            ("siteA", "Site A", &site_a_url, &["modelX"]),
            ("siteB", "Site B", &site_b_url, &["modelX"]),
        ],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
        AggregateNamingMode::default(),
        Vec::new(),
        false,
        &[],
    );

    let (status, bytes) = send_request(&gateway.url, "siteA.modelX").await;
    // Site A's own failure surfaces verbatim; no cross-site attempt happens, so
    // another site's balance is never spent on this request.
    assert_eq!(status, 404, "{}", String::from_utf8_lossy(&bytes));
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["error"]["code"], "model_not_found");
    assert_eq!(response["error"]["message"], "model modelX not found");

    let captured_a = tokio::time::timeout(REQUEST_TIMEOUT, site_a_task)
        .await
        .expect("site A request should arrive")
        .unwrap()
        .expect("site A request should be captured");
    assert_eq!(captured_a["model"], "modelX");

    settle_cross_site_window().await;
    assert_eq!(
        site_b_hits.load(Ordering::SeqCst),
        0,
        "site B must never be contacted while the cross-site gate is off"
    );
    site_b_counter.abort();
}

#[tokio::test]
async fn repeated_stream_protocol_errors_do_not_cool_the_named_model_site() {
    let site_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a.local_addr().unwrap());
    let protocol_errors_to_reach_threshold = ProxyGatewaySettings::default()
        .model_failure_score_threshold
        .max(1) as usize;
    let site_a_task = tokio::spawn(async move {
        let mut captured_requests = Vec::with_capacity(protocol_errors_to_reach_threshold + 1);
        for attempt in 0..=protocol_errors_to_reach_threshold {
            let (mut socket, _) = tokio::time::timeout(REQUEST_TIMEOUT, site_a.accept())
                .await
                .expect("the named site should keep receiving requests")
                .expect("upstream listener should accept");
            captured_requests.push(read_http_json(&mut socket).await);
            if attempt < protocol_errors_to_reach_threshold {
                write_http_sse(
                    &mut socket,
                    b"event: response.failed\n\
                      data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_failed\",\"status\":\"failed\",\"error\":{\"code\":\"server_error\",\"message\":\"failed upstream\"}}}\n\n",
                )
                .await;
            } else {
                write_http_sse(
                    &mut socket,
                    b"event: response.output_text.delta\n\
                      data: {\"type\":\"response.output_text.delta\",\"delta\":\"recovered\",\"item_id\":\"msg_fixture\",\"output_index\":0}\n\n\
                      event: response.completed\n\
                      data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_modelX\",\"status\":\"completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"total_tokens\":5}}}\n\n",
                )
                .await;
            }
        }
        captured_requests
    });

    let gateway = RunningAggregateGateway::new_with_slug_table(
        &[("siteA", "Site A", &site_a_url, &["modelX"])],
        GatewayProxyMode::Aggregate,
        &["siteA"],
        AggregateNamingMode::default(),
        Vec::new(),
        false,
        &[],
    );

    for attempt in 0..protocol_errors_to_reach_threshold {
        let (status, bytes) = send_streaming_request(&gateway.url, "siteA.modelX").await;
        if status != reqwest::StatusCode::BAD_GATEWAY {
            site_a_task.abort();
            panic!(
                "protocol envelope attempt {} should preserve the 502 response, got {status}: {}",
                attempt + 1,
                String::from_utf8_lossy(&bytes)
            );
        }
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["error"], "upstream_stream_first_chunk_failed");
        assert_eq!(
            response["message"],
            "Upstream streaming response reported a protocol error envelope"
        );
    }

    // Each response.failed contributes one point to the default model health
    // threshold; despite reaching it, the next request must still visit site A.
    let (status, bytes) = send_streaming_request(&gateway.url, "siteA.modelX").await;
    if status != reqwest::StatusCode::OK {
        site_a_task.abort();
        panic!(
            "the named site should remain available after protocol errors, got {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("response.completed"), "{body}");
    assert!(body.contains("recovered"), "{body}");

    let captured_requests = tokio::time::timeout(REQUEST_TIMEOUT, site_a_task)
        .await
        .expect("all requests should reach site A")
        .unwrap();
    assert_eq!(
        captured_requests.len(),
        protocol_errors_to_reach_threshold + 1
    );
    for captured in captured_requests {
        assert_eq!(captured["model"], "modelX");
        assert_eq!(captured["stream"], true);
        assert_eq!(
            captured["input"][0]["content"], "say hello",
            "the upstream request body must remain intact"
        );
    }
}

/// Gate fully off + the named site is cooling down: the request is refused with
/// a 503 rather than quietly served by the other site.
#[tokio::test]
async fn aggregate_gate_off_reports_a_cooling_site_without_crossing_over() {
    let site_a_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_b_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a_listener.local_addr().unwrap());
    let site_b_url = format!("http://{}", site_b_listener.local_addr().unwrap());
    // Neither listener may be contacted: the only permitted site is cooling.
    let (site_a_hits, site_a_counter) = spawn_upstream_counter(site_a_listener);
    let (site_b_hits, site_b_counter) = spawn_upstream_counter(site_b_listener);

    let gateway = RunningAggregateGateway::new_with_slug_table(
        &[
            ("siteA", "Site A", &site_a_url, &["modelX"]),
            ("siteB", "Site B", &site_b_url, &["modelX"]),
        ],
        GatewayProxyMode::Aggregate,
        &["siteA", "siteB"],
        AggregateNamingMode::default(),
        Vec::new(),
        false,
        &["siteA"],
    );

    let (status, bytes) = send_request(&gateway.url, "siteA.modelX").await;
    assert_eq!(status, 503, "{}", String::from_utf8_lossy(&bytes));
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    // `error_category: cooling_down` is not part of the wire body, so the
    // observable contract is this envelope plus the skipped site list.
    assert_eq!(response["error"], "model_temporarily_unavailable");
    assert_eq!(response["skipped_providers"], json!(["Site A"]));

    settle_cross_site_window().await;
    assert_eq!(site_a_hits.load(Ordering::SeqCst), 0);
    assert_eq!(site_b_hits.load(Ordering::SeqCst), 0);
    site_a_counter.abort();
    site_b_counter.abort();
}

#[tokio::test]
async fn single_mode_keeps_bare_model_routing_and_does_not_regress() {
    let site_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let site_a_url = format!("http://{}", site_a.local_addr().unwrap());
    let site_a_task = spawn_upstream_capture(site_a, 200, responses_success("modelX", "single"));

    let gateway = RunningAggregateGateway::new(
        &[("siteA", "Site A", &site_a_url, &["modelX"])],
        GatewayProxyMode::Single,
        &["siteA"],
    );
    let (status, bytes) = send_request(&gateway.url, "modelX").await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["model"], "modelX");
    assert_eq!(response["output"][0]["content"][0]["text"], "single");

    let captured = tokio::time::timeout(REQUEST_TIMEOUT, site_a_task)
        .await
        .expect("single-mode upstream request should arrive")
        .unwrap()
        .expect("single-mode upstream request should be captured");
    assert_eq!(captured["model"], "modelX");
}
