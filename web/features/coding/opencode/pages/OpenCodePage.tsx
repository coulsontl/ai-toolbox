import React from 'react';
import { Button, Empty, Space, Typography, message, Spin, Select, Card, Collapse, Tag, Form } from 'antd';
import { PlusOutlined, FolderOpenOutlined, CodeOutlined, LinkOutlined, EyeOutlined, EditOutlined, EnvironmentOutlined, CloudDownloadOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { useNavigate, useLocation } from 'react-router-dom';
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
import { readOpenCodeConfig, saveOpenCodeConfig, getOpenCodeConfigPathInfo, type ConfigPathInfo } from '@/services/opencodeApi';
import type { OpenCodeConfig, OpenCodeProvider, OpenCodeModel } from '@/types/opencode';
import type { ProviderDisplayData, ModelDisplayData } from '@/components/common/ProviderCard/types';
import ProviderCard from '@/components/common/ProviderCard';
import ProviderFormModal, { ProviderFormValues } from '@/components/common/ProviderFormModal';
import ModelFormModal, { ModelFormValues } from '@/components/common/ModelFormModal';
import FetchModelsModal from '@/components/common/FetchModelsModal';
import type { FetchedModel } from '@/components/common/FetchModelsModal/types';
import PluginSettings from '../components/PluginSettings';
import McpSettings from '../components/McpSettings';
import ConfigPathModal from '../components/ConfigPathModal';
import OhMyOpenCodeConfigSelector from '../components/OhMyOpenCodeConfigSelector';
import OhMyOpenCodeSettings from '../components/OhMyOpenCodeSettings';
import JsonEditor from '@/components/common/JsonEditor';
import { usePreviewStore, useAppStore, useRefreshStore } from '@/stores';

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

const OpenCodePage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { setPreviewData } = usePreviewStore();
  const appStoreState = useAppStore.getState();
  const { openCodeConfigRefreshKey } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [config, setConfig] = React.useState<OpenCodeConfig | null>(null);
  const [configPathInfo, setConfigPathInfo] = React.useState<ConfigPathInfo | null>(null);

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

  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);
  const [pathModalOpen, setPathModalOpen] = React.useState(false);
  const [otherConfigCollapsed, setOtherConfigCollapsed] = React.useState(true);
  
  // Use ref for validation state to avoid re-renders during editing
  const otherConfigJsonValidRef = React.useRef(true);
  const [ohMyOpenCodeRefreshKey, setOhMyOpenCodeRefreshKey] = React.useState(0); // 用于触发 OhMyOpenCodeConfigSelector 刷新
  const [ohMyOpenCodeSettingsRefreshKey, setOhMyOpenCodeSettingsRefreshKey] = React.useState(0); // 用于触发 OhMyOpenCodeSettings 刷新

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const loadConfig = React.useCallback(async () => {
    setLoading(true);
    try {
      const pathInfo = await getOpenCodeConfigPathInfo();
      setConfigPathInfo(pathInfo);

      const data = await readOpenCodeConfig();
      if (data) {
        setConfig(data);
      } else {
        // Initialize empty config
        setConfig({
          $schema: 'https://opencode.ai/config.json',
          provider: {},
        });
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
    try {
      if (configPathInfo?.path) {
        await revealItemInDir(configPathInfo.path);
      }
    } catch (error) {
      console.error('Failed to open folder:', error);
      message.error(t('common.error'));
    }
  };

  const handlePathModalSuccess = () => {
    setPathModalOpen(false);
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

    setCurrentProviderId(providerId);
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
    });
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (providerId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    setCurrentProviderId('');
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

    const newProvider: OpenCodeProvider = {
      npm: values.sdkType || '@ai-sdk/openai-compatible',
      name: values.name,
      options: {
        baseURL: values.baseUrl,
        ...(values.apiKey && { apiKey: values.apiKey }),
        ...(values.headers && { headers: values.headers as Record<string, string> }),
        ...(values.disableTimeout 
          ? { timeout: false as const } 
          : values.timeout !== undefined && { timeout: values.timeout }),
        ...(values.setCacheKey !== undefined && { setCacheKey: values.setCacheKey }),
      },
      models: currentProviderId ? config.provider[currentProviderId]?.models || {} : {},
    };

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [values.id]: newProvider,
      },
    });

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

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [currentModelProviderId]: {
          ...provider,
          models: {
            ...provider.models,
            [values.id]: newModel,
          },
        },
      },
    });

    setModelModalOpen(false);
    setModelInitialValues(undefined);
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

    await doSaveConfig({
      ...config,
      provider: {
        ...config.provider,
        [fetchModelsProviderId]: {
          ...provider,
          models: newModels,
        },
      },
    });

    setFetchModelsModalOpen(false);
    message.success(t('opencode.fetchModels.addSuccess', { count: selectedModels.length }));
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
    appStoreState.setCurrentModule('coding');
    appStoreState.setCurrentSubTab('opencode');
    setPreviewData(t('opencode.preview.title'), config, location.pathname);
    navigate('/preview/config');
  };

  const providerEntries = config && config.provider ? Object.entries(config.provider) : [];
  const existingProviderIds = providerEntries.map(([id]) => id);
  const existingModelIds = React.useMemo(() => {
    if (!config || !config.provider || !currentModelProviderId) return [];
    const provider = config.provider[currentModelProviderId];
    return provider && provider.models ? Object.keys(provider.models) : [];
  }, [config, currentModelProviderId]);

  // Collect all available models for model selectors
  const modelOptions = React.useMemo(() => {
    if (!config || !config.provider) return [];
    const options: { label: string; value: string }[] = [];

    Object.entries(config.provider).forEach(([providerId, provider]) => {
      if (!provider || !provider.models) return;

      Object.keys(provider.models).forEach((modelId) => {
        const model = provider.models[modelId];
        options.push({
          label: `${provider.name || providerId} / ${model.name || modelId}`,
          value: `${providerId}/${modelId}`,
        });
      });
    });

    return options;
  }, [config]);

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

  const handleMcpChange = async (mcp: Record<string, import('@/types/opencode').McpServerConfig>) => {
    if (!config) return;
    
    await doSaveConfig({
      ...config,
      mcp: Object.keys(mcp).length > 0 ? mcp : undefined,
    });
  };

  // Extract other config fields (unknown fields)
  const otherConfigFields = React.useMemo(() => {
    if (!config) return {};
    const knownFields = ['$schema', 'provider', 'model', 'small_model', 'plugin', 'mcp'];
    const other: Record<string, unknown> = {};
    Object.keys(config).forEach((key) => {
      if (!knownFields.includes(key)) {
        other[key] = config[key];
      }
    });
    return other;
  }, [config]);

  const handleOtherConfigChange = async (value: unknown, isValid: boolean) => {
    otherConfigJsonValidRef.current = isValid;
    
    if (!config || !isValid) {
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
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 16 }}>
        <div>
          <div style={{ marginBottom: 8 }}>
            <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
              <CodeOutlined style={{ marginRight: 8 }} />
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
              <Tag bordered style={{ fontSize: 12, backgroundColor: '#f0f0f0', color: 'rgba(0, 0, 0, 0.65)', borderColor: '#d9d9d9' }}>
                {t('opencode.configPathSource.default')}
              </Tag>
            )}
            <Text code style={{ fontSize: 12 }}>
              {configPathInfo?.path}
            </Text>
            <Button
              type="link"
              size="small"
              icon={<EditOutlined />}
              onClick={() => setPathModalOpen(true)}
              style={{ padding: 0, fontSize: 12 }}
            >
              {t('opencode.configPathSource.customize')}
            </Button>
            <Button
              type="link"
              size="small"
              icon={<FolderOpenOutlined />}
              onClick={handleOpenConfigFolder}
              style={{ padding: 0, fontSize: 12 }}
            >
              {t('opencode.openFolder')}
            </Button>
          </Space>
        </div>
      </div>

      <Card
        title={t('opencode.modelSettings.title')}
        style={{ marginBottom: 16 }}
        size="small"
      >
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

