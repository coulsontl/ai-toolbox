//! Per-tool builders for portable provider imports.

use serde_json::{json, Map, Value};

use super::parser::DeepLinkImportRequest;
use super::portable::{model_catalog, static_headers, uses_bearer_auth};

/// `{"env": {ANTHROPIC_AUTH_TOKEN?, ANTHROPIC_BASE_URL?, ANTHROPIC_MODEL?}}`.
/// If `config` is provided, it overrides the built `settings_config` verbatim.
/// `extra` (decoded) becomes `extra_settings_config`, defaulting to `"{}"`.
pub(super) fn build_claude_settings(
    request: &DeepLinkImportRequest,
) -> Result<(String, Option<String>), String> {
    let settings_config = match request.config.as_deref() {
        Some(cfg) if !cfg.trim().is_empty() => cfg.to_string(),
        _ => {
            let mut env = Map::new();
            if let Some(api_key) = &request.api_key {
                env.insert(
                    if request
                        .connection
                        .api_key_field
                        .as_deref()
                        .is_some_and(|field| {
                            matches!(
                                field.to_ascii_lowercase().as_str(),
                                "x-api-key" | "api_key" | "anthropic_api_key"
                            )
                        })
                    {
                        "ANTHROPIC_API_KEY"
                    } else {
                        "ANTHROPIC_AUTH_TOKEN"
                    }
                    .to_string(),
                    Value::String(api_key.clone()),
                );
            }
            if let Some(base_url) = &request.base_url {
                env.insert(
                    "ANTHROPIC_BASE_URL".to_string(),
                    Value::String(base_url.clone()),
                );
            }
            if let Some(model) = &request.model {
                env.insert("ANTHROPIC_MODEL".to_string(), Value::String(model.clone()));
            }
            if request.extra.is_none() {
                let fallback = if request.connection.model_roles.is_empty() {
                    request.model.as_ref()
                } else {
                    None
                };
                for role in ["haiku", "sonnet", "opus", "fable"] {
                    if let Some(model) = request.connection.model_roles.get(role).or(fallback) {
                        env.insert(
                            format!("ANTHROPIC_DEFAULT_{}_MODEL", role.to_uppercase()),
                            json!(model),
                        );
                    }
                }
            }
            serde_json::to_string(&json!({ "env": Value::Object(env) }))
                .map_err(|e| format!("deep-link: failed to serialize claude settings: {e}"))?
        }
    };

    let extra_settings_config = match request.extra.as_deref() {
        Some(extra) if !extra.trim().is_empty() => Some(extra.to_string()),
        _ => Some("{}".to_string()),
    };

    Ok((settings_config, extra_settings_config))
}

/// `{"auth": {"OPENAI_API_KEY"?}, "config": "<TOML string>"}`.
/// The TOML contains `model_provider = "<slug>"`, optional `model`, and a
/// `[model_providers.<slug>]` table with `name`/`base_url`. If `config` is
/// provided, it is used as the TOML `config` string verbatim.
pub(super) fn build_codex_settings(request: &DeepLinkImportRequest) -> Result<String, String> {
    let slug = slugify(&request.name);
    let slug = if matches!(slug.as_str(), "openai" | "ollama" | "lmstudio") {
        format!("shared-{slug}")
    } else {
        slug
    };

    let config_toml = match request.config.as_deref() {
        Some(cfg) if !cfg.trim().is_empty() => cfg.to_string(),
        _ => {
            let mut root = toml::map::Map::new();
            root.insert(
                "model_provider".to_string(),
                toml::Value::String(slug.clone()),
            );
            if let Some(model) = &request.model {
                root.insert("model".to_string(), toml::Value::String(model.clone()));
            }

            let mut provider_table = toml::map::Map::new();
            provider_table.insert(
                "wire_api".to_string(),
                toml::Value::String("responses".to_string()),
            );
            provider_table.insert(
                "name".to_string(),
                toml::Value::String(request.name.clone()),
            );
            if let Some(base_url) = &request.base_url {
                provider_table.insert(
                    "base_url".to_string(),
                    toml::Value::String(base_url.clone()),
                );
            }
            let mut model_providers = toml::map::Map::new();
            model_providers.insert(slug, toml::Value::Table(provider_table));
            root.insert(
                "model_providers".to_string(),
                toml::Value::Table(model_providers),
            );

            toml::to_string(&toml::Value::Table(root))
                .map_err(|e| format!("deep-link: failed to serialize codex TOML: {e}"))?
        }
    };

    let mut auth = Map::new();
    if let Some(api_key) = &request.api_key {
        auth.insert("OPENAI_API_KEY".to_string(), Value::String(api_key.clone()));
    }

    let mut settings = json!({
        "auth": Value::Object(auth),
        "config": config_toml,
    });
    if !request.connection.models.is_empty() {
        settings["modelCatalog"] = json!({ "models": model_catalog(&request.connection.models) });
    }
    serde_json::to_string(&settings)
        .map_err(|e| format!("deep-link: failed to serialize codex settings: {e}"))
}

