import React from 'react';
import {
  Button,
  Empty,
  Space,
  Typography,
  message,
  Spin,
  Collapse,
  Alert,
  Modal,
} from 'antd';
import {
  PlusOutlined,
  FolderOpenOutlined,
  EyeOutlined,
  EditOutlined,
  ReloadOutlined,
  LinkOutlined,
  EllipsisOutlined,
  DatabaseOutlined,
  RobotOutlined,
  SettingOutlined,
  MoreOutlined,
  ImportOutlined,
  ThunderboltOutlined,
  MessageOutlined,
  EnvironmentOutlined,
  ToolOutlined,
  GlobalOutlined,
  CheckSquareOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { listen } from '@tauri-apps/api/event';
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';

import {
  readOpenClawConfigWithResult,
  saveOpenClawConfig,
  getOpenClawConfigPathInfo,
  backupOpenClawConfig,
  getOpenClawAgentsDefaults,
  getOpenClawEnv,
  getOpenClawTools,
  scanOpenClawConfigHealth,
  openOpenClawWebUi,
  launchOpenClawGateway,
} from '@/services/openclawApi';
import {
  listFavoriteProviders,
  upsertFavoriteProvider,
  type OpenCodeDiagnosticsConfig,
  type OpenCodeFavoriteProvider,
} from '@/services/opencodeApi';
import { useProviderSharing } from '@/features/coding/shared/providerShare';
import { findPresetModelById } from '@/constants/presetModels';
import {
  buildFetchedOpenClawModel,
} from '../utils/openClawFetchedModels';
import { applyOpenClawUserAgent } from '../utils/providerHeaders';
import { refreshTrayMenu, hasAllApiHubExtension } from '@/services/appApi';
import type {
  OpenClawConfig,
  OpenClawConfigPathInfo,
  OpenClawProviderConfig,
  OpenClawModel,
  OpenClawAgentsDefaults,
  OpenClawEnvConfig,
  OpenClawHealthWarning,
  OpenClawToolsConfig,
} from '@/types/openclaw';
import type { OpenCodeProvider } from '@/types/opencode';

import JsonEditor from '@/components/common/JsonEditor';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchModelsApplyResult } from '@/components/common/FetchModelsModal/types';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import ConnectivityTestModal from '@/features/coding/opencode/components/ConnectivityTestModal';
import OpenClawProviderCard from '../components/OpenClawProviderCard';
import OpenClawProviderFormModal, {
  type ProviderFormValues,
} from '../components/OpenClawProviderFormModal';
import OpenClawModelFormModal, {
  type ModelFormValues,
} from '../components/OpenClawModelFormModal';
import ImportFromOpenCodeModal, {
  type ImportedProvider,
} from '../components/ImportFromOpenCodeModal';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import AgentsDefaultsCard, { type AgentsDefaultsCardRef } from '../components/AgentsDefaultsCard';
import EnvCard from '../components/EnvCard';
import ToolsCard from '../components/ToolsCard';
import OpenClawHealthBanner from '../components/OpenClawHealthBanner';
import OpenClawConfigPathModal from '../components/OpenClawConfigPathModal';
import { useRefreshStore } from '@/stores';
import { useSettingsStore } from '@/stores';
import type { OpenClawAllApiHubProvider } from '@/services/openclawApi';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import CliManualPathSetting from '@/components/common/CliManualPathSetting';
import {
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import {
  buildFavoriteProviderOptions,
  buildFavoriteProviderStorageKey,
  findDefaultTestModelIdForProvider,
  findDiagnosticsForProvider,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  mergeDiagnosticsIntoFavoriteProviders,
  type OpenClawFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import {
  PROVIDER_SORT_MODES_BASIC,
  backupProvidersBeforeDelete,
  ProviderBatchToolbar,
  ProviderSearchEmpty,
  ProviderSearchInput,
  ProviderSortDropdown,
  filterProviderItems,
  sortProviderItems,
  useProviderBatchSelection,
  useProviderListSort,
} from '@/features/coding/shared/providerList';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import { extractOpenClawProviderFromCcSwitch } from '../utils/importMapping';

import styles from './OpenClawPage.module.less';

const { Title, Text, Link } = Typography;

const OPENAI_COMPATIBLE_NPM = '@ai-sdk/openai-compatible';
const OPENAI_RESPONSES_NPM = '@ai-sdk/openai';

/**
 * Map OpenClaw `api` protocol (+ optional baseUrl hint) to OpenCode `npm` SDK name.
 * Used by FetchModelsModal and ConnectivityTestModal.
 */
const apiToNpm = (api?: string, baseUrl?: string): string => {
  // Explicit protocol match
  if (api === 'openai-completions') return OPENAI_COMPATIBLE_NPM;
  if (api === 'openai-responses') return OPENAI_RESPONSES_NPM;
  if (api === 'anthropic-messages') return '@ai-sdk/anthropic';
  if (api === 'google-generative-ai') return '@ai-sdk/google';

  // Infer from baseUrl (covers old imports without api, or generic api like openai-completions)
  const url = (baseUrl || '').toLowerCase();
  if (url.includes('anthropic')) return '@ai-sdk/anthropic';
  if (url.includes('generativelanguage.googleapis.com') || url.includes('google')) return '@ai-sdk/google';

  return '@ai-sdk/openai-compatible';
};

/**
 * Convert OpenClawProviderConfig to OpenCodeProvider shape for ConnectivityTestModal.
 */
const toOpenCodeProvider = (cfg: OpenClawProviderConfig): OpenCodeProvider => ({
  npm: apiToNpm(cfg.api, cfg.baseUrl),
  options: {
    baseURL: cfg.baseUrl || '',
    apiKey: cfg.apiKey,
  },
  models: Object.fromEntries(
    (cfg.models || []).map((m) => [m.id, { name: m.name || m.id }]),
  ),
});

const buildOpenClawFavoriteProviderConfig = (
  providerId: string,
  config: OpenClawProviderConfig,
): OpenCodeProvider =>
  buildFavoriteProviderOptions(toOpenCodeProvider(config), {
    providerId,
    config,
  } satisfies OpenClawFavoriteProviderPayload);

const OpenClawPage: React.FC = () => {
  const { t } = useTranslation();
  const { shareProvider, shareModal } = useProviderSharing('openclaw', () => loadConfig());
  const { openClawConfigRefreshKey } = useRefreshStore();
  const {
    sidebarHiddenByPage,
    setSidebarHidden,
  } = useSettingsStore();

  // Drag sensors
  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'openclaw-agents',
      title: t('openclaw.agents.title'),
      order: 1,
    },
    {
      id: 'openclaw-providers',
      title: t('openclaw.providers.title'),
      order: 2,
    },
    {
      id: 'openclaw-env',
      title: t('openclaw.env.title'),
      order: 3,
    },
    {
      id: 'openclaw-tools',
      title: t('openclaw.tools.title'),
      order: 4,
    },
    {
      id: 'openclaw-other',
      title: t('openclaw.other.title'),
      order: 5,
    },
    {
      id: 'openclaw-session-manager',
      title: t('sessionManager.title'),
      order: 6,
    },
  ], [t]);

  // Loading & config state
  const [loading, setLoading] = React.useState(false);
  const [config, setConfig] = React.useState<OpenClawConfig | null>(null);
  const [configPathInfo, setConfigPathInfo] = React.useState<OpenClawConfigPathInfo | null>(null);
  const [parseError, setParseError] = React.useState<{
    path: string;
    error: string;
    contentPreview?: string;
  } | null>(null);

  // Section data
  const [agentsDefaults, setAgentsDefaults] = React.useState<OpenClawAgentsDefaults | null>(null);
  const [env, setEnv] = React.useState<OpenClawEnvConfig | null>(null);
  const [tools, setTools] = React.useState<OpenClawToolsConfig | null>(null);
  const [healthWarnings, setHealthWarnings] = React.useState<OpenClawHealthWarning[]>([]);

  // Modal states
  const [previewOpen, setPreviewOpen] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const sidebarHidden = sidebarHiddenByPage.openclaw;
  const [configPathModalOpen, setConfigPathModalOpen] = React.useState(false);
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<{
    id: string;
    config: OpenClawProviderConfig;
  } | null>(null);
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [editingModel, setEditingModel] = React.useState<OpenClawModel | null>(null);
  const [modelTargetProvider, setModelTargetProvider] = React.useState<string>('');
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [favoriteImportModalOpen, setFavoriteImportModalOpen] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [batchDeleteProviderId, setBatchDeleteProviderId] = React.useState<string | null>(null);
  const [selectedModelIdsByProvider, setSelectedModelIdsByProvider] = React.useState<Record<string, string[]>>({});
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string>('');
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityProviderId, setConnectivityProviderId] = React.useState<string>('');
  const [favoriteProviders, setFavoriteProviders] = React.useState<OpenCodeFavoriteProvider[]>([]);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);
  // Collapse states
  const [providersCollapsed, setProvidersCollapsed] = React.useState(false);
  const [agentsCollapsed, setAgentsCollapsed] = React.useState(false);
  const [envCollapsed, setEnvCollapsed] = React.useState(true);
  const [toolsCollapsed, setToolsCollapsed] = React.useState(true);
  const [otherCollapsed, setOtherCollapsed] = React.useState(true);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);

  // Refs
  const agentsDefaultsRef = React.useRef<AgentsDefaultsCardRef>(null);
  const otherConfigJsonValidRef = React.useRef(true);

  // ================================================================
  // Data loading
  // ================================================================
  const loadConfig = React.useCallback(async (silent = false) => {
    try {
      setLoading(true);
      setParseError(null);

      const [pathInfo, result, warnings] = await Promise.all([
        getOpenClawConfigPathInfo(),
        readOpenClawConfigWithResult(),
        scanOpenClawConfigHealth(),
      ]);

      setConfigPathInfo(pathInfo);
      setHealthWarnings(result.status === 'parseError' ? [] : warnings);

      switch (result.status) {
        case 'success':
          setConfig(result.config);
          break;
        case 'notFound':
          setConfig(null);
          break;
        case 'parseError':
          setParseError({
            path: result.path,
            error: result.error,
            contentPreview: result.contentPreview,
          });
          setConfig(null);
          break;
        case 'error':
          if (!silent) {
            message.error(result.error);
          }
          setConfig(null);
          break;
      }
    } catch (error) {
      console.error('Failed to load config:', error);
      if (!silent) {
        message.error(t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  const loadSectionData = React.useCallback(async () => {
    try {
      const [defaults, envData, toolsData] = await Promise.all([
        getOpenClawAgentsDefaults(),
        getOpenClawEnv(),
        getOpenClawTools(),
      ]);
      setAgentsDefaults(defaults);
      setEnv(envData);
      setTools(toolsData);
    } catch (error) {
      console.error('Failed to load section data:', error);
    }
  }, []);

  const loadFavoriteProviders = React.useCallback(async () => {
    try {
      const allFavoriteProviders = await listFavoriteProviders();
      setFavoriteProviders(allFavoriteProviders.filter((provider) => isFavoriteProviderForSource('openclaw', provider)));
    } catch (error) {
      console.error('Failed to load OpenClaw favorite providers:', error);
    }
  }, []);

  React.useEffect(() => {
    void openClawConfigRefreshKey;
    loadConfig();
    loadSectionData();
  }, [loadConfig, loadSectionData, openClawConfigRefreshKey]);

  React.useEffect(() => {
    loadFavoriteProviders();
  }, [loadFavoriteProviders]);

  React.useEffect(() => {
    void openClawConfigRefreshKey;
    const checkAllApiHubAvailability = async () => {
      try {
        const available = await hasAllApiHubExtension();
        setAllApiHubAvailable(available);
      } catch (error) {
        console.error('Failed to check All API Hub availability for OpenClaw:', error);
        setAllApiHubAvailable(false);
      }
    };

    checkAllApiHubAvailability();

    const checkCcSwitchAvailability = async () => {
      try {
        setCcSwitchAvailable(await hasCcSwitchDb());
      } catch (error) {
        console.error('Failed to check CC Switch availability for OpenClaw:', error);
        setCcSwitchAvailable(false);
      }
    };

    checkCcSwitchAvailability();
  }, [openClawConfigRefreshKey]);

  // Listen for config-changed events (from tray)
  React.useEffect(() => {
    const unlisten = listen('openclaw-config-changed', () => {
      loadConfig(true);
      loadSectionData();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [loadConfig, loadSectionData]);

  // ================================================================
  // Provider CRUD
  // ================================================================
  const providerEntries = React.useMemo(() => {
    if (!config?.models?.providers) return [];
    return Object.entries(config.models.providers);
  }, [config]);

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('openclaw');
  const [providerKeyword, setProviderKeyword] = React.useState('');
  // Search and non-custom sort modes bypass the config file order, so
  // dragging would write a stale custom order — dnd is only enabled in
  // custom mode.
  const providerDragDisabled = sortMode !== 'custom' || providerKeyword.trim() !== '';
  const visibleProviderEntries = React.useMemo(
    () =>
      sortProviderItems(
        filterProviderItems(providerEntries, providerKeyword, ([providerId, providerConfig]) => [
          providerId,
          providerConfig.baseUrl ?? '',
          ...(providerConfig.models ?? []).flatMap((model) => [model.id, model.name ?? '']),
        ]),
        sortMode,
        { name: ([providerId]) => providerId },
        ([providerId]) => lastUsedAt(providerId),
      ),
    [providerEntries, providerKeyword, sortMode, lastUsedAt],
  );

  const handleAddProvider = () => {
    setEditingProvider(null);
    setProviderModalOpen(true);
  };

  // OpenCode import entry removed; keep open/import handlers for later restore.
  const _handleOpenImportModal = () => {
    setProviderModalOpen(false);
    setImportModalOpen(true);
  };
  void _handleOpenImportModal;

  const handleImportFromOpenCode = async (imported: ImportedProvider[]) => {
    try {
      const currentConfig = config || { models: { providers: {} } };
      const providers = { ...(currentConfig.models?.providers || {}) };

      for (const item of imported) {
        providers[item.providerId] = item.config;
      }

      const newConfig: OpenClawConfig = {
        ...currentConfig,
        models: {
          ...(currentConfig.models || {}),
          providers,
        },
      };

      await saveOpenClawConfig(newConfig);
      for (const item of imported) {
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('openclaw', item.providerId),
            buildOpenClawFavoriteProviderConfig(item.providerId, item.config),
          );
        } catch (favoriteError) {
          console.error('Failed to save OpenClaw favorite provider from OpenCode import:', favoriteError);
        }
      }
      message.success(
        t('openclaw.providers.importSuccess', { count: imported.length }),
      );
      setImportModalOpen(false);
      loadConfig();
      loadSectionData();
      loadFavoriteProviders();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from OpenCode:', error);
      message.error(t('common.error'));
    }
  };

  const handleImportFromAllApiHub = async (imported: OpenClawAllApiHubProvider[]) => {
    try {
      const currentConfig = config || { models: { providers: {} } };
      const providers = { ...(currentConfig.models?.providers || {}) };

      for (const item of imported) {
        providers[item.providerId] = item.config;
      }

      const newConfig: OpenClawConfig = {
        ...currentConfig,
        models: {
          ...(currentConfig.models || {}),
          providers,
        },
      };

      await saveOpenClawConfig(newConfig);
      for (const item of imported) {
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('openclaw', item.providerId),
            buildOpenClawFavoriteProviderConfig(item.providerId, item.config),
          );
        } catch (favoriteError) {
          console.error('Failed to save OpenClaw favorite provider from All API Hub import:', favoriteError);
        }
      }
      message.success(
        t('openclaw.providers.importAllApiHubSuccess', { count: imported.length }),
      );
      setAllApiHubImportModalOpen(false);
      loadConfig();
      loadSectionData();
      loadFavoriteProviders();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from All API Hub:', error);
      message.error(t('common.error'));
    }
  };

  const handleImportFromCcSwitch = async (imported: CcSwitchProviderCandidate[]) => {
    try {
      const currentConfig = config || { models: { providers: {} } };
      const providers = { ...(currentConfig.models?.providers || {}) };

      const toImport = imported.filter((candidate) =>
        !Object.prototype.hasOwnProperty.call(providers, candidate.providerId)
          && extractOpenClawProviderFromCcSwitch(candidate),
      );

      let ok = 0;
      let fail = 0;
      for (const candidate of toImport) {
        const providerConfig = extractOpenClawProviderFromCcSwitch(candidate);
        if (!providerConfig) {
          continue;
        }
        try {
          providers[candidate.providerId] = providerConfig;
          ok += 1;
        } catch (error) {
          console.error('Failed to map CC Switch provider for OpenClaw:', candidate.name, error);
          fail += 1;
        }
      }

      if (ok > 0) {
        const newConfig: OpenClawConfig = {
          ...currentConfig,
          models: {
            ...(currentConfig.models || {}),
            providers,
          },
        };
        await saveOpenClawConfig(newConfig);
        for (const candidate of toImport) {
          const providerConfig = extractOpenClawProviderFromCcSwitch(candidate);
          if (providerConfig) {
            try {
              await upsertFavoriteProvider(
                buildFavoriteProviderStorageKey('openclaw', candidate.providerId),
                buildOpenClawFavoriteProviderConfig(candidate.providerId, providerConfig),
              );
            } catch (favoriteError) {
              console.error('Failed to save OpenClaw favorite provider from CC Switch import:', favoriteError);
            }
          }
        }
      }

      setCcSwitchImportModalOpen(false);
      if (ok > 0 && fail === 0) {
        message.success(t('common.ccSwitch.importSuccess', { count: ok }));
      } else if (ok > 0 && fail > 0) {
        message.warning(t('common.ccSwitch.importPartial', { ok, fail }));
      } else if (fail > 0) {
        message.error(t('common.error'));
      }

      loadConfig();
      loadSectionData();
      loadFavoriteProviders();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from CC Switch:', error);
      message.error(t('common.error'));
    }
  };

  const handleEditProvider = (providerId: string, providerConfig: OpenClawProviderConfig) => {
    setEditingProvider({ id: providerId, config: providerConfig });
    setProviderModalOpen(true);
  };

  const handleImportFavoriteProviders = React.useCallback(async (providersToImport: OpenCodeFavoriteProvider[]) => {
    try {
      const currentConfig = config || { models: { providers: {} } };
      const nextProviders = { ...(currentConfig.models?.providers || {}) };
      let importedCount = 0;

      for (const favoriteProvider of providersToImport) {
        const payload = getFavoriteProviderPayload<OpenClawFavoriteProviderPayload>(favoriteProvider);
        if (!payload) {
          continue;
        }

        nextProviders[payload.providerId] = payload.config as OpenClawProviderConfig;
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('openclaw', payload.providerId),
            buildOpenClawFavoriteProviderConfig(
              payload.providerId,
              payload.config as OpenClawProviderConfig,
            ),
            favoriteProvider.diagnostics,
          );
        } catch (favoriteError) {
          console.error('Failed to copy OpenClaw favorite provider diagnostics during import:', favoriteError);
        }
        importedCount += 1;
      }

      const nextConfig: OpenClawConfig = {
        ...currentConfig,
        models: {
          ...(currentConfig.models || {}),
          providers: nextProviders,
        },
      };

      await saveOpenClawConfig(nextConfig);
      setFavoriteImportModalOpen(false);
      message.success(t('opencode.provider.importSuccess', { count: importedCount }));
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import OpenClaw favorite providers:', error);
      message.error(t('common.error'));
    }
  }, [config, loadConfig, loadSectionData, t]);

  const handleDeleteProvider = async (providerId: string) => {
    if (!config) return;
    const provider = config.models?.providers?.[providerId];
    if (!provider) return;

    const performDelete = async () => {
      try {
        const newProviders = { ...(config.models?.providers || {}) };
        delete newProviders[providerId];

        const newConfig: OpenClawConfig = {
          ...config,
          models: {
            ...(config.models || {}),
            providers: newProviders,
          },
        };

        await saveOpenClawConfig(newConfig);
        await loadFavoriteProviders();
        message.success(t('common.success'));
        loadConfig();
        loadSectionData();
        refreshTrayMenu();
      } catch (error) {
        console.error('Failed to delete provider:', error);
        message.error(t('common.error'));
      }
    };

    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('openclaw', providerId),
        buildOpenClawFavoriteProviderConfig(providerId, provider),
      );
      await performDelete();
    } catch (favoriteError) {
      console.error('Failed to preserve OpenClaw favorite provider before deletion:', favoriteError);
      Modal.confirm({
        title: t('common.deleteWithoutBackupTitle'),
        content: t('common.deleteWithoutBackupContent'),
        okText: t('common.continueDelete'),
        cancelText: t('common.cancel'),
        onOk: async () => {
          await performDelete();
        },
      });
    }
  };

  const handleBatchDeleteProviders = React.useCallback(
    async (ids: string[]): Promise<boolean> => {
      if (!config) return false;
      const defaultProviderId = agentsDefaults?.model?.primary?.split('/')[0];
      const currentProviders = config.models?.providers ?? {};
      const deletableIds = ids.filter(
        (id) => id !== defaultProviderId && currentProviders[id],
      );
      if (deletableIds.length === 0) return false;
      try {
        await backupProvidersBeforeDelete(
          deletableIds,
          (id) => upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('openclaw', id),
            buildOpenClawFavoriteProviderConfig(id, currentProviders[id]),
          ),
          (id) => t('common.batch.backupFailed', { name: id }),
        );
        const newProviders = { ...(config.models?.providers || {}) };
        for (const id of deletableIds) {
          delete newProviders[id];
        }
        await saveOpenClawConfig({
          ...config,
          models: { ...(config.models || {}), providers: newProviders },
        });
        await loadFavoriteProviders();
        await loadConfig();
        await loadSectionData();
        await refreshTrayMenu();
        message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig();
        await loadFavoriteProviders();
        return false;
      }
    },
    [config, agentsDefaults, loadFavoriteProviders, loadConfig, loadSectionData, refreshTrayMenu, t],
  );

  // Selectable ids exclude providers that can't be deleted (e.g. the default model's provider).
  const batchSelectableIds = React.useMemo(
    () =>
      visibleProviderEntries
        .filter(([providerId]) => agentsDefaults?.model?.primary?.split('/')[0] !== providerId)
        .map(([providerId]) => providerId),
    [visibleProviderEntries, agentsDefaults],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  const handleProviderSubmit = async (values: ProviderFormValues) => {
    try {
      const currentConfig = config || {
        models: { providers: {} },
      };
      const providers = { ...(currentConfig.models?.providers || {}) };

      const providerConfig: OpenClawProviderConfig = editingProvider
        ? { ...editingProvider.config }
        : { models: [] };

      if (values.baseUrl) providerConfig.baseUrl = values.baseUrl;
      else delete providerConfig.baseUrl;
      if (values.apiKey) providerConfig.apiKey = values.apiKey;
      else delete providerConfig.apiKey;
      if (values.api) providerConfig.api = values.api;
      else delete providerConfig.api;

      providers[values.providerId] = applyOpenClawUserAgent(
        providerConfig,
        Boolean(values.userAgent),
      );

      const newConfig: OpenClawConfig = {
        ...currentConfig,
        models: {
          ...(currentConfig.models || {}),
          providers,
        },
      };

      await saveOpenClawConfig(newConfig);
      try {
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('openclaw', values.providerId),
          buildOpenClawFavoriteProviderConfig(values.providerId, providerConfig),
        );
        await loadFavoriteProviders();
      } catch (favoriteError) {
        console.error('Failed to save OpenClaw favorite provider:', favoriteError);
      }
      message.success(t('common.success'));
      setProviderModalOpen(false);
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save provider:', error);
      message.error(t('common.error'));
    }
  };

  // ================================================================
  // Model CRUD
  // ================================================================
  const handleAddModel = (providerId: string) => {
    setModelTargetProvider(providerId);
    setEditingModel(null);
    setModelModalOpen(true);
  };

  const handleEditModel = (providerId: string, model: OpenClawModel) => {
    setModelTargetProvider(providerId);
    setEditingModel(model);
    setModelModalOpen(true);
  };

  const handleDeleteModel = async (providerId: string, modelId: string) => {
    if (!config?.models?.providers?.[providerId]) return;
    try {
      const provider = { ...config.models.providers[providerId] };
      provider.models = (provider.models || []).filter((m) => m.id !== modelId);

      const newConfig: OpenClawConfig = {
        ...config,
        models: {
          ...config.models,
          providers: {
            ...config.models.providers,
            [providerId]: provider,
          },
        },
      };

      await saveOpenClawConfig(newConfig);
      try {
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('openclaw', providerId),
          buildOpenClawFavoriteProviderConfig(providerId, provider),
        );
        await loadFavoriteProviders();
      } catch (favoriteError) {
        console.error('Failed to update OpenClaw favorite provider after deleting model:', favoriteError);
      }
      message.success(t('common.success'));
      loadConfig();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to delete model:', error);
      message.error(t('common.error'));
    }
  };

  const handleToggleBatchDeleteMode = (providerId: string) => {
    setBatchDeleteProviderId((prev) => {
      const next = prev === providerId ? null : providerId;
      // 清理涉及到的 provider 的勾选:关闭时清当前,切换到另一个 provider 时清前一个,
      // 否则旧 provider 的勾选会残留,重入时被静默预选导致误删本次会话从未选中的模型。
      const keysToClean = next === null ? [providerId] : prev && prev !== providerId ? [prev] : [];
      if (keysToClean.length > 0) {
        setSelectedModelIdsByProvider((selected) => {
          const copy = { ...selected };
          keysToClean.forEach((k) => delete copy[k]);
          return copy;
        });
      }
      return next;
    });
  };

  const handleToggleModelSelection = (providerId: string, modelId: string, selected: boolean) => {
    setSelectedModelIdsByProvider((prev) => {
      const current = prev[providerId] ?? [];
      if (selected) {
        return { ...prev, [providerId]: current.includes(modelId) ? current : [...current, modelId] };
      }
      return { ...prev, [providerId]: current.filter((id) => id !== modelId) };
    });
  };

  const handleBatchDeleteModels = async (providerId: string) => {
    if (!config?.models?.providers?.[providerId]) {
      return;
    }
    const provider = config.models.providers[providerId];
    const selected = selectedModelIdsByProvider[providerId] ?? [];
    if (selected.length === 0) {
      return;
    }
    const selectedSet = new Set(selected);
    const nextProvider = {
      ...provider,
      models: (provider.models || []).filter((model) => !selectedSet.has(model.id)),
    };
    try {
      const newConfig: OpenClawConfig = {
        ...config,
        models: {
          ...config.models,
          providers: {
            ...config.models.providers,
            [providerId]: nextProvider,
          },
        },
      };
      await saveOpenClawConfig(newConfig);
      try {
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('openclaw', providerId),
          buildOpenClawFavoriteProviderConfig(providerId, nextProvider),
        );
        await loadFavoriteProviders();
      } catch (favoriteError) {
        console.error('Failed to update OpenClaw favorite provider after batch delete:', favoriteError);
      }
      message.success(t('openclaw.providers.batchDeleteSuccess', { count: selected.length }));
      loadConfig();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to batch delete models:', error);
      message.error(t('common.error'));
    } finally {
      setBatchDeleteProviderId(null);
      setSelectedModelIdsByProvider((prev) => {
        const copy = { ...prev };
        delete copy[providerId];
        return copy;
      });
    }
  };

  const handleModelSubmit = async (values: ModelFormValues) => {
    if (!config?.models?.providers?.[modelTargetProvider]) return;
    try {
      const provider = { ...config.models.providers[modelTargetProvider] };
      const models = [...(provider.models || [])];

      // Build new model from form values + extra params (from JSON editor).
      // Preserve `input` and `alias` from the original model if they existed,
      // since these known fields don't have dedicated form controls.
      const preserved: Partial<OpenClawModel> = {};
      if (editingModel?.input) preserved.input = editingModel.input;
      if (editingModel?.alias) preserved.alias = editingModel.alias;

      const newModel: OpenClawModel = {
        ...preserved,
        id: values.id,
        name: values.name || undefined,
        contextWindow: values.contextWindow,
        maxTokens: values.maxTokens,
        reasoning: values.reasoning || undefined,
        cost:
          values.costInput !== undefined || values.costOutput !== undefined
            ? {
              input: values.costInput || 0,
              output: values.costOutput || 0,
              cacheRead: values.costCacheRead,
              cacheWrite: values.costCacheWrite,
            }
            : undefined,
        ...(values.extraParams || {}),
      };

      if (editingModel) {
        const idx = models.findIndex((m) => m.id === editingModel.id);
        if (idx >= 0) models[idx] = newModel;
      } else {
        models.push(newModel);
      }

      provider.models = models;

      const newConfig: OpenClawConfig = {
        ...config,
        models: {
          ...config.models,
          providers: {
            ...config.models.providers,
            [modelTargetProvider]: provider,
          },
        },
      };

      await saveOpenClawConfig(newConfig);
      try {
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('openclaw', modelTargetProvider),
          buildOpenClawFavoriteProviderConfig(modelTargetProvider, provider),
        );
        await loadFavoriteProviders();
      } catch (favoriteError) {
        console.error('Failed to update OpenClaw favorite provider after saving model:', favoriteError);
      }
      message.success(t('common.success'));
      setModelModalOpen(false);
      loadConfig();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save model:', error);
      message.error(t('common.error'));
    }
  };

  // ================================================================
  // Connectivity Test & Fetch Models
  // ================================================================
  const handleOpenConnectivityTest = (providerId: string) => {
    setConnectivityProviderId(providerId);
    setConnectivityModalOpen(true);
  };

  const handleBatchTestProviders = React.useCallback(async () => {
    if (providerEntries.length === 0) {
      return;
    }

    const targets = providerEntries.map(([providerId, providerConfig]) =>
      buildProviderConnectivityBatchTarget(
        {
          providerId,
          providerName: providerId,
          providerConfig: toOpenCodeProvider(providerConfig),
          modelIds: (providerConfig.models || []).map((model) => model.id),
        },
        {
          requireBaseUrl: true,
          requireApiKey: true,
          preferredModelId: findDefaultTestModelIdForProvider(favoriteProviders, 'openclaw', providerId),
          errorMessages: {
            missingBaseUrl: t('common.baseUrlMissing'),
            missingApiKey: t('common.apiKeyMissing'),
            missingModel: t('common.modelMissing'),
          },
        },
      ),
    );

    setConnectivityStatuses(
      Object.fromEntries(
        providerEntries.map(([providerId]) => [
          providerId,
          { status: 'running' as const },
        ]),
      ),
    );
    setBatchTestingProviders(true);

    try {
      await runProviderConnectivityBatch(targets, (providerId, status) => {
        const nextStatus = status.status === 'success'
          ? {
              ...status,
              tooltipMessage: status.totalMs !== undefined
                ? t('common.connectivityBatchSuccessWithTiming', {
                    model: status.modelId || t('common.notSet'),
                    totalMs: status.totalMs,
                  })
                : t('common.connectivityBatchSuccess', {
                    model: status.modelId || t('common.notSet'),
                  }),
            }
          : status;
        setConnectivityStatuses((previousStatuses) => ({
          ...previousStatuses,
          [providerId]: nextStatus,
        }));
      });
    } catch (error) {
      console.error('Failed to batch test OpenClaw providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [providerEntries, t, favoriteProviders]);

  const handleSaveDiagnostics = async (diagnostics: OpenCodeDiagnosticsConfig) => {
    if (!config || !connectivityProviderId) {
      return;
    }

    const provider = config.models?.providers?.[connectivityProviderId];
    if (!provider) {
      return;
    }

    try {
      const favoriteProvider = await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('openclaw', connectivityProviderId),
        buildOpenClawFavoriteProviderConfig(connectivityProviderId, provider),
        diagnostics,
      );
      setFavoriteProviders((previousProviders) =>
        mergeDiagnosticsIntoFavoriteProviders(previousProviders, favoriteProvider, 'openclaw'),
      );
    } catch (error) {
      console.error('Failed to save OpenClaw connectivity diagnostics:', error);
      message.error(t('common.error'));
    }
  };

  const handleRemoveModels = async (modelIdsToRemove: string[]) => {
    if (!config || !connectivityProviderId) return;
    const provider = config.models?.providers?.[connectivityProviderId];
    if (!provider) return;

    const providers = config.models?.providers;
    if (!providers) return;

    const newModels = (provider.models || []).filter((m) => !modelIdsToRemove.includes(m.id));
    const newConfig: OpenClawConfig = {
      ...config,
      models: {
        ...config.models,
        providers: {
          ...providers,
          [connectivityProviderId]: { ...provider, models: newModels },
        },
      },
    };
    await saveOpenClawConfig(newConfig);
    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('openclaw', connectivityProviderId),
        buildOpenClawFavoriteProviderConfig(connectivityProviderId, { ...provider, models: newModels }),
      );
      await loadFavoriteProviders();
    } catch (favoriteError) {
      console.error('Failed to update OpenClaw favorite provider after removing models:', favoriteError);
    }
    loadConfig();
    refreshTrayMenu();
  };

  const connectivityProviderInfo = React.useMemo(() => {
    if (!config || !connectivityProviderId) return null;
    const provider = config.models?.providers?.[connectivityProviderId];
    if (!provider) return null;
    return {
      name: connectivityProviderId,
      config: toOpenCodeProvider(provider),
      modelIds: (provider.models || []).map((m) => m.id),
    };
  }, [config, connectivityProviderId]);

  const handleOpenFetchModels = (providerId: string) => {
    setFetchModelsProviderId(providerId);
    setFetchModelsModalOpen(true);
  };

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!config || !fetchModelsProviderId) return null;
    const provider = config.models?.providers?.[fetchModelsProviderId];
    if (!provider) return null;
    return {
      providerId: fetchModelsProviderId,
      name: fetchModelsProviderId,
      baseUrl: provider.baseUrl || '',
      apiKey: provider.apiKey,
      sdkName: apiToNpm(provider.api, provider.baseUrl),
      existingModelIds: (provider.models || []).map((m) => m.id),
    };
  }, [config, fetchModelsProviderId]);

  const handleFetchModelsSuccess = async ({ selectedModels, removedModelIds }: FetchModelsApplyResult) => {
    if (!config || !fetchModelsProviderId) return;
    const provider = config.models?.providers?.[fetchModelsProviderId];
    if (!provider) return;
    const providerNpm = apiToNpm(provider.api, provider.baseUrl);

    const providers = config.models?.providers;
    if (!providers) return;
    const removedModelIdSet = new Set(removedModelIds);
    const newModels = (provider.models || []).filter((model) => !removedModelIdSet.has(model.id));
    for (const model of selectedModels) {
      if (!newModels.find((m) => m.id === model.id)) {
        const matchedPresetModel = findPresetModelById(model.id, providerNpm);
        newModels.push(buildFetchedOpenClawModel(model, providerNpm, matchedPresetModel));
      }
    }

    const newConfig: OpenClawConfig = {
      ...config,
      models: {
        ...config.models,
        providers: {
          ...providers,
          [fetchModelsProviderId]: { ...provider, models: newModels },
        },
      },
    };
    await saveOpenClawConfig(newConfig);
    setFetchModelsModalOpen(false);
    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('openclaw', fetchModelsProviderId),
        buildOpenClawFavoriteProviderConfig(fetchModelsProviderId, { ...provider, models: newModels }),
      );
      await loadFavoriteProviders();
    } catch (favoriteError) {
      console.error('Failed to update OpenClaw favorite provider after fetching models:', favoriteError);
    }
    message.success(t('openclaw.providers.fetchModelsApplySuccess', {
      addCount: selectedModels.length,
      removeCount: removedModelIds.length,
    }));
    loadConfig();
    refreshTrayMenu();
  };

  // ================================================================
  // Other config (flatten fields excluding known sections)
  // ================================================================
  const otherConfigFields = React.useMemo(() => {
    if (!config) return undefined;
    const { models, agents, ...rest } = config;
    return Object.keys(rest).length > 0 ? rest : undefined;
  }, [config]);

  const handleOtherConfigChange = (_value: unknown, isValid: boolean) => {
    otherConfigJsonValidRef.current = isValid;
  };

  const handleOtherConfigBlur = async (value: unknown) => {
    if (!config || !otherConfigJsonValidRef.current) return;
    try {
      const newConfig: OpenClawConfig = {
        models: config.models,
        agents: config.agents,
      };
      if (typeof value === 'object' && value !== null) {
        Object.assign(newConfig, value);
      }
      await saveOpenClawConfig(newConfig);
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save other config:', error);
      message.error(t('common.error'));
    }
  };

  // ================================================================
  // Drag handlers
  // ================================================================
  const handleProviderDragEnd = async (event: DragEndEvent) => {
    if (!config?.models?.providers) return;
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const providerIds = Object.keys(config.models.providers);
    const oldIndex = providerIds.indexOf(active.id as string);
    const newIndex = providerIds.indexOf(over.id as string);
    const newOrder = arrayMove(providerIds, oldIndex, newIndex);

    const reordered: Record<string, OpenClawProviderConfig> = {};
    for (const key of newOrder) {
      reordered[key] = config.models.providers[key];
    }

    const newConfig: OpenClawConfig = {
      ...config,
      models: { ...config.models, providers: reordered },
    };
    await saveOpenClawConfig(newConfig);
    loadConfig();
    refreshTrayMenu();
  };

  const handleReorderModels = async (providerId: string, modelIds: string[]) => {
    if (!config?.models?.providers?.[providerId]) return;
    const provider = config.models.providers[providerId];
    const modelMap = new Map((provider.models || []).map((m) => [m.id, m]));
    const reordered = modelIds
      .map((id) => modelMap.get(id))
      .filter((m): m is OpenClawModel => Boolean(m));

    const newConfig: OpenClawConfig = {
      ...config,
      models: {
        ...config.models,
        providers: {
          ...config.models.providers,
          [providerId]: { ...provider, models: reordered },
        },
      },
    };
    await saveOpenClawConfig(newConfig);
    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('openclaw', providerId),
        buildOpenClawFavoriteProviderConfig(providerId, { ...provider, models: reordered }),
      );
      await loadFavoriteProviders();
    } catch (favoriteError) {
      console.error('Failed to update OpenClaw favorite provider after reordering models:', favoriteError);
    }
    loadConfig();
    refreshTrayMenu();
  };

  // ================================================================
  // Handlers
  // ================================================================
  const handleRefresh = () => {
    loadConfig();
    loadSectionData();
  };

  const handleOpenWebUi = async () => {
    try {
      await openOpenClawWebUi();
    } catch {
      Modal.confirm({
        title: t('openclaw.openWebUi'),
        content: t('openclaw.openWebUiOffline'),
        okText: t('openclaw.launchGateway'),
        onOk: async () => {
          try {
            await launchOpenClawGateway();
            message.success(t('openclaw.gatewayLaunched'));
          } catch {
            message.error(t('common.error'));
          }
        },
      });
    }
  };

  const handleOpenFolder = async () => {
    if (configPathInfo?.path) {
      try {
        await revealItemInDir(configPathInfo.path);
      } catch (error) {
        console.error('Failed to open folder:', error);
      }
    }
  };

  const handleConfigPathSuccess = () => {
    setConfigPathModalOpen(false);
    loadConfig();
    loadSectionData();
  };

  const handleSectionSaved = () => {
    loadConfig();
    loadSectionData();
    refreshTrayMenu();
  };

  // ================================================================
  // Render
  // ================================================================
  return (
    <SectionSidebarLayout
      sidebarTitle={t('openclaw.title')}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'openclaw-agents':
            return <RobotOutlined />;
          case 'openclaw-providers':
            return <DatabaseOutlined />;
          case 'openclaw-env':
            return <EnvironmentOutlined />;
          case 'openclaw-tools':
            return <ToolOutlined />;
          case 'openclaw-other':
            return <SettingOutlined />;
          case 'openclaw-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'openclaw-agents':
            setAgentsCollapsed(false);
            break;
          case 'openclaw-providers':
            setProvidersCollapsed(false);
            break;
          case 'openclaw-env':
            setEnvCollapsed(false);
            break;
          case 'openclaw-tools':
            setToolsCollapsed(false);
            break;
          case 'openclaw-other':
            setOtherCollapsed(false);
            break;
          case 'openclaw-session-manager':
            setSessionManagerExpandNonce((value) => value + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        {parseError ? (
          <Alert
            type="error"
            message={`Config parse error: ${parseError.error}`}
            description={
              <div>
                <Text code>{parseError.path}</Text>
                {parseError.contentPreview && (
                  <pre style={{ fontSize: 12, marginTop: 8, maxHeight: 200, overflow: 'auto' }}>
                    {parseError.contentPreview}
                  </pre>
                )}
                <Space style={{ marginTop: 8 }}>
                  <Button onClick={async () => { await backupOpenClawConfig(); loadConfig(); }}>
                    Backup & Reset
                  </Button>
                  <Button onClick={handleRefresh}>Retry</Button>
                </Space>
              </div>
            }
          />
        ) : (
          <>
            {/* ===== HEADER ===== */}
            <div style={{ marginBottom: 16 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
                <div>
                  <div style={{ marginBottom: 8 }}>
                    <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                      {t('openclaw.title')}
                    </Title>
                    <Link
                      type="secondary"
                      style={{ fontSize: 12 }}
                      onClick={(e) => {
                        e.stopPropagation();
                        openUrl('https://docs.openclaw.ai/concepts/model-providers');
                      }}
                    >
                      <LinkOutlined /> {t('openclaw.viewDocs')}
                    </Link>
                    <Link
                      type="secondary"
                      style={{ fontSize: 12, marginLeft: 16 }}
                      onClick={(e) => {
                        e.stopPropagation();
                        setPreviewOpen(true);
                      }}
                    >
                      <EyeOutlined /> {t('openclaw.previewConfig')}
                    </Link>
                  </div>
                  <Space>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('openclaw.configPath')}:
                    </Text>
                    <Text code style={{ fontSize: 12 }}>
                      {configPathInfo?.path || '~/.openclaw/openclaw.json'}
                    </Text>
                    <Button
                      type="text"
                      size="small"
                      icon={<EditOutlined />}
                      onClick={() => setConfigPathModalOpen(true)}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('openclaw.customConfigPath')}
                    </Button>
                    <Button
                      type="text"
                      size="small"
                      icon={<FolderOpenOutlined />}
                      onClick={handleOpenFolder}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('openclaw.openFolder')}
                    </Button>
                    <Button
                      type="text"
                      size="small"
                      icon={<ReloadOutlined />}
                      onClick={handleRefresh}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('openclaw.refreshConfig')}
                    </Button>
                    <Button
                      type="text"
                      size="small"
                      icon={<GlobalOutlined />}
                      onClick={handleOpenWebUi}
                      style={{ padding: 0, fontSize: 12 }}
                    >
                      {t('openclaw.openWebUi')}
                    </Button>
                  </Space>
                </div>
                <Space>
                  <Button type="text" icon={<EllipsisOutlined />} onClick={() => setSettingsModalOpen(true)}>
                    {t('common.moreOptions')}
                  </Button>
                </Space>
              </div>

              <div
                style={{
                  fontSize: 12,
                  color: 'var(--color-text-tertiary)',
                  borderLeft: '2px solid var(--color-border)',
                  paddingLeft: 8,
                  marginTop: 8,
                }}
              >
                {t('openclaw.configFileHint')}
              </div>
            </div>

            <OpenClawHealthBanner warnings={healthWarnings} onReload={handleRefresh} />

            {/* ===== AGENTS DEFAULTS COLLAPSE ===== */}
            <div
              id="openclaw-agents"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.agents.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={agentsCollapsed ? [] : ['agents']}
                onChange={(keys) => setAgentsCollapsed(!keys.includes('agents'))}
                items={[
                  {
                    key: 'agents',
                    label: (
                      <Text strong>
                        <RobotOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.agents.title')}
                      </Text>
                    ),
                    extra: (
                      <Button
                        type="link"
                        size="small"
                        icon={<MoreOutlined />}
                        onClick={(e) => {
                          e.stopPropagation();
                          agentsDefaultsRef.current?.openMoreParams();
                        }}
                      >
                        {t('openclaw.agents.moreParams')}
                      </Button>
                    ),
                    children: (
                      <AgentsDefaultsCard
                        ref={agentsDefaultsRef}
                        defaults={agentsDefaults}
                        config={config}
                        onSaved={handleSectionSaved}
                        onProviderUsed={noteProviderUsed}
                      />
                    ),
                  },
                ]}
              />
            </div>

            {/* ===== PROVIDERS COLLAPSE ===== */}
            <div
              id="openclaw-providers"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.providers.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={providersCollapsed ? [] : ['providers']}
                onChange={(keys) => setProvidersCollapsed(!keys.includes('providers'))}
                items={[
                  {
                    key: 'providers',
                    label: (
                      <Text strong>
                        <DatabaseOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.providers.title')}
                      </Text>
                    ),
                    extra: (
                      <Space size={4} wrap>
                        <Button
                          type="link"
                          size="small"
                          style={{ fontSize: 12, display: 'inline-flex', alignItems: 'center', gap: 4 }}
                          onClick={(e) => {
                            e.stopPropagation();
                            if (providerBatch.selectionMode) {
                              providerBatch.exitSelection();
                            } else {
                              providerBatch.enterSelection();
                            }
                          }}
                        >
                          <CheckSquareOutlined style={{ fontSize: 13, lineHeight: 1 }} />
                          <span>
                            {providerBatch.selectionMode
                              ? t('common.batch.exit')
                              : t('common.batch.manage')}
                          </span>
                        </Button>
                        {providerBatch.selectionMode && (
                          <ProviderBatchToolbar
                            hasSelection={providerBatch.hasSelection}
                            visibleCount={batchSelectableIds.length}
                            isAllSelected={providerBatch.isAllSelected}
                            indeterminate={providerBatch.indeterminate}
                            onSelectAll={providerBatch.selectAllFiltered}
                            onBatchDelete={providerBatch.batchDelete}
                            disabled={loading}
                          />
                        )}
                        <ProviderSearchInput value={providerKeyword} onChange={setProviderKeyword} />
                        <ProviderSortDropdown
                          mode={sortMode}
                          modes={PROVIDER_SORT_MODES_BASIC}
                          onChange={setSortMode}
                        />
                        <Button
                          type="link"
                          size="small"
                          icon={<ThunderboltOutlined />}
                          loading={batchTestingProviders}
                          onClick={(e) => {
                            e.stopPropagation();
                            handleBatchTestProviders();
                          }}
                        >
                          {t('common.batchTest')}
                        </Button>
                        <Button
                          type="link"
                          size="small"
                          icon={<PlusOutlined />}
                          onClick={(e) => {
                            e.stopPropagation();
                            handleAddProvider();
                          }}
                        >
                          {t('openclaw.providers.addProvider')}
                        </Button>
                      </Space>
                    ),
                    children: (
                      <Spin spinning={loading}>
                        {providerEntries.length === 0 ? (
                          <Empty description={t('openclaw.providers.emptyText')} />
                        ) : visibleProviderEntries.length === 0 ? (
                          <ProviderSearchEmpty />
                        ) : (
                          <DndContext
                            sensors={providerBatchDragDisabled ? [] : sensors}
                            collisionDetection={closestCenter}
                            modifiers={[restrictToVerticalAxis]}
                            onDragEnd={handleProviderDragEnd}
                          >
                            <SortableContext
                              items={providerEntries.map(([id]) => id)}
                              strategy={verticalListSortingStrategy}
                            >
                              {visibleProviderEntries.map(([providerId, providerConfig]) => (
                                <OpenClawProviderCard
                                  key={providerId}
                                  providerId={providerId}
                                  config={providerConfig}
                                  draggable={!providerBatchDragDisabled}
                                  sortableId={providerId}
                                  modelsDraggable
                                  onReorderModels={(modelIds) => handleReorderModels(providerId, modelIds)}
                                  onEdit={() => handleEditProvider(providerId, providerConfig)}
                                  onShare={() => shareProvider({
                                    id: providerId, name: providerId, category: 'custom',
                                    settingsConfig: JSON.stringify(providerConfig),
                                    defaultModel: agentsDefaults?.model?.primary?.startsWith(providerId + '/') ? agentsDefaults.model.primary.slice(providerId.length + 1) : undefined,
                                  })}
                                  onDelete={() => handleDeleteProvider(providerId)}
                                  deleteDisabledReason={
                                    agentsDefaults?.model?.primary?.split('/')[0] === providerId
                                      ? t('openclaw.providers.deleteDisabledDefault', { defaultValue: '该渠道已设为默认，不可删除' })
                                      : undefined
                                  }
                                  selectable={providerBatch.selectionMode && providerBatch.isSelectable(providerId)}
                                  selected={providerBatch.selectedIds.has(providerId)}
                                  onSelectChange={(checked) =>
                                    providerBatch.toggleSelect(providerId, checked)
                                  }
                                  onAddModel={() => handleAddModel(providerId)}
                                  onEditModel={(model) => handleEditModel(providerId, model)}
                                  onDeleteModel={(modelId) => handleDeleteModel(providerId, modelId)}
                                  onConnectivityTest={() => handleOpenConnectivityTest(providerId)}
                                  onFetchModels={() => handleOpenFetchModels(providerId)}
                                  connectivityStatus={connectivityStatuses[providerId]}
                                  modelSelectionMode={batchDeleteProviderId === providerId}
                                  selectedModelIds={selectedModelIdsByProvider[providerId] ?? []}
                                  onToggleModelSelection={(modelId, selected) => handleToggleModelSelection(providerId, modelId, selected)}
                                  onToggleBatchDeleteMode={() => handleToggleBatchDeleteMode(providerId)}
                                  onBatchDeleteModels={() => handleBatchDeleteModels(providerId)}
                                />
                              ))}
                            </SortableContext>
                          </DndContext>
                        )}
                        <div style={{ marginTop: 12 }}>
                          <Space wrap>
                            <Button
                              type="dashed"
                              icon={<ImportOutlined />}
                              onClick={() => setFavoriteImportModalOpen(true)}
                            >
                              {t('opencode.provider.importFavorite')}
                            </Button>
                            {allApiHubAvailable && (
                              <Button
                                type="dashed"
                                icon={<AllApiHubIcon />}
                                onClick={() => setAllApiHubImportModalOpen(true)}
                              >
                                {t('openclaw.providers.importFromAllApiHub')}
                              </Button>
                            )}
                            {ccSwitchAvailable && (
                              <Button
                                type="dashed"
                                icon={<ImportOutlined />}
                                onClick={() => setCcSwitchImportModalOpen(true)}
                              >
                                {t('common.ccSwitch.importFromCcSwitch')}
                              </Button>
                            )}
                          </Space>
                        </div>
                      </Spin>
                    ),
                  },
                ]}
              />
            </div>

            {/* ===== ENV COLLAPSE ===== */}
            <div
              id="openclaw-env"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.env.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={envCollapsed ? [] : ['env']}
                onChange={(keys) => setEnvCollapsed(!keys.includes('env'))}
                items={[
                  {
                    key: 'env',
                    label: (
                      <Text strong>
                        <EnvironmentOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.env.title')}
                      </Text>
                    ),
                    children: <EnvCard env={env} onSaved={handleSectionSaved} />,
                  },
                ]}
              />
            </div>

            {/* ===== TOOLS COLLAPSE ===== */}
            <div
              id="openclaw-tools"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.tools.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={toolsCollapsed ? [] : ['tools']}
                onChange={(keys) => setToolsCollapsed(!keys.includes('tools'))}
                items={[
                  {
                    key: 'tools',
                    label: (
                      <Text strong>
                        <ToolOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.tools.title')}
                      </Text>
                    ),
                    children: <ToolsCard tools={tools} onSaved={handleSectionSaved} />,
                  },
                ]}
              />
            </div>

            {/* ===== OTHER CONFIG COLLAPSE ===== */}
            <div
              id="openclaw-other"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.other.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={otherCollapsed ? [] : ['other']}
                onChange={(keys) => setOtherCollapsed(!keys.includes('other'))}
                items={[
                  {
                    key: 'other',
                    label: (
                      <Text strong>
                        <SettingOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.other.title')}
                      </Text>
                    ),
                    children: (
                      <div>
                        <JsonEditor
                          value={otherConfigFields}
                          onChange={handleOtherConfigChange}
                          onBlur={handleOtherConfigBlur}
                          height={300}
                          minHeight={200}
                          maxHeight={500}
                          resizable
                          mode="text"
                          placeholder={`{
    "env": { "vars": {}, "shellEnv": {} },
    "tools": { "profile": "coding" }
}`}
                        />
                        <div style={{ marginTop: 8 }}>
                          <Text type="secondary">{t('openclaw.other.hint')}，</Text>
                          <span style={{ color: '#1677ff' }}>
                            {t('openclaw.other.autoSaveHint')}
                          </span>
                        </div>
                      </div>
                    ),
                  },
                ]}
              />
            </div>

            <div
              id="openclaw-session-manager"
              data-sidebar-section="true"
              data-sidebar-title={t('sessionManager.title')}
            >
              <SessionManagerPanel tool="openclaw" expandNonce={sessionManagerExpandNonce} />
            </div>

            {/* ===== MODALS ===== */}
            <OpenClawProviderFormModal
              open={providerModalOpen}
              editingProvider={editingProvider}
              existingIds={providerEntries.map(([id]) => id)}
              onCancel={() => setProviderModalOpen(false)}
              onSubmit={handleProviderSubmit}
            />

            {/* OpenCode import entry removed; keep modal/handler for later restore */}
            <ImportFromOpenCodeModal
              open={importModalOpen}
              existingProviderIds={providerEntries.map(([id]) => id)}
              onCancel={() => setImportModalOpen(false)}
              onImport={handleImportFromOpenCode}
            />

            <ImportProviderModal
              open={favoriteImportModalOpen}
              onClose={() => setFavoriteImportModalOpen(false)}
              onImport={handleImportFavoriteProviders}
              existingProviderIds={providerEntries.map(([id]) => buildFavoriteProviderStorageKey('openclaw', id))}
              providerFilter={(provider) => isFavoriteProviderForSource('openclaw', provider)}
            />

            {allApiHubAvailable && (
              <ImportFromAllApiHubModal
                open={allApiHubImportModalOpen}
                existingProviderIds={providerEntries.map(([id]) => id)}
                onCancel={() => setAllApiHubImportModalOpen(false)}
                onImport={handleImportFromAllApiHub}
              />
            )}

            {ccSwitchAvailable && (
              <ImportFromCcSwitchModal
                open={ccSwitchImportModalOpen}
                appType="claude"
                existingProviderIds={providerEntries.map(([id]) => id)}
                onClose={() => setCcSwitchImportModalOpen(false)}
                onImport={handleImportFromCcSwitch}
              />
            )}

            {ccSwitchAvailable && (
              <ImportFromCcSwitchModal
                open={ccSwitchImportModalOpen}
                appType="claude"
                existingProviderIds={providerEntries.map(([id]) => id)}
                onClose={() => setCcSwitchImportModalOpen(false)}
                onImport={handleImportFromCcSwitch}
              />
            )}

            <OpenClawModelFormModal
              open={modelModalOpen}
              editingModel={editingModel}
              existingIds={
                config?.models?.providers?.[modelTargetProvider]?.models?.map((m) => m.id) || []
              }
              apiProtocol={config?.models?.providers?.[modelTargetProvider]?.api}
              onCancel={() => setModelModalOpen(false)}
              onSubmit={handleModelSubmit}
            />

            <OpenClawConfigPathModal
              open={configPathModalOpen}
              currentPathInfo={configPathInfo}
              onCancel={() => setConfigPathModalOpen(false)}
              onSuccess={handleConfigPathSuccess}
            />

            <FileConfigPreviewModal
              open={previewOpen}
              onClose={() => setPreviewOpen(false)}
              title={t('openclaw.previewConfig')}
              files={[
                {
                  key: 'config',
                  label: configPathInfo?.path?.split(/[\\/]/).pop() || 'openclaw.json',
                  content: config,
                },
              ]}
            />

            {/* Fetch Models Modal (reuse common component) */}
            {fetchModelsProviderInfo && (
              <FetchModelsModal
                open={fetchModelsModalOpen}
                providerId={fetchModelsProviderInfo.providerId}
                providerName={fetchModelsProviderInfo.name}
                baseUrl={fetchModelsProviderInfo.baseUrl}
                apiKey={fetchModelsProviderInfo.apiKey}
                sdkType={fetchModelsProviderInfo.sdkName}
                existingModelIds={fetchModelsProviderInfo.existingModelIds}
                onCancel={() => setFetchModelsModalOpen(false)}
                onSuccess={handleFetchModelsSuccess}
              />
            )}

            {/* Connectivity Test Modal (reuse OpenCode component) */}
            {connectivityProviderInfo && (
              <ConnectivityTestModal
                open={connectivityModalOpen}
                onCancel={() => setConnectivityModalOpen(false)}
                providerId={connectivityProviderId}
                providerName={connectivityProviderInfo.name}
                providerConfig={connectivityProviderInfo.config}
                modelIds={connectivityProviderInfo.modelIds}
                diagnostics={findDiagnosticsForProvider(favoriteProviders, 'openclaw', connectivityProviderId)}
                onSaveDiagnostics={handleSaveDiagnostics}
                onRemoveModels={handleRemoveModels}
              />
            )}

            <SidebarSettingsModal
              open={settingsModalOpen}
              onClose={() => setSettingsModalOpen(false)}
              sidebarVisible={!sidebarHidden}
              onSidebarVisibleChange={(visible) => setSidebarHidden('openclaw', !visible)}
            >
          <CliManualPathSetting commandName="openclaw" labelKey="subModules.openclaw" />
        </SidebarSettingsModal>
          </>
        )}
      </div>

      {shareModal}
    </SectionSidebarLayout>
  );
};

export default OpenClawPage;
