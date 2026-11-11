import type { HttpConfig, StdioConfig } from '../types';

/**
 * `server_config` keys the add/edit form rebuilds on every save. Anything else
 * is tool-specific data (usually imported from a tool file) and must survive an
 * edit verbatim.
 */
const FORM_OWNED_SERVER_CONFIG_KEYS = [
  'type',
  'command',
  'args',
  'env',
  'environment',
  'url',
  'httpUrl',
  'serverUrl',
  'headers',
  'startup_timeout_sec',
  'tool_timeout_sec',
];

/**
 * Returns the fields on `config` that the form does not own. `additionallyOwnedKeys`
 * covers the keys owned by the current save operation only (for example OmO's
 * two timeouts are form-owned only while OmO is a sync target).
 */
export function preservedServerConfigFields(
  config: Record<string, unknown> | StdioConfig | HttpConfig | undefined,
  additionallyOwnedKeys: string[] = [],
): Record<string, unknown> {
  if (!config) {
    return {};
  }
  const ownedKeys = new Set([...FORM_OWNED_SERVER_CONFIG_KEYS, ...additionallyOwnedKeys]);
  return Object.fromEntries(
    Object.entries(config as unknown as Record<string, unknown>).filter(
      ([key]) => !ownedKeys.has(key),
    ),
  );
}

/**
 * The complement used by the JSON importer: drop the fields it normalizes into
 * the central shape, keep everything else (OmO's `lifecycle`, an imported
 * `enabled`, ...).
 */
export function extraServerConfigFields(config: Record<string, unknown>): Record<string, unknown> {
  return preservedServerConfigFields(config, [
    'name',
    'id',
    'connectTimeoutMs',
    'requestTimeoutMs',
  ]);
}

/** Parse a positive number; blanks and non-positive values mean "not set". */
export function optionalPositiveNumber(value: unknown): number | undefined {
  if (value === null || value === undefined || value === '') {
    return undefined;
  }
  const numeric = Number(value);
  return Number.isFinite(numeric) && numeric > 0 ? numeric : undefined;
}

/**
 * OmO's millisecond timeouts are serialized as integers. The Rust writer only
 * accepts whole numbers (`Value::as_u64`), so a fractional value accepted here
 * would be silently dropped at sync time.
 */
export function optionalPositiveInteger(value: unknown): number | undefined {
  const numeric = optionalPositiveNumber(value);
  return numeric !== undefined && Number.isInteger(numeric) ? numeric : undefined;
}
