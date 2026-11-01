/**
 * Which models a failed connectivity test may delete.
 *
 * The two lists are not interchangeable: `modelIds` is everything the probe
 * sends (including config-only defaults and official read-only entries), while
 * `removableModelIds` is what the caller owns and can delete. Removal is
 * therefore opt-in — a caller that passes no removable list gets none, rather
 * than a button that deletes every probed model.
 */
export function selectRemovableModelIds(
  failedModelIds: string[],
  removableModelIds?: string[],
): string[] {
  const removable = new Set(removableModelIds ?? []);
  return failedModelIds.filter((modelId) => removable.has(modelId));
}

/**
 * True when a removal button should be offered at all: the caller has to both
 * supply a handler and name the ids it owns.
 */
export function canOfferModelRemoval(
  onRemoveModels: ((modelIds: string[]) => Promise<void>) | undefined,
  removableModelIds?: string[],
): boolean {
  return Boolean(onRemoveModels) && (removableModelIds?.length ?? 0) > 0;
}
