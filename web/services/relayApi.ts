import { invoke } from '@tauri-apps/api/core';

/** What backs a 2API channel. */
export type RelayChannelKind = 'http' | 'plugin';

export interface RelayChannel {
  id: string;
  name: string;
  kind: RelayChannelKind;
  base_url: string;
  api_key: string;
  target_protocol: string;
  plugin_provider_id: string;
  enabled: boolean;
  sort_index: number;
  created_at: number;
  updated_at: number;
}

export interface RelayModel {
  id: string;
  channel_id: string;
  model_id: string;
  display_name: string;
  /** The AI SDK package this model is spoken with; decides its wire protocol. */
  npm: string;
  enabled: boolean;
  sort_index: number;
}

export interface RelayPlugin {
  id: string;
  spec: string;
  target: string;
  version: string;
  enabled: boolean;
  options_json: string;
  installed_at: number;
}

export interface RelayPluginProvider {
  id: string;
  plugin_id: string;
  provider_id: string;
  label: string;
  /** The module export that produced this provider (a package can export several). */
  entry_export: string;
  npm: string;
  capabilities_json: string;
  enabled: boolean;
}

export interface RelayPluginAccount {
  id: string;
  plugin_provider_id: string;
  account_key: string;
  display_name: string;
  /** OpenCode `auth.json` shape, kept verbatim: plugins read and write it. */
  auth_json: string;
  last_used_at: number;
}

export interface RelayAccessKey {
  id: string;
  name: string;
  key_hash: string;
  key_prefix: string;
  enabled: boolean;
  last_used_at: number;
  created_at: number;
}

export interface RelayCheckinSchedule {
  id: string;
  label: string;
  enabled: boolean;
  time: string;
  time_end: string;
  providers: string[];
  provider_times: Record<string, string>;
  retry_time: string;
  retry_gap_minutes: number;
  retry_limit: number;
  last_fired_date: string;
  account_plan_json: string;
}

export interface RelayCheckinRecord {
  id: string;
  ts: number;
  date: string;
  source: string;
  plugin_provider_id: string;
  provider: string;
  account_id: string;
  account_name: string;
  /** `claimed` / `done` / `ineligible` / `inactive` / `captcha` / `failed`. */
  outcome: string;
  ok: boolean;
  credit: number;
  streak: number;
  message: string;
  until: string;
}

export const listRelayChannels = async (): Promise<RelayChannel[]> => {
  return invoke<RelayChannel[]>('relay_list_channels');
};

export const upsertRelayChannel = async (
  channel: RelayChannel,
): Promise<RelayChannel> => {
  return invoke<RelayChannel>('relay_upsert_channel', { channel });
};

export const deleteRelayChannel = async (id: string): Promise<void> => {
  return invoke<void>('relay_delete_channel', { id });
};

export const listRelayModels = async (
  channelId?: string,
): Promise<RelayModel[]> => {
  return invoke<RelayModel[]>('relay_list_models', {
    input: { channel_id: channelId ?? null },
  });
};

export const replaceRelayChannelModels = async (
  channelId: string,
  models: RelayModel[],
): Promise<RelayModel[]> => {
  return invoke<RelayModel[]>('relay_replace_channel_models', {
    channelId,
    models,
  });
};

export const listRelayPlugins = async (): Promise<RelayPlugin[]> => {
  return invoke<RelayPlugin[]>('relay_list_plugins');
};

export const upsertRelayPlugin = async (
  plugin: RelayPlugin,
): Promise<RelayPlugin> => {
  return invoke<RelayPlugin>('relay_upsert_plugin', { plugin });
};

export const deleteRelayPlugin = async (id: string): Promise<void> => {
  return invoke<void>('relay_delete_plugin', { id });
};

export const listRelayPluginProviders = async (
  pluginId?: string,
): Promise<RelayPluginProvider[]> => {
  return invoke<RelayPluginProvider[]>('relay_list_plugin_providers', {
    input: { plugin_id: pluginId ?? null },
  });
};

export const upsertRelayPluginProvider = async (
  provider: RelayPluginProvider,
): Promise<RelayPluginProvider> => {
  return invoke<RelayPluginProvider>('relay_upsert_plugin_provider', {
    provider,
  });
};

export const listRelayPluginAccounts = async (
  pluginProviderId?: string,
): Promise<RelayPluginAccount[]> => {
  return invoke<RelayPluginAccount[]>('relay_list_plugin_accounts', {
    input: { plugin_provider_id: pluginProviderId ?? null },
  });
};

export const upsertRelayPluginAccount = async (
  account: RelayPluginAccount,
): Promise<RelayPluginAccount> => {
  return invoke<RelayPluginAccount>('relay_upsert_plugin_account', {
    account,
  });
};

export const deleteRelayPluginAccount = async (id: string): Promise<void> => {
  return invoke<void>('relay_delete_plugin_account', { id });
};

export const listRelayAccessKeys = async (): Promise<RelayAccessKey[]> => {
  return invoke<RelayAccessKey[]>('relay_list_access_keys');
};

export const upsertRelayAccessKey = async (
  key: RelayAccessKey,
): Promise<RelayAccessKey> => {
  return invoke<RelayAccessKey>('relay_upsert_access_key', { key });
};

