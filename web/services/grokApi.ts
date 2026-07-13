/**
 * Grok API Service
 *
 * Handles all Grok configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  GrokProvider,
  GrokOfficialAccount,
  GrokOfficialModelsResponse,
  GrokCommonConfig,
  GrokCommonConfigInput,
  ConfigPathInfo,
  GrokLocalConfigInput,
  GrokSettings,
  GrokInstalledPlugin,
  GrokMarketplacePlugin,
  GrokPluginActionInput,
  GrokPluginBulkActionInput,
  GrokPluginBulkActionResult,
  GrokPluginMarketplace,
  GrokPluginRuntimeStatus,
  GrokPluginWorkspaceRoot,
  GrokPluginWorkspaceRootInput,
} from '@/types/grok';
import type { OpenCodeAllApiHubProvider, OpenCodeAllApiHubProvidersResult } from '@/services/opencodeApi';

/**
 * Get Grok config directory path
 */
export const getGrokConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_grok_config_dir_path');
};

export const getGrokRootPathInfo = async (): Promise<ConfigPathInfo> => {
  return await invoke<ConfigPathInfo>('get_grok_root_path_info');
};

/**
 * Get Grok config.toml file path
 */
export const getGrokConfigFilePath = async (): Promise<string> => {
  return await invoke<string>('get_grok_config_file_path');
};

export const getGrokPluginRuntimeStatus = async (): Promise<GrokPluginRuntimeStatus> => {
  return await invoke<GrokPluginRuntimeStatus>('get_grok_plugin_runtime_status');
};

export const listGrokInstalledPlugins = async (): Promise<GrokInstalledPlugin[]> => {
  return await invoke<GrokInstalledPlugin[]>('list_grok_installed_plugins');
};

export const listGrokMarketplaces = async (): Promise<GrokPluginMarketplace[]> => {
  return await invoke<GrokPluginMarketplace[]>('list_grok_marketplaces');
};

export const listGrokPluginWorkspaceRoots = async (): Promise<GrokPluginWorkspaceRoot[]> => {
  return await invoke<GrokPluginWorkspaceRoot[]>('list_grok_plugin_workspace_roots');
};

export const addGrokPluginWorkspaceRoot = async (
  input: GrokPluginWorkspaceRootInput,
): Promise<void> => {
  await invoke('add_grok_plugin_workspace_root', { input });
};

export const removeGrokPluginWorkspaceRoot = async (
  input: GrokPluginWorkspaceRootInput,
): Promise<void> => {
  await invoke('remove_grok_plugin_workspace_root', { input });
};

export const listGrokMarketplacePlugins = async (): Promise<GrokMarketplacePlugin[]> => {
  return await invoke<GrokMarketplacePlugin[]>('list_grok_marketplace_plugins');
};

export const installGrokPlugin = async (input: GrokPluginActionInput): Promise<void> => {
  await invoke('install_grok_plugin', { input });
};

export const enableGrokPlugin = async (input: GrokPluginActionInput): Promise<void> => {
  await invoke('enable_grok_plugin', { input });
};

export const disableGrokPlugin = async (input: GrokPluginActionInput): Promise<void> => {
  await invoke('disable_grok_plugin', { input });
};

export const setGrokInstalledPluginsEnabled = async (
  input: GrokPluginBulkActionInput,
): Promise<GrokPluginBulkActionResult> => {
  return await invoke<GrokPluginBulkActionResult>('set_grok_installed_plugins_enabled', {
    input,
  });
};

export const uninstallGrokPlugin = async (input: GrokPluginActionInput): Promise<void> => {
  await invoke('uninstall_grok_plugin', { input });
};

export const updateGrokPlugin = async (input: GrokPluginActionInput): Promise<void> => {
  await invoke('update_grok_plugin', { input });
};

export const getGrokPluginDetails = async (input: GrokPluginActionInput): Promise<string> => {
  return await invoke<string>('get_grok_plugin_details', { input });
};

export const validateGrokPlugin = async (input: GrokPluginActionInput): Promise<string> => {
  return await invoke<string>('validate_grok_plugin', { input });
};

export const updateGrokPluginMarketplace = async (
  input: GrokPluginWorkspaceRootInput,
): Promise<void> => {
  await invoke('update_grok_plugin_marketplace', { input });
};

/**
 * Reveal Grok config folder in file explorer
 */
export const revealGrokConfigFolder = async (): Promise<void> => {
  await invoke('reveal_grok_config_folder');
};

/**
 * List all Grok providers
 */
export const listGrokProviders = async (): Promise<GrokProvider[]> => {
  return await invoke<GrokProvider[]>('list_grok_providers');
};

