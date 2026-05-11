/**
 * OpenCode API Service
 *
 * Handles all OpenCode configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type { OpenCodeConfig, OpenCodeProvider } from '@/types/opencode';

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
  showPluginsInTray: boolean;
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
 * Read current OpenCode providers from the active config file.
 * Returns an empty object when config is missing or unreadable.
 */
export const readCurrentOpenCodeProviders = async (): Promise<Record<string, OpenCodeProvider>> => {
  const config = await readOpenCodeConfig();
  return config?.provider || {};
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
  baseModelId?: string;
  experimentalMode?: string;
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
  baseModelId?: string;
  experimentalMode?: string;
}

/**
 * Get unified model list combining custom providers and official providers from auth.json
 * Returns all available models sorted by display name
 */
export const getOpenCodeUnifiedModels = async (): Promise<UnifiedModelOption[]> => {
  return await invoke<UnifiedModelOption[]>('get_opencode_unified_models');
};

/**
 * Build a map of model ID to its variant keys
 * This combines variants from:
 * 1. Config providers (config.provider[providerId].models[modelId].variants)
 * 2. Preset models (PRESET_MODELS)
 *
 * @param config - The OpenCode config
 * @param unifiedModels - The unified model list
 * @param presetModels - Optional preset models (defaults to PRESET_MODELS)
 * @returns Record<string, string[]> - Map of model ID to variant keys
 */
