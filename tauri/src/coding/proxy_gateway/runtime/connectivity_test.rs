use super::http_io::{self, DebugHttpRequest};
use super::upstream::{
    gateway_body_reports_error, route_request_with_options, GatewayRequestOptions,
};
use super::{providers, GatewayRuntimeContext, NEXT_REQUEST_ID};
use crate::coding::open_code::models_api::{
    vision_probe_confirmed, VISION_PROBE_EXPECTED_TOKEN, VISION_PROBE_IMAGE_BASE64,
};
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
    if super::super::provider_protocol::native_cli_protocol(request.cli_key).is_none() {
        return Err(format!(
            "{} does not support Gateway connectivity testing",
            request.cli_key.as_str()
        ));
    }

    // Resolve (and validate) the provider up front so a missing, disabled or
    // official provider fails with a clear message before any model loop runs.
    providers::load_provider_by_id_for_connectivity_test(
        &db,
        request.cli_key,
        &request.provider_id,
        Some(&settings),
    )
    .await?;

    // A provider whose target protocol already matches the CLI's native protocol is
    // an identity/passthrough route: takeover, provider compat and forwarding still
    // apply, so the test runs it instead of refusing with "does not require
    // protocol conversion".
    let vision_probe = request.vision_probe && supports_gateway_vision_probe(request.cli_key);
    let stream = gateway_connectivity_stream(&request);
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
            results.push(empty_gateway_result(model_id, "Missing model"));
            continue;
        }
        let prompt = if vision_probe {
            vision_probe_prompt(&request.prompt)
        } else {
            request.prompt.clone()
        };
        let debug_request = build_gateway_connectivity_request(
            request.cli_key,
            &model_id,
            &prompt,
            stream,
            vision_probe,
        )?;
        let mut result = run_gateway_connectivity_request(
            &context,
            &options,
            &test_settings,
            debug_request,
            &model_id,
            timeout_secs,
        )
        .await;
        if vision_probe {
            if let Some(response_body) = result.response_body.as_ref() {
                match classify_gateway_vision(request.cli_key, response_body) {
                    Ok(()) => result.vision_status = Some("passed".to_string()),
                    Err(reason) => {
                        result.vision_status = Some("failed".to_string());
                        result.vision_error = Some(reason);
                    }
                }
            } else {
                result.vision_status = Some("failed".to_string());
                result.vision_error = Some("the probe returned no body to inspect".to_string());
            }
        } else if request.vision_probe {
            result.vision_status = Some("unavailable".to_string());
        }
        results.push(result);
    }

    Ok(GatewayConnectivityTestResponse { results })
}

fn empty_gateway_result(model_id: String, message: &str) -> GatewayConnectivityTestResult {
    GatewayConnectivityTestResult {
        model_id,
        status: "error".to_string(),
        first_byte_ms: None,
        total_ms: None,
        error_message: Some(message.to_string()),
        request_url: String::new(),
        request_headers: json!({}),
        request_body: json!({}),
        response_headers: None,
        response_body: None,
        status_code: None,
        status_text: None,
        upstream_status_code: None,
        upstream_url: None,
        vision_status: None,
        vision_error: None,
    }
}

fn vision_probe_prompt(base_prompt: &str) -> String {
    format!(
        "{base_prompt}\n\nThe message also contains a 1x1 image. If you can actually see images, reply with exactly `{VISION_PROBE_EXPECTED_TOKEN}` and nothing else. If you cannot process images, reply `NO_IMAGE`.",
    )
}

/// All gateway-supported CLIs except Kimi use a relay that passes the image
/// block through; Kimi's native chat protocol has no image part.
fn supports_gateway_vision_probe(cli_key: GatewayCliKey) -> bool {
    matches!(
        cli_key,
        GatewayCliKey::Claude
            | GatewayCliKey::ClaudeDesktop
            | GatewayCliKey::Codex
            | GatewayCliKey::Grok
            | GatewayCliKey::Gemini
    )
}

