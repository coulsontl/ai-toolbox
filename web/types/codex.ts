/**
 * Codex Configuration Types
 *
 * Type definitions for Codex configuration management.
 */

import type { CustomHeaderEntry } from '@/features/coding/shared/providerHeaders/customHeadersUtils';
import type { ModelRewriteEntry } from '@/features/coding/shared/providerModelRewrites/modelRewritesUtils';
import type { ModelServiceTierEntry } from '@/features/coding/shared/providerServiceTiers/serviceTierOverridesUtils';

export type CodexProviderCategory = 'official' | 'third_party' | 'custom';
export type CodexApiFormat =
  | 'openai_responses'
  | 'openai_chat'
  | 'anthropic_messages'
  | 'gemini_native';

export interface GatewayProviderProfileReference {
  tool?: 'claude' | 'codex' | 'grok' | 'gemini';
  profileId: string;
  endpointId: string;
}

export interface GatewayProviderMeta {
  gatewayProfile?: GatewayProviderProfileReference;
  providerType?: string;
  apiFormat?: CodexApiFormat | string;
  apiKeyField?: string;
  isFullUrl?: boolean;
  promptCacheKey?: string;
  reasoningField?: 'reasoning_content' | 'content' | 'reasoning' | 'none' | 'all' | string;
  defaultMaxTokens?: number;
  codexChatReasoning?: Record<string, unknown>;
  /**
   * User opt-in (custom Codex provider with an `openai_chat` upstream) to
   * forward the client's explicit reasoning effort as the top-level
   * `reasoning_effort` field instead of letting the gateway's generic
   * third-party Chat cleanup strip it. `undefined` keeps the provider-specific
   * default; only the Codex provider form writes it (issue #412).
   */
  preserveReasoningEffort?: boolean;
  imageInputPolicy?: 'auto' | 'preserve' | 'strip' | 'text_only' | string;
  textOnlyModels?: string[];
  imageCapableModels?: string[];
  allowTextOnlyModelHeuristic?: boolean;
  costMultiplier?: string;
  pricingModelSource?: 'upstream' | 'requested' | string;
  /**
   * Extra multiplier this provider charges when the upstream serves the Fast
   * service tier (`service_tier = priority|fast`). Omitted means the tier is
   * recorded but not surcharged — OpenAI's own ratio differs per billing mode
   * (subscription allowance vs purchased credits vs API tokens), so there is no
   * correct built-in default.
   */
  fastCostMultiplier?: string;
  /** Same as {@link GatewayProviderMeta.fastCostMultiplier} for Ultrafast. */
  ultrafastCostMultiplier?: string;
  /** Provider-level custom request-header overrides applied by the gateway on upstream requests. */
  customHeaders?: CustomHeaderEntry[];
  /** Provider-level exact model rewrite rules applied by the gateway (issue #321). */
  modelRewrites?: ModelRewriteEntry[];
  /** Provider-level per-model service (speed) tier overrides applied by the
   * gateway (issue #414). Only OpenAI Chat / Responses targets honour them. */
  modelServiceTiers?: ModelServiceTierEntry[];
}

export interface CodexAuthConfig extends Record<string, unknown> {
  OPENAI_API_KEY?: string;
}

export interface CodexCatalogModel {
  model: string;
  displayName?: string;
  contextWindow?: string | number;
  supportsImage?: boolean;
  vision?: boolean;
  attachment?: boolean;
  modalities?: {
    input?: string[];
    output?: string[];
  };
  /** Per-model reasoning effort levels exposed in the generated Codex catalog
   * (e.g. ["low", "medium", "high", "xhigh", "max"]). When omitted the neutral
   * template keeps its 6-level default; official vendor entries keep the
   * vendor's declared levels. */
  reasoningLevels?: string[];
  /** Per-model default reasoning effort. Only meaningful together with
   * reasoningLevels; when omitted the template default is kept if still in the
   * list, otherwise the highest declared level wins. */
  defaultReasoningLevel?: string;
  /** Per-model service (speed) tier ids exposed in the generated Codex catalog
   * (e.g. ["priority", "ultrafast"] for Fast/Ultrafast). The backend expands
   * each id to a full {id,name,description} object; unknown ids are dropped.
   * When omitted the generated entry advertises no speed tier (empty array). */
  serviceTiers?: string[];
}

export interface CodexModelCatalog {
  models: CodexCatalogModel[];
}

/**
 * Codex Provider settings configuration
 * Contains auth.json and config.toml content
 */
