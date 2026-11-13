use super::*;
use crate::coding::proxy_gateway::paths::ProxyGatewayPaths;
use crate::coding::proxy_gateway::types::GatewayRequestLogFilters;
use crate::coding::proxy_gateway::{request_log, usage_stats};
use crate::db::helpers::{db_create, db_get, db_put};
use crate::db::schema::{Cli, DbTable};
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
                DbTable::CliProvider(Cli::Claude)
            } else {
                DbTable::CliProvider(Cli::ClaudeDesktop)
            },
            json!({"env":{"ANTHROPIC_BASE_URL":upstream_url,"ANTHROPIC_API_KEY":"upstream-test-key","ANTHROPIC_AUTH_TOKEN":"upstream-test-key"}}),
        ),
        GatewayCliKey::Codex => (
            DbTable::CliProvider(Cli::Codex),
            json!({"auth":{"OPENAI_API_KEY":"upstream-test-key"}, "config":format!("model_provider = \"custom\"\nmodel = \"test-model\"\n[model_providers.custom]\nbase_url = \"{upstream_url}\"\nwire_api = \"responses\"\nsupports_websockets = true\n")}),
        ),
        GatewayCliKey::Grok => (
            DbTable::CliProvider(Cli::Grok),
            json!({"auth":{"API_KEY":"upstream-test-key"},"config":format!("[models]\ndefault = \"custom\"\n[model.custom]\nmodel = \"test-model\"\nbase_url = \"{upstream_url}\"\napi_backend = \"responses\"\n")}),
        ),
        GatewayCliKey::Kimi => (
            DbTable::CliProvider(Cli::Kimi),
            json!({"auth":{"API_KEY":"upstream-test-key"},"providerConfigs":{"custom":{"type":"openai","base_url":upstream_url}},"defaultModelKey":"test-model","modelCatalog":{"models":[{"key":"test-model","model":"test-model","provider":"custom"}]}}),
        ),
        GatewayCliKey::Gemini => (
            DbTable::CliProvider(Cli::Gemini),
            json!({"env":{"GOOGLE_GEMINI_BASE_URL":upstream_url,"GEMINI_API_KEY":"upstream-test-key"}}),
        ),
        GatewayCliKey::Antigravity => (
            DbTable::CliProvider(Cli::Antigravity),
            json!({"env":{"GOOGLE_GEMINI_BASE_URL":upstream_url,"GEMINI_API_KEY":"upstream-test-key"}}),
        ),
        GatewayCliKey::Workbuddy => (
            DbTable::CliProvider(Cli::Workbuddy),
            json!({"baseUrl": upstream_url, "apiKey": "upstream-test-key"}),
        ),
        GatewayCliKey::Codebuddy => (
            DbTable::CliProvider(Cli::Codebuddy),
            json!({"baseUrl": upstream_url, "apiKey": "upstream-test-key"}),
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

/// Serves the native attempt every Responses-target compaction turn makes first,
/// and returns the summarizing retry the relay sends on the same socket.
///
/// The relay offers the trigger before it summarizes, so a non-native upstream
/// sees a compaction turn twice: once as Codex wrote it, and once as a plain
/// summary. This answers the first the way such an upstream does — with an
/// ordinary assistant message, which the relay refuses and discards — and hands
/// back the second request for the caller to inspect and answer.
async fn refuse_native_compaction_attempt<S>(socket: &mut WebSocketStream<S>) -> Value
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let offered = receive_json(socket).await;
    assert_eq!(
        offered["type"], "response.create",
        "the first frame must be the offered compaction turn: {offered}"
    );
    assert!(
        offered["input"]
            .as_array()
            .is_some_and(|input| input.iter().any(|item| item["type"] == "compaction_trigger")),
        "the offer must carry the trigger: {offered}"
    );

    send_json(
        socket,
        json!({"type":"response.created","response":{"id":"resp_refused","output":[]}}),
    )
    .await;
    send_json(
        socket,
        json!({"type":"response.output_item.added","output_index":0,
               "item":{"type":"message","role":"assistant"}}),
    )
    .await;
    send_json(
        socket,
        json!({"type":"response.output_text.delta","delta":"An ordinary assistant reply"}),
    )
    .await;
    send_json(
        socket,
        json!({"type":"response.completed","response":{
            "id":"resp_refused",
            "output":[{"type":"message","role":"assistant",
                       "content":[{"type":"output_text","text":"An ordinary assistant reply"}]}]}}),
    )
    .await;

    let retry = receive_json(socket).await;
    assert_eq!(
        retry["type"], "response.create",
        "the refused attempt must be followed by a summarizing retry: {retry}"
    );
    assert!(
        retry["input"]
            .as_array()
            .is_some_and(|input| input.iter().all(|item| item["type"] != "compaction_trigger")),
        "the retry must not offer the trigger again: {retry}"
    );
    retry
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
        let request = refuse_native_compaction_attempt(&mut socket).await;
        // The retry is a plain summarization turn: the private control item is
        // gone, and so is the tool inventory that is irrelevant to summarizing.
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

    // The turn is billed like any other request, and the log says the trigger
    // was offered and refused rather than never tried.
    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(detail.summary.total_attempt_count, 1);
    assert_eq!(
        detail.summary.compaction_mode.as_deref(),
        Some("fallback"),
        "a refused offer must be recorded as a fallback, not as a simulation"
    );
}

