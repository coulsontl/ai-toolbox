use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Runtime, State};

use super::adapter::parse_sync_details;
use super::cache_cleanup::{
    cleanup_git_cache_dirs, get_git_cache_cleanup_days, get_git_cache_ttl_secs,
    set_git_cache_cleanup_days as set_cleanup_days,
};
use super::central_repo::{
    clear_central_repo_path, ensure_central_repo, expand_home_path, resolve_central_repo_path,
    resolve_default_central_repo_path, resolve_skill_central_path, save_central_repo_path,
    to_relative_central_path,
};
use super::content_hash::hash_dir;
use super::cron_utils::parse_cron;
use super::git_fetcher::{set_proxy, GitProxyMode};
use super::installer::{
    install_git_skill, install_git_skill_from_selection, install_local_skill,
    install_local_skill_from_selection, list_git_skills, list_local_skills,
    update_managed_skill_from_source,
};
use super::onboarding::build_onboarding_plan;
use super::path_executor::{
    remove_skill_target_checked, sync_skill_to_target, target_path_changed,
    validate_skill_sync_target,
};
use super::skill_store;
use super::sync_engine::{copy_dir_recursive, ensure_source_target_not_overlapping};
use super::tool_adapters::{
    adapter_by_key, get_all_tool_adapters, is_tool_installed_with_state_async,
    resolve_runtime_skills_path_with_state_async, runtime_adapter_by_key,
};
use super::types::{
    now_ms, AdoptCentralSkillsResultDto, ApplyCentralRepoPathOptionsDto,
    ApplyCentralRepoPathResultDto, AutoUpdateConfigDto, CentralRepoConflictDto,
    CentralRepoMigrationCandidateDto, CentralRepoPathPreviewDto, CentralRepoPathStatusDto,
    CentralRepoScanDto, CentralRepoTargetImpactDto, CentralSkillMatchDto,
    CentralSkillRepairCandidateDto, CustomTool, CustomToolDto, DeleteManagedSkillOptionsDto,
    DetectedCentralSkillDto, GitSkillCandidate, InstallResultDto, ManagedSkillDto,
    ManagedSkillSummaryDto, OnboardingPlan, Skill, SkillDocumentDto, SkillGroupDto,
    SkillGroupRecord, SkillInventoryGroupJson, SkillInventoryJson, SkillInventoryPreviewDto,
    SkillInventorySkillJson, SkillRepo, SkillRepoDto, SkillTarget, SkillTargetDto,
    SkillsUpdateProgress, SyncResultDto, ToolInfoDto, ToolStatusDto, UpdateAllErrorDto,
    UpdateAllResultDto, UpdateResultDto,
};
use crate::coding::runtime_location;
use crate::http_client;
use crate::SqliteDbState;

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
    super::frontmatter::parse_skill_md_frontmatter(path).1
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
pub async fn skills_get_tool_status(
    state: State<'_, SqliteDbState>,
) -> Result<ToolStatusDto, String> {
    // Get custom tools
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();

    // Get all adapters (built-in + custom)
    let all_adapters = get_all_tool_adapters(&custom_tools);

    let mut tools: Vec<ToolInfoDto> = Vec::new();
    let mut installed: Vec<String> = Vec::new();

    for adapter in &all_adapters {
        let ok = is_tool_installed_with_state_async(state.db(), adapter)
            .await
            .unwrap_or(false);
        let skills_path = resolve_runtime_skills_path_with_state_async(state.db(), adapter)
            .await
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        tools.push(ToolInfoDto {
            key: adapter.key.clone(),
            label: adapter.display_name.clone(),
            installed: ok,
            skills_dir: skills_path,
            icon_url: adapter.icon_url.clone(),
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
        let state_ref = state.db().clone();
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

fn parse_skill_md_metadata(path: &Path) -> (Option<String>, Option<String>) {
    super::frontmatter::parse_skill_md_frontmatter(path)
}

fn normalize_path_for_compare(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => normalize_path_for_compare(left) == normalize_path_for_compare(right),
    }
}

fn normalize_central_relative_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Err("Root-level SKILL.md is not supported as a managed Skill".to_string());
    }

    let replaced = trimmed.replace('\\', "/");
    if replaced.starts_with('/')
        || replaced.starts_with("~/")
        || replaced == "~"
        || (replaced.len() >= 3
            && replaced.as_bytes()[0].is_ascii_alphabetic()
            && replaced.as_bytes()[1] == b':')
    {
        return Err("Skill relative path must not be absolute".to_string());
    }

    let trimmed = replaced.trim_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return Err("Root-level SKILL.md is not supported as a managed Skill".to_string());
    }

    let mut parts = Vec::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.trim().is_empty() {
                    return Err("Skill relative path contains an empty segment".to_string());
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(
                    "Skill relative path must stay inside the central directory".to_string()
                );
            }
        }
    }

    if parts.is_empty() {
        return Err("Root-level SKILL.md is not supported as a managed Skill".to_string());
    }
    Ok(parts.join("/"))
}

fn skill_relative_path_for_repo(skill: &Skill, current_central_dir: &Path) -> Option<String> {
    if let Ok(relative_path) = normalize_central_relative_path(&skill.central_path) {
        return Some(relative_path);
    }

    let resolved = resolve_skill_central_path(&skill.central_path, current_central_dir);
    normalize_central_relative_path(&to_relative_central_path(&resolved, current_central_dir)).ok()
}

fn path_can_write(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| !metadata.permissions().readonly())
        .unwrap_or(false)
}

fn parent_can_create(path: &Path) -> bool {
    path.parent()
        .and_then(|parent| std::fs::metadata(parent).ok())
        .map(|metadata| metadata.is_dir() && !metadata.permissions().readonly())
        .unwrap_or(false)
}

fn build_path_status(current_path: PathBuf, default_path: PathBuf) -> CentralRepoPathStatusDto {
    let exists = current_path.exists();
    let is_directory = current_path.is_dir();
    let can_read = std::fs::read_dir(&current_path).is_ok();
    let can_write = is_directory && path_can_write(&current_path);
    let warning = if exists && !is_directory {
        Some("Path exists but is not a directory".to_string())
    } else if exists && !can_read {
        Some("Directory cannot be read".to_string())
    } else if exists && !can_write {
        Some("Directory may not be writable".to_string())
    } else {
        None
    };

    CentralRepoPathStatusDto {
        current_path: current_path.to_string_lossy().to_string(),
        default_path: default_path.to_string_lossy().to_string(),
        uses_default: paths_equivalent(&current_path, &default_path),
        exists,
        is_directory,
        can_read,
        can_write,
        warning,
    }
}

fn detected_skill_from_dir(base: &Path, dir: &Path) -> Option<DetectedCentralSkillDto> {
    let manifest_path = dir.join("SKILL.md");
    if !manifest_path.is_file() {
        return None;
    }

    let relative_path = dir.strip_prefix(base).ok()?;
    let relative_path = normalize_central_relative_path(&relative_path.to_string_lossy()).ok()?;
    let (manifest_name, description) = parse_skill_md_metadata(&manifest_path);
    let fallback_name = dir
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)?;
    let name = manifest_name.unwrap_or(fallback_name);
    let content_hash = hash_dir(dir).ok();

    Some(DetectedCentralSkillDto {
        name,
        description,
        relative_path,
        absolute_path: dir.to_string_lossy().to_string(),
        content_hash,
    })
}

fn scan_central_dir_recursive(
    base: &Path,
    dir: &Path,
    visited: &mut HashSet<PathBuf>,
    detected: &mut Vec<DetectedCentralSkillDto>,
) -> Result<(), String> {
    let real_dir = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    if !visited.insert(real_dir) {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        if !std::fs::metadata(&path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false)
        {
            continue;
        }

        if let Some(skill) = detected_skill_from_dir(base, &path) {
            detected.push(skill);
            continue;
        }

        scan_central_dir_recursive(base, &path, visited, detected)?;
    }

    Ok(())
}

