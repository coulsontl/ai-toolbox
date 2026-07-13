use crate::coding::proxy_gateway::types::{
    normalize_pricing_model_source, CodexChatReasoningMeta, GatewayCliKey,
    GatewayProviderProfileReference, GatewayProxyMode, ProviderGatewayMeta, ProviderPriorityEntry,
    ProxyGatewaySettings,
};
use crate::coding::proxy_gateway::{
    cli_proxy::manifest::CliProxyManifest, paths::ProxyGatewayPaths,
    provider_profiles::load_gateway_provider_profiles_for_runtime, transformer::AiProtocol,
};
use crate::coding::{claude_code, codex, gemini_cli, grok};
use crate::db::helpers::db_list;
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use serde_json::Value;
use std::fs;

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
    pub(crate) fable_model: Option<String>,
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
        GatewayCliKey::Grok => DbTable::GrokProvider,
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

pub(crate) async fn load_provider_by_id_for_connectivity_test(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    provider_id: &str,
    settings: Option<&ProxyGatewaySettings>,
) -> Result<UpstreamProvider, String> {
    let table = match cli_key {
        GatewayCliKey::Claude => DbTable::ClaudeProvider,
        GatewayCliKey::Codex => DbTable::CodexProvider,
        GatewayCliKey::Grok => DbTable::GrokProvider,
        GatewayCliKey::Gemini => DbTable::GeminiCliProvider,
        GatewayCliKey::OpenCode => {
            return Err(
                "OpenCode adapter is intentionally out of scope for the gateway MVP".to_string(),
            )
        }
    };
    let records = db.with_conn(|conn| db_list(conn, table, None))?;
    let record = records
        .into_iter()
        .find(|record| record.get("id").and_then(Value::as_str) == Some(provider_id))
        .ok_or_else(|| {
            format!(
                "Provider '{provider_id}' was not found for {}",
                cli_key.as_str()
            )
        })?;
    provider_from_record(cli_key, record, settings)?.ok_or_else(|| {
        format!(
            "Provider '{provider_id}' is disabled or not eligible for Gateway connectivity testing"
        )
    })
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
            let (api_key, mut auth_strategy) = claude_auth_from_settings(
                env,
                &settings,
                target_protocol,
                meta.api_key_field.as_deref(),
                &provider.name,
            )?;
            if target_protocol == AiProtocol::AnthropicMessages
                && anthropic_platform_uses_bearer_auth(&meta)
            {
                auth_strategy = ProviderAuthStrategy::Bearer;
            }
            let base_url = json_object_string(env, "ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            let (base_url, is_full_url) = normalize_provider_base_url(base_url, meta.is_full_url);
            let model_mapping = claude_model_mapping_from_settings(&settings);
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                target_protocol,
                auth_strategy,
                is_full_url,
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
            let (base_url, is_full_url) = normalize_provider_base_url(base_url, meta.is_full_url);
            let target_protocol = codex_target_protocol(&meta, &settings, &base_url);
            let mut auth_strategy = auth_strategy_for_target_protocol(
                target_protocol,
                meta.api_key_field.as_deref(),
                &api_key,
                ProviderAuthStrategy::Bearer,
            );
            if target_protocol == AiProtocol::AnthropicMessages
                && anthropic_platform_uses_bearer_auth(&meta)
            {
                auth_strategy = ProviderAuthStrategy::Bearer;
            }
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                target_protocol,
                auth_strategy,
                is_full_url,
                sort_index: provider.sort_index,
                meta,
                model_mapping: UpstreamModelMapping::default(),
            }))
        }
        GatewayCliKey::Grok => {
            let provider = grok::adapter::provider_from_db_value(record);
            if provider.is_disabled || is_official_provider_category(&provider.category) {
                return Ok(None);
            }
            let settings =
                parse_json_config(&provider.settings_config, "Grok provider settings_config")?;
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let selected_model = grok_selected_model_from_settings(&settings, config_toml);
            let api_key = settings
                .pointer("/auth/API_KEY")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    selected_model
                        .as_ref()
                        .and_then(|model| model.api_key.clone())
                })
                .ok_or_else(|| format!("Grok provider '{}' has no API_KEY", provider.name))?;
            let base_url = selected_model
                .as_ref()
                .and_then(|model| model.base_url.clone())
                .unwrap_or_else(|| "https://api.x.ai/v1".to_string());
            let (base_url, is_full_url) = normalize_provider_base_url(base_url, meta.is_full_url);
            let target_protocol = meta
                .api_format
                .as_deref()
                .and_then(AiProtocol::from_api_format)
                .or_else(|| {
                    selected_model
                        .as_ref()
                        .and_then(|model| model.api_backend.as_deref())
                        .and_then(AiProtocol::from_api_format)
                })
                .unwrap_or(AiProtocol::OpenAiChat);
            let mut auth_strategy = auth_strategy_for_target_protocol(
                target_protocol,
                meta.api_key_field.as_deref(),
                &api_key,
                ProviderAuthStrategy::Bearer,
            );
            if target_protocol == AiProtocol::AnthropicMessages
                && anthropic_platform_uses_bearer_auth(&meta)
            {
                auth_strategy = ProviderAuthStrategy::Bearer;
            }
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                target_protocol,
                auth_strategy,
                is_full_url,
                sort_index: provider.sort_index,
                meta,
                model_mapping: UpstreamModelMapping {
                    default_model: selected_model.and_then(|model| model.model_id),
                    ..UpstreamModelMapping::default()
                },
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
            let (base_url, is_full_url) = normalize_provider_base_url(base_url, meta.is_full_url);
            let target_protocol = gemini_target_protocol(&meta, &settings);
            let (api_key, mut auth_strategy) = gemini_auth_from_settings(
                env,
                &settings,
                target_protocol,
                meta.api_key_field.as_deref(),
                &provider.name,
            )?;
            if target_protocol == AiProtocol::AnthropicMessages
                && anthropic_platform_uses_bearer_auth(&meta)
            {
                auth_strategy = ProviderAuthStrategy::Bearer;
            }
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                target_protocol,
                auth_strategy,
                is_full_url,
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

