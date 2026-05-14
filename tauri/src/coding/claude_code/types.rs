use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;

fn default_json_object_string() -> String {
    "{}".to_string()
}

// ============================================================================
// ClaudeCode Provider Types
// ============================================================================

/// ClaudeCodeProvider - Database record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProviderRecord {
    pub id: Thing,
    pub name: String,
    pub category: String,
    pub settings_config: String,
    #[serde(default = "default_json_object_string")]
    pub extra_settings_config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// ClaudeCodeProvider - API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeProvider {
    pub id: String,
    pub name: String,
    pub category: String,
    pub settings_config: String,
    #[serde(default = "default_json_object_string")]
    pub extra_settings_config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ClaudeCodeProviderRecord> for ClaudeCodeProvider {
    fn from(record: ClaudeCodeProviderRecord) -> Self {
        ClaudeCodeProvider {
            id: record.id.id.to_string(),
            name: record.name,
            category: record.category,
            settings_config: record.settings_config,
            extra_settings_config: record.extra_settings_config,
            source_provider_id: record.source_provider_id,
            website_url: record.website_url,
            notes: record.notes,
            icon: record.icon,
            icon_color: record.icon_color,
            sort_index: record.sort_index,
            is_applied: record.is_applied,
            is_disabled: record.is_disabled,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// ClaudeCodeProvider - Content for create/update (Database storage)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProviderContent {
    pub name: String,
    pub category: String,
    pub settings_config: String,
    #[serde(default = "default_json_object_string")]
    pub extra_settings_config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// ClaudeCodeProvider - Input from frontend (for create operation)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeProviderInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub category: String,
    pub settings_config: String,
    #[serde(default)]
    pub extra_settings_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
}

// ============================================================================
// ClaudeCode Common Config Types
// ============================================================================

/// ClaudeCommonConfig - Database record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCommonConfigRecord {
    pub id: Thing,
    pub config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// ClaudeCommonConfig - API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCommonConfig {
    pub config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPathInfo {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCommonConfigInput {
    pub config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

/// Input for saving local config (provider and/or common)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLocalConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ClaudeCodeProviderInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub common_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

/// Claude settings.json structure (for reading/writing config file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<serde_json::Value>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

// ============================================================================
// Claude Plugin Integration Types
// ============================================================================

/// ClaudePluginStatus - API response for plugin integration status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePluginStatus {
    /// Whether primaryApiKey = "any" is set (third-party providers enabled)
    pub enabled: bool,
    /// Whether ~/.claude/config.json exists
    pub has_config_file: bool,
}

// ============================================================================
// Claude Prompt Config Types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePromptConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePromptConfig {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudePromptConfigContent {
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

// ============================================================================
// Claude All API Hub Import Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAllApiHubProvider {
    pub provider_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub requires_browser_open: bool,
    pub is_disabled: bool,
    pub has_api_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_cny: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_type: Option<String>,
    pub account_label: String,
    pub source_profile_name: String,
    pub source_extension_id: String,
    pub provider_config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAllApiHubProvidersResult {
    pub found: bool,
    pub profiles: Vec<crate::coding::all_api_hub::AllApiHubProfileInfo>,
    pub providers: Vec<ClaudeAllApiHubProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveClaudeAllApiHubProvidersRequest {
    pub provider_ids: Vec<String>,
}
