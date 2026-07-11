use chrono::Local;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::adapter;
use super::constants::{
    PI_AUTH_FILE, PI_BUILTIN_PROVIDERS, PI_ENV_KEY, PI_MCP_FILE, PI_MODELS_FILE, PI_PROMPT_FILE,
    PI_SETTINGS_FILE, builtin_provider_name, is_builtin_provider,
};
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::open_code::shell_env;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::runtime_location;
use crate::coding::skills::commands::resync_all_skills_if_tool_path_changed;
use crate::db::SqliteDbState;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use tauri::{Emitter, Runtime};

const PI_THINKING_LEVEL_KEYS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
const PI_EXTENDED_THINKING_LEVEL_KEYS: [&str; 2] = ["xhigh", "max"];
const PI_OTHER_SETTINGS_PROTECTED_KEYS: [&str; 1] = ["packages"];

fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

pub fn get_pi_default_root_dir() -> Result<PathBuf, String> {
    Ok(get_home_dir()?.join(".pi").join("agent"))
}

fn get_pi_root_dir_from_shell() -> Option<PathBuf> {
    shell_env::get_env_from_shell_config(PI_ENV_KEY)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub fn get_pi_root_dir_without_db() -> Result<PathBuf, String> {
    if let Ok(env_path) = std::env::var(PI_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return Ok(PathBuf::from(env_path));
        }
    }
    if let Some(shell_path) = get_pi_root_dir_from_shell() {
        return Ok(shell_path);
    }
    get_pi_default_root_dir()
}

pub async fn get_pi_custom_root_dir_async(db: &SqliteDbState) -> Option<PathBuf> {
    db.with_conn(|conn| db_get(conn, DbTable::PiSettingsConfig, "common"))
        .ok()
        .flatten()
        .and_then(|value| adapter::settings_from_db_value(value).root_dir)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub fn get_pi_root_path_info_from_db(db: &SqliteDbState) -> Result<PiPathInfo, String> {
    let location = runtime_location::get_pi_runtime_location_sync(db)?;
    Ok(PiPathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

pub async fn get_pi_root_path_info_from_db_async(db: &SqliteDbState) -> Result<PiPathInfo, String> {
    let location = runtime_location::get_pi_runtime_location_async(db).await?;
    Ok(PiPathInfo {
        path: location.host_path.to_string_lossy().to_string(),
        source: location.source,
    })
}

pub async fn get_pi_root_dir_from_db_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(runtime_location::get_pi_runtime_location_async(db)
        .await?
        .host_path)
}

pub fn get_pi_settings_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(PI_SETTINGS_FILE)
}

pub fn get_pi_auth_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(PI_AUTH_FILE)
}

pub fn get_pi_models_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(PI_MODELS_FILE)
}

pub fn get_pi_mcp_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(PI_MCP_FILE)
}

pub fn get_pi_prompt_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(PI_PROMPT_FILE)
}

pub async fn get_pi_settings_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_pi_settings_path_from_root(
        &get_pi_root_dir_from_db_async(db).await?,
    ))
}

pub async fn get_pi_auth_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_pi_auth_path_from_root(
        &get_pi_root_dir_from_db_async(db).await?,
    ))
}

pub async fn get_pi_models_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_pi_models_path_from_root(
        &get_pi_root_dir_from_db_async(db).await?,
    ))
}

pub async fn get_pi_mcp_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_pi_mcp_path_from_root(
        &get_pi_root_dir_from_db_async(db).await?,
    ))
}

pub async fn get_pi_prompt_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_pi_prompt_path_from_root(
        &get_pi_root_dir_from_db_async(db).await?,
    ))
}

fn read_json_object_or_empty(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let parsed: Value = serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(format!("{} must contain a JSON object", path.display()))
    }
}

fn write_json_object(path: &Path, value: &Value) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "{} must be written as a JSON object",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(value)
        .map_err(|error| format!("Failed to serialize {}: {error}", path.display()))?;
    fs::write(path, format!("{content}\n"))
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn set_auth_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn set_auth_file_permissions(_path: &Path) {}

