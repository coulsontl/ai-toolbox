use super::types::{
    FreeModel, GetAuthProvidersResponse, OfficialModel, OfficialProvider, OpenCodeProvider,
    ProviderModelsData, UnifiedModelOption,
};
use crate::db::DbState;
use crate::http_client;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const DEFAULT_MODELS_JSON: &str = include_str!("../../../resources/models.dev.json");

const MODELS_API_URL: &str = "https://models.dev/api.json";
const CACHE_FILE_NAME: &str = "models.dev.json";
const OPENCODE_PROVIDER_ID: &str = "opencode";
const CACHE_DURATION_HOURS: u64 = 6;
const MIN_REFRESH_INTERVAL_SECS: u64 = 30;

/// Global flag to prevent concurrent background refresh
static IS_REFRESHING: AtomicBool = AtomicBool::new(false);

/// Last refresh timestamp for debounce
static LAST_REFRESH: Mutex<Option<Instant>> = Mutex::new(None);

/// App data directory path, set once at startup by lib.rs
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// On-disk cache structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelsCache {
    providers: serde_json::Value,
    updated_at: String,
}

// ============================================================================
// Cache directory management
// ============================================================================

/// Set the cache directory (called once from lib.rs at startup)
pub fn set_cache_dir(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

fn get_cache_file_path() -> Option<PathBuf> {
    CACHE_DIR.get().map(|dir| dir.join(CACHE_FILE_NAME))
}

/// Check if the cache file has been initialized (exists on disk)
fn is_cache_initialized() -> bool {
    get_cache_file_path()
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// Get the cache file path as a String (for backup utilities)
pub fn get_models_cache_path() -> Option<PathBuf> {
    get_cache_file_path()
}

// ============================================================================
// File-based cache read / write
// ============================================================================

fn read_cache_file() -> Option<ModelsCache> {
    let path = get_cache_file_path()?;
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Atomic write: write to .tmp then rename
fn write_cache_file(cache: &ModelsCache) -> Result<(), String> {
    let path =
        get_cache_file_path().ok_or_else(|| "Cache directory not initialized".to_string())?;

    let tmp_path = path.with_extension("json.tmp");

    let json =
        serde_json::to_string(cache).map_err(|e| format!("Failed to serialize cache: {}", e))?;

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
        }
    }

    fs::write(&tmp_path, json).map_err(|e| format!("Failed to write tmp cache file: {}", e))?;
    fs::rename(&tmp_path, &path).map_err(|e| format!("Failed to rename tmp cache file: {}", e))?;

    Ok(())
}

/// Read a specific provider's data from cache file
fn read_provider_from_cache(provider_id: &str) -> Option<ProviderModelsData> {
    let cache = read_cache_file()?;
    extract_provider_from_cache(&cache, provider_id)
}

/// Extract a provider from an already-loaded cache (no file IO)
fn extract_provider_from_cache(
    cache: &ModelsCache,
    provider_id: &str,
) -> Option<ProviderModelsData> {
    let value = cache.providers.get(provider_id)?.clone();
    Some(ProviderModelsData {
        provider_id: provider_id.to_string(),
        value,
        updated_at: cache.updated_at.clone(),
    })
}

/// Read multiple providers in one file read
fn read_providers_batch(provider_ids: &[String]) -> HashMap<String, ProviderModelsData> {
    let mut result = HashMap::new();
    if let Some(cache) = read_cache_file() {
        for id in provider_ids {
            if let Some(data) = extract_provider_from_cache(&cache, id) {
                result.insert(id.clone(), data);
            }
        }
    }
    result
}

/// Save all providers to cache file
fn save_all_providers_to_cache(
    all_providers: &serde_json::Value,
    updated_at: &str,
) -> Result<usize, String> {
    let count = all_providers.as_object().map(|m| m.len()).unwrap_or(0);
    let cache = ModelsCache {
        providers: all_providers.clone(),
        updated_at: updated_at.to_string(),
    };
    write_cache_file(&cache)?;
    Ok(count)
}

// ============================================================================
// Default data from embedded models.dev.json
// ============================================================================

fn get_all_default_providers_data() -> serde_json::Value {
    serde_json::from_str(DEFAULT_MODELS_JSON).unwrap_or_else(|e| {
        eprintln!("Failed to parse default models.dev.json: {}", e);
        serde_json::json!({})
    })
}

pub fn get_default_provider_data() -> serde_json::Value {
    let api_response = get_all_default_providers_data();
    if let Some(opencode) = api_response.get(OPENCODE_PROVIDER_ID) {
        opencode.clone()
    } else {
        serde_json::json!({ "name": "OpenCode Zen", "models": {} })
    }
}

pub fn get_default_free_models() -> Vec<FreeModel> {
    let provider_data = get_default_provider_data();
    filter_free_models(OPENCODE_PROVIDER_ID, &provider_data)
}

/// Read multiple providers from the compile-time embedded default data
fn read_providers_batch_from_defaults(
    provider_ids: &[String],
) -> HashMap<String, ProviderModelsData> {
    let all_defaults = get_all_default_providers_data();
    let mut result = HashMap::new();
    for id in provider_ids {
        if let Some(value) = all_defaults.get(id.as_str()).cloned() {
            result.insert(
                id.clone(),
                ProviderModelsData {
                    provider_id: id.clone(),
                    value,
                    updated_at: String::new(),
                },
            );
        }
    }
    result
}

/// Trigger a background refresh of all providers (non-blocking, debounced)
fn trigger_background_refresh(state: &DbState) {
    if should_skip_refresh() {
        return;
    }
    let db_state = DbState(state.0.clone());
    tauri::async_runtime::spawn(async move {
        if IS_REFRESHING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            log::info!("[Models Cache] Starting background refresh (cache miss)...");
            mark_refresh_time();
            let result = fetch_and_update_all_providers(&db_state).await;
            IS_REFRESHING.store(false, Ordering::SeqCst);
            match result {
                Ok(count) => log::info!(
                    "[Models Cache] Successfully refreshed {} providers",
                    count
                ),
                Err(e) => log::warn!("[Models Cache] Failed to refresh providers: {}", e),
            }
        }
    });
}

