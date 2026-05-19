//! Codex Tray Support Module
//!
//! Provides standardized API for tray menu integration.

use crate::coding::codex::apply_config_internal;
use crate::db::SqliteDbState;
use tauri::{AppHandle, Manager, Runtime};

/// Item for provider selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayProviderItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
    pub is_disabled: bool,
    pub sort_index: i64,
}

/// Data for provider submenu
#[derive(Debug, Clone)]
pub struct TrayProviderData {
    pub title: String,
    pub items: Vec<TrayProviderItem>,
}

/// Get tray provider data for Codex
pub async fn get_codex_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let providers = super::commands::list_codex_providers(app.state()).await?;
    let mut items: Vec<TrayProviderItem> = providers
        .into_iter()
        .filter(|provider| provider.id != "__local__")
        .map(|provider| TrayProviderItem {
            id: provider.id,
            display_name: provider.name,
            is_selected: provider.is_applied,
            is_disabled: provider.is_disabled,
            sort_index: provider.sort_index.unwrap_or(0) as i64,
        })
        .collect();

    items.sort_by_key(|c| c.sort_index);

    Ok(TrayProviderData {
        title: "──── Codex ────".to_string(),
        items,
    })
}

/// Apply provider selection from tray menu
pub async fn apply_codex_provider<R: Runtime>(
    app: &AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    let state = app.state::<SqliteDbState>();
    let db = state.db();

    apply_config_internal(&db, app, provider_id, true).await
}

/// Check if Codex should be shown in tray menu
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

pub async fn get_codex_prompt_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayPromptData, String> {
    let configs = super::commands::list_codex_prompt_configs(app.state()).await?;

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

pub async fn apply_codex_prompt_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    super::commands::apply_prompt_config_internal(app.state(), app, config_id, true).await
}
