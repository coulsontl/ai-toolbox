/**
 * Reading helpers for dsh provider records.
 *
 * These lived as private functions in `DshPage.tsx`; the migration to the
 * shared card moved them here so the card, the form modal and the page read
 * the same `models` array shape instead of each deciding what a model entry is.
 */
import type { DshRuntimeProviderView } from '@/types/dsh';

export const asRecord = (value: unknown): Record<string, unknown> => (
  value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
);

export const getStringField = (value: Record<string, unknown>, key: string): string => {
  const fieldValue = value[key];
  return typeof fieldValue === 'string' ? fieldValue : '';
};

export const getNumberField = (value: Record<string, unknown>, key: string): number | undefined => {
  const fieldValue = value[key];
  return typeof fieldValue === 'number' && Number.isFinite(fieldValue) ? fieldValue : undefined;
};

/**
 * The model entries a route declares explicitly.
 *
 * A route with no `models` list serves the adapter's bundled catalog
 * (`modelSource === 'builtin'`), which this function deliberately does not
 * include — that catalog is read-only and never written back.
 */
export const getProviderModelRecords = (
  providerConfig: Record<string, unknown> | undefined,
): Array<{ id: string; model: Record<string, unknown> }> => {
  if (!providerConfig) {
    return [];
  }
  const models = providerConfig.models;
  if (!Array.isArray(models)) {
    return [];
  }
  return models
    .map((model) => {
      if (typeof model === 'string') {
        return { id: model, model: { id: model } };
      }
      if (model && typeof model === 'object' && typeof (model as Record<string, unknown>).id === 'string') {
        return {
          id: (model as Record<string, string>).id,
          model: model as Record<string, unknown>,
        };
      }
      return null;
    })
    .filter((entry): entry is { id: string; model: Record<string, unknown> } => !!entry);
};

/**
 * The rows a route actually serves: its explicit `models` list, or the
 * adapter's bundled catalog when it declares none (`modelSource === 'builtin'`).
 *
 * Read by the card, the connectivity probe and every save path. **The three
 * fallbacks must stay in one place**: with a second copy, the card and the save
 * path can disagree about which models a route serves, and the disagreement is
 * invisible (each side looks right on its own).
 */
export const getDshModelRecords = (
  provider: DshRuntimeProviderView,
): Array<{ id: string; model: Record<string, unknown> }> => {
  const explicit = getProviderModelRecords(provider.provider);
  if (explicit.length > 0) {
    return explicit;
  }
  if (provider.modelSource !== 'builtin' || !provider.builtinModels) {
    return [];
  }
  return provider.builtinModels
    .filter((model): model is Record<string, unknown> & { id: string } => (
      typeof model.id === 'string'
    ))
    .map((model) => ({ id: model.id, model }));
};
