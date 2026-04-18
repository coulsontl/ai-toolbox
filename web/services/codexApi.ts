/**
 * Codex API Service
 *
 * Handles all Codex configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  CodexProvider,
  CodexOfficialAccount,
  CodexCommonConfig,
  CodexCommonConfigInput,
  ConfigPathInfo,
  CodexLocalConfigInput,
  CodexSettings,
  CodexInstalledPlugin,
  CodexMarketplacePlugin,
  CodexPluginActionInput,
  CodexPluginMarketplace,
  CodexPluginRuntimeStatus,
  CodexPluginWorkspaceRoot,
  CodexPluginWorkspaceRootInput,
} from '@/types/codex';
import type { OpenCodeAllApiHubProvider, OpenCodeAllApiHubProvidersResult } from '@/services/opencodeApi';

/**
 * Get Codex config directory path
 */
export const getCodexConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_codex_config_dir_path');
};

export const getCodexRootPathInfo = async (): Promise<ConfigPathInfo> => {
  return await invoke<ConfigPathInfo>('get_codex_root_path_info');
};

/**
 * Get Codex config.toml file path
 */
export const getCodexConfigFilePath = async (): Promise<string> => {
  return await invoke<string>('get_codex_config_file_path');
};

export const getCodexPluginRuntimeStatus = async (): Promise<CodexPluginRuntimeStatus> => {
  return await invoke<CodexPluginRuntimeStatus>('get_codex_plugin_runtime_status');
};

export const listCodexInstalledPlugins = async (): Promise<CodexInstalledPlugin[]> => {
  return await invoke<CodexInstalledPlugin[]>('list_codex_installed_plugins');
};

export const listCodexMarketplaces = async (): Promise<CodexPluginMarketplace[]> => {
  return await invoke<CodexPluginMarketplace[]>('list_codex_marketplaces');
};

export const listCodexPluginWorkspaceRoots = async (): Promise<CodexPluginWorkspaceRoot[]> => {
  return await invoke<CodexPluginWorkspaceRoot[]>('list_codex_plugin_workspace_roots');
};

export const addCodexPluginWorkspaceRoot = async (
  input: CodexPluginWorkspaceRootInput,
): Promise<void> => {
  await invoke('add_codex_plugin_workspace_root', { input });
};

export const removeCodexPluginWorkspaceRoot = async (
  input: CodexPluginWorkspaceRootInput,
): Promise<void> => {
  await invoke('remove_codex_plugin_workspace_root', { input });
};

export const listCodexMarketplacePlugins = async (): Promise<CodexMarketplacePlugin[]> => {
  return await invoke<CodexMarketplacePlugin[]>('list_codex_marketplace_plugins');
};

export const installCodexPlugin = async (input: CodexPluginActionInput): Promise<void> => {
  await invoke('install_codex_plugin', { input });
};

export const enableCodexPlugin = async (input: CodexPluginActionInput): Promise<void> => {
  await invoke('enable_codex_plugin', { input });
};

export const disableCodexPlugin = async (input: CodexPluginActionInput): Promise<void> => {
  await invoke('disable_codex_plugin', { input });
};

export const uninstallCodexPlugin = async (input: CodexPluginActionInput): Promise<void> => {
  await invoke('uninstall_codex_plugin', { input });
};

export const enableCodexPluginsFeature = async (): Promise<void> => {
  await invoke('enable_codex_plugins_feature');
};

/**
 * Reveal Codex config folder in file explorer
 */
export const revealCodexConfigFolder = async (): Promise<void> => {
  await invoke('reveal_codex_config_folder');
};

/**
 * List all Codex providers
 */
export const listCodexProviders = async (): Promise<CodexProvider[]> => {
  return await invoke<CodexProvider[]>('list_codex_providers');
};

export const listCodexOfficialAccounts = async (providerId: string): Promise<CodexOfficialAccount[]> => {
  return await invoke<CodexOfficialAccount[]>('list_codex_official_accounts', { providerId });
};

