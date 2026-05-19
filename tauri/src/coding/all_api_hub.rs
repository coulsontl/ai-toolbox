use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use log::{info, warn};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rusty_leveldb::{Options, DB};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::open_claw::types::{OpenClawModel, OpenClawProviderConfig};
use super::open_code::types::{OpenCodeModel, OpenCodeProvider, OpenCodeProviderOptions};
use crate::db::SqliteDbState;
use crate::http_client;

const KNOWN_EXTENSION_IDS: &[&str] = &[
    "hnmbbaagobbadojmjkeilcgbnpdfifmk",
    "lapnciffpekdengooeolaienkeoilfeo",
];

const STORAGE_KEY: &str = "site_accounts";
const QUOTA_TO_USD_CONVERSION_FACTOR: f64 = 500_000.0;
const ALL_API_HUB_EXTENSION_NOT_FOUND: &str = "all_api_hub_extension_not_found";
const ALL_API_HUB_NO_ENABLED_PROVIDER_DATA: &str = "all_api_hub_no_enabled_provider_data";
const ALL_API_HUB_COOKIE_MODELS_UNSUPPORTED: &str = "all_api_hub_cookie_models_unsupported";

/// Cache for has_all_api_hub_extension result (30-second TTL)
static EXTENSION_CACHE: Mutex<Option<(bool, Instant)>> = Mutex::new(None);
const EXTENSION_CACHE_TTL_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct ExtensionInfo {
    pub profile_name: String,
    pub extension_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllApiHubProfileInfo {
    pub profile_name: String,
    pub extension_id: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct AllApiHubProviderCandidate {
    pub provider_id: String,
    pub name: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub is_disabled: bool,
    pub balance_usd: Option<f64>,
    pub balance_cny: Option<f64>,
    pub access_token: Option<String>,
    pub user_id: Option<i64>,
    pub auth_type: Option<String>,
    pub cookie_auth_session_cookie: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub npm: String,
    pub api_protocol: String,
    pub site_name: Option<String>,
    pub site_type: Option<String>,
    pub account_label: String,
    pub source_profile_name: String,
    pub source_extension_id: String,
}

#[derive(Debug, Clone)]
pub struct AllApiHubDiscovery {
    pub found: bool,
    pub profiles: Vec<AllApiHubProfileInfo>,
    pub providers: Vec<AllApiHubProviderCandidate>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllApiHubProviderModelsRequest {
    pub provider_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllApiHubProviderModelsResult {
    pub provider_id: String,
    pub models: Vec<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct TempDirCleanup {
    path: PathBuf,
}

impl TempDirCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tauri::command]
pub fn has_all_api_hub_extension() -> bool {
    // Check cache first
    if let Ok(guard) = EXTENSION_CACHE.lock() {
        if let Some((cached_result, cached_at)) = *guard {
            if cached_at.elapsed().as_secs() < EXTENSION_CACHE_TTL_SECS {
                return cached_result;
            }
        }
    }

    let has_extension = !discover_extension_dirs().is_empty();
    if !has_extension {
        info!("All API Hub extension check: no discoverable extension storage found");
    }

    // Update cache
    if let Ok(mut guard) = EXTENSION_CACHE.lock() {
        *guard = Some((has_extension, Instant::now()));
    }

    has_extension
}

pub fn list_provider_candidates() -> Result<AllApiHubDiscovery, String> {
    let dirs = discover_extension_dirs();
    let profiles = dirs
        .iter()
        .map(|info| AllApiHubProfileInfo {
            profile_name: info.profile_name.clone(),
            extension_id: info.extension_id.clone(),
            path: info.path.display().to_string(),
        })
        .collect::<Vec<_>>();

    if dirs.is_empty() {
        return Ok(AllApiHubDiscovery {
            found: false,
            profiles,
            providers: vec![],
            message: Some(ALL_API_HUB_EXTENSION_NOT_FOUND.to_string()),
        });
    }

    let mut providers = Vec::new();
    let mut seen_signatures = HashSet::new();
    let mut last_error: Option<String> = None;
    let mut provider_id_counts: HashMap<String, usize> = HashMap::new();

    for info in &dirs {
        match read_extension_storage(&info.path).and_then(|raw| {
            parse_providers_from_storage(&raw, info, &mut seen_signatures, &mut provider_id_counts)
        }) {
            Ok(mut items) => {
                providers.append(&mut items);
            }
            Err(err) => {
                warn!(
                    "Failed to parse All API Hub storage from profile {}: {}",
                    info.profile_name, err
                );
                last_error = Some(err);
            }
        }
    }

    let message = if providers.is_empty() {
        Some(last_error.unwrap_or_else(|| ALL_API_HUB_NO_ENABLED_PROVIDER_DATA.to_string()))
    } else {
        None
    };

    Ok(AllApiHubDiscovery {
        found: true,
        profiles,
        providers,
        message,
    })
}

pub async fn list_provider_candidates_with_keys(
    db_state: &SqliteDbState,
) -> Result<AllApiHubDiscovery, String> {
    let mut discovery = list_provider_candidates()?;
    hydrate_missing_api_keys(db_state, &mut discovery.providers).await?;
    Ok(discovery)
}

pub async fn resolve_provider_candidates_with_keys(
    db_state: &SqliteDbState,
    provider_ids: &[String],
) -> Result<Vec<AllApiHubProviderCandidate>, String> {
    let provider_id_set: HashSet<&str> = provider_ids.iter().map(|id| id.as_str()).collect();
    let mut discovery = list_provider_candidates()?;
    discovery.providers.retain(|provider| {
        !provider.is_disabled && provider_id_set.contains(provider.provider_id.as_str())
    });
    hydrate_missing_api_keys(db_state, &mut discovery.providers).await?;
    Ok(discovery.providers)
}

pub async fn resolve_provider_candidates_models(
    db_state: &SqliteDbState,
    provider_ids: &[String],
) -> Result<Vec<AllApiHubProviderModelsResult>, String> {
    let order_map: HashMap<&str, usize> = provider_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    let provider_id_set: HashSet<&str> = provider_ids.iter().map(|id| id.as_str()).collect();
    let discovery = list_provider_candidates()?;
    let selected_candidates = discovery
        .providers
        .into_iter()
        .filter(|provider| {
            !provider.is_disabled && provider_id_set.contains(provider.provider_id.as_str())
        })
        .collect::<Vec<_>>();

    let client = http_client::client_with_timeout(db_state, 20).await?;
    let mut results = selected_candidates
        .iter()
        .map(|candidate| async { resolve_candidate_models_with_client(&client, candidate).await })
        .collect::<Vec<_>>();

    let mut resolved = Vec::with_capacity(results.len());
    for future in results.drain(..) {
        resolved.push(future.await);
    }

    resolved.sort_by_key(|item| {
        order_map
            .get(item.provider_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    Ok(resolved)
}

#[tauri::command]
pub async fn get_all_api_hub_provider_models(
    state: tauri::State<'_, SqliteDbState>,
    request: AllApiHubProviderModelsRequest,
) -> Result<Vec<AllApiHubProviderModelsResult>, String> {
    resolve_provider_candidates_models(&state, &request.provider_ids).await
}

fn imported_provider_name(candidate: &AllApiHubProviderCandidate) -> String {
    candidate
        .site_name
        .clone()
        .or_else(|| candidate.base_url.as_ref().map(|url| extract_host(url)))
        .unwrap_or_else(|| "All API Hub".to_string())
}

pub fn candidate_to_opencode_provider(candidate: &AllApiHubProviderCandidate) -> OpenCodeProvider {
    let normalized_base_url = candidate
        .base_url
        .as_deref()
        .map(|url| normalize_provider_base_url(url, &candidate.npm));
    let options = if normalized_base_url.is_some() || candidate.api_key.is_some() {
        Some(OpenCodeProviderOptions {
            base_url: normalized_base_url,
            api_key: candidate.api_key.clone(),
            headers: None,
            timeout: None,
            set_cache_key: None,
            extra: serde_json::Map::new(),
        })
    } else {
        None
    };

    OpenCodeProvider {
        api: None,
        env: None,
        id: None,
        npm: Some(candidate.npm.clone()),
        name: Some(imported_provider_name(candidate)),
        options,
        models: indexmap::IndexMap::<String, OpenCodeModel>::new(),
        whitelist: None,
        blacklist: None,
        extra: serde_json::Map::new(),
    }
}

pub fn candidate_to_openclaw_provider(
    candidate: &AllApiHubProviderCandidate,
) -> OpenClawProviderConfig {
    OpenClawProviderConfig {
        base_url: candidate
            .base_url
            .as_deref()
            .map(|url| normalize_provider_base_url(url, &candidate.npm)),
        api_key: candidate.api_key.clone(),
        api: Some(candidate.api_protocol.clone()),
        models: Vec::<OpenClawModel>::new(),
        extra: HashMap::new(),
    }
}

pub fn mask_api_key_preview(api_key: &str) -> String {
    let trimmed = api_key.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    let char_count = chars.len();

    if char_count <= 4 {
        return "****".to_string();
    }

    if char_count <= 6 {
        let prefix: String = chars.iter().take(2).collect();
        return format!("{}***", prefix);
    }

    if char_count <= 12 {
        let prefix: String = chars.iter().take(3).collect();
        let suffix: String = chars
            .iter()
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return format!("{}...{}", prefix, suffix);
    }

    let prefix: String = chars.iter().take(6).collect();
    let suffix: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}...{}", prefix, suffix)
}

fn get_chrome_base_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("Google")
                .join("Chrome")
        })
    }

    #[cfg(target_os = "linux")]
    {
        dirs::home_dir().map(|h| h.join(".config").join("google-chrome"))
    }

    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir().map(|d| d.join("Google").join("Chrome").join("User Data"))
    }
}

fn list_chrome_profiles(base: &Path) -> Vec<(String, PathBuf)> {
    let mut profiles = Vec::new();

    let Ok(entries) = std::fs::read_dir(base) else {
        return profiles;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "Default" || name.starts_with("Profile ") {
            let ext_settings = entry.path().join("Local Extension Settings");
            if ext_settings.is_dir() {
                profiles.push((name, ext_settings));
            }
        }
    }

    profiles
}

pub fn discover_extension_dirs() -> Vec<ExtensionInfo> {
    let base = match get_chrome_base_dir() {
        Some(path) if path.is_dir() => path,
        _ => {
            info!("Chrome base directory not found");
            return Vec::new();
        }
    };

    let profiles = list_chrome_profiles(&base);
    if profiles.is_empty() {
        info!("No Chrome profiles found in {:?}", base);
        return Vec::new();
    }

    let mut results = Vec::new();

    for (profile_name, ext_settings_dir) in &profiles {
        for &ext_id in KNOWN_EXTENSION_IDS {
            let candidate = ext_settings_dir.join(ext_id);
            if candidate.is_dir() {
                results.push(ExtensionInfo {
                    profile_name: profile_name.clone(),
                    extension_id: ext_id.to_string(),
                    path: candidate,
                });
            }
        }
    }

    if !results.is_empty() {
        return results;
    }

    results
}

fn read_extension_storage(ext_dir: &Path) -> Result<String, String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "ai-toolbox-all-api-hub-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let _cleanup = TempDirCleanup::new(temp_dir.clone());

    copy_dir_all(ext_dir, &temp_dir)
        .map_err(|e| format!("Failed to copy LevelDB dir to temp: {}", e))?;

    let lock_file = temp_dir.join("LOCK");
    if lock_file.exists() {
        let _ = std::fs::remove_file(&lock_file);
    }

    let opts = Options::default();
    let mut db =
        DB::open(&temp_dir, opts).map_err(|e| format!("Failed to open LevelDB: {:?}", e))?;

    let raw_value = db
        .get(STORAGE_KEY.as_bytes())
        .ok_or_else(|| "Key 'site_accounts' not found in LevelDB".to_string())?;

    drop(db);

    let value_str = String::from_utf8(raw_value.to_vec())
        .map_err(|e| format!("Invalid UTF-8 in LevelDB value: {}", e))?;

    let cleaned = unwrap_json_string(&value_str);
    let trimmed = cleaned.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return Err("site_accounts value is not valid JSON".to_string());
    }

    Ok(cleaned)
}

