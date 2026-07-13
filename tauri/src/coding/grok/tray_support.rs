use crate::coding::proxy_gateway::{
    cli_proxy, paths::ProxyGatewayPaths, provider_protocol, provider_switch, types::GatewayCliKey,
    ProxyGatewayState,
};
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

use super::constants::GROK_LOCAL_PROVIDER_ID;

#[derive(Debug, Clone)]
pub struct TrayProviderItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
    pub is_disabled: bool,
    pub sort_index: i64,
}
#[derive(Debug, Clone)]
pub struct TrayProviderData {
    pub title: String,
    pub items: Vec<TrayProviderItem>,
}
#[derive(Debug, Clone)]
pub struct TrayModelItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
    pub is_disabled: bool,
}
#[derive(Debug, Clone)]
pub struct TrayModelData {
    pub title: String,
    pub current_display: String,
    pub items: Vec<TrayModelItem>,
}
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

fn gateway_provider_switch_locked<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.path()
        .app_data_dir()
        .map(ProxyGatewayPaths::new)
        .map(|paths| cli_proxy::provider_switch_locked_by_manifest(&paths, GatewayCliKey::Grok))
        .unwrap_or(false)
}

fn gateway_running<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<ProxyGatewayState>()
        .manager
        .lock()
        .map(|manager| manager.status().running)
        .unwrap_or(false)
}

pub async fn get_grok_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let gateway_switch_locked = gateway_provider_switch_locked(app);
    let gateway_running = gateway_running(app);
    let mut items = super::commands::list_grok_providers(app.state())
        .await?
        .into_iter()
        .filter(|provider| provider.id != GROK_LOCAL_PROVIDER_ID)
        .map(|provider| {
            let provider_needs_proxy = provider_protocol::provider_needs_gateway_proxy(
                GatewayCliKey::Grok,
                &provider.category,
                provider.meta.as_ref(),
                &provider.settings_config,
            );
            let is_disabled = provider.is_disabled
                || (provider_needs_proxy && !gateway_running)
                || (gateway_switch_locked
                    && (!gateway_running
                        || provider.is_applied
                        || provider.category == "official"));
            TrayProviderItem {
                id: provider.id,
                display_name: provider.name,
                is_selected: provider.is_applied,
                is_disabled,
                sort_index: provider.sort_index.unwrap_or(0) as i64,
            }
        })
        .collect::<Vec<_>>();
    items.sort_by_key(|item| item.sort_index);
    Ok(TrayProviderData {
        title: "Grok".to_string(),
        items,
    })
}

pub async fn apply_grok_provider<R: Runtime>(
    app: &AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    provider_switch::apply_or_switch_provider(app, GatewayCliKey::Grok, provider_id, true)
        .await
        .map(|_| ())
}

fn model_items_from_provider(
    settings_config: &str,
    gateway_switch_locked: bool,
) -> Result<TrayModelData, String> {
    let settings: Value = serde_json::from_str(settings_config)
        .map_err(|error| format!("Invalid Grok provider settings JSON: {error}"))?;
    let default_model_key = settings
        .get("defaultModelKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let mut items = settings
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let key = model
                .get("key")
                .or_else(|| model.get("model"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let display_name = model
                .get("displayName")
                .or_else(|| model.get("name"))
                .or_else(|| model.get("model"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(key);
            Some(TrayModelItem {
                id: key.to_string(),
                display_name: display_name.to_string(),
                is_selected: key == default_model_key,
                is_disabled: gateway_switch_locked,
            })
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    let current_display = items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_else(|| default_model_key.to_string());
    Ok(TrayModelData {
        title: "Main Model".to_string(),
        current_display,
        items,
    })
}

pub async fn get_grok_model_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayModelData, String> {
    let applied_provider = super::commands::list_grok_providers_for_db(
        app.state::<crate::db::SqliteDbState>().inner(),
    )?
    .into_iter()
    .find(|provider| provider.is_applied);
    let Some(provider) = applied_provider else {
        return Ok(TrayModelData {
            title: "Main Model".to_string(),
            current_display: String::new(),
            items: Vec::new(),
        });
    };
    model_items_from_provider(
        &provider.settings_config,
        gateway_provider_switch_locked(app),
    )
}

pub async fn apply_grok_model<R: Runtime>(
    app: &AppHandle<R>,
    model_key: &str,
) -> Result<(), String> {
    if gateway_provider_switch_locked(app) {
        return Err("Restore Grok direct mode before changing the model".to_string());
    }
    let state = app.state::<crate::db::SqliteDbState>();
    super::commands::select_grok_model_internal(state.inner(), app, model_key).await
}

pub async fn get_grok_prompt_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayPromptData, String> {
    let items = super::commands::list_grok_prompt_configs(app.state())
        .await?
        .into_iter()
        .filter(|item| item.id != GROK_LOCAL_PROVIDER_ID)
        .map(|item| TrayPromptItem {
            id: item.id,
            display_name: item.name,
            is_selected: item.is_applied,
        })
        .collect::<Vec<_>>();
    let current_display = items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default();
    Ok(TrayPromptData {
        title: "Global Prompt".to_string(),
        current_display,
        items,
    })
}

pub async fn apply_grok_prompt_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    let state = app.state::<crate::db::SqliteDbState>();
    super::commands::apply_grok_prompt_config_internal(state.inner(), app, config_id).await
}

pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_items_use_applied_default_and_preserve_display_names() {
        let data = model_items_from_provider(
            &serde_json::json!({
                "defaultModelKey": "fast",
                "modelCatalog": { "models": [
                    { "key": "deep", "model": "grok-deep", "displayName": "Deep" },
                    { "key": "fast", "model": "grok-fast", "displayName": "Fast" }
                ]}
            })
            .to_string(),
            false,
        )
        .expect("build model tray data");

        assert_eq!(data.current_display, "Fast");
        assert_eq!(data.items.len(), 2);
        assert!(data
            .items
            .iter()
            .any(|item| item.id == "fast" && item.is_selected));
        assert!(data.items.iter().all(|item| !item.is_disabled));
    }

    #[test]
    fn model_items_are_disabled_during_gateway_takeover() {
        let data = model_items_from_provider(
            &serde_json::json!({
                "defaultModelKey": "grok-build",
                "modelCatalog": { "models": [{ "key": "grok-build", "model": "grok-build" }]}
            })
            .to_string(),
            true,
        )
        .expect("build locked model tray data");

        assert!(data.items.iter().all(|item| item.is_disabled));
    }
}
