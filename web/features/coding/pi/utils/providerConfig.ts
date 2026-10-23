/**
 * Small record helpers for Pi's `models.json` provider entries.
 *
 * Shared by the page (which writes them) and `PiProviderFormModal` (which
 * builds them), so "empty means delete the key" is decided in one place.
 */

export const isRecordEmpty = (value: Record<string, unknown>): boolean => (
  Object.keys(value).length === 0
);

/**
 * A short, non-reversible preview of one API key value.
 *
 * Values starting with `$` or `!` are config-value templates (env interpolation
 * / command substitution), not secrets, so they are shown as-is: masking them
 * would hide the fact that the key is configured via a template.
 */
export const maskApiKeyValue = (key: string): string => {
  const trimmed = key.trim();
  if (!trimmed) {
    return '';
  }
  if (trimmed.startsWith('$') || trimmed.startsWith('!')) {
    return trimmed;
  }
  if (trimmed.length <= 10) {
    return '********';
  }
  return `${trimmed.slice(0, 4)}...${trimmed.slice(-4)}`;
};

/**
 * The masked key the detail sidebar shows.
 *
 * Must go through {@link resolvePiProviderApiKey}: masking only the `auth.json`
 * credential made a provider configured solely through `models.json` read as
 * "not configured" while its requests actually carried a key (the mirror image
 * of issue #435).
 */
export const maskProviderApiKey = (provider: {
  credential?: unknown;
  modelsProvider?: Record<string, unknown>;
}): string => maskApiKeyValue(resolvePiProviderApiKey(provider));

/**
 * The API key a Pi provider actually authenticates with (issue #435).
 *
 * Pi resolves a stored `auth.json` credential *before* the channel's
 * `models.json` `apiKey` (`packages/ai/README.md` — "a stored credential owns
 * its provider"; `provider-composer.ts` `composeApiKeyAuth.resolve` tries
 * `input.credential` first). Every outbound request from this page therefore
 * has to start from the credential: reading `models.json` alone sent keyless
 * requests for `/login` and form-entered keys.
 *
 * OAuth credentials carry tokens, not a portable API key, so they are not
 * used here — the caller reports "credential unavailable" instead. Only the
 * explicit `type: "api_key"` shape is read: Pi's `auth-storage.ts` rejects any
 * entry that carries neither a valid `api_key` nor a valid `oauth` shape, and
 * the backend's `credential_kind` treats a typeless entry as OAuth.
 */
export const resolvePiProviderApiKey = (provider: {
  credential?: unknown;
  modelsProvider?: Record<string, unknown>;
}): string => {
  const credential = provider.credential;
  if (credential && typeof credential === 'object') {
    const record = credential as Record<string, unknown>;
    if (record.type === 'api_key') {
      const key = record.key;
      if (typeof key === 'string' && key.trim() !== '') {
        return key.trim();
      }
    }
  }
  const modelsApiKey = provider.modelsProvider?.apiKey;
  return typeof modelsApiKey === 'string' ? modelsApiKey.trim() : '';
};

/**
 * Write a string field, or remove it when the value is blank.
 *
 * Pi's `models.json` treats an absent key and an empty string differently for
 * some fields, and a blank form input means "not configured", not "set it to
 * empty".
 */
export const setOptionalStringField = (
  target: Record<string, unknown>,
  key: string,
  value: unknown,
): void => {
  if (typeof value === 'string' && value.trim()) {
    target[key] = value.trim();
  } else {
    delete target[key];
  }
};

/** The starting `models.json` entry for a provider that does not exist yet. */
export const createDefaultProviderConfig = (): Record<string, unknown> => ({
  api: 'openai-completions',
  baseUrl: '',
  models: [],
});

/**
 * Whether the edited entry says anything at all.
 *
 * A provider that exists only in `auth.json` and whose channel config was just
 * blanked out should not create an empty `models.json` entry.
 */
export const hasProviderConfigContent = (providerConfig: Record<string, unknown>): boolean => (
  Object.values(providerConfig).some((value) => {
    if (value === null || value === undefined) {
      return false;
    }
    if (typeof value === 'string') {
      return value.trim() !== '';
    }
    if (Array.isArray(value)) {
      return value.length > 0;
    }
    if (typeof value === 'object') {
      return !isRecordEmpty(value as Record<string, unknown>);
    }
    return true;
  })
);
