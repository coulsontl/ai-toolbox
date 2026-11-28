//! Wire installed gateway plugin providers into each coding CLI tab.
//!
//! Two families of CLI live here:
//!
//! - **Gateway takeover** CLIs keep providers in SQLite (`cli_provider`). A
//!   plugin provider is written as a normal record whose `meta.pluginProvider`
//!   points at the plugin; `runtime::providers::provider_from_record` then
//!   loads it as a plugin-backed upstream.
//! - **Direct-route** CLIs are configured by files. Their entries point at
//!   `http://127.0.0.1:<port>/plugin/<provider_id>/v1`, so the CLI talks to
//!   the gateway's direct plugin route instead of an upstream.
//!
//! Both modes are idempotent: re-adding the same plugin provider updates the
//! existing entry instead of creating a duplicate card.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, State};

use crate::coding::dsh::types::DshCredentialInput;
use crate::coding::proxy_gateway::settings::load_settings_from_sqlite_state;
use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::coding::proxy_gateway::types::{GatewayCliKey, ProxyGatewaySettings};
use crate::coding::proxy_gateway::ProxyGatewayState;
use crate::coding::workbuddy_family::models_json::{
    build_model_entry, entry_id, normalize_channel_base_url, ModelEntryInput, ModelsDocument,
};
use crate::coding::zcode::types::ZcodeSettingsConfig;
use crate::db::helpers::{db_get, db_list, db_put};
use crate::db::schema::{Cli, DbTable};
use crate::db::SqliteDbState;

use super::gateway_bridge::{
    self, npm_to_ai_protocol, PluginAccountStrategy, PluginModelInfo, PluginProviderRef,
    PluginProviderRow,
};
use super::PluginManagerState;

/// Placeholder credential written into every generated CLI config. The gateway
/// replaces the auth header before the request reaches the plugin host, so the
/// value is never used upstream; it only has to be non-empty for CLIs that
/// refuse to start without an API key.
const PLUGIN_API_KEY: &str = "ai-toolbox-plugin";

/// One coding CLI tab as far as plugin integration is concerned.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginCliTarget {
    /// Frontend `MODULES` sub-tab key.
    pub cli_key: String,
    pub label: String,
    /// `gateway_takeover` | `direct_plugin_route`.
    pub mode: String,
    pub supported: bool,
    pub reason: Option<String>,
}

/// Every coding sub-tab in sidebar order.
const CLI_TARGETS: &[(&str, &str, &str)] = &[
    ("claudecode", "Claude Code", "gateway_takeover"),
    ("claudedesktop", "Claude Desktop", "gateway_takeover"),
    ("codex", "Codex", "gateway_takeover"),
    ("grok", "Grok CLI", "gateway_takeover"),
    ("kimi", "Kimi CLI", "gateway_takeover"),
    ("geminicli", "Gemini CLI", "gateway_takeover"),
    ("antigravity", "Antigravity", "gateway_takeover"),
    ("zcode", "ZCode", "direct_plugin_route"),
    ("workbuddy", "WorkBuddy", "direct_plugin_route"),
    ("codebuddy", "CodeBuddy", "direct_plugin_route"),
    ("opencode", "OpenCode", "direct_plugin_route"),
    ("openclaw", "OpenClaw", "direct_plugin_route"),
    ("pi", "Pi", "direct_plugin_route"),
    ("oh_my_pi", "Oh My Pi", "direct_plugin_route"),
    ("omo_native", "OMO Native", "direct_plugin_route"),
    ("hermes", "Hermes", "direct_plugin_route"),
    ("dsh", "DSH", "direct_plugin_route"),
];

