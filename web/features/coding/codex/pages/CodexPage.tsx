import React from 'react';
import { Typography, Button, Space, Empty, message, Modal, Spin, Collapse, Descriptions, Checkbox, Drawer, Input } from 'antd';
import { PlusOutlined, FolderOpenOutlined, AppstoreOutlined, SyncOutlined, EyeOutlined, ExclamationCircleOutlined, LinkOutlined, EllipsisOutlined, DatabaseOutlined, ImportOutlined, FileTextOutlined, ThunderboltOutlined, EditOutlined, CopyOutlined, MessageOutlined, BulbOutlined, CheckSquareOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
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
  CodexCatalogModel,
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
  startCodexOfficialAccountDeviceAuth,
  type CodexDeviceAuthStartResult,
  saveCodexOfficialLocalAccount,
  importCodexOfficialAccountFromAuthJson,
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
  setCodexUnifiedSessionHistory,
  hasCodexUnifiedHistoryBackup,
  restoreCodexUnifiedSessionHistory,
} from '@/services/codexApi';
import { codexPromptApi } from '@/services/codexPromptApi';
import CodexDeviceAuthModal from '../components/CodexDeviceAuthModal';
import { refreshTrayMenu, hasAllApiHubExtension } from '@/services/appApi';
import { formatQuotaResetRemainingText } from '../utils/codexQuotaDisplay';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import { TRAY_CONFIG_REFRESH_EVENT, DEEP_LINK_IMPORT_COMPLETED } from '@/constants/configEvents';
import { useSettingsStore } from '@/stores';
import CodexProviderCard from '../components/CodexProviderCard';
import CodexProviderFormModal from '../components/CodexProviderFormModal';
import CodexModelFormModal from '../components/CodexModelFormModal';
import CodexCommonConfigModal from '../components/CodexCommonConfigModal';
import ImportConflictDialog from '../components/ImportConflictDialog';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import CodexPluginsPanel from '../components/CodexPluginsPanel';
import CodexMemoriesPanel from '../components/CodexMemoriesPanel/CodexMemoriesPanel';
import CodexHistorySyncModal from '../components/CodexHistorySyncModal';
import GatewayAggregateSettings from '@/features/coding/gateway/components/GatewayAggregateSettings';
import {
  CODEX_LOCAL_PROVIDER_ID,
  isCodexLocalProviderId,
  shouldLoadCodexOfficialAccounts,
} from '../utils/localProvider';
import {
  codexProviderNeedsGatewayProxy,
  primaryCodexProviderNeedsGatewayProxy,
} from '../utils/codexGatewayProxyNeed';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import CodexConfigPreviewModal from '@/components/common/CodexConfigPreviewModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchModelsApplyResult } from '@/components/common/FetchModelsModal/types';
import { findPresetModelById } from '@/constants/presetModels';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import ShareProviderModal from '@/features/coding/shared/providerShare';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import SidebarSettingsModal, {
  SettingsToggleRow,
} from '@/components/common/SidebarSettingsModal';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import {
  areGatewayProviderProfilesInitialized,
  GatewayAggregateButton,
  GatewayFailoverButton,
  getGatewayProviderProfilesVersion,
  isGatewayAggregateMode,
  isGatewayProxyMode,
  resolveGatewayReengageMode,
  saveProviderWithGatewayReengage,
  subscribeGatewayProviderProfiles,
  toGatewayAggregateReengageConfig,
} from '@/features/coding/shared/gateway';
import ProviderConnectivityTestModal, {
  buildCodexProviderConnectivityInfo,
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import { SessionManagerPanel, type SessionSourceMode } from '@/features/coding/shared/sessionManager';
import {
  PROVIDER_SORT_MODES,
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
  type CodexFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import { extractCodexBaseUrl, extractCodexModel } from '@/utils/codexConfigUtils';
import {
  buildCodexSettingsConfig,
  parseCodexSettingsConfig,
  resolveCodexAutoReviewModelOverride,
} from '../utils/codexSettingsConfig';
import {
  codexCatalogRowKey,
  findCodexCatalogRowIndex,
  importModelsIntoCatalog,
  removeCodexCatalogModels,
  reorderCodexCatalogModels,
  resolveCodexDefaultReasoningEffort,
  upsertCodexCatalogModel,
  type CodexCatalogPresetResolver,
} from '../utils/codexCatalogModels';
import { saveCodexProviderCatalogWithGatewayReengage } from '../utils/codexProviderCatalogSave';
import {
  engageProxyGatewayAggregate,
  engageProxyGatewayFailover,
  engageProxyGatewaySingle,
  getProxyGatewayCliStatus,
  getProxyGatewayStatus,
  restoreProxyGatewayCliDirect,
  type GatewayCliTakeoverStatus,
} from '@/services';

const { Title, Text, Link } = Typography;

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
      ...(provider.meta ? { meta: provider.meta } : {}),
      ...(provider.notes ? { notes: provider.notes } : {}),
    } satisfies CodexFavoriteProviderPayload,
  );
}

/** Catalog rows persisted on a Codex provider, ignoring blank entries. */
function getCodexProviderCatalogModels(provider: CodexProvider): CodexCatalogModel[] {
  const settingsConfig = parseCodexSettingsConfig(provider.settingsConfig);
  const rawModels = settingsConfig.modelCatalog?.models;
  if (!Array.isArray(rawModels)) {
    return [];
  }
  return rawModels.filter((item) => Boolean(item?.model?.trim()));
}

/** Discovery SDK for a saved provider, derived from its stored API format. */
function getCodexProviderSdkType(provider: CodexProvider): string {
  const apiFormat = typeof provider.meta?.apiFormat === 'string' ? provider.meta.apiFormat : undefined;
  if (apiFormat === 'anthropic_messages') {
    return '@ai-sdk/anthropic';
  }
  if (apiFormat === 'gemini_native') {
    return '@ai-sdk/google';
  }
  return '@ai-sdk/openai';
}

