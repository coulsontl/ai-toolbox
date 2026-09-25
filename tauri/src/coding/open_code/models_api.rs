use serde::{Deserialize, Serialize};

use crate::coding::config_value_host::{config_value_host_from_location, ConfigValueHost};
use crate::coding::omp_config_value;
use crate::coding::pi_config_value;
use crate::coding::runtime_location;
use crate::db::SqliteDbState;
use crate::http_client;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;
use uuid::Uuid;

/// API type for fetching models
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiType {
    /// Provider's native models endpoint
    Native,
    /// OpenAI compatible /v1/models endpoint
    OpenaiCompat,
}

/// Config value syntax a request opts into for its credential fields.
///
/// Pi stores `$ENV_VAR` / `!command` templates in `models.json` and OMP stores
/// `!command` / environment-variable names in `models.yml`. Callers that opt in
/// ask the shared discovery/connectivity commands to resolve them before the
/// value reaches an HTTP request; every other caller keeps sending raw literals.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValueMode {
    Pi,
    Omp,
}

/// Request parameters for fetching models from provider API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchModelsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: Option<serde_json::Value>,
    pub api_type: ApiType,
    pub sdk_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_value_mode: Option<ConfigValueMode>,
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
    pub created: Option<Value>,
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

/// Codex model catalog response: `{"models":[{"slug": "...", ...}]}`.
///
/// Zhipu GLM 的 Codex 原生 Responses 端点 `/api/v1/models` 与 Codex 官方
/// `models.json` 使用同一 schema；OpenAI `data[]` 和 Google `models[].name`
/// 都解析不到 `slug`，必须单独识别，否则整个模型列表会被拒绝。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCatalogModelsResponse {
    pub models: Vec<CodexCatalogModel>,
}

/// Codex model catalog entry; unknown metadata fields are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCatalogModel {
    #[serde(default, alias = "id")]
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
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

fn parse_anthropic_models_response(response_text: &str) -> Result<Vec<FetchedModel>, String> {
    match serde_json::from_str::<AnthropicModelsResponse>(response_text) {
        Ok(anthropic_response) => Ok(anthropic_response
            .data
            .into_iter()
            .map(|model| {
                let name = model
                    .display_name
                    .clone()
                    .unwrap_or_else(|| model.id.clone());
                FetchedModel {
                    id: model.id,
                    name: Some(name),
                    owned_by: Some("anthropic".to_string()),
                    created: None,
                }
            })
            .collect()),
        Err(anthropic_error) => {
            match serde_json::from_str::<OpenAIModelsResponse>(response_text) {
                Ok(openai_response) => Ok(openai_response
                    .data
                    .into_iter()
                    .map(|model| FetchedModel {
                        id: model.id.clone(),
                        name: Some(model.id),
                        owned_by: model.owned_by,
                        created: model.created.and_then(|value| value.as_i64()),
                    })
                    .collect()),
                Err(openai_error) => Err(format!(
                    "Failed to parse Anthropic-compatible response: Anthropic format: {}; OpenAI format: {}",
                    anthropic_error, openai_error
                )),
            }
        }
    }
}

