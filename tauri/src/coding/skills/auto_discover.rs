//! Background discover pass for Skills (issue #446).
//!
//! Every 60s tick scans the configured source directories for Skills that the
//! DB does not know yet, adopts them, and syncs only those newly adopted Skills
//! to the configured target tools. Existing Skills keep their tool bindings:
//! a pass must never resurrect a tool the user un-synced by hand.
//!
//! Sources are stored as stable identities rather than absolute paths, so a
//! changed tool runtime root or a cross-machine restore re-resolves them:
//!
//! | id                  | meaning                                         |
//! |---------------------|-------------------------------------------------|
//! | `central`           | the Skills central repo (`central_repo_path`)   |
//! | `tool:<key>`        | a tool's runtime skills dir (built-in or custom)|
//! | `extra:<key>`       | third-party store from `onboarding` extras      |
//! | `plugin:<id>`       | a Claude Code plugin's `skills/` dir            |
//! | `custom:<path>`     | a user-added directory                          |
//!
//! Only the central repo adopts in place (`source_type = "central"`), like the
//! existing central-repo scan. Every other source (tool dirs, extras, plugins
//! and user-added directories) copies the Skill into the central repo
//! (`source_type = "local"`, `source_ref` = original directory) through the
//! same installer the manual import flow uses.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::db::SqliteDbState;

use super::central_repo::{resolve_central_repo_path, resolve_skill_central_path};
use super::frontmatter::parse_skill_md_frontmatter;
use super::skill_store;
use super::tool_adapters::{
    get_all_tool_adapters, resolve_runtime_skills_path_with_state_async, RuntimeToolAdapter,
};
use super::types::{AutoSyncRunResultDto, AutoSyncSourceDto, Skill};

/// Built-in third-party stores. Mirrors `onboarding::EXTRA_SKILL_SOURCES`;
/// keep both lists in sync when a store is added.
const EXTRA_SOURCES: &[(&str, &str, &str)] = &[("cc_switch", "CC Switch", "~/.cc-switch/skills")];

const CENTRAL_SOURCE_ID: &str = "central";
const CENTRAL_LABEL: &str = "Skills storage directory";

/// A resolved scan source: identity plus the directory to read right now.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedSource {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub path: PathBuf,
    pub exists: bool,
}

impl ResolvedSource {
    fn status(&self, detected_count: usize, new_count: usize) -> AutoSyncSourceDto {
        AutoSyncSourceDto {
            id: self.id.clone(),
            label: self.label.clone(),
            kind: self.kind.clone(),
            path: self.path.to_string_lossy().to_string(),
            exists: self.exists,
            detected_count,
            new_count,
            locked: self.id == CENTRAL_SOURCE_ID,
        }
    }
}

/// The enabled source set actually used by a pass. `None` (never configured)
/// means central only, and central is always present.
pub(crate) fn effective_source_ids(configured: Option<&[String]>) -> Vec<String> {
    let mut ids: Vec<String> = configured
        .map(|values| values.iter().cloned().collect())
        .unwrap_or_default();
    if !ids.iter().any(|id| id == CENTRAL_SOURCE_ID) {
        ids.insert(0, CENTRAL_SOURCE_ID.to_string());
    }
    ids
}

fn custom_dir_id(path: &str) -> String {
    format!("custom:{}", path.replace('\\', "/"))
}

/// Normalize a user-added directory into portable storage form (`~/...` under
/// the home directory) so the same folder added twice keeps one identity and
/// survives a cross-machine restore.
pub fn normalize_custom_dir(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = super::central_repo::expand_home_path(trimmed).ok()?;
    let normalized =
        crate::coding::tools::path_utils::normalize_path(&expanded.to_string_lossy());
    Some(crate::coding::tools::path_utils::to_storage_path(&normalized))
}