fn parse_providers_from_storage(
    raw_json: &str,
    info: &ExtensionInfo,
    seen_signatures: &mut HashSet<String>,
    provider_id_counts: &mut HashMap<String, usize>,
) -> Result<Vec<AllApiHubProviderCandidate>, String> {
    let storage_config: Value = serde_json::from_str(raw_json)
        .map_err(|e| format!("Failed to parse extension storage JSON: {}", e))?;

    let raw_accounts = storage_config
        .get("accounts")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Extension storage does not contain an 'accounts' array".to_string())?;

    let mut providers = Vec::new();

    for account in raw_accounts {
        let site_name = get_string(account, &["site_name", "siteName"]);
        let site_url = get_string(account, &["site_url", "siteUrl"]);
        let site_type = get_string(account, &["site_type", "siteType"]);
        let account_info = account
            .get("account_info")
            .or_else(|| account.get("accountInfo"));
        let is_disabled = account
            .get("disabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if is_disabled {
            continue;
        }

        let username = account_info.and_then(|v| get_string(v, &["username", "user_name"]));
        let account_id = account_info
            .and_then(|v| get_i64(v, &["id"]))
            .map(|id| id.to_string())
            .or_else(|| account_info.and_then(|v| get_string(v, &["id"])));
        let api_key = account_info.and_then(|v| get_string(v, &["api_key", "apiKey"]));
        let quota = account_info.and_then(|v| get_f64(v, &["quota"]));
        let exchange_rate = get_f64(account, &["exchange_rate", "exchangeRate"]);
        let balance_usd = quota.map(|value| value / QUOTA_TO_USD_CONVERSION_FACTOR);
        let balance_cny = match (quota, exchange_rate) {
            (Some(value), Some(rate)) => Some((value / QUOTA_TO_USD_CONVERSION_FACTOR) * rate),
            _ => None,
        };

        let account_label = username
            .clone()
            .or(account_id.clone())
            .unwrap_or_else(|| "account".to_string());

        let signature = format!(
            "{}|{}|{}|{}",
            site_url.clone().unwrap_or_default(),
            site_name.clone().unwrap_or_default(),
            account_label,
            api_key.clone().unwrap_or_default()
        );
        if seen_signatures.contains(&signature) {
            continue;
        }
        seen_signatures.insert(signature);

        let base_name = site_name
            .clone()
            .or_else(|| site_url.as_ref().map(|url| extract_host(url)))
            .unwrap_or_else(|| "All API Hub".to_string());
        let provider_name = format!("{} ({})", base_name, account_label);

        let raw_provider_id = format!("{}-{}", slugify(&base_name), slugify(&account_label));
        let provider_id = uniquify_provider_id(&raw_provider_id, provider_id_counts);
        let npm = infer_npm(site_type.as_deref(), site_url.as_deref());
        let api_protocol = infer_openclaw_api(&npm);

        providers.push(AllApiHubProviderCandidate {
            provider_id,
            name: provider_name,
            base_url: site_url,
            api_key,
            is_disabled,
            balance_usd,
            balance_cny,
            access_token: account_info
                .and_then(|v| get_string(v, &["access_token", "accessToken"])),
            user_id: account_info.and_then(|v| get_i64(v, &["id"])).or_else(|| {
                account_info
                    .and_then(|v| get_string(v, &["id"]))
                    .and_then(|id| id.parse::<i64>().ok())
            }),
            auth_type: get_string(account, &["authType", "auth_type"]),
            cookie_auth_session_cookie: account
                .get("cookieAuth")
                .or_else(|| account.get("cookie_auth"))
                .and_then(|v| get_string(v, &["sessionCookie", "session_cookie"])),
            refresh_token: account
                .get("sub2apiAuth")
                .or_else(|| account.get("sub2api_auth"))
                .and_then(|v| get_string(v, &["refreshToken", "refresh_token"])),
            token_expires_at: account
                .get("sub2apiAuth")
                .or_else(|| account.get("sub2api_auth"))
                .and_then(|v| get_i64(v, &["tokenExpiresAt", "token_expires_at"])),
            npm,
            api_protocol,
            site_name,
            site_type,
            account_label,
            source_profile_name: info.profile_name.clone(),
            source_extension_id: info.extension_id.clone(),
        });
    }

    Ok(providers)
}

