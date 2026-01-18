use crate::db::DbState;
use crate::http_client;
use super::types::{FreeModel, ProviderModelsData, UnifiedModelOption, OpenCodeProvider, OfficialModel, OfficialProvider, GetAuthProvidersResponse};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use indexmap::IndexMap;
use std::fs;
use std::path::PathBuf;

// Load default models data from resources/models.json at compile time
const DEFAULT_MODELS_JSON: &str = include_str!("../../../resources/models.json");

const MODELS_API_URL: &str = "https://models.dev/api.json";
const DB_TABLE: &str = "provider_models";
const OPENCODE_PROVIDER_ID: &str = "opencode"; // Default provider for free models
const CACHE_DURATION_HOURS: u64 = 6; // 6 hours cache duration

/// Get all providers data from resources/models.json
/// Returns the complete JSON object containing all providers
fn get_all_default_providers_data() -> serde_json::Value {
    serde_json::from_str(DEFAULT_MODELS_JSON).unwrap_or_else(|e| {
        eprintln!("Failed to parse default models.json: {}", e);
        serde_json::json!({})
    })
}

/// Get default provider data (opencode channel) from resources/models.json
/// Returns the complete JSON object for the opencode provider
pub fn get_default_provider_data() -> serde_json::Value {
    let api_response = get_all_default_providers_data();

    // Extract the opencode provider object
    if let Some(opencode) = api_response.get(OPENCODE_PROVIDER_ID) {
        opencode.clone()
    } else {
        serde_json::json!({
            "name": "OpenCode Zen",
            "models": {}
        })
    }
}

/// Get default free models from resources/models.json
/// Returns filtered free models from the opencode channel
pub fn get_default_free_models() -> Vec<FreeModel> {
    let provider_data = get_default_provider_data();
    filter_free_models(OPENCODE_PROVIDER_ID, &provider_data)
}

/// Fetch all providers data from API
/// Returns the complete JSON object containing all providers
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

/// Fetch provider data (opencode channel) from API
pub async fn fetch_provider_data_from_api(state: &DbState) -> Result<serde_json::Value, String> {
    let api_response = fetch_all_providers_from_api(state).await?;

    // Extract the opencode provider object
    let opencode_data = api_response
        .get(OPENCODE_PROVIDER_ID)
        .cloned()
        .ok_or_else(|| "opencode channel not found in API response".to_string())?;

    Ok(opencode_data)
}

/// Filter free models from provider data (where cost.input and cost.output are both 0)
fn filter_free_models(provider_id: &str, provider_data: &serde_json::Value) -> Vec<FreeModel> {
    let mut free_models = Vec::new();

    // Get provider name (e.g., "OpenCode Zen")
    let provider_name = provider_data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    // Get models object
    let models_obj = match provider_data.get("models").and_then(|v| v.as_object()) {
        Some(obj) => obj,
        None => {
            return free_models;
        }
    };

    // Iterate through models
    for (model_id, model_obj) in models_obj {
        if let Some(model) = model_obj.as_object() {
            // Check if cost.input and cost.output are both 0
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
                let model_name = model
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(model_id)
                    .to_string();

                let free_model = FreeModel {
                    id: model_id.clone(),
                    name: model_name,
                    provider_id: provider_id.to_string(),
                    provider_name: provider_name.clone(),
                    context: model
                        .get("limit")
                        .and_then(|limit| limit.as_object())
                        .and_then(|limit| limit.get("context"))
                        .and_then(|v| v.as_i64()),
                };
                free_models.push(free_model);
            }
        }
    }

    free_models
}

/// Read provider models data from database by provider_id
pub async fn read_provider_models_from_db(state: &DbState, provider_id: &str) -> Result<Option<ProviderModelsData>, String> {
    let db = state.0.lock().await;

    // Query using type::string(id) to convert Thing to string
    let records_result: Result<Vec<serde_json::Value>, _> = db
        .query(&format!("SELECT *, type::string(id) as id FROM {}:`{}` LIMIT 1", DB_TABLE, provider_id))
        .await
        .map_err(|e| format!("Failed to query provider models: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                // Parse using the flat structure
                let data = ProviderModelsData {
                    provider_id: record
                        .get("provider_id")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .unwrap_or_default(),
                    value: record
                        .get("value")
                        .cloned()
                        .unwrap_or(serde_json::json!({})),
                    updated_at: record
                        .get("updated_at")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .unwrap_or_default(),
                };

                Ok(Some(data))
            } else {
                Ok(None)
            }
        }
        Err(e) => {
            Err(e.to_string())
        }
    }
}