/// An upstream that answers the offered trigger with its own `compaction` item
/// has that reply replayed verbatim — the relay holds the frames back, sees the
/// item among them, and forwards what the upstream wrote.
///
/// Nothing is synthesized, nothing is sealed, and no second `response.create` is
/// sent: this is the behaviour a chain ending at a real Responses endpoint has
/// when Codex talks to it directly.
#[tokio::test]
async fn websocket_native_compaction_item_is_replayed_verbatim() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let offered = receive_json(&mut socket).await;
        assert!(
            offered["input"]
                .as_array()
                .is_some_and(|input| input.iter().any(|item| item["type"] == "compaction_trigger")),
            "the trigger must be offered: {offered}"
        );

        let item = json!({
            "id": "cmp_upstream_1", "type": "compaction",
            "encrypted_content": "gAAAAABq-upstream-cipher"
        });
        send_json(&mut socket, json!({"type":"response.created","response":{"id":"resp_native","output":[]}})).await;
        send_json(&mut socket, json!({"type":"response.output_item.added","output_index":0,"item":item})).await;
        send_json(&mut socket, json!({"type":"response.output_item.done","output_index":0,"item":item})).await;
        send_json(&mut socket, json!({"type":"response.completed","response":{
            "id":"resp_native","status":"completed","output":[item],
            "usage":{"input_tokens":40,"output_tokens":5}}})).await;

        // A native reply answers the turn outright. The socket stays open for
        // later turns, so the only thing that may arrive is the client's own
        // close — never a second `response.create`.
        if let Ok(Some(Ok(Message::Text(text)))) =
            timeout(Duration::from_millis(500), socket.next()).await
        {
            assert!(
                !text.contains("response.create"),
                "a native reply must not be followed by a summarizing retry: {text}"
            );
        }
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
        let terminal = frame["type"] == "response.completed";
        frames.push(frame);
        if terminal {
            break;
        }
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    let items: Vec<&Value> = frames
        .iter()
        .filter_map(|frame| (frame["type"] == "response.output_item.done").then(|| &frame["item"]))
        .collect();
    assert_eq!(items.len(), 1, "expected exactly one done item: {frames:?}");
    assert_eq!(
        items[0]["encrypted_content"], "gAAAAABq-upstream-cipher",
        "the upstream's own ciphertext must be relayed: {frames:?}"
    );
    assert!(
        frames
            .iter()
            .all(|frame| !frame.to_string().contains("ai-toolbox-compact-v1:")),
        "a native turn must not be sealed by the Gateway: {frames:?}"
    );
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == "response.completed")
            .count(),
        1,
        "exactly one terminal must reach Codex: {frames:?}"
    );

    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(
        detail.summary.compaction_mode.as_deref(),
        Some("native"),
        "the log must say the upstream wrote the item"
    );
    assert_eq!(detail.summary.total_attempt_count, 1);
}

/// A transient native attempt may still use the existing one-shot summarizing
/// retry, but must not disable native offers for later turns of this provider.
/// The ordinary assistant-reply control must retain refusal memory.
async fn assert_websocket_compaction_refusal_memory(first_terminal: Value, expect_probe: bool) {
    timeout(Duration::from_secs(30), async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let (_directory, context, _) = test_context(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            "openai_responses",
            true,
        );
        let upstream = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let offered = receive_json(&mut socket).await;
            assert!(offered["input"].as_array().unwrap().iter()
                .any(|item| item["type"] == "compaction_trigger"));
            send_json(&mut socket, json!({"type":"response.created","response":{
                "id":"resp_probe_first","output":[]}})).await;
            send_json(&mut socket, first_terminal).await;
            let retry = receive_json(&mut socket).await;
            assert_eq!(retry["type"], "response.create");
            assert!(retry["input"].as_array().unwrap().iter()
                .all(|item| item["type"] != "compaction_trigger"));
            send_json(&mut socket, json!({"type":"response.output_text.delta",
                "delta":"First complete summary"})).await;
            send_json(&mut socket, json!({"type":"response.completed","response":{
                "id":"resp_summary_first","status":"completed"}})).await;

            let second = receive_json(&mut socket).await;
            let probed = second["input"].as_array().unwrap().iter()
                .any(|item| item["type"] == "compaction_trigger");
            // Answer either actual request shape so the negative case fails at the
            // assertion below, rather than hanging waiting for an upstream reply.
            if probed {
                let item = json!({"id":"cmp_native_second","type":"compaction",
                    "encrypted_content":"gAAAAABq-upstream-cipher"});
                send_json(&mut socket, json!({"type":"response.created","response":{
                    "id":"resp_native_second","output":[]}})).await;
                send_json(&mut socket, json!({"type":"response.output_item.done",
                    "output_index":0,"item":item})).await;
                send_json(&mut socket, json!({"type":"response.completed","response":{
                    "id":"resp_native_second","status":"completed","output":[item]}})).await;
            } else {
                send_json(&mut socket, json!({"type":"response.output_text.delta",
                    "delta":"Second complete summary"})).await;
                send_json(&mut socket, json!({"type":"response.completed","response":{
                    "id":"resp_summary_second","status":"completed"}})).await;
            }
            // A completed second turn must not cause another summarizing request.
            if let Ok(Some(Ok(Message::Text(text)))) =
                timeout(Duration::from_millis(500), socket.next()).await
            {
                assert!(!text.contains("response.create"), "unexpected retry: {text}");
            }
            probed
        });
        let (mut client, gateway) = gateway_connection(context.clone()).await;
        let mut outputs = Vec::new();
        for turn in 0..2 {
            send_json(&mut client, json!({"type":"response.create","model":"test-model",
                "input":[{"type":"message","role":"user","content":[{
                    "type":"input_text","text":format!("turn {turn}")}]},
                    {"type":"compaction_trigger"}]})).await;
            loop {
                let frame = receive_json(&mut client).await;
                if frame["type"] == "response.completed" {
                    outputs.push(frame["response"]["output"].clone());
                    break;
                }
            }
        }
        let _ = client.close(None).await;
        gateway.await.unwrap();
        let probed = upstream.await.unwrap();
        assert_eq!(probed, expect_probe,
            "only an ordinary refusal may suppress the next native compaction offer");
        assert_eq!(outputs[0].as_array().unwrap().len(), 1);
        assert_eq!(outputs[0][0]["type"], "compaction");
        assert!(outputs[0][0]["encrypted_content"].as_str().unwrap()
            .starts_with("ai-toolbox-compact-v1:"));
        assert_eq!(outputs[1].as_array().unwrap().len(), 1);
        assert_eq!(outputs[1][0]["type"], "compaction");
        if expect_probe {
            assert_eq!(outputs[1][0]["encrypted_content"], "gAAAAABq-upstream-cipher");
        } else {
            assert!(outputs[1][0]["encrypted_content"].as_str().unwrap()
                .starts_with("ai-toolbox-compact-v1:"));
        }
    })
    .await
    .expect("two-turn compaction regression must finish within 30 seconds");
}

#[tokio::test]
async fn websocket_compaction_refusal_memory_ignores_failed_event() {
    assert_websocket_compaction_refusal_memory(json!({"type":"response.failed",
        "response":{"id":"resp_probe_first","status":"failed",
            "error":{"code":"server_error","message":"transient fixture"}}}), true).await;
}

#[tokio::test]
async fn websocket_compaction_refusal_memory_ignores_incomplete_event() {
    assert_websocket_compaction_refusal_memory(json!({"type":"response.incomplete",
        "response":{"id":"resp_probe_first","status":"incomplete"}}), true).await;
}

