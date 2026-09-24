import type {
  GrokApiFormat,
  GrokCatalogModel,
  GrokProviderCategory,
  GrokSettingsConfig,
} from '../../../../types/grok';
import {
  extractGrokSettingsBaseUrl,
  normalizeGrokConfigForOfficialMode,
} from '../../../../utils/grokConfigUtils';
import { isJsonObject } from '../../../../utils/json';
import { normalizeGrokCatalogModels } from './grokCatalogModels';

export const DEFAULT_GROK_MODEL = 'grok-4.5';
/** Fixed local catalog key for custom providers. Users never edit this. */
export const CUSTOM_GROK_MODEL_KEY = 'custom';

/** Official / default effort levels commonly used in Grok Build menus. */
export const GROK_REASONING_EFFORT_OPTIONS = ['low', 'medium', 'high', 'xhigh'] as const;
export type GrokReasoningEffort = (typeof GROK_REASONING_EFFORT_OPTIONS)[number];

export interface BuildGrokSettingsConfigInput {
  category: GrokProviderCategory;
  apiKey: string;
  baseUrl: string;
  /** Upstream model ID for bootstrap / official default (not always the local catalog key). */
  model: string;
  apiFormat?: GrokApiFormat;
  /** When set, force every projected [model.*] to this backend-search flag. */
  supportsBackendSearch?: boolean;
  /**
   * Channel-level default reasoning effort.
   * - official → `defaultReasoningEffort` → `[models].default_reasoning_effort`
   * - custom → only stamps the default catalog model when that model has no menu yet
   * - empty/undefined → omit (follow CLI/model default)
   */
  reasoningEffort?: string;
  /** Preferred default catalog key when multi-model catalog is managed outside the channel form. */
  defaultModelKey?: string;
  config: string;
  catalogModels: GrokCatalogModel[];
  auth: Record<string, unknown>;
}

export interface ApplyGrokEndpointSettingsConfigInput {
  settingsConfig: string;
  apiFormat: GrokApiFormat;
  endpointBaseUrl: string;
  endpointModel?: string;
  endpointCatalogModels: GrokCatalogModel[];
}

/** Channel API format → live `api_backend` token. Single source for both directions
 * (the reverse mapping lives in `grokProviderModels.mapGrokApiBackendToApiFormat`). */
export function mapGrokApiFormatToBackend(apiFormat?: GrokApiFormat): string {
  if (apiFormat === 'openai_responses') {
    return 'responses';
  }
  if (apiFormat === 'anthropic_messages') {
    return 'messages';
  }
  return 'chat_completions';
}

export function resolveGrokCatalogBackendSearchFlag(
  models: GrokCatalogModel[] | undefined,
): boolean | undefined {
  if (!Array.isArray(models) || models.length === 0) {
    return undefined;
  }
  if (models.every((model) => model.supportsBackendSearch === true)) {
    return true;
  }
  if (models.every((model) => model.supportsBackendSearch === false)) {
    return false;
  }
  return undefined;
}

export function normalizeGrokReasoningEffort(value: unknown): GrokReasoningEffort | undefined {
  if (typeof value !== 'string') {
    return undefined;
  }
  const normalized = value.trim().toLowerCase();
  return (GROK_REASONING_EFFORT_OPTIONS as readonly string[]).includes(normalized)
    ? normalized as GrokReasoningEffort
    : undefined;
}

/** When every catalog model shares the same non-empty reasoningEffort, return it. */
export function resolveGrokCatalogReasoningEffort(
  models: GrokCatalogModel[] | undefined,
): string | undefined {
  if (!Array.isArray(models) || models.length === 0) {
    return undefined;
  }
  const efforts = models.map((model) => model.reasoningEffort?.trim() || '');
  const first = efforts[0];
  if (!first) {
    return undefined;
  }
  return efforts.every((effort) => effort === first) ? first : undefined;
}

export function parseGrokSettingsConfig(rawConfig: string | undefined): GrokSettingsConfig {
  if (!rawConfig?.trim()) return {};

  try {
    const parsedConfig = JSON.parse(rawConfig) as unknown;
    return isJsonObject(parsedConfig) ? parsedConfig as GrokSettingsConfig : {};
  } catch (error) {
    console.error('Failed to parse Grok settings config:', error);
    return {};
  }
}

/**
 * Project channel-owned fields onto the model catalog without collapsing multi-model keys.
 * - Base URL / API format / backend search remain channel-level and apply to every model.
 * - Catalog keys stay free (no longer forced to `custom`).
 * - Empty catalog bootstraps one model from the form upstream model id.
 */
