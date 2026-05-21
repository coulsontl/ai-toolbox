import React from 'react';
import { Card, Space, Button, Dropdown, Tag, Typography, Switch, Tooltip, message } from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
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
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';

const { Text } = Typography;

interface ClaudeProviderCardProps {
  provider: ClaudeCodeProvider;
  isApplied: boolean;
  onEdit: (provider: ClaudeCodeProvider) => void;
  onDelete: (provider: ClaudeCodeProvider) => void;
  onCopy: (provider: ClaudeCodeProvider) => void;
  onTest: (provider: ClaudeCodeProvider) => void;
  onSelect: (provider: ClaudeCodeProvider) => void;
  onToggleDisabled: (provider: ClaudeCodeProvider, isDisabled: boolean) => void;
  connectivityStatus?: ProviderConnectivityStatusItem;
  gatewayTakeoverActive?: boolean;
}

const ClaudeProviderCard: React.FC<ClaudeProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
  onTest,
  onSelect,
  onToggleDisabled,
  connectivityStatus,
  gatewayTakeoverActive = false,
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

  const configuredModelIds = React.useMemo(
    () =>
      Array.from(
        new Set(
          [
            settingsConfig.model,
            settingsConfig.haikuModel,
            settingsConfig.sonnetModel,
            settingsConfig.opusModel,
            settingsConfig.reasoningModel,
          ].filter((modelId): modelId is string => Boolean(modelId?.trim()))
        )
      ),
    [settingsConfig]
  );
  const configuredApiKey =
    settingsConfig.env?.ANTHROPIC_AUTH_TOKEN?.trim() ||
    settingsConfig.env?.ANTHROPIC_API_KEY?.trim() ||
    '';
  const configuredBaseUrl = settingsConfig.env?.ANTHROPIC_BASE_URL?.trim() || '';
  const isOfficialProvider = provider.category === 'official';
  const requiresExplicitBaseUrl = !isOfficialProvider;
  const canRunConnectivityTest =
    !isOfficialProvider &&
    Boolean(configuredApiKey) &&
    configuredModelIds.length > 0 &&
    (!requiresExplicitBaseUrl || Boolean(configuredBaseUrl));

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
    settingsConfig.reasoningModel ||
    settingsConfig.haikuModel ||
    settingsConfig.sonnetModel ||
    settingsConfig.opusModel;
  const hasConfiguredModels = Boolean(settingsConfig.model || hasModels);
  const showRuntimeApplied = isApplied;
  const showApplyAction = !gatewayTakeoverActive && !isApplied;
  const actionAreaWidth = showApplyAction ? 112 : 40;

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <Card
        size="small"
        style={{
          marginBottom: 12,
          borderColor: showRuntimeApplied ? 'var(--ant-color-primary)' : 'var(--color-border-card)',
          backgroundColor: showRuntimeApplied ? 'var(--color-bg-selected)' : undefined,
          boxShadow: '0 2px 8px rgba(0, 0, 0, 0.06)',
          transition: 'opacity 0.3s ease, border-color 0.2s ease, box-shadow 0.2s ease',
        }}
        styles={{ body: { padding: 16 } }}
        onMouseEnter={(e) => {
          e.currentTarget.style.boxShadow = '0 4px 12px rgba(0, 0, 0, 0.1)';
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.boxShadow = '0 2px 8px rgba(0, 0, 0, 0.06)';
        }}
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
              <ProviderConnectivityStatus item={connectivityStatus} />
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
              {isOfficialProvider && (
                <Tag>{t('claudecode.provider.modeOfficial')}</Tag>
              )}
              {isOfficialProvider && gatewayTakeoverActive && (
                <Tooltip title={t('gateway.takeover.officialBypassedTooltip')}>
                  <Tag color="gold">{t('gateway.takeover.officialBypassedTag')}</Tag>
                </Tooltip>
              )}
              {showRuntimeApplied && (
                <Tag color="green" icon={<CheckCircleOutlined />}>
                  {t('claudecode.provider.applied')}
                </Tag>
              )}
            </div>

            <div style={{ display: 'flex', alignItems: 'flex-start', gap: '8px 16px', flexWrap: 'wrap', marginTop: 4 }}>
                {settingsConfig.model && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('claudecode.model.defaultLabel')}:
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
                {settingsConfig.reasoningModel && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('claudecode.model.reasoningLabel')}:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {settingsConfig.reasoningModel}
                    </Text>
                  </div>
                )}
                {!hasConfiguredModels && provider.notes && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {provider.notes}
                  </Text>
                )}
                <Text type="secondary" style={{ fontSize: 12 }}>|</Text>
              <Button
                type="text"
                size="small"
                icon={<ApiOutlined />}
                onClick={() => onTest(provider)}
                disabled={!canRunConnectivityTest}
                title={isOfficialProvider ? t('claudecode.provider.officialConnectivityHint') : undefined}
                style={{ fontSize: 12, padding: '0 4px', height: 'auto', flexShrink: 0 }}
              >
                {t('opencode.connectivity.button')}
              </Button>
            </div>

            {/* 备注 */}
            {provider.notes && hasConfiguredModels && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {provider.notes}
                </Text>
              </div>
            )}
        </Space>
        </div>

        {/* 操作按钮 */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'flex-end',
            gap: 8,
            width: actionAreaWidth,
            whiteSpace: 'nowrap',
          }}
        >
          {showApplyAction && (
            <Button
              type="link"
              size="small"
              icon={<CheckOutlined />}
              onClick={() => onSelect(provider)}
              disabled={provider.isDisabled}
            >
              {t('claudecode.provider.apply')}
            </Button>
          )}
          <Dropdown menu={{ items: menuItems }} trigger={['click']}>
            <Button type="text" size="small" icon={<MoreOutlined />} />
          </Dropdown>
        </div>
      </div>
    </Card>
    </div>
  );
};

export default ClaudeProviderCard;
