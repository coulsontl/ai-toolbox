//! Optional runtime privacy policy. Wire conversion and credentials remain outside this module.
mod config;
mod payload;
pub(crate) mod stream;

pub(crate) use config::load_settings;
pub use config::{
    PrivacyCustomRule, PrivacyRuleKind, PrivacyRules, PrivacySettings, PrivacySettingsUpdate,
};

use super::types::GatewayCliKey;
use super::usage_parser::{
    anthropic_start_input_placeholder_bytes, from_response_body_with_provider_type,
    is_anthropic_cumulative_usage_snapshot_bytes, TokenUsage,
};
use crate::db::SqliteDbState;
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(crate) const TOKEN_PREFIX: &str = "__AITB_PRIV_";
const MAX_MAPPINGS: usize = 2048;
const MAX_MAPPING_BYTES: usize = 1024 * 1024;
const MAX_SESSIONS: usize = 128;
const MAX_RESPONSE_IDS: usize = 2048;
const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_MATCHES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
pub struct PrivacyDetail {
    pub matched_values: usize,
    pub restored_values: usize,
    pub rules: BTreeMap<String, usize>,
    pub log_redacted: bool,
    pub failed: bool,
}

struct CompiledRule {
    id: String,
    regex: Regex,
    priority: i32,
    capture_group: Option<&'static str>,
}

pub(crate) struct CompiledPolicy {
    enabled: bool,
    rules: Vec<CompiledRule>,
    allowlist: HashSet<String>,
    passwords: bool,
}

impl CompiledPolicy {
    fn disabled() -> Arc<Self> {
        Arc::new(Self {
            enabled: false,
            rules: Vec::new(),
            allowlist: HashSet::new(),
            passwords: false,
        })
    }
    pub(crate) fn compile(settings: &PrivacySettings) -> Result<Arc<Self>, String> {
        let config = &settings.rules;
        if config.custom.len() > 100 || config.allowlist.len() > 1000 {
            return Err(
                "privacy_config_limit: at most 100 custom rules and 1000 allowed values".into(),
            );
        }
        if config
            .allowlist
            .iter()
            .any(|value| value.is_empty() || value.len() > 4096)
        {
            return Err("privacy_allowlist_invalid: values must contain 1 to 4096 bytes".into());
        }
        let mut rules = Vec::new();
        let mut ids = HashSet::new();
        for id in &config.builtins {
            let (pattern, priority) = match id.as_str() {
                "credentials" => (
                    r"\b(?:sk-(?:proj-|ant-)?[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[A-Z0-9]{16}|xox[baprs]-[A-Za-z0-9-]{20,})\b",
                    80,
                ),
                "private_keys" => (
                    r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----",
                    100,
                ),
                "passwords" => (
                    r#"(?i)(?:password|passwd|pwd|api[_-]?key|access[_-]?token|client[_-]?secret)\s*[=:]\s*["']?(?P<secret>[^\s"',;}{]{4,})"#,
                    70,
                ),
                "connection_strings" => (
                    r#"(?i)\b(?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis|amqp(?:s)?|mssql)://[^\s/:]+:[^\s@]+@[^\s"<>]+"#,
                    90,
                ),
                "email" => (
                    r"\b[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+\b",
                    10,
                ),
                "ip" => (
                    r"\b(?:(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\b",
                    10,
                ),
                _ => return Err(format!("privacy_builtin_unknown: {id}")),
            };
            if !ids.insert(id.clone()) {
                return Err("privacy_duplicate_rule".into());
            }
            rules.push(CompiledRule {
                id: id.clone(),
                regex: Regex::new(pattern).map_err(|_| "privacy_builtin_invalid")?,
                priority,
                capture_group: (id == "passwords").then_some("secret"),
            });
        }
        for rule in &config.custom {
            if rule.id.is_empty()
                || rule.id.len() > 64
                || !rule
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                || !ids.insert(rule.id.clone())
            {
                return Err(
                    "privacy_rule_id_invalid: rule IDs must be unique ASCII identifiers".into(),
                );
            }
            if rule.name.trim().is_empty()
                || rule.name.len() > 200
                || rule.pattern.is_empty()
                || rule.pattern.len() > 4096
            {
                return Err(format!("privacy_rule_invalid: {}", rule.id));
            }
            let pattern = match rule.kind {
                PrivacyRuleKind::Literal => regex::escape(&rule.pattern),
                PrivacyRuleKind::Regex => rule.pattern.clone(),
            };
            let regex = RegexBuilder::new(&pattern).size_limit(1024 * 1024).build()
                .map_err(|_| format!("privacy_regex_invalid: {} (use Rust regex syntax; no lookaround or backreferences)", rule.id))?;
            if regex.is_match("") {
                return Err(format!("privacy_regex_empty_match: {}", rule.id));
            }
            if rule.enabled {
                rules.push(CompiledRule {
                    id: rule.id.clone(),
                    regex,
                    priority: rule.priority,
                    capture_group: None,
                });
            }
        }
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        Ok(Arc::new(Self {
            enabled: settings.enabled,
            rules,
            allowlist: config.allowlist.iter().cloned().collect(),
            passwords: config.builtins.iter().any(|id| id == "passwords"),
        }))
    }

