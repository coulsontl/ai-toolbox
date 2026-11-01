use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// Re-export CustomTool from tool_adapters for backward compatibility
pub use super::tool_adapters::CustomTool;

/// Skill record stored in SQLite JSONB
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub source_type: String, // "local" | "git" | "import" | "central"
    pub source_ref: Option<String>,
    pub source_revision: Option<String>,
    pub central_path: String,
    pub content_hash: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_sync_at: Option<i64>,
    pub status: String,

    // Sort order for drag-and-drop reordering
    pub sort_index: i32,

    // User-managed local metadata for organization inside AI Toolbox.
    pub user_group: Option<String>,
    pub group_id: Option<String>,
    pub user_note: Option<String>,
    pub management_enabled: bool,
    pub disabled_previous_tools: Vec<String>,
    pub tags: Vec<String>,

    // Enabled tool keys list
    pub enabled_tools: Vec<String>, // ["claude_code", "codex", "opencode"]

    // Sync details JSON (per-tool target_path/mode/status etc.)
    // Structure: { "claude_code": { "target_path": "...", "mode": "...", ... }, ... }
    pub sync_details: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillGroupRecord {
    pub id: String,
    pub name: String,
    pub note: Option<String>,
    pub sort_index: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Skill target info - used within sync_details (no longer a separate table)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillTarget {
    pub tool: String,
    pub target_path: String,
    pub mode: String, // "symlink" | "copy" | "junction"
    pub status: String,
    pub synced_at: Option<i64>,
    pub error_message: Option<String>,
}

/// Skill repository source - user configured skill source repos
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillRepo {
    pub id: String, // Format: "owner/name"
    pub owner: String,
    pub name: String,
    pub branch: String, // default: "main"
    pub enabled: bool,  // default: true
    pub created_at: i64,
}

/// Skill preferences - user preference settings (structured wide table)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillPreferences {
    pub id: String,                           // Fixed "default"
    pub preferred_tools: Option<Vec<String>>, // User selected preferred tools
    pub default_view_mode: String,
    /// Grouped-view organization: `auto` (source when no custom groups exist),
    /// `custom`, or `source`.
    pub group_mode: String,
    pub git_cache_cleanup_days: i32,
    pub git_cache_ttl_secs: i32,
    pub known_tool_versions: Option<Value>,
    pub installed_tools: Option<Vec<String>>, // Detected installed tools
    pub show_skills_in_tray: bool,            // Show skills in system tray quick menu
    pub auto_update_enabled: bool,            // Enable scheduled auto-update of all skills
    pub auto_update_schedule: String,         // 5-field cron expression (min hour dom mon dow)
    pub limit_add_more_to_preferred_tools: bool, // Add-more menus on Skill cards only show preferred tools
    /// Enable the background discover pass (issue #446).
    pub auto_sync_enabled: bool,
    /// Enabled scan source identities (see `auto_discover`), e.g. `central`,
    /// `tool:codex`, `extra:cc_switch`, `plugin:<id>`, `custom:<path>`.
    /// `None` means "never configured" and resolves to central only.
    pub auto_sync_sources: Option<Vec<String>>,
    /// User-added directories shown in the source list. Kept separately from
    /// the enabled set so unchecking a directory does not forget its path.
    pub auto_sync_custom_dirs: Option<Vec<String>>,
    /// Target tools for newly discovered Skills. `None` means the default
    /// target only (the Universal directory); an empty list distributes nowhere.
    pub auto_sync_target_tools: Option<Vec<String>>,
    pub updated_at: i64,
}

impl Default for SkillPreferences {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            preferred_tools: None,
            default_view_mode: "flat".to_string(),
            group_mode: "auto".to_string(),
            git_cache_cleanup_days: 30,
            git_cache_ttl_secs: 60,
            known_tool_versions: None,
            installed_tools: None,
            show_skills_in_tray: false,
            auto_update_enabled: false,
            auto_update_schedule: "0 3 * * *".to_string(),
            limit_add_more_to_preferred_tools: false,
            auto_sync_enabled: false,
            auto_sync_sources: None,
            auto_sync_custom_dirs: None,
            auto_sync_target_tools: None,
            updated_at: 0,
        }
    }
}

/// Tool detection status
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDetection {
    pub tool: String,
    pub installed: bool,
    pub skills_dir: Option<String>,
    pub detected_at: i64,
    pub first_seen_at: Option<i64>,
}

