/**
 * Provider share-link generation for the `aitoolbox://` deep-link import flow.
 *
 * The share URL carries only the generic connection fields (name / category /
 * apiKey / baseUrl / model / homepage / notes / icon / iconColor) — never the
 * tool-specific `config` / `extra` blobs. The receiving side's import builders
 * (in `tauri/src/coding/deeplink/provider.rs`) rebuild the tool-specific settings
 * shape from these generic fields, so one URL can target any supported app regardless of which
 * tool the provider originally belongs to.
 *
 * URL format mirrors `tauri/src/coding/deeplink/parser.rs` (`SCHEME`, `VERSION`,
 * `PATH` and the accepted query params); query encoding uses
 * `application/x-www-form-urlencoded` semantics which match the Rust
 * `form_urlencoded` parser used by `url::Url::query_pairs()`.
 */

import { extractCodexBaseUrl, extractCodexModel } from '../../../utils/codexConfigUtils';
import { getClaudeConfiguredModelIds } from '../../coding/claudecode/utils/claudeModelConfig';
import type { ClaudeSettingsConfig } from '../../../types/claudecode';
import type { ProviderShareApp, SharedConnectionFields } from './providerTransfer';

export type { ProviderShareApp } from './providerTransfer';

export interface ProviderConnectionFields {
  apiKey?: string;
  baseUrl?: string;
  model?: string;
}

export interface ProviderShareUrlInput extends SharedConnectionFields {
  /** Target tool the receiving side imports into (`app` query param). */
  app: ProviderShareApp;
  name: string;
  category?: string;
  homepage?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sourceProviderId?: string;
}

const SHARE_SCHEME = 'aitoolbox';
const SHARE_VERSION = 'v1';
const SHARE_PATH = '/import';

