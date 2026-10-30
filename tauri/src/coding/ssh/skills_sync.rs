//! Skills sync to SSH remote
//!
//! Full sync of managed skills to remote server's central repo with owned copies or symlinks in tool directories.

use std::collections::HashSet;

use log::info;
use tauri::{AppHandle, Emitter};

use super::commands::get_ssh_config_internal;
use super::session::SshSession;
use super::sync::{
    list_remote_dir, manage_remote_skill_target, read_remote_file_raw, remove_remote_path,
    sync_directory_with_progress, write_remote_file,
};
use super::types::{default_directory_excludes, SyncProgress};
use crate::coding::runtime_location;
use crate::coding::skills::central_repo::{resolve_central_repo_path, resolve_skill_central_path};
use crate::coding::skills::content_hash::hash_dir;
use crate::coding::skills::remote_target::RemoteSkillTargetAction;
use crate::coding::skills::skill_store;
use crate::coding::tools::builtin::BUILTIN_TOOLS;
use crate::SqliteDbState;

const SSH_CENTRAL_DIR: &str = "~/.ai-toolbox/skills";

/// Record a user-facing non-fatal notice: append to the run's warning list and
/// emit `ssh-sync-warning` so the manual-sync modal shows it live.
fn record_warning(warnings: &mut Vec<String>, app: &AppHandle, message: String) {
    warnings.push(message.clone());
    let _ = app.emit("ssh-sync-warning", message);
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

/// Get the remote skills directory path for a tool key
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

fn get_remote_tool_skills_dir(tool_key: &str) -> Option<String> {
    BUILTIN_TOOLS
        .iter()
        .find(|t| t.key == tool_key && t.relative_skills_dir.is_some())
        .map(|t| {
            let dir = t.relative_skills_dir.unwrap();
            if dir.starts_with("~/") || dir.starts_with("~\\") {
                dir.to_string()
            } else {
                format!("~/{}", dir)
            }
        })
}

/// Tools whose remote Skills target follows the runtime location resolved by
/// `runtime_location::get_tool_skills_path_async`, so a WSL Direct runtime keeps
/// its custom root instead of silently falling back to the remote default.
///
/// Hermes is excluded on purpose: its file mappings always write `~/.hermes/*`,
/// so its Skills stay at `~/.hermes/skills` even when a Direct state appears
/// (see the module AGENTS.md).
const SKILLS_TARGETS_FROM_RUNTIME_LOCATION: &[&str] = &[
    "claude_code",
    "codex",
    "grok",
    "kimi",
    "opencode",
    "openclaw",
    "pi",
    "oh_my_pi",
    "gemini_cli",
    "antigravity_cli",
    "zcode",
    "omo_native",
    "workbuddy",
    "codebuddy",
];

async fn get_remote_tool_skills_dir_with_db(
    db: &crate::db::SqliteDbState,
    tool_key: &str,
) -> Option<String> {
    if !SKILLS_TARGETS_FROM_RUNTIME_LOCATION.contains(&tool_key) {
        return get_remote_tool_skills_dir(tool_key);
    }

    runtime_location::get_tool_skills_path_async(db, tool_key)
        .await
        .and_then(|path| path.to_str().and_then(runtime_location::parse_wsl_unc_path))
        .map(|wsl| wsl.linux_path)
        .or_else(|| get_remote_tool_skills_dir(tool_key))
}

/// Get all tool keys that support skills
fn get_all_skill_tool_keys() -> Vec<&'static str> {
    BUILTIN_TOOLS
        .iter()
        .filter(|t| t.relative_skills_dir.is_some())
        .map(|t| t.key)
        .collect()
}

/// Sync all skills to SSH remote (called on skills-changed event)
pub async fn sync_skills_to_ssh(
    state: &SqliteDbState,
    session: &SshSession,
    app: AppHandle,
) -> Result<(), String> {
    let mut warnings = Vec::new();
    sync_skills_to_ssh_with_warnings(state, session, app, &mut warnings).await
}

