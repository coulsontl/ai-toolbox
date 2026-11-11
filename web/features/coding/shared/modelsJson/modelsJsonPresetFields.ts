import { findPresetModelById, type PresetModel } from '@/constants/presetModels';

/**
 * `models.json` entries only, expressed in the shape that file accepts.
 *
 * Every field is optional because the mapping is partial: a preset either has
 * the fact or it does not, and "the preset did not say" must stay "leave the
 * form alone" rather than being written as a default.
 */
export interface ModelsJsonPresetFields {
  name?: string;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  supportsImages?: boolean;
  supportsToolCall?: boolean;
  supportsReasoning?: boolean;
  canDisableThinking?: boolean;
  defaultEffort?: string;
  supportedEfforts?: string[];
}

/**
 * Effort levels WorkBuddy's model picker can display
 * (`settings.models.efforts.*` / `chatInput.modelSelector.effort.*`).
 *
 * Preset variant keys outside this set (notably `none` / `off`) are not
 * efforts: they mean "thinking can be turned off", which is already the
 * default for `canDisableThinking`. The level order mirrors WorkBuddy's own
 * `validateEffort` list (`minimal`, `low`, `medium`, `high`, `xhigh`, `max`).
 */
export const MODELS_JSON_REASONING_EFFORTS = [
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
  'max',
] as const;

/** Variant keys the models.json reasoning menu can represent. */
const presetSupportedEfforts = (preset: PresetModel): string[] => {
  // The remote catalogue is only checked at the top level, so a malformed
  // entry can reach here with a non-object `variants`; `in` would throw.
  const variants = preset.variants;
  if (!variants || typeof variants !== 'object') {
    return [];
  }
  return MODELS_JSON_REASONING_EFFORTS.filter((effort) => effort in variants);
};

/**
 * The npm group `models.json` entries belong to.
 *
 * `models.json` stores a `url` and an `apiKey` per entry — no protocol field at
 * all — so every entry speaks OpenAI-compatible chat completions. That is the
 * group to look presets up in, and it is why this is a constant rather than
 * something derived from the channel (unlike dsh, whose entries name their api).
 */
export const MODELS_JSON_PRESET_NPM = '@ai-sdk/openai-compatible';

/**
 * Maps a preset model onto the fields a `models.json` entry can hold.
 *
 * Deliberately partial, per SOP §4.2.4: "本 CLI 没有对应字段的，留空/继承，不要
 * 硬塞". The preset carries `cost` and `options` with no counterpart here —
 * writing them would produce keys the products never read. `variants` is not
 * written as a field, but its effort keys map to `supportedEfforts` (see
 * below). `id` is not returned either: the model id is the entry's identity
 * and the rename path differs between add and edit, so the caller owns it.
 *
 * Capability flags map like this:
 *
 * - `tool_call` → `supportsToolCall` (default `true` in `models.json`, so only
 *   an explicit `false` from the preset changes anything)
 * - `reasoning` → `supportsReasoning`
 * - `modalities.input` containing `image` → `supportsImages`. The file has one
 *   image flag and no pdf/audio/video flags, so those modalities are dropped
 *   rather than folded into `supportsImages` — a `pdf`-only model would
 *   otherwise be marked as image-capable.
 *
 * `temperature` is **not** mapped: the preset's value is a boolean ("the model
 * accepts a temperature parameter") while `models.json` stores a number
 * (the default value to send). Writing `true`/`false` there would be a type
 * error on the product's side.
 *
 * `variants` is not written as a field. Its effort keys *are* mapped to
 * `supportedEfforts`, which is what WorkBuddy's model picker reads. `none` /
 * `off` variants are not effort levels: they express that thinking can be
 * disabled, which is already `canDisableThinking`'s default (`true`), so the
 * mapping leaves the key absent rather than writing a redundant `true`.
 */
export const presetToModelsJsonFields = (preset: PresetModel): ModelsJsonPresetFields => {
  const supportedEfforts = presetSupportedEfforts(preset);
  return {
    ...(preset.name ? { name: preset.name } : {}),
    ...(preset.contextLimit ? { maxInputTokens: preset.contextLimit } : {}),
    ...(preset.outputLimit ? { maxOutputTokens: preset.outputLimit } : {}),
    ...(preset.modalities?.input?.includes('image') ? { supportsImages: true } : {}),
    ...(preset.tool_call === undefined ? {} : { supportsToolCall: preset.tool_call }),
    ...(preset.reasoning === undefined ? {} : { supportsReasoning: preset.reasoning }),
    ...(supportedEfforts.length > 0 ? { supportedEfforts } : {}),
  };
};