export const deleteRelayAccessKey = async (id: string): Promise<void> => {
  return invoke<void>('relay_delete_access_key', { id });
};

export const listRelayCheckinSchedules = async (): Promise<
  RelayCheckinSchedule[]
> => {
  return invoke<RelayCheckinSchedule[]>('relay_list_checkin_schedules');
};

export const upsertRelayCheckinSchedule = async (
  schedule: RelayCheckinSchedule,
): Promise<RelayCheckinSchedule> => {
  return invoke<RelayCheckinSchedule>('relay_upsert_checkin_schedule', {
    schedule,
  });
};

export const listRelayCheckinRecords = async (
  limit?: number,
): Promise<RelayCheckinRecord[]> => {
  return invoke<RelayCheckinRecord[]>('relay_list_checkin_records', {
    limit: limit ?? null,
  });
};

// ---- Bun runtime -------------------------------------------------------------

export interface BunRuntimeStatus {
  version: string;
  /** Absolute path of the installed binary; empty when not installed. */
  path: string;
  installed: boolean;
  /** False when this platform has no pinned checksum: installing is refused. */
  verified: boolean;
  asset: string;
}

export interface BunDownloadProgress {
  downloaded: number;
  total: number;
  speed: number;
}

export const getRelayBunStatus = async (): Promise<BunRuntimeStatus> => {
  return invoke<BunRuntimeStatus>('relay_bun_status');
};

export const installRelayBun = async (): Promise<BunRuntimeStatus> => {
  return invoke<BunRuntimeStatus>('relay_bun_install');
};

export const uninstallRelayBun = async (): Promise<void> => {
  return invoke<void>('relay_bun_uninstall');
};

// ---- Plugin host -------------------------------------------------------------

export interface RelayHostStatus {
  running: boolean;
  /** Consecutive failed starts; a successful one resets it. */
  failures: number;
  /** True after the crash limit: auto-restart is off until the user retries. */
  disabled: boolean;
  bunPath: string;
}

export interface RelayHostProvider {
  id: string;
  name: string;
  npm: string;
  api: string;
  models: string[];
  listError: string;
  methods: RelaySignInMethod[];
  options: RelayPluginOption[];
  usage: boolean;
  checkin: boolean;
  single: boolean;
  signedIn: boolean;
  account: string;
  /** The package that declared this provider. */
  spec: string;
  /** The module export that produced it (a package can export several). */
  exportName: string;
  file: string;
}

export interface RelaySignInMethod {
  type: string;
  label: string;
  keyless?: boolean;
  placeholder?: string;
}

export interface RelayPluginOption {
  key: string;
  label: string;
  description: string;
  default: boolean;
}

export interface RelayHostInitResult {
  plugins: {
    spec: string;
    error: string;
    exports: string[];
  }[];
  exports: { spec: string; exportName: string; file: string }[];
  providers: RelayHostProvider[];
}

export interface RelayInstallPluginResult {
  plugin: RelayPlugin;
  providers: RelayHostProvider[];
  /** The host's own complaint about this package, if it made one. */
  hostError: string;
}

export interface RelaySignInResult {
  provider: string;
  account: string;
  /** Empty when the sign-in finished; otherwise the pending OAuth session. */
  session: string;
  url: string;
  instructions: string;
}

export const getRelayHostStatus = async (): Promise<RelayHostStatus> => {
  return invoke<RelayHostStatus>('relay_host_status');
};

export const restartRelayHost = async (): Promise<RelayHostStatus> => {
  return invoke<RelayHostStatus>('relay_host_restart');
};

export const stopRelayHost = async (): Promise<void> => {
  return invoke<void>('relay_host_stop');
};

/** What the live host process actually serves, not what SQLite remembers. */
export const listRelayHostProviders = async (): Promise<
  RelayHostProvider[]
> => {
  return invoke<RelayHostProvider[]>('relay_host_providers');
};

export const installRelayPlugin = async (
  spec: string,
): Promise<RelayInstallPluginResult> => {
  return invoke<RelayInstallPluginResult>('relay_install_plugin', {
    input: { spec },
  });
};

export const uninstallRelayPlugin = async (id: string): Promise<void> => {
  return invoke<void>('relay_uninstall_plugin', { id });
};

export const refreshRelayPlugins =
  async (): Promise<RelayHostInitResult> => {
    return invoke<RelayHostInitResult>('relay_refresh_plugins');
  };

export const signInRelayProvider = async (input: {
  provider: string;
  account?: string;
  method?: string;
  key?: string;
  inputs?: Record<string, unknown>;
}): Promise<RelaySignInResult> => {
  return invoke<RelaySignInResult>('relay_sign_in', {
    input: {
      provider: input.provider,
      account: input.account ?? '',
      method: input.method ?? '',
      key: input.key ?? '',
      inputs: input.inputs ?? {},
    },
  });
};

export const completeRelayOAuth = async (
  session: string,
  code: string,
): Promise<RelaySignInResult> => {
  return invoke<RelaySignInResult>('relay_oauth_callback', {
    input: { session, code },
  });
};

export const signOutRelayProvider = async (
  provider: string,
  account: string,
): Promise<void> => {
  return invoke<void>('relay_sign_out', { provider, account });
};
