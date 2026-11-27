//! Wire format of the Bun plugin host.
//!
//! One JSON message per line. Requests are `{id, method, params}`; the host
//! answers `{id, result}` or `{id, error}`. Replies that stream a body send
//! `{id, event: "head", status, headers}` first, then `{id, event: "chunk",
//! data}` (base64) frames, and close with `{id, result: null}`. Messages
//! without an `id` are host-initiated events (logs, toasts, auth changes).
//!
//! The limits below are the host's own envelope; a line past them is a
//! protocol violation, not something to buffer.

use serde_json::{json, Value};

/// Largest line we accept from the host. A streaming frame is at most
/// `MAX_FRAME` decoded bytes, so this only guards against a runaway result
/// or a plugin writing raw bytes to the pipe.
pub const MAX_WIRE_LINE: usize = 32 * 1024 * 1024;
/// One base64 chunk decodes to at most this many bytes.
pub const MAX_FRAME: usize = 64 * 1024;
/// A reply head (status + headers) costs at most this many UTF-8 bytes,
/// counting each entry's name, value and four bytes of framing.
pub const HEAD_MAX: usize = 64 * 1024;
pub const HEAD_ENTRIES: usize = 256;
const HEAD_ENTRY_COST: usize = 4;
/// A captured outbound request body is capped at this many decoded bytes; the
/// host's own capture cap matches it.
pub const WIRE_BODY_MAX: usize = 512 * 1024;

pub const STREAM_WINDOW: usize = 512 * 1024;
pub const FRAME_OVERHEAD: usize = 2048;
/// Total bytes one fetch may stream. A response this large is a runaway
/// plugin, not an agent completion.
pub const STREAM_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct FetchHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

/// The outbound HTTP request a provider plugin really sent, as the host
/// captured it on the plugin's behalf (`captureRequest`).
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedWireRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    /// Decoded body bytes, already cut at `maxBodyBytes`.
    pub body: Option<Vec<u8>>,
    /// Original body length before truncation.
    pub body_bytes: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    Log {
        level: String,
        message: String,
    },
    Toast {
        level: String,
        message: String,
    },
    Auth {
        provider: String,
        account: String,
    },
    SignIn {
        provider: String,
        account: String,
        said: String,
    },
    Heavy {
        count: u64,
    },
    Other {
        kind: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostMessage {
    Response {
        id: u64,
        result: Value,
    },
    Failure {
        id: u64,
        message: String,
    },
    Head {
        id: u64,
        head: FetchHead,
    },
    /// A captured outbound request for a fetch that asked for one. It always
    /// arrives before the fetch's `Head`.
    WireRequest {
        id: u64,
        request: CapturedWireRequest,
    },
    Chunk {
        id: u64,
        data: Vec<u8>,
    },
    Event(HostEvent),
    /// A line that is not a message we understand. Logged and skipped; the
    /// host is not killed for it.
    Invalid(String),
}

pub fn encode_request(id: u64, method: &str, params: &Value) -> String {
    let message = json!({ "id": id, "method": method, "params": params });
    let mut line = message.to_string();
    line.push('\n');
    line
}

pub fn encode_abort(id: u64) -> String {
    let message = json!({ "id": id, "method": "abort", "params": { "id": id } });
    let mut line = message.to_string();
    line.push('\n');
    line
}

pub fn encode_credit(id: u64, credited: usize) -> String {
    let message = json!({
        "id": id,
        "method": "credit",
        "params": { "id": id, "n": credited },
    });
    let mut line = message.to_string();
    line.push('\n');
    line
}

/// Decode one line. A line is exactly one JSON object; anything else is
/// [`HostMessage::Invalid`].
pub fn parse_host_line(line: &str) -> HostMessage {
    let line = line.trim();
    if line.is_empty() {
        return HostMessage::Invalid("empty line".to_string());
    }
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => return HostMessage::Invalid(format!("invalid JSON: {error}")),
    };
    let Some(object) = value.as_object() else {
        return HostMessage::Invalid("message is not a JSON object".to_string());
    };

    let id = object.get("id").and_then(Value::as_u64);
    let Some(id) = id else {
        return HostMessage::Event(parse_event(object));
    };

    if let Some(event) = object.get("event").and_then(Value::as_str) {
        return match event {
            "head" => match parse_head(object) {
                Ok(head) => HostMessage::Head { id, head },
                Err(error) => HostMessage::Invalid(error),
            },
            "wire_request" => match parse_wire_request(object) {
                Ok(request) => HostMessage::WireRequest { id, request },
                Err(error) => HostMessage::Invalid(error),
            },
            "chunk" => match object.get("data").and_then(Value::as_str) {
                Some(data) => {
                    match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data) {
                        Ok(bytes) if bytes.len() <= MAX_FRAME => {
                            HostMessage::Chunk { id, data: bytes }
                        }
                        Ok(bytes) => HostMessage::Invalid(format!(
                            "chunk of {} bytes exceeds the {MAX_FRAME}-byte frame",
                            bytes.len()
                        )),
                        Err(error) => HostMessage::Invalid(format!("chunk is not base64: {error}")),
                    }
                }
                None => HostMessage::Invalid("chunk without data".to_string()),
            },
            other => HostMessage::Invalid(format!("unknown event '{other}' for request {id}")),
        };
    }

    if let Some(error) = object.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| error.as_str().map(str::to_string))
            .unwrap_or_else(|| "the plugin host reported an error".to_string());
        return HostMessage::Failure { id, message };
    }

    HostMessage::Response {
        id,
        result: object.get("result").cloned().unwrap_or(Value::Null),
    }
}

