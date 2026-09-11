//! Portable provider connection data. This is configuration adaptation, not
//! request/response protocol conversion (which remains owned by Gateway).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::coding::proxy_gateway::types::{CustomHeaderOverride, GatewayProviderProfileReference};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SharedConnection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url_style: Option<SharedBaseUrlStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<SharedModel>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub model_roles: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<CustomHeaderOverride>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_profile: Option<GatewayProviderProfileReference>,
    /// Only explicit legacy metadata, never a snapshot of a profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedBaseUrlStyle {
    Root,
    Versioned,
}

pub(crate) fn string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| value.get(*key).and_then(string))
}

pub(crate) fn canonical_api_format(value: &str) -> Option<&'static str> {
    match value
        .trim()
        .to_lowercase()
        .replace(['-', '/'], "_")
        .as_str()
    {
        "anthropic" | "anthropic_messages" | "claude" | "messages" => Some("anthropic_messages"),
        "openai" | "openai_chat" | "openai_completions" | "chat" | "chat_completions" => {
            Some("openai_chat")
        }
        "openai_responses" | "responses" => Some("openai_responses"),
        "google" | "google_generative_ai" | "gemini" | "gemini_native" => Some("gemini_native"),
        "ollama" | "ollama_chat" => Some("ollama/chat"),
        _ => None,
    }
}

pub(crate) fn native_format(app: &str) -> &'static str {
    match app {
        "claude" | "claudedesktop" => "anthropic_messages",
        "codex" => "openai_responses",
        "gemini" => "gemini_native",
        _ => "openai_chat",
    }
}

pub(crate) fn profile_tool(app: &str) -> Option<&'static str> {
    match app {
        "claude" => Some("claude"),
        "claudedesktop" => Some("claude_desktop"),
        "codex" => Some("codex"),
        "grok" => Some("grok"),
        "gemini" => Some("gemini"),
        _ => None,
    }
}

pub(crate) fn is_database_app(app: &str) -> bool {
    matches!(
        app,
        "claude" | "claudedesktop" | "codex" | "grok" | "kimi" | "gemini"
    )
}

pub(crate) fn catalog_profile<'a>(catalog: &'a Value, id: &str) -> Option<&'a Value> {
    catalog
        .get("profiles")?
        .as_array()?
        .iter()
        .find(|profile| profile["id"].as_str() == Some(id))
}

pub(crate) fn profile_endpoint<'a>(
    profile: &'a Value,
    tool: &str,
    endpoint: &str,
) -> Option<&'a Value> {
    profile
        .get("tools")?
        .get(tool)?
        .get("endpoints")?
        .as_array()?
        .iter()
        .find(|value| value["id"].as_str() == Some(endpoint))
}

/// Keep the exact endpoint identity where possible. Only a unique matching
/// protocol is an acceptable fallback; the user's Base URL is never changed.
pub(crate) fn remap_profile(
    reference: &GatewayProviderProfileReference,
    target_app: &str,
    api_format: &str,
    catalog: &Value,
) -> Option<GatewayProviderProfileReference> {
    let tool = profile_tool(target_app)?;
    let profile = catalog_profile(catalog, &reference.profile_id)?;
    let endpoints = profile
        .get("tools")?
        .get(tool)?
        .get("endpoints")?
        .as_array()?;
    let matching: Vec<_> = endpoints
        .iter()
        .filter(|endpoint| {
            endpoint["apiFormat"]
                .as_str()
                .and_then(canonical_api_format)
                == Some(api_format)
        })
        .collect();
    let endpoint = matching
        .iter()
        .find(|endpoint| endpoint["id"].as_str() == Some(&reference.endpoint_id))
        .copied()
        .or_else(|| (matching.len() == 1).then(|| matching[0]))?;
    Some(GatewayProviderProfileReference {
        tool: Some(tool.to_string()),
        profile_id: reference.profile_id.clone(),
        endpoint_id: endpoint["id"].as_str()?.to_string(),
    })
}

pub(crate) fn strip_context_marker(model: &str) -> String {
    let model = model.trim();
    if model.to_lowercase().ends_with("[1m]") {
        model[..model.len() - 4].to_string()
    } else {
        model.to_string()
    }
}

