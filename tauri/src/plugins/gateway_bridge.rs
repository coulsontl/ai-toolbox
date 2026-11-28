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
use std::time::{Duration, Instant, SystemTime};

use base64::Engine as _;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value};

use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::db::SqliteDbState;

use super::bun_host::auth::list_accounts;
use super::bun_host::protocol::CapturedWireRequest;
use super::store;
use super::types::PluginAccountMetaRecord;
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
    /// The model's own API base URL, when it differs from the provider's.
    pub url: String,
    /// The vendor's own id for the model, when it differs from `id`.
    pub api_id: String,
    /// Other AI SDK packages the vendor serves this model on, besides `npm`.
    pub also: Vec<String>,
    /// Every kind of output the model declared (`text`, `image`, ...).
    pub outputs: Vec<String>,
    /// Whether the plugin actually said what the model outputs. A model that
    /// said nothing is treated as chat-capable.
    pub outputs_said: bool,
}

/// Whether a model can serve a chat CLI. Models that never declared their
/// output kinds keep the old behaviour; a model that declared them must list
/// `text` somewhere.
pub fn model_supports_chat(model: &PluginModelInfo) -> bool {
    !model.outputs_said
        || model
            .outputs
            .iter()
            .any(|output| output.eq_ignore_ascii_case("text"))
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
    /// The vendor's own id for the requested model, when it differs from the
    /// public id the CLI asked for. Upstream requests must carry this value.
    pub upstream_model_id: Option<String>,
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
/// Bounds `Retry-After` so a broken header can neither thrash nor park an
/// account for longer than an hour.
const RETRY_AFTER_MIN: Duration = Duration::from_secs(5);
const RETRY_AFTER_MAX: Duration = Duration::from_secs(3600);
/// How long an account whose sign-in the host marked `expired` is skipped.
const EXPIRED_SIGN_IN_COOLDOWN: Duration = Duration::from_secs(300);
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
        self.mark_rate_limited_for(provider_id, account, RATE_LIMIT_COOLDOWN);
    }

    /// Cool an account down for an explicit duration (a `Retry-After`, or a
    /// sign-in the host marked expired).
    pub fn mark_rate_limited_for(&self, provider_id: &str, account: &str, duration: Duration) {
        let mut cooldowns = self
            .cooldowns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cooldowns.insert(
            cooldown_key(provider_id, account),
            Instant::now() + duration,
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

/// Whether an account's cached quota snapshot says it has nothing left to
/// spend. Deliberately conservative: only an explicit full window (or a
/// non-positive balance) counts, so a plugin's partial or unfamiliar shape
/// never sidelines a working account.
pub fn is_quota_exhausted(quota: &Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(object) = quota.as_object() else {
        return false;
    };
    if let Some(windows) = object
        .get("windows")
        .and_then(Value::as_array)
        .filter(|windows| !windows.is_empty())
    {
        return windows
            .iter()
            .any(|window| quota_window_exhausted(window, now));
    }
    let Some(balance) = object.get("balance") else {
        return false;
    };
    if let Some(balance) = balance.as_f64() {
        return balance <= 0.0;
    }
    let Some(balance) = balance.as_str() else {
        return false;
    };
    matches!(balance.trim(), "0" | "$0" | "$0.00")
}

/// One usage window. `used` is a percentage (0–100) of the window, exactly as
/// `host.js` documents it; `limit` is the window's own amount in its unit and
/// is not `used`'s denominator. A window marked `aside` never sidelines the
/// account, and one whose reset moment already passed (or carries an
/// unreadable one) is not evidence.
fn quota_window_exhausted(window: &Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(window) = window.as_object() else {
        return false;
    };
    if window.get("aside").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if let Some(resets_at) = window.get("resetsAt").filter(|value| !value.is_null()) {
        let Some(resets_at) = resets_at
            .as_str()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|resets_at| resets_at.with_timezone(&chrono::Utc))
        else {
            return false;
        };
        if resets_at <= now {
            return false;
        }
    }
    let Some(used) = window.get("used").and_then(Value::as_f64) else {
        return false;
    };
    used >= 100.0
}

/// Split candidate account keys into the ones whose cached quota is not
/// exhausted. When every candidate looks exhausted the full list is returned:
/// a stale snapshot must never leave the router without an account to try.
pub fn preferred_account_candidates(
    db: &SqliteDbState,
    provider_id: &str,
    candidates: &[String],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<String> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let metas = store::list_account_metas(db).unwrap_or_default();
    let healthy = candidates
        .iter()
        .filter(|account| !account_quota_exhausted(&metas, provider_id, account, now))
        .cloned()
        .collect::<Vec<_>>();
    if healthy.is_empty() {
        candidates.to_vec()
    } else {
        healthy
    }
}

fn account_quota_exhausted(
    metas: &[PluginAccountMetaRecord],
    provider_id: &str,
    account: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    metas.iter().any(|meta| {
        meta.id == account
            && (meta.provider_id.trim().is_empty() || meta.provider_id == provider_id)
            && is_quota_exhausted(&meta.quota, now)
    })
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

    /// The last provider listing, without asking the host. Background
    /// maintenance reads `renewLead` from here so it never starts Bun just to
    /// learn a renewal window.
    pub fn cached_provider_rows(&self) -> Vec<PluginProviderRow> {
        self.cached_rows("providers").unwrap_or_default()
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
    /// Account keys in the host's display order.
    pub accounts: Vec<String>,
    /// Per-account model ids, when the plugin lists each account separately.
    pub account_models: HashMap<String, Vec<String>>,
    /// The channel only ever uses one account.
    pub single: bool,
    /// The plugin's sign-ins are renewed in the background.
    pub renews: bool,
    /// The lead window the plugin asks for before expiry.
    pub renew_lead_ms: Option<i64>,
    /// The vendor spends a refresh token once, so never copy the sign-in.
    pub rotating: bool,
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
    let params = provider_listing_params(manager).await?;
    let listing = manager.host().providers(params).await?;
    let rows = parse_provider_rows(&listing);
    state.store_rows("providers".to_string(), rows.clone());
    Ok(rows)
}

/// The params the host's `providers` RPC takes. `proxies` is keyed by
/// provider id and then by account key (`""` covers every account), so one
/// entry per declared provider is enough.
pub async fn provider_listing_params(manager: &PluginManagerState) -> Result<Value, String> {
    let proxy = manager.proxy_url().await?;
    let mut proxies = JsonMap::new();
    if !proxy.trim().is_empty() {
        for record in super::store::list_installed_plugins(manager.db())? {
            for provider in record.providers {
                proxies
                    .entry(provider)
                    .or_insert_with(|| json!({ "": proxy }));
            }
        }
        // A legacy package may not declare `magpie.providers`; the previous
        // listing still knows the ids it exposed.
        for row in manager.gateway().cached_provider_rows() {
            proxies
                .entry(row.id)
                .or_insert_with(|| json!({ "": proxy }));
        }
    }
    if proxies.is_empty() {
        Ok(json!({}))
    } else {
        Ok(json!({ "proxies": proxies }))
    }
}

/// The proxy a plugin host call should use: an explicit caller override wins,
/// otherwise the app proxy the plugin settings follow (empty = direct).
pub async fn effective_proxy(
    manager: &PluginManagerState,
    explicit: Option<String>,
) -> Result<String, String> {
    match explicit
        .map(|proxy| proxy.trim().to_string())
        .filter(|proxy| !proxy.is_empty())
    {
        Some(proxy) => Ok(proxy),
        None => manager.proxy_url().await,
    }
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
                                url: json_text(model.get("url")).unwrap_or_default(),
                                api_id: json_text(model.get("apiId")).unwrap_or_default(),
                                also: json_string_array(model.get("also")),
                                outputs: json_string_array(model.get("outputs")),
                                outputs_said: model
                                    .get("outputsSaid")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let accounts = entry
                .get("accounts")
                .and_then(Value::as_array)
                .map(|accounts| {
                    accounts
                        .iter()
                        .filter_map(|account| json_text(account.get("key")))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut account_models = HashMap::new();
            if let Some(accounts) = entry.get("accounts").and_then(Value::as_array) {
                for account in accounts {
                    let Some(key) = json_text(account.get("key")) else {
                        continue;
                    };
                    let per_account = json_string_array(account.get("models"));
                    if !per_account.is_empty() {
                        account_models.insert(key, per_account);
                    }
                }
            }
            Some(PluginProviderRow {
                id,
                spec: json_text(entry.get("spec")).unwrap_or_default(),
                name: json_text(entry.get("name")).unwrap_or_default(),
                api: json_text(entry.get("api")).unwrap_or_default(),
                npm: json_text(entry.get("npm")).unwrap_or_default(),
                models,
                accounts,
                account_models,
                single: entry
                    .get("single")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                renews: entry
                    .get("renews")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                renew_lead_ms: entry
                    .get("renewLead")
                    .and_then(|lead| {
                        lead.as_i64()
                            .or_else(|| lead.as_f64().map(|lead| lead.round() as i64))
                    })
                    .filter(|lead| *lead >= 0),
                rotating: entry
                    .get("rotating")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn json_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn json_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect()
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
    let model = row.models.iter().find(|model| model.id == requested_model);
    // A model may be served on a different API base than its provider, and
    // the host reports that as `model.url`.
    let base_url = model
        .map(|model| model.url.trim())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| row.api.trim())
        .trim_end_matches('/')
        .to_string();
    if base_url.is_empty() {
        return Err(format!(
            "Plugin provider '{}' does not declare an API base URL",
            row.id
        ));
    }
    let npm = model
        .map(|model| model.npm.clone())
        .filter(|npm| !npm.trim().is_empty())
        .unwrap_or_else(|| row.npm.clone());
    let target_protocol = npm_to_ai_protocol(&npm).unwrap_or(fallback_protocol);
    // A model may be public under one id while the vendor serves it under
    // another (`api.id`); the gateway must send the vendor's id upstream.
    let upstream_model_id = model.and_then(|model| {
        let api_id = model.api_id.trim();
        (!api_id.is_empty() && api_id != model.id).then(|| api_id.to_string())
    });
    Ok(PluginProviderEndpoint {
        provider_id: row.id.clone(),
        plugin_spec: row.spec.clone(),
        provider_name: row.name.clone(),
        base_url,
        npm,
        target_protocol,
        upstream_model_id,
        models: row.models.clone(),
    })
}

/// The models a coding CLI may be offered: chat-capable ones only, and for a
/// single-account channel narrowed to that account's own list when the
/// plugin lists models per account.
pub fn models_for_cli(
    row: &PluginProviderRow,
    strategy: PluginAccountStrategy,
    pinned_account: Option<&str>,
) -> Vec<PluginModelInfo> {
    let mut models = row
        .models
        .iter()
        .filter(|model| model_supports_chat(model))
        .cloned()
        .collect::<Vec<_>>();
    if strategy == PluginAccountStrategy::Single {
        if let Some(allowed) = account_model_ids(row, pinned_account) {
            models.retain(|model| allowed.iter().any(|id| id == &model.id));
        }
    }
    models
}

/// The model ids the single selected account serves, when the plugin lists
/// them. `pinned_account` wins; otherwise the first listed account that has
/// its own list is used, mirroring the host's single-account order.
fn account_model_ids<'a>(
    row: &'a PluginProviderRow,
    pinned_account: Option<&str>,
) -> Option<&'a Vec<String>> {
    if let Some(pinned) = pinned_account {
        // The pinned account is the one that will serve requests: when the
        // plugin gives it no list of its own, the provider's union is right.
        return row.account_models.get(pinned);
    }
    row.accounts
        .iter()
        .find_map(|key| row.account_models.get(key))
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
    let selectable = preferred_account_candidates(
        manager.db(),
        &reference.provider_id,
        &candidates,
        chrono::Utc::now(),
    );
    manager.gateway().accounts.select(
        &reference.provider_id,
        reference.account_strategy,
        reference.account_key.as_deref(),
        &selectable,
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
    // Rotation prefers accounts whose cached quota is not used up; a fresh
    // snapshot is not required — the persisted one is the conservative guess
    // until the next `usage` call replaces it.
    let selectable = preferred_account_candidates(
        manager.db(),
        &reference.provider_id,
        &candidates,
        chrono::Utc::now(),
    );
    let mut account = initial_account.to_string();
    let mut switches_left = if reference.account_strategy == PluginAccountStrategy::AutoRotate {
        candidates.len().clamp(1, MAX_ACCOUNT_SWITCHES)
    } else {
        0
    };
    let mut reloaded = false;
    let session = session_from_headers(headers);

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
            if let Some(session) = session {
                object.insert("session".to_string(), Value::String(session.to_string()));
            }
        }
        let response = manager.host().fetch(params).await?;

        if response.status == 401 {
            let sign_in = header_value(&response.headers, "x-magpie-sign-in");
            if sign_in.is_some_and(|said| said.eq_ignore_ascii_case("expired")) {
                // The host already judged this account's sign-in dead. Do not
                // retry it: park it briefly and, in rotation mode, move on to
                // another account.
                manager.gateway().accounts.mark_rate_limited_for(
                    &reference.provider_id,
                    &account,
                    EXPIRED_SIGN_IN_COOLDOWN,
                );
                if reference.account_strategy == PluginAccountStrategy::AutoRotate
                    && switches_left > 0
                {
                    switches_left -= 1;
                    let next = manager.gateway().accounts.select(
                        &reference.provider_id,
                        reference.account_strategy,
                        reference.account_key.as_deref(),
                        &selectable,
                    );
                    if next != account {
                        drop(response.body);
                        account = next;
                        continue;
                    }
                }
            } else if !reloaded {
                // The account was renewed on disk after the host cached it.
                // Clearing this account's loader/auth cache makes the host
                // re-read `plugin-auth.json` on the retry; the whole Bun
                // process must keep running.
                reloaded = true;
                drop(response.body);
                if let Err(error) = manager
                    .host()
                    .reload_account(&reference.provider_id, Some(&account))
                    .await
                {
                    log::debug!("reloading plugin account '{account}' after a 401 failed: {error}");
                }
                continue;
            }
        }

        if response.status == 429
            && reference.account_strategy == PluginAccountStrategy::AutoRotate
            && switches_left > 0
        {
            let cooldown = parse_retry_after(&response.headers, SystemTime::now())
                .unwrap_or(RATE_LIMIT_COOLDOWN);
            manager.gateway().accounts.mark_rate_limited_for(
                &reference.provider_id,
                &account,
                cooldown,
            );
            switches_left -= 1;
            let next = manager.gateway().accounts.select(
                &reference.provider_id,
                reference.account_strategy,
                reference.account_key.as_deref(),
                &selectable,
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

/// The `Retry-After` of a plugin reply, as a cooldown. Accepts decimal
/// seconds and an HTTP-date, and clamps to 5s..=1h.
pub fn parse_retry_after(headers: &[(String, String)], now: SystemTime) -> Option<Duration> {
    let raw = header_value(headers, "retry-after")?;
    let duration = if let Ok(seconds) = raw.parse::<u64>() {
        Duration::from_secs(seconds)
    } else {
        let at = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
        let now = chrono::DateTime::<chrono::Utc>::from(now);
        Duration::from_secs(at.signed_duration_since(now).num_seconds().max(0) as u64)
    };
    Some(duration.clamp(RETRY_AFTER_MIN, RETRY_AFTER_MAX))
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// A session identifier the host may hand to `chat.headers` hooks. Only
/// well-known session headers are read; nothing else from the request is
/// forwarded as a session.
fn session_from_headers(headers: &HeaderMap) -> Option<&str> {
    const SESSION_HEADERS: [&str; 4] = [
        "x-magpie-session",
        "x-claude-code-session-id",
        "x-session-id",
        "session_id",
    ];
    SESSION_HEADERS.iter().find_map(|name| {
        headers
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
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

    fn quota_now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-11-28T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn quota_windows_mark_only_full_unreset_accounts() {
        let now = quota_now();
        // `used` is a percentage: 100% is exhausted even without a limit.
        assert!(is_quota_exhausted(
            &json!({ "windows": [{ "used": 100 }] }),
            now
        ));
        assert!(is_quota_exhausted(
            &json!({ "windows": [{ "used": 120 }] }),
            now
        ));
        // `limit` is the window's own amount, never `used`'s denominator:
        // 50% of 10 credits still has half its allowance left.
        assert!(!is_quota_exhausted(
            &json!({ "windows": [{ "used": 50, "limit": 10 }] }),
            now
        ));
        // Full and not reset yet.
        assert!(is_quota_exhausted(
            &json!({ "windows": [{ "used": 100, "resetsAt": "2026-11-29T00:00:00Z" }] }),
            now
        ));
        // The reset moment already passed: the snapshot is stale.
        assert!(!is_quota_exhausted(
            &json!({ "windows": [{ "used": 100, "resetsAt": "2026-11-27T00:00:00Z" }] }),
            now
        ));
        // An unreadable reset is not evidence either.
        assert!(!is_quota_exhausted(
            &json!({ "windows": [{ "used": 100, "resetsAt": "soon" }] }),
            now
        ));
        // `aside` windows never stop the account, even at 100%.
        assert!(!is_quota_exhausted(
            &json!({ "windows": [{ "used": 100, "aside": true }] }),
            now
        ));
        assert!(is_quota_exhausted(
            &json!({ "windows": [{ "used": 100, "aside": false }] }),
            now
        ));
        // Room left.
        assert!(!is_quota_exhausted(
            &json!({ "windows": [{ "used": 99 }] }),
            now
        ));
        assert!(!is_quota_exhausted(
            &json!({ "windows": [{ "used": 99, "limit": 100 }] }),
            now
        ));
    }

    #[test]
    fn quota_balances_apply_only_without_windows() {
        let now = quota_now();
        assert!(is_quota_exhausted(&json!({ "balance": 0 }), now));
        assert!(is_quota_exhausted(&json!({ "balance": -3.5 }), now));
        assert!(is_quota_exhausted(&json!({ "balance": "0" }), now));
        assert!(is_quota_exhausted(&json!({ "balance": "$0" }), now));
        assert!(is_quota_exhausted(&json!({ "balance": "$0.00" }), now));
        assert!(!is_quota_exhausted(&json!({ "balance": 1.5 }), now));
        assert!(!is_quota_exhausted(&json!({ "balance": "$1.00" }), now));
        // Empty windows are no valid windows, so the balance decides.
        assert!(is_quota_exhausted(
            &json!({ "windows": [], "balance": 0 }),
            now
        ));
        // A non-empty window list always wins over the balance.
        assert!(!is_quota_exhausted(
            &json!({ "windows": [{ "used": 1, "limit": 100 }], "balance": 0 }),
            now
        ));
    }

    #[test]
    fn quota_shapes_that_are_not_objects_are_ignored() {
        let now = quota_now();
        assert!(!is_quota_exhausted(&json!({}), now));
        assert!(!is_quota_exhausted(&json!(null), now));
        assert!(!is_quota_exhausted(&json!("0"), now));
        assert!(!is_quota_exhausted(&json!([{ "used": 100 }]), now));
    }

    #[tokio::test]
    async fn quota_exhausted_accounts_are_rotated_out() {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let manager = crate::plugins::PluginManager::new(db.clone(), root.path().to_path_buf());
        manager.paths().ensure_dirs().unwrap();
        std::fs::write(
            manager.paths().auth_path(),
            json!({
                "demo": { "type": "oauth", "access": "one" },
                "demo#2": { "type": "oauth", "access": "two" },
            })
            .to_string(),
        )
        .unwrap();
        store::upsert_account_meta(
            &db,
            &PluginAccountMetaRecord {
                id: "demo".to_string(),
                plugin_id: "demo-plugin".to_string(),
                provider_id: "demo".to_string(),
                quota: json!({
                    "windows": [{ "used": 100, "limit": 100, "resetsAt": "2999-01-01T00:00:00Z" }],
                }),
                ..PluginAccountMetaRecord::default()
            },
        )
        .unwrap();

        let reference = PluginProviderRef {
            plugin_spec: "demo-plugin".to_string(),
            provider_id: "demo".to_string(),
            account_key: None,
            account_strategy: PluginAccountStrategy::AutoRotate,
        };
        // The exhausted account is never selected while a healthy one exists,
        // even across the round-robin cursor.
        for _ in 0..4 {
            assert_eq!(select_plugin_account(&manager, &reference), "demo#2");
        }

        // With only the exhausted account left, it is still attempted rather
        // than failing locally.
        store::upsert_account_meta(
            &db,
            &PluginAccountMetaRecord {
                id: "demo#2".to_string(),
                plugin_id: "demo-plugin".to_string(),
                enabled: false,
                ..PluginAccountMetaRecord::default()
            },
        )
        .unwrap();
        assert_eq!(select_plugin_account(&manager, &reference), "demo");
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
                ..PluginModelInfo::default()
            }],
            ..PluginProviderRow::default()
        };
        let endpoint =
            endpoint_from_row(&row, "claude-x", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(endpoint.base_url, "https://api.example.com");
        assert_eq!(endpoint.target_protocol, AiProtocol::AnthropicMessages);

        let fallback = endpoint_from_row(&row, "other", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(fallback.target_protocol, AiProtocol::OpenAiChat);
    }

    #[test]
    fn endpoint_carries_the_vendor_api_id_of_the_requested_model() {
        let row = PluginProviderRow {
            id: "demo".to_string(),
            spec: "demo-plugin".to_string(),
            api: "https://api.example.com/v1".to_string(),
            npm: "@ai-sdk/openai-compatible".to_string(),
            models: vec![
                PluginModelInfo {
                    id: "public-alias".to_string(),
                    api_id: "vendor-model".to_string(),
                    ..PluginModelInfo::default()
                },
                PluginModelInfo {
                    id: "same-id".to_string(),
                    api_id: "same-id".to_string(),
                    ..PluginModelInfo::default()
                },
                PluginModelInfo {
                    id: "no-api-id".to_string(),
                    ..PluginModelInfo::default()
                },
            ],
            ..PluginProviderRow::default()
        };

        let endpoint =
            endpoint_from_row(&row, "public-alias", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(endpoint.upstream_model_id.as_deref(), Some("vendor-model"));

        // A model whose api.id is its public id needs no rewrite.
        let endpoint =
            endpoint_from_row(&row, "same-id", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(endpoint.upstream_model_id, None);

        let endpoint =
            endpoint_from_row(&row, "no-api-id", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(endpoint.upstream_model_id, None);
    }

    fn model(id: &str, outputs_said: bool, outputs: &[&str]) -> PluginModelInfo {
        PluginModelInfo {
            id: id.to_string(),
            name: id.to_string(),
            outputs: outputs.iter().map(|output| output.to_string()).collect(),
            outputs_said,
            ..PluginModelInfo::default()
        }
    }

    #[test]
    fn a_model_url_overrides_the_provider_api_base() {
        let row = PluginProviderRow {
            id: "demo".to_string(),
            spec: "demo-plugin".to_string(),
            api: "https://api.example.com/v1".to_string(),
            npm: "@ai-sdk/openai-compatible".to_string(),
            models: vec![PluginModelInfo {
                id: "vision-model".to_string(),
                url: "https://media.example.com/v2/".to_string(),
                ..PluginModelInfo::default()
            }],
            ..PluginProviderRow::default()
        };
        let endpoint =
            endpoint_from_row(&row, "vision-model", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(endpoint.base_url, "https://media.example.com/v2");

        // A model without its own URL keeps the provider base.
        let endpoint = endpoint_from_row(&row, "other", AiProtocol::OpenAiChat).expect("endpoint");
        assert_eq!(endpoint.base_url, "https://api.example.com/v1");
    }

    #[test]
    fn chat_models_skip_declared_non_chat_outputs() {
        assert!(model_supports_chat(&model("chat", false, &[])));
        assert!(model_supports_chat(&model(
            "chat",
            true,
            &["text", "image"]
        )));
        assert!(model_supports_chat(&model("chat", true, &["TEXT"])));
        assert!(!model_supports_chat(&model("image", true, &["image"])));

        let row = PluginProviderRow {
            id: "demo".to_string(),
            models: vec![
                model("chat", true, &["text"]),
                model("image", true, &["image"]),
                model("unsaid", false, &[]),
            ],
            ..PluginProviderRow::default()
        };
        let models = models_for_cli(&row, PluginAccountStrategy::AutoRotate, None);
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["chat", "unsaid"]
        );
    }

    #[test]
    fn a_single_account_narrows_models_to_its_own_list() {
        let row = PluginProviderRow {
            id: "demo".to_string(),
            models: vec![
                model("alpha", true, &["text"]),
                model("beta", true, &["text"]),
                model("gamma", true, &["text"]),
            ],
            accounts: vec!["demo".to_string(), "demo#2".to_string()],
            account_models: HashMap::from([
                ("demo".to_string(), vec!["alpha".to_string()]),
                ("demo#2".to_string(), vec!["beta".to_string()]),
            ]),
            ..PluginProviderRow::default()
        };

        let pinned = models_for_cli(&row, PluginAccountStrategy::Single, Some("demo#2"));
        assert_eq!(
            pinned
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["beta"]
        );

        // No pinned key: the first listed account with its own list wins.
        let first = models_for_cli(&row, PluginAccountStrategy::Single, None);
        assert_eq!(
            first
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"]
        );

        // Rotation keeps the union.
        let union = models_for_cli(&row, PluginAccountStrategy::AutoRotate, Some("demo"));
        assert_eq!(union.len(), 3);
    }

    #[test]
    fn parse_retry_after_accepts_seconds_dates_and_clamps() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let header = |value: &str| vec![("retry-after".to_string(), value.to_string())];

        assert_eq!(
            parse_retry_after(&header("120"), now),
            Some(Duration::from_secs(120))
        );
        // Anything shorter than five seconds is raised to five.
        assert_eq!(
            parse_retry_after(&header("0"), now),
            Some(Duration::from_secs(5))
        );
        // Anything longer than an hour is capped.
        assert_eq!(
            parse_retry_after(&header("86400"), now),
            Some(Duration::from_secs(3600))
        );

        let at = chrono::DateTime::<chrono::Utc>::from(now) + chrono::Duration::seconds(90);
        assert_eq!(
            parse_retry_after(&header(&at.to_rfc2822()), now),
            Some(Duration::from_secs(90))
        );
        let past = chrono::DateTime::<chrono::Utc>::from(now) - chrono::Duration::seconds(90);
        assert_eq!(
            parse_retry_after(&header(&past.to_rfc2822()), now),
            Some(Duration::from_secs(5))
        );

        assert_eq!(parse_retry_after(&header("soon"), now), None);
        assert_eq!(parse_retry_after(&[], now), None);
    }

    #[test]
    fn provider_rows_carry_the_hosts_account_and_renewal_facts() {
        let rows = parse_provider_rows(&json!([{
            "id": "demo",
            "spec": "demo-plugin",
            "name": "Demo",
            "api": "https://api.example.com",
            "npm": "@ai-sdk/openai-compatible",
            "single": true,
            "renews": true,
            "renewLead": 900000,
            "rotating": true,
            "accounts": [
                { "key": "demo", "models": ["alpha"] },
                { "key": "demo#2" },
                { "key": "demo#3", "models": [] }
            ],
            "models": [{
                "id": "alpha",
                "name": "Alpha",
                "npm": "",
                "url": "https://alpha.example.com/v1",
                "apiId": "vendor-alpha",
                "also": ["@ai-sdk/anthropic"],
                "outputs": ["text"],
                "outputsSaid": true
            }]
        }]));

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.single);
        assert!(row.renews);
        assert_eq!(row.renew_lead_ms, Some(900_000));
        assert!(row.rotating);
        assert_eq!(
            row.accounts,
            vec![
                "demo".to_string(),
                "demo#2".to_string(),
                "demo#3".to_string()
            ]
        );
        assert_eq!(
            row.account_models.get("demo"),
            Some(&vec!["alpha".to_string()])
        );
        assert!(!row.account_models.contains_key("demo#2"));
        assert!(!row.account_models.contains_key("demo#3"));
        assert_eq!(row.models[0].url, "https://alpha.example.com/v1");
        assert_eq!(row.models[0].api_id, "vendor-alpha");
        assert_eq!(row.models[0].also, vec!["@ai-sdk/anthropic"]);
        assert_eq!(row.models[0].outputs, vec!["text"]);
        assert!(row.models[0].outputs_said);
    }
}