/// Resolve every source the settings page can offer, enabled or not.
/// Disabled sources are resolved too so the list can show their path and count.
pub(crate) async fn list_sources(
    app: &AppHandle<impl Runtime>,
    state: &SqliteDbState,
) -> Result<Vec<ResolvedSource>, String> {
    let prefs = skill_store::get_skill_preferences(state).await?;
    let central = resolve_central_repo_path(app, state)
        .await
        .map_err(|e| format!("{:#}", e))?;

    let mut sources: Vec<ResolvedSource> = vec![ResolvedSource {
        id: CENTRAL_SOURCE_ID.to_string(),
        label: CENTRAL_LABEL.to_string(),
        kind: "central".to_string(),
        path: central.clone(),
        exists: central.is_dir(),
    }];

    let custom_tools = skill_store::get_custom_tools(state)
        .await
        .unwrap_or_default();
    for adapter in get_all_tool_adapters(&custom_tools) {
        sources.push(resolve_tool_source(state, &adapter).await);
    }

    for (key, label, storage_path) in EXTRA_SOURCES {
        let path = crate::coding::tools::path_utils::resolve_storage_path(storage_path)
            .ok_or_else(|| format!("cannot resolve {storage_path}"))?;
        let exists = path.is_dir();
        sources.push(ResolvedSource {
            id: format!("extra:{key}"),
            label: label.to_string(),
            kind: "extra".to_string(),
            path,
            exists,
        });
    }

    let plugins = crate::coding::tools::claude_plugins::get_installed_plugins(&state.db()).await;
    for plugin in plugins {
        let path = plugin.install_path.join("skills");
        let exists = path.is_dir();
        sources.push(ResolvedSource {
            id: format!("plugin:{}", plugin.plugin_id),
            label: format!("Plugin: {}", plugin.display_name),
            kind: "plugin".to_string(),
            path,
            exists,
        });
    }

    for stored in prefs.auto_sync_custom_dirs.unwrap_or_default() {
        let Some(path) = super::central_repo::expand_home_path(&stored).ok() else {
            continue;
        };
        let exists = path.is_dir();
        sources.push(ResolvedSource {
            id: custom_dir_id(&stored),
            label: stored.clone(),
            kind: "custom".to_string(),
            path,
            exists,
        });
    }

    sort_sources_for_display(&mut sources);
    Ok(sources)
}

/// Order the dialog's source list by how often a user touches it: the central
/// repo first, then the two shared stores, then the two most-used CLIs. The UI
/// keeps exactly these five pinned and folds the rest, so this order is the
/// visible list — do not sort it alphabetically.
fn sort_sources_for_display(sources: &mut [ResolvedSource]) {
    const PINNED: &[&str] = &[
        CENTRAL_SOURCE_ID,
        "tool:shared_agents",
        "extra:cc_switch",
        "tool:claude_code",
        "tool:codex",
    ];
    let rank = |id: &str| {
        PINNED
            .iter()
            .position(|pinned| *pinned == id)
            .unwrap_or(PINNED.len())
    };
    sources.sort_by_key(|source| rank(&source.id));
}

/// A tool source is offered even when the CLI is not installed: its directory
/// simply does not exist yet, and the dialog shows it as unavailable.
async fn resolve_tool_source(
    state: &SqliteDbState,
    adapter: &RuntimeToolAdapter,
) -> ResolvedSource {
    let path = resolve_runtime_skills_path_with_state_async(state.db(), adapter)
        .await
        .unwrap_or_else(|_| PathBuf::new());
    ResolvedSource {
        id: format!("tool:{}", adapter.key),
        label: adapter.display_name.clone(),
        kind: "tool".to_string(),
        exists: path.is_dir(),
        path,
    }
}

/// Skills found in one directory. Only the manifest check + frontmatter name
/// are read here; content hashing belongs to the adopt path so a steady-state
/// tick stays cheap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoundSkill {
    /// Directory name, used as the relative path inside the source.
    pub dir_name: String,
    /// Name from SKILL.md frontmatter, falling back to the directory name.
    pub name: String,
    pub path: PathBuf,
}