fn infer_npm(site_type: Option<&str>, site_url: Option<&str>) -> String {
    let site_type = site_type.unwrap_or_default().to_lowercase();
    let site_url = site_url.unwrap_or_default().to_lowercase();

    if site_type.contains("anthropic")
        || site_url.contains("anthropic")
        || site_url.contains("claude")
    {
        "@ai-sdk/anthropic".to_string()
    } else if site_type.contains("google")
        || site_type.contains("gemini")
        || site_url.contains("googleapis.com")
        || site_url.contains("gemini")
    {
        "@ai-sdk/google".to_string()
    } else if site_type.contains("openai") || site_url.contains("openai.com") {
        "@ai-sdk/openai".to_string()
    } else {
        "@ai-sdk/openai-compatible".to_string()
    }
}

async fn resolve_candidate_models_with_client(
    client: &reqwest::Client,
    candidate: &AllApiHubProviderCandidate,
) -> AllApiHubProviderModelsResult {
    match fetch_candidate_available_models(client, candidate).await {
        Ok(models) => AllApiHubProviderModelsResult {
            provider_id: candidate.provider_id.clone(),
            models,
            status: "loaded".to_string(),
            error: None,
        },
        Err(ModelsFetchError::Unsupported(message)) => AllApiHubProviderModelsResult {
            provider_id: candidate.provider_id.clone(),
            models: vec![],
            status: "unsupported".to_string(),
            error: Some(message),
        },
        Err(ModelsFetchError::Request(message)) => AllApiHubProviderModelsResult {
            provider_id: candidate.provider_id.clone(),
            models: vec![],
            status: "error".to_string(),
            error: Some(message),
        },
    }
}

