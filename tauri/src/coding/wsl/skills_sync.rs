//! Skills sync to WSL
//!
//! Full sync of managed skills to WSL's central repo with owned copies or symlinks in tool directories.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use log::info;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use super::adapter;
use super::sync::{
    list_wsl_dir, manage_wsl_skill_target, manage_wsl_skill_targets_batch, read_wsl_file_raw,
    remove_wsl_path, sync_directory, write_wsl_file,
};
use super::types::{SyncProgress, WSLSyncConfig};
use crate::coding::runtime_location;
use crate::coding::skills::central_repo::{resolve_central_repo_path, resolve_skill_central_path};
use crate::coding::skills::content_hash::hash_dir;
use crate::coding::skills::remote_target::{RemoteSkillTargetAction, SkillTargetOperation};
use crate::coding::skills::skill_store;
use crate::coding::skills::sync_warnings::merge_foreign_path_warnings;
use crate::coding::tools::builtin::BUILTIN_TOOLS;
use crate::db::helpers::db_get;
use crate::db::schema::{DbTable, SyncEntityKind};
use crate::SqliteDbState;

const WSL_CENTRAL_DIR: &str = "~/.ai-toolbox/skills";
static SKILLS_WSL_SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct PendingSkillTarget {
    tool_key: String,
    target: String,
    action: RemoteSkillTargetAction,
}

/// Run all target operations for one skill in a single `wsl.exe` invocation.
/// Returns true when at least one operation failed.
fn run_pending_targets(
    distro: &str,
    source: &str,
    skill_name: &str,
    pending_targets: &[PendingSkillTarget],
    warnings: &mut Vec<String>,
    app: &AppHandle,
) -> bool {
    if pending_targets.is_empty() {
        return false;
    }

    let operations: Vec<SkillTargetOperation> = pending_targets
        .iter()
        .map(|pending| SkillTargetOperation {
            source: source.to_string(),
            target: pending.target.clone(),
            central_root: WSL_CENTRAL_DIR.to_string(),
            action: pending.action,
        })
        .collect();

    let mut failed = false;
    match manage_wsl_skill_targets_batch(distro, &operations) {
        Ok(results) => {
            for (pending, result) in pending_targets.iter().zip(results) {
                failed |= result.is_err();
                report_target_result(
                    warnings,
                    app,
                    skill_name,
                    &pending.tool_key,
                    &pending.target,
                    result,
                );
            }
        }
        Err(error) => {
            log::warn!(
                "Skills WSL sync: batched target maintenance failed, falling back to individual calls: {}",
                error
            );
            for pending in pending_targets {
                let result = manage_wsl_skill_target(
                    distro,
                    source,
                    &pending.target,
                    WSL_CENTRAL_DIR,
                    pending.action,
                );
                failed |= result.is_err();
                report_target_result(
                    warnings,
                    app,
                    skill_name,
                    &pending.tool_key,
                    &pending.target,
                    result,
                );
            }
        }
    }
    failed
}

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
            "技能 '{}' 在工具 '{}' 的同步目标维护失败：{}",
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
            "技能 '{}' 在工具 '{}' 的路径 '{}' 不是 AI Toolbox 管理的同步目标，已保留原样",
            skill,
            tool_display_name(tool_key),
            link_path
        ),
    );
}

fn report_target_result(
    warnings: &mut Vec<String>,
    app: &AppHandle,
    skill: &str,
    tool_key: &str,
    target: &str,
    result: Result<bool, String>,
) {
    match result {
        Ok(true) => {}
        Ok(false) => warn_foreign_path_kept(warnings, app, skill, tool_key, target),
        Err(error) => warn_link_maintenance_failed(warnings, app, skill, tool_key, error),
    }
}

/// Read WSL sync config directly from database
async fn get_wsl_config(state: &SqliteDbState) -> Result<WSLSyncConfig, String> {
    let db = state.db();
    let record = db.with_conn(|conn| db_get(conn, DbTable::SyncEntity(SyncEntityKind::WslConfig), "config"))?;
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
///
/// Owns the run's persisted outcome; [`sync_skills_to_wsl_with_warnings`] only
/// does the work so an enclosing full sync can report the combined result.
pub async fn sync_skills_to_wsl(state: &SqliteDbState, app: AppHandle) -> Result<(), String> {
    let _sync_guard = super::sync_lock::acquire().await;

    // Disabled sync must not rewrite the persisted status either: the pass
    // returns early and would otherwise report a "successful" run.
    let config = get_wsl_config(state).await?;
    if !config.enabled || !config.sync_skills {
        return Ok(());
    }

    let mut warnings = Vec::new();
    let outcome = sync_skills_to_wsl_with_warnings(state, app.clone(), &mut warnings).await;

    let sync_result = match &outcome {
        Ok(()) => super::types::SyncResult {
            success: true,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec![],
            warnings: warnings.clone(),
        },
        Err(error_message) => super::types::SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec![error_message.clone()],
            warnings: warnings.clone(),
        },
    };
    // Partial run: merge rather than replace, so the MCP chain's diagnostics
    // survive a skills-only trigger (and vice versa).
    let _ = super::commands::update_sync_status(state, &sync_result).await;
    let _ = super::commands::merge_sync_warnings(state, &warnings).await;

    // Emit event for UI feedback
    let _ = app.emit("wsl-skills-sync-completed", ());
    let _ = app.emit("wsl-sync-completed", &sync_result);

    outcome
}

