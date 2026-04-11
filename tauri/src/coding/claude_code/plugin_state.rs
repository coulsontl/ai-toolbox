use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::plugin_types::{
    ClaudeInstalledPlugin, ClaudeKnownMarketplace, ClaudeMarketplaceOwner, ClaudeMarketplacePlugin,
    ClaudePluginRuntimeStatus,
};
use crate::coding::runtime_location::{self, RuntimeLocationInfo, RuntimeLocationMode};

#[derive(Debug, Deserialize, Default)]
struct InstalledPluginsFile {
    #[serde(default)]
    plugins: HashMap<String, Vec<InstalledPluginEntry>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct InstalledPluginEntry {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    install_path: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct KnownMarketplaceEntry {
    #[serde(default)]
    source: Value,
    #[serde(default)]
    install_location: Option<String>,
    #[serde(default)]
    last_updated: Option<String>,
    #[serde(default)]
    auto_update_enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct MarketplaceManifest {
    #[serde(default)]
    owner: Option<MarketplaceOwnerEntry>,
    #[serde(default)]
    metadata: Option<MarketplaceMetadataEntry>,
    #[serde(default)]
    plugins: Vec<MarketplacePluginEntry>,
}

#[derive(Debug, Deserialize, Default)]
struct MarketplaceOwnerEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct MarketplaceMetadataEntry {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct MarketplacePluginEntry {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    source: Value,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PluginManifest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    hooks: Option<Value>,
    #[serde(default)]
    mcp_servers: Option<Value>,
    #[serde(default)]
    lsp_servers: Option<Value>,
    #[serde(default)]
    agents: Option<Value>,
}

fn read_json_file_or_default<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> Deserialize<'de> + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }

    let raw_content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?;

    serde_json::from_str(&raw_content)
        .map_err(|error| format!("Failed to parse {}: {}", path.display(), error))
}

fn claude_plugins_root(root_dir: &Path) -> PathBuf {
    root_dir.join("plugins")
}

fn installed_plugins_path(root_dir: &Path) -> PathBuf {
    claude_plugins_root(root_dir).join("installed_plugins.json")
}

fn known_marketplaces_path(root_dir: &Path) -> PathBuf {
    claude_plugins_root(root_dir).join("known_marketplaces.json")
}

fn write_json_file_pretty<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: serde::Serialize,
{
    if let Some(parent_dir) = path.parent() {
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)
                .map_err(|error| format!("Failed to create {}: {}", parent_dir.display(), error))?;
        }
    }

    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| format!("Failed to serialize {}: {}", path.display(), error))?;
    fs::write(path, format!("{serialized}\n"))
        .map_err(|error| format!("Failed to write {}: {}", path.display(), error))
}

fn plugin_manifest_path(install_path: &Path) -> PathBuf {
    install_path.join(".claude-plugin").join("plugin.json")
}

fn parse_plugin_id(plugin_id: &str) -> (String, String) {
    match plugin_id.rsplit_once('@') {
        Some((name, marketplace_name)) => (name.to_string(), marketplace_name.to_string()),
        None => (plugin_id.to_string(), String::new()),
    }
}

