//! Skills sync to SSH remote
//!
//! Full sync of managed skills to remote server's central repo with symlinks to tool directories.

use std::collections::HashSet;

use log::info;
use tauri::{AppHandle, Emitter};

use super::commands::get_ssh_config_internal;
use super::session::SshSession;
use super::sync::{
    check_remote_symlink_exists, create_remote_symlink, list_remote_dir, read_remote_file_raw,
    remove_remote_path, sync_directory_with_progress, write_remote_file,
};
use super::types::{default_directory_excludes, SyncProgress};
use crate::coding::runtime_location;
use crate::coding::skills::central_repo::{resolve_central_repo_path, resolve_skill_central_path};
use crate::coding::skills::skill_store;
use crate::coding::tools::builtin::BUILTIN_TOOLS;
use crate::DbState;

const SSH_CENTRAL_DIR: &str = "~/.ai-toolbox/skills";

/// Get the remote skills directory path for a tool key
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

async fn get_remote_tool_skills_dir_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool_key: &str,
) -> Option<String> {
    match tool_key {
        "claude_code" | "codex" | "opencode" | "openclaw" => {
            runtime_location::get_tool_skills_path_async(db, tool_key)
                .await
                .and_then(|path| path.to_str().and_then(runtime_location::parse_wsl_unc_path))
                .map(|wsl| wsl.linux_path)
                .or_else(|| get_remote_tool_skills_dir(tool_key))
        }
        _ => get_remote_tool_skills_dir(tool_key),
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

/// Sync all skills to SSH remote (called on skills-changed event)
pub async fn sync_skills_to_ssh(
    state: &DbState,
    session: &SshSession,
    app: AppHandle,
) -> Result<(), String> {
    let db = state.db();
    let config = get_ssh_config_internal(&db, false).await?;
    drop(db);

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

    // 3. Delete skills in remote that no longer exist locally
    for remote_skill in &existing_remote_skills {
        if !local_skill_names.contains(remote_skill) {
            log::trace!(
                "Skills SSH sync removing orphan remote skill: skill_name={}",
                remote_skill
            );
            for tool_key in get_all_skill_tool_keys() {
                if let Some(remote_skills_dir) =
                    get_remote_tool_skills_dir_with_db(&db, tool_key).await
                {
                    let link_path = format!("{}/{}", remote_skills_dir, remote_skill);
                    if let Err(error) = remove_remote_path(session, &link_path).await {
                        log::warn!(
                            "Skills SSH sync failed to remove orphan tool symlink: tool_key={}, skill_name={}, link_path={}, error={}",
                            tool_key,
                            remote_skill,
                            link_path,
                            error
                        );
                    }
                }
            }
            let skill_path = format!("{}/{}", SSH_CENTRAL_DIR, remote_skill);
            if let Err(error) = remove_remote_path(session, &skill_path).await {
                log::warn!(
                    "Skills SSH sync failed to remove orphan remote skill directory: skill_name={}, skill_path={}, error={}",
                    remote_skill,
                    skill_path,
                    error
                );
            }
        }
    }

    // 4. Sync/update each skill
    let mut synced_count = 0;
    let mut all_errors: Vec<String> = vec![];
    for (idx, skill) in skills.iter().enumerate() {
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
            continue;
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

        // Ensure symlinks for each enabled tool
        for tool_key in &skill.enabled_tools {
            if let Some(remote_skills_dir) = get_remote_tool_skills_dir_with_db(&db, tool_key).await
            {
                let link_path = format!("{}/{}", remote_skills_dir, skill.name);
                if !check_remote_symlink_exists(session, &link_path, &remote_target).await {
                    if let Err(error) =
                        create_remote_symlink(session, &remote_target, &link_path).await
                    {
                        log::warn!(
                            "Skills SSH sync failed to create symlink: tool_key={}, skill_name={}, target={}, link_path={}, error={}",
                            tool_key,
                            skill.name,
                            remote_target,
                            link_path,
                            error
                        );
                    }
                } else {
                    log::trace!(
                        "Skills SSH sync symlink already correct: tool_key={}, skill_name={}, link_path={}",
                        tool_key,
                        skill.name,
                        link_path
                    );
                }
            }
        }

        // Remove symlinks for tools that are no longer enabled
        let enabled_set: HashSet<&str> = skill.enabled_tools.iter().map(|s| s.as_str()).collect();
        for tool_key in get_all_skill_tool_keys() {
            if !enabled_set.contains(tool_key) {
                if let Some(remote_skills_dir) =
                    get_remote_tool_skills_dir_with_db(&db, tool_key).await
                {
                    let link_path = format!("{}/{}", remote_skills_dir, skill.name);
                    log::trace!(
                        "Skills SSH sync removing disabled-tool symlink: tool_key={}, skill_name={}, link_path={}",
                        tool_key,
                        skill.name,
                        link_path
                    );
                    if let Err(error) = remove_remote_path(session, &link_path).await {
                        log::warn!(
                            "Skills SSH sync failed to remove disabled-tool symlink: tool_key={}, skill_name={}, link_path={}, error={}",
                            tool_key,
                            skill.name,
                            link_path,
                            error
                        );
                    }
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

    if !all_errors.is_empty() {
        return Err(all_errors.join("; "));
    }

    let _ = app.emit("ssh-skills-sync-completed", ());

    Ok(())
}
