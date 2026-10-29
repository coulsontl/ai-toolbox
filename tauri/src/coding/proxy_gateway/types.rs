use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayCliKey {
    Claude,
    ClaudeDesktop,
    Codex,
    Grok,
    Kimi,
    Gemini,
    Antigravity,
    OpenCode,
}

impl GatewayCliKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ClaudeDesktop => "claude_desktop",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Kimi => "kimi",
            Self::Gemini => "gemini",
            Self::Antigravity => "antigravity",
            Self::OpenCode => "opencode",
        }
    }

    pub fn supported_mvp() -> Vec<Self> {
        vec![
            Self::Claude,
            Self::ClaudeDesktop,
            Self::Codex,
            Self::Grok,
            Self::Kimi,
            Self::Gemini,
            Self::Antigravity,
        ]
    }
}

/// Usage collection is independent of gateway takeover support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayUsageTool {
    Claude,
    ClaudeDesktop,
    Codex,
    Grok,
    Kimi,
    Gemini,
    Antigravity,
    #[serde(rename = "opencode", alias = "open_code")]
    OpenCode,
    Pi,
    OhMyPi,
    OmoNative,
    Dsh,
    Hermes,
    #[serde(rename = "openclaw", alias = "open_claw")]
    OpenClaw,
    KimiCli,
    /// ZCode's CLI session database (`cli/db/db.sqlite`). Usage-only, like the
    /// other native-transcript tools: the gateway never proxies it.
    Zcode,
    /// MiMoCode's OpenCode-derived SQLite store. Usage-only.
    Mimocode,
    /// Prime Agent's session JSONL tree. Usage-only.
    PrimeAgent,
    /// Devin CLI's SQLite session store. Usage-only.
    Devin,
    /// Muse Code's session event tree. Usage-only.
    Musecode,
    /// Command Code's per-project transcript tree. Usage-only.
    Cmdcode,
    /// Qwen Code's per-request and per-session usage files. Usage-only.
    Qwencode,
    /// WorkBuddy's Claude-Code-shaped transcripts. Usage-only.
    Workbuddy,
    /// WorkBuddy international transcripts, kept separate from the mainland
    /// edition. Usage-only.
    WorkbuddyAi,
    /// CodeBuddy Code's transcripts. Usage-only.
    Codebuddy,
    /// MiniMax Code's runtime state database and logs. Usage-only.
    Minimax,
    /// Qoder Desktop (international IDE) chat history database. Usage-only.
    QoderIde,
    /// Qoder CN Desktop (mainland IDE) chat history database. Usage-only and
    /// kept separate from `qoder_ide` so the two accounts never merge.
    QoderIdeCn,
    /// Qoder CLI's Agent transcript tree (`~/.qoder/projects`). Usage-only.
    ///
    /// The explicit rename is required: `rename_all = "snake_case"` would spell
    /// this `qoder_cli`, while `as_str()` (the value stored in `app_type` and
    /// sent as the frontend filter) is `qodercli`. A mismatch leaves every
    /// statistics label untranslated and makes the frontend's cli filter miss
    /// the rows.
    #[serde(rename = "qodercli")]
    QoderCli,
    /// Qoder CN (mainland edition) transcript tree (`~/.qoder-cn/projects`),
    /// kept separate from `qodercli` so the two editions never merge.
    #[serde(rename = "qodercli_cn")]
    QoderCliCn,
}

impl GatewayUsageTool {
    pub fn all() -> Vec<Self> {
        vec![
            Self::Claude,
            Self::ClaudeDesktop,
            Self::Codex,
            Self::Grok,
            Self::Kimi,
            Self::Gemini,
            Self::Antigravity,
            Self::OpenCode,
            Self::Pi,
            Self::OhMyPi,
            Self::OmoNative,
            Self::Dsh,
            Self::Hermes,
            Self::OpenClaw,
            Self::KimiCli,
            Self::Zcode,
            Self::Mimocode,
            Self::PrimeAgent,
            Self::Devin,
            Self::Musecode,
            Self::Cmdcode,
            Self::Qwencode,
            Self::Workbuddy,
            Self::WorkbuddyAi,
            Self::Codebuddy,
            Self::Minimax,
            Self::QoderIde,
            Self::QoderIdeCn,
            Self::QoderCli,
            Self::QoderCliCn,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ClaudeDesktop => "claude_desktop",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Kimi => "kimi",
            Self::Gemini => "gemini",
            Self::Antigravity => "antigravity",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::OhMyPi => "oh_my_pi",
            Self::OmoNative => "omo_native",
            Self::Dsh => "dsh",
            Self::Hermes => "hermes",
            Self::OpenClaw => "openclaw",
            Self::KimiCli => "kimi_cli",
            Self::Zcode => "zcode",
            Self::Mimocode => "mimocode",
            Self::PrimeAgent => "prime_agent",
            Self::Devin => "devin",
            Self::Musecode => "musecode",
            Self::Cmdcode => "cmdcode",
            Self::Qwencode => "qwencode",
            Self::Workbuddy => "workbuddy",
            Self::WorkbuddyAi => "workbuddy_ai",
            Self::Codebuddy => "codebuddy",
            Self::Minimax => "minimax",
            Self::QoderIde => "qoder_ide",
            Self::QoderIdeCn => "qoder_ide_cn",
            Self::QoderCli => "qodercli",
            Self::QoderCliCn => "qodercli_cn",
        }
    }

    pub fn gateway_cli(self) -> Option<GatewayCliKey> {
        Some(match self {
            Self::Claude => GatewayCliKey::Claude,
            Self::ClaudeDesktop => GatewayCliKey::ClaudeDesktop,
            Self::Codex => GatewayCliKey::Codex,
            Self::Grok => GatewayCliKey::Grok,
            Self::Kimi => GatewayCliKey::Kimi,
            Self::Gemini => GatewayCliKey::Gemini,
            Self::Antigravity => GatewayCliKey::Antigravity,
            Self::OpenCode => GatewayCliKey::OpenCode,
            _ => return None,
        })
    }
}

