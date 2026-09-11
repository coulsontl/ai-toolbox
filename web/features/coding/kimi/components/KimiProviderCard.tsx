import React from 'react';
import { Card, Space, Button, Dropdown, Switch, Tag, Typography, Tooltip, message } from 'antd';
import type { MenuProps } from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
  CopyOutlined,
  EditOutlined,
  DeleteOutlined,
  MoreOutlined,
  HolderOutlined,
  GlobalOutlined,
} from '@ant-design/icons';
import { BarChart2, Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { KimiProvider, KIMI_LOCAL_PROVIDER_ID } from '@/types/kimi';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import {
  extractKimiBaseUrl,
  extractKimiDefaultModel,
} from '../utils/settingsConfig';
import AppliedTag from '@/components/common/AppliedTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProxyTag from '@/components/common/ProxyTag';
import ProviderConnectivityStatus from '@/features/coding/shared/providerConnectivity/ProviderConnectivityStatus';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import {
  canApplyProviderWithGatewayProxy,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  openAiApiFormatFromBaseUrl,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import { ManagementCheckbox } from '@/features/coding/shared/management';
import styles from './KimiProviderCard.module.less';

const { Text } = Typography;

interface KimiProviderCardProps {
  provider: KimiProvider;
  isApplied: boolean;
  gatewayTakeoverActive?: boolean;
  gatewayStatus?: GatewayCliTakeoverStatus | null;
  onGatewayStatusChange?: (status: GatewayCliTakeoverStatus) => void | Promise<void>;
  onEdit: (provider: KimiProvider) => void;
  onDelete: (provider: KimiProvider) => void;
  onApply: (provider: KimiProvider) => void | Promise<void>;
  onToggleDisabled: (provider: KimiProvider, isDisabled: boolean) => void | Promise<void>;
  onTest?: (provider: KimiProvider) => void;
  onCopy?: (provider: KimiProvider) => void;
  onShare?: (provider: KimiProvider) => void;
  connectivityStatus?: ProviderConnectivityStatusItem;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (checked: boolean) => void;
}

const KimiProviderCard: React.FC<KimiProviderCardProps> = ({
  provider,
  isApplied,
  gatewayTakeoverActive = false,
  gatewayStatus = null,
  onGatewayStatusChange,
  onEdit,
  onDelete,
  onApply,
  onToggleDisabled,
  onTest,
  onCopy,
  onShare,
  connectivityStatus,
  selectable = false,
  selected = false,
  onSelectChange,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);

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
    opacity: isDragging ? 0.5 : provider.isDisabled ? 0.6 : 1,
  };

  const isOfficialProvider = provider.category === 'official';
  const isLocalProvider = provider.id === KIMI_LOCAL_PROVIDER_ID;

  // `__local__` is a local-file bridge, not a managed applied preset.
  const showRuntimeApplied = isApplied && !isLocalProvider;

  const baseUrl = React.useMemo(
    () => extractKimiBaseUrl(provider.settingsConfig),
    [provider.settingsConfig],
  );
  const modelName = React.useMemo(
    () => extractKimiDefaultModel(provider.settingsConfig),
    [provider.settingsConfig],
  );

  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const providerProfileApiFormat = React.useMemo(
    () => getGatewayProviderApiFormatFromMeta(provider.meta, 'kimi'),
    [gatewayProviderProfilesVersion, provider.meta],
  );
  const providerApiFormat = firstGatewayApiFormat(
    providerProfileApiFormat,
    typeof provider.meta?.apiFormat === 'string' ? provider.meta.apiFormat : undefined,
    openAiApiFormatFromBaseUrl(baseUrl),
  );

  const needsGatewayProxy =
    !isOfficialProvider &&
    !isLocalProvider &&
    providerNeedsGatewayProxy(providerApiFormat, 'openai_chat');

  const restoreDirectUnavailableTitle = t(
    'gateway.proxy.restoreDirectUnavailableHintProtocol',
    { cli: t('settings.gateway.cli.kimi') },
  );

  const gatewayCanApplyProxy = canApplyProviderWithGatewayProxy(gatewayStatus);
  const gatewayMode = gatewayStatus?.mode ?? null;
  const gatewayFailoverActive = gatewayMode === 'failover';
  const gatewayProxyActive = gatewayMode === 'single' || gatewayFailoverActive;
  const priorityEntry = gatewayFailoverActive
    ? gatewayStatus?.provider_priorities.find((entry) => entry.provider_id === provider.id)
    : undefined;
  const isGatewayPrimary = priorityEntry?.label === 'P0';

  const showProxyTag = showRuntimeApplied && gatewayProxyActive;
  const canShowGatewayProxyButton =
    showRuntimeApplied &&
    !gatewayMode &&
    Boolean(gatewayStatus?.can_takeover) &&
    !provider.isDisabled &&
    !isOfficialProvider &&
    !isLocalProvider;
  const canRestoreDirect =
    showRuntimeApplied && gatewayProxyActive && Boolean(gatewayStatus?.can_restore_direct);
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
  const showGatewayLockedApply =
    gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;
  const applyWithProxyDisabled = provider.isDisabled || !gatewayCanApplyProxy;

  const actionAreaWidth =
    showApplyWithProxyAction
      ? 160
      : showApplyAction ||
          showGatewaySwitchAction ||
          showGatewayLockedApply ||
          canShowGatewayProxyButton ||
          canShowRestoreDirectButton ||
          canShowRestoreDirectUnavailable
        ? 140
        : 40;

  const refreshTrayAfterGatewayChange = () => {
    void refreshTrayMenu().catch(() => {});
  };

  const handleEngageGatewayProxy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setEngagingGatewayProxy(true);
    try {
      const nextStatus = await engageProxyGatewaySingle('kimi', provider.id);
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('kimi', provider.id);
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
      const nextStatus = await restoreProxyGatewayCliDirect('kimi');
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
    setEngagingGatewayProxy(true);
    try {
      const nextStatus = await switchProxyGatewayPrimaryProvider('kimi', provider.id);
      onGatewayStatusChange?.(nextStatus);
      refreshTrayAfterGatewayChange();
      message.success(t('gateway.proxy.notice.switched'));
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(t('gateway.proxy.notice.switchFailed', { error: errorMessage }));
    } finally {
      setEngagingGatewayProxy(false);
    }
  };

  const handleToggleDisabled = (checked: boolean) => {
    if (showRuntimeApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    void onToggleDisabled(provider, !checked);
  };

  const menuItems: MenuProps['items'] = [
    ...(isLocalProvider
      ? []
      : [
          {
            key: 'toggle',
            label: (
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                  <span>{t('common.enable')}</span>
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    {provider.isDisabled ? t('kimi.providerDisabled') : t('kimi.providerEnabled')}
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
        ]),
    {
      key: 'edit',
      label: t('common.edit'),
      icon: <EditOutlined />,
      onClick: () => onEdit(provider),
    },
    {
      key: 'test',
      label: t('opencode.connectivity.button'),
      icon: <ApiOutlined />,
      // Official channels authenticate via the OAuth login state; there is no
      // static API key to probe directly (same rule as grok/claude/codex).
      disabled: isOfficialProvider || provider.isDisabled,
      onClick: () => onTest?.(provider),
    },
    {
      key: 'copy',
      label: t('common.copy'),
      icon: <CopyOutlined />,
      onClick: () => onCopy?.(provider),
    },
    ...(onShare ? [{
      key: 'share', label: t('common.share'), icon: <Share2 size={14} />,
      onClick: () => onShare(provider),
    }] : []),
    ...(isLocalProvider
      ? []
      : [
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
        ]),
  ];

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

  return (
    <div ref={setNodeRef} style={sortableStyle}>
      <Card
        size="small"
        className={styles.card}
        style={{
          borderColor: cardBorderColor,
          background: cardBackground,
          marginBottom: 12,
          boxShadow: 'var(--shadow-card-sm)',
          transition: 'all 0.3s ease',
        }}
        styles={{
          body: {
            padding: '12px 16px',
          },
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          {/* Drag Handle / Selection Checkbox */}
          {selectable ? (
            <div style={{ display: 'flex', alignItems: 'center', padding: '4px 0' }}>
              <ManagementCheckbox
                checked={selected}
                ariaLabel={t('common.batch.selectItem')}
                onChange={onSelectChange ?? (() => {})}
                style={{ width: 13, height: 13 }}
              />
            </div>
          ) : (
            <div
              {...attributes}
              {...listeners}
              className={styles.dragHandle}
              style={{
                cursor: 'grab',
                display: 'flex',
                alignItems: 'center',
                color: 'var(--color-text-tertiary)',
                padding: '4px 2px',
                borderRadius: 4,
                flexShrink: 0,
              }}
            >
              <HolderOutlined style={{ fontSize: 14 }} />
            </div>
          )}

          {/* Provider Info */}
          <div style={{ flex: 1, minWidth: 0 }}>
            <Space direction="vertical" size={4} style={{ width: '100%' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <ProviderConnectivityStatus item={connectivityStatus} />
                <ProviderNameLink
                  name={provider.name}
                  baseUrl={baseUrl}
                  style={{ fontSize: 14, fontWeight: 600 }}
                />
                {isLocalProvider && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    ({t('kimi.localConfigHint')})
                  </Text>
                )}
                {isOfficialProvider && (
                  <Tag>{t('kimi.provider.modeOfficial')}</Tag>
                )}
                {isOfficialProvider && gatewayTakeoverActive && (
                  <Tooltip title={t('gateway.takeover.officialBypassedTooltip')}>
                    <Tag color="gold">{t('gateway.takeover.officialBypassedTag')}</Tag>
                  </Tooltip>
                )}
                {showRuntimeApplied && (
                  <AppliedTag>{t('kimi.provider.applied')}</AppliedTag>
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
                  <Tag
                    color={priorityEntry.label === 'P0' ? 'success' : 'default'}
                    style={{ margin: 0 }}
                  >
                    {priorityEntry.label}
                  </Tag>
                )}
              </div>

              {/* Meta Info */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 16, flexWrap: 'wrap' }}>
                {isLocalProvider ? (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    ({t('kimi.localConfigHint')})
                  </Text>
                ) : (
                  <>
                    {modelName && (
                      <Text type="secondary" style={{ fontSize: 12 }}>
                        <BarChart2
                          size={12}
                          style={{ marginRight: 4, verticalAlign: -1 }}
                        />
                        {modelName}
                      </Text>
                    )}
                    {baseUrl && (
                      <Text
                        type="secondary"
                        style={{
                          fontSize: 12,
                          maxWidth: 320,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        <GlobalOutlined style={{ marginRight: 4 }} />
                        {baseUrl}
                      </Text>
                    )}
                  </>
                )}
              </div>

              {provider.notes && (
                <Text
                  type="secondary"
                  style={{
                    fontSize: 12,
                    display: 'block',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {provider.notes}
                </Text>
              )}
            </Space>
          </div>

          {/* Action Buttons Area */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'flex-end',
              gap: 4,
              flexShrink: 0,
              width: actionAreaWidth,
            }}
          >
            {canShowGatewayProxyButton && (
              <Tooltip title={t('gateway.proxy.singleHint')}>
                <Button
                  type="link"
                  size="small"
                  icon={<ApiOutlined />}
                  loading={engagingGatewayProxy}
                  onClick={handleEngageGatewayProxy}
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
                  loading={restoringDirect}
                  onClick={handleRestoreDirect}
                >
                  {t('gateway.proxy.restoreDirectButton')}
                </Button>
              </Tooltip>
            )}

            {canShowRestoreDirectUnavailable && (
              <Tooltip title={restoreDirectUnavailableTitle}>
                <Button type="link" size="small" disabled>
                  {t('gateway.proxy.restoreDirectButton')}
                </Button>
              </Tooltip>
            )}

            {showDirectApplyAction && (
              <Button
                type="link"
                size="small"
                icon={<CheckOutlined />}
                disabled={provider.isDisabled}
                onClick={() => onApply(provider)}
              >
                {t('kimi.provider.apply')}
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
                    disabled={applyWithProxyDisabled}
                    loading={engagingGatewayProxy}
                    onClick={handleApplyWithGatewayProxy}
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
                  loading={engagingGatewayProxy}
                  onClick={handleSwitchGatewayProvider}
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
                    {t('common.apply')}
                  </Button>
                </span>
              </Tooltip>
            )}

            <Dropdown menu={{ items: menuItems }} trigger={['click']} placement="bottomRight">
              <Button
                type="text"
                size="small"
                icon={<MoreOutlined />}
                style={{ color: 'var(--color-text-secondary)' }}
              />
            </Dropdown>
          </div>
        </div>
      </Card>
    </div>
  );
};

export default KimiProviderCard;