/// Antigravity CLI (`agy`) has no custom-provider path: it only supports
/// Google OAuth plus an optional GCP project, and the module explicitly
/// rejects API keys and custom base URLs. Writing a fake provider would look
/// configured while never working.
const ANTIGRAVITY_REASON: &str =
    "Antigravity CLI 只支持 Google 官方 OAuth，没有自定义供应商/API Key/Base URL 入口，无法接入插件 provider。";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddPluginProviderToCliInput {
    pub cli_key: String,
    pub plugin_spec: String,
    pub provider_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub account_strategy: PluginAccountStrategy,
    #[serde(default)]
    pub account_key: Option<String>,
    /// Empty means "every model the plugin provider exposes".
    #[serde(default)]
    pub model_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddPluginProviderToCliResult {
    pub cli_key: String,
    pub provider_id: String,
    pub provider_name: String,
    pub mode: String,
    pub endpoint_url: String,
    pub model_count: usize,
    pub gateway_running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPluginModelsResult {
    pub cli_key: String,
    pub cli_provider_id: String,
    pub model_count: usize,
    /// Model ids the refreshed list added.
    pub added: usize,
    /// Model ids that exist in both the old and refreshed lists.
    pub updated: usize,
}

/// Direct-route CLIs, keyed by the tab key the frontend sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileCli {
    OpenCode,
    OpenClaw,
    Pi,
    OhMyPi,
    OmoNative,
    Hermes,
    Dsh,
    Zcode,
    Workbuddy,
    Codebuddy,
}

pub fn cli_targets() -> Vec<PluginCliTarget> {
    CLI_TARGETS
        .iter()
        .map(|(cli_key, label, mode)| {
            let supported = *cli_key != "antigravity";
            PluginCliTarget {
                cli_key: (*cli_key).to_string(),
                label: (*label).to_string(),
                mode: (*mode).to_string(),
                supported,
                reason: (!supported).then(|| ANTIGRAVITY_REASON.to_string()),
            }
        })
        .collect()
}

#[tauri::command]
pub fn plugins_list_cli_targets() -> Vec<PluginCliTarget> {
    cli_targets()
}

fn normalize_cli_key(cli_key: &str) -> Result<String, String> {
    let normalized = cli_key.trim().to_ascii_lowercase();
    if CLI_TARGETS
        .iter()
        .any(|(key, _, _)| *key == normalized.as_str())
    {
        Ok(normalized)
    } else {
        Err(format!("Unknown coding CLI '{cli_key}'"))
    }
}

fn takeover_cli(cli_key: &str) -> Option<Cli> {
    match cli_key {
        "claudecode" => Some(Cli::Claude),
        "claudedesktop" => Some(Cli::ClaudeDesktop),
        "codex" => Some(Cli::Codex),
        "grok" => Some(Cli::Grok),
        "kimi" => Some(Cli::Kimi),
        "geminicli" => Some(Cli::Gemini),
        _ => None,
    }
}

fn file_cli(cli_key: &str) -> Option<FileCli> {
    match cli_key {
        "opencode" => Some(FileCli::OpenCode),
        "openclaw" => Some(FileCli::OpenClaw),
        "pi" => Some(FileCli::Pi),
        "oh_my_pi" => Some(FileCli::OhMyPi),
        "omo_native" => Some(FileCli::OmoNative),
        "hermes" => Some(FileCli::Hermes),
        "dsh" => Some(FileCli::Dsh),
        "zcode" => Some(FileCli::Zcode),
        "workbuddy" => Some(FileCli::Workbuddy),
        "codebuddy" => Some(FileCli::Codebuddy),
        _ => None,
    }
}

/// Keep each direct-route module's own change/sync event contract; the generic
/// `config-changed` is emitted once by the command itself.
fn emit_file_cli_sync_events(app: &AppHandle, cli: FileCli) {
    if cli == FileCli::OpenClaw {
        let _ = app.emit("openclaw-config-changed", "window");
    }
    #[cfg(target_os = "windows")]
    {
        let event = match cli {
            FileCli::OpenCode => Some("wsl-sync-request-opencode"),
            FileCli::OpenClaw => Some("wsl-sync-request-openclaw"),
            FileCli::Pi => Some("wsl-sync-request-pi"),
            FileCli::OhMyPi => Some("wsl-sync-request-omp"),
            FileCli::Hermes => Some("wsl-sync-request-hermes"),
            FileCli::Dsh => Some("wsl-sync-request-dsh"),
            FileCli::Zcode => Some("wsl-sync-request-zcode"),
            FileCli::OmoNative | FileCli::Workbuddy | FileCli::Codebuddy => None,
        };
        if let Some(event) = event {
            let _ = app.emit(event, ());
        }
    }
}

fn model_label(model: &PluginModelInfo) -> String {
    let name = model.name.trim();
    if name.is_empty() {
        model.id.clone()
    } else {
        name.to_string()
    }
}

/// `[{id, name}]` — the shape OpenClaw, Pi, OMP, OMO Native and DSH store.
fn model_entries(models: &[PluginModelInfo]) -> Vec<Value> {
    models
        .iter()
        .map(|model| json!({ "id": model.id, "name": model_label(model) }))
        .collect()
}

/// `{id: {name}}` — OpenCode keys models by id.
fn opencode_model_map(models: &[PluginModelInfo]) -> Value {
    let mut map = Map::new();
    for model in models {
        map.insert(model.id.clone(), json!({ "name": model_label(model) }));
    }
    Value::Object(map)
}

/// `{id: {}}` — Hermes keys models by id and derives display data elsewhere.
fn hermes_model_map(models: &[PluginModelInfo]) -> Value {
    let mut map = Map::new();
    for model in models {
        map.insert(model.id.clone(), json!({}));
    }
    Value::Object(map)
}

/// Stable, filesystem/config-safe credential ref for DSH.
fn dsh_credential_ref(provider_id: &str) -> String {
    let mut out = String::from("AI_TOOLBOX_PLUGIN_");
    for character in provider_id.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    out
}

/// The origin a local CLI should call. The gateway only binds loopback, but a
/// persisted wildcard host is still mapped to loopback so the written config
/// never points at an address a CLI cannot reach.
fn local_gateway_origin(settings: &ProxyGatewaySettings) -> String {
    let host = match settings.listen_host.trim() {
        "" | "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        other => other,
    };
    crate::coding::proxy_gateway::listen::format_listen_base_url(host, settings.listen_port)
}

/// The real origin to write into a CLI config: the running gateway's actual
/// address when it is up (it may have auto-selected a different port), else
/// the configured one.
fn gateway_origin(
    settings: &ProxyGatewaySettings,
    gateway_state: Option<&ProxyGatewayState>,
) -> (String, bool) {
    if let Some(state) = gateway_state {
        if let Ok(manager) = state.manager.lock() {
            let status = manager.status();
            if status.running {
                if let Some(base_url) = status.base_url.filter(|url| !url.trim().is_empty()) {
                    return (base_url, true);
                }
            }
        }
    }
    (local_gateway_origin(settings), false)
}

/// `http://host:port/plugin/<provider_id>` — the root of the direct route.
fn plugin_root_url(origin: &str, provider_id: &str) -> String {
    format!(
        "{}/plugin/{}",
        origin.trim_end_matches('/'),
        provider_id.trim()
    )
}

/// `http://host:port/plugin/<provider_id>/v1` — the OpenAI-compatible base the
/// direct-route CLIs append `/chat/completions` to.
fn plugin_direct_base_url(origin: &str, provider_id: &str) -> String {
    format!("{}/v1", plugin_root_url(origin, provider_id))
}

/// The protocol the plugin provider actually speaks, from its npm package.
fn plugin_protocol(row: &PluginProviderRow) -> AiProtocol {
    npm_to_ai_protocol(&row.npm)
        .or_else(|| {
            row.models
                .iter()
                .find_map(|model| npm_to_ai_protocol(&model.npm))
        })
        .unwrap_or(AiProtocol::OpenAiChat)
}

fn filter_models(models: &[PluginModelInfo], model_ids: &[String]) -> Vec<PluginModelInfo> {
    if model_ids.is_empty() {
        return models.to_vec();
    }
    models
        .iter()
        .filter(|model| model_ids.iter().any(|id| id == &model.id))
        .cloned()
        .collect()
}

/// The plugin account a CLI provider was configured with, read back from the
/// stored `meta.pluginProvider` reference.
fn stored_plugin_account_ref(
    db: &SqliteDbState,
    cli: Cli,
    provider_id: &str,
) -> Option<PluginProviderRef> {
    let row = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(cli), provider_id))
        .ok()??;
    let plugin = row.get("meta")?.get("pluginProvider")?;
    serde_json::from_value(plugin.clone()).ok()
}

/// The account whose model list a single-account channel should use. The
/// pinned key wins; otherwise the router would start on the first enabled
/// account.
fn single_account_key(
    manager: &PluginManagerState,
    provider_id: &str,
    pinned: Option<String>,
) -> Option<String> {
    pinned
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .or_else(|| {
            gateway_bridge::enabled_accounts(manager, provider_id)
                .ok()
                .and_then(|accounts| accounts.into_iter().next())
        })
}

/// Build the stored `settings_config` for a gateway-takeover CLI.
///
/// The gateway ignores these endpoints for plugin-backed records, but a user
/// can still apply the record directly, so every endpoint points at the
/// plugin's own direct route instead of a fake upstream.
fn takeover_settings(
    cli_key: &str,
    plugin_origin: &str,
    endpoint: &str,
    models: &[PluginModelInfo],
    protocol: AiProtocol,
) -> Result<Value, String> {
    let first_model = models.first().map(|model| model.id.clone());
    match cli_key {
        "claudecode" | "claudedesktop" => {
            let mut env = Map::new();
            env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), json!(PLUGIN_API_KEY));
            env.insert("ANTHROPIC_BASE_URL".to_string(), json!(plugin_origin));
            if let Some(first) = &first_model {
                env.insert("ANTHROPIC_MODEL".to_string(), json!(first));
            }
            Ok(json!({ "env": env }))
        }
        "codex" => {
            let wire_api = match protocol {
                AiProtocol::OpenAiResponses => "responses",
                _ => "chat",
            };
            let model_line = first_model
                .as_deref()
                .map(|model| format!("model = \"{model}\"\n"))
                .unwrap_or_default();
            let config = format!(
                "{model_line}model_provider = \"custom\"\n\n[model_providers.custom]\nname = \"AI Toolbox Plugin\"\nbase_url = \"{endpoint}\"\nwire_api = \"{wire_api}\"\n"
            );
            let catalog = models
                .iter()
                .map(|model| json!({ "model": model.id, "displayName": model_label(model) }))
                .collect::<Vec<_>>();
            Ok(json!({
                "config": config,
                "auth": { "OPENAI_API_KEY": PLUGIN_API_KEY },
                "modelCatalog": { "models": catalog }
            }))
        }
        "grok" => {
            let catalog = models
                .iter()
                .map(|model| {
                    json!({
                        "key": model.id,
                        "model": model.id,
                        "displayName": model_label(model),
                        "baseUrl": endpoint,
                    })
                })
                .collect::<Vec<_>>();
            let mut value = json!({
                "auth": { "API_KEY": PLUGIN_API_KEY },
                "modelCatalog": { "models": catalog },
                "baseUrl": endpoint,
            });
            if let Some(first) = &first_model {
                value["defaultModelKey"] = json!(first);
            }
            Ok(value)
        }
        "kimi" => {
            let catalog = models
                .iter()
                .map(|model| {
                    json!({
                        "key": model.id,
                        "model": model.id,
                        "provider": "plugin",
                        "displayName": model_label(model),
                        "maxContextSize": 262_144,
                    })
                })
                .collect::<Vec<_>>();
            let mut value = json!({
                "auth": { "API_KEY": PLUGIN_API_KEY },
                "modelCatalog": { "models": catalog },
                "providerConfigs": {
                    "plugin": { "type": "openai_legacy", "base_url": endpoint }
                },
                "config": "",
            });
            if let Some(first) = &first_model {
                value["defaultModelKey"] = json!(first);
            }
            Ok(value)
        }
        "geminicli" => {
            let gemini_base = format!("{}/v1beta", plugin_origin);
            let mut env = Map::new();
            env.insert("GEMINI_API_KEY".to_string(), json!(PLUGIN_API_KEY));
            env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(gemini_base));
            if let Some(first) = &first_model {
                env.insert("GEMINI_MODEL".to_string(), json!(first));
            }
            Ok(json!({
                "env": env,
                "config": {
                    "security": { "auth": { "selectedType": "gemini-api-key" } }
                }
            }))
        }
        other => Err(format!("'{other}' is not a gateway-takeover CLI")),
    }
}

