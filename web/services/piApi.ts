import { invoke } from '@tauri-apps/api/core';
import type {
  PiExtensionActionInput,
  PiExtensionCommandResult,
  PiExtensionEnabledInput,
  PiExtensionInstallInput,
  PiExtensionListResult,
  PiExtensionUpdateInput,
  PiAuthProviderInput,
  PiDeleteScope,
  PiModelSettingsInput,
  PiModelsProviderInput,
  PiPathInfo,
  PiRuntimeConfig,
  PiSettingsConfig,
  PiSettingsConfigInput,
} from '@/types/pi';

export const getPiRootPathInfo = async (): Promise<PiPathInfo> => {
  return await invoke<PiPathInfo>('get_pi_root_path_info');
};

export const getPiSettingsConfig = async (): Promise<PiSettingsConfig | null> => {
  return await invoke<PiSettingsConfig | null>('get_pi_settings_config');
};

export const savePiSettingsConfig = async (
  input: PiSettingsConfigInput,
): Promise<void> => {
  await invoke('save_pi_settings_config', { input });
};

export const readPiRuntimeConfig = async (): Promise<PiRuntimeConfig> => {
  return await invoke<PiRuntimeConfig>('read_pi_runtime_config');
};

export const savePiModelSettings = async (
  input: PiModelSettingsInput,
): Promise<PiRuntimeConfig> => {
  return await invoke<PiRuntimeConfig>('save_pi_model_settings', { input });
};

export const savePiOtherSettings = async (
  otherSettings: Record<string, unknown>,
): Promise<PiRuntimeConfig> => {
  return await invoke<PiRuntimeConfig>('save_pi_other_settings', { otherSettings });
};

export const savePiAuthProvider = async (
  input: PiAuthProviderInput,
): Promise<PiRuntimeConfig> => {
  return await invoke<PiRuntimeConfig>('save_pi_auth_provider', { input });
};

export const savePiModelsProvider = async (
  input: PiModelsProviderInput,
): Promise<PiRuntimeConfig> => {
  return await invoke<PiRuntimeConfig>('save_pi_models_provider', { input });
};

/** Persist the drag order of `models.json`'s `providers` map. */
export const reorderPiProviders = async (
  providerKeys: string[],
): Promise<PiRuntimeConfig> => {
  return await invoke<PiRuntimeConfig>('reorder_pi_providers', { providerKeys });
};

export const deletePiRuntimeProvider = async (
  providerKey: string,
  scope: PiDeleteScope,
): Promise<PiRuntimeConfig> => {
  return await invoke<PiRuntimeConfig>('delete_pi_runtime_provider', { providerKey, scope });
};

export const listPiExtensions = async (): Promise<PiExtensionListResult> => {
  return await invoke<PiExtensionListResult>('list_pi_extensions');
};

export const installPiExtension = async (
  input: PiExtensionInstallInput,
): Promise<PiExtensionCommandResult> => {
  return await invoke<PiExtensionCommandResult>('install_pi_extension', { input });
};

export const uninstallPiExtension = async (
  input: PiExtensionActionInput,
): Promise<PiExtensionCommandResult> => {
  return await invoke<PiExtensionCommandResult>('uninstall_pi_extension', { input });
};

export const updatePiExtensions = async (
  input?: PiExtensionUpdateInput,
): Promise<PiExtensionCommandResult> => {
  return await invoke<PiExtensionCommandResult>('update_pi_extensions', { input });
};

export const setPiExtensionEnabled = async (
  input: PiExtensionEnabledInput,
): Promise<void> => {
  await invoke('set_pi_extension_enabled', { input });
};
