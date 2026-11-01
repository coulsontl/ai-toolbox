use chrono::Local;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::adapter;
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::open_code::shell_env;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::runtime_location;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put, db_query_by_bool,
    db_update_applied_status,
};
use crate::db::schema::{Cli, CliConfigKind, DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use crate::http_client;
use tauri::Emitter;

#[allow(dead_code)]
const MANAGED_ENV_KEYS: [&str; 14] = [
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GEMINI_BASE_URL",
    "GOOGLE_VERTEX_BASE_URL",
    "GOOGLE_GENAI_USE_GCA",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GEMINI_CLI_USE_COMPUTE_ADC",
    "GEMINI_CLI_CUSTOM_HEADERS",
    "GEMINI_MODEL",
    "GEMINI_API_KEY_AUTH_MECHANISM",
    "GOOGLE_GENAI_API_VERSION",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_PROJECT_ID",
    "GOOGLE_CLOUD_LOCATION",
];

const OFFICIAL_PROVIDER_REMOVED_ENV_KEYS: [&str; 13] = [
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GEMINI_BASE_URL",
    "GOOGLE_VERTEX_BASE_URL",
    "GOOGLE_GENAI_USE_GCA",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GEMINI_CLI_USE_COMPUTE_ADC",
    "GEMINI_CLI_CUSTOM_HEADERS",
    "GEMINI_API_KEY_AUTH_MECHANISM",
    "GOOGLE_GENAI_API_VERSION",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_PROJECT_ID",
    "GOOGLE_CLOUD_LOCATION",
];

const ANTIGRAVITY_OFFICIAL_AUTH_TYPE: &str = "oauth-personal";
pub const ANTIGRAVITY_CLI_HOME_ENV_KEY: &str = "ANTIGRAVITY_CLI_HOME";
pub const DEFAULT_ANTIGRAVITY_PROMPT_FILE: &str = "GEMINI.md";
#[allow(dead_code)]
const ANTIGRAVITY_NO_LOCAL_PROVIDER_CONFIG_ERROR: &str =
    "No Antigravity local provider config found";

const ANTIGRAVITY_MODEL_CATALOG_URLS: [&str; 2] = [
    "https://raw.githubusercontent.com/router-for-me/models/refs/heads/main/models.json",
    "https://models.router-for.me/models.json",
];

const ANTIGRAVITY_ALIAS_MODEL_IDS: [&str; 6] = [
    "auto",
    "auto-gemini-3",
    "auto-gemini-2.5",
    "pro",
    "flash",
    "flash-lite",
];

const ANTIGRAVITY_SOURCE_MODEL_IDS: [&str; 10] = [
    "gemini-3.1-flash-lite-preview",
    "gemini-3.1-pro-preview",
    "gemini-3.1-pro-preview-customtools",
    "gemini-3-pro-preview",
    "gemini-3-flash-preview",
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "gemini-2.5-flash-lite",
    "gemini-2.0-pro-exp-02-05",
    "gemini-2.0-flash",
];

#[derive(Debug, Clone, Deserialize)]
struct RemoteModelCatalogResponse {
    #[serde(default)]
    models: Vec<RemoteModelCatalogItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteModelCatalogItem {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<i64>,
}

pub fn get_antigravity_default_root_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    Ok(home.join(".gemini").join("antigravity-cli"))
}

pub fn get_antigravity_root_dir_from_home_override(home_dir: &str) -> Option<PathBuf> {
    let trimmed = home_dir.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(trimmed);
    if candidate.file_name().and_then(|name| name.to_str()) == Some("antigravity-cli") {
        Some(candidate)
    } else {
        Some(candidate.join(".gemini").join("antigravity-cli"))
    }
}

pub fn get_antigravity_root_dir_from_env() -> Option<PathBuf> {
    std::env::var(ANTIGRAVITY_CLI_HOME_ENV_KEY)
        .ok()
        .and_then(|value| get_antigravity_root_dir_from_home_override(&value))
}

pub fn get_antigravity_root_dir_from_db(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    let common_config = get_antigravity_common_config_from_db(db)?;
    if let Some(custom_root_dir) = common_config.root_dir.filter(|dir| !dir.trim().is_empty()) {
        return Ok(PathBuf::from(custom_root_dir));
    }
    if let Some(env_root_dir) = get_antigravity_root_dir_from_env() {
        return Ok(env_root_dir);
    }
    if let Some(shell_home) = shell_env::get_env_from_shell_config(ANTIGRAVITY_CLI_HOME_ENV_KEY) {
        if let Some(shell_root_dir) = get_antigravity_root_dir_from_home_override(&shell_home) {
            return Ok(shell_root_dir);
        }
    }
    get_antigravity_default_root_dir()
}

/// Resolve the Antigravity runtime root without database access.
///
/// Priority is env override > shell config > platform default, matching the
/// DB-backed resolver minus the custom root stored in `antigravity_common_config`.
/// Backup restore runs before the restored DB is available, so it needs this
/// variant to learn where `.env` / `settings.json` / `tmp` should land.
pub(crate) fn get_antigravity_root_dir_without_db() -> Result<PathBuf, String> {
    if let Some(env_root_dir) = get_antigravity_root_dir_from_env() {
        return Ok(env_root_dir);
    }
    if let Some(shell_root_dir) = shell_env::get_env_from_shell_config(ANTIGRAVITY_CLI_HOME_ENV_KEY)
        .and_then(|home_dir| get_antigravity_root_dir_from_home_override(&home_dir))
    {
        return Ok(shell_root_dir);
    }
    get_antigravity_default_root_dir()
}

pub fn get_antigravity_root_path_info_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<ConfigPathInfo, String> {
    let common_config = get_antigravity_common_config_from_db(db)?;
    if let Some(custom_root_dir) = common_config.root_dir.filter(|dir| !dir.trim().is_empty()) {
        return Ok(ConfigPathInfo {
            path: custom_root_dir,
            source: "custom".to_string(),
        });
    }
    if let Some(env_root_dir) = get_antigravity_root_dir_from_env() {
        return Ok(ConfigPathInfo {
            path: env_root_dir.to_string_lossy().to_string(),
            source: "env".to_string(),
        });
    }
    if let Some(shell_home) = shell_env::get_env_from_shell_config(ANTIGRAVITY_CLI_HOME_ENV_KEY) {
        if let Some(shell_root_dir) = get_antigravity_root_dir_from_home_override(&shell_home) {
            return Ok(ConfigPathInfo {
                path: shell_root_dir.to_string_lossy().to_string(),
                source: "shell".to_string(),
            });
        }
    }
    let default_root_dir = get_antigravity_default_root_dir()?;
    Ok(ConfigPathInfo {
        path: default_root_dir.to_string_lossy().to_string(),
        source: "default".to_string(),
    })
}

/// Directory `agy` treats as its global customization root.
///
/// Verified against a real install: a marker written to
/// `~/.gemini/config/GEMINI.md` is picked up by `agy --print`, while the same
/// marker under `~/.gemini/antigravity-cli/` is ignored. The directory is
/// derived from `$HOME` only, so `ANTIGRAVITY_CLI_HOME` and the app-level custom
/// root directory do NOT move it.
pub fn get_antigravity_global_rules_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    Ok(home.join(".gemini").join("config"))
}

