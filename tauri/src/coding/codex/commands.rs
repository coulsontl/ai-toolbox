use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::adapter;
use super::constants::{AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME, CODEX_LOCAL_PROVIDER_ID};
use super::history_sync;
use super::official_accounts::{
    auth_has_official_runtime, clear_all_codex_official_account_apply_status,
    codex_provider_has_official_accounts, ensure_codex_provider_has_no_official_accounts,
    sync_codex_official_account_apply_status,
};
use super::plugin_ops;
use super::plugin_state;
use super::plugin_types::{
    CodexInstalledPlugin, CodexMarketplacePlugin, CodexPluginActionInput,
    CodexPluginBulkActionInput, CodexPluginBulkActionResult, CodexPluginMarketplace,
    CodexPluginRuntimeStatus, CodexPluginWorkspaceRoot, CodexPluginWorkspaceRootInput,
};
use super::plugin_workspace;
use super::types::*;
use super::unified_history;
use crate::coding::all_api_hub;
use crate::coding::db_id::db_new_id;
use crate::coding::open_code::shell_env;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::proxy_gateway::{
    cli_proxy, paths::ProxyGatewayPaths, provider_protocol, provider_switch, types::GatewayCliKey,
};
use crate::coding::runtime_location;
use crate::coding::skills::commands::resync_all_skills_if_tool_path_changed;
use crate::db::helpers::{
    db_count, db_delete, db_delete_all, db_get, db_list, db_max_i64, db_patch_fields, db_put,
    db_query_by_bool, db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use crate::http_client;
use chrono::Local;
use tauri::{Emitter, Runtime};

const PROTECTED_TOP_LEVEL_TOML_KEYS: [&str; 2] = ["mcp_servers", "plugins"];
const PROTECTED_FEATURE_TOML_KEYS: [&str; 1] = ["plugins"];
const CODEX_NO_LOCAL_PROVIDER_CONFIG_ERROR: &str = "No config files found";
const CODEX_MODEL_CATALOG_URLS: [&str; 2] = [
    "https://raw.githubusercontent.com/router-for-me/models/refs/heads/main/models.json",
    "https://models.router-for.me/models.json",
];

const CODEX_BUILTIN_IMAGE_MODEL_ID: &str = "gpt-image-2";

const CODEX_BUNDLED_FREE_MODELS: [(&str, &str); 6] = [
    ("gpt-5.2", "GPT 5.2"),
    ("gpt-5.3-codex", "GPT 5.3 Codex"),
    ("gpt-5.4", "GPT 5.4"),
    ("gpt-5.4-mini", "GPT 5.4 Mini"),
    ("gpt-5.5", "GPT 5.5"),
    ("codex-auto-review", "Codex Auto Review"),
];

const CODEX_BUNDLED_PLUS_PRO_MODELS: [(&str, &str); 7] = [
    ("gpt-5.2", "GPT 5.2"),
    ("gpt-5.3-codex", "GPT 5.3 Codex"),
    ("gpt-5.3-codex-spark", "GPT 5.3 Codex Spark"),
    ("gpt-5.4", "GPT 5.4"),
    ("gpt-5.4-mini", "GPT 5.4 Mini"),
    ("gpt-5.5", "GPT 5.5"),
    ("codex-auto-review", "Codex Auto Review"),
];

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistorySourceInput {
    #[serde(default)]
    pub source_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexHistorySourceMode {
    All,
    Local,
    Wsl,
}

impl CodexHistorySourceMode {
    fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("all")
        {
            "all" => Ok(Self::All),
            "local" => Ok(Self::Local),
            "wsl" => Ok(Self::Wsl),
            value => Err(format!("Unsupported Codex history source mode: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CodexHistoryRuntimeSource {
    Local,
    Wsl,
}

impl CodexHistoryRuntimeSource {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Wsl => "wsl",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CodexHistorySourceCandidate {
    pub(super) root_dir: PathBuf,
    pub(super) source: CodexHistoryRuntimeSource,
    pub(super) distro: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexHistorySourceTarget {
    root_dir: PathBuf,
    source: CodexHistoryRuntimeSource,
    distro: Option<String>,
    available_sources: Vec<history_sync::CodexHistorySourceOption>,
}

#[derive(Debug, Deserialize)]
struct RemoteCodexModelCatalog {
    #[serde(default, rename = "codex-free")]
    codex_free: Vec<RemoteCodexModel>,
    #[serde(default, rename = "codex-team")]
    codex_team: Vec<RemoteCodexModel>,
    #[serde(default, rename = "codex-plus")]
    codex_plus: Vec<RemoteCodexModel>,
    #[serde(default, rename = "codex-pro")]
    codex_pro: Vec<RemoteCodexModel>,
}

#[derive(Debug, Deserialize)]
struct RemoteCodexModel {
    id: String,
    #[serde(default, alias = "displayName")]
    display_name: Option<String>,
    #[serde(default, alias = "ownedBy")]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<i64>,
}

// ============================================================================
// Codex Config Path Commands
// ============================================================================

/// Get Codex config directory path (~/.codex/)
fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

pub fn get_codex_default_root_dir() -> Result<PathBuf, String> {
    Ok(get_home_dir()?.join(".codex"))
}

fn get_codex_root_dir_from_shell() -> Option<PathBuf> {
    shell_env::get_env_from_shell_config("CODEX_HOME")
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub(crate) fn get_codex_root_dir_without_db() -> Result<PathBuf, String> {
    if let Ok(env_path) = std::env::var("CODEX_HOME") {
        if !env_path.trim().is_empty() {
            return Ok(PathBuf::from(env_path));
        }
    }

    if let Some(shell_path) = get_codex_root_dir_from_shell() {
        return Ok(shell_path);
    }

    get_codex_default_root_dir()
}

pub(super) async fn get_codex_custom_root_dir_async(
    db: &crate::db::SqliteDbState,
) -> Option<PathBuf> {
    if let Ok(Some(config)) = get_codex_common_from_sqlite(db) {
        return config
            .root_dir
            .filter(|dir| !dir.trim().is_empty())
            .map(PathBuf::from);
    }
    None
}

pub fn get_codex_root_dir_from_db(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(runtime_location::get_codex_runtime_location_sync(db)?.host_path)
}

fn resolve_local_provider_meta(
    provider_input: Option<&CodexProviderInput>,
    base_meta: Option<Value>,
) -> Option<Value> {
    provider_input
        .and_then(|provider| provider.meta.clone())
        .or(base_meta)
}

pub(super) async fn get_codex_root_dir_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(runtime_location::get_codex_runtime_location_async(db)
        .await?
        .host_path)
}

pub fn get_codex_root_path_info_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<ConfigPathInfo, String> {
    let location = runtime_location::get_codex_runtime_location_sync(db)?;
    Ok(ConfigPathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

async fn resolve_codex_history_source_target(
    db: &crate::db::SqliteDbState,
    input: Option<&CodexHistorySourceInput>,
) -> Result<CodexHistorySourceTarget, String> {
    let source_mode =
        CodexHistorySourceMode::parse(input.and_then(|value| value.source_mode.as_deref()))?;
    let candidates = resolve_codex_history_source_candidates(db).await?;
    let available_sources = codex_history_available_sources(&candidates);

    let selected = select_codex_history_source_candidate(source_mode, &candidates)?;

    Ok(CodexHistorySourceTarget {
        root_dir: selected.root_dir.clone(),
        source: selected.source,
        distro: selected.distro.clone(),
        available_sources,
    })
}

fn select_codex_history_source_candidate<'a>(
    source_mode: CodexHistorySourceMode,
    candidates: &'a [CodexHistorySourceCandidate],
) -> Result<&'a CodexHistorySourceCandidate, String> {
    match source_mode {
        CodexHistorySourceMode::All => candidates
            .iter()
            .find(|candidate| candidate.source == CodexHistoryRuntimeSource::Local)
            .or_else(|| candidates.first()),
        CodexHistorySourceMode::Local => candidates
            .iter()
            .find(|candidate| candidate.source == CodexHistoryRuntimeSource::Local),
        CodexHistorySourceMode::Wsl => candidates
            .iter()
            .find(|candidate| candidate.source == CodexHistoryRuntimeSource::Wsl),
    }
    .ok_or_else(|| match source_mode {
        CodexHistorySourceMode::Local => {
            "Codex history local source is unavailable for the current runtime".to_string()
        }
        CodexHistorySourceMode::Wsl => {
            "Codex history WSL source is unavailable. Enable WSL sync or use a WSL Codex root first"
                .to_string()
        }
        CodexHistorySourceMode::All => "No Codex history source is available".to_string(),
    })
}

pub(super) async fn resolve_codex_history_source_candidates(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<CodexHistorySourceCandidate>, String> {
    let runtime_location = runtime_location::get_codex_runtime_location_async(db).await?;
    if let Some(wsl) = runtime_location.wsl.clone().or_else(|| {
        runtime_location
            .host_path
            .to_str()
            .and_then(runtime_location::parse_wsl_unc_path)
    }) {
        return Ok(vec![CodexHistorySourceCandidate {
            root_dir: runtime_location.host_path,
            source: CodexHistoryRuntimeSource::Wsl,
            distro: Some(wsl.distro),
        }]);
    }

    let mut candidates = vec![CodexHistorySourceCandidate {
        root_dir: runtime_location.host_path,
        source: CodexHistoryRuntimeSource::Local,
        distro: None,
    }];

    if let Some(wsl_candidate) = resolve_wsl_sync_codex_history_source(db) {
        candidates.push(wsl_candidate);
    }

    Ok(candidates)
}

fn resolve_wsl_sync_codex_history_source(
    db: &crate::db::SqliteDbState,
) -> Option<CodexHistorySourceCandidate> {
    let config = db
        .with_conn(|conn| db_get(conn, DbTable::WslSyncConfig, "config"))
        .ok()??;

    if config.get("enabled").and_then(Value::as_bool) != Some(true) {
        return None;
    }

    let configured_distro = config
        .get("distro")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let distro = crate::coding::wsl::get_effective_distro(configured_distro).ok()?;
    let linux_home = crate::coding::wsl::get_wsl_user_home(&distro).ok()?;
    let codex_linux_root = linux_join(&linux_home, ".codex");

    Some(CodexHistorySourceCandidate {
        root_dir: runtime_location::build_windows_unc_path(&distro, &codex_linux_root),
        source: CodexHistoryRuntimeSource::Wsl,
        distro: Some(distro),
    })
}

fn linux_join(root: &str, suffix: &str) -> String {
    format!(
        "{}/{}",
        root.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

fn codex_history_available_sources(
    candidates: &[CodexHistorySourceCandidate],
) -> Vec<history_sync::CodexHistorySourceOption> {
    let mut sources = Vec::new();
    let mut seen = BTreeSet::new();

    for source in [
        CodexHistoryRuntimeSource::Local,
        CodexHistoryRuntimeSource::Wsl,
    ] {
        let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.source == source)
        else {
            continue;
        };
        if seen.insert(source.as_str()) {
            sources.push(history_sync::CodexHistorySourceOption {
                source: source.as_str().to_string(),
                distro: candidate.distro.clone(),
            });
        }
    }

    sources
}

fn decorate_codex_history_status(
    status: &mut history_sync::CodexHistorySyncStatus,
    target: &CodexHistorySourceTarget,
) {
    status.available_sources = target.available_sources.clone();
    status.runtime_source = Some(target.source.as_str().to_string());
    status.runtime_distro = target.distro.clone();
}

async fn get_codex_root_path_info_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<ConfigPathInfo, String> {
    let location = runtime_location::get_codex_runtime_location_async(db).await?;
    Ok(ConfigPathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

fn get_codex_config_dir() -> Result<std::path::PathBuf, String> {
    get_codex_root_dir_without_db()
}

async fn get_codex_config_dir_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<std::path::PathBuf, String> {
    get_codex_root_dir_from_db_async(db).await
}

async fn get_codex_auth_path_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<std::path::PathBuf, String> {
    Ok(get_codex_config_dir_from_db_async(db)
        .await?
        .join("auth.json"))
}

async fn get_codex_config_path_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<std::path::PathBuf, String> {
    Ok(get_codex_config_dir_from_db_async(db)
        .await?
        .join("config.toml"))
}

fn get_codex_prompt_file_path() -> Result<std::path::PathBuf, String> {
    Ok(runtime_location::resolve_codex_prompt_file_path(
        &get_codex_config_dir()?,
    ))
}

async fn get_codex_prompt_file_path_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<std::path::PathBuf, String> {
    Ok(runtime_location::resolve_codex_prompt_file_path(
        &get_codex_config_dir_from_db_async(db).await?,
    ))
}

async fn get_local_prompt_config(
    db: Option<&crate::db::SqliteDbState>,
) -> Result<Option<CodexPromptConfig>, String> {
    let prompt_path = if let Some(db) = db {
        get_codex_prompt_file_path_from_db_async(db).await?
    } else {
        get_codex_prompt_file_path()?
    };
    let Some(prompt_content) = read_prompt_content_file(&prompt_path, "Codex")? else {
        return Ok(None);
    };

    let now = Local::now().to_rfc3339();
    Ok(Some(CodexPromptConfig {
        id: CODEX_LOCAL_PROVIDER_ID.to_string(),
        name: "default".to_string(),
        content: prompt_content,
        is_applied: true,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }))
}

async fn read_codex_settings_from_disk(
    db: Option<&crate::db::SqliteDbState>,
) -> Result<CodexSettings, String> {
    let (auth_path, config_path) = if let Some(db) = db {
        (
            get_codex_auth_path_from_db_async(db).await?,
            get_codex_config_path_from_db_async(db).await?,
        )
    } else {
        let root_dir = get_codex_config_dir()?;
        (root_dir.join("auth.json"), root_dir.join("config.toml"))
    };

    let auth: Option<serde_json::Value> = if auth_path.exists() {
        let content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        Some(
            serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse auth.json: {}", e))?,
        )
    } else {
        None
    };

    let config = if config_path.exists() {
        let raw = fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.toml: {}", e))?;
        // Self-heal a dangling `model_provider` that points to a missing
        // `[model_providers.<id>]` table. Older gateway takeovers wrote
        // `model_provider = "ai-toolbox-gateway"`; if that sentinel table was
        // later lost (manual edit, partial migration, cross-replica sync) the
        // leftover field makes Codex CLI refuse to load config.toml with
        // "Model provider `ai-toolbox-gateway` not found".
        heal_dangling_codex_model_provider(&config_path, &raw)?.or(Some(raw))
    } else {
        None
    };

    Ok(CodexSettings { auth, config })
}

/// Remove a dangling top-level `model_provider` when it points to a
/// `[model_providers.<id>]` table that does not exist in config.toml.
///
/// Returns `Some(healed_text)` and writes it back to disk when a dangling
/// reference was repaired; returns `None` when no healing was needed (empty,
/// unparseable, no `model_provider`, or the referenced table is present).
/// Reads are the widest funnel to repair this regardless of how the dangling
/// state was produced, so Codex CLI can always load config.toml.
fn heal_dangling_codex_model_provider(
    config_path: &Path,
    config_text: &str,
) -> Result<Option<String>, String> {
    if config_text.trim().is_empty() {
        return Ok(None);
    }
    let mut document = match parse_toml_document(config_text, "config.toml self-heal") {
        Ok(document) => document,
        // Don't break the read path on a pre-existing parse error; surface the
        // raw text so the caller or Codex itself can report it.
        Err(_) => return Ok(None),
    };

    let Some(provider_id) = active_codex_model_provider_id(&document) else {
        return Ok(None);
    };

    let table_exists = document
        .as_table()
        .get("model_providers")
        .and_then(toml_edit::Item::as_table)
        .and_then(|providers| providers.get(provider_id.as_str()))
        .is_some();

    if table_exists {
        return Ok(None);
    }

    document.as_table_mut().remove("model_provider");
    let healed = document.to_string();
    fs::write(config_path, &healed)
        .map_err(|e| format!("Failed to heal dangling model_provider in config.toml: {e}"))?;
    log::warn!(
        "Codex config.toml had dangling model_provider = \"{}\" with no matching [model_providers.{}] table; removed it to restore Codex loadability.",
        provider_id,
        provider_id
    );
    Ok(Some(healed))
}

fn normalize_codex_model_tier(plan_type: &str) -> &'static str {
    match plan_type.trim().to_lowercase().as_str() {
        "free" => "free",
        "team" | "business" | "go" => "team",
        "plus" => "plus",
        "pro" => "pro",
        _ => "pro",
    }
}

fn push_codex_official_model(
    models: &mut Vec<CodexOfficialModel>,
    seen_model_ids: &mut BTreeSet<String>,
    model: CodexOfficialModel,
) {
    let model_id = model.id.trim();
    if model_id.is_empty() {
        return;
    }

    let model_id_key = model_id.to_lowercase();
    if seen_model_ids.contains(&model_id_key) {
        return;
    }

    seen_model_ids.insert(model_id_key);
    models.push(CodexOfficialModel {
        id: model_id.to_string(),
        name: model
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
        owned_by: model
            .owned_by
            .map(|owned_by| owned_by.trim().to_string())
            .filter(|owned_by| !owned_by.is_empty()),
        created: model.created,
    });
}

fn codex_bundled_models_for_tier(tier: &str) -> &'static [(&'static str, &'static str)] {
    match tier {
        "free" | "team" => &CODEX_BUNDLED_FREE_MODELS,
        _ => &CODEX_BUNDLED_PLUS_PRO_MODELS,
    }
}

fn append_codex_builtin_models(
    models: &mut Vec<CodexOfficialModel>,
    seen_model_ids: &mut BTreeSet<String>,
) {
    push_codex_official_model(
        models,
        seen_model_ids,
        CodexOfficialModel {
            id: CODEX_BUILTIN_IMAGE_MODEL_ID.to_string(),
            name: Some("GPT Image 2".to_string()),
            owned_by: Some("openai".to_string()),
            created: Some(1_704_067_200),
        },
    );
}

fn static_codex_official_models(tier: &str) -> Vec<CodexOfficialModel> {
    let mut models = Vec::new();
    let mut seen_model_ids = BTreeSet::new();

    for (model_id, display_name) in codex_bundled_models_for_tier(tier) {
        push_codex_official_model(
            &mut models,
            &mut seen_model_ids,
            CodexOfficialModel {
                id: (*model_id).to_string(),
                name: Some((*display_name).to_string()),
                owned_by: Some("openai".to_string()),
                created: None,
            },
        );
    }

    append_codex_builtin_models(&mut models, &mut seen_model_ids);
    models
}

fn select_remote_codex_models(
    catalog: RemoteCodexModelCatalog,
    tier: &str,
) -> Vec<RemoteCodexModel> {
    match tier {
        "free" => catalog.codex_free,
        "team" => catalog.codex_team,
        "plus" => catalog.codex_plus,
        _ => catalog.codex_pro,
    }
}

fn merge_remote_codex_official_models(
    remote_models: Vec<RemoteCodexModel>,
) -> Vec<CodexOfficialModel> {
    let mut models = Vec::new();
    let mut seen_model_ids = BTreeSet::new();

    for remote_model in remote_models {
        if remote_model
            .id
            .trim()
            .eq_ignore_ascii_case(CODEX_BUILTIN_IMAGE_MODEL_ID)
        {
            continue;
        }

        push_codex_official_model(
            &mut models,
            &mut seen_model_ids,
            CodexOfficialModel {
                id: remote_model.id,
                name: remote_model.display_name,
                owned_by: remote_model.owned_by.or_else(|| Some("openai".to_string())),
                created: remote_model.created,
            },
        );
    }

    append_codex_builtin_models(&mut models, &mut seen_model_ids);
    models
}

async fn fetch_remote_codex_model_catalog(
    client: &reqwest::Client,
    url: &str,
    tier: &str,
) -> Result<Vec<RemoteCodexModel>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("request failed: {}", error))?;

    if !response.status().is_success() {
        return Err(format!("request failed with status {}", response.status()));
    }

    let catalog = response
        .json::<RemoteCodexModelCatalog>()
        .await
        .map_err(|error| format!("failed to parse model catalog: {}", error))?;
    let models = select_remote_codex_models(catalog, tier);
    if models.is_empty() {
        return Err(format!("codex {} model catalog is empty", tier));
    }

    Ok(models)
}

#[tauri::command]
pub async fn fetch_codex_official_models(
    state: tauri::State<'_, SqliteDbState>,
    plan_type: String,
) -> Result<CodexOfficialModelsResponse, String> {
    let tier = normalize_codex_model_tier(&plan_type);

    if let Ok(client) = http_client::client_with_timeout(&state, 30).await {
        for url in CODEX_MODEL_CATALOG_URLS {
            match fetch_remote_codex_model_catalog(&client, url, tier).await {
                Ok(remote_models) => {
                    let models = merge_remote_codex_official_models(remote_models);
                    let total = models.len();
                    return Ok(CodexOfficialModelsResponse {
                        models,
                        total,
                        source: "remote".to_string(),
                        tier: tier.to_string(),
                    });
                }
                Err(error) => {
                    log::warn!(
                        "[Codex] Failed to fetch official model catalog from {} for tier {}: {}",
                        url,
                        tier,
                        error
                    );
                }
            }
        }
    } else {
        log::warn!("[Codex] Failed to create HTTP client for official model catalog");
    }

    let models = static_codex_official_models(tier);
    let total = models.len();
    Ok(CodexOfficialModelsResponse {
        models,
        total,
        source: "bundled".to_string(),
        tier: tier.to_string(),
    })
}

async fn write_prompt_content_to_file(
    db: Option<&crate::db::SqliteDbState>,
    prompt_content: Option<&str>,
) -> Result<(), String> {
    let prompt_path = if let Some(db) = db {
        get_codex_prompt_file_path_from_db_async(db).await?
    } else {
        get_codex_prompt_file_path()?
    };
    write_prompt_content_file(&prompt_path, prompt_content, "Codex")
}

fn emit_prompt_sync_requests<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "windows")]
    let _ = _app.emit("wsl-sync-request-codex", ());
}

fn emit_codex_runtime_config_changed<R: Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("config-changed", "window");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-codex", ());
}

fn codex_gateway_takeover_active<R: Runtime>(_app: &tauri::AppHandle<R>) -> bool {
    let paths = ProxyGatewayPaths::new(crate::app_paths::resolved_data_dir());
    cli_proxy::provider_switch_locked_by_manifest(&paths, GatewayCliKey::Codex)
}

fn ensure_codex_provider_native_for_direct(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let Some(provider) = get_codex_provider_from_sqlite(db, provider_id)? else {
        return Ok(());
    };
    if provider_protocol::provider_needs_gateway_proxy(
        GatewayCliKey::Codex,
        &provider.category,
        provider.meta.as_ref(),
        &provider.settings_config,
    ) {
        return Err("该渠道协议不是 Codex 原生协议，请先开启网关后使用“应用并代理”".to_string());
    }
    Ok(())
}

fn codex_provider_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::single(OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?))
}

fn codex_prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn list_codex_providers_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<CodexProvider>, String> {
    let order = codex_provider_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::CodexProvider, Some(&order))?
            .into_iter()
            .map(adapter::from_db_value_provider)
            .collect())
    })
}

