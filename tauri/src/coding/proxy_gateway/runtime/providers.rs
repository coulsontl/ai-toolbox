use crate::coding::proxy_gateway::types::{
    normalize_pricing_model_source, GatewayCliKey, GatewayProxyMode, ProviderGatewayMeta,
    ProviderPriorityEntry, ProxyGatewaySettings,
};
use crate::coding::proxy_gateway::{
    cli_proxy::manifest::CliProxyManifest, paths::ProxyGatewayPaths, transformer::AiProtocol,
};
use crate::coding::{claude_code, codex, gemini_cli};
use crate::db::helpers::db_list;
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use serde_json::Value;
use std::fs;
use toml_edit::{DocumentMut, Item};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpstreamProvider {
    pub(crate) cli_key: GatewayCliKey,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) target_protocol: AiProtocol,
    pub(crate) auth_strategy: ProviderAuthStrategy,
    pub(crate) is_full_url: bool,
    pub(crate) sort_index: Option<i32>,
    pub(crate) meta: ProviderGatewayMeta,
    pub(crate) model_mapping: UpstreamModelMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAuthStrategy {
    Bearer,
    AnthropicApiKey,
    GoogleApiKey,
    GoogleOAuth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UpstreamModelMapping {
    pub(crate) default_model: Option<String>,
    pub(crate) haiku_model: Option<String>,
    pub(crate) sonnet_model: Option<String>,
    pub(crate) opus_model: Option<String>,
    pub(crate) reasoning_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GatewayProviderSelection {
    pub(crate) mode: GatewayProxyMode,
    pub(crate) primary_provider_id: String,
}

pub(crate) async fn load_candidate_providers(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
) -> Result<Vec<UpstreamProvider>, String> {
    load_candidate_providers_with_settings(db, cli_key, None).await
}

pub(crate) async fn load_candidate_providers_with_settings(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    settings: Option<&ProxyGatewaySettings>,
) -> Result<Vec<UpstreamProvider>, String> {
    load_candidate_providers_with_settings_and_selection(db, cli_key, settings, None).await
}

pub(crate) async fn load_candidate_providers_with_settings_and_selection(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    settings: Option<&ProxyGatewaySettings>,
    selection: Option<&GatewayProviderSelection>,
) -> Result<Vec<UpstreamProvider>, String> {
    let table = match cli_key {
        GatewayCliKey::Claude => DbTable::ClaudeProvider,
        GatewayCliKey::Codex => DbTable::CodexProvider,
        GatewayCliKey::Gemini => DbTable::GeminiCliProvider,
        GatewayCliKey::OpenCode => {
            return Err(
                "OpenCode adapter is intentionally out of scope for the gateway MVP".to_string(),
            )
        }
    };
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("updated_at", OrderDirection::Desc)?,
    ]);
    let records = db.with_conn(|conn| db_list(conn, table, Some(&order)))?;

    let mut providers = Vec::new();
    let mut parse_errors = Vec::new();
    for record in records {
        match provider_from_record(cli_key, record, settings) {
            Ok(Some(provider)) => providers.push(provider),
            Ok(None) => {}
            Err(error) => parse_errors.push(error),
        }
    }
    sort_candidate_providers(&mut providers);

    if providers.is_empty() && !parse_errors.is_empty() {
        return Err(parse_errors.join("; "));
    }

    apply_provider_selection(providers, selection)
}

pub(crate) fn load_gateway_provider_selection(
    paths: Option<&ProxyGatewayPaths>,
    cli_key: GatewayCliKey,
) -> Result<Option<GatewayProviderSelection>, String> {
    let Some(paths) = paths else {
        return Ok(None);
    };
    let manifest_path = paths.manifest_path(cli_key);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "Failed to read gateway manifest {}: {}",
            manifest_path.display(),
            error
        )
    })?;
    let manifest = serde_json::from_str::<CliProxyManifest>(&content).map_err(|error| {
        format!(
            "Failed to parse gateway manifest {}: {}",
            manifest_path.display(),
            error
        )
    })?;
    if !manifest.enabled {
        return Ok(None);
    }
    Ok(Some(GatewayProviderSelection {
        mode: manifest.mode,
        primary_provider_id: manifest.primary_provider_id,
    }))
}