export const startCodexOfficialAccountOauth = async (
  providerId: string,
): Promise<CodexOfficialAccount> => {
  return await invoke<CodexOfficialAccount>('start_codex_official_account_oauth', { providerId });
};

export const saveCodexOfficialLocalAccount = async (
  providerId: string,
): Promise<CodexOfficialAccount> => {
  return await invoke<CodexOfficialAccount>('save_codex_official_local_account', { providerId });
};

export const applyCodexOfficialAccount = async (
  providerId: string,
  accountId: string,
): Promise<void> => {
  await invoke('apply_codex_official_account', { providerId, accountId });
};

export const deleteCodexOfficialAccount = async (
  providerId: string,
  accountId: string,
): Promise<void> => {
  await invoke('delete_codex_official_account', { providerId, accountId });
};

export const refreshCodexOfficialAccountLimits = async (
  providerId: string,
  accountId: string,
): Promise<CodexOfficialAccount> => {
  return await invoke<CodexOfficialAccount>('refresh_codex_official_account_limits', {
    providerId,
    accountId,
  });
};

export const copyCodexOfficialAccountToken = async (
  providerId: string,
  accountId: string,
  tokenKind: 'access' | 'refresh',
): Promise<void> => {
  await invoke('copy_codex_official_account_token', {
    input: {
      providerId,
      accountId,
      tokenKind,
    },
  });
};

/**
 * Create a new Codex provider
 */
export const createCodexProvider = async (
  provider: Omit<CodexProvider, 'id' | 'createdAt' | 'updatedAt'>
): Promise<CodexProvider> => {
  return await invoke<CodexProvider>('create_codex_provider', { provider });
};

/**
 * Update an existing Codex provider
 */
export const updateCodexProvider = async (
  provider: CodexProvider
): Promise<CodexProvider> => {
  return await invoke<CodexProvider>('update_codex_provider', { provider });
};

/**
 * Delete a Codex provider
 */
export const deleteCodexProvider = async (id: string): Promise<void> => {
  await invoke('delete_codex_provider', { id });
};

/**
 * Select a Codex provider
 */
export const selectCodexProvider = async (id: string): Promise<void> => {
  await invoke('select_codex_provider', { id });
};

export async function toggleCodexProviderDisabled(
  providerId: string,
  isDisabled: boolean
): Promise<void> {
  await invoke('toggle_codex_provider_disabled', { providerId, isDisabled });
}

/**
 * Read Codex settings from files
 */
export const readCodexSettings = async (): Promise<CodexSettings> => {
  return await invoke<CodexSettings>('read_codex_settings');
};

/**
 * Get common configuration
 */
export const getCodexCommonConfig = async (): Promise<CodexCommonConfig | null> => {
  return await invoke<CodexCommonConfig | null>('get_codex_common_config');
};

export const extractCodexCommonConfigFromCurrentFile = async (): Promise<CodexCommonConfig> => {
  return await invoke<CodexCommonConfig>('extract_codex_common_config_from_current_file');
};

/**
 * Save common configuration
 */
export const saveCodexCommonConfig = async (input: CodexCommonConfigInput): Promise<void> => {
  await invoke('save_codex_common_config', { input });
};

/**
 * Reorder Codex providers
 */
export const reorderCodexProviders = async (ids: string[]): Promise<void> => {
  await invoke('reorder_codex_providers', { ids });
};

/**
 * Save local config (provider and/or common) into database
 */
export const saveCodexLocalConfig = async (
  input: CodexLocalConfigInput
): Promise<void> => {
  await invoke('save_codex_local_config', { input });
};

export const listCodexAllApiHubProviders = async (): Promise<OpenCodeAllApiHubProvidersResult> => {
  return await invoke<OpenCodeAllApiHubProvidersResult>('list_codex_all_api_hub_providers');
};

export const resolveCodexAllApiHubProviders = async (
  providerIds: string[]
): Promise<OpenCodeAllApiHubProvider[]> => {
  return await invoke<OpenCodeAllApiHubProvider[]>('resolve_codex_all_api_hub_providers', {
    request: { providerIds },
  });
};