fn normalize_provider_base_url(base_url: String, is_full_url: bool) -> (String, bool) {
    if let Some(raw_url) = base_url.trim().strip_suffix("##") {
        return (raw_url.trim_end().to_string(), true);
    }
    (base_url, is_full_url)
}

fn anthropic_platform_uses_bearer_auth(meta: &ProviderGatewayMeta) -> bool {
    meta.provider_type
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase().replace(['_', ' '], "-"))
        .is_some_and(|value| {
            matches!(
                value.as_str(),
                "bedrock" | "anthropic-bedrock" | "aws-bedrock" | "longcat" | "long-cat"
            )
        })
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
        gateway_profile: gateway_profile_reference_from_meta(meta_value),
        provider_type: json_string_compat(meta_value, "provider_type", "providerType"),
        api_format: json_string_compat(meta_value, "api_format", "apiFormat"),
        api_key_field: json_string_compat(meta_value, "api_key_field", "apiKeyField"),
        is_full_url: json_bool_compat(meta_value, "is_full_url", "isFullUrl").unwrap_or(false),
        prompt_cache_key: json_string_compat(meta_value, "prompt_cache_key", "promptCacheKey"),
        reasoning_field: json_string_compat(meta_value, "reasoning_field", "reasoningField"),
        default_max_tokens: json_i64_compat(meta_value, "default_max_tokens", "defaultMaxTokens"),
        codex_chat_reasoning: meta_value
            .get("codex_chat_reasoning")
            .or_else(|| meta_value.get("codexChatReasoning"))
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        image_input_policy: json_string_compat(
            meta_value,
            "image_input_policy",
            "imageInputPolicy",
        ),
        text_only_models: json_string_vec_compat(meta_value, "text_only_models", "textOnlyModels"),
        image_capable_models: json_string_vec_compat(
            meta_value,
            "image_capable_models",
            "imageCapableModels",
        ),
        allow_text_only_model_heuristic: json_bool_compat(
            meta_value,
            "allow_text_only_model_heuristic",
            "allowTextOnlyModelHeuristic",
        )
        .unwrap_or(false),
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
    apply_gateway_profile_reference(cli_key, &mut meta);
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
    merge_model_catalog_image_capabilities(&mut meta, record.get("settings_config"));
    meta
}

