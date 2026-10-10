use super::*;

#[tokio::test]
async fn close_drains_in_flight_messages_until_peer_acknowledges() {
    let (gateway_io, client_io) = tokio::io::duplex(4096);
    let mut gateway = WebSocketStream::from_raw_socket(gateway_io, Role::Server, None).await;
    let mut client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model","input":"already in flight"}),
    )
    .await;

    {
        let closing = close_socket(&mut gateway);
        tokio::pin!(closing);
        assert!(futures_util::poll!(&mut closing).is_pending());
        let message = timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap();
        assert!(matches!(message, Some(Ok(Message::Close(_)))));
        assert!(
            futures_util::poll!(&mut closing).is_pending(),
            "sending Close must not release a socket before the peer acknowledges it"
        );
        client.flush().await.unwrap();
        timeout(Duration::from_secs(5), &mut closing).await.unwrap();
    }
    drop(gateway);
    assert!(timeout(Duration::from_secs(5), client.next())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn close_flushes_reply_after_either_peer_initiates_handshake() {
    use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};

    for (gateway_role, peer_role) in [(Role::Server, Role::Client), (Role::Client, Role::Server)] {
        let (gateway_io, peer_io) = tokio::io::duplex(4096);
        let mut gateway = WebSocketStream::from_raw_socket(gateway_io, gateway_role, None).await;
        let mut peer = WebSocketStream::from_raw_socket(peer_io, peer_role, None).await;
        let frame = CloseFrame {
            code: CloseCode::Normal,
            reason: "finished".into(),
        };
        peer.close(Some(frame.clone())).await.unwrap();
        assert_eq!(
            gateway.next().await.unwrap().unwrap(),
            Message::Close(Some(frame.clone()))
        );
        let closing = tokio::spawn(async move { close_socket(&mut gateway).await });
        assert_eq!(
            timeout(Duration::from_secs(5), peer.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            Message::Close(Some(frame)),
            "{gateway_role:?} must flush its already queued Close reply"
        );
        drop(peer);
        timeout(Duration::from_secs(5), closing)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn close_times_out_when_peer_never_acknowledges() {
    let (gateway_io, _unresponsive_peer) = tokio::io::duplex(4096);
    let mut gateway = WebSocketStream::from_raw_socket(gateway_io, Role::Server, None).await;
    let closing = close_socket(&mut gateway);
    tokio::pin!(closing);
    assert!(futures_util::poll!(&mut closing).is_pending());
    timeout(Duration::from_secs(5), closing).await.unwrap();
}

#[tokio::test]
async fn multiplexed_turns_keep_usage_and_errors_in_their_own_lane() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let events = vec![
        json!({"type":"response.created","response":{"id":"default"}}),
        json!({"type":"response.created","stream_id":"a","response":{"id":"a-first"}}),
        json!({"type":"response.created","stream_id":"b","response":{"id":"b-first"}}),
        json!({"type":"error","status":400,"error":{"type":"invalid_request_error","message":"Bad default turn"}}),
        json!({"type":"response.incomplete","stream_id":"a","response":{"id":"a-first","usage":{"input_tokens":7,"output_tokens":2}}}),
        // A repeated terminal must not consume the queued turn on this lane.
        json!({"type":"response.incomplete","stream_id":"a","response":{"id":"a-first","usage":{"input_tokens":7,"output_tokens":2}}}),
        json!({"type":"response.cancelled","stream_id":"b","response":{"id":"b-first","usage":{"input_tokens":3,"output_tokens":1}}}),
        json!({"type":"response.created","stream_id":"a","response":{"id":"a-second"}}),
        // Some relays omit stream_id once a response_id has been assigned.
        json!({"type":"response.output_text.delta","response_id":"a-second","delta":"ok"}),
        json!({"type":"response.completed","stream_id":"a","response":{"id":"a-second","usage":{"input_tokens":11,"output_tokens":4}}}),
    ];
    let upstream_events = events.clone();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        for _ in 0..4 {
            receive_json(&mut socket).await;
        }
        for event in upstream_events {
            send_json(&mut socket, event).await;
        }
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    for stream_id in [None, Some("a"), Some("b"), Some("a")] {
        let mut request = json!({"type":"response.create","model":"test-model","input":"hello"});
        if let Some(id) = stream_id {
            request["stream_id"] = json!(id);
        }
        send_json(&mut client, request).await;
    }
    for expected in events {
        assert_eq!(receive_json(&mut client).await, expected);
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();
    let details = recorded_details(&context);
    assert_eq!(details.len(), 4);
    for (id, outcome, tokens) in [
        ("default", GatewayStreamOutcome::Failed, None),
        ("a-first", GatewayStreamOutcome::Incomplete, Some(9)),
        ("b-first", GatewayStreamOutcome::Canceled, Some(4)),
        ("a-second", GatewayStreamOutcome::Completed, Some(15)),
    ] {
        let detail = details
            .iter()
            .find(|detail| detail.websocket.as_ref().unwrap().response_id.as_deref() == Some(id))
            .unwrap();
        assert_eq!(detail.summary.stream_outcome, Some(outcome), "{id}");
        assert_eq!(detail.summary.total_tokens, tokens, "{id}");
    }
    assert_eq!(context.requests_per_minute(), 4);
    let summary =
        usage_stats::usage_summary(context.db.as_ref().unwrap(), None, None, None, true).unwrap();
    assert_eq!(summary.total_requests, 4);
    assert_eq!(summary.total_tokens, 28);
    assert_eq!(summary.success_rate, 25.0);
}

#[tokio::test]
async fn warmup_and_truncated_or_disabled_bodies_do_not_change_usage() {
    for (store_body, log_enabled) in [(true, true), (false, true), (false, false)] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let (_directory, context, _) = test_context(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            "openai_responses",
            store_body,
        );
        context.settings.write().unwrap().request_log_enabled = log_enabled;
        let upstream = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let warmup = receive_json(&mut socket).await;
            assert_eq!(warmup["generate"], false);
            send_json(&mut socket, json!({"type":"response.completed","response":{"id":"warmup","usage":{"input_tokens":5,"output_tokens":0,"input_tokens_details":{"cached_tokens":2}}}})).await;
            receive_json(&mut socket).await;
            send_json(
                &mut socket,
                json!({"type":"response.created","response":{"id":"body-limit"}}),
            )
            .await;
            send_json(&mut socket, json!({"type":"response.output_text.delta","response_id":"body-limit","delta":"x".repeat(6000)})).await;
            send_json(&mut socket, json!({"type":"response.completed","response":{"id":"body-limit","usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":30}}}})).await;
            let _ = socket.next().await;
        });
        let (mut client, gateway) = gateway_connection(context.clone()).await;
        send_json(
            &mut client,
            json!({"type":"response.create","model":"test-model","generate":false}),
        )
        .await;
        receive_json(&mut client).await;
        send_json(
            &mut client,
            json!({"type":"response.create","model":"test-model","input":"hello"}),
        )
        .await;
        for _ in 0..3 {
            receive_json(&mut client).await;
        }
        let _ = client.close(None).await;
        gateway.await.unwrap();
        upstream.await.unwrap();
        assert_eq!(context.requests_per_minute(), 1);
        let db = context.db.as_ref().unwrap();
        let summary = usage_stats::usage_summary(db, None, None, None, true).unwrap();
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.total_tokens, 125);
        assert_eq!(summary.total_cache_read_tokens, 32);
        assert_eq!(summary.success_rate, 100.0);
        let logs = usage_stats::request_logs(db, &GatewayRequestLogFilters::default(), 0, 10, true)
            .unwrap();
        assert_eq!(logs.total, 2);
        for entry in logs.data {
            let fallback = usage_stats::request_log_detail_from_summary(db, &entry.trace_id)
                .unwrap()
                .unwrap();
            assert_eq!(fallback.summary.status_code, None);
            assert_eq!(
                fallback.summary.transport,
                GatewayRequestTransport::Websocket
            );
            assert_eq!(
                fallback.summary.stream_outcome,
                Some(GatewayStreamOutcome::Completed)
            );
            let detail = request_log::get_request_log_detail(
                context.paths.as_ref().unwrap(),
                &entry.trace_id,
            )
            .unwrap();
            assert_eq!(detail.is_some(), log_enabled);
            if let Some(detail) = detail {
                assert_eq!(detail.request_body.is_some(), store_body);
                assert_eq!(detail.response_body.is_some(), store_body);
                if store_body && entry.request_kind == GatewayRequestKind::Request {
                    let body = detail.response_body.unwrap();
                    assert!(body.contains("truncated"));
                    assert!(!body.contains("response.completed"));
                    assert_eq!(detail.summary.total_tokens, Some(120));
                }
            }
        }
    }
}

