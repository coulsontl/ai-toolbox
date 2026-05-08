/**
 * API service for oh-my-opencode-slim plugin configuration management.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  OhMyOpenCodeSlimConfig,
  OhMyOpenCodeSlimConfigInput,
  OhMyOpenCodeSlimGlobalConfig,
  OhMyOpenCodeSlimGlobalConfigInput,
  ConfigPathInfo,
} from '@/types/ohMyOpenCodeSlim';

/**
 * List all oh-my-opencode-slim configurations
 */
export const listOhMyOpenCodeSlimConfigs = async (): Promise<OhMyOpenCodeSlimConfig[]> => {
  return await invoke<OhMyOpenCodeSlimConfig[]>('list_oh_my_opencode_slim_configs');
};

/**
 * Create a new oh-my-opencode-slim configuration
 */
export const createOhMyOpenCodeSlimConfig = async (
  input: OhMyOpenCodeSlimConfigInput
): Promise<OhMyOpenCodeSlimConfig> => {
  return await invoke<OhMyOpenCodeSlimConfig>('create_oh_my_opencode_slim_config', { input });
};

/**
 * Update an existing oh-my-opencode-slim configuration
 */
export const updateOhMyOpenCodeSlimConfig = async (
  input: OhMyOpenCodeSlimConfigInput
): Promise<OhMyOpenCodeSlimConfig> => {
  return await invoke<OhMyOpenCodeSlimConfig>('update_oh_my_opencode_slim_config', { input });
};

/**
 * Delete an oh-my-opencode-slim configuration
 */
export const deleteOhMyOpenCodeSlimConfig = async (id: string): Promise<void> => {
  await invoke('delete_oh_my_opencode_slim_config', { id });
};

/**
 * Clear the applied oh-my-opencode-slim runtime config file without deleting the saved profile.
 */
export const clearOhMyOpenCodeSlimAppliedConfig = async (configId: string): Promise<void> => {
  await invoke('clear_oh_my_opencode_slim_applied_config', { configId });
};

/**
 * Apply a configuration to the oh-my-opencode-slim.json file
 */
export const applyOhMyOpenCodeSlimConfig = async (configId: string): Promise<void> => {
  await invoke('apply_oh_my_opencode_slim_config', { configId });
};

/**
 * Reorder oh-my-opencode-slim configurations
 */
export const reorderOhMyOpenCodeSlimConfigs = async (ids: string[]): Promise<void> => {
  await invoke('reorder_oh_my_opencode_slim_configs', { ids });
};

/**
 * Get oh-my-opencode-slim config file path info
 */
export const getOhMyOpenCodeSlimConfigPathInfo = async (): Promise<ConfigPathInfo> => {
  return await invoke<ConfigPathInfo>('get_oh_my_opencode_slim_config_path_info');
};

/**
 * Get oh-my-opencode-slim global config
 */
export const getOhMyOpenCodeSlimGlobalConfig = async (): Promise<OhMyOpenCodeSlimGlobalConfig> => {
  return await invoke<OhMyOpenCodeSlimGlobalConfig>('get_oh_my_opencode_slim_global_config');
};

/**
 * Save oh-my-opencode-slim global config
 */
export const saveOhMyOpenCodeSlimGlobalConfig = async (
  input: OhMyOpenCodeSlimGlobalConfigInput
): Promise<OhMyOpenCodeSlimGlobalConfig> => {
  return await invoke<OhMyOpenCodeSlimGlobalConfig>('save_oh_my_opencode_slim_global_config', { input });
};

/**
 * Save local config (both Agents Profile and Global Config) into database
 * This is used when saving __local__ temporary config to database
 */
export interface OhMyOpenCodeSlimLocalConfigInput {
  config?: OhMyOpenCodeSlimConfigInput;
  globalConfig?: OhMyOpenCodeSlimGlobalConfigInput;
}

export const saveOhMyOpenCodeSlimLocalConfig = async (
  input: OhMyOpenCodeSlimLocalConfigInput
): Promise<void> => {
  await invoke('save_oh_my_opencode_slim_local_config', { input });
};

/**
 * Check if local oh-my-opencode-slim config file exists
 * Returns true if ~/.config/opencode/oh-my-opencode-slim.jsonc or .json exists
 */
export const checkOhMyOpenCodeSlimConfigExists = async (): Promise<boolean> => {
  return await invoke<boolean>('check_oh_my_opencode_slim_config_exists');
};

/**
 * Toggle is_disabled status for a config
 */
export async function toggleOhMyOpenCodeSlimConfigDisabled(
  configId: string,
  isDisabled: boolean
): Promise<void> {
  return invoke('toggle_oh_my_opencode_slim_config_disabled', {
    configId,
    isDisabled,
  });
}