#[derive(Debug, Clone)]
enum ModelsFetchError {
    Unsupported(String),
    Request(String),
}

async fn fetch_candidate_available_models(
    client: &reqwest::Client,
    candidate: &AllApiHubProviderCandidate,
) -> Result<Vec<String>, ModelsFetchError> {
    if candidate
        .auth_type
        .as_deref()
        .map(|value| value.trim().eq_ignore_ascii_case("cookie"))
        .unwrap_or(false)
    {
        return Err(ModelsFetchError::Unsupported(
            ALL_API_HUB_COOKIE_MODELS_UNSUPPORTED.to_string(),
        ));
    }

    let site_url = candidate
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ModelsFetchError::Unsupported("provider base URL is missing".to_string()))?;

    let normalized_site_type = candidate
        .site_type
        .as_deref()
        .map(normalize_site_type)
        .unwrap_or_default();

    if normalized_site_type == "sub2api" {
        return Err(ModelsFetchError::Unsupported(
            "Sub2API does not expose account model lists".to_string(),
        ));
    }

    if normalized_site_type == "octopus" {
        return Err(ModelsFetchError::Unsupported(
            "Octopus model discovery depends on global site credentials".to_string(),
        ));
    }

    let payload = if normalized_site_type == "one-hub" || normalized_site_type == "done-hub" {
        fetch_api_payload(client, site_url, "/api/available_model", candidate).await?
    } else {
        fetch_api_payload(client, site_url, "/api/user/models", candidate).await?
    };

    let models = if payload.is_object()
        && (normalized_site_type == "one-hub" || normalized_site_type == "done-hub")
    {
        extract_model_keys(&payload)
    } else {
        extract_model_values(&payload)
    };

    Ok(normalize_model_list(models))
}

