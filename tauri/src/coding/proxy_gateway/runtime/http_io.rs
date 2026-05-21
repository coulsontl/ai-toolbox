use crate::coding::proxy_gateway::types::{
    GatewayCliKey, GatewayProviderAttempt, ProxyGatewaySettings,
};
use crate::coding::proxy_gateway::usage_parser::{SseUsageCollector, TokenUsage};
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use std::io::Write;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time;

pub(super) const MAX_REQUEST_HEADER_BYTES: usize = 64 * 1024;
pub(super) const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024;

pub(super) type DebugBodyStream =
    Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>;

#[derive(Debug)]
pub(super) struct DebugHttpRequest {
    pub(super) id: u64,
    pub(super) peer_addr: SocketAddr,
    pub(super) method: String,
    pub(super) path: String,
    pub(super) version: String,
    pub(super) first_line: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) raw_len: usize,
}

pub(super) struct DebugHttpResponse {
    pub(super) status_code: u16,
    pub(super) status_text: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) body_stream: Option<DebugBodyStream>,
    pub(super) response_body_bytes: u64,
    pub(super) token_usage: TokenUsage,
    pub(super) first_token_ms: Option<u64>,
    pub(super) is_streaming: bool,
    pub(super) cli_key: Option<GatewayCliKey>,
    pub(super) route_name: String,
    pub(super) provider_id: Option<String>,
    pub(super) provider_name: Option<String>,
    pub(super) provider_type: Option<String>,
    pub(super) cost_multiplier: Option<String>,
    pub(super) pricing_model_source: Option<String>,
    pub(super) requested_model: Option<String>,
    pub(super) upstream_model_id: Option<String>,
    pub(super) upstream_request_body: Option<Vec<u8>>,
    pub(super) upstream_url: Option<String>,
    pub(super) error_category: Option<String>,
    pub(super) attempt_count: u32,
    pub(super) provider_attempt_count: u32,
    pub(super) provider_attempts: Vec<GatewayProviderAttempt>,
    pub(super) failover: bool,
    pub(super) note: String,
}

pub(super) async fn read_http_request(
    stream: &mut TcpStream,
    request_id: u64,
    peer_addr: SocketAddr,
) -> std::io::Result<DebugHttpRequest> {
    let mut raw = Vec::new();
    let mut header_end = None;
    let mut buffer = [0_u8; 8192];

    while header_end.is_none() {
        let read = time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timed out reading gateway request headers",
                )
            })??;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        if raw.len() > MAX_REQUEST_HEADER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Gateway request headers exceed the maximum allowed size",
            ));
        }
        header_end = find_header_end(&raw);
    }

    let header_end = header_end.unwrap_or(raw.len());
    let mut header_text = String::from_utf8_lossy(&raw[..header_end]).to_string();
    while header_text.ends_with('\n') || header_text.ends_with('\r') {
        header_text.pop();
    }

    let mut lines = header_text.lines();
    let first_line = lines.next().unwrap_or_default().trim().to_string();
    let mut first_parts = first_line.split_whitespace();
    let method = first_parts.next().unwrap_or_default().to_string();
    let path = first_parts.next().unwrap_or_default().to_string();
    let version = first_parts.next().unwrap_or_default().to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();

    let content_length = header_value(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Gateway request body exceeds the maximum allowed size",
        ));
    }
    let body_start = header_end.min(raw.len());
    let mut body = raw[body_start..].to_vec();
    if body.len() > MAX_REQUEST_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Gateway request body exceeds the maximum allowed size",
        ));
    }
    while body.len() < content_length {
        let read = time::timeout(Duration::from_secs(30), stream.read(&mut buffer))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timed out reading gateway request body",
                )
            })??;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        body.extend_from_slice(&buffer[..read]);
        if body.len() > MAX_REQUEST_BODY_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Gateway request body exceeds the maximum allowed size",
            ));
        }
    }

    Ok(DebugHttpRequest {
        id: request_id,
        peer_addr,
        method,
        path,
        version,
        first_line,
        headers,
        body,
        raw_len: raw.len(),
    })
}

