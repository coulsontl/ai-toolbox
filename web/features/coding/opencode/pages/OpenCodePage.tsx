import React from 'react';
import { Button, Empty, Space, Typography, message, Spin, Select, Collapse, Tag, Form, Tooltip } from 'antd';
import { PlusOutlined, FolderOpenOutlined, LinkOutlined, EyeOutlined, EditOutlined, EnvironmentOutlined, CloudDownloadOutlined, ReloadOutlined, FileOutlined, ImportOutlined, ApiOutlined, SafetyCertificateOutlined, RobotOutlined, ToolOutlined, DatabaseOutlined } from '@ant-design/icons';

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
  DragEndEvent,
} from '@dnd-kit/core';
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import { readOpenCodeConfigWithResult, saveOpenCodeConfig, getOpenCodeConfigPathInfo, getOpenCodeUnifiedModels, getOpenCodeAuthProviders, getOpenCodeAuthConfigPath, listFavoriteProviders, upsertFavoriteProvider, buildModelVariantsMap, type ConfigPathInfo, type UnifiedModelOption, type GetAuthProvidersResponse, type OpenCodeFavoriteProvider, type OpenCodeDiagnosticsConfig } from '@/services/opencodeApi';
import { listOhMyOpenCodeConfigs, applyOhMyOpenCodeConfig } from '@/services/ohMyOpenCodeApi';
import { listOhMyOpenCodeSlimConfigs } from '@/services/ohMyOpenCodeSlimApi';
import { refreshTrayMenu } from '@/services/appApi';
import type { OpenCodeConfig, OpenCodeProvider, OpenCodeModel } from '@/types/opencode';
import { PRESET_MODELS } from '@/constants/presetModels';
import type { ProviderDisplayData, ModelDisplayData, OfficialModelDisplayData } from '@/components/common/ProviderCard/types';
import ProviderCard from '@/components/common/ProviderCard';
import OfficialProviderCard from '@/components/common/OfficialProviderCard';
import ProviderFormModal, { ProviderFormValues } from '@/components/common/ProviderFormModal';
import ModelFormModal, { ModelFormValues } from '@/components/common/ModelFormModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import ImportProviderModal from '@/components/common/ImportProviderModal';
import type { FetchedModel } from '@/components/common/FetchModelsModal/types';
import PluginSettings from '../components/PluginSettings';
import ConfigPathModal from '../components/ConfigPathModal';
import ConfigParseErrorAlert from '../components/ConfigParseErrorAlert';
import OhMyOpenCodeConfigSelector from '../components/OhMyOpenCodeConfigSelector';
import OhMyOpenCodeSlimConfigSelector from '../components/OhMyOpenCodeSlimConfigSelector';
import OhMyOpenCodeSettings from '../components/OhMyOpenCodeSettings';
import OhMyOpenCodeSlimSettings from '../components/OhMyOpenCodeSlimSettings';
import JsonEditor from '@/components/common/JsonEditor';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import ConnectivityTestModal from '../components/ConnectivityTestModal';
import { useRefreshStore } from '@/stores';

import styles from './OpenCodePage.module.less';

const { Title, Text, Link } = Typography;

// Helper function to convert OpenCodeProvider to ProviderDisplayData
const toProviderDisplayData = (id: string, provider: OpenCodeProvider): ProviderDisplayData => ({
  id,
  name: provider.name || id,
  sdkName: provider.npm || '@ai-sdk/openai-compatible',
  baseUrl: provider.options?.baseURL || '',
});

// Helper function to convert OpenCodeModel to ModelDisplayData
const toModelDisplayData = (id: string, model: OpenCodeModel): ModelDisplayData => ({
  id,
  name: model.name || id,
  contextLimit: model.limit?.context,
  outputLimit: model.limit?.output,
});

// Helper function to reorder object entries and return a new object
const reorderObject = <T,>(obj: Record<string, T>, newOrder: string[]): Record<string, T> => {
  const result: Record<string, T> = {};
  for (const key of newOrder) {
    if (obj[key]) {
      result[key] = obj[key];
    }
  }
  return result;
};

const SUPPORTED_PROVIDER_NPMS = new Set([
  '@ai-sdk/openai',
  '@ai-sdk/openai-compatible',
  '@ai-sdk/google',
  '@ai-sdk/anthropic',
]);

