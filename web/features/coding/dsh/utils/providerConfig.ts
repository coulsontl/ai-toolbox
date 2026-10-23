/**
 * Small record helpers for dsh's `llm-pi-ai.providers.<route>` entries.
 *
 * Shared by the page (which writes them) and `DshProviderFormModal` (which
 * builds them), so "empty means delete the key" is decided in one place.
 */
import type { DshRuntimeProviderView } from '@/types/dsh';

/**
 * Whether this route takes its display name and wire protocol from the adapter
 * catalog, which means the form must not offer those two fields (and the save
 * path must delete them).
 *
 * **A copy is never catalog-owned.** Copying a built-in route creates a new key
 * that the catalog does not contain, so the copy owns both fields; treating it
 * as built-in hid the inputs and then deleted the values on save.
 *
 * Both the modal (which fields to show) and the page (which fields to write)
 * ask this one function — they had separate copies of the condition and only
 * one of them learned about `copy`.
 */
export const isCatalogOwnedRoute = (
  provider: DshRuntimeProviderView | undefined,
  copy: boolean,
): boolean => Boolean(provider && !copy
  && (provider.isBuiltin || provider.modelSource === 'builtin'));

export const isRecordEmpty = (value: Record<string, unknown>): boolean => (
  Object.keys(value).length === 0
);

/**
 * Write a string field, or remove it when the value is blank.
 *
 * dsh distinguishes an absent key from an empty string for the catalog-owned
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

/** The starting `llm-pi-ai.providers` entry for a route that does not exist yet. */
export const createDefaultProviderConfig = (): Record<string, unknown> => ({
  api: 'openai-completions',
  baseURL: '',
  models: [],
});

/**
 * Whether the edited entry says anything at all.
 *
 * A route added only to point a credential at a catalog-backed adapter should
 * not get an empty provider entry.
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
