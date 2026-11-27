//! Loopback HTTP/1.1 server built on the hyper/tokio stack already in the
//! dependency tree. No web framework is introduced.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tauri::Manager;
use tokio::net::TcpListener;

use super::discovery::{self, DiscoveryFile};
use super::error::{ApiError, ApiResult};
use super::{routes, ControlApiState, ServerContext};

const MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;

pub async fn start(state: &ControlApiState) -> Result<(), String> {
    let token = generate_token()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("Failed to bind the control API listener: {error}"))?;
    let addr = listener
        .local_addr()
        .map_err(|error| format!("Failed to read the control API listener address: {error}"))?;
    state
        .context
        .port
        .store(addr.port(), std::sync::atomic::Ordering::Relaxed);

    let discovery_path = discovery::default_discovery_path();
    let info = DiscoveryFile {
        version: 1,
        pid: std::process::id(),
        url: format!("http://127.0.0.1:{}", addr.port()),
        token: token.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: crate::app_paths::resolved_data_dir()
            .to_string_lossy()
            .to_string(),
        started_at: state.context.started_at.clone(),
    };
    if let Err(error) = discovery::write_discovery_file(&discovery_path, &info) {
        state
            .context
            .port
            .store(0, std::sync::atomic::Ordering::Relaxed);
        return Err(error);
    }

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let context = state.context.clone();
    let server_token = token.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let context = context.clone();
                            let token = server_token.clone();
                            tokio::spawn(async move {
                                let service = service_fn(move |request| {
                                    handle_request(context.clone(), token.clone(), request)
                                });
                                let connection = hyper::server::conn::http1::Builder::new()
                                    .serve_connection(TokioIo::new(stream), service);
                                if let Err(error) = connection.await {
                                    log::debug!("control API connection closed: {error}");
                                }
                            });
                        }
                        Err(error) => {
                            log::warn!("control API accept failed: {error}");
                        }
                    }
                }
            }
        }
        log::debug!("control API listener stopped");
    });

    state.store_runtime(token, shutdown_tx, task);
    log::info!("Control API listening on http://127.0.0.1:{}", addr.port());
    Ok(())
}

pub async fn stop(state: &ControlApiState) -> Result<(), String> {
    let path = discovery::default_discovery_path();
    discovery::remove_discovery_file_if_owned(&path, std::process::id());
    if let Some((shutdown, task)) = state.take_runtime() {
        drop(shutdown);
        if let Some(task) = task {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
        }
    }
    state
        .context
        .port
        .store(0, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

fn generate_token() -> Result<String, String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Failed to generate the control API token".to_string())?;
    Ok(hex::encode(bytes))
}

#[allow(deprecated)]
fn token_matches(expected: &str, provided: &str) -> bool {
    ring::constant_time::verify_slices_are_equal(expected.as_bytes(), provided.as_bytes()).is_ok()
}

async fn handle_request(
    context: Arc<ServerContext>,
    token: String,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let response = match route_request(context, token, request).await {
        Ok(response) => response,
        Err(error) => json_response(error.status, error.body()),
    };
    Ok(response)
}

async fn route_request(
    context: Arc<ServerContext>,
    token: String,
    request: Request<Incoming>,
) -> ApiResult<Response<Full<Bytes>>> {
    if request.headers().contains_key(ORIGIN) {
        return Err(ApiError::forbidden(
            "Browser-originated requests are not accepted by the control API",
        ));
    }
    validate_host(&request, context.port())?;

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let segments = split_path(&path)?;
    let query = parse_query(request.uri().query());

    if method == Method::GET && segments == ["v1", "ping"] {
        return Ok(json_response(
            StatusCode::OK.as_u16(),
            json!({
                "ok": true,
                "app": "ai-toolbox",
                "version": env!("CARGO_PKG_VERSION"),
            }),
        ));
    }

    let provided_token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or_default();
    if provided_token.is_empty() || !token_matches(&token, provided_token) {
        return Err(ApiError::unauthorized(
            "Missing or invalid local control token; run `aitb` so it can read the discovery file",
        ));
    }

    let body = read_body(request).await?;
    let data = routes::dispatch(&context, &method, &segments, &query, body).await?;
    Ok(json_response(StatusCode::OK.as_u16(), data))
}

fn validate_host(request: &Request<Incoming>, port: u16) -> ApiResult<()> {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let expected_ip = format!("127.0.0.1:{port}");
    let expected_localhost = format!("localhost:{port}");
    if host == expected_ip || host == expected_localhost {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "Unexpected Host header for the local control API",
        ))
    }
}