fn object_ref(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, String> {
    value
        .as_object_mut()
        .ok_or_else(|| "Expected JSON object".to_string())
}

fn get_auth_entries(auth: &Value) -> Vec<(String, Value)> {
    object_ref(auth)
        .map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn get_models_providers(models: &Value) -> Vec<(String, Value)> {
    models
        .get("providers")
        .and_then(Value::as_object)
        .map(|providers| {
            providers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn credential_kind(credential: Option<&Value>, is_builtin: bool) -> PiCredentialKind {
    match credential
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
    {
        Some("api_key") => PiCredentialKind::ApiKey,
        Some("oauth") => PiCredentialKind::Oauth,
        Some(_) => PiCredentialKind::Oauth,
        None if credential.is_some() => PiCredentialKind::Oauth,
        None if is_builtin => PiCredentialKind::EnvPossible,
        None => PiCredentialKind::None,
    }
}

fn model_ids_from_provider(provider: Option<&Value>) -> Vec<String> {
    provider
        .and_then(|value| value.get("models"))
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn build_other_settings(settings: &Value) -> Value {
    let mut other = settings.as_object().cloned().unwrap_or_default();
    for key in other.keys().cloned().collect::<Vec<_>>() {
        if !is_pi_other_settings_editable_key(&key) {
            other.remove(&key);
        }
    }
    Value::Object(other)
}

fn is_pi_default_model_settings_key(key: &str) -> bool {
    matches!(
        key,
        "defaultProvider" | "defaultModel" | "defaultThinkingLevel"
    )
}

fn is_pi_other_settings_protected_key(key: &str) -> bool {
    PI_OTHER_SETTINGS_PROTECTED_KEYS.contains(&key)
}

fn is_pi_other_settings_editable_key(key: &str) -> bool {
    !is_pi_default_model_settings_key(key) && !is_pi_other_settings_protected_key(key)
}

fn apply_pi_other_settings(
    settings_object: &mut Map<String, Value>,
    other_settings: &Map<String, Value>,
) {
    for key in settings_object.keys().cloned().collect::<Vec<_>>() {
        if is_pi_other_settings_editable_key(&key) {
            settings_object.remove(&key);
        }
    }
    for (key, value) in other_settings {
        if is_pi_other_settings_editable_key(key) {
            settings_object.insert(key.clone(), value.clone());
        }
    }
}

fn default_selection_from_settings(settings: &Value) -> PiDefaultSelection {
    PiDefaultSelection {
        provider_key: settings
            .get("defaultProvider")
            .and_then(Value::as_str)
            .map(str::to_string),
        model_id: settings
            .get("defaultModel")
            .and_then(Value::as_str)
            .map(str::to_string),
        thinking_level: settings
            .get("defaultThinkingLevel")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn find_model_config<'a>(
    models: &'a Value,
    provider_key: &str,
    model_id: &str,
) -> Option<&'a Value> {
    let provider = models
        .get("providers")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(provider_key))?;

    provider
        .get("models")
        .and_then(Value::as_array)
        .and_then(|model_list| {
            model_list.iter().find(|model| {
                model
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| id == model_id)
                    .unwrap_or(false)
            })
        })
        .or_else(|| {
            provider
                .get("modelOverrides")
                .and_then(Value::as_object)
                .and_then(|overrides| overrides.get(model_id))
        })
}

fn model_supports_thinking_level(model: &Value, thinking_level: &str) -> bool {
    if !PI_THINKING_LEVEL_KEYS.contains(&thinking_level) {
        return false;
    }
    if model.get("reasoning").and_then(Value::as_bool) != Some(true) {
        return false;
    }

    match model
        .get("thinkingLevelMap")
        .and_then(Value::as_object)
        .and_then(|map| map.get(thinking_level))
    {
        Some(Value::Null) => false,
        Some(_) => true,
        None => !PI_EXTENDED_THINKING_LEVEL_KEYS.contains(&thinking_level),
    }
}

fn build_provider_views(
    settings: &Value,
    auth: &Value,
    models: &Value,
) -> Vec<PiRuntimeProviderView> {
    let default_provider = settings
        .get("defaultProvider")
        .and_then(Value::as_str)
        .map(str::to_string);
    let default_model = settings
        .get("defaultModel")
        .and_then(Value::as_str)
        .map(str::to_string);

    let auth_entries = get_auth_entries(auth);
    let models_entries = get_models_providers(models);
    let auth_map: Map<String, Value> = auth_entries.iter().cloned().collect();
    let models_map: Map<String, Value> = models_entries.iter().cloned().collect();

    let mut keys = BTreeSet::new();
    for (key, _) in &auth_entries {
        keys.insert(key.clone());
    }
    for (key, _) in &models_entries {
        keys.insert(key.clone());
    }
    if let Some(default_provider) = &default_provider {
        if !default_provider.trim().is_empty() {
            keys.insert(default_provider.clone());
        }
    }

    let mut views = Vec::new();
    for provider_key in keys {
        let credential = auth_map.get(&provider_key);
        let models_provider = models_map.get(&provider_key);
        let is_builtin = is_builtin_provider(&provider_key);
        let is_default = default_provider.as_deref() == Some(provider_key.as_str());
        let is_override = is_builtin && models_provider.is_some();

        let mut sources = Vec::new();
        if is_builtin {
            sources.push(PiProviderSource::OfficialBuiltin);
        }
        if credential.is_some() {
            sources.push(PiProviderSource::AuthJson);
        }
        if models_provider.is_some() {
            sources.push(PiProviderSource::ModelsJson);
        }
        if is_default {
            sources.push(PiProviderSource::SettingsJson);
        }

        let mut categories = Vec::new();
        let kind = credential_kind(credential, is_builtin);
        match kind {
            PiCredentialKind::ApiKey => categories.push(PiProviderCategory::ApiKey),
            PiCredentialKind::Oauth => categories.push(PiProviderCategory::Subscription),
            PiCredentialKind::EnvPossible | PiCredentialKind::None => {}
        }
        if models_provider.is_some() {
            categories.push(PiProviderCategory::Custom);
        }
        if categories.is_empty() && is_builtin {
            categories.push(PiProviderCategory::ApiKey);
        }

        let model_ids = model_ids_from_provider(models_provider);
        let mut warnings = Vec::new();
        if !is_builtin && credential.is_none() && models_provider.is_none() {
            warnings.push(PiProviderWarning::MissingProvider);
        }
        if is_default {
            if let Some(default_model) = default_model.as_deref() {
                if !default_model.trim().is_empty()
                    && !model_ids.is_empty()
                    && !model_ids.iter().any(|id| id == default_model)
                {
                    warnings.push(PiProviderWarning::MissingModel);
                }
            }
        }

        let mut runtime_files = Vec::new();
        if credential.is_some() {
            runtime_files.push(PI_AUTH_FILE.to_string());
        }
        if models_provider.is_some() {
            runtime_files.push(PI_MODELS_FILE.to_string());
        }
        if is_default {
            runtime_files.push(PI_SETTINGS_FILE.to_string());
        }

        views.push(PiRuntimeProviderView {
            display_name: builtin_provider_name(&provider_key)
                .map(str::to_string)
                .or_else(|| {
                    models_provider
                        .and_then(|value| value.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| provider_key.clone()),
            provider_key,
            sources,
            categories,
            credential_kind: kind,
            credential: credential.cloned(),
            models_provider: models_provider.cloned(),
            runtime_files,
            is_builtin,
            is_override,
            is_default,
            model_ids,
            warnings,
        });
    }

    views
}

fn builtin_providers() -> Vec<PiBuiltinProvider> {
    PI_BUILTIN_PROVIDERS
        .iter()
        .map(|(key, name)| PiBuiltinProvider {
            key: (*key).to_string(),
            name: (*name).to_string(),
        })
        .collect()
}

fn emit_config_changed<R: Runtime>(app: &tauri::AppHandle<R>, payload: &str) {
    let _ = app.emit("config-changed", payload);
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-pi", ());
}

#[tauri::command]
pub async fn get_pi_root_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<PiPathInfo, String> {
    get_pi_root_path_info_from_db_async(state.db()).await
}

#[tauri::command]
pub async fn get_pi_settings_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<PiSettingsConfig>, String> {
    Ok(state
        .db()
        .with_conn(|conn| db_get(conn, DbTable::PiSettingsConfig, "common"))?
        .map(adapter::settings_from_db_value))
}

/// Normalize a Pi root directory path for WSL UNC scenarios.
///
/// On Windows, the native file dialog cannot navigate into hidden (dot-prefixed)
/// WSL directories like `.pi`, so users can only select the parent directory.
/// Pi's config files always live in the `agent` subdirectory. This function
/// appends `agent` when the WSL linux path ends with `/.pi`.
fn normalize_pi_root_dir(path: &str) -> String {
    if let Some(wsl_info) = runtime_location::parse_wsl_unc_path(path) {
        let linux_path = wsl_info.linux_path.trim_end_matches('/');
        if linux_path.ends_with("/.pi") {
            let new_linux_path = format!("{}/agent", linux_path);
            return runtime_location::build_windows_unc_path(&wsl_info.distro, &new_linux_path)
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string()
}

#[tauri::command]
pub async fn save_pi_settings_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiSettingsConfigInput,
) -> Result<(), String> {
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(&db, "pi").await;
    let existing = get_pi_settings_config(state.clone()).await?;
    let root_dir = if input.clear_root_dir {
        None
    } else {
        input
            .root_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(normalize_pi_root_dir)
            .or_else(|| existing.and_then(|value| value.root_dir))
    };
    let data = adapter::settings_to_db_value(root_dir.as_deref());
    db.with_conn(|conn| db_put(conn, DbTable::PiSettingsConfig, "common", &data))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "pi").await?;
    resync_all_skills_if_tool_path_changed(app.clone(), state.inner(), "pi", previous_skills_path)
        .await;
    emit_config_changed(&app, "window");
    Ok(())
}

#[tauri::command]
pub async fn read_pi_runtime_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<PiRuntimeConfig, String> {
    let db = state.db();
    let root_path_info = get_pi_root_path_info_from_db_async(&db).await?;
    let root_dir = PathBuf::from(&root_path_info.path);
    let settings_path = get_pi_settings_path_from_root(&root_dir);
    let auth_path = get_pi_auth_path_from_root(&root_dir);
    let models_path = get_pi_models_path_from_root(&root_dir);
    let prompt_path = get_pi_prompt_path_from_root(&root_dir);

    let settings = read_json_object_or_empty(&settings_path)?;
    let auth = read_json_object_or_empty(&auth_path)?;
    let models = read_json_object_or_empty(&models_path)?;

    Ok(PiRuntimeConfig {
        root_path_info,
        settings_path: settings_path.to_string_lossy().to_string(),
        auth_path: auth_path.to_string_lossy().to_string(),
        models_path: models_path.to_string_lossy().to_string(),
        prompt_path: prompt_path.to_string_lossy().to_string(),
        other_settings: build_other_settings(&settings),
        model_settings: default_selection_from_settings(&settings),
        providers: build_provider_views(&settings, &auth, &models),
        builtin_providers: builtin_providers(),
        settings,
        auth,
        models,
    })
}

#[tauri::command]
pub async fn save_pi_model_settings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiModelSettingsInput,
) -> Result<PiRuntimeConfig, String> {
    let db = state.db();
    let settings_path = get_pi_settings_path_async(&db).await?;
    let mut settings = read_json_object_or_empty(&settings_path)?;
    let settings_object = object_mut(&mut settings)?;

    match input.default_provider {
        Some(value) if !value.trim().is_empty() => {
            settings_object.insert("defaultProvider".to_string(), json!(value.trim()));
        }
        Some(_) => {
            settings_object.remove("defaultProvider");
        }
        None => {}
    }
    match input.default_model {
        Some(value) if !value.trim().is_empty() => {
            settings_object.insert("defaultModel".to_string(), json!(value.trim()));
        }
        Some(_) => {
            settings_object.remove("defaultModel");
        }
        None => {}
    }
    match input.default_thinking_level {
        Some(value) if !value.trim().is_empty() => {
            settings_object.insert("defaultThinkingLevel".to_string(), json!(value.trim()));
        }
        Some(_) => {
            settings_object.remove("defaultThinkingLevel");
        }
        None => {}
    }

    write_json_object(&settings_path, &settings)?;
    emit_config_changed(&app, "window");
    read_pi_runtime_config(state).await
}

pub async fn apply_pi_default_provider_internal<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_key: &str,
    from_tray: bool,
) -> Result<(), String> {
    let provider_key = provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider key is required".to_string());
    }

    let settings_path = get_pi_settings_path_async(db).await?;
    let mut settings = read_json_object_or_empty(&settings_path)?;
    object_mut(&mut settings)?.insert("defaultProvider".to_string(), json!(provider_key));
    write_json_object(&settings_path, &settings)?;
    emit_config_changed(app, if from_tray { "tray" } else { "window" });
    Ok(())
}

pub async fn apply_pi_default_model_internal<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_key: &str,
    model_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    let provider_key = provider_key.trim();
    let model_id = model_id.trim();
    if provider_key.is_empty() {
        return Err("Provider key is required".to_string());
    }
    if model_id.is_empty() {
        return Err("Model id is required".to_string());
    }

    let settings_path = get_pi_settings_path_async(db).await?;
    let mut settings = read_json_object_or_empty(&settings_path)?;
    let current_thinking_level = settings
        .get("defaultThinkingLevel")
        .and_then(Value::as_str)
        .map(str::to_string);
    let should_remove_thinking_level = if let Some(thinking_level) = current_thinking_level {
        let models_path = get_pi_models_path_async(db).await?;
        let models = read_json_object_or_empty(&models_path)?;
        find_model_config(&models, provider_key, model_id)
            .map(|model| !model_supports_thinking_level(model, &thinking_level))
            .unwrap_or(false)
    } else {
        false
    };
    let settings_object = object_mut(&mut settings)?;
    settings_object.insert("defaultProvider".to_string(), json!(provider_key));
    settings_object.insert("defaultModel".to_string(), json!(model_id));
    if should_remove_thinking_level {
        settings_object.remove("defaultThinkingLevel");
    }
    write_json_object(&settings_path, &settings)?;
    emit_config_changed(app, if from_tray { "tray" } else { "window" });
    Ok(())
}

#[tauri::command]
pub async fn save_pi_other_settings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    other_settings: Value,
) -> Result<PiRuntimeConfig, String> {
    if !other_settings.is_object() {
        return Err("Pi other settings must be a JSON object".to_string());
    }

    let db = state.db();
    let settings_path = get_pi_settings_path_async(&db).await?;
    let mut settings = read_json_object_or_empty(&settings_path)?;
    let settings_object = object_mut(&mut settings)?;
    apply_pi_other_settings(settings_object, other_settings.as_object().unwrap());

    write_json_object(&settings_path, &settings)?;
    emit_config_changed(&app, "window");
    read_pi_runtime_config(state).await
}

#[tauri::command]
pub async fn save_pi_auth_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiAuthProviderInput,
) -> Result<PiRuntimeConfig, String> {
    let provider_key = input.provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider key is required".to_string());
    }
    if !input.credential.is_object() {
        return Err("Pi auth credential must be a JSON object".to_string());
    }

    let db = state.db();
    let auth_path = get_pi_auth_path_async(&db).await?;
    let mut auth = read_json_object_or_empty(&auth_path)?;
    object_mut(&mut auth)?.insert(provider_key.to_string(), input.credential);
    write_json_object(&auth_path, &auth)?;
    set_auth_file_permissions(&auth_path);
    emit_config_changed(&app, "window");
    read_pi_runtime_config(state).await
}

