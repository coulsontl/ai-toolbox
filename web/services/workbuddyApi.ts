import { invoke } from '@tauri-apps/api/core';
import type {
  WorkbuddyModelInput,
  WorkbuddyPathInfo,
  WorkbuddyRuntimeConfig,
  WorkbuddySettingsConfig,
  WorkbuddySettingsConfigInput,
} from '@/types/workbuddy';

export const getWorkbuddyDefaultConfigDir = async (): Promise<string> => {
  return await invoke<string>('get_workbuddy_default_config_dir');
};

export const getWorkbuddyConfigDirWithoutDb = async (): Promise<string> => {
  return await invoke<string>('get_workbuddy_config_dir_without_db');
};

export const getWorkbuddyPathInfo = async (): Promise<WorkbuddyPathInfo> => {
  return await invoke<WorkbuddyPathInfo>('get_workbuddy_path_info');
};

export const getWorkbuddySettingsConfig =
  async (): Promise<WorkbuddySettingsConfig | null> => {
    return await invoke<WorkbuddySettingsConfig | null>('get_workbuddy_settings_config');
  };

export const saveWorkbuddySettingsConfig = async (
  input: WorkbuddySettingsConfigInput,
): Promise<void> => {
  await invoke('save_workbuddy_settings_config', { input });
};

export const readWorkbuddyRuntimeConfig = async (): Promise<WorkbuddyRuntimeConfig> => {
  return await invoke<WorkbuddyRuntimeConfig>('read_workbuddy_runtime_config');
};

/** `models.json` verbatim, for the preview dialog. `null` when absent. */
export const readWorkbuddyModelsFile = async (): Promise<string | null> => {
  return await invoke<string | null>('read_workbuddy_models_file');
};

export const saveWorkbuddyModel = async (
  input: WorkbuddyModelInput,
): Promise<WorkbuddyRuntimeConfig> => {
  return await invoke<WorkbuddyRuntimeConfig>('save_workbuddy_model', { input });
};

export const deleteWorkbuddyModel = async (
  modelId: string,
): Promise<WorkbuddyRuntimeConfig> => {
  return await invoke<WorkbuddyRuntimeConfig>('delete_workbuddy_model', { modelId });
};

/** Deletes every model entry of one channel. */
export const deleteWorkbuddyProvider = async (
  baseUrl: string,
  apiKey: string,
): Promise<WorkbuddyRuntimeConfig> => {
  return await invoke<WorkbuddyRuntimeConfig>('delete_workbuddy_provider', {
    baseUrl,
    apiKey,
  });
};
