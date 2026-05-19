//! Oh My OpenAgent Tray Support Module
//!
//! Provides standardized API for tray menu integration.

use crate::db::helpers::db_list;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;
use tauri::{AppHandle, Manager, Runtime};

fn is_oh_my_openagent_plugin(plugin_name: &str) -> bool {
    let base_name = plugin_name.split('@').next().unwrap_or(plugin_name);
    matches!(base_name, "oh-my-openagent" | "oh-my-opencode")
}

/// Item for config selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayConfigItem {
    /// Config ID (used in event handling)
    pub id: String,
    /// Display name in menu
    pub display_name: String,
    /// Whether this config is currently selected/applied
    pub is_selected: bool,
    /// Whether this config is disabled
    pub is_disabled: bool,
    /// Sort index for ordering
    pub sort_index: i64,
}

/// Data for config submenu
#[derive(Debug, Clone)]
pub struct TrayConfigData {
    /// Title of the section
    pub title: String,
    /// Items for selection
    pub items: Vec<TrayConfigItem>,
}

/// Get tray config data for Oh My OpenAgent.
pub async fn get_oh_my_openagent_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayConfigData, String> {
    let state = app.state::<SqliteDbState>();
    let db = state.db();

    let mut items = db.with_conn(|conn| {
        db_list(conn, DbTable::OhMyOpenAgentConfig, None).map(|records| {
            records
                .into_iter()
                .filter_map(|record| {
                    let id = record.get("id")?.as_str()?;
                    let name = record.get("name")?.as_str()?;
                    let is_applied = record
                        .get("is_applied")
                        .or_else(|| record.get("isApplied"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let is_disabled = record
                        .get("is_disabled")
                        .or_else(|| record.get("isDisabled"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let sort_index = record
                        .get("sort_index")
                        .or_else(|| record.get("sortIndex"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);

                    Some(TrayConfigItem {
                        id: id.to_string(),
                        display_name: name.to_string(),
                        is_selected: is_applied,
                        is_disabled,
                        sort_index,
                    })
                })
                .collect::<Vec<_>>()
        })
    })?;
    items.sort_by_key(|c| c.sort_index);

    let data = TrayConfigData {
        title: "──── Oh My OpenAgent ────".to_string(),
        items,
    };

    Ok(data)
}

/// Apply config selection from tray menu
pub async fn apply_oh_my_openagent_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    let state = app.state::<SqliteDbState>();
    let db = state.db();

    super::commands::apply_config_internal(&db, app, config_id, true).await?;

    Ok(())
}

/// Check if Oh My OpenAgent should be shown in tray menu.
/// Accept both the canonical "oh-my-openagent" name and legacy "oh-my-opencode".
pub async fn is_enabled_for_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    use crate::coding::open_code::read_opencode_config;
    use crate::coding::open_code::types::ReadConfigResult;

    let state = app.state::<SqliteDbState>();
    let config = match read_opencode_config(state).await {
        Ok(ReadConfigResult::Success { config }) => config,
        _ => return false,
    };

    if let Some(plugins) = &config.plugin {
        plugins
            .iter()
            .any(|plugin_entry| is_oh_my_openagent_plugin(plugin_entry.name()))
    } else {
        false
    }
}