    fn matches<'a>(
        &'a self,
        text: &str,
        credential: bool,
    ) -> Result<Vec<(usize, usize, &'a str)>, String> {
        if self.allowlist.contains(text) {
            return Ok(Vec::new());
        }
        let mut selected = Vec::new();
        let protected = placeholder_ranges(text);
        if credential && self.passwords && !text.is_empty() && !text.starts_with(TOKEN_PREFIX) {
            return Ok(vec![(0, text.len(), "passwords")]);
        }
        for rule in &self.rules {
            for captures in rule.regex.captures_iter(text) {
                let found = rule
                    .capture_group
                    .and_then(|name| captures.name(name))
                    .or_else(|| captures.get(0))
                    .expect("regex match");
                if found.is_empty() {
                    return Err("privacy_regex_empty_match".into());
                }
                if self.allowlist.contains(found.as_str()) || found.as_str().contains(TOKEN_PREFIX)
                {
                    continue;
                }
                let start = found.start();
                let end = found.end();
                if protected
                    .iter()
                    .any(|(left, right)| start < *right && end > *left)
                {
                    continue;
                }
                if !selected
                    .iter()
                    .any(|(left, right, _)| start < *right && end > *left)
                {
                    selected.push((start, end, rule.id.as_str()));
                    if selected.len() > MAX_MATCHES {
                        return Err("privacy_match_limit".into());
                    }
                }
            }
        }
        selected.sort_by_key(|(start, _, _)| *start);
        Ok(selected)
    }
}

fn placeholder_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut protected = Vec::new();
    let mut offset = 0;
    while let Some(start) = text[offset..].find(TOKEN_PREFIX) {
        let start = offset + start;
        let Some(end) = text[start + TOKEN_PREFIX.len()..].find("__") else {
            break;
        };
        offset = start + TOKEN_PREFIX.len() + end + 2;
        protected.push((start, offset));
    }
    protected
}

/// Choose ranges in the original text once, so short secrets cannot rewrite generated tokens.
fn redact_known_values(text: &str, originals: &[(String, String)]) -> Result<String, String> {
    let protected = placeholder_ranges(text);
    let mut selected = Vec::new();
    for (token, original) in originals {
        for (start, matched) in text.match_indices(original.as_str()) {
            let end = start + matched.len();
            if protected
                .iter()
                .any(|(left, right)| start < *right && end > *left)
                || selected
                    .iter()
                    .any(|(left, right, _)| start < *right && end > *left)
            {
                continue;
            }
            selected.push((start, end, token.as_str()));
            if selected.len() > MAX_MATCHES {
                return Err("privacy_match_limit".into());
            }
        }
    }
    selected.sort_by_key(|(start, _, _)| *start);
    let mut result = String::new();
    let mut offset = 0;
    for (start, end, token) in selected {
        result.push_str(&text[offset..start]);
        result.push_str(token);
        offset = end;
    }
    result.push_str(&text[offset..]);
    Ok(result)
}

