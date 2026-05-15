//! Skills sync to WSL
//!
//! Full sync of managed skills to WSL's central repo with symlinks to tool directories.

use std::collections::HashSet;
use std::sync::OnceLock;

use log::info;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use super::adapter;
use super::sync::{
    check_wsl_symlink_exists, create_wsl_symlink, list_wsl_dir, read_wsl_file_raw, remove_wsl_path,
    sync_directory, write_wsl_file,
};
use super::types::{SyncProgress, WSLSyncConfig};
use crate::coding::runtime_location;
use crate::coding::skills::central_repo::{resolve_central_repo_path, resolve_skill_central_path};
use crate::coding::skills::skill_store;
use crate::coding::tools::builtin::BUILTIN_TOOLS;
use crate::DbState;

const WSL_CENTRAL_DIR: &str = "~/.ai-toolbox/skills";
static SKILLS_WSL_SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Read WSL sync config directly from database
async fn get_wsl_config(state: &DbState) -> Result<WSLSyncConfig, String> {
    let db = state.db();

    let config_result: Result<Vec<serde_json::Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM wsl_sync_config:`config` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query WSL config: {}", e))?
        .take(0);

    match config_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(adapter::config_from_db_value(record.clone(), vec![]))
            } else {
                Ok(WSLSyncConfig::default())
            }
        }
        Err(_) => Ok(WSLSyncConfig::default()),
    }
}

/// Get the WSL skills directory path for a tool key
fn get_wsl_tool_skills_dir(tool_key: &str) -> Option<String> {
    BUILTIN_TOOLS
        .iter()
        .find(|t| t.key == tool_key && t.relative_skills_dir.is_some())
        .map(|t| {
            let dir = t.relative_skills_dir.unwrap();
            // relative_skills_dir already has ~/ prefix since path unification
            if dir.starts_with("~/") || dir.starts_with("~\\") {
                dir.to_string()
            } else {
                format!("~/{}", dir)
            }
        })
}

async fn get_wsl_tool_skills_dir_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool_key: &str,
) -> Option<String> {
    match tool_key {
        "claude_code" | "codex" | "opencode" | "openclaw" => {
            runtime_location::get_tool_skills_path_async(db, tool_key)
                .await
                .and_then(|path| path.to_str().and_then(runtime_location::parse_wsl_unc_path))
                .map(|wsl| wsl.linux_path)
                // Default-path Windows runtimes are still expected to sync into the
                // tool's standard WSL skills directory.
                .or_else(|| get_wsl_tool_skills_dir(tool_key))
        }
        _ => get_wsl_tool_skills_dir(tool_key),
    }
}

/// Get all tool keys that support skills
fn get_all_skill_tool_keys() -> Vec<&'static str> {
    BUILTIN_TOOLS
        .iter()
        .filter(|t| t.relative_skills_dir.is_some())
        .map(|t| t.key)
        .collect()
}

