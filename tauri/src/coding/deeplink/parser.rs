//! Pure URL → struct parsing for `aitoolbox://` deep links.
//!
//! No DB access, no `AppState`. The parser is a pure transform; all side
//! effects (event emission, window focus, DB writes) live in `mod.rs` and
//! `provider.rs`.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use super::portable::SharedConnection;
use super::utils::tolerant_base64_decode;

/// The scheme registered in `tauri.conf.json` (`plugins.deep-link.desktop.schemes`).
pub const SCHEME: &str = "aitoolbox";
/// The protocol version (sits in the URL host position, mirroring cc-switch).
pub const VERSION: &str = "v1";
/// The mandatory path.
pub const PATH: &str = "/import";

/// Public app IDs. Keep the original v1 IDs for existing links.
pub const SUPPORTED_APPS: &[&str] = &[
    "claude",
    "claudedesktop",
    "codex",
    "grok",
    "kimi",
    "gemini",
    "opencode",
    "openclaw",
    "pi",
    "omp",
    "hermes",
    "dsh",
];

/// The only resource type supported in v1.
pub const SUPPORTED_RESOURCE: &str = "provider";

/// A parsed deep-link import request. Serialized camelCase and emitted to the
/// frontend. `config`/`extra` carry the **decoded** strings — the raw base64
/// never crosses the IPC boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkImportRequest {
    pub resource: String,
    pub app: String,
    pub name: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    /// Decoded tool-specific JSON/TOML blob that overrides the builder's
    /// `settings_config` (Claude/Gemini) or `config` TOML (Codex).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// Decoded Claude `extra_settings_config` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(flatten)]
    pub connection: SharedConnection,
    /// The original URL (kept so the frontend/dialog can show it if needed;
    /// the backend logs the *redacted* form separately).
    #[serde(rename = "rawUrl")]
    pub raw_url: String,
}

/// A redacted error payload emitted alongside `deep-link-error`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkErrorPayload {
    pub url: String,
    pub error: String,
}

#[derive(Debug, Error)]
pub enum DeepLinkError {
    #[error("unsupported scheme (expected '{expected}')")]
    BadScheme { expected: String },
    #[error("unsupported version (expected '{expected}')")]
    BadVersion { expected: String },
    #[error("unsupported path (expected '{expected}')")]
    BadPath { expected: String },
    #[error("unsupported resource (only 'provider' is supported)")]
    UnsupportedResource,
    #[error("unsupported app '{0}'")]
    UnsupportedApp(String),
    #[error("invalid provider connection data: {0}")]
    InvalidConnection(String),
    #[error("unsupported parameter '{0}'")]
    UnsupportedParam(&'static str),
    #[error("missing required parameter '{0}'")]
    MissingParam(&'static str),
    #[error("invalid URL for '{field}': {detail}")]
    InvalidUrl { field: String, detail: String },
    #[error("invalid base64 in parameter '{0}'")]
    InvalidBase64(&'static str),
    #[error("internal: {0}")]
    Internal(String),
}

/// Normalize a category token the same way `cc_switch::normalize_category` does,
/// re-implemented locally to avoid cross-module coupling.
fn normalize_category(raw: Option<&str>) -> String {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some("official") => "official",
        Some("aggregator") => "third_party",
        Some("omo") => "custom",
        Some("third_party") => "third_party",
        Some("custom") => "custom",
        _ => "custom",
    }
    .to_string()
}

/// Validate that `s` parses as an http/https URL. Returns the original string
/// on success so the caller can store it verbatim.
pub fn validate_url(s: &str, field: &str) -> Result<String, DeepLinkError> {
    let parsed = Url::parse(s).map_err(|e| DeepLinkError::InvalidUrl {
        field: field.to_string(),
        detail: e.to_string(),
    })?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(DeepLinkError::InvalidUrl {
            field: field.to_string(),
            detail: format!("scheme must be http or https, got '{scheme}'"),
        });
    }
    Ok(s.to_string())
}

/// Parse an `aitoolbox://v1/import?...` URL into a `DeepLinkImportRequest`.
pub fn parse_deeplink_url(raw: &str) -> Result<DeepLinkImportRequest, DeepLinkError> {
    let url = Url::parse(raw).map_err(|_| DeepLinkError::BadScheme {
        expected: SCHEME.to_string(),
    })?;

    if url.scheme() != SCHEME {
        return Err(DeepLinkError::BadScheme {
            expected: SCHEME.to_string(),
        });
    }

    // The version sits in the host position (`aitoolbox://v1/import`).
    let host = url.host_str().unwrap_or("");
    if host != VERSION {
        return Err(DeepLinkError::BadVersion {
            expected: VERSION.to_string(),
        });
    }

    if url.path() != PATH {
        return Err(DeepLinkError::BadPath {
            expected: PATH.to_string(),
        });
    }

    let params: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let get = |key: &str| -> Option<String> {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.to_string())
    };

