use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiProvider {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiProviderContent {
    pub name: String,
    pub category: String,
    pub settings_config: String,
    pub source_provider_id: Option<String>,
    pub website_url: Option<String>,
    pub notes: Option<String>,
    pub icon: Option<String>,
    pub icon_color: Option<String>,
    pub sort_index: Option<i32>,
    pub meta: Option<Value>,
    pub is_applied: bool,
    pub is_disabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub category: String,
    pub settings_config: String,
    pub source_provider_id: Option<String>,
    pub website_url: Option<String>,
    pub notes: Option<String>,
    pub icon: Option<String>,
    pub icon_color: Option<String>,
    pub sort_index: Option<i32>,
    pub meta: Option<Value>,
    pub is_disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCommonConfig {
    pub config: String,
    pub root_dir: Option<String>,
    /// Where the official-account card sits in the provider list, counted as
    /// the number of provider cards above it. UI state, not Kimi state — it
    /// lives here because this record is the module's only singleton row.
    pub official_account_index: Option<i64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCommonConfigInput {
    pub config: String,
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiPathInfo {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiSettings {
    pub config: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiLocalConfigInput {
    pub provider: Option<KimiProviderInput>,
    pub common_config: Option<String>,
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiPromptConfigInput {
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiPromptConfig {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    pub sort_index: Option<i32>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiPromptConfigContent {
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiPlugin {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiOfficialAccount {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub kind: String,
    pub email: Option<String>,
    pub subject: Option<String>,
    #[serde(skip_serializing)]
    pub auth_snapshot: Option<String>,
    pub token_endpoint: Option<String>,
    pub expires_at: Option<i64>,
    pub last_refresh: Option<String>,
    pub last_error: Option<String>,
    pub plan_type: Option<String>,
    pub limit_weekly_text: Option<String>,
    pub limit_monthly_text: Option<String>,
    pub limit_weekly_reset_at: Option<i64>,
    pub limit_monthly_reset_at: Option<i64>,
    pub last_limits_fetched_at: Option<String>,
    pub is_applied: bool,
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}
