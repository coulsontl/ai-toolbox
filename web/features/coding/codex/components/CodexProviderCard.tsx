import React from 'react';
import { Card, Space, Button, Dropdown, Tag, Typography, Switch, Tooltip, message } from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
  EditOutlined,
  DeleteOutlined,
  CopyOutlined,
  MoreOutlined,
  HolderOutlined,
  DownOutlined,
  RightOutlined,
  LinkOutlined,
  SyncOutlined,
  EyeOutlined,
} from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { BarChart2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { CodexOfficialAccount, CodexProvider, CodexSettingsConfig } from '@/types/codex';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { extractCodexBaseUrl, extractCodexModel, extractCodexReasoningEffort } from '@/utils/codexConfigUtils';
import AppliedTag from '@/components/common/AppliedTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProxyTag from '@/components/common/ProxyTag';
import {
  canApplyProviderWithGatewayProxy,
  codexWireApiFormatFromConfig,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  openAiApiFormatFromBaseUrl,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import {
  CODEX_LOCAL_PROVIDER_ID,
  isCodexLocalProviderId,
  shouldShowCodexOfficialAccounts,
} from '../utils/localProvider';

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
  gatewayTakeoverActive?: boolean;
  gatewayStatus?: GatewayCliTakeoverStatus | null;
  onGatewayStatusChange?: (status: GatewayCliTakeoverStatus) => void | Promise<void>;
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
  gatewayTakeoverActive = false,
  gatewayStatus = null,
  onGatewayStatusChange,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(true);
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);
  const [switchingGatewayProvider, setSwitchingGatewayProvider] = React.useState(false);

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
  const isLocalProvider = isCodexLocalProviderId(provider.id);
  // `__local__` is a local-file bridge, not a managed applied preset.
  const showRuntimeApplied = isApplied && !isLocalProvider;
  const settingsConfigApiFormat = settingsConfig as CodexSettingsConfig & {
    apiFormat?: unknown;
    api_format?: unknown;
  };
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const providerProfileApiFormat = React.useMemo(
    () => getGatewayProviderApiFormatFromMeta(provider.meta, 'codex'),
    [gatewayProviderProfilesVersion, provider.meta],
  );
  const providerApiFormat = firstGatewayApiFormat(
    providerProfileApiFormat,
    provider.meta?.apiFormat,
    typeof settingsConfigApiFormat.apiFormat === 'string'
      ? settingsConfigApiFormat.apiFormat
      : undefined,
    typeof settingsConfigApiFormat.api_format === 'string'
      ? settingsConfigApiFormat.api_format
      : undefined,
    codexWireApiFormatFromConfig(settingsConfig.config),
    openAiApiFormatFromBaseUrl(baseUrl),
  );
  const needsGatewayProxy =
    !isOfficialProvider &&
    !isLocalProvider &&
    providerNeedsGatewayProxy(providerApiFormat, 'openai_responses');
  const gatewayCanApplyProxy = canApplyProviderWithGatewayProxy(gatewayStatus);
  const gatewayMode = gatewayStatus?.mode ?? null;
  const gatewayFailoverActive = gatewayMode === 'failover';
  const gatewayProxyActive = gatewayMode === 'single' || gatewayFailoverActive;
  const priorityEntry = gatewayFailoverActive
    ? gatewayStatus?.provider_priorities.find((entry) => entry.provider_id === provider.id)
    : undefined;
  const isGatewayPrimary = priorityEntry?.label === 'P0';
  const hasOfficialAccounts = isOfficialProvider && officialAccounts.length > 0;
  const displayModelName = modelName && reasoningEffort
    ? `${modelName} (${reasoningEffort})`
    : modelName;
  const requiresExplicitBaseUrl = !isOfficialProvider;
  const canRunConnectivityTest =
    !isOfficialProvider &&
    Boolean(apiKey?.trim()) &&
    Boolean(modelName?.trim()) &&
    (!requiresExplicitBaseUrl || Boolean(baseUrl?.trim()));
  const showProxyTag = showRuntimeApplied && gatewayProxyActive;
  const showOfficialRuntimeState = !gatewayProxyActive && !gatewayTakeoverActive;
  const canShowGatewayProxyButton =
    showRuntimeApplied &&
    !gatewayMode &&
    Boolean(gatewayStatus?.can_takeover) &&
    !provider.isDisabled &&
    !isOfficialProvider &&
    !isLocalProvider;
  const canRestoreDirect = showRuntimeApplied && gatewayProxyActive && Boolean(gatewayStatus?.can_restore_direct);
  const canShowRestoreDirectButton = canRestoreDirect && !needsGatewayProxy;
  const canShowRestoreDirectUnavailable = canRestoreDirect && needsGatewayProxy;
  const canSwitchGatewayProvider =
    gatewayProxyActive &&
    !isApplied &&
    !provider.isDisabled &&
    !isOfficialProvider &&
    !isLocalProvider;
  const showApplyAction = !gatewayProxyActive && !isApplied && !isLocalProvider;
  const showApplyWithProxyAction = showApplyAction && needsGatewayProxy;
  const showDirectApplyAction = showApplyAction && !needsGatewayProxy;
  const showGatewaySwitchAction = canSwitchGatewayProvider;
  const showGatewayLockedApply = gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;
  const applyWithProxyDisabled = provider.isDisabled || !gatewayCanApplyProxy;
  // Match Claude: size the action rail from actual action flags, not applied chrome.
  const actionAreaWidth =
    showApplyWithProxyAction
      ? 160
      : showApplyAction || showGatewaySwitchAction || showGatewayLockedApply || canShowGatewayProxyButton || canShowRestoreDirectButton || canShowRestoreDirectUnavailable
        ? 140
        : 40;

  const handleToggleDisabled = (checked: boolean) => {
    if (showRuntimeApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !checked);
  };
  const cardBorderColor = isGatewayPrimary
    ? 'var(--color-status-success)'
    : showRuntimeApplied
      ? 'var(--ant-color-primary)'
      : 'var(--color-border-card)';
  const cardBackground = isGatewayPrimary
    ? 'linear-gradient(135deg, color-mix(in srgb, var(--color-status-success) 12%, var(--color-bg-container)), var(--color-bg-container))'
    : showRuntimeApplied
      ? 'var(--color-bg-selected)'
      : undefined;
  const shouldShowOfficialAccounts = shouldShowCodexOfficialAccounts(
    provider,
    officialAccounts.length,
  );

  const refreshTrayAfterGatewayChange = () => {
    void refreshTrayMenu().catch((error) => {
      console.error('Failed to refresh tray menu after gateway change:', error);
    });
  };

  const formatOfficialAccountLabel = (account: CodexOfficialAccount) => {
    if (account.id === CODEX_LOCAL_PROVIDER_ID) {
      return account.email || t('codex.provider.officialAccountLocal');
    }
    return account.email || account.name;
  };

  const handleEngageGatewayProxy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setEngagingGatewayProxy(true);
    try {
      const nextStatus = await engageProxyGatewaySingle('codex', provider.id);
      onGatewayStatusChange?.(nextStatus);
      refreshTrayAfterGatewayChange();
      message.success(t('gateway.proxy.notice.enabled'));
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(t('gateway.proxy.notice.enableFailed', { error: errorMessage }));
    } finally {
      setEngagingGatewayProxy(false);
    }
  };

  const handleApplyWithGatewayProxy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setEngagingGatewayProxy(true);
    try {
      const nextStatus = await switchProxyGatewayPrimaryProvider('codex', provider.id);
      await onGatewayStatusChange?.(nextStatus);
      refreshTrayAfterGatewayChange();
      message.success(t('gateway.proxy.notice.enabled'));
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(t('gateway.proxy.notice.enableFailed', { error: errorMessage }));
    } finally {
      setEngagingGatewayProxy(false);
    }
  };

  const handleRestoreDirect = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setRestoringDirect(true);
    try {
      const nextStatus = await restoreProxyGatewayCliDirect('codex');
      onGatewayStatusChange?.(nextStatus);
      refreshTrayAfterGatewayChange();
      message.success(t('gateway.proxy.notice.restored'));
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(t('gateway.proxy.notice.restoreFailed', { error: errorMessage }));
    } finally {
      setRestoringDirect(false);
    }
  };

  const handleSwitchGatewayProvider = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setSwitchingGatewayProvider(true);
    try {
      const nextStatus = await switchProxyGatewayPrimaryProvider('codex', provider.id);
      await onGatewayStatusChange?.(nextStatus);
      refreshTrayAfterGatewayChange();
      message.success(t('gateway.proxy.notice.switched'));
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(t('gateway.proxy.notice.switchFailed', { error: errorMessage }));
    } finally {
      setSwitchingGatewayProvider(false);
    }
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
                    strong={showOfficialRuntimeState && account.isApplied}
                    style={{ fontSize: 12 }}
                    ellipsis={{ tooltip: formatOfficialAccountLabel(account) }}
                  >
                    {formatOfficialAccountLabel(account)}
                  </Text>
                  <Tag style={{ margin: 0, fontSize: 10 }}>
                    {account.id === CODEX_LOCAL_PROVIDER_ID
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
                      {account.limitMonthlyText && (
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          {`${t('codex.provider.officialAccountMonthlyLimitLabel')}: ${account.limitMonthlyText}`}
                        </Text>
                      )}
                    </>
                  )}
                  {showOfficialRuntimeState && account.isApplied && (
                    <AppliedTag style={{ fontSize: 10 }}>
                      {t('codex.provider.applied')}
                    </AppliedTag>
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
                  {account.id === CODEX_LOCAL_PROVIDER_ID ? (
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
    ...(!isLocalProvider ? [{
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
    }] : []),
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
    ...(!isLocalProvider
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
          borderColor: cardBorderColor,
          background: cardBackground,
          boxShadow: 'var(--shadow-card-sm)',
          transition: 'opacity 0.3s ease, border-color 0.2s ease, box-shadow 0.2s ease',
        }}
        styles={{ body: { padding: 16 } }}
        onMouseEnter={(e) => {
          e.currentTarget.style.boxShadow = 'var(--shadow-card-sm-hover)';
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.boxShadow = 'var(--shadow-card-sm)';
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
            <div style={{ width: '100%', display: 'flex', flexDirection: 'column', gap: 4 }}>
              {/* Provider name and status */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <ProviderConnectivityStatus item={connectivityStatus} />
                <ProviderNameLink
                  name={provider.name}
                  baseUrl={baseUrl}
                  style={{ fontSize: 14, fontWeight: 600 }}
                />
                {isLocalProvider && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    ({t('codex.localConfigHint')})
                  </Text>
                )}
                {isOfficialProvider && (
                  <Tag>{t('codex.provider.modeOfficial')}</Tag>
                )}
                {isOfficialProvider && gatewayTakeoverActive && (
                  <Tooltip title={t('gateway.takeover.officialBypassedTooltip')}>
                    <Tag color="gold">{t('gateway.takeover.officialBypassedTag')}</Tag>
                  </Tooltip>
                )}
                {showRuntimeApplied && (
                  <AppliedTag>
                    {t('codex.provider.applied')}
                  </AppliedTag>
                )}
                {showProxyTag && (
                  <ProxyTag>
                    {t('gateway.proxy.proxyTag')}
                  </ProxyTag>
                )}
                {showProxyTag && (
                  <Tooltip title={t('gateway.proxy.statisticsTooltip')}>
                    <BarChart2
                      size={14}
                      aria-label={t('gateway.proxy.statisticsTooltip')}
                      onClick={(event) => {
                        event.stopPropagation();
                        navigate('/gateway/statistics');
                      }}
                      style={{
                        color: 'var(--color-text-tertiary)',
                        cursor: 'pointer',
                        flexShrink: 0,
                      }}
                    />
                  </Tooltip>
                )}
                {priorityEntry && (
                  <>
                    <span
                      style={{
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 4,
                        padding: '0 6px',
                        height: 20,
                        borderRadius: 10,
                        fontSize: 10,
                        fontWeight: 500,
                        background: 'rgba(16,185,129,0.08)',
                        color: '#059669',
                      }}
                    >
                      <span
                        style={{
                          width: 6,
                          height: 6,
                          borderRadius: '50%',
                          background: '#10b981',
                        }}
                      />
                      {t('gateway.page.modelHealthState.healthy')}
                    </span>
                    <Tooltip
                      title={
                        isGatewayPrimary
                          ? t('gateway.failover.priorityP0')
                          : t('gateway.failover.priorityPn', { label: priorityEntry.label })
                      }
                    >
                      <span
                        style={{
                          display: 'inline-flex',
                          alignItems: 'center',
                          padding: '0 6px',
                          height: 20,
                          borderRadius: 4,
                          fontSize: 10,
                          fontWeight: 650,
                          background: 'rgba(16,185,129,0.08)',
                          color: '#059669',
                        }}
                      >
                        {priorityEntry.label}
                      </span>
                    </Tooltip>
                  </>
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
            </div>
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
            {canShowGatewayProxyButton && (
              <Tooltip title={t('gateway.proxy.singleHint')}>
                <Button
                  type="link"
                  size="small"
                  icon={<ApiOutlined />}
                  onClick={handleEngageGatewayProxy}
                  loading={engagingGatewayProxy}
                >
                  {t('gateway.proxy.singleButton')}
                </Button>
              </Tooltip>
            )}
            {canShowRestoreDirectButton && (
              <Tooltip title={t('gateway.proxy.restoreDirectHint')}>
                <Button
                  type="link"
                  size="small"
                  onClick={handleRestoreDirect}
                  loading={restoringDirect}
                >
                  {t('gateway.proxy.restoreDirectButton')}
                </Button>
              </Tooltip>
            )}
            {canShowRestoreDirectUnavailable && (
              <Tooltip title={t('gateway.proxy.restoreDirectUnavailableHint')}>
                <Button
                  type="link"
                  size="small"
                  disabled
                >
                  {t('gateway.proxy.restoreDirectButton')}
                </Button>
              </Tooltip>
            )}
            {showDirectApplyAction && (
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
            {showApplyWithProxyAction && (
              <Tooltip
                title={
                  gatewayCanApplyProxy
                    ? t('gateway.proxy.applyWithProxyHint')
                    : t('gateway.proxy.applyWithProxyDisabledTooltip')
                }
              >
                <span>
                  <Button
                    type="link"
                    size="small"
                    icon={<CheckOutlined />}
                    onClick={handleApplyWithGatewayProxy}
                    disabled={applyWithProxyDisabled}
                    loading={engagingGatewayProxy}
                  >
                    {t('gateway.proxy.applyWithProxyButton')}
                  </Button>
                </span>
              </Tooltip>
            )}
            {showGatewaySwitchAction && (
              <Tooltip
                title={
                  gatewayFailoverActive
                    ? t('gateway.proxy.switchPrimaryFailoverHint')
                    : t('gateway.proxy.switchPrimaryHint')
                }
              >
                <Button
                  type="link"
                  size="small"
                  icon={<CheckOutlined />}
                  onClick={handleSwitchGatewayProvider}
                  loading={switchingGatewayProvider}
                >
                  {gatewayFailoverActive
                    ? t('gateway.proxy.switchPrimaryP0Button')
                    : t('gateway.proxy.switchPrimaryButton')}
                </Button>
              </Tooltip>
            )}
            {showGatewayLockedApply && (
              <Tooltip title={t('gateway.proxy.applyLockedTooltip')}>
                <span>
                  <Button type="link" size="small" icon={<CheckOutlined />} disabled>
                    {t('codex.provider.apply')}
                  </Button>
                </span>
              </Tooltip>
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