#[tokio::test]
async fn websocket_compaction_refusal_memory_ignores_failed_snapshot() {
    assert_websocket_compaction_refusal_memory(json!({"type":"response.completed",
        "response":{"id":"resp_probe_first","status":"failed",
            "error":{"code":"server_error","message":"transient fixture"}}}), true).await;
}

#[tokio::test]
async fn websocket_compaction_refusal_memory_ignores_incomplete_snapshot() {
    assert_websocket_compaction_refusal_memory(json!({"type":"response.completed",
        "response":{"id":"resp_probe_first","status":"incomplete"}}), true).await;
}

#[tokio::test]
async fn websocket_compaction_refusal_memory_ignores_cancelled_event() {
    // A dedicated `response.cancelled` event (no completed snapshot carrying a
    // status) must end the turn and count as transient, the same way
    // `is_failed` reads `canceled`/`cancelled` on the HTTP side.
    assert_websocket_compaction_refusal_memory(json!({"type":"response.cancelled",
        "response":{"id":"resp_probe_first","status":"cancelled"}}), true).await;
}

#[tokio::test]
async fn websocket_compaction_refusal_memory_keeps_ordinary_refusal() {
    assert_websocket_compaction_refusal_memory(json!({"type":"response.completed",
        "response":{"id":"resp_probe_first","status":"completed","output":[{
            "type":"message","role":"assistant","content":[{
                "type":"output_text","text":"An ordinary assistant reply"}]}]}}), false).await;
}