/// Scan one source directory for Skill subdirectories.
///
/// Rules match the existing central-repo scan: hidden entries are skipped,
/// recursion stops at the first directory containing `SKILL.md`, and symlinked
/// directories are followed once (visited set by canonical path).
pub(crate) fn scan_source_dir(root: &Path) -> Vec<FoundSkill> {
    let mut found = Vec::new();
    if !root.is_dir() {
        return found;
    }
    let mut visited = HashSet::new();
    scan_dir_recursive(root, root, &mut visited, &mut found);
    found.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.dir_name.cmp(&right.dir_name))
    });
    found
}

fn scan_dir_recursive(
    root: &Path,
    dir: &Path,
    visited: &mut HashSet<PathBuf>,
    found: &mut Vec<FoundSkill>,
) {
    let Ok(real_dir) = std::fs::canonicalize(dir) else {
        return;
    };
    if !visited.insert(real_dir) {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if let Some(skill) = detect_skill_in_dir(root, &path) {
            found.push(skill);
            continue;
        }
        scan_dir_recursive(root, &path, visited, found);
    }
}

fn detect_skill_in_dir(root: &Path, dir: &Path) -> Option<FoundSkill> {
    let manifest = dir.join("SKILL.md");
    if !manifest.is_file() {
        return None;
    }
    let relative = dir.strip_prefix(root).ok()?;
    let dir_name = relative.to_string_lossy().replace('\\', "/");
    if dir_name.is_empty() {
        // A root-level SKILL.md is not a managed Skill (same rule as the
        // central-repo scan's `root_skill_warning`).
        return None;
    }
    let (manifest_name, _) = parse_skill_md_frontmatter(&manifest);
    let name = manifest_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| dir_name.clone())
        });
    Some(FoundSkill {
        dir_name,
        name,
        path: dir.to_path_buf(),
    })
}

/// Split found Skills into name-unique entries and duplicate-name groups.
/// Duplicates are skipped whole: guessing which of two same-named directories
/// is authoritative is not something a background pass should do.
pub(crate) fn partition_by_unique_name(found: Vec<FoundSkill>) -> (Vec<FoundSkill>, Vec<String>) {
    let mut by_name: std::collections::HashMap<String, Vec<FoundSkill>> =
        std::collections::HashMap::new();
    for skill in found {
        by_name
            .entry(skill.name.trim().to_lowercase())
            .or_default()
            .push(skill);
    }

    let mut unique = Vec::new();
    let mut conflicts = Vec::new();
    for (_, mut group) in by_name {
        if group.len() == 1 {
            unique.push(group.remove(0));
        } else {
            conflicts.push(group[0].name.clone());
        }
    }
    unique.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    conflicts.sort();
    (unique, conflicts)
}

fn managed_name_keys(skills: &[super::types::Skill]) -> HashSet<String> {
    skills
        .iter()
        .map(|skill| skill.name.trim().to_lowercase())
        .collect()
}

/// Build the settings-page source list: every candidate source with its
/// current Skill count and how many of them are not managed yet.
pub(crate) async fn build_source_list(
    app: &AppHandle<impl Runtime>,
    state: &SqliteDbState,
) -> Result<Vec<AutoSyncSourceDto>, String> {
    let managed = managed_name_keys(&skill_store::get_managed_skills(state).await?);
    let sources = list_sources(app, state).await?;

    Ok(sources
        .into_iter()
        .map(|source| {
            let found = scan_source_dir(&source.path);
            let detected_count = found.len();
            let new_count = found
                .iter()
                .filter(|skill| !managed.contains(&skill.name.trim().to_lowercase()))
                .count();
            source.status(detected_count, new_count)
        })
        .collect())
}

