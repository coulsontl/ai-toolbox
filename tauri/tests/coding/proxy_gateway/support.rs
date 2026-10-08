use ai_toolbox_lib::coding::proxy_gateway::{
    paths::ProxyGatewayPaths, types::ProxyGatewaySettings, ProxyGatewayState,
};
use ai_toolbox_lib::db::{
    helpers::{db_create, db_put},
    schema::DbTable,
    SqliteDbState,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How much of each request/response body the gateway is asked to record.
///
/// `Truncated` shrinks `log_max_body_size_kb`, which is how the body-logging
/// limits are exercised.
#[derive(Clone, Copy)]
pub(super) enum BodyLogging {
    Enabled,
    Disabled,
    Truncated,
}

/// A real gateway process bound to an ephemeral port, forwarding to `upstream_url`.
pub(super) struct RunningGateway {
    state: ProxyGatewayState,
    pub(super) url: String,
    _directory: tempfile::TempDir,
}

impl RunningGateway {
    pub(super) fn new(upstream_url: &str, provider_type: &str, logging: BodyLogging) -> Self {
        Self::new_for_api_format(upstream_url, provider_type, logging, "openai_chat")
    }

    pub(super) fn new_for_api_format(
        upstream_url: &str,
        provider_type: &str,
        logging: BodyLogging,
        api_format: &str,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let api_version = if api_format == "gemini_native" {
            "v1beta"
        } else {
            "v1"
        };
        let config = format!(
            "model = \"fixture-model\"\nmodel_provider = \"fixture\"\n\
             [model_providers.fixture]\nbase_url = \"{upstream_url}/{api_version}\"\nwire_api = \"chat\"\n"
        );
        db.with_conn(|connection| {
            db_put(
                connection,
                DbTable::Settings,
                "app",
                &json!({"proxy_mode": "direct"}),
            )?;
            db_create(
                connection,
                DbTable::CodexProvider,
                &json!({
                    "name": "Parallel tool fixture", "category": "custom",
                    "is_applied": true, "is_disabled": false,
                    "settings_config": json!({
                        "config": config, "auth": {"OPENAI_API_KEY": "fixture-key"}
                    }).to_string(),
                    "meta": {"apiFormat": api_format, "providerType": provider_type}
                }),
            )
        })
        .unwrap();
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let settings = ProxyGatewaySettings {
            listen_port: port,
            port_auto_select: true,
            max_retry_count: 0,
            retry_interval_secs: 0,
            streaming_first_byte_timeout_secs: 5,
            streaming_idle_timeout_secs: 5,
            non_streaming_timeout_secs: 5,
            request_log_enabled: !matches!(logging, BodyLogging::Disabled),
            store_request_body: !matches!(logging, BodyLogging::Disabled),
            store_response_body: !matches!(logging, BodyLogging::Disabled),
            log_max_body_size_kb: if matches!(logging, BodyLogging::Truncated) {
                1
            } else {
                256
            },
            ..ProxyGatewaySettings::default()
        };
        let state = ProxyGatewayState::default();
        let status = state
            .manager
            .lock()
            .unwrap()
            .start_with_context(settings, db, ProxyGatewayPaths::new(directory.path()))
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

impl Drop for RunningGateway {
    fn drop(&mut self) {
        let _ = self.state.manager.lock().unwrap().stop();
    }
}

pub(super) async fn read_mock_request(
    socket: &mut TcpStream,
    expected_path_prefix: &str,
) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    let header_end = loop {
        let size = socket.read(&mut buffer).await.unwrap();
        assert!(size > 0, "upstream request ended before headers");
        bytes.extend_from_slice(&buffer[..size]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    assert!(
        headers.starts_with(&format!("POST {expected_path_prefix}")),
        "{headers}"
    );
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .unwrap()
        .1
        .trim()
        .parse::<usize>()
        .unwrap();
    while bytes.len() < header_end + length {
        let size = socket.read(&mut buffer).await.unwrap();
        assert!(size > 0, "upstream request ended before body");
        bytes.extend_from_slice(&buffer[..size]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}

pub(super) async fn write_mock_reply(
    socket: &mut TcpStream,
    status: u16,
    body: &[u8],
    streaming: bool,
) {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    if streaming {
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\n\
             Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        for chunk in body.chunks(37) {
            socket
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(chunk).await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
        }
        socket.write_all(b"0\r\n\r\n").await.unwrap();
    } else {
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
    }
    socket.shutdown().await.unwrap();
}

pub(super) fn function_call(call_id: &str, name: &str, arguments: Value) -> Value {
    json!({
        "type": "function_call", "id": format!("fc_{call_id}"),
        "call_id": call_id, "name": name, "arguments": arguments.to_string()
    })
}

pub(super) fn tool_output(call_id: &str, output: Value) -> Value {
    json!({"type": "function_call_output", "call_id": call_id, "output": output})
}

pub(super) fn parallel_input() -> Vec<Value> {
    vec![
        function_call("tool_a", "read_file", json!({"path": "first.txt"})),
        function_call("tool_b", "read_file", json!({"path": "second.txt"})),
        tool_output("tool_a", json!("first result")),
        tool_output("tool_b", json!("second result")),
    ]
}

pub(super) fn responses_request(input: Vec<Value>) -> Value {
    json!({
        "model": "fixture-model",
        "input": input,
        "parallel_tool_calls": true,
        "tools": [{
            "type": "function", "name": "read_file",
            "parameters": {
                "type": "object", "properties": {"path": {"type": "string"}}
            }
        }]
    })
}

/// Enforce the Chat contract independently of the Responses converter:
/// every assistant batch is answered once, before another non-tool message.
pub(super) fn validate_chat_tool_history(body: &Value) -> Result<(), String> {
    let messages = body["messages"].as_array().ok_or("messages missing")?;
    let mut pending = BTreeSet::new();
    for (index, message) in messages.iter().enumerate() {
        if message["role"] == "tool" {
            let call_id = message["tool_call_id"]
                .as_str()
                .ok_or("tool_call_id missing")?;
            if !pending.remove(call_id) {
                return Err(format!("message {index}: unexpected result {call_id}"));
            }
            continue;
        }
        if !pending.is_empty() {
            return Err(format!(
                "message {index}: unanswered tool calls {pending:?}"
            ));
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            if message["role"] != "assistant" {
                return Err("tool_calls outside assistant message".to_string());
            }
            for call in calls {
                let call_id = call["id"].as_str().ok_or("tool call id missing")?;
                if !pending.insert(call_id) {
                    return Err(format!("duplicate call id {call_id}"));
                }
            }
        }
    }
    if !pending.is_empty() {
        return Err(format!("unanswered tool calls {pending:?}"));
    }
    Ok(())
}

pub(super) fn assert_chat_tool_history(body: &Value) {
    assert_eq!(validate_chat_tool_history(body), Ok(()), "{body}");
}

pub(super) fn chat_tool_response() -> Value {
    json!({
        "id": "chatcmpl_parallel_fixture", "object": "chat.completion",
        "created": 1, "model": "fixture-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant", "content": null,
                "tool_calls": [
                    {"id": "tool_a", "type": "function", "function": {
                        "name": "read_file", "arguments": "{\"path\":\"first.txt\"}"}},
                    {"id": "tool_b", "type": "function", "function": {
                        "name": "read_file", "arguments": "{\"path\":\"second.txt\"}"}}
                ]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    })
}

pub(super) fn chat_tool_stream(delayed_ids: bool) -> Vec<u8> {
    let mut first = json!({"index": 0, "type": "function", "function": {
        "name": "read_file", "arguments": "{\"path\":"}});
    let mut second = json!({"index": 1, "type": "function", "function": {
        "name": "read_file", "arguments": "{\"path\":"}});
    if !delayed_ids {
        first["id"] = json!("tool_a");
        second["id"] = json!("tool_b");
    }
    let mut chunks = vec![
        json!({"id": "chatcmpl_parallel_fixture", "model": "fixture-model", "choices": [
            {"index": 0, "delta": {"role": "assistant", "reasoning_content": "Read both files."}, "finish_reason": null}
        ]}),
        json!({"id": "chatcmpl_parallel_fixture", "choices": [
            {"index": 0, "delta": {"tool_calls": [first, second]}, "finish_reason": null}
        ]}),
    ];
    if delayed_ids {
        chunks.push(json!({"id": "chatcmpl_parallel_fixture", "choices": [
            {"index": 0, "delta": {"tool_calls": [
                {"index": 1, "id": "tool_b"},
                {"index": 0, "id": "tool_a"}
            ]}, "finish_reason": null}
        ]}));
    }
    chunks.extend([
        json!({"id": "chatcmpl_parallel_fixture", "choices": [
            {"index": 0, "delta": {"tool_calls": [
                {"index": 1, "function": {"arguments": "\"second.txt\"}"}},
                {"index": 0, "function": {"arguments": "\"first.txt\"}"}}
            ]}, "finish_reason": null}
        ]}),
        json!({"id": "chatcmpl_parallel_fixture", "choices": [
            {"index": 0, "delta": {}, "finish_reason": "tool_calls"}
        ]}),
        json!({"id": "chatcmpl_parallel_fixture", "choices": [],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}),
    ]);
    let mut text = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect::<String>();
    text.push_str("data: [DONE]\n\n");
    text.into_bytes()
}

pub(super) fn sse_events(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .unwrap()
        .split("\n\n")
        .filter_map(|block| {
            let data = block
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .collect::<Vec<_>>()
                .join("\n");
            serde_json::from_str(&data).ok()
        })
        .collect()
}

pub(super) fn chat_tool_stream_with_late_text(delayed_ids: bool) -> Vec<u8> {
    let mut chunks = sse_events(&chat_tool_stream(delayed_ids));
    let finish_index = chunks
        .iter()
        .position(|chunk| chunk["choices"][0]["finish_reason"] == "tool_calls")
        .unwrap();
    chunks.insert(
        finish_index,
        json!({"id": "chatcmpl_parallel_fixture", "choices": [
            {"index": 0, "delta": {"content": "Reading both files now."}, "finish_reason": null}
        ]}),
    );
    let mut wire = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect::<String>();
    wire.push_str("data: [DONE]\n\n");
    wire.into_bytes()
}

pub(super) fn completed_response(events: &[Value]) -> Value {
    let completed = events
        .iter()
        .filter(|event| event["type"] == "response.completed")
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 1, "{events:?}");
    assert_eq!(completed[0]["response"]["status"], "completed");
    completed[0]["response"].clone()
}

#[test]
fn chat_contract_rejects_the_original_split_parallel_history() {
    let response = chat_tool_response();
    let calls = &response["choices"][0]["message"]["tool_calls"];
    let body = json!({"messages": [
        {"role": "assistant", "tool_calls": [calls[0].clone()]},
        {"role": "assistant", "tool_calls": [calls[1].clone()]},
        {"role": "tool", "tool_call_id": "tool_a", "content": "a"},
        {"role": "tool", "tool_call_id": "tool_b", "content": "b"}
    ]});
    assert!(validate_chat_tool_history(&body)
        .unwrap_err()
        .contains("unanswered tool calls"));
}

#[test]
fn chat_contract_rejects_missing_and_duplicate_results() {
    let response = chat_tool_response();
    let calls = &response["choices"][0]["message"]["tool_calls"];
    let mut body = json!({"messages": [
        {"role": "assistant", "tool_calls": calls},
        {"role": "tool", "tool_call_id": "tool_a", "content": "a"}
    ]});
    assert!(validate_chat_tool_history(&body).is_err());
    body["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role": "tool", "tool_call_id": "tool_a", "content": "a again"}));
    assert!(validate_chat_tool_history(&body)
        .unwrap_err()
        .contains("unexpected result"));
}