    let resource = get("resource").unwrap_or_default();
    if resource != SUPPORTED_RESOURCE {
        return Err(DeepLinkError::UnsupportedResource);
    }

    let app = get("app").unwrap_or_default();
    if !SUPPORTED_APPS.contains(&app.as_str()) {
        return Err(DeepLinkError::UnsupportedApp(app));
    }

    let name = get("name")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(DeepLinkError::MissingParam("name"))?;

    let category = normalize_category(get("category").as_deref());

    let api_key = get("apiKey").filter(|s| !s.is_empty());
    let model = get("model").filter(|s| !s.is_empty());
    let homepage = match get("homepage") {
        Some(s) if !s.is_empty() => Some(validate_url(&s, "homepage")?),
        _ => None,
    };
    let base_url = match get("baseUrl") {
        Some(s) if !s.is_empty() => Some(validate_url(&s, "baseUrl")?),
        _ => None,
    };

    if get("endpoints").is_some_and(|s| !s.trim().is_empty()) {
        return Err(DeepLinkError::UnsupportedParam("endpoints"));
    }

    let notes = get("notes").filter(|s| !s.is_empty());
    let icon = get("icon").filter(|s| !s.is_empty());
    let icon_color = get("iconColor").filter(|s| !s.is_empty());
    let source_provider_id = get("sourceProviderId").filter(|s| !s.is_empty());

    let config = match get("config") {
        Some(s) if !s.is_empty() => {
            Some(tolerant_base64_decode(&s).map_err(|_| DeepLinkError::InvalidBase64("config"))?)
        }
        _ => None,
    };
    let extra = match get("extra") {
        Some(s) if !s.is_empty() => {
            Some(tolerant_base64_decode(&s).map_err(|_| DeepLinkError::InvalidBase64("extra"))?)
        }
        _ => None,
    };

    let mut connection_fields = serde_json::Map::new();
    for key in [
        "sourceApp",
        "baseUrlStyle",
        "apiFormat",
        "apiVersion",
        "providerType",
        "apiKeyField",
    ] {
        if let Some(value) = get(key).filter(|value| !value.trim().is_empty()) {
            connection_fields.insert(key.to_string(), serde_json::Value::String(value));
        }
    }
    for key in ["models", "modelRoles", "headers", "gatewayProfile"] {
        if let Some(value) = get(key).filter(|value| !value.trim().is_empty()) {
            let value = serde_json::from_str(&value)
                .map_err(|_| DeepLinkError::InvalidConnection(format!("invalid {key}")))?;
            connection_fields.insert(key.to_string(), value);
        }
    }
    let connection = serde_json::from_value(serde_json::Value::Object(connection_fields))
        .map_err(|error| DeepLinkError::InvalidConnection(error.to_string()))?;