fn scan_central_dir(
    base: &Path,
) -> Result<
    (
        Vec<DetectedCentralSkillDto>,
        Vec<CentralRepoConflictDto>,
        Option<String>,
    ),
    String,
> {
    if !base.exists() {
        return Ok((Vec::new(), Vec::new(), None));
    }
    if !base.is_dir() {
        return Err(format!("Path is not a directory: {}", base.display()));
    }

    let root_skill_warning = if base.join("SKILL.md").is_file() {
        Some(
            "Root-level SKILL.md was found but is not supported. Put each Skill in its own subdirectory."
                .to_string(),
        )
    } else {
        None
    };

    let mut detected = Vec::new();
    let mut visited = HashSet::new();
    scan_central_dir_recursive(base, base, &mut visited, &mut detected)?;

    detected.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });

    let mut paths_by_name: HashMap<String, Vec<String>> = HashMap::new();
    for skill in &detected {
        paths_by_name
            .entry(skill.name.trim().to_lowercase())
            .or_default()
            .push(skill.relative_path.clone());
    }

    let mut conflicts: Vec<CentralRepoConflictDto> = paths_by_name
        .into_iter()
        .filter_map(|(name, paths)| {
            if paths.len() > 1 {
                Some(CentralRepoConflictDto {
                    name,
                    paths,
                    reason: "Duplicate Skill name in central directory".to_string(),
                })
            } else {
                None
            }
        })
        .collect();
    conflicts.sort_by(|left, right| left.name.cmp(&right.name));

    Ok((detected, conflicts, root_skill_warning))
}

async fn build_central_repo_path_preview<R: Runtime>(
    app: &AppHandle<R>,
    state: &SqliteDbState,
    requested_path: &str,
) -> Result<CentralRepoPathPreviewDto, String> {
    let current_dir = resolve_central_repo_path(app, state)
        .await
        .map_err(|e| format_error(e))?;
    let default_dir = resolve_default_central_repo_path(app).map_err(|e| format_error(e))?;
    let resolved_dir = expand_home_path(requested_path).map_err(|e| format_error(e))?;

    let exists = resolved_dir.exists();
    let is_directory = resolved_dir.is_dir();
    let can_create = !exists && parent_can_create(&resolved_dir);
    let can_read = exists && std::fs::read_dir(&resolved_dir).is_ok();
    let can_write = exists && is_directory && path_can_write(&resolved_dir);

    let mut blocking_errors = Vec::new();
    let mut path_warnings = Vec::new();
    if !resolved_dir.is_absolute() {
        blocking_errors.push("Storage path must be absolute".to_string());
    }
    if exists && !is_directory {
        blocking_errors.push("Storage path exists but is not a directory".to_string());
    }
    if exists && is_directory && !can_read {
        blocking_errors.push("Storage directory cannot be read".to_string());
    }
    if exists && is_directory && !can_write {
        path_warnings.push("Storage directory may not be writable".to_string());
    }
    if !exists && !can_create {
        path_warnings.push("Storage directory will need to be created, but the parent directory may not be writable".to_string());
    }

    let (detected_skills, conflicts, root_skill_warning) =
        if blocking_errors.is_empty() && (!exists || is_directory) {
            scan_central_dir(&resolved_dir)?
        } else {
            (Vec::new(), Vec::new(), None)
        };

    let skills = skill_store::get_managed_skills(state).await?;
    let detected_by_relative: HashMap<String, DetectedCentralSkillDto> = detected_skills
        .iter()
        .map(|skill| (skill.relative_path.clone(), skill.clone()))
        .collect();
    let mut detected_by_name: HashMap<String, Vec<DetectedCentralSkillDto>> = HashMap::new();
    for detected in &detected_skills {
        detected_by_name
            .entry(detected.name.trim().to_lowercase())
            .or_default()
            .push(detected.clone());
    }

    let mut matched_existing = Vec::new();
    let mut missing_existing = Vec::new();
    let mut repair_candidates = Vec::new();
    let mut migration_candidates = Vec::new();
    let mut migration_conflicts = Vec::new();
    let mut affected_targets = Vec::new();
    let mut existing_name_keys = HashSet::new();
    let mut matched_detected_paths = HashSet::new();

    for skill in &skills {
        existing_name_keys.insert(skill.name.trim().to_lowercase());
        for target in parse_sync_details(skill) {
            affected_targets.push(CentralRepoTargetImpactDto {
                skill_id: skill.id.clone(),
                skill_name: skill.name.clone(),
                tool: target.tool,
                mode: target.mode,
                target_path: target.target_path,
            });
        }

        let Some(relative_path) = skill_relative_path_for_repo(skill, &current_dir) else {
            missing_existing.push(ManagedSkillSummaryDto {
                id: skill.id.clone(),
                name: skill.name.clone(),
            });
            continue;
        };

        let target_path = resolved_dir.join(&relative_path);
        if target_path.exists() {
            if let Some(detected) = detected_by_relative.get(&relative_path) {
                if detected.name.eq_ignore_ascii_case(&skill.name) {
                    matched_existing.push(CentralSkillMatchDto {
                        skill_id: skill.id.clone(),
                        name: skill.name.clone(),
                        relative_path: relative_path.clone(),
                        absolute_path: target_path.to_string_lossy().to_string(),
                    });
                    matched_detected_paths.insert(relative_path.clone());
                    continue;
                }

                matched_detected_paths.insert(relative_path.clone());
                migration_conflicts.push(CentralRepoConflictDto {
                    name: skill.name.clone(),
                    paths: vec![relative_path.clone()],
                    reason: format!(
                        "Target path already contains a different Skill name: {}",
                        detected.name
                    ),
                });
            } else {
                migration_conflicts.push(CentralRepoConflictDto {
                    name: skill.name.clone(),
                    paths: vec![relative_path.clone()],
                    reason: "Target path already exists but does not contain a valid SKILL.md"
                        .to_string(),
                });
            }

            missing_existing.push(ManagedSkillSummaryDto {
                id: skill.id.clone(),
                name: skill.name.clone(),
            });
            continue;
        }

        missing_existing.push(ManagedSkillSummaryDto {
            id: skill.id.clone(),
            name: skill.name.clone(),
        });

        let source_path = resolve_skill_central_path(&skill.central_path, &current_dir);
        if !paths_equivalent(&current_dir, &resolved_dir) && source_path.exists() {
            migration_candidates.push(CentralRepoMigrationCandidateDto {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                relative_path: relative_path.clone(),
                source_path: source_path.to_string_lossy().to_string(),
                target_path: target_path.to_string_lossy().to_string(),
            });
        } else if let Some(candidates) = detected_by_name.get(&skill.name.trim().to_lowercase()) {
            if candidates.len() == 1 {
                let detected = &candidates[0];
                repair_candidates.push(CentralSkillRepairCandidateDto {
                    skill_id: skill.id.clone(),
                    name: skill.name.clone(),
                    current_relative_path: relative_path.clone(),
                    detected_relative_path: detected.relative_path.clone(),
                    detected_absolute_path: detected.absolute_path.clone(),
                    description: detected.description.clone(),
                });
            }
        }
    }

    let unmanaged_detected: Vec<DetectedCentralSkillDto> = detected_skills
        .iter()
        .filter(|skill| !matched_detected_paths.contains(&skill.relative_path))
        .filter(|skill| !existing_name_keys.contains(&skill.name.trim().to_lowercase()))
        .cloned()
        .collect();

    let can_apply =
        blocking_errors.is_empty() && conflicts.is_empty() && migration_conflicts.is_empty();

    Ok(CentralRepoPathPreviewDto {
        requested_path: requested_path.to_string(),
        resolved_path: resolved_dir.to_string_lossy().to_string(),
        current_path: current_dir.to_string_lossy().to_string(),
        default_path: default_dir.to_string_lossy().to_string(),
        current_uses_default: paths_equivalent(&current_dir, &default_dir),
        requested_is_default: paths_equivalent(&resolved_dir, &default_dir),
        exists,
        is_directory,
        can_create,
        can_read,
        can_write,
        detected_skills,
        matched_existing,
        unmanaged_detected,
        missing_existing,
        repair_candidates,
        migration_candidates,
        migration_conflicts,
        affected_targets,
        conflicts,
        root_skill_warning,
        path_warnings,
        blocking_errors,
        can_apply,
    })
}

