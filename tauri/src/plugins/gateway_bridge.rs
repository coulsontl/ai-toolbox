//! Bridge between the Proxy Gateway and the plugin runtimes.
//!
//! The gateway owns protocol conversion, routing and SSE rewriting; provider
//! plugins own the vendor transport. This module is the only place that turns
//! one into the other:
//!
//! - it resolves a plugin provider (its API base URL, its target protocol and
//!   its model list) from the Bun host's provider listing;
//! - it selects and cools down plugin accounts (the host keeps the actual
//!   credentials in `plugin-auth.json`);
//! - it sends one upstream request through the host's `fetch` RPC and hands
//!   the reply back as a plain status/headers/body triple.
//!
//! Everything else stays in `coding::proxy_gateway::runtime`: conversion
//! routes, headers, usage parsing and response shaping are reused unchanged.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value};

use crate::coding::proxy_gateway::transformer::AiProtocol;

use super::bun_host::auth::list_accounts;
use super::bun_host::protocol::CapturedWireRequest;
use super::PluginManagerState;

/// The body of a plugin fetch reply, in the shape the gateway forwards.
///
/// This is the same concrete type as the gateway's private `DebugBodyStream`;
/// keeping a local alias avoids widening that module's visibility.
pub type PluginGatewayBody = Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>;

/// How a gateway provider that references a plugin picks accounts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginAccountStrategy {
    #[default]
    AutoRotate,
    Single,
}

/// A gateway provider record that delegates its transport to a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProviderRef {
    /// The plugin package spec, used for display and to disambiguate two
    /// plugins that happen to register the same provider id.
    pub plugin_spec: String,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_key: Option<String>,
    #[serde(default)]
    pub account_strategy: PluginAccountStrategy,
}

/// Map the npm package a plugin model runs on onto the wire protocol the
/// gateway has to speak to it.
pub fn npm_to_ai_protocol(npm: &str) -> Option<AiProtocol> {
    let npm = npm
        .split([',', ' ', ';'])
        .map(str::trim)
        .find(|part| !part.is_empty())?;
    match npm {
        "@ai-sdk/anthropic" | "@ai-sdk/google-vertex/anthropic" => {
            Some(AiProtocol::AnthropicMessages)
        }
        "@ai-sdk/openai" | "@ai-sdk/azure" => Some(AiProtocol::OpenAiResponses),
        "@ai-sdk/openai-compatible"
        | "@ai-sdk/openrouter"
        | "@ai-sdk/deepseek"
        | "@ai-sdk/xai"
        | "@ai-sdk/github-copilot" => Some(AiProtocol::OpenAiChat),
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => Some(AiProtocol::GeminiNative),
        _ => None,
    }
}

/// One model a plugin provider exposes, as far as the gateway needs it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginModelInfo {
    pub id: String,
    pub name: String,
    pub npm: String,
}

/// Everything the gateway needs to talk to one plugin provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginProviderEndpoint {
    pub provider_id: String,
    pub plugin_spec: String,
    pub provider_name: String,
    /// Base URL the provider's paths are appended to.
    pub base_url: String,
    pub npm: String,
    pub target_protocol: AiProtocol,
    pub models: Vec<PluginModelInfo>,
}

/// Ask the Bun host to capture the outbound HTTP request a provider plugin
/// really sent. Both switches off means no capture at all.
#[derive(Clone, Debug, Default)]
pub struct PluginRequestCapture {
    pub headers: bool,
    pub body: bool,
    /// Decoded body cap in bytes; the host falls back to its own 256 KiB
    /// default when this is zero and never exceeds 256 KiB.
    pub max_body_bytes: usize,
}

/// The request the host captured, re-exported for gateway consumers.
pub type CapturedPluginRequest = CapturedWireRequest;

/// A fetch reply headed for `UpstreamResponse::PluginStream`.
pub struct PluginFetchReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: PluginGatewayBody,
    /// Present when the caller asked for a capture and the plugin's fetch
    /// reached the vendor through the host's `fetch`.
    pub wire_request: Option<CapturedPluginRequest>,
}

const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
const PROVIDER_CACHE_TTL: Duration = Duration::from_secs(60);
/// Upper bound on account switches for one logical request (429 only).
const MAX_ACCOUNT_SWITCHES: usize = 4;

