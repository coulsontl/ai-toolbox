use super::support::*;
use ai_toolbox_lib::http_client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpListener;

#[derive(Clone, Copy)]
enum History {
    Full,
    PreviousResponse,
    CallIdsOnly,
    Partial,
}

async fn exercise_parallel_followups(
    provider_type: &str,
    upstream_streaming: bool,
    client_streaming: bool,
    history: History,
    logging: BodyLogging,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let mut first_body = if upstream_streaming {
        chat_tool_stream_with_late_text(true)
    } else {
        serde_json::to_vec(&chat_tool_response()).unwrap()
    };
    if matches!(logging, BodyLogging::Truncated) {
        assert!(upstream_streaming);
        first_body = String::from_utf8(first_body)
            .unwrap()
            .replace("Read both files.", &"Long fixture reasoning. ".repeat(256))
            .into_bytes();
    }
    let upstream = tokio::spawn(async move {
        let mut captured = Vec::new();
        for round in 0..3 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .expect("gateway must send the next request")
                .unwrap();
            let body = tokio::time::timeout(
                Duration::from_secs(10),
                read_mock_request(&mut socket, "/v1/chat/completions "),
            )
            .await
            .expect("complete upstream request");
            let error = validate_chat_tool_history(&body).err();
            let (status, reply, streaming) = if let Some(error) = error {
                (
                    400,
                    serde_json::to_vec(&json!({"error": {
                        "type": "invalid_request_error", "message": error
                    }}))
                    .unwrap(),
                    false,
                )
            } else if round == 0 {
                (200, first_body.clone(), upstream_streaming)
            } else {
                (200, serde_json::to_vec(&json!({
                    "id": format!("chatcmpl_followup_{round}"), "object": "chat.completion",
                    "model": "fixture-model", "choices": [{
                        "index": 0, "message": {"role": "assistant", "content": "Both tools completed."},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 14, "completion_tokens": 3, "total_tokens": 17}
                })).unwrap(), false)
            };
            captured.push(body);
            tokio::time::timeout(
                Duration::from_secs(10),
                write_mock_reply(&mut socket, status, &reply, streaming),
            )
            .await
            .expect("write upstream response");
        }
        captured
    });

    let gateway = RunningGateway::new(&upstream_url, provider_type, logging);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let mut request =
        responses_request(vec![json!({"role": "user", "content": "Read both files."})]);
    request["stream"] = json!(client_streaming);
    let response = client
        .post(&gateway.url)
        .json(&request)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let first_response: Value = if client_streaming {
        completed_response(&sse_events(&bytes))
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    assert_eq!(first_response["status"], "completed");
    assert_eq!(first_response["usage"]["input_tokens"], 10);
    assert_eq!(first_response["usage"]["output_tokens"], 5);
    let calls = first_response["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["call_id"], "tool_a");
    assert_eq!(calls[1]["call_id"], "tool_b");

    // Replay the same complete history twice, reversing result completion
    // order on the second request. Old affected sessions must remain usable.
    for replay in 0..2 {
        let mut input = match history {
            History::Full => first_response["output"].as_array().unwrap().clone(),
            History::Partial => vec![calls[0].clone()],
            History::PreviousResponse | History::CallIdsOnly => Vec::new(),
        };
        let ids = if replay == 0 {
            ["tool_a", "tool_b"]
        } else {
            ["tool_b", "tool_a"]
        };
        input.extend(ids.map(|id| tool_output(id, json!(format!("result {id}")))));
        let mut next = responses_request(input);
        if matches!(history, History::PreviousResponse | History::Partial) {
            next["previous_response_id"] = first_response["id"].clone();
        }
        next["parallel_tool_calls"] = json!(replay == 0);
        let response = client.post(&gateway.url).json(&next).send().await.unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        assert_eq!(
            status,
            200,
            "replay {replay}: {}",
            String::from_utf8_lossy(&bytes)
        );
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["status"], "completed");
        assert_eq!(
            response["output"][0]["content"][0]["text"],
            "Both tools completed."
        );
    }
    let captured = upstream.await.unwrap();
    assert_eq!(captured.len(), 3);
    for (index, body) in captured[1..].iter().enumerate() {
        assert_chat_tool_history(body);
        let messages = body["messages"].as_array().unwrap();
        let batches = messages
            .iter()
            .filter_map(|message| message["tool_calls"].as_array())
            .collect::<Vec<_>>();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
        assert_eq!(batches[0][0]["id"], "tool_a");
        assert_eq!(batches[0][1]["id"], "tool_b");
        assert_eq!(
            batches[0][0]["function"]["arguments"],
            "{\"path\":\"first.txt\"}"
        );
        assert_eq!(
            batches[0][1]["function"]["arguments"],
            "{\"path\":\"second.txt\"}"
        );
        let outputs = messages
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect::<Vec<_>>();
        assert_eq!(outputs.len(), 2);
        let ids = if index == 0 {
            ["tool_a", "tool_b"]
        } else {
            ["tool_b", "tool_a"]
        };
        for (output, id) in outputs.iter().zip(ids) {
            assert_eq!(output["tool_call_id"], id);
            assert_eq!(output["content"], format!("result {id}"));
        }
        assert_eq!(body["parallel_tool_calls"], index == 0);
        if upstream_streaming && matches!(history, History::Full) {
            assert_eq!(
                body.to_string().matches("Reading both files now.").count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn moonshot_json_parallel_history_survives_repeated_followups() {
    exercise_parallel_followups(
        "moonshot",
        false,
        false,
        History::Full,
        BodyLogging::Enabled,
    )
    .await;
}

#[tokio::test]
async fn glm_json_previous_response_restores_the_entire_parallel_batch() {
    exercise_parallel_followups(
        "zai",
        false,
        false,
        History::PreviousResponse,
        BodyLogging::Enabled,
    )
    .await;
}

#[tokio::test]
async fn custom_chat_sse_parallel_history_preserves_ids_and_arguments() {
    exercise_parallel_followups("custom", true, true, History::Full, BodyLogging::Enabled).await;
}

#[tokio::test]
async fn moonshot_sse_previous_response_restores_the_entire_parallel_batch() {
    exercise_parallel_followups(
        "moonshot",
        true,
        true,
        History::PreviousResponse,
        BodyLogging::Enabled,
    )
    .await;
}

#[tokio::test]
async fn glm_forced_sse_aggregation_preserves_parallel_followups() {
    exercise_parallel_followups(
        "zai",
        true,
        false,
        History::PreviousResponse,
        BodyLogging::Enabled,
    )
    .await;
}

#[tokio::test]
async fn custom_chat_parallel_outputs_restore_calls_without_previous_response_id() {
    exercise_parallel_followups(
        "custom",
        false,
        false,
        History::CallIdsOnly,
        BodyLogging::Enabled,
    )
    .await;
}

#[tokio::test]
async fn moonshot_partial_parallel_history_restores_only_the_missing_call() {
    exercise_parallel_followups(
        "moonshot",
        false,
        false,
        History::Partial,
        BodyLogging::Enabled,
    )
    .await;
}

#[tokio::test]
async fn parallel_sse_history_is_independent_of_disabled_body_logging() {
    exercise_parallel_followups(
        "custom",
        true,
        true,
        History::PreviousResponse,
        BodyLogging::Disabled,
    )
    .await;
}

#[tokio::test]
async fn parallel_sse_history_is_independent_of_truncated_body_logging() {
    exercise_parallel_followups(
        "moonshot",
        true,
        true,
        History::PreviousResponse,
        BodyLogging::Truncated,
    )
    .await;
}

async fn exercise_gemini_parallel_followup(
    native_ids: bool,
    upstream_streaming: bool,
    client_streaming: bool,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let mut parts = [("tool_a", "first.txt"), ("tool_b", "second.txt")]
        .into_iter()
        .map(|(id, path)| {
            let mut call = json!({"name": "read_file", "args": {"path": path}});
            if native_ids {
                call["id"] = json!(id);
            }
            json!({"functionCall": call})
        })
        .collect::<Vec<_>>();
    parts.push(json!({"text": "Reading both files now."}));
    let first_reply = json!({"responseId": "gemini_http_fixture", "modelVersion": "gemini-2.5-pro", "candidates": [
        {"index": 0, "content": {"role": "model", "parts": parts}, "finishReason": "STOP"}
    ], "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5, "totalTokenCount": 15}});
    let first_wire = if upstream_streaming {
        parts.iter().enumerate().map(|(index, part)| {
            let mut chunk = json!({"responseId": "gemini_http_fixture", "modelVersion": "gemini-2.5-pro", "candidates": [
                {"index": 0, "content": {"role": "model", "parts": [part]}}
            ]});
            if index == parts.len() - 1 {
                chunk["candidates"][0]["finishReason"] = json!("STOP");
                chunk["usageMetadata"] = first_reply["usageMetadata"].clone();
            }
            format!("data: {chunk}\n\n")
        }).collect::<String>().into_bytes()
    } else {
        serde_json::to_vec(&first_reply).unwrap()
    };
    let upstream = tokio::spawn(async move {
        let mut captured = Vec::new();
        for round in 0..2 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let body = tokio::time::timeout(
                Duration::from_secs(10),
                read_mock_request(&mut socket, "/v1beta/models/"),
            )
            .await
            .unwrap();
            if round == 1 {
                let contents = body["contents"].as_array().unwrap();
                assert_eq!(contents.len(), 3, "{body}");
                assert_eq!(contents[0]["role"], "user");
                assert_eq!(contents[1]["role"], "model");
                assert_eq!(contents[2]["role"], "user");
                let calls = contents[1]["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|part| part.get("functionCall"))
                    .collect::<Vec<_>>();
                let results = contents[2]["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|part| part.get("functionResponse"))
                    .collect::<Vec<_>>();
                assert_eq!(calls.len(), 2);
                assert_eq!(results.len(), 2);
                assert_eq!(calls[0]["args"]["path"], "first.txt");
                assert_eq!(calls[1]["args"]["path"], "second.txt");
                for (index, (call, result)) in calls.iter().zip(&results).enumerate() {
                    assert_eq!(call["name"], "read_file");
                    assert_eq!(result["name"], "read_file");
                    if native_ids {
                        assert_eq!(call["id"], if index == 0 { "tool_a" } else { "tool_b" });
                        assert_eq!(result["id"], if index == 0 { "tool_b" } else { "tool_a" });
                        assert_eq!(
                            result["response"]["result"],
                            format!("result {}", 1 - index)
                        );
                    } else {
                        assert!(call.get("id").is_none());
                        assert!(result.get("id").is_none());
                        assert_eq!(result["response"]["result"], format!("result {index}"));
                    }
                }
                assert_eq!(
                    body.to_string().matches("Reading both files now.").count(),
                    1
                );
            }
            captured.push(body);
            let reply = if round == 0 {
                first_wire.clone()
            } else {
                serde_json::to_vec(&json!({"responseId": "gemini_followup", "modelVersion": "gemini-2.5-pro", "candidates": [
                    {"index": 0, "content": {"role": "model", "parts": [{"text": "Both tools completed."}]}, "finishReason": "STOP"}
                ], "usageMetadata": {"promptTokenCount": 14, "candidatesTokenCount": 3, "totalTokenCount": 17}})).unwrap()
            };
            tokio::time::timeout(
                Duration::from_secs(10),
                write_mock_reply(&mut socket, 200, &reply, round == 0 && upstream_streaming),
            )
            .await
            .unwrap();
        }
        captured
    });
    let gateway = RunningGateway::new_for_api_format(
        &upstream_url,
        "gemini",
        BodyLogging::Disabled,
        "gemini_native",
    );
    let client = http_client::create_client_no_proxy(10).unwrap();
    let user_message = json!({"role": "user", "content": "Read both files."});
    let mut first = responses_request(vec![user_message.clone()]);
    first["stream"] = json!(client_streaming);
    let response = client.post(&gateway.url).json(&first).send().await.unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let reply: Value = if client_streaming {
        completed_response(&sse_events(&bytes))
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    assert_eq!(reply["status"], "completed");
    assert_eq!(reply["usage"]["input_tokens"], 10);
    assert_eq!(reply["usage"]["output_tokens"], 5);
    let calls = reply["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "{reply}");
    assert_ne!(calls[0]["call_id"], calls[1]["call_id"]);
    let mut input = vec![user_message];
    input.extend(reply["output"].as_array().unwrap().clone());
    input.extend((0..2).rev().map(|index| {
        tool_output(
            calls[index]["call_id"].as_str().unwrap(),
            json!(format!("result {index}")),
        )
    }));
    let response = client
        .post(&gateway.url)
        .json(&responses_request(input))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["status"], "completed");
    assert_eq!(
        reply["output"][0]["content"][0]["text"],
        "Both tools completed."
    );
    assert_eq!(upstream.await.unwrap().len(), 2);
}

#[tokio::test]
async fn gemini_json_followup_preserves_one_native_parallel_result_turn() {
    exercise_gemini_parallel_followup(true, false, false).await;
}

#[tokio::test]
async fn gemini_anonymous_sse_followup_restores_result_order_without_leaking_ids() {
    exercise_gemini_parallel_followup(false, true, true).await;
}

#[tokio::test]
async fn gemini_forced_sse_aggregation_keeps_both_native_parallel_calls() {
    exercise_gemini_parallel_followup(true, true, false).await;
}

#[tokio::test]
async fn gemini_forced_sse_aggregation_keeps_anonymous_parallel_calls_distinct() {
    exercise_gemini_parallel_followup(false, true, false).await;
}