/// Codex keeps the compaction item at the head of its history, so a session
/// that compacts a second time sends that item *and* a fresh
/// `compaction_trigger` in the same turn.
///
/// The capsule is this Gateway's own ciphertext — the upstream that rejected it
/// is the one that reported `invalid_encrypted_content`. Expanding it therefore
/// cannot be the replay turn's private step, and the WS transport rewrites the
/// request through the same helper the HTTP route does.
#[tokio::test]
async fn websocket_second_compaction_expands_the_previous_capsule() {
    let previous = super::super::compat::codex_remote_compaction::build_compaction_item(
        "Goal: the first summary",
        "test-model",
    )
    .expect("the fixture capsule must seal");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        // The offered attempt expands the previous capsule too, not just the
        // retry: an upstream that answers natively must still not receive a
        // ciphertext it cannot open.
        let offered = receive_json(&mut socket).await;
        assert!(
            !offered.to_string().contains("ai-toolbox-compact-v1:"),
            "the capsule prefix leaked upstream: {offered}"
        );
        assert!(
            offered["input"].as_array().is_some_and(|input| input
                .iter()
                .any(|item| item["content"][0]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Goal: the first summary")))),
            "the previous summary must reach the offered turn as context: {offered}"
        );
        send_json(
            &mut socket,
            json!({"type":"response.created","response":{"id":"resp_refused","output":[]}}),
        )
        .await;
        send_json(
            &mut socket,
            json!({"type":"response.output_item.added","output_index":0,
                   "item":{"type":"message","role":"assistant"}}),
        )
        .await;
        send_json(
            &mut socket,
            json!({"type":"response.completed","response":{
                "id":"resp_refused",
                "output":[{"type":"message","role":"assistant",
                           "content":[{"type":"output_text","text":"An ordinary assistant reply"}]}]}}),
        )
        .await;

        let request = receive_json(&mut socket).await;
        let input = request["input"].as_array().cloned().unwrap_or_default();

        assert!(
            !input.iter().any(|item| item["type"] == "compaction"),
            "the Gateway's own capsule reached the upstream: {request}"
        );
        assert!(
            !request.to_string().contains("ai-toolbox-compact-v1:"),
            "the capsule prefix leaked upstream: {request}"
        );
        assert!(
            !input.iter().any(|item| item["type"] == "compaction_trigger"),
            "the compaction trigger leaked upstream: {request}"
        );
        assert!(
            input.iter().any(|item| item["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("Goal: the first summary"))),
            "the previous summary must reach the summarizer as context: {request}"
        );

        send_json(
            &mut socket,
            json!({"type":"response.output_text.delta","delta":"Goal: the second"}),
        )
        .await;
        send_json(
            &mut socket,
            json!({"type":"response.completed","response":{"id":"upstream-response","usage":{"input_tokens":7,"output_tokens":2}}}),
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
            "instructions": "You are a coding agent.",
            "input": [
                previous,
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "keep going" }] },
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

    let items: Vec<&Value> = frames
        .iter()
        .filter_map(|frame| (frame["type"] == "response.output_item.done").then(|| &frame["item"]))
        .collect();
    assert_eq!(items.len(), 1, "expected exactly one done item: {frames:?}");
    assert_eq!(items[0]["type"], "compaction");
    let summary = super::super::compat::codex_remote_compaction::open_compaction_summary(
        items[0]["encrypted_content"].as_str().unwrap(),
    )
    .expect("the item must carry a capsule this Gateway can read back");
    assert_eq!(summary, "Goal: the second");
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
        let request = refuse_native_compaction_attempt(&mut socket).await;
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
        refuse_native_compaction_attempt(&mut socket).await;
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
        refuse_native_compaction_attempt(&mut socket).await;
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
        refuse_native_compaction_attempt(&mut socket).await;
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

fn install_codex_aux_model(
    context: &GatewayRuntimeContext,
    provider_id: &str,
    task: &str,
    model: &str,
) {
    let db = context.db.as_ref().expect("test database");
    db.with_conn(|conn| {
        let mut provider = db_get(conn, DbTable::CliProvider(Cli::Codex), provider_id)?
            .expect("test provider");
        let mut settings: Value = serde_json::from_str(
            provider["settings_config"]
                .as_str()
                .expect("settings_config is a JSON string"),
        )
        .map_err(|error| error.to_string())?;
        settings["auxiliaryModels"] = json!({
            task: { "mode": "model", "model": model }
        });
        provider["settings_config"] = Value::String(settings.to_string());
        db_put(
            conn,
            DbTable::CliProvider(Cli::Codex),
            provider_id,
            &provider,
        )
    })
    .expect("install auxiliary model");
    super::super::providers::clear_gateway_provider_selection_cache();
}

fn install_codex_aux_main_model(
    context: &GatewayRuntimeContext,
    provider_id: &str,
    task: &str,
) {
    let db = context.db.as_ref().expect("test database");
    db.with_conn(|conn| {
        let mut provider = db_get(conn, DbTable::CliProvider(Cli::Codex), provider_id)?
            .expect("test provider");
        let mut settings: Value = serde_json::from_str(
            provider["settings_config"]
                .as_str()
                .expect("settings_config is a JSON string"),
        )
        .map_err(|error| error.to_string())?;
        settings["auxiliaryModels"] = json!({
            task: { "mode": "main" }
        });
        provider["settings_config"] = Value::String(settings.to_string());
        db_put(
            conn,
            DbTable::CliProvider(Cli::Codex),
            provider_id,
            &provider,
        )
    })
    .expect("install auxiliary main route");
    super::super::providers::clear_gateway_provider_selection_cache();
}

fn remove_codex_default_model(context: &GatewayRuntimeContext, provider_id: &str) {
    let db = context.db.as_ref().expect("test database");
    db.with_conn(|conn| {
        let mut provider = db_get(conn, DbTable::CliProvider(Cli::Codex), provider_id)?
            .expect("test provider");
        let mut settings: Value = serde_json::from_str(
            provider["settings_config"]
                .as_str()
                .expect("settings_config is a JSON string"),
        )
        .map_err(|error| error.to_string())?;
        let config = settings["config"]
            .as_str()
            .expect("Codex config string")
            .replace("model = \"test-model\"\n", "");
        settings["config"] = Value::String(config);
        provider["settings_config"] = Value::String(settings.to_string());
        db_put(
            conn,
            DbTable::CliProvider(Cli::Codex),
            provider_id,
            &provider,
        )
    })
    .expect("remove default model");
    super::super::providers::clear_gateway_provider_selection_cache();
}

async fn receive_http_request(socket: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    use tokio::io::AsyncReadExt;

    let mut raw = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let read = socket.read(&mut chunk).await.expect("read HTTP request");
        assert!(read > 0, "HTTP request ended before headers");
        raw.extend_from_slice(&chunk[..read]);
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while raw.len() < header_end + content_length {
        let mut chunk = [0_u8; 2048];
        let read = socket.read(&mut chunk).await.expect("read HTTP body");
        assert!(read > 0, "HTTP request ended before the declared body");
        raw.extend_from_slice(&chunk[..read]);
    }
    (
        headers,
        raw[header_end..header_end + content_length].to_vec(),
    )
}

async fn write_http_json(socket: &mut tokio::net::TcpStream, value: &Value) {
    use tokio::io::AsyncWriteExt;

    let body = serde_json::to_vec(value).expect("serialize HTTP response");
    let header = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    socket
        .write_all(header.as_bytes())
        .await
        .expect("write HTTP headers");
    socket
        .write_all(&body)
        .await
        .expect("write HTTP body");
    socket.shutdown().await.expect("close HTTP response");
}

#[tokio::test]
async fn websocket_title_auxiliary_turn_uses_http_and_shapes_the_frames() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let selected = tokio::select! {
            message = websocket.next() => (None, message),
            accepted = listener.accept() => (Some(accepted.unwrap().0), None),
        };
        assert!(
            selected.1.is_none(),
            "a route-key auxiliary turn must not use the pinned WebSocket: {:?}",
            selected.1
        );
        let mut http = selected.0.expect("the auxiliary HTTP connection");

        let (headers, body) = receive_http_request(&mut http).await;
        assert!(
            headers.starts_with("POST /v1/responses"),
            "unexpected auxiliary path: {headers}"
        );
        let request: Value = serde_json::from_slice(&body).expect("auxiliary JSON body");
        assert_eq!(request["model"], "title-model");
        assert_eq!(request["stream"], false);
        assert!(request.get("type").is_none(), "WS type leaked into HTTP");
        assert!(request.get("stream_id").is_none(), "WS stream_id leaked into HTTP");
        let turn_metadata: Value = serde_json::from_str(
            request["client_metadata"]["x-codex-turn-metadata"]
                .as_str()
                .expect("turn metadata string"),
        )
        .expect("turn metadata JSON");
        assert_eq!(turn_metadata, json!({"thread_source": "thread_title"}));

        write_http_json(
            &mut http,
            &json!({
                "id": "resp_title",
                "object": "response",
                "status": "completed",
                "model": "title-model",
                "output": [{
                    "id": "msg_title",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "My generated title" }]
                }],
                "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
            }),
        )
        .await;

        // Keep the pinned socket alive until the client closes so the relay can
        // finish the turn without an upstream disconnect.
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "title-lane",
            "input": "hello",
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "thread_title"}).to_string()
            }
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

    let delta = frames
        .iter()
        .find(|frame| frame["type"] == "response.output_text.delta")
        .and_then(|frame| frame["delta"].as_str())
        .expect("shaped delta frame");
    assert_eq!(delta, r#"{"title":"My generated title"}"#);
    let completed = frames.last().expect("completed frame");
    assert_eq!(completed["response"]["model"], "title-model");
    assert_eq!(
        completed["response"]["output"][0]["content"][0]["text"],
        r#"{"title":"My generated title"}"#
    );
    assert_eq!(completed["response"]["usage"]["input_tokens"], 3);
    assert_eq!(completed["response"]["usage"]["output_tokens"], 2);

    let detail = recorded_details(&context).pop().expect("request detail");
    assert_eq!(
        detail.summary.aux_task,
        Some(crate::coding::proxy_gateway::types::CodexAuxTaskKind::Title)
    );
    assert_eq!(detail.summary.aux_route_source.as_deref(), Some("explicit"));
    assert_eq!(detail.summary.upstream_model_id.as_deref(), Some("title-model"));
    assert_eq!(detail.summary.total_tokens, Some(5));
    let auxiliary = detail.auxiliary.expect("auxiliary detail");
    assert!(auxiliary.route_applied);
    assert!(auxiliary.shaped);
}

