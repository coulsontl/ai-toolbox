use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpPathInfo {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpSettingsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpSettingsConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpPromptConfig {
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
pub struct OmpPromptConfigContent {
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
pub struct OmpPromptConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpBuiltinProvider {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmpProviderSource {
    OfficialBuiltin,
    ModelsYml,
    SettingsYml,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmpProviderCategory {
    Subscription,
    ApiKey,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmpCredentialKind {
    ApiKey,
    Oauth,
    EnvPossible,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmpProviderWarning {
    MissingProvider,
    MissingModel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmpOauthStatus {
    Stored,
    Missing,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpRuntimeProviderView {
    pub provider_key: String,
    pub display_name: String,
    pub sources: Vec<OmpProviderSource>,
    pub categories: Vec<OmpProviderCategory>,
    pub credential_kind: OmpCredentialKind,
    /// Metadata only: stored credentials are not proof of a valid session.
    pub oauth_status: Option<OmpOauthStatus>,
    #[serde(default)]
    pub runtime_models: Vec<Value>,
    pub runtime_catalog_error: Option<String>,
    pub login_command: Option<String>,
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
    pub warnings: Vec<OmpProviderWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpDefaultSelection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpRuntimeConfig {
    pub root_path_info: OmpPathInfo,
    pub config_path: String,
    pub models_path: String,
    pub mcp_path: String,
    pub prompt_path: String,
    pub settings: Value,
    pub models: Value,
    pub other_settings: Value,
    pub model_settings: OmpDefaultSelection,
    pub providers: Vec<OmpRuntimeProviderView>,
    pub builtin_providers: Vec<OmpBuiltinProvider>,
    /// Raw `config.yml` / `config.yaml` file content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_content: Option<String>,
    /// Raw `models.yml` file content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_content: Option<String>,
    /// Raw `mcp.json` file content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_content: Option<String>,
    /// Raw prompt file content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpModelSettingsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<String>,
    /// Explicitly remove `defaultThinkingLevel` from `config.yml`.
    /// This is separate from `default_thinking_level: ""` because an empty
    /// value on a form submit may simply mean "not touched".
    #[serde(default)]
    pub clear_thinking_level: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpModelsProviderInput {
    pub provider_key: String,
    pub provider: Value,
}

/// OMP subagent / roles 集中配置:一套方案(profile)。
/// - `model_roles`: 核心模型角色映射(default, plan, task, advisor, commit, tiny 等),
///   apply 时落盘写入运行时 `config.yml` 的 `modelRoles`。
/// - `agents`: 自定义 subagent 映射,apply 时渲染为 `<agentDir>/agents/*.md` 文件。
/// - `other_fields`: 保留扩展字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpAgentsConfig {
    pub id: String,
    pub name: String,
    pub is_applied: bool,
    pub is_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_roles: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// OMP subagent 方案内容(落库部分,不含辅助 id/时间戳之外的渲染字段)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmpAgentsConfigContent {
    pub name: String,
    pub is_applied: bool,
    pub is_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_roles: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建/更新 OMP subagent 方案的入参。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpAgentsConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_roles: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_fields: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpExtensionSummary {
    pub id: String,
    pub source: String,
    pub scope: OmpExtensionScope,
    pub kind: OmpExtensionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub built_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    /// npm registry `dist-tags.latest` when the package is unpinned and reachable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// True when `latest_version` is newer than `current_version`.
    #[serde(default)]
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpExtensionListResult {
    pub extensions_path: String,
    pub packages_path: String,
    pub extensions: Vec<OmpExtensionSummary>,
    pub raw: String,
    /// Resolved host-side `omp` binary path (or WSL invocation label) used for plugin CLI ops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    /// Best-effort `omp --version` stdout for the resolved CLI; omitted when probing fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpExtensionInstallInput {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpExtensionUpdateInput {
    /// When set, updates a single plugin source (`omp plugin upgrade <name>`).
    /// When omitted, updates all plugins (`omp plugin upgrade`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpExtensionActionInput {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<OmpExtensionScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<OmpExtensionKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpExtensionCommandResult {
    pub command: String,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmpExtensionScope {
    User,
    Project,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmpExtensionKind {
    Package,
    LocalFile,
    LocalDirectory,
}