/// Cheap fingerprint for one Skill directory: newest mtime + entry count.
/// Deliberately not a content hash: this runs for every discovered Skill on
/// every tick, while the real hash comparison happens in the update path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SkillFingerprint {
    pub newest_mtime_ms: i64,
    pub entry_count: u32,
}

pub(crate) fn fingerprint_skill_dir(dir: &Path) -> SkillFingerprint {
    let mut newest: i64 = 0;
    let mut count: u32 = 0;
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
        .flatten()
    {
        if entry.depth() == 0 {
            continue;
        }
        count = count.saturating_add(1);
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH) {
                    newest = newest.max(elapsed.as_millis() as i64);
                }
            }
        }
    }
    SkillFingerprint {
        newest_mtime_ms: newest,
        entry_count: count,
    }
}

/// Persisted per-Skill fingerprints, stored in the `skill_settings` singleton
/// (`auto_sync_fingerprints`) rather than a side file: the DB is the single
/// store for skill state and it already round-trips through backup/restore.
const FINGERPRINTS_KEY: &str = "auto_sync_fingerprints";

fn load_fingerprints(state: &SqliteDbState) -> HashMap<String, SkillFingerprint> {
    let raw = state
        .with_conn(|conn| crate::db::helpers::db_get(conn, crate::db::schema::DbTable::SkillEntity(crate::db::schema::SkillEntityKind::Settings), "skills"))
        .ok()
        .flatten();
    let Some(value) = raw
        .as_ref()
        .and_then(|record| record.get(FINGERPRINTS_KEY))
        .and_then(|value| value.as_object())
    else {
        return HashMap::new();
    };

    value
        .iter()
        .filter_map(|(name, entry)| {
            let newest = entry.get("newest_mtime_ms").and_then(|v| v.as_i64())?;
            let count = entry.get("entry_count").and_then(|v| v.as_u64())?;
            Some((
                name.clone(),
                SkillFingerprint {
                    newest_mtime_ms: newest,
                    entry_count: count as u32,
                },
            ))
        })
        .collect()
}

fn save_fingerprints(
    state: &SqliteDbState,
    fingerprints: &HashMap<String, SkillFingerprint>,
) -> Result<(), String> {
    let payload: serde_json::Map<String, serde_json::Value> = fingerprints
        .iter()
        .map(|(name, entry)| {
            (
                name.clone(),
                serde_json::json!({
                    "newest_mtime_ms": entry.newest_mtime_ms,
                    "entry_count": entry.entry_count,
                }),
            )
        })
        .collect();
    super::central_repo::merge_skill_settings_sqlite(
        state,
        serde_json::json!({ FINGERPRINTS_KEY: payload }),
    )
}

/// Keeps background ticks from overlapping each other. Only the scheduler takes
/// it: a tick that finds it taken skips the round. User-triggered passes never
/// touch it — they serialize on the single-writer database instead.
static PASS_RUNNING: AtomicBool = AtomicBool::new(false);
static SCHEDULER_STARTED: AtomicBool = AtomicBool::new(false);

/// Clears `PASS_RUNNING` on drop, including when the pass unwinds.
struct PassRunningGuard;

impl PassRunningGuard {
    fn acquire() -> Option<Self> {
        PASS_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for PassRunningGuard {
    fn drop(&mut self) {
        PASS_RUNNING.store(false, Ordering::SeqCst);
    }
}

const GLOBAL_INITIAL_DELAY: Duration = Duration::from_secs(60);
const TICK_GRANULARITY: Duration = Duration::from_secs(60);

/// Start the discover loop once per process.
pub fn start(app: AppHandle) {
    if SCHEDULER_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GLOBAL_INITIAL_DELAY).await;
        loop {
            tokio::time::sleep(TICK_GRANULARITY).await;
            let Some(db) = app.try_state::<SqliteDbState>() else {
                continue;
            };
            let prefs = match skill_store::get_skill_preferences(db.inner()).await {
                Ok(prefs) => prefs,
                Err(_) => continue,
            };
            if !prefs.auto_sync_enabled {
                continue;
            }
            // One pass at a time: skip this tick when the previous one is still
            // running (large sources on a slow disk can outlast the interval).
            // The guard clears the flag even if the pass panics, otherwise one
            // panic would stop every later tick for the life of the process.
            let Some(_guard) = PassRunningGuard::acquire() else {
                continue;
            };
            // Background passes stay silent: no progress events, failures log only.
            let outcome = run_pass(&app, db.inner(), false).await;
            drop(_guard);
            match outcome {
                Ok(result) => {
                    if result.adopted > 0 || result.refreshed > 0 {
                        log::debug!(
                            "skills auto-discover: adopted {}, refreshed {}, synced {} targets",
                            result.adopted,
                            result.refreshed,
                            result.synced_targets
                        );
                    }
                }
                Err(error) => log::debug!("skills auto-discover pass failed: {error}"),
            }
        }
    });
}

