// 相对路径（不用 `@/` 别名）：本模块被 node:test 单测直接 import，而测试用的
// loader 只解析相对 specifier。
import { LOCAL_CONFIG_ID } from '../features/coding/shared/localConfig';

export type KimiProviderCategory = 'official' | 'custom' | string;

/**
 * Temporary provider projected from the on-disk config when the DB has none.
 *
 * 与其余 CLI 共用同一个保留 id（见 `shared/localConfig.ts`），不再各写各的字面量。
 */
export const KIMI_LOCAL_PROVIDER_ID = LOCAL_CONFIG_ID;

export interface KimiProviderFormData {
  name: string;
  category: KimiProviderCategory;
  settingsConfig: string;
  notes?: string;
  meta?: Record<string, unknown>;
}

export interface KimiCatalogModel {
  key: string;
  model: string;
  provider: string;
  displayName?: string;
  /** `max_context_size` — required by the CLI to be a positive integer. */
  maxContextSize?: number;
  /** `max_input_size` — optional, must be a positive integer when present. */
  maxInputSize?: number;
  /** `max_output_size` — optional, must be a positive integer when present. */
  maxOutputSize?: number;
  /** `reasoning_key` — the response field carrying reasoning content. */
  reasoningKey?: string;
  capabilities?: string[];
  supportEfforts?: string[];
  defaultEffort?: string;
  [key: string]: unknown;
}

/** One catalog model from the bundled models.dev data (`get_kimi_preset_models`). */
export interface KimiPresetModel {
  id: string;
  displayName?: string;
  maxContextSize?: number;
  maxInputSize?: number;
  maxOutputSize?: number;
  reasoning: boolean;
  /** The catalog's tool-support flag; gates the `tool_use` capability. */
  toolCall: boolean;
  inputModalities: string[];
  outputModalities: string[];
}

export interface KimiProviderConfig {
  type?: string;
  base_url?: string;
  [key: string]: unknown;
}

export interface KimiSettingsConfig {
  auth?: {
    API_KEY?: string;
    [key: string]: unknown;
  };
  defaultModelKey?: string;
  providerConfigs?: Record<string, KimiProviderConfig>;
  modelCatalog?: {
    models?: KimiCatalogModel[];
    [key: string]: unknown;
  };
  config?: string;
  [key: string]: unknown;
}

export interface KimiProvider {
  id: string;
  name: string;
  category: KimiProviderCategory;
  settingsConfig: string;
  sourceProviderId?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  meta?: Record<string, unknown>;
  isApplied?: boolean;
  isDisabled?: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface KimiProviderInput {
  id?: string;
  name: string;
  category: KimiProviderCategory;
  settingsConfig: string;
  sourceProviderId?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  meta?: Record<string, unknown>;
  isDisabled?: boolean;
}

export interface KimiCommonConfig {
  config: string;
  rootDir?: string | null;
  /**
   * Where the official-account card sits in the provider list, counted as the
   * number of provider cards above it. UI state, not Kimi state — the backend
   * keeps it in the common-config singleton because that is the module's only
   * singleton row.
   */
  officialAccountIndex?: number | null;
  updatedAt?: string;
}

export interface KimiCommonConfigInput {
  config: string;
  rootDir?: string | null;
  clearRootDir?: boolean;
}

export interface KimiPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

/** Live on-disk Kimi settings returned by `read_kimi_settings`. */
export interface KimiSettings {
  config: string | null;
}

export interface KimiOfficialAccount {
  id: string;
  providerId: string;
  name: string;
  kind: string;
  email?: string;
  subject?: string;
  tokenEndpoint?: string;
  expiresAt?: number;
  lastRefresh?: string;
  lastError?: string;
  planType?: string;
  limitWeeklyText?: string;
  limitMonthlyText?: string;
  limitWeeklyResetAt?: number;
  limitMonthlyResetAt?: number;
  lastLimitsFetchedAt?: string;
  isApplied: boolean;
  sortIndex?: number;
  createdAt: string;
  updatedAt: string;
  /**
   * Mirrors the login that is live on disk but has no stored row yet. It can
   * only be saved — never switched to (it is what is live) or deleted.
   */
  isVirtual?: boolean;
}

export interface KimiPlugin {
  name: string;
  version?: string;
  description?: string;
  enabled?: boolean;
}

export interface KimiDeviceAuthStartResult {
  sessionId: string;
  verificationUri: string;
  verificationUriComplete?: string;
  userCode: string;
  expiresAt: number;
  pollIntervalSeconds: number;
}

export interface KimiAuthStatusEvent {
  sessionId: string;
  status: string;
  message?: string;
  accountId?: string;
}