// ============================================================================
// Connectivity Test Types
// ============================================================================

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectivityApiFormat {
    OpenaiCodexResponses,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityTestRequest {
    pub npm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_format: Option<ConnectivityApiFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub headers: Option<Value>,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    pub model_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_value_mode: Option<ConfigValueMode>,
}

impl ConnectivityTestRequest {
    fn is_codex(&self) -> bool {
        self.api_format == Some(ConnectivityApiFormat::OpenaiCodexResponses)
    }

    fn effective_npm(&self) -> &str {
        if self.is_codex() {
            "@ai-sdk/openai"
        } else {
            &self.npm
        }
    }

    fn streaming(&self) -> bool {
        self.is_codex() || self.stream.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityTestResult {
    pub model_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_byte_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub request_url: String,
    pub request_headers: Value,
    pub request_body: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityTestResponse {
    pub results: Vec<ConnectivityTestResult>,
}

#[derive(Debug, Clone)]
struct ResolvedProviderRequest {
    base_url: String,
    api_key: Option<String>,
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|trimmed| !trimmed.is_empty())
        .map(str::to_string)
}

fn resolve_provider_request(
    provider_id: Option<&str>,
    base_url: &str,
    api_key: Option<&str>,
    allow_stored_credential_fallback: bool,
) -> ResolvedProviderRequest {
    let resolved_base_url = normalize_optional_string(Some(base_url))
        .or_else(|| provider_id.and_then(super::free_models::resolve_provider_api_base_url))
        .unwrap_or_default();

    let resolved_api_key = normalize_optional_string(api_key).or_else(|| {
        if allow_stored_credential_fallback {
            provider_id.and_then(super::free_models::resolve_auth_credential)
        } else {
            None
        }
    });

    ResolvedProviderRequest {
        base_url: resolved_base_url,
        api_key: resolved_api_key,
    }
}

/// Whether the shared command may fall back to a credential kept by another
/// tool when this request resolved to no API key.
///
/// OMP omits an API key whose `!command` fails or prints nothing, and a
/// configured key belongs to the OMP provider: replacing it with an entry from
/// another tool's auth store (e.g. OpenCode `auth.json`) would authenticate the
/// request with the wrong secret. Requests without a configured key keep the
/// existing provider fallback.
fn stored_credential_fallback_allowed(
    config_value_mode: Option<ConfigValueMode>,
    configured_api_key: Option<&str>,
) -> bool {
    let has_configured_key = configured_api_key.is_some_and(|key| !key.trim().is_empty());
    match config_value_mode {
        Some(ConfigValueMode::Omp) => !has_configured_key,
        _ => true,
    }
}

/// Build models endpoint URL based on API type and SDK type
fn normalize_google_models_base_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let last_segment = base.rsplit('/').next().unwrap_or(base);

    if matches!(last_segment, "v1" | "v1alpha" | "v1beta") {
        base.to_string()
    } else {
        format!("{}/v1beta", base)
    }
}

fn build_models_url(
    base_url: &str,
    api_type: &ApiType,
    sdk_type: Option<&str>,
    api_key: Option<&str>,
) -> String {
    let base = base_url.trim_end_matches('/');

    match api_type {
        ApiType::OpenaiCompat => {
            format!("{}/models", base)
        }
        ApiType::Native => match sdk_type {
            Some("@ai-sdk/google") => {
                let base = normalize_google_models_base_url(base);
                let models_url = format!("{}/models", base);
                if let Some(key) = api_key {
                    if !key.is_empty() {
                        return format!("{}?key={}", models_url, key);
                    }
                }
                models_url
            }
            _ => {
                format!("{}/models", base)
            }
        },
    }
}

// ============================================================================
// Provider config value syntax
// ============================================================================

/// Environment a config value belongs to.
///
/// Pi and OMP resolve `!command` inside their own runtime, so a WSL Direct root
/// has to be resolved inside that distribution instead of against the desktop
/// process environment.
async fn config_value_host(
    state: &SqliteDbState,
    mode: ConfigValueMode,
) -> Result<ConfigValueHost, String> {
    let location = match mode {
        ConfigValueMode::Pi => runtime_location::get_pi_runtime_location_async(state).await?,
        ConfigValueMode::Omp => {
            runtime_location::get_oh_my_pi_runtime_location_async(state).await?
        }
    };
    Ok(config_value_host_from_location(&location))
}

/// Whether the mode resolves credential values in the tool's own runtime.
fn resolves_config_values(config_value_mode: Option<ConfigValueMode>) -> bool {
    matches!(
        config_value_mode,
        Some(ConfigValueMode::Pi | ConfigValueMode::Omp)
    )
}

/// Resolve the credential fields a caller passed through.
///
/// Only callers that opt into a `ConfigValueMode` are touched, so every other
/// tool keeps forwarding its provider-owned strings untouched. Pi reports an
/// unresolvable value as an error; OMP omits it, the way its own runtime does.
async fn resolve_credentials(
    state: &SqliteDbState,
    provider_id: Option<&str>,
    api_key: Option<&str>,
    headers: Option<&Value>,
    config_value_mode: Option<ConfigValueMode>,
) -> Result<(Option<String>, Option<Value>), String> {
    let Some(mode) = config_value_mode else {
        return Ok((api_key.map(str::to_string), headers.cloned()));
    };

    // Nothing to resolve: skip the runtime location lookup (and do not fail the
    // request on an unrelated lookup error) when no credential field needs
    // resolution.
    if !has_config_values(api_key, headers) {
        return Ok((None, headers.cloned()));
    }

    let host = config_value_host(state, mode).await?;
    let provider_label = provider_id.unwrap_or(match mode {
        ConfigValueMode::Pi => "pi",
        ConfigValueMode::Omp => "omp",
    });

    match mode {
        ConfigValueMode::Pi => {
            let resolved_api_key = match api_key {
                Some(raw_api_key) => {
                    let label = format!("API key for provider \"{provider_label}\"");
                    Some(pi_config_value::resolve_config_value(raw_api_key, &label, &host).await?)
                }
                None => None,
            };

            let resolved_headers = match headers {
                Some(Value::Object(header_map)) => Some(Value::Object(
                    pi_config_value::resolve_header_values(header_map, provider_label, &host)
                        .await?,
                )),
                other => other.cloned(),
            };

            Ok((resolved_api_key, resolved_headers))
        }
        ConfigValueMode::Omp => {
            let resolved_api_key = match api_key {
                Some(raw_api_key) => {
                    omp_config_value::resolve_config_value(raw_api_key, &host).await
                }
                None => None,
            };

            let resolved_headers = match headers {
                Some(Value::Object(header_map)) => Some(Value::Object(
                    omp_config_value::resolve_header_values(header_map, &host).await,
                )),
                other => other.cloned(),
            };

            Ok((resolved_api_key, resolved_headers))
        }
    }
}

/// Whether a request carries any value that may need resolving.
///
/// An empty header object and a blank API key are both common (the modal always
/// sends the provider header map, and the pages send an empty string when a
/// provider has no credential), so neither may trigger a runtime location lookup
/// on its own.
fn has_config_values(api_key: Option<&str>, headers: Option<&Value>) -> bool {
    let has_header_values = match headers {
        Some(Value::Object(header_map)) => !header_map.is_empty(),
        Some(_) => true,
        None => false,
    };
    let has_api_key = api_key.is_some_and(|key| !key.trim().is_empty());
    has_api_key || has_header_values
}

/// Append a resolved Google API key to a model-discovery URL.
///
/// Google native auth travels in the query string rather than an Authorization
/// header, and a Pi caller cannot embed a runtime-resolved key into the URL the
/// modal displays, so the resolved key is added here.
fn append_google_native_key(url: String, api_key: Option<&str>) -> String {
    let Some(api_key) = api_key.filter(|key| !key.is_empty()) else {
        return url;
    };
    if url.contains("key=") {
        return url;
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}key={api_key}")
}

/// Fetch models list from provider API
#[tauri::command]
pub async fn fetch_provider_models(
    state: tauri::State<'_, SqliteDbState>,
    request: FetchModelsRequest,
) -> Result<FetchModelsResponse, String> {
    // Pi / OMP credentials are resolved in that tool's own runtime, so they are
    // resolved before the provider fallback to keep another tool's stored
    // credential out of the config value syntax.
    let (api_key, headers) = resolve_credentials(
        &state,
        request.provider_id.as_deref(),
        request.api_key.as_deref(),
        request.headers.as_ref(),
        request.config_value_mode,
    )
    .await?;

    let resolved_request = resolve_provider_request(
        request.provider_id.as_deref(),
        &request.base_url,
        api_key.as_deref(),
        stored_credential_fallback_allowed(request.config_value_mode, request.api_key.as_deref()),
    );

    // Determine if this is Google Native (no Authorization header, key in URL)
    let is_google_native = matches!(request.api_type, ApiType::Native)
        && matches!(request.sdk_type.as_deref(), Some("@ai-sdk/google"));

    // Create HTTP client with timeout and proxy support
    let client = http_client::client_with_timeout(&state, 30).await?;

    // Build request URL based on API type and SDK type
    // Use custom_url if provided, otherwise calculate it
    let url = if let Some(custom) = &request.custom_url {
        if !custom.trim().is_empty() {
            custom.trim().to_string()
        } else {
            if resolved_request.base_url.is_empty() {
                return Err("Missing base URL".to_string());
            }
            build_models_url(
                &resolved_request.base_url,
                &request.api_type,
                request.sdk_type.as_deref(),
                resolved_request.api_key.as_deref(),
            )
        }
    } else {
        if resolved_request.base_url.is_empty() {
            return Err("Missing base URL".to_string());
        }
        build_models_url(
            &resolved_request.base_url,
            &request.api_type,
            request.sdk_type.as_deref(),
            resolved_request.api_key.as_deref(),
        )
    };

    // A Pi / OMP caller cannot bake a runtime-resolved key into the discovery
    // URL, so query-parameter auth is completed here instead.
    let url = if resolves_config_values(request.config_value_mode) && is_google_native {
        append_google_native_key(url, resolved_request.api_key.as_deref())
    } else {
        url
    };

    // Build request
    let mut req_builder = client.get(&url);

    // Add authentication based on SDK type and API type
    match request.sdk_type.as_deref() {
        Some("@ai-sdk/google") if is_google_native => {
            // Google Native: API key is in URL, no Authorization header
        }
        Some("@ai-sdk/anthropic") if matches!(request.api_type, ApiType::Native) => {
            // Anthropic Native: keep the standard Authorization Bearer header so
            // proxies that only forward Authorization keep working. Anthropic-compatible
            // gateways such as New API accept Bearer and return either the Anthropic or
            // the OpenAI model-list schema, both handled by parse_anthropic_models_response.
            if let Some(api_key) = &resolved_request.api_key {
                if !api_key.is_empty() {
                    req_builder =
                        req_builder.header("Authorization", format!("Bearer {}", api_key));
                    req_builder = req_builder.header("anthropic-version", "2023-06-01");
                }
            }
        }
        _ => {
            // OpenAI Compatible or others: use Bearer token
            if let Some(api_key) = &resolved_request.api_key {
                if !api_key.is_empty() {
                    req_builder =
                        req_builder.header("Authorization", format!("Bearer {}", api_key));
                }
            }
        }
    }

    // Add custom headers
    if let Some(resolved_headers) = &headers {
        if let Some(obj) = resolved_headers.as_object() {
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
            // Anthropic-compatible gateways may expose either the Anthropic model
            // list schema or the OpenAI-compatible schema. Read the body once and
            // accept both so authentication and schema quirks do not make discovery
            // fail when inference itself is compatible.
            let response_text = response
                .text()
                .await
                .map_err(|e| format!("Failed to read Anthropic response: {}", e))?;

            parse_anthropic_models_response(&response_text)?
        }
        _ => {
            // Parse OpenAI compatible response format
            let response_text = response
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {}", e))?;

            parse_generic_models_response(&response_text)?
        }
    };

    let total = models.len();

    Ok(FetchModelsResponse { models, total })
}

/// Parse the model-list schemas accepted for OpenAI-compatible discovery.
///
/// Order matters: OpenAI `data[]` first, then Google `models[].name`, then the
/// Codex catalog schema (`models[].slug`) used by Codex-native endpoints such
/// as Zhipu GLM `/api/v1`. A response that matches none of them keeps the
/// original diagnostic text so users can see the raw payload.
fn parse_generic_models_response(response_text: &str) -> Result<Vec<FetchedModel>, String> {
    if let Ok(openai_response) = serde_json::from_str::<OpenAIModelsResponse>(response_text) {
        return Ok(openai_response
            .data
            .into_iter()
            .map(|m| FetchedModel {
                id: m.id.clone(),
                name: Some(m.id),
                owned_by: m.owned_by,
                created: m.created.and_then(|v| v.as_i64()),
            })
            .collect());
    }
    if let Ok(google_response) = serde_json::from_str::<GoogleModelsResponse>(response_text) {
        return Ok(google_response
            .models
            .into_iter()
            .map(|m| {
                let id = m
                    .name
                    .strip_prefix("models/")
                    .unwrap_or(&m.name)
                    .to_string();
                FetchedModel {
                    id: id.clone(),
                    name: m.display_name.or(Some(id)),
                    owned_by: None,
                    created: None,
                }
            })
            .collect());
    }
    if let Ok(codex_response) = serde_json::from_str::<CodexCatalogModelsResponse>(response_text) {
        return Ok(codex_response
            .models
            .into_iter()
            .filter_map(|m| {
                let id = m.slug.trim().to_string();
                if id.is_empty() {
                    return None;
                }
                let name = m
                    .display_name
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| id.clone());
                Some(FetchedModel {
                    id,
                    name: Some(name),
                    owned_by: None,
                    created: None,
                })
            })
            .collect());
    }
    Err(format!(
        "Failed to parse models response. Response was: {}",
        response_text
    ))
}

// ============================================================================
// Connectivity Test Command
// ============================================================================

fn headers_to_value(headers: &BTreeMap<String, String>) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in headers {
        map.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(map)
}

fn header_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        Value::Null => None,
        _ => Some(value.to_string()),
    }
}