/// One discover pass: scan every enabled source, adopt unknown Skills, refresh
/// Skills whose content changed, and sync only the newly adopted ones.
///
/// `emit_events` is false for the background tick (a scheduled pass must not
/// push UI work) and true for the dialog's "scan now", which refreshes the list
/// and the tray with the result the user just asked for.
pub async fn run_pass(
    app: &AppHandle<impl Runtime>,
    state: &SqliteDbState,
    emit_events: bool,
) -> Result<AutoSyncRunResultDto, String> {
    let result = run_pass_inner(app, state).await?;
    if emit_events && (result.adopted > 0 || result.refreshed > 0) {
        let _ = app.emit("skills-changed", "auto");
        if let Err(error) = crate::tray::refresh_tray_menus(app).await {
            log::debug!("skills auto-discover tray refresh failed: {error}");
        }
    }
    Ok(result)
}

async fn run_pass_inner(
    app: &AppHandle<impl Runtime>,
    state: &SqliteDbState,
) -> Result<AutoSyncRunResultDto, String> {
    let prefs = skill_store::get_skill_preferences(state).await?;
    let enabled_ids: HashSet<String> = effective_source_ids(prefs.auto_sync_sources.as_deref())
        .into_iter()
        .collect();
    let central_dir = resolve_central_repo_path(app, state)
        .await
        .map_err(|e| format!("{:#}", e))?;

    let managed_before = skill_store::get_managed_skills(state).await?;
    let mut managed_by_name: HashMap<String, Skill> = managed_before
        .into_iter()
        .map(|skill| (skill.name.trim().to_lowercase(), skill))
        .collect();

    let target_tools = resolve_target_tools(prefs.auto_sync_target_tools.as_deref());
    let mut fingerprints = load_fingerprints(state);
    let mut result = AutoSyncRunResultDto::default();
    let mut changed_skill_ids: Vec<String> = Vec::new();

    let sources = list_sources(app, state).await?;
    for source in sources
        .into_iter()
        .filter(|source| enabled_ids.contains(&source.id))
    {
        if !source.exists {
            continue;
        }
        let (unique, conflicts) = partition_by_unique_name(scan_source_dir(&source.path));
        result.discovered += unique.len();
        result.skipped_conflicts.extend(conflicts);

        for found in unique {
            let key = found.name.trim().to_lowercase();
            let fingerprint = fingerprint_skill_dir(&found.path);

            if let Some(existing) = managed_by_name.get(&key) {
                // A Skill can be visible from several sources (its own folder,
                // plus every tool that links to it). Only the source that owns
                // it may judge its content, otherwise two sources with the same
                // name would each see the other's fingerprint and refresh it on
                // every tick. The key therefore carries the source id.
                if !source_owns_skill(&source, &found.path, existing) {
                    continue;
                }
                let state_key = fingerprint_key(&source.id, &key);
                let must_refresh = fingerprints.get(&state_key) != Some(&fingerprint);

                fingerprints.insert(state_key, fingerprint);
                if must_refresh && existing.management_enabled {
                    changed_skill_ids.push(existing.id.clone());
                }
                continue;
            }

            match adopt_found_skill(app, state, &central_dir, &source, &found).await {
                Ok(Some(skill)) => {
                    result.adopted += 1;
                    fingerprints.insert(fingerprint_key(&source.id, &key), fingerprint);
                    managed_by_name.insert(key, skill.clone());
                    // Only newly adopted Skills are distributed, and never back
                    // into the tool directory they came from.
                    let sync_tools: Vec<String> = target_tools
                        .iter()
                        .filter(|tool| source.id != format!("tool:{tool}"))
                        .cloned()
                        .collect();
                    match sync_new_skill(app, state, &skill, &sync_tools).await {
                        Ok(count) => result.synced_targets += count,
                        Err(error) => result.errors.push(format!("{}: {}", skill.name, error)),
                    }
                }
                Ok(None) => result.skipped_names.push(found.name.clone()),
                Err(error) => result.errors.push(format!("{}: {}", found.name, error)),
            }
        }
    }

    // Refresh content-changed Skills through the existing update chain. That
    // path re-syncs copy-mode targets and leaves symlink/junction targets alone.
    // The same Skill may be seen from several sources; refresh it once.
    let mut refreshed_ids = HashSet::new();
    for skill_id in changed_skill_ids {
        if !refreshed_ids.insert(skill_id.clone()) {
            continue;
        }
        match super::commands::update_managed_skill_internal(app, state, &skill_id).await {
            Ok(_) => result.refreshed += 1,
            Err(error) => result.errors.push(format!("{skill_id}: {error}")),
        }
    }

    save_fingerprints(state, &fingerprints)?;
    Ok(result)
}

