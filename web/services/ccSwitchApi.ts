import { invoke } from '@tauri-apps/api/core';

export interface CcSwitchProviderCandidate {
  providerId: string;
  rawId: string;
  name: string;
  appType: string;
  category?: string;
  normalizedCategory: string;
  /** Row tools: JSON string. Map tools: object. */
  settingsConfig: string | Record<string, unknown>;
  extraSettingsConfig?: string;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  baseUrlPreview?: string;
  hasApiKey: boolean;
  isLocalEndpoint: boolean;
  modelPreview?: string;
  sourceProviderId?: string;
}

export interface CcSwitchDiscovery {
  found: boolean;
  dbPath?: string;
  providers: CcSwitchProviderCandidate[];
  message?: string;
}

export const hasCcSwitchDb = async (): Promise<boolean> => {
  return await invoke<boolean>('has_cc_switch_db');
};

export const listCcSwitchProviders = async (
  appType: string,
  dbPath?: string,
): Promise<CcSwitchDiscovery> => {
  return await invoke<CcSwitchDiscovery>('list_cc_switch_providers', {
    appType,
    dbPath: dbPath ?? null,
  });
};
