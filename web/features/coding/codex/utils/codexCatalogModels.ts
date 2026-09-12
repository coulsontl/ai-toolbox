import type { CodexCatalogModel } from '../../../../types/codex';

function normalizeStringArray(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }

  const items = value
    .map((item) => (typeof item === 'string' ? item.trim() : ''))
    .filter((item) => item.length > 0);

  return items.length > 0 ? items : undefined;
}

export function normalizeCodexCatalogModalities(value: unknown): CodexCatalogModel['modalities'] | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return undefined;
  }

  const modalities = value as { input?: unknown; output?: unknown };
  const input = normalizeStringArray(modalities.input);
  const output = normalizeStringArray(modalities.output);

  if (!input && !output) {
    return undefined;
  }

  return {
    ...(input ? { input } : {}),
    ...(output ? { output } : {}),
  };
}

export function normalizeCodexCatalogReasoningLevels(value: unknown): string[] | undefined {
  return normalizeStringArray(value);
}

export function normalizeCodexCatalogServiceTiers(value: unknown): string[] | undefined {
  return normalizeStringArray(value);
}

export function normalizeCodexCatalogModels(models: CodexCatalogModel[]): CodexCatalogModel[] {
  // Dedup by (model, displayName) so the same actual request model can appear
  // multiple times under different menu display names (e.g. mapping both
  // "luna" and "terra" menu entries to the same upstream model). Fully
  // identical rows are still collapsed.
  const seenKeys = new Set<string>();
  const normalizedModels: CodexCatalogModel[] = [];

  for (const item of models) {
    const model = item.model.trim();
    if (!model) {
      continue;
    }
    const displayName = item.displayName?.trim();
    const dedupKey = `${model}\0${displayName ?? ''}`;
    if (seenKeys.has(dedupKey)) {
      continue;
    }
    seenKeys.add(dedupKey);

    const rawContextWindow = String(item.contextWindow ?? '').replace(/[^\d]/g, '');
    const contextWindow = rawContextWindow ? Number.parseInt(rawContextWindow, 10) : undefined;
    const modalities = normalizeCodexCatalogModalities(item.modalities);
    const reasoningLevels = normalizeCodexCatalogReasoningLevels(item.reasoningLevels);
    const defaultReasoningLevel =
      typeof item.defaultReasoningLevel === 'string' && item.defaultReasoningLevel.trim()
        ? item.defaultReasoningLevel.trim()
        : undefined;
    const serviceTiers = normalizeCodexCatalogServiceTiers(item.serviceTiers);

    normalizedModels.push({
      model,
      ...(displayName ? { displayName } : {}),
      ...(contextWindow && contextWindow > 0 ? { contextWindow } : {}),
      ...(typeof item.supportsImage === 'boolean' ? { supportsImage: item.supportsImage } : {}),
      ...(typeof item.vision === 'boolean' ? { vision: item.vision } : {}),
      ...(typeof item.attachment === 'boolean' ? { attachment: item.attachment } : {}),
      ...(modalities ? { modalities } : {}),
      ...(reasoningLevels ? { reasoningLevels } : {}),
      ...(defaultReasoningLevel ? { defaultReasoningLevel } : {}),
      ...(serviceTiers ? { serviceTiers } : {}),
    });
  }

  return normalizedModels;
}

/** Looks up the preset context window (tokens) for a model id, or undefined
 * when no preset matches or the limit is not positive. Injected by the caller
 * so this module stays decoupled from the preset store. */
export type CatalogContextWindowResolver = (modelId: string) => number | undefined;

/**
 * Merges models imported from the provider API (FetchModelsModal) into the
 * current mapping rows.
 *
 * - The final order follows `orderedModelIds` — the modal's grouped display
 *   order of the fetched list — so the mapping mirrors what the user saw,
 *   including rows that already existed. Rows whose model is unknown to this
 *   fetch (custom/pinned models) keep their previous relative order at the
 *   end.
 * - Rows whose model id is in removedModelIds are dropped. The modal only
 *   fills this list when the user explicitly opts in to removing models that
 *   no longer exist upstream, so transient upstream fluctuations never wipe
 *   mappings behind the user's back.
 * - Selected ids that already exist in the mapping are skipped, preserving
 *   user customizations (display name / context window / levels); the modal
 *   also disables checkboxes for existing ids. Rows sharing a model id but
 *   differing in displayName are kept intact.
 * - New rows only set contextWindow from the injected exact preset lookup.
 *   reasoningLevels stay unset on purpose: the generated Codex catalog keeps
 *   its default full reasoning level set for rows without an explicit one.
 */
export function importModelsIntoCatalog(
  current: CodexCatalogModel[],
  selectedModels: Array<{ id?: string }>,
  removedModelIds: string[],
  orderedModelIds: string[],
  resolveContextWindow: CatalogContextWindowResolver,
): CodexCatalogModel[] {
  const removed = new Set(removedModelIds);
  // Group current rows by model id so duplicate ids with different display
  // names survive the reorder as intact groups.
  const keptGroups = new Map<string, CodexCatalogModel[]>();
  for (const item of current) {
    if (removed.has(item.model)) {
      continue;
    }
    const group = keptGroups.get(item.model);
    if (group) {
      group.push(item);
    } else {
      keptGroups.set(item.model, [item]);
    }
  }

  const selectedIds = new Set(
    selectedModels
      .map((item) => item.id?.trim())
      .filter((id): id is string => !!id),
  );

  const rows: CodexCatalogModel[] = [];
  const placedIds = new Set<string>();
  for (const rawId of orderedModelIds) {
    const model = rawId?.trim();
    if (!model || placedIds.has(model)) {
      continue;
    }
    placedIds.add(model);
    const group = keptGroups.get(model);
    if (group) {
      rows.push(...group);
      keptGroups.delete(model);
      continue;
    }
    if (selectedIds.has(model)) {
      const contextWindow = resolveContextWindow(model);
      rows.push({
        model,
        ...(contextWindow && contextWindow > 0 ? { contextWindow } : {}),
      });
    }
  }

  // Models unknown to this fetch (custom/pinned entries) keep their previous
  // relative order after the grouped block.
  for (const group of keptGroups.values()) {
    rows.push(...group);
  }

  return rows;
}