#[tauri::command]
pub async fn save_pi_models_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiModelsProviderInput,
) -> Result<PiRuntimeConfig, String> {
    let provider_key = input.provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider key is required".to_string());
    }
    if !input.provider.is_object() {
        return Err("Pi models provider config must be a JSON object".to_string());
    }

    let db = state.db();
    let models_path = get_pi_models_path_async(&db).await?;
    let mut models = read_json_object_or_empty(&models_path)?;
    let models_object = object_mut(&mut models)?;
    if !models_object
        .get("providers")
        .map(Value::is_object)
        .unwrap_or(false)
    {
        models_object.insert("providers".to_string(), Value::Object(Map::new()));
    }
    models_object
        .get_mut("providers")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "models.providers must be a JSON object".to_string())?
        .insert(provider_key.to_string(), input.provider);

    write_json_object(&models_path, &models)?;
    emit_config_changed(&app, "window");
    read_pi_runtime_config(state).await
}

#[tauri::command]
pub async fn delete_pi_runtime_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_key: String,
    scope: PiDeleteScope,
) -> Result<PiRuntimeConfig, String> {
    let provider_key = provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider key is required".to_string());
    }
    let db = state.db();

    if matches!(scope, PiDeleteScope::Credential | PiDeleteScope::Both) {
        let auth_path = get_pi_auth_path_async(&db).await?;
        let mut auth = read_json_object_or_empty(&auth_path)?;
        object_mut(&mut auth)?.remove(provider_key);
        write_json_object(&auth_path, &auth)?;
        set_auth_file_permissions(&auth_path);
    }

    if matches!(scope, PiDeleteScope::ProviderConfig | PiDeleteScope::Both) {
        let models_path = get_pi_models_path_async(&db).await?;
        let mut models = read_json_object_or_empty(&models_path)?;
        if let Some(providers) = models.get_mut("providers").and_then(Value::as_object_mut) {
            providers.remove(provider_key);
        }
        write_json_object(&models_path, &models)?;
    }

    emit_config_changed(&app, "window");
    read_pi_runtime_config(state).await
}

