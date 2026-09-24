import type {
  GrokApiFormat,
  GrokCatalogModel,
  GrokProvider,
  GrokSettingsConfig,
} from '../../../../types/grok';
import { extractGrokSettingsApiBackend } from '../../../../utils/grokConfigUtils';
import { normalizeGrokCatalogModels } from './grokCatalogModels';
import {
  CUSTOM_GROK_MODEL_KEY,
  normalizeGrokReasoningEffort,
} from './grokSettingsConfig';

/** Selectable effort levels for Grok Build model catalog / config.toml. */
export const GROK_MODEL_REASONING_EFFORT_OPTIONS = [
  'none',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
] as const;

/** OpenCode preset group for official xAI models; always in Grok primary strip. */
export const GROK_XAI_PRESET_NPM = '@ai-sdk/xai';

const DEFAULT_GROK_API_FORMAT: GrokApiFormat = 'openai_chat';

/** Normalize free-form API format tokens to Grok provider form values. */
export function normalizeGrokApiFormat(value?: string | null): GrokApiFormat {
  if (
    value === 'openai_chat'
    || value === 'openai_responses'
    || value === 'anthropic_messages'
  ) {
    return value;
  }
  return DEFAULT_GROK_API_FORMAT;
}

/** Map a live/catalog `apiBackend` string to the channel-level API format. */
export function mapGrokApiBackendToApiFormat(
  apiBackend?: string | null,
): GrokApiFormat | undefined {
  const normalized = apiBackend?.trim().toLowerCase();
  if (!normalized) {
    return undefined;
  }
  if (normalized === 'responses' || normalized === 'openai_responses') {
    return 'openai_responses';
  }
  if (
    normalized === 'anthropic'
    || normalized === 'anthropic_messages'
    || normalized === 'messages'
  ) {
    return 'anthropic_messages';
  }
  if (
    normalized === 'chat'
    || normalized === 'chat_completions'
    || normalized === 'openai_chat'
  ) {
    return 'openai_chat';
  }
  return undefined;
}

/**
 * Resolve the channel API format used for model-preset grouping.
 * Prefer meta, then settingsConfig.apiFormat, then catalog apiBackend.
 */
export function resolveGrokProviderApiFormat(
  provider: Pick<GrokProvider, 'meta' | 'settingsConfig'>,
): GrokApiFormat {
  if (typeof provider.meta?.apiFormat === 'string' && provider.meta.apiFormat.trim()) {
    return normalizeGrokApiFormat(provider.meta.apiFormat);
  }

  const settings = parseGrokProviderSettings(provider) as GrokSettingsConfig & {
    apiFormat?: unknown;
    api_format?: unknown;
  };
  if (typeof settings.apiFormat === 'string' && settings.apiFormat.trim()) {
    return normalizeGrokApiFormat(settings.apiFormat);
  }
  if (typeof settings.api_format === 'string' && settings.api_format.trim()) {
    return normalizeGrokApiFormat(settings.api_format);
  }

  const fromDefaultBackend = mapGrokApiBackendToApiFormat(
    extractGrokSettingsApiBackend(settings),
  );
  if (fromDefaultBackend) {
    return fromDefaultBackend;
  }

  for (const model of settings.modelCatalog?.models || []) {
    const fromModel = mapGrokApiBackendToApiFormat(model.apiBackend);
    if (fromModel) {
      return fromModel;
    }
  }

  return DEFAULT_GROK_API_FORMAT;
}

/**
 * Primary OpenCode preset npm groups for the model editor:
 * always xAI, plus the group matching the current channel protocol.
 *
 * Chat / Responses / Anthropic are three different channel types — do not merge
 * Chat and Responses into one primary strip.
 *
 * - openai_chat → openai-compatible (third-party Chat Completions endpoints)
 * - openai_responses → openai (native OpenAI / Responses-oriented catalog)
 * - anthropic_messages → anthropic
 */
export function getGrokPrimaryPresetNpmTypes(
  apiFormat?: GrokApiFormat | string | null,
): string[] {
  const format = normalizeGrokApiFormat(apiFormat);
  if (format === 'anthropic_messages') {
    return [GROK_XAI_PRESET_NPM, '@ai-sdk/anthropic'];
  }
  if (format === 'openai_responses') {
    return [GROK_XAI_PRESET_NPM, '@ai-sdk/openai'];
  }
  return [GROK_XAI_PRESET_NPM, '@ai-sdk/openai-compatible'];
}

const GROK_MODEL_REASONING_EFFORT_SET = new Set<string>(GROK_MODEL_REASONING_EFFORT_OPTIONS);

