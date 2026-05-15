// Skills feature types

export interface ManagedSkill {
  id: string;
  name: string;
  source_type: 'local' | 'git' | 'import';
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

export interface SkillPreferences {
  preferred_tools: string[] | null;
  default_view_mode: SkillViewMode;
  git_cache_cleanup_days: number;
  git_cache_ttl_secs: number;
  installed_tools: string[] | null;
}

export interface ToolInfo {
  key: string;
  label: string;
  installed: boolean;
  skills_dir: string;
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
  sourceType: 'git' | 'local' | 'import' | 'custom';
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

export type SkillEnabledFilter = 'all' | 'enabled' | 'disabled';

export interface ToolOption {
  id: string;
  label: string;
  installed: boolean;
}

export interface CustomTool {
  key: string;
  display_name: string;
  relative_skills_dir: string;
  relative_detect_dir: string;
  created_at: number;
  force_copy: boolean;
}
