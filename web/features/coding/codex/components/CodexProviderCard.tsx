import React from 'react';
import { Card, Space, Button, Dropdown, Tag, Typography, Switch, message } from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
  EditOutlined,
  DeleteOutlined,
  CopyOutlined,
  MoreOutlined,
  CheckCircleOutlined,
  HolderOutlined,
  DownOutlined,
  RightOutlined,
  LinkOutlined,
  SyncOutlined,
  EyeOutlined,
} from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { CodexOfficialAccount, CodexProvider, CodexSettingsConfig } from '@/types/codex';
import { extractCodexBaseUrl, extractCodexModel, extractCodexReasoningEffort } from '@/utils/codexConfigUtils';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';

const { Text } = Typography;

interface CodexProviderCardProps {
  provider: CodexProvider;
  isApplied: boolean;
  onEdit: (provider: CodexProvider) => void;
  onDelete: (provider: CodexProvider) => void;
  onCopy: (provider: CodexProvider) => void;
  onTest: (provider: CodexProvider) => void;
  onSelect: (provider: CodexProvider) => void;
  onToggleDisabled: (provider: CodexProvider, isDisabled: boolean) => void;
  officialAccounts?: CodexOfficialAccount[];
  onOfficialAccountLogin?: (provider: CodexProvider) => void;
  onOfficialLocalAccountSave?: (provider: CodexProvider, account: CodexOfficialAccount) => void;
  onOfficialAccountApply?: (provider: CodexProvider, account: CodexOfficialAccount) => void;
  onOfficialAccountDelete?: (provider: CodexProvider, account: CodexOfficialAccount) => void;
  onOfficialAccountRefresh?: (provider: CodexProvider, account: CodexOfficialAccount) => void;
  onOfficialAccountViewDetails?: (provider: CodexProvider, account: CodexOfficialAccount) => void;
  refreshingOfficialAccountId?: string | null;
  savingOfficialAccountId?: string | null;
  connectivityStatus?: ProviderConnectivityStatusItem;
}

