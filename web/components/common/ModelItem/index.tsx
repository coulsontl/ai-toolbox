import React from 'react';
import { Button, Space, Typography, Popconfirm } from 'antd';
import { EditOutlined, DeleteOutlined, HolderOutlined, CopyOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { ModelDisplayData, I18nPrefix } from '@/components/common/ProviderCard/types';

const { Text } = Typography;

interface ModelItemProps {
  model: ModelDisplayData;
  
  /** Whether the item is draggable */
  draggable?: boolean;
  /** Unique ID for sortable (defaults to model.id) */
  sortableId?: string;
  
  /** Callbacks */
  onEdit?: () => void;
  onCopy?: () => void;
  onDelete?: () => void;
  
  /** i18n prefix for translations */
  i18nPrefix?: I18nPrefix;
}

/**
 * A reusable model list item component with optional drag-and-drop support
 */
const ModelItem: React.FC<ModelItemProps> = ({
  model,
  draggable = false,
  sortableId,
  onEdit,
  onCopy,
  onDelete,
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
    id: sortableId || model.id,
    disabled: !draggable,
  });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
    background: '#fff',
    border: '1px solid #e8e8e8',
    borderRadius: 4,
    padding: '8px 12px',
    display: 'flex',
    alignItems: 'center',
    gap: 8,
  };

  const hasLimits = model.contextLimit !== undefined || model.outputLimit !== undefined;

  return (
    <div ref={setNodeRef} style={style}>
      {draggable && (
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
      )}

      <div style={{ flex: 1 }}>
        <div>
          <Text strong style={{ fontSize: 13 }}>
            {model.name}
          </Text>
          <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
            ({model.id})
          </Text>
        </div>
        {hasLimits && (
          <div style={{ marginTop: 2 }}>
            <Text type="secondary" style={{ fontSize: 11 }}>
              {model.contextLimit !== undefined && `${t(`${i18nPrefix}.model.contextLimit`)}: ${model.contextLimit.toLocaleString()}`}
              {model.contextLimit !== undefined && model.outputLimit !== undefined && ' | '}
              {model.outputLimit !== undefined && `${t(`${i18nPrefix}.model.outputLimit`)}: ${model.outputLimit.toLocaleString()}`}
            </Text>
          </div>
        )}
      </div>

      <Space>
        {onEdit && (
          <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit} />
        )}
        {onCopy && (
          <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy} />
        )}
        {onDelete && (
          <Popconfirm
            title={t(`${i18nPrefix}.model.deleteModel`)}
            description={t(`${i18nPrefix}.model.confirmDelete`, { name: model.name })}
            onConfirm={onDelete}
            okText={t('common.confirm')}
            cancelText={t('common.cancel')}
          >
            <Button size="small" type="text" danger icon={<DeleteOutlined />} />
          </Popconfirm>
        )}
      </Space>
    </div>
  );
};

export default ModelItem;