/// Fingerprint state is keyed by `source_id + "\n" + lowercase name`: the same
/// Skill name can be visible from several sources (its own folder plus every
/// tool that links to it), and only the owning source may judge its content.
fn fingerprint_key(source_id: &str, name_key: &str) -> String {
    format!("{source_id}\n{name_key}")
}

/// Whether this source is the one that can legitimately report a content change
/// for an already-managed Skill.
///
/// It can when the discovered directory actually backs the record:
///   * for the central source, a Skill adopted in place there
///     (`source_type == "central"`), and
///   * for every other source, a directory that is the record's `source_ref`
///     (resolved through symlinks/junctions, so a tool that merely links the
///     central content never becomes a second authority).
fn source_owns_skill(source: &ResolvedSource, found_path: &Path, skill: &Skill) -> bool {
    // A Skill copied in from another source also lives in the central repo, but
    // its fingerprint was recorded under the source that produced it. Letting
    // the central directory own it would refresh it on every single tick.
    if source.kind == "central" {
        return skill.source_type == "central";
    }
    let Ok(found_real) = std::fs::canonicalize(found_path) else {
        return false;
    };
    if let Some(source_ref) = skill.source_ref.as_deref() {
        if let Ok(reference_real) = std::fs::canonicalize(source_ref) {
            if reference_real == found_real {
                return true;
            }
        }
    }
    false
}

/// The tool every auto-discovered Skill goes to before the user opts into
/// anything else. It is the cross-tool agentskills.io directory, so a fresh
/// install distributes without guessing which CLIs the user has.
pub const DEFAULT_TARGET_TOOL: &str = "shared_agents";

/// Resolve the target tool list. `None` (never configured) means the default
/// target only; an explicit empty list stays empty.
///
/// The default is deliberately not "every installed tool": a background pass
/// must not write into a CLI the user never pointed it at.
fn resolve_target_tools(configured: Option<&[String]>) -> Vec<String> {
    match configured {
        Some(tools) => tools.to_vec(),
        None => vec![DEFAULT_TARGET_TOOL.to_string()],
    }
}