fn wrap_json_object(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        other => json!({ "value": other }),
    }
}

fn parse_json_or_wrap(text: &str) -> Value {
    if text.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value) => wrap_json_object(value),
        Err(_) => json!({ "raw": text }),
    }
}

fn parse_stream_response(text: &str) -> Value {
    let mut items: Vec<Value> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(payload) = trimmed.strip_prefix("data:") {
            let payload = payload.trim();
            if payload == "[DONE]" {
                continue;
            }
            match serde_json::from_str::<Value>(payload) {
                Ok(value) => items.push(wrap_json_object(value)),
                Err(_) => items.push(json!({ "raw": payload })),
            }
        } else {
            match serde_json::from_str::<Value>(trimmed) {
                Ok(value) => items.push(wrap_json_object(value)),
                Err(_) => items.push(json!({ "raw": trimmed })),
            }
        }
    }
    if items.is_empty() {
        items.push(json!({ "raw": text }));
    }
    Value::Array(items)
}

fn generate_anthropic_user_id() -> String {
    let user_hex = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let session_id = Uuid::new_v4();
    format!("user_{}_account__session_{}", user_hex, session_id)
}

fn ensure_anthropic_metadata(body: &mut Value, user_id: &str) {
    match body.get_mut("metadata") {
        Some(Value::Object(map)) => {
            if !map.contains_key("user_id") {
                map.insert("user_id".to_string(), Value::String(user_id.to_string()));
            }
        }
        _ => {
            body["metadata"] = json!({ "user_id": user_id });
        }
    }
}

