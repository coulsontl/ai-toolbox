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

export const SIDEBAR_PAGE_KEYS = ['opencode', 'claudecode', 'codex', 'openclaw'] as const;

export type SidebarPageKey = typeof SIDEBAR_PAGE_KEYS[number];

export type SidebarHiddenByPage = Record<SidebarPageKey, boolean>;

export type ProxyMode = 'direct' | 'custom' | 'system';

type LegacySidebarVisibilityValue = boolean | {
  hidden?: boolean;
};

export const createDefaultSidebarHiddenByPage = (): SidebarHiddenByPage => ({
  opencode: false,
  claudecode: false,
  codex: false,
  openclaw: false,
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
  launch_on_startup: boolean;
  minimize_to_tray_on_close: boolean;
  start_minimized: boolean;
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
  launch_on_startup: true,
  minimize_to_tray_on_close: true,
  start_minimized: false,
  proxy_mode: 'system',
  proxy_url: '',
  theme: 'system',
  auto_backup_enabled: false,
  auto_backup_interval_days: 7,
  auto_backup_max_keep: 10,
  last_auto_backup_time: null,
  auto_check_update: true,
  visible_tabs: ['opencode', 'claudecode', 'codex', 'openclaw', 'image', 'ssh', 'wsl'],
  sidebar_hidden_by_page: createDefaultSidebarHiddenByPage(),
  opencode_allow_clear_applied_oh_my_config: false,
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
      sidebar_hidden_by_page: normalizeSidebarHiddenByPage(
        settings.sidebar_hidden_by_page ?? settings.sidebar_visibility_by_page
      ),
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

/**
 * Test proxy connection
 */
export const testProxyConnection = async (proxyUrl: string): Promise<void> => {
  await invoke('test_proxy_connection', { proxyUrl });
};