fn gateway_profile_reference_from_meta(value: &Value) -> Option<GatewayProviderProfileReference> {
    let reference = value
        .get("gateway_profile")
        .or_else(|| value.get("gatewayProfile"))?;
    let profile_id = json_string_compat(reference, "profile_id", "profileId")?;
    let endpoint_id = json_string_compat(reference, "endpoint_id", "endpointId")?;
    Some(GatewayProviderProfileReference {
        tool: json_value_string(reference, "tool"),
        profile_id,
        endpoint_id,
    })
}

fn apply_gateway_profile_reference(cli_key: GatewayCliKey, meta: &mut ProviderGatewayMeta) {
    let Some(reference) = meta.gateway_profile.clone() else {
        return;
    };
    let Some(tool) = gateway_profile_tool_for_cli(cli_key) else {
        return;
    };
    if !gateway_profile_reference_matches_tool(&reference, tool) {
        return;
    }
    let Some(catalog) = load_gateway_provider_profiles_for_runtime() else {
        return;
    };
    let Some((profile, endpoint)) = find_gateway_profile_endpoint(
        &catalog,
        tool,
        reference.profile_id.trim(),
        reference.endpoint_id.trim(),
    ) else {
        return;
    };

    meta.provider_type = json_value_string(profile, "providerType");
    meta.api_format = json_value_string(endpoint, "apiFormat");
    meta.api_key_field = json_value_string(endpoint, "apiKeyField")
        .or_else(|| json_value_string(profile, "apiKeyField"));
    meta.reasoning_field = json_value_string(endpoint, "reasoningField")
        .or_else(|| json_value_string(profile, "reasoningField"));
    meta.default_max_tokens = json_i64_compat(endpoint, "default_max_tokens", "defaultMaxTokens")
        .or_else(|| json_i64_compat(profile, "default_max_tokens", "defaultMaxTokens"));
    meta.image_input_policy = json_value_string(endpoint, "imageInputPolicy")
        .or_else(|| json_value_string(profile, "imageInputPolicy"));
    meta.text_only_models = {
        let endpoint_models =
            json_string_vec_compat(endpoint, "text_only_models", "textOnlyModels");
        if endpoint_models.is_empty() {
            json_string_vec_compat(profile, "text_only_models", "textOnlyModels")
        } else {
            endpoint_models
        }
    };
    meta.image_capable_models = {
        let endpoint_models =
            json_string_vec_compat(endpoint, "image_capable_models", "imageCapableModels");
        if endpoint_models.is_empty() {
            json_string_vec_compat(profile, "image_capable_models", "imageCapableModels")
        } else {
            endpoint_models
        }
    };
    meta.allow_text_only_model_heuristic = json_bool_compat(
        endpoint,
        "allow_text_only_model_heuristic",
        "allowTextOnlyModelHeuristic",
    )
    .or_else(|| {
        json_bool_compat(
            profile,
            "allow_text_only_model_heuristic",
            "allowTextOnlyModelHeuristic",
        )
    })
    .unwrap_or(false);
    meta.codex_chat_reasoning = if tool == "codex" {
        codex_chat_reasoning_from_profile_value(endpoint)
            .or_else(|| codex_chat_reasoning_from_profile_value(profile))
    } else {
        None
    };
}

