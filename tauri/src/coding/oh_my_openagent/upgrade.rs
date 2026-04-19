use std::fs;
use std::path::{Path, PathBuf};

use tauri::Emitter;

use crate::coding::open_code;
use crate::coding::runtime_location;
use crate::coding::ssh;
use crate::coding::wsl;
use crate::db::DbState;

use super::types::{OhMyOpenAgentLegacyUpgradeResult, OhMyOpenAgentLegacyUpgradeStatus};

const OMO_CANONICAL_PLUGIN: &str = "oh-my-openagent";
const OMO_LEGACY_PLUGIN: &str = "oh-my-opencode";
const OMO_CANONICAL_JSONC: &str = "oh-my-openagent.jsonc";
const OMO_CANONICAL_JSON: &str = "oh-my-openagent.json";
const OMO_LEGACY_JSONC: &str = "oh-my-opencode.jsonc";
const OMO_LEGACY_JSON: &str = "oh-my-opencode.json";
const OMO_MAPPING_ID: &str = "opencode-oh-my";

#[derive(Debug, Clone, Copy)]
struct LegacyFileName {
    canonical_name: &'static str,
}

fn legacy_file_name(file_name: &str) -> Option<LegacyFileName> {
    match file_name {
        OMO_LEGACY_JSONC => Some(LegacyFileName {
            canonical_name: OMO_CANONICAL_JSONC,
        }),
        OMO_LEGACY_JSON => Some(LegacyFileName {
            canonical_name: OMO_CANONICAL_JSON,
        }),
        _ => None,
    }
}

fn normalize_omo_plugin_name(plugin_name: &str) -> String {
    if plugin_name == OMO_LEGACY_PLUGIN {
        return OMO_CANONICAL_PLUGIN.to_string();
    }

    if let Some(version_suffix) = plugin_name.strip_prefix("oh-my-opencode@") {
        return format!("oh-my-openagent@{}", version_suffix);
    }

    plugin_name.to_string()
}

fn normalize_omo_plugin_entry(
    plugin_entry: &open_code::types::OpenCodePluginEntry,
) -> open_code::types::OpenCodePluginEntry {
    match plugin_entry {
        open_code::types::OpenCodePluginEntry::Name(plugin_name) => {
            open_code::types::OpenCodePluginEntry::Name(normalize_omo_plugin_name(plugin_name))
        }
        open_code::types::OpenCodePluginEntry::NameWithOptions((plugin_name, plugin_options)) => {
            open_code::types::OpenCodePluginEntry::NameWithOptions((
                normalize_omo_plugin_name(plugin_name),
                plugin_options.clone(),
            ))
        }
    }
}

fn path_file_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

fn is_legacy_omo_path(path: &str) -> bool {
    path_file_name(path)
        .as_deref()
        .and_then(legacy_file_name)
        .is_some()
}

fn canonicalize_omo_path_string(path: &str) -> Option<String> {
    let path_buf = PathBuf::from(path);
    let file_name = path_buf.file_name()?.to_str()?;
    let legacy_name = legacy_file_name(file_name)?;
    Some(
        path_buf
            .with_file_name(legacy_name.canonical_name)
            .to_string_lossy()
            .to_string(),
    )
}

fn canonicalize_mapping_path(path: &str) -> Option<String> {
    if path.contains(OMO_LEGACY_JSONC) {
        return Some(path.replace(OMO_LEGACY_JSONC, OMO_CANONICAL_JSONC));
    }

    if path.contains(OMO_LEGACY_JSON) {
        return Some(path.replace(OMO_LEGACY_JSON, OMO_CANONICAL_JSON));
    }

    None
}

fn rename_local_file_atomic(from_path: &Path, to_path: &Path) -> Result<(), String> {
    if !from_path.exists() {
        return Ok(());
    }

    if to_path.exists() {
        return Err(format!(
            "Canonical Oh My OpenAgent config already exists: {}",
            to_path.to_string_lossy()
        ));
    }

    fs::rename(from_path, to_path).map_err(|e| {
        format!(
            "Failed to rename local Oh My OpenAgent config from {} to {}: {}",
            from_path.to_string_lossy(),
            to_path.to_string_lossy(),
            e
        )
    })
}

#[cfg(target_os = "windows")]
fn rename_wsl_file_if_needed(distro: &str, from_path: &str, to_path: &str) -> Result<bool, String> {
    if from_path == to_path {
        return Ok(false);
    }

    if !wsl::wsl_path_exists(distro, from_path) {
        return Ok(false);
    }

    if wsl::wsl_path_exists(distro, to_path) {
        return Err(format!(
            "Canonical Oh My OpenAgent config already exists in WSL: {}",
            to_path
        ));
    }

    let from_target = from_path.replace("~", "$HOME");
    let to_target = to_path.replace("~", "$HOME");
    let command = format!(
        "mkdir -p \"$(dirname \"{}\")\" && mv \"{}\" \"{}\"",
        to_target, from_target, to_target
    );

    let output = std::process::Command::new("wsl")
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to rename WSL Oh My OpenAgent config: {}", e))?;

    if output.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "Failed to rename WSL Oh My OpenAgent config: {}",
            stderr
        ))
    }
}