pub(super) async fn sync_skills_to_ssh_with_warnings(
    state: &SqliteDbState,
    session: &SshSession,
    app: AppHandle,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    let config = get_ssh_config_internal(&db, false).await?;
    let _ = db;

    if !config.enabled {
        info!("Skills SSH sync skipped: enabled={}", config.enabled);
        return Ok(());
    }
    info!(
        "Skills SSH sync start: active_connection_id={}, remote_central_dir={}",
        config.active_connection_id, SSH_CENTRAL_DIR
    );

    // Get all managed skills
    let skills = skill_store::get_managed_skills(state).await?;
    let db = state.db();
    let central_dir = resolve_central_repo_path(&app, state)
        .await
        .map_err(|e| format!("{}", e))?;

    let total_skills = skills.len() as u32;
    info!(
        "Skills SSH sync: {} skills found, central_dir={}",
        total_skills,
        central_dir.display()
    );

    // Emit initial progress
    let _ = app.emit(
        "ssh-sync-progress",
        SyncProgress {
            phase: "skills".to_string(),
            current_item: "准备中...".to_string(),
            current: 0,
            total: total_skills,
            message: format!("Skills 同步: 0/{}", total_skills),
            current_file: None,
        },
    );

    // 1. Get existing skills in remote central repo
    let existing_remote_skills = list_remote_dir(session, SSH_CENTRAL_DIR)
        .await
        .unwrap_or_default();
    info!(
        "Skills SSH sync remote central repo scan: existing_remote_skills={}",
        existing_remote_skills.len()
    );

    // 2. Collect local skill names
    let local_skill_names: HashSet<String> = skills.iter().map(|s| s.name.clone()).collect();

    // 3. Delete skills in remote that no longer exist locally.
    // Only app-managed targets are removed; unmarked directories and foreign
    // symlinks at the same name are left untouched.
    for remote_skill in &existing_remote_skills {
        if !local_skill_names.contains(remote_skill) {
            let mut target_cleanup_failed = false;
            log::trace!(
                "Skills SSH sync removing orphan remote skill: skill_name={}",
                remote_skill
            );
            for tool_key in get_all_skill_tool_keys() {
                if let Some(remote_skills_dir) =
                    get_remote_tool_skills_dir_with_db(&db, tool_key).await
                {
                    let link_path = format!("{}/{}", remote_skills_dir, remote_skill);
                    let source = format!("{}/{}", SSH_CENTRAL_DIR, remote_skill);
                    let result = manage_remote_skill_target(
                        session,
                        &source,
                        &link_path,
                        SSH_CENTRAL_DIR,
                        RemoteSkillTargetAction::Remove,
                    )
                    .await;
                    target_cleanup_failed |= result.is_err();
                    report_target_result(
                        warnings,
                        &app,
                        remote_skill,
                        tool_key,
                        &link_path,
                        result,
                    );
                }
            }
            if target_cleanup_failed {
                continue;
            }
            let skill_path = format!("{}/{}", SSH_CENTRAL_DIR, remote_skill);
            if let Err(error) = remove_remote_path(session, &skill_path).await {
                log::warn!(
                    "Skills SSH sync failed to remove orphan remote skill directory: skill_name={}, skill_path={}, error={}",
                    remote_skill,
                    skill_path,
                    error
                );
                record_warning(
                    warnings,
                    &app,
                    format!("技能 '{}' 的远端目录清理失败：{}", remote_skill, error),
                );
            }
        }
    }

    // 4. Sync/update each skill
    let mut synced_count = 0;
    let mut all_errors: Vec<String> = vec![];
    for (idx, skill) in skills.iter().enumerate() {
        let mut skill = skill.clone();
        let current_idx = (idx + 1) as u32;

        let _ = app.emit(
            "ssh-sync-progress",
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
            log::warn!(
                "Skills SSH sync: skip '{}', source not found: {}",
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
                                "Skills SSH sync: failed to update content hash for '{}': {}",
                                skill.name,
                                error
                            );
                        }
                        skill.content_hash = Some(content_hash);
                    }
                }
                Err(error) => {
                    log::warn!(
                        "Skills SSH sync: failed to hash central Skill '{}': {}",
                        skill.name,
                        error
                    );
                }
            }
        }

        let remote_target = format!("{}/{}", SSH_CENTRAL_DIR, skill.name);
        let hash_file = format!("{}/.synced_hash", remote_target);

        // Check if content needs updating using content_hash
        let remote_hash = read_remote_file_raw(session, &hash_file)
            .await
            .unwrap_or_default()
            .trim()
            .to_string();
        let local_hash = skill.content_hash.as_deref().unwrap_or("");

        let needs_update = remote_hash != local_hash;
        log::trace!(
            "Skills SSH sync evaluating skill: name={}, source={}, remote_target={}, enabled_tools={:?}, remote_hash_len={}, local_hash_len={}, needs_update={}",
            skill.name,
            source.display(),
            remote_target,
            skill.enabled_tools,
            remote_hash.len(),
            local_hash.len(),
            needs_update
        );

        if needs_update {
            let source_str = source.to_string_lossy().to_string();
            log::trace!(
                "Skills SSH sync: syncing '{}' from {} to {}",
                skill.name,
                source_str,
                remote_target
            );
            let report_current_file = |current_file: String| {
                let _ = app.emit(
                    "ssh-sync-progress",
                    SyncProgress {
                        phase: "skills".to_string(),
                        current_item: skill.name.clone(),
                        current: current_idx,
                        total: total_skills,
                        message: format!(
                            "Skills 同步: {}/{} - {}",
                            current_idx, total_skills, skill.name
                        ),
                        current_file: Some(current_file),
                    },
                );
            };

            match sync_directory_with_progress(
                &source_str,
                &remote_target,
                session,
                &default_directory_excludes(),
                Some(&report_current_file),
            )
            .await
            {
                Ok(_) => {
                    if let Err(e) = write_remote_file(session, &hash_file, local_hash).await {
                        log::warn!(
                            "Skills SSH sync: failed to write hash for '{}': {}",
                            skill.name,
                            e
                        );
                        record_warning(
                            warnings,
                            &app,
                            format!("技能 '{}' 的同步哈希写入失败：{}", skill.name, e),
                        );
                    }
                    synced_count += 1;
                }
                Err(e) => {
                    let msg = format!("Skill '{}': {}", skill.name, e);
                    log::warn!("Skills SSH sync failed: {}", msg);
                    all_errors.push(msg);
                    continue;
                }
            }
        } else {
            log::trace!(
                "Skills SSH sync skipped content upload because hashes match: name={}, remote_target={}",
                skill.name,
                remote_target
            );
        }

        // Maintain owned copies or links for each enabled tool. Only app-managed targets are
        // created or replaced; unmarked directories and foreign symlinks stay untouched.
        for tool_key in &skill.enabled_tools {
            if let Some(remote_skills_dir) = get_remote_tool_skills_dir_with_db(&db, tool_key).await
            {
                let link_path = format!("{}/{}", remote_skills_dir, skill.name);
                let result = manage_remote_skill_target(
                    session,
                    &remote_target,
                    &link_path,
                    SSH_CENTRAL_DIR,
                    RemoteSkillTargetAction::sync_for_tool(tool_key),
                )
                .await;
                report_target_result(warnings, &app, &skill.name, tool_key, &link_path, result);
            } else {
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
            if !enabled_set.contains(tool_key) {
                if let Some(remote_skills_dir) =
                    get_remote_tool_skills_dir_with_db(&db, tool_key).await
                {
                    let link_path = format!("{}/{}", remote_skills_dir, skill.name);
                    let source = format!("{}/{}", SSH_CENTRAL_DIR, &skill.name);
                    let result = manage_remote_skill_target(
                        session,
                        &source,
                        &link_path,
                        SSH_CENTRAL_DIR,
                        RemoteSkillTargetAction::Remove,
                    )
                    .await;
                    report_target_result(warnings, &app, &skill.name, tool_key, &link_path, result);
                }
            }
        }
    }

    info!(
        "Skills SSH sync completed: updated_skills={}, total_skills={}, errors={}",
        synced_count,
        skills.len(),
        all_errors.len()
    );

    let _ = super::commands::update_sync_warnings(state, warnings).await;

    if !all_errors.is_empty() {
        return Err(all_errors.join("; "));
    }

    let _ = app.emit("ssh-skills-sync-completed", ());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SKILLS_TARGETS_FROM_RUNTIME_LOCATION;

    /// SSH Skills targets must cover every tool that `get_tool_skills_path_async`
    /// resolves. A missing key does not fail loudly — it silently pins a WSL
    /// Direct runtime to the remote default directory, so guard the alignment
    /// here. Hermes is the documented exception (see the const doc comment).
    #[test]
    fn runtime_location_skills_tools_are_all_covered() {
        const SOURCE: &str = include_str!("../runtime_location.rs");
        let start = SOURCE
            .find("pub async fn get_tool_skills_path_async")
            .expect("get_tool_skills_path_async should exist in runtime_location.rs");
        let body = &SOURCE[start..];
        let end = body
            .find("\n}\n")
            .expect("get_tool_skills_path_async should have a top-level closing brace");

        let mut resolvable: Vec<&str> = Vec::new();
        for line in body[..end].lines() {
            let Some(rest) = line.trim().strip_prefix('"') else {
                continue;
            };
            let Some((key, after)) = rest.split_once('"') else {
                continue;
            };
            if after.trim_start().starts_with("=>") {
                resolvable.push(key);
            }
        }

        let mut expected: Vec<&str> = SKILLS_TARGETS_FROM_RUNTIME_LOCATION.to_vec();
        expected.push("hermes");
        resolvable.sort_unstable();
        expected.sort_unstable();

        assert_eq!(
            resolvable, expected,
            "SSH Skills target list drifted from runtime_location::get_tool_skills_path_async"
        );
    }
}