fn takeover_record(
    display_name: &str,
    plugin_provider: &PluginProviderRef,
    protocol: AiProtocol,
    settings: Value,
) -> Value {
    json!({
        "name": display_name,
        "category": "custom",
        "settings_config": serde_json::to_string(&settings)
            .unwrap_or_else(|_| "{}".to_string()),
        "meta": {
            "apiFormat": protocol.as_str(),
            "providerType": "custom",
            "pluginProvider": plugin_provider,
        },
        "is_applied": false,
        "is_disabled": false,
    })
}

fn plugin_ref_from_row(
    row: &PluginProviderRow,
    requested_spec: &str,
    account_strategy: PluginAccountStrategy,
    account_key: Option<String>,
) -> PluginProviderRef {
    let spec = requested_spec.trim();
    PluginProviderRef {
        plugin_spec: if spec.is_empty() {
            row.spec.clone()
        } else {
            spec.to_string()
        },
        provider_id: row.id.clone(),
        account_key: account_key.filter(|key| !key.trim().is_empty()),
        account_strategy,
    }
}

fn existing_plugin_record_id(
    conn: &rusqlite::Connection,
    table: DbTable,
    preferred_id: &str,
    plugin_provider_id: &str,
) -> Result<Option<Value>, String> {
    if let Some(found) = db_get(conn, table, preferred_id)? {
        return Ok(Some(found));
    }
    Ok(db_list(conn, table, None)?.into_iter().find(|row| {
        row.get("meta")
            .and_then(|meta| {
                meta.get("pluginProvider")
                    .or_else(|| meta.get("plugin_provider"))
            })
            .and_then(|plugin| {
                plugin
                    .get("providerId")
                    .or_else(|| plugin.get("provider_id"))
            })
            .and_then(Value::as_str)
            == Some(plugin_provider_id)
    }))
}

/// Insert or update one plugin-owned provider row, keeping the user's sort
/// position and applied/disabled state for an existing card.
fn upsert_plugin_provider_record(
    db: &SqliteDbState,
    cli: Cli,
    preferred_id: &str,
    plugin_provider_id: &str,
    mut data: Value,
) -> Result<(), String> {
    db.with_conn(|conn| {
        let table = DbTable::CliProvider(cli);
        let existing = existing_plugin_record_id(conn, table, preferred_id, plugin_provider_id)?;
        let record_id = match &existing {
            Some(row) => row
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(preferred_id)
                .to_string(),
            None => preferred_id.to_string(),
        };
        let data_object = data
            .as_object_mut()
            .ok_or_else(|| "Provider record must be a JSON object".to_string())?;
        match existing {
            Some(row) => {
                for key in [
                    "sort_index",
                    "is_applied",
                    "is_disabled",
                    "notes",
                    "website_url",
                    "icon",
                    "icon_color",
                ] {
                    if let Some(value) = row.get(key) {
                        data_object.insert(key.to_string(), value.clone());
                    }
                }
            }
            None => {
                let next_sort_index = db_list(conn, table, None)?
                    .iter()
                    .filter_map(|row| row.get("sort_index").and_then(Value::as_i64))
                    .max()
                    .map(|index| index + 1)
                    .unwrap_or(0);
                data_object.insert("sort_index".to_string(), json!(next_sort_index));
            }
        }
        db_put(conn, table, &record_id, &data)
    })
}

async fn upsert_plugin_file_provider(
    app: &AppHandle,
    db: &SqliteDbState,
    cli: FileCli,
    provider_id: &str,
    display_name: &str,
    endpoint: &str,
    models: &[PluginModelInfo],
) -> Result<(), String> {
    let provider_key = format!("plugin-{provider_id}");
    match cli {
        FileCli::OpenCode => {
            let config_path =
                crate::coding::open_code::commands::get_opencode_config_path_from_db(db)?;
            let provider = json!({
                "name": display_name,
                "npm": "@ai-sdk/openai-compatible",
                "options": { "baseURL": endpoint, "apiKey": PLUGIN_API_KEY },
                "models": opencode_model_map(models),
            });
            crate::coding::open_code::commands::upsert_opencode_plugin_provider(
                db,
                Path::new(&config_path),
                &provider_key,
                provider,
            )
            .await
        }
        FileCli::OpenClaw => {
            let config_path =
                crate::coding::open_claw::commands::get_openclaw_config_path_from_db(db)?;
            let backup_dir = crate::app_paths::resolved_data_dir()
                .join("backups")
                .join("openclaw");
            let provider = json!({
                "api": "openai-completions",
                "baseUrl": endpoint,
                "apiKey": PLUGIN_API_KEY,
                "models": model_entries(models),
            });
            crate::coding::open_claw::commands::upsert_openclaw_provider_at(
                Path::new(&config_path),
                &backup_dir,
                usize::MAX,
                &provider_key,
                provider,
            )
        }
        FileCli::Pi => {
            let models_path = crate::coding::pi::commands::get_pi_models_path_async(db).await?;
            crate::coding::pi::commands::upsert_pi_models_provider_at(
                &models_path,
                &provider_key,
                json!({
                    "api": "openai-completions",
                    "baseUrl": endpoint,
                    "apiKey": PLUGIN_API_KEY,
                    "models": model_entries(models),
                }),
            )
        }
        FileCli::OhMyPi => {
            let models_path =
                crate::coding::oh_my_pi::commands::get_omp_models_path_async(db).await?;
            crate::coding::oh_my_pi::commands::upsert_omp_models_provider_at(
                &models_path,
                &provider_key,
                json!({
                    "api": "openai-completions",
                    "baseUrl": endpoint,
                    "apiKey": PLUGIN_API_KEY,
                    "models": model_entries(models),
                }),
            )
        }
        FileCli::OmoNative => {
            let agent_dir =
                crate::coding::omo_native::providers::omo_native_agent_dir_from_db(db).await?;
            crate::coding::omo_native::providers::upsert_omo_native_provider_at(
                &agent_dir,
                &provider_key,
                json!({
                    "name": display_name,
                    "api": "openai-completions",
                    "baseUrl": endpoint,
                    "models": model_entries(models),
                }),
                Some(PLUGIN_API_KEY),
            )
        }
        FileCli::Hermes => {
            let config_path =
                crate::coding::hermes::commands::get_hermes_config_path_async(db).await?;
            crate::coding::hermes::commands::upsert_hermes_models_provider_at(
                app,
                &config_path,
                &provider_key,
                json!({
                    "display_name": display_name,
                    "api_mode": "openai",
                    "base_url": endpoint,
                    "api_key": PLUGIN_API_KEY,
                    "models": hermes_model_map(models),
                }),
            )
        }
        FileCli::Dsh => {
            let credential_ref = dsh_credential_ref(provider_id);
            crate::coding::dsh::commands::upsert_dsh_models_provider_at(
                db,
                &provider_key,
                json!({
                    "api": "openai-completions",
                    "baseURL": endpoint,
                    "apiKeyEnv": credential_ref,
                    "displayName": display_name,
                    "models": model_entries(models),
                }),
                Some(DshCredentialInput {
                    ref_name: credential_ref,
                    value: PLUGIN_API_KEY.to_string(),
                }),
            )
            .await
        }
        FileCli::Zcode => {
            upsert_zcode_provider(db, provider_id, display_name, endpoint, models).await
        }
        FileCli::Workbuddy => {
            upsert_workbuddy_family_models(db, GatewayCliKey::Workbuddy, endpoint, models).await
        }
        FileCli::Codebuddy => {
            upsert_workbuddy_family_models(db, GatewayCliKey::Codebuddy, endpoint, models).await
        }
    }
}

