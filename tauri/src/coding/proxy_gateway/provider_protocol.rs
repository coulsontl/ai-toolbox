use super::transformer::AiProtocol;
use super::types::GatewayCliKey;
use serde_json::Value;
use toml_edit::{DocumentMut, Item};

pub(crate) fn native_cli_protocol(cli_key: GatewayCliKey) -> Option<AiProtocol> {
    match cli_key {
        GatewayCliKey::Claude => Some(AiProtocol::AnthropicMessages),
        GatewayCliKey::Codex => Some(AiProtocol::OpenAiResponses),
        GatewayCliKey::Grok => Some(AiProtocol::OpenAiResponses),
        GatewayCliKey::Gemini => Some(AiProtocol::GeminiNative),
        GatewayCliKey::OpenCode => None,
    }
}

pub(crate) fn provider_needs_gateway_proxy(
    cli_key: GatewayCliKey,
    category: &str,
    meta: Option<&Value>,
    settings_config: &str,
) -> bool {
    if category.trim().eq_ignore_ascii_case("official") {
        return false;
    }

    // Grok CLI natively supports responses, chat_completions and messages, but
    // not Gemini Native. Gemini endpoints still require Gateway conversion.
    if cli_key == GatewayCliKey::Grok {
        return provider_target_protocol(cli_key, meta, settings_config)
            == AiProtocol::GeminiNative;
    }

    let Some(native_protocol) = native_cli_protocol(cli_key) else {
        return false;
    };
    provider_target_protocol(cli_key, meta, settings_config) != native_protocol
}

fn provider_target_protocol(
    cli_key: GatewayCliKey,
    meta: Option<&Value>,
    settings_config: &str,
) -> AiProtocol {
    let settings = serde_json::from_str::<Value>(settings_config).unwrap_or(Value::Null);
    match cli_key {
        GatewayCliKey::Claude => protocol_from_meta_or_settings(meta, &settings)
            .or_else(|| {
                settings
                    .get("openrouter_compat_mode")
                    .and_then(json_bool_value)
                    .filter(|enabled| *enabled)
                    .map(|_| AiProtocol::OpenAiChat)
            })
            .unwrap_or(AiProtocol::AnthropicMessages),
        GatewayCliKey::Codex => protocol_from_meta_or_settings(meta, &settings)
            .or_else(|| {
                settings
                    .get("config")
                    .and_then(Value::as_str)
                    .and_then(codex_wire_api_from_config)
                    .and_then(|value| AiProtocol::from_api_format(&value))
            })
            .unwrap_or_else(|| {
                let base_url = settings
                    .get("config")
                    .and_then(Value::as_str)
                    .and_then(codex_base_url_from_config);
                if base_url.as_deref().is_some_and(is_chat_completions_url) {
                    AiProtocol::OpenAiChat
                } else {
                    AiProtocol::OpenAiResponses
                }
            }),
        GatewayCliKey::Grok => protocol_from_meta_or_settings(meta, &settings)
            .or_else(|| {
                settings
                    .get("config")
                    .and_then(Value::as_str)
                    .and_then(grok_api_backend_from_config)
                    .and_then(|value| AiProtocol::from_api_format(&value))
            })
            .unwrap_or(AiProtocol::OpenAiChat),
        GatewayCliKey::Gemini => {
            protocol_from_meta_or_settings(meta, &settings).unwrap_or(AiProtocol::GeminiNative)
        }
        GatewayCliKey::OpenCode => AiProtocol::OpenAiResponses,
    }
}

pub(crate) fn grok_api_backend_from_config(config_toml: &str) -> Option<String> {
    let document = config_toml.trim().parse::<DocumentMut>().ok()?;
    let root = document.as_table();
    let default_model = root
        .get("models")
        .and_then(Item::as_table)
        .and_then(|models| toml_string(models, "default"));
    let model_tables = root.get("model").and_then(Item::as_table)?;
    default_model
        .as_deref()
        .and_then(|key| model_tables.get(key))
        .and_then(Item::as_table)
        .and_then(|model| toml_string(model, "api_backend"))
        .or_else(|| {
            model_tables.iter().find_map(|(_, item)| {
                item.as_table()
                    .and_then(|model| toml_string(model, "api_backend"))
            })
        })
}

fn protocol_from_meta_or_settings(meta: Option<&Value>, settings: &Value) -> Option<AiProtocol> {
    meta.and_then(|value| json_string_compat(value, "api_format", "apiFormat"))
        .or_else(|| json_value_string(settings, "api_format"))
        .or_else(|| json_value_string(settings, "apiFormat"))
        .and_then(|value| AiProtocol::from_api_format(&value))
}

pub(crate) fn codex_wire_api_from_config(config_toml: &str) -> Option<String> {
    let document = config_toml.trim().parse::<DocumentMut>().ok()?;
    let root = document.as_table();
    selected_codex_provider_table(root)
        .and_then(|provider| {
            toml_string(provider, "wire_api").or_else(|| toml_string(provider, "api_format"))
        })
        .or_else(|| toml_string(root, "wire_api"))
        .or_else(|| toml_string(root, "api_format"))
        .or_else(|| {
            codex_provider_tables(root).and_then(|providers| {
                providers.iter().find_map(|(_, item)| {
                    item.as_table().and_then(|provider| {
                        toml_string(provider, "wire_api")
                            .or_else(|| toml_string(provider, "api_format"))
                    })
                })
            })
        })
}

