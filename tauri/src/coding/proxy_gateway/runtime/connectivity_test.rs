use super::http_io::{self, DebugHttpRequest};
use super::upstream::{route_request_with_options, GatewayRequestOptions};
use super::{providers, GatewayRuntimeContext, NEXT_REQUEST_ID};
use crate::coding::proxy_gateway::types::{
    AppProxyConfig, GatewayCliKey, GatewayConnectivityTestRequest, GatewayConnectivityTestResponse,
    GatewayConnectivityTestResult, ProxyGatewaySettings,
};
use crate::coding::url_utils::encode_url_path_segment;
use crate::db::SqliteDbState;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub(crate) async fn test_gateway_provider_model_connectivity(
    settings: ProxyGatewaySettings,
    db: SqliteDbState,
    request: GatewayConnectivityTestRequest,
) -> Result<GatewayConnectivityTestResponse, String> {
    let Some(native_protocol) =
        super::super::provider_protocol::native_cli_protocol(request.cli_key)
    else {
        return Err(format!(
            "{} does not support Gateway connectivity testing",
            request.cli_key.as_str()
        ));
    };

    let provider = providers::load_provider_by_id_for_connectivity_test(
        &db,
        request.cli_key,
        &request.provider_id,
        Some(&settings),
    )
    .await?;

    if provider.target_protocol == native_protocol {
        return Err(format!(
            "Provider '{}' does not require Gateway protocol conversion",
            provider.name
        ));
    }

    let stream = request.stream.unwrap_or(true);
    let timeout_secs = request.timeout_secs.unwrap_or(30).max(1);
    let mut test_settings = settings;
    test_settings.request_log_enabled = false;
    test_settings.metrics_enabled = false;
    test_settings.store_request_body = false;
    test_settings.store_headers = false;
    test_settings.store_response_body = false;
    let app_config = test_settings
        .app_configs
        .entry(request.cli_key)
        .or_insert_with(AppProxyConfig::default);
    app_config.streaming_first_byte_timeout_secs = Some(timeout_secs);
    app_config.streaming_idle_timeout_secs = Some(timeout_secs);
    app_config.non_streaming_timeout_secs = Some(timeout_secs);
    app_config.per_provider_retry_count = Some(0);
    app_config.max_retry_count = Some(0);
    app_config.retry_interval_secs = Some(0);

    let context = GatewayRuntimeContext::new(test_settings.clone(), Some(db), None);
    let options = GatewayRequestOptions {
        provider_override_id: Some(request.provider_id.clone()),
        disable_health_mutation: true,
    };

    let mut results = Vec::new();
    for model_id in request.model_ids {
        if model_id.trim().is_empty() {
            results.push(GatewayConnectivityTestResult {
                model_id,
                status: "error".to_string(),
                first_byte_ms: None,
                total_ms: None,
                error_message: Some("Missing model".to_string()),
                request_url: String::new(),
                request_headers: json!({}),
                request_body: json!({}),
                response_headers: None,
                response_body: None,
            });
            continue;
        }
        let debug_request = build_gateway_connectivity_request(
            request.cli_key,
            &model_id,
            &request.prompt,
            stream,
        )?;
        let result = run_gateway_connectivity_request(
            &context,
            &options,
            &test_settings,
            debug_request,
            &model_id,
            timeout_secs,
        )
        .await;
        results.push(result);
    }

    Ok(GatewayConnectivityTestResponse { results })
}

fn build_gateway_connectivity_request(
    cli_key: GatewayCliKey,
    model_id: &str,
    prompt: &str,
    stream: bool,
) -> Result<DebugHttpRequest, String> {
    let body = match cli_key {
        GatewayCliKey::Claude => json!({
            "model": model_id,
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": prompt }
                    ]
                }
            ],
            "stream": stream,
        }),
        GatewayCliKey::Codex | GatewayCliKey::Grok => json!({
            "model": model_id,
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": prompt }
                    ]
                }
            ],
            "stream": stream,
            "store": false,
        }),
        GatewayCliKey::Gemini => json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [
                        { "text": prompt }
                    ]
                }
            ],
        }),
        GatewayCliKey::OpenCode => {
            return Err(
                "OpenCode adapter is intentionally out of scope for the gateway MVP".to_string(),
            )
        }
    };
    let body_bytes = serde_json::to_vec(&body)
        .map_err(|error| format!("Failed to serialize gateway test body: {error}"))?;
    let path = gateway_connectivity_path(cli_key, model_id, stream);
    let mut headers = vec![
        ("Host".to_string(), "127.0.0.1".to_string()),
        (
            "Authorization".to_string(),
            "Bearer ai-toolbox-connectivity-test".to_string(),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Content-Length".to_string(), body_bytes.len().to_string()),
    ];
    if stream {
        headers.push(("Accept".to_string(), "text/event-stream".to_string()));
    }

    Ok(DebugHttpRequest {
        id: NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst),
        method: "POST".to_string(),
        path,
        headers,
        body: body_bytes,
    })
}

