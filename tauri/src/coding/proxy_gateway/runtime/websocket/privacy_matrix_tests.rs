use super::privacy_tests::{enable_privacy, token, upstream_json, SECRET};
use super::*;

const PROTOCOLS: [&str; 4] = [
    "anthropic_messages",
    "openai_chat",
    "openai_responses",
    "gemini_native",
];

fn incoming_request(protocol: &str, streaming: bool) -> Value {
    let schema = json!({"type":"object","properties":{"value":{"type":"string","description":SECRET}},"required":["value"]});
    match protocol {
        "anthropic_messages" => {
            json!({"model":"test-model","max_tokens":1024,"stream":streaming,"system":SECRET,"messages":[{"role":"user","content":SECRET},{"role":"assistant","content":[{"type":"tool_use","id":"previous-tool","name":"write_file","input":{"value":SECRET}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"previous-tool","content":json!({"value":SECRET}).to_string()}]}],"tools":[{"name":"write_file","description":SECRET,"input_schema":schema}]})
        }
        "openai_chat" => {
            json!({"model":"test-model","stream":streaming,"messages":[{"role":"system","content":SECRET},{"role":"user","content":SECRET},{"role":"assistant","tool_calls":[{"id":"previous-tool","type":"function","function":{"name":"write_file","arguments":json!({"value":SECRET}).to_string()}}]},{"role":"tool","tool_call_id":"previous-tool","content":json!({"value":SECRET}).to_string()}],"tools":[{"type":"function","function":{"name":"write_file","description":SECRET,"parameters":schema}}]})
        }
        "openai_responses" => {
            json!({"model":"test-model","stream":streaming,"instructions":SECRET,"input":[{"role":"user","content":SECRET},{"type":"function_call","id":"previous-item","call_id":"previous-tool","name":"write_file","arguments":json!({"value":SECRET}).to_string()},{"type":"function_call_output","call_id":"previous-tool","output":json!({"value":SECRET}).to_string()}],"tools":[{"type":"function","name":"write_file","description":SECRET,"parameters":schema}]})
        }
        "gemini_native" => {
            json!({"systemInstruction":{"parts":[{"text":SECRET}]},"contents":[{"role":"user","parts":[{"text":SECRET}]},{"role":"model","parts":[{"functionCall":{"name":"write_file","args":{"value":SECRET}}}]},{"role":"user","parts":[{"functionResponse":{"name":"write_file","response":{"value":SECRET}}}]}],"tools":[{"functionDeclarations":[{"name":"write_file","description":SECRET,"parameters":schema}]}]})
        }
        _ => unreachable!(),
    }
}

