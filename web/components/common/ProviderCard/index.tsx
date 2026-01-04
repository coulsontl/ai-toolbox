import React from 'react';
import { Button, Card, Empty, Space, Typography, Popconfirm, Collapse } from 'antd';
import { PlusOutlined, EditOutlined, DeleteOutlined, HolderOutlined, CopyOutlined } from '@ant-design/icons';
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
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import SdkTag from '@/components/common/SdkTag';
import ModelItem from '@/components/common/ModelItem';
import type { ProviderDisplayData, ModelDisplayData, I18nPrefix } from './types';

const { Title, Text } = Typography;

interface ProviderCardProps {
  provider: ProviderDisplayData;
  models: ModelDisplayData[];
  
  /** Whether the card is draggable */
  draggable?: boolean;
  /** Unique ID for sortable (defaults to provider.id) */
  sortableId?: string;
  
  /** Provider action callbacks */
  onEdit?: () => void;
  onCopy?: () => void;
  onDelete?: () => void;
  /** Extra action buttons (e.g., "Save to Settings" button for OpenCode) */
  extraActions?: React.ReactNode;
  
  /** Model action callbacks */
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  
  /** Model drag-and-drop */
  modelsDraggable?: boolean;
  onReorderModels?: (modelIds: string[]) => void;
  
  /** i18n prefix for translations */
  i18nPrefix?: I18nPrefix;
}

/**
 * A reusable provider card component with optional drag-and-drop support
 */
const ProviderCard: React.FC<ProviderCardProps> = ({
  provider,
  models,
  draggable = false,
  sortableId,
  onEdit,
  onCopy,
  onDelete,
  extraActions,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  modelsDraggable = false,
  onReorderModels,
  i18nPrefix = 'settings',
}) => {
  const { t } = useTranslation();
  
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ 
    id: sortableId || provider.id,
    disabled: !draggable,
  });

  const style: React.CSSProperties = {
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
      onReorderModels?.(newModels.map((m) => m.id));
    }
  };

  const renderModelList = () => {
    if (models.length === 0) {
      return (
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description={t(`${i18nPrefix}.model.emptyText`)}
          style={{ margin: '8px 0' }}
        />
      );
    }

    const modelItems = models.map((model) => (
      <ModelItem
        key={model.id}
        model={model}
        draggable={modelsDraggable}
        sortableId={model.id}
        onEdit={onEditModel ? () => onEditModel(model.id) : undefined}
        onCopy={onCopyModel ? () => onCopyModel(model.id) : undefined}
        onDelete={onDeleteModel ? () => onDeleteModel(model.id) : undefined}
        i18nPrefix={i18nPrefix}
      />
    ));

    if (modelsDraggable) {
      return (
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
              {modelItems}
            </Space>
          </SortableContext>
        </DndContext>
      );
    }

    return (
      <Space direction="vertical" style={{ width: '100%' }} size={4}>
        {modelItems}
      </Space>
    );
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
          {draggable && (
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
          )}

          <div style={{ flex: 1 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 8 }}>
              <div>
                <Title level={5} style={{ margin: 0, marginBottom: 4 }}>
                  {provider.name}
                </Title>
                <Space size={8} wrap>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    ID: {provider.id}
                  </Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    •
                  </Text>
                  <SdkTag name={provider.sdkName} />
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    •
                  </Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {provider.baseUrl}
                  </Text>
                </Space>
              </div>

              <Space>
                {extraActions}
                {onEdit && (
                  <Button
                    size="small"
                    icon={<EditOutlined />}
                    onClick={onEdit}
                  />
                )}
                {onCopy && (
                  <Button
                    size="small"
                    icon={<CopyOutlined />}
                    onClick={onCopy}
                  />
                )}
                {onDelete && (
                  <Popconfirm
                    title={t(`${i18nPrefix}.provider.deleteProvider`)}
                    description={t(`${i18nPrefix}.provider.confirmDelete`, { name: provider.name })}
                    onConfirm={onDelete}
                    okText={t('common.confirm')}
                    cancelText={t('common.cancel')}
                  >
                    <Button size="small" danger icon={<DeleteOutlined />} />
                  </Popconfirm>
                )}
              </Space>
            </div>

            <Collapse
              defaultActiveKey={[]}
              ghost
              style={{ marginTop: 8 }}
              items={[
                {
                  key: provider.id,
                  label: (
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', width: '100%' }}>
                      <Text strong style={{ fontSize: 13 }}>
                        {t(`${i18nPrefix}.model.title`)} ({models.length})
                      </Text>
                      {onAddModel && (
                        <Button
                          size="small"
                          type="link"
                          icon={<PlusOutlined />}
                          onClick={(e) => {
                            e.stopPropagation();
                            onAddModel();
                          }}
                        >
                          {t(`${i18nPrefix}.model.addModel`)}
                        </Button>
                      )}
                    </div>
                  ),
                  children: renderModelList(),
                },
              ]}
            />
          </div>
        </div>
      </Card>
    </div>
  );
};

export default ProviderCard;
