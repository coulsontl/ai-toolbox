import { create } from 'zustand';
import {
  defaultSettings,
  getSettings,
  saveSettings,
  saveManualCliPath,
  setAutoLaunch,
  setKeepAwake,
  type AppSettings,
  type ProxyMode,
  type WebDAVConfig,
  type S3Config,
  type BackupCustomEntry,
  type BackupFileFilterRule,
  type SidebarPageKey,
  type SidebarHiddenByPage,
  normalizeSidebarHiddenByPage,
} from '@/services';
import {
  saveBackupSettings,
  type BackupRepositoryConfigFE,
  type BackupSettingsSaveOutcome,
} from '@/services/backupApi';
import {
  setCodexPreserveOfficialAuthOnSwitch as setCodexPreserveOfficialAuthOnSwitchApi,
  setCodexDaemonRestartMode as setCodexDaemonRestartModeApi,
} from '@/services/codexApi';
import type { CodexDaemonRestartMode } from '@/types/codex';
import { createKeepAwakeSettingsSlice, type KeepAwakeSettingsState } from './keepAwakeSettings';
import { backupSettingsStatePatch, buildLaunchOnStartupSettings } from './settingsStoreUtils';

// Re-export types for convenience (using camelCase for frontend)
export interface WebDAVConfigFE {
  url: string;
  username: string;
  password: string;
  remotePath: string;
  hostLabel: string;
}

export interface S3ConfigFE {
  accessKey: string;
  secretKey: string;
  bucket: string;
  region: string;
  prefix: string;
  endpointUrl: string;
  forcePathStyle: boolean;
  publicDomain: string;
}

export type BackupType = 'local' | 'webdav' | 'repository';

/** Payload for the unified backup settings save (all channels + encryption). */
export interface BackupSettingsFormValues {
  backupType: BackupType;
  localBackupPath: string;
  webdav: WebDAVConfigFE;
  backupEncryptionEnabled: boolean;
  /** Submit a new password; omit to keep the stored credential. */
  encryptionPassword?: string;
  repository: BackupRepositoryConfigFE;
  /** Empty keeps the stored token (platform switch still requires a new one). */
  repositoryToken?: string;
  backupImageAssetsEnabled: boolean;
  backupCliConfigFilesEnabled: boolean;
  backupCustomEntries: BackupCustomEntry[];
  backupFileFilterRules: BackupFileFilterRule[];
  autoBackupEnabled: boolean;
  autoBackupIntervalDays: number;
  autoBackupMaxKeep: number;
}

interface SettingsState extends KeepAwakeSettingsState {
  // Loading state
  isLoading: boolean;
  isInitialized: boolean;

  // Backup settings
  backupType: BackupType;
  localBackupPath: string;
  webdav: WebDAVConfigFE;
  lastBackupTime: string | null;
  backupImageAssetsEnabled: boolean;
  backupCliConfigFilesEnabled: boolean;
  backupCustomEntries: BackupCustomEntry[];
  backupFileFilterRules: BackupFileFilterRule[];
  /** Encryption switch persisted in settings; the password lives in the OS credential store. */
  backupEncryptionEnabled: boolean;

  // S3 storage settings
  s3: S3ConfigFE;

  // Window settings
  launchOnStartup: boolean;
  minimizeToTrayOnClose: boolean;
  startMinimized: boolean;
  startLightweight: boolean;
  lightweightOnClose: boolean;

  // Proxy settings
  proxyMode: ProxyMode;
  proxyUrl: string;

  // Auto backup settings
  autoBackupEnabled: boolean;
  autoBackupIntervalDays: number;
  autoBackupMaxKeep: number;
  lastAutoBackupTime: string | null;

  // Update settings
  autoCheckUpdate: boolean;
  receiveBetaUpdates: boolean;

  // Tab visibility settings
  visibleTabs: string[];

  // Sidebar visibility settings
  sidebarHiddenByPage: SidebarHiddenByPage;

  // OpenCode options
  opencodeAllowClearAppliedOhMyConfig: boolean;
  opencodeUseLegacyOhMyConfig: boolean;
  opencodeOmoUpgradeConfirmed: boolean;
  opencodeDualWriteReasoningVariant: boolean;