{/* Oh My OpenCode Config Selector - only show when plugin is selected */}
          {/* Use startsWith to support versioned plugins like "oh-my-opencode@3.0.0-beta.5" */}
          {config?.plugin?.some((p) => p.startsWith('oh-my-opencode')) && (
            <div>
              <div style={{ marginBottom: 4 }}>
                <Text strong>{t('opencode.ohMyOpenCode.configLabel')}</Text>
                <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                  {t('opencode.ohMyOpenCode.configHint')}
                </Text>
              </div>
              <OhMyOpenCodeConfigSelector
                key={ohMyOpenCodeRefreshKey} // 当 key 改变时，组件会重新挂载并刷新
                onConfigSelected={() => {
                  message.success(t('opencode.ohMyOpenCode.configSelected'));
                  // 当在快速切换框中选择配置时，触发设置列表刷新
                  setOhMyOpenCodeSettingsRefreshKey((prev) => prev + 1);
                }}
              />
            </div>
          )}
        </Space>
      </Card>

      <PluginSettings
        plugins={config?.plugin || []}
        onChange={handlePluginChange}
      />

{/* Oh My OpenCode Settings - only show when plugin is selected */}
      {/* Use startsWith to support versioned plugins like "oh-my-opencode@3.0.0-beta.5" */}
      {config?.plugin?.some((p) => p.startsWith('oh-my-opencode')) && (
        <OhMyOpenCodeSettings
          key={ohMyOpenCodeSettingsRefreshKey} // 当 key 改变时，组件会重新挂载并刷新
          modelOptions={modelOptions}
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

      <McpSettings
        mcp={config?.mcp || {}}
        onChange={handleMcpChange}
      />

      <Collapse
        style={{ marginBottom: 16 }}
        activeKey={providerListCollapsed ? [] : ['providers']}
        onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
        items={[
          {
            key: 'providers',
            label: (
              <Text strong>{t('opencode.provider.title')}</Text>
            ),
            extra: (
              <Button
                type="primary"
                size="small"
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
                    onDragEnd={handleProviderDragEnd}
                  >
                    <SortableContext
                      items={providerEntries.map(([id]) => id)}
                      strategy={verticalListSortingStrategy}
                    >
                      {providerEntries.map(([providerId, provider]) => (
                        <ProviderCard
                          key={providerId}
                          provider={toProviderDisplayData(providerId, provider)}
                          models={provider.models ? Object.entries(provider.models).map(([modelId, model]) =>
                            toModelDisplayData(modelId, model)
                          ) : []}
                          draggable
                          sortableId={providerId}
                          onEdit={() => handleEditProvider(providerId)}
                          onCopy={() => handleCopyProvider(providerId)}
                          onDelete={() => handleDeleteProvider(providerId)}
                          extraActions={
                            <Button
                              size="small"
                              type="text"
                              onClick={() => handleOpenFetchModels(providerId)}
                            >
                              <CloudDownloadOutlined style={{ marginRight: 0 }} />
                              {t('opencode.fetchModels.button')}
                            </Button>
                          }
                          onAddModel={() => handleAddModel(providerId)}
                          onEditModel={(modelId) => handleEditModel(providerId, modelId)}
                          onCopyModel={(modelId) => handleCopyModel(providerId, modelId)}
                          onDeleteModel={(modelId) => handleDeleteModel(providerId, modelId)}
                          modelsDraggable
                          onReorderModels={(modelIds) => handleReorderModels(providerId, modelIds)}
                          i18nPrefix="opencode"
                        />
                      ))}
                    </SortableContext>
                  </DndContext>
                )}
              </Spin>
            ),
          },
        ]}
      />

      <Collapse
        style={{ marginBottom: 16 }}
        activeKey={otherConfigCollapsed ? [] : ['other']}
        onChange={(keys) => setOtherConfigCollapsed(!keys.includes('other'))}
        items={[
          {
            key: 'other',
            label: <Text strong>{t('opencode.otherConfig.title')}</Text>,
            children: (
              <div>
                <Text type="secondary" style={{ display: 'block', marginBottom: 12, fontSize: 12 }}>
                  {t('opencode.otherConfig.hint')}
                </Text>
                <Form.Item
                  help={t('opencode.otherConfig.hint')}
                  style={{ marginBottom: 0 }}
                >
                  <JsonEditor
                    value={otherConfigFields}
                    onChange={handleOtherConfigChange}
                    height={300}
                    minHeight={200}
                    maxHeight={500}
                    resizable
                    mode="text"
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
    </div>
  );
};

export default OpenCodePage;
