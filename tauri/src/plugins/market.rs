//! Magpie-compatible market registries, merging and npm metadata.
//!
//! A market is a `registry.json` in the shape Magpie publishes: a top-level
//! `plugins` array whose entries name an npm package and describe what it
//! contributes. Every source is fetched through the application HTTP client so
//! the user's proxy setting applies, cached under `plugins/cache/markets`, and
//! the built-in source has a compiled-in snapshot for offline use.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::paths::PluginPaths;
use super::types::{InstalledPluginRecord, MarketPluginListing, MarketSourceRecord, PluginKind};

pub const BUILTIN_MARKET_ID: &str = "magpie-community";
pub const BUILTIN_MARKET_NAME: &str = "Magpie Community";
pub const BUILTIN_MARKET_URL: &str =
    "https://raw.githubusercontent.com/magpie-community/plugins/main/registry.json";

/// Hard cap for one registry response. A market is a list of package names and
/// short summaries; anything larger is a misconfigured URL or a hostile server.
pub const MAX_REGISTRY_BYTES: usize = 2 * 1024 * 1024;

const REGISTRY_FETCH_TIMEOUT_SECS: u64 = 20;
const NPM_REQUEST_TIMEOUT_SECS: u64 = 6;
const NPM_SEARCH_TIMEOUT_SECS: u64 = 10;
const NPM_CONCURRENCY: usize = 6;
pub const NPM_CACHE_TTL_SECS: i64 = 3600;
const NPM_DOWNLOADS_BASE_URL: &str = "https://api.npmjs.org";
const NPM_SEARCH_PAGE_SIZE: usize = 30;

/// The built-in snapshot, embedded at compile time so an offline first launch
/// still has a browsable market.
pub const FALLBACK_MARKET_JSON: &str = include_str!("../../resources/magpie_market_fallback.json");

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MarketListing {
    pub package: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub community: bool,
    #[serde(default)]
    pub replaces: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub summary: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct RegistryFile {
    plugins: Vec<MarketListing>,
}

fn package_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(@[a-z0-9][a-z0-9._~-]*/)?[a-z0-9][a-z0-9._~-]*$")
            .expect("package name regex is valid")
    })
}

pub fn is_valid_package_name(name: &str) -> bool {
    package_name_regex().is_match(name)
}