async fn refresh_central_skill_hash_if_needed(
    state: &SqliteDbState,
    skill: &mut Skill,
    source_path: &Path,
) -> Result<(), String> {
    if skill.source_type != "central" {
        return Ok(());
    }
    let hash = hash_dir(source_path).map_err(|e| format_error(e))?;
    if skill.content_hash.as_deref() != Some(hash.as_str()) {
        skill_store::update_skill_content_hash(state, &skill.id, Some(hash.clone())).await?;
        skill.content_hash = Some(hash);
    }
    Ok(())
}

fn safe_source_delete_allowed(source_path: &Path, central_dir: &Path) -> bool {
    let Ok(source) = std::fs::canonicalize(source_path) else {
        return false;
    };
    let Ok(central) = std::fs::canonicalize(central_dir) else {
        return false;
    };
    source != central && source.starts_with(&central)
}

fn copy_skill_source_for_migration(source: &Path, target: &Path) -> Result<Option<String>, String> {
    if target.exists() {
        return Err(format!("Target path already exists: {}", target.display()));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut warning = None;
    let copy_source = if std::fs::symlink_metadata(source)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        warning = Some(format!(
            "Symlink source was migrated as a real directory: {}",
            source.display()
        ));
        std::fs::canonicalize(source).map_err(|e| e.to_string())?
    } else {
        source.to_path_buf()
    };
    ensure_source_target_not_overlapping(&copy_source, target).map_err(format_error)?;
    copy_dir_recursive(&copy_source, target).map_err(|e| format_error(e))?;
    Ok(warning)
}

fn source_path_missing_for_delete(path: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound
    )
}

fn central_path_update_for_existing_match(
    target_dir: &Path,
    relative_path: &str,
) -> (String, Option<String>) {
    let source_path = target_dir.join(relative_path);
    (relative_path.to_string(), hash_dir(&source_path).ok())
}

async fn adopt_detected_central_skill(
    state: &SqliteDbState,
    central_dir: &Path,
    relative_path: &str,
) -> Result<bool, String> {
    let relative_path = normalize_central_relative_path(relative_path)?;
    let source_path = central_dir.join(&relative_path);
    let Some(detected) = detected_skill_from_dir(central_dir, &source_path) else {
        return Err(format!("No Skill found at {}", source_path.display()));
    };
    if skill_store::get_skill_by_name(state, &detected.name)
        .await?
        .is_some()
    {
        return Ok(false);
    }

    let now = now_ms();
    let skill = Skill {
        id: String::new(),
        tags: Vec::new(),
        name: detected.name,
        source_type: "central".to_string(),
        source_ref: None,
        source_revision: None,
        central_path: relative_path,
        content_hash: detected.content_hash,
        created_at: now,
        updated_at: now,
        last_sync_at: None,
        status: "ok".to_string(),
        sort_index: 0,
        user_group: None,
        group_id: None,
        user_note: None,
        management_enabled: true,
        disabled_previous_tools: Vec::new(),
        enabled_tools: Vec::new(),
        sync_details: Some(serde_json::Value::Object(serde_json::Map::new())),
    };
    skill_store::upsert_skill(state, &skill).await?;
    Ok(true)
}

