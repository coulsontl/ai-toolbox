/**
 * Claude Code API Service
 *
 * Handles all Claude Code configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  ClaudeCodeProvider,
  ClaudeCommonConfig,
  ClaudeSettings,
} from '@/types/claudecode';

/**
 * Get Claude Code configuration file path
 */
export const getClaudeConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_claude_config_path');
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
  provider: Omit<ClaudeCodeProvider, 'createdAt' | 'updatedAt'>
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

/**
 * Save common configuration
 */
export const saveClaudeCommonConfig = async (config: string): Promise<void> => {
  await invoke('save_claude_common_config', { config });
};
