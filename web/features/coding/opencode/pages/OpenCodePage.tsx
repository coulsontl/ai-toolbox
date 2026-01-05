import React from 'react';
import { Button, Empty, Space, Typography, message, Spin, Tooltip, Modal, Select, Card, Collapse } from 'antd';
import { PlusOutlined, FolderOpenOutlined, SyncOutlined, CodeOutlined, SaveOutlined, QuestionCircleOutlined, EyeOutlined } from '@ant-design/icons';
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
import { readOpenCodeConfig, saveOpenCodeConfig, getOpenCodeConfigPath } from '@/services/opencodeApi';
import { listProviders, listModels, createProvider, updateProvider, createModel, updateModel } from '@/services/providerApi';
import type { OpenCodeConfig, OpenCodeProvider, OpenCodeModel } from '@/types/opencode';
import type { Provider } from '@/types/provider';
import type { ProviderDisplayData, ModelDisplayData } from '@/components/common/ProviderCard/types';
import ProviderCard from '@/components/common/ProviderCard';
import ProviderFormModal, { ProviderFormValues } from '@/components/common/ProviderFormModal';
import ModelFormModal, { ModelFormValues } from '@/components/common/ModelFormModal';
import SyncFromSettingsModal from '../components/SyncFromSettingsModal';
import { usePreviewStore, useAppStore } from '@/stores';

const { Title, Text } = Typography;

// Helper function to convert OpenCodeProvider to ProviderDisplayData
const toProviderDisplayData = (id: string, provider: OpenCodeProvider): ProviderDisplayData => ({
  id,
  name: provider.name || id,
  sdkName: provider.npm,
  baseUrl: provider.options.baseURL,
});

