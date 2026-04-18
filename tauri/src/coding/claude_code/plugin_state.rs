use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use tokio::sync::Mutex;

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

static MARKETPLACE_AUTO_UPDATE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

fn write_json_value_atomic(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent_dir) = path.parent() {
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)
                .map_err(|error| format!("Failed to create {}: {}", parent_dir.display(), error))?;
        }

        let mut temp_file = NamedTempFile::new_in(parent_dir).map_err(|error| {
            format!(
                "Failed to create temp file for {}: {}",
                path.display(),
                error
            )
        })?;
        serde_json::to_writer_pretty(temp_file.as_file_mut(), value)
            .map_err(|error| format!("Failed to serialize {}: {}", path.display(), error))?;
        use std::io::Write;
        temp_file
            .as_file_mut()
            .write_all(b"\n")
            .map_err(|error| format!("Failed to finalize {}: {}", path.display(), error))?;
        temp_file
            .persist(path)
            .map_err(|error| format!("Failed to replace {}: {}", path.display(), error.error))?;
        return Ok(());
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

fn marketplace_file_as_object(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let raw_content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?;
    match serde_json::from_str::<Value>(&raw_content)
        .map_err(|error| format!("Failed to parse {}: {}", path.display(), error))?
    {
        Value::Object(object) => Ok(object),
        _ => Err(format!(
            "Expected {} to contain a JSON object",
            path.display()
        )),
    }
}

fn get_marketplace_auto_update_lock() -> &'static Mutex<()> {
    MARKETPLACE_AUTO_UPDATE_LOCK.get_or_init(|| Mutex::new(()))
}

fn extract_marketplace_auto_update_settings_from_object(
    marketplaces_file: &Map<String, Value>,
) -> HashMap<String, bool> {
    let mut settings = HashMap::new();

    for (name, entry) in marketplaces_file {
        let Some(entry_object) = entry.as_object() else {
            continue;
        };
        let Some(enabled) = entry_object
            .get("autoUpdateEnabled")
            .and_then(Value::as_bool)
        else {
            continue;
        };
        settings.insert(name.clone(), enabled);
    }

    settings
}

fn merge_marketplace_auto_update_settings_into_object(
    marketplaces_file: &mut Map<String, Value>,
    settings: &HashMap<String, bool>,
) -> bool {
    let mut changed = false;

    for (name, enabled) in settings {
        let Some(entry_value) = marketplaces_file.get_mut(name) else {
            continue;
        };
        let Some(entry_object) = entry_value.as_object_mut() else {
            continue;
        };
        let current_enabled = entry_object
            .get("autoUpdateEnabled")
            .and_then(Value::as_bool);
        if current_enabled == Some(*enabled) {
            continue;
        }
        entry_object.insert("autoUpdateEnabled".to_string(), Value::Bool(*enabled));
        changed = true;
    }

    changed
}

fn update_marketplace_auto_update_in_object(
    marketplaces_file: &mut Map<String, Value>,
    marketplace_name: &str,
    auto_update_enabled: bool,
) -> Result<bool, String> {
    let entry_value = marketplaces_file
        .get_mut(marketplace_name)
        .ok_or_else(|| format!("Marketplace not found: {}", marketplace_name))?;
    let entry_object = entry_value.as_object_mut().ok_or_else(|| {
        format!(
            "Marketplace entry is not a JSON object: {}",
            marketplace_name
        )
    })?;
    let current_enabled = entry_object
        .get("autoUpdateEnabled")
        .and_then(Value::as_bool);
    if current_enabled == Some(auto_update_enabled) {
        return Ok(false);
    }
    entry_object.insert(
        "autoUpdateEnabled".to_string(),
        Value::Bool(auto_update_enabled),
    );
    Ok(true)
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

pub async fn run_claude_marketplace_command_preserving_auto_update<F, Fut>(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    marketplace_command: F,
) -> Result<(), String>
where
    F: FnOnce(RuntimeLocationInfo) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let _lock = get_marketplace_auto_update_lock().lock().await;
    let runtime_location = runtime_location::get_claude_runtime_location_async(db).await?;
    let known_marketplaces_file_path = known_marketplaces_path(&runtime_location.host_path);
    let before_marketplaces = marketplace_file_as_object(&known_marketplaces_file_path)?;
    let auto_update_settings =
        extract_marketplace_auto_update_settings_from_object(&before_marketplaces);

    marketplace_command(runtime_location.clone()).await?;

    if auto_update_settings.is_empty() {
        return Ok(());
    }

    let mut after_marketplaces = marketplace_file_as_object(&known_marketplaces_file_path)?;
    if !merge_marketplace_auto_update_settings_into_object(
        &mut after_marketplaces,
        &auto_update_settings,
    ) {
        return Ok(());
    }

    write_json_value_atomic(
        &known_marketplaces_file_path,
        &Value::Object(after_marketplaces),
    )
}