/// Adopt one discovered Skill. Returns `Ok(None)` when the name is already
/// managed (e.g. adopted earlier in the same pass from another source).
async fn adopt_found_skill(
    app: &AppHandle<impl Runtime>,
    state: &SqliteDbState,
    central_dir: &Path,
    source: &ResolvedSource,
    found: &FoundSkill,
) -> Result<Option<Skill>, String> {
    if skill_store::get_skill_by_name(state, &found.name)
        .await?
        .is_some()
    {
        return Ok(None);
    }

    // Only the central repo adopts in place. Every other source (tool dirs,
    // extras, plugins and user-added directories) copies into the central repo,
    // so the original stays an untouched `source_ref`.
    let central_relative = source.kind == "central";

    if central_relative {
        // In-place adoption: record the directory, do not copy or move it.
        let relative = super::commands::normalize_central_relative_path(&found.dir_name)?;
        let source_path = central_dir.join(&relative);
        let Some(name) = detect_skill_name(&source_path) else {
            return Err(format!("No Skill found at {}", source_path.display()));
        };
        if skill_store::get_skill_by_name(state, &name)
            .await?
            .is_some()
        {
            return Ok(None);
        }
        let skill = new_skill_record(
            name,
            "central".to_string(),
            None,
            relative,
            super::content_hash::hash_dir(&source_path).ok(),
        );
        let id = skill_store::upsert_skill(state, &skill).await?;
        return skill_store::get_skill_by_id(state, &id).await;
    }

    // Copy sources land in the central repo as `local` Skills so the original
    // directory stays a plain `source_ref` and later edits can be pulled from
    // it. `overwrite = false` keeps an existing same-named central directory.
    let central_path = central_dir.join(&found.name);
    if central_path.exists() {
        return Ok(None);
    }
    super::installer::install_local_skill_from_selection(app, state, &found.path, ".", false)
        .await
        .map_err(|e| format!("{:#}", e))?;
    Ok(skill_store::get_skill_by_name(state, &found.name).await?)
}

