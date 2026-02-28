//! OpenClaw Tray Support Module
//!
//! Provides standardized API for tray menu integration.

use super::types::{OpenClawConfig, ReadOpenClawConfigResult};
use super::commands::read_openclaw_config;
use tauri::{AppHandle, Manager, Runtime};

/// Item for model selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayModelItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
}

/// Data for model submenu in tray
#[derive(Debug, Clone)]
pub struct TrayModelData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayModelItem>,
}

/// Helper to extract OpenClawConfig from ReadOpenClawConfigResult
fn extract_config_or_default(result: ReadOpenClawConfigResult) -> OpenClawConfig {
    match result {
        ReadOpenClawConfigResult::Success { config } => config,
        _ => OpenClawConfig {
            models: None,
            agents: None,
            env: None,
            tools: None,
            other: serde_json::Map::new(),
        },
    }
}

/// Get tray model data for OpenClaw primary model
pub async fn get_openclaw_tray_model_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayModelData, String> {
    let result = read_openclaw_config(app.state()).await?;
    let config = extract_config_or_default(result);

    // Get current primary model
    let current_primary = config
        .agents
        .as_ref()
        .and_then(|a| a.defaults.as_ref())
        .and_then(|d| d.model.as_ref())
        .map(|m| m.primary.as_str())
        .unwrap_or("");

    // Build model items from all providers
    let mut items: Vec<TrayModelItem> = Vec::new();
    if let Some(ref models_section) = config.models {
        if let Some(ref providers) = models_section.providers {
            for (provider_id, provider_config) in providers {
                for model in &provider_config.models {
                    let model_full_id = format!("{}/{}", provider_id, model.id);
                    let display_name = format!(
                        "{} / {}",
                        provider_id,
                        model.name.as_deref().unwrap_or(&model.id)
                    );
                    items.push(TrayModelItem {
                        id: model_full_id.clone(),
                        display_name,
                        is_selected: current_primary == model_full_id,
                    });
                }
            }
        }
    }

    // Find current display name
    let current_display = items
        .iter()
        .find(|i| i.is_selected)
        .map(|i| {
            i.display_name
                .split(" / ")
                .nth(1)
                .unwrap_or(&i.display_name)
                .to_string()
        })
        .unwrap_or_else(|| {
            if current_primary.is_empty() {
                String::new()
            } else if let Some(pos) = current_primary.rfind('/') {
                current_primary[pos + 1..].to_string()
            } else {
                current_primary.to_string()
            }
        });

    Ok(TrayModelData {
        title: "主模型".to_string(),
        current_display,
        items,
    })
}

/// Apply model selection from tray menu
pub async fn apply_openclaw_model<R: Runtime>(
    app: &AppHandle<R>,
    item_id: &str,
) -> Result<(), String> {
    let result = read_openclaw_config(app.state()).await?;
    let mut config = extract_config_or_default(result);

    // Ensure agents.defaults.model exists
    let mut agents = config.agents.unwrap_or(super::types::OpenClawAgentsSection {
        defaults: None,
        extra: std::collections::HashMap::new(),
    });
    let mut defaults = agents.defaults.unwrap_or(super::types::OpenClawAgentsDefaults {
        model: None,
        models: None,
        extra: std::collections::HashMap::new(),
    });

    if let Some(ref mut model) = defaults.model {
        model.primary = item_id.to_string();
    } else {
        defaults.model = Some(super::types::OpenClawDefaultModel {
            primary: item_id.to_string(),
            fallbacks: Vec::new(),
            extra: std::collections::HashMap::new(),
        });
    }

    agents.defaults = Some(defaults);
    config.agents = Some(agents);

    super::commands::apply_config_internal(app.state(), app, config, true).await
}

/// Check if OpenClaw should be shown in tray menu
/// Returns true if config file exists
pub async fn is_enabled_for_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    match read_openclaw_config(app.state()).await {
        Ok(ReadOpenClawConfigResult::Success { .. }) => true,
        _ => false,
    }
}