pub(crate) fn provider_priority_entries(
    providers: &[UpstreamProvider],
) -> Vec<ProviderPriorityEntry> {
    providers
        .iter()
        .enumerate()
        .map(|(index, provider)| ProviderPriorityEntry {
            provider_id: provider.id.clone(),
            label: format!("P{index}"),
        })
        .collect()
}

fn apply_provider_selection(
    mut providers: Vec<UpstreamProvider>,
    selection: Option<&GatewayProviderSelection>,
) -> Result<Vec<UpstreamProvider>, String> {
    let Some(selection) = selection else {
        return Ok(providers);
    };

    let primary_index = providers
        .iter()
        .position(|provider| provider.id == selection.primary_provider_id);
    match selection.mode {
        GatewayProxyMode::Single => {
            let Some(primary_index) = primary_index else {
                return Err(format!(
                    "Primary gateway proxy provider '{}' was not found for {}",
                    selection.primary_provider_id,
                    selection.mode.as_str()
                ));
            };
            Ok(vec![providers.remove(primary_index)])
        }
        GatewayProxyMode::Failover => {
            if let Some(primary_index) = primary_index {
                let primary_provider = providers.remove(primary_index);
                let mut ordered_providers = Vec::with_capacity(providers.len() + 1);
                ordered_providers.push(primary_provider);
                ordered_providers.extend(providers);
                Ok(ordered_providers)
            } else {
                Ok(providers)
            }
        }
    }
}

fn sort_candidate_providers(providers: &mut [UpstreamProvider]) {
    providers.sort_by(|left, right| {
        left.sort_index
            .unwrap_or(0)
            .cmp(&right.sort_index.unwrap_or(0))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn provider_from_record(
    cli_key: GatewayCliKey,
    record: Value,
    settings: Option<&ProxyGatewaySettings>,
) -> Result<Option<UpstreamProvider>, String> {
    let meta = provider_meta_from_record(cli_key, &record, settings);
    match cli_key {
        GatewayCliKey::Claude => {
            let provider = claude_code::adapter::from_db_value_provider(record);
            if provider.is_disabled {
                return Ok(None);
            }
            if is_official_provider_category(&provider.category) {
                return Ok(None);
            }
            let settings =
                parse_json_config(&provider.settings_config, "Claude provider settings_config")?;
            let env = settings.get("env").and_then(Value::as_object);
            let target_protocol = claude_target_protocol(&meta, &settings);
            let (api_key, auth_strategy) = claude_auth_from_settings(
                env,
                &settings,
                target_protocol,
                meta.api_key_field.as_deref(),
                &provider.name,
            )?;
            let base_url = json_object_string(env, "ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            let model_mapping = claude_model_mapping_from_settings(&settings);
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                target_protocol,
                auth_strategy,
                is_full_url: meta.is_full_url,
                sort_index: provider.sort_index,
                meta,
                model_mapping,
            }))
        }
        GatewayCliKey::Codex => {
            let provider = codex::adapter::from_db_value_provider(record);
            if provider.is_disabled {
                return Ok(None);
            }
            if is_official_provider_category(&provider.category) {
                return Ok(None);
            }
            let settings =
                parse_json_config(&provider.settings_config, "Codex provider settings_config")?;
            let auth = settings.get("auth").and_then(Value::as_object);
            let api_key = json_object_string(auth, "OPENAI_API_KEY").ok_or_else(|| {
                format!("Codex provider '{}' has no OPENAI_API_KEY", provider.name)
            })?;
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let base_url = codex_base_url_from_config(config_toml)
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let target_protocol = codex_target_protocol(&meta, &settings, &base_url);
            let auth_strategy = auth_strategy_for_target_protocol(
                target_protocol,
                meta.api_key_field.as_deref(),
                &api_key,
                ProviderAuthStrategy::Bearer,
            );
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                target_protocol,
                auth_strategy,
                is_full_url: meta.is_full_url,
                sort_index: provider.sort_index,
                meta,
                model_mapping: UpstreamModelMapping::default(),
            }))
        }
        GatewayCliKey::Gemini => {
            let provider = gemini_cli::adapter::from_db_value_provider(record);
            if provider.is_disabled {
                return Ok(None);
            }
            if is_official_provider_category(&provider.category) {
                return Ok(None);
            }
            let settings = parse_json_config(
                &provider.settings_config,
                "Gemini CLI provider settings_config",
            )?;
            let env = settings.get("env").and_then(Value::as_object);
            let base_url = json_object_string(env, "GOOGLE_GEMINI_BASE_URL")
                .or_else(|| json_object_string(env, "GOOGLE_VERTEX_BASE_URL"))
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
            let target_protocol = gemini_target_protocol(&meta, &settings);
            let (api_key, auth_strategy) = gemini_auth_from_settings(
                env,
                &settings,
                target_protocol,
                meta.api_key_field.as_deref(),
                &provider.name,
            )?;
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                target_protocol,
                auth_strategy,
                is_full_url: meta.is_full_url,
                sort_index: provider.sort_index,
                meta,
                model_mapping: UpstreamModelMapping::default(),
            }))
        }
        GatewayCliKey::OpenCode => unreachable!("OpenCode is rejected before query"),
    }
}