// Helper function to convert OpenCodeModel to ModelDisplayData
const toModelDisplayData = (id: string, model: OpenCodeModel): ModelDisplayData => ({
  id,
  name: model.name,
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
  const [loading, setLoading] = React.useState(false);
  const [config, setConfig] = React.useState<OpenCodeConfig | null>(null);
  const [configPath, setConfigPath] = React.useState<string>('');

  // Provider modal state
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [currentProviderId, setCurrentProviderId] = React.useState<string>('');
  const [providerInitialValues, setProviderInitialValues] = React.useState<Partial<ProviderFormValues> | undefined>();

  // Model modal state
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [currentModelProviderId, setCurrentModelProviderId] = React.useState<string>('');
  const [currentModelId, setCurrentModelId] = React.useState<string>('');
  const [modelInitialValues, setModelInitialValues] = React.useState<Partial<ModelFormValues> | undefined>();

  const [syncModalOpen, setSyncModalOpen] = React.useState(false);
  const [providerListCollapsed, setProviderListCollapsed] = React.useState(false);

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const loadConfig = React.useCallback(async () => {
    setLoading(true);
    try {
      const path = await getOpenCodeConfigPath();
      setConfigPath(path);

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
  }, [loadConfig]);

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
      await revealItemInDir(configPath);
    } catch (error) {
      console.error('Failed to open folder:', error);
      message.error(t('common.error'));
    }
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
      sdkType: provider.npm,
      baseUrl: provider.options.baseURL,
      apiKey: provider.options.apiKey || '',
      headers: provider.options.headers,
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
      baseUrl: provider.options.baseURL,
      apiKey: provider.options.apiKey || '',
      headers: provider.options.headers,
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
      npm: values.sdkType,
      name: values.name,
      options: {
        baseURL: values.baseUrl,
        ...(values.apiKey && { apiKey: values.apiKey }),
        ...(values.headers && { headers: values.headers as Record<string, string> }),
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
      name: values.name,
      ...(values.contextLimit || values.outputLimit
        ? {
            limit: {
              ...(values.contextLimit && { context: values.contextLimit }),
              ...(values.outputLimit && { output: values.outputLimit }),
            },
          }
        : {}),
      ...(values.options ? { options: JSON.parse(values.options) } : {}),
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

  const handleSyncSuccess = async (newConfig: OpenCodeConfig) => {
    await doSaveConfig(newConfig);
    setSyncModalOpen(false);
  };

  const handlePreviewConfig = async () => {
    if (!config) return;
    appStoreState.setCurrentModule('coding');
    appStoreState.setCurrentSubTab('opencode');
    setPreviewData(t('opencode.preview.title'), config, location.pathname);
    navigate('/preview/config');
  };

  const handleSaveToSettings = async (providerId: string) => {
    if (!config) return;
    const provider = config.provider[providerId];
    if (!provider) return;

    try {
      const existingProviders = await listProviders();
      const existingProvider = existingProviders.find((p) => p.id === providerId);

      if (existingProvider) {
        Modal.confirm({
          title: t('opencode.sync.conflictTitle'),
          content: t('opencode.sync.conflictDescription', { name: provider.name }),
          okText: t('opencode.sync.replaceProvider'),
          cancelText: t('opencode.sync.addModelsOnly'),
          onOk: async () => {
            await doSaveToSettings(providerId, provider, existingProvider, 'replace');
          },
          onCancel: async () => {
            await doSaveToSettings(providerId, provider, existingProvider, 'addModels');
          },
        });
      } else {
        await doSaveToSettings(providerId, provider, null, 'create');
      }
    } catch (error) {
      console.error('Failed to save to settings:', error);
      message.error(t('common.error'));
    }
  };

  const doSaveToSettings = async (
    providerId: string,
    provider: OpenCodeProvider,
    existingProvider: Provider | null,
    mode: 'create' | 'replace' | 'addModels'
  ) => {
    try {
      if (mode === 'create') {
        const existingProviders = await listProviders();
        await createProvider({
          id: providerId,
          name: provider.name || providerId,
          provider_type: provider.npm,
          base_url: provider.options.baseURL,
          api_key: provider.options.apiKey || '',
          headers: provider.options.headers ? JSON.stringify(provider.options.headers) : undefined,
          sort_order: existingProviders.length,
        });
      } else if (mode === 'replace' && existingProvider) {
        await updateProvider({
          ...existingProvider,
          name: provider.name || providerId,
          provider_type: provider.npm,
          base_url: provider.options.baseURL,
          api_key: provider.options.apiKey || existingProvider.api_key,
          headers: provider.options.headers ? JSON.stringify(provider.options.headers) : existingProvider.headers,
        });
      }

      // Get existing models
      const existingModels = await listModels(providerId);
      const existingModelsMap = new Map(existingModels.map((m) => [m.id, m]));

      // Create or update models
      const modelEntries = Object.entries(provider.models);
      let addedCount = 0;
      for (let i = 0; i < modelEntries.length; i++) {
        const [modelId, model] = modelEntries[i];
        const existingModel = existingModelsMap.get(modelId);

        if (existingModel) {
          // Update existing model (only in replace mode)
          if (mode === 'replace') {
            await updateModel({
              ...existingModel,
              name: model.name,
              context_limit: model.limit?.context || existingModel.context_limit,
              output_limit: model.limit?.output || existingModel.output_limit,
              options: model.options ? JSON.stringify(model.options) : existingModel.options,
            });
          }
        } else {
          // Create new model
          await createModel({
            id: modelId,
            provider_id: providerId,
            name: model.name,
            context_limit: model.limit?.context || 128000,
            output_limit: model.limit?.output || 8000,
            options: model.options ? JSON.stringify(model.options) : '{}',
            sort_order: existingModels.length + addedCount,
          });
          addedCount++;
        }
      }

      message.success(t('opencode.sync.saveSuccess'));
    } catch (error) {
      console.error('Failed to save to settings:', error);
      message.error(t('common.error'));
    }
  };

  const providerEntries = config ? Object.entries(config.provider) : [];
  const existingProviderIds = providerEntries.map(([id]) => id);
  const existingModelIds = React.useMemo(() => {
    if (!config || !currentModelProviderId) return [];
    const provider = config.provider[currentModelProviderId];
    return provider ? Object.keys(provider.models) : [];
  }, [config, currentModelProviderId]);

  // Collect all available models for model selectors
  const modelOptions = React.useMemo(() => {
    if (!config) return [];
    const options: { label: string; value: string }[] = [];
    
    Object.entries(config.provider).forEach(([providerId, provider]) => {
      Object.keys(provider.models).forEach((modelId) => {
        options.push({
          label: `${provider.name} / ${provider.models[modelId].name}`,
          value: `${providerId}/${modelId}`,
        });
      });
    });
    
    return options;
  }, [config]);

  const handleModelChange = async (field: 'model' | 'small_model', value: string | undefined) => {
    if (!config) return;
    
    await doSaveConfig({
      ...config,
      [field]: value || undefined,
    });
  };

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 16 }}>
        <div>
          <Title level={4} style={{ margin: 0, marginBottom: 8 }}>
            <CodeOutlined style={{ marginRight: 8 }} />
            {t('opencode.title')}
          </Title>
          <Space>
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t('opencode.configPath')}:
            </Text>
            <Text code style={{ fontSize: 12 }}>
              {configPath}
            </Text>
            <Button
              type="link"
              size="small"
              icon={<FolderOpenOutlined />}
              onClick={handleOpenConfigFolder}
              style={{ padding: 0 }}
            >
              {t('opencode.openFolder')}
            </Button>
            <Button
              type="link"
              size="small"
              icon={<QuestionCircleOutlined />}
              onClick={() => openUrl('https://opencode.ai/docs/config/#format')}
              style={{ padding: 0 }}
            >
              {t('opencode.viewDocs')}
            </Button>
            <Button
              type="link"
              size="small"
              icon={<EyeOutlined />}
              onClick={handlePreviewConfig}
              style={{ padding: 0 }}
            >
              {t('common.previewConfig')}
            </Button>
          </Space>
        </div>
        <Space>
          <Button icon={<SyncOutlined />} onClick={() => setSyncModalOpen(true)}>
            {t('opencode.syncFromSettings')}
          </Button>
          <Button type="primary" icon={<PlusOutlined />} onClick={handleAddProvider}>
            {t('opencode.addProvider')}
          </Button>
        </Space>
      </div>

      <Card
        title={t('opencode.modelSettings.title')}
        style={{ marginBottom: 16 }}
        size="small"
      >
        <Space direction="vertical" style={{ width: '100%' }} size={12}>
          <div>
            <div style={{ marginBottom: 4 }}>
              <Text strong>{t('opencode.modelSettings.modelLabel')}</Text>
            </div>
            <Select
              value={config?.model}
              onChange={(value) => handleModelChange('model', value)}
              placeholder={t('opencode.modelSettings.modelPlaceholder')}
              allowClear
              options={modelOptions}
              style={{ width: '100%' }}
              notFoundContent={t('opencode.modelSettings.noModels')}
            />
          </div>
          
          <div>
            <div style={{ marginBottom: 4 }}>
              <Space>
                <Text strong>{t('opencode.modelSettings.smallModelLabel')}</Text>
              </Space>
              <div style={{ marginTop: 4 }}>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('opencode.modelSettings.smallModelHint')}
                </Text>
              </div>
            </div>
            <Select
              value={config?.small_model}
              onChange={(value) => handleModelChange('small_model', value)}
              placeholder={t('opencode.modelSettings.smallModelPlaceholder')}
              allowClear
              options={modelOptions}
              style={{ width: '100%' }}
              notFoundContent={t('opencode.modelSettings.noModels')}
            />
          </div>
        </Space>
      </Card>

      <Collapse 
        style={{ marginBottom: 16 }}
        ghost
        activeKey={providerListCollapsed ? [] : ['providers']}
        onChange={(keys) => setProviderListCollapsed(!keys.includes('providers'))}
        items={[
          {
            key: 'providers',
            label: (
              <Text strong>{t('opencode.provider.title')}</Text>
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
                          models={Object.entries(provider.models).map(([modelId, model]) => 
                            toModelDisplayData(modelId, model)
                          )}
                          draggable
                          sortableId={providerId}
                          onEdit={() => handleEditProvider(providerId)}
                          onCopy={() => handleCopyProvider(providerId)}
                          onDelete={() => handleDeleteProvider(providerId)}
                          extraActions={
                            <Tooltip title={t('opencode.sync.saveToSettings')}>
                              <Button
                                size="small"
                                icon={<SaveOutlined />}
                                onClick={() => handleSaveToSettings(providerId)}
                              />
                            </Tooltip>
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
      />

      <ModelFormModal
        open={modelModalOpen}
        isEdit={!!currentModelId}
        initialValues={modelInitialValues}
        existingIds={currentModelId ? [] : existingModelIds}
        showOptions
        limitRequired={false}
        onCancel={() => {
          setModelModalOpen(false);
          setModelInitialValues(undefined);
        }}
        onSuccess={handleModelSuccess}
        onDuplicateId={handleModelDuplicateId}
        i18nPrefix="opencode"
      />

      <SyncFromSettingsModal
        open={syncModalOpen}
        currentConfig={config || { provider: {} }}
        onCancel={() => setSyncModalOpen(false)}
        onSuccess={handleSyncSuccess}
      />
    </div>
  );
};

export default OpenCodePage;
