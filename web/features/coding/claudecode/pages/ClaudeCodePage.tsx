import React from 'react';
import { Typography, Button, Space, Empty, message, Modal, Spin, Collapse } from 'antd';
import { PlusOutlined, FolderOpenOutlined, AppstoreOutlined, SyncOutlined, ExclamationCircleOutlined, LinkOutlined, EyeOutlined, EllipsisOutlined, DatabaseOutlined, ImportOutlined, FileTextOutlined, ThunderboltOutlined, EditOutlined, MessageOutlined } from '@ant-design/icons';
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
  ClaudeCodeProvider,
  ConfigPathInfo,
  ClaudeProviderFormValues,
  ClaudeProviderInput,
  ImportConflictInfo,
  ImportConflictAction,
} from '@/types/claudecode';
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
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
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
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import ProviderConnectivityTestModal, {
  buildClaudeProviderConnectivityInfo,
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
  extractFavoriteProviderRawId,
  findDefaultTestModelIdForProvider,
  findDiagnosticsForProvider,
  getFavoriteProviderPayload,
  isFavoriteProviderForSource,
  mergeDiagnosticsIntoFavoriteProviders,
  type ClaudeFavoriteProviderPayload,
} from '@/features/coding/shared/favoriteProviders';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';

const { Title, Text, Link } = Typography;

function parseClaudeSettingsConfig(rawConfig: string) {
  try {
    return JSON.parse(rawConfig) as {
      env?: {
        ANTHROPIC_AUTH_TOKEN?: string;
        ANTHROPIC_API_KEY?: string;
        ANTHROPIC_BASE_URL?: string;
      };
      model?: string;
      haikuModel?: string;
      sonnetModel?: string;
      opusModel?: string;
      reasoningModel?: string;
    };
  } catch (error) {
    console.error('Failed to parse Claude settings config:', error);
    return {};
  }
}

