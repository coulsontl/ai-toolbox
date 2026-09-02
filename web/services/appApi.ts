/**
 * App API Service
 *
 * Handles app-level operations like version info and updates.
 */

import { getVersion } from '@tauri-apps/api/app';
import { openUrl as openUrlExternal } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
import { PRESET_MODELS_REMOTE_URL, updatePresetModels } from '@/constants/presetModels';
import type { PresetModel } from '@/constants/presetModels';
import {
  markGatewayProviderProfilesInitialized,
  GATEWAY_PROVIDER_PROFILES_REMOTE_URL,
  updateGatewayProviderProfiles,
  type GatewayProviderProfileCatalog,
} from '@/features/coding/shared/gateway/providerProfiles';

const GITHUB_REPO = 'coulsontl/ai-toolbox';
export { GITHUB_REPO };
const GITHUB_URL = `https://github.com/${GITHUB_REPO}`;

export interface UpdateInfo {
  hasUpdate: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseUrl: string;
  releaseNotes: string;
  signature?: string;
  url?: string;
}

interface UpdateCheckResult {
  has_update: boolean;
  current_version: string;
  latest_version: string;
  release_url: string;
  release_notes: string;
  signature?: string;
  url?: string;
}

/**
 * Get current app version
 */
export const getAppVersion = async (): Promise<string> => {
  return await getVersion();
};

/**
 * Check for updates from GitHub releases (via Tauri backend)
 */
export const checkForUpdates = async (): Promise<UpdateInfo> => {
  const result = await invoke<UpdateCheckResult>('check_for_updates');

  return {
    hasUpdate: result.has_update,
    currentVersion: result.current_version,
    latestVersion: result.latest_version,
    releaseUrl: result.release_url,
    releaseNotes: result.release_notes,
    signature: result.signature,
    url: result.url,
  };
};

/**
 * Install the update if available
 */
export const installUpdate = async (): Promise<boolean> => {
  return await invoke('install_update');
};

/**
 * Latest release download page (used as the manual-download fallback when
 * the in-app auto-updater is unavailable or fails).
 */
export const LATEST_RELEASE_URL = `${GITHUB_URL}/releases/latest`;

/**
 * Startup recovery: returns the user-facing error message if the app booted
 * into recovery mode (SQLite schema too new), otherwise `null` (normal boot).
 * DB-free — safe to call before the database is initialized.
 */
export const getStartupRecovery = async (): Promise<string | null> => {
  return await invoke<string | null>('get_startup_recovery');
};

/**
 * Exit the application. Used by the recovery screen when the user gives up
 * after a failed auto-update. DB-free.
 */
export const exitApp = async (): Promise<void> => {
  await invoke('exit_app');
};

/**
 * Open GitHub repository page
 */
export const openGitHubPage = async (): Promise<void> => {
  await openUrlExternal(GITHUB_URL);
};

/**
 * Open a URL in the default browser
 */
export const openExternalUrl = async (url: string): Promise<void> => {
  await openUrlExternal(url);
};

/**
 * Refresh the system tray menu
 */
export const refreshTrayMenu = async (): Promise<void> => {
  await invoke('refresh_tray_menu');
};

export const hasAllApiHubExtension = async (): Promise<boolean> => {
  return await invoke<boolean>('has_all_api_hub_extension');
};

/**
 * Set window background color (affects macOS titlebar color)
 */
export const setWindowBackgroundColor = async (r: number, g: number, b: number): Promise<void> => {
  await invoke('set_window_background_color', { r, g, b });
};

/**
 * Load preset models from local cache file (app data dir).
 * Returns true if the cache was found and applied, false otherwise.
 */
export const loadCachedPresetModels = async (): Promise<boolean> => {
  try {
    const json = await invoke<Record<string, PresetModel[]> | null>(
      'load_cached_preset_models',
    );
    if (json && typeof json === 'object') {
      updatePresetModels(json);
      console.log('[PresetModels] Loaded from local cache');
      return true;
    }
  } catch (err) {
    console.warn('[PresetModels] Failed to load local cache:', err);
  }
  return false;
};

/**
 * Fetch preset models from the remote repository, save to local
 * cache file, and update the in-memory PRESET_MODELS map.
 * Silently falls back to bundled defaults on network or parse errors.
 */
export const fetchRemotePresetModels = async (): Promise<void> => {
  try {
    const json = await invoke<Record<string, PresetModel[]>>(
      'fetch_remote_preset_models',
      { url: PRESET_MODELS_REMOTE_URL },
    );
    if (json && typeof json === 'object') {
      updatePresetModels(json);
      console.log('[PresetModels] Updated from remote');
    }
  } catch (err) {
    console.warn('[PresetModels] Failed to fetch remote, using bundled defaults:', err);
  }
};

export const loadCachedGatewayProviderProfiles = async (): Promise<boolean> => {
  try {
    const json = await invoke<GatewayProviderProfileCatalog | null>(
      'load_cached_gateway_provider_profiles',
    );
    if (json && typeof json === 'object') {
      updateGatewayProviderProfiles(json);
      markGatewayProviderProfilesInitialized();
      console.log('[GatewayProviderProfiles] Loaded from local cache');
      return true;
    }
  } catch (err) {
    console.warn('[GatewayProviderProfiles] Failed to load local cache:', err);
  }
  markGatewayProviderProfilesInitialized();
  return false;
};

export const fetchRemoteGatewayProviderProfiles = async (): Promise<void> => {
  try {
    const json = await invoke<GatewayProviderProfileCatalog>(
      'fetch_remote_gateway_provider_profiles',
      { url: GATEWAY_PROVIDER_PROFILES_REMOTE_URL },
    );
    if (json && typeof json === 'object') {
      updateGatewayProviderProfiles(json);
      markGatewayProviderProfilesInitialized();
      console.log('[GatewayProviderProfiles] Updated from remote');
    }
  } catch (err) {
    console.warn('[GatewayProviderProfiles] Failed to fetch remote:', err);
  }
};