// ============================================================================
// API fetch
// ============================================================================

async fn fetch_all_providers_from_api(state: &DbState) -> Result<serde_json::Value, String> {
    let client = http_client::client_with_timeout(state, 30).await?;

    let response = client
        .get(MODELS_API_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models API: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()));
    }

    let api_response: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    Ok(api_response)
}

pub async fn fetch_provider_data_from_api(state: &DbState) -> Result<serde_json::Value, String> {
    let api_response = fetch_all_providers_from_api(state).await?;
    let opencode_data = api_response
        .get(OPENCODE_PROVIDER_ID)
        .cloned()
        .ok_or_else(|| "opencode channel not found in API response".to_string())?;
    Ok(opencode_data)
}

// ============================================================================
// Filter helpers
// ============================================================================

fn filter_free_models(provider_id: &str, provider_data: &serde_json::Value) -> Vec<FreeModel> {
    let mut free_models = Vec::new();

    let provider_name = provider_data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    let models_obj = match provider_data.get("models").and_then(|v| v.as_object()) {
        Some(obj) => obj,
        None => return free_models,
    };

    for (model_id, model_obj) in models_obj {
        if let Some(model) = model_obj.as_object() {
            let is_free = model
                .get("cost")
                .and_then(|cost| cost.as_object())
                .map(|cost| {
                    let input = cost.get("input").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                    let output = cost.get("output").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                    input == 0.0 && output == 0.0
                })
                .unwrap_or(false);

            if is_free {
                let status = model.get("status").and_then(|v| v.as_str());
                if status == Some("deprecated") {
                    continue;
                }

                let model_name = model
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(model_id)
                    .to_string();

                free_models.push(FreeModel {
                    id: model_id.clone(),
                    name: model_name,
                    provider_id: provider_id.to_string(),
                    provider_name: provider_name.clone(),
                    context: model
                        .get("limit")
                        .and_then(|limit| limit.as_object())
                        .and_then(|limit| limit.get("context"))
                        .and_then(|v| v.as_i64()),
                });
            }
        }
    }

    free_models
}