#[tokio::test]
async fn upstream_eof_preserves_partial_usage_without_recording_success() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        false,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        receive_json(&mut socket).await;
        send_json(&mut socket, json!({"type":"response.created","response":{"id":"partial","usage":{"input_tokens":80,"output_tokens":5}}})).await;
        socket.close(None).await.unwrap();
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model","input":"hello"}),
    )
    .await;
    receive_json(&mut client).await;
    let _ = timeout(Duration::from_secs(5), client.next())
        .await
        .unwrap();
    gateway.await.unwrap();
    upstream.await.unwrap();
    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(detail.summary.total_tokens, Some(85));
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Incomplete)
    );
    assert!(!detail.summary.success);
    assert_eq!(
        detail.summary.error_category.as_deref(),
        Some("stream_incomplete")
    );
}

#[tokio::test]
async fn stopping_gateway_cancels_pending_turns_and_closes_both_peers() {
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
        send_json(
            &mut socket,
            json!({"type":"response.created","response":{"id":"stopped"}}),
        )
        .await;
        assert!(matches!(
            timeout(Duration::from_secs(5), socket.next())
                .await
                .unwrap(),
            Some(Ok(Message::Close(_)))
        ));
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model"}),
    )
    .await;
    receive_json(&mut client).await;
    context.websocket_shutdown.send_replace(true);
    assert!(matches!(
        timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap(),
        Some(Ok(Message::Close(_)))
    ));
    gateway.await.unwrap();
    upstream.await.unwrap();
    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Canceled)
    );
    assert_eq!(
        detail.summary.error_category.as_deref(),
        Some("gateway_stopped")
    );
}