/**
 * Normalize a free-form preset effort token to a Grok catalog value.
 * `max` is treated as the CLI/UX alias of `xhigh` (same as Grok Build source).
 */
export function normalizeGrokCatalogEffortToken(raw: string): string | undefined {
  const token = raw.trim().toLowerCase();
  if (!token) {
    return undefined;
  }
  if (token === 'max') {
    return 'xhigh';
  }
  return GROK_MODEL_REASONING_EFFORT_SET.has(token) ? token : undefined;
}

type PresetVariantLike = {
  reasoningEffort?: unknown;
  effort?: unknown;
  thinkingLevel?: unknown;
  thinkingConfig?: {
    thinkingLevel?: unknown;
  };
};

/**
 * Extract supported reasoning-effort levels from a preset model's `variants` map.
 * Collects variant keys and nested effort fields (`reasoningEffort` / `effort` /
 * `thinkingLevel`), normalizes to Grok tokens, and returns a unique ordered list.
 */
export function extractReasoningEffortsFromPresetVariants(
  variants: Record<string, unknown> | undefined | null,
): string[] {
  if (!variants || typeof variants !== 'object' || Array.isArray(variants)) {
    return [];
  }

  const collected: string[] = [];
  const pushToken = (raw: unknown) => {
    if (typeof raw !== 'string') {
      return;
    }
    const normalized = normalizeGrokCatalogEffortToken(raw);
    if (normalized) {
      collected.push(normalized);
    }
  };

  Object.entries(variants).forEach(([variantKey, variantValue]) => {
    pushToken(variantKey);
    if (!variantValue || typeof variantValue !== 'object' || Array.isArray(variantValue)) {
      return;
    }
    const variant = variantValue as PresetVariantLike;
    pushToken(variant.reasoningEffort);
    pushToken(variant.effort);
    pushToken(variant.thinkingLevel);
    pushToken(variant.thinkingConfig?.thinkingLevel);
  });

  // Stable order: follow GROK_MODEL_REASONING_EFFORT_OPTIONS, then any extras.
  const unique = Array.from(new Set(collected));
  const ordered = GROK_MODEL_REASONING_EFFORT_OPTIONS.filter((level) => unique.includes(level));
  unique.forEach((level) => {
    if (!ordered.includes(level as typeof GROK_MODEL_REASONING_EFFORT_OPTIONS[number])) {
      ordered.push(level as typeof GROK_MODEL_REASONING_EFFORT_OPTIONS[number]);
    }
  });
  return ordered;
}

/** Prefer high as the default selected effort when present; otherwise first in list. */
export function pickDefaultReasoningEffort(efforts: string[]): string | undefined {
  if (efforts.includes('high')) {
    return 'high';
  }
  return efforts[0];
}

export function parseGrokProviderSettings(provider: Pick<GrokProvider, 'settingsConfig'>): GrokSettingsConfig {
  try {
    const parsed = JSON.parse(provider.settingsConfig || '{}') as unknown;
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return {};
    }
    return parsed as GrokSettingsConfig;
  } catch {
    return {};
  }
}

/**
 * Migrate legacy single-slot catalogs that used fixed key `custom`.
 * New product rule: local catalog key always equals upstream model id.
 * Existing free multi-model keys are left untouched.
 */
export function migrateLegacyGrokCatalogModels(
  models: GrokCatalogModel[],
  defaultModelKey?: string,
): { models: GrokCatalogModel[]; defaultModelKey?: string } {
  const normalized = normalizeGrokCatalogModels(models);
  if (normalized.length === 0) {
    return { models: normalized, defaultModelKey: defaultModelKey?.trim() || undefined };
  }

  let nextDefault = defaultModelKey?.trim() || undefined;
  const usedKeys = new Set<string>();
  const migrated = normalized.map((model) => {
    const previousKey = model.key?.trim() || model.model;
    const upstream = model.model?.trim() || previousKey;
    // Only rewrite the historical fixed slot. Real multi-model keys stay as-is.
    const nextKey = previousKey === CUSTOM_GROK_MODEL_KEY
      ? suggestGrokModelKeyFromUpstream(upstream)
      : previousKey;
    let uniqueKey = nextKey || upstream || CUSTOM_GROK_MODEL_KEY;
    if (usedKeys.has(uniqueKey)) {
      let suffix = 2;
      while (usedKeys.has(`${uniqueKey}-${suffix}`)) {
        suffix += 1;
      }
      uniqueKey = `${uniqueKey}-${suffix}`;
    }
    usedKeys.add(uniqueKey);
    if (nextDefault && nextDefault === previousKey) {
      nextDefault = uniqueKey;
    }
    return {
      ...model,
      key: uniqueKey,
      model: upstream || uniqueKey,
    };
  });

  if (!nextDefault || !migrated.some((model) => model.key === nextDefault)) {
    nextDefault = migrated[0]?.key || undefined;
  }

  return {
    models: normalizeGrokCatalogModels(migrated),
    defaultModelKey: nextDefault,
  };
}

