import React from 'react';
import { Typography, Button, Space, Empty, message, Modal, Spin, Collapse, Descriptions } from 'antd';
import { PlusOutlined, FolderOpenOutlined, AppstoreOutlined, SyncOutlined, EyeOutlined, ExclamationCircleOutlined, LinkOutlined, EllipsisOutlined, DatabaseOutlined, ImportOutlined, FileTextOutlined, ThunderboltOutlined, EditOutlined, CopyOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
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
  CodexProvider,
  CodexOfficialAccount,
  CodexProviderFormValues,
  CodexProviderInput,
  ConfigPathInfo,
  CodexSettings,
  CodexSettingsConfig,
  ImportConflictInfo,
  ImportConflictAction,
} from '@/types/codex';
import {
  getCodexConfigFilePath,
  getCodexRootPathInfo,
  getCodexCommonConfig,
  listCodexProviders,
  listCodexOfficialAccounts,
  startCodexOfficialAccountOauth,
  saveCodexOfficialLocalAccount,
  applyCodexOfficialAccount,
  deleteCodexOfficialAccount,
  refreshCodexOfficialAccountLimits,
  copyCodexOfficialAccountToken,
  selectCodexProvider,
  readCodexSettings,
  createCodexProvider,
  updateCodexProvider,
  saveCodexLocalConfig,
  saveCodexCommonConfig,
  deleteCodexProvider,
  toggleCodexProviderDisabled,
  reorderCodexProviders,
} from '@/services/codexApi';
import { codexPromptApi } from '@/services/codexPromptApi';
import { refreshTrayMenu, hasAllApiHubExtension } from '@/services/appApi';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import { useSettingsStore } from '@/stores';
import CodexProviderCard from '../components/CodexProviderCard';
import CodexProviderFormModal from '../components/CodexProviderFormModal';
import CodexCommonConfigModal from '../components/CodexCommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import CodexPluginsPanel from '../components/CodexPluginsPanel';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import CodexConfigPreviewModal from '@/components/common/CodexConfigPreviewModal';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import ProviderConnectivityTestModal, {
  buildCodexProviderConnectivityInfo,
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
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
  type CodexFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import { extractCodexBaseUrl, extractCodexModel } from '@/utils/codexConfigUtils';

const { Title, Text, Link } = Typography;

function parseCodexSettingsConfig(rawConfig: string): CodexSettingsConfig {
  try {
    return JSON.parse(rawConfig) as CodexSettingsConfig;
  } catch (error) {
    console.error('Failed to parse Codex settings config:', error);
    return {};
  }
}

function buildCodexFavoriteProviderConfig(provider: CodexProvider) {
  const settingsConfig = parseCodexSettingsConfig(provider.settingsConfig);
  const baseUrl = extractCodexBaseUrl(settingsConfig.config)?.trim();
  const modelId = extractCodexModel(settingsConfig.config)?.trim();

  return buildFavoriteProviderOptions(
    {
      npm: '@ai-sdk/openai',
      name: provider.name,
      options: {
        ...(baseUrl ? { baseURL: baseUrl } : {}),
        ...(settingsConfig.auth?.OPENAI_API_KEY?.trim()
          ? { apiKey: settingsConfig.auth.OPENAI_API_KEY.trim() }
          : {}),
      },
      models: Object.fromEntries(modelId ? [[modelId, {}]] : []),
    },
    {
      name: provider.name,
      category: provider.category,
      settingsConfig: provider.settingsConfig,
      ...(provider.notes ? { notes: provider.notes } : {}),
    } satisfies CodexFavoriteProviderPayload,
  );
}

const ACCOUNT_DETAILS_EMPTY_VALUE = '-';