/// Sync a freshly adopted Skill to the configured target tools.
///
/// `overwrite = false` on purpose: a same-named directory the tool already has
/// (e.g. installed by another manager) belongs to the user, and a background
/// pass must not replace it. Failures are collected by the caller.
async fn sync_new_skill(
    app: &AppHandle<impl Runtime>,
    state: &SqliteDbState,
    skill: &Skill,
    tools: &[String],
) -> Result<usize, String> {
    if tools.is_empty() {
        return Ok(0);
    }
    let central_dir = resolve_central_repo_path(app, state)
        .await
        .map_err(|e| format!("{:#}", e))?;
    let source_path = resolve_skill_central_path(&skill.central_path, &central_dir);
    let custom_tools = skill_store::get_custom_tools(state)
        .await
        .unwrap_or_default();

    let mut synced = 0usize;
    let mut failures = Vec::new();
    for tool in tools {
        match super::commands::sync_skill_to_tool_record(
            state,
            skill,
            tool,
            &source_path,
            false,
            &custom_tools,
        )
        .await
        {
            Ok(_) => synced += 1,
            Err(error) => failures.push(format!("{tool}: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(synced)
    } else {
        Err(format!(
            "synced {synced} target(s), failed:\n- {}",
            failures.join("\n- ")
        ))
    }
}

fn detect_skill_name(dir: &Path) -> Option<String> {
    let manifest = dir.join("SKILL.md");
    if !manifest.is_file() {
        return None;
    }
    let (manifest_name, _) = parse_skill_md_frontmatter(&manifest);
    manifest_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            dir.file_name()
                .map(|value| value.to_string_lossy().to_string())
        })
}

fn new_skill_record(
    name: String,
    source_type: String,
    source_ref: Option<String>,
    relative_path: String,
    content_hash: Option<String>,
) -> Skill {
    let now = super::types::now_ms();
    Skill {
        id: String::new(),
        name,
        source_type,
        source_ref,
        source_revision: None,
        central_path: relative_path,
        content_hash,
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
        tags: Vec::new(),
        enabled_tools: Vec::new(),
        sync_details: Some(serde_json::Value::Object(serde_json::Map::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, relative: &str, name: Option<&str>) {
        let dir = root.join(relative);
        std::fs::create_dir_all(&dir).expect("create skill dir");
        let body = match name {
            Some(name) => format!("---\nname: {name}\n---\n\nbody\n"),
            None => "# no frontmatter\n".to_string(),
        };
        std::fs::write(dir.join("SKILL.md"), body).expect("write SKILL.md");
    }

    #[test]
    fn central_is_always_an_enabled_source() {
        assert_eq!(effective_source_ids(None), vec!["central".to_string()]);
        assert_eq!(
            effective_source_ids(Some(&["tool:codex".to_string()])),
            vec!["central".to_string(), "tool:codex".to_string()]
        );
        assert_eq!(
            effective_source_ids(Some(&["central".to_string()])),
            vec!["central".to_string()]
        );
    }

    fn source(id: &str) -> ResolvedSource {
        ResolvedSource {
            id: id.to_string(),
            label: id.to_string(),
            kind: "tool".to_string(),
            path: PathBuf::new(),
            exists: true,
        }
    }

    #[test]
    fn pinned_sources_lead_the_dialog_order_and_the_rest_keeps_its_order() {
        let mut sources = vec![
            source("tool:goose"),
            source("tool:codex"),
            source("extra:cc_switch"),
            source("tool:claude_code"),
            source("central"),
            source("tool:shared_agents"),
            source("plugin:alpha"),
        ];
        sort_sources_for_display(&mut sources);

        let ids: Vec<&str> = sources.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(&ids[..5], &[
            "central",
            "tool:shared_agents",
            "extra:cc_switch",
            "tool:claude_code",
            "tool:codex",
        ]);
        // Everything else stays behind the pinned block, in its original order.
        assert_eq!(&ids[5..], &["tool:goose", "plugin:alpha"]);
    }

    #[test]
    fn scan_finds_nested_and_root_skills_but_skips_hidden_dirs() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        write_skill(root, "alpha", Some("alpha"));
        write_skill(root, "group/beta", Some("beta"));
        write_skill(root, ".hidden/gamma", Some("gamma"));
        std::fs::create_dir_all(root.join("plain")).expect("plain dir");

        let found = scan_source_dir(root);
        let names: Vec<&str> = found.iter().map(|skill| skill.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert_eq!(found[1].dir_name, "group/beta");
    }

    #[test]
    fn scan_falls_back_to_directory_name_without_frontmatter() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        write_skill(root, "no-name", None);

        let found = scan_source_dir(root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "no-name");
    }

    #[test]
    fn duplicate_names_are_conflicts_and_are_not_returned() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        write_skill(root, "a/same", Some("same"));
        write_skill(root, "b/same", Some("same"));
        write_skill(root, "unique", Some("unique"));

        let (unique, conflicts) = partition_by_unique_name(scan_source_dir(root));
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].name, "unique");
        assert_eq!(conflicts, vec!["same".to_string()]);
    }

    #[test]
    fn missing_directory_scan_is_empty() {
        let temp = tempfile::tempdir().expect("temp dir");
        assert!(scan_source_dir(&temp.path().join("missing")).is_empty());
    }

    #[test]
    fn fingerprint_changes_when_content_is_rewritten() {
        let temp = tempfile::tempdir().expect("temp dir");
        let dir = temp.path().join("skill");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("SKILL.md"), "one").expect("write");
        let first = fingerprint_skill_dir(&dir);

        std::fs::write(dir.join("SKILL.md"), "two-longer").expect("rewrite");
        std::fs::write(dir.join("extra.md"), "x").expect("add file");
        let second = fingerprint_skill_dir(&dir);

        assert_ne!(first, second);
        assert_eq!(second.entry_count, 2);
    }
}