#[derive(Default)]
struct Mapping {
    namespace: String,
    values: Vec<(String, String)>,
    originals: HashMap<String, usize>,
    bytes: usize,
}

impl Mapping {
    fn token(&mut self, original: &str) -> Result<String, String> {
        if let Some(index) = self.originals.get(original) {
            return Ok(self.values[*index].0.clone());
        }
        if self.values.len() >= MAX_MAPPINGS
            || self.bytes.saturating_add(original.len()) > MAX_MAPPING_BYTES
        {
            return Err("privacy_mapping_limit: start a new conversation".into());
        }
        if self.namespace.is_empty() {
            self.namespace = Uuid::new_v4().simple().to_string();
        }
        let token = format!("{TOKEN_PREFIX}{}_{:x}__", self.namespace, self.values.len());
        self.originals
            .insert(original.to_string(), self.values.len());
        self.bytes += original.len();
        self.values.push((token.clone(), original.to_string()));
        Ok(token)
    }
}

struct SessionEntry {
    touched: Instant,
    mapping: Arc<Mutex<Mapping>>,
}
struct ResponseEntry {
    touched: Instant,
    mapping: Weak<Mutex<Mapping>>,
    session: Option<String>,
}
#[derive(Default)]
struct MappingStore {
    sessions: HashMap<String, SessionEntry>,
    responses: HashMap<String, ResponseEntry>,
}

impl MappingStore {
    fn prune(&mut self) {
        self.sessions.retain(|_, entry| {
            entry.touched.elapsed() < SESSION_TTL || Arc::strong_count(&entry.mapping) > 1
        });
        self.responses.retain(|_, entry| {
            entry.touched.elapsed() < SESSION_TTL && entry.mapping.strong_count() > 0
        });
    }
    fn mapping(
        &mut self,
        namespace: &str,
        session: Option<&str>,
        previous: Option<&str>,
        provider: &str,
    ) -> Result<Arc<Mutex<Mapping>>, String> {
        self.prune();
        let mapping = if let Some(previous) = previous {
            let key = format!("{namespace}:{provider}:{previous}");
            let entry = self.responses.get_mut(&key).ok_or("privacy_history_missing: previous response mapping expired or belongs to another provider; start a new conversation")?;
            if entry.session.as_deref() != session {
                return Err("privacy_history_scope_mismatch".into());
            }
            entry.touched = Instant::now();
            entry
                .mapping
                .upgrade()
                .ok_or("privacy_history_missing: mapping expired; start a new conversation")?
        } else if let Some(entry) = session.and_then(|key| self.sessions.get_mut(key)) {
            entry.touched = Instant::now();
            entry.mapping.clone()
        } else {
            Arc::new(Mutex::new(Mapping::default()))
        };
        let retention_key = session
            .map(str::to_string)
            .unwrap_or_else(|| format!("{namespace}:{}", Uuid::new_v4()));
        {
            if self.sessions.len() >= MAX_SESSIONS && !self.sessions.contains_key(&retention_key) {
                if let Some(oldest) = self
                    .sessions
                    .iter()
                    .min_by_key(|(_, e)| e.touched)
                    .map(|(key, _)| key.clone())
                {
                    self.sessions.remove(&oldest);
                }
            }
            self.sessions.insert(
                retention_key,
                SessionEntry {
                    touched: Instant::now(),
                    mapping: mapping.clone(),
                },
            );
        }
        Ok(mapping)
    }
}

#[derive(Clone)]
pub(crate) struct PrivacyRuntime {
    policy: Arc<RwLock<Result<Arc<CompiledPolicy>, String>>>,
    mappings: Arc<Mutex<MappingStore>>,
    epoch: Arc<AtomicU64>,
}