function parseJsonObject(raw: string | undefined): Record<string, unknown> | null {
  if (!raw?.trim()) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function readEnvString(env: unknown, key: string): string | undefined {
  if (!env || typeof env !== 'object') return undefined;
  const value = (env as Record<string, unknown>)[key];
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function readString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

/**
 * Pick the representative model for a Claude provider. `ANTHROPIC_MODEL`
 * (the explicit fallback) wins; when it is unset but role models (sonnet /
 * opus / fable / haiku, plus legacy top-level fields) are configured, the
 * first configured one is shared instead so cross-tool imports do not end up
 * with an empty model. The Claude-only `[1M]` context suffix is stripped —
 * it is meaningless to other tools.
 */
function firstClaudeModel(parsed: Record<string, unknown>): string | undefined {
  const configured = getClaudeConfiguredModelIds(
    parsed as ClaudeSettingsConfig,
    { stripOneMMarker: true },
  );
  return configured[0];
}

/**
 * Extract the generic connection fields from a provider's `settingsConfig`
 * JSON string, dispatching by the tool the provider belongs to. Fields that
 * cannot be located are omitted — the deep-link import treats them as absent.
 */
export function extractProviderConnectionFields(
  sourceApp: 'claude' | 'codex' | 'gemini',
  settingsConfig: string | undefined,
): ProviderConnectionFields {
  const parsed = parseJsonObject(settingsConfig);
  if (!parsed) return {};

  let fields: ProviderConnectionFields;
  if (sourceApp === 'claude') {
    fields = {
      apiKey: readEnvString(parsed.env, 'ANTHROPIC_AUTH_TOKEN') ?? readEnvString(parsed.env, 'ANTHROPIC_API_KEY'),
      baseUrl: readEnvString(parsed.env, 'ANTHROPIC_BASE_URL'),
      model: firstClaudeModel(parsed),
    };
  } else if (sourceApp === 'codex') {
    const configToml = typeof parsed.config === 'string' ? parsed.config : undefined;
    fields = {
      apiKey: readString(
        parsed.auth && typeof parsed.auth === 'object'
          ? (parsed.auth as Record<string, unknown>).OPENAI_API_KEY
          : undefined,
      ),
      baseUrl: extractCodexBaseUrlSafe(configToml),
      model: extractCodexModelSafe(configToml),
    };
  } else {
    const env = parsed.env;
    fields = {
      apiKey: readEnvString(env, 'GEMINI_API_KEY'),
      baseUrl: readEnvString(env, 'GOOGLE_GEMINI_BASE_URL'),
      model: readEnvString(env, 'GEMINI_MODEL'),
    };
  }

  return compactFields(fields);
}

function compactFields(fields: ProviderConnectionFields): ProviderConnectionFields {
  const compacted: ProviderConnectionFields = {};
  if (fields.apiKey) compacted.apiKey = fields.apiKey;
  if (fields.baseUrl) compacted.baseUrl = fields.baseUrl;
  if (fields.model) compacted.model = fields.model;
  return compacted;
}

function extractCodexBaseUrlSafe(configToml: string | undefined): string | undefined {
  try {
    return extractCodexBaseUrl(configToml)?.trim() || undefined;
  } catch {
    return undefined;
  }
}

function extractCodexModelSafe(configToml: string | undefined): string | undefined {
  try {
    return extractCodexModel(configToml)?.trim() || undefined;
  } catch {
    return undefined;
  }
}

function isHttpUrl(value: string | undefined): value is string {
  return typeof value === 'string' && /^https?:\/\//i.test(value.trim());
}

/**
 * Sanitize a provider homepage for the deep-link share path. The backend
 * parser hard-fails the whole import on a non-http/https homepage, so both
 * share outputs (the URL query param and the direct local-import request)
 * must go through this same filter — an unfiltered value would make "copy
 * link" silently drop the field while "import to this device" errors out.
 */
export function sanitizeShareHomepage(value: string | undefined): string | undefined {
  return isHttpUrl(value) ? value.trim() : undefined;
}

/**
 * Build the `aitoolbox://v1/import?...` share URL. Empty optional fields are
 * omitted entirely (the backend parser also drops empty values, but keeping
 * the URL clean makes it shorter and easier to inspect).
 */
export function buildProviderShareUrl(input: ProviderShareUrlInput): string {
  const params = new URLSearchParams();
  params.set('resource', 'provider');
  params.set('app', input.app);
  params.set('name', input.name.trim());
  if (input.category?.trim()) params.set('category', input.category.trim());
  if (input.apiKey?.trim()) params.set('apiKey', input.apiKey.trim());
  if (input.baseUrl?.trim()) params.set('baseUrl', input.baseUrl.trim());
  if (input.model?.trim()) params.set('model', input.model.trim());
  // The backend parser only accepts http/https URLs for `homepage`; skip
  // anything else so the receiving import cannot fail on a malformed value.
  const homepage = sanitizeShareHomepage(input.homepage);
  if (homepage) params.set('homepage', homepage);
  if (input.notes?.trim()) params.set('notes', input.notes.trim());
  if (input.icon?.trim()) params.set('icon', input.icon.trim());
  if (input.iconColor?.trim()) params.set('iconColor', input.iconColor.trim());
  for (const key of ['sourceApp', 'baseUrlStyle', 'apiFormat', 'apiVersion', 'sourceProviderId', 'providerType', 'apiKeyField'] as const) {
    if (input[key]?.trim()) params.set(key, input[key].trim());
  }
  for (const key of ['models', 'modelRoles', 'headers', 'gatewayProfile'] as const) {
    const value = input[key];
    if (value && Object.keys(value).length > 0) params.set(key, JSON.stringify(value));
  }

  return `${SHARE_SCHEME}://${SHARE_VERSION}${SHARE_PATH}?${params.toString()}`;
}

/** Mask an API key for preview display: first 4 chars + 20 asterisks; ≤4 chars fully masked. */
export function maskApiKey(apiKey: string | undefined): string {
  if (!apiKey) return '';
  if (apiKey.length <= 4) return '****';
  return `${apiKey.slice(0, 4)}${'*'.repeat(20)}`;
}