/// Save provider models data to database
pub async fn save_provider_models_to_db(state: &DbState, data: &ProviderModelsData) -> Result<(), String> {
    let db = state.0.lock().await;

    // Use json! macro to create a flat structure (same pattern as existing code)
    let json_data = serde_json::json!({
        "provider_id": data.provider_id,
        "value": data.value,
        "updated_at": data.updated_at
    });

    // Use UPSERT to create or update record
    db.query(format!("UPSERT {}:`{}` CONTENT $data", DB_TABLE, data.provider_id))
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save provider models: {}", e))?;

    Ok(())
}

/// Save all provider models data to database (batch insert)
async fn save_all_provider_models_to_db(state: &DbState, all_providers: &serde_json::Value, updated_at: &str) -> Result<usize, String> {
    let providers_obj = match all_providers.as_object() {
        Some(obj) => obj,
        None => return Err("Invalid providers data: not an object".to_string()),
    };

    // Acquire lock once for all operations
    let db = state.0.lock().await;
    let mut saved_count = 0;

    for (provider_id, provider_data) in providers_obj {
        let json_data = serde_json::json!({
            "provider_id": provider_id,
            "value": provider_data,
            "updated_at": updated_at
        });

        // Use UPSERT to create or update record
        match db.query(format!("UPSERT {}:`{}` CONTENT $data", DB_TABLE, provider_id))
            .bind(("data", json_data))
            .await
        {
            Ok(_) => saved_count += 1,
            Err(e) => eprintln!("Failed to save record for {}: {}", provider_id, e),
        }
    }

    Ok(saved_count)
}

/// Check if cache is expired (6 hours)
fn is_cache_expired(updated_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(updated_at) {
        Ok(datetime) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(datetime);
            duration.num_hours() >= CACHE_DURATION_HOURS as i64
        }
        Err(_) => true, // Parse failed, consider as expired
    }
}