impl PrivacyRuntime {
    pub(crate) fn new(db: Option<&SqliteDbState>) -> Self {
        let policy = db
            .map(load_settings)
            .unwrap_or_else(|| Ok(PrivacySettings::default()))
            .and_then(|settings| {
                if settings.enabled {
                    CompiledPolicy::compile(&settings)
                } else {
                    Ok(CompiledPolicy::disabled())
                }
            });
        Self {
            policy: Arc::new(RwLock::new(policy)),
            mappings: Arc::new(Mutex::new(MappingStore::default())),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn publish(&self, policy: Arc<CompiledPolicy>) {
        let mut current = self
            .policy
            .write()
            .unwrap_or_else(|error| error.into_inner());
        // Old requests keep their mapping Arc; mode changes start a new privacy epoch.
        if current.as_ref().map(|p| p.enabled).ok() != Some(policy.enabled) {
            self.epoch.fetch_add(1, Ordering::Relaxed);
            *self
                .mappings
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = MappingStore::default();
        }
        *current = Ok(policy);
    }

    pub(crate) fn enabled(&self) -> bool {
        self.policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|p| p.enabled)
            .unwrap_or(true)
    }

    pub(crate) fn begin(
        &self,
        cli: &str,
        headers: &[(String, String)],
        body: &[u8],
        connection: Option<&str>,
    ) -> Result<Option<PrivacyRequest>, String> {
        let (policy, epoch) = {
            let current = self
                .policy
                .read()
                .unwrap_or_else(|error| error.into_inner());
            (current.clone()?, self.epoch.load(Ordering::Relaxed))
        };
        if !policy.enabled {
            return Ok(None);
        }
        let value: Value = serde_json::from_slice(body)
            .map_err(|_| "privacy_request_invalid: expected a JSON request")?;
        if !value.is_object() {
            return Err("privacy_request_invalid: expected a JSON object".into());
        }
        let header = |names: &[&str]| {
            headers
                .iter()
                .find(|(name, value)| {
                    !value.trim().is_empty()
                        && names.iter().any(|key| name.eq_ignore_ascii_case(key))
                })
                .map(|(_, value)| value.as_str())
        };
        let namespace = format!(
            "{epoch}:{cli}:{:x}",
            Sha256::digest(header(&["authorization", "x-api-key"]).unwrap_or_default())
        );
        let turn_metadata = header(&["x-codex-turn-metadata"])
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        let session = header(&[
            "session_id",
            "x-ai-toolbox-session-id",
            "x-session-id",
            "x-conversation-id",
            "chatgpt-conversation-id",
        ])
        .or_else(|| {
            value
                .pointer("/metadata/session_id")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value
                .pointer("/metadata/conversation_id")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            turn_metadata
                .as_ref()
                .and_then(|value| value.get("session_id"))
                .and_then(Value::as_str)
        })
        .or(connection)
        .filter(|id| !id.is_empty())
        .map(|id| format!("{namespace}:{:x}", Sha256::digest(id)));
        let previous = value
            .get("previous_response_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let cli_key = match cli {
            "claude" => GatewayCliKey::Claude,
            "claude_desktop" => GatewayCliKey::ClaudeDesktop,
            "gemini" => GatewayCliKey::Gemini,
            "kimi" => GatewayCliKey::Kimi,
            "grok" => GatewayCliKey::Grok,
            _ => GatewayCliKey::Codex,
        };
        Ok(Some(PrivacyRequest {
            policy,
            store: self.mappings.clone(),
            namespace,
            session,
            previous,
            cli_key,
            state: Arc::new(Mutex::new(RequestState::default())),
        }))
    }
}

#[derive(Default)]
struct RequestState {
    mapping: Option<Arc<Mutex<Mapping>>>,
    provider: String,
    matched_tokens: HashSet<String>,
    detail: PrivacyDetail,
    usage: TokenUsage,
    usage_provider_type: Option<String>,
    /// Set once an Anthropic `message_delta` snapshot replaced the accumulated
    /// input count, so a repeat of that input may still bring final cache
    /// counters (see `TokenUsage::apply_snapshot`).
    usage_input_from_delta: bool,
    /// Set once an Anthropic `message_start` reported a positive input count.
    /// Until then the start is a placeholder, so any positive delta may fill it.
    usage_start_input_authoritative: bool,
    /// Set when the observed bytes came from our own conversion layer; its
    /// `message_start` usage is a fixed placeholder, not an upstream estimate.
    usage_gateway_generated: bool,
}

#[derive(Clone)]
pub(crate) struct PrivacyRequest {
    policy: Arc<CompiledPolicy>,
    store: Arc<Mutex<MappingStore>>,
    namespace: String,
    session: Option<String>,
    previous: Option<String>,
    cli_key: GatewayCliKey,
    state: Arc<Mutex<RequestState>>,
}

impl PrivacyRequest {
    pub(crate) fn prepare(&self, body: &[u8], provider: &str) -> Result<Vec<u8>, String> {
        {
            let mut state = self.state.lock().map_err(|_| "privacy_state_unavailable")?;
            if self.previous.is_some() && !state.provider.is_empty() && state.provider != provider {
                return Err("privacy_history_provider_changed: start a new conversation".into());
            }
            if state.mapping.is_none() {
                state.mapping = Some(
                    self.store
                        .lock()
                        .map_err(|_| "privacy_state_unavailable")?
                        .mapping(
                            &self.namespace,
                            self.session.as_deref(),
                            self.previous.as_deref(),
                            provider,
                        )?,
                );
            }
            state.provider = provider.to_string();
        }
        self.transform_body(body, false)
    }

