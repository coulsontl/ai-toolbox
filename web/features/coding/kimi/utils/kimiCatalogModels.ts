import type { KimiCatalogModel, KimiPresetModel } from '@/types/kimi';

/**
 * Capabilities the Kimi CLI understands, with the caption each shows in the
 * model dialog. Values outside this set are dropped when the model catalog is
 * projected into `config.toml`.
 *
 * `thinking` lets the user disable reasoning; `always_thinking` marks a model
 * that always reasons (the CLI adds one of the two automatically for known
 * Anthropic profiles). `*_in` entries declare input modalities.
 *
 * **One list, two consumers.** The dialog renders these as checkboxes and
 * `normalizeKimiCapabilities` uses the same entries as its allowlist. They used
 * to be two literals — a `CAPABILITY_FIELDS` table in the dialog and this array
 * — so adding a capability meant remembering to edit both, and forgetting the
 * second would silently drop the value on save.
 */
export const KIMI_SUPPORTED_CAPABILITIES = [
  { value: 'image_in', labelKey: 'kimi.model.capabilityImageIn' },
  { value: 'video_in', labelKey: 'kimi.model.capabilityVideoIn' },
  { value: 'audio_in', labelKey: 'kimi.model.capabilityAudioIn' },
  { value: 'thinking', labelKey: 'kimi.model.capabilityThinking' },
  { value: 'always_thinking', labelKey: 'kimi.model.capabilityAlwaysThinking' },
  { value: 'tool_use', labelKey: 'kimi.model.capabilityToolUse' },
] as const;

/** The identifiers alone, for validation and projection. */
export const KIMI_CAPABILITY_VALUES = KIMI_SUPPORTED_CAPABILITIES.map((item) => item.value);

/**
 * Thinking efforts the Kimi CLI ships profiles for. `support_efforts` is free
 * text in the schema, but the built-in Anthropic profiles only declare these
 * four, so the picker offers exactly this set.
 */
export const KIMI_SUPPORTED_EFFORTS = ['low', 'medium', 'high', 'max'] as const;

/**
 * Row identity for the provider card's model list.
 *
 * A catalog row is identified by its alias key: the `[models."<key>"]` table
 * name in `config.toml`. Unlike Codex (which dedups on model+displayName), Kimi
 * keys are unique per entry, so the key alone is the row identity.
 */
export function kimiCatalogRowKey(item: Pick<KimiCatalogModel, 'key'>): string {
  return item.key.trim();
}

export function findKimiCatalogRowIndex(models: KimiCatalogModel[], rowKey: string): number {
  return models.findIndex((item) => kimiCatalogRowKey(item) === rowKey);
}

/** Replace the row behind `previousRowKey`, or append when it is gone. */
export function upsertKimiCatalogModel(
  models: KimiCatalogModel[],
  next: KimiCatalogModel,
  previousRowKey?: string,
): KimiCatalogModel[] {
  const index = previousRowKey ? findKimiCatalogRowIndex(models, previousRowKey) : -1;
  if (index < 0) {
    return [...models, next];
  }
  return models.map((item, itemIndex) => (itemIndex === index ? next : item));
}

export function removeKimiCatalogModels(
  models: KimiCatalogModel[],
  rowKeys: string[],
): KimiCatalogModel[] {
  const keys = new Set(rowKeys);
  return models.filter((item) => !keys.has(kimiCatalogRowKey(item)));
}

/**
 * Normalize a capability list to the vocabulary the CLI understands.
 * Returns undefined for an empty result so callers omit the field instead of
 * writing an empty array.
 */
export function normalizeKimiCapabilities(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const allowed = new Set<string>(KIMI_CAPABILITY_VALUES);
  const items = value
    .map((item) => (typeof item === 'string' ? item.trim() : ''))
    .filter((item) => allowed.has(item));
  return items.length > 0 ? items : undefined;
}

/** Normalize a free-form string list (efforts); drops blanks. */
export function normalizeKimiStringArray(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const items = value
    .map((item) => (typeof item === 'string' ? item.trim() : ''))
    .filter((item) => item.length > 0);
  return items.length > 0 ? items : undefined;
}

/**
 * Map catalog modalities to Kimi capability names. The catalog speaks
 * `text` / `image` / `video` / `audio`; the CLI wants `*_in` input capabilities.
 * `text` has no capability counterpart and is dropped.
 */
function modalitiesToCapabilities(input: string[]): string[] | undefined {
  const capabilities: string[] = [];
  const push = (value: string) => {
    if (value && !capabilities.includes(value)) {
      capabilities.push(value);
    }
  };
  for (const modality of input) {
    if (modality === 'image') push('image_in');
    else if (modality === 'video') push('video_in');
    else if (modality === 'audio') push('audio_in');
  }
  return capabilities.length > 0 ? capabilities : undefined;
}

/**
 * Build catalog rows from a bundled models.dev preset match.
 *
 * Keys use the `<providerKey>/<modelId>` shape `kimi provider catalog add`
 * writes, so a preset-imported catalog is indistinguishable from a CLI-imported
 * one. Rows already present (by upstream id) are skipped to preserve edits.
 */
