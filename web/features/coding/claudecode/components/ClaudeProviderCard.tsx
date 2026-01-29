import React from 'react';
import { Card, Space, Button, Dropdown, Tag, Typography, Switch, message } from 'antd';
import {
  EditOutlined,
  DeleteOutlined,
  CopyOutlined,
  MoreOutlined,
  CheckCircleOutlined,
  HolderOutlined,
} from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { ClaudeCodeProvider } from '@/types/claudecode';

const { Text } = Typography;

interface ClaudeProviderCardProps {
  provider: ClaudeCodeProvider;
  isApplied: boolean;
  onEdit: (provider: ClaudeCodeProvider) => void;
  onDelete: (provider: ClaudeCodeProvider) => void;
  onCopy: (provider: ClaudeCodeProvider) => void;
  onSelect: (provider: ClaudeCodeProvider) => void;
  onToggleDisabled: (provider: ClaudeCodeProvider, isDisabled: boolean) => void;
}

const ClaudeProviderCard: React.FC<ClaudeProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
  onSelect,
  onToggleDisabled,
}) => {
  const { t } = useTranslation();

  // 拖拽排序
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: provider.id });

  const sortableStyle = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : (provider.isDisabled ? 0.6 : 1),
  };

  const handleToggleDisabled = (checked: boolean) => {
    if (isApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !checked);  // Switch 的 checked 表示"启用"，所以取反
  };

  // 解析 settingsConfig JSON 字符串
  const settingsConfig = React.useMemo(() => {
    try {
      return JSON.parse(provider.settingsConfig);
    } catch (error) {
      console.error('Failed to parse settingsConfig:', error);
      return {};
    }
  }, [provider.settingsConfig]);

  const menuItems: MenuProps['items'] = [
    {
      key: 'toggle',
      label: (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span>{t('common.enable')}</span>
            <Text type="secondary" style={{ fontSize: 11 }}>
              {provider.isDisabled ? t('claudecode.configDisabled') : t('claudecode.configEnabled')}
            </Text>
          </div>
          <Switch
            checked={!provider.isDisabled}
            onChange={handleToggleDisabled}
            size="small"
          />
        </div>
      ),
    },
    {
      key: 'edit',
      label: t('common.edit'),
      icon: <EditOutlined />,
      onClick: () => onEdit(provider),
    },
    {
      key: 'copy',
      label: t('common.copy'),
      icon: <CopyOutlined />,
      onClick: () => onCopy(provider),
    },
    // Hide delete button for __local__ provider
    ...(provider.id !== '__local__' ? [
      {
        type: 'divider' as const,
      },
      {
        key: 'delete',
        label: t('common.delete'),
        icon: <DeleteOutlined />,
        danger: true,
        onClick: () => onDelete(provider),
      },
    ] : []),
  ].filter(Boolean) as MenuProps['items'];

  const hasModels =
    settingsConfig.haikuModel ||
    settingsConfig.sonnetModel ||
    settingsConfig.opusModel;

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <Card
        size="small"
        style={{
          marginBottom: 12,
          borderColor: isApplied ? '#1890ff' : 'var(--color-border-card)',
          backgroundColor: isApplied ? 'var(--color-bg-selected)' : undefined,
          transition: 'opacity 0.3s ease, border-color 0.2s ease',
        }}
        styles={{ body: { padding: 16 } }}
      >
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
          <div style={{ flex: 1, display: 'flex', alignItems: 'flex-start', gap: 8 }}>
            {/* 拖拽手柄 */}
            <div
              {...attributes}
              {...listeners}
              style={{
                cursor: isDragging ? 'grabbing' : 'grab',
                color: '#999',
                padding: '4px 0',
                touchAction: 'none',
              }}
            >
              <HolderOutlined />
            </div>
            <Space direction="vertical" size={4} style={{ width: '100%' }}>
            {/* 供应商名称、状态和 URL */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
              <Text strong style={{ fontSize: 14 }}>
                {provider.name}
              </Text>
              {provider.id === '__local__' && (
                <Text type="secondary" style={{ fontSize: 11 }}>
                  ({t('claudecode.localConfigHint')})
                </Text>
              )}
              {settingsConfig.env?.ANTHROPIC_BASE_URL && (
                <Text type="secondary" style={{ fontSize: 11 }}>
                  {settingsConfig.env.ANTHROPIC_BASE_URL}
                </Text>
              )}
              {isApplied && (
                <Tag color="green" icon={<CheckCircleOutlined />}>
                  {t('claudecode.provider.applied')}
                </Tag>
              )}
            </div>

            {/* 备注 */}
            {provider.notes && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {provider.notes}
                </Text>
              </div>
            )}

            {/* 所有模型配置 - 整体换行展示 */}
            {(settingsConfig.model || hasModels) && (
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '8px 16px', marginTop: 4 }}>
                {settingsConfig.model && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('claudecode.provider.defaultModel')}:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {settingsConfig.model}
                    </Text>
                  </div>
                )}
                {settingsConfig.haikuModel && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      Haiku:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {settingsConfig.haikuModel}
                    </Text>
                  </div>
                )}
                {settingsConfig.sonnetModel && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      Sonnet:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {settingsConfig.sonnetModel}
                    </Text>
                  </div>
                )}
                {settingsConfig.opusModel && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      Opus:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {settingsConfig.opusModel}
                    </Text>
                  </div>
                )}
              </div>
            )}
        </Space>
        </div>

        {/* 操作按钮 */}
        <Space>
          {!isApplied && (
            <Button
              type="link"
              size="small"
              onClick={() => onSelect(provider)}
              disabled={provider.isDisabled}
            >
              {t('claudecode.provider.apply')}
            </Button>
          )}
          <Dropdown menu={{ items: menuItems }} trigger={['click']}>
            <Button type="text" size="small" icon={<MoreOutlined />} />
          </Dropdown>
        </Space>
      </div>
    </Card>
    </div>
  );
};

export default ClaudeProviderCard;