    pub(crate) fn restore(&self, body: &[u8]) -> Result<Vec<u8>, String> {
        self.transform_body(body, true)
    }

    fn transform_body(&self, body: &[u8], restoring: bool) -> Result<Vec<u8>, String> {
        if body.is_empty() {
            return Ok(body.to_vec());
        }
        let mut value: Value =
            serde_json::from_slice(body).map_err(|_| "privacy_payload_invalid: expected JSON")?;
        let changed = self.transform_value(&mut value, restoring)?;
        if changed {
            serde_json::to_vec(&value).map_err(|_| "privacy_payload_invalid".into())
        } else {
            Ok(body.to_vec())
        }
    }

    pub(crate) fn transform_value(
        &self,
        value: &mut Value,
        restoring: bool,
    ) -> Result<bool, String> {
        payload::transform(
            value,
            &mut |text, credential| {
                if restoring {
                    self.restore_text(text, false).map(|v| v.0)
                } else {
                    self.redact_text(text, credential)
                }
            },
            restoring,
        )
    }

    pub(crate) fn redact_text(&self, text: &str, credential: bool) -> Result<String, String> {
        let matches = self.policy.matches(text, credential)?;
        if matches.is_empty() {
            return Ok(text.to_string());
        }
        let mut state = self.state.lock().map_err(|_| "privacy_state_unavailable")?;
        let mapping = state
            .mapping
            .get_or_insert_with(|| Arc::new(Mutex::new(Mapping::default())))
            .clone();
        let mut mapping = mapping.lock().map_err(|_| "privacy_state_unavailable")?;
        let mut out = String::new();
        let mut offset = 0;
        for (start, end, rule) in matches {
            out.push_str(&text[offset..start]);
            let token = mapping.token(&text[start..end])?;
            if state.matched_tokens.insert(token.clone()) {
                state.detail.matched_values += 1;
                *state.detail.rules.entry(rule.to_string()).or_default() += 1;
            }
            out.push_str(&token);
            offset = end;
        }
        out.push_str(&text[offset..]);
        Ok(out)
    }