fn upstream_sse(protocol: &str, echoed: &str) -> String {
    let mut events = Vec::new();
    let arguments = json!({"value":echoed}).to_string();
    match protocol {
        "anthropic_messages" => {
            events.push(json!({"type":"message_start","message":{"id":"msg-matrix","type":"message","role":"assistant","model":"test-model","content":[],"usage":{"input_tokens":12,"output_tokens":0}}}));
            events.push(json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}));
            for character in echoed.chars() {
                events.push(json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":character.to_string()}}));
            }
            events.push(json!({"type":"content_block_stop","index":0}));
            events.push(json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool_1","name":"write_file","input":{}}}));
            for character in arguments.chars() {
                events.push(json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":character.to_string()}}));
            }
            events.push(json!({"type":"content_block_stop","index":1}));
            events.push(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}));
            events.push(json!({"type":"message_stop"}));
        }
        "openai_chat" => {
            events.push(json!({"id":"chat-matrix","model":"test-model","choices":[{"index":0,"delta":{"role":"assistant"}}]}));
            for character in echoed.chars() {
                events.push(json!({"id":"chat-matrix","choices":[{"index":0,"delta":{"content":character.to_string()}}]}));
            }
            events.push(json!({"id":"chat-matrix","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"tool_1","type":"function","function":{"name":"write_file","arguments":""}}]}}]}));
            for character in arguments.chars() {
                events.push(json!({"id":"chat-matrix","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":character.to_string()}}]}}]}));
            }
            events.push(json!({"id":"chat-matrix","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":12,"completion_tokens":3}}));
        }
        "openai_responses" => {
            events.push(json!({"type":"response.created","response":{"id":"resp-matrix","model":"test-model","status":"in_progress","output":[]}}));
            events.push(json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"message_1","role":"assistant","content":[]}}));
            events.push(json!({"type":"response.content_part.added","item_id":"message_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":""}}));
            for character in echoed.chars() {
                events.push(json!({"type":"response.output_text.delta","item_id":"message_1","output_index":0,"content_index":0,"delta":character.to_string()}));
            }
            events.push(json!({"type":"response.output_text.done","item_id":"message_1","output_index":0,"content_index":0,"text":echoed}));
            events.push(json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"function_1","call_id":"tool_1","name":"write_file","arguments":""}}));
            for character in arguments.chars() {
                events.push(json!({"type":"response.function_call_arguments.delta","item_id":"function_1","output_index":1,"delta":character.to_string()}));
            }
            events.push(json!({"type":"response.function_call_arguments.done","item_id":"function_1","output_index":1,"arguments":arguments}));
            let mut response = upstream_json(protocol, echoed);
            response["id"] = json!("resp-matrix");
            events.push(json!({"type":"response.completed","response":response}));
        }
        "gemini_native" => {
            for character in echoed.chars() {
                events.push(json!({"responseId":"gemini-matrix","candidates":[{"index":0,"content":{"role":"model","parts":[{"text":character.to_string()}]}}]}));
            }
            events.push(json!({"responseId":"gemini-matrix","candidates":[{"index":0,"content":{"role":"model","parts":[{"functionCall":{"name":"write_file","args":{"value":echoed}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":3,"totalTokenCount":15}}));
        }
        _ => unreachable!(),
    }
    let mut wire = events
        .into_iter()
        .map(|event| {
            let event_name = event["type"]
                .as_str()
                .map(|name| format!("event: {name}\n"))
                .unwrap_or_default();
            format!("{event_name}data: {event}\n\n")
        })
        .collect::<String>();
    if protocol == "openai_chat" {
        wire.push_str("data: [DONE]\n\n");
    }
    wire
}

fn assert_json_response(protocol: &str, body: &str, case: &str) {
    let context = format!("{case}: {body}");
    let case = context.as_str();
    let response: Value = serde_json::from_str(body).expect(case);
    let (text, arguments) = match protocol {
        "anthropic_messages" => {
            let blocks = response["content"].as_array().expect(case);
            (
                blocks
                    .iter()
                    .find(|block| block["type"] == "text")
                    .expect(case)["text"]
                    .clone(),
                blocks
                    .iter()
                    .find(|block| block["type"] == "tool_use")
                    .expect(case)["input"]
                    .clone(),
            )
        }
        "openai_chat" => {
            let message = &response["choices"][0]["message"];
            let text = if let Some(parts) = message["content"].as_array() {
                json!(parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<String>())
            } else {
                message["content"].clone()
            };
            (
                text,
                serde_json::from_str(
                    message["tool_calls"][0]["function"]["arguments"]
                        .as_str()
                        .expect(case),
                )
                .expect(case),
            )
        }
        "openai_responses" => {
            let output = response["output"].as_array().expect(case);
            (
                output
                    .iter()
                    .find(|item| item["type"] == "message")
                    .expect(case)["content"][0]["text"]
                    .clone(),
                serde_json::from_str(
                    output
                        .iter()
                        .find(|item| item["type"] == "function_call")
                        .expect(case)["arguments"]
                        .as_str()
                        .expect(case),
                )
                .expect(case),
            )
        }
        "gemini_native" => {
            let parts = response["candidates"][0]["content"]["parts"]
                .as_array()
                .expect(case);
            (
                parts
                    .iter()
                    .find(|part| part["text"].is_string())
                    .expect(case)["text"]
                    .clone(),
                parts
                    .iter()
                    .find(|part| part["functionCall"].is_object())
                    .expect(case)["functionCall"]["args"]
                    .clone(),
            )
        }
        _ => unreachable!(),
    };
    assert_eq!(text, SECRET, "{case}: response text");
    assert_eq!(arguments, json!({"value":SECRET}), "{case}: tool arguments");
}

fn assert_stream_response(protocol: &str, body: &str, case: &str) {
    let mut text = String::new();
    let mut arguments = String::new();
    let mut gemini_arguments = Value::Null;
    for event in body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
    {
        match protocol {
            "anthropic_messages" => {
                text.push_str(
                    event
                        .pointer("/delta/text")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                arguments.push_str(
                    event
                        .pointer("/delta/partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
            }
            "openai_chat" => {
                text.push_str(
                    event
                        .pointer("/choices/0/delta/content")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                arguments.push_str(
                    event
                        .pointer("/choices/0/delta/tool_calls/0/function/arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
            }
            "openai_responses" => match event["type"].as_str() {
                Some("response.output_text.delta") => {
                    text.push_str(event["delta"].as_str().expect(case))
                }
                Some("response.function_call_arguments.delta") => {
                    arguments.push_str(event["delta"].as_str().expect(case))
                }
                _ => {}
            },
            "gemini_native" => {
                for part in event
                    .pointer("/candidates/0/content/parts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    text.push_str(part["text"].as_str().unwrap_or_default());
                    if let Some(value) = part.pointer("/functionCall/args") {
                        gemini_arguments = value.clone();
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(text, SECRET, "{case}: streamed text");
    let arguments: Value = if protocol == "gemini_native" {
        gemini_arguments
    } else {
        serde_json::from_str(&arguments).expect(case)
    };
    assert_eq!(
        arguments,
        json!({"value":SECRET}),
        "{case}: streamed tool arguments"
    );
}

const CLI_ENTRIES: [(GatewayCliKey, &str, &str); 10] = [
    (
        GatewayCliKey::Claude,
        "anthropic_messages",
        "/anthropic/v1/messages",
    ),
    (
        GatewayCliKey::ClaudeDesktop,
        "anthropic_messages",
        "/claude-desktop/v1/messages",
    ),
    (
        GatewayCliKey::Codex,
        "openai_responses",
        "/openai/v1/responses",
    ),
    (
        GatewayCliKey::Codex,
        "openai_chat",
        "/openai/v1/chat/completions",
    ),
    (
        GatewayCliKey::Grok,
        "openai_responses",
        "/grok/v1/responses",
    ),
    (
        GatewayCliKey::Kimi,
        "openai_chat",
        "/kimi/v1/chat/completions",
    ),
    (
        GatewayCliKey::Gemini,
        "gemini_native",
        "/gemini/v1beta/models/test-model:generateContent",
    ),
    (
        GatewayCliKey::Antigravity,
        "gemini_native",
        "/antigravity/v1beta/models/test-model:generateContent",
    ),
    (
        GatewayCliKey::Workbuddy,
        "openai_chat",
        "/workbuddy/v1/chat/completions",
    ),
    (
        GatewayCliKey::Codebuddy,
        "openai_chat",
        "/codebuddy/v1/chat/completions",
    ),
];

#[tokio::test]
async fn privacy_all_cli_entries_and_protocols_round_trip_json_sse_and_forced_sse() {
    assert_eq!(
        CLI_ENTRIES
            .iter()
            .map(|(cli, _, _)| *cli)
            .collect::<std::collections::HashSet<_>>(),
        GatewayCliKey::supported_mvp().into_iter().collect()
    );
    for (cli, source, path) in CLI_ENTRIES {
        for target in PROTOCOLS {
            for enabled in [true, false] {
                for (mode, client_streaming, upstream_streaming) in [
                    ("json", false, false),
                    ("sse", true, true),
                    ("forced_sse", false, true),
                ] {
                    let case = format!("{cli:?}/{source} -> {target}, {mode}, privacy={enabled}");
                    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                    let (_directory, context, _) = test_context_for_cli(
                        cli,
                        &format!("http://{}", listener.local_addr().unwrap()),
                        target,
                        true,
                    );
                    context.settings.write().unwrap().log_max_body_size_kb = 64;
                    if enabled {
                        enable_privacy(&context);
                    }
                    let upstream_case = case.clone();
                    let upstream = tokio::spawn(async move {
                        let (mut socket, _) = timeout(Duration::from_secs(30), listener.accept())
                            .await
                            .expect("upstream connection")
                            .unwrap();
                        let request =
                            super::super::super::http_io::read_http_request(&mut socket, 0)
                                .await
                                .unwrap();
                        let echoed = if enabled {
                            token(&request.body)
                        } else {
                            assert!(
                                !String::from_utf8_lossy(&request.body).contains("__AITB_PRIV_"),
                                "{upstream_case}"
                            );
                            SECRET.to_string()
                        };
                        let payload: Value = serde_json::from_slice(&request.body).unwrap();
                        let root = match target {
                            "openai_responses" => "input",
                            "gemini_native" => "contents",
                            _ => "messages",
                        };
                        assert!(
                            payload[root].is_array(),
                            "{upstream_case}: upstream protocol shape"
                        );
                        assert!(
                            request.headers.iter().any(|(name, value)| [
                                "authorization",
                                "x-api-key",
                                "x-goog-api-key"
                            ]
                            .iter()
                            .any(|candidate| name.eq_ignore_ascii_case(candidate))
                                && value.ends_with("upstream-test-key")),
                            "{upstream_case}: upstream auth"
                        );
                        let (content_type, body) = if upstream_streaming {
                            ("text/event-stream", upstream_sse(target, &echoed))
                        } else {
                            (
                                "application/json",
                                upstream_json(target, &echoed).to_string(),
                            )
                        };
                        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    });
                    let (address, gateway) = start_gateway(context.clone(), 1).await;
                    let path = if matches!(cli, GatewayCliKey::Gemini | GatewayCliKey::Antigravity)
                        && client_streaming
                    {
                        path.replace(":generateContent", ":streamGenerateContent?alt=sse")
                    } else {
                        path.to_string()
                    };
                    let response = crate::http_client::create_client_no_proxy(30)
                        .unwrap()
                        .post(format!("http://{address}{path}"))
                        .header("Authorization", "Bearer client-test-key")
                        .header("x-ai-toolbox-session-id", "matrix-session")
                        .json(&incoming_request(source, client_streaming))
                        .send()
                        .await
                        .expect(&case);
                    let status = response.status();
                    let body = response.text().await.expect(&case);
                    assert_eq!(status, 200, "{case}: {body}");
                    assert!(
                        !body.contains("__AITB_PRIV_"),
                        "{case}: client received a placeholder"
                    );
                    if client_streaming {
                        assert_stream_response(source, &body, &case);
                    } else {
                        assert_json_response(source, &body, &case);
                    }
                    gateway.await.unwrap();
                    upstream.await.unwrap();
                    let details = recorded_details(&context);
                    assert_eq!(details.len(), 1, "{case}");
                    let detail = &details[0];
                    assert_eq!(
                        detail.summary.cli_key,
                        Some(cli.into()),
                        "{case}: recorded CLI"
                    );
                    assert!(
                        detail.summary.success,
                        "{case}: {:?}",
                        detail.summary.error_message
                    );
                    assert_eq!(detail.summary.total_tokens, Some(15), "{case}: usage");
                    assert_eq!(detail.privacy.is_some(), enabled, "{case}: policy state");
                    if enabled {
                        assert!(
                            detail.privacy.as_ref().unwrap().restored_values > 0,
                            "{case}"
                        );
                        assert!(
                            !serde_json::to_string(detail).unwrap().contains("secret"),
                            "{case}: log leaked text"
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn privacy_http_errors_use_each_clients_protocol_envelope() {
    for (cli, source, path) in CLI_ENTRIES {
        for phase in ["invalid_json", "unknown_placeholder", "signed_history"] {
            if phase == "signed_history"
                && !matches!(source, "anthropic_messages" | "gemini_native")
            {
                continue;
            }
            let case = format!("{cli:?}/{source}: {phase}");
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let (_directory, context, _) = test_context_for_cli(
                cli,
                &format!("http://{}", listener.local_addr().unwrap()),
                source,
                true,
            );
            enable_privacy(&context);
            let upstream = if phase == "unknown_placeholder" {
                Some(tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    super::super::super::http_io::read_http_request(&mut socket, 0)
                        .await
                        .unwrap();
                    let body = upstream_json(source, "__AITB_PRIV_unknown__").to_string();
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                }))
            } else {
                None
            };
            let request_body = match phase {
                "invalid_json" => "invalid JSON".to_string(),
                "signed_history" if source == "anthropic_messages" => json!({"model":"test-model","max_tokens":100,"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":SECRET,"signature":"opaque-signature"}]}]}).to_string(),
                "signed_history" => json!({"contents":[{"role":"model","parts":[{"text":SECRET,"thought":true,"thoughtSignature":"opaque-signature"}]}]}).to_string(),
                _ => incoming_request(source, false).to_string(),
            };
            let (address, gateway) = start_gateway(context.clone(), 1).await;
            let response = crate::http_client::create_client_no_proxy(30)
                .unwrap()
                .post(format!("http://{address}{path}"))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send()
                .await
                .expect(&case);
            let status = if phase == "unknown_placeholder" {
                502
            } else {
                400
            };
            assert_eq!(response.status(), status, "{case}");
            let body: Value = response.json().await.expect(&case);
            assert!(
                body.pointer("/error/message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| !message.is_empty()),
                "{case}: {body}"
            );
            if source == "anthropic_messages" {
                assert_eq!(body["type"], "error", "{case}");
            }
            if source == "gemini_native" {
                assert_eq!(body["error"]["code"], status, "{case}");
            }
            gateway.await.unwrap();
            if let Some(upstream) = upstream {
                upstream.await.unwrap();
            }
            assert!(
                context
                    .health_items()
                    .unwrap_or_default()
                    .iter()
                    .all(|item| item.failure_score == 0),
                "{case}: local privacy failure changed provider health"
            );
        }
    }
}

#[tokio::test]
async fn privacy_all_cli_entries_restore_ollama_json_and_ndjson() {
    for (cli, source, path) in CLI_ENTRIES {
        for streaming in [false, true] {
            for enabled in [false, true] {
                let case =
                    format!("{cli:?}/{source} -> Ollama, streaming={streaming}, privacy={enabled}");
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let (_directory, context, _) = test_context_for_cli(
                    cli,
                    &format!("http://{}", listener.local_addr().unwrap()),
                    "ollama/chat",
                    true,
                );
                context.settings.write().unwrap().log_max_body_size_kb = 64;
                if enabled {
                    enable_privacy(&context);
                }
                let upstream = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let request = super::super::super::http_io::read_http_request(&mut socket, 0)
                        .await
                        .unwrap();
                    assert_eq!(request.path.split('?').next(), Some("/api/chat"));
                    let payload: Value = serde_json::from_slice(&request.body).unwrap();
                    assert_eq!(payload["tools"][0]["function"]["name"], "write_file");
                    let history_call = payload["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find_map(|message| message.pointer("/tool_calls/0/function"))
                        .expect("Ollama tool history retained");
                    assert!(
                        history_call["arguments"].is_object(),
                        "Ollama arguments must be objects"
                    );
                    let echoed = if enabled {
                        token(&request.body)
                    } else {
                        SECRET.to_string()
                    };
                    let tool =
                        json!({"function":{"name":"write_file","arguments":{"value":echoed}}});
                    let response = json!({"model":"test-model","message":{"role":"assistant","content":echoed,"tool_calls":[tool]},"done":true,"done_reason":"stop","prompt_eval_count":12,"eval_count":3});
                    let (content_type, body) = if streaming {
                        let mut body = echoed.chars().map(|character| format!("{}\n", json!({"model":"test-model","message":{"role":"assistant","content":character.to_string()},"done":false}))).collect::<String>();
                        let mut terminal = response;
                        terminal["message"]["content"] = json!("");
                        body.push_str(&terminal.to_string());
                        body.push('\n');
                        ("application/x-ndjson", body)
                    } else {
                        ("application/json", response.to_string())
                    };
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                });
                let (address, gateway) = start_gateway(context.clone(), 1).await;
                let path = if matches!(cli, GatewayCliKey::Gemini | GatewayCliKey::Antigravity)
                    && streaming
                {
                    path.replace(":generateContent", ":streamGenerateContent?alt=sse")
                } else {
                    path.to_string()
                };
                let response = crate::http_client::create_client_no_proxy(30)
                    .unwrap()
                    .post(format!("http://{address}{path}"))
                    .json(&incoming_request(source, streaming))
                    .send()
                    .await
                    .expect(&case);
                let status = response.status();
                let body = response.text().await.expect(&case);
                assert_eq!(status, 200, "{case}: {body}");
                assert!(!body.contains("__AITB_PRIV_"), "{case}");
                if streaming {
                    assert_stream_response(source, &body, &case);
                } else {
                    assert_json_response(source, &body, &case);
                }
                gateway.await.unwrap();
                upstream.await.unwrap();
                let details = recorded_details(&context);
                assert_eq!(details.len(), 1, "{case}");
                assert!(details[0].summary.success, "{case}");
                assert_eq!(details[0].summary.input_tokens, Some(12), "{case}");
                assert_eq!(details[0].summary.output_tokens, Some(3), "{case}");
                assert_eq!(details[0].summary.total_tokens, Some(15), "{case}");
            }
        }
    }
}

#[tokio::test]
async fn privacy_legacy_completion_route_restores_json_and_sse() {
    for streaming in [false, true] {
        for enabled in [false, true] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let (_directory, context, _) = test_context(
                &format!("http://{}", listener.local_addr().unwrap()),
                "openai_chat",
                true,
            );
            if enabled {
                enable_privacy(&context);
            }
            let upstream = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = super::super::super::http_io::read_http_request(&mut socket, 0)
                    .await
                    .unwrap();
                assert_eq!(request.path, "/v1/completions");
                let echoed = if enabled {
                    token(&request.body)
                } else {
                    SECRET.to_string()
                };
                let (content_type, body) = if streaming {
                    let mut body = echoed.chars().map(|character| format!("data: {}\n\n", json!({"id":"completion","choices":[{"index":0,"text":character.to_string()}]}))).collect::<String>();
                    body.push_str("data: {\"id\":\"completion\",\"choices\":[{\"index\":0,\"text\":\"\",\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n");
                    ("text/event-stream", body)
                } else {
                    ("application/json", json!({"id":"completion","choices":[{"index":0,"text":echoed,"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3}}).to_string())
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let (address, gateway) = start_gateway(context.clone(), 1).await;
            let response = crate::http_client::create_client_no_proxy(10)
                .unwrap()
                .post(format!("http://{address}/openai/v1/completions"))
                .json(&json!({"model":"test-model","prompt":SECRET,"stream":streaming}))
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body = response.text().await.unwrap();
            assert_eq!(
                status, 200,
                "legacy completion: streaming={streaming}, privacy={enabled}, body={body}"
            );
            let text = if streaming {
                body.lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .filter_map(|data| serde_json::from_str::<Value>(data).ok())
                    .map(|value| {
                        value["choices"][0]["text"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string()
                    })
                    .collect::<String>()
            } else {
                serde_json::from_str::<Value>(&body).unwrap()["choices"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            assert_eq!(text, SECRET);
            gateway.await.unwrap();
            upstream.await.unwrap();
        }
    }
}

#[tokio::test]
async fn privacy_enabled_websocket_conversion_falls_back_to_protected_http() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (_directory, context, _) = test_context(
        &format!("http://{}", listener.local_addr().unwrap()),
        "openai_chat",
        true,
    );
    enable_privacy(&context);
    let (address, gateway) = start_gateway(context.clone(), 2).await;
    assert_eq!(
        rejected_upgrade(address, "/openai/v1/responses")
            .await
            .status(),
        426
    );
    assert!(timeout(Duration::from_millis(25), listener.accept())
        .await
        .is_err());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = super::super::super::http_io::read_http_request(&mut socket, 0)
            .await
            .unwrap();
        let body = upstream_json("openai_chat", &token(&request.body)).to_string();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    let response = crate::http_client::create_client_no_proxy(10)
        .unwrap()
        .post(format!("http://{address}/openai/v1/responses"))
        .json(&incoming_request("openai_responses", false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_json_response(
        "openai_responses",
        &response.text().await.unwrap(),
        "privacy fallback",
    );
    gateway.await.unwrap();
    upstream.await.unwrap();
    assert_eq!(context.requests_per_minute(), 1);
}

#[tokio::test]
async fn privacy_ollama_stream_failures_do_not_deliver_a_success_terminal() {
    for upstream_error in [false, true] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let (_directory, context, _) = test_context(
            &format!("http://{}", listener.local_addr().unwrap()),
            "ollama/chat",
            true,
        );
        enable_privacy(&context);
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = super::super::super::http_io::read_http_request(&mut socket, 0)
                .await
                .unwrap();
            let mut body = format!(
                "{}\n",
                json!({"model":"test-model","message":{"role":"assistant","content":token(&request.body)},"done":false})
            );
            if upstream_error {
                body.push_str("{\"error\":\"upstream failed\"}\n");
            }
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let (address, gateway) = start_gateway(context.clone(), 1).await;
        let response = crate::http_client::create_client_no_proxy(10)
            .unwrap()
            .post(format!("http://{address}/openai/v1/responses"))
            .json(&incoming_request("openai_responses", true))
            .send()
            .await
            .unwrap();
        let body = response.text().await.unwrap();
        assert!(!body.contains("response.completed"), "{body}");
        assert!(
            body.contains("response.failed") || body.contains("upstream_stream_first_chunk_failed"),
            "{body}"
        );
        gateway.await.unwrap();
        upstream.await.unwrap();
        assert!(!recorded_details(&context)[0].summary.success);
    }
}
