use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use super::adapter;
use super::official_accounts::{
    clear_all_codex_official_account_apply_status, codex_provider_has_official_accounts,
    ensure_codex_provider_has_no_official_accounts, sync_codex_official_account_apply_status,
};
use super::plugin_ops;
use super::plugin_state;
use super::plugin_types::{
    CodexInstalledPlugin, CodexMarketplacePlugin, CodexPluginActionInput, CodexPluginMarketplace,
    CodexPluginRuntimeStatus, CodexPluginWorkspaceRoot, CodexPluginWorkspaceRootInput,
};
use super::plugin_workspace;
use super::types::*;
use crate::coding::all_api_hub;
use crate::coding::db_id::{db_new_id, db_record_id};
use crate::coding::open_code::shell_env;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::runtime_location;
use crate::coding::skills::commands::resync_all_skills_if_tool_path_changed;
use crate::db::DbState;
use crate::http_client;
use chrono::Local;
use tauri::Emitter;

const PROTECTED_TOP_LEVEL_TOML_KEYS: [&str; 3] = ["mcp_servers", "features", "plugins"];

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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Option<PathBuf> {
    let mut result = db
        .query("SELECT * OMIT id FROM codex_common_config:`common` LIMIT 1")
        .await
        .ok()?;
    let records: Vec<Value> = result.take(0).ok()?;
    let record = records.into_iter().next()?;
    let config = adapter::from_db_value_common(record);
    config
        .root_dir
        .filter(|dir| !dir.trim().is_empty())
        .map(PathBuf::from)
}

pub fn get_codex_root_dir_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(runtime_location::get_codex_runtime_location_sync(db)?.host_path)
}

pub(super) async fn get_codex_root_dir_from_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(runtime_location::get_codex_runtime_location_async(db)
        .await?
        .host_path)
}

pub fn get_codex_root_path_info_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<ConfigPathInfo, String> {
    let location = runtime_location::get_codex_runtime_location_sync(db)?;
    Ok(ConfigPathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

async fn get_codex_root_path_info_from_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<std::path::PathBuf, String> {
    get_codex_root_dir_from_db_async(db).await
}

async fn get_codex_auth_path_from_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<std::path::PathBuf, String> {
    Ok(get_codex_config_dir_from_db_async(db)
        .await?
        .join("auth.json"))
}

async fn get_codex_config_path_from_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<std::path::PathBuf, String> {
    Ok(runtime_location::resolve_codex_prompt_file_path(
        &get_codex_config_dir_from_db_async(db).await?,
    ))
}

async fn get_local_prompt_config(
    db: Option<&surrealdb::Surreal<surrealdb::engine::local::Db>>,
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
        id: "__local__".to_string(),
        name: "default".to_string(),
        content: prompt_content,
        is_applied: true,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }))
}

async fn read_codex_settings_from_disk(
    db: Option<&surrealdb::Surreal<surrealdb::engine::local::Db>>,
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
        Some(
            fs::read_to_string(&config_path)
                .map_err(|e| format!("Failed to read config.toml: {}", e))?,
        )
    } else {
        None
    };

    Ok(CodexSettings { auth, config })
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
    state: tauri::State<'_, DbState>,
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
    db: Option<&surrealdb::Surreal<surrealdb::engine::local::Db>>,
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

fn emit_codex_plugin_config_changed<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit("config-changed", "window");
    emit_prompt_sync_requests(app);
}

