use super::*;
use futures_util::StreamExt;
use serde_json::json;

fn runtime() -> PrivacyRuntime {
    let runtime = PrivacyRuntime::new(None);
    let mut settings = PrivacySettings::default();
    settings.enabled = true;
    settings.rules.custom.push(PrivacyCustomRule {
        id: "test".into(),
        name: "Test".into(),
        enabled: true,
        kind: PrivacyRuleKind::Literal,
        pattern: "secret\"line\nvalue".into(),
        priority: 200,
    });
    settings.rules.builtins.push("email".into());
    runtime.publish(CompiledPolicy::compile(&settings).unwrap());
    runtime
}

fn request(runtime: &PrivacyRuntime, session: &str) -> PrivacyRequest {
    runtime
        .begin(
            "codex",
            &[("session_id".into(), session.into())],
            b"{}",
            None,
        )
        .unwrap()
        .unwrap()
}

#[test]
fn disabled_accepts_unparsed_payload_without_mapping() {
    let runtime = PrivacyRuntime::new(None);
    assert!(runtime
        .begin("codex", &[], b"invalid JSON", None)
        .unwrap()
        .is_none());
    assert!(runtime.mappings.lock().unwrap().sessions.is_empty());
}

#[test]
fn paths_preserve_protocol_fields_but_scan_business_name_url_and_nested_arguments() {
    let runtime = runtime();
    let request = request(&runtime, "one");
    let value = json!({"model":"model@example.com", "tools":[{"type":"function","name":"tool@example.com","description":"mail user@example.com"}], "input":[{"type":"function_call","name":"tool@example.com","call_id":"call@example.com","arguments":json!({"name":"user@example.com","url":"https://site/user@example.com","value":"secret\"line\nvalue"}).to_string()}]});
    let body = serde_json::to_vec(&value).unwrap();
    let redacted = request.prepare(&body, "p").unwrap();
    let masked: Value = serde_json::from_slice(&redacted).unwrap();
    assert_eq!(masked["model"], value["model"]);
    assert_eq!(masked["tools"][0]["name"], value["tools"][0]["name"]);
    assert_eq!(masked["input"][0]["call_id"], value["input"][0]["call_id"]);
    assert!(!masked["input"][0]["arguments"]
        .as_str()
        .unwrap()
        .contains("user@example.com"));
    let restored: Value = serde_json::from_slice(&request.restore(&redacted).unwrap()).unwrap();
    assert_eq!(restored, value);
}

#[test]
fn no_matches_preserve_original_bytes_and_signed_matches_fail() {
    let request = request(&runtime(), "one");
    let body = b"{ \"model\": \"x\", \"input\": \"hello {\" }";
    assert_eq!(request.prepare(body, "p").unwrap(), body);
    let signed = json!({"contents":[{"role":"model","parts":[{"text":"user@example.com","thoughtSignature":"opaque"}]}]});
    assert!(request
        .prepare(&serde_json::to_vec(&signed).unwrap(), "p")
        .unwrap_err()
        .starts_with("privacy_signed_payload"));
}

#[test]
fn sessions_retries_previous_responses_and_disable_keep_correct_mapping_lifetime() {
    let runtime = runtime();
    let first = request(&runtime, "one");
    let body = br#"{"input":"user@example.com"}"#;
    let masked = first.prepare(body, "p").unwrap();
    assert_eq!(first.prepare(body, "fallback").unwrap(), masked);
    first.remember_response(&json!({"id":"resp_1"}));
    let second = request(&runtime, "one");
    assert_eq!(second.prepare(body, "p").unwrap(), masked);
    assert_ne!(request(&runtime, "two").prepare(body, "p").unwrap(), masked);
    let previous = runtime
        .begin(
            "codex",
            &[("session_id".into(), "one".into())],
            br#"{"previous_response_id":"resp_1"}"#,
            None,
        )
        .unwrap()
        .unwrap();
    previous.prepare(b"{}", "fallback").unwrap();
    assert_eq!(previous.restore(&masked).unwrap(), body);
    runtime.publish(CompiledPolicy::compile(&PrivacySettings::default()).unwrap());
    assert_eq!(first.restore(&masked).unwrap(), body);
    assert!(runtime
        .begin("codex", &[], b"invalid", None)
        .unwrap()
        .is_none());
}

#[test]
fn exact_allowlist_and_priority_choose_one_reversible_value() {
    let mut config = PrivacySettings::default();
    config.rules.builtins.push("email".into());
    config.rules.allowlist.push("allowed@example.com".into());
    let result = preview(
        config.rules,
        "allowed@example.com hidden@example.com".into(),
    )
    .unwrap();
    assert!(result.redacted.starts_with("allowed@example.com "));
    assert_eq!(result.detail.matched_values, 1);
    assert_eq!(result.restored, "allowed@example.com hidden@example.com");
}