export const buildModelVariantsMap = (
  config: { provider?: Record<string, { models?: Record<string, { variants?: Record<string, unknown> }> }> } | null | undefined,
  unifiedModels: UnifiedModelOption[],
  presetModels?: Record<string, Array<{ id: string; variants?: Record<string, unknown> }>>
): Record<string, string[]> => {
  const variantsMap: Record<string, string[]> = {};
  const variantKeysByConfiguredModel = new Map<string, string[]>();
  const variantKeysByPresetModel = new Map<string, string[]>();

  // Get variants from config providers
  if (config?.provider) {
    Object.entries(config.provider).forEach(([providerId, provider]) => {
      if (provider.models) {
        Object.entries(provider.models).forEach(([modelId, model]) => {
          if (model.variants && Object.keys(model.variants).length > 0) {
            const variantKeys = Object.keys(model.variants);
            const fullModelId = `${providerId}/${modelId}`;
            variantKeysByConfiguredModel.set(fullModelId, variantKeys);
            variantsMap[fullModelId] = variantKeys;
          }
        });
      }
    });
  }

  // Get variants from preset models (for npm-based providers)
  if (presetModels) {
    Object.entries(presetModels).forEach(([_npmPackage, models]) => {
      models.forEach((model) => {
        if (model.variants && Object.keys(model.variants).length > 0) {
          const variantKeys = Object.keys(model.variants);
          variantKeysByPresetModel.set(model.id, variantKeys);
          // Match preset model ID with unified model IDs
          unifiedModels.forEach((um) => {
            if (um.modelId === model.id && !variantsMap[um.id]) {
              variantsMap[um.id] = variantKeys;
            }
          });
        }
      });
    });
  }

  // OpenCode expands models.dev experimental modes into virtual model IDs,
  // e.g. gpt-5.5 + mode fast -> gpt-5.5-fast. Those virtual models inherit
  // the base model variants, so the variant dropdown should remain available.
  unifiedModels.forEach((um) => {
    if (variantsMap[um.id]) return;
    if (!um.baseModelId || !um.experimentalMode) return;

    const baseUnifiedId = `${um.providerId}/${um.baseModelId}`;
    const baseVariantKeys = variantsMap[baseUnifiedId]
      ?? variantKeysByConfiguredModel.get(baseUnifiedId)
      ?? variantKeysByPresetModel.get(um.baseModelId);
    if (baseVariantKeys && baseVariantKeys.length > 0) {
      variantsMap[um.id] = baseVariantKeys;
    }
  });

  return variantsMap;
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
  status?: string;
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
  /** Provider IDs that can resolve auth.json credential + default API base URL */
  resolvedAuthProviderIds: string[];
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

// ============================================================================
// Favorite Plugin Types and Functions
// ============================================================================

/**
 * Favorite plugin information
 */
export interface OpenCodeFavoritePlugin {
  id: string;
  pluginName: string;
  createdAt: string;
}

/**
 * List all favorite plugins
 * Auto-initializes default plugins if database is empty
 */
export const listFavoritePlugins = async (): Promise<OpenCodeFavoritePlugin[]> => {
  return await invoke<OpenCodeFavoritePlugin[]>('list_opencode_favorite_plugins');
};

/**
 * Add a favorite plugin
 * Returns the created plugin, or existing one if already exists
 */
export const addFavoritePlugin = async (pluginName: string): Promise<OpenCodeFavoritePlugin> => {
  return await invoke<OpenCodeFavoritePlugin>('add_opencode_favorite_plugin', { pluginName });
};

/**
 * Delete a favorite plugin by plugin name
 */
export const deleteFavoritePlugin = async (pluginName: string): Promise<void> => {
  await invoke('delete_opencode_favorite_plugin', { pluginName });
};

// ============================================================================
// Favorite Provider Types and Functions
// ============================================================================

/**
 * Favorite provider information (stored in database)
 */
export interface OpenCodeFavoriteProvider {
  id: string;
  providerId: string;
  /** SDK package name (extracted from providerConfig.npm) */
  npm: string;
  /** Base URL (extracted from providerConfig.options.baseURL, can be empty) */
  baseUrl: string;
  /** Complete provider configuration */
  providerConfig: OpenCodeProvider;
  /** Last used diagnostics configuration */
  diagnostics?: OpenCodeDiagnosticsConfig;
  createdAt: string;
  updatedAt: string;
}

export interface OpenCodeDiagnosticsConfig {
  prompt: string;
  defaultTestModelId?: string;
  temperature?: number;
  maxTokens?: number;
  maxOutputTokens?: number;
  stream?: boolean;
  headers?: Record<string, unknown>;
  body?: Record<string, unknown>;
}

/**
 * List all favorite providers
 */
export const listFavoriteProviders = async (): Promise<OpenCodeFavoriteProvider[]> => {
  return await invoke<OpenCodeFavoriteProvider[]>('list_opencode_favorite_providers');
};

/**
 * Upsert (create or update) a favorite provider
 * Called automatically when user adds/modifies a provider
 */
export const upsertFavoriteProvider = async (
  providerId: string,
  providerConfig: OpenCodeProvider,
  diagnostics?: OpenCodeDiagnosticsConfig
): Promise<OpenCodeFavoriteProvider> => {
  return await invoke<OpenCodeFavoriteProvider>('upsert_opencode_favorite_provider', {
    providerId,
    providerConfig,
    diagnostics,
  });
};

/**
 * Delete a favorite provider from database
 */
export const deleteFavoriteProvider = async (providerId: string): Promise<void> => {
  await invoke('delete_opencode_favorite_provider', { providerId });
};

export interface AllApiHubProfileInfo {
  profileName: string;
  extensionId: string;
  path: string;
}

export interface OpenCodeAllApiHubProvider {
  providerId: string;
  name: string;
  npm: string;
  baseUrl?: string;
  requiresBrowserOpen: boolean;
  isDisabled: boolean;
  hasApiKey: boolean;
  apiKeyPreview?: string;
  balanceUsd?: number;
  balanceCny?: number;
  siteName?: string;
  siteType?: string;
  accountLabel: string;
  sourceProfileName: string;
  sourceExtensionId: string;
  providerConfig: OpenCodeProvider;
}

export interface OpenCodeAllApiHubProvidersResult {
  found: boolean;
  profiles: AllApiHubProfileInfo[];
  providers: OpenCodeAllApiHubProvider[];
  message?: string;
}

export const listOpenCodeAllApiHubProviders = async (): Promise<OpenCodeAllApiHubProvidersResult> => {
  return await invoke<OpenCodeAllApiHubProvidersResult>('list_opencode_all_api_hub_providers');
};

export const resolveOpenCodeAllApiHubProviders = async (
  providerIds: string[]
): Promise<OpenCodeAllApiHubProvider[]> => {
  return await invoke<OpenCodeAllApiHubProvider[]>('resolve_opencode_all_api_hub_providers', {
    request: { providerIds },
  });
};


// ============================================================================
// Connectivity Test Types and Functions
// ============================================================================

export interface ConnectivityTestRequest {
  npm: string;
  providerId?: string;
  baseUrl: string;
  apiKey?: string;
  reasoningEffort?: string;
  headers?: Record<string, unknown>;
  prompt: string;
  temperature?: number;
  maxTokens?: number;
  maxOutputTokens?: number;
  stream?: boolean;
  body?: Record<string, unknown>;
  modelIds: string[];
  timeoutSecs?: number;
}

export interface ConnectivityTestResult {
  modelId: string;
  status: string;
  firstByteMs?: number;
  totalMs?: number;
  errorMessage?: string;
  requestUrl: string;
  requestHeaders: Record<string, unknown>;
  requestBody: Record<string, unknown>;
  responseHeaders?: Record<string, unknown>;
  responseBody?: unknown;
}

export interface ConnectivityTestResponse {
  results: ConnectivityTestResult[];
}

/**
 * Test connectivity for provider models
 */
export const testProviderModelConnectivity = async (
  request: ConnectivityTestRequest
): Promise<ConnectivityTestResponse> => {
  return await invoke<ConnectivityTestResponse>('test_provider_model_connectivity', { request });
};
