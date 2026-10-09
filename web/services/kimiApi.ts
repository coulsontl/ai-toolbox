import { invoke } from '@tauri-apps/api/core';
import type {
  KimiProvider,
  KimiProviderInput,
  KimiCommonConfig,
  KimiCommonConfigInput,
  KimiPathInfo,
  KimiOfficialAccount,
  KimiPlugin,
  KimiDeviceAuthStartResult,
  KimiPresetModel,
  KimiSettings,
} from '@/types/kimi';

export async function listKimiProviders(): Promise<KimiProvider[]> {
  return await invoke<KimiProvider[]>('list_kimi_providers');
}

export async function selectKimiProvider(id: string): Promise<void> {
  await invoke('select_kimi_provider', { id });
}

export async function createKimiProvider(provider: KimiProviderInput): Promise<KimiProvider> {
  return await invoke<KimiProvider>('create_kimi_provider', { provider });
}

export async function updateKimiProvider(provider: KimiProvider): Promise<KimiProvider> {
  return await invoke<KimiProvider>('update_kimi_provider', { provider });
}

export async function toggleKimiProviderDisabled(id: string, isDisabled: boolean): Promise<void> {
  await invoke('toggle_kimi_provider_disabled', { id, disabled: isDisabled });
}

export async function deleteKimiProvider(id: string): Promise<void> {
  await invoke('delete_kimi_provider', { id });
}

export async function getKimiCommonConfig(): Promise<KimiCommonConfig | null> {
  return await invoke<KimiCommonConfig | null>('get_kimi_common_config');
}

export async function getKimiRootPathInfo(): Promise<KimiPathInfo> {
  return await invoke<KimiPathInfo>('get_kimi_root_path_info');
}

export async function getKimiConfigFilePath(): Promise<string> {
  return await invoke<string>('get_kimi_config_file_path');
}

/** Reveal the effective Kimi config root directory in the OS file manager. */
export async function revealKimiConfigFolder(): Promise<void> {
  await invoke('reveal_kimi_config_folder');
}

/** Read the live on-disk Kimi config (config.toml) without redaction. */
export async function readKimiSettings(): Promise<KimiSettings> {
  return await invoke<KimiSettings>('read_kimi_settings');
}

export async function saveKimiCommonConfig(input: KimiCommonConfigInput): Promise<void> {
  await invoke('save_kimi_common_config', { input });
}

export async function listKimiOfficialAccounts(): Promise<KimiOfficialAccount[]> {
  return await invoke<KimiOfficialAccount[]>('list_kimi_official_accounts');
}

export async function applyKimiOfficialAccount(id: string): Promise<void> {
  await invoke('apply_kimi_official_account', { accountId: id });
}

export async function deleteKimiOfficialAccount(id: string): Promise<void> {
  await invoke('delete_kimi_official_account', { accountId: id });
}

/**
 * Starts a device-code login. Takes no provider: the official channel row is
 * created by the backend once the exchange succeeds, so an abandoned login
 * leaves nothing behind.
 */
export async function startKimiOfficialAccountDeviceAuth(): Promise<KimiDeviceAuthStartResult> {
  return await invoke<KimiDeviceAuthStartResult>('start_kimi_official_account_device_auth');
}

/** Stores the login that is live on disk as a real account. */
export async function saveKimiOfficialLocalAccount(): Promise<KimiOfficialAccount> {
  return await invoke<KimiOfficialAccount>('save_kimi_official_local_account');
}

export async function getKimiOfficialAccountAuthStatus(sessionId: string): Promise<string> {
  return await invoke<string>('get_kimi_official_account_auth_status', { sessionId });
}

export async function cancelKimiOfficialAccountDeviceAuth(sessionId: string): Promise<void> {
  await invoke('cancel_kimi_official_account_device_auth', { sessionId });
}

export async function saveKimiLocalConfig(input: { provider?: KimiProviderInput }): Promise<string> {
  return invoke<string>('save_kimi_local_config', { input });
}

export async function reorderKimiProviders(ids: string[]): Promise<void> {
  await invoke('reorder_kimi_providers', { ids });
}

export async function listKimiPlugins(): Promise<KimiPlugin[]> {
  return await invoke<KimiPlugin[]>('list_kimi_plugins');
}

/**
 * Bundled models.dev catalog models matching a provider base URL.
 * Returns an empty list when nothing matches — the caller shows no preset hint.
 */
export async function getKimiPresetModels(baseUrl: string): Promise<KimiPresetModel[]> {
  return await invoke<KimiPresetModel[]>('get_kimi_preset_models', { baseUrl });
}