export function getGrokProviderCatalogModels(provider: Pick<GrokProvider, 'settingsConfig'>): GrokCatalogModel[] {
  const settings = parseGrokProviderSettings(provider);
  const migrated = migrateLegacyGrokCatalogModels(
    settings.modelCatalog?.models || [],
    settings.defaultModelKey,
  );
  return migrated.models;
}

export function getGrokProviderDefaultModelKey(
  provider: Pick<GrokProvider, 'settingsConfig' | 'category'>,
): string | undefined {
  const settings = parseGrokProviderSettings(provider);
  const migrated = migrateLegacyGrokCatalogModels(
    settings.modelCatalog?.models || [],
    settings.defaultModelKey,
  );
  return migrated.defaultModelKey;
}

export function buildGrokProviderSettingsWithModels(
  provider: Pick<GrokProvider, 'settingsConfig' | 'category'>,
  models: GrokCatalogModel[],
  defaultModelKey?: string,
): string {
  const settings = parseGrokProviderSettings(provider);
  const migrated = migrateLegacyGrokCatalogModels(models, defaultModelKey || settings.defaultModelKey);
  const normalizedModels = migrated.models;
  let nextDefault = migrated.defaultModelKey || '';
  if (nextDefault && !normalizedModels.some((model) => model.key === nextDefault)) {
    nextDefault = normalizedModels[0]?.key || '';
  }
  if (!nextDefault && normalizedModels.length > 0) {
    nextDefault = normalizedModels[0].key || normalizedModels[0].model;
  }

  const next: GrokSettingsConfig = {
    ...settings,
    ...(nextDefault ? { defaultModelKey: nextDefault } : {}),
  };

  // Channel Base URL is a catalog-independent channel property: every model-list
  // mutation (delete / batch delete / remove-invalid / add / edit / fetch / set
  // default) goes through here, so it must never drop the URL even when the last
  // catalog model carrying it is removed (issue #391). Per-model entries are always
  // left untouched.
  //
  // Records written before the channel field existed only carry per-model URLs, so
  // they are promoted on the first mutation — but only when the catalog states one
  // unambiguous address. Mixed per-model URLs are third-party data this app does not
  // own (it has no per-model URL editor), so they are neither collapsed nor frozen
  // into the channel field; the surviving entries keep their own addresses.
  if (provider.category === 'official') {
    delete next.baseUrl;
    delete next.modelCatalog;
    if (nextDefault) {
      next.defaultModelKey = nextDefault;
    }
  } else {
    const existingChannelBaseUrl = typeof settings.baseUrl === 'string' ? settings.baseUrl.trim() : '';
    const previousEntryBaseUrls = (settings.modelCatalog?.models || [])
      .map((model) => model.baseUrl?.trim())
      .filter((value): value is string => Boolean(value));
    const promotedChannelBaseUrl = previousEntryBaseUrls[0];
    if (existingChannelBaseUrl) {
      next.baseUrl = existingChannelBaseUrl;
    } else if (
      promotedChannelBaseUrl
      && previousEntryBaseUrls.every((value) => value === promotedChannelBaseUrl)
    ) {
      next.baseUrl = promotedChannelBaseUrl;
    }
    next.modelCatalog = { models: normalizedModels };
    if (nextDefault) {
      next.defaultModelKey = nextDefault;
    } else {
      delete next.defaultModelKey;
    }
    // Custom multi-model: channel-level defaultReasoningEffort is not the SoT.
    delete next.defaultReasoningEffort;
  }

  return JSON.stringify(next);
}

export function upsertGrokCatalogModel(
  models: GrokCatalogModel[],
  nextModel: GrokCatalogModel,
  previousKey?: string,
): GrokCatalogModel[] {
  const normalizedNext = normalizeGrokCatalogModels([nextModel])[0];
  if (!normalizedNext) {
    return models;
  }
  const withoutPrevious = models.filter((model) => {
    const key = model.key?.trim() || model.model;
    if (previousKey && key === previousKey) {
      return false;
    }
    return key !== normalizedNext.key;
  });
  return normalizeGrokCatalogModels([...withoutPrevious, normalizedNext]);
}