/** Existing entry values used when a preset does not state the field. */
export interface ModelsJsonExistingModelFields {
  name?: string;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  supportsToolCall?: boolean;
  supportsImages?: boolean;
  supportsReasoning?: boolean;
  canDisableThinking?: boolean;
  defaultEffort?: string;
  supportedEfforts?: string[];
}

/**
 * The first model entry created by the provider form.
 *
 * That form only asks for an id (plus an optional name), so everything else
 * comes from the preset catalogue. WorkBuddy displays custom entries as
 * `${name}:${id}`, making a missing name look like a duplicated bare id. When
 * the id already exists, keep its values instead of resetting them to the
 * form's defaults; `models.json` merges by id and the save would otherwise
 * wipe capabilities the user had configured.
 *
 * Shared by WorkBuddy and CodeBuddy so the two pages cannot drift.
 */
export const buildProviderFirstModelFields = (
  modelId: string,
  modelName: string | undefined,
  existing?: ModelsJsonExistingModelFields,
) => {
  const id = modelId.trim();
  const preset = findPresetModelById(id, MODELS_JSON_PRESET_NPM);
  const presetFields = preset ? presetToModelsJsonFields(preset) : {};
  return {
    id,
    name: modelName?.trim() || presetFields.name || existing?.name || id,
    maxInputTokens: presetFields.maxInputTokens ?? existing?.maxInputTokens,
    maxOutputTokens: presetFields.maxOutputTokens ?? existing?.maxOutputTokens,
    supportsToolCall: presetFields.supportsToolCall ?? existing?.supportsToolCall ?? true,
    supportsImages: presetFields.supportsImages ?? existing?.supportsImages ?? false,
    supportsReasoning: presetFields.supportsReasoning ?? existing?.supportsReasoning ?? false,
    canDisableThinking: presetFields.canDisableThinking ?? existing?.canDisableThinking ?? true,
    defaultEffort: presetFields.defaultEffort ?? existing?.defaultEffort,
    supportedEfforts: presetFields.supportedEfforts ?? existing?.supportedEfforts ?? [],
  };
};

/**
 * The value a freshly fetched model should be saved with.
 *
 * SOP §4.2.5: 「获取模型」's apply callback must match the id against the preset
 * catalogue and carry the parameters over, or the user gets a bare id and has
 * to look every limit up by hand. `existing` supplies the fields a preset has
 * nothing to say about, so the entry keeps whatever the form would have
 * defaulted to.
 */
export const buildFetchedModelsJsonModel = (
  fetched: { id: string; name?: string },
  matchedPreset: PresetModel | undefined,
  existing: {
    supportsToolCall: boolean;
    supportsImages: boolean;
    supportsReasoning: boolean;
    canDisableThinking: boolean;
    defaultEffort?: string;
    supportedEfforts: string[];
  },
): {
  id: string;
  name: string;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  supportsToolCall: boolean;
  supportsImages: boolean;
  supportsReasoning: boolean;
  canDisableThinking: boolean;
  defaultEffort?: string;
  supportedEfforts: string[];
} => {
  const presetFields = matchedPreset ? presetToModelsJsonFields(matchedPreset) : {};
  return {
    id: fetched.id,
    name: presetFields.name || fetched.name || fetched.id,
    maxInputTokens: presetFields.maxInputTokens,
    maxOutputTokens: presetFields.maxOutputTokens,
    supportsToolCall: presetFields.supportsToolCall ?? existing.supportsToolCall,
    supportsImages: presetFields.supportsImages ?? existing.supportsImages,
    supportsReasoning: presetFields.supportsReasoning ?? existing.supportsReasoning,
    canDisableThinking: presetFields.canDisableThinking ?? existing.canDisableThinking,
    defaultEffort: presetFields.defaultEffort ?? existing.defaultEffort,
    supportedEfforts: presetFields.supportedEfforts ?? existing.supportedEfforts,
  };
};