#[tokio::test]
async fn sse_restores_interleaved_tool_arguments_across_every_byte_boundary() {
    let request = request(&runtime(), "one");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"secret\"line\nvalue"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    let mut wire = String::new();
    let arguments = json!({"value":token}).to_string();
    for byte in arguments.bytes() {
        for index in [0, 1] {
            let value = json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":index,"function":{"arguments":(byte as char).to_string()}}]}}]});
            wire.push_str(&format!(
                "id: same\r\nevent: message\r\ndata: {value}\r\n\r\n"
            ));
        }
    }
    wire.push_str("data: [DONE]\r\n\r\n");
    let chunks = wire
        .as_bytes()
        .chunks(7)
        .map(|chunk| Ok(chunk.to_vec()))
        .collect::<Vec<_>>();
    let output = stream::restore_sse_stream(Box::pin(futures_util::stream::iter(chunks)), request)
        .collect::<Vec<_>>()
        .await;
    let output = String::from_utf8(
        output
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .concat(),
    )
    .unwrap();
    let mut collected = [String::new(), String::new()];
    for line in output
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
    {
        if line == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(line).unwrap();
        let call = &value["choices"][0]["delta"]["tool_calls"][0];
        collected[call["index"].as_u64().unwrap() as usize]
            .push_str(call["function"]["arguments"].as_str().unwrap());
    }
    for arguments in collected {
        assert_eq!(
            serde_json::from_str::<Value>(&arguments).unwrap(),
            json!({"value":"secret\"line\nvalue"})
        );
    }
    assert!(output.contains("id: same\r\nevent: message\r\n"));
    assert!(output.ends_with("data: [DONE]\r\n\r\n"));
}

