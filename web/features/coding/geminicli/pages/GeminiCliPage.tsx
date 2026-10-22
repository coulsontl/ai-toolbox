import React from 'react';
import { Button, Descriptions, Modal, Typography, message } from 'antd';
import {
  CopyOutlined,
  DatabaseOutlined,
  ExclamationCircleOutlined,
  FileTextOutlined,
  ImportOutlined,
  MessageOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  SortableContext,
  arrayMove,
  rectSortingStrategy,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis, restrictToWindowEdges } from '@dnd-kit/modifiers';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import {
  firstGatewayApiFormat,
  GatewayFailoverButton,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  providerNeedsGatewayProxy,
  resolveGatewayReengageMode,
  saveProviderWithGatewayReengage,
  subscribeGatewayProviderProfiles,
  toGatewayAggregateReengageConfig,
} from '@/features/coding/shared/gateway';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import {
  PROVIDER_SORT_MODES,
  filterProviderItems,
  sortProviderItems,
  providerSelectionDisabledReason,
  useProviderBatchSelection,
  useProviderListSort,
  useProviderListViewPrefs,
  type ProviderSelectionBlockReason,
} from '@/features/coding/shared/providerList';
import { ProviderCardGrid } from '@/features/coding/shared/providerCardView';
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import ProviderListSection from '@/features/coding/shared/ProviderListSection';
import { TRAY_CONFIG_REFRESH_EVENT, DEEP_LINK_IMPORT_COMPLETED } from '@/constants/configEvents';
import { useSettingsStore } from '@/stores';
import { refreshTrayMenu } from '@/services/appApi';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import ShareProviderModal from '@/features/coding/shared/providerShare';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import {
  createGeminiCliProvider,
  deleteGeminiCliProvider,
  applyGeminiCliOfficialAccount,
  copyGeminiCliOfficialAccountToken,
  deleteGeminiCliOfficialAccount,
  getGeminiCliCommonConfig,
  getGeminiCliConfigPath,
  getGeminiCliRootPathInfo,
  listGeminiCliOfficialAccounts,
  listGeminiCliProviders,
  readGeminiCliSettings,
  refreshGeminiCliOfficialAccountLimits,
  reorderGeminiCliProviders,
  saveGeminiCliCommonConfig,
  saveGeminiCliOfficialLocalAccount,
  saveGeminiCliLocalConfig,
  selectGeminiCliProvider,
  startGeminiCliOfficialAccountOauth,
  toggleGeminiCliProviderDisabled,
  updateGeminiCliProvider,
} from '@/services/geminiCliApi';
import { geminiCliPromptApi } from '@/services/geminiCliPromptApi';
import type {
  ConfigPathInfo,
  GeminiCliOfficialAccount,
  GeminiCliProvider,
  GeminiCliProviderFormValues,
  GeminiCliProviderInput,
  GeminiCliSettings,
} from '@/types/geminicli';
import {
  engageProxyGatewayFailover,
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  type GatewayCliTakeoverStatus,
} from '@/services';
import GeminiCliProviderCard from '../components/GeminiCliProviderCard';
import GeminiCliProviderFormModal from '../components/GeminiCliProviderFormModal';
import GeminiCliCommonConfigModal from '../components/GeminiCliCommonConfigModal';
import {
  GEMINI_CLI_LOCAL_PROVIDER_ID,
  shouldLoadGeminiCliOfficialAccounts,
} from '../utils/localProvider';

const { Text } = Typography;
const ACCOUNT_DETAILS_EMPTY_VALUE = '-';

function maskTokenPreview(tokenKind: 'access' | 'refresh', account: GeminiCliOfficialAccount): string {
  const preview = tokenKind === 'access'
    ? account.accessTokenPreview
    : account.refreshTokenPreview;
  if (preview?.trim()) {
    return preview.trim();
  }
  return ACCOUNT_DETAILS_EMPTY_VALUE;
}

function renderTokenPreview(
  previewValue: string,
  onCopy: () => Promise<void>,
): React.ReactNode {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8, maxWidth: '100%' }}>
      <div
        style={{
          flex: 1,
          minWidth: 0,
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          fontFamily: 'ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, monospace',
          fontSize: 12,
        }}
      >
        {previewValue}
      </div>
      <Button
        type="text"
        size="small"
        icon={<CopyOutlined />}
        onClick={() => {
          void onCopy();
        }}
        style={{ height: 'auto', paddingInline: 4, flexShrink: 0 }}
      />
    </div>
  );
}

const GeminiCliPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [providers, setProviders] = React.useState<GeminiCliProvider[]>([]);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [officialAccountsByProviderId, setOfficialAccountsByProviderId] = React.useState<
    Record<string, GeminiCliOfficialAccount[]>
  >({});
  const [appliedProviderId, setAppliedProviderId] = React.useState('');
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
    if (!primaryProvider || primaryProvider.category === 'official' || primaryProvider.id === GEMINI_CLI_LOCAL_PROVIDER_ID) {
      return false;
    }
    const providerApiFormat = firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(primaryProvider.meta, 'gemini'),
      primaryProvider.meta?.apiFormat,
    );
    return providerNeedsGatewayProxy(providerApiFormat, 'gemini_native');
  }, [gatewayCliStatus?.primary_provider_id, gatewayProviderProfilesVersion, providers]);
  const primaryGatewayProviderNeedsProxyReason = primaryGatewayProviderNeedsProxy ? 'protocol' : null;
  const [refreshingOfficialAccountId, setRefreshingOfficialAccountId] = React.useState<string | null>(null);
  const [savingOfficialAccountId, setSavingOfficialAccountId] = React.useState<string | null>(null);
  const [officialAccountDetails, setOfficialAccountDetails] = React.useState<{
    provider: GeminiCliProvider;
    account: GeminiCliOfficialAccount;
  } | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewData] = React.useState<GeminiCliSettings | null>(null);
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<GeminiCliProvider | null>(null);
  const [shareProvider, setShareProvider] = React.useState<GeminiCliProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const sidebarHidden = sidebarHiddenByPage.geminicli;

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(() => [
    { id: 'geminicli-providers', title: t('geminicli.provider.title'), order: 1 },
    { id: 'geminicli-global-prompt', title: t('common.prompt.title'), order: 2 },
    { id: 'geminicli-session-manager', title: t('sessionManager.title'), order: 3 },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [path, nextRootPathInfo, providerList] = await Promise.all([
        getGeminiCliConfigPath(),
        getGeminiCliRootPathInfo(),
        listGeminiCliProviders(),
      ]);
      setConfigPath(path);
      setRootPathInfo(nextRootPathInfo);
      setProviders(providerList);
      const officialAccountEntries = await Promise.all(
        providerList.map(async (provider) => [
          provider.id,
          shouldLoadGeminiCliOfficialAccounts(provider)
            ? await listGeminiCliOfficialAccounts(provider.id)
            : [],
        ] as const),
      );
      setOfficialAccountsByProviderId(Object.fromEntries(officialAccountEntries));
      setAppliedProviderId(providerList.find((provider) => provider.isApplied)?.id || '');
    } catch (error) {
      console.error('Failed to load Gemini CLI config:', error);
      if (!silent) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    void loadConfig();
  }, [loadConfig]);

  React.useEffect(() => {
    const checkCcSwitchAvailability = async () => {
      try {
        const available = await hasCcSwitchDb();
        setCcSwitchAvailable(available);
      } catch {
        setCcSwitchAvailable(false);
      }
    };
    void checkCcSwitchAvailability();
  }, []);

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

        await createGeminiCliProvider({
          name: candidate.name,
          category: (candidate.normalizedCategory || 'custom') as GeminiCliProviderInput['category'],
          settingsConfig,
          sourceProviderId: candidate.sourceProviderId,
          websiteUrl: candidate.websiteUrl,
          notes: candidate.notes,
          icon: candidate.icon,
          iconColor: candidate.iconColor,
        });
        ok += 1;
      } catch (error) {
        console.error('Failed to import Gemini CLI provider from CC Switch:', candidate.name, error);
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
    await refreshTrayMenu();
  }, [providers, loadConfig, t]);

  const hasInitializedRef = React.useRef(false);
  React.useEffect(() => {
    if (!isActive) {
      hasInitializedRef.current = true;
      return;
    }
    if (hasInitializedRef.current) {
      void loadConfig(true);
    }
  }, [isActive, loadConfig]);

  React.useEffect(() => {
    const handleTrayConfigRefresh = (event: Event) => {
      event.preventDefault();
      void loadConfig(true);
    };
    const handleDeepLinkImport = (event: Event) => {
      const detail = (event as CustomEvent<{ app?: string; id?: string }>).detail;
      if (detail?.app === 'gemini') {
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

  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig({
    t,
    translationKeyPrefix: 'geminicli',
    defaultConfig: '{}',
    rootDirectoryChangeLocked: gatewayTakeoverActive,
    rootDirectoryChangeLockedText: t('gateway.proxy.rootDirectorySaveLockedTooltip'),
    loadConfig,
    getCommonConfig: getGeminiCliCommonConfig,
    saveCommonConfig: saveGeminiCliCommonConfig,
  });

  const handleOpenFolder = async () => {
    if (!configPath) return;

    try {
      await revealItemInDir(configPath);
    } catch {
      try {
        const parentDir = configPath.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open Gemini CLI folder:', error);
        const errorMsg = error instanceof Error ? error.message : String(error);
        message.error(errorMsg || t('common.error'));
      }
    }
  };

  const handlePreviewCurrentConfig = async () => {
    try {
      const settings = await readGeminiCliSettings();
      setPreviewData(settings);
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview Gemini CLI config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const oldIndex = providers.findIndex((provider) => provider.id === active.id);
    const newIndex = providers.findIndex((provider) => provider.id === over.id);
    if (oldIndex < 0 || newIndex < 0) return;

    const oldProviders = [...providers];
    const newProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(newProviders);

    try {
      await reorderGeminiCliProviders(newProviders.map((provider) => provider.id));
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to reorder Gemini CLI providers:', error);
      setProviders(oldProviders);
      message.error(t('common.error'));
    }
  };

  const handleAddProvider = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: GeminiCliProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: GeminiCliProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name}_copy`,
      isApplied: false,
    });
    setIsCopyMode(true);
    setProviderModalOpen(true);
  };

  const handleShareProvider = (provider: GeminiCliProvider) => {
    setShareProvider(provider);
  };

  const handleDeleteProvider = (provider: GeminiCliProvider) => {
    Modal.confirm({
      title: t('geminicli.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteGeminiCliProvider(provider.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete Gemini CLI provider:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        }
      },
    });
  };

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('geminicli');
  const { viewMode, setViewMode } = useProviderListViewPrefs('geminicli');
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
    (provider: GeminiCliProvider) => provider.id !== GEMINI_CLI_LOCAL_PROVIDER_ID
      && (officialAccountsByProviderId[provider.id]?.length ?? 0) === 0,
    [officialAccountsByProviderId],
  );

  /** Why a row cannot be picked, for the disabled checkbox's tooltip. */
  const batchBlockReason = React.useCallback(
    (provider: GeminiCliProvider): ProviderSelectionBlockReason | undefined => {
      if (provider.id === GEMINI_CLI_LOCAL_PROVIDER_ID) return 'local';
      if ((officialAccountsByProviderId[provider.id]?.length ?? 0) > 0) return 'officialAccount';
      return undefined;
    },
    [officialAccountsByProviderId],
  );

  const handleBatchDeleteProviders = React.useCallback(
    async (ids: string[]): Promise<boolean> => {
      const providersToDelete = providers.filter(
        (provider) => ids.includes(provider.id) && canBatchDeleteProvider(provider),
      );
      if (providersToDelete.length === 0) return false;
      try {
        for (const provider of providersToDelete) {
          await deleteGeminiCliProvider(provider.id);
        }
        await loadConfig();
        await refreshTrayMenu();
        message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete Gemini CLI providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig();
        return false;
      }
    },
    [providers, canBatchDeleteProvider, loadConfig, t],
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

  /**
   * The locate target: the applied provider, unless that is the `__local__`
   * bridge (which the UI never treats as a managed applied preset, so pointing
   * the locate ring at it would report a state the rest of the page denies).
   */
  const locateProviderId = appliedProviderId === GEMINI_CLI_LOCAL_PROVIDER_ID
    ? ''
    : appliedProviderId;

  /** One provider's card props, shared by both layouts. */
  const renderProviderCard = (provider: GeminiCliProvider) => (
    <GeminiCliProviderCard
      key={provider.id}
      provider={provider}
      viewMode={viewMode}
      isApplied={provider.id === appliedProviderId}
      dragDisabled={providerDragDisabled}
      onEdit={handleEditProvider}
      onDelete={handleDeleteProvider}
      onCopy={handleCopyProvider}
      onShare={handleShareProvider}
      onSelect={handleSelectProvider}
      onToggleDisabled={handleToggleDisabled}
      selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.id)}
      selected={providerBatch.selectedIds.has(provider.id)}
      onSelectChange={(checked) => providerBatch.toggleSelect(provider.id, checked)}
      selectDisabledReason={
        providerBatch.selectionMode
          ? providerSelectionDisabledReason(t, batchBlockReason(provider))
          : undefined
      }
      officialAccounts={officialAccountsByProviderId[provider.id] || []}
      onOfficialAccountLogin={handleStartOfficialAccountOauth}
      onOfficialLocalAccountSave={handleSaveOfficialLocalAccount}
      onOfficialAccountApply={handleApplyOfficialAccount}
      onOfficialAccountDelete={handleDeleteOfficialAccount}
      onOfficialAccountRefresh={handleRefreshOfficialAccount}
      onOfficialAccountViewDetails={handleViewOfficialAccountDetails}
      refreshingOfficialAccountId={refreshingOfficialAccountId}
      savingOfficialAccountId={savingOfficialAccountId}
      gatewayTakeoverActive={gatewayTakeoverActive}
      gatewayStatus={gatewayCliStatus}
      onGatewayStatusChange={async (status) => {
        setGatewayCliStatus(status);
        await loadConfig();
      }}
    />
  );

  const handleSelectProvider = async (provider: GeminiCliProvider) => {
    try {
      await selectGeminiCliProvider(provider.id);
      message.success(t('geminicli.apply.success'));
      noteProviderUsed(provider.id);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to select Gemini CLI provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleStartOfficialAccountOauth = async (provider: GeminiCliProvider) => {
    try {
      await startGeminiCliOfficialAccountOauth(provider.id);
      message.success(t('geminicli.provider.officialAccountOauthSuccess'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to start Gemini official account OAuth:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleApplyOfficialAccount = async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    try {
      await applyGeminiCliOfficialAccount(provider.id, account.id);
      message.success(t('geminicli.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to apply Gemini official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleSaveOfficialLocalAccount = async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    try {
      setSavingOfficialAccountId(account.id);
      await saveGeminiCliOfficialLocalAccount(provider.id);
      if (officialAccountDetails?.account.id === account.id) {
        setOfficialAccountDetails(null);
      }
      message.success(t('geminicli.provider.officialAccountSaveSuccess'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Gemini local official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setSavingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleDeleteOfficialAccount = (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    Modal.confirm({
      title: t('geminicli.provider.officialAccountDeleteConfirm', {
        name: account.email || account.projectId || account.name,
      }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteGeminiCliOfficialAccount(provider.id, account.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete Gemini official account:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        }
      },
    });
  };

  const handleRefreshOfficialAccount = async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    try {
      setRefreshingOfficialAccountId(account.id);
      const refreshedAccount = await refreshGeminiCliOfficialAccountLimits(provider.id, account.id);
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
      message.success(t('geminicli.provider.officialAccountRefreshSuccess'));
    } catch (error) {
      console.error('Failed to refresh Gemini official account quota:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setRefreshingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleViewOfficialAccountDetails = (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
  ) => {
    setOfficialAccountDetails({ provider, account });
  };

  const handleCopyOfficialAccountToken = React.useCallback(async (
    provider: GeminiCliProvider,
    account: GeminiCliOfficialAccount,
    tokenKind: 'access' | 'refresh',
  ) => {
    try {
      await copyGeminiCliOfficialAccountToken(provider.id, account.id, tokenKind);
      message.success(t('common.copied'));
    } catch (error) {
      console.error('Failed to copy Gemini official account token:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  }, [t]);

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

  const handleToggleDisabled = async (provider: GeminiCliProvider, isDisabled: boolean) => {
    try {
      await toggleGeminiCliProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('geminicli.providerDisabled') : t('geminicli.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle Gemini CLI provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleProviderSubmit = async (values: GeminiCliProviderFormValues) => {
    try {
      const providerInput: GeminiCliProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig: values.settingsConfig,
        meta: values.meta,
        notes: values.notes,
      };

      const isLocalTemp = editingProvider?.id === GEMINI_CLI_LOCAL_PROVIDER_ID;
      const gatewayModeBeforeSave = resolveGatewayReengageMode(gatewayCliStatus);
      const gatewayAggregateBeforeSave = toGatewayAggregateReengageConfig(gatewayCliStatus);
      const shouldReengageGatewayProxy =
        Boolean(editingProvider && !isCopyMode && !isLocalTemp && editingProvider.isApplied) &&
        gatewayModeBeforeSave !== null;

      await saveProviderWithGatewayReengage({
        gatewayMode: shouldReengageGatewayProxy ? gatewayModeBeforeSave : null,
        aggregateConfig: shouldReengageGatewayProxy ? gatewayAggregateBeforeSave : null,
        restoreDirect: () => restoreProxyGatewayCliDirect('gemini'),
        engageSingle: () => engageProxyGatewaySingle('gemini', editingProvider?.id || ''),
        engageFailover: () => engageProxyGatewayFailover('gemini'),
        // No `engageAggregate`: aggregate mode is Codex-only, so this CLI can
        // never report it and the re-engage helper would reject the replay.
        onGatewayStatusChange: setGatewayCliStatus,
        saveProvider: async () => {
          if (isLocalTemp) {
            await saveGeminiCliLocalConfig({ provider: providerInput });
          } else if (editingProvider && !isCopyMode) {
            await updateGeminiCliProvider({
              ...editingProvider,
              name: values.name,
              category: values.category,
              settingsConfig: values.settingsConfig,
              meta: values.meta,
              notes: values.notes,
            });
          } else {
            await createGeminiCliProvider(providerInput);
          }
        },
      });

      message.success(t('common.success'));
      setProviderModalOpen(false);
      setIsCopyMode(false);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Gemini CLI provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
      throw error;
    }
  };

  return (
    <SectionSidebarLayout
      sidebarTitle={t('geminicli.title')}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'geminicli-providers':
            return <DatabaseOutlined />;
          case 'geminicli-global-prompt':
            return <FileTextOutlined />;
          case 'geminicli-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'geminicli-providers':
            setProviderListCollapsed(false);
            break;
          case 'geminicli-global-prompt':
            setPromptExpandNonce((value) => value + 1);
            break;
          case 'geminicli-session-manager':
            setSessionManagerExpandNonce((value) => value + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        <CodingPageHeader
          title={t('geminicli.title')}
          docsUrl="https://github.com/google-gemini/gemini-cli"
          onPreviewConfig={appliedProviderId ? () => void handlePreviewCurrentConfig() : undefined}
          configPath={configPath || '~/.gemini/settings.json'}
          onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
          onOpenFolder={() => void handleOpenFolder()}
          onRefresh={() => void loadConfig()}
          onMoreOptions={() => setSettingsModalOpen(true)}
        />

        <ProviderListSection
          sectionId="geminicli-providers"
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
          dragDisabledBySort={providerDragDisabled}
          locateProviderId={locateProviderId}
          onOpenCommonConfig={() => setCommonConfigModalOpen(true)}
          onAddProvider={handleAddProvider}
          headerExtra={
            <GatewayFailoverButton
              cliKey="gemini"
              status={gatewayCliStatus}
              primaryProviderNeedsGatewayProxy={primaryGatewayProviderNeedsProxy}
              primaryProviderNeedsProxyReason={primaryGatewayProviderNeedsProxyReason}
              onStatusChange={setGatewayCliStatus}
            />
          }
          viewMode={viewMode}
          onViewModeChange={setViewMode}
          hint={
            <>
              <div>{t('geminicli.pageHint')}</div>
              <div>{t('geminicli.pageWarning')}</div>
            </>
          }
          footer={
            ccSwitchAvailable ? (
              <Button
                type="dashed"
                icon={<ImportOutlined />}
                onClick={() => setCcSwitchImportModalOpen(true)}
              >
                {t('common.ccSwitch.importFromCcSwitch')}
              </Button>
            ) : undefined
          }
        >
          {/* One `DndContext` serves both layouts; only the sorting strategy and
              the drag modifier change with the view. */}
          <DndContext
            sensors={providerBatchDragDisabled ? [] : sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleDragEnd}
            modifiers={[viewMode === 'card' ? restrictToWindowEdges : restrictToVerticalAxis]}
          >
            <SortableContext
              items={visibleProviders.map((provider) => provider.id)}
              strategy={viewMode === 'card' ? rectSortingStrategy : verticalListSortingStrategy}
            >
              {viewMode === 'card' ? (
                <ProviderCardGrid
                  items={visibleProviders}
                  getKey={(provider) => provider.id}
                  renderItem={renderProviderCard}
                />
              ) : (
                <div>{visibleProviders.map(renderProviderCard)}</div>
              )}
            </SortableContext>
          </DndContext>
        </ProviderListSection>

        <div
          id="geminicli-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('common.prompt.title')}
        >
          <GlobalPromptSettings
            key={`geminicli-prompt-${promptExpandNonce}`}
            toolName="Gemini"
            promptFileName="GEMINI.md"
            service={geminiCliPromptApi}
            collapseKey="geminicli-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <div
          id="geminicli-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title')}
        >
          <SessionManagerPanel tool="geminicli" expandNonce={sessionManagerExpandNonce} />
        </div>

        <GeminiCliProviderFormModal
          open={providerModalOpen}
          provider={editingProvider}
          isCopy={isCopyMode}
          onCancel={() => {
            setProviderModalOpen(false);
            setEditingProvider(null);
            setIsCopyMode(false);
          }}
          onSubmit={handleProviderSubmit}
        />

        {ccSwitchAvailable && (
          <ImportFromCcSwitchModal
            open={ccSwitchImportModalOpen}
            appType="gemini"
            existingProviderIds={providers
              .map((provider) => provider.sourceProviderId)
              .filter((id): id is string => Boolean(id))}
            onClose={() => setCcSwitchImportModalOpen(false)}
            onImport={handleImportFromCcSwitch}
          />
        )}

        <ShareProviderModal
          open={shareProvider !== null}
          sourceApp="gemini"
          provider={shareProvider}
          onClose={() => setShareProvider(null)}
        />

        <GeminiCliCommonConfigModal
          open={commonConfigModalOpen}
          onCancel={() => setCommonConfigModalOpen(false)}
          onSuccess={() => {
            void loadConfig();
            void refreshTrayMenu();
          }}
          gatewaySaveLocked={gatewayTakeoverActive}
        />

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(rootPathInfo)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        <JsonPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('geminicli.preview.currentConfigTitle')}
          data={previewData}
        />

        <Modal
          open={Boolean(officialAccountDetails)}
          title={t('geminicli.provider.officialAccountDetailsTitle')}
          onCancel={() => setOfficialAccountDetails(null)}
          footer={null}
          width={720}
        >
          {officialAccountDetails && (
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label={t('geminicli.provider.officialAccountLabel')}>
                {officialAccountDetails.account.email
                  || officialAccountDetails.account.projectId
                  || officialAccountDetails.account.name}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountProjectId')}>
                {officialAccountDetails.account.projectId || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountPlanType')}>
                {officialAccountDetails.account.planType || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountWeeklyLimit')}>
                {officialAccountDetails.account.limitWeeklyText || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountWeeklyResetAt')}>
                {formatUnixTimestamp(officialAccountDetails.account.limitWeeklyResetAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountLastLimitRefreshAt')}>
                {formatDateTime(officialAccountDetails.account.lastLimitsFetchedAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountTokenExpiresAt')}>
                {formatUnixTimestamp(officialAccountDetails.account.tokenExpiresAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountAccessToken')}>
                {renderTokenPreview(
                  maskTokenPreview('access', officialAccountDetails.account),
                  () => handleCopyOfficialAccountToken(
                    officialAccountDetails.provider,
                    officialAccountDetails.account,
                    'access',
                  ),
                )}
              </Descriptions.Item>
              <Descriptions.Item label={t('geminicli.provider.officialAccountRefreshToken')}>
                {renderTokenPreview(
                  maskTokenPreview('refresh', officialAccountDetails.account),
                  () => handleCopyOfficialAccountToken(
                    officialAccountDetails.provider,
                    officialAccountDetails.account,
                    'refresh',
                  ),
                )}
              </Descriptions.Item>
              {officialAccountDetails.account.lastError && (
                <Descriptions.Item label={t('geminicli.provider.officialAccountLastErrorLabel')}>
                  <Text type="danger">{officialAccountDetails.account.lastError}</Text>
                </Descriptions.Item>
              )}
            </Descriptions>
          )}
        </Modal>

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={(visible) => setSidebarHidden('geminicli', !visible)}
        />
      </div>
    </SectionSidebarLayout>
  );
};

export default GeminiCliPage;
