/**
 * Plugin API Service
 *
 * Typed wrappers around the Rust plugin commands. Records cross the Tauri
 * boundary as camelCase JSON, so the interfaces below mirror the Rust DTOs
 * field for field; the provider/host payloads mirror the Bun host protocol.
 */

import { invoke } from '@tauri-apps/api/core';

export type PluginKind = 'provider' | 'middleware' | 'both' | 'page' | 'unknown';

/** One custom page a plugin's package declares. */
export interface PluginPageManifest {
  id: string;
  title: string;
  titles: Record<string, string>;
  entryFile: string;
  iconFile?: string | null;
  keepAlive: boolean;
  servicePort?: number | null;
  serviceOption?: string | null;
}

export interface InstalledPluginRecord {
  id: string;
  spec: string;
  packageName: string;
  displayName: string;
  version: string;
  kind: PluginKind;
  enabled: boolean;
  /** `npm` | `github` | `local`. */
  sourceType: string;
  marketId?: string | null;
  localPath: string;
  middlewareEntry: string;
  hasProviderEntry: boolean;
  providers: string[];
  options: unknown;
  summary: Record<string, string>;
  icon: string;
  installedAt: string;
  updatedAt: string;
  lastError?: string | null;
  /** Pages the plugin's package declares; absent for older records. */
  pages?: PluginPageManifest[];
  /** Reads the plugin package asked for; absent for older records. */
  permissions?: string[];
}

export interface MarketSourceRecord {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
  /** The built-in Magpie Community source; it can be disabled, never deleted. */
  builtin: boolean;
  pluginCount: number;
  lastSyncedAt?: string | null;
  lastError?: string | null;
}

export interface MarketPluginListing {
  package: string;
  name: string;
  icon: string;
  providers: string[];
  community: boolean;
  replaces?: string | null;
  kind: string;
  summary: Record<string, string>;
  marketIds: string[];
  installed: boolean;
  installedVersion?: string | null;
  latestVersion?: string | null;
  weeklyDownloads?: number | null;
  description: string;
  repository?: string | null;
  homepage?: string | null;
  license?: string | null;
}

export interface PluginSettings {
  /** Explicit Bun executable; empty means “find `bun` on PATH”. */
  bunManualPath: string;
  npmRegistry: string;
  followAppProxy: boolean;
  /** Explicit per-channel auto check-in switches keyed by provider id. */
  providerAutoCheckin?: Record<string, boolean>;
  /** Background auto-update opt-in; defaults to false. */
  autoUpdateEnabled?: boolean;
  /** Hours between automatic checks; the backend clamps to 1..=720. */
  autoUpdateIntervalHours?: number;
  /**
   * When the last full update check ran. Backend-owned: never send a value,
   * `savePluginSettings` clears it before invoking the command.
   */
  lastAutoUpdateCheckAt?: string | null;
}

/** One plugin's outcome from an update check or apply pass. */
export interface PluginUpdateItem {
  pluginId: string;
  packageName: string;
  displayName: string;
  currentVersion: string;
  latestVersion: string;
  /** `up_to_date` | `available` | `updated` | `skipped` | `failed`. */
  status: string;
  /** Skip reason, error text, or a short success note. */
  message: string;
}

export interface PluginUpdateSummary {
  checkedAt: string;
  updatedCount: number;
  availableCount: number;
  failedCount: number;
  items: PluginUpdateItem[];
}

export interface BunRuntimeStatus {
  available: boolean;
  path?: string | null;
  version?: string | null;
  /** `manual` | `managed` | `system`. */
  source?: string | null;
  expectedVersion: string;
  asset: string;
  managedPath: string;
  downloading: boolean;
  lastError?: string | null;
}

export interface BunPathProbe {
  ok: boolean;
  version?: string | null;
  error?: string | null;
}

export interface BunHostStatus {
  running: boolean;
  pid?: number | null;
  pendingRequests: number;
  crashes: number;
  circuitOpen: boolean;
  retryInMillis: number;
  lastError?: string | null;
  startedAt?: string | null;
  logEntries: number;
}

export interface PluginRuntimeLogEntry {
  seq: number;
  at: string;
  source: string;
  level: string;
  message: string;
}

