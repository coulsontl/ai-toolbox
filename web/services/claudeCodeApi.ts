/**
 * Claude Code API Service
 *
 * Handles all Claude Code configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  ClaudeCodeProvider,
  ClaudeCommonConfig,
  ClaudeCommonConfigInput,
  ConfigPathInfo,
  ClaudeLocalConfigInput,
  ClaudeSettings,
  ClaudePluginStatus,
  ClaudeInstalledPlugin,
  ClaudeKnownMarketplace,
  ClaudeMarketplaceAddInput,
  ClaudeMarketplaceAutoUpdateInput,
  ClaudeMarketplacePlugin,
  ClaudeMarketplaceRemoveInput,
  ClaudeMarketplaceUpdateInput,
  ClaudePluginActionInput,
  ClaudePluginRuntimeStatus,
  ClaudeProviderInput,
} from '@/types/claudecode';
import type { OpenCodeAllApiHubProvider, OpenCodeAllApiHubProvidersResult } from '@/services/opencodeApi';

/**
 * Get Claude Code configuration file path
 */
export const getClaudeConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_claude_config_path');
};

export const getClaudeRootPathInfo = async (): Promise<ConfigPathInfo> => {
  return await invoke<ConfigPathInfo>('get_claude_root_path_info');
};

/**
 * Reveal Claude Code configuration folder in file explorer
 */
export const revealClaudeConfigFolder = async (): Promise<void> => {
  await invoke('reveal_claude_config_folder');
};

/**
 * List all Claude Code providers
 */
export const listClaudeProviders = async (): Promise<ClaudeCodeProvider[]> => {
  return await invoke<ClaudeCodeProvider[]>('list_claude_providers');
};

/**
 * Create a new Claude Code provider
 */
export const createClaudeProvider = async (
  provider: ClaudeProviderInput
): Promise<ClaudeCodeProvider> => {
  return await invoke<ClaudeCodeProvider>('create_claude_provider', { provider });
};

/**
 * Update an existing Claude Code provider
 */
export const updateClaudeProvider = async (
  provider: ClaudeCodeProvider
): Promise<ClaudeCodeProvider> => {
  return await invoke<ClaudeCodeProvider>('update_claude_provider', { provider });
};

/**
 * Delete a Claude Code provider
 */
export const deleteClaudeProvider = async (id: string): Promise<void> => {
  await invoke('delete_claude_provider', { id });
};

/**
 * Reorder Claude Code providers
 * Note: UI for drag-and-drop reordering is not yet implemented
 * This API is reserved for future functionality
 */
export const reorderClaudeProviders = async (ids: string[]): Promise<void> => {
  await invoke('reorder_claude_providers', { ids });
};

/**
 * Select a Claude Code provider (mark as current, but not applied yet)
 */
export const selectClaudeProvider = async (id: string): Promise<void> => {
  await invoke('select_claude_provider', { id });
};

/**
 * Apply Claude Code configuration (write to settings.json)
 */
export const applyClaudeConfig = async (providerId: string): Promise<void> => {
  await invoke('apply_claude_config', { providerId });
};

/**
 * Read Claude Code settings.json
 */
export const readClaudeSettings = async (): Promise<ClaudeSettings> => {
  return await invoke<ClaudeSettings>('read_claude_settings');
};

/**
 * Get common configuration
 */
export const getClaudeCommonConfig = async (): Promise<ClaudeCommonConfig | null> => {
  return await invoke<ClaudeCommonConfig | null>('get_claude_common_config');
};

export const extractClaudeCommonConfigFromCurrentFile = async (): Promise<ClaudeCommonConfig> => {
  return await invoke<ClaudeCommonConfig>('extract_claude_common_config_from_current_file');
};

/**
 * Save common configuration
 */
export const saveClaudeCommonConfig = async (input: ClaudeCommonConfigInput): Promise<void> => {
  await invoke('save_claude_common_config', { input });
};

/**
 * Save local config (provider and/or common) into database
 */
export const saveClaudeLocalConfig = async (
  input: ClaudeLocalConfigInput
): Promise<void> => {
  await invoke('save_claude_local_config', { input });
};

/**
 * Get Claude plugin integration status
 */
export const getClaudePluginStatus = async (): Promise<ClaudePluginStatus> => {
  return await invoke<ClaudePluginStatus>('get_claude_plugin_status');
};

