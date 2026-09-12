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
    check_wsl_symlink_exists, create_wsl_symlink, inspect_wsl_path_kind, list_wsl_dir,
    read_wsl_file_raw, remove_wsl_managed_symlink, remove_wsl_path, sync_directory, write_wsl_file,
    WslPathKind,
};
use super::types::{SyncProgress, WSLSyncConfig};
use crate::coding::runtime_location;
use crate::coding::skills::central_repo::{resolve_central_repo_path, resolve_skill_central_path};
use crate::coding::skills::content_hash::hash_dir;
use crate::coding::skills::skill_store;
use crate::coding::tools::builtin::BUILTIN_TOOLS;
use crate::db::helpers::db_get;
use crate::db::schema::DbTable;
use crate::SqliteDbState;

const WSL_CENTRAL_DIR: &str = "~/.ai-toolbox/skills";
static SKILLS_WSL_SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Record a user-facing non-fatal notice: append to the run's warning list and
/// emit `wsl-sync-warning` so the manual-sync modal shows it live.
fn record_warning(warnings: &mut Vec<String>, app: &AppHandle, message: String) {
    warnings.push(message.clone());
    let _ = app.emit("wsl-sync-warning", message);
}

/// Prefer the built-in display name in user-facing warnings.
fn tool_display_name(tool_key: &str) -> String {
    BUILTIN_TOOLS
        .iter()
        .find(|t| t.key == tool_key)
        .map(|t| t.display_name.to_string())
        .unwrap_or_else(|| tool_key.to_string())
}

/// Warn that creating/refreshing/removing a tool symlink failed.
fn warn_link_maintenance_failed(
    warnings: &mut Vec<String>,
    app: &AppHandle,
    skill: &str,
    tool_key: &str,
    detail: impl std::fmt::Display,
) {
    record_warning(
        warnings,
        app,
        format!(
            "技能 '{}' 在工具 '{}' 的链接维护失败：{}",
            skill,
            tool_display_name(tool_key),
            detail
        ),
    );
}

/// Warn that a real directory or foreign symlink was left untouched.
fn warn_foreign_path_kept(
    warnings: &mut Vec<String>,
    app: &AppHandle,
    skill: &str,
    tool_key: &str,
    link_path: &str,
) {
    record_warning(
        warnings,
        app,
        format!(
            "技能 '{}' 在工具 '{}' 的路径 '{}' 不是 AI Toolbox 管理的链接，已保留原样",
            skill,
            tool_display_name(tool_key),
            link_path
        ),
    );
}

