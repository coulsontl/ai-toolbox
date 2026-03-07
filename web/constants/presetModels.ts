/**
 * Preset models configuration for different AI SDK types.
 *
 * The canonical data lives in tauri/resources/preset_models.json.
 * On app startup the Rust backend loads the bundled defaults (or local
 * cache) and populates PRESET_MODELS, then the frontend background-
 * fetches the latest version from the remote repository.
 */

export interface PresetModel {
  id: string;
  name: string;
  contextLimit?: number;
  outputLimit?: number;
  modalities?: { input: string[]; output: string[] };
  attachment?: boolean;
  reasoning?: boolean;
  tool_call?: boolean;
  temperature?: boolean;
  variants?: Record<string, unknown>;
  options?: Record<string, unknown>;
}

/**
 * Remote URL for fetching the latest preset models JSON.
 * Points to the raw file in the main branch of the repository.
 */
export const PRESET_MODELS_REMOTE_URL =
  'https://raw.githubusercontent.com/coulsontl/ai-toolbox/main/tauri/resources/preset_models.json';

/**
 * Preset models grouped by npm SDK type.
 *
 * Starts empty and is populated at startup from the Rust backend
 * (bundled defaults or local cache), then updated from remote.
 * Because the object reference itself never changes, every module
 * that imported it will see the latest data on its next property
 * access — no re-import needed.
 */
export const PRESET_MODELS: Record<string, PresetModel[]> = {};

/**
 * Replace the contents of PRESET_MODELS with `models`.
 * The object reference stays the same so existing imports remain valid.
 *
 * If `models` is empty or invalid the call is a no-op so that
 * existing data is never accidentally wiped out.
 */
export const updatePresetModels = (models: Record<string, PresetModel[]>) => {
  // Guard: never replace with empty / invalid data
  if (!models || typeof models !== 'object' || Object.keys(models).length === 0) {
    return;
  }
  // Remove old keys
  for (const key of Object.keys(PRESET_MODELS)) {
    delete PRESET_MODELS[key];
  }
  // Copy new keys
  Object.assign(PRESET_MODELS, models);
};