function maskTokenPreview(tokenKind: 'access' | 'refresh', account: CodexOfficialAccount): string {
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

const CodexPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const {
    sidebarHiddenByPage,
    setSidebarHidden,
  } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [providers, setProviders] = React.useState<CodexProvider[]>([]);
  const [officialAccountsByProviderId, setOfficialAccountsByProviderId] = React.useState<
    Record<string, CodexOfficialAccount[]>
  >({});
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');
  const [refreshingOfficialAccountId, setRefreshingOfficialAccountId] = React.useState<string | null>(null);
  const [savingOfficialAccountId, setSavingOfficialAccountId] = React.useState<string | null>(null);
  const [officialAccountDetails, setOfficialAccountDetails] = React.useState<{
    provider: CodexProvider;
    account: CodexOfficialAccount;
  } | null>(null);

  // Modal states
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<CodexProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [providerModalMode, setProviderModalMode] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<CodexProviderFormValues | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<CodexSettings | null>(null);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityInfo, setConnectivityInfo] = React.useState<ProviderConnectivityInfo | null>(null);
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
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const sidebarHidden = sidebarHiddenByPage.codex;

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
      id: 'codex-providers',
      title: t('codex.provider.title'),
      order: 1,
    },
    {
      id: 'codex-global-prompt',
      title: t('codex.prompt.title'),
      order: 2,
    },
    {
      id: 'codex-plugins',
      title: t('codex.plugins.title'),
      order: 3,
    },
    {
      id: 'codex-session-manager',
      title: t('sessionManager.title'),
      order: 4,
    },
  ], [t]);

  const loadConfig = React.useCallback(async (silent = false) => {
    setLoading(true);
    try {
      const [path, nextRootPathInfo, providerList] = await Promise.all([
        getCodexConfigFilePath(),
        getCodexRootPathInfo(),
        listCodexProviders(),
      ]);
      setConfigPath(path);
      setRootPathInfo(nextRootPathInfo);
      setProviders(providerList);
      const officialAccountEntries = await Promise.all(
        providerList.map(async (provider) => [
          provider.id,
          await listCodexOfficialAccounts(provider.id),
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
      const codexFavoriteProviders = allFavoriteProviders.filter((provider) =>
        isFavoriteProviderForSource('codex', provider),
      );
      const currentStorageKeys = new Set(
        providers.map((provider) => buildFavoriteProviderStorageKey('codex', provider.id)),
      );
      const { keptProviders, duplicateIds } = dedupeFavoriteProvidersByPayload(
        codexFavoriteProviders,
        currentStorageKeys,
      );

      if (duplicateIds.length > 0) {
        await Promise.all(
          duplicateIds.map(async (providerId) => {
            try {
              await deleteFavoriteProvider(providerId);
            } catch (error) {
              console.error('Failed to delete duplicate Codex favorite provider:', error);
            }
          }),
        );
      }

      setFavoriteProviders(keptProviders);
    } catch (error) {
      console.error('Failed to load Codex favorite providers:', error);
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
    translationKeyPrefix: 'codex',
    defaultConfig: '',
    loadConfig,
    getCommonConfig: getCodexCommonConfig,
    saveCommonConfig: saveCodexCommonConfig,
  });

  const handleSelectProvider = async (provider: CodexProvider) => {
    try {
      await selectCodexProvider(provider.id);
      message.success(t('codex.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to select provider:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleToggleDisabled = async (provider: CodexProvider, isDisabled: boolean) => {
    try {
      await toggleCodexProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('codex.providerDisabled') : t('codex.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle provider disabled status:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleStartOfficialAccountOauth = async (provider: CodexProvider) => {
    try {
      await startCodexOfficialAccountOauth(provider.id);
      message.success(t('codex.provider.officialAccountOauthSuccess'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to start Codex official account OAuth:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleApplyOfficialAccount = async (
    provider: CodexProvider,
    account: CodexOfficialAccount,
  ) => {
    try {
      await applyCodexOfficialAccount(provider.id, account.id);
      message.success(t('codex.apply.success'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to apply Codex official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleSaveOfficialLocalAccount = async (
    provider: CodexProvider,
    account: CodexOfficialAccount,
  ) => {
    try {
      setSavingOfficialAccountId(account.id);
      await saveCodexOfficialLocalAccount(provider.id);
      if (officialAccountDetails?.account.id === account.id) {
        setOfficialAccountDetails(null);
      }
      message.success(t('codex.provider.officialAccountSaveSuccess'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Codex local official account:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setSavingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleDeleteOfficialAccount = async (
    provider: CodexProvider,
    account: CodexOfficialAccount,
  ) => {
    Modal.confirm({
      title: t('codex.provider.officialAccountDeleteConfirm', {
        name: account.email || account.name,
      }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await deleteCodexOfficialAccount(provider.id, account.id);
          message.success(t('common.success'));
          await loadConfig();
          await refreshTrayMenu();
        } catch (error) {
          console.error('Failed to delete Codex official account:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        }
      },
    });
  };

  const handleRefreshOfficialAccount = async (
    provider: CodexProvider,
    account: CodexOfficialAccount,
  ) => {
    try {
      setRefreshingOfficialAccountId(account.id);
      const refreshedAccount = await refreshCodexOfficialAccountLimits(provider.id, account.id);
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
      message.success(t('codex.provider.officialAccountRefreshSuccess'));
    } catch (error) {
      console.error('Failed to refresh Codex official account usage:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setRefreshingOfficialAccountId((current) => (current === account.id ? null : current));
    }
  };

  const handleViewOfficialAccountDetails = (
    provider: CodexProvider,
    account: CodexOfficialAccount,
  ) => {
    setOfficialAccountDetails({ provider, account });
  };

  const handleCopyOfficialAccountToken = React.useCallback(async (
    provider: CodexProvider,
    account: CodexOfficialAccount,
    tokenKind: 'access' | 'refresh',
  ) => {
    try {
      await copyCodexOfficialAccountToken(provider.id, account.id, tokenKind);
      message.success(t('common.copied'));
    } catch (error) {
      console.error('Failed to copy official account token:', error);
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
      await reorderCodexProviders(newProviders.map((p) => p.id));
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

  const handleEditProvider = (provider: CodexProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalMode('manual');
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: CodexProvider) => {
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

  const handleTestProvider = (provider: CodexProvider) => {
    if (provider.category === 'official') {
      message.info(t('codex.provider.officialConnectivityHint'));
      return;
    }

    setConnectivityInfo(buildCodexProviderConnectivityInfo(provider));
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
        buildFavoriteProviderStorageKey('codex', targetProvider.id),
        buildCodexFavoriteProviderConfig(targetProvider),
        diagnostics,
      );
      setFavoriteProviders((previousProviders) =>
        mergeDiagnosticsIntoFavoriteProviders(previousProviders, favoriteProvider, 'codex'),
      );
    } catch (error) {
      console.error('Failed to save Codex connectivity diagnostics:', error);
      message.error(t('common.error'));
    }
  }, [connectivityInfo, providers, t]);

  const handleBatchTestProviders = React.useCallback(async () => {
    if (providers.length === 0) {
      return;
    }

    const officialProviders = providers.filter((provider) => provider.category === 'official');
    const testableProviders = providers.filter((provider) => provider.category !== 'official');

    if (officialProviders.length > 0) {
      message.info(t('codex.provider.officialBatchSkipped', { count: officialProviders.length }));
    }

    if (testableProviders.length === 0) {
      setConnectivityStatuses({});
      return;
    }

    const targets = testableProviders.map((provider) => {
      const connectivityInfo = buildCodexProviderConnectivityInfo(provider);
      let settingsConfig: {
        config?: string;
      } = {};
      try {
        settingsConfig = JSON.parse(provider.settingsConfig || '{}') as typeof settingsConfig;
      } catch (error) {
        console.error('Failed to parse Codex provider settings config for batch test:', error);
      }
      const hasExplicitBaseUrl = Boolean(
        settingsConfig.config?.match(/^\s*base_url\s*=\s*['"]/m),
      );

      if (provider.category !== 'official' && !hasExplicitBaseUrl) {
        return {
          providerId: provider.id,
          errorMessage: t('common.baseUrlMissing'),
        };
      }

      return buildProviderConnectivityBatchTarget(connectivityInfo, {
        requireBaseUrl: false,
        requireApiKey: true,
        preferredModelId: findDefaultTestModelIdForProvider(favoriteProviders, 'codex', provider.id),
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
      console.error('Failed to batch test Codex providers:', error);
      message.error(t('common.error'));
    } finally {
      setBatchTestingProviders(false);
    }
  }, [providers, t, favoriteProviders]);

  const handleDeleteProvider = (provider: CodexProvider) => {
    const performDelete = async () => {
      try {
        await deleteCodexProvider(provider.id);
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
      title: t('codex.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('codex', provider.id),
            buildCodexFavoriteProviderConfig(provider),
          );
          await performDelete();
        } catch (favoriteError) {
          console.error('Failed to preserve Codex favorite provider before deletion:', favoriteError);
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

  const handleProviderSubmit = async (values: CodexProviderFormValues) => {
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
        const configLines = [
          'model_provider = "custom"',
          'model_reasoning_effort = "high"',
          '',
          '[model_providers.custom]',
          'name = "OpenAI"',
          'wire_api = "responses"',
          'requires_openai_auth = true',
        ];

        if (baseUrl) {
          configLines.push(`base_url = "${baseUrl}"`);
        }

        const providerInput: CodexProviderInput = {
          name: item.name,
          category: 'custom',
          settingsConfig: JSON.stringify({
            auth: apiKey ? { OPENAI_API_KEY: apiKey } : {},
            config: configLines.join('\n'),
          }),
          sourceProviderId: item.providerId,
          notes: undefined,
        };

        const createdProvider = await createCodexProvider(providerInput);
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('codex', createdProvider.id),
            buildCodexFavoriteProviderConfig(createdProvider),
          );
        } catch (favoriteError) {
          console.error('Failed to save Codex favorite provider from All API Hub import:', favoriteError);
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
        const payload = getFavoriteProviderPayload<CodexFavoriteProviderPayload>(favoriteProvider);
        if (!payload) {
          continue;
        }

        const createdProvider = await createCodexProvider({
          name: payload.name,
          category: payload.category as CodexProviderInput['category'],
          settingsConfig: payload.settingsConfig,
          notes: payload.notes,
        });
        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('codex', createdProvider.id),
            buildCodexFavoriteProviderConfig(createdProvider),
            favoriteProvider.diagnostics,
          );
        } catch (favoriteError) {
          console.error('Failed to copy Codex favorite provider diagnostics during import:', favoriteError);
        }
        importedCount += 1;
      }

      setImportModalOpen(false);
      message.success(t('opencode.provider.importSuccess', { count: importedCount }));
      await loadConfig();
      await loadFavoriteProviders();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import Codex favorite providers:', error);
      message.error(t('common.error'));
    }
  }, [loadConfig, loadFavoriteProviders, t]);

  const doSaveProvider = async (values: CodexProviderFormValues) => {
    try {
      // 新架构：直接使用 settingsConfig（由 Hook 构建）
      // 旧架构：手动构建（向后兼容）
      let settingsConfig: string;
      if (values.settingsConfig) {
        settingsConfig = values.settingsConfig;
      } else {
        const settingsConfigObj: CodexSettingsConfig =
          values.category === 'official'
            ? {
                auth: {},
                config: values.configToml || '',
              }
            : {
                auth: values.apiKey ? { OPENAI_API_KEY: values.apiKey } : {},
              };

        if (values.category !== 'official') {
          const configParts: string[] = [];
          if (values.baseUrl) {
            configParts.push(`base_url = "${values.baseUrl}"`);
          }
          if (values.model) {
            configParts.push(`[chat]\nmodel = "${values.model}"`);
          }
          if (configParts.length > 0) {
            settingsConfigObj.config = configParts.join('\n');
          }
          if (values.configToml) {
            settingsConfigObj.config = (settingsConfigObj.config || '') + '\n' + values.configToml;
          }
        }

        settingsConfig = JSON.stringify(settingsConfigObj);
      }

      // Check if this is a temporary provider from local files
      const isLocalTemp = editingProvider?.id === "__local__";

      const providerInput: CodexProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig,
        sourceProviderId: values.sourceProviderId,
        notes: values.notes,
      };

      let savedProviderId = isLocalTemp ? '__local__' : '';
      let savedProvider: CodexProvider | null = null;

      if (isLocalTemp) {
        await saveCodexLocalConfig({ provider: providerInput });
      } else if (editingProvider && !isCopyMode) {
        savedProvider = await updateCodexProvider({
          id: editingProvider.id,
          name: values.name,
          category: values.category,
          settingsConfig: providerInput.settingsConfig,
          sourceProviderId: values.sourceProviderId,
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
        savedProvider = await createCodexProvider(providerInput);
        savedProviderId = savedProvider.id;
      }

      try {
        const providerForFavorite: CodexProvider = savedProvider || {
          id: savedProviderId,
          name: values.name,
          category: values.category,
          settingsConfig,
          notes: values.notes,
          isApplied: false,
          isDisabled: false,
          createdAt: '',
          updatedAt: '',
        };
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('codex', providerForFavorite.id),
          buildCodexFavoriteProviderConfig(providerForFavorite),
        );
        await loadFavoriteProviders();
      } catch (error) {
        console.error('Failed to save Codex favorite provider:', error);
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

  const doUpdateProvider = async (id: string, values: CodexProviderFormValues) => {
    try {
      const existingProvider = providers.find((p) => p.id === id);
      if (!existingProvider) return;

      // 新架构：直接使用 settingsConfig（由 Hook 构建）
      // 旧架构：手动构建（向后兼容）
      let settingsConfig: string;
      if (values.settingsConfig) {
        settingsConfig = values.settingsConfig;
      } else {
        const settingsConfigObj: CodexSettingsConfig =
          values.category === 'official'
            ? {
                auth: {},
                config: values.configToml || '',
              }
            : {
                auth: values.apiKey ? { OPENAI_API_KEY: values.apiKey } : {},
              };

        if (values.category !== 'official') {
          const configParts: string[] = [];
          if (values.baseUrl) {
            configParts.push(`base_url = "${values.baseUrl}"`);
          }
          if (values.model) {
            configParts.push(`[chat]\nmodel = "${values.model}"`);
          }
          if (configParts.length > 0) {
            settingsConfigObj.config = configParts.join('\n');
          }
          if (values.configToml) {
            settingsConfigObj.config = (settingsConfigObj.config || '') + '\n' + values.configToml;
          }
        }

        settingsConfig = JSON.stringify(settingsConfigObj);
      }

      const providerData: CodexProvider = {
        ...existingProvider,
        name: values.name,
        category: values.category,
        settingsConfig,
        notes: values.notes,
        isDisabled: existingProvider.isDisabled,
        createdAt: existingProvider.createdAt,
        updatedAt: existingProvider.updatedAt,
      };

      await updateCodexProvider(providerData);
      try {
        await upsertFavoriteProvider(
          buildFavoriteProviderStorageKey('codex', existingProvider.id),
          buildCodexFavoriteProviderConfig(providerData),
        );
        await loadFavoriteProviders();
      } catch (error) {
        console.error('Failed to update Codex favorite provider:', error);
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
      const settings = await readCodexSettings();
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
      sidebarTitle={t('codex.title')}
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'codex-providers':
            return <DatabaseOutlined />;
          case 'codex-global-prompt':
            return <FileTextOutlined />;
          case 'codex-plugins':
            return <AppstoreOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'codex-providers':
            setProviderListCollapsed(false);
            break;
          case 'codex-global-prompt':
            setPromptExpandNonce((v) => v + 1);
            break;
          case 'codex-plugins':
            setPluginListCollapsed(false);
            break;
          case 'codex-session-manager':
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
                  {t('codex.title')}
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openUrl('https://developers.openai.com/codex/config-basic');
                  }}
                >
                  <LinkOutlined /> {t('codex.viewDocs')}
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
                  {t('codex.configPath')}:
                </Text>
                <Text code style={{ fontSize: 12 }}>
                  {configPath || '~/.codex/config.toml'}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => setRootDirectoryModalOpen(true)}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('codex.rootPathSource.customize')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenFolder}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('codex.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<SyncOutlined />}
                  onClick={handleRefreshPage}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('codex.refreshConfig')}
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
          id="codex-providers"
          data-sidebar-section="true"
          data-sidebar-title={t('codex.provider.title')}
        >
          <Collapse
            style={{ marginBottom: 16 }}
            activeKey={providerListCollapsed ? [] : ['providers']}
            onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
            items={[
              {
                key: 'providers',
                label: (
                  <Text strong>
                    <DatabaseOutlined style={{ marginRight: 8 }} />
                    {t('codex.provider.title')}
                  </Text>
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
                      {t('codex.commonConfigButton')}
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
                      {t('codex.addProvider')}
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
                      <div>{t('codex.pageHint')}</div>
                      <div>{t('codex.pageWarning')}</div>
                    </div>

                    {providers.length === 0 ? (
                      <Empty description={t('codex.emptyText')} style={{ marginTop: 40 }} />
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
                              <CodexProviderCard
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
                          {t('codex.importFromOpenCode')}
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
          id="codex-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('codex.prompt.title')}
        >
          <GlobalPromptSettings
            key={`codex-prompt-${promptExpandNonce}`}
            translationKeyPrefix="codex.prompt"
            service={codexPromptApi}
            collapseKey="codex-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <div
          id="codex-plugins"
          data-sidebar-section="true"
          data-sidebar-title={t('codex.plugins.title')}
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
                    {t('codex.plugins.title')}
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
                  <CodexPluginsPanel refreshToken={pluginPanelRefreshToken} />
                ),
              },
            ]}
          />
        </div>

        <div
          id="codex-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title')}
        >
          <SessionManagerPanel tool="codex" expandNonce={sessionManagerExpandNonce} />
        </div>

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
          diagnostics={connectivityInfo ? findDiagnosticsForProvider(favoriteProviders, 'codex', connectivityInfo.providerId) : undefined}
          onSaveDiagnostics={handleSaveConnectivityDiagnostics}
          onCancel={() => setConnectivityModalOpen(false)}
        />

        <ImportProviderModal
          open={importModalOpen}
          onClose={() => setImportModalOpen(false)}
          onImport={handleImportFavoriteProviders}
          existingProviderIds={providers.map((provider) => buildFavoriteProviderStorageKey('codex', provider.id))}
          providerFilter={(provider) => isFavoriteProviderForSource('codex', provider)}
        />

        {/* Modals */}
        {providerModalOpen && (
          <CodexProviderFormModal
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

        <CodexCommonConfigModal
          open={commonConfigModalOpen}
          onCancel={() => setCommonConfigModalOpen(false)}
          onSuccess={() => {
            setCommonConfigModalOpen(false);
          }}
          isLocalProvider={providers.some((provider) => provider.id === '__local__')}
        />

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(rootPathInfo)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
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
        <CodexConfigPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('codex.preview.currentConfigTitle')}
          data={previewData}
        />

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={(visible) => setSidebarHidden('codex', !visible)}
        />
        <Modal
          open={Boolean(officialAccountDetails)}
          title={t('codex.provider.officialAccountDetailsTitle')}
          onCancel={() => setOfficialAccountDetails(null)}
          footer={null}
          width={640}
        >
          {officialAccountDetails && (
            <Descriptions
              column={1}
              size="small"
              labelStyle={{ width: 180 }}
              items={[
                {
                  key: 'provider',
                  label: t('codex.provider.name'),
                  children: officialAccountDetails.provider.name,
                },
                {
                  key: 'account',
                  label: t('codex.provider.officialAccountLabel'),
                  children: officialAccountDetails.account.email || officialAccountDetails.account.name,
                },
                {
                  key: 'type',
                  label: t('codex.provider.mode'),
                  children: officialAccountDetails.account.id === '__local__'
                    ? t('codex.provider.officialAccountLocalTag')
                    : t('codex.provider.officialAccountOauthTag'),
                },
                {
                  key: 'plan',
                  label: t('codex.provider.officialAccountPlanType'),
                  children: officialAccountDetails.account.planType || ACCOUNT_DETAILS_EMPTY_VALUE,
                },
                {
                  key: 'usage5h',
                  label: t('codex.provider.officialAccountShortWindowUsage', {
                    label: officialAccountDetails.account.limitShortLabel || '5h',
                  }),
                  children: officialAccountDetails.account.limit5hText || ACCOUNT_DETAILS_EMPTY_VALUE,
                },
                {
                  key: 'usageWeek',
                  label: t('codex.provider.officialAccountWeeklyLimit'),
                  children: officialAccountDetails.account.limitWeeklyText || ACCOUNT_DETAILS_EMPTY_VALUE,
                },
                {
                  key: 'reset5h',
                  label: t('codex.provider.officialAccountShortWindowResetAt'),
                  children: formatUnixTimestamp(officialAccountDetails.account.limit5hResetAt),
                },
                {
                  key: 'resetWeek',
                  label: t('codex.provider.officialAccountWeeklyResetAt'),
                  children: formatUnixTimestamp(officialAccountDetails.account.limitWeeklyResetAt),
                },
                {
                  key: 'lastFetched',
                  label: t('codex.provider.officialAccountLastLimitRefreshAt'),
                  children: formatDateTime(officialAccountDetails.account.lastLimitsFetchedAt),
                },
                {
                  key: 'tokenExpiresAt',
                  label: t('codex.provider.officialAccountTokenExpiresAt'),
                  children: formatUnixTimestamp(officialAccountDetails.account.tokenExpiresAt),
                },
                {
                  key: 'accessToken',
                  label: t('codex.provider.officialAccountAccessToken'),
                  children: renderTokenPreview(
                    maskTokenPreview('access', officialAccountDetails.account),
                    async () => handleCopyOfficialAccountToken(
                      officialAccountDetails.provider,
                      officialAccountDetails.account,
                      'access',
                    ),
                  ),
                },
                {
                  key: 'refreshToken',
                  label: t('codex.provider.officialAccountRefreshToken'),
                  children: renderTokenPreview(
                    maskTokenPreview('refresh', officialAccountDetails.account),
                    async () => handleCopyOfficialAccountToken(
                      officialAccountDetails.provider,
                      officialAccountDetails.account,
                      'refresh',
                    ),
                  ),
                },
                {
                  key: 'lastError',
                  label: t('codex.provider.officialAccountLastErrorLabel'),
                  children: officialAccountDetails.account.lastError || ACCOUNT_DETAILS_EMPTY_VALUE,
                },
              ]}
            />
          )}
        </Modal>
      </div>
    </SectionSidebarLayout>
  );
};

export default CodexPage;
