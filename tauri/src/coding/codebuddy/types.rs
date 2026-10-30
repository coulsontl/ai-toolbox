use crate::coding::workbuddy_family::models_json::ModelEntryInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyPathInfo {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddySettingsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddySettingsConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    #[serde(default)]
    pub clear_config_dir: bool,
}

/// Input for creating or updating one model entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyModelInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_id: Option<String>,
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default = "default_true")]
    pub supports_tool_call: bool,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default)]
    pub only_reasoning: bool,
    #[serde(default)]
    pub use_custom_protocol: bool,
}

impl CodebuddyModelInput {
    /// The shared entry builder's input.
    pub fn as_entry_input(&self) -> ModelEntryInput<'_> {
        ModelEntryInput {
            id: &self.id,
            name: Some(&self.name),
            vendor: Some(&self.vendor),
            base_url: &self.base_url,
            api_key: Some(&self.api_key),
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            temperature: self.temperature,
            supports_tool_call: self.supports_tool_call,
            supports_images: self.supports_images,
            supports_reasoning: self.supports_reasoning,
            only_reasoning: self.only_reasoning,
            use_custom_protocol: self.use_custom_protocol,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyPromptConfigInput {
    pub id: Option<String>,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyPromptConfig {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    pub sort_index: Option<i32>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyPromptConfigContent {
    pub name: String,
    pub content: String,
    pub is_applied: bool,
    pub sort_index: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}
