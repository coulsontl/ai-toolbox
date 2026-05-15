use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Runtime, State};

use super::adapter::parse_sync_details;
use super::cache_cleanup::{
    cleanup_git_cache_dirs, get_git_cache_cleanup_days, get_git_cache_ttl_secs,
    set_git_cache_cleanup_days as set_cleanup_days,
};
use super::central_repo::{
    ensure_central_repo, expand_home_path, resolve_central_repo_path, resolve_skill_central_path,
    save_central_repo_path,
};
use super::git_fetcher::{set_proxy, GitProxyMode};
use super::installer::{
    install_git_skill, install_git_skill_from_selection, install_local_skill,
    install_local_skill_from_selection, list_git_skills, list_local_skills,
    update_managed_skill_from_source,
};
use super::onboarding::build_onboarding_plan;
use super::path_executor::{
    remove_skill_target_checked, sync_skill_to_target, target_path_changed,
};
use super::skill_store;
use super::tool_adapters::{
    adapter_by_key, get_all_tool_adapters, is_tool_installed_async,
    resolve_runtime_skills_path_async, runtime_adapter_by_key,
};
use super::types::{
    now_ms, CustomTool, CustomToolDto, GitSkillCandidate, InstallResultDto, ManagedSkillDto,
    ManagedSkillSummaryDto, OnboardingPlan, Skill, SkillGroupDto, SkillGroupRecord,
    SkillInventoryGroupJson, SkillInventoryJson, SkillInventoryPreviewDto, SkillInventorySkillJson,
    SkillRepo, SkillRepoDto, SkillTarget, SkillTargetDto, SyncResultDto, ToolInfoDto,
    ToolStatusDto, UpdateResultDto,
};
use crate::coding::runtime_location;
use crate::http_client;
use crate::DbState;

fn format_error(err: anyhow::Error) -> String {
    let first = err.to_string();
    // Frontend relies on these prefixes for special flows
    if first.starts_with("MULTI_SKILLS|")
        || first.starts_with("TARGET_EXISTS|")
        || first.starts_with("TOOL_NOT_INSTALLED|")
        || first.starts_with("SKILL_DISABLED|")
    {
        return first;
    }
    format!("{:#}", err)
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_optional_id(value: Option<String>) -> Option<String> {
    normalize_optional_text(value)
}

fn normalize_tool_ids(tools: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    tools
        .iter()
        .filter_map(|tool| normalize_optional_text(Some(tool.clone())))
        .filter(|tool| seen.insert(tool.clone()))
        .collect()
}

#[derive(Clone)]
struct DescriptionCacheEntry {
    content_hash: Option<String>,
    description: Option<String>,
}

static DESCRIPTION_CACHE: OnceLock<Mutex<HashMap<String, DescriptionCacheEntry>>> = OnceLock::new();

fn read_skill_description(central_path: &Path, content_hash: &Option<String>) -> Option<String> {
    let key = central_path.to_string_lossy().to_string();
    let cache = DESCRIPTION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(entry) = guard.get(&key) {
            if &entry.content_hash == content_hash {
                return entry.description.clone();
            }
        }
    }

    let description = parse_skill_md_description(&central_path.join("SKILL.md"));
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            key,
            DescriptionCacheEntry {
                content_hash: content_hash.clone(),
                description: description.clone(),
            },
        );
    }
    description
}

fn parse_skill_md_description(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            let description = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !description.is_empty() {
                return Some(description.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillSourceDiagnosis {
    health: String,
    error: Option<String>,
}

fn diagnose_skill_source_path(path: &Path) -> SkillSourceDiagnosis {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return SkillSourceDiagnosis {
                health: "warning".to_string(),
                error: Some(format!(
                    "Source path is missing. Restore or reinstall this Skill: {}",
                    path.display()
                )),
            };
        }
        Err(_) => {
            return SkillSourceDiagnosis {
                health: "warning".to_string(),
                error: Some(format!(
                    "Source path cannot be inspected. Restore or reinstall this Skill: {}",
                    path.display()
                )),
            };
        }
    }

    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => SkillSourceDiagnosis {
            health: "ok".to_string(),
            error: None,
        },
        Ok(_) => SkillSourceDiagnosis {
            health: "warning".to_string(),
            error: Some(format!(
                "Source path is not a directory. Restore or reinstall this Skill: {}",
                path.display()
            )),
        },
        Err(_) => SkillSourceDiagnosis {
            health: "warning".to_string(),
            error: Some(format!(
                "Source path is not a resolvable directory. Restore or reinstall this Skill: {}",
                path.display()
            )),
        },
    }
}

fn group_to_dto(group: SkillGroupRecord) -> SkillGroupDto {
    SkillGroupDto {
        id: group.id,
        name: group.name,
        note: group.note,
        sort_index: group.sort_index,
        created_at: group.created_at,
        updated_at: group.updated_at,
    }
}