fn gateway_profile_tool_for_cli(cli_key: GatewayCliKey) -> Option<&'static str> {
    match cli_key {
        GatewayCliKey::Claude => Some("claude"),
        GatewayCliKey::Codex => Some("codex"),
        GatewayCliKey::Grok => Some("grok"),
        GatewayCliKey::Gemini => Some("gemini"),
        GatewayCliKey::OpenCode => None,
    }
}

#[derive(Debug, Clone, Default)]
struct GrokSelectedModel {
    model_id: Option<String>,
    base_url: Option<String>,
    api_backend: Option<String>,
    api_key: Option<String>,
}

fn grok_selected_model_from_settings(
    settings: &Value,
    config_toml: &str,
) -> Option<GrokSelectedModel> {
    if !config_toml.trim().is_empty() {
        if let Some(model) = grok_selected_model_from_toml(config_toml) {
            return Some(model);
        }
    }
    let default_key = settings
        .get("defaultModelKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let models = settings.pointer("/modelCatalog/models")?.as_array()?;
    let model = default_key
        .and_then(|key| {
            models.iter().find(|model| {
                model
                    .get("key")
                    .or_else(|| model.get("model"))
                    .and_then(Value::as_str)
                    == Some(key)
            })
        })
        .or_else(|| models.first())?;
    Some(GrokSelectedModel {
        model_id: json_value_string(model, "model").or_else(|| json_value_string(model, "key")),
        base_url: json_value_string(model, "baseUrl")
            .or_else(|| json_value_string(model, "base_url")),
        api_backend: json_value_string(model, "apiBackend")
            .or_else(|| json_value_string(model, "api_backend")),
        api_key: json_value_string(model, "apiKey").or_else(|| json_value_string(model, "api_key")),
    })
}

fn grok_selected_model_from_toml(config_toml: &str) -> Option<GrokSelectedModel> {
    let document = config_toml.parse::<toml_edit::DocumentMut>().ok()?;
    let root = document.as_table();
    let default_key = root
        .get("models")
        .and_then(toml_edit::Item::as_table)
        .and_then(|models| models.get("default"))
        .and_then(toml_edit::Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let models = root.get("model").and_then(toml_edit::Item::as_table)?;
    let table = default_key
        .and_then(|key| models.get(key))
        .and_then(toml_edit::Item::as_table)
        .or_else(|| models.iter().find_map(|(_, item)| item.as_table()))?;
    let string = |key: &str| {
        table
            .get(key)
            .and_then(toml_edit::Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Some(GrokSelectedModel {
        model_id: string("model").or_else(|| default_key.map(str::to_string)),
        base_url: string("base_url"),
        api_backend: string("api_backend"),
        api_key: string("api_key"),
    })
}

fn gateway_profile_reference_matches_tool(
    reference: &GatewayProviderProfileReference,
    expected_tool: &str,
) -> bool {
    reference
        .tool
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none_or(|tool| tool.eq_ignore_ascii_case(expected_tool))
}

fn find_gateway_profile_endpoint<'a>(
    catalog: &'a Value,
    tool: &str,
    profile_id: &str,
    endpoint_id: &str,
) -> Option<(&'a Value, &'a Value)> {
    let profile = catalog
        .get("profiles")
        .and_then(Value::as_array)?
        .iter()
        .find(|profile| {
            profile
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|id| id == profile_id)
        })?;
    let endpoint = profile
        .get("tools")
        .and_then(|tools| tools.get(tool))
        .and_then(|tool_profile| tool_profile.get("endpoints"))
        .and_then(Value::as_array)?
        .iter()
        .find(|endpoint| {
            endpoint
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|id| id == endpoint_id)
        })?;
    Some((profile, endpoint))
}

fn codex_chat_reasoning_from_profile_value(value: &Value) -> Option<CodexChatReasoningMeta> {
    value
        .get("codex_chat_reasoning")
        .or_else(|| value.get("codexChatReasoning"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
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

fn json_i64_compat(value: &Value, snake_key: &str, camel_key: &str) -> Option<i64> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
        })
}

fn json_string_vec_compat(value: &Value, snake_key: &str, camel_key: &str) -> Vec<String> {
    value
        .get(snake_key)
        .or_else(|| value.get(camel_key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn merge_model_catalog_image_capabilities(
    meta: &mut ProviderGatewayMeta,
    settings_config: Option<&Value>,
) {
    let Some(settings_config) = settings_config else {
        return;
    };
    let settings_value = match settings_config {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        Value::Object(_) => Some(settings_config.clone()),
        _ => None,
    };
    let Some(settings_value) = settings_value else {
        return;
    };
    let Some(models) = settings_value
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .or_else(|| settings_value.get("models"))
        .and_then(Value::as_array)
    else {
        return;
    };

    for model in models {
        let Some(model_id) = model_catalog_model_id(model) else {
            continue;
        };
        match model_catalog_image_support(model) {
            Some(true) => push_unique_string(&mut meta.image_capable_models, model_id),
            Some(false) => push_unique_string(&mut meta.text_only_models, model_id),
            None => {}
        }
    }
}

fn model_catalog_model_id(model: &Value) -> Option<String> {
    [
        model.get("model"),
        model.get("id"),
        model.get("name"),
        model.get("modelId"),
        model.get("model_id"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_string)
}

fn model_catalog_image_support(model: &Value) -> Option<bool> {
    if let Some(value) = model
        .get("supportsImage")
        .or_else(|| model.get("supports_image"))
        .or_else(|| model.get("vision"))
        .or_else(|| model.get("attachment"))
        .and_then(Value::as_bool)
    {
        return Some(value);
    }
    [
        model.get("input"),
        model.pointer("/modalities/input"),
        model.get("inputModalities"),
        model.get("input_modalities"),
    ]
    .into_iter()
    .flatten()
    .find_map(input_modalities_support_image)
}

fn input_modalities_support_image(value: &Value) -> Option<bool> {
    let modalities = value.as_array()?;
    Some(modalities.iter().any(|item| {
        item.as_str()
            .map(str::trim)
            .is_some_and(|item| item.eq_ignore_ascii_case("image"))
    }))
}

fn push_unique_string(items: &mut Vec<String>, item: String) {
    if !items.iter().any(|existing| existing == &item) {
        items.push(item);
    }
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
        .and_then(super::super::provider_protocol::codex_wire_api_from_config)
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
        fable_model: json_value_string(settings, "fableModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_DEFAULT_FABLE_MODEL")),
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
    super::super::provider_protocol::codex_base_url_from_config(config_toml)
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
    fn grok_selected_model_uses_default_toml_table() {
        let config = r#"
[models]
default = "custom"

[model.other]
model = "other-upstream"
base_url = "https://other.example.com/v1"
api_backend = "chat_completions"

[model.custom]
model = "custom-upstream"
base_url = "https://custom.example.com/v1"
api_backend = "messages"
api_key = "secret"
"#;
        let selected = grok_selected_model_from_toml(config).expect("selected model");
        assert_eq!(selected.model_id.as_deref(), Some("custom-upstream"));
        assert_eq!(
            selected.base_url.as_deref(),
            Some("https://custom.example.com/v1")
        );
        assert_eq!(selected.api_backend.as_deref(), Some("messages"));
        assert_eq!(selected.api_key.as_deref(), Some("secret"));
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
            "fableModel": "provider-fable",
            "reasoningModel": "provider-reasoning",
            "env": {
                "ANTHROPIC_REASONING_MODEL": "env-reasoning"
            }
        }));

        assert_eq!(mapping.default_model.as_deref(), Some("provider-default"));
        assert_eq!(mapping.fable_model.as_deref(), Some("provider-fable"));
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
    fn claude_raw_url_suffix_selects_openai_chat_target_and_bearer_auth() {
        let result = provider_from_record(
            GatewayCliKey::Claude,
            serde_json::json!({
                "id": "claude-openai-chat",
                "name": "Claude OpenAI Chat",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://openrouter.example.com/v1/chat/completions##",
                        "OPENAI_API_KEY": "openai-key"
                    }
                }).to_string(),
                "meta": {
                    "apiFormat": "openai_chat"
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.target_protocol, AiProtocol::OpenAiChat);
        assert_eq!(result.auth_strategy, ProviderAuthStrategy::Bearer);
        assert_eq!(
            result.base_url,
            "https://openrouter.example.com/v1/chat/completions"
        );
        assert!(result.is_full_url);
    }

    #[test]
    fn provider_meta_reads_reasoning_field_policy() {
        let result = provider_from_record(
            GatewayCliKey::Codex,
            serde_json::json!({
                "id": "codex-openrouter",
                "name": "Codex OpenRouter",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "auth": {"OPENAI_API_KEY": "openrouter-key"},
                    "config": r#"
model_provider = "custom"
[model_providers.custom]
base_url = "https://openrouter.ai/api/v1"
"#
                }).to_string(),
                "meta": {
                    "providerType": "openrouter",
                    "apiFormat": "openai_chat",
                    "reasoningField": "reasoning"
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(result.meta.provider_type.as_deref(), Some("openrouter"));
        assert_eq!(result.target_protocol, AiProtocol::OpenAiChat);
        assert_eq!(result.meta.reasoning_field.as_deref(), Some("reasoning"));
    }

    #[test]
    fn provider_meta_reads_codex_chat_reasoning_config() {
        let result = provider_from_record(
            GatewayCliKey::Codex,
            serde_json::json!({
                "id": "codex-deepseek",
                "name": "Codex DeepSeek",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "auth": {"OPENAI_API_KEY": "deepseek-key"},
                    "config": r#"
model_provider = "custom"
[model_providers.custom]
base_url = "https://api.deepseek.com/v1"
"#
                }).to_string(),
                "meta": {
                    "providerType": "deepseek",
                    "apiFormat": "openai_chat",
                    "codexChatReasoning": {
                        "supportsThinking": true,
                        "supportsEffort": true,
                        "thinkingParam": "thinking",
                        "effortParam": "reasoning_effort",
                        "effortValueMode": "deepseek",
                        "outputFormat": "reasoning_content"
                    }
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        let config = result
            .meta
            .codex_chat_reasoning
            .as_ref()
            .expect("codex chat reasoning config");
        assert_eq!(config.supports_thinking, Some(true));
        assert_eq!(config.supports_effort, Some(true));
        assert_eq!(config.thinking_param.as_deref(), Some("thinking"));
        assert_eq!(config.effort_param.as_deref(), Some("reasoning_effort"));
        assert_eq!(config.effort_value_mode.as_deref(), Some("deepseek"));
        assert_eq!(config.output_format.as_deref(), Some("reasoning_content"));
    }

    #[test]
    fn provider_meta_extracts_model_catalog_image_capabilities() {
        let result = provider_from_record(
            GatewayCliKey::Codex,
            serde_json::json!({
                "id": "codex-catalog",
                "name": "Codex Catalog",
                "category": "custom",
                "settings_config": serde_json::json!({
                    "auth": {"OPENAI_API_KEY": "catalog-key"},
                    "config": r#"
model_provider = "custom"
[model_providers.custom]
base_url = "https://api.example.com/v1"
"#,
                    "modelCatalog": {
                        "models": [
                            {
                                "model": "text-only-model",
                                "modalities": {"input": ["text"], "output": ["text"]}
                            },
                            {
                                "model": "vision-model",
                                "modalities": {"input": ["text", "image"], "output": ["text"]}
                            }
                        ]
                    }
                }).to_string(),
                "meta": {
                    "apiFormat": "openai_chat"
                },
                "is_disabled": false
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            result.meta.text_only_models,
            vec!["text-only-model".to_string()]
        );
        assert_eq!(
            result.meta.image_capable_models,
            vec!["vision-model".to_string()]
        );
    }

    #[test]
    fn provider_meta_resolves_gateway_profile_reference_from_catalog() {
        let meta = provider_meta_from_record(
            GatewayCliKey::Codex,
            &serde_json::json!({
                "category": "custom",
                "meta": {
                    "gatewayProfile": {
                        "tool": "codex",
                        "profileId": "deepseek",
                        "endpointId": "openai_chat"
                    }
                }
            }),
            None,
        );

        assert_eq!(meta.provider_type.as_deref(), Some("deepseek"));
        assert_eq!(meta.api_format.as_deref(), Some("openai_chat"));
        assert_eq!(
            meta.codex_chat_reasoning
                .as_ref()
                .and_then(|config| config.effort_value_mode.as_deref()),
            Some("deepseek")
        );
    }

    #[test]
    fn provider_meta_gateway_profile_reference_preserves_user_owned_fields() {
        let meta = provider_meta_from_record(
            GatewayCliKey::Claude,
            &serde_json::json!({
                "category": "custom",
                "meta": {
                    "gatewayProfile": {
                        "tool": "claude",
                        "profileId": "deepseek",
                        "endpointId": "openai_chat"
                    },
                    "costMultiplier": "1.25",
                    "pricingModelSource": "requested",
                    "isFullUrl": true,
                    "promptCacheKey": "session-key"
                }
            }),
            None,
        );

        assert_eq!(meta.provider_type.as_deref(), Some("deepseek"));
        assert_eq!(meta.api_format.as_deref(), Some("openai_chat"));
        assert_eq!(meta.cost_multiplier, "1.25");
        assert_eq!(meta.pricing_model_source, "requested");
        assert!(meta.is_full_url);
        assert_eq!(meta.prompt_cache_key.as_deref(), Some("session-key"));
    }

    #[test]
    fn provider_meta_gateway_profile_reference_uses_current_profile_fields() {
        let meta = provider_meta_from_record(
            GatewayCliKey::Gemini,
            &serde_json::json!({
                "category": "custom",
                "meta": {
                    "gatewayProfile": {
                        "tool": "gemini",
                        "profileId": "openrouter",
                        "endpointId": "openai_chat"
                    }
                }
            }),
            None,
        );

        assert_eq!(meta.provider_type.as_deref(), Some("openrouter"));
        assert_eq!(meta.api_format.as_deref(), Some("openai_chat"));
        assert_eq!(meta.reasoning_field.as_deref(), Some("reasoning"));
    }

    #[test]
    fn provider_meta_gateway_profile_reference_does_not_apply_codex_reasoning_for_gemini() {
        let meta = provider_meta_from_record(
            GatewayCliKey::Gemini,
            &serde_json::json!({
                "category": "custom",
                "meta": {
                    "gatewayProfile": {
                        "tool": "gemini",
                        "profileId": "openrouter",
                        "endpointId": "openai_chat"
                    },
                    "codexChatReasoning": {
                        "supportsEffort": true,
                        "effortValueMode": "legacy"
                    }
                }
            }),
            None,
        );

        assert_eq!(meta.provider_type.as_deref(), Some("openrouter"));
        assert!(meta.codex_chat_reasoning.is_none());
    }

    #[test]
    fn provider_meta_gateway_profile_reference_falls_back_to_legacy_when_missing() {
        let meta = provider_meta_from_record(
            GatewayCliKey::Codex,
            &serde_json::json!({
                "category": "custom",
                "meta": {
                    "gatewayProfile": {
                        "tool": "codex",
                        "profileId": "missing-profile",
                        "endpointId": "openai_chat"
                    },
                    "providerType": "legacy-provider",
                    "apiFormat": "openai_chat"
                }
            }),
            None,
        );

        assert_eq!(meta.provider_type.as_deref(), Some("legacy-provider"));
        assert_eq!(meta.api_format.as_deref(), Some("openai_chat"));
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
