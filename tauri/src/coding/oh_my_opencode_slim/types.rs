use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Config path info
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPathInfo {
    pub path: String,
    pub source: String,
}

/// Fallback config for oh-my-opencode-slim
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyOpenCodeSlimFallbackConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_on_empty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chains: Option<Value>,
    #[serde(flatten)]
    pub other_fields: std::collections::BTreeMap<String, Value>,
}

/// Input type for creating/updating Agents Profile (简化版)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyOpenCodeSlimAgentsProfileInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>, // Optional - will be generated if not provided
    pub name: String,
    pub agents: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub council: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<OhMyOpenCodeSlimFallbackConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
}

/// Oh My OpenCode Slim Agents Profile stored in database (简化版)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyOpenCodeSlimAgentsProfile {
    pub id: String,
    pub name: String,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub agents: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub council: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<OhMyOpenCodeSlimFallbackConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>, // For manual ordering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Oh My OpenCode Slim Agents Profile content for database storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OhMyOpenCodeSlimAgentsProfileContent {
    pub name: String,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub agents: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub council: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<OhMyOpenCodeSlimFallbackConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>, // For manual ordering
    pub created_at: String,
    pub updated_at: String,
}

/// Input type for Global Config
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyOpenCodeSlimGlobalConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sisyphus_agent: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_agents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_mcps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_hooks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub council: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
}

/// Oh My OpenCode Slim Global Config stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyOpenCodeSlimGlobalConfig {
    pub id: String, // 固定为 "global"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sisyphus_agent: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_agents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_mcps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_hooks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub council: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Oh My OpenCode Slim Global Config content for database storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OhMyOpenCodeSlimGlobalConfigContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sisyphus_agent: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_agents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_mcps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_hooks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub council: Option<Value>, // JSON, no specific structure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    pub updated_at: String,
}

/// @deprecated 使用 OhMyOpenCodeSlimAgentsProfileInput 代替
pub type OhMyOpenCodeSlimConfigInput = OhMyOpenCodeSlimAgentsProfileInput;

/// @deprecated 使用 OhMyOpenCodeSlimAgentsProfile 代替
pub type OhMyOpenCodeSlimConfig = OhMyOpenCodeSlimAgentsProfile;

/// @deprecated 使用 OhMyOpenCodeSlimAgentsProfileContent 代替
pub type OhMyOpenCodeSlimConfigContent = OhMyOpenCodeSlimAgentsProfileContent;

/// Input type for saving local config (both Agents Profile and Global Config)
/// Used when saving __local__ temporary config to database
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyOpenCodeSlimLocalConfigInput {
    /// Agents Profile config (optional, will be loaded from local file if not provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<OhMyOpenCodeSlimConfigInput>,
    /// Global Config (optional, will be loaded from local file if not provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_config: Option<OhMyOpenCodeSlimGlobalConfigInput>,
}