/// Global rules file AI Toolbox manages for `agy`.
///
/// `agy` discovers `GEMINI.md`, `AGENTS.md`, `rules/AGENTS.md` and
/// `.agents/rules/*.md`; `GEMINI.md` is the documented standalone default.
pub fn get_antigravity_prompt_path() -> Result<PathBuf, String> {
    Ok(get_antigravity_global_rules_dir()?.join(DEFAULT_ANTIGRAVITY_PROMPT_FILE))
}

/// Prompt file inside an explicit global rules directory.
pub fn get_antigravity_prompt_path_from_global_rules_dir(global_rules_dir: &Path) -> PathBuf {
    global_rules_dir.join(DEFAULT_ANTIGRAVITY_PROMPT_FILE)
}

pub async fn get_antigravity_env_path_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    runtime_location::get_antigravity_env_path_async(db).await
}

pub async fn get_antigravity_settings_path_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    runtime_location::get_antigravity_settings_path_async(db).await
}

pub async fn get_antigravity_prompt_path_from_db_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    runtime_location::get_antigravity_prompt_path_async(db).await
}

pub fn get_antigravity_env_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    runtime_location::get_antigravity_env_path_sync(db)
}

pub fn get_antigravity_settings_path_sync(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    runtime_location::get_antigravity_settings_path_sync(db)
}

pub fn get_antigravity_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    runtime_location::get_antigravity_prompt_path_sync(db)
}