#[tokio::test]
async fn websocket_auxiliary_http_failure_synthesizes_a_failed_frame() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let selected = tokio::select! {
            message = websocket.next() => (None, message),
            accepted = listener.accept() => (Some(accepted.unwrap().0), None),
        };
        assert!(selected.1.is_none());
        let mut http = selected.0.expect("auxiliary HTTP connection");
        let _ = receive_http_request(&mut http).await;
        use tokio::io::AsyncWriteExt;
        let body = br#"{"error":{"message":"title model is unavailable"}}"#;
        let header = format!(
            "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        http.write_all(header.as_bytes()).await.unwrap();
        http.write_all(body).await.unwrap();
        http.shutdown().await.unwrap();
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "title-lane",
            "input": "hello",
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "thread_title"}).to_string()
            }
        }),
    )
    .await;

    let created = receive_json(&mut client).await;
    assert_eq!(created["type"], "response.created");
    let response_id = created["response"]["id"]
        .as_str()
        .expect("synthetic auxiliary response id")
        .to_string();
    assert!(response_id.starts_with("resp_aux_ws_"));

    let failure = receive_json(&mut client).await;
    assert_eq!(failure["type"], "response.failed");
    assert_eq!(failure["response"]["id"], response_id);
    assert_eq!(failure["response"]["status"], "failed");
    assert_eq!(
        failure["response"]["error"]["code"],
        "aux_task_model_not_found"
    );
    assert!(
        failure["response"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("title model is unavailable")
    );
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    let detail = recorded_details(&context).pop().expect("request detail");
    assert_eq!(detail.summary.aux_route_source.as_deref(), Some("explicit"));
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Failed)
    );
    let auxiliary = detail.auxiliary.expect("auxiliary detail");
    assert!(auxiliary.route_applied);
    assert!(!auxiliary.shaped);
}

#[test]
fn auxiliary_http_request_drops_connection_level_aux_headers() {
    let request = DebugHttpRequest {
        id: 1,
        method: "WS".to_string(),
        path: "/openai/v1/responses".to_string(),
        headers: vec![
            ("authorization".to_string(), "Bearer client-key".to_string()),
            ("x-openai-subagent".to_string(), "guardian".to_string()),
            ("x-openai-memgen-request".to_string(), "true".to_string()),
            (
                "openai-beta".to_string(),
                "responses_websockets=2026-02-06".to_string(),
            ),
            ("sec-websocket-key".to_string(), "dGhlIHNhbXBsZSBub25jZQ==".to_string()),
        ],
        body: b"{}".to_vec(),
    };
    let http_request = build_websocket_aux_http_request(
        &request,
        &json!({
            "type": "response.create",
            "model": "test-model",
            "stream_id": "title-lane",
            "stream": true
        }),
        "title-model",
    )
    .expect("build auxiliary HTTP request");

    assert_eq!(http_request.method, "POST");
    assert!(http_request
        .headers
        .iter()
        .any(|(name, value)| name == "authorization" && value == "Bearer client-key"));
    assert!(http_request
        .headers
        .iter()
        .any(|(name, value)| name == "content-type" && value == "application/json"));
    for dropped in [
        "x-openai-subagent",
        "x-openai-memgen-request",
        "openai-beta",
        "sec-websocket-key",
    ] {
        assert!(
            http_request
                .headers
                .iter()
                .all(|(name, _)| !name.eq_ignore_ascii_case(dropped)),
            "{dropped} must not describe the synthesized HTTP request"
        );
    }
}

#[tokio::test]
async fn websocket_auxiliary_http_protocol_error_is_not_shaped() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let selected = tokio::select! {
            message = websocket.next() => (None, message),
            accepted = listener.accept() => (Some(accepted.unwrap().0), None),
        };
        assert!(selected.1.is_none());
        let mut http = selected.0.expect("auxiliary HTTP connection");
        let _ = receive_http_request(&mut http).await;
        write_http_json(
            &mut http,
            &json!({
                "type": "response.failed",
                "response": {
                    "id": "resp_refused",
                    "status": "failed",
                    "error": {
                        "code": "server_error",
                        "message": "title upstream refused"
                    }
                }
            }),
        )
        .await;
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "title-lane",
            "input": "hello",
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "thread_title"}).to_string()
            }
        }),
    )
    .await;

    let created = receive_json(&mut client).await;
    assert_eq!(created["type"], "response.created");
    let failure = receive_json(&mut client).await;
    assert_eq!(failure["type"], "response.failed");
    assert_eq!(failure["response"]["id"], created["response"]["id"]);
    assert_eq!(
        failure["response"]["error"]["code"],
        "aux_task_upstream_bad_request"
    );
    assert!(
        failure["response"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("title upstream refused")
    );

    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    let detail = recorded_details(&context).pop().expect("request detail");
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Failed)
    );
    let auxiliary = detail.auxiliary.expect("auxiliary detail");
    assert!(auxiliary.route_applied);
    assert!(!auxiliary.shaped);
}

#[tokio::test]
async fn websocket_auxiliary_cancel_drops_the_late_http_result() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let selected = tokio::select! {
            message = websocket.next() => (None, message),
            accepted = listener.accept() => (Some(accepted.unwrap().0), None),
        };
        assert!(selected.1.is_none());
        let mut http = selected.0.expect("auxiliary HTTP connection");
        let _ = receive_http_request(&mut http).await;
        request_seen_tx.send(()).expect("signal HTTP request");
        release_rx.await.expect("wait for the late HTTP reply");
        write_http_json(
            &mut http,
            &json!({
                "id": "resp_late",
                "object": "response",
                "status": "completed",
                "model": "title-model",
                "output": [{
                    "id": "msg_late",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "late title" }]
                }]
            }),
        )
        .await;
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "title-lane",
            "input": "hello",
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "thread_title"}).to_string()
            }
        }),
    )
    .await;
    request_seen_rx.await.expect("auxiliary request reached HTTP");

    send_json(
        &mut client,
        json!({"type": "response.cancel", "stream_id": "title-lane"}),
    )
    .await;
    // Let the relay pop the lane before the HTTP task completes, so the test
    // exercises the "late result must be dropped" path rather than a race.
    tokio::time::sleep(Duration::from_millis(200)).await;
    release_tx.send(()).expect("release the HTTP reply");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), client.next())
            .await
            .is_err(),
        "a cancelled auxiliary turn must not deliver the late HTTP response"
    );

    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    let detail = recorded_details(&context).pop().expect("request detail");
    assert_eq!(detail.summary.aux_route_source.as_deref(), Some("explicit"));
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Canceled)
    );
    assert_eq!(
        detail.summary.error_category.as_deref(),
        Some("client_cancelled")
    );
}