export function buildKimiCatalogModelsFromPresets(
  presets: KimiPresetModel[],
  providerKey: string,
  existing: KimiCatalogModel[],
  fallbackContextSize: number,
): KimiCatalogModel[] {
  const existingUpstreamIds = new Set(existing.map((item) => item.model.trim()));
  const existingKeys = new Set(existing.map((item) => kimiCatalogRowKey(item)));
  const rows: KimiCatalogModel[] = [];

  for (const preset of presets) {
    const upstreamId = preset.id.trim();
    if (!upstreamId || existingUpstreamIds.has(upstreamId)) {
      continue;
    }
    const key = `${providerKey}/${upstreamId}`;
    if (existingKeys.has(key)) {
      continue;
    }
    const row: KimiCatalogModel = {
      key,
      model: upstreamId,
      provider: providerKey,
      // The CLI refuses to start a session without a positive context size.
      maxContextSize: preset.maxContextSize ?? fallbackContextSize,
    };
    if (preset.displayName?.trim()) {
      row.displayName = preset.displayName.trim();
    }
    if (preset.maxInputSize) {
      row.maxInputSize = preset.maxInputSize;
    }
    if (preset.maxOutputSize) {
      row.maxOutputSize = preset.maxOutputSize;
    }
    const capabilities = modalitiesToCapabilities(preset.inputModalities ?? []);
    if (capabilities) {
      row.capabilities = capabilities;
    }
    // `tool_call` is the catalog's own tool-support flag — the output modality
    // alone does not imply it, and declaring tool_use on a model that rejects
    // tool payloads makes the CLI fail at runtime.
    if (preset.toolCall) {
      row.capabilities = [...(row.capabilities ?? []), 'tool_use'];
    }
    if (preset.reasoning) {
      row.capabilities = [...(row.capabilities ?? []), 'thinking'];
    }
    rows.push(row);
  }

  return rows;
}

/**
 * Merge models pulled from the provider API into the current catalog rows.
 *
 * - The result follows `orderedModelIds` (the modal's grouped display order) so
 *   the catalog mirrors what the user saw. Rows the fetch does not know about
 *   (custom/pinned entries) keep their previous relative order at the end.
 * - Rows whose upstream id is in `removedModelIds` are dropped. The modal only
 *   fills that list when the user explicitly opts in to removing models missing
 *   upstream, so transient upstream fluctuations never wipe rows silently.
 * - A fetched id whose alias key already exists is skipped, preserving the
 *   user's customizations.
 * - A selected upstream id lands in the catalog exactly once: the ordered
 *   loop keeps the first row that already maps it, and the trailing loop does
 *   not resurrect a second alias for the same upstream.
 * - New rows get the alias key `<providerKey>/<upstreamId>` — the same shape
 *   `kimi provider catalog add` writes — and the CLI-required fallback context
 *   size, since the models API does not report one.
 */
export function importModelsIntoKimiCatalog(
  current: KimiCatalogModel[],
  selectedModels: Array<{ id?: string; name?: string }>,
  removedModelIds: string[],
  orderedModelIds: string[],
  providerKey: string,
  fallbackContextSize: number,
): KimiCatalogModel[] {
  const removed = new Set(removedModelIds.map((id) => id.trim()).filter(Boolean));
  const keptByKey = new Map<string, KimiCatalogModel>();
  const keptUpstreamIds = new Set<string>();
  for (const item of current) {
    const upstreamId = item.model.trim();
    if (removed.has(upstreamId)) {
      continue;
    }
    keptByKey.set(kimiCatalogRowKey(item), item);
    keptUpstreamIds.add(upstreamId);
  }

  const selectedById = new Map<string, { id?: string; name?: string }>();
  for (const selected of selectedModels) {
    const id = selected.id?.trim();
    if (id) selectedById.set(id, selected);
  }

  const rows: KimiCatalogModel[] = [];
  const placedKeys = new Set<string>();
  for (const rawId of orderedModelIds) {
    const upstreamId = rawId?.trim();
    if (!upstreamId || placedKeys.has(upstreamId)) {
      continue;
    }
    placedKeys.add(upstreamId);

    const existing = current.find((item) => item.model.trim() === upstreamId);
    if (existing) {
      // Already in the catalog under some alias; keep the user's row intact.
      if (!removed.has(upstreamId)) {
        rows.push(existing);
      }
      continue;
    }

    const selected = selectedById.get(upstreamId);
    if (!selected || keptUpstreamIds.has(upstreamId)) {
      continue;
    }
    const key = `${providerKey}/${upstreamId}`;
    if (keptByKey.has(key)) {
      continue;
    }
    const row: KimiCatalogModel = {
      key,
      model: upstreamId,
      provider: providerKey,
      maxContextSize: fallbackContextSize,
    };
    if (selected.name?.trim()) {
      row.displayName = selected.name.trim();
    }
    rows.push(row);
  }

  // Rows the fetch did not mention keep their previous relative order. A row
  // whose upstream id was already placed must not come back as a second alias
  // for the same upstream — the import flow treats the upstream id as the row
  // identity (the ordered loop keeps the first matching row intact), so the
  // trailing loop dedupes by upstream id, not by row reference.
  for (const item of current) {
    if (removed.has(item.model.trim()) || placedKeys.has(item.model.trim())) {
      continue;
    }
    rows.push(item);
  }

  return rows;
}
