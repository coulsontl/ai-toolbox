use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::plugin_state;
use super::plugin_toml;
use crate::coding::skills::sync_engine::copy_dir_recursive;

const CURATED_MARKETPLACE_NAME: &str = "openai-curated";
const DEFAULT_PLUGIN_VERSION: &str = "local";

fn parse_plugin_id(plugin_id: &str) -> Result<(String, String), String> {
    let Some((plugin_name, marketplace_name)) = plugin_id.rsplit_once('@') else {
        return Err(format!(
            "Invalid plugin id `{}`; expected <plugin>@<marketplace>",
            plugin_id
        ));
    };

    if plugin_name.trim().is_empty() || marketplace_name.trim().is_empty() {
        return Err(format!(
            "Invalid plugin id `{}`; expected <plugin>@<marketplace>",
            plugin_id
        ));
    }

    Ok((plugin_name.to_string(), marketplace_name.to_string()))
}

fn plugins_cache_root(root_dir: &Path) -> PathBuf {
    root_dir.join("plugins").join("cache")
}

fn plugin_base_dir(root_dir: &Path, marketplace_name: &str, plugin_name: &str) -> PathBuf {
    plugins_cache_root(root_dir)
        .join(marketplace_name)
        .join(plugin_name)
}

fn plugin_version_dir(
    root_dir: &Path,
    marketplace_name: &str,
    plugin_name: &str,
    version: &str,
) -> PathBuf {
    plugin_base_dir(root_dir, marketplace_name, plugin_name).join(version)
}

fn curated_sha_path(root_dir: &Path) -> PathBuf {
    root_dir.join(".tmp").join("plugins.sha")
}

fn read_curated_sha(root_dir: &Path) -> Option<String> {
    fs::read_to_string(curated_sha_path(root_dir))
        .ok()
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
}

fn remove_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("Failed to remove {}: {}", path.display(), error))
    } else {
        fs::remove_file(path)
            .map_err(|error| format!("Failed to remove {}: {}", path.display(), error))
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawPluginManifest {
    #[serde(default)]
    name: String,
}

fn read_plugin_manifest_name(plugin_root: &Path) -> Result<String, String> {
    let manifest_path = plugin_root.join(".codex-plugin").join("plugin.json");
    let raw_content = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("Failed to read {}: {}", manifest_path.display(), error))?;
    let manifest = serde_json::from_str::<RawPluginManifest>(&raw_content)
        .map_err(|error| format!("Failed to parse {}: {}", manifest_path.display(), error))?;
    let plugin_name = manifest.name.trim().to_string();
    if plugin_name.is_empty() {
        return Err(format!(
            "Plugin manifest name is missing: {}",
            manifest_path.display()
        ));
    }
    Ok(plugin_name)
}

fn install_plugin_atomically(source_path: &Path, target_dir: &Path) -> Result<(), String> {
    let Some(plugin_root_dir) = target_dir.parent() else {
        return Err(format!(
            "Invalid plugin target directory: {}",
            target_dir.display()
        ));
    };
    let Some(cache_root_dir) = plugin_root_dir.parent() else {
        return Err(format!(
            "Invalid plugin cache root for {}",
            target_dir.display()
        ));
    };
    let plugin_dir_name = plugin_root_dir
        .file_name()
        .ok_or_else(|| {
            format!(
                "Invalid plugin directory name: {}",
                plugin_root_dir.display()
            )
        })?
        .to_os_string();
    let version_dir_name = target_dir
        .file_name()
        .ok_or_else(|| format!("Invalid plugin version directory: {}", target_dir.display()))?
        .to_os_string();

    fs::create_dir_all(cache_root_dir)
        .map_err(|error| format!("Failed to create {}: {}", cache_root_dir.display(), error))?;

    let staging_root = tempfile::Builder::new()
        .prefix("codex-plugin-install-")
        .tempdir_in(cache_root_dir)
        .map_err(|error| {
            format!(
                "Failed to create temporary plugin cache directory in {}: {}",
                cache_root_dir.display(),
                error
            )
        })?;
    let staged_plugin_root = staging_root.path().join(&plugin_dir_name);
    let staged_target_dir = staged_plugin_root.join(&version_dir_name);
    copy_dir_recursive(source_path, &staged_target_dir)
        .map_err(|error| format!("Failed to stage plugin files: {}", error))?;

    if plugin_root_dir.exists() {
        let backup_root = tempfile::Builder::new()
            .prefix("codex-plugin-backup-")
            .tempdir_in(cache_root_dir)
            .map_err(|error| {
                format!(
                    "Failed to create temporary plugin backup directory in {}: {}",
                    cache_root_dir.display(),
                    error
                )
            })?;
        let backup_plugin_root = backup_root.path().join(&plugin_dir_name);
        fs::rename(plugin_root_dir, &backup_plugin_root).map_err(|error| {
            format!(
                "Failed to back up existing plugin directory {}: {}",
                plugin_root_dir.display(),
                error
            )
        })?;

        if let Err(error) = fs::rename(&staged_plugin_root, plugin_root_dir) {
            let rollback_error = fs::rename(&backup_plugin_root, plugin_root_dir).err();
            return match rollback_error {
                Some(rollback_error) => Err(format!(
                    "Failed to activate plugin directory {}: {}; rollback also failed: {}",
                    plugin_root_dir.display(),
                    error,
                    rollback_error
                )),
                None => Err(format!(
                    "Failed to activate plugin directory {}: {}",
                    plugin_root_dir.display(),
                    error
                )),
            };
        }
    } else {
        fs::rename(&staged_plugin_root, plugin_root_dir).map_err(|error| {
            format!(
                "Failed to activate plugin directory {}: {}",
                plugin_root_dir.display(),
                error
            )
        })?;
    }

    Ok(())
}

