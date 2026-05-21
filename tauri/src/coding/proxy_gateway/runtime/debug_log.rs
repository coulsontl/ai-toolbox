use super::http_io::{DebugHttpRequest, DebugHttpResponse};
use super::providers::UpstreamProvider;
use crate::coding::proxy_gateway::request_log;
use crate::coding::proxy_gateway::types::ProxyGatewaySettings;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;

pub(super) fn log_incoming_request(request: &DebugHttpRequest, settings: &ProxyGatewaySettings) {
    if !should_log_summary(settings) {
        return;
    }
    println!(
        "[proxy-gateway] request_begin id={} peer={} raw_bytes={} first_line={}",
        request.id, request.peer_addr, request.raw_len, request.first_line
    );
    println!(
        "[proxy-gateway] request_line id={} method={} path={} version={}",
        request.id, request.method, request.path, request.version
    );
    if should_log_headers(settings) {
        println!(
            "[proxy-gateway] request_headers_begin id={} count={}",
            request.id,
            request.headers.len()
        );
        for (name, value) in &request.headers {
            println!(
                "[proxy-gateway] request_header id={} {}: {}",
                request.id,
                name,
                format_header_text_value_for_debug(name, value)
            );
        }
        println!("[proxy-gateway] request_headers_end id={}", request.id);
    }
    if should_log_body(settings) {
        println!(
            "[proxy-gateway] request_body_begin id={} bytes={}",
            request.id,
            request.body.len()
        );
        if request.body.is_empty() {
            println!("[proxy-gateway] request_body id={} <empty>", request.id);
        } else {
            println!(
                "[proxy-gateway] request_body id={}\n{}",
                request.id,
                format_body_for_debug_log(&request.body)
            );
        }
        println!("[proxy-gateway] request_body_end id={}", request.id);
    }
}

pub(super) fn format_body_for_debug_log(body: &[u8]) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }

    let text = String::from_utf8_lossy(body);
    let Ok(mut json) = serde_json::from_str::<Value>(&text) else {
        return text.to_string();
    };

    sanitize_debug_json(&mut json);
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| text.to_string())
}

fn sanitize_debug_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if is_sensitive_json_key(key) {
                    *child = Value::String("[REDACTED]".to_string());
                    continue;
                }
                sanitize_debug_json(child);
            }
            if let Some(messages) = object.get_mut("messages") {
                *messages = summarize_omitted_json_value(messages);
            }
        }
        Value::Array(items) => {
            for child in items {
                sanitize_debug_json(child);
            }
        }
        _ => {}
    }
}

fn summarize_omitted_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => {
            Value::String(format!("[omitted messages array: {} items]", items.len()))
        }
        Value::Object(object) => {
            Value::String(format!("[omitted messages object: {} keys]", object.len()))
        }
        Value::String(text) => {
            Value::String(format!("[omitted messages string: {} chars]", text.len()))
        }
        _ => Value::String("[omitted messages]".to_string()),
    }
}

pub(super) fn log_gateway_decision(
    request: &DebugHttpRequest,
    response: &DebugHttpResponse,
    settings: &ProxyGatewaySettings,
) {
    if !should_log_summary(settings) {
        return;
    }
    println!(
        "[proxy-gateway] route_decision id={} route={} upstream={} note={}",
        request.id,
        response.route_name,
        response.upstream_url.as_deref().unwrap_or("<none>"),
        response.note
    );
}

pub(super) fn log_response(
    request: &DebugHttpRequest,
    response: &DebugHttpResponse,
    settings: &ProxyGatewaySettings,
) {
    if !should_log_summary(settings) {
        return;
    }
    println!(
        "[proxy-gateway] response_begin id={} status={} {} body_bytes={}",
        request.id, response.status_code, response.status_text, response.response_body_bytes
    );
    if should_log_headers(settings) {
        for (name, value) in &response.headers {
            println!(
                "[proxy-gateway] response_header id={} {}: {}",
                request.id,
                name,
                format_header_text_value_for_debug(name, value)
            );
        }
        println!(
            "[proxy-gateway] response_header id={} Content-Length: {}",
            request.id, response.response_body_bytes
        );
    }
    if should_log_body(settings) {
        println!(
            "[proxy-gateway] response_body id={}\n{}",
            request.id,
            format_body_for_debug_log(&response.body)
        );
    }
    println!("[proxy-gateway] response_end id={}", request.id);
}

