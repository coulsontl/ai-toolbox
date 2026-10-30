import { invoke } from '@tauri-apps/api/core';
import type {
  CodebuddyModelInput,
  CodebuddyPathInfo,
  CodebuddyPromptConfig,
  CodebuddyPromptConfigInput,
  CodebuddyRuntimeConfig,
  CodebuddySettingsConfig,
  CodebuddySettingsConfigInput,
} from '@/types/codebuddy';

export const getCodebuddyDefaultConfigDir = async (): Promise<string> => {
  return await invoke<string>('get_codebuddy_default_config_dir');
};

export const getCodebuddyConfigDirWithoutDb = async (): Promise<string> => {
  return await invoke<string>('get_codebuddy_config_dir_without_db');
};

export const getCodebuddyPathInfo = async (): Promise<CodebuddyPathInfo> => {
  return await invoke<CodebuddyPathInfo>('get_codebuddy_path_info');
};

export const getCodebuddySettingsConfig =
  async (): Promise<CodebuddySettingsConfig | null> => {
    return await invoke<CodebuddySettingsConfig | null>('get_codebuddy_settings_config');
  };

export const saveCodebuddySettingsConfig = async (
  input: CodebuddySettingsConfigInput,
): Promise<void> => {
  await invoke('save_codebuddy_settings_config', { input });
};

/** The raw `settings.json` body, for the read-only "other config" panel. */
export const readCodebuddySettingsFile = async (): Promise<string | null> => {
  return await invoke<string | null>('read_codebuddy_settings_file');
};

export const readCodebuddyRuntimeConfig = async (): Promise<CodebuddyRuntimeConfig> => {
  return await invoke<CodebuddyRuntimeConfig>('read_codebuddy_runtime_config');
};

/** `models.json` verbatim, for the preview dialog. `null` when absent. */
export const readCodebuddyModelsFile = async (): Promise<string | null> => {
  return await invoke<string | null>('read_codebuddy_models_file');
};

export const saveCodebuddyModel = async (
  input: CodebuddyModelInput,
): Promise<CodebuddyRuntimeConfig> => {
  return await invoke<CodebuddyRuntimeConfig>('save_codebuddy_model', { input });
};

export const deleteCodebuddyModel = async (
  modelId: string,
): Promise<CodebuddyRuntimeConfig> => {
  return await invoke<CodebuddyRuntimeConfig>('delete_codebuddy_model', { modelId });
};

export const deleteCodebuddyProvider = async (
  baseUrl: string,
  apiKey: string,
): Promise<CodebuddyRuntimeConfig> => {
  return await invoke<CodebuddyRuntimeConfig>('delete_codebuddy_provider', {
    baseUrl,
    apiKey,
  });
};

export const listCodebuddyPromptConfigs = async (): Promise<CodebuddyPromptConfig[]> => {
  return await invoke<CodebuddyPromptConfig[]>('list_codebuddy_prompt_configs');
};

export const createCodebuddyPromptConfig = async (
  input: CodebuddyPromptConfigInput,
): Promise<CodebuddyPromptConfig> => {
  return await invoke<CodebuddyPromptConfig>('create_codebuddy_prompt_config', { input });
};

export const updateCodebuddyPromptConfig = async (
  input: CodebuddyPromptConfigInput,
): Promise<CodebuddyPromptConfig> => {
  return await invoke<CodebuddyPromptConfig>('update_codebuddy_prompt_config', { input });
};

export const deleteCodebuddyPromptConfig = async (id: string): Promise<void> => {
  await invoke('delete_codebuddy_prompt_config', { id });
};

export const applyCodebuddyPromptConfig = async (configId: string): Promise<void> => {
  await invoke('apply_codebuddy_prompt_config', { configId });
};

/** Clears every applied flag and empties the live `CODEBUDDY.md`. */
export const disableCodebuddyPromptConfig = async (configId: string): Promise<void> => {
  await invoke('disable_codebuddy_prompt_config', { configId });
};

export const reorderCodebuddyPromptConfigs = async (ids: string[]): Promise<void> => {
  await invoke('reorder_codebuddy_prompt_configs', { ids });
};
