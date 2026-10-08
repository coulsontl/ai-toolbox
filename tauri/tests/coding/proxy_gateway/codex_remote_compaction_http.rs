//! Codex remote compaction v2 over the HTTP route.
//!
//! The unit tests around `compat::codex_remote_compaction` cover the pure
//! functions; these drive the real gateway so the *mounting* is covered too —
//! that a v2 turn is pulled out of the Responses identity passthrough, rewritten,
//! sealed, and replayed on the next turn. An earlier revision wired the module
//! correctly but only ever proved it by hand against a live upstream.

use super::support::*;
use ai_toolbox_lib::http_client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

const CAPSULE_PREFIX: &str = "ai-toolbox-compact-v1:";
const COMPACTED_CONTEXT_HEADING: &str = "Context summary from previous turns:";
const SUMMARY_TEXT: &str = "## Goal\nShip the compaction passthrough";

/// A Codex v2 compaction turn: an ordinary Responses request whose input ends in
/// `compaction_trigger`, carrying the tools and base instructions Codex sends.
fn compaction_turn(streaming: bool) -> Value {
    json!({
        "model": "fixture-model",
        "stream": streaming,
        "instructions": "You are a coding agent running in the Codex CLI.",
        "parallel_tool_calls": true,
        "tool_choice": "auto",
        "tools": [{
            "type": "function", "name": "read_file",
            "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
        }],
        "input": [
            { "type": "message", "role": "user", "content": [
                { "type": "input_text", "text": "compact the session" }
            ] },
            { "type": "compaction_trigger" }
        ]
    })
}

fn responses_reply(text: &str) -> Value {
    json!({
        "id": "upstream-summary", "object": "response", "status": "completed",
        "model": "fixture-model",
        "output": [{ "type": "message", "role": "assistant",
            "content": [{ "type": "output_text", "text": text }] }],
        "usage": { "input_tokens": 21, "output_tokens": 9, "total_tokens": 30 }
    })
}

async fn accept_upstream(listener: &TcpListener) -> TcpStream {
    tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .expect("gateway must send the next upstream request")
        .unwrap()
        .0
}

async fn reply_with(socket: &mut TcpStream, reply: &Value) {
    write_mock_reply(socket, 200, &serde_json::to_vec(reply).unwrap(), false).await;
}

fn compaction_gateway(upstream_url: &str) -> RunningGateway {
    RunningGateway::new_for_api_format(
        upstream_url,
        "custom",
        BodyLogging::Disabled,
        "openai_responses",
    )
}

/// A v2 turn on the Responses identity route must leave it: the upstream sees a
/// plain summarization request, and the client gets a sealed compaction item
/// instead of the summary text.
#[tokio::test]
async fn identity_responses_compaction_is_rewritten_and_sealed() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        let body = read_mock_request(&mut socket, "/v1/responses ").await;

        let input = body["input"].as_array().unwrap();
        assert!(
            !input.iter().any(|item| item["type"] == "compaction_trigger"),
            "the trigger leaked upstream: {body}"
        );
        assert_eq!(
            input.last().unwrap()["content"][0]["text"],
            "Produce the continuation summary now."
        );
        for stripped in ["tools", "tool_choice", "parallel_tool_calls"] {
            assert!(
                body.get(stripped).is_none(),
                "`{stripped}` leaked upstream: {body}"
            );
        }
        let instructions = body["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("continuation summary"),
            "the summarization instruction did not replace Codex's: {instructions}"
        );
        assert!(
            !instructions.contains("Codex CLI"),
            "Codex's coding instructions leaked upstream: {instructions}"
        );
        assert_eq!(
            body["stream"],
            json!(false),
            "the summary must be fetched in one piece"
        );

        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let events = sse_events(&bytes);
    assert_eq!(events.len(), 5, "{events:?}");
    let completed = completed_response(&events);
    let output = completed["output"].as_array().unwrap();
    assert_eq!(
        output.len(),
        1,
        "Codex installs the returned item as the whole history: {completed}"
    );
    assert_eq!(output[0]["type"], "compaction");
    assert!(
        output[0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{completed}"
    );
    assert_eq!(completed["usage"]["input_tokens"], 21);
    assert_eq!(completed["usage"]["output_tokens"], 9);
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {}",
        String::from_utf8_lossy(&bytes)
    );

    upstream.await.unwrap();
}