fn prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?]))
}

fn put_pi_prompt_to_sqlite(
    db: &SqliteDbState,
    id: &str,
    content: &PiPromptConfigContent,
) -> Result<(), String> {
    let value = adapter::prompt_to_db_value(content);
    db.with_conn(|conn| db_put(conn, DbTable::PiPromptConfig, id, &value))
}

fn get_pi_prompt_from_sqlite(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<PiPromptConfig>, String> {
    Ok(db
        .with_conn(|conn| db_get(conn, DbTable::PiPromptConfig, id))?
        .map(adapter::prompt_from_db_value))
}

async fn get_local_prompt_config(db: &SqliteDbState) -> Result<Option<PiPromptConfig>, String> {
    let prompt_path = get_pi_prompt_path_async(db).await?;
    if !prompt_path.exists() {
        return Ok(None);
    }
    let Some(content) = read_prompt_content_file(&prompt_path, "Pi")? else {
        return Ok(None);
    };
    Ok(Some(PiPromptConfig {
        id: "__local__".to_string(),
        name: "Local AGENTS.md".to_string(),
        content,
        is_applied: false,
        sort_index: Some(-1),
        created_at: None,
        updated_at: None,
    }))
}

async fn write_prompt_content_to_file(
    db: &SqliteDbState,
    content: Option<&str>,
) -> Result<(), String> {
    let path = get_pi_prompt_path_async(db).await?;
    write_prompt_content_file(&path, content, "Pi")
}

#[tauri::command]
pub async fn list_pi_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<PiPromptConfig>, String> {
    let db = state.db();
    let mut prompts = db.with_conn(|conn| {
        Ok(
            db_list(conn, DbTable::PiPromptConfig, Some(&prompt_order()?))?
                .into_iter()
                .map(adapter::prompt_from_db_value)
                .collect::<Vec<_>>(),
        )
    })?;
    if !prompts.iter().any(|prompt| prompt.is_applied) {
        if let Some(local_prompt) = get_local_prompt_config(&db).await? {
            prompts.insert(0, local_prompt);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_pi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiPromptConfigInput,
) -> Result<PiPromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let next_sort_index = db.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::PiPromptConfig,
            &JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;
    let content = PiPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };
    let prompt_id = db_new_id();
    put_pi_prompt_to_sqlite(&db, &prompt_id, &content)?;
    let _ = app.emit("config-changed", "window");
    Ok(adapter::prompt_from_db_value(json!({
        "id": prompt_id,
        "name": content.name,
        "content": content.content,
        "is_applied": content.is_applied,
        "sort_index": content.sort_index,
        "created_at": content.created_at,
        "updated_at": content.updated_at
    })))
}

#[tauri::command]
pub async fn update_pi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiPromptConfigInput,
) -> Result<PiPromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let existing = get_pi_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;
    let content = PiPromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied: existing.is_applied,
        sort_index: existing.sort_index,
        created_at: existing.created_at.unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    put_pi_prompt_to_sqlite(&db, &config_id, &content)?;
    if existing.is_applied {
        write_prompt_content_to_file(&db, Some(input.content.as_str())).await?;
        emit_config_changed(&app, "window");
    } else {
        let _ = app.emit("config-changed", "window");
    }
    get_pi_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found after update", config_id))
}