fn parse_head(object: &serde_json::Map<String, Value>) -> Result<FetchHead, String> {
    let status = object
        .get("status")
        .and_then(Value::as_u64)
        .ok_or_else(|| "head without a numeric status".to_string())?;
    let status = u16::try_from(status)
        .ok()
        .filter(|status| (100..=599).contains(status))
        .ok_or_else(|| format!("head status {status} is not a valid HTTP status"))?;

    let mut headers = Vec::new();
    let mut cost = 0usize;
    if let Some(map) = object.get("headers").and_then(Value::as_object) {
        for (name, value) in map {
            let value = match value.as_str() {
                Some(value) => value,
                None => continue,
            };
            cost += name.len() + value.len() + HEAD_ENTRY_COST;
            headers.push((name.clone(), value.to_string()));
        }
    }
    if headers.len() > HEAD_ENTRIES || cost > HEAD_MAX {
        return Err(format!(
            "reply head is past the host envelope ({} entries, {cost} bytes; limits are {HEAD_ENTRIES} and {HEAD_MAX})",
            headers.len()
        ));
    }
    Ok(FetchHead { status, headers })
}

fn parse_wire_request(
    object: &serde_json::Map<String, Value>,
) -> Result<CapturedWireRequest, String> {
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "wire_request without a url".to_string())?
        .to_string();

    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|method| !method.is_empty())
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| "GET".to_string());

    let mut headers = Vec::new();
    if let Some(map) = object.get("headers").and_then(Value::as_object) {
        if map.len() > HEAD_ENTRIES {
            return Err(format!(
                "wire_request carries {} headers; the limit is {HEAD_ENTRIES}",
                map.len()
            ));
        }
        for (name, value) in map {
            let Some(value) = value.as_str() else {
                continue;
            };
            headers.push((name.clone(), value.to_string()));
        }
    }

    let body = match object.get("body") {
        None | Some(Value::Null) => None,
        Some(Value::String(data)) => {
            // Reject the obviously oversized before decoding, then check the
            // decoded length too: base64 is 4 bytes per 3 decoded bytes.
            if data.len() > WIRE_BODY_MAX.div_ceil(3) * 4 {
                return Err(format!(
                    "wire_request body is past the {WIRE_BODY_MAX}-byte limit"
                ));
            }
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                .map_err(|error| format!("wire_request body is not base64: {error}"))?;
            if bytes.len() > WIRE_BODY_MAX {
                return Err(format!(
                    "wire_request body of {} bytes is past the {WIRE_BODY_MAX}-byte limit",
                    bytes.len()
                ));
            }
            Some(bytes)
        }
        Some(_) => return Err("wire_request body is neither base64 nor null".to_string()),
    };

    let body_bytes = object
        .get("bodyBytes")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| body.as_ref().map_or(0, |body| body.len() as u64));
    let truncated = object
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(CapturedWireRequest {
        url,
        method,
        headers,
        body,
        body_bytes,
        truncated,
    })
}

