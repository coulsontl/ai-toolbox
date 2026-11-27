//! Per-CLI dispatch for providers, models, global config files and prompts.
//!
//! This module deliberately contains no configuration parsing of its own. Each
//! arm calls the same `#[tauri::command]` implementation the desktop page uses,
//! via `AppHandle::state()`, so provider normalization, self-loop checks,
//! backups, event broadcasts and WSL sync requests are inherited automatically.

use serde_json::{json, Map, Value};
use tauri::Manager;

use super::error::{ApiError, ApiResult};
use super::redact::{has_secret_value, redact_json_string, redact_text, redact_value};
use super::ServerContext;
use crate::db::SqliteDbState;

pub const CLI_KEYS: [&str; 17] = [
    "claudecode",
    "claudedesktop",
    "codex",
    "grok",
    "kimi",
    "geminicli",
    "antigravity",
    "opencode",
    "openclaw",
    "pi",
    "oh_my_pi",
    "omo_native",
    "hermes",
    "dsh",
    "zcode",
    "workbuddy",
    "codebuddy",
];

pub fn normalize_cli_key(raw: &str) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    let key = match normalized.as_str() {
        "claudecode" | "claude" | "claude_code" => "claudecode",
        "claudedesktop" | "claude_desktop" => "claudedesktop",
        "codex" => "codex",
        "grok" | "grok_cli" => "grok",
        "kimi" | "kimi_cli" | "kimi_code" => "kimi",
        "geminicli" | "gemini_cli" | "gemini" => "geminicli",
        "antigravity" | "antigravity_cli" => "antigravity",
        "opencode" | "open_code" => "opencode",
        "openclaw" | "open_claw" => "openclaw",
        "pi" => "pi",
        "oh_my_pi" | "ohmy pi" | "oh-my-pi" | "omp" => "oh_my_pi",
        "omo_native" | "omo" | "omo-native" => "omo_native",
        "hermes" => "hermes",
        "dsh" => "dsh",
        "zcode" | "zcode_cli" => "zcode",
        "workbuddy" | "workbuddy_desktop" => "workbuddy",
        "codebuddy" | "codebuddy_cli" => "codebuddy",
        _ => return None,
    };
    Some(key)
}

struct CliCapability {
    kind: &'static str,
    provider_crud: bool,
    model_crud: bool,
    config: bool,
    config_read_only: bool,
    prompts: bool,
    apply: bool,
    test: bool,
}

fn capability(key: &'static str) -> CliCapability {
    let kind = match key {
        "claudecode" | "claudedesktop" | "codex" | "grok" | "kimi" | "geminicli" | "zcode" => {
            "database"
        }
        "antigravity" => "official",
        _ => "file",
    };
    let provider_crud = key != "antigravity";
    let model_crud = key != "antigravity";
    let prompts = !matches!(key, "openclaw" | "workbuddy");
    let apply = !matches!(key, "antigravity" | "workbuddy" | "codebuddy");
    let test = key != "antigravity";
    CliCapability {
        kind,
        provider_crud,
        model_crud,
        // WorkBuddy has no separate global config file; its models.json is
        // reached through the provider/model endpoints. CodeBuddy's
        // settings.json is product-owned and read-only.
        config: key != "workbuddy",
        config_read_only: key == "codebuddy",
        prompts,
        apply,
        test,
    }
}

pub fn capability_matrix() -> Value {
    let rows = CLI_KEYS
        .iter()
        .map(|key| {
            let capability = capability(key);
            json!({
                "cliKey": key,
                "kind": capability.kind,
                "providerCrud": capability.provider_crud,
                "modelCrud": capability.model_crud,
                "config": capability.config,
                "configReadOnly": capability.config_read_only,
                "prompts": capability.prompts,
                "apply": capability.apply,
                "test": capability.test,
                "promptPlans": capability.prompts,
                "paths": true,
            })
        })
        .collect::<Vec<_>>();
    json!({ "clis": rows })
}

fn app(context: &ServerContext) -> tauri::AppHandle {
    context.app.clone()
}

fn db_state(context: &ServerContext) -> tauri::State<'_, SqliteDbState> {
    context.app.state::<SqliteDbState>()
}

fn invalid(error: String) -> ApiError {
    ApiError::bad_request(error)
}

fn internal(error: String) -> ApiError {
    ApiError::internal(error)
}

/// Map the precondition strings that command-layer delete/update helpers return
/// to the HTTP codes the API documents.
///
/// These commands predate the control API and signal "missing row" or "applied
/// row cannot be removed" as ordinary `Err(String)`. Passing every one through
/// `internal` made a missing provider a 500, which tells callers the server
/// broke instead of pointing at the request.
fn command_error(error: String) -> ApiError {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("not found") {
        ApiError::not_found(error)
    } else if lowered.contains("cannot be deleted")
        || lowered.contains("cannot be disabled")
        || lowered.contains("must be saved before")
    {
        ApiError::conflict(error)
    } else {
        ApiError::internal(error)
    }
}

/// A create has no stored value to fall back to, so a `***` placeholder can
/// only mean the caller echoed a masked read. Reject it before any writer runs.
fn ensure_no_redacted_input(value: &Value, action: &str) -> ApiResult<()> {
    if super::redact::contains_redacted(value) {
        return Err(ApiError::bad_request(format!(
            "Masked secrets ('***') cannot be used to {action}; send the full value or omit the key"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Provider serialization and canonical field mapping
// ---------------------------------------------------------------------------

/// Turns a native provider DTO into the public shape: `settingsConfig` is
/// parsed and redacted, `hasApiKey` records whether a credential existed.
fn public_db_provider(mut value: Value) -> Value {
    let mut had_secret = false;
    if let Some(settings_config) = value.get("settingsConfig").and_then(Value::as_str) {
        had_secret |= settings_config_has_secret(settings_config);
        let redacted = redact_json_string(settings_config);
        value["settingsConfig"] = redacted;
    }
    if let Some(extra) = value.get("extraSettingsConfig").and_then(Value::as_str) {
        had_secret |= settings_config_has_secret(extra);
        let redacted = redact_json_string(extra);
        value["extraSettingsConfig"] = redacted;
    }
    if let Some(meta) = value.get("meta").cloned() {
        had_secret |= has_secret_value(&meta);
    }
    redact_value(&mut value);
    if let Some(object) = value.as_object_mut() {
        object.insert("hasApiKey".to_string(), Value::Bool(had_secret));
    }
    value
}

fn settings_config_has_secret(raw: &str) -> bool {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => has_secret_value(&value),
        Err(_) => redact_text(raw) != raw,
    }
}

fn public_file_provider(provider_key: &str, provider: Value, has_key: bool) -> Value {
    let mut value = json!({
        "id": provider_key,
        "providerKey": provider_key,
        "provider": provider,
        "hasApiKey": has_key,
    });
    redact_value(&mut value);
    value
}

/// Parse `settingsConfig` (a string in every DB provider) to an object.
fn settings_config_object(value: &Value) -> Map<String, Value> {
    value
        .get("settingsConfig")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|parsed| parsed.as_object().cloned())
        .unwrap_or_default()
}

fn set_settings_config(value: &mut Value, object: Map<String, Value>) {
    value["settingsConfig"] = Value::String(Value::Object(object).to_string());
}

fn validate_zcode_settings(raw: &Value) -> ApiResult<()> {
    serde_json::from_value::<crate::coding::zcode::types::ZcodeSettingsConfig>(Value::Object(
        settings_config_object(raw),
    ))
    .map(|_| ())
    .map_err(|error| invalid(format!("Invalid ZCode provider settings: {error}")))
}

/// Applies canonical `baseUrl` / `apiKey` / `model` fields to a DB provider
/// settings object. Native `settingsConfig` always wins over these shortcuts.
fn apply_canonical_fields(cli: &str, body: &Value, settings: &mut Map<String, Value>) {
    let base_url = canonical_string(body, "baseUrl");
    let api_key = canonical_string(body, "apiKey");
    let model = canonical_string(body, "model");

    match cli {
        "claudecode" | "claudedesktop" => {
            let env = ensure_object(settings, "env");
            if let Some(base_url) = base_url {
                env.insert("ANTHROPIC_BASE_URL".to_string(), json!(base_url));
            }
            if let Some(api_key) = api_key {
                env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), json!(api_key));
            }
            if let Some(model) = model {
                env.insert("ANTHROPIC_MODEL".to_string(), json!(model));
            }
        }
        "codex" => {
            let auth = ensure_object(settings, "auth");
            if let Some(api_key) = api_key {
                auth.insert("OPENAI_API_KEY".to_string(), json!(api_key));
            }
            if base_url.is_some() || model.is_some() {
                let existing = settings
                    .get("config")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Ok(patched) =
                    crate::coding::codex::commands::patch_provider_config_toml(
                        &existing, base_url, model,
                    )
                {
                    settings.insert("config".to_string(), json!(patched));
                }
            }
        }
        "grok" => {
            let auth = ensure_object(settings, "auth");
            if let Some(api_key) = api_key {
                auth.insert("API_KEY".to_string(), json!(api_key));
            }
            if let Some(base_url) = base_url {
                settings.insert("baseUrl".to_string(), json!(base_url));
            }
            // Grok custom providers project `[providers]`/`[model.*]` from the
            // catalog, so a canonical model must exist there as a row. The
            // channel address lives on both the top-level field and every row.
            let catalog = ensure_array(settings, "modelCatalog", "models");
            if let Some(base_url) = base_url {
                for row in catalog.iter_mut() {
                    if let Some(object) = row.as_object_mut() {
                        object.insert("baseUrl".to_string(), json!(base_url));
                    }
                }
            }
            match model {
                Some(model) => {
                    let position = catalog.iter().position(|row| {
                        row.get("key").and_then(Value::as_str) == Some(model)
                            || row.get("model").and_then(Value::as_str) == Some(model)
                    });
                    let position = match position {
                        Some(position) => position,
                        None => {
                            catalog.push(json!({
                                "key": model,
                                "model": model,
                                "displayName": model,
                                "apiBackend": "chat_completions",
                            }));
                            catalog.len() - 1
                        }
                    };
                    if let Some(base_url) = base_url {
                        catalog[position]["baseUrl"] = json!(base_url);
                    }
                    let key = catalog[position]
                        .get("key")
                        .and_then(Value::as_str)
                        .unwrap_or(model)
                        .to_string();
                    settings.insert("defaultModelKey".to_string(), json!(key));
                }
                None => {
                    if catalog.is_empty() {
                        let fallback = "grok-4.5";
                        catalog.push(json!({
                            "key": fallback,
                            "model": fallback,
                            "displayName": fallback,
                            "apiBackend": "chat_completions",
                        }));
                        settings.insert("defaultModelKey".to_string(), json!(fallback));
                    }
                }
            }
        }
        "kimi" => {
            let auth = ensure_object(settings, "auth");
            if let Some(api_key) = api_key {
                auth.insert("API_KEY".to_string(), json!(api_key));
            }
            if base_url.is_some() {
                let provider_key = kimi_provider_key(settings);
                let configs = ensure_object(settings, "providerConfigs");
                let entry = configs
                    .entry(provider_key)
                    .or_insert_with(|| json!({ "type": "openai" }));
                if !entry.is_object() {
                    *entry = json!({ "type": "openai" });
                }
                if let Some(base_url) = base_url {
                    entry["base_url"] = json!(base_url);
                }
            }
            if let Some(model) = model {
                let provider_key = kimi_provider_key(settings);
                let catalog = ensure_array(settings, "modelCatalog", "models");
                let position = catalog.iter().position(|row| {
                    row.get("key").and_then(Value::as_str) == Some(model)
                        || row.get("model").and_then(Value::as_str) == Some(model)
                });
                let position = match position {
                    Some(position) => position,
                    None => {
                        catalog.push(json!({
                            "key": model,
                            "model": model,
                            "provider": provider_key,
                        }));
                        catalog.len() - 1
                    }
                };
                let key = catalog[position]
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or(model)
                    .to_string();
                settings.insert("defaultModelKey".to_string(), json!(key));
            }
        }
        "geminicli" => {
            let env = ensure_object(settings, "env");
            if let Some(api_key) = api_key {
                env.insert("GEMINI_API_KEY".to_string(), json!(api_key));
            }
            if let Some(base_url) = base_url {
                env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(base_url));
            }
            if let Some(model) = model {
                env.insert("GEMINI_MODEL".to_string(), json!(model));
            }
        }
        "zcode" => {
            // ZCode stores a typed `ZcodeSettingsConfig`, not a flat
            // `{baseUrl, apiKey}` pair, so canonical fields are projected into
            // the provider id/name, `config.api`, `config.access` and models.
            if settings
                .get("providerId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                let requested = canonical_string(body, "providerId")
                    .filter(|value| value.starts_with("custom:"))
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        format!(
                            "custom:{}",
                            crate::coding::zcode::commands::zcode_provider_id_slug(&provider_name(
                                body
                            ))
                        )
                    });
                settings.insert("providerId".to_string(), json!(requested));
            }
            // A canonical `name` is the display name shown by ZCode, so an
            // update must overwrite it; without a name the stored value wins.
            match canonical_string(body, "name") {
                Some(name) => {
                    settings.insert("providerName".to_string(), json!(name));
                }
                None => {
                    settings
                        .entry("providerName".to_string())
                        .or_insert_with(|| json!(provider_name(body)));
                }
            }

            if base_url.is_some() || api_key.is_some() {
                let config = ensure_object(settings, "config");
                if let Some(base_url) = base_url {
                    let api = ensure_object(config, "api");
                    api.entry("type".to_string())
                        .or_insert_with(|| json!("anthropic-messages"));
                    api.insert("baseUrl".to_string(), json!(base_url));
                }
                if let Some(api_key) = api_key {
                    let access = ensure_object(config, "access");
                    access.insert("type".to_string(), json!("api-key"));
                    access.insert("apiKey".to_string(), json!(api_key));
                }
            }
            if let Some(model) = model {
                let models = ensure_root_array(settings, "models");
                let position = models
                    .iter()
                    .position(|row| row.get("modelId").and_then(Value::as_str) == Some(model));
                let position = match position {
                    Some(position) => position,
                    None => {
                        models.push(json!({
                            "modelId": model,
                            "ruleKind": "smart",
                            "isDefault": true,
                        }));
                        models.len() - 1
                    }
                };
                for (index, row) in models.iter_mut().enumerate() {
                    if let Some(object) = row.as_object_mut() {
                        object.insert("isDefault".to_string(), json!(index == position));
                    }
                }
                settings.insert("defaultModelId".to_string(), json!(model));
            }
        }
        _ => {}
    }
}

fn canonical_string<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        // `***` is a masked read, never a credential. Writing it back would
        // destroy the real key in every downstream config writer.
        .filter(|value| !value.contains(super::redact::REDACTED))
}

fn ensure_object<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let value = map.entry(key.to_string()).or_insert_with(|| json!({}));
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("object just ensured")
}

fn ensure_root_array<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    let value = map.entry(key.to_string()).or_insert_with(|| json!([]));
    if !value.is_array() {
        *value = json!([]);
    }
    value.as_array_mut().expect("array just ensured")
}

/// Ensures `root.key` holds an array.
fn ensure_array<'a>(map: &'a mut Map<String, Value>, root: &str, key: &str) -> &'a mut Vec<Value> {
    ensure_root_array(ensure_object(map, root), key)
}

/// Kimi stores custom channels under a provider key; reuse the single existing
/// key when the caller already configured one, otherwise use `custom`.
fn kimi_provider_key(settings: &Map<String, Value>) -> String {
    let keys: Vec<&String> = settings
        .get("providerConfigs")
        .and_then(Value::as_object)
        .map(|configs| configs.keys().collect())
        .unwrap_or_default();
    if keys.len() == 1 {
        keys[0].clone()
    } else {
        "custom".to_string()
    }
}

fn provider_slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        format!("provider-{}", uuid::Uuid::new_v4().simple())
    } else {
        slug
    }
}