fn gateway_connectivity_path(cli_key: GatewayCliKey, model_id: &str, stream: bool) -> String {
    match cli_key {
        GatewayCliKey::Claude => "/anthropic/v1/messages".to_string(),
        GatewayCliKey::Codex => "/openai/v1/responses".to_string(),
        GatewayCliKey::Grok => "/grok/v1/responses".to_string(),
        GatewayCliKey::Gemini => {
            let model = model_id
                .trim()
                .strip_prefix("models/")
                .unwrap_or_else(|| model_id.trim());
            let action = if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            };
            format!(
                "/gemini/v1beta/models/{}:{}",
                encode_url_path_segment(model),
                action
            )
        }
        GatewayCliKey::OpenCode => "/".to_string(),
    }
}

async fn run_gateway_connectivity_request(
    context: &GatewayRuntimeContext,
    options: &GatewayRequestOptions,
    settings: &ProxyGatewaySettings,
    request: DebugHttpRequest,
    model_id: &str,
    timeout_secs: u64,
) -> GatewayConnectivityTestResult {
    let started = Instant::now();
    let request_url = request.path.clone();
    let request_headers = header_pairs_to_value(&request.headers);
    let request_body = parse_json_or_raw(&request.body);

    let total_timeout = Duration::from_secs(timeout_secs.max(1));
    let mut response = route_request_with_options(&request, context, options).await;
    let mut stream_error = None;
    if let Some(body_stream) = response.body_stream.take() {
        if let Some(remaining_timeout) = remaining_total_timeout(total_timeout, started.elapsed()) {
            match tokio::time::timeout(
                remaining_timeout,
                drain_gateway_body_stream(body_stream, &mut response, settings, started),
            )
            .await
            {
                Err(_) => {
                    stream_error =
                        Some("Gateway connectivity stream exceeded the total timeout".to_string())
                }
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    stream_error = Some(error);
                }
            }
        } else {
            stream_error =
                Some("Gateway connectivity stream exceeded the total timeout".to_string());
        }
    } else if response.first_token_ms.is_none() && !response.body.is_empty() {
        response.first_token_ms =
            Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
    }

    let total_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let body_has_content = !response.body.is_empty();
    let body_reports_error = gateway_body_reports_error(&response.body);
    let status = if (200..300).contains(&response.status_code)
        && stream_error.is_none()
        && body_has_content
        && !body_reports_error
    {
        "success"
    } else {
        "error"
    };
    let error_message = stream_error.or_else(|| {
        (!(200..300).contains(&response.status_code))
            .then(|| format!("Gateway API error: {}", response.status_code))
            .or_else(|| {
                (!body_has_content).then(|| "Gateway returned an empty response".to_string())
            })
            .or_else(|| {
                body_reports_error.then(|| "Gateway response contained an error event".to_string())
            })
    });

    GatewayConnectivityTestResult {
        model_id: model_id.to_string(),
        status: status.to_string(),
        first_byte_ms: response.first_token_ms.or(Some(total_ms)),
        total_ms: Some(total_ms),
        error_message,
        request_url,
        request_headers,
        request_body,
        response_headers: Some(header_pairs_to_value(&response.headers)),
        response_body: Some(parse_json_or_raw(&response.body)),
    }
}

fn remaining_total_timeout(total_timeout: Duration, elapsed: Duration) -> Option<Duration> {
    total_timeout
        .checked_sub(elapsed)
        .filter(|remaining| !remaining.is_zero())
}

