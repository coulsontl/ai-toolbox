use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;

// ============================================================================
// ClaudeCode Provider Types
// ============================================================================

/// ClaudeCodeProvider - Database record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProviderRecord {
    pub id: Thing,
    pub provider_id: String,
    pub name: String,
    pub category: String,
    pub settings_config: String,
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
    pub is_current: bool,
    pub is_applied: bool,
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
    pub is_current: bool,
    pub is_applied: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ClaudeCodeProviderRecord> for ClaudeCodeProvider {
    fn from(record: ClaudeCodeProviderRecord) -> Self {
        ClaudeCodeProvider {
            id: record.provider_id,
            name: record.name,
            category: record.category,
            settings_config: record.settings_config,
            source_provider_id: record.source_provider_id,
            website_url: record.website_url,
            notes: record.notes,
            icon: record.icon,
            icon_color: record.icon_color,
            sort_index: record.sort_index,
            is_current: record.is_current,
            is_applied: record.is_applied,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// ClaudeCodeProvider - Content for create/update (Database storage)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeProviderContent {
    pub provider_id: String,
    pub name: String,
    pub category: String,
    pub settings_config: String,
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
    pub is_current: bool,
    pub is_applied: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// ClaudeCodeProvider - Input from frontend (for create operation)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeProviderInput {
    pub id: String,
    pub name: String,
    pub category: String,
    pub settings_config: String,
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
    pub updated_at: Option<String>,
}

/// ClaudeCommonConfig - API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCommonConfig {
    pub config: String,
    pub updated_at: String,
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
