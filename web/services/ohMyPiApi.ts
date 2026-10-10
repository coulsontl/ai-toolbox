import { invoke } from '@tauri-apps/api/core';
import type {
  OmpCodexAccountsResult,
  OmpCodexAccountUsage,
  OmpAgentsConfig,
  OmpAgentsConfigInput,
  OmpAgentFile,
  OmpExtensionActionInput,
  OmpExtensionCommandResult,
  OmpExtensionInstallInput,
  OmpExtensionListResult,
  OmpExtensionUpdateInput,
  OmpModelSettingsInput,
  OmpModelsProviderInput,
  OmpPathInfo,
  OmpRuntimeConfig,
  OmpSettingsConfig,
  OmpSettingsConfigInput,
  SaveOmpAgentFileRequest,
} from '@/types/ohMyPi';

export const listOmpCodexAccounts = (rootPath: string): Promise<OmpCodexAccountsResult> =>
  invoke('list_omp_codex_accounts', { rootPath });

export const importOmpCodexAccount = (rootPath: string, authJson: string): Promise<OmpCodexAccountsResult> =>
  invoke('import_omp_codex_account', { rootPath, authJson });

export const switchOmpCodexAccount = (rootPath: string, accountId: string): Promise<OmpCodexAccountsResult> =>
  invoke('switch_omp_codex_account', { rootPath, accountId });

export const getOmpCodexAccountUsage = (rootPath: string, accountId: string): Promise<OmpCodexAccountUsage> =>
  invoke('get_omp_codex_account_usage', { rootPath, accountId });

export const getOmpRootPathInfo = async (): Promise<OmpPathInfo> => {
  return await invoke<OmpPathInfo>('get_omp_root_path_info');
};

export const getOmpSettingsConfig = async (): Promise<OmpSettingsConfig | null> => {
  return await invoke<OmpSettingsConfig | null>('get_omp_settings_config');
};

export const saveOmpSettingsConfig = async (
  input: OmpSettingsConfigInput,
): Promise<void> => {
  await invoke('save_omp_settings_config', { input });
};

export const readOmpRuntimeConfig = async (): Promise<OmpRuntimeConfig> => {
  return await invoke<OmpRuntimeConfig>('read_omp_runtime_config');
};

/**
 * Re-run `omp models` to refill the Codex catalog, then return the new config.
 *
 * Separate from `readOmpRuntimeConfig` because the CLI call is not a read: it
 * writes `agent.db`/`models.db` and may hit the network. Only the subscription
 * card's refresh button and the first page load should pay for it.
 */
export const refreshOmpCodexCatalog = async (): Promise<OmpRuntimeConfig> => {
  return await invoke<OmpRuntimeConfig>('refresh_omp_codex_catalog');
};

export const saveOmpModelSettings = async (
  input: OmpModelSettingsInput,
): Promise<OmpRuntimeConfig> => {
  return await invoke<OmpRuntimeConfig>('save_omp_model_settings', { input });
};

export const saveOmpOtherSettings = async (
  otherSettings: Record<string, unknown>,
): Promise<OmpRuntimeConfig> => {
  return await invoke<OmpRuntimeConfig>('save_omp_other_settings', { otherSettings });
};

export const saveOmpModelsProvider = async (
  input: OmpModelsProviderInput,
): Promise<OmpRuntimeConfig> => {
  return await invoke<OmpRuntimeConfig>('save_omp_models_provider', { input });
};

export const deleteOmpRuntimeProvider = async (
  providerKey: string,
): Promise<OmpRuntimeConfig> => {
  return await invoke<OmpRuntimeConfig>('delete_omp_runtime_provider', { providerKey });
};

// ============================================================================
// OMP subagent 集中配置(多套方案 + 一键切换)
// ============================================================================

/** 列出所有 subagent 方案;空库时返回本地 agents/*.md 的 __local__ 桥接态。 */
export const listOmpAgentsConfigs = async (): Promise<OmpAgentsConfig[]> => {
  return await invoke<OmpAgentsConfig[]>('list_omp_agents_configs');
};

export const createOmpAgentsConfig = async (
  input: OmpAgentsConfigInput,
): Promise<OmpAgentsConfig> => {
  return await invoke<OmpAgentsConfig>('create_omp_agents_config', { input });
};

export const updateOmpAgentsConfig = async (
  input: OmpAgentsConfigInput,
): Promise<OmpAgentsConfig> => {
  return await invoke<OmpAgentsConfig>('update_omp_agents_config', { input });
};

export const deleteOmpAgentsConfig = async (id: string): Promise<void> => {
  await invoke('delete_omp_agents_config', { id });
};

export const applyOmpAgentsConfig = async (configId: string): Promise<void> => {
  await invoke('apply_omp_agents_config', { configId });
};

export const clearOmpAgentsAppliedConfig = async (configId: string): Promise<void> => {
  await invoke('clear_omp_agents_applied_config', { configId });
};

export const toggleOmpAgentsConfigDisabled = async (
  configId: string,
  isDisabled: boolean,
): Promise<void> => {
  await invoke('toggle_omp_agents_config_disabled', { configId, isDisabled });
};

export const reorderOmpAgentsConfigs = async (ids: string[]): Promise<void> => {
  await invoke('reorder_omp_agents_configs', { ids });
};

/** 低层:列出现有 agents/*.md 文件视图(编辑弹窗预览当前目录)。 */
export const listOmpAgents = async (): Promise<OmpAgentFile[]> => {
  return await invoke<OmpAgentFile[]>('list_omp_agents');
};

export const saveOmpAgentFile = async (
  request: SaveOmpAgentFileRequest,
): Promise<OmpAgentFile> => {
  return await invoke<OmpAgentFile>('save_omp_agent', { request });
};

export const deleteOmpAgentFile = async (
  request: Omit<SaveOmpAgentFileRequest, 'content'>,
): Promise<void> => {
  await invoke('delete_omp_agent', { request });
};

export const listOmpExtensions = async (): Promise<OmpExtensionListResult> => {
  return await invoke<OmpExtensionListResult>('list_omp_extensions');
};

export const installOmpExtension = async (
  input: OmpExtensionInstallInput,
): Promise<OmpExtensionCommandResult> => {
  return await invoke<OmpExtensionCommandResult>('install_omp_extension', { input });
};

export const uninstallOmpExtension = async (
  input: OmpExtensionActionInput,
): Promise<OmpExtensionCommandResult> => {
  return await invoke<OmpExtensionCommandResult>('uninstall_omp_extension', { input });
};

export const updateOmpExtensions = async (
  input?: OmpExtensionUpdateInput,
): Promise<OmpExtensionCommandResult> => {
  return await invoke<OmpExtensionCommandResult>('update_omp_extensions', { input });
};