export interface MiddlewareStateView {
  pluginId: string;
  spec: string;
  name: string;
  path: string;
  hooks: string[];
  /** `null` means every event; an empty list means no event is wanted. */
  events?: string[] | null;
  calls: number;
  failures: number;
  avgMicros: number;
  lastError?: string | null;
  loadError?: string | null;
  usageRows: number;
  usageDropped: number;
}

export interface PluginAccountSummary {
  /** Storage key: the provider id, or `id#<slot>`. */
  account: string;
  provider: string;
  /** `oauth` or `api`. */
  authType: string;
  hasAccess: boolean;
  hasRefresh: boolean;
  hasKey: boolean;
  /** Unix milliseconds from the auth file, when it records an expiry. */
  expiresAt?: number | null;
  displayName: string;
  enabled: boolean;
  sortIndex: number;
  /** Whether the host checks this account in once each local day. */
  autoCheckin?: boolean;
  /** Local day (`YYYY-MM-DD`) of the last scheduled check-in. */
  lastCheckinDay?: string | null;
  /** Outcome reported by the last scheduled check-in, when one ran. */
  lastCheckinOutcome?: string | null;
  /** When the last scheduled check-in ran. */
  lastCheckinAt?: string | null;
  /** Short message the host attached to the last scheduled check-in. */
  lastCheckinMessage?: string | null;
  /** Last persisted quota/usage snapshot for instant rendering before live refresh. */
  quota?: PluginUsage | null;
  /** Timestamp when `quota` was last persisted. */
  quotaUpdatedAt?: string | null;
}

export interface PluginAccountMetaRecord {
  id: string;
  pluginId: string;
  providerId: string;
  displayName: string;
  enabled: boolean;
  sortIndex: number;
  quota: unknown;
  checkin: unknown;
  updatedAt: string;
}

export interface PluginProviderOptionValue {
  providerId: string;
  options: unknown;
  updatedAt: string;
}

/** One sign-in method the plugin's `auth` hook declares. */
export interface PluginProviderMethod {
  type: string;
  label?: string;
  labels?: Record<string, string>;
  keyless?: boolean;
  keyAsked?: boolean;
  placeholder?: string;
}

/** One switch the plugin declares under `auth.options`. */
export interface PluginProviderOptionDeclaration {
  key: string;
  label: string;
  description: string;
  default: boolean;
  confirm: string;
  labels?: Record<string, string>;
  descriptions?: Record<string, string>;
  confirms?: Record<string, string>;
  filtersModels: boolean;
  placement?: string | null;
}

export interface PluginProviderModel {
  id: string;
  name: string;
  npm: string;
  url: string;
  apiId: string;
  context: number;
  input: number;
  output: number;
  reasoning: boolean;
  image: boolean;
  imageSaid: boolean;
  inputs: string[];
  inputsSaid: boolean;
  outputs: string[];
  outputsSaid: boolean;
  temperature?: boolean;
  released: string;
  cost?: unknown;
  variants: string[];
  free: boolean;
  rate: number;
  rateWas: number;
  fast: boolean;
  serviceTiers: string[];
  decides: boolean;
  searches: boolean;
  also: string[];
}

export interface PluginProviderAccount {
  key: string;
  type: string;
  accountId: string;
  hint: string;
  models?: string[];
  fellBack: boolean;
  listError: string;
}

/** One provider row returned by the plugin host's `providers` method. */
export interface PluginProviderRow {
  id: string;
  spec: string;
  name: string;
  npm: string;
  api: string;
  methods: PluginProviderMethod[];
  single: boolean;
  importable?: boolean;
  importFrom?: string;
  importFroms?: Record<string, string>;
  cli?: { name: string; manual?: string };
  icon: string;
  usage: boolean;
  checkin: boolean;
  calls: boolean;
  renews: boolean;
  renewLead: number;
  rotating: boolean;
  options: PluginProviderOptionDeclaration[];
  /** Persisted per-provider option switches; missing entries fall back to `options[].default`. */
  optionValues?: Record<string, boolean>;
  /** Channel-level daily auto check-in switch resolved from the backend. */
  autoCheckin?: boolean;
  signedIn: boolean;
  authType: string;
  accountId: string;
  fellBack: boolean;
  listError: string;
  accounts: PluginProviderAccount[];
  models: PluginProviderModel[];
}

/** One CLI tab a plugin provider can be added to. */
export interface PluginCliTarget {
  cliKey: string;
  label: string;
  mode: 'gateway_takeover' | 'direct_plugin_route';
  supported: boolean;
  reason?: string | null;
}

