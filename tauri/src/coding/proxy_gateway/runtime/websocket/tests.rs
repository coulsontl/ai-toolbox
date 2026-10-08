use super::*;
use crate::coding::proxy_gateway::paths::ProxyGatewayPaths;
use crate::coding::proxy_gateway::types::GatewayRequestLogFilters;
use crate::coding::proxy_gateway::{request_log, usage_stats};
use crate::db::helpers::{db_create, db_put};
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;
use tokio::net::TcpListener;

#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;

#[path = "settings_tests.rs"]
mod settings_tests;

#[path = "privacy_tests.rs"]
mod privacy_tests;

#[path = "privacy_matrix_tests.rs"]
mod privacy_matrix_tests;

fn test_context(
    upstream_url: &str,
    protocol: &str,
    record_body: bool,
) -> (tempfile::TempDir, GatewayRuntimeContext, String) {
    test_context_for_cli(GatewayCliKey::Codex, upstream_url, protocol, record_body)
}

fn test_context_for_cli(
    cli_key: GatewayCliKey,
    upstream_url: &str,
    protocol: &str,
    record_body: bool,
) -> (tempfile::TempDir, GatewayRuntimeContext, String) {
    let directory = tempfile::tempdir().unwrap();
    let db = SqliteDbState::in_memory_for_test().unwrap();
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::Settings,
            "app",
            &json!({"proxy_mode":"direct"}),
        )
    })
    .unwrap();
    let (table, provider_settings) = match cli_key {
        GatewayCliKey::Claude | GatewayCliKey::ClaudeDesktop => (
            if cli_key == GatewayCliKey::Claude {
                DbTable::ClaudeProvider
            } else {
                DbTable::ClaudeDesktopProvider
            },
            json!({"env":{"ANTHROPIC_BASE_URL":upstream_url,"ANTHROPIC_API_KEY":"upstream-test-key","ANTHROPIC_AUTH_TOKEN":"upstream-test-key"}}),
        ),
        GatewayCliKey::Codex => (
            DbTable::CodexProvider,
            json!({"auth":{"OPENAI_API_KEY":"upstream-test-key"}, "config":format!("model_provider = \"custom\"\nmodel = \"test-model\"\n[model_providers.custom]\nbase_url = \"{upstream_url}\"\nwire_api = \"responses\"\nsupports_websockets = true\n")}),
        ),
        GatewayCliKey::Grok => (
            DbTable::GrokProvider,
            json!({"auth":{"API_KEY":"upstream-test-key"},"config":format!("[models]\ndefault = \"custom\"\n[model.custom]\nmodel = \"test-model\"\nbase_url = \"{upstream_url}\"\napi_backend = \"responses\"\n")}),
        ),
        GatewayCliKey::Kimi => (
            DbTable::KimiProvider,
            json!({"auth":{"API_KEY":"upstream-test-key"},"providerConfigs":{"custom":{"type":"openai","base_url":upstream_url}},"defaultModelKey":"test-model","modelCatalog":{"models":[{"key":"test-model","model":"test-model","provider":"custom"}]}}),
        ),
        GatewayCliKey::Gemini => (
            DbTable::GeminiCliProvider,
            json!({"env":{"GOOGLE_GEMINI_BASE_URL":upstream_url,"GEMINI_API_KEY":"upstream-test-key"}}),
        ),
        GatewayCliKey::Antigravity => (
            DbTable::AntigravityProvider,
            json!({"env":{"GOOGLE_GEMINI_BASE_URL":upstream_url,"GEMINI_API_KEY":"upstream-test-key"}}),
        ),
        GatewayCliKey::OpenCode => panic!("OpenCode has no gateway takeover route"),
    };
    let provider = db
        .with_conn(|conn| {
            db_create(
                conn,
                table,
                &json!({
                    "name":"WebSocket test", "category":"custom", "is_applied":true,
                    "settings_config":provider_settings.to_string(),
                    "meta":{"apiFormat":protocol}
                }),
            )
        })
        .unwrap();
    let provider_id = provider["id"].as_str().unwrap().to_string();
    let settings = ProxyGatewaySettings {
        codex_websocket_enabled: true,
        request_log_enabled: true,
        metrics_enabled: true,
        store_headers: true,
        store_request_body: record_body,
        store_response_body: record_body,
        log_max_body_size_kb: 1,
        log_retention_days: 0,
        retry_interval_secs: 0,
        max_retry_count: 0,
        per_provider_retry_count: 0,
        ..Default::default()
    };
    let context = GatewayRuntimeContext::new(
        settings,
        Some(db),
        Some(ProxyGatewayPaths::new(directory.path())),
    );
    (directory, context, provider_id)
}

