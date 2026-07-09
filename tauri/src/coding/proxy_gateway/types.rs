use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayCliKey {
    Claude,
    Codex,
    Gemini,
    OpenCode,
}

impl GatewayCliKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
        }
    }

    pub fn supported_mvp() -> Vec<Self> {
        vec![Self::Claude, Self::Codex, Self::Gemini]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayProxyMode {
    Single,
    Failover,
}

impl GatewayProxyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Failover => "failover",
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
    pub image_input_policy: Option<String>,
    pub text_only_models: Vec<String>,
    pub image_capable_models: Vec<String>,
    pub allow_text_only_model_heuristic: bool,
    pub cost_multiplier: String,
    pub pricing_model_source: String,
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
            image_input_policy: None,
            text_only_models: Vec::new(),
            image_capable_models: Vec::new(),
            allow_text_only_model_heuristic: false,
            cost_multiplier: "1.0".to_string(),
            pricing_model_source: "upstream".to_string(),
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
    pub request_log_enabled: bool,
    pub request_log_level: String,
    pub metrics_enabled: bool,
    pub store_request_body: bool,
    pub store_headers: bool,
    pub store_response_body: bool,
    pub thinking_rectifier_enabled: bool,
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
    pub app_configs: HashMap<GatewayCliKey, AppProxyConfig>,
    pub model_failure_score_threshold: i32,
    pub model_failure_window_seconds: u64,
    pub model_base_cooldown_seconds: u64,
    pub model_max_cooldown_seconds: u64,
    pub half_open_success_required: u32,
}

impl Default for ProxyGatewaySettings {
    fn default() -> Self {
        Self {
            enabled_on_startup: false,
            listen_host: "127.0.0.1".to_string(),
            listen_port: 37123,
            port_auto_select: false,
            wsl_host: String::new(),
            enabled_cli_keys: GatewayCliKey::supported_mvp(),
            request_log_enabled: true,
            request_log_level: "summary".to_string(),
            metrics_enabled: true,
            store_request_body: false,
            store_headers: false,
            store_response_body: false,
            thinking_rectifier_enabled: true,
            thinking_budget_rectifier_enabled: true,
            cache_injection_enabled: false,
            lossy_rejection_enabled: false,
            streaming_first_byte_timeout_secs: 60,
            streaming_idle_timeout_secs: 120,
            non_streaming_timeout_secs: 600,
            log_retention_days: 7,
            log_max_dir_size_mb: 512,
            log_max_body_size_kb: 256,
            per_provider_retry_count: 0,
            max_retry_count: 8,
            retry_interval_secs: 1,
            app_configs: HashMap::new(),
            model_failure_score_threshold: 5,
            model_failure_window_seconds: 300,
            model_base_cooldown_seconds: 120,
            model_max_cooldown_seconds: 1800,
            half_open_success_required: 2,
        }
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
    pub cli_key: Option<GatewayCliKey>,
    pub provider_name: Option<String>,
    pub model: Option<String>,
    pub status_code: Option<u16>,
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
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
    pub trace_id: String,
    pub cli_key: GatewayCliKey,
    pub route_name: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub provider_id: String,
    pub provider_name: Option<String>,
    pub requested_model: Option<String>,
    pub upstream_model_id: String,
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
    pub cli_key: GatewayCliKey,
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
    pub cli_key: GatewayCliKey,
    pub provider_id: String,
    pub provider_name: Option<String>,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: String,
    pub success_rate: f32,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayModelStats {
    pub cli_key: GatewayCliKey,
    pub model: String,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: String,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestLogSummary {
    pub trace_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub cli_key: Option<GatewayCliKey>,
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
    pub upstream_url: Option<String>,
    pub status_code: Option<u16>,
    pub success: bool,
    pub error_category: Option<String>,
    pub error_message: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GatewayRequestLogDetail {
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
    Codex,
    Gemini,
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
    pub cli_key: Option<GatewayCliKey>,
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
    pub skipped_records: u64,
}

impl GatewaySessionUsageImportResult {
    pub fn merge(&mut self, other: Self) {
        self.scanned_files = self.scanned_files.saturating_add(other.scanned_files);
        self.parsed_records = self.parsed_records.saturating_add(other.parsed_records);
        self.inserted_records = self.inserted_records.saturating_add(other.inserted_records);
        self.skipped_records = self.skipped_records.saturating_add(other.skipped_records);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct GatewayUsageRecordedEvent {
    pub cli_key: Option<GatewayCliKey>,
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