export interface AddPluginProviderToCliInput {
  cliKey: string;
  pluginSpec: string;
  providerId: string;
  displayName?: string;
  accountStrategy?: 'auto_rotate' | 'single';
  accountKey?: string | null;
  modelIds?: string[];
}

export interface AddPluginProviderToCliResult {
  cliKey: string;
  providerId: string;
  providerName: string;
  mode: 'gateway_takeover' | 'direct_plugin_route';
  endpointUrl: string;
  modelCount: number;
  gatewayRunning: boolean;
}

export interface SyncPluginModelsResult {
  cliKey: string;
  cliProviderId: string;
  modelCount: number;
  added: number;
  updated: number;
}

export interface PluginLoginPromptOption {
  label: string;
  value: string;
  hint: string;
  labels?: Record<string, string>;
  hints?: Record<string, string>;
}

export interface PluginLoginPrompt {
  type: string;
  key: string;
  message: string;
  messages?: Record<string, string>;
  placeholder: string;
  options?: PluginLoginPromptOption[];
}

export interface PluginLoginPromptResponse {
  prompt: PluginLoginPrompt | null;
}

export interface PluginLoginValidateResponse {
  error: string | null;
}

export interface PluginLoginAuthorizeResult {
  ok: boolean;
  error?: string;
  session?: string;
  url?: string;
  instructions?: string;
  method?: string;
  opened?: boolean;
}

export interface PluginLoginCompletion {
  ok: boolean;
  error?: string;
  provider?: string;
  account?: string;
}

export interface PluginImportResult {
  status: string;
  user: string;
  error?: string;
  provider?: string;
  account?: string;
}

export interface PluginImportFilesResponse {
  results: PluginImportResult[];
}

export interface PluginUsageWindow {
  name: string;
  /** Percent used (0–100), `null` when the plugin did not say. */
  used: number | null;
  resetsAt: string;
  resetSecs: number;
  display: string;
  amount: number;
  limit: number;
  unit: string;
  span: number;
  model: string;
  models: string[];
  notModels: string[];
  family: string;
  pool: string;
  aside: boolean;
  runsOutAt: string;
  runsOut: number;
  credits: boolean;
  balance: number | null;
}

export interface PluginUsageResetList {
  count: number;
  until: string;
  byWindow: boolean;
  fiveHour: number;
  weekly: number;
  team: boolean;
  each: Array<{ until: string; window: string }>;
}

export interface PluginUsage {
  plan: string;
  until: string;
  /** `auto` | `off` | empty. */
  renew: string;
  balance: string;
  error: string;
  user: string;
  /** `expired` | `kept` | `renewed` | empty. */
  signIn: string;
  resets: PluginUsageResetList | null;
  windows: PluginUsageWindow[];
}

export interface PluginCheckinResult {
  /** `claimed` | `done` | `ineligible` | `inactive` | `captcha` | `failed`. */
  outcome: string;
  credit: number;
  streak: number;
  message: string;
  until: string;
}

export interface PluginAccountCheckResult {
  models: string[];
  usage: PluginUsage | null;
  refused: string;
}

/** One page of an installed, enabled plugin, joined with its plugin metadata. */
export interface RegisteredPluginPage {
  pluginId: string;
  pluginName: string;
  packageName: string;
  pluginVersion: string;
  pluginIcon: string;
  pageId: string;
  title: string;
  titles: Record<string, string>;
  iconDataUrl?: string | null;
  keepAlive: boolean;
  servicePort?: number | null;
  serviceOption?: string | null;
  permissions: string[];
}

/** What the host needs to mount one page: a URL the sandbox may load. */
export interface PluginPageSession {
  pluginId: string;
  pageId: string;
  url: string;
  serviceUrl?: string | null;
  options: Record<string, unknown>;
  permissions: string[];
  keepAlive: boolean;
}

// ---- installed plugins -----------------------------------------------------

export const listInstalledPlugins = (): Promise<InstalledPluginRecord[]> =>
  invoke<InstalledPluginRecord[]>('plugins_list_installed');

export const installPlugin = (
  spec: string,
  marketId?: string | null,
): Promise<InstalledPluginRecord> =>
  invoke<InstalledPluginRecord>('plugins_install', { spec, marketId: marketId ?? null });

