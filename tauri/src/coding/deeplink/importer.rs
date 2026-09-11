use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::Mutex;

use crate::coding::db_id::db_extract_id;
use crate::coding::proxy_gateway::provider_profiles::load_gateway_provider_profiles_for_runtime;
use crate::coding::{
    claude_code, claude_desktop, codex, dsh, gemini_cli, grok, hermes, kimi, oh_my_pi, open_claw,
    open_code, pi,
};
use crate::db::helpers::db_list;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;

use super::parser::{validate_url, DeepLinkImportRequest, SUPPORTED_APPS, SUPPORTED_RESOURCE};
use super::portable::{
    adapt_base_url, canonical_api_format, catalog_profile, is_database_app, native_format,
    profile_endpoint, remap_profile, static_headers, supports_native_auth, SharedModel,
};
use super::provider::{
    build_catalog_settings, build_claude_settings, build_codex_settings, build_desktop_routes,
    build_gemini_settings, build_native_provider, slugify,
};

static IMPORT_LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportConflictPolicy {
    #[default]
    Skip,
    Copy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkImportPreview {
    pub api_format: String,
    pub base_url: Option<String>,
    pub requires_gateway: bool,
    pub writes_runtime_files: bool,
    pub profile_preserved: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkImportResult {
    #[serde(rename = "type")]
    pub kind: String,
    pub app: String,
    pub id: String,
    pub name: String,
    pub status: String,
    pub requires_gateway: bool,
}

pub(super) fn prepare_import(
    request: &DeepLinkImportRequest,
    catalog: &Value,
) -> Result<(DeepLinkImportRequest, Option<Value>, DeepLinkImportPreview), String> {
    let mut request = request.clone();
    if request.resource != SUPPORTED_RESOURCE || !SUPPORTED_APPS.contains(&request.app.as_str()) {
        return Err("Unsupported provider import target".to_string());
    }
    request.name = request.name.trim().to_string();
    for value in [
        &mut request.api_key,
        &mut request.base_url,
        &mut request.homepage,
    ] {
        *value = value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    if request.name.is_empty() {
        return Err("Provider name is required".to_string());
    }
    if let Some(source) = &request.connection.source_app {
        if !SUPPORTED_APPS.contains(&source.as_str()) {
            return Err("Unsupported provider source tool".to_string());
        }
        if request
            .base_url
            .as_deref()
            .is_none_or(|url| url.trim().is_empty())
        {
            return Err("Provider Base URL is required for cross-tool sharing".to_string());
        }
    }
    for (field, value) in [
        ("baseUrl", &request.base_url),
        ("homepage", &request.homepage),
    ] {
        if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
            validate_url(value, field).map_err(|error| error.to_string())?;
        }
    }
    if (request.config.is_some() || request.extra.is_some())
        && !matches!(request.app.as_str(), "claude" | "codex" | "gemini")
    {
        return Err("Tool-specific config blobs cannot be imported into another tool".to_string());
    }
    if request.config.is_some() || request.extra.is_some() {
        let original_app = request.connection.source_app.clone().or_else(|| {
            super::parser::parse_deeplink_url(&request.raw_url)
                .ok()
                .map(|original| original.app)
        });
        if original_app.is_some_and(|app| app != request.app) {
            return Err(
                "Tool-specific config blobs cannot be imported into another tool".to_string(),
            );
        }
        return prepare_legacy_import(request);
    }
    let source_profile = request
        .connection
        .gateway_profile
        .as_ref()
        .and_then(|reference| catalog_profile(catalog, &reference.profile_id));
    let profile_format = request
        .connection
        .gateway_profile
        .as_ref()
        .and_then(|reference| {
            profile_endpoint(
                source_profile?,
                reference.tool.as_deref()?,
                &reference.endpoint_id,
            )?["apiFormat"]
                .as_str()
        });
    if request.connection.gateway_profile.is_some() && profile_format.is_none() {
        return Err("The shared built-in channel is unavailable; select the API protocol explicitly to use a custom connection".to_string());
    }
    let raw_format = profile_format
        .or(request.connection.api_format.as_deref())
        .unwrap_or(native_format(&request.app));
    let api_format = canonical_api_format(raw_format)
        .ok_or_else(|| format!("Unsupported API format '{raw_format}'"))?
        .to_string();
    request.connection.api_format = Some(api_format.clone());
    if let Some(base_url) = &request.base_url {
        request.base_url = Some(adapt_base_url(
            &mut request.connection,
            &request.app,
            base_url,
        ));
    }
    let mut unique_models = Vec::<SharedModel>::new();
    let source_is_claude = matches!(
        request.connection.source_app.as_deref(),
        Some("claude" | "claudedesktop")
    );
    let normalize_model = |model: &str| {
        if source_is_claude {
            super::portable::strip_context_marker(model)
        } else {
            model.trim().to_string()
        }
    };
    for model in &request.connection.models {
        let mut model = model.clone();
        model.id = normalize_model(&model.id);
        if model.id.is_empty() {
            return Err("Model ID cannot be empty".to_string());
        }
        if !unique_models.iter().any(|existing| existing.id == model.id) {
            unique_models.push(model);
        }
    }
    request.model = request
        .model
        .as_deref()
        .map(normalize_model)
        .filter(|model| !model.is_empty());
    if let Some(model) = &request.model {
        if !unique_models.iter().any(|existing| &existing.id == model) {
            unique_models.insert(
                0,
                SharedModel {
                    id: model.clone(),
                    ..Default::default()
                },
            );
        }
    }
    for model in request.connection.model_roles.values_mut() {
        *model = super::portable::strip_context_marker(model);
        if model.is_empty() {
            return Err("Role model ID cannot be empty".to_string());
        }
        if !unique_models.iter().any(|entry| &entry.id == model) {
            unique_models.push(SharedModel {
                id: model.clone(),
                ..Default::default()
            });
        }
    }
    request.connection.models = unique_models;
    if request.connection.source_app.is_some()
        && matches!(request.app.as_str(), "claude" | "gemini")
        && !request.connection.models.is_empty()
        && request.model.is_none()
        && (request.app == "gemini" || request.connection.model_roles.is_empty())
    {
        return Err("Select a default model for this target tool".to_string());
    }
    if !matches!(
        request.app.as_str(),
        "claude" | "claudedesktop" | "codex" | "gemini"
    ) && request.connection.models.is_empty()
    {
        return Err("Add at least one model before importing into this tool".to_string());
    }
    if request
        .connection
        .model_roles
        .keys()
        .any(|role| !matches!(role.as_str(), "haiku" | "sonnet" | "opus" | "fable"))
    {
        return Err("Unsupported model role".to_string());
    }
    for header in &request.connection.headers {
        let names = match header.op.as_str() {
            "set" | "delete" => vec![header.name.as_str()],
            "rename" | "copy" => vec![header.from.as_str(), header.to.as_str()],
            _ => return Err("Unsupported header operation".to_string()),
        };
        for name in names {
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "Invalid request header name".to_string())?;
        }
        if header.op == "set" {
            reqwest::header::HeaderValue::from_str(&header.value)
                .map_err(|_| "Invalid request header value".to_string())?;
        }
    }
    let provider_type = source_profile
        .and_then(|profile| profile["providerType"].as_str())
        .or(request.connection.provider_type.as_deref())
        .unwrap_or("");
    let needs_adapter = api_format == "ollama/chat"
        || matches!(
            provider_type,
            "ollama" | "github_copilot" | "copilot" | "codex" | "bedrock" | "vertex"
        );
    let profile_auth_field = request
        .connection
        .gateway_profile
        .as_ref()
        .and_then(|reference| {
            profile_endpoint(
                source_profile?,
                reference.tool.as_deref()?,
                &reference.endpoint_id,
            )?["apiKeyField"]
                .as_str()
        })
        .or_else(|| source_profile.and_then(|profile| profile["apiKeyField"].as_str()));
    if let Some(field) = profile_auth_field {
        request.connection.api_key_field = Some(field.to_string());
    }
    let database_app = is_database_app(&request.app);
    if !database_app {
        if needs_adapter {
            return Err(
                "This channel requires a Gateway provider adapter; choose a Gateway-enabled tool"
                    .to_string(),
            );
        }
        if request
            .base_url
            .as_deref()
            .is_some_and(|url| url.ends_with("##"))
        {
            return Err("Full endpoint URLs require a Gateway-enabled tool".to_string());
        }
        static_headers(&request.connection.headers)?;
        if matches!(request.app.as_str(), "dsh" | "hermes")
            && !request.connection.headers.is_empty()
        {
            return Err(
                "This tool cannot import custom request headers; choose a Gateway-enabled tool"
                    .to_string(),
            );
        }
        if request.app == "dsh" && api_format == "gemini_native" {
            return Err("DSH does not support Gemini Native provider routes".to_string());
        }
        if !supports_native_auth(
            &request.app,
            &api_format,
            request.connection.api_key_field.as_deref(),
        ) {
            return Err("This tool cannot preserve the source authentication method; choose a Gateway-enabled tool".to_string());
        }
        if api_format == "gemini_native"
            && request.app != "opencode"
            && request
                .connection
                .api_version
                .as_deref()
                .is_some_and(|version| version != "v1beta")
        {
            return Err("This tool cannot preserve the selected Gemini API version; choose Gemini CLI or OpenCode".to_string());
        }
    }
    let target_profile = request
        .connection
        .gateway_profile
        .as_ref()
        .and_then(|reference| remap_profile(reference, &request.app, &api_format, catalog));
    if database_app
        && needs_adapter
        && target_profile.is_none()
        && request.connection.gateway_profile.is_some()
    {
        return Err("The target tool has no matching built-in channel endpoint".to_string());
    }
    let mut meta = json!({});
    if let Some(reference) = &target_profile {
        meta["gatewayProfile"] = json!(reference);
    } else {
        meta["apiFormat"] = json!(api_format);
        if let Some(provider_type) = &request.connection.provider_type {
            meta["providerType"] = json!(provider_type);
        }
        if let Some(field) = &request.connection.api_key_field {
            meta["apiKeyField"] = json!(field);
        }
    }
    if !request.connection.headers.is_empty() {
        meta["customHeaders"] = json!(request.connection.headers);
    }
    if request.app == "claudedesktop" && !request.connection.models.is_empty() {
        meta["claudeDesktopModelRoutes"] = build_desktop_routes(&request);
    }
    let protocol_mismatch = if request.app == "grok" {
        api_format == "gemini_native"
    } else {
        api_format != native_format(&request.app)
    };
    let desktop_model_mapping = request.app == "claudedesktop"
        && request
            .connection
            .models
            .iter()
            .any(|model| !claude_desktop::config_writer::is_claude_safe_model_id(&model.id));
    let preview = DeepLinkImportPreview {
        api_format,
        base_url: request.base_url.clone(),
        requires_gateway: database_app
            && (protocol_mismatch
                || desktop_model_mapping
                || needs_adapter
                || !request.connection.headers.is_empty()
                || !supports_native_auth(
                    &request.app,
                    request.connection.api_format.as_deref().unwrap_or_default(),
                    request.connection.api_key_field.as_deref(),
                )
                || request
                    .base_url
                    .as_deref()
                    .is_some_and(|url| url.ends_with("##"))),
        writes_runtime_files: !database_app,
        profile_preserved: target_profile.is_some(),
    };
    Ok((request, database_app.then_some(meta), preview))
}

fn prepare_legacy_import(
    mut request: DeepLinkImportRequest,
) -> Result<(DeepLinkImportRequest, Option<Value>, DeepLinkImportPreview), String> {
    // Old config/extra links replace the tool payload. Do not attach new
    // protocol meta or a catalog synthesized from unrelated flat URL fields.
    request.connection = Default::default();
    let raw_settings = match request.app.as_str() {
        "claude" => build_claude_settings(&request)?.0,
        "codex" => build_codex_settings(&request)?,
        "gemini" => build_gemini_settings(&request)?,
        _ => return Err("Unsupported legacy config target".to_string()),
    };
    let settings: Value = serde_json::from_str(&raw_settings)
        .map_err(|_| "Legacy provider config must be valid JSON".to_string())?;
    if !settings.is_object() {
        return Err("Legacy provider config must be an object".to_string());
    }
    let (format, base_url) = if request.app == "codex" {
        let config = settings["config"].as_str().unwrap_or_default();
        toml::from_str::<toml::Value>(config)
            .map_err(|_| "Legacy Codex config must be valid TOML".to_string())?;
        let wire =
            crate::coding::proxy_gateway::provider_protocol::codex_wire_api_from_config(config);
        (
            wire.as_deref()
                .and_then(canonical_api_format)
                .unwrap_or("openai_responses")
                .to_string(),
            crate::coding::proxy_gateway::provider_protocol::codex_base_url_from_config(config),
        )
    } else {
        let explicit = settings
            .get("apiFormat")
            .or_else(|| settings.get("api_format"))
            .and_then(Value::as_str)
            .and_then(canonical_api_format);
        let format = explicit.unwrap_or_else(|| {
            if request.app == "claude" && settings["openrouter_compat_mode"] == true {
                "openai_chat"
            } else {
                native_format(&request.app)
            }
        });
        let key = if request.app == "claude" {
            "ANTHROPIC_BASE_URL"
        } else {
            "GOOGLE_GEMINI_BASE_URL"
        };
        (
            format.to_string(),
            settings["env"][key].as_str().map(str::to_string),
        )
    };
    let preview = DeepLinkImportPreview {
        requires_gateway: format != native_format(&request.app),
        api_format: format,
        base_url,
        writes_runtime_files: false,
        profile_preserved: false,
    };
    Ok((request, None, preview))
}

#[tauri::command]
pub fn preview_deeplink_import(
    request: DeepLinkImportRequest,
) -> Result<DeepLinkImportPreview, String> {
    let catalog = load_gateway_provider_profiles_for_runtime().unwrap_or(Value::Null);
    prepare_import(&request, &catalog).map(|(_, _, preview)| preview)
}

#[derive(Debug)]
struct ExistingProvider {
    id: String,
    name: String,
    source_id: Option<String>,
}

async fn existing_providers(
    state: State<'_, SqliteDbState>,
    app: &str,
) -> Result<Vec<ExistingProvider>, String> {
    let table = match app {
        "claude" => Some(DbTable::ClaudeProvider),
        "claudedesktop" => Some(DbTable::ClaudeDesktopProvider),
        "codex" => Some(DbTable::CodexProvider),
        "grok" => Some(DbTable::GrokProvider),
        "kimi" => Some(DbTable::KimiProvider),
        "gemini" => Some(DbTable::GeminiCliProvider),
        _ => None,
    };
    if let Some(table) = table {
        return state
            .db()
            .with_conn(|conn| db_list(conn, table, None))
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|row| {
                        let id = db_extract_id(&row);
                        (id != "__common__").then(|| ExistingProvider {
                            id,
                            name: row["name"].as_str().unwrap_or_default().to_string(),
                            source_id: super::portable::first_string(
                                &row,
                                &["source_provider_id", "sourceProviderId"],
                            ),
                        })
                    })
                    .collect()
            });
    }
    let pairs: Vec<(String, String)> = match app {
        "opencode" => {
            let mut providers: Vec<_> = read_opencode(state)
                .await?
                .provider
                .unwrap_or_default()
                .into_iter()
                .map(|(id, provider)| (id.clone(), provider.name.unwrap_or(id)))
                .collect();
            for id in open_code::free_models::read_auth_channels() {
                if !providers.iter().any(|(key, _)| key == &id) {
                    providers.push((id.clone(), id));
                }
            }
            providers
        }
        "openclaw" => read_openclaw(state)
            .await?
            .models
            .and_then(|models| models.providers)
            .unwrap_or_default()
            .into_iter()
            .map(|(id, _)| (id.clone(), id))
            .collect(),
        "pi" => {
            let config = pi::read_pi_runtime_config(state).await?;
            if config
                .models
                .get("providers")
                .is_some_and(|providers| !providers.is_object())
            {
                return Err("models.providers must be an object".to_string());
            }
            config
                .providers
                .into_iter()
                .filter(|provider| {
                    provider.models_provider.is_some() || provider.credential.is_some()
                })
                .map(|provider| (provider.provider_key, provider.display_name))
                .collect()
        }
        "omp" => {
            let config = oh_my_pi::read_omp_runtime_config(state).await?;
            if config
                .models
                .get("providers")
                .is_some_and(|providers| !providers.is_object())
            {
                return Err("models.providers must be a mapping".to_string());
            }
            config
                .providers
                .into_iter()
                .filter(|provider| {
                    provider.models_provider.is_some() || provider.credential.is_some()
                })
                .map(|provider| (provider.provider_key, provider.display_name))
                .collect()
        }
        "hermes" => {
            let config = hermes::read_hermes_runtime_config(state).await?;
            if config
                .config
                .get("custom_providers")
                .is_some_and(|providers| !providers.is_array())
            {
                return Err("custom_providers must be a list".to_string());
            }
            config
                .providers
                .into_iter()
                .filter(|provider| provider.provider.is_some() || provider.credential.is_some())
                .map(|provider| (provider.provider_key, provider.display_name))
                .collect()
        }
        "dsh" => dsh::read_dsh_runtime_config(state)
            .await?
            .providers
            .into_iter()
            .filter(|provider| provider.provider.is_some() || provider.credential_exists)
            .map(|provider| (provider.provider_key, provider.display_name))
            .collect(),
        _ => return Err("Unsupported provider import target".to_string()),
    };
    Ok(pairs
        .into_iter()
        .map(|(id, name)| ExistingProvider {
            id,
            name,
            source_id: None,
        })
        .collect())
}

