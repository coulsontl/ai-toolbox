import React from 'react';
import { Typography, Button, Space, Empty, message, Modal, Spin, Collapse, Descriptions } from 'antd';
import { PlusOutlined, FolderOpenOutlined, AppstoreOutlined, SyncOutlined, EyeOutlined, ExclamationCircleOutlined, LinkOutlined, EllipsisOutlined, DatabaseOutlined, ImportOutlined, FileTextOutlined, ThunderboltOutlined, EditOutlined, MessageOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
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
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
  arrayMove,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import type {
  GrokProvider,
  GrokOfficialAccount,
  GrokProviderFormValues,
  GrokProviderInput,
  ConfigPathInfo,
  GrokSettings,
  GrokSettingsConfig,
  ImportConflictInfo,
  ImportConflictAction,
} from '@/types/grok';
import {
  getGrokConfigFilePath,
  getGrokRootPathInfo,
  getGrokCommonConfig,
  listGrokProviders,
  listGrokOfficialAccounts,
  startGrokOfficialAccountDeviceAuth,
  saveGrokOfficialLocalAccount,
  applyGrokOfficialAccount,
  deleteGrokOfficialAccount,
  refreshGrokOfficialAccount,
  selectGrokProvider,
  readGrokSettings,
  createGrokProvider,
  updateGrokProvider,
  saveGrokLocalConfig,
  saveGrokCommonConfig,
  deleteGrokProvider,
  toggleGrokProviderDisabled,
  reorderGrokProviders,
  type GrokDeviceAuthStartResult,
} from '@/services/grokApi';
import { grokPromptApi } from '@/services/grokPromptApi';
import { refreshTrayMenu, hasAllApiHubExtension } from '@/services/appApi';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import { useSettingsStore } from '@/stores';
import GrokProviderCard from '../components/GrokProviderCard';
import GrokProviderFormModal from '../components/GrokProviderFormModal';
import GrokCommonConfigModal from '../components/GrokCommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import GrokPluginsPanel from '../components/GrokPluginsPanel';
import GrokDeviceAuthModal from '../components/GrokDeviceAuthModal';
import { GROK_LOCAL_PROVIDER_ID, shouldLoadGrokOfficialAccounts } from '../utils/localProvider';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import GrokConfigPreviewModal from '@/components/common/GrokConfigPreviewModal';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import {
  areGatewayProviderProfilesInitialized,
  grokWireApiFormatFromConfig,
  firstGatewayApiFormat,
  GatewayFailoverButton,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  openAiApiFormatFromBaseUrl,
  saveProviderWithGatewayReengage,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import ProviderConnectivityTestModal, {
  buildGrokProviderConnectivityInfo,
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import { SessionManagerPanel, type SessionSourceMode } from '@/features/coding/shared/sessionManager';
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
  findDefaultTestModelIdForProvider,
  findDiagnosticsForProvider,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  mergeDiagnosticsIntoFavoriteProviders,
  type GrokFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import {
  extractGrokSettingsApiBackend,
  extractGrokSettingsBaseUrl,
  extractGrokSettingsModel,
} from '@/utils/grokConfigUtils';
import { parseGrokSettingsConfig } from '../utils/grokSettingsConfig';
import {
  engageProxyGatewayFailover,
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  type GatewayCliTakeoverStatus,
} from '@/services';

const { Title, Text, Link } = Typography;

function buildGrokFavoriteProviderConfig(provider: GrokProvider) {
  const settingsConfig = parseGrokSettingsConfig(provider.settingsConfig);
  const catalogModels = settingsConfig.modelCatalog?.models || [];
  const defaultModelKey = settingsConfig.defaultModelKey?.trim();
  const selectedModel = catalogModels.find(
    (catalogModel) => catalogModel.key === defaultModelKey || catalogModel.model === defaultModelKey,
  ) || catalogModels[0];
  const baseUrl = selectedModel?.baseUrl?.trim() || extractGrokSettingsBaseUrl(settingsConfig)?.trim();
  const modelIds = catalogModels.map((catalogModel) => catalogModel.model.trim()).filter(Boolean);
  const fallbackModelId = defaultModelKey || extractGrokSettingsModel(settingsConfig)?.trim();

  return buildFavoriteProviderOptions(
    {
      npm: '@ai-sdk/openai',
      name: provider.name,
      options: {
        ...(baseUrl ? { baseURL: baseUrl } : {}),
        ...(settingsConfig.auth?.API_KEY?.trim()
          ? { apiKey: settingsConfig.auth.API_KEY.trim() }
          : {}),
      },
      models: Object.fromEntries(
        (modelIds.length > 0 ? modelIds : fallbackModelId ? [fallbackModelId] : [])
          .map((modelId) => [modelId, {}]),
      ),
    },
    {
      name: provider.name,
      category: provider.category,
      settingsConfig: provider.settingsConfig,
      ...(provider.meta ? { meta: provider.meta } : {}),
      ...(provider.notes ? { notes: provider.notes } : {}),
    } satisfies GrokFavoriteProviderPayload,
  );
}

const ACCOUNT_DETAILS_EMPTY_VALUE = '-';
let rememberedGrokSessionSourceMode: SessionSourceMode = 'all';

const GrokPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const {
    sidebarHiddenByPage,
    setSidebarHidden,
  } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [providers, setProviders] = React.useState<GrokProvider[]>([]);
  const [officialAccountsByProviderId, setOfficialAccountsByProviderId] = React.useState<
    Record<string, GrokOfficialAccount[]>
  >({});
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
    if (!primaryProvider || primaryProvider.category === 'official' || primaryProvider.id === GROK_LOCAL_PROVIDER_ID) {
      return false;
    }
    const settingsConfig = parseGrokSettingsConfig(primaryProvider.settingsConfig) as GrokSettingsConfig & {
      apiFormat?: unknown;
      api_format?: unknown;
    };
    const baseUrl = extractGrokSettingsBaseUrl(settingsConfig);
    const providerApiFormat = firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(primaryProvider.meta, 'grok'),
      primaryProvider.meta?.apiFormat,
      typeof settingsConfig.apiFormat === 'string' ? settingsConfig.apiFormat : undefined,
      typeof settingsConfig.api_format === 'string' ? settingsConfig.api_format : undefined,
      extractGrokSettingsApiBackend(settingsConfig),
      grokWireApiFormatFromConfig(settingsConfig.config),
      openAiApiFormatFromBaseUrl(baseUrl),
    );
    return providerApiFormat === 'gemini_native';
  }, [gatewayCliStatus?.primary_provider_id, gatewayProviderProfilesVersion, providers]);
  const [refreshingOfficialAccountId, setRefreshingOfficialAccountId] = React.useState<string | null>(null);
  const [savingOfficialAccountId, setSavingOfficialAccountId] = React.useState<string | null>(null);
  const [officialAccountDetails, setOfficialAccountDetails] = React.useState<{
    provider: GrokProvider;
    account: GrokOfficialAccount;
  } | null>(null);

  // Modal states
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<GrokProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [providerModalMode, setProviderModalMode] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [deviceAuthSession, setDeviceAuthSession] = React.useState<GrokDeviceAuthStartResult | null>(null);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<GrokProviderFormValues | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<GrokSettings | null>(null);
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
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [pluginListCollapsed, setPluginListCollapsed] = React.useState(true);
  const [pluginPanelRefreshToken, setPluginPanelRefreshToken] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [sessionManagerRefreshNonce] = React.useState(0);
  const [sessionSourceMode, setSessionSourceMode] = React.useState<SessionSourceMode>(() => rememberedGrokSessionSourceMode);

  const handleSessionSourceModeChange = React.useCallback((sourceMode: SessionSourceMode) => {
    rememberedGrokSessionSourceMode = sourceMode;
    setSessionSourceMode(sourceMode);
  }, []);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const sidebarHidden = sidebarHiddenByPage.grok;

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
      id: 'grok-providers',
      title: t('grok.provider.title'),
      order: 1,
    },
    {
      id: 'grok-global-prompt',
      title: t('grok.prompt.title'),
      order: 2,
    },
    {
      id: 'grok-plugins',
      title: t('grok.plugins.title'),
      order: 3,
    },
    {
      id: 'grok-session-manager',
      title: t('sessionManager.title'),
      order: 4,
    },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [path, nextRootPathInfo, providerList] = await Promise.all([
        getGrokConfigFilePath(),
        getGrokRootPathInfo(),
        listGrokProviders(),
      ]);
      setConfigPath(path);
      setRootPathInfo(nextRootPathInfo);
      setProviders(providerList);
      const officialAccountEntries = await Promise.all(
        providerList.map(async (provider) => [
          provider.id,
          shouldLoadGrokOfficialAccounts(provider)
            ? await listGrokOfficialAccounts(provider.id)
            : [],
        ] as const),
      );
      setOfficialAccountsByProviderId(Object.fromEntries(officialAccountEntries));
      setPluginPanelRefreshToken((value) => value + 1);
      const applied = providerList.find((p) => p.isApplied);
      setAppliedProviderId(applied?.id || '');
    } catch (error) {
      console.error('Failed to load config:', error);
      if (!silent) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  const loadFavoriteProviders = React.useCallback(async () => {
    try {
      const allFavoriteProviders = await listFavoriteProviders();
      const grokFavoriteProviders = allFavoriteProviders.filter((provider) =>
        isFavoriteProviderForSource('grok', provider),
      );
      const currentStorageKeys = new Set(
        providers.map((provider) => buildFavoriteProviderStorageKey('grok', provider.id)),
      );
      const { keptProviders, duplicateIds } = dedupeFavoriteProvidersByPayload(
        grokFavoriteProviders,
        currentStorageKeys,
      );

      if (duplicateIds.length > 0) {
        await Promise.all(
          duplicateIds.map(async (providerId) => {
            try {
              await deleteFavoriteProvider(providerId);
            } catch (error) {
              console.error('Failed to delete duplicate Grok favorite provider:', error);
            }
          }),
        );
      }

      setFavoriteProviders(keptProviders);
    } catch (error) {
      console.error('Failed to load Grok favorite providers:', error);
    }
  }, [providers]);

  React.useEffect(() => {
    loadConfig();
  }, [loadConfig]);

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

  // 从其他 Tab 切回时刷新数据
  const hasInitializedRef = React.useRef(false);
  React.useEffect(() => {
    if (!isActive) {
      hasInitializedRef.current = true;
      return;
    }
    if (hasInitializedRef.current) {
      loadConfig(true);
    }
  }, [isActive, loadConfig]);

  React.useEffect(() => {
    const handleTrayConfigRefresh = (event: Event) => {
      event.preventDefault();
      void loadConfig(true);
    };

    window.addEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    return () => {
      window.removeEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    };
  }, [loadConfig]);

  React.useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<string[]>('grok-config-warning', (event) => {
      if (event.payload.length > 0) {
        message.warning(t('grok.apply.preservedModelWarning', {
          models: event.payload.map((modelKey) => `[model.${modelKey}]`).join(', '),
        }));
      }
    }).then((dispose) => {
      unlisten = dispose;
    }).catch((error) => {
      console.error('Failed to listen for Grok config warnings:', error);
    });

    return () => {
      unlisten?.();
    };
  }, [t]);

  React.useEffect(() => {
    const checkAllApiHubAvailability = async () => {
      try {
        const available = await hasAllApiHubExtension();
        setAllApiHubAvailable(available);
      } catch {
        setAllApiHubAvailable(false);
      }
    };

    checkAllApiHubAvailability();
  }, []);

  const handleOpenFolder = async () => {
    if (!configPath) return;

    try {
      await revealItemInDir(configPath);
    } catch {
      try {
        const parentDir = configPath.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open folder:', error);
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
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
    translationKeyPrefix: 'grok',
    defaultConfig: '',
    rootDirectoryChangeLocked: gatewayTakeoverActive,
    rootDirectoryChangeLockedText: t('gateway.proxy.rootDirectorySaveLockedTooltip'),
    loadConfig,
    getCommonConfig: getGrokCommonConfig,
    saveCommonConfig: saveGrokCommonConfig,
  });

  const handleSelectProvider = async (provider: GrokProvider) => {
    try {
      await selectGrokProvider(provider.id);
      message.success(t('grok.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to select provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleToggleDisabled = async (provider: GrokProvider, isDisabled: boolean) => {
    try {
      await toggleGrokProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('grok.providerDisabled') : t('grok.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle provider disabled status:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleStartOfficialAccountOauth = async (provider: GrokProvider) => {
    try {
      setDeviceAuthSession(await startGrokOfficialAccountDeviceAuth(provider.id));
    } catch (error) {
      console.error('Failed to start Grok official account OAuth:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleApplyOfficialAccount = async (
    _provider: GrokProvider,
    account: GrokOfficialAccount,
  ) => {
    try {
      await applyGrokOfficialAccount(account.id);
      message.success(t('grok.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to apply Grok official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleSaveOfficialLocalAccount = async (
    provider: GrokProvider,
    account: GrokOfficialAccount,
  ) => {
    try {
      setSavingOfficialAccountId(account.id);
      await saveGrokOfficialLocalAccount(provider.id);
      if (officialAccountDetails?.account.id === account.id) {
        setOfficialAccountDetails(null);
      }
      message.success(t('grok.provider.officialAccountSaveSuccess'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Grok local official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setSavingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleDeleteOfficialAccount = async (
    _provider: GrokProvider,
    account: GrokOfficialAccount,
  ) => {
    Modal.confirm({
      title: t('grok.provider.officialAccountDeleteConfirm', {
        name: account.email || account.name,
      }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteGrokOfficialAccount(account.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete Grok official account:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        }
      },
    });
  };

  const handleRefreshOfficialAccount = async (
    provider: GrokProvider,
    account: GrokOfficialAccount,
  ) => {
    try {
      setRefreshingOfficialAccountId(account.id);
      const refreshedAccount = await refreshGrokOfficialAccount(account.id);
      setOfficialAccountsByProviderId((previous) => ({
        ...previous,
        [provider.id]: (previous[provider.id] || []).map((currentAccount) =>
          currentAccount.id === refreshedAccount.id ? refreshedAccount : currentAccount,
        ),
      }));
      setOfficialAccountDetails((current) => {
        if (!current || current.account.id !== refreshedAccount.id) {
          return current;
        }
        return {
          provider: current.provider,
          account: refreshedAccount,
        };
      });
      message.success(t('grok.provider.officialAccountRefreshSuccess'));
    } catch (error) {
      console.error('Failed to refresh Grok official account usage:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setRefreshingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleViewOfficialAccountDetails = (
    provider: GrokProvider,
    account: GrokOfficialAccount,
  ) => {
    setOfficialAccountDetails({ provider, account });
  };

  const formatDateTime = React.useCallback((value?: string | null) => {
    if (!value) {
      return ACCOUNT_DETAILS_EMPTY_VALUE;
    }
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }
    return date.toLocaleString();
  }, []);

  const formatUnixTimestamp = React.useCallback((value?: number | null) => {
    if (value == null) {
      return ACCOUNT_DETAILS_EMPTY_VALUE;
    }
    return new Date(value * 1000).toLocaleString();
  }, []);

  // 拖拽排序处理
  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const oldIndex = providers.findIndex((p) => p.id === active.id);
    const newIndex = providers.findIndex((p) => p.id === over.id);
    const oldProviders = [...providers];
    const newProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(newProviders);

    try {
      await reorderGrokProviders(newProviders.map((p) => p.id));
      await refreshTrayMenu();
    } catch (error) {
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

  const handleImportFromOpenCode = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalMode('import');
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: GrokProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: GrokProvider) => {
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

  const handleTestProvider = (provider: GrokProvider) => {
    if (provider.category === 'official') {
      message.info(t('grok.provider.officialConnectivityHint'));
      return;
    }
    if (!areGatewayProviderProfilesInitialized()) {
      message.info(t('common.loading'));
      return;
    }

    const settingsConfig = parseGrokSettingsConfig(provider.settingsConfig) as GrokSettingsConfig & {
      apiFormat?: unknown;
      api_format?: unknown;
    };
    const baseUrl = extractGrokSettingsBaseUrl(settingsConfig);
    const providerApiFormat = firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(provider.meta, 'grok'),
      provider.meta?.apiFormat,
      typeof settingsConfig.apiFormat === 'string' ? settingsConfig.apiFormat : undefined,
      typeof settingsConfig.api_format === 'string' ? settingsConfig.api_format : undefined,
      extractGrokSettingsApiBackend(settingsConfig),
      grokWireApiFormatFromConfig(settingsConfig.config),
      openAiApiFormatFromBaseUrl(baseUrl),
    );
    setConnectivityInfo(buildGrokProviderConnectivityInfo(provider));
    setConnectivityUsesGateway(providerApiFormat === 'gemini_native');
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
        buildFavoriteProviderStorageKey('grok', targetProvider.id),
        buildGrokFavoriteProviderConfig(targetProvider),
        diagnostics,
      );
      setFavoriteProviders((previousProviders) =>
        mergeDiagnosticsIntoFavoriteProviders(previousProviders, favoriteProvider, 'grok'),
      );
    } catch (error) {
      console.error('Failed to save Grok connectivity diagnostics:', error);
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
      message.info(t('grok.provider.officialBatchSkipped', { count: officialProviders.length }));
    }

    if (testableProviders.length === 0) {
      setConnectivityStatuses({});
      return;
    }

    const targets = testableProviders.map((provider) => {
      const connectivityInfo = buildGrokProviderConnectivityInfo(provider);
      const settingsConfig = parseGrokSettingsConfig(provider.settingsConfig) as GrokSettingsConfig & {
        apiFormat?: unknown;
        api_format?: unknown;
      };
      const baseUrl = extractGrokSettingsBaseUrl(settingsConfig);
      const hasExplicitBaseUrl = Boolean(baseUrl);
      const providerApiFormat = firstGatewayApiFormat(
        getGatewayProviderApiFormatFromMeta(provider.meta, 'grok'),
        provider.meta?.apiFormat,
        typeof settingsConfig.apiFormat === 'string' ? settingsConfig.apiFormat : undefined,
        typeof settingsConfig.api_format === 'string' ? settingsConfig.api_format : undefined,
        extractGrokSettingsApiBackend(settingsConfig),
        grokWireApiFormatFromConfig(settingsConfig.config),
        openAiApiFormatFromBaseUrl(baseUrl),
      );
      const useGateway = providerApiFormat === 'gemini_native';

      if (provider.category !== 'official' && !hasExplicitBaseUrl) {
        return {
          providerId: provider.id,
          errorMessage: t('common.baseUrlMissing'),
        };
      }

      return buildProviderConnectivityBatchTarget(connectivityInfo, {
        requireBaseUrl: false,
        requireApiKey: !useGateway,
        gatewayCliKey: 'grok',
        useGateway,
        preferredModelId: findDefaultTestModelIdForProvider(favoriteProviders, 'grok', provider.id),
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
      console.error('Failed to batch test Grok providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [providers, t, favoriteProviders, gatewayProviderProfilesVersion]);

  const handleDeleteProvider = (provider: GrokProvider) => {
    const performDelete = async () => {
      try {
        await deleteGrokProvider(provider.id);
        await loadFavoriteProviders();
        message.success(t('common.success'));
        await loadConfig();
        await refreshTrayMenu();
      } catch (error) {
        console.error('Failed to delete provider:', error);
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    };

    Modal.confirm({
      title: t('grok.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('grok', provider.id),
            buildGrokFavoriteProviderConfig(provider),
          );
          await performDelete();
        } catch (favoriteError) {
          console.error('Failed to preserve Grok favorite provider before deletion:', favoriteError);
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

  const handleProviderSubmit = async (values: GrokProviderFormValues) => {
    // Check for conflicts
    if (values.sourceProviderId && !editingProvider) {
      const existingProvider = providers.find(
        (p) => p.sourceProviderId === values.sourceProviderId
      );

      if (existingProvider) {
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
      await doUpdateProvider(conflictInfo.existingProvider.id, pendingFormValues);
    } else {
      await doSaveProvider({
        ...pendingFormValues,
        sourceProviderId: undefined,
      });
    }

    setConflictDialogOpen(false);
    setConflictInfo(null);
    setPendingFormValues(null);
  };

  const handleImportFromAllApiHub = async (imported: OpenCodeAllApiHubProvider[]) => {
    try {
      for (const item of imported) {
        const baseUrl = item.providerConfig.options?.baseURL || '';
        const apiKey = item.providerConfig.options?.apiKey || '';
        const importedModelIds = Object.keys(item.providerConfig.models || {});
        const defaultModelKey = importedModelIds[0];

        const providerInput: GrokProviderInput = {
          name: item.name,
          category: 'custom',
          settingsConfig: JSON.stringify({
            auth: apiKey ? { API_KEY: apiKey } : {},
            config: '',
            ...(defaultModelKey ? { defaultModelKey } : {}),
            modelCatalog: {
              models: importedModelIds.map((modelId) => ({
                key: modelId,
                model: modelId,
                displayName: modelId,
                ...(baseUrl ? { baseUrl } : {}),
                apiBackend: 'chat_completions',
              })),
            },
          }),
          sourceProviderId: item.providerId,
          notes: undefined,
        };

        const createdProvider = await createGrokProvider(providerInput);
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('grok', createdProvider.id),
            buildGrokFavoriteProviderConfig(createdProvider),
          );
        } catch (favoriteError) {
          console.error('Failed to save Grok favorite provider from All API Hub import:', favoriteError);
        }
      }

      message.success(t('common.allApiHub.importSuccess', { count: imported.length }));
      setAllApiHubImportModalOpen(false);
      await loadConfig();
      await loadFavoriteProviders();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from All API Hub:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleImportFavoriteProviders = React.useCallback(async (providersToImport: OpenCodeFavoriteProvider[]) => {
    try {
      let importedCount = 0;
      for (const favoriteProvider of providersToImport) {
        const payload = getFavoriteProviderPayload<GrokFavoriteProviderPayload>(favoriteProvider);
        if (!payload) {
          continue;
        }

        const createdProvider = await createGrokProvider({
          name: payload.name,
          category: payload.category as GrokProviderInput['category'],
          settingsConfig: payload.settingsConfig,
          meta: payload.meta as GrokProviderInput['meta'],
          notes: payload.notes,
        });
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('grok', createdProvider.id),
            buildGrokFavoriteProviderConfig(createdProvider),
            favoriteProvider.diagnostics,
          );
        } catch (favoriteError) {
          console.error('Failed to copy Grok favorite provider diagnostics during import:', favoriteError);
        }
        importedCount += 1;
      }

      setImportModalOpen(false);
      message.success(t('opencode.provider.importSuccess', { count: importedCount }));
      await loadConfig();
      await loadFavoriteProviders();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import Grok favorite providers:', error);
      message.error(t('common.error'));
    }
  }, [loadConfig, loadFavoriteProviders, t]);

  const doSaveProvider = async (values: GrokProviderFormValues) => {
    try {
      // 新架构：直接使用 settingsConfig（由 Hook 构建）
      // 旧架构：手动构建（向后兼容）
      let settingsConfig: string;
      if (values.settingsConfig) {
        settingsConfig = values.settingsConfig;
      } else {
        const settingsConfigObj: GrokSettingsConfig =
          values.category === 'official'
            ? {
                auth: {},
                config: values.configToml || '',
              }
            : {
                auth: values.apiKey ? { API_KEY: values.apiKey } : {},
                config: values.configToml || '',
              };

        if (values.category !== 'official' && values.model) {
          settingsConfigObj.defaultModelKey = values.model;
          settingsConfigObj.modelCatalog = {
            models: [{
              key: values.model,
              model: values.model,
              displayName: values.model,
              ...(values.baseUrl ? { baseUrl: values.baseUrl } : {}),
              apiBackend: values.apiFormat === 'openai_responses'
                ? 'responses'
                : values.apiFormat === 'anthropic_messages'
                  ? 'messages'
                  : 'chat_completions',
            }],
          };
        }

        settingsConfig = JSON.stringify(settingsConfigObj);
      }

      // Check if this is a temporary provider from local files
      const isLocalTemp = editingProvider?.id === GROK_LOCAL_PROVIDER_ID;

      const providerInput: GrokProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig,
        sourceProviderId: values.sourceProviderId,
        meta: values.meta,
        notes: values.notes,
      };

      let savedProviderId = isLocalTemp ? GROK_LOCAL_PROVIDER_ID : '';
      let savedProvider: GrokProvider | null = null;
      const gatewayModeBeforeSave = gatewayCliStatus?.mode;
      const shouldReengageGatewayProxy =
        Boolean(editingProvider && !isCopyMode && !isLocalTemp && editingProvider.isApplied) &&
        (gatewayModeBeforeSave === 'single' || gatewayModeBeforeSave === 'failover');

      await saveProviderWithGatewayReengage({
        gatewayMode: shouldReengageGatewayProxy ? gatewayModeBeforeSave : null,
        restoreDirect: () => restoreProxyGatewayCliDirect('grok'),
        engageSingle: () => engageProxyGatewaySingle('grok', savedProviderId),
        engageFailover: () => engageProxyGatewayFailover('grok'),
        onGatewayStatusChange: setGatewayCliStatus,
        saveProvider: async () => {
          if (isLocalTemp) {
            await saveGrokLocalConfig({ provider: providerInput });
          } else if (editingProvider && !isCopyMode) {
            savedProvider = await updateGrokProvider({
              id: editingProvider.id,
              name: values.name,
              category: values.category,
              settingsConfig: providerInput.settingsConfig,
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
            savedProvider = await createGrokProvider(providerInput);
            savedProviderId = savedProvider.id;
          }
        },
      });

      try {
        const providerForFavorite: GrokProvider = savedProvider || {
          id: savedProviderId,
          name: values.name,
          category: values.category,
          settingsConfig,
          meta: values.meta,
          notes: values.notes,
          isApplied: false,
          isDisabled: false,
          createdAt: '',
          updatedAt: '',
        };
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('grok', providerForFavorite.id),
          buildGrokFavoriteProviderConfig(providerForFavorite),
        );
        await loadFavoriteProviders();
      } catch (error) {
        console.error('Failed to save Grok favorite provider:', error);
      }

      message.success(t('common.success'));
      setProviderModalOpen(false);
      setIsCopyMode(false);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
      throw error;
    }
  };

  const doUpdateProvider = async (id: string, values: GrokProviderFormValues) => {
    try {
      const existingProvider = providers.find((p) => p.id === id);
      if (!existingProvider) return;

      // 新架构：直接使用 settingsConfig（由 Hook 构建）
      // 旧架构：手动构建（向后兼容）
      let settingsConfig: string;
      if (values.settingsConfig) {
        settingsConfig = values.settingsConfig;
      } else {
        const settingsConfigObj: GrokSettingsConfig =
          values.category === 'official'
            ? {
                auth: {},
                config: values.configToml || '',
              }
            : {
                auth: values.apiKey ? { API_KEY: values.apiKey } : {},
                config: values.configToml || '',
              };

        if (values.category !== 'official' && values.model) {
          settingsConfigObj.defaultModelKey = values.model;
          settingsConfigObj.modelCatalog = {
            models: [{
              key: values.model,
              model: values.model,
              displayName: values.model,
              ...(values.baseUrl ? { baseUrl: values.baseUrl } : {}),
              apiBackend: values.apiFormat === 'openai_responses'
                ? 'responses'
                : values.apiFormat === 'anthropic_messages'
                  ? 'messages'
                  : 'chat_completions',
            }],
          };
        }

        settingsConfig = JSON.stringify(settingsConfigObj);
      }

      const providerData: GrokProvider = {
        ...existingProvider,
        name: values.name,
        category: values.category,
        settingsConfig,
        meta: values.meta,
        notes: values.notes,
        isDisabled: existingProvider.isDisabled,
        createdAt: existingProvider.createdAt,
        updatedAt: existingProvider.updatedAt,
      };

      await updateGrokProvider(providerData);
      try {
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('grok', existingProvider.id),
          buildGrokFavoriteProviderConfig(providerData),
        );
        await loadFavoriteProviders();
      } catch (error) {
        console.error('Failed to update Grok favorite provider:', error);
      }
      message.success(t('common.success'));
      setProviderModalOpen(false);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to update provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
      throw error;
    }
  };

  const handlePreviewCurrentConfig = async () => {
    try {
      const settings = await readGrokSettings();
      setPreviewDataLocal(settings);
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  return (
    <SectionSidebarLayout
      sidebarTitle={t('grok.title')}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'grok-providers':
            return <DatabaseOutlined />;
          case 'grok-global-prompt':
            return <FileTextOutlined />;
          case 'grok-plugins':
            return <AppstoreOutlined />;
          case 'grok-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'grok-providers':
            setProviderListCollapsed(false);
            break;
          case 'grok-global-prompt':
            setPromptExpandNonce((v) => v + 1);
            break;
          case 'grok-plugins':
            setPluginListCollapsed(false);
            break;
          case 'grok-session-manager':
            setSessionManagerExpandNonce((v) => v + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        {/* Page Header */}
        <div style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
            <div>
              <div style={{ marginBottom: 8 }}>
                <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                  {t('grok.title')}
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openUrl('https://docs.x.ai/build/overview');
                  }}
                >
                  <LinkOutlined /> {t('grok.viewDocs')}
                </Link>
                {appliedProviderId && (
                  <Link
                    type="secondary"
                    style={{ fontSize: 12, marginLeft: 16 }}
                    onClick={(e) => {
                      e.stopPropagation();
                      handlePreviewCurrentConfig();
                    }}
                  >
                    <EyeOutlined /> {t('common.previewConfig')}
                  </Link>
                )}
              </div>
              <Space size="small">
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('grok.configPath')}:
                </Text>
                <Text code style={{ fontSize: 12 }}>
                  {configPath || '~/.grok/config.toml'}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => setRootDirectoryModalOpen(true)}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('grok.rootPathSource.customize')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenFolder}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('grok.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<SyncOutlined />}
                  onClick={handleRefreshPage}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('grok.refreshConfig')}
                </Button>
              </Space>
            </div>

            <Space>
              <Button type="text" icon={<EllipsisOutlined />} onClick={() => setSettingsModalOpen(true)}>
                {t('common.moreOptions')}
              </Button>
            </Space>
          </div>
        </div>

        {/* Provider List */}
        <div
          id="grok-providers"
          data-sidebar-section="true"
          data-sidebar-title={t('grok.provider.title')}
        >
          <Collapse
            style={{ marginBottom: 16 }}
            activeKey={providerListCollapsed ? [] : ['providers']}
            onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
            items={[
              {
                key: 'providers',
                label: (
                  <Space size={8} wrap>
                    <Text strong>
                      <DatabaseOutlined style={{ marginRight: 8 }} />
                      {t('grok.provider.title')}
                    </Text>
                    <GatewayFailoverButton
                      cliKey="grok"
                      status={gatewayCliStatus}
                      primaryProviderNeedsGatewayProxy={primaryGatewayProviderNeedsProxy}
                      onStatusChange={setGatewayCliStatus}
                    />
                  </Space>
                ),
                extra: (
                  <Space size={4}>
                    <Button
                      type="link"
                      size="small"
                      style={{ fontSize: 12 }}
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
                      style={{ fontSize: 12 }}
                      icon={<AppstoreOutlined />}
                      onClick={(e) => {
                        e.stopPropagation();
                        setCommonConfigModalOpen(true);
                      }}
                    >
                      {t('grok.commonConfigButton')}
                    </Button>
                    <Button
                      type="link"
                      size="small"
                      style={{ fontSize: 12 }}
                      icon={<PlusOutlined />}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleAddProvider();
                      }}
                    >
                      {t('grok.addProvider')}
                    </Button>
                  </Space>
                ),
                children: (
                  <Spin spinning={loading}>
                    <div
                      style={{
                        fontSize: 12,
                        color: 'var(--color-text-secondary)',
                        borderLeft: '2px solid var(--color-border)',
                        paddingLeft: 8,
                        marginBottom: 12,
                      }}
                    >
                      <div>{t('grok.pageHint')}</div>
                      <div>{t('grok.pageWarning')}</div>
                    </div>

                    {providers.length === 0 ? (
                      <Empty description={t('grok.emptyText')} style={{ marginTop: 40 }} />
                    ) : (
                      <DndContext
                        sensors={sensors}
                        collisionDetection={closestCenter}
                        onDragEnd={handleDragEnd}
                        modifiers={[restrictToVerticalAxis]}
                      >
                        <SortableContext
                          items={providers.map((p) => p.id)}
                          strategy={verticalListSortingStrategy}
                        >
                          <div>
                            {providers.map((provider) => (
                              <GrokProviderCard
                                key={provider.id}
                                provider={provider}
                                isApplied={provider.id === appliedProviderId}
                                officialAccounts={officialAccountsByProviderId[provider.id] || []}
                                onEdit={handleEditProvider}
                                onDelete={handleDeleteProvider}
                                onCopy={handleCopyProvider}
                                onTest={handleTestProvider}
                                onSelect={handleSelectProvider}
                                onToggleDisabled={handleToggleDisabled}
                                onOfficialAccountLogin={handleStartOfficialAccountOauth}
                                onOfficialLocalAccountSave={handleSaveOfficialLocalAccount}
                                onOfficialAccountApply={handleApplyOfficialAccount}
                                onOfficialAccountDelete={handleDeleteOfficialAccount}
                                onOfficialAccountRefresh={handleRefreshOfficialAccount}
                                onOfficialAccountViewDetails={handleViewOfficialAccountDetails}
                                refreshingOfficialAccountId={refreshingOfficialAccountId}
                                savingOfficialAccountId={savingOfficialAccountId}
                                connectivityStatus={connectivityStatuses[provider.id]}
                                gatewayTakeoverActive={gatewayTakeoverActive}
                                gatewayStatus={gatewayCliStatus}
                                onGatewayStatusChange={async (status) => {
                                  setGatewayCliStatus(status);
                                  await loadConfig();
                                }}
                              />
                            ))}
                          </div>
                        </SortableContext>
                      </DndContext>
                    )}

                    <div style={{ marginTop: 12 }}>
                      <Space wrap>
                        <Button
                          type="dashed"
                          icon={<ImportOutlined />}
                          onClick={() => setImportModalOpen(true)}
                        >
                          {t('opencode.provider.importFavorite')}
                        </Button>
                        <Button
                          type="dashed"
                          icon={<ImportOutlined />}
                          onClick={handleImportFromOpenCode}
                        >
                          {t('grok.importFromOpenCode')}
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
                      </Space>
                    </div>
                  </Spin>
                ),
              },
            ]}
          />
        </div>

        <div
          id="grok-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('grok.prompt.title')}
        >
          <GlobalPromptSettings
            key={`grok-prompt-${promptExpandNonce}`}
            translationKeyPrefix="grok.prompt"
            service={grokPromptApi}
            collapseKey="grok-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <div
          id="grok-plugins"
          data-sidebar-section="true"
          data-sidebar-title={t('grok.plugins.title')}
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
                    {t('grok.plugins.title')}
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
                  <GrokPluginsPanel refreshToken={pluginPanelRefreshToken} />
                ),
              },
            ]}
          />
        </div>

        <div
          id="grok-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title')}
        >
          <SessionManagerPanel
            tool="grok"
            expandNonce={sessionManagerExpandNonce}
            refreshNonce={sessionManagerRefreshNonce}
            sourceMode={sessionSourceMode}
            onSourceModeChange={handleSessionSourceModeChange}
          />
        </div>

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
          gatewayCliKey="grok"
          useGateway={connectivityUsesGateway}
          diagnostics={connectivityInfo ? findDiagnosticsForProvider(favoriteProviders, 'grok', connectivityInfo.providerId) : undefined}
          onSaveDiagnostics={handleSaveConnectivityDiagnostics}
          onCancel={() => setConnectivityModalOpen(false)}
        />

        <ImportProviderModal
          open={importModalOpen}
          onClose={() => setImportModalOpen(false)}
          onImport={handleImportFavoriteProviders}
          existingProviderIds={providers.map((provider) => buildFavoriteProviderStorageKey('grok', provider.id))}
          providerFilter={(provider) => isFavoriteProviderForSource('grok', provider)}
        />

        {/* Modals */}
        {providerModalOpen && (
          <GrokProviderFormModal
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

        <GrokCommonConfigModal
          open={commonConfigModalOpen}
          onCancel={() => setCommonConfigModalOpen(false)}
          onSuccess={() => {
            setCommonConfigModalOpen(false);
          }}
          isLocalProvider={providers.some((provider) => provider.id === GROK_LOCAL_PROVIDER_ID)}
          gatewaySaveLocked={gatewayTakeoverActive}
        />

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(rootPathInfo)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        <GrokDeviceAuthModal
          authSession={deviceAuthSession}
          onClose={() => setDeviceAuthSession(null)}
          onCompleted={async () => {
            setDeviceAuthSession(null);
            await loadConfig();
            await refreshTrayMenu();
          }}
        />

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

        {/* Preview Modal */}
        <GrokConfigPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('grok.preview.currentConfigTitle')}
          data={previewData}
        />

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={(visible) => setSidebarHidden('grok', !visible)}
        />
        <Modal
          open={Boolean(officialAccountDetails)}
          title={t('grok.provider.officialAccountDetailsTitle')}
          onCancel={() => setOfficialAccountDetails(null)}
          footer={null}
          width={720}
        >
          {officialAccountDetails && (
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label={t('grok.provider.name')}>
                {officialAccountDetails.provider.name}
              </Descriptions.Item>
              <Descriptions.Item label={t('grok.provider.officialAccountLabel')}>
                {officialAccountDetails.account.email || officialAccountDetails.account.name}
              </Descriptions.Item>
              <Descriptions.Item label={t('grok.provider.mode')}>
                {officialAccountDetails.account.id === GROK_LOCAL_PROVIDER_ID
                  ? t('grok.provider.officialAccountLocalTag')
                  : t('grok.provider.officialAccountOauthTag')}
              </Descriptions.Item>
              <Descriptions.Item label={t('grok.provider.officialAccountTokenExpiresAt')}>
                {formatUnixTimestamp(officialAccountDetails.account.expiresAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('grok.provider.officialAccountSubject')}>
                {officialAccountDetails.account.subject || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('grok.provider.officialAccountLastRefreshAt')}>
                {formatDateTime(officialAccountDetails.account.lastRefresh)}
              </Descriptions.Item>
              {officialAccountDetails.account.lastError && (
                <Descriptions.Item label={t('grok.provider.officialAccountLastErrorLabel')}>
                  <Text type="danger">{officialAccountDetails.account.lastError}</Text>
                </Descriptions.Item>
              )}
            </Descriptions>
          )}
        </Modal>
      </div>
    </SectionSidebarLayout>
  );
};

export default GrokPage;
