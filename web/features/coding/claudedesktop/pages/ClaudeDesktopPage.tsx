import React from 'react';
import { useProviderSharing } from '@/features/coding/shared/providerShare';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import { Typography, Button, Space, Empty, message, Modal, Spin, Collapse } from 'antd';
import {
  PlusOutlined,
  FolderOpenOutlined,
  DatabaseOutlined,
  SyncOutlined,
  ExclamationCircleOutlined,
  AppstoreOutlined,
  ImportOutlined,
  LinkOutlined,
  FileTextOutlined,
  MessageOutlined,
  EyeOutlined,
  EllipsisOutlined,
  CheckSquareOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
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
  ClaudeDesktopFormValues,
  ClaudeDesktopModelRoutes,
  ClaudeDesktopPathInfo,
  ClaudeDesktopProvider,
  ClaudeDesktopProviderInput,
  } from '@/types/claudedesktop';
import {
  applyClaudeDesktopProvider,
  createClaudeDesktopProvider,
  listClaudeDesktopAllApiHubProviders,
  resolveClaudeDesktopAllApiHubProviders,
  deleteClaudeDesktopProvider,
  getClaudeDesktopPaths,
  getClaudeDesktopPreview,
  importClaudeDesktopProvidersFromClaude,
  listClaudeDesktopProviders,
  reorderClaudeDesktopProviders,
  toggleClaudeDesktopProviderDisabled,
  updateClaudeDesktopProvider,
} from '@/services/claudeDesktopApi';
import { claudeDesktopPromptApi } from '@/services/claudeDesktopPromptApi';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
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
import { useRefreshStore, useSettingsStore } from '@/stores';
import {
  engageProxyGatewayFailover,
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { hasAllApiHubExtension, refreshTrayMenu } from '@/services/appApi';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import {
  GatewayFailoverButton,
  saveProviderWithGatewayReengage,
} from '@/features/coding/shared/gateway';
import {
  CUSTOM_PROVIDER_PROFILE_ID,
  toGatewayProviderProfileReference,
} from '@/features/coding/shared/gateway/providerProfiles';
import ClaudeDesktopProviderCard from '../components/ClaudeDesktopProviderCard';
import ClaudeDesktopProviderFormModal from '../components/ClaudeDesktopProviderFormModal';
import ClaudeDesktopCommonConfigModal from '../components/ClaudeDesktopCommonConfigModal';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import ProviderConnectivityTestModal, {
  type ProviderConnectivityInfo,
} from '@/features/coding/shared/providerConnectivity/ProviderConnectivityTestModal';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import ImportFromCcSwitchModal from '@/features/coding/shared/ccSwitch/ImportFromCcSwitchModal';
import ImportFromAllApiHubModalForTool from '@/features/coding/shared/allApiHub/ImportFromAllApiHubModalForTool';
import { mergeCustomHeadersIntoMeta } from '@/features/coding/shared/providerHeaders/customHeadersUtils';
import { mergeModelRewritesIntoMeta } from '@/features/coding/shared/providerModelRewrites/modelRewritesUtils';
import {
  buildFavoriteProviderOptions,
  buildFavoriteProviderStorageKey,
  extractFavoriteProviderRawId,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  type ClaudeDesktopFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import { upsertFavoriteProvider, type OpenCodeFavoriteProvider } from '@/services/opencodeApi';
import type { OpenCodeProvider } from '@/types/opencode';
import type { AllApiHubProviderItem } from '@/types/allApiHub';
import { hasCcSwitchDb, type CcSwitchProviderCandidate } from '@/services/ccSwitchApi';
import {
  getClaudeConfiguredModelIds,
  hasClaudeOneMMarker,
  parseClaudeSettingsConfig,
  stripClaudeOneMMarker,
} from '../../claudecode/utils/claudeModelConfig';
import styles from './ClaudeDesktopPage.module.less';

const { Title, Text, Link } = Typography;

const DEFAULT_BASE_URL = 'https://api.anthropic.com/v1';

function normalizeBaseUrl(baseUrl?: string): string {
  const trimmed = baseUrl?.trim();
  if (!trimmed) {
    return DEFAULT_BASE_URL;
  }
  return trimmed.replace(/\/+$/, '') + '/v1';
}

function buildProviderSettingsConfig(values: ClaudeDesktopFormValues): string {
  // Official mode: an empty env makes apply restore the official 1P profile.
  if (values.category === 'official') {
    return '{"env":{}}';
  }
  // Claude Desktop 的模型路由以 meta.claudeDesktopModelRoutes 为单一事实源
  // （见 buildProviderMeta）：它驱动 3P profile 的 inferenceModels 菜单，网关
  // 运行时也从它构建上游模型映射。env 仅承载网关凭据。
  const env: Record<string, string> = {};
  if (values.baseUrl?.trim()) {
    env.ANTHROPIC_BASE_URL = values.baseUrl.trim();
  }
  if (values.apiKey?.trim()) {
    env.ANTHROPIC_AUTH_TOKEN = values.apiKey.trim();
  }
  return JSON.stringify({ env }, null, 2);
}

/** claude-safe route_id per role, mirroring cc-switch CLAUDE_DESKTOP_ROLE_ROUTE_IDS. */
const CLAUDE_DESKTOP_ROLE_ROUTE_IDS: Record<string, string> = {
  sonnet: 'claude-sonnet-5',
  opus: 'claude-opus-5',
  fable: 'claude-fable-5',
  haiku: 'claude-haiku-4-5',
};
const CLAUDE_DESKTOP_ROLE_ROUTE_ORDER: Array<'sonnet' | 'opus' | 'fable' | 'haiku'> = [
  'sonnet',
  'opus',
  'fable',
  'haiku',
];

/** Claude Desktop `anthropicFamilyTier` legal values (per official config schema). */
const TIER_ALIAS_VALUES = ['haiku', 'sonnet', 'opus', 'fable', 'mythos'] as const;
/** Normalize a tier alias input to a legal `anthropicFamilyTier` value, or undefined. */
function normalizeTierAlias(value?: string): string | undefined {
  const trimmed = value?.trim().toLowerCase();
  if (!trimmed) {
    return undefined;
  }
  return TIER_ALIAS_VALUES.includes(trimmed as (typeof TIER_ALIAS_VALUES)[number])
    ? trimmed
    : undefined;
}

/** Build `meta.claudeDesktopModelRoutes` from the form's role model mapping.
 * route_id is the claude-safe name Claude Desktop accepts; `model` is the real
 * upstream model; `labelOverride` is the in-app menu display name. */
function buildClaudeDesktopModelRoutes(
  values: ClaudeDesktopFormValues,
): ClaudeDesktopModelRoutes {
  const routes: ClaudeDesktopModelRoutes = {};
  for (const role of CLAUDE_DESKTOP_ROLE_ROUTE_ORDER) {
    const model =
      role === 'sonnet'
        ? values.sonnetModel
        : role === 'opus'
          ? values.opusModel
          : role === 'fable'
            ? values.fableModel
            : values.haikuModel;
    const labelOverride =
      role === 'sonnet'
        ? values.sonnetModelName
        : role === 'opus'
          ? values.opusModelName
          : role === 'fable'
            ? values.fableModelName
            : values.haikuModelName;
    const tierAlias =
      role === 'sonnet'
        ? values.sonnetTierAlias
        : role === 'opus'
          ? values.opusTierAlias
          : role === 'fable'
            ? values.fableTierAlias
            : values.haikuTierAlias;
    const rawModel = model?.trim();
    if (!rawModel) {
      continue;
    }
    // The [1m] marker on the model string expresses the user's 1M intent (set by
    // the form checkbox). Strip it from the stored upstream `model` and carry the
    // intent as `supports1m`; config_writer writes `supports1m: true` into the
    // profile's inferenceModels. Storing the marker in `model` would also make
    // direct mode reject it as a model mapping (upstream != route_id).
    const modelBase = stripClaudeOneMMarker(rawModel).trim();
    if (!modelBase) {
      continue;
    }
    const normalizedTierAlias = normalizeTierAlias(tierAlias);
    routes[CLAUDE_DESKTOP_ROLE_ROUTE_IDS[role]] = {
      model: modelBase,
      ...(labelOverride?.trim() ? { labelOverride: labelOverride.trim() } : {}),
      supports1m: hasClaudeOneMMarker(rawModel),
      ...(normalizedTierAlias ? { tierAlias: normalizedTierAlias } : {}),
    };
  }
  return routes;
}

function buildProviderMeta(
  values: ClaudeDesktopFormValues,
  existingMeta?: ClaudeDesktopProvider['meta'],
): ClaudeDesktopProvider['meta'] {
  const meta = { ...(existingMeta || {}) };
  // Drop the legacy per-provider gateway-mode field; model routing now flows
  // through claudeDesktopModelRoutes built from the form role mapping below.
  delete meta.claudeDesktopMode;

  if (values.category === 'official') {
    delete meta.claudeDesktopModelRoutes;
    delete meta.apiFormat;
    delete meta.gatewayProfile;
    delete meta.customHeaders;
    delete meta.custom_headers;
    delete meta.customUserAgent;
    delete meta.custom_user_agent;
    delete meta.modelRewrites;
    delete meta.model_rewrites;
    return meta;
  }

  const routes = buildClaudeDesktopModelRoutes(values);
  if (Object.keys(routes).length > 0) {
    meta.claudeDesktopModelRoutes = routes;
  } else {
    delete meta.claudeDesktopModelRoutes;
  }

  // Persist the channel preset reference + upstream protocol when a gateway
  // endpoint is selected (custom uses nothing here).
  const endpointSelected =
    values.providerProfileId &&
    values.providerProfileId !== CUSTOM_PROVIDER_PROFILE_ID &&
    values.providerEndpointId;
  if (endpointSelected) {
    meta.gatewayProfile = toGatewayProviderProfileReference(
      'claude_desktop',
      values.providerProfileId!,
      values.providerEndpointId!,
    );
  } else {
    delete meta.gatewayProfile;
  }
  if (values.apiFormat) {
    meta.apiFormat = values.apiFormat;
  } else {
    delete meta.apiFormat;
  }

  return mergeModelRewritesIntoMeta(
    mergeCustomHeadersIntoMeta(meta, values.customHeaders ?? { enabled: false, headers: [] }),
    values.modelRewrites ?? { enabled: false, rewrites: [] },
  );
}

function buildDesktopProviderConnectivityInfo(provider: ClaudeDesktopProvider): ProviderConnectivityInfo {
  const settingsConfig = parseClaudeSettingsConfig(provider.settingsConfig);
  const apiKey =
    settingsConfig.env?.ANTHROPIC_AUTH_TOKEN?.trim() ||
    settingsConfig.env?.ANTHROPIC_API_KEY?.trim();
  const routes = provider.meta?.claudeDesktopModelRoutes;
  const routeModelIds = routes
    ? [...new Set(Object.values(routes).map((route) => route.model.trim()).filter(Boolean))]
    : [];
  // Imported-from-Claude-Code rows carry their role models in env
  // (ANTHROPIC_DEFAULT_*_MODEL) until re-saved; fall back to those.
  const modelIds = routeModelIds.length > 0
    ? routeModelIds
    : getClaudeConfiguredModelIds(settingsConfig, { stripOneMMarker: true });

  return {
    providerId: provider.id,
    providerName: provider.name,
    providerConfig: {
      npm: '@ai-sdk/anthropic',
      name: provider.name,
      options: {
        baseURL: normalizeBaseUrl(settingsConfig.env?.ANTHROPIC_BASE_URL),
        ...(apiKey ? { apiKey } : {}),
      },
      models: Object.fromEntries(modelIds.map((modelId) => [modelId, {}])),
    },
    modelIds,
  };
}

/** Build the OpenCode-provider envelope that wraps a Claude Desktop favorite, embedding a
 *  `ClaudeDesktopFavoriteProviderPayload` so the row can be fully replayed on re-import. */
function buildDesktopFavoriteProviderConfig(provider: ClaudeDesktopProvider): OpenCodeProvider {
  const { providerConfig } = buildDesktopProviderConnectivityInfo(provider);
  const payload: ClaudeDesktopFavoriteProviderPayload = {
    name: provider.name,
    category: provider.category,
    settingsConfig: provider.settingsConfig,
    ...(provider.sourceProviderId ? { sourceProviderId: provider.sourceProviderId } : {}),
    ...(provider.websiteUrl ? { websiteUrl: provider.websiteUrl } : {}),
    ...(provider.notes ? { notes: provider.notes } : {}),
    ...(provider.icon ? { icon: provider.icon } : {}),
    ...(provider.iconColor ? { iconColor: provider.iconColor } : {}),
    ...(provider.sortIndex != null ? { sortIndex: provider.sortIndex } : {}),
    ...(provider.meta ? { meta: provider.meta } : {}),
  };
  return buildFavoriteProviderOptions(providerConfig, payload);
}

const ClaudeDesktopPage: React.FC = () => {
  const { t } = useTranslation();
  const { shareProvider, shareModal } = useProviderSharing('claudedesktop', () => loadConfig());
  const { claudeProviderRefreshKey } = useRefreshStore();
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const [loading, setLoading] = React.useState(false);
  const [pathInfo, setPathInfo] = React.useState<ClaudeDesktopPathInfo | null>(null);
  const [providers, setProviders] = React.useState<ClaudeDesktopProvider[]>([]);
  const [appliedProviderId, setAppliedProviderId] = React.useState<string>('');
  const [gatewayCliStatus, setGatewayCliStatus] = React.useState<GatewayCliTakeoverStatus | null>(null);
  const gatewayTakeoverActive = Boolean(gatewayCliStatus?.can_restore_direct);

  // Modal states
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<ClaudeDesktopProvider | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [commonConfigModalOpen, setCommonConfigModalOpen] = React.useState(false);
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityInfo, setConnectivityInfo] = React.useState<ProviderConnectivityInfo | null>(null);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [ccSwitchAvailable, setCcSwitchAvailable] = React.useState(false);
  const [ccSwitchImportModalOpen, setCcSwitchImportModalOpen] = React.useState(false);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewData] = React.useState<unknown>(null);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);

  const sidebarHidden = sidebarHiddenByPage.claudedesktop;

  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(
    () => [
      {
        id: 'claudedesktop-providers',
        title: '供应商列表',
        order: 1,
      },
      {
        id: 'claudedesktop-global-prompt',
        title: '全局提示词',
        order: 2,
      },
      {
        id: 'claudedesktop-session-manager',
        title: '会话管理',
        order: 3,
      },
    ],
    [],
  );

  const loadConfig = React.useCallback(
    async (silent = false) => {
      setLoading(true);
      try {
        const [paths, providerList] = await Promise.all([
          getClaudeDesktopPaths(),
          listClaudeDesktopProviders(),
        ]);
        setPathInfo(paths);
        setProviders(providerList);

        const applied = providerList.find((p) => p.isApplied);
        setAppliedProviderId(applied?.id || '');
      } catch (error) {
        console.error('Failed to load Claude Desktop config:', error);
        if (!silent) {
          message.error(t('common.error'));
        }
      } finally {
        setLoading(false);
      }
    },
    [t],
  );

  const loadConfigRef = React.useRef(loadConfig);
  loadConfigRef.current = loadConfig;

  const existingFavoriteProviderIds = React.useMemo(
    () => providers.map((provider) => buildFavoriteProviderStorageKey('claudedesktop', provider.id)),
    [providers],
  );

  /** Persist a Claude Desktop provider into the shared favorite-provider history. Errors are
   *  swallowed so a favorite write failure never blocks the primary save/import flow. */
  const upsertDesktopFavoriteProvider = React.useCallback((provider: ClaudeDesktopProvider) => {
    return upsertFavoriteProvider(
      buildFavoriteProviderStorageKey('claudedesktop', provider.id),
      buildDesktopFavoriteProviderConfig(provider),
    ).catch((error) => {
      console.error('Failed to save Claude Desktop favorite provider:', error);
    });
  }, []);

  React.useEffect(() => {
    void claudeProviderRefreshKey;
    void loadConfigRef.current();
  }, [claudeProviderRefreshKey]);

  React.useEffect(() => {
    const checkCcSwitch = async () => {
      try {
        setCcSwitchAvailable(await hasCcSwitchDb());
      } catch {
        setCcSwitchAvailable(false);
      }
    };
    void checkCcSwitch();

    const checkAllApiHub = async () => {
      try {
        setAllApiHubAvailable(await hasAllApiHubExtension());
      } catch {
        setAllApiHubAvailable(false);
      }
    };
    void checkAllApiHub();

    const handleTrayConfigRefresh = (event: Event) => {
      event.preventDefault();
      void loadConfigRef.current(true);
    };
    window.addEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    return () => {
      window.removeEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    };
  }, []);

  const handlePreviewCurrentConfig = async () => {
    try {
      setPreviewData(await getClaudeDesktopPreview());
      setPreviewModalOpen(true);
    } catch (error) {
      console.error('Failed to preview config:', error);
      message.error(t('common.error'));
    }
  };

  const handleRefreshPage = () => {
    void loadConfig();
  };

  const handleOpenFolder = async () => {
    const targetPath = pathInfo?.normalConfigPath || pathInfo?.profilePath || pathInfo?.configLibraryPath;
    if (!targetPath) {
      message.info('未找到 Claude Desktop 配置文件路径');
      return;
    }
    try {
      // Try to reveal the exact file/dir in the file manager.
      await revealItemInDir(targetPath);
    } catch (error) {
      console.error('Failed to reveal path:', error);
      // Claude Desktop may never have been initialised, so the target file/dir
      // does not exist yet. Walk up to the nearest existing ancestor and open
      // that folder instead (mirrors the Claude Code page behaviour).
      try {
        const parentDir = targetPath.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (secondError) {
        console.error('Failed to open folder:', secondError);
        message.error(t('common.error'));
      }
    }
  };

  const { sortMode, setSortMode, lastUsedAt, noteProviderUsed } = useProviderListSort('claudedesktop');
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
    (provider: ClaudeDesktopProvider) => Boolean(provider.id),
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
            buildFavoriteProviderStorageKey('claudedesktop', provider.id),
            buildDesktopFavoriteProviderConfig(provider),
          ),
          (provider) => t('common.batch.backupFailed', { name: provider.name }),
        );
        for (const provider of providersToDelete) {
          await deleteClaudeDesktopProvider(provider.id);
        }
        await loadConfig();
        await refreshTrayMenu();
        message.success(t('common.success'));
        return true;
      } catch (error) {
        console.error('Failed to batch delete claudedesktop providers:', error);
        message.error(error instanceof Error ? error.message : String(error));
        await loadConfig();
        return false;
      }
    },
    [providers, canBatchDeleteProvider, loadConfig, t],
  );
  // All visible providers are deletable in Claude Desktop (no __local__ preset).
  const batchSelectableIds = React.useMemo(
    () => visibleProviders.filter(canBatchDeleteProvider).map((provider) => provider.id),
    [visibleProviders, canBatchDeleteProvider],
  );
  const providerBatch = useProviderBatchSelection({
    allIds: batchSelectableIds,
    onBatchDelete: handleBatchDeleteProviders,
  });
  const providerBatchDragDisabled = providerDragDisabled || providerBatch.selectionMode;

  const handleSelectProvider = async (provider: ClaudeDesktopProvider) => {
    try {
      await applyClaudeDesktopProvider(provider.id);
      message.success(t('claudecode.apply.success'));
      noteProviderUsed(provider.id);
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      console.error('Failed to apply Claude Desktop provider:', error);
      message.error(errorMsg || t('common.error'));
    }
  };

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

    const oldProviders = [...providers];
    const newProviders = arrayMove(providers, oldIndex, newIndex);
    setProviders(newProviders);
    try {
      await reorderClaudeDesktopProviders(newProviders.map((p) => p.id));
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to reorder Claude Desktop providers:', error);
      setProviders(oldProviders);
      message.error(t('common.error'));
    }
  };

  const handleToggleDisabled = async (provider: ClaudeDesktopProvider, isDisabled: boolean) => {
    try {
      await toggleClaudeDesktopProviderDisabled(provider.id, isDisabled);
      message.success(isDisabled ? t('claudecode.providerDisabled') : t('claudecode.providerEnabled'));
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle Claude Desktop provider disabled status:', error);
      message.error(t('common.error'));
    }
  };

  const handleAddProvider = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: ClaudeDesktopProvider) => {
    setEditingProvider(provider);
    setIsCopyMode(false);
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (provider: ClaudeDesktopProvider) => {
    setEditingProvider({
      ...provider,
      id: `${provider.id}_copy`,
      name: `${provider.name}_copy`,
      isApplied: false,
      isDisabled: false,
    });
    setIsCopyMode(true);
    setProviderModalOpen(true);
  };

  const handleTestProvider = (provider: ClaudeDesktopProvider) => {
    setConnectivityInfo(buildDesktopProviderConnectivityInfo(provider));
    setConnectivityModalOpen(true);
  };

  const handleDeleteProvider = (provider: ClaudeDesktopProvider) => {
    Modal.confirm({
      title: t('claudecode.provider.confirmDelete', { name: provider.name }),
      icon: <ExclamationCircleOutlined />,
      onOk: async () => {
        try {
          // Back up the provider into the favorite-provider history before deletion
          // so it can still be re-imported later from "导入我使用过的供应商".
          try {
            await upsertFavoriteProvider(
              buildFavoriteProviderStorageKey('claudedesktop', provider.id),
              buildDesktopFavoriteProviderConfig(provider),
            );
          } catch (error) {
            console.error('Failed to preserve Claude Desktop favorite provider before deletion:', error);
          }
          await deleteClaudeDesktopProvider(provider.id);
          await loadConfig();
          await refreshTrayMenu();
          message.success(t('common.success'));
        } catch (error) {
          console.error('Failed to delete Claude Desktop provider:', error);
          message.error(t('common.error'));
        }
      },
    });
  };

  const handleProviderSubmit = async (values: ClaudeDesktopFormValues) => {
    try {
      let savedProvider: ClaudeDesktopProvider | null = null;
      const gatewayModeBeforeSave = gatewayCliStatus?.mode;
      const shouldReengageGatewayProxy =
        Boolean(editingProvider && !isCopyMode && editingProvider.isApplied) &&
        (gatewayModeBeforeSave === 'single' || gatewayModeBeforeSave === 'failover');

      await saveProviderWithGatewayReengage({
        gatewayMode: shouldReengageGatewayProxy ? gatewayModeBeforeSave : null,
        restoreDirect: () => restoreProxyGatewayCliDirect('claude_desktop'),
        engageSingle: () => engageProxyGatewaySingle('claude_desktop', savedProvider?.id || ''),
        engageFailover: () => engageProxyGatewayFailover('claude_desktop'),
        onGatewayStatusChange: setGatewayCliStatus,
        saveProvider: async () => {
          const category = values.category || editingProvider?.category || 'custom';
          if (isCopyMode || !editingProvider) {
            const providerInput: ClaudeDesktopProviderInput = {
              name: values.name,
              category,
              settingsConfig: buildProviderSettingsConfig(values),
              meta: buildProviderMeta(values, editingProvider?.meta),
              notes: values.notes || undefined,
            };
            savedProvider = await createClaudeDesktopProvider(providerInput);
          } else if (editingProvider) {
            const payload: ClaudeDesktopProvider = {
              ...editingProvider,
              name: values.name,
              category,
              settingsConfig: buildProviderSettingsConfig(values),
              meta: buildProviderMeta(values, editingProvider.meta),
              notes: values.notes,
            };
            savedProvider = await updateClaudeDesktopProvider(payload);
          }
        },
      });

      message.success(t('common.success'));
      setProviderModalOpen(false);
      setEditingProvider(null);
      setIsCopyMode(false);
      if (savedProvider) {
        void upsertDesktopFavoriteProvider(savedProvider);
      }
      await loadConfig();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save Claude Desktop provider:', error);
      message.error(t('common.error'));
      throw error;
    }
  };

  const handleImportFromClaude = async () => {
    try {
      const beforeIds = new Set(providers.map((provider) => provider.id));
      const count = await importClaudeDesktopProvidersFromClaude();
      if (count > 0) {
        message.success(`已从 Claude Code 导入 ${count} 个供应商`);
      } else {
        message.info('没有可导入的新供应商');
      }
      await loadConfig();
      await refreshTrayMenu();
      // The backend batch-creates providers directly and returns only a count, so re-fetch the
      // list and backfill the favorite history for any newly created rows.
      if (count > 0) {
        try {
          const afterProviders = await listClaudeDesktopProviders();
          for (const provider of afterProviders.filter((p) => !beforeIds.has(p.id))) {
            void upsertDesktopFavoriteProvider(provider);
          }
        } catch (error) {
          console.error('Failed to backfill Claude Desktop favorites after Claude import:', error);
        }
      }
    } catch (error) {
      console.error('Failed to import Claude Desktop providers from Claude:', error);
      message.error(t('common.error'));
    }
  };

  const handleImportFromCcSwitch = React.useCallback(
    async (imported: CcSwitchProviderCandidate[]) => {
      const existingSourceIds = new Set(
        providers.map((provider) => provider.sourceProviderId).filter(Boolean),
      );
      const toImport = imported.filter(
        (candidate) => candidate.sourceProviderId && !existingSourceIds.has(candidate.sourceProviderId),
      );

      let ok = 0;
      let fail = 0;
      for (const candidate of toImport) {
        try {
          const settingsConfig =
            typeof candidate.settingsConfig === 'string'
              ? candidate.settingsConfig
              : JSON.stringify(candidate.settingsConfig);
          const created = await createClaudeDesktopProvider({
            name: candidate.name,
            category: candidate.normalizedCategory || 'custom',
            settingsConfig,
            sourceProviderId: candidate.sourceProviderId,
            websiteUrl: candidate.websiteUrl,
            notes: candidate.notes,
            icon: candidate.icon,
            iconColor: candidate.iconColor,
          });
          void upsertDesktopFavoriteProvider(created);
          ok += 1;
        } catch (error) {
          console.error('Failed to import Claude Desktop provider from CC Switch:', candidate.name, error);
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
    },
    [providers, loadConfig, t],
  );

  const handleImportFromAllApiHub = React.useCallback(
    async (imported: AllApiHubProviderItem[]) => {
      const existingSourceIds = new Set(
        providers.map((provider) => provider.sourceProviderId).filter(Boolean),
      );
      const toImport = imported.filter((item) => !existingSourceIds.has(item.providerId));

      let ok = 0;
      let fail = 0;
      for (const item of toImport) {
        try {
          const created = await createClaudeDesktopProvider({
            name: item.name,
            category: 'custom',
            settingsConfig: JSON.stringify(item.config),
            sourceProviderId: item.providerId,
          });
          void upsertDesktopFavoriteProvider(created);
          ok += 1;
        } catch (error) {
          console.error('Failed to import All API Hub provider:', item.name, error);
          fail += 1;
        }
      }

      setAllApiHubImportModalOpen(false);
      if (ok > 0 && fail === 0) {
        message.success(t('common.allApiHub.importSuccess', { count: ok }));
      } else if (ok > 0 && fail > 0) {
        message.warning(t('common.allApiHub.importPartial', { ok, fail }));
      } else if (fail > 0) {
        message.error(t('common.error'));
      }

      await loadConfig();
      await refreshTrayMenu();
    },
    [providers, loadConfig, t],
  );

  const handleImportFavoriteProviders = React.useCallback(
    async (providersToImport: OpenCodeFavoriteProvider[]) => {
      const existingIds = new Set(providers.map((provider) => provider.id));
      let importedCount = 0;
      for (const favoriteProvider of providersToImport) {
        const payload = getFavoriteProviderPayload<ClaudeDesktopFavoriteProviderPayload>(favoriteProvider);
        if (!payload) {
          continue;
        }
        // Avoid colliding with an existing provider; favorite rows share the same id space.
        const sourceFavoriteId = extractFavoriteProviderRawId('claudedesktop', favoriteProvider.providerId);
        if (existingIds.has(sourceFavoriteId)) {
          continue;
        }
        try {
          const created = await createClaudeDesktopProvider({
            name: payload.name,
            category: payload.category,
            settingsConfig: payload.settingsConfig,
            ...(payload.sourceProviderId ? { sourceProviderId: payload.sourceProviderId } : {}),
            ...(payload.websiteUrl ? { websiteUrl: payload.websiteUrl } : {}),
            ...(payload.notes ? { notes: payload.notes } : {}),
            ...(payload.icon ? { icon: payload.icon } : {}),
            ...(payload.iconColor ? { iconColor: payload.iconColor } : {}),
            ...(payload.sortIndex != null ? { sortIndex: payload.sortIndex } : {}),
            ...(payload.meta ? { meta: payload.meta } : {}),
          });
          existingIds.add(created.id);
          importedCount += 1;
        } catch (error) {
          console.error('Failed to import Claude Desktop favorite provider:', payload.name, error);
        }
      }

      if (importedCount > 0) {
        setImportModalOpen(false);
        message.success(t('opencode.provider.importSuccess', { count: importedCount }));
        await loadConfig();
        await refreshTrayMenu();
      }
    },
    [providers, loadConfig, t],
  );

  return (
    <SectionSidebarLayout
      sidebarTitle="Claude Desktop"
      sidebarHidden={sidebarHidden}
      sections={sidebarSections}
      getIcon={(id) => {
        switch (id) {
          case 'claudedesktop-providers':
            return <DatabaseOutlined />;
          case 'claudedesktop-global-prompt':
            return <FileTextOutlined />;
          case 'claudedesktop-session-manager':
            return <MessageOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'claudedesktop-providers':
            setProviderListCollapsed(false);
            break;
          case 'claudedesktop-global-prompt':
            setPromptExpandNonce((value) => value + 1);
            break;
          case 'claudedesktop-session-manager':
            setSessionManagerExpandNonce((value) => value + 1);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        <div style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
            <div>
              <div style={{ marginBottom: 8 }}>
                <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                  Claude Desktop
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openUrl('https://support.anthropic.com/en/collections/8485703-claude-desktop');
                  }}
                >
                  <LinkOutlined /> 官方文档
                </Link>
                <Link
                  type="secondary"
                  style={{ fontSize: 12, marginLeft: 16 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    void handlePreviewCurrentConfig();
                  }}
                >
                  <EyeOutlined /> 预览配置
                </Link>
              </div>
              <Space size={12} wrap>
                {pathInfo && !pathInfo.supported ? (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {pathInfo.message || '当前平台不支持 Claude Desktop 3P 配置管理'}
                  </Text>
                ) : (
                  <Space size={4}>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      配置文件路径:
                    </Text>
                    <Text code style={{ fontSize: 12 }}>
                      {pathInfo?.normalConfigPath || ''}
                    </Text>
                  </Space>
                )}
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenFolder}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('claudecode.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<SyncOutlined />}
                  onClick={handleRefreshPage}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('claudecode.refreshConfig')}
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

        <div
          id="claudedesktop-providers"
          data-sidebar-section="true"
          data-sidebar-title="供应商"
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
                      供应商列表
                    </Text>
                    <GatewayFailoverButton
                      cliKey="claude_desktop"
                      status={gatewayCliStatus}
                      onStatusChange={setGatewayCliStatus}
                    />
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
                      icon={<AppstoreOutlined />}
                      onClick={(e) => {
                        e.stopPropagation();
                        setCommonConfigModalOpen(true);
                      }}
                    >
                      通用配置
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
                      新增供应商
                    </Button>
                  </Space>
                ),
                children: (
                  <Spin spinning={loading}>
                    <div className={styles.pageHint}>
                      <div>
                        可添加多套供应商配置并通过「应用」或系统托盘快捷菜单快速切换。「通用配置」的内容为所有供应商共享，应用时会自动与供应商配置合并写入。
                      </div>
                      <div>注意：配置存储在应用数据库中，请勿手动修改本地配置文件，会被下次「应用」操作覆盖。</div>
                    </div>

                    {providers.length === 0 ? (
                      <Empty
                        description="尚未配置供应商，点击上方“新增供应商”或从下方导入"
                        style={{ marginTop: 40 }}
                      />
                    ) : visibleProviders.length === 0 ? (
                      <ProviderSearchEmpty />
                    ) : (
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
                              <ClaudeDesktopProviderCard
                                key={provider.id}
                                provider={provider}
                                isApplied={provider.id === appliedProviderId && provider.isApplied}
                                onEdit={handleEditProvider}
                                onDelete={handleDeleteProvider}
                                onCopy={handleCopyProvider}
                                onShare={shareProvider}
                                onTest={handleTestProvider}
                                onSelect={handleSelectProvider}
                                onToggleDisabled={handleToggleDisabled}
                                selectable={providerBatch.selectionMode && providerBatch.isSelectable(provider.id)}
                                selected={providerBatch.selectedIds.has(provider.id)}
                                onSelectChange={(checked) =>
                                  providerBatch.toggleSelect(provider.id, checked)
                                }
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
                        <Button
                          type="dashed"
                          icon={<ImportOutlined />}
                          onClick={handleImportFromClaude}
                        >
                          从 Claude Code 导入
                        </Button>
                      </Space>
                    </div>
                  </Spin>
                ),
              },
            ]}
          />
        </div>

        <div
          id="claudedesktop-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title="全局提示词"
        >
          <GlobalPromptSettings
            key={`claudedesktop-prompt-${promptExpandNonce}`}
            translationKeyPrefix="claudedesktop.prompt"
            service={claudeDesktopPromptApi}
            collapseKey="claudedesktop-prompt"
            refreshKey={claudeProviderRefreshKey}
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <div
          id="claudedesktop-session-manager"
          data-sidebar-section="true"
          data-sidebar-title="会话管理"
        >
          <SessionManagerPanel
            tool="claudedesktop"
            expandNonce={sessionManagerExpandNonce}
          />
        </div>
      </div>

      {providerModalOpen && (
        <ClaudeDesktopProviderFormModal
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
      )}

      <ClaudeDesktopCommonConfigModal
        open={commonConfigModalOpen}
        onCancel={() => setCommonConfigModalOpen(false)}
        onSuccess={() => message.success(t('common.success'))}
      />

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

      <ImportProviderModal
        open={importModalOpen}
        onClose={() => setImportModalOpen(false)}
        onImport={handleImportFavoriteProviders}
        existingProviderIds={existingFavoriteProviderIds}
        providerFilter={(provider) => isFavoriteProviderForSource('claudedesktop', provider)}
      />

      {allApiHubAvailable && (
        <ImportFromAllApiHubModalForTool
          open={allApiHubImportModalOpen}
          existingProviderIds={providers
            .map((provider) => provider.sourceProviderId)
            .filter((id): id is string => Boolean(id))}
          onCancel={() => setAllApiHubImportModalOpen(false)}
          onImport={handleImportFromAllApiHub}
          listProviders={listClaudeDesktopAllApiHubProviders}
          resolveProviders={resolveClaudeDesktopAllApiHubProviders}
          warnOnNonAnthropicProtocol
        />
      )}

      <ProviderConnectivityTestModal
        open={connectivityModalOpen}
        connectivityInfo={connectivityInfo}
        gatewayCliKey="claude_desktop"
        onCancel={() => {
          setConnectivityModalOpen(false);
          setConnectivityInfo(null);
        }}
      />

      <FileConfigPreviewModal
        open={previewModalOpen}
        onClose={() => setPreviewModalOpen(false)}
        title="Claude Desktop 配置预览"
        files={[
          {
            key: 'normalConfig',
            label: 'claude_desktop_config.json (normal)',
            content: (previewData as Record<string, unknown> | null)?.normalConfig,
          },
          {
            key: 'threepConfig',
            label: 'claude_desktop_config.json (3P)',
            content: (previewData as Record<string, unknown> | null)?.threepConfig,
          },
          {
            key: 'profile',
            label: 'profile.json',
            content: (previewData as Record<string, unknown> | null)?.profile,
          },
          {
            key: 'meta',
            label: '_meta.json',
            content: (previewData as Record<string, unknown> | null)?.meta,
          },
        ]}
      />

      <SidebarSettingsModal
        open={settingsModalOpen}
        onClose={() => setSettingsModalOpen(false)}
        sidebarVisible={!sidebarHidden}
        onSidebarVisibleChange={async (visible) => {
          await setSidebarHidden('claudedesktop', !visible);
        }}
      />

      {shareModal}
    </SectionSidebarLayout>
  );
};

export default ClaudeDesktopPage;
