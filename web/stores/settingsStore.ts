import { create } from 'zustand';
import {
  getSettings,
  saveSettings,
  setAutoLaunch,
  type AppSettings,
  type WebDAVConfig,
  type S3Config,
} from '@/services';

// Re-export types for convenience (using camelCase for frontend)
export interface WebDAVConfigFE {
  url: string;
  username: string;
  password: string;
  remotePath: string;
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

interface SettingsState {
  // Loading state
  isLoading: boolean;
  isInitialized: boolean;

  // Backup settings
  backupType: 'local' | 'webdav';
  localBackupPath: string;
  webdav: WebDAVConfigFE;
  lastBackupTime: string | null;

  // S3 storage settings
  s3: S3ConfigFE;

  // Window settings
  launchOnStartup: boolean;
  minimizeToTrayOnClose: boolean;

  // Proxy settings
  proxyUrl: string;

  // Actions
  initSettings: () => Promise<void>;
  setBackupSettings: (config: {
    backupType: 'local' | 'webdav';
    localBackupPath?: string;
    webdav?: Partial<WebDAVConfigFE>;
  }) => Promise<void>;
  setS3: (config: Partial<S3ConfigFE>) => Promise<void>;
  setLastBackupTime: (time: string | null) => Promise<void>;
  setLaunchOnStartup: (enabled: boolean) => Promise<void>;
  setMinimizeToTrayOnClose: (enabled: boolean) => Promise<void>;
  setProxyUrl: (url: string) => Promise<void>;
}

// Convert backend snake_case to frontend camelCase
const toFrontendWebDAV = (webdav: WebDAVConfig): WebDAVConfigFE => ({
  url: webdav.url,
  username: webdav.username,
  password: webdav.password,
  remotePath: webdav.remote_path,
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
const toBackendWebDAV = (webdav: WebDAVConfigFE): WebDAVConfig => ({
  url: webdav.url,
  username: webdav.username,
  password: webdav.password,
  remote_path: webdav.remotePath,
});

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
  isLoading: false,
  isInitialized: false,
  backupType: 'local',
  localBackupPath: '',
  webdav: defaultWebDAV,
  s3: defaultS3,
  lastBackupTime: null,
  launchOnStartup: true,
  minimizeToTrayOnClose: true,
  proxyUrl: '',

  initSettings: async () => {
    if (get().isInitialized) return;

    set({ isLoading: true });
    try {
      const settings = await getSettings();
      set({
        backupType: (settings.backup_type as 'local' | 'webdav') || 'local',
        localBackupPath: settings.local_backup_path,
        webdav: toFrontendWebDAV(settings.webdav),
        s3: toFrontendS3(settings.s3),
        lastBackupTime: settings.last_backup_time,
        launchOnStartup: settings.launch_on_startup,
        minimizeToTrayOnClose: settings.minimize_to_tray_on_close,
        proxyUrl: settings.proxy_url || '',
        isInitialized: true,
      });
    } catch (error) {
      console.error('Failed to load settings:', error);
    } finally {
      set({ isLoading: false });
    }
  },

  setBackupSettings: async (config) => {
    const state = get();
    const newWebdav = config.webdav
      ? { ...state.webdav, ...config.webdav }
      : state.webdav;
    const newLocalPath = config.localBackupPath ?? state.localBackupPath;

    set({
      backupType: config.backupType,
      localBackupPath: newLocalPath,
      webdav: newWebdav,
    });

    // Get current settings and update
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      backup_type: config.backupType,
      local_backup_path: newLocalPath,
      webdav: toBackendWebDAV(newWebdav),
    };
    await saveSettings(newSettings);
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
    set({ launchOnStartup: enabled });

    // Update system auto-launch
    await setAutoLaunch(enabled);

    // Update database
    const currentSettings = await getSettings();
    const newSettings: AppSettings = {
      ...currentSettings,
      launch_on_startup: enabled,
    };
    await saveSettings(newSettings);
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
}));
