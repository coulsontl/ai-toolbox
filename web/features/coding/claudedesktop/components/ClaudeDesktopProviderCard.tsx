import React from 'react';
import { Card, Space, Button, Dropdown, Switch, Tag, Typography, Tooltip, message } from 'antd';
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
import { BarChart2, Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { ClaudeDesktopProvider } from '@/types/claudedesktop';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import AppliedTag from '@/components/common/AppliedTag';
import ProxyTag from '@/components/common/ProxyTag';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import { ManagementCheckbox } from '@/features/coding/shared/management';
import {
  canApplyProviderWithGatewayProxy,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  hasNonClaudeModelIds,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import {
  getClaudeConfiguredModelIds,
  parseClaudeSettingsConfig,
} from '../../claudecode/utils/claudeModelConfig';

const { Text } = Typography;

/** claude-safe route_id per role, mirroring cc-switch CLAUDE_DESKTOP_ROLE_ROUTE_IDS. */
const CLAUDE_DESKTOP_ROLE_ROUTE_IDS: Record<string, string> = {
  sonnet: 'claude-sonnet-5',
  opus: 'claude-opus-5',
  fable: 'claude-fable-5',
  haiku: 'claude-haiku-4-5',
};
const CLAUDE_DESKTOP_ROLE_ROUTE_ORDER: Array<'sonnet' | 'opus' | 'fable' | 'haiku'> = [
  'sonnet',
  'opus',
  'fable',
  'haiku',
];

interface ClaudeDesktopProviderCardProps {
  provider: ClaudeDesktopProvider;
  isApplied: boolean;
  onEdit: (provider: ClaudeDesktopProvider) => void;
  onDelete: (provider: ClaudeDesktopProvider) => void;
  onCopy: (provider: ClaudeDesktopProvider) => void;
  onShare?: (provider: ClaudeDesktopProvider) => void;
  onTest: (provider: ClaudeDesktopProvider) => void;
  onSelect: (provider: ClaudeDesktopProvider) => void;
  onToggleDisabled: (provider: ClaudeDesktopProvider, isDisabled: boolean) => void;
  connectivityStatus?: ProviderConnectivityStatusItem;
  gatewayTakeoverActive?: boolean;
  gatewayStatus?: GatewayCliTakeoverStatus | null;
  onGatewayStatusChange?: (status: GatewayCliTakeoverStatus) => void | Promise<void>;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (checked: boolean) => void;
}

const ClaudeDesktopProviderCard: React.FC<ClaudeDesktopProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
  onShare,
  onTest,
  onSelect,
  onToggleDisabled,
  connectivityStatus,
  gatewayTakeoverActive = false,
  gatewayStatus = null,
  onGatewayStatusChange,
  selectable = false,
  selected = false,
  onSelectChange,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);
  const [switchingGatewayProvider, setSwitchingGatewayProvider] = React.useState(false);

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

  const settingsConfig = React.useMemo(
    () => parseClaudeSettingsConfig(provider.settingsConfig),
    [provider.settingsConfig],
  );
  const modelRoutes = provider.meta?.claudeDesktopModelRoutes;
  // Effective role routes drive both the model grid and the gateway/connectivity
  // decisions. Prefer meta.claudeDesktopModelRoutes (Desktop form); fall back to
  // deriving the same routes from the Claude Code style env role models, so rows
  // imported before re-save still display their mappings on the card.
  const effectiveRoutes = React.useMemo(() => {
    if (modelRoutes && Object.keys(modelRoutes).length > 0) {
      return modelRoutes;
    }
    const env = (settingsConfig.env || {}) as Record<string, string | undefined>;
    const derived: Record<string, { model: string; labelOverride?: string; supports1m?: boolean; tierAlias?: string }> = {};
    (Object.keys(CLAUDE_DESKTOP_ROLE_ROUTE_IDS) as Array<keyof typeof CLAUDE_DESKTOP_ROLE_ROUTE_IDS>).forEach(
      (role) => {
        const routeId = CLAUDE_DESKTOP_ROLE_ROUTE_IDS[role];
        const model = env[`ANTHROPIC_DEFAULT_${role.toUpperCase()}_MODEL`];
        const trimmed = typeof model === 'string' ? model.trim() : '';
        if (!trimmed) {
          return;
        }
        const name = env[`ANTHROPIC_DEFAULT_${role.toUpperCase()}_MODEL_NAME`];
        const displayName = typeof name === 'string' ? name.trim() : '';
        derived[routeId] = {
          model: trimmed,
          labelOverride: displayName || trimmed.replace(/\s*\[1m\]$/i, ''),
          supports1m: false,
        };
      },
    );
    return derived;
  }, [modelRoutes, settingsConfig]);
  const routeModelIds = React.useMemo(
    () =>
      [
        ...new Set(
          Object.values(effectiveRoutes)
            .map((r) => r.model.trim())
            .filter(Boolean),
        ),
      ],
    [effectiveRoutes],
  );
  const envModelIds = React.useMemo(
    () => getClaudeConfiguredModelIds(settingsConfig, { stripOneMMarker: true }),
    [settingsConfig],
  );
  const configuredModelIds = React.useMemo(
    () => (routeModelIds.length > 0 ? routeModelIds : envModelIds),
    [routeModelIds, envModelIds],
  );
  const configuredApiKey =
    settingsConfig.env?.ANTHROPIC_AUTH_TOKEN?.trim() ||
    settingsConfig.env?.ANTHROPIC_API_KEY?.trim() ||
    '';
  const configuredBaseUrl = settingsConfig.env?.ANTHROPIC_BASE_URL?.trim() || '';
  const isOfficialProvider = provider.category === 'official';
  const showRuntimeApplied = isApplied;
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const providerApiFormat = React.useMemo(
    () => firstGatewayApiFormat(
      getGatewayProviderApiFormatFromMeta(provider.meta, 'claude_desktop'),
      provider.meta?.apiFormat,
      typeof (settingsConfig as { apiFormat?: unknown }).apiFormat === 'string'
        ? (settingsConfig as { apiFormat?: string }).apiFormat
        : undefined,
      typeof (settingsConfig as { api_format?: unknown }).api_format === 'string'
        ? (settingsConfig as { api_format?: string }).api_format
        : undefined,
    ),
    [gatewayProviderProfilesVersion, provider.meta, settingsConfig],
  );
  const needsGatewayProxy =
    !isOfficialProvider &&
    (providerNeedsGatewayProxy(providerApiFormat, 'anthropic') ||
      hasNonClaudeModelIds(configuredModelIds));
  const gatewayCanApplyProxy = canApplyProviderWithGatewayProxy(gatewayStatus);
  const gatewayMode = gatewayStatus?.mode ?? null;
  const gatewayFailoverActive = gatewayMode === 'failover';
  const gatewayProxyActive = gatewayMode === 'single' || gatewayFailoverActive;
  const priorityEntry = gatewayFailoverActive
    ? gatewayStatus?.provider_priorities.find((entry) => entry.provider_id === provider.id)
    : undefined;
  const isGatewayPrimary = priorityEntry?.label === 'P0';
  const canShowGatewayProxyButton =
    showRuntimeApplied &&
    !gatewayMode &&
    Boolean(gatewayStatus?.can_takeover) &&
    !provider.isDisabled &&
    !isOfficialProvider;
  // Claude Desktop 的「恢复直连」= restore_official（脱离网关回到官方 1P），不依赖当前
  // provider 能否直连，因此只看后端 can_restore_direct，不叠加 needsGatewayProxy gate。
  // 其他 CLI 的恢复直连会重新直连当前 provider，那里继续保留 gate（见各自 ProviderCard）。
  const canRestoreDirect = showRuntimeApplied && gatewayProxyActive && Boolean(gatewayStatus?.can_restore_direct);
  const canShowRestoreDirectButton = canRestoreDirect;
  const canSwitchGatewayProvider =
    gatewayProxyActive &&
    !isApplied &&
    !provider.isDisabled &&
    !isOfficialProvider;
  const canRunConnectivityTest =
    !isOfficialProvider &&
    Boolean(configuredApiKey) &&
    configuredModelIds.length > 0 &&
    Boolean(configuredBaseUrl);

  const handleToggleDisabled = (checked: boolean) => {
    if (showRuntimeApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !checked);  // Switch 的 checked 表示"启用"，所以取反
  };

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
    ...(onShare ? [{
      key: 'share', label: t('common.share'), icon: <Share2 size={14} />,
      onClick: () => onShare(provider),
    }] : []),
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
  ];

  const hasConfiguredModels = Boolean(effectiveRoutes && Object.keys(effectiveRoutes).length > 0);
  const showProxyTag = showRuntimeApplied && gatewayProxyActive;
  const showApplyAction = !gatewayProxyActive && !isApplied;
  const showApplyWithProxyAction = showApplyAction && needsGatewayProxy;
  const showDirectApplyAction = showApplyAction && !needsGatewayProxy;
  const showGatewaySwitchAction = canSwitchGatewayProvider;
  const showGatewayLockedApply = gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;
  const applyWithProxyDisabled = provider.isDisabled || !gatewayCanApplyProxy;
  const actionAreaWidth =
    showApplyWithProxyAction
      ? 160
      : showApplyAction || showGatewaySwitchAction || showGatewayLockedApply || canShowGatewayProxyButton || canShowRestoreDirectButton
        ? 140
        : 40;
  const cardBorderColor = selectable && selected
    ? 'var(--ant-color-primary)'
    : isGatewayPrimary
      ? 'var(--color-status-success)'
      : showRuntimeApplied
        ? 'var(--ant-color-primary)'
        : 'var(--color-border-card)';
  const cardBackground = isGatewayPrimary
    ? 'linear-gradient(135deg, color-mix(in srgb, var(--color-status-success) 12%, var(--color-bg-container)), var(--color-bg-container))'
    : showRuntimeApplied
      ? 'var(--color-bg-selected)'
      : undefined;

  const handleEngageGatewayProxy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setEngagingGatewayProxy(true);
    try {
      const nextStatus = await engageProxyGatewaySingle('claude_desktop', provider.id);
      await onGatewayStatusChange?.(nextStatus);
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('claude_desktop', provider.id);
      await onGatewayStatusChange?.(nextStatus);
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
      const nextStatus = await restoreProxyGatewayCliDirect('claude_desktop');
      await onGatewayStatusChange?.(nextStatus);
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('claude_desktop', provider.id);
      await onGatewayStatusChange?.(nextStatus);
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
            {selectable ? (
              <div style={{ display: 'flex', alignItems: 'center', padding: '4px 0' }}>
                <ManagementCheckbox
                  checked={selected}
                  ariaLabel={t('common.batch.selectItem')}
                  onChange={onSelectChange ?? (() => {})}
                />
              </div>
            ) : (
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
            )}
            <Space direction="vertical" size={4} style={{ width: '100%' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <ProviderConnectivityStatus item={connectivityStatus} />
                <ProviderNameLink
                  name={provider.name}
                  baseUrl={configuredBaseUrl}
                  style={{ fontSize: 14, fontWeight: 600 }}
                />
                {configuredBaseUrl && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    {configuredBaseUrl}
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
                  <AppliedTag>{t('claudecode.provider.applied')}</AppliedTag>
                )}
                {showProxyTag && (
                  <ProxyTag>{t('gateway.proxy.proxyTag')}</ProxyTag>
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
                {CLAUDE_DESKTOP_ROLE_ROUTE_IDS &&
                  effectiveRoutes &&
                  CLAUDE_DESKTOP_ROLE_ROUTE_ORDER.map((role) => {
                    const routeId = CLAUDE_DESKTOP_ROLE_ROUTE_IDS[role];
                    const route = effectiveRoutes[routeId];
                    if (!route?.model) {
                      return null;
                    }
                    return (
                      <div key={role}>
                        <Text type="secondary" style={{ fontSize: 12 }}>
                          {role.charAt(0).toUpperCase() + role.slice(1)}:
                        </Text>{' '}
                        <Text code style={{ fontSize: 12 }}>
                          {route.labelOverride || route.model}
                        </Text>
                        {route.tierAlias && (
                          <Tag style={{ fontSize: 10, marginInlineStart: 4 }}>
                            {t('claudecode.model.tierAliasLabel')}: {route.tierAlias}
                          </Tag>
                        )}
                      </div>
                    );
                  })}
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

              {provider.notes && hasConfiguredModels && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {provider.notes}
                  </Text>
                </div>
              )}
            </Space>
          </div>

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

export default ClaudeDesktopProviderCard;