fn split_path(path: &str) -> ApiResult<Vec<String>> {
    if !path.starts_with("/v1/") && path != "/v1" {
        return Err(ApiError::not_found(format!(
            "Unknown control API path '{path}'"
        )));
    }
    let mut segments = Vec::new();
    for raw in path.trim_start_matches('/').split('/') {
        if raw.is_empty() {
            continue;
        }
        segments.push(percent_decode(raw)?);
    }
    Ok(segments)
}

fn parse_query(raw: Option<&str>) -> std::collections::HashMap<String, String> {
    let mut params = std::collections::HashMap::new();
    let Some(raw) = raw else {
        return params;
    };
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(&key.replace('+', " ")).unwrap_or_else(|_| key.to_string());
        let value = percent_decode(&value.replace('+', " ")).unwrap_or_else(|_| value.to_string());
        params.insert(key, value);
    }
    params
}

fn percent_decode(raw: &str) -> ApiResult<String> {
    let bytes = raw.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(ApiError::bad_request("Malformed percent-encoded path"));
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .map_err(|_| ApiError::bad_request("Malformed percent-encoded path"))?;
            let value = u8::from_str_radix(hex, 16)
                .map_err(|_| ApiError::bad_request("Malformed percent-encoded path"))?;
            output.push(value);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| ApiError::bad_request("Path is not valid UTF-8"))
}

async fn read_body(request: Request<Incoming>) -> ApiResult<Value> {
    let declared = request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if declared > MAX_REQUEST_BODY_BYTES {
        return Err(ApiError::new(
            413,
            "payload_too_large",
            format!("Request body exceeds {MAX_REQUEST_BODY_BYTES} bytes"),
        ));
    }
    let bytes = request
        .into_body()
        .collect()
        .await
        .map_err(|error| ApiError::bad_request(format!("Failed to read request body: {error}")))?
        .to_bytes();
    if bytes.len() > MAX_REQUEST_BODY_BYTES {
        return Err(ApiError::new(
            413,
            "payload_too_large",
            format!("Request body exceeds {MAX_REQUEST_BODY_BYTES} bytes"),
        ));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| ApiError::bad_request(format!("Request body must be JSON: {error}")))
}

fn json_response(status: u16, mut body: Value) -> Response<Full<Bytes>> {
    if status >= 400 {
        // Every error crosses the response boundary, including messages raised
        // by code that forgot `ApiError::body`. Redact the whole `error`
        // payload so an upstream parser cannot echo a credential.
        if let Some(error) = body.get_mut("error") {
            redact_strings(error);
        }
    }
    let text = serde_json::to_vec(&body).unwrap_or_else(|_| {
        b"{\"error\":{\"code\":\"internal_error\",\"message\":\"serialization failed\"}}".to_vec()
    });
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(Full::new(Bytes::from(text)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}"))))
}

fn redact_strings(value: &mut Value) {
    match value {
        Value::String(text) => *text = super::redact::redact_text(text),
        Value::Array(items) => {
            for item in items {
                redact_strings(item);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                redact_strings(item);
            }
        }
        _ => {}
    }
}

/// Small helper used by command handlers that need the live context.
pub fn context_for(app: &tauri::AppHandle) -> Option<std::sync::Arc<ServerContext>> {
    app.try_state::<ControlApiState>()
        .map(|state| state.context.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_payloads_redact_strings_before_serialization() {
        let mut body = serde_json::json!({
            "error": {
                "code": "bad_request",
                "message": "failed with api_key = \"sk-live\"",
                "details": ["token: tok-1"],
            }
        });
        redact_strings(body.get_mut("error").unwrap());
        let text = body.to_string();
        assert!(!text.contains("sk-live"), "body: {text}");
        assert!(!text.contains("tok-1"), "body: {text}");
        assert!(text.contains("bad_request"));
    }

    #[test]
    fn token_comparison_requires_the_exact_value() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("abc", ""));
    }

    #[test]
    fn path_segments_are_percent_decoded() {
        assert_eq!(
            split_path("/v1/clis/claude%20code/providers").unwrap(),
            vec!["v1", "clis", "claude code", "providers"]
        );
        assert!(split_path("/v1/clis/%zz").is_err());
        assert!(split_path("/health").is_err());
    }
}