#[tauri::command]
pub async fn delete_pi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.db();
    let was_applied = get_pi_prompt_from_sqlite(&db, &id)?
        .map(|prompt| prompt.is_applied)
        .unwrap_or(false);
    db.with_conn(|conn| db_delete(conn, DbTable::PiPromptConfig, &id).map(|_| ()))?;
    if was_applied {
        write_prompt_content_to_file(&db, None).await?;
        emit_config_changed(&app, "window");
    } else {
        let _ = app.emit("config-changed", "window");
    }
    Ok(())
}

pub async fn apply_pi_prompt_config_internal<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    let db = state.db();
    if config_id == "__local__" {
        let local_prompt = get_local_prompt_config(&db)
            .await?
            .ok_or_else(|| "Local Pi prompt not found".to_string())?;
        write_prompt_content_to_file(&db, Some(local_prompt.content.as_str())).await?;
        emit_config_changed(app, if from_tray { "tray" } else { "window" });
        return Ok(());
    }

    let prompt = get_pi_prompt_from_sqlite(&db, config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::PiPromptConfig, Some(config_id), &now)
    })?;
    write_prompt_content_to_file(&db, Some(prompt.content.as_str())).await?;
    emit_config_changed(app, if from_tray { "tray" } else { "window" });
    Ok(())
}