fn zcode_settings(
    provider_id: &str,
    display_name: &str,
    endpoint: &str,
    models: &[PluginModelInfo],
) -> Value {
    let rows = models
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let mut row = json!({
                "modelId": model.id,
                "ruleKind": "smart",
                "enabled": true,
                "isDefault": index == 0,
            });
            if model.name.trim() != model.id {
                row["displayName"] = json!(model_label(model));
            }
            row
        })
        .collect::<Vec<_>>();
    json!({
        "providerId": format!("custom:plugin-{provider_id}"),
        "providerName": display_name,
        "config": {
            "access": { "type": "api-key", "apiKey": PLUGIN_API_KEY },
            "api": { "type": "openai-chat-completions", "baseUrl": endpoint },
        },
        "models": rows,
        "defaultModelId": models.first().map(|model| model.id.clone()),
    })
}

async fn upsert_zcode_provider(
    db: &SqliteDbState,
    provider_id: &str,
    display_name: &str,
    endpoint: &str,
    models: &[PluginModelInfo],
) -> Result<(), String> {
    let path = crate::coding::zcode::commands::zcode_provider_config_path(db)?;
    let mut settings: ZcodeSettingsConfig =
        serde_json::from_value(zcode_settings(provider_id, display_name, endpoint, models))
            .map_err(|error| format!("Invalid ZCode plugin provider config: {error}"))?;
    crate::coding::zcode::commands::project_zcode_provider_at(&path, &mut settings).await?;

    let record = json!({
        "name": display_name,
        "category": "custom",
        "settings_config": serde_json::to_string(&settings)
            .map_err(|error| format!("Failed to serialize ZCode provider settings: {error}"))?,
        "meta": {
            "providerType": "custom",
            "pluginProvider": {
                "providerId": provider_id,
                "pluginSpec": "",
                "accountStrategy": "auto_rotate",
            },
        },
        "is_applied": false,
        "is_disabled": false,
    });
    upsert_plugin_provider_record(
        db,
        Cli::Zcode,
        &format!("plugin-{provider_id}"),
        provider_id,
        record,
    )
}

async fn workbuddy_family_root(db: &SqliteDbState, cli_key: GatewayCliKey) -> std::path::PathBuf {
    match cli_key {
        GatewayCliKey::Codebuddy => {
            crate::coding::codebuddy::commands::config_root_from_db(db)
                .await
                .0
        }
        _ => {
            crate::coding::workbuddy::commands::config_root_from_db(db)
                .await
                .0
        }
    }
}

async fn upsert_workbuddy_family_models(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    endpoint: &str,
    models: &[PluginModelInfo],
) -> Result<(), String> {
    crate::coding::workbuddy_family::ensure_models_file_direct(cli_key)?;
    let root = workbuddy_family_root(db, cli_key).await;
    let mut document = ModelsDocument::load(&root)?;
    // Drop models the plugin no longer exposes from this exact channel, so a
    // refresh mirrors the catalog instead of accumulating stale entries.
    let channel_base_url = normalize_channel_base_url(endpoint);
    for entry in document.models_for_channel(&channel_base_url, PLUGIN_API_KEY) {
        if let Some(model_id) = entry_id(&entry) {
            if !models.iter().any(|model| model.id == model_id) {
                document.remove_model(&model_id);
            }
        }
    }
    apply_workbuddy_models(&mut document, endpoint, models);
    document.save(&root)
}

/// Insert every selected plugin model into one WorkBuddy-family
/// `models.json`, merging onto existing entries so product-owned keys survive.
fn apply_workbuddy_models(
    document: &mut ModelsDocument,
    endpoint: &str,
    models: &[PluginModelInfo],
) {
    for model in models {
        let label = model_label(model);
        let entry = build_model_entry(&ModelEntryInput {
            id: &model.id,
            name: Some(&label),
            vendor: Some("AI Toolbox Plugin"),
            base_url: endpoint,
            api_key: Some(PLUGIN_API_KEY),
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
            supported_efforts: None,
        });
        document.upsert_model_renaming(entry, None);
    }
}

#[tauri::command]
pub async fn plugins_add_provider_to_cli(
    app: AppHandle,
    db: State<'_, SqliteDbState>,
    manager: State<'_, PluginManagerState>,
    gateway: State<'_, ProxyGatewayState>,
    input: AddPluginProviderToCliInput,
) -> Result<AddPluginProviderToCliResult, String> {
    let cli_key = normalize_cli_key(&input.cli_key)?;
    if cli_key == "antigravity" {
        return Err(ANTIGRAVITY_REASON.to_string());
    }
    let rows = gateway_bridge::list_plugin_provider_rows(manager.inner(), false).await?;
    let row = gateway_bridge::find_plugin_provider_row(
        &rows,
        Some(&input.plugin_spec),
        &input.provider_id,
    )?;
    let account_key = match input.account_strategy {
        PluginAccountStrategy::Single => {
            single_account_key(manager.inner(), &row.id, input.account_key.clone())
        }
        PluginAccountStrategy::AutoRotate => None,
    };
    let models =
        gateway_bridge::models_for_cli(row, input.account_strategy, account_key.as_deref());
    let models = filter_models(&models, &input.model_ids);
    if !input.model_ids.is_empty() && models.is_empty() {
        return Err(format!(
            "None of the selected models are offered by plugin provider '{}'",
            row.id
        ));
    }

    let display_name = input
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let name = row.name.trim();
            if name.is_empty() {
                row.id.clone()
            } else {
                name.to_string()
            }
        });
    let plugin_provider = plugin_ref_from_row(
        row,
        &input.plugin_spec,
        input.account_strategy,
        input.account_key,
    );

    let settings = load_settings_from_sqlite_state(&db)?;
    let (origin, gateway_running) = gateway_origin(&settings, Some(&gateway));
    let plugin_origin = plugin_root_url(&origin, &row.id);
    let endpoint = plugin_direct_base_url(&origin, &row.id);

    let mode = if let Some(cli) = takeover_cli(&cli_key) {
        let protocol = plugin_protocol(row);
        let settings_config =
            takeover_settings(&cli_key, &plugin_origin, &endpoint, &models, protocol)?;
        let record = takeover_record(&display_name, &plugin_provider, protocol, settings_config);
        upsert_plugin_provider_record(&db, cli, &format!("plugin-{}", row.id), &row.id, record)?;
        "gateway_takeover"
    } else {
        let cli = file_cli(&cli_key)
            .ok_or_else(|| format!("'{cli_key}' has no plugin integration path"))?;
        upsert_plugin_file_provider(&app, &db, cli, &row.id, &display_name, &endpoint, &models)
            .await?;
        emit_file_cli_sync_events(&app, cli);
        "direct_plugin_route"
    };
    let _ = app.emit("config-changed", "window");

    Ok(AddPluginProviderToCliResult {
        cli_key,
        provider_id: row.id.clone(),
        provider_name: display_name,
        mode: mode.to_string(),
        endpoint_url: endpoint,
        model_count: models.len(),
        gateway_running,
    })
}