impl From<GatewayCliKey> for GatewayUsageTool {
    fn from(value: GatewayCliKey) -> Self {
        match value {
            GatewayCliKey::Claude => Self::Claude,
            GatewayCliKey::ClaudeDesktop => Self::ClaudeDesktop,
            GatewayCliKey::Codex => Self::Codex,
            GatewayCliKey::Grok => Self::Grok,
            GatewayCliKey::Kimi => Self::Kimi,
            GatewayCliKey::Gemini => Self::Gemini,
            GatewayCliKey::Antigravity => Self::Antigravity,
            GatewayCliKey::OpenCode => Self::OpenCode,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionUsageGranularity {
    #[default]
    Request,
    Turn,
    Session,
}

/// Only native facts are saved here; current provider settings are not history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct SessionUsageMetadata {
    pub granularity: SessionUsageGranularity,
    pub native_provider: Option<String>,
    pub call_count: Option<u64>,
    pub reported_total_tokens: Option<u64>,
    pub incomplete: bool,
    pub cost_source: Option<String>,
    pub window_start: Option<i64>,
    pub window_end: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayProxyMode {
    Single,
    Failover,
    /// Aggregate mode keeps every selected provider as a candidate and routes
    /// each request by the model name the CLI asked for. The model catalog
    /// exposes one entry per (site, model) pair named `<site_id><sep><model>`;
    /// the gateway strips that prefix before forwarding. Providers that declare
    /// the same upstream model act as fallbacks for each other.
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayAggregateConfig {
    pub provider_ids: Vec<String>,
    pub separator: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, String>,
    pub naming: crate::coding::proxy_gateway::aggregate_naming::AggregateNamingMode,
    /// Whether one request may be retried on *another* selected site when the
    /// site its slug or `<site><sep><model>` prefix names fails.
    ///
    /// `false` (the value every manifest written before this field existed
    /// deserializes to) pins the request to the single site the model name
    /// addresses: a failing or cooling site surfaces its own error instead of
    /// spending another site's balance. `true` restores the historical
    /// "sites declaring the same upstream model back each other up" behavior.
    #[serde(default)]
    pub cross_site_failover: bool,
    /// Bare upstream model names promoted into Codex's visible model list.
    ///
    /// Absent or empty keeps the historical default: every bare model remains
    /// addressable, but none is promoted into the picker. A name is an exact
    /// hidden alias unless it already matches a visible site slug. Non-empty
    /// values are ordered by tick order and promote those names into the visible
    /// catalog; unticked names remain hidden aliases unless such a slug exists.
    ///
    /// Order is the user's tick order and is the priority order of the five
    /// `spawn_agent` model hints: the first entry is promoted to the lowest
    /// priority and becomes visible in the picker. A `Vec` keeps that order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagent_exposed_models: Vec<String>,
    /// Codex `[agents]` defaults this takeover owns, if any. Absent means the
    /// takeover wrote none, so the settings form must show the fields as
    /// unmanaged rather than as blank values it would then write back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<GatewayAggregateSubagentDefaults>,
}

/// One bare model name the drawer may promote into the picker or leave addressable only.
///
/// Carries its declaring site so the settings form can show where a name comes
/// from even when it is currently *not* published (the debt this fixes: a name
/// removed from the exposure set has to stay selectable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayAggregateBareModel {
    pub model: String,
    pub provider_id: String,
    pub provider_name: String,
}

/// Read-only view of the aggregate catalog's programmable bare names.
///
/// `entries` are bare names currently published in the catalog (visible or
/// hidden); `bare_models` is the full universe the selected sites declare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewaySubagentCatalog {
    pub cli_key: GatewayCliKey,
    /// True while an aggregate manifest is enabled for this CLI.
    pub aggregate_mode: bool,
    #[serde(default)]
    pub entries: Vec<GatewayAggregateBareModel>,
    /// Absent in payloads written before this field existed; the drawer then
    /// falls back to `entries` alone.
    #[serde(default)]
    pub bare_models: Vec<GatewayAggregateBareModel>,
}

/// The `[agents]` defaults shown back to the settings form.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayAggregateSubagentDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl From<&crate::coding::proxy_gateway::cli_proxy::manifest::AggregateSubagentDefaults>
    for GatewayAggregateSubagentDefaults
{
    fn from(
        defaults: &crate::coding::proxy_gateway::cli_proxy::manifest::AggregateSubagentDefaults,
    ) -> Self {
        Self {
            model: defaults.model.clone(),
            reasoning_effort: defaults.reasoning_effort.clone(),
        }
    }
}

impl GatewayProxyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Failover => "failover",
            Self::Aggregate => "aggregate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct AppProxyConfig {
    pub streaming_first_byte_timeout_secs: Option<u64>,
    pub streaming_idle_timeout_secs: Option<u64>,
    pub non_streaming_timeout_secs: Option<u64>,
    pub per_provider_retry_count: Option<u32>,
    pub max_retry_count: Option<u32>,
    pub retry_interval_secs: Option<u64>,
    pub cost_multiplier: Option<String>,
    pub pricing_model_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProviderGatewayMeta {
    #[serde(rename = "gatewayProfile", alias = "gateway_profile")]
    pub gateway_profile: Option<GatewayProviderProfileReference>,
    pub provider_type: Option<String>,
    pub api_format: Option<String>,
    pub api_key_field: Option<String>,
    pub is_full_url: bool,
    pub prompt_cache_key: Option<String>,
    pub reasoning_field: Option<String>,
    #[serde(alias = "defaultMaxTokens")]
    pub default_max_tokens: Option<i64>,
    pub codex_chat_reasoning: Option<CodexChatReasoningMeta>,
    /// User opt-in (Codex provider form) to forward the client's explicit
    /// reasoning effort to a custom OpenAI Chat upstream as the top-level
    /// `reasoning_effort` field instead of letting the generic third-party Chat
    /// cleanup strip it. `None` keeps the existing provider-specific defaults
    /// (see `runtime::upstream::should_preserve_chat_reasoning_effort`); only
    /// meaningful for a Chat target with no recognized vendor profile.
    #[serde(default, alias = "preserveReasoningEffort")]
    pub preserve_reasoning_effort: Option<bool>,
    pub image_input_policy: Option<String>,
    pub text_only_models: Vec<String>,
    pub image_capable_models: Vec<String>,
    pub allow_text_only_model_heuristic: bool,
    pub cost_multiplier: String,
    pub pricing_model_source: String,
    /// Extra multiplier this channel charges on top of `cost_multiplier` when
    /// it serves the Fast service tier (`service_tier = priority|fast`). Empty
    /// means no premium (1.0): the recorded cost only changes once the user
    /// opts in, because OpenAI publishes no single ratio for the tier.
    #[serde(default, alias = "fastCostMultiplier")]
    pub fast_cost_multiplier: Option<String>,
    /// Same as [`Self::fast_cost_multiplier`] for the Ultrafast tier.
    #[serde(default, alias = "ultrafastCostMultiplier")]
    pub ultrafast_cost_multiplier: Option<String>,
    /// Provider-level custom request-header overrides applied to upstream
    /// requests. Each entry is one operation (`set`/`delete`/`rename`/`copy`).
    /// Applied last in `build_upstream_headers`, so overrides win over every
    /// preceding injector. Copilot providers skip operations whose affected
    /// header names fall inside `COPILOT_MANAGED_HEADERS` to preserve the
    /// fingerprint managed by `inject_copilot_headers`.
    #[serde(default, rename = "customHeaders", alias = "custom_headers")]
    pub custom_headers: Option<Vec<CustomHeaderOverride>>,
    /// Provider-level user-defined exact model rewrite rules (issue #321).
    /// When the CLI requests `from`, the gateway forwards `to` to this
    /// provider instead. Applied in every proxy mode (connectivity tests
    /// keep the pinned model) and wins over family/default mapping.
    #[serde(default, rename = "modelRewrites", alias = "model_rewrites")]
    pub model_rewrites: Option<Vec<ModelRewriteRule>>,
    /// Provider-level per-model service (speed) tier overrides (issue #414).
    /// When the upstream model matches `model` and the request does not
    /// already ask for a premium tier, the gateway writes `service_tier`
    /// before forwarding. Only OpenAI Chat / Responses targets honour it;
    /// an explicit Fast / Ultrafast choice from the CLI always wins.
    #[serde(default, rename = "modelServiceTiers", alias = "model_service_tiers")]
    pub model_service_tiers: Option<Vec<ModelServiceTierRule>>,
    /// Upstream model ids this provider declares in its `modelCatalog`.
    ///
    /// Populated from `settingsConfig.modelCatalog.models` so aggregate mode can
    /// tell whether a site actually offers the requested model. A bare model
    /// name only resolves to sites listed here; empty means the provider never
    /// declared a catalog, so it is excluded rather than guessed at. A site
    /// named by an explicit `<site><sep><model>` prefix is routed regardless,
    /// because the prefix is authoritative.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_models: Vec<String>,
}

/// One user-defined exact model rewrite rule: when the CLI requests `from`
/// (compared trim + case-insensitively after stripping the `[1M]` context
/// marker), the gateway forwards `to` to the upstream instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ModelRewriteRule {
    pub from: String,
    pub to: String,
}

/// One per-model service (speed) tier override: when the upstream model matches
/// `model` (exact, trim + case-insensitive after stripping the `[1M]` context
/// marker), the gateway forwards `service_tier` to the upstream.
///
/// `service_tier` is the wire value itself — `priority` (Codex's Fast id) or
/// `ultrafast` — so what the user picks is exactly what the upstream sees.
/// Rules whose tier is not a recognized premium tier are dropped at parse time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelServiceTierRule {
    pub model: String,
    /// Wire tier forwarded to the upstream (`priority` / `ultrafast`).
    #[serde(alias = "service_tier")]
    pub service_tier: String,
}