fn build_connectivity_url(
    npm: &str,
    base_url: &str,
    model_id: &str,
    api_key: Option<&str>,
    stream: bool,
) -> String {
    let base = base_url.trim_end_matches('/');
    match npm {
        "@ai-sdk/openai" => format!("{}/responses", base),
        "@ai-sdk/google" => {
            let normalized_model = model_id.strip_prefix("models/").unwrap_or(model_id);
            let action = if stream {
                "streamGenerateContent"
            } else {
                "generateContent"
            };
            let url = format!("{}/models/{}:{}", base, normalized_model, action);
            if let Some(key) = api_key {
                if !key.is_empty() {
                    return format!("{}?key={}", url, key);
                }
            }
            url
        }
        "@ai-sdk/anthropic" => format!("{}/messages", base),
        "@ai-sdk/openai-compatible" => format!("{}/chat/completions", base),
        _ => format!("{}/chat/completions", base),
    }
}

fn merge_json(base: &mut Value, overrides: &Value) {
    match (base, overrides) {
        (Value::Object(base_map), Value::Object(override_map)) => {
            for (key, override_value) in override_map {
                match base_map.get_mut(key) {
                    Some(base_value) => merge_json(base_value, override_value),
                    None => {
                        base_map.insert(key.clone(), override_value.clone());
                    }
                }
            }
        }
        (base_value, override_value) => {
            *base_value = override_value.clone();
        }
    }
}

fn build_default_body(
    request: &ConnectivityTestRequest,
    model_id: &str,
    anthropic_user_id: Option<&str>,
) -> Value {
    let stream_enabled = request.streaming();
    match request.effective_npm() {
        "@ai-sdk/google" => {
            let mut generation_config = serde_json::Map::new();
            if let Some(temperature) = request.temperature {
                generation_config.insert("temperature".to_string(), json!(temperature));
            }
            if let Some(max_output_tokens) = request.max_output_tokens {
                generation_config.insert("maxOutputTokens".to_string(), json!(max_output_tokens));
            }

            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            { "text": request.prompt }
                        ]
                    }
                ],
                "generationConfig": Value::Object(generation_config)
            })
        }
        "@ai-sdk/anthropic" => {
            let max_tokens = request.max_tokens.unwrap_or(32000);
            let mut body = json!({
                "model": model_id,
                "max_tokens": max_tokens,
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            { "text": request.prompt, "type": "text" }
                        ]
                    }
                ],
                "metadata": {
                    "user_id": anthropic_user_id.unwrap_or("opencode_connectivity_test")
                },
                "stream": stream_enabled,
                "system": [
                    {
                        "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                        "type": "text"
                    },
                    {
                        "text": "Reply must be made according to the user's requirements.",
                        "type": "text"
                    }
                ],
                "tools": []
            });
            if let Some(temperature) = request.temperature {
                body["temperature"] = json!(temperature);
            }
            body
        }
        "@ai-sdk/openai" => {
            let mut body = json!({
                "model": model_id,
                "input": [
                    {
                        "type": "message",
                        "role": "developer",
                        "content": [
                            {
                                "type": "input_text",
                                "text": "You are OpenCode, the best coding agent on the planet."
                            }
                        ]
                    },
                    {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": request.prompt
                            }
                        ]
                    }
                ],
                "stream": stream_enabled,
            });
            if let Some(reasoning_effort) = request.reasoning_effort.as_deref() {
                if !reasoning_effort.trim().is_empty() {
                    body["reasoning"] = json!({
                        "effort": reasoning_effort.trim()
                    });
                }
            }
            if let Some(temperature) = request.temperature {
                body["temperature"] = json!(temperature);
            }
            if let Some(max_tokens) = request.max_tokens {
                body["max_output_tokens"] = json!(max_tokens);
            }
            body
        }
        _ => {
            let mut body = json!({
                "model": model_id,
                "messages": [
                    { "role": "user", "content": request.prompt }
                ],
            });
            if let Some(temperature) = request.temperature {
                body["temperature"] = json!(temperature);
            }
            if let Some(max_tokens) = request.max_tokens {
                body["max_tokens"] = json!(max_tokens);
            }
            body["stream"] = json!(stream_enabled);
            body
        }
    }
}

fn build_codex_connectivity_url(base_url: &str) -> String {
    let append_path = |path: &str| {
        let path = path.trim_end_matches('/');
        if path.ends_with("/codex/responses") {
            path.to_string()
        } else if path.ends_with("/codex") {
            format!("{path}/responses")
        } else {
            format!("{path}/codex/responses")
        }
    };
    match reqwest::Url::parse(base_url) {
        Ok(mut url) => {
            url.set_path(&append_path(url.path()));
            url.to_string()
        }
        Err(_) => append_path(base_url),
    }
}

fn codex_account_id(api_key: &str) -> Option<String> {
    use base64::Engine;
    let payload = api_key.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(str::to_string)
}