/// In-memory account selection state: a per-provider round-robin cursor and
/// a rate-limit cooldown table. Nothing here is persisted; a restart simply
/// starts the rotation over.
#[derive(Default)]
pub struct PluginAccountRouter {
    cursors: Mutex<HashMap<String, usize>>,
    cooldowns: Mutex<HashMap<String, Instant>>,
}

impl PluginAccountRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pick the account for one attempt.
    ///
    /// `candidates` are the provider's enabled, signed-in account keys in
    /// display order. An empty list means the provider authenticates without
    /// a stored sign-in, so the provider id itself is the host-side key.
    pub fn select(
        &self,
        provider_id: &str,
        strategy: PluginAccountStrategy,
        pinned: Option<&str>,
        candidates: &[String],
    ) -> String {
        if candidates.is_empty() {
            return provider_id.to_string();
        }
        if strategy == PluginAccountStrategy::Single {
            if let Some(pinned) = pinned.filter(|key| candidates.iter().any(|c| c == *key)) {
                return pinned.to_string();
            }
            return candidates[0].clone();
        }

        let now = Instant::now();
        let mut cooldowns = self
            .cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cooldowns.retain(|_, until| *until > now);
        let is_available =
            |account: &String| !cooldowns.contains_key(&cooldown_key(provider_id, account));

        let mut cursor = self
            .cursors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let start = cursor.get(provider_id).copied().unwrap_or(0);
        for offset in 0..candidates.len() {
            let index = (start + offset) % candidates.len();
            if is_available(&candidates[index]) {
                cursor.insert(provider_id.to_string(), index + 1);
                return candidates[index].clone();
            }
        }
        // Every account is cooling down: use the one that frees up first so
        // the request is still attempted rather than failing locally.
        let soonest = candidates
            .iter()
            .min_by_key(|account| {
                cooldowns
                    .get(&cooldown_key(provider_id, account))
                    .copied()
                    .unwrap_or(now)
            })
            .cloned()
            .unwrap_or_else(|| provider_id.to_string());
        soonest
    }

    /// Cool an account down after the upstream answered 429.
    pub fn mark_rate_limited(&self, provider_id: &str, account: &str) {
        let mut cooldowns = self
            .cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cooldowns.insert(
            cooldown_key(provider_id, account),
            Instant::now() + RATE_LIMIT_COOLDOWN,
        );
    }

    /// The accounts of `provider_id` that are currently cooling down.
    pub fn cooling_down(&self, provider_id: &str) -> Vec<String> {
        let now = Instant::now();
        let prefix = format!("{provider_id}\u{1}");
        let mut cooldowns = self
            .cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cooldowns.retain(|_, until| *until > now);
        cooldowns
            .keys()
            .filter_map(|key| key.strip_prefix(&prefix).map(str::to_string))
            .collect()
    }
}

fn cooldown_key(provider_id: &str, account: &str) -> String {
    format!("{provider_id}\u{1}{account}")
}

/// Provider rows fetched from the host plus a short-lived cache.
pub struct PluginGatewayState {
    pub accounts: PluginAccountRouter,
    provider_cache: Mutex<HashMap<String, (Instant, Vec<PluginProviderRow>)>>,
}

impl Default for PluginGatewayState {
    fn default() -> Self {
        Self {
            accounts: PluginAccountRouter::new(),
            provider_cache: Mutex::new(HashMap::new()),
        }
    }
}

impl PluginGatewayState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop cached provider listings (plugin install / enable / option change).
    pub fn clear_provider_cache(&self) {
        self.provider_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    fn cached_rows(&self, key: &str) -> Option<Vec<PluginProviderRow>> {
        let cache = self
            .provider_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache
            .get(key)
            .filter(|(at, _)| at.elapsed() <= PROVIDER_CACHE_TTL)
            .map(|(_, rows)| rows.clone())
    }

    fn store_rows(&self, key: String, rows: Vec<PluginProviderRow>) {
        self.provider_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, (Instant::now(), rows));
    }
}

