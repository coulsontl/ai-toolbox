import { invoke } from '@tauri-apps/api/core';
import type { AllApiHubProviderItem, AllApiHubProvidersResult } from '@/types/allApiHub';
import type {
  DshCredentialInput,
  DshModelSettingsInput,
  DshModelsProviderInput,
  DshPathInfo,
  DshRuntimeConfig,
  DshSettingsConfig,
  DshSettingsConfigInput,
} from '@/types/dsh';

export const getDshPathInfo = async (): Promise<DshPathInfo> => {
  return await invoke<DshPathInfo>('get_dsh_path_info');
};

export const getDshSettingsConfig = async (): Promise<DshSettingsConfig | null> => {
  return await invoke<DshSettingsConfig | null>('get_dsh_settings_config');
};

export const saveDshSettingsConfig = async (
  input: DshSettingsConfigInput,
): Promise<void> => {
  await invoke('save_dsh_settings_config', { input });
};

export const readDshRuntimeConfig = async (): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('read_dsh_runtime_config');
};

export const saveDshModelSettings = async (
  input: DshModelSettingsInput,
): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('save_dsh_model_settings', { input });
};

export const saveDshOtherSettings = async (
  otherSettings: Record<string, unknown>,
): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('save_dsh_other_settings', { otherSettings });
};

export const saveDshModelsProvider = async (
  input: DshModelsProviderInput,
): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('save_dsh_models_provider', { input });
};

/** Persist the drag order of `llm-pi-ai.providers`. */
export const reorderDshProviders = async (
  providerKeys: string[],
): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('reorder_dsh_providers', { providerKeys });
};

export const saveDshCredential = async (
  input: DshCredentialInput,
): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('save_dsh_credential', { input });
};

export const getDshCredentialValue = async (refName: string): Promise<string | null> => {
  return await invoke<string | null>('get_dsh_credential_value', { refName });
};

export const deleteDshCredential = async (
  refName: string,
): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('delete_dsh_credential', { refName });
};

export const deleteDshRuntimeProvider = async (
  providerKey: string,
): Promise<DshRuntimeConfig> => {
  return await invoke<DshRuntimeConfig>('delete_dsh_runtime_provider', { providerKey });
};
export const listDshAllApiHubProviders = async (): Promise<AllApiHubProvidersResult> => {
  return await invoke<AllApiHubProvidersResult>('list_dsh_all_api_hub_providers');
};

export const resolveDshAllApiHubProviders = async (
  providerIds: string[]
): Promise<AllApiHubProviderItem[]> => {
  return await invoke<AllApiHubProviderItem[]>('resolve_dsh_all_api_hub_providers', {
    request: { providerIds },
  });
};

/** 探测并打开 DSh 本地 Web UI;服务离线时 reject。 */
export const openDshWebUi = async (path?: string): Promise<void> => {
  await invoke('open_dsh_web_ui', { path });
};

/** 在用户终端启动 `dsh web`(或经 useNpx 回退 `npx @deepseek-ai/dsh web`)。 */
export const launchDshDashboard = async (useNpx = false): Promise<void> => {
  await invoke('launch_dsh_dashboard', { useNpx });
};

export interface DshAgentInstructionsStatus {
  enabled: boolean;
}

/** 检测 agent-instructions 插件是否启用。 */
export const checkDshAgentInstructions = async (): Promise<DshAgentInstructionsStatus> => {
  return await invoke<DshAgentInstructionsStatus>('check_dsh_agent_instructions');
};

/** 启用 agent-instructions 插件(往 home 级 cordis.patch.yml 写 disabled: false)。 */
export const enableDshAgentInstructions = async (): Promise<void> => {
  await invoke('enable_dsh_agent_instructions');
};