async fn fetch_api_payload(
    client: &reqwest::Client,
    site_url: &str,
    endpoint: &str,
    candidate: &AllApiHubProviderCandidate,
) -> Result<Value, ModelsFetchError> {
    let mut headers = build_model_request_headers(candidate)?;
    headers.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/json"),
    );

    let url = format!("{}{}", site_url.trim_end_matches('/'), endpoint);
    let response = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| ModelsFetchError::Request(format!("Failed to fetch {}: {}", endpoint, e)))?;

    let status = response.status();
    let body = response.text().await.map_err(|e| {
        ModelsFetchError::Request(format!("Failed to read {} response: {}", endpoint, e))
    })?;

    if !status.is_success() {
        return Err(ModelsFetchError::Request(format!(
            "{} returned HTTP {}: {}",
            endpoint, status, body
        )));
    }

    let parsed: Value = serde_json::from_str(&body)
        .map_err(|e| ModelsFetchError::Request(format!("Invalid JSON from {}: {}", endpoint, e)))?;

    if parsed.get("success").and_then(|value| value.as_bool()) == Some(false) {
        let message = parsed
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown error");
        return Err(ModelsFetchError::Request(format!(
            "{} responded with failure: {}",
            endpoint, message
        )));
    }

    Ok(parsed.get("data").cloned().unwrap_or(parsed))
}

fn build_model_request_headers(
    candidate: &AllApiHubProviderCandidate,
) -> Result<HeaderMap, ModelsFetchError> {
    let mut headers = HeaderMap::new();
    if let Some(user_id) = candidate.user_id {
        headers.extend(build_user_id_headers(user_id));
    }

    let access_token = candidate.access_token.as_deref().unwrap_or_default();
    let auth_type = candidate.auth_type.as_deref();
    let cookie = candidate.cookie_auth_session_cookie.as_deref();

    apply_auth_headers(&mut headers, auth_type, access_token, cookie)
        .map_err(ModelsFetchError::Request)?;

    Ok(headers)
}

fn normalize_site_type(site_type: &str) -> String {
    site_type.trim().to_ascii_lowercase()
}

fn extract_model_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
}

fn extract_model_values(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(model) => Some(model.trim().to_string()),
                Value::Object(map) => map
                    .get("id")
                    .and_then(|value| value.as_str())
                    .or_else(|| map.get("name").and_then(|value| value.as_str()))
                    .map(|value| value.trim().to_string()),
                _ => None,
            })
            .collect(),
        Value::Object(_) => extract_model_keys(value),
        _ => vec![],
    }
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();

    for model in models {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            continue;
        }

        if seen.insert(trimmed.to_string()) {
            normalized.push(trimmed.to_string());
        }
    }

    normalized
}

