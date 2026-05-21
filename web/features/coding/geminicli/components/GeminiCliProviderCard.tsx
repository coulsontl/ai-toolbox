import React from 'react';
import { Card, Space, Button, Dropdown, Tag, Typography, Switch, Tooltip, message } from 'antd';
import type { MenuProps } from 'antd';
import {
  CheckCircleOutlined,
  CheckOutlined,
  CopyOutlined,
  DeleteOutlined,
  DownOutlined,
  EditOutlined,
  EyeOutlined,
  HolderOutlined,
  LinkOutlined,
  MoreOutlined,
  RightOutlined,
  SyncOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { GeminiCliOfficialAccount, GeminiCliProvider, GeminiCliSettingsConfig } from '@/types/geminicli';
import { GEMINI_CLI_LOCAL_PROVIDER_ID, shouldShowGeminiCliOfficialAccounts } from '../utils/localProvider';

const { Text } = Typography;

interface GeminiCliProviderCardProps {
  provider: GeminiCliProvider;
  isApplied: boolean;
  onEdit: (provider: GeminiCliProvider) => void;
  onDelete: (provider: GeminiCliProvider) => void;
  onCopy: (provider: GeminiCliProvider) => void;
  onSelect: (provider: GeminiCliProvider) => void;
  onToggleDisabled: (provider: GeminiCliProvider, isDisabled: boolean) => void;
  officialAccounts?: GeminiCliOfficialAccount[];
  onOfficialAccountLogin?: (provider: GeminiCliProvider) => void;
  onOfficialLocalAccountSave?: (provider: GeminiCliProvider, account: GeminiCliOfficialAccount) => void;
  onOfficialAccountApply?: (provider: GeminiCliProvider, account: GeminiCliOfficialAccount) => void;
  onOfficialAccountDelete?: (provider: GeminiCliProvider, account: GeminiCliOfficialAccount) => void;
  onOfficialAccountRefresh?: (provider: GeminiCliProvider, account: GeminiCliOfficialAccount) => void;
  onOfficialAccountViewDetails?: (provider: GeminiCliProvider, account: GeminiCliOfficialAccount) => void;
  refreshingOfficialAccountId?: string | null;
  savingOfficialAccountId?: string | null;
  gatewayTakeoverActive?: boolean;
}

const parseSettingsConfig = (rawConfig: string): GeminiCliSettingsConfig => {
  try {
    return JSON.parse(rawConfig) as GeminiCliSettingsConfig;
  } catch (error) {
    console.error('Failed to parse Gemini CLI settingsConfig:', error);
    return {};
  }
};

const maskSecret = (value?: string) => {
  if (!value) return null;
  if (value.length <= 12) return `${value.slice(0, 4)}...`;
  return `${value.slice(0, 8)}...${value.slice(-4)}`;
};

const extractModelName = (settingsConfig: GeminiCliSettingsConfig) => {
  const envModel = settingsConfig.env?.GEMINI_MODEL?.trim();
  if (envModel) {
    return envModel;
  }

  const settingsModel = settingsConfig.config?.model;
  if (typeof settingsModel === 'string') {
    return settingsModel.trim() || undefined;
  }
  if (!settingsModel || typeof settingsModel !== 'object') {
    return undefined;
  }

  const modelName = (settingsModel as { name?: unknown }).name;
  return typeof modelName === 'string' ? modelName.trim() || undefined : undefined;
};

const GeminiCliProviderCard: React.FC<GeminiCliProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
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
  gatewayTakeoverActive = false,
}) => {
  const { t } = useTranslation();
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(true);
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: provider.id });

  const settingsConfig = React.useMemo(
    () => parseSettingsConfig(provider.settingsConfig),
    [provider.settingsConfig],
  );

  const env = settingsConfig.env || {};
  const baseUrl = env.GOOGLE_GEMINI_BASE_URL?.trim();
  const modelName = extractModelName(settingsConfig);
  const maskedApiKey = maskSecret(env.GEMINI_API_KEY || env.GOOGLE_API_KEY);
  const isOfficialProvider = provider.category === 'official';
  const hasOfficialAccounts = isOfficialProvider && officialAccounts.length > 0;
  const shouldShowOfficialAccounts = shouldShowGeminiCliOfficialAccounts(
    provider,
    officialAccounts.length,
  );
  const showRuntimeApplied = isApplied && !gatewayTakeoverActive;
  const showOfficialRuntimeState = !gatewayTakeoverActive;

  const handleToggleDisabled = (checked: boolean) => {
    if (isApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !checked);
  };

  const menuItems: MenuProps['items'] = [
    {
      key: 'toggle',
      label: (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span>{t('common.enable')}</span>
            <Text type="secondary" style={{ fontSize: 11 }}>
              {provider.isDisabled ? t('geminicli.configDisabled') : t('geminicli.configEnabled')}
            </Text>
          </div>
          <Switch checked={!provider.isDisabled} onChange={handleToggleDisabled} size="small" />
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
    ...(provider.id !== GEMINI_CLI_LOCAL_PROVIDER_ID
      ? [
          { type: 'divider' as const },
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

  const formatOfficialAccountLabel = (account: GeminiCliOfficialAccount) => {
    if (account.id === GEMINI_CLI_LOCAL_PROVIDER_ID) {
      return account.email || t('geminicli.provider.officialAccountLocal');
    }
    return account.email || account.projectId || account.name;
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
                {t('geminicli.provider.officialAccountsTitle')}
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
              {t('geminicli.provider.officialAccountLogin')}
            </Button>
          ) : (
            <Text type="secondary" style={{ fontSize: 11 }}>
              {t('geminicli.provider.officialAccountLegacyNotice')}
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
                    strong={showOfficialRuntimeState && account.isApplied}
                    style={{ fontSize: 12 }}
                    ellipsis={{ tooltip: formatOfficialAccountLabel(account) }}
                  >
                    {formatOfficialAccountLabel(account)}
                  </Text>
                  <Tag style={{ margin: 0, fontSize: 10 }}>
                    {account.id === GEMINI_CLI_LOCAL_PROVIDER_ID
                      ? t('geminicli.provider.officialAccountLocalTag')
                      : t('geminicli.provider.officialAccountOauthTag')}
                  </Tag>
                  {account.projectId && (
                    <Text type="secondary" style={{ fontSize: 11 }}>
                      {account.projectId}
                    </Text>
                  )}
                  {account.planType && (
                    <Text type="secondary" style={{ fontSize: 11 }}>
                      {account.planType}
                    </Text>
                  )}
                  {account.lastError ? (
                    <Text type="danger" style={{ fontSize: 11 }}>
                      {t('geminicli.provider.officialAccountLastError', { message: account.lastError })}
                    </Text>
                  ) : (
                    <>
                      {account.limit5hText && (
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          {`${t('geminicli.provider.officialAccountShortWindowLimitLabel', {
                            label: account.limitShortLabel || '5h',
                          })}: ${account.limit5hText}`}
                        </Text>
                      )}
                      {account.limitWeeklyText && (
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          {`${t('geminicli.provider.officialAccountWeeklyLimitLabel')}: ${account.limitWeeklyText}`}
                        </Text>
                      )}
                    </>
                  )}
                  {showOfficialRuntimeState && account.isApplied && (
                    <Tag color="green" style={{ margin: 0, fontSize: 10 }}>
                      {t('geminicli.provider.applied')}
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
                    {t('geminicli.provider.officialAccountRefresh')}
                  </Button>
                  <Button
                    type="text"
                    size="small"
                    icon={<EyeOutlined />}
                    onClick={() => onOfficialAccountViewDetails?.(provider, account)}
                    style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                  >
                    {t('geminicli.provider.officialAccountViewDetails')}
                  </Button>
                  {account.id === GEMINI_CLI_LOCAL_PROVIDER_ID ? (
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
                  ) : showOfficialRuntimeState && !account.isApplied ? (
                    <Button
                      type="text"
                      size="small"
                      icon={<CheckOutlined />}
                      onClick={() => onOfficialAccountApply?.(provider, account)}
                      style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                    >
                      {t('geminicli.provider.apply')}
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
                {t('geminicli.provider.officialAccountsEmpty')}
              </Text>
            )}
          </div>
        )}
      </div>
    );
  };

  return (
    <div
      ref={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
        opacity: isDragging ? 0.5 : provider.isDisabled ? 0.6 : 1,
      }}
    >
      <Card
        size="small"
        style={{
          marginBottom: 12,
          borderColor: showRuntimeApplied ? 'var(--ant-color-primary)' : 'var(--color-border-card)',
          backgroundColor: showRuntimeApplied ? 'var(--color-bg-selected)' : undefined,
          boxShadow: 'var(--color-shadow)',
          transition: 'opacity 0.3s ease, border-color 0.2s ease, box-shadow 0.2s ease',
        }}
        styles={{ body: { padding: 16 } }}
        onMouseEnter={(event) => {
          event.currentTarget.style.boxShadow = 'var(--color-shadow-secondary)';
        }}
        onMouseLeave={(event) => {
          event.currentTarget.style.boxShadow = 'var(--color-shadow)';
        }}
      >
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
          <div style={{ flex: 1, display: 'flex', alignItems: 'flex-start', gap: 8, minWidth: 0 }}>
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
            <Space direction="vertical" size={4} style={{ width: '100%', minWidth: 0 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <Text strong style={{ fontSize: 14 }}>
                  {provider.name}
                </Text>
                {provider.id === GEMINI_CLI_LOCAL_PROVIDER_ID && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    ({t('geminicli.localConfigHint')})
                  </Text>
                )}
                {isOfficialProvider && <Tag>{t('geminicli.provider.modeOfficial')}</Tag>}
                {isOfficialProvider && gatewayTakeoverActive && (
                  <Tooltip title={t('gateway.takeover.officialBypassedTooltip')}>
                    <Tag color="gold">{t('gateway.takeover.officialBypassedTag')}</Tag>
                  </Tooltip>
                )}
                {showRuntimeApplied && (
                  <Tag color="green" icon={<CheckCircleOutlined />}>
                    {t('geminicli.provider.applied')}
                  </Tag>
                )}
              </div>

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
                {maskedApiKey && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    API Key: {maskedApiKey}
                  </Text>
                )}
                {provider.notes && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {provider.notes}
                  </Text>
                )}
              </div>
            </Space>
          </div>

          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 8 }}>
            {!gatewayTakeoverActive && !isApplied && (
              <Button
                type="link"
                size="small"
                icon={<CheckOutlined />}
                onClick={() => onSelect(provider)}
                disabled={provider.isDisabled}
              >
                {t('geminicli.provider.apply')}
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

export default GeminiCliProviderCard;