pub async fn set_claude_marketplace_auto_update_enabled(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    marketplace_name: &str,
    auto_update_enabled: bool,
) -> Result<(), String> {
    let _lock = get_marketplace_auto_update_lock().lock().await;
    let runtime_location = runtime_location::get_claude_runtime_location_async(db).await?;
    let known_marketplaces_file_path = known_marketplaces_path(&runtime_location.host_path);
    let mut marketplaces_file = marketplace_file_as_object(&known_marketplaces_file_path)?;
    if !update_marketplace_auto_update_in_object(
        &mut marketplaces_file,
        marketplace_name,
        auto_update_enabled,
    )? {
        return Ok(());
    }

    write_json_value_atomic(
        &known_marketplaces_file_path,
        &Value::Object(marketplaces_file),
    )
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

#[cfg(test)]
mod tests {
    use super::{
        extract_marketplace_auto_update_settings_from_object,
        merge_marketplace_auto_update_settings_into_object,
        update_marketplace_auto_update_in_object,
    };
    use serde_json::{json, Value};

    #[test]
    fn merge_marketplace_auto_update_preserves_runtime_owned_fields() {
        let before_marketplaces = json!({
            "alpha": {
                "source": { "type": "git", "url": "https://example.com/a" },
                "installLocation": "/tmp/a",
                "autoUpdateEnabled": true
            }
        });
        let mut after_marketplaces = serde_json::from_value(json!({
            "alpha": {
                "source": { "type": "git", "url": "https://example.com/a" },
                "installLocation": "/tmp/a",
                "lastUpdated": "2026-04-11T00:00:00Z",
                "cliOwnedField": {
                    "etag": "v2",
                    "signature": "keep-me"
                }
            },
            "beta": {
                "source": { "type": "git", "url": "https://example.com/b" },
                "installLocation": "/tmp/b"
            }
        }))
        .unwrap();

        let auto_update_settings = extract_marketplace_auto_update_settings_from_object(
            before_marketplaces.as_object().unwrap(),
        );
        let changed = merge_marketplace_auto_update_settings_into_object(
            &mut after_marketplaces,
            &auto_update_settings,
        );

        assert!(changed);
        let alpha_entry = after_marketplaces
            .get("alpha")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(
            alpha_entry
                .get("autoUpdateEnabled")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            alpha_entry
                .get("cliOwnedField")
                .and_then(Value::as_object)
                .and_then(|field_object| field_object.get("signature"))
                .and_then(Value::as_str),
            Some("keep-me")
        );
        let beta_entry = after_marketplaces
            .get("beta")
            .and_then(Value::as_object)
            .unwrap();
        assert!(!beta_entry.contains_key("autoUpdateEnabled"));
    }

    #[test]
    fn update_marketplace_auto_update_only_touches_target_field() {
        let mut marketplaces = serde_json::from_value(json!({
            "alpha": {
                "source": { "type": "git", "url": "https://example.com/a" },
                "installLocation": "/tmp/a",
                "cliOwnedField": {
                    "etag": "v1"
                }
            }
        }))
        .unwrap();

        let changed =
            update_marketplace_auto_update_in_object(&mut marketplaces, "alpha", true).unwrap();

        assert!(changed);
        let alpha_entry = marketplaces
            .get("alpha")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(
            alpha_entry
                .get("autoUpdateEnabled")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            alpha_entry
                .get("cliOwnedField")
                .and_then(Value::as_object)
                .and_then(|field_object| field_object.get("etag"))
                .and_then(Value::as_str),
            Some("v1")
        );
    }
}
