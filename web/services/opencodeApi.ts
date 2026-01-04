/**
 * OpenCode API Service
 *
 * Handles all OpenCode configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type { OpenCodeConfig } from '@/types/opencode';

/**
 * Get OpenCode configuration file path
 */
export const getOpenCodeConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_opencode_config_path');
};

/**
 * Read OpenCode configuration file
 */
export const readOpenCodeConfig = async (): Promise<OpenCodeConfig | null> => {
  return await invoke<OpenCodeConfig | null>('read_opencode_config');
};

/**
 * Save OpenCode configuration file
 */
export const saveOpenCodeConfig = async (config: OpenCodeConfig): Promise<void> => {
  await invoke('save_opencode_config', { config });
};
