use std::path::PathBuf;
use std::time::Duration;

use tauri::State;

use super::cache_cleanup::{cleanup_git_cache_dirs, get_git_cache_cleanup_days, set_git_cache_cleanup_days as set_cleanup_days, get_git_cache_ttl_secs};
use super::central_repo::{ensure_central_repo, expand_home_path, resolve_central_repo_path};
use super::git_fetcher::set_proxy;
use super::installer::{install_git_skill, install_git_skill_from_selection, install_local_skill, list_git_skills, update_managed_skill_from_source};
use super::onboarding::build_onboarding_plan;
use super::skill_store;
use super::sync_engine::{remove_path, sync_dir_for_tool_with_overwrite};
use super::tool_adapters::{adapter_by_key, get_all_tool_adapters, is_runtime_tool_installed, resolve_runtime_skills_path, runtime_adapter_by_key};
use super::adapter::parse_sync_details;
use super::types::{
    CustomTool, CustomToolDto, GitSkillCandidate, InstallResultDto, ManagedSkillDto, OnboardingPlan, SkillRepo, SkillRepoDto, SkillTarget,
    SkillTargetDto, SyncResultDto, ToolInfoDto, ToolStatusDto, UpdateResultDto, now_ms,
};
use crate::http_client;
use crate::DbState;

fn format_error(err: anyhow::Error) -> String {
    let first = err.to_string();
    // Frontend relies on these prefixes for special flows
    if first.starts_with("MULTI_SKILLS|")
        || first.starts_with("TARGET_EXISTS|")
        || first.starts_with("TOOL_NOT_INSTALLED|")
    {
        return first;
    }
    format!("{:#}", err)
}

// --- Tool Status ---

#[tauri::command]
pub async fn skills_get_tool_status(state: State<'_, DbState>) -> Result<ToolStatusDto, String> {
    // Get custom tools
    let custom_tools = skill_store::get_custom_tools(&state).await.unwrap_or_default();

    // Get all adapters (built-in + custom)
    let all_adapters = get_all_tool_adapters(&custom_tools);

    let mut tools: Vec<ToolInfoDto> = Vec::new();
    let mut installed: Vec<String> = Vec::new();

    for adapter in &all_adapters {
        let ok = is_runtime_tool_installed(adapter).unwrap_or(false);
        let skills_path = resolve_runtime_skills_path(adapter)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        tools.push(ToolInfoDto {
            key: adapter.key.clone(),
            label: adapter.display_name.clone(),
            installed: ok,
            skills_dir: skills_path,
        });
        if ok {
            installed.push(adapter.key.clone());
        }
    }

    installed.dedup();

    // Track newly installed tools
    let prev: Vec<String> = skill_store::get_setting(&state, "installed_tools_v1")
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default();

    let prev_set: std::collections::HashSet<String> = prev.into_iter().collect();
    let newly_installed: Vec<String> = installed
        .iter()
        .filter(|k| !prev_set.contains(*k))
        .cloned()
        .collect();

    // Persist current set
    let _ = skill_store::set_setting(
        &state,
        "installed_tools_v1",
        &serde_json::to_string(&installed).unwrap_or_else(|_| "[]".to_string()),
    )
    .await;

    Ok(ToolStatusDto {
        tools,
        installed,
        newly_installed,
    })
}

// --- Central Repo Path ---

