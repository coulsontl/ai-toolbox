//! Minimal blocking HTTP/1.1 client for the loopback control API.
//!
//! The API is plain HTTP on 127.0.0.1 and every response is small JSON, so a
//! hand-rolled client keeps `aitb` dependency-light. `Connection: close` makes
//! reading to EOF a complete response read.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Value,
}

#[derive(Debug)]
pub enum TransportError {
    /// The app is not listening, the discovery file is stale, or auth failed.
    Unavailable(String),
    /// The desktop user denied the approval or it timed out.
    Denied(String),
    /// The request reached the app but failed.
    Failed(String),
}

pub struct Client {
    host: String,
    port: u16,
    token: String,
    timeout: Duration,
}

impl Client {
    pub fn new(host: String, port: u16, token: String, timeout_secs: u64) -> Self {
        Self {
            host,
            port,
            token,
            timeout: Duration::from_secs(timeout_secs.max(1)),
        }
    }

    pub fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<HttpResponse, TransportError> {
        let body_text = match body {
            Some(value) => serde_json::to_string(value).map_err(|error| {
                TransportError::Failed(format!("Cannot encode request: {error}"))
            })?,
            None => String::new(),
        };
        let host_header = if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        };
        let request = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            method.as_str(),
            path,
            host_header,
            self.token,
            body_text.len(),
            body_text
        );

        let address = format!("{}:{}", self.host, self.port)
            .to_socket_addrs()
            .map_err(|error| {
                TransportError::Unavailable(format!(
                    "Cannot resolve the control API address: {error}"
                ))
            })?
            .next()
            .ok_or_else(|| {
                TransportError::Unavailable("The control API address did not resolve".to_string())
            })?;

        let mut stream = TcpStream::connect_timeout(&address, self.timeout).map_err(|error| {
            TransportError::Unavailable(format!(
                "AI Toolbox is not reachable on {}:{} ({error}). Is the desktop app running and Local Control enabled?",
                self.host, self.port
            ))
        })?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|_| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|error| {
                TransportError::Unavailable(format!("Cannot configure socket: {error}"))
            })?;
        stream.write_all(request.as_bytes()).map_err(|error| {
            TransportError::Unavailable(format!("Cannot send request: {error}"))
        })?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(|error| {
            TransportError::Unavailable(format!("Cannot read response: {error}"))
        })?;
        let response = parse_response(&raw)?;
        classify(response)
    }
}

fn parse_response(raw: &[u8]) -> Result<HttpResponse, TransportError> {
    let split = find_header_end(raw).ok_or_else(|| {
        TransportError::Unavailable("The control API returned an incomplete response".to_string())
    })?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| {
        TransportError::Unavailable("The control API returned a non-UTF-8 header".to_string())
    })?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| TransportError::Unavailable("Missing HTTP status line".to_string()))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| {
            TransportError::Unavailable(format!("Malformed HTTP status line: {status_line}"))
        })?;

    let mut content_length: Option<usize> = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
    }

    let body_bytes = &raw[split + 4..];
    let body_bytes = match content_length {
        Some(length) if length <= body_bytes.len() => &body_bytes[..length],
        _ => body_bytes,
    };
    let text = String::from_utf8_lossy(body_bytes);
    let body = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text.to_string()))
    };
    Ok(HttpResponse { status, body })
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

fn classify(response: HttpResponse) -> Result<HttpResponse, TransportError> {
    if (200..300).contains(&response.status) {
        return Ok(response);
    }
    let (code, message) = match serde_json::from_value::<ErrorEnvelope>(response.body.clone()) {
        Ok(envelope) => (envelope.error.code, envelope.error.message),
        Err(_) => (
            format!("http_{}", response.status),
            response
                .body
                .as_str()
                .unwrap_or("The control API returned an error")
                .to_string(),
        ),
    };
    let text = format!("{message} ({code})");
    match response.status {
        401 => Err(TransportError::Unavailable(text)),
        403 => Err(TransportError::Denied(text)),
        408 if code == "approval_timeout" => Err(TransportError::Denied(text)),
        _ => Err(TransportError::Failed(text)),
    }
}

/// Percent-encodes a single path segment (RFC 3986 unreserved set).
pub fn encode_segment(segment: &str) -> String {
    let mut output = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char)
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

pub fn encode_query(value: &str) -> String {
    encode_segment(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn spawn_server(response: &'static str) -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (port, handle)
    }

    #[test]
    fn sends_auth_and_json_and_parses_the_body() {
        let body = "{\"ok\":true}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let response: &'static str = Box::leak(response.into_boxed_str());
        let (port, handle) = spawn_server(response);
        let client = Client::new("127.0.0.1".to_string(), port, "secret-token".to_string(), 5);
        let result = client
            .request(
                Method::Post,
                "/v1/plugins/reload",
                Some(&serde_json::json!({"id": "x"})),
            )
            .unwrap();
        assert_eq!(result.status, 200);
        assert_eq!(result.body["ok"], true);

        let request = handle.join().unwrap();
        assert!(request.starts_with("POST /v1/plugins/reload HTTP/1.1"));
        assert!(request.contains("Authorization: Bearer secret-token"));
        assert!(request.contains("\"id\":\"x\""));
    }

    #[test]
    fn maps_denied_and_unavailable_statuses() {
        let denied = HttpResponse {
            status: 403,
            body: serde_json::json!({"error": {"code": "approval_denied", "message": "no"}}),
        };
        assert!(matches!(classify(denied), Err(TransportError::Denied(_))));

        let timeout = HttpResponse {
            status: 408,
            body: serde_json::json!({"error": {"code": "approval_timeout", "message": "late"}}),
        };
        assert!(matches!(classify(timeout), Err(TransportError::Denied(_))));

        let unauthorized = HttpResponse {
            status: 401,
            body: serde_json::json!({"error": {"code": "unauthorized", "message": "bad token"}}),
        };
        assert!(matches!(
            classify(unauthorized),
            Err(TransportError::Unavailable(_))
        ));

        let failure = HttpResponse {
            status: 500,
            body: serde_json::json!({"error": {"code": "internal_error", "message": "boom"}}),
        };
        assert!(matches!(classify(failure), Err(TransportError::Failed(_))));
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
        assert_eq!(encode_segment("channel-123"), "channel-123");
    }
}