fn get_codex_provider_from_sqlite(
    sqlite_state: &SqliteDbState,
    provider_id: &str,
) -> Result<Option<CodexProvider>, String> {
    sqlite_state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::CodexProvider, provider_id)?.map(adapter::from_db_value_provider))
    })
}

fn put_codex_provider_to_sqlite(
    sqlite_state: &SqliteDbState,
    provider_id: &str,
    content: &CodexProviderContent,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CodexProvider,
            provider_id,
            &adapter::to_db_value_provider(content),
        )
    })
}

fn delete_codex_provider_from_sqlite(
    sqlite_state: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| db_delete(conn, DbTable::CodexProvider, provider_id).map(|_| ()))
}

fn list_codex_prompts_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<CodexPromptConfig>, String> {
    let order = codex_prompt_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::CodexPromptConfig, Some(&order))?
            .into_iter()
            .map(adapter::from_db_value_prompt)
            .collect())
    })
}

fn get_codex_prompt_from_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
) -> Result<Option<CodexPromptConfig>, String> {
    sqlite_state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::CodexPromptConfig, config_id)?.map(adapter::from_db_value_prompt))
    })
}

fn put_codex_prompt_to_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
    content: &CodexPromptConfigContent,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CodexPromptConfig,
            config_id,
            &adapter::to_db_value_prompt(content),
        )
    })
}

fn delete_codex_prompt_from_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
) -> Result<(), String> {
    sqlite_state
        .with_conn(|conn| db_delete(conn, DbTable::CodexPromptConfig, config_id).map(|_| ()))
}

fn get_codex_common_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Option<CodexCommonConfig>, String> {
    sqlite_state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::CodexCommonConfig, "common")?.map(adapter::from_db_value_common))
    })
}

fn put_codex_common_to_sqlite(sqlite_state: &SqliteDbState, data: &Value) -> Result<(), String> {
    sqlite_state.with_conn(|conn| db_put(conn, DbTable::CodexCommonConfig, "common", data))
}

fn emit_codex_plugin_config_changed<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("config-changed", "window");
    emit_prompt_sync_requests(app);
}