/// One provider as the host lists it.
#[derive(Clone, Debug, Default)]
pub struct PluginProviderRow {
    pub id: String,
    pub spec: String,
    pub name: String,
    pub api: String,
    pub npm: String,
    pub models: Vec<PluginModelInfo>,
}

/// List every provider the installed plugins expose, cached briefly.
pub async fn list_plugin_provider_rows(
    manager: &PluginManagerState,
    refresh: bool,
) -> Result<Vec<PluginProviderRow>, String> {
    let state = manager.gateway();
    if !refresh {
        if let Some(rows) = state.cached_rows("providers") {
            return Ok(rows);
        }
    }
    let listing = manager.host().providers(json!({})).await?;
    let rows = parse_provider_rows(&listing);
    state.store_rows("providers".to_string(), rows.clone());
    Ok(rows)
}

fn parse_provider_rows(listing: &Value) -> Vec<PluginProviderRow> {
    let Some(entries) = listing.as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let id = json_text(entry.get("id"))?;
            let models = entry
                .get("models")
                .and_then(Value::as_array)
                .map(|models| {
                    models
                        .iter()
                        .filter_map(|model| {
                            let model_id = json_text(model.get("id"))?;
                            Some(PluginModelInfo {
                                id: model_id,
                                name: json_text(model.get("name")).unwrap_or_default(),
                                npm: json_text(model.get("npm")).unwrap_or_default(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(PluginProviderRow {
                id,
                spec: json_text(entry.get("spec")).unwrap_or_default(),
                name: json_text(entry.get("name")).unwrap_or_default(),
                api: json_text(entry.get("api")).unwrap_or_default(),
                npm: json_text(entry.get("npm")).unwrap_or_default(),
                models,
            })
        })
        .collect()
}

fn json_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Resolve one plugin provider from the host listing.
///
/// `requested_model` only chooses the protocol: a model may run on a
/// different npm package than its provider's default.
pub async fn resolve_plugin_provider_endpoint(
    manager: &PluginManagerState,
    plugin_spec: Option<&str>,
    provider_id: &str,
    requested_model: &str,
    fallback_protocol: AiProtocol,
) -> Result<PluginProviderEndpoint, String> {
    let rows = list_plugin_provider_rows(manager, false).await?;
    let row = find_plugin_provider_row(&rows, plugin_spec, provider_id)?;
    endpoint_from_row(row, requested_model, fallback_protocol)
}

/// Find one provider row. A non-empty plugin spec is authoritative: when that
/// plugin does not expose the provider the call fails rather than silently
/// using another plugin's provider with the same id.
pub fn find_plugin_provider_row<'a>(
    rows: &'a [PluginProviderRow],
    plugin_spec: Option<&str>,
    provider_id: &str,
) -> Result<&'a PluginProviderRow, String> {
    let plugin_spec = plugin_spec
        .map(str::trim)
        .filter(|plugin_spec| !plugin_spec.is_empty());
    rows.iter()
        .find(|row| {
            row.id == provider_id && plugin_spec.is_none_or(|plugin_spec| row.spec == plugin_spec)
        })
        .ok_or_else(|| match plugin_spec {
            Some(plugin_spec) => {
                format!("Plugin provider '{provider_id}' is not offered by plugin '{plugin_spec}'")
            }
            None => {
                format!("Plugin provider '{provider_id}' is not offered by any installed plugin")
            }
        })
}

fn endpoint_from_row(
    row: &PluginProviderRow,
    requested_model: &str,
    fallback_protocol: AiProtocol,
) -> Result<PluginProviderEndpoint, String> {
    let base_url = row.api.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err(format!(
            "Plugin provider '{}' does not declare an API base URL",
            row.id
        ));
    }
    let model = row.models.iter().find(|model| model.id == requested_model);
    let npm = model
        .map(|model| model.npm.clone())
        .filter(|npm| !npm.trim().is_empty())
        .unwrap_or_else(|| row.npm.clone());
    let target_protocol = npm_to_ai_protocol(&npm).unwrap_or(fallback_protocol);
    Ok(PluginProviderEndpoint {
        provider_id: row.id.clone(),
        plugin_spec: row.spec.clone(),
        provider_name: row.name.clone(),
        base_url,
        npm,
        target_protocol,
        models: row.models.clone(),
    })
}

/// The enabled account keys of one plugin provider, in display order.
pub fn enabled_accounts(
    manager: &PluginManagerState,
    provider_id: &str,
) -> Result<Vec<String>, String> {
    let mut accounts = list_accounts(manager.db(), manager.paths())?
        .into_iter()
        .filter(|account| account.provider == provider_id && account.enabled)
        .collect::<Vec<_>>();
    accounts.sort_by(|left, right| {
        left.sort_index
            .cmp(&right.sort_index)
            .then_with(|| left.account.cmp(&right.account))
    });
    Ok(accounts
        .into_iter()
        .map(|account| account.account)
        .collect())
}

/// Pick the account one request will start on.
pub fn select_plugin_account(
    manager: &PluginManagerState,
    reference: &PluginProviderRef,
) -> String {
    let candidates = enabled_accounts(manager, &reference.provider_id).unwrap_or_default();
    manager.gateway().accounts.select(
        &reference.provider_id,
        reference.account_strategy,
        reference.account_key.as_deref(),
        &candidates,
    )
}

/// Send one upstream request through the plugin host.
///
/// Credentials never leave the host: the gateway passes no auth header, the
/// host injects the signed-in account's own key. `headers` are the request
/// headers the gateway already decided to forward.
#[allow(clippy::too_many_arguments)]
pub async fn send_via_plugin_host(
    manager: &PluginManagerState,
    reference: &PluginProviderRef,
    npm: &str,
    upstream_model_id: &str,
    url: &str,
    method: &str,
    headers: &HeaderMap,
    body: &[u8],
    proxy: Option<&str>,
    initial_account: &str,
) -> Result<PluginFetchReply, String> {
    send_via_plugin_host_with_capture(
        manager,
        reference,
        npm,
        upstream_model_id,
        url,
        method,
        headers,
        body,
        proxy,
        initial_account,
        None,
    )
    .await
}

/// Send one upstream request and, when `capture` asks for it, also return the
/// request the plugin really made to the vendor.
#[allow(clippy::too_many_arguments)]
pub async fn send_via_plugin_host_with_capture(
    manager: &PluginManagerState,
    reference: &PluginProviderRef,
    npm: &str,
    upstream_model_id: &str,
    url: &str,
    method: &str,
    headers: &HeaderMap,
    body: &[u8],
    proxy: Option<&str>,
    initial_account: &str,
    capture: Option<PluginRequestCapture>,
) -> Result<PluginFetchReply, String> {
    let capture = capture.filter(|capture| capture.headers || capture.body);
    let candidates = enabled_accounts(manager, &reference.provider_id)?;
    let mut account = initial_account.to_string();
    let mut switches_left = if reference.account_strategy == PluginAccountStrategy::AutoRotate {
        candidates.len().clamp(1, MAX_ACCOUNT_SWITCHES)
    } else {
        0
    };
    let mut reloaded = false;

    loop {
        let mut params = json!({
            "provider": reference.provider_id,
            "account": account,
            "model": upstream_model_id,
            "npm": npm,
            "url": url,
            "method": method,
            "headers": header_map_to_json(headers),
            "body": base64::engine::general_purpose::STANDARD.encode(body),
            "proxy": proxy.unwrap_or(""),
        });
        if let Some(object) = params.as_object_mut() {
            if let Some(capture) = capture_request_json(capture.as_ref()) {
                object.insert("captureRequest".to_string(), capture);
            }
        }
        let response = manager.host().fetch(params).await?;

        if response.status == 401
            && reference.account_strategy == PluginAccountStrategy::AutoRotate
            && !reloaded
        {
            // The account was renewed on disk after the host cached it. One
            // restart makes the host re-read `plugin-auth.json`; retry the
            // same account rather than burning the whole rotation.
            reloaded = true;
            drop(response.body);
            manager.host().restart().await?;
            continue;
        }

        if response.status == 429
            && reference.account_strategy == PluginAccountStrategy::AutoRotate
            && switches_left > 0
        {
            manager
                .gateway()
                .accounts
                .mark_rate_limited(&reference.provider_id, &account);
            switches_left -= 1;
            let next = manager.gateway().accounts.select(
                &reference.provider_id,
                reference.account_strategy,
                reference.account_key.as_deref(),
                &candidates,
            );
            if next != account {
                drop(response.body);
                account = next;
                continue;
            }
            // Every account is cooling down; the caller keeps this 429 instead
            // of paying for another request.
        }

        return Ok(PluginFetchReply {
            status: response.status,
            headers: plugin_headers(&response.headers),
            body: Box::pin(
                response
                    .body
                    .map(|chunk| chunk.map(|bytes: Bytes| bytes.to_vec())),
            ),
            wire_request: response.wire_request,
        });
    }
}

/// Request headers forwarded to the host. Inbound placeholder credentials are
/// dropped: the host injects the signed-in account's real key itself, and a
/// stale gateway placeholder must never overwrite it.
fn header_map_to_json(headers: &HeaderMap) -> Value {
    let mut object = JsonMap::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "authorization" | "x-api-key" | "x-goog-api-key"
        ) {
            continue;
        }
        let Ok(text) = value.to_str() else {
            continue;
        };
        object.insert(lower, Value::String(text.to_string()));
    }
    Value::Object(object)
}