struct FailingWriter(tokio::io::DuplexStream);

#[tokio::test]
async fn websocket_first_event_timeout_records_one_failed_turn() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    context
        .settings
        .write()
        .unwrap()
        .streaming_first_byte_timeout_secs = 1;
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        receive_json(&mut socket).await;
        let _ = timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap();
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model"}),
    )
    .await;
    assert!(matches!(
        timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap(),
        Some(Ok(Message::Close(_)))
    ));
    gateway.await.unwrap();
    upstream.await.unwrap();
    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Failed)
    );
    assert_eq!(
        detail.summary.error_category.as_deref(),
        Some("stream_first_byte_timeout")
    );
    assert_eq!(context.requests_per_minute(), 1);
}

#[tokio::test]
async fn already_stopped_gateway_does_not_open_an_upstream_socket() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    context.websocket_shutdown.send_replace(true);
    let (address, gateway) = start_gateway(context.clone(), 1).await;
    let socket = TcpStream::connect(address).await.unwrap();
    assert!(timeout(
        Duration::from_secs(5),
        tokio_tungstenite::client_async(format!("ws://{address}/openai/v1/responses"), socket)
    )
    .await
    .unwrap()
    .is_err());
    gateway.await.unwrap();
    assert!(timeout(Duration::from_millis(20), listener.accept())
        .await
        .is_err());
    assert!(recorded_details(&context).is_empty());
}