/// Get Codex config directory path
#[tauri::command]
pub async fn get_codex_config_dir_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    let db = state.db();
    let config_dir = get_codex_config_dir_from_db_async(&db).await?;
    Ok(config_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_codex_plugin_runtime_status(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<CodexPluginRuntimeStatus, String> {
    let db = state.db();
    plugin_state::get_codex_plugin_runtime_status(&db).await
}

#[tauri::command]
pub async fn list_codex_installed_plugins(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<CodexInstalledPlugin>, String> {
    let db = state.db();
    plugin_state::list_codex_installed_plugins(&db).await
}

#[tauri::command]
pub async fn list_codex_marketplaces(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<CodexPluginMarketplace>, String> {
    let db = state.db();
    plugin_state::list_codex_marketplaces(&db).await
}

#[tauri::command]
pub async fn list_codex_marketplace_plugins(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<CodexMarketplacePlugin>, String> {
    let db = state.db();
    plugin_state::list_codex_marketplace_plugins(&db).await
}

#[tauri::command]
pub async fn list_codex_plugin_workspace_roots(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<CodexPluginWorkspaceRoot>, String> {
    let db = state.db();
    plugin_workspace::list_codex_plugin_workspace_roots(&db).await
}

#[tauri::command]
pub async fn add_codex_plugin_workspace_root(
    state: tauri::State<'_, SqliteDbState>,
    input: CodexPluginWorkspaceRootInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_workspace::add_codex_plugin_workspace_root(&db, &input.path).await
}

#[tauri::command]
pub async fn remove_codex_plugin_workspace_root(
    state: tauri::State<'_, SqliteDbState>,
    input: CodexPluginWorkspaceRootInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_workspace::remove_codex_plugin_workspace_root(&db, &input.path).await
}

#[tauri::command]
pub async fn install_codex_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPluginActionInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_ops::install_codex_plugin(&db, &input.plugin_id).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn enable_codex_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPluginActionInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_ops::set_codex_plugin_enabled(&db, &input.plugin_id, true).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn disable_codex_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPluginActionInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_ops::set_codex_plugin_enabled(&db, &input.plugin_id, false).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_codex_installed_plugins_enabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPluginBulkActionInput,
) -> Result<CodexPluginBulkActionResult, String> {
    let db = state.db();
    let updated_count = plugin_ops::set_codex_installed_plugins_enabled(&db, input.enabled).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(CodexPluginBulkActionResult { updated_count })
}

#[tauri::command]
pub async fn uninstall_codex_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPluginActionInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_ops::uninstall_codex_plugin(&db, &input.plugin_id).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn enable_codex_plugins_feature(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let db = state.db();
    plugin_ops::ensure_codex_plugins_feature_enabled(&db).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn get_codex_root_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ConfigPathInfo, String> {
    let db = state.db();
    get_codex_root_path_info_from_db_async(&db).await
}

#[tauri::command]
pub async fn get_codex_history_sync_status(
    state: tauri::State<'_, SqliteDbState>,
    input: Option<CodexHistorySourceInput>,
) -> Result<history_sync::CodexHistorySyncStatus, String> {
    let target = resolve_codex_history_source_target(&state.db(), input.as_ref()).await?;
    let root_dir = target.root_dir.clone();
    let mut status =
        tauri::async_runtime::spawn_blocking(move || history_sync::get_status(&root_dir))
            .await
            .map_err(|error| format!("Failed to get Codex history sync status: {error}"))??;
    decorate_codex_history_status(&mut status, &target);
    Ok(status)
}

#[tauri::command]
pub async fn backup_codex_history(
    state: tauri::State<'_, SqliteDbState>,
    input: Option<CodexHistorySourceInput>,
) -> Result<history_sync::CodexHistoryBackupResult, String> {
    let target = resolve_codex_history_source_target(&state.db(), input.as_ref()).await?;
    let root_dir = target.root_dir;
    tauri::async_runtime::spawn_blocking(move || history_sync::backup(&root_dir, "manual"))
        .await
        .map_err(|error| format!("Failed to backup Codex history: {error}"))?
}

#[tauri::command]
pub async fn sync_codex_history(
    state: tauri::State<'_, SqliteDbState>,
    input: Option<CodexHistorySourceInput>,
) -> Result<history_sync::CodexHistorySyncResult, String> {
    let target = resolve_codex_history_source_target(&state.db(), input.as_ref()).await?;
    let root_dir = target.root_dir.clone();
    let mut result = tauri::async_runtime::spawn_blocking(move || history_sync::sync(&root_dir))
        .await
        .map_err(|error| format!("Failed to sync Codex history: {error}"))??;
    decorate_codex_history_status(&mut result.status, &target);
    Ok(result)
}

#[tauri::command]
pub async fn restore_latest_codex_history_backup(
    state: tauri::State<'_, SqliteDbState>,
    input: Option<CodexHistorySourceInput>,
) -> Result<history_sync::CodexHistoryRestoreResult, String> {
    let target = resolve_codex_history_source_target(&state.db(), input.as_ref()).await?;
    let root_dir = target.root_dir.clone();
    let mut result =
        tauri::async_runtime::spawn_blocking(move || history_sync::restore_latest(&root_dir))
            .await
            .map_err(|error| format!("Failed to restore Codex history backup: {error}"))??;
    decorate_codex_history_status(&mut result.status, &target);
    Ok(result)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifiedSessionHistoryInput {
    pub enabled: bool,
    #[serde(default)]
    pub migrate_existing: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifiedSessionHistoryUpdateResult {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration: Option<unified_history::CodexUnifiedHistoryMigrationResult>,
}

#[tauri::command]
pub async fn set_codex_unified_session_history(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexUnifiedSessionHistoryInput,
) -> Result<CodexUnifiedSessionHistoryUpdateResult, String> {
    if codex_gateway_takeover_active(&app) {
        return Err("当前 Codex 已由网关接管，请先恢复直连后再修改统一会话历史设置".to_string());
    }

    let db = state.db();
    let existing_settings = crate::settings::store::load_settings_from_sqlite_state(&db)?;
    let applied_provider = get_applied_codex_provider(&db).await?;
    let previous_managed_config_toml = match applied_provider.as_ref() {
        Some(provider) => Some(
            get_managed_codex_config_for_provider_cleanup_with_unified_history(
                &db,
                provider,
                existing_settings.codex_unified_session_history_enabled,
            )
            .await?,
        ),
        None => None,
    };

    let mut next_settings = existing_settings.clone();
    next_settings.codex_unified_session_history_enabled = input.enabled;
    crate::settings::store::save_settings_to_sqlite_state(&db, &next_settings)?;

    if let Some(provider) = applied_provider
        .as_ref()
        .filter(|provider| provider.category == "official")
    {
        if let Err(error) = apply_config_to_file_with_previous_managed_config(
            &db,
            &provider.id,
            previous_managed_config_toml,
        )
        .await
        {
            if let Err(rollback_error) =
                crate::settings::store::save_settings_to_sqlite_state(&db, &existing_settings)
            {
                log::error!(
                    "Failed to roll back Codex unified session history setting: {rollback_error}"
                );
            }
            return Err(format!(
                "统一 Codex 会话历史设置未生效（live 配置重写失败）: {error}"
            ));
        }
        emit_codex_runtime_config_changed(&app);
    }

    let migration = if input.enabled && input.migrate_existing {
        let root_dir = get_codex_root_dir_from_db_async(&db).await?;
        Some(
            match tauri::async_runtime::spawn_blocking(move || {
                unified_history::migrate_official_history_to_unified(&root_dir)
            })
            .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => {
                    log::warn!("Failed to migrate Codex official history after enabling unified session history: {error}");
                    unified_history::CodexUnifiedHistoryMigrationResult {
                        migrated_session_files: 0,
                        migrated_session_entries: 0,
                        migrated_thread_rows: 0,
                        rewritten_index_entries: 0,
                        backup_path: None,
                        skipped_reason: Some("migration_failed".to_string()),
                        duration_ms: 0,
                    }
                }
                Err(error) => {
                    log::warn!("Failed to join Codex official history migration task after enabling unified session history: {error}");
                    unified_history::CodexUnifiedHistoryMigrationResult {
                        migrated_session_files: 0,
                        migrated_session_entries: 0,
                        migrated_thread_rows: 0,
                        rewritten_index_entries: 0,
                        backup_path: None,
                        skipped_reason: Some("migration_failed".to_string()),
                        duration_ms: 0,
                    }
                }
            },
        )
    } else {
        None
    };

    Ok(CodexUnifiedSessionHistoryUpdateResult {
        enabled: input.enabled,
        migration,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPreserveOfficialAuthOnSwitchInput {
    pub enabled: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPreserveOfficialAuthOnSwitchUpdateResult {
    pub enabled: bool,
}

/// Toggle "keep official login when switching third-party providers".
///
/// Writes the setting first, then re-applies the current provider (if any) so
/// live `auth.json` / `config.toml` pick up the new projection immediately.
/// Gateway takeover reuses `apply_or_switch_provider` (restore direct → apply →
/// re-engage single/failover).
#[tauri::command]
pub async fn set_codex_preserve_official_auth_on_switch(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPreserveOfficialAuthOnSwitchInput,
) -> Result<CodexPreserveOfficialAuthOnSwitchUpdateResult, String> {
    let db = state.db();
    let existing_settings = crate::settings::store::load_settings_from_sqlite_state(&db)?;
    if existing_settings.codex_preserve_official_auth_on_switch == input.enabled {
        return Ok(CodexPreserveOfficialAuthOnSwitchUpdateResult {
            enabled: input.enabled,
        });
    }

    let applied_provider = get_applied_codex_provider(&db).await?;
    let mut next_settings = existing_settings.clone();
    next_settings.codex_preserve_official_auth_on_switch = input.enabled;
    crate::settings::store::save_settings_to_sqlite_state(&db, &next_settings)?;

    if let Some(provider) = applied_provider {
        if let Err(error) = provider_switch::apply_or_switch_provider(
            &app,
            GatewayCliKey::Codex,
            &provider.id,
            false,
        )
        .await
        {
            if let Err(rollback_error) =
                crate::settings::store::save_settings_to_sqlite_state(&db, &existing_settings)
            {
                log::error!(
                    "Failed to roll back Codex preserve official auth setting: {rollback_error}"
                );
            }
            return Err(format!(
                "切换第三方时保留官方登录设置未生效（live 配置重写失败）: {error}"
            ));
        }
    }

    Ok(CodexPreserveOfficialAuthOnSwitchUpdateResult {
        enabled: input.enabled,
    })
}

#[tauri::command]
pub async fn has_codex_unified_history_backup(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<bool, String> {
    let root_dir = get_codex_root_dir_from_db_async(&state.db()).await?;
    tauri::async_runtime::spawn_blocking(move || {
        unified_history::has_codex_unified_history_backup(&root_dir)
    })
    .await
    .map_err(|error| format!("Failed to inspect Codex unified history backup: {error}"))
}

#[tauri::command]
pub async fn restore_codex_unified_session_history(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<unified_history::CodexUnifiedHistoryRestoreResult, String> {
    let db = state.db();
    let settings = crate::settings::store::load_settings_from_sqlite_state(&db)?;
    let unified_history_enabled = settings.codex_unified_session_history_enabled;
    let root_dir = get_codex_root_dir_from_db_async(&db).await?;
    tauri::async_runtime::spawn_blocking(move || {
        unified_history::restore_official_history_from_unified_backups(
            &root_dir,
            unified_history_enabled,
        )
    })
    .await
    .map_err(|error| format!("Failed to restore Codex unified history: {error}"))?
}

/// Get Codex config.toml file path
#[tauri::command]
pub async fn get_codex_config_file_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    let db = state.db();
    let config_path = get_codex_config_path_from_db_async(&db).await?;
    Ok(config_path.to_string_lossy().to_string())
}

/// Reveal Codex config folder in file explorer
#[tauri::command]
pub async fn reveal_codex_config_folder(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<(), String> {
    let db = state.db();
    let config_dir = get_codex_config_dir_from_db_async(&db).await?;

    // Ensure directory exists
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create .codex directory: {}", e))?;
    }

    // Open in file explorer (platform-specific)
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&config_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}

// ============================================================================
// Codex Provider Commands
// ============================================================================

/// List all Codex providers ordered by sort_index
/// If database is empty, persists a local official login as the default provider before
/// falling back to a temporary provider loaded from local config files.
#[tauri::command]
pub async fn list_codex_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<CodexProvider>, String> {
    list_codex_providers_for_db(state.db()).await
}

pub async fn list_codex_providers_for_db(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<CodexProvider>, String> {
    let mut providers = list_codex_providers_from_sqlite(db)?;
    if providers.is_empty() {
        import_codex_default_provider_from_local_files(db, true).await?;
        providers = list_codex_providers_from_sqlite(db)?;
    }
    if providers.is_empty() {
        if let Ok(temp_provider) = load_temp_provider_from_files_with_db(Some(db)).await {
            return Ok(vec![temp_provider]);
        }
    }
    Ok(providers)
}

/// 修复损坏的 Codex provider 数据
/// This is used when the database is empty and we want to show the local config
async fn load_local_codex_provider_snapshot(
    db: Option<&crate::db::SqliteDbState>,
) -> Result<(serde_json::Value, serde_json::Value, String), String> {
    let root_dir = if let Some(db) = db {
        get_codex_root_dir_from_db_async(db).await?
    } else {
        get_codex_root_dir_without_db()?
    };
    let auth_path = root_dir.join("auth.json");
    let config_path = root_dir.join("config.toml");

    if !auth_path.exists() && !config_path.exists() {
        return Err(CODEX_NO_LOCAL_PROVIDER_CONFIG_ERROR.to_string());
    }

    let auth: serde_json::Value = if auth_path.exists() {
        let auth_content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        serde_json::from_str(&auth_content)
            .map_err(|e| format!("Failed to parse auth.json: {}", e))?
    } else {
        serde_json::json!({})
    };

    let config_toml = if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    let settings = serde_json::json!({
        "auth": auth,
        "config": config_toml
    });
    let stored_common_toml = if let Some(db) = db {
        get_codex_common_toml(db).await?
    } else {
        None
    };
    let provider_settings =
        extract_provider_settings_for_storage(&settings, stored_common_toml.as_deref())?;
    let settings_config = serde_json::to_string(&provider_settings)
        .map_err(|error| format!("Failed to serialize provider settings: {error}"))?;

    Ok((auth, provider_settings, settings_config))
}

/// 修复损坏的 Codex provider 数据
/// This is used when the database is empty and we want to show the local config
async fn load_temp_provider_from_files_with_db(
    db: Option<&crate::db::SqliteDbState>,
) -> Result<CodexProvider, String> {
    let (_, provider_settings, settings_config) = load_local_codex_provider_snapshot(db).await?;
    let category = infer_codex_provider_category_from_settings(&provider_settings);
    if category == "official" {
        return Err("No third-party local config found".to_string());
    }

    let now = Local::now().to_rfc3339();
    Ok(CodexProvider {
        id: CODEX_LOCAL_PROVIDER_ID.to_string(), // Special ID to indicate this is from local files
        name: "default".to_string(),
        category,
        settings_config,
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: None,
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

pub async fn import_codex_default_provider_from_local_files(
    db: &crate::db::SqliteDbState,
    require_local_official_runtime: bool,
) -> Result<Option<CodexProvider>, String> {
    if db.with_conn(|conn| db_count(conn, DbTable::CodexProvider))? > 0 {
        return Ok(None);
    }

    let (auth, provider_settings, settings_config) =
        match load_local_codex_provider_snapshot(Some(db)).await {
            Ok(snapshot) => snapshot,
            Err(error) if error == CODEX_NO_LOCAL_PROVIDER_CONFIG_ERROR => return Ok(None),
            Err(error) => return Err(error),
        };
    let category = infer_codex_provider_category_from_settings(&provider_settings);
    if require_local_official_runtime
        && (category != "official" || !auth_has_official_runtime(&auth))
    {
        return Ok(None);
    }

    let now = Local::now().to_rfc3339();
    let content = CodexProviderContent {
        name: "默认配置".to_string(),
        category,
        settings_config,
        source_provider_id: None,
        website_url: None,
        notes: Some("从配置文件自动导入".to_string()),
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: None,
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_id = db_new_id();
    let inserted = db.with_conn(|conn| {
        if db_count(conn, DbTable::CodexProvider)? > 0 {
            return Ok(false);
        }
        db_put(
            conn,
            DbTable::CodexProvider,
            &provider_id,
            &adapter::to_db_value_provider(&content),
        )?;
        Ok(true)
    })?;

    if !inserted {
        return Ok(None);
    }

    Ok(Some(CodexProvider {
        id: provider_id,
        name: content.name,
        category: content.category,
        settings_config: content.settings_config,
        source_provider_id: content.source_provider_id,
        website_url: content.website_url,
        notes: content.notes,
        icon: content.icon,
        icon_color: content.icon_color,
        sort_index: content.sort_index,
        meta: content.meta,
        is_applied: content.is_applied,
        is_disabled: content.is_disabled,
        created_at: content.created_at,
        updated_at: content.updated_at,
    }))
}

async fn get_codex_common_toml(db: &crate::db::SqliteDbState) -> Result<Option<String>, String> {
    Ok(get_codex_common_from_sqlite(db)?
        .map(|config| config.config)
        .filter(|config| !config.trim().is_empty()))
}

async fn normalize_provider_settings_for_storage(
    db: &crate::db::SqliteDbState,
    raw_settings_config: &str,
    common_config_override: Option<&str>,
) -> Result<String, String> {
    let parsed_settings = parse_codex_settings_config(raw_settings_config)?;
    let effective_common_config = match common_config_override {
        Some(value) => Some(value.to_string()),
        None => get_codex_common_toml(db).await?,
    };

    let normalized_settings = extract_provider_settings_for_storage(
        &parsed_settings,
        effective_common_config.as_deref(),
    )?;

    serde_json::to_string(&normalized_settings)
        .map_err(|error| format!("Failed to serialize normalized provider config: {}", error))
}

async fn extract_codex_common_config_from_current_files_with_db(
    db: &crate::db::SqliteDbState,
) -> Result<CodexCommonConfig, String> {
    // Resolve root once and read only config.toml. Do not reuse read_codex_settings_from_disk:
    // that path also opens auth.json, which is unused here and can hang on unreachable WSL UNC.
    let config_path = get_codex_config_path_from_db_async(db).await?;
    let config_toml =
        crate::coding::file_io::read_text_file_with_timeout(config_path, "Codex config.toml")
            .await?;
    let common_toml = extract_codex_common_config_from_settings_toml(&config_toml)?;
    let now = Local::now().to_rfc3339();

    Ok(CodexCommonConfig {
        config: common_toml,
        root_dir: get_codex_custom_root_dir_async(db)
            .await
            .map(|path| path.to_string_lossy().to_string()),
        updated_at: now,
    })
}

fn extract_codex_common_config_from_settings_toml(config_toml: &str) -> Result<String, String> {
    if config_toml.trim().is_empty() {
        return Ok(String::new());
    }

    let mut document = parse_toml_document(config_toml, "config.toml")?;
    let root_table = document.as_table_mut();
    if config_contains_managed_codex_provider(config_toml)
        || root_table
            .get("base_url")
            .and_then(|item| item.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    {
        root_table.remove("model");
    }
    root_table.remove("model_provider");
    root_table.remove("base_url");
    root_table.remove("model_providers");
    strip_protected_top_level_toml_keys(&mut document);

    Ok(document.to_string().trim().to_string())
}

async fn get_applied_codex_provider(
    db: &crate::db::SqliteDbState,
) -> Result<Option<CodexProvider>, String> {
    let providers = db.with_conn(|conn| {
        db_query_by_bool(
            conn,
            DbTable::CodexProvider,
            &JsonFieldPath::new("is_applied")?,
            true,
            None,
            Some(1),
        )
    })?;
    Ok(providers
        .into_iter()
        .next()
        .map(adapter::from_db_value_provider))
}

async fn query_codex_provider_by_id(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<CodexProvider, String> {
    get_codex_provider_from_sqlite(db, provider_id)?.ok_or_else(|| "Provider not found".to_string())
}

fn parse_toml_document(raw_toml: &str, context: &str) -> Result<toml_edit::DocumentMut, String> {
    if raw_toml.trim().is_empty() {
        Ok(toml_edit::DocumentMut::new())
    } else {
        raw_toml
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("Failed to parse {}: {}", context, e))
    }
}

fn parse_codex_settings_config(
    provider_settings_config: &str,
) -> Result<serde_json::Value, String> {
    serde_json::from_str(provider_settings_config)
        .map_err(|error| format!("Failed to parse provider config: {}", error))
}

fn config_contains_managed_codex_provider(config_toml: &str) -> bool {
    let trimmed_config = config_toml.trim();
    if trimmed_config.is_empty() {
        return false;
    }

    let parsed_document = match parse_toml_document(trimmed_config, "provider config") {
        Ok(document) => document,
        Err(_) => return false,
    };

    selected_codex_provider_has_base_url(parsed_document.as_table())
}

fn config_contains_managed_base_url(config_toml: &str) -> bool {
    let trimmed_config = config_toml.trim();
    if trimmed_config.is_empty() {
        return false;
    }

    let parsed_document = match parse_toml_document(trimmed_config, "provider config") {
        Ok(document) => document,
        Err(_) => return false,
    };

    let root_table = parsed_document.as_table();
    root_table
        .get("base_url")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || selected_codex_provider_has_base_url(root_table)
}

fn selected_codex_provider_has_base_url(root_table: &toml_edit::Table) -> bool {
    let provider_key = root_table
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let Some(provider_key) = provider_key else {
        return false;
    };

    let Some(model_providers_item) = root_table.get("model_providers") else {
        return false;
    };
    let Some(model_providers_table) = model_providers_item.as_table() else {
        return false;
    };
    let Some(selected_provider_item) = model_providers_table.get(provider_key) else {
        return false;
    };
    let Some(selected_provider_table) = selected_provider_item.as_table() else {
        return false;
    };

    selected_provider_table.contains_key("base_url")
}

pub(super) fn infer_codex_provider_category_from_settings(
    provider_settings: &serde_json::Value,
) -> String {
    let has_managed_api_key = provider_settings
        .get("auth")
        .and_then(|value| value.as_object())
        .and_then(|auth| auth.get("OPENAI_API_KEY"))
        .and_then(|value| value.as_str())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    let has_managed_base_url = provider_settings
        .get("config")
        .and_then(|value| value.as_str())
        .map(config_contains_managed_base_url)
        .unwrap_or(false);

    if !has_managed_api_key && !has_managed_base_url {
        "official".to_string()
    } else {
        "custom".to_string()
    }
}

fn extract_codex_managed_api_key(auth: &serde_json::Value) -> Option<String> {
    auth.as_object()
        .and_then(|auth| auth.get("OPENAI_API_KEY"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn active_codex_model_provider_id(document: &toml_edit::DocumentMut) -> Option<String> {
    document
        .as_table()
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_codex_experimental_bearer_token(config_toml: &str) -> Result<Option<String>, String> {
    if config_toml.trim().is_empty() {
        return Ok(None);
    }

    let document = parse_toml_document(config_toml, "config.toml")?;
    if let Some(provider_id) = active_codex_model_provider_id(&document) {
        if let Some(token) = document
            .as_table()
            .get("model_providers")
            .and_then(|item| item.as_table_like())
            .and_then(|providers| providers.get(&provider_id))
            .and_then(|provider| provider.as_table_like())
            .and_then(|provider| provider.get("experimental_bearer_token"))
            .and_then(|token| token.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(Some(token.to_string()));
        }
    }

    Ok(document
        .as_table()
        .get("experimental_bearer_token")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn set_codex_experimental_bearer_token(config_toml: &str, token: &str) -> Result<String, String> {
    let token = token.trim();
    if token.is_empty() {
        return Ok(config_toml.to_string());
    }

    let mut document = parse_toml_document(config_toml, "config.toml")?;
    document.as_table_mut().remove("experimental_bearer_token");
    let active_provider_id = active_codex_model_provider_id(&document).ok_or_else(|| {
        "Cannot preserve official Codex auth for this provider because config.toml has no active model_provider".to_string()
    })?;
    let provider_table = document
        .as_table_mut()
        .get_mut("model_providers")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|providers| providers.get_mut(&active_provider_id))
        .and_then(|provider| provider.as_table_like_mut())
        .ok_or_else(|| {
            format!(
                "Cannot preserve official Codex auth for this provider because [model_providers.{active_provider_id}] is missing"
            )
        })?;

    provider_table.insert("experimental_bearer_token", toml_edit::value(token));

    Ok(document.to_string())
}

fn remove_codex_experimental_bearer_token(config_toml: &str) -> Result<String, String> {
    if config_toml.trim().is_empty() {
        return Ok(String::new());
    }

    let mut document = parse_toml_document(config_toml, "config.toml")?;
    let active_provider_id = active_codex_model_provider_id(&document);
    document.as_table_mut().remove("experimental_bearer_token");

    if let Some(provider_id) = active_provider_id {
        if let Some(provider_table) = document
            .as_table_mut()
            .get_mut("model_providers")
            .and_then(|item| item.as_table_like_mut())
            .and_then(|providers| providers.get_mut(&provider_id))
            .and_then(|item| item.as_table_like_mut())
        {
            provider_table.remove("experimental_bearer_token");
        }
    }

    Ok(document.to_string())
}

fn project_codex_auth_to_runtime_config(
    managed_config_toml: &str,
    managed_auth: &serde_json::Value,
    preserve_official_auth: bool,
) -> Result<String, String> {
    if !preserve_official_auth {
        return Ok(managed_config_toml.to_string());
    }

    let Some(api_key) = extract_codex_managed_api_key(managed_auth) else {
        return Ok(managed_config_toml.to_string());
    };

    set_codex_experimental_bearer_token(managed_config_toml, &api_key)
}

fn should_preserve_codex_official_auth(provider: &CodexProvider, setting_enabled: bool) -> bool {
    setting_enabled && provider.category != "official"
}

fn load_codex_auth_preservation_enabled(db: &crate::db::SqliteDbState) -> Result<bool, String> {
    Ok(crate::settings::store::load_settings_from_sqlite_state(db)?
        .codex_preserve_official_auth_on_switch)
}

fn load_codex_unified_session_history_enabled(
    db: &crate::db::SqliteDbState,
) -> Result<bool, String> {
    Ok(crate::settings::store::load_settings_from_sqlite_state(db)?
        .codex_unified_session_history_enabled)
}

fn restore_codex_provider_token_for_storage(
    settings_value: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut settings_object = settings_value
        .as_object()
        .cloned()
        .ok_or_else(|| "Codex settings must be a JSON object".to_string())?;
    let config_toml = settings_object
        .get("config")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let Some(token) = extract_codex_experimental_bearer_token(config_toml)? else {
        return Ok(serde_json::Value::Object(settings_object));
    };

    let cleaned_config_toml = remove_codex_experimental_bearer_token(config_toml)?;
    settings_object.insert(
        "config".to_string(),
        serde_json::Value::String(cleaned_config_toml),
    );

    let auth_value = settings_object
        .entry("auth".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !auth_value.is_object() {
        *auth_value = serde_json::json!({});
    }
    if let Some(auth_object) = auth_value.as_object_mut() {
        auth_object.insert(
            "OPENAI_API_KEY".to_string(),
            serde_json::Value::String(token),
        );
    }

    Ok(serde_json::Value::Object(settings_object))
}

fn merge_codex_auth_json(
    existing_auth: &serde_json::Value,
    managed_auth: &serde_json::Value,
) -> serde_json::Value {
    let mut merged_auth = existing_auth.as_object().cloned().unwrap_or_default();

    let next_api_key = extract_codex_managed_api_key(managed_auth);

    match next_api_key {
        Some(api_key) => {
            merged_auth.insert(
                "OPENAI_API_KEY".to_string(),
                serde_json::Value::String(api_key),
            );
            merged_auth.insert(
                "auth_mode".to_string(),
                serde_json::Value::String("apikey".to_string()),
            );
        }
        None => {
            merged_auth.remove("OPENAI_API_KEY");
            if merged_auth
                .get("tokens")
                .and_then(|value| value.as_object())
                .is_some_and(|tokens| !tokens.is_empty())
            {
                merged_auth.insert(
                    "auth_mode".to_string(),
                    serde_json::Value::String("chatgpt".to_string()),
                );
            }
        }
    }

    serde_json::Value::Object(merged_auth)
}

fn toml_value_is_subset(target: &toml_edit::Value, source: &toml_edit::Value) -> bool {
    match (target, source) {
        (toml_edit::Value::String(target), toml_edit::Value::String(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Integer(target), toml_edit::Value::Integer(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Float(target), toml_edit::Value::Float(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Boolean(target), toml_edit::Value::Boolean(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Datetime(target), toml_edit::Value::Datetime(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Array(target), toml_edit::Value::Array(source)) => {
            toml_array_contains_subset(target, source)
        }
        (toml_edit::Value::InlineTable(target), toml_edit::Value::InlineTable(source)) => {
            source.iter().all(|(key, source_item)| {
                target
                    .get(key)
                    .is_some_and(|target_item| toml_value_is_subset(target_item, source_item))
            })
        }
        _ => false,
    }
}

fn toml_array_contains_subset(target: &toml_edit::Array, source: &toml_edit::Array) -> bool {
    let mut matched = vec![false; target.len()];
    let target_items: Vec<&toml_edit::Value> = target.iter().collect();

    source.iter().all(|source_item| {
        if let Some((index, _)) = target_items
            .iter()
            .enumerate()
            .find(|(index, target_item)| {
                !matched[*index] && toml_value_is_subset(target_item, source_item)
            })
        {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn toml_remove_array_items(target: &mut toml_edit::Array, source: &toml_edit::Array) {
    for source_item in source.iter() {
        let index = {
            let target_items: Vec<&toml_edit::Value> = target.iter().collect();
            target_items
                .iter()
                .enumerate()
                .find(|(_, target_item)| toml_value_is_subset(target_item, source_item))
                .map(|(index, _)| index)
        };

        if let Some(index) = index {
            target.remove(index);
        }
    }
}

fn remove_toml_item(target: &mut toml_edit::Item, source: &toml_edit::Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            remove_toml_table_like(target_table, source_table);
            if target_table.is_empty() {
                *target = toml_edit::Item::None;
            }
            return;
        }
    }

    if let Some(source_value) = source.as_value() {
        let mut remove_item = false;

        if let Some(target_value) = target.as_value_mut() {
            match (target_value, source_value) {
                (toml_edit::Value::Array(target_array), toml_edit::Value::Array(source_array)) => {
                    toml_remove_array_items(target_array, source_array);
                    remove_item = target_array.is_empty();
                }
                (target_value, source_value)
                    if toml_value_is_subset(target_value, source_value) =>
                {
                    remove_item = true;
                }
                _ => {}
            }
        }

        if remove_item {
            *target = toml_edit::Item::None;
        }
    }
}

fn remove_toml_table_like(
    target: &mut dyn toml_edit::TableLike,
    source: &dyn toml_edit::TableLike,
) {
    let keys: Vec<String> = source.iter().map(|(key, _)| key.to_string()).collect();

    for key in keys {
        let mut remove_key = false;
        if let (Some(target_item), Some(source_item)) = (target.get_mut(&key), source.get(&key)) {
            remove_toml_item(target_item, source_item);
            remove_key = target_item.is_none()
                || target_item
                    .as_table_like()
                    .is_some_and(|table_like| table_like.is_empty());
        }

        if remove_key {
            target.remove(&key);
        }
    }
}

fn strip_codex_common_config_from_toml(
    config_toml: &str,
    common_config_toml: &str,
) -> Result<String, String> {
    if config_toml.trim().is_empty() || common_config_toml.trim().is_empty() {
        return Ok(config_toml.to_string());
    }

    let mut config_document = parse_toml_document(config_toml, "provider config.toml")?;
    let mut common_document = parse_toml_document(common_config_toml, "common config.toml")?;
    strip_protected_top_level_toml_keys(&mut common_document);
    remove_toml_table_like(config_document.as_table_mut(), common_document.as_table());
    Ok(config_document.to_string())
}

fn strip_protected_top_level_sections_from_toml(config_toml: &str) -> Result<String, String> {
    if config_toml.trim().is_empty() {
        return Ok(String::new());
    }

    let mut document = parse_toml_document(config_toml, "config.toml")?;
    strip_protected_top_level_toml_keys(&mut document);
    Ok(document.to_string())
}

fn extract_provider_settings_for_storage(
    settings_value: &serde_json::Value,
    common_config_toml: Option<&str>,
) -> Result<serde_json::Value, String> {
    let restored_settings_value = restore_codex_provider_token_for_storage(settings_value)?;
    let settings_object = restored_settings_value
        .as_object()
        .ok_or_else(|| "Codex settings must be a JSON object".to_string())?;

    let auth_value = settings_object
        .get("auth")
        .and_then(|value| value.as_object())
        .map(|auth_object| {
            let mut managed_auth = serde_json::Map::new();
            if let Some(api_key) = auth_object
                .get("OPENAI_API_KEY")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                managed_auth.insert(
                    "OPENAI_API_KEY".to_string(),
                    serde_json::Value::String(api_key.to_string()),
                );
            }
            serde_json::Value::Object(managed_auth)
        })
        .unwrap_or_else(|| serde_json::json!({}));
    let config_toml = settings_object
        .get("config")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let storage_config_toml = unified_history::strip_unified_session_history_config(config_toml)?;

    let stripped_common_config_toml = if let Some(common_toml) = common_config_toml {
        strip_codex_common_config_from_toml(&storage_config_toml, common_toml)?
    } else {
        storage_config_toml
    };
    let normalized_config_toml =
        strip_protected_top_level_sections_from_toml(&stripped_common_config_toml)?;

    let mut provider_settings = serde_json::json!({
        "auth": auth_value,
        "config": normalized_config_toml,
    });
    if let Some(model_catalog) = normalize_codex_model_catalog_for_storage(settings_object) {
        provider_settings["modelCatalog"] = model_catalog;
    }
    if let Some(auto_review_model_override) =
        resolve_codex_auto_review_model_override(settings_object)
    {
        provider_settings["autoReviewModelOverride"] =
            serde_json::Value::String(auto_review_model_override);
    }

    Ok(provider_settings)
}

fn normalize_codex_optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn resolve_codex_auto_review_model_override(
    settings_object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    if let Some(top_level) = normalize_codex_optional_string(
        settings_object
            .get("autoReviewModelOverride")
            .or_else(|| settings_object.get("auto_review_model_override")),
    ) {
        return Some(top_level);
    }

    let models = settings_object
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .and_then(|models| models.as_array())?;
    for item in models {
        if let Some(legacy) = normalize_codex_optional_string(
            item.get("autoReviewModelOverride")
                .or_else(|| item.get("auto_review_model_override")),
        ) {
            return Some(legacy);
        }
    }

    None
}

fn extract_codex_top_level_model(config_toml: &str) -> Option<String> {
    let document = config_toml.parse::<toml_edit::DocumentMut>().ok()?;
    document
        .get("model")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_codex_model_catalog_for_storage(
    settings_object: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let models = settings_object
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .and_then(|models| models.as_array())?;
    let mut seen_keys = BTreeSet::new();
    let mut normalized_models = Vec::new();

    for item in models {
        let Some(model) = item
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|model| !model.is_empty())
        else {
            continue;
        };
        let display_name = item
            .get("displayName")
            .or_else(|| item.get("display_name"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        // Dedup by (model, displayName) so the same request model can appear
        // under multiple menu display names; fully identical rows still collapse.
        let dedup_key = (model.to_string(), display_name.clone());
        if !seen_keys.insert(dedup_key) {
            continue;
        }

        let mut normalized_item = serde_json::Map::new();
        normalized_item.insert(
            "model".to_string(),
            serde_json::Value::String(model.to_string()),
        );

        if let Some(display_name) = display_name.as_deref() {
            normalized_item.insert(
                "displayName".to_string(),
                serde_json::Value::String(display_name.to_string()),
            );
        }

        if let Some(context_window) = parse_codex_positive_u64(
            item.get("contextWindow")
                .or_else(|| item.get("context_window")),
        ) {
            normalized_item.insert(
                "contextWindow".to_string(),
                serde_json::Value::Number(serde_json::Number::from(context_window)),
            );
        }

        for key in ["supportsImage", "vision", "attachment"] {
            if let Some(value) = item.get(key).and_then(|value| value.as_bool()) {
                normalized_item.insert(key.to_string(), serde_json::Value::Bool(value));
            }
        }

        if let Some(modalities) = normalize_codex_model_catalog_modalities(item.get("modalities")) {
            normalized_item.insert("modalities".to_string(), modalities);
        }

        // Per-model reasoning-level override: preserve user-declared levels
        // (camelCase only — DB is the SSOT). Unknown values are dropped later
        // at catalog-generation time; here we only keep non-empty arrays so
        // the storage stays compact and round-trips cleanly.
        let reasoning_levels = item
            .get("reasoningLevels")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|level| !level.is_empty())
                    .map(|level| serde_json::Value::String(level.to_string()))
                    .collect::<Vec<_>>()
            })
            .filter(|levels| !levels.is_empty());
        if let Some(levels) = reasoning_levels {
            normalized_item.insert(
                "reasoningLevels".to_string(),
                serde_json::Value::Array(levels),
            );
        }

        let default_reasoning_level = item
            .get("defaultReasoningLevel")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|level| !level.is_empty())
            .map(|level| serde_json::Value::String(level.to_string()));
        if let Some(default_level) = default_reasoning_level {
            normalized_item.insert("defaultReasoningLevel".to_string(), default_level);
        }

        // Per-model service (speed) tier ids: preserve user-declared ids
        // (camelCase only — DB is the SSOT). Unknown ids are dropped later at
        // catalog-generation time; here we only keep non-empty arrays so the
        // storage stays compact and round-trips cleanly. Reuses the shared
        // `normalize_codex_model_catalog_string_array` helper (same path as
        // modalities) instead of inlining the trim/filter/collect chain.
        if let Some(items) = normalize_codex_model_catalog_string_array(item.get("serviceTiers")) {
            normalized_item.insert("serviceTiers".to_string(), items);
        }

        normalized_models.push(serde_json::Value::Object(normalized_item));
    }

    if normalized_models.is_empty() {
        return None;
    }

    Some(serde_json::json!({ "models": normalized_models }))
}

fn normalize_codex_model_catalog_modalities(value: Option<&serde_json::Value>) -> Option<Value> {
    let object = value?.as_object()?;
    let mut normalized = serde_json::Map::new();

    for key in ["input", "output"] {
        if let Some(items) = normalize_codex_model_catalog_string_array(object.get(key)) {
            normalized.insert(key.to_string(), items);
        }
    }

    if normalized.is_empty() {
        return None;
    }

    Some(Value::Object(normalized))
}

fn normalize_codex_model_catalog_string_array(value: Option<&serde_json::Value>) -> Option<Value> {
    let items = value?.as_array()?;
    let normalized: Vec<Value> = items
        .iter()
        .filter_map(|item| item.as_str().map(str::trim))
        .filter(|item| !item.is_empty())
        .map(|item| Value::String(item.to_string()))
        .collect();

    if normalized.is_empty() {
        return None;
    }

    Some(Value::Array(normalized))
}

fn strip_protected_top_level_toml_keys(document: &mut toml_edit::DocumentMut) {
    for protected_key in PROTECTED_TOP_LEVEL_TOML_KEYS {
        document.as_table_mut().remove(protected_key);
    }

    let mut remove_features_table = false;
    if let Some(features_item) = document.as_table_mut().get_mut("features") {
        if let Some(features_table) = features_item.as_table_like_mut() {
            for protected_key in PROTECTED_FEATURE_TOML_KEYS {
                features_table.remove(protected_key);
            }
            remove_features_table = features_table.is_empty();
        }
    }
    if remove_features_table {
        document.as_table_mut().remove("features");
    }
}

fn merge_toml_tables(base: &mut toml_edit::Table, overlay: &toml_edit::Table) {
    for (key, overlay_item) in overlay.iter() {
        if let Some(base_item) = base.get_mut(key) {
            merge_toml_items(base_item, overlay_item);
        } else {
            base.insert(key, overlay_item.clone());
        }
    }
}

fn merge_toml_items(base: &mut toml_edit::Item, overlay: &toml_edit::Item) {
    match (base, overlay) {
        (toml_edit::Item::Table(base_table), toml_edit::Item::Table(overlay_table)) => {
            merge_toml_tables(base_table, overlay_table);
        }
        (base_item, overlay_item) => {
            *base_item = overlay_item.clone();
        }
    }
}

fn remove_managed_toml_fields(
    current_table: &mut toml_edit::Table,
    previous_table: &toml_edit::Table,
    preserve_protected_top_level_keys: bool,
) {
    let mut keys_to_remove = Vec::new();

    for (key, previous_item) in previous_table.iter() {
        let key_name = key.to_string();
        if preserve_protected_top_level_keys
            && PROTECTED_TOP_LEVEL_TOML_KEYS.contains(&key_name.as_str())
        {
            continue;
        }

        let should_remove_key = if let Some(current_item) = current_table.get_mut(key) {
            match previous_item {
                toml_edit::Item::Table(previous_child_table) => {
                    if let Some(current_child_table) = current_item.as_table_mut() {
                        remove_managed_toml_fields(
                            current_child_table,
                            previous_child_table,
                            false,
                        );
                        current_child_table.is_empty()
                    } else {
                        true
                    }
                }
                _ => true,
            }
        } else {
            false
        };

        if should_remove_key {
            keys_to_remove.push(key.to_string());
        }
    }

    for key in keys_to_remove {
        current_table.remove(&key);
    }
}

fn render_codex_config_document(document: &toml_edit::DocumentMut) -> String {
    let document_content = document.to_string();
    if document_content.trim_start().starts_with("#:schema") {
        document_content
    } else {
        format!("#:schema none\n{}", document_content)
    }
}

fn build_written_codex_config_toml(
    existing_config_toml: &str,
    previous_managed_config_toml: Option<&str>,
    next_managed_config_toml: &str,
) -> Result<String, String> {
    let mut current_document = parse_toml_document(existing_config_toml, "existing config.toml")?;
    let mut next_managed_document =
        parse_toml_document(next_managed_config_toml, "new config.toml")?;
    strip_protected_top_level_toml_keys(&mut next_managed_document);

    if let Some(previous_managed_config_toml) = previous_managed_config_toml {
        let mut previous_managed_document =
            parse_toml_document(previous_managed_config_toml, "previous managed config.toml")?;
        strip_protected_top_level_toml_keys(&mut previous_managed_document);
        remove_managed_toml_fields(
            current_document.as_table_mut(),
            previous_managed_document.as_table(),
            true,
        );
    }

    merge_toml_tables(
        current_document.as_table_mut(),
        next_managed_document.as_table(),
    );

    Ok(render_codex_config_document(&current_document))
}

fn build_managed_codex_config(
    provider_settings_config: &str,
    common_toml: Option<&str>,
) -> Result<String, String> {
    let provider_config = parse_codex_settings_config(provider_settings_config)?;
    let provider_toml = provider_config
        .get("config")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    let merged_toml = if let Some(common_toml) = common_toml {
        if !common_toml.trim().is_empty() {
            append_toml_configs(provider_toml, common_toml)?
        } else {
            provider_toml.to_string()
        }
    } else {
        provider_toml.to_string()
    };

    let mut managed_document = parse_toml_document(&merged_toml, "managed config")?;
    strip_protected_top_level_toml_keys(&mut managed_document);
    Ok(managed_document.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexCatalogModelSpec {
    model: String,
    /// Explicit user value only (falls back to `model` for the neutral
    /// template; official vendor catalog entries keep their own display name).
    display_name: Option<String>,
    /// Explicit user value only (falls back to the config's top-level
    /// `model_context_window` or the default for the neutral template; official
    /// vendor catalog entries keep the vendor's declared window).
    context_window: Option<u64>,
    auto_review_model_override: Option<String>,
    /// Per-row override for the generated catalog's `supported_reasoning_levels`.
    /// When omitted the neutral template keeps its 6-level default; official
    /// vendor entries keep the vendor's declared levels.
    reasoning_levels: Option<Vec<String>>,
    /// Per-row override for the generated catalog's `default_reasoning_level`.
    /// Only meaningful together with `reasoning_levels`; when absent the
    /// template default is kept if still supported, otherwise the highest
    /// declared level wins.
    default_reasoning_level: Option<String>,
    /// Per-row override for the generated catalog's `service_tiers` (speed
    /// tiers like Fast/Ultrafast). Stored as bare tier ids; the catalog
    /// generator expands them to `{ id, name, description }` objects. When
    /// omitted the neutral template writes an empty array (no tier advertised,
    /// matching cc-switch's safe default); official vendor entries keep the
    /// vendor's declared tiers.
    service_tiers: Option<Vec<String>>,
}

/// Canonical reasoning effort levels Codex understands, in ascending depth
/// order, with the descriptions the official gpt-5.6 template uses. `none`
/// disables thinking. Source: codex-rs/models-manager/models.json.
const CODEX_REASONING_LEVEL_DESCRIPTIONS: &[(&str, &str)] = &[
    ("none", "Disable Thinking"),
    ("minimal", "Minimal reasoning"),
    ("low", "Fast responses with lighter reasoning"),
    (
        "medium",
        "Balances speed and reasoning depth for everyday tasks",
    ),
    ("high", "Greater reasoning depth for complex problems"),
    ("xhigh", "Extra high reasoning depth for complex problems"),
    ("max", "Maximum reasoning depth for the hardest problems"),
    ("ultra", "Maximum reasoning with automatic task delegation"),
];

fn codex_reasoning_level_description(effort: &str) -> Option<&'static str> {
    CODEX_REASONING_LEVEL_DESCRIPTIONS
        .iter()
        .find(|(candidate, _)| *candidate == effort)
        .map(|(_, description)| *description)
}

/// Canonical service (speed) tiers Codex understands, mirroring the entries
/// the official gpt-5.5 catalog declares. Each tier is emitted into a model's
/// `service_tiers` array as `{ id, name, description }`. Only these ids are
/// recognized; user-declared values are filtered to this set so a typo can
/// never produce a tier Codex would reject. Source: codex-rs/models-manager.
const CODEX_SERVICE_TIER_ENTRIES: &[(&str, &str, &str)] = &[
    ("priority", "Fast", "1.5x speed, increased usage"),
    (
        "ultrafast",
        "Ultrafast",
        "The fastest available responses for latency-sensitive work.",
    ),
];

/// Build a `service_tiers` array from user-declared tier ids. Unknown ids are
/// dropped so a typo can never produce a tier Codex would reject; the result
/// preserves canonical order (Fast before Ultrafast) regardless of input order.
fn codex_service_tiers_value(ids: &[String]) -> Value {
    let entries: Vec<Value> = CODEX_SERVICE_TIER_ENTRIES
        .iter()
        .filter(|(id, _, _)| ids.iter().any(|candidate| candidate == id))
        .map(|(id, name, description)| {
            serde_json::json!({ "id": id, "name": name, "description": description })
        })
        .collect();
    serde_json::json!(entries)
}

/// User-declared levels reduced to the canonical efforts Codex understands,
/// in canonical (lowest → highest) order regardless of declaration order.
/// Unknown efforts are dropped so a typo can never produce an entry Codex
/// would reject.
fn codex_canonical_efforts(levels: &[String]) -> Vec<&str> {
    CODEX_REASONING_LEVEL_DESCRIPTIONS
        .iter()
        .filter(|(effort, _)| levels.iter().any(|candidate| candidate == effort))
        .map(|(effort, _)| *effort)
        .collect()
}

/// Build a `supported_reasoning_levels` array from user-declared effort values.
fn codex_supported_reasoning_levels(levels: &[String]) -> Value {
    let entries: Vec<Value> = codex_canonical_efforts(levels)
        .into_iter()
        .map(|effort| {
            let description = codex_reasoning_level_description(effort)
                .expect("canonical effort always has a description");
            serde_json::json!({ "effort": effort, "description": description })
        })
        .collect();
    serde_json::json!(entries)
}

/// Apply a per-model reasoning-level override onto a catalog entry. Returns
/// true when the override was applied. `template_default` is the base entry's
/// `default_reasoning_level` (from the neutral template or an official vendor
/// entry) used as the fallback when the user did not declare one explicitly.
fn apply_codex_reasoning_level_override(
    entry_obj: &mut serde_json::Map<String, Value>,
    template_default: Option<&str>,
    spec: &CodexCatalogModelSpec,
) -> bool {
    let Some(levels) = spec.reasoning_levels.as_deref() else {
        return false;
    };
    let canonical = codex_canonical_efforts(levels);
    if canonical.is_empty() {
        return false;
    }
    let supported = codex_supported_reasoning_levels(levels);
    entry_obj.insert("supported_reasoning_levels".to_string(), supported);

    // Default: explicit user value wins; otherwise keep the template default
    // when it is still supported; otherwise fall back to the highest supported
    // level in canonical order. All candidates are validated against the
    // canonical set so the default can never reference a dropped effort.
    let default_level = spec
        .default_reasoning_level
        .as_deref()
        .filter(|level| canonical.contains(level))
        .or_else(|| template_default.filter(|level| canonical.contains(level)))
        .or_else(|| canonical.last().copied());
    if let Some(default_level) = default_level {
        entry_obj.insert(
            "default_reasoning_level".to_string(),
            serde_json::json!(default_level),
        );
    }
    true
}

fn parse_codex_positive_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number.as_u64().filter(|value| *value > 0),
        Some(Value::String(text)) => text.trim().parse::<u64>().ok().filter(|value| *value > 0),
        _ => None,
    }
}

/// Codex 内置模型目录（codex-rs/models-manager/models.json）中每条 entry
/// 统一使用 272000 作为上下文窗口。当 ai-toolbox 的 provider 设置中
/// 既没有显式填写模型 contextWindow，也没有在 config.toml 设置顶层
/// `model_context_window` 时，以此值兜底，避免用武断的 128k 把模型
/// 上下文窗口压缩到比 Codex 默认更小。来源：codex-rs/models-manager/models.json。
const CODEX_DEFAULT_CONTEXT_WINDOW: u64 = 272_000;

fn extract_codex_top_level_u64(config_toml: &str, field: &str) -> Option<u64> {
    let document = config_toml.parse::<toml_edit::DocumentMut>().ok()?;
    document
        .get(field)
        .and_then(|item| item.as_integer())
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn codex_catalog_model_specs(
    provider_settings_config: &Value,
    config_toml: &str,
) -> Vec<CodexCatalogModelSpec> {
    let settings_object = provider_settings_config.as_object();
    let auto_review_model_override =
        settings_object.and_then(|object| resolve_codex_auto_review_model_override(object));
    let models = provider_settings_config
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .and_then(|models| models.as_array());

    // Dedup user mapping rows by (model, displayName) so the same request
    // model can appear under multiple menu display names (e.g. "luna" and
    // "terra" both pointing at terra). Fully identical rows still collapse.
    let mut seen_keys = BTreeSet::new();
    // Track every model already present in specs (by model id, ignoring
    // display name) so the auto-review fallback below only seeds the default
    // model when it is genuinely absent from the user's mapping.
    let mut seen_models = BTreeSet::new();
    let mut specs = Vec::new();

    if let Some(models) = models {
        for item in models {
            let Some(model) = item
                .get("model")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|model| !model.is_empty())
            else {
                continue;
            };

            let display_name = item
                .get("displayName")
                .or_else(|| item.get("display_name"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);

            let dedup_key = (model.to_string(), display_name.clone());
            if !seen_keys.insert(dedup_key) {
                continue;
            }
            seen_models.insert(model.to_string());

            let context_window = parse_codex_positive_u64(
                item.get("contextWindow")
                    .or_else(|| item.get("context_window")),
            );

            let reasoning_levels = item
                .get("reasoningLevels")
                .or_else(|| item.get("reasoning_levels"))
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(str::trim)
                        .filter(|level| !level.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|levels| !levels.is_empty());
            let default_reasoning_level = item
                .get("defaultReasoningLevel")
                .or_else(|| item.get("default_reasoning_level"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|level| !level.is_empty())
                .map(str::to_string);

            let service_tiers = item
                .get("serviceTiers")
                .or_else(|| item.get("service_tiers"))
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(str::trim)
                        .filter(|tier| !tier.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|tiers| !tiers.is_empty());

            specs.push(CodexCatalogModelSpec {
                model: model.to_string(),
                display_name,
                context_window,
                auto_review_model_override: auto_review_model_override.clone(),
                reasoning_levels,
                default_reasoning_level,
                service_tiers,
            });
        }
    }

    // Auto-review override is provider-level and is read from the *current*
    // session model entry. Always ensure that model exists in the generated
    // catalog (seed when mapping is empty; append when default model is missing
    // from mapping) so Codex can resolve the override.
    if let Some(auto_review_model_override) = auto_review_model_override.as_ref() {
        let default_model = extract_codex_top_level_model(config_toml)
            .unwrap_or_else(|| auto_review_model_override.clone());
        if seen_models.insert(default_model.clone()) {
            specs.push(CodexCatalogModelSpec {
                model: default_model.clone(),
                display_name: None,
                context_window: None,
                auto_review_model_override: Some(auto_review_model_override.clone()),
                reasoning_levels: None,
                default_reasoning_level: None,
                service_tiers: None,
            });
        }
    }

    specs
}

fn codex_model_catalog_entry(
    spec: &CodexCatalogModelSpec,
    index: usize,
    default_context_window: u64,
) -> Value {
    let display_name = spec.display_name.as_deref().unwrap_or(&spec.model);
    let context_window = spec.context_window.unwrap_or(default_context_window);
    let mut entry = serde_json::json!({
        "slug": spec.model.as_str(),
        "display_name": display_name,
        "description": display_name,
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            { "effort": "low", "description": "Fast responses with lighter reasoning" },
            { "effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks" },
            { "effort": "high", "description": "Greater reasoning depth for complex problems" },
            { "effort": "xhigh", "description": "Extra high reasoning depth for complex problems" },
            { "effort": "max", "description": "Maximum reasoning depth for the hardest problems" },
            { "effort": "ultra", "description": "Maximum reasoning with automatic task delegation" }
        ],
        "shell_type": "unified_exec",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1000 + index,
        "base_instructions": "You are Codex, a coding agent. Follow the user's instructions and use tools carefully.",
        "model_messages": {
            "instructions_template": "You are Codex, a coding agent. Follow the user's instructions and use tools carefully.",
            "instructions_variables": {
                "personality_default": "",
                "personality_friendly": "",
                "personality_pragmatic": ""
            }
        },
        "supports_reasoning_summaries": true,
        "default_reasoning_summary": "none",
        "support_verbosity": true,
        "default_verbosity": "low",
        "apply_patch_tool_type": "freeform",
        "web_search_tool_type": "text_and_image",
        "truncation_policy": {
            "mode": "tokens",
            "limit": 10000
        },
        "supports_parallel_tool_calls": true,
        "supports_image_detail_original": true,
        "context_window": context_window,
        "max_context_window": context_window,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": true
    });

    if let Some(auto_review_model_override) = &spec.auto_review_model_override {
        entry["auto_review_model_override"] =
            serde_json::Value::String(auto_review_model_override.clone());
    }

    // Per-model reasoning-level override: when the spec declares levels they
    // replace the neutral template's default set; otherwise the 6-level
    // default above survives untouched.
    if let Some(entry_obj) = entry.as_object_mut() {
        apply_codex_reasoning_level_override(entry_obj, Some("medium"), spec);
        // Per-model service (speed) tiers: when the spec declares tiers they
        // are emitted as full {id,name,description} objects; otherwise the
        // neutral template advertises no speed tier (empty array, matching
        // cc-switch's safe default so a third-party provider never falsely
        // claims a tier it does not honor).
        let service_tiers = spec
            .service_tiers
            .as_deref()
            .map(codex_service_tiers_value)
            .unwrap_or_else(|| serde_json::json!([]));
        entry_obj.insert("service_tiers".to_string(), service_tiers);
    }

    entry
}

fn codex_model_catalog_from_specs(
    specs: &[CodexCatalogModelSpec],
    default_context_window: u64,
) -> Value {
    let models: Vec<Value> = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| codex_model_catalog_entry(spec, index, default_context_window))
        .collect();

    serde_json::json!({ "models": models })
}

/// Hosts whose native `/responses` gateway publishes an OFFICIAL Codex model
/// catalog (models.json) that AI Toolbox mirrors verbatim. Matched against
/// the parsed `base_url` host ONLY — deliberately NOT by model brand: official
/// entries GRANT capabilities (freeform `apply_patch`, vendor harness), and an
/// aggregator merely hosting the same model may not honor them. The safe
/// failure direction for aggregators is the neutral template (degraded but
/// working).
const CODEX_DEEPSEEK_OFFICIAL_CATALOG_HOST: &str = "deepseek.com";

fn codex_deepseek_official_host(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    url.host_str()
        .map(|host| {
            let host = host.to_ascii_lowercase();
            host == CODEX_DEEPSEEK_OFFICIAL_CATALOG_HOST
                || host.ends_with(&format!(".{CODEX_DEEPSEEK_OFFICIAL_CATALOG_HOST}"))
        })
        .unwrap_or(false)
}

/// Bundled copy of DeepSeek's official Codex models.json — the exact file
/// their one-click integration script writes (api-docs.deepseek.com →
/// quick_start/agent_integrations/codex): freeform apply_patch, GPT-5 harness
/// base_instructions, low/high/max reasoning levels, web_search supported,
/// 1m context.
fn load_codex_deepseek_official_catalog_models() -> Vec<Value> {
    let text = include_str!("../../../resources/codex_deepseek_catalog_template.json");
    let catalog: Value =
        serde_json::from_str(text).expect("bundled DeepSeek official catalog must be valid JSON");
    catalog
        .get("models")
        .and_then(|models| models.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Official vendor catalog entries for the provider in `config_toml`, if its
/// gateway ships one. Only the native Responses provider qualifies: the
/// neutral template still powers ProxyChat/Anthropic targets. Host-driven like
/// the web_search blacklist, so existing providers pick it up on their next
/// switch without a re-save.
///
/// Reuses the same TOML semantics as the Gateway runtime
/// (`provider_protocol::codex_wire_api_from_config` /
/// `codex_base_url_from_config`), so the catalog decision cannot diverge from
/// whether the runtime actually proxies a native Responses provider.
fn codex_official_vendor_catalog_models(config_toml: &str) -> Option<Vec<Value>> {
    // Only native `/responses` providers qualify (wire_api/api_format =
    // "responses"); ProxyChat/Anthropic targets keep the neutral template.
    let uses_native_responses = provider_protocol::codex_wire_api_from_config(config_toml)
        .is_some_and(|wire_api| wire_api.eq_ignore_ascii_case("responses"));
    if !uses_native_responses {
        return None;
    }

    let base_url = provider_protocol::codex_base_url_from_config(config_toml)?;
    if !codex_deepseek_official_host(&base_url) {
        return None;
    }

    let models = load_codex_deepseek_official_catalog_models();
    if models.is_empty() {
        return None;
    }
    Some(models)
}

/// Build one catalog entry from an official vendor catalog: match the user's
/// model id against the vendor entries by slug; an unknown id clones the
/// vendor's first (flagship) entry so it keeps the gateway's capability
/// profile without impersonating the flagship. The official entry is
/// authoritative — no tool-profile stripping — but explicit per-row user
/// overrides still win.
fn codex_vendor_catalog_model_entry(
    vendor_models: &[Value],
    spec: &CodexCatalogModelSpec,
    priority: usize,
) -> Value {
    let matched = vendor_models.iter().find(|entry| {
        entry
            .get("slug")
            .and_then(|slug| slug.as_str())
            .is_some_and(|slug| slug.eq_ignore_ascii_case(&spec.model))
    });
    let mut entry = match matched {
        Some(found) => found.clone(),
        None => vendor_models
            .first()
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    };
    let Some(entry_obj) = entry.as_object_mut() else {
        return serde_json::json!({});
    };

    if matched.is_none() && spec.display_name.is_none() {
        // Unknown model id clones the flagship entry for its capability
        // profile, but must NOT impersonate the flagship's display name /
        // description: show the user's own model id instead. An explicit user
        // displayName (handled below) still wins.
        entry_obj.insert("slug".to_string(), serde_json::json!(spec.model));
        entry_obj.insert("display_name".to_string(), serde_json::json!(spec.model));
        entry_obj.insert("description".to_string(), serde_json::json!(spec.model));
    } else if matched.is_none() {
        entry_obj.insert("slug".to_string(), serde_json::json!(spec.model));
    }
    if matched.is_none() {
        entry_obj.insert("priority".to_string(), serde_json::json!(1000 + priority));
    }

    // Explicit user overrides win over the official entry; absent values keep
    // the vendor's declarations (context window, modalities, harness, ...).
    if let Some(display_name) = spec.display_name.as_deref() {
        entry_obj.insert("display_name".to_string(), serde_json::json!(display_name));
        entry_obj.insert("description".to_string(), serde_json::json!(display_name));
    }
    if let Some(context_window) = spec.context_window {
        entry_obj.insert(
            "context_window".to_string(),
            serde_json::json!(context_window),
        );
        entry_obj.insert(
            "max_context_window".to_string(),
            serde_json::json!(context_window),
        );
    }
    if let Some(auto_review_model_override) = spec.auto_review_model_override.as_deref() {
        entry_obj.insert(
            "auto_review_model_override".to_string(),
            serde_json::json!(auto_review_model_override),
        );
    }

    // Per-model reasoning-level override: when the spec declares levels they
    // replace the vendor's declared set (e.g. DeepSeek low/high/max); absent
    // override keeps the official entry verbatim.
    let template_default = entry_obj
        .get("default_reasoning_level")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    apply_codex_reasoning_level_override(entry_obj, template_default.as_deref(), spec);

    // Per-model service (speed) tiers override: when the spec declares tiers
    // they replace the vendor's declared set; absent override keeps the
    // official entry verbatim (DeepSeek entries carry no `service_tiers`
    // array, so codex sees no fast mode unless the user opts in here).
    if let Some(tiers) = spec.service_tiers.as_deref() {
        entry_obj.insert(
            "service_tiers".to_string(),
            codex_service_tiers_value(tiers),
        );
    }

    // Defensive: if a future codex parser requires a field the vendor file
    // predates, backfill only whitelisted parser-required keys.
    fill_template_fields_from_static(&mut entry);
    entry
}

fn codex_vendor_catalog_from_specs(
    specs: &[CodexCatalogModelSpec],
    vendor_models: &[Value],
) -> Value {
    let models: Vec<Value> = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| codex_vendor_catalog_model_entry(vendor_models, spec, index))
        .collect();

    serde_json::json!({ "models": models })
}

/// Fields Codex's external-catalog parser REQUIRES (no serde default): when
/// one is missing Codex rejects the whole catalog file at startup ("missing
/// field ..."). The neutral template and the official vendor entries both
/// carry them; this backfill is a defensive net for vendor files that predate
/// a newer parser.
fn fill_template_fields_from_static(entry: &mut serde_json::Value) {
    let Some(entry_obj) = entry.as_object_mut() else {
        return;
    };
    if !entry_obj.contains_key("shell_type") {
        entry_obj.insert("shell_type".to_string(), serde_json::json!("unified_exec"));
    }
    if !entry_obj.contains_key("visibility") {
        entry_obj.insert("visibility".to_string(), serde_json::json!("list"));
    }
    if !entry_obj.contains_key("supported_in_api") {
        entry_obj.insert("supported_in_api".to_string(), serde_json::json!(true));
    }
}

fn set_codex_model_catalog_json_field(
    config_toml: &str,
    should_write_catalog: bool,
) -> Result<String, String> {
    let mut document = parse_toml_document(config_toml, "managed config")?;

    if should_write_catalog {
        document["model_catalog_json"] = toml_edit::value(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME);
    } else {
        let should_remove = document
            .get("model_catalog_json")
            .and_then(|item| item.as_str())
            .map(|path| {
                Path::new(path).file_name().and_then(|name| name.to_str())
                    == Some(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME)
            })
            .unwrap_or(false);
        if should_remove {
            document.as_table_mut().remove("model_catalog_json");
        }
    }

    Ok(document.to_string())
}

fn prepare_codex_config_with_model_catalog(
    config_dir: &Path,
    provider_settings_config: Option<&Value>,
    config_toml: &str,
) -> Result<String, String> {
    let specs = provider_settings_config
        .map(|settings| codex_catalog_model_specs(settings, config_toml))
        .unwrap_or_default();

    if specs.is_empty() {
        return set_codex_model_catalog_json_field(config_toml, false);
    }

    let default_context_window = extract_codex_top_level_u64(config_toml, "model_context_window")
        .unwrap_or(CODEX_DEFAULT_CONTEXT_WINDOW);
    // DeepSeek publishes an OFFICIAL Codex models.json for its native
    // `/responses` gateway (freeform apply_patch, GPT-5 harness, 1m context,
    // low/high/max reasoning). Mirror it verbatim instead of the neutral
    // template: the harness tells the model to use apply_patch, so stripping
    // the tool while keeping the harness would be self-inconsistent.
    let catalog = codex_official_vendor_catalog_models(config_toml)
        .map(|vendor_models| codex_vendor_catalog_from_specs(&specs, &vendor_models))
        .unwrap_or_else(|| codex_model_catalog_from_specs(&specs, default_context_window));
    let catalog_path = config_dir.join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME);
    let catalog_content = serde_json::to_string_pretty(&catalog)
        .map_err(|e| format!("Failed to serialize Codex model catalog: {}", e))?;
    fs::write(&catalog_path, catalog_content)
        .map_err(|e| format!("Failed to write Codex model catalog: {}", e))?;

    set_codex_model_catalog_json_field(config_toml, true)
}

async fn get_managed_codex_config_for_provider(
    db: &crate::db::SqliteDbState,
    provider_settings_config: &str,
) -> Result<String, String> {
    let common_toml = get_codex_common_toml(db).await?;
    build_managed_codex_config(provider_settings_config, common_toml.as_deref())
}

async fn get_managed_codex_config_for_provider_cleanup(
    db: &crate::db::SqliteDbState,
    provider: &CodexProvider,
) -> Result<String, String> {
    let provider_config = parse_codex_settings_config(&provider.settings_config)?;
    let auth = provider_config
        .get("auth")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let managed_config =
        get_managed_codex_config_for_provider(db, &provider.settings_config).await?;
    let projected_config = project_codex_auth_to_runtime_config(&managed_config, &auth, true)?;
    if provider.category == "official" && load_codex_unified_session_history_enabled(db)? {
        unified_history::inject_unified_session_history_config(&projected_config)
    } else {
        Ok(projected_config)
    }
}

async fn get_managed_codex_config_for_provider_cleanup_with_unified_history(
    db: &crate::db::SqliteDbState,
    provider: &CodexProvider,
    unified_history_enabled: bool,
) -> Result<String, String> {
    let provider_config = parse_codex_settings_config(&provider.settings_config)?;
    let auth = provider_config
        .get("auth")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let managed_config =
        get_managed_codex_config_for_provider(db, &provider.settings_config).await?;
    let projected_config = project_codex_auth_to_runtime_config(&managed_config, &auth, true)?;
    if provider.category == "official" && unified_history_enabled {
        unified_history::inject_unified_session_history_config(&projected_config)
    } else {
        Ok(projected_config)
    }
}

async fn get_current_applied_managed_codex_config(
    db: &crate::db::SqliteDbState,
) -> Result<Option<String>, String> {
    let Some(applied_provider) = get_applied_codex_provider(db).await? else {
        return Ok(None);
    };

    Ok(Some(
        get_managed_codex_config_for_provider_cleanup(db, &applied_provider).await?,
    ))
}

/// 修复损坏的 Codex provider 数据
/// 删除所有 provider 记录，需要重新创建
#[tauri::command]
pub async fn repair_codex_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    let db = state.db();
    db.with_conn(|conn| db_delete_all(conn, DbTable::CodexProvider).map(|_| ()))?;

    Ok("All Codex providers have been deleted. Please recreate them.".to_string())
}

/// Create a new Codex provider
#[tauri::command]
pub async fn create_codex_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: CodexProviderInput,
) -> Result<CodexProvider, String> {
    create_codex_provider_inner(&state, &app, provider).await
}

/// Pure async core of `create_codex_provider`, callable in-process (e.g. from
/// the deep-link import path) without a `tauri::State` wrapper.
pub async fn create_codex_provider_inner<R: tauri::Runtime>(
    state: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider: CodexProviderInput,
) -> Result<CodexProvider, String> {
    let db = state.db();
    let normalized_settings_config =
        normalize_provider_settings_for_storage(&db, &provider.settings_config, None).await?;

    let now = Local::now().to_rfc3339();
    let content = CodexProviderContent {
        name: provider.name,
        category: provider.category,
        settings_config: normalized_settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        meta: provider.meta,
        is_applied: false,
        is_disabled: provider.is_disabled.unwrap_or(false),
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_id = db_new_id();
    put_codex_provider_to_sqlite(db, &provider_id, &content)?;

    // Notify to refresh tray menu
    let _ = app.emit("config-changed", "window");

    Ok(CodexProvider {
        id: provider_id,
        name: content.name,
        category: content.category,
        settings_config: content.settings_config,
        source_provider_id: content.source_provider_id,
        website_url: content.website_url,
        notes: content.notes,
        icon: content.icon,
        icon_color: content.icon_color,
        sort_index: content.sort_index,
        meta: content.meta,
        is_applied: content.is_applied,
        is_disabled: content.is_disabled,
        created_at: content.created_at,
        updated_at: content.updated_at,
    })
}

/// Update an existing Codex provider
#[tauri::command]
pub async fn update_codex_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider: CodexProvider,
) -> Result<CodexProvider, String> {
    let db = state.db();
    let normalized_settings_config =
        normalize_provider_settings_for_storage(&db, &provider.settings_config, None).await?;

    // Use the id from frontend (pure string id without table prefix)
    let id = provider.id.clone();
    let now = Local::now().to_rfc3339();

    // Get existing record to preserve created_at
    let existing_provider = get_codex_provider_from_sqlite(db, &id)?
        .ok_or_else(|| format!("Codex provider with ID '{}' not found", id))?;
    if provider.category != "official" && codex_provider_has_official_accounts(&db, &id).await? {
        return Err(
            "This provider still has official accounts. Delete them before switching the provider away from official mode"
                .to_string(),
        );
    }

    // Get created_at and is_disabled from existing record
    let (created_at, existing_is_disabled) = if !provider.created_at.is_empty() {
        (provider.created_at.clone(), existing_provider.is_disabled)
    } else {
        (
            existing_provider.created_at.clone(),
            existing_provider.is_disabled,
        )
    };

    let previous_managed_config_toml = if provider.is_applied {
        Some(get_managed_codex_config_for_provider_cleanup(&db, &existing_provider).await?)
    } else {
        None
    };

    let content = CodexProviderContent {
        name: provider.name,
        category: provider.category,
        settings_config: normalized_settings_config,
        source_provider_id: provider.source_provider_id,
        website_url: provider.website_url,
        notes: provider.notes,
        icon: provider.icon,
        icon_color: provider.icon_color,
        sort_index: provider.sort_index,
        meta: provider.meta,
        is_applied: provider.is_applied,
        is_disabled: existing_is_disabled,
        created_at,
        updated_at: now,
    };

    put_codex_provider_to_sqlite(db, &id, &content)?;

    // If this provider is applied, re-apply to config file
    if content.is_applied {
        if let Err(e) = apply_config_to_file_with_previous_managed_config(
            &db,
            &id,
            previous_managed_config_toml,
        )
        .await
        {
            eprintln!("Failed to auto-apply updated config: {}", e);
        } else {
            #[cfg(target_os = "windows")]
            let _ = app.emit("wsl-sync-request-codex", ());
        }
    }

    // Notify frontend and tray to refresh
    let _ = app.emit("config-changed", "window");

    Ok(CodexProvider {
        id,
        name: content.name,
        category: content.category,
        settings_config: content.settings_config,
        source_provider_id: content.source_provider_id,
        website_url: content.website_url,
        notes: content.notes,
        icon: content.icon,
        icon_color: content.icon_color,
        sort_index: content.sort_index,
        meta: content.meta,
        is_applied: content.is_applied,
        is_disabled: content.is_disabled,
        created_at: content.created_at,
        updated_at: content.updated_at,
    })
}

/// Delete a Codex provider
#[tauri::command]
pub async fn delete_codex_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    if id == CODEX_LOCAL_PROVIDER_ID {
        return Err("Local Codex provider must be saved before it can be deleted".to_string());
    }
    let db = state.db();
    ensure_codex_provider_has_no_official_accounts(&db, &id).await?;

    delete_codex_provider_from_sqlite(db, &id)?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Reorder Codex providers
#[tauri::command]
pub async fn reorder_codex_providers(
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    if ids.iter().any(|id| id == CODEX_LOCAL_PROVIDER_ID) {
        return Err("Local Codex provider must be saved before it can be reordered".to_string());
    }
    let db = state.db();
    let now = Local::now().to_rfc3339();

    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CodexProvider,
                id,
                &[
                    (
                        "sort_index",
                        serde_json::Value::Number((index as i64).into()),
                    ),
                    ("updated_at", serde_json::Value::String(now.clone())),
                ],
            )
            .map(|_| ())
        })?;
    }

    Ok(())
}

/// Select a Codex provider and mark it as applied in SQLite.
#[tauri::command]
pub async fn select_codex_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    if id == CODEX_LOCAL_PROVIDER_ID {
        return Err("Local Codex provider must be saved before it can be selected".to_string());
    }
    if codex_gateway_takeover_active(&app) {
        return Err(
            "当前 Codex 已由网关接管，请通过网关代理切换入口切换渠道，或先恢复直连".to_string(),
        );
    }
    let db = state.db();
    let provider = query_codex_provider_by_id(&db, &id).await?;
    apply_config_internal(&db, &app, &id, false).await?;
    if provider.category == "official" {
        sync_codex_official_account_apply_status(&db, &id).await?;
    } else {
        clear_all_codex_official_account_apply_status(&db).await?;
    }
    Ok(())
}

/// Internal function: update is_applied status
async fn update_is_applied_status(
    db: &crate::db::SqliteDbState,
    target_id: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    let target_id = target_id.to_string(); // Clone for bind

    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CodexProvider, Some(&target_id), &now)
    })?;

    Ok(())
}

// ============================================================================
// Codex Config File Commands
// ============================================================================

/// Internal function: apply provider config to files
async fn apply_config_to_file(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_to_file_with_previous_managed_config(db, provider_id, None).await
}

/// Public version for tray module
pub async fn apply_config_to_file_public(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_to_file_with_previous_managed_config(db, provider_id, None).await
}

async fn apply_config_to_file_with_previous_managed_config(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
    previous_managed_config_toml: Option<String>,
) -> Result<(), String> {
    let previous_managed_config_toml = match previous_managed_config_toml {
        Some(config) => Some(config),
        None => get_current_applied_managed_codex_config(db).await?,
    };

    let provider = query_codex_provider_by_id(db, provider_id).await?;

    // Check if provider is disabled
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider_id
        ));
    }

    // Parse provider settings_config
    let provider_config = parse_codex_settings_config(&provider.settings_config)?;

    let common_toml = get_codex_common_toml(db).await?;

    // Extract auth and config
    let auth = provider_config
        .get("auth")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let auth_preservation_enabled = load_codex_auth_preservation_enabled(db)?;
    let preserve_official_auth =
        should_preserve_codex_official_auth(&provider, auth_preservation_enabled);
    let managed_config =
        build_managed_codex_config(&provider.settings_config, common_toml.as_deref())?;
    let mut final_config =
        project_codex_auth_to_runtime_config(&managed_config, &auth, preserve_official_auth)?;
    if provider.category == "official" && load_codex_unified_session_history_enabled(db)? {
        final_config = unified_history::inject_unified_session_history_config(&final_config)?;
    }

    write_codex_config_files(
        Some(db),
        &auth,
        previous_managed_config_toml.as_deref(),
        &final_config,
        Some(&provider_config),
        preserve_official_auth,
    )
    .await?;
    Ok(())
}

/// Merge common defaults with provider config while preserving explicit provider overrides.
fn append_toml_configs(provider: &str, common: &str) -> Result<String, String> {
    let provider_content = provider.trim();
    let common_content = common.trim();

    if provider_content.is_empty() {
        return Ok(common_content.to_string());
    }
    if common_content.is_empty() {
        return Ok(provider_content.to_string());
    }

    let provider_doc = parse_toml_document(provider_content, "provider config")?;
    let mut common_doc = parse_toml_document(common_content, "common config")?;

    merge_toml_tables(common_doc.as_table_mut(), provider_doc.as_table());
    Ok(common_doc.to_string())
}

/// Write auth.json and config.toml files
async fn write_codex_config_files(
    db: Option<&crate::db::SqliteDbState>,
    managed_auth: &serde_json::Value,
    previous_managed_config_toml: Option<&str>,
    next_managed_config_toml: &str,
    model_catalog_settings: Option<&serde_json::Value>,
    preserve_official_auth: bool,
) -> Result<(), String> {
    let config_dir = if let Some(db) = db {
        get_codex_config_dir_from_db_async(db).await?
    } else {
        get_codex_config_dir()?
    };

    // Ensure directory exists
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create .codex directory: {}", e))?;
    }

    // Replace only AI Toolbox-managed auth fields and keep runtime-owned OAuth data.
    let auth_path = config_dir.join("auth.json");
    let existing_auth = if auth_path.exists() {
        let existing_auth_content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        serde_json::from_str(&existing_auth_content)
            .map_err(|e| format!("Failed to parse auth.json: {}", e))?
    } else {
        serde_json::json!({})
    };
    let empty_auth = serde_json::json!({});
    let auth_to_write = if preserve_official_auth {
        &empty_auth
    } else {
        managed_auth
    };
    let merged_auth = merge_codex_auth_json(&existing_auth, auth_to_write);
    let auth_content = serde_json::to_string_pretty(&merged_auth)
        .map_err(|e| format!("Failed to serialize auth: {}", e))?;
    fs::write(&auth_path, auth_content).map_err(|e| format!("Failed to write auth.json: {}", e))?;

    // Replace previous AI Toolbox managed config while preserving runtime-owned sections.
    let config_path = config_dir.join("config.toml");
    let existing_config_toml = if config_path.exists() {
        fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.toml: {}", e))?
    } else {
        String::new()
    };
    let existing_config_toml =
        unified_history::strip_unified_session_history_config(&existing_config_toml)?;
    let has_model_catalog = model_catalog_settings
        .map(|settings| !codex_catalog_model_specs(settings, next_managed_config_toml).is_empty())
        .unwrap_or(false);
    let next_managed_config_toml = prepare_codex_config_with_model_catalog(
        &config_dir,
        model_catalog_settings,
        next_managed_config_toml,
    )?;
    let mut final_content = build_written_codex_config_toml(
        &existing_config_toml,
        previous_managed_config_toml,
        &next_managed_config_toml,
    )?;
    if !has_model_catalog {
        final_content = set_codex_model_catalog_json_field(&final_content, false)?;
    }
    fs::write(config_path, final_content)
        .map_err(|e| format!("Failed to write config.toml: {}", e))?;

    Ok(())
}

/// Apply Codex config to files
#[tauri::command]
pub async fn apply_codex_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    if provider_id == CODEX_LOCAL_PROVIDER_ID {
        return Err("Local Codex provider must be saved before it can be applied".to_string());
    }
    if codex_gateway_takeover_active(&app) {
        return Err(
            "当前 Codex 已由网关接管，请通过网关代理切换入口切换渠道，或先恢复直连".to_string(),
        );
    }
    let db = state.db();
    ensure_codex_provider_native_for_direct(&db, &provider_id)?;
    apply_config_internal(&db, &app, &provider_id, false).await
}

/// Toggle is_disabled status for a provider
#[tauri::command]
pub async fn toggle_codex_provider_disabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    if provider_id == CODEX_LOCAL_PROVIDER_ID {
        return Err("Local Codex provider must be saved before it can be changed".to_string());
    }
    let db = state.db();

    // Update is_disabled field in database
    let now = Local::now().to_rfc3339();
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CodexProvider,
            &provider_id,
            &[
                ("is_disabled", serde_json::Value::Bool(is_disabled)),
                ("updated_at", serde_json::Value::String(now.clone())),
            ],
        )
        .map(|_| ())
    })?;

    // If this provider is applied and now disabled, re-apply config to update files
    let provider = query_codex_provider_by_id(&db, &provider_id).await.ok();

    if let Some(provider_value) = provider {
        if provider_value.is_applied {
            // Re-apply config to update files (will check is_disabled internally)
            apply_config_internal(&db, &app, &provider_id, false).await?;
        }
    }

    Ok(())
}

/// Internal function to apply config
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &crate::db::SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_config_internal_with_sync(db, app, provider_id, from_tray, true).await
}

pub async fn apply_config_internal_with_sync<R: tauri::Runtime>(
    db: &crate::db::SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
    emit_sync_request: bool,
) -> Result<(), String> {
    apply_config_internal_with_events(db, app, provider_id, from_tray, true, emit_sync_request)
        .await
}

pub async fn apply_config_internal_without_events<R: tauri::Runtime>(
    db: &crate::db::SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_internal_with_events(db, app, provider_id, false, false, false).await
}

async fn apply_config_internal_with_events<R: tauri::Runtime>(
    db: &crate::db::SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
    emit_config_changed: bool,
    emit_sync_request: bool,
) -> Result<(), String> {
    if provider_id == CODEX_LOCAL_PROVIDER_ID {
        return Err("Local Codex provider must be saved before it can be applied".to_string());
    }
    // Apply config to files
    apply_config_to_file(db, provider_id).await?;

    // Update is_applied status in SQLite.
    update_is_applied_status(db, provider_id).await?;

    if emit_config_changed {
        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
    }

    // Trigger WSL sync via event (Windows only)
    if emit_sync_request {
        #[cfg(target_os = "windows")]
        let _ = app.emit("wsl-sync-request-codex", ());
    }

    record_last_used_best_effort(db, "codex", provider_id);

    Ok(())
}

/// Best-effort "recently used" marker for provider list sorting. Failures must
/// never break the provider apply flow itself.
fn record_last_used_best_effort(db: &crate::db::SqliteDbState, module: &str, provider_id: &str) {
    if let Err(error) =
        crate::settings::provider_list_state::record_provider_last_used_in_sqlite_state(
            db,
            module,
            provider_id,
        )
    {
        log::warn!("Failed to record provider last-used for {module}:{provider_id}: {error}");
    }
}

// ============================================================================
// Codex Prompt Config Commands
// ============================================================================

#[tauri::command]
pub async fn list_codex_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<CodexPromptConfig>, String> {
    let db = state.db();

    let prompts = list_codex_prompts_from_sqlite(db)?;
    if prompts.is_empty() {
        if let Some(local_config) = get_local_prompt_config(Some(db)).await? {
            return Ok(vec![local_config]);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_codex_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPromptConfigInput,
) -> Result<CodexPromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    let next_sort_index = db.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::CodexPromptConfig,
            &JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;

    let content = CodexPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };

    let prompt_id = db_new_id();
    put_codex_prompt_to_sqlite(db, &prompt_id, &content)?;

    let created_config = CodexPromptConfig {
        id: prompt_id,
        name: content.name,
        content: content.content,
        is_applied: content.is_applied,
        sort_index: content.sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    };

    let _ = app.emit("config-changed", "window");

    Ok(created_config)
}

#[tauri::command]
pub async fn update_codex_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPromptConfigInput,
) -> Result<CodexPromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let existing_prompt = get_codex_prompt_from_sqlite(db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    let (created_at, is_applied, sort_index) = {
        let prompt = existing_prompt;
        (
            prompt
                .created_at
                .unwrap_or_else(|| Local::now().to_rfc3339()),
            prompt.is_applied,
            prompt.sort_index,
        )
    };

    let now = Local::now().to_rfc3339();
    let content = CodexPromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied,
        sort_index,
        created_at,
        updated_at: now.clone(),
    };
    put_codex_prompt_to_sqlite(db, &config_id, &content)?;

    if is_applied {
        write_prompt_content_to_file(Some(&db), Some(input.content.as_str())).await?;
        emit_prompt_sync_requests(&app);
    }

    let _ = app.emit("config-changed", "window");

    Ok(CodexPromptConfig {
        id: config_id,
        name: content.name,
        content: content.content,
        is_applied,
        sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(now),
    })
}

#[tauri::command]
pub async fn delete_codex_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    // Only delete the DB prompt record.
    // Keep the live AGENTS.md / prompt file on disk so deleting a saved prompt
    // never wipes the local runtime file. Matches Claude Code / OpenCode.
    let db = state.db();
    delete_codex_prompt_from_sqlite(db, &id)?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub async fn apply_prompt_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_prompt_config_internal_with_events(state, app, config_id, from_tray, true).await
}

pub async fn apply_prompt_config_internal_without_events<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_prompt_config_internal_with_events(state, app, config_id, false, false).await
}

async fn apply_prompt_config_internal_with_events<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
    emit_events: bool,
) -> Result<(), String> {
    if config_id == CODEX_LOCAL_PROVIDER_ID {
        let db = state.db();
        let local_prompt = get_local_prompt_config(Some(&db))
            .await?
            .ok_or_else(|| "Local default prompt not found".to_string())?;
        write_prompt_content_to_file(Some(&db), Some(local_prompt.content.as_str())).await?;

        if emit_events {
            let payload = if from_tray { "tray" } else { "window" };
            let _ = app.emit("config-changed", payload);
            emit_prompt_sync_requests(app);
        }

        return Ok(());
    }

    let db = state.db();
    let prompt_config = get_codex_prompt_from_sqlite(db, config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    let now = Local::now().to_rfc3339();

    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CodexPromptConfig, Some(config_id), &now)
    })?;
    write_prompt_content_to_file(Some(&db), Some(prompt_config.content.as_str())).await?;

    if emit_events {
        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
        emit_prompt_sync_requests(app);
    }

    Ok(())
}

