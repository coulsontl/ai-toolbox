use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiPathInfo {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiSettingsConfigRecord {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiSettingsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiSettingsConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiPromptConfigRecord {
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
#[serde(rename_all = "camelCase")]
pub struct PiPromptConfig {
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
pub struct PiPromptConfigContent {
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiPromptConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiBuiltinProvider {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiProviderSource {
    OfficialBuiltin,
    AuthJson,
    ModelsJson,
    SettingsJson,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiProviderCategory {
    Subscription,
    ApiKey,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiCredentialKind {
    ApiKey,
    Oauth,
    EnvPossible,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiProviderWarning {
    MissingProvider,
    MissingModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiDefaultSelection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiRuntimeProviderView {
    pub provider_key: String,
    pub display_name: String,
    pub sources: Vec<PiProviderSource>,
    pub categories: Vec<PiProviderCategory>,
    pub credential_kind: PiCredentialKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_provider: Option<Value>,
    pub runtime_files: Vec<String>,
    pub is_builtin: bool,
    pub is_override: bool,
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<PiProviderWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiRuntimeConfig {
    pub root_path_info: PiPathInfo,
    pub settings_path: String,
    pub auth_path: String,
    pub models_path: String,
    pub prompt_path: String,
    pub settings: Value,
    pub auth: Value,
    pub models: Value,
    pub other_settings: Value,
    pub model_settings: PiDefaultSelection,
    pub providers: Vec<PiRuntimeProviderView>,
    pub builtin_providers: Vec<PiBuiltinProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiModelSettingsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiAuthProviderInput {
    pub provider_key: String,
    pub credential: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiModelsProviderInput {
    pub provider_key: String,
    pub provider: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiDeleteScope {
    Credential,
    ProviderConfig,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiExtensionScope {
    User,
    Project,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiExtensionKind {
    Package,
    LocalFile,
    LocalDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiExtensionSummary {
    pub id: String,
    pub source: String,
    pub scope: PiExtensionScope,
    pub kind: PiExtensionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub built_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiExtensionListResult {
    pub extensions_path: String,
    pub packages_path: String,
    pub extensions: Vec<PiExtensionSummary>,
    pub raw: String,
    /// Resolved host-side `pi` binary path (or WSL invocation label) used for extension CLI ops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    /// Best-effort `pi --version` stdout for the resolved CLI; omitted when probing fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiExtensionInstallInput {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiExtensionActionInput {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<PiExtensionScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<PiExtensionKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiExtensionCommandResult {
    pub command: String,
    pub output: String,
}
