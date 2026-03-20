import React from 'react';
import {
  Button,
  Empty,
  Space,
  Typography,
  message,
  Spin,
  Collapse,
  Tag,
  Alert,
} from 'antd';
import {
  PlusOutlined,
  FolderOpenOutlined,
  EyeOutlined,
  EditOutlined,
  ReloadOutlined,
  LinkOutlined,
  DatabaseOutlined,
  RobotOutlined,
  SettingOutlined,
  MoreOutlined,
  ImportOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
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
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';

import {
  readOpenClawConfigWithResult,
  saveOpenClawConfig,
  getOpenClawConfigPathInfo,
  backupOpenClawConfig,
  getOpenClawAgentsDefaults,
} from '@/services/openclawApi';
import {
  PRESET_MODELS,
  type PresetModel,
} from '@/constants/presetModels';
import type { OpenCodeDiagnosticsConfig } from '@/services/opencodeApi';
import { refreshTrayMenu, hasAllApiHubExtension } from '@/services/appApi';
import type {
  OpenClawConfig,
  OpenClawConfigPathInfo,
  OpenClawProviderConfig,
  OpenClawModel,
  OpenClawAgentsDefaults,
} from '@/types/openclaw';
import type { OpenCodeProvider } from '@/types/opencode';

import JsonEditor from '@/components/common/JsonEditor';
import JsonPreviewModal from '@/components/common/JsonPreviewModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchedModel } from '@/components/common/FetchModelsModal/types';
import AllApiHubIcon from '@/components/common/AllApiHubIcon';
import ConnectivityTestModal from '@/features/coding/opencode/components/ConnectivityTestModal';
import OpenClawProviderCard from '../components/OpenClawProviderCard';
import OpenClawProviderFormModal, {
  type ProviderFormValues,
} from '../components/OpenClawProviderFormModal';
import OpenClawModelFormModal, {
  type ModelFormValues,
} from '../components/OpenClawModelFormModal';
import ImportFromOpenCodeModal, {
  type ImportedProvider,
} from '../components/ImportFromOpenCodeModal';
import ImportFromAllApiHubModal from '../components/ImportFromAllApiHubModal';
import AgentsDefaultsCard, { type AgentsDefaultsCardRef } from '../components/AgentsDefaultsCard';
import OpenClawConfigPathModal from '../components/OpenClawConfigPathModal';
import { useRefreshStore } from '@/stores';
import type { OpenClawAllApiHubProvider } from '@/services/openclawApi';
import SectionSidebarLayout from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';

import styles from './OpenClawPage.module.less';

const { Title, Text, Link } = Typography;

const OPENAI_COMPATIBLE_NPM = '@ai-sdk/openai-compatible';
const OPENAI_RESPONSES_NPM = '@ai-sdk/openai';

/**
 * Map OpenClaw `api` protocol (+ optional baseUrl hint) to OpenCode `npm` SDK name.
 * Used by FetchModelsModal and ConnectivityTestModal.
 */
const apiToNpm = (api?: string, baseUrl?: string): string => {
  // Explicit protocol match
  if (api === 'openai-completions') return OPENAI_COMPATIBLE_NPM;
  if (api === 'openai-responses') return OPENAI_RESPONSES_NPM;
  if (api === 'anthropic-messages') return '@ai-sdk/anthropic';
  if (api === 'google-generative-ai') return '@ai-sdk/google';

  // Infer from baseUrl (covers old imports without api, or generic api like openai-completions)
  const url = (baseUrl || '').toLowerCase();
  if (url.includes('anthropic')) return '@ai-sdk/anthropic';
  if (url.includes('generativelanguage.googleapis.com') || url.includes('google')) return '@ai-sdk/google';

  return '@ai-sdk/openai-compatible';
};

const getOpenClawFetchedModelDefaultInput = (providerNpm?: string): string[] => (
  providerNpm === OPENAI_COMPATIBLE_NPM ? ['text'] : ['text', 'image']
);

const buildOpenClawModelFromPreset = (preset: PresetModel, fallbackName: string): OpenClawModel => ({
  id: preset.id,
  name: preset.name || fallbackName,
  contextWindow: preset.contextLimit,
  maxTokens: preset.outputLimit,
  reasoning: preset.reasoning ?? false,
  ...(preset.modalities?.input ? { input: preset.modalities.input } : {}),
});

const buildFetchedOpenClawModel = (
  fetchedModel: FetchedModel,
  providerNpm?: string,
): OpenClawModel => {
  const presetModels = providerNpm ? PRESET_MODELS[providerNpm] || [] : [];
  const matchedPresetModel = presetModels.find((presetModel) => presetModel.id === fetchedModel.id);

  if (matchedPresetModel) {
    return buildOpenClawModelFromPreset(matchedPresetModel, fetchedModel.name || fetchedModel.id);
  }

  return {
    id: fetchedModel.id,
    name: fetchedModel.name || fetchedModel.id,
    reasoning: true,
    input: getOpenClawFetchedModelDefaultInput(providerNpm),
  };
};

/**
 * Convert OpenClawProviderConfig to OpenCodeProvider shape for ConnectivityTestModal.
 */
const toOpenCodeProvider = (cfg: OpenClawProviderConfig): OpenCodeProvider => ({
  npm: apiToNpm(cfg.api, cfg.baseUrl),
  options: {
    baseURL: cfg.baseUrl || '',
    apiKey: cfg.apiKey,
  },
  models: Object.fromEntries(
    (cfg.models || []).map((m) => [m.id, { name: m.name || m.id }]),
  ),
});

const OpenClawPage: React.FC = () => {
  const { t } = useTranslation();
  const { openClawConfigRefreshKey } = useRefreshStore();

  // Drag sensors
  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  // Loading & config state
  const [loading, setLoading] = React.useState(false);
  const [config, setConfig] = React.useState<OpenClawConfig | null>(null);
  const [configPathInfo, setConfigPathInfo] = React.useState<OpenClawConfigPathInfo | null>(null);
  const [parseError, setParseError] = React.useState<{
    path: string;
    error: string;
    contentPreview?: string;
  } | null>(null);

  // Section data
  const [agentsDefaults, setAgentsDefaults] = React.useState<OpenClawAgentsDefaults | null>(null);

  // Modal states
  const [previewOpen, setPreviewOpen] = React.useState(false);
  const [configPathModalOpen, setConfigPathModalOpen] = React.useState(false);
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [editingProvider, setEditingProvider] = React.useState<{
    id: string;
    config: OpenClawProviderConfig;
  } | null>(null);
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [editingModel, setEditingModel] = React.useState<OpenClawModel | null>(null);
  const [modelTargetProvider, setModelTargetProvider] = React.useState<string>('');
  const [importModalOpen, setImportModalOpen] = React.useState(false);
  const [allApiHubImportModalOpen, setAllApiHubImportModalOpen] = React.useState(false);
  const [allApiHubAvailable, setAllApiHubAvailable] = React.useState(false);
  const [fetchModelsModalOpen, setFetchModelsModalOpen] = React.useState(false);
  const [fetchModelsProviderId, setFetchModelsProviderId] = React.useState<string>('');
  const [connectivityModalOpen, setConnectivityModalOpen] = React.useState(false);
  const [connectivityProviderId, setConnectivityProviderId] = React.useState<string>('');
  const [connectivityDiagnostics, setConnectivityDiagnostics] = React.useState<OpenCodeDiagnosticsConfig | undefined>(undefined);
  // Collapse states
  const [providersCollapsed, setProvidersCollapsed] = React.useState(false);
  const [agentsCollapsed, setAgentsCollapsed] = React.useState(false);
  const [otherCollapsed, setOtherCollapsed] = React.useState(true);

  // Refs
  const agentsDefaultsRef = React.useRef<AgentsDefaultsCardRef>(null);
  const otherConfigJsonValidRef = React.useRef(true);

  // ================================================================
  // Data loading
  // ================================================================
  const loadConfig = React.useCallback(async (silent = false) => {
    try {
      setLoading(true);
      setParseError(null);

      const [pathInfo, result] = await Promise.all([
        getOpenClawConfigPathInfo(),
        readOpenClawConfigWithResult(),
      ]);

      setConfigPathInfo(pathInfo);

      switch (result.status) {
        case 'success':
          setConfig(result.config);
          break;
        case 'notFound':
          setConfig(null);
          break;
        case 'parseError':
          setParseError({
            path: result.path,
            error: result.error,
            contentPreview: result.contentPreview,
          });
          setConfig(null);
          break;
        case 'error':
          if (!silent) {
            message.error(result.error);
          }
          setConfig(null);
          break;
      }
    } catch (error) {
      console.error('Failed to load config:', error);
      if (!silent) {
        message.error(t('common.error'));
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  const loadSectionData = React.useCallback(async () => {
    try {
      const defaults = await getOpenClawAgentsDefaults();
      setAgentsDefaults(defaults);
    } catch (error) {
      console.error('Failed to load section data:', error);
    }
  }, []);

  React.useEffect(() => {
    void openClawConfigRefreshKey;
    loadConfig();
    loadSectionData();
  }, [loadConfig, loadSectionData, openClawConfigRefreshKey]);

  React.useEffect(() => {
    void openClawConfigRefreshKey;
    const checkAllApiHubAvailability = async () => {
      try {
        const available = await hasAllApiHubExtension();
        setAllApiHubAvailable(available);
      } catch (error) {
        console.error('Failed to check All API Hub availability for OpenClaw:', error);
        setAllApiHubAvailable(false);
      }
    };

    checkAllApiHubAvailability();
  }, [openClawConfigRefreshKey]);

  // Listen for config-changed events (from tray)
  React.useEffect(() => {
    const unlisten = listen('openclaw-config-changed', () => {
      loadConfig(true);
      loadSectionData();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [loadConfig, loadSectionData]);

  // ================================================================
  // Provider CRUD
  // ================================================================
  const providerEntries = React.useMemo(() => {
    if (!config?.models?.providers) return [];
    return Object.entries(config.models.providers);
  }, [config]);

  const handleAddProvider = () => {
    setEditingProvider(null);
    setProviderModalOpen(true);
  };

  const handleOpenImportModal = () => {
    setProviderModalOpen(false);
    setImportModalOpen(true);
  };

  const handleImportFromOpenCode = async (imported: ImportedProvider[]) => {
    try {
      const currentConfig = config || { models: { providers: {} } };
      const providers = { ...(currentConfig.models?.providers || {}) };

      for (const item of imported) {
        providers[item.providerId] = item.config;
      }

      const newConfig: OpenClawConfig = {
        ...currentConfig,
        models: {
          ...(currentConfig.models || {}),
          providers,
        },
      };

      await saveOpenClawConfig(newConfig);
      message.success(
        t('openclaw.providers.importSuccess', { count: imported.length }),
      );
      setImportModalOpen(false);
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from OpenCode:', error);
      message.error(t('common.error'));
    }
  };

  const handleImportFromAllApiHub = async (imported: OpenClawAllApiHubProvider[]) => {
    try {
      const currentConfig = config || { models: { providers: {} } };
      const providers = { ...(currentConfig.models?.providers || {}) };

      for (const item of imported) {
        providers[item.providerId] = item.config;
      }

      const newConfig: OpenClawConfig = {
        ...currentConfig,
        models: {
          ...(currentConfig.models || {}),
          providers,
        },
      };

      await saveOpenClawConfig(newConfig);
      message.success(
        t('openclaw.providers.importAllApiHubSuccess', { count: imported.length }),
      );
      setAllApiHubImportModalOpen(false);
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to import from All API Hub:', error);
      message.error(t('common.error'));
    }
  };

  const handleEditProvider = (providerId: string, providerConfig: OpenClawProviderConfig) => {
    setEditingProvider({ id: providerId, config: providerConfig });
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = async (providerId: string) => {
    if (!config) return;
    try {
      const newProviders = { ...(config.models?.providers || {}) };
      delete newProviders[providerId];

      const newConfig: OpenClawConfig = {
        ...config,
        models: {
          ...(config.models || {}),
          providers: newProviders,
        },
      };

      await saveOpenClawConfig(newConfig);
      message.success(t('common.success'));
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to delete provider:', error);
      message.error(t('common.error'));
    }
  };

  const handleProviderSubmit = async (values: ProviderFormValues) => {
    try {
      const currentConfig = config || {
        models: { providers: {} },
      };
      const providers = { ...(currentConfig.models?.providers || {}) };

      const providerConfig: OpenClawProviderConfig = editingProvider
        ? { ...editingProvider.config }
        : { models: [] };

      if (values.baseUrl) providerConfig.baseUrl = values.baseUrl;
      else delete providerConfig.baseUrl;
      if (values.apiKey) providerConfig.apiKey = values.apiKey;
      else delete providerConfig.apiKey;
      if (values.api) providerConfig.api = values.api;
      else delete providerConfig.api;

      providers[values.providerId] = providerConfig;

      const newConfig: OpenClawConfig = {
        ...currentConfig,
        models: {
          ...(currentConfig.models || {}),
          providers,
        },
      };

      await saveOpenClawConfig(newConfig);
      message.success(t('common.success'));
      setProviderModalOpen(false);
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save provider:', error);
      message.error(t('common.error'));
    }
  };

  // ================================================================
  // Model CRUD
  // ================================================================
  const handleAddModel = (providerId: string) => {
    setModelTargetProvider(providerId);
    setEditingModel(null);
    setModelModalOpen(true);
  };

  const handleEditModel = (providerId: string, model: OpenClawModel) => {
    setModelTargetProvider(providerId);
    setEditingModel(model);
    setModelModalOpen(true);
  };

  const handleDeleteModel = async (providerId: string, modelId: string) => {
    if (!config?.models?.providers?.[providerId]) return;
    try {
      const provider = { ...config.models.providers[providerId] };
      provider.models = (provider.models || []).filter((m) => m.id !== modelId);

      const newConfig: OpenClawConfig = {
        ...config,
        models: {
          ...config.models,
          providers: {
            ...config.models.providers,
            [providerId]: provider,
          },
        },
      };

      await saveOpenClawConfig(newConfig);
      message.success(t('common.success'));
      loadConfig();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to delete model:', error);
      message.error(t('common.error'));
    }
  };

  const handleModelSubmit = async (values: ModelFormValues) => {
    if (!config?.models?.providers?.[modelTargetProvider]) return;
    try {
      const provider = { ...config.models.providers[modelTargetProvider] };
      const models = [...(provider.models || [])];

      // Build new model from form values + extra params (from JSON editor).
      // Preserve `input` and `alias` from the original model if they existed,
      // since these known fields don't have dedicated form controls.
      const preserved: Partial<OpenClawModel> = {};
      if (editingModel?.input) preserved.input = editingModel.input;
      if (editingModel?.alias) preserved.alias = editingModel.alias;

      const newModel: OpenClawModel = {
        ...preserved,
        id: values.id,
        name: values.name || undefined,
        contextWindow: values.contextWindow,
        maxTokens: values.maxTokens,
        reasoning: values.reasoning || undefined,
        cost:
          values.costInput !== undefined || values.costOutput !== undefined
            ? {
              input: values.costInput || 0,
              output: values.costOutput || 0,
              cacheRead: values.costCacheRead,
              cacheWrite: values.costCacheWrite,
            }
            : undefined,
        ...(values.extraParams || {}),
      };

      if (editingModel) {
        const idx = models.findIndex((m) => m.id === editingModel.id);
        if (idx >= 0) models[idx] = newModel;
      } else {
        models.push(newModel);
      }

      provider.models = models;

      const newConfig: OpenClawConfig = {
        ...config,
        models: {
          ...config.models,
          providers: {
            ...config.models.providers,
            [modelTargetProvider]: provider,
          },
        },
      };

      await saveOpenClawConfig(newConfig);
      message.success(t('common.success'));
      setModelModalOpen(false);
      loadConfig();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save model:', error);
      message.error(t('common.error'));
    }
  };

  // ================================================================
  // Connectivity Test & Fetch Models
  // ================================================================
  const handleOpenConnectivityTest = (providerId: string) => {
    setConnectivityProviderId(providerId);
    setConnectivityModalOpen(true);
  };

  const handleSaveDiagnostics = async (diagnostics: OpenCodeDiagnosticsConfig) => {
    setConnectivityDiagnostics(diagnostics);
  };

  const handleRemoveModels = async (modelIdsToRemove: string[]) => {
    if (!config || !connectivityProviderId) return;
    const provider = config.models?.providers?.[connectivityProviderId];
    if (!provider) return;

    const providers = config.models?.providers;
    if (!providers) return;

    const newModels = (provider.models || []).filter((m) => !modelIdsToRemove.includes(m.id));
    const newConfig: OpenClawConfig = {
      ...config,
      models: {
        ...config.models,
        providers: {
          ...providers,
          [connectivityProviderId]: { ...provider, models: newModels },
        },
      },
    };
    await saveOpenClawConfig(newConfig);
    loadConfig();
    refreshTrayMenu();
  };

  const connectivityProviderInfo = React.useMemo(() => {
    if (!config || !connectivityProviderId) return null;
    const provider = config.models?.providers?.[connectivityProviderId];
    if (!provider) return null;
    return {
      name: connectivityProviderId,
      config: toOpenCodeProvider(provider),
      modelIds: (provider.models || []).map((m) => m.id),
    };
  }, [config, connectivityProviderId]);

  const handleOpenFetchModels = (providerId: string) => {
    setFetchModelsProviderId(providerId);
    setFetchModelsModalOpen(true);
  };

  const fetchModelsProviderInfo = React.useMemo(() => {
    if (!config || !fetchModelsProviderId) return null;
    const provider = config.models?.providers?.[fetchModelsProviderId];
    if (!provider) return null;
    return {
      name: fetchModelsProviderId,
      baseUrl: provider.baseUrl || '',
      apiKey: provider.apiKey,
      sdkName: apiToNpm(provider.api, provider.baseUrl),
      existingModelIds: (provider.models || []).map((m) => m.id),
    };
  }, [config, fetchModelsProviderId]);

  const handleFetchModelsSuccess = async (selectedModels: FetchedModel[]) => {
    if (!config || !fetchModelsProviderId) return;
    const provider = config.models?.providers?.[fetchModelsProviderId];
    if (!provider) return;
    const providerNpm = apiToNpm(provider.api, provider.baseUrl);

    const providers = config.models?.providers;
    if (!providers) return;

    const newModels = [...(provider.models || [])];
    for (const model of selectedModels) {
      if (!newModels.find((m) => m.id === model.id)) {
        newModels.push(buildFetchedOpenClawModel(model, providerNpm));
      }
    }

    const newConfig: OpenClawConfig = {
      ...config,
      models: {
        ...config.models,
        providers: {
          ...providers,
          [fetchModelsProviderId]: { ...provider, models: newModels },
        },
      },
    };
    await saveOpenClawConfig(newConfig);
    setFetchModelsModalOpen(false);
    message.success(t('openclaw.providers.fetchModelsAddSuccess', { count: selectedModels.length }));
    loadConfig();
    refreshTrayMenu();
  };

  // ================================================================
  // Other config (flatten fields excluding known sections)
  // ================================================================
  const otherConfigFields = React.useMemo(() => {
    if (!config) return undefined;
    const { models, agents, ...rest } = config;
    return Object.keys(rest).length > 0 ? rest : undefined;
  }, [config]);

  const handleOtherConfigChange = (_value: unknown, isValid: boolean) => {
    otherConfigJsonValidRef.current = isValid;
  };

  const handleOtherConfigBlur = async (value: unknown) => {
    if (!config || !otherConfigJsonValidRef.current) return;
    try {
      const newConfig: OpenClawConfig = {
        models: config.models,
        agents: config.agents,
      };
      if (typeof value === 'object' && value !== null) {
        Object.assign(newConfig, value);
      }
      await saveOpenClawConfig(newConfig);
      loadConfig();
      loadSectionData();
      refreshTrayMenu();
    } catch (error) {
      console.error('Failed to save other config:', error);
      message.error(t('common.error'));
    }
  };

  // ================================================================
  // Drag handlers
  // ================================================================
  const handleProviderDragEnd = async (event: DragEndEvent) => {
    if (!config?.models?.providers) return;
    const { active, over } = event;
    if (!over || active.id === over.id) return;

    const providerIds = Object.keys(config.models.providers);
    const oldIndex = providerIds.indexOf(active.id as string);
    const newIndex = providerIds.indexOf(over.id as string);
    const newOrder = arrayMove(providerIds, oldIndex, newIndex);

    const reordered: Record<string, OpenClawProviderConfig> = {};
    for (const key of newOrder) {
      reordered[key] = config.models.providers[key];
    }

    const newConfig: OpenClawConfig = {
      ...config,
      models: { ...config.models, providers: reordered },
    };
    await saveOpenClawConfig(newConfig);
    loadConfig();
    refreshTrayMenu();
  };

  const handleReorderModels = async (providerId: string, modelIds: string[]) => {
    if (!config?.models?.providers?.[providerId]) return;
    const provider = config.models.providers[providerId];
    const modelMap = new Map((provider.models || []).map((m) => [m.id, m]));
    const reordered = modelIds
      .map((id) => modelMap.get(id))
      .filter((m): m is OpenClawModel => Boolean(m));

    const newConfig: OpenClawConfig = {
      ...config,
      models: {
        ...config.models,
        providers: {
          ...config.models.providers,
          [providerId]: { ...provider, models: reordered },
        },
      },
    };
    await saveOpenClawConfig(newConfig);
    loadConfig();
    refreshTrayMenu();
  };

  // ================================================================
  // Handlers
  // ================================================================
  const handleRefresh = () => {
    loadConfig();
    loadSectionData();
  };

  const handleOpenFolder = async () => {
    if (configPathInfo?.path) {
      try {
        await revealItemInDir(configPathInfo.path);
      } catch (error) {
        console.error('Failed to open folder:', error);
      }
    }
  };

  const handleConfigPathSuccess = () => {
    setConfigPathModalOpen(false);
    loadConfig();
    loadSectionData();
  };

  const handleSectionSaved = () => {
    loadConfig();
    loadSectionData();
    refreshTrayMenu();
  };

  // ================================================================
  // Render
  // ================================================================
  return (
    <SectionSidebarLayout
      sidebarTitle={t('openclaw.title')}
      getIcon={(id) => {
        switch (id) {
          case 'openclaw-agents':
            return <RobotOutlined />;
          case 'openclaw-providers':
            return <DatabaseOutlined />;
          case 'openclaw-other':
            return <SettingOutlined />;
          default:
            return null;
        }
      }}
      onSectionSelect={(id) => {
        switch (id) {
          case 'openclaw-agents':
            setAgentsCollapsed(false);
            break;
          case 'openclaw-providers':
            setProvidersCollapsed(false);
            break;
          case 'openclaw-other':
            setOtherCollapsed(false);
            break;
          default:
            break;
        }
      }}
    >
      <div>
        {parseError ? (
          <Alert
            type="error"
            message={`Config parse error: ${parseError.error}`}
            description={
              <div>
                <Text code>{parseError.path}</Text>
                {parseError.contentPreview && (
                  <pre style={{ fontSize: 12, marginTop: 8, maxHeight: 200, overflow: 'auto' }}>
                    {parseError.contentPreview}
                  </pre>
                )}
                <Space style={{ marginTop: 8 }}>
                  <Button onClick={async () => { await backupOpenClawConfig(); loadConfig(); }}>
                    Backup & Reset
                  </Button>
                  <Button onClick={handleRefresh}>Retry</Button>
                </Space>
              </div>
            }
          />
        ) : (
          <>
            {/* ===== HEADER ===== */}
            <div style={{ marginBottom: 16 }}>
              <div>
                <div style={{ marginBottom: 8 }}>
                  <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
                    {t('openclaw.title')}
                  </Title>
                  <Link
                    type="secondary"
                    style={{ fontSize: 12 }}
                    onClick={(e) => {
                      e.stopPropagation();
                      openUrl('https://docs.openclaw.ai/concepts/model-providers');
                    }}
                  >
                    <LinkOutlined /> {t('openclaw.viewDocs')}
                  </Link>
                  <Link
                    type="secondary"
                    style={{ fontSize: 12, marginLeft: 16 }}
                    onClick={(e) => {
                      e.stopPropagation();
                      setPreviewOpen(true);
                    }}
                  >
                    <EyeOutlined /> {t('openclaw.previewConfig')}
                  </Link>
                </div>
                <Space>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {t('openclaw.configPath')}:
                  </Text>
                  {configPathInfo?.source === 'custom' && (
                    <Tag color="green" style={{ fontSize: 12 }}>custom</Tag>
                  )}
                  <Text code style={{ fontSize: 12 }}>
                    {configPathInfo?.path || '~/.openclaw/openclaw.json'}
                  </Text>
                  <Button
                    type="text"
                    size="small"
                    icon={<EditOutlined />}
                    onClick={() => setConfigPathModalOpen(true)}
                    style={{ padding: 0, fontSize: 12 }}
                  />
                  <Button
                    type="text"
                    size="small"
                    icon={<FolderOpenOutlined />}
                    onClick={handleOpenFolder}
                    style={{ padding: 0, fontSize: 12 }}
                  />
                  <Button
                    type="text"
                    size="small"
                    icon={<ReloadOutlined />}
                    onClick={handleRefresh}
                    style={{ padding: 0, fontSize: 12 }}
                  />
                </Space>
              </div>

              <div
                style={{
                  fontSize: 12,
                  color: 'var(--color-text-tertiary)',
                  borderLeft: '2px solid var(--color-border)',
                  paddingLeft: 8,
                  marginTop: 8,
                }}
              >
                {t('openclaw.configFileHint')}
              </div>
            </div>

            {/* ===== AGENTS DEFAULTS COLLAPSE ===== */}
            <div
              id="openclaw-agents"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.agents.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={agentsCollapsed ? [] : ['agents']}
                onChange={(keys) => setAgentsCollapsed(!keys.includes('agents'))}
                items={[
                  {
                    key: 'agents',
                    label: (
                      <Text strong>
                        <RobotOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.agents.title')}
                      </Text>
                    ),
                    extra: (
                      <Button
                        type="link"
                        size="small"
                        icon={<MoreOutlined />}
                        onClick={(e) => {
                          e.stopPropagation();
                          agentsDefaultsRef.current?.openMoreParams();
                        }}
                      >
                        {t('openclaw.agents.moreParams')}
                      </Button>
                    ),
                    children: (
                      <AgentsDefaultsCard
                        ref={agentsDefaultsRef}
                        defaults={agentsDefaults}
                        config={config}
                        onSaved={handleSectionSaved}
                      />
                    ),
                  },
                ]}
              />
            </div>

            {/* ===== PROVIDERS COLLAPSE ===== */}
            <div
              id="openclaw-providers"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.providers.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={providersCollapsed ? [] : ['providers']}
                onChange={(keys) => setProvidersCollapsed(!keys.includes('providers'))}
                items={[
                  {
                    key: 'providers',
                    label: (
                      <Text strong>
                        <DatabaseOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.providers.title')}
                      </Text>
                    ),
                    extra: (
                      <Button
                        type="link"
                        size="small"
                        icon={<PlusOutlined />}
                        onClick={(e) => {
                          e.stopPropagation();
                          handleAddProvider();
                        }}
                      >
                        {t('openclaw.providers.addProvider')}
                      </Button>
                    ),
                    children: (
                      <Spin spinning={loading}>
                        {providerEntries.length === 0 ? (
                          <Empty description={t('openclaw.providers.emptyText')} />
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
                              {providerEntries.map(([providerId, providerConfig]) => (
                                <OpenClawProviderCard
                                  key={providerId}
                                  providerId={providerId}
                                  config={providerConfig}
                                  draggable
                                  sortableId={providerId}
                                  modelsDraggable
                                  onReorderModels={(modelIds) => handleReorderModels(providerId, modelIds)}
                                  onEdit={() => handleEditProvider(providerId, providerConfig)}
                                  onDelete={() => handleDeleteProvider(providerId)}
                                  onAddModel={() => handleAddModel(providerId)}
                                  onEditModel={(model) => handleEditModel(providerId, model)}
                                  onDeleteModel={(modelId) => handleDeleteModel(providerId, modelId)}
                                  onConnectivityTest={() => handleOpenConnectivityTest(providerId)}
                                  onFetchModels={() => handleOpenFetchModels(providerId)}
                                />
                              ))}
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
                              {t('openclaw.providers.importFromOpenCode')}
                            </Button>
                            {allApiHubAvailable && (
                              <Button
                                type="dashed"
                                icon={<AllApiHubIcon />}
                                onClick={() => setAllApiHubImportModalOpen(true)}
                              >
                                {t('openclaw.providers.importFromAllApiHub')}
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

            {/* ===== OTHER CONFIG COLLAPSE ===== */}
            <div
              id="openclaw-other"
              data-sidebar-section="true"
              data-sidebar-title={t('openclaw.other.title')}
            >
              <Collapse
                className={styles.collapseCard}
                activeKey={otherCollapsed ? [] : ['other']}
                onChange={(keys) => setOtherCollapsed(!keys.includes('other'))}
                items={[
                  {
                    key: 'other',
                    label: (
                      <Text strong>
                        <SettingOutlined style={{ marginRight: 8 }} />
                        {t('openclaw.other.title')}
                      </Text>
                    ),
                    children: (
                      <div>
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
    "env": {},
    "tools": { "profile": "default" }
}`}
                        />
                        <div style={{ marginTop: 8 }}>
                          <Text type="secondary">{t('openclaw.other.hint')}，</Text>
                          <span style={{ color: '#1677ff' }}>
                            {t('openclaw.other.autoSaveHint')}
                          </span>
                        </div>
                      </div>
                    ),
                  },
                ]}
              />
            </div>

            {/* ===== MODALS ===== */}
            <OpenClawProviderFormModal
              open={providerModalOpen}
              editingProvider={editingProvider}
              existingIds={providerEntries.map(([id]) => id)}
              onCancel={() => setProviderModalOpen(false)}
              onSubmit={handleProviderSubmit}
              onOpenImport={handleOpenImportModal}
            />

            <ImportFromOpenCodeModal
              open={importModalOpen}
              existingProviderIds={providerEntries.map(([id]) => id)}
              onCancel={() => setImportModalOpen(false)}
              onImport={handleImportFromOpenCode}
            />

            {allApiHubAvailable && (
              <ImportFromAllApiHubModal
                open={allApiHubImportModalOpen}
                existingProviderIds={providerEntries.map(([id]) => id)}
                onCancel={() => setAllApiHubImportModalOpen(false)}
                onImport={handleImportFromAllApiHub}
              />
            )}

            <OpenClawModelFormModal
              open={modelModalOpen}
              editingModel={editingModel}
              existingIds={
                config?.models?.providers?.[modelTargetProvider]?.models?.map((m) => m.id) || []
              }
              apiProtocol={config?.models?.providers?.[modelTargetProvider]?.api}
              onCancel={() => setModelModalOpen(false)}
              onSubmit={handleModelSubmit}
            />

            <OpenClawConfigPathModal
              open={configPathModalOpen}
              currentPathInfo={configPathInfo}
              onCancel={() => setConfigPathModalOpen(false)}
              onSuccess={handleConfigPathSuccess}
            />

            <JsonPreviewModal
              open={previewOpen}
              onClose={() => setPreviewOpen(false)}
              title={t('openclaw.previewConfig')}
              data={config}
            />

            {/* Fetch Models Modal (reuse common component) */}
            {fetchModelsProviderInfo && (
              <FetchModelsModal
                open={fetchModelsModalOpen}
                providerName={fetchModelsProviderInfo.name}
                baseUrl={fetchModelsProviderInfo.baseUrl}
                apiKey={fetchModelsProviderInfo.apiKey}
                sdkType={fetchModelsProviderInfo.sdkName}
                existingModelIds={fetchModelsProviderInfo.existingModelIds}
                onCancel={() => setFetchModelsModalOpen(false)}
                onSuccess={handleFetchModelsSuccess}
              />
            )}

            {/* Connectivity Test Modal (reuse OpenCode component) */}
            {connectivityProviderInfo && (
              <ConnectivityTestModal
                open={connectivityModalOpen}
                onCancel={() => setConnectivityModalOpen(false)}
                providerId={connectivityProviderId}
                providerName={connectivityProviderInfo.name}
                providerConfig={connectivityProviderInfo.config}
                modelIds={connectivityProviderInfo.modelIds}
                diagnostics={connectivityDiagnostics}
                onSaveDiagnostics={handleSaveDiagnostics}
                onRemoveModels={handleRemoveModels}
              />
            )}
          </>
        )}
      </div>
    </SectionSidebarLayout>
  );
};

export default OpenClawPage;