#[tauri::command]
pub async fn apply_codex_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_prompt_config_internal(state, &app, &config_id, false).await
}

fn disable_prompt_config_at_root(
    db: &SqliteDbState,
    config_id: &str,
    root: &Path,
) -> Result<(), String> {
    get_codex_prompt_from_sqlite(db, config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    for file_name in runtime_location::CODEX_PROMPT_FILE_NAMES {
        let prompt_path = root.join(file_name);
        if prompt_path.try_exists().map_err(|error| {
            format!(
                "Failed to inspect Codex prompt file ({}): {error}",
                prompt_path.display()
            )
        })? {
            write_prompt_content_file(&prompt_path, Some(""), "Codex")?;
        }
    }

    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| db_update_applied_status(conn, DbTable::CodexPromptConfig, None, &now))
}

#[tauri::command]
pub async fn disable_codex_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    let root = get_codex_config_dir_from_db_async(db).await?;
    disable_prompt_config_at_root(db, &config_id, &root)?;

    let _ = app.emit("config-changed", "window");
    emit_prompt_sync_requests(&app);
    Ok(())
}

#[tauri::command]
pub async fn reorder_codex_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();

    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CodexPromptConfig,
                id,
                &[(
                    "sort_index",
                    serde_json::Value::Number((index as i64).into()),
                )],
            )
            .map(|_| ())
        })?;
    }

    let _ = db;
    let _ = app.emit("config-changed", "window");

    Ok(())
}