/// Do the skills sync work, appending user-facing warnings to `warnings`.
///
/// Never persists status or emits completion events: full sync embeds this and
/// reports one combined outcome, so a pass-local `update_sync_status` followed
/// by an immediate overwrite would lose whichever ran second.
pub(super) async fn sync_skills_to_wsl_with_warnings(
    state: &SqliteDbState,
    app: AppHandle,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let outcome = sync_skills_to_wsl_inner(state, app, warnings).await;
    merge_foreign_path_warnings(warnings);
    outcome
}

async fn sync_skills_to_wsl_inner(
    state: &SqliteDbState,
    app: AppHandle,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let _skills_guard = SKILLS_WSL_SYNC_LOCK
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
            // Surface it instead of returning Ok: the standalone entry point
            // would otherwise record a "successful" run for a sync that never
            // reached WSL.
            log::warn!("WSL Skills sync skipped: {}", e);
            let _ = app.emit("wsl-sync-warning", format!("WSL Skills 同步已跳过：{}", e));
            return Err(format!("WSL Skills sync skipped: {}", e));
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

    let mut total_target_operations = 0usize;
    let mut total_target_time = Duration::ZERO;
    let mut total_hash_time = Duration::ZERO;

    // 3. Delete skills in WSL that no longer exist in Windows.
    // Only app-managed targets are removed; unmarked directories and foreign
    // symlinks at the same name are left untouched.
    for wsl_skill in &existing_wsl_skills {
        if !windows_skill_names.contains(wsl_skill) {
            let mut pending_targets = Vec::new();
            // Remove symlinks from all tool directories first
            for tool_key in get_all_skill_tool_keys() {
                if skipped_tool_keys.contains(tool_key) {
                    continue;
                }
                if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                    pending_targets.push(PendingSkillTarget {
                        tool_key: tool_key.to_string(),
                        target: format!("{}/{}", wsl_skills_dir, wsl_skill),
                        action: RemoteSkillTargetAction::Remove,
                    });
                }
            }

            total_target_operations += pending_targets.len();
            let target_started = Instant::now();
            let source = format!("{}/{}", WSL_CENTRAL_DIR, wsl_skill);
            let target_cleanup_failed = run_pending_targets(
                &distro,
                &source,
                wsl_skill,
                &pending_targets,
                warnings,
                &app,
            );
            total_target_time += target_started.elapsed();

            // Keep the central entry discoverable so failed target cleanup is retried.
            if target_cleanup_failed {
                continue;
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
            let hash_started = Instant::now();
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
            total_hash_time += hash_started.elapsed();
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
                    // Return the error only: the enclosing entry point persists
                    // the status and emits the completion event.
                    return Err(format!(
                        "Skills WSL sync failed for '{}': {}",
                        skill.name, e
                    ));
                }
            }
        }

        // Collect target work first, then run it in one batched wsl.exe call.
        let mut pending_targets = Vec::new();

        // Maintain owned copies or links for each enabled tool
        for tool_key in &skill.enabled_tools {
            if skipped_tool_keys.contains(tool_key) {
                continue;
            }
            if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                pending_targets.push(PendingSkillTarget {
                    tool_key: tool_key.clone(),
                    target: format!("{}/{}", wsl_skills_dir, skill.name),
                    action: RemoteSkillTargetAction::sync_for_tool(tool_key),
                });
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

        // Remove owned targets for tools that are no longer enabled
        let enabled_set: HashSet<&str> = skill.enabled_tools.iter().map(|s| s.as_str()).collect();
        for tool_key in get_all_skill_tool_keys() {
            if skipped_tool_keys.contains(tool_key) {
                continue;
            }
            if !enabled_set.contains(tool_key) {
                if let Some(wsl_skills_dir) = get_wsl_tool_skills_dir_with_db(&db, tool_key).await {
                    pending_targets.push(PendingSkillTarget {
                        tool_key: tool_key.to_string(),
                        target: format!("{}/{}", wsl_skills_dir, skill.name),
                        action: RemoteSkillTargetAction::Remove,
                    });
                }
            }
        }

        total_target_operations += pending_targets.len();
        let target_started = Instant::now();
        run_pending_targets(
            &distro,
            &wsl_target,
            &skill.name,
            &pending_targets,
            warnings,
            &app,
        );
        total_target_time += target_started.elapsed();
    }

    info!(
        "Skills WSL sync completed: {} skills updated, {} total, target_operations={}, hash_ms={}, target_ms={}",
        synced_count,
        skills.len(),
        total_target_operations,
        total_hash_time.as_millis(),
        total_target_time.as_millis()
    );

    Ok(())
}