fn parse_env_content(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let clean_key = key.trim().to_string();
        if clean_key.is_empty() {
            continue;
        }
        let mut clean_value = value.trim().to_string();
        if (clean_value.starts_with('"') && clean_value.ends_with('"') && clean_value.len() >= 2)
            || (clean_value.starts_with('\'')
                && clean_value.ends_with('\'')
                && clean_value.len() >= 2)
        {
            clean_value = clean_value[1..clean_value.len() - 1].to_string();
        }
        map.insert(clean_key, clean_value);
    }
    map
}

fn serialize_env_content(map: &BTreeMap<String, String>) -> String {
    let mut output = String::new();
    for (key, value) in map {
        if value.contains(' ') || value.contains('"') || value.contains('\n') {
            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            output.push_str(&format!("{key}=\"{escaped}\"\n"));
        } else {
            output.push_str(&format!("{key}={value}\n"));
        }
    }
    output
}

/// Remove previously-managed keys from a `.env` file while preserving
/// user-owned keys. Returns the cleaned content and whether anything changed.
fn remove_managed_env_keys(original_env_content: &str, keys_to_remove: &[&str]) -> (String, bool) {
    let mut parsed = parse_env_content(original_env_content);
    let mut changed = false;
    for key in keys_to_remove {
        if parsed.remove(*key).is_some() {
            changed = true;
        }
    }
    (serialize_env_content(&parsed), changed)
}

/// Keep the live Antigravity settings in official OAuth mode.
fn apply_model_provider_state(settings: &mut Value) {
    let Some(obj) = settings.as_object_mut() else {
        return;
    };
    obj.remove("modelProvider");
}

fn merge_json_values(base: &mut Value, override_val: &Value) {
    match (base, override_val) {
        (Value::Object(base_map), Value::Object(override_map)) => {
            for (k, v) in override_map {
                merge_json_values(base_map.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (base_slot, override_val) => {
            *base_slot = override_val.clone();
        }
    }
}

pub fn normalize_antigravity_settings_config_for_save(
    raw_settings_config: &str,
) -> Result<String, String> {
    let mut parsed: Value = if raw_settings_config.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(raw_settings_config)
            .map_err(|e| format!("Invalid JSON in settings_config: {e}"))?
    };

    if !parsed.is_object() {
        parsed = Value::Object(Map::new());
    }

    let config_obj = parsed
        .as_object_mut()
        .unwrap()
        .entry("config")
        .or_insert_with(|| Value::Object(Map::new()));
    if !config_obj.is_object() {
        *config_obj = Value::Object(Map::new());
    }
    let sec = config_obj
        .as_object_mut()
        .unwrap()
        .entry("security")
        .or_insert_with(|| Value::Object(Map::new()));
    if !sec.is_object() {
        *sec = Value::Object(Map::new());
    }
    let auth = sec
        .as_object_mut()
        .unwrap()
        .entry("auth")
        .or_insert_with(|| Value::Object(Map::new()));
    if !auth.is_object() {
        *auth = Value::Object(Map::new());
    }
    auth["selectedType"] = json!(ANTIGRAVITY_OFFICIAL_AUTH_TYPE);

    if let Some(env_obj) = parsed.get_mut("env").and_then(Value::as_object_mut) {
        for key in OFFICIAL_PROVIDER_REMOVED_ENV_KEYS {
            env_obj.remove(key);
        }
    }

    Ok(serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string()))
}

pub fn query_provider_by_id(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<AntigravityProvider, String> {
    let value = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Antigravity), provider_id))?
        .ok_or_else(|| format!("Antigravity provider not found: {provider_id}"))?;
    Ok(adapter::from_db_value_provider(value))
}

pub fn get_antigravity_common_config_from_db(
    db: &SqliteDbState,
) -> Result<AntigravityCommonConfig, String> {
    let record = db.with_conn(|conn| db_get(conn, DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Common), "common"))?;
    match record {
        Some(val) => Ok(adapter::from_db_value_common(val)),
        None => Ok(AntigravityCommonConfig {
            config: "{}".to_string(),
            root_dir: None,
            updated_at: Local::now().to_rfc3339(),
        }),
    }
}

