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
} from '@ant-design/icons';
import type { MenuProps } from 'antd';
import { BarChart2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { ClaudeCodeProvider } from '@/types/claudecode';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import AppliedTag from '@/components/common/AppliedTag';
import ProxyTag from '@/components/common/ProxyTag';
import {
  canApplyProviderWithGatewayProxy,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import {
  getClaudeConfiguredModelIds,
  getClaudeProviderModelConfig,
  parseClaudeSettingsConfig,
} from '../utils/claudeModelConfig';

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
  gatewayStatus?: GatewayCliTakeoverStatus | null;
  onGatewayStatusChange?: (status: GatewayCliTakeoverStatus) => void | Promise<void>;
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
  gatewayStatus = null,
  onGatewayStatusChange,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
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

  const handleToggleDisabled = (checked: boolean) => {
    if (isApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !checked);  // Switch 的 checked 表示"启用"，所以取反
  };

  // 解析 settingsConfig JSON 字符串
  const settingsConfig = React.useMemo(
    () => parseClaudeSettingsConfig(provider.settingsConfig),
    [provider.settingsConfig],
  );
  const modelConfig = React.useMemo(
    () => getClaudeProviderModelConfig(settingsConfig),
    [settingsConfig],
  );

  const configuredModelIds = React.useMemo(
    () => getClaudeConfiguredModelIds(settingsConfig),
    [settingsConfig],
  );
  const configuredApiKey =
    settingsConfig.env?.ANTHROPIC_AUTH_TOKEN?.trim() ||
    settingsConfig.env?.ANTHROPIC_API_KEY?.trim() ||
    '';
  const configuredBaseUrl = settingsConfig.env?.ANTHROPIC_BASE_URL?.trim() || '';
  const isOfficialProvider = provider.category === 'official';
  const settingsConfigApiFormat = settingsConfig as {
    apiFormat?: unknown;
    api_format?: unknown;
  };
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const providerProfileApiFormat = React.useMemo(
    () => getGatewayProviderApiFormatFromMeta(provider.meta, 'claude'),
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
  );
  const needsGatewayProxy =
    !isOfficialProvider &&
    provider.id !== '__local__' &&
    providerNeedsGatewayProxy(providerApiFormat, 'anthropic');
  const gatewayCanApplyProxy = canApplyProviderWithGatewayProxy(gatewayStatus);
  const gatewayMode = gatewayStatus?.mode ?? null;
  const gatewayFailoverActive = gatewayMode === 'failover';
  const gatewayProxyActive = gatewayMode === 'single' || gatewayFailoverActive;
  const priorityEntry = gatewayFailoverActive
    ? gatewayStatus?.provider_priorities.find((entry) => entry.provider_id === provider.id)
    : undefined;
  const isGatewayPrimary = priorityEntry?.label === 'P0';
  const canShowGatewayProxyButton =
    isApplied &&
    !gatewayMode &&
    Boolean(gatewayStatus?.can_takeover) &&
    !provider.isDisabled &&
    !isOfficialProvider &&
    provider.id !== '__local__';
  const canRestoreDirect = isApplied && gatewayProxyActive && Boolean(gatewayStatus?.can_restore_direct);
  const canShowRestoreDirectButton = canRestoreDirect && !needsGatewayProxy;
  const canShowRestoreDirectUnavailable = canRestoreDirect && needsGatewayProxy;
  const canSwitchGatewayProvider =
    gatewayProxyActive &&
    !isApplied &&
    !provider.isDisabled &&
    !isOfficialProvider &&
    provider.id !== '__local__';
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
    modelConfig.roles.haiku.model ||
    modelConfig.roles.sonnet.model ||
    modelConfig.roles.opus.model ||
    modelConfig.roles.fable.model ||
    modelConfig.legacyReasoningModel;
  const hasConfiguredModels = Boolean(modelConfig.fallbackModel || hasModels);
  const showRuntimeApplied = isApplied;
  const showProxyTag = isApplied && gatewayProxyActive;
  const showApplyAction = !gatewayProxyActive && !isApplied;
  const showApplyWithProxyAction = showApplyAction && needsGatewayProxy;
  const showDirectApplyAction = showApplyAction && !needsGatewayProxy;
  const showGatewaySwitchAction = canSwitchGatewayProvider;
  const showGatewayLockedApply = gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;
  const applyWithProxyDisabled = provider.isDisabled || !gatewayCanApplyProxy;
  const actionAreaWidth =
    showApplyWithProxyAction
      ? 160
      : showApplyAction || showGatewaySwitchAction || showGatewayLockedApply || canShowGatewayProxyButton || canShowRestoreDirectButton || canShowRestoreDirectUnavailable
        ? 140
      : 40;
  const cardBorderColor = isGatewayPrimary
    ? 'var(--color-status-success)'
    : isApplied
      ? 'var(--ant-color-primary)'
      : 'var(--color-border-card)';
  const cardBackground = isGatewayPrimary
    ? 'linear-gradient(135deg, color-mix(in srgb, var(--color-status-success) 12%, var(--color-bg-container)), var(--color-bg-container))'
    : isApplied
      ? 'var(--color-bg-selected)'
      : undefined;

  const refreshTrayAfterGatewayChange = () => {
    void refreshTrayMenu().catch((error) => {
      console.error('Failed to refresh tray menu after gateway change:', error);
    });
  };

  const handleEngageGatewayProxy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setEngagingGatewayProxy(true);
    try {
      const nextStatus = await engageProxyGatewaySingle('claude', provider.id);
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('claude', provider.id);
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
      const nextStatus = await restoreProxyGatewayCliDirect('claude');
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('claude', provider.id);
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
                <AppliedTag>
                  {t('claudecode.provider.applied')}
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

            <div style={{ display: 'flex', alignItems: 'flex-start', gap: '8px 16px', flexWrap: 'wrap', marginTop: 4 }}>
                {modelConfig.fallbackModel && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('claudecode.model.defaultLabel')}:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {modelConfig.fallbackModel}
                    </Text>
                  </div>
                )}
                {modelConfig.roles.haiku.model && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      Haiku:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {modelConfig.roles.haiku.model}
                    </Text>
                  </div>
                )}
                {modelConfig.roles.sonnet.model && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      Sonnet:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {modelConfig.roles.sonnet.model}
                    </Text>
                  </div>
                )}
                {modelConfig.roles.opus.model && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      Opus:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {modelConfig.roles.opus.model}
                    </Text>
                  </div>
                )}
                {modelConfig.roles.fable.model && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      Fable:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {modelConfig.roles.fable.model}
                    </Text>
                  </div>
                )}
                {modelConfig.legacyReasoningModel && (
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t('claudecode.model.reasoningLabel')}:
                    </Text>{' '}
                    <Text code style={{ fontSize: 12 }}>
                      {modelConfig.legacyReasoningModel}
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
              {t('claudecode.provider.apply')}
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
                  {t('claudecode.provider.apply')}
                </Button>
              </span>
            </Tooltip>
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