function projectCustomChannelOntoCatalogModels(
  catalogModels: GrokCatalogModel[],
  upstreamModel: string,
  normalizedBaseUrl: string,
  apiBackend: string,
  backendSearchFields: { supportsBackendSearch?: boolean },
): GrokCatalogModel[] {
  if (catalogModels.length === 0) {
    return [{
      key: upstreamModel || CUSTOM_GROK_MODEL_KEY,
      model: upstreamModel || DEFAULT_GROK_MODEL,
      displayName: upstreamModel || DEFAULT_GROK_MODEL,
      ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
      apiBackend,
      ...backendSearchFields,
    }];
  }

  return catalogModels.map((catalogModel) => ({
    ...catalogModel,
    key: catalogModel.key?.trim() || catalogModel.model,
    ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
    apiBackend,
    ...backendSearchFields,
  }));
}

function resolveCustomDefaultModelKey(
  models: GrokCatalogModel[],
  preferredKey?: string,
): string {
  const preferred = preferredKey?.trim();
  if (preferred && models.some((model) => model.key === preferred || model.model === preferred)) {
    const matched = models.find((model) => model.key === preferred || model.model === preferred);
    return matched?.key?.trim() || matched?.model || preferred;
  }
  // Legacy single-slot catalogs used fixed key "custom". Prefer that slot's
  // upstream model id as the default pointer when present.
  const legacyCustom = models.find((model) => model.key === CUSTOM_GROK_MODEL_KEY);
  if (legacyCustom) {
    return legacyCustom.key?.trim() || legacyCustom.model || CUSTOM_GROK_MODEL_KEY;
  }
  return models[0]?.key?.trim() || models[0]?.model || CUSTOM_GROK_MODEL_KEY;
}

/**
 * Soft-migrate legacy fixed-key "custom" catalogs so UI/default pointers keep working
 * while still accepting old saved provider JSON without rewrite until next save.
 */
function softMigrateLegacyCustomCatalog(
  models: GrokCatalogModel[],
  preferredDefaultKey?: string,
): { models: GrokCatalogModel[]; defaultModelKey: string } {
  const normalized = normalizeGrokCatalogModels(models);
  if (normalized.length === 0) {
    const fallback = preferredDefaultKey?.trim() || CUSTOM_GROK_MODEL_KEY;
    return { models: normalized, defaultModelKey: fallback };
  }

  // If there is exactly one legacy "custom" slot, rewrite key to upstream model id.
  if (
    normalized.length === 1
    && (normalized[0].key === CUSTOM_GROK_MODEL_KEY || preferredDefaultKey === CUSTOM_GROK_MODEL_KEY)
  ) {
    const upstream = normalized[0].model?.trim() || CUSTOM_GROK_MODEL_KEY;
    const migrated = [{
      ...normalized[0],
      key: upstream,
      model: upstream,
      displayName: normalized[0].displayName || upstream,
    }];
    return {
      models: migrated,
      defaultModelKey: upstream,
    };
  }

  return {
    models: normalized,
    defaultModelKey: resolveCustomDefaultModelKey(normalized, preferredDefaultKey),
  };
}

