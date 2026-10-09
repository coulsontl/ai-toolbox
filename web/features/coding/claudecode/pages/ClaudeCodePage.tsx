import React from 'react';
import { Typography, Button, Space, message, Modal, Collapse } from 'antd';
import { AppstoreOutlined, SyncOutlined, ExclamationCircleOutlined, DatabaseOutlined, ImportOutlined, FileTextOutlined, MessageOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
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
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
  arrayMove,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import type {
  ClaudeCodeProvider,
  ConfigPathInfo,
  ClaudeProviderFormValues,
  ClaudeProviderInput,
  ImportConflictInfo,
  ImportConflictAction,
} from '@/types/claudecode';
import { DEFAULT_CLAUDE_SETTINGS_MERGE_STRATEGY } from '@/types/claudecode';
import {
  getClaudeConfigPath,
  getClaudeRootPathInfo,
  getClaudeCommonConfig,
  saveClaudeCommonConfig,
  listClaudeProviders,
  createClaudeProvider,
  updateClaudeProvider,
  saveClaudeLocalConfig,
  deleteClaudeProvider,
  applyClaudeConfig,
  readClaudeSettings,
  toggleClaudeCodeProviderDisabled,
  reorderClaudeProviders,
} from '@/services/claudeCodeApi';
import { useRefreshStore, useSettingsStore } from '@/stores';
import { refreshTrayMenu, hasAllApiHubExtension } from '@/services/appApi';
import { TRAY_CONFIG_REFRESH_EVENT, DEEP_LINK_IMPORT_COMPLETED } from '@/constants/configEvents';
import { claudeCodePromptApi } from '@/services/claudeCodePromptApi';
import ClaudeProviderCard from '../components/ClaudeProviderCard';
import ClaudeProviderFormModal from '../components/ClaudeProviderFormModal';
import CommonConfigModal from '../components/CommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import ClaudeCodeSettingsModal from '../components/ClaudeCodeSettingsModal';
import ClaudePluginsPanel from '../components/ClaudePluginsPanel';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import ShareProviderModal from '@/features/coding/shared/providerShare';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import ProviderListSection from '@/features/coding/shared/ProviderListSection';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import {
  areGatewayProviderProfilesInitialized,
  firstGatewayApiFormat,
  GatewayFailoverButton,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  isGatewayConfigFlagEnabled,
  providerNeedsGatewayProxy,
  resolveGatewayReengageMode,
  saveProviderWithGatewayReengage,
  subscribeGatewayProviderProfiles,
  toGatewayAggregateReengageConfig,
} from '@/features/coding/shared/gateway';
import ProviderConnectivityTestModal, {
  buildClaudeProviderConnectivityInfo,
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import {
  PROVIDER_SORT_MODES,
  backupProvidersBeforeDelete,
  filterProviderItems,
  sortProviderItems,
  useProviderBatchSelection,
  useProviderListSort,
} from '@/features/coding/shared/providerList';
import {
  deleteFavoriteProvider,
  listFavoriteProviders,
  upsertFavoriteProvider,
  type OpenCodeDiagnosticsConfig,
  type OpenCodeFavoriteProvider,
} from '@/services/opencodeApi';
import {
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import { getEnabledCustomProviderBatchCandidates } from '@/features/coding/shared/providerConnectivity/batchTestFilters';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import {
  buildFavoriteProviderOptions,
  buildFavoriteProviderStorageKey,
  dedupeFavoriteProvidersByPayload,
  extractFavoriteProviderRawId,
  findDefaultTestModelIdForProvider,
  findDiagnosticsForProvider,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  mergeDiagnosticsIntoFavoriteProviders,
  type ClaudeFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import {
  engageProxyGatewayFailover,
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  type GatewayCliTakeoverStatus,
} from '@/services';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import {
  getClaudeConfiguredModelIds,
  parseClaudeSettingsConfig,
} from '../utils/claudeModelConfig';
import { LOCAL_CONFIG_ID } from '../../shared/localConfig';

const { Text } = Typography;

function buildClaudeFavoriteProviderConfig(provider: ClaudeCodeProvider) {
  const settingsConfig = parseClaudeSettingsConfig(provider.settingsConfig);
  const apiKey =
    settingsConfig.env?.ANTHROPIC_AUTH_TOKEN?.trim() ||
    settingsConfig.env?.ANTHROPIC_API_KEY?.trim();
  const modelIds = getClaudeConfiguredModelIds(settingsConfig, { stripOneMMarker: true });

  return buildFavoriteProviderOptions(
    {
      npm: '@ai-sdk/anthropic',
      name: provider.name,
      options: {
        ...(settingsConfig.env?.ANTHROPIC_BASE_URL
          ? { baseURL: settingsConfig.env.ANTHROPIC_BASE_URL }
          : {}),
        ...(apiKey ? { apiKey } : {}),
      },
      models: Object.fromEntries(modelIds.map((modelId) => [modelId, {}])),
    },
    {
      name: provider.name,
      category: provider.category,
      settingsConfig: provider.settingsConfig,
      extraSettingsConfig: provider.extraSettingsConfig,
      extraSettingsMergeStrategy: provider.extraSettingsMergeStrategy,
      ...(provider.meta ? { meta: provider.meta } : {}),
      ...(provider.notes ? { notes: provider.notes } : {}),
    } satisfies ClaudeFavoriteProviderPayload,
  );
}

function buildClaudeProviderSettingsConfig(values: ClaudeProviderFormValues): string {
  const settingsConfigObj: Record<string, unknown> = {};
  const envConfig: Record<string, string> = {};

  const normalizedBaseUrl = values.baseUrl?.trim();
  const normalizedApiKey = values.apiKey?.trim();

  if (normalizedBaseUrl) {
    envConfig.ANTHROPIC_BASE_URL = normalizedBaseUrl;
  }

  if (normalizedApiKey) {
    envConfig.ANTHROPIC_AUTH_TOKEN = normalizedApiKey;
  }

  if (values.model?.trim()) {
    envConfig.ANTHROPIC_MODEL = values.model.trim();
  }
  if (values.haikuModel?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_HAIKU_MODEL = values.haikuModel.trim();
  }
  if (values.haikuModelName?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME = values.haikuModelName.trim();
  }
  if (values.sonnetModel?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_SONNET_MODEL = values.sonnetModel.trim();
  }
  if (values.sonnetModelName?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME = values.sonnetModelName.trim();
  }
  if (values.opusModel?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_OPUS_MODEL = values.opusModel.trim();
  }
  if (values.opusModelName?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME = values.opusModelName.trim();
  }
  if (values.fableModel?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_FABLE_MODEL = values.fableModel.trim();
  }
  if (values.fableModelName?.trim()) {
    envConfig.ANTHROPIC_DEFAULT_FABLE_MODEL_NAME = values.fableModelName.trim();
  }

  if (Object.keys(envConfig).length > 0) {
    settingsConfigObj.env = envConfig;
  }

  return JSON.stringify(settingsConfigObj);
}

const ClaudeCodePage: React.FC = () => {
  const { t } = useTranslation();
  const { claudeProviderRefreshKey } = useRefreshStore();
  const {
    sidebarHiddenByPage,
    setSidebarHidden,
  } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [providers, setProviders] = React.useState<ClaudeCodeProvider[]>([]);
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');
  const [gatewayCliStatus, setGatewayCliStatus] = React.useState<GatewayCliTakeoverStatus | null>(null);
  const gatewayTakeoverActive = Boolean(gatewayCliStatus?.can_restore_direct);
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const primaryGatewayProviderNeedsProxy = React.useMemo(() => {
    const primaryProvider = providers.find(
      (provider) => provider.id === gatewayCliStatus?.primary_provider_id,
    );
    if (!primaryProvider || primaryProvider.category === 'official' || primaryProvider.id === LOCAL_CONFIG_ID) {
      return false;
    }
    const settingsConfig = parseClaudeSettingsConfig(primaryProvider.settingsConfig) as {
      apiFormat?: unknown;
      api_format?: unknown;
      openrouter_compat_mode?: unknown;
    };
    const providerApiFormat = firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(primaryProvider.meta, 'claude'),
      primaryProvider.meta?.apiFormat,
      typeof settingsConfig.apiFormat === 'string' ? settingsConfig.apiFormat : undefined,
      typeof settingsConfig.api_format === 'string' ? settingsConfig.api_format : undefined,
      isGatewayConfigFlagEnabled(settingsConfig.openrouter_compat_mode)
        ? 'openai_chat'
        : undefined,
    );
    return providerNeedsGatewayProxy(providerApiFormat, 'anthropic');
  }, [gatewayCliStatus?.primary_provider_id, gatewayProviderProfilesVersion, providers]);
  const primaryGatewayProviderNeedsProxyReason = primaryGatewayProviderNeedsProxy ? 'protocol' : null;

  // 模态框状态
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<ClaudeCodeProvider | null>(null);
  const [shareProvider, setShareProvider] = React.useState<ClaudeCodeProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [providerModalMode, setProviderModalMode] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<ClaudeProviderFormValues | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<unknown>(null);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityInfo, setConnectivityInfo] = React.useState<ProviderConnectivityInfo | null>(null);
  const [connectivityUsesGateway, setConnectivityUsesGateway] = React.useState(false);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);
  const [favoriteProviders, setFavoriteProviders] = React.useState<OpenCodeFavoriteProvider[]>([]);
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [pluginListCollapsed, setPluginListCollapsed] = React.useState(true);
  const [pluginPanelRefreshToken, setPluginPanelRefreshToken] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const sidebarHidden = sidebarHiddenByPage.claudecode;

  // 配置拖拽传感器
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8, // 防止点击误触
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    {
      id: 'claudecode-providers',
      title: t('claudecode.provider.title'),
      order: 1,
    },
    {
      id: 'claudecode-global-prompt',
      title: t('common.prompt.title'),
      order: 2,
    },
    {
      id: 'claudecode-plugins',
      title: t('claudecode.plugins.title'),
      order: 3,
    },
    {
      id: 'claudecode-session-manager',
      title: t('sessionManager.title'),
      order: 4,
    },
  ], [t]);

  React.useEffect(() => {
    const checkAllApiHubAvailability = async () => {
      try {
        const available = await hasAllApiHubExtension();
        setAllApiHubAvailable(available);
      } catch {
        setAllApiHubAvailable(false);
      }
    };

    const checkCcSwitchAvailability = async () => {
      try {
        const available = await hasCcSwitchDb();
        setCcSwitchAvailable(available);
      } catch {
        setCcSwitchAvailable(false);
      }
    };

    checkAllApiHubAvailability();
    checkCcSwitchAvailability();
  }, []);

  const loadFavoriteProviders = React.useCallback(async () => {
    try {
      const allFavoriteProviders = await listFavoriteProviders();
      const claudeFavoriteProviders = allFavoriteProviders.filter((provider) =>
        isFavoriteProviderForSource('claudecode', provider),
      );
      const currentStorageKeys = new Set(
        providers.map((provider) => buildFavoriteProviderStorageKey('claudecode', provider.id)),
      );
      const { keptProviders, duplicateIds } = dedupeFavoriteProvidersByPayload(
        claudeFavoriteProviders,
        currentStorageKeys,
      );

      if (duplicateIds.length > 0) {
        await Promise.all(
          duplicateIds.map(async (providerId) => {
            try {
              await deleteFavoriteProvider(providerId);
            } catch (error) {
              console.error('Failed to delete duplicate Claude favorite provider:', error);
            }
          }),
        );
      }

      setFavoriteProviders(keptProviders);
    } catch (error) {
      console.error('Failed to load Claude favorite providers:', error);
    }
  }, [providers]);

  React.useEffect(() => {
    loadFavoriteProviders();
  }, [loadFavoriteProviders]);

  React.useEffect(() => {
    setConnectivityStatuses((previousStatuses) => {
      const nextStatuses = Object.fromEntries(
        Object.entries(previousStatuses).filter(([providerId]) => {
          const provider = providers.find((item) => item.id === providerId);
          return provider && provider.category !== 'official';
        }),
      );

      return Object.keys(nextStatuses).length === Object.keys(previousStatuses).length
        ? previousStatuses
        : nextStatuses;
    });
  }, [providers]);

  const loadConfig = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [path, nextRootPathInfo, providerList] = await Promise.all([
        getClaudeConfigPath(),
        getClaudeRootPathInfo(),
        listClaudeProviders(),
      ]);

      setConfigPath(path);
      setRootPathInfo(nextRootPathInfo);
      setProviders(providerList);
      setPluginPanelRefreshToken((value) => value + 1);

      const applied = providerList.find((p) => p.isApplied);
      setAppliedProviderId(applied?.id || '');
    } catch (error) {
      console.error('Failed to load config:', error);
      if (!silent) {
        message.error(t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  const loadConfigRef = React.useRef(loadConfig);
  loadConfigRef.current = loadConfig;

  // 加载配置（on mount and when refresh key changes）
  React.useEffect(() => {
    void claudeProviderRefreshKey;
    loadConfigRef.current();
  }, [claudeProviderRefreshKey]);

  React.useEffect(() => {
    const handleTrayConfigRefresh = (event: Event) => {
      event.preventDefault();
      void loadConfig(true);
    };
    const handleDeepLinkImport = (event: Event) => {
      const detail = (event as CustomEvent<{ app?: string; id?: string }>).detail;
      if (detail?.app === 'claude') {
        void loadConfig(true);
      }
    };

    window.addEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    window.addEventListener(DEEP_LINK_IMPORT_COMPLETED, handleDeepLinkImport);
    return () => {
      window.removeEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
      window.removeEventListener(DEEP_LINK_IMPORT_COMPLETED, handleDeepLinkImport);
    };
  }, [loadConfig]);

  const handleOpenFolder = async () => {
    if (!configPath) return;

    try {
      // Try to reveal the file in explorer
      await revealItemInDir(configPath);
    } catch {
      // If file doesn't exist, fallback to opening parent directory
      try {
        const parentDir = configPath.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open folder:', error);
        message.error(t('common.error'));
      }
    }
  };

  const handleRefreshPage = () => {
    loadConfig();
  };

  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig({
    t,
    translationKeyPrefix: 'claudecode',
    defaultConfig: '{}',
    rootDirectoryChangeLocked: gatewayTakeoverActive,
    rootDirectoryChangeLockedText: t('gateway.proxy.rootDirectorySaveLockedTooltip'),
    loadConfig,
    getCommonConfig: getClaudeCommonConfig,
    saveCommonConfig: saveClaudeCommonConfig,
  });

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('claudecode');
  const [providerKeyword, setProviderKeyword] = React.useState('');
  // Search and non-custom sort modes bypass sort_index, so dragging would
  // write a stale custom order — dnd is only enabled in custom mode.
  const providerDragDisabled = sortMode !== 'custom' || providerKeyword.trim() !== '';
  const visibleProviders = React.useMemo(
    () =>
      sortProviderItems(
        filterProviderItems(providers, providerKeyword, (provider) => [
          provider.name,
          provider.notes ?? '',
          provider.websiteUrl ?? '',
        ]),
        sortMode,
        { name: (provider) => provider.name, createdAt: (provider) => provider.createdAt },
        (provider) => lastUsedAt(provider.id),
      ),
    [providers, providerKeyword, sortMode, lastUsedAt],
  );

  const canBatchDeleteProvider = React.useCallback(
    (provider: ClaudeCodeProvider) => provider.id !== LOCAL_CONFIG_ID,
    [],
  );

  const handleBatchDeleteProviders = React.useCallback(
    async (ids: string[]): Promise<boolean> => {
      const providersToDelete = providers.filter(
        (provider) => ids.includes(provider.id) && canBatchDeleteProvider(provider),
      );
      if (providersToDelete.length === 0) return false;
      try {
        await backupProvidersBeforeDelete(
          providersToDelete,
          (provider) => upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('claudecode', provider.id),
            buildClaudeFavoriteProviderConfig(provider),
          ),
          (provider) => t('common.batch.backupFailed', { name: provider.name }),
        );
        for (const provider of providersToDelete) {
          await deleteClaudeProvider(provider.id);
        }
        await loadFavoriteProviders();
        await loadConfig();
        await refreshTrayMenu();
        message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete claudecode providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig();
        await loadFavoriteProviders();
        return false;
      }
    },
    [providers, canBatchDeleteProvider, loadFavoriteProviders, loadConfig, t],
  );
  // Selectable ids exclude `__local__` (no delete path / not a managed preset).
  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter(canBatchDeleteProvider).map((provider) => provider.id),
    [visibleProviders, canBatchDeleteProvider],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  const handleSelectProvider = async (provider: ClaudeCodeProvider) => {
    try {
      await applyClaudeConfig(provider.id);
      message.success(t('claudecode.apply.success'));
      noteProviderUsed(provider.id);
      await loadConfig();
    } catch (error) {
      console.error('Failed to select provider:', error);
      message.error(t('common.error'));
    }
  };

  const handleToggleDisabled = async (provider: ClaudeCodeProvider, isDisabled: boolean) => {
    try {
      await toggleClaudeCodeProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('claudecode.providerDisabled') : t('claudecode.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle provider disabled status:', error);
      message.error(t('common.error'));
    }
  };

  // 拖拽结束
  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;

    if (!over || active.id === over.id) {
      return;
    }

    const oldIndex = providers.findIndex((p) => p.id === active.id);
    const newIndex = providers.findIndex((p) => p.id === over.id);

    if (oldIndex === -1 || newIndex === -1) {
      return;
    }

    // 乐观更新
    const oldProviders = [...providers];
    const newProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(newProviders);

    try {
      await reorderClaudeProviders(newProviders.map((p) => p.id));
      await refreshTrayMenu();
    } catch (error) {
      // 失败回滚
      console.error('Failed to reorder providers:', error);
      setProviders(oldProviders);
      message.error(t('common.error'));
    }
  };

  const handleAddProvider = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  // OpenCode import entry removed; keep handler for later restore.
  const _handleImportFromOpenCode = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalMode('import');
    setProviderModalOpen(true);
  };
  void _handleImportFromOpenCode;

  const handleEditProvider = (provider: ClaudeCodeProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: ClaudeCodeProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name}_copy`,
      isApplied: false,
    });
    setIsCopyMode(true);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  const handleShareProvider = (provider: ClaudeCodeProvider) => {
    setShareProvider(provider);
  };

  const handleTestProvider = (provider: ClaudeCodeProvider) => {
    if (provider.category === 'official') {
      message.info(t('claudecode.provider.officialConnectivityHint'));
      return;
    }
    if (!areGatewayProviderProfilesInitialized()) {
      message.info(t('common.loading'));
      return;
    }

    const settingsConfig = parseClaudeSettingsConfig(provider.settingsConfig) as {
      apiFormat?: unknown;
      api_format?: unknown;
      openrouter_compat_mode?: unknown;
    };
    const providerApiFormat = firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(provider.meta, 'claude'),
      provider.meta?.apiFormat,
      typeof settingsConfig.apiFormat === 'string' ? settingsConfig.apiFormat : undefined,
      typeof settingsConfig.api_format === 'string' ? settingsConfig.api_format : undefined,
      isGatewayConfigFlagEnabled(settingsConfig.openrouter_compat_mode) ? 'openai_chat' : undefined,
    );
    setConnectivityInfo(buildClaudeProviderConnectivityInfo(provider));
    setConnectivityUsesGateway(providerNeedsGatewayProxy(providerApiFormat, 'anthropic'));
    setConnectivityModalOpen(true);
  };

  const handleSaveConnectivityDiagnostics = React.useCallback(async (diagnostics: OpenCodeDiagnosticsConfig) => {
    if (!connectivityInfo) {
      return;
    }

    const targetProvider = providers.find((provider) => provider.id === connectivityInfo.providerId);
    if (!targetProvider) {
      return;
    }

    try {
      const favoriteProvider = await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('claudecode', targetProvider.id),
        buildClaudeFavoriteProviderConfig(targetProvider),
        diagnostics,
      );
      setFavoriteProviders((previousProviders) =>
        mergeDiagnosticsIntoFavoriteProviders(previousProviders, favoriteProvider, 'claudecode'),
      );
    } catch (error) {
      console.error('Failed to save Claude connectivity diagnostics:', error);
      message.error(t('common.error'));
    }
  }, [connectivityInfo, providers, t]);

  const handleBatchTestProviders = React.useCallback(async () => {
    if (!areGatewayProviderProfilesInitialized()) {
      message.info(t('common.loading'));
      return;
    }
    if (providers.length === 0) {
      return;
    }

    const officialProviders = providers.filter((provider) => provider.category === 'official');
    const testableProviders = getEnabledCustomProviderBatchCandidates(providers);

    if (officialProviders.length > 0) {
      message.info(t('claudecode.provider.officialBatchSkipped', { count: officialProviders.length }));
    }

    if (testableProviders.length === 0) {
      setConnectivityStatuses({});
      return;
    }

    const targets = testableProviders.map((provider) => {

      const connectivityInfo = buildClaudeProviderConnectivityInfo(provider);
      const settingsConfig = parseClaudeSettingsConfig(provider.settingsConfig) as {
        env?: {
          ANTHROPIC_BASE_URL?: string;
        };
        apiFormat?: unknown;
        api_format?: unknown;
        openrouter_compat_mode?: unknown;
      };
      const hasExplicitBaseUrl = Boolean(settingsConfig.env?.ANTHROPIC_BASE_URL?.trim());
      const providerApiFormat = firstGatewayApiFormat(
        getGatewayProviderApiFormatFromMeta(provider.meta, 'claude'),
        provider.meta?.apiFormat,
        typeof settingsConfig.apiFormat === 'string' ? settingsConfig.apiFormat : undefined,
        typeof settingsConfig.api_format === 'string' ? settingsConfig.api_format : undefined,
        isGatewayConfigFlagEnabled(settingsConfig.openrouter_compat_mode)
          ? 'openai_chat'
          : undefined,
      );
      const useGateway = providerNeedsGatewayProxy(providerApiFormat, 'anthropic');

      if (provider.category !== 'official' && !hasExplicitBaseUrl) {
        return {
          providerId: provider.id,
          errorMessage: t('common.baseUrlMissing'),
        };
      }

      return buildProviderConnectivityBatchTarget(connectivityInfo, {
        requireBaseUrl: false,
        requireApiKey: !useGateway,
        gatewayCliKey: 'claude',
        useGateway,
        preferredModelId: findDefaultTestModelIdForProvider(favoriteProviders, 'claudecode', provider.id),
        errorMessages: {
          missingBaseUrl: t('common.baseUrlMissing'),
          missingApiKey: t('common.apiKeyMissing'),
          missingModel: t('common.modelMissing'),
        },
      });
    });

    setConnectivityStatuses(
      Object.fromEntries(
        testableProviders.map((provider) => [
          provider.id,
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
      console.error('Failed to batch test Claude providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [providers, t, favoriteProviders, gatewayProviderProfilesVersion]);

  const handleDeleteProvider = (provider: ClaudeCodeProvider) => {
    const performDelete = async () => {
      try {
        await deleteClaudeProvider(provider.id);
        await loadFavoriteProviders();
        message.success(t('common.success'));
        await loadConfig();
      } catch (error) {
        console.error('Failed to delete provider:', error);
        message.error(t('common.error'));
      }
    };

    Modal.confirm({
      title: t('claudecode.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('claudecode', provider.id),
            buildClaudeFavoriteProviderConfig(provider),
          );
          await performDelete();
        } catch (favoriteError) {
          console.error('Failed to preserve Claude favorite provider before deletion:', favoriteError);
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
      },
    });
  };

  const handleProviderSubmit = async (values: ClaudeProviderFormValues) => {
    // 检查是否有冲突
    if (values.sourceProviderId && !editingProvider) {
      const existingProvider = providers.find(
        (p) => p.sourceProviderId === values.sourceProviderId
      );

      if (existingProvider) {
        // 显示冲突对话框
        setConflictInfo({
          existingProvider,
          newProviderName: values.name,
          sourceProviderId: values.sourceProviderId,
        });
        setPendingFormValues(values);
        setConflictDialogOpen(true);
        return;
      }
    }

    // 没有冲突，直接保存
    await doSaveProvider(values);
  };

  const handleConflictResolve = async (action: ImportConflictAction) => {
    if (!pendingFormValues || !conflictInfo) return;

    if (action === 'cancel') {
      setConflictDialogOpen(false);
      setConflictInfo(null);
      setPendingFormValues(null);
      return;
    }

    if (action === 'overwrite') {
      // 覆盖现有配置
      await doUpdateProvider(conflictInfo.existingProvider.id, pendingFormValues);
    } else {
      // 创建副本
      await doSaveProvider({
        ...pendingFormValues,
        sourceProviderId: undefined, // 移除 sourceProviderId 以避免再次冲突
      });
    }

    setConflictDialogOpen(false);
    setConflictInfo(null);
    setPendingFormValues(null);
  };

  const handleImportFromAllApiHub = async (imported: OpenCodeAllApiHubProvider[]) => {
    try {
      for (const item of imported) {
        const providerInput: ClaudeProviderInput = {
          name: item.name,
          category: 'custom',
          settingsConfig: JSON.stringify({
            env: {
              ...(item.providerConfig.options?.baseURL && {
                ANTHROPIC_BASE_URL: item.providerConfig.options.baseURL.replace(/\/v1$/, ''),
              }),
              ...(item.providerConfig.options?.apiKey && {
                ANTHROPIC_AUTH_TOKEN: item.providerConfig.options.apiKey,
              }),
            },
          }),
          extraSettingsConfig: '{}',
          extraSettingsMergeStrategy: undefined,
          sourceProviderId: item.providerId,
          notes: undefined,
        };

        const createdProvider = await createClaudeProvider(providerInput);
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('claudecode', createdProvider.id),
            buildClaudeFavoriteProviderConfig(createdProvider),
          );
        } catch (favoriteError) {
          console.error('Failed to save Claude favorite provider from All API Hub import:', favoriteError);
        }
      }

      message.success(t('common.allApiHub.importSuccess', { count: imported.length }));
      setAllApiHubImportModalOpen(false);
      await loadConfig();
      await loadFavoriteProviders();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from All API Hub:', error);
      message.error(t('common.error'));
    }
  };

  const handleImportFromCcSwitch = React.useCallback(async (imported: CcSwitchProviderCandidate[]) => {
    const existingSourceIds = new Set(
      providers.map((provider) => provider.sourceProviderId).filter(Boolean),
    );
    const toImport = imported.filter(
      (candidate) =>
        candidate.sourceProviderId &&
        !existingSourceIds.has(candidate.sourceProviderId),
    );

    let ok = 0;
    let fail = 0;

    for (const candidate of toImport) {
      try {
        const settingsConfig =
          typeof candidate.settingsConfig === 'string'
            ? candidate.settingsConfig
            : JSON.stringify(candidate.settingsConfig);

        const createdProvider = await createClaudeProvider({
          name: candidate.name,
          category: (candidate.normalizedCategory || 'custom') as ClaudeProviderInput['category'],
          settingsConfig,
          extraSettingsConfig: candidate.extraSettingsConfig || '{}',
          extraSettingsMergeStrategy: undefined,
          sourceProviderId: candidate.sourceProviderId,
          websiteUrl: candidate.websiteUrl,
          notes: candidate.notes,
          icon: candidate.icon,
          iconColor: candidate.iconColor,
        });

        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('claudecode', createdProvider.id),
            buildClaudeFavoriteProviderConfig(createdProvider),
          );
        } catch (favoriteError) {
          console.error('Failed to save Claude favorite provider from CC Switch import:', favoriteError);
        }
        ok += 1;
      } catch (error) {
        console.error('Failed to import Claude provider from CC Switch:', candidate.name, error);
        fail += 1;
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

    await loadConfig();
    await loadFavoriteProviders();
    await refreshTrayMenu();
  }, [providers, loadConfig, loadFavoriteProviders, t]);

  const handleImportFavoriteProviders = React.useCallback(async (providersToImport: OpenCodeFavoriteProvider[]) => {
    try {
      let importedCount = 0;
      for (const favoriteProvider of providersToImport) {
        const payload = getFavoriteProviderPayload<ClaudeFavoriteProviderPayload>(favoriteProvider);
        if (!payload) {
          continue;
        }

        const createdProvider = await createClaudeProvider({
          name: payload.name,
          category: payload.category as ClaudeProviderInput['category'],
          settingsConfig: payload.settingsConfig,
          extraSettingsConfig: payload.extraSettingsConfig || '{}',
          extraSettingsMergeStrategy: payload.extraSettingsMergeStrategy,
          meta: payload.meta as ClaudeProviderInput['meta'],
          notes: payload.notes,
          sourceProviderId: extractFavoriteProviderRawId('claudecode', favoriteProvider.providerId),
        });
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('claudecode', createdProvider.id),
            buildClaudeFavoriteProviderConfig(createdProvider),
            favoriteProvider.diagnostics,
          );
        } catch (favoriteError) {
          console.error('Failed to copy Claude favorite provider diagnostics during import:', favoriteError);
        }
        importedCount += 1;
      }

      setImportModalOpen(false);
      message.success(t('opencode.provider.importSuccess', { count: importedCount }));
      await loadConfig();
      await loadFavoriteProviders();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import Claude favorite providers:', error);
      message.error(t('common.error'));
    }
  }, [loadConfig, loadFavoriteProviders, t]);

  const doSaveProvider = async (values: ClaudeProviderFormValues) => {
    try {
      const settingsConfig = buildClaudeProviderSettingsConfig(values);

      // Check if this is a temporary provider from local file
      const isLocalTemp = editingProvider?.id === "__local__";

      const providerInput: ClaudeProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig,
        extraSettingsConfig: values.extraSettingsConfig || '{}',
        extraSettingsMergeStrategy: values.extraSettingsMergeStrategy,
        sourceProviderId: values.sourceProviderId,
        meta: values.meta,
        notes: values.notes,
      };

      let savedProviderId = isLocalTemp ? LOCAL_CONFIG_ID : '';
      let savedProvider: ClaudeCodeProvider | null = null;
      const gatewayModeBeforeSave = resolveGatewayReengageMode(gatewayCliStatus);
      const gatewayAggregateBeforeSave = toGatewayAggregateReengageConfig(gatewayCliStatus);
      const shouldReengageGatewayProxy =
        Boolean(editingProvider && !isCopyMode && !isLocalTemp && editingProvider.isApplied) &&
        gatewayModeBeforeSave !== null;

      await saveProviderWithGatewayReengage({
        gatewayMode: shouldReengageGatewayProxy ? gatewayModeBeforeSave : null,
        aggregateConfig: shouldReengageGatewayProxy ? gatewayAggregateBeforeSave : null,
        restoreDirect: () => restoreProxyGatewayCliDirect('claude'),
        engageSingle: () => engageProxyGatewaySingle('claude', savedProviderId),
        engageFailover: () => engageProxyGatewayFailover('claude'),
        // No `engageAggregate`: aggregate mode is Codex-only, so this CLI can
        // never report it and the re-engage helper would reject the replay.
        onGatewayStatusChange: setGatewayCliStatus,
        saveProvider: async () => {
          if (isLocalTemp) {
            await saveClaudeLocalConfig({ provider: providerInput });
          } else if (editingProvider && !isCopyMode) {
            // Update existing provider
            savedProvider = await updateClaudeProvider({
              id: editingProvider.id,
              name: values.name,
              category: values.category,
              settingsConfig: providerInput.settingsConfig,
              extraSettingsConfig: providerInput.extraSettingsConfig || '{}',
              extraSettingsMergeStrategy:
                providerInput.extraSettingsMergeStrategy || DEFAULT_CLAUDE_SETTINGS_MERGE_STRATEGY,
              sourceProviderId: values.sourceProviderId,
              meta: values.meta,
              notes: values.notes,
              sortIndex: editingProvider.sortIndex,
              isApplied: editingProvider.isApplied,
              isDisabled: editingProvider.isDisabled,
              createdAt: editingProvider.createdAt,
              updatedAt: editingProvider.updatedAt,
            });
            savedProviderId = editingProvider.id;
          } else {
            // 让服务端生成 ID
            savedProvider = await createClaudeProvider(providerInput);
            savedProviderId = savedProvider.id;
          }
        },
      });

      try {
        const providerForFavorite: ClaudeCodeProvider = savedProvider || {
          id: savedProviderId,
          name: values.name,
          category: values.category,
          settingsConfig: providerInput.settingsConfig,
          extraSettingsConfig: providerInput.extraSettingsConfig || '{}',
          extraSettingsMergeStrategy:
            providerInput.extraSettingsMergeStrategy || DEFAULT_CLAUDE_SETTINGS_MERGE_STRATEGY,
          sourceProviderId: values.sourceProviderId,
          meta: values.meta,
          notes: values.notes,
          isApplied: false,
          isDisabled: false,
          createdAt: '',
          updatedAt: '',
        };
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('claudecode', providerForFavorite.id),
          buildClaudeFavoriteProviderConfig(providerForFavorite),
        );
        await loadFavoriteProviders();
      } catch (error) {
        console.error('Failed to save Claude favorite provider:', error);
      }

      message.success(t('common.success'));
      setProviderModalOpen(false);
      setIsCopyMode(false);
      await loadConfig();
      if (shouldReengageGatewayProxy) {
        await refreshTrayMenu();
      }
    } catch (error) {
      console.error('Failed to save provider:', error);
      message.error(t('common.error'));
      throw error;
    }
  };

  const doUpdateProvider = async (id: string, values: ClaudeProviderFormValues) => {
    try {
      const existingProvider = providers.find((p) => p.id === id);
      if (!existingProvider) return;
      const settingsConfig = buildClaudeProviderSettingsConfig(values);

      const providerData: ClaudeCodeProvider = {
        ...existingProvider,
        name: values.name,
        category: values.category,
        settingsConfig,
        extraSettingsConfig: values.extraSettingsConfig || '{}',
        extraSettingsMergeStrategy:
          values.extraSettingsMergeStrategy ||
          existingProvider.extraSettingsMergeStrategy ||
          DEFAULT_CLAUDE_SETTINGS_MERGE_STRATEGY,
        meta: values.meta,
        notes: values.notes,
        createdAt: existingProvider.createdAt,
        updatedAt: existingProvider.updatedAt,
      };

      await updateClaudeProvider(providerData);
      try {
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('claudecode', existingProvider.id),
          buildClaudeFavoriteProviderConfig(providerData),
        );
        await loadFavoriteProviders();
      } catch (error) {
        console.error('Failed to update Claude favorite provider:', error);
      }
      message.success(t('common.success'));
      setProviderModalOpen(false);
      await loadConfig();
    } catch (error) {
      console.error('Failed to update provider:', error);
      message.error(t('common.error'));
      throw error;
    }
  };

  const handlePreviewCurrentConfig = async () => {
    try {
      const settings = await readClaudeSettings();
      const finalConfig: Record<string, unknown> = { ...settings };
      setPreviewDataLocal(finalConfig);
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview config:', error);
      message.error(t('common.error'));
    }
  };

  return (
    <SectionSidebarLayout
      sidebarTitle={t('claudecode.title')}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'claudecode-providers':
            return <DatabaseOutlined />;
          case 'claudecode-global-prompt':
            return <FileTextOutlined />;
          case 'claudecode-plugins':
            return <AppstoreOutlined />;
          case 'claudecode-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'claudecode-providers':
            setProviderListCollapsed(false);
            break;
          case 'claudecode-global-prompt':
            setPromptExpandNonce((v) => v + 1);
            break;
          case 'claudecode-plugins':
            setPluginListCollapsed(false);
            break;
          case 'claudecode-session-manager':
            setSessionManagerExpandNonce((v) => v + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        {/* 页面头部 */}
        <CodingPageHeader
          title={t('claudecode.title')}
          docsUrl="https://code.claude.com/docs/en/settings#environment-variables"
          onPreviewConfig={handlePreviewCurrentConfig}
          configPath={configPath || '~/.claude/settings.json'}
          onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
          onOpenFolder={handleOpenFolder}
          onRefresh={handleRefreshPage}
          onMoreOptions={() => setSettingsModalOpen(true)}
        />

        {/* Provider 列表 */}
        <ProviderListSection
          sectionId="claudecode-providers"
          emptyTextHint={t('claudecode.importFromOpenCode')}
          collapsed={providerListCollapsed}
          onCollapsedChange={setProviderListCollapsed}
          loading={loading}
          providerCount={providers.length}
          visibleCount={visibleProviders.length}
          batch={providerBatch}
          batchSelectableIds={batchSelectableIds}
          keyword={providerKeyword}
          onKeywordChange={setProviderKeyword}
          sortMode={sortMode}
          sortModes={PROVIDER_SORT_MODES}
          onSortModeChange={setSortMode}
          dragDisabledBySort={sortMode !== 'custom'}
          locateProviderId={appliedProviderId}
          onBatchTest={handleBatchTestProviders}
          batchTesting={batchTestingProviders}
          onOpenCommonConfig={() => setCommonConfigModalOpen(true)}
          onAddProvider={handleAddProvider}
          headerExtra={
            <GatewayFailoverButton
              cliKey="claude"
              status={gatewayCliStatus}
              primaryProviderNeedsGatewayProxy={primaryGatewayProviderNeedsProxy}
              primaryProviderNeedsProxyReason={primaryGatewayProviderNeedsProxyReason}
              onStatusChange={setGatewayCliStatus}
            />
          }
          hint={
            <>
              <div>{t('claudecode.pageHint')}</div>
              <div>{t('claudecode.pageWarning')}</div>
            </>
          }
          footer={
            <Space wrap>
              <Button
                type="dashed"
                icon={<ImportOutlined />}
                onClick={() => setImportModalOpen(true)}
              >
                {t('opencode.provider.importFavorite')}
              </Button>
              {allApiHubAvailable && (
                <Button
                  type="dashed"
                  icon={<AllApiHubIcon />}
                  onClick={() => setAllApiHubImportModalOpen(true)}
                >
                  {t('common.allApiHub.importFromAllApiHub')}
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
          }
        >
          <DndContext
            sensors={providerBatchDragDisabled ? [] : sensors}
            collisionDetection={closestCenter}
            modifiers={[restrictToVerticalAxis]}
            onDragEnd={handleDragEnd}
          >
            <SortableContext
              items={providers.map((p) => p.id)}
              strategy={verticalListSortingStrategy}
            >
              <div>
                {visibleProviders.map((provider) => (
                  <ClaudeProviderCard
                    key={provider.id}
                    provider={provider}
                    isApplied={provider.id === appliedProviderId}
                    onEdit={handleEditProvider}
                    onDelete={handleDeleteProvider}
                    onCopy={handleCopyProvider}
                    onShare={handleShareProvider}
                    onTest={handleTestProvider}
                    onSelect={handleSelectProvider}
                    onToggleDisabled={handleToggleDisabled}
                    selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.id)}
                    selected={providerBatch.selectedIds.has(provider.id)}
                    onSelectChange={(checked) =>
                      providerBatch.toggleSelect(provider.id, checked)
                    }
                    connectivityStatus={connectivityStatuses[provider.id]}
                    gatewayTakeoverActive={gatewayTakeoverActive}
                    gatewayStatus={gatewayCliStatus}
                    onGatewayStatusChange={async (status) => {
                      setGatewayCliStatus(status);
                      await loadConfig();
                    }}
                    dragDisabled={providerDragDisabled}
                  />
                ))}
              </div>
            </SortableContext>
          </DndContext>
        </ProviderListSection>

        <div
          id="claudecode-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('common.prompt.title')}
        >
          <GlobalPromptSettings
            key={`claudecode-prompt-${promptExpandNonce}`}
            toolName="Claude Code"
            promptFileName="CLAUDE.md"
            service={claudeCodePromptApi}
            collapseKey="claudecode-prompt"
            refreshKey={claudeProviderRefreshKey}
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <div
          id="claudecode-plugins"
          data-sidebar-section="true"
          data-sidebar-title={t('claudecode.plugins.title')}
        >
          <Collapse
            style={{ marginBottom: 16 }}
            activeKey={pluginListCollapsed ? [] : ['plugins']}
            onChange={(keys) => setPluginListCollapsed(!keys.includes('plugins'))}
            items={[
              {
                key: 'plugins',
                label: (
                  <Text strong>
                    <AppstoreOutlined style={{ marginRight: 8 }} />
                    {t('claudecode.plugins.title')}
                  </Text>
                ),
                extra: (
                  <Button
                    type="link"
                    size="small"
                    style={{ fontSize: 12 }}
                    icon={<SyncOutlined />}
                    onClick={(event) => {
                      event.stopPropagation();
                      loadConfig(true);
                    }}
                  >
                    {t('common.refresh')}
                  </Button>
                ),
                children: (
                  <ClaudePluginsPanel refreshToken={claudeProviderRefreshKey + pluginPanelRefreshToken} />
                ),
              },
            ]}
          />
        </div>

        <div
          id="claudecode-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title')}
        >
          <SessionManagerPanel tool="claudecode" expandNonce={sessionManagerExpandNonce} />
        </div>

        {/* 模态框 */}
        {providerModalOpen && (
          <ClaudeProviderFormModal
            open={providerModalOpen}
            provider={editingProvider}
            isCopy={isCopyMode}
            mode={providerModalMode}
            onCancel={() => {
              setProviderModalOpen(false);
              setEditingProvider(null);
              setIsCopyMode(false);
            }}
            onSubmit={handleProviderSubmit}
          />
        )}

        <CommonConfigModal
          open={commonConfigModalOpen}
          onCancel={() => setCommonConfigModalOpen(false)}
          onSuccess={() => {
            setCommonConfigModalOpen(false);
            message.success(t('common.success'));
          }}
          isLocalProvider={providers.some((provider) => provider.id === LOCAL_CONFIG_ID)}
          gatewaySaveLocked={gatewayTakeoverActive}
        />

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(rootPathInfo)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        {settingsModalOpen && (
          <ClaudeCodeSettingsModal
            open={settingsModalOpen}
            onClose={() => setSettingsModalOpen(false)}
            sidebarVisible={!sidebarHidden}
            onSidebarVisibleChange={(visible) => setSidebarHidden('claudecode', !visible)}
          />
        )}

        <ImportConflictDialog
          open={conflictDialogOpen}
          conflictInfo={conflictInfo}
          onResolve={handleConflictResolve}
          onCancel={() => {
            setConflictDialogOpen(false);
            setConflictInfo(null);
            setPendingFormValues(null);
          }}
        />

        {allApiHubAvailable && (
          <ImportFromAllApiHubModal
            open={allApiHubImportModalOpen}
            existingProviderIds={providers.map((provider) => provider.sourceProviderId || provider.id)}
            onCancel={() => setAllApiHubImportModalOpen(false)}
            onImport={handleImportFromAllApiHub}
          />
        )}

        {ccSwitchAvailable && (
          <ImportFromCcSwitchModal
            open={ccSwitchImportModalOpen}
            appType="claude"
            existingProviderIds={providers
              .map((provider) => provider.sourceProviderId)
              .filter((id): id is string => Boolean(id))}
            onClose={() => setCcSwitchImportModalOpen(false)}
            onImport={handleImportFromCcSwitch}
          />
        )}

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
          gatewayCliKey="claude"
          useGateway={connectivityUsesGateway}
          diagnostics={connectivityInfo ? findDiagnosticsForProvider(favoriteProviders, 'claudecode', connectivityInfo.providerId) : undefined}
          onSaveDiagnostics={handleSaveConnectivityDiagnostics}
          onCancel={() => setConnectivityModalOpen(false)}
        />

        <ImportProviderModal
          open={importModalOpen}
          onClose={() => setImportModalOpen(false)}
          onImport={handleImportFavoriteProviders}
          existingProviderIds={providers.map((provider) => buildFavoriteProviderStorageKey('claudecode', provider.id))}
          providerFilter={(provider) => isFavoriteProviderForSource('claudecode', provider)}
        />

        <ShareProviderModal
          open={shareProvider !== null}
          sourceApp="claude"
          provider={shareProvider}
          onClose={() => setShareProvider(null)}
        />

        {/* Preview Modal */}
        <JsonPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('claudecode.preview.currentConfigTitle')}
          data={previewData}
        />
      </div>
    </SectionSidebarLayout>
  );
};

export default ClaudeCodePage;
