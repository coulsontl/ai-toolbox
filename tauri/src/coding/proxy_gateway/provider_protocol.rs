use super::transformer::AiProtocol;
use super::types::GatewayCliKey;
use serde_json::Value;
use toml_edit::{DocumentMut, Item};

pub(crate) fn native_cli_protocol(cli_key: GatewayCliKey) -> Option<AiProtocol> {
    match cli_key {
        GatewayCliKey::Claude => Some(AiProtocol::AnthropicMessages),
        GatewayCliKey::Codex => Some(AiProtocol::OpenAiResponses),
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
        GatewayCliKey::Gemini => {
            protocol_from_meta_or_settings(meta, &settings).unwrap_or(AiProtocol::GeminiNative)
        }
        GatewayCliKey::OpenCode => AiProtocol::OpenAiResponses,
    }
}

fn protocol_from_meta_or_settings(meta: Option<&Value>, settings: &Value) -> Option<AiProtocol> {
    meta.and_then(|value| json_string_compat(value, "api_format", "apiFormat"))
        .or_else(|| json_value_string(settings, "api_format"))
        .or_else(|| json_value_string(settings, "apiFormat"))
        .and_then(|value| AiProtocol::from_api_format(&value))
}

fn codex_wire_api_from_config(config_toml: &str) -> Option<AiProtocol> {
    let document = config_toml.trim().parse::<DocumentMut>().ok()?;
    document
        .as_table()
        .get("wire_api")
        .and_then(Item::as_str)
        .or_else(|| document.as_table().get("api_format").and_then(Item::as_str))
        .and_then(AiProtocol::from_api_format)
}

fn codex_base_url_from_config(config_toml: &str) -> Option<String> {
    let document = config_toml.trim().parse::<DocumentMut>().ok()?;
    let provider_name = document
        .as_table()
        .get("model_provider")
        .and_then(Item::as_str)?;
    document
        .get("model_providers")?
        .get(provider_name)?
        .get("base_url")
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
