use serde::{Deserialize, Serialize};

use crate::db::DbState;
use crate::http_client;

/// API type for fetching models
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiType {
    /// Provider's native models endpoint
    Native,
    /// OpenAI compatible /v1/models endpoint
    OpenaiCompat,
}

/// Request parameters for fetching models from provider API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchModelsRequest {
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: Option<serde_json::Value>,
    pub api_type: ApiType,
    pub sdk_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_url: Option<String>,
}

/// OpenAI compatible models list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIModelsResponse {
    pub object: Option<String>,
    pub data: Vec<OpenAIModel>,
}

/// OpenAI model object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
}

/// Google AI models list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleModelsResponse {
    pub models: Vec<GoogleModel>,
}

/// Google AI model object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_token_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_token_limit: Option<i64>,
}

/// Anthropic models list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicModelsResponse {
    pub data: Vec<AnthropicModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
}

/// Anthropic model object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicModel {
    pub id: String,
    #[serde(rename = "type")]
    pub model_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// Unified model info returned to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    pub name: Option<String>,
    pub owned_by: Option<String>,
    pub created: Option<i64>,
}

/// Response for fetch models command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchModelsResponse {
    pub models: Vec<FetchedModel>,
    pub total: usize,
}

/// Build models endpoint URL based on API type and SDK type
fn build_models_url(
    base_url: &str,
    api_type: &ApiType,
    sdk_type: Option<&str>,
    api_key: Option<&str>,
) -> String {
    let base = base_url.trim_end_matches('/');

    // Strip existing /v1 or /v1beta suffix
    let base_stripped = if base.ends_with("/v1beta") {
        base.trim_end_matches("/v1beta")
    } else if base.ends_with("/v1") {
        base.trim_end_matches("/v1")
    } else {
        base
    };

    match api_type {
        ApiType::OpenaiCompat => {
            // Always use /v1/models for OpenAI compatible
            format!("{}/v1/models", base_stripped)
        }
        ApiType::Native => {
            // Native endpoint depends on SDK type
            match sdk_type {
                Some("@ai-sdk/google") => {
                    // Google uses /v1beta/models with API key as query parameter
                    let models_url = format!("{}/v1beta/models", base_stripped);
                    if let Some(key) = api_key {
                        if !key.is_empty() {
                            return format!("{}?key={}", models_url, key);
                        }
                    }
                    models_url
                }
                Some("@ai-sdk/anthropic") => {
                    // Anthropic uses /v1/models
                    format!("{}/v1/models", base_stripped)
                }
                _ => {
                    // Fallback to OpenAI compatible format
                    format!("{}/v1/models", base_stripped)
                }
            }
        }
    }
}