fn model_ids_from_value(value: &Value) -> Vec<String> {
    if let Some(models) = value.as_object() {
        return models.keys().cloned().collect();
    }
    value
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    row.get("id")
                        .or_else(|| row.get("modelId"))
                        .or_else(|| row.get("model"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn patch_entry_models(cli: FileCli, entry: &mut Value, models: &[PluginModelInfo]) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let models_value = match cli {
        FileCli::OpenCode => opencode_model_map(models),
        FileCli::Hermes => hermes_model_map(models),
        FileCli::OpenClaw | FileCli::Pi | FileCli::OhMyPi | FileCli::OmoNative | FileCli::Dsh => {
            Value::Array(model_entries(models))
        }
        FileCli::Zcode | FileCli::Workbuddy | FileCli::Codebuddy => return,
    };
    object.insert("models".to_string(), models_value);
}

async fn sync_file_provider_models(
    app: &AppHandle,
    db: &SqliteDbState,
    cli: FileCli,
    cli_provider_id: &str,
    plugin_provider_id: &str,
    origin: &str,
    models: &[PluginModelInfo],
) -> Result<Vec<String>, String> {
    let provider_key = if cli_provider_id.trim().is_empty() {
        format!("plugin-{plugin_provider_id}")
    } else {
        cli_provider_id.trim().to_string()
    };
    match cli {
        FileCli::OpenCode => {
            let path = Path::new(
                &crate::coding::open_code::commands::get_opencode_config_path_from_db(db)?,
            )
            .to_path_buf();
            let mut entry = crate::coding::open_code::commands::read_opencode_provider_at(
                &path,
                &provider_key,
            )?
            .ok_or_else(|| format!("OpenCode provider '{provider_key}' was not found"))?;
            let old = model_ids_from_value(entry.get("models").unwrap_or(&Value::Null));
            patch_entry_models(cli, &mut entry, models);
            crate::coding::open_code::commands::upsert_opencode_plugin_provider(
                db,
                &path,
                &provider_key,
                entry,
            )
            .await?;
            Ok(old)
        }
        FileCli::OpenClaw => {
            let config_path =
                crate::coding::open_claw::commands::get_openclaw_config_path_from_db(db)?;
            let path = Path::new(&config_path);
            let mut entry =
                crate::coding::open_claw::commands::read_openclaw_provider_at(path, &provider_key)?
                    .ok_or_else(|| format!("OpenClaw provider '{provider_key}' was not found"))?;
            let old = model_ids_from_value(entry.get("models").unwrap_or(&Value::Null));
            patch_entry_models(cli, &mut entry, models);
            let backup_dir = crate::app_paths::resolved_data_dir()
                .join("backups")
                .join("openclaw");
            crate::coding::open_claw::commands::upsert_openclaw_provider_at(
                path,
                &backup_dir,
                usize::MAX,
                &provider_key,
                entry,
            )?;
            Ok(old)
        }
        FileCli::Pi => {
            let path = crate::coding::pi::commands::get_pi_models_path_async(db).await?;
            let mut entry =
                crate::coding::pi::commands::read_pi_models_provider_at(&path, &provider_key)?
                    .ok_or_else(|| format!("Pi provider '{provider_key}' was not found"))?;
            let old = model_ids_from_value(entry.get("models").unwrap_or(&Value::Null));
            patch_entry_models(cli, &mut entry, models);
            crate::coding::pi::commands::upsert_pi_models_provider_at(&path, &provider_key, entry)?;
            Ok(old)
        }
        FileCli::OhMyPi => {
            let path = crate::coding::oh_my_pi::commands::get_omp_models_path_async(db).await?;
            let mut entry = crate::coding::oh_my_pi::commands::read_omp_models_provider_at(
                &path,
                &provider_key,
            )?
            .ok_or_else(|| format!("OMP provider '{provider_key}' was not found"))?;
            let old = model_ids_from_value(entry.get("models").unwrap_or(&Value::Null));
            patch_entry_models(cli, &mut entry, models);
            crate::coding::oh_my_pi::commands::upsert_omp_models_provider_at(
                &path,
                &provider_key,
                entry,
            )?;
            Ok(old)
        }
        FileCli::OmoNative => {
            let agent_dir =
                crate::coding::omo_native::providers::omo_native_agent_dir_from_db(db).await?;
            let mut entry = crate::coding::omo_native::providers::read_omo_native_provider_at(
                &agent_dir,
                &provider_key,
            )?
            .ok_or_else(|| format!("OMO Native provider '{provider_key}' was not found"))?;
            let old = model_ids_from_value(entry.get("models").unwrap_or(&Value::Null));
            patch_entry_models(cli, &mut entry, models);
            crate::coding::omo_native::providers::upsert_omo_native_provider_at(
                &agent_dir,
                &provider_key,
                entry,
                None,
            )?;
            Ok(old)
        }
        FileCli::Hermes => {
            let config_path =
                crate::coding::hermes::commands::get_hermes_config_path_async(db).await?;
            let mut entry = crate::coding::hermes::commands::read_hermes_models_provider_at(
                &config_path,
                &provider_key,
            )?
            .ok_or_else(|| format!("Hermes provider '{provider_key}' was not found"))?;
            let old = model_ids_from_value(entry.get("models").unwrap_or(&Value::Null));
            patch_entry_models(cli, &mut entry, models);
            crate::coding::hermes::commands::upsert_hermes_models_provider_at(
                app,
                &config_path,
                &provider_key,
                entry,
            )?;
            Ok(old)
        }
        FileCli::Dsh => {
            let mut entry =
                crate::coding::dsh::commands::read_dsh_models_provider_at(db, &provider_key)
                    .await?
                    .ok_or_else(|| format!("DSH provider '{provider_key}' was not found"))?;
            let old = model_ids_from_value(entry.get("models").unwrap_or(&Value::Null));
            patch_entry_models(cli, &mut entry, models);
            crate::coding::dsh::commands::upsert_dsh_models_provider_at(
                db,
                &provider_key,
                entry,
                None,
            )
            .await?;
            Ok(old)
        }
        FileCli::Workbuddy | FileCli::Codebuddy => {
            let cli_key = if cli == FileCli::Codebuddy {
                GatewayCliKey::Codebuddy
            } else {
                GatewayCliKey::Workbuddy
            };
            let endpoint = plugin_direct_base_url(origin, plugin_provider_id);
            let root = workbuddy_family_root(db, cli_key).await;
            let old = ModelsDocument::load(&root)?
                .models_for_channel(&normalize_channel_base_url(&endpoint), PLUGIN_API_KEY)
                .iter()
                .filter_map(entry_id)
                .collect();
            upsert_workbuddy_family_models(db, cli_key, &endpoint, models).await?;
            Ok(old)
        }
        FileCli::Zcode => Ok(Vec::new()),
    }
}

/// Replace a takeover record's model list in place: names, sort index, notes
/// and every other user field survive, only the catalog/default model changes.
fn patch_takeover_settings(
    cli_key: &str,
    settings: &mut Value,
    models: &[PluginModelInfo],
) -> Vec<String> {
    let old = match cli_key {
        "claudecode" | "claudedesktop" => settings
            .get("env")
            .and_then(|env| env.get("ANTHROPIC_MODEL"))
            .and_then(Value::as_str)
            .map(|model| vec![model.to_string()])
            .unwrap_or_default(),
        "codex" | "grok" | "kimi" => settings
            .get("modelCatalog")
            .and_then(|catalog| catalog.get("models"))
            .map(model_ids_from_value)
            .unwrap_or_default(),
        "geminicli" => settings
            .get("env")
            .and_then(|env| env.get("GEMINI_MODEL"))
            .and_then(Value::as_str)
            .map(|model| vec![model.to_string()])
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let first_model = models.first().map(|model| model.id.clone());
    match cli_key {
        "claudecode" | "claudedesktop" => {
            if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
                match &first_model {
                    Some(first) => {
                        env.insert("ANTHROPIC_MODEL".to_string(), json!(first));
                    }
                    None => {
                        env.remove("ANTHROPIC_MODEL");
                    }
                }
            }
        }
        "codex" => {
            let catalog = models
                .iter()
                .map(|model| json!({ "model": model.id, "displayName": model_label(model) }))
                .collect::<Vec<_>>();
            settings["modelCatalog"] = json!({ "models": catalog });
        }
        "grok" => {
            let catalog = models
                .iter()
                .map(|model| {
                    json!({
                        "key": model.id,
                        "model": model.id,
                        "displayName": model_label(model),
                    })
                })
                .collect::<Vec<_>>();
            settings["modelCatalog"] = json!({ "models": catalog });
            if let Some(first) = first_model {
                settings["defaultModelKey"] = json!(first);
            }
        }
        "kimi" => {
            let catalog = models
                .iter()
                .map(|model| {
                    json!({
                        "key": model.id,
                        "model": model.id,
                        "provider": "plugin",
                        "displayName": model_label(model),
                        "maxContextSize": 262_144,
                    })
                })
                .collect::<Vec<_>>();
            settings["modelCatalog"] = json!({ "models": catalog });
            if let Some(first) = first_model {
                settings["defaultModelKey"] = json!(first);
            }
        }
        "geminicli" => {
            if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
                match &first_model {
                    Some(first) => {
                        env.insert("GEMINI_MODEL".to_string(), json!(first));
                    }
                    None => {
                        env.remove("GEMINI_MODEL");
                    }
                }
            }
        }
        _ => {}
    }
    old
}

async fn sync_takeover_models(
    db: &SqliteDbState,
    cli: Cli,
    cli_provider_id: &str,
    models: &[PluginModelInfo],
) -> Result<Vec<String>, String> {
    let row = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(cli), cli_provider_id))?
        .ok_or_else(|| format!("Provider '{cli_provider_id}' was not found"))?;
    let settings_text = row
        .get("settings_config")
        .and_then(Value::as_str)
        .ok_or_else(|| "Provider has no settings_config".to_string())?;
    let mut settings: Value = serde_json::from_str(settings_text)
        .map_err(|error| format!("Invalid provider settings_config: {error}"))?;
    let cli_key = match cli {
        Cli::Claude => "claudecode",
        Cli::ClaudeDesktop => "claudedesktop",
        Cli::Codex => "codex",
        Cli::Grok => "grok",
        Cli::Kimi => "kimi",
        Cli::Gemini => "geminicli",
        _ => return Err("Not a gateway-takeover CLI".to_string()),
    };
    let old = patch_takeover_settings(cli_key, &mut settings, models);

    let mut data = row.clone();
    let data_object = data
        .as_object_mut()
        .ok_or_else(|| "Provider row must be a JSON object".to_string())?;
    data_object.insert(
        "settings_config".to_string(),
        json!(serde_json::to_string(&settings)
            .map_err(|error| format!("Failed to serialize provider settings: {error}"))?),
    );
    data_object.remove("id");
    data_object.remove("created_at");
    data_object.remove("updated_at");
    db.with_conn(|conn| db_put(conn, DbTable::CliProvider(cli), cli_provider_id, &data))?;
    Ok(old)
}