/// Whether the connectivity request must be sent as non-streaming.
///
/// A vision probe has to inspect a JSON body, but the gateway only aggregates an
/// upstream SSE stream when the client asked for a non-streaming response (see
/// `sse_aggregation_kind_for_non_streaming_client`). A streaming probe would hand
/// `classify_gateway_vision` raw SSE text instead, which has no `content`/`output`
/// structure and would always report "failed". Everything else keeps the
/// caller's preference.
fn gateway_connectivity_stream(request: &GatewayConnectivityTestRequest) -> bool {
    if request.vision_probe && supports_gateway_vision_probe(request.cli_key) {
        false
    } else {
        request.stream.unwrap_or(true)
    }
}

fn classify_gateway_vision(cli_key: GatewayCliKey, response_body: &Value) -> Result<(), String> {
    // The gateway relays the CLI's native protocol back, so the body is in that
    // shape. The probe always runs non-streaming (`gateway_connectivity_stream`),
    // which is what makes the gateway aggregate the upstream SSE into this single
    // JSON body in the first place.
    let text = match cli_key {
        GatewayCliKey::Claude | GatewayCliKey::ClaudeDesktop => response_body
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        GatewayCliKey::Codex | GatewayCliKey::Grok => response_body
            .get("output")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("content").and_then(Value::as_array))
                    .flatten()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        GatewayCliKey::Gemini => response_body
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        // Kimi (no image part), OpenCode and Antigravity are excluded by
        // `supports_gateway_vision_probe`, so this arm is unreachable at runtime;
        // it returns no text rather than guessing a response shape and only keeps
        // the match exhaustive.
        GatewayCliKey::Kimi
        | GatewayCliKey::OpenCode
        | GatewayCliKey::Antigravity
        | GatewayCliKey::Workbuddy
        | GatewayCliKey::Codebuddy => {
            String::new()
        }
    };
    if vision_probe_confirmed(&text) {
        Ok(())
    } else {
        Err(format!(
            "model did not confirm image input (response: {})",
            text.trim().chars().take(120).collect::<String>()
        ))
    }
}