const OpenCodePage: React.FC = () => {
  const { t } = useTranslation();
  const { openCodeConfigRefreshKey, omosConfigRefreshKey, incrementOpenCodeConfigRefresh, incrementOmoConfigRefresh, incrementOmosConfigRefresh } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [config, setConfig] = React.useState<OpenCodeConfig | null>(null);
  const [configPathInfo, setConfigPathInfo] = React.useState<ConfigPathInfo | null>(null);
  const [parseError, setParseError] = React.useState<{
    path: string;
    error: string;
    contentPreview?: string;
  } | null>(null);

  // Preview modal state
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [previewData, setPreviewDataLocal] = React.useState<unknown>(null);

  // Provider modal state
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [currentProviderId, setCurrentProviderId] = React.useState<string>('');
  const [providerInitialValues, setProviderInitialValues] = React.useState<Partial<ProviderFormValues> | undefined>();

  // Model modal state
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [currentModelProviderId, setCurrentModelProviderId] = React.useState<string>('');
  const [currentModelId, setCurrentModelId] = React.useState<string>('');
  const [modelInitialValues, setModelInitialValues] = React.useState<Partial<ModelFormValues> | undefined>();

  // Fetch models modal state
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string>('');

  // Import provider modal state
  const [importModalOpen, setImportModalOpen] = React.useState(false);

  const [favoriteProviders, setFavoriteProviders] = React.useState<OpenCodeFavoriteProvider[]>([]);

  // Connectivity test modal state
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityProviderId, setConnectivityProviderId] = React.useState<string>('');


  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [officialProvidersCollapsed, setOfficialProvidersCollapsed] = React.useState(false);
  const [pathModalOpen, setPathModalOpen] = React.useState(false);
  const [otherConfigCollapsed, setOtherConfigCollapsed] = React.useState(true);
  const [unifiedModels, setUnifiedModels] = React.useState<UnifiedModelOption[]>([]);
  const [authProvidersData, setAuthProvidersData] = React.useState<GetAuthProvidersResponse | null>(null);
  const [authConfigPath, setAuthConfigPath] = React.useState<string>('');

  // Use ref for validation state to avoid re-renders during editing
  const otherConfigJsonValidRef = React.useRef(true);
  const [ohMyOpenCodeRefreshKey, setOhMyOpenCodeRefreshKey] = React.useState(0); // 用于触发 OhMyOpenCodeConfigSelector 刷新
  const [ohMyOpenCodeSettingsRefreshKey, setOhMyOpenCodeSettingsRefreshKey] = React.useState(0); // 用于触发 OhMyOpenCodeSettings 刷新
  const [omoConfigs, setOmoConfigs] = React.useState<Array<{ id: string; name: string; isApplied?: boolean }>>([]); // omo 配置列表
  const [omoSlimConfigs, setOmoSlimConfigs] = React.useState<Array<{ id: string; name: string; isApplied?: boolean }>>([]); // omo slim 配置列表

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const loadConfig = React.useCallback(async (showSuccessMessage = false) => {
    setLoading(true);
    setParseError(null); // Reset parse error state

    try {
      const pathInfo = await getOpenCodeConfigPathInfo();
      setConfigPathInfo(pathInfo);

      const result = await readOpenCodeConfigWithResult();

      switch (result.status) {
        case 'success':
          setConfig(result.config);
          if (showSuccessMessage) {
            message.success(t('opencode.refreshSuccess'));
          }
          break;

        case 'notFound':
          // Config file doesn't exist, initialize empty config
          setConfig({
            $schema: 'https://opencode.ai/config.json',
            provider: {},
          });
          if (showSuccessMessage) {
            message.success(t('opencode.refreshSuccess'));
          }
          break;

        case 'parseError':
          // Parse failed, set error state but still initialize empty config
          setParseError({
            path: result.path,
            error: result.error,
            contentPreview: result.contentPreview,
          });
          setConfig({
            $schema: 'https://opencode.ai/config.json',
            provider: {},
          });
          break;

        case 'error':
          // Other errors (e.g., permission denied)
          message.error(result.error);
          setConfig({
            $schema: 'https://opencode.ai/config.json',
            provider: {},
          });
          break;
      }
    } catch (error: unknown) {
      console.error('Failed to load config:', error);
      const errorMessage = error instanceof Error ? error.message : t('common.error');
      message.error(errorMessage);
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    loadConfig();
  }, [loadConfig, openCodeConfigRefreshKey]);

  // Reload config when MCP changes (from tray menu or MCP page)
  React.useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      unlisten = await listen('mcp-changed', () => {
        loadConfig();
      });
    };
    setup();
    return () => { unlisten?.(); };
  }, [loadConfig]);

  // Check if oh-my-opencode plugin is enabled (use contains matching for fork versions)
  const omoPluginEnabled = config?.plugin?.some((p) => {
    const baseName = p.split('@')[0];
    return baseName.includes('oh-my-opencode') && !baseName.includes('oh-my-opencode-slim');
  }) ?? false;

  // Check if oh-my-opencode-slim plugin is enabled (use contains matching for fork versions)
  const omoSlimPluginEnabled = config?.plugin?.some((p) => {
    const baseName = p.split('@')[0];
    return baseName.includes('oh-my-opencode-slim');
  }) ?? false;

  // Load omo config list
  React.useEffect(() => {
    const loadOmoConfigs = async () => {
      try {
        const configs = await listOhMyOpenCodeConfigs();
        setOmoConfigs(configs.map(c => ({ id: c.id, name: c.name, isApplied: c.isApplied })));
      } catch (error) {
        console.error('Failed to load omo configs:', error);
        setOmoConfigs([]);
      }
    };
    loadOmoConfigs();
  }, [openCodeConfigRefreshKey, ohMyOpenCodeSettingsRefreshKey]);

  // Load omo slim config list
  React.useEffect(() => {
    const loadOmoSlimConfigs = async () => {
      try {
        const configs = await listOhMyOpenCodeSlimConfigs();
        setOmoSlimConfigs(configs.map(c => ({ id: c.id, name: c.name, isApplied: c.isApplied })));
      } catch (error) {
        console.error('Failed to load omo slim configs:', error);
        setOmoSlimConfigs([]);
      }
    };
    loadOmoSlimConfigs();
  }, [openCodeConfigRefreshKey]);

  // Auto-apply the applied config when plugin is enabled
  const prevOmoPluginEnabledRef = React.useRef(omoPluginEnabled);
  React.useEffect(() => {
    const autoApplyConfig = async () => {
      // Only auto-apply when plugin changes from disabled to enabled
      if (!prevOmoPluginEnabledRef.current && omoPluginEnabled && omoConfigs.length > 0) {
        // Find the applied config
        const appliedConfig = omoConfigs.find((c) => c.isApplied);
        if (appliedConfig) {
          try {
            await applyOhMyOpenCodeConfig(appliedConfig.id);
            console.log('Auto-applied omo config:', appliedConfig.name);
          } catch (error) {
            console.error('Failed to auto-apply omo config:', error);
          }
        }
      }
      prevOmoPluginEnabledRef.current = omoPluginEnabled;
    };
    autoApplyConfig();
  }, [omoPluginEnabled, omoConfigs]);

  // Load unified models (combining custom providers and official auth providers)
  React.useEffect(() => {
    const loadUnifiedModels = async () => {
      try {
        const models = await getOpenCodeUnifiedModels();
        setUnifiedModels(models);
        // Refresh tray menu to update model list
        await refreshTrayMenu();
      } catch (error) {
        console.error('Failed to load unified models:', error);
      }
    };

    loadUnifiedModels();
  }, [openCodeConfigRefreshKey]);

  const providerFilterOptions = React.useMemo(() => {
    if (!config || !currentProviderId) return [];
    const provider = config.provider[currentProviderId];
    if (!provider) return [];

    const modelMap = new Map<string, string>();

    if (provider.models) {
      Object.entries(provider.models).forEach(([id, model]) => {
        modelMap.set(id, model.name || id);
      });
    }

    const officialModels = authProvidersData?.mergedModels?.[currentProviderId] || [];
    officialModels.forEach((model) => {
      if (!modelMap.has(model.id)) {
        modelMap.set(model.id, model.name || model.id);
      }
    });

    return Array.from(modelMap.entries()).map(([id, name]) => ({
      label: `${name} (${id})`,
      value: id,
    }));
  }, [config, currentProviderId, authProvidersData]);

  // Load official auth providers data
  React.useEffect(() => {
    const loadAuthProviders = async () => {
      try {
        const data = await getOpenCodeAuthProviders();
        setAuthProvidersData(data);
      } catch (error) {
        console.error('Failed to load auth providers:', error);
      }
    };

    const loadAuthConfigPath = async () => {
      try {
        const path = await getOpenCodeAuthConfigPath();
        setAuthConfigPath(path);
      } catch (error) {
        console.error('Failed to load auth config path:', error);
      }
    };

    loadAuthProviders();
    loadAuthConfigPath();
  }, [openCodeConfigRefreshKey]);

  React.useEffect(() => {
    const loadFavProviders = async () => {
      try {
        const providers = await listFavoriteProviders();
        setFavoriteProviders(providers);
      } catch (error) {
        console.error('Failed to load favorite providers:', error);
      }
    };
    loadFavProviders();
  }, [openCodeConfigRefreshKey, omosConfigRefreshKey]);

  // Open auth.json config file
  const handleOpenAuthConfig = async () => {
    if (!authConfigPath) {
      message.warning(t('opencode.official.configNotFound'));
      return;
    }
    try {
      await revealItemInDir(authConfigPath);
    } catch (error) {
      console.error('Failed to open auth config:', error);
      message.error(t('common.error'));
    }
  };

  const doSaveConfig = async (newConfig: OpenCodeConfig) => {
    try {
      await saveOpenCodeConfig(newConfig);
      setConfig(newConfig);
      message.success(t('common.success'));
    } catch {
      message.error(t('common.error'));
      throw new Error('Save failed');
    }
  };

  const handleOpenConfigFolder = async () => {
    if (!configPathInfo?.path) return;

    try {
      // Try to reveal the file in explorer
      await revealItemInDir(configPathInfo.path);
    } catch {
      // If file doesn't exist, fallback to opening parent directory
      try {
        const parentDir = configPathInfo.path.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (error) {
        console.error('Failed to open folder:', error);
        message.error(t('common.error'));
      }
    }
  };

  const handlePathModalSuccess = () => {
    setPathModalOpen(false);
    loadConfig();
  };

  const handleParseErrorBackedUp = () => {
    setParseError(null);
    loadConfig();
  };

  // Provider handlers
  const handleAddProvider = () => {
    setCurrentProviderId('');
    setProviderInitialValues(undefined);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (providerId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    // 提取已知字段之外的额外参数
    const knownOptionKeys = ['baseURL', 'apiKey', 'headers', 'timeout', 'setCacheKey'];
    const extraOptions: Record<string, unknown> = {};
    if (provider.options) {
      Object.keys(provider.options).forEach((key) => {
        if (!knownOptionKeys.includes(key)) {
          extraOptions[key] = provider.options![key];
        }
      });
    }

    setCurrentProviderId(providerId);
    const hasBlacklist = provider.blacklist !== undefined;
    setProviderInitialValues({
      id: providerId,
      name: provider.name,
      sdkType: provider.npm || '@ai-sdk/openai-compatible',
      baseUrl: provider.options?.baseURL || '',
      apiKey: provider.options?.apiKey || '',
      headers: provider.options?.headers,
      timeout: provider.options?.timeout === false ? undefined : (provider.options?.timeout as number | undefined),
      disableTimeout: provider.options?.timeout === false,
      setCacheKey: provider.options?.setCacheKey,
      extraOptions: Object.keys(extraOptions).length > 0 ? extraOptions : undefined,
      filterMode: hasBlacklist ? 'blacklist' : 'whitelist',
      filterModels: hasBlacklist ? (provider.blacklist || []) : (provider.whitelist || []),
    });
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (providerId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    // 提取已知字段之外的额外参数
    const knownOptionKeys = ['baseURL', 'apiKey', 'headers', 'timeout', 'setCacheKey'];
    const extraOptions: Record<string, unknown> = {};
    if (provider.options) {
      Object.keys(provider.options).forEach((key) => {
        if (!knownOptionKeys.includes(key)) {
          extraOptions[key] = provider.options![key];
        }
      });
    }

    setCurrentProviderId('');
    const hasBlacklist = provider.blacklist !== undefined;
    setProviderInitialValues({
      id: `${providerId}_copy`,
      name: provider.name,
      sdkType: provider.npm,
      baseUrl: provider.options?.baseURL || '',
      apiKey: provider.options?.apiKey || '',
      headers: provider.options?.headers,
      timeout: provider.options?.timeout === false ? undefined : (provider.options?.timeout as number | undefined),
      disableTimeout: provider.options?.timeout === false,
      setCacheKey: provider.options?.setCacheKey,
      extraOptions: Object.keys(extraOptions).length > 0 ? extraOptions : undefined,
      filterMode: hasBlacklist ? 'blacklist' : 'whitelist',
      filterModels: hasBlacklist ? (provider.blacklist || []) : (provider.whitelist || []),
    });
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = async (providerId: string) => {
    if (!config) return;

    const newProviders = { ...config.provider };
    delete newProviders[providerId];

    await doSaveConfig({
      ...config,
      provider: newProviders,
    });
  };

  const handleProviderSuccess = async (values: ProviderFormValues) => {
    if (!config) return;

    const filterMode = values.filterMode || 'whitelist';
    const filterModels = (values.filterModels || []).filter((modelId) => modelId);
    const shouldPersistFilter = filterModels.length > 0;

    const newProvider: OpenCodeProvider = {
      npm: values.sdkType || '@ai-sdk/openai-compatible',
      name: values.name,
      options: {
        ...(values.baseUrl && { baseURL: values.baseUrl }),
        ...(values.apiKey && { apiKey: values.apiKey }),
        ...(values.headers && { headers: values.headers as Record<string, string> }),
        ...(values.disableTimeout
          ? { timeout: false as const }
          : values.timeout !== undefined && { timeout: values.timeout }),
        ...(values.setCacheKey !== undefined && { setCacheKey: values.setCacheKey }),
        // 合并额外参数
        ...(values.extraOptions && { ...values.extraOptions }),
      },
      models: currentProviderId ? config.provider[currentProviderId]?.models || {} : {},
      whitelist: shouldPersistFilter && filterMode === 'whitelist' ? filterModels : undefined,
      blacklist: shouldPersistFilter && filterMode === 'blacklist' ? filterModels : undefined,
    };

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [values.id]: newProvider,
      },
    });

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(values.id, newProvider);
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    setProviderModalOpen(false);
    setProviderInitialValues(undefined);
  };

  const handleProviderDuplicateId = () => {
    message.error(t('opencode.provider.idExists'));
  };

  // Model handlers
  const handleAddModel = (providerId: string) => {
    setCurrentModelProviderId(providerId);
    setCurrentModelId('');
    setModelInitialValues(undefined);
    setModelModalOpen(true);
  };

  const handleEditModel = (providerId: string, modelId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;
    const model = provider.models[modelId];
    if (!model) return;

    setCurrentModelProviderId(providerId);
    setCurrentModelId(modelId);
    setModelInitialValues({
      id: modelId,
      name: model.name,
      contextLimit: model.limit?.context,
      outputLimit: model.limit?.output,
      options: model.options ? JSON.stringify(model.options) : undefined,
      variants: model.variants ? JSON.stringify(model.variants) : undefined,
      modalities: model.modalities ? JSON.stringify(model.modalities) : undefined,
    });
    setModelModalOpen(true);
  };

  const handleCopyModel = (providerId: string, modelId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;
    const model = provider.models[modelId];
    if (!model) return;

    setCurrentModelProviderId(providerId);
    setCurrentModelId('');
    setModelInitialValues({
      id: `${modelId}_copy`,
      name: model.name,
      contextLimit: model.limit?.context,
      outputLimit: model.limit?.output,
      options: model.options ? JSON.stringify(model.options) : undefined,
      variants: model.variants ? JSON.stringify(model.variants) : undefined,
      modalities: model.modalities ? JSON.stringify(model.modalities) : undefined,
    });
    setModelModalOpen(true);
  };

  const handleDeleteModel = async (providerId: string, modelId: string) => {
    if (!config) return;

    const provider = config.provider[providerId];
    if (!provider) return;

    const newModels = { ...provider.models };
    delete newModels[modelId];

    const updatedProvider: OpenCodeProvider = {
      ...provider,
      models: newModels,
    };

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [providerId]: updatedProvider,
      },
    });

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(providerId, updatedProvider);
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    // Refresh tray menu and model list after deleting model
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  const handleModelSuccess = async (values: ModelFormValues) => {
    if (!config) return;

    const provider = config.provider[currentModelProviderId];
    if (!provider) return;

    const newModel: OpenCodeModel = {
      ...(values.name && { name: values.name }),
      ...(values.contextLimit || values.outputLimit
        ? {
            limit: {
              ...(values.contextLimit && { context: values.contextLimit }),
              ...(values.outputLimit && { output: values.outputLimit }),
            },
          }
        : {}),
      ...(values.modalities && { modalities: JSON.parse(values.modalities) }),
      ...(values.options && { options: JSON.parse(values.options) }),
      ...(values.variants && { variants: JSON.parse(values.variants) }),
    };

    const updatedProvider: OpenCodeProvider = {
      ...provider,
      models: {
        ...provider.models,
        [values.id]: newModel,
      },
    };

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [currentModelProviderId]: updatedProvider,
      },
    });

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(currentModelProviderId, updatedProvider);
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    setModelModalOpen(false);
    setModelInitialValues(undefined);
    // Refresh tray menu and model list after adding/editing model
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  const handleModelDuplicateId = () => {
    message.error(t('opencode.model.idExists'));
  };

  // Fetch models handlers
  const handleOpenFetchModels = (providerId: string) => {
    setFetchModelsProviderId(providerId);
    setFetchModelsModalOpen(true);
  };

  const handleFetchModelsSuccess = async (selectedModels: FetchedModel[]) => {
    if (!config || !fetchModelsProviderId) return;

    const provider = config.provider[fetchModelsProviderId];
    if (!provider) return;

    // Add selected models to provider
    const newModels = { ...provider.models };
    selectedModels.forEach((model) => {
      newModels[model.id] = {
        name: model.name || model.id,
      };
    });

    const updatedProvider: OpenCodeProvider = {
      ...provider,
      models: newModels,
    };

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [fetchModelsProviderId]: updatedProvider,
      },
    });

    // Auto-save to favorite providers (silently)
    try {
      await upsertFavoriteProvider(fetchModelsProviderId, updatedProvider);
    } catch (error) {
      console.error('Failed to save favorite provider:', error);
    }

    setFetchModelsModalOpen(false);
    message.success(t('opencode.fetchModels.addSuccess', { count: selectedModels.length }));
    // Refresh tray menu and model list after fetching models
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  // Get current provider info for FetchModelsModal
  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!config || !fetchModelsProviderId) return null;
    const provider = config.provider[fetchModelsProviderId];
    if (!provider) return null;
    return {
      name: provider.name || fetchModelsProviderId,
      baseUrl: provider.options?.baseURL || '',
      apiKey: provider.options?.apiKey,
      headers: provider.options?.headers as Record<string, string> | undefined,
      sdkName: provider.npm,
      existingModelIds: Object.keys(provider.models || {}),
    };
  }, [config, fetchModelsProviderId]);

  // Import provider handlers
  const handleImportProviders = async (providers: OpenCodeFavoriteProvider[]) => {
    if (!config) return;

    // Build new providers object
    const newProviders = { ...config.provider };
    providers.forEach((p) => {
      // Only add if not already exists
      if (!newProviders[p.providerId]) {
        newProviders[p.providerId] = p.providerConfig;
      }
    });

    await doSaveConfig({
      ...config,
      provider: newProviders,
    });

    setImportModalOpen(false);
    message.success(t('opencode.provider.importSuccess', { count: providers.length }));
    // Refresh tray menu and model list after importing
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  const favoriteProvidersMap = React.useMemo(() => {
    return new Map(favoriteProviders.map((item) => [item.providerId, item]));
  }, [favoriteProviders]);

  // Get current provider info for ConnectivityTestModal
  const connectivityProviderInfo = React.useMemo(() => {
    if (!config || !connectivityProviderId) return null;
    const provider = config.provider[connectivityProviderId];
    if (!provider) return null;
    return {
      name: provider.name || connectivityProviderId,
      config: provider,
      modelIds: provider.models ? Object.keys(provider.models) : [],
      diagnostics: favoriteProvidersMap.get(connectivityProviderId)?.diagnostics,
    };
  }, [config, connectivityProviderId, favoriteProvidersMap]);

  const handleOpenConnectivityTest = (providerId: string) => {
    setConnectivityProviderId(providerId);
    setConnectivityModalOpen(true);
  };

  const handleSaveDiagnostics = async (diagnostics: OpenCodeDiagnosticsConfig) => {
    if (!config || !connectivityProviderId) return;

    const provider = config.provider[connectivityProviderId];
    if (!provider) return;

    // Save diagnostics to favorite provider ONLY
    try {
      const updatedFav = await upsertFavoriteProvider(connectivityProviderId, provider, diagnostics);
      
      // Update local state
      setFavoriteProviders((prev) => {
        const index = prev.findIndex((p) => p.providerId === connectivityProviderId);
        if (index >= 0) {
          const newFavs = [...prev];
          newFavs[index] = updatedFav;
          return newFavs;
        } else {
          return [...prev, updatedFav];
        }
      });
    } catch (error) {
      console.error('Failed to save diagnostics:', error);
      message.error(t('common.error'));
    }
  };

  const handleRemoveModels = async (modelIdsToRemove: string[]) => {
    if (!config || !connectivityProviderId) return;

    const provider = config.provider[connectivityProviderId];
    if (!provider || !provider.models) return;

    // Create new models object without the removed models
    const newModels = { ...provider.models };
    for (const modelId of modelIdsToRemove) {
      delete newModels[modelId];
    }

    const newConfig = {
      ...config,
      provider: {
        ...config.provider,
        [connectivityProviderId]: {
          ...provider,
          models: newModels,
        },
      },
    };

    await doSaveConfig(newConfig);
    await refreshTrayMenu();
    incrementOpenCodeConfigRefresh();
  };

  // Drag handlers

  const handleProviderDragEnd = async (event: DragEndEvent) => {
    if (!config) return;
    const { active, over } = event;

    if (over && active.id !== over.id) {
      const providerIds = Object.keys(config.provider);
      const oldIndex = providerIds.indexOf(active.id as string);
      const newIndex = providerIds.indexOf(over.id as string);

      const newOrder = arrayMove(providerIds, oldIndex, newIndex);
      const newProviders = reorderObject(config.provider, newOrder);

      await doSaveConfig({
        ...config,
        provider: newProviders,
      });
    }
  };

  const handleReorderModels = async (providerId: string, modelIds: string[]) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    const newModels = reorderObject(provider.models, modelIds);

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [providerId]: {
          ...provider,
          models: newModels,
        },
      },
    });
  };

  const handlePreviewConfig = async () => {
    if (!config) return;
    setPreviewDataLocal(config);
    setPreviewModalOpen(true);
  };

  const providerEntries = config && config.provider ? Object.entries(config.provider) : [];
  const existingProviderIds = providerEntries.map(([id]) => id);
  const existingModelIds = React.useMemo(() => {
    if (!config || !config.provider || !currentModelProviderId) return [];
    const provider = config.provider[currentModelProviderId];
    return provider && provider.models ? Object.keys(provider.models) : [];
  }, [config, currentModelProviderId]);

  // Collect all available models for model selectors using unified models
  const modelOptions = React.useMemo(() => {
    return unifiedModels.map((m) => ({
      label: m.displayName,
      value: m.id,
    }));
  }, [unifiedModels]);

  // Build model variants map from config and preset models
  const modelVariantsMap = React.useMemo(
    () => buildModelVariantsMap(config, unifiedModels, PRESET_MODELS),
    [config, unifiedModels]
  );

  // 主模型选项 - 基于 modelOptions 添加选中标记
  const mainModelOptions = React.useMemo(() => {
    return modelOptions.map((opt) => ({
      ...opt,
      label: config?.model === opt.value ? `${opt.label} ✓` : opt.label,
    }));
  }, [modelOptions, config?.model]);

  // 小模型选项 - 基于 modelOptions 添加选中标记
  const smallModelOptions = React.useMemo(() => {
    return modelOptions.map((opt) => ({
      ...opt,
      label: config?.small_model === opt.value ? `${opt.label} ✓` : opt.label,
    }));
  }, [modelOptions, config?.small_model]);

  const handleModelChange = async (field: 'model' | 'small_model', value: string | undefined) => {
    if (!config) return;
    
    await doSaveConfig({
      ...config,
      [field]: value || undefined,
    });
  };

  const handlePluginChange = async (plugins: string[]) => {
    if (!config) return;
    
    await doSaveConfig({
      ...config,
      plugin: plugins.length > 0 ? plugins : undefined,
    });
  };

  // Extract other config fields (unknown fields)
  const otherConfigFields = React.useMemo(() => {
    if (!config) return undefined;
    const knownFields = ['$schema', 'provider', 'model', 'small_model', 'plugin', 'mcp'];
    const other: Record<string, unknown> = {};
    Object.keys(config).forEach((key) => {
      if (!knownFields.includes(key)) {
        other[key] = config[key];
      }
    });
    // 如果没有其他字段，返回 undefined 而不是空对象，这样 JsonEditor 会显示 placeholder
    return Object.keys(other).length > 0 ? other : undefined;
  }, [config]);

  const handleOtherConfigChange = (_value: unknown, isValid: boolean) => {
    otherConfigJsonValidRef.current = isValid;
    // 只验证，不保存
  };

  // 保存其他配置（用于 onBlur 回调）
  const handleOtherConfigBlur = async (value: unknown) => {
    if (!config || !otherConfigJsonValidRef.current) {
      return;
    }

    // Remove old unknown fields
    const newConfig: OpenCodeConfig = {
      $schema: config.$schema,
      provider: config.provider,
      model: config.model,
      small_model: config.small_model,
      plugin: config.plugin,
      mcp: config.mcp,
    };

    // Add new other fields
    if (typeof value === 'object' && value !== null) {
      Object.assign(newConfig, value);
    }

    await doSaveConfig(newConfig);
  };

  return (
    <div>
      {/* If parse error exists, only show the error alert */}
      {parseError ? (
        <div style={{
          display: 'flex',
          justifyContent: 'center',
          alignItems: 'center',
          minHeight: '60vh',
          padding: '24px'
        }}>
          <div style={{ width: '100%', maxWidth: '800px' }}>
            <ConfigParseErrorAlert
              path={parseError.path}
              error={parseError.error}
              contentPreview={parseError.contentPreview}
              onBackedUp={handleParseErrorBackedUp}
            />
          </div>
        </div>
      ) : (
        <>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 16 }}>
            <div>
              <div style={{ marginBottom: 8 }}>
                <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                  {t('opencode.title')}
                </Title>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openUrl('https://opencode.ai/docs/config/#format');
                  }}
                >
                  <LinkOutlined /> {t('opencode.viewDocs')}
                </Link>
                <Link
                  type="secondary"
                  style={{ fontSize: 12, marginLeft: 16 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    handlePreviewConfig();
                  }}
                >
                  <EyeOutlined /> {t('common.previewConfig')}
                </Link>
              </div>
              <Space>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('opencode.configPath')}:
                </Text>
                {configPathInfo?.source === 'env' && (
                  <Tag color="blue" icon={<EnvironmentOutlined />} style={{ fontSize: 12 }}>
                    {t('opencode.configPathSource.fromEnv')}
                  </Tag>
                )}
                {configPathInfo?.source === 'custom' && (
                  <Tag color="green" style={{ fontSize: 12 }}>
                    {t('opencode.configPathSource.custom')}
                  </Tag>
                )}
                {configPathInfo?.source === 'shell' && (
                  <Tag color="cyan" style={{ fontSize: 12 }}>
                    {t('opencode.configPathSource.fromShell')}
                  </Tag>
                )}
                {configPathInfo?.source === 'default' && (
                  <Tag style={{ fontSize: 12 }}>
                    {t('opencode.configPathSource.default')}
                  </Tag>
                )}
                <Text code style={{ fontSize: 12 }}>
                  {configPathInfo?.path}
                </Text>
                <Button
                  type="text"
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => setPathModalOpen(true)}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('opencode.configPathSource.customize')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenConfigFolder}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('opencode.openFolder')}
                </Button>
                <Button
                  type="text"
                  size="small"
                  icon={<ReloadOutlined />}
                  onClick={() => {
                    loadConfig(true);
                    incrementOpenCodeConfigRefresh();
                    incrementOmoConfigRefresh();
                    incrementOmosConfigRefresh();
                  }}
                  style={{ padding: 0, fontSize: 12 }}
                >
                  {t('common.refresh')}
                </Button>
              </Space>
            </div>
          </div>

          <div className={styles.modelCard}>
        <Title level={5} className={styles.modelCardTitle}>
          <RobotOutlined style={{ marginRight: 8 }} />
          {t('opencode.modelSettings.title')}
        </Title>
        <div className={styles.modelCardContent}>
          <Space orientation="vertical" style={{ width: '100%' }} size={12}>
            <div>
              <div style={{ marginBottom: 4 }}>
                <Text strong>{t('opencode.modelSettings.modelLabel')}</Text>
              </div>
              <Select
                value={config?.model}
                onChange={(value) => handleModelChange('model', value)}
                placeholder={t('opencode.modelSettings.modelPlaceholder')}
                allowClear
                options={mainModelOptions}
                optionLabelProp="label"
                style={{ width: '100%' }}
                notFoundContent={t('opencode.modelSettings.noModels')}
              />
            </div>

            <div>
              <div style={{ marginBottom: 4 }}>
                <Text strong>{t('opencode.modelSettings.smallModelLabel')}</Text>
                <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                  {t('opencode.modelSettings.smallModelHint')}
                </Text>
              </div>
              <Select
                value={config?.small_model}
                onChange={(value) => handleModelChange('small_model', value)}
                placeholder={t('opencode.modelSettings.smallModelPlaceholder')}
                allowClear
                options={smallModelOptions}
                optionLabelProp="label"
                style={{ width: '100%' }}
                notFoundContent={t('opencode.modelSettings.noModels')}
              />
            </div>