/// Fetch models list from provider API
#[tauri::command]
pub async fn fetch_provider_models(
    state: tauri::State<'_, DbState>,
    request: FetchModelsRequest,
) -> Result<FetchModelsResponse, String> {
    // Create HTTP client with timeout and proxy support
    let client = http_client::client_with_timeout(&state, 30).await?;

    // Build request URL based on API type and SDK type
    // Use custom_url if provided, otherwise calculate it
    let url = if let Some(custom) = &request.custom_url {
        if !custom.is_empty() {
            custom.clone()
        } else {
            build_models_url(
                &request.base_url,
                &request.api_type,
                request.sdk_type.as_deref(),
                request.api_key.as_deref(),
            )
        }
    } else {
        build_models_url(
            &request.base_url,
            &request.api_type,
            request.sdk_type.as_deref(),
            request.api_key.as_deref(),
        )
    };

    // Build request
    let mut req_builder = client.get(&url);

    // Determine if this is Google Native (no Authorization header, key in URL)
    let is_google_native = matches!(request.api_type, ApiType::Native)
        && matches!(request.sdk_type.as_deref(), Some("@ai-sdk/google"));

    // Add authentication based on SDK type and API type
    match request.sdk_type.as_deref() {
        Some("@ai-sdk/google") if is_google_native => {
            // Google Native: API key is in URL, no Authorization header
        }
        Some("@ai-sdk/anthropic") if matches!(request.api_type, ApiType::Native) => {
            // Anthropic Native: use X-Api-Key header
            if let Some(api_key) = &request.api_key {
                if !api_key.is_empty() {
                    req_builder = req_builder.header("X-Api-Key", api_key);
                    req_builder = req_builder.header("anthropic-version", "2023-06-01");
                }
            }
        }
        _ => {
            // OpenAI Compatible or others: use Bearer token
            if let Some(api_key) = &request.api_key {
                if !api_key.is_empty() {
                    req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
                }
            }
        }
    }

    // Add custom headers
    if let Some(headers) = &request.headers {
        if let Some(obj) = headers.as_object() {
            for (key, value) in obj {
                if let Some(v) = value.as_str() {
                    req_builder = req_builder.header(key, v);
                }
            }
        }
    }

    // Send request
    let response = req_builder
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    // Check response status
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API error: {} - {}", status, body));
    }

    // Parse response based on SDK type and API type
    let models: Vec<FetchedModel> = match (request.api_type, request.sdk_type.as_deref()) {
        (ApiType::Native, Some("@ai-sdk/google")) => {
            // Parse Google AI response format
            let google_response: GoogleModelsResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse Google response: {}", e))?;

            google_response
                .models
                .into_iter()
                .map(|m| {
                    // Google model name format: "models/gemini-1.5-pro"
                    // Extract the model ID part after "models/"
                    let id = m
                        .name
                        .strip_prefix("models/")
                        .unwrap_or(&m.name)
                        .to_string();
                    FetchedModel {
                        id: id.clone(),
                        name: m.display_name.or(Some(id)),
                        owned_by: Some("google".to_string()),
                        created: None,
                    }
                })
                .collect()
        }
        (ApiType::Native, Some("@ai-sdk/anthropic")) => {
            // Parse Anthropic response format
            let anthropic_response: AnthropicModelsResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse Anthropic response: {}", e))?;

            anthropic_response
                .data
                .into_iter()
                .map(|m| {
                    let name = m.display_name.clone().unwrap_or_else(|| m.id.clone());
                    FetchedModel {
                        id: m.id.clone(),
                        name: Some(name),
                        owned_by: Some("anthropic".to_string()),
                        created: None,
                    }
                })
                .collect()
        }
        _ => {
            // Parse OpenAI compatible response format
            // First, get response text for debugging
            let response_text = response.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

            // Try to parse as OpenAI format
            let openai_response: OpenAIModelsResponse = serde_json::from_str(&response_text)
                .map_err(|e| format!("Failed to parse OpenAI response: {}. Response was: {}", e, response_text))?;

            openai_response
                .data
                .into_iter()
                .map(|m| FetchedModel {
                    id: m.id.clone(),
                    name: Some(m.id),
                    owned_by: m.owned_by,
                    created: m.created,
                })
                .collect()
        }
    };

    let total = models.len();

    Ok(FetchModelsResponse { models, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_models_url_openai_compat() {
        // Base URL without /v1
        assert_eq!(
            build_models_url("https://api.openai.com", &ApiType::OpenaiCompat, None, None),
            "https://api.openai.com/v1/models"
        );

        // Base URL with /v1
        assert_eq!(
            build_models_url("https://api.openai.com/v1", &ApiType::OpenaiCompat, None, None),
            "https://api.openai.com/v1/models"
        );

        // Base URL with trailing slash
        assert_eq!(
            build_models_url("https://api.openai.com/v1/", &ApiType::OpenaiCompat, None, None),
            "https://api.openai.com/v1/models"
        );

        // Base URL with /v1beta (Google style) should convert to /v1
        assert_eq!(
            build_models_url(
                "https://generativelanguage.googleapis.com/v1beta",
                &ApiType::OpenaiCompat,
                None,
                None
            ),
            "https://generativelanguage.googleapis.com/v1/models"
        );
    }

    #[test]
    fn test_build_models_url_native_google() {
        // Google Native without api key
        assert_eq!(
            build_models_url(
                "https://generativelanguage.googleapis.com",
                &ApiType::Native,
                Some("@ai-sdk/google"),
                None
            ),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );

        // Google Native with /v1beta (should strip and re-add)
        assert_eq!(
            build_models_url(
                "https://generativelanguage.googleapis.com/v1beta",
                &ApiType::Native,
                Some("@ai-sdk/google"),
                None
            ),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );

        // Google Native with api key
        assert_eq!(
            build_models_url(
                "https://generativelanguage.googleapis.com",
                &ApiType::Native,
                Some("@ai-sdk/google"),
                Some("test-api-key")
            ),
            "https://generativelanguage.googleapis.com/v1beta/models?key=test-api-key"
        );
    }

    #[test]
    fn test_build_models_url_native_anthropic() {
        // Anthropic Native
        assert_eq!(
            build_models_url(
                "https://api.anthropic.com",
                &ApiType::Native,
                Some("@ai-sdk/anthropic"),
                None
            ),
            "https://api.anthropic.com/v1/models"
        );

        // Anthropic Native with /v1 (should strip and re-add)
        assert_eq!(
            build_models_url(
                "https://api.anthropic.com/v1",
                &ApiType::Native,
                Some("@ai-sdk/anthropic"),
                None
            ),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn test_build_models_url_native_fallback() {
        // Unknown SDK type falls back to OpenAI compatible format
        assert_eq!(
            build_models_url(
                "https://api.example.com",
                &ApiType::Native,
                Some("@ai-sdk/unknown"),
                None
            ),
            "https://api.example.com/v1/models"
        );
    }
}