pub(crate) fn codex_base_url_from_config(config_toml: &str) -> Option<String> {
    let document = config_toml.trim().parse::<DocumentMut>().ok()?;
    let root = document.as_table();
    selected_codex_provider_table(root)
        .and_then(|provider| toml_string(provider, "base_url"))
        .or_else(|| toml_string(root, "base_url"))
        .or_else(|| {
            codex_provider_tables(root).and_then(|providers| {
                providers.iter().find_map(|(_, item)| {
                    item.as_table()
                        .and_then(|provider| toml_string(provider, "base_url"))
                })
            })
        })
}

fn codex_provider_tables(root: &toml_edit::Table) -> Option<&toml_edit::Table> {
    root.get("model_providers").and_then(Item::as_table)
}

fn selected_codex_provider_table(root: &toml_edit::Table) -> Option<&toml_edit::Table> {
    let provider_name = root
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    codex_provider_tables(root)?
        .get(provider_name)
        .and_then(Item::as_table)
}

fn toml_string(table: &toml_edit::Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_chat_completions_url(url: &str) -> bool {
    let normalized = url.trim_end_matches('/').to_ascii_lowercase();
    normalized.ends_with("/chat/completions") || normalized.contains("/chat/completions?")
}

fn json_string_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<String> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn json_value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn json_bool_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => value.as_i64().map(|value| value != 0),
        Value::String(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                None
            } else {
                Some(matches!(normalized.as_str(), "true" | "1" | "yes" | "on"))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_openai_chat_provider_needs_gateway_proxy() {
        assert!(provider_needs_gateway_proxy(
            GatewayCliKey::Claude,
            "custom",
            Some(&json!({ "apiFormat": "openai_chat" })),
            "{}",
        ));
    }

    #[test]
    fn claude_anthropic_provider_does_not_need_gateway_proxy() {
        assert!(!provider_needs_gateway_proxy(
            GatewayCliKey::Claude,
            "custom",
            Some(&json!({ "apiFormat": "anthropic" })),
            "{}",
        ));
    }

    #[test]
    fn codex_anthropic_provider_needs_gateway_proxy() {
        assert!(provider_needs_gateway_proxy(
            GatewayCliKey::Codex,
            "custom",
            Some(&json!({ "apiFormat": "anthropic_messages" })),
            "{}",
        ));
    }

    #[test]
    fn codex_responses_provider_does_not_need_gateway_proxy() {
        assert!(!provider_needs_gateway_proxy(
            GatewayCliKey::Codex,
            "custom",
            Some(&json!({ "apiFormat": "openai_responses" })),
            "{}",
        ));
    }

    #[test]
    fn grok_native_protocol_variants_do_not_require_gateway_proxy() {
        for api_backend in ["responses", "chat_completions", "messages"] {
            let settings = json!({
                "config": format!(
                    "[models]\ndefault = \"custom\"\n[model.custom]\napi_backend = \"{api_backend}\"\n"
                )
            });
            assert!(!provider_needs_gateway_proxy(
                GatewayCliKey::Grok,
                "custom",
                None,
                &settings.to_string(),
            ));
        }
        assert!(provider_needs_gateway_proxy(
            GatewayCliKey::Grok,
            "custom",
            Some(&json!({ "apiFormat": "gemini_native" })),
            "{}",
        ));
    }

    #[test]
    fn grok_api_backend_comes_from_selected_model() {
        let config = r#"
[models]
default = "selected"
[model.first]
api_backend = "responses"
[model.selected]
api_backend = "messages"
"#;
        assert_eq!(
            grok_api_backend_from_config(config).as_deref(),
            Some("messages")
        );
    }

    #[test]
    fn codex_selected_provider_fields_drive_protocol_and_base_url() {
        let config = r#"
model_provider = "chat"
wire_api = "responses"
base_url = "https://legacy.example.com/v1"

[model_providers.responses]
wire_api = "responses"
base_url = "https://responses.example.com/v1"

[model_providers.chat]
wire_api = "chat"
base_url = "https://chat.example.com/v1/chat/completions"
"#;

        assert_eq!(codex_wire_api_from_config(config).as_deref(), Some("chat"));
        assert_eq!(
            codex_base_url_from_config(config).as_deref(),
            Some("https://chat.example.com/v1/chat/completions")
        );
        assert!(provider_needs_gateway_proxy(
            GatewayCliKey::Codex,
            "custom",
            None,
            &serde_json::json!({ "config": config }).to_string(),
        ));
    }

    #[test]
    fn codex_root_fields_remain_supported_for_legacy_configs() {
        let config = r#"
wire_api = "chat"
base_url = "https://legacy.example.com/v1/chat/completions"
"#;

        assert_eq!(codex_wire_api_from_config(config).as_deref(), Some("chat"));
        assert_eq!(
            codex_base_url_from_config(config).as_deref(),
            Some("https://legacy.example.com/v1/chat/completions")
        );
    }

    #[test]
    fn slash_api_format_aliases_are_supported() {
        assert!(!provider_needs_gateway_proxy(
            GatewayCliKey::Claude,
            "custom",
            Some(&json!({ "apiFormat": "anthropic/messages" })),
            "{}",
        ));
        assert!(!provider_needs_gateway_proxy(
            GatewayCliKey::Codex,
            "custom",
            Some(&json!({ "apiFormat": "openai/responses" })),
            "{}",
        ));
    }

    #[test]
    fn official_provider_does_not_need_gateway_proxy() {
        assert!(!provider_needs_gateway_proxy(
            GatewayCliKey::Codex,
            "official",
            Some(&json!({ "apiFormat": "anthropic_messages" })),
            "{}",
        ));
    }
}
