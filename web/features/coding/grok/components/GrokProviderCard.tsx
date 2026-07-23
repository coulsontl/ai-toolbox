import React from 'react';
import './GrokProviderCard.less';
import { Card, Space, Button, Dropdown, Tag, Typography, Switch, Tooltip, message, Collapse, Empty } from 'antd';
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
  PlusOutlined,
  CloudDownloadOutlined,
} from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { BarChart2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { GrokOfficialAccount, GrokProvider, GrokSettingsConfig } from '@/types/grok';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import {
  extractGrokSettingsApiBackend,
  extractGrokSettingsBaseUrl,
  extractGrokSettingsModel,
  extractGrokSettingsReasoningEffort,
} from '@/utils/grokConfigUtils';
import AppliedTag from '@/components/common/AppliedTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProxyTag from '@/components/common/ProxyTag';
import {
  canApplyProviderWithGatewayProxy,
  grokProviderNeedsGatewayProxy,
  grokWireApiFormatFromConfig,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  openAiApiFormatFromBaseUrl,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import type { ModelDisplayData, ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import ModelItem from '@/components/common/ModelItem';
import {
  GROK_LOCAL_PROVIDER_ID,
  isGrokLocalProviderId,
  shouldShowGrokOfficialAccounts,
} from '../utils/localProvider';
import {
  getGrokProviderCatalogModels,
  getGrokProviderDefaultModelKey,
} from '../utils/grokProviderModels';

const { Text } = Typography;

interface GrokProviderCardProps {
  provider: GrokProvider;
  isApplied: boolean;
  onEdit: (provider: GrokProvider) => void;
  onDelete: (provider: GrokProvider) => void;
  onCopy: (provider: GrokProvider) => void;
  onTest: (provider: GrokProvider) => void;
  onSelect: (provider: GrokProvider) => void;
  onToggleDisabled: (provider: GrokProvider, isDisabled: boolean) => void;
  onAddModel?: (provider: GrokProvider) => void;
  onEditModel?: (provider: GrokProvider, modelKey: string) => void;
  onDeleteModel?: (provider: GrokProvider, modelKey: string) => void;
  onSetDefaultModel?: (provider: GrokProvider, modelKey: string) => void;
  onFetchModels?: (provider: GrokProvider) => void;
  officialAccounts?: GrokOfficialAccount[];
  onOfficialAccountLogin?: (provider: GrokProvider) => void;
  onOfficialLocalAccountSave?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountApply?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountDelete?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountRefresh?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountViewDetails?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  refreshingOfficialAccountId?: string | null;
  savingOfficialAccountId?: string | null;
  connectivityStatus?: ProviderConnectivityStatusItem;
  gatewayTakeoverActive?: boolean;
  gatewayStatus?: GatewayCliTakeoverStatus | null;
  onGatewayStatusChange?: (status: GatewayCliTakeoverStatus) => void | Promise<void>;
}

const GrokProviderCard: React.FC<GrokProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
  onTest,
  onSelect,
  onToggleDisabled,
  onAddModel,
  onEditModel,
  onDeleteModel,
  onSetDefaultModel,
  onFetchModels,
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
  const settingsConfig: GrokSettingsConfig = React.useMemo(() => {
    try {
      return JSON.parse(provider.settingsConfig);
    } catch (error) {
      console.error('Failed to parse settingsConfig:', error);
      return {};
    }
  }, [provider.settingsConfig]);

  // Extract display info from config
  const apiKey = settingsConfig.auth?.API_KEY;
  const maskedApiKey = apiKey ? `${apiKey.slice(0, 8)}...${apiKey.slice(-4)}` : null;

  // Extract base_url and model from config.toml using utility function
  const baseUrl = React.useMemo(() => {
    return extractGrokSettingsBaseUrl(settingsConfig);
  }, [settingsConfig]);

  const modelName = React.useMemo(() => {
    return extractGrokSettingsModel(settingsConfig);
  }, [settingsConfig]);
  const reasoningEffort = React.useMemo(() => {
    return extractGrokSettingsReasoningEffort(settingsConfig);
  }, [settingsConfig]);
  const catalogModels = React.useMemo(
    () => getGrokProviderCatalogModels(provider),
    [provider],
  );
  const defaultModelKey = React.useMemo(
    () => getGrokProviderDefaultModelKey(provider),
    [provider],
  );
  const modelListItems = React.useMemo<ModelDisplayData[]>(() => {
    return catalogModels.map((catalogModel) => {
      const key = catalogModel.key?.trim() || catalogModel.model;
      const efforts = Array.isArray(catalogModel.reasoningEfforts)
        ? catalogModel.reasoningEfforts
        : [];
      const effortLabel = catalogModel.reasoningEffort
        || (efforts.length > 0 ? efforts.join('/') : undefined);
      const displayName = catalogModel.displayName || catalogModel.model || key;
      return {
        id: key,
        name: effortLabel ? `${displayName} (${effortLabel})` : displayName,
        contextLimit: typeof catalogModel.contextWindow === 'number'
          ? catalogModel.contextWindow
          : (catalogModel.contextWindow
            ? Number(catalogModel.contextWindow) || undefined
            : undefined),
        isPrimary: key === defaultModelKey,
      };
    });
  }, [catalogModels, defaultModelKey]);
  const isOfficialProvider = provider.category === 'official';
  const isLocalProvider = isGrokLocalProviderId(provider.id);
  const showModelList = !isOfficialProvider && !isLocalProvider;
  // `__local__` is a local-file bridge, not a managed applied preset.
  const showRuntimeApplied = isApplied && !isLocalProvider;
  const settingsConfigApiFormat = settingsConfig as GrokSettingsConfig & {
    apiFormat?: unknown;
    api_format?: unknown;
  };
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const providerProfileApiFormat = React.useMemo(
    () => getGatewayProviderApiFormatFromMeta(provider.meta, 'grok'),
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
    extractGrokSettingsApiBackend(settingsConfig),
    grokWireApiFormatFromConfig(settingsConfig.config),
    openAiApiFormatFromBaseUrl(baseUrl),
  );
  const apiFormatLabel = React.useMemo(() => {
    if (!providerApiFormat || isOfficialProvider) {
      return null;
    }
    if (providerApiFormat === 'openai_responses') {
      return t('grok.provider.apiFormatOpenAIResponses');
    }
    if (providerApiFormat === 'openai_chat') {
      return t('grok.provider.apiFormatOpenAIChat');
    }
    if (providerApiFormat === 'anthropic_messages') {
      return t('grok.provider.apiFormatAnthropicMessages');
    }
    if (providerApiFormat === 'gemini_native') {
      return 'Gemini Native';
    }
    return providerApiFormat;
  }, [isOfficialProvider, providerApiFormat, t]);
  // Grok CLI natively speaks responses / chat / anthropic; only Gemini needs gateway.
  const needsGatewayProxy =
    !isOfficialProvider &&
    !isLocalProvider &&
    grokProviderNeedsGatewayProxy(providerApiFormat);
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
  const shouldShowOfficialAccounts = shouldShowGrokOfficialAccounts(
    provider,
    officialAccounts.length,
  );

  const refreshTrayAfterGatewayChange = () => {
    void refreshTrayMenu().catch((error) => {
      console.error('Failed to refresh tray menu after gateway change:', error);
    });
  };

  const formatOfficialAccountLabel = (account: GrokOfficialAccount) => {
    if (account.id === GROK_LOCAL_PROVIDER_ID) {
      return account.email || t('grok.provider.officialAccountLocal');
    }
    return account.email || account.name;
  };

  const handleEngageGatewayProxy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setEngagingGatewayProxy(true);
    try {
      const nextStatus = await engageProxyGatewaySingle('grok', provider.id);
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('grok', provider.id);
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
      const nextStatus = await restoreProxyGatewayCliDirect('grok');
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('grok', provider.id);
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
                {t('grok.provider.officialAccountsTitle')}
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
              {t('grok.provider.officialAccountLogin')}
            </Button>
          ) : (
            <Text type="secondary" style={{ fontSize: 11 }}>
              {t('grok.provider.officialAccountLegacyNotice')}
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
                    {account.id === GROK_LOCAL_PROVIDER_ID
                      ? t('grok.provider.officialAccountLocalTag')
                      : t('grok.provider.officialAccountOauthTag')}
                  </Tag>
                  {account.lastError ? (
                    <Text type="danger" style={{ fontSize: 11 }}>
                      {t('grok.provider.officialAccountLastError', { message: account.lastError })}
                    </Text>
                  ) : null}
                  {showOfficialRuntimeState && account.isApplied && (
                    <AppliedTag style={{ fontSize: 10 }}>
                      {t('grok.provider.applied')}
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
                    {t('grok.provider.officialAccountRefresh')}
                  </Button>
                  <Button
                    type="text"
                    size="small"
                    icon={<EyeOutlined />}
                    onClick={() => onOfficialAccountViewDetails?.(provider, account)}
                    style={{ height: 'auto', paddingInline: 4, fontSize: 11 }}
                  >
                    {t('grok.provider.officialAccountViewDetails')}
                  </Button>
                  {account.id === GROK_LOCAL_PROVIDER_ID ? (
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
                      {t('grok.provider.apply')}
                    </Button>
                  ) : null}
                  {(
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
                {t('grok.provider.officialAccountsEmpty')}
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
              {provider.isDisabled ? t('grok.configDisabled') : t('grok.configEnabled')}
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
                    ({t('grok.localConfigHint')})
                  </Text>
                )}
                {isOfficialProvider && (
                  <Tag>{t('grok.provider.modeOfficial')}</Tag>
                )}
                {isOfficialProvider && gatewayTakeoverActive && (
                  <Tooltip title={t('gateway.takeover.officialBypassedTooltip')}>
                    <Tag color="gold">{t('gateway.takeover.officialBypassedTag')}</Tag>
                  </Tooltip>
                )}
                {showRuntimeApplied && (
                  <AppliedTag>
                    {t('grok.provider.applied')}
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
                  {apiFormatLabel && (
                    <Tag color="purple" style={{ fontSize: 11, margin: 0 }}>
                      {apiFormatLabel}
                    </Tag>
                  )}
                  {(baseUrl || modelName || apiFormatLabel) && maskedApiKey && (
                    <Text type="secondary" style={{ fontSize: 12 }}>|</Text>
                  )}
                  {maskedApiKey && (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      API Key: {maskedApiKey}
                    </Text>
                  )}
                  {(baseUrl || modelName || apiFormatLabel || maskedApiKey) && provider.notes && (
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
                  title={isOfficialProvider ? t('grok.provider.officialConnectivityHint') : undefined}
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
                {t('grok.provider.apply')}
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
                    {t('grok.provider.apply')}
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
        {showModelList && (
          <Collapse
            defaultActiveKey={[]}
            ghost
            className="grok-model-list-collapse"
            style={{ marginTop: 12, background: 'transparent' }}
            items={[{
              key: `models-${provider.id}`,
              label: (
                <div
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                    width: '100%',
                    background: 'transparent',
                  }}
                >
                  <Text strong style={{ fontSize: 13 }}>
                    {t('grok.model.title')} ({modelListItems.length})
                  </Text>
                  <Space size={4} onClick={(event) => event.stopPropagation()}>
                    {onFetchModels && (
                      <Button
                        size="small"
                        type="text"
                        style={{ fontSize: 12 }}
                        onClick={() => onFetchModels(provider)}
                      >
                        <CloudDownloadOutlined style={{ marginRight: 4 }} />
                        {t('grok.fetchModels.button')}
                      </Button>
                    )}
                    {onAddModel && (
                      <Button
                        size="small"
                        type="text"
                        style={{ fontSize: 12 }}
                        onClick={() => onAddModel(provider)}
                      >
                        <PlusOutlined style={{ marginRight: 0 }} />
                        {t('grok.model.addModel')}
                      </Button>
                    )}
                  </Space>
                </div>
              ),
              children: (
                <div style={{ background: 'transparent' }}>
                  {modelListItems.length > 0 ? (
                    <Space direction="vertical" style={{ width: '100%' }} size={4}>
                      {modelListItems.map((model) => (
                        <ModelItem
                          key={model.id}
                          model={model}
                          i18nPrefix="grok"
                          transparentBackground
                          onEdit={onEditModel ? () => onEditModel(provider, model.id) : undefined}
                          onDelete={onDeleteModel ? () => onDeleteModel(provider, model.id) : undefined}
                          onSetPrimary={onSetDefaultModel
                            ? () => onSetDefaultModel(provider, model.id)
                            : undefined}
                        />
                      ))}
                    </Space>
                  ) : (
                    <Empty
                      image={Empty.PRESENTED_IMAGE_SIMPLE}
                      description={t('grok.model.emptyText')}
                      style={{ margin: '8px 0', background: 'transparent' }}
                    />
                  )}
                </div>
              ),
            }]}
          />
        )}
    </Card>
    </div>
  );
};

export default GrokProviderCard;
