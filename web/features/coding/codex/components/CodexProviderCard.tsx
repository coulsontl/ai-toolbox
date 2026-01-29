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
import type { CodexProvider, CodexSettingsConfig } from '@/types/codex';
import { extractCodexBaseUrl, extractCodexModel } from '@/utils/codexConfigUtils';

const { Text } = Typography;

interface CodexProviderCardProps {
  provider: CodexProvider;
  isApplied: boolean;
  onEdit: (provider: CodexProvider) => void;
  onDelete: (provider: CodexProvider) => void;
  onCopy: (provider: CodexProvider) => void;
  onSelect: (provider: CodexProvider) => void;
  onToggleDisabled: (provider: CodexProvider, isDisabled: boolean) => void;
}

const CodexProviderCard: React.FC<CodexProviderCardProps> = ({
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
    onToggleDisabled(provider, !checked);
  };

  // Parse settingsConfig JSON string
  const settingsConfig: CodexSettingsConfig = React.useMemo(() => {
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
              {provider.isDisabled ? t('codex.configDisabled') : t('codex.configEnabled')}
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

  // Extract display info from config
  const apiKey = settingsConfig.auth?.OPENAI_API_KEY;
  const maskedApiKey = apiKey ? `${apiKey.slice(0, 8)}...${apiKey.slice(-4)}` : null;

  // Extract base_url and model from config.toml using utility function
  const baseUrl = React.useMemo(() => {
    const configContent = settingsConfig.config || '';
    return extractCodexBaseUrl(configContent);
  }, [settingsConfig.config]);

  const modelName = React.useMemo(() => {
    const configContent = settingsConfig.config || '';
    return extractCodexModel(configContent);
  }, [settingsConfig.config]);

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
              {/* Provider name and status */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Text strong style={{ fontSize: 14 }}>
                  {provider.name}
                </Text>
                {provider.id === '__local__' && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    ({t('codex.localConfigHint')})
                  </Text>
                )}
                {isApplied && (
                  <Tag color="green" icon={<CheckCircleOutlined />}>
                    {t('codex.provider.applied')}
                  </Tag>
                )}
              </div>

              {/* Base URL, Model, API Key */}
              {(maskedApiKey || baseUrl || modelName || provider.notes) && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                  {baseUrl && (
                    <Text code style={{ fontSize: 11, padding: '0 4px' }}>
                      {baseUrl}
                    </Text>
                  )}
                  {modelName && (
                    <Tag color="blue" style={{ fontSize: 11, margin: 0 }}>
                      {modelName}
                    </Tag>
                  )}
                  {(baseUrl || modelName) && maskedApiKey && (
                    <Text type="secondary" style={{ fontSize: 12 }}>|</Text>
                  )}
                  {maskedApiKey && (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      API Key: {maskedApiKey}
                    </Text>
                  )}
                  {(baseUrl || modelName || maskedApiKey) && provider.notes && (
                    <Text type="secondary" style={{ fontSize: 12 }}>|</Text>
                  )}
                  {provider.notes && (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {provider.notes}
                    </Text>
                  )}
                </div>
              )}
            </Space>
          </div>

          {/* Action buttons */}
          <Space>
            {!isApplied && (
              <Button type="link" size="small" onClick={() => onSelect(provider)} disabled={provider.isDisabled}>
                {t('codex.provider.apply')}
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

export default CodexProviderCard;