async fn read_opencode(
    state: State<'_, SqliteDbState>,
) -> Result<open_code::OpenCodeConfig, String> {
    match open_code::read_opencode_config(state).await? {
        open_code::ReadConfigResult::Success { config } => Ok(config),
        open_code::ReadConfigResult::NotFound { .. } => {
            serde_json::from_value(json!({"provider": {}})).map_err(|error| error.to_string())
        }
        open_code::ReadConfigResult::Error { error }
        | open_code::ReadConfigResult::ParseError { error, .. } => Err(error),
    }
}

async fn read_openclaw(
    state: State<'_, SqliteDbState>,
) -> Result<open_claw::OpenClawConfig, String> {
    match open_claw::read_openclaw_config(state).await? {
        open_claw::ReadOpenClawConfigResult::Success { config } => Ok(config),
        open_claw::ReadOpenClawConfigResult::NotFound { .. } => {
            serde_json::from_value(json!({})).map_err(|error| error.to_string())
        }
        open_claw::ReadOpenClawConfigResult::Error { error }
        | open_claw::ReadOpenClawConfigResult::ParseError { error, .. } => Err(error),
    }
}

pub(super) async fn build_and_create_provider<R: Runtime>(
    state: State<'_, SqliteDbState>,
    app: &AppHandle<R>,
    request: &DeepLinkImportRequest,
    policy: ImportConflictPolicy,
) -> Result<DeepLinkImportResult, String> {
    let _guard = IMPORT_LOCK.lock().await;
    let catalog = load_gateway_provider_profiles_for_runtime().unwrap_or(Value::Null);
    let (mut request, meta, preview) = prepare_import(request, &catalog)?;
    let existing = existing_providers(state.clone(), &request.app).await?;
    let key = native_provider_key(&request.name);
    let conflict = existing.iter().find(|provider| {
        provider.name.eq_ignore_ascii_case(&request.name)
            || (!is_database_app(&request.app) && provider.id == key)
            || request
                .source_provider_id
                .as_ref()
                .is_some_and(|source| provider.source_id.as_ref() == Some(source))
    });
    if let Some(provider) = conflict {
        if policy == ImportConflictPolicy::Skip {
            return Ok(DeepLinkImportResult {
                kind: "provider".to_string(),
                app: request.app,
                id: provider.id.clone(),
                name: provider.name.clone(),
                status: "skipped".to_string(),
                requires_gateway: preview.requires_gateway,
            });
        }
        let original_name = request.name.clone();
        let mut suffix = 2;
        loop {
            request.name = format!("{original_name} ({suffix})");
            if !existing.iter().any(|provider| {
                provider.name.eq_ignore_ascii_case(&request.name)
                    || provider.id == native_provider_key(&request.name)
            }) {
                break;
            }
            suffix += 1;
        }
    }
    let key = native_provider_key(&request.name);
    let mut input = json!({
        "name": request.name, "category": request.category, "sourceProviderId": request.source_provider_id,
        "websiteUrl": request.homepage, "notes": request.notes, "icon": request.icon, "iconColor": request.icon_color, "meta": meta,
    });
    let parse_error = |error: serde_json::Error| format!("Invalid provider configuration: {error}");
    let id = match request.app.as_str() {
        "claude" => {
            let (settings, extra) = build_claude_settings(&request)?;
            input["settingsConfig"] = json!(settings);
            input["extraSettingsConfig"] = json!(extra);
            claude_code::create_claude_provider_inner(
                &state,
                app,
                serde_json::from_value(input).map_err(parse_error)?,
            )
            .await?
            .id
        }
        "claudedesktop" => {
            input["settingsConfig"] = json!(build_claude_settings(&request)?.0);
            claude_desktop::create_claude_desktop_provider(
                state.clone(),
                app.clone(),
                serde_json::from_value(input).map_err(parse_error)?,
            )
            .await?
            .id
        }
        "codex" => {
            input["settingsConfig"] = json!(build_codex_settings(&request)?);
            codex::create_codex_provider_inner(
                &state,
                app,
                serde_json::from_value(input).map_err(parse_error)?,
            )
            .await?
            .id
        }
        "gemini" => {
            input["settingsConfig"] = json!(build_gemini_settings(&request)?);
            gemini_cli::create_gemini_cli_provider_inner(
                &state,
                app,
                serde_json::from_value(input).map_err(parse_error)?,
            )
            .await?
            .id
        }
        "grok" => {
            input["settingsConfig"] = json!(build_catalog_settings(&request)?);
            grok::create_grok_provider_inner(
                &state,
                app,
                serde_json::from_value(input).map_err(parse_error)?,
            )
            .await?
            .id
        }
        "kimi" => {
            input["settingsConfig"] = json!(build_catalog_settings(&request)?);
            kimi::create_kimi_provider(
                state.clone(),
                app.clone(),
                serde_json::from_value(input).map_err(parse_error)?,
            )
            .await?
            .id
        }
        "opencode" => {
            let mut config = read_opencode(state.clone()).await?;
            config.provider.get_or_insert_with(Default::default).insert(
                key.clone(),
                serde_json::from_value(build_native_provider(&request)?).map_err(parse_error)?,
            );
            open_code::save_opencode_config(state.clone(), app.clone(), config).await?;
            key.clone()
        }
        "openclaw" => {
            let mut config =
                serde_json::to_value(read_openclaw(state.clone()).await?).map_err(parse_error)?;
            if !config["models"].is_object() {
                config["models"] = json!({});
            }
            if !config["models"]["providers"].is_object() {
                config["models"]["providers"] = json!({});
            }
            config["models"]["providers"][&key] = build_native_provider(&request)?;
            open_claw::save_openclaw_config(
                state.clone(),
                app.clone(),
                serde_json::from_value(config).map_err(parse_error)?,
            )
            .await?;
            key.clone()
        }
        "pi" => {
            pi::save_pi_models_provider(
                state.clone(),
                app.clone(),
                pi::types::PiModelsProviderInput {
                    provider_key: key.clone(),
                    provider: build_native_provider(&request)?,
                },
            )
            .await?;
            key.clone()
        }
        "omp" => {
            oh_my_pi::save_omp_models_provider(
                state.clone(),
                app.clone(),
                oh_my_pi::types::OmpModelsProviderInput {
                    provider_key: key.clone(),
                    provider: build_native_provider(&request)?,
                },
            )
            .await?;
            key.clone()
        }
        "hermes" => {
            hermes::save_hermes_models_provider(
                state.clone(),
                app.clone(),
                hermes::types::HermesModelsProviderInput {
                    provider_key: key.clone(),
                    provider: build_native_provider(&request)?,
                },
            )
            .await?;
            key.clone()
        }
        "dsh" => {
            let mut provider = build_native_provider(&request)?;
            let credential = request
                .api_key
                .as_ref()
                .map(|value| dsh::types::DshCredentialInput {
                    ref_name: format!(
                        "AI_TOOLBOX_SHARE_{}",
                        uuid::Uuid::new_v4().simple().to_string().to_uppercase()
                    ),
                    value: value.clone(),
                });
            if let Some(credential) = &credential {
                provider["apiKeyEnv"] = json!(credential.ref_name);
            }
            dsh::save_dsh_models_provider(
                state.clone(),
                app.clone(),
                dsh::types::DshModelsProviderInput {
                    provider_key: key.clone(),
                    provider,
                    credential,
                },
            )
            .await?;
            key.clone()
        }
        _ => return Err("Unsupported provider import target".to_string()),
    };
    Ok(DeepLinkImportResult {
        kind: "provider".to_string(),
        app: request.app,
        id,
        name: request.name,
        status: "created".to_string(),
        requires_gateway: preview.requires_gateway,
    })
}