export function removeGrokCatalogModel(models: GrokCatalogModel[], modelKey: string): GrokCatalogModel[] {
  return models.filter((model) => (model.key?.trim() || model.model) !== modelKey);
}

/** Values for the Grok-adapted model editor (OpenCode-like shell, Grok fields). */
export interface GrokModelFormValues {
  key: string;
  model: string;
  displayName?: string;
  contextWindow?: number;
  reasoningEfforts?: string[];
  reasoningEffort?: string;
  supportsBackendSearch?: boolean;
}

export interface SharedModelFormLike {
  id: string;
  name: string;
  contextLimit?: number;
  outputLimit?: number;
  variants?: string;
  modalities?: string;
  reasoning?: boolean;
  attachment?: boolean;
  tool_call?: boolean;
  temperature?: boolean;
}

/** Convert catalog model → shared ModelFormModal initial values (OpenCode-compatible). */
export function toSharedModelFormValues(model: GrokCatalogModel): Partial<SharedModelFormLike> {
  const key = model.key?.trim() || model.model;
  const efforts = Array.isArray(model.reasoningEfforts)
    ? model.reasoningEfforts.filter(Boolean)
    : [];
  // Prefer an explicit menu; if only a scalar effort exists, still seed one variant
  // so the OpenCode-style variants editor is not empty on edit.
  const seedEfforts = efforts.length > 0
    ? efforts
    : (model.reasoningEffort?.trim() ? [model.reasoningEffort.trim()] : []);
  let variants: string | undefined;
  if (seedEfforts.length > 0) {
    const variantMap: Record<string, { reasoningEffort: string }> = {};
    seedEfforts.forEach((effort) => {
      variantMap[effort] = { reasoningEffort: effort };
    });
    variants = JSON.stringify(variantMap);
  }

  return {
    id: key,
    name: model.displayName || model.model || key,
    contextLimit: typeof model.contextWindow === 'number'
      ? model.contextWindow
      : (model.contextWindow ? Number(model.contextWindow) || undefined : undefined),
    variants,
    reasoning: model.supportsReasoningEffort === false
      ? false
      : (model.supportsReasoningEffort === true || seedEfforts.length > 0),
    attachment: model.attachment === true || model.supportsImage === true,
    modalities: model.modalities
      ? JSON.stringify(model.modalities)
      : undefined,
  };
}

/**
 * Convert shared ModelFormModal values → Grok catalog model.
 * Variants shaped like OpenCode/xAI presets:
 * `{ "low": { "reasoningEffort": "low" }, "high": { "reasoningEffort": "high" } }`
 * become `reasoningEfforts` (+ optional default `reasoningEffort`).
 */
export function fromSharedModelFormValues(
  values: SharedModelFormLike,
  existing?: GrokCatalogModel,
  channelDefaults?: {
    baseUrl?: string;
    apiBackend?: string;
  },
  previousDefaultEffort?: string,
): GrokCatalogModel {
  const key = values.id.trim();
  const model = existing?.model?.trim() || key;
  const displayName = values.name?.trim() || model;

  const efforts: string[] = [];
  if (values.variants?.trim()) {
    try {
      const parsed = JSON.parse(values.variants) as Record<string, unknown>;
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        Object.entries(parsed).forEach(([variantKey, variantValue]) => {
          if (variantValue && typeof variantValue === 'object' && !Array.isArray(variantValue)) {
            const effort = (variantValue as { reasoningEffort?: unknown }).reasoningEffort;
            if (typeof effort === 'string' && effort.trim()) {
              efforts.push(effort.trim());
              return;
            }
          }
          // Bare key as effort token (e.g. { "high": {} }).
          if (GROK_MODEL_REASONING_EFFORT_OPTIONS.includes(
            variantKey as (typeof GROK_MODEL_REASONING_EFFORT_OPTIONS)[number],
          )) {
            efforts.push(variantKey);
          }
        });
      }
    } catch {
      // ignore invalid variants; leave efforts empty
    }
  }

  const uniqueEfforts = Array.from(new Set(efforts));
  const preferredDefault = normalizeGrokReasoningEffort(previousDefaultEffort)
    || normalizeGrokReasoningEffort(existing?.reasoningEffort)
    || existing?.reasoningEffort?.trim();
  const selectedEffort = preferredDefault && uniqueEfforts.includes(preferredDefault)
    ? preferredDefault
    : uniqueEfforts[0];

  let modalities = existing?.modalities;
  if (values.modalities?.trim()) {
    try {
      const parsed = JSON.parse(values.modalities) as GrokCatalogModel['modalities'];
      if (parsed && typeof parsed === 'object') {
        modalities = parsed;
      }
    } catch {
      // keep existing
    }
  }

  const supportsReasoning = values.reasoning === true || uniqueEfforts.length > 0;

  return {
    ...existing,
    key,
    model,
    displayName,
    ...(typeof values.contextLimit === 'number' && values.contextLimit > 0
      ? { contextWindow: values.contextLimit }
      : {}),
    ...(channelDefaults?.baseUrl && !existing?.baseUrl
      ? { baseUrl: channelDefaults.baseUrl }
      : {}),
    ...(channelDefaults?.apiBackend && !existing?.apiBackend
      ? { apiBackend: channelDefaults.apiBackend }
      : {}),
    ...(modalities ? { modalities } : {}),
    ...(typeof values.attachment === 'boolean'
      ? { attachment: values.attachment, supportsImage: values.attachment }
      : {}),
    ...(supportsReasoning
      ? {
          supportsReasoningEffort: true,
          ...(uniqueEfforts.length > 0
            ? {
                reasoningEfforts: uniqueEfforts,
                ...(selectedEffort ? { reasoningEffort: selectedEffort } : {}),
              }
            : {
                ...(existing?.reasoningEffort
                  ? { reasoningEffort: existing.reasoningEffort }
                  : {}),
              }),
        }
      : {
          supportsReasoningEffort: false,
          reasoningEfforts: undefined,
          reasoningEffort: undefined,
        }),
  };
}