fn is_cache_expired(updated_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(updated_at) {
        Ok(datetime) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(datetime);
            duration.num_hours() >= CACHE_DURATION_HOURS as i64
        }
        Err(_) => true,
    }
}

/// Check 30-second debounce window
fn should_skip_refresh() -> bool {
    if let Ok(guard) = LAST_REFRESH.lock() {
        if let Some(last) = *guard {
            return last.elapsed().as_secs() < MIN_REFRESH_INTERVAL_SECS;
        }
    }
    false
}

fn mark_refresh_time() {
    if let Ok(mut guard) = LAST_REFRESH.lock() {
        *guard = Some(Instant::now());
    }
}

// ============================================================================
// Public read / write API  (signatures unchanged)
// ============================================================================

pub async fn read_provider_models_from_db(
    _state: &DbState,
    provider_id: &str,
) -> Result<Option<ProviderModelsData>, String> {
    Ok(read_provider_from_cache(provider_id))
}

pub async fn save_provider_models_to_db(
    _state: &DbState,
    data: &ProviderModelsData,
) -> Result<(), String> {
    let mut cache = read_cache_file().unwrap_or_else(|| ModelsCache {
        providers: serde_json::json!({}),
        updated_at: String::new(),
    });

    if let Some(obj) = cache.providers.as_object_mut() {
        obj.insert(data.provider_id.clone(), data.value.clone());
    }
    cache.updated_at = data.updated_at.clone();
    write_cache_file(&cache)
}

async fn save_all_provider_models(
    _all_providers: &serde_json::Value,
    updated_at: &str,
) -> Result<usize, String> {
    save_all_providers_to_cache(_all_providers, updated_at)
}

// ============================================================================
// Cache logic
// ============================================================================

pub async fn get_free_models(
    state: &DbState,
    force_refresh: bool,
) -> Result<(Vec<FreeModel>, bool, Option<String>), String> {
    if !force_refresh {
        if let Some(cached_data) = read_provider_from_cache(OPENCODE_PROVIDER_ID) {
            if !is_cache_expired(&cached_data.updated_at) {
                let free_models = filter_free_models(OPENCODE_PROVIDER_ID, &cached_data.value);
                return Ok((free_models, true, Some(cached_data.updated_at)));
            }

            let cached_models = filter_free_models(OPENCODE_PROVIDER_ID, &cached_data.value);
            let updated_at = cached_data.updated_at.clone();
            log::info!(
                "[Models Cache] Cache expired (updated_at: {}), returning {} stale models",
                updated_at,
                cached_models.len()
            );

            trigger_background_refresh(state);

            return Ok((cached_models, true, Some(updated_at)));
        }

        // Cache does not exist: return defaults immediately, refresh in background
        log::info!("[Models Cache] No cache found, returning default models and triggering background refresh");
        trigger_background_refresh(state);
        return Ok((get_default_free_models(), false, None));
    }

    // force_refresh=true: sync fetch and report errors
    log::info!("[Models Cache] Fetching all providers from API (force_refresh=true)");
    fetch_and_update_all_providers(state).await?;

    match read_provider_from_cache(OPENCODE_PROVIDER_ID) {
        Some(data) => {
            let free_models = filter_free_models(OPENCODE_PROVIDER_ID, &data.value);
            if free_models.is_empty() {
                Ok((get_default_free_models(), false, None))
            } else {
                Ok((free_models, false, None))
            }
        }
        _ => Ok((get_default_free_models(), false, None)),
    }
}

