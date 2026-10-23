/**
 * Small record helpers for OMP's `models.yml` provider entries.
 *
 * Shared by the page (which writes them) and `OmpProviderFormModal` (which
 * builds them), so "empty means delete the key" is decided in one place. Kept
 * separate from Pi's copy on purpose: the two CLIs write different files and
 * only happen to agree on these three rules today.
 */

export const asRecord = (value: unknown): Record<string, unknown> => (
  value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
);

export const getStringField = (value: Record<string, unknown>, key: string): string => {
  const fieldValue = value[key];
  return typeof fieldValue === 'string' ? fieldValue : '';
};

export const isRecordEmpty = (value: Record<string, unknown>): boolean => (
  Object.keys(value).length === 0
);

/**
 * Write a string field, or remove it when the value is blank.
 *
 * `models.yml` distinguishes an absent key from an empty string for some
 * fields, and a blank form input means "not configured".
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

/** The starting `models.yml` entry for a provider that does not exist yet. */
export const createDefaultProviderConfig = (): Record<string, unknown> => ({
  api: 'openai-completions',
  baseUrl: '',
  models: [],
});

/**
 * Whether the edited entry says anything at all.
 *
 * A provider whose channel config was just blanked out should not become an
 * empty `models.yml` entry.
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
      return !isRecordEmpty(asRecord(value));
    }
    return true;
  })
);