#[tauri::command]
pub async fn get_antigravity_config_path(
    db: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    let settings_path = get_antigravity_settings_path_from_db_async(&db).await?;
    Ok(settings_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_antigravity_root_path_info(
    db: tauri::State<'_, SqliteDbState>,
) -> Result<ConfigPathInfo, String> {
    get_antigravity_root_path_info_from_db(&db)
}

#[tauri::command]
pub async fn reveal_antigravity_config_folder(
    db: tauri::State<'_, SqliteDbState>,
) -> Result<(), String> {
    let settings_path = get_antigravity_settings_path_from_db_async(&db).await?;
    let folder = settings_path
        .parent()
        .ok_or_else(|| "Failed to get config directory".to_string())?;
    tauri_plugin_opener::reveal_item_in_dir(folder)
        .map_err(|e| format!("Failed to open config folder: {e}"))
}

#[tauri::command]
pub async fn read_antigravity_settings(
    db: tauri::State<'_, SqliteDbState>,
) -> Result<AntigravitySettings, String> {
    let settings_path = get_antigravity_settings_path_from_db_async(&db).await?;

    let settings_val = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path).unwrap_or_default();
        serde_json::from_str::<Value>(&content).ok()
    } else {
        None
    };

    Ok(AntigravitySettings {
        env: None,
        config: settings_val,
    })
}

#[tauri::command]
pub async fn fetch_antigravity_official_models(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<AntigravityOfficialModelsResponse, String> {
    if let Ok(client) = http_client::client_with_timeout(&state, 30).await {
        for url in ANTIGRAVITY_MODEL_CATALOG_URLS {
            if let Ok(res) = client.get(url).send().await {
                if res.status().is_success() {
                    if let Ok(catalog) = res.json::<RemoteModelCatalogResponse>().await {
                        let models: Vec<AntigravityOfficialModel> = catalog
                            .models
                            .into_iter()
                            .map(|m| AntigravityOfficialModel {
                                id: m.id,
                                name: m.name,
                                owned_by: m.owned_by,
                                created: m.created,
                            })
                            .collect();
                        if !models.is_empty() {
                            return Ok(AntigravityOfficialModelsResponse {
                                total: models.len(),
                                models,
                                source: "remote".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    let mut models = Vec::new();
    for alias in ANTIGRAVITY_ALIAS_MODEL_IDS {
        models.push(AntigravityOfficialModel {
            id: alias.to_string(),
            name: Some(alias.to_string()),
            owned_by: Some("google".to_string()),
            created: None,
        });
    }
    for source in ANTIGRAVITY_SOURCE_MODEL_IDS {
        models.push(AntigravityOfficialModel {
            id: source.to_string(),
            name: Some(source.to_string()),
            owned_by: Some("google".to_string()),
            created: None,
        });
    }

    Ok(AntigravityOfficialModelsResponse {
        total: models.len(),
        models,
        source: "builtin".to_string(),
    })
}

fn antigravity_provider_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::single(OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?))
}

pub async fn init_antigravity_provider_from_settings(
    db: &crate::db::SqliteDbState,
) -> Result<(), String> {
    let providers = list_antigravity_providers_from_sqlite(db)?;
    if providers
        .iter()
        .any(|provider| provider.category == "official")
    {
        return Ok(());
    }

    let now = Local::now().to_rfc3339();
    let content = AntigravityProviderContent {
        name: "Google Official".to_string(),
        category: "official".to_string(),
        settings_config: normalize_antigravity_settings_config_for_save(
            r#"{"config":{"security":{"auth":{"selectedType":"oauth-personal"}}}}"#,
        )?,
        source_provider_id: None,
        website_url: Some("https://antigravity.google".to_string()),
        notes: Some("Antigravity Google OAuth account".to_string()),
        icon: None,
        icon_color: None,
        sort_index: Some(0),
        meta: None,
        is_applied: false,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };
    put_antigravity_provider_to_sqlite(db, &db_new_id(), &content)?;
    Ok(())
}

pub(crate) fn put_antigravity_provider_to_sqlite(
    sqlite_state: &SqliteDbState,
    provider_id: &str,
    content: &AntigravityProviderContent,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CliProvider(Cli::Antigravity),
            provider_id,
            &adapter::to_db_value_provider(content),
        )
    })
}

pub(crate) fn list_antigravity_providers_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<AntigravityProvider>, String> {
    let order = antigravity_provider_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::CliProvider(Cli::Antigravity), Some(&order))?
            .into_iter()
            .map(adapter::from_db_value_provider)
            .collect())
    })
}