pub async fn ensure_codex_plugins_feature_enabled(
    db: &crate::db::SqliteDbState,
) -> Result<(), String> {
    let config_path = crate::coding::runtime_location::get_codex_config_path_async(db).await?;
    plugin_toml::set_plugins_feature_enabled(&config_path, true)
}

pub async fn install_codex_plugin(
    db: &crate::db::SqliteDbState,
    plugin_id: &str,
) -> Result<(), String> {
    let runtime_location =
        crate::coding::runtime_location::get_codex_runtime_location_async(db).await?;
    let (plugin_name, marketplace_name) = parse_plugin_id(plugin_id)?;
    let marketplace_plugins = plugin_state::list_codex_marketplace_plugins(db).await?;
    let marketplace_plugin = marketplace_plugins
        .into_iter()
        .find(|plugin| plugin.plugin_id == plugin_id)
        .ok_or_else(|| format!("Plugin not found in marketplaces: {}", plugin_id))?;

    if !marketplace_plugin.install_available {
        return Err(format!(
            "Plugin is not available for install: {}",
            plugin_id
        ));
    }

    let source_path = marketplace_plugin
        .source_path
        .map(PathBuf::from)
        .ok_or_else(|| format!("Plugin source path is missing: {}", plugin_id))?;
    if !source_path.is_dir() {
        return Err(format!(
            "Plugin source path does not exist: {}",
            source_path.display()
        ));
    }
    let manifest_plugin_name = read_plugin_manifest_name(&source_path)?;
    if manifest_plugin_name != plugin_name {
        return Err(format!(
            "Plugin manifest name `{}` does not match marketplace plugin name `{}`",
            manifest_plugin_name, plugin_name
        ));
    }

    let version = if marketplace_name == CURATED_MARKETPLACE_NAME {
        read_curated_sha(&runtime_location.host_path)
            .unwrap_or_else(|| DEFAULT_PLUGIN_VERSION.to_string())
    } else {
        DEFAULT_PLUGIN_VERSION.to_string()
    };
    let target_dir = plugin_version_dir(
        &runtime_location.host_path,
        &marketplace_name,
        &plugin_name,
        &version,
    );

    if let Some(parent_dir) = target_dir.parent() {
        fs::create_dir_all(parent_dir)
            .map_err(|error| format!("Failed to create {}: {}", parent_dir.display(), error))?;
    }

    install_plugin_atomically(&source_path, &target_dir)
        .map_err(|error| format!("Failed to install plugin {}: {}", plugin_id, error))?;

    let config_path = runtime_location.host_path.join("config.toml");
    plugin_toml::set_plugins_feature_enabled(&config_path, true)?;
    plugin_toml::set_plugin_enabled(&config_path, plugin_id, true)?;
    Ok(())
}

pub async fn uninstall_codex_plugin(
    db: &crate::db::SqliteDbState,
    plugin_id: &str,
) -> Result<(), String> {
    let runtime_location =
        crate::coding::runtime_location::get_codex_runtime_location_async(db).await?;
    let (plugin_name, marketplace_name) = parse_plugin_id(plugin_id)?;
    let plugin_dir = plugin_base_dir(&runtime_location.host_path, &marketplace_name, &plugin_name);
    remove_path(&plugin_dir)?;

    let config_path = runtime_location.host_path.join("config.toml");
    plugin_toml::remove_plugin_entry(&config_path, plugin_id)?;
    Ok(())
}

pub async fn set_codex_plugin_enabled(
    db: &crate::db::SqliteDbState,
    plugin_id: &str,
    enabled: bool,
) -> Result<(), String> {
    let runtime_location =
        crate::coding::runtime_location::get_codex_runtime_location_async(db).await?;
    let config_path = runtime_location.host_path.join("config.toml");
    if enabled {
        plugin_toml::set_plugins_feature_enabled(&config_path, true)?;
    }
    plugin_toml::set_plugin_enabled(&config_path, plugin_id, enabled)
}
