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

/** Official Grok Build effort levels for config.toml (API primary set). */
export const GROK_REASONING_EFFORT_OPTIONS = ['low', 'medium', 'high'] as const;
export type GrokReasoningEffort = (typeof GROK_REASONING_EFFORT_OPTIONS)[number];

export interface BuildGrokSettingsConfigInput {
  category: GrokProviderCategory;
  apiKey: string;
  baseUrl: string;
  /** Upstream model ID for the default slot (not the local catalog key). */
  model: string;
  apiFormat?: GrokApiFormat;
  /** When set, force every projected [model.*] to this backend-search flag. */
  supportsBackendSearch?: boolean;
  /**
   * Channel-level reasoning effort.
   * - official → `defaultReasoningEffort` → `[models].default_reasoning_effort`
   * - custom → every catalog model `reasoningEffort` + `supportsReasoningEffort=true`
   * - empty/undefined → omit (follow CLI/model default)
   */
  reasoningEffort?: string;
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

function mapGrokApiFormatToBackend(apiFormat?: GrokApiFormat): string {
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
 * Project form-owned fields onto the fixed `custom` default slot.
 * - Form "model name" = upstream model ID → catalog entry model field only.
 * - Local key is always `custom` (user never edits it).
 * - Mapping displayName is owned by the mapping UI; do not rewrite it here.
 */
function projectCustomDefaultCatalogModels(
  catalogModels: GrokCatalogModel[],
  upstreamModel: string,
  normalizedBaseUrl: string,
  apiBackend: string,
  backendSearchFields: { supportsBackendSearch?: boolean },
  reasoningEffortFields: {
    supportsReasoningEffort?: boolean;
    reasoningEffort?: string;
  },
): GrokCatalogModel[] {
  let models = catalogModels.map((catalogModel) => {
    const next: GrokCatalogModel = {
      ...catalogModel,
      key: catalogModel.key?.trim() || catalogModel.model,
      ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
      apiBackend,
      ...backendSearchFields,
    };
    if (reasoningEffortFields.reasoningEffort) {
      next.supportsReasoningEffort = true;
      next.reasoningEffort = reasoningEffortFields.reasoningEffort;
    } else {
      delete next.supportsReasoningEffort;
      delete next.reasoningEffort;
    }
    return next;
  });

  if (models.length === 0) {
    return [{
      key: CUSTOM_GROK_MODEL_KEY,
      model: upstreamModel,
      displayName: upstreamModel,
      ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
      apiBackend,
      ...backendSearchFields,
      ...reasoningEffortFields,
    }];
  }

  let defaultIndex = models.findIndex(
    (catalogModel) => catalogModel.key === CUSTOM_GROK_MODEL_KEY,
  );
  if (defaultIndex < 0) {
    // Migrate legacy catalogs that used the upstream id (or other values) as key.
    defaultIndex = models.findIndex(
      (catalogModel) => (
        catalogModel.model === upstreamModel || catalogModel.key === upstreamModel
      ),
    );
    if (defaultIndex < 0 && models.length === 1) {
      defaultIndex = 0;
    }
  }

  if (defaultIndex < 0) {
    models = [
      ...models,
      {
        key: CUSTOM_GROK_MODEL_KEY,
        model: upstreamModel,
        displayName: upstreamModel,
        ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
        apiBackend,
        ...backendSearchFields,
        ...reasoningEffortFields,
      },
    ];
    return models;
  }

  const existing = models[defaultIndex];
  models[defaultIndex] = {
    ...existing,
    key: CUSTOM_GROK_MODEL_KEY,
    model: upstreamModel,
    // Keep mapping displayName untouched when the form model name changes.
  };
  return models;
}

export function buildGrokSettingsConfig({
  category,
  apiKey,
  baseUrl,
  model,
  apiFormat,
  supportsBackendSearch,
  reasoningEffort,
  config,
  catalogModels,
  auth,
}: BuildGrokSettingsConfigInput): string {
  const finalConfig = category === 'official'
    ? normalizeGrokConfigForOfficialMode(config)
    : config.trim();
  const normalizedApiKey = apiKey.trim();
  const normalizedBaseUrl = baseUrl.trim();
  // Form model name = upstream model ID. Official and custom both fall back to the
  // current Grok default when the field is left empty.
  const normalizedUpstreamModel = model.trim() || DEFAULT_GROK_MODEL;
  // Form-level API format is the provider protocol source of truth. Model mapping
  // UI does not edit per-model apiBackend, so always project the selected format.
  // Keeping a previous "responses" value when the form is "chat" left live config
  // with api_backend = "responses" after apply.
  const apiBackend = mapGrokApiFormatToBackend(apiFormat);
  const backendSearchFields = typeof supportsBackendSearch === 'boolean'
    ? { supportsBackendSearch }
    : {};
  const normalizedReasoningEffort = normalizeGrokReasoningEffort(reasoningEffort);
  const reasoningEffortFields = normalizedReasoningEffort
    ? {
        supportsReasoningEffort: true as const,
        reasoningEffort: normalizedReasoningEffort,
      }
    : {};
  let normalizedCatalogModels = normalizeGrokCatalogModels(catalogModels);

  if (category === 'custom') {
    // Form-level Base URL is the channel SoT (model-mapping UI cannot edit per-model
    // baseUrl). Always overwrite catalog baseUrl when the form value is non-empty —
    // same rule as apiBackend / supportsBackendSearch. Keeping a previous catalog
    // baseUrl left live [model.<key>].base_url stale after the user edited Base URL
    // and saved (issue #256).
    //
    // Local catalog key for the default slot is fixed to `custom`. Form "model name"
    // only updates the upstream model field on that slot; mapping displayName stays
    // under mapping-UI control.
    //
    // Channel-level reasoning effort mirrors backend search: when set, overwrite every
    // catalog model; when unset, strip so apply omits official reasoning_effort fields.
    normalizedCatalogModels = projectCustomDefaultCatalogModels(
      normalizedCatalogModels,
      normalizedUpstreamModel,
      normalizedBaseUrl,
      apiBackend,
      backendSearchFields,
      reasoningEffortFields,
    );
  }

  const finalAuth = { ...auth };
  if (category === 'custom' && normalizedApiKey) {
    finalAuth.API_KEY = normalizedApiKey;
  } else {
    delete finalAuth.API_KEY;
  }

  // Custom providers always select the fixed local key. Official stores the model id
  // directly as defaultModelKey (no modelCatalog).
  const defaultModelKey = category === 'custom'
    ? CUSTOM_GROK_MODEL_KEY
    : normalizedUpstreamModel;

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
  const catalogReasoningEffort = normalizeGrokReasoningEffort(
    resolveGrokCatalogReasoningEffort(seededCatalogModels),
  );
  const reasoningEffortFields = catalogReasoningEffort
    ? {
        supportsReasoningEffort: true as const,
        reasoningEffort: catalogReasoningEffort,
      }
    : {};
  // Built-in endpoints lock API format, but Base URL stays editable. Prefer the
  // form-level baseUrl already projected by buildGrokSettingsConfig (issue #256).
  const formBaseUrl = extractGrokSettingsBaseUrl(parsed)?.trim() || '';
  const normalizedBaseUrl = formBaseUrl || endpointBaseUrl.trim();
  // Upstream model comes from the form-built custom slot, not defaultModelKey (always "custom").
  const customEntry = seededCatalogModels.find(
    (catalogModel) => catalogModel.key?.trim() === CUSTOM_GROK_MODEL_KEY,
  ) || seededCatalogModels[0];
  const upstreamModel = customEntry?.model?.trim()
    || endpointModel?.trim()
    || DEFAULT_GROK_MODEL;

  const normalizedCatalogModels = projectCustomDefaultCatalogModels(
    seededCatalogModels,
    upstreamModel,
    normalizedBaseUrl,
    apiBackend,
    backendSearchFields,
    reasoningEffortFields,
  );

  return JSON.stringify({
    ...parsed,
    defaultModelKey: CUSTOM_GROK_MODEL_KEY,
    modelCatalog: {
      models: normalizedCatalogModels,
    },
  } satisfies GrokSettingsConfig);
}