async fn fetch_and_update_all_providers(state: &DbState) -> Result<usize, String> {
    let all_providers = fetch_all_providers_from_api(state).await?;

    let final_providers = if all_providers
        .as_object()
        .map(|m| m.is_empty())
        .unwrap_or(true)
    {
        log::warn!("[Models Cache] API returned empty providers, using default data");
        get_all_default_providers_data()
    } else {
        all_providers
    };

    if let Some(providers_obj) = final_providers.as_object() {
        log::info!(
            "[Models Cache] Saving {} providers to cache file",
            providers_obj.len()
        );
    }

    let updated_at = chrono::Utc::now().to_rfc3339();
    save_all_provider_models(&final_providers, &updated_at).await
}

/// Initialize default provider models cache (called on app startup, synchronous)
pub fn init_default_provider_models() {
    if let Some(cached_data) = read_provider_from_cache(OPENCODE_PROVIDER_ID) {
        log::info!(
            "[Models Cache] Cache already exists (updated_at: {}), skipping initialization",
            cached_data.updated_at
        );
        return;
    }

    log::info!("[Models Cache] No cache found, initializing with default data");
    let all_providers = get_all_default_providers_data();
    let updated_at = chrono::Utc::now().to_rfc3339();

    match save_all_providers_to_cache(&all_providers, &updated_at) {
        Ok(count) => log::info!(
            "[Models Cache] Successfully initialized {} providers with default data",
            count
        ),
        Err(e) => log::warn!("[Models Cache] Failed to initialize providers: {}", e),
    }
}

pub async fn get_provider_models_internal(
    _state: &DbState,
    provider_id: &str,
) -> Result<Option<ProviderModelsData>, String> {
    Ok(read_provider_from_cache(provider_id))
}

// ============================================================================
// Auth.json Reading
// ============================================================================

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthEntry {
    #[serde(rename = "type")]
    auth_type: String,
    key: Option<String>,
    access: Option<String>,
    refresh: Option<String>,
}

fn get_auth_json_path() -> Result<PathBuf, String> {
    let home_dir = dirs::home_dir().ok_or_else(|| "Failed to get home directory".to_string())?;
    Ok(home_dir.join(".local/share/opencode/auth.json"))
}

#[tauri::command]
pub fn get_opencode_auth_config_path() -> Result<String, String> {
    let path = get_auth_json_path()?;
    Ok(path.to_string_lossy().to_string())
}

