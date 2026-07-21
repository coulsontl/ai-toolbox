/**
 * Grok Configuration Types
 *
 * Type definitions for Grok configuration management.
 */

export type GrokProviderCategory = 'official' | 'third_party' | 'custom';
export type GrokApiFormat =
  | 'openai_responses'
  | 'openai_chat'
  | 'anthropic_messages';

export interface GatewayProviderProfileReference {
  tool?: 'claude' | 'grok' | 'gemini';
  profileId: string;
  endpointId: string;
}

export interface GatewayProviderMeta {
  gatewayProfile?: GatewayProviderProfileReference;
  providerType?: string;
  apiFormat?: GrokApiFormat | string;
  apiKeyField?: string;
  isFullUrl?: boolean;
  promptCacheKey?: string;
  reasoningField?: 'reasoning_content' | 'content' | 'reasoning' | 'none' | 'all' | string;
  defaultMaxTokens?: number;
  grokChatReasoning?: Record<string, unknown>;
  imageInputPolicy?: 'auto' | 'preserve' | 'strip' | 'text_only' | string;
  textOnlyModels?: string[];
  imageCapableModels?: string[];
  allowTextOnlyModelHeuristic?: boolean;
  costMultiplier?: string;
  pricingModelSource?: 'upstream' | 'requested' | string;
}

export interface GrokAuthConfig extends Record<string, unknown> {
  API_KEY?: string;
}

export interface GrokCatalogModel {
  key?: string;
  model: string;
  displayName?: string;
  description?: string;
  baseUrl?: string;
  apiBackend?: 'chat' | 'responses' | 'messages' | string;
  apiKey?: string | null;
  envKey?: string;
  contextWindow?: string | number;
  maxCompletionTokens?: number;
  temperature?: number;
  topP?: number;
  supportsBackendSearch?: boolean;
  supportsReasoningEffort?: boolean;
  reasoningEffort?: string;
  streamToolCalls?: boolean;
  maxRetries?: number;
  inferenceIdleTimeoutSecs?: number;
  extraHeaders?: Record<string, string>;
  extraConfig?: Record<string, unknown>;
  supportsImage?: boolean;
  vision?: boolean;
  attachment?: boolean;
  modalities?: {
    input?: string[];
    output?: string[];
  };
}

export interface GrokModelCatalog {
  models: GrokCatalogModel[];
}

/**
 * Grok Provider settings configuration
 * Contains auth.json and config.toml content
 */
export interface GrokSettingsConfig {
  auth?: GrokAuthConfig;
  config?: string; // TOML format string
  defaultModelKey?: string;
  /**
   * Official providers only. Projects to `[models].default_reasoning_effort`.
   * Custom providers store effort on `modelCatalog.models[].reasoningEffort`.
   */
  defaultReasoningEffort?: string;
  modelCatalog?: GrokModelCatalog;
}

/**
 * Grok Provider stored in database
 */
export interface GrokProvider {
  id: string;
  name: string;
  category: GrokProviderCategory;
  settingsConfig: string; // JSON string of GrokSettingsConfig
  sourceProviderId?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  meta?: GatewayProviderMeta;
  isApplied?: boolean;
  isDisabled?: boolean;
  createdAt: string;
  updatedAt: string;
}

/**
 * Common configuration for all providers
 */
export interface GrokCommonConfig {
  config: string; // TOML format string
  rootDir?: string | null;
  updatedAt?: string;
}

export interface ConfigPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

/**
 * Grok settings from files
 */
export interface GrokSettings {
  auth?: Record<string, unknown>;
  config?: string;
}

export type GrokOfficialAccountKind = 'oauth' | 'local';

export interface GrokOfficialAccount {
  id: string;
  providerId: string;
  name: string;
  kind: GrokOfficialAccountKind;
  email?: string;
  subject?: string;
  tokenEndpoint?: string;
  expiresAt?: number;
  lastRefresh?: string;
  lastError?: string;
  sortIndex?: number;
  isApplied: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface GrokOfficialModel {
  id: string;
  name?: string;
  ownedBy?: string;
  created?: number;
}

export interface GrokOfficialModelsResponse {
  models: GrokOfficialModel[];
  total: number;
  source: 'remote' | 'bundled';
  tier: string;
}

export interface GrokPluginRuntimeStatus {
  mode: 'local' | 'wslDirect';
  source: 'custom' | 'env' | 'shell' | 'default';
  rootDir: string;
  configPath: string;
  pluginsDir: string;
  curatedMarketplacePath?: string;
  distro?: string;
  linuxRootDir?: string;
}

export interface GrokPluginMarketplace {
  name: string;
  path: string;
  displayName?: string;
  description?: string;
  pluginCount: number;
  isCurated: boolean;
}

export interface GrokPluginWorkspaceRoot {
  path: string;
  status: 'ready' | 'missing';
  resolutionSource?: 'direct' | 'gitRepo';
  resolvedMarketplacePath?: string;
  resolvedRepoRoot?: string;
  error?: string;
}

export interface GrokPluginWorkspaceRootInput {
  path: string;
}

export interface GrokMarketplacePlugin {
  pluginId: string;
  marketplaceName: string;
  marketplacePath: string;
  name: string;
  displayName?: string;
  description?: string;
  category?: string;
  capabilities: string[];
  sourcePath?: string;
  installSource?: string;
  installed: boolean;
  enabled: boolean;
  installAvailable: boolean;
}

export interface GrokInstalledPlugin {
  pluginId: string;
  marketplaceName: string;
  name: string;
  displayName?: string;
  description?: string;
  category?: string;
  installedPath?: string;
  activeVersion?: string;
  enabled: boolean;
  hasSkills: boolean;
  hasMcpServers: boolean;
  hasApps: boolean;
  capabilities: string[];
}

export interface GrokPluginActionInput {
  pluginId: string;
  source?: string;
}

export interface GrokPluginBulkActionInput {
  enabled: boolean;
}

export interface GrokPluginBulkActionResult {
  updatedCount: number;
  failures: string[];
}

/**
 * Form values for creating/editing a provider
 */
export interface GrokProviderFormValues {
  name: string;
  category: GrokProviderCategory;
  // 新架构：直接使用 settingsConfig（JSON 字符串）
  settingsConfig?: string;
  // 旧架构（向后兼容）
  providerEndpointKey?: string;
  providerProfileId?: string;
  providerEndpointId?: string;
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  configToml?: string;
  meta?: GatewayProviderMeta;
  apiFormat?: GrokApiFormat;
  notes?: string;
  sourceProviderId?: string;
}

/**
 * Provider input for saving local config
 */
export interface GrokProviderInput {
  name: string;
  category: GrokProviderCategory;
  settingsConfig: string;
  sourceProviderId?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  meta?: GatewayProviderMeta;
  isDisabled?: boolean;
}

/**
 * Local config save input
 */
export interface GrokLocalConfigInput {
  provider?: GrokProviderInput;
  commonConfig?: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface GrokCommonConfigInput {
  config: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

/**
 * Import conflict action
 */
export type ImportConflictAction = 'overwrite' | 'duplicate' | 'cancel';

/**
 * Import conflict info
 */
export interface ImportConflictInfo {
  existingProvider: GrokProvider;
  newProviderName: string;
  sourceProviderId: string;
}
