//! HTTP error shape shared by every control-plane route.
//!
//! The CLI maps these error codes to stable exit codes, so the string values
//! are part of the public contract. Keep them kebab/snake free and stable.

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "bad_request", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(401, "unauthorized", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, "forbidden", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "not_found", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, "conflict", message)
    }

    pub fn read_only(message: impl Into<String>) -> Self {
        Self::new(403, "read_only", message)
    }

    pub fn approval_denied() -> Self {
        Self::new(
            403,
            "approval_denied",
            "The desktop user denied this request",
        )
    }

    pub fn approval_timeout() -> Self {
        Self::new(
            408,
            "approval_timeout",
            "The desktop approval request timed out",
        )
    }

    pub fn not_running(message: impl Into<String>) -> Self {
        Self::new(503, "control_api_unavailable", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "internal_error", message)
    }

    pub fn body(&self) -> Value {
        json!({
            "error": {
                "code": self.code,
                // Upstream/parse errors can quote the offending config line, so
                // every error message crosses the same text redaction as reads.
                "message": super::redact::redact_text(&self.message),
                "status": self.status,
            }
        })
    }
}

impl From<String> for ApiError {
    fn from(message: String) -> Self {
        Self::internal(message)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(format!("Failed to serialize response: {error}"))
    }
}

impl From<&str> for ApiError {
    fn from(message: &str) -> Self {
        Self::internal(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_bodies_redact_upstream_credentials() {
        let error = ApiError::bad_request(
            "Upstream rejected api_key = \"sk-live-secret\" (token: tok-123)",
        );
        let body = error.body();
        let message = body["error"]["message"].as_str().unwrap();
        assert!(!message.contains("sk-live-secret"), "message: {message}");
        assert!(!message.contains("tok-123"), "message: {message}");
        assert_eq!(body["error"]["code"], "bad_request");
        assert_eq!(body["error"]["status"], 400);
    }

    #[test]
    fn unsupported_is_its_own_code_not_read_only() {
        let error = ApiError::new(400, "unsupported", "nope");
        assert_eq!(error.code, "unsupported");
        assert_eq!(error.body()["error"]["code"], "unsupported");
    }
}
