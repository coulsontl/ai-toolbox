use serde::{Deserialize, Serialize};

use crate::db::DbState;
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

// ============================================================================
// Connectivity Test Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityTestRequest {
    pub npm: String,
    pub base_url: String,
    pub api_key: Option<String>,
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

// ============================================================================
// Connectivity Test Command
// ============================================================================

fn normalize_base_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1beta") {
        base.trim_end_matches("/v1beta").to_string()
    } else if base.ends_with("/v1") {
        base.trim_end_matches("/v1").to_string()
    } else {
        base.to_string()
    }
}

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
    let base = normalize_base_url(base_url);
    match npm {
        "@ai-sdk/openai" => format!("{}/v1/chat/completions", base),
        "@ai-sdk/google" => {
            let normalized_model = model_id.strip_prefix("models/").unwrap_or(model_id);
            let action = if stream { "streamGenerateContent" } else { "generateContent" };
            let url = format!("{}/v1beta/models/{}:{}", base, normalized_model, action);
            if let Some(key) = api_key {
                if !key.is_empty() {
                    return format!("{}?key={}", url, key);
                }
            }
            url
        }
        "@ai-sdk/anthropic" => format!("{}/v1/messages", base),
        "@ai-sdk/openai-compatible" => format!("{}/v1/chat/completions", base),
        _ => format!("{}/v1/chat/completions", base),
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
    let stream_enabled = request.stream.unwrap_or(true);
    match request.npm.as_str() {
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
                "messages": [
            { "role": "user", "content": request.prompt }
        ],
                "stream": stream_enabled,
            });
            if let Some(temperature) = request.temperature {
                body["temperature"] = json!(temperature);
            }
            if let Some(max_tokens) = request.max_tokens {
                body["max_tokens"] = json!(max_tokens);
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
             body["messages"] = json!([
        { "role": "user", "content": prompt }
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
    let stream_enabled = request.stream.unwrap_or(true);
    let anthropic_user_id = if request.npm == "@ai-sdk/anthropic" {
        Some(generate_anthropic_user_id())
    } else {
        None
    };
    let url = build_connectivity_url(
        request.npm.as_str(),
        request.base_url.as_str(),
        model_id,
        request.api_key.as_deref(),
        stream_enabled,
    );

    let mut body = build_default_body(request, model_id, anthropic_user_id.as_deref());
    if let Some(custom_body) = &request.body {
        merge_json(&mut body, custom_body);
    }
    enforce_prompt_and_model(request.npm.as_str(), &mut body, model_id, &request.prompt);
    if let Some(user_id) = anthropic_user_id.as_deref() {
        ensure_anthropic_metadata(&mut body, user_id);
    }

    let mut req_builder = client.post(&url).json(&body);

    let is_google = request.npm == "@ai-sdk/google";
    let is_anthropic = request.npm == "@ai-sdk/anthropic";

    let mut request_headers = BTreeMap::new();
    if is_anthropic {
        request_headers.insert("Accept".to_string(), "application/json".to_string());
        request_headers.insert("Accept-Encoding".to_string(), "gzip, deflate, br, zstd".to_string());
        request_headers.insert("Connection".to_string(), "keep-alive".to_string());
        request_headers.insert("Content-Type".to_string(), "application/json".to_string());
        request_headers.insert("User-Agent".to_string(), "claude-cli/2.1.19 (external, cli)".to_string());
        request_headers.insert("anthropic-beta".to_string(), "interleaved-thinking-2025-05-14".to_string());
        request_headers.insert("anthropic-dangerous-direct-browser-access".to_string(), "true".to_string());
        request_headers.insert("anthropic-version".to_string(), "2023-06-01".to_string());
    }

    if is_anthropic {
        if let Some(api_key) = &request.api_key {
            if !api_key.is_empty() {
                request_headers.insert("X-Api-Key".to_string(), api_key.to_string());
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

    if !status_code.is_success() {
        return ConnectivityTestResult {
            model_id: model_id.to_string(),
            status: "error".to_string(),
            first_byte_ms,
            total_ms: Some(total_ms),
            error_message: Some(format!("API error: {}", status_code)),
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
    state: tauri::State<'_, DbState>,
    request: ConnectivityTestRequest,
) -> Result<ConnectivityTestResponse, String> {
    let timeout_secs = request.timeout_secs.unwrap_or(30);
    let client = http_client::client_with_timeout(&state, timeout_secs).await?;

    let mut results = Vec::new();
    for model_id in &request.model_ids {
        let result = run_connectivity_test_for_model(&client, &request, model_id).await;
        results.push(result);
    }

    Ok(ConnectivityTestResponse { results })
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
