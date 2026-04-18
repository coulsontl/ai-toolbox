/**
 * Codex Configuration Types
 *
 * Type definitions for Codex configuration management.
 */

export type CodexProviderCategory = 'official' | 'third_party' | 'custom';

export interface CodexAuthConfig extends Record<string, unknown> {
  OPENAI_API_KEY?: string;
}

/**
 * Codex Provider settings configuration
 * Contains auth.json and config.toml content
 */
export interface CodexSettingsConfig {
  auth?: CodexAuthConfig;
  config?: string; // TOML format string
}

/**
 * Codex Provider stored in database
 */
export interface CodexProvider {
  id: string;
  name: string;
  category: CodexProviderCategory;
  settingsConfig: string; // JSON string of CodexSettingsConfig
  sourceProviderId?: string;
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
 * Common configuration for all providers
 */
export interface CodexCommonConfig {
  config: string; // TOML format string
  rootDir?: string | null;
  updatedAt?: string;
}

export interface ConfigPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

/**
 * Codex settings from files
 */
export interface CodexSettings {
  auth?: Record<string, unknown>;
  config?: string;
}

export type CodexOfficialAccountKind = 'oauth' | 'local';

export interface CodexOfficialAccount {
  id: string;
  providerId: string;
  name: string;
  kind: CodexOfficialAccountKind;
  email?: string;
  authMode?: string;
  accountId?: string;
  planType?: string;
  lastRefresh?: string;
  tokenExpiresAt?: number;
  accessTokenPreview?: string;
  refreshTokenPreview?: string;
  limitShortLabel?: string;
  limit5hText?: string;
  limitWeeklyText?: string;
  limit5hResetAt?: number;
  limitWeeklyResetAt?: number;
  lastLimitsFetchedAt?: string;
  lastError?: string;
  sortIndex?: number;
  isApplied: boolean;
  isVirtual: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface CodexPluginRuntimeStatus {
  mode: 'local' | 'wslDirect';
  source: 'custom' | 'env' | 'shell' | 'default';
  rootDir: string;
  configPath: string;
  pluginsDir: string;
  pluginsFeatureEnabled: boolean;
  curatedMarketplacePath?: string;
  distro?: string;
  linuxRootDir?: string;
}

export interface CodexPluginMarketplace {
  name: string;
  path: string;
  displayName?: string;
  description?: string;
  pluginCount: number;
  isCurated: boolean;
}

export interface CodexPluginWorkspaceRoot {
  path: string;
  status: 'ready' | 'missing';
  resolutionSource?: 'direct' | 'gitRepo';
  resolvedMarketplacePath?: string;
  resolvedRepoRoot?: string;
  error?: string;
}

export interface CodexPluginWorkspaceRootInput {
  path: string;
}

export interface CodexMarketplacePlugin {
  pluginId: string;
  marketplaceName: string;
  marketplacePath: string;
  name: string;
  displayName?: string;
  description?: string;
  category?: string;
  capabilities: string[];
  sourcePath?: string;
  installed: boolean;
  enabled: boolean;
  installAvailable: boolean;
}

export interface CodexInstalledPlugin {
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

export interface CodexPluginActionInput {
  pluginId: string;
}

/**
 * Form values for creating/editing a provider
 */
export interface CodexProviderFormValues {
  name: string;
  category: CodexProviderCategory;
  // 新架构：直接使用 settingsConfig（JSON 字符串）
  settingsConfig?: string;
  // 旧架构（向后兼容）
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  configToml?: string;
  notes?: string;
  sourceProviderId?: string;
}

/**
 * Provider input for saving local config
 */
export interface CodexProviderInput {
  name: string;
  category: CodexProviderCategory;
  settingsConfig: string;
  sourceProviderId?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  isDisabled?: boolean;
}

/**
 * Local config save input
 */
export interface CodexLocalConfigInput {
  provider?: CodexProviderInput;
  commonConfig?: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface CodexCommonConfigInput {
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
  existingProvider: CodexProvider;
  newProviderName: string;
  sourceProviderId: string;
}