export const uninstallPlugin = (
  id: string,
  options?: { purgeData?: boolean },
): Promise<void> =>
  invoke<void>('plugins_uninstall', { id, purgeData: options?.purgeData ?? false });

export const setPluginEnabled = (id: string, enabled: boolean): Promise<InstalledPluginRecord> =>
  invoke<InstalledPluginRecord>('plugins_set_enabled', { id, enabled });

export const setPluginOptions = (id: string, options: unknown): Promise<InstalledPluginRecord> =>
  invoke<InstalledPluginRecord>('plugins_set_options', { id, options });

// ---- markets ---------------------------------------------------------------

export const listPluginMarkets = (): Promise<MarketSourceRecord[]> =>
  invoke<MarketSourceRecord[]>('plugins_list_markets');

export const addPluginMarket = (name: string, url: string): Promise<MarketSourceRecord> =>
  invoke<MarketSourceRecord>('plugins_add_market', { name, url });

export const updatePluginMarket = (
  id: string,
  updates: { name?: string | null; url?: string | null; enabled?: boolean | null },
): Promise<MarketSourceRecord> =>
  invoke<MarketSourceRecord>('plugins_update_market', {
    id,
    name: updates.name ?? null,
    url: updates.url ?? null,
    enabled: updates.enabled ?? null,
  });

export const removePluginMarket = (id: string): Promise<boolean> =>
  invoke<boolean>('plugins_remove_market', { id });

export const refreshPluginMarkets = (): Promise<MarketSourceRecord[]> =>
  invoke<MarketSourceRecord[]>('plugins_refresh_markets');

export const getPluginMarketListings = (filters?: {
  search?: string | null;
  kind?: string | null;
  marketId?: string | null;
  includeNpmSearch?: boolean;
}): Promise<MarketPluginListing[]> =>
  invoke<MarketPluginListing[]>('plugins_market_listings', {
    search: filters?.search ?? null,
    kind: filters?.kind ?? null,
    marketId: filters?.marketId ?? null,
    includeNpmSearch: filters?.includeNpmSearch ?? false,
  });

// ---- settings and Bun runtime ----------------------------------------------

export const getPluginSettings = (): Promise<PluginSettings> =>
  invoke<PluginSettings>('plugins_get_settings');

/**
 * Persist plugin settings. `lastAutoUpdateCheckAt` is backend-owned, so a stale
 * snapshot must never be written back; the store keeps the DB value regardless.
 */
export const savePluginSettings = (settings: PluginSettings): Promise<PluginSettings> =>
  invoke<PluginSettings>('plugins_save_settings', {
    settings: { ...settings, lastAutoUpdateCheckAt: null },
  });

/** Desktop check: a full check records `lastAutoUpdateCheckAt`. */
export const checkPluginUpdates = (pluginId?: string): Promise<PluginUpdateSummary> =>
  invoke<PluginUpdateSummary>('plugins_check_updates', {
    pluginId: pluginId ?? null,
    recordCheckTime: true,
  });

export const updatePlugins = (pluginId?: string): Promise<PluginUpdateSummary> =>
  invoke<PluginUpdateSummary>('plugins_update_plugins', { pluginId: pluginId ?? null });

export const getPluginBunStatus = (): Promise<BunRuntimeStatus> =>
  invoke<BunRuntimeStatus>('plugins_bun_status');

export const ensurePluginBun = (): Promise<BunRuntimeStatus> =>
  invoke<BunRuntimeStatus>('plugins_bun_ensure');

export const probePluginBunPath = (path: string): Promise<BunPathProbe> =>
  invoke<BunPathProbe>('plugins_bun_probe_path', { path });

// ---- host and middleware runtime -------------------------------------------

export const getPluginHostStatus = (): Promise<BunHostStatus> =>
  invoke<BunHostStatus>('plugins_host_status');

export const restartPluginHost = (): Promise<BunHostStatus> =>
  invoke<BunHostStatus>('plugins_host_restart');

export const getPluginRuntimeLogs = (limit = 200): Promise<PluginRuntimeLogEntry[]> =>
  invoke<PluginRuntimeLogEntry[]>('plugins_runtime_logs', { limit });

export const clearPluginRuntimeLogs = (): Promise<void> =>
  invoke<void>('plugins_clear_runtime_logs');

export const getPluginMiddlewareStates = (): Promise<MiddlewareStateView[]> =>
  invoke<MiddlewareStateView[]>('plugins_middleware_states');

