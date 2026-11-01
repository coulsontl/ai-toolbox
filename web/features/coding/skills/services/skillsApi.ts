import { invoke } from '@tauri-apps/api/core';
import type {
  ManagedSkill,
  ToolStatus,
  InstallResult,
  SyncResult,
  UpdateResult,
  UpdateAllResult,
  AutoUpdateConfig,
  AutoSyncConfig,
  AutoSyncConfigInput,
  AutoSyncRunResult,
  AutoSyncSourceList,
  GitSkillCandidate,
  OnboardingPlan,
  SkillRepo,
  CustomTool,
  SkillGroupRecord,
  SkillInventoryPreview,
  SkillViewMode,
  CentralRepoPathStatus,
  CentralRepoPathPreview,
  ApplyCentralRepoPathOptions,
  ApplyCentralRepoPathResult,
  CentralRepoScan,
  AdoptCentralSkillsResult,
  DeleteManagedSkillOptions,
  SkillDocument,
} from '../types';
import type { SkillGroupingMode, SkillGroupModePreference } from '../utils/skillGrouping';

// Tool Status
export const getToolStatus = async (): Promise<ToolStatus> => {
  return invoke<ToolStatus>('skills_get_tool_status');
};

// Central Repo Path
export const getCentralRepoPath = async (): Promise<string> => {
  return invoke<string>('skills_get_central_repo_path');
};

export const setCentralRepoPath = async (path: string): Promise<string> => {
  return invoke<string>('skills_set_central_repo_path', { path });
};

export const getDefaultCentralRepoPath = async (): Promise<string> => {
  return invoke<string>('skills_get_default_central_repo_path');
};

export const getCentralRepoPathStatus = async (): Promise<CentralRepoPathStatus> => {
  return invoke<CentralRepoPathStatus>('skills_get_central_repo_path_status');
};

export const previewCentralRepoPath = async (path: string): Promise<CentralRepoPathPreview> => {
  return invoke<CentralRepoPathPreview>('skills_preview_central_repo_path', { path });
};

export const applyCentralRepoPathChange = async (
  path: string,
  options: ApplyCentralRepoPathOptions,
): Promise<ApplyCentralRepoPathResult> => {
  return invoke<ApplyCentralRepoPathResult>('skills_apply_central_repo_path_change', {
    path,
    options,
  });
};

export const scanCentralRepo = async (): Promise<CentralRepoScan> => {
  return invoke<CentralRepoScan>('skills_scan_central_repo');
};

export const adoptCentralRepoSkills = async (
  relativePaths: string[],
): Promise<AdoptCentralSkillsResult> => {
  return invoke<AdoptCentralSkillsResult>('skills_adopt_central_repo_skills', { relativePaths });
};

export const repairCentralRepoSkill = async (
  skillId: string,
  relativePath: string,
): Promise<AdoptCentralSkillsResult> => {
  return invoke<AdoptCentralSkillsResult>('skills_repair_central_repo_skill', {
    skillId,
    relativePath,
  });
};

// Managed Skills
export const getManagedSkills = async (): Promise<ManagedSkill[]> => {
  return invoke<ManagedSkill[]>('skills_get_managed_skills');
};

// Install Skills
export const installLocalSkill = async (
  sourcePath: string,
  overwrite?: boolean
): Promise<InstallResult> => {
  return invoke<InstallResult>('skills_install_local', { sourcePath, overwrite });
};

export const listLocalSkills = async (sourcePath: string): Promise<GitSkillCandidate[]> => {
  return invoke<GitSkillCandidate[]>('skills_list_local_skills', { sourcePath });
};

export const installLocalSelection = async (
  sourcePath: string,
  subpath: string,
  overwrite?: boolean
): Promise<InstallResult> => {
  return invoke<InstallResult>('skills_install_local_selection', { sourcePath, subpath, overwrite });
};

export const installGitSkill = async (
  repoUrl: string,
  branch?: string,
  overwrite?: boolean
): Promise<InstallResult> => {
  return invoke<InstallResult>('skills_install_git', { repoUrl, branch, overwrite });
};

export const listGitSkills = async (repoUrl: string, branch?: string): Promise<GitSkillCandidate[]> => {
  return invoke<GitSkillCandidate[]>('skills_list_git_skills', { repoUrl, branch });
};

export const installGitSelection = async (
  repoUrl: string,
  subpath: string,
  branch?: string,
  overwrite?: boolean
): Promise<InstallResult> => {
  return invoke<InstallResult>('skills_install_git_selection', { repoUrl, subpath, branch, overwrite });
};

// Sync Skills
export const syncSkillToTool = async (
  sourcePath: string,
  skillId: string,
  tool: string,
  name: string,
  overwrite?: boolean
): Promise<SyncResult> => {
  return invoke<SyncResult>('skills_sync_to_tool', {
    sourcePath,
    skillId,
    tool,
    name,
    overwrite,
  });
};