    /// Partial tokens are held only by the event restorer, never emitted to the client.
    pub(crate) fn restore_text(
        &self,
        text: &str,
        json_fragment: bool,
    ) -> Result<(String, usize), String> {
        if !text.contains(TOKEN_PREFIX) {
            return Ok((text.to_string(), 0));
        }
        let mut state = self.state.lock().map_err(|_| "privacy_state_unavailable")?;
        let mapping = state.mapping.clone().ok_or("privacy_mapping_missing")?;
        let mapping = mapping.lock().map_err(|_| "privacy_state_unavailable")?;
        let mut out = String::new();
        let mut remainder = text;
        let mut count = 0;
        while let Some(start) = remainder.find(TOKEN_PREFIX) {
            out.push_str(&remainder[..start]);
            let token_text = &remainder[start..];
            let end = token_text[TOKEN_PREFIX.len()..]
                .find("__")
                .map(|end| TOKEN_PREFIX.len() + end + 2)
                .ok_or("privacy_token_incomplete")?;
            let token = &token_text[..end];
            let original = mapping
                .values
                .iter()
                .find(|(candidate, _)| candidate == token)
                .map(|(_, value)| value)
                .ok_or("privacy_token_unknown: mapping unavailable; start a new conversation")?;
            if json_fragment {
                let escaped =
                    serde_json::to_string(original).map_err(|_| "privacy_payload_invalid")?;
                out.push_str(&escaped[1..escaped.len() - 1]);
            } else {
                out.push_str(original);
            }
            count += 1;
            remainder = &token_text[end..];
        }
        out.push_str(remainder);
        state.detail.restored_values += count;
        Ok((out, count))
    }

    pub(crate) fn remember_response(&self, value: &Value) {
        let response = value.get("response").unwrap_or(value);
        let Some(id) = response.get("id").and_then(Value::as_str) else {
            return;
        };
        if id.len() > 512 {
            return;
        }
        let Ok(state) = self.state.lock() else {
            return;
        };
        let Some(mapping) = &state.mapping else {
            return;
        };
        let Ok(mut store) = self.store.lock() else {
            return;
        };
        for entry in store.sessions.values_mut() {
            if Arc::ptr_eq(&entry.mapping, mapping) {
                entry.touched = Instant::now();
            }
        }
        store.prune();
        if store.responses.len() >= MAX_RESPONSE_IDS {
            if let Some(oldest) = store
                .responses
                .iter()
                .min_by_key(|(_, e)| e.touched)
                .map(|(key, _)| key.clone())
            {
                store.responses.remove(&oldest);
            }
        }
        store.responses.insert(
            format!("{}:{}:{id}", self.namespace, state.provider),
            ResponseEntry {
                touched: Instant::now(),
                mapping: Arc::downgrade(mapping),
                session: self.session.clone(),
            },
        );
    }

