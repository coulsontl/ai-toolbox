use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use super::central_repo::resolve_central_repo_path;
use super::content_hash::hash_dir;
use super::skill_store;
use super::tool_adapters::{get_all_tool_adapters, RuntimeToolAdapter};
use super::types::{OnboardingGroup, OnboardingPlan, OnboardingVariant};
use crate::coding::tools::claude_plugins::PluginInfo;
use crate::SqliteDbState;

/// Extra skill source directories to scan during onboarding discovery.
/// These are third-party skill stores outside the built-in tool adapters.
/// To add a new source, just append an entry here.
struct ExtraSkillSource {
    key: &'static str,
    display_name: &'static str,
    /// Path to the skills directory (supports ~/ and %APPDATA%/ prefixes)
    skills_dir: &'static str,
}

const EXTRA_SKILL_SOURCES: &[ExtraSkillSource] = &[ExtraSkillSource {
    key: "cc_switch",
    display_name: "CC Switch",
    skills_dir: "~/.cc-switch/skills",
}];

/// Build an onboarding plan by scanning installed tools for existing skills
pub async fn build_onboarding_plan(
    app: &tauri::AppHandle,
    state: &SqliteDbState,
) -> Result<OnboardingPlan> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("failed to resolve home directory"))?;
    let central = resolve_central_repo_path(app, state).await?;

    // Get custom tools
    let custom_tools = skill_store::get_custom_tools(state)
        .await
        .unwrap_or_default();

    // Get already managed target paths to exclude them
    let managed_targets = skill_store::list_all_skill_target_paths(state)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(tool, path)| managed_target_key(&tool, Path::new(&path)))
        .collect::<std::collections::HashSet<_>>();

    // Get already managed skill names to exclude them (covers plugin skills
    // whose source path doesn't match any managed target path)
    let managed_names = skill_store::get_managed_skills(state)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.name)
        .collect::<std::collections::HashSet<_>>();
    let claude_plugins =
        crate::coding::tools::claude_plugins::get_installed_plugins(&state.db()).await;

    // Run the blocking file system operations in a dedicated thread pool
    // to avoid blocking the tokio async runtime
    tokio::task::spawn_blocking(move || {
        let filter_ctx = FilterContext {
            exclude_root: Some(&central),
            managed_targets: Some(&managed_targets),
            managed_names: Some(&managed_names),
        };
        build_onboarding_plan_in_home(&home, &filter_ctx, &custom_tools, &claude_plugins)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {}", e))?
}

fn build_onboarding_plan_in_home(
    _home: &Path,
    filter_ctx: &FilterContext<'_>,
    custom_tools: &[super::types::CustomTool],
    claude_plugins: &[PluginInfo],
) -> Result<OnboardingPlan> {
    // Get all adapters (built-in + custom)
    let adapters = get_all_tool_adapters(custom_tools);
    let mut all_detected: Vec<super::types::DetectedSkill> = Vec::new();
    let mut scanned = 0usize;

    for adapter in &adapters {
        // Check if tool is installed using path_utils
        let detect_path =
            crate::coding::tools::path_utils::resolve_storage_path(&adapter.relative_detect_dir);
        if detect_path.is_none() || !detect_path.as_ref().unwrap().exists() {
            continue;
        }
        scanned += 1;
        // Resolve skills directory using path_utils to handle ~/  and %APPDATA%/ paths correctly
        let dir =
            crate::coding::tools::path_utils::resolve_storage_path(&adapter.relative_skills_dir);
        if let Some(skills_dir) = dir {
            let detected = scan_runtime_tool_dir(adapter, &skills_dir)?;
            all_detected.extend(filter_detected(detected, filter_ctx));
        }
    }

    // Scan extra skill directories (third-party skill stores)
    for source in EXTRA_SKILL_SOURCES {
        let skills_dir = crate::coding::tools::path_utils::resolve_storage_path(source.skills_dir);
        if let Some(dir) = skills_dir {
            if dir.exists() {
                let adapter = RuntimeToolAdapter {
                    key: source.key.to_string(),
                    display_name: source.display_name.to_string(),
                    relative_skills_dir: source.skills_dir.to_string(),
                    relative_detect_dir: source.skills_dir.to_string(),
                    is_custom: false,
                    force_copy: false,
                };
                scanned += 1;
                let detected = scan_runtime_tool_dir(&adapter, &dir)?;
                all_detected.extend(filter_detected(detected, filter_ctx));
            }
        }
    }

    // Scan Claude Code plugins for skills
    for plugin in claude_plugins {
        let skills_dir = plugin.install_path.join("skills");
        if !skills_dir.exists() {
            continue;
        }
        let adapter = RuntimeToolAdapter {
            key: format!("plugin::{}", plugin.plugin_id),
            display_name: format!("Plugin: {}", plugin.display_name),
            relative_skills_dir: skills_dir.to_string_lossy().to_string(),
            relative_detect_dir: skills_dir.to_string_lossy().to_string(),
            is_custom: false,
            force_copy: true,
        };
        scanned += 1;
        let detected = scan_runtime_tool_dir(&adapter, &skills_dir)?;
        all_detected.extend(filter_detected(detected, filter_ctx));
    }

    let mut grouped: HashMap<String, Vec<OnboardingVariant>> = HashMap::new();
    for skill in all_detected.iter() {
        let fingerprint = hash_dir(&skill.path).ok();
        let entry = grouped.entry(skill.name.clone()).or_default();
        entry.push(OnboardingVariant {
            tool: skill.tool.clone(),
            tool_display: skill.tool_display.clone(),
            name: skill.name.clone(),
            path: skill.path.to_string_lossy().to_string(),
            fingerprint,
            is_link: skill.is_link,
            link_target: skill
                .link_target
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            conflicting_tools: Vec::new(), // Will be calculated later
        });
    }

    let groups: Vec<OnboardingGroup> = grouped
        .into_iter()
        .map(|(name, mut variants)| {
            // Build fingerprint -> tools mapping (owned data to avoid borrow conflict)
            let mut fingerprint_tools: HashMap<String, Vec<String>> = HashMap::new();
            for v in &variants {
                if let Some(ref fp) = v.fingerprint {
                    fingerprint_tools
                        .entry(fp.clone())
                        .or_default()
                        .push(v.tool.clone());
                }
            }

            let uniq_fingerprints = fingerprint_tools.len();
            let has_conflict = uniq_fingerprints > 1;

            // Calculate conflicting tools for each variant
            for v in &mut variants {
                if let Some(ref my_fp) = v.fingerprint {
                    // Find tools with different fingerprints
                    let mut conflicting: Vec<String> = Vec::new();
                    for (fp, tools) in &fingerprint_tools {
                        if fp != my_fp {
                            conflicting.extend(tools.iter().cloned());
                        }
                    }
                    v.conflicting_tools = conflicting;
                }
            }

            OnboardingGroup {
                name,
                has_conflict,
                variants,
            }
        })
        .collect();

    Ok(OnboardingPlan {
        total_tools_scanned: scanned,
        total_skills_found: all_detected.len(),
        groups,
    })
}

/// Exclusion context for filtering detected skills during onboarding scan.
#[derive(Default)]
struct FilterContext<'a> {
    exclude_root: Option<&'a Path>,
    managed_targets: Option<&'a std::collections::HashSet<String>>,
    managed_names: Option<&'a std::collections::HashSet<String>>,
}