pub(super) fn json_response(
    status_code: u16,
    status_text: &str,
    value: Value,
    route_name: &str,
    upstream_url: Option<String>,
    note: &str,
) -> DebugHttpResponse {
    let body = serde_json::to_vec(&value)
        .unwrap_or_else(|_| br#"{"error":"response_serialize_failed"}"#.to_vec());
    DebugHttpResponse {
        status_code,
        status_text: status_text.to_string(),
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        response_body_bytes: body.len() as u64,
        body,
        body_stream: None,
        token_usage: TokenUsage::default(),
        first_token_ms: None,
        is_streaming: false,
        cli_key: None,
        route_name: route_name.to_string(),
        provider_id: None,
        provider_name: None,
        provider_type: None,
        cost_multiplier: None,
        pricing_model_source: None,
        requested_model: None,
        upstream_model_id: None,
        upstream_request_body: None,
        upstream_url,
        error_category: None,
        attempt_count: 0,
        provider_attempt_count: 0,
        provider_attempts: Vec::new(),
        failover: false,
        note: note.to_string(),
    }
}

pub(super) fn empty_response(
    status_code: u16,
    status_text: &str,
    route_name: &str,
    note: &str,
) -> DebugHttpResponse {
    DebugHttpResponse {
        status_code,
        status_text: status_text.to_string(),
        headers: Vec::new(),
        response_body_bytes: 0,
        body: Vec::new(),
        body_stream: None,
        token_usage: TokenUsage::default(),
        first_token_ms: None,
        is_streaming: false,
        cli_key: None,
        route_name: route_name.to_string(),
        provider_id: None,
        provider_name: None,
        provider_type: None,
        cost_multiplier: None,
        pricing_model_source: None,
        requested_model: None,
        upstream_model_id: None,
        upstream_request_body: None,
        upstream_url: None,
        error_category: None,
        attempt_count: 0,
        provider_attempt_count: 0,
        provider_attempts: Vec::new(),
        failover: false,
        note: note.to_string(),
    }
}

pub(super) async fn write_response(
    stream: &mut TcpStream,
    response: &mut DebugHttpResponse,
    started_instant: Instant,
    settings: &ProxyGatewaySettings,
) -> std::io::Result<()> {
    let mut header = Vec::new();
    write!(
        &mut header,
        "HTTP/1.1 {} {}\r\n",
        response.status_code, response.status_text
    )?;
    let mut has_content_length = false;
    let mut has_connection = false;
    let streaming = response.body_stream.is_some();
    for (name, value) in &response.headers {
        if streaming
            && (name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("transfer-encoding"))
        {
            continue;
        }
        if name.eq_ignore_ascii_case("content-length") {
            has_content_length = true;
        }
        if name.eq_ignore_ascii_case("connection") {
            has_connection = true;
        }
        write!(&mut header, "{}: {}\r\n", name, value)?;
    }
    if streaming {
        write!(&mut header, "Transfer-Encoding: chunked\r\n")?;
    } else if !has_content_length {
        write!(&mut header, "Content-Length: {}\r\n", response.body.len())?;
    }
    if !has_connection {
        write!(&mut header, "Connection: close\r\n")?;
    }
    write!(&mut header, "\r\n")?;
    stream.write_all(&header).await?;
    if streaming {
        write_streaming_body(stream, response, started_instant, settings).await?;
    } else {
        stream.write_all(&response.body).await?;
    }
    stream.flush().await
}

async fn write_streaming_body(
    stream: &mut TcpStream,
    response: &mut DebugHttpResponse,
    started_instant: Instant,
    settings: &ProxyGatewaySettings,
) -> std::io::Result<()> {
    let mut body_stream = match response.body_stream.take() {
        Some(body_stream) => body_stream,
        None => return Ok(()),
    };
    let mut usage_collector = response.cli_key.map(|_| SseUsageCollector::default());
    response.response_body_bytes = 0;
    response.body.clear();
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

    let mut write_result: std::io::Result<()> = Ok(());
    loop {
        let next_chunk = match time::timeout(idle_timeout, body_stream.next()).await {
            Ok(next_chunk) => next_chunk,
            Err(_) => {
                response.error_category = Some("stream_idle_timeout".to_string());
                response.note = format!(
                    "upstream streaming response was idle for {} seconds",
                    idle_timeout.as_secs()
                );
                write_result = Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    response.note.clone(),
                ));
                break;
            }
        };
        let Some(chunk_result) = next_chunk else {
            break;
        };
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(error) => {
                write_result = Err(std::io::Error::new(std::io::ErrorKind::Other, error));
                break;
            }
        };
        if chunk.is_empty() {
            continue;
        }
        if response.first_token_ms.is_none() {
            response.first_token_ms = Some(
                started_instant
                    .elapsed()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            );
        }
        response.response_body_bytes = response
            .response_body_bytes
            .saturating_add(chunk.len() as u64);
        if let (Some(cli_key), Some(collector)) = (response.cli_key, usage_collector.as_mut()) {
            collector.push_chunk(cli_key, &chunk);
        }
        append_body_snapshot(response, &chunk, settings);
        let chunk_header = format!("{:X}\r\n", chunk.len());
        if let Err(error) = stream.write_all(chunk_header.as_bytes()).await {
            write_result = Err(error);
            break;
        }
        if let Err(error) = stream.write_all(&chunk).await {
            write_result = Err(error);
            break;
        }
        if let Err(error) = stream.write_all(b"\r\n").await {
            write_result = Err(error);
            break;
        }
        if let Err(error) = stream.flush().await {
            write_result = Err(error);
            break;
        }
    }

    if write_result.is_ok() {
        write_result = stream.write_all(b"0\r\n\r\n").await;
    }
    if let (Some(cli_key), Some(collector)) = (response.cli_key, usage_collector) {
        response.token_usage = collector.finish(cli_key);
    }
    write_result
}

fn append_body_snapshot(
    response: &mut DebugHttpResponse,
    chunk: &[u8],
    settings: &ProxyGatewaySettings,
) {
    if !settings.store_response_body {
        return;
    }
    let max_bytes = settings.log_max_body_size_kb.saturating_mul(1024) as usize;
    if max_bytes == 0 || response.body.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes.saturating_sub(response.body.len());
    response
        .body
        .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
}

pub(super) fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            raw.windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })
}

pub(super) fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
