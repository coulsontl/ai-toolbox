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

export interface AppSettings {
  language: string;
  current_module: string;
  current_sub_tab: string;
  backup_type: string;
  local_backup_path: string;
  webdav: WebDAVConfig;
  s3: S3Config;
  last_backup_time: string | null;
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
};

/**
 * Get settings from database
 */
export const getSettings = async (): Promise<AppSettings> => {
  try {
    const settings = await invoke<AppSettings>('get_settings');
    return settings;
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
