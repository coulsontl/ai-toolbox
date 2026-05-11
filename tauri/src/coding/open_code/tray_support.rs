//! OpenCode Tray Support Module
//!
//! Provides standardized API for tray menu integration.
//! This module handles all data fetching and processing for tray menu display.

use crate::coding::open_code::commands::{
    is_opencode_plugin_equivalent, sanitize_opencode_plugin_list,
};
use crate::coding::open_code::free_models;
use crate::coding::open_code::types::{
    OpenCodePluginEntry, OpenCodeProvider, ReadConfigResult, UnifiedModelOption,
};
use crate::coding::open_code::{read_opencode_config, OpenCodeConfig};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, Runtime};

/// Helper to extract OpenCodeConfig from ReadConfigResult, returning default config for non-success cases
fn extract_config_or_default(result: ReadConfigResult) -> OpenCodeConfig {
    match result {
        ReadConfigResult::Success { config } => config,
        _ => OpenCodeConfig {
            schema: None,
            provider: Some(IndexMap::<String, OpenCodeProvider>::new()),
            disabled_providers: None,
            model: None,
            small_model: None,
            plugin: None,
            mcp: None,
            other: serde_json::Map::new(),
        },
    }
}

/// Item for model selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayModelItem {
    /// Unique identifier for the model (used in event handling)
    pub id: String,
    /// Display name in menu (format: "provider_name / model_name")
    pub display_name: String,
    /// Whether this model is currently selected
    pub is_selected: bool,
}

/// Data for a model submenu
#[derive(Debug, Clone)]
pub struct TrayModelData {
    /// Title of the submenu (e.g., "主模型")
    pub title: String,
    /// Currently selected model display name (shown in parentheses)
    pub current_display: String,
    /// List of available models
    pub items: Vec<TrayModelItem>,
}

/// Get tray model data for both main and small models
/// Uses the unified model fetching logic that combines custom providers and official auth providers
pub async fn get_opencode_tray_model_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(TrayModelData, TrayModelData), String> {
    let result = read_opencode_config(app.state()).await?;
    let config = extract_config_or_default(result);

    let current_main = config
        .model
        .as_ref()
        .map(|s: &String| s.as_str())
        .unwrap_or("");
    let current_small = config
        .small_model
        .as_ref()
        .map(|s: &String| s.as_str())
        .unwrap_or("");

    // Read auth.json to get official provider ids
    let auth_channels = free_models::read_auth_channels();

    // Use the unified model fetching function
    let unified_models =
        free_models::get_unified_models(&*app.state(), config.provider.as_ref(), &auth_channels)
            .await;

    // Filter out disabled providers while keeping current selections visible.
    let disabled_provider_ids: HashSet<String> = config
        .disabled_providers
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();

    let unified_models: Vec<UnifiedModelOption> = unified_models
        .into_iter()
        .filter(|m| {
            if !disabled_provider_ids.contains(&m.provider_id) {
                return true;
            }
            // Keep current selections visible even if their provider is disabled.
            m.id == current_main || m.id == current_small
        })
        .collect();

    // Convert to TrayModelItem
    let items: Vec<TrayModelItem> = unified_models
        .into_iter()
        .map(|m: UnifiedModelOption| TrayModelItem {
            id: m.id,
            display_name: m.display_name,
            is_selected: false,
        })
        .collect();

    // Find current selections - create separate clones for each model type
    let main_items: Vec<TrayModelItem> = items
        .iter()
        .map(|item| TrayModelItem {
            id: item.id.clone(),
            display_name: item.display_name.clone(),
            is_selected: current_main == item.id,
        })
        .collect();

    let small_items: Vec<TrayModelItem> = items
        .iter()
        .map(|item| TrayModelItem {
            id: item.id.clone(),
            display_name: item.display_name.clone(),
            is_selected: current_small == item.id,
        })
        .collect();

    // Extract current display names
    let main_display = find_model_display_name(&main_items, current_main);
    let small_display = find_model_display_name(&small_items, current_small);

    let main_data = TrayModelData {
        title: "主模型".to_string(),
        current_display: main_display,
        items: main_items,
    };

    let small_data = TrayModelData {
        title: "小模型".to_string(),
        current_display: small_display,
        items: small_items,
    };

    Ok((main_data, small_data))
}