fn codex_stream_error(text: &str) -> Option<String> {
    let normalized = text.replace("\r\n", "\n");
    let events: Vec<Value> = normalized
        .split_inclusive("\n\n")
        .filter(|frame| frame.ends_with("\n\n"))
        .filter_map(|frame| {
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            serde_json::from_str(&data).ok()
        })
        .collect();
    for event in &events {
        if matches!(
            event.get("type").and_then(Value::as_str),
            Some("error" | "response.failed" | "response.incomplete")
        ) || event.get("error").is_some_and(|error| !error.is_null())
            || matches!(
                event.pointer("/response/status").and_then(Value::as_str),
                Some("failed" | "incomplete")
            )
        {
            return Some(
                event
                    .pointer("/error/message")
                    .or_else(|| event.pointer("/response/error/message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Codex Responses stream failed")
                    .to_string(),
            );
        }
    }
    if events
        .iter()
        .any(|event| event.get("type").and_then(Value::as_str) == Some("response.completed"))
    {
        None
    } else {
        Some("Codex Responses stream ended without response.completed".to_string())
    }
}

fn enforce_prompt_and_model(npm: &str, body: &mut Value, model_id: &str, prompt: &str) {
    match npm {
        "@ai-sdk/google" => {
            body["contents"] = json!([
                {
                    "role": "user",
                    "parts": [
                        { "text": prompt }
                    ]
                }
            ]);
        }
        "@ai-sdk/anthropic" => {
            body["model"] = json!(model_id);
            body["messages"] = json!([
                {
                    "role": "user",
                    "content": [
                        { "text": prompt, "type": "text" }
                    ]
                }
            ]);
        }
        "@ai-sdk/openai" => {
            body["model"] = json!(model_id);
            body["input"] = json!([
                {
                    "type": "message",
                    "role": "developer",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "You are OpenCode, the best coding agent on the planet."
                        }
                    ]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": prompt
                        }
                    ]
                }
            ]);
        }
        "@ai-sdk/openai-compatible" => {
            body["model"] = json!(model_id);
            body["messages"] = json!([
                { "role": "user", "content": prompt }
            ]);
        }
        _ => {
            body["model"] = json!(model_id);
            body["messages"] = json!([
                { "role": "user", "content": prompt }
            ]);
        }
    }
}

async fn run_connectivity_test_for_model(
    client: &reqwest::Client,
    request: &ConnectivityTestRequest,
    model_id: &str,
) -> ConnectivityTestResult {
    let start_time = Instant::now();
    let stream_enabled = request.streaming();
    let npm = request.effective_npm();
    let anthropic_user_id = if npm == "@ai-sdk/anthropic" {
        Some(generate_anthropic_user_id())
    } else {
        None
    };
    let url = if request.is_codex() {
        build_codex_connectivity_url(&request.base_url)
    } else {
        build_connectivity_url(
            npm,
            request.base_url.as_str(),
            model_id,
            request.api_key.as_deref(),
            stream_enabled,
        )
    };

    let mut body = build_default_body(request, model_id, anthropic_user_id.as_deref());
    if let Some(custom_body) = &request.body {
        merge_json(&mut body, custom_body);
    }
    enforce_prompt_and_model(npm, &mut body, model_id, &request.prompt);
    if request.is_codex() {
        body["stream"] = json!(true);
        body["store"] = json!(false);
        if !body.get("instructions").is_some_and(Value::is_string) {
            body["instructions"] = json!("You are a helpful coding assistant.");
        }
        for field in ["temperature", "max_tokens", "max_output_tokens"] {
            body.as_object_mut().unwrap().remove(field);
        }
    }
    if let Some(user_id) = anthropic_user_id.as_deref() {
        ensure_anthropic_metadata(&mut body, user_id);
    }

    let mut req_builder = client.post(&url).json(&body);

    let is_google = npm == "@ai-sdk/google";
    let is_anthropic = npm == "@ai-sdk/anthropic";

    let mut request_headers = BTreeMap::new();
    if is_anthropic {
        request_headers.insert("Accept".to_string(), "application/json".to_string());
        request_headers.insert(
            "Accept-Encoding".to_string(),
            "gzip, deflate, br, zstd".to_string(),
        );
        request_headers.insert("Connection".to_string(), "keep-alive".to_string());
        request_headers.insert("Content-Type".to_string(), "application/json".to_string());
        request_headers.insert(
            "User-Agent".to_string(),
            "claude-cli/2.1.19 (external, cli)".to_string(),
        );
        request_headers.insert(
            "anthropic-beta".to_string(),
            "interleaved-thinking-2025-05-14".to_string(),
        );
        request_headers.insert(
            "anthropic-dangerous-direct-browser-access".to_string(),
            "true".to_string(),
        );
        request_headers.insert("anthropic-version".to_string(), "2023-06-01".to_string());
    }

    if is_anthropic {
        if let Some(api_key) = &request.api_key {
            if !api_key.is_empty() {
                request_headers.insert("Authorization".to_string(), format!("Bearer {}", api_key));
            }
        }
    } else if !is_google {
        if let Some(api_key) = &request.api_key {
            if !api_key.is_empty() {
                request_headers.insert("Authorization".to_string(), format!("Bearer {}", api_key));
            }
        }
    }

    if stream_enabled && !is_google && !is_anthropic {
        request_headers.insert("Accept".to_string(), "text/event-stream".to_string());
    }

    if request.is_codex() {
        request_headers.insert(
            "OpenAI-Beta".to_string(),
            "responses=experimental".to_string(),
        );
        if let Some(account_id) = request.api_key.as_deref().and_then(codex_account_id) {
            request_headers.insert("ChatGPT-Account-Id".to_string(), account_id);
        }
    }

    if let Some(Value::Object(obj)) = request.headers.as_ref() {
        for (key, value) in obj {
            if let Some(v) = header_value_to_string(value) {
                request_headers.insert(key.clone(), v);
            }
        }
    }

    for (key, value) in &request_headers {
        req_builder = req_builder.header(key, value);
    }

    let request_headers_value = headers_to_value(&request_headers);
    let request_body_value = body.clone();

    let response = match req_builder.send().await {
        Ok(resp) => resp,
        Err(err) => {
            let status = if err.is_timeout() { "timeout" } else { "error" };
            return ConnectivityTestResult {
                model_id: model_id.to_string(),
                status: status.to_string(),
                first_byte_ms: None,
                total_ms: None,
                error_message: Some(err.to_string()),
                request_url: url,
                request_headers: request_headers_value,
                request_body: request_body_value,
                response_headers: None,
                response_body: None,
            };
        }
    };

    let status_code = response.status();
    let mut response_headers_map = serde_json::Map::new();
    for (key, value) in response.headers().iter() {
        let header_value = value.to_str().unwrap_or("");
        response_headers_map.insert(key.to_string(), Value::String(header_value.to_string()));
    }
    let response_headers_value = Value::Object(response_headers_map);

    let mut first_byte_ms: Option<u64> = None;
    let mut body_bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                if first_byte_ms.is_none() {
                    first_byte_ms = Some(start_time.elapsed().as_millis() as u64);
                }
                body_bytes.extend_from_slice(&bytes);
            }
            Err(err) => {
                let body_text = String::from_utf8_lossy(&body_bytes).to_string();
                let response_body_value = if stream_enabled {
                    parse_stream_response(&body_text)
                } else {
                    parse_json_or_wrap(&body_text)
                };
                return ConnectivityTestResult {
                    model_id: model_id.to_string(),
                    status: "error".to_string(),
                    first_byte_ms,
                    total_ms: Some(start_time.elapsed().as_millis() as u64),
                    error_message: Some(err.to_string()),
                    request_url: url,
                    request_headers: request_headers_value,
                    request_body: request_body_value,
                    response_headers: Some(response_headers_value),
                    response_body: Some(response_body_value),
                };
            }
        }
    }

    let total_ms = start_time.elapsed().as_millis() as u64;
    if first_byte_ms.is_none() {
        first_byte_ms = Some(total_ms);
    }

    let body_text = String::from_utf8_lossy(&body_bytes).to_string();
    let response_body_value = if stream_enabled {
        parse_stream_response(&body_text)
    } else {
        parse_json_or_wrap(&body_text)
    };

    let protocol_error = request
        .is_codex()
        .then(|| codex_stream_error(&body_text))
        .flatten();
    if !status_code.is_success() || protocol_error.is_some() {
        return ConnectivityTestResult {
            model_id: model_id.to_string(),
            status: "error".to_string(),
            first_byte_ms,
            total_ms: Some(total_ms),
            error_message: Some(if status_code.is_success() {
                protocol_error.unwrap()
            } else {
                format!("API error: {}", status_code)
            }),
            request_url: url,
            request_headers: request_headers_value,
            request_body: request_body_value,
            response_headers: Some(response_headers_value),
            response_body: Some(response_body_value),
        };
    }

    ConnectivityTestResult {
        model_id: model_id.to_string(),
        status: "success".to_string(),
        first_byte_ms,
        total_ms: Some(total_ms),
        error_message: None,
        request_url: url,
        request_headers: request_headers_value,
        request_body: request_body_value,
        response_headers: Some(response_headers_value),
        response_body: Some(response_body_value),
    }
}

