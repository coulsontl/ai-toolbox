use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::plugin_toml;
use super::plugin_types::{
    CodexInstalledPlugin, CodexMarketplacePlugin, CodexPluginMarketplace, CodexPluginRuntimeStatus,
};
use super::plugin_workspace;
use crate::coding::runtime_location::{self, RuntimeLocationMode};

const MARKETPLACE_RELATIVE_PATH: &str = ".agents/plugins/marketplace.json";
const DEFAULT_PLUGIN_VERSION: &str = "local";

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawMarketplaceManifest {
    name: String,
    #[serde(default)]
    interface: Option<RawMarketplaceInterface>,
    #[serde(default)]
    plugins: Vec<RawMarketplacePlugin>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawMarketplaceInterface {
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawMarketplacePlugin {
    name: String,
    #[serde(default)]
    source: RawMarketplacePluginSource,
    #[serde(default)]
    policy: RawMarketplacePluginPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
enum RawMarketplacePluginSource {
    Local { path: String },
}

impl Default for RawMarketplacePluginSource {
    fn default() -> Self {
        Self::Local {
            path: String::new(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawMarketplacePluginPolicy {
    #[serde(default = "default_install_policy")]
    installation: String,
    #[serde(default)]
    products: Option<Vec<String>>,
}

fn default_install_policy() -> String {
    "AVAILABLE".to_string()
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawPluginManifest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    skills: Option<String>,
    #[serde(default)]
    mcp_servers: Option<String>,
    #[serde(default)]
    apps: Option<String>,
    #[serde(default)]
    interface: Option<RawPluginInterface>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawPluginInterface {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
struct MarketplaceRecord {
    name: String,
    path: PathBuf,
    display_name: Option<String>,
    plugins: Vec<MarketplacePluginRecord>,
    is_curated: bool,
}

#[derive(Debug, Clone)]
struct MarketplacePluginRecord {
    plugin_id: String,
    marketplace_name: String,
    marketplace_path: PathBuf,
    plugin_name: String,
    source_path: Option<PathBuf>,
    display_name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    capabilities: Vec<String>,
    install_available: bool,
}

fn read_json_file<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let raw_content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?;
    serde_json::from_str::<T>(&raw_content)
        .map_err(|error| format!("Failed to parse {}: {}", path.display(), error))
}

fn codex_plugins_dir(root_dir: &Path) -> PathBuf {
    root_dir.join("plugins")
}

fn codex_plugins_cache_dir(root_dir: &Path) -> PathBuf {
    codex_plugins_dir(root_dir).join("cache")
}

fn codex_curated_repo_dir(root_dir: &Path) -> PathBuf {
    root_dir.join(".tmp").join("plugins")
}

fn codex_curated_marketplace_path(root_dir: &Path) -> PathBuf {
    codex_curated_repo_dir(root_dir).join(MARKETPLACE_RELATIVE_PATH)
}

fn home_marketplace_path(
    runtime_location: &runtime_location::RuntimeLocationInfo,
) -> Option<PathBuf> {
    if let Some(wsl) = runtime_location.wsl.as_ref() {
        let linux_user_root = wsl.linux_user_root.as_deref()?;
        let linux_marketplace_path = format!(
            "{}/.agents/plugins/marketplace.json",
            linux_user_root.trim_end_matches('/')
        );
        return Some(runtime_location::build_windows_unc_path(
            &wsl.distro,
            &linux_marketplace_path,
        ));
    }

    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(PathBuf::from)
        .map(|home_dir| home_dir.join(MARKETPLACE_RELATIVE_PATH))
}

fn plugin_manifest_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(".codex-plugin").join("plugin.json")
}

fn read_plugin_manifest(plugin_root: &Path) -> Option<RawPluginManifest> {
    let manifest_path = plugin_manifest_path(plugin_root);
    if !manifest_path.is_file() {
        return None;
    }
    read_json_file::<RawPluginManifest>(&manifest_path).ok()
}

fn plugin_id(plugin_name: &str, marketplace_name: &str) -> String {
    format!("{}@{}", plugin_name, marketplace_name)
}

fn discover_marketplace_paths(
    runtime_location: &runtime_location::RuntimeLocationInfo,
) -> Vec<(PathBuf, bool)> {
    let mut paths = Vec::new();

    if let Some(home_marketplace_path) = home_marketplace_path(runtime_location) {
        if home_marketplace_path.is_file() {
            paths.push((home_marketplace_path, false));
        }
    }

    let curated_marketplace = codex_curated_marketplace_path(&runtime_location.host_path);
    if curated_marketplace.is_file() {
        paths.push((curated_marketplace, true));
    }

    paths
}

fn merge_marketplace_paths(
    runtime_location: &runtime_location::RuntimeLocationInfo,
    additional_marketplace_paths: &[PathBuf],
) -> Vec<(PathBuf, bool)> {
    let mut marketplace_paths = discover_marketplace_paths(runtime_location);

    for marketplace_path in additional_marketplace_paths {
        if marketplace_paths
            .iter()
            .any(|(existing_path, _)| existing_path == marketplace_path)
        {
            continue;
        }
        marketplace_paths.push((marketplace_path.clone(), false));
    }

    marketplace_paths
}

fn resolve_marketplace_root(marketplace_path: &Path) -> Option<PathBuf> {
    marketplace_path
        .parent()?
        .parent()?
        .parent()
        .map(Path::to_path_buf)
}

fn resolve_plugin_source_path(
    marketplace_path: &Path,
    source: &RawMarketplacePluginSource,
) -> Option<PathBuf> {
    let RawMarketplacePluginSource::Local { path } = source;
    let trimmed = path.trim();
    let relative_path = trimmed.strip_prefix("./")?;
    let marketplace_root = resolve_marketplace_root(marketplace_path)?;
    Some(marketplace_root.join(relative_path))
}

fn is_codex_product_allowed(products: Option<&[String]>) -> bool {
    match products {
        None => true,
        Some([]) => false,
        Some(products) => products
            .iter()
            .any(|product| product.trim().eq_ignore_ascii_case("codex")),
    }
}

fn load_marketplace_record(
    marketplace_path: &Path,
    is_curated: bool,
) -> Result<MarketplaceRecord, String> {
    let raw_marketplace = read_json_file::<RawMarketplaceManifest>(marketplace_path)?;
    let mut plugins = Vec::new();

    for raw_plugin in raw_marketplace.plugins {
        if !is_codex_product_allowed(raw_plugin.policy.products.as_deref()) {
            continue;
        }

        let source_path = resolve_plugin_source_path(marketplace_path, &raw_plugin.source);
        let manifest = source_path.as_deref().and_then(read_plugin_manifest);
        let display_name = manifest
            .as_ref()
            .and_then(|item| item.interface.as_ref())
            .and_then(|item| item.display_name.clone());
        let description = manifest.as_ref().and_then(|item| item.description.clone());
        let category = manifest
            .as_ref()
            .and_then(|item| item.interface.as_ref())
            .and_then(|item| item.category.clone());
        let capabilities = manifest
            .as_ref()
            .and_then(|item| item.interface.as_ref())
            .map(|item| item.capabilities.clone())
            .unwrap_or_default();
        let plugin_id = plugin_id(&raw_plugin.name, &raw_marketplace.name);
        let install_available = raw_plugin.policy.installation != "NOT_AVAILABLE";

        plugins.push(MarketplacePluginRecord {
            plugin_id,
            marketplace_name: raw_marketplace.name.clone(),
            marketplace_path: marketplace_path.to_path_buf(),
            plugin_name: raw_plugin.name,
            source_path,
            display_name,
            description,
            category,
            capabilities,
            install_available,
        });
    }

    Ok(MarketplaceRecord {
        name: raw_marketplace.name,
        path: marketplace_path.to_path_buf(),
        display_name: raw_marketplace
            .interface
            .as_ref()
            .and_then(|item| item.display_name.clone()),
        plugins,
        is_curated,
    })
}

fn list_marketplace_records(
    runtime_location: &runtime_location::RuntimeLocationInfo,
    additional_marketplace_paths: &[PathBuf],
) -> Vec<MarketplaceRecord> {
    let mut marketplaces = Vec::new();
    for (marketplace_path, is_curated) in
        merge_marketplace_paths(runtime_location, additional_marketplace_paths)
    {
        if let Ok(marketplace) = load_marketplace_record(&marketplace_path, is_curated) {
            marketplaces.push(marketplace);
        }
    }
    marketplaces.sort_by(|left, right| left.name.cmp(&right.name));
    marketplaces
}

fn active_plugin_version(plugin_base_dir: &Path) -> Option<String> {
    let entries = fs::read_dir(plugin_base_dir).ok()?;
    let mut versions = Vec::new();
    for entry in entries.flatten() {
        let file_type = entry.file_type().ok()?;
        if !file_type.is_dir() {
            continue;
        }
        let version_name = entry.file_name().to_string_lossy().to_string();
        if !version_name.is_empty() {
            versions.push(version_name);
        }
    }
    versions.sort();
    if versions
        .iter()
        .any(|version| version == DEFAULT_PLUGIN_VERSION)
    {
        return Some(DEFAULT_PLUGIN_VERSION.to_string());
    }
    versions.pop()
}

fn build_installed_plugin_map(root_dir: &Path) -> BTreeMap<String, (String, PathBuf)> {
    let mut installed_plugins = BTreeMap::new();
    let cache_root = codex_plugins_cache_dir(root_dir);
    let Ok(marketplace_dirs) = fs::read_dir(&cache_root) else {
        return installed_plugins;
    };

    for marketplace_entry in marketplace_dirs.flatten() {
        let Ok(marketplace_type) = marketplace_entry.file_type() else {
            continue;
        };
        if !marketplace_type.is_dir() {
            continue;
        }
        let marketplace_name = marketplace_entry.file_name().to_string_lossy().to_string();
        let Ok(plugin_dirs) = fs::read_dir(marketplace_entry.path()) else {
            continue;
        };
        for plugin_entry in plugin_dirs.flatten() {
            let Ok(plugin_type) = plugin_entry.file_type() else {
                continue;
            };
            if !plugin_type.is_dir() {
                continue;
            }
            let plugin_name = plugin_entry.file_name().to_string_lossy().to_string();
            let Some(version) = active_plugin_version(&plugin_entry.path()) else {
                continue;
            };
            let installed_root = plugin_entry.path().join(&version);
            installed_plugins.insert(
                plugin_id(&plugin_name, &marketplace_name),
                (version, installed_root),
            );
        }
    }

    installed_plugins
}

pub async fn get_codex_plugin_runtime_status(
    db: &crate::db::SqliteDbState,
) -> Result<CodexPluginRuntimeStatus, String> {
    let runtime_location = runtime_location::get_codex_runtime_location_async(db).await?;
    let mode = match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => "local".to_string(),
        RuntimeLocationMode::WslDirect => "wslDirect".to_string(),
    };

    Ok(CodexPluginRuntimeStatus {
        mode,
        source: runtime_location.source,
        root_dir: runtime_location.host_path.to_string_lossy().to_string(),
        config_path: runtime_location
            .host_path
            .join("config.toml")
            .to_string_lossy()
            .to_string(),
        plugins_dir: codex_plugins_dir(&runtime_location.host_path)
            .to_string_lossy()
            .to_string(),
        plugins_feature_enabled: plugin_toml::read_plugin_config_state(
            &runtime_location.host_path.join("config.toml"),
        )?
        .plugins_feature_enabled,
        curated_marketplace_path: codex_curated_marketplace_path(&runtime_location.host_path)
            .is_file()
            .then(|| {
                codex_curated_marketplace_path(&runtime_location.host_path)
                    .to_string_lossy()
                    .to_string()
            }),
        distro: runtime_location
            .wsl
            .as_ref()
            .map(|item| item.distro.clone()),
        linux_root_dir: runtime_location
            .wsl
            .as_ref()
            .map(|item| item.linux_path.clone()),
    })
}

pub async fn list_codex_marketplaces(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<CodexPluginMarketplace>, String> {
    let runtime_location = runtime_location::get_codex_runtime_location_async(db).await?;
    let additional_marketplace_paths =
        plugin_workspace::list_ready_codex_workspace_marketplace_paths(db).await?;
    let marketplaces = list_marketplace_records(&runtime_location, &additional_marketplace_paths)
        .into_iter()
        .map(|marketplace| CodexPluginMarketplace {
            name: marketplace.name,
            path: marketplace.path.to_string_lossy().to_string(),
            display_name: marketplace.display_name,
            description: None,
            plugin_count: marketplace.plugins.len(),
            is_curated: marketplace.is_curated,
        })
        .collect();
    Ok(marketplaces)
}

pub async fn list_codex_marketplace_plugins(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<CodexMarketplacePlugin>, String> {
    let runtime_location = runtime_location::get_codex_runtime_location_async(db).await?;
    let additional_marketplace_paths =
        plugin_workspace::list_ready_codex_workspace_marketplace_paths(db).await?;
    let config_path = runtime_location.host_path.join("config.toml");
    let enabled_map = plugin_toml::read_plugin_enabled_map(&config_path)?;
    let installed_map = build_installed_plugin_map(&runtime_location.host_path);

    let mut plugins = Vec::new();
    for marketplace in list_marketplace_records(&runtime_location, &additional_marketplace_paths) {
        for plugin in marketplace.plugins {
            let installed = installed_map.contains_key(&plugin.plugin_id);
            plugins.push(CodexMarketplacePlugin {
                plugin_id: plugin.plugin_id.clone(),
                marketplace_name: plugin.marketplace_name,
                marketplace_path: plugin.marketplace_path.to_string_lossy().to_string(),
                name: plugin.plugin_name,
                display_name: plugin.display_name,
                description: plugin.description,
                category: plugin.category,
                capabilities: plugin.capabilities,
                source_path: plugin
                    .source_path
                    .map(|path| path.to_string_lossy().to_string()),
                installed,
                enabled: enabled_map
                    .get(&plugin.plugin_id)
                    .copied()
                    .unwrap_or(installed),
                install_available: plugin.install_available,
            });
        }
    }
    plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    Ok(plugins)
}

pub async fn list_codex_installed_plugins(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<CodexInstalledPlugin>, String> {
    let runtime_location = runtime_location::get_codex_runtime_location_async(db).await?;
    let config_path = runtime_location.host_path.join("config.toml");
    let enabled_map = plugin_toml::read_plugin_enabled_map(&config_path)?;
    let installed_map = build_installed_plugin_map(&runtime_location.host_path);
    let marketplace_plugins = list_codex_marketplace_plugins(db).await?;
    let marketplace_plugin_map: HashMap<String, CodexMarketplacePlugin> = marketplace_plugins
        .into_iter()
        .map(|plugin| (plugin.plugin_id.clone(), plugin))
        .collect();

    let mut installed_plugins = Vec::new();
    for (plugin_id, (active_version, installed_root)) in installed_map {
        let marketplace_plugin = marketplace_plugin_map.get(&plugin_id);
        let manifest = read_plugin_manifest(&installed_root);
        let display_name = marketplace_plugin
            .and_then(|plugin| plugin.display_name.clone())
            .or_else(|| {
                manifest
                    .as_ref()
                    .and_then(|item| item.interface.as_ref())
                    .and_then(|item| item.display_name.clone())
            });
        let description = marketplace_plugin
            .and_then(|plugin| plugin.description.clone())
            .or_else(|| manifest.as_ref().and_then(|item| item.description.clone()));
        let category = marketplace_plugin
            .and_then(|plugin| plugin.category.clone())
            .or_else(|| {
                manifest
                    .as_ref()
                    .and_then(|item| item.interface.as_ref())
                    .and_then(|item| item.category.clone())
            });
        let capabilities = marketplace_plugin
            .map(|plugin| plugin.capabilities.clone())
            .or_else(|| {
                manifest
                    .as_ref()
                    .and_then(|item| item.interface.as_ref())
                    .map(|item| item.capabilities.clone())
            })
            .unwrap_or_default();
        let plugin_name = manifest
            .as_ref()
            .map(|item| item.name.clone())
            .filter(|name| !name.trim().is_empty())
            .or_else(|| marketplace_plugin.map(|plugin| plugin.name.clone()))
            .unwrap_or_else(|| plugin_id.clone());
        let marketplace_name = plugin_id
            .rsplit_once('@')
            .map(|(_, marketplace_name)| marketplace_name.to_string())
            .unwrap_or_default();

        installed_plugins.push(CodexInstalledPlugin {
            plugin_id: plugin_id.clone(),
            marketplace_name,
            name: plugin_name,
            display_name,
            description,
            category,
            installed_path: Some(installed_root.to_string_lossy().to_string()),
            active_version: Some(active_version),
            enabled: enabled_map.get(&plugin_id).copied().unwrap_or(true),
            has_skills: manifest
                .as_ref()
                .and_then(|item| item.skills.as_ref())
                .is_some(),
            has_mcp_servers: manifest
                .as_ref()
                .and_then(|item| item.mcp_servers.as_ref())
                .is_some(),
            has_apps: manifest
                .as_ref()
                .and_then(|item| item.apps.as_ref())
                .is_some(),
            capabilities,
        });
    }

    installed_plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    Ok(installed_plugins)
}