async fn sync_zcode_models(
    db: &SqliteDbState,
    cli_provider_id: &str,
    models: &[PluginModelInfo],
) -> Result<Vec<String>, String> {
    let mut settings = read_zcode_settings(db, cli_provider_id)?;
    let old = settings
        .models
        .iter()
        .map(|row| row.model_id.clone())
        .collect::<Vec<_>>();
    settings.models = models
        .iter()
        .enumerate()
        .map(
            |(index, model)| crate::coding::zcode::types::ZcodeModelRow {
                model_id: model.id.clone(),
                display_name: (model.name.trim() != model.id).then(|| model_label(model)),
                rule_kind: crate::coding::zcode::types::ZcodeModelRuleKind::Smart,
                enabled: Some(true),
                properties: None,
                option_specs: None,
                is_default: index == 0,
            },
        )
        .collect();
    settings.default_model_id = models.first().map(|model| model.id.clone());
    let path = crate::coding::zcode::commands::zcode_provider_config_path(db)?;
    crate::coding::zcode::commands::project_zcode_provider_at(&path, &mut settings).await?;

    let row = read_zcode_row(db, cli_provider_id)?;
    let row_id = row
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "ZCode provider row has no id".to_string())?
        .to_string();
    let mut data = row;
    let data_object = data
        .as_object_mut()
        .ok_or_else(|| "ZCode provider row must be a JSON object".to_string())?;
    data_object.insert(
        "settings_config".to_string(),
        json!(serde_json::to_string(&settings)
            .map_err(|error| format!("Failed to serialize ZCode settings: {error}"))?),
    );
    data_object.remove("created_at");
    data_object.remove("updated_at");
    db.with_conn(|conn| db_put(conn, DbTable::CliProvider(Cli::Zcode), &row_id, &data))?;
    Ok(old)
}

