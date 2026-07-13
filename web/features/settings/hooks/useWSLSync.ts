/**
 * WSL Sync Hook
 *
 * Manages WSL sync configuration and operations.
 *
 * NOTE: This hook is used by both MainLayout (for status indicator) and
 * WSLSyncModal (for configuration UI). Event listeners are set up only once
 * using module-level state to avoid duplicate API calls.
 */

import { useState, useEffect, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import type {
  WSLSyncConfig,
  WSLStatusResult,
  SyncResult,
  FileMapping,
  WSLDetectResult,
  SyncProgress,
  WslDirectModuleStatus,
} from '@/types/wslsync';
import {
  wslGetConfig,
  wslSaveConfig,
  wslSync,
  wslGetStatus,
  wslDetect,
  wslCheckDistro,
  wslGetDefaultMappings,
} from '@/services/wslSyncApi';
import { useSettingsStore } from '@/stores';

// Module-level state to share across hook instances and avoid duplicate listeners
let globalConfig: WSLSyncConfig | null = null;
let globalStatus: WSLStatusResult | null = null;
let globalLoading = false;
let globalLoadError: string | null = null;
let globalSyncWarning: string | null = null;
let globalSyncProgress: SyncProgress | null = null;
const configListeners = new Set<(config: WSLSyncConfig | null) => void>();
const statusListeners = new Set<(status: WSLStatusResult | null) => void>();
const loadingListeners = new Set<(loading: boolean) => void>();
const loadErrorListeners = new Set<(error: string | null) => void>();
const syncWarningListeners = new Set<(warning: string | null) => void>();
const syncProgressListeners = new Set<(progress: SyncProgress | null) => void>();
let listenersSetup = false;
let configLoadPromise: Promise<void> | null = null;
let statusLoadPromise: Promise<void> | null = null;
let configRequestSeq = 0;
let statusRequestSeq = 0;

// Map visibleTabs keys to sync module keys
const TAB_TO_MODULE: Record<string, string> = {
  opencode: 'opencode',
  claudecode: 'claude',
  codex: 'codex',
  grok: 'grok',
  openclaw: 'openclaw',
  geminicli: 'geminicli',
  pi: 'pi',
};
const ALL_CODING_MODULES = ['opencode', 'claude', 'codex', 'grok', 'geminicli', 'openclaw', 'pi'];

const notify = <T,>(listeners: Set<(value: T) => void>, value: T) => {
  listeners.forEach((listener) => listener(value));
};

const setGlobalConfig = (value: WSLSyncConfig | null) => {
  globalConfig = value;
  notify(configListeners, value);
};

const setGlobalStatus = (value: WSLStatusResult | null) => {
  globalStatus = value;
  notify(statusListeners, value);
};

const setGlobalLoading = (value: boolean) => {
  globalLoading = value;
  notify(loadingListeners, value);
};

const setGlobalLoadError = (value: string | null) => {
  globalLoadError = value;
  notify(loadErrorListeners, value);
};

const setGlobalSyncWarning = (value: string | null) => {
  globalSyncWarning = value;
  notify(syncWarningListeners, value);
};

const setGlobalSyncProgress = (value: SyncProgress | null) => {
  globalSyncProgress = value;
  notify(syncProgressListeners, value);
};

const loadConfigShared = async (force = false) => {
  if (configLoadPromise && !force) {
    return configLoadPromise;
  }

  const requestId = ++configRequestSeq;
  const promise = (async () => {
    try {
      setGlobalLoading(true);
      setGlobalLoadError(null);
      const data = await wslGetConfig();

      if (requestId !== configRequestSeq) {
        return;
      }

      // If no file mappings exist, initialize with defaults
      if (!data.fileMappings || data.fileMappings.length === 0) {
        const defaultMappings = await wslGetDefaultMappings();
        const defaultConfig: WSLSyncConfig = {
          ...data,
          fileMappings: defaultMappings,
        };
        await wslSaveConfig(defaultConfig);
        if (requestId === configRequestSeq) {
          setGlobalConfig(defaultConfig);
        }
      } else {
        setGlobalConfig(data);
      }
    } catch (error) {
      console.error('Failed to load WSL config:', error);
      if (requestId === configRequestSeq) {
        setGlobalLoadError(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (requestId === configRequestSeq) {
        setGlobalLoading(false);
        configLoadPromise = null;
      }
    }
  })();

  configLoadPromise = promise;
  return promise;
};

const loadStatusShared = async (force = false) => {
  if (statusLoadPromise && !force) {
    return statusLoadPromise;
  }

  const requestId = ++statusRequestSeq;
  const promise = (async () => {
    try {
      setGlobalLoadError(null);
      const data = await wslGetStatus();
      if (requestId === statusRequestSeq) {
        setGlobalStatus(data);
      }
    } catch (error) {
      console.error('Failed to load WSL status:', error);
      if (requestId === statusRequestSeq) {
        setGlobalLoadError(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (requestId === statusRequestSeq) {
        statusLoadPromise = null;
      }
    }
  })();

  statusLoadPromise = promise;
  return promise;
};

const setupWslEventListeners = () => {
  if (listenersSetup) return;
  listenersSetup = true;

  listen('wsl-config-changed', () => {
    loadConfigShared(true);
    loadStatusShared(true);
  }).catch((error) => {
    listenersSetup = false;
    console.error('Failed to listen to WSL config changes:', error);
  });

  listen<SyncResult>('wsl-sync-completed', () => {
    loadStatusShared(true);
    setGlobalSyncProgress(null);
  }).catch((error) => {
    listenersSetup = false;
    console.error('Failed to listen to WSL sync completion:', error);
  });

  listen<string>('wsl-sync-warning', (event) => {
    setGlobalSyncWarning(event.payload);
  }).catch((error) => {
    listenersSetup = false;
    console.error('Failed to listen to WSL sync warnings:', error);
  });

  listen<SyncProgress>('wsl-sync-progress', (event) => {
    setGlobalSyncProgress(event.payload);
  }).catch((error) => {
    listenersSetup = false;
    console.error('Failed to listen to WSL sync progress:', error);
  });
};

export function useWSLSync() {
  const [config, setConfig] = useState<WSLSyncConfig | null>(globalConfig);
  const [status, setStatus] = useState<WSLStatusResult | null>(globalStatus);
  const [loading, setLoading] = useState(globalLoading);
  const [syncing, setSyncing] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(globalLoadError);
  const [syncWarning, setSyncWarning] = useState<string | null>(globalSyncWarning);
  const [syncProgress, setSyncProgress] = useState<SyncProgress | null>(globalSyncProgress);

  // Subscribe to global config/status changes
  useEffect(() => {
    configListeners.add(setConfig);
    statusListeners.add(setStatus);
    loadingListeners.add(setLoading);
    loadErrorListeners.add(setLoadError);
    syncWarningListeners.add(setSyncWarning);
    syncProgressListeners.add(setSyncProgress);
    // Sync with current global state
    setConfig(globalConfig);
    setStatus(globalStatus);
    setLoading(globalLoading);
    setLoadError(globalLoadError);
    setSyncWarning(globalSyncWarning);
    setSyncProgress(globalSyncProgress);
    return () => {
      configListeners.delete(setConfig);
      statusListeners.delete(setStatus);
      loadingListeners.delete(setLoading);
      loadErrorListeners.delete(setLoadError);
      syncWarningListeners.delete(setSyncWarning);
      syncProgressListeners.delete(setSyncProgress);
    };
  }, []);

  /**
   * Load WSL sync configuration
   */
  const loadConfig = useCallback(async () => {
    await loadConfigShared();
  }, []);

  /**
   * Load WSL sync status
   */
  const loadStatus = useCallback(async () => {
    await loadStatusShared();
  }, []);

  /**
   * Save WSL sync configuration
   */
  const saveConfig = useCallback(async (newConfig: WSLSyncConfig) => {
    try {
      setGlobalLoading(true);
      await wslSaveConfig(newConfig);
      setGlobalConfig(newConfig);
      await Promise.all([loadConfigShared(true), loadStatusShared(true)]);
    } catch (error) {
      console.error('Failed to save WSL config:', error);
      throw error;
    } finally {
      setGlobalLoading(false);
    }
  }, []);

  /**
   * Execute sync operation
   */
  const sync = useCallback(async (module?: string) => {
    try {
      setSyncing(true);
      setGlobalSyncProgress(null); // Clear previous progress
      // Compute skip modules from visibleTabs
      const { visibleTabs } = useSettingsStore.getState();
      const visibleModules = visibleTabs
        .map((k) => TAB_TO_MODULE[k])
        .filter(Boolean);
      const wslDirectModules = (config?.moduleStatuses || [])
        .filter((item) => item.isWslDirect)
        .map((item) => item.module);
      const skipModules = ALL_CODING_MODULES.filter(
        (m) => !visibleModules.includes(m) || wslDirectModules.includes(m)
      );
      const result = await wslSync(module, skipModules.length > 0 ? skipModules : undefined);
      await loadStatusShared(true);
      return result;
    } catch (error) {
      console.error('Failed to sync:', error);
      throw error;
    } finally {
      setSyncing(false);
      setGlobalSyncProgress(null); // Clear progress when done
    }
  }, [config?.moduleStatuses]);

  /**
   * Detect WSL availability
   */
  const detect = useCallback(async (): Promise<WSLDetectResult> => {
    try {
      return await wslDetect();
    } catch (error) {
      console.error('Failed to detect WSL:', error);
      throw error;
    }
  }, []);

  /**
   * Check if a specific distro is available
   */
  const checkDistro = useCallback(async (distro: string) => {
    try {
      return await wslCheckDistro(distro);
    } catch (error) {
      console.error('Failed to check distro:', error);
      throw error;
    }
  }, []);

  /**
   * Get default file mappings
   */
  const getDefaultMappings = useCallback(async (): Promise<FileMapping[]> => {
    try {
      return await wslGetDefaultMappings();
    } catch (error) {
      console.error('Failed to get default mappings:', error);
      throw error;
    }
  }, []);

  /**
   * Initialize default configuration if not exists
   */
  const initializeDefaultConfig = useCallback(async () => {
    try {
      setGlobalLoading(true);
      const defaultMappings = await getDefaultMappings();
      const defaultConfig: WSLSyncConfig = {
        enabled: false,
        distro: 'Ubuntu',
        syncMcp: true,
        syncSkills: true,
        fileMappings: defaultMappings,
        moduleStatuses: [],
        lastSyncStatus: 'never',
      };
      await wslSaveConfig(defaultConfig);
      setGlobalConfig(defaultConfig);
    } catch (error) {
      console.error('Failed to initialize default config:', error);
      throw error;
    } finally {
      setGlobalLoading(false);
    }
  }, [getDefaultMappings]);

  // Load config and status on mount (only for first instance)
  useEffect(() => {
    setupWslEventListeners();
    if (globalConfig === null) {
      loadConfig();
    }
    if (globalStatus === null) {
      loadStatus();
    }
  }, [loadConfig, loadStatus]);

  /**
   * Dismiss sync warning
   */
  const dismissSyncWarning = useCallback(() => {
    setGlobalSyncWarning(null);
  }, []);

  return {
    config,
    status,
    loading,
    syncing,
    loadError,
    syncWarning,
    syncProgress,
    moduleStatuses: config?.moduleStatuses || status?.moduleStatuses || ([] as WslDirectModuleStatus[]),
    loadConfig,
    loadStatus,
    saveConfig,
    sync,
    detect,
    checkDistro,
    getDefaultMappings,
    initializeDefaultConfig,
    dismissSyncWarning,
  };
}