export const unsyncSkillFromTool = async (
  skillId: string,
  tool: string
): Promise<void> => {
  return invoke('skills_unsync_from_tool', { skillId, tool });
};

// Update/Delete Skills
export const updateManagedSkill = async (skillId: string): Promise<UpdateResult> => {
  return invoke<UpdateResult>('skills_update_managed', { skillId });
};

export const updateAllSkills = async (): Promise<UpdateAllResult> => {
  return invoke<UpdateAllResult>('skills_update_all');
};

export const getAutoUpdate = async (): Promise<AutoUpdateConfig> => {
  return invoke<AutoUpdateConfig>('skills_get_auto_update');
};

export const setAutoUpdate = async (config: AutoUpdateConfig): Promise<AutoUpdateConfig> => {
  return invoke<AutoUpdateConfig>('skills_set_auto_update', { config });
};

export const previewAutoUpdateSchedule = async (
  schedule: string,
  count?: number,
): Promise<string[]> => {
  return invoke<string[]>('skills_preview_auto_update_schedule', { schedule, count });
};

// Auto Discover (issue #446)
export const getAutoSyncSources = async (): Promise<AutoSyncSourceList> => {
  return invoke<AutoSyncSourceList>('skills_get_auto_sync_sources');
};

export const getAutoSync = async (): Promise<AutoSyncConfig> => {
  return invoke<AutoSyncConfig>('skills_get_auto_sync');
};

export const setAutoSync = async (config: AutoSyncConfigInput): Promise<AutoSyncConfig> => {
  return invoke<AutoSyncConfig>('skills_set_auto_sync', { config });
};

export const addAutoSyncDir = async (path: string): Promise<void> => {
  return invoke('skills_add_auto_sync_dir', { path });
};

export const removeAutoSyncDir = async (path: string): Promise<void> => {
  return invoke('skills_remove_auto_sync_dir', { path });
};

export const runAutoSync = async (): Promise<AutoSyncRunResult> => {
  return invoke<AutoSyncRunResult>('skills_run_auto_sync');
};

export const deleteManagedSkill = async (
  skillId: string,
  options?: DeleteManagedSkillOptions,
): Promise<void> => {
  return invoke('skills_delete_managed', { skillId, options });
};

export const updateSkillMetadata = async (
  skillId: string,
  groupId: string | null,
  userNote: string | null,
  tags?: string[],
): Promise<void> => {
  // Backend tri-state: undefined keeps existing tags, array overwrites the list.
  const payload: Record<string, unknown> = { skillId, groupId, userNote };
  if (typeof tags !== 'undefined') {
    payload.tags = tags;
  }
  return invoke('skills_update_metadata', payload);
};

export const batchUpdateSkillGroup = async (
  skillIds: string[],
  groupId: string | null,
): Promise<void> => {
  return invoke('skills_batch_update_group', { skillIds, groupId });
};

export const getSkillGroups = async (): Promise<SkillGroupRecord[]> => {
  return invoke<SkillGroupRecord[]>('skills_get_groups');
};

export const saveSkillGroup = async (
  name: string,
  note: string | null,
  sortIndex: number,
  id?: string,
): Promise<string> => {
  return invoke<string>('skills_save_group', { id, name, note, sortIndex });
};

export const deleteSkillGroup = async (groupId: string): Promise<void> => {
  return invoke('skills_delete_group', { groupId });
};

export const setSkillManagementEnabled = async (
  skillId: string,
  enabled: boolean,
): Promise<string[]> => {
  return invoke<string[]>('skills_set_management_enabled', { skillId, enabled });
};

export const exportSkillInventory = async (): Promise<string> => {
  return invoke<string>('skills_export_inventory');
};

export const exportSkillInventoryFile = async (): Promise<string> => {
  return invoke<string>('skills_export_inventory_file');
};

export const previewSkillInventoryImport = async (
  inventoryJson: string,
): Promise<SkillInventoryPreview> => {
  return invoke<SkillInventoryPreview>('skills_preview_inventory_import', { inventoryJson });
};

export const previewSkillInventoryImportFile = async (
  filePath: string,
): Promise<SkillInventoryPreview> => {
  return invoke<SkillInventoryPreview>('skills_preview_inventory_import_file', { filePath });
};

export const applySkillInventoryImport = async (
  inventoryJson: string,
): Promise<SkillInventoryPreview> => {
  return invoke<SkillInventoryPreview>('skills_apply_inventory_import', { inventoryJson });
};

export const applySkillInventoryImportFile = async (
  filePath: string,
): Promise<SkillInventoryPreview> => {
  return invoke<SkillInventoryPreview>('skills_apply_inventory_import_file', { filePath });
};