    pub(crate) fn observe_usage(&self, bytes: &[u8]) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(placeholder) = anthropic_start_input_placeholder_bytes(bytes) {
                state.usage_start_input_authoritative = !placeholder;
            }
            let usage = from_response_body_with_provider_type(
                self.cli_key,
                state.usage_provider_type.as_deref(),
                bytes,
            );
            // The redactor sees one SSE `data` payload at a time, so the
            // collector never gets to fold an Anthropic cumulative
            // `message_delta` into its `message_start` estimate; apply the same
            // rule here instead of max-merging the two snapshots.
            if !state.usage_gateway_generated && is_anthropic_cumulative_usage_snapshot_bytes(bytes) {
                let input_from_delta = state.usage_input_from_delta;
                let start_input_placeholder = !state.usage_start_input_authoritative;
                state.usage_input_from_delta |= state.usage.apply_snapshot(
                    usage,
                    input_from_delta,
                    start_input_placeholder,
                );
            } else {
                state.usage.merge_max(usage);
            }
        }
    }

    /// Mark the observed stream as produced by our own conversion layer, whose
    /// `message_start` usage is a fixed placeholder rather than an upstream
    /// estimate. Must be set before the stream is wrapped.
    pub(crate) fn set_usage_gateway_generated(&self, gateway_generated: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.usage_gateway_generated = gateway_generated;
        }
    }

    pub(crate) fn set_usage_provider(&self, provider_type: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.usage_provider_type = provider_type;
        }
    }

    pub(crate) fn usage(&self) -> TokenUsage {
        self.state
            .lock()
            .map(|state| state.usage.clone())
            .unwrap_or_default()
    }

    pub(crate) fn fail(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.detail.failed = true;
        }
    }
    pub(crate) fn detail(&self) -> PrivacyDetail {
        self.state
            .lock()
            .map(|s| s.detail.clone())
            .unwrap_or_else(|_| PrivacyDetail {
                failed: true,
                ..Default::default()
            })
    }

    /// Logs are independent copies; truncated/invalid payloads must never fall back to plaintext.
    pub(crate) fn log_body(&self, body: &[u8], original_len: u64) -> String {
        if (body.len() as u64) < original_len {
            return "[privacy: truncated body omitted]".into();
        }
        let preview = self.isolated();
        let originals = self.known_values();
        let mut redact = |text: &str, credential: bool| {
            let text = redact_known_values(text, &originals)?;
            preview.redact_text(&text, credential)
        };
        if let Ok(mut value) = serde_json::from_slice::<Value>(body) {
            if payload::transform_business(&mut value, &mut redact, true).is_ok() {
                return value.to_string();
            }
        } else if let Ok(log) = stream::redact_log(body, &mut redact) {
            return log;
        }
        "[privacy: unparseable or incomplete body omitted]".into()
    }

    fn known_values(&self) -> Vec<(String, String)> {
        let mut values = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.mapping.clone())
            .and_then(|mapping| mapping.lock().ok().map(|mapping| mapping.values.clone()))
            .unwrap_or_default();
        values.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        values
    }

    pub(crate) fn log_text(&self, text: &str) -> String {
        redact_known_values(text, &self.known_values())
            .and_then(|text| self.isolated().redact_text(&text, false))
            .unwrap_or_else(|_| "[privacy: text omitted]".into())
    }

    fn isolated(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            store: Arc::new(Mutex::new(MappingStore::default())),
            namespace: String::new(),
            session: None,
            previous: None,
            cli_key: self.cli_key,
            state: Arc::new(Mutex::new(RequestState::default())),
        }
    }
}

#[derive(Serialize)]
pub struct PrivacyPreview {
    pub redacted: String,
    pub restored: String,
    pub detail: PrivacyDetail,
}

pub(crate) fn preview(rules: PrivacyRules, text: String) -> Result<PrivacyPreview, String> {
    if text.len() > 64 * 1024 {
        return Err("privacy_preview_limit: at most 64 KiB".into());
    }
    let runtime = PrivacyRuntime::new(None);
    runtime.publish(CompiledPolicy::compile(&PrivacySettings {
        enabled: true,
        rules,
    })?);
    let request = runtime
        .begin("preview", &[], b"{}", None)?
        .ok_or("privacy_preview_unavailable")?;
    let redacted = request.redact_text(&text, false)?;
    let restored = request.restore_text(&redacted, false)?.0;
    Ok(PrivacyPreview {
        redacted,
        restored,
        detail: request.detail(),
    })
}

/// Caller serializes the entire read/compile/save/publish operation with the manager lock.
pub(crate) fn update_settings(
    db: &SqliteDbState,
    update: PrivacySettingsUpdate,
) -> Result<(PrivacySettings, Arc<CompiledPolicy>), String> {
    let mut settings = load_settings(db)?;
    let validate_rules = update.rules.is_some();
    if let Some(enabled) = update.enabled {
        settings.enabled = enabled;
    }
    if let Some(rules) = update.rules {
        settings.rules = rules;
    }
    let policy = if !settings.enabled && !validate_rules {
        CompiledPolicy::disabled()
    } else {
        CompiledPolicy::compile(&settings)?
    };
    config::save_settings(db, &settings)?;
    Ok((settings, policy))
}

#[cfg(test)]
mod tests;