fn native_provider_key(name: &str) -> String {
    // Imported connections must not shadow implicit built-ins or change the
    // active default just because the user shares a provider named "OpenAI".
    let slug = slugify(name);
    if name.is_ascii() {
        format!("{slug}-shared")
    } else {
        let digest = format!("{:x}", Sha256::digest(name.as_bytes()));
        format!("{slug}-{}-shared", &digest[..10])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::deeplink::parser::parse_deeplink_url;
    use crate::coding::proxy_gateway::types::GatewayProviderProfileReference;

    fn request(target: &str) -> DeepLinkImportRequest {
        parse_deeplink_url(&format!("aitoolbox://v1/import?resource=provider&app={target}&name=Relay&apiKey=test-key&baseUrl=https://relay.test&model=model-a")).unwrap()
    }

    #[test]
    fn sdk_url_prefixes_and_auth_are_adapted_without_losing_proxy_paths_or_queries() {
        let mut input = request("claude");
        input.connection.source_app = Some("opencode".to_string());
        input.connection.api_format = Some("anthropic_messages".to_string());
        input.connection.api_key_field = Some("x-api-key".to_string());
        input.base_url = Some("https://relay.test/proxy/v1?project=a%2Bb".to_string());
        let (prepared, _, preview) = prepare_import(&input, &Value::Null).unwrap();
        assert_eq!(
            preview.base_url.as_deref(),
            Some("https://relay.test/proxy?project=a%2Bb")
        );
        assert!(!preview.requires_gateway);
        let settings: Value =
            serde_json::from_str(&build_claude_settings(&prepared).unwrap().0).unwrap();
        assert_eq!(settings["env"]["ANTHROPIC_API_KEY"], "test-key");
        assert!(settings["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());

        input.app = "opencode".to_string();
        input.connection.source_app = Some("claude".to_string());
        input.connection.api_key_field = Some("bearer".to_string());
        input.base_url = Some("https://relay.test/proxy?project=a%2Bb".to_string());
        let (prepared, _, _) = prepare_import(&input, &Value::Null).unwrap();
        let provider = build_native_provider(&prepared).unwrap();
        assert_eq!(
            provider["options"]["baseURL"],
            "https://relay.test/proxy/v1?project=a%2Bb"
        );
        assert_eq!(provider["options"]["authToken"], "test-key");
        assert!(provider["options"].get("apiKey").is_none());
    }

    #[test]
    fn gemini_sdk_version_is_separate_from_its_base_url() {
        let mut input = request("gemini");
        input.connection.source_app = Some("opencode".to_string());
        input.connection.api_format = Some("gemini_native".to_string());
        input.base_url = Some("https://relay.test/google/v1alpha".to_string());
        let (prepared, _, preview) = prepare_import(&input, &Value::Null).unwrap();
        assert_eq!(
            preview.base_url.as_deref(),
            Some("https://relay.test/google")
        );
        let settings: Value =
            serde_json::from_str(&build_gemini_settings(&prepared).unwrap()).unwrap();
        assert_eq!(settings["env"]["GOOGLE_GENAI_API_VERSION"], "v1alpha");
        assert_eq!(
            settings["env"]["GOOGLE_GEMINI_BASE_URL"],
            "https://relay.test/google"
        );
    }

    #[test]
    fn profile_mapping_preserves_references_instead_of_copying_profile_snapshots() {
        let catalog = json!({ "profiles": [{ "id": "relay", "providerType": "openai", "bodyFilter": ["secret-field"], "tools": {
            "claude": { "endpoints": [{ "id": "chat", "apiFormat": "openai_chat" }] },
            "codex": { "endpoints": [{ "id": "chat", "apiFormat": "openai_chat" }] }
        } }] });
        let mut input = request("codex");
        input.connection.gateway_profile = Some(GatewayProviderProfileReference {
            tool: Some("claude".to_string()),
            profile_id: "relay".to_string(),
            endpoint_id: "chat".to_string(),
        });
        let (_, meta, preview) = prepare_import(&input, &catalog).unwrap();
        let meta = meta.unwrap();
        assert!(preview.profile_preserved);
        assert!(preview.requires_gateway);
        assert_eq!(meta["gatewayProfile"]["tool"], "codex");
        assert!(meta.get("providerType").is_none());
        assert!(meta.get("apiFormat").is_none());
        assert!(meta.get("bodyFilter").is_none());
        input.app = "kimi".to_string();
        let (_, meta, preview) = prepare_import(&input, &catalog).unwrap();
        assert!(!preview.profile_preserved);
        assert_eq!(meta.unwrap()["apiFormat"], "openai_chat");
    }

    #[test]
    fn legacy_config_cannot_be_retargeted_and_unsupported_native_adapters_are_rejected() {
        let mut input = request("claude");
        input.config = Some("{\"env\":{}}".to_string());
        input.app = "codex".to_string();
        assert!(prepare_import(&input, &Value::Null).is_err());
        let mut input = request("pi");
        input.connection.provider_type = Some("github_copilot".to_string());
        assert!(prepare_import(&input, &Value::Null).is_err());
        input.connection.provider_type = None;
        input.connection.headers =
            serde_json::from_value(json!([{ "op": "rename", "from": "X-A", "to": "X-B" }]))
                .unwrap();
        assert!(prepare_import(&input, &Value::Null).is_err());
        input.app = "claude".to_string();
        assert!(
            prepare_import(&input, &Value::Null)
                .unwrap()
                .2
                .requires_gateway
        );
    }

    #[test]
    fn desktop_native_models_use_their_own_ids_without_creating_a_gateway_mapping() {
        let mut input = request("claudedesktop");
        input.model = Some("claude-sonnet-4-6".to_string());
        let (_, meta, preview) = prepare_import(&input, &Value::Null).unwrap();
        assert!(!preview.requires_gateway);
        let meta = meta.unwrap();
        assert_eq!(
            meta["claudeDesktopModelRoutes"]["claude-sonnet-4-6"]["model"],
            "claude-sonnet-4-6"
        );
        assert!(!claude_desktop::config_writer::has_routing_models(
            Some(&meta),
            None
        ));
    }

    #[test]
    fn gateway_targets_keep_nondefault_gemini_versions_and_native_targets_do_not_drop_them() {
        let mut input = request("codex");
        input.connection.source_app = Some("opencode".to_string());
        input.connection.api_format = Some("gemini_native".to_string());
        input.base_url = Some("https://relay.test/google/v1alpha".to_string());
        let (prepared, _, preview) = prepare_import(&input, &Value::Null).unwrap();
        assert_eq!(prepared.base_url, input.base_url);
        assert!(preview.requires_gateway);
        input.app = "pi".to_string();
        assert!(prepare_import(&input, &Value::Null).is_err());
    }

    #[test]
    fn explicit_source_address_style_preserves_native_sdk_prefixes() {
        let mut input = request("claude");
        input.connection.source_app = Some("kimi".to_string());
        input.connection.api_format = Some("anthropic_messages".to_string());
        input.connection.base_url_style = Some(super::super::portable::SharedBaseUrlStyle::Root);
        input.base_url = Some("https://relay.test/prefix/v1".to_string());
        assert_eq!(
            prepare_import(&input, &Value::Null).unwrap().0.base_url,
            input.base_url
        );
    }

    #[test]
    fn unsupported_authentication_needs_gateway_and_missing_profiles_are_not_guessed() {
        let mut input = request("codex");
        input.connection.api_format = Some("openai_responses".to_string());
        input.connection.api_key_field = Some("x-api-key".to_string());
        assert!(
            prepare_import(&input, &Value::Null)
                .unwrap()
                .2
                .requires_gateway
        );
        input.app = "opencode".to_string();
        assert!(prepare_import(&input, &Value::Null).is_err());
        input.connection.gateway_profile = Some(GatewayProviderProfileReference {
            tool: Some("claude".to_string()),
            profile_id: "unavailable".to_string(),
            endpoint_id: "messages".to_string(),
        });
        input.app = "claude".to_string();
        assert!(prepare_import(&input, &Value::Null).is_err());
    }

    #[test]
    fn role_only_models_and_unicode_names_keep_distinct_identities() {
        let mut input = request("pi");
        input
            .connection
            .model_roles
            .insert("sonnet".to_string(), "role-model[1M]".to_string());
        let (prepared, _, _) = prepare_import(&input, &Value::Null).unwrap();
        assert!(prepared
            .connection
            .models
            .iter()
            .any(|model| model.id == "role-model"));
        assert_ne!(native_provider_key("渠道一"), native_provider_key("渠道二"));
        assert_eq!(native_provider_key("渠道一"), native_provider_key("渠道一"));
    }

    #[test]
    fn legacy_blobs_do_not_gain_conflicting_protocol_meta_or_model_catalogs() {
        let mut input = request("claude");
        input.config = Some(
            r#"{"apiFormat":"openai_chat","env":{"ANTHROPIC_AUTH_TOKEN":"legacy-key"}}"#
                .to_string(),
        );
        let (prepared, meta, preview) = prepare_import(&input, &Value::Null).unwrap();
        assert!(meta.is_none());
        assert!(prepared.connection.models.is_empty());
        assert_eq!(preview.api_format, "openai_chat");
        assert!(preview.requires_gateway);
        assert_eq!(
            build_claude_settings(&prepared).unwrap().0,
            input.config.unwrap()
        );

        let mut input = request("codex");
        input.config = Some("model=\"actual-model\"\nmodel_provider=\"custom\"\n[model_providers.custom]\nwire_api=\"responses\"".to_string());
        let (prepared, meta, _) = prepare_import(&input, &Value::Null).unwrap();
        let settings: Value =
            serde_json::from_str(&build_codex_settings(&prepared).unwrap()).unwrap();
        assert!(meta.is_none());
        assert!(settings.get("modelCatalog").is_none());
        assert!(settings["config"]
            .as_str()
            .unwrap()
            .contains("actual-model"));
    }

    #[test]
    fn context_markers_only_apply_to_claude_source_model_ids() {
        let mut input = request("pi");
        input.model = Some("vendor/model[1m]".to_string());
        input.connection.source_app = Some("opencode".to_string());
        assert_eq!(
            prepare_import(&input, &Value::Null)
                .unwrap()
                .0
                .model
                .as_deref(),
            Some("vendor/model[1m]")
        );
        input.connection.source_app = Some("claude".to_string());
        assert_eq!(
            prepare_import(&input, &Value::Null)
                .unwrap()
                .0
                .model
                .as_deref(),
            Some("vendor/model")
        );
    }
}