async fn gateway_connection(
    context: GatewayRuntimeContext,
) -> (WebSocketStream<TcpStream>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        super::super::handle_connection(&mut socket, &context)
            .await
            .unwrap();
    });
    let socket = TcpStream::connect(address).await.unwrap();
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(format!(
            "ws://{address}/openai/v1/responses"
        ))
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer client-test-key"),
    );
    let (websocket, response) = tokio_tungstenite::client_async(request, socket)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    (websocket, task)
}

async fn receive_json<S: AsyncRead + AsyncWrite + Unpin>(socket: &mut WebSocketStream<S>) -> Value {
    loop {
        let message = timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

async fn send_json<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    value: Value,
) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn start_gateway(
    context: GatewayRuntimeContext,
    connections: usize,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        for _ in 0..connections {
            let (mut socket, _) = listener.accept().await.unwrap();
            super::super::handle_connection(&mut socket, &context)
                .await
                .unwrap();
        }
    });
    (address, task)
}

#[tokio::test]
async fn aggregate_websocket_upgrade_returns_local_426_without_upstream_connection() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (directory, context, provider_id) = test_context(
        &format!("http://{}/v1", upstream.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let paths = ProxyGatewayPaths::new(directory.path());
    let manifest_path = paths.manifest_path(GatewayCliKey::Codex);
    std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "managed_by": "ai-toolbox-test",
            "cli_key": "codex",
            "enabled": true,
            "mode": "aggregate",
            "primary_provider_id": provider_id,
            "base_origin": "http://127.0.0.1:37123",
            "created_at": "2026-09-27T00:00:00Z",
            "updated_at": "2026-09-27T00:00:00Z",
            "files": [],
            "aggregate": {"provider_ids": [provider_id], "cross_site_failover": false}
        }))
        .unwrap(),
    )
    .unwrap();
    super::super::providers::clear_gateway_provider_selection_cache();

    let (address, gateway) = start_gateway(context.clone(), 1).await;
    let response = rejected_upgrade(address, "/openai/v1/responses").await;
    assert_eq!(response.status(), 426);
    assert!(
        String::from_utf8_lossy(response.body().as_deref().unwrap_or_default())
            .contains("aggregate mode uses HTTP/SSE")
    );
    gateway.await.unwrap();
    assert!(timeout(Duration::from_millis(50), upstream.accept())
        .await
        .is_err());
    let details = recorded_details(&context);
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].summary.status_code, Some(426));
    assert_eq!(context.requests_per_minute(), 0);
    drop(directory);
}

async fn rejected_upgrade(
    address: std::net::SocketAddr,
    path: &str,
) -> http::Response<Option<Vec<u8>>> {
    let socket = TcpStream::connect(address).await.unwrap();
    let error = tokio_tungstenite::client_async(format!("ws://{address}{path}"), socket)
        .await
        .unwrap_err();
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => *response,
        error => panic!("unexpected handshake error: {error}"),
    }
}

fn recorded_details(
    context: &GatewayRuntimeContext,
) -> Vec<crate::coding::proxy_gateway::types::GatewayRequestLogDetail> {
    let logs = usage_stats::request_logs(
        context.db.as_ref().unwrap(),
        &GatewayRequestLogFilters::default(),
        0,
        100,
        true,
    )
    .unwrap();
    logs.data
        .iter()
        .map(|entry| {
            request_log::get_request_log_detail(context.paths.as_ref().unwrap(), &entry.trace_id)
                .unwrap()
                .unwrap()
        })
        .collect()
}