/**
 * Apply Claude plugin configuration
 * @param enabled - true to enable third-party providers (set primaryApiKey = "any"),
 *                  false to disable (remove primaryApiKey field)
 */
export const applyClaudePluginConfig = async (enabled: boolean): Promise<boolean> => {
  return await invoke<boolean>('apply_claude_plugin_config', { enabled });
};

export const getClaudePluginRuntimeStatus = async (): Promise<ClaudePluginRuntimeStatus> => {
  return await invoke<ClaudePluginRuntimeStatus>('get_claude_plugin_runtime_status');
};

export const listClaudeInstalledPlugins = async (): Promise<ClaudeInstalledPlugin[]> => {
  return await invoke<ClaudeInstalledPlugin[]>('list_claude_installed_plugins');
};

export const listClaudeKnownMarketplaces = async (): Promise<ClaudeKnownMarketplace[]> => {
  return await invoke<ClaudeKnownMarketplace[]>('list_claude_known_marketplaces');
};

export const listClaudeMarketplacePlugins = async (): Promise<ClaudeMarketplacePlugin[]> => {
  return await invoke<ClaudeMarketplacePlugin[]>('list_claude_marketplace_plugins');
};

export const addClaudeMarketplace = async (input: ClaudeMarketplaceAddInput): Promise<void> => {
  await invoke('add_claude_marketplace', { input });
};

export const updateClaudeMarketplace = async (
  input: ClaudeMarketplaceUpdateInput,
): Promise<void> => {
  await invoke('update_claude_marketplace', { input });
};

export const setClaudeMarketplaceAutoUpdate = async (
  input: ClaudeMarketplaceAutoUpdateInput,
): Promise<void> => {
  await invoke('set_claude_marketplace_auto_update', { input });
};

export const removeClaudeMarketplace = async (
  input: ClaudeMarketplaceRemoveInput,
): Promise<void> => {
  await invoke('remove_claude_marketplace', { input });
};

export const installClaudePluginUserScope = async (
  input: ClaudePluginActionInput,
): Promise<void> => {
  await invoke('install_claude_plugin_user_scope', { input });
};

export const enableClaudePluginUserScope = async (
  input: ClaudePluginActionInput,
): Promise<void> => {
  await invoke('enable_claude_plugin_user_scope', { input });
};

export const disableClaudePluginUserScope = async (
  input: ClaudePluginActionInput,
): Promise<void> => {
  await invoke('disable_claude_plugin_user_scope', { input });
};

export const updateClaudePluginUserScope = async (
  input: ClaudePluginActionInput,
): Promise<void> => {
  await invoke('update_claude_plugin_user_scope', { input });
};

export const uninstallClaudePluginUserScope = async (
  input: ClaudePluginActionInput,
): Promise<void> => {
  await invoke('uninstall_claude_plugin_user_scope', { input });
};

/**
 * Toggle is_disabled status for a provider
 */
export async function toggleClaudeCodeProviderDisabled(
  providerId: string,
  isDisabled: boolean
): Promise<void> {
  return invoke('toggle_claude_code_provider_disabled', {
    providerId,
    isDisabled,
  });
}

/**
 * Get Claude onboarding status
 * @returns true if hasCompletedOnboarding is set
 */
export const getClaudeOnboardingStatus = async (): Promise<boolean> => {
  return await invoke<boolean>('get_claude_onboarding_status');
};

/**
 * Skip Claude Code initial setup confirmation
 * Writes hasCompletedOnboarding=true to ~/.claude.json
 */
export const applyClaudeOnboardingSkip = async (): Promise<boolean> => {
  return await invoke<boolean>('apply_claude_onboarding_skip');
};

/**
 * Restore Claude Code initial setup confirmation
 * Removes hasCompletedOnboarding field from ~/.claude.json
 */
export const clearClaudeOnboardingSkip = async (): Promise<boolean> => {
  return await invoke<boolean>('clear_claude_onboarding_skip');
};

export const listClaudeAllApiHubProviders = async (): Promise<OpenCodeAllApiHubProvidersResult> => {
  return await invoke<OpenCodeAllApiHubProvidersResult>('list_claude_all_api_hub_providers');
};

export const resolveClaudeAllApiHubProviders = async (
  providerIds: string[]
): Promise<OpenCodeAllApiHubProvider[]> => {
  return await invoke<OpenCodeAllApiHubProvider[]>('resolve_claude_all_api_hub_providers', {
    request: { providerIds },
  });
};