{/* Oh My OpenCode Config Selector - show only if plugin is enabled */}
            {omoPluginEnabled && (
              <div>
                <div style={{ marginBottom: 4 }}>
                  <Text strong>{t('opencode.ohMyOpenCode.configLabel')}</Text>
                  <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                    {t('opencode.ohMyOpenCode.configHint')}
                  </Text>
                </div>
                <OhMyOpenCodeConfigSelector
                  key={ohMyOpenCodeRefreshKey} // 当 key 改变时，组件会重新挂载并刷新
                  disabled={false}
                  onConfigSelected={() => {
                    message.success(t('opencode.ohMyOpenCode.configSelected'));
                    // 当在快速切换框中选择配置时，触发设置列表刷新
                    setOhMyOpenCodeSettingsRefreshKey((prev) => prev + 1);
                  }}
                />
              </div>
            )}

            {/* Oh My OpenCode Slim Config Selector - show only if plugin is enabled */}
            {omoSlimPluginEnabled && (
              <div>
                <div style={{ marginBottom: 4 }}>
                  <Text strong>{t('opencode.ohMyOpenCode.slimConfigLabel')}</Text>
                  <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                    {t('opencode.ohMyOpenCode.slimConfigHint')}
                  </Text>
                </div>
                <OhMyOpenCodeSlimConfigSelector
                  disabled={false}
                  onConfigSelected={() => {
                    message.success(t('opencode.ohMyOpenCode.configSelected'));
                  }}
                />
              </div>
            )}
          </Space>
        </div>
      </div>

      <PluginSettings
        plugins={config?.plugin || []}
        onChange={handlePluginChange}
      />