/// AI SDK providers include the version in their prefix; the native Anthropic
/// and Google SDKs add it themselves. Adapt only known terminal version paths,
/// keeping proxy prefixes, query strings and explicit full endpoints intact.
pub(crate) fn adapt_base_url(
    connection: &mut SharedConnection,
    target: &str,
    base: &str,
) -> String {
    let Some(source) = connection.source_app.as_deref() else {
        return base.to_string();
    };
    if source == target || base.ends_with("##") {
        return base.to_string();
    }
    let format = connection.api_format.as_deref().unwrap_or_default();
    let version = match format {
        "anthropic_messages" => "v1",
        "gemini_native" => connection.api_version.as_deref().unwrap_or("v1beta"),
        _ => return base.to_string(),
    };
    let suffix_start = base.find(['?', '#']).unwrap_or(base.len());
    let (path, suffix) = base.split_at(suffix_start);
    let path = path.trim_end_matches('/');
    let uses_versioned_prefix = |app: &str| {
        app == "opencode"
            || (is_database_app(app)
                && native_format(app) != format
                && !(app == "grok" && format == "anthropic_messages"))
    };
    let source_uses_versioned_prefix = match connection.base_url_style {
        Some(SharedBaseUrlStyle::Root) => false,
        Some(SharedBaseUrlStyle::Versioned) => true,
        None => uses_versioned_prefix(source),
    };
    let mut root = path.to_string();
    let mut selected_version = version.to_string();
    if source_uses_versioned_prefix {
        if let Some((prefix, last)) = path.rsplit_once('/') {
            let known_version =
                last == "v1" || (format == "gemini_native" && matches!(last, "v1alpha" | "v1beta"));
            if known_version {
                root = prefix.to_string();
                selected_version = last.to_string();
            }
        }
    }
    if format == "gemini_native" {
        connection.api_version = Some(selected_version.clone());
    }
    if uses_versioned_prefix(target) {
        format!("{root}/{selected_version}{suffix}")
    } else {
        format!("{root}{suffix}")
    }
}

pub(crate) fn uses_bearer_auth(field: Option<&str>) -> bool {
    field.is_some_and(|field| {
        matches!(
            field.to_ascii_lowercase().as_str(),
            "authorization" | "bearer" | "auth_token" | "anthropic_auth_token"
        )
    })
}

pub(crate) fn supports_native_auth(app: &str, format: &str, field: Option<&str>) -> bool {
    let Some(field) = field.filter(|field| !field.is_empty()) else {
        return true;
    };
    let bearer = uses_bearer_auth(Some(field));
    let field = field.to_ascii_lowercase();
    match format {
        "anthropic_messages" => {
            matches!(
                field.as_str(),
                "x-api-key" | "api_key" | "anthropic_api_key"
            ) || (bearer
                && matches!(
                    app,
                    "claude" | "claudedesktop" | "opencode" | "pi" | "omp" | "openclaw"
                ))
        }
        "gemini_native" => {
            matches!(
                field.as_str(),
                "x-goog-api-key" | "google_api_key" | "gemini_api_key"
            ) || (bearer && app == "gemini")
        }
        "openai_chat" | "openai_responses" => bearer,
        _ => false,
    }
}

pub(crate) fn model_catalog(models: &[SharedModel]) -> Value {
    Value::Array(
        models
            .iter()
            .map(|model| {
                let mut value = json!({ "model": model.id });
                if let Some(name) = &model.name {
                    value["displayName"] = json!(name);
                }
                if let Some(window) = model.context_window {
                    value["contextWindow"] = json!(window);
                }
                if let Some(input) = &model.input {
                    value["modalities"] = json!({ "input": input });
                }
                value
            })
            .collect(),
    )
}

pub(crate) fn static_headers(
    headers: &[CustomHeaderOverride],
) -> Result<Map<String, Value>, String> {
    let mut result = Map::new();
    for header in headers {
        if header.op != "set" {
            return Err("This tool only supports fixed request headers; use a Gateway-enabled tool for delete/rename/copy header rules".to_string());
        }
        result.insert(header.name.clone(), json!(header.value));
    }
    Ok(result)
}