export const listGrokOfficialAccounts = async (providerId: string): Promise<GrokOfficialAccount[]> => {
  return await invoke<GrokOfficialAccount[]>('list_grok_official_accounts', { providerId });
};

export interface GrokDeviceAuthStartResult {
  sessionId: string;
  verificationUri: string;
  verificationUriComplete?: string;
  userCode: string;
  expiresAt: number;
  pollIntervalSeconds: number;
}

export const startGrokOfficialAccountDeviceAuth = async (
  providerId: string,
): Promise<GrokDeviceAuthStartResult> => {
  return await invoke<GrokDeviceAuthStartResult>('start_grok_official_account_device_auth', { providerId });
};

export const cancelGrokOfficialAccountDeviceAuth = async (sessionId: string): Promise<void> => {
  await invoke('cancel_grok_official_account_device_auth', { sessionId });
};

export const getGrokOfficialAccountAuthStatus = async (sessionId: string): Promise<string> => {
  return await invoke<string>('get_grok_official_account_auth_status', { sessionId });
};

export const saveGrokOfficialLocalAccount = async (
  providerId: string,
): Promise<GrokOfficialAccount> => {
  return await invoke<GrokOfficialAccount>('save_grok_official_local_account', { providerId });
};

export const applyGrokOfficialAccount = async (accountId: string): Promise<void> => {
  await invoke('apply_grok_official_account', { accountId });
};

export const deleteGrokOfficialAccount = async (accountId: string): Promise<void> => {
  await invoke('delete_grok_official_account', { accountId });
};

export const refreshGrokOfficialAccount = async (accountId: string): Promise<GrokOfficialAccount> => {
  return await invoke<GrokOfficialAccount>('refresh_grok_official_account', { accountId });
};

export const logoutGrokOfficialRuntime = async (): Promise<void> => {
  await invoke('logout_grok_official_runtime');
};

export const fetchGrokOfficialModels = async (): Promise<GrokOfficialModelsResponse> => {
  return await invoke<GrokOfficialModelsResponse>('fetch_grok_official_models');
};

/**
 * Create a new Grok provider
 */
export const createGrokProvider = async (
  provider: Omit<GrokProvider, 'id' | 'createdAt' | 'updatedAt'>
): Promise<GrokProvider> => {
  return await invoke<GrokProvider>('create_grok_provider', { provider });
};

/**
 * Update an existing Grok provider
 */
export const updateGrokProvider = async (
  provider: GrokProvider
): Promise<GrokProvider> => {
  return await invoke<GrokProvider>('update_grok_provider', { provider });
};

/**
 * Delete a Grok provider
 */
export const deleteGrokProvider = async (id: string): Promise<void> => {
  await invoke('delete_grok_provider', { id });
};

/**
 * Select a Grok provider
 */
export const selectGrokProvider = async (id: string): Promise<void> => {
  await invoke('select_grok_provider', { id });
};

export async function toggleGrokProviderDisabled(
  id: string,
  disabled: boolean
): Promise<void> {
  await invoke('toggle_grok_provider_disabled', { id, disabled });
}

/**
 * Read Grok settings from files
 */
export const readGrokSettings = async (): Promise<GrokSettings> => {
  return await invoke<GrokSettings>('read_grok_settings');
};

/**
 * Get common configuration
 */
export const getGrokCommonConfig = async (): Promise<GrokCommonConfig | null> => {
  return await invoke<GrokCommonConfig | null>('get_grok_common_config');
};

export const extractGrokCommonConfigFromCurrentFile = async (): Promise<GrokCommonConfig> => {
  return await invoke<GrokCommonConfig>('extract_grok_common_config_from_current_file');
};

/**
 * Save common configuration
 */
export const saveGrokCommonConfig = async (input: GrokCommonConfigInput): Promise<void> => {
  await invoke('save_grok_common_config', { input });
};

/**
 * Reorder Grok providers
 */
export const reorderGrokProviders = async (ids: string[]): Promise<void> => {
  await invoke('reorder_grok_providers', { ids });
};

/**
 * Save local config (provider and/or common) into database
 */
export const saveGrokLocalConfig = async (
  input: GrokLocalConfigInput
): Promise<void> => {
  await invoke('save_grok_local_config', { input });
};

export const listGrokAllApiHubProviders = async (): Promise<OpenCodeAllApiHubProvidersResult> => {
  return await invoke<OpenCodeAllApiHubProvidersResult>('list_grok_all_api_hub_providers');
};

export const resolveGrokAllApiHubProviders = async (
  providerIds: string[]
): Promise<OpenCodeAllApiHubProvider[]> => {
  return await invoke<OpenCodeAllApiHubProvider[]>('resolve_grok_all_api_hub_providers', {
    request: { providerIds },
  });
};
