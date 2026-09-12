use crate::coding::db_id::db_clean_id;
use crate::coding::proxy_gateway::{
    cli_proxy, paths::ProxyGatewayPaths, provider_switch, types::GatewayCliKey,
};
use crate::db::helpers::db_list;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

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
    cli_proxy::provider_switch_locked_by_manifest(&paths, GatewayCliKey::Gemini)
}

fn provider_disabled_for_tray(
    provider_disabled: bool,
    is_applied: bool,
    category: &str,
    gateway_active: bool,
) -> bool {
    provider_disabled || (gateway_active && (is_applied || category == "official"))
}

pub async fn get_gemini_cli_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let state = app.state::<SqliteDbState>();
    let db = state.db();
    let gateway_switch_locked = gateway_provider_switch_locked(app);
    let records = db.with_conn(|conn| db_list(conn, DbTable::GeminiCliProvider, None))?;
    let mut items = records
        .into_iter()
        .filter_map(|record| {
            let raw_id = record.get("id").and_then(Value::as_str)?;
            let name = record.get("name").and_then(Value::as_str)?;
            let is_applied = record
                .get("is_applied")
                .or_else(|| record.get("isApplied"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let sort_index = record
                .get("sort_index")
                .or_else(|| record.get("sortIndex"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let is_disabled = record
                .get("is_disabled")
                .or_else(|| record.get("isDisabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let category = record
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or("custom");
            Some(TrayProviderItem {
                id: db_clean_id(raw_id),
                display_name: name.to_string(),
                is_selected: is_applied,
                is_disabled: provider_disabled_for_tray(
                    is_disabled,
                    is_applied,
                    category,
                    gateway_switch_locked,
                ),
                sort_index,
            })
        })
        .collect::<Vec<_>>();
    items.sort_by_key(|item| item.sort_index);
    let current_display = find_provider_display_name(&items);
    Ok(TrayProviderData {
        title: "──── Gemini CLI ────".to_string(),
        current_display,
        items,
    })
}

pub async fn apply_gemini_cli_provider<R: Runtime>(
    app: &AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    provider_switch::apply_or_switch_provider(app, GatewayCliKey::Gemini, provider_id, true)
        .await?;
    Ok(())
}

pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
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

fn find_prompt_display_name(items: &[TrayPromptItem]) -> String {
    items
        .iter()
        .find(|item| item.is_selected)
        .map(|item| item.display_name.clone())
        .unwrap_or_default()
}

pub async fn get_gemini_cli_prompt_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayPromptData, String> {
    let configs = super::commands::list_gemini_cli_prompt_configs(app.state()).await?;
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

pub async fn apply_gemini_cli_prompt_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    super::commands::apply_prompt_config_internal(app.state(), app, config_id, true).await
}