    Ok(DeepLinkImportRequest {
        resource,
        app,
        name,
        category,
        api_key,
        base_url,
        model,
        homepage,
        notes,
        icon,
        icon_color,
        source_provider_id,
        config,
        extra,
        connection,
        raw_url: raw.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_bad_scheme() {
        let err = parse_deeplink_url("https://v1/import?resource=provider").unwrap_err();
        assert!(matches!(err, DeepLinkError::BadScheme { .. }));
    }

    #[test]
    fn parse_rejects_bad_version() {
        let err = parse_deeplink_url("aitoolbox://v2/import?resource=provider&app=codex&name=Test")
            .unwrap_err();
        assert!(matches!(err, DeepLinkError::BadVersion { .. }));
    }

    #[test]
    fn parse_rejects_bad_path() {
        let err = parse_deeplink_url("aitoolbox://v1/apply?resource=provider&app=codex&name=Test")
            .unwrap_err();
        assert!(matches!(err, DeepLinkError::BadPath { .. }));
    }

    #[test]
    fn parse_rejects_unknown_resource() {
        let url = "aitoolbox://v1/import?resource=mcp&app=codex&name=Test";
        let err = parse_deeplink_url(url).unwrap_err();
        assert!(matches!(err, DeepLinkError::UnsupportedResource));
    }

    #[test]
    fn parse_rejects_unknown_app() {
        let url = "aitoolbox://v1/import?resource=provider&app=unknown-tool&name=Test";
        let err = parse_deeplink_url(url).unwrap_err();
        assert!(matches!(err, DeepLinkError::UnsupportedApp(_)));
    }

    #[test]
    fn parse_accepts_all_portable_targets_and_extended_connection_fields() {
        for app in SUPPORTED_APPS {
            let mut url = Url::parse("aitoolbox://v1/import").unwrap();
            url.query_pairs_mut().extend_pairs([
                ("resource", "provider"),
                ("app", *app),
                ("sourceApp", "opencode"),
                ("name", "Relay + 中文"),
                ("apiFormat", "anthropic_messages"),
                ("apiKeyField", "x-api-key"),
                (
                    "models",
                    r#"[{"id":"vendor/a+b","contextWindow":64000,"input":["text","image"]}]"#,
                ),
                ("modelRoles", r#"{"sonnet":"vendor/a+b"}"#),
                (
                    "headers",
                    r#"[{"op":"set","name":"X-Project","value":"a+b & c"}]"#,
                ),
            ]);
            let parsed = parse_deeplink_url(url.as_str()).unwrap();
            assert_eq!(parsed.app, *app);
            assert_eq!(parsed.connection.source_app.as_deref(), Some("opencode"));
            assert_eq!(parsed.connection.models[0].id, "vendor/a+b");
            assert_eq!(parsed.connection.headers[0].value, "a+b & c");
        }
    }

    #[test]
    fn parse_rejects_malformed_portable_json() {
        for (field, value) in [
            ("models", "{"),
            ("models", r#"[{"name":"missing id"}]"#),
            ("headers", "false"),
            ("gatewayProfile", "[]"),
        ] {
            let mut url =
                Url::parse("aitoolbox://v1/import?resource=provider&app=pi&name=Relay").unwrap();
            url.query_pairs_mut().append_pair(field, value);
            assert!(parse_deeplink_url(url.as_str()).is_err(), "{field}");
        }
    }

    #[test]
    fn parse_requires_name() {
        let err =
            parse_deeplink_url("aitoolbox://v1/import?resource=provider&app=codex&category=custom")
                .unwrap_err();
        assert!(matches!(err, DeepLinkError::MissingParam("name")));
    }

    #[test]
    fn parse_normalizes_category() {
        let req = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&category=aggregator",
        )
        .unwrap();
        assert_eq!(req.category, "third_party");

        let req = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&category=unknown",
        )
        .unwrap();
        assert_eq!(req.category, "custom");
    }

    #[test]
    fn parse_validates_baseurl_and_rejects_unsupported_endpoints() {
        let err = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&baseUrl=ftp://x",
        )
        .unwrap_err();
        assert!(matches!(err, DeepLinkError::InvalidUrl { .. }));

        let err = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&endpoints=https://a.com",
        )
        .unwrap_err();
        assert!(matches!(err, DeepLinkError::UnsupportedParam("endpoints")));
    }