/// Read WSL sync config directly from database
async fn get_wsl_config(state: &SqliteDbState) -> Result<WSLSyncConfig, String> {
    let db = state.db();
    let record = db.with_conn(|conn| db_get(conn, DbTable::WslSyncConfig, "config"))?;
    Ok(record
        .map(|value| adapter::config_from_db_value(value, vec![]))
        .unwrap_or_default())
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
    db: &crate::db::SqliteDbState,
    tool_key: &str,
) -> Option<String> {
    runtime_location::get_tool_skills_path_async(db, tool_key)
        .await
        .and_then(|path| path.to_str().and_then(runtime_location::parse_wsl_unc_path))
        .map(|wsl| wsl.linux_path)
        // Local Windows runtimes still sync into the tool's standard WSL directory.
        .or_else(|| get_wsl_tool_skills_dir(tool_key))
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
pub async fn sync_skills_to_wsl(state: &SqliteDbState, app: AppHandle) -> Result<(), String> {
    let mut warnings = Vec::new();
    sync_skills_to_wsl_with_warnings(state, app, &mut warnings).await
}

pub(super) async fn sync_skills_to_wsl_with_warnings(
    state: &SqliteDbState,
    app: AppHandle,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
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
    let direct_modules = runtime_location::get_wsl_direct_modules_async(state.db()).await?;
    let skipped_tool_keys: HashSet<String> = direct_modules
        .into_iter()
        .map(|module| match module.as_str() {
            "claude" => "claude_code".to_string(),
            "geminicli" => "gemini_cli".to_string(),
            _ => module,
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

    // 3. Delete skills in WSL that no longer exist in Windows.
    // Only app-managed symlinks into the central repo are removed; real
    // directories or foreign symlinks at the same name are left untouched.
    for wsl_skill in &existing_wsl_skills {
        if !windows_skill_names.contains(wsl_skill) {
            // Remove symlinks from all tool directories first
            for tool_key in get_all_skill_tool_keys() {
                if skipped_tool_keys.contains(tool_key) {
                    continue;
                }
                if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                    let link_path = format!("{}/{}", wsl_skills_dir, wsl_skill);
                    if let Err(error) =
                        remove_wsl_managed_symlink(&distro, &link_path, WSL_CENTRAL_DIR)
                    {
                        log::warn!(
                            "Skills WSL sync: failed to remove stale tool symlink '{}': {}",
                            link_path,
                            error
                        );
                        warn_link_maintenance_failed(warnings, &app, wsl_skill, tool_key, &error);
                    } else if inspect_wsl_path_kind(&distro, &link_path, WSL_CENTRAL_DIR)
                        == WslPathKind::Foreign
                    {
                        log::warn!(
                            "Skills WSL sync: keeping non-app-managed path '{}' (not a symlink into {})",
                            link_path,
                            WSL_CENTRAL_DIR
                        );
                        warn_foreign_path_kept(warnings, &app, wsl_skill, tool_key, &link_path);
                    }
                }
            }
            // Remove from central repo
            let skill_path = format!("{}/{}", WSL_CENTRAL_DIR, wsl_skill);
            if let Err(error) = remove_wsl_path(&distro, &skill_path) {
                log::warn!(
                    "Skills WSL sync: failed to remove orphan directory '{}': {}",
                    skill_path,
                    error
                );
                record_warning(
                    warnings,
                    &app,
                    format!("技能 '{}' 的远端目录清理失败：{}", wsl_skill, error),
                );
            }
        }
    }

    // 4. Sync/update each skill
    let mut synced_count = 0;
    for (idx, skill) in skills.iter().enumerate() {
        let mut skill = skill.clone();
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
            record_warning(
                warnings,
                &app,
                format!(
                    "技能 '{}' 的源目录不存在，已跳过同步：{}",
                    skill.name,
                    source.display()
                ),
            );
            continue;
        }
        if skill.source_type == "central" {
            match hash_dir(&source) {
                Ok(content_hash) => {
                    if skill.content_hash.as_deref() != Some(content_hash.as_str()) {
                        if let Err(error) = skill_store::update_skill_content_hash(
                            state,
                            &skill.id,
                            Some(content_hash.clone()),
                        )
                        .await
                        {
                            log::warn!(
                                "Skills WSL sync: failed to update content hash for '{}': {}",
                                skill.name,
                                error
                            );
                        }
                        skill.content_hash = Some(content_hash);
                    }
                }
                Err(error) => {
                    log::warn!(
                        "Skills WSL sync: failed to hash central Skill '{}': {}",
                        skill.name,
                        error
                    );
                }
            }
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
                    // Save hash for future comparison. A hash-marker failure is
                    // non-fatal: the content is already synced and the next run
                    // simply re-uploads, so warn instead of aborting the run.
                    if let Err(error) = write_wsl_file(&distro, &hash_file, windows_hash) {
                        log::warn!(
                            "Skills WSL sync: failed to write sync hash for '{}': {}",
                            skill.name,
                            error
                        );
                        record_warning(
                            warnings,
                            &app,
                            format!("技能 '{}' 的同步哈希写入失败：{}", skill.name, error),
                        );
                    }
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
                        warnings: warnings.clone(),
                    };
                    let _ = super::commands::update_sync_status(state, &sync_result).await;
                    let _ = super::commands::update_sync_warnings(state, warnings).await;
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
                match inspect_wsl_path_kind(&distro, &link_path, WSL_CENTRAL_DIR) {
                    // Nothing there: create the managed link.
                    WslPathKind::Missing => {
                        if let Err(error) = create_wsl_symlink(&distro, &wsl_target, &link_path) {
                            log::warn!(
                                "Skills WSL sync: failed to create symlink for skill '{}' tool '{}' at '{}': {}",
                                skill.name,
                                tool_key,
                                link_path,
                                error
                            );
                            warn_link_maintenance_failed(
                                warnings,
                                &app,
                                &skill.name,
                                tool_key,
                                &error,
                            );
                        }
                    }
                    // Already an app-managed link: rebuild only when stale.
                    WslPathKind::Managed => {
                        if !check_wsl_symlink_exists(&distro, &link_path, &wsl_target) {
                            if let Err(error) = create_wsl_symlink(&distro, &wsl_target, &link_path)
                            {
                                log::warn!(
                                    "Skills WSL sync: failed to refresh symlink for skill '{}' tool '{}' at '{}': {}",
                                    skill.name,
                                    tool_key,
                                    link_path,
                                    error
                                );
                                warn_link_maintenance_failed(
                                    warnings,
                                    &app,
                                    &skill.name,
                                    tool_key,
                                    &error,
                                );
                            }
                        }
                    }
                    // Real directory or foreign symlink: never touch user data.
                    WslPathKind::Foreign => {
                        log::warn!(
                            "Skills WSL sync: keeping non-app-managed path '{}' while syncing skill '{}' to tool '{}'",
                            link_path,
                            skill.name,
                            tool_key
                        );
                        warn_foreign_path_kept(warnings, &app, &skill.name, tool_key, &link_path);
                    }
                }
            } else {
                log::warn!(
                    "Skills WSL sync: could not resolve WSL skills dir for skill '{}' tool '{}'",
                    skill.name,
                    tool_key
                );
                warn_link_maintenance_failed(
                    warnings,
                    &app,
                    &skill.name,
                    tool_key,
                    "无法解析 Skills 目标目录",
                );
            }
        }

        // Remove symlinks for tools that are no longer enabled (managed links only)
        let enabled_set: HashSet<&str> = skill.enabled_tools.iter().map(|s| s.as_str()).collect();
        for tool_key in get_all_skill_tool_keys() {
            if skipped_tool_keys.contains(tool_key) {
                continue;
            }
            if !enabled_set.contains(tool_key) {
                if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                    let link_path = format!("{}/{}", wsl_skills_dir, skill.name);
                    if let Err(error) =
                        remove_wsl_managed_symlink(&distro, &link_path, WSL_CENTRAL_DIR)
                    {
                        log::warn!(
                            "Skills WSL sync: failed to remove stale symlink for skill '{}' tool '{}' at '{}': {}",
                            skill.name,
                            tool_key,
                            link_path,
                            error
                        );
                        warn_link_maintenance_failed(warnings, &app, &skill.name, tool_key, &error);
                    } else if inspect_wsl_path_kind(&distro, &link_path, WSL_CENTRAL_DIR)
                        == WslPathKind::Foreign
                    {
                        log::warn!(
                            "Skills WSL sync: keeping non-app-managed path '{}' while unsyncing skill '{}' from tool '{}'",
                            link_path,
                            skill.name,
                            tool_key
                        );
                        warn_foreign_path_kept(warnings, &app, &skill.name, tool_key, &link_path);
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
        warnings: warnings.clone(),
    };
    let _ = super::commands::update_sync_status(state, &sync_result).await;
    let _ = super::commands::update_sync_warnings(state, warnings).await;

    // Emit event for UI feedback
    let _ = app.emit("wsl-skills-sync-completed", ());
    let _ = app.emit("wsl-sync-completed", &sync_result);

    Ok(())
}