async fn hydrate_missing_api_keys(
    db_state: &SqliteDbState,
    providers: &mut [AllApiHubProviderCandidate],
) -> Result<(), String> {
    let client = http_client::client_with_timeout(db_state, 15).await?;

    for provider in providers.iter_mut() {
        if provider
            .api_key
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            continue;
        }

        let site_url = match provider.base_url.as_deref() {
            Some(value) if !value.is_empty() => value,
            _ => continue,
        };
        let site_type = provider.site_type.as_deref().unwrap_or_default();
        let user_id = match provider.user_id {
            Some(value) => value,
            None => continue,
        };
        let auth_type = provider.auth_type.as_deref();
        let cookie_auth_session_cookie = provider.cookie_auth_session_cookie.as_deref();
        let refresh_token = provider.refresh_token.as_deref();
        let token_expires_at = provider.token_expires_at;
        let access_token = provider.access_token.as_deref().unwrap_or_default();

        let normalized_auth_type = auth_type.unwrap_or("access_token").trim().to_lowercase();
        let has_cookie_auth = normalized_auth_type == "cookie"
            && cookie_auth_session_cookie
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
        let has_access_token = !access_token.trim().is_empty();

        if !has_cookie_auth && !has_access_token {
            continue;
        }

        match fetch_api_key_with_client(
            &client,
            site_url,
            site_type,
            access_token,
            user_id,
            auth_type,
            cookie_auth_session_cookie,
            refresh_token,
            token_expires_at,
        )
        .await
        {
            Ok(api_key) if !api_key.is_empty() => {
                provider.api_key = Some(api_key);
            }
            Ok(_) => {}
            Err(error) => {
                warn!(
                    "Failed to hydrate API key for provider {} from {}: {}",
                    provider.provider_id, site_url, error
                );
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct TokenEntry {
    key: String,
    enabled: bool,
}

fn build_user_id_headers(user_id: i64) -> HeaderMap {
    let id_str = user_id.to_string();
    let mut headers = HeaderMap::new();

    let names = [
        "New-API-User",
        "Veloera-User",
        "voapi-user",
        "User-id",
        "Rix-Api-User",
        "neo-api-user",
    ];

    for name in names {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&id_str),
        ) {
            headers.insert(header_name, header_value);
        }
    }

    headers
}

fn parse_token_entries(body: &Value) -> Vec<TokenEntry> {
    extract_token_array(body)
        .unwrap_or_default()
        .iter()
        .filter_map(parse_token_entry)
        .collect()
}

fn select_first_usable_key(entries: &[TokenEntry]) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.enabled && !entry.key.is_empty())
        .map(|entry| entry.key.clone())
}

async fn fetch_api_key_with_client(
    client: &reqwest::Client,
    site_url: &str,
    site_type: &str,
    access_token: &str,
    user_id: i64,
    auth_type: Option<&str>,
    cookie_auth_session_cookie: Option<&str>,
    refresh_token: Option<&str>,
    token_expires_at: Option<i64>,
) -> Result<String, String> {
    if site_type.eq_ignore_ascii_case("sub2api") {
        return fetch_sub2api_api_key(
            client,
            site_url,
            access_token,
            refresh_token,
            token_expires_at,
        )
        .await;
    }

    let url = format!("{}/api/token/?p=0&size=100", site_url.trim_end_matches('/'));

    let mut headers = build_user_id_headers(user_id);
    apply_auth_headers(
        &mut headers,
        auth_type,
        access_token,
        cookie_auth_session_cookie,
    )?;

    let resp = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch tokens from {}: {}", site_url, e))?;

    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(format!(
            "Auth failed fetching tokens from {} (HTTP {})",
            site_url, status
        ));
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "Unexpected status {} fetching tokens from {}",
            status, site_url
        ));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response from {}: {}", site_url, e))?;

    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("{} responded: {}", site_url, msg));
    }

    let entries = parse_token_entries(&body);
    if entries.is_empty() {
        return Err(format!("No tokens returned from {}", site_url));
    }

    select_first_usable_key(&entries)
        .ok_or_else(|| format!("No enabled tokens found on {}", site_url))
}