#[tauri::command]
pub async fn get_antigravity_official_provider(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<AntigravityProvider, String> {
    let db = state.db();
    init_antigravity_provider_from_settings(db).await?;
    list_antigravity_providers_from_sqlite(db)?
        .into_iter()
        .find(|provider| provider.category == "official")
        .ok_or_else(|| "Antigravity official provider not found".to_string())
}

pub async fn apply_config_internal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    apply_config_internal_with_sync(app, db, provider_id, true).await
}

pub async fn apply_config_internal_with_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    db: &SqliteDbState,
    provider_id: &str,
    emit_sync: bool,
) -> Result<(), String> {
    apply_config_internal_without_events(db, provider_id).await?;
    let _ = app.emit("config-changed", ());
    if emit_sync {
        let _ = app.emit("wsl-sync-request-antigravity", ());
    }
    Ok(())
}

pub async fn apply_config_internal_without_events(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let provider = query_provider_by_id(db, provider_id)?;
    if provider.category != "official" {
        return Err("Antigravity only supports the official Google OAuth provider".to_string());
    }
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider_id
        ));
    }
    let parsed_provider_settings: Value = serde_json::from_str(&provider.settings_config)
        .unwrap_or_else(|_| Value::Object(Map::new()));

    let env_path = get_antigravity_env_path_from_db_async(db).await?;
    let settings_path = get_antigravity_settings_path_from_db_async(db).await?;

    // 1. Antigravity CLI (agy) 不读 .env 文件。
    // 如果存在由旧版本或外部遗留的 .env 文件，清理其中由本应用管理的键，
    // 避免给排障或用户造成".env 正在生效"的误导。若清理后文件变空则移除。
    if env_path.exists() {
        let current_env_content = fs::read_to_string(&env_path).unwrap_or_default();
        let (cleaned_env, changed) =
            remove_managed_env_keys(&current_env_content, &MANAGED_ENV_KEYS);
        if changed {
            if cleaned_env.trim().is_empty() {
                let _ = fs::remove_file(&env_path);
            } else {
                let _ = fs::write(&env_path, cleaned_env);
            }
        }
    }

    // 2. 写入官方规范的 settings.json
    if let Some(parent) = settings_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut current_settings: Value = if settings_path.exists() {
        fs::read_to_string(&settings_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_else(|| Value::Object(Map::new()))
    } else {
        Value::Object(Map::new())
    };

    // 官方规范：
    // custom: 写入 "modelProvider": "gemini" 开启 API-key / 自定义端点模式
    // official: 移除 "modelProvider" 恢复默认的账号 OAuth 登录模式
    apply_model_provider_state(&mut current_settings);

    let common = get_antigravity_common_config_from_db(db)?;
    if let Ok(common_val) = serde_json::from_str::<Value>(&common.config) {
        if common_val.is_object() {
            merge_json_values(&mut current_settings, &common_val);
        }
    }

    // Common settings provide defaults; the selected provider must win on
    // overlapping keys, matching Gemini CLI's apply semantics.
    if let Some(provider_config) = parsed_provider_settings.get("config") {
        merge_json_values(&mut current_settings, provider_config);
    }
    // Ensure the official/custom mode cannot be overridden by either JSON
    // payload.
    apply_model_provider_state(&mut current_settings);

    let serialized_settings = serde_json::to_string_pretty(&current_settings)
        .map_err(|e| format!("Failed to format settings JSON: {e}"))?;
    fs::write(&settings_path, serialized_settings).map_err(|e| {
        format!(
            "Failed to write settings file at {}: {e}",
            settings_path.display()
        )
    })?;

    let now_str = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(
            conn,
            DbTable::CliProvider(Cli::Antigravity),
            Some(provider_id),
            &now_str,
        )
    })?;

    Ok(())
}

#[tauri::command]
pub async fn get_antigravity_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<AntigravityCommonConfig, String> {
    let db = state.db();
    get_antigravity_common_config_from_db(db)
}