/// Sync all skills to WSL (called on skills-changed event)
pub async fn sync_skills_to_wsl(state: &DbState, app: AppHandle) -> Result<(), String> {
    let _sync_guard = SKILLS_WSL_SYNC_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .await;

    let config = get_wsl_config(state).await?;

    if !config.enabled || !config.sync_skills {
        info!(
            "Skills WSL sync skipped: enabled={}, sync_skills={}",
            config.enabled, config.sync_skills
        );
        return Ok(());
    }

    // Get effective distro (auto-resolve if configured one doesn't exist)
    let distro = match super::sync::get_effective_distro(&config.distro) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("WSL Skills sync skipped: {}", e);
            return Ok(());
        }
    };
    let direct_statuses = runtime_location::get_wsl_direct_status_map_async(&state.db()).await?;
    let skipped_tool_keys: HashSet<String> = direct_statuses
        .into_iter()
        .filter(|status| status.is_wsl_direct)
        .filter_map(|status| match status.module.as_str() {
            "claude" => Some("claude_code".to_string()),
            "codex" => Some("codex".to_string()),
            "opencode" => Some("opencode".to_string()),
            "openclaw" => Some("openclaw".to_string()),
            _ => None,
        })
        .collect();

    // Get all managed skills
    let skills = skill_store::get_managed_skills(state).await?;
    let db = state.db();
    let central_dir = resolve_central_repo_path(&app, state)
        .await
        .map_err(|e| format!("{}", e))?;

    let total_skills = skills.len() as u32;
    info!(
        "Skills WSL sync: {} skills found, central_dir={}",
        total_skills,
        central_dir.display()
    );

    // Emit initial progress
    let _ = app.emit(
        "wsl-sync-progress",
        SyncProgress {
            phase: "skills".to_string(),
            current_item: "准备中...".to_string(),
            current: 0,
            total: total_skills,
            message: format!("Skills 同步: 0/{}", total_skills),
            current_file: None,
        },
    );

    // 1. Get existing skills in WSL central repo
    let existing_wsl_skills = list_wsl_dir(&distro, WSL_CENTRAL_DIR).unwrap_or_default();

    // 2. Collect Windows skill names
    let windows_skill_names: HashSet<String> = skills.iter().map(|s| s.name.clone()).collect();

    // 3. Delete skills in WSL that no longer exist in Windows
    for wsl_skill in &existing_wsl_skills {
        if !windows_skill_names.contains(wsl_skill) {
            // Remove symlinks from all tool directories first
            for tool_key in get_all_skill_tool_keys() {
                if skipped_tool_keys.contains(tool_key) {
                    continue;
                }
                if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                    let link_path = format!("{}/{}", wsl_skills_dir, wsl_skill);
                    let _ = remove_wsl_path(&distro, &link_path);
                }
            }
            // Remove from central repo
            let skill_path = format!("{}/{}", WSL_CENTRAL_DIR, wsl_skill);
            let _ = remove_wsl_path(&distro, &skill_path);
        }
    }

    // 4. Sync/update each skill
    let mut synced_count = 0;
    for (idx, skill) in skills.iter().enumerate() {
        let current_idx = (idx + 1) as u32;

        // Emit progress for each skill
        let _ = app.emit(
            "wsl-sync-progress",
            SyncProgress {
                phase: "skills".to_string(),
                current_item: skill.name.clone(),
                current: current_idx,
                total: total_skills,
                message: format!(
                    "Skills 同步: {}/{} - {}",
                    current_idx, total_skills, skill.name
                ),
                current_file: None,
            },
        );

        let source = resolve_skill_central_path(&skill.central_path, &central_dir);
        if !source.exists() {
            info!(
                "Skills WSL sync: skip '{}', source not found: {}",
                skill.name,
                source.display()
            );
            continue;
        }

        let wsl_target = format!("{}/{}", WSL_CENTRAL_DIR, skill.name);
        let hash_file = format!("{}/.synced_hash", wsl_target);

        // Check if content needs updating using content_hash
        let wsl_hash = read_wsl_file_raw(&distro, &hash_file)
            .unwrap_or_default()
            .trim()
            .to_string();
        let windows_hash = skill.content_hash.as_deref().unwrap_or("");

        let needs_update = wsl_hash != windows_hash;

        if needs_update {
            // Convert Windows path to WSL-accessible path and sync
            let source_str = source.to_string_lossy().to_string();
            info!(
                "Skills WSL sync: syncing '{}' from {} to {}",
                skill.name, source_str, wsl_target
            );
            match sync_directory(&source_str, &wsl_target, &distro) {
                Ok(_) => {
                    // Save hash for future comparison
                    write_wsl_file(&distro, &hash_file, windows_hash)?;
                    synced_count += 1;
                }
                Err(e) => {
                    let error_message =
                        format!("Skills WSL sync failed for '{}': {}", skill.name, e);
                    let sync_result = super::types::SyncResult {
                        success: false,
                        synced_files: vec![],
                        skipped_files: vec![],
                        errors: vec![error_message.clone()],
                    };
                    let _ = super::commands::update_sync_status(state, &sync_result).await;
                    let _ = app.emit("wsl-sync-completed", &sync_result);
                    return Err(error_message);
                }
            }
        }

        // Ensure symlinks for each enabled tool
        for tool_key in &skill.enabled_tools {
            if skipped_tool_keys.contains(tool_key) {
                continue;
            }
            if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                let link_path = format!("{}/{}", wsl_skills_dir, skill.name);
                if !check_wsl_symlink_exists(&distro, &link_path, &wsl_target) {
                    if let Err(error) = create_wsl_symlink(&distro, &wsl_target, &link_path) {
                        log::warn!(
                            "Skills WSL sync: failed to create symlink for skill '{}' tool '{}' at '{}': {}",
                            skill.name,
                            tool_key,
                            link_path,
                            error
                        );
                    }
                }
            } else {
                log::warn!(
                    "Skills WSL sync: could not resolve WSL skills dir for skill '{}' tool '{}'",
                    skill.name,
                    tool_key
                );
            }
        }

        // Remove symlinks for tools that are no longer enabled
        let enabled_set: HashSet<&str> = skill.enabled_tools.iter().map(|s| s.as_str()).collect();
        for tool_key in get_all_skill_tool_keys() {
            if skipped_tool_keys.contains(tool_key) {
                continue;
            }
            if !enabled_set.contains(tool_key) {
                if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                    let link_path = format!("{}/{}", wsl_skills_dir, skill.name);
                    if let Err(error) = remove_wsl_path(&distro, &link_path) {
                        log::warn!(
                            "Skills WSL sync: failed to remove stale symlink for skill '{}' tool '{}' at '{}': {}",
                            skill.name,
                            tool_key,
                            link_path,
                            error
                        );
                    }
                }
            }
        }
    }

    info!(
        "Skills WSL sync completed: {} skills updated, {} total",
        synced_count,
        skills.len()
    );

    // Update sync status
    let sync_result = super::types::SyncResult {
        success: true,
        synced_files: vec![],
        skipped_files: vec![],
        errors: vec![],
    };
    let _ = super::commands::update_sync_status(state, &sync_result).await;

    // Emit event for UI feedback
    let _ = app.emit("wsl-skills-sync-completed", ());
    let _ = app.emit("wsl-sync-completed", &sync_result);

    Ok(())
}
