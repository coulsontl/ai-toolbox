/**
 * Settings API Service
 *
 * Handles all settings-related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';

// Types matching Rust structures
export interface WebDAVConfig {
  url: string;
  username: string;
  password: string;
  remote_path: string;
  host_label: string;
}

export interface S3Config {
  access_key: string;
  secret_key: string;
  bucket: string;
  region: string;
  prefix: string;
  endpoint_url: string;
  force_path_style: boolean;
  public_domain: string;
}

export type BackupCustomEntryType = 'file' | 'directory';

export interface BackupCustomEntry {
  id: string;
  name: string;
  source_path: string;
  restore_path: string | null;
  entry_type: BackupCustomEntryType;
  enabled: boolean;
}

export interface BackupFileFilterRule {
  tool: string;
  file_path: string;
}

export interface BackupFileFilterPathOption {
  tool: string;
  file_path: string;
}

// Session detail filter chips (role + content visibility), persisted to SQLite
// under the `session_detail_filters` key of the settings singleton record.
export interface SessionDetailRoleFilter {
  user: boolean;
  assistant: boolean;
}

export interface SessionDetailContentFilter {
  text: boolean;
  thinking: boolean;
  tool_call: boolean;
  command: boolean;
}

export interface SessionDetailFilters {
  role_filter: SessionDetailRoleFilter;
  content_filter: SessionDetailContentFilter;
}

export const SIDEBAR_PAGE_KEYS = ['opencode', 'claudecode', 'claudedesktop', 'codex', 'grok', 'geminicli', 'kimi', 'openclaw', 'pi', 'oh_my_pi', 'hermes', 'dsh'] as const;

export type SidebarPageKey = typeof SIDEBAR_PAGE_KEYS[number];

export type SidebarHiddenByPage = Record<SidebarPageKey, boolean>;

export type ProxyMode = 'direct' | 'custom' | 'system';

type LegacySidebarVisibilityValue = boolean | {
  hidden?: boolean;
};

export const createDefaultSidebarHiddenByPage = (): SidebarHiddenByPage => ({
  opencode: false,
  claudecode: false,
  claudedesktop: false,
  codex: false,
  grok: false,
  geminicli: false,
  kimi: false,
  openclaw: false,
  pi: false,
  oh_my_pi: false,
  hermes: false,
  dsh: false,
});

export const normalizeSidebarHiddenByPage = (
  value?: Partial<Record<SidebarPageKey, LegacySidebarVisibilityValue>> | null
): SidebarHiddenByPage => {
  const normalizedValue = createDefaultSidebarHiddenByPage();

  for (const pageKey of SIDEBAR_PAGE_KEYS) {
    const pageValue = value?.[pageKey];
    if (!pageValue) continue;

    if (typeof pageValue === 'boolean') {
      normalizedValue[pageKey] = pageValue;
      continue;
    }

    normalizedValue[pageKey] = pageValue.hidden ?? false;
  }

  return normalizedValue;
};

export interface AppSettings {
  language: string;
  current_module: string;
  current_sub_tab: string;
  backup_type: string;
  local_backup_path: string;
  webdav: WebDAVConfig;
  s3: S3Config;
  last_backup_time: string | null;
  backup_image_assets_enabled: boolean;
  backup_cli_config_files_enabled: boolean;
  backup_custom_entries: BackupCustomEntry[];
  backup_file_filter_rules: BackupFileFilterRule[];
  launch_on_startup: boolean;
  minimize_to_tray_on_close: boolean;
  start_minimized: boolean;
  start_lightweight: boolean;
  lightweight_on_close: boolean;
  keep_computer_awake: boolean;
  proxy_mode: ProxyMode;
  proxy_url: string;
  theme: string;
  auto_backup_enabled: boolean;
  auto_backup_interval_days: number;
  auto_backup_max_keep: number;
  last_auto_backup_time: string | null;
  auto_check_update: boolean;
  visible_tabs: string[];
  sidebar_hidden_by_page: SidebarHiddenByPage;
  opencode_allow_clear_applied_oh_my_config: boolean;
  opencode_use_legacy_oh_my_config: boolean;
  opencode_omo_upgrade_confirmed: boolean;
  opencode_dual_write_reasoning_variant: boolean;
  codex_preserve_official_auth_on_switch: boolean;
  codex_unified_session_history_enabled: boolean;
  claude_cli_launch_full_access: boolean;
  /** Windows terminal preference for the Claude CLI launch: `cmd` | `powershell` | `wt` | `gitbash`. */
  preferred_terminal: string | null;
  cli_manual_paths: Record<string, string>;
}