/// The `captureRequest` param sent to the host, or `None` when even the
/// capture itself is off (both switches false).
fn capture_request_json(capture: Option<&PluginRequestCapture>) -> Option<Value> {
    let capture = capture.filter(|capture| capture.headers || capture.body)?;
    Some(json!({
        "headers": capture.headers,
        "body": capture.body,
        "maxBodyBytes": capture.max_body_bytes,
    }))
}

/// Host reply headers as a `HeaderMap`, dropping framing headers the gateway
/// rewrites itself. Bun has already decoded any content encoding.
fn plugin_headers(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let lower = name.trim().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "content-length"
                | "content-encoding"
                | "transfer-encoding"
                | "connection"
                | "keep-alive"
                | "proxy-connection"
        ) {
            continue;
        }
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(lower.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        headers.insert(name, value);
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_packages_map_to_wire_protocols() {
        assert_eq!(
            npm_to_ai_protocol("@ai-sdk/anthropic"),
            Some(AiProtocol::AnthropicMessages)
        );
        assert_eq!(
            npm_to_ai_protocol("@ai-sdk/openai"),
            Some(AiProtocol::OpenAiResponses)
        );
        assert_eq!(
            npm_to_ai_protocol("@ai-sdk/openai-compatible"),
            Some(AiProtocol::OpenAiChat)
        );
        assert_eq!(
            npm_to_ai_protocol("@ai-sdk/google-vertex"),
            Some(AiProtocol::GeminiNative)
        );
        assert_eq!(
            npm_to_ai_protocol("@ai-sdk/azure"),
            Some(AiProtocol::OpenAiResponses)
        );
        assert_eq!(
            npm_to_ai_protocol("@ai-sdk/google-vertex/anthropic"),
            Some(AiProtocol::AnthropicMessages)
        );
        assert_eq!(
            npm_to_ai_protocol("@ai-sdk/github-copilot"),
            Some(AiProtocol::OpenAiChat)
        );
        assert_eq!(npm_to_ai_protocol("@ai-sdk/unknown"), None);
    }

    #[test]
    fn a_capture_request_is_only_sent_when_asked_for() {
        assert!(capture_request_json(None).is_none());
        assert!(capture_request_json(Some(&PluginRequestCapture::default())).is_none());

        let value = capture_request_json(Some(&PluginRequestCapture {
            headers: true,
            body: false,
            max_body_bytes: 4096,
        }))
        .expect("capture params");
        assert_eq!(value["headers"], true);
        assert_eq!(value["body"], false);
        assert_eq!(value["maxBodyBytes"], 4096);
    }

    #[test]
    fn inbound_placeholder_credentials_never_reach_the_host() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer gateway"));
        headers.insert("X-Api-Key", HeaderValue::from_static("placeholder"));
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_static("gemini-placeholder"),
        );
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

        let value = header_map_to_json(&headers);
        assert!(value.get("authorization").is_none());
        assert!(value.get("x-api-key").is_none());
        assert!(value.get("x-goog-api-key").is_none());
        assert_eq!(value["anthropic-version"], "2023-06-01");
    }

    #[test]
    fn a_pinned_plugin_spec_never_falls_back_to_another_plugin() {
        let rows = vec![
            PluginProviderRow {
                id: "demo".to_string(),
                spec: "plugin-a".to_string(),
                ..PluginProviderRow::default()
            },
            PluginProviderRow {
                id: "demo".to_string(),
                spec: "plugin-b".to_string(),
                ..PluginProviderRow::default()
            },
        ];
        let exact = find_plugin_provider_row(&rows, Some("plugin-b"), "demo").unwrap();
        assert_eq!(exact.spec, "plugin-b");

        let error = find_plugin_provider_row(&rows, Some("missing"), "demo").unwrap_err();
        assert!(error.contains("missing"), "{error}");

        // An empty spec still means "any plugin".
        let any = find_plugin_provider_row(&rows, Some("  "), "demo").unwrap();
        assert_eq!(any.spec, "plugin-a");
    }

    fn candidates() -> Vec<String> {
        vec![
            "demo".to_string(),
            "demo#2".to_string(),
            "demo#3".to_string(),
        ]
    }

    #[test]
    fn auto_rotate_walks_accounts_in_order() {
        let router = PluginAccountRouter::new();
        let accounts = candidates();
        let first = router.select("demo", PluginAccountStrategy::AutoRotate, None, &accounts);
        let second = router.select("demo", PluginAccountStrategy::AutoRotate, None, &accounts);
        let third = router.select("demo", PluginAccountStrategy::AutoRotate, None, &accounts);
        let fourth = router.select("demo", PluginAccountStrategy::AutoRotate, None, &accounts);
        assert_eq!(first, "demo");
        assert_eq!(second, "demo#2");
        assert_eq!(third, "demo#3");
        assert_eq!(fourth, "demo");
    }

    #[test]
    fn auto_rotate_skips_cooling_accounts() {
        let router = PluginAccountRouter::new();
        let accounts = candidates();
        router.mark_rate_limited("demo", "demo");
        let selected = router.select("demo", PluginAccountStrategy::AutoRotate, None, &accounts);
        assert_eq!(selected, "demo#2");
        assert_eq!(router.cooling_down("demo"), vec!["demo".to_string()]);
    }

    #[test]
    fn auto_rotate_falls_back_to_the_soonest_account() {
        let router = PluginAccountRouter::new();
        let accounts = candidates();
        for account in &accounts {
            router.mark_rate_limited("demo", account);
        }
        let selected = router.select("demo", PluginAccountStrategy::AutoRotate, None, &accounts);
        assert!(accounts.contains(&selected));
    }

    #[test]
    fn single_strategy_pins_or_falls_back_to_the_first_account() {
        let router = PluginAccountRouter::new();
        let accounts = candidates();
        assert_eq!(
            router.select(
                "demo",
                PluginAccountStrategy::Single,
                Some("demo#3"),
                &accounts
            ),
            "demo#3"
        );
        assert_eq!(
            router.select(
                "demo",
                PluginAccountStrategy::Single,
                Some("missing"),
                &accounts
            ),
            "demo"
        );
    }

    #[test]
    fn no_accounts_falls_back_to_the_provider_key() {
        let router = PluginAccountRouter::new();
        assert_eq!(
            router.select("demo", PluginAccountStrategy::AutoRotate, None, &[]),
            "demo"
        );
    }

    #[test]
    fn endpoint_prefers_the_model_npm_over_the_provider_default() {
        let row = PluginProviderRow {
            id: "demo".to_string(),
            spec: "demo-plugin".to_string(),
            name: "Demo".to_string(),
            api: "https://api.example.com/".to_string(),
            npm: "@ai-sdk/openai-compatible".to_string(),
            models: vec![PluginModelInfo {
                id: "claude-x".to_string(),
                name: "Claude X".to_string(),
                npm: "@ai-sdk/anthropic".to_string(),
            }],
        };
        let endpoint =
            endpoint_from_row(&row, "claude-x", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(endpoint.base_url, "https://api.example.com");
        assert_eq!(endpoint.target_protocol, AiProtocol::AnthropicMessages);

        let fallback = endpoint_from_row(&row, "other", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(fallback.target_protocol, AiProtocol::OpenAiChat);
    }
}