#[tokio::test]
async fn websocket_auxiliary_http_completions_may_arrive_out_of_order() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let mut http_connections = Vec::new();
        while http_connections.len() < 2 {
            tokio::select! {
                message = websocket.next() => {
                    panic!("auxiliary turns must not use the pinned WebSocket: {message:?}");
                }
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    let _ = receive_http_request(&mut socket).await;
                    http_connections.push(socket);
                }
            }
        }

        // Complete the second turn before the first. Codex sends frames
        // without `stream_id`, so both turns share the empty lane unless the
        // relay assigns each in-flight auxiliary request its own queue.
        let mut second = http_connections.pop().expect("second HTTP request");
        let mut first = http_connections.pop().expect("first HTTP request");
        write_http_json(
            &mut second,
            &json!({
                "id": "resp_second",
                "object": "response",
                "status": "completed",
                "model": "title-model",
                "output": [{
                    "id": "msg_second",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "second title" }]
                }],
                "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
            }),
        )
        .await;
        write_http_json(
            &mut first,
            &json!({
                "id": "resp_first",
                "object": "response",
                "status": "completed",
                "model": "title-model",
                "output": [{
                    "id": "msg_first",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "first title" }]
                }],
                "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
            }),
        )
        .await;
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    for input in ["first", "second"] {
        send_json(
            &mut client,
            json!({
                "type": "response.create",
                "model": "test-model",
                "stream": true,
                "input": input,
                "client_metadata": {
                    "x-codex-turn-metadata": json!({"thread_source": "thread_title"}).to_string()
                }
            }),
        )
        .await;
    }

    let mut deltas = Vec::new();
    let mut completed = 0;
    while completed < 2 {
        let frame = receive_json(&mut client).await;
        match frame["type"].as_str() {
            Some("response.output_text.delta") => deltas.push(
                frame["delta"]
                    .as_str()
                    .expect("delta text")
                    .to_string(),
            ),
            Some("response.completed") => completed += 1,
            Some("response.failed") => panic!("auxiliary turn failed: {frame}"),
            _ => {}
        }
    }
    deltas.sort();
    assert_eq!(
        deltas,
        vec![
            r#"{"title":"first title"}"#.to_string(),
            r#"{"title":"second title"}"#.to_string(),
        ]
    );

    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();
}

#[tokio::test]
async fn websocket_memory_auxiliary_turn_rewrites_without_shaping() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "memory", "memory-model");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let selected = tokio::select! {
            message = websocket.next() => (None, message),
            accepted = listener.accept() => (Some(accepted.unwrap().0), None),
        };
        assert!(
            selected.1.is_none(),
            "a memory turn must not use the pinned WebSocket"
        );
        let mut http = selected.0.expect("memory HTTP connection");
        let (_, body) = receive_http_request(&mut http).await;
        let request: Value = serde_json::from_slice(&body).expect("memory JSON body");
        assert_eq!(request["model"], "memory-model");
        assert_eq!(request["stream"], false);
        write_http_json(
            &mut http,
            &json!({
                "id": "resp_memory",
                "object": "response",
                "status": "completed",
                "model": "memory-model",
                "output": [{
                    "id": "msg_memory",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "remember this" }]
                }],
                "usage": { "input_tokens": 4, "output_tokens": 2, "total_tokens": 6 }
            }),
        )
        .await;
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "memory-lane",
            "input": "consolidate",
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "memory_consolidation"}).to_string()
            }
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

    let delta = frames
        .iter()
        .find(|frame| frame["type"] == "response.output_text.delta")
        .and_then(|frame| frame["delta"].as_str())
        .expect("memory delta frame");
    assert_eq!(delta, "remember this");
    let completed = frames.last().expect("completed frame");
    assert_eq!(completed["response"]["model"], "memory-model");
    assert_eq!(
        completed["response"]["output"][0]["content"][0]["text"],
        "remember this"
    );

    let detail = recorded_details(&context).pop().expect("request detail");
    assert_eq!(
        detail.summary.aux_task,
        Some(crate::coding::proxy_gateway::types::CodexAuxTaskKind::Memory)
    );
    assert_eq!(detail.summary.aux_route_source.as_deref(), Some("explicit"));
    let auxiliary = detail.auxiliary.expect("auxiliary detail");
    assert!(auxiliary.route_applied);
    assert!(!auxiliary.shaped);
}

#[tokio::test]
async fn websocket_auxiliary_main_route_missing_fails_before_upstream() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    remove_codex_default_model(&context, &provider_id);
    install_codex_aux_main_model(&context, &provider_id, "title");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        if let Ok(Some(Ok(Message::Text(text)))) =
            timeout(Duration::from_millis(500), websocket.next()).await
        {
            panic!(
                "the missing main route must be rejected before any upstream frame: {text}"
            );
        }
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "title-lane",
            "input": "hello",
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "thread_title"}).to_string()
            }
        }),
    )
    .await;

    let error = receive_json(&mut client).await;
    assert_eq!(error["type"], "error");
    assert_eq!(error["status"], 400);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("aux_task_main_model_missing")
    );
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    let detail = recorded_details(&context).pop().expect("request detail");
    assert_eq!(
        detail.summary.aux_task,
        Some(crate::coding::proxy_gateway::types::CodexAuxTaskKind::Title)
    );
    assert_eq!(detail.summary.aux_route_source.as_deref(), Some("none"));
    let auxiliary = detail.auxiliary.expect("auxiliary detail");
    assert!(!auxiliary.route_applied);
    assert_eq!(
        auxiliary.route_error.as_deref(),
        Some("aux_task_main_model_missing")
    );
}

#[tokio::test]
async fn websocket_already_shaped_auxiliary_reply_is_not_marked_shaped() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let selected = tokio::select! {
            message = websocket.next() => (None, message),
            accepted = listener.accept() => (Some(accepted.unwrap().0), None),
        };
        assert!(selected.1.is_none());
        let mut http = selected.0.expect("auxiliary HTTP connection");
        let _ = receive_http_request(&mut http).await;
        write_http_json(
            &mut http,
            &json!({
                "id": "resp_title",
                "object": "response",
                "status": "completed",
                "model": "title-model",
                "output": [{
                    "id": "msg_title",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{
                        "type": "output_text",
                        "text": "{\"title\":\"already shaped\"}"
                    }]
                }],
                "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
            }),
        )
        .await;
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "title-lane",
            "input": "hello",
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "thread_title"}).to_string()
            }
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

    let delta = frames
        .iter()
        .find(|frame| frame["type"] == "response.output_text.delta")
        .and_then(|frame| frame["delta"].as_str())
        .expect("delta frame");
    assert_eq!(delta, r#"{"title":"already shaped"}"#);
    let detail = recorded_details(&context).pop().expect("request detail");
    let auxiliary = detail.auxiliary.expect("auxiliary detail");
    assert!(auxiliary.route_applied);
    assert!(!auxiliary.shaped);
}