fn build_gateway_connectivity_request(
    cli_key: GatewayCliKey,
    model_id: &str,
    prompt: &str,
    stream: bool,
    vision_probe: bool,
) -> Result<DebugHttpRequest, String> {
    let data_url = format!("data:image/png;base64,{VISION_PROBE_IMAGE_BASE64}");
    let body = match cli_key {
        GatewayCliKey::Claude | GatewayCliKey::ClaudeDesktop => {
            let mut content = vec![json!({ "type": "text", "text": prompt })];
            if vision_probe {
                content.push(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": VISION_PROBE_IMAGE_BASE64,
                    }
                }));
            }
            json!({
                "model": model_id,
                "max_tokens": 1024,
                "messages": [{ "role": "user", "content": content }],
                "stream": stream,
            })
        }
        GatewayCliKey::Codex | GatewayCliKey::Grok => {
            let mut content = vec![json!({ "type": "input_text", "text": prompt })];
            if vision_probe {
                content.push(json!({ "type": "input_image", "image_url": data_url }));
            }
            json!({
                "model": model_id,
                "input": [{ "type": "message", "role": "user", "content": content }],
                "stream": stream,
                "store": false,
            })
        }
        GatewayCliKey::Kimi | GatewayCliKey::Workbuddy | GatewayCliKey::Codebuddy => json!({
            "model": model_id,
            "messages": [{ "role": "user", "content": prompt }],
            "stream": stream,
        }),
        GatewayCliKey::Gemini | GatewayCliKey::Antigravity => {
            let mut parts = vec![json!({ "text": prompt })];
            if vision_probe {
                parts.push(json!({
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": VISION_PROBE_IMAGE_BASE64,
                    }
                }));
            }
            json!({
                "contents": [{ "role": "user", "parts": parts }],
            })
        }
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
        GatewayCliKey::ClaudeDesktop => "/claude-desktop/v1/messages".to_string(),
        GatewayCliKey::Codex => "/openai/v1/responses".to_string(),
        GatewayCliKey::Grok => "/grok/v1/responses".to_string(),
        GatewayCliKey::Kimi => "/kimi/v1/chat/completions".to_string(),
        GatewayCliKey::Gemini | GatewayCliKey::Antigravity => {
            let model = model_id
                .trim()
                .strip_prefix("models/")
                .unwrap_or_else(|| model_id.trim());
            let action = if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            };
            let prefix = if cli_key == GatewayCliKey::Antigravity {
                "antigravity"
            } else {
                "gemini"
            };
            format!(
                "/{prefix}/v1beta/models/{}:{}",
                encode_url_path_segment(model),
                action
            )
        }
        // Both family products speak OpenAI Chat: their custom model entries
        // point at a full `/chat/completions` URL, so the gateway also takes a
        // Chat request body.
        GatewayCliKey::Workbuddy => "/workbuddy/v1/chat/completions".to_string(),
        GatewayCliKey::Codebuddy => "/codebuddy/v1/chat/completions".to_string(),
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

    // Keep the real upstream code visible even when the gateway did not substitute a
    // synthetic status (same fallback upstream.rs applies before rewriting a failure).
    let upstream_status_code = response
        .upstream_status_code
        .or_else(|| response.upstream_url.as_ref().map(|_| response.status_code));

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
        status_code: Some(response.status_code),
        status_text: Some(response.status_text.clone()),
        upstream_status_code,
        upstream_url: response.upstream_url.clone(),
        vision_status: None,
        vision_error: None,
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
    use super::super::upstream::gateway_body_reports_error;
    use super::{
        build_gateway_connectivity_request, classify_gateway_vision, gateway_connectivity_stream,
        remaining_total_timeout, supports_gateway_vision_probe,
    };
    use crate::coding::proxy_gateway::types::{GatewayCliKey, GatewayConnectivityTestRequest};
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn gateway_vision_probe_injects_an_image_per_cli() {
        for cli_key in [
            GatewayCliKey::Claude,
            GatewayCliKey::Codex,
            GatewayCliKey::Grok,
            GatewayCliKey::Gemini,
        ] {
            let request =
                build_gateway_connectivity_request(cli_key, "m", "probe", false, true).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let body_text = body.to_string();
            assert!(
                body_text.contains("image"),
                "{cli_key:?} body should carry an image: {body_text}"
            );
        }

        // Kimi has no native image part and must stay text-only.
        let kimi =
            build_gateway_connectivity_request(GatewayCliKey::Kimi, "m", "probe", false, true)
                .unwrap();
        let kimi_body: serde_json::Value = serde_json::from_slice(&kimi.body).unwrap();
        assert!(!kimi_body.to_string().contains("image"));
    }

    #[test]
    fn gateway_vision_classification_is_protocol_aware() {
        assert!(classify_gateway_vision(
            GatewayCliKey::Claude,
            &json!({ "content": [{ "type": "text", "text": "OK" }] })
        )
        .is_ok());
        assert!(classify_gateway_vision(
            GatewayCliKey::Codex,
            &json!({ "output": [{ "content": [{ "type": "output_text", "text": "NO_IMAGE" }] }] })
        )
        .is_err());
        assert!(classify_gateway_vision(
            GatewayCliKey::Gemini,
            &json!({ "candidates": [{ "content": { "parts": [{ "text": "ok" }] } }] })
        )
        .is_ok());
        // A longer reply that merely mentions "OK" is not proof of vision.
        assert!(classify_gateway_vision(
            GatewayCliKey::Claude,
            &json!({ "content": [{ "type": "text", "text": "OK, I'll help" }] })
        )
        .is_err());

        assert!(supports_gateway_vision_probe(GatewayCliKey::Claude));
        assert!(!supports_gateway_vision_probe(GatewayCliKey::Kimi));
    }

    #[test]
    fn gateway_vision_probe_forces_a_non_streaming_request() {
        let request = |cli_key: GatewayCliKey, vision_probe: bool, stream: Option<bool>| {
            GatewayConnectivityTestRequest {
                cli_key,
                provider_id: "p".to_string(),
                prompt: "probe".to_string(),
                stream,
                model_ids: vec!["m".to_string()],
                timeout_secs: None,
                vision_probe,
            }
        };

        // A supported probe must be non-streaming so the gateway aggregates the
        // upstream SSE into a JSON body the classifier can actually read.
        assert!(!gateway_connectivity_stream(&request(
            GatewayCliKey::Claude,
            true,
            Some(true)
        )));
        // Without a probe, or on a CLI the probe skips, keep the caller's choice.
        assert!(gateway_connectivity_stream(&request(
            GatewayCliKey::Claude,
            false,
            Some(true)
        )));
        assert!(gateway_connectivity_stream(&request(
            GatewayCliKey::Kimi,
            true,
            Some(true)
        )));
    }

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
