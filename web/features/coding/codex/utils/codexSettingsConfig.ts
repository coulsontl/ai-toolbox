import type {
  CodexAuxiliaryModelRoute,
  CodexAuxiliaryTask,
  CodexCatalogModel,
  CodexProviderCategory,
  CodexRequiresOpenaiAuthMode,
  CodexRequiresOpenaiAuthModeSelection,
  CodexSettingsConfig,
} from '../../../../types/codex';
import {
  normalizeCodexConfigForOfficialMode,
  removeCodexBaseUrl,
  removeCodexModel,
  setCodexBaseUrl,
  setCodexModel,
  setCodexReasoningEffort,
} from '../../../../utils/codexConfigUtils';
import { isJsonObject } from '../../../../utils/json';
import { normalizeCodexCatalogModels } from './codexCatalogModels';

export interface BuildCodexSettingsConfigInput {
  category: CodexProviderCategory;
  apiKey: string;
  baseUrl: string;
  model: string;
  config: string;
  /** Explicit default reasoning level for the main model; omitted means keep. */
  reasoningEffort?: string;
  catalogModels: CodexCatalogModel[];
  /** Per-task auxiliary routes; a missing task key means automatic. */
  auxiliaryModels?: CodexSettingsConfig['auxiliaryModels'];
  autoReviewModelOverride?: string;
  /** Explicit `requires_openai_auth` override; `auto` means keep deriving it. */
  requiresOpenaiAuthMode?: CodexRequiresOpenaiAuthModeSelection;
  auth: Record<string, unknown>;
}

export function normalizeCodexAutoReviewModelOverride(value: unknown): string | undefined {
  if (typeof value !== 'string') {
    return undefined;
  }
  const normalized = value.trim();
  return normalized || undefined;
}

/**
 * Normalize the explicit `requires_openai_auth` override (issue #394).
 * Only `keep`/`strip` carry an opinion; `auto` and anything unrecognized mean
 * "derive it from the auth mechanism", which the backend stores as an absent
 * key — so a typo can never pin the projection to a wrong state.
 */
export function normalizeCodexRequiresOpenaiAuthMode(
  value: unknown,
): CodexRequiresOpenaiAuthMode | undefined {
  return value === 'keep' || value === 'strip' ? value : undefined;
}

/**
 * Resolve provider-level auto-review override.
 * Prefer top-level settingsConfig.autoReviewModelOverride; fall back to the
 * first non-empty legacy per-model value for short-lived local drafts.
 */
export function resolveCodexAutoReviewModelOverride(
  settings: CodexSettingsConfig | undefined,
): string | undefined {
  if (!settings) {
    return undefined;
  }

  const topLevel = normalizeCodexAutoReviewModelOverride(settings.autoReviewModelOverride);
  if (topLevel) {
    return topLevel;
  }

  const models = settings.modelCatalog?.models || [];
  for (const item of models) {
    const legacyItem = item as CodexCatalogModel & {
      autoReviewModelOverride?: unknown;
      auto_review_model_override?: unknown;
    };
    const fromCamel = normalizeCodexAutoReviewModelOverride(legacyItem.autoReviewModelOverride);
    if (fromCamel) {
      return fromCamel;
    }
    const fromSnake = normalizeCodexAutoReviewModelOverride(legacyItem.auto_review_model_override);
    if (fromSnake) {
      return fromSnake;
    }
  }

  return undefined;
}

export function parseCodexSettingsConfig(rawConfig: string | undefined): CodexSettingsConfig {
  if (!rawConfig?.trim()) return {};

  try {
    const parsedConfig = JSON.parse(rawConfig) as unknown;
    return isJsonObject(parsedConfig) ? parsedConfig as CodexSettingsConfig : {};
  } catch (error) {
    console.error('Failed to parse Codex settings config:', error);
    return {};
  }
}

const AUXILIARY_TASKS: CodexAuxiliaryTask[] = [
  'title',
  'description',
  'summary',
  'prediction',
  'memory',
];

/**
 * Normalize the stored per-task routes. `auto` is the absence of a key, so an
 * explicit `auto` object, an empty model or an unknown mode all disappear.
 */
export function normalizeCodexAuxiliaryModels(
  value: unknown,
): Partial<Record<CodexAuxiliaryTask, CodexAuxiliaryModelRoute>> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return undefined;
  }
  const source = value as Record<string, unknown>;
  const result: Partial<Record<CodexAuxiliaryTask, CodexAuxiliaryModelRoute>> = {};
  for (const task of AUXILIARY_TASKS) {
    const route = source[task];
    if (!route || typeof route !== 'object' || Array.isArray(route)) {
      continue;
    }
    const mode = (route as { mode?: unknown }).mode;
    if (mode === 'main') {
      result[task] = { mode: 'main' };
      continue;
    }
    if (mode === 'model') {
      const model = (route as { model?: unknown }).model;
      if (typeof model === 'string' && model.trim()) {
        result[task] = { mode: 'model', model: model.trim() };
      }
    }
  }
  return Object.keys(result).length > 0 ? result : undefined;
}

export function buildCodexSettingsConfig({
  category,
  apiKey,
  baseUrl,
  model,
  config,
  reasoningEffort,
  catalogModels,
  auxiliaryModels,
  autoReviewModelOverride,
  requiresOpenaiAuthMode,
  auth,
}: BuildCodexSettingsConfigInput): string {
  let finalConfig = config;
  const normalizedApiKey = apiKey.trim();
  const normalizedCatalogModels = normalizeCodexCatalogModels(catalogModels);
  const normalizedAuxiliaryModels = normalizeCodexAuxiliaryModels(auxiliaryModels);
  const normalizedAutoReviewModelOverride = normalizeCodexAutoReviewModelOverride(
    autoReviewModelOverride,
  );
  const normalizedRequiresOpenaiAuthMode = normalizeCodexRequiresOpenaiAuthMode(
    requiresOpenaiAuthMode,
  );

  if (category === 'custom') {
    finalConfig = baseUrl
      ? setCodexBaseUrl(finalConfig, baseUrl)
      : removeCodexBaseUrl(finalConfig);
  } else {
    finalConfig = normalizeCodexConfigForOfficialMode(finalConfig);
  }
  finalConfig = model
    ? setCodexModel(finalConfig, model)
    : removeCodexModel(finalConfig);
  // Only projected when the caller explicitly resolves a level for the main
  // model; a row without a default level must not clear config.toml.
  if (reasoningEffort?.trim()) {
    finalConfig = setCodexReasoningEffort(finalConfig, reasoningEffort);
  }

  const finalAuth = { ...auth };
  if (category === 'custom' && normalizedApiKey) {
    finalAuth.OPENAI_API_KEY = normalizedApiKey;
  } else {
    delete finalAuth.OPENAI_API_KEY;
  }

  const settingsConfig: CodexSettingsConfig = {
    auth: finalAuth,
    config: finalConfig.trim(),
  };
  if (category === 'custom' && normalizedCatalogModels.length > 0) {
    settingsConfig.modelCatalog = {
      models: normalizedCatalogModels,
    };
  }
  if (category === 'custom' && normalizedAuxiliaryModels) {
    settingsConfig.auxiliaryModels = normalizedAuxiliaryModels;
  }
  if (category === 'custom' && normalizedAutoReviewModelOverride) {
    settingsConfig.autoReviewModelOverride = normalizedAutoReviewModelOverride;
  }
  if (category === 'custom' && normalizedRequiresOpenaiAuthMode) {
    settingsConfig.requiresOpenaiAuthMode = normalizedRequiresOpenaiAuthMode;
  }

  return JSON.stringify(settingsConfig);
}