fn parse_event(object: &serde_json::Map<String, Value>) -> HostEvent {
    let kind = object
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match kind.as_str() {
        "log" => HostEvent::Log {
            level: text("level"),
            message: text("message"),
        },
        "toast" => HostEvent::Toast {
            level: text("level"),
            message: text("message"),
        },
        "auth" => HostEvent::Auth {
            provider: text("provider"),
            account: text("account"),
        },
        "signIn" => HostEvent::SignIn {
            provider: text("provider"),
            account: text("account"),
            said: text("said"),
        },
        "heavy" => HostEvent::Heavy {
            count: object.get("count").and_then(Value::as_u64).unwrap_or(0),
        },
        _ => HostEvent::Other { kind },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_results_errors_and_events() {
        assert_eq!(
            parse_host_line(r#"{"id":7,"result":{"plugins":[]}}"#),
            HostMessage::Response {
                id: 7,
                result: json!({ "plugins": [] })
            }
        );
        assert_eq!(
            parse_host_line(r#"{"id":8,"error":{"message":"nope"}}"#),
            HostMessage::Failure {
                id: 8,
                message: "nope".to_string()
            }
        );
        assert_eq!(
            parse_host_line(r#"{"event":"toast","level":"info","message":"hi"}"#),
            HostMessage::Event(HostEvent::Toast {
                level: "info".to_string(),
                message: "hi".to_string()
            })
        );
        assert!(matches!(
            parse_host_line("not json"),
            HostMessage::Invalid(_)
        ));
    }

    #[test]
    fn parses_a_streamed_head_and_chunk() {
        let message = parse_host_line(
            r#"{"id":3,"event":"head","status":200,"headers":{"content-type":"text/event-stream"}}"#,
        );
        assert_eq!(
            message,
            HostMessage::Head {
                id: 3,
                head: FetchHead {
                    status: 200,
                    headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
                },
            }
        );

        let chunk = parse_host_line(r#"{"id":3,"event":"chunk","data":"aGVsbG8="}"#);
        assert_eq!(
            chunk,
            HostMessage::Chunk {
                id: 3,
                data: b"hello".to_vec()
            }
        );
    }

    #[test]
    fn parses_a_wire_request_frame() {
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            br#"{"model":"m"}"#,
        );
        let line = json!({
            "id": 4,
            "event": "wire_request",
            "url": "https://api.example.com/v1/messages",
            "method": "post",
            "headers": { "content-type": "application/json" },
            "body": encoded,
            "bodyBytes": 13,
            "truncated": false,
        })
        .to_string();
        assert_eq!(
            parse_host_line(&line),
            HostMessage::WireRequest {
                id: 4,
                request: CapturedWireRequest {
                    url: "https://api.example.com/v1/messages".to_string(),
                    method: "POST".to_string(),
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: Some(br#"{"model":"m"}"#.to_vec()),
                    body_bytes: 13,
                    truncated: false,
                },
            }
        );

        let bodyless = json!({
            "id": 5,
            "event": "wire_request",
            "url": "https://api.example.com/v1/models",
        })
        .to_string();
        assert_eq!(
            parse_host_line(&bodyless),
            HostMessage::WireRequest {
                id: 5,
                request: CapturedWireRequest {
                    url: "https://api.example.com/v1/models".to_string(),
                    method: "GET".to_string(),
                    headers: Vec::new(),
                    body: None,
                    body_bytes: 0,
                    truncated: false,
                },
            }
        );
    }

    #[test]
    fn refuses_a_wire_request_past_its_limits() {
        let oversized = json!({
            "id": 1,
            "event": "wire_request",
            "url": "https://api.example.com/v1/messages",
            "body": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                vec![0u8; WIRE_BODY_MAX + 1],
            ),
        })
        .to_string();
        assert!(matches!(
            parse_host_line(&oversized),
            HostMessage::Invalid(_)
        ));

        let not_base64 = json!({
            "id": 1,
            "event": "wire_request",
            "url": "https://api.example.com/v1/messages",
            "body": "%%% not base64 %%%",
        })
        .to_string();
        assert!(matches!(
            parse_host_line(&not_base64),
            HostMessage::Invalid(_)
        ));

        let headers: serde_json::Map<String, Value> = (0..=HEAD_ENTRIES)
            .map(|index| (format!("x-header-{index}"), Value::String("v".to_string())))
            .collect();
        let too_many = json!({
            "id": 1,
            "event": "wire_request",
            "url": "https://api.example.com/v1/messages",
            "headers": headers,
        })
        .to_string();
        assert!(matches!(
            parse_host_line(&too_many),
            HostMessage::Invalid(_)
        ));

        let no_url = json!({ "id": 1, "event": "wire_request", "method": "POST" }).to_string();
        assert!(matches!(parse_host_line(&no_url), HostMessage::Invalid(_)));
    }

    #[test]
    fn refuses_an_oversized_frame_and_bad_head() {
        let oversized = format!(
            r#"{{"id":1,"event":"chunk","data":"{}"}}"#,
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                vec![0u8; MAX_FRAME + 1]
            )
        );
        assert!(matches!(
            parse_host_line(&oversized),
            HostMessage::Invalid(_)
        ));

        assert!(matches!(
            parse_host_line(r#"{"id":1,"event":"head","status":999,"headers":{}}"#),
            HostMessage::Invalid(_)
        ));
    }

    #[test]
    fn encodes_requests_aborts_and_credits() {
        let request = encode_request(4, "providers", &json!({ "off": true }));
        assert!(request.ends_with('\n'));
        let value: Value = serde_json::from_str(request.trim()).unwrap();
        assert_eq!(value["id"], 4);
        assert_eq!(value["method"], "providers");
        assert_eq!(value["params"]["off"], true);

        let abort: Value = serde_json::from_str(encode_abort(9).trim()).unwrap();
        assert_eq!(abort["method"], "abort");
        assert_eq!(abort["params"]["id"], 9);

        let credit: Value = serde_json::from_str(encode_credit(9, 4096).trim()).unwrap();
        assert_eq!(credit["method"], "credit");
        assert_eq!(credit["params"]["n"], 4096);
    }
}
