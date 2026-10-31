import type { PresetModel } from '@/constants/presetModels';

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
}

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
 * 硬塞". The preset carries `cost`, `options` and `variants`, and none of them
 * have a counterpart here — writing them would produce keys the products never
 * read. `id` is not returned either: the model id is the entry's identity and
 * the rename path differs between add and edit, so the caller owns it.
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
 */
export const presetToModelsJsonFields = (preset: PresetModel): ModelsJsonPresetFields => ({
  ...(preset.name ? { name: preset.name } : {}),
  ...(preset.contextLimit ? { maxInputTokens: preset.contextLimit } : {}),
  ...(preset.outputLimit ? { maxOutputTokens: preset.outputLimit } : {}),
  ...(preset.modalities?.input?.includes('image') ? { supportsImages: true } : {}),
  ...(preset.tool_call === undefined ? {} : { supportsToolCall: preset.tool_call }),
  ...(preset.reasoning === undefined ? {} : { supportsReasoning: preset.reasoning }),
});

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
  },
): {
  id: string;
  name: string;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  supportsToolCall: boolean;
  supportsImages: boolean;
  supportsReasoning: boolean;
} => ({
  id: fetched.id,
  name: matchedPreset?.name || fetched.name || fetched.id,
  ...(matchedPreset?.contextLimit ? { maxInputTokens: matchedPreset.contextLimit } : {}),
  ...(matchedPreset?.outputLimit ? { maxOutputTokens: matchedPreset.outputLimit } : {}),
  supportsToolCall: matchedPreset?.tool_call ?? existing.supportsToolCall,
  supportsImages: matchedPreset
    ? Boolean(matchedPreset.modalities?.input?.includes('image'))
    : existing.supportsImages,
  supportsReasoning: matchedPreset?.reasoning ?? existing.supportsReasoning,
});
