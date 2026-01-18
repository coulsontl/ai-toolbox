import React from 'react';
import { Card, Space, Button, Dropdown, Tag, Typography } from 'antd';
import {
  EditOutlined,
  DeleteOutlined,
  CopyOutlined,
  MoreOutlined,
  CheckCircleOutlined,
  EyeOutlined,
} from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { useTranslation } from 'react-i18next';
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
  onPreview?: (provider: CodexProvider) => void;
}

const CodexProviderCard: React.FC<CodexProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
  onSelect,
  onPreview,
}) => {
  const { t } = useTranslation();

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
      key: 'edit',
      label: t('codex.provider.editProvider'),
      icon: <EditOutlined />,
      onClick: () => onEdit(provider),
    },
    {
      key: 'preview',
      label: t('codex.provider.previewConfig'),
      icon: <EyeOutlined />,
      onClick: () => onPreview?.(provider),
    },
    {
      key: 'copy',
      label: t('codex.provider.copyProvider'),
      icon: <CopyOutlined />,
      onClick: () => onCopy(provider),
    },
    {
      type: 'divider',
    },
    {
      key: 'delete',
      label: t('codex.provider.deleteProvider'),
      icon: <DeleteOutlined />,
      danger: true,
      onClick: () => onDelete(provider),
    },
  ];

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
    <Card
      size="small"
      style={{
        marginBottom: 12,
        borderColor: isApplied ? '#1890ff' : 'rgb(228, 228, 231)',
        backgroundColor: isApplied ? '#fff' : undefined,
      }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div style={{ flex: 1 }}>
          <Space direction="vertical" size={4} style={{ width: '100%' }}>
            {/* Provider name and status */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Text strong style={{ fontSize: 14 }}>
                {provider.name}
              </Text>
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
            <Button type="primary" size="small" onClick={() => onSelect(provider)}>
              {t('codex.provider.enable')}
            </Button>
          )}
          <Dropdown menu={{ items: menuItems }} trigger={['click']}>
            <Button type="text" size="small" icon={<MoreOutlined />} />
          </Dropdown>
        </Space>
      </div>
    </Card>
  );
};

export default CodexProviderCard;