export function toGrokModelFormValues(model: GrokCatalogModel): GrokModelFormValues {
  const key = model.key?.trim() || model.model;
  const efforts = Array.isArray(model.reasoningEfforts)
    ? model.reasoningEfforts.filter(Boolean)
    : [];
  return {
    key,
    model: model.model,
    displayName: model.displayName || model.model,
    contextWindow: typeof model.contextWindow === 'number'
      ? model.contextWindow
      : (model.contextWindow ? Number(model.contextWindow) || undefined : undefined),
    reasoningEfforts: efforts,
    reasoningEffort: normalizeGrokReasoningEffort(model.reasoningEffort) || model.reasoningEffort,
    supportsBackendSearch: model.supportsBackendSearch === true,
  };
}

export function fromGrokModelFormValues(
  values: GrokModelFormValues,
  existing?: GrokCatalogModel,
  channelDefaults?: {
    baseUrl?: string;
    apiBackend?: string;
  },
): GrokCatalogModel {
  // Product rule: local catalog key always equals upstream model id.
  const model = (values.model.trim() || values.key.trim());
  const key = model;
  const efforts = (values.reasoningEfforts || [])
    .map((effort) => effort.trim())
    .filter(Boolean);
  const preferred = normalizeGrokReasoningEffort(values.reasoningEffort)
    || values.reasoningEffort?.trim();
  const selectedEffort = preferred && efforts.includes(preferred)
    ? preferred
    : efforts[0];

  return {
    ...existing,
    key,
    model,
    displayName: values.displayName?.trim() || model,
    ...(typeof values.contextWindow === 'number' && values.contextWindow > 0
      ? { contextWindow: values.contextWindow }
      : {}),
    ...(channelDefaults?.baseUrl && !existing?.baseUrl
      ? { baseUrl: channelDefaults.baseUrl }
      : {}),
    ...(channelDefaults?.apiBackend && !existing?.apiBackend
      ? { apiBackend: channelDefaults.apiBackend }
      : {}),
    ...(typeof values.supportsBackendSearch === 'boolean'
      ? { supportsBackendSearch: values.supportsBackendSearch }
      : {}),
    ...(efforts.length > 0
      ? {
          supportsReasoningEffort: true,
          reasoningEfforts: efforts,
          ...(selectedEffort ? { reasoningEffort: selectedEffort } : {}),
        }
      : {
          supportsReasoningEffort: undefined,
          reasoningEfforts: undefined,
          reasoningEffort: undefined,
        }),
  };
}

/** Prefer a stable non-custom key when migrating legacy single-slot providers. */
export function suggestGrokModelKeyFromUpstream(upstreamModel: string): string {
  const model = upstreamModel.trim();
  if (!model || model === CUSTOM_GROK_MODEL_KEY) {
    return CUSTOM_GROK_MODEL_KEY;
  }
  return model;
}
