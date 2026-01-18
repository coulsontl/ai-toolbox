/**
 * OpenCode API Service
 *
 * Handles all OpenCode configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type { OpenCodeConfig } from '@/types/opencode';

/**
 * Configuration path information
 */
export interface ConfigPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

/**
 * OpenCode common configuration
 */
export interface OpenCodeCommonConfig {
  configPath: string | null;
  updatedAt: string;
}

/**
 * Result of reading OpenCode config file
 */
export type ReadConfigResult =
  | { status: 'success'; config: OpenCodeConfig }
  | { status: 'notFound'; path: string }
  | { status: 'parseError'; path: string; error: string; contentPreview?: string }
  | { status: 'error'; error: string };

/**
 * Get OpenCode configuration file path
 */
export const getOpenCodeConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_opencode_config_path');
};

/**
 * Get OpenCode configuration path info including source
 */
export const getOpenCodeConfigPathInfo = async (): Promise<ConfigPathInfo> => {
  return await invoke<ConfigPathInfo>('get_opencode_config_path_info');
};

/**
 * Read OpenCode configuration file with detailed result
 */
export const readOpenCodeConfigWithResult = async (): Promise<ReadConfigResult> => {
  return await invoke<ReadConfigResult>('read_opencode_config');
};

/**
 * Backup OpenCode configuration file by renaming it with .bak.{timestamp} suffix
 * @returns The backup file path
 */
export const backupOpenCodeConfig = async (): Promise<string> => {
  return await invoke<string>('backup_opencode_config');
};

/**
 * Read OpenCode configuration file (legacy function, returns null on not found)
 * @deprecated Use readOpenCodeConfigWithResult instead for better error handling
 */
export const readOpenCodeConfig = async (): Promise<OpenCodeConfig | null> => {
  const result = await readOpenCodeConfigWithResult();
  if (result.status === 'success') {
    return result.config;
  }
  return null;
};

/**
 * Save OpenCode configuration file
 */
export const saveOpenCodeConfig = async (config: OpenCodeConfig): Promise<void> => {
  await invoke('save_opencode_config', { config });
};

/**
 * Get OpenCode common config
 */
export const getOpenCodeCommonConfig = async (): Promise<OpenCodeCommonConfig | null> => {
  return await invoke<OpenCodeCommonConfig | null>('get_opencode_common_config');
};

/**
 * Save OpenCode common config
 */
export const saveOpenCodeCommonConfig = async (config: OpenCodeCommonConfig): Promise<void> => {
  await invoke('save_opencode_common_config', { config });
};

/**
 * Free model information
 */
export interface FreeModel {
  id: string;
  name: string;
  providerId: string;       // Config key (e.g., "opencode")
  providerName: string;     // Display name (e.g., "OpenCode Zen")
  context?: number;
}

/**
 * Response for get_opencode_free_models command
 */
export interface FreeModelsResponse {
  freeModels: FreeModel[];
  total: number;
  fromCache: boolean;
}

/**
 * Get OpenCode free models from opencode channel
 * @param forceRefresh Force refresh from API (ignore cache)
 */
export const getOpenCodeFreeModels = async (forceRefresh: boolean = false): Promise<FreeModelsResponse> => {
  return await invoke<FreeModelsResponse>('get_opencode_free_models', { forceRefresh });
};

/**
 * Provider models data stored in database
 */
export interface ProviderModelsData {
  providerId: string;
  value: Record<string, unknown>;
  updatedAt: string;
}

/**
 * Get provider models data by provider ID
 * @param providerId The provider ID (e.g., "openai", "anthropic", "google")
 */
export const getProviderModels = async (providerId: string): Promise<ProviderModelsData | null> => {
  return await invoke<ProviderModelsData | null>('get_provider_models', { providerId });
};

/**
 * Unified model option for both custom and official providers
 */
export interface UnifiedModelOption {
  id: string;           // Format: "provider_id/model_id"
  displayName: string;  // Format: "Provider Name / Model Name (Free?)"
  providerId: string;
  modelId: string;
  isFree: boolean;      // Whether this is a free model
}

/**
 * Get unified model list combining custom providers and official providers from auth.json
 * Returns all available models sorted by display name
 */
export const getOpenCodeUnifiedModels = async (): Promise<UnifiedModelOption[]> => {
  return await invoke<UnifiedModelOption[]>('get_opencode_unified_models');
};

// ============================================================================
// Official Auth Providers Types
// ============================================================================

/**
 * Official model information from auth.json providers
 */
export interface OfficialModel {
  id: string;
  name: string;
  context?: number;
  output?: number;
  isFree: boolean;
}

/**
 * Official provider information from auth.json
 */
export interface OfficialProvider {
  id: string;
  name: string;
  models: OfficialModel[];
}

/**
 * Response for get_opencode_auth_providers command
 */
export interface GetAuthProvidersResponse {
  /** Official providers that are NOT in custom config (standalone) */
  standaloneProviders: OfficialProvider[];
  /** Official models from providers that ARE in custom config (merged) */
  mergedModels: Record<string, OfficialModel[]>;
  /** All custom provider IDs for reference */
  customProviderIds: string[];
}

/**
 * Get official auth providers data from auth.json
 * Returns providers split into standalone (not in custom config) and merged (models only)
 */
export const getOpenCodeAuthProviders = async (): Promise<GetAuthProvidersResponse> => {
  return await invoke<GetAuthProvidersResponse>('get_opencode_auth_providers');
};

/**
 * Get auth.json file path
 */
export const getOpenCodeAuthConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_opencode_auth_config_path');
};