/// Helper to find display name for current selection
fn find_model_display_name(items: &[TrayModelItem], current: &str) -> String {
    if current.is_empty() {
        return String::new();
    }

    // item.id format: "provider_id/model_id"
    // current format: "provider_id/model_id" (from config)
    for item in items {
        if item.id == current {
            // Extract just the model name from display_name (format: "provider_name / model_name")
            if let Some(model_name) = item.display_name.split(" / ").nth(1) {
                return model_name.to_string();
            }
            return item.display_name.clone();
        }
    }

    // Fallback: if not found, extract model_id from current value
    if let Some(slash_pos) = current.rfind('/') {
        let model_part = &current[slash_pos + 1..];
        return model_part.to_string();
    }

    current.to_string()
}

fn parse_model_item_id(item_id: &str) -> Result<(&str, &str), String> {
    let (provider_id, model_id) = item_id
        .split_once('/')
        .ok_or_else(|| format!("Invalid model ID format: {}", item_id))?;

    if provider_id.is_empty() || model_id.is_empty() {
        return Err(format!("Invalid model ID format: {}", item_id));
    }

    Ok((provider_id, model_id))
}

/// Apply model selection from tray menu
pub async fn apply_opencode_model<R: Runtime>(
    app: &AppHandle<R>,
    model_type: &str, // "main" or "small"
    item_id: &str,    // Format: "provider/model"
) -> Result<(), String> {
    // Parse item_id to get provider_id and model_id
    let (provider_id, model_id) = parse_model_item_id(item_id)?;

    // Read current config
    let result = read_opencode_config(app.state()).await?;
    let mut config = extract_config_or_default(result);

    // Build new config value: "provider_id/model_id" format
    let new_model_value = format!("{}/{}", provider_id, model_id);

    // Update config
    if model_type == "main" {
        config.model = Some(new_model_value);
    } else if model_type == "small" {
        config.small_model = Some(new_model_value);
    } else {
        return Err(format!("Invalid model type: {}", model_type));
    }

    // Save config from tray (will emit "tray" event)
    super::commands::apply_config_internal(app.state(), app, config, true).await?;

    Ok(())
}

/// Check if OpenCode models should be shown in tray menu
/// Returns true - OpenCode models are always visible as a core feature
pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
}

// ============================================================================
// Prompt Tray Support
// ============================================================================

#[derive(Debug, Clone)]
pub struct TrayPromptItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
}

#[derive(Debug, Clone)]
pub struct TrayPromptData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayPromptItem>,
}

fn find_prompt_display_name(items: &[TrayPromptItem]) -> String {
    items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default()
}

pub async fn get_opencode_prompt_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayPromptData, String> {
    let configs = super::commands::list_opencode_prompt_configs(app.state()).await?;

    let items: Vec<TrayPromptItem> = configs
        .into_iter()
        .filter(|config| config.id != "__local__")
        .map(|config| TrayPromptItem {
            id: config.id,
            display_name: config.name,
            is_selected: config.is_applied,
        })
        .collect();

    Ok(TrayPromptData {
        title: "全局提示词".to_string(),
        current_display: find_prompt_display_name(&items),
        items,
    })
}

pub async fn apply_opencode_prompt_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    super::commands::apply_prompt_config_internal(app.state(), app, config_id, true).await
}

// ============================================================================
// Plugin Tray Support
// ============================================================================

/// Item for plugin selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayPluginItem {
    /// Unique identifier for the plugin (plugin name)
    pub id: String,
    /// Display name in menu (plugin name)
    pub display_name: String,
    /// Whether this plugin is currently enabled in config
    pub is_selected: bool,
    /// Whether this plugin is disabled due to mutual exclusivity
    pub is_disabled: bool,
}