// ---- providers and accounts ------------------------------------------------

export const listPluginProviders = (): Promise<PluginProviderRow[]> =>
  invoke<PluginProviderRow[]>('plugins_list_providers');

export const refreshPluginProviderModels = (): Promise<PluginProviderRow[]> =>
  invoke<PluginProviderRow[]>('plugins_refresh_provider_models');

/** Every CLI tab a provider can be handed to, with its transport mode. */
export const listPluginCliTargets = (): Promise<PluginCliTarget[]> =>
  invoke<PluginCliTarget[]>('plugins_list_cli_targets');

/** Write one plugin provider into a CLI's own provider list. */
export const addPluginProviderToCli = (
  input: AddPluginProviderToCliInput,
): Promise<AddPluginProviderToCliResult> =>
  invoke<AddPluginProviderToCliResult>('plugins_add_provider_to_cli', { input });

/** Merge the plugin's current model catalog into an already-added CLI provider. */
export const syncPluginCliProviderModels = (
  cliKey: string,
  cliProviderId: string,
  pluginProviderId: string,
): Promise<SyncPluginModelsResult> =>
  invoke<SyncPluginModelsResult>('plugins_sync_cli_provider_models', {
    cliKey,
    cliProviderId,
    pluginProviderId,
  });

export const loginPluginPrompt = (
  provider: string,
  method?: string | null,
  inputs?: Record<string, unknown> | null,
): Promise<PluginLoginPromptResponse> =>
  invoke<PluginLoginPromptResponse>('plugins_login_prompt', {
    provider,
    method: method ?? null,
    inputs: inputs ?? {},
  });

export const loginPluginValidate = (
  provider: string,
  method: string | null,
  key: string,
  value: unknown,
): Promise<PluginLoginValidateResponse> =>
  invoke<PluginLoginValidateResponse>('plugins_login_validate', {
    provider,
    method,
    key,
    value,
  });

export const loginPluginAuthorize = (
  provider: string,
  method?: string | null,
  inputs?: Record<string, unknown> | null,
  account?: string | null,
  proxy?: string | null,
): Promise<PluginLoginAuthorizeResult> =>
  invoke<PluginLoginAuthorizeResult>('plugins_login_authorize', {
    provider,
    method: method ?? null,
    inputs: inputs ?? {},
    account: account ?? null,
    proxy: proxy ?? null,
  });

export const loginPluginCallback = (
  session: string,
  code?: string | null,
): Promise<PluginLoginCompletion> =>
  invoke<PluginLoginCompletion>('plugins_login_callback', { session, code: code ?? null });

export const loginPluginApiKey = (
  provider: string,
  method?: string | null,
  inputs?: Record<string, unknown> | null,
  key?: string | null,
  account?: string | null,
  proxy?: string | null,
): Promise<PluginLoginCompletion> =>
  invoke<PluginLoginCompletion>('plugins_login_api_key', {
    provider,
    method: method ?? null,
    inputs: inputs ?? {},
    key: key ?? null,
    account: account ?? null,
    proxy: proxy ?? null,
  });

export const loginPluginImportFiles = (
  provider: string,
  texts: string[],
  proxy?: string | null,
): Promise<PluginImportFilesResponse> =>
  invoke<PluginImportFilesResponse>('plugins_login_import_files', {
    provider,
    texts,
    proxy: proxy ?? null,
  });

export const cancelPluginLogin = (): Promise<number> =>
  invoke<number>('plugins_login_cancel');

export const listPluginAccounts = (): Promise<PluginAccountSummary[]> =>
  invoke<PluginAccountSummary[]>('plugins_list_accounts');

export const signOutPluginAccount = (
  provider: string,
  account?: string | null,
): Promise<unknown> =>
  invoke<unknown>('plugins_account_sign_out', { provider, account: account ?? null });

export const checkPluginAccount = (
  provider: string,
  account?: string | null,
  proxy?: string | null,
): Promise<PluginAccountCheckResult> =>
  invoke<PluginAccountCheckResult>('plugins_account_check', {
    provider,
    account: account ?? null,
    proxy: proxy ?? null,
  });

export const getPluginAccountUsage = (
  provider: string,
  account?: string | null,
  proxy?: string | null,
): Promise<PluginUsage> =>
  invoke<PluginUsage>('plugins_account_usage', {
    provider,
    account: account ?? null,
    proxy: proxy ?? null,
  });

