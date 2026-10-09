import React from 'react';
import { Alert, Button, Modal, Space, Spin, Typography, message } from 'antd';
import {
  DatabaseOutlined,
  FileTextOutlined,
  ImportOutlined,
  MessageOutlined,
} from '@ant-design/icons';
import { DndContext, closestCenter, type DragEndEvent } from '@dnd-kit/core';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import {
  SortableContext,
  arrayMove,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { useTranslation } from 'react-i18next';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import ProviderListSection from '@/features/coding/shared/ProviderListSection';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
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
  buildProviderConnectivityBatchTarget,
  runProviderConnectivityBatch,
} from '@/features/coding/shared/providerConnectivity/batchTest';
import ProviderConnectivityTestModal, {
  buildZcodeProviderConnectivityInfo,
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import {
  buildFavoriteProviderStorageKey,
  dedupeFavoriteProvidersByPayload,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  type ZcodeFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import {
  deleteFavoriteProvider,
  listFavoriteProviders,
  upsertFavoriteProvider,
  type OpenCodeFavoriteProvider,
} from '@/services/opencodeApi';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import ZcodeImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import { hasAllApiHubExtension } from '@/services/appApi';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import { buildZcodeFavoriteProviderConfig } from '../utils/zcodeFavoriteProvider';
import {
  buildZcodeSettingsConfigFromImport,
  extractZcodeProviderFromAllApiHub,
  extractZcodeProviderFromCcSwitch,
} from '../utils/zcodeImportMapping';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import type { FetchModelsApplyResult } from '@/components/common/FetchModelsModal/types';
import { useSettingsStore } from '@/stores';
import { refreshTrayMenu } from '@/services/appApi';
import {
  applyZcodeOfficialAccount,
  cancelZcodeOfficialAccountOauth,
  createZcodeProvider,
  deleteZcodeOfficialAccount,
  deleteZcodeProvider,
  getZcodeCommonConfig,
  getZcodeConfigFilePath,
  getZcodeGenerationStatus,
  getZcodePreview,
  getZcodeRootPathInfo,
  listZcodeOfficialAccounts,
  listZcodeProviders,
  reorderZcodeProviders,
  revealZcodeConfigFolder,
  saveZcodeCommonConfig,
  saveZcodeOfficialAccountIndex,
  saveZcodeOfficialLocalAccount,
  saveZcodeProvider,
  selectZcodeProvider,
  startZcodeOfficialAccountOauth,
  toggleZcodeProviderDisabled,
  updateZcodeProvider,
} from '@/services/zcodeApi';
import { zcodePromptApi } from '@/services/zcodePromptApi';
import {
  ZCODE_LOGIN_PROVIDERS,
  type ConfigPathInfo,
  type ZcodeConfigPreview,
  type ZcodeModelRow,
  type ZcodeOfficialAccount,
  type ZcodeProvider,
  type ZcodeSettingsConfig,
} from '@/types/zcode';
import ZcodeProviderCard from '../components/ZcodeProviderCard';
import ZcodeOfficialAccountCard from '../components/ZcodeOfficialAccountCard';
import ZcodeModelFormModal from '../components/ZcodeModelFormModal';
import ZcodeProviderFormModal from '../components/ZcodeProviderFormModal';
import { parseZcodeProviderSettings } from '../utils/zcodeSettingsConfig';
import {
  buildZcodeModelRowFromPreset,
  preferredPresetNpmTypes,
} from '../utils/zcodeModelFields';
import { findPresetModelById } from '@/constants/presetModels';


/**
 * Sortable id of the official-account card.
 *
 * The card is not a provider row, so it has no id of its own to sort by; this
 * sentinel stands in for it in the one ordering that mixes the two.
 */
const OFFICIAL_ACCOUNT_CARD_ID = 'zcode-official-account';

const ZcodePage: React.FC = () => {
  const { t } = useTranslation();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const { isActive } = useKeepAlive();

  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [providers, setProviders] = React.useState<ZcodeProvider[]>([]);
  const [hasNewGenerationRegistry, setHasNewGenerationRegistry] = React.useState(true);
  const [officialAccounts, setOfficialAccounts] = React.useState<ZcodeOfficialAccount[]>([]);
  /** Provider cards above the official-account card; UI state, not ZCode's. */
  const [officialAccountIndex, setOfficialAccountIndex] = React.useState(0);
  const [applyingOfficialAccountId, setApplyingOfficialAccountId] = React.useState<string | null>(
    null,
  );
  const [savingOfficialAccount, setSavingOfficialAccount] = React.useState(false);
  /**
   * The provider a browser login is currently waiting on. Non-null means a flow
   * is open in the browser: the modal owns the pending state and the cancel
   * button is the only way out, because the command resolves when the flow does.
   */
  const [officialLoginProviderId, setOfficialLoginProviderId] = React.useState<string | null>(null);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [formModalOpen, setFormModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<ZcodeProvider | null>(null);
  /** 弹窗预填了 provider 但要存成新记录（复制），而不是改原来那条。 */
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const [providerKeyword, setProviderKeyword] = React.useState('');
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<
    Record<string, ProviderConnectivityStatusItem>
  >({});
  const [batchTestingProviders, setBatchTestingProviders] = React.useState(false);
  const [modelModal, setModelModal] = React.useState<{
    provider: ZcodeProvider;
    /** `null` means "add"; otherwise the index of the row being edited. */
    modelIndex: number | null;
    /**
     * Seed row for "add", used by copy. A copy cannot reuse the source id —
     * ZCode keys catalog rows by model id — so it opens the *add* flow
     * prefilled and lets the user name the new one.
     */
    prefill?: ZcodeModelRow;
  } | null>(null);
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string | null>(null);
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewData] = React.useState<ZcodeConfigPreview | null>(null);
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  /** 单个 provider 的连通性测试弹窗（共享 `ProviderConnectivityTestModal`）。 */
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityInfo, setConnectivityInfo] = React.useState<ProviderConnectivityInfo | null>(
    null,
  );
  /** Provider whose model list is in batch-delete mode, if any. */
  const [modelBatchDeleteProviderId, setModelBatchDeleteProviderId] = React.useState<string | null>(
    null,
  );
  const [selectedModelIdsByProvider, setSelectedModelIdsByProvider] = React.useState<
    Record<string, string[]>
  >({});

  const sidebarHidden = sidebarHiddenByPage.zcode ?? false;

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('zcode');

  // Search and non-custom sort modes bypass sort_index, so dragging would write
  // a stale custom order — dnd is only enabled in custom mode.
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

  /** The card wearing ZCode's "默认" tag — what the provider list's 定位 targets. */
  const appliedProviderId = React.useMemo(
    () => providers.find((provider) => provider.isApplied)?.id ?? '',
    [providers],
  );

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(
    () => [
      {
        id: 'zcode-providers',
        title: t('zcode.provider.title', { defaultValue: '供应商' }),
        order: 1,
      },
      {
        id: 'zcode-global-prompt',
        title: t('common.prompt.title'),
        order: 2,
      },
      {
        id: 'zcode-session-manager',
        title: t('sessionManager.title', { defaultValue: '会话管理' }),
        order: 3,
      },
    ],
    [t],
  );

  const loadConfig = React.useCallback(async () => {
    setLoading(true);
    try {
      const [
        path,
        nextRootPathInfo,
        generation,
        nextProviders,
        nextOfficialAccounts,
        nextCommonConfig,
      ] = await Promise.all([
        getZcodeConfigFilePath(),
        getZcodeRootPathInfo(),
        getZcodeGenerationStatus(),
        listZcodeProviders(),
        listZcodeOfficialAccounts(),
        getZcodeCommonConfig(),
      ]);
      setConfigPath(path);
      setRootPathInfo(nextRootPathInfo);
      setHasNewGenerationRegistry(generation);
      setProviders(nextProviders);
      setOfficialAccounts(nextOfficialAccounts);
      setOfficialAccountIndex(nextCommonConfig.officialAccountIndex ?? 0);
    } catch (error) {
      console.error('Failed to load ZCode config:', error);
      const detail = error instanceof Error ? error.message : String(error);
      void message.error(detail ? `${t('zcode.loadFailed')}：${detail}` : t('zcode.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    if (isActive) {
      void loadConfig();
    }
  }, [isActive, loadConfig]);

  /**
   * Official accounts live in their own file, so they refresh on their own
   * rather than through `loadConfig` — switching one must not reload the
   * provider registry, which the user may be mid-edit on.
   */
  const loadOfficialAccounts = React.useCallback(async () => {
    try {
      setOfficialAccounts(await listZcodeOfficialAccounts());
    } catch (error) {
      console.error('Failed to load ZCode official accounts:', error);
      const detail = error instanceof Error ? error.message : String(error);
      void message.error(
        detail
          ? `${t('zcode.officialAccount.loadFailed')}：${detail}`
          : t('zcode.officialAccount.loadFailed'),
      );
    }
  }, [t]);

  const handleSaveOfficialLocalAccount = React.useCallback(async () => {
    setSavingOfficialAccount(true);
    try {
      await saveZcodeOfficialLocalAccount();
      await loadOfficialAccounts();
      void message.success(t('zcode.officialAccount.saveSuccess'));
    } catch (error) {
      console.error('Failed to save ZCode official account:', error);
      const detail = error instanceof Error ? error.message : String(error);
      void message.error(
        detail
          ? `${t('zcode.officialAccount.saveFailed')}：${detail}`
          : t('zcode.officialAccount.saveFailed'),
      );
    } finally {
      setSavingOfficialAccount(false);
    }
  }, [loadOfficialAccounts, t]);

  const handleApplyOfficialAccount = React.useCallback(
    async (account: ZcodeOfficialAccount) => {
      setApplyingOfficialAccountId(account.id);
      try {
        const result = await applyZcodeOfficialAccount(account.id);
        await loadOfficialAccounts();
        // The outgoing login is captured automatically, so say so — otherwise
        // an account the user never created appears with no explanation.
        if (result.preservedAs) {
          void message.success(
            t('zcode.officialAccount.appliedWithBackup', { name: result.preservedAs }),
          );
        } else {
          void message.success(t('zcode.officialAccount.applySuccess'));
        }
      } catch (error) {
        console.error('Failed to apply ZCode official account:', error);
        const detail = error instanceof Error ? error.message : String(error);
        void message.error(
          detail
            ? `${t('zcode.officialAccount.applyFailed')}：${detail}`
            : t('zcode.officialAccount.applyFailed'),
        );
      } finally {
        setApplyingOfficialAccountId(null);
      }
    },
    [loadOfficialAccounts, t],
  );

  const handleDeleteOfficialAccount = React.useCallback(
    async (account: ZcodeOfficialAccount) => {
      try {
        await deleteZcodeOfficialAccount(account.id);
        await loadOfficialAccounts();
        void message.success(t('zcode.officialAccount.deleteSuccess'));
      } catch (error) {
        console.error('Failed to delete ZCode official account:', error);
        const detail = error instanceof Error ? error.message : String(error);
        void message.error(
          detail
            ? `${t('zcode.officialAccount.deleteFailed')}：${detail}`
            : t('zcode.officialAccount.deleteFailed'),
        );
      }
    },
    [loadOfficialAccounts, t],
  );

  const handleCancelOfficialLogin = React.useCallback(async () => {
    try {
      await cancelZcodeOfficialAccountOauth();
    } catch (error) {
      console.error('Failed to cancel ZCode login:', error);
    }
  }, []);

  const handleStartOfficialLogin = React.useCallback(
    async (providerId: string) => {
      setOfficialLoginProviderId(providerId);
      try {
        await startZcodeOfficialAccountOauth(providerId);
        await loadOfficialAccounts();
        void message.success(t('zcode.officialAccount.loginSuccess'));
      } catch (error) {
        console.error('Failed to log in to ZCode:', error);
        const detail = error instanceof Error ? error.message : String(error);
        void message.error(
          detail
            ? `${t('zcode.officialAccount.loginFailed')}：${detail}`
            : t('zcode.officialAccount.loginFailed'),
        );
      } finally {
        setOfficialLoginProviderId(null);
      }
    },
    [loadOfficialAccounts, t],
  );

  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig({
    t,
    translationKeyPrefix: 'zcode',
    defaultConfig: '{}',
    loadConfig,
    getCommonConfig: getZcodeCommonConfig,
    saveCommonConfig: async ({ config, rootDir, clearRootDir }) => {
      await saveZcodeCommonConfig({ config, rootDir, clearRootDir });
    },
  });

  const handleOpenFolder = async () => {
    try {
      if (configPath) {
        await revealItemInDir(configPath);
      } else {
        await revealZcodeConfigFolder();
      }
    } catch {
      await revealZcodeConfigFolder();
    }
  };

  /**
   * Copies a provider into the favorites store before it is deleted.
   *
   * The store is shared across CLIs, so a ZCode provider becomes restorable from
   * the provider list's import entry — the only undo a delete has.
   */
  const backUpProviderToFavorites = React.useCallback(async (provider: ZcodeProvider) => {
    await upsertFavoriteProvider(
      buildFavoriteProviderStorageKey('zcode', provider.id),
      buildZcodeFavoriteProviderConfig(provider),
    );
  }, []);

  /**
   * Drops favorites that hold the same provider twice.
   *
   * A record is written both when a provider is deleted and when one is
   * imported, so the same config can land under two keys. The store is shared
   * with the other CLIs, so the duplicates are cleaned here rather than left for
   * whichever tool reads the list next. Nothing else in this page reads the
   * favorites list — the import dialog fetches its own.
   */
  const pruneDuplicateFavoriteProviders = React.useCallback(async () => {
    try {
      const all = await listFavoriteProviders();
      const zcodeFavorites = all.filter((provider) =>
        isFavoriteProviderForSource('zcode', provider),
      );
      const currentStorageKeys = new Set(
        providers.map((provider) => buildFavoriteProviderStorageKey('zcode', provider.id)),
      );
      const { duplicateIds } = dedupeFavoriteProvidersByPayload(zcodeFavorites, currentStorageKeys);
      await Promise.all(
        duplicateIds.map(async (providerId) => {
          try {
            await deleteFavoriteProvider(providerId);
          } catch (error) {
            console.error('Failed to delete duplicate ZCode favorite provider:', error);
          }
        }),
      );
    } catch (error) {
      console.error('Failed to load ZCode favorite providers:', error);
    }
  }, [providers]);

  React.useEffect(() => {
    void pruneDuplicateFavoriteProviders();
  }, [pruneDuplicateFavoriteProviders]);

  /** Adopts providers picked from the shared favorites list. */
  const handleImportFavoriteProviders = React.useCallback(
    async (providersToImport: OpenCodeFavoriteProvider[]) => {
      let importedCount = 0;
      for (const favoriteProvider of providersToImport) {
        const payload = getFavoriteProviderPayload<ZcodeFavoriteProviderPayload>(favoriteProvider);
        if (!payload) {
          continue;
        }
        try {
          const created = await createZcodeProvider({
            name: payload.name,
            category: payload.category,
            settingsConfig: payload.settingsConfig,
            notes: payload.notes,
          });
          // Keep the imported row in the store so it survives a later delete.
          try {
            await upsertFavoriteProvider(
              buildFavoriteProviderStorageKey('zcode', created.id),
              buildZcodeFavoriteProviderConfig(created),
            );
          } catch (favoriteError) {
            console.error('Failed to keep the imported ZCode favorite provider:', favoriteError);
          }
          importedCount += 1;
        } catch (error) {
          console.error('Failed to import ZCode favorite provider:', error);
        }
      }
      void message.success(t('common.success'));
      setImportModalOpen(false);
      await loadConfig();
      await pruneDuplicateFavoriteProviders();
      await refreshTrayMenu();
      return importedCount;
    },
    [loadConfig, pruneDuplicateFavoriteProviders, t],
  );

  /** Whether the two optional import sources are present on this machine. */
  React.useEffect(() => {
    const checkSources = async () => {
      try {
        setAllApiHubAvailable(await hasAllApiHubExtension());
      } catch {
        setAllApiHubAvailable(false);
      }
      try {
        setCcSwitchAvailable(await hasCcSwitchDb());
      } catch {
        setCcSwitchAvailable(false);
      }
    };
    void checkSources();
  }, []);

  const handleImportFromAllApiHub = React.useCallback(
    async (imported: OpenCodeAllApiHubProvider[]) => {
      let ok = 0;
      let fail = 0;
      for (const item of imported) {
        const mapped = extractZcodeProviderFromAllApiHub(item);
        if (!mapped) {
          continue;
        }
        try {
          await createZcodeProvider({
            name: mapped.name,
            category: 'custom',
            settingsConfig: buildZcodeSettingsConfigFromImport(mapped),
            // Remembers where the row came from, so re-importing the same
            // source provider is recognised instead of duplicated.
            sourceProviderId: item.providerId,
          });
          ok += 1;
        } catch (error) {
          console.error('Failed to import ZCode provider from All API Hub:', item.providerId, error);
          fail += 1;
        }
      }
      setAllApiHubImportModalOpen(false);
      if (ok > 0 && fail === 0) {
        void message.success(t('common.allApiHub.importSuccess', { count: ok }));
      } else if (fail > 0) {
        void message.error(t('common.error'));
      }
      await loadConfig();
      await refreshTrayMenu();
    },
    [loadConfig, t],
  );

  const handleImportFromCcSwitch = React.useCallback(
    async (imported: CcSwitchProviderCandidate[]) => {
      const existingSourceIds = new Set(
        providers.map((provider) => provider.sourceProviderId).filter(Boolean),
      );
      let ok = 0;
      let fail = 0;
      for (const candidate of imported) {
        const sourceProviderId = candidate.sourceProviderId ?? candidate.providerId;
        if (existingSourceIds.has(sourceProviderId)) {
          continue;
        }
        const mapped = extractZcodeProviderFromCcSwitch(candidate);
        if (!mapped) {
          continue;
        }
        try {
          await createZcodeProvider({
            name: mapped.name,
            category: 'custom',
            settingsConfig: buildZcodeSettingsConfigFromImport(mapped),
            sourceProviderId,
          });
          ok += 1;
        } catch (error) {
          console.error('Failed to import ZCode provider from CC Switch:', candidate.providerId, error);
          fail += 1;
        }
      }
      setCcSwitchImportModalOpen(false);
      if (ok > 0 && fail === 0) {
        void message.success(t('common.ccSwitch.importSuccess', { count: ok }));
      } else if (ok > 0 && fail > 0) {
        void message.warning(t('common.ccSwitch.importPartial', { ok, fail }));
      } else if (fail > 0) {
        void message.error(t('common.error'));
      }
      await loadConfig();
      await refreshTrayMenu();
    },
    [providers, loadConfig, t],
  );

  const handleDeleteProvider = async (provider: ZcodeProvider) => {
    try {
      await backUpProviderToFavorites(provider);
      await deleteZcodeProvider(provider.id);
      await refreshTrayMenu();
      await loadConfig();
      await pruneDuplicateFavoriteProviders();
    } catch (error) {
      console.error('Failed to delete ZCode provider:', error);
      void message.error(String(error));
    }
  };

  /**
   * Flips a provider's disabled flag.
   *
   * Disabling hides the provider from ZCode's picker without deleting it; it
   * does not clear a `defaultModelSelection` that points here, so a disabled
   * provider can still be the active one until another is chosen.
   */
  const handleToggleProviderDisabled = async (provider: ZcodeProvider, isDisabled: boolean) => {
    try {
      await toggleZcodeProviderDisabled(provider.id, isDisabled);
      await loadConfig();
    } catch (error) {
      console.error('Failed to toggle ZCode provider:', error);
      void message.error(String(error));
    }
  };

  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) {
      return;
    }
    // Indices come from the merged ordering, not from `providers`: the official
    // -account card is one of the items and shifts every index below it.
    const oldIndex = sortableItemIds.indexOf(String(active.id));
    const newIndex = sortableItemIds.indexOf(String(over.id));
    if (oldIndex < 0 || newIndex < 0) {
      return;
    }

    const nextIds = arrayMove(sortableItemIds, oldIndex, newIndex);
    const nextProviderIds = nextIds.filter((id) => id !== OFFICIAL_ACCOUNT_CARD_ID);
    const nextCardIndex = nextIds.indexOf(OFFICIAL_ACCOUNT_CARD_ID);
    const providerOrderChanged = nextProviderIds.some((id, index) => id !== providers[index]?.id);
    const cardSlotChanged = nextCardIndex !== officialAccountIndex;

    // Optimistic: the list resequences immediately and reloads from the backend
    // afterwards, so a failure still converges on the stored order.
    const providerById = new Map(providers.map((provider) => [provider.id, provider]));
    setProviders(
      nextProviderIds
        .map((id) => providerById.get(id))
        .filter((provider): provider is ZcodeProvider => Boolean(provider)),
    );
    setOfficialAccountIndex(nextCardIndex);

    try {
      if (providerOrderChanged) {
        await reorderZcodeProviders(nextProviderIds);
      }
      if (cardSlotChanged) {
        await saveZcodeOfficialAccountIndex(nextCardIndex);
      }
      await loadConfig();
    } catch (error) {
      console.error('Failed to reorder ZCode providers:', error);
      void message.error(String(error));
      await loadConfig();
    }
  };

  const handleBatchDeleteProviders = React.useCallback(
    async (ids: string[]): Promise<boolean> => {
      const providersToDelete = providers.filter((provider) => ids.includes(provider.id));
      if (providersToDelete.length === 0) {
        return false;
      }
      try {
        await backupProvidersBeforeDelete(
          providersToDelete,
          backUpProviderToFavorites,
          (provider) => t('common.batch.backupFailed', { name: provider.name }),
        );
        for (const provider of providersToDelete) {
          await deleteZcodeProvider(provider.id);
        }
        await refreshTrayMenu();
        await loadConfig();
        await pruneDuplicateFavoriteProviders();
        void message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete ZCode providers:', error);
        void message.error(error instanceof Error ? error.message : String(error));
        return false;
      }
    },
    [providers, loadConfig, pruneDuplicateFavoriteProviders, backUpProviderToFavorites, t],
  );

  /**
   * Persists a provider's model catalog.
   *
   * ZCode stores models inside `settingsConfig`, so every model edit rewrites
   * that blob and re-projects the provider — the same "DB row alone does not
   * reach ZCode" rule the provider form follows.
   */
  const persistProviderModels = React.useCallback(
    async (provider: ZcodeProvider, nextModels: ZcodeModelRow[]) => {
      const settings = parseZcodeProviderSettings(provider.settingsConfig);
      if (!settings) {
        return;
      }
      const nextSettings: ZcodeSettingsConfig = {
        ...settings,
        models: nextModels,
        defaultModelId: nextModels.find((model) => model.isDefault)?.modelId,
      };
      const settingsConfig = JSON.stringify(nextSettings);
      try {
        const updated = await updateZcodeProvider({
          ...provider,
          settingsConfig,
        });
        await saveZcodeProvider({
          id: updated.id,
          name: updated.name,
          category: updated.category,
          settingsConfig: updated.settingsConfig,
          notes: updated.notes,
        });
        await loadConfig();
      } catch (error) {
        console.error('Failed to save ZCode provider models:', error);
        void message.error(error instanceof Error ? error.message : String(error));
      }
    },
    [loadConfig],
  );

  const handleAddModel = React.useCallback((provider: ZcodeProvider) => {
    setModelModal({ provider, modelIndex: null });
  }, []);

  const handleEditModel = React.useCallback((provider: ZcodeProvider, modelId: string) => {
    const models = parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [];
    const modelIndex = models.findIndex((model) => model.modelId === modelId);
    if (modelIndex >= 0) {
      setModelModal({ provider, modelIndex });
    }
  }, []);

  const handleSubmitModel = React.useCallback(
    async (model: ZcodeModelRow) => {
      if (!modelModal) {
        return;
      }
      const { provider, modelIndex } = modelModal;
      const models = parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [];
      const nextModels =
        modelIndex === null
          ? [
              ...models,
              // The first model becomes the default so a fresh provider is
              // immediately usable.
              { ...model, isDefault: models.length === 0 },
            ]
          : models.map((existing, index) =>
              index === modelIndex ? { ...model, isDefault: existing.isDefault } : existing,
            );
      setModelModal(null);
      await persistProviderModels(provider, nextModels);
    },
    [modelModal, persistProviderModels],
  );

  /**
 * Shows every file ZCode loads, one tab each.
 *
 * ZCode reads three: the provider registry AI Toolbox writes, the CLI's own
 * `config.json`, and the desktop `setting.json` that decides where the rest of
 * them live. Showing only the provider registry reads as though the other two
 * were not part of the picture.
 */
  const handlePreviewCurrentConfig = async () => {
    try {
      setPreviewData(await getZcodePreview());
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview ZCode config:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleCopyModel = React.useCallback(
    (provider: ZcodeProvider, modelId: string) => {
      const models = parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [];
      const source = models.find((model) => model.modelId === modelId);
      if (!source) {
        return;
      }
      setModelModal({
        provider,
        modelIndex: null,
        prefill: {
          ...source,
          displayName: `${source.displayName?.trim() || source.modelId.trim()} copy`,
          isDefault: false,
        },
      });
    },
    [],
  );

  const handleDeleteModel = React.useCallback(
    async (provider: ZcodeProvider, modelId: string) => {
      const models = parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [];
      const nextModels = models.filter((model) => model.modelId !== modelId);
      // Dropping the default model would leave the provider unappliable, so the
      // first remaining row inherits the flag.
      if (!nextModels.some((model) => model.isDefault) && nextModels.length > 0) {
        nextModels[0] = { ...nextModels[0], isDefault: true };
      }
      await persistProviderModels(provider, nextModels);
    },
    [persistProviderModels],
  );

  /**
   * Makes one model the default, both in the provider's own catalog and in
   * ZCode's registry.
   *
   * Two writes, deliberately: `isDefault` is a ZCode catalog flag (which model
   * the provider prefers), while `defaultModelSelection` is the runtime's
   * "start new sessions with this" pointer. Writing only the first would leave
   * the two disagreeing — the card would show a default that ZCode never uses.
   */
  const handleSetPrimaryModel = React.useCallback(
    async (provider: ZcodeProvider, modelId: string) => {
      const models = parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [];
      await persistProviderModels(
        provider,
        models.map((model) => ({ ...model, isDefault: model.modelId === modelId })),
      );
      try {
        await selectZcodeProvider(provider.id, modelId);
        noteProviderUsed(provider.id);
        await refreshTrayMenu();
        await loadConfig();
      } catch (error) {
        console.error('Failed to set ZCode default model:', error);
        void message.error(error instanceof Error ? error.message : String(error));
      }
    },
    [loadConfig, noteProviderUsed, persistProviderModels],
  );

  /**
   * Switches one model on or off in the provider's catalog.
   *
   * ZCode's own dialog puts this switch on the model list row, not in the edit
   * dialog, so that is where it lives here too. An enabled model writes no key
   * at all — only an explicit `false` is stored — so switching back on removes
   * the flag rather than writing `true`.
   */
  const handleToggleModelDisabled = React.useCallback(
    async (provider: ZcodeProvider, modelId: string, isDisabled: boolean) => {
      const models = parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [];
      await persistProviderModels(
        provider,
        models.map((model) =>
          model.modelId === modelId
            ? { ...model, enabled: isDisabled ? false : undefined }
            : model,
        ),
      );
    },
    [persistProviderModels],
  );

  const handleReorderModels = React.useCallback(
    async (provider: ZcodeProvider, orderedModelIds: string[]) => {
      const models = parseZcodeProviderSettings(provider.settingsConfig)?.models ?? [];
      const byId = new Map(models.map((model) => [model.modelId, model]));
      const ordered = orderedModelIds
        .map((id) => byId.get(id))
        .filter((model): model is ZcodeModelRow => Boolean(model));
      // Rows missing from the incoming order keep their previous position.
      for (const model of models) {
        if (!ordered.includes(model)) {
          ordered.push(model);
        }
      }
      await persistProviderModels(provider, ordered);
    },
    [persistProviderModels],
  );

  const batchSelectableIds = React.useMemo(
    () => visibleProviders.map((provider) => provider.id),
    [visibleProviders],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  /**
   * The one ordering that mixes provider cards with the official-account card.
   *
   * `officialAccountIndex` counts providers, so it must be clamped before use:
   * deleting providers can leave it pointing past the end.
   */
  const sortableItemIds = React.useMemo(() => {
    const ids: string[] = providers.map((provider) => provider.id);
    ids.splice(Math.min(officialAccountIndex, ids.length), 0, OFFICIAL_ACCOUNT_CARD_ID);
    return ids;
  }, [providers, officialAccountIndex]);

  /**
   * Where the card lands among the *visible* provider cards.
   *
   * A search filter hides providers the index still counts, so the slot is the
   * number of providers above the card that survived the filter — otherwise a
   * filtered list would push the card to the bottom for no visible reason.
   */
  const officialAccountSlot = React.useMemo(() => {
    const visibleIds = new Set(visibleProviders.map((provider) => provider.id));
    const above = providers
      .slice(0, Math.min(officialAccountIndex, providers.length))
      .filter((provider) => visibleIds.has(provider.id));
    return Math.min(above.length, visibleProviders.length);
  }, [providers, officialAccountIndex, visibleProviders]);

  const officialAccountCard = (
    <ZcodeOfficialAccountCard
      accounts={officialAccounts}
      applyingAccountId={applyingOfficialAccountId}
      savingCurrent={savingOfficialAccount}
      loginPending={officialLoginProviderId !== null}
      loginProviders={ZCODE_LOGIN_PROVIDERS}
      sortableId={OFFICIAL_ACCOUNT_CARD_ID}
      dragDisabled={providerBatchDragDisabled}
      onLogin={(providerId) => void handleStartOfficialLogin(providerId)}
      onApply={(account) => void handleApplyOfficialAccount(account)}
      onDelete={(account) => void handleDeleteOfficialAccount(account)}
      onSaveCurrent={() => void handleSaveOfficialLocalAccount()}
    />
  );

  const handleBatchTestProviders = React.useCallback(async () => {
    setBatchTestingProviders(true);
    try {
      // 单个测试与批量测试共用同一个 builder，避免两处形状漂移。
      const targets = visibleProviders.map((provider) =>
        buildProviderConnectivityBatchTarget(buildZcodeProviderConnectivityInfo(provider), {
          requireBaseUrl: true,
          requireApiKey: true,
          errorMessages: {
            missingBaseUrl: t('zcode.test.missingBaseUrl', {
              name: provider.name,
            }),
            missingApiKey: t('zcode.test.missingApiKey', { name: provider.name }),
            missingModel: t('zcode.test.missingModel', { name: provider.name }),
          },
        }),
      );
      setConnectivityStatuses({});
      await runProviderConnectivityBatch(targets, (providerId, status) => {
        setConnectivityStatuses((previous) => ({ ...previous, [providerId]: status }));
      });
    } finally {
      setBatchTestingProviders(false);
    }
  }, [visibleProviders, t]);

  /** Enters or leaves batch-delete mode for one provider's model list. */
  const handleToggleModelBatchDeleteMode = React.useCallback(
    (provider: ZcodeProvider) => {
      if (modelBatchDeleteProviderId === provider.id) {
        setSelectedModelIdsByProvider({});
        setModelBatchDeleteProviderId(null);
        return;
      }
      setSelectedModelIdsByProvider({});
      setModelBatchDeleteProviderId(provider.id);
    },
    [modelBatchDeleteProviderId],
  );

  const handleToggleModelSelection = React.useCallback(
    (provider: ZcodeProvider, modelId: string, selected: boolean) => {
      setSelectedModelIdsByProvider((previous) => {
        const current = previous[provider.id] ?? [];
        const next = selected
          ? Array.from(new Set([...current, modelId]))
          : current.filter((id) => id !== modelId);
        if (next.length === 0) {
          const nextState = { ...previous };
          delete nextState[provider.id];
          return nextState;
        }
        return { ...previous, [provider.id]: next };
      });
    },
    [],
  );

  const handleBatchDeleteModels = React.useCallback(
    (provider: ZcodeProvider) => {
      const selectedIds = selectedModelIdsByProvider[provider.id] ?? [];
      if (selectedIds.length === 0) {
        return;
      }
      Modal.confirm({
        title: t('common.model.batchDeleteConfirmTitle'),
        content: t('common.model.batchDeleteConfirmContent', { count: selectedIds.length }),
        okText: t('common.confirm'),
        cancelText: t('common.cancel'),
        onOk: async () => {
          const settings = parseZcodeProviderSettings(provider.settingsConfig);
          if (!settings) {
            return;
          }
          const removed = new Set(selectedIds);
          const nextModels = settings.models.filter((model) => !removed.has(model.modelId));
          await persistProviderModels(provider, nextModels);
          setSelectedModelIdsByProvider((previous) => {
            if (!(provider.id in previous)) {
              return previous;
            }
            const nextState = { ...previous };
            delete nextState[provider.id];
            return nextState;
          });
          setModelBatchDeleteProviderId((current) => (current === provider.id ? null : current));
          message.success(t('common.success'));
        },
      });
    },
    [persistProviderModels, selectedModelIdsByProvider, t],
  );

  /** Runs the connectivity probe for one provider's catalog only. */
  /**
   * 单个 provider 的「模型测试」：打开共享的连通性测试弹窗。
   *
   * ⚠️ **不要退回 inline 状态徽标**。共享 `ProviderConnectivityTestModal` 是
   * 其余 10 个页面的标准做法（逐个模型列出结果、可移除失败项），本页此前是
   * 全仓仅有的两个例外之一（另一个是 OmO Native），用户报「模型测试没弹窗」
   * （2026-10-07 修）。卡片头部的**批量**测试仍走 inline 徽标——那与 Codex 一致。
   */
  const handleTestProviderModels = React.useCallback((provider: ZcodeProvider) => {
    setConnectivityInfo(buildZcodeProviderConnectivityInfo(provider));
    setConnectivityModalOpen(true);
  }, []);

  const fetchModelsProvider = React.useMemo(
    () => providers.find((provider) => provider.id === fetchModelsProviderId) ?? null,
    [fetchModelsProviderId, providers],
  );

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!fetchModelsProvider) {
      return null;
    }
    const settings = parseZcodeProviderSettings(fetchModelsProvider.settingsConfig);
    return {
      providerId: fetchModelsProvider.id,
      name: fetchModelsProvider.name,
      baseUrl: settings?.config?.api?.baseUrl ?? '',
      apiKey: settings?.config?.access?.apiKey ?? '',
      existingModelIds: (settings?.models ?? []).map((model) => model.modelId),
    };
  }, [fetchModelsProvider]);

  /**
   * Merges the fetched models into the provider's catalog.
   *
   * Rows are written as `smart` rules and filled from the preset catalog, the
   * same defaults a manually added row gets — a fetched list otherwise lands as
   * bare ids and the user has to look every parameter up by hand. A model the
   * catalog does not know keeps the name the provider's API reported and leaves
   * ZCode's built-in rules to supply the rest.
   *
   * Ids already in the catalog are skipped, so re-fetching never overwrites
   * edits the user made by hand. Rows the modal reports as gone upstream are
   * dropped — the modal only fills that list when the user opts in.
   */
  const handleFetchModelsApply = React.useCallback(
    async (result: FetchModelsApplyResult) => {
      if (!fetchModelsProvider) {
        return;
      }
      const settings = parseZcodeProviderSettings(fetchModelsProvider.settingsConfig);
      if (!settings) {
        return;
      }
      const preferredNpm = preferredPresetNpmTypes(settings.config?.api?.type)[0];
      const removedIds = new Set(result.removedModelIds);
      const keptModels = settings.models.filter((model) => !removedIds.has(model.modelId));
      const existingIds = new Set(keptModels.map((model) => model.modelId));
      const added: ZcodeModelRow[] = result.selectedModels
        .filter((model) => !existingIds.has(model.id))
        .map((model) => {
          const row = buildZcodeModelRowFromPreset(
            model.id,
            'smart',
            findPresetModelById(model.id, preferredNpm),
          );
          // A preset name is more precise than the provider's label; the label
          // is still better than no name at all.
          return { ...row, displayName: row.displayName ?? (model.name || undefined) };
        });
      if (added.length === 0 && keptModels.length === settings.models.length) {
        setFetchModelsModalOpen(false);
        setFetchModelsProviderId(null);
        return;
      }
      await persistProviderModels(fetchModelsProvider, [...keptModels, ...added]);
      setFetchModelsModalOpen(false);
      setFetchModelsProviderId(null);
    },
    [fetchModelsProvider, persistProviderModels],
  );

  return (
    <SectionSidebarLayout
      sidebarTitle={t('zcode.title', { defaultValue: 'ZCode 配置管理' })}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'zcode-providers':
            return <DatabaseOutlined />;
          case 'zcode-global-prompt':
            return <FileTextOutlined />;
          case 'zcode-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'zcode-providers':
            setProviderListCollapsed(false);
            break;
          case 'zcode-global-prompt':
            setPromptExpandNonce((value) => value + 1);
            break;
          case 'zcode-session-manager':
            setSessionManagerExpandNonce((value) => value + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        <CodingPageHeader
          title={t('zcode.title')}
          docsUrl="https://zcode.z.ai/cn/docs/configuration"
          configPath={configPath || '~/.zcode/v2/provider_config.json'}
          onPreviewConfig={() => void handlePreviewCurrentConfig()}
          onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
          onOpenFolder={() => void handleOpenFolder()}
          onRefresh={() => void loadConfig()}
          onMoreOptions={() => setSettingsModalOpen(true)}
        />

        {!hasNewGenerationRegistry && (
          <Alert
            type="warning"
            showIcon
            style={{ marginBottom: 16 }}
            title={t('zcode.legacyGeneration.title', {
              defaultValue: 'ZCode 尚未迁移到新版供应商配置',
            })}
            description={t('zcode.legacyGeneration.description', {
              defaultValue:
                '当前 ZCode 仍读取旧版 config.json。请先启动一次 ZCode 桌面端完成迁移，否则这里的改动不会生效。',
            })}
          />
        )}

        <ProviderListSection
          sectionId="zcode-providers"
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
          locateProviderId={appliedProviderId}
          onBatchTest={handleBatchTestProviders}
          batchTesting={batchTestingProviders}
          onAddProvider={() => {
            setEditingProvider(null);
            setIsCopyMode(false);
            setFormModalOpen(true);
          }}
          hint={
            <>
              <div>{t('zcode.pageHint')}</div>
              <div>{t('zcode.pageWarning')}</div>
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
          alwaysVisible={officialAccountCard}
        >
          <DndContext
            sensors={providerBatchDragDisabled ? [] : undefined}
            collisionDetection={closestCenter}
            modifiers={[restrictToVerticalAxis]}
            onDragEnd={(event) => void handleDragEnd(event)}
          >
            <SortableContext items={sortableItemIds} strategy={verticalListSortingStrategy}>
              <div>
                {visibleProviders.map((provider, providerIndex) => (
                  <React.Fragment key={provider.id}>
                    {/* The card shares this ordering with the provider cards, so
                        it is rendered in place rather than pinned above them. */}
                    {providerIndex === officialAccountSlot && officialAccountCard}
                    <ZcodeProviderCard
                    provider={provider}
                    onEdit={() => {
                      setEditingProvider(provider);
                      setIsCopyMode(false);
                      setFormModalOpen(true);
                    }}
                    onCopy={() => {
                      setEditingProvider(provider);
                      setIsCopyMode(true);
                      setFormModalOpen(true);
                    }}
                    onDelete={() => void handleDeleteProvider(provider)}
                    onToggleDisabled={() =>
                      void handleToggleProviderDisabled(provider, !provider.isDisabled)
                    }
                    selectable={
                      providerBatch.selectionMode && providerBatch.isSelectable(provider.id)
                    }
                    selected={providerBatch.selectedIds.has(provider.id)}
                    onSelectChange={(selected) => providerBatch.toggleSelect(provider.id, selected)}
                    connectivityStatus={connectivityStatuses[provider.id]}
                    onAddModel={() => handleAddModel(provider)}
                    onEditModel={(modelId) => handleEditModel(provider, modelId)}
                    onCopyModel={(modelId) => handleCopyModel(provider, modelId)}
                    onDeleteModel={(modelId) => void handleDeleteModel(provider, modelId)}
                    onSetPrimaryModel={(modelId) => void handleSetPrimaryModel(provider, modelId)}
                    onToggleModelDisabled={(modelId, isDisabled) =>
                      void handleToggleModelDisabled(provider, modelId, isDisabled)
                    }
                    onReorderModels={(orderedModelIds) =>
                      void handleReorderModels(provider, orderedModelIds)
                    }
                    modelSelectionMode={modelBatchDeleteProviderId === provider.id}
                    selectedModelIds={selectedModelIdsByProvider[provider.id] ?? []}
                    onToggleModelSelection={(modelId, selected) =>
                      handleToggleModelSelection(provider, modelId, selected)
                    }
                    onToggleBatchDeleteMode={() => handleToggleModelBatchDeleteMode(provider)}
                    onBatchDeleteModels={() => handleBatchDeleteModels(provider)}
                    onTestModels={() => handleTestProviderModels(provider)}
                    testModelsDisabled={
                      !(parseZcodeProviderSettings(provider.settingsConfig)?.models ?? []).length
                    }
                    testModelsDisabledTooltip={t('common.modelMissing')}
                    onFetchModels={() => {
                      setFetchModelsProviderId(provider.id);
                      setFetchModelsModalOpen(true);
                    }}
                    />
                  </React.Fragment>
                ))}
                {/* Past the last provider, so the card can sit at the bottom. */}
                {officialAccountSlot >= visibleProviders.length && officialAccountCard}
              </div>
            </SortableContext>
          </DndContext>
        </ProviderListSection>

        <div
          id="zcode-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('common.prompt.title')}
        >
          <GlobalPromptSettings
            key={`zcode-prompt-${promptExpandNonce}`}
            toolName="ZCode"
            promptFileName="AGENTS.md"
            service={zcodePromptApi}
            collapseKey="zcode-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <div
          id="zcode-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title', { defaultValue: '会话管理' })}
        >
          <SessionManagerPanel
            tool="zcode"
            expandNonce={sessionManagerExpandNonce}
            refreshNonce={0}
          />
        </div>
      </div>

      {rootDirectoryModalOpen && (
        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(rootPathInfo)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />
      )}

      {formModalOpen && (
        <ZcodeProviderFormModal
          open={formModalOpen}
          provider={editingProvider}
          isCopy={isCopyMode}
          onCancel={() => {
            setFormModalOpen(false);
            setEditingProvider(null);
            setIsCopyMode(false);
          }}
          onSaved={async () => {
            setFormModalOpen(false);
            setEditingProvider(null);
            setIsCopyMode(false);
            await refreshTrayMenu();
            await loadConfig();
          }}
        />
      )}

      {modelModal && (
        <ZcodeModelFormModal
          open
          isEdit={modelModal.modelIndex !== null}
          apiType={
            parseZcodeProviderSettings(modelModal.provider.settingsConfig)?.config?.api?.type
          }
          initialValues={
            modelModal.prefill ??
            (modelModal.modelIndex === null
              ? undefined
              : (parseZcodeProviderSettings(modelModal.provider.settingsConfig)?.models ?? [])[
                  modelModal.modelIndex
                ])
          }
          onCancel={() => setModelModal(null)}
          onSubmit={handleSubmitModel}
        />
      )}

      {fetchModelsProviderInfo && (
        <FetchModelsModal
          open={fetchModelsModalOpen}
          providerId={fetchModelsProviderInfo.providerId}
          providerName={fetchModelsProviderInfo.name}
          baseUrl={fetchModelsProviderInfo.baseUrl}
          apiKey={fetchModelsProviderInfo.apiKey || undefined}
          existingModelIds={fetchModelsProviderInfo.existingModelIds}
          onCancel={() => {
            setFetchModelsModalOpen(false);
            setFetchModelsProviderId(null);
          }}
          onSuccess={(result) => void handleFetchModelsApply(result)}
        />
      )}

      <ProviderConnectivityTestModal
        open={connectivityModalOpen}
        connectivityInfo={connectivityInfo}
        onCancel={() => setConnectivityModalOpen(false)}
      />

      <FileConfigPreviewModal
        open={previewModalOpen}
        onClose={() => setPreviewModalOpen(false)}
        title={t('zcode.preview.title')}
        files={[
          {
            key: 'provider-config',
            label: 'provider_config.json',
            content: previewData?.providerConfig.content,
            language: 'json',
          },
          {
            key: 'cli-config',
            label: 'cli/config.json',
            content: previewData?.cliConfig.content,
            language: 'json',
          },
          {
            key: 'setting',
            label: 'setting.json',
            content: previewData?.setting.content,
            language: 'json',
          },
          // Only present while the runtime still reads the legacy map.
          {
            key: 'legacy-config',
            label: 'config.json',
            content: previewData?.legacyConfig?.content,
            language: 'json',
          },
        ]}
      />

      {/**
       * The login command resolves only when the browser flow finishes, so the
       * modal *is* the pending state: it stays open for the whole wait and
       * cannot be dismissed except by cancelling, which is what tells the
       * backend to stop polling.
       */}
      <Modal
        open={officialLoginProviderId !== null}
        title={t('zcode.officialAccount.loginTitle')}
        closable={false}
        maskClosable={false}
        keyboard={false}
        okText={t('common.cancel')}
        cancelButtonProps={{ style: { display: 'none' } }}
        onOk={() => void handleCancelOfficialLogin()}
      >
        <Space direction="vertical" size="small">
          {/* The spinner sits with the text, not on the cancel button: the
              wait is in the browser, and a spinning "取消" reads as though the
              cancellation itself were still in progress. */}
          <Space size="small">
            <Spin size="small" />
            <Typography.Text>
              {t('zcode.officialAccount.loginPending', {
                provider:
                  ZCODE_LOGIN_PROVIDERS.find(
                    (provider) => provider.value === officialLoginProviderId,
                  )?.label ?? officialLoginProviderId ?? '',
              })}
            </Typography.Text>
          </Space>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {t('zcode.officialAccount.loginPendingHint')}
          </Typography.Text>
        </Space>
      </Modal>

      <ImportProviderModal
        open={importModalOpen}
        onClose={() => setImportModalOpen(false)}
        onImport={(imported) => void handleImportFavoriteProviders(imported)}
        existingProviderIds={providers.map((provider) =>
          buildFavoriteProviderStorageKey('zcode', provider.id),
        )}
        providerFilter={(provider) => isFavoriteProviderForSource('zcode', provider)}
      />

      {allApiHubAvailable && (
        <ZcodeImportFromAllApiHubModal
          open={allApiHubImportModalOpen}
          existingProviderIds={providers.map((provider) => provider.sourceProviderId || provider.id)}
          onCancel={() => setAllApiHubImportModalOpen(false)}
          onImport={(imported) => void handleImportFromAllApiHub(imported)}
        />
      )}

      {ccSwitchAvailable && (
        <ImportFromCcSwitchModal
          open={ccSwitchImportModalOpen}
          // CC Switch only stores Claude-shaped providers; `claude` is the app
          // type those rows live under, and what other Anthropic-family CLIs
          // read here too.
          appType="claude"
          existingProviderIds={providers
            .map((provider) => provider.sourceProviderId)
            .filter((id): id is string => Boolean(id))}
          onClose={() => setCcSwitchImportModalOpen(false)}
          onImport={(imported) => void handleImportFromCcSwitch(imported)}
        />
      )}

      <SidebarSettingsModal
        open={settingsModalOpen}
        onClose={() => setSettingsModalOpen(false)}
        sidebarVisible={!sidebarHidden}
        onSidebarVisibleChange={(visible) => setSidebarHidden('zcode', !visible)}
      />
    </SectionSidebarLayout>
  );
};

export default ZcodePage;