#[tauri::command]
pub async fn save_codex_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexPromptConfigInput,
) -> Result<CodexPromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        let db = state.db();
        get_local_prompt_config(Some(&db))
            .await?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };

    let created = create_codex_prompt_config(
        state.clone(),
        app.clone(),
        CodexPromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;

    apply_prompt_config_internal(state.clone(), &app, &created.id, false).await?;

    let db = state.db();
    Ok(get_codex_prompt_from_sqlite(db, &created.id)?.unwrap_or(created))
}

#[tauri::command]
pub async fn list_codex_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<CodexAllApiHubProvidersResult, String> {
    let _ = state;
    let discovery = all_api_hub::list_provider_candidates()?;

    let providers = discovery
        .providers
        .iter()
        .map(|candidate| CodexAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            npm: Some(candidate.npm.clone()),
            base_url: candidate.base_url.clone(),
            requires_browser_open: candidate
                .auth_type
                .as_deref()
                .map(|value| value.trim().eq_ignore_ascii_case("cookie"))
                .unwrap_or(false),
            is_disabled: candidate.is_disabled,
            has_api_key: candidate
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            api_key_preview: candidate
                .api_key
                .as_ref()
                .map(|value| all_api_hub::mask_api_key_preview(value)),
            balance_usd: candidate.balance_usd,
            balance_cny: candidate.balance_cny,
            site_name: candidate.site_name.clone(),
            site_type: candidate.site_type.clone(),
            account_label: candidate.account_label.clone(),
            source_profile_name: candidate.source_profile_name.clone(),
            source_extension_id: candidate.source_extension_id.clone(),
            provider_config: serde_json::to_value(all_api_hub::candidate_to_opencode_provider(
                candidate,
            ))
            .unwrap_or_else(|_| serde_json::json!({})),
        })
        .collect();

    Ok(CodexAllApiHubProvidersResult {
        found: discovery.found,
        profiles: discovery.profiles,
        providers,
        message: discovery.message,
    })
}

