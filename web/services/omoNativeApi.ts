import { invoke } from '@tauri-apps/api/core';
import type {
  OmoNativeAgentsConfig,
  OmoNativeAgentsConfigInput,
  OmoNativeBuiltinProvider,
  OmoNativeCliInfo,
  OmoNativeExtensionActionInput,
  OmoNativeExtensionCommandResult,
  OmoNativeExtensionEnabledInput,
  OmoNativeExtensionInstallInput,
  OmoNativeExtensionListResult,
  OmoNativeExtensionUpdateInput,
  OmoNativeMcpServer,
  OmoNativeModelProfile,
  OmoNativeModelSettingsInput,
  OmoNativeOtherSettingsInput,
  OmoNativePathInfo,
  OmoNativeProvider,
  OmoNativeProviderInput,
  OmoNativeRuntimeConfig,
  OmoNativeSettingsConfig,
  OmoNativeSettingsConfigInput,
} from '@/types/omoNative';

// ============================================================================
// 路径与设置
// ============================================================================

export const getOmoNativeRootPathInfo = async (): Promise<OmoNativePathInfo> =>
  await invoke<OmoNativePathInfo>('get_omo_native_root_path_info');

export const getOmoNativeSettingsConfig = async (): Promise<OmoNativeSettingsConfig | null> =>
  await invoke<OmoNativeSettingsConfig | null>('get_omo_native_settings_config');

export const saveOmoNativeSettingsConfig = async (
  config: OmoNativeSettingsConfigInput,
): Promise<void> => {
  await invoke('save_omo_native_settings_config', { config });
};

export const readOmoNativeRuntimeConfig = async (): Promise<OmoNativeRuntimeConfig> =>
  await invoke<OmoNativeRuntimeConfig>('read_omo_native_runtime_config');

/**
 * 保存 `settings.json` 里的默认模型选择（`defaultProvider` / `defaultModel` /
 * `defaultThinkingLevel`），按键局部更新，其余键原样保留。
 *
 * 字段为 `''` 表示删除该键（回落引擎默认），`undefined` 表示这次不动它。
 */
export const saveOmoNativeModelSettings = async (
  input: OmoNativeModelSettingsInput,
): Promise<OmoNativeRuntimeConfig> =>
  await invoke<OmoNativeRuntimeConfig>('save_omo_native_model_settings', { input });

export const getOmoNativeCliInfo = async (): Promise<OmoNativeCliInfo> =>
  await invoke<OmoNativeCliInfo>('get_omo_native_cli_info');

// ============================================================================
// Agent/Category 方案
// ============================================================================

export const listOmoNativeAgentsConfigs = async (): Promise<OmoNativeAgentsConfig[]> =>
  await invoke<OmoNativeAgentsConfig[]>('list_omo_native_agents_configs');

export const createOmoNativeAgentsConfig = async (
  config: OmoNativeAgentsConfigInput,
): Promise<OmoNativeAgentsConfig> =>
  await invoke<OmoNativeAgentsConfig>('create_omo_native_agents_config', { config });

export const updateOmoNativeAgentsConfig = async (
  config: OmoNativeAgentsConfigInput,
): Promise<OmoNativeAgentsConfig> =>
  await invoke<OmoNativeAgentsConfig>('update_omo_native_agents_config', { config });

export const deleteOmoNativeAgentsConfig = async (configId: string): Promise<void> => {
  await invoke('delete_omo_native_agents_config', { configId });
};

export const reorderOmoNativeAgentsConfigs = async (ids: string[]): Promise<void> => {
  await invoke('reorder_omo_native_agents_configs', { ids });
};

export const toggleOmoNativeAgentsConfigDisabled = async (
  configId: string,
  isDisabled: boolean,
): Promise<void> => {
  await invoke('toggle_omo_native_agents_config_disabled', { configId, isDisabled });
};

export const applyOmoNativeAgentsConfig = async (configId: string): Promise<void> => {
  await invoke('apply_omo_native_agents_config', { configId });
};

export const clearOmoNativeAppliedConfig = async (configId: string): Promise<void> => {
  await invoke('clear_omo_native_applied_config', { configId });
};

export const saveOmoNativeLocalConfig = async (
  config: OmoNativeAgentsConfigInput,
): Promise<OmoNativeAgentsConfig> =>
  await invoke<OmoNativeAgentsConfig>('save_omo_native_local_config', { config });

// ============================================================================
// 名单与模型档
// ============================================================================

export const listOmoNativeBuiltinAgents = async (): Promise<string[]> =>
  await invoke<string[]>('list_omo_native_builtin_agents');

export const listOmoNativeBuiltinCategories = async (): Promise<string[]> =>
  await invoke<string[]>('list_omo_native_builtin_categories');

export const listOmoNativeModelProfiles = async (): Promise<OmoNativeModelProfile[]> =>
  await invoke<OmoNativeModelProfile[]>('list_omo_native_model_profiles');

// ============================================================================
// Providers
// ============================================================================