/// One request-header override operation, mirroring axonhub's flat
/// `OverrideOperation` shape (header subset only).
///
/// - `set`: replace `name` with `value`
/// - `delete`: drop `name`
/// - `rename`: move `from` values to `to`
/// - `copy`: duplicate `from` values into `to`
///
/// Only `set`/`delete`/`rename`/`copy` are honored at runtime; unknown ops
/// are silently skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct CustomHeaderOverride {
    pub op: String,
    /// Target header for `set`/`delete`.
    pub name: String,
    /// Replacement value for `set`.
    pub value: String,
    /// Source header for `rename`/`copy`.
    pub from: String,
    /// Destination header for `rename`/`copy`.
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayProviderProfileReference {
    pub tool: Option<String>,
    #[serde(rename = "profileId", alias = "profile_id")]
    pub profile_id: String,
    #[serde(rename = "endpointId", alias = "endpoint_id")]
    pub endpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct CodexChatReasoningMeta {
    #[serde(alias = "supportsThinking")]
    pub supports_thinking: Option<bool>,
    #[serde(alias = "supportsEffort")]
    pub supports_effort: Option<bool>,
    #[serde(alias = "thinkingParam")]
    pub thinking_param: Option<String>,
    #[serde(alias = "effortParam")]
    pub effort_param: Option<String>,
    #[serde(alias = "effortValueMode")]
    pub effort_value_mode: Option<String>,
    #[serde(alias = "outputFormat")]
    pub output_format: Option<String>,
}

impl Default for ProviderGatewayMeta {
    fn default() -> Self {
        Self {
            gateway_profile: None,
            provider_type: None,
            api_format: None,
            api_key_field: None,
            is_full_url: false,
            prompt_cache_key: None,
            reasoning_field: None,
            default_max_tokens: None,
            codex_chat_reasoning: None,
            preserve_reasoning_effort: None,
            image_input_policy: None,
            text_only_models: Vec::new(),
            image_capable_models: Vec::new(),
            allow_text_only_model_heuristic: false,
            cost_multiplier: "1.0".to_string(),
            pricing_model_source: "upstream".to_string(),
            fast_cost_multiplier: None,
            ultrafast_cost_multiplier: None,
            custom_headers: None,
            model_rewrites: None,
            model_service_tiers: None,
            declared_models: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProxyGatewaySettings {
    pub enabled_on_startup: bool,
    pub listen_host: String,
    pub listen_port: u16,
    pub port_auto_select: bool,
    /// WSL Direct CLI 访问本机网关时使用的宿主机可达地址，留空则继续使用 listen origin。
    pub wsl_host: String,
    pub enabled_cli_keys: Vec<GatewayCliKey>,
    /// Allow Codex Responses WebSocket upgrades. HTTP/SSE remains the default transport.
    pub codex_websocket_enabled: bool,
    /// Offer a Codex remote-compaction v2 turn to the upstream natively first.
    /// On by default: the `compaction_trigger` is forwarded unchanged, and the
    /// Gateway summarizes the turn itself only when the upstream answers with
    /// something other than its own `compaction` item. When off, every
    /// compaction turn is summarized by the Gateway without probing.
    pub codex_compaction_native_passthrough_enabled: bool,
    pub request_log_enabled: bool,
    pub request_log_level: String,
    pub metrics_enabled: bool,
    /// Whether locally-imported CLI session usage (requests that bypassed the
    /// gateway) participates in usage statistics and the request list.
    pub session_usage_enabled: bool,
    pub store_request_body: bool,
    pub store_headers: bool,
    pub store_response_body: bool,
    pub thinking_rectifier_enabled: bool,
    pub responses_encrypted_content_rectifier_enabled: bool,
    pub thinking_budget_rectifier_enabled: bool,
    pub cache_injection_enabled: bool,
    pub lossy_rejection_enabled: bool,
    pub streaming_first_byte_timeout_secs: u64,
    pub streaming_idle_timeout_secs: u64,
    pub non_streaming_timeout_secs: u64,
    pub log_retention_days: u32,
    pub log_max_dir_size_mb: u64,
    pub log_max_body_size_kb: u64,
    pub per_provider_retry_count: u32,
    pub max_retry_count: u32,
    pub retry_interval_secs: u64,
    /// Comma-separated HTTP status codes/ranges that may trigger same-provider
    /// retry or cross-provider failover. Example: `400,401,429,500-599`.
    pub retryable_status_codes: String,
    /// Skip candidate providers whose declared model catalog does not list the
    /// upstream model (issue #414). Off by default: a provider with no catalog
    /// declares nothing, and the whole feature would otherwise refuse to route.
    /// The runtime additionally requires at least one candidate to declare
    /// *something* before the filter engages, so a deployment where nobody
    /// maintains a catalog keeps its old behavior instead of failing outright.
    pub skip_undeclared_models: bool,
    pub app_configs: HashMap<GatewayCliKey, AppProxyConfig>,
    pub model_failure_score_threshold: i32,
    pub model_failure_window_seconds: u64,
    pub model_base_cooldown_seconds: u64,
    pub model_max_cooldown_seconds: u64,
    pub half_open_success_required: u32,
}

const DEFAULT_GATEWAY_LISTEN_PORT: u16 = 37123;
const GATEWAY_LISTEN_PORT_ENV: &str = "AI_TOOLBOX_GATEWAY_PORT";

#[cfg(test)]
fn configured_gateway_listen_port(value: Option<&str>) -> u16 {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u16>().ok())
        // Keep the same privileged-port boundary as the gateway's listen
        // validator. Invalid overrides fail closed to the production default.
        .filter(|port| *port >= 1024)
        .unwrap_or(DEFAULT_GATEWAY_LISTEN_PORT)
}

/// Valid `AI_TOOLBOX_GATEWAY_PORT` override, if the isolated launcher set one.
///
/// Unlike [`configured_gateway_listen_port`] this keeps "unset/invalid" distinct
/// from the production default: callers that force the port on top of a
/// persisted gateway record must not overwrite a user's stored port with 37123
/// just because the env value was malformed.
pub(crate) fn configured_gateway_listen_port_override() -> Option<u16> {
    let value = std::env::var(GATEWAY_LISTEN_PORT_ENV).ok()?;
    value
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port >= 1024)
}

fn default_gateway_listen_port() -> u16 {
    configured_gateway_listen_port_override().unwrap_or(DEFAULT_GATEWAY_LISTEN_PORT)
}

impl Default for ProxyGatewaySettings {
    fn default() -> Self {
        Self {
            enabled_on_startup: false,
            listen_host: "127.0.0.1".to_string(),
            listen_port: default_gateway_listen_port(),
            port_auto_select: false,
            wsl_host: String::new(),
            enabled_cli_keys: GatewayCliKey::supported_mvp(),
            codex_websocket_enabled: false,
            codex_compaction_native_passthrough_enabled: true,
            request_log_enabled: true,
            request_log_level: "summary".to_string(),
            metrics_enabled: true,
            session_usage_enabled: true,
            store_request_body: false,
            store_headers: false,
            store_response_body: false,
            thinking_rectifier_enabled: true,
            responses_encrypted_content_rectifier_enabled: true,
            thinking_budget_rectifier_enabled: true,
            cache_injection_enabled: false,
            lossy_rejection_enabled: false,
            streaming_first_byte_timeout_secs: 90,
            streaming_idle_timeout_secs: 180,
            non_streaming_timeout_secs: 600,
            log_retention_days: 7,
            log_max_dir_size_mb: 512,
            log_max_body_size_kb: 256,
            per_provider_retry_count: 0,
            max_retry_count: 8,
            retry_interval_secs: 1,
            retryable_status_codes: super::retryable_status::default_retryable_status_codes_compact(
            ),
            skip_undeclared_models: false,
            app_configs: HashMap::new(),
            model_failure_score_threshold: 5,
            model_failure_window_seconds: 300,
            model_base_cooldown_seconds: 120,
            model_max_cooldown_seconds: 1800,
            half_open_success_required: 2,
        }
    }
}

#[cfg(test)]
mod isolated_runtime_defaults_tests {
    use super::*;

    #[test]
    fn gateway_port_defaults_to_the_production_port() {
        assert_eq!(configured_gateway_listen_port(None), 37123);
        assert_eq!(configured_gateway_listen_port(Some("")), 37123);
        assert_eq!(configured_gateway_listen_port(Some("not-a-port")), 37123);
    }

    #[test]
    fn gateway_port_accepts_a_non_privileged_test_port() {
        assert_eq!(configured_gateway_listen_port(Some("38123")), 38123);
        assert_eq!(configured_gateway_listen_port(Some(" 38123 ")), 38123);
    }

    #[test]
    fn gateway_port_rejects_privileged_or_out_of_range_values() {
        assert_eq!(configured_gateway_listen_port(Some("80")), 37123);
        assert_eq!(configured_gateway_listen_port(Some("0")), 37123);
        assert_eq!(configured_gateway_listen_port(Some("70000")), 37123);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveAppProxyConfig {
    pub streaming_first_byte_timeout_secs: u64,
    pub streaming_idle_timeout_secs: u64,
    pub non_streaming_timeout_secs: u64,
    pub per_provider_retry_count: u32,
    pub max_retry_count: u32,
    pub retry_interval_secs: u64,
}

impl ProxyGatewaySettings {
    pub fn effective_app_config(&self, cli_key: GatewayCliKey) -> EffectiveAppProxyConfig {
        let app_config = self.app_configs.get(&cli_key);
        EffectiveAppProxyConfig {
            streaming_first_byte_timeout_secs: app_config
                .and_then(|config| config.streaming_first_byte_timeout_secs)
                .unwrap_or(self.streaming_first_byte_timeout_secs),
            streaming_idle_timeout_secs: app_config
                .and_then(|config| config.streaming_idle_timeout_secs)
                .unwrap_or(self.streaming_idle_timeout_secs),
            non_streaming_timeout_secs: app_config
                .and_then(|config| config.non_streaming_timeout_secs)
                .unwrap_or(self.non_streaming_timeout_secs),
            per_provider_retry_count: app_config
                .and_then(|config| config.per_provider_retry_count)
                .unwrap_or(self.per_provider_retry_count),
            max_retry_count: app_config
                .and_then(|config| config.max_retry_count)
                .unwrap_or(self.max_retry_count),
            retry_interval_secs: app_config
                .and_then(|config| config.retry_interval_secs)
                .unwrap_or(self.retry_interval_secs),
        }
    }

    pub fn default_cost_multiplier_for(&self, cli_key: GatewayCliKey) -> String {
        self.app_configs
            .get(&cli_key)
            .and_then(|config| config.cost_multiplier.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("1.0")
            .to_string()
    }

    pub fn default_pricing_model_source_for(&self, cli_key: GatewayCliKey) -> String {
        let source = self
            .app_configs
            .get(&cli_key)
            .and_then(|config| config.pricing_model_source.as_deref())
            .unwrap_or("upstream");
        normalize_pricing_model_source(source)
    }
}

/// Extra cost multiplier the provider charges for the canonical service tier a
/// request was billed under. Unset / blank / unknown tiers return `"1.0"`, so
/// the default records the plain token cost: OpenAI publishes no single ratio
/// (Fast is 2.5x on subscription allowance, 2x on purchased credits, ~1.6x on
/// some API token tables), so the channel decides.
pub fn service_tier_cost_multiplier(
    fast_cost_multiplier: Option<&str>,
    ultrafast_cost_multiplier: Option<&str>,
    service_tier: Option<&str>,
) -> String {
    let configured = match service_tier {
        Some(SERVICE_TIER_FAST) => fast_cost_multiplier,
        Some(SERVICE_TIER_ULTRAFAST) => ultrafast_cost_multiplier,
        _ => None,
    };
    configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("1.0")
        .to_string()
}

/// Canonical service tier ids kept in the request summary. Codex exposes Fast
/// mode as `service_tier = "priority"` (the legacy wire value) or `"fast"`;
/// Ultrafast is `"ultrafast"`. Only these are recognized — anything else
/// (`auto`, `default`, `flex`, vendor-specific values) is not a billed tier.
pub const SERVICE_TIER_FAST: &str = "fast";
pub const SERVICE_TIER_ULTRAFAST: &str = "ultrafast";

/// Normalize a wire `service_tier` value to a canonical tier id. Unknown and
/// non-premium values return `None` so callers fall back instead of guessing.
pub fn normalize_service_tier(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "priority" | "fast" => Some(SERVICE_TIER_FAST),
        "ultrafast" => Some(SERVICE_TIER_ULTRAFAST),
        _ => None,
    }
}

/// Canonical tier id → the value to actually put on the wire.
///
/// `normalize_service_tier` accepts the `fast` spelling because upstreams may
/// echo it, but OpenAI only accepts `priority` on the request side (a literal
/// `fast` is rejected as an unsupported tier). Codex itself normalizes
/// `fast` → `priority` before sending, so a user-configured tier is canonicalized
/// the same way instead of being forwarded verbatim.
pub fn service_tier_wire_value(value: &str) -> Option<&'static str> {
    match normalize_service_tier(value)? {
        SERVICE_TIER_FAST => Some("priority"),
        SERVICE_TIER_ULTRAFAST => Some("ultrafast"),
        _ => None,
    }
}

pub fn normalize_pricing_model_source(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "request" | "requested" => "requested".to_string(),
        _ => "upstream".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ModelPricing {
    pub model_id: String,
    pub display_name: String,
    pub input_cost_per_million: String,
    pub output_cost_per_million: String,
    pub cache_read_cost_per_million: String,
    pub cache_creation_cost_per_million: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProxyGatewayStatus {
    pub running: bool,
    pub base_url: Option<String>,
    pub listen_host: String,
    pub listen_port: Option<u16>,
    pub active_connections: u32,
    pub requests_per_minute: u64,
    pub requests_per_minute_by_cli: HashMap<GatewayCliKey, u64>,
    pub last_error: Option<String>,
}

impl ProxyGatewayStatus {
    pub fn stopped(settings: &ProxyGatewaySettings, last_error: Option<String>) -> Self {
        Self {
            running: false,
            base_url: None,
            listen_host: settings.listen_host.clone(),
            listen_port: None,
            active_connections: 0,
            requests_per_minute: 0,
            requests_per_minute_by_cli: HashMap::new(),
            last_error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayFailoverEvent {
    pub cli_key: GatewayCliKey,
    pub from_provider_id: String,
    pub from_provider_name: Option<String>,
    pub to_provider_id: String,
    pub to_provider_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProxyGatewayPortCheckInput {
    pub listen_host: String,
    pub listen_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProxyGatewayPortCheckResult {
    pub available: bool,
    pub listen_host: String,
    pub listen_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProxyGatewayHealthCheckResult {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConnectivityTestRequest {
    pub cli_key: GatewayCliKey,
    pub provider_id: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    pub model_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Send a tiny inline test image so the relayed upstream must actually read
    /// an image; lets users tell declared vision from working vision.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vision_probe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConnectivityTestResult {
    pub model_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_byte_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub request_url: String,
    pub request_headers: serde_json::Value,
    pub request_body: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body: Option<serde_json::Value>,
    /// Client-facing status the gateway answered with. Recorded separately from
    /// the response headers so a failure whose body was empty or truncated still
    /// shows why the test failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    /// Real upstream status when the gateway substituted a synthetic one
    /// (e.g. an empty 200 stream rewritten into a local 502).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_url: Option<String>,
    /// Vision probe outcome when one was requested: passed / failed / unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConnectivityTestResponse {
    pub results: Vec<GatewayConnectivityTestResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayCliTakeoverState {
    Direct,
    TakeoverApplied,
    GatewayStopped,
    OutdatedOrigin,
    Drifted,
    NoProxyProvider,
    RestoreUnavailable,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayCliStatusDot {
    Gray,
    Green,
    Orange,
    Red,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayManagedTarget {
    pub kind: String,
    pub path: String,
    pub existed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderPriorityEntry {
    pub provider_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayCliTakeoverStatus {
    pub cli_key: GatewayCliKey,
    pub state: GatewayCliTakeoverState,
    pub dot: GatewayCliStatusDot,
    pub can_takeover: bool,
    pub can_restore_direct: bool,
    pub gateway_origin: Option<String>,
    pub runtime_root: Option<String>,
    pub managed_targets: Vec<GatewayManagedTarget>,
    pub mode: Option<GatewayProxyMode>,
    pub primary_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<GatewayAggregateConfig>,
    pub provider_priorities: Vec<ProviderPriorityEntry>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProxyGatewayStopPreflight {
    pub allowed: bool,
    pub blocking_cli_takeovers: Vec<GatewayCliTakeoverStatus>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderModelHealthKey {
    pub cli_key: GatewayCliKey,
    pub provider_id: String,
    pub upstream_model_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderHealthKey {
    pub cli_key: GatewayCliKey,
    pub provider_id: String,
}

impl From<&ProviderModelHealthKey> for ProviderHealthKey {
    fn from(key: &ProviderModelHealthKey) -> Self {
        Self {
            cli_key: key.cli_key,
            provider_id: key.provider_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelHealthStateKind {
    Healthy,
    Degraded,
    CoolingDown,
    Probing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ModelHealthEntry {
    pub state: ModelHealthStateKind,
    pub failure_score: i32,
    pub consecutive_open_count: u32,
    pub half_open_success_count: u32,
    pub next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_failure_at: Option<DateTime<Utc>>,
    pub last_error_category: Option<String>,
}

impl Default for ModelHealthEntry {
    fn default() -> Self {
        Self {
            state: ModelHealthStateKind::Healthy,
            failure_score: 0,
            consecutive_open_count: 0,
            half_open_success_count: 0,
            next_retry_at: None,
            last_failure_at: None,
            last_error_category: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProxyGatewayRequestLogListInput {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestLogFilters {
    pub data_source: Option<String>,
    pub cli_key: Option<GatewayUsageTool>,
    pub provider_name: Option<String>,
    pub model: Option<String>,
    pub status_code: Option<u16>,
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
    /// When true, exclude GET/HEAD model-list requests from request log queries.
    pub exclude_model_list: Option<bool>,
    /// When true, surface only failed requests. A request counts as failed when
    /// its HTTP status is non-2xx/3xx, or its recorded stream outcome is
    /// `incomplete`, `failed`, or `canceled` (a 200 whose stream never delivered
    /// a terminal event to the client).
    #[serde(default)]
    pub only_failed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayPaginatedRequestLogs {
    pub data: Vec<GatewayRequestLogItem>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestLogItem {
    #[serde(default)]
    pub transport: GatewayRequestTransport,
    #[serde(default)]
    pub request_kind: GatewayRequestKind,
    /// Canonical kinds of native/hosted tools the upstream actually invoked
    /// (`web_search`, `mcp`, …). Empty means "nothing observed": legacy rows,
    /// imported session usage and locally rejected requests were never
    /// inspected, so this must not read as "no tool was used".
    #[serde(default)]
    pub native_tools_used: Vec<String>,
    #[serde(default)]
    pub stream_outcome: Option<GatewayStreamOutcome>,
    #[serde(default)]
    pub usage_metadata: Option<SessionUsageMetadata>,
    #[serde(default)]
    pub extra_tokens: u64,
    pub trace_id: String,
    pub data_source: String,
    pub cli_key: GatewayUsageTool,
    pub route_name: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub provider_id: String,
    pub provider_name: Option<String>,
    pub requested_model: Option<String>,
    pub upstream_model_id: String,
    pub reasoning_effort: Option<String>,
    /// Canonical billed service tier (`fast` / `ultrafast`) when the request ran
    /// on a premium OpenAI service tier, else `None`.
    #[serde(default)]
    pub service_tier: Option<String>,
    /// `native` / `fallback` / `simulated` on a Codex remote compaction v2 turn,
    /// else `None`.
    #[serde(default)]
    pub compaction_mode: Option<String>,
    pub status_code: u16,
    pub success: bool,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_tokens: u64,
    pub total_cost_usd: String,
    pub is_streaming: bool,
    pub first_token_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayUsageSummary {
    pub total_requests: u64,
    pub total_cost_usd: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub success_rate: f32,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayUsageSummaryByCli {
    pub cli_key: GatewayUsageTool,
    pub summary: GatewayUsageSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayUsageTrendPoint {
    pub date: String,
    pub request_count: u64,
    pub total_cost_usd: String,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayProviderStats {
    pub cli_key: GatewayUsageTool,
    pub provider_id: String,
    pub provider_name: Option<String>,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: String,
    pub success_rate: f32,
    pub avg_latency_ms: Option<u64>,
    /// Token-weighted input cache hit ratio (0..=1); None when input usage is absent.
    pub cache_hit_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayModelStats {
    pub cli_key: GatewayUsageTool,
    pub model: String,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: String,
    /// HTTP success rate over gateway (proxy) requests only; imported session
    /// rows carry placeholder status codes, so a model without proxy traffic
    /// reports None instead of a placeholder-derived rate.
    pub success_rate: Option<f32>,
    pub avg_latency_ms: Option<u64>,
    /// Token-weighted input cache hit ratio (0..=1); None when input usage is absent.
    pub cache_hit_rate: Option<f64>,
    /// Whether the pricing table can price this model id at all, decided by the
    /// same matcher the write path and the session cost reconciliation use.
    ///
    /// `false` means no cost can be derived from `model_pricing`, so native
    /// (session) usage for this model lands at `0`; it does **not** mean the
    /// model is free. `true` does not promise a non-zero amount either: rows
    /// already stored are only requoted by the session cost reconciliation, not
    /// by pricing CRUD.
    pub has_pricing: bool,
    /// The four per-million prices that actually priced this model, when the
    /// pricing table matched one. Mirrors what the write path used, so the row
    /// can be expanded without re-implementing the matcher in the frontend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
}

/// One project a local CLI worked in, aggregated across every CLI that touched
/// it (tokei's project footprint stays merged by path, not split per tool).
///
/// Only imported session usage can have a project: proxy traffic never records
/// a working directory, so it lands in the `""` bucket, which the UI labels as
/// "unknown project" instead of pretending it belongs somewhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayProjectStats {
    /// Absolute working directory; `""` for usage with no recorded project.
    pub project_path: String,
    /// Distinct local sessions that touched the project.
    pub session_count: u64,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: String,
    /// The model that used the most tokens in this project, for the row's
    /// second line. `None` when every row lacks a model name.
    pub top_model: Option<String>,
    /// CLI keys that reported usage in this project, sorted for a stable order.
    pub cli_keys: Vec<GatewayUsageTool>,
    /// Local date (`YYYY-MM-DD`) of the most recent recorded usage.
    pub last_active: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestLogSummary {
    #[serde(default)]
    pub transport: GatewayRequestTransport,
    #[serde(default)]
    pub request_kind: GatewayRequestKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<SessionUsageMetadata>,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub cli_key: Option<GatewayUsageTool>,
    pub route_name: String,
    pub method: String,
    pub path: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    #[serde(default)]
    pub provider_type: Option<String>,
    #[serde(default)]
    pub cost_multiplier: Option<String>,
    #[serde(default)]
    pub pricing_model_source: Option<String>,
    pub requested_model: Option<String>,
    pub upstream_model_id: Option<String>,
    /// Explicit effort from the final upstream attempt, independent of body storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Canonical billed service tier (`fast` / `ultrafast`), independent of body
    /// storage. Prefers the tier the upstream confirmed in its response and
    /// falls back to the tier requested in the final upstream body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// Effective extra multiplier the provider charges for that tier, resolved
    /// at request time so a later provider edit cannot rewrite history. `None`
    /// on rows with no premium tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_multiplier: Option<String>,
    /// Which side wrote the `compaction` item on a Codex remote compaction v2
    /// turn: `native` when the upstream answered with its own item, `fallback`
    /// when the trigger was offered first and refused, `simulated` when the
    /// target protocol could never have served it natively. `None` on every
    /// other turn. Recorded so a user can tell them apart without reading the
    /// wire — the item itself looks the same in all three cases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_mode: Option<String>,
    pub upstream_url: Option<String>,
    pub status_code: Option<u16>,
    /// Original HTTP status the upstream returned before the gateway rewrote a
    /// failure into its own synthetic code (e.g. a 200 SSE stream carrying an
    /// error envelope becomes 502). `None` when the gateway did not substitute
    /// the upstream status.
    #[serde(default)]
    pub upstream_status_code: Option<u16>,
    pub success: bool,
    pub error_category: Option<String>,
    pub error_message: Option<String>,
    /// How the streaming response actually ended for the client. Derived from
    /// the terminal-event verdict written during `write_streaming_body`, not
    /// from the HTTP status code, so mid-stream failures on an already-written
    /// 200 are no longer recorded as successes.
    #[serde(default)]
    pub stream_outcome: Option<GatewayStreamOutcome>,
    pub duration_ms: u64,
    pub attempt_count: u32,
    #[serde(default)]
    pub total_attempt_count: u32,
    pub failover: bool,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub request_body_bytes: u64,
    pub response_body_bytes: u64,
    #[serde(default)]
    pub is_streaming: bool,
    #[serde(default)]
    pub first_token_ms: Option<u64>,
    #[serde(default)]
    pub detail_file: Option<String>,
    #[serde(default)]
    pub detail_offset: Option<u64>,
}

/// How a streaming gateway response actually ended for the client.
///
/// Once the gateway has written `HTTP/1.1 200` + chunked headers, every later
/// failure happens inside `write_streaming_body`, where the status code can no
/// longer be changed. This verdict is derived from whether a protocol-level
/// terminal event was actually written to the client, so the request record
/// reflects stream reality instead of the (already-sent) 200.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GatewayStreamOutcome {
    /// Non-streaming response, or the outcome was not observed.
    #[default]
    NotStreaming,
    /// A terminal event was written to the client and the stream closed.
    Completed,
    /// Stream ended at EOF without a terminal event and without an error.
    Incomplete,
    /// Explicit stream error (idle timeout, upstream stream error, write error
    /// after the terminal event was already delivered is still `Completed`).
    Failed,
    /// Client disconnected before the terminal event was delivered.
    Canceled,
}

impl GatewayStreamOutcome {
    /// Stable lowercase key persisted in SQLite and reported in JSONL.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStreaming => "not_streaming",
            Self::Completed => "completed",
            Self::Incomplete => "incomplete",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// Parse the key produced by [`as_str`]. Unknown / corrupt / legacy values
    /// return `None` so the caller can fall back to HTTP-status-based success
    /// derivation. (Previously unknown values mapped to `NotStreaming`, whose
    /// `is_success()` is `true` — so a corrupt `stream_outcome` on a 500 row was
    /// recorded as success, contradicting the status code.)
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim() {
            "completed" => Some(Self::Completed),
            "incomplete" => Some(Self::Incomplete),
            "failed" => Some(Self::Failed),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }

    /// Whether this outcome should be recorded as a successful request.
    pub fn is_success(self) -> bool {
        matches!(self, Self::Completed | Self::NotStreaming)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayProviderAttempt {
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub upstream_model_id: Option<String>,
    pub status_code: Option<u16>,
    pub success: bool,
    pub error_category: Option<String>,
    pub error_message: Option<String>,
    pub attempt_count: u32,
    pub total_attempt_count: u32,
}

/// One native/hosted tool seen on a request, recorded for the detail dialog.
///
/// Persisted only in the JSONL detail; see
/// `runtime/observations.rs` for why this never enters `proxy_request_logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayNativeToolRecord {
    /// Canonical kind shared by every protocol (`web_search`, `mcp`, …).
    pub kind: String,
    /// The client's request declared this tool.
    pub declared: bool,
    /// The tool declaration survived into the body sent upstream.
    pub forwarded: bool,
    /// The upstream's reply actually invoked this tool.
    pub used: bool,
}

/// One gateway-side rewrite applied to a request on its way upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestRewrite {
    /// Stable machine key (`prompt_cache_key`, `lossy_conversion`, …).
    pub kind: String,
    /// Human-readable specifics, already truncated by the runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestLogDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<super::privacy::PrivacyDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket: Option<GatewayWebSocketMetadata>,
    /// Native/hosted tools this request declared, forwarded, or used. Detail
    /// only (JSONL), never part of the SQLite summary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_tools: Vec<GatewayNativeToolRecord>,
    /// Gateway rewrites applied to the outbound request. Detail only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rewrites: Vec<GatewayRequestRewrite>,
    #[serde(flatten)]
    pub summary: GatewayRequestLogSummary,
    pub request_headers: Option<BTreeMap<String, String>>,
    pub request_body: Option<String>,
    #[serde(default)]
    pub upstream_request_body: Option<String>,
    pub response_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub upstream_response_body: Option<String>,
    pub response_body: Option<String>,
    #[serde(default)]
    pub provider_attempts: Vec<GatewayProviderAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestLogRecord {
    pub schema_version: u32,
    #[serde(flatten)]
    pub detail: GatewayRequestLogDetail,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRequestTransport {
    #[default]
    Http,
    Websocket,
}

impl GatewayRequestTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Websocket => "websocket",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "websocket" => Self::Websocket,
            _ => Self::Http,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRequestKind {
    #[default]
    Request,
    WebsocketHandshake,
    WebsocketWarmup,
}

impl GatewayRequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::WebsocketHandshake => "websocket_handshake",
            Self::WebsocketWarmup => "websocket_warmup",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "websocket_handshake" => Self::WebsocketHandshake,
            "websocket_warmup" => Self::WebsocketWarmup,
            _ => Self::Request,
        }
    }
}

/// Connection metadata belongs in JSONL detail, never in the compact usage store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayWebSocketMetadata {
    pub connection_id: String,
    pub response_id: Option<String>,
    pub stream_id: Option<String>,
    pub previous_response_id: Option<String>,
    pub event_type: Option<String>,
    pub handshake_status: u16,
    pub upstream_handshake_status: Option<u16>,
    pub error_status: Option<u16>,
    pub fallback_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handshake_attempts: Vec<GatewayProviderAttempt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayModelHealthScope {
    Model,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayModelHealthItem {
    pub scope: GatewayModelHealthScope,
    pub cli_key: GatewayCliKey,
    pub provider_id: String,
    #[serde(default)]
    pub provider_name: Option<String>,
    pub upstream_model_id: Option<String>,
    pub state: ModelHealthStateKind,
    pub failure_score: i32,
    pub consecutive_open_count: u32,
    pub half_open_success_count: u32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub last_error_category: Option<String>,
}

impl Default for GatewayCliKey {
    fn default() -> Self {
        Self::Claude
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewaySessionImportCli {
    All,
    Claude,
    ClaudeDesktop,
    Codex,
    Grok,
    Kimi,
    Gemini,
    Antigravity,
    #[serde(rename = "opencode", alias = "open_code")]
    OpenCode,
    Pi,
    OhMyPi,
    OmoNative,
    Dsh,
    Hermes,
    #[serde(rename = "openclaw", alias = "open_claw")]
    OpenClaw,
    KimiCli,
    /// ZCode's CLI session database (`cli/db/db.sqlite`). Usage-only, like the
    /// other native-transcript tools: the gateway never proxies it.
    Zcode,
    Mimocode,
    PrimeAgent,
    Devin,
    Musecode,
    Cmdcode,
    Qwencode,
    Workbuddy,
    WorkbuddyAi,
    Codebuddy,
    Minimax,
    QoderIde,
    QoderIdeCn,
    #[serde(rename = "qodercli")]
    QoderCli,
    #[serde(rename = "qodercli_cn")]
    QoderCliCn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct GatewaySessionUsageImportInput {
    pub cli_key: GatewaySessionImportCli,
}

impl Default for GatewaySessionUsageImportInput {
    fn default() -> Self {
        Self {
            cli_key: GatewaySessionImportCli::All,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DataSourceBreakdownInput {
    pub cli_key: Option<GatewayUsageTool>,
    pub start_unix_secs: Option<i64>,
    pub end_unix_secs: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DataSourceBreakdownItem {
    pub data_source: String,
    pub request_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct GatewaySessionUsageImportResult {
    pub scanned_files: u64,
    pub parsed_records: u64,
    pub inserted_records: u64,
    pub updated_records: u64,
    pub skipped_records: u64,
    pub failed_files: u64,
}

impl GatewaySessionUsageImportResult {
    pub fn merge(&mut self, other: Self) {
        self.scanned_files = self.scanned_files.saturating_add(other.scanned_files);
        self.parsed_records = self.parsed_records.saturating_add(other.parsed_records);
        self.inserted_records = self.inserted_records.saturating_add(other.inserted_records);
        self.updated_records = self.updated_records.saturating_add(other.updated_records);
        self.skipped_records = self.skipped_records.saturating_add(other.skipped_records);
        self.failed_files = self.failed_files.saturating_add(other.failed_files);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct GatewayUsageRecordedEvent {
    pub cli_key: Option<GatewayUsageTool>,
    pub trace_id: Option<String>,
    pub data_source: String,
    pub inserted_records: u64,
}

impl Default for GatewayUsageRecordedEvent {
    fn default() -> Self {
        Self {
            cli_key: None,
            trace_id: None,
            data_source: "proxy".to_string(),
            inserted_records: 0,
        }
    }
}
