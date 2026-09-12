import type { FetchedModel } from './types';

/**
 * Sorts fetched models grouped by their `ownedBy` vendor so the list (and the
 * mapping rows created from a selection) read vendor-by-vendor instead of raw
 * API return order. Owners listed in `priorityOwnedBy` are pinned to the
 * front in that order (e.g. Codex pins "openai"); other owners follow
 * alphabetically; models without an `ownedBy` sort last. Ids compare
 * case-insensitively with natural numeric ordering (gpt-5.6 < gpt-5.10).
 * The collation locale is pinned to "en" so ordering is deterministic
 * regardless of the machine's system language.
 */
export function createFetchedModelsComparator(priorityOwnedBy?: string[]) {
  const priorities = priorityOwnedBy ?? [];
  return (a: FetchedModel, b: FetchedModel): number => {
    const ownedA = a.ownedBy?.trim() ?? '';
    const ownedB = b.ownedBy?.trim() ?? '';
    if (ownedA !== ownedB) {
      if (!ownedA) {
        return 1;
      }
      if (!ownedB) {
        return -1;
      }
      const rankA = priorities.indexOf(ownedA);
      const rankB = priorities.indexOf(ownedB);
      const rankA2 = rankA === -1 ? priorities.length : rankA;
      const rankB2 = rankB === -1 ? priorities.length : rankB;
      if (rankA2 !== rankB2) {
        return rankA2 - rankB2;
      }
    }
    if (ownedA && ownedB && ownedA !== ownedB) {
      return ownedA.localeCompare(ownedB, 'en', { sensitivity: 'base', numeric: true });
    }
    return a.id.localeCompare(b.id, 'en', { sensitivity: 'base', numeric: true });
  };
}

/** Default comparator: owner grouping without any pinned vendor. */
export const compareFetchedModels = createFetchedModelsComparator();
