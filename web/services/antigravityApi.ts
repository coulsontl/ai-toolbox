import { invoke } from '@tauri-apps/api/core';
import type {
  ConfigPathInfo,
  AntigravityCommonConfig,
  AntigravityCommonConfigInput,
  AntigravityLocalRuleFile,
  AntigravityOfficialAccount,
  AntigravityOfficialModelsResponse,
  AntigravityProvider,
  AntigravitySettings,
} from '@/types/antigravity';

export const getAntigravityConfigPath = async (): Promise<string> => {
  return await invoke<string>('get_antigravity_config_path');
};

export const getAntigravityRootPathInfo = async (): Promise<ConfigPathInfo> => {
  return await invoke<ConfigPathInfo>('get_antigravity_root_path_info');
};

export const revealAntigravityConfigFolder = async (): Promise<void> => {
  await invoke('reveal_antigravity_config_folder');
};

export const readAntigravitySettings = async (): Promise<AntigravitySettings> => {
  return await invoke<AntigravitySettings>('read_antigravity_settings');
};

export const getAntigravityOfficialProvider = async (): Promise<AntigravityProvider> => {
  return await invoke<AntigravityProvider>('get_antigravity_official_provider');
};

export const listAntigravityOfficialAccounts = async (
  providerId: string,
): Promise<AntigravityOfficialAccount[]> => {
  return await invoke<AntigravityOfficialAccount[]>('list_antigravity_official_accounts', {
    providerId,
  });
};

export const startAntigravityOfficialAccountOauth = async (
  providerId: string,
): Promise<AntigravityOfficialAccount> => {
  return await invoke<AntigravityOfficialAccount>('start_antigravity_official_account_oauth', {
    providerId,
  });
};

export const saveAntigravityOfficialLocalAccount = async (
  providerId: string,
): Promise<AntigravityOfficialAccount> => {
  return await invoke<AntigravityOfficialAccount>('save_antigravity_official_local_account', {
    providerId,
  });
};

export const applyAntigravityOfficialAccount = async (
  providerId: string,
  accountId: string,
): Promise<void> => {
  await invoke('apply_antigravity_official_account', { providerId, accountId });
};

export const deleteAntigravityOfficialAccount = async (
  providerId: string,
  accountId: string,
): Promise<void> => {
  await invoke('delete_antigravity_official_account', { providerId, accountId });
};

export const refreshAntigravityOfficialAccountLimits = async (
  providerId: string,
  accountId: string,
): Promise<AntigravityOfficialAccount> => {
  return await invoke<AntigravityOfficialAccount>('refresh_antigravity_official_account_limits', {
    providerId,
    accountId,
  });
};

export const copyAntigravityOfficialAccountToken = async (
  providerId: string,
  accountId: string,
  tokenKind: 'access' | 'refresh',
): Promise<void> => {
  await invoke('copy_antigravity_official_account_token', {
    input: {
      providerId,
      accountId,
      tokenKind,
    },
  });
};

export const getAntigravityCommonConfig = async (): Promise<AntigravityCommonConfig | null> => {
  return await invoke<AntigravityCommonConfig | null>('get_antigravity_common_config');
};

export const extractAntigravityCommonConfigFromCurrentFile =
  async (): Promise<AntigravityCommonConfig> => {
    return await invoke<AntigravityCommonConfig>('extract_antigravity_common_config_from_current_file');
  };

export const saveAntigravityCommonConfig = async (
  input: AntigravityCommonConfigInput,
): Promise<void> => {
  await invoke('save_antigravity_common_config', { input });
};

export const fetchAntigravityOfficialModels = async (): Promise<AntigravityOfficialModelsResponse> => {
  return await invoke<AntigravityOfficialModelsResponse>('fetch_antigravity_official_models');
};

export const listAntigravityLocalRuleFiles = async (): Promise<AntigravityLocalRuleFile[]> => {
  return await invoke<AntigravityLocalRuleFile[]>('list_antigravity_local_rule_files');
};