#[tokio::test]
async fn conversion_upgrade_returns_426_then_http_conversion_still_works() {
    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", upstream_listener.local_addr().unwrap()),
        "openai_chat",
        true,
    );
    let (address, gateway) = start_gateway(context.clone(), 2).await;
    let response = rejected_upgrade(address, "/openai/v1/responses").await;
    assert_eq!(response.status(), 426);
    // A protocol-incompatible provider must not receive an upgrade probe.
    assert!(
        timeout(Duration::from_millis(50), upstream_listener.accept())
            .await
            .is_err()
    );
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = upstream_listener.accept().await.unwrap();
        let request = super::super::http_io::read_http_request(&mut socket, 0)
            .await
            .unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/chat/completions");
        assert!(serde_json::from_slice::<Value>(&request.body)
            .unwrap()
            .get("messages")
            .is_some());
        let body = json!({"id":"chatcmpl-http-fallback","model":"test-model","choices":[{"message":{"role":"assistant","content":"HTTP works"},"finish_reason":"stop","index":0}],"usage":{"prompt_tokens":12,"completion_tokens":3}}).to_string();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    let response = crate::http_client::create_client_no_proxy(5)
        .unwrap()
        .post(format!("http://{address}/openai/v1/responses"))
        .json(&json!({"model":"test-model","input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["object"], "response");
    gateway.await.unwrap();
    upstream.await.unwrap();
    assert_eq!(context.requests_per_minute(), 1);
    let summary =
        usage_stats::usage_summary(context.db.as_ref().unwrap(), None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 1);
    assert_eq!(summary.total_tokens, 15);
    assert_eq!(summary.success_rate, 100.0);
    let details = recorded_details(&context);
    assert_eq!(details.len(), 2);
    let fallback = details
        .iter()
        .find(|detail| detail.summary.request_kind == GatewayRequestKind::WebsocketHandshake)
        .unwrap();
    assert_eq!(fallback.summary.status_code, Some(426));
    assert!(fallback
        .websocket
        .as_ref()
        .unwrap()
        .fallback_reason
        .is_some());
    assert_eq!(
        usage_stats::request_logs(
            context.db.as_ref().unwrap(),
            &GatewayRequestLogFilters {
                only_failed: Some(true),
                ..Default::default()
            },
            0,
            10,
            true
        )
        .unwrap()
        .total,
        0
    );
}

#[tokio::test]
async fn unsupported_routes_and_explicit_opt_out_fall_back_without_connecting() {
    for (protocol, path, disabled) in [
        ("anthropic_messages", "/openai/v1/responses", false),
        ("gemini_native", "/openai/v1/responses", false),
        ("openai_responses", "/openai/v1/responses/compact", false),
        ("openai_responses", "/grok/v1/responses", false),
        ("openai_responses", "/openai/v1/responses", true),
    ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let (_directory, context, _) = test_context(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            protocol,
            true,
        );
        if disabled {
            let db = context.db.as_ref().unwrap();
            let mut candidates = context
                .load_candidate_providers(db, GatewayCliKey::Codex)
                .await
                .unwrap();
            candidates.providers[0].supports_websockets = Some(false);
            // Replace the cached provider only; the gateway still resolves its normal candidate path.
            context
                .provider_cache
                .lock()
                .unwrap()
                .get_mut(&GatewayCliKey::Codex)
                .unwrap()
                .providers = candidates.providers;
        }
        let (address, gateway) = start_gateway(context.clone(), 1).await;
        assert_eq!(rejected_upgrade(address, path).await.status(), 426);
        gateway.await.unwrap();
        assert!(timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err());
        assert_eq!(context.requests_per_minute(), 0);
    }
}

#[tokio::test]
async fn upstream_handshake_preserves_auth_rate_limit_errors_and_falls_back_on_unsupported() {
    for (upstream_status, downstream_status) in [
        (404, 426),
        (200, 426),
        (302, 426),
        (401, 401),
        (429, 429),
        (503, 503),
    ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let (_directory, context, _) = test_context(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            "openai_responses",
            true,
        );
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = super::super::http_io::read_http_request(&mut socket, 0)
                .await
                .unwrap();
            assert_eq!(request.method, "GET");
            socket.write_all(format!("HTTP/1.1 {upstream_status} Test\r\nContent-Length: 0\r\nRetry-After: 4\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let (address, gateway) = start_gateway(context.clone(), 1).await;
        let response = rejected_upgrade(address, "/openai/v1/responses").await;
        assert_eq!(response.status(), downstream_status);
        assert_eq!(response.headers()["retry-after"], "4");
        gateway.await.unwrap();
        upstream.await.unwrap();
        assert_eq!(context.requests_per_minute(), 0);
        assert_eq!(
            usage_stats::usage_summary(context.db.as_ref().unwrap(), None, None, None, true)
                .unwrap()
                .total_requests,
            0
        );
        let detail = recorded_details(&context).pop().unwrap();
        assert_eq!(
            detail.websocket.unwrap().upstream_handshake_status,
            Some(upstream_status)
        );
    }
}

/// The WebSocket transport has no non-streamed mode, so a compaction turn is
/// summarized by consuming the upstream's own event stream and replacing it with
/// a synthesized `compaction` item. Codex must see exactly one compaction item
/// and one terminal event, and the request must reach upstream as a plain
/// summarization turn rather than a private `compaction_trigger`.
#[tokio::test]
async fn websocket_compaction_turn_is_summarized_and_replaced() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let request = receive_json(&mut socket).await;
        // The private control item must not reach the upstream, and the tool
        // inventory is irrelevant to summarizing.
        let input = request["input"].as_array().cloned().unwrap_or_default();
        assert!(
            !input.iter().any(|item| item["type"] == "compaction_trigger"),
            "the compaction trigger leaked upstream: {request}"
        );
        assert!(request.get("tools").is_none(), "tools leaked upstream");
        assert!(
            request["instructions"]
                .as_str()
                .is_some_and(|text| text.contains("continuation summary")),
            "base instructions were not replaced: {request}"
        );
        assert!(
            input
                .last()
                .is_some_and(|item| item["content"][0]["text"] == "Produce the continuation summary now."),
            "the follow-up summarization turn is missing: {request}"
        );

        send_json(&mut socket, json!({"type":"response.output_text.delta","delta":"Goal: ship "})).await;
        send_json(&mut socket, json!({"type":"response.output_text.delta","delta":"compaction v2"})).await;
        send_json(&mut socket, json!({"type":"response.completed","response":{"id":"upstream-response","usage":{"input_tokens":11,"output_tokens":4}}})).await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "instructions": "You are a coding agent.",
            "tools": [{ "type": "function", "name": "shell" }],
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "fix the bug" }] },
                { "type": "compaction_trigger" },
            ],
        }),
    )
    .await;

    let mut frames = Vec::new();
    loop {
        let frame = receive_json(&mut client).await;
        let terminal = frame["type"] == "response.completed";
        frames.push(frame);
        if terminal {
            break;
        }
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    // The upstream's own deltas must not reach the client: the summary is only
    // ever delivered sealed inside the compaction item.
    assert!(
        !frames
            .iter()
            .any(|frame| frame.to_string().contains("Goal: ship")),
        "the unsealed summary leaked to the client: {frames:?}",
    );

    // Exactly one compaction item, delivered by the terminal event.
    let items: Vec<&Value> = frames
        .iter()
        .filter_map(|frame| (frame["type"] == "response.output_item.done").then(|| &frame["item"]))
        .collect();
    assert_eq!(items.len(), 1, "expected exactly one done item: {frames:?}");
    assert_eq!(items[0]["type"], "compaction");
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == "response.completed")
            .count(),
        1,
        "the upstream terminal must not be forwarded alongside the synthesized one",
    );

    // The summary is sealed into the capsule, and the upstream's own usage and
    // model survive the rewrite.
    let completed = frames.last().unwrap();
    assert_eq!(completed["response"]["model"], "test-model");
    assert_eq!(completed["response"]["usage"]["input_tokens"], 11);
    assert_eq!(completed["response"]["usage"]["output_tokens"], 4);
    let sealed = items[0]["encrypted_content"].as_str().unwrap();
    let summary = super::super::compat::codex_remote_compaction::open_compaction_summary(sealed)
        .expect("the item must carry a capsule this Gateway can read back");
    assert_eq!(summary, "Goal: ship compaction v2");
    assert!(
        !sealed.contains("Goal: ship"),
        "the summary must not travel in plaintext: {sealed}"
    );

    // The turn is billed like any other request.
    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(detail.summary.total_attempt_count, 1);
}