#[tauri::command]
pub async fn extract_antigravity_common_config_from_current_file(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<AntigravityCommonConfig, String> {
    let db = state.db();
    let settings_path = get_antigravity_settings_path_from_db_async(db).await?;
    let Some(content) = crate::coding::file_io::read_optional_text_file_with_timeout(
        settings_path.clone(),
        "Antigravity settings.json",
    )
    .await?
    else {
        return Ok(AntigravityCommonConfig {
            config: "{}".to_string(),
            root_dir: settings_path
                .parent()
                .map(|path| path.to_string_lossy().to_string()),
            updated_at: Local::now().to_rfc3339(),
        });
    };
    Ok(AntigravityCommonConfig {
        config: content,
        root_dir: settings_path
            .parent()
            .map(|path| path.to_string_lossy().to_string()),
        updated_at: Local::now().to_rfc3339(),
    })
}

#[tauri::command]
pub async fn save_antigravity_common_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    input: AntigravityCommonConfigInput,
) -> Result<AntigravityCommonConfig, String> {
    let db = state.db();
    let current = get_antigravity_common_config_from_db(db)?;
    let target_root = if input.clear_root_dir {
        None
    } else {
        input.root_dir.or(current.root_dir)
    };

    let db_val = adapter::to_db_value_common(&input.config, target_root.as_deref());
    db.with_conn(|conn| db_put(conn, DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Common), "common", &db_val))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(db, "antigravity").await?;

    let applied_provider = db
        .with_conn(|conn| {
            db_query_by_bool(
                conn,
                DbTable::CliProvider(Cli::Antigravity),
                &JsonFieldPath::new("is_applied")?,
                true,
                None,
                Some(1),
            )
        })?
        .into_iter()
        .next()
        .map(adapter::from_db_value_provider);

    if let Some(provider) = applied_provider {
        apply_config_internal(&app, db, &provider.id).await?;
    }

    let _ = app.emit("config-changed", ());
    let _ = app.emit("wsl-sync-request-antigravity", ());

    get_antigravity_common_config_from_db(db)
}

fn antigravity_prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn list_antigravity_prompts_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<AntigravityPromptConfig>, String> {
    let order = antigravity_prompt_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(
            db_list(conn, DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt), Some(&order))?
                .into_iter()
                .map(adapter::from_db_value_prompt)
                .collect(),
        )
    })
}

fn get_antigravity_prompt_from_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
) -> Result<Option<AntigravityPromptConfig>, String> {
    sqlite_state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt), config_id)?
            .map(adapter::from_db_value_prompt))
    })
}

fn put_antigravity_prompt_to_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
    content: &AntigravityPromptConfigContent,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt),
            config_id,
            &adapter::to_db_value_prompt(content),
        )
    })
}

async fn get_local_prompt_config(
    db: Option<&crate::db::SqliteDbState>,
) -> Result<Option<AntigravityPromptConfig>, String> {
    // The global rules file lives in `~/.gemini/config`, independent of the
    // runtime root directory, so both branches resolve the same path.
    let _ = db;
    let prompt_path = get_antigravity_prompt_path()?;
    let Some(prompt_content) = read_prompt_content_file(&prompt_path, "Antigravity")? else {
        return Ok(None);
    };
    let now = Local::now().to_rfc3339();
    Ok(Some(AntigravityPromptConfig {
        id: crate::coding::local_bridge::LOCAL_CONFIG_ID.to_string(),
        name: "default".to_string(),
        content: prompt_content,
        is_applied: true,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }))
}

#[tauri::command]
pub async fn list_antigravity_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<AntigravityPromptConfig>, String> {
    let db = state.db();
    let prompts = list_antigravity_prompts_from_sqlite(db)?;
    if prompts.is_empty() {
        if let Some(local_config) = get_local_prompt_config(Some(db)).await? {
            return Ok(vec![local_config]);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_antigravity_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: AntigravityPromptConfigInput,
) -> Result<AntigravityPromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let next_sort_index = db.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt),
            &JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;

    let content = AntigravityPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };
    let prompt_id = db_new_id();
    put_antigravity_prompt_to_sqlite(db, &prompt_id, &content)?;

    let _ = app.emit("config-changed", "window");
    Ok(AntigravityPromptConfig {
        id: prompt_id,
        name: content.name,
        content: content.content,
        is_applied: content.is_applied,
        sort_index: content.sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    })
}