#[tauri::command]
pub async fn skills_get_central_repo_path(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
) -> Result<String, String> {
    let path = resolve_central_repo_path(&app, &state).await.map_err(|e| format_error(e))?;
    ensure_central_repo(&path).map_err(|e| format_error(e))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn skills_set_central_repo_path(
    state: State<'_, DbState>,
    path: String,
) -> Result<String, String> {
    let new_base = expand_home_path(&path).map_err(|e| format_error(e))?;
    if !new_base.is_absolute() {
        return Err("storage path must be absolute".to_string());
    }
    ensure_central_repo(&new_base).map_err(|e| format_error(e))?;

    // Save new path to settings
    skill_store::set_setting(&state, "central_repo_path", new_base.to_string_lossy().as_ref())
        .await
        .map_err(|e| e)?;

    Ok(new_base.to_string_lossy().to_string())
}

// --- Managed Skills ---

#[tauri::command]
pub async fn skills_get_managed_skills(state: State<'_, DbState>) -> Result<Vec<ManagedSkillDto>, String> {
    let skills = skill_store::get_managed_skills(&state).await?;

    let mut result: Vec<ManagedSkillDto> = Vec::new();
    for skill in skills {
        let targets = parse_sync_details(&skill)
            .into_iter()
            .map(|t| SkillTargetDto {
                tool: t.tool,
                mode: t.mode,
                status: t.status,
                target_path: t.target_path,
                synced_at: t.synced_at,
            })
            .collect();

        result.push(ManagedSkillDto {
            id: skill.id,
            name: skill.name,
            source_type: skill.source_type,
            source_ref: skill.source_ref,
            central_path: skill.central_path,
            created_at: skill.created_at,
            updated_at: skill.updated_at,
            last_sync_at: skill.last_sync_at,
            status: skill.status,
            sort_index: skill.sort_index,
            enabled_tools: skill.enabled_tools,
            targets,
        });
    }

    Ok(result)
}

// --- Install Skills ---

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_install_local(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    sourcePath: String,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let result = install_local_skill(&app, &state, std::path::Path::new(&sourcePath), overwrite.unwrap_or(false))
        .await
        .map_err(|e| format_error(e))?;

    Ok(InstallResultDto {
        skill_id: result.skill_id,
        name: result.name,
        central_path: result.central_path.to_string_lossy().to_string(),
        content_hash: result.content_hash,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_install_git(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    repoUrl: String,
    branch: Option<String>,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let result = install_git_skill(&app, &state, &repoUrl, branch.as_deref(), overwrite.unwrap_or(false))
        .await
        .map_err(|e| format_error(e))?;

    Ok(InstallResultDto {
        skill_id: result.skill_id,
        name: result.name,
        central_path: result.central_path.to_string_lossy().to_string(),
        content_hash: result.content_hash,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_list_git_skills(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    repoUrl: String,
    branch: Option<String>,
) -> Result<Vec<GitSkillCandidate>, String> {
    // Initialize proxy from app settings
    let proxy_url = http_client::get_proxy_from_settings(&state).await.ok();
    set_proxy(proxy_url);

    let ttl = get_git_cache_ttl_secs(&state).await;
    let branch_clone = branch.clone();

    tokio::task::spawn_blocking(move || {
        list_git_skills(&app, ttl, &repoUrl, branch_clone.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format_error(e))
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_install_git_selection(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    repoUrl: String,
    subpath: String,
    branch: Option<String>,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let result = install_git_skill_from_selection(&app, &state, &repoUrl, &subpath, branch.as_deref(), overwrite.unwrap_or(false))
        .await
        .map_err(|e| format_error(e))?;

    Ok(InstallResultDto {
        skill_id: result.skill_id,
        name: result.name,
        central_path: result.central_path.to_string_lossy().to_string(),
        content_hash: result.content_hash,
    })
}

// --- Sync Skills ---

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_sync_to_tool(
    state: State<'_, DbState>,
    sourcePath: String,
    skillId: String,
    tool: String,
    name: String,
    overwrite: Option<bool>,
) -> Result<SyncResultDto, String> {
    // Get custom tools for runtime adapter lookup
    let custom_tools = skill_store::get_custom_tools(&state).await.unwrap_or_default();

    let runtime_adapter = runtime_adapter_by_key(&tool, &custom_tools)
        .ok_or_else(|| "unknown tool".to_string())?;

    // Skip install check for custom tools - they're always considered "installed"
    if !runtime_adapter.is_custom && !is_runtime_tool_installed(&runtime_adapter).unwrap_or(false) {
        let skills_path = resolve_runtime_skills_path(&runtime_adapter)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        return Err(format!("TOOL_NOT_INSTALLED|{}|{}", runtime_adapter.key, skills_path));
    }

    let tool_root = resolve_runtime_skills_path(&runtime_adapter).map_err(|e| format_error(e))?;
    let target = tool_root.join(&name);
    let overwrite = overwrite.unwrap_or(false);

    let result = sync_dir_for_tool_with_overwrite(&tool, std::path::Path::new(&sourcePath), &target, overwrite)
        .map_err(|err| {
            let msg = err.to_string();
            if msg.contains("target already exists") {
                format!("TARGET_EXISTS|{}", target.to_string_lossy())
            } else {
                format_error(err)
            }
        })?;

    let record = SkillTarget {
        tool: tool.clone(),
        target_path: result.target_path.to_string_lossy().to_string(),
        mode: result.mode_used.as_str().to_string(),
        status: "ok".to_string(),
        error_message: None,
        synced_at: Some(now_ms()),
    };
    skill_store::upsert_skill_target(&state, &skillId, &record).await?;

    Ok(SyncResultDto {
        mode_used: result.mode_used.as_str().to_string(),
        target_path: result.target_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_unsync_from_tool(
    state: State<'_, DbState>,
    skillId: String,
    tool: String,
) -> Result<(), String> {
    // Get custom tools for runtime adapter lookup
    let custom_tools = skill_store::get_custom_tools(&state).await.unwrap_or_default();

    // If the tool is not installed, do nothing
    if let Some(adapter) = runtime_adapter_by_key(&tool, &custom_tools) {
        if !is_runtime_tool_installed(&adapter).unwrap_or(false) {
            return Ok(());
        }
    }

    if let Some(target) = skill_store::get_skill_target(&state, &skillId, &tool).await? {
        // Remove the link/copy in tool directory first
        remove_path(&target.target_path)?;
        skill_store::delete_skill_target(&state, &skillId, &tool).await?;
    }

    Ok(())
}

// --- Update/Delete Skills ---

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_update_managed(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    skillId: String,
) -> Result<UpdateResultDto, String> {
    let res = update_managed_skill_from_source(&app, &state, &skillId)
        .await
        .map_err(|e| format_error(e))?;

    Ok(UpdateResultDto {
        skill_id: res.skill_id,
        name: res.name,
        content_hash: res.content_hash,
        source_revision: res.source_revision,
        updated_targets: res.updated_targets,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_delete_managed(
    state: State<'_, DbState>,
    skillId: String,
) -> Result<(), String> {
    // Delete synced targets first
    let targets = skill_store::get_skill_targets(&state, &skillId).await?;

    let mut remove_failures: Vec<String> = Vec::new();
    for target in targets {
        if let Err(err) = remove_path(&target.target_path) {
            remove_failures.push(format!("{}: {}", target.target_path, err));
        }
    }

    let record = skill_store::get_skill_by_id(&state, &skillId).await?;
    if let Some(skill) = record {
        let path = PathBuf::from(skill.central_path);
        if path.exists() {
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        }
        skill_store::delete_skill(&state, &skillId).await?;
    }

    if !remove_failures.is_empty() {
        return Err(format!(
            "Deleted managed record, but some tool directories could not be cleaned:\n- {}",
            remove_failures.join("\n- ")
        ));
    }

    Ok(())
}

// --- Onboarding ---

#[tauri::command]
pub async fn skills_get_onboarding_plan(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
) -> Result<OnboardingPlan, String> {
    build_onboarding_plan(&app, &state)
        .await
        .map_err(|e| format_error(e))
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_import_existing(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    sourcePath: String,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let result = install_local_skill(&app, &state, std::path::Path::new(&sourcePath), overwrite.unwrap_or(false))
        .await
        .map_err(|e| format_error(e))?;

    Ok(InstallResultDto {
        skill_id: result.skill_id,
        name: result.name,
        central_path: result.central_path.to_string_lossy().to_string(),
        content_hash: result.content_hash,
    })
}

// --- Git Cache ---

#[tauri::command]
pub async fn skills_get_git_cache_cleanup_days(state: State<'_, DbState>) -> Result<i64, String> {
    Ok(get_git_cache_cleanup_days(&state).await)
}

#[tauri::command]
pub async fn skills_set_git_cache_cleanup_days(
    state: State<'_, DbState>,
    days: i64,
) -> Result<i64, String> {
    set_cleanup_days(&state, days)
        .await
        .map_err(|e| format_error(e))
}

#[tauri::command]
pub async fn skills_get_git_cache_ttl_secs(state: State<'_, DbState>) -> Result<i64, String> {
    Ok(get_git_cache_ttl_secs(&state).await)
}

#[tauri::command]
pub async fn skills_clear_git_cache(app: tauri::AppHandle) -> Result<usize, String> {
    cleanup_git_cache_dirs(&app, Duration::from_secs(0)).map_err(|e| format_error(e))
}

#[tauri::command]
pub async fn skills_get_git_cache_path(app: tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let cache_dir = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    let cache_path = cache_dir.join("skills-git-cache");
    if !cache_path.exists() {
        std::fs::create_dir_all(&cache_path).map_err(|e| e.to_string())?;
    }
    Ok(cache_path.to_string_lossy().to_string())
}

// --- Preferred Tools ---

#[tauri::command]
pub async fn skills_get_preferred_tools(state: State<'_, DbState>) -> Result<Option<Vec<String>>, String> {
    let raw = skill_store::get_setting(&state, "preferred_tools_v1")
        .await
        .ok()
        .flatten();
    match raw {
        Some(s) => Ok(serde_json::from_str::<Vec<String>>(&s).ok()),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn skills_set_preferred_tools(
    state: State<'_, DbState>,
    tools: Vec<String>,
) -> Result<(), String> {
    skill_store::set_setting(
        &state,
        "preferred_tools_v1",
        &serde_json::to_string(&tools).unwrap_or_else(|_| "[]".to_string()),
    )
    .await
}

// --- Show Skills in Tray ---

#[tauri::command]
pub async fn skills_get_show_in_tray(state: State<'_, DbState>) -> Result<bool, String> {
    let raw = skill_store::get_setting(&state, "show_skills_in_tray")
        .await
        .ok()
        .flatten();
    match raw {
        Some(s) => Ok(s == "true"),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn skills_set_show_in_tray(
    state: State<'_, DbState>,
    enabled: bool,
) -> Result<(), String> {
    skill_store::set_setting(
        &state,
        "show_skills_in_tray",
        if enabled { "true" } else { "false" },
    )
    .await
}

// --- Custom Tools ---

#[tauri::command]
pub async fn skills_get_custom_tools(state: State<'_, DbState>) -> Result<Vec<CustomToolDto>, String> {
    let tools = skill_store::get_custom_tools(&state).await?;
    Ok(tools
        .into_iter()
        .map(|t| CustomToolDto {
            key: t.key,
            display_name: t.display_name,
            relative_skills_dir: t.relative_skills_dir,
            relative_detect_dir: t.relative_detect_dir,
            created_at: t.created_at,
        })
        .collect())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_add_custom_tool(
    state: State<'_, DbState>,
    key: String,
    displayName: String,
    relativeSkillsDir: String,
    relativeDetectDir: String,
) -> Result<(), String> {
    // Trim whitespace from all inputs
    let key = key.trim().to_string();
    let display_name = displayName.trim().to_string();
    // Also strip leading ~/ since paths are relative to home dir
    let relative_skills_dir = relativeSkillsDir.trim().trim_start_matches("~/").to_string();
    let relative_detect_dir = relativeDetectDir.trim().trim_start_matches("~/").to_string();

    // Validate key format
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Key must contain only letters, numbers, and underscores".to_string());
    }
    // Check for duplicate with built-in tools
    if adapter_by_key(&key).is_some() {
        return Err(format!("Key '{}' conflicts with a built-in tool", key));
    }

    let tool = CustomTool {
        key,
        display_name,
        relative_skills_dir,
        relative_detect_dir,
        created_at: now_ms(),
    };
    skill_store::save_custom_tool(&state, &tool).await
}

#[tauri::command]
pub async fn skills_remove_custom_tool(
    state: State<'_, DbState>,
    key: String,
) -> Result<(), String> {
    skill_store::delete_custom_tool(&state, &key).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_check_custom_tool_path(
    relativeSkillsDir: String,
) -> Result<bool, String> {
    let home = dirs::home_dir().ok_or_else(|| "failed to resolve home directory".to_string())?;
    let path = home.join(relativeSkillsDir.trim().trim_start_matches("~/"));
    Ok(path.exists())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_create_custom_tool_path(
    relativeSkillsDir: String,
) -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "failed to resolve home directory".to_string())?;
    let path = home.join(relativeSkillsDir.trim().trim_start_matches("~/"));
    std::fs::create_dir_all(&path).map_err(|e| format!("Failed to create directory: {}", e))?;
    Ok(())
}

// --- Skill Repos ---

// --- Reorder Skills ---

#[tauri::command]
pub async fn skills_reorder(
    state: State<'_, DbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    skill_store::reorder_skills(&state, &ids).await
}

// --- Skill Repos (cont.) ---

#[tauri::command]
pub async fn skills_get_repos(state: State<'_, DbState>) -> Result<Vec<SkillRepoDto>, String> {
    let repos = skill_store::get_skill_repos(&state).await?;
    Ok(repos
        .into_iter()
        .map(|r| SkillRepoDto {
            id: r.id,
            owner: r.owner,
            name: r.name,
            branch: r.branch,
            enabled: r.enabled,
            created_at: r.created_at,
        })
        .collect())
}

#[tauri::command]
pub async fn skills_add_repo(
    state: State<'_, DbState>,
    owner: String,
    name: String,
    branch: Option<String>,
) -> Result<(), String> {
    let repo = SkillRepo {
        id: format!("{}/{}", owner, name),
        owner,
        name,
        branch: branch.unwrap_or_else(|| "main".to_string()),
        enabled: true,
        created_at: now_ms(),
    };
    skill_store::save_skill_repo(&state, &repo).await
}

#[tauri::command]
pub async fn skills_remove_repo(
    state: State<'_, DbState>,
    owner: String,
    name: String,
) -> Result<(), String> {
    skill_store::delete_skill_repo(&state, &owner, &name).await
}

#[tauri::command]
pub async fn skills_init_default_repos(state: State<'_, DbState>) -> Result<usize, String> {
    let existing = skill_store::get_skill_repos(&state).await?;
    if !existing.is_empty() {
        return Ok(0);
    }

    let default_repos = vec![
        ("anthropics", "skills", "main"),
        ("ComposioHQ", "awesome-claude-skills", "master"),
        ("cexll", "myclaude", "master"),
        ("JimLiu", "baoyu-skills", "main"),
        ("nextlevelbuilder", "ui-ux-pro-max-skill", "main"),
    ];

    for (owner, name, branch) in &default_repos {
        let repo = SkillRepo {
            id: format!("{}/{}", owner, name),
            owner: owner.to_string(),
            name: name.to_string(),
            branch: branch.to_string(),
            enabled: true,
            created_at: now_ms(),
        };
        skill_store::save_skill_repo(&state, &repo).await?;
    }

    Ok(default_repos.len())
}

// --- Resync All Skills ---

/// Re-sync all skills to installed tools (used after restore)
#[tauri::command]
pub async fn skills_resync_all(
    state: State<'_, DbState>,
) -> Result<Vec<String>, String> {
    let custom_tools = skill_store::get_custom_tools(&state).await.unwrap_or_default();
    let skills = skill_store::get_managed_skills(&state).await?;

    let mut synced: Vec<String> = Vec::new();

    for skill in skills {
        let central_path = PathBuf::from(&skill.central_path);
        if !central_path.exists() {
            continue;
        }

        // Re-sync to each enabled tool
        for tool_key in &skill.enabled_tools {
            let runtime_adapter = match runtime_adapter_by_key(tool_key, &custom_tools) {
                Some(a) => a,
                None => continue,
            };

            // Skip if tool not installed (for non-custom tools)
            if !runtime_adapter.is_custom && !is_runtime_tool_installed(&runtime_adapter).unwrap_or(false) {
                continue;
            }

            let tool_root = match resolve_runtime_skills_path(&runtime_adapter) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let target = tool_root.join(&skill.name);

            // Sync with overwrite
            if let Ok(result) = sync_dir_for_tool_with_overwrite(tool_key, &central_path, &target, true) {
                let record = SkillTarget {
                    tool: tool_key.clone(),
                    target_path: result.target_path.to_string_lossy().to_string(),
                    mode: result.mode_used.as_str().to_string(),
                    status: "ok".to_string(),
                    error_message: None,
                    synced_at: Some(now_ms()),
                };
                let _ = skill_store::upsert_skill_target(&state, &skill.id, &record).await;
                synced.push(format!("{}:{}", skill.name, tool_key));
            }
        }
    }

    Ok(synced)
}