export interface CodexSettingsConfig {
  auth?: CodexAuthConfig;
  config?: string; // TOML format string
  modelCatalog?: CodexModelCatalog;
  /**
   * Provider-level Codex auto-review model. When set, every generated
   * model_catalog entry gets auto_review_model_override so guardian review
   * uses this slug instead of the default codex-auto-review.
   */
  autoReviewModelOverride?: string;
  /**
   * Explicit override for the generated `requires_openai_auth` line (issue #394).
   * Omitted means "derive it from the auth mechanism"; an absent key *is* auto,
   * so `auto` is never stored. Only meaningful for `custom` providers — official
   * ones are always automatic.
   */
  requiresOpenaiAuthMode?: CodexRequiresOpenaiAuthMode;
}

/**
 * Explicit `requires_openai_auth` override stored on a custom provider.
 * `keep`/`strip` win over the automatic auth-mechanism rule.
 */
export type CodexRequiresOpenaiAuthMode = 'keep' | 'strip';

/** The provider form's three-way selection; `auto` normalizes to an absent key. */
export type CodexRequiresOpenaiAuthModeSelection = 'auto' | CodexRequiresOpenaiAuthMode;

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
  meta?: GatewayProviderMeta;
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
  /** Raw text of the model catalog file named by config.toml's `model_catalog_json` pointer (or the leftover AI Toolbox catalog file when the pointer is absent). */
  modelCatalog?: string;
  /** Whether config.toml's `model_catalog_json` pointer is set, i.e. Codex actually reads the catalog. */
  modelCatalogActive?: boolean;
}

export type CodexOfficialAccountKind = 'oauth' | 'local';

export interface CodexCredits {
  hasCredits?: boolean;
  unlimited?: boolean;
  /** Native Codex credits, not currency, tokens or a percentage. May be negative. */
  balance?: number;
}

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
  limitMonthlyText?: string;
  limit5hResetAt?: number;
  limitWeeklyResetAt?: number;
  limitMonthlyResetAt?: number;
  resetCreditsAvailable?: number;
  credits?: CodexCredits;
  lastLimitsFetchedAt?: string;
  lastError?: string;
  sortIndex?: number;
  isApplied: boolean;
  isVirtual: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface CodexOfficialModel {
  id: string;
  name?: string;
  ownedBy?: string;
  created?: number;
}

