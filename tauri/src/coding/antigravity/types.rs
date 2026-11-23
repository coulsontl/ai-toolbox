use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityProviderRecord {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityProvider {
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

impl From<AntigravityProviderRecord> for AntigravityProvider {
    fn from(record: AntigravityProviderRecord) -> Self {
        AntigravityProvider {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityProviderContent {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOfficialModel {
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
pub struct AntigravityOfficialModelsResponse {
    pub models: Vec<AntigravityOfficialModel>,
    pub total: usize,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityOfficialAccountRecord {
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
    pub project_id: Option<String>,
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
    pub limit_5h_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_reset_at: Option<i64>,
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
pub struct AntigravityOfficialAccount {
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
    pub project_id: Option<String>,
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
    pub limit_5h_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_reset_at: Option<i64>,
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

impl From<AntigravityOfficialAccountRecord> for AntigravityOfficialAccount {
    fn from(record: AntigravityOfficialAccountRecord) -> Self {
        AntigravityOfficialAccount {
            id: record.id,
            provider_id: record.provider_id,
            name: record.name,
            kind: record.kind,
            email: record.email,
            auth_snapshot: Some(record.auth_snapshot),
            auth_mode: record.auth_mode,
            account_id: record.account_id,
            project_id: record.project_id,
            plan_type: record.plan_type,
            last_refresh: record.last_refresh,
            token_expires_at: None,
            access_token_preview: None,
            refresh_token_preview: None,
            limit_short_label: record.limit_short_label,
            limit_5h_text: record.limit_5h_text,
            limit_weekly_text: record.limit_weekly_text,
            limit_5h_reset_at: record.limit_5h_reset_at,
            limit_weekly_reset_at: record.limit_weekly_reset_at,
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
pub struct AntigravityOfficialAccountContent {
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
    pub project_id: Option<String>,
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
    pub limit_5h_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_weekly_reset_at: Option<i64>,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOfficialAccountTokenCopyInput {
    pub provider_id: String,
    pub account_id: String,
    pub token_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityCommonConfigRecord {
    pub id: String,
    pub config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityCommonConfig {
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
pub struct AntigravityCommonConfigInput {
    pub config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravitySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityEnvVarStatus {
    pub key: String,
    pub expected_value: String,
    pub is_set_in_current_process: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityEnvSetupInfo {
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub provider_category: String,
    pub requires_env_vars: bool,
    pub export_bash: String,
    pub export_powershell: String,
    pub export_cmd: String,
    pub vars: Vec<AntigravityEnvVarStatus>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityPromptConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityPromptConfig {
    pub id: String,
    pub name: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_file: Option<String>,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityPromptConfigContent {
    pub name: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_file: Option<String>,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityLocalRuleFile {
    pub relative_path: String,
    pub full_path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub content: String,
    pub is_managed_target: bool,
}