pub(super) fn log_upstream_request(
    request: &DebugHttpRequest,
    provider: &UpstreamProvider,
    upstream_url: &reqwest::Url,
    headers: &HeaderMap,
    upstream_body: &[u8],
    settings: &ProxyGatewaySettings,
) {
    if !should_log_summary(settings) {
        return;
    }
    println!(
        "[proxy-gateway] upstream_request_begin id={} provider_id={} provider_name={} cli={} method={} url={} body_bytes={}",
        request.id,
        provider.id,
        provider.name,
        provider.cli_key.as_str(),
        request.method,
        upstream_url,
        upstream_body.len()
    );
    if should_log_headers(settings) {
        println!(
            "[proxy-gateway] upstream_request_headers_begin id={} count={}",
            request.id,
            headers.len()
        );
        for (name, value) in headers {
            println!(
                "[proxy-gateway] upstream_request_header id={} {}: {}",
                request.id,
                name,
                format_header_value_for_debug(name.as_str(), value)
            );
        }
        println!(
            "[proxy-gateway] upstream_request_headers_end id={}",
            request.id
        );
    }
    if should_log_body(settings) {
        if upstream_body.is_empty() {
            println!(
                "[proxy-gateway] upstream_request_body id={} <empty>",
                request.id
            );
        } else {
            println!(
                "[proxy-gateway] upstream_request_body id={}\n{}",
                request.id,
                format_body_for_debug_log(upstream_body)
            );
        }
    }
    println!("[proxy-gateway] upstream_request_end id={}", request.id);
}

pub(super) fn log_upstream_response(
    request: &DebugHttpRequest,
    response: &DebugHttpResponse,
    settings: &ProxyGatewaySettings,
) {
    if !should_log_summary(settings) {
        return;
    }
    println!(
        "[proxy-gateway] upstream_response_begin id={} status={} {} body_bytes={}",
        request.id, response.status_code, response.status_text, response.response_body_bytes
    );
    if should_log_headers(settings) {
        for (name, value) in &response.headers {
            println!(
                "[proxy-gateway] upstream_response_header id={} {}: {}",
                request.id,
                name,
                format_header_text_value_for_debug(name, value)
            );
        }
    }
    if should_log_body(settings) {
        if response.body.is_empty() {
            println!(
                "[proxy-gateway] upstream_response_body id={} <empty>",
                request.id
            );
        } else {
            println!(
                "[proxy-gateway] upstream_response_body id={}\n{}",
                request.id,
                format_body_for_debug_log(&response.body)
            );
        }
    }
    println!("[proxy-gateway] upstream_response_end id={}", request.id);
}

fn format_header_value_for_debug(name: &str, value: &HeaderValue) -> String {
    let value = value.to_str().unwrap_or("<non-utf8>");
    format_header_text_value_for_debug(name, value)
}

fn format_header_text_value_for_debug(name: &str, value: &str) -> String {
    if is_sensitive_header(name) {
        mask_secret(value)
    } else {
        value.to_string()
    }
}

fn is_sensitive_header(name: &str) -> bool {
    request_log::is_sensitive_header(&name.to_ascii_lowercase())
}

fn is_sensitive_json_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    request_log::is_sensitive_header(&normalized)
        || normalized.contains("api_key")
        || normalized.contains("apikey")
        || normalized.contains("access_token")
        || normalized.contains("refresh_token")
        || normalized.contains("id_token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized == "key"
}

fn should_log_summary(settings: &ProxyGatewaySettings) -> bool {
    settings.request_log_enabled && settings.request_log_level != "off"
}

fn should_log_headers(settings: &ProxyGatewaySettings) -> bool {
    should_log_summary(settings)
        && matches!(settings.request_log_level.as_str(), "headers" | "full")
}

fn should_log_body(settings: &ProxyGatewaySettings) -> bool {
    should_log_summary(settings) && matches!(settings.request_log_level.as_str(), "body" | "full")
}

fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    let char_count = trimmed.chars().count();
    if char_count <= 12 {
        return "***".to_string();
    }
    let head: String = trimmed.chars().take(6).collect();
    let tail: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}...{tail}")
}
