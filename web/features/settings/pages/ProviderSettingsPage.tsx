import React from 'react';
import { Button, Card, Empty, Space, Typography, Popconfirm, message, Spin, Tag } from 'antd';
import { PlusOutlined, EditOutlined, DeleteOutlined, HolderOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
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
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import {
  getAllProvidersWithModels,
  deleteProvider,
  deleteModel,
  reorderProviders,
  reorderModels,
} from '@/services/providerApi';
import type { Provider, Model, ProviderWithModels } from '@/types/provider';
import ProviderFormModal from '../components/ProviderFormModal';
import ModelFormModal from '../components/ModelFormModal';

const { Title, Text } = Typography;

const AI_SDK_DOCS_URL = 'https://ai-sdk.dev/docs/foundations/providers-and-models#ai-sdk-providers';

// Sortable Provider Card Component
interface SortableProviderCardProps {
  providerWithModels: ProviderWithModels;
  onEditProvider: (provider: Provider) => void;
  onDeleteProvider: (provider: Provider) => void;
  onAddModel: (providerId: string) => void;
  onEditModel: (model: Model) => void;
  onDeleteModel: (providerId: string, model: Model) => void;
  onReorderModels: (providerId: string, models: Model[]) => void;
}

const SortableProviderCard: React.FC<SortableProviderCardProps> = ({
  providerWithModels,
  onEditProvider,
  onDeleteProvider,
  onAddModel,
  onEditModel,
  onDeleteModel,
  onReorderModels,
}) => {
  const { t } = useTranslation();
  const { provider, models } = providerWithModels;

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: provider.id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  // Model drag sensors
  const modelSensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  const handleModelDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;

    if (over && active.id !== over.id) {
      const oldIndex = models.findIndex((m) => m.id === active.id);
      const newIndex = models.findIndex((m) => m.id === over.id);

      const newModels = arrayMove(models, oldIndex, newIndex);
      onReorderModels(provider.id, newModels);
    }
  };

  return (
    <div ref={setNodeRef} style={style}>
      <Card
        style={{ marginBottom: 16 }}
        styles={{
          body: { padding: 16 },
        }}
      >
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
          <div
            {...attributes}
            {...listeners}
            style={{
              cursor: 'grab',
              padding: '4px 0',
              display: 'flex',
              alignItems: 'center',
            }}
          >
            <HolderOutlined style={{ fontSize: 16, color: '#999' }} />
          </div>

          <div style={{ flex: 1 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 12 }}>
              <div>
                <Title level={5} style={{ margin: 0, marginBottom: 4 }}>
                  {provider.name}
                </Title>
                <Space size={8}>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    ID: {provider.id}
                  </Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    •
                  </Text>
                  <Tag
                    color="blue"
                    style={{ margin: 0, cursor: 'pointer' }}
                    onClick={(e) => {
                      e.stopPropagation();
                      openUrl(AI_SDK_DOCS_URL);
                    }}
                  >
                    {provider.provider_type}
                  </Tag>
                </Space>
                <div style={{ marginTop: 4 }}>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {provider.base_url}
                  </Text>
                </div>
              </div>

              <Space>
                <Button
                  size="small"
                  icon={<EditOutlined />}
                  onClick={() => onEditProvider(provider)}
                />
                <Popconfirm
                  title={t('settings.provider.deleteProvider')}
                  description={t('settings.provider.confirmDelete', { name: provider.name })}
                  onConfirm={() => onDeleteProvider(provider)}
                  okText={t('common.confirm')}
                  cancelText={t('common.cancel')}
                >
                  <Button size="small" danger icon={<DeleteOutlined />} />
                </Popconfirm>
              </Space>
            </div>

            <div
              style={{
                background: '#fafafa',
                borderRadius: 4,
                padding: 12,
              }}
            >
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
                <Text strong style={{ fontSize: 13 }}>
                  {t('settings.model.title')}
                </Text>
                <Button
                  size="small"
                  type="link"
                  icon={<PlusOutlined />}
                  onClick={() => onAddModel(provider.id)}
                >
                  {t('settings.model.addModel')}
                </Button>
              </div>

              {models.length === 0 ? (
                <Empty
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                  description={t('settings.model.emptyText')}
                  style={{ margin: '8px 0' }}
                />
              ) : (
                <DndContext
                  sensors={modelSensors}
                  collisionDetection={closestCenter}
                  onDragEnd={handleModelDragEnd}
                >
                  <SortableContext
                    items={models.map((m) => m.id)}
                    strategy={verticalListSortingStrategy}
                  >
                    <Space direction="vertical" style={{ width: '100%' }} size={4}>
                      {models.map((model) => (
                        <SortableModelItem
                          key={model.id}
                          model={model}
                          providerId={provider.id}
                          onEdit={onEditModel}
                          onDelete={onDeleteModel}
                        />
                      ))}
                    </Space>
                  </SortableContext>
                </DndContext>
              )}
            </div>
          </div>
        </div>
      </Card>
    </div>
  );
};

// Sortable Model Item Component
interface SortableModelItemProps {
  model: Model;
  providerId: string;
  onEdit: (model: Model) => void;
  onDelete: (providerId: string, model: Model) => void;
}