/// A non-streaming client gets the same item as one JSON body. Codex always
/// streams, so this is the defensive branch, and it is the one that returns the
/// sealed item directly instead of re-rendering it as SSE.
#[tokio::test]
async fn a_non_streaming_client_still_gets_one_compaction_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        read_mock_request(&mut socket, "/v1/responses ").await;
        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(false))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["output"][0]["type"], "compaction");
    assert!(
        body["output"][0]["encrypted_content"]
            .as_str()
            .unwrap()
            .starts_with(CAPSULE_PREFIX),
        "{body}"
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Ship the compaction passthrough"),
        "the summary reached the client in the clear: {body}"
    );

    upstream.await.unwrap();
}

/// The next turn replays the item Codex kept. The upstream must receive readable
/// context, not an opaque capsule it cannot open.
#[tokio::test]
async fn a_replayed_capsule_becomes_readable_context_upstream() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        read_mock_request(&mut socket, "/v1/responses ").await;
        reply_with(&mut socket, &responses_reply(SUMMARY_TEXT)).await;

        let mut socket = accept_upstream(&listener).await;
        let body = read_mock_request(&mut socket, "/v1/responses ").await;
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["type"], "message", "{body}");
        assert_eq!(input[0]["role"], "assistant", "{body}");
        let text = input[0]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with(COMPACTED_CONTEXT_HEADING),
            "the summary must be labelled as background: {text}"
        );
        assert!(text.contains("Ship the compaction passthrough"), "{text}");
        assert!(
            !body.to_string().contains(CAPSULE_PREFIX),
            "the capsule reached an upstream that cannot open it: {body}"
        );
        assert!(
            body.get("tools").is_some(),
            "an ordinary turn keeps its tools: {body}"
        );
        assert!(
            !input.iter().any(|item| item["type"] == "compaction"),
            "the item must have been expanded, not passed through: {body}"
        );
        reply_with(&mut socket, &responses_reply("done")).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let first = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let bytes = first.bytes().await.unwrap();
    let item = completed_response(&sse_events(&bytes))["output"][0].clone();
    assert_eq!(item["type"], "compaction");

    let mut replay = compaction_turn(false);
    replay["input"] = json!([
        item,
        { "type": "message", "role": "user", "content": [
            { "type": "input_text", "text": "keep going" }
        ] }
    ]);
    let second = client
        .post(&gateway.url)
        .json(&replay)
        .send()
        .await
        .unwrap();
    let status = second.status();
    let bytes = second.bytes().await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));

    upstream.await.unwrap();
}

/// An unusable summary must fail the turn. Codex installs whatever item comes
/// back as the entire conversation history, so a placeholder would silently
/// discard the context the compaction was supposed to preserve.
#[tokio::test]
async fn an_empty_summary_fails_instead_of_faking_an_item() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let mut socket = accept_upstream(&listener).await;
        read_mock_request(&mut socket, "/v1/responses ").await;
        // 200, but nothing to summarize with.
        reply_with(&mut socket, &responses_reply("   ")).await;
    });

    let gateway = compaction_gateway(&upstream_url);
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&compaction_turn(true))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 400, "{}", String::from_utf8_lossy(&bytes));

    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"], "gateway_request_schema_rejected");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("empty compaction summary"),
        "{body}"
    );
    assert!(body.get("output").is_none(), "an item was faked: {body}");
    assert!(
        !String::from_utf8_lossy(&bytes).contains(CAPSULE_PREFIX),
        "a capsule was sealed from nothing: {body}"
    );

    upstream.await.unwrap();
}