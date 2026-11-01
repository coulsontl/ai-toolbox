// Skills feature types

export interface ManagedSkill {
  id: string;
  name: string;
  source_type: 'local' | 'git' | 'import' | 'central';
  source_ref: string | null;
  central_path: string;
  created_at: number;
  updated_at: number;
  last_sync_at: number | null;
  status: string;
  sort_index: number;
  user_group: string | null;
  group_id: string | null;
  user_note: string | null;
  management_enabled: boolean;
  disabled_previous_tools: string[];
  tags: string[];
  description: string | null;
  content_hash: string | null;
  source_health: SkillSourceHealth;
  source_error: string | null;

  // New fields
  enabled_tools: string[]; // ["claude_code", "codex", ...]

  // Derived from sync_details (maintained for compatibility)
  targets: SkillTarget[];
}

export type SkillSourceHealth = 'ok' | 'warning';

export interface SkillTarget {
  tool: string;
  mode: string;
  status: string;
  target_path: string;
  synced_at: number | null;
}

export interface SkillRepo {
  id: string;
  owner: string;
  name: string;
  branch: string;
  enabled: boolean;
  created_at: number;
}

export type SkillViewMode = 'flat' | 'grouped';

export type SkillGroupingMode = 'custom' | 'source';

/**
 * Persisted grouped-view preference. `auto` keeps the historical source
 * grouping until the user owns at least one custom group.
 */
export type SkillGroupModePreference = 'auto' | SkillGroupingMode;

export interface SkillPreferences {
  preferred_tools: string[] | null;
  default_view_mode: SkillViewMode;
  group_mode: SkillGroupModePreference;
  git_cache_cleanup_days: number;
  git_cache_ttl_secs: number;
  installed_tools: string[] | null;
}

export interface ToolInfo {
  key: string;
  label: string;
  installed: boolean;
  skills_dir: string;
  /** Optional brand icon for custom tools (http(s) image URL) */
  icon_url?: string | null;
}

export interface ToolStatus {
  tools: ToolInfo[];
  installed: string[];
  newly_installed: string[];
}

export interface InstallResult {
  skill_id: string;
  name: string;
  central_path: string;
  content_hash: string | null;
}

export interface SyncResult {
  mode_used: string;
  target_path: string;
}

export interface UpdateResult {
  skill_id: string;
  name: string;
  content_hash: string | null;
  source_revision: string | null;
  updated_targets: string[];
}

export interface UpdateAllError {
  skill_id: string;
  name: string;
  error: string;
}

export interface UpdateAllResult {
  total: number;
  updated: UpdateResult[];
  errors: UpdateAllError[];
}

/** Progress event payload emitted on the `skills-update-progress` channel. */
export interface SkillsUpdateProgress {
  current: number;
  total: number;
  skill_id: string;
  name: string;
  message: string;
}

export interface AutoUpdateConfig {
  enabled: boolean;
  schedule: string;
}

/** One candidate directory the auto-discover pass can scan (issue #446). */
export interface AutoSyncSource {
  id: string;
  label: string;
  kind: 'central' | 'tool' | 'extra' | 'plugin' | 'custom';
  path: string;
  exists: boolean;
  detected_count: number;
  new_count: number;
  locked: boolean;
}

export interface AutoSyncSourceList {
  sources: AutoSyncSource[];
}

export interface AutoSyncConfig {
  enabled: boolean;
  sources: string[];
  target_tools: string[] | null;
}

export interface AutoSyncConfigInput {
  enabled: boolean;
  sources: string[];
  target_tools: string[] | null;
}

export interface AutoSyncRunResult {
  discovered: number;
  adopted: number;
  refreshed: number;
  synced_targets: number;
  skipped_conflicts: string[];
  skipped_names: string[];
  errors: string[];
}

export interface GitSkillCandidate {
  name: string;
  description: string | null;
  subpath: string;
}

export interface OnboardingVariant {
  tool: string;
  tool_display: string;
  name: string;
  path: string;
  fingerprint: string | null;
  is_link: boolean;
  link_target: string | null;
  conflicting_tools: string[];
  /** Tool keys marked enabled at the source; only set for the CC Switch source. */
  source_enabled_tools?: string[];
}

export interface OnboardingGroup {
  name: string;
  variants: OnboardingVariant[];
  has_conflict: boolean;
}

export interface OnboardingPlan {
  total_tools_scanned: number;
  total_skills_found: number;
  groups: OnboardingGroup[];
}

export interface SkillGroup {
  key: string;
  id: string | null;
  label: string;
  note?: string | null;
  sort_index?: number;
  sourceType: 'git' | 'local' | 'import' | 'central' | 'custom';
  skills: ManagedSkill[];
}

export interface SkillGroupRecord {
  id: string;
  name: string;
  note: string | null;
  sort_index: number;
  created_at: number;
  updated_at: number;
}

export interface SkillInventoryPreview {
  valid: boolean;
  errors: string[];
  group_count: number;
  matched_skill_count: number;
  unmatched_inventory_skills: string[];
  local_missing_from_inventory: Array<{ id: string; name: string }>;
  default_disable_count: number;
  content_changed_count: number;
}

export interface CentralRepoPathStatus {
  current_path: string;
  default_path: string;
  uses_default: boolean;
  exists: boolean;
  is_directory: boolean;
  can_read: boolean;
  can_write: boolean;
  warning: string | null;
}

export interface DetectedCentralSkill {
  name: string;
  description: string | null;
  relative_path: string;
  absolute_path: string;
  content_hash: string | null;
}

export interface CentralSkillRepairCandidate {
  skill_id: string;
  name: string;
  current_relative_path: string;
  detected_relative_path: string;
  detected_absolute_path: string;
  description: string | null;
}

export interface CentralRepoMigrationCandidate {
  skill_id: string;
  name: string;
  relative_path: string;
  source_path: string;
  target_path: string;
}

export interface CentralRepoConflict {
  name: string;
  paths: string[];
  reason: string;
}

export interface CentralRepoTargetImpact {
  skill_id: string;
  skill_name: string;
  tool: string;
  mode: string;
  target_path: string;
}

export interface CentralRepoPathPreview {
  requested_path: string;
  resolved_path: string;
  current_path: string;
  default_path: string;
  current_uses_default: boolean;
  requested_is_default: boolean;
  exists: boolean;
  is_directory: boolean;
  can_create: boolean;
  can_read: boolean;
  can_write: boolean;
  detected_skills: DetectedCentralSkill[];
  matched_existing: Array<{
    skill_id: string;
    name: string;
    relative_path: string;
    absolute_path: string;
  }>;
  unmanaged_detected: DetectedCentralSkill[];
  missing_existing: Array<{ id: string; name: string }>;
  repair_candidates: CentralSkillRepairCandidate[];
  migration_candidates: CentralRepoMigrationCandidate[];
  migration_conflicts: CentralRepoConflict[];
  affected_targets: CentralRepoTargetImpact[];
  conflicts: CentralRepoConflict[];
  root_skill_warning: string | null;
  path_warnings: string[];
  blocking_errors: string[];
  can_apply: boolean;
}

export interface ApplyCentralRepoPathOptions {
  adoptDetectedSkillPaths: string[];
  repairExistingSkillPaths: Record<string, string>;
  migrateExistingSkillIds: string[];
  useDefaultPath: boolean;
  resyncEnabledTools: boolean;
}

export interface ApplyCentralRepoPathResult {
  path: string;
  uses_default: boolean;
  adopted_count: number;
  repaired_count: number;
  migrated_count: number;
  resynced_targets: string[];
  warnings: string[];
}

export interface CentralRepoScan {
  central_path: string;
  detected_skills: DetectedCentralSkill[];
  unmanaged_detected: DetectedCentralSkill[];
  repair_candidates: CentralSkillRepairCandidate[];
  conflicts: CentralRepoConflict[];
  root_skill_warning: string | null;
}

export interface AdoptCentralSkillsResult {
  adopted_count: number;
  repaired_count: number;
}

export interface DeleteManagedSkillOptions {
  deleteSourceFiles?: boolean;
}

export type SkillEnabledFilter = 'all' | 'enabled' | 'disabled';

export interface ToolOption {
  id: string;
  label: string;
  installed: boolean;
  /** Resolved skills directory of this tool (shown in the detail panel tooltip) */
  skillDir?: string | null;
  /** Optional brand icon for custom tools (http(s) image URL) */
  iconUrl?: string | null;
}

export interface CustomTool {
  key: string;
  display_name: string;
  relative_skills_dir: string;
  relative_detect_dir: string;
  created_at: number;
  force_copy: boolean;
  /** Optional brand icon (http(s) image URL) */
  icon_url?: string | null;
}

export interface SkillDocument {
  filename: string;
  content: string;
  truncated: boolean;
}