impl AsyncRead for FailingWriter {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for FailingWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Ready(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "client write failed",
        )))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn failed_terminal_write_keeps_usage_but_is_not_a_success() {
    for invalid_request in [false, true] {
        let (_directory, context, _) =
            test_context("http://unused.invalid/v1", "openai_responses", true);
        let provider = context
            .load_candidate_providers(context.db.as_ref().unwrap(), GatewayCliKey::Codex)
            .await
            .unwrap()
            .providers
            .remove(0);
        let (client_io, gateway_io) = tokio::io::duplex(4096);
        let (upstream_io, server_io) = tokio::io::duplex(4096);
        let downstream =
            WebSocketStream::from_raw_socket(FailingWriter(gateway_io), Role::Server, None).await;
        let upstream_socket =
            WebSocketStream::from_raw_socket(upstream_io, Role::Client, None).await;
        let mut client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let upstream = tokio::spawn(async move {
            let mut server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
            if !invalid_request {
                receive_json(&mut server).await;
                send_json(&mut server, json!({"type":"response.completed","response":{"id":"undelivered","usage":{"input_tokens":30,"output_tokens":5}}})).await;
            }
            let _ = server.next().await;
        });
        let mut payload = json!({"type":"response.create","model":"test-model","input":"hello"});
        if invalid_request {
            payload.as_object_mut().unwrap().remove("model");
        }
        send_json(&mut client, payload).await;
        let handshake = DebugHttpRequest {
            id: 0,
            method: "GET".into(),
            path: "/openai/v1/responses".into(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        timeout(
            Duration::from_secs(5),
            relay(
                downstream,
                upstream_socket,
                &handshake,
                provider,
                &context,
                "failure-connection".into(),
                "ws://unused.invalid/v1/responses".into(),
                Vec::new(),
                false,
                Vec::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        upstream.await.unwrap();
        let detail = recorded_details(&context).pop().unwrap();
        assert_eq!(
            detail.summary.total_tokens,
            if invalid_request { None } else { Some(35) }
        );
        assert_eq!(
            detail.summary.stream_outcome,
            Some(GatewayStreamOutcome::Canceled)
        );
        assert!(!detail.summary.success);
        if invalid_request {
            assert!(detail.upstream_response_body.is_none());
        } else {
            assert!(detail
                .upstream_response_body
                .unwrap()
                .contains("response.completed"));
        }
        assert!(detail
            .response_body
            .as_deref()
            .unwrap_or_default()
            .is_empty());
    }
}

#[tokio::test]
async fn connection_error_is_recorded_for_each_affected_turn() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        for _ in 0..2 {
            receive_json(&mut socket).await;
        }
        send_json(&mut socket, json!({"type":"error","status":401,"error":{"type":"authentication_error","message":"Session expired"}})).await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    for lane in ["a", "b"] {
        send_json(
            &mut client,
            json!({"type":"response.create","stream_id":lane,"model":"test-model"}),
        )
        .await;
    }
    assert_eq!(receive_json(&mut client).await["status"], 401);
    let _ = client.next().await;
    gateway.await.unwrap();
    upstream.await.unwrap();
    let details = recorded_details(&context);
    assert_eq!(details.len(), 2);
    for detail in details {
        assert_eq!(
            detail.summary.stream_outcome,
            Some(GatewayStreamOutcome::Failed)
        );
        assert_eq!(
            detail.summary.error_message.as_deref(),
            Some("Session expired")
        );
        assert_eq!(detail.websocket.unwrap().error_status, Some(401));
        assert!(detail
            .upstream_response_body
            .unwrap()
            .contains("Session expired"));
        assert!(detail.response_body.unwrap().contains("Session expired"));
    }
}

/// A relay that pins a Responses WebSocket to its first model rejects a later
/// model switch with a 400. Codex reuses one socket for a whole session and
/// reads that 400 as terminal, so the switch never lands — the dead turn behind
/// issue #420. The relay must deliver a code Codex retries and close the socket,
/// so its own reconnect dials a connection the relay has not locked.
#[tokio::test]
async fn upstream_model_lock_error_is_rewritten_and_closes_the_connection() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    // Single mode is where the CLI carries the real model, so a switch reaches
    // the relay as reported in issue #420. Failover mode maps the request onto
    // the provider's default model, so the relay need never see a switch at all.
    let manifest_path =
        ProxyGatewayPaths::new(directory.path()).manifest_path(GatewayCliKey::Codex);
    std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(
            &crate::coding::proxy_gateway::cli_proxy::manifest::CliProxyManifest::new(
                GatewayCliKey::Codex,
                "http://127.0.0.1:37123".to_string(),
                "2026-10-10T00:00:00Z".to_string(),
                GatewayProxyMode::Single,
                provider_id,
            ),
        )
        .unwrap(),
    )
    .unwrap();
    crate::coding::proxy_gateway::runtime::providers::clear_gateway_provider_selection_cache();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        // The first turn selects the channel and pins the connection.
        let first = receive_json(&mut socket).await;
        assert_eq!(first["model"], "test-model");
        send_json(
            &mut socket,
            json!({"type":"response.created","response":{"id":"resp-1"}}),
        )
        .await;
        send_json(
            &mut socket,
            json!({"type":"response.completed","response":{"id":"resp-1","usage":{"input_tokens":1,"output_tokens":1}}}),
        )
        .await;
        // The switch is refused, exactly as the relay's lock does.
        let second = receive_json(&mut socket).await;
        assert_eq!(second["model"], "other-model");
        send_json(
            &mut socket,
            json!({
                "type": "error",
                "status": 400,
                "error": {
                    "type": "new_api_error",
                    "code": "invalid_request",
                    "message": "responses websocket connection is locked to model \"test-model\", but the request is for \"other-model\"",
                },
            }),
        )
        .await;
        // The gateway must close its side after delivering the rewrite.
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;

    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model","input":"hello"}),
    )
    .await;
    assert_eq!(receive_json(&mut client).await["type"], "response.created");
    assert_eq!(
        receive_json(&mut client).await["type"],
        "response.completed"
    );

    send_json(
        &mut client,
        json!({"type":"response.create","model":"other-model","input":"hello"}),
    )
    .await;
    let error = receive_json(&mut client).await;
    assert_eq!(error["type"], "error");
    assert_eq!(
        error["error"]["code"], "websocket_connection_limit_reached",
        "the lock must arrive as a code Codex retries and reconnects on: {error}"
    );
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("locked to model")),
        "the upstream message — it names the locked model — must survive: {error}"
    );
    assert!(
        matches!(
            timeout(Duration::from_secs(5), client.next())
                .await
                .unwrap(),
            Some(Ok(Message::Close(_)))
        ),
        "the socket must close so the retry is not refused by the same lock"
    );
    gateway.await.unwrap();
    upstream.await.unwrap();

    // The switched turn is still billed, under its own category.
    let detail = recorded_details(&context)
        .into_iter()
        .find(|detail| detail.summary.requested_model.as_deref() == Some("other-model"))
        .expect("the switched turn must be recorded");
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Failed)
    );
    assert_eq!(
        detail.summary.error_category.as_deref(),
        Some("websocket_model_locked")
    );
    assert!(!detail.summary.success);
}