#[tauri::command]
pub async fn skills_get_central_repo_path(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
) -> Result<String, String> {
    let path = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;
    ensure_central_repo(&path).map_err(|e| format_error(e))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn skills_set_central_repo_path(
    state: State<'_, SqliteDbState>,
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

#[tauri::command]
pub async fn skills_get_default_central_repo_path(app: tauri::AppHandle) -> Result<String, String> {
    let path = resolve_default_central_repo_path(&app).map_err(|e| format_error(e))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn skills_get_central_repo_path_status(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
) -> Result<CentralRepoPathStatusDto, String> {
    let current_path = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;
    let default_path = resolve_default_central_repo_path(&app).map_err(|e| format_error(e))?;
    Ok(build_path_status(current_path, default_path))
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_preview_central_repo_path(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
    path: String,
) -> Result<CentralRepoPathPreviewDto, String> {
    build_central_repo_path_preview(&app, &state, &path).await
}

#[tauri::command]
pub async fn skills_scan_central_repo(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
) -> Result<CentralRepoScanDto, String> {
    let central_dir = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;
    let preview =
        build_central_repo_path_preview(&app, &state, &central_dir.to_string_lossy()).await?;
    Ok(CentralRepoScanDto {
        central_path: central_dir.to_string_lossy().to_string(),
        detected_skills: preview.detected_skills,
        unmanaged_detected: preview.unmanaged_detected,
        repair_candidates: preview.repair_candidates,
        conflicts: preview.conflicts,
        root_skill_warning: preview.root_skill_warning,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_adopt_central_repo_skills<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SqliteDbState>,
    relativePaths: Vec<String>,
) -> Result<AdoptCentralSkillsResultDto, String> {
    let central_dir = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;
    let mut adopted_count = 0;
    for relative_path in relativePaths {
        if adopt_detected_central_skill(&state, &central_dir, &relative_path).await? {
            adopted_count += 1;
        }
    }
    let _ = app.emit("skills-changed", "window");
    Ok(AdoptCentralSkillsResultDto {
        adopted_count,
        repaired_count: 0,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_repair_central_repo_skill<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SqliteDbState>,
    skillId: String,
    relativePath: String,
) -> Result<AdoptCentralSkillsResultDto, String> {
    let central_dir = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;
    let relative_path = normalize_central_relative_path(&relativePath)?;
    let source_path = central_dir.join(&relative_path);
    if !source_path.join("SKILL.md").is_file() {
        return Err(format!("No Skill found at {}", source_path.display()));
    }
    let content_hash = hash_dir(&source_path).ok();
    skill_store::update_skill_central_path_and_hash(&state, &skillId, relative_path, content_hash)
        .await?;
    let _ = app.emit("skills-changed", "window");
    Ok(AdoptCentralSkillsResultDto {
        adopted_count: 0,
        repaired_count: 1,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_apply_central_repo_path_change<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SqliteDbState>,
    path: String,
    options: ApplyCentralRepoPathOptionsDto,
) -> Result<ApplyCentralRepoPathResultDto, String> {
    let requested_path = if options.use_default_path {
        resolve_default_central_repo_path(&app)
            .map_err(|e| format_error(e))?
            .to_string_lossy()
            .to_string()
    } else {
        path
    };
    let preview = build_central_repo_path_preview(&app, &state, &requested_path).await?;
    if !preview.can_apply {
        let mut apply_errors = preview.blocking_errors.clone();
        apply_errors.extend(preview.conflicts.iter().map(|conflict| {
            format!(
                "{}: {} ({})",
                conflict.name,
                conflict.reason,
                conflict.paths.join(", ")
            )
        }));
        apply_errors.extend(preview.migration_conflicts.iter().map(|conflict| {
            format!(
                "{}: {} ({})",
                conflict.name,
                conflict.reason,
                conflict.paths.join(", ")
            )
        }));
        if apply_errors.is_empty() {
            apply_errors.push("Unknown central directory validation error".to_string());
        }
        return Err(format!(
            "Central directory cannot be applied:\n- {}",
            apply_errors.join("\n- ")
        ));
    }

    let target_dir = PathBuf::from(&preview.resolved_path);
    ensure_central_repo(&target_dir).map_err(|e| format_error(e))?;

    let current_dir = PathBuf::from(&preview.current_path);
    let default_dir = PathBuf::from(&preview.default_path);
    let skills = skill_store::get_managed_skills(&state).await?;
    let migrate_set: HashSet<String> = options.migrate_existing_skill_ids.into_iter().collect();
    let mut migrated_count = 0;
    let mut warnings = preview.path_warnings.clone();
    if let Some(root_warning) = preview.root_skill_warning.clone() {
        warnings.push(root_warning);
    }
    let mut central_path_updates: HashMap<String, (String, Option<String>)> = HashMap::new();
    for matched in &preview.matched_existing {
        let Some(skill) = skills.iter().find(|skill| skill.id == matched.skill_id) else {
            continue;
        };
        central_path_updates.insert(
            skill.id.clone(),
            central_path_update_for_existing_match(&target_dir, &matched.relative_path),
        );
    }

    let mut migration_errors = Vec::new();
    for skill in &skills {
        if !migrate_set.contains(&skill.id) {
            continue;
        }
        let Some(relative_path) = skill_relative_path_for_repo(skill, &current_dir) else {
            migration_errors.push(format!("{}: central path is invalid", skill.name));
            continue;
        };
        let source_path = resolve_skill_central_path(&skill.central_path, &current_dir);
        let target_path = target_dir.join(&relative_path);
        match copy_skill_source_for_migration(&source_path, &target_path) {
            Ok(warning) => {
                migrated_count += 1;
                central_path_updates.insert(
                    skill.id.clone(),
                    (relative_path.clone(), hash_dir(&target_path).ok()),
                );
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
            }
            Err(error) => {
                migration_errors.push(format!("{}: {}", skill.name, error));
            }
        }
    }
    if !migration_errors.is_empty() {
        return Err(format!(
            "Central directory migration failed:\n- {}",
            migration_errors.join("\n- ")
        ));
    }

    if paths_equivalent(&target_dir, &default_dir) {
        clear_central_repo_path(&state)
            .await
            .map_err(|e| format_error(e))?;
    } else {
        save_central_repo_path(&state, &target_dir)
            .await
            .map_err(|e| format_error(e))?;
    }

    for (skill_id, (relative_path, content_hash)) in central_path_updates {
        skill_store::update_skill_central_path_and_hash(
            &state,
            &skill_id,
            relative_path,
            content_hash,
        )
        .await?;
    }

    let mut repaired_count = 0;
    for (skill_id, relative_path) in options.repair_existing_skill_paths {
        let relative_path = match normalize_central_relative_path(&relative_path) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(format!("Skipped repair for '{}': {}", skill_id, error));
                continue;
            }
        };
        let source_path = target_dir.join(&relative_path);
        if !source_path.join("SKILL.md").is_file() {
            warnings.push(format!(
                "Skipped repair for '{}': no SKILL.md at {}",
                skill_id,
                source_path.display()
            ));
            continue;
        }
        let content_hash = hash_dir(&source_path).ok();
        skill_store::update_skill_central_path_and_hash(
            &state,
            &skill_id,
            relative_path,
            content_hash,
        )
        .await?;
        repaired_count += 1;
    }

    let mut adopted_count = 0;
    for relative_path in options.adopt_detected_skill_paths {
        match adopt_detected_central_skill(&state, &target_dir, &relative_path).await {
            Ok(true) => adopted_count += 1,
            Ok(false) => warnings.push(format!(
                "Skipped adopting '{}' because a Skill with the same name is already managed",
                relative_path
            )),
            Err(error) => warnings.push(format!("Skipped adopting '{}': {}", relative_path, error)),
        }
    }

    let mut resynced_targets = Vec::new();
    if options.resync_enabled_tools {
        match resync_all_skills_internal(app.clone(), &state).await {
            Ok(targets) => resynced_targets = targets,
            Err(error) => warnings.push(format!(
                "Resync after central directory change failed: {}",
                error
            )),
        }
    }

    let _ = app.emit("skills-changed", "window");

    Ok(ApplyCentralRepoPathResultDto {
        path: target_dir.to_string_lossy().to_string(),
        uses_default: paths_equivalent(&target_dir, &default_dir),
        adopted_count,
        repaired_count,
        migrated_count,
        resynced_targets,
        warnings,
    })
}

// --- Managed Skills ---

#[tauri::command]
pub async fn skills_get_managed_skills(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
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
            tags: skill.tags.clone(),
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

    #[test]
    fn central_relative_path_rejects_root_and_parent_escape() {
        assert!(normalize_central_relative_path("").is_err());
        assert!(normalize_central_relative_path(".").is_err());
        assert!(normalize_central_relative_path("../outside").is_err());
        assert!(normalize_central_relative_path("/Users/alice/.skills/foo").is_err());
        assert!(normalize_central_relative_path("/home/alice/.skills/foo").is_err());
        assert!(normalize_central_relative_path("C:\\Users\\alice\\.skills\\foo").is_err());
        assert!(normalize_central_relative_path("~/skills/foo").is_err());
        assert_eq!(
            normalize_central_relative_path("demo-skill").expect("relative path"),
            "demo-skill"
        );
        assert_eq!(
            normalize_central_relative_path("group/demo-skill").expect("nested relative path"),
            "group/demo-skill"
        );
    }

    #[test]
    fn central_dir_scan_warns_root_skill_and_detects_child_skill() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp.path().join("SKILL.md"), "---\nname: root\n---\n")
            .expect("write root skill");
        let child = temp.path().join("demo");
        std::fs::create_dir(&child).expect("create child dir");
        std::fs::write(child.join("SKILL.md"), "---\nname: demo\n---\n")
            .expect("write child skill");

        let (detected, conflicts, warning) =
            scan_central_dir(temp.path()).expect("scan central dir");

        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].name, "demo");
        assert_eq!(detected[0].relative_path, "demo");
        assert!(conflicts.is_empty());
        assert!(warning
            .as_deref()
            .expect("root warning")
            .contains("Root-level"));
    }

    #[test]
    fn central_dir_scan_detects_nested_skills() {
        let temp = tempfile::tempdir().expect("temp dir");
        let nested = temp.path().join("frontend").join("review");
        std::fs::create_dir_all(&nested).expect("create nested skill dir");
        std::fs::write(
            nested.join("SKILL.md"),
            "---\nname: review\ndescription: Review frontend code\n---\n",
        )
        .expect("write nested skill");

        let (detected, conflicts, warning) =
            scan_central_dir(temp.path()).expect("scan central dir");

        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].name, "review");
        assert_eq!(detected[0].relative_path, "frontend/review");
        assert_eq!(
            detected[0].description.as_deref(),
            Some("Review frontend code")
        );
        assert!(conflicts.is_empty());
        assert!(warning.is_none());
    }

    #[test]
    fn migration_copy_rejects_source_target_overlap() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("repo").join("foo");
        std::fs::create_dir_all(&source).expect("create source dir");
        std::fs::write(source.join("SKILL.md"), "---\nname: foo\n---\n").expect("write skill");
        let target = source.join("foo");

        let error = copy_skill_source_for_migration(&source, &target)
            .expect_err("overlapping copy must fail");

        assert!(error.contains("inside source") || error.contains("overlap"));
        assert!(!target.exists());
    }

    #[test]
    fn existing_match_update_refreshes_target_hash_even_when_relative_path_is_same() {
        let temp = tempfile::tempdir().expect("temp dir");
        let target_skill = temp.path().join("foo");
        std::fs::create_dir(&target_skill).expect("create target skill dir");
        std::fs::write(
            target_skill.join("SKILL.md"),
            "---\nname: foo\n---\nnew content",
        )
        .expect("write skill");

        let (relative_path, content_hash) =
            central_path_update_for_existing_match(temp.path(), "foo");

        assert_eq!(relative_path, "foo");
        assert_eq!(
            content_hash,
            Some(hash_dir(&target_skill).expect("hash target skill"))
        );
    }

    #[test]
    fn delete_missing_source_path_is_noop() {
        let temp = tempfile::tempdir().expect("temp dir");
        let missing = temp.path().join("missing");

        assert!(source_path_missing_for_delete(&missing));
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

    #[test]
    fn read_skill_document_returns_markdown_content() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp.path().join("SKILL.md"), "# Demo\n\nBody text")
            .expect("write SKILL.md");

        let doc = read_skill_document_file(temp.path(), "SKILL.md").expect("read doc");

        assert_eq!(doc.filename, "SKILL.md");
        assert!(doc.content.contains("# Demo"));
        assert!(!doc.truncated);
    }

    #[test]
    fn read_skill_document_skips_missing_file() {
        let temp = tempfile::tempdir().expect("temp dir");

        let doc = read_skill_document_file(temp.path(), "README.md");

        assert!(doc.is_none());
    }

    #[test]
    fn read_skill_document_truncates_large_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let oversized = vec![b'x'; SKILL_DOCUMENT_READ_CAP_BYTES * 2];
        let mut content = b"---\nname: big\n---\n".to_vec();
        content.extend_from_slice(&oversized);
        std::fs::write(temp.path().join("SKILL.md"), &content).expect("write big SKILL.md");

        let doc = read_skill_document_file(temp.path(), "SKILL.md").expect("read doc");

        assert!(doc.truncated);
        assert!(doc.content.len() <= SKILL_DOCUMENT_READ_CAP_BYTES + 16);
    }
}

