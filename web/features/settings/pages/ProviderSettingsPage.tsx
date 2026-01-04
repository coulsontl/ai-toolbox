import React from 'react';
import { Button, Empty, Typography, message, Spin } from 'antd';
import { PlusOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
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
import {
  getAllProvidersWithModels,
  deleteProvider,
  deleteModel,
  reorderProviders,
  reorderModels,
  createProvider,
  updateProvider,
  listProviders,
  createModel,
  updateModel,
  listModels,
} from '@/services/providerApi';
import type { Provider, Model, ProviderWithModels } from '@/types/provider';
import type { ProviderDisplayData, ModelDisplayData } from '@/components/common/ProviderCard/types';
import ProviderCard from '@/components/common/ProviderCard';
import ProviderFormModal, { ProviderFormValues } from '@/components/common/ProviderFormModal';
import ModelFormModal, { ModelFormValues } from '@/components/common/ModelFormModal';

const { Title } = Typography;

// Helper function to convert Provider to ProviderDisplayData
const toProviderDisplayData = (provider: Provider): ProviderDisplayData => ({
  id: provider.id,
  name: provider.name,
  sdkName: provider.provider_type,
  baseUrl: provider.base_url,
});

// Helper function to convert Model to ModelDisplayData
const toModelDisplayData = (model: Model): ModelDisplayData => ({
  id: model.id,
  name: model.name,
  contextLimit: model.context_limit,
  outputLimit: model.output_limit,
});

const ProviderSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [providersWithModels, setProvidersWithModels] = React.useState<ProviderWithModels[]>([]);

  // Provider modal state
  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [currentProvider, setCurrentProvider] = React.useState<Provider | null>(null);
  const [providerInitialValues, setProviderInitialValues] = React.useState<Partial<ProviderFormValues> | undefined>();

  // Model modal state
  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [currentProviderId, setCurrentProviderId] = React.useState<string>('');
  const [currentModel, setCurrentModel] = React.useState<Model | null>(null);
  const [modelInitialValues, setModelInitialValues] = React.useState<Partial<ModelFormValues> | undefined>();

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const loadData = React.useCallback(async () => {
    setLoading(true);
    try {
      const data = await getAllProvidersWithModels();
      setProvidersWithModels(data);
    } catch (error: unknown) {
      console.error('Failed to load providers:', error);
      const errorMessage = error instanceof Error 
        ? error.message 
        : typeof error === 'string' 
          ? error 
          : t('common.error');
      message.error(errorMessage);
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    loadData();
  }, [loadData]);

  // Provider handlers
  const handleAddProvider = () => {
    setCurrentProvider(null);
    setProviderInitialValues(undefined);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (providerId: string) => {
    const item = providersWithModels.find(p => p.provider.id === providerId);
    if (!item) return;
    
    const provider = item.provider;
    setCurrentProvider(provider);
    setProviderInitialValues({
      id: provider.id,
      name: provider.name,
      sdkType: provider.provider_type,
      baseUrl: provider.base_url,
      apiKey: provider.api_key,
      headers: provider.headers,
    });
    setProviderModalOpen(true);
  };

  const handleCopyProvider = (providerId: string) => {
    const item = providersWithModels.find(p => p.provider.id === providerId);
    if (!item) return;
    
    const provider = item.provider;
    setCurrentProvider(null);
    setProviderInitialValues({
      id: `${provider.id}_copy`,
      name: provider.name,
      sdkType: provider.provider_type,
      baseUrl: provider.base_url,
      apiKey: provider.api_key,
      headers: provider.headers,
    });
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = async (providerId: string) => {
    try {
      await deleteProvider(providerId);
      message.success(t('common.success'));
      await loadData();
    } catch {
      message.error(t('common.error'));
    }
  };

  const handleProviderSuccess = async (values: ProviderFormValues) => {
    try {
      if (currentProvider) {
        // Update existing provider
        await updateProvider({
          ...currentProvider,
          id: values.id,
          name: values.name,
          provider_type: values.sdkType,
          base_url: values.baseUrl,
          api_key: values.apiKey || '',
          headers: values.headers as string | undefined,
        });
      } else {
        // Create new provider
        const existingProviders = await listProviders();
        await createProvider({
          id: values.id,
          name: values.name,
          provider_type: values.sdkType,
          base_url: values.baseUrl,
          api_key: values.apiKey || '',
          headers: values.headers as string | undefined,
          sort_order: existingProviders.length,
        });
      }
      message.success(t('common.success'));
      setProviderModalOpen(false);
      setProviderInitialValues(undefined);
      await loadData();
    } catch (error) {
      console.error('Provider save error:', error);
      message.error(t('common.error'));
    }
  };

  const handleProviderDuplicateId = () => {
    message.error(t('settings.provider.idExists'));
  };

  // Model handlers
  const handleAddModel = (providerId: string) => {
    setCurrentProviderId(providerId);
    setCurrentModel(null);
    setModelInitialValues(undefined);
    setModelModalOpen(true);
  };

  const handleEditModel = (providerId: string, modelId: string) => {
    const item = providersWithModels.find(p => p.provider.id === providerId);
    if (!item) return;
    
    const model = item.models.find(m => m.id === modelId);
    if (!model) return;

    setCurrentProviderId(providerId);
    setCurrentModel(model);
    setModelInitialValues({
      id: model.id,
      name: model.name,
      contextLimit: model.context_limit,
      outputLimit: model.output_limit,
      options: model.options,
    });
    setModelModalOpen(true);
  };

  const handleCopyModel = (providerId: string, modelId: string) => {
    const item = providersWithModels.find(p => p.provider.id === providerId);
    if (!item) return;
    
    const model = item.models.find(m => m.id === modelId);
    if (!model) return;

    setCurrentProviderId(providerId);
    setCurrentModel(null);
    setModelInitialValues({
      id: `${model.id}_copy`,
      name: model.name,
      contextLimit: model.context_limit,
      outputLimit: model.output_limit,
      options: model.options,
    });
    setModelModalOpen(true);
  };

  const handleDeleteModel = async (providerId: string, modelId: string) => {
    try {
      await deleteModel(providerId, modelId);
      message.success(t('common.success'));
      await loadData();
    } catch {
      message.error(t('common.error'));
    }
  };

  const handleModelSuccess = async (values: ModelFormValues) => {
    try {
      if (currentModel) {
        // Update existing model
        await updateModel({
          ...currentModel,
          id: values.id,
          name: values.name,
          context_limit: values.contextLimit || 128000,
          output_limit: values.outputLimit || 8000,
          options: values.options || '{}',
        });
      } else {
        // Create new model
        const existingModels = await listModels(currentProviderId);
        await createModel({
          id: values.id,
          provider_id: currentProviderId,
          name: values.name,
          context_limit: values.contextLimit || 128000,
          output_limit: values.outputLimit || 8000,
          options: values.options || '{}',
          sort_order: existingModels.length,
        });
      }
      message.success(t('common.success'));
      setModelModalOpen(false);
      setModelInitialValues(undefined);
      await loadData();
    } catch (error) {
      console.error('Model save error:', error);
      message.error(t('common.error'));
    }
  };

  const handleModelDuplicateId = () => {
    message.error(t('settings.model.idExists'));
  };

  // Drag handlers
  const handleProviderDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;

    if (over && active.id !== over.id) {
      const oldIndex = providersWithModels.findIndex((p) => p.provider.id === active.id);
      const newIndex = providersWithModels.findIndex((p) => p.provider.id === over.id);

      const newProviders = arrayMove(providersWithModels, oldIndex, newIndex);
      setProvidersWithModels(newProviders);

      try {
        await reorderProviders(newProviders.map((p) => p.provider.id));
      } catch {
        message.error(t('common.error'));
        await loadData();
      }
    }
  };

  const handleReorderModels = async (providerId: string, modelIds: string[]) => {
    const item = providersWithModels.find(p => p.provider.id === providerId);
    if (!item) return;

    // Reorder models based on new IDs order
    const modelMap = new Map(item.models.map(m => [m.id, m]));
    const newModels = modelIds.map(id => modelMap.get(id)!).filter(Boolean);

    // Optimistic update
    setProvidersWithModels((prev) =>
      prev.map((p) =>
        p.provider.id === providerId ? { ...p, models: newModels } : p
      )
    );

    try {
      await reorderModels(providerId, modelIds);
    } catch {
      message.error(t('common.error'));
      await loadData();
    }
  };

  // Get existing IDs for duplicate check
  const existingProviderIds = providersWithModels.map(p => p.provider.id);
  const existingModelIds = React.useMemo(() => {
    const item = providersWithModels.find(p => p.provider.id === currentProviderId);
    return item ? item.models.map(m => m.id) : [];
  }, [providersWithModels, currentProviderId]);

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
        <Title level={4} style={{ margin: 0 }}>
          {t('settings.provider.title')}
        </Title>
        <Button type="primary" icon={<PlusOutlined />} onClick={handleAddProvider}>
          {t('settings.provider.addProvider')}
        </Button>
      </div>

      <Spin spinning={loading}>
        {providersWithModels.length === 0 ? (
          <Empty description={t('settings.provider.emptyText')} style={{ marginTop: 40 }} />
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleProviderDragEnd}
          >
            <SortableContext
              items={providersWithModels.map((p) => p.provider.id)}
              strategy={verticalListSortingStrategy}
            >
              {providersWithModels.map((item) => (
                <ProviderCard
                  key={item.provider.id}
                  provider={toProviderDisplayData(item.provider)}
                  models={item.models.map(toModelDisplayData)}
                  draggable
                  sortableId={item.provider.id}
                  onEdit={() => handleEditProvider(item.provider.id)}
                  onCopy={() => handleCopyProvider(item.provider.id)}
                  onDelete={() => handleDeleteProvider(item.provider.id)}
                  onAddModel={() => handleAddModel(item.provider.id)}
                  onEditModel={(modelId) => handleEditModel(item.provider.id, modelId)}
                  onCopyModel={(modelId) => handleCopyModel(item.provider.id, modelId)}
                  onDeleteModel={(modelId) => handleDeleteModel(item.provider.id, modelId)}
                  modelsDraggable
                  onReorderModels={(modelIds) => handleReorderModels(item.provider.id, modelIds)}
                  i18nPrefix="settings"
                />
              ))}
            </SortableContext>
          </DndContext>
        )}
      </Spin>

      <ProviderFormModal
        open={providerModalOpen}
        isEdit={!!currentProvider}
        initialValues={providerInitialValues}
        existingIds={currentProvider ? [] : existingProviderIds}
        apiKeyRequired
        onCancel={() => {
          setProviderModalOpen(false);
          setProviderInitialValues(undefined);
        }}
        onSuccess={handleProviderSuccess}
        onDuplicateId={handleProviderDuplicateId}
        i18nPrefix="settings"
        headersOutputFormat="string"
      />

      <ModelFormModal
        open={modelModalOpen}
        isEdit={!!currentModel}
        initialValues={modelInitialValues}
        existingIds={currentModel ? [] : existingModelIds}
        showOptions
        limitRequired
        onCancel={() => {
          setModelModalOpen(false);
          setModelInitialValues(undefined);
        }}
        onSuccess={handleModelSuccess}
        onDuplicateId={handleModelDuplicateId}
        i18nPrefix="settings"
      />
    </div>
  );
};

export default ProviderSettingsPage;