#[tauri::command]
pub async fn resolve_codex_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
    request: ResolveCodexAllApiHubProvidersRequest,
) -> Result<Vec<CodexAllApiHubProvider>, String> {
    let providers =
        all_api_hub::resolve_provider_candidates_with_keys(&state, &request.provider_ids).await?;

    Ok(providers
        .iter()
        .map(|candidate| CodexAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            npm: Some(candidate.npm.clone()),
            base_url: candidate.base_url.clone(),
            requires_browser_open: candidate
                .auth_type
                .as_deref()
                .map(|value| value.trim().eq_ignore_ascii_case("cookie"))
                .unwrap_or(false),
            is_disabled: candidate.is_disabled,
            has_api_key: candidate
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            api_key_preview: candidate
                .api_key
                .as_ref()
                .map(|value| all_api_hub::mask_api_key_preview(value)),
            balance_usd: candidate.balance_usd,
            balance_cny: candidate.balance_cny,
            site_name: candidate.site_name.clone(),
            site_type: candidate.site_type.clone(),
            account_label: candidate.account_label.clone(),
            source_profile_name: candidate.source_profile_name.clone(),
            source_extension_id: candidate.source_extension_id.clone(),
            provider_config: serde_json::to_value(all_api_hub::candidate_to_opencode_provider(
                candidate,
            ))
            .unwrap_or_else(|_| serde_json::json!({})),
        })
        .collect())
}

/// Read current Codex settings from files
#[tauri::command]
pub async fn read_codex_settings(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<CodexSettings, String> {
    let db = state.db();
    read_codex_settings_from_disk(Some(&db)).await
}

#[cfg(test)]
mod tests {
    use super::{
        append_toml_configs, build_written_codex_config_toml, codex_catalog_model_specs,
        extract_codex_common_config_from_settings_toml, extract_provider_settings_for_storage,
        fill_template_fields_from_static, heal_dangling_codex_model_provider,
        infer_codex_provider_category_from_settings, merge_codex_auth_json,
        merge_remote_codex_official_models, normalize_codex_model_tier,
        prepare_codex_config_with_model_catalog, project_codex_auth_to_runtime_config,
        resolve_local_provider_meta, static_codex_official_models,
        strip_codex_common_config_from_toml, CodexHistoryRuntimeSource,
        CodexHistorySourceCandidate, CodexHistorySourceMode, RemoteCodexModel,
        AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME, CODEX_BUILTIN_IMAGE_MODEL_ID,
    };
    use crate::coding::codex::types::CodexProviderInput;
    use crate::coding::codex::unified_history;
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use toml_edit::DocumentMut;

    fn history_source_candidate(
        root: &str,
        source: CodexHistoryRuntimeSource,
    ) -> CodexHistorySourceCandidate {
        CodexHistorySourceCandidate {
            root_dir: PathBuf::from(root),
            source,
            distro: (source == CodexHistoryRuntimeSource::Wsl).then(|| "Ubuntu".to_string()),
        }
    }

    #[test]
    fn disabling_prompt_clears_fallback_files_and_keeps_saved_content() {
        for file_names in [
            vec!["AGENTS.md"],
            vec!["AGENTS.override.md"],
            vec!["AGENTS.md", "AGENTS.override.md"],
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let database = crate::db::SqliteDbState::in_memory_for_test().unwrap();
            let saved = super::CodexPromptConfigContent {
                name: "Saved prompt".to_string(),
                content: "saved instructions".to_string(),
                is_applied: true,
                sort_index: Some(0),
                created_at: "2026-09-06T00:00:00Z".to_string(),
                updated_at: "2026-09-06T00:00:00Z".to_string(),
            };
            super::put_codex_prompt_to_sqlite(&database, "prompt", &saved).unwrap();
            for file_name in &file_names {
                std::fs::write(root.join(file_name), "runtime instructions").unwrap();
            }

            super::disable_prompt_config_at_root(&database, "prompt", root).unwrap();

            for file_name in &file_names {
                assert_eq!(std::fs::read_to_string(root.join(file_name)).unwrap(), "");
            }
            let active_path = super::runtime_location::resolve_codex_prompt_file_path(root);
            assert!(super::read_prompt_content_file(&active_path, "Codex")
                .unwrap()
                .is_none());
            let stored = super::get_codex_prompt_from_sqlite(&database, "prompt")
                .unwrap()
                .unwrap();
            assert!(!stored.is_applied);
            assert_eq!(stored.content, saved.content);
            super::write_prompt_content_file(&active_path, Some(&stored.content), "Codex").unwrap();
            assert_eq!(
                super::read_prompt_content_file(
                    &super::runtime_location::resolve_codex_prompt_file_path(root),
                    "Codex",
                )
                .unwrap(),
                Some(saved.content),
            );
        }
    }

    #[test]
    fn codex_history_source_all_prefers_local_when_available() {
        let candidates = vec![
            history_source_candidate("C:/Users/me/.codex", CodexHistoryRuntimeSource::Local),
            history_source_candidate(
                r"\\wsl.localhost\Ubuntu\home\me\.codex",
                CodexHistoryRuntimeSource::Wsl,
            ),
        ];

        let selected =
            super::select_codex_history_source_candidate(CodexHistorySourceMode::All, &candidates)
                .expect("selected source");

        assert_eq!(selected.source, CodexHistoryRuntimeSource::Local);
        assert_eq!(selected.root_dir, PathBuf::from("C:/Users/me/.codex"));
    }

    #[test]
    fn codex_history_source_all_uses_wsl_when_it_is_the_only_source() {
        let candidates = vec![history_source_candidate(
            r"\\wsl.localhost\Ubuntu\home\me\.codex",
            CodexHistoryRuntimeSource::Wsl,
        )];

        let selected =
            super::select_codex_history_source_candidate(CodexHistorySourceMode::All, &candidates)
                .expect("selected source");

        assert_eq!(selected.source, CodexHistoryRuntimeSource::Wsl);
    }

    #[test]
    fn codex_history_source_wsl_requires_available_wsl_source() {
        let candidates = vec![history_source_candidate(
            "C:/Users/me/.codex",
            CodexHistoryRuntimeSource::Local,
        )];

        let error =
            super::select_codex_history_source_candidate(CodexHistorySourceMode::Wsl, &candidates)
                .expect_err("wsl should be unavailable");

        assert!(error.contains("WSL source is unavailable"));
    }

    #[test]
    fn normalize_codex_model_tier_matches_cli_proxy_plan_mapping() {
        assert_eq!(normalize_codex_model_tier("free"), "free");
        assert_eq!(normalize_codex_model_tier("team"), "team");
        assert_eq!(normalize_codex_model_tier("business"), "team");
        assert_eq!(normalize_codex_model_tier("go"), "team");
        assert_eq!(normalize_codex_model_tier("plus"), "plus");
        assert_eq!(normalize_codex_model_tier("pro"), "pro");
        assert_eq!(normalize_codex_model_tier(""), "pro");
        assert_eq!(normalize_codex_model_tier("unknown"), "pro");
    }

    #[test]
    fn static_codex_official_models_adds_builtin_image_model() {
        let free_models = static_codex_official_models("free");
        let pro_models = static_codex_official_models("pro");

        assert!(free_models
            .iter()
            .any(|model| model.id == CODEX_BUILTIN_IMAGE_MODEL_ID));
        assert!(pro_models
            .iter()
            .any(|model| model.id == CODEX_BUILTIN_IMAGE_MODEL_ID));
        assert!(!free_models
            .iter()
            .any(|model| model.id == "gpt-5.3-codex-spark"));
        assert!(pro_models
            .iter()
            .any(|model| model.id == "gpt-5.3-codex-spark"));
    }

    #[test]
    fn merge_remote_codex_official_models_replaces_remote_builtin_with_local_definition() {
        let models = merge_remote_codex_official_models(vec![
            RemoteCodexModel {
                id: "gpt-5.3-codex".to_string(),
                display_name: Some("GPT 5.3 Codex".to_string()),
                owned_by: Some("openai".to_string()),
                created: None,
            },
            RemoteCodexModel {
                id: CODEX_BUILTIN_IMAGE_MODEL_ID.to_string(),
                display_name: Some("Remote Image".to_string()),
                owned_by: Some("remote".to_string()),
                created: None,
            },
        ]);

        let image_models: Vec<_> = models
            .iter()
            .filter(|model| model.id == CODEX_BUILTIN_IMAGE_MODEL_ID)
            .collect();
        assert_eq!(image_models.len(), 1);
        assert_eq!(image_models[0].name.as_deref(), Some("GPT Image 2"));
        assert_eq!(image_models[0].owned_by.as_deref(), Some("openai"));
    }

    #[test]
    fn append_toml_configs_keeps_common_root_keys_at_root() {
        let provider = r#"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
"#;

        let common = r#"
approval_policy = "never"
sandbox_mode = "danger-full-access"
"#;

        let merged = append_toml_configs(provider, common).unwrap();
        let doc: DocumentMut = merged.parse().unwrap();

        assert_eq!(doc["approval_policy"].as_str(), Some("never"));
        assert_eq!(doc["sandbox_mode"].as_str(), Some("danger-full-access"));
        assert_eq!(
            doc["model_providers"]["custom"]["name"].as_str(),
            Some("custom")
        );
    }

    #[test]
    fn append_toml_configs_merges_common_tables_without_overwriting_provider_table() {
        let provider = r#"
[model_providers.custom]
name = "custom"
"#;

        let common = r#"
[model_providers.custom]
wire_api = "responses"
"#;

        let merged = append_toml_configs(provider, common).unwrap();
        let doc: DocumentMut = merged.parse().unwrap();

        assert_eq!(
            doc["model_providers"]["custom"]["name"].as_str(),
            Some("custom")
        );
        assert_eq!(
            doc["model_providers"]["custom"]["wire_api"].as_str(),
            Some("responses")
        );
    }

    #[test]
    fn append_toml_configs_keeps_explicit_provider_scalar_overrides() {
        let provider = r#"
model = "glm-5.2"
model_reasoning_effort = "high"
"#;
        let common = r#"
model = "gpt-5.4"
model_reasoning_effort = "medium"
approval_policy = "never"
"#;

        let merged = append_toml_configs(provider, common).unwrap();
        let doc: DocumentMut = merged.parse().unwrap();

        assert_eq!(doc["model"].as_str(), Some("glm-5.2"));
        assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(doc["approval_policy"].as_str(), Some("never"));
    }

    #[test]
    fn codex_model_catalog_dedupes_identical_rows_keeps_distinct_display_names() {
        let settings = json!({
            "autoReviewModelOverride": "gpt-5.5",
            "modelCatalog": {
                "models": [
                    {
                        "model": "deepseek-v4-flash",
                        "displayName": "DeepSeek Flash",
                        "contextWindow": "64000",
                        "reasoningLevels": ["low", "high", "max", "ultra"],
                        "defaultReasoningLevel": "max"
                    },
                    {
                        // Fully identical to the first row → collapsed.
                        "model": "deepseek-v4-flash",
                        "displayName": "DeepSeek Flash"
                    },
                    {
                        // Same model, different display name → kept, so the
                        // same upstream model can appear twice in the Codex
                        // model picker under different menu names.
                        "model": "deepseek-v4-flash",
                        "displayName": "Flash Alias"
                    },
                    {
                        "model": "kimi-k2"
                    }
                ]
            }
        });

        let specs = codex_catalog_model_specs(&settings, "model_context_window = 256000");

        // No default model in config.toml, so the provider-level override itself
        // is also seeded as a catalog entry to keep auto-review resolvable.
        assert_eq!(specs.len(), 4);
        assert_eq!(specs[0].model, "deepseek-v4-flash");
        assert_eq!(specs[0].display_name.as_deref(), Some("DeepSeek Flash"));
        assert_eq!(specs[0].context_window, Some(64_000));
        assert_eq!(
            specs[0].auto_review_model_override.as_deref(),
            Some("gpt-5.5")
        );
        // Reasoning fields survive into the spec (canonical filtering happens
        // later at catalog-entry generation).
        assert_eq!(
            specs[0].reasoning_levels,
            Some(vec![
                "low".to_string(),
                "high".to_string(),
                "max".to_string(),
                "ultra".to_string()
            ])
        );
        assert_eq!(specs[0].default_reasoning_level.as_deref(), Some("max"));
        assert_eq!(specs[1].model, "deepseek-v4-flash");
        assert_eq!(specs[1].display_name.as_deref(), Some("Flash Alias"));
        assert_eq!(specs[1].context_window, None);
        assert_eq!(specs[1].reasoning_levels, None);
        assert_eq!(specs[1].default_reasoning_level, None);
        assert_eq!(
            specs[1].auto_review_model_override.as_deref(),
            Some("gpt-5.5")
        );
        assert_eq!(specs[2].model, "kimi-k2");
        assert_eq!(specs[2].display_name, None);
        assert_eq!(specs[2].context_window, None);
        assert_eq!(
            specs[2].auto_review_model_override.as_deref(),
            Some("gpt-5.5")
        );
        assert_eq!(specs[3].model, "gpt-5.5");
        assert_eq!(specs[3].context_window, None);
        assert_eq!(specs[3].reasoning_levels, None);
        assert_eq!(specs[3].default_reasoning_level, None);
        assert_eq!(
            specs[3].auto_review_model_override.as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn codex_model_catalog_applies_per_model_reasoning_override() {
        let settings = json!({
            "modelCatalog": {
                "models": [
                    {
                        "model": "mapped-a",
                        "displayName": "Mapped A",
                        "reasoningLevels": ["max", "ultra", "bogus"],
                        "defaultReasoningLevel": "max"
                    },
                    {
                        // No reasoning override → keeps the neutral template's
                        // 6-level default (low..ultra).
                        "model": "mapped-b"
                    },
                    {
                        // Reasoning levels declared but default points at a
                        // dropped/unknown value → falls back to highest declared.
                        "model": "mapped-c",
                        "displayName": "Mapped C",
                        "reasoningLevels": ["low", "high"],
                        "defaultReasoningLevel": "max"
                    }
                ]
            }
        });

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            "model_provider = \"custom\"",
        )
        .expect("catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

        // Row 0: user override wins; unknown "bogus" dropped; canonical order;
        // explicit default "max" preserved.
        let entry_a = &catalog["models"][0];
        let efforts_a: Vec<&str> = entry_a["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|level| level.get("effort").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(efforts_a, vec!["max", "ultra"]);
        assert_eq!(
            entry_a
                .get("default_reasoning_level")
                .and_then(|v| v.as_str()),
            Some("max")
        );

        // Row 1: no override → neutral 6-level default survives untouched.
        let entry_b = &catalog["models"][1];
        let efforts_b: Vec<&str> = entry_b["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|level| level.get("effort").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            efforts_b,
            vec!["low", "medium", "high", "xhigh", "max", "ultra"]
        );
        assert_eq!(
            entry_b
                .get("default_reasoning_level")
                .and_then(|v| v.as_str()),
            Some("medium")
        );

        // Row 2: default "max" not in declared set → falls back to highest
        // declared ("high").
        let entry_c = &catalog["models"][2];
        let efforts_c: Vec<&str> = entry_c["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|level| level.get("effort").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(efforts_c, vec!["low", "high"]);
        assert_eq!(
            entry_c
                .get("default_reasoning_level")
                .and_then(|v| v.as_str()),
            Some("high")
        );
    }

    #[test]
    fn codex_model_catalog_applies_per_model_service_tiers() {
        let settings = json!({
            "modelCatalog": {
                "models": [
                    {
                        "model": "mapped-a",
                        "displayName": "Mapped A",
                        // Declaration order is ignored; unknown "bogus" dropped;
                        // canonical order (Fast before Ultrafast) is restored.
                        "serviceTiers": ["ultrafast", "priority", "bogus"]
                    },
                    {
                        // No service_tiers declared → neutral template writes
                        // an empty array (no speed tier advertised, matching
                        // cc-switch's safe default for third-party providers).
                        "model": "mapped-b"
                    }
                ]
            }
        });

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            "model_provider = \"custom\"",
        )
        .expect("catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

        // Row 0: user override wins; each tier expands to a full
        // {id,name,description} object in canonical order.
        let entry_a = &catalog["models"][0];
        let tiers_a: Vec<&str> = entry_a["service_tiers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tier| tier.get("id").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(tiers_a, vec!["priority", "ultrafast"]);
        let first = &entry_a["service_tiers"].as_array().unwrap()[0];
        assert_eq!(first["name"].as_str(), Some("Fast"));
        assert_eq!(
            first["description"].as_str(),
            Some("1.5x speed, increased usage")
        );

        // Row 1: no override → empty array (safe default).
        let entry_b = &catalog["models"][1];
        assert!(entry_b["service_tiers"].is_array());
        assert!(entry_b["service_tiers"].as_array().unwrap().is_empty());
    }

    #[test]
    fn codex_model_catalog_shell_type_uses_unified_exec() {
        // Codex #39757 removed the standalone `shell_command` runtime; the
        // value is now only a legacy alias of `unified_exec`. The neutral
        // template must declare the canonical `unified_exec` value the
        // official models.json uses for all current models.
        let settings = json!({
            "modelCatalog": {
                "models": [
                    { "model": "mapped-a", "displayName": "Mapped A" },
                    { "model": "mapped-b" }
                ]
            }
        });

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            "model_provider = \"custom\"",
        )
        .expect("catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

        for entry in catalog["models"].as_array().unwrap() {
            assert_eq!(
                entry.get("shell_type").and_then(|v| v.as_str()),
                Some("unified_exec")
            );
        }
    }

    #[test]
    fn fill_template_fields_backfills_unified_exec_and_preserves_declared_shell_type() {
        // Missing shell_type is backfilled with the canonical `unified_exec`;
        // an already-declared value (e.g. a vendor file still using the
        // legacy alias) must survive untouched.
        let mut missing = json!({ "slug": "vendor-model" });
        fill_template_fields_from_static(&mut missing);
        assert_eq!(
            missing.get("shell_type").and_then(|v| v.as_str()),
            Some("unified_exec")
        );

        let mut declared = json!({ "slug": "vendor-model", "shell_type": "shell_command" });
        fill_template_fields_from_static(&mut declared);
        assert_eq!(
            declared.get("shell_type").and_then(|v| v.as_str()),
            Some("shell_command")
        );
    }

    #[test]
    fn codex_model_catalog_seeds_default_model_when_only_auto_review_override_is_set() {
        let settings = json!({
            "autoReviewModelOverride": "gpt-5.5"
        });

        let specs = codex_catalog_model_specs(&settings, "model = \"gpt-5.5\"");

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].model, "gpt-5.5");
        assert_eq!(
            specs[0].auto_review_model_override.as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn codex_model_catalog_appends_default_model_when_missing_from_mapping() {
        let settings = json!({
            "autoReviewModelOverride": "reviewer-model",
            "modelCatalog": {
                "models": [
                    {
                        "model": "mapped-a",
                        "displayName": "Mapped A"
                    },
                    {
                        "model": "mapped-b"
                    }
                ]
            }
        });

        let specs = codex_catalog_model_specs(
            &settings,
            "model = \"default-main\"\nmodel_context_window = 200000",
        );

        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].model, "mapped-a");
        assert_eq!(
            specs[0].auto_review_model_override.as_deref(),
            Some("reviewer-model")
        );
        assert_eq!(specs[1].model, "mapped-b");
        assert_eq!(
            specs[1].auto_review_model_override.as_deref(),
            Some("reviewer-model")
        );
        assert_eq!(specs[2].model, "default-main");
        assert_eq!(specs[2].display_name, None);
        assert_eq!(specs[2].context_window, None);
        assert_eq!(
            specs[2].auto_review_model_override.as_deref(),
            Some("reviewer-model")
        );
    }

    #[test]
    fn codex_model_catalog_does_not_duplicate_default_model_already_in_mapping() {
        let settings = json!({
            "autoReviewModelOverride": "reviewer-model",
            "modelCatalog": {
                "models": [
                    {
                        "model": "default-main",
                        "displayName": "Default Main",
                        "contextWindow": 64000
                    },
                    {
                        "model": "mapped-b"
                    }
                ]
            }
        });

        let specs = codex_catalog_model_specs(&settings, "model = \"default-main\"");

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].model, "default-main");
        assert_eq!(specs[0].display_name.as_deref(), Some("Default Main"));
        assert_eq!(specs[0].context_window, Some(64_000));
        assert_eq!(
            specs[0].auto_review_model_override.as_deref(),
            Some("reviewer-model")
        );
        assert_eq!(specs[1].model, "mapped-b");
    }

    #[test]
    fn prepare_codex_config_with_model_catalog_writes_relative_pointer() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let settings = json!({
            "autoReviewModelOverride": "gpt-5.5",
            "modelCatalog": {
                "models": [
                    {
                        "model": "deepseek-v4-flash",
                        "displayName": "DeepSeek Flash",
                        "contextWindow": 64000
                    },
                    {
                        "model": "kimi-k2",
                        "displayName": "Kimi K2"
                    }
                ]
            }
        });

        let rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            "model_provider = \"custom\"",
        )
        .unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();
        let catalog_path = temp_dir
            .path()
            .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME);
        let catalog_text = std::fs::read_to_string(catalog_path).unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

        assert_eq!(
            doc["model_catalog_json"].as_str(),
            Some(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME)
        );
        assert_eq!(
            catalog["models"][0]["slug"].as_str(),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            catalog["models"][0]["context_window"].as_u64(),
            Some(64_000)
        );
        assert_eq!(
            catalog["models"][0]["auto_review_model_override"].as_str(),
            Some("gpt-5.5")
        );
        assert!(catalog["models"][0].get("model_messages").is_some());
        // kimi-k2 has no explicit context window; the entry falls back to
        // CODEX_DEFAULT_CONTEXT_WINDOW (272_000), matching Codex's bundled
        // models.json instead of the old hard-coded 128_000.
        assert_eq!(
            catalog["models"][1]["context_window"].as_u64(),
            Some(272_000)
        );
        assert_eq!(
            catalog["models"][1]["max_context_window"].as_u64(),
            Some(272_000)
        );
        assert_eq!(
            catalog["models"][1]["auto_review_model_override"].as_str(),
            Some("gpt-5.5")
        );
    }

    const DEEPSEEK_NATIVE_CONFIG: &str = r#"model = "deepseek-v4-flash"