#[tauri::command]
pub async fn apply_pi_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_pi_prompt_config_internal(state, &app, &config_id, false).await
}

#[tauri::command]
pub async fn reorder_pi_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::PiPromptConfig,
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn save_pi_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiPromptConfigInput,
) -> Result<PiPromptConfig, String> {
    let db = state.db();
    let content = if input.content.trim().is_empty() {
        get_local_prompt_config(&db)
            .await?
            .map(|prompt| prompt.content)
            .unwrap_or_default()
    } else {
        input.content
    };
    let created = create_pi_prompt_config(
        state.clone(),
        app.clone(),
        PiPromptConfigInput {
            id: None,
            name: input.name,
            content,
        },
    )
    .await?;
    apply_pi_prompt_config_internal(state.clone(), &app, &created.id, false).await?;
    Ok(get_pi_prompt_from_sqlite(state.db(), &created.id)?.unwrap_or(created))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_level_map_only_defaults_standard_omitted_levels_to_supported() {
        let model = json!({
            "id": "deepseek-v4-pro",
            "reasoning": true,
            "thinkingLevelMap": {
                "minimal": null,
                "low": null,
                "medium": null,
                "high": "high",
                "xhigh": "max"
            }
        });

        assert!(model_supports_thinking_level(&model, "off"));
        assert!(model_supports_thinking_level(&model, "high"));
        assert!(model_supports_thinking_level(&model, "xhigh"));
        assert!(!model_supports_thinking_level(&model, "max"));
        assert!(!model_supports_thinking_level(&model, "minimal"));
        assert!(!model_supports_thinking_level(&model, "ultra"));
        assert!(!model_supports_thinking_level(&model, "unknown"));
    }

    #[test]
    fn thinking_level_map_accepts_explicit_max_mapping() {
        let model = json!({
            "reasoning": true,
            "thinkingLevelMap": {
                "max": "max"
            }
        });

        assert!(model_supports_thinking_level(&model, "max"));
    }

    #[test]
    fn find_model_config_reads_models_before_model_overrides() {
        let models = json!({
            "providers": {
                "openrouter": {
                    "models": [
                        { "id": "anthropic/claude-sonnet-4", "reasoning": false }
                    ],
                    "modelOverrides": {
                        "anthropic/claude-sonnet-4": { "reasoning": true },
                        "openai/gpt-5": { "reasoning": true }
                    }
                }
            }
        });

        let custom_model = find_model_config(&models, "openrouter", "anthropic/claude-sonnet-4")
            .expect("custom model should be found first");
        assert_eq!(
            custom_model.get("reasoning").and_then(Value::as_bool),
            Some(false)
        );

        let override_model = find_model_config(&models, "openrouter", "openai/gpt-5")
            .expect("model override should be found");
        assert_eq!(
            override_model.get("reasoning").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn build_other_settings_excludes_model_defaults_and_packages() {
        let settings = json!({
            "defaultProvider": "anthropic",
            "defaultModel": "claude-sonnet-4",
            "defaultThinkingLevel": "high",
            "theme": "dark",
            "packages": ["npm:context-mode"],
            "extensions": ["./extensions"]
        });

        assert_eq!(
            build_other_settings(&settings),
            json!({
                "theme": "dark",
                "extensions": ["./extensions"]
            })
        );
    }

    #[test]
    fn apply_pi_other_settings_preserves_packages_and_defaults() {
        let mut settings = json!({
            "defaultProvider": "anthropic",
            "defaultModel": "claude-sonnet-4",
            "defaultThinkingLevel": "high",
            "theme": "dark",
            "packages": ["npm:context-mode"],
            "extensions": ["./extensions"]
        });
        let other_settings = json!({
            "theme": "light",
            "packages": ["npm:should-not-overwrite"],
            "enabledModels": ["anthropic/*"]
        });

        apply_pi_other_settings(
            settings.as_object_mut().expect("settings object"),
            other_settings.as_object().expect("other settings object"),
        );

        assert_eq!(
            settings,
            json!({
                "defaultProvider": "anthropic",
                "defaultModel": "claude-sonnet-4",
                "defaultThinkingLevel": "high",
                "theme": "light",
                "packages": ["npm:context-mode"],
                "enabledModels": ["anthropic/*"]
            })
        );
    }

    #[test]
    fn normalize_pi_root_dir_appends_agent_for_wsl_unc_path_ending_with_dot_pi() {
        let path = r"\\wsl.localhost\Ubuntu\home\tester\.pi";
        let normalized = normalize_pi_root_dir(path);
        assert_eq!(normalized, r"\\wsl.localhost\Ubuntu\home\tester\.pi\agent");
    }

    #[test]
    fn normalize_pi_root_dir_preserves_wsl_path_already_containing_agent() {
        let path = r"\\wsl.localhost\Ubuntu\home\tester\.pi\agent";
        let normalized = normalize_pi_root_dir(path);
        assert_eq!(normalized, r"\\wsl.localhost\Ubuntu\home\tester\.pi\agent");
    }

    #[test]
    fn normalize_pi_root_dir_preserves_non_wsl_path() {
        let path = r"C:\Users\tester\.pi";
        let normalized = normalize_pi_root_dir(path);
        assert_eq!(normalized, r"C:\Users\tester\.pi");
    }

    #[test]
    fn normalize_pi_root_dir_preserves_wsl_path_not_ending_with_dot_pi() {
        let path = r"\\wsl.localhost\Ubuntu\home\tester\custom-agent";
        let normalized = normalize_pi_root_dir(path);
        assert_eq!(
            normalized,
            r"\\wsl.localhost\Ubuntu\home\tester\custom-agent"
        );
    }

    #[test]
    fn normalize_pi_root_dir_preserves_wsl_path_ending_with_dot_pi_agent() {
        // Path already ends with agent, should not double-append
        let path = r"\\wsl.localhost\Ubuntu\home\tester\.pi\agent";
        let normalized = normalize_pi_root_dir(path);
        assert_eq!(normalized, r"\\wsl.localhost\Ubuntu\home\tester\.pi\agent");
    }
}