/// DTO for tool status response
#[derive(Debug, Serialize)]
pub struct ToolStatusDto {
    pub tools: Vec<ToolInfoDto>,
    pub installed: Vec<String>,
    pub newly_installed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolInfoDto {
    pub key: String,
    pub label: String,
    pub installed: bool,
    pub skills_dir: String,
    /// Optional brand icon for custom tools (http(s) image URL)
    pub icon_url: Option<String>,
}

/// DTO for managed skills (frontend display)
#[derive(Debug, Serialize)]
pub struct ManagedSkillDto {
    pub id: String,
    pub name: String,
    pub source_type: String,
    pub source_ref: Option<String>,
    pub central_path: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_sync_at: Option<i64>,
    pub status: String,
    pub sort_index: i32,
    pub user_group: Option<String>,
    pub group_id: Option<String>,
    pub user_note: Option<String>,
    pub management_enabled: bool,
    pub disabled_previous_tools: Vec<String>,
    pub description: Option<String>,
    pub content_hash: Option<String>,
    pub source_health: String,
    pub source_error: Option<String>,
    pub tags: Vec<String>,
    pub enabled_tools: Vec<String>,
    pub targets: Vec<SkillTargetDto>, // Derived from sync_details
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkillGroupDto {
    pub id: String,
    pub name: String,
    pub note: Option<String>,
    pub sort_index: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkillInventoryGroupJson {
    pub name: String,
    pub note: Option<String>,
    pub order: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkillInventorySkillJson {
    pub id: Option<String>,
    pub name: String,
    pub group: Option<String>,
    pub user_note: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub order: i32,
    pub enabled: bool,
    pub enabled_tools: Vec<String>,
    pub previous_enabled_tools: Vec<String>,
    pub source_type: String,
    pub source_ref: Option<String>,
    pub central_path: String,
    pub content_hash: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkillInventoryJson {
    pub schema_version: u32,
    pub exported_at: i64,
    pub groups: Vec<SkillInventoryGroupJson>,
    pub skills: Vec<SkillInventorySkillJson>,
}

#[derive(Debug, Serialize)]
pub struct SkillInventoryPreviewDto {
    pub valid: bool,
    pub errors: Vec<String>,
    pub group_count: usize,
    pub matched_skill_count: usize,
    pub unmatched_inventory_skills: Vec<String>,
    pub local_missing_from_inventory: Vec<ManagedSkillSummaryDto>,
    pub default_disable_count: usize,
    pub content_changed_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ManagedSkillSummaryDto {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CentralRepoPathStatusDto {
    pub current_path: String,
    pub default_path: String,
    pub uses_default: bool,
    pub exists: bool,
    pub is_directory: bool,
    pub can_read: bool,
    pub can_write: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DetectedCentralSkillDto {
    pub name: String,
    pub description: Option<String>,
    pub relative_path: String,
    pub absolute_path: String,
    pub content_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CentralSkillMatchDto {
    pub skill_id: String,
    pub name: String,
    pub relative_path: String,
    pub absolute_path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CentralSkillRepairCandidateDto {
    pub skill_id: String,
    pub name: String,
    pub current_relative_path: String,
    pub detected_relative_path: String,
    pub detected_absolute_path: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CentralRepoMigrationCandidateDto {
    pub skill_id: String,
    pub name: String,
    pub relative_path: String,
    pub source_path: String,
    pub target_path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CentralRepoConflictDto {
    pub name: String,
    pub paths: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CentralRepoTargetImpactDto {
    pub skill_id: String,
    pub skill_name: String,
    pub tool: String,
    pub mode: String,
    pub target_path: String,
}

#[derive(Debug, Serialize)]
pub struct CentralRepoPathPreviewDto {
    pub requested_path: String,
    pub resolved_path: String,
    pub current_path: String,
    pub default_path: String,
    pub current_uses_default: bool,
    pub requested_is_default: bool,
    pub exists: bool,
    pub is_directory: bool,
    pub can_create: bool,
    pub can_read: bool,
    pub can_write: bool,
    pub detected_skills: Vec<DetectedCentralSkillDto>,
    pub matched_existing: Vec<CentralSkillMatchDto>,
    pub unmanaged_detected: Vec<DetectedCentralSkillDto>,
    pub missing_existing: Vec<ManagedSkillSummaryDto>,
    pub repair_candidates: Vec<CentralSkillRepairCandidateDto>,
    pub migration_candidates: Vec<CentralRepoMigrationCandidateDto>,
    pub migration_conflicts: Vec<CentralRepoConflictDto>,
    pub affected_targets: Vec<CentralRepoTargetImpactDto>,
    pub conflicts: Vec<CentralRepoConflictDto>,
    pub root_skill_warning: Option<String>,
    pub path_warnings: Vec<String>,
    pub blocking_errors: Vec<String>,
    pub can_apply: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyCentralRepoPathOptionsDto {
    #[serde(default)]
    pub adopt_detected_skill_paths: Vec<String>,
    #[serde(default)]
    pub repair_existing_skill_paths: HashMap<String, String>,
    #[serde(default)]
    pub migrate_existing_skill_ids: Vec<String>,
    #[serde(default)]
    pub use_default_path: bool,
    #[serde(default = "default_true")]
    pub resync_enabled_tools: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct ApplyCentralRepoPathResultDto {
    pub path: String,
    pub uses_default: bool,
    pub adopted_count: usize,
    pub repaired_count: usize,
    pub migrated_count: usize,
    pub resynced_targets: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CentralRepoScanDto {
    pub central_path: String,
    pub detected_skills: Vec<DetectedCentralSkillDto>,
    pub unmanaged_detected: Vec<DetectedCentralSkillDto>,
    pub repair_candidates: Vec<CentralSkillRepairCandidateDto>,
    pub conflicts: Vec<CentralRepoConflictDto>,
    pub root_skill_warning: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdoptCentralSkillsResultDto {
    pub adopted_count: usize,
    pub repaired_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteManagedSkillOptionsDto {
    #[serde(default)]
    pub delete_source_files: bool,
}

#[derive(Debug, Serialize)]
pub struct SkillTargetDto {
    pub tool: String,
    pub mode: String,
    pub status: String,
    pub target_path: String,
    pub synced_at: Option<i64>,
}

/// DTO for install result
#[derive(Debug, Serialize)]
pub struct InstallResultDto {
    pub skill_id: String,
    pub name: String,
    pub central_path: String,
    pub content_hash: Option<String>,
}

/// DTO for sync result
#[derive(Debug, Serialize)]
pub struct SyncResultDto {
    pub mode_used: String,
    pub target_path: String,
}

/// DTO for update result
#[derive(Debug, Serialize)]
pub struct UpdateResultDto {
    pub skill_id: String,
    pub name: String,
    pub content_hash: Option<String>,
    pub source_revision: Option<String>,
    pub updated_targets: Vec<String>,
}

/// DTO for a single failed skill during a bulk update
#[derive(Debug, Serialize)]
pub struct UpdateAllErrorDto {
    pub skill_id: String,
    pub name: String,
    pub error: String,
}

/// DTO for update-all result
#[derive(Debug, Serialize)]
pub struct UpdateAllResultDto {
    pub total: usize,
    pub updated: Vec<UpdateResultDto>,
    pub errors: Vec<UpdateAllErrorDto>,
}

/// Progress event for the manual "Update All" pass.
///
/// Emitted via `skills-update-progress` while `update_all_skills_internal`
/// iterates. Not emitted from the scheduled auto-update pass (it passes
/// `emit_progress = false`) to stay silent per the auto-update contract.
#[derive(Debug, Clone, Serialize)]
pub struct SkillsUpdateProgress {
    /// 1-based index of the skill currently being updated (0 before the first).
    pub current: usize,
    /// Total managed skills in this pass.
    pub total: usize,
    /// Id of the skill currently being updated.
    pub skill_id: String,
    /// Display name of the skill currently being updated.
    pub name: String,
    /// Human-readable status line, e.g. "正在更新: xxx (3/10)".
    pub message: String,
}

/// DTO for scheduled auto-update configuration
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoUpdateConfigDto {
    pub enabled: bool,
    pub schedule: String,
}

/// One scan source offered by the auto-discover settings (issue #446).
///
/// `id` is the stable identity persisted in preferences; `path` is resolved at
/// read time so WSL/custom runtime roots and cross-machine restores keep working.
#[derive(Clone, Debug, Serialize)]
pub struct AutoSyncSourceDto {
    /// Stable identity: `central`, `tool:<key>`, `extra:<key>`, `plugin:<id>`, `custom:<path>`.
    pub id: String,
    pub label: String,
    /// `central` | `tool` | `extra` | `plugin` | `custom`
    pub kind: String,
    pub path: String,
    /// False when the directory does not currently exist on disk.
    pub exists: bool,
    /// Number of Skills currently found in this directory.
    pub detected_count: usize,
    /// Number of detected Skills that are not managed yet.
    pub new_count: usize,
    /// Central is always scanned; the UI shows it as locked-on.
    pub locked: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutoSyncSourceListDto {
    pub sources: Vec<AutoSyncSourceDto>,
}

/// Auto-discover configuration returned by `skills_get_auto_sync`.
#[derive(Clone, Debug, Serialize)]
pub struct AutoSyncConfigDto {
    pub enabled: bool,
    pub sources: Vec<String>,
    /// `None` means "not configured": the pass then targets the Universal
    /// directory only (see `auto_discover::DEFAULT_TARGET_TOOL`).
    pub target_tools: Option<Vec<String>>,
}

/// Input for `skills_set_auto_sync`.
///
/// Field names stay snake_case: `skills_get_auto_sync` returns a snake_case
/// `AutoSyncConfigDto`, and the dialog round-trips the same object through both,
/// so a camelCase rename here would silently drop `target_tools`.
#[derive(Clone, Debug, Deserialize)]
pub struct AutoSyncConfigInputDto {
    pub enabled: bool,
    pub sources: Vec<String>,
    pub target_tools: Option<Vec<String>>,
}

/// Result of one discover pass (manual "scan now" or the background tick).
#[derive(Clone, Debug, Default, Serialize)]
pub struct AutoSyncRunResultDto {
    pub discovered: usize,
    pub adopted: usize,
    pub refreshed: usize,
    pub synced_targets: usize,
    pub skipped_conflicts: Vec<String>,
    pub skipped_names: Vec<String>,
    pub errors: Vec<String>,
}

/// Git skill candidate for multi-skill repos
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitSkillCandidate {
    pub name: String,
    pub description: Option<String>,
    pub subpath: String,
}

/// Onboarding plan for discovered skills
#[derive(Clone, Debug, Serialize)]
pub struct OnboardingPlan {
    pub total_tools_scanned: usize,
    pub total_skills_found: usize,
    pub groups: Vec<OnboardingGroup>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OnboardingGroup {
    pub name: String,
    pub variants: Vec<OnboardingVariant>,
    pub has_conflict: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OnboardingVariant {
    pub tool: String,
    /// Human-readable tool label for display (e.g. "Plugin: demo-plugin" instead of "plugin::demo-plugin@xxx")
    pub tool_display: String,
    pub name: String,
    pub path: String,
    pub fingerprint: Option<String>,
    pub is_link: bool,
    pub link_target: Option<String>,
    /// Tools that have the same skill name but different content (conflicting versions)
    pub conflicting_tools: Vec<String>,
    /// AI Toolbox tool keys marked enabled at the source. Only set for the
    /// CC Switch source (`tool = "cc_switch"`), matched by directory name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_enabled_tools: Vec<String>,
}

/// Internal struct for install operations
pub struct InstallResult {
    pub skill_id: String,
    pub name: String,
    pub central_path: std::path::PathBuf,
    pub content_hash: Option<String>,
}

/// Internal struct for update operations
pub struct UpdateResult {
    pub skill_id: String,
    pub name: String,
    pub central_path: std::path::PathBuf,
    pub content_hash: Option<String>,
    pub source_revision: Option<String>,
    pub updated_targets: Vec<String>,
}

/// Sync mode used for skill syncing
#[derive(Clone, Debug)]
pub enum SyncMode {
    Auto,
    Symlink,
    Junction,
    Copy,
}

impl SyncMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncMode::Auto => "auto",
            SyncMode::Symlink => "symlink",
            SyncMode::Junction => "junction",
            SyncMode::Copy => "copy",
        }
    }
}

/// Sync outcome from sync operations
#[derive(Clone, Debug)]
pub struct SyncOutcome {
    pub mode_used: SyncMode,
    pub target_path: std::path::PathBuf,
    pub replaced: bool,
}

/// Detected skill in a tool directory
#[derive(Clone, Debug)]
pub struct DetectedSkill {
    pub tool: String,
    /// Human-readable tool label for display
    pub tool_display: String,
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_link: bool,
    pub link_target: Option<std::path::PathBuf>,
    /// AI Toolbox tool keys marked enabled at the source (CC Switch only).
    pub source_enabled_tools: Vec<String>,
}

/// DTO for custom tool
#[derive(Debug, Serialize)]
pub struct CustomToolDto {
    pub key: String,
    pub display_name: String,
    pub relative_skills_dir: String,
    pub relative_detect_dir: String,
    pub created_at: i64,
    pub force_copy: bool,
    /// Optional brand icon for UI display (http(s) image URL)
    pub icon_url: Option<String>,
}

/// DTO for skill repo
#[derive(Debug, Serialize)]
pub struct SkillRepoDto {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub branch: String,
    pub enabled: bool,
    pub created_at: i64,
}

/// A document file (e.g. SKILL.md / README.md) inside a Skill's central dir,
/// returned for the Skill detail preview panel. Content is truncated to keep
/// large files from stalling the UI.
#[derive(Debug, Serialize)]
pub struct SkillDocumentDto {
    pub filename: String,
    pub content: String,
    /// True when the file was truncated because it exceeded the read cap.
    pub truncated: bool,
}

/// Helper function to get current timestamp in milliseconds
pub fn now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