model_provider = "custom"

[model_providers.custom]
name = "deepseek"
base_url = "https://api.deepseek.com"
wire_api = "responses"
"#;

    #[test]
    fn deepseek_host_native_catalog_mirrors_official_entries() {
        // DeepSeek publishes an official Codex models.json (freeform
        // apply_patch + GPT-5 harness + low/high/max reasoning levels). For a
        // deepseek.com native provider the generated catalog must mirror it
        // verbatim instead of the stripped neutral template — the harness
        // tells the model to use apply_patch, so stripping the tool while
        // keeping the harness would be self-inconsistent.
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let settings = json!({
            "modelCatalog": {
                "models": [
                    { "model": "deepseek-v4-flash", "displayName": "DeepSeek V4 Flash" },
                    { "model": "deepseek-v4-pro", "contextWindow": 500_000 }
                ]
            }
        });

        let _rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            DEEPSEEK_NATIVE_CONFIG,
        )
        .expect("vendor catalog generation should not error");
        let catalog_path = temp_dir
            .path()
            .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME);
        let catalog_text = std::fs::read_to_string(catalog_path).unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

        let flash = &catalog["models"][0];
        assert_eq!(
            flash.get("slug").and_then(|v| v.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            flash.get("apply_patch_tool_type").and_then(|v| v.as_str()),
            Some("freeform"),
            "official DeepSeek entries keep the freeform apply_patch grant"
        );
        assert!(
            flash
                .get("base_instructions")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("Codex")),
            "official harness must survive verbatim"
        );
        let efforts: Vec<&str> = flash["supported_reasoning_levels"]
            .as_array()
            .expect("official reasoning levels array")
            .iter()
            .filter_map(|level| level.get("effort").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(efforts, vec!["low", "high", "max"]);
        assert_eq!(flash.get("input_modalities"), Some(&json!(["text"])));
        assert!(
            flash.get("model_messages").is_some(),
            "official entries are mirrored verbatim, incl. model_messages"
        );
        // No explicit contextWindow on the row: the official 1m window must
        // survive instead of being clobbered by the neutral default.
        assert_eq!(
            flash.get("context_window").and_then(|v| v.as_u64()),
            Some(1_048_576)
        );
        // Explicit user display name still wins over the official one.
        assert_eq!(
            flash.get("display_name").and_then(|v| v.as_str()),
            Some("DeepSeek V4 Flash")
        );

        let pro = &catalog["models"][1];
        assert_eq!(
            pro.get("slug").and_then(|v| v.as_str()),
            Some("deepseek-v4-pro")
        );
        // Matches the official slug, so the official display name survives when
        // the user did not provide an explicit one.
        assert_eq!(
            pro.get("display_name").and_then(|v| v.as_str()),
            Some("DeepSeek-V4-Pro")
        );
        // Explicit user context window override wins over the official 1m.
        assert_eq!(
            pro.get("context_window").and_then(|v| v.as_u64()),
            Some(500_000)
        );
        assert_eq!(
            pro.get("apply_patch_tool_type").and_then(|v| v.as_str()),
            Some("freeform")
        );
    }

    #[test]
    fn deepseek_unknown_model_clones_flagship_capabilities_without_impersonation() {
        // A user model that does not match any official slug (e.g.
        // `deepseek-reasoner`) clones the flagship entry for its capability
        // profile but must NOT keep the flagship's display name / description —
        // it would show the wrong model name in the Codex model selector.
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let settings = json!({
            "modelCatalog": {
                "models": [
                    { "model": "deepseek-reasoner" }
                ]
            }
        });

        let _rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            DEEPSEEK_NATIVE_CONFIG,
        )
        .expect("vendor catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

        let entry = &catalog["models"][0];
        assert_eq!(
            entry.get("slug").and_then(|v| v.as_str()),
            Some("deepseek-reasoner")
        );
        // Must show the user's own model id, not the flagship's name.
        assert_eq!(
            entry.get("display_name").and_then(|v| v.as_str()),
            Some("deepseek-reasoner")
        );
        assert_eq!(
            entry.get("description").and_then(|v| v.as_str()),
            Some("deepseek-reasoner")
        );
        // Capability profile still inherited from the flagship.
        assert_eq!(
            entry.get("apply_patch_tool_type").and_then(|v| v.as_str()),
            Some("freeform")
        );
        assert_eq!(entry.get("input_modalities"), Some(&json!(["text"])));
    }

    #[test]
    fn deepseek_vendor_entry_user_reasoning_override_wins_over_official() {
        // DeepSeek's official catalog declares low/high/max. A per-model
        // reasoningLevels override must replace the vendor's set, and the
        // default falls back to the template default ("high") when the user
        // omits defaultReasoningLevel and it is still canonical.
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let settings = json!({
            "modelCatalog": {
                "models": [
                    {
                        "model": "deepseek-v4-flash",
                        "displayName": "DeepSeek V4 Flash",
                        "reasoningLevels": ["low", "medium", "high", "xhigh", "max", "ultra"]
                    }
                ]
            }
        });

        let _rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            DEEPSEEK_NATIVE_CONFIG,
        )
        .expect("vendor catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

        let entry = &catalog["models"][0];
        let efforts: Vec<&str> = entry["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|level| level.get("effort").and_then(|v| v.as_str()))
            .collect();
        // User's 6-level set replaces the vendor's low/high/max.
        assert_eq!(
            efforts,
            vec!["low", "medium", "high", "xhigh", "max", "ultra"]
        );
        // No explicit user default → template default "high" is still canonical → kept.
        assert_eq!(
            entry
                .get("default_reasoning_level")
                .and_then(|v| v.as_str()),
            Some("high")
        );
    }

    #[test]
    fn non_deepseek_or_non_native_provider_keeps_neutral_template() {
        // Aggregator host or non-Responses target must NOT inherit the
        // official freeform apply_patch / image-free catalog: wrongly granting
        // freeform apply_patch to an aggregator that does not honor it would
        // reintroduce the custom-tool rejection bug.
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let settings = json!({
            "modelCatalog": {
                "models": [
                    { "model": "deepseek-v4-flash", "displayName": "DeepSeek Flash" }
                ]
            }
        });

        // Aggregator host with native responses: neutral template, not mirror.
        let aggregator_config = r#"model = "deepseek-v4-flash"
model_provider = "custom"

[model_providers.custom]
name = "aggregator"
base_url = "https://proxy.example.com/v1"
wire_api = "responses"
"#;
        let _rendered = prepare_codex_config_with_model_catalog(
            temp_dir.path(),
            Some(&settings),
            aggregator_config,
        )
        .expect("aggregator host catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();
        let entry = &catalog["models"][0];
        assert_eq!(
            entry.get("slug").and_then(|v| v.as_str()),
            Some("deepseek-v4-flash")
        );
        // Neutral template: image-friendly input modalities, search tool on.
        assert_eq!(
            entry.get("input_modalities"),
            Some(&json!(["text", "image"]))
        );
        assert_eq!(
            entry.get("web_search_tool_type").and_then(|v| v.as_str()),
            Some("text_and_image")
        );
        assert_eq!(
            entry.get("context_window").and_then(|v| v.as_u64()),
            Some(272_000)
        );

        // DeepSeek host but non-native target (chat): also neutral template.
        let chat_config = r#"model = "deepseek-v4-flash"
model_provider = "custom"

[model_providers.custom]
name = "deepseek"
base_url = "https://api.deepseek.com"
wire_api = "chat"
"#;
        let _rendered =
            prepare_codex_config_with_model_catalog(temp_dir.path(), Some(&settings), chat_config)
                .expect("chat target catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();
        let entry = &catalog["models"][0];
        assert_eq!(
            entry.get("input_modalities"),
            Some(&json!(["text", "image"]))
        );

        // Host that merely CONTAINS "deepseek.com" as a substring is not the
        // official gateway — must stay neutral to avoid granting the official
        // freeform apply_patch capability to an unrelated host.
        let spoof_config = r#"model = "deepseek-v4-flash"
model_provider = "custom"

[model_providers.custom]
name = "deepseek"
base_url = "https://deepseek.com.evil.example/v1"
wire_api = "responses"
"#;
        let _rendered =
            prepare_codex_config_with_model_catalog(temp_dir.path(), Some(&settings), spoof_config)
                .expect("spoof host catalog generation should not error");
        let catalog_text = std::fs::read_to_string(
            temp_dir
                .path()
                .join(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME),
        )
        .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();
        let entry = &catalog["models"][0];
        // Substring host must NOT trigger the official DeepSeek mirror: keep
        // the neutral image-friendly template and the neutral 272k default,
        // not the official text-only modality / 1m window.
        assert_eq!(
            entry.get("input_modalities"),
            Some(&json!(["text", "image"])),
            "substring host must not trigger the official DeepSeek mirror"
        );
        assert_eq!(
            entry.get("context_window").and_then(|v| v.as_u64()),
            Some(272_000),
            "spoof host must not inherit the official 1m context window"
        );
    }

    #[test]
    fn prepare_codex_config_with_empty_catalog_removes_ai_toolbox_pointer() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            "model_catalog_json = \"{}\"\nmodel_provider = \"custom\"",
            AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME
        );

        let rendered =
            prepare_codex_config_with_model_catalog(temp_dir.path(), None, &config).unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();

        assert!(doc.get("model_catalog_json").is_none());
        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
    }

    #[test]
    fn prepare_codex_config_with_empty_catalog_preserves_external_pointer() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config = "model_catalog_json = \"external-catalog.json\"\nmodel_provider = \"custom\"";

        let rendered =
            prepare_codex_config_with_model_catalog(temp_dir.path(), None, config).unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();

        assert_eq!(
            doc["model_catalog_json"].as_str(),
            Some("external-catalog.json")
        );
    }

    #[test]
    fn local_provider_meta_prefers_submitted_billing_meta() {
        let provider_input = CodexProviderInput {
            id: None,
            name: "Codex Gateway".to_string(),
            category: "custom".to_string(),
            settings_config: "{}".to_string(),
            source_provider_id: None,
            website_url: None,
            notes: None,
            icon: None,
            icon_color: None,
            sort_index: None,
            meta: Some(json!({
                "costMultiplier": "1.25",
                "pricingModelSource": "requested"
            })),
            is_disabled: None,
        };
        let base_meta = Some(json!({
            "costMultiplier": "0.75"
        }));

        assert_eq!(
            resolve_local_provider_meta(Some(&provider_input), base_meta),
            Some(json!({
                "costMultiplier": "1.25",
                "pricingModelSource": "requested"
            }))
        );
    }

    #[test]
    fn local_provider_meta_falls_back_to_base_meta() {
        let provider_input = CodexProviderInput {
            id: None,
            name: "Codex Gateway".to_string(),
            category: "custom".to_string(),
            settings_config: "{}".to_string(),
            source_provider_id: None,
            website_url: None,
            notes: None,
            icon: None,
            icon_color: None,
            sort_index: None,
            meta: None,
            is_disabled: None,
        };
        let base_meta = Some(json!({
            "costMultiplier": "0.75",
            "pricingModelSource": "upstream"
        }));

        assert_eq!(
            resolve_local_provider_meta(Some(&provider_input), base_meta.clone()),
            base_meta
        );
    }

    #[test]
    fn build_written_codex_config_toml_replaces_old_managed_fields_but_keeps_plugins_and_mcp() {
        let existing = r#"
#:schema none
model_provider = "old"
approval_policy = "never"

[model_providers.old]
name = "old-provider"

[features]
plugins = true

[plugins."demo@local"]
enabled = true

[mcp_servers.test]
command = "uvx"
"#;

        let previous_managed = r#"
model_provider = "old"
approval_policy = "never"

[model_providers.old]
name = "old-provider"
"#;

        let next_managed = r#"
model_provider = "custom"
sandbox_mode = "danger-full-access"

[model_providers.custom]
name = "new-provider"
"#;

        let rendered =
            build_written_codex_config_toml(existing, Some(previous_managed), next_managed)
                .unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();

        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert_eq!(doc["sandbox_mode"].as_str(), Some("danger-full-access"));
        assert!(doc.get("approval_policy").is_none());
        assert!(doc["model_providers"].get("old").is_none());
        assert_eq!(
            doc["model_providers"]["custom"]["name"].as_str(),
            Some("new-provider")
        );
        assert_eq!(doc["features"]["plugins"].as_bool(), Some(true));
        assert_eq!(
            doc["plugins"]["demo@local"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(doc["mcp_servers"]["test"]["command"].as_str(), Some("uvx"));
    }

    #[test]
    fn build_written_codex_config_toml_manages_common_features_but_keeps_plugin_feature() {
        let existing = r#"
[features]
plugins = true
test_generation = true
runtime_only = true
"#;

        let previous_managed = r#"
[features]
plugins = false
test_generation = true
old_common_feature = true
"#;

        let next_managed = r#"
[features]
plugins = false
test_generation = false
image_generation = false
"#;

        let rendered =
            build_written_codex_config_toml(existing, Some(previous_managed), next_managed)
                .unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();

        assert_eq!(doc["features"]["plugins"].as_bool(), Some(true));
        assert_eq!(doc["features"]["test_generation"].as_bool(), Some(false));
        assert_eq!(doc["features"]["image_generation"].as_bool(), Some(false));
        assert_eq!(doc["features"]["runtime_only"].as_bool(), Some(true));
        assert!(doc["features"].get("old_common_feature").is_none());
    }

    #[test]
    fn build_written_codex_config_toml_keeps_existing_runtime_sections_without_previous_snapshot() {
        let existing = r#"
[features]
plugins = true

[plugins."demo@local"]
enabled = false
"#;

        let next_managed = r#"
model_provider = "custom"
"#;

        let rendered = build_written_codex_config_toml(existing, None, next_managed).unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();

        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert_eq!(doc["features"]["plugins"].as_bool(), Some(true));
        assert_eq!(
            doc["plugins"]["demo@local"]["enabled"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn project_codex_auth_to_runtime_config_writes_provider_scoped_bearer_token() {
        let managed_config = r#"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
base_url = "https://api.example.com/v1"
"#;
        let managed_auth = json!({
            "OPENAI_API_KEY": "sk-third-party"
        });

        let projected =
            project_codex_auth_to_runtime_config(managed_config, &managed_auth, true).unwrap();
        let doc: DocumentMut = projected.parse().unwrap();

        assert_eq!(
            doc["model_providers"]["custom"]["experimental_bearer_token"].as_str(),
            Some("sk-third-party")
        );
        assert!(doc.get("experimental_bearer_token").is_none());
    }

    #[test]
    fn project_codex_auth_to_runtime_config_rejects_missing_provider_table() {
        let managed_config = r#"
model = "gpt-5.4"
"#;
        let managed_auth = json!({
            "OPENAI_API_KEY": "sk-third-party"
        });

        let error =
            project_codex_auth_to_runtime_config(managed_config, &managed_auth, true).unwrap_err();

        assert!(
            error.contains("config.toml has no active model_provider"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn build_written_codex_config_toml_removes_previous_generated_bearer_token() {
        let existing = r#"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
base_url = "https://api.example.com/v1"
experimental_bearer_token = "sk-old"
"#;
        let previous_managed = r#"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
base_url = "https://api.example.com/v1"
experimental_bearer_token = "sk-old"
"#;
        let next_managed = r#"
model = "gpt-5.4"
"#;

        let rendered =
            build_written_codex_config_toml(existing, Some(previous_managed), next_managed)
                .unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();

        assert_eq!(doc["model"].as_str(), Some("gpt-5.4"));
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model_providers").is_none());
    }

    #[test]
    fn merge_codex_auth_json_removes_managed_api_key_but_keeps_runtime_oauth_fields() {
        let existing_auth = json!({
            "OPENAI_API_KEY": "sk-old",
            "auth_mode": "chatgpt",
            "last_refresh": "2026-04-10T00:00:00Z",
            "tokens": {
                "access_token": "access-token",
                "account_id": "account-id"
            }
        });
        let managed_auth = json!({});

        let merged_auth = merge_codex_auth_json(&existing_auth, &managed_auth);

        assert_eq!(merged_auth.get("OPENAI_API_KEY"), None);
        assert_eq!(
            merged_auth
                .get("auth_mode")
                .and_then(|value| value.as_str()),
            Some("chatgpt")
        );
        assert_eq!(
            merged_auth
                .pointer("/tokens/access_token")
                .and_then(|value| value.as_str()),
            Some("access-token")
        );
        assert_eq!(
            merged_auth
                .pointer("/tokens/account_id")
                .and_then(|value| value.as_str()),
            Some("account-id")
        );
    }

    #[test]
    fn merge_codex_auth_json_api_key_mode_keeps_chatgpt_runtime_fields_for_restore() {
        let existing_auth = json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": "sk-old",
            "last_refresh": "2026-04-10T00:00:00Z",
            "tokens": {
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "account_id": "account-id"
            },
            "agent_identity": {
                "workspace_id": "workspace-id"
            }
        });
        let managed_auth = json!({
            "OPENAI_API_KEY": "sk-new"
        });

        let merged_auth = merge_codex_auth_json(&existing_auth, &managed_auth);

        assert_eq!(
            merged_auth
                .get("OPENAI_API_KEY")
                .and_then(|value| value.as_str()),
            Some("sk-new")
        );
        assert_eq!(
            merged_auth
                .get("auth_mode")
                .and_then(|value| value.as_str()),
            Some("apikey")
        );
        assert_eq!(
            merged_auth
                .pointer("/tokens/access_token")
                .and_then(|value| value.as_str()),
            Some("access-token")
        );
        assert_eq!(
            merged_auth
                .pointer("/tokens/refresh_token")
                .and_then(|value| value.as_str()),
            Some("refresh-token")
        );
        assert_eq!(
            merged_auth
                .get("last_refresh")
                .and_then(|value| value.as_str()),
            Some("2026-04-10T00:00:00Z")
        );
        assert!(merged_auth.get("agent_identity").is_some());
    }

    #[test]
    fn merge_codex_auth_json_official_mode_removes_api_key_without_dropping_chatgpt_tokens() {
        let existing_auth = json!({
            "auth_mode": "apikey",
            "OPENAI_API_KEY": "sk-old",
            "last_refresh": "2026-04-10T00:00:00Z",
            "tokens": {
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "account_id": "account-id"
            }
        });

        let merged_auth = merge_codex_auth_json(&existing_auth, &json!({}));

        assert!(merged_auth.get("OPENAI_API_KEY").is_none());
        assert_eq!(
            merged_auth
                .get("auth_mode")
                .and_then(|value| value.as_str()),
            Some("chatgpt")
        );
        assert_eq!(
            merged_auth
                .pointer("/tokens/access_token")
                .and_then(|value| value.as_str()),
            Some("access-token")
        );
        assert_eq!(
            merged_auth
                .pointer("/tokens/refresh_token")
                .and_then(|value| value.as_str()),
            Some("refresh-token")
        );
    }

    #[test]
    fn infer_codex_provider_category_from_settings_detects_official_mode() {
        let provider_settings = json!({
            "auth": {
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": "access-token"
                }
            },
            "config": r#"
model = "gpt-5.4"
model_reasoning_effort = "high"
"#
        });

        assert_eq!(
            infer_codex_provider_category_from_settings(&provider_settings),
            "official"
        );
    }

    #[test]
    fn strip_codex_common_config_from_toml_preserves_runtime_owned_sections() {
        let config_toml = r#"
model_provider = "custom"
approval_policy = "never"

[model_providers.custom]
name = "OpenAI"
base_url = "https://api.example.com"

[features]
plugins = true
test_generation = false
runtime_owned = true

[plugins.demo]
enabled = true

[mcp_servers.local]
command = "uvx"
"#;
        let common_toml = r#"
approval_policy = "never"

[features]
plugins = false
test_generation = true

[plugins.demo]
enabled = false
"#;

        let stripped = strip_codex_common_config_from_toml(config_toml, common_toml)
            .expect("strip should succeed");
        let doc: DocumentMut = stripped.parse().expect("parse stripped config");

        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            doc["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://api.example.com")
        );
        assert_eq!(doc["features"]["plugins"].as_bool(), Some(true));
        assert_eq!(doc["features"]["test_generation"].as_bool(), Some(false));
        assert_eq!(doc["features"]["runtime_owned"].as_bool(), Some(true));
        assert_eq!(doc["plugins"]["demo"]["enabled"].as_bool(), Some(true));
        assert_eq!(doc["mcp_servers"]["local"]["command"].as_str(), Some("uvx"));
        assert!(doc.get("approval_policy").is_none());
    }

    #[test]
    fn strip_codex_common_config_from_toml_preserves_different_provider_values() {
        let config_toml = r#"
model = "glm-5.2"
model_reasoning_effort = "high"
approval_policy = "never"
"#;
        let common_toml = r#"
model = "gpt-5.4"
model_reasoning_effort = "medium"
approval_policy = "never"
"#;

        let stripped = strip_codex_common_config_from_toml(config_toml, common_toml)
            .expect("strip should succeed");
        let doc: DocumentMut = stripped.parse().expect("parse stripped config");

        assert_eq!(doc["model"].as_str(), Some("glm-5.2"));
        assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
        assert!(doc.get("approval_policy").is_none());
    }

    #[test]
    fn extract_provider_settings_for_storage_strips_common_toml_and_protected_sections() {
        let settings = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test",
                "auth_mode": "chatgpt",
                "last_refresh": "2026-04-10T00:00:00Z",
                "tokens": {
                    "access_token": "access-token"
                }
            },
            "config": r#"
model_provider = "custom"
approval_policy = "never"

[model_providers.custom]
name = "OpenAI"
base_url = "https://api.example.com"

[features]
plugins = true
test_generation = false
"#
        });
        let common_toml = r#"
approval_policy = "never"
"#;

        let provider_settings =
            extract_provider_settings_for_storage(&settings, Some(common_toml)).unwrap();

        assert_eq!(
            provider_settings.pointer("/auth/OPENAI_API_KEY"),
            Some(&json!("sk-test"))
        );
        assert!(provider_settings.pointer("/auth/auth_mode").is_none());
        assert!(provider_settings.pointer("/auth/last_refresh").is_none());
        assert!(provider_settings.pointer("/auth/tokens").is_none());
        let provider_config = provider_settings
            .get("config")
            .and_then(|value| value.as_str())
            .expect("config string");
        let doc: DocumentMut = provider_config.parse().expect("parse provider config");
        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            doc["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://api.example.com")
        );
        assert!(doc.get("approval_policy").is_none());
        assert!(doc["features"].get("plugins").is_none());
        assert_eq!(doc["features"]["test_generation"].as_bool(), Some(false));
    }

    #[test]
    fn extract_provider_settings_for_storage_preserves_model_catalog() {
        let settings = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test"
            },
            "config": r#"