/// Get free models with cache logic
/// Returns (free_models, from_cache, updated_at)
///
/// Cache strategy:
/// - If cache is fresh (< 6 hours): return cached data immediately
/// - If cache is expired (>= 6 hours): return cached data immediately, then refresh in background
/// - If no cache exists: fetch from API (synchronous)
/// - If force_refresh: fetch from API (synchronous)
pub async fn get_free_models(state: &DbState, force_refresh: bool) -> Result<(Vec<FreeModel>, bool, Option<String>), String> {
    // 1. Try to read opencode provider from database (unless force_refresh)
    if !force_refresh {
        match read_provider_models_from_db(state, OPENCODE_PROVIDER_ID).await {
            Ok(Some(cached_data)) => {
                if !is_cache_expired(&cached_data.updated_at) {
                    // Cache is fresh: filter free models from cached provider data
                    let free_models = filter_free_models(OPENCODE_PROVIDER_ID, &cached_data.value);
                    return Ok((free_models, true, Some(cached_data.updated_at)));
                }

                // Cache expired: return filtered free models from cached data, then refresh in background
                let cached_models = filter_free_models(OPENCODE_PROVIDER_ID, &cached_data.value);
                let updated_at = cached_data.updated_at.clone();
                eprintln!("[CACHE EXPIRED] (updated_at: {}), returning {} stale models and refreshing in background...", updated_at, cached_models.len());

                // Spawn background task to refresh cache
                let db_arc = state.0.clone();
                let db_state = DbState(db_arc);
                tauri::async_runtime::spawn(async move {
                    eprintln!("[Background] Starting all providers data refresh...");
                    match fetch_and_update_all_providers(&db_state).await {
                        Ok(count) => {
                            eprintln!("[Background] Successfully refreshed {} providers", count);
                        }
                        Err(e) => {
                            eprintln!("[Background] Failed to refresh providers: {}", e);
                        }
                    }
                });

                return Ok((cached_models, true, Some(updated_at)));
            }
            Ok(None) => {
                eprintln!("[CACHE MISS] No cached data found, will fetch from API");
            }
            Err(e) => {
                eprintln!("[CACHE ERROR] Failed to read cache: {}, will fetch from API", e);
            }
        }
    }

    // 2. No cache or force_refresh: fetch all providers from API (synchronous)
    eprintln!("[FETCH] No cache or force_refresh, fetching all providers from API...");
    fetch_and_update_all_providers(state).await?;

    // 3. Read opencode provider from database and filter free models
    match read_provider_models_from_db(state, OPENCODE_PROVIDER_ID).await {
        Ok(Some(data)) => {
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

/// Fetch all providers from API and save to database
async fn fetch_and_update_all_providers(state: &DbState) -> Result<usize, String> {
    let all_providers = fetch_all_providers_from_api(state).await?;

    // If API returned empty, use default providers data
    let final_providers = if all_providers.as_object().map(|m| m.is_empty()).unwrap_or(true) {
        eprintln!("API returned empty providers, using default data");
        get_all_default_providers_data()
    } else {
        all_providers
    };

    // Log provider IDs being saved
    if let Some(providers_obj) = final_providers.as_object() {
        eprintln!("Saving {} providers to database", providers_obj.len());
    }

    // Save all providers to database
    let updated_at = chrono::Utc::now().to_rfc3339();
    save_all_provider_models_to_db(state, &final_providers, &updated_at).await
}

/// Initialize default provider models in database (called on app startup)
/// Only writes if no cached data exists (checks opencode as indicator)
pub async fn init_default_provider_models(state: &DbState) -> Result<(), String> {
    // Check if opencode provider exists as indicator for all providers
    match read_provider_models_from_db(state, OPENCODE_PROVIDER_ID).await {
        Ok(Some(data)) => {
            eprintln!("Provider models cache already exists (updated_at: {}), skipping initialization", data.updated_at);
            Ok(())
        }
        Ok(None) => {
            eprintln!("No provider models cache found, initializing with default data for all providers");
            let all_providers = get_all_default_providers_data();
            let updated_at = chrono::Utc::now().to_rfc3339();

            match save_all_provider_models_to_db(state, &all_providers, &updated_at).await {
                Ok(count) => {
                    eprintln!("Successfully initialized {} providers with default data", count);
                    Ok(())
                }
                Err(e) => {
                    eprintln!("Failed to initialize providers: {}", e);
                    Err(e)
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to check provider models cache: {}, skipping initialization", e);
            Ok(())
        }
    }
}

/// Get provider models data by provider_id (internal function)
/// This is the internal API to get specific provider's model information
pub async fn get_provider_models_internal(state: &DbState, provider_id: &str) -> Result<Option<ProviderModelsData>, String> {
    read_provider_models_from_db(state, provider_id).await
}

// ============================================================================
// Auth.json Reading
// ============================================================================

/// Auth entry in auth.json
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthEntry {
    #[serde(rename = "type")]
    auth_type: String,
    key: String,
}

/// Get auth.json file path: ~/.local/share/opencode/auth.json
fn get_auth_json_path() -> Result<PathBuf, String> {
    let home_dir = dirs::home_dir().ok_or_else(|| "Failed to get home directory".to_string())?;
    Ok(home_dir.join(".local/share/opencode/auth.json"))
}

/// Get auth.json file path for UI display
#[tauri::command]
pub fn get_opencode_auth_config_path() -> Result<String, String> {
    let path = get_auth_json_path()?;
    Ok(path.to_string_lossy().to_string())
}

/// Read auth.json and return the list of logged-in provider ids
/// Returns empty vector if file doesn't exist or fails to parse
pub fn read_auth_channels() -> Vec<String> {
    let auth_path = match get_auth_json_path() {
        Ok(path) => path,
        Err(_) => return vec![],
    };

    // Return empty list if file doesn't exist
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

    // Return all provider ids (keys)
    auth_map.keys().cloned().collect()
}

// ============================================================================
// Unified Models API
// ============================================================================

/// Check if a model from models.dev is free (cost.input and cost.output are both 0)
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

/// Get unified model list combining custom providers and official providers
///
/// # Arguments
/// * `state` - Database state for reading cached provider models
/// * `custom_providers` - Optional custom providers from user config (IndexMap preserves order)
/// * `auth_channels` - List of provider ids from auth.json
///
/// # Returns
/// Vector of unified model options in order: custom providers (config order) → auth providers → free models
/// If custom provider id matches auth provider id, they are merged with custom name
pub async fn get_unified_models(
    state: &DbState,
    custom_providers: Option<&IndexMap<String, OpenCodeProvider>>,
    auth_channels: &[String],
) -> Vec<UnifiedModelOption> {
    let mut models: Vec<UnifiedModelOption> = Vec::new();

    // Check if opencode is in auth
    let has_opencode_auth = auth_channels.contains(&"opencode".to_string());
    let mut official_provider_ids = auth_channels.to_vec();

    // If opencode is not in auth, we'll add free models separately later
    if !has_opencode_auth {
        official_provider_ids.retain(|id| id != "opencode");
    }

    // Get official provider models from database
    let mut official_models: HashMap<String, ProviderModelsData> = HashMap::new();

    // Check if we have any official models cached
    let mut any_missing = false;
    for provider_id in &official_provider_ids {
        match read_provider_models_from_db(state, provider_id).await {
            Ok(Some(data)) => {
                official_models.insert(provider_id.clone(), data);
            }
            Ok(None) => {
                any_missing = true;
            }
            Err(_) => {
                any_missing = true;
            }
        }
    }

    // If any official provider data is missing, try to fetch all providers from API
    if any_missing && !official_provider_ids.is_empty() {
        if fetch_and_update_all_providers(state).await.is_ok() {
            // Reload all official providers
            official_models.clear();
            for provider_id in &official_provider_ids {
                if let Ok(Some(data)) = read_provider_models_from_db(state, provider_id).await {
                    official_models.insert(provider_id.clone(), data);
                }
            }
        }
    }

    // Track which auth providers have been merged with custom providers
    let mut merged_auth_providers: HashSet<String> = HashSet::new();

    // 1. Process custom providers (merge with auth if id matches)
    if let Some(providers) = custom_providers {
        for (provider_id, provider) in providers {
            let provider_name = provider.name.as_deref().unwrap_or(provider_id);
            let mut provider_models: Vec<UnifiedModelOption> = Vec::new();

            // Collect custom model ids for deduplication
            let mut custom_model_ids: HashSet<String> = HashSet::new();

            // Add custom models first
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

            // Check if this provider has matching auth provider
            if let Some(official_data) = official_models.get(provider_id) {
                merged_auth_providers.insert(provider_id.clone());

                if let Some(models_obj) = official_data.value.get("models").and_then(|m| m.as_object()) {
                    for (model_id, model_obj) in models_obj {
                        let full_id = format!("{}/{}", provider_id, model_id);

                        // Skip if already in custom models
                        if custom_model_ids.contains(&full_id) {
                            continue;
                        }

                        let model_name = model_obj.get("name").and_then(|n| n.as_str()).unwrap_or(model_id);
                        let is_free = is_model_free_from_value(model_obj);

                        // Use custom provider name, but add (Free) for opencode free models
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

            // Sort this provider's models by display name and add to main list
            provider_models.sort_by(|a, b| a.display_name.cmp(&b.display_name));
            models.extend(provider_models);
        }
    }

    // 2. Add auth providers that don't have custom config
    for (provider_id, official_data) in &official_models {
        // Skip if already merged with custom provider
        if merged_auth_providers.contains(provider_id) {
            continue;
        }

        let provider_name = official_data
            .value
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(provider_id);

        let mut provider_models: Vec<UnifiedModelOption> = Vec::new();

        if let Some(models_obj) = official_data.value.get("models").and_then(|m| m.as_object()) {
            for (model_id, model_obj) in models_obj {
                let model_name = model_obj.get("name").and_then(|n| n.as_str()).unwrap_or(model_id);
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

        // Sort this provider's models by display name and add to main list
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
                        display_name: format!("{} / {} (Free)", free_model.provider_name, free_model.name),
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

    models
}

// ============================================================================
// Official Auth Providers API
// ============================================================================

/// Get official auth providers data for display in UI
///
/// # Arguments
/// * `state` - Database state for reading cached provider models
/// * `custom_providers` - Optional custom providers from user config
///
/// # Returns
/// GetAuthProvidersResponse containing:
/// - standalone_providers: Official providers not in custom config
/// - merged_models: Official models from providers that are in custom config (excluding duplicates)
/// - custom_provider_ids: All custom provider IDs for reference
pub async fn get_auth_providers_data(
    state: &DbState,
    custom_providers: Option<&IndexMap<String, OpenCodeProvider>>,
) -> GetAuthProvidersResponse {
    // Read auth.json to get official provider ids
    let auth_channels = read_auth_channels();

    // Get custom provider IDs
    let custom_provider_ids: Vec<String> = custom_providers
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();

    // Collect custom model IDs for deduplication (provider_id/model_id)
    let mut custom_model_ids: HashSet<String> = HashSet::new();
    if let Some(providers) = custom_providers {
        for (provider_id, provider) in providers {
            for model_id in provider.models.keys() {
                custom_model_ids.insert(format!("{}/{}", provider_id, model_id));
            }
        }
    }

    // Get official provider models from database
    let mut official_models: HashMap<String, ProviderModelsData> = HashMap::new();

    // Filter out opencode from auth channels (it's for free models, not auth)
    let official_provider_ids: Vec<String> = auth_channels
        .into_iter()
        .filter(|id| id != "opencode")
        .collect();

    // Check if we have any official models cached
    let mut any_missing = false;
    for provider_id in &official_provider_ids {
        match read_provider_models_from_db(state, provider_id).await {
            Ok(Some(data)) => {
                official_models.insert(provider_id.clone(), data);
            }
            Ok(None) => {
                any_missing = true;
            }
            Err(_) => {
                any_missing = true;
            }
        }
    }

    // If any official provider data is missing, try to fetch all providers from API
    if any_missing && !official_provider_ids.is_empty() {
        if fetch_and_update_all_providers(state).await.is_ok() {
            // Reload all official providers
            official_models.clear();
            for provider_id in &official_provider_ids {
                if let Ok(Some(data)) = read_provider_models_from_db(state, provider_id).await {
                    official_models.insert(provider_id.clone(), data);
                }
            }
        }
    }

    // Split into standalone providers and merged models
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

        if let Some(models_obj) = official_data.value.get("models").and_then(|m| m.as_object()) {
            for (model_id, model_obj) in models_obj {
                let full_id = format!("{}/{}", provider_id, model_id);

                // Skip if already in custom models
                if custom_model_ids.contains(&full_id) {
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

                let is_free = is_model_free_from_value(model_obj);

                official_models_list.push(OfficialModel {
                    id: model_id.to_string(),
                    name: model_name,
                    context,
                    output,
                    is_free,
                });
            }
        }

        // Sort models by name
        official_models_list.sort_by(|a, b| a.name.cmp(&b.name));

        // Check if this provider is in custom config
        if custom_provider_ids.contains(provider_id) {
            // Add to merged models
            if !official_models_list.is_empty() {
                merged_models.insert(provider_id.clone(), official_models_list);
            }
        } else {
            // Add as standalone provider
            standalone_providers.push(OfficialProvider {
                id: provider_id.clone(),
                name: provider_name,
                models: official_models_list,
            });
        }
    }

    // Sort standalone providers by name
    standalone_providers.sort_by(|a, b| a.name.cmp(&b.name));

    let response = GetAuthProvidersResponse {
        standalone_providers,
        merged_models,
        custom_provider_ids,
    };

    response
}