/// The lock is a property of the connection, not of the model. Scoring it would
/// cool the model down for something no request against it could fix, so the
/// turn must leave the health registry untouched.
#[tokio::test]
async fn a_locked_model_turn_does_not_penalize_model_health() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        receive_json(&mut socket).await;
        send_json(
            &mut socket,
            json!({
                "type": "error",
                "status": 400,
                "error": {
                    "type": "new_api_error",
                    "code": "invalid_request",
                    "message": "responses websocket connection is locked to model \"test-model\"",
                },
            }),
        )
        .await;
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    send_json(
        &mut client,
        json!({"type":"response.create","model":"test-model","input":"hello"}),
    )
    .await;
    assert_eq!(
        receive_json(&mut client).await["error"]["code"],
        "websocket_connection_limit_reached"
    );
    assert!(matches!(
        timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap(),
        Some(Ok(Message::Close(_)))
    ));
    gateway.await.unwrap();
    upstream.await.unwrap();

    // The turn ran and was recorded ...
    let detail = recorded_details(&context).pop().unwrap();
    assert_eq!(
        detail.summary.error_category.as_deref(),
        Some("websocket_model_locked")
    );

    // ... and the model carries no blame for it.
    let registry = context.health_registry.as_ref().unwrap().lock().unwrap();
    let key = ProviderModelHealthKey {
        cli_key: GatewayCliKey::Codex,
        provider_id,
        upstream_model_id: "test-model".to_string(),
    };
    assert!(
        registry.model_entry(&key).is_none(),
        "a connection-level lock must not create a model-health entry"
    );
}

/// The rewrite keys off the relay's message text, because every rejection shares
/// the generic `invalid_request` code. A different 400 must travel byte-identical
/// and keep the socket open — and must still count against the model, so the
/// exemption above stays a deliberate exception rather than a blanket one.
#[tokio::test]
async fn an_unrelated_400_error_is_forwarded_unchanged() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, provider_id) = test_context(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "openai_responses",
        true,
    );
    let unrelated = json!({
        "type": "error",
        "status": 400,
        "error": {
            "type": "invalid_request_error",
            "code": "invalid_request",
            "message": "Unsupported parameter: foo",
        },
    });
    let upstream_events = unrelated.clone();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        // Two turns: the second request reaching this stub at all is what proves
        // the rejection did not close the socket.
        for _ in 0..2 {
            receive_json(&mut socket).await;
            send_json(&mut socket, upstream_events.clone()).await;
        }
        let _ = socket.next().await;
    });
    let (mut client, gateway) = gateway_connection(context.clone()).await;
    for _ in 0..2 {
        send_json(
            &mut client,
            json!({"type":"response.create","model":"test-model","input":"hello"}),
        )
        .await;
        assert_eq!(
            receive_json(&mut client).await,
            unrelated,
            "a non-lock 400 must reach the client untouched"
        );
    }
    let _ = client.close(None).await;
    gateway.await.unwrap();
    upstream.await.unwrap();

    let detail = recorded_details(&context)
        .into_iter()
        .find(|detail| detail.summary.error_category.as_deref() == Some("upstream_error"))
        .expect("the rejected turn must be recorded as a plain upstream error");
    assert_eq!(
        detail.summary.stream_outcome,
        Some(GatewayStreamOutcome::Failed)
    );
    // It still counts against the model — that is what makes the exemption above
    // a deliberate exception rather than a path that never scores anything.
    let registry = context.health_registry.as_ref().unwrap().lock().unwrap();
    let key = ProviderModelHealthKey {
        cli_key: GatewayCliKey::Codex,
        provider_id,
        upstream_model_id: "test-model".to_string(),
    };
    let entry = registry
        .model_entry(&key)
        .expect("an ordinary 400 must still score against the model");
    assert_eq!(
        entry.last_error_category.as_deref(),
        Some("upstream_bad_request")
    );
}