// --- Install Skills ---

/// Resolve local skill source paths so `~/...` / `%APPDATA%/...` aliases work
/// the same way as central-repo path inputs.
fn resolve_local_source_path(source_path: &str) -> Result<PathBuf, String> {
    expand_home_path(source_path).map_err(|e| format_error(e))
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_install_local(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
    sourcePath: String,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let source_path = resolve_local_source_path(&sourcePath)?;
    let result = install_local_skill(&app, &state, &source_path, overwrite.unwrap_or(false))
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
    let source_path = resolve_local_source_path(&sourcePath)?;
    list_local_skills(&source_path).map_err(|e| format_error(e))
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_install_local_selection(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
    sourcePath: String,
    subpath: String,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let source_path = resolve_local_source_path(&sourcePath)?;
    let result = install_local_skill_from_selection(
        &app,
        &state,
        &source_path,
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
    state: State<'_, SqliteDbState>,
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
    state: State<'_, SqliteDbState>,
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
    state: State<'_, SqliteDbState>,
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
    state: &SqliteDbState,
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

/// Check whether source and target resolve to the same physical path (e.g.
/// when the user configured the central repo to `~/.agents/skills`, which is
/// also the shared_agents tool's skills dir). Returns `true` when the sync
/// should be skipped (no-op).
fn source_target_overlap(source: &Path, target: &Path) -> bool {
    if let (Ok(source_canon), Ok(target_canon)) =
        (std::fs::canonicalize(source), std::fs::canonicalize(target))
    {
        source_canon == target_canon
    } else {
        false
    }
}

async fn sync_skill_to_tool_record(
    state: &SqliteDbState,
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
        && !is_tool_installed_with_state_async(state.db(), &runtime_adapter)
            .await
            .unwrap_or(false)
    {
        let skills_path =
            resolve_runtime_skills_path_with_state_async(state.db(), &runtime_adapter)
                .await
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
        return Err(format!(
            "TOOL_NOT_INSTALLED|{}|{}",
            runtime_adapter.key, skills_path
        ));
    }

    let tool_root = resolve_runtime_skills_path_with_state_async(state.db(), &runtime_adapter)
        .await
        .map_err(|e| format_error(e))?;
    let target = tool_root.join(&skill.name);
    let previous_target = skill_store::get_skill_target(state, &skill.id, tool).await?;

    // When the tool's skills dir is the same physical directory as the central
    // repo (e.g. shared_agents targets ~/.agents/skills which is also the
    // central repo path when the user configured it so), syncing would be a
    // no-op. Skip gracefully instead of triggering ensure_source_target_not_overlapping's bail.
    if source_target_overlap(source_path, &target) {
        let record = SkillTarget {
            tool: tool.to_string(),
            target_path: target.to_string_lossy().to_string(),
            mode: "skip".to_string(),
            status: "ok".to_string(),
            error_message: None,
            synced_at: Some(now_ms()),
        };
        skill_store::upsert_skill_target(state, &skill.id, &record).await?;
        return Ok(SyncResultDto {
            mode_used: "skip".to_string(),
            target_path: target.to_string_lossy().to_string(),
        });
    }

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
            remove_skill_target_best_effort(skill, source_path, existing_target);
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

fn disabled_previous_tools_for_skill(skill: &Skill) -> Vec<String> {
    if skill.enabled_tools.is_empty() {
        skill.disabled_previous_tools.clone()
    } else {
        skill.enabled_tools.clone()
    }
}

fn remove_skill_target_best_effort(skill: &Skill, source_path: &Path, target: &SkillTarget) {
    if let Err(err) = remove_skill_target_checked(source_path, &target.target_path) {
        log::warn!(
            "Failed to clean Skills target '{}' for skill '{}' on '{}': {}",
            target.target_path,
            skill.name,
            target.tool,
            err
        );
    }
}

async fn remove_skill_targets_best_effort(
    state: &SqliteDbState,
    skill: &Skill,
    source_path: Option<&Path>,
) -> Result<(), String> {
    let targets = skill_store::get_skill_targets(state, &skill.id).await?;
    let Some(source_path) = source_path else {
        if !targets.is_empty() {
            log::warn!(
                "Skipped cleaning {} Skills target(s) for '{}' because source path could not be resolved",
                targets.len(),
                skill.name
            );
        }
        return Ok(());
    };

    for target in targets {
        remove_skill_target_best_effort(skill, source_path, &target);
    }
    Ok(())
}

async fn resolve_skill_source_path_for_cleanup<R: Runtime>(
    app: &AppHandle<R>,
    state: &SqliteDbState,
    skill: &Skill,
) -> Option<PathBuf> {
    match resolve_skill_source_path(app, state, skill).await {
        Ok(path) => Some(path),
        Err(error) => {
            log::warn!(
                "Skipped Skills target cleanup for '{}' because source path could not be resolved: {}",
                skill.name,
                error
            );
            None
        }
    }
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_sync_to_tool<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SqliteDbState>,
    sourcePath: String,
    skillId: String,
    tool: String,
    name: String,
    overwrite: Option<bool>,
) -> Result<SyncResultDto, String> {
    let mut skill = skill_store::get_skill_by_id(&state, &skillId)
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
    refresh_central_skill_hash_if_needed(&state, &mut skill, &source_path).await?;

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
    state: State<'_, SqliteDbState>,
    skillId: String,
    tool: String,
) -> Result<(), String> {
    if let Some(target) = skill_store::get_skill_target(&state, &skillId, &tool).await? {
        let skill = skill_store::get_skill_by_id(&state, &skillId)
            .await?
            .ok_or_else(|| format!("Skill not found: {}", skillId))?;
        let source_path = resolve_skill_source_path_for_cleanup(&app, &state, &skill).await;
        if let Some(source_path) = source_path.as_deref() {
            remove_skill_target_best_effort(&skill, source_path, &target);
        }
        skill_store::delete_skill_target(&state, &skillId, &tool).await?;
    }

    // Emit skills-changed for WSL sync
    let _ = app.emit("skills-changed", "window");

    Ok(())
}

// --- Update/Delete Skills ---

#[tauri::command]
#[allow(non_snake_case)]
/// Update a single managed skill from its source (shared by single update and bulk update).
async fn update_managed_skill_internal(
    app: &tauri::AppHandle,
    state: &State<'_, SqliteDbState>,
    skill_id: &str,
) -> Result<UpdateResultDto, String> {
    if let Some(mut skill) = skill_store::get_skill_by_id(state, skill_id).await? {
        if skill.source_type == "central" {
            let source_path = resolve_skill_source_path(app, state, &skill).await?;
            if !source_path.is_dir() {
                return Err(format!(
                    "Central Skill source path is missing or not a directory: {}",
                    source_path.display()
                ));
            }
            refresh_central_skill_hash_if_needed(state, &mut skill, &source_path).await?;

            let custom_tools = skill_store::get_custom_tools(state)
                .await
                .unwrap_or_default();
            let mut updated_targets = Vec::new();
            let mut sync_errors = Vec::new();
            for tool in skill.enabled_tools.clone() {
                match sync_skill_to_tool_record(
                    state,
                    &skill,
                    &tool,
                    &source_path,
                    true,
                    &custom_tools,
                )
                .await
                {
                    Ok(_) => updated_targets.push(tool),
                    Err(error) => sync_errors.push(format!("{}: {}", tool, error)),
                }
            }
            if !sync_errors.is_empty() {
                return Err(format!(
                    "Central Skill content was refreshed, but some tool targets failed to sync:\n- {}",
                    sync_errors.join("\n- ")
                ));
            }

            return Ok(UpdateResultDto {
                skill_id: skill.id,
                name: skill.name,
                content_hash: skill.content_hash,
                source_revision: skill.source_revision,
                updated_targets,
            });
        }
    }

    let res = update_managed_skill_from_source(app, state, skill_id)
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
pub async fn skills_update_managed(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
    skillId: String,
) -> Result<UpdateResultDto, String> {
    let res = update_managed_skill_internal(&app, &state, &skillId).await?;

    // Emit skills-changed for WSL sync
    let _ = app.emit("skills-changed", "window");

    Ok(res)
}

/// Run an update pass over all managed skills, aggregating per-skill results. Shared by
/// the one-click command and the scheduled auto-update task. Emits `skills-changed` once.
pub(crate) async fn update_all_skills_internal(
    app: &tauri::AppHandle,
    state: &State<'_, SqliteDbState>,
    emit_progress: bool,
) -> Result<UpdateAllResultDto, String> {
    let skills = skill_store::get_managed_skills(state).await?;
    let total = skills.len();
    let mut updated = Vec::new();
    let mut errors = Vec::new();

    if emit_progress {
        let _ = app.emit(
            "skills-update-progress",
            SkillsUpdateProgress {
                current: 0,
                total,
                skill_id: String::new(),
                name: String::new(),
                message: format!("0/{}", total),
            },
        );
    }

    for (index, skill) in skills.into_iter().enumerate() {
        if emit_progress {
            let current = index + 1;
            let _ = app.emit(
                "skills-update-progress",
                SkillsUpdateProgress {
                    current,
                    total,
                    skill_id: skill.id.clone(),
                    name: skill.name.clone(),
                    message: format!("{}/{}", current, total),
                },
            );
        }
        match update_managed_skill_internal(app, state, &skill.id).await {
            Ok(result) => updated.push(result),
            Err(error) => errors.push(UpdateAllErrorDto {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                error,
            }),
        }
    }

    // Emit skills-changed once after the whole pass for WSL sync
    let _ = app.emit("skills-changed", "window");

    Ok(UpdateAllResultDto {
        total,
        updated,
        errors,
    })
}

#[tauri::command]
pub async fn skills_update_all(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
) -> Result<UpdateAllResultDto, String> {
    // Manual trigger: emit progress so the UI can show the pass advancing.
    update_all_skills_internal(&app, &state, true).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_delete_managed(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
    skillId: String,
    options: Option<DeleteManagedSkillOptionsDto>,
) -> Result<(), String> {
    let record = skill_store::get_skill_by_id(&state, &skillId).await?;
    let mut remove_failures: Vec<String> = Vec::new();
    if let Some(skill) = record {
        // Resolve central_path (handles cross-platform legacy paths)
        let central_dir = resolve_central_repo_path(&app, &state)
            .await
            .map_err(|e| format_error(e))?;
        let default_dir = resolve_default_central_repo_path(&app).map_err(|e| format_error(e))?;
        let uses_default_central_dir = paths_equivalent(&central_dir, &default_dir);
        let path = resolve_skill_central_path(&skill.central_path, &central_dir);
        let targets = skill_store::get_skill_targets(&state, &skillId).await?;
        for target in targets {
            if let Err(err) = remove_skill_target_checked(&path, &target.target_path) {
                remove_failures.push(format!("{}: {}", target.target_path, err));
            }
        }

        let default_delete_source = skill.source_type != "central" && uses_default_central_dir;
        let delete_source_files = options
            .map(|options| options.delete_source_files)
            .unwrap_or(default_delete_source);
        if delete_source_files {
            if source_path_missing_for_delete(&path) {
                // The DB record can still be removed if the user already deleted the source dir.
            } else if safe_source_delete_allowed(&path, &central_dir) {
                if path.exists() {
                    std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
                }
            } else {
                remove_failures.push(format!(
                    "{}: source path is outside the current central directory or is the central directory itself",
                    path.display()
                ));
            }
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
    state: State<'_, SqliteDbState>,
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
    state: State<'_, SqliteDbState>,
    sourcePath: String,
    overwrite: Option<bool>,
) -> Result<InstallResultDto, String> {
    let source_path = resolve_local_source_path(&sourcePath)?;
    let result = install_local_skill(&app, &state, &source_path, overwrite.unwrap_or(false))
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
pub async fn skills_get_git_cache_cleanup_days(
    state: State<'_, SqliteDbState>,
) -> Result<i64, String> {
    Ok(get_git_cache_cleanup_days(&state).await)
}

#[tauri::command]
pub async fn skills_set_git_cache_cleanup_days(
    state: State<'_, SqliteDbState>,
    days: i64,
) -> Result<i64, String> {
    set_cleanup_days(&state, days)
        .await
        .map_err(|e| format_error(e))
}

#[tauri::command]
pub async fn skills_get_git_cache_ttl_secs(state: State<'_, SqliteDbState>) -> Result<i64, String> {
    Ok(get_git_cache_ttl_secs(&state).await)
}

#[tauri::command]
pub async fn skills_clear_git_cache(app: tauri::AppHandle) -> Result<usize, String> {
    cleanup_git_cache_dirs(&app, Duration::from_secs(0)).map_err(|e| format_error(e))
}

#[tauri::command]
pub async fn skills_get_git_cache_path(app: tauri::AppHandle) -> Result<String, String> {
    let _ = app;
    let cache_dir = crate::app_paths::resolved_cache_dir();
    let cache_path = cache_dir.join("skills-git-cache");
    if !cache_path.exists() {
        std::fs::create_dir_all(&cache_path).map_err(|e| e.to_string())?;
    }
    Ok(cache_path.to_string_lossy().to_string())
}

// --- Auto Update ---

#[tauri::command]
pub async fn skills_get_auto_update(
    state: State<'_, SqliteDbState>,
) -> Result<AutoUpdateConfigDto, String> {
    let prefs = skill_store::get_skill_preferences(&state).await?;
    Ok(AutoUpdateConfigDto {
        enabled: prefs.auto_update_enabled,
        schedule: prefs.auto_update_schedule,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_set_auto_update(
    state: State<'_, SqliteDbState>,
    config: AutoUpdateConfigDto,
) -> Result<AutoUpdateConfigDto, String> {
    let schedule = config.schedule.trim().to_string();
    // Validate before persisting so an enabled config never holds a broken schedule.
    parse_cron(&schedule)?;
    skill_store::set_setting(&state, "auto_update_enabled", &config.enabled.to_string()).await?;
    skill_store::set_setting(&state, "auto_update_schedule", &schedule).await?;
    Ok(AutoUpdateConfigDto {
        enabled: config.enabled,
        schedule,
    })
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_preview_auto_update_schedule(
    schedule: String,
    count: Option<u32>,
) -> Result<Vec<String>, String> {
    let count = (count.unwrap_or(10)).clamp(1, 100) as usize;
    super::cron_utils::next_n_occurrences(&schedule, count)
}

// --- Preferred Tools ---

#[tauri::command]
pub async fn skills_get_preferred_tools(
    state: State<'_, SqliteDbState>,
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
    app: AppHandle,
    state: State<'_, SqliteDbState>,
    tools: Vec<String>,
) -> Result<(), String> {
    skill_store::set_setting(
        &state,
        "preferred_tools_v1",
        &serde_json::to_string(&tools).unwrap_or_else(|_| "[]".to_string()),
    )
    .await?;
    // Rebuild the tray menu so the skills submenu reflects the new preferred
    // tools immediately (the "show only preferred tools" toggle reads this).
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Get whether Skill card add-more menus are limited to preferred tools.
#[tauri::command]
pub async fn skills_get_limit_add_more_to_preferred_tools(
    state: State<'_, SqliteDbState>,
) -> Result<bool, String> {
    let raw = skill_store::get_setting(&state, "limit_add_more_to_preferred_tools").await?;
    Ok(raw.as_deref() == Some("true"))
}

/// Set whether Skill card add-more menus are limited to preferred tools.
#[tauri::command]
pub async fn skills_set_limit_add_more_to_preferred_tools(
    app: AppHandle,
    state: State<'_, SqliteDbState>,
    enabled: bool,
) -> Result<(), String> {
    skill_store::set_setting(
        &state,
        "limit_add_more_to_preferred_tools",
        if enabled { "true" } else { "false" },
    )
    .await?;
    // Rebuild the tray menu so the skills submenu reflects the toggle
    // immediately (it controls whether non-preferred tools are hidden).
    let _ = app.emit("config-changed", "window");
    Ok(())
}

// --- Show Skills in Tray ---

#[tauri::command]
pub async fn skills_get_show_in_tray(state: State<'_, SqliteDbState>) -> Result<bool, String> {
    let raw = skill_store::get_setting(&state, "show_skills_in_tray").await?;
    match raw {
        Some(s) => Ok(s == "true"),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn skills_set_show_in_tray(
    state: State<'_, SqliteDbState>,
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
pub async fn skills_get_default_view_mode(
    state: State<'_, SqliteDbState>,
) -> Result<String, String> {
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
    state: State<'_, SqliteDbState>,
    mode: String,
) -> Result<(), String> {
    skill_store::set_setting(&state, "default_view_mode", &mode).await
}

// --- Custom Tools ---

#[tauri::command]
pub async fn skills_get_custom_tools(
    state: State<'_, SqliteDbState>,
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
            icon_url: t.icon_url,
        })
        .collect())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_add_custom_tool(
    state: State<'_, SqliteDbState>,
    key: String,
    displayName: String,
    relativeSkillsDir: String,
    relativeDetectDir: String,
    forceCopy: Option<bool>,
    iconUrl: Option<String>,
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

    // Icon must be an http(s) image URL when provided; empty clears it
    let icon_url = iconUrl
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    if let Some(ref url) = icon_url {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("Icon URL must start with http:// or https://".to_string());
        }
    }

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
        icon_url,
    };
    skill_store::save_custom_tool(&state, &tool).await
}

#[tauri::command]
pub async fn skills_remove_custom_tool(
    state: State<'_, SqliteDbState>,
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
pub async fn skills_get_groups(
    state: State<'_, SqliteDbState>,
) -> Result<Vec<SkillGroupDto>, String> {
    let groups = skill_store::get_skill_groups(&state).await?;
    Ok(groups.into_iter().map(group_to_dto).collect())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_save_group(
    state: State<'_, SqliteDbState>,
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
pub async fn skills_delete_group(
    state: State<'_, SqliteDbState>,
    groupId: String,
) -> Result<(), String> {
    skill_store::delete_skill_group(&state, &groupId).await
}

#[tauri::command]
pub async fn skills_reorder(
    state: State<'_, SqliteDbState>,
    ids: Vec<String>,
) -> Result<(), String> {
    skill_store::reorder_skills(&state, &ids).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_update_metadata(
    state: State<'_, SqliteDbState>,
    skillId: String,
    groupId: Option<String>,
    userNote: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<(), String> {
    skill_store::update_skill_metadata(
        &state,
        &skillId,
        normalize_optional_id(groupId),
        normalize_optional_text(userNote),
        tags,
    )
    .await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_batch_update_group(
    state: State<'_, SqliteDbState>,
    skillIds: Vec<String>,
    groupId: Option<String>,
) -> Result<(), String> {
    skill_store::update_skills_group(&state, &skillIds, normalize_optional_id(groupId)).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_set_management_enabled<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SqliteDbState>,
    skillId: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    if !enabled {
        let skill = skill_store::get_skill_by_id(&state, &skillId)
            .await?
            .ok_or_else(|| format!("Skill not found: {}", skillId))?;
        let previous_tools = disabled_previous_tools_for_skill(&skill);
        let source_path = resolve_skill_source_path_for_cleanup(&app, &state, &skill).await;
        remove_skill_targets_best_effort(&state, &skill, source_path.as_deref()).await?;
        skill_store::disable_skill_with_previous_tools(&state, &skillId, previous_tools.clone())
            .await?;
        let _ = app.emit("skills-changed", "window");
        return Ok(previous_tools);
    }
    let previous = skill_store::set_skill_management_enabled(&state, &skillId, enabled).await?;
    let _ = app.emit("skills-changed", "window");
    Ok(previous)
}

#[tauri::command]
pub async fn skills_export_inventory(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
) -> Result<String, String> {
    build_inventory_json(&app, &state).await
}

#[tauri::command]
pub async fn skills_export_inventory_file(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
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

async fn build_inventory_json(
    app: &tauri::AppHandle,
    state: &SqliteDbState,
) -> Result<String, String> {
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
                    tags: skill.tags,
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
    state: State<'_, SqliteDbState>,
    inventoryJson: String,
) -> Result<SkillInventoryPreviewDto, String> {
    preview_inventory_import(&state, &inventoryJson).await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_preview_inventory_import_file(
    state: State<'_, SqliteDbState>,
    filePath: String,
) -> Result<SkillInventoryPreviewDto, String> {
    let raw = read_inventory_file(&filePath)?;
    preview_inventory_import(&state, &raw).await
}

async fn reconcile_inventory_skill_tools<R: Runtime>(
    app: &AppHandle<R>,
    state: &SqliteDbState,
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
    let needs_new_sync = desired_tools
        .iter()
        .any(|tool| !current_tool_set.contains(tool));
    let source_path = if needs_new_sync {
        Some(resolve_skill_source_path(app, state, &skill).await?)
    } else {
        resolve_skill_source_path_for_cleanup(app, state, &skill).await
    };

    for target in current_targets {
        if desired_tool_set.contains(&target.tool) {
            continue;
        }
        if let Some(source_path) = source_path.as_deref() {
            remove_skill_target_best_effort(&skill, source_path, &target);
        }
        skill_store::delete_skill_target(state, skill_id, &target.tool).await?;
    }

    if desired_tools.is_empty() {
        return Ok(());
    }

    if !skill.management_enabled {
        return Err(format!("SKILL_DISABLED|{}", skill_id));
    }

    if !needs_new_sync {
        return Ok(());
    }

    let Some(source_path) = source_path.as_deref() else {
        return Err(format!(
            "Failed to resolve source path for skill: {}",
            skill.name
        ));
    };

    for tool in desired_tools {
        if current_tool_set.contains(&tool) {
            continue;
        }
        sync_skill_to_tool_record(state, &skill, &tool, source_path, true, custom_tools).await?;
    }

    Ok(())
}

async fn preflight_inventory_tool_sync(
    state: &SqliteDbState,
    skill: &Skill,
    tool: &str,
    source_path: &Path,
    custom_tools: &[CustomTool],
) -> Result<(), String> {
    let runtime_adapter =
        runtime_adapter_by_key(tool, custom_tools).ok_or_else(|| "unknown tool".to_string())?;

    if !runtime_adapter.is_custom
        && !is_tool_installed_with_state_async(state.db(), &runtime_adapter)
            .await
            .unwrap_or(false)
    {
        let skills_path =
            resolve_runtime_skills_path_with_state_async(state.db(), &runtime_adapter)
                .await
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
        return Err(format!(
            "TOOL_NOT_INSTALLED|{}|{}",
            runtime_adapter.key, skills_path
        ));
    }

    let tool_root = resolve_runtime_skills_path_with_state_async(state.db(), &runtime_adapter)
        .await
        .map_err(|e| format_error(e))?;
    let target = tool_root.join(&skill.name);
    let force_copy = tool.eq_ignore_ascii_case("cursor") || runtime_adapter.force_copy;
    validate_skill_sync_target(source_path, &target, force_copy).map_err(format_error)
}

async fn preflight_inventory_apply<R: Runtime>(
    app: &AppHandle<R>,
    state: &SqliteDbState,
    inventory: &SkillInventoryJson,
    local_skills: &[Skill],
    custom_tools: &[CustomTool],
) -> Result<(), String> {
    for item in &inventory.skills {
        if !item.enabled {
            continue;
        }
        let Some(skill) = match_inventory_skill(item, local_skills) else {
            continue;
        };
        let desired_tools = normalize_tool_ids(&item.enabled_tools);
        if desired_tools.is_empty() {
            continue;
        }

        let current_targets = skill_store::get_skill_targets(state, &skill.id).await?;
        let current_tool_set: HashSet<String> = current_targets
            .iter()
            .map(|target| target.tool.clone())
            .collect();
        let tools_to_sync: Vec<String> = desired_tools
            .into_iter()
            .filter(|tool| !current_tool_set.contains(tool))
            .collect();
        if tools_to_sync.is_empty() {
            continue;
        }

        let source_path = resolve_skill_source_path(app, state, skill).await?;
        for tool in tools_to_sync {
            preflight_inventory_tool_sync(state, skill, &tool, &source_path, custom_tools).await?;
        }
    }

    Ok(())
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_apply_inventory_import<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SqliteDbState>,
    inventoryJson: String,
) -> Result<SkillInventoryPreviewDto, String> {
    let preview = preview_inventory_import(&state, &inventoryJson).await?;
    if !preview.valid {
        return Ok(preview);
    }

    let inventory = parse_inventory(&inventoryJson)?;
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let local_skills = skill_store::get_managed_skills(&state).await?;
    preflight_inventory_apply(&app, &state, &inventory, &local_skills, &custom_tools).await?;

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
        .map(|group| (group.name.trim().to_lowercase(), group.id))
        .collect();

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
            Some(item.tags.clone()),
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
            let source_path = resolve_skill_source_path_for_cleanup(&app, &state, skill).await;
            remove_skill_targets_best_effort(&state, skill, source_path.as_deref()).await?;
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
        let previous_tools = disabled_previous_tools_for_skill(skill);
        let source_path = resolve_skill_source_path_for_cleanup(&app, &state, skill).await;
        remove_skill_targets_best_effort(&state, skill, source_path.as_deref()).await?;
        skill_store::disable_skill_with_previous_tools(&state, &skill.id, previous_tools).await?;
        skill_store::update_skill_metadata(&state, &skill.id, None, skill.user_note.clone(), None)
            .await?;
    }

    let _ = app.emit("skills-changed", "window");
    Ok(preview)
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn skills_apply_inventory_import_file<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SqliteDbState>,
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
    state: &SqliteDbState,
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
pub async fn skills_get_repos(
    state: State<'_, SqliteDbState>,
) -> Result<Vec<SkillRepoDto>, String> {
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
    state: State<'_, SqliteDbState>,
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
    state: State<'_, SqliteDbState>,
    owner: String,
    name: String,
) -> Result<(), String> {
    skill_store::delete_skill_repo(&state, &owner, &name).await
}

#[tauri::command]
pub async fn skills_init_default_repos(state: State<'_, SqliteDbState>) -> Result<usize, String> {
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

pub async fn resync_all_skills_internal<R: Runtime>(
    app: AppHandle<R>,
    state: &SqliteDbState,
) -> Result<Vec<String>, String> {
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let skills = skill_store::get_managed_skills(&state).await?;
    let central_dir = resolve_central_repo_path(&app, state)
        .await
        .map_err(|e| format_error(e))?;

    let mut synced: Vec<String> = Vec::new();

    for mut skill in skills {
        if !skill.management_enabled {
            continue;
        }

        // Resolve central_path (handles cross-platform legacy paths)
        let central_path = resolve_skill_central_path(&skill.central_path, &central_dir);
        if !central_path.exists() {
            continue;
        }
        refresh_central_skill_hash_if_needed(state, &mut skill, &central_path).await?;

        // Re-sync to each enabled tool
        for tool_key in &skill.enabled_tools {
            let runtime_adapter = match runtime_adapter_by_key(tool_key, &custom_tools) {
                Some(a) => a,
                None => continue,
            };

            // Skip if tool not installed (for non-custom tools)
            if !runtime_adapter.is_custom
                && !is_tool_installed_with_state_async(state, &runtime_adapter)
                    .await
                    .unwrap_or(false)
            {
                continue;
            }

            let tool_root =
                match resolve_runtime_skills_path_with_state_async(state, &runtime_adapter).await {
                    Ok(p) => p,
                    Err(_) => continue,
                };

            let target = tool_root.join(&skill.name);
            let previous_target = skill_store::get_skill_target(&state, &skill.id, tool_key)
                .await
                .ok()
                .flatten();

            // When source (central repo) and target resolve to the same path
            // (e.g. shared_agents == central repo), skip gracefully — same guard
            // as sync_skill_to_tool_record.
            if source_target_overlap(&central_path, &target) {
                let record = SkillTarget {
                    tool: tool_key.clone(),
                    target_path: target.to_string_lossy().to_string(),
                    mode: "skip".to_string(),
                    status: "ok".to_string(),
                    error_message: None,
                    synced_at: Some(now_ms()),
                };
                let _ = skill_store::upsert_skill_target(&state, &skill.id, &record).await;
                synced.push(format!("{}:{}", skill.name, tool_key));
                continue;
            }

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
    state: &SqliteDbState,
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
    state: State<'_, SqliteDbState>,
) -> Result<Vec<String>, String> {
    resync_all_skills_internal(app, state.inner()).await
}

const SKILL_DOCUMENT_READ_CAP_BYTES: usize = 128 * 1024;
const SKILL_DOCUMENT_HARD_CAP_BYTES: u64 = (SKILL_DOCUMENT_READ_CAP_BYTES * 10) as u64;

fn read_skill_document_file(path: &Path, filename: &str) -> Option<SkillDocumentDto> {
    let file_path = path.join(filename);
    let stat = std::fs::metadata(&file_path).ok()?;
    if !stat.is_file() {
        return None;
    }

    // Skip files that look like binaries / are unreasonably huge up-front.
    if stat.len() > SKILL_DOCUMENT_HARD_CAP_BYTES {
        return Some(SkillDocumentDto {
            filename: filename.to_string(),
            content: String::new(),
            truncated: true,
        });
    }

    let bytes = std::fs::read(&file_path).ok()?;
    if bytes.len() > SKILL_DOCUMENT_READ_CAP_BYTES {
        let text = String::from_utf8_lossy(&bytes[..SKILL_DOCUMENT_READ_CAP_BYTES]);
        return Some(SkillDocumentDto {
            filename: filename.to_string(),
            content: text.into_owned(),
            truncated: true,
        });
    }

    let text = String::from_utf8_lossy(&bytes);
    Some(SkillDocumentDto {
        filename: filename.to_string(),
        content: text.into_owned(),
        truncated: false,
    })
}

/// Read the Skill's central SKILL.md and README.md content for the detail panel.
/// The command is read-only and never mutates the record or central repo.
#[tauri::command]
pub async fn skills_get_skill_documents(
    app: tauri::AppHandle,
    state: State<'_, SqliteDbState>,
    skill_id: String,
) -> Result<Vec<SkillDocumentDto>, String> {
    let Some(skill) = skill_store::get_skill_by_id(&state, &skill_id).await? else {
        return Ok(Vec::new());
    };

    let central_dir = resolve_central_repo_path(&app, &state)
        .await
        .map_err(|e| format_error(e))?;
    let central_path = resolve_skill_central_path(&skill.central_path, &central_dir);

    let candidates = ["SKILL.md", "README.md"];
    let documents = candidates
        .into_iter()
        .filter_map(|name| read_skill_document_file(&central_path, name))
        .collect::<Vec<_>>();

    Ok(documents)
}