#[tauri::command]
pub async fn update_antigravity_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: AntigravityPromptConfigInput,
) -> Result<AntigravityPromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "Prompt config ID is required for update".to_string())?;

    let db = state.db();
    let now = Local::now().to_rfc3339();
    let existing_prompt = get_antigravity_prompt_from_sqlite(db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;
    let (created_at, is_applied, sort_index) = {
        let prompt = existing_prompt;
        (
            prompt.created_at.unwrap_or_else(|| now.clone()),
            prompt.is_applied,
            prompt.sort_index,
        )
    };

    let content = AntigravityPromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied,
        sort_index,
        created_at,
        updated_at: now.clone(),
    };
    put_antigravity_prompt_to_sqlite(db, &config_id, &content)?;

    if is_applied {
        let prompt_path = get_antigravity_prompt_path_from_db_async(db).await?;
        write_prompt_content_file(&prompt_path, Some(input.content.as_str()), "Antigravity")?;
        let _ = app.emit("wsl-sync-request-antigravity", ());
    }

    let _ = app.emit("config-changed", "window");
    Ok(AntigravityPromptConfig {
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
pub async fn delete_antigravity_prompt_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    db.with_conn(|conn| db_delete(conn, DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt), &id))?;
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
    let db = state.db();
    let record = db
        .with_conn(|conn| db_get(conn, DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt), config_id))?
        .ok_or_else(|| format!("Antigravity prompt config not found: {config_id}"))?;
    let config = adapter::from_db_value_prompt(record);

    let prompt_path = get_antigravity_prompt_path_from_db_async(db).await?;
    write_prompt_content_file(&prompt_path, Some(&config.content), "Antigravity")?;

    let now_str = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(
            conn,
            DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt),
            Some(config_id),
            &now_str,
        )
    })?;

    if emit_events {
        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
        let _ = app.emit("wsl-sync-request-antigravity", ());
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_antigravity_prompt_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    id: String,
) -> Result<(), String> {
    apply_prompt_config_internal(state, &app, &id, false).await
}

#[tauri::command]
pub async fn disable_antigravity_prompt_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    get_antigravity_prompt_from_sqlite(db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    // 与其它模块保持一致：「禁用」是唯一显式清除生效提示词的语义，保留 DB 记录
    // 内容不动，只把全部 is_applied 置 false 并清空运行时规则文件。
    let prompt_path = get_antigravity_prompt_path_from_db_async(db).await?;
    write_prompt_content_file(&prompt_path, None, "Antigravity")?;

    let now_str = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt), None, &now_str)
    })?;

    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-antigravity", ());
    Ok(())
}

#[tauri::command]
pub async fn reorder_antigravity_prompt_configs(
    app: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    for (index, id) in ids.iter().enumerate() {
        let _ = db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Prompt),
                id,
                &[("sort_index", Value::Number((index as i32).into()))],
            )
        });
    }
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn save_antigravity_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: AntigravityPromptConfigInput,
) -> Result<AntigravityPromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        let db = state.db();
        get_local_prompt_config(Some(db))
            .await?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };
    let created = create_antigravity_prompt_config(
        state.clone(),
        app.clone(),
        AntigravityPromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;
    apply_prompt_config_internal(state.clone(), &app, &created.id, false).await?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_model_provider_state_official_removes_key() {
        let mut settings = json!({
            "modelProvider": "gemini",
            "otherField": true
        });
        apply_model_provider_state(&mut settings);
        assert!(settings.get("modelProvider").is_none());
        assert_eq!(settings["otherField"], true);
    }

    #[test]
    fn test_remove_managed_env_keys_preserves_user_keys() {
        let env_content = "GEMINI_API_KEY=test-key\nUSER_CUSTOM_VAR=hello\nGOOGLE_GEMINI_BASE_URL=https://example.com\n";
        let (cleaned, changed) = remove_managed_env_keys(env_content, &MANAGED_ENV_KEYS);
        assert!(changed);
        assert!(!cleaned.contains("GEMINI_API_KEY"));
        assert!(!cleaned.contains("GOOGLE_GEMINI_BASE_URL"));
        assert!(cleaned.contains("USER_CUSTOM_VAR=hello"));
    }

    #[test]
    fn test_normalize_antigravity_settings_config() {
        let custom = r#"{"env":{"GEMINI_API_KEY":"test"},"config":{"other":1}}"#;
        let normalized = normalize_antigravity_settings_config_for_save(custom).unwrap();
        let parsed: Value = serde_json::from_str(&normalized).unwrap();
        assert_eq!(
            parsed.pointer("/config/security/auth/selectedType"),
            Some(&json!(ANTIGRAVITY_OFFICIAL_AUTH_TYPE))
        );
        assert!(parsed.pointer("/env/GEMINI_API_KEY").is_none());
    }
}