fn is_official_provider_category(category: &str) -> bool {
    category.trim().eq_ignore_ascii_case("official")
}

fn provider_meta_from_record(
    cli_key: GatewayCliKey,
    record: &Value,
    settings: Option<&ProxyGatewaySettings>,
) -> ProviderGatewayMeta {
    let meta_value = record.get("meta").unwrap_or(&Value::Null);
    let default_cost_multiplier = settings
        .map(|settings| settings.default_cost_multiplier_for(cli_key))
        .unwrap_or_else(|| "1.0".to_string());
    let default_pricing_model_source = settings
        .map(|settings| settings.default_pricing_model_source_for(cli_key))
        .unwrap_or_else(|| "upstream".to_string());
    let mut meta = ProviderGatewayMeta {
        provider_type: json_string_compat(meta_value, "provider_type", "providerType"),
        api_format: json_string_compat(meta_value, "api_format", "apiFormat"),
        api_key_field: json_string_compat(meta_value, "api_key_field", "apiKeyField"),
        is_full_url: json_bool_compat(meta_value, "is_full_url", "isFullUrl").unwrap_or(false),
        prompt_cache_key: json_string_compat(meta_value, "prompt_cache_key", "promptCacheKey"),
        cost_multiplier: json_string_compat(meta_value, "cost_multiplier", "costMultiplier")
            .unwrap_or_else(|| default_cost_multiplier.clone()),
        pricing_model_source: json_string_compat(
            meta_value,
            "pricing_model_source",
            "pricingModelSource",
        )
        .map(|value| normalize_pricing_model_source(&value))
        .unwrap_or_else(|| default_pricing_model_source.clone()),
    };
    if meta.provider_type.is_none() {
        meta.provider_type = record
            .get("category")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    if meta.cost_multiplier.trim().is_empty() {
        meta.cost_multiplier = default_cost_multiplier;
    }
    if meta.pricing_model_source.trim().is_empty() {
        meta.pricing_model_source = default_pricing_model_source;
    }
    meta
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

fn json_bool_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<bool> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(json_bool_value)
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

fn claude_target_protocol(meta: &ProviderGatewayMeta, settings: &Value) -> AiProtocol {
    meta.api_format
        .as_deref()
        .and_then(AiProtocol::from_api_format)
        .or_else(|| {
            json_value_string(settings, "api_format")
                .and_then(|value| AiProtocol::from_api_format(&value))
        })
        .or_else(|| {
            json_value_string(settings, "apiFormat")
                .and_then(|value| AiProtocol::from_api_format(&value))
        })
        .or_else(|| {
            settings
                .get("openrouter_compat_mode")
                .and_then(json_bool_value)
                .filter(|enabled| *enabled)
                .map(|_| AiProtocol::OpenAiChat)
        })
        .unwrap_or(AiProtocol::AnthropicMessages)
}

fn codex_target_protocol(
    meta: &ProviderGatewayMeta,
    settings: &Value,
    base_url: &str,
) -> AiProtocol {
    meta.api_format
        .as_deref()
        .and_then(AiProtocol::from_api_format)
        .or_else(|| {
            json_value_string(settings, "api_format")
                .and_then(|value| AiProtocol::from_api_format(&value))
        })
        .or_else(|| {
            json_value_string(settings, "apiFormat")
                .and_then(|value| AiProtocol::from_api_format(&value))
        })
        .or_else(|| {
            codex_wire_api_from_settings(settings)
                .and_then(|value| AiProtocol::from_api_format(&value))
        })
        .unwrap_or_else(|| {
            if is_chat_completions_url(base_url) {
                AiProtocol::OpenAiChat
            } else {
                AiProtocol::OpenAiResponses
            }
        })
}

fn gemini_target_protocol(meta: &ProviderGatewayMeta, settings: &Value) -> AiProtocol {
    meta.api_format
        .as_deref()
        .and_then(AiProtocol::from_api_format)
        .or_else(|| {
            json_value_string(settings, "api_format")
                .and_then(|value| AiProtocol::from_api_format(&value))
        })
        .or_else(|| {
            json_value_string(settings, "apiFormat")
                .and_then(|value| AiProtocol::from_api_format(&value))
        })
        .unwrap_or(AiProtocol::GeminiNative)
}

fn codex_wire_api_from_settings(settings: &Value) -> Option<String> {
    settings
        .get("config")
        .and_then(Value::as_str)
        .and_then(codex_wire_api_from_config)
}

fn codex_wire_api_from_config(config_toml: &str) -> Option<String> {
    let trimmed = config_toml.trim();
    if trimmed.is_empty() {
        return None;
    }
    let document = trimmed.parse::<DocumentMut>().ok()?;
    document
        .as_table()
        .get("wire_api")
        .and_then(Item::as_str)
        .or_else(|| document.as_table().get("api_format").and_then(Item::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_chat_completions_url(url: &str) -> bool {
    let normalized = url.trim_end_matches('/').to_ascii_lowercase();
    normalized.ends_with("/chat/completions") || normalized.contains("/chat/completions?")
}

fn claude_auth_from_settings(
    env: Option<&serde_json::Map<String, Value>>,
    settings: &Value,
    target_protocol: AiProtocol,
    api_key_field: Option<&str>,
    provider_name: &str,
) -> Result<(String, ProviderAuthStrategy), String> {
    let candidates: &[&str] = match target_protocol {
        AiProtocol::GeminiNative => &[
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
        ],
        AiProtocol::OpenAiChat | AiProtocol::OpenAiResponses => &[
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
        ],
        AiProtocol::AnthropicMessages => &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"],
    };

    for key_name in candidates {
        if let Some(api_key) = json_object_string(env, key_name) {
            let default_strategy = match (*key_name, target_protocol) {
                ("ANTHROPIC_AUTH_TOKEN", AiProtocol::AnthropicMessages) => {
                    ProviderAuthStrategy::Bearer
                }
                (_, AiProtocol::AnthropicMessages) => ProviderAuthStrategy::AnthropicApiKey,
                (_, AiProtocol::GeminiNative) => gemini_auth_strategy(&api_key),
                _ => ProviderAuthStrategy::Bearer,
            };
            return Ok((
                api_key.clone(),
                auth_strategy_for_target_protocol(
                    target_protocol,
                    api_key_field,
                    &api_key,
                    default_strategy,
                ),
            ));
        }
    }

    if let Some(api_key) =
        json_value_string(settings, "apiKey").or_else(|| json_value_string(settings, "api_key"))
    {
        let default_strategy = match target_protocol {
            AiProtocol::AnthropicMessages => ProviderAuthStrategy::AnthropicApiKey,
            AiProtocol::GeminiNative => gemini_auth_strategy(&api_key),
            AiProtocol::OpenAiChat | AiProtocol::OpenAiResponses => ProviderAuthStrategy::Bearer,
        };
        return Ok((
            api_key.clone(),
            auth_strategy_for_target_protocol(
                target_protocol,
                api_key_field,
                &api_key,
                default_strategy,
            ),
        ));
    }

    Err(format!(
        "Claude provider '{}' has no API key",
        provider_name
    ))
}

fn gemini_auth_from_settings(
    env: Option<&serde_json::Map<String, Value>>,
    settings: &Value,
    target_protocol: AiProtocol,
    api_key_field: Option<&str>,
    provider_name: &str,
) -> Result<(String, ProviderAuthStrategy), String> {
    let candidates: &[&str] = match target_protocol {
        AiProtocol::AnthropicMessages => &[
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
        ],
        AiProtocol::OpenAiChat | AiProtocol::OpenAiResponses => {
            &["OPENAI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"]
        }
        AiProtocol::GeminiNative => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
    };

    for key_name in candidates {
        if let Some(api_key) = json_object_string(env, key_name) {
            let default_strategy = match (*key_name, target_protocol) {
                ("ANTHROPIC_AUTH_TOKEN", AiProtocol::AnthropicMessages) => {
                    ProviderAuthStrategy::Bearer
                }
                (_, AiProtocol::AnthropicMessages) => ProviderAuthStrategy::AnthropicApiKey,
                (_, AiProtocol::GeminiNative) => gemini_auth_strategy(&api_key),
                _ => ProviderAuthStrategy::Bearer,
            };
            return Ok((
                api_key.clone(),
                auth_strategy_for_target_protocol(
                    target_protocol,
                    api_key_field,
                    &api_key,
                    default_strategy,
                ),
            ));
        }
    }

    if let Some(api_key) =
        json_value_string(settings, "apiKey").or_else(|| json_value_string(settings, "api_key"))
    {
        let default_strategy = match target_protocol {
            AiProtocol::AnthropicMessages => ProviderAuthStrategy::AnthropicApiKey,
            AiProtocol::GeminiNative => gemini_auth_strategy(&api_key),
            AiProtocol::OpenAiChat | AiProtocol::OpenAiResponses => ProviderAuthStrategy::Bearer,
        };
        return Ok((
            api_key.clone(),
            auth_strategy_for_target_protocol(
                target_protocol,
                api_key_field,
                &api_key,
                default_strategy,
            ),
        ));
    }

    Err(format!(
        "Gemini CLI provider '{}' has no API key",
        provider_name
    ))
}

fn auth_strategy_for_target_protocol(
    target_protocol: AiProtocol,
    api_key_field: Option<&str>,
    api_key: &str,
    default_strategy: ProviderAuthStrategy,
) -> ProviderAuthStrategy {
    if let Some(strategy) = auth_strategy_from_api_key_field(api_key_field) {
        return strategy;
    }

    match target_protocol {
        AiProtocol::AnthropicMessages => default_strategy,
        AiProtocol::OpenAiChat | AiProtocol::OpenAiResponses => ProviderAuthStrategy::Bearer,
        AiProtocol::GeminiNative => gemini_auth_strategy(api_key),
    }
}

fn auth_strategy_from_api_key_field(value: Option<&str>) -> Option<ProviderAuthStrategy> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "authorization" | "bearer" | "auth_token" | "anthropic_auth_token" => {
            Some(ProviderAuthStrategy::Bearer)
        }
        "x-api-key" | "api_key" | "anthropic_api_key" => {
            Some(ProviderAuthStrategy::AnthropicApiKey)
        }
        "x-goog-api-key" | "google_api_key" | "gemini_api_key" => {
            Some(ProviderAuthStrategy::GoogleApiKey)
        }
        "google_oauth" | "oauth" => Some(ProviderAuthStrategy::GoogleOAuth),
        _ => None,
    }
}

fn gemini_auth_strategy(api_key: &str) -> ProviderAuthStrategy {
    let trimmed = api_key.trim();
    if trimmed.starts_with("ya29.") {
        return ProviderAuthStrategy::GoogleOAuth;
    }
    if trimmed.starts_with('{')
        && serde_json::from_str::<Value>(trimmed)
            .ok()
            .and_then(|value| {
                value
                    .get("access_token")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .is_some()
    {
        return ProviderAuthStrategy::GoogleOAuth;
    }
    ProviderAuthStrategy::GoogleApiKey
}

fn parse_json_config(raw: &str, label: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|error| format!("Failed to parse {label}: {error}"))
}

fn claude_model_mapping_from_settings(settings: &Value) -> UpstreamModelMapping {
    let env = settings.get("env").and_then(Value::as_object);
    UpstreamModelMapping {
        default_model: json_value_string(settings, "model")
            .or_else(|| json_object_string(env, "ANTHROPIC_MODEL")),
        haiku_model: json_value_string(settings, "haikuModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_DEFAULT_HAIKU_MODEL")),
        sonnet_model: json_value_string(settings, "sonnetModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_DEFAULT_SONNET_MODEL")),
        opus_model: json_value_string(settings, "opusModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_DEFAULT_OPUS_MODEL")),
        reasoning_model: json_value_string(settings, "reasoningModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_REASONING_MODEL")),
    }
}