model_provider = "custom"
model = "deepseek-v4-flash"
"#,
            "autoReviewModelOverride": "gpt-5.5",
            "modelCatalog": {
                "models": [
                    {
                        "model": "deepseek-v4-flash",
                        "displayName": "DeepSeek Flash",
                        "contextWindow": "64000",
                        "supportsImage": false,
                        "vision": false,
                        "attachment": false,
                        "modalities": {
                            "input": ["text", " "],
                            "output": ["text"]
                        },
                        "reasoningLevels": ["low", "medium", "high", "xhigh", "max", "ultra"],
                        "defaultReasoningLevel": "high"
                    },
                    {
                        // Fully identical to the first row → collapsed.
                        "model": "deepseek-v4-flash",
                        "displayName": "DeepSeek Flash"
                    },
                    {
                        // Same model, distinct display name → kept.
                        "model": "deepseek-v4-flash",
                        "displayName": "Duplicate"
                    },
                    {
                        "model": "kimi-k2",
                        "display_name": "Kimi K2",
                        "context_window": 128000,
                        "supportsImage": true,
                        "modalities": {
                            "input": ["text", "image"]
                        }
                    },
                    {
                        "model": " "
                    }
                ]
            }
        });

        let provider_settings = extract_provider_settings_for_storage(&settings, None).unwrap();

        assert_eq!(
            provider_settings.pointer("/autoReviewModelOverride"),
            Some(&json!("gpt-5.5"))
        );
        assert_eq!(
            provider_settings.pointer("/modelCatalog/models/0"),
            Some(&json!({
                "model": "deepseek-v4-flash",
                "displayName": "DeepSeek Flash",
                "contextWindow": 64000,
                "supportsImage": false,
                "vision": false,
                "attachment": false,
                "modalities": {
                    "input": ["text"],
                    "output": ["text"]
                },
                "reasoningLevels": ["low", "medium", "high", "xhigh", "max", "ultra"],
                "defaultReasoningLevel": "high"
            }))
        );
        assert_eq!(
            provider_settings.pointer("/modelCatalog/models/1"),
            Some(&json!({
                "model": "deepseek-v4-flash",
                "displayName": "Duplicate"
            }))
        );
        assert_eq!(
            provider_settings.pointer("/modelCatalog/models/2"),
            Some(&json!({
                "model": "kimi-k2",
                "displayName": "Kimi K2",
                "contextWindow": 128000,
                "supportsImage": true,
                "modalities": {
                    "input": ["text", "image"]
                }
            }))
        );
        assert!(provider_settings
            .pointer("/modelCatalog/models/3")
            .is_none());
    }

    #[test]
    fn extract_provider_settings_for_storage_keeps_default_model_independent_from_catalog() {
        let settings = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test"
            },
            "config": r#"
model = "gpt-5.4"
model_provider = "custom"
"#,
            "modelCatalog": {
                "models": [
                    {
                        "model": "glm-5.2",
                        "displayName": "GLM 5.2"
                    }
                ]
            }
        });
        let common_toml = r#"
model = "gpt-5.5"
"#;

        let provider_settings =
            extract_provider_settings_for_storage(&settings, Some(common_toml)).unwrap();
        let config = provider_settings
            .get("config")
            .and_then(Value::as_str)
            .expect("config string");
        let doc: DocumentMut = config.parse().expect("parse provider config");

        assert_eq!(doc["model"].as_str(), Some("gpt-5.4"));
        assert_eq!(
            provider_settings.pointer("/modelCatalog/models/0/model"),
            Some(&json!("glm-5.2"))
        );
    }

    #[test]
    fn extract_provider_settings_for_storage_strips_unified_history_injection() {
        let live_config =
            unified_history::inject_unified_session_history_config("model = \"gpt-5\"\n")
                .expect("inject unified history");
        let settings = json!({
            "auth": {},
            "config": live_config,
        });

        let provider_settings = extract_provider_settings_for_storage(&settings, None).unwrap();

        let provider_config = provider_settings
            .get("config")
            .and_then(|value| value.as_str())
            .expect("config string");
        let doc: DocumentMut = provider_config.parse().expect("parse provider config");
        assert_eq!(doc["model"].as_str(), Some("gpt-5"));
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model_providers").is_none());
    }

    #[test]
    fn extract_provider_settings_for_storage_moves_experimental_bearer_token_to_auth() {
        let settings = json!({
            "auth": {
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": "official-access"
                }
            },
            "config": r#"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
base_url = "https://api.example.com/v1"
experimental_bearer_token = "sk-live"
"#
        });

        let provider_settings = extract_provider_settings_for_storage(&settings, None).unwrap();

        assert_eq!(
            provider_settings.pointer("/auth/OPENAI_API_KEY"),
            Some(&json!("sk-live"))
        );
        assert!(provider_settings.pointer("/auth/tokens").is_none());

        let provider_config = provider_settings
            .get("config")
            .and_then(|value| value.as_str())
            .expect("config string");
        let doc: DocumentMut = provider_config.parse().expect("parse provider config");
        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert!(doc["model_providers"]["custom"]
            .as_table_like()
            .expect("custom provider table")
            .get("experimental_bearer_token")
            .is_none());
    }

    #[test]
    fn provider_switching_with_auth_preservation_clears_old_tokens() {
        // Scenario: Provider A → Provider B → Official → Disable switch → Provider A
        // This integration test ensures experimental_bearer_token is properly cleaned up

        // Provider A config with preserve=true
        let provider_a_config = r#"
model_provider = "provider-a"

[model_providers.provider-a]
name = "Provider A"
base_url = "https://api.provider-a.com/v1"
"#;
        let provider_a_auth = json!({"OPENAI_API_KEY": "sk-provider-a"});
        let projected_a =
            project_codex_auth_to_runtime_config(provider_a_config, &provider_a_auth, true)
                .unwrap();
        let doc_a: DocumentMut = projected_a.parse().unwrap();
        assert_eq!(
            doc_a["model_providers"]["provider-a"]["experimental_bearer_token"].as_str(),
            Some("sk-provider-a")
        );

        // Switch to Provider B with preserve=true
        let provider_b_config = r#"
model_provider = "provider-b"

[model_providers.provider-b]
name = "Provider B"
base_url = "https://api.provider-b.com/v1"
"#;
        let provider_b_auth = json!({"OPENAI_API_KEY": "sk-provider-b"});
        let projected_b =
            project_codex_auth_to_runtime_config(provider_b_config, &provider_b_auth, true)
                .unwrap();

        // Simulate diff cleanup: previous_managed has provider-a token, next_managed has provider-b
        let cleaned_b = build_written_codex_config_toml(
            &projected_a,       // existing file with provider-a token
            Some(&projected_a), // previous managed
            &projected_b,       // next managed
        )
        .unwrap();
        let doc_b: DocumentMut = cleaned_b.parse().unwrap();
        assert_eq!(doc_b["model_provider"].as_str(), Some("provider-b"));
        assert_eq!(
            doc_b["model_providers"]["provider-b"]["experimental_bearer_token"].as_str(),
            Some("sk-provider-b")
        );
        // Provider A's table should be completely removed
        assert!(doc_b
            .get("model_providers")
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get("provider-a"))
            .is_none());

        // Switch to official provider with preserve=false
        let official_config = r#"
model = "claude-sonnet-4-6"
"#;
        let official_auth = json!({});
        let projected_official = project_codex_auth_to_runtime_config(
            official_config,
            &official_auth,
            false, // preserve=false for official
        )
        .unwrap();

        let cleaned_official = build_written_codex_config_toml(
            &cleaned_b,
            Some(&cleaned_b), // previous was provider-b with token
            &projected_official,
        )
        .unwrap();
        let doc_official: DocumentMut = cleaned_official.parse().unwrap();
        assert_eq!(doc_official["model"].as_str(), Some("claude-sonnet-4-6"));
        // All provider tables and tokens should be gone
        assert!(doc_official.get("model_provider").is_none());
        assert!(doc_official.get("model_providers").is_none());
        assert!(doc_official.get("experimental_bearer_token").is_none());

        // Switch back to Provider A with preserve=false (switch disabled)
        let projected_a_no_preserve =
            project_codex_auth_to_runtime_config(provider_a_config, &provider_a_auth, false)
                .unwrap();
        let doc_a_no_preserve: DocumentMut = projected_a_no_preserve.parse().unwrap();
        // Should not have experimental_bearer_token when preserve=false
        assert!(doc_a_no_preserve["model_providers"]["provider-a"]
            .as_table_like()
            .and_then(|table| table.get("experimental_bearer_token"))
            .is_none());
    }

    #[test]
    fn extract_codex_common_config_from_settings_toml_removes_provider_specific_sections() {
        let config_toml = r#"
model_provider = "custom"
model = "gpt-5.4"
approval_policy = "never"

[model_providers.custom]
name = "OpenAI"
base_url = "https://api.example.com"

[features]
plugins = true
test_generation = false
"#;

        let common_toml = extract_codex_common_config_from_settings_toml(config_toml)
            .expect("extract common config");
        let doc: DocumentMut = common_toml.parse().expect("parse common config");

        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model").is_none());
        assert!(doc.get("model_providers").is_none());
        assert_eq!(doc["approval_policy"].as_str(), Some("never"));
        assert!(doc["features"].get("plugins").is_none());
        assert_eq!(doc["features"]["test_generation"].as_bool(), Some(false));
    }

    #[test]
    fn extract_codex_common_config_from_settings_toml_keeps_shared_model_for_official_mode() {
        let config_toml = r#"
model = "gpt-5.4"
approval_policy = "never"

[features]
plugins = true
"#;

        let common_toml = extract_codex_common_config_from_settings_toml(config_toml)
            .expect("extract common config");
        let doc: DocumentMut = common_toml.parse().expect("parse common config");

        assert_eq!(doc["model"].as_str(), Some("gpt-5.4"));
        assert_eq!(doc["approval_policy"].as_str(), Some("never"));
        assert!(doc.get("features").is_none());
    }

    #[test]
    fn extract_codex_common_config_from_settings_toml_removes_model_for_top_level_custom_base_url()
    {
        let config_toml = r#"
model = "gpt-5.4"
base_url = "https://api.example.com/v1"
approval_policy = "never"
"#;

        let common_toml = extract_codex_common_config_from_settings_toml(config_toml)
            .expect("extract common config");
        let doc: DocumentMut = common_toml.parse().expect("parse common config");

        assert!(doc.get("model").is_none());
        assert!(doc.get("base_url").is_none());
        assert_eq!(doc["approval_policy"].as_str(), Some("never"));
    }

    // Regression for issue #311: a leftover `model_provider` pointing to a
    // missing `[model_providers.<id>]` table makes Codex CLI refuse to load
    // config.toml ("Model provider `...` not found"). The read path must
    // self-heal it so Codex can always start, regardless of how the dangling
    // state was produced (legacy ai-toolbox-gateway takeover, manual edit,
    // partial migration, cross-replica sync).

    #[test]
    fn heal_removes_legacy_ai_toolbox_gateway_dangling_model_provider() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let original = r#"
model_provider = "ai-toolbox-gateway"

[mcp_servers.keep]
command = "node"
"#;
        std::fs::write(&config_path, original).unwrap();

        let healed = heal_dangling_codex_model_provider(&config_path, original)
            .expect("heal dangling model_provider");
        assert!(healed.is_some(), "should heal dangling legacy sentinel");

        let on_disk = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(on_disk, healed.unwrap());
        let doc: DocumentMut = on_disk.parse().unwrap();
        assert!(doc.get("model_provider").is_none());
        // Runtime-owned sections must be preserved verbatim.
        assert_eq!(doc["mcp_servers"]["keep"]["command"].as_str(), Some("node"));
    }

    #[test]
    fn heal_removes_arbitrary_dangling_model_provider() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let original = r#"
model_provider = "missing"

[model_providers.real]
name = "Real"
base_url = "https://real.example.com/v1"
"#;
        std::fs::write(&config_path, original).unwrap();

        let healed = heal_dangling_codex_model_provider(&config_path, original)
            .expect("heal dangling model_provider");
        assert!(healed.is_some());

        let doc: DocumentMut = std::fs::read_to_string(&config_path)
            .unwrap()
            .parse()
            .unwrap();
        assert!(doc.get("model_provider").is_none());
        // An unrelated provider table is not the active one, keep it untouched.
        assert_eq!(
            doc["model_providers"]["real"]["base_url"].as_str(),
            Some("https://real.example.com/v1")
        );
    }

    #[test]
    fn heal_leaves_valid_config_untouched() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let original = r#"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
base_url = "http://127.0.0.1:37123/openai/v1"
"#;
        std::fs::write(&config_path, original).unwrap();

        let healed =
            heal_dangling_codex_model_provider(&config_path, original).expect("heal check");
        assert!(healed.is_none(), "valid config must not be rewritten");

        let on_disk = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(on_disk, original);
    }

    #[test]
    fn heal_leaves_config_without_model_provider_untouched() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let original = r#"
approval_policy = "never"
"#;
        std::fs::write(&config_path, original).unwrap();

        let healed =
            heal_dangling_codex_model_provider(&config_path, original).expect("heal check");
        assert!(healed.is_none());
        let on_disk = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(on_disk, original);
    }

    #[test]
    fn heal_leaves_empty_and_unparseable_config_untouched() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");

        // Empty content -> no healing.
        std::fs::write(&config_path, "   ").unwrap();
        assert_eq!(
            heal_dangling_codex_model_provider(&config_path, "   ").unwrap(),
            None
        );

        // Unparseable content -> no healing, no panic, file untouched.
        let garbage = "this is not = = valid toml [[";
        std::fs::write(&config_path, garbage).unwrap();
        assert_eq!(
            heal_dangling_codex_model_provider(&config_path, garbage).unwrap(),
            None
        );
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), garbage);
    }
}

// ============================================================================
// Codex Common Config Commands
// ============================================================================

/// Get Codex common config
/// If database is empty, returns empty config (Codex doesn't have common config in local files)
#[tauri::command]
pub async fn get_codex_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<CodexCommonConfig>, String> {
    let db = state.db();
    get_codex_common_from_sqlite(db)
}

#[tauri::command]
pub async fn extract_codex_common_config_from_current_file(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<CodexCommonConfig, String> {
    let db = state.db();
    extract_codex_common_config_from_current_files_with_db(&db).await
}

/// Save Codex common config
#[tauri::command]
pub async fn save_codex_common_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexCommonConfigInput,
) -> Result<(), String> {
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(&db, "codex").await;
    let previous_managed_config_toml = get_current_applied_managed_codex_config(&db).await?;

    // Validate TOML if not empty
    if !input.config.trim().is_empty() {
        let _: toml::Table =
            toml::from_str(&input.config).map_err(|e| format!("Invalid TOML: {}", e))?;
    }

    let existing_common = get_codex_common_config(state.clone()).await?;
    let root_dir = if input.clear_root_dir {
        None
    } else {
        input
            .root_dir
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(str::to_string)
            .or_else(|| existing_common.and_then(|config| config.root_dir))
    };
    let json_data = adapter::to_db_value_common(&input.config, root_dir.as_deref());
    put_codex_common_to_sqlite(db, &json_data)?;
    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "codex").await?;

    // Re-apply current provider config to write merged config to file
    if let Some(provider) = get_applied_codex_provider(&db).await? {
        if let Err(e) = apply_config_to_file_with_previous_managed_config(
            &db,
            &provider.id,
            previous_managed_config_toml.clone(),
        )
        .await
        {
            eprintln!("Failed to re-apply config: {}", e);
        } else {
            #[cfg(target_os = "windows")]
            let _ = app.emit("wsl-sync-request-codex", ());
        }
    }

    resync_all_skills_if_tool_path_changed(
        app.clone(),
        state.inner(),
        "codex",
        previous_skills_path,
    )
    .await;

    // Emit config-changed event to notify frontend
    let _ = app.emit("config-changed", "window");

    Ok(())
}

/// Save local config (provider and/or common) into database
/// Input can include provider and/or commonConfig; missing parts will be loaded from local files
#[tauri::command]
pub async fn save_codex_local_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: CodexLocalConfigInput,
) -> Result<(), String> {
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(&db, "codex").await;

    // Load current live settings to capture the full managed snapshot before normalization.
    let current_live_settings = read_codex_settings_from_disk(Some(&db)).await?;
    let current_live_settings_value = serde_json::json!({
        "auth": current_live_settings.auth.unwrap_or(serde_json::json!({})),
        "config": current_live_settings.config.unwrap_or_default(),
    });
    let previous_managed_config_toml = strip_protected_top_level_sections_from_toml(
        current_live_settings_value
            .get("config")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
    )?;

    // Load base provider from local files
    let base_provider = load_temp_provider_from_files_with_db(Some(&db)).await?;

    let provider_input = input.provider;
    let provider_name = provider_input
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or(base_provider.name);
    let provider_category = provider_input
        .as_ref()
        .map(|p| p.category.clone())
        .unwrap_or(base_provider.category);
    let provider_settings_config = provider_input
        .as_ref()
        .map(|p| p.settings_config.clone())
        .unwrap_or(base_provider.settings_config);
    let provider_source_id = provider_input
        .as_ref()
        .and_then(|p| p.source_provider_id.clone());
    let provider_notes = provider_input
        .as_ref()
        .and_then(|p| p.notes.clone())
        .or(base_provider.notes);
    let provider_sort_index = provider_input
        .as_ref()
        .and_then(|p| p.sort_index)
        .or(base_provider.sort_index);
    let provider_is_disabled = provider_input
        .as_ref()
        .and_then(|p| p.is_disabled)
        .unwrap_or(false);

    let common_config = input.common_config.unwrap_or_default();

    let now = Local::now().to_rfc3339();
    let normalized_provider_settings_config = normalize_provider_settings_for_storage(
        &db,
        &provider_settings_config,
        Some(&common_config),
    )
    .await?;
    let provider_content = CodexProviderContent {
        name: provider_name,
        category: provider_category,
        settings_config: normalized_provider_settings_config,
        source_provider_id: provider_source_id,
        website_url: None,
        notes: provider_notes,
        icon: None,
        icon_color: None,
        sort_index: provider_sort_index,
        meta: resolve_local_provider_meta(provider_input.as_ref(), base_provider.meta),
        is_applied: true,
        is_disabled: provider_is_disabled,
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_id = db_new_id();
    put_codex_provider_to_sqlite(db, &provider_id, &provider_content)?;

    let root_dir = if input.clear_root_dir {
        None
    } else {
        let trimmed_root_dir = input
            .root_dir
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(str::to_string);
        if trimmed_root_dir.is_some() {
            trimmed_root_dir
        } else {
            get_codex_custom_root_dir_async(&db)
                .await
                .map(|path| path.to_string_lossy().to_string())
        }
    };
    let common_json = adapter::to_db_value_common(&common_config, root_dir.as_deref());
    put_codex_common_to_sqlite(db, &common_json)?;
    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "codex").await?;

    // Re-apply config to files using the newly created provider
    if let Err(e) = apply_config_to_file_with_previous_managed_config(
        &db,
        &provider_id,
        Some(previous_managed_config_toml),
    )
    .await
    {
        eprintln!("Failed to apply config after local save: {}", e);
    } else {
        #[cfg(target_os = "windows")]
        let _ = app.emit("wsl-sync-request-codex", ());
    }

    resync_all_skills_if_tool_path_changed(
        app.clone(),
        state.inner(),
        "codex",
        previous_skills_path,
    )
    .await;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

// ============================================================================
// Codex Initialization
// ============================================================================

/// Initialize Codex provider from existing config files
pub async fn init_codex_provider_from_settings(
    db: &crate::db::SqliteDbState,
) -> Result<(), String> {
    if import_codex_default_provider_from_local_files(db, true)
        .await?
        .is_some()
    {
        println!("✅ Imported Codex settings as default provider");
    }
    Ok(())
}