export const listOmoNativeProviders = async (): Promise<OmoNativeProvider[]> =>
  await invoke<OmoNativeProvider[]>('list_omo_native_providers');

export const saveOmoNativeProvider = async (input: OmoNativeProviderInput): Promise<void> => {
  await invoke('save_omo_native_provider', { input });
};

export const deleteOmoNativeProvider = async (
  providerKey: string,
  removeKey: boolean,
): Promise<void> => {
  await invoke('delete_omo_native_provider', { providerKey, removeKey });
};

/** 拖拽排序的落盘动作：按给定 key 顺序重写 `models.json` 里 `providers` 的键序。 */
export const reorderOmoNativeProviders = async (keys: string[]): Promise<void> => {
  await invoke('reorder_omo_native_providers', { keys });
};

/**
 * 保存「其他配置」：`omo.jsonc` 里不属于任何 harness 块的顶层共享键。
 *
 * 后端按顶层键做原地补丁，`[native]` / `[opencode]` 两个块与控制键一律不碰。
 */
export const saveOmoNativeOtherConfig = async (config: Record<string, unknown>): Promise<void> => {
  await invoke('save_omo_native_other_config', { config });
};

/**
 * 保存「其他配置」→ `settings.json` 页签：`settings.json` 里不属于其他区块的键。
 *
 * 只提交用户实际改动的 `set` / `remove`，后端在盘上最新内容上打补丁。
 * `defaultProvider` / `defaultModel` / `defaultThinkingLevel` 归「模型设置」，
 * `packages` / `extensions` 归「扩展」区块；后端会原样保留这些键。
 */
export const saveOmoNativeOtherSettings = async (
  input: OmoNativeOtherSettingsInput,
): Promise<OmoNativeRuntimeConfig> => {
  return await invoke<OmoNativeRuntimeConfig>('save_omo_native_other_settings', {
    input,
  });
};

/**
 * **已配置凭据**的引擎内建 provider（官方认证渠道）及其内建模型目录。
 *
 * 后端逐个跑 `omo auth check`（引擎没有批量接口），实测约 8 秒——所以只在页面
 * 加载时调一次，不要放进依赖频繁变化的重渲染路径里。
 */
export const listOmoNativeBuiltinProviders = async (): Promise<OmoNativeBuiltinProvider[]> =>
  await invoke<OmoNativeBuiltinProvider[]>('list_omo_native_builtin_providers');

/**
 * 保存整份 `<agentDir>/auth.json`（整份覆盖，不是按 key 局部更新）。
 *
 * 「引擎内建渠道」的 auth.json 入口用它，让密钥能在应用内查看并编辑。
 * 后端会对每个条目的 `key` 做 config value 转义。
 */
export const saveOmoNativeAuthConfig = async (
  config: Record<string, unknown>,
): Promise<void> => {
  await invoke('save_omo_native_auth_config', { config });
};

// ============================================================================
// MCP 与 Skills
// ============================================================================

export const listOmoNativeMcpServers = async (): Promise<OmoNativeMcpServer[]> =>
  await invoke<OmoNativeMcpServer[]>('list_omo_native_mcp_servers');

export const saveOmoNativeMcpServer = async (
  name: string,
  config: Record<string, unknown>,
): Promise<void> => {
  await invoke('save_omo_native_mcp_server', { name, config });
};

export const deleteOmoNativeMcpServer = async (name: string): Promise<void> => {
  await invoke('delete_omo_native_mcp_server', { name });
};

// ============================================================================
// 扩展（`settings.json` 的 `packages` + `<agentDir>/extensions`）
// ============================================================================

/**
 * 列出已装扩展：`omo list` 的包 + `<agentDir>/extensions` 的本地 `.ts` / 目录。
 *
 * 会联网查 npm registry 的最新版本（未钉版本的包），所以不要在频繁重渲染的路径里调。
 */
export const listOmoNativeExtensions = async (): Promise<OmoNativeExtensionListResult> =>
  await invoke<OmoNativeExtensionListResult>('list_omo_native_extensions');

export const installOmoNativeExtension = async (
  input: OmoNativeExtensionInstallInput,
): Promise<OmoNativeExtensionCommandResult> =>
  await invoke<OmoNativeExtensionCommandResult>('install_omo_native_extension', { input });

export const uninstallOmoNativeExtension = async (
  input: OmoNativeExtensionActionInput,
): Promise<OmoNativeExtensionCommandResult> =>
  await invoke<OmoNativeExtensionCommandResult>('uninstall_omo_native_extension', { input });

export const updateOmoNativeExtensions = async (
  input?: OmoNativeExtensionUpdateInput,
): Promise<OmoNativeExtensionCommandResult> =>
  await invoke<OmoNativeExtensionCommandResult>('update_omo_native_extensions', { input });

/** 通过写 `settings.json` 过滤器启用 / 停用一个已装扩展。 */
export const setOmoNativeExtensionEnabled = async (
  input: OmoNativeExtensionEnabledInput,
): Promise<void> => {
  await invoke('set_omo_native_extension_enabled', { input });
};
