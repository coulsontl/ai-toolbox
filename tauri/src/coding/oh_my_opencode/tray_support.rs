//! Oh My OpenCode Tray Support Module
//!
//! Provides standardized API for tray menu integration.

use crate::coding::db_id::db_clean_id;
use crate::db::DbState;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

/// Item for config selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayConfigItem {
    /// Config ID (used in event handling)
    pub id: String,
    /// Display name in menu
    pub display_name: String,
    /// Whether this config is currently selected/applied
    pub is_selected: bool,
}

/// Data for config submenu
#[derive(Debug, Clone)]
pub struct TrayConfigData {
    /// Title of the section
    pub title: String,
    /// Items for selection
    pub items: Vec<TrayConfigItem>,
}

/// Get tray config data for Oh My OpenCode
pub async fn get_oh_my_opencode_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayConfigData, String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    // Query configs from database
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM oh_my_opencode_config")
        .await
        .map_err(|e| format!("Failed to query configs: {}", e))?
        .take(0);

    let mut items: Vec<TrayConfigItem> = Vec::new();

    match records_result {
        Ok(records) => {
            for record in records {
                // 使用数据库返回的 id 字段（来自 type::string(id) as id）
                // 注意：需要使用 db_clean_id 清理表名前缀
                if let (Some(id), Some(name), Some(is_applied)) = (
                    record.get("id").and_then(|v| v.as_str()),
                    record.get("name").and_then(|v| v.as_str()),
                    record.get("is_applied").or_else(|| record.get("isApplied")).and_then(|v| v.as_bool()),
                ) {
                    items.push(TrayConfigItem {
                        id: db_clean_id(id),
                        display_name: name.to_string(),
                        is_selected: is_applied,
                    });
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to deserialize configs for tray: {}", e);
        }
    }

    // Sort by name
    items.sort_by_key(|c| c.display_name.clone());

    let data = TrayConfigData {
        title: "──── Oh My OpenCode ────".to_string(),
        items,
    };

    Ok(data)
}

/// Apply config selection from tray menu
pub async fn apply_oh_my_opencode_config<R: Runtime>(
    app: &AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    super::commands::apply_config_internal(&db, app, config_id, true).await?;

    Ok(())
}

/// Check if Oh My OpenCode should be shown in tray menu
/// Returns true if "oh-my-opencode" is in the OpenCode plugin list
pub async fn is_enabled_for_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    use crate::coding::open_code::read_opencode_config;
    use crate::coding::open_code::types::ReadConfigResult;

    let state = app.state::<DbState>();
    let config = match read_opencode_config(state).await {
        Ok(ReadConfigResult::Success { config }) => config,
        _ => return false,
    };

    // Check if "oh-my-opencode" is in the plugin list
    // Use starts_with to support versioned plugins like "oh-my-opencode@3.0.0-beta.5"
    // But exclude "oh-my-opencode-slim" which is a different plugin
    if let Some(plugins) = &config.plugin {
        plugins.iter().any(|p: &String| {
            p.starts_with("oh-my-opencode") && !p.starts_with("oh-my-opencode-slim")
        })
    } else {
        false
    }
}