// Default settings
export const defaultSettings: AppSettings = {
  language: 'zh-CN',
  current_module: 'coding',
  current_sub_tab: 'opencode',
  backup_type: 'local',
  local_backup_path: '',
  webdav: {
    url: '',
    username: '',
    password: '',
    remote_path: '',
    host_label: '',
  },
  s3: {
    access_key: '',
    secret_key: '',
    bucket: '',
    region: '',
    prefix: '',
    endpoint_url: '',
    force_path_style: false,
    public_domain: '',
  },
  last_backup_time: null,
  backup_image_assets_enabled: true,
  backup_cli_config_files_enabled: true,
  backup_custom_entries: [],
  backup_file_filter_rules: [],
  launch_on_startup: true,
  minimize_to_tray_on_close: true,
  start_minimized: false,
  start_lightweight: false,
  lightweight_on_close: false,
  keep_computer_awake: false,
  proxy_mode: 'system',
  proxy_url: '',
  theme: 'system',
  auto_backup_enabled: false,
  auto_backup_interval_days: 7,
  auto_backup_max_keep: 10,
  last_auto_backup_time: null,
  auto_check_update: true,
  visible_tabs: ['opencode', 'claudecode', 'claudedesktop', 'codex', 'grok', 'geminicli', 'kimi', 'openclaw', 'pi', 'oh_my_pi', 'hermes', 'dsh', 'gateway', 'image', 'ssh', 'wsl'],
  sidebar_hidden_by_page: createDefaultSidebarHiddenByPage(),
  opencode_allow_clear_applied_oh_my_config: false,
  opencode_use_legacy_oh_my_config: false,
  opencode_omo_upgrade_confirmed: false,
  opencode_dual_write_reasoning_variant: false,
  codex_preserve_official_auth_on_switch: false,
  codex_unified_session_history_enabled: false,
  claude_cli_launch_full_access: false,
  preferred_terminal: null,
  cli_manual_paths: {},
};

/**
 * Get settings from database
 */
export const getSettings = async (): Promise<AppSettings> => {
  try {
    const settings = await invoke<AppSettings & {
      sidebar_visibility_by_page?: Partial<Record<SidebarPageKey, LegacySidebarVisibilityValue>>;
    }>('get_settings');
    return {
      ...settings,
      backup_custom_entries: settings.backup_custom_entries ?? [],
      backup_file_filter_rules: settings.backup_file_filter_rules ?? [],
      backup_cli_config_files_enabled: settings.backup_cli_config_files_enabled ?? true,
      codex_preserve_official_auth_on_switch: settings.codex_preserve_official_auth_on_switch ?? false,
      codex_unified_session_history_enabled: settings.codex_unified_session_history_enabled ?? false,
      sidebar_hidden_by_page: normalizeSidebarHiddenByPage(
        settings.sidebar_hidden_by_page ?? settings.sidebar_visibility_by_page
      ),
      cli_manual_paths: settings.cli_manual_paths ?? {},
    };
  } catch (error) {
    console.error('Failed to get settings:', error);
    return defaultSettings;
  }
};

/**
 * Save settings to database
 */
export const saveSettings = async (settings: AppSettings): Promise<void> => {
  await invoke('save_settings', { settings });
};

/**
 * Get persisted session detail filter chips (role + content visibility).
 * Returns null when no record exists yet (first run) — callers default to all-on.
 */
export const getSessionDetailFilters = async (): Promise<SessionDetailFilters | null> => {
  try {
    return await invoke<SessionDetailFilters | null>('get_session_detail_filters');
  } catch (error) {
    console.error('Failed to get session detail filters:', error);
    return null;
  }
};