function buildClaudeFavoriteProviderConfig(provider: ClaudeCodeProvider) {
  const settingsConfig = parseClaudeSettingsConfig(provider.settingsConfig);
  const apiKey =
    settingsConfig.env?.ANTHROPIC_AUTH_TOKEN?.trim() ||
    settingsConfig.env?.ANTHROPIC_API_KEY?.trim();
  const modelIds = Array.from(
    new Set(
      [
        settingsConfig.model,
        settingsConfig.haikuModel,
        settingsConfig.sonnetModel,
        settingsConfig.opusModel,
        settingsConfig.reasoningModel,
      ].filter((modelId): modelId is string => Boolean(modelId?.trim())),
    ),
  );

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

  if (Object.keys(envConfig).length > 0) {
    settingsConfigObj.env = envConfig;
  }

  if (values.model?.trim()) settingsConfigObj.model = values.model.trim();
  if (values.haikuModel?.trim()) settingsConfigObj.haikuModel = values.haikuModel.trim();
  if (values.sonnetModel?.trim()) settingsConfigObj.sonnetModel = values.sonnetModel.trim();
  if (values.opusModel?.trim()) settingsConfigObj.opusModel = values.opusModel.trim();
  if (values.reasoningModel?.trim()) {
    settingsConfigObj.reasoningModel = values.reasoningModel.trim();
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

  // 模态框状态
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<ClaudeCodeProvider | null>(null);
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
      title: t('claudecode.prompt.title'),
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

    checkAllApiHubAvailability();
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

    window.addEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
    return () => {
      window.removeEventListener(TRAY_CONFIG_REFRESH_EVENT, handleTrayConfigRefresh);
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
    loadConfig,
    getCommonConfig: getClaudeCommonConfig,
    saveCommonConfig: saveClaudeCommonConfig,
  });

  const handleSelectProvider = async (provider: ClaudeCodeProvider) => {
    try {
      await applyClaudeConfig(provider.id);
      message.success(t('claudecode.apply.success'));
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

  const handleImportFromOpenCode = () => {
    setEditingProvider(null);
    setIsCopyMode(false);
    setProviderModalMode('import');
    setProviderModalOpen(true);
  };

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

  const handleTestProvider = (provider: ClaudeCodeProvider) => {
    if (provider.category === 'official') {
      message.info(t('claudecode.provider.officialConnectivityHint'));
      return;
    }

    setConnectivityInfo(buildClaudeProviderConnectivityInfo(provider));
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
    if (providers.length === 0) {
      return;
    }

    const officialProviders = providers.filter((provider) => provider.category === 'official');
    const testableProviders = providers.filter((provider) => provider.category !== 'official');

    if (officialProviders.length > 0) {
      message.info(t('claudecode.provider.officialBatchSkipped', { count: officialProviders.length }));
    }

    if (testableProviders.length === 0) {
      setConnectivityStatuses({});
      return;
    }

    const targets = testableProviders.map((provider) => {

      const connectivityInfo = buildClaudeProviderConnectivityInfo(provider);
      let settingsConfig: {
        env?: {
          ANTHROPIC_BASE_URL?: string;
        };
      } = {};
      try {
        settingsConfig = JSON.parse(provider.settingsConfig || '{}') as typeof settingsConfig;
      } catch (error) {
        console.error('Failed to parse Claude provider settings config for batch test:', error);
      }
      const hasExplicitBaseUrl = Boolean(settingsConfig.env?.ANTHROPIC_BASE_URL?.trim());

      if (provider.category !== 'official' && !hasExplicitBaseUrl) {
        return {
          providerId: provider.id,
          errorMessage: t('common.baseUrlMissing'),
        };
      }

      return buildProviderConnectivityBatchTarget(connectivityInfo, {
        requireBaseUrl: false,
        requireApiKey: true,
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
  }, [providers, t, favoriteProviders]);

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
        sourceProviderId: values.sourceProviderId,
        notes: values.notes,
      };

      let savedProviderId = isLocalTemp ? '__local__' : '';
      let savedProvider: ClaudeCodeProvider | null = null;

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
        savedProvider = await createClaudeProvider(providerInput);
        savedProviderId = savedProvider.id;
      }

      try {
        const providerForFavorite: ClaudeCodeProvider = savedProvider || {
          id: savedProviderId,
          name: values.name,
          category: values.category,
          settingsConfig: providerInput.settingsConfig,
          extraSettingsConfig: providerInput.extraSettingsConfig || '{}',
          sourceProviderId: values.sourceProviderId,
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
        <div style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
            <div>
              <div style={{ marginBottom: 8 }}>
                <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                  {t('claudecode.title')}
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openUrl('https://code.claude.com/docs/en/settings#environment-variables');
                  }}
                >
                  <LinkOutlined /> {t('claudecode.viewDocs')}
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
              <Space>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('claudecode.configPath')}:
                </Text>
                <Text code style={{ fontSize: 12 }}>
                  {configPath || '~/.claude/settings.json'}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => setRootDirectoryModalOpen(true)}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('claudecode.rootPathSource.customize')}
                </Button>
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

        {/* Provider 列表 */}
        <div
          id="claudecode-providers"
          data-sidebar-section="true"
          data-sidebar-title={t('claudecode.provider.title')}
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
                    {t('claudecode.provider.title')}
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
                      {t('claudecode.commonConfigButton')}
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
                      {t('claudecode.addProvider')}
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
                      <div>{t('claudecode.pageHint')}</div>
                      <div>{t('claudecode.pageWarning')}</div>
                    </div>

                    {providers.length === 0 ? (
                      <Empty description={t('claudecode.emptyText')} style={{ marginTop: 40 }} />
                    ) : (
                      <DndContext
                        sensors={sensors}
                        collisionDetection={closestCenter}
                        modifiers={[restrictToVerticalAxis]}
                        onDragEnd={handleDragEnd}
                      >
                        <SortableContext
                          items={providers.map((p) => p.id)}
                          strategy={verticalListSortingStrategy}
                        >
                          <div>
                            {providers.map((provider) => (
                              <ClaudeProviderCard
                                key={provider.id}
                                provider={provider}
                                isApplied={provider.id === appliedProviderId}
                                onEdit={handleEditProvider}
                                onDelete={handleDeleteProvider}
                                onCopy={handleCopyProvider}
                                onTest={handleTestProvider}
                                onSelect={handleSelectProvider}
                                onToggleDisabled={handleToggleDisabled}
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
                          {t('claudecode.importFromOpenCode')}
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
          id="claudecode-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('claudecode.prompt.title')}
        >
          <GlobalPromptSettings
            key={`claudecode-prompt-${promptExpandNonce}`}
            translationKeyPrefix="claudecode.prompt"
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
          isLocalProvider={providers.some((provider) => provider.id === '__local__')}
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

        <ProviderConnectivityTestModal
          open={connectivityModalOpen}
          connectivityInfo={connectivityInfo}
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