/// Data for plugin section in tray menu
#[derive(Debug, Clone)]
pub struct TrayPluginData {
    /// Title of the plugin section
    pub title: String,
    /// List of available plugins
    pub items: Vec<TrayPluginItem>,
}

/// Mutually exclusive plugins - if one is selected, the other should be disabled
const MUTUALLY_EXCLUSIVE_PLUGINS: &[(&str, &str)] = &[
    ("oh-my-openagent", "oh-my-opencode-slim"),
    ("oh-my-opencode", "oh-my-opencode-slim"),
    ("oh-my-opencode-slim", "oh-my-openagent"),
    ("oh-my-opencode-slim", "oh-my-opencode"),
];

fn remembered_plugin_entries() -> &'static Mutex<HashMap<String, OpenCodePluginEntry>> {
    static REMEMBERED_PLUGIN_ENTRIES: OnceLock<Mutex<HashMap<String, OpenCodePluginEntry>>> =
        OnceLock::new();
    REMEMBERED_PLUGIN_ENTRIES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember_plugin_entry(plugin_entry: &OpenCodePluginEntry) {
    if matches!(plugin_entry, OpenCodePluginEntry::Name(_)) {
        return;
    }

    if let Ok(mut remembered_entries) = remembered_plugin_entries().lock() {
        remembered_entries.retain(|remembered_name, _| {
            !is_opencode_plugin_equivalent(remembered_name, plugin_entry.name())
        });
        remembered_entries.insert(plugin_entry.name().to_string(), plugin_entry.clone());
    }
}

fn remembered_plugin_entry(plugin_name: &str) -> Option<OpenCodePluginEntry> {
    let remembered_entries = remembered_plugin_entries().lock().ok()?;
    remembered_entries
        .iter()
        .find(|(remembered_name, _)| is_opencode_plugin_equivalent(remembered_name, plugin_name))
        .map(|(_, plugin_entry)| plugin_entry.clone())
}

/// Get plugins disabled due to mutual exclusivity
fn get_disabled_plugins(selected_plugins: &[OpenCodePluginEntry]) -> Vec<String> {
    let mut disabled = Vec::new();
    for selected in selected_plugins {
        for (exclusive_a, exclusive_b) in MUTUALLY_EXCLUSIVE_PLUGINS {
            if is_opencode_plugin_equivalent(selected.name(), exclusive_a)
                && !disabled.iter().any(|item| item == exclusive_b)
            {
                disabled.push(exclusive_b.to_string());
            }
        }
    }
    disabled
}

/// Check if OpenCode plugins should be shown in tray menu
/// Reads the show_plugins_in_tray setting from common config
pub async fn is_plugins_enabled_for_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    if let Ok(Some(common_config)) = super::commands::get_opencode_common_config(app.state()).await
    {
        return common_config.show_plugins_in_tray;
    }
    false
}

/// Get plugin tray data
/// Returns all favorite plugins with their selection and disabled states
pub async fn get_opencode_tray_plugin_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayPluginData, String> {
    // Get all favorite plugins
    let favorite_plugins = super::commands::list_opencode_favorite_plugins(app.state()).await?;

    // Read current config to get enabled plugins
    let result = read_opencode_config(app.state()).await?;
    let config = extract_config_or_default(result);
    let enabled_plugins = sanitize_opencode_plugin_list(&config.plugin.unwrap_or_default());

    enabled_plugins.iter().for_each(remember_plugin_entry);

    // Calculate disabled plugins due to mutual exclusivity
    let disabled_plugins = get_disabled_plugins(&enabled_plugins);

    // Build plugin items from favorites
    let favorite_names: Vec<String> = favorite_plugins
        .iter()
        .map(|p| p.plugin_name.clone())
        .collect();

    let mut items: Vec<TrayPluginItem> = favorite_plugins
        .into_iter()
        .map(|p| {
            let plugin_name = p.plugin_name.clone();
            TrayPluginItem {
                id: plugin_name.clone(),
                display_name: plugin_name.clone(),
                is_selected: enabled_plugins
                    .iter()
                    .any(|enabled| is_opencode_plugin_equivalent(enabled.name(), &plugin_name)),
                is_disabled: disabled_plugins
                    .iter()
                    .any(|disabled| is_opencode_plugin_equivalent(disabled, &plugin_name)),
            }
        })
        .collect();

    // Append enabled plugins not already in favorites (e.g. third-party plugins from config)
    for plugin_entry in &enabled_plugins {
        let plugin_name = plugin_entry.name();
        if !favorite_names
            .iter()
            .any(|favorite_name| is_opencode_plugin_equivalent(favorite_name, plugin_name))
        {
            items.push(TrayPluginItem {
                id: plugin_name.to_string(),
                display_name: plugin_name.to_string(),
                is_selected: true,
                is_disabled: disabled_plugins
                    .iter()
                    .any(|disabled| is_opencode_plugin_equivalent(disabled, plugin_name)),
            });
        }
    }

    Ok(TrayPluginData {
        title: "──── OpenCode 插件 ────".to_string(),
        items,
    })
}

