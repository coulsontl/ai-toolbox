import type { CustomHeaderEntry } from '@/features/coding/shared/providerHeaders/customHeadersUtils';
import type { ModelRewriteEntry } from '@/features/coding/shared/providerModelRewrites/modelRewritesUtils';

export type AntigravityProviderCategory = 'official' | 'custom' | 'third_party' | string;
export type AntigravityApiFormat = 'gemini_native' | 'openai_chat' | 'openai_responses' | 'anthropic';

export interface GatewayProviderProfileReference {
  tool?: 'claude' | 'codex' | 'grok' | 'gemini' | 'antigravity';
  profileId: string;
  endpointId: string;
}

export interface GatewayProviderMeta {
  gatewayProfile?: GatewayProviderProfileReference;
  providerType?: string;
  apiFormat?: AntigravityApiFormat | string;
  apiKeyField?: string;
  reasoningField?: 'reasoning_content' | 'content' | 'reasoning' | 'none' | 'all' | string;
  defaultMaxTokens?: number;
  imageInputPolicy?: 'auto' | 'preserve' | 'strip' | 'text_only' | string;
  textOnlyModels?: string[];
  imageCapableModels?: string[];
  allowTextOnlyModelHeuristic?: boolean;
  costMultiplier?: string;
  pricingModelSource?: 'upstream' | 'requested' | string;
  /** Provider-level custom request-header overrides applied by the gateway on upstream requests. */
  customHeaders?: CustomHeaderEntry[];
  /** Provider-level exact model rewrite rules applied by the gateway. */
  modelRewrites?: ModelRewriteEntry[];
}

export interface AntigravitySettingsConfig {
  env?: Record<string, string>;
  config?: Record<string, unknown>;
}

export interface AntigravityProvider {
  id: string;
  name: string;
  category: AntigravityProviderCategory;
  settingsConfig: string;
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

export interface AntigravityOfficialModel {
  id: string;
  name?: string;
  ownedBy?: string;
  created?: number;
}

export interface AntigravityOfficialModelsResponse {
  models: AntigravityOfficialModel[];
  total: number;
  source: string;
}

export type AntigravityOfficialAccountKind = 'oauth' | 'local';

export interface AntigravityOfficialAccount {
  id: string;
  providerId: string;
  name: string;
  kind: AntigravityOfficialAccountKind;
  email?: string;
  authMode?: string;
  accountId?: string;
  projectId?: string;
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

export interface AntigravityCommonConfig {
  config: string;
  rootDir?: string | null;
  updatedAt?: string;
}

export interface AntigravityCommonConfigInput {
  config: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface AntigravitySettings {
  env?: Record<string, string>;
  config?: Record<string, unknown>;
}

export interface ConfigPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

export interface AntigravityLocalRuleFile {
  relativePath: string;
  fullPath: string;
  kind: 'config_gemini' | 'config_agents' | 'shared_gemini' | 'home_agents' | 'rules_dir' | string;
  sizeBytes: number;
  content: string;
  isManagedTarget: boolean;
}