#[tauri::command]
pub async fn test_provider_model_connectivity(
    state: tauri::State<'_, SqliteDbState>,
    request: ConnectivityTestRequest,
) -> Result<ConnectivityTestResponse, String> {
    let timeout_secs = request.timeout_secs.unwrap_or(30);
    let client = http_client::client_with_timeout(&state, timeout_secs).await?;
    let mut request = request;
    let (api_key, headers) = resolve_credentials(
        &state,
        request.provider_id.as_deref(),
        request.api_key.as_deref(),
        request.headers.as_ref(),
        request.config_value_mode,
    )
    .await?;

    let resolved_request = resolve_provider_request(
        request.provider_id.as_deref(),
        &request.base_url,
        api_key.as_deref(),
        stored_credential_fallback_allowed(request.config_value_mode, request.api_key.as_deref()),
    );
    request.base_url = resolved_request.base_url;
    request.api_key = resolved_request.api_key;
    request.headers = headers;

    let mut results = Vec::new();
    for model_id in &request.model_ids {
        if request.base_url.trim().is_empty() {
            results.push(ConnectivityTestResult {
                model_id: model_id.clone(),
                status: "error".to_string(),
                first_byte_ms: None,
                total_ms: None,
                error_message: Some("Missing Base URL".to_string()),
                request_url: String::new(),
                request_headers: json!({}),
                request_body: json!({}),
                response_headers: None,
                response_body: None,
            });
            continue;
        }

        let result = run_connectivity_test_for_model(&client, &request, model_id).await;
        results.push(result);
    }

    Ok(ConnectivityTestResponse { results })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_connectivity_endpoint_preserves_the_configured_prefix() {
        for base in [
            "https://example.com/backend-api",
            "https://example.com/backend-api/codex/",
            "https://example.com/backend-api/codex/responses",
        ] {
            assert_eq!(
                build_codex_connectivity_url(base),
                "https://example.com/backend-api/codex/responses"
            );
        }
        assert_eq!(
            build_codex_connectivity_url("https://example.com/proxy?tenant=test"),
            "https://example.com/proxy/codex/responses?tenant=test"
        );
        assert!(codex_account_id("third-party-api-key").is_none());
    }

    #[test]
    fn codex_diagnostics_parse_complete_multiline_sse_frames() {
        assert_eq!(codex_stream_error("event: response.completed\r\ndata: {\r\ndata: \"type\": \"response.completed\",\r\ndata: \"error\": null\r\ndata: }\r\n\r\n"), None);
        assert!(codex_stream_error("data: {\"type\":\"response.completed\"}\n").is_some());
        assert!(codex_stream_error("data: {\"type\":\"response.incomplete\"}\n\n").is_some());
    }

    #[tokio::test]
    async fn codex_connectivity_sends_native_requests_and_requires_a_completed_stream() {
        use base64::Engine;
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        let claims =
            json!({ "https://api.openai.com/auth": { "chatgpt_account_id": "test-account" } });
        let token = format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        for (response, expected_status) in [
            ("data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n", "success"),
            ("data: {\"type\":\"error\",\"error\":{\"message\":\"rejected\"}}\n\n", "error"),
            ("data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n", "error"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let upstream = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                let header_end = loop {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") { break index + 4; }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length: usize = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(str::trim).map(str::to_string)).unwrap().parse().unwrap();
                while bytes.len() < header_end + length {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let body: Value = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
                (headers, body)
            });
            let request: ConnectivityTestRequest = serde_json::from_value(json!({
                "npm": "@ai-sdk/openai-compatible", "apiFormat": "openai-codex-responses",
                "baseUrl": format!("http://{address}/backend-api"), "apiKey": token,
                "prompt": "probe", "modelIds": ["test-model"], "stream": false,
                "temperature": 1, "maxTokens": 100,
                "headers": { "x-review": "preserved" },
                "body": { "instructions": "Custom instructions", "store": true, "stream": false },
            })).unwrap();
            let client = http_client::create_client_no_proxy(5).unwrap();
            let result = run_connectivity_test_for_model(&client, &request, "test-model").await;
            let (headers, body) = upstream.join().unwrap();
            assert!(headers.starts_with("POST /backend-api/codex/responses "));
            assert!(headers.to_ascii_lowercase().contains("chatgpt-account-id: test-account"));
            assert!(headers.to_ascii_lowercase().contains("x-review: preserved"));
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["store"], false);
            assert_eq!(body["stream"], true);
            assert_eq!(body["instructions"], "Custom instructions");
            assert_eq!(body["input"][1]["content"][0]["text"], "probe");
            assert!(body.get("messages").is_none());
            assert!(body.get("temperature").is_none());
            assert!(body.get("max_output_tokens").is_none());
            assert_eq!(result.status, expected_status, "{:?}", result.error_message);
        }
    }

    #[test]
    fn empty_credentials_do_not_need_config_value_resolution() {
        assert!(!has_config_values(None, None));
        assert!(!has_config_values(None, Some(&json!({}))));
        // The pages send an empty string when a provider has no credential.
        assert!(!has_config_values(Some(""), None));
        assert!(!has_config_values(Some("   "), Some(&json!({}))));
        assert!(has_config_values(Some("sk-live"), None));
        assert!(has_config_values(None, Some(&json!({ "X-Test": "1" }))));
    }

    #[test]
    fn only_pi_and_omp_modes_resolve_config_values() {
        assert!(!resolves_config_values(None));
        assert!(resolves_config_values(Some(ConfigValueMode::Pi)));
        assert!(resolves_config_values(Some(ConfigValueMode::Omp)));
    }

    #[test]
    fn omp_mode_does_not_borrow_another_tools_stored_credential() {
        // Non-mode callers and Pi keep the provider credential fallback.
        assert!(stored_credential_fallback_allowed(None, Some("sk-live")));
        assert!(stored_credential_fallback_allowed(
            Some(ConfigValueMode::Pi),
            Some("sk-live")
        ));

        // An OMP provider that configured its own apiKey owns the result, even
        // when the OMP runtime omits it.
        assert!(!stored_credential_fallback_allowed(
            Some(ConfigValueMode::Omp),
            Some("!vault read omp-key")
        ));

        // Without a configured key the provider fallback still applies, and a
        // blank key is the pages' "no credential" value.
        assert!(stored_credential_fallback_allowed(
            Some(ConfigValueMode::Omp),
            None
        ));
        assert!(stored_credential_fallback_allowed(
            Some(ConfigValueMode::Omp),
            Some("")
        ));
    }

    #[test]
    fn google_native_discovery_key_is_appended_only_when_absent() {
        assert_eq!(
            append_google_native_key(
                "https://g.example/v1beta/models".to_string(),
                Some("sk-live")
            ),
            "https://g.example/v1beta/models?key=sk-live"
        );
        assert_eq!(
            append_google_native_key("https://g.example/v1beta/models".to_string(), None),
            "https://g.example/v1beta/models"
        );
        assert_eq!(
            append_google_native_key("https://g.example/v1beta/models".to_string(), Some("")),
            "https://g.example/v1beta/models"
        );
        assert_eq!(
            append_google_native_key(
                "https://g.example/v1beta/models?key=stored".to_string(),
                Some("sk-live")
            ),
            "https://g.example/v1beta/models?key=stored"
        );
        assert_eq!(
            append_google_native_key(
                "https://g.example/v1beta/models?tenant=acme".to_string(),
                Some("sk-live")
            ),
            "https://g.example/v1beta/models?tenant=acme&key=sk-live"
        );
    }

    #[test]
    fn config_value_mode_stays_optional_for_existing_callers() {
        let request: FetchModelsRequest = serde_json::from_value(json!({
            "baseUrl": "https://api.example.com",
            "apiType": "openai_compat",
        }))
        .expect("requests without a config value mode must keep deserializing");
        assert_eq!(request.config_value_mode, None);

        let request: FetchModelsRequest = serde_json::from_value(json!({
            "baseUrl": "https://api.example.com",
            "apiType": "openai_compat",
            "configValueMode": "pi",
        }))
        .expect("the pi config value mode must deserialize");
        assert_eq!(request.config_value_mode, Some(ConfigValueMode::Pi));

        let request: FetchModelsRequest = serde_json::from_value(json!({
            "baseUrl": "https://api.example.com",
            "apiType": "openai_compat",
            "configValueMode": "omp",
        }))
        .expect("the omp config value mode must deserialize");
        assert_eq!(request.config_value_mode, Some(ConfigValueMode::Omp));

        let request: ConnectivityTestRequest = serde_json::from_value(json!({
            "npm": "@ai-sdk/openai-compatible",
            "baseUrl": "https://api.example.com",
            "prompt": "probe",
            "modelIds": ["model-a"],
            "configValueMode": "omp",
        }))
        .expect("connectivity requests must accept the omp config value mode");
        assert_eq!(request.config_value_mode, Some(ConfigValueMode::Omp));
    }

    #[test]
    fn test_build_models_url_openai_compat() {
        // Base URL without version
        assert_eq!(
            build_models_url("https://api.openai.com", &ApiType::OpenaiCompat, None, None),
            "https://api.openai.com/models"
        );

        // Base URL with /v1
        assert_eq!(
            build_models_url(
                "https://api.openai.com/v1",
                &ApiType::OpenaiCompat,
                None,
                None
            ),
            "https://api.openai.com/v1/models"
        );

        // Base URL with trailing slash
        assert_eq!(
            build_models_url(
                "https://api.openai.com/v1/",
                &ApiType::OpenaiCompat,
                None,
                None
            ),
            "https://api.openai.com/v1/models"
        );

        // Base URL with /v1beta (Google style) should keep as-is
        assert_eq!(
            build_models_url(
                "https://generativelanguage.googleapis.com/v1beta",
                &ApiType::OpenaiCompat,
                None,
                None
            ),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn test_build_models_url_native_google() {
        // Google Native model listing uses Gemini API v1beta by default when no version is present.
        assert_eq!(
            build_models_url(
                "https://generativelanguage.googleapis.com",
                &ApiType::Native,
                Some("@ai-sdk/google"),
                None
            ),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );

        // Custom Gemini API gateways commonly expose the same root URL that Gemini CLI stores in GOOGLE_GEMINI_BASE_URL.
        assert_eq!(
            build_models_url(
                "https://gemini.example.com",
                &ApiType::Native,
                Some("@ai-sdk/google"),
                Some("test-api-key")
            ),
            "https://gemini.example.com/v1beta/models?key=test-api-key"
        );

        // Google Native with /v1beta
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
                "https://generativelanguage.googleapis.com/v1beta",
                &ApiType::Native,
                Some("@ai-sdk/google"),
                Some("test-api-key")
            ),
            "https://generativelanguage.googleapis.com/v1beta/models?key=test-api-key"
        );

        // Google Native with explicit /v1 should keep the caller-provided version.
        assert_eq!(
            build_models_url(
                "https://generativelanguage.googleapis.com/v1",
                &ApiType::Native,
                Some("@ai-sdk/google"),
                Some("test-api-key")
            ),
            "https://generativelanguage.googleapis.com/v1/models?key=test-api-key"
        );
    }

    #[test]
    fn test_build_models_url_native_anthropic() {
        // Anthropic Native with /v1
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
    fn test_parse_anthropic_models_response() {
        let models = parse_anthropic_models_response(
            r#"{
                "data": [{
                    "id": "claude-sonnet-4-6",
                    "type": "model",
                    "display_name": "Claude Sonnet 4.6"
                }]
            }"#,
        )
        .expect("Anthropic model list should parse");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude-sonnet-4-6");
        assert_eq!(models[0].name.as_deref(), Some("Claude Sonnet 4.6"));
        assert_eq!(models[0].owned_by.as_deref(), Some("anthropic"));
    }

    #[test]
    fn test_parse_openai_compatible_models_response_for_anthropic_gateway() {
        let models = parse_anthropic_models_response(
            r#"{
                "object": "list",
                "data": [{
                    "id": "claude-sonnet-4-6",
                    "object": "model",
                    "created": 1626777600,
                    "owned_by": "claude"
                }]
            }"#,
        )
        .expect("OpenAI-compatible model list should parse");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude-sonnet-4-6");
        assert_eq!(models[0].name.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(models[0].owned_by.as_deref(), Some("claude"));
        assert_eq!(models[0].created, Some(1626777600));
    }

    #[test]
    fn test_parse_codex_catalog_models_response() {
        // Zhipu GLM `/api/v1/models` and Codex `models.json` share the `slug` schema.
        let models = parse_generic_models_response(
            r#"{
                "models": [{
                    "slug": "glm-5.3",
                    "display_name": "GLM-5.3",
                    "apply_patch_tool_type": "freeform",
                    "context_window": 1048576
                }, {
                    "slug": "glm-5-turbo"
                }, {
                    "slug": "   "
                }]
            }"#,
        )
        .expect("Codex catalog model list should parse");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "glm-5.3");
        assert_eq!(models[0].name.as_deref(), Some("GLM-5.3"));
        assert_eq!(models[1].id, "glm-5-turbo");
        assert_eq!(models[1].name.as_deref(), Some("glm-5-turbo"));
    }

    #[test]
    fn test_generic_models_response_keeps_openai_and_google_schemas() {
        let openai_models = parse_generic_models_response(
            r#"{"object":"list","data":[{"id":"gpt-4o","owned_by":"openai","created":1626777600}]}"#,
        )
        .expect("OpenAI model list should parse");
        assert_eq!(openai_models[0].id, "gpt-4o");
        assert_eq!(openai_models[0].owned_by.as_deref(), Some("openai"));
        assert_eq!(openai_models[0].created, Some(1626777600));

        let google_models = parse_generic_models_response(
            r#"{"models":[{"name":"models/gemini-2.5-pro","displayName":"Gemini 2.5 Pro"}]}"#,
        )
        .expect("Google model list should parse");
        assert_eq!(google_models[0].id, "gemini-2.5-pro");
        assert_eq!(google_models[0].name.as_deref(), Some("Gemini 2.5 Pro"));

        let error = parse_generic_models_response(r#"{"foo":"bar"}"#).expect_err("unknown schema");
        assert!(error.contains("Failed to parse models response"));
    }

    #[test]
    fn test_build_models_url_native_fallback() {
        // Unknown SDK type falls back to /models
        assert_eq!(
            build_models_url(
                "https://api.example.com/v1",
                &ApiType::Native,
                Some("@ai-sdk/unknown"),
                None
            ),
            "https://api.example.com/v1/models"
        );
    }
}