fn has_non_empty_value(value: &Option<Value>) -> bool {
    match value {
        Some(Value::Null) | None => false,
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

fn dir_exists(path: &Path) -> bool {
    path.exists() && path.is_dir()
}

fn resolve_runtime_storage_path(runtime_location: &RuntimeLocationInfo, raw_path: &str) -> PathBuf {
    let trimmed_path = raw_path.trim();
    if let Some(wsl) = runtime_location.wsl.as_ref() {
        let expanded_path = runtime_location::expand_home_from_user_root(
            wsl.linux_user_root.as_deref(),
            trimmed_path,
        );
        if expanded_path.starts_with('/') {
            return runtime_location::build_windows_unc_path(&wsl.distro, &expanded_path);
        }
    }

    PathBuf::from(trimmed_path)
}

pub async fn get_claude_plugin_runtime_status(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<ClaudePluginRuntimeStatus, String> {
    let runtime_location = runtime_location::get_claude_runtime_location_async(db).await?;
    let mode = match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => "local".to_string(),
        RuntimeLocationMode::WslDirect => "wslDirect".to_string(),
    };

    let distro = runtime_location
        .wsl
        .as_ref()
        .map(|item| item.distro.clone());
    let linux_root_dir = runtime_location
        .wsl
        .as_ref()
        .map(|item| item.linux_path.clone());
    let plugins_dir = claude_plugins_root(&runtime_location.host_path);

    Ok(ClaudePluginRuntimeStatus {
        mode,
        source: runtime_location.source,
        root_dir: runtime_location.host_path.to_string_lossy().to_string(),
        settings_path: runtime_location
            .host_path
            .join("settings.json")
            .to_string_lossy()
            .to_string(),
        plugins_dir: plugins_dir.to_string_lossy().to_string(),
        distro,
        linux_root_dir,
    })
}

pub async fn list_claude_known_marketplaces(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Vec<ClaudeKnownMarketplace>, String> {
    let runtime_location = runtime_location::get_claude_runtime_location_async(db).await?;
    let marketplaces_file: HashMap<String, KnownMarketplaceEntry> =
        read_json_file_or_default(&known_marketplaces_path(&runtime_location.host_path))?;

    let mut marketplaces = Vec::new();

    for (marketplace_name, marketplace_entry) in marketplaces_file {
        let manifest = marketplace_entry
            .install_location
            .as_deref()
            .map(|install_location| {
                resolve_runtime_storage_path(&runtime_location, install_location)
            })
            .map(|install_location| {
                install_location
                    .join(".claude-plugin")
                    .join("marketplace.json")
            })
            .filter(|path| path.exists())
            .map(|path| read_json_file_or_default::<MarketplaceManifest>(&path))
            .transpose()?
            .unwrap_or_default();
        let install_location = marketplace_entry
            .install_location
            .as_deref()
            .map(|location| resolve_runtime_storage_path(&runtime_location, location))
            .map(|location| location.to_string_lossy().to_string());

        marketplaces.push(ClaudeKnownMarketplace {
            name: marketplace_name,
            source: marketplace_entry.source,
            install_location,
            last_updated: marketplace_entry.last_updated,
            auto_update_enabled: marketplace_entry.auto_update_enabled.unwrap_or(false),
            owner: manifest.owner.map(|owner| ClaudeMarketplaceOwner {
                name: owner.name,
                email: owner.email,
            }),
            description: manifest
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.description.clone()),
            version: manifest.metadata.and_then(|metadata| metadata.version),
            plugin_count: manifest.plugins.len(),
        });
    }

    marketplaces.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(marketplaces)
}

/// Backup auto_update_enabled settings for all known marketplaces.
/// Used to preserve these settings before calling external CLI commands
/// that may rewrite known_marketplaces.json without this field.
pub fn backup_marketplace_auto_update_settings(
    host_path: &Path,
) -> Result<HashMap<String, bool>, String> {
    let known_marketplaces_file_path = known_marketplaces_path(host_path);
    let marketplaces_file: HashMap<String, KnownMarketplaceEntry> =
        read_json_file_or_default(&known_marketplaces_file_path)?;

    let mut settings = HashMap::new();
    for (name, entry) in &marketplaces_file {
        if let Some(enabled) = entry.auto_update_enabled {
            settings.insert(name.clone(), enabled);
        }
    }
    Ok(settings)
}

/// Restore auto_update_enabled settings after external CLI commands
/// have potentially rewritten known_marketplaces.json.
pub fn restore_marketplace_auto_update_settings(
    host_path: &Path,
    settings: &HashMap<String, bool>,
) -> Result<(), String> {
    if settings.is_empty() {
        return Ok(());
    }
    let known_marketplaces_file_path = known_marketplaces_path(host_path);
    let mut marketplaces_file: HashMap<String, KnownMarketplaceEntry> =
        read_json_file_or_default(&known_marketplaces_file_path)?;

    let mut changed = false;
    for (name, enabled) in settings {
        if let Some(entry) = marketplaces_file.get_mut(name) {
            if entry.auto_update_enabled != Some(*enabled) {
                entry.auto_update_enabled = Some(*enabled);
                changed = true;
            }
        }
    }

    if changed {
        write_json_file_pretty(&known_marketplaces_file_path, &marketplaces_file)?;
    }
    Ok(())
}

pub async fn set_claude_marketplace_auto_update_enabled(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    marketplace_name: &str,
    auto_update_enabled: bool,
) -> Result<(), String> {
    let runtime_location = runtime_location::get_claude_runtime_location_async(db).await?;
    let known_marketplaces_file_path = known_marketplaces_path(&runtime_location.host_path);
    let mut marketplaces_file: HashMap<String, KnownMarketplaceEntry> =
        read_json_file_or_default(&known_marketplaces_file_path)?;

    let marketplace_entry = marketplaces_file
        .get_mut(marketplace_name)
        .ok_or_else(|| format!("Marketplace not found: {}", marketplace_name))?;
    marketplace_entry.auto_update_enabled = Some(auto_update_enabled);

    write_json_file_pretty(&known_marketplaces_file_path, &marketplaces_file)
}

pub async fn list_claude_marketplace_plugins(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Vec<ClaudeMarketplacePlugin>, String> {
    let runtime_location = runtime_location::get_claude_runtime_location_async(db).await?;
    let marketplaces_file: HashMap<String, KnownMarketplaceEntry> =
        read_json_file_or_default(&known_marketplaces_path(&runtime_location.host_path))?;

    let mut plugins = Vec::new();

    for (marketplace_name, marketplace_entry) in marketplaces_file {
        let Some(install_location) = marketplace_entry.install_location.as_deref() else {
            continue;
        };

        let manifest_path = resolve_runtime_storage_path(&runtime_location, install_location)
            .join(".claude-plugin")
            .join("marketplace.json");
        if !manifest_path.exists() {
            continue;
        }

        let manifest: MarketplaceManifest = read_json_file_or_default(&manifest_path)?;
        for plugin_entry in manifest.plugins {
            plugins.push(ClaudeMarketplacePlugin {
                marketplace_name: marketplace_name.clone(),
                plugin_id: format!("{}@{}", plugin_entry.name, marketplace_name),
                name: plugin_entry.name,
                description: plugin_entry.description,
                version: plugin_entry.version,
                homepage: plugin_entry.homepage,
                repository: plugin_entry.repository,
                category: plugin_entry.category,
                tags: plugin_entry.tags,
                source: plugin_entry.source,
            });
        }
    }

    plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    Ok(plugins)
}

pub async fn list_claude_installed_plugins(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Vec<ClaudeInstalledPlugin>, String> {
    let runtime_location = runtime_location::get_claude_runtime_location_async(db).await?;
    let installed_plugins: InstalledPluginsFile =
        read_json_file_or_default(&installed_plugins_path(&runtime_location.host_path))?;
    let known_marketplaces = list_claude_marketplace_plugins(db).await?;
    let marketplace_plugin_map: HashMap<String, ClaudeMarketplacePlugin> = known_marketplaces
        .into_iter()
        .map(|plugin| (plugin.plugin_id.clone(), plugin))
        .collect();

    let settings_path = runtime_location.host_path.join("settings.json");
    let settings_value = if settings_path.exists() {
        read_json_file_or_default::<Value>(&settings_path)?
    } else {
        Value::Object(serde_json::Map::new())
    };
    let enabled_plugins = settings_value
        .as_object()
        .and_then(|object| object.get("enabledPlugins"))
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();

    let mut plugin_statuses = Vec::new();

    for (plugin_id, install_entries) in installed_plugins.plugins {
        let (plugin_name, marketplace_name) = parse_plugin_id(&plugin_id);
        let metadata = marketplace_plugin_map.get(&plugin_id);
        let first_install_entry = install_entries.first();
        let install_path = first_install_entry
            .and_then(|entry| entry.install_path.as_deref())
            .map(|install_path| resolve_runtime_storage_path(&runtime_location, install_path));
        let manifest = install_path
            .as_ref()
            .map(|path| plugin_manifest_path(path))
            .filter(|path| path.exists())
            .map(|path| read_json_file_or_default::<PluginManifest>(&path))
            .transpose()?
            .unwrap_or_default();

        let install_scopes: Vec<String> = install_entries
            .iter()
            .filter_map(|entry| entry.scope.clone())
            .collect();
        let user_scope_installed = install_entries
            .iter()
            .any(|entry| entry.scope.as_deref() == Some("user"));
        let user_scope_enabled = enabled_plugins
            .get(&plugin_id)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let install_path_string = install_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        let install_root_path = install_path.as_deref().unwrap_or_else(|| Path::new(""));

        plugin_statuses.push(ClaudeInstalledPlugin {
            plugin_id: plugin_id.clone(),
            name: metadata
                .map(|plugin| plugin.name.clone())
                .or(manifest.name)
                .unwrap_or(plugin_name),
            marketplace_name,
            description: metadata
                .and_then(|plugin| plugin.description.clone())
                .or(manifest.description),
            version: first_install_entry
                .and_then(|entry| entry.version.clone())
                .or_else(|| metadata.and_then(|plugin| plugin.version.clone()))
                .or(manifest.version),
            homepage: metadata
                .and_then(|plugin| plugin.homepage.clone())
                .or(manifest.homepage),
            repository: metadata
                .and_then(|plugin| plugin.repository.clone())
                .or(manifest.repository),
            install_path: install_path_string,
            user_scope_installed,
            user_scope_enabled,
            install_scopes,
            has_skills: dir_exists(&install_root_path.join("skills")),
            has_agents: dir_exists(&install_root_path.join("agents"))
                || has_non_empty_value(&manifest.agents),
            has_hooks: dir_exists(&install_root_path.join("hooks"))
                || has_non_empty_value(&manifest.hooks),
            has_mcp_servers: install_root_path.join(".mcp.json").exists()
                || has_non_empty_value(&manifest.mcp_servers),
            has_lsp_servers: install_root_path.join(".lsp.json").exists()
                || has_non_empty_value(&manifest.lsp_servers),
        });
    }

    plugin_statuses.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    Ok(plugin_statuses)
}