fn filter_detected(
    detected: Vec<super::types::DetectedSkill>,
    ctx: &FilterContext<'_>,
) -> Vec<super::types::DetectedSkill> {
    if ctx.exclude_root.is_none() && ctx.managed_targets.is_none() && ctx.managed_names.is_none() {
        return detected;
    }
    detected
        .into_iter()
        .filter(|skill| {
            if let Some(exclude_root) = ctx.exclude_root {
                if is_under(&skill.path, exclude_root) {
                    return false;
                }
                if let Some(target) = &skill.link_target {
                    if is_under(target, exclude_root) {
                        return false;
                    }
                }
            }
            if let Some(exclude) = ctx.managed_targets {
                if exclude.contains(&managed_target_key(&skill.tool, &skill.path)) {
                    return false;
                }
            }
            // Exclude skills whose name is already managed in the database
            // (covers plugin skills whose source path doesn't match any managed target)
            if let Some(names) = ctx.managed_names {
                if names.contains(&skill.name) {
                    return false;
                }
            }
            true
        })
        .collect()
}

fn is_under(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

fn managed_target_key(tool: &str, path: &Path) -> String {
    let tool = tool.to_ascii_lowercase();
    let normalized = normalize_path_for_key(path);
    format!("{tool}\n{normalized}")
}

fn normalize_path_for_key(path: &Path) -> String {
    let normalized: std::path::PathBuf = path.components().collect();
    let s = normalized.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        s.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        s
    }
}

/// Scan a tool directory for skills (using RuntimeToolAdapter)
fn scan_runtime_tool_dir(
    adapter: &RuntimeToolAdapter,
    dir: &Path,
) -> Result<Vec<super::types::DetectedSkill>> {
    let mut results = Vec::new();
    if !dir.exists() {
        return Ok(results);
    }

    // Ignore paths containing our central repo
    let ignore_hint = "Application Support/com.ai-toolbox/skills";

    for entry in std::fs::read_dir(dir).with_context(|| format!("read dir {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let is_dir = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        // Skip system directories for codex
        if adapter.key == "codex" && name == ".system" {
            continue;
        }

        let (is_link, link_target) = detect_link(&path);
        if path.to_string_lossy().contains(ignore_hint)
            || link_target
                .as_ref()
                .map(|p| p.to_string_lossy().contains(ignore_hint))
                .unwrap_or(false)
        {
            continue;
        }

        results.push(super::types::DetectedSkill {
            tool: adapter.key.clone(),
            tool_display: adapter.display_name.clone(),
            name,
            path,
            is_link,
            link_target,
        });
    }

    Ok(results)
}

fn detect_link(path: &Path) -> (bool, Option<std::path::PathBuf>) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = std::fs::read_link(path).ok();
            (true, target)
        }
        _ => {
            let target = std::fs::read_link(path).ok();
            if target.is_some() {
                (true, target)
            } else {
                (false, None)
            }
        }
    }
}