/// Get Codex config directory path
#[tauri::command]
pub async fn get_codex_config_dir_path(state: tauri::State<'_, DbState>) -> Result<String, String> {
    let db = state.db();
    let config_dir = get_codex_config_dir_from_db_async(&db).await?;
    Ok(config_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_codex_plugin_runtime_status(
    state: tauri::State<'_, DbState>,
) -> Result<CodexPluginRuntimeStatus, String> {
    let db = state.db();
    plugin_state::get_codex_plugin_runtime_status(&db).await
}

#[tauri::command]
pub async fn list_codex_installed_plugins(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<CodexInstalledPlugin>, String> {
    let db = state.db();
    plugin_state::list_codex_installed_plugins(&db).await
}

#[tauri::command]
pub async fn list_codex_marketplaces(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<CodexPluginMarketplace>, String> {
    let db = state.db();
    plugin_state::list_codex_marketplaces(&db).await
}

#[tauri::command]
pub async fn list_codex_marketplace_plugins(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<CodexMarketplacePlugin>, String> {
    let db = state.db();
    plugin_state::list_codex_marketplace_plugins(&db).await
}

#[tauri::command]
pub async fn list_codex_plugin_workspace_roots(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<CodexPluginWorkspaceRoot>, String> {
    let db = state.db();
    plugin_workspace::list_codex_plugin_workspace_roots(&db).await
}

#[tauri::command]
pub async fn add_codex_plugin_workspace_root(
    state: tauri::State<'_, DbState>,
    input: CodexPluginWorkspaceRootInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_workspace::add_codex_plugin_workspace_root(&db, &input.path).await
}

#[tauri::command]
pub async fn remove_codex_plugin_workspace_root(
    state: tauri::State<'_, DbState>,
    input: CodexPluginWorkspaceRootInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_workspace::remove_codex_plugin_workspace_root(&db, &input.path).await
}

#[tauri::command]
pub async fn install_codex_plugin(
    state: tauri::State<'_, DbState>,
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
    state: tauri::State<'_, DbState>,
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
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: CodexPluginActionInput,
) -> Result<(), String> {
    let db = state.db();
    plugin_ops::set_codex_plugin_enabled(&db, &input.plugin_id, false).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn uninstall_codex_plugin(
    state: tauri::State<'_, DbState>,
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
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let db = state.db();
    plugin_ops::ensure_codex_plugins_feature_enabled(&db).await?;
    emit_codex_plugin_config_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn get_codex_root_path_info(
    state: tauri::State<'_, DbState>,
) -> Result<ConfigPathInfo, String> {
    let db = state.db();
    get_codex_root_path_info_from_db_async(&db).await
}

/// Get Codex config.toml file path
#[tauri::command]
pub async fn get_codex_config_file_path(
    state: tauri::State<'_, DbState>,
) -> Result<String, String> {
    let db = state.db();
    let config_path = get_codex_config_path_from_db_async(&db).await?;
    Ok(config_path.to_string_lossy().to_string())
}

/// Reveal Codex config folder in file explorer
#[tauri::command]
pub async fn reveal_codex_config_folder(state: tauri::State<'_, DbState>) -> Result<(), String> {
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
/// If database is empty, returns a temporary provider loaded from local config files
#[tauri::command]
pub async fn list_codex_providers(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<CodexProvider>, String> {
    let db = state.db();

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider")
        .await
        .map_err(|e| format!("Failed to query providers: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if records.is_empty() {
                // Database is empty, try to load from local files as temporary provider
                if let Ok(temp_provider) = load_temp_provider_from_files_with_db(Some(&db)).await {
                    return Ok(vec![temp_provider]);
                }
                Ok(Vec::new())
            } else {
                let mut result: Vec<CodexProvider> = records
                    .into_iter()
                    .map(adapter::from_db_value_provider)
                    .collect();
                result.sort_by_key(|p| p.sort_index.unwrap_or(0));
                Ok(result)
            }
        }
        Err(e) => {
            eprintln!("Failed to deserialize providers: {}", e);
            // Try to load from local files as fallback
            if let Ok(temp_provider) = load_temp_provider_from_files_with_db(Some(&db)).await {
                return Ok(vec![temp_provider]);
            }
            Ok(Vec::new())
        }
    }
}

/// 修复损坏的 Codex provider 数据
/// This is used when the database is empty and we want to show the local config
async fn load_temp_provider_from_files_with_db(
    db: Option<&surrealdb::Surreal<surrealdb::engine::local::Db>>,
) -> Result<CodexProvider, String> {
    let root_dir = if let Some(db) = db {
        get_codex_root_dir_from_db_async(db).await?
    } else {
        get_codex_root_dir_without_db()?
    };
    let auth_path = root_dir.join("auth.json");
    let config_path = root_dir.join("config.toml");

    if !auth_path.exists() && !config_path.exists() {
        return Err("No config files found".to_string());
    }

    // Read auth.json (optional)
    let auth: serde_json::Value = if auth_path.exists() {
        let auth_content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        serde_json::from_str(&auth_content)
            .map_err(|e| format!("Failed to parse auth.json: {}", e))?
    } else {
        serde_json::json!({})
    };

    // Read config.toml (optional)
    let config_toml = if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Build settings_config
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
    let category = infer_codex_provider_category_from_settings(&provider_settings);

    let now = Local::now().to_rfc3339();
    Ok(CodexProvider {
        id: "__local__".to_string(), // Special ID to indicate this is from local files
        name: "default".to_string(),
        category,
        settings_config: serde_json::to_string(&provider_settings).unwrap_or_default(),
        source_provider_id: None,
        website_url: None,
        notes: None,
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

async fn get_codex_common_toml(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<String>, String> {
    let common_config_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM codex_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query common config: {}", e))?
        .take(0);

    Ok(match common_config_result {
        Ok(records) => records.first().and_then(|record| {
            record
                .get("config")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        }),
        Err(_) => None,
    })
}

async fn normalize_provider_settings_for_storage(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<CodexCommonConfig, String> {
    let settings = read_codex_settings_from_disk(Some(db)).await?;
    let config_toml = settings.config.unwrap_or_default();
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<CodexProvider>, String> {
    let applied_result: Result<Vec<Value>, _> = db
        .query(
            "SELECT *, type::string(id) as id FROM codex_provider WHERE is_applied = true LIMIT 1",
        )
        .await
        .map_err(|e| format!("Failed to query applied provider: {}", e))?
        .take(0);

    match applied_result {
        Ok(records) => Ok(records
            .first()
            .map(|record| adapter::from_db_value_provider(record.clone()))),
        Err(error) => Err(format!("Failed to deserialize applied provider: {}", error)),
    }
}

async fn query_codex_provider_by_id(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<CodexProvider, String> {
    let record_id = db_record_id("codex_provider", provider_id);
    let provider_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query provider: {}", e))?
        .take(0);

    match provider_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::from_db_value_provider(record.clone()))
            } else {
                Err("Provider not found".to_string())
            }
        }
        Err(e) => Err(format!("Failed to deserialize provider: {}", e)),
    }
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

    let root_table = parsed_document.as_table();
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
        .map(config_contains_managed_codex_provider)
        .unwrap_or(false);

    if !has_managed_api_key && !has_managed_base_url {
        "official".to_string()
    } else {
        "custom".to_string()
    }
}

fn merge_codex_auth_json(
    existing_auth: &serde_json::Value,
    managed_auth: &serde_json::Value,
) -> serde_json::Value {
    let mut merged_auth = existing_auth.as_object().cloned().unwrap_or_default();

    let next_api_key = managed_auth
        .as_object()
        .and_then(|auth| auth.get("OPENAI_API_KEY"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

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
    let settings_object = settings_value
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

    let stripped_common_config_toml = if let Some(common_toml) = common_config_toml {
        strip_codex_common_config_from_toml(config_toml, common_toml)?
    } else {
        config_toml.to_string()
    };
    let normalized_config_toml =
        strip_protected_top_level_sections_from_toml(&stripped_common_config_toml)?;

    Ok(serde_json::json!({
        "auth": auth_value,
        "config": normalized_config_toml,
    }))
}

fn strip_protected_top_level_toml_keys(document: &mut toml_edit::DocumentMut) {
    let root_table = document.as_table_mut();
    for protected_key in PROTECTED_TOP_LEVEL_TOML_KEYS {
        root_table.remove(protected_key);
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

async fn get_managed_codex_config_for_provider(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_settings_config: &str,
) -> Result<String, String> {
    let common_toml = get_codex_common_toml(db).await?;
    build_managed_codex_config(provider_settings_config, common_toml.as_deref())
}

async fn get_current_applied_managed_codex_config(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<String>, String> {
    let Some(applied_provider) = get_applied_codex_provider(db).await? else {
        return Ok(None);
    };

    Ok(Some(
        get_managed_codex_config_for_provider(db, &applied_provider.settings_config).await?,
    ))
}

/// 修复损坏的 Codex provider 数据
/// 删除所有 provider 记录，需要重新创建
#[tauri::command]
pub async fn repair_codex_providers(state: tauri::State<'_, DbState>) -> Result<String, String> {
    let db = state.db();

    db.query("DELETE codex_provider")
        .await
        .map_err(|e| format!("Failed to delete providers: {}", e))?;

    Ok("All Codex providers have been deleted. Please recreate them.".to_string())
}

/// Create a new Codex provider
#[tauri::command]
pub async fn create_codex_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
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
        is_applied: false,
        is_disabled: provider.is_disabled.unwrap_or(false),
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    // Create new provider - SurrealDB auto-generates record ID
    db.query("CREATE codex_provider CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    // Fetch the created record to get the auto-generated ID
    let result: Result<Vec<Value>, _> = db
        .query(
            "SELECT *, type::string(id) as id FROM codex_provider ORDER BY created_at DESC LIMIT 1",
        )
        .await
        .map_err(|e| format!("Failed to fetch created provider: {}", e))?
        .take(0);

    // Notify to refresh tray menu
    let _ = app.emit("config-changed", "window");

    match result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::from_db_value_provider(record.clone()))
            } else {
                Err("Failed to retrieve created provider".to_string())
            }
        }
        Err(e) => Err(format!("Failed to retrieve created provider: {}", e)),
    }
}

/// Update an existing Codex provider
#[tauri::command]
pub async fn update_codex_provider(
    state: tauri::State<'_, DbState>,
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
    let record_id = db_record_id("codex_provider", &id);
    let existing_result: Result<Vec<Value>, _> = db
        .query(&format!("SELECT * OMIT id FROM {} LIMIT 1", record_id))
        .await
        .map_err(|e| format!("Failed to query existing provider: {}", e))?
        .take(0);

    // Check if provider exists
    if let Ok(records) = &existing_result {
        if records.is_empty() {
            return Err(format!("Codex provider with ID '{}' not found", id));
        }
    }
    if provider.category != "official" && codex_provider_has_official_accounts(&db, &id).await? {
        return Err(
            "This provider still has official accounts. Delete them before switching the provider away from official mode"
                .to_string(),
        );
    }

    // Get created_at and is_disabled from existing record
    let (created_at, existing_is_disabled) = if !provider.created_at.is_empty() {
        (provider.created_at, false)
    } else if let Ok(records) = &existing_result {
        if let Some(record) = records.first() {
            let created = record
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or(&now)
                .to_string();
            let is_disabled = record
                .get("is_disabled")
                .or_else(|| record.get("isDisabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            (created, is_disabled)
        } else {
            (now.clone(), false)
        }
    } else {
        (now.clone(), false)
    };

    let previous_managed_config_toml = if provider.is_applied {
        if let Ok(records) = &existing_result {
            if let Some(record) = records.first() {
                if let Some(settings_config) = record
                    .get("settings_config")
                    .and_then(|value| value.as_str())
                {
                    Some(get_managed_codex_config_for_provider(&db, settings_config).await?)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
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
        is_applied: provider.is_applied,
        is_disabled: existing_is_disabled,
        created_at,
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    // Use database id for update
    db.query(format!("UPDATE codex_provider:`{}` CONTENT $data", id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to update provider: {}", e))?;

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
        is_applied: content.is_applied,
        is_disabled: content.is_disabled,
        created_at: content.created_at,
        updated_at: content.updated_at,
    })
}

/// Delete a Codex provider
#[tauri::command]
pub async fn delete_codex_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    ensure_codex_provider_has_no_official_accounts(&db, &id).await?;

    db.query(format!("DELETE codex_provider:`{}`", id))
        .await
        .map_err(|e| format!("Failed to delete codex provider: {}", e))?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Reorder Codex providers
/// 使用 DELETE + CREATE 模式避免 SurrealDB MVCC 版本控制问题
#[tauri::command]
pub async fn reorder_codex_providers(
    state: tauri::State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    for (index, id) in ids.iter().enumerate() {
        // 首先获取现有记录
        let record_id = db_record_id("codex_provider", id);
        let existing_result: Result<Vec<Value>, _> = db
            .query(&format!(
                "SELECT *, type::string(id) as id FROM {} LIMIT 1",
                record_id
            ))
            .await
            .map_err(|e| format!("Failed to query provider {}: {}", id, e))?
            .take(0);

        if let Ok(records) = existing_result {
            if let Some(record) = records.first() {
                // 构建更新后的内容
                let content = CodexProviderContent {
                    name: record
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    category: record
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    settings_config: record
                        .get("settings_config")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string(),
                    source_provider_id: record
                        .get("source_provider_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    website_url: record
                        .get("website_url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    notes: record
                        .get("notes")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    icon: record
                        .get("icon")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    icon_color: record
                        .get("icon_color")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    sort_index: Some(index as i32),
                    is_applied: record
                        .get("is_applied")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    is_disabled: record
                        .get("is_disabled")
                        .or_else(|| record.get("isDisabled"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    created_at: record
                        .get("created_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&now)
                        .to_string(),
                    updated_at: now.clone(),
                };

                let json_data = adapter::to_db_value_provider(&content);

                // Use Blind Write pattern with native ID format
                db.query(format!("UPDATE codex_provider:`{}` CONTENT $data", id))
                    .bind(("data", json_data))
                    .await
                    .map_err(|e| format!("Failed to update provider {}: {}", id, e))?;
            }
        }
    }

    Ok(())
}

/// Select a Codex provider (mark as applied in database)
/// 使用 DELETE + CREATE 模式避免 SurrealDB MVCC 版本控制问题
#[tauri::command]
pub async fn select_codex_provider(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
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
/// Use UPDATE with WHERE to avoid SurrealDB MVCC version control issues
async fn update_is_applied_status(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    target_id: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    let target_id = target_id.to_string(); // Clone for bind

    // Clear current applied status (only update the currently applied one)
    db.query(
        "UPDATE codex_provider SET is_applied = false, updated_at = $now WHERE is_applied = true",
    )
    .bind(("now", now.clone()))
    .await
    .map_err(|e| format!("Failed to clear applied status: {}", e))?;

    // Set target provider as applied
    let record_id = db_record_id("codex_provider", &target_id);
    db.query(&format!(
        "UPDATE {} SET is_applied = true, updated_at = $now",
        record_id
    ))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to set applied status: {}", e))?;

    Ok(())
}

// ============================================================================
// Codex Config File Commands
// ============================================================================

/// Internal function: apply provider config to files
async fn apply_config_to_file(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_to_file_with_previous_managed_config(db, provider_id, None).await
}

/// Public version for tray module
pub async fn apply_config_to_file_public(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_to_file_with_previous_managed_config(db, provider_id, None).await
}

async fn apply_config_to_file_with_previous_managed_config(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
    previous_managed_config_toml: Option<String>,
) -> Result<(), String> {
    let previous_managed_config_toml = match previous_managed_config_toml {
        Some(config) => Some(config),
        None => get_current_applied_managed_codex_config(db).await?,
    };

    // Get the provider
    let record_id = db_record_id("codex_provider", provider_id);
    let provider_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query provider: {}", e))?
        .take(0);

    let provider = match provider_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::from_db_value_provider(record.clone())
            } else {
                return Err("Provider not found".to_string());
            }
        }
        Err(e) => return Err(format!("Failed to deserialize provider: {}", e)),
    };

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
        .unwrap_or(serde_json::json!({}));
    let final_config =
        build_managed_codex_config(&provider.settings_config, common_toml.as_deref())?;

    write_codex_config_files(
        Some(db),
        &auth,
        previous_managed_config_toml.as_deref(),
        &final_config,
    )
    .await?;
    Ok(())
}

/// Append common TOML config to provider config (common is appended after provider)
fn append_toml_configs(provider: &str, common: &str) -> Result<String, String> {
    let provider_content = provider.trim();
    let common_content = common.trim();

    if provider_content.is_empty() {
        return Ok(common_content.to_string());
    }
    if common_content.is_empty() {
        return Ok(provider_content.to_string());
    }

    let mut provider_doc = parse_toml_document(provider_content, "provider config")?;
    let common_doc = parse_toml_document(common_content, "common config")?;

    merge_toml_tables(provider_doc.as_table_mut(), common_doc.as_table());
    Ok(provider_doc.to_string())
}

/// Write auth.json and config.toml files
async fn write_codex_config_files(
    db: Option<&surrealdb::Surreal<surrealdb::engine::local::Db>>,
    managed_auth: &serde_json::Value,
    previous_managed_config_toml: Option<&str>,
    next_managed_config_toml: &str,
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
    let merged_auth = merge_codex_auth_json(&existing_auth, managed_auth);
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
    let final_content = build_written_codex_config_toml(
        &existing_config_toml,
        previous_managed_config_toml,
        next_managed_config_toml,
    )?;
    fs::write(config_path, final_content)
        .map_err(|e| format!("Failed to write config.toml: {}", e))?;

    Ok(())
}

/// Apply Codex config to files
#[tauri::command]
pub async fn apply_codex_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(), String> {
    let db = state.db();
    apply_config_internal(&db, &app, &provider_id, false).await
}

/// Toggle is_disabled status for a provider
#[tauri::command]
pub async fn toggle_codex_provider_disabled(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
    is_disabled: bool,
) -> Result<(), String> {
    let db = state.db();

    // Update is_disabled field in database
    let now = Local::now().to_rfc3339();
    db.query(format!(
        "UPDATE codex_provider:`{}` SET is_disabled = $is_disabled, updated_at = $now",
        provider_id
    ))
    .bind(("is_disabled", is_disabled))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to toggle provider disabled status: {}", e))?;

    // If this provider is applied and now disabled, re-apply config to update files
    let toggle_id = db_record_id("codex_provider", &provider_id);
    let provider: Option<Value> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {}",
            toggle_id
        ))
        .await
        .map_err(|e| format!("Failed to query provider: {}", e))?
        .take(0)
        .map_err(|e| format!("Failed to parse provider: {}", e))?;

    if let Some(provider_value) = provider {
        let is_applied = provider_value
            .get("is_applied")
            .or_else(|| provider_value.get("isApplied"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_applied {
            // Re-apply config to update files (will check is_disabled internally)
            apply_config_internal(&db, &app, &provider_id, false).await?;
        }
    }

    Ok(())
}

/// Internal function to apply config
/// 使用 DELETE + CREATE 模式避免 SurrealDB MVCC 版本控制问题
pub async fn apply_config_internal<R: tauri::Runtime>(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    app: &tauri::AppHandle<R>,
    provider_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    // Apply config to files
    apply_config_to_file(db, provider_id).await?;

    // Update is_applied status using DELETE + CREATE pattern
    update_is_applied_status(db, provider_id).await?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-codex", ());

    Ok(())
}

// ============================================================================
// Codex Prompt Config Commands
// ============================================================================

#[tauri::command]
pub async fn list_codex_prompt_configs(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<CodexPromptConfig>, String> {
    let db = state.db();

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_prompt_config")
        .await
        .map_err(|e| format!("Failed to query prompt configs: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if records.is_empty() {
                if let Some(local_config) = get_local_prompt_config(Some(&db)).await? {
                    return Ok(vec![local_config]);
                }
                return Ok(Vec::new());
            }

            let mut result: Vec<CodexPromptConfig> = records
                .into_iter()
                .map(adapter::from_db_value_prompt)
                .collect();

            result.sort_by(|a, b| match (a.sort_index, b.sort_index) {
                (Some(ai), Some(bi)) => ai.cmp(&bi),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.cmp(&b.name),
            });

            Ok(result)
        }
        Err(e) => {
            eprintln!("Failed to deserialize Codex prompt configs: {}", e);
            if let Some(local_config) = get_local_prompt_config(Some(&db)).await? {
                return Ok(vec![local_config]);
            }
            Ok(Vec::new())
        }
    }
}

#[tauri::command]
pub async fn create_codex_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: CodexPromptConfigInput,
) -> Result<CodexPromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();

    let sort_index_result: Result<Vec<Value>, _> = db
        .query("SELECT sort_index FROM codex_prompt_config ORDER BY sort_index DESC LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query prompt sort index: {}", e))?
        .take(0);

    let next_sort_index = sort_index_result
        .ok()
        .and_then(|records| records.first().cloned())
        .and_then(|record| record.get("sort_index").and_then(|value| value.as_i64()))
        .map(|value| value as i32 + 1)
        .unwrap_or(0);

    let content = CodexPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_prompt(&content);
    let prompt_id = db_new_id();
    let record_id = db_record_id("codex_prompt_config", &prompt_id);

    db.query(&format!("CREATE {} CONTENT $data", record_id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create prompt config: {}", e))?;

    let records_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query created prompt config: {}", e))?
        .take(0);
    let created_config = match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::from_db_value_prompt(record.clone())
            } else {
                return Err("Failed to retrieve created prompt config".to_string());
            }
        }
        Err(e) => {
            return Err(format!(
                "Failed to deserialize created prompt config: {}",
                e
            ));
        }
    };

    let _ = app.emit("config-changed", "window");

    Ok(created_config)
}

#[tauri::command]
pub async fn update_codex_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    input: CodexPromptConfigInput,
) -> Result<CodexPromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let record_id = db_record_id("codex_prompt_config", &config_id);

    let existing_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT created_at, is_applied, sort_index FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query prompt config: {}", e))?
        .take(0);

    let (created_at, is_applied, sort_index) = match existing_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                let created_at = record
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| Box::leak(Local::now().to_rfc3339().into_boxed_str()))
                    .to_string();
                let is_applied = record
                    .get("is_applied")
                    .or_else(|| record.get("isApplied"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let sort_index = record
                    .get("sort_index")
                    .or_else(|| record.get("sortIndex"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                (created_at, is_applied, sort_index)
            } else {
                return Err(format!("Prompt config '{}' not found", config_id));
            }
        }
        Err(e) => return Err(format!("Failed to deserialize prompt config: {}", e)),
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
    let json_data = adapter::to_db_value_prompt(&content);

    db.query(&format!("UPDATE {} CONTENT $data", record_id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to update prompt config: {}", e))?;

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
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("codex_prompt_config", &id);
    let was_applied = db
        .query(&format!("SELECT is_applied FROM {} LIMIT 1", record_id))
        .await
        .ok()
        .and_then(|mut result| result.take::<Vec<Value>>(0).ok())
        .and_then(|records| records.into_iter().next())
        .and_then(|record| {
            record
                .get("is_applied")
                .or_else(|| record.get("isApplied"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete prompt config: {}", e))?;

    if was_applied {
        write_prompt_content_to_file(Some(&db), None).await?;
        emit_prompt_sync_requests(&app);
    }

    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub async fn apply_prompt_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    if config_id == "__local__" {
        let db = state.db();
        let local_prompt = get_local_prompt_config(Some(&db))
            .await?
            .ok_or_else(|| "Local default prompt not found".to_string())?;
        write_prompt_content_to_file(Some(&db), Some(local_prompt.content.as_str())).await?;

        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
        emit_prompt_sync_requests(app);

        return Ok(());
    }

    let db = state.db();
    let record_id = db_record_id("codex_prompt_config", config_id);
    let records_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query prompt config: {}", e))?
        .take(0);

    let prompt_config = match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::from_db_value_prompt(record.clone())
            } else {
                return Err(format!("Prompt config '{}' not found", config_id));
            }
        }
        Err(e) => return Err(format!("Failed to deserialize prompt config: {}", e)),
    };

    let now = Local::now().to_rfc3339();

    db.query("UPDATE codex_prompt_config SET is_applied = false, updated_at = $now WHERE is_applied = true")
        .bind(("now", now.clone()))
        .await
        .map_err(|e| format!("Failed to clear prompt applied flags: {}", e))?;

    db.query(&format!(
        "UPDATE {} SET is_applied = true, updated_at = $now",
        record_id
    ))
    .bind(("now", now))
    .await
    .map_err(|e| format!("Failed to set prompt applied flag: {}", e))?;
    write_prompt_content_to_file(Some(&db), Some(prompt_config.content.as_str())).await?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);
    emit_prompt_sync_requests(app);

    Ok(())
}

#[tauri::command]
pub async fn apply_codex_prompt_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_prompt_config_internal(state, &app, &config_id, false).await
}

#[tauri::command]
pub async fn reorder_codex_prompt_configs(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();

    for (index, id) in ids.iter().enumerate() {
        let record_id = db_record_id("codex_prompt_config", id);
        db.query(&format!("UPDATE {} SET sort_index = $index", record_id))
            .bind(("index", index as i32))
            .await
            .map_err(|e| format!("Failed to update prompt sort index: {}", e))?;
    }

    drop(db);
    let _ = app.emit("config-changed", "window");

    Ok(())
}

#[tauri::command]
pub async fn save_codex_local_prompt_config(
    state: tauri::State<'_, DbState>,
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
    let record_id = db_record_id("codex_prompt_config", &created.id);
    let refreshed_result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query saved local prompt config: {}", e))?
        .take(0);

    match refreshed_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::from_db_value_prompt(record.clone()))
            } else {
                Ok(created)
            }
        }
        Err(_) => Ok(created),
    }
}

#[tauri::command]
pub async fn list_codex_all_api_hub_providers(
    state: tauri::State<'_, DbState>,
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
    state: tauri::State<'_, DbState>,
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
    state: tauri::State<'_, DbState>,
) -> Result<CodexSettings, String> {
    let db = state.db();
    read_codex_settings_from_disk(Some(&db)).await
}

#[cfg(test)]
mod tests {
    use super::{
        append_toml_configs, build_written_codex_config_toml,
        extract_codex_common_config_from_settings_toml, extract_provider_settings_for_storage,
        infer_codex_provider_category_from_settings, merge_codex_auth_json,
        merge_remote_codex_official_models, normalize_codex_model_tier,
        static_codex_official_models, strip_codex_common_config_from_toml, RemoteCodexModel,
        CODEX_BUILTIN_IMAGE_MODEL_ID,
    };
    use serde_json::json;
    use toml_edit::DocumentMut;

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

[plugins.demo]
enabled = true

[mcp_servers.local]
command = "uvx"
"#;
        let common_toml = r#"
approval_policy = "never"

[features]
plugins = false

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
        assert_eq!(doc["plugins"]["demo"]["enabled"].as_bool(), Some(true));
        assert_eq!(doc["mcp_servers"]["local"]["command"].as_str(), Some("uvx"));
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
        assert!(doc.get("features").is_none());
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
"#;

        let common_toml = extract_codex_common_config_from_settings_toml(config_toml)
            .expect("extract common config");
        let doc: DocumentMut = common_toml.parse().expect("parse common config");

        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model").is_none());
        assert!(doc.get("model_providers").is_none());
        assert_eq!(doc["approval_policy"].as_str(), Some("never"));
        assert!(doc.get("features").is_none());
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
}

// ============================================================================
// Codex Common Config Commands
// ============================================================================

/// Get Codex common config
/// If database is empty, returns empty config (Codex doesn't have common config in local files)
#[tauri::command]
pub async fn get_codex_common_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<CodexCommonConfig>, String> {
    let db = state.db();

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query common config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(Some(adapter::from_db_value_common(record.clone())))
            } else {
                // Database is empty, return None (Codex doesn't have common config in local files)
                Ok(None)
            }
        }
        Err(e) => {
            // 反序列化失败，删除旧数据以修复版本冲突
            eprintln!(
                "⚠️ Codex common config has incompatible format, cleaning up: {}",
                e
            );
            let _ = db.query("DELETE codex_common_config:`common`").await;
            let _ = runtime_location::refresh_runtime_location_cache_for_module_async(&db, "codex")
                .await;
            Ok(None)
        }
    }
}

#[tauri::command]
pub async fn extract_codex_common_config_from_current_file(
    state: tauri::State<'_, DbState>,
) -> Result<CodexCommonConfig, String> {
    let db = state.db();
    extract_codex_common_config_from_current_files_with_db(&db).await
}

/// Save Codex common config
#[tauri::command]
pub async fn save_codex_common_config(
    state: tauri::State<'_, DbState>,
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

    // Use UPSERT to handle both update and create
    db.query("UPSERT codex_common_config:`common` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save config: {}", e))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "codex").await?;

    // Re-apply current provider config to write merged config to file
    let applied_result: Result<Vec<Value>, _> = db
        .query(
            "SELECT *, type::string(id) as id FROM codex_provider WHERE is_applied = true LIMIT 1",
        )
        .await
        .map_err(|e| format!("Failed to query applied provider: {}", e))?
        .take(0);

    if let Ok(records) = applied_result {
        if let Some(record) = records.first() {
            let provider = adapter::from_db_value_provider(record.clone());
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
    state: tauri::State<'_, DbState>,
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
    let previous_managed_settings =
        extract_provider_settings_for_storage(&current_live_settings_value, None)?;
    let previous_managed_config_toml = previous_managed_settings
        .get("config")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

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
        is_applied: true,
        is_disabled: provider_is_disabled,
        created_at: now.clone(),
        updated_at: now,
    };

    let provider_json = adapter::to_db_value_provider(&provider_content);
    db.query("CREATE codex_provider CONTENT $data")
        .bind(("data", provider_json))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

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
    db.query("UPSERT codex_common_config:`common` CONTENT $data")
        .bind(("data", common_json))
        .await
        .map_err(|e| format!("Failed to save common config: {}", e))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "codex").await?;

    // Re-apply config to files using the newly created provider
    let created_result: Result<Vec<Value>, _> = db
        .query(
            "SELECT *, type::string(id) as id FROM codex_provider ORDER BY created_at DESC LIMIT 1",
        )
        .await
        .map_err(|e| format!("Failed to fetch created provider: {}", e))?
        .take(0);
    if let Ok(records) = created_result {
        if let Some(record) = records.first() {
            let created_provider = adapter::from_db_value_provider(record.clone());
            if let Err(e) = apply_config_to_file_with_previous_managed_config(
                &db,
                &created_provider.id,
                Some(previous_managed_config_toml),
            )
            .await
            {
                eprintln!("Failed to apply config after local save: {}", e);
            } else {
                #[cfg(target_os = "windows")]
                let _ = app.emit("wsl-sync-request-codex", ());
            }
        }
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(), String> {
    // Check if any providers exist by querying for one record
    let check_result: Result<Vec<Value>, _> = db
        .query("SELECT * OMIT id FROM codex_provider LIMIT 1")
        .await
        .map_err(|e| format!("Failed to check providers: {}", e))?
        .take(0);

    let has_providers = match check_result {
        Ok(records) => !records.is_empty(),
        Err(_) => false,
    };

    if has_providers {
        return Ok(());
    }

    // Check if config files exist
    let root_dir = get_codex_root_dir_without_db()?;
    let auth_path = root_dir.join("auth.json");
    let config_path = root_dir.join("config.toml");
    if !auth_path.exists() && !config_path.exists() {
        return Ok(());
    }

    // Read auth.json (optional)
    let auth: serde_json::Value = if auth_path.exists() {
        let auth_content = fs::read_to_string(&auth_path)
            .map_err(|e| format!("Failed to read auth.json: {}", e))?;
        serde_json::from_str(&auth_content)
            .map_err(|e| format!("Failed to parse auth.json: {}", e))?
    } else {
        serde_json::json!({})
    };

    // Read config.toml (optional)
    let config_toml = if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Build settings_config
    let settings = serde_json::json!({
        "auth": auth,
        "config": config_toml
    });
    let common_toml = get_codex_common_toml(db).await?;
    let provider_settings =
        extract_provider_settings_for_storage(&settings, common_toml.as_deref())?;

    let now = Local::now().to_rfc3339();
    let content = CodexProviderContent {
        name: "默认配置".to_string(),
        category: infer_codex_provider_category_from_settings(&provider_settings),
        settings_config: serde_json::to_string(&provider_settings).unwrap_or_default(),
        source_provider_id: None,
        website_url: None,
        notes: Some("从配置文件自动导入".to_string()),
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        is_applied: true,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let json_data = adapter::to_db_value_provider(&content);

    // Create new provider with auto-generated random ID
    db.query("CREATE codex_provider CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to create provider: {}", e))?;

    println!("✅ Imported Codex settings as default provider");
    Ok(())
}