/// `{"env": {GEMINI_API_KEY?, GOOGLE_GEMINI_BASE_URL?, GEMINI_MODEL?}, "config": {}}`.
/// If `config` is provided, it overrides the built `settings_config` verbatim.
pub(super) fn build_gemini_settings(request: &DeepLinkImportRequest) -> Result<String, String> {
    let settings_config = match request.config.as_deref() {
        Some(cfg) if !cfg.trim().is_empty() => cfg.to_string(),
        _ => {
            let mut env = Map::new();
            if let Some(api_key) = &request.api_key {
                env.insert("GEMINI_API_KEY".to_string(), Value::String(api_key.clone()));
            }
            if let Some(base_url) = &request.base_url {
                env.insert(
                    "GOOGLE_GEMINI_BASE_URL".to_string(),
                    Value::String(base_url.clone()),
                );
            }
            if let Some(model) = &request.model {
                env.insert("GEMINI_MODEL".to_string(), Value::String(model.clone()));
            }
            if let Some(version) = &request.connection.api_version {
                env.insert("GOOGLE_GENAI_API_VERSION".to_string(), json!(version));
            }
            if uses_bearer_auth(request.connection.api_key_field.as_deref()) {
                env.insert("GEMINI_API_KEY_AUTH_MECHANISM".to_string(), json!("bearer"));
            }
            serde_json::to_string(&json!({
                "env": Value::Object(env),
                "config": Value::Object(Map::new()),
            }))
            .map_err(|e| format!("deep-link: failed to serialize gemini settings: {e}"))?
        }
    };
    Ok(settings_config)
}

pub(super) fn build_desktop_routes(request: &DeepLinkImportRequest) -> Value {
    let mut routes = Map::new();
    for (index, model) in request.connection.models.iter().enumerate() {
        let id = if crate::coding::claude_desktop::config_writer::is_claude_safe_model_id(&model.id)
        {
            model.id.clone()
        } else {
            format!("claude-sonnet-share-{}", index + 1)
        };
        let mut route =
            json!({ "model": model.id, "labelOverride": model.name.as_ref().unwrap_or(&model.id) });
        let role = request
            .connection
            .model_roles
            .iter()
            .find_map(|(role, id)| (id == &model.id).then_some(role.as_str()))
            .or_else(|| (request.model.as_ref() == Some(&model.id)).then_some("sonnet"));
        if let Some(role) = role {
            route["tierAlias"] = json!(role);
        }
        routes.insert(id, route);
    }
    Value::Object(routes)
}

/// Lowercase, replace non-[a-z0-9] runs with `-`, trim leading/trailing `-`.
/// Used as the codex `model_provider` id and `[model_providers.<id>]` key.
pub(super) fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = true; // suppress leading dashes
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "provider".to_string()
    } else {
        slug
    }
}

