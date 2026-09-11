use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshPathInfo {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DshSettingsConfigRecord {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshSettingsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshSettingsConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub clear_root_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DshPromptConfigRecord {
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
pub struct DshPromptConfig {
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
pub struct DshPromptConfigContent {
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
pub struct DshPromptConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshBuiltinProvider {
    pub key: String,
    pub name: String,
}

/// A single credential entry from `.credentials.yaml` (`REF: secret`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshCredentialView {
    pub ref_name: String,
    pub has_value: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DshProviderWarning {
    MissingProvider,
    MissingModel,
}

/// A single dsh provider merged from `llm-pi-ai.providers.<route>` (dict,
/// writable) or a built-in default provider record.
/// Where a provider route's served models come from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DshModelSource {
    /// Route configures its own `models` list (models.yaml / settings.yaml).
    #[default]
    Explicit,
    /// Route inherits the installed adapter catalog's default models.
    Builtin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshRuntimeProviderView {
    pub provider_key: String,
    pub display_name: String,
    /// `apiKeyEnv` reference stored on the provider (credential env-var name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Whether a matching `REF` entry exists in `.credentials.yaml`.
    pub credential_exists: bool,
    /// Resolved credential value from `.credentials.yaml` (empty when absent).
    /// Used by the frontend for upstream API calls without an extra round-trip.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// Raw provider JSON from `llm-pi-ai.providers.<route>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_ids: Vec<String>,
    /// Where `model_ids` / the served models come from. `builtin` means the
    /// route has no explicit `models` and serves the adapter default catalog.
    #[serde(default, skip_serializing_if = "is_explicit_source")]
    pub model_source: DshModelSource,
    /// The adapter default model records (route model schema shape) when
    /// `model_source` is `builtin`; the frontend renders/edits these as the
    /// route's model directory without writing them into `settings.yaml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_models: Option<Vec<Value>>,
    pub is_builtin: bool,
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<DshProviderWarning>,
}

fn is_explicit_source(source: &DshModelSource) -> bool {
    *source == DshModelSource::Explicit
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshRuntimeConfig {
    pub root_path_info: DshPathInfo,
    pub config_path: String,
    pub credentials_path: String,
    pub prompt_path: String,
    /// Raw `settings.yaml` content as a JSON object (unknown top-level keys pass
    /// through untouched).
    pub config: Value,
    pub model_settings: DshModelSettingsInput,
    /// Everything except the managed keys (`llm-pi-ai`, `agent-default-model`).
    pub other_settings: Value,
    pub providers: Vec<DshRuntimeProviderView>,
    pub builtin_providers: Vec<DshBuiltinProvider>,
    /// Credential references present in `.credentials.yaml`.
    pub credentials: Vec<DshCredentialView>,
    /// Raw `settings.yaml` file content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_content: Option<String>,
    /// Raw `.credentials.yaml` file content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_content: Option<String>,
    /// Raw `AGENTS.md` file content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_content: Option<String>,
    /// Home-level `cordis.patch.yml` path (AI Toolbox-managed MCP plugin layer).
    pub cordis_patch_path: String,
    /// Raw home-level `cordis.patch.yml` content for file-based preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cordis_patch_content: Option<String>,
}

/// Top-level `agent-default-model` section, also used as the save input.
///
/// String fields follow pi's clear semantics: `Some("")` removes the key,
/// `Some(value)` writes it, `None` leaves it untouched.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshModelSettingsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

/// Upsert a single `llm-pi-ai.providers.<route>` entry, keyed by route id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshModelsProviderInput {
    pub provider_key: String,
    pub provider: Value,
    /// Optional credential written together with the route (shared imports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<DshCredentialInput>,
}

/// Input for `save_dsh_credential`: writes `REF: value` into `.credentials.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshCredentialInput {
    pub ref_name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstructionsStatus {
    pub enabled: bool,
}
