/**
 * Claude Code Configuration Types
 *
 * Type definitions for Claude Code configuration management.
 */

export type ClaudeProviderCategory = 'official' | 'third_party' | 'custom';

/**
 * Claude Code Provider settings configuration
 * Maps to the settings.json env section
 */
export interface ClaudeSettingsConfig {
  env?: {
    ANTHROPIC_AUTH_TOKEN?: string;
    ANTHROPIC_API_KEY?: string; // 兼容旧版本，读取时检查，写入时不使用
    ANTHROPIC_BASE_URL?: string;
    ANTHROPIC_REASONING_MODEL?: string;
  };
  // Model configurations
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
  reasoningModel?: string;
}

/**
 * Claude Code Provider stored in database
 */
export interface ClaudeCodeProvider {
  id: string;
  name: string;
  category: ClaudeProviderCategory;
  settingsConfig: string; // JSON string of ClaudeSettingsConfig
  extraSettingsConfig: string; // JSON string of additional settings.json fields for custom providers
  // Source info if imported from settings
  sourceProviderId?: string;
  // Metadata
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  isApplied?: boolean;
  isDisabled?: boolean;
  createdAt: string;
  updatedAt: string;
}

/**
 * Common configuration that applies to all providers
 * Stored as a single record in database
 */
export interface ClaudeCommonConfig {
  config: string; // JSON string like '{ "statusLine": {...}, "skipWebFetchPreflight": true }'
  rootDir?: string | null;
  updatedAt?: string;
}

export interface ConfigPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

/**
 * Claude Code settings.json file structure
 * Note: Due to #[serde(flatten)] in Rust, other fields are flattened at the top level
 */
export interface ClaudeSettings {
  env?: {
    ANTHROPIC_AUTH_TOKEN?: string;
    ANTHROPIC_API_KEY?: string; // 兼容旧版本
    ANTHROPIC_BASE_URL?: string;
    ANTHROPIC_MODEL?: string;
    ANTHROPIC_DEFAULT_HAIKU_MODEL?: string;
    ANTHROPIC_DEFAULT_SONNET_MODEL?: string;
    ANTHROPIC_DEFAULT_OPUS_MODEL?: string;
    ANTHROPIC_REASONING_MODEL?: string;
  };
  // Common config fields (flattened at top level)
  [key: string]: unknown;
}

/**
 * Form values for creating/editing a provider
 */
export interface ClaudeProviderFormValues {
  name: string;
  category: ClaudeProviderCategory;
  baseUrl?: string;
  apiKey?: string;
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
  reasoningModel?: string;
  extraSettingsConfig?: string;
  notes?: string;
  isDisabled?: boolean;
  // For import from settings
  sourceProviderId?: string;
}

/**
 * Provider input for saving local config
 */
export interface ClaudeProviderInput {
  name: string;
  category: ClaudeProviderCategory;
  settingsConfig: string;
  extraSettingsConfig?: string;
  sourceProviderId?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
}

/**
 * Local config save input
 */
export interface ClaudeLocalConfigInput {
  provider?: ClaudeProviderInput;
  commonConfig?: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface ClaudeCommonConfigInput {
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
  existingProvider: ClaudeCodeProvider;
  newProviderName: string;
  sourceProviderId: string;
}

/**
 * Claude Plugin integration status
 */
export interface ClaudePluginStatus {
  enabled: boolean;       // Whether primaryApiKey = "any" is set
  hasConfigFile: boolean; // Whether ~/.claude/config.json exists
}

export interface ClaudePluginRuntimeStatus {
  mode: 'local' | 'wslDirect';
  source: 'custom' | 'env' | 'shell' | 'default';
  rootDir: string;
  settingsPath: string;
  pluginsDir: string;
  distro?: string;
  linuxRootDir?: string;
}

export interface ClaudeKnownMarketplace {
  name: string;
  source: unknown;
  installLocation?: string;
  lastUpdated?: string;
  autoUpdateEnabled: boolean;
  owner?: {
    name?: string;
    email?: string;
  };
  description?: string;
  version?: string;
  pluginCount: number;
}

export interface ClaudeMarketplacePlugin {
  marketplaceName: string;
  name: string;
  description?: string;
  version?: string;
  homepage?: string;
  repository?: string;
  category?: string;
  tags: string[];
  source: unknown;
  pluginId: string;
}

export interface ClaudeInstalledPlugin {
  pluginId: string;
  name: string;
  marketplaceName: string;
  description?: string;
  version?: string;
  homepage?: string;
  repository?: string;
  installPath?: string;
  userScopeInstalled: boolean;
  userScopeEnabled: boolean;
  installScopes: string[];
  hasSkills: boolean;
  hasAgents: boolean;
  hasHooks: boolean;
  hasMcpServers: boolean;
  hasLspServers: boolean;
}

export interface ClaudeMarketplaceAddInput {
  source: string;
}

export interface ClaudeMarketplaceUpdateInput {
  marketplaceName?: string;
}

export interface ClaudeMarketplaceAutoUpdateInput {
  marketplaceName: string;
  autoUpdateEnabled: boolean;
}

export interface ClaudeMarketplaceRemoveInput {
  marketplaceName: string;
}

export interface ClaudePluginActionInput {
  pluginId: string;
}