/// Accept a plain HTTP(S) registry URL or `github:owner/repo`, which is
/// normalized to the raw file on the repository's `main` branch.
pub fn normalize_market_url(input: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() {
        return Err("Market URL cannot be empty".to_string());
    }

    if let Some(rest) = value.strip_prefix("github:") {
        let rest = rest.split('#').next().unwrap_or("");
        let (owner, repo) = rest
            .split_once('/')
            .ok_or_else(|| "github: sources must be github:owner/repo".to_string())?;
        if !is_valid_github_part(owner) || !is_valid_github_part(repo) {
            return Err("github: sources must be github:owner/repo".to_string());
        }
        return Ok(format!(
            "https://raw.githubusercontent.com/{owner}/{repo}/main/registry.json"
        ));
    }

    let parsed =
        url::Url::parse(value).map_err(|error| format!("Invalid market URL '{value}': {error}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!("Market URL '{value}' must use http or https"));
    }
    if parsed.host_str().is_none() {
        return Err(format!("Market URL '{value}' has no host"));
    }
    Ok(value.to_string())
}

fn is_valid_github_part(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

/// Parse a registry body. Individual entries with an invalid package name are
/// skipped (one typo must not take the whole market down); a missing `plugins`
/// array or a body with no valid entry is rejected.
pub fn parse_registry(bytes: &[u8]) -> Result<Vec<MarketListing>, String> {
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err(format!(
            "Market registry is {} bytes; the limit is {}",
            bytes.len(),
            MAX_REGISTRY_BYTES
        ));
    }

    let registry: RegistryFile = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid market registry JSON: {error}"))?;

    let mut listings = Vec::new();
    for mut listing in registry.plugins {
        let package = listing.package.trim().to_string();
        if !is_valid_package_name(&package) {
            continue;
        }
        listing.package = package.clone();
        if listing.name.trim().is_empty() {
            listing.name = package;
        }
        listings.push(listing);
    }

    if listings.is_empty() {
        return Err("The market registry has no valid plugins".to_string());
    }
    Ok(listings)
}

/// The compiled-in snapshot, parsed once per call site. It is only used after a
/// live fetch of the built-in source fails.
pub fn fallback_market_listings() -> Vec<MarketListing> {
    match parse_registry(FALLBACK_MARKET_JSON.as_bytes()) {
        Ok(listings) => listings,
        Err(error) => {
            log::warn!("Built-in Magpie market snapshot is invalid: {error}");
            Vec::new()
        }
    }
}

/// Download and parse one registry, enforcing the size cap while streaming so
/// a chunked response cannot exhaust memory before the check runs.
pub async fn fetch_registry(client: &Client, url: &str) -> Result<Vec<MarketListing>, String> {
    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(REGISTRY_FETCH_TIMEOUT_SECS))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "ai-toolbox")
        .send()
        .await
        .map_err(|error| format!("Failed to fetch market registry {url}: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Market registry {url} returned HTTP {}",
            response.status()
        ));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_REGISTRY_BYTES as u64 {
            return Err(format!(
                "Market registry {url} is {length} bytes; the limit is {MAX_REGISTRY_BYTES}"
            ));
        }
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Failed to read market registry {url}: {error}"))?
    {
        if body.len() + chunk.len() > MAX_REGISTRY_BYTES {
            return Err(format!(
                "Market registry {url} exceeded the {MAX_REGISTRY_BYTES}-byte limit"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    parse_registry(&body)
}

pub fn write_market_cache(
    paths: &PluginPaths,
    market_id: &str,
    listings: &[MarketListing],
) -> Result<(), String> {
    let path = paths.market_cache_path(market_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec(&RegistryFile {
        plugins: listings.to_vec(),
    })
    .map_err(|error| format!("Failed to serialize market cache: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, &bytes)
        .map_err(|error| format!("Failed to write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| format!("Failed to replace {}: {error}", path.display()))
}

pub fn read_market_cache(paths: &PluginPaths, market_id: &str) -> Option<Vec<MarketListing>> {
    let bytes = std::fs::read(paths.market_cache_path(market_id)).ok()?;
    parse_registry(&bytes).ok()
}

/// What a refresh produced. `last_error` is set when the list shown is not the
/// one just fetched (fallback snapshot or a previous cache hit).
#[derive(Clone, Debug, Default)]
pub struct MarketFetchOutcome {
    pub listings: Vec<MarketListing>,
    pub last_error: Option<String>,
    pub from_fallback: bool,
}

/// The list to show when a fetch failed: the compiled snapshot for the built-in
/// market, the last cached copy for a custom one, or nothing.
pub fn offline_fallback(
    paths: &PluginPaths,
    source: &MarketSourceRecord,
    error: String,
) -> MarketFetchOutcome {
    if source.id == BUILTIN_MARKET_ID {
        return MarketFetchOutcome {
            listings: fallback_market_listings(),
            last_error: Some(error),
            from_fallback: true,
        };
    }
    if let Some(cached) = read_market_cache(paths, &source.id) {
        return MarketFetchOutcome {
            listings: cached,
            last_error: Some(error),
            from_fallback: false,
        };
    }
    MarketFetchOutcome {
        listings: Vec::new(),
        last_error: Some(error),
        from_fallback: false,
    }
}

/// Fetch one source, falling back to [`offline_fallback`] on failure.
pub async fn refresh_market_source(
    client: &Client,
    paths: &PluginPaths,
    source: &MarketSourceRecord,
) -> MarketFetchOutcome {
    match fetch_registry(client, &source.url).await {
        Ok(listings) => {
            if let Err(error) = write_market_cache(paths, &source.id, &listings) {
                log::warn!("Failed to cache market {}: {error}", source.id);
            }
            MarketFetchOutcome {
                listings,
                last_error: None,
                from_fallback: false,
            }
        }
        Err(error) => offline_fallback(paths, source, error),
    }
}

fn listing_to_market_plugin(listing: &MarketListing) -> MarketPluginListing {
    MarketPluginListing {
        package: listing.package.clone(),
        name: listing.name.clone(),
        icon: listing.icon.clone(),
        providers: listing.providers.clone(),
        community: listing.community,
        replaces: (!listing.replaces.trim().is_empty()).then(|| listing.replaces.clone()),
        kind: listing.kind.clone(),
        summary: listing.summary.clone(),
        ..MarketPluginListing::default()
    }
}

/// Merge every source into one list keyed by package name.
///
/// First sighting wins for metadata and position; later sources only add their
/// id to `market_ids`, preserving source order. Installed state comes from the
/// package-name map so a plugin installed before any market was refreshed still
/// shows as installed.
pub fn merge_market_listings(
    markets: &[(String, Vec<MarketListing>)],
    installed: &HashMap<String, InstalledPluginRecord>,
) -> Vec<MarketPluginListing> {
    let mut merged: Vec<MarketPluginListing> = Vec::new();
    let mut index_by_package: HashMap<String, usize> = HashMap::new();

    for (market_id, listings) in markets {
        for listing in listings {
            if let Some(index) = index_by_package.get(&listing.package).copied() {
                let entry = &mut merged[index];
                if !entry.market_ids.iter().any(|id| id == market_id) {
                    entry.market_ids.push(market_id.clone());
                }
                continue;
            }

            let mut entry = listing_to_market_plugin(listing);
            entry.market_ids = vec![market_id.clone()];
            if let Some(installed) = installed.get(&listing.package) {
                entry.installed = true;
                entry.installed_version = Some(installed.version.clone());
            }
            index_by_package.insert(listing.package.clone(), merged.len());
            merged.push(entry);
        }
    }

    merged
}

/// Apply the discovery page's `search` and `kind` filters.
///
/// `kind` accepts the same values the UI uses: `provider` matches provider and
/// both listings, `middleware` matches middleware and both, and `both` matches
/// only packages that declare both entries.
pub fn filter_market_listings(
    listings: Vec<MarketPluginListing>,
    search: Option<&str>,
    kind: Option<&str>,
) -> Vec<MarketPluginListing> {
    let needle = search
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase);
    let kind = kind
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("all"))
        .map(str::to_lowercase);

    listings
        .into_iter()
        .filter(|listing| {
            if let Some(needle) = &needle {
                let mut haystack = format!(
                    "{} {} {} {}",
                    listing.package, listing.name, listing.description, listing.kind
                );
                for value in listing.summary.values() {
                    haystack.push(' ');
                    haystack.push_str(value);
                }
                for provider in &listing.providers {
                    haystack.push(' ');
                    haystack.push_str(provider);
                }
                if !haystack.to_lowercase().contains(needle) {
                    return false;
                }
            }

            let Some(kind) = &kind else {
                return true;
            };
            let entry_kind = PluginKind::from_listing_kind(&listing.kind);
            match kind.as_str() {
                "provider" => matches!(entry_kind, PluginKind::Provider | PluginKind::Both),
                "middleware" => {
                    matches!(entry_kind, PluginKind::Middleware | PluginKind::Both)
                }
                "both" => matches!(entry_kind, PluginKind::Both),
                "page" => matches!(entry_kind, PluginKind::Page),
                other => listing.kind.eq_ignore_ascii_case(other),
            }
        })
        .collect()
}

// ---- npm metadata -----------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpmPackageInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub weekly_downloads: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NpmCacheEntry {
    fetched_at: i64,
    #[serde(default)]
    info: NpmPackageInfo,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NpmInfoCache {
    #[serde(default)]
    packages: HashMap<String, NpmCacheEntry>,
}

fn load_npm_cache(path: &Path) -> NpmInfoCache {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_npm_cache(path: &Path, cache: &NpmInfoCache) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        log::warn!(
            "Failed to create npm cache directory {}: {error}",
            parent.display()
        );
        return;
    }
    let Ok(bytes) = serde_json::to_vec(cache) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, &bytes).is_ok() {
        let _ = std::fs::rename(&temporary, path);
    }
}

fn apply_npm_info(listing: &mut MarketPluginListing, info: &NpmPackageInfo) {
    if !info.version.is_empty() {
        listing.latest_version = Some(info.version.clone());
    }
    if !info.description.is_empty() {
        listing.description = info.description.clone();
    }
    if !info.repository.is_empty() {
        listing.repository = Some(info.repository.clone());
    }
    if !info.homepage.is_empty() {
        listing.homepage = Some(info.homepage.clone());
    }
    if !info.license.is_empty() {
        listing.license = Some(info.license.clone());
    }
    if info.weekly_downloads > 0 {
        listing.weekly_downloads = Some(info.weekly_downloads);
    }
}

fn apply_cached_npm_info(listing: &mut MarketPluginListing, cache: &NpmInfoCache) {
    if let Some(entry) = cache.packages.get(&listing.package) {
        apply_npm_info(listing, &entry.info);
    }
}

/// Fill version / description / repository / weekly downloads from npm.
///
/// Cached answers younger than [`NPM_CACHE_TTL_SECS`] are applied without a
/// request. Missing or stale packages are fetched at most [`NPM_CONCURRENCY`]
/// at a time, with a per-request timeout; a failed lookup leaves the listing's
/// registry metadata intact.
pub async fn enrich_listings_with_npm_info(
    client: &Client,
    registry: &str,
    cache_path: &Path,
    listings: &mut [MarketPluginListing],
) {
    let mut cache = load_npm_cache(cache_path);
    let now = chrono::Utc::now().timestamp();

    let mut stale: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for listing in listings.iter_mut() {
        let cached_is_fresh = cache
            .packages
            .get(&listing.package)
            .map(|entry| now - entry.fetched_at < NPM_CACHE_TTL_SECS)
            .unwrap_or(false);
        if cached_is_fresh {
            apply_cached_npm_info(listing, &cache);
        } else if seen.insert(listing.package.clone()) {
            stale.push(listing.package.clone());
        }
    }

    if stale.is_empty() {
        return;
    }

    let registry = registry.trim_end_matches('/').to_string();
    let fetched = futures_util::stream::iter(stale.into_iter().map(|package| {
        let client = client.clone();
        let registry = registry.clone();
        async move {
            let info = fetch_npm_info(&client, &registry, &package).await;
            (package, info)
        }
    }))
    .buffer_unordered(NPM_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let mut changed = false;
    for (package, info) in fetched {
        if let Ok(info) = info {
            cache.packages.insert(
                package,
                NpmCacheEntry {
                    fetched_at: now,
                    info,
                },
            );
            changed = true;
        }
    }
    if changed {
        save_npm_cache(cache_path, &cache);
    }

    for listing in listings.iter_mut() {
        apply_cached_npm_info(listing, &cache);
    }
}

async fn fetch_npm_info(
    client: &Client,
    registry: &str,
    package: &str,
) -> Result<NpmPackageInfo, String> {
    let path = npm_package_url_path(package);
    let latest_url = format!("{registry}/{path}/latest");
    let downloads_url = format!("{NPM_DOWNLOADS_BASE_URL}/downloads/point/last-week/{path}");

    let latest = client
        .get(&latest_url)
        .timeout(Duration::from_secs(NPM_REQUEST_TIMEOUT_SECS))
        .send();
    let downloads = client
        .get(&downloads_url)
        .timeout(Duration::from_secs(NPM_REQUEST_TIMEOUT_SECS))
        .send();
    let (latest, downloads) = tokio::join!(latest, downloads);

    let mut info = NpmPackageInfo::default();
    if let Ok(response) = latest {
        if response.status().is_success() {
            if let Ok(value) = response.json::<Value>().await {
                info.version = value
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.description = value
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.homepage = value
                    .get("homepage")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.license = value
                    .get("license")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.repository = value
                    .get("repository")
                    .map(repository_url)
                    .unwrap_or_default();
            }
        }
    }
    if let Ok(response) = downloads {
        if response.status().is_success() {
            if let Ok(value) = response.json::<Value>().await {
                info.weekly_downloads = value
                    .get("downloads")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
            }
        }
    }

    if info.version.is_empty() && info.weekly_downloads == 0 {
        return Err(format!("npm has no metadata for {package}"));
    }
    Ok(info)
}

/// Normalize `repository` (string or `{ "url": ... }`) into a web URL.
fn repository_url(value: &Value) -> String {
    let raw = match value {
        Value::String(value) => value.clone(),
        Value::Object(object) => object
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => return String::new(),
    };
    let mut url = raw.trim().to_string();
    url = url.strip_prefix("git+").unwrap_or(&url).to_string();
    url = url.strip_suffix(".git").unwrap_or(&url).to_string();
    url = url.replacen("git://", "https://", 1);
    url = url.replacen("ssh://git@", "https://", 1);
    if let Some(rest) = url.strip_prefix("github:") {
        url = format!("https://github.com/{rest}");
    } else if !url.contains("://") && url.matches('/').count() == 1 {
        url = format!("https://github.com/{url}");
    }
    if url.starts_with("https://") {
        url
    } else {
        String::new()
    }
}

fn npm_package_url_path(package: &str) -> String {
    let mut encoded = String::new();
    for byte in package.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'@' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

// ---- npm search -------------------------------------------------------------

/// Search npm for plugin packages. Results carry npm's own metadata; the
/// caller merges them with installed state and the registry listings.
pub async fn search_npm(
    client: &Client,
    registry: &str,
    query: &str,
) -> Result<Vec<MarketPluginListing>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let base = registry.trim_end_matches('/');
    let mut url = url::Url::parse(&format!("{base}/-/v1/search"))
        .map_err(|error| format!("Invalid npm registry URL '{registry}': {error}"))?;
    url.query_pairs_mut()
        .append_pair("text", query)
        .append_pair("size", &NPM_SEARCH_PAGE_SIZE.to_string());

    let response = client
        .get(url)
        .timeout(Duration::from_secs(NPM_SEARCH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| format!("npm search failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("npm search returned HTTP {}", response.status()));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| format!("npm search returned invalid JSON: {error}"))?;

    let mut results = Vec::new();
    for object in value
        .get("objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(package) = object.get("package") else {
            continue;
        };
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !is_valid_package_name(name) {
            continue;
        }
        let keywords: Vec<String> = package
            .get("keywords")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let is_middleware = name.to_ascii_lowercase().contains("middleware")
            || keywords
                .iter()
                .any(|keyword| keyword.to_ascii_lowercase().contains("middleware"));

        results.push(MarketPluginListing {
            package: name.to_string(),
            name: name.to_string(),
            community: name.starts_with("@magpie-community/")
                || keywords.iter().any(|keyword| keyword == "magpie-plugin"),
            kind: if is_middleware {
                "middleware".to_string()
            } else {
                String::new()
            },
            latest_version: package
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_string),
            description: package
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            license: package
                .get("license")
                .and_then(Value::as_str)
                .map(str::to_string),
            homepage: package
                .pointer("/links/homepage")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            repository: package
                .pointer("/links/repository")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            weekly_downloads: object.pointer("/downloads/weekly").and_then(Value::as_u64),
            ..MarketPluginListing::default()
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn installed(package: &str, version: &str) -> InstalledPluginRecord {
        InstalledPluginRecord {
            package_name: package.to_string(),
            version: version.to_string(),
            ..InstalledPluginRecord::default()
        }
    }

    #[test]
    fn parses_the_built_in_snapshot() {
        let listings = fallback_market_listings();
        assert!(!listings.is_empty());
        assert!(listings
            .iter()
            .any(|listing| listing.package == "@magpie-community/opencode-zen-free-auth"));
    }

    #[test]
    fn parses_registry_and_skips_invalid_packages() {
        let body = br#"{
            "version": 1,
            "plugins": [
                { "package": "@scope/demo", "name": "Demo", "kind": "middleware", "summary": { "en": "hi" } },
                { "package": "Not A Package", "name": "Broken" },
                { "package": "plain", "name": "" }
            ]
        }"#;
        let listings = parse_registry(body).unwrap();
        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].name, "Demo");
        assert_eq!(
            listings[1].name, "plain",
            "a missing name defaults to the package"
        );
    }

    #[test]
    fn rejects_a_registry_without_a_plugins_array() {
        let error = parse_registry(br#"{"version":1}"#).unwrap_err();
        assert!(error.contains("missing field"), "{error}");
    }

    #[test]
    fn rejects_a_registry_over_the_size_limit() {
        let oversized = vec![b' '; MAX_REGISTRY_BYTES + 1];
        let error = parse_registry(&oversized).unwrap_err();
        assert!(
            error.contains("limit") || error.contains("larger"),
            "{error}"
        );
    }

    #[test]
    fn rejects_a_registry_with_no_valid_plugins() {
        let error = parse_registry(br#"{"plugins":[{"package":"!!!"}]}"#).unwrap_err();
        assert!(error.contains("no valid plugins"), "{error}");
    }

    #[test]
    fn normalizes_github_sources_and_rejects_other_schemes() {
        assert_eq!(
            normalize_market_url("github:owner/repo").unwrap(),
            "https://raw.githubusercontent.com/owner/repo/main/registry.json"
        );
        assert_eq!(
            normalize_market_url("https://example.com/registry.json").unwrap(),
            "https://example.com/registry.json"
        );
        assert!(normalize_market_url("github:owner").is_err());
        assert!(normalize_market_url("ftp://example.com/list.json").is_err());
        assert!(normalize_market_url("not a url").is_err());
    }

    #[test]
    fn builtin_market_falls_back_to_the_embedded_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        let source = MarketSourceRecord {
            id: BUILTIN_MARKET_ID.to_string(),
            name: BUILTIN_MARKET_NAME.to_string(),
            url: BUILTIN_MARKET_URL.to_string(),
            enabled: true,
            builtin: true,
            ..MarketSourceRecord::default()
        };

        let outcome = offline_fallback(&paths, &source, "network down".to_string());
        assert!(outcome.from_fallback);
        assert_eq!(outcome.last_error.as_deref(), Some("network down"));
        assert!(!outcome.listings.is_empty());
    }

    #[test]
    fn custom_market_falls_back_to_its_last_cache_or_nothing() {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        let source = MarketSourceRecord {
            id: "custom".to_string(),
            name: "Custom".to_string(),
            url: "https://example.com/registry.json".to_string(),
            enabled: true,
            ..MarketSourceRecord::default()
        };

        let empty = offline_fallback(&paths, &source, "down".to_string());
        assert!(empty.listings.is_empty());
        assert!(!empty.from_fallback);
        assert_eq!(empty.last_error.as_deref(), Some("down"));

        write_market_cache(
            &paths,
            "custom",
            &[MarketListing {
                package: "cached".to_string(),
                name: "Cached".to_string(),
                ..MarketListing::default()
            }],
        )
        .unwrap();
        let cached = offline_fallback(&paths, &source, "down".to_string());
        assert_eq!(cached.listings.len(), 1);
        assert_eq!(cached.listings[0].package, "cached");
        assert!(!cached.from_fallback);
        assert_eq!(cached.last_error.as_deref(), Some("down"));
    }

    #[test]
    fn merges_duplicate_packages_and_marks_installed_state() {
        let first = vec![
            MarketListing {
                package: "shared".to_string(),
                name: "Shared".to_string(),
                ..MarketListing::default()
            },
            MarketListing {
                package: "only-a".to_string(),
                name: "Only A".to_string(),
                ..MarketListing::default()
            },
        ];
        let second = vec![MarketListing {
            package: "shared".to_string(),
            name: "Shared (other)".to_string(),
            ..MarketListing::default()
        }];

        let mut installed_map = HashMap::new();
        installed_map.insert("shared".to_string(), installed("shared", "2.0.0"));

        let merged = merge_market_listings(
            &[
                ("market-a".to_string(), first),
                ("market-b".to_string(), second),
            ],
            &installed_map,
        );

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].package, "shared");
        assert_eq!(merged[0].name, "Shared", "first sighting wins");
        assert_eq!(merged[0].market_ids, vec!["market-a", "market-b"]);
        assert!(merged[0].installed);
        assert_eq!(merged[0].installed_version.as_deref(), Some("2.0.0"));
        assert_eq!(merged[1].market_ids, vec!["market-a"]);
        assert!(!merged[1].installed);
    }

    #[test]
    fn filters_by_search_text_and_kind() {
        let listings = vec![
            MarketPluginListing {
                package: "middleware-one".to_string(),
                name: "Middleware One".to_string(),
                kind: "middleware".to_string(),
                ..MarketPluginListing::default()
            },
            MarketPluginListing {
                package: "provider-one".to_string(),
                name: "Provider One".to_string(),
                ..MarketPluginListing::default()
            },
        ];

        let middleware = filter_market_listings(listings.clone(), None, Some("middleware"));
        assert_eq!(middleware.len(), 1);
        assert_eq!(middleware[0].package, "middleware-one");

        let providers = filter_market_listings(listings.clone(), None, Some("provider"));
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].package, "provider-one");

        let searched = filter_market_listings(listings, Some("PROVIDER"), None);
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].package, "provider-one");
    }

    #[test]
    fn npm_package_paths_keep_the_scope_and_escape_the_slash() {
        assert_eq!(npm_package_url_path("plain"), "plain");
        assert_eq!(npm_package_url_path("@scope/pkg"), "@scope%2Fpkg");
        assert_eq!(
            npm_package_url_path("@scope/pkg name"),
            "@scope%2Fpkg%20name"
        );
    }

    #[test]
    fn repository_urls_are_normalized() {
        assert_eq!(
            repository_url(&json!({ "url": "git+https://github.com/o/r.git" })),
            "https://github.com/o/r"
        );
        assert_eq!(
            repository_url(&json!("owner/repo")),
            "https://github.com/owner/repo"
        );
        assert_eq!(repository_url(&json!({ "url": "ftp://nope" })), "");
    }
}