/// Codex may send the compaction turn as an incremental delta: only the items
/// since the last response, plus the trigger, with `previous_response_id`
/// carrying the rest of the conversation.
///
/// The Gateway must preserve Codex's own full-vs-delta decision — dropping
/// `previous_response_id` would leave the upstream with nothing to summarize,
/// while forcing the full history through would duplicate it.
#[tokio::test]
async fn websocket_compaction_turn_keeps_an_incremental_delta() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["previous_response_id"], "resp_prev");
        let input = request["input"].as_array().cloned().unwrap_or_default();
        assert!(
            !input.iter().any(|item| item["type"] == "compaction_trigger"),
            "the compaction trigger leaked upstream: {request}"
        );
        // The delta's own items survive; only the trigger is replaced.
        assert_eq!(input[0]["content"][0]["text"], "and then fix the tests");
        assert_eq!(
            input.last().unwrap()["content"][0]["text"],
            "Produce the continuation summary now."
        );

        send_json(&mut socket, json!({"type":"response.output_text.delta","delta":"delta summary"})).await;
        send_json(&mut socket, json!({"type":"response.completed","response":{"id":"upstream-response","usage":{"input_tokens":3,"output_tokens":2}}})).await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "previous_response_id": "resp_prev",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "and then fix the tests" }] },
                { "type": "compaction_trigger" },
            ],
        }),
    )
    .await;

    loop {
        if receive_json(&mut client).await["type"] == "response.completed" {
            break;
        }
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();
}

