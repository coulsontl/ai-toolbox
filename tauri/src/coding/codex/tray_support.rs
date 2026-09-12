//! Codex Tray Support Module
//!
//! Provides standardized API for tray menu integration.

use crate::coding::proxy_gateway::{
    cli_proxy, paths::ProxyGatewayPaths, provider_protocol, provider_switch, types::GatewayCliKey,
    ProxyGatewayState,
};
use tauri::{AppHandle, Manager, Runtime};

use super::constants::CODEX_LOCAL_PROVIDER_ID;

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
    pub current_display: String,
    pub items: Vec<TrayProviderItem>,
}

fn find_provider_display_name(items: &[TrayProviderItem]) -> String {
    items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default()
}

fn gateway_provider_switch_locked<R: Runtime>(_app: &AppHandle<R>) -> bool {
    let paths = ProxyGatewayPaths::new(crate::app_paths::resolved_data_dir());
    cli_proxy::provider_switch_locked_by_manifest(&paths, GatewayCliKey::Codex)
}

fn gateway_running<R: Runtime>(app: &AppHandle<R>) -> bool {
    let gateway_state = app.state::<ProxyGatewayState>();
    gateway_state
        .manager
        .lock()
        .map(|manager| manager.status().running)
        .unwrap_or(false)
}

fn provider_disabled_for_tray(
    provider_disabled: bool,
    is_applied: bool,
    category: &str,
    gateway_active: bool,
    gateway_running: bool,
    provider_needs_proxy: bool,
) -> bool {
    provider_disabled
        || (provider_needs_proxy && !gateway_running)
        || (gateway_active && (!gateway_running || is_applied || category == "official"))
}

/// Get tray provider data for Codex
pub async fn get_codex_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let providers = super::commands::list_codex_providers(app.state()).await?;
    let gateway_switch_locked = gateway_provider_switch_locked(app);
    let gateway_running = gateway_running(app);
    let mut items: Vec<TrayProviderItem> = providers
        .into_iter()
        .filter(|provider| provider.id != CODEX_LOCAL_PROVIDER_ID)
        .map(|provider| {
            let provider_needs_proxy = provider_protocol::provider_needs_gateway_proxy(
                GatewayCliKey::Codex,
                &provider.category,
                provider.meta.as_ref(),
                &provider.settings_config,
            );
            TrayProviderItem {
                id: provider.id,
                display_name: provider.name,
                is_selected: provider.is_applied,
                is_disabled: provider_disabled_for_tray(
                    provider.is_disabled,
                    provider.is_applied,
                    &provider.category,
                    gateway_switch_locked,
                    gateway_running,
                    provider_needs_proxy,
                ),
                sort_index: provider.sort_index.unwrap_or(0) as i64,
            }
        })
        .collect();

    items.sort_by_key(|c| c.sort_index);

    let current_display = find_provider_display_name(&items);

    Ok(TrayProviderData {
        title: "──── Codex ────".to_string(),
        current_display,
        items,
    })
}

/// Apply provider selection from tray menu
pub async fn apply_codex_provider<R: Runtime>(
    app: &AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    provider_switch::apply_or_switch_provider(app, GatewayCliKey::Codex, provider_id, true).await?;
    Ok(())
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
        .filter(|config| config.id != CODEX_LOCAL_PROVIDER_ID)
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