// Onboarding
export const getOnboardingPlan = async (): Promise<OnboardingPlan> => {
  return invoke<OnboardingPlan>('skills_get_onboarding_plan');
};

export const importExistingSkill = async (
  sourcePath: string,
  overwrite?: boolean
): Promise<InstallResult> => {
  return invoke<InstallResult>('skills_import_existing', { sourcePath, overwrite });
};

// Git Cache
export const getGitCacheCleanupDays = async (): Promise<number> => {
  return invoke<number>('skills_get_git_cache_cleanup_days');
};

export const setGitCacheCleanupDays = async (days: number): Promise<number> => {
  return invoke<number>('skills_set_git_cache_cleanup_days', { days });
};

export const getGitCacheTtlSecs = async (): Promise<number> => {
  return invoke<number>('skills_get_git_cache_ttl_secs');
};

export const clearGitCache = async (): Promise<number> => {
  return invoke<number>('skills_clear_git_cache');
};

export const getGitCachePath = async (): Promise<string> => {
  return invoke<string>('skills_get_git_cache_path');
};

// Preferred Tools
export const getPreferredTools = async (): Promise<string[] | null> => {
  return invoke<string[] | null>('skills_get_preferred_tools');
};

export const setPreferredTools = async (tools: string[]): Promise<void> => {
  return invoke('skills_set_preferred_tools', { tools });
};

export const getLimitAddMoreToPreferredTools = async (): Promise<boolean> => {
  return invoke<boolean>('skills_get_limit_add_more_to_preferred_tools');
};

export const setLimitAddMoreToPreferredTools = async (enabled: boolean): Promise<void> => {
  return invoke('skills_set_limit_add_more_to_preferred_tools', { enabled });
};

// Show Skills in Tray
export const getShowSkillsInTray = async (): Promise<boolean> => {
  return invoke<boolean>('skills_get_show_in_tray');
};

export const setShowSkillsInTray = async (enabled: boolean): Promise<void> => {
  return invoke('skills_set_show_in_tray', { enabled });
};

// Default View Mode
export const getDefaultViewMode = async (): Promise<SkillViewMode> => {
  return invoke<SkillViewMode>('skills_get_default_view_mode');
};

export const setDefaultViewMode = async (mode: SkillViewMode): Promise<void> => {
  return invoke('skills_set_default_view_mode', { mode });
};

// Group Mode
export const getSkillGroupMode = async (): Promise<SkillGroupModePreference> => {
  return invoke<SkillGroupModePreference>('skills_get_group_mode');
};

export const setSkillGroupMode = async (mode: SkillGroupingMode): Promise<void> => {
  return invoke('skills_set_group_mode', { mode });
};

// Skill Repos
export const getSkillRepos = async (): Promise<SkillRepo[]> => {
  return invoke<SkillRepo[]>('skills_get_repos');
};

export const addSkillRepo = async (owner: string, name: string, branch?: string): Promise<void> => {
  return invoke('skills_add_repo', { owner, name, branch });
};

export const removeSkillRepo = async (owner: string, name: string): Promise<void> => {
  return invoke('skills_remove_repo', { owner, name });
};

export const initDefaultRepos = async (): Promise<number> => {
  return invoke<number>('skills_init_default_repos');
};

// Custom Tools
export const getCustomTools = async (): Promise<CustomTool[]> => {
  return invoke<CustomTool[]>('skills_get_custom_tools');
};

export const addCustomTool = async (
  key: string,
  displayName: string,
  relativeSkillsDir: string,
  relativeDetectDir: string,
  forceCopy?: boolean,
  iconUrl?: string,
): Promise<void> => {
  return invoke('skills_add_custom_tool', {
    key,
    displayName,
    relativeSkillsDir,
    relativeDetectDir,
    forceCopy,
    iconUrl,
  });
};

export const removeCustomTool = async (key: string): Promise<void> => {
  return invoke('skills_remove_custom_tool', { key });
};

export const checkCustomToolPath = async (relativeSkillsDir: string): Promise<boolean> => {
  return invoke<boolean>('skills_check_custom_tool_path', { relativeSkillsDir });
};

export const createCustomToolPath = async (relativeSkillsDir: string): Promise<void> => {
  return invoke('skills_create_custom_tool_path', { relativeSkillsDir });
};

// Reorder Skills
export const reorderSkills = async (ids: string[]): Promise<void> => {
  return invoke('skills_reorder', { ids });
};

// Skill documents (detail panel preview)
export const getSkillDocuments = async (skillId: string): Promise<SkillDocument[]> => {
  return invoke<SkillDocument[]>('skills_get_skill_documents', { skillId });
};