#[tokio::test]
async fn websocket_auxiliary_http_restores_privacy_tokens() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    privacy_tests::enable_privacy(&context);
    install_codex_aux_model(&context, &provider_id, "memory", "memory-model");

    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let selected = tokio::select! {
            message = websocket.next() => (None, message),
            accepted = listener.accept() => (Some(accepted.unwrap().0), None),
        };
        assert!(selected.1.is_none());
        let mut http = selected.0.expect("auxiliary HTTP connection");
        let (_, body) = receive_http_request(&mut http).await;
        let placeholder = privacy_tests::token(&body);
        write_http_json(
            &mut http,
            &json!({
                "id": "resp_memory",
                "object": "response",
                "status": "completed",
                "model": "memory-model",
                "output": [{
                    "id": "msg_memory",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": placeholder }]
                }],
                "usage": { "input_tokens": 4, "output_tokens": 2, "total_tokens": 6 }
            }),
        )
        .await;
        let _ = websocket.next().await;
    });

    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({
            "type": "response.create",
            "model": "test-model",
            "stream": true,
            "stream_id": "memory-lane",
            "input": privacy_tests::SECRET,
            "client_metadata": {
                "x-codex-turn-metadata": json!({"thread_source": "memory_consolidation"}).to_string()
            }
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

    let delta = frames
        .iter()
        .find(|frame| frame["type"] == "response.output_text.delta")
        .and_then(|frame| frame["delta"].as_str())
        .expect("memory delta frame");
    assert_eq!(delta, privacy_tests::SECRET);
    let completed = frames.last().expect("completed frame");
    assert_eq!(
        completed["response"]["output"][0]["content"][0]["text"],
        privacy_tests::SECRET
    );

    let detail = recorded_details(&context).pop().expect("request detail");
    assert!(detail.summary.success);
    assert_eq!(detail.privacy.as_ref().unwrap().matched_values, 1);
    assert!(
        !serde_json::to_string(&detail).unwrap().contains("secret"),
        "auxiliary privacy detail leaked the original text"
    );
}

fn real_codex_mock_lifecycle(model: &str, text: &str) -> Vec<Value> {
    let item = json!({
        "id": "msg_real_e2e",
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text, "annotations": [] }]
    });
    let completed = json!({
        "id": "resp_real_e2e",
        "object": "response",
        "status": "completed",
        "model": model,
        "output": [item],
        "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
    });
    let mut opened = completed.clone();
    opened["status"] = json!("in_progress");
    opened["output"] = json!([]);
    let mut in_progress = item.clone();
    in_progress["status"] = json!("in_progress");
    vec![
        json!({ "type": "response.created", "response": opened }),
        json!({ "type": "response.in_progress", "response": opened }),
        json!({ "type": "response.output_item.added", "output_index": 0, "item": in_progress }),
        json!({ "type": "response.output_text.delta", "delta": text, "item_id": "msg_real_e2e", "output_index": 0, "content_index": 0 }),
        json!({ "type": "response.output_text.done", "text": text, "item_id": "msg_real_e2e", "output_index": 0, "content_index": 0 }),
        json!({ "type": "response.output_item.done", "output_index": 0, "item": item }),
        json!({ "type": "response.completed", "response": completed }),
    ]
}