/**
 * Persist session detail filter chips to the settings record.
 * Uses the dedicated command so it patches only the `session_detail_filters`
 * key instead of racing with a full settings save.
 */
export const saveSessionDetailFilters = async (filters: SessionDetailFilters): Promise<void> => {
  await invoke('save_session_detail_filters', { filters });
};

/**
 * Normalize a custom backup path to ~/... or %APPDATA%/... when possible.
 */
export const normalizeBackupCustomEntryPath = async (path: string): Promise<string> => {
  return invoke<string>('normalize_backup_custom_entry_path', { path });
};

/**
 * List file paths that can currently be excluded from backup by tool.
 */
export const listBackupFileFilterPathOptions = async (): Promise<BackupFileFilterPathOption[]> => {
  return invoke<BackupFileFilterPathOption[]>('list_backup_file_filter_path_options');
};

/**
 * Probe a manual CLI path's version (live), without saving.
 */
export const probeManualCliVersion = async (path: string): Promise<string> => {
  return invoke<string>('probe_manual_cli_version', { path });
};

/**
 * Validate + persist a manual CLI path for a given command name.
 * Returns the probed version. Passing an empty path clears the saved override.
 */
export const saveManualCliPath = async (
  commandName: string,
  path: string
): Promise<string> => {
  return invoke<string>('set_manual_cli_path', { commandName, path });
};

/**
 * Auto-detect the current local CLI path for a command name (for pre-filling
 * the manual CLI path input).
 */
export const detectManualCliPath = async (commandName: string): Promise<string> => {
  return invoke<string>('detect_manual_cli_path', { commandName });
};

/**
 * Update partial settings
 */
export const updateSettings = async (
  partialSettings: Partial<AppSettings>
): Promise<AppSettings> => {
  const currentSettings = await getSettings();
  const newSettings = { ...currentSettings, ...partialSettings };
  await saveSettings(newSettings);
  return newSettings;
};

/**
 * Open the app data directory in file explorer
 */
export const openAppDataDir = async (): Promise<void> => {
  await invoke('open_app_data_dir');
};

/**
 * Set auto launch on startup
 */
export const setAutoLaunch = async (enabled: boolean): Promise<void> => {
  await invoke('set_auto_launch', { enabled });
};

/**
 * Keep the computer awake (prevent idle sleep). Off = restore normal behavior.
 */
export const setKeepAwake = async (enabled: boolean): Promise<void> => {
  await invoke('set_keep_awake', { enabled });
};

/**
 * Get auto launch status
 */
export const getAutoLaunchStatus = async (): Promise<boolean> => {
  try {
    return await invoke<boolean>('get_auto_launch_status');
  } catch (error) {
    console.error('Failed to get auto launch status:', error);
    return false;
  }
};

/**
 * Restart the application
 */
export const restartApp = async (): Promise<void> => {
  await invoke('restart_app');
};

/** Custom data-directory configuration (issue #345). */
export interface AppDataDirInfo {
  /** The override path stored in the bootstrap file, if any (null = default). */
  override: string | null;
  /** The directory actually in effect for the current running session. */
  effective: string;
  /** The platform-default directory (no override). */
  default: string;
  /** These describe the running session and the separately saved next start. */
  is_custom: boolean;
  next_start: string;
  restart_required: boolean;
}

/**
 * Read the current data-directory configuration for the settings UI.
 */
export const getAppDataDirInfo = async (): Promise<AppDataDirInfo> => {
  return invoke<AppDataDirInfo>('get_app_data_dir_info');
};

/**
 * Set or clear the custom data directory. Requires a restart to take effect.
 * Pass null/empty to revert to the platform default.
 */
export const setAppDataDirOverride = async (path: string | null): Promise<AppDataDirInfo> => {
  return invoke<AppDataDirInfo>('set_app_data_dir_override', { path });
};

/**
 * Test proxy connection
 */
export const testProxyConnection = async (proxyUrl: string): Promise<void> => {
  await invoke('test_proxy_connection', { proxyUrl });
};