{/* Oh My OpenCode Settings - show if plugin is enabled or has configs */}
      {(omoPluginEnabled || omoConfigs.length > 0) && (
        <OhMyOpenCodeSettings
          key={ohMyOpenCodeSettingsRefreshKey} // 当 key 改变时，组件会重新挂载并刷新
          modelOptions={modelOptions}
          modelVariantsMap={modelVariantsMap}
          disabled={!omoPluginEnabled}
          onConfigApplied={() => {
            // 当配置被应用时，触发 Selector 刷新以更新选中状态
            setOhMyOpenCodeRefreshKey((prev) => prev + 1);
          }}
          onConfigUpdated={() => {
            // 当配置被创建/更新/删除时，触发 Selector 刷新
            setOhMyOpenCodeRefreshKey((prev) => prev + 1);
          }}
        />
      )}

{/* Oh My OpenCode Slim Settings - show if plugin is enabled or has configs */}
      {(omoSlimPluginEnabled || omoSlimConfigs.length > 0) && (
        <OhMyOpenCodeSlimSettings
          modelOptions={modelOptions}
          modelVariantsMap={modelVariantsMap}
          disabled={!omoSlimPluginEnabled}
          onConfigApplied={() => {
            message.success('配置已应用');
          }}
          onConfigUpdated={() => {
            // 配置更新后刷新
            loadConfig();
          }}
        />
      )}


      <Collapse
        className={styles.collapseCard}
        activeKey={providerListCollapsed ? [] : ['providers']}
        onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
        items={[
          {
            key: 'providers',
            label: (
              <Text strong><DatabaseOutlined style={{ marginRight: 8 }} />{t('opencode.provider.title')}</Text>
            ),
            extra: (
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
                {t('opencode.addProvider')}
              </Button>
            ),
            children: (
              <Spin spinning={loading}>
                {providerEntries.length === 0 ? (
                  <Empty description={t('opencode.emptyText')} style={{ marginTop: 40 }} />
                ) : (
                  <DndContext
                    sensors={sensors}
                    collisionDetection={closestCenter}
                    modifiers={[restrictToVerticalAxis]}
                    onDragEnd={handleProviderDragEnd}
                  >
                    <SortableContext
                      items={providerEntries.map(([id]) => id)}
                      strategy={verticalListSortingStrategy}
                    >
                      {providerEntries.map(([providerId, provider]) => {
                        const providerNpm = provider.npm || '@ai-sdk/openai-compatible';
                        const isConnectivitySupported = SUPPORTED_PROVIDER_NPMS.has(providerNpm);
                        const providerBaseUrl = provider.options?.baseURL?.trim() || '';
                        const providerApiKey = provider.options?.apiKey?.trim() || '';
                        const isProviderAuthReady = Boolean(providerBaseUrl && providerApiKey);
                        const connectivityTooltip = !isConnectivitySupported
                          ? t('opencode.connectivity.unsupportedNpm', { npm: providerNpm })
                          : !isProviderAuthReady
                            ? t('opencode.provider.completeUrlAndKey')
                            : '';
                        const fetchModelsTooltip = !isProviderAuthReady
                          ? t('opencode.provider.completeUrlAndKey')
                          : '';
                        return (
                        <ProviderCard
                          key={providerId}
                          provider={toProviderDisplayData(providerId, provider)}
                          models={provider.models ? Object.entries(provider.models).map(([modelId, model]) =>
                            toModelDisplayData(modelId, model)
                          ) : []}
                          officialModels={authProvidersData?.mergedModels?.[providerId]?.map((m): OfficialModelDisplayData => ({
                            id: m.id,
                            name: m.name,
                            isFree: m.isFree,
                            context: m.context,
                            output: m.output,
                            status: m.status,
                          }))}
                          draggable
                          sortableId={providerId}
                          onEdit={() => handleEditProvider(providerId)}
                          onCopy={() => handleCopyProvider(providerId)}
                          onDelete={() => handleDeleteProvider(providerId)}
                          extraActions={
                            <Space size={0}>
                              <Tooltip title={connectivityTooltip}>
                                <span>
                                  <Button
                                    size="small"
                                    type="text"
                                    style={{ fontSize: 12 }}
                                    onClick={() => handleOpenConnectivityTest(providerId)}
                                    disabled={!isConnectivitySupported || !isProviderAuthReady}
                                  >
                                    <ApiOutlined style={{ marginRight: 4 }} />
                                    {t('opencode.connectivity.button')}
                                  </Button>
                                </span>
                              </Tooltip>
                              <Tooltip title={fetchModelsTooltip}>
                                <span>
                                  <Button
                                    size="small"
                                    type="text"
                                    style={{ fontSize: 12 }}
                                    onClick={() => handleOpenFetchModels(providerId)}
                                    disabled={!isProviderAuthReady}
                                  >
                                    <CloudDownloadOutlined style={{ marginRight: 4 }} />
                                    {t('opencode.fetchModels.button')}
                                  </Button>
                                </span>
                              </Tooltip>
                            </Space>
                          }

                          onAddModel={() => handleAddModel(providerId)}
                          onEditModel={(modelId) => handleEditModel(providerId, modelId)}
                          onCopyModel={(modelId) => handleCopyModel(providerId, modelId)}
                          onDeleteModel={(modelId) => handleDeleteModel(providerId, modelId)}
                          modelsDraggable
                          onReorderModels={(modelIds) => handleReorderModels(providerId, modelIds)}
                          i18nPrefix="opencode"
                        />
                        );
                      })}
                    </SortableContext>
                  </DndContext>
                )}
                <div style={{ marginTop: 12 }}>
                  <Button
                    type="dashed"
                    icon={<ImportOutlined />}
                    onClick={() => setImportModalOpen(true)}
                  >
                    {t('opencode.provider.importFavorite')}
                  </Button>
                </div>
              </Spin>
            ),
          },
        ]}
      />

      {/* Official Auth Providers Section - only show if there are standalone providers */}
      {authProvidersData && authProvidersData.standaloneProviders.length > 0 && (
        <Collapse
          className={styles.collapseCard}
          activeKey={officialProvidersCollapsed ? [] : ['official-providers']}
          onChange={(keys) => setOfficialProvidersCollapsed(!keys.includes('official-providers'))}
          items={[
            {
              key: 'official-providers',
              label: (
                <Space size={8}>
                  <Text strong><SafetyCertificateOutlined style={{ marginRight: 8 }} />{t('opencode.official.title')}</Text>
                  <Tooltip title={t('opencode.official.openConfigHint')}>
                    <Button
                      type="link"
                      size="small"
                      icon={<FileOutlined />}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleOpenAuthConfig();
                      }}
                      style={{ padding: 0, height: 'auto' }}
                    >
                      auth.json
                    </Button>
                  </Tooltip>
                </Space>
              ),
              children: (
                <div>
                  <div style={{ marginBottom: 12 }}>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('opencode.official.description')}
                    </Text>
                  </div>
                  {authProvidersData.standaloneProviders.map((provider) => (
                    <OfficialProviderCard
                      key={provider.id}
                      id={provider.id}
                      name={provider.name}
                      models={provider.models}
                      i18nPrefix="opencode"
                    />
                  ))}
                </div>
              ),
            },
          ]}
        />
      )}

      <Collapse
        className={styles.collapseCard}
        activeKey={otherConfigCollapsed ? [] : ['other']}
        onChange={(keys) => setOtherConfigCollapsed(!keys.includes('other'))}
        items={[
          {
            key: 'other',
            label: <Text strong><ToolOutlined style={{ marginRight: 8 }} />{t('opencode.otherConfig.title')}</Text>,
            children: (
              <div>
                <Form.Item
                  help={
                    <span>
                      <Text type="secondary">{t('opencode.otherConfig.hint')}，</Text>
                      <span style={{ color: '#1677ff' }}>
                        {t('opencode.otherConfig.autoSaveHint')}
                      </span>
                    </span>
                  }
                  style={{ marginBottom: 0 }}
                >
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
    "permission": "allow",
    "autoupdate": true
}`}
                  />
                </Form.Item>
              </div>
            ),
          },
        ]}
      />

      <ProviderFormModal
        open={providerModalOpen}
        isEdit={!!currentProviderId}
        initialValues={providerInitialValues}
        existingIds={currentProviderId ? [] : existingProviderIds}
        apiKeyRequired={false}
        onCancel={() => {
          setProviderModalOpen(false);
          setProviderInitialValues(undefined);
        }}
        onSuccess={handleProviderSuccess}
        onDuplicateId={handleProviderDuplicateId}
        i18nPrefix="opencode"
        headersOutputFormat="object"
        showOpenCodeAdvanced={true}
        modelOptions={providerFilterOptions}
      />

      <ModelFormModal
        open={modelModalOpen}
        isEdit={!!currentModelId}
        initialValues={modelInitialValues}
        existingIds={currentModelId ? [] : existingModelIds}
        showOptions
        showVariants={true}
        showModalities={true}
        limitRequired={false}
        nameRequired={false}
        npmType={currentModelProviderId && config?.provider[currentModelProviderId]?.npm || '@ai-sdk/openai-compatible'}
        onCancel={() => {
          setModelModalOpen(false);
          setModelInitialValues(undefined);
        }}
        onSuccess={handleModelSuccess}
        onDuplicateId={handleModelDuplicateId}
        i18nPrefix="opencode"
      />

      <ConfigPathModal
        open={pathModalOpen}
        currentPathInfo={configPathInfo}
        onCancel={() => setPathModalOpen(false)}
        onSuccess={handlePathModalSuccess}
      />

      {fetchModelsProviderInfo && (
        <FetchModelsModal
          open={fetchModelsModalOpen}
          providerName={fetchModelsProviderInfo.name}
          baseUrl={fetchModelsProviderInfo.baseUrl}
          apiKey={fetchModelsProviderInfo.apiKey}
          headers={fetchModelsProviderInfo.headers}
          sdkType={fetchModelsProviderInfo.sdkName}
          existingModelIds={fetchModelsProviderInfo.existingModelIds}
          onCancel={() => setFetchModelsModalOpen(false)}
          onSuccess={handleFetchModelsSuccess}
        />
      )}

      <ImportProviderModal
        open={importModalOpen}
        onClose={() => setImportModalOpen(false)}
        onImport={handleImportProviders}
        existingProviderIds={existingProviderIds}
      />

      {connectivityProviderInfo && (
        <ConnectivityTestModal
          open={connectivityModalOpen}
          onCancel={() => setConnectivityModalOpen(false)}
          providerId={connectivityProviderId}
          providerName={connectivityProviderInfo.name}
          providerConfig={connectivityProviderInfo.config}
          modelIds={connectivityProviderInfo.modelIds}
          diagnostics={connectivityProviderInfo.diagnostics}
          onSaveDiagnostics={handleSaveDiagnostics}
          onRemoveModels={handleRemoveModels}
        />
      )}

      {/* Preview Modal */}
      <JsonPreviewModal
        open={previewModalOpen}
        onClose={() => setPreviewModalOpen(false)}
        title={t('opencode.preview.title')}
        data={previewData}
      />

        </>
      )}
    </div>
  );
};

export default OpenCodePage;