const SortableModelItem: React.FC<SortableModelItemProps> = ({
  model,
  providerId,
  onEdit,
  onDelete,
}) => {
  const { t } = useTranslation();

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: model.id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  return (
    <div
      ref={setNodeRef}
      style={{
        ...style,
        background: '#fff',
        border: '1px solid #e8e8e8',
        borderRadius: 4,
        padding: '8px 12px',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
      }}
    >
      <div
        {...attributes}
        {...listeners}
        style={{
          cursor: 'grab',
          display: 'flex',
          alignItems: 'center',
        }}
      >
        <HolderOutlined style={{ fontSize: 14, color: '#bbb' }} />
      </div>

      <div style={{ flex: 1 }}>
        <div>
          <Text strong style={{ fontSize: 13 }}>
            {model.name}
          </Text>
          <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
            ({model.id})
          </Text>
        </div>
        <div style={{ marginTop: 2 }}>
          <Text type="secondary" style={{ fontSize: 11 }}>
            {t('settings.model.contextLimit')}: {model.context_limit.toLocaleString()} | {t('settings.model.outputLimit')}: {model.output_limit.toLocaleString()}
          </Text>
        </div>
      </div>

      <Space>
        <Button size="small" type="text" icon={<EditOutlined />} onClick={() => onEdit(model)} />
        <Popconfirm
          title={t('settings.model.deleteModel')}
          description={t('settings.model.confirmDelete', { name: model.name })}
          onConfirm={() => onDelete(providerId, model)}
          okText={t('common.confirm')}
          cancelText={t('common.cancel')}
        >
          <Button size="small" type="text" danger icon={<DeleteOutlined />} />
        </Popconfirm>
      </Space>
    </div>
  );
};

// Main Page Component
const ProviderSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [providersWithModels, setProvidersWithModels] = React.useState<ProviderWithModels[]>([]);

  const [providerModalOpen, setProviderModalOpen] = React.useState(false);
  const [currentProvider, setCurrentProvider] = React.useState<Provider | null>(null);

  const [modelModalOpen, setModelModalOpen] = React.useState(false);
  const [currentProviderId, setCurrentProviderId] = React.useState<string>('');
  const [currentModel, setCurrentModel] = React.useState<Model | null>(null);

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

  const handleAddProvider = () => {
    setCurrentProvider(null);
    setProviderModalOpen(true);
  };

  const handleEditProvider = (provider: Provider) => {
    setCurrentProvider(provider);
    setProviderModalOpen(true);
  };

  const handleDeleteProvider = async (provider: Provider) => {
    try {
      await deleteProvider(provider.id);
      message.success(t('common.success'));
      await loadData();
    } catch (error) {
      message.error(t('common.error'));
    }
  };

  const handleAddModel = (providerId: string) => {
    setCurrentProviderId(providerId);
    setCurrentModel(null);
    setModelModalOpen(true);
  };

  const handleEditModel = (model: Model) => {
    setCurrentProviderId(model.provider_id);
    setCurrentModel(model);
    setModelModalOpen(true);
  };

  const handleDeleteModel = async (providerId: string, model: Model) => {
    try {
      await deleteModel(providerId, model.id);
      message.success(t('common.success'));
      await loadData();
    } catch (error) {
      message.error(t('common.error'));
    }
  };

  const handleProviderDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;

    if (over && active.id !== over.id) {
      const oldIndex = providersWithModels.findIndex((p) => p.provider.id === active.id);
      const newIndex = providersWithModels.findIndex((p) => p.provider.id === over.id);

      const newProviders = arrayMove(providersWithModels, oldIndex, newIndex);
      setProvidersWithModels(newProviders);

      try {
        await reorderProviders(newProviders.map((p) => p.provider.id));
      } catch (error) {
        message.error(t('common.error'));
        await loadData();
      }
    }
  };

  const handleReorderModels = async (providerId: string, models: Model[]) => {
    // Optimistic update
    setProvidersWithModels((prev) =>
      prev.map((p) =>
        p.provider.id === providerId ? { ...p, models } : p
      )
    );

    try {
      await reorderModels(providerId, models.map((m) => m.id));
    } catch (error) {
      message.error(t('common.error'));
      await loadData();
    }
  };

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
                <SortableProviderCard
                  key={item.provider.id}
                  providerWithModels={item}
                  onEditProvider={handleEditProvider}
                  onDeleteProvider={handleDeleteProvider}
                  onAddModel={handleAddModel}
                  onEditModel={handleEditModel}
                  onDeleteModel={handleDeleteModel}
                  onReorderModels={handleReorderModels}
                />
              ))}
            </SortableContext>
          </DndContext>
        )}
      </Spin>

      <ProviderFormModal
        open={providerModalOpen}
        provider={currentProvider}
        onCancel={() => setProviderModalOpen(false)}
        onSuccess={() => {
          setProviderModalOpen(false);
          loadData();
        }}
      />

      <ModelFormModal
        open={modelModalOpen}
        providerId={currentProviderId}
        model={currentModel}
        onCancel={() => setModelModalOpen(false)}
        onSuccess={() => {
          setModelModalOpen(false);
          loadData();
        }}
      />
    </div>
  );
};

export default ProviderSettingsPage;