#[cfg(not(target_os = "windows"))]
fn rename_wsl_file_if_needed(
    _distro: &str,
    _from_path: &str,
    _to_path: &str,
) -> Result<bool, String> {
    Ok(false)
}

async fn load_opencode_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<open_code::OpenCodeConfig>, String> {
    match open_code::read_opencode_config(state).await? {
        open_code::ReadConfigResult::Success { config } => Ok(Some(config)),
        open_code::ReadConfigResult::NotFound { .. } => Ok(None),
        open_code::ReadConfigResult::ParseError { error, .. } => Err(format!(
            "Failed to upgrade OpenCode plugins because config parsing failed: {}",
            error
        )),
        open_code::ReadConfigResult::Error { error } => Err(error),
    }
}

async fn detect_legacy_plugin_usage(state: tauri::State<'_, DbState>) -> Result<bool, String> {
    Ok(load_normalized_opencode_config(state).await?.is_some())
}

async fn load_normalized_opencode_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<open_code::OpenCodeConfig>, String> {
    let Some(mut config) = load_opencode_config(state.clone()).await? else {
        return Ok(None);
    };

    let Some(plugins) = config.plugin.clone() else {
        return Ok(None);
    };

    let normalized_plugins: Vec<open_code::types::OpenCodePluginEntry> =
        plugins.iter().map(normalize_omo_plugin_entry).collect();

    if normalized_plugins == plugins {
        return Ok(None);
    }

    config.plugin = Some(normalized_plugins);
    Ok(Some(config))
}

async fn update_custom_opencode_config_path_if_needed(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle,
) -> Result<bool, String> {
    let Some(mut common_config) = open_code::get_opencode_common_config(state.clone()).await?
    else {
        return Ok(false);
    };

    let Some(config_path) = common_config.config_path.clone() else {
        return Ok(false);
    };

    let Some(canonical_path) = canonicalize_omo_path_string(&config_path) else {
        return Ok(false);
    };

    common_config.config_path = Some(canonical_path);
    open_code::save_opencode_common_config(state, app.clone(), common_config).await?;
    Ok(true)
}

async fn update_wsl_mapping_if_needed(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle,
    wsl_config: &wsl::WSLSyncConfig,
) -> Result<(bool, bool), String> {
    let Some(mapping) = wsl_config
        .file_mappings
        .iter()
        .find(|mapping| mapping.id == OMO_MAPPING_ID)
        .cloned()
    else {
        return Ok((false, false));
    };

    let new_windows_path = canonicalize_mapping_path(&mapping.windows_path);
    let new_wsl_path = canonicalize_mapping_path(&mapping.wsl_path);

    let path_updated = new_windows_path.is_some() || new_wsl_path.is_some();
    if !path_updated {
        return Ok((false, false));
    }

    let mut updated_mapping = mapping.clone();
    if let Some(path) = new_windows_path {
        updated_mapping.windows_path = path;
    }
    if let Some(path) = new_wsl_path.clone() {
        updated_mapping.wsl_path = path;
    }

    let mut wsl_file_renamed = false;
    if let Some(status) = wsl_config
        .module_statuses
        .iter()
        .find(|item| item.module == "opencode" && item.is_wsl_direct)
    {
        if let (Some(distro), Some(from_path), Some(to_path)) = (
            status.distro.as_deref(),
            new_wsl_path.as_ref().map(|_| mapping.wsl_path.as_str()),
            new_wsl_path.as_deref(),
        ) {
            wsl_file_renamed = rename_wsl_file_if_needed(distro, from_path, to_path)?;
        }
    } else if !wsl_config.distro.is_empty() {
        if let (Some(from_path), Some(to_path)) =
            (Some(mapping.wsl_path.as_str()), new_wsl_path.as_deref())
        {
            wsl_file_renamed = rename_wsl_file_if_needed(&wsl_config.distro, from_path, to_path)?;
        }
    }

    wsl::wsl_update_file_mapping(state, app.clone(), updated_mapping).await?;
    Ok((true, wsl_file_renamed))
}

async fn update_ssh_mapping_if_needed(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle,
    ssh_config: &ssh::SSHSyncConfig,
) -> Result<bool, String> {
    let Some(mapping) = ssh_config
        .file_mappings
        .iter()
        .find(|mapping| mapping.id == OMO_MAPPING_ID)
        .cloned()
    else {
        return Ok(false);
    };

    let new_local_path = canonicalize_mapping_path(&mapping.local_path);
    let new_remote_path = canonicalize_mapping_path(&mapping.remote_path);

    if new_local_path.is_none() && new_remote_path.is_none() {
        return Ok(false);
    }

    let mut updated_mapping = mapping;
    if let Some(path) = new_local_path {
        updated_mapping.local_path = path;
    }
    if let Some(path) = new_remote_path {
        updated_mapping.remote_path = path;
    }

    ssh::ssh_update_file_mapping(state, app.clone(), updated_mapping).await?;

    Ok(true)
}