pub(super) fn build_catalog_settings(request: &DeepLinkImportRequest) -> Result<String, String> {
    let format = request
        .connection
        .api_format
        .as_deref()
        .unwrap_or("openai_chat");
    let models: Vec<Value> = request
        .connection
        .models
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let key = format!("model-{}", index + 1);
            let mut entry = json!({ "key": key, "model": model.id });
            if let Some(name) = &model.name {
                entry["displayName"] = json!(name);
            }
            if request.app == "kimi" {
                entry["provider"] = json!("custom");
                entry["maxContextSize"] = json!(model.context_window.unwrap_or(262_144));
                let mut capabilities = Vec::new();
                if let Some(input) = &model.input {
                    if input.iter().any(|kind| kind == "image") {
                        capabilities.push("image_in");
                    }
                    if input.iter().any(|kind| kind == "video") {
                        capabilities.push("video_in");
                    }
                }
                if model.reasoning == Some(true) {
                    capabilities.push("thinking");
                }
                if !capabilities.is_empty() {
                    entry["capabilities"] = json!(capabilities);
                }
            } else {
                entry["apiBackend"] = json!(match format {
                    "openai_responses" => "responses",
                    "anthropic_messages" => "messages",
                    _ => "chat_completions",
                });
                if let Some(url) = &request.base_url {
                    entry["baseUrl"] = json!(url);
                }
                if let Some(window) = model.context_window {
                    entry["contextWindow"] = json!(window);
                }
                if let Some(input) = &model.input {
                    entry["modalities"] = json!({ "input": input });
                }
            }
            entry
        })
        .collect();
    let default_key = request
        .model
        .as_ref()
        .and_then(|default| {
            models
                .iter()
                .find(|model| model["model"].as_str() == Some(default))
        })
        .or_else(|| models.first())
        .and_then(|model| model["key"].as_str());
    let mut settings = json!({ "auth": {}, "modelCatalog": { "models": models } });
    if let Some(key) = &request.api_key {
        settings["auth"]["API_KEY"] = json!(key);
    }
    if let Some(key) = default_key {
        settings["defaultModelKey"] = json!(key);
    }
    if request.app == "kimi" {
        settings["providerConfigs"] = json!({ "custom": { "type": "openai_legacy" } });
        if let Some(url) = &request.base_url {
            settings["providerConfigs"]["custom"]["base_url"] = json!(url);
        }
    }
    serde_json::to_string(&settings).map_err(|error| error.to_string())
}