/// A failed summarization must reach Codex as the upstream's own error.
///
/// Codex classifies `response.failed` by its error code and retries the
/// retryable ones; it also installs a returned item as the whole conversation
/// history. Synthesizing a successful item here would both discard the real
/// context and swallow the retry.
#[tokio::test]
async fn websocket_compaction_failure_is_forwarded_not_synthesized() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        receive_json(&mut socket).await;
        // Summarization fails after partial output: the deltas must not leak,
        // and the error must survive.
        send_json(&mut socket, json!({"type":"response.output_text.delta","delta":"partial summary that must not leak"})).await;
        send_json(&mut socket, json!({
            "type": "response.failed",
            "response": {
                "id": "upstream-failed",
                "status": "failed",
                "error": { "code": "rate_limit_exceeded", "message": "Rate limit reached" },
            },
        }))
        .await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "fix the bug" }] },
                { "type": "compaction_trigger" },
            ],
        }),
    )
    .await;

    let mut frames = Vec::new();
    loop {
        let frame = receive_json(&mut client).await;
        let terminal = matches!(
            frame["type"].as_str(),
            Some("response.failed" | "response.completed")
        );
        frames.push(frame);
        if terminal {
            break;
        }
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    assert_eq!(frames.len(), 1, "only the upstream terminal may be delivered: {frames:?}");
    assert_eq!(frames[0]["type"], "response.failed");
    assert_eq!(
        frames[0]["response"]["error"]["code"],
        "rate_limit_exceeded",
        "Codex needs the real code to classify and retry the failure",
    );
    assert!(
        !frames
            .iter()
            .any(|frame| frame["type"] == "response.output_item.done"),
        "a failed summary must not produce a compaction item: {frames:?}",
    );
    assert!(
        !frames
            .iter()
            .any(|frame| frame.to_string().contains("must not leak")),
        "the unsealed summary leaked to the client: {frames:?}",
    );
}

