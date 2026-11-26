use super::support::*;
use ai_toolbox_lib::coding::proxy_gateway::{types::GatewayRequestLogFilters, usage_stats};
use ai_toolbox_lib::http_client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpListener;

/// A zero `message_start` input snapshot must not mask the final counts the
/// upstream reports in `message_delta`, after real protocol conversion and
/// SQLite persistence. Disabling request-body logging must not lose the usage
/// summary either: `metrics_enabled` still writes the compact row.
#[tokio::test]
async fn anthropic_final_usage_survives_http_conversion_and_sqlite_without_body_logging() {
    for logging in [BodyLogging::Enabled, BodyLogging::Disabled] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let gateway = RunningGateway::new_for_api_format(
            &format!("http://{}", listener.local_addr().unwrap()),
            "custom",
            logging,
            "anthropic",
        );
        let upstream = tokio::spawn(async move {
            let (mut socket, _) =
                tokio::time::timeout(Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let request = read_mock_request(&mut socket, "/v1/messages").await;
            assert_eq!(request["stream"], true);
            let body = concat!(
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_final_usage\",\"model\":\"fixture-model\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n",
                "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
                "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":17,\"output_tokens\":5,\"cache_read_input_tokens\":2,\"cache_creation_input_tokens\":3}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            );
            write_mock_reply(&mut socket, 200, body.as_bytes(), true).await;
        });
        let client = http_client::create_client_no_proxy(10).unwrap();
        let response = client
            .post(&gateway.url)
            .json(&json!({
                "model":"fixture-model", "stream":true,
                "input":[{"role":"user","content":"say hi"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.text().await.unwrap();
        upstream.await.unwrap();
        let values: Vec<Value> = body
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data: ")
                    .and_then(|data| serde_json::from_str(data).ok())
            })
            .collect();
        let completed: Vec<_> = values
            .iter()
            .filter(|value| value["type"] == "response.completed")
            .collect();
        assert_eq!(completed.len(), 1, "{body}");
        assert!(body.contains("hi"));
        let usage = &completed[0]["response"]["usage"];
        assert_eq!(usage["input_tokens"], 22);
        assert_eq!(usage["output_tokens"], 5);
        assert_eq!(usage["input_tokens_details"]["cached_tokens"], 2);
        assert_eq!(usage["input_tokens_details"]["cache_write_tokens"], 3);

        // The summary is written after the response body, so wait for it
        // independently. Turning body logging off must still keep real usage.
        let row = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = usage_stats::request_logs(
                    &gateway.db,
                    &GatewayRequestLogFilters::default(),
                    0,
                    10,
                    false,
                )
                .unwrap();
                if let Some(row) = page.data.into_iter().next() {
                    break row;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("converted usage must be persisted");
        assert_eq!(row.input_tokens, 17);
        assert_eq!(row.output_tokens, 5);
        assert_eq!(row.cache_read_tokens, 2);
        assert_eq!(row.cache_creation_tokens, 3);
        assert_eq!(row.total_tokens, 27);
    }
}

/// An upstream that reports the whole context window in `message_start` and the
/// corrected fresh input in `message_delta` must be billed by the correction.
/// Before the merged stream adopted a smaller positive delta input, the start
/// estimate (26,161) masked the real fresh count (6,737) and the dashboard
/// undercounted the cache hit rate.
#[tokio::test]
async fn anthropic_smaller_delta_input_replaces_context_estimate_in_sqlite() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let gateway = RunningGateway::new_for_api_format(
        &format!("http://{}", listener.local_addr().unwrap()),
        "custom",
        BodyLogging::Disabled,
        "anthropic",
    );
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let request = read_mock_request(&mut socket, "/v1/messages").await;
        assert_eq!(request["stream"], true);
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_corrected_usage\",\"model\":\"fixture-model\",\"usage\":{\"input_tokens\":26161,\"output_tokens\":0,\"cache_read_input_tokens\":18000,\"cache_creation_input_tokens\":0}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":6737,\"output_tokens\":200,\"cache_read_input_tokens\":51968,\"cache_creation_input_tokens\":0}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        write_mock_reply(&mut socket, 200, body.as_bytes(), true).await;
    });
    let client = http_client::create_client_no_proxy(10).unwrap();
    let response = client
        .post(&gateway.url)
        .json(&json!({
            "model":"fixture-model", "stream":true,
            "input":[{"role":"user","content":"say hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    upstream.await.unwrap();
    let values: Vec<Value> = body
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data: ")
                .and_then(|data| serde_json::from_str(data).ok())
        })
        .collect();
    let completed: Vec<_> = values
        .iter()
        .filter(|value| value["type"] == "response.completed")
        .collect();
    assert_eq!(completed.len(), 1, "{body}");
    let usage = &completed[0]["response"]["usage"];
    assert_eq!(usage["input_tokens"], 6737_u64 + 51968);
    assert_eq!(usage["output_tokens"], 200);
    assert_eq!(usage["input_tokens_details"]["cached_tokens"], 51968);

    let row = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = usage_stats::request_logs(
                &gateway.db,
                &GatewayRequestLogFilters::default(),
                0,
                10,
                false,
            )
            .unwrap();
            if let Some(row) = page.data.into_iter().next() {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("corrected usage must be persisted");
    assert_eq!(row.input_tokens, 6737);
    assert_eq!(row.output_tokens, 200);
    assert_eq!(row.cache_read_tokens, 51968);
    assert_eq!(row.total_tokens, 6737 + 51968 + 200);
}