fn apply_auth_headers(
    headers: &mut HeaderMap,
    auth_type: Option<&str>,
    access_token: &str,
    cookie_auth_session_cookie: Option<&str>,
) -> Result<(), String> {
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );

    let normalized_auth_type = auth_type.unwrap_or("access_token").trim().to_lowercase();
    if normalized_auth_type == "cookie" {
        if let Some(cookie) = cookie_auth_session_cookie.filter(|value| !value.trim().is_empty()) {
            let cookie_value = HeaderValue::from_str(cookie.trim())
                .map_err(|e| format!("Invalid cookie header value: {}", e))?;
            headers.insert(reqwest::header::COOKIE, cookie_value);
        }
        return Ok(());
    }

    if !access_token.trim().is_empty() {
        let bearer = format!("Bearer {}", access_token.trim());
        let auth_value = HeaderValue::from_str(&bearer)
            .map_err(|e| format!("Invalid authorization header value: {}", e))?;
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
    }

    Ok(())
}

async fn fetch_sub2api_api_key(
    client: &reqwest::Client,
    site_url: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    token_expires_at: Option<i64>,
) -> Result<String, String> {
    let mut current_access_token = access_token.trim().to_string();

    if should_refresh_sub2api_token(token_expires_at) {
        if let Some(refresh_token) = refresh_token.filter(|value| !value.trim().is_empty()) {
            if let Ok(refreshed) =
                refresh_sub2api_access_token(client, site_url, &current_access_token, refresh_token)
                    .await
            {
                current_access_token = refreshed;
            }
        }
    }

    match fetch_sub2api_keys_once(client, site_url, &current_access_token).await {
        Ok(key) => Ok(key),
        Err(error) if is_auth_error_message(&error) => {
            let refresh_token = refresh_token
                .filter(|value| !value.trim().is_empty())
                .ok_or(error)?;
            let refreshed = refresh_sub2api_access_token(
                client,
                site_url,
                &current_access_token,
                refresh_token,
            )
            .await?;
            fetch_sub2api_keys_once(client, site_url, &refreshed).await
        }
        Err(error) => Err(error),
    }
}

async fn fetch_sub2api_keys_once(
    client: &reqwest::Client,
    site_url: &str,
    access_token: &str,
) -> Result<String, String> {
    let url = format!(
        "{}/api/v1/keys?page=1&page_size=100",
        site_url.trim_end_matches('/')
    );

    let mut headers = HeaderMap::new();
    apply_auth_headers(&mut headers, Some("access_token"), access_token, None)?;

    let resp = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Sub2API keys from {}: {}", site_url, e))?;

    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(format!(
            "Auth failed fetching Sub2API keys from {} (HTTP {})",
            site_url, status
        ));
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "Unexpected status {} fetching Sub2API keys from {}",
            status, site_url
        ));
    }

    let body: Value = resp.json().await.map_err(|e| {
        format!(
            "Failed to parse Sub2API key response from {}: {}",
            site_url, e
        )
    })?;

    if body.get("code").and_then(|v| v.as_i64()).unwrap_or(0) != 0 {
        let message = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("{} responded: {}", site_url, message));
    }

    let entries = parse_token_entries(&body);
    if entries.is_empty() {
        return Err(format!("No keys returned from {}", site_url));
    }

    select_first_usable_key(&entries)
        .ok_or_else(|| format!("No enabled keys found on {}", site_url))
}

async fn refresh_sub2api_access_token(
    client: &reqwest::Client,
    site_url: &str,
    access_token: &str,
    refresh_token: &str,
) -> Result<String, String> {
    let url = format!("{}/api/v1/auth/refresh", site_url.trim_end_matches('/'));

    let mut request = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "refresh_token": refresh_token.trim(),
        }));

    if !access_token.trim().is_empty() {
        request = request.header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", access_token.trim()),
        );
    }

    let resp = request
        .send()
        .await
        .map_err(|e| format!("Failed to refresh Sub2API token from {}: {}", site_url, e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Failed to refresh Sub2API token from {} (HTTP {})",
            site_url,
            resp.status()
        ));
    }

    let body: Value = resp.json().await.map_err(|e| {
        format!(
            "Failed to parse Sub2API refresh response from {}: {}",
            site_url, e
        )
    })?;

    if body.get("code").and_then(|v| v.as_i64()).unwrap_or(0) != 0 {
        let message = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("{} responded: {}", site_url, message));
    }

    body.get("data")
        .and_then(|value| value.get("access_token"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "Sub2API refresh response missing access_token from {}",
                site_url
            )
        })
}