/// An upstream that *succeeds* without producing a summary must not be papered
/// over into a compaction item either.
///
/// `build_websocket_compaction_response` returns `None` here, and the relay has
/// to fall back to the upstream's own terminal rather than emit a half-built
/// turn. Codex then raises its own "expected exactly one compaction output
/// item" error against a completed response instead of installing a fabricated
/// context as the whole history.
#[tokio::test]
async fn websocket_empty_summary_forwards_the_upstream_terminal() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        receive_json(&mut socket).await;
        send_json(&mut socket, json!({
            "type": "response.output_item.done",
            "item": {
                "id": "upstream-empty", "type": "message", "role": "assistant",
                "content": [{ "type": "output_text", "text": "" }],
            },
        }))
        .await;
        send_json(&mut socket, json!({
            "type": "response.completed",
            "response": {
                "id": "upstream-empty", "status": "completed",
                "output": [{ "type": "message", "role": "assistant",
                    "content": [{ "type": "output_text", "text": "" }] }],
                "usage": { "input_tokens": 3, "output_tokens": 0 },
            },
        }))
        .await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "fix the bug" }] },
                { "type": "compaction_trigger" },
            ],
        }),
    )
    .await;

    let mut frames = Vec::new();
    loop {
        let frame = receive_json(&mut client).await;
        let terminal = matches!(
            frame["type"].as_str(),
            Some("response.completed" | "response.failed" | "response.incomplete")
        );
        frames.push(frame);
        if terminal {
            break;
        }
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    assert_eq!(
        frames.len(),
        1,
        "only the upstream terminal may be delivered: {frames:?}"
    );
    assert_eq!(
        frames[0]["response"]["id"], "upstream-empty",
        "the upstream's own terminal must survive: {frames:?}",
    );
    assert!(
        !frames.iter().any(|frame| frame["item"]["type"] == "compaction"
            || frame["response"]["output"]
                .as_array()
                .is_some_and(|output| output.iter().any(|item| item["type"] == "compaction"))),
        "an empty summary must not produce a compaction item: {frames:?}",
    );
    assert!(
        !frames
            .iter()
            .any(|frame| frame.to_string().contains("ai-toolbox-compact-v1:")),
        "a capsule was sealed from nothing: {frames:?}",
    );
}

/// The same fallback for a truncated summary: the upstream's `incomplete`
/// terminal is the truth, and the partial text must not be installed as context.
#[tokio::test]
async fn websocket_truncated_summary_forwards_the_upstream_terminal() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        receive_json(&mut socket).await;
        // A half-written summary that must never reach the client.
        send_json(
            &mut socket,
            json!({"type":"response.output_text.delta","delta":"half a summary, cut off mid-"}),
        )
        .await;
        send_json(
            &mut socket,
            json!({"type":"response.incomplete","response":{"id":"upstream-incomplete","status":"incomplete"}}),
        )
        .await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "fix the bug" }] },
                { "type": "compaction_trigger" },
            ],
        }),
    )
    .await;

    let mut frames = Vec::new();
    loop {
        let frame = receive_json(&mut client).await;
        let terminal = matches!(
            frame["type"].as_str(),
            Some("response.completed" | "response.failed" | "response.incomplete")
        );
        frames.push(frame);
        if terminal {
            break;
        }
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    assert_eq!(
        frames.len(),
        1,
        "only the upstream terminal may be delivered: {frames:?}"
    );
    assert_eq!(frames[0]["type"], "response.incomplete");
    assert_eq!(frames[0]["response"]["id"], "upstream-incomplete");
    assert!(
        !frames
            .iter()
            .any(|frame| frame["item"]["type"] == "compaction"),
        "a truncated summary must not produce a compaction item: {frames:?}",
    );
    assert!(
        !frames
            .iter()
            .any(|frame| frame.to_string().contains("half a summary")),
        "the partial summary leaked to the client: {frames:?}",
    );
}