  // Codex options
  codexPreserveOfficialAuthOnSwitch: boolean;
  /** What to do after an apply while a Codex process still holds the old config. */
  codexDaemonRestartMode: CodexDaemonRestartMode;
  codexUnifiedSessionHistoryEnabled: boolean;

  // Claude Code options
  claudeCliLaunchFullAccess: boolean;
  /** Windows terminal preference for the Claude CLI launch: 'cmd' | 'powershell' | 'wt'. */
  preferredTerminal: string | null;

  // Manual CLI paths by command name (e.g. opencode, claude, grok, pi, omp, hermes, dsh, openclaw)
  cliManualPaths: Record<string, string>;

  // Actions
  initSettings: () => Promise<void>;
  /**
   * Unified backup settings save: patches only backup-related settings fields,
   * updates the repository connection record, and stores a newly submitted
   * encryption password. Throws on validation/save failure so the modal can keep
   * the draft and show the error.
   */
  saveBackupSettingsUnified: (
    config: BackupSettingsFormValues
  ) => Promise<BackupSettingsSaveOutcome>;
  setS3: (config: Partial<S3ConfigFE>) => Promise<void>;
  setLastBackupTime: (time: string | null) => Promise<void>;
  setLaunchOnStartup: (enabled: boolean) => Promise<void>;
  setMinimizeToTrayOnClose: (enabled: boolean) => Promise<void>;
  setStartMinimized: (enabled: boolean) => Promise<void>;
  setStartLightweight: (enabled: boolean) => Promise<void>;
  setLightweightOnClose: (enabled: boolean) => Promise<void>;
  setProxyMode: (mode: ProxyMode) => Promise<void>;
  setProxyUrl: (url: string) => Promise<void>;
  setLastAutoBackupTime: (time: string) => void;
  setAutoCheckUpdate: (enabled: boolean) => Promise<void>;
  setReceiveBetaUpdates: (enabled: boolean) => Promise<void>;
  setVisibleTabs: (tabs: string[]) => Promise<void>;
  setSidebarHidden: (page: SidebarPageKey, hidden: boolean) => Promise<void>;
  setOpencodeAllowClearAppliedOhMyConfig: (enabled: boolean) => Promise<void>;
  setOpencodeUseLegacyOhMyConfig: (enabled: boolean) => Promise<void>;
  setOpencodeOmoUpgradeConfirmed: (confirmed: boolean) => Promise<void>;
  setOpencodeDualWriteReasoningVariant: (enabled: boolean) => Promise<void>;
  setCodexPreserveOfficialAuthOnSwitch: (enabled: boolean) => Promise<void>;
  setCodexDaemonRestartMode: (mode: CodexDaemonRestartMode) => Promise<void>;
  setCodexUnifiedSessionHistoryEnabled: (enabled: boolean) => void;
  setClaudeCliLaunchFullAccess: (enabled: boolean) => Promise<void>;
  setPreferredTerminal: (terminal: string) => Promise<void>;
  setManualCliPath: (commandName: string, path: string) => Promise<string>;
}

// Convert backend snake_case to frontend camelCase
const toFrontendWebDAV = (webdav: WebDAVConfig): WebDAVConfigFE => ({
  url: webdav.url,
  username: webdav.username,
  password: webdav.password,
  remotePath: webdav.remote_path,
  hostLabel: webdav.host_label,
});

const toFrontendS3 = (s3: S3Config): S3ConfigFE => ({
  accessKey: s3.access_key,
  secretKey: s3.secret_key,
  bucket: s3.bucket,
  region: s3.region,
  prefix: s3.prefix,
  endpointUrl: s3.endpoint_url,
  forcePathStyle: s3.force_path_style,
  publicDomain: s3.public_domain,
});

// Convert frontend camelCase to backend snake_case
const toBackendS3 = (s3: S3ConfigFE): S3Config => ({
  access_key: s3.accessKey,
  secret_key: s3.secretKey,
  bucket: s3.bucket,
  region: s3.region,
  prefix: s3.prefix,
  endpoint_url: s3.endpointUrl,
  force_path_style: s3.forcePathStyle,
  public_domain: s3.publicDomain,
});