const CodexProviderCard: React.FC<CodexProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
  onTest,
  onSelect,
  onToggleDisabled,
  officialAccounts = [],
  onOfficialAccountLogin,
  onOfficialLocalAccountSave,
  onOfficialAccountApply,
  onOfficialAccountDelete,
  onOfficialAccountRefresh,
  onOfficialAccountViewDetails,
  refreshingOfficialAccountId,
  savingOfficialAccountId,
  connectivityStatus,
}) => {
  const { t } = useTranslation();
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(true);

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
  const reasoningEffort = React.useMemo(() => {
    const configContent = settingsConfig.config || '';
    return extractCodexReasoningEffort(configContent);
  }, [settingsConfig.config]);
  const isOfficialProvider = provider.category === 'official';
  const hasOfficialAccounts = isOfficialProvider && officialAccounts.length > 0;
  const displayModelName = modelName === 'gpt-5.4' && reasoningEffort
    ? `${modelName} (${reasoningEffort})`
    : modelName;
  const requiresExplicitBaseUrl = !isOfficialProvider;
  const canRunConnectivityTest =
    !isOfficialProvider &&
    Boolean(apiKey?.trim()) &&
    Boolean(modelName?.trim()) &&
    (!requiresExplicitBaseUrl || Boolean(baseUrl?.trim()));
  const actionAreaWidth = isApplied ? 40 : 112;
  const shouldShowOfficialAccounts = officialAccounts.length > 0 || isOfficialProvider;

  const formatOfficialAccountLabel = (account: CodexOfficialAccount) => {
    if (account.id === '__local__') {
      return account.email || t('codex.provider.officialAccountLocal');
    }
    return account.email || account.name;
  };

  const renderOfficialAccounts = () => {
    if (!shouldShowOfficialAccounts) {
      return null;
    }

    return (
      <div
        style={{
          marginTop: 12,
          paddingTop: 12,
          borderTop: '1px solid var(--color-border)',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
            marginBottom: accountsCollapsed ? 0 : 10,
          }}
        >
          <Button
            type="text"
            size="small"
            onClick={() => setAccountsCollapsed((current) => !current)}
            style={{
              padding: 0,
              height: 'auto',
              color: 'var(--color-text-secondary)',
              fontSize: 12,
            }}
          >
            <Space size={6}>
              {accountsCollapsed ? <RightOutlined /> : <DownOutlined />}
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t('codex.provider.officialAccountsTitle')}
              </Text>
              <Text type="secondary" style={{ fontSize: 11 }}>
                ({officialAccounts.length})
              </Text>
            </Space>
          </Button>

          {isOfficialProvider ? (
            <Button
              type="link"
              size="small"
              icon={<LinkOutlined />}
              onClick={() => onOfficialAccountLogin?.(provider)}
              style={{ paddingInline: 0, height: 'auto', fontSize: 12 }}
            >
              {t('codex.provider.officialAccountLogin')}
            </Button>
          ) : (
            <Text type="secondary" style={{ fontSize: 11 }}>
              {t('codex.provider.officialAccountLegacyNotice')}
            </Text>
          )}
        </div>

        {!accountsCollapsed && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
              paddingLeft: 18,
            }}
          >
            {hasOfficialAccounts ? officialAccounts.map((account) => (
              <div
                key={account.id}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 12,
                  padding: '6px 0',
                  borderBottom: '1px solid var(--color-border)',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    flexWrap: 'wrap',
                    gap: 8,
                    minWidth: 0,
                    flex: 1,
                  }}
                >
                  <Text
                    strong={account.isApplied}
                    style={{ fontSize: 12 }}
                    ellipsis={{ tooltip: formatOfficialAccountLabel(account) }}
                  >
                    {formatOfficialAccountLabel(account)}
                  </Text>
                  <Tag style={{ margin: 0, fontSize: 10 }}>
                    {account.id === '__local__'
                      ? t('codex.provider.officialAccountLocalTag')
                      : t('codex.provider.officialAccountOauthTag')}
                  </Tag>
                  {account.planType && (
                    <Text type="secondary" style={{ fontSize: 11 }}>
                      {account.planType}
                    </Text>
                  )}
                  {account.lastError ? (
                    <Text type="danger" style={{ fontSize: 11 }}>
                      {t('codex.provider.officialAccountLastError', { message: account.lastError })}
                    </Text>
                  ) : (
                    <>
                      {account.limit5hText && (
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          {`${t('codex.provider.officialAccountShortWindowLimitLabel', {
                            label: account.limitShortLabel || '5h',
                          })}: ${account.limit5hText}`}
                        </Text>
                      )}
                      {account.limitWeeklyText && (
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          {`${t('codex.provider.officialAccountWeeklyLimitLabel')}: ${account.limitWeeklyText}`}
                        </Text>
                      )}
                    </>
                  )}
                  {account.isApplied && (
                    <Tag color="green" style={{ margin: 0, fontSize: 10 }}>
                      {t('codex.provider.applied')}
                    </Tag>
                  )}
                </div>

                <Space size={4} wrap>
                  <Button
                    type="text"
                    size="small"
                    icon={<SyncOutlined />}
                    onClick={() => onOfficialAccountRefresh?.(provider, account)}
                    loading={refreshingOfficialAccountId === account.id}
                    style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                  >
                    {t('codex.provider.officialAccountRefresh')}
                  </Button>
                  <Button
                    type="text"
                    size="small"
                    icon={<EyeOutlined />}
                    onClick={() => onOfficialAccountViewDetails?.(provider, account)}
                    style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                  >
                    {t('codex.provider.officialAccountViewDetails')}
                  </Button>
                  {account.id === '__local__' ? (
                    <Button
                      type="text"
                      size="small"
                      icon={<CheckOutlined />}
                      onClick={() => onOfficialLocalAccountSave?.(provider, account)}
                      loading={savingOfficialAccountId === account.id}
                      style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                    >
                      {t('common.save')}
                    </Button>
                  ) : !account.isApplied ? (
                    <Button
                      type="text"
                      size="small"
                      icon={<CheckOutlined />}
                      onClick={() => onOfficialAccountApply?.(provider, account)}
                      style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                    >
                      {t('codex.provider.apply')}
                    </Button>
                  ) : null}
                  {!account.isVirtual && (
                    <Button
                      type="text"
                      danger
                      size="small"
                      icon={<DeleteOutlined />}
                      onClick={() => onOfficialAccountDelete?.(provider, account)}
                      style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                    >
                      {t('common.delete')}
                    </Button>
                  )}
                </Space>
              </div>
            )) : (
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t('codex.provider.officialAccountsEmpty')}
              </Text>
            )}
          </div>
        )}
      </div>
    );
  };

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
    ...(provider.id !== '__local__'
      ? [
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
        ]
      : []),
  ].filter(Boolean) as MenuProps['items'];

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <Card
        size="small"
        style={{
          marginBottom: 12,
          borderColor: isApplied ? 'var(--ant-color-primary)' : 'var(--color-border-card)',
          backgroundColor: isApplied ? 'var(--color-bg-selected)' : undefined,
          boxShadow: 'var(--color-shadow)',
          transition: 'opacity 0.3s ease, border-color 0.2s ease, box-shadow 0.2s ease',
        }}
        styles={{ body: { padding: 16 } }}
        onMouseEnter={(e) => {
          e.currentTarget.style.boxShadow = 'var(--color-shadow-secondary)';
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.boxShadow = 'var(--color-shadow)';
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
                color: 'var(--color-text-tertiary)',
                padding: '4px 0',
                touchAction: 'none',
              }}
            >
              <HolderOutlined />
            </div>
            <Space direction="vertical" size={4} style={{ width: '100%' }}>
              {/* Provider name and status */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <ProviderConnectivityStatus item={connectivityStatus} />
                <Text strong style={{ fontSize: 14 }}>
                  {provider.name}
                </Text>
                {provider.id === '__local__' && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    ({t('codex.localConfigHint')})
                  </Text>
                )}
                {isOfficialProvider && (
                  <Tag>{t('codex.provider.modeOfficial')}</Tag>
                )}
                {isApplied && (
                  <Tag color="green" icon={<CheckCircleOutlined />}>
                    {t('codex.provider.applied')}
                  </Tag>
                )}
              </div>

              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                  {baseUrl && (
                    <Text code style={{ fontSize: 11, padding: '0 4px' }}>
                      {baseUrl}
                    </Text>
                  )}
                  {displayModelName && (
                    <Tag color="blue" style={{ fontSize: 11, margin: 0 }}>
                      {displayModelName}
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
                <Text type="secondary" style={{ fontSize: 11 }}>|</Text>
                <Button
                  type="text"
                  size="small"
                  icon={<ApiOutlined />}
                  onClick={() => onTest(provider)}
                  disabled={!canRunConnectivityTest}
                  title={isOfficialProvider ? t('codex.provider.officialConnectivityHint') : undefined}
                  style={{ fontSize: 11, padding: '0 4px', height: 'auto', flexShrink: 0 }}
                >
                  {t('opencode.connectivity.button')}
                </Button>
              </div>
            </Space>
          </div>

          {/* Action buttons */}
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
            {!isApplied && (
              <Button
                type="link"
                size="small"
                icon={<CheckOutlined />}
                onClick={() => onSelect(provider)}
                disabled={provider.isDisabled}
              >
                {t('codex.provider.apply')}
              </Button>
            )}
            <Dropdown menu={{ items: menuItems }} trigger={['click']}>
              <Button type="text" size="small" icon={<MoreOutlined />} />
            </Dropdown>
          </div>
      </div>
        {renderOfficialAccounts()}
    </Card>
    </div>
  );
};

export default CodexProviderCard;
