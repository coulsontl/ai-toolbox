//! Read-only defaults for runtime-backed providers whose connection metadata
//! lives in the CLI's built-in catalog rather than its user config.

use serde::Serialize;
use serde_json::Value;

use super::portable::{adapt_base_url, first_string, SharedConnection, SharedModel};
use crate::coding::open_code::free_models;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderShareDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub credential_unavailable: bool,
    #[serde(flatten)]
    pub connection: SharedConnection,
}

#[tauri::command]
pub fn get_provider_share_defaults(
    source_app: String,
    provider_id: String,
) -> Result<ProviderShareDefaults, String> {
    if !matches!(
        source_app.as_str(),
        "opencode" | "openclaw" | "pi" | "omp" | "hermes" | "dsh"
    ) {
        return Err("This tool does not use runtime provider defaults".to_string());
    }
    let mut defaults = ProviderShareDefaults::default();
    if source_app == "opencode" {
        (defaults.api_key, defaults.credential_unavailable) =
            free_models::shareable_auth_key(&provider_id)?;
    }
    if let Some(metadata) = free_models::provider_metadata_for_sharing(&provider_id) {
        defaults.connection.api_format = match metadata["npm"].as_str().unwrap_or_default() {
            "@ai-sdk/anthropic" => Some("anthropic_messages"),
            "@ai-sdk/google" => Some("gemini_native"),
            "@ai-sdk/openai" => Some("openai_responses"),
            "@ai-sdk/openai-compatible"
            | "@openrouter/ai-sdk-provider"
            | "@ai-sdk/xai"
            | "@ai-sdk/mistral" => Some("openai_chat"),
            _ => None,
        }
        .map(str::to_string);
        defaults.connection.models = metadata["models"]
            .as_object()
            .map(|models| {
                models
                    .iter()
                    .filter(|(_, model)| model["status"] != "deprecated")
                    .map(|(id, model)| {
                        let positive = |value: &Value| {
                            value
                                .as_u64()
                                .filter(|value| *value > 0)
                                .and_then(|value| u32::try_from(value).ok())
                        };
                        SharedModel {
                            id: model["id"].as_str().unwrap_or(id).to_string(),
                            name: first_string(model, &["name"]),
                            context_window: positive(&model["limit"]["context"]),
                            max_tokens: positive(&model["limit"]["output"]),
                            reasoning: model["reasoning"].as_bool(),
                            input: model["modalities"]["input"].as_array().map(|input| {
                                input
                                    .iter()
                                    .filter_map(|value| value.as_str().map(str::to_string))
                                    .collect()
                            }),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    defaults.base_url = free_models::resolve_provider_api_base_url(&provider_id).or_else(|| {
        match provider_id.as_str() {
            "xai" => Some("https://api.x.ai/v1".to_string()),
            "mistral" => Some("https://api.mistral.ai/v1".to_string()),
            _ => None,
        }
    });
    if let Some(base) = &defaults.base_url {
        defaults.connection.source_app = Some("opencode".to_string());
        defaults.base_url = Some(adapt_base_url(&mut defaults.connection, &source_app, base));
        defaults.connection.source_app = None;
    }
    if matches!(
        provider_id.as_str(),
        "github-copilot" | "github-copilot-enterprise"
    ) {
        defaults.connection.provider_type = Some("github_copilot".to_string());
    }
    Ok(defaults)
}
