export type GeminiCliProviderCategory = 'official' | 'custom' | 'third_party' | string;
export type GeminiCliApiFormat = 'gemini_native' | 'openai_chat' | 'openai_responses' | 'anthropic';

export interface GatewayProviderMeta {
  providerType?: string;
  apiFormat?: GeminiCliApiFormat | string;
  costMultiplier?: string;
  pricingModelSource?: 'upstream' | 'requested' | string;
}

export interface GeminiCliSettingsConfig {
  env?: Record<string, string>;
  config?: Record<string, unknown>;
}

export interface GeminiCliProvider {
  id: string;
  name: string;
  category: GeminiCliProviderCategory;
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

export interface GeminiCliProviderInput {
  id?: string;
  name: string;
  category: GeminiCliProviderCategory;
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

export type GeminiCliOfficialAccountKind = 'oauth' | 'local';

export interface GeminiCliOfficialAccount {
  id: string;
  providerId: string;
  name: string;
  kind: GeminiCliOfficialAccountKind;
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

export interface GeminiCliProviderFormValues {
  name: string;
  category: GeminiCliProviderCategory;
  settingsConfig: string;
  apiFormat?: GeminiCliApiFormat;
  meta?: GatewayProviderMeta;
  notes?: string;
}

export interface GeminiCliCommonConfig {
  config: string;
  rootDir?: string | null;
  updatedAt?: string;
}

export interface GeminiCliCommonConfigInput {
  config: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface GeminiCliLocalConfigInput {
  provider?: GeminiCliProviderInput;
  commonConfig?: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface GeminiCliSettings {
  env?: Record<string, string>;
  config?: Record<string, unknown>;
}

export interface ConfigPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}