export const checkinPluginAccount = (
  provider: string,
  account?: string | null,
  proxy?: string | null,
): Promise<PluginCheckinResult> =>
  invoke<PluginCheckinResult>('plugins_account_checkin', {
    provider,
    account: account ?? null,
    proxy: proxy ?? null,
  });

export const setPluginAccountEnabled = (input: {
  account: string;
  pluginId: string;
  provider?: string | null;
  displayName?: string | null;
  enabled: boolean;
}): Promise<PluginAccountMetaRecord> =>
  invoke<PluginAccountMetaRecord>('plugins_account_set_enabled', {
    account: input.account,
    pluginId: input.pluginId,
    provider: input.provider ?? null,
    displayName: input.displayName ?? null,
    enabled: input.enabled,
  });

export const setPluginAccountAutoCheckin = (input: {
  account: string;
  pluginId: string;
  provider?: string | null;
  enabled: boolean;
}): Promise<PluginAccountMetaRecord> =>
  invoke<PluginAccountMetaRecord>('plugins_account_set_auto_checkin', {
    account: input.account,
    pluginId: input.pluginId,
    provider: input.provider ?? null,
    enabled: input.enabled,
  });

export const setPluginProviderOption = (
  providerId: string,
  options: Record<string, boolean>,
): Promise<PluginProviderOptionValue> =>
  invoke<PluginProviderOptionValue>('plugins_set_provider_option', { providerId, options });

/** Toggle the daily auto check-in switch of a whole channel. */
export const setPluginProviderAutoCheckin = (
  providerId: string,
  enabled: boolean,
): Promise<boolean> =>
  invoke<boolean>('plugins_set_provider_auto_checkin', { providerId, enabled });

// ---- custom plugin pages ---------------------------------------------------

export const listPluginPages = (): Promise<RegisteredPluginPage[]> =>
  invoke<RegisteredPluginPage[]>('plugins_list_pages');

export const openPluginPage = (
  pluginId: string,
  pageId?: string,
): Promise<PluginPageSession> =>
  invoke<PluginPageSession>('plugins_open_page', { pluginId, pageId });

/** Release the host-side token a page session holds. */
export const closePluginPage = (pluginId: string, pageId: string): Promise<void> =>
  invoke<void>('plugins_close_page', { pluginId, pageId });

/** One read a page's package declared, performed by the Rust host. */
export const readPluginPageBridge = (
  pluginId: string,
  scope: string,
  params?: Record<string, unknown>,
): Promise<unknown> =>
  invoke<unknown>('plugins_page_bridge_read', { pluginId, scope, params });

/** Persist a page's option changes; answers with the stored options. */
export const setPluginPageBridgeOptions = (
  pluginId: string,
  options: Record<string, unknown>,
): Promise<Record<string, unknown>> =>
  invoke<Record<string, unknown>>('plugins_page_bridge_set_options', { pluginId, options });

export const copyPluginPageImage = (dataBase64: string): Promise<void> =>
  invoke<void>('plugins_page_bridge_copy_image', { dataBase64 });

export const savePluginPageImage = (
  dataBase64: string,
  filename?: string,
): Promise<string> =>
  invoke<string>('plugins_page_bridge_save_image', { dataBase64, filename });

export type PluginPageKvOp = 'get' | 'set' | 'delete' | 'list' | 'clear';

export interface PluginPageKvInput {
  pluginId: string;
  op: PluginPageKvOp;
  key?: string;
  value?: unknown;
  prefix?: string;
}

export type PluginPageFilesOp =
  | 'readText'
  | 'writeText'
  | 'readBytes'
  | 'writeBytes'
  | 'list'
  | 'stat'
  | 'delete';

export interface PluginPageFilesInput {
  pluginId: string;
  op: PluginPageFilesOp;
  path?: string;
  content?: string;
}

/** One lightweight key/value call for a plugin page (permission checked in Rust). */
export const invokePluginPageKv = (input: PluginPageKvInput): Promise<unknown> =>
  invoke<unknown>('plugins_page_bridge_kv', { ...input });

/** One sandboxed file call for a plugin page (permission checked in Rust). */
export const invokePluginPageFiles = (input: PluginPageFilesInput): Promise<unknown> =>
  invoke<unknown>('plugins_page_bridge_files', { ...input });
