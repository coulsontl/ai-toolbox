import type { CodexHistoryProviderGroup } from '@/types/codex';

/**
 * The provider buckets the history sync should rewrite.
 *
 * Selection is by existing `model_provider`, not "everything that differs from
 * the current provider": official OpenAI sessions and other third-party
 * upstreams must stay on their own bucket unless the user explicitly opts in.
 */
export interface CodexHistorySyncSelection {
  sourceProviders: string[];
  includeMissingProvider: boolean;
}

export const emptyCodexHistorySyncSelection = (): CodexHistorySyncSelection => ({
  sourceProviders: [],
  includeMissingProvider: false,
});

/** The active provider is already routed correctly and is never migratable. */
export const isCodexHistoryGroupSelectable = (
  group: CodexHistoryProviderGroup,
): boolean => group.kind !== 'current';

/** Only legacy AI Toolbox buckets are preselected. */
export const defaultCodexHistorySyncSelection = (
  groups: CodexHistoryProviderGroup[] | null | undefined,
): CodexHistorySyncSelection => ({
  sourceProviders: (groups ?? [])
    .filter(
      (group) => group.kind === 'ai_toolbox_legacy' && Boolean(group.providerId),
    )
    .map((group) => group.providerId as string),
  includeMissingProvider: false,
});

export const setCodexHistoryProviderSelected = (
  selection: CodexHistorySyncSelection,
  providerId: string,
  selected: boolean,
): CodexHistorySyncSelection => {
  const trimmed = providerId.trim();
  if (!trimmed) {
    return selection;
  }
  if (selected) {
    if (selection.sourceProviders.includes(trimmed)) {
      return selection;
    }
    return {
      ...selection,
      sourceProviders: [...selection.sourceProviders, trimmed],
    };
  }
  if (!selection.sourceProviders.includes(trimmed)) {
    return selection;
  }
  return {
    ...selection,
    sourceProviders: selection.sourceProviders.filter((id) => id !== trimmed),
  };
};

export const setCodexHistoryMissingProviderSelected = (
  selection: CodexHistorySyncSelection,
  selected: boolean,
): CodexHistorySyncSelection => (
  selection.includeMissingProvider === selected
    ? selection
    : { ...selection, includeMissingProvider: selected }
);

export const isCodexHistorySyncSelectionEmpty = (
  selection: CodexHistorySyncSelection,
): boolean => selection.sourceProviders.length === 0 && !selection.includeMissingProvider;

export const countCodexHistorySelectedGroups = (
  selection: CodexHistorySyncSelection,
): number => selection.sourceProviders.length + (selection.includeMissingProvider ? 1 : 0);

/**
 * Whether the selection crosses upstreams.
 *
 * Migrating an `official` or `other` bucket rewrites the session to the current
 * upstream, so continuing that conversation sends its history there; encrypted
 * reasoning payloads may also be undecryptable by the new upstream.
 */
export const hasCrossUpstreamCodexHistorySelection = (
  selection: CodexHistorySyncSelection,
  groups: CodexHistoryProviderGroup[] | null | undefined,
): boolean => {
  const selected = new Set(selection.sourceProviders);
  return (groups ?? []).some(
    (group) => Boolean(group.providerId)
      && selected.has(group.providerId as string)
      && (group.kind === 'official' || group.kind === 'other'),
  );
};