/// Apply plugin toggle from tray menu
/// Toggles the plugin selection and handles mutual exclusivity
pub async fn apply_opencode_plugin<R: Runtime>(
    app: &AppHandle<R>,
    plugin_name: &str,
) -> Result<(), String> {
    // Read current config
    let result = read_opencode_config(app.state()).await?;
    let mut config = extract_config_or_default(result);

    // Get current plugins or create empty vector
    let mut plugins = config.plugin.unwrap_or_default();

    // Toggle plugin selection
    if plugins
        .iter()
        .any(|existing| is_opencode_plugin_equivalent(existing.name(), plugin_name))
    {
        // Remove if already selected
        plugins.retain(|existing| !is_opencode_plugin_equivalent(existing.name(), plugin_name));
    } else {
        // Add if not selected
        plugins.push(
            remembered_plugin_entry(plugin_name)
                .unwrap_or_else(|| OpenCodePluginEntry::Name(plugin_name.to_string())),
        );

        // Handle mutual exclusivity - remove mutually exclusive plugins
        for (exclusive_a, exclusive_b) in MUTUALLY_EXCLUSIVE_PLUGINS {
            if is_opencode_plugin_equivalent(plugin_name, exclusive_a) {
                plugins.retain(|existing| {
                    !is_opencode_plugin_equivalent(existing.name(), exclusive_b)
                });
            }
        }
    }

    // Update config
    config.plugin = Some(sanitize_opencode_plugin_list(&plugins));

    // Save config from tray (will emit "tray" event)
    super::commands::apply_config_internal(app.state(), app, config, true).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_model_item_id, remember_plugin_entry, remembered_plugin_entries,
        remembered_plugin_entry,
    };
    use crate::coding::open_code::types::OpenCodePluginEntry;
    use serde_json::json;

    fn clear_remembered_plugin_entries() {
        if let Ok(mut remembered_entries) = remembered_plugin_entries().lock() {
            remembered_entries.clear();
        }
    }

    #[test]
    fn remembered_plugin_entry_restores_tuple_options_for_equivalent_plugin_name() {
        clear_remembered_plugin_entries();
        let tuple_plugin_entry = OpenCodePluginEntry::NameWithOptions((
            "oh-my-openagent@latest".to_string(),
            json!({ "enabled": true }).as_object().cloned().unwrap(),
        ));

        remember_plugin_entry(&tuple_plugin_entry);

        assert_eq!(
            remembered_plugin_entry("oh-my-opencode"),
            Some(tuple_plugin_entry)
        );
    }

    #[test]
    fn parse_model_item_id_keeps_slashes_inside_model_id() {
        assert_eq!(
            parse_model_item_id("zenmux/openai/gpt-5.5-fast"),
            Ok(("zenmux", "openai/gpt-5.5-fast"))
        );
    }

    #[test]
    fn parse_model_item_id_rejects_missing_provider_or_model() {
        assert!(parse_model_item_id("gpt-5.5-fast").is_err());
        assert!(parse_model_item_id("/gpt-5.5-fast").is_err());
        assert!(parse_model_item_id("opencode/").is_err());
    }
}