#[tokio::test]
async fn handshake_retry_is_recorded_separately_from_turn_attempts() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    context.settings.write().unwrap().max_retry_count = 1;
    context.settings.write().unwrap().per_provider_retry_count = 1;
    let upstream = tokio::spawn(async move {
        let (mut rejected, _) = listener.accept().await.unwrap();
        super::super::http_io::read_http_request(&mut rejected, 0)
            .await
            .unwrap();
        rejected
            .write_all(
                b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        receive_json(&mut socket).await;
        send_json(&mut socket, json!({"type":"response.completed","response":{"id":"retry-response","usage":{"input_tokens":1,"output_tokens":2}}})).await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model","input":"hello"}),
    )
    .await;
    receive_json(&mut client).await;
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();
    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(detail.summary.total_attempt_count, 1);
    assert!(!detail.summary.failover);
    assert_eq!(detail.provider_attempts.len(), 1);
    let attempts = detail.websocket.unwrap().handshake_attempts;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status_code, Some(503));
    assert_eq!(attempts[1].status_code, Some(101));
    assert_eq!(context.requests_per_minute(), 1);
}

#[tokio::test]
async fn exhausted_handshake_budget_keeps_the_real_upstream_error() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let mut candidates = context
        .load_candidate_providers(context.db.as_ref().unwrap(), GatewayCliKey::Codex)
        .await
        .unwrap()
        .providers;
    let mut converted_provider = candidates[0].clone();
    converted_provider.id = "requires-conversion".into();
    converted_provider.target_protocol = AiProtocol::OpenAiChat;
    candidates.push(converted_provider);
    context
        .provider_cache
        .lock()
        .unwrap()
        .get_mut(&GatewayCliKey::Codex)
        .unwrap()
        .providers = candidates;
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        super::super::http_io::read_http_request(&mut socket, 0)
            .await
            .unwrap();
        socket
            .write_all(
                b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    });
    let (address, gateway) = start_gateway(context.clone(), 1).await;
    assert_eq!(
        rejected_upgrade(address, "/openai/v1/responses")
            .await
            .status(),
        503
    );
    gateway.await.unwrap();
    upstream.await.unwrap();
    assert!(recorded_details(&context)[0]
        .websocket
        .as_ref()
        .unwrap()
        .fallback_reason
        .is_none());
}

#[tokio::test]
async fn websocket_uses_configured_http_proxy_raw_url_and_client_beta() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        "http://upstream.invalid/custom?source=provider##",
        "openai_responses",
        true,
    );
    let db = context.db.as_ref().unwrap();
    db.with_conn(|conn| db_put(conn, DbTable::Settings, "app", &json!({"proxy_mode":"custom","proxy_url":format!("http://{}", listener.local_addr().unwrap())}))).unwrap();
    context
        .load_candidate_providers(db, GatewayCliKey::Codex)
        .await
        .unwrap();
    context
        .provider_cache
        .lock()
        .unwrap()
        .get_mut(&GatewayCliKey::Codex)
        .unwrap()
        .providers[0]
        .supports_websockets = None;
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            socket,
            |request: &server::Request, mut response: server::Response| {
                assert_eq!(request.uri().host(), Some("upstream.invalid"));
                assert_eq!(request.uri().path(), "/custom");
                let query = request.uri().query().unwrap();
                assert!(query.contains("source=provider"));
                assert!(query.contains("client=codex"));
                assert_eq!(
                    request.headers()["openai-beta"],
                    "responses_websockets=client-version"
                );
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer upstream-test-key"
                );
                response
                    .headers_mut()
                    .insert("x-request-id", HeaderValue::from_static("handshake-id"));
                Ok(response)
            },
        )
        .await
        .unwrap();
        receive_json(&mut socket).await;
        send_json(
            &mut socket,
            json!({"type":"response.completed","response":{"id":"via-proxy"}}),
        )
        .await;
        let _ = socket.next().await;
    });
    let (address, gateway) = start_gateway(context.clone(), 1).await;
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(format!(
            "ws://{address}/openai/v1/responses?client=codex"
        ))
        .unwrap();
    request.headers_mut().insert(
        "openai-beta",
        HeaderValue::from_static("responses_websockets=client-version"),
    );
    let (mut client, response) =
        tokio_tungstenite::client_async(request, TcpStream::connect(address).await.unwrap())
            .await
            .unwrap();
    assert_eq!(response.status(), 101);
    assert_eq!(response.headers()["x-request-id"], "handshake-id");
    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model","input":"hello"}),
    )
    .await;
    receive_json(&mut client).await;
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();
    assert_eq!(recorded_details(&context).len(), 1);
}