pub fn read_auth_channels() -> Vec<String> {
    let auth_path = match get_auth_json_path() {
        Ok(path) => path,
        Err(_) => return vec![],
    };

    if !auth_path.exists() {
        return vec![];
    }

    let content = match fs::read_to_string(&auth_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let auth_map: HashMap<String, AuthEntry> = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    auth_map.keys().cloned().collect()
}

// ============================================================================
// Unified Models API
// ============================================================================

fn is_model_free_from_value(model_obj: &serde_json::Value) -> bool {
    model_obj
        .get("cost")
        .and_then(|cost| cost.as_object())
        .map(|cost| {
            let input = cost.get("input").and_then(|v| v.as_f64()).unwrap_or(-1.0);
            let output = cost.get("output").and_then(|v| v.as_f64()).unwrap_or(-1.0);
            input == 0.0 && output == 0.0
        })
        .unwrap_or(false)
}

fn apply_model_filters(
    models: Vec<UnifiedModelOption>,
    custom_providers: Option<&IndexMap<String, OpenCodeProvider>>,
) -> Vec<UnifiedModelOption> {
    let providers = match custom_providers {
        Some(p) => p,
        None => return models,
    };

    models
        .into_iter()
        .filter(|model| {
            let provider = match providers.get(&model.provider_id) {
                Some(p) => p,
                None => return true,
            };

            if let Some(whitelist) = &provider.whitelist {
                return whitelist.iter().any(|id| id == &model.model_id);
            }

            if let Some(blacklist) = &provider.blacklist {
                return !blacklist.iter().any(|id| id == &model.model_id);
            }

            true
        })
        .collect()
}

pub async fn get_unified_models(
    state: &DbState,
    custom_providers: Option<&IndexMap<String, OpenCodeProvider>>,
    auth_channels: &[String],
) -> Vec<UnifiedModelOption> {
    let mut models: Vec<UnifiedModelOption> = Vec::new();

    let has_opencode_auth = auth_channels.contains(&"opencode".to_string());
    let mut official_provider_ids = auth_channels.to_vec();

    if !has_opencode_auth {
        official_provider_ids.retain(|id| id != "opencode");
    }

    let mut official_models = read_providers_batch(&official_provider_ids);

    if official_models.is_empty() && !official_provider_ids.is_empty() && !is_cache_initialized() {
        official_models = read_providers_batch_from_defaults(&official_provider_ids);
        trigger_background_refresh(state);
    }

    let mut merged_auth_providers: HashSet<String> = HashSet::new();

    // 1. Process custom providers (merge with auth if id matches)
    if let Some(providers) = custom_providers {
        for (provider_id, provider) in providers {
            let provider_name = provider.name.as_deref().unwrap_or(provider_id);
            let mut provider_models: Vec<UnifiedModelOption> = Vec::new();
            let mut custom_model_ids: HashSet<String> = HashSet::new();

            for (model_id, model) in &provider.models {
                let model_name = model.name.as_deref().unwrap_or(model_id);
                custom_model_ids.insert(format!("{}/{}", provider_id, model_id));

                provider_models.push(UnifiedModelOption {
                    id: format!("{}/{}", provider_id, model_id),
                    display_name: format!("{} / {}", provider_name, model_name),
                    provider_id: provider_id.clone(),
                    model_id: model_id.clone(),
                    is_free: false,
                });
            }

            if let Some(official_data) = official_models.get(provider_id) {
                merged_auth_providers.insert(provider_id.clone());

                if let Some(models_obj) = official_data
                    .value
                    .get("models")
                    .and_then(|m| m.as_object())
                {
                    for (model_id, model_obj) in models_obj {
                        let full_id = format!("{}/{}", provider_id, model_id);

                        if custom_model_ids.contains(&full_id) {
                            continue;
                        }

                        let status = model_obj.get("status").and_then(|v| v.as_str());
                        if status == Some("deprecated") {
                            continue;
                        }

                        let model_name = model_obj
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or(model_id);
                        let is_free = is_model_free_from_value(model_obj);

                        let display_name = if provider_id == "opencode" && is_free {
                            format!("{} / {} (Free)", provider_name, model_name)
                        } else {
                            format!("{} / {}", provider_name, model_name)
                        };

                        provider_models.push(UnifiedModelOption {
                            id: full_id,
                            display_name,
                            provider_id: provider_id.clone(),
                            model_id: model_id.clone(),
                            is_free,
                        });
                    }
                }
            }

            provider_models.sort_by(|a, b| a.display_name.cmp(&b.display_name));
            models.extend(provider_models);
        }
    }

    // 2. Add auth providers that don't have custom config
    for (provider_id, official_data) in &official_models {
        if merged_auth_providers.contains(provider_id) {
            continue;
        }

        let provider_name = official_data
            .value
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(provider_id);

        let mut provider_models: Vec<UnifiedModelOption> = Vec::new();

        if let Some(models_obj) = official_data
            .value
            .get("models")
            .and_then(|m| m.as_object())
        {
            for (model_id, model_obj) in models_obj {
                let status = model_obj.get("status").and_then(|v| v.as_str());
                if status == Some("deprecated") {
                    continue;
                }

                let model_name = model_obj
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(model_id);
                let is_free = is_model_free_from_value(model_obj);

                let display_name = if provider_id == "opencode" && is_free {
                    format!("{} / {} (Free)", provider_name, model_name)
                } else {
                    format!("{} / {}", provider_name, model_name)
                };

                provider_models.push(UnifiedModelOption {
                    id: format!("{}/{}", provider_id, model_id),
                    display_name,
                    provider_id: provider_id.clone(),
                    model_id: model_id.clone(),
                    is_free,
                });
            }
        }

        provider_models.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        models.extend(provider_models);
    }

    // 3. Add free models if opencode is not in auth
    if !has_opencode_auth {
        match get_free_models(state, false).await {
            Ok((free_models, _, _)) => {
                let mut free_vec: Vec<UnifiedModelOption> = Vec::new();
                for free_model in free_models {
                    free_vec.push(UnifiedModelOption {
                        id: format!("{}/{}", free_model.provider_id, free_model.id),
                        display_name: format!(
                            "{} / {} (Free)",
                            free_model.provider_name, free_model.name
                        ),
                        provider_id: free_model.provider_id,
                        model_id: free_model.id,
                        is_free: true,
                    });
                }
                free_vec.sort_by(|a, b| a.display_name.cmp(&b.display_name));
                models.extend(free_vec);
            }
            Err(e) => {
                eprintln!("Failed to load free models: {}", e);
            }
        }
    }

    apply_model_filters(models, custom_providers)
}

// ============================================================================
// Official Auth Providers API
// ============================================================================

pub async fn get_auth_providers_data(
    state: &DbState,
    custom_providers: Option<&IndexMap<String, OpenCodeProvider>>,
) -> GetAuthProvidersResponse {
    let auth_channels = read_auth_channels();

    let custom_provider_ids: Vec<String> = custom_providers
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();

    let mut custom_model_ids: HashSet<String> = HashSet::new();
    if let Some(providers) = custom_providers {
        for (provider_id, provider) in providers {
            for model_id in provider.models.keys() {
                custom_model_ids.insert(format!("{}/{}", provider_id, model_id));
            }
        }
    }

    let official_provider_ids: Vec<String> = auth_channels
        .into_iter()
        .filter(|id| id != "opencode")
        .collect();

    let mut official_models = read_providers_batch(&official_provider_ids);

    if official_models.is_empty() && !official_provider_ids.is_empty() && !is_cache_initialized() {
        official_models = read_providers_batch_from_defaults(&official_provider_ids);
        trigger_background_refresh(state);
    }

    let mut standalone_providers: Vec<OfficialProvider> = Vec::new();
    let mut merged_models: HashMap<String, Vec<OfficialModel>> = HashMap::new();

    for (provider_id, official_data) in &official_models {
        let provider_name = official_data
            .value
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(provider_id)
            .to_string();

        let mut official_models_list: Vec<OfficialModel> = Vec::new();

        if let Some(models_obj) = official_data
            .value
            .get("models")
            .and_then(|m| m.as_object())
        {
            for (model_id, model_obj) in models_obj {
                let full_id = format!("{}/{}", provider_id, model_id);

                if custom_model_ids.contains(&full_id) {
                    continue;
                }

                let status = model_obj.get("status").and_then(|v| v.as_str());
                if status == Some("deprecated") {
                    continue;
                }

                let model_name = model_obj
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(model_id)
                    .to_string();

                let context = model_obj
                    .get("limit")
                    .and_then(|limit| limit.as_object())
                    .and_then(|limit| limit.get("context"))
                    .and_then(|v| v.as_i64());

                let output = model_obj
                    .get("limit")
                    .and_then(|limit| limit.as_object())
                    .and_then(|limit| limit.get("output"))
                    .and_then(|v| v.as_i64());

                let is_free = if provider_id == "opencode" {
                    is_model_free_from_value(model_obj)
                } else {
                    false
                };

                let status = model_obj
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                official_models_list.push(OfficialModel {
                    id: model_id.to_string(),
                    name: model_name,
                    context,
                    output,
                    is_free,
                    status,
                });
            }
        }

        official_models_list.sort_by(|a, b| a.name.cmp(&b.name));

        if custom_provider_ids.contains(provider_id) {
            if !official_models_list.is_empty() {
                merged_models.insert(provider_id.clone(), official_models_list);
            }
        } else {
            standalone_providers.push(OfficialProvider {
                id: provider_id.clone(),
                name: provider_name,
                models: official_models_list,
            });
        }
    }

    standalone_providers.sort_by(|a, b| a.name.cmp(&b.name));

    GetAuthProvidersResponse {
        standalone_providers,
        merged_models,
        custom_provider_ids,
    }
}
