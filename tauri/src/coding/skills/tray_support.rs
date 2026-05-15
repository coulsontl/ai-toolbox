//! Skills Tray Support Module
//!
//! Provides standardized API for tray menu integration.
//! This module handles all data fetching and processing for skills tray menu display.

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::adapter::parse_sync_details;
use super::central_repo::{resolve_central_repo_path, resolve_skill_central_path};
use super::path_executor::{remove_skill_target_checked, sync_skill_to_target};
use super::skill_store;
use super::tool_adapters::{
    get_all_tool_adapters, is_tool_installed_async, resolve_runtime_skills_path_async,
    runtime_adapter_by_key,
};
use super::types::{now_ms, SkillTarget};
use crate::DbState;

/// Item for tool selection in skill submenu
#[derive(Debug, Clone)]
pub struct TraySkillToolItem {
    /// Tool key (e.g. "claude_code")
    pub tool_key: String,
    /// Display name (e.g. "Claude Code")
    pub display_name: String,
    /// Whether this skill is synced to this tool
    pub is_synced: bool,
    /// Whether the tool is installed
    pub is_installed: bool,
}

/// Item for skill in tray menu
#[derive(Debug, Clone)]
pub struct TraySkillItem {
    /// Skill ID
    pub id: String,
    /// Skill name for display
    pub display_name: String,
    /// Central path for sync operations
    pub central_path: String,
    /// List of available tools with sync status
    pub tools: Vec<TraySkillToolItem>,
}

/// Data for skills section in tray menu
#[derive(Debug, Clone)]
pub struct TraySkillData {
    /// Title of the skills section
    pub title: String,
    /// List of managed skills
    pub items: Vec<TraySkillItem>,
}

/// Check if skills should be shown in tray menu
/// Reads the show_skills_in_tray setting
pub async fn is_skills_enabled_for_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    let state = app.state::<DbState>();
    let raw = skill_store::get_setting(&state, "show_skills_in_tray")
        .await
        .ok()
        .flatten();
    match raw {
        Some(s) => s == "true",
        None => false,
    }
}

/// Get skills tray data
/// Returns all managed skills with their tool sync states
pub async fn get_skills_tray_data<R: Runtime>(app: &AppHandle<R>) -> Result<TraySkillData, String> {
    let state = app.state::<DbState>();
    super::tool_adapters::set_runtime_db(state.db());

    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let all_adapters = get_all_tool_adapters(&custom_tools);

    let preferred_tools_raw = skill_store::get_setting(&state, "preferred_tools_v1")
        .await
        .ok()
        .flatten();
    let preferred_tools: Option<Vec<String>> =
        preferred_tools_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

    let tools_to_show: Vec<String> = if let Some(preferred_tool_keys) = preferred_tools {
        if !preferred_tool_keys.is_empty() {
            preferred_tool_keys
        } else {
            let mut installed_tool_keys = Vec::new();
            for adapter in &all_adapters {
                if is_tool_installed_async(adapter).await.unwrap_or(false) {
                    installed_tool_keys.push(adapter.key.clone());
                }
            }
            installed_tool_keys
        }
    } else {
        let mut installed_tool_keys = Vec::new();
        for adapter in &all_adapters {
            if is_tool_installed_async(adapter).await.unwrap_or(false) {
                installed_tool_keys.push(adapter.key.clone());
            }
        }
        installed_tool_keys
    };

    let skills = skill_store::get_managed_skills(&state).await?;
    let mut items: Vec<TraySkillItem> = Vec::new();

    for skill in skills.into_iter().filter(|skill| skill.management_enabled) {
        let targets = parse_sync_details(&skill);
        let synced_tools: std::collections::HashSet<String> =
            targets.iter().map(|target| target.tool.clone()).collect();

        let mut tool_items: Vec<TraySkillToolItem> = Vec::new();
        for tool_key in &tools_to_show {
            let Some(adapter) = all_adapters.iter().find(|item| item.key == *tool_key) else {
                continue;
            };

            let is_installed = is_tool_installed_async(adapter).await.unwrap_or(false);
            tool_items.push(TraySkillToolItem {
                tool_key: tool_key.clone(),
                display_name: adapter.display_name.clone(),
                is_synced: synced_tools.contains(tool_key),
                is_installed,
            });
        }

        items.push(TraySkillItem {
            id: skill.id,
            display_name: skill.name,
            central_path: skill.central_path,
            tools: tool_items,
        });
    }

    Ok(TraySkillData {
        title: "──── Skills ────".to_string(),
        items,
    })
}

/// Apply skill tool toggle from tray menu
/// Toggles the sync state of a skill for a specific tool
pub async fn apply_skills_tool_toggle<R: Runtime>(
    app: &AppHandle<R>,
    skill_id: &str,
    tool_key: &str,
) -> Result<(), String> {
    let state = app.state::<DbState>();

    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();

    let skill = skill_store::get_skill_by_id(&state, skill_id)
        .await?
        .ok_or_else(|| format!("Skill not found: {}", skill_id))?;
    if !skill.management_enabled {
        return Err(format!("Skill is disabled: {}", skill_id));
    }

    let runtime_adapter = runtime_adapter_by_key(tool_key, &custom_tools)
        .ok_or_else(|| format!("Unknown tool: {}", tool_key))?;

    if !runtime_adapter.is_custom
        && !is_tool_installed_async(&runtime_adapter)
            .await
            .unwrap_or(false)
    {
        return Err(format!("Tool not installed: {}", tool_key));
    }

    let existing_target = skill_store::get_skill_target(&state, skill_id, tool_key).await?;
    let central_dir = resolve_central_repo_path(app, &state)
        .await
        .map_err(|e| format!("{:#}", e))?;
    let skill_source_path = resolve_skill_central_path(&skill.central_path, &central_dir);

    if let Some(target) = existing_target.as_ref() {
        remove_skill_target_checked(&skill_source_path, &target.target_path)
            .map_err(|e| format!("{:#}", e))?;
        skill_store::delete_skill_target(&state, skill_id, tool_key).await?;
    } else {
        let tool_root = resolve_runtime_skills_path_async(&runtime_adapter)
            .await
            .map_err(|e| format!("{:#}", e))?;
        let target = tool_root.join(&skill.name);

        let result = sync_skill_to_target(
            tool_key,
            &skill_source_path,
            &target,
            true,
            runtime_adapter.force_copy,
        )
        .map_err(|e| format!("{:#}", e))?;

        let record = SkillTarget {
            tool: tool_key.to_string(),
            target_path: result.target_path.to_string_lossy().to_string(),
            mode: result.mode_used.as_str().to_string(),
            status: "ok".to_string(),
            error_message: None,
            synced_at: Some(now_ms()),
        };
        skill_store::upsert_skill_target(&state, skill_id, &record).await?;
    }

    let _ = app.emit("skills-changed", "tray");

    Ok(())
}
