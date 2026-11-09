import type { CodexApiFormat, GatewayProviderMeta } from '../../../../types/codex';

/**
 * Tool surface Codex may advertise to a directly-connected native Responses
 * provider.
 *
 * `safe` omits the private tool types (`custom` apply_patch, `tool_search`,
 * `namespace` multi-agent) that third-party Responses gateways validate
 * against a closed enum. `full` keeps the complete Codex template.
 */
export type CodexResponsesToolProfile = 'safe' | 'full';

/** Minimal shape needed from a provider-profile endpoint. */
export interface CodexResponsesToolProfileSource {
  codexResponsesToolProfile?: string;
  codex_responses_tool_profile?: string;
}

/**
 * Accept the same spellings as the Rust resolver so a catalog or meta record
 * written by another tool cannot make the form show a different profile than
 * the backend applies.
 */
export function normalizeCodexResponsesToolProfile(
  value: unknown,
): CodexResponsesToolProfile | undefined {
  if (typeof value !== 'string') {
    return undefined;
  }
  switch (value.trim().toLowerCase()) {
    case 'safe':
    case 'native_safe':
    case 'native-safe':
      return 'safe';
    case 'full':
    case 'native':
      return 'full';
    default:
      return undefined;
  }
}

/** Explicit user override stored on the provider. */
export function readCodexResponsesToolProfile(
  meta: CodexResponsesToolProfileSource | undefined,
): CodexResponsesToolProfile | undefined {
  return normalizeCodexResponsesToolProfile(
    meta?.codexResponsesToolProfile ?? meta?.codex_responses_tool_profile,
  );
}

/** Replace the explicit override; `undefined` removes it. */
export function withCodexResponsesToolProfile<T extends object>(
  meta: T | undefined,
  value: CodexResponsesToolProfile | undefined,
): T | undefined {
  const nextMeta = { ...(meta || {}) } as Record<string, unknown>;
  delete nextMeta.codexResponsesToolProfile;
  delete nextMeta.codex_responses_tool_profile;
  if (value) {
    nextMeta.codexResponsesToolProfile = value;
  }
  return Object.values(nextMeta).some(
    (item) => item !== undefined && item !== null && item !== '',
  )
    ? (nextMeta as T)
    : undefined;
}

/**
 * Preset default for the selected endpoint. An undeclared endpoint keeps the
 * complete template so custom/unknown providers do not lose Codex features.
 */
export function defaultCodexResponsesToolProfile(
  endpoint: CodexResponsesToolProfileSource | undefined,
): CodexResponsesToolProfile {
  return normalizeCodexResponsesToolProfile(
    endpoint?.codexResponsesToolProfile ?? endpoint?.codex_responses_tool_profile,
  ) ?? 'full';
}

/** Effective value shown in the form: explicit override, else preset default. */
export function resolveCodexResponsesToolProfile(
  meta: GatewayProviderMeta | undefined,
  endpoint: CodexResponsesToolProfileSource | undefined,
): CodexResponsesToolProfile {
  return (
    readCodexResponsesToolProfile(meta) ??
    defaultCodexResponsesToolProfile(endpoint)
  );
}

/**
 * The switch only affects a direct native Responses connection. Gateway-bound
 * providers are flattened by the local transformer and official providers use
 * the built-in catalog, so the field must stay absent there.
 */
export function codexResponsesToolProfileApplies(
  isOfficial: boolean,
  apiFormat: CodexApiFormat | string | undefined,
): boolean {
  return !isOfficial && apiFormat === 'openai_responses';
}

/**
 * Persist only a real override: a value equal to the preset default is omitted
 * so future preset updates keep flowing to providers the user never changed.
 */
export function codexResponsesToolProfileOverride(
  safeTools: boolean | undefined,
  endpoint: CodexResponsesToolProfileSource | undefined,
): CodexResponsesToolProfile | undefined {
  const selected: CodexResponsesToolProfile = safeTools ? 'safe' : 'full';
  return selected === defaultCodexResponsesToolProfile(endpoint)
    ? undefined
    : selected;
}