fn json_value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn json_object_string(
    object: Option<&serde_json::Map<String, Value>>,
    key: &str,
) -> Option<String> {
    object
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn codex_base_url_from_config(config_toml: &str) -> Option<String> {
    let trimmed = config_toml.trim();
    if trimmed.is_empty() {
        return None;
    }
    let document = trimmed.parse::<DocumentMut>().ok()?;
    let root = document.as_table();
    let providers = root.get("model_providers")?.as_table()?;
    let selected_provider = root
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(selected_provider) = selected_provider {
        if let Some(base_url) = providers
            .get(selected_provider)
            .and_then(Item::as_table)
            .and_then(|provider| provider.get("base_url"))
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(base_url.to_string());
        }
    }

    let fallback = providers.iter().find_map(|(_, item)| {
        item.as_table()
            .and_then(|provider| provider.get("base_url"))
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, sort_index: Option<i32>) -> UpstreamProvider {
        UpstreamProvider {
            cli_key: GatewayCliKey::Claude,
            id: name.to_string(),
            name: name.to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: "key".to_string(),
            target_protocol: AiProtocol::AnthropicMessages,
            auth_strategy: ProviderAuthStrategy::AnthropicApiKey,
            is_full_url: false,
            sort_index,
            meta: ProviderGatewayMeta::default(),
            model_mapping: UpstreamModelMapping::default(),
        }
    }

    #[test]
    fn candidate_providers_keep_topmost_sort_index_first() {
        let mut providers = vec![
            provider("third", Some(30)),
            provider("first", Some(10)),
            provider("second", Some(20)),
        ];

        sort_candidate_providers(&mut providers);

        let names: Vec<&str> = providers
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second", "third"]);
    }

    #[test]
    fn none_sort_index_treated_as_zero_matching_frontend() {
        let mut providers = vec![provider("second", Some(1)), provider("first", None)];

        sort_candidate_providers(&mut providers);

        let names: Vec<&str> = providers
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn single_selection_returns_only_primary_provider() {
        let providers = vec![
            provider("first", Some(10)),
            provider("primary", Some(20)),
            provider("third", Some(30)),
        ];
        let selection = GatewayProviderSelection {
            mode: GatewayProxyMode::Single,
            primary_provider_id: "primary".to_string(),
        };

        let selected = apply_provider_selection(providers, Some(&selection)).unwrap();

        let names: Vec<&str> = selected
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, vec!["primary"]);
    }

    #[test]
    fn failover_selection_promotes_primary_provider_to_p0() {
        let providers = vec![
            provider("first", Some(10)),
            provider("primary", Some(20)),
            provider("third", Some(30)),
        ];
        let selection = GatewayProviderSelection {
            mode: GatewayProxyMode::Failover,
            primary_provider_id: "primary".to_string(),
        };

        let selected = apply_provider_selection(providers, Some(&selection)).unwrap();

        let names: Vec<&str> = selected
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, vec!["primary", "first", "third"]);
    }

    #[test]
    fn failover_selection_keeps_sorted_order_when_primary_is_missing() {
        let providers = vec![provider("first", Some(10)), provider("second", Some(20))];
        let selection = GatewayProviderSelection {
            mode: GatewayProxyMode::Failover,
            primary_provider_id: "missing".to_string(),
        };

        let selected = apply_provider_selection(providers, Some(&selection)).unwrap();

        let names: Vec<&str> = selected
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn claude_model_mapping_reads_reasoning_model() {
        let mapping = claude_model_mapping_from_settings(&serde_json::json!({
            "model": "provider-default",
            "reasoningModel": "provider-reasoning",
            "env": {
                "ANTHROPIC_REASONING_MODEL": "env-reasoning"
            }
        }));

        assert_eq!(mapping.default_model.as_deref(), Some("provider-default"));
        assert_eq!(
            mapping.reasoning_model.as_deref(),
            Some("provider-reasoning")
        );
    }

    #[test]
    fn official_providers_are_not_gateway_candidates() {
        for cli_key in [
            GatewayCliKey::Claude,
            GatewayCliKey::Codex,
            GatewayCliKey::Gemini,
        ] {
            let result = provider_from_record(
                cli_key,
                serde_json::json!({
                    "id": format!("{}-official", cli_key.as_str()),
                    "name": "Official",
                    "category": "official",
                    "settings_config": "{}",
                    "is_disabled": false
                }),
                None,
            )
            .unwrap();

            assert!(result.is_none());
        }
    }

    #[test]
    fn claude_api_format_meta_selects_openai_chat_target_and_bearer_auth() {
        let result = provider_from_record(
            GatewayCliKey::Claude,
            serde_json::json!({
                "id": "claude-openai-chat",
                "name": "Claude OpenAI Chat",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://openrouter.example.com/v1",
                        "OPENAI_API_KEY": "openai-key"
                    }
                }).to_string(),
                "meta": {
                    "apiFormat": "openai_chat",
                    "isFullUrl": true
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.target_protocol, AiProtocol::OpenAiChat);
        assert_eq!(result.auth_strategy, ProviderAuthStrategy::Bearer);
        assert!(result.is_full_url);
    }

    #[test]
    fn codex_api_format_meta_selects_anthropic_target() {
        let result = provider_from_record(
            GatewayCliKey::Codex,
            serde_json::json!({
                "id": "codex-anthropic",
                "name": "Codex Anthropic",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "auth": {"OPENAI_API_KEY": "anthropic-key"},
                    "config": r#"
model_provider = "custom"
[model_providers.custom]
base_url = "https://api.anthropic.com"
"#
                }).to_string(),
                "meta": {
                    "apiFormat": "anthropic_messages",
                    "apiKeyField": "x-api-key"
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.target_protocol, AiProtocol::AnthropicMessages);
        assert_eq!(result.auth_strategy, ProviderAuthStrategy::AnthropicApiKey);
    }

    #[test]
    fn codex_slash_api_format_meta_selects_anthropic_target() {
        let result = provider_from_record(
            GatewayCliKey::Codex,
            serde_json::json!({
                "id": "codex-anthropic-slash",
                "name": "Codex Anthropic Slash",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "auth": {"OPENAI_API_KEY": "anthropic-key"},
                    "config": r#"
model_provider = "custom"
[model_providers.custom]
base_url = "https://api.anthropic.com"
"#
                }).to_string(),
                "meta": {
                    "apiFormat": "anthropic/messages",
                    "apiKeyField": "x-api-key"
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.target_protocol, AiProtocol::AnthropicMessages);
        assert_eq!(result.auth_strategy, ProviderAuthStrategy::AnthropicApiKey);
    }

    #[test]
    fn gemini_api_format_meta_selects_anthropic_target_and_api_key_auth() {
        let result = provider_from_record(
            GatewayCliKey::Gemini,
            serde_json::json!({
                "id": "gemini-anthropic",
                "name": "Gemini Anthropic",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "env": {
                        "GOOGLE_GEMINI_BASE_URL": "https://api.anthropic.com",
                        "GEMINI_API_KEY": "anthropic-key"
                    }
                }).to_string(),
                "meta": {
                    "apiFormat": "anthropic"
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.target_protocol, AiProtocol::AnthropicMessages);
        assert_eq!(result.auth_strategy, ProviderAuthStrategy::AnthropicApiKey);
        assert_eq!(result.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn gemini_api_format_meta_selects_anthropic_bearer_for_auth_token() {
        let result = provider_from_record(
            GatewayCliKey::Gemini,
            serde_json::json!({
                "id": "gemini-anthropic-token",
                "name": "Gemini Anthropic Token",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "env": {
                        "GOOGLE_GEMINI_BASE_URL": "https://api.anthropic.com",
                        "ANTHROPIC_AUTH_TOKEN": "token"
                    }
                }).to_string(),
                "meta": {
                    "apiFormat": "anthropic"
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.target_protocol, AiProtocol::AnthropicMessages);
        assert_eq!(result.auth_strategy, ProviderAuthStrategy::Bearer);
    }

    #[test]
    fn gemini_provider_defaults_to_native_protocol_and_google_auth() {
        let result = provider_from_record(
            GatewayCliKey::Gemini,
            serde_json::json!({
                "id": "gemini-native",
                "name": "Gemini Native",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "env": {
                        "GOOGLE_GEMINI_BASE_URL": "https://generativelanguage.googleapis.com/v1beta",
                        "GEMINI_API_KEY": "google-key"
                    }
                }).to_string(),
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.target_protocol, AiProtocol::GeminiNative);
        assert_eq!(result.auth_strategy, ProviderAuthStrategy::GoogleApiKey);
    }
}