#[test]
fn websocket_terminal_waits_for_restoration_and_keeps_ordinary_braces() {
    let request = request(&runtime(), "one");
    request.prepare(b"{}", "p").unwrap();
    let mut stream = stream::EventRestorer::new(request.clone());
    let plain = br#"{"type":"response.output_text.delta","item_id":"a","delta":"hello {"}"#;
    assert_eq!(stream.push_json(plain).unwrap(), vec![plain.to_vec()]);
    assert!(stream
        .push_json(
            br#"{"type":"response.output_text.delta","item_id":"a","delta":"__AITB_PRIV_missing"}"#
        )
        .unwrap()
        .is_empty());
    assert!(stream
        .push_json(br#"{"type":"response.completed","response":{"id":"r"}}"#)
        .is_err());
    assert!(request.detail().failed);
}

#[test]
fn independent_config_round_trip_and_invalid_save_leave_old_policy() {
    let db = SqliteDbState::in_memory_for_test().unwrap();
    let update = PrivacySettingsUpdate {
        enabled: Some(true),
        rules: None,
    };
    let (saved, _) = update_settings(&db, update).unwrap();
    assert!(load_settings(&db).unwrap().enabled);
    assert_eq!(
        crate::coding::proxy_gateway::settings::load_settings_from_sqlite_state(&db).unwrap(),
        crate::coding::proxy_gateway::types::ProxyGatewaySettings::default()
    );
    let mut invalid = saved.rules.clone();
    invalid.custom.push(PrivacyCustomRule {
        id: "invalid".into(),
        name: "Invalid".into(),
        enabled: true,
        kind: PrivacyRuleKind::Regex,
        pattern: "(".into(),
        priority: 0,
    });
    assert!(update_settings(
        &db,
        PrivacySettingsUpdate {
            enabled: None,
            rules: Some(invalid)
        }
    )
    .is_err());
    assert_eq!(load_settings(&db).unwrap(), saved);
}

#[test]
fn rule_updates_preserve_enabled_and_failed_writes_keep_saved_settings() {
    let db = SqliteDbState::in_memory_for_test().unwrap();
    update_settings(
        &db,
        PrivacySettingsUpdate {
            enabled: Some(true),
            rules: None,
        },
    )
    .unwrap();
    let mut rules = PrivacyRules::default();
    rules.allowlist.push(" allowed with spaces ".into());
    let (saved, policy) = update_settings(
        &db,
        PrivacySettingsUpdate {
            enabled: None,
            rules: Some(rules.clone()),
        },
    )
    .unwrap();
    assert!(saved.enabled);
    assert!(policy.enabled);
    assert_eq!(load_settings(&db).unwrap().rules, rules);
    db.with_conn(|connection| {
        connection
            .execute_batch("PRAGMA query_only = ON")
            .map_err(|error| error.to_string())
    })
    .unwrap();
    assert!(update_settings(
        &db,
        PrivacySettingsUpdate {
            enabled: Some(false),
            rules: None
        }
    )
    .is_err());
    assert_eq!(load_settings(&db).unwrap(), saved);
}

#[test]
fn policy_changes_are_isolated_from_existing_requests_and_previous_privacy_epochs() {
    let runtime = runtime();
    let first = request(&runtime, "one");
    let masked = first
        .prepare(br#"{"input":"user@example.com"}"#, "p")
        .unwrap();
    runtime.publish(CompiledPolicy::compile(&PrivacySettings::default()).unwrap());
    first.remember_response(&json!({"id":"old-epoch"}));
    let mut next = PrivacySettings::default();
    next.enabled = true;
    runtime.publish(CompiledPolicy::compile(&next).unwrap());
    let next = runtime
        .begin(
            "codex",
            &[("session_id".into(), "one".into())],
            br#"{"previous_response_id":"old-epoch"}"#,
            None,
        )
        .unwrap()
        .unwrap();
    assert!(next
        .prepare(b"{}", "p")
        .unwrap_err()
        .contains("privacy_history_missing"));
    assert_eq!(
        first.restore(&masked).unwrap(),
        br#"{"input":"user@example.com"}"#
    );
}

#[test]
fn mapping_limits_block_and_anonymous_requests_do_not_share_values() {
    let runtime = runtime();
    let first = runtime.begin("codex", &[], b"{}", None).unwrap().unwrap();
    let second = runtime.begin("codex", &[], b"{}", None).unwrap().unwrap();
    let body = br#"{"input":"user@example.com"}"#;
    assert_ne!(
        first.prepare(body, "p").unwrap(),
        second.prepare(body, "p").unwrap()
    );
    let values = (0..=MAX_MAPPINGS)
        .map(|index| format!("user{index}@example.com"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(first
        .prepare(&json!({"input":values}).to_string().into_bytes(), "p")
        .unwrap_err()
        .starts_with("privacy_mapping_limit"));
}

#[test]
fn builtin_private_key_and_password_fields_round_trip_without_invalid_json() {
    let request = request(&runtime(), "one");
    let private_key = "-----BEGIN PRIVATE KEY-----\nabc123\n-----END PRIVATE KEY-----";
    let body = json!({"input":[{"type":"function_call","name":"write_file","arguments":json!({"password":"short", "name":private_key, "url":"postgres://user:secret@db.example/db"}).to_string()}]});
    let masked = request
        .prepare(&serde_json::to_vec(&body).unwrap(), "p")
        .unwrap();
    assert!(!String::from_utf8_lossy(&masked).contains("abc123"));
    assert!(!String::from_utf8_lossy(&masked).contains("short"));
    assert_eq!(
        serde_json::from_slice::<Value>(&request.restore(&masked).unwrap()).unwrap(),
        body
    );
}

#[tokio::test]
async fn sse_and_websocket_cover_anthropic_gemini_responses_and_flattened_events() {
    let request = request(&runtime(), "one");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"user@example.com"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    for protocol in ["anthropic", "gemini", "responses"] {
        let mut restore = stream::EventRestorer::new(request.clone());
        let mut output = Vec::new();
        for piece in token.as_bytes().chunks(2) {
            let piece = std::str::from_utf8(piece).unwrap();
            let event = match protocol {
                "anthropic" => {
                    json!({"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":piece}})
                }
                "gemini" => {
                    json!({"candidates":[{"index":1,"content":{"parts":[{"text":piece}]}}]})
                }
                _ => {
                    json!({"type":"response.output_text.delta","item_id":"item_1","content_index":0,"delta":piece})
                }
            };
            output.extend(
                restore
                    .push_json(&serde_json::to_vec(&event).unwrap())
                    .unwrap(),
            );
        }
        output.extend(restore.finish().unwrap());
        let joined = output
            .iter()
            .map(|bytes| {
                let value: Value = serde_json::from_slice(bytes).unwrap();
                value
                    .pointer(match protocol {
                        "anthropic" => "/delta/text",
                        "gemini" => "/candidates/0/content/parts/0/text",
                        _ => "/delta",
                    })
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<String>();
        assert_eq!(joined, "user@example.com", "{protocol}");
    }
    let first = json!({"type":"response.output_text.delta","item_id":"a","delta":&token[..15]});
    let second = json!({"type":"response.output_text.delta","item_id":"a","delta":&token[15..]});
    let wire = format!("event: response.output_text.delta data: {first} event: response.output_text.delta data: {second} data: [DONE]");
    let chunks = wire
        .as_bytes()
        .chunks(3)
        .map(|bytes| Ok(bytes.to_vec()))
        .collect::<Vec<_>>();
    let result = stream::restore_sse_stream(Box::pin(futures_util::stream::iter(chunks)), request)
        .collect::<Vec<_>>()
        .await;
    let result = String::from_utf8(
        result
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .concat(),
    )
    .unwrap();
    assert!(result.contains("user@example.com"));
    assert!(!result.contains(TOKEN_PREFIX));
    assert!(result.ends_with("[DONE]"));
}

#[test]
fn logs_redact_values_split_across_tool_deltas_and_omit_truncation() {
    let request = request(&runtime(), "one");
    request
        .prepare(br#"{"input":"user@example.com"}"#, "p")
        .unwrap();
    let pieces = ["{\"name\":\"user@", "example.com\"}"];
    let wire = pieces.into_iter().map(|piece| format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":piece}}]}}]}))).collect::<String>();
    let log = request.log_body(wire.as_bytes(), wire.len() as u64);
    assert!(!log.contains("user@"));
    assert!(log.contains("redacted_stream"));
    assert!(request
        .log_body(&wire.as_bytes()[..wire.len() - 8], wire.len() as u64)
        .contains("omitted"));
}

#[tokio::test]
async fn enabled_no_match_stream_preserves_sse_metadata_crlf_and_utf8() {
    let request = request(&runtime(), "one");
    request.prepare(b"{}", "p").unwrap();
    let wire = "id: 4\r\nretry: 900\r\nevent: message\r\ndata: { \"choices\": [{\"delta\":{\"content\":\"你好 {\"},\"index\":0}] }\r\n\r\n: heartbeat\n\ndata: [DONE]\n\n";
    let chunks = wire
        .as_bytes()
        .chunks(1)
        .map(|bytes| Ok(bytes.to_vec()))
        .collect::<Vec<_>>();
    let result = stream::restore_sse_stream(Box::pin(futures_util::stream::iter(chunks)), request)
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        result
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .concat(),
        wire.as_bytes()
    );
}

#[test]
fn signed_thinking_deltas_fail_before_emitting_modified_text() {
    let request = request(&runtime(), "one");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"user@example.com"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    for event in [
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":token}}),
        json!({"candidates":[{"content":{"parts":[{"thought":true,"text":token}]}}]}),
    ] {
        let mut restorer = stream::EventRestorer::new(request.clone());
        assert!(restorer
            .push_json(&serde_json::to_vec(&event).unwrap())
            .unwrap_err()
            .starts_with("privacy_signed_payload"));
    }
    let mut restorer = stream::EventRestorer::new(request.clone());
    for (index, part) in token.as_bytes().chunks(4).enumerate() {
        let event = json!({"candidates":[{"content":{"parts":[{"thought":index == 0,"text":std::str::from_utf8(part).unwrap()}]}}]});
        let result = restorer.push_json(&serde_json::to_vec(&event).unwrap());
        if (index + 1) * 4 >= token.len() {
            assert!(result.unwrap_err().starts_with("privacy_signed_payload"));
        } else {
            assert!(result.unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn restoration_failure_keeps_usage_received_in_the_same_sse_chunk() {
    let request = request(&runtime(), "one");
    request.prepare(b"{}", "p").unwrap();
    let wire = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"__AITB_PRIV_unknown__\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_bad\",\"usage\":{\"input_tokens\":8,\"output_tokens\":2}}}\n\n";
    let stream = stream::restore_sse_stream(
        Box::pin(futures_util::stream::iter(vec![Ok(wire.to_vec())])),
        request.clone(),
    );
    let output = stream.collect::<Vec<_>>().await;
    assert!(output[0]
        .as_ref()
        .unwrap_err()
        .starts_with("privacy_token_unknown"));
    assert_eq!(request.usage().total_tokens(), Some(10));
}

#[test]
fn anthropic_cumulative_delta_usage_replaces_start_estimate() {
    // The redactor observes one SSE `data` payload at a time, so it never gets
    // the collector's fold: the cumulative `message_delta` must still replace
    // the `message_start` context estimate instead of max-merging with it.
    let runtime = runtime();
    let request = runtime.begin("claude", &[], b"{}", None).unwrap().unwrap();
    request.observe_usage(
        br#"{"type":"message_start","message":{"usage":{"input_tokens":26161,"cache_read_input_tokens":18000}}}"#,
    );
    request.observe_usage(
        br#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":6737,"cache_read_input_tokens":51968,"output_tokens":200}}"#,
    );
    let usage = request.usage();
    assert_eq!(usage.input_tokens, Some(6737));
    assert_eq!(usage.cache_read_tokens, Some(51968));
    assert_eq!(usage.output_tokens, Some(200));
}

#[test]
fn streamed_tools_restore_nested_json_and_unicode_encoded_placeholders() {
    let request = request(&runtime(), "nested-tools");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"secret\"line\nvalue"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    for protocol in ["chat", "anthropic", "responses"] {
        for unicode_encoded in [false, true] {
            let mut arguments = json!({"content": json!({"value": token}).to_string()}).to_string();
            if unicode_encoded {
                arguments = arguments.replace('_', "\\u005f");
            }
            let mut restorer = stream::EventRestorer::new(request.clone());
            let mut events = Vec::new();
            for piece in arguments.as_bytes().chunks(3) {
                let piece = std::str::from_utf8(piece).unwrap();
                let event = match protocol {
                    "chat" => {
                        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":piece}}]}}]})
                    }
                    "anthropic" => {
                        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":piece}})
                    }
                    _ => {
                        json!({"type":"response.function_call_arguments.delta","item_id":"call-1","delta":piece})
                    }
                };
                events.extend(
                    restorer
                        .push_json(&serde_json::to_vec(&event).unwrap())
                        .unwrap(),
                );
            }
            events.extend(restorer.finish().unwrap());
            let arguments = events
                .iter()
                .map(|event| {
                    let event: Value = serde_json::from_slice(event).unwrap();
                    event
                        .pointer(match protocol {
                            "chat" => "/choices/0/delta/tool_calls/0/function/arguments",
                            "anthropic" => "/delta/partial_json",
                            _ => "/delta",
                        })
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect::<String>();
            let arguments: Value = serde_json::from_str(&arguments).unwrap();
            let content: Value =
                serde_json::from_str(arguments["content"].as_str().unwrap()).unwrap();
            assert_eq!(
                content,
                json!({"value":"secret\"line\nvalue"}),
                "{protocol}, unicode={unicode_encoded}"
            );
        }
    }
}

#[test]
fn empty_error_fields_do_not_terminate_text_fragments() {
    let request = request(&runtime(), "empty-error");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"user@example.com"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    for empty_error in [Value::Null, json!({}), json!("")] {
        let mut restorer = stream::EventRestorer::new(request.clone());
        let first =
            json!({"choices":[{"index":0,"delta":{"content":&token[..15]}}],"error":empty_error});
        assert!(restorer
            .push_json(&serde_json::to_vec(&first).unwrap())
            .unwrap()
            .is_empty());
        let second = json!({"choices":[{"index":0,"delta":{"content":&token[15..]}}]});
        let events = restorer
            .push_json(&serde_json::to_vec(&second).unwrap())
            .unwrap();
        let restored = events
            .iter()
            .map(|event| {
                serde_json::from_slice::<Value>(event).unwrap()["choices"][0]["delta"]["content"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<String>();
        assert_eq!(restored, "user@example.com");
    }
}

#[test]
fn json_tool_results_redact_password_fields_in_each_protocol() {
    let original = json!({"password":"short-password"}).to_string();
    for body in [
        json!({"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":original}]}]}),
        json!({"messages":[{"role":"tool","tool_call_id":"call_1","content":original}]}),
        json!({"input":[{"type":"function_call_output","call_id":"call_1","output":original}]}),
        json!({"contents":[{"role":"user","parts":[{"functionResponse":{"name":"read","response":{"content":original}}}]}]}),
    ] {
        let request = request(&runtime(), "tool-results");
        let masked = request
            .prepare(&serde_json::to_vec(&body).unwrap(), "p")
            .unwrap();
        assert!(!String::from_utf8_lossy(&masked).contains("short-password"));
        assert_eq!(
            serde_json::from_slice::<Value>(&request.restore(&masked).unwrap()).unwrap(),
            body
        );
    }
}

#[test]
fn namespace_tool_definitions_scan_descriptions_and_schema_defaults() {
    let request = request(&runtime(), "namespace");
    let body = json!({"tools":[{"type":"namespace","name":"user@example.com","description":"outer","tools":[{"type":"function","name":"user@example.com","description":"user@example.com","parameters":{"type":"object","properties":{"account":{"type":"string","default":"user@example.com"}}}}]}]});
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(&serde_json::to_vec(&body).unwrap(), "p")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(masked["tools"][0]["name"], "user@example.com");
    assert_eq!(masked["tools"][0]["tools"][0]["name"], "user@example.com");
    assert_ne!(
        masked["tools"][0]["tools"][0]["description"],
        "user@example.com"
    );
    assert_ne!(
        masked["tools"][0]["tools"][0]["parameters"]["properties"]["account"]["default"],
        "user@example.com"
    );
}

#[test]
fn logs_do_not_replace_inside_existing_or_new_placeholders() {
    let request = request(&runtime(), "logs");
    let body = json!({"input":[{"type":"function_call","name":"use_credentials","arguments":json!({"password":"I","api_key":"P"}).to_string()}]});
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(&serde_json::to_vec(&body).unwrap(), "p")
            .unwrap(),
    )
    .unwrap();
    let arguments: Value =
        serde_json::from_str(masked["input"][0]["arguments"].as_str().unwrap()).unwrap();
    let expected = format!(
        "{} {}",
        arguments["password"].as_str().unwrap(),
        arguments["api_key"].as_str().unwrap()
    );
    assert_eq!(request.log_text("I P"), expected);
    assert_eq!(request.log_text(&expected), expected);
    let body = json!({"message":"I P"}).to_string();
    let log: Value =
        serde_json::from_str(&request.log_body(body.as_bytes(), body.len() as u64)).unwrap();
    assert_eq!(log["message"], expected);
}

#[test]
fn active_request_refreshes_idle_expiry_before_previous_response_continuation() {
    let runtime = runtime();
    let first = request(&runtime, "long-running");
    let body = br#"{"input":"user@example.com"}"#;
    let masked = first.prepare(body, "p").unwrap();
    for entry in runtime.mappings.lock().unwrap().sessions.values_mut() {
        entry.touched = Instant::now() - SESSION_TTL - Duration::from_secs(1);
    }
    // Another request prunes the cache while the first request is still active.
    request(&runtime, "other").prepare(b"{}", "p").unwrap();
    first.remember_response(&json!({"id":"resp-long"}));
    drop(first);
    let next = runtime
        .begin(
            "codex",
            &[("session_id".into(), "long-running".into())],
            br#"{"previous_response_id":"resp-long"}"#,
            None,
        )
        .unwrap()
        .unwrap();
    next.prepare(b"{}", "p").unwrap();
    assert_eq!(next.restore(&masked).unwrap(), body);
}

#[test]
fn custom_regex_replaces_the_full_match_and_respects_overlap_priority() {
    let mut rules = PrivacyRules::default();
    rules.builtins.clear();
    for (id, pattern, priority) in [
        ("high", "(?P<secret>ABC)-DEF", 200),
        ("low", "DEF-extra", 100),
    ] {
        rules.custom.push(PrivacyCustomRule {
            id: id.into(),
            name: id.into(),
            enabled: true,
            kind: PrivacyRuleKind::Regex,
            pattern: pattern.into(),
            priority,
        });
    }
    let result = preview(rules, "ABC-DEF-extra".into()).unwrap();
    assert!(result.redacted.ends_with("__-extra"));
    assert_eq!(result.detail.rules.get("high"), Some(&1));
    assert_eq!(result.detail.rules.get("low"), None);
    assert_eq!(result.restored, "ABC-DEF-extra");
}

#[test]
fn anthropic_response_blocks_preserve_media_and_reject_signed_thinking() {
    let request = request(&runtime(), "response-blocks");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"user@example.com"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    for block in [
        json!({"type":"image","source":{"type":"url","url":token}}),
        json!({"type":"redacted_thinking","data":token}),
    ] {
        let body = json!({"type":"message","content":[block]}).to_string();
        assert_eq!(request.restore(body.as_bytes()).unwrap(), body.as_bytes());
    }
    let body =
        json!({"type":"message","content":[{"type":"thinking","thinking":token}]}).to_string();
    assert!(request
        .restore(body.as_bytes())
        .unwrap_err()
        .starts_with("privacy_signed_payload"));
}

#[test]
fn tool_stream_preserves_json_keys_and_handles_split_escapes_and_long_values() {
    let request = request(&runtime(), "argument-strings");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"user@example.com"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    let value = format!("{}\"\\你好\n{token}", "ordinary ".repeat(1000));
    let arguments = json!({token:value,"empty":"","nested":{"key":"plain"}}).to_string();
    let mut restorer = stream::EventRestorer::new(request.clone());
    let mut restored = String::new();
    for character in arguments.chars() {
        let event = json!({"type":"response.function_call_arguments.delta","item_id":"item","delta":character.to_string()});
        for bytes in restorer
            .push_json(&serde_json::to_vec(&event).unwrap())
            .unwrap()
        {
            restored.push_str(
                serde_json::from_slice::<Value>(&bytes).unwrap()["delta"]
                    .as_str()
                    .unwrap(),
            );
        }
    }
    for bytes in restorer.finish().unwrap() {
        restored.push_str(
            serde_json::from_slice::<Value>(&bytes).unwrap()["delta"]
                .as_str()
                .unwrap(),
        );
    }
    let restored: Value = serde_json::from_str(&restored).unwrap();
    assert_eq!(restored[token], value.replace(token, "user@example.com"));
    assert_eq!(restored["empty"], "");
    assert_eq!(restored["nested"]["key"], "plain");
}

#[test]
fn protocol_media_locations_keep_urls_opaque_while_redacting_neighboring_text() {
    let image_url = "https://cdn.example/user@example.com.png";
    for (body, image_pointer, text_pointer) in [
        (
            json!({"input":[{"type":"function_call_output","call_id":"call","output":[{"type":"input_text","text":"user@example.com"},{"type":"input_image","image_url":image_url}]}]}),
            "/input/0/output/1/image_url",
            "/input/0/output/0/text",
        ),
        (
            json!({"messages":[{"role":"user","content":"user@example.com","images":[image_url]}]}),
            "/messages/0/images/0",
            "/messages/0/content",
        ),
    ] {
        let request = request(&runtime(), "media");
        let masked: Value = serde_json::from_slice(
            &request
                .prepare(&serde_json::to_vec(&body).unwrap(), "p")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            masked.pointer(image_pointer),
            body.pointer(image_pointer),
            "{image_pointer}"
        );
        assert_ne!(
            masked.pointer(text_pointer),
            body.pointer(text_pointer),
            "{text_pointer}"
        );
    }
}

#[test]
fn gemini_function_response_media_bytes_are_not_business_values() {
    let runtime = runtime();
    let mut settings = PrivacySettings::default();
    settings.enabled = true;
    settings.rules.custom.push(PrivacyCustomRule {
        id: "base64".into(),
        name: "Base64 text".into(),
        enabled: true,
        kind: PrivacyRuleKind::Literal,
        pattern: "YWJj".into(),
        priority: 200,
    });
    runtime.publish(CompiledPolicy::compile(&settings).unwrap());
    let request = request(&runtime, "gemini-media");
    let body = json!({"contents":[{"role":"user","parts":[{"text":"YWJj"},{"functionResponse":{"name":"read_image","response":{"text":"YWJj"},"parts":[{"inlineData":{"mimeType":"image/png","data":"YWJj"}}]}}]}]});
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(&serde_json::to_vec(&body).unwrap(), "p")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        masked.pointer("/contents/0/parts/1/functionResponse/parts"),
        body.pointer("/contents/0/parts/1/functionResponse/parts")
    );
    assert_ne!(
        masked.pointer("/contents/0/parts/1/functionResponse/response/text"),
        body.pointer("/contents/0/parts/1/functionResponse/response/text")
    );
}

#[test]
fn schema_locations_scan_descriptions_without_rewriting_type_format_or_required_keys() {
    let runtime = runtime();
    let mut settings = PrivacySettings::default();
    settings.enabled = true;
    for pattern in ["email", "string"] {
        settings.rules.custom.push(PrivacyCustomRule {
            id: pattern.into(),
            name: pattern.into(),
            enabled: true,
            kind: PrivacyRuleKind::Literal,
            pattern: pattern.into(),
            priority: 200,
        });
    }
    runtime.publish(CompiledPolicy::compile(&settings).unwrap());
    let schema = json!({"type":"object","properties":{"email":{"type":"string","format":"email","description":"email"}},"required":["email"]});
    for (body, pointer) in [
        (
            json!({"response_format":{"type":"json_schema","json_schema":{"name":"email","schema":schema}}}),
            "/response_format/json_schema/schema",
        ),
        (json!({"format":schema}), "/format"),
        (
            json!({"generationConfig":{"responseSchema":schema,"responseMimeType":"application/json"}}),
            "/generationConfig/responseSchema",
        ),
    ] {
        let request = request(&runtime, "schema");
        let masked: Value = serde_json::from_slice(
            &request
                .prepare(&serde_json::to_vec(&body).unwrap(), "p")
                .unwrap(),
        )
        .unwrap();
        let masked_schema = masked.pointer(pointer).unwrap();
        assert_eq!(
            masked_schema["properties"]["email"]["type"], "string",
            "{pointer}"
        );
        assert_eq!(
            masked_schema["properties"]["email"]["format"], "email",
            "{pointer}"
        );
        assert_eq!(masked_schema["required"], json!(["email"]), "{pointer}");
        assert_ne!(
            masked_schema["properties"]["email"]["description"], "email",
            "{pointer}"
        );
    }
}

#[test]
fn responses_history_keeps_namespace_identity() {
    let runtime = runtime();
    let mut settings = PrivacySettings::default();
    settings.enabled = true;
    settings.rules.custom.push(PrivacyCustomRule {
        id: "namespace".into(),
        name: "Namespace word".into(),
        enabled: true,
        kind: PrivacyRuleKind::Literal,
        pattern: "workspace".into(),
        priority: 200,
    });
    runtime.publish(CompiledPolicy::compile(&settings).unwrap());
    let request = request(&runtime, "namespace-identity");
    let body = json!({"tools":[{"type":"namespace","name":"workspace","tools":[{"type":"function","name":"run","description":"workspace"}]}],"input":[{"type":"function_call","namespace":"workspace","name":"run","call_id":"call","arguments":"{}"}]});
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(&serde_json::to_vec(&body).unwrap(), "p")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        masked["input"][0]["namespace"],
        body["input"][0]["namespace"]
    );
    assert_ne!(
        masked["tools"][0]["tools"][0]["description"],
        body["tools"][0]["tools"][0]["description"]
    );
}

#[test]
fn ollama_tool_name_is_identity_while_tool_result_fields_are_business_text() {
    let runtime = runtime();
    let mut settings = PrivacySettings::default();
    settings.enabled = true;
    settings.rules.custom.push(PrivacyCustomRule {
        id: "tool-name".into(),
        name: "Tool name in business text".into(),
        enabled: true,
        kind: PrivacyRuleKind::Literal,
        pattern: "write_file".into(),
        priority: 200,
    });
    runtime.publish(CompiledPolicy::compile(&settings).unwrap());
    let request = request(&runtime, "ollama-tool-identity");
    let body = json!({"messages":[{"role":"tool","tool_name":"write_file","content":json!({"tool_name":"write_file"}).to_string()}]});
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(&serde_json::to_vec(&body).unwrap(), "p")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(masked["messages"][0]["tool_name"], "write_file");
    let content: Value =
        serde_json::from_str(masked["messages"][0]["content"].as_str().unwrap()).unwrap();
    assert_ne!(content["tool_name"], "write_file");
}

#[test]
fn anthropic_plain_text_documents_are_scanned_without_touching_binary_sources() {
    let request = request(&runtime(), "documents");
    let body = json!({"messages":[{"role":"user","content":[{"type":"document","title":"user@example.com","source":{"type":"text","media_type":"text/plain","data":"user@example.com"}},{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"opaque-base64"}}]}]});
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(&serde_json::to_vec(&body).unwrap(), "p")
            .unwrap(),
    )
    .unwrap();
    assert_ne!(
        masked["messages"][0]["content"][0]["source"]["data"],
        "user@example.com"
    );
    assert_ne!(
        masked["messages"][0]["content"][0]["title"],
        "user@example.com"
    );
    assert_eq!(
        masked["messages"][0]["content"][1],
        body["messages"][0]["content"][1]
    );
}

#[tokio::test]
async fn legacy_completion_text_restores_across_sse_fragments() {
    let request = request(&runtime(), "completions");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"prompt":"user@example.com"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["prompt"].as_str().unwrap();
    let mut wire = token
        .chars()
        .map(|character| {
            format!(
                "data: {}\n\n",
                json!({"choices":[{"index":0,"text":character.to_string(),"finish_reason":null}]})
            )
        })
        .collect::<String>();
    wire.push_str("data: {\"choices\":[{\"index\":0,\"text\":\"\",\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    let output = stream::restore_sse_stream(
        Box::pin(futures_util::stream::iter(vec![Ok(wire.into_bytes())])),
        request,
    )
    .collect::<Vec<_>>()
    .await;
    let output = String::from_utf8(
        output
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .concat(),
    )
    .unwrap();
    let text = output
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .map(|value| {
            value["choices"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect::<String>();
    assert_eq!(text, "user@example.com");
}

#[test]
fn gemini_thought_and_public_text_at_the_same_part_index_use_separate_channels() {
    let request = request(&runtime(), "gemini-channel-kinds");
    let masked: Value = serde_json::from_slice(
        &request
            .prepare(br#"{"input":"user@example.com"}"#, "p")
            .unwrap(),
    )
    .unwrap();
    let token = masked["input"].as_str().unwrap();
    let mut restorer = stream::EventRestorer::new(request);
    let thinking = json!({"candidates":[{"index":0,"content":{"parts":[{"thought":true,"text":"Plan safely","thoughtSignature":"opaque-signature"}]}}]});
    restorer
        .push_json(&serde_json::to_vec(&thinking).unwrap())
        .unwrap();
    let mut output = Vec::new();
    for piece in token.as_bytes().chunks(3) {
        let event = json!({"candidates":[{"index":0,"content":{"parts":[{"text":std::str::from_utf8(piece).unwrap()}]}}]});
        output.extend(
            restorer
                .push_json(&serde_json::to_vec(&event).unwrap())
                .unwrap(),
        );
    }
    let text = output
        .iter()
        .map(|bytes| {
            serde_json::from_slice::<Value>(bytes).unwrap()["candidates"][0]["content"]["parts"][0]
                ["text"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<String>();
    assert_eq!(text, "user@example.com");
}