/// End-to-end check with the real Codex CLI: a title turn must leave the
/// pinned WebSocket and go through the auxiliary HTTP channel, then reach the
/// client as synthesized bare JSON frames.
///
/// Run with:
/// `cargo test --lib real_codex_title_turn_uses_the_auxiliary_http_channel -- --ignored --nocapture`
#[tokio::test]
#[ignore = "requires the real codex CLI on PATH"]
async fn real_codex_title_turn_uses_the_auxiliary_http_channel() {
    use std::process::Stdio;

    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{upstream_address}/v1"),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let (events_tx, mut events_rx) =
        tokio::sync::mpsc::unbounded_channel::<(&'static str, Value)>();
    let upstream = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = upstream_listener.accept().await else {
                break;
            };
            let events_tx = events_tx.clone();
            tokio::spawn(async move {
                let mut peek = [0_u8; 4];
                let peeked = socket.peek(&mut peek).await.unwrap_or(0);
                if &peek[..peeked] == b"GET " {
                    let Ok(mut websocket) = tokio_tungstenite::accept_async(socket).await else {
                        return;
                    };
                    while let Some(Ok(message)) = websocket.next().await {
                        let Message::Text(text) = message else {
                            continue;
                        };
                        let Ok(value) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        let is_create = value["type"] == "response.create";
                        let _ = events_tx.send(("ws", value.clone()));
                        if is_create {
                            let model = value["model"].as_str().unwrap_or("test-model");
                            for event in real_codex_mock_lifecycle(model, "warmup") {
                                if websocket
                                    .send(Message::Text(event.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                } else {
                    let mut socket = socket;
                    let (_, body) = receive_http_request(&mut socket).await;
                    let value = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
                    let _ = events_tx.send(("http", value));
                    write_http_json(
                        &mut socket,
                        &json!({
                            "id": "resp_real_title",
                            "object": "response",
                            "status": "completed",
                            "model": "title-model",
                            "output": [{
                                "id": "msg_real_title",
                                "type": "message",
                                "status": "completed",
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": "Real Title" }]
                            }],
                            "usage": { "input_tokens": 10, "output_tokens": 2, "total_tokens": 12 }
                        }),
                    )
                    .await;
                }
            });
        }
    });

    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let gateway_address = gateway_listener.local_addr().unwrap();
    let gateway_context = context.clone();
    let gateway = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = gateway_listener.accept().await else {
                break;
            };
            let context = gateway_context.clone();
            tokio::spawn(async move {
                let _ = super::super::handle_connection(&mut socket, &context).await;
            });
        }
    });

    let codex_home = tempfile::tempdir().unwrap();
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            "model_provider = \"e2e\"\n\
             model = \"test-model\"\n\
             [model_providers.e2e]\n\
             name = \"e2e\"\n\
             base_url = \"http://{gateway_address}/openai/v1\"\n\
             wire_api = \"responses\"\n\
             supports_websockets = true\n\
             env_key = \"CODEX_E2E_API_KEY\"\n"
        ),
    )
    .unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let codex_binary =
        std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
    let mut command = tokio::process::Command::new(codex_binary);
    command
        .args([
            "exec",
            "--skip-git-repo-check",
            "--dangerously-bypass-approvals-and-sandbox",
            "--thread-source",
            "thread_title",
            "say hi",
        ])
        .current_dir(workdir.path())
        .env("CODEX_HOME", codex_home.path())
        .env("CODEX_E2E_API_KEY", "e2e-test-key")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = timeout(Duration::from_secs(90), command.output())
        .await
        .expect("real codex exec timed out")
        .expect("run real codex");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "real codex failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let mut websocket_creates = Vec::new();
    let mut http_requests = Vec::new();
    while let Ok((kind, value)) = events_rx.try_recv() {
        match kind {
            "ws" if value["type"] == "response.create" => websocket_creates.push(value),
            "http" => http_requests.push(value),
            _ => {}
        }
    }

    assert!(
        stdout.contains("Real Title"),
        "the shaped auxiliary title never reached real Codex\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        http_requests.len(),
        1,
        "the title turn must be sent over the auxiliary HTTP channel exactly once: {http_requests:?}"
    );
    assert_eq!(http_requests[0]["model"], "title-model");
    assert_eq!(http_requests[0]["stream"], false);
    assert!(
        http_requests[0].get("type").is_none(),
        "WS control fields leaked into the auxiliary HTTP body"
    );
    let turn_metadata: Value = serde_json::from_str(
        http_requests[0]["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .expect("real Codex sends turn metadata"),
    )
    .expect("turn metadata JSON");
    assert_eq!(turn_metadata["thread_source"], "thread_title");
    assert_eq!(turn_metadata["request_kind"], "turn");
    for create in &websocket_creates {
        assert_eq!(
            create["generate"], false,
            "the title turn leaked onto the pinned WebSocket: {create}"
        );
    }

    let detail = timeout(Duration::from_secs(10), async {
        loop {
            if let Some(detail) = recorded_details(&context)
                .into_iter()
                .find(|detail| {
                    detail.summary.aux_task
                        == Some(crate::coding::proxy_gateway::types::CodexAuxTaskKind::Title)
                        && detail.summary.aux_route_source.as_deref() == Some("explicit")
                })
            {
                return detail;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the real title turn must produce a request record");
    assert_eq!(detail.summary.aux_route_source.as_deref(), Some("explicit"));
    assert_eq!(
        detail.summary.upstream_model_id.as_deref(),
        Some("title-model")
    );
    let auxiliary = detail.auxiliary.expect("auxiliary detail");
    assert!(auxiliary.route_applied);
    assert!(auxiliary.shaped);

    gateway.abort();
    upstream.abort();
}

/// The synthesized `response.failed` must terminate a real Codex turn even
/// though the auxiliary HTTP request never sent a `response.created` frame and
/// the frame carries no response id yet.
///
/// Run with:
/// `cargo test --lib real_codex_auxiliary_http_failure_terminates -- --ignored --nocapture`
#[tokio::test]
#[ignore = "requires the real codex CLI on PATH"]
async fn real_codex_auxiliary_http_failure_terminates() {
    use std::process::Stdio;

    let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{upstream_address}/v1"),
        "openai_responses",
        true,
    );
    install_codex_aux_model(&context, &provider_id, "title", "title-model");

    let upstream = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = upstream_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut peek = [0_u8; 4];
                let peeked = socket.peek(&mut peek).await.unwrap_or(0);
                if &peek[..peeked] == b"GET " {
                    let Ok(mut websocket) = tokio_tungstenite::accept_async(socket).await else {
                        return;
                    };
                    while let Some(Ok(message)) = websocket.next().await {
                        let Message::Text(text) = message else {
                            continue;
                        };
                        let Ok(value) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        if value["type"] == "response.create" {
                            let model = value["model"].as_str().unwrap_or("test-model");
                            for event in real_codex_mock_lifecycle(model, "warmup") {
                                if websocket
                                    .send(Message::Text(event.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                } else {
                    let mut socket = socket;
                    let _ = receive_http_request(&mut socket).await;
                    use tokio::io::AsyncWriteExt;
                    let body = br#"{"error":{"message":"title model is unavailable"}}"#;
                    let header = format!(
                        "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    let _ = socket.shutdown().await;
                }
            });
        }
    });

    let gateway_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let gateway_address = gateway_listener.local_addr().unwrap();
    let gateway_context = context.clone();
    let gateway = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = gateway_listener.accept().await else {
                break;
            };
            let context = gateway_context.clone();
            tokio::spawn(async move {
                let _ = super::super::handle_connection(&mut socket, &context).await;
            });
        }
    });

    let codex_home = tempfile::tempdir().unwrap();
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            "model_provider = \"e2e\"\n\
             model = \"test-model\"\n\
             [model_providers.e2e]\n\
             name = \"e2e\"\n\
             base_url = \"http://{gateway_address}/openai/v1\"\n\
             wire_api = \"responses\"\n\
             supports_websockets = true\n\
             env_key = \"CODEX_E2E_API_KEY\"\n"
        ),
    )
    .unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let codex_binary =
        std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
    let mut command = tokio::process::Command::new(codex_binary);
    command
        .args([
            "exec",
            "--skip-git-repo-check",
            "--dangerously-bypass-approvals-and-sandbox",
            "--thread-source",
            "thread_title",
            "say hi",
        ])
        .current_dir(workdir.path())
        .env("CODEX_HOME", codex_home.path())
        .env("CODEX_E2E_API_KEY", "e2e-test-key")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = timeout(Duration::from_secs(45), command.output())
        .await
        .expect("real Codex hung after the synthesized response.failed")
        .expect("run real codex");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        !output.status.success(),
        "real Codex must fail the forced title turn\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("title model is unavailable"),
        "real Codex did not surface the auxiliary failure\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let detail = timeout(Duration::from_secs(10), async {
        loop {
            if let Some(detail) = recorded_details(&context)
                .into_iter()
                .find(|detail| {
                    detail.summary.aux_task
                        == Some(crate::coding::proxy_gateway::types::CodexAuxTaskKind::Title)
                        && detail.summary.stream_outcome
                            == Some(GatewayStreamOutcome::Failed)
                })
            {
                return detail;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "the failed auxiliary turn must be recorded\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )
    });
    assert_eq!(
        detail.summary.error_category.as_deref(),
        Some("aux_task_model_not_found")
    );

    gateway.abort();
    upstream.abort();
}