fn read_zcode_row(db: &SqliteDbState, cli_provider_id: &str) -> Result<Value, String> {
    db.with_conn(|conn| {
        if let Some(row) = db_get(conn, DbTable::CliProvider(Cli::Zcode), cli_provider_id)? {
            return Ok(Some(row));
        }
        Ok(db_list(conn, DbTable::CliProvider(Cli::Zcode), None)?
            .into_iter()
            .find(|row| {
                row.get("settings_config")
                    .and_then(Value::as_str)
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .and_then(|settings| {
                        settings
                            .get("providerId")
                            .or_else(|| settings.get("provider_id"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .as_deref()
                    == Some(cli_provider_id)
            }))
    })?
    .ok_or_else(|| format!("ZCode provider '{cli_provider_id}' was not found"))
}

fn read_zcode_settings(
    db: &SqliteDbState,
    cli_provider_id: &str,
) -> Result<ZcodeSettingsConfig, String> {
    let row = read_zcode_row(db, cli_provider_id)?;
    let settings_text = row
        .get("settings_config")
        .and_then(Value::as_str)
        .ok_or_else(|| "ZCode provider has no settings_config".to_string())?;
    serde_json::from_str(settings_text)
        .map_err(|error| format!("Invalid ZCode provider settings: {error}"))
}

fn count_model_changes(old: &[String], models: &[PluginModelInfo]) -> (usize, usize) {
    let new_ids = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    let added = new_ids
        .iter()
        .filter(|id| !old.iter().any(|old_id| old_id == *id))
        .count();
    let updated = new_ids
        .iter()
        .filter(|id| old.iter().any(|old_id| old_id == *id))
        .count();
    (added, updated)
}

#[tauri::command]
pub async fn plugins_sync_cli_provider_models(
    app: AppHandle,
    db: State<'_, SqliteDbState>,
    manager: State<'_, PluginManagerState>,
    gateway: State<'_, ProxyGatewayState>,
    cli_key: String,
    cli_provider_id: String,
    plugin_provider_id: String,
) -> Result<SyncPluginModelsResult, String> {
    let cli_key = normalize_cli_key(&cli_key)?;
    if cli_key == "antigravity" {
        return Err(ANTIGRAVITY_REASON.to_string());
    }
    let rows = gateway_bridge::list_plugin_provider_rows(manager.inner(), true).await?;
    let row = rows
        .iter()
        .find(|row| row.id == plugin_provider_id)
        .ok_or_else(|| {
            format!("Plugin provider '{plugin_provider_id}' is not offered by any installed plugin")
        })?;

    let settings = load_settings_from_sqlite_state(&db)?;
    let (origin, _) = gateway_origin(&settings, Some(&gateway));

    let (old, models) = if let Some(cli) = takeover_cli(&cli_key) {
        let provider_id = if cli_provider_id.trim().is_empty() {
            format!("plugin-{}", row.id)
        } else {
            cli_provider_id.trim().to_string()
        };
        let stored = stored_plugin_account_ref(&db, cli, &provider_id);
        let strategy = stored
            .as_ref()
            .map(|reference| reference.account_strategy)
            .unwrap_or_default();
        let account_key = match strategy {
            PluginAccountStrategy::Single => single_account_key(
                manager.inner(),
                &row.id,
                stored.and_then(|reference| reference.account_key),
            ),
            PluginAccountStrategy::AutoRotate => None,
        };
        let models = gateway_bridge::models_for_cli(row, strategy, account_key.as_deref());
        let old = sync_takeover_models(&db, cli, &provider_id, &models).await?;
        (old, models)
    } else {
        let cli = file_cli(&cli_key)
            .ok_or_else(|| format!("'{cli_key}' has no plugin integration path"))?;
        let models = gateway_bridge::models_for_cli(row, PluginAccountStrategy::AutoRotate, None);
        let old = if cli == FileCli::Zcode {
            sync_zcode_models(&db, &cli_provider_id, &models).await?
        } else {
            sync_file_provider_models(&app, &db, cli, &cli_provider_id, &row.id, &origin, &models)
                .await?
        };
        emit_file_cli_sync_events(&app, cli);
        (old, models)
    };
    let _ = app.emit("config-changed", "window");

    let (added, updated) = count_model_changes(&old, &models);
    Ok(SyncPluginModelsResult {
        cli_key,
        cli_provider_id,
        model_count: models.len(),
        added,
        updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_coding_tab_is_listed_once() {
        let targets = cli_targets();
        let keys = targets
            .iter()
            .map(|target| target.cli_key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "claudecode",
                "claudedesktop",
                "codex",
                "grok",
                "kimi",
                "geminicli",
                "antigravity",
                "zcode",
                "workbuddy",
                "codebuddy",
                "opencode",
                "openclaw",
                "pi",
                "oh_my_pi",
                "omo_native",
                "hermes",
                "dsh",
            ]
        );
        for target in &targets {
            assert!(matches!(
                target.mode.as_str(),
                "gateway_takeover" | "direct_plugin_route"
            ));
        }
        let antigravity = targets
            .iter()
            .find(|target| target.cli_key == "antigravity")
            .unwrap();
        assert!(!antigravity.supported);
        assert!(antigravity.reason.is_some());
        assert!(targets
            .iter()
            .filter(|target| target.cli_key != "antigravity")
            .all(|target| target.supported && target.reason.is_none()));
    }

    #[test]
    fn wildcard_listen_host_is_written_as_loopback() {
        let settings = ProxyGatewaySettings {
            listen_host: "0.0.0.0".to_string(),
            listen_port: 37123,
            ..Default::default()
        };
        assert_eq!(local_gateway_origin(&settings), "http://127.0.0.1:37123");
        assert_eq!(
            plugin_direct_base_url("http://127.0.0.1:37123", "demo"),
            "http://127.0.0.1:37123/plugin/demo/v1"
        );
        assert_eq!(
            plugin_root_url("http://127.0.0.1:37123", "demo"),
            "http://127.0.0.1:37123/plugin/demo"
        );
    }

    fn sample_models() -> Vec<PluginModelInfo> {
        vec![
            PluginModelInfo {
                id: "alpha-model".to_string(),
                name: "Alpha Model".to_string(),
                npm: "@ai-sdk/openai-compatible".to_string(),
                ..PluginModelInfo::default()
            },
            PluginModelInfo {
                id: "beta-model".to_string(),
                name: String::new(),
                npm: String::new(),
                ..PluginModelInfo::default()
            },
        ]
    }

    #[test]
    fn takeover_settings_cover_every_gateway_cli() {
        let models = sample_models();
        for (cli_key, cli) in [
            ("claudecode", Cli::Claude),
            ("claudedesktop", Cli::ClaudeDesktop),
            ("codex", Cli::Codex),
            ("grok", Cli::Grok),
            ("kimi", Cli::Kimi),
            ("geminicli", Cli::Gemini),
        ] {
            assert_eq!(takeover_cli(cli_key), Some(cli));
            let settings = takeover_settings(
                cli_key,
                "http://127.0.0.1:37123/plugin/demo",
                "http://127.0.0.1:37123/plugin/demo/v1",
                &models,
                AiProtocol::OpenAiChat,
            )
            .expect("takeover settings");
            assert!(settings.is_object(), "{cli_key} settings must be an object");
            if cli_key == "codex" {
                let config = settings["config"].as_str().expect("codex config");
                assert!(
                    config.contains(r#"model_provider = "custom""#),
                    "codex takeover must reuse the shared custom provider id: {config}"
                );
                assert!(
                    config.contains("[model_providers.custom]"),
                    "codex takeover must define the shared custom provider table: {config}"
                );
                assert!(
                    !config.contains("[model_providers.plugin]"),
                    "codex takeover must not introduce a plugin provider id: {config}"
                );
            }
        }
    }

    #[test]
    fn takeover_record_round_trips_plugin_metadata() {
        let provider_ref = PluginProviderRef {
            plugin_spec: "@acme/plugin".to_string(),
            provider_id: "demo".to_string(),
            account_key: None,
            account_strategy: PluginAccountStrategy::Single,
        };
        let record = takeover_record(
            "Demo",
            &provider_ref,
            AiProtocol::AnthropicMessages,
            json!({ "env": { "ANTHROPIC_MODEL": "alpha-model" } }),
        );
        assert_eq!(record["category"], "custom");
        assert_eq!(record["meta"]["apiFormat"], "anthropic_messages");
        assert_eq!(record["meta"]["pluginProvider"]["providerId"], "demo");
        assert_eq!(
            record["meta"]["pluginProvider"]["accountStrategy"],
            "single"
        );
        let settings: Value =
            serde_json::from_str(record["settings_config"].as_str().unwrap()).unwrap();
        assert_eq!(settings["env"]["ANTHROPIC_MODEL"], "alpha-model");
    }

    #[test]
    fn patching_takeover_models_keeps_the_first_default() {
        let models = sample_models();
        let mut settings = json!({
            "modelCatalog": { "models": [{ "model": "alpha-model" }] },
            "defaultModelKey": "alpha-model",
        });
        let old = patch_takeover_settings("kimi", &mut settings, &models);
        assert_eq!(old, vec!["alpha-model".to_string()]);
        assert_eq!(settings["defaultModelKey"], "alpha-model");
        assert_eq!(
            settings["modelCatalog"]["models"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            settings["modelCatalog"]["models"][1]["displayName"],
            "beta-model"
        );
    }

    #[test]
    fn model_change_counts_split_added_and_updated() {
        let models = sample_models();
        let old = vec!["alpha-model".to_string(), "retired-model".to_string()];
        assert_eq!(count_model_changes(&old, &models), (1, 1));
    }

    #[test]
    fn file_clients_cover_every_direct_route_tab() {
        for cli_key in [
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
        ] {
            assert!(
                file_cli(cli_key).is_some(),
                "{cli_key} must map to a file CLI"
            );
        }
        assert!(normalize_cli_key("unknown").is_err());
    }

    fn sample_provider_ref() -> PluginProviderRef {
        PluginProviderRef {
            plugin_spec: "@acme/plugin".to_string(),
            provider_id: "demo".to_string(),
            account_key: None,
            account_strategy: PluginAccountStrategy::AutoRotate,
        }
    }

    #[test]
    fn takeover_records_load_as_plugin_providers() {
        use crate::coding::proxy_gateway::types::GatewayCliKey;

        let models = sample_models();
        for (cli_key, gateway_key) in [
            ("claudecode", GatewayCliKey::Claude),
            ("codex", GatewayCliKey::Codex),
            ("kimi", GatewayCliKey::Kimi),
            ("geminicli", GatewayCliKey::Gemini),
        ] {
            let settings = takeover_settings(
                cli_key,
                "http://127.0.0.1:37123/plugin/demo",
                "http://127.0.0.1:37123/plugin/demo/v1",
                &models,
                AiProtocol::OpenAiChat,
            )
            .expect("takeover settings");
            let mut record = takeover_record(
                "Demo",
                &sample_provider_ref(),
                AiProtocol::OpenAiChat,
                settings,
            );
            record["id"] = json!("plugin-demo");

            let provider = crate::coding::proxy_gateway::provider_from_record_for_test(
                gateway_key,
                record.clone(),
            )
            .expect("record parses")
            .expect("plugin record is enabled");
            let plugin = provider
                .plugin_provider
                .as_ref()
                .unwrap_or_else(|| panic!("{cli_key} must load as a plugin provider"));
            assert_eq!(plugin.provider_id, "demo");
            assert_eq!(provider.target_protocol, AiProtocol::OpenAiChat);

            let settings: Value =
                serde_json::from_str(record["settings_config"].as_str().unwrap()).unwrap();
            match cli_key {
                "claudecode" => assert_eq!(settings["env"]["ANTHROPIC_MODEL"], "alpha-model"),
                "codex" | "kimi" => {
                    let catalog = settings["modelCatalog"]["models"].as_array().unwrap();
                    assert_eq!(catalog.len(), 2, "{cli_key} keeps every selected model");
                }
                "geminicli" => assert_eq!(settings["env"]["GEMINI_MODEL"], "alpha-model"),
                _ => {}
            }
        }
    }

    #[test]
    fn opencode_provider_write_preserves_other_providers_and_unknown_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("opencode.json");
        std::fs::write(
            &path,
            r#"{
                "$schema": "https://opencode.ai/config.json",
                "unknownSection": { "keep": true },
                "provider": {
                    "keep": {
                        "npm": "@ai-sdk/openai-compatible",
                        "options": { "baseURL": "https://keep.test/v1", "apiKey": "sk-keep" },
                        "models": { "keep-model": { "name": "Keep Model" } }
                    }
                }
            }"#,
        )
        .unwrap();

        crate::coding::open_code::commands::upsert_opencode_provider_at(
            &path,
            "plugin-demo",
            json!({
                "name": "Demo",
                "npm": "@ai-sdk/openai-compatible",
                "options": {
                    "baseURL": "http://127.0.0.1:37123/plugin/demo/v1",
                    "apiKey": PLUGIN_API_KEY,
                },
                "models": opencode_model_map(&sample_models()),
            }),
        )
        .expect("write OpenCode config");

        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["unknownSection"]["keep"], true);
        assert_eq!(
            after["provider"]["keep"]["options"]["baseURL"],
            "https://keep.test/v1"
        );
        assert_eq!(
            after["provider"]["plugin-demo"]["options"]["baseURL"],
            "http://127.0.0.1:37123/plugin/demo/v1"
        );
        assert!(after["provider"]["plugin-demo"]["models"]["alpha-model"].is_object());
    }

    #[test]
    fn pi_provider_write_preserves_other_providers_and_unknown_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("models.json");
        std::fs::write(
            &path,
            r#"{
                "unknownTopLevel": 7,
                "providers": {
                    "keep": {
                        "api": "openai-completions",
                        "baseUrl": "https://keep.test/v1",
                        "apiKey": "sk-keep",
                        "models": [{ "id": "keep-model" }]
                    }
                }
            }"#,
        )
        .unwrap();

        crate::coding::pi::commands::upsert_pi_models_provider_at(
            &path,
            "plugin-demo",
            json!({
                "api": "openai-completions",
                "baseUrl": "http://127.0.0.1:37123/plugin/demo/v1",
                "apiKey": PLUGIN_API_KEY,
                "models": model_entries(&sample_models()),
            }),
        )
        .expect("write Pi models");

        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["unknownTopLevel"], 7);
        assert_eq!(after["providers"]["keep"]["apiKey"], "sk-keep");
        assert_eq!(
            after["providers"]["plugin-demo"]["baseUrl"],
            "http://127.0.0.1:37123/plugin/demo/v1"
        );
        assert_eq!(
            after["providers"]["plugin-demo"]["models"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn hermes_provider_write_preserves_other_sections_and_providers() {
        let app = tauri::test::mock_app();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "model: keep-model\n# user comment\ncustom_providers:\n  - name: keep\n    display_name: Keep\n    api_mode: openai\n    base_url: https://keep.test/v1\n    api_key: sk-keep\n    models:\n      keep-model: {}\nother_section:\n  keep: true\n",
        )
        .unwrap();

        crate::coding::hermes::commands::upsert_hermes_models_provider_at(
            app.handle(),
            &path,
            "plugin-demo",
            json!({
                "display_name": "Demo",
                "api_mode": "openai",
                "base_url": "http://127.0.0.1:37123/plugin/demo/v1",
                "api_key": PLUGIN_API_KEY,
                "models": hermes_model_map(&sample_models()),
            }),
        )
        .expect("write Hermes config");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# user comment"), "comments must survive");
        assert!(after.contains("other_section:"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&after).unwrap();
        let parsed: Value = serde_json::to_value(parsed).unwrap();
        let providers = parsed["custom_providers"].as_array().unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0]["name"], "keep");
        assert_eq!(providers[1]["name"], "plugin-demo");
        assert_eq!(providers[1]["api_key"], PLUGIN_API_KEY);
        assert!(providers[1]["models"]["alpha-model"].is_object());
    }

    #[test]
    fn workbuddy_models_write_preserves_other_entries_and_top_level_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join("models.json"),
            r#"{
                "version": 2,
                "models": [
                    {
                        "id": "keep-model",
                        "url": "https://keep.test/v1/chat/completions",
                        "apiKey": "sk-keep"
                    }
                ]
            }"#,
        )
        .unwrap();

        let mut document = ModelsDocument::load(root).expect("load models.json");
        apply_workbuddy_models(
            &mut document,
            "http://127.0.0.1:37123/plugin/demo/v1",
            &sample_models(),
        );
        document.save(root).expect("save models.json");

        let after: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("models.json")).unwrap())
                .unwrap();
        assert_eq!(after["version"], 2);
        let models = after["models"].as_array().unwrap();
        assert!(models.iter().any(|entry| entry["id"] == "keep-model"));
        let plugin_entry = models
            .iter()
            .find(|entry| entry["id"] == "alpha-model")
            .expect("plugin model entry");
        assert_eq!(plugin_entry["apiKey"], PLUGIN_API_KEY);
        let url = plugin_entry["url"].as_str().unwrap();
        assert!(
            url.contains("/plugin/demo/v1"),
            "endpoint must point at the plugin route: {url}"
        );
    }

    #[test]
    fn pi_model_sync_updates_models_and_keeps_custom_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("models.json");
        std::fs::write(
            &path,
            r#"{
                "providers": {
                    "plugin-demo": {
                        "api": "openai-completions",
                        "baseUrl": "http://127.0.0.1:37123/plugin/demo/v1",
                        "apiKey": "ai-toolbox-plugin",
                        "displayName": "User Rename",
                        "customFlag": true,
                        "models": [{ "id": "old-model" }]
                    }
                }
            }"#,
        )
        .unwrap();

        let mut entry =
            crate::coding::pi::commands::read_pi_models_provider_at(&path, "plugin-demo")
                .unwrap()
                .expect("provider entry");
        let old = model_ids_from_value(entry.get("models").unwrap());
        assert_eq!(old, vec!["old-model".to_string()]);
        patch_entry_models(FileCli::Pi, &mut entry, &sample_models());
        crate::coding::pi::commands::upsert_pi_models_provider_at(&path, "plugin-demo", entry)
            .unwrap();

        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let provider = &after["providers"]["plugin-demo"];
        assert_eq!(provider["displayName"], "User Rename");
        assert_eq!(provider["customFlag"], true);
        assert_eq!(provider["models"].as_array().unwrap().len(), 2);
        let (added, updated) = count_model_changes(&old, &sample_models());
        assert_eq!((added, updated), (2, 0));
    }
}