async fn drain_gateway_body_stream(
    mut body_stream: http_io::DebugBodyStream,
    response: &mut http_io::DebugHttpResponse,
    settings: &ProxyGatewaySettings,
    started: Instant,
) -> Result<(), String> {
    let idle_timeout_secs = response
        .cli_key
        .map(|cli_key| {
            settings
                .effective_app_config(cli_key)
                .streaming_idle_timeout_secs
        })
        .unwrap_or(settings.streaming_idle_timeout_secs)
        .max(1);
    let idle_timeout = Duration::from_secs(idle_timeout_secs);
    response.body.clear();
    response.response_body_bytes = 0;

    loop {
        let next_chunk = tokio::time::timeout(idle_timeout, body_stream.next())
            .await
            .map_err(|_| {
                format!(
                    "Gateway stream was idle for {} seconds",
                    idle_timeout.as_secs()
                )
            })?;
        let Some(chunk_result) = next_chunk else {
            break;
        };
        let chunk = chunk_result?;
        if chunk.is_empty() {
            continue;
        }
        if response.first_token_ms.is_none() {
            response.first_token_ms =
                Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        }
        response.response_body_bytes = response
            .response_body_bytes
            .saturating_add(chunk.len() as u64);
        const MAX_RESPONSE_PREVIEW_BYTES: usize = 256 * 1024;
        let remaining_preview_bytes =
            MAX_RESPONSE_PREVIEW_BYTES.saturating_sub(response.body.len());
        if remaining_preview_bytes > 0 {
            response
                .body
                .extend_from_slice(&chunk[..chunk.len().min(remaining_preview_bytes)]);
        }
    }
    Ok(())
}

fn gateway_body_reports_error(body: &[u8]) -> bool {
    if body.is_empty() {
        return false;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        return gateway_json_reports_error(&value);
    }
    String::from_utf8_lossy(body).lines().any(|line| {
        let trimmed = line.trim();
        if trimmed
            .strip_prefix("event:")
            .map(str::trim)
            .is_some_and(gateway_event_reports_error)
        {
            return true;
        }
        let payload = trimmed
            .strip_prefix("data:")
            .map(str::trim)
            .unwrap_or(trimmed);
        serde_json::from_str::<Value>(payload)
            .ok()
            .is_some_and(|value| gateway_json_reports_error(&value))
    })
}

fn gateway_json_reports_error(value: &Value) -> bool {
    value.get("error").is_some_and(|error| !error.is_null())
        || value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(gateway_event_reports_error)
        || value
            .get("event")
            .and_then(Value::as_str)
            .is_some_and(gateway_event_reports_error)
        || value.get("response").is_some_and(|response| {
            response.get("error").is_some_and(|error| !error.is_null())
                || response
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(gateway_response_status_reports_error)
        })
}

fn gateway_event_reports_error(event: &str) -> bool {
    matches!(
        event.trim(),
        "error"
            | "response.failed"
            | "response.cancelled"
            | "response.canceled"
            | "response.incomplete"
    )
}

fn gateway_response_status_reports_error(status: &str) -> bool {
    matches!(
        status.trim(),
        "failed" | "cancelled" | "canceled" | "incomplete"
    )
}

fn header_pairs_to_value(headers: &[(String, String)]) -> Value {
    let mut object = serde_json::Map::new();
    for (name, value) in headers {
        object.insert(name.clone(), Value::String(value.clone()));
    }
    Value::Object(object)
}

fn parse_json_or_raw(body: &[u8]) -> Value {
    if body.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice::<Value>(body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).to_string()))
}

#[cfg(test)]
mod tests {
    use super::{gateway_body_reports_error, remaining_total_timeout};
    use std::time::Duration;

    #[test]
    fn gateway_body_error_detection_handles_json_and_sse() {
        assert!(gateway_body_reports_error(
            br#"{"error":{"message":"failed"}}"#
        ));
        assert!(gateway_body_reports_error(
            b"event: error\ndata: {\"type\":\"error\",\"message\":\"failed\"}\n\n"
        ));
        assert!(gateway_body_reports_error(
            b"event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\"error\":{\"message\":\"failed\"}}}\n\n"
        ));
        assert!(gateway_body_reports_error(
            br#"{"response":{"status":"failed","error":{"message":"failed"}}}"#
        ));
        assert!(!gateway_body_reports_error(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n"
        ));
    }

    #[test]
    fn total_timeout_only_exposes_time_remaining_after_routing() {
        assert_eq!(
            remaining_total_timeout(Duration::from_secs(30), Duration::from_secs(12)),
            Some(Duration::from_secs(18))
        );
        assert_eq!(
            remaining_total_timeout(Duration::from_secs(30), Duration::from_secs(30)),
            None
        );
        assert_eq!(
            remaining_total_timeout(Duration::from_secs(30), Duration::from_secs(31)),
            None
        );
    }
}
