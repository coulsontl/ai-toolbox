use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Codex Provider Types
// ============================================================================

/// CodexProvider - database record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexProviderRecord {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// CodexProvider - API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProvider {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<CodexProviderRecord> for CodexProvider {
    fn from(record: CodexProviderRecord) -> Self {
        CodexProvider {
            id: record.id,
            name: record.name,
            category: record.category,
            settings_config: record.settings_config,
            source_provider_id: record.source_provider_id,
            website_url: record.website_url,
            notes: record.notes,
            icon: record.icon,
            icon_color: record.icon_color,
            sort_index: record.sort_index,
            meta: record.meta,
            is_applied: record.is_applied,
            is_disabled: record.is_disabled,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// CodexProvider - Content for create/update (Database storage)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexProviderContent {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// CodexProvider - Input from frontend (for create operation)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProviderInput {
    #[serde(default)]
    pub id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOfficialModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOfficialModelsResponse {
    pub models: Vec<CodexOfficialModel>,
    pub total: usize,
    pub source: String,
    pub tier: String,
}

// ============================================================================
// Codex Common Config Types
// ============================================================================

/// CodexCommonConfig - database record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCommonConfigRecord {
    pub id: String,
    pub config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// CodexCommonConfig - API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCommonConfig {
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
pub struct CodexCommonConfigInput {
    pub config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

/// Input for saving local config (provider and/or common)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<CodexProviderInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub common_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

/// Codex settings structure (for reading/writing config files)
/// auth.json + config.toml combined
// Frontend-facing wire type: the web side consumes camelCase keys, matching
// the other command response types in this module. `auth`/`config` are single
// words so this only renames `model_catalog`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// Raw text of the model catalog file for the read-only config preview:
    /// the file named by config.toml's `model_catalog_json` pointer, or —
    /// when the pointer is absent — the AI Toolbox-managed catalog file as a
    /// leftover fallback. `None` when neither is readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_catalog: Option<String>,
    /// Whether config.toml's `model_catalog_json` pointer is set (Codex
    /// actually reads the catalog). Present whenever the preview tab is
    /// rendered (catalog content shown or a dangling pointer exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_catalog_active: Option<bool>,
}

// ============================================================================
// Codex Official Account Types
// ============================================================================

/// Additional Codex credits, independent of subscription windows and reset cards.
/// Optional flags distinguish an omitted/invalid field from an explicit false.
///
/// This is the IPC wire shape (camelCase, read by the web layer). The JSONB
/// column uses snake_case like every other field of the row, so the database
/// representation is mapped by hand in `adapter` (`credits_from_db_value` /
/// `credits_to_db_value`) rather than by this struct's serde attributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCredits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_credits: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlimited: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexOfficialAccountRecord {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub auth_snapshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_short_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_5h_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_monthly_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_5h_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_monthly_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_credits_available: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<CodexCredits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_limits_fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub is_applied: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOfficialAccount {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing, skip_deserializing, default)]
    pub auth_snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_short_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_5h_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_monthly_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_5h_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_monthly_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_credits_available: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<CodexCredits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_limits_fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub is_applied: bool,
    pub is_virtual: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<CodexOfficialAccountRecord> for CodexOfficialAccount {
    fn from(record: CodexOfficialAccountRecord) -> Self {
        CodexOfficialAccount {
            id: record.id,
            provider_id: record.provider_id,
            name: record.name,
            kind: record.kind,
            email: record.email,
            auth_snapshot: Some(record.auth_snapshot),
            auth_mode: record.auth_mode,
            account_id: record.account_id,
            plan_type: record.plan_type,
            last_refresh: record.last_refresh,
            token_expires_at: None,
            access_token_preview: None,
            refresh_token_preview: None,
            limit_short_label: record.limit_short_label,
            limit_5h_text: record.limit_5h_text,
            limit_weekly_text: record.limit_weekly_text,
            limit_monthly_text: record.limit_monthly_text,
            limit_5h_reset_at: record.limit_5h_reset_at,
            limit_weekly_reset_at: record.limit_weekly_reset_at,
            limit_monthly_reset_at: record.limit_monthly_reset_at,
            reset_credits_available: record.reset_credits_available,
            credits: record.credits,
            last_limits_fetched_at: record.last_limits_fetched_at,
            last_error: record.last_error,
            sort_index: record.sort_index,
            is_applied: record.is_applied,
            is_virtual: false,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexOfficialAccountContent {
    pub provider_id: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub auth_snapshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_short_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_5h_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_monthly_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_5h_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_monthly_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_credits_available: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<CodexCredits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_limits_fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub is_applied: bool,
    pub created_at: String,
    pub updated_at: String,
}

// ============================================================================
// Codex Prompt Config Types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPromptConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPromptConfig {
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
pub struct CodexPromptConfigContent {
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

// ============================================================================
// Codex All API Hub Import Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAllApiHubProvider {
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
pub struct CodexAllApiHubProvidersResult {
    pub found: bool,
    pub profiles: Vec<crate::coding::all_api_hub::AllApiHubProfileInfo>,
    pub providers: Vec<CodexAllApiHubProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveCodexAllApiHubProvidersRequest {
    pub provider_ids: Vec<String>,
}