// --- Tool Status ---

#[tauri::command]
pub async fn skills_get_tool_status(state: State<'_, DbState>) -> Result<ToolStatusDto, String> {
    super::tool_adapters::set_runtime_db(state.db());

    // Get custom tools
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();

    // Get all adapters (built-in + custom)
    let all_adapters = get_all_tool_adapters(&custom_tools);

    let mut tools: Vec<ToolInfoDto> = Vec::new();
    let mut installed: Vec<String> = Vec::new();

    for adapter in &all_adapters {
        let ok = is_tool_installed_async(adapter).await.unwrap_or(false);
        let skills_path = resolve_runtime_skills_path_async(adapter)
            .await
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        tools.push(ToolInfoDto {
            key: adapter.key.clone(),
            label: adapter.display_name.clone(),
            installed: ok,
            skills_dir: skills_path,
        });
        // Only track built-in tools for "installed" detection
        // Custom tools are always "installed" but shouldn't trigger save
        if ok && !adapter.is_custom {
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

    // Persist current set in background (only if changed, to reduce lock contention)
    // This doesn't block the command return
    let current_set: std::collections::HashSet<String> = installed.iter().cloned().collect();
    if current_set != prev_set {
        let installed_clone = installed.clone();
        let state_ref = DbState(state.0.clone());
        tokio::spawn(async move {
            // Small delay to let other operations complete first
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = skill_store::set_setting(
                &state_ref,
                "installed_tools_v1",
                &serde_json::to_string(&installed_clone).unwrap_or_else(|_| "[]".to_string()),
            )
            .await;
        });
    }

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
    let path = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;
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

    // Save new path to the same authoritative store used by resolve_central_repo_path.
    save_central_repo_path(&state, &new_base)
        .await
        .map_err(|e| format_error(e))?;

    Ok(new_base.to_string_lossy().to_string())
}

// --- Managed Skills ---

#[tauri::command]
pub async fn skills_get_managed_skills(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
) -> Result<Vec<ManagedSkillDto>, String> {
    let skills = skill_store::get_managed_skills(&state).await?;
    let groups = skill_store::get_skill_groups(&state).await?;
    let group_names: HashMap<String, String> = groups
        .into_iter()
        .map(|group| (group.id, group.name))
        .collect();
    let central_dir = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;

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

        // Resolve central_path to absolute for frontend use
        let resolved_path = resolve_skill_central_path(&skill.central_path, &central_dir);
        let user_group = skill
            .group_id
            .as_ref()
            .and_then(|group_id| group_names.get(group_id).cloned())
            .or(skill.user_group.clone());
        let description = read_skill_description(&resolved_path, &skill.content_hash);
        let source_diagnosis = diagnose_skill_source_path(&resolved_path);

        result.push(ManagedSkillDto {
            id: skill.id,
            name: skill.name,
            source_type: skill.source_type,
            source_ref: skill.source_ref,
            central_path: resolved_path.to_string_lossy().to_string(),
            created_at: skill.created_at,
            updated_at: skill.updated_at,
            last_sync_at: skill.last_sync_at,
            status: skill.status,
            sort_index: skill.sort_index,
            user_group,
            group_id: skill.group_id,
            user_note: skill.user_note,
            management_enabled: skill.management_enabled,
            disabled_previous_tools: skill.disabled_previous_tools,
            description,
            content_hash: skill.content_hash,
            source_health: source_diagnosis.health,
            source_error: source_diagnosis.error,
            enabled_tools: skill.enabled_tools,
            targets,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod skill_source_tests {
    use super::*;

    #[test]
    fn skill_source_diagnose_valid_dir_is_ok() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        std::fs::create_dir(&source).expect("create source dir");

        let diagnosis = diagnose_skill_source_path(&source);

        assert_eq!(diagnosis.health, "ok");
        assert_eq!(diagnosis.error, None);
    }

    #[test]
    fn skill_source_diagnose_missing_is_warning() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("missing");

        let diagnosis = diagnose_skill_source_path(&source);

        assert_eq!(diagnosis.health, "warning");
        assert!(diagnosis
            .error
            .as_deref()
            .expect("source error")
            .contains("missing"));
    }

    #[test]
    fn skill_source_diagnose_file_is_warning() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source.txt");
        std::fs::write(&source, "not a directory").expect("write source file");

        let diagnosis = diagnose_skill_source_path(&source);

        assert_eq!(diagnosis.health, "warning");
        assert!(diagnosis
            .error
            .as_deref()
            .expect("source error")
            .contains("not a directory"));
    }

    #[cfg(unix)]
    #[test]
    fn skill_source_diagnose_self_symlink_is_warning() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        std::os::unix::fs::symlink(&source, &source).expect("create self symlink");

        let diagnosis = diagnose_skill_source_path(&source);

        assert_eq!(diagnosis.health, "warning");
        assert!(diagnosis
            .error
            .as_deref()
            .expect("source error")
            .contains("not a resolvable directory"));
    }

    #[cfg(unix)]
    #[test]
    fn skill_source_diagnose_broken_symlink_is_warning() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let missing = temp.path().join("missing");
        std::os::unix::fs::symlink(&missing, &source).expect("create broken symlink");

        let diagnosis = diagnose_skill_source_path(&source);

        assert_eq!(diagnosis.health, "warning");
        assert!(diagnosis
            .error
            .as_deref()
            .expect("source error")
            .contains("not a resolvable directory"));
    }
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
    let result = install_local_skill(
        &app,
        &state,
        std::path::Path::new(&sourcePath),
        overwrite.unwrap_or(false),
    )
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
pub async fn skills_list_local_skills(
    sourcePath: String,
) -> Result<Vec<GitSkillCandidate>, String> {
    let source = std::path::Path::new(&sourcePath);
    list_local_skills(source).map_err(|e| format_error(e))
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_install_local_selection(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    sourcePath: String,
    subpath: String,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let result = install_local_skill_from_selection(
        &app,
        &state,
        std::path::Path::new(&sourcePath),
        &subpath,
        overwrite.unwrap_or(false),
    )
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
    let result = install_git_skill(
        &app,
        &state,
        &repoUrl,
        branch.as_deref(),
        overwrite.unwrap_or(false),
    )
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
    let proxy_result = http_client::get_proxy_from_settings(&state).await.ok();
    let proxy_mode = match proxy_result {
        Some((http_client::ProxyMode::Direct, _)) => GitProxyMode::Direct,
        Some((http_client::ProxyMode::Custom, url)) if !url.is_empty() => GitProxyMode::Custom(url),
        _ => GitProxyMode::System,
    };
    set_proxy(proxy_mode);

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
    let result = install_git_skill_from_selection(
        &app,
        &state,
        &repoUrl,
        &subpath,
        branch.as_deref(),
        overwrite.unwrap_or(false),
    )
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

async fn resolve_skill_source_path<R: Runtime>(
    app: &AppHandle<R>,
    state: &DbState,
    skill: &Skill,
) -> Result<PathBuf, String> {
    let central_dir = resolve_central_repo_path(app, state)
        .await
        .map_err(|e| format_error(e))?;
    Ok(resolve_skill_central_path(
        &skill.central_path,
        &central_dir,
    ))
}

async fn sync_skill_to_tool_record(
    state: &DbState,
    skill: &Skill,
    tool: &str,
    source_path: &Path,
    overwrite: bool,
    custom_tools: &[CustomTool],
) -> Result<SyncResultDto, String> {
    let runtime_adapter =
        runtime_adapter_by_key(tool, custom_tools).ok_or_else(|| "unknown tool".to_string())?;

    // Skip install check for custom tools - they're always considered "installed"
    if !runtime_adapter.is_custom
        && !is_tool_installed_async(&runtime_adapter)
            .await
            .unwrap_or(false)
    {
        let skills_path = resolve_runtime_skills_path_async(&runtime_adapter)
            .await
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        return Err(format!(
            "TOOL_NOT_INSTALLED|{}|{}",
            runtime_adapter.key, skills_path
        ));
    }

    let tool_root = resolve_runtime_skills_path_async(&runtime_adapter)
        .await
        .map_err(|e| format_error(e))?;
    let target = tool_root.join(&skill.name);
    let previous_target = skill_store::get_skill_target(state, &skill.id, tool).await?;

    let result = sync_skill_to_target(
        tool,
        source_path,
        &target,
        overwrite,
        runtime_adapter.force_copy,
    )
    .map_err(|err| {
        let msg = err.to_string();
        if msg.contains("target already exists") {
            format!("TARGET_EXISTS|{}", target.to_string_lossy())
        } else {
            format_error(err)
        }
    })?;

    if let Some(existing_target) = previous_target.as_ref() {
        if target_path_changed(&existing_target.target_path, &target) {
            remove_skill_target_checked(source_path, &existing_target.target_path)
                .map_err(format_error)?;
        }
    }

    let record = SkillTarget {
        tool: tool.to_string(),
        target_path: result.target_path.to_string_lossy().to_string(),
        mode: result.mode_used.as_str().to_string(),
        status: "ok".to_string(),
        error_message: None,
        synced_at: Some(now_ms()),
    };
    skill_store::upsert_skill_target(state, &skill.id, &record).await?;

    Ok(SyncResultDto {
        mode_used: result.mode_used.as_str().to_string(),
        target_path: result.target_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_sync_to_tool<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    sourcePath: String,
    skillId: String,
    tool: String,
    name: String,
    overwrite: Option<bool>,
) -> Result<SyncResultDto, String> {
    let skill = skill_store::get_skill_by_id(&state, &skillId)
        .await?
        .ok_or_else(|| format!("Skill not found: {}", skillId))?;
    if !skill.management_enabled {
        return Err(format!("SKILL_DISABLED|{}", skillId));
    }
    // Backward-compatible API fields only. The real sync source and skill name
    // must come from the DB record + central repo resolver, never from frontend
    // payloads that may be stale or point at a tool runtime directory.
    let _ = (&sourcePath, &name);
    let source_path = resolve_skill_source_path(&app, &state, &skill).await?;

    // Get custom tools for runtime adapter lookup
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let overwrite = overwrite.unwrap_or(false);
    let result = sync_skill_to_tool_record(
        &state,
        &skill,
        &tool,
        &source_path,
        overwrite,
        &custom_tools,
    )
    .await?;

    // Emit skills-changed for WSL sync
    let _ = app.emit("skills-changed", "window");

    Ok(result)
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_unsync_from_tool<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    skillId: String,
    tool: String,
) -> Result<(), String> {
    // Get custom tools for runtime adapter lookup
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();

    // If the tool is not installed, do nothing
    if let Some(adapter) = runtime_adapter_by_key(&tool, &custom_tools) {
        if !is_tool_installed_async(&adapter).await.unwrap_or(false) {
            return Ok(());
        }
    }

    if let Some(target) = skill_store::get_skill_target(&state, &skillId, &tool).await? {
        let skill = skill_store::get_skill_by_id(&state, &skillId)
            .await?
            .ok_or_else(|| format!("Skill not found: {}", skillId))?;
        let source_path = resolve_skill_source_path(&app, &state, &skill).await?;
        // Remove the link/copy in tool directory first
        remove_skill_target_checked(&source_path, &target.target_path).map_err(format_error)?;
        skill_store::delete_skill_target(&state, &skillId, &tool).await?;
    }

    // Emit skills-changed for WSL sync
    let _ = app.emit("skills-changed", "window");

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

    // Emit skills-changed for WSL sync
    let _ = app.emit("skills-changed", "window");

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
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    skillId: String,
) -> Result<(), String> {
    let record = skill_store::get_skill_by_id(&state, &skillId).await?;
    let mut remove_failures: Vec<String> = Vec::new();
    if let Some(skill) = record {
        // Resolve central_path (handles cross-platform legacy paths)
        let central_dir = resolve_central_repo_path(&app, &state)
            .await
            .map_err(|e| format_error(e))?;
        let path = resolve_skill_central_path(&skill.central_path, &central_dir);
        let targets = skill_store::get_skill_targets(&state, &skillId).await?;
        for target in targets {
            if let Err(err) = remove_skill_target_checked(&path, &target.target_path) {
                remove_failures.push(format!("{}: {}", target.target_path, err));
            }
        }
        if path.exists() {
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        }
        skill_store::delete_skill(&state, &skillId).await?;
    }

    // Emit skills-changed for WSL sync
    let _ = app.emit("skills-changed", "window");

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
    // Add 30 second timeout to prevent hanging on large directories
    match tokio::time::timeout(Duration::from_secs(30), build_onboarding_plan(&app, &state)).await {
        Ok(result) => result.map_err(|e| format_error(e)),
        Err(_) => {
            Err("Scan timed out after 30 seconds. Please check your custom tool paths.".to_string())
        }
    }
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_import_existing(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    sourcePath: String,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let result = install_local_skill(
        &app,
        &state,
        std::path::Path::new(&sourcePath),
        overwrite.unwrap_or(false),
    )
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
pub async fn skills_get_preferred_tools(
    state: State<'_, DbState>,
) -> Result<Option<Vec<String>>, String> {
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

// --- Default View Mode ---

#[tauri::command]
pub async fn skills_get_default_view_mode(state: State<'_, DbState>) -> Result<String, String> {
    let raw = skill_store::get_setting(&state, "default_view_mode")
        .await
        .ok()
        .flatten();
    match raw.as_deref() {
        Some("grouped") => Ok("grouped".to_string()),
        _ => Ok("flat".to_string()),
    }
}

#[tauri::command]
pub async fn skills_set_default_view_mode(
    state: State<'_, DbState>,
    mode: String,
) -> Result<(), String> {
    skill_store::set_setting(&state, "default_view_mode", &mode).await
}

// --- Custom Tools ---

#[tauri::command]
pub async fn skills_get_custom_tools(
    state: State<'_, DbState>,
) -> Result<Vec<CustomToolDto>, String> {
    let tools = skill_store::get_custom_tools(&state).await?;
    Ok(tools
        .into_iter()
        .map(|t| CustomToolDto {
            key: t.key,
            display_name: t.display_name,
            relative_skills_dir: t.relative_skills_dir,
            relative_detect_dir: t.relative_detect_dir,
            created_at: t.created_at,
            force_copy: t.force_copy,
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
    forceCopy: Option<bool>,
) -> Result<(), String> {
    use crate::coding::tools::path_utils::{normalize_path, to_storage_path};

    // Trim whitespace from all inputs
    let key = key.trim().to_string();
    let display_name = displayName.trim().to_string();

    // Normalize paths using the new path utility
    let normalized_skills = normalize_path(relativeSkillsDir.trim());
    let normalized_detect = normalize_path(relativeDetectDir.trim());
    let relative_skills_dir = to_storage_path(&normalized_skills);
    let relative_detect_dir = to_storage_path(&normalized_detect);

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
        force_copy: forceCopy.unwrap_or(false),
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
pub async fn skills_check_custom_tool_path(relativeSkillsDir: String) -> Result<bool, String> {
    use crate::coding::tools::path_utils::{normalize_path, resolve_storage_path, to_storage_path};

    // Normalize the path first to get the storage format
    let normalized = normalize_path(relativeSkillsDir.trim());
    let storage_path = to_storage_path(&normalized);

    // Resolve to absolute path
    let path =
        resolve_storage_path(&storage_path).ok_or_else(|| "failed to resolve path".to_string())?;
    Ok(path.exists())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_create_custom_tool_path(relativeSkillsDir: String) -> Result<(), String> {
    use crate::coding::tools::path_utils::{normalize_path, resolve_storage_path, to_storage_path};

    // Normalize the path first to get the storage format
    let normalized = normalize_path(relativeSkillsDir.trim());
    let storage_path = to_storage_path(&normalized);

    // Resolve to absolute path
    let path =
        resolve_storage_path(&storage_path).ok_or_else(|| "failed to resolve path".to_string())?;
    std::fs::create_dir_all(&path).map_err(|e| format!("Failed to create directory: {}", e))?;
    Ok(())
}

// --- Skill Repos ---

// --- Reorder Skills ---

#[tauri::command]
pub async fn skills_get_groups(state: State<'_, DbState>) -> Result<Vec<SkillGroupDto>, String> {
    let groups = skill_store::get_skill_groups(&state).await?;
    Ok(groups.into_iter().map(group_to_dto).collect())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_save_group(
    state: State<'_, DbState>,
    id: Option<String>,
    name: String,
    note: Option<String>,
    sortIndex: Option<i32>,
) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Group name is required".to_string());
    }
    let normalized_id = normalize_optional_id(id);
    let existing_groups = skill_store::get_skill_groups(&state).await?;
    if existing_groups.iter().any(|group| {
        group.name.trim().eq_ignore_ascii_case(&name)
            && normalized_id.as_deref() != Some(group.id.as_str())
    }) {
        return Err(format!("Duplicate group name: {}", name));
    }
    let now = now_ms();
    let existing_group = normalized_id
        .as_ref()
        .and_then(|group_id| existing_groups.iter().find(|group| group.id == *group_id));
    let group = SkillGroupRecord {
        id: normalized_id.unwrap_or_default(),
        name,
        note: normalize_optional_text(note),
        sort_index: sortIndex.unwrap_or(0),
        created_at: existing_group.map(|group| group.created_at).unwrap_or(now),
        updated_at: now,
    };
    skill_store::save_skill_group(&state, &group).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_delete_group(state: State<'_, DbState>, groupId: String) -> Result<(), String> {
    skill_store::delete_skill_group(&state, &groupId).await
}

#[tauri::command]
pub async fn skills_reorder(state: State<'_, DbState>, ids: Vec<String>) -> Result<(), String> {
    skill_store::reorder_skills(&state, &ids).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_update_metadata(
    state: State<'_, DbState>,
    skillId: String,
    groupId: Option<String>,
    userNote: Option<String>,
) -> Result<(), String> {
    skill_store::update_skill_metadata(
        &state,
        &skillId,
        normalize_optional_id(groupId),
        normalize_optional_text(userNote),
    )
    .await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_batch_update_group(
    state: State<'_, DbState>,
    skillIds: Vec<String>,
    groupId: Option<String>,
) -> Result<(), String> {
    skill_store::update_skills_group(&state, &skillIds, normalize_optional_id(groupId)).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_set_management_enabled<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    skillId: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    if !enabled {
        if let Some(skill) = skill_store::get_skill_by_id(&state, &skillId).await? {
            let previous_tools = if skill.enabled_tools.is_empty() {
                skill.disabled_previous_tools.clone()
            } else {
                skill.enabled_tools.clone()
            };
            skill_store::record_disabled_previous_tools(&state, &skillId, previous_tools).await?;
            let source_path = resolve_skill_source_path(&app, &state, &skill).await?;
            let targets = skill_store::get_skill_targets(&state, &skillId).await?;
            for target in targets {
                remove_skill_target_checked(&source_path, &target.target_path)
                    .map_err(format_error)?;
            }
        }
    }
    let previous = skill_store::set_skill_management_enabled(&state, &skillId, enabled).await?;
    let _ = app.emit("skills-changed", "window");
    Ok(previous)
}

#[tauri::command]
pub async fn skills_export_inventory(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
) -> Result<String, String> {
    build_inventory_json(&app, &state).await
}

#[tauri::command]
pub async fn skills_export_inventory_file(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
) -> Result<String, String> {
    let json = build_inventory_json(&app, &state).await?;
    let path = default_inventory_export_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create export directory: {}", e))?;
    }
    std::fs::write(&path, json).map_err(|e| format!("Failed to write inventory file: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

async fn build_inventory_json(app: &tauri::AppHandle, state: &DbState) -> Result<String, String> {
    let groups = skill_store::get_skill_groups(state).await?;
    let skills = skill_store::get_managed_skills(state).await?;
    let central_dir = resolve_central_repo_path(app, state)
        .await
        .map_err(|e| format_error(e))?;
    let group_by_id: HashMap<String, String> = groups
        .iter()
        .map(|group| (group.id.clone(), group.name.clone()))
        .collect();
    let inventory = SkillInventoryJson {
        schema_version: 1,
        exported_at: now_ms(),
        groups: groups
            .into_iter()
            .map(|group| SkillInventoryGroupJson {
                name: group.name,
                note: group.note,
                order: group.sort_index,
            })
            .collect(),
        skills: skills
            .into_iter()
            .map(|skill| {
                let resolved_path = resolve_skill_central_path(&skill.central_path, &central_dir);
                SkillInventorySkillJson {
                    id: Some(skill.id),
                    name: skill.name,
                    group: skill
                        .group_id
                        .as_ref()
                        .and_then(|group_id| group_by_id.get(group_id).cloned())
                        .or(skill.user_group),
                    user_note: skill.user_note,
                    order: skill.sort_index,
                    enabled: skill.management_enabled,
                    enabled_tools: skill.enabled_tools,
                    previous_enabled_tools: skill.disabled_previous_tools,
                    source_type: skill.source_type,
                    source_ref: skill.source_ref,
                    central_path: resolved_path.to_string_lossy().to_string(),
                    content_hash: skill.content_hash,
                }
            })
            .collect(),
    };
    serde_json::to_string_pretty(&inventory)
        .map_err(|e| format!("Failed to serialize inventory: {}", e))
}

fn default_inventory_export_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    Ok(home.join(format!("skill-group-{}.json", now_ms())))
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_preview_inventory_import(
    state: State<'_, DbState>,
    inventoryJson: String,
) -> Result<SkillInventoryPreviewDto, String> {
    preview_inventory_import(&state, &inventoryJson).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_preview_inventory_import_file(
    state: State<'_, DbState>,
    filePath: String,
) -> Result<SkillInventoryPreviewDto, String> {
    let raw = read_inventory_file(&filePath)?;
    preview_inventory_import(&state, &raw).await
}

async fn reconcile_inventory_skill_tools<R: Runtime>(
    app: &AppHandle<R>,
    state: &DbState,
    skill_id: &str,
    desired_tools: &[String],
    custom_tools: &[CustomTool],
) -> Result<(), String> {
    let desired_tools = normalize_tool_ids(desired_tools);
    let desired_tool_set: HashSet<String> = desired_tools.iter().cloned().collect();
    let current_targets = skill_store::get_skill_targets(state, skill_id).await?;
    let current_tool_set: HashSet<String> = current_targets
        .iter()
        .map(|target| target.tool.clone())
        .collect();
    let skill = skill_store::get_skill_by_id(state, skill_id)
        .await?
        .ok_or_else(|| format!("Skill not found: {}", skill_id))?;
    let source_path = resolve_skill_source_path(app, state, &skill).await?;

    for target in current_targets {
        if desired_tool_set.contains(&target.tool) {
            continue;
        }
        remove_skill_target_checked(&source_path, &target.target_path).map_err(format_error)?;
        skill_store::delete_skill_target(state, skill_id, &target.tool).await?;
    }

    if desired_tools.is_empty() {
        return Ok(());
    }

    if !skill.management_enabled {
        return Err(format!("SKILL_DISABLED|{}", skill_id));
    }

    for tool in desired_tools {
        if current_tool_set.contains(&tool) {
            continue;
        }
        sync_skill_to_tool_record(state, &skill, &tool, &source_path, true, custom_tools).await?;
    }

    Ok(())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_apply_inventory_import<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    inventoryJson: String,
) -> Result<SkillInventoryPreviewDto, String> {
    let preview = preview_inventory_import(&state, &inventoryJson).await?;
    if !preview.valid {
        return Ok(preview);
    }

    let inventory = parse_inventory(&inventoryJson)?;
    let now = now_ms();
    let groups: Vec<SkillGroupRecord> = inventory
        .groups
        .iter()
        .map(|group| SkillGroupRecord {
            id: crate::coding::db_id::db_new_id(),
            name: group.name.trim().to_string(),
            note: group
                .note
                .as_ref()
                .and_then(|note| normalize_optional_text(Some(note.clone()))),
            sort_index: group.order,
            created_at: now,
            updated_at: now,
        })
        .collect();
    let saved_groups = skill_store::replace_skill_groups(&state, &groups).await?;
    let group_id_by_name: HashMap<String, String> = saved_groups
        .into_iter()
        .map(|group| (group.name.to_lowercase(), group.id))
        .collect();
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();

    let local_skills = skill_store::get_managed_skills(&state).await?;
    let mut matched_ids = HashSet::new();
    for item in &inventory.skills {
        let Some(skill) = match_inventory_skill(item, &local_skills) else {
            continue;
        };
        matched_ids.insert(skill.id.clone());
        let group_id = item
            .group
            .as_ref()
            .and_then(|name| group_id_by_name.get(&name.trim().to_lowercase()).cloned());
        skill_store::update_skill_metadata(
            &state,
            &skill.id,
            group_id,
            normalize_optional_text(item.user_note.clone()),
        )
        .await?;
        skill_store::update_skill_sort_index(&state, &skill.id, item.order).await?;
        if item.enabled {
            skill_store::set_skill_management_enabled(&state, &skill.id, true).await?;
            reconcile_inventory_skill_tools(
                &app,
                &state,
                &skill.id,
                &item.enabled_tools,
                &custom_tools,
            )
            .await?;
        } else {
            let source_path = resolve_skill_source_path(&app, &state, &skill).await?;
            for target in skill_store::get_skill_targets(&state, &skill.id).await? {
                remove_skill_target_checked(&source_path, &target.target_path)
                    .map_err(format_error)?;
            }
            let previous_tools = if item.previous_enabled_tools.is_empty() {
                normalize_tool_ids(&item.enabled_tools)
            } else {
                normalize_tool_ids(&item.previous_enabled_tools)
            };
            skill_store::disable_skill_with_previous_tools(&state, &skill.id, previous_tools)
                .await?;
        }
    }

    for skill in local_skills
        .iter()
        .filter(|skill| !matched_ids.contains(&skill.id))
    {
        let source_path = resolve_skill_source_path(&app, &state, skill).await?;
        for target in skill_store::get_skill_targets(&state, &skill.id).await? {
            remove_skill_target_checked(&source_path, &target.target_path).map_err(format_error)?;
        }
        skill_store::set_skill_management_enabled(&state, &skill.id, false).await?;
        skill_store::update_skill_metadata(&state, &skill.id, None, skill.user_note.clone())
            .await?;
    }

    let _ = app.emit("skills-changed", "window");
    Ok(preview)
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_apply_inventory_import_file<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DbState>,
    filePath: String,
) -> Result<SkillInventoryPreviewDto, String> {
    let raw = read_inventory_file(&filePath)?;
    skills_apply_inventory_import(app, state, raw).await
}

fn read_inventory_file(file_path: &str) -> Result<String, String> {
    let path = PathBuf::from(file_path);
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read inventory file: {}", e))
}

fn parse_inventory(raw: &str) -> Result<SkillInventoryJson, String> {
    let inventory: SkillInventoryJson =
        serde_json::from_str(raw).map_err(|e| format!("Invalid inventory JSON: {}", e))?;
    if inventory.schema_version != 1 {
        return Err(format!(
            "Unsupported inventory schema version: {}",
            inventory.schema_version
        ));
    }
    Ok(inventory)
}

async fn preview_inventory_import(
    state: &DbState,
    raw: &str,
) -> Result<SkillInventoryPreviewDto, String> {
    let inventory = match parse_inventory(raw) {
        Ok(value) => value,
        Err(error) => {
            return Ok(SkillInventoryPreviewDto {
                valid: false,
                errors: vec![error],
                group_count: 0,
                matched_skill_count: 0,
                unmatched_inventory_skills: Vec::new(),
                local_missing_from_inventory: Vec::new(),
                default_disable_count: 0,
                content_changed_count: 0,
            })
        }
    };

    let mut errors = Vec::new();
    let mut group_names = HashSet::new();
    for group in &inventory.groups {
        let name = group.name.trim().to_lowercase();
        if name.is_empty() {
            errors.push("Group name is required".to_string());
        } else if !group_names.insert(name) {
            errors.push(format!("Duplicate group name: {}", group.name));
        }
    }
    for item in &inventory.skills {
        if let Some(group) = &item.group {
            if !group.trim().is_empty() && !group_names.contains(&group.trim().to_lowercase()) {
                errors.push(format!(
                    "Skill '{}' references unknown group '{}'",
                    item.name, group
                ));
            }
        }
    }

    let local_skills = skill_store::get_managed_skills(state).await?;
    let mut matched_ids = HashSet::new();
    let mut unmatched = Vec::new();
    let mut content_changed_count = 0;
    for item in &inventory.skills {
        if let Some(skill) = match_inventory_skill(item, &local_skills) {
            if item.content_hash.is_some() && skill.content_hash != item.content_hash {
                content_changed_count += 1;
            }
            matched_ids.insert(skill.id.clone());
        } else {
            unmatched.push(item.name.clone());
        }
    }
    let local_missing_from_inventory: Vec<ManagedSkillSummaryDto> = local_skills
        .iter()
        .filter(|skill| !matched_ids.contains(&skill.id))
        .map(|skill| ManagedSkillSummaryDto {
            id: skill.id.clone(),
            name: skill.name.clone(),
        })
        .collect();
    Ok(SkillInventoryPreviewDto {
        valid: errors.is_empty(),
        errors,
        group_count: inventory.groups.len(),
        matched_skill_count: matched_ids.len(),
        unmatched_inventory_skills: unmatched,
        default_disable_count: local_missing_from_inventory.len(),
        local_missing_from_inventory,
        content_changed_count,
    })
}

fn match_inventory_skill<'a>(
    item: &SkillInventorySkillJson,
    local_skills: &'a [super::types::Skill],
) -> Option<&'a super::types::Skill> {
    if let Some(id) = item.id.as_ref().filter(|id| !id.trim().is_empty()) {
        if let Some(skill) = local_skills.iter().find(|skill| skill.id == *id) {
            return Some(skill);
        }
    }
    local_skills.iter().find(|skill| {
        skill.name == item.name
            && (skill.source_ref == item.source_ref
                || skill.central_path == item.central_path
                || item.central_path.ends_with(&skill.central_path))
    })
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

pub async fn resync_all_skills_internal(
    app: tauri::AppHandle,
    state: &DbState,
) -> Result<Vec<String>, String> {
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let skills = skill_store::get_managed_skills(&state).await?;
    let central_dir = resolve_central_repo_path(&app, state)
        .await
        .map_err(|e| format_error(e))?;

    let mut synced: Vec<String> = Vec::new();

    for skill in skills {
        if !skill.management_enabled {
            continue;
        }

        // Resolve central_path (handles cross-platform legacy paths)
        let central_path = resolve_skill_central_path(&skill.central_path, &central_dir);
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
            if !runtime_adapter.is_custom
                && !is_tool_installed_async(&runtime_adapter)
                    .await
                    .unwrap_or(false)
            {
                continue;
            }

            let tool_root = match resolve_runtime_skills_path_async(&runtime_adapter).await {
                Ok(p) => p,
                Err(_) => continue,
            };

            let target = tool_root.join(&skill.name);
            let previous_target = skill_store::get_skill_target(&state, &skill.id, tool_key)
                .await
                .ok()
                .flatten();

            // Sync with overwrite
            if let Ok(result) = sync_skill_to_target(
                tool_key,
                &central_path,
                &target,
                true,
                runtime_adapter.force_copy,
            ) {
                if let Some(existing_target) = previous_target.as_ref() {
                    if target_path_changed(&existing_target.target_path, &target) {
                        let _ = remove_skill_target_checked(
                            &central_path,
                            &existing_target.target_path,
                        );
                    }
                }
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

pub async fn resync_all_skills_if_tool_path_changed(
    app: tauri::AppHandle,
    state: &DbState,
    tool_key: &str,
    previous_skills_path: Option<PathBuf>,
) {
    let current_skills_path =
        runtime_location::get_tool_skills_path_async(&state.db(), tool_key).await;

    let path_changed = match (&previous_skills_path, &current_skills_path) {
        (Some(previous), Some(current)) => {
            target_path_changed(&previous.to_string_lossy(), current)
        }
        (None, Some(_)) | (Some(_), None) => true,
        (None, None) => false,
    };

    if !path_changed {
        return;
    }

    if let Err(err) = resync_all_skills_internal(app, state).await {
        log::warn!(
            "Skills resync after '{}' runtime path change failed: {}",
            tool_key,
            err
        );
    }
}

/// Re-sync all skills to installed tools (used after restore)
#[tauri::command]
pub async fn skills_resync_all(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
) -> Result<Vec<String>, String> {
    resync_all_skills_internal(app, state.inner()).await
}
