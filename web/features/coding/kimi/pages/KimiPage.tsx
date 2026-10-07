import React from 'react';
import { App, Button, Collapse, Space, Tag, Typography, message } from 'antd';
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import {
  AppstoreOutlined,
  DatabaseOutlined,
  ExclamationCircleOutlined,
  FileTextOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  MessageOutlined,
  SyncOutlined,
  UserOutlined,
} from '@ant-design/icons';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import { useProviderSharing } from '@/features/coding/shared/providerShare';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import CliManualPathSetting from '@/components/common/CliManualPathSetting';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import ProviderListSection from '@/features/coding/shared/ProviderListSection';
import {
  GatewayFailoverButton,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  openAiApiFormatFromBaseUrl,
  providerNeedsGatewayProxy,
  resolveGatewayReengageMode,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import {
  saveProviderWithGatewayReengage,
} from '@/features/coding/shared/gateway/providerSaveReengage';
import {
  engageProxyGatewayFailover,
  engageProxyGatewaySingle,
  getProxyGatewayCliStatus,
  restoreProxyGatewayCliDirect,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchModelsApplyResult } from '@/components/common/FetchModelsModal/types';
import ProviderConnectivityTestModal, {
  buildKimiProviderConnectivityInfo,
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import {
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import { getEnabledCustomProviderBatchCandidates } from '@/features/coding/shared/providerConnectivity/batchTestFilters';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import { useSettingsStore } from '@/stores';
import {
  applyKimiOfficialAccount,
  createKimiProvider,
  deleteKimiOfficialAccount,
  deleteKimiProvider,
  getKimiCommonConfig,
  getKimiConfigFilePath,
  getKimiRootPathInfo,
  listKimiOfficialAccounts,
  listKimiPlugins,
  listKimiProviders,
  readKimiSettings,
  reorderKimiProviders,
  revealKimiConfigFolder,
  saveKimiCommonConfig,
  saveKimiLocalConfig,
  selectKimiProvider,
  startKimiOfficialAccountDeviceAuth,
  toggleKimiProviderDisabled,
  updateKimiProvider,
} from '@/services/kimiApi';
import { kimiPromptApi } from '@/services/kimiPromptApi';
import type {
  KimiCatalogModel,
  KimiCommonConfig,
  KimiCommonConfigInput,
  KimiDeviceAuthStartResult,
  KimiOfficialAccount,
  KimiPlugin,
  KimiProvider,
  KimiProviderInput,
  KimiSettings,
} from '@/types/kimi';
import { KIMI_LOCAL_PROVIDER_ID } from '@/types/kimi';
import type { GatewayCliTakeoverStatus } from '@/services';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import {
  PROVIDER_SORT_MODES,
  filterProviderItems,
  sortProviderItems,
  useProviderBatchSelection,
  useProviderListSort,
} from '@/features/coding/shared/providerList';
import KimiCommonConfigModal from '../components/KimiCommonConfigModal';
import KimiDeviceAuthModal from '../components/KimiDeviceAuthModal';
import KimiModelFormModal from '../components/KimiModelFormModal';
import KimiPluginsPanel from '../components/KimiPluginsPanel';
import KimiProviderCard from '../components/KimiProviderCard';
import KimiProviderFormModal from '../components/KimiProviderFormModal';
import {
  buildKimiSettingsConfig,
  extractKimiBaseUrl,
  KIMI_OFFICIAL_DEFAULT_MODEL_KEY,
  KIMI_OFFICIAL_DEFAULT_MODEL_MAX_CONTEXT_SIZE,
  parseKimiSettingsConfig,
} from '../utils/settingsConfig';
import { canDeleteKimiProvider } from '../utils/providerDeletion';
import {
  buildKimiProviderSavePlan,
  shouldReengageKimiGatewayOnSave,
} from '../utils/providerSaveFlow';
import {
  importModelsIntoKimiCatalog,
  kimiCatalogRowKey,
  removeKimiCatalogModels,
  upsertKimiCatalogModel,
} from '../utils/kimiCatalogModels';
import { saveKimiProviderCatalogWithGatewayReengage } from '../utils/kimiProviderCatalogSave';

const { Link, Text } = Typography;

/**
 * Template used when the official-login flow must create the official provider
 * row first. Must pass backend validation (`validate_provider_settings`).
 */
const KIMI_OFFICIAL_PROVIDER_TEMPLATE: KimiProviderInput = {
  name: 'Kimi Official',
  category: 'official',
  settingsConfig: `{\n  "auth": { "API_KEY": "" },\n  "defaultModelKey": "${KIMI_OFFICIAL_DEFAULT_MODEL_KEY}",\n  "providerConfigs": {}\n}`,
  sortIndex: 0,
};

const KimiPage: React.FC = () => {
  const { t } = useTranslation();
  const { shareProvider, shareModal } = useProviderSharing('kimi', () => loadConfig());
  const { modal } = App.useApp();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState('');
  const [rootPathInfo, setRootPathInfo] = React.useState<Awaited<ReturnType<typeof getKimiRootPathInfo>> | null>(null);
  const [providers, setProviders] = React.useState<KimiProvider[]>([]);
  const [plugins, setPlugins] = React.useState<KimiPlugin[]>([]);
  const [officialAccounts, setOfficialAccounts] = React.useState<KimiOfficialAccount[]>([]);
  const [commonConfig, setCommonConfig] = React.useState<KimiCommonConfig | null>(null);
  const [appliedProviderId, setAppliedProviderId] = React.useState('');
  const [gatewayCliStatus, setGatewayCliStatus] = React.useState<GatewayCliTakeoverStatus | null>(null);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [pluginListCollapsed, setPluginListCollapsed] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<KimiProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  // Model catalog editor (single-model modal, opened from a provider card).
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [modelModalProviderId, setModelModalProviderId] = React.useState<string | null>(null);
  const [modelModalRowKey, setModelModalRowKey] = React.useState('');
  const [modelModalInitialValues, setModelModalInitialValues] = React.useState<KimiCatalogModel | undefined>(undefined);
  // Fetch-models modal (pull the upstream model list into the catalog).
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string | null>(null);
  const [connectivityInfo, setConnectivityInfo] = React.useState<ProviderConnectivityInfo | null>(null);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [deviceAuthSession, setDeviceAuthSession] = React.useState<KimiDeviceAuthStartResult | null>(null);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewData] = React.useState<KimiSettings | null>(null);
  const sidebarHidden = sidebarHiddenByPage.kimi;
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
    if (!primaryProvider || primaryProvider.category === 'official' || primaryProvider.id === KIMI_LOCAL_PROVIDER_ID) {
      return false;
    }
    const baseUrl = extractKimiBaseUrl(primaryProvider.settingsConfig);
    const providerApiFormat = firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(primaryProvider.meta, 'kimi'),
      typeof primaryProvider.meta?.apiFormat === 'string' ? primaryProvider.meta.apiFormat : undefined,
      openAiApiFormatFromBaseUrl(baseUrl),
    );
    return providerNeedsGatewayProxy(providerApiFormat, 'openai_chat');
  }, [gatewayCliStatus?.primary_provider_id, gatewayProviderProfilesVersion, providers]);
  const primaryGatewayProviderNeedsProxyReason = primaryGatewayProviderNeedsProxy ? 'protocol' : null;

  // Alias keys of the applied provider's catalog — the options the swarm pool
  // may reference, since a pool key must resolve to a projected [models.<key>].
  const appliedModelAliasKeys = React.useMemo(() => {
    const appliedProvider = providers.find((provider) => provider.id === appliedProviderId);
    if (!appliedProvider) {
      return [];
    }
    return parseKimiSettingsConfig(appliedProvider.settingsConfig)
      .catalogModels.map((model) => model.key.trim())
      .filter(Boolean);
  }, [appliedProviderId, providers]);

  // Monotonic request id: overlapping loads (manual refresh while a silent
  // reload is in flight) must not let a stale response overwrite newer state.
  const loadConfigRequestIdRef = React.useRef(0);

  const loadConfig = React.useCallback(async (silent = false) => {
    const requestId = ++loadConfigRequestIdRef.current;
    setLoading(true);
    try {
      const [filePath, pathInfo, providerList, pluginList, accountList, nextCommonConfig] = await Promise.all([
        getKimiConfigFilePath(),
        getKimiRootPathInfo(),
        listKimiProviders(),
        listKimiPlugins().catch(() => []),
        listKimiOfficialAccounts().catch(() => []),
        getKimiCommonConfig(),
      ]);
      if (requestId !== loadConfigRequestIdRef.current) return;
      setConfigPath(filePath);
      setRootPathInfo(pathInfo);
      setProviders(providerList);
      setPlugins(pluginList);
      setOfficialAccounts(accountList);
      setCommonConfig(nextCommonConfig);
      setAppliedProviderId(providerList.find((provider) => provider.isApplied)?.id ?? '');
      // Drop statuses of providers that no longer exist so a delete/reload
      // cannot leave stale badges behind.
      setConnectivityStatuses((previous) => {
        const liveIds = new Set(providerList.map((provider) => provider.id));
        const pruned = Object.fromEntries(
          Object.entries(previous).filter(([id]) => liveIds.has(id)),
        );
        return Object.keys(pruned).length === Object.keys(previous).length ? previous : pruned;
      });
      // Takeover eligibility depends on the provider rows (proxyable
      // candidates), so re-read it with the list; otherwise a stale error
      // status keeps the gateway proxy button hidden after the underlying
      // provider issue is already fixed.
      void getProxyGatewayCliStatus('kimi')
        .then((status) => {
          if (requestId !== loadConfigRequestIdRef.current) return;
          setGatewayCliStatus(status);
        })
        .catch(() => {});
    } catch (error) {
      if (requestId !== loadConfigRequestIdRef.current) return;
      if (!silent) {
        const errorMessage = error instanceof Error ? error.message : String(error);
        message.error(errorMessage || t('common.error'));
      }
    } finally {
      if (requestId === loadConfigRequestIdRef.current) {
        setLoading(false);
      }
    }
  }, [t]);

  React.useEffect(() => {
    void loadConfig();
  }, [loadConfig]);

  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig({
    t,
    translationKeyPrefix: 'kimi',
    defaultConfig: '',
    rootDirectoryChangeLocked: gatewayTakeoverActive,
    rootDirectoryChangeLockedText: t('gateway.proxy.switchPrimaryHint'),
    loadConfig,
    getCommonConfig: getKimiCommonConfig,
    saveCommonConfig: saveKimiCommonConfig,
  });

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    { id: 'kimi-providers', title: t('kimi.provider.title'), order: 1 },
    { id: 'kimi-global-prompt', title: t('common.prompt.title'), order: 2 },
    { id: 'kimi-plugins', title: t('kimi.plugins.title'), order: 3 },
    { id: 'kimi-session-manager', title: t('kimi.sessions'), order: 4 },
  ], [t]);

  const handleAddProvider = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: KimiProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: KimiProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name}_copy`,
      isApplied: false,
    });
    setIsCopyMode(true);
    setProviderModalOpen(true);
  };

  const modelModalProvider = React.useMemo(
    () => providers.find((provider) => provider.id === modelModalProviderId) ?? null,
    [modelModalProviderId, providers],
  );

  const handleEditModel = (provider: KimiProvider, model?: KimiCatalogModel) => {
    setModelModalProviderId(provider.id);
    setModelModalRowKey(model ? kimiCatalogRowKey(model) : '');
    setModelModalInitialValues(model);
    setModelModalOpen(true);
  };

  /**
   * Persist a catalog edit back onto the provider row. Catalog edits rebuild
   * `settingsConfig`, so an applied provider needs the gateway takeover
   * replayed around the write (the backend rejects in-takeover direct writes).
   */
  const persistProviderCatalog = React.useCallback(async (
    provider: KimiProvider,
    models: KimiCatalogModel[],
  ) => {
    const settings = parseKimiSettingsConfig(provider.settingsConfig);
    // Keep `defaultModelKey` resolvable: the CLI refuses to resolve an unknown
    // key, and the backend rejects a default with no catalog. Removing or
    // renaming the default row must therefore re-point it (or clear it).
    const modelKeys = models.map((model) => model.key.trim()).filter(Boolean);
    const defaultModelKey = modelKeys.includes(settings.defaultModelKey)
      ? settings.defaultModelKey
      : modelKeys[0] ?? '';
    const settingsConfig = buildKimiSettingsConfig({
      category: provider.category,
      apiKey: settings.apiKey,
      baseUrl: settings.baseUrl,
      providerKey: settings.providerKey,
      defaultModelKey,
      catalogModels: models,
      customTomlConfig: settings.customTomlConfig,
      rawObject: settings.rawObject,
    });
    const gatewayModeBeforeSave = resolveGatewayReengageMode(gatewayCliStatus);
    await saveKimiProviderCatalogWithGatewayReengage({
      provider,
      settingsConfig,
      gatewayMode: gatewayModeBeforeSave,
      updateProvider: updateKimiProvider,
      restoreDirect: () => restoreProxyGatewayCliDirect('kimi'),
      engageSingle: () => engageProxyGatewaySingle('kimi', provider.id),
      engageFailover: () => engageProxyGatewayFailover('kimi'),
      onGatewayStatusChange: setGatewayCliStatus,
    });
    await loadConfig(true);
    await refreshTrayMenu();
  }, [gatewayCliStatus, loadConfig]);

  const handleModelFormSubmit = React.useCallback(async (model: KimiCatalogModel) => {
    if (!modelModalProvider) {
      return;
    }
    const currentModels = parseKimiSettingsConfig(modelModalProvider.settingsConfig).catalogModels;
    const nextModels = upsertKimiCatalogModel(currentModels, model, modelModalRowKey || undefined);
    try {
      await persistProviderCatalog(modelModalProvider, nextModels);
      message.success(t('kimi.saveSuccess'));
      setModelModalOpen(false);
      setModelModalProviderId(null);
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [modelModalProvider, modelModalRowKey, persistProviderCatalog, t]);

  /**
   * Delete one catalog row.
   *
   * No confirmation here: the shared model list wraps the row's delete button
   * in its own `Popconfirm`, so asking again in a modal would double-prompt.
   */
  const handleDeleteModel = React.useCallback(async (
    provider: KimiProvider,
    model: KimiCatalogModel,
  ) => {
    try {
      const currentModels = parseKimiSettingsConfig(provider.settingsConfig).catalogModels;
      const nextModels = removeKimiCatalogModels(currentModels, [kimiCatalogRowKey(model)]);
      await persistProviderCatalog(provider, nextModels);
      message.success(t('kimi.deleteSuccess'));
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [persistProviderCatalog, t]);

  /**
   * Duplicate a catalog row under a free key.
   *
   * The key is the `[models."<key>"]` table name and must be unique, so the copy
   * walks a `-copy` / `-copy-2` … suffix until one is free rather than asking
   * the user to invent one (they can rename it afterwards in the editor).
   */
  const handleCopyModel = React.useCallback(async (
    provider: KimiProvider,
    model: KimiCatalogModel,
  ) => {
    const currentModels = parseKimiSettingsConfig(provider.settingsConfig).catalogModels;
    const existingKeys = new Set(currentModels.map((item) => item.key.trim()));
    const baseKey = `${model.key.trim()}-copy`;
    let nextKey = baseKey;
    let suffix = 2;
    while (existingKeys.has(nextKey)) {
      nextKey = `${baseKey}-${suffix}`;
      suffix += 1;
    }
    try {
      await persistProviderCatalog(
        provider,
        upsertKimiCatalogModel(currentModels, { ...model, key: nextKey }),
      );
      message.success(t('kimi.saveSuccess'));
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [persistProviderCatalog, t]);

  /**
   * Make one catalog row the provider's default model.
   *
   * The default lives in `settingsConfig.defaultModelKey` (the CLI resolves the
   * alias through the catalog), so this is a settings write, not a catalog
   * edit — but it still rebuilds `settingsConfig`, so it goes through the same
   * gateway-aware save path.
   */
  const handleSetPrimaryModel = React.useCallback(async (
    provider: KimiProvider,
    model: KimiCatalogModel,
  ) => {
    const settings = parseKimiSettingsConfig(provider.settingsConfig);
    const settingsConfig = buildKimiSettingsConfig({
      category: provider.category,
      apiKey: settings.apiKey,
      baseUrl: settings.baseUrl,
      providerKey: settings.providerKey,
      defaultModelKey: model.key.trim(),
      catalogModels: settings.catalogModels,
      customTomlConfig: settings.customTomlConfig,
      rawObject: settings.rawObject,
    });
    try {
      await saveKimiProviderCatalogWithGatewayReengage({
        provider,
        settingsConfig,
        gatewayMode: resolveGatewayReengageMode(gatewayCliStatus),
        updateProvider: updateKimiProvider,
        restoreDirect: () => restoreProxyGatewayCliDirect('kimi'),
        engageSingle: () => engageProxyGatewaySingle('kimi', provider.id),
        engageFailover: () => engageProxyGatewayFailover('kimi'),
        onGatewayStatusChange: setGatewayCliStatus,
      });
      await loadConfig(true);
      await refreshTrayMenu();
      message.success(t('kimi.saveSuccess'));
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [gatewayCliStatus, loadConfig, t]);

  /**
   * Open the upstream model-list modal. Official channels authenticate through
   * OAuth and `__local__` is a read-only bridge, so neither can be probed with
   * a static key.
   */
  const handleOpenFetchModels = React.useCallback((provider: KimiProvider) => {
    if (provider.category === 'official' || provider.id === KIMI_LOCAL_PROVIDER_ID) {
      return;
    }
    setFetchModelsProviderId(provider.id);
    setFetchModelsModalOpen(true);
  }, []);

  const fetchModelsProvider = React.useMemo(
    () => providers.find((provider) => provider.id === fetchModelsProviderId) ?? null,
    [fetchModelsProviderId, providers],
  );

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!fetchModelsProvider) {
      return null;
    }
    const settings = parseKimiSettingsConfig(fetchModelsProvider.settingsConfig);
    return {
      providerId: fetchModelsProvider.id,
      name: fetchModelsProvider.name,
      baseUrl: settings.baseUrl,
      apiKey: settings.apiKey,
      providerKey: settings.providerKey,
      existingModelIds: settings.catalogModels.map((item) => item.model.trim()),
    };
  }, [fetchModelsProvider]);

  const handleFetchModelsApply = React.useCallback(async (result: FetchModelsApplyResult) => {
    if (!fetchModelsProvider || !fetchModelsProviderInfo) {
      return;
    }
    try {
      const settings = parseKimiSettingsConfig(fetchModelsProvider.settingsConfig);
      const models = importModelsIntoKimiCatalog(
        settings.catalogModels,
        result.selectedModels,
        result.removedModelIds,
        result.orderedModelIds,
        fetchModelsProviderInfo.providerKey,
        KIMI_OFFICIAL_DEFAULT_MODEL_MAX_CONTEXT_SIZE,
      );
      await persistProviderCatalog(fetchModelsProvider, models);
      message.success(t('common.success'));
      setFetchModelsModalOpen(false);
      setFetchModelsProviderId(null);
    } catch (error) {
      console.error('Failed to apply fetched Kimi models:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [fetchModelsProvider, fetchModelsProviderInfo, persistProviderCatalog, t]);

  const handleDeleteModels = React.useCallback((provider: KimiProvider, models: KimiCatalogModel[]) => {
    modal.confirm({
      title: t('common.model.batchDeleteConfirmTitle'),
      content: t('common.model.batchDeleteConfirmContent', { count: models.length }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          // Re-read the current catalog: the confirm is async, so a concurrent
          // fetch/import may have changed it. Report what was actually removed
          // rather than the count captured when the dialog opened.
          const currentModels = parseKimiSettingsConfig(provider.settingsConfig).catalogModels;
          const rowKeys = models.map((model) => kimiCatalogRowKey(model));
          const nextModels = removeKimiCatalogModels(currentModels, rowKeys);
          const removedCount = currentModels.length - nextModels.length;
          await persistProviderCatalog(provider, nextModels);
          message.success(t('kimi.model.batchDeleteSuccess', { count: removedCount }));
        } catch (error) {
          message.error(error instanceof Error ? error.message : String(error));
        }
      },
    });
  }, [persistProviderCatalog, t, modal]);

  const handleSaveProvider = async (values: KimiProviderInput) => {
    const plan = buildKimiProviderSavePlan(editingProvider, values, { isCopy: isCopyMode });
    const gatewayModeBeforeSave = resolveGatewayReengageMode(gatewayCliStatus);
    const shouldReengageGatewayProxy = shouldReengageKimiGatewayOnSave(editingProvider, gatewayModeBeforeSave);

    let savedProviderId = editingProvider?.id ?? '';

    await saveProviderWithGatewayReengage({
      gatewayMode: shouldReengageGatewayProxy ? gatewayModeBeforeSave : null,
      restoreDirect: () => restoreProxyGatewayCliDirect('kimi'),
      engageSingle: () => engageProxyGatewaySingle('kimi', savedProviderId),
      engageFailover: () => engageProxyGatewayFailover('kimi'),
      // No `engageAggregate`: aggregate mode is Codex-only, so this CLI can
      // never report it and the re-engage helper would reject the replay.
      onGatewayStatusChange: setGatewayCliStatus,
      saveProvider: async () => {
        switch (plan.action) {
          case 'adopt_local': {
            // The `__local__` provider is a temp projection of the on-disk
            // config; saving it adopts the live config into a real applied
            // DB record instead of duplicating it. Backfill the fresh record
            // id before gateway re-engage — engaging with `__local__` would
            // resolve no provider and silently drop back to direct.
            savedProviderId = await saveKimiLocalConfig({ provider: plan.input });
            break;
          }
          case 'update':
            await updateKimiProvider(plan.provider);
            savedProviderId = plan.provider.id;
            break;
          case 'create': {
            const created = await createKimiProvider(plan.input);
            savedProviderId = created.id;
            break;
          }
        }
      },
    });
    message.success(t('kimi.saveSuccess'));
    setProviderModalOpen(false);
    setEditingProvider(null);
    setIsCopyMode(false);
    await loadConfig(true);
    await refreshTrayMenu();
  };

  const handleDeleteProvider = (provider: KimiProvider) => {
    modal.confirm({
      title: t('kimi.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteKimiProvider(provider.id);
          message.success(t('kimi.deleteSuccess'));
          await loadConfig(true);
          await refreshTrayMenu();
        } catch (error) {
          message.error(error instanceof Error ? error.message : String(error));
        }
      },
    });
  };

  const handleToggleDisabled = async (provider: KimiProvider, isDisabled: boolean) => {
    try {
      await toggleKimiProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('common.disabled') : t('common.enabled'));
      await loadConfig(true);
      await refreshTrayMenu();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('kimi');
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

  const handleBatchDeleteProviders = React.useCallback(
    async (ids: string[]): Promise<boolean> => {
      const providersToDelete = providers.filter(
        (provider) => ids.includes(provider.id) && canDeleteKimiProvider(provider, officialAccounts),
      );
      if (providersToDelete.length === 0) return false;
      try {
        for (const provider of providersToDelete) {
          await deleteKimiProvider(provider.id);
        }
        await loadConfig(true);
        await refreshTrayMenu();
        message.success(t('kimi.deleteSuccess'));
        return true;
      } catch (error) {
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig(true);
        await refreshTrayMenu();
        return false;
      }
    },
    [providers, officialAccounts, loadConfig, t],
  );

  // Selectable ids exclude the local provider (no delete path / not a managed preset).
  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter((provider) => canDeleteKimiProvider(provider, officialAccounts)).map((provider) => provider.id),
    [visibleProviders, officialAccounts],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  const handleApplyProvider = async (provider: KimiProvider) => {
    try {
      await selectKimiProvider(provider.id);
      message.success(t('kimi.applySuccess'));
      noteProviderUsed(provider.id);
      await loadConfig(true);
      await refreshTrayMenu();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleApplyOfficialAccount = async (account: KimiOfficialAccount) => {
    try {
      await applyKimiOfficialAccount(account.id);
      message.success(t('kimi.account.applySuccess'));
      await loadConfig(true);
      await refreshTrayMenu();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleDeleteOfficialAccount = (account: KimiOfficialAccount) => {
    modal.confirm({
      title: t('kimi.account.deleteConfirm', {
        name: account.email || account.name || account.id,
      }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteKimiOfficialAccount(account.id);
          message.success(t('kimi.account.deleteSuccess'));
          await loadConfig(true);
          await refreshTrayMenu();
        } catch (error) {
          message.error(error instanceof Error ? error.message : String(error));
        }
      },
    });
  };

  const handleTestProvider = (provider: KimiProvider) => {
    if (provider.category === 'official') {
      message.info(t('kimi.provider.officialConnectivityHint'));
      return;
    }
    setConnectivityInfo(buildKimiProviderConnectivityInfo(provider));
    setConnectivityModalOpen(true);
  };

  const handleBatchTestProviders = React.useCallback(async () => {
    if (providers.length === 0) {
      return;
    }

    const officialProviders = providers.filter((provider) => provider.category === 'official');
    const testableProviders = getEnabledCustomProviderBatchCandidates(providers);

    if (officialProviders.length > 0) {
      message.info(t('kimi.provider.officialBatchSkipped', { count: officialProviders.length }));
    }

    if (testableProviders.length === 0) {
      setConnectivityStatuses({});
      return;
    }

    const targets = testableProviders.map((provider) => (
      buildProviderConnectivityBatchTarget(buildKimiProviderConnectivityInfo(provider), {
        requireBaseUrl: false,
        requireApiKey: true,
        errorMessages: {
          missingBaseUrl: t('common.baseUrlMissing'),
          missingApiKey: t('common.apiKeyMissing'),
          missingModel: t('common.modelMissing'),
        },
      })
    ));

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
      console.error('Failed to batch test Kimi providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [providers, t]);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const oldIndex = providers.findIndex((provider) => provider.id === active.id);
    const newIndex = providers.findIndex((provider) => provider.id === over.id);
    const previousProviders = [...providers];
    const nextProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(nextProviders);
    try {
      await reorderKimiProviders(nextProviders.map((provider) => provider.id));
    } catch (error) {
      setProviders(previousProviders);
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleSaveCommonConfig = async (input: KimiCommonConfigInput) => {
    await saveKimiCommonConfig(input);
    message.success(t('kimi.saveSuccess'));
    setCommonConfigModalOpen(false);
    await loadConfig(true);
  };

  const handlePreviewCurrentConfig = async () => {
    try {
      const settings = await readKimiSettings();
      setPreviewData(settings);
      setPreviewModalOpen(true);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    }
  };

  const handleStartOfficialAccountAuth = async () => {
    // The `__local__` projection is not a real DB row — the backend cannot
    // resolve it as an official provider, so only look at persisted rows.
    let officialProvider = providers.find(
      (p) => p.category === 'official' && p.id !== KIMI_LOCAL_PROVIDER_ID,
    );
    if (!officialProvider) {
      try {
        const newProvider = await createKimiProvider({ ...KIMI_OFFICIAL_PROVIDER_TEMPLATE });
        officialProvider = newProvider;
        setProviders((prev) => [...prev, newProvider]);
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : String(error);
        message.error(errorMessage || t('common.error'));
        return;
      }
    }

    try {
      const session = await startKimiOfficialAccountDeviceAuth(officialProvider.id);
      setDeviceAuthSession(session);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    }
  };

  const handleOpenConfigFolder = async () => {
    try {
      await revealKimiConfigFolder();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  /** The header's refresh button; `loadConfig` also drives the section spinners. */
  const handleRefreshPage = () => {
    void loadConfig();
  };

  const handleOpenPluginsDirectory = async () => {
    const rootPath = rootPathInfo?.path;
    if (!rootPath) return;
    try {
      await invoke('open_folder', { path: `${rootPath.replace(/[\\/]+$/, '')}/plugins` });
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(errorMessage || t('common.error'));
    }
  };

  return (
    <SectionSidebarLayout
      sidebarTitle={t('kimi.title')}
      sections={sidebarSections}
      sidebarHidden={sidebarHidden}
      getIcon={(id) => {
        switch (id) {
          case 'kimi-providers':
            return <DatabaseOutlined />;
          case 'kimi-global-prompt':
            return <FileTextOutlined />;
          case 'kimi-plugins':
            return <AppstoreOutlined />;
          case 'kimi-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'kimi-providers':
            setProviderListCollapsed(false);
            break;
          case 'kimi-global-prompt':
            setPromptExpandNonce((value) => value + 1);
            break;
          case 'kimi-plugins':
            setPluginListCollapsed(false);
            break;
          case 'kimi-session-manager':
            setSessionManagerExpandNonce((value) => value + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        <CodingPageHeader
          title={t('kimi.title')}
          docsUrl="https://www.npmjs.com/package/@moonshot-ai/kimi-code"
          onPreviewConfig={handlePreviewCurrentConfig}
          configPath={configPath || '~/.kimi-code/config.toml'}
          onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
          onOpenFolder={handleOpenConfigFolder}
          onRefresh={handleRefreshPage}
          onMoreOptions={() => setSettingsModalOpen(true)}
        />
      </div>

      <ProviderListSection
        sectionId="kimi-providers"
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
        onBatchTest={() => void handleBatchTestProviders()}
        batchTesting={batchTestingProviders}
        onOpenCommonConfig={() => setCommonConfigModalOpen(true)}
        onAddProvider={handleAddProvider}
        headerExtra={
          <GatewayFailoverButton
            cliKey="kimi"
            status={gatewayCliStatus}
            primaryProviderNeedsGatewayProxy={primaryGatewayProviderNeedsProxy}
            primaryProviderNeedsProxyReason={primaryGatewayProviderNeedsProxyReason}
            onStatusChange={setGatewayCliStatus}
          />
        }
        toolbarExtra={
          <Button
            type="link"
            size="small"
            style={{ fontSize: 12 }}
            icon={<UserOutlined />}
            onClick={(event) => {
              event.stopPropagation();
              void handleStartOfficialAccountAuth();
            }}
          >
            {t('kimi.officialAccountButton')}
          </Button>
        }
        hint={<div>{t('kimi.pageHint')}</div>}
        footer={
          officialAccounts.length > 0 ? (
            <div style={{ marginTop: 16, display: 'flex', flexDirection: 'column', gap: 6 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <Text strong style={{ fontSize: 12 }}>{t('kimi.officialAccounts')}</Text>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(event) => {
                    event.stopPropagation();
                    void openUrl('https://www.kimi.com/code/console');
                  }}
                >
                  <LinkOutlined /> {t('kimi.viewUsage')}
                </Link>
              </div>
              {officialAccounts.map((account) => (
                <div
                  key={account.id}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    fontSize: 12,
                    color: 'var(--color-text-secondary)',
                    flexWrap: 'wrap',
                  }}
                >
                  <UserOutlined />
                  <span>{account.email || account.name || account.id}</span>
                  {account.isApplied ? (
                    <Tag color="success" style={{ marginInlineEnd: 0 }}>
                      {t('kimi.account.applied')}
                    </Tag>
                  ) : (
                    <Button
                      type="link"
                      size="small"
                      style={{ fontSize: 12, padding: 0, height: 'auto' }}
                      onClick={() => void handleApplyOfficialAccount(account)}
                    >
                      {t('kimi.account.apply')}
                    </Button>
                  )}
                  <Button
                    type="link"
                    size="small"
                    danger
                    style={{ fontSize: 12, padding: 0, height: 'auto' }}
                    disabled={account.isApplied}
                    onClick={() => handleDeleteOfficialAccount(account)}
                  >
                    {t('kimi.account.delete')}
                  </Button>
                </div>
              ))}
            </div>
          ) : null
        }
      >
        <DndContext
          sensors={providerBatchDragDisabled ? [] : sensors}
          collisionDetection={closestCenter}
          onDragEnd={(event) => void handleDragEnd(event)}
          modifiers={[restrictToVerticalAxis]}
        >
          <SortableContext
            items={providers.map((provider) => provider.id)}
            strategy={verticalListSortingStrategy}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              {visibleProviders.map((provider) => (
                <KimiProviderCard
                  key={provider.id}
                  provider={provider}
                  isApplied={provider.id === appliedProviderId}
                  gatewayTakeoverActive={gatewayTakeoverActive}
                  gatewayStatus={gatewayCliStatus}
                  onGatewayStatusChange={setGatewayCliStatus}
                  onEdit={handleEditProvider}
                  onDelete={(value) => void handleDeleteProvider(value)}
                  onApply={(value) => void handleApplyProvider(value)}
                  onToggleDisabled={handleToggleDisabled}
                  onTest={handleTestProvider}
                  onCopy={handleCopyProvider}
                  onShare={shareProvider}
                  connectivityStatus={connectivityStatuses[provider.id]}
                  selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.id)}
                  selected={providerBatch.selectedIds.has(provider.id)}
                  onSelectChange={(checked) =>
                    providerBatch.toggleSelect(provider.id, checked)
                  }
                  onEditModel={handleEditModel}
                  onCopyModel={handleCopyModel}
                  onSetPrimaryModel={handleSetPrimaryModel}
                  onDeleteModel={handleDeleteModel}
                  onDeleteModels={handleDeleteModels}
                  onFetchModels={handleOpenFetchModels}
                  canFetchModels={
                    provider.category !== 'official'
                    && provider.id !== KIMI_LOCAL_PROVIDER_ID
                    && Boolean(extractKimiBaseUrl(provider.settingsConfig))
                  }
                  dragDisabled={providerDragDisabled}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      </ProviderListSection>

      <div id="kimi-global-prompt" data-sidebar-section="true" data-sidebar-title={t('common.prompt.title')}>
        <GlobalPromptSettings
          key={`kimi-prompt-${promptExpandNonce}`}
          service={kimiPromptApi}
          toolName="Kimi"
          promptFileName="AGENTS.md"
          collapseKey="kimi_prompt"
          defaultExpanded
        />
      </div>

      <div id="kimi-plugins" data-sidebar-section="true" data-sidebar-title={t('kimi.plugins.title')}>
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
                  {t('kimi.plugins.title')}
                </Text>
              ),
              extra: (
                <Space size={4}>
                  <Button
                    type="link"
                    size="small"
                    style={{ fontSize: 12 }}
                    icon={<FolderOpenOutlined />}
                    disabled={!rootPathInfo?.path}
                    onClick={(event) => {
                      event.stopPropagation();
                      void handleOpenPluginsDirectory();
                    }}
                  >
                    {t('kimi.plugins.openDirectory')}
                  </Button>
                  <Button
                    type="link"
                    size="small"
                    style={{ fontSize: 12 }}
                    icon={<SyncOutlined />}
                    onClick={(event) => {
                      event.stopPropagation();
                      void loadConfig(true);
                    }}
                  >
                    {t('common.refresh')}
                  </Button>
                </Space>
              ),
              children: (
                <KimiPluginsPanel plugins={plugins} loading={loading} />
              ),
            },
          ]}
        />
      </div>

      <div id="kimi-session-manager" data-sidebar-section="true" data-sidebar-title={t('kimi.sessions')}>
        <SessionManagerPanel tool="kimi" expandNonce={sessionManagerExpandNonce} />
      </div>

      <KimiProviderFormModal
        open={providerModalOpen}
        provider={editingProvider}
        onCancel={() => {
          setProviderModalOpen(false);
          setEditingProvider(null);
          setIsCopyMode(false);
        }}
        onSubmit={handleSaveProvider}
      />

      {modelModalProvider && (
        <KimiModelFormModal
          open={modelModalOpen}
          isEdit={Boolean(modelModalRowKey)}
          initialValues={modelModalInitialValues}
          providerKey={parseKimiSettingsConfig(modelModalProvider.settingsConfig).providerKey}
          existingKeys={parseKimiSettingsConfig(modelModalProvider.settingsConfig)
            .catalogModels.map((model) => model.key)
            // The edited row's own key is not a collision.
            .filter((key) => key !== modelModalRowKey)}
          onCancel={() => {
            setModelModalOpen(false);
            setModelModalProviderId(null);
            setModelModalRowKey('');
            setModelModalInitialValues(undefined);
          }}
          onSubmit={handleModelFormSubmit}
        />
      )}

      {fetchModelsProviderInfo && (
        <FetchModelsModal
          open={fetchModelsModalOpen}
          providerId={fetchModelsProviderInfo.providerId}
          providerName={fetchModelsProviderInfo.name}
          baseUrl={fetchModelsProviderInfo.baseUrl}
          apiKey={fetchModelsProviderInfo.apiKey || undefined}
          // Kimi custom providers are OpenAI-compatible (`type = "openai"`),
          // so the models list always comes from `{base_url}/models`.
          sdkType="@ai-sdk/openai"
          existingModelIds={fetchModelsProviderInfo.existingModelIds}
          onCancel={() => {
            setFetchModelsModalOpen(false);
            setFetchModelsProviderId(null);
          }}
          onSuccess={handleFetchModelsApply}
        />
      )}

      <ProviderConnectivityTestModal
        open={connectivityModalOpen}
        connectivityInfo={connectivityInfo}
        gatewayCliKey="kimi"
        onCancel={() => setConnectivityModalOpen(false)}
      />

      <KimiCommonConfigModal
        open={commonConfigModalOpen}
        config={commonConfig}
        modelAliasKeys={appliedModelAliasKeys}
        onCancel={() => setCommonConfigModalOpen(false)}
        onSubmit={handleSaveCommonConfig}
      />

      <KimiDeviceAuthModal
        authSession={deviceAuthSession}
        onClose={() => setDeviceAuthSession(null)}
        onCompleted={async () => {
          setDeviceAuthSession(null);
          await loadConfig(true);
        }}
      />

      <RootDirectoryModal
        open={rootDirectoryModalOpen}
        {...getRootDirectoryModalProps(rootPathInfo)}
        onCancel={() => setRootDirectoryModalOpen(false)}
        onSubmit={handleSaveRootDirectory}
        onReset={handleResetRootDirectory}
      />

      <SidebarSettingsModal
        open={settingsModalOpen}
        onClose={() => setSettingsModalOpen(false)}
        sidebarVisible={!sidebarHidden}
        onSidebarVisibleChange={(visible) => setSidebarHidden('kimi', !visible)}
      >
        <CliManualPathSetting commandName="kimi" labelKey="subModules.kimi" />
      </SidebarSettingsModal>

      <JsonPreviewModal
        open={previewModalOpen}
        title={t('common.previewConfig')}
        data={previewData}
        onClose={() => setPreviewModalOpen(false)}
      />

      {shareModal}
    </SectionSidebarLayout>
  );
};

export default KimiPage;