pub(super) fn build_native_provider(request: &DeepLinkImportRequest) -> Result<Value, String> {
    let format = request
        .connection
        .api_format
        .as_deref()
        .unwrap_or("openai_chat");
    let api = match format {
        "anthropic_messages" => "anthropic-messages",
        "openai_responses" => "openai-responses",
        "gemini_native" => "google-generative-ai",
        _ => "openai-completions",
    };
    let headers = static_headers(&request.connection.headers)?;
    if request.app == "opencode" {
        let npm = match format {
            "anthropic_messages" => "@ai-sdk/anthropic",
            "openai_responses" => "@ai-sdk/openai",
            "gemini_native" => "@ai-sdk/google",
            _ => "@ai-sdk/openai-compatible",
        };
        let mut provider = json!({ "name": request.name, "npm": npm, "options": {}, "models": {} });
        if let Some(url) = &request.base_url {
            provider["options"]["baseURL"] = json!(url);
        }
        if let Some(key) = &request.api_key {
            let field = if format == "anthropic_messages"
                && uses_bearer_auth(request.connection.api_key_field.as_deref())
            {
                "authToken"
            } else {
                "apiKey"
            };
            provider["options"][field] = json!(key);
        }
        if !headers.is_empty() {
            provider["options"]["headers"] = json!(headers);
        }
        for model in &request.connection.models {
            let mut entry = json!({ "name": model.name.as_ref().unwrap_or(&model.id) });
            if let (Some(context), Some(output)) = (model.context_window, model.max_tokens) {
                entry["limit"] = json!({ "context": context, "output": output });
            }
            if let Some(input) = &model.input {
                entry["modalities"] = json!({ "input": input, "output": ["text"] });
            }
            if let Some(reasoning) = model.reasoning {
                entry["reasoning"] = json!(reasoning);
            }
            provider["models"][&model.id] = entry;
        }
        return Ok(provider);
    }
    if request.app == "hermes" {
        let mode = match format {
            "anthropic_messages" => "anthropic",
            "openai_responses" => "openai-responses",
            "gemini_native" => "google",
            _ => "openai",
        };
        let mut models = request.connection.models.clone();
        if let Some(default) = &request.model {
            if let Some(index) = models.iter().position(|model| &model.id == default) {
                let model = models.remove(index);
                models.insert(0, model);
            }
        }
        let models: Vec<Value> = models
            .iter()
            .map(|model| {
                let mut entry = json!({ "id": model.id });
                if let Some(context) = model.context_window {
                    entry["context_length"] = json!(context);
                }
                if let Some(tokens) = model.max_tokens {
                    entry["max_tokens"] = json!(tokens);
                }
                if let Some(name) = &model.name {
                    entry["name"] = json!(name);
                }
                entry
            })
            .collect();
        let mut provider =
            json!({ "api_mode": mode, "models": models, "display_name": request.name });
        if let Some(url) = &request.base_url {
            provider["base_url"] = json!(url);
        }
        if let Some(key) = &request.api_key {
            provider["api_key"] = json!(key);
        }
        return Ok(provider);
    }
    let mut models = request.connection.models.clone();
    for model in &mut models {
        if let Some(input) = &mut model.input {
            input.retain(|kind| matches!(kind.as_str(), "text" | "image"));
            if input.is_empty() {
                model.input = None;
            }
        }
    }
    let mut provider = json!({ "api": api, "models": models });
    if let Some(url) = &request.base_url {
        provider[if request.app == "dsh" {
            "baseURL"
        } else {
            "baseUrl"
        }] = json!(url);
    }
    if request.app != "dsh" {
        if let Some(key) = &request.api_key {
            provider["apiKey"] = json!(key);
        }
    }
    if !headers.is_empty() {
        provider["headers"] = json!(headers);
    }
    if format == "anthropic_messages"
        && uses_bearer_auth(request.connection.api_key_field.as_deref())
    {
        provider["authHeader"] = json!(true);
    }
    if request.app == "dsh" {
        provider["displayName"] = json!(request.name);
    }
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::deeplink::parser::parse_deeplink_url;

    fn req_for(app: &str) -> DeepLinkImportRequest {
        parse_deeplink_url(&format!(
            "aitoolbox://v1/import?resource=provider&app={app}&name=My%20Provider&category=custom&apiKey=sk-x&baseUrl=https%3A%2F%2Fapi.example.com&model=m1"
        ))
        .unwrap()
    }

    #[test]
    fn claude_env_shape() {
        let req = req_for("claude");
        let (settings, extra) = build_claude_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-x");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://api.example.com");
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "m1");
        assert_eq!(extra.as_deref(), Some("{}"));
    }

    #[test]
    fn claude_config_override() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let cfg = URL_SAFE_NO_PAD.encode(r#"{"env":{"X":"1"}}"#);
        let req = parse_deeplink_url(&format!(
            "aitoolbox://v1/import?resource=provider&app=claude&name=T&category=custom&config={cfg}"
        ))
        .unwrap();
        let (settings, _) = build_claude_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(v["env"]["X"], "1");
        assert!(v["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn codex_toml_shape() {
        let req = req_for("codex");
        let settings = build_codex_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(v["auth"]["OPENAI_API_KEY"], "sk-x");
        let config = v["config"].as_str().unwrap();
        assert!(config.contains(r#"model_provider = "my-provider""#));
        assert!(config.contains("model = \"m1\""));
        // `my-provider` is a valid TOML bare key (dashes allowed), so the
        // table header is emitted without quotes.
        assert!(config.contains("[model_providers.my-provider]"));
        assert!(config.contains("base_url = \"https://api.example.com\""));
        assert!(config.contains("name = \"My Provider\""));
    }

    #[test]
    fn gemini_env_shape() {
        let req = req_for("gemini");
        let settings = build_gemini_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(v["env"]["GEMINI_API_KEY"], "sk-x");
        assert_eq!(
            v["env"]["GOOGLE_GEMINI_BASE_URL"],
            "https://api.example.com"
        );
        assert_eq!(v["env"]["GEMINI_MODEL"], "m1");
        assert!(v["config"].is_object());
    }

    #[test]
    fn slugify_handles_names() {
        assert_eq!(slugify("OpenRouter"), "openrouter");
        assert_eq!(slugify("My Cool API!"), "my-cool-api");
        assert_eq!(slugify("---"), "provider");
        assert_eq!(slugify("  spaces  "), "spaces");
    }

    #[test]
    fn codex_config_override_uses_verbatim_toml() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let custom_toml = r#"model_provider = "custom"
model = "override"
[model_providers.custom]
name = "Custom"
base_url = "https://override.example.com"
"#;
        let cfg = URL_SAFE_NO_PAD.encode(custom_toml);
        let req = parse_deeplink_url(&format!(
            "aitoolbox://v1/import?resource=provider&app=codex&name=X&category=custom&config={cfg}"
        ))
        .unwrap();
        let settings = build_codex_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        // When config is provided it is used verbatim as the TOML `config`
        // string; apiKey (none here) still drives auth but no slug-derived
        // provider block is synthesized.
        let config = v["config"].as_str().unwrap();
        assert!(config.contains("model_provider = \"custom\""));
        assert!(config.contains("model = \"override\""));
        assert!(
            !config.contains("[model_providers.x]"),
            "slug block must NOT be synthesized when config overrides"
        );
        assert!(v["auth"].is_object());
    }

    #[test]
    fn codex_without_model_omits_model_line() {
        // No `model` param: the TOML must keep model_providers but drop `model`.
        let req = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=codex&name=NoModel&category=custom&baseUrl=https%3A%2F%2Fx.com",
        )
        .unwrap();
        let settings = build_codex_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        let config = v["config"].as_str().unwrap();
        assert!(config.contains(r#"model_provider = "nomodel""#));
        assert!(!config.contains("model ="));
        assert!(config.contains("[model_providers.nomodel]"));
    }

    #[test]
    fn gemini_config_override_replaces_settings() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let cfg = URL_SAFE_NO_PAD.encode(r#"{"env":{"GEMINI_API_KEY":"custom-key"}}"#);
        let req = parse_deeplink_url(&format!(
            "aitoolbox://v1/import?resource=provider&app=gemini&name=X&category=custom&config={cfg}"
        ))
        .unwrap();
        let settings = build_gemini_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(v["env"]["GEMINI_API_KEY"], "custom-key");
        // The override replaces the whole settings_config; builder adds nothing.
        assert!(v.get("config").is_none());
    }

    #[test]
    fn claude_settings_omit_unset_fields() {
        // Only name+category: no apiKey/baseUrl/model, so env must be empty object.
        let req = parse_deeplink_url(
            "aitoolbox://v1/import?resource=provider&app=claude&name=Empty&category=official",
        )
        .unwrap();
        let (settings, extra) = build_claude_settings(&req).unwrap();
        let v: Value = serde_json::from_str(&settings).unwrap();
        assert!(v["env"].as_object().unwrap().is_empty());
        assert_eq!(extra.as_deref(), Some("{}"));
    }

    #[test]
    fn native_model_metadata_obeys_target_limit_and_modality_shapes() {
        let mut request = req_for("opencode");
        request.connection.api_format = Some("openai_chat".to_string());
        request.connection.models = serde_json::from_value(json!([
            { "id": "complete", "contextWindow": 64000, "maxTokens": 8000, "input": ["text", "image", "audio"] },
            { "id": "context-only", "contextWindow": 128000 },
            { "id": "output-only", "maxTokens": 16000 }
        ])).unwrap();
        let opencode = build_native_provider(&request).unwrap();
        assert_eq!(
            opencode["models"]["complete"]["limit"],
            json!({ "context": 64000, "output": 8000 })
        );
        assert_eq!(
            opencode["models"]["complete"]["modalities"]["output"],
            json!(["text"])
        );
        assert!(opencode["models"]["context-only"].get("limit").is_none());
        assert!(opencode["models"]["output-only"].get("limit").is_none());
        for target in ["pi", "omp", "openclaw", "dsh"] {
            request.app = target.to_string();
            let provider = build_native_provider(&request).unwrap();
            assert_eq!(
                provider["models"][0]["input"],
                json!(["text", "image"]),
                "{target}"
            );
            assert_eq!(provider["models"][1]["contextWindow"], 128000, "{target}");
        }
    }
}