export function buildGrokSettingsConfig({
  category,
  apiKey,
  baseUrl,
  model,
  apiFormat,
  supportsBackendSearch,
  reasoningEffort,
  defaultModelKey: preferredDefaultModelKey,
  config,
  catalogModels,
  auth,
}: BuildGrokSettingsConfigInput): string {
  const finalConfig = category === 'official'
    ? normalizeGrokConfigForOfficialMode(config)
    : config.trim();
  const normalizedApiKey = apiKey.trim();
  const normalizedBaseUrl = baseUrl.trim();
  // Form model name = upstream model ID. Official and empty custom catalogs fall back
  // to the current Grok default when the field is left empty.
  const normalizedUpstreamModel = model.trim() || DEFAULT_GROK_MODEL;
  // Form-level API format is the provider protocol source of truth. Always project the
  // selected format onto catalog models so live api_backend does not go stale.
  const apiBackend = mapGrokApiFormatToBackend(apiFormat);
  const backendSearchFields = typeof supportsBackendSearch === 'boolean'
    ? { supportsBackendSearch }
    : {};
  const normalizedReasoningEffort = normalizeGrokReasoningEffort(reasoningEffort);
  let normalizedCatalogModels = normalizeGrokCatalogModels(catalogModels);
  let defaultModelKey = category === 'custom'
    ? resolveCustomDefaultModelKey(normalizedCatalogModels, preferredDefaultModelKey)
    : normalizedUpstreamModel;

  if (category === 'custom') {
    // Soft-migrate historical fixed key "custom" when building settings for save/apply.
    const migrated = softMigrateLegacyCustomCatalog(
      normalizedCatalogModels,
      preferredDefaultModelKey || defaultModelKey,
    );
    normalizedCatalogModels = migrated.models;
    defaultModelKey = migrated.defaultModelKey;

    // Channel SoT for Base URL / API format / optional backend-search projection.
    // Multi-model keys and per-model reasoning menus are owned by the model list UI.
    normalizedCatalogModels = projectCustomChannelOntoCatalogModels(
      normalizedCatalogModels,
      normalizedUpstreamModel,
      normalizedBaseUrl,
      apiBackend,
      backendSearchFields,
    );
    defaultModelKey = resolveCustomDefaultModelKey(
      normalizedCatalogModels,
      defaultModelKey,
    );
  }

  const finalAuth = { ...auth };
  if (category === 'custom' && normalizedApiKey) {
    finalAuth.API_KEY = normalizedApiKey;
  } else {
    delete finalAuth.API_KEY;
  }

  // Optional channel-level effort only bootstraps the default model when that model
  // does not already own a reasoning menu (model list is the multi-model SoT).
  if (category === 'custom' && normalizedReasoningEffort) {
    normalizedCatalogModels = normalizedCatalogModels.map((catalogModel) => {
      if (catalogModel.key !== defaultModelKey) {
        return catalogModel;
      }
      if (Array.isArray(catalogModel.reasoningEfforts) && catalogModel.reasoningEfforts.length > 0) {
        return {
          ...catalogModel,
          supportsReasoningEffort: true,
          reasoningEffort: catalogModel.reasoningEfforts.includes(normalizedReasoningEffort)
            ? normalizedReasoningEffort
            : catalogModel.reasoningEffort,
        };
      }
      return {
        ...catalogModel,
        supportsReasoningEffort: true,
        reasoningEffort: normalizedReasoningEffort,
      };
    });
  }

  const settingsConfig: GrokSettingsConfig = {
    auth: finalAuth,
    config: finalConfig.trim(),
    defaultModelKey,
  };
  if (category === 'official' && normalizedReasoningEffort) {
    // Official has no modelCatalog; effort is the global default for [models].default.
    settingsConfig.defaultReasoningEffort = normalizedReasoningEffort;
  }
  if (category === 'custom') {
    // Channel-level Base URL SoT. The catalog projection above is what reaches the
    // live `[model.<key>].base_url`, but only this field survives model-list
    // mutations (issue #391). Official channels have no Base URL.
    if (normalizedBaseUrl) {
      settingsConfig.baseUrl = normalizedBaseUrl;
    }
    settingsConfig.modelCatalog = {
      models: normalizedCatalogModels,
    };
  }

  return JSON.stringify(settingsConfig);
}

export function applyGrokEndpointSettingsConfig({
  settingsConfig,
  apiFormat,
  endpointBaseUrl,
  endpointModel,
  endpointCatalogModels,
}: ApplyGrokEndpointSettingsConfigInput): string {
  const parsed = parseGrokSettingsConfig(settingsConfig);
  const apiBackend = mapGrokApiFormatToBackend(apiFormat);
  const currentCatalogModels = normalizeGrokCatalogModels(
    parsed.modelCatalog?.models || [],
  );
  // Prefer the already-built form catalog. Endpoint catalog is only a bootstrap
  // fallback when the form truly has no mappings.
  const seededCatalogModels = currentCatalogModels.length > 0
    ? currentCatalogModels
    : normalizeGrokCatalogModels(endpointCatalogModels);
  const backendSearchFlag = resolveGrokCatalogBackendSearchFlag(seededCatalogModels);
  const backendSearchFields = typeof backendSearchFlag === 'boolean'
    ? { supportsBackendSearch: backendSearchFlag }
    : {};
  // Built-in endpoints lock API format, but Base URL stays editable. Prefer the
  // form-level baseUrl already projected by buildGrokSettingsConfig (issue #256).
  const formBaseUrl = extractGrokSettingsBaseUrl(parsed)?.trim() || '';
  const normalizedBaseUrl = formBaseUrl || endpointBaseUrl.trim();
  const preferredDefaultKey = parsed.defaultModelKey?.trim();
  const customEntry = seededCatalogModels.find(
    (catalogModel) => catalogModel.key?.trim() === preferredDefaultKey,
  ) || seededCatalogModels.find(
    (catalogModel) => catalogModel.key?.trim() === CUSTOM_GROK_MODEL_KEY,
  ) || seededCatalogModels[0];
  const upstreamModel = customEntry?.model?.trim()
    || endpointModel?.trim()
    || DEFAULT_GROK_MODEL;

  const normalizedCatalogModels = projectCustomChannelOntoCatalogModels(
    seededCatalogModels,
    upstreamModel,
    normalizedBaseUrl,
    apiBackend,
    backendSearchFields,
  );
  const defaultModelKey = resolveCustomDefaultModelKey(
    normalizedCatalogModels,
    preferredDefaultKey,
  );

  return JSON.stringify({
    ...parsed,
    // Keep the channel-level SoT in sync with the endpoint finalization so built-in
    // channels also survive model-list mutations (issue #391).
    ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
    defaultModelKey,
    modelCatalog: {
      models: normalizedCatalogModels,
    },
  } satisfies GrokSettingsConfig);
}