const ACCOUNT_DETAILS_EMPTY_VALUE = '-';
let rememberedCodexSessionSourceMode: SessionSourceMode = 'all';

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
    codexPreserveOfficialAuthOnSwitch,
    codexUnifiedSessionHistoryEnabled,
    setCodexPreserveOfficialAuthOnSwitch,
    setCodexUnifiedSessionHistoryEnabled,
  } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [configPath, setConfigPath] = React.useState<string>('');
  const [rootPathInfo, setRootPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [providers, setProviders] = React.useState<CodexProvider[]>([]);
  const [officialAccountsByProviderId, setOfficialAccountsByProviderId] = React.useState<
    Record<string, CodexOfficialAccount[]>
  >({});
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');
  const [gatewayCliStatus, setGatewayCliStatus] = React.useState<GatewayCliTakeoverStatus | null>(null);
  const [aggregateSettingsOpen, setAggregateSettingsOpen] = React.useState(false);
  const [aggregateSettingsOpening, setAggregateSettingsOpening] = React.useState(false);
  const [aggregateGatewayRunning, setAggregateGatewayRunning] = React.useState(false);
  const aggregateSettingsRequestRef = React.useRef(0);
  const gatewayStatusRequestRef = React.useRef(0);
  const gatewayStatusRevisionRef = React.useRef(0);
  // `can_restore_direct` only describes whether the original files can be
  // restored.  A manifest can still be actively routing through the gateway
  // after its backups are unavailable, so the lock follows the explicit
  // gateway proxy mode instead.
  const gatewayTakeoverActive = isGatewayProxyMode(gatewayCliStatus?.mode);
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  // Shared with the provider card and the aggregate settings panel so all three
  // answer "must this provider keep the takeover?" the same way.
  const {
    needsProxy: primaryGatewayProviderNeedsProxy,
    reason: primaryGatewayProviderNeedsProxyReason,
  } = React.useMemo(
    () =>
      primaryCodexProviderNeedsGatewayProxy(
        providers,
        gatewayCliStatus?.primary_provider_id,
        isCodexLocalProviderId,
      ),
    [gatewayCliStatus?.primary_provider_id, gatewayProviderProfilesVersion, providers],
  );

  const applyGatewayCliStatus = React.useCallback((nextStatus: GatewayCliTakeoverStatus) => {
    gatewayStatusRevisionRef.current += 1;
    setGatewayCliStatus(nextStatus);
  }, []);

  const refreshGatewayCliStatus = React.useCallback(async () => {
    const request = gatewayStatusRequestRef.current + 1;
    gatewayStatusRequestRef.current = request;
    const revision = gatewayStatusRevisionRef.current;
    const nextStatus = await getProxyGatewayCliStatus('codex');
    if (
      isActive
      && gatewayStatusRequestRef.current === request
      && gatewayStatusRevisionRef.current === revision
    ) {
      applyGatewayCliStatus(nextStatus);
    }
    return nextStatus;
  }, [applyGatewayCliStatus, isActive]);

  const handleOpenAggregateSettings = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();
      if (aggregateSettingsOpening) {
        return;
      }
      const request = ++aggregateSettingsRequestRef.current;
      setAggregateSettingsOpening(true);
      void getProxyGatewayStatus()
        .then((status) => {
          if (aggregateSettingsRequestRef.current !== request) {
            return;
          }
          setAggregateGatewayRunning(status.running);
          setAggregateSettingsOpen(true);
        })
        .catch((error) => {
          if (aggregateSettingsRequestRef.current !== request) {
            return;
          }
          message.error(
            t('gateway.aggregate.notice.statusLoadFailed', {
              error: error instanceof Error ? error.message : String(error),
            }),
          );
        })
        .finally(() => {
          if (aggregateSettingsRequestRef.current === request) {
            setAggregateSettingsOpening(false);
          }
        });
    },
    [aggregateSettingsOpening, t],
  );

  const handleAggregateTakeoverChange = React.useCallback(() => {
    const request = aggregateSettingsRequestRef.current;
    void getProxyGatewayStatus()
      .then((status) => {
        if (isActive && aggregateSettingsRequestRef.current === request) {
          setAggregateGatewayRunning(status.running);
        }
      })
      .catch(() => undefined);
    void refreshGatewayCliStatus().catch(() => undefined);
  }, [isActive, refreshGatewayCliStatus]);

  React.useEffect(() => {
    if (!isActive) {
      gatewayStatusRevisionRef.current += 1;
      gatewayStatusRequestRef.current += 1;
      return;
    }
    void refreshGatewayCliStatus().catch(() => undefined);
  }, [isActive, refreshGatewayCliStatus]);

  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<boolean>('gateway-running-changed', () => {
      if (disposed || !isActive) {
        return;
      }
      gatewayStatusRevisionRef.current += 1;
      void refreshGatewayCliStatus().catch(() => undefined);
      const request = aggregateSettingsRequestRef.current;
      void getProxyGatewayStatus()
        .then((status) => {
          if (!disposed && isActive && aggregateSettingsRequestRef.current === request) {
            setAggregateGatewayRunning(status.running);
          }
        })
        .catch(() => undefined);
    }).then((dispose) => {
      if (disposed) {
        dispose();
      } else {
        unlisten = dispose;
      }
    }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [isActive, refreshGatewayCliStatus]);

  React.useEffect(() => {
    if (isActive) {
      return;
    }
    aggregateSettingsRequestRef.current += 1;
    setAggregateSettingsOpening(false);
    setAggregateSettingsOpen(false);
  }, [isActive]);
  const [savingCodexUnifiedHistory, setSavingCodexUnifiedHistory] = React.useState(false);
  const [refreshingOfficialAccountId, setRefreshingOfficialAccountId] = React.useState<string | null>(null);
  const [savingOfficialAccountId, setSavingOfficialAccountId] = React.useState<string | null>(null);
  const [codexDeviceAuthSession, setCodexDeviceAuthSession] = React.useState<
    CodexDeviceAuthStartResult | null
  >(null);
  const [officialAccountDetails, setOfficialAccountDetails] = React.useState<{
    provider: CodexProvider;
    account: CodexOfficialAccount;
  } | null>(null);

  // Modal states
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<CodexProvider | null>(null);
  const [shareProvider, setShareProvider] = React.useState<CodexProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [providerModalMode, setProviderModalMode] = React.useState<'manual' | 'import'>('manual');
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [historySyncModalOpen, setHistorySyncModalOpen] = React.useState(false);
  const [conflictDialogOpen, setConflictDialogOpen] = React.useState(false);
  const [conflictInfo, setConflictInfo] = React.useState<ImportConflictInfo | null>(null);
  const [pendingFormValues, setPendingFormValues] = React.useState<CodexProviderFormValues | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<CodexSettings | null>(null);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityInfo, setConnectivityInfo] = React.useState<ProviderConnectivityInfo | null>(null);
  const [connectivityUsesGateway, setConnectivityUsesGateway] = React.useState(false);
  const [connectivityStatuses, setConnectivityStatuses] = React.useState<Record<string, ProviderConnectivityStatusItem>>({});
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [modelModalProviderId, setModelModalProviderId] = React.useState<string | null>(null);
  const [modelModalRowKey, setModelModalRowKey] = React.useState('');
  const [modelModalInitialValues, setModelModalInitialValues] = React.useState<CodexCatalogModel | undefined>(undefined);
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string | null>(null);
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [modelBatchDeleteProviderId, setModelBatchDeleteProviderId] = React.useState<string | null>(null);
  const [selectedModelRowKeysByProvider, setSelectedModelRowKeysByProvider] = React.useState<Record<string, string[]>>({});
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
  const [memoriesListCollapsed, setMemoriesListCollapsed] = React.useState(true);
  const [pluginPanelRefreshToken, setPluginPanelRefreshToken] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [sessionManagerRefreshNonce, setSessionManagerRefreshNonce] = React.useState(0);
  const [sessionSourceMode, setSessionSourceMode] = React.useState<SessionSourceMode>(() => rememberedCodexSessionSourceMode);

  const handleSessionSourceModeChange = React.useCallback((sourceMode: SessionSourceMode) => {
    rememberedCodexSessionSourceMode = sourceMode;
    setSessionSourceMode(sourceMode);
  }, []);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const [savingCodexAuthPreservation, setSavingCodexAuthPreservation] = React.useState(false);
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
    {
      id: 'codex-memories',
      title: t('codex.memories.title'),
      order: 5,
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
          shouldLoadCodexOfficialAccounts(provider)
            ? await listCodexOfficialAccounts(provider.id)
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
    const handleDeepLinkImport = (event: Event) => {
      const detail = (event as CustomEvent<{ app?: string; id?: string }>).detail;
      if (detail?.app === 'codex') {
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
    rootDirectoryChangeLocked: gatewayTakeoverActive,
    rootDirectoryChangeLockedText: t('gateway.proxy.rootDirectorySaveLockedTooltip'),
    loadConfig,
    getCommonConfig: getCodexCommonConfig,
    saveCommonConfig: saveCodexCommonConfig,
  });

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('codex');
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
    (provider: CodexProvider) => provider.id !== CODEX_LOCAL_PROVIDER_ID
      && (officialAccountsByProviderId[provider.id]?.length ?? 0) === 0,
    [officialAccountsByProviderId],
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
            buildFavoriteProviderStorageKey('codex', provider.id),
            buildCodexFavoriteProviderConfig(provider),
          ),
          (provider) => t('common.batch.backupFailed', { name: provider.name }),
        );
        for (const provider of providersToDelete) {
          await deleteCodexProvider(provider.id);
        }
        await loadFavoriteProviders();
        await loadConfig();
        await refreshTrayMenu();
        message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete codex providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig();
        await loadFavoriteProviders();
        return false;
      }
    },
    [providers, canBatchDeleteProvider, loadFavoriteProviders, loadConfig, t],
  );
  // Selectable ids exclude the local provider (no delete path / not a managed preset).
  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter(canBatchDeleteProvider).map((provider) => provider.id),
    [visibleProviders, canBatchDeleteProvider],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  const handleSelectProvider = async (provider: CodexProvider) => {
    try {
      await selectCodexProvider(provider.id);
      message.success(t('codex.apply.success'));
      noteProviderUsed(provider.id);
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

  const startBrowserOfficialAccountOauth = async (provider: CodexProvider) => {
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

  const startDeviceOfficialAccountOauth = async (provider: CodexProvider) => {
    try {
      const session = await startCodexOfficialAccountDeviceAuth(provider.id);
      setCodexDeviceAuthSession(session);
    } catch (error) {
      console.error('Failed to start Codex official account device auth:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleStartOfficialAccountOauth = (provider: CodexProvider) => {
    const loginMethodModal = Modal.confirm({
      title: t('codex.provider.loginMethodTitle'),
      icon: null,
      footer: null,
      content: (
        <Space direction="vertical" size={12} style={{ width: '100%', marginTop: 12 }}>
          <Button
            block
            type="primary"
            icon={<LinkOutlined />}
            onClick={() => {
              loginMethodModal.destroy();
              void startBrowserOfficialAccountOauth(provider);
            }}
          >
            {t('codex.provider.browserOauth')}
          </Button>
          <Button
            block
            onClick={() => {
              loginMethodModal.destroy();
              void startDeviceOfficialAccountOauth(provider);
            }}
          >
            {t('codex.provider.deviceAuth')}
          </Button>
          <Button
            block
            onClick={() => {
              loginMethodModal.destroy();
              let authJson = '';
              Modal.confirm({
                title: t('codex.provider.authJsonImportTitle'),
                icon: null,
                okText: t('common.confirm'),
                cancelText: t('common.cancel'),
                width: 640,
                content: (
                  <Input.TextArea
                    autoFocus
                    autoSize={{ minRows: 10, maxRows: 18 }}
                    placeholder={t('codex.provider.authJsonImportPlaceholder')}
                    onChange={(event) => {
                      authJson = event.target.value;
                    }}
                  />
                ),
                onOk: async () => {
                  if (!authJson.trim()) {
                    message.error(t('codex.provider.authJsonImportEmpty'));
                    return;
                  }
                  try {
                    await importCodexOfficialAccountFromAuthJson(provider.id, authJson);
                    message.success(t('codex.provider.authJsonImportSuccess'));
                    await loadConfig();
                    await refreshTrayMenu();
                  } catch (error) {
                    console.error('Failed to import Codex auth.json account:', error);
                    const errorMsg = error instanceof Error ? error.message : String(error);
                    message.error(errorMsg || t('common.error'));
                    throw error;
                  }
                },
              });
            }}
          >
            {t('codex.provider.authJsonImport')}
          </Button>
        </Space>
      ),
    });
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

  const formatResetMoment = React.useCallback((value?: number | null) => {
    if (value == null) {
      return ACCOUNT_DETAILS_EMPTY_VALUE;
    }
    const absolute = new Date(value * 1000).toLocaleString();
    const remaining = formatQuotaResetRemainingText(value, {
      expiredLabel: t('codex.provider.officialAccountResetDone'),
    });
    return remaining ? `${absolute} (${remaining})` : absolute;
  }, [t]);

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

  // OpenCode import entry removed; keep handler for later restore.
  const _handleImportFromOpenCode = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalMode('import');
    setProviderModalOpen(true);
  };
  void _handleImportFromOpenCode;

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

  const handleShareProvider = (provider: CodexProvider) => {
    setShareProvider(provider);
  };

  const handleTestProvider = (provider: CodexProvider) => {
    if (provider.category === 'official') {
      message.info(t('codex.provider.officialConnectivityHint'));
      return;
    }
    if (!areGatewayProviderProfilesInitialized()) {
      message.info(t('common.loading'));
      return;
    }

    setConnectivityInfo(buildCodexProviderConnectivityInfo(provider));
    setConnectivityUsesGateway(codexProviderNeedsGatewayProxy(provider));
    setConnectivityModalOpen(true);
  };

  /**
   * Persist catalog edits on a saved provider.
   *
   * Model rows are catalog-only, so the config.toml model name is passed in
   * explicitly and only written when the caller changes it. Applied providers
   * replay their active gateway takeover (single / failover / aggregate).
   */
  const persistProviderCatalog = React.useCallback(async (
    provider: CodexProvider,
    models: CodexCatalogModel[],
    options?: {
      defaultModel?: string;
      autoReviewModelOverride?: string | null;
      /** Default level to project into config.toml; omit to keep the current value. */
      defaultReasoningEffort?: string;
    },
  ) => {
    const settings = parseCodexSettingsConfig(provider.settingsConfig);
    const defaultModel = options?.defaultModel !== undefined
      ? options.defaultModel
      : (extractCodexModel(settings.config) || '');
    const autoReviewModelOverride = options?.autoReviewModelOverride === undefined
      ? resolveCodexAutoReviewModelOverride(settings)
      : (options.autoReviewModelOverride?.trim() || undefined);
    const settingsConfig = buildCodexSettingsConfig({
      category: provider.category,
      apiKey: settings.auth?.OPENAI_API_KEY || '',
      baseUrl: extractCodexBaseUrl(settings.config) || '',
      model: defaultModel,
      reasoningEffort: options?.defaultReasoningEffort,
      config: settings.config || '',
      catalogModels: models,
      autoReviewModelOverride,
      auth: settings.auth ?? {},
    });

    const gatewayModeBeforeSave = resolveGatewayReengageMode(gatewayCliStatus);
    const shouldReengageGateway = Boolean(provider.isApplied) && gatewayModeBeforeSave !== null;
    const savedProvider = await saveCodexProviderCatalogWithGatewayReengage({
      provider,
      settingsConfig,
      gatewayMode: shouldReengageGateway ? gatewayModeBeforeSave : null,
      aggregateConfig: shouldReengageGateway
        ? toGatewayAggregateReengageConfig(gatewayCliStatus)
        : null,
      updateProvider: (nextProvider) => updateCodexProvider(nextProvider),
      restoreDirect: () => restoreProxyGatewayCliDirect('codex'),
      engageSingle: () => engageProxyGatewaySingle('codex', provider.id),
      engageFailover: () => engageProxyGatewayFailover('codex'),
      engageAggregate: ({
        providerIds,
        separator,
        aliases,
        naming,
        subagentModel,
        subagentReasoningEffort,
        crossSiteFailover,
        subagentExposedModels,
      }) =>
        engageProxyGatewayAggregate(
          'codex',
          providerIds,
          separator,
          aliases,
          naming,
          subagentModel,
          subagentReasoningEffort,
          crossSiteFailover,
          subagentExposedModels,
        ),
      onGatewayStatusChange: applyGatewayCliStatus,
    });

    try {
      await upsertFavoriteProvider(
        buildFavoriteProviderStorageKey('codex', savedProvider.id),
        buildCodexFavoriteProviderConfig(savedProvider),
      );
      await loadFavoriteProviders();
    } catch (error) {
      console.error('Failed to refresh Codex favorite provider after a model change:', error);
    }

    await loadConfig(true);
    await refreshTrayMenu();
    return savedProvider;
  }, [applyGatewayCliStatus, gatewayCliStatus, loadConfig, loadFavoriteProviders]);

  const modelModalProvider = React.useMemo(
    () => providers.find((provider) => provider.id === modelModalProviderId) ?? null,
    [modelModalProviderId, providers],
  );
  // The preset library is provider-aware: a saved provider knows its SDK from
  // the stored API format.
  const resolveModelCatalogPreset = React.useCallback<CodexCatalogPresetResolver>(
    (modelId) => (modelModalProvider
      ? findPresetModelById(modelId, getCodexProviderSdkType(modelModalProvider))
      : undefined),
    [modelModalProvider],
  );

  const handleAddModel = React.useCallback((provider: CodexProvider) => {
    if (provider.category === 'official' || isCodexLocalProviderId(provider.id)) {
      return;
    }
    setModelModalProviderId(provider.id);
    setModelModalRowKey('');
    setModelModalInitialValues(undefined);
    setModelModalOpen(true);
  }, []);

  const handleEditModel = React.useCallback((provider: CodexProvider, modelRowKey: string) => {
    const models = getCodexProviderCatalogModels(provider);
    const rowIndex = findCodexCatalogRowIndex(models, modelRowKey);
    if (rowIndex < 0) {
      return;
    }
    setModelModalProviderId(provider.id);
    setModelModalRowKey(modelRowKey);
    setModelModalInitialValues(models[rowIndex]);
    setModelModalOpen(true);
  }, []);

  const handleCopyModel = React.useCallback((provider: CodexProvider, modelRowKey: string) => {
    const models = getCodexProviderCatalogModels(provider);
    const rowIndex = findCodexCatalogRowIndex(models, modelRowKey);
    if (rowIndex < 0) {
      return;
    }
    const source = models[rowIndex];
    setModelModalProviderId(provider.id);
    setModelModalRowKey('');
    setModelModalInitialValues({
      ...source,
      displayName: `${source.displayName?.trim() || source.model.trim()} copy`,
    });
    setModelModalOpen(true);
  }, []);

  const handleModelFormSubmit = React.useCallback(async (model: CodexCatalogModel) => {
    if (!modelModalProvider) {
      return;
    }
    try {
      const models = getCodexProviderCatalogModels(modelModalProvider);
      const currentDefault = extractCodexModel(
        parseCodexSettingsConfig(modelModalProvider.settingsConfig).config,
      ) || '';
      const editedRowModel = modelModalRowKey
        ? models.find((item) => codexCatalogRowKey(item) === modelModalRowKey)?.model.trim() ?? ''
        : '';
      const nextModels = upsertCodexCatalogModel(models, model, modelModalRowKey || undefined);
      // Renaming the row that config.toml points at must keep that pointer valid;
      // a provider with no default model adopts its first catalog row.
      let nextDefault = currentDefault;
      if (editedRowModel && currentDefault === editedRowModel) {
        nextDefault = model.model.trim();
      }
      if (!nextDefault) {
        nextDefault = nextModels[0]?.model.trim() ?? '';
      }
      // The main model's reasoning level follows the same config.toml pointer as
      // `model`: project it when this row is (or becomes) the main model.
      const editedPrimaryModel = Boolean(nextDefault) && nextDefault === model.model.trim();
      const primaryModelChanged = nextDefault !== currentDefault;
      await persistProviderCatalog(modelModalProvider, nextModels, {
        defaultModel: nextDefault,
        ...(editedPrimaryModel || primaryModelChanged
          ? { defaultReasoningEffort: resolveCodexDefaultReasoningEffort(nextModels, nextDefault) }
          : {}),
      });
      setModelModalOpen(false);
      setModelModalProviderId(null);
      setModelModalRowKey('');
      setModelModalInitialValues(undefined);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to save Codex catalog model:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [modelModalProvider, modelModalRowKey, persistProviderCatalog, t]);

  const handleDeleteModel = React.useCallback(async (provider: CodexProvider, modelRowKey: string) => {
    try {
      const models = getCodexProviderCatalogModels(provider);
      const deletedModel = models.find((item) => codexCatalogRowKey(item) === modelRowKey)?.model.trim() ?? '';
      const currentDefault = extractCodexModel(parseCodexSettingsConfig(provider.settingsConfig).config) || '';
      const nextModels = removeCodexCatalogModels(models, [modelRowKey]);
      const nextDefault = currentDefault === deletedModel
        ? (nextModels[0]?.model.trim() ?? '')
        : currentDefault;
      await persistProviderCatalog(provider, nextModels, {
        defaultModel: nextDefault,
        ...(nextDefault !== currentDefault
          ? { defaultReasoningEffort: resolveCodexDefaultReasoningEffort(nextModels, nextDefault) }
          : {}),
      });
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to delete Codex catalog model:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [persistProviderCatalog, t]);

  const handleSetPrimaryModel = React.useCallback(async (provider: CodexProvider, modelRowKey: string) => {
    try {
      const models = getCodexProviderCatalogModels(provider);
      const row = models.find((item) => codexCatalogRowKey(item) === modelRowKey);
      if (!row) {
        return;
      }
      await persistProviderCatalog(provider, models, {
        defaultModel: row.model.trim(),
        defaultReasoningEffort: resolveCodexDefaultReasoningEffort(models, row.model.trim()),
      });
      message.success(t('codex.model.setAsPrimarySuccess', {
        name: row.displayName?.trim() || row.model.trim(),
      }));
    } catch (error) {
      console.error('Failed to set Codex default model:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [persistProviderCatalog, t]);

  const handleSetAutoReviewModel = React.useCallback(async (provider: CodexProvider, modelRowKey: string) => {
    try {
      const models = getCodexProviderCatalogModels(provider);
      const row = models.find((item) => codexCatalogRowKey(item) === modelRowKey);
      if (!row) {
        return;
      }
      await persistProviderCatalog(provider, models, { autoReviewModelOverride: row.model.trim() });
      message.success(t('codex.model.setAutoReviewSuccess', {
        name: row.displayName?.trim() || row.model.trim(),
      }));
    } catch (error) {
      console.error('Failed to set Codex auto-review model:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [persistProviderCatalog, t]);

  const handleClearAutoReviewModel = React.useCallback(async (provider: CodexProvider) => {
    try {
      await persistProviderCatalog(provider, getCodexProviderCatalogModels(provider), {
        autoReviewModelOverride: null,
      });
      message.success(t('codex.model.clearAutoReviewSuccess'));
    } catch (error) {
      console.error('Failed to clear Codex auto-review model:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [persistProviderCatalog, t]);

  const handleReorderModels = React.useCallback(async (
    provider: CodexProvider,
    orderedModelRowKeys: string[],
  ) => {
    try {
      const models = reorderCodexCatalogModels(
        getCodexProviderCatalogModels(provider),
        orderedModelRowKeys,
      );
      await persistProviderCatalog(provider, models);
    } catch (error) {
      console.error('Failed to reorder Codex catalog models:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [persistProviderCatalog, t]);

  const handleToggleModelBatchDeleteMode = React.useCallback((provider: CodexProvider) => {
    if (modelBatchDeleteProviderId === provider.id) {
      setSelectedModelRowKeysByProvider({});
      setModelBatchDeleteProviderId(null);
      return;
    }
    setSelectedModelRowKeysByProvider({});
    setModelBatchDeleteProviderId(provider.id);
  }, [modelBatchDeleteProviderId]);

  const handleToggleModelSelection = React.useCallback((
    provider: CodexProvider,
    modelRowKey: string,
    selected: boolean,
  ) => {
    setSelectedModelRowKeysByProvider((previous) => {
      const current = previous[provider.id] ?? [];
      const next = selected
        ? Array.from(new Set([...current, modelRowKey]))
        : current.filter((key) => key !== modelRowKey);
      if (next.length === 0) {
        const nextState = { ...previous };
        delete nextState[provider.id];
        return nextState;
      }
      return { ...previous, [provider.id]: next };
    });
  }, []);

  const handleBatchDeleteModels = React.useCallback((provider: CodexProvider) => {
    const selectedRowKeys = selectedModelRowKeysByProvider[provider.id] ?? [];
    if (selectedRowKeys.length === 0) {
      return;
    }
    Modal.confirm({
      title: t('codex.model.batchDeleteConfirmTitle'),
      content: t('codex.model.batchDeleteConfirmContent', { count: selectedRowKeys.length }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        try {
          const models = getCodexProviderCatalogModels(provider);
          const removedModels = models
            .filter((item) => selectedRowKeys.includes(codexCatalogRowKey(item)))
            .map((item) => item.model.trim());
          const currentDefault = extractCodexModel(
            parseCodexSettingsConfig(provider.settingsConfig).config,
          ) || '';
          const nextModels = removeCodexCatalogModels(models, selectedRowKeys);
          const nextDefault = removedModels.includes(currentDefault)
            ? (nextModels[0]?.model.trim() ?? '')
            : currentDefault;
          await persistProviderCatalog(provider, nextModels, {
            defaultModel: nextDefault,
            ...(nextDefault !== currentDefault
              ? { defaultReasoningEffort: resolveCodexDefaultReasoningEffort(nextModels, nextDefault) }
              : {}),
          });
          setSelectedModelRowKeysByProvider((previous) => {
            if (!(provider.id in previous)) {
              return previous;
            }
            const nextState = { ...previous };
            delete nextState[provider.id];
            return nextState;
          });
          setModelBatchDeleteProviderId((current) => (current === provider.id ? null : current));
          message.success(t('codex.model.batchDeleteSuccess', { count: selectedRowKeys.length }));
        } catch (error) {
          console.error('Failed to batch delete Codex catalog models:', error);
          message.error(error instanceof Error ? error.message : String(error));
        }
      },
    });
  }, [persistProviderCatalog, selectedModelRowKeysByProvider, t]);

  const handleOpenFetchModels = React.useCallback((provider: CodexProvider) => {
    if (provider.category === 'official' || isCodexLocalProviderId(provider.id)) {
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
    const settings = parseCodexSettingsConfig(fetchModelsProvider.settingsConfig);
    return {
      providerId: fetchModelsProvider.id,
      name: fetchModelsProvider.name,
      baseUrl: extractCodexBaseUrl(settings.config) || '',
      apiKey: settings.auth?.OPENAI_API_KEY || '',
      sdkType: getCodexProviderSdkType(fetchModelsProvider),
      existingModelIds: getCodexProviderCatalogModels(fetchModelsProvider)
        .map((item) => item.model.trim()),
    };
  }, [fetchModelsProvider]);

  const handleFetchModelsApply = React.useCallback(async (result: FetchModelsApplyResult) => {
    if (!fetchModelsProvider) {
      return;
    }
    try {
      const models = importModelsIntoCatalog(
        getCodexProviderCatalogModels(fetchModelsProvider),
        result.selectedModels,
        result.removedModelIds,
        result.orderedModelIds,
        (modelId) => findPresetModelById(modelId, getCodexProviderSdkType(fetchModelsProvider)),
      );
      const currentDefault = extractCodexModel(
        parseCodexSettingsConfig(fetchModelsProvider.settingsConfig).config,
      ) || '';
      const nextDefault = currentDefault || (models[0]?.model.trim() ?? '');
      await persistProviderCatalog(fetchModelsProvider, models, {
        defaultModel: nextDefault,
        ...(nextDefault !== currentDefault
          ? { defaultReasoningEffort: resolveCodexDefaultReasoningEffort(models, nextDefault) }
          : {}),
      });
      setFetchModelsModalOpen(false);
      setFetchModelsProviderId(null);
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to apply fetched Codex models:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [fetchModelsProvider, persistProviderCatalog, t]);

  /** Remove catalog rows selected after a failed connectivity test. */
  const handleRemoveConnectivityModels = React.useCallback(async (modelIds: string[]) => {
    if (!connectivityInfo || modelIds.length === 0) {
      return;
    }
    const provider = providers.find((item) => item.id === connectivityInfo.providerId);
    if (!provider) {
      return;
    }
    try {
      const models = getCodexProviderCatalogModels(provider);
      const failedModels = new Set(modelIds);
      const rowKeys = models
        .filter((item) => failedModels.has(item.model.trim()))
        .map((item) => codexCatalogRowKey(item));
      const currentDefault = extractCodexModel(
        parseCodexSettingsConfig(provider.settingsConfig).config,
      ) || '';
      const nextModels = removeCodexCatalogModels(models, rowKeys);
      const nextDefault = failedModels.has(currentDefault)
        ? (nextModels[0]?.model.trim() ?? '')
        : currentDefault;
      await persistProviderCatalog(provider, nextModels, {
        defaultModel: nextDefault,
        ...(nextDefault !== currentDefault
          ? { defaultReasoningEffort: resolveCodexDefaultReasoningEffort(nextModels, nextDefault) }
          : {}),
      });
      message.success(t('common.success'));
    } catch (error) {
      console.error('Failed to remove Codex models after connectivity test:', error);
      message.error(error instanceof Error ? error.message : String(error));
    }
  }, [connectivityInfo, persistProviderCatalog, providers, t]);

  const connectivityRemovableModelIds = React.useMemo(() => {
    if (!connectivityInfo) {
      return [];
    }
    const provider = providers.find((item) => item.id === connectivityInfo.providerId);
    if (!provider) {
      return [];
    }
    return [...new Set(
      getCodexProviderCatalogModels(provider).map((item) => item.model.trim()),
    )];
  }, [connectivityInfo, providers]);

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
      message.info(t('codex.provider.officialBatchSkipped', { count: officialProviders.length }));
    }

    if (testableProviders.length === 0) {
      setConnectivityStatuses({});
      return;
    }

    const targets = testableProviders.map((provider) => {
      const connectivityInfo = buildCodexProviderConnectivityInfo(provider);
      const settingsConfig = parseCodexSettingsConfig(provider.settingsConfig) as {
        config?: string;
      };
      const hasExplicitBaseUrl = Boolean(
        settingsConfig.config?.match(/^\s*base_url\s*=\s*['"]/m),
      );
      const useGateway = codexProviderNeedsGatewayProxy(provider);

      if (provider.category !== 'official' && !hasExplicitBaseUrl) {
        return {
          providerId: provider.id,
          errorMessage: t('common.baseUrlMissing'),
        };
      }

      return buildProviderConnectivityBatchTarget(connectivityInfo, {
        requireBaseUrl: false,
        requireApiKey: !useGateway,
        gatewayCliKey: 'codex',
        useGateway,
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
  }, [providers, t, favoriteProviders, gatewayProviderProfilesVersion]);

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

        const createdProvider = await createCodexProvider({
          name: candidate.name,
          category: (candidate.normalizedCategory || 'custom') as CodexProviderInput['category'],
          settingsConfig,
          sourceProviderId: candidate.sourceProviderId,
          websiteUrl: candidate.websiteUrl,
          notes: candidate.notes,
          icon: candidate.icon,
          iconColor: candidate.iconColor,
        });

        try {
          await upsertFavoriteProvider(
            buildFavoriteProviderStorageKey('codex', createdProvider.id),
            buildCodexFavoriteProviderConfig(createdProvider),
          );
        } catch (favoriteError) {
          console.error('Failed to save Codex favorite provider from CC Switch import:', favoriteError);
        }
        ok += 1;
      } catch (error) {
        console.error('Failed to import Codex provider from CC Switch:', candidate.name, error);
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
        const payload = getFavoriteProviderPayload<CodexFavoriteProviderPayload>(favoriteProvider);
        if (!payload) {
          continue;
        }

        const createdProvider = await createCodexProvider({
          name: payload.name,
          category: payload.category as CodexProviderInput['category'],
          settingsConfig: payload.settingsConfig,
          meta: payload.meta as CodexProviderInput['meta'],
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
      const isLocalTemp = editingProvider?.id === CODEX_LOCAL_PROVIDER_ID;

      const providerInput: CodexProviderInput = {
        name: values.name,
        category: values.category,
        settingsConfig,
        sourceProviderId: values.sourceProviderId,
        meta: values.meta,
        notes: values.notes,
      };

      let savedProviderId = isLocalTemp ? CODEX_LOCAL_PROVIDER_ID : '';
      let savedProvider: CodexProvider | null = null;
      const gatewayModeBeforeSave = resolveGatewayReengageMode(gatewayCliStatus);
      const gatewayAggregateBeforeSave = toGatewayAggregateReengageConfig(gatewayCliStatus);
      const shouldReengageGatewayProxy =
        Boolean(editingProvider && !isCopyMode && !isLocalTemp && editingProvider.isApplied) &&
        gatewayModeBeforeSave !== null;

      await saveProviderWithGatewayReengage({
        gatewayMode: shouldReengageGatewayProxy ? gatewayModeBeforeSave : null,
        aggregateConfig: shouldReengageGatewayProxy ? gatewayAggregateBeforeSave : null,
        restoreDirect: () => restoreProxyGatewayCliDirect('codex'),
        engageSingle: () => engageProxyGatewaySingle('codex', savedProviderId),
        engageFailover: () => engageProxyGatewayFailover('codex'),
        engageAggregate: ({
          providerIds,
          separator,
          aliases,
          naming,
          subagentModel,
          subagentReasoningEffort,
          crossSiteFailover,
          subagentExposedModels,
        }) =>
          engageProxyGatewayAggregate(
            'codex',
            providerIds,
            separator,
            aliases,
            naming,
            subagentModel,
            subagentReasoningEffort,
            crossSiteFailover,
            subagentExposedModels,
          ),
        onGatewayStatusChange: applyGatewayCliStatus,
        saveProvider: async () => {
          if (isLocalTemp) {
            await saveCodexLocalConfig({ provider: providerInput });
          } else if (editingProvider && !isCopyMode) {
            savedProvider = await updateCodexProvider({
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
            savedProvider = await createCodexProvider(providerInput);
            savedProviderId = savedProvider.id;
          }
        },
      });

      try {
        const providerForFavorite: CodexProvider = savedProvider || {
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
        meta: values.meta,
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

  const handleCodexAuthPreservationChange = async (enabled: boolean) => {
    setSavingCodexAuthPreservation(true);
    try {
      await setCodexPreserveOfficialAuthOnSwitch(enabled);
      message.success(
        enabled
          ? t('codex.settings.preserveOfficialAuthOnSwitchEnabled')
          : t('codex.settings.preserveOfficialAuthOnSwitchDisabled'),
      );
    } catch (error) {
      // Store only updates after the backend succeeds; no local optimistic flip to undo.
      console.error('Failed to update Codex auth preservation setting:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setSavingCodexAuthPreservation(false);
    }
  };

  const unifiedHistorySkippedText = (reason?: string) => {
    switch (reason) {
      case 'live_not_unified':
        return t('codex.settings.unifiedSessionHistorySkippedLiveNotUnified');
      case 'no_official_history':
        return t('codex.settings.unifiedSessionHistorySkippedNoOfficialHistory');
      case 'no_backup_ledger':
        return t('codex.settings.unifiedSessionHistorySkippedNoBackupLedger');
      case 'nothing_to_restore':
        return t('codex.settings.unifiedSessionHistorySkippedNothingToRestore');
      case 'unify_toggle_on':
        return t('codex.settings.unifiedSessionHistorySkippedToggleOn');
      case 'migration_failed':
        return t('codex.settings.unifiedSessionHistorySkippedMigrationFailed');
      default:
        return reason || t('common.unknown');
    }
  };

  const showUnifiedHistoryMigrationResult = (
    migration?: Awaited<ReturnType<typeof setCodexUnifiedSessionHistory>>['migration'],
  ) => {
    if (!migration) {
      return;
    }
    if (migration.skippedReason) {
      message.info(t('codex.settings.unifiedSessionHistoryMigrationSkipped', {
        reason: unifiedHistorySkippedText(migration.skippedReason),
      }));
      return;
    }
    message.success(t('codex.settings.unifiedSessionHistoryMigrationSuccess', {
      files: migration.migratedSessionFiles,
      threads: migration.migratedThreadRows,
    }));
  };

  const handleCodexUnifiedHistoryEnable = () => {
    let migrateExisting = false;
    Modal.confirm({
      title: t('codex.settings.unifiedSessionHistoryEnableTitle'),
      content: (
        <Space direction="vertical" size={10}>
          <Text>{t('codex.settings.unifiedSessionHistoryEnableContent')}</Text>
          <Text type="secondary">{t('codex.settings.unifiedSessionHistoryRiskHint')}</Text>
          <Checkbox onChange={(event) => {
            migrateExisting = event.target.checked;
          }}>
            {t('codex.settings.unifiedSessionHistoryMigrateExisting')}
          </Checkbox>
        </Space>
      ),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        setSavingCodexUnifiedHistory(true);
        try {
          const result = await setCodexUnifiedSessionHistory(true, migrateExisting);
          setCodexUnifiedSessionHistoryEnabled(result.enabled);
          message.success(t('codex.settings.unifiedSessionHistoryEnabled'));
          showUnifiedHistoryMigrationResult(result.migration);
        } catch (error) {
          console.error('Failed to enable Codex unified session history:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        } finally {
          setSavingCodexUnifiedHistory(false);
        }
      },
    });
  };

  const handleCodexUnifiedHistoryDisable = async () => {
    setSavingCodexUnifiedHistory(true);
    let hasBackup = false;
    try {
      hasBackup = await hasCodexUnifiedHistoryBackup();
    } catch (error) {
      console.warn('Failed to inspect Codex unified history backup:', error);
    } finally {
      setSavingCodexUnifiedHistory(false);
    }

    let restoreBackup = hasBackup;
    Modal.confirm({
      title: t('codex.settings.unifiedSessionHistoryDisableTitle'),
      content: (
        <Space direction="vertical" size={10}>
          <Text>{t('codex.settings.unifiedSessionHistoryDisableContent')}</Text>
          {hasBackup ? (
            <Checkbox
              defaultChecked
              onChange={(event) => {
                restoreBackup = event.target.checked;
              }}
            >
              {t('codex.settings.unifiedSessionHistoryRestoreBackup')}
            </Checkbox>
          ) : (
            <Text type="secondary">{t('codex.settings.unifiedSessionHistoryNoBackup')}</Text>
          )}
        </Space>
      ),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        setSavingCodexUnifiedHistory(true);
        try {
          const result = await setCodexUnifiedSessionHistory(false, false);
          setCodexUnifiedSessionHistoryEnabled(result.enabled);
          message.success(t('codex.settings.unifiedSessionHistoryDisabled'));
          if (restoreBackup) {
            const restore = await restoreCodexUnifiedSessionHistory();
            if (restore.skippedReason) {
              message.info(t('codex.settings.unifiedSessionHistoryRestoreSkipped', {
                reason: unifiedHistorySkippedText(restore.skippedReason),
              }));
            } else {
              message.success(t('codex.settings.unifiedSessionHistoryRestoreSuccess', {
                files: restore.restoredSessionFiles,
                threads: restore.restoredThreadRows,
              }));
            }
          }
        } catch (error) {
          console.error('Failed to disable Codex unified session history:', error);
          const errorMsg = error instanceof Error ? error.message : String(error);
          message.error(errorMsg || t('common.error'));
        } finally {
          setSavingCodexUnifiedHistory(false);
        }
      },
    });
  };

  const handleCodexUnifiedHistoryChange = (enabled: boolean) => {
    if (gatewayTakeoverActive) {
      message.warning(t('codex.settings.unifiedSessionHistoryGatewayLocked'));
      return;
    }
    if (enabled) {
      handleCodexUnifiedHistoryEnable();
    } else {
      void handleCodexUnifiedHistoryDisable();
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
          case 'codex-session-manager':
            return <MessageOutlined />;
          case 'codex-memories':
            return <BulbOutlined />;
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
          case 'codex-memories':
            setMemoriesListCollapsed(false);
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
                  <Space size={8} wrap>
                    <Text strong>
                      <DatabaseOutlined style={{ marginRight: 8 }} />
                      {t('codex.provider.title')}
                    </Text>
                    <GatewayFailoverButton
                      cliKey="codex"
                      status={gatewayCliStatus}
                      primaryProviderNeedsGatewayProxy={primaryGatewayProviderNeedsProxy}
                      primaryProviderNeedsProxyReason={primaryGatewayProviderNeedsProxyReason}
                      onStatusChange={applyGatewayCliStatus}
                    />
                    {gatewayTakeoverActive ? (
                      <GatewayAggregateButton
                        current={isGatewayAggregateMode(gatewayCliStatus?.mode)}
                        loading={aggregateSettingsOpening}
                        onClick={handleOpenAggregateSettings}
                      />
                    ) : null}
                  </Space>
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
                      modes={PROVIDER_SORT_MODES}
                      onChange={setSortMode}
                    />
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
                    ) : visibleProviders.length === 0 ? (
                      <ProviderSearchEmpty />
                    ) : (
                      <DndContext
                            sensors={providerBatchDragDisabled ? [] : sensors}
                            collisionDetection={closestCenter}
                            onDragEnd={handleDragEnd}
                            modifiers={[restrictToVerticalAxis]}
                          >
                            <SortableContext
                              items={providers.map((p) => p.id)}
                              strategy={verticalListSortingStrategy}
                            >
                              <div>
                                {visibleProviders.map((provider) => (
                              <CodexProviderCard
                                key={provider.id}
                                provider={provider}
                                isApplied={provider.id === appliedProviderId}
                                officialAccounts={officialAccountsByProviderId[provider.id] || []}
                                onEdit={handleEditProvider}
                                onDelete={handleDeleteProvider}
                                onCopy={handleCopyProvider}
                                onShare={handleShareProvider}
                                onTest={handleTestProvider}
                                onSelect={handleSelectProvider}
                                onToggleDisabled={handleToggleDisabled}
                                onAddModel={handleAddModel}
                                onEditModel={handleEditModel}
                                onCopyModel={handleCopyModel}
                                onDeleteModel={handleDeleteModel}
                                onSetPrimaryModel={handleSetPrimaryModel}
                                onSetAutoReviewModel={handleSetAutoReviewModel}
                                onClearAutoReviewModel={handleClearAutoReviewModel}
                                onFetchModels={handleOpenFetchModels}
                                onReorderModels={handleReorderModels}
                                onToggleBatchDeleteMode={handleToggleModelBatchDeleteMode}
                                onBatchDeleteModels={handleBatchDeleteModels}
                                modelSelectionMode={modelBatchDeleteProviderId === provider.id}
                                selectedModelRowKeys={selectedModelRowKeysByProvider[provider.id] ?? []}
                                onToggleModelSelection={handleToggleModelSelection}
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
                                  applyGatewayCliStatus(status);
                                  await loadConfig();
                                }}
                                selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.id)}
                                selected={providerBatch.selectedIds.has(provider.id)}
                                onSelectChange={(checked) =>
                                  providerBatch.toggleSelect(provider.id, checked)
                                }
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
          <SessionManagerPanel
            tool="codex"
            expandNonce={sessionManagerExpandNonce}
            refreshNonce={sessionManagerRefreshNonce}
            sourceMode={sessionSourceMode}
            onSourceModeChange={handleSessionSourceModeChange}
            extra={(
              <Button
                type="link"
                size="small"
                style={{ fontSize: 12 }}
                icon={<SyncOutlined />}
                onClick={(event) => {
                  event.stopPropagation();
                  setHistorySyncModalOpen(true);
                }}
              >
                {t('codex.historySync.menu')}
              </Button>
            )}
          />
        </div>

        <div
          id="codex-memories"
          data-sidebar-section="true"
          data-sidebar-title={t('codex.memories.title')}
        >
          <Collapse
            style={{ marginBottom: 16 }}
            activeKey={memoriesListCollapsed ? [] : ['memories']}
            onChange={(keys) => setMemoriesListCollapsed(!keys.includes('memories'))}
            items={[
              {
                key: 'memories',
                label: (
                  <Text strong>
                    <FileTextOutlined style={{ marginRight: 8 }} />
                    {t('codex.memories.title')}
                  </Text>
                ),
                children: <CodexMemoriesPanel refreshToken={pluginPanelRefreshToken} />,
              },
            ]}
          />
        </div>

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
          gatewayCliKey="codex"
          useGateway={connectivityUsesGateway}
          diagnostics={connectivityInfo ? findDiagnosticsForProvider(favoriteProviders, 'codex', connectivityInfo.providerId) : undefined}
          onSaveDiagnostics={handleSaveConnectivityDiagnostics}
          removableModelIds={connectivityRemovableModelIds}
          onRemoveModels={handleRemoveConnectivityModels}
          onCancel={() => setConnectivityModalOpen(false)}
        />

        {modelModalProvider && (
          <CodexModelFormModal
            open={modelModalOpen}
            isEdit={Boolean(modelModalRowKey)}
            presetNpmType={getCodexProviderSdkType(modelModalProvider)}
            resolvePreset={resolveModelCatalogPreset}
            initialValues={modelModalInitialValues}
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
            sdkType={fetchModelsProviderInfo.sdkType}
            existingModelIds={fetchModelsProviderInfo.existingModelIds}
            priorityOwnedBy={['openai']}
            onCancel={() => {
              setFetchModelsModalOpen(false);
              setFetchModelsProviderId(null);
            }}
            onSuccess={handleFetchModelsApply}
          />
        )}

        <Drawer
          title={t('gateway.aggregate.title')}
          placement="right"
          width="min(92vw, 920px)"
          open={aggregateSettingsOpen}
          destroyOnHidden
          onClose={() => {
            aggregateSettingsRequestRef.current += 1;
            setAggregateSettingsOpen(false);
          }}
        >
          <GatewayAggregateSettings
            running={aggregateGatewayRunning}
            onTakeoverChange={handleAggregateTakeoverChange}
          />
        </Drawer>

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
          isLocalProvider={providers.some((provider) => provider.id === CODEX_LOCAL_PROVIDER_ID)}
          gatewaySaveLocked={gatewayTakeoverActive}
        />

        <RootDirectoryModal
          open={rootDirectoryModalOpen}
          {...getRootDirectoryModalProps(rootPathInfo)}
          onCancel={() => setRootDirectoryModalOpen(false)}
          onSubmit={handleSaveRootDirectory}
          onReset={handleResetRootDirectory}
        />

        <CodexHistorySyncModal
          open={historySyncModalOpen}
          sourceMode={sessionSourceMode}
          onCancel={() => setHistorySyncModalOpen(false)}
          onChanged={() => {
            setSessionManagerExpandNonce((current) => current + 1);
            setSessionManagerRefreshNonce((current) => current + 1);
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

        {ccSwitchAvailable && (
          <ImportFromCcSwitchModal
            open={ccSwitchImportModalOpen}
            appType="codex"
            existingProviderIds={providers
              .map((provider) => provider.sourceProviderId)
              .filter((id): id is string => Boolean(id))}
            onClose={() => setCcSwitchImportModalOpen(false)}
            onImport={handleImportFromCcSwitch}
          />
        )}

        {/* Preview Modal */}
        <CodexConfigPreviewModal
          open={previewModalOpen}
          onClose={() => setPreviewModalOpen(false)}
          title={t('codex.preview.currentConfigTitle')}
          data={previewData}
        />

        <ShareProviderModal
          open={shareProvider !== null}
          sourceApp="codex"
          provider={shareProvider}
          onClose={() => setShareProvider(null)}
        />

        <SidebarSettingsModal
          open={settingsModalOpen}
          onClose={() => setSettingsModalOpen(false)}
          sidebarVisible={!sidebarHidden}
          onSidebarVisibleChange={(visible) => setSidebarHidden('codex', !visible)}
        >
          <SettingsToggleRow
            title={t('codex.settings.preserveOfficialAuthOnSwitch')}
            hint={t('codex.settings.preserveOfficialAuthOnSwitchHint')}
            checked={codexPreserveOfficialAuthOnSwitch}
            loading={savingCodexAuthPreservation}
            onChange={(checked) => {
              void handleCodexAuthPreservationChange(checked);
            }}
          />
          <SettingsToggleRow
            title={t('codex.settings.unifiedSessionHistory')}
            hint={
              gatewayTakeoverActive
                ? t('codex.settings.unifiedSessionHistoryGatewayLocked')
                : t('codex.settings.unifiedSessionHistoryHint')
            }
            checked={codexUnifiedSessionHistoryEnabled}
            loading={savingCodexUnifiedHistory}
            disabled={gatewayTakeoverActive}
            onChange={(checked) => {
              handleCodexUnifiedHistoryChange(checked);
            }}
          />
        </SidebarSettingsModal>
        <Modal
          open={Boolean(officialAccountDetails)}
          title={t('codex.provider.officialAccountDetailsTitle')}
          onCancel={() => setOfficialAccountDetails(null)}
          footer={null}
          width={720}
        >
          {officialAccountDetails && (
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label={t('codex.provider.name')}>
                {officialAccountDetails.provider.name}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountLabel')}>
                {officialAccountDetails.account.email || officialAccountDetails.account.name}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.mode')}>
                {officialAccountDetails.account.id === CODEX_LOCAL_PROVIDER_ID
                  ? t('codex.provider.officialAccountLocalTag')
                  : t('codex.provider.officialAccountOauthTag')}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountPlanType')}>
                {officialAccountDetails.account.planType || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item
                label={t('codex.provider.officialAccountShortWindowUsage', {
                  label: officialAccountDetails.account.limitShortLabel || '5h',
                })}
              >
                {officialAccountDetails.account.limit5hText || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountWeeklyLimit')}>
                {officialAccountDetails.account.limitWeeklyText || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountMonthlyLimit')}>
                {officialAccountDetails.account.limitMonthlyText || ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountShortWindowResetAt')}>
                {formatResetMoment(officialAccountDetails.account.limit5hResetAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountWeeklyResetAt')}>
                {formatResetMoment(officialAccountDetails.account.limitWeeklyResetAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountMonthlyResetAt')}>
                {formatResetMoment(officialAccountDetails.account.limitMonthlyResetAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountResetCreditsLabel')}>
                {typeof officialAccountDetails.account.resetCreditsAvailable === 'number'
                  ? t('codex.provider.officialAccountResetCredits', {
                    count: officialAccountDetails.account.resetCreditsAvailable,
                  })
                  : ACCOUNT_DETAILS_EMPTY_VALUE}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountLastLimitRefreshAt')}>
                {formatDateTime(officialAccountDetails.account.lastLimitsFetchedAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountTokenExpiresAt')}>
                {formatUnixTimestamp(officialAccountDetails.account.tokenExpiresAt)}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountAccessToken')}>
                {renderTokenPreview(
                  maskTokenPreview('access', officialAccountDetails.account),
                  () => handleCopyOfficialAccountToken(
                    officialAccountDetails.provider,
                    officialAccountDetails.account,
                    'access',
                  ),
                )}
              </Descriptions.Item>
              <Descriptions.Item label={t('codex.provider.officialAccountRefreshToken')}>
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
                <Descriptions.Item label={t('codex.provider.officialAccountLastErrorLabel')}>
                  <Text type="danger">{officialAccountDetails.account.lastError}</Text>
                </Descriptions.Item>
              )}
            </Descriptions>
          )}
        </Modal>
        <CodexDeviceAuthModal
          authSession={codexDeviceAuthSession}
          onClose={() => setCodexDeviceAuthSession(null)}
          onCompleted={async () => {
            setCodexDeviceAuthSession(null);
            await loadConfig();
            await refreshTrayMenu();
          }}
        />
      </div>
    </SectionSidebarLayout>
  );
};

export default CodexPage;