#[tokio::test]
async fn responses_websocket_records_each_turn_and_reuses_connection() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            socket,
            |request: &server::Request, response: server::Response| {
                assert_eq!(request.uri().path(), "/v1/responses");
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer upstream-test-key"
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        for turn in 0..2 {
            let request = receive_json(&mut socket).await;
            assert_eq!(request["type"], "response.create");
            assert!(request.get("stream").is_none());
            if turn == 1 {
                assert_eq!(request["previous_response_id"], "resp_0");
            }
            let response_id = format!("resp_{turn}");
            socket
                .send(Message::Text(
                    json!({"type":"response.created","response":{"id":response_id}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket.send(Message::Text(json!({"type":"response.output_text.delta","response_id":response_id,"delta":"Hello"}).to_string().into())).await.unwrap();
            socket.send(Message::Text(json!({"type":"response.completed","response":{"id":response_id,"status":"completed","usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":30}}}}).to_string().into())).await.unwrap();
        }
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    for turn in 0..2 {
        let mut request = json!({"type":"response.create","model":"test-model","reasoning":{"effort":"high"},"input":"hello"});
        if turn == 1 {
            request["previous_response_id"] = json!("resp_0");
        }
        client
            .send(Message::Text(request.to_string().into()))
            .await
            .unwrap();
        assert_eq!(receive_json(&mut client).await["type"], "response.created");
        assert_eq!(
            receive_json(&mut client).await["type"],
            "response.output_text.delta"
        );
        assert_eq!(
            receive_json(&mut client).await["type"],
            "response.completed"
        );
    }
    let _ = client.close(None).await;
    timeout(Duration::from_secs(5), gateway)
        .await
        .unwrap()
        .unwrap();
    upstream.await.unwrap();
    let db = context.db.as_ref().unwrap();
    let logs =
        usage_stats::request_logs(db, &GatewayRequestLogFilters::default(), 0, 10, true).unwrap();
    assert_eq!(logs.total, 2);
    assert_eq!(context.requests_per_minute(), 2);
    for entry in logs.data {
        assert_eq!(entry.transport, GatewayRequestTransport::Websocket);
        assert_eq!(entry.stream_outcome, Some(GatewayStreamOutcome::Completed));
        assert!(entry.success);
        assert_eq!(entry.input_tokens, 70);
        assert_eq!(entry.output_tokens, 20);
        assert_eq!(entry.cache_read_tokens, 30);
        assert_eq!(entry.reasoning_effort.as_deref(), Some("high"));
        assert!(entry
            .trace_id
            .starts_with(&format!("SESSION:codex:{provider_id}:resp_")));
        let detail =
            request_log::get_request_log_detail(context.paths.as_ref().unwrap(), &entry.trace_id)
                .unwrap()
                .unwrap();
        assert!(detail.request_body.unwrap().contains("response.create"));
        assert!(detail.response_body.unwrap().contains("response.completed"));
        assert_eq!(detail.summary.status_code, None);
        assert_eq!(detail.websocket.unwrap().handshake_status, 101);
        assert!(!format!("{:?}", detail.request_headers).contains("client-test-key"));
    }
    let failed = usage_stats::request_logs(
        db,
        &GatewayRequestLogFilters {
            only_failed: Some(true),
            ..Default::default()
        },
        0,
        10,
        true,
    )
    .unwrap();
    assert_eq!(failed.total, 0);
}
