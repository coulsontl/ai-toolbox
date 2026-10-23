/**
 * Pure form logic for the Oh My Pi provider editor.
 *
 * These lived inline in `OhMyPiPage` and were only exercised by scraping the
 * page source. The modal owns the fields now, so the decisions that used to be
 * testable only through the page — which endpoint the form auto-fills, when it
 * stops owning it, and what the saved provider record looks like — live here.
 */
import type { OmpRuntimeProviderView } from '@/types/ohMyPi';
import { getOmpApiBaseUrlUpdate } from './ompApiOptions.ts';
import {
  asRecord,
  createDefaultProviderConfig,
  getStringField,
  isRecordEmpty,
  setOptionalStringField,
} from './ompProviderConfig.ts';

export interface OmpProviderFormValues {
  providerKey: string;
  displayName?: string;
  api?: string;
  baseUrl?: string;
  providerApiKey?: string;
  authHeader?: boolean;
}

export interface OmpProviderFormJsonBlocks {
  headers: Record<string, unknown>;
  compat: Record<string, unknown>;
  modelOverrides: Record<string, unknown>;
}

export interface OmpProviderFormInitialState {
  values: OmpProviderFormValues;
  jsonBlocks: OmpProviderFormJsonBlocks;
  /**
   * The endpoint the form is allowed to replace on the next protocol switch.
   * `undefined` means "the user owns the endpoint"; `''` means "the form owns
   * it and there is no automatic endpoint yet".
   */
  automaticBaseUrl: string | undefined;
}

/** Pre-fill the editor from a provider (edit), a source provider (copy), or nothing (add). */
export const resolveOmpProviderFormInitialState = (
  provider?: OmpRuntimeProviderView,
  copy = false,
): OmpProviderFormInitialState => {
  const isCopy = copy && Boolean(provider);
  const isExistingProviderEdit = Boolean(provider) && !isCopy;
  const providerConfig = provider?.modelsProvider
    ? asRecord(provider.modelsProvider)
    : isExistingProviderEdit
      ? {}
      : createDefaultProviderConfig();

  return {
    values: {
      providerKey: (isCopy && provider ? `${provider.providerKey}_copy` : provider?.providerKey) ?? '',
      displayName: getStringField(providerConfig, 'name'),
      api: getStringField(providerConfig, 'api') || undefined,
      baseUrl: getStringField(providerConfig, 'baseUrl'),
      providerApiKey: getStringField(providerConfig, 'apiKey'),
      authHeader: typeof providerConfig.authHeader === 'boolean'
        ? providerConfig.authHeader
        : undefined,
    },
    jsonBlocks: {
      headers: asRecord(providerConfig.headers),
      compat: asRecord(providerConfig.compat),
      modelOverrides: asRecord(providerConfig.modelOverrides),
    },
    // An edit or a copy starts from an endpoint the user already accepted, so
    // the form must not replace it; a fresh add leaves the form owning it.
    automaticBaseUrl: provider ? undefined : '',
  };
};

/**
 * Apply a protocol switch to the endpoint.
 *
 * The automatic endpoint may be replaced only while it still equals what the
 * form last wrote; once the user touches the field, or clears it, a later
 * protocol switch must leave it alone.
 */
export const resolveOmpAutomaticBaseUrlChange = (
  apiValue: string | undefined,
  currentBaseUrl: unknown,
  automaticBaseUrl: string | undefined,
): { automaticBaseUrl: string | undefined; baseUrl?: string } => {
  const nextAutomaticBaseUrl = getOmpApiBaseUrlUpdate(apiValue, currentBaseUrl, automaticBaseUrl);
  return {
    automaticBaseUrl: nextAutomaticBaseUrl,
    baseUrl: nextAutomaticBaseUrl,
  };
};

/** The provider record the page writes to `models.yml` from the form values. */
export const buildOmpProviderConfigFromForm = (
  base: Record<string, unknown>,
  values: OmpProviderFormValues,
  jsonBlocks: OmpProviderFormJsonBlocks,
): Record<string, unknown> => {
  const next = { ...base };
  setOptionalStringField(next, 'name', values.displayName);
  setOptionalStringField(next, 'api', values.api);
  setOptionalStringField(next, 'baseUrl', values.baseUrl);
  setOptionalStringField(next, 'apiKey', values.providerApiKey);
  if (values.authHeader === true) {
    next.authHeader = true;
  } else {
    delete next.authHeader;
  }
  if (isRecordEmpty(jsonBlocks.headers)) {
    delete next.headers;
  } else {
    next.headers = jsonBlocks.headers;
  }
  if (isRecordEmpty(jsonBlocks.compat)) {
    delete next.compat;
  } else {
    next.compat = jsonBlocks.compat;
  }
  if (isRecordEmpty(jsonBlocks.modelOverrides)) {
    delete next.modelOverrides;
  } else {
    next.modelOverrides = jsonBlocks.modelOverrides;
  }
  return next;
};

/**
 * Why this key cannot be saved through the API-key form, if it cannot.
 *
 * `openai-codex` is an OAuth subscription route: it is managed by the account
 * block, and adding it here would create a second, unauthenticated route. An
 * existing `models.yml` override of it stays editable (that is how a YAML-only
 * override is tuned).
 */
export const ompProviderKeySaveError = (
  providerKey: string,
  sourceProvider?: OmpRuntimeProviderView,
): 'subscriptionUseLoginGuidance' | null => (
  providerKey === 'openai-codex' && !sourceProvider?.sources.includes('models_yml')
    ? 'subscriptionUseLoginGuidance'
    : null
);
