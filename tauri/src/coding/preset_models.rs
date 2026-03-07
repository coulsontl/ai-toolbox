use crate::db::DbState;
use crate::http_client;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

const CACHE_FILE_NAME: &str = "preset_models.json";

/// Bundled preset models JSON (compile-time embedded from resources/)
const DEFAULT_PRESET_MODELS_JSON: &str = include_str!("../../resources/preset_models.json");

/// App data directory path, set once at startup by lib.rs
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

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

/// Public getter for the cache file path (used by backup/restore)
pub fn get_preset_models_cache_path() -> Option<PathBuf> {
    get_cache_file_path()
}

// ============================================================================
// Bundled defaults
// ============================================================================

fn get_bundled_preset_models() -> Option<Value> {
    let data: Value = serde_json::from_str(DEFAULT_PRESET_MODELS_JSON).ok()?;
    if is_valid_preset_models(&data) {
        Some(data)
    } else {
        None
    }
}

// ============================================================================
// File-based cache read / write
// ============================================================================

fn read_cache_file() -> Option<Value> {
    let path = get_cache_file_path()?;
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Atomic write: write to .tmp then rename
fn write_cache_file(data: &Value) -> Result<(), String> {
    let path = get_cache_file_path()
        .ok_or_else(|| "Cache directory not initialized".to_string())?;

    let tmp_path = path.with_extension("json.tmp");

    let json = serde_json::to_string(data)
        .map_err(|e| format!("Failed to serialize preset models cache: {}", e))?;

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
        }
    }

    fs::write(&tmp_path, json)
        .map_err(|e| format!("Failed to write tmp cache file: {}", e))?;
    fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to rename tmp cache file: {}", e))?;

    Ok(())
}

/// Validate that the JSON looks like a preset models map
/// (non-empty object with at least one key).
fn is_valid_preset_models(data: &Value) -> bool {
    data.as_object().map(|m| !m.is_empty()).unwrap_or(false)
}

// ============================================================================
// Tauri commands
// ============================================================================

/// Load preset models: local cache first, then bundled defaults as fallback.
#[tauri::command]
pub fn load_cached_preset_models() -> Result<Option<Value>, String> {
    // Try local cache first
    if let Some(data) = read_cache_file() {
        if is_valid_preset_models(&data) {
            return Ok(Some(data));
        }
    }
    // Fallback to bundled defaults
    Ok(get_bundled_preset_models())
}

/// Fetch preset models JSON from a remote URL, save to local cache,
/// and return the data to the frontend.
#[tauri::command]
pub async fn fetch_remote_preset_models(
    state: tauri::State<'_, DbState>,
    url: String,
) -> Result<Value, String> {
    let client = http_client::client_with_timeout(&state, 30).await?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch remote preset models: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Remote preset models request failed: {}",
            response.status()
        ));
    }

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse remote preset models JSON: {}", e))?;

    // Only cache valid data
    if !is_valid_preset_models(&json) {
        return Err("Remote preset models JSON is empty or invalid".to_string());
    }

    // Save to local cache file
    if let Err(e) = write_cache_file(&json) {
        log::warn!("[PresetModels] Failed to write cache: {}", e);
    } else {
        log::info!("[PresetModels] Cache updated from remote");
    }

    Ok(json)
}