const defaultWebDAV: WebDAVConfigFE = {
  url: '',
  username: '',
  password: '',
  remotePath: '',
  hostLabel: '',
};

const defaultS3: S3ConfigFE = {
  accessKey: '',
  secretKey: '',
  bucket: '',
  region: '',
  prefix: '',
  endpointUrl: '',
  forcePathStyle: false,
  publicDomain: '',
};

export const useSettingsStore = create<SettingsState>()((set, get) => ({
  ...createKeepAwakeSettingsSlice(set, { getSettings, saveSettings, setKeepAwake }),
  isLoading: false,
  isInitialized: false,
  backupType: 'local' as BackupType,
  localBackupPath: '',
  webdav: defaultWebDAV,
  s3: defaultS3,
  lastBackupTime: null,
  backupImageAssetsEnabled: true,
  backupCliConfigFilesEnabled: true,
  backupCustomEntries: [],
  backupFileFilterRules: [],
  backupEncryptionEnabled: false,
  launchOnStartup: true,
  minimizeToTrayOnClose: true,
  startMinimized: false,
  startLightweight: false,
  lightweightOnClose: false,
  proxyMode: 'system',
  proxyUrl: '',
  autoBackupEnabled: false,
  autoBackupIntervalDays: 7,
  autoBackupMaxKeep: 10,
  lastAutoBackupTime: null,
  autoCheckUpdate: true,
  receiveBetaUpdates: false,
  visibleTabs: defaultSettings.visible_tabs,
  sidebarHiddenByPage: normalizeSidebarHiddenByPage(),
  opencodeAllowClearAppliedOhMyConfig: false,
  opencodeUseLegacyOhMyConfig: false,
  opencodeOmoUpgradeConfirmed: false,
  opencodeDualWriteReasoningVariant: false,
  codexPreserveOfficialAuthOnSwitch: false,
  codexDaemonRestartMode: 'prompt',
  codexUnifiedSessionHistoryEnabled: false,
  claudeCliLaunchFullAccess: false,
  preferredTerminal: null,
  cliManualPaths: {},

  initSettings: async () => {
    if (get().isInitialized) return;

    set({ isLoading: true });
    try {
      const settings = await getSettings();
      set({
        backupType: (['local', 'webdav', 'repository'].includes(settings.backup_type)
          ? settings.backup_type
          : 'local') as BackupType,
        localBackupPath: settings.local_backup_path,
        webdav: toFrontendWebDAV(settings.webdav),
        s3: toFrontendS3(settings.s3),
        lastBackupTime: settings.last_backup_time,
        backupImageAssetsEnabled: settings.backup_image_assets_enabled ?? true,
        backupCliConfigFilesEnabled: settings.backup_cli_config_files_enabled ?? true,
        backupCustomEntries: settings.backup_custom_entries ?? [],
        backupFileFilterRules: settings.backup_file_filter_rules ?? [],
        backupEncryptionEnabled: settings.backup_encryption?.enabled ?? false,
        launchOnStartup: settings.launch_on_startup,
        minimizeToTrayOnClose: settings.minimize_to_tray_on_close,
        startMinimized: settings.start_minimized ?? false,
        startLightweight: settings.start_lightweight ?? false,
        lightweightOnClose: settings.lightweight_on_close ?? false,
        keepComputerAwake: settings.keep_computer_awake ?? false,
        proxyMode: settings.proxy_mode ?? 'system',
        proxyUrl: settings.proxy_url || '',
        autoBackupEnabled: settings.auto_backup_enabled ?? false,
        autoBackupIntervalDays: settings.auto_backup_interval_days ?? 7,
        autoBackupMaxKeep: settings.auto_backup_max_keep ?? 10,
        lastAutoBackupTime: settings.last_auto_backup_time ?? null,
        autoCheckUpdate: settings.auto_check_update ?? true,
        receiveBetaUpdates: settings.receive_beta_updates ?? false,
        visibleTabs: settings.visible_tabs ?? defaultSettings.visible_tabs,
        sidebarHiddenByPage: normalizeSidebarHiddenByPage(settings.sidebar_hidden_by_page),
        opencodeAllowClearAppliedOhMyConfig: settings.opencode_allow_clear_applied_oh_my_config ?? false,
        opencodeUseLegacyOhMyConfig: settings.opencode_use_legacy_oh_my_config ?? false,
        opencodeOmoUpgradeConfirmed: settings.opencode_omo_upgrade_confirmed ?? false,
        opencodeDualWriteReasoningVariant: settings.opencode_dual_write_reasoning_variant ?? false,
        codexPreserveOfficialAuthOnSwitch: settings.codex_preserve_official_auth_on_switch ?? false,
        codexDaemonRestartMode: settings.codex_daemon_restart_mode ?? 'prompt',
        codexUnifiedSessionHistoryEnabled: settings.codex_unified_session_history_enabled ?? false,
        claudeCliLaunchFullAccess: settings.claude_cli_launch_full_access ?? false,
        preferredTerminal: settings.preferred_terminal ?? null,
        cliManualPaths: settings.cli_manual_paths ?? {},
        isInitialized: true,
      });
    } catch (error) {
      console.error('Failed to load settings:', error);
    } finally {
      set({ isLoading: false });
    }
  },

  saveBackupSettingsUnified: async (config) => {
    const outcome = await saveBackupSettings({
      backup_type: config.backupType,
      local_backup_path: config.localBackupPath,
      webdav: {
        url: config.webdav.url,
        username: config.webdav.username,
        password: config.webdav.password,
        remote_path: config.webdav.remotePath,
        host_label: config.webdav.hostLabel,
      },
      backup_encryption_enabled: config.backupEncryptionEnabled,
      encryption_password: config.encryptionPassword || undefined,
      repository: config.repository,
      repository_token: config.repositoryToken || undefined,
      backup_image_assets_enabled: config.backupImageAssetsEnabled,
      backup_cli_config_files_enabled: config.backupCliConfigFilesEnabled,
      backup_custom_entries: config.backupCustomEntries,
      backup_file_filter_rules: config.backupFileFilterRules,
      auto_backup_enabled: config.autoBackupEnabled,
      auto_backup_interval_days: config.autoBackupIntervalDays,
      auto_backup_max_keep: config.autoBackupMaxKeep,
    });

    // Reflect the saved state; concurrent settings writes are untouched because the
    // backend patches only backup-related fields. The shared patch helper keeps
    // every submitted field (including auto-backup parameters) in sync.
    set(backupSettingsStatePatch(config, outcome));
    return outcome;
  },

  setS3: async (config) => {
    const state = get();
    const newS3 = { ...state.s3, ...config };

    set({ s3: newS3 });

    // Get current settings and update
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      s3: toBackendS3(newS3),
    };
    await saveSettings(newSettings);
  },

  setLastBackupTime: async (time) => {
    set({ lastBackupTime: time });

    // Get current settings and update
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      last_backup_time: time,
    };
    await saveSettings(newSettings);
  },

  setLaunchOnStartup: async (enabled) => {
    const currentStartMinimized = get().startMinimized;
    set({
      launchOnStartup: enabled,
      startMinimized: enabled ? currentStartMinimized : false,
    });

    const currentSettings = await getSettings();
    const updatedSettings = buildLaunchOnStartupSettings(currentSettings, enabled);
    await saveSettings(updatedSettings);

    // Apply the OS-level side effect after the preference is persisted.
    await setAutoLaunch(enabled);
  },

  setMinimizeToTrayOnClose: async (enabled) => {
    set({ minimizeToTrayOnClose: enabled });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      minimize_to_tray_on_close: enabled,
    };
    await saveSettings(newSettings);
  },

  setStartMinimized: async (enabled) => {
    set({ startMinimized: enabled });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      start_minimized: enabled,
    };
    await saveSettings(newSettings);
  },

  setStartLightweight: async (enabled) => {
    set({ startLightweight: enabled });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      start_lightweight: enabled,
    };
    await saveSettings(newSettings);
  },

  setLightweightOnClose: async (enabled) => {
    set({ lightweightOnClose: enabled });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      lightweight_on_close: enabled,
    };
    await saveSettings(newSettings);
  },

  setProxyMode: async (mode) => {
    set({ proxyMode: mode });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      proxy_mode: mode,
    };
    await saveSettings(newSettings);
  },

  setProxyUrl: async (url) => {
    set({ proxyUrl: url });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      proxy_url: url,
    };
    await saveSettings(newSettings);
  },

  setLastAutoBackupTime: (time) => {
    set({ lastAutoBackupTime: time });
  },

  setAutoCheckUpdate: async (enabled) => {
    set({ autoCheckUpdate: enabled });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      auto_check_update: enabled,
    };
    await saveSettings(newSettings);
  },

  setReceiveBetaUpdates: async (enabled) => {
    set({ receiveBetaUpdates: enabled });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      receive_beta_updates: enabled,
    };
    await saveSettings(newSettings);
  },

  setVisibleTabs: async (tabs) => {
    set({ visibleTabs: tabs });

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      visible_tabs: tabs,
    };
    await saveSettings(newSettings);
  },

  setSidebarHidden: async (page, hidden) => {
    const currentVisibility = normalizeSidebarHiddenByPage(get().sidebarHiddenByPage);
    const nextVisibility = {
      ...currentVisibility,
      [page]: hidden,
    };

    set({ sidebarHiddenByPage: nextVisibility });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      sidebar_hidden_by_page: normalizeSidebarHiddenByPage({
        ...currentSettings.sidebar_hidden_by_page,
        [page]: nextVisibility[page],
      }),
    };
    await saveSettings(newSettings);
  },

  setOpencodeAllowClearAppliedOhMyConfig: async (enabled) => {
    set({ opencodeAllowClearAppliedOhMyConfig: enabled });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      opencode_allow_clear_applied_oh_my_config: enabled,
    };
    await saveSettings(newSettings);
  },

  setOpencodeUseLegacyOhMyConfig: async (enabled) => {
    set({ opencodeUseLegacyOhMyConfig: enabled });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      opencode_use_legacy_oh_my_config: enabled,
    };
    await saveSettings(newSettings);
  },

  setOpencodeOmoUpgradeConfirmed: async (confirmed) => {
    set({ opencodeOmoUpgradeConfirmed: confirmed });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      opencode_omo_upgrade_confirmed: confirmed,
    };
    await saveSettings(newSettings);
  },

  setOpencodeDualWriteReasoningVariant: async (enabled) => {
    set({ opencodeDualWriteReasoningVariant: enabled });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      opencode_dual_write_reasoning_variant: enabled,
    };
    await saveSettings(newSettings);
  },

  setCodexPreserveOfficialAuthOnSwitch: async (enabled) => {
    // Persist + re-project the currently applied Codex provider via the dedicated
    // backend command (direct apply, or restore → apply → re-engage under Gateway).
    const result = await setCodexPreserveOfficialAuthOnSwitchApi(enabled);
    set({ codexPreserveOfficialAuthOnSwitch: result.enabled });
  },

  setCodexDaemonRestartMode: async (mode) => {
    // Backend persists and echoes the value back; no optimistic flip, so a
    // rejected mode cannot leave the store disagreeing with the database.
    const result = await setCodexDaemonRestartModeApi(mode);
    set({ codexDaemonRestartMode: result.mode });
  },

  setCodexUnifiedSessionHistoryEnabled: (enabled) => {
    set({ codexUnifiedSessionHistoryEnabled: enabled });
  },

  setClaudeCliLaunchFullAccess: async (enabled) => {
    set({ claudeCliLaunchFullAccess: enabled });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      claude_cli_launch_full_access: enabled,
    };
    await saveSettings(newSettings);
  },

  setPreferredTerminal: async (terminal) => {
    set({ preferredTerminal: terminal });

    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      preferred_terminal: terminal,
    };
    await saveSettings(newSettings);
  },

  setManualCliPath: async (commandName, path) => {
    const version = await saveManualCliPath(commandName, path);

    const currentSettings = await getSettings();
    const cliManualPaths = { ...(currentSettings.cli_manual_paths ?? {}) };
    if (path.trim()) {
      cliManualPaths[commandName] = path.trim();
    } else {
      delete cliManualPaths[commandName];
    }
    set({ cliManualPaths });
    return version;
  },
}));