fn should_refresh_sub2api_token(token_expires_at: Option<i64>) -> bool {
    let Some(expires_at_ms) = token_expires_at else {
        return false;
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    expires_at_ms - now_ms <= 120_000
}

fn is_auth_error_message(message: &str) -> bool {
    message.contains("HTTP 401") || message.contains("HTTP 403") || message.contains("Auth failed")
}

fn extract_token_array(body: &Value) -> Option<Vec<Value>> {
    if let Some(arr) = body.as_array() {
        return Some(arr.to_vec());
    }

    if let Some(arr) = body.get("items").and_then(|value| value.as_array()) {
        return Some(arr.to_vec());
    }

    if let Some(data) = body.get("data") {
        if let Some(arr) = data.as_array() {
            return Some(arr.to_vec());
        }
        if let Some(arr) = data.get("items").and_then(|value| value.as_array()) {
            return Some(arr.to_vec());
        }
        if let Some(arr) = data.get("data").and_then(|value| value.as_array()) {
            return Some(arr.to_vec());
        }
    }

    None
}

fn parse_token_entry(value: &Value) -> Option<TokenEntry> {
    let key = get_string(value, &["key", "channel_key"])?;
    let enabled = parse_enabled_status(value.get("status"));

    Some(TokenEntry {
        key: ensure_sk_prefixed_key(&key),
        enabled,
    })
}

fn parse_enabled_status(value: Option<&Value>) -> bool {
    match value {
        None => true,
        Some(Value::Number(number)) => number.as_i64().unwrap_or_default() == 1,
        Some(Value::String(status)) => {
            let normalized = status.trim().to_lowercase();
            normalized.is_empty()
                || normalized == "active"
                || normalized == "enabled"
                || normalized == "1"
        }
        Some(Value::Bool(flag)) => *flag,
        _ => false,
    }
}

fn ensure_sk_prefixed_key(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.to_lowercase().starts_with("sk-") {
        trimmed.to_string()
    } else {
        format!("sk-{}", trimmed)
    }
}

fn infer_openclaw_api(npm: &str) -> String {
    match npm {
        "@ai-sdk/anthropic" => "anthropic-messages".to_string(),
        "@ai-sdk/google" => "google-generative-ai".to_string(),
        _ => "openai-completions".to_string(),
    }
}

fn normalize_provider_base_url(url: &str, npm: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return trimmed;
    }

    match npm {
        "@ai-sdk/google" => {
            if trimmed.ends_with("/v1") || trimmed.ends_with("/v1beta") {
                trimmed
            } else {
                format!("{}/v1beta", trimmed)
            }
        }
        "@ai-sdk/anthropic" | "@ai-sdk/openai-compatible" => {
            if trimmed.ends_with("/v1") {
                trimmed
            } else {
                format!("{}/v1", trimmed)
            }
        }
        _ => trimmed,
    }
}

fn uniquify_provider_id(base_id: &str, counts: &mut HashMap<String, usize>) -> String {
    let count = counts.entry(base_id.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base_id.to_string()
    } else {
        format!("{}-{}", base_id, *count)
    }
}

fn extract_host(url: &str) -> String {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .split(':')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

fn slugify(input: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "provider".to_string()
    } else {
        trimmed
    }
}

fn get_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

fn get_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_i64()))
}

fn get_f64(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|n| n as f64))
                .or_else(|| v.as_u64().map(|n| n as f64))
        })
    })
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }

    Ok(())
}

fn unwrap_json_string(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        if let Ok(inner) = serde_json::from_str::<String>(trimmed) {
            return inner;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::{infer_npm, mask_api_key_preview, slugify, uniquify_provider_id};
    use std::collections::HashMap;

    #[test]
    fn slugify_handles_symbols() {
        assert_eq!(slugify("OpenAI 官方"), "openai");
        assert_eq!(slugify("My Provider / Prod"), "my-provider-prod");
    }

    #[test]
    fn unique_ids_increment() {
        let mut counts = HashMap::new();
        assert_eq!(uniquify_provider_id("demo", &mut counts), "demo");
        assert_eq!(uniquify_provider_id("demo", &mut counts), "demo-2");
    }

    #[test]
    fn infer_sdk_by_url() {
        assert_eq!(
            infer_npm(None, Some("https://api.anthropic.com")),
            "@ai-sdk/anthropic"
        );
        assert_eq!(
            infer_npm(None, Some("https://generativelanguage.googleapis.com")),
            "@ai-sdk/google"
        );
    }

    #[test]
    fn mask_api_key_preview_never_exposes_short_key() {
        assert_eq!(mask_api_key_preview("abcd"), "****");
        assert_eq!(mask_api_key_preview("abcdef"), "ab***");
        assert_eq!(mask_api_key_preview("abcdefghij"), "abc...ij");
    }
}