fn detect_legacy_local_config_path(config_path: &Path) -> bool {
    config_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(legacy_file_name)
        .is_some()
}

#[tauri::command]
pub async fn get_oh_my_openagent_upgrade_status(
    state: tauri::State<'_, DbState>,
) -> Result<OhMyOpenAgentLegacyUpgradeStatus, String> {
    let db = state.db();
    let opencode_config_path = open_code::get_opencode_config_path_info(state.clone()).await?;
    let oh_my_openagent_path = runtime_location::get_omo_config_path_async(&db).await?;
    let wsl_config = wsl::wsl_get_config(state.clone()).await?;
    let ssh_config = ssh::ssh_get_config(state.clone()).await?;

    let has_legacy_plugin = detect_legacy_plugin_usage(state.clone()).await?;
    let has_legacy_local_config = detect_legacy_local_config_path(&oh_my_openagent_path);
    let has_legacy_custom_config_path =
        opencode_config_path.source == "custom" && is_legacy_omo_path(&opencode_config_path.path);
    let has_legacy_wsl_mapping = wsl_config.file_mappings.iter().any(|mapping| {
        mapping.id == OMO_MAPPING_ID
            && (is_legacy_omo_path(&mapping.windows_path) || is_legacy_omo_path(&mapping.wsl_path))
    });
    let has_legacy_ssh_mapping = ssh_config.file_mappings.iter().any(|mapping| {
        mapping.id == OMO_MAPPING_ID
            && (is_legacy_omo_path(&mapping.local_path) || is_legacy_omo_path(&mapping.remote_path))
    });

    Ok(OhMyOpenAgentLegacyUpgradeStatus {
        needs_upgrade: has_legacy_plugin
            || has_legacy_local_config
            || has_legacy_custom_config_path
            || has_legacy_wsl_mapping
            || has_legacy_ssh_mapping,
        has_legacy_plugin,
        has_legacy_local_config,
        has_legacy_custom_config_path,
        has_legacy_wsl_mapping,
        has_legacy_ssh_mapping,
        local_config_path: Some(oh_my_openagent_path.to_string_lossy().to_string()),
    })
}

#[tauri::command]
pub async fn upgrade_oh_my_openagent_legacy_setup(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
) -> Result<OhMyOpenAgentLegacyUpgradeResult, String> {
    let db = state.db();
    let oh_my_openagent_path = runtime_location::get_omo_config_path_async(&db).await?;
    let wsl_config = wsl::wsl_get_config(state.clone()).await?;
    let ssh_config = ssh::ssh_get_config(state.clone()).await?;
    let normalized_opencode_config = load_normalized_opencode_config(state.clone()).await?;

    let mut plugin_updated = false;
    let mut local_config_renamed = false;
    let mut custom_config_path_updated = false;
    let wsl_mapping_updated;
    let wsl_file_renamed;
    let mut ssh_mapping_updated = false;

    if let Some(file_name) = oh_my_openagent_path
        .file_name()
        .and_then(|name| name.to_str())
    {
        if let Some(legacy_name) = legacy_file_name(file_name) {
            let canonical_path = oh_my_openagent_path.with_file_name(legacy_name.canonical_name);
            rename_local_file_atomic(&oh_my_openagent_path, &canonical_path)?;
            local_config_renamed = true;
        }
    }

    if update_custom_opencode_config_path_if_needed(state.clone(), &app).await? {
        custom_config_path_updated = true;
    }

    let (did_update_wsl_mapping, did_rename_wsl_file) =
        update_wsl_mapping_if_needed(state.clone(), &app, &wsl_config).await?;
    wsl_mapping_updated = did_update_wsl_mapping;
    wsl_file_renamed = did_rename_wsl_file;

    if update_ssh_mapping_if_needed(state.clone(), &app, &ssh_config).await? {
        ssh_mapping_updated = true;
    }

    if let Some(config) = normalized_opencode_config {
        open_code::apply_config_internal(state.clone(), &app, config, false).await?;
        plugin_updated = true;
    }

    if !plugin_updated && (local_config_renamed || custom_config_path_updated) {
        let _ = app.emit("config-changed", "window");
    }

    let changed = plugin_updated
        || local_config_renamed
        || custom_config_path_updated
        || wsl_mapping_updated
        || ssh_mapping_updated;

    Ok(OhMyOpenAgentLegacyUpgradeResult {
        changed,
        plugin_updated,
        local_config_renamed,
        custom_config_path_updated,
        wsl_mapping_updated,
        wsl_file_renamed,
        ssh_mapping_updated,
    })
}