    #[test]
    fn parse_decodes_config_and_extra() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let cfg = URL_SAFE_NO_PAD.encode(r#"{"env":{"X":"1"}}"#);
        let extra = URL_SAFE_NO_PAD.encode(r#"{"permissions":{}}"#);
        let url = format!(
            "aitoolbox://v1/import?resource=provider&app=claude&name=T&config={cfg}&extra={extra}"
        );
        let req = parse_deeplink_url(&url).unwrap();
        assert_eq!(req.config.as_deref(), Some(r#"{"env":{"X":"1"}}"#));
        assert_eq!(req.extra.as_deref(), Some(r#"{"permissions":{}}"#));
    }

    #[test]
    fn parse_full_provider() {
        let req = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=OpenRouter&category=third_party&apiKey=sk-x&baseUrl=https://openrouter.ai/api/v1&model=gpt-5&homepage=https://openrouter.ai",
        )
        .unwrap();
        assert_eq!(req.app, "codex");
        assert_eq!(req.name, "OpenRouter");
        assert_eq!(req.category, "third_party");
        assert_eq!(req.api_key.as_deref(), Some("sk-x"));
        assert_eq!(
            req.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(req.model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn parse_rejects_invalid_base64_config() {
        let err = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=claude&name=T&config=%21%21%21not-base64%21%21%21",
        )
        .unwrap_err();
        assert!(matches!(err, DeepLinkError::InvalidBase64(_)));
    }

    #[test]
    fn parse_rejects_invalid_base64_extra() {
        let err = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=claude&name=T&extra=%40%40%40garbage%40%40%40",
        )
        .unwrap_err();
        assert!(matches!(err, DeepLinkError::InvalidBase64(_)));
    }

    #[test]
    fn parse_optional_fields_pass_through() {
        let req = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&category=custom&notes=hello&icon=star&iconColor=%23fff&sourceProviderId=ccs%3Acodex%3A42",
        )
        .unwrap();
        assert_eq!(req.notes.as_deref(), Some("hello"));
        assert_eq!(req.icon.as_deref(), Some("star"));
        assert_eq!(req.icon_color.as_deref(), Some("#fff"));
        assert_eq!(req.source_provider_id.as_deref(), Some("ccs:codex:42"));
    }

    #[test]
    fn parse_empty_string_params_are_dropped() {
        // Empty-valued params (e.g. `apiKey=`) must become None, not Some("").
        let req = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&category=custom&apiKey=&baseUrl=&model=",
        )
        .unwrap();
        assert!(req.api_key.is_none());
        assert!(req.base_url.is_none());
        assert!(req.model.is_none());
    }

    #[test]
    fn parse_requires_resource() {
        let err = parse_deeplink_url("aitoolbox://v1/import?app=codex&name=T&category=custom")
            .unwrap_err();
        assert!(matches!(err, DeepLinkError::UnsupportedResource));
    }

    #[test]
    fn parse_requires_app() {
        let err =
            parse_deeplink_url("aitoolbox://v1/import?resource=provider&name=T&category=custom")
                .unwrap_err();
        assert!(matches!(err, DeepLinkError::UnsupportedApp(_)));
    }

    #[test]
    fn parse_homepage_must_be_http() {
        let err = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&homepage=ftp://x.com",
        )
        .unwrap_err();
        assert!(matches!(err, DeepLinkError::InvalidUrl { .. }));
    }

    #[test]
    fn parse_rejects_endpoints_even_with_empty_segments() {
        let err = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=T&endpoints=https://a.com,,https://b.com,",
        )
        .unwrap_err();
        assert!(matches!(err, DeepLinkError::UnsupportedParam("endpoints")));
    }

    #[test]
    fn parse_preserves_raw_url() {
        let raw = "aitoolbox://v1/import?resource=provider&app=codex&name=T&category=custom";
        let req = parse_deeplink_url(raw).unwrap();
        assert_eq!(req.raw_url, raw);
    }
}