export interface CodexOfficialModelsResponse {
  models: CodexOfficialModel[];
  total: number;
  source: 'remote' | 'bundled';
  tier: string;
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

export interface CodexPluginBulkActionInput {
  enabled: boolean;
}

export interface CodexPluginBulkActionResult {
  updatedCount: number;
}

export type CodexHistorySourceMode = 'all' | 'local' | 'wsl';

export interface CodexHistorySourceOption {
  source: 'local' | 'wsl';
  distro?: string | null;
}

export interface CodexHistorySyncStatus {
  codexHome: string;
  configPath: string;
  dbPath: string;
  sessionsDir: string;
  sessionIndexPath: string;
  backupDir: string;
  currentProvider: string;
  currentModel?: string;
  totalThreads: number;
  providerMismatchThreads: number;
  modelMismatchThreads: number;
  modelColumnExists: boolean;
  sessionFileCount: number;
  sessionMetaMismatchCount: number;
  indexedThreads: number;
  missingSessionIndexEntries: number;
  backupCount: number;
  latestBackupPath?: string;
  availableSources?: CodexHistorySourceOption[];
  runtimeSource?: 'local' | 'wsl';
  runtimeDistro?: string | null;
  hasWork: boolean;
}

export interface CodexHistoryBackupResult {
  backupPath: string;
  backupDir: string;
  durationMs: number;
}

export interface CodexHistorySyncResult {
  status: CodexHistorySyncStatus;
  backupPath: string;
  updatedThreadRows: number;
  updatedSessionFiles: number;
  failedSessionFiles: number;
  firstSessionFileError?: string;
  rewrittenIndexEntries: number;
  missingSessionIndexEntriesBefore: number;
  preservedIndexOnlyEntries: number;
  attempts: number;
  lockWaitMs: number;
  durationMs: number;
  partialSuccess: boolean;
}

export interface CodexHistoryRestoreResult {
  restoredBackupPath: string;
  safetyBackupPath: string;
  restoredSessionMetaFiles: number;
  skippedSessionMetaFiles: number;
  rewrittenIndexEntries: number;
  attempts: number;
  lockWaitMs: number;
  durationMs: number;
  status: CodexHistorySyncStatus;
}

export interface CodexUnifiedHistoryMigrationResult {
  migratedSessionFiles: number;
  migratedSessionEntries: number;
  migratedThreadRows: number;
  rewrittenIndexEntries: number;
  backupPath?: string;
  skippedReason?: string;
  durationMs: number;
}

export interface CodexUnifiedHistoryRestoreResult {
  restoredSessionFiles: number;
  restoredSessionEntries: number;
  restoredThreadRows: number;
  rewrittenIndexEntries: number;
  backupPath?: string;
  skippedReason?: string;
  durationMs: number;
}

export interface CodexUnifiedSessionHistoryUpdateResult {
  enabled: boolean;
  migration?: CodexUnifiedHistoryMigrationResult;
}

/**
 * Form values for creating/editing a provider
 */
export interface CodexProviderFormValues {
  name: string;
  category: CodexProviderCategory;
  // 新架构：直接使用 settingsConfig（JSON 字符串）
  settingsConfig?: string;
  /** Explicit `requires_openai_auth` override; `auto` keeps deriving it. */
  requiresOpenaiAuthMode?: CodexRequiresOpenaiAuthModeSelection;
  // 旧架构（向后兼容）
  providerEndpointKey?: string;
  providerProfileId?: string;
  providerEndpointId?: string;
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  configToml?: string;
  meta?: GatewayProviderMeta;
  apiFormat?: CodexApiFormat;
  /** Form-only toggle: forward the client's reasoning effort to a custom
   * `openai_chat` upstream (persisted as `meta.preserveReasoningEffort`). */
  preserveReasoningEffort?: boolean;
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
  meta?: GatewayProviderMeta;
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

/**
 * Codex memories management (issue #296)
 */
export type CodexMemoriesSourceMode = 'local' | 'wsl';

export interface CodexMemoriesSourceOption {
  source: 'local' | 'wsl';
  distro?: string | null;
}

export interface CodexMemoriesEntry {
  name: string;
  relativePath: string;
  entryType: 'file' | 'directory';
  size: number;
  modifiedAtMs?: number | null;
}

export interface CodexMemoriesListResult {
  rootPath: string;
  source: 'local' | 'wsl';
  distro?: string | null;
  availableSources: CodexMemoriesSourceOption[];
  entries: CodexMemoriesEntry[];
  /** Requested source does not exist on this machine (e.g. host root is WSL Direct). */
  unavailable?: boolean;
}

export interface CodexMemoryFileContent {
  content: string;
  size: number;
  modifiedAtMs?: number | null;
}

/**
 * Codex project-less ("scratch") chat residue (issue #400).
 *
 * Codex creates a workspace directory and a `[projects]` trust entry for every
 * chat started without a project, and reclaims neither when the chat is deleted.
 */
export interface CodexScratchWorkspaceInfo {
  path: string;
  exists: boolean;
  isEmpty: boolean;
  fileCount: number;
  totalBytes: number;
  hasGit: boolean;
  truncated: boolean;
}

export interface CodexScratchResidueWorkspace {
  info: CodexScratchWorkspaceInfo;
  runtimeSource: 'local' | 'wsl';
  runtimeDistro?: string | null;
}

export interface CodexScratchResidueTrustEntry {
  key: string;
  configPath: string;
  dirExists: boolean;
  runtimeSource: 'local' | 'wsl';
  runtimeDistro?: string | null;
}

export interface CodexScratchResidue {
  source: CodexHistorySourceMode;
  /** The requested source has no Codex home on this machine. */
  unavailable: boolean;
  /**
   * `false` when the rollout scan could not prove that a directory is
   * unreferenced. Cleanup must not be offered in that case.
   */
  scanComplete: boolean;
  configPaths: string[];
  workspaces: CodexScratchResidueWorkspace[];
  trustEntries: CodexScratchResidueTrustEntry[];
  emptyDateDirs: string[];
}

export interface CodexScratchCleanSkip {
  target: string;
  reason: string;
}

export interface CodexScratchCleanFailure {
  target: string;
  error: string;
}

export interface CodexScratchCleanResult {
  removedWorkspaces: string[];
  removedTrustKeys: string[];
  removedDateDirs: string[];
  skipped: CodexScratchCleanSkip[];
  failures: CodexScratchCleanFailure[];
}

/**
 * A process that keeps serving Codex from the config it read at startup.
 *
 * `daemon` is the CLI's detached background service; `desktopApp` is the
 * ChatGPT desktop app, which runs its own bundled app-server. Both hold
 * `config.toml` / `auth.json`, so an applied provider stays invisible until the
 * holder restarts (issue #443).
 */
export type CodexConfigHolder = 'daemon' | 'desktopApp';

/**
 * What to do after applying a provider while a holder is still running.
 *
 * - `prompt`: show the restart notification (default; the behaviour before this
 *   setting existed).
 * - `auto`: restart without asking, then report the outcome.
 * - `off`: neither prompt nor restart; the user restarts Codex themselves.
 */
export type CodexDaemonRestartMode = 'prompt' | 'auto' | 'off';

/**
 * State of the processes holding the Codex config.
 *
 * Codex 0.16x reads config and credentials once when an app-server starts, so
 * anything AI Toolbox writes afterwards is invisible to that process (issue #443).
 */
export interface CodexDaemonStatus {
  running: boolean;
  /** Every holder found; empty when nothing is running. */
  holders: CodexConfigHolder[];
  /** Version reported by the CLI daemon; absent when it did not answer. */
  appServerVersion?: string | null;
}
