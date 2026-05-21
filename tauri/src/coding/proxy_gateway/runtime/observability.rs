use super::http_io::{DebugHttpRequest, DebugHttpResponse};
use super::routes::split_request_target;
use super::GatewayRuntimeContext;
use crate::coding::proxy_gateway::request_log;
use crate::coding::proxy_gateway::types::{GatewayRequestLogDetail, GatewayRequestLogSummary};
use crate::coding::proxy_gateway::usage_stats;
use chrono::{DateTime, Utc};
use std::sync::OnceLock;

static TRACE_RUN_ID: OnceLock<String> = OnceLock::new();

pub(super) fn record_gateway_observability(
    request: &DebugHttpRequest,
    response: &DebugHttpResponse,
    context: &GatewayRuntimeContext,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) {
    let Some(paths) = context.paths.as_ref() else {
        return;
    };
    let (request_path, _) = split_request_target(&request.path);
    if should_skip_observability(&request.method, &request_path) {
        return;
    }

    let duration_ms = ended_at
        .signed_duration_since(started_at)
        .num_milliseconds()
        .max(0) as u64;
    let input_tokens = response.token_usage.input_tokens;
    let output_tokens = response.token_usage.output_tokens;
    let cache_read_tokens = response.token_usage.cache_read_tokens;
    let cache_creation_tokens = response.token_usage.cache_creation_tokens;
    let total_tokens = response.token_usage.total_tokens();
    let settings = context.settings_snapshot();
    let trace_id = trace_id(request);

    let should_record_summary = settings.request_log_enabled || settings.metrics_enabled;
    if should_record_summary {
        let mut detail = GatewayRequestLogDetail {
            summary: GatewayRequestLogSummary {
                trace_id,
                started_at,
                ended_at,
                cli_key: response.cli_key,
                route_name: response.route_name.clone(),
                method: request.method.clone(),
                path: request.path.clone(),
                provider_id: response.provider_id.clone(),
                provider_name: response.provider_name.clone(),
                provider_type: response.provider_type.clone(),
                cost_multiplier: response.cost_multiplier.clone(),
                pricing_model_source: response.pricing_model_source.clone(),
                requested_model: response.requested_model.clone(),
                upstream_model_id: response.upstream_model_id.clone(),
                upstream_url: response.upstream_url.clone(),
                status_code: Some(response.status_code),
                success: is_success_status(response.status_code),
                error_category: response.error_category.clone(),
                error_message: (!is_success_status(response.status_code))
                    .then(|| response.note.clone()),
                duration_ms,
                attempt_count: response.provider_attempt_count.max(1),
                total_attempt_count: response.attempt_count.max(1),
                failover: response.failover,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                total_tokens,
                request_body_bytes: request.body.len() as u64,
                response_body_bytes: response.response_body_bytes,
                is_streaming: response.is_streaming,
                first_token_ms: response.first_token_ms,
                detail_file: None,
                detail_offset: None,
            },
            request_headers: settings
                .store_headers
                .then(|| request_log::redact_headers(&request.headers)),
            request_body: stored_body_text(
                &request.body,
                request.body.len() as u64,
                settings.store_request_body,
                settings.log_max_body_size_kb,
            ),
            upstream_request_body: response.upstream_request_body.as_deref().and_then(|body| {
                stored_body_text(
                    body,
                    body.len() as u64,
                    settings.store_request_body,
                    settings.log_max_body_size_kb,
                )
            }),
            response_headers: settings
                .store_headers
                .then(|| request_log::redact_headers(&response.headers)),
            response_body: stored_body_text(
                &response.body,
                response.response_body_bytes,
                settings.store_response_body,
                settings.log_max_body_size_kb,
            ),
            provider_attempts: response.provider_attempts.clone(),
        };

        if settings.request_log_enabled {
            let record = request_log::new_request_log_record(detail.clone());
            match request_log::write_request_log(paths, &settings, &record) {
                Ok(Some(location)) => {
                    detail.summary.detail_file = Some(location.detail_file);
                    detail.summary.detail_offset = Some(location.detail_offset);
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn!("Failed to record proxy gateway request detail: {error}");
                }
            }
        }

        if let Some(db) = context.db.as_ref() {
            if let Err(error) = usage_stats::record_request_summary(db, &settings, &detail) {
                log::warn!("Failed to record proxy gateway request summary: {error}");
            }
        }
    }
}

fn should_skip_observability(method: &str, request_path: &str) -> bool {
    if method == "GET" && request_path == "/health" {
        return true;
    }
    matches!(method, "GET" | "HEAD")
        && matches!(request_path, "/anthropic" | "/openai/v1" | "/gemini/v1beta")
}

fn trace_id(request: &DebugHttpRequest) -> String {
    let run_id = TRACE_RUN_ID
        .get_or_init(|| format!("{}-{}", std::process::id(), Utc::now().timestamp_micros()));
    format!("gw-{}-{}", run_id, request.id)
}

fn is_success_status(status_code: u16) -> bool {
    (200..=399).contains(&status_code)
}

fn stored_body_text(
    body: &[u8],
    original_len: u64,
    enabled: bool,
    max_body_size_kb: u64,
) -> Option<String> {
    if !enabled {
        return None;
    }
    let max_bytes = max_body_size_kb.saturating_mul(1024) as usize;
    if max_bytes == 0 {
        return Some(String::new());
    }
    if original_len <= max_bytes as u64 {
        return Some(String::from_utf8_lossy(body).to_string());
    }
    let mut text = String::from_utf8_lossy(&body[..body.len().min(max_bytes)]).to_string();
    text.push_str(&format!(
        "\n[truncated: stored {} of {} bytes]",
        body.len().min(max_bytes),
        original_len
    ));
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_id(id: u64) -> DebugHttpRequest {
        DebugHttpRequest {
            id,
            peer_addr: "127.0.0.1:50000".parse().unwrap(),
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            version: "HTTP/1.1".to_string(),
            first_line: "POST /anthropic/v1/messages HTTP/1.1".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            raw_len: 0,
        }
    }

    #[test]
    fn trace_id_contains_process_run_prefix() {
        let trace = trace_id(&request_with_id(1));

        assert!(trace.starts_with("gw-"));
        assert!(trace.ends_with("-1"));
        assert_ne!(trace, "gw-1");
    }

    #[test]
    fn skips_cli_root_probe_observability() {
        assert!(should_skip_observability("HEAD", "/anthropic"));
        assert!(should_skip_observability("GET", "/openai/v1"));
        assert!(should_skip_observability("HEAD", "/gemini/v1beta"));
        assert!(!should_skip_observability("POST", "/anthropic"));
        assert!(!should_skip_observability("POST", "/anthropic/v1/messages"));
    }
}