fn requested_provider_key(body: &Value) -> Option<String> {
    body.get("providerId")
        .or_else(|| body.get("providerKey"))
        .or_else(|| body.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn provider_name(body: &Value) -> String {
    body.get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Custom provider")
        .to_string()
}

fn provider_category(body: &Value) -> String {
    body.get("category")
        .and_then(Value::as_str)
        .unwrap_or("custom")
        .to_string()
}

fn db_provider_input(cli: &str, body: &Value) -> Value {
    let base = body
        .get("provider")
        .cloned()
        .unwrap_or_else(|| body.clone());
    let mut input = base.as_object().cloned().unwrap_or_default();
    input
        .entry("name".to_string())
        .or_insert_with(|| json!(provider_name(body)));
    input
        .entry("category".to_string())
        .or_insert_with(|| json!(provider_category(body)));
    let mut settings = match input.get("settingsConfig") {
        Some(Value::String(raw)) => serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        Some(Value::Object(object)) => object.clone(),
        _ => Map::new(),
    };
    // `base` is the provider object when the caller nests it under
    // `"provider"`, otherwise the body itself, so canonical shortcuts work in
    // both shapes.
    apply_canonical_fields(cli, &base, &mut settings);
    input.insert(
        "settingsConfig".to_string(),
        Value::String(Value::Object(settings).to_string()),
    );
    Value::Object(input)
}

// ---------------------------------------------------------------------------
// Provider CRUD
// ---------------------------------------------------------------------------

pub async fn list_providers(context: &ServerContext, cli: &str) -> ApiResult<Value> {
    match cli {
        "antigravity" => Ok(json!({ "providers": [], "providerCrud": false })),
        "claudecode" => {
            let providers =
                crate::coding::claude_code::commands::list_claude_providers(db_state(context))
                    .await
                    .map_err(internal)?;
            Ok(providers_value(
                providers
                    .into_iter()
                    .map(|item| serde_json::to_value(item))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "claudedesktop" => {
            let providers = crate::coding::claude_desktop::commands::list_claude_desktop_providers(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            Ok(providers_value(
                providers
                    .into_iter()
                    .map(|item| serde_json::to_value(item))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "codex" => {
            let providers = crate::coding::codex::commands::list_codex_providers(db_state(context))
                .await
                .map_err(internal)?;
            Ok(providers_value(
                providers
                    .into_iter()
                    .map(|item| serde_json::to_value(item))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "grok" => {
            let providers = crate::coding::grok::commands::list_grok_providers(db_state(context))
                .await
                .map_err(internal)?;
            Ok(providers_value(
                providers
                    .into_iter()
                    .map(|item| serde_json::to_value(item))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "kimi" => {
            let providers = crate::coding::kimi::commands::list_kimi_providers(db_state(context))
                .await
                .map_err(internal)?;
            Ok(providers_value(
                providers
                    .into_iter()
                    .map(|item| serde_json::to_value(item))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "geminicli" => {
            let providers =
                crate::coding::gemini_cli::commands::list_gemini_cli_providers(db_state(context))
                    .await
                    .map_err(internal)?;
            Ok(providers_value(
                providers
                    .into_iter()
                    .map(|item| serde_json::to_value(item))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "zcode" => {
            let providers = crate::coding::zcode::commands::list_zcode_providers(db_state(context))
                .await
                .map_err(internal)?;
            Ok(providers_value(
                providers
                    .into_iter()
                    .map(|item| serde_json::to_value(item))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "opencode" => list_file_providers_opencode(context).await,
        "openclaw" => list_file_providers_openclaw(context).await,
        "pi" => list_runtime_providers(context, RuntimeCli::Pi).await,
        "oh_my_pi" => list_runtime_providers(context, RuntimeCli::Omp).await,
        "hermes" => list_runtime_providers(context, RuntimeCli::Hermes).await,
        "dsh" => list_runtime_providers(context, RuntimeCli::Dsh).await,
        "omo_native" => list_omo_providers(context).await,
        "workbuddy" | "codebuddy" => list_family_providers(context, cli).await,
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

fn providers_value(providers: Vec<Value>) -> Value {
    let items = providers
        .into_iter()
        .map(public_db_provider)
        .collect::<Vec<_>>();
    json!({ "providers": items })
}

pub async fn get_provider(context: &ServerContext, cli: &str, id: &str) -> ApiResult<Value> {
    let listing = list_providers(context, cli).await?;
    let found = listing
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("id").and_then(Value::as_str) == Some(id)
                    || item.get("providerKey").and_then(Value::as_str) == Some(id)
                    || item.get("providerId").and_then(Value::as_str) == Some(id)
            })
        })
        .cloned();
    found.ok_or_else(|| ApiError::not_found(format!("Provider '{id}' was not found in '{cli}'")))
}

pub async fn create_provider(context: &ServerContext, cli: &str, body: Value) -> ApiResult<Value> {
    ensure_no_redacted_input(&body, "create a provider")?;
    // Provider creation is a source write like update: while the gateway has
    // the CLI taken over it must happen inside restore -> write -> re-engage.
    with_gateway_reattach_api(context, cli, || async {
        create_provider_inner(context, cli, body).await
    })
    .await
}

async fn create_provider_inner(
    context: &ServerContext,
    cli: &str,
    body: Value,
) -> ApiResult<Value> {
    match cli {
        "antigravity" => Err(unsupported("antigravity", "Custom providers")),
        "claudecode" => {
            let input = serde_json::from_value(db_provider_input(cli, &body))
                .map_err(|error| invalid(format!("Invalid Claude provider: {error}")))?;
            let provider = crate::coding::claude_code::commands::create_claude_provider(
                db_state(context),
                app(context),
                input,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "claudedesktop" => {
            let input = serde_json::from_value(db_provider_input(cli, &body))
                .map_err(|error| invalid(format!("Invalid Claude Desktop provider: {error}")))?;
            let provider = crate::coding::claude_desktop::commands::create_claude_desktop_provider(
                db_state(context),
                app(context),
                input,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "codex" => {
            let input = serde_json::from_value(db_provider_input(cli, &body))
                .map_err(|error| invalid(format!("Invalid Codex provider: {error}")))?;
            let provider = crate::coding::codex::commands::create_codex_provider_inner(
                &db_state(context),
                &app(context),
                input,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "grok" => {
            let input = serde_json::from_value(db_provider_input(cli, &body))
                .map_err(|error| invalid(format!("Invalid Grok provider: {error}")))?;
            let provider = crate::coding::grok::commands::create_grok_provider_inner(
                &db_state(context),
                &app(context),
                input,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "kimi" => {
            let input = serde_json::from_value(db_provider_input(cli, &body))
                .map_err(|error| invalid(format!("Invalid Kimi provider: {error}")))?;
            let provider = crate::coding::kimi::commands::create_kimi_provider(
                db_state(context),
                app(context),
                input,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "geminicli" => {
            let input = serde_json::from_value(db_provider_input(cli, &body))
                .map_err(|error| invalid(format!("Invalid Gemini provider: {error}")))?;
            let provider = crate::coding::gemini_cli::commands::create_gemini_cli_provider_inner(
                &db_state(context),
                &app(context),
                input,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "zcode" => {
            let input = serde_json::from_value(db_provider_input(cli, &body))
                .map_err(|error| invalid(format!("Invalid ZCode provider: {error}")))?;
            let provider = crate::coding::zcode::commands::create_zcode_provider(
                db_state(context),
                app(context),
                input,
            )
            .await
            .map_err(internal)?;
            let value = serde_json::to_value(provider)?;
            project_zcode_provider(context, value.clone()).await?;
            Ok(public_db_provider(value))
        }
        "opencode" => create_opencode_provider(context, body).await,
        "openclaw" => create_openclaw_provider(context, body).await,
        "pi" => create_runtime_provider(context, RuntimeCli::Pi, body).await,
        "oh_my_pi" => create_runtime_provider(context, RuntimeCli::Omp, body).await,
        "hermes" => create_runtime_provider(context, RuntimeCli::Hermes, body).await,
        "dsh" => create_runtime_provider(context, RuntimeCli::Dsh, body).await,
        "omo_native" => create_omo_provider(context, body).await,
        "workbuddy" | "codebuddy" => create_family_provider(context, cli, body).await,
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

/// ZCode keeps a DB row plus a projected provider registry. The desktop form
/// always calls `save_zcode_provider` after create/update; the control API must
/// do the same or the CLI never sees the new provider.
async fn project_zcode_provider(context: &ServerContext, provider: Value) -> ApiResult<()> {
    let provider: crate::coding::zcode::types::ZcodeProvider =
        serde_json::from_value(provider)
            .map_err(|error| invalid(format!("Invalid ZCode provider: {error}")))?;
    crate::coding::zcode::commands::save_zcode_provider(
        db_state(context),
        app(context),
        crate::coding::zcode::types::ZcodeProviderInput {
            id: Some(provider.id),
            name: provider.name,
            category: provider.category,
            settings_config: provider.settings_config,
            source_provider_id: provider.source_provider_id,
            website_url: provider.website_url,
            notes: provider.notes,
            icon: provider.icon,
            icon_color: provider.icon_color,
            is_disabled: Some(provider.is_disabled),
            meta: provider.meta,
        },
    )
    .await
    .map_err(internal)?;
    Ok(())
}

/// Runs a provider/config write through the shared restore -> write -> re-engage
/// sequence while preserving the original `ApiError` status/code.
pub(crate) async fn with_gateway_reattach_api<T, F, Fut>(
    context: &ServerContext,
    cli: &str,
    write: F,
) -> ApiResult<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ApiResult<T>>,
{
    let Some(gateway_cli) = gateway_cli_key(cli) else {
        return write().await;
    };
    let snapshot =
        crate::coding::proxy_gateway::provider_switch::begin_provider_write(
            &context.app,
            gateway_cli,
        )
        .await
        .map_err(internal)?;
    let Some(snapshot) = snapshot else {
        return write().await;
    };

    let write_result = write().await;
    let reengage_result =
        crate::coding::proxy_gateway::provider_switch::finish_provider_write(
            &context.app,
            gateway_cli,
            snapshot,
        )
        .await;

    match (write_result, reengage_result) {
        (Ok(value), Ok(_)) => Ok(value),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(ApiError::internal(format!(
            "The write succeeded but the gateway takeover could not be re-engaged: {error}"
        ))),
        (Err(mut write_error), Err(reengage_error)) => {
            write_error.message = format!(
                "{}; additionally the gateway takeover could not be re-engaged: {reengage_error}",
                write_error.message
            );
            Err(write_error)
        }
    }
}

pub async fn update_provider(
    context: &ServerContext,
    cli: &str,
    id: &str,
    body: Value,
) -> ApiResult<Value> {
    reject_redacted_canonical_key(&body)?;
    with_gateway_reattach_api(context, cli, || async {
        update_provider_inner(context, cli, id, body).await
    })
    .await
}

/// An update has a stored value to fall back to, so a top-level `apiKey` of
/// `***` can only be a masked read echoed back. Silently ignoring it made the
/// documented "masked writes are rejected" contract untrue; nested
/// `settingsConfig` echoes are still allowed because the merge layer preserves
/// the stored secret for those.
fn reject_redacted_canonical_key(body: &Value) -> ApiResult<()> {
    if body
        .get("apiKey")
        .is_some_and(super::redact::is_redacted_value)
    {
        return Err(ApiError::bad_request(
            "Masked secrets ('***') cannot be used to update a provider; send the full value or omit the key",
        ));
    }
    Ok(())
}

async fn update_provider_inner(
    context: &ServerContext,
    cli: &str,
    id: &str,
    body: Value,
) -> ApiResult<Value> {
    match cli {
        "antigravity" => Err(unsupported("antigravity", "Custom providers")),
        "claudecode" | "claudedesktop" | "codex" | "grok" | "kimi" | "geminicli" | "zcode" => {
            update_db_provider(context, cli, id, body).await
        }
        "opencode" => update_opencode_provider(context, id, body).await,
        "openclaw" => update_openclaw_provider(context, id, body).await,
        "pi" => update_runtime_provider(context, RuntimeCli::Pi, id, body).await,
        "oh_my_pi" => update_runtime_provider(context, RuntimeCli::Omp, id, body).await,
        "hermes" => update_runtime_provider(context, RuntimeCli::Hermes, id, body).await,
        "dsh" => update_runtime_provider(context, RuntimeCli::Dsh, id, body).await,
        "omo_native" => update_omo_provider(context, id, body).await,
        "workbuddy" | "codebuddy" => update_family_provider(context, cli, id, body).await,
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

async fn update_db_provider(
    context: &ServerContext,
    cli: &str,
    id: &str,
    body: Value,
) -> ApiResult<Value> {
    let patch = body
        .get("provider")
        .cloned()
        .unwrap_or_else(|| body.clone());

    // Rebuild the raw provider from the same command output, not from the
    // redacted public DTO, so secrets that the caller did not send survive.
    let raw = raw_db_provider(context, cli, id).await?;
    let mut merged = raw;
    merge_settings_config(&mut merged, &patch);
    let merged = apply_canonical_to_provider_value(cli, merged, &patch);

    match cli {
        "claudecode" => {
            let provider = serde_json::from_value(merged)
                .map_err(|error| invalid(format!("Invalid Claude provider: {error}")))?;
            let provider = crate::coding::claude_code::commands::update_claude_provider(
                db_state(context),
                app(context),
                provider,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "claudedesktop" => {
            let provider = serde_json::from_value(merged)
                .map_err(|error| invalid(format!("Invalid Claude Desktop provider: {error}")))?;
            let provider = crate::coding::claude_desktop::commands::update_claude_desktop_provider(
                db_state(context),
                app(context),
                provider,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "codex" => {
            let provider = serde_json::from_value(merged)
                .map_err(|error| invalid(format!("Invalid Codex provider: {error}")))?;
            let provider = crate::coding::codex::commands::update_codex_provider(
                db_state(context),
                app(context),
                provider,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "grok" => {
            let provider = serde_json::from_value(merged)
                .map_err(|error| invalid(format!("Invalid Grok provider: {error}")))?;
            let provider = crate::coding::grok::commands::update_grok_provider(
                db_state(context),
                app(context),
                provider,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "kimi" => {
            let provider = serde_json::from_value(merged)
                .map_err(|error| invalid(format!("Invalid Kimi provider: {error}")))?;
            let provider = crate::coding::kimi::commands::update_kimi_provider(
                db_state(context),
                app(context),
                provider,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "geminicli" => {
            let provider = serde_json::from_value(merged)
                .map_err(|error| invalid(format!("Invalid Gemini provider: {error}")))?;
            let provider = crate::coding::gemini_cli::commands::update_gemini_cli_provider(
                db_state(context),
                app(context),
                provider,
            )
            .await
            .map_err(internal)?;
            Ok(public_db_provider(serde_json::to_value(provider)?))
        }
        "zcode" => {
            // The ZCode command persists first and projects afterwards. An
            // invalid model row therefore used to return 500 after the bad row
            // was already stored. Validate the merged settings before writing.
            validate_zcode_settings(&merged)?;
            let provider = serde_json::from_value(merged)
                .map_err(|error| invalid(format!("Invalid ZCode provider: {error}")))?;
            let provider = crate::coding::zcode::commands::update_zcode_provider(
                db_state(context),
                app(context),
                provider,
            )
            .await
            .map_err(internal)?;
            let value = serde_json::to_value(provider)?;
            project_zcode_provider(context, value.clone()).await?;
            Ok(public_db_provider(value))
        }
        _ => Err(ApiError::not_found("Unknown database CLI")),
    }
}

fn merge_settings_config(raw: &mut Value, patch: &Value) {
    // `settingsConfig` reaches this layer in both shapes: the public API uses an
    // object, while internal callers (add_model / delete_model /
    // set_default_model) pass the raw DB row where it is a JSON string. Both
    // must merge into the stored value, otherwise those commands return 200
    // while the database silently keeps the old model list.
    let patch_settings = match patch.get("settingsConfig") {
        Some(Value::Object(settings)) => Some(Value::Object(settings.clone())),
        Some(Value::String(text)) => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object),
        _ => None,
    };
    if let Some(settings) = patch_settings {
        let mut stored = Value::Object(settings_config_object(raw));
        super::redact::merge_patch_preserving_secrets(&mut stored, settings);
        if let Some(object) = stored.as_object() {
            set_settings_config(raw, object.clone());
        }
    }
    if let Some(object) = patch.as_object() {
        for (key, value) in object {
            if matches!(
                key.as_str(),
                "settingsConfig"
                    | "apiKey"
                    | "baseUrl"
                    | "model"
                    | "provider"
                    | "id"
                    | "isApplied"
                    | "isDefault"
                    | "createdAt"
                    | "updatedAt"
            ) {
                continue;
            }
            raw[key] = value.clone();
        }
    }
}

fn apply_canonical_to_provider_value(cli: &str, mut raw: Value, body: &Value) -> Value {
    let mut settings = settings_config_object(&raw);
    apply_canonical_fields(cli, body, &mut settings);
    set_settings_config(&mut raw, settings);
    raw
}

async fn raw_db_provider(context: &ServerContext, cli: &str, id: &str) -> ApiResult<Value> {
    match cli {
        "claudecode" => {
            let items =
                crate::coding::claude_code::commands::list_claude_providers(db_state(context))
                    .await
                    .map_err(internal)?;
            items
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| serde_json::to_value(item).map_err(ApiError::from))
                .transpose()?
                .ok_or_else(|| ApiError::not_found(format!("Provider '{id}' not found")))
        }
        "claudedesktop" => {
            let items = crate::coding::claude_desktop::commands::list_claude_desktop_providers(
                db_state(context),
            )
            .await
            .map_err(internal)?;
            items
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| serde_json::to_value(item).map_err(ApiError::from))
                .transpose()?
                .ok_or_else(|| ApiError::not_found(format!("Provider '{id}' not found")))
        }
        "codex" => {
            let items = crate::coding::codex::commands::list_codex_providers(db_state(context))
                .await
                .map_err(internal)?;
            items
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| serde_json::to_value(item).map_err(ApiError::from))
                .transpose()?
                .ok_or_else(|| ApiError::not_found(format!("Provider '{id}' not found")))
        }
        "grok" => {
            let items = crate::coding::grok::commands::list_grok_providers(db_state(context))
                .await
                .map_err(internal)?;
            items
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| serde_json::to_value(item).map_err(ApiError::from))
                .transpose()?
                .ok_or_else(|| ApiError::not_found(format!("Provider '{id}' not found")))
        }
        "kimi" => {
            let items = crate::coding::kimi::commands::list_kimi_providers(db_state(context))
                .await
                .map_err(internal)?;
            items
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| serde_json::to_value(item).map_err(ApiError::from))
                .transpose()?
                .ok_or_else(|| ApiError::not_found(format!("Provider '{id}' not found")))
        }
        "geminicli" => {
            let items =
                crate::coding::gemini_cli::commands::list_gemini_cli_providers(db_state(context))
                    .await
                    .map_err(internal)?;
            items
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| serde_json::to_value(item).map_err(ApiError::from))
                .transpose()?
                .ok_or_else(|| ApiError::not_found(format!("Provider '{id}' not found")))
        }
        "zcode" => {
            let items = crate::coding::zcode::commands::list_zcode_providers(db_state(context))
                .await
                .map_err(internal)?;
            items
                .into_iter()
                .find(|item| item.id == id)
                .map(|item| serde_json::to_value(item).map_err(ApiError::from))
                .transpose()?
                .ok_or_else(|| ApiError::not_found(format!("Provider '{id}' not found")))
        }
        _ => Err(ApiError::not_found("Unknown database CLI")),
    }
}

pub async fn delete_provider(context: &ServerContext, cli: &str, id: &str) -> ApiResult<Value> {
    match cli {
        "antigravity" => Err(unsupported("antigravity", "Custom providers")),
        "claudecode" => {
            crate::coding::claude_code::commands::delete_claude_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(command_error)?;
            Ok(json!({"deleted": id}))
        }
        "claudedesktop" => {
            crate::coding::claude_desktop::commands::delete_claude_desktop_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(command_error)?;
            Ok(json!({"deleted": id}))
        }
        "codex" => {
            crate::coding::codex::commands::delete_codex_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(command_error)?;
            Ok(json!({"deleted": id}))
        }
        "grok" => {
            crate::coding::grok::commands::delete_grok_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(command_error)?;
            Ok(json!({"deleted": id}))
        }
        "kimi" => {
            crate::coding::kimi::commands::delete_kimi_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(command_error)?;
            Ok(json!({"deleted": id}))
        }
        "geminicli" => {
            crate::coding::gemini_cli::commands::delete_gemini_cli_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(command_error)?;
            Ok(json!({"deleted": id}))
        }
        "zcode" => {
            crate::coding::zcode::commands::delete_zcode_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(command_error)?;
            Ok(json!({"deleted": id}))
        }
        "opencode" => delete_opencode_provider(context, id).await,
        "openclaw" => delete_openclaw_provider(context, id).await,
        "pi" => delete_runtime_provider(context, RuntimeCli::Pi, id).await,
        "oh_my_pi" => delete_runtime_provider(context, RuntimeCli::Omp, id).await,
        "hermes" => delete_runtime_provider(context, RuntimeCli::Hermes, id).await,
        "dsh" => delete_runtime_provider(context, RuntimeCli::Dsh, id).await,
        "omo_native" => delete_omo_provider(context, id).await,
        "workbuddy" | "codebuddy" => delete_family_provider(context, cli, id).await,
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

pub async fn apply_provider(
    context: &ServerContext,
    cli: &str,
    id: &str,
    model_id: Option<&str>,
) -> ApiResult<Value> {
    // The six gateway-managed DB CLIs must share the desktop orchestration:
    // restore direct -> apply -> re-engage single/failover/aggregate. Calling
    // the bare `apply_*` commands here would rewrite gateway-managed runtime
    // files and desynchronize the manifest from reality.
    if let Some(gateway_cli) = gateway_cli_key(cli) {
        let status = crate::coding::proxy_gateway::provider_switch::apply_or_switch_provider(
            &context.app,
            gateway_cli,
            id,
            false,
        )
        .await
        .map_err(internal)?;
        return Ok(json!({"applied": id, "gatewayTakeover": status}));
    }

    match cli {
        "zcode" => {
            let model = model_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::bad_request(
                        "ZCode requires a model id to apply a provider; pass 'modelId'",
                    )
                })?;
            crate::coding::zcode::commands::select_zcode_provider(
                db_state(context),
                app(context),
                id.to_string(),
                model.to_string(),
            )
            .await
            .map_err(internal)?;
            Ok(json!({"applied": id, "modelId": model}))
        }
        "opencode" => apply_opencode_provider(context, id, model_id).await,
        "openclaw" => apply_openclaw_provider(context, id, model_id).await,
        "pi" => apply_runtime_provider(context, RuntimeCli::Pi, id, model_id).await,
        "oh_my_pi" => apply_runtime_provider(context, RuntimeCli::Omp, id, model_id).await,
        "hermes" => apply_runtime_provider(context, RuntimeCli::Hermes, id, model_id).await,
        "dsh" => apply_runtime_provider(context, RuntimeCli::Dsh, id, model_id).await,
        "omo_native" => apply_omo_provider(context, id, model_id).await,
        "workbuddy" | "codebuddy" => Err(unsupported(cli, "Provider apply")),
        "antigravity" => Err(unsupported("antigravity", "Custom providers")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

// ---------------------------------------------------------------------------
// File-backed provider helpers
// ---------------------------------------------------------------------------

async fn list_file_providers_opencode(context: &ServerContext) -> ApiResult<Value> {
    let config = read_opencode_config_value(context).await?;
    let providers = config
        .get("provider")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let items = providers
        .into_iter()
        .map(|(key, provider)| {
            let has_key = provider
                .get("options")
                .and_then(|options| options.get("apiKey"))
                .is_some_and(|value| !value.is_null());
            public_file_provider(&key, provider, has_key)
        })
        .collect::<Vec<_>>();
    Ok(json!({ "providers": items }))
}

pub(crate) async fn read_opencode_config_value(context: &ServerContext) -> ApiResult<Value> {
    let result = crate::coding::open_code::commands::read_opencode_config(db_state(context))
        .await
        .map_err(internal)?;
    let value = serde_json::to_value(result)?;
    match value.get("status").and_then(Value::as_str) {
        Some("success") => Ok(value.get("config").cloned().unwrap_or_else(|| json!({}))),
        Some("notFound") => Ok(json!({"provider": {}})),
        _ => Err(ApiError::bad_request(
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("OpenCode config could not be parsed"),
        )),
    }
}

pub(crate) async fn save_opencode_config_value(context: &ServerContext, config: Value) -> ApiResult<()> {
    let config: crate::coding::open_code::types::OpenCodeConfig = serde_json::from_value(config)
        .map_err(|error| invalid(format!("Invalid OpenCode config: {error}")))?;
    crate::coding::open_code::commands::save_opencode_config(
        db_state(context),
        app(context),
        config,
    )
    .await
    .map_err(internal)
}

async fn create_opencode_provider(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let provider = body.get("provider").cloned().unwrap_or_else(|| {
        let mut provider = json!({
            "name": provider_name(&body),
            "npm": "@ai-sdk/openai-compatible",
            "options": {},
            "models": {},
        });
        if let Some(base_url) = body.get("baseUrl").and_then(Value::as_str) {
            provider["options"]["baseURL"] = json!(base_url);
        }
        if let Some(api_key) = body.get("apiKey").and_then(Value::as_str) {
            provider["options"]["apiKey"] = json!(api_key);
        }
        provider
    });
    let key = requested_provider_key(&body).unwrap_or_else(|| provider_slug(&provider_name(&body)));
    let mut config = read_opencode_config_value(context).await?;
    if !config.get("provider").is_some_and(Value::is_object) {
        config["provider"] = json!({});
    }
    config["provider"][&key] = provider;
    save_opencode_config_value(context, config).await?;
    get_provider(context, "opencode", &key).await
}

async fn update_opencode_provider(
    context: &ServerContext,
    id: &str,
    body: Value,
) -> ApiResult<Value> {
    let mut config = read_opencode_config_value(context).await?;
    let existing = config
        .get_mut("provider")
        .and_then(Value::as_object_mut)
        .and_then(|providers| providers.get_mut(id))
        .ok_or_else(|| ApiError::not_found(format!("OpenCode provider '{id}' not found")))?;
    let patch = body
        .get("provider")
        .cloned()
        .unwrap_or_else(|| body.clone());
    super::redact::merge_patch_preserving_secrets(existing, patch);
    save_opencode_config_value(context, config).await?;
    get_provider(context, "opencode", id).await
}

async fn delete_opencode_provider(context: &ServerContext, id: &str) -> ApiResult<Value> {
    let mut config = read_opencode_config_value(context).await?;
    let removed = config
        .get_mut("provider")
        .and_then(Value::as_object_mut)
        .and_then(|providers| providers.remove(id))
        .is_some();
    if !removed {
        return Err(ApiError::not_found(format!(
            "OpenCode provider '{id}' not found"
        )));
    }
    let selection_matches = match config.get("model") {
        Some(Value::Object(selection)) => {
            selection.get("providerID").and_then(Value::as_str) == Some(id)
        }
        Some(Value::String(selection)) => selection.starts_with(&format!("{id}/")),
        _ => false,
    };
    if selection_matches {
        if let Some(object) = config.as_object_mut() {
            object.remove("model");
        }
    }
    save_opencode_config_value(context, config).await?;
    Ok(json!({"deleted": id}))
}

async fn apply_opencode_provider(
    context: &ServerContext,
    id: &str,
    model_id: Option<&str>,
) -> ApiResult<Value> {
    let mut config = read_opencode_config_value(context).await?;
    let models = config
        .get("provider")
        .and_then(|providers| providers.get(id))
        .and_then(|provider| provider.get("models"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let model_id = model_id
        .map(str::to_string)
        .or_else(|| models.keys().next().cloned())
        .ok_or_else(|| ApiError::bad_request("This provider has no models to apply"))?;
    config["model"] = json!(format!("{id}/{model_id}"));
    save_opencode_config_value(context, config).await?;
    Ok(json!({"applied": id, "modelId": model_id}))
}

async fn list_file_providers_openclaw(context: &ServerContext) -> ApiResult<Value> {
    let config = read_openclaw_config_value(context).await?;
    let providers = config
        .get("models")
        .and_then(|models| models.get("providers"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let items = providers
        .into_iter()
        .map(|(key, provider)| {
            let has_key = provider.get("apiKey").is_some_and(|value| !value.is_null());
            public_file_provider(&key, provider, has_key)
        })
        .collect::<Vec<_>>();
    Ok(json!({ "providers": items }))
}

pub(crate) async fn read_openclaw_config_value(context: &ServerContext) -> ApiResult<Value> {
    let result = crate::coding::open_claw::commands::read_openclaw_config(db_state(context))
        .await
        .map_err(internal)?;
    let value = serde_json::to_value(result)?;
    match value.get("status").and_then(Value::as_str) {
        Some("success") => Ok(value.get("config").cloned().unwrap_or_else(|| json!({}))),
        Some("notFound") => Ok(json!({"models": {"providers": {}}})),
        _ => Err(ApiError::bad_request(
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("OpenClaw config could not be parsed"),
        )),
    }
}

pub(crate) async fn save_openclaw_config_value(context: &ServerContext, config: Value) -> ApiResult<()> {
    let config: crate::coding::open_claw::types::OpenClawConfig = serde_json::from_value(config)
        .map_err(|error| invalid(format!("Invalid OpenClaw config: {error}")))?;
    crate::coding::open_claw::commands::save_openclaw_config(
        db_state(context),
        app(context),
        config,
    )
    .await
    .map_err(internal)
}

async fn create_openclaw_provider(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let provider = body.get("provider").cloned().unwrap_or_else(|| {
        let mut provider = json!({"models": []});
        if let Some(base_url) = body.get("baseUrl").and_then(Value::as_str) {
            provider["baseUrl"] = json!(base_url);
        }
        if let Some(api_key) = body.get("apiKey").and_then(Value::as_str) {
            provider["apiKey"] = json!(api_key);
        }
        provider
    });
    let key = requested_provider_key(&body).unwrap_or_else(|| provider_slug(&provider_name(&body)));
    let mut config = read_openclaw_config_value(context).await?;
    if !config.get("models").is_some_and(Value::is_object) {
        config["models"] = json!({});
    }
    if !config["models"]
        .get("providers")
        .is_some_and(Value::is_object)
    {
        config["models"]["providers"] = json!({});
    }
    config["models"]["providers"][&key] = provider;
    save_openclaw_config_value(context, config).await?;
    get_provider(context, "openclaw", &key).await
}

async fn update_openclaw_provider(
    context: &ServerContext,
    id: &str,
    body: Value,
) -> ApiResult<Value> {
    let mut config = read_openclaw_config_value(context).await?;
    let existing = config
        .get_mut("models")
        .and_then(|models| models.get_mut("providers"))
        .and_then(Value::as_object_mut)
        .and_then(|providers| providers.get_mut(id))
        .ok_or_else(|| ApiError::not_found(format!("OpenClaw provider '{id}' not found")))?;
    let patch = body
        .get("provider")
        .cloned()
        .unwrap_or_else(|| body.clone());
    super::redact::merge_patch_preserving_secrets(existing, patch);
    save_openclaw_config_value(context, config).await?;
    get_provider(context, "openclaw", id).await
}

async fn delete_openclaw_provider(context: &ServerContext, id: &str) -> ApiResult<Value> {
    let mut config = read_openclaw_config_value(context).await?;
    let removed = config
        .get_mut("models")
        .and_then(|models| models.get_mut("providers"))
        .and_then(Value::as_object_mut)
        .and_then(|providers| providers.remove(id))
        .is_some();
    if !removed {
        return Err(ApiError::not_found(format!(
            "OpenClaw provider '{id}' not found"
        )));
    }
    clear_openclaw_model_selection(&mut config, id, None);
    save_openclaw_config_value(context, config).await?;
    Ok(json!({"deleted": id}))
}

async fn apply_openclaw_provider(
    context: &ServerContext,
    id: &str,
    model_id: Option<&str>,
) -> ApiResult<Value> {
    let mut config = read_openclaw_config_value(context).await?;
    let models = config
        .get("models")
        .and_then(|models| models.get("providers"))
        .and_then(|providers| providers.get(id))
        .and_then(|provider| provider.get("models"))
        .cloned();
    let model_id = model_id
        .map(str::to_string)
        .or_else(|| {
            models.as_ref().and_then(|models| {
                models.as_array().and_then(|items| {
                    items.iter().find_map(|item| {
                        item.get("id")
                            .or_else(|| item.get("model"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
            })
        })
        .ok_or_else(|| ApiError::bad_request("This provider has no models to apply"))?;
    if !config.get("agents").is_some_and(Value::is_object) {
        config["agents"] = json!({});
    }
    if !config["agents"]
        .get("defaults")
        .is_some_and(Value::is_object)
    {
        config["agents"]["defaults"] = json!({});
    }
    // Only replace the primary entry: OpenClaw's `model` object also carries a
    // user-configured `fallbacks` list that an apply must not drop.
    if !config["agents"]["defaults"]["model"].is_object() {
        config["agents"]["defaults"]["model"] = json!({});
    }
    config["agents"]["defaults"]["model"]["primary"] = json!(format!("{id}/{model_id}"));
    save_openclaw_config_value(context, config).await?;
    Ok(json!({"applied": id, "modelId": model_id}))
}

#[derive(Clone, Copy)]
enum RuntimeCli {
    Pi,
    Omp,
    Hermes,
    Dsh,
}

impl RuntimeCli {
    fn key(self) -> &'static str {
        match self {
            RuntimeCli::Pi => "pi",
            RuntimeCli::Omp => "oh_my_pi",
            RuntimeCli::Hermes => "hermes",
            RuntimeCli::Dsh => "dsh",
        }
    }

    fn label(self) -> &'static str {
        match self {
            RuntimeCli::Pi => "Pi",
            RuntimeCli::Omp => "Oh My Pi",
            RuntimeCli::Hermes => "Hermes",
            RuntimeCli::Dsh => "DSH",
        }
    }
}

async fn runtime_config(context: &ServerContext, cli: RuntimeCli) -> ApiResult<Value> {
    let state = db_state(context);
    let value = match cli {
        RuntimeCli::Pi => serde_json::to_value(
            crate::coding::pi::commands::read_pi_runtime_config(state)
                .await
                .map_err(internal)?,
        )?,
        RuntimeCli::Omp => serde_json::to_value(
            crate::coding::oh_my_pi::commands::read_omp_runtime_config(state)
                .await
                .map_err(internal)?,
        )?,
        RuntimeCli::Hermes => serde_json::to_value(
            crate::coding::hermes::commands::read_hermes_runtime_config(state)
                .await
                .map_err(internal)?,
        )?,
        RuntimeCli::Dsh => serde_json::to_value(
            crate::coding::dsh::commands::read_dsh_runtime_config(state)
                .await
                .map_err(internal)?,
        )?,
    };
    Ok(value)
}

async fn list_runtime_providers(context: &ServerContext, cli: RuntimeCli) -> ApiResult<Value> {
    let config = runtime_config(context, cli).await?;
    let providers = config
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let items = providers
        .into_iter()
        .map(|provider| {
            let key = provider
                .get("providerKey")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let raw = provider
                .get("modelsProvider")
                .or_else(|| provider.get("provider"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let has_key = raw.get("apiKey").is_some_and(|value| !value.is_null())
                || provider
                    .get("credential")
                    .is_some_and(|value| !value.is_null());
            let mut public = public_file_provider(&key, raw, has_key);
            public["sources"] = provider
                .get("sources")
                .cloned()
                .unwrap_or_else(|| json!([]));
            public["modelIds"] = provider
                .get("modelIds")
                .cloned()
                .unwrap_or_else(|| json!([]));
            public["isDefault"] = provider
                .get("isDefault")
                .cloned()
                .unwrap_or(Value::Bool(false));
            public["warnings"] = provider
                .get("warnings")
                .cloned()
                .unwrap_or_else(|| json!([]));
            public
        })
        .collect::<Vec<_>>();
    Ok(json!({ "providers": items }))
}

async fn create_runtime_provider(
    context: &ServerContext,
    cli: RuntimeCli,
    body: Value,
) -> ApiResult<Value> {
    let key = requested_provider_key(&body).unwrap_or_else(|| provider_slug(&provider_name(&body)));
    let provider = body.get("provider").cloned().unwrap_or_else(|| {
        let mut provider = json!({"models": []});
        if let Some(base_url) = body.get("baseUrl").and_then(Value::as_str) {
            provider["baseUrl"] = json!(base_url);
        }
        if let Some(api_key) = body.get("apiKey").and_then(Value::as_str) {
            provider["apiKey"] = json!(api_key);
        }
        provider
    });
    save_runtime_provider(
        context,
        cli,
        &key,
        provider,
        body.get("apiKey").and_then(Value::as_str),
    )
    .await?;
    get_provider(context, cli.key(), &key).await
}

async fn update_runtime_provider(
    context: &ServerContext,
    cli: RuntimeCli,
    id: &str,
    body: Value,
) -> ApiResult<Value> {
    let listing = list_runtime_providers(context, cli).await?;
    let existing = listing
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["providerKey"] == id))
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("{} provider '{id}' not found", cli.label())))?;
    let mut provider = existing
        .get("provider")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let patch = body
        .get("provider")
        .cloned()
        .unwrap_or_else(|| body.clone());
    // The public DTO is redacted; re-read the raw runtime config for secrets.
    let raw = runtime_config(context, cli).await?;
    let raw_provider = raw
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["providerKey"] == id))
        .and_then(|item| item.get("modelsProvider").or_else(|| item.get("provider")))
        .cloned();
    if let Some(raw_provider) = raw_provider {
        provider = raw_provider;
    }
    super::redact::merge_patch_preserving_secrets(&mut provider, patch);
    save_runtime_provider(
        context,
        cli,
        id,
        provider,
        body.get("apiKey").and_then(Value::as_str),
    )
    .await?;
    get_provider(context, cli.key(), id).await
}

async fn save_runtime_provider(
    context: &ServerContext,
    cli: RuntimeCli,
    key: &str,
    provider: Value,
    api_key: Option<&str>,
) -> ApiResult<()> {
    match cli {
        RuntimeCli::Pi => {
            crate::coding::pi::commands::save_pi_models_provider(
                db_state(context),
                app(context),
                crate::coding::pi::types::PiModelsProviderInput {
                    provider_key: key.to_string(),
                    provider,
                },
            )
            .await
            .map_err(internal)?;
            if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
                crate::coding::pi::commands::save_pi_auth_provider(
                    db_state(context),
                    app(context),
                    crate::coding::pi::types::PiAuthProviderInput {
                        provider_key: key.to_string(),
                        credential: json!({"type": "api_key", "key": api_key}),
                    },
                )
                .await
                .map_err(internal)?;
            }
        }
        RuntimeCli::Omp => {
            crate::coding::oh_my_pi::commands::save_omp_models_provider(
                db_state(context),
                app(context),
                crate::coding::oh_my_pi::types::OmpModelsProviderInput {
                    provider_key: key.to_string(),
                    provider,
                },
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Hermes => {
            crate::coding::hermes::commands::save_hermes_models_provider(
                db_state(context),
                app(context),
                crate::coding::hermes::types::HermesModelsProviderInput {
                    provider_key: key.to_string(),
                    provider,
                },
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Dsh => {
            let credential = api_key
                .filter(|value| !value.trim().is_empty())
                .map(|value| crate::coding::dsh::types::DshCredentialInput {
                    ref_name: format!(
                        "AITB_{}",
                        uuid::Uuid::new_v4().simple().to_string().to_uppercase()
                    ),
                    value: value.to_string(),
                });
            let mut provider = provider;
            if let Some(credential) = &credential {
                provider["apiKeyEnv"] = json!(credential.ref_name);
            }
            crate::coding::dsh::commands::save_dsh_models_provider(
                db_state(context),
                app(context),
                crate::coding::dsh::types::DshModelsProviderInput {
                    provider_key: key.to_string(),
                    provider,
                    credential,
                },
            )
            .await
            .map_err(internal)?;
        }
    }
    Ok(())
}

async fn delete_runtime_provider(
    context: &ServerContext,
    cli: RuntimeCli,
    id: &str,
) -> ApiResult<Value> {
    match cli {
        RuntimeCli::Pi => {
            crate::coding::pi::commands::delete_pi_runtime_provider(
                db_state(context),
                app(context),
                id.to_string(),
                crate::coding::pi::types::PiDeleteScope::Both,
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Omp => {
            crate::coding::oh_my_pi::commands::delete_omp_runtime_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Hermes => {
            crate::coding::hermes::commands::delete_hermes_runtime_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Dsh => {
            crate::coding::dsh::commands::delete_dsh_runtime_provider(
                db_state(context),
                app(context),
                id.to_string(),
            )
            .await
            .map_err(internal)?;
        }
    }
    Ok(json!({"deleted": id}))
}

async fn apply_runtime_provider(
    context: &ServerContext,
    cli: RuntimeCli,
    id: &str,
    model_id: Option<&str>,
) -> ApiResult<Value> {
    let listing = list_runtime_providers(context, cli).await?;
    let model_id = model_id.map(str::to_string).or_else(|| {
        listing
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|item| item["providerKey"] == id))
            .and_then(|item| item.get("modelIds"))
            .and_then(Value::as_array)
            .and_then(|ids| ids.first())
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let model_id =
        model_id.ok_or_else(|| ApiError::bad_request("This provider has no models to apply"))?;
    match cli {
        RuntimeCli::Pi => {
            crate::coding::pi::commands::save_pi_model_settings(
                db_state(context),
                app(context),
                crate::coding::pi::types::PiModelSettingsInput {
                    default_provider: Some(id.to_string()),
                    default_model: Some(model_id.clone()),
                    default_thinking_level: None,
                },
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Omp => {
            crate::coding::oh_my_pi::commands::save_omp_model_settings(
                db_state(context),
                app(context),
                crate::coding::oh_my_pi::types::OmpModelSettingsInput {
                    default_provider: Some(id.to_string()),
                    default_model: Some(model_id.clone()),
                    default_thinking_level: None,
                    clear_thinking_level: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Hermes => {
            crate::coding::hermes::commands::save_hermes_model_settings(
                db_state(context),
                app(context),
                crate::coding::hermes::types::HermesModelSettingsInput {
                    default_model: Some(model_id.clone()),
                    default_provider: Some(id.to_string()),
                    base_url: None,
                    context_length: None,
                    max_tokens: None,
                    clear_context_length: false,
                    clear_max_tokens: false,
                },
            )
            .await
            .map_err(internal)?;
        }
        RuntimeCli::Dsh => {
            crate::coding::dsh::commands::save_dsh_model_settings(
                db_state(context),
                app(context),
                crate::coding::dsh::types::DshModelSettingsInput {
                    provider: Some(id.to_string()),
                    model: Some(model_id.clone()),
                    reasoning_effort: None,
                },
            )
            .await
            .map_err(internal)?;
        }
    }
    Ok(json!({"applied": id, "modelId": model_id}))
}

async fn list_omo_providers(context: &ServerContext) -> ApiResult<Value> {
    let providers =
        crate::coding::omo_native::providers::list_omo_native_providers(db_state(context))
            .await
            .map_err(internal)?;
    let items = providers
        .into_iter()
        .map(|provider| {
            let mut value = public_file_provider(
                &provider.key,
                provider.config.clone(),
                provider.api_key.is_some(),
            );
            value["builtin"] = json!(provider.builtin);
            value["oauth"] = json!(provider.oauth);
            value["custom"] = json!(provider.custom);
            value
        })
        .collect::<Vec<_>>();
    Ok(json!({ "providers": items }))
}

async fn create_omo_provider(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let key = requested_provider_key(&body).unwrap_or_else(|| provider_slug(&provider_name(&body)));
    let config = body
        .get("provider")
        .cloned()
        .unwrap_or_else(|| json!({"name": provider_name(&body), "models": []}));
    crate::coding::omo_native::providers::save_omo_native_provider(
        db_state(context),
        app(context),
        crate::coding::omo_native::types::OmoNativeProviderInput {
            key: key.clone(),
            config,
            api_key: body
                .get("apiKey")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
    )
    .await
    .map_err(internal)?;
    get_provider(context, "omo_native", &key).await
}

async fn raw_omo_provider(context: &ServerContext, id: &str) -> ApiResult<Value> {
    let providers = crate::coding::omo_native::providers::list_omo_native_providers(
        db_state(context),
    )
    .await
    .map_err(internal)?;
    providers
        .into_iter()
        .find(|provider| provider.key == id)
        .map(|provider| provider.config)
        .ok_or_else(|| ApiError::not_found(format!("OMO provider '{id}' not found")))
}

async fn update_omo_provider(context: &ServerContext, id: &str, body: Value) -> ApiResult<Value> {
    // Start from the raw models.json entry, never the redacted public DTO:
    // merging from a masked tree writes `***` as the provider's apiKey.
    let mut config = raw_omo_provider(context, id).await?;
    let patch = body
        .get("provider")
        .cloned()
        .unwrap_or_else(|| body.clone());
    super::redact::merge_patch_preserving_secrets(&mut config, patch);
    let api_key = body
        .get("apiKey")
        .and_then(Value::as_str)
        .filter(|value| !value.contains(super::redact::REDACTED))
        .map(str::to_string);
    crate::coding::omo_native::providers::save_omo_native_provider(
        db_state(context),
        app(context),
        crate::coding::omo_native::types::OmoNativeProviderInput {
            key: id.to_string(),
            config,
            api_key,
        },
    )
    .await
    .map_err(internal)?;
    get_provider(context, "omo_native", id).await
}

async fn add_omo_model(
    context: &ServerContext,
    provider_id: &str,
    model_id: &str,
    body: &Value,
) -> ApiResult<Value> {
    let mut config = raw_omo_provider(context, provider_id).await?;
    let mut models = config
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !models
        .iter()
        .any(|model| model.get("id").and_then(Value::as_str) == Some(model_id))
    {
        let mut model = body.get("model").cloned().unwrap_or_else(|| json!({}));
        if !model.is_object() {
            model = json!({});
        }
        model["id"] = json!(model_id);
        if let Some(name) = body.get("name").and_then(Value::as_str) {
            model["name"] = json!(name);
        }
        models.push(model);
    }
    config["models"] = Value::Array(models);
    crate::coding::omo_native::providers::save_omo_native_provider(
        db_state(context),
        app(context),
        crate::coding::omo_native::types::OmoNativeProviderInput {
            key: provider_id.to_string(),
            config,
            api_key: None,
        },
    )
    .await
    .map_err(internal)?;
    list_models(context, "omo_native", provider_id).await
}

async fn delete_omo_model(
    context: &ServerContext,
    provider_id: &str,
    model_id: &str,
) -> ApiResult<()> {
    let mut config = raw_omo_provider(context, provider_id).await?;
    if let Some(models) = config.get_mut("models").and_then(Value::as_array_mut) {
        models.retain(|model| model.get("id").and_then(Value::as_str) != Some(model_id));
    }
    crate::coding::omo_native::providers::save_omo_native_provider(
        db_state(context),
        app(context),
        crate::coding::omo_native::types::OmoNativeProviderInput {
            key: provider_id.to_string(),
            config,
            api_key: None,
        },
    )
    .await
    .map_err(internal)
}

async fn delete_omo_provider(context: &ServerContext, id: &str) -> ApiResult<Value> {
    crate::coding::omo_native::providers::delete_omo_native_provider(
        db_state(context),
        app(context),
        id.to_string(),
        true,
    )
    .await
    .map_err(internal)?;
    Ok(json!({"deleted": id}))
}

async fn apply_omo_provider(
    context: &ServerContext,
    id: &str,
    model_id: Option<&str>,
) -> ApiResult<Value> {
    let listing = list_omo_providers(context).await?;
    let model_id = model_id.map(str::to_string).or_else(|| {
        listing
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|item| item["providerKey"] == id))
            .and_then(|item| item.get("provider"))
            .and_then(|provider| provider.get("models"))
            .and_then(Value::as_array)
            .and_then(|models| models.first())
            .and_then(|model| {
                model
                    .get("id")
                    .or_else(|| model.get("model"))
                    .and_then(Value::as_str)
            })
            .map(str::to_string)
    });
    let model_id =
        model_id.ok_or_else(|| ApiError::bad_request("This provider has no models to apply"))?;
    crate::coding::omo_native::commands::save_omo_native_model_settings(
        db_state(context),
        app(context),
        crate::coding::omo_native::types::OmoNativeModelSettingsInput {
            default_provider: Some(id.to_string()),
            default_model: Some(model_id.clone()),
            default_thinking_level: None,
        },
    )
    .await
    .map_err(internal)?;
    Ok(json!({"applied": id, "modelId": model_id}))
}

async fn list_family_providers(context: &ServerContext, cli: &str) -> ApiResult<Value> {
    let config = family_runtime_config(context, cli).await?;
    let providers = config
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let items = providers
        .into_iter()
        .map(|provider| {
            let mut value = provider.clone();
            let has_key = provider.get("apiKey").is_some_and(|value| !value.is_null());
            redact_value(&mut value);
            value["hasApiKey"] = Value::Bool(has_key);
            value
        })
        .collect::<Vec<_>>();
    Ok(json!({ "providers": items, "providerKind": "channel" }))
}

async fn family_runtime_config(context: &ServerContext, cli: &str) -> ApiResult<Value> {
    let state = db_state(context);
    let value = if cli == "workbuddy" {
        serde_json::to_value(
            crate::coding::workbuddy::commands::read_workbuddy_runtime_config(state)
                .await
                .map_err(internal)?,
        )?
    } else {
        serde_json::to_value(
            crate::coding::codebuddy::commands::read_codebuddy_runtime_config(state)
                .await
                .map_err(internal)?,
        )?
    };
    Ok(value)
}

async fn create_family_provider(
    context: &ServerContext,
    cli: &str,
    body: Value,
) -> ApiResult<Value> {
    let base_url = body
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("'baseUrl' is required for this CLI"))?;
    let api_key = body
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let models = body
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first_model = models
        .first()
        .cloned()
        .or_else(|| {
            body.get("modelId")
                .or_else(|| body.get("model"))
                .and_then(Value::as_str)
                .map(|id| json!({"id": id}))
        })
        .ok_or_else(|| {
            ApiError::bad_request(
                "This CLI derives channels from models.json; provide at least one model (id) when creating a channel",
            )
        })?;
    let model_id = first_model
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("Each model needs an 'id'"))?;
    if cli == "workbuddy" {
        crate::coding::workbuddy::commands::save_workbuddy_model(
            db_state(context),
            crate::coding::workbuddy::types::WorkbuddyModelInput {
                previous_id: None,
                id: model_id.to_string(),
                name: first_model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(model_id)
                    .to_string(),
                vendor: body
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                max_input_tokens: None,
                max_output_tokens: None,
                temperature: None,
                supports_tool_call: true,
                supports_images: false,
                supports_reasoning: false,
                only_reasoning: false,
                use_custom_protocol: false,
                can_disable_thinking: false,
                default_effort: None,
                supported_efforts: Vec::new(),
            },
            app(context),
        )
        .await
        .map_err(internal)?;
    } else {
        crate::coding::codebuddy::commands::save_codebuddy_model(
            db_state(context),
            crate::coding::codebuddy::types::CodebuddyModelInput {
                previous_id: None,
                id: model_id.to_string(),
                name: first_model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(model_id)
                    .to_string(),
                vendor: body
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                max_input_tokens: None,
                max_output_tokens: None,
                temperature: None,
                supports_tool_call: true,
                supports_images: false,
                supports_reasoning: false,
                only_reasoning: false,
                use_custom_protocol: false,
                can_disable_thinking: false,
                default_effort: None,
                supported_efforts: Vec::new(),
            },
            app(context),
        )
        .await
        .map_err(internal)?;
    }
    let channel_id = family_channel_id(base_url, api_key);
    get_provider(context, cli, &channel_id).await
}

async fn update_family_provider(
    _context: &ServerContext,
    cli: &str,
    _id: &str,
    _body: Value,
) -> ApiResult<Value> {
    // Channel identity is (baseUrl, apiKey); changing either creates a new
    // channel. Returning 200 here made `aitoolbox provider update` look successful
    // while nothing changed, so the contract is an explicit refusal.
    Err(ApiError::new(
        405,
        "method_not_allowed",
        format!(
            "'{cli}' channels are identified by (baseUrl, apiKey) and cannot be renamed in place; edit models through the model endpoints"
        ),
    ))
}

async fn delete_family_provider(context: &ServerContext, cli: &str, id: &str) -> ApiResult<Value> {
    let config = family_runtime_config(context, cli).await?;
    let channel = config
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["id"] == id))
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("Channel '{id}' not found")))?;
    let base_url = channel
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let api_key = channel
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if cli == "workbuddy" {
        crate::coding::workbuddy::commands::delete_workbuddy_provider(
            db_state(context),
            base_url,
            api_key,
            app(context),
        )
        .await
        .map_err(internal)?;
    } else {
        crate::coding::codebuddy::commands::delete_codebuddy_provider(
            db_state(context),
            base_url,
            api_key,
            app(context),
        )
        .await
        .map_err(internal)?;
    }
    Ok(json!({"deleted": id}))
}

fn family_channel_id(base_url: &str, api_key: &str) -> String {
    crate::coding::workbuddy_family::channel::channel_id(base_url, api_key)
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

pub async fn list_models(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
) -> ApiResult<Value> {
    let provider = get_provider(context, cli, provider_id).await?;
    let models = extract_models(cli, &provider);
    Ok(json!({ "models": models }))
}

fn extract_models(cli: &str, provider: &Value) -> Vec<Value> {
    if matches!(cli, "workbuddy" | "codebuddy") {
        return provider
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
    }
    if matches!(
        cli,
        "opencode" | "openclaw" | "pi" | "oh_my_pi" | "hermes" | "dsh" | "omo_native"
    ) {
        let base = provider.get("provider").unwrap_or(provider);
        return models_from_value(base.get("models"));
    }
    let settings = provider
        .get("settingsConfig")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if cli == "zcode" {
        return models_from_value(settings.get("models"));
    }
    if cli == "claudecode" || cli == "claudedesktop" {
        let env = settings
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        return [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "ANTHROPIC_CUSTOM_MODEL_OPTION",
            "ANTHROPIC_REASONING_MODEL",
        ]
        .iter()
        .filter_map(|key| env.get(*key).and_then(Value::as_str))
        .map(|id| json!({"id": id}))
        .collect();
    }
    if cli == "gemini" || cli == "geminicli" {
        let env = settings
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        return env
            .get("GEMINI_MODEL")
            .and_then(Value::as_str)
            .map(|id| vec![json!({"id": id})])
            .unwrap_or_default();
    }
    let catalog = settings
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .cloned();
    let mut models = models_from_value(catalog.as_ref());
    if cli == "codex" {
        if let Some(default_model) = codex_default_model(&settings) {
            if !models.iter().any(|model| model["id"] == default_model) {
                models.insert(0, json!({"id": default_model, "default": true}));
            }
        }
    }
    models
}

fn models_from_value(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(id) => Some(json!({"id": id})),
                Value::Object(object) => {
                    let id = object
                        .get("id")
                        .or_else(|| object.get("model"))
                        .or_else(|| object.get("modelId"))
                        .and_then(Value::as_str)?;
                    let mut out = object.clone();
                    out.insert("id".to_string(), json!(id));
                    Some(Value::Object(out))
                }
                _ => None,
            })
            .collect(),
        Some(Value::Object(map)) => map.keys().map(|key| json!({"id": key})).collect(),
        _ => Vec::new(),
    }
}

fn codex_default_model(settings: &Value) -> Option<String> {
    let config = settings.get("config").and_then(Value::as_str)?;
    crate::coding::proxy_gateway::provider_protocol::codex_model_from_config(config)
}

pub async fn add_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    body: Value,
) -> ApiResult<Value> {
    with_gateway_reattach_api(context, cli, || async {
        add_model_inner(context, cli, provider_id, body).await
    })
    .await
}

async fn add_model_inner(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    body: Value,
) -> ApiResult<Value> {
    let model_id = body
        .get("modelId")
        .or_else(|| body.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("'modelId' is required"))?
        .to_string();
    match cli {
        "claudecode" | "claudedesktop" | "codex" | "grok" | "kimi" | "geminicli" => {
            let mut raw = raw_db_provider(context, cli, provider_id).await?;
            let mut settings = settings_config_object(&raw);
            add_model_to_settings(cli, &mut settings, &model_id, &body)?;
            set_settings_config(&mut raw, settings);
            update_db_provider(context, cli, provider_id, raw).await
        }
        "zcode" => {
            let mut raw = raw_db_provider(context, cli, provider_id).await?;
            let mut settings = settings_config_object(&raw);
            add_model_to_zcode_settings(&mut settings, &model_id, &body)?;
            set_settings_config(&mut raw, settings);
            update_db_provider(context, cli, provider_id, raw).await
        }
        "opencode" | "openclaw" => {
            add_file_model(context, cli, provider_id, &model_id, &body).await
        }
        "pi" | "oh_my_pi" | "hermes" | "dsh" => {
            add_runtime_model(context, cli, provider_id, &model_id, &body).await
        }
        "omo_native" => add_omo_model(context, provider_id, &model_id, &body).await,
        "workbuddy" | "codebuddy" => {
            add_family_model(context, cli, provider_id, &model_id, &body).await
        }
        "antigravity" => Err(unsupported("antigravity", "Custom models")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

fn add_model_to_settings(
    cli: &str,
    settings: &mut Map<String, Value>,
    model_id: &str,
    body: &Value,
) -> ApiResult<()> {
    match cli {
        "claudecode" | "claudedesktop" => {
            let env = settings
                .entry("env".to_string())
                .or_insert_with(|| json!({}));
            if !env.is_object() {
                *env = json!({});
            }
            let role = body.get("role").and_then(Value::as_str).unwrap_or("model");
            let key = match role {
                "haiku" => "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                "sonnet" => "ANTHROPIC_DEFAULT_SONNET_MODEL",
                "opus" => "ANTHROPIC_DEFAULT_OPUS_MODEL",
                "fable" => "ANTHROPIC_DEFAULT_FABLE_MODEL",
                "reasoning" => "ANTHROPIC_REASONING_MODEL",
                _ => "ANTHROPIC_MODEL",
            };
            env[key] = json!(model_id);
        }
        "codex" | "grok" | "kimi" => {
            let catalog = settings
                .entry("modelCatalog".to_string())
                .or_insert_with(|| json!({"models": []}));
            if !catalog.is_object() {
                *catalog = json!({"models": []});
            }
            if !catalog.get("models").is_some_and(Value::is_array) {
                catalog["models"] = json!([]);
            }
            let models = catalog["models"].as_array_mut().expect("array");
            if !models.iter().any(|model| {
                model.get("model").and_then(Value::as_str) == Some(model_id)
                    || model.get("key").and_then(Value::as_str) == Some(model_id)
            }) {
                let mut entry = body.get("model").cloned().unwrap_or_else(|| json!({}));
                if !entry.is_object() {
                    entry = json!({});
                }
                entry["model"] = json!(model_id);
                if entry.get("key").is_none() && matches!(cli, "grok" | "kimi") {
                    entry["key"] = json!(model_id);
                }
                if let Some(name) = body.get("name").and_then(Value::as_str) {
                    entry["displayName"] = json!(name);
                }
                models.push(entry);
            }
        }
        "geminicli" => {
            let env = settings
                .entry("env".to_string())
                .or_insert_with(|| json!({}));
            if !env.is_object() {
                *env = json!({});
            }
            env["GEMINI_MODEL"] = json!(model_id);
        }
        _ => return Err(ApiError::not_found("Unknown database CLI")),
    }
    Ok(())
}

/// ZCode rows are typed (`ZcodeModelRow`), so a bare `{modelId}` is rejected by
/// the projection after the row was already stored. Build a `smart` row here,
/// mirroring the create dialog, and keep the default flags consistent.
fn add_model_to_zcode_settings(
    settings: &mut Map<String, Value>,
    model_id: &str,
    body: &Value,
) -> ApiResult<()> {
    let mut models = settings
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if models.iter().any(|model| {
        model.get("modelId").and_then(Value::as_str) == Some(model_id)
            || model.get("id").and_then(Value::as_str) == Some(model_id)
    }) {
        return Err(ApiError::bad_request(format!(
            "Model '{model_id}' already exists in this ZCode provider"
        )));
    }
    let mut entry = body.get("model").cloned().unwrap_or_else(|| json!({}));
    if !entry.is_object() {
        entry = json!({});
    }
    entry["modelId"] = json!(model_id);
    if entry.get("ruleKind").is_none() {
        entry["ruleKind"] = json!("smart");
    }
    if let Some(name) = body.get("name").and_then(Value::as_str) {
        entry["displayName"] = json!(name);
    }
    let has_default = models
        .iter()
        .any(|model| model.get("isDefault").and_then(Value::as_bool) == Some(true));
    if !has_default
        && settings
            .get("defaultModelId")
            .and_then(Value::as_str)
            .is_none()
    {
        entry["isDefault"] = json!(true);
        settings.insert("defaultModelId".to_string(), json!(model_id));
    }
    models.push(entry);
    settings.insert("models".to_string(), Value::Array(models));
    Ok(())
}

fn set_zcode_default_model(settings: &mut Map<String, Value>, model_id: &str) -> ApiResult<()> {
    let models = settings
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ApiError::bad_request("This ZCode provider has no models"))?;
    let mut found = false;
    for model in models.iter_mut() {
        let is_target = model.get("modelId").and_then(Value::as_str) == Some(model_id)
            || model.get("id").and_then(Value::as_str) == Some(model_id);
        if is_target {
            found = true;
        }
        model["isDefault"] = json!(is_target);
    }
    if !found {
        return Err(ApiError::not_found(format!(
            "Model '{model_id}' was not found in this ZCode provider"
        )));
    }
    settings.insert("defaultModelId".to_string(), json!(model_id));
    Ok(())
}

/// After a model is removed, a default that pointed at it must move to a
/// remaining row (or disappear when the list is empty), otherwise the stored
/// provider fails validation on the next projection.
fn fix_zcode_default_after_delete(settings: &mut Map<String, Value>, removed_id: &str) {
    let default_was_removed = settings
        .get("defaultModelId")
        .and_then(Value::as_str)
        .is_some_and(|value| value == removed_id);
    let models = settings
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .map(|models| {
            models.retain(|model| {
                model.get("modelId").and_then(Value::as_str) != Some(removed_id)
                    && model.get("id").and_then(Value::as_str) != Some(removed_id)
            });
            models.clone()
        })
        .unwrap_or_default();
    if !default_was_removed
        && models
            .iter()
            .any(|model| model.get("isDefault").and_then(Value::as_bool) == Some(true))
    {
        return;
    }
    match models.first().and_then(|model| {
        model
            .get("modelId")
            .or_else(|| model.get("id"))
            .and_then(Value::as_str)
    }) {
        Some(next_id) => {
            let next_id = next_id.to_string();
            if let Some(models) = settings.get_mut("models").and_then(Value::as_array_mut) {
                for model in models.iter_mut() {
                    let is_target = model.get("modelId").and_then(Value::as_str)
                        == Some(next_id.as_str())
                        || model.get("id").and_then(Value::as_str) == Some(next_id.as_str());
                    model["isDefault"] = json!(is_target);
                }
            }
            settings.insert("defaultModelId".to_string(), json!(next_id));
        }
        None => {
            // `null` survives the merge layer and deserializes to `None`;
            // removing the key would be re-added from the stored settings.
            settings.insert("defaultModelId".to_string(), Value::Null);
        }
    }
}

async fn add_file_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
    body: &Value,
) -> ApiResult<Value> {
    let patch_model = body.get("model").cloned().unwrap_or_else(|| json!({}));
    if cli == "opencode" {
        let mut config = read_opencode_config_value(context).await?;
        let provider = config
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .and_then(|providers| providers.get_mut(provider_id))
            .ok_or_else(|| {
                ApiError::not_found(format!("OpenCode provider '{provider_id}' not found"))
            })?;
        if !provider.get("models").is_some_and(Value::is_object) {
            provider["models"] = json!({});
        }
        let mut model = patch_model;
        if !model.is_object() {
            model = json!({});
        }
        model["name"] = body
            .get("name")
            .cloned()
            .or_else(|| model.get("name").cloned())
            .unwrap_or_else(|| json!(model_id));
        config["provider"][provider_id]["models"][model_id] = model;
        save_opencode_config_value(context, config).await?;
    } else {
        let mut config = read_openclaw_config_value(context).await?;
        let provider = config
            .get_mut("models")
            .and_then(|models| models.get_mut("providers"))
            .and_then(Value::as_object_mut)
            .and_then(|providers| providers.get_mut(provider_id))
            .ok_or_else(|| {
                ApiError::not_found(format!("OpenClaw provider '{provider_id}' not found"))
            })?;
        if !provider.get("models").is_some_and(Value::is_array) {
            provider["models"] = json!([]);
        }
        let models = provider["models"].as_array_mut().expect("array");
        let mut model = patch_model;
        if !model.is_object() {
            model = json!({});
        }
        model["id"] = json!(model_id);
        if let Some(name) = body.get("name").and_then(Value::as_str) {
            model["name"] = json!(name);
        }
        if !models.iter().any(|item| item["id"] == model_id) {
            models.push(model);
        }
        save_openclaw_config_value(context, config).await?;
    }
    list_models(context, cli, provider_id).await
}

async fn add_runtime_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
    body: &Value,
) -> ApiResult<Value> {
    let runtime_cli = runtime_cli_from_key(cli)?;
    let raw = runtime_config(context, runtime_cli).await?;
    let mut provider = raw
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["providerKey"] == provider_id))
        .and_then(|item| item.get("modelsProvider").or_else(|| item.get("provider")))
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("Provider '{provider_id}' not found")))?;
    let mut models = provider.get("models").cloned().unwrap_or_else(|| json!([]));
    let mut model = body.get("model").cloned().unwrap_or_else(|| json!({}));
    if !model.is_object() {
        model = json!({});
    }
    model["id"] = json!(model_id);
    if let Some(name) = body.get("name").and_then(Value::as_str) {
        model["name"] = json!(name);
    }
    match &mut models {
        Value::Array(items) => {
            if !items.iter().any(|item| item["id"] == model_id) {
                items.push(model);
            }
        }
        Value::Object(map) => {
            map.insert(model_id.to_string(), model);
        }
        _ => {
            models = json!([model]);
        }
    }
    provider["models"] = models;
    save_runtime_provider(context, runtime_cli, provider_id, provider, None).await?;
    list_models(context, cli, provider_id).await
}

fn runtime_cli_from_key(cli: &str) -> ApiResult<RuntimeCli> {
    match cli {
        "pi" => Ok(RuntimeCli::Pi),
        "oh_my_pi" => Ok(RuntimeCli::Omp),
        "hermes" => Ok(RuntimeCli::Hermes),
        "dsh" => Ok(RuntimeCli::Dsh),
        _ => Err(ApiError::not_found(format!(
            "'{cli}' is not a runtime-config CLI"
        ))),
    }
}

async fn add_family_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
    body: &Value,
) -> ApiResult<Value> {
    let config = family_runtime_config(context, cli).await?;
    let channel = config
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["id"] == provider_id))
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("Channel '{provider_id}' not found")))?;
    let base_url = channel
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let api_key = channel
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let input = json!({
        "previousId": null,
        "id": model_id,
        "name": body.get("name").and_then(Value::as_str).unwrap_or(model_id),
        "vendor": channel.get("name").and_then(Value::as_str).unwrap_or_default(),
        "baseUrl": base_url,
        "apiKey": api_key,
        "supportsToolCall": body.get("supportsToolCall").and_then(Value::as_bool).unwrap_or(true),
        "supportsImages": body.get("supportsImages").and_then(Value::as_bool).unwrap_or(false),
        "supportsReasoning": body.get("supportsReasoning").and_then(Value::as_bool).unwrap_or(false),
        "onlyReasoning": body.get("onlyReasoning").and_then(Value::as_bool).unwrap_or(false),
        "useCustomProtocol": body.get("useCustomProtocol").and_then(Value::as_bool).unwrap_or(false),
        "canDisableThinking": body.get("canDisableThinking").and_then(Value::as_bool).unwrap_or(false),
        "maxInputTokens": body.get("maxInputTokens"),
        "maxOutputTokens": body.get("maxOutputTokens"),
        "temperature": body.get("temperature"),
        "defaultEffort": body.get("defaultEffort"),
        "supportedEfforts": body.get("supportedEfforts").cloned().unwrap_or_else(|| json!([])),
    });
    if cli == "workbuddy" {
        let input: crate::coding::workbuddy::types::WorkbuddyModelInput =
            serde_json::from_value(input)
                .map_err(|error| invalid(format!("Invalid WorkBuddy model: {error}")))?;
        crate::coding::workbuddy::commands::save_workbuddy_model(
            db_state(context),
            input,
            app(context),
        )
        .await
        .map_err(internal)?;
    } else {
        let input: crate::coding::codebuddy::types::CodebuddyModelInput =
            serde_json::from_value(input)
                .map_err(|error| invalid(format!("Invalid CodeBuddy model: {error}")))?;
        crate::coding::codebuddy::commands::save_codebuddy_model(
            db_state(context),
            input,
            app(context),
        )
        .await
        .map_err(internal)?;
    }
    list_models(context, cli, provider_id).await
}

pub async fn delete_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
) -> ApiResult<Value> {
    with_gateway_reattach_api(context, cli, || async {
        delete_model_inner(context, cli, provider_id, model_id).await
    })
    .await
}

async fn delete_model_inner(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
) -> ApiResult<Value> {
    match cli {
        "workbuddy" | "codebuddy" => {
            if cli == "workbuddy" {
                crate::coding::workbuddy::commands::delete_workbuddy_model(
                    db_state(context),
                    model_id.to_string(),
                    app(context),
                )
                .await
                .map_err(internal)?;
            } else {
                crate::coding::codebuddy::commands::delete_codebuddy_model(
                    db_state(context),
                    model_id.to_string(),
                    app(context),
                )
                .await
                .map_err(internal)?;
            }
            Ok(json!({"deleted": model_id}))
        }
        "antigravity" => Err(unsupported("antigravity", "Custom models")),
        "claudecode" | "claudedesktop" | "codex" | "grok" | "kimi" | "geminicli" | "zcode" => {
            let mut raw = raw_db_provider(context, cli, provider_id).await?;
            let mut settings = settings_config_object(&raw);
            if cli == "zcode" {
                fix_zcode_default_after_delete(&mut settings, model_id);
            } else {
                let removed_default = is_default_catalog_model(cli, &settings, model_id);
                remove_model_from_settings(cli, &mut settings, model_id);
                if removed_default {
                    fix_catalog_default_after_delete(cli, &mut settings)?;
                }
            }
            set_settings_config(&mut raw, settings);
            update_db_provider(context, cli, provider_id, raw).await?;
            Ok(json!({"deleted": model_id}))
        }
        "opencode" | "openclaw" => {
            delete_file_model(context, cli, provider_id, model_id).await?;
            Ok(json!({"deleted": model_id}))
        }
        "pi" | "oh_my_pi" | "hermes" | "dsh" => {
            delete_runtime_model(context, cli, provider_id, model_id).await?;
            Ok(json!({"deleted": model_id}))
        }
        "omo_native" => {
            delete_omo_model(context, provider_id, model_id).await?;
            Ok(json!({"deleted": model_id}))
        }
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

fn remove_model_from_settings(cli: &str, settings: &mut Map<String, Value>, model_id: &str) {
    match cli {
        "claudecode" | "claudedesktop" => {
            if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
                for key in [
                    "ANTHROPIC_MODEL",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL",
                    "ANTHROPIC_DEFAULT_FABLE_MODEL",
                    "CLAUDE_CODE_SUBAGENT_MODEL",
                    "ANTHROPIC_CUSTOM_MODEL_OPTION",
                    "ANTHROPIC_REASONING_MODEL",
                ] {
                    if env.get(key).and_then(Value::as_str) == Some(model_id) {
                        env.remove(key);
                    }
                }
            }
        }
        "codex" | "grok" | "kimi" => {
            if let Some(models) = settings
                .get_mut("modelCatalog")
                .and_then(|catalog| catalog.get_mut("models"))
                .and_then(Value::as_array_mut)
            {
                models.retain(|model| {
                    model.get("model").and_then(Value::as_str) != Some(model_id)
                        && model.get("key").and_then(Value::as_str) != Some(model_id)
                });
            }
        }
        "geminicli" => {
            if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
                if env.get("GEMINI_MODEL").and_then(Value::as_str) == Some(model_id) {
                    env.remove("GEMINI_MODEL");
                }
            }
        }
        "zcode" => {
            if let Some(models) = settings.get_mut("models").and_then(Value::as_array_mut) {
                models.retain(|model| {
                    model.get("modelId").and_then(Value::as_str) != Some(model_id)
                        && model.get("id").and_then(Value::as_str) != Some(model_id)
                });
            }
        }
        _ => {}
    }
}

fn is_default_catalog_model(cli: &str, settings: &Map<String, Value>, model_id: &str) -> bool {
    match cli {
        "kimi" | "grok" => {
            let Some(default_key) = settings.get("defaultModelKey").and_then(Value::as_str) else {
                return false;
            };
            if default_key == model_id {
                return true;
            }
            // The runtime id may be the catalog entry's `model` while the
            // default points at its `key` (or the other way around).
            settings
                .get("modelCatalog")
                .and_then(|catalog| catalog.get("models"))
                .and_then(Value::as_array)
                .is_some_and(|models| {
                    models.iter().any(|model| {
                        let key = model.get("key").and_then(Value::as_str);
                        let model_value = model.get("model").and_then(Value::as_str);
                        let matches_target =
                            key == Some(model_id) || model_value == Some(model_id);
                        matches_target
                            && (key == Some(default_key) || model_value == Some(default_key))
                    })
                })
        }
        "codex" => codex_default_model(&Value::Object(settings.clone()))
            .is_some_and(|value| value == model_id),
        _ => false,
    }
}

/// Deleting the current default model used to leave `defaultModelKey` (Kimi /
/// Grok) or the TOML `model` (Codex) pointing at a model that no longer exists,
/// which Kimi rejects outright. Move the default to the first remaining model,
/// or drop it when the catalog is empty.
fn fix_catalog_default_after_delete(
    cli: &str,
    settings: &mut Map<String, Value>,
) -> ApiResult<()> {
    let remaining: Vec<String> = settings
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    model
                        .get("key")
                        .or_else(|| model.get("model"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    match cli {
        "kimi" | "grok" => {
            match remaining.first() {
                Some(next) => {
                    settings.insert("defaultModelKey".to_string(), json!(next));
                }
                None => {
                    // See `fix_zcode_default_after_delete`: omission cannot
                    // delete through the merge layer, but `null` can.
                    settings.insert("defaultModelKey".to_string(), Value::Null);
                }
            }
        }
        "codex" => {
            let config = settings
                .get("config")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let patched = match remaining.first() {
                Some(next) => crate::coding::codex::commands::patch_default_model_in_config_toml(
                    config, next,
                ),
                None => crate::coding::codex::commands::clear_default_model_in_config_toml(config),
            }
            .map_err(invalid)?;
            settings.insert("config".to_string(), json!(patched));
        }
        _ => {}
    }
    Ok(())
}

fn clear_opencode_model_selection(
    config: &mut Value,
    provider_id: &str,
    removed_model: &str,
) {
    let selection_matches = match config.get("model") {
        Some(Value::Object(selection)) => {
            selection.get("providerID").and_then(Value::as_str) == Some(provider_id)
                && selection.get("model").and_then(Value::as_str) == Some(removed_model)
        }
        Some(Value::String(selection)) => selection == &format!("{provider_id}/{removed_model}"),
        _ => false,
    };
    if !selection_matches {
        return;
    }
    let next = config
        .get("provider")
        .and_then(|providers| providers.get(provider_id))
        .and_then(|provider| provider.get("models"))
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next().cloned());
    match next {
        Some(next) => {
            config["model"] = json!({"providerID": provider_id, "model": next});
        }
        None => {
            if let Some(object) = config.as_object_mut() {
                object.remove("model");
            }
        }
    }
}

/// Drop a default selection that points at a removed model or provider.
///
/// OpenClaw requires `agents.defaults.model.primary` whenever that block
/// exists, so removing only the key would leave a document OpenClaw rejects on
/// the next save. Promote the first surviving candidate instead: another model
/// in the same provider, then the first configured fallback. With nothing left
/// to promote, remove the whole block (the schema treats it as optional).
fn clear_openclaw_model_selection(
    config: &mut Value,
    provider_id: &str,
    removed_model: Option<&str>,
) {
    let stale_primary = removed_model.map(|model| format!("{provider_id}/{model}"));
    let selection = config
        .get("agents")
        .and_then(|agents| agents.get("defaults"))
        .and_then(|defaults| defaults.get("model"));
    let Some(selection) = selection else {
        return;
    };
    let matches = match (
        &stale_primary,
        selection.get("primary").and_then(Value::as_str),
    ) {
        (Some(stale), Some(primary)) => primary == stale,
        (None, Some(primary)) => primary.starts_with(&format!("{provider_id}/")),
        _ => false,
    };
    if !matches {
        return;
    }

    let same_provider = config
        .get("models")
        .and_then(|models| models.get("providers"))
        .and_then(|providers| providers.get(provider_id))
        .and_then(|provider| provider.get("models"))
        .and_then(Value::as_array)
        .and_then(|models| {
            models.iter().find_map(|model| {
                model
                    .get("id")
                    .or_else(|| model.get("model"))
                    .and_then(Value::as_str)
                    .map(|id| format!("{provider_id}/{id}"))
            })
        });
    let fallback = selection
        .get("fallbacks")
        .and_then(Value::as_array)
        .and_then(|fallbacks| fallbacks.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    let replacement = same_provider
        .map(|model| (model, false))
        .or_else(|| fallback.map(|model| (model, true)));

    let Some(defaults) = config
        .get_mut("agents")
        .and_then(|agents| agents.get_mut("defaults"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    match replacement {
        Some((next, from_fallback)) => {
            let Some(model) = defaults.get_mut("model").and_then(Value::as_object_mut) else {
                return;
            };
            model.insert("primary".to_string(), json!(next));
            if from_fallback {
                if let Some(fallbacks) = model.get_mut("fallbacks").and_then(Value::as_array_mut) {
                    if !fallbacks.is_empty() {
                        fallbacks.remove(0);
                    }
                }
            }
        }
        None => {
            defaults.remove("model");
        }
    }
}

async fn delete_file_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
) -> ApiResult<()> {
    if cli == "opencode" {
        let mut config = read_opencode_config_value(context).await?;
        if let Some(models) = config
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .and_then(|providers| providers.get_mut(provider_id))
            .and_then(|provider| provider.get_mut("models"))
            .and_then(Value::as_object_mut)
        {
            models.remove(model_id);
        }
        clear_opencode_model_selection(&mut config, provider_id, model_id);
        save_opencode_config_value(context, config).await
    } else {
        let mut config = read_openclaw_config_value(context).await?;
        if let Some(models) = config
            .get_mut("models")
            .and_then(|models| models.get_mut("providers"))
            .and_then(Value::as_object_mut)
            .and_then(|providers| providers.get_mut(provider_id))
            .and_then(|provider| provider.get_mut("models"))
            .and_then(Value::as_array_mut)
        {
            models.retain(|model| model["id"] != model_id);
        }
        clear_openclaw_model_selection(&mut config, provider_id, Some(model_id));
        save_openclaw_config_value(context, config).await
    }
}

async fn delete_runtime_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
) -> ApiResult<()> {
    let runtime_cli = runtime_cli_from_key(cli)?;
    let raw = runtime_config(context, runtime_cli).await?;
    let mut provider = raw
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["providerKey"] == provider_id))
        .and_then(|item| item.get("modelsProvider").or_else(|| item.get("provider")))
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("Provider '{provider_id}' not found")))?;
    match provider.get_mut("models") {
        Some(Value::Array(items)) => items.retain(|model| model["id"] != model_id),
        Some(Value::Object(map)) => {
            map.remove(model_id);
        }
        _ => {}
    }
    save_runtime_provider(context, runtime_cli, provider_id, provider, None).await
}

pub async fn set_default_model(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
) -> ApiResult<Value> {
    with_gateway_reattach_api(context, cli, || async {
        set_default_model_inner(context, cli, provider_id, model_id).await
    })
    .await
}

async fn set_default_model_inner(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    model_id: &str,
) -> ApiResult<Value> {
    match cli {
        "claudecode" | "claudedesktop" | "geminicli" => {
            let mut raw = raw_db_provider(context, cli, provider_id).await?;
            let mut settings = settings_config_object(&raw);
            add_model_to_settings(cli, &mut settings, model_id, &json!({"role": "model"}))?;
            set_settings_config(&mut raw, settings);
            update_db_provider(context, cli, provider_id, raw).await?;
            Ok(json!({"defaultModel": model_id}))
        }
        "grok" | "kimi" => {
            let mut raw = raw_db_provider(context, cli, provider_id).await?;
            let mut settings = settings_config_object(&raw);
            settings.insert("defaultModelKey".to_string(), json!(model_id));
            set_settings_config(&mut raw, settings);
            update_db_provider(context, cli, provider_id, raw).await?;
            Ok(json!({"defaultModel": model_id}))
        }
        "codex" => {
            let mut raw = raw_db_provider(context, cli, provider_id).await?;
            let mut settings = settings_config_object(&raw);
            let config = settings
                .get("config")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let patched = crate::coding::codex::commands::patch_default_model_in_config_toml(
                &config, model_id,
            )
            .map_err(invalid)?;
            settings.insert("config".to_string(), json!(patched));
            set_settings_config(&mut raw, settings);
            update_db_provider(context, cli, provider_id, raw).await?;
            Ok(json!({"defaultModel": model_id}))
        }
        "zcode" => {
            let mut raw = raw_db_provider(context, cli, provider_id).await?;
            let mut settings = settings_config_object(&raw);
            set_zcode_default_model(&mut settings, model_id)?;
            set_settings_config(&mut raw, settings);
            update_db_provider(context, cli, provider_id, raw).await?;
            Ok(json!({"defaultModel": model_id}))
        }
        "opencode" | "openclaw" | "pi" | "oh_my_pi" | "hermes" | "dsh" | "omo_native" => {
            apply_provider(context, cli, provider_id, Some(model_id)).await
        }
        "workbuddy" | "codebuddy" | "antigravity" => Err(unsupported(cli, "Default model")),
        other => Err(ApiError::not_found(format!("Unknown CLI '{other}'"))),
    }
}

// ---------------------------------------------------------------------------
// Test / fetch models
// ---------------------------------------------------------------------------

/// Raw provider payload used to build outbound connections.
///
/// Connectivity tests and model discovery must never be fed the public,
/// redacted DTO: `***` is not a credential and would turn a valid key into a
/// guaranteed 401.
async fn raw_provider_for_connection(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
) -> ApiResult<Value> {
    match cli {
        "claudecode" | "claudedesktop" | "codex" | "grok" | "kimi" | "geminicli" | "zcode" => {
            raw_db_provider(context, cli, provider_id).await
        }
        "opencode" => {
            let config = read_opencode_config_value(context).await?;
            let provider = config
                .get("provider")
                .and_then(|providers| providers.get(provider_id))
                .cloned()
                .ok_or_else(|| {
                    ApiError::not_found(format!("OpenCode provider '{provider_id}' not found"))
                })?;
            Ok(json!({
                "id": provider_id,
                "providerKey": provider_id,
                "provider": provider,
            }))
        }
        "openclaw" => {
            let config = read_openclaw_config_value(context).await?;
            let provider = config
                .get("models")
                .and_then(|models| models.get("providers"))
                .and_then(|providers| providers.get(provider_id))
                .cloned()
                .ok_or_else(|| {
                    ApiError::not_found(format!("OpenClaw provider '{provider_id}' not found"))
                })?;
            Ok(json!({
                "id": provider_id,
                "providerKey": provider_id,
                "provider": provider,
            }))
        }
        "pi" | "oh_my_pi" | "hermes" | "dsh" => {
            let runtime_cli = runtime_cli_from_key(cli)?;
            let config = runtime_config(context, runtime_cli).await?;
            let provider = config
                .get("providers")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().find(|item| item["providerKey"] == provider_id))
                .and_then(|item| item.get("modelsProvider").or_else(|| item.get("provider")))
                .cloned()
                .ok_or_else(|| {
                    ApiError::not_found(format!("Provider '{provider_id}' not found"))
                })?;
            Ok(json!({"providerKey": provider_id, "provider": provider}))
        }
        "omo_native" => Ok(json!({
            "providerKey": provider_id,
            "provider": raw_omo_provider(context, provider_id).await?,
        })),
        "workbuddy" | "codebuddy" => {
            let config = family_runtime_config(context, cli).await?;
            config
                .get("providers")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().find(|item| item["id"] == provider_id))
                .cloned()
                .ok_or_else(|| ApiError::not_found(format!("Channel '{provider_id}' not found")))
        }
        other => Err(ApiError::not_found(format!(
            "'{other}' has no connectivity test"
        ))),
    }
}

/// DB providers store `settingsConfig` as a JSON string; parse it so the
/// connection builder can read `env` / `auth` like the direct paths do.
fn connection_provider(mut provider: Value) -> Value {
    if let Some(Value::String(raw)) = provider.get("settingsConfig").cloned() {
        if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
            provider["settingsConfig"] = parsed;
        }
    }
    provider
}

fn provider_config_value_mode(
    cli: &str,
) -> Option<crate::coding::open_code::models_api::ConfigValueMode> {
    use crate::coding::open_code::models_api::ConfigValueMode;
    match cli {
        "pi" => Some(ConfigValueMode::Pi),
        "oh_my_pi" => Some(ConfigValueMode::Omp),
        // OmO Native uses Pi's interpolation syntax, not OMP's.
        "omo_native" => Some(ConfigValueMode::Omo),
        _ => None,
    }
}

pub async fn test_provider(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    body: &Value,
) -> ApiResult<Value> {
    if cli == "antigravity" {
        return Err(unsupported(cli, "Provider test"));
    }
    let provider = get_provider(context, cli, provider_id).await?;
    let model_ids = body
        .get("modelIds")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            extract_models(cli, &provider)
                .into_iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
                .take(1)
                .collect()
        });
    let model_ids = if model_ids.is_empty() {
        return Err(ApiError::bad_request("This provider has no model to test"));
    } else {
        model_ids
    };
    let prompt = body
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("say hi!");
    let timeout_secs = body
        .get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(30);

    if let Some(gateway_cli) = gateway_cli_key(cli) {
        let request = crate::coding::proxy_gateway::types::GatewayConnectivityTestRequest {
            cli_key: gateway_cli,
            provider_id: provider_id.to_string(),
            prompt: prompt.to_string(),
            stream: Some(true),
            model_ids: model_ids.clone(),
            timeout_secs: Some(timeout_secs),
            vision_probe: false,
        };
        let result =
            crate::coding::proxy_gateway::commands::proxy_gateway_test_provider_model_connectivity(
                context
                    .app
                    .state::<crate::coding::proxy_gateway::ProxyGatewayState>(),
                db_state(context),
                context.app.state::<crate::plugins::PluginManagerState>(),
                request,
            )
            .await;
        if let Ok(response) = result {
            return Ok(super::routes::redacted(serde_json::to_value(response)?));
        }
    }

    let connection =
        provider_connection(cli, &connection_provider(raw_provider_for_connection(context, cli, provider_id).await?))?;
    let request = crate::coding::open_code::models_api::ConnectivityTestRequest {
        npm: connection.npm,
        api_format: None,
        provider_id: Some(provider_id.to_string()),
        base_url: connection.base_url,
        api_key: connection.api_key,
        reasoning_effort: None,
        headers: connection.headers,
        prompt: prompt.to_string(),
        temperature: None,
        max_tokens: None,
        max_output_tokens: None,
        stream: Some(true),
        body: None,
        model_ids,
        timeout_secs: Some(timeout_secs),
        config_value_mode: provider_config_value_mode(cli),
        vision_probe: false,
    };
    let response = crate::coding::open_code::models_api::test_provider_model_connectivity(
        db_state(context),
        request,
    )
    .await
    .map_err(internal)?;
    Ok(super::routes::redacted(serde_json::to_value(response)?))
}

struct ProviderConnection {
    npm: String,
    base_url: String,
    api_key: Option<String>,
    headers: Option<Value>,
}

/// A `customUrl` may move the models probe to another path, but never to
/// another origin while the provider's stored credential is attached: that
/// would hand the key to an endpoint the user never configured.
fn ensure_custom_url_same_origin(
    base_url: &str,
    custom_url: &str,
    has_credential: bool,
) -> ApiResult<()> {
    if !has_credential || base_url.trim().is_empty() {
        return Ok(());
    }
    let base = url::Url::parse(base_url.trim())
        .map_err(|_| ApiError::bad_request("The provider base URL is not a valid URL"))?;
    let custom = url::Url::parse(custom_url.trim())
        .map_err(|_| ApiError::bad_request("'customUrl' is not a valid URL"))?;
    let base_origin = (base.scheme(), base.host_str(), base.port_or_known_default());
    let custom_origin = (
        custom.scheme(),
        custom.host_str(),
        custom.port_or_known_default(),
    );
    if base_origin != custom_origin {
        return Err(ApiError::bad_request(
            "'customUrl' must use the same origin as the provider base URL; refusing to send the provider credential to another host",
        ));
    }
    Ok(())
}

fn provider_connection(cli: &str, provider: &Value) -> ApiResult<ProviderConnection> {
    let default_npm = "@ai-sdk/openai-compatible".to_string();
    if matches!(
        cli,
        "opencode" | "pi" | "oh_my_pi" | "hermes" | "dsh" | "omo_native"
    ) {
        let base = provider.get("provider").unwrap_or(provider);
        let npm = base
            .get("npm")
            .and_then(Value::as_str)
            .unwrap_or(&default_npm)
            .to_string();
        let options = base.get("options").unwrap_or(base);
        let base_url = options
            .get("baseURL")
            .or_else(|| options.get("baseUrl"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return Ok(ProviderConnection {
            npm,
            base_url,
            api_key: options
                .get("apiKey")
                .and_then(Value::as_str)
                .map(str::to_string),
            headers: options.get("headers").cloned(),
        });
    }
    if matches!(cli, "workbuddy" | "codebuddy") {
        return Ok(ProviderConnection {
            npm: default_npm,
            base_url: provider
                .get("baseUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            api_key: provider
                .get("apiKey")
                .and_then(Value::as_str)
                .map(str::to_string),
            headers: None,
        });
    }
    let settings = provider
        .get("settingsConfig")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match cli {
        "claudecode" | "claudedesktop" => {
            let env = settings.get("env").cloned().unwrap_or_else(|| json!({}));
            Ok(ProviderConnection {
                npm: default_npm,
                base_url: env
                    .get("ANTHROPIC_BASE_URL")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                api_key: env
                    .get("ANTHROPIC_AUTH_TOKEN")
                    .or_else(|| env.get("ANTHROPIC_API_KEY"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                headers: None,
            })
        }
        "codex" => {
            let api_key = settings
                .get("auth")
                .and_then(|auth| auth.get("OPENAI_API_KEY"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let base_url = settings
                .get("config")
                .and_then(Value::as_str)
                .and_then(
                    crate::coding::proxy_gateway::provider_protocol::codex_base_url_from_config,
                )
                .unwrap_or_default();
            Ok(ProviderConnection {
                npm: default_npm,
                base_url,
                api_key,
                headers: None,
            })
        }
        "grok" => Ok(ProviderConnection {
            npm: default_npm,
            base_url: settings
                .get("baseUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            api_key: settings
                .get("auth")
                .and_then(|auth| auth.get("API_KEY"))
                .and_then(Value::as_str)
                .map(str::to_string),
            headers: None,
        }),
        "kimi" => {
            let key = settings
                .get("defaultModelKey")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let base_url = settings
                .get("providerConfigs")
                .and_then(|configs| configs.get(key))
                .and_then(|config| config.get("base_url"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Ok(ProviderConnection {
                npm: default_npm,
                base_url,
                api_key: settings
                    .get("auth")
                    .and_then(|auth| auth.get("API_KEY"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                headers: None,
            })
        }
        "geminicli" => {
            let env = settings.get("env").cloned().unwrap_or_else(|| json!({}));
            Ok(ProviderConnection {
                npm: "@ai-sdk/google".to_string(),
                base_url: env
                    .get("GOOGLE_GEMINI_BASE_URL")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                api_key: env
                    .get("GEMINI_API_KEY")
                    .or_else(|| env.get("GOOGLE_API_KEY"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                headers: None,
            })
        }
        "zcode" => {
            let config = settings.get("config").cloned().unwrap_or_else(|| json!({}));
            Ok(ProviderConnection {
                npm: default_npm,
                base_url: config
                    .get("api")
                    .and_then(|api| api.get("baseUrl"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                api_key: config
                    .get("access")
                    .and_then(|access| access.get("apiKey"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                headers: None,
            })
        }
        _ => Err(ApiError::not_found(format!(
            "'{cli}' has no connectivity test"
        ))),
    }
}

fn gateway_cli_key(cli: &str) -> Option<crate::coding::proxy_gateway::types::GatewayCliKey> {
    use crate::coding::proxy_gateway::types::GatewayCliKey;
    match cli {
        "claudecode" => Some(GatewayCliKey::Claude),
        "claudedesktop" => Some(GatewayCliKey::ClaudeDesktop),
        "codex" => Some(GatewayCliKey::Codex),
        "grok" => Some(GatewayCliKey::Grok),
        "kimi" => Some(GatewayCliKey::Kimi),
        "geminicli" => Some(GatewayCliKey::Gemini),
        _ => None,
    }
}

pub async fn fetch_models(
    context: &ServerContext,
    cli: &str,
    provider_id: &str,
    body: &Value,
) -> ApiResult<Value> {
    if cli == "antigravity" {
        return Err(unsupported(cli, "Model discovery"));
    }
    let connection =
        provider_connection(cli, &connection_provider(raw_provider_for_connection(context, cli, provider_id).await?))?;
    let api_type = match body.get("apiType").and_then(Value::as_str) {
        Some("native") => crate::coding::open_code::models_api::ApiType::Native,
        _ => crate::coding::open_code::models_api::ApiType::OpenaiCompat,
    };
    let custom_url = body
        .get("customUrl")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(custom_url) = custom_url.as_deref() {
        ensure_custom_url_same_origin(&connection.base_url, custom_url, connection.api_key.is_some())?;
    }
    let request = crate::coding::open_code::models_api::FetchModelsRequest {
        provider_id: Some(provider_id.to_string()),
        base_url: connection.base_url,
        api_key: connection.api_key,
        headers: connection.headers,
        api_type,
        sdk_type: body
            .get("sdkType")
            .and_then(Value::as_str)
            .map(str::to_string),
        custom_url,
        config_value_mode: provider_config_value_mode(cli),
    };
    let response =
        crate::coding::open_code::models_api::fetch_provider_models(db_state(context), request)
            .await
            .map_err(internal)?;
    Ok(serde_json::to_value(response)?)
}

// ---------------------------------------------------------------------------
// Helpers used by routes for prompt/config dispatch
// ---------------------------------------------------------------------------

pub fn unsupported(cli: &str, feature: &str) -> ApiError {
    // Its own code, not `read_only`: the user cannot fix this by changing the
    // approval policy, and the CLI must not tell them to try.
    ApiError::new(
        400,
        "unsupported",
        format!("{feature} is not supported for CLI '{cli}'"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_preconditions_map_to_not_found_and_conflict() {
        let missing = command_error("ZCode provider 'x' not found".to_string());
        assert_eq!((missing.status, missing.code), (404, "not_found"));

        let applied = command_error("The applied Kimi provider cannot be deleted".to_string());
        assert_eq!((applied.status, applied.code), (409, "conflict"));

        let unexpected = command_error("disk exploded".to_string());
        assert_eq!(
            (unexpected.status, unexpected.code),
            (500, "internal_error")
        );
    }

    #[test]
    fn aliases_normalize_to_the_canonical_seventeen() {
        assert_eq!(normalize_cli_key("claude"), Some("claudecode"));
        assert_eq!(normalize_cli_key("claude-code"), Some("claudecode"));
        assert_eq!(normalize_cli_key("gemini"), Some("geminicli"));
        assert_eq!(normalize_cli_key("OMP"), Some("oh_my_pi"));
        assert_eq!(normalize_cli_key("oh-my-pi"), Some("oh_my_pi"));
        assert_eq!(normalize_cli_key("omo"), Some("omo_native"));
        assert_eq!(normalize_cli_key("nope"), None);
        for key in CLI_KEYS {
            assert!(normalize_cli_key(key).is_some(), "{key} must normalize");
        }
    }

    #[test]
    fn capability_matrix_covers_every_cli_and_marks_special_cases() {
        let matrix = capability_matrix();
        let rows = matrix["clis"].as_array().unwrap();
        assert_eq!(rows.len(), 17);
        let antigravity = rows
            .iter()
            .find(|row| row["cliKey"] == "antigravity")
            .unwrap();
        assert_eq!(antigravity["providerCrud"], false);
        let codebuddy = rows
            .iter()
            .find(|row| row["cliKey"] == "codebuddy")
            .unwrap();
        assert_eq!(codebuddy["apply"], false);
        let claude = rows
            .iter()
            .find(|row| row["cliKey"] == "claudecode")
            .unwrap();
        assert_eq!(claude["kind"], "database");
        assert_eq!(claude["providerCrud"], true);
    }

    #[test]
    fn db_model_edits_merge_into_a_string_settings_config() {
        // Regression for the seven DB CLIs: add/delete/set-default pass the raw
        // row (`settingsConfig` is a JSON string). The old merge ignored that
        // shape, returned 200, and silently kept the old model list.
        for cli in [
            "claudecode",
            "claudedesktop",
            "codex",
            "grok",
            "kimi",
            "geminicli",
            "zcode",
        ] {
            let mut stored = json!({
                "id": "p1",
                "settingsConfig": r#"{"models":[{"id":"old"}],"keep":"yes"}"#,
            });
            merge_settings_config(
                &mut stored,
                &json!({
                    "settingsConfig": r#"{"models":[{"id":"new"}]}"#,
                }),
            );
            let merged: Value =
                serde_json::from_str(stored["settingsConfig"].as_str().unwrap()).unwrap();
            assert_eq!(
                merged["models"][0]["id"], "new",
                "{cli} must persist the new model list"
            );
            assert_eq!(merged["keep"], "yes", "{cli} must keep unknown keys");
        }
    }

    #[test]
    fn canonical_fields_ignore_mask_placeholders_and_create_rejects_them() {
        assert_eq!(canonical_string(&json!({"apiKey": "***"}), "apiKey"), None);
        assert_eq!(canonical_string(&json!({"apiKey": "sk-real"}), "apiKey"), Some("sk-real"));
        assert!(ensure_no_redacted_input(
            &json!({"settingsConfig": {"env": {"ANTHROPIC_AUTH_TOKEN": "***"}}}),
            "create a provider"
        )
        .is_err());
        assert!(ensure_no_redacted_input(&json!({"apiKey": "sk-real"}), "create a provider").is_ok());
    }

    #[test]
    fn omo_update_merge_keeps_a_real_api_key_when_the_patch_echoes_the_mask() {
        // Mirrors `update_omo_provider`: start from the raw models.json entry,
        // merge the patch, and never let `***` replace the stored credential.
        let mut raw = json!({
            "apiKey": "sk-real",
            "baseUrl": "https://omo.test/v1",
            "models": [{"id": "m1"}],
        });
        super::super::redact::merge_patch_preserving_secrets(
            &mut raw,
            json!({
                "apiKey": "***",
                "baseUrl": "https://new.test/v1",
                "models": [{"id": "m1"}, {"id": "m2"}],
            }),
        );
        assert_eq!(raw["apiKey"], "sk-real");
        assert_eq!(raw["baseUrl"], "https://new.test/v1");
        assert_eq!(raw["models"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn connectivity_tests_pass_the_cli_specific_config_value_mode() {
        use crate::coding::open_code::models_api::ConfigValueMode;
        assert_eq!(provider_config_value_mode("pi"), Some(ConfigValueMode::Pi));
        assert_eq!(
            provider_config_value_mode("oh_my_pi"),
            Some(ConfigValueMode::Omp)
        );
        assert_eq!(
            provider_config_value_mode("omo_native"),
            Some(ConfigValueMode::Omo)
        );
        assert_eq!(provider_config_value_mode("codex"), None);
    }

    #[test]
    fn custom_model_url_must_keep_the_provider_origin_when_a_key_is_attached() {
        assert!(ensure_custom_url_same_origin(
            "https://api.test/v1",
            "https://api.test/v1/models?x=1",
            true
        )
        .is_ok());
        assert!(ensure_custom_url_same_origin(
            "https://api.test/v1",
            "https://evil.test/v1/models",
            true
        )
        .is_err());
        // No credential means no trust boundary to cross.
        assert!(ensure_custom_url_same_origin(
            "https://api.test/v1",
            "https://evil.test/v1/models",
            false
        )
        .is_ok());
    }

    #[test]
    fn family_channel_ids_are_stable_and_distinct() {
        let one = family_channel_id("https://a.test", "sk-a");
        let two = family_channel_id("https://a.test", "sk-a");
        let three = family_channel_id("https://b.test", "sk-a");
        assert_eq!(one, two);
        assert_ne!(one, three);
        assert!(one.starts_with("channel-"));
        assert_eq!(one.len(), "channel-".len() + 16);
    }

    #[test]
    fn canonical_fields_land_in_the_expected_cli_shapes() {
        let mut claude = Map::new();
        apply_canonical_fields(
            "claudecode",
            &json!({"baseUrl": "https://x.test", "apiKey": "sk-a", "model": "m1"}),
            &mut claude,
        );
        assert_eq!(claude["env"]["ANTHROPIC_BASE_URL"], "https://x.test");
        assert_eq!(claude["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-a");
        assert_eq!(claude["env"]["ANTHROPIC_MODEL"], "m1");

        let mut gemini = Map::new();
        apply_canonical_fields(
            "geminicli",
            &json!({"apiKey": "g-key", "baseUrl": "https://g.test"}),
            &mut gemini,
        );
        assert_eq!(gemini["env"]["GEMINI_API_KEY"], "g-key");
        assert_eq!(gemini["env"]["GOOGLE_GEMINI_BASE_URL"], "https://g.test");
    }

    #[test]
    fn canonical_fields_build_typed_zcode_grok_and_kimi_settings() {
        let mut zcode = Map::new();
        apply_canonical_fields(
            "zcode",
            &json!({
                "name": "My ZCode",
                "baseUrl": "https://z.test",
                "apiKey": "sk-z",
                "model": "z-model",
            }),
            &mut zcode,
        );
        assert_eq!(zcode["providerId"], "custom:my-zcode");
        assert_eq!(zcode["providerName"], "My ZCode");
        assert_eq!(zcode["config"]["api"]["baseUrl"], "https://z.test");
        assert_eq!(zcode["config"]["access"]["apiKey"], "sk-z");
        assert_eq!(zcode["models"][0]["modelId"], "z-model");
        assert_eq!(zcode["defaultModelId"], "z-model");
        let parsed: crate::coding::zcode::types::ZcodeSettingsConfig =
            serde_json::from_value(Value::Object(zcode)).unwrap();
        assert_eq!(parsed.provider_id, "custom:my-zcode");
        assert_eq!(parsed.models.len(), 1);
        assert!(parsed.models[0].is_default);

        let mut grok = Map::new();
        apply_canonical_fields(
            "grok",
            &json!({
                "baseUrl": "https://g.test",
                "apiKey": "sk-g",
                "model": "grok-4-fast",
            }),
            &mut grok,
        );
        assert_eq!(grok["auth"]["API_KEY"], "sk-g");
        assert_eq!(grok["baseUrl"], "https://g.test");
        assert_eq!(grok["modelCatalog"]["models"][0]["model"], "grok-4-fast");
        assert_eq!(
            grok["modelCatalog"]["models"][0]["baseUrl"],
            "https://g.test"
        );
        assert_eq!(grok["defaultModelKey"], "grok-4-fast");

        let mut kimi = Map::new();
        apply_canonical_fields(
            "kimi",
            &json!({
                "baseUrl": "https://k.test",
                "apiKey": "sk-k",
                "model": "kimi-k2",
            }),
            &mut kimi,
        );
        assert_eq!(kimi["auth"]["API_KEY"], "sk-k");
        assert_eq!(
            kimi["providerConfigs"]["custom"]["base_url"],
            "https://k.test"
        );
        assert_eq!(kimi["modelCatalog"]["models"][0]["provider"], "custom");
        assert_eq!(kimi["defaultModelKey"], "kimi-k2");
    }

    #[test]
    fn provider_update_rejects_a_masked_canonical_key() {
        let error = reject_redacted_canonical_key(&json!({"apiKey": "***"})).unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(error.code, "bad_request");
        // Omitting the key keeps the stored secret; a full value is accepted.
        assert!(reject_redacted_canonical_key(&json!({"name": "x"})).is_ok());
        assert!(reject_redacted_canonical_key(&json!({"apiKey": "sk-real"})).is_ok());
        // Nested settingsConfig echoes are still handled by the merge layer.
        assert!(reject_redacted_canonical_key(&json!({
            "settingsConfig": {"auth": {"API_KEY": "***"}}
        }))
        .is_ok());
    }

    #[test]
    fn zcode_added_models_are_typed_and_keep_one_default() {
        let mut settings = json!({
            "providerId": "custom:demo",
            "providerName": "Demo",
            "config": {"api": {"baseUrl": "https://demo.test"}},
            "models": [{"modelId": "model-a", "ruleKind": "smart", "isDefault": true}],
            "defaultModelId": "model-a"
        })
        .as_object()
        .unwrap()
        .clone();

        add_model_to_zcode_settings(&mut settings, "model-b", &json!({"name": "Model B"}))
            .unwrap();
        let parsed: crate::coding::zcode::types::ZcodeSettingsConfig =
            serde_json::from_value(Value::Object(settings.clone())).unwrap();
        assert_eq!(parsed.models.len(), 2);
        assert_eq!(parsed.models[1].model_id, "model-b");
        assert_eq!(
            parsed.models[1].rule_kind,
            crate::coding::zcode::types::ZcodeModelRuleKind::Smart
        );
        assert_eq!(parsed.models[1].display_name.as_deref(), Some("Model B"));
        assert!(!parsed.models[1].is_default);
        assert!(parsed.models[0].is_default);

        set_zcode_default_model(&mut settings, "model-b").unwrap();
        let parsed: crate::coding::zcode::types::ZcodeSettingsConfig =
            serde_json::from_value(Value::Object(settings)).unwrap();
        assert_eq!(parsed.default_model_id.as_deref(), Some("model-b"));
        assert!(!parsed.models[0].is_default);
        assert!(parsed.models[1].is_default);
    }

    #[test]
    fn zcode_deleting_the_default_moves_it_to_the_next_row() {
        let mut settings = json!({
            "providerId": "custom:demo",
            "models": [
                {"modelId": "model-a", "ruleKind": "smart", "isDefault": true},
                {"modelId": "model-b", "ruleKind": "smart", "isDefault": false}
            ],
            "defaultModelId": "model-a"
        })
        .as_object()
        .unwrap()
        .clone();

        fix_zcode_default_after_delete(&mut settings, "model-a");
        let parsed: crate::coding::zcode::types::ZcodeSettingsConfig =
            serde_json::from_value(Value::Object(settings.clone())).unwrap();
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].model_id, "model-b");
        assert!(parsed.models[0].is_default);
        assert_eq!(parsed.default_model_id.as_deref(), Some("model-b"));

        fix_zcode_default_after_delete(&mut settings, "model-b");
        let parsed: crate::coding::zcode::types::ZcodeSettingsConfig =
            serde_json::from_value(Value::Object(settings)).unwrap();
        assert!(parsed.models.is_empty());
        assert_eq!(parsed.default_model_id, None);
    }

    #[test]
    fn zcode_validation_rejects_a_bare_model_row_before_it_is_stored() {
        let raw = json!({
            "settingsConfig": json!({
                "providerId": "custom:demo",
                "models": [{"id": "model-a"}]
            })
            .to_string()
        });
        let error = validate_zcode_settings(&raw).unwrap_err();
        assert_eq!(error.code, "bad_request");
        assert!(error.message.contains("Invalid ZCode provider settings"));
    }

    #[test]
    fn deleting_the_kimi_default_falls_back_or_drops_the_key() {
        let mut settings = json!({
            "modelCatalog": {"models": [
                {"key": "model-a", "model": "model-a"},
                {"key": "model-b", "model": "model-b"}
            ]},
            "defaultModelKey": "model-a"
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(is_default_catalog_model("kimi", &settings, "model-a"));
        remove_model_from_settings("kimi", &mut settings, "model-a");
        fix_catalog_default_after_delete("kimi", &mut settings).unwrap();
        assert_eq!(settings["defaultModelKey"], json!("model-b"));

        remove_model_from_settings("kimi", &mut settings, "model-b");
        fix_catalog_default_after_delete("kimi", &mut settings).unwrap();
        // The merge layer cannot delete an omitted key, so "no default" is an
        // explicit null rather than a removed field.
        assert_eq!(settings.get("defaultModelKey"), Some(&Value::Null));
    }

    #[test]
    fn file_cli_model_selections_do_not_dangle_after_a_delete() {
        let mut opencode = json!({
            "provider": {"demo": {"models": {"keep": {}}}},
            "model": {"providerID": "demo", "model": "gone"}
        });
        clear_opencode_model_selection(&mut opencode, "demo", "gone");
        assert_eq!(opencode["model"], json!({"providerID": "demo", "model": "keep"}));

        let mut openclaw = json!({
            "models": {"providers": {"demo": {"models": [{"id": "keep"}]}}},
            "agents": {"defaults": {"model": {"primary": "demo/gone", "fallbacks": ["x"]}}}
        });
        clear_openclaw_model_selection(&mut openclaw, "demo", Some("gone"));
        assert_eq!(openclaw["agents"]["defaults"]["model"]["primary"], json!("demo/keep"));
        assert_eq!(openclaw["agents"]["defaults"]["model"]["fallbacks"], json!(["x"]));

        // Deleting the provider removes its models too, so the first fallback
        // becomes the primary; OpenClaw rejects a model block without one.
        let mut provider_deleted = json!({
            "models": {"providers": {}},
            "agents": {"defaults": {"model": {"primary": "demo/gone", "fallbacks": ["other/model", "x"]}}}
        });
        clear_openclaw_model_selection(&mut provider_deleted, "demo", None);
        assert_eq!(
            provider_deleted["agents"]["defaults"]["model"]["primary"],
            json!("other/model")
        );
        assert_eq!(
            provider_deleted["agents"]["defaults"]["model"]["fallbacks"],
            json!(["x"])
        );
        assert!(
            serde_json::from_value::<crate::coding::open_claw::types::OpenClawConfig>(
                provider_deleted
            )
            .is_ok()
        );

        // With nothing left to promote, drop the optional block rather than
        // leaving a `model` object that fails OpenClaw's deserializer.
        let mut dropped = json!({
            "agents": {"defaults": {"model": {"primary": "demo/gone"}}}
        });
        clear_openclaw_model_selection(&mut dropped, "demo", Some("gone"));
        assert!(dropped["agents"]["defaults"].get("model").is_none());
        assert!(
            serde_json::from_value::<crate::coding::open_claw::types::OpenClawConfig>(dropped)
                .is_ok()
        );
    }
}
