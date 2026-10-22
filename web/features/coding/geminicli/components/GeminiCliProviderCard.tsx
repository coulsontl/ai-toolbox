import React from 'react';
import { Button, Space, Tag, Tooltip, Typography, message } from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
  LinkOutlined,
} from '@ant-design/icons';
import { BarChart2, Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import type { GeminiCliOfficialAccount, GeminiCliProvider, GeminiCliSettingsConfig } from '@/types/geminicli';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import AppliedTag from '@/components/common/AppliedTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import ProxyTag from '@/components/common/ProxyTag';
import {
  GEMINI_CLI_LOCAL_PROVIDER_ID,
  isGeminiCliLocalProviderId,
  shouldShowGeminiCliOfficialAccounts,
} from '../utils/localProvider';
import {
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  isGatewayAggregateMode,
  isGatewayFailoverMode,
  isGatewayProxyMode,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import ClaudeStyleCard from '@/features/coding/shared/providerCardVariants/ClaudeStyleCard';
import {
  renderProviderPrimaryAction,
} from '@/features/coding/shared/providerCardVariants/providerCardActions';
import type {
  ProviderCardMetaEntry,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import {
  ProviderCompactCard,
  type ProviderDetailSection,
} from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import {
  OfficialAccountCount,
  OfficialAccountHeadingIcon,
  OfficialAccountHint,
  OfficialAccountsSection,
} from '@/features/coding/shared/officialAccounts';
import type {
  OfficialAccountPendingAction,
  OfficialAccountRowView,
} from '@/features/coding/shared/officialAccounts';

const { Text } = Typography;

interface GeminiCliProviderCardProps {
  provider: GeminiCliProvider;
  isApplied: boolean;
  /** Which layout the page is in; `'card'` renders the compact grid cell. */
  viewMode?: ProviderViewMode;
  onEdit: (provider: GeminiCliProvider) => void;
  onDelete: (provider: GeminiCliProvider) => void;
  onCopy: (provider: GeminiCliProvider) => void;
  onShare: (provider: GeminiCliProvider) => void;
  onSelect: (provider: GeminiCliProvider) => void;
  onToggleDisabled: (provider: GeminiCliProvider, isDisabled: boolean) => void;
  /** Hides the drag handle when reordering is unavailable. */
  dragDisabled?: boolean;
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
  gatewayStatus?: GatewayCliTakeoverStatus | null;
  onGatewayStatusChange?: (status: GatewayCliTakeoverStatus) => void | Promise<void>;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (checked: boolean) => void;
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
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

/**
 * A Gemini CLI provider, rendered in the Claude style.
 *
 * Style choice follows SOP §4.2.1: the CLI stores a single `GEMINI_MODEL`
 * rather than a model catalog, and exactly one provider is applied. Official
 * logins live in an embedded account block (`footer`), which is the same slot
 * Codex and Kimi use.
 *
 * Everything below is this CLI's own semantics — settings parsing, the gateway
 * decisions and the account handlers. The layouts come from the shared cards.
 */
const GeminiCliProviderCard: React.FC<GeminiCliProviderCardProps> = ({
  provider,
  isApplied,
  viewMode = 'list',
  onEdit,
  onDelete,
  onCopy,
  onShare,
  onSelect,
  onToggleDisabled,
  dragDisabled = false,
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
  gatewayStatus = null,
  onGatewayStatusChange,
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(true);
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);
  const [switchingGatewayProvider, setSwitchingGatewayProvider] = React.useState(false);

  const settingsConfig = React.useMemo(
    () => parseSettingsConfig(provider.settingsConfig),
    [provider.settingsConfig],
  );

  const env = settingsConfig.env || {};
  const baseUrl = env.GOOGLE_GEMINI_BASE_URL?.trim();
  const modelName = extractModelName(settingsConfig);
  const maskedApiKey = maskSecret(env.GEMINI_API_KEY || env.GOOGLE_API_KEY);
  const isOfficialProvider = provider.category === 'official';
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  const providerProfileApiFormat = React.useMemo(
    () => getGatewayProviderApiFormatFromMeta(provider.meta, 'gemini'),
    [gatewayProviderProfilesVersion, provider.meta],
  );
  const providerApiFormat = firstGatewayApiFormat(
    providerProfileApiFormat,
    provider.meta?.apiFormat,
  );
  const isLocalProvider = isGeminiCliLocalProviderId(provider.id);
  // `__local__` is a local-file bridge, not a managed applied preset.
  const showRuntimeApplied = isApplied && !isLocalProvider;
  const needsGatewayProxy =
    !isOfficialProvider &&
    !isLocalProvider &&
    providerNeedsGatewayProxy(providerApiFormat, 'gemini_native');
  const restoreDirectUnavailableTitle = t(
    'gateway.proxy.restoreDirectUnavailableHintProtocol',
    { cli: t('settings.gateway.cli.gemini') },
  );
  const gatewayMode = gatewayStatus?.mode ?? null;
  const gatewayFailoverActive = isGatewayFailoverMode(gatewayMode);
  const gatewayAggregateActive = isGatewayAggregateMode(gatewayMode);
  const gatewayProxyActive = isGatewayProxyMode(gatewayMode);
  const priorityEntry = gatewayFailoverActive
    ? gatewayStatus?.provider_priorities.find((entry) => entry.provider_id === provider.id)
    : undefined;
  const isGatewayPrimary = priorityEntry?.label === 'P0';
  const shouldShowOfficialAccounts = shouldShowGeminiCliOfficialAccounts(
    provider,
    officialAccounts.length,
  );
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
    // Aggregate has no single primary to switch; its site list is edited in the
    // gateway settings aggregate block, so hide the P0-style switch action.
    !gatewayAggregateActive &&
    !isApplied &&
    !provider.isDisabled &&
    !isOfficialProvider &&
    !isLocalProvider;
  const showApplyAction = !gatewayProxyActive && !isApplied && !isLocalProvider;
  const showGatewaySwitchAction = canSwitchGatewayProvider;
  const showGatewayLockedApply = gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;

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
      const nextStatus = await engageProxyGatewaySingle('gemini', provider.id);
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

  const handleRestoreDirect = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setRestoringDirect(true);
    try {
      const nextStatus = await restoreProxyGatewayCliDirect('gemini');
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
      const nextStatus = await switchProxyGatewayPrimaryProvider('gemini', provider.id);
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

  /** The shared switch hands over the new enabled state; invert it once, here. */
  const handleToggleDisabled = (enabled: boolean) => {
    if (showRuntimeApplied && !enabled) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !enabled);
  };

  const formatOfficialAccountLabel = (account: GeminiCliOfficialAccount) => {
    if (account.id === GEMINI_CLI_LOCAL_PROVIDER_ID) {
      return account.email || t('geminicli.provider.officialAccountLocal');
    }
    return account.email || account.projectId || account.name;
  };

  const officialAccountRows = React.useMemo<OfficialAccountRowView[]>(
    () =>
      officialAccounts.map((account) => ({
        id: account.id,
        label: formatOfficialAccountLabel(account),
        kindTag: account.id === GEMINI_CLI_LOCAL_PROVIDER_ID
          ? t('geminicli.provider.officialAccountLocalTag')
          : t('geminicli.provider.officialAccountOauthTag'),
        metaLines: [
          account.projectId,
          account.planType,
          account.limit5hText
            ? `${t('geminicli.provider.officialAccountShortWindowLimitLabel', {
                label: account.limitShortLabel || '5h',
              })}: ${account.limit5hText}`
            : undefined,
          account.limitWeeklyText
            ? `${t('geminicli.provider.officialAccountWeeklyLimitLabel')}: ${account.limitWeeklyText}`
            : undefined,
        ].filter((line): line is string => Boolean(line)),
        isApplied: showOfficialRuntimeState && account.isApplied,
        isVirtual: account.isVirtual,
        lastError: account.lastError
          ? t('geminicli.provider.officialAccountLastError', { message: account.lastError })
          : undefined,
      })),
    [officialAccounts, showOfficialRuntimeState, t],
  );

  const findOfficialAccount = (row: OfficialAccountRowView) =>
    officialAccounts.find((account) => account.id === row.id);

  const officialAccountPending = React.useMemo<OfficialAccountPendingAction | null>(() => {
    if (refreshingOfficialAccountId) {
      return { accountId: refreshingOfficialAccountId, action: 'refresh' };
    }
    if (savingOfficialAccountId) {
      return { accountId: savingOfficialAccountId, action: 'save' };
    }
    return null;
  }, [refreshingOfficialAccountId, savingOfficialAccountId]);

  /**
   * The account block.
   *
   * `fold` is what the list card gets; the detail sidebar renders the same
   * section uncollapsed (`ProviderDetailPanel` opens it for reading, so a fold
   * there would only be a header in front of the rows).
   */
  const renderOfficialAccounts = (fallback: 'footer' | 'detail') => {
    if (!shouldShowOfficialAccounts) {
      return undefined;
    }
    return (
      <OfficialAccountsSection
        variant="embedded"
        listTitle={t('common.officialAccount.listTitle')}
        emptyText={t('geminicli.provider.officialAccountsEmpty')}
        accounts={officialAccountRows}
        {...(fallback === 'footer'
          ? {
              collapsed: accountsCollapsed,
              onToggleCollapsed: () => setAccountsCollapsed((current) => !current),
            }
          : {})}
        pending={officialAccountPending}
        applyHint={t('geminicli.officialAccount.applyHint')}
        loginAction={
          isOfficialProvider ? (
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
          )
        }
        onRefresh={onOfficialAccountRefresh
          ? (row) => {
              const account = findOfficialAccount(row);
              if (account) onOfficialAccountRefresh(provider, account);
            }
          : undefined}
        onViewDetails={onOfficialAccountViewDetails
          ? (row) => {
              const account = findOfficialAccount(row);
              if (account) onOfficialAccountViewDetails(provider, account);
            }
          : undefined}
        onSaveLocal={onOfficialLocalAccountSave
          ? (row) => {
              const account = findOfficialAccount(row);
              if (account) onOfficialLocalAccountSave(provider, account);
            }
          : undefined}
        onApply={showOfficialRuntimeState && onOfficialAccountApply
          ? (row) => {
              const account = findOfficialAccount(row);
              if (account) onOfficialAccountApply(provider, account);
            }
          : undefined}
        onDelete={onOfficialAccountDelete
          ? (row) => {
              const account = findOfficialAccount(row);
              if (account) onOfficialAccountDelete(provider, account);
            }
          : undefined}
      />
    );
  };

  const stateTags = (
    <>
      {isLocalProvider && (
        <Text type="secondary" style={{ fontSize: 11 }}>
          ({t('geminicli.localConfigHint')})
        </Text>
      )}
      {isOfficialProvider && (
        <OfficialAccountCount count={officialAccounts.length} />
      )}
      {isOfficialProvider && gatewayTakeoverActive && (
        <Tooltip title={t('gateway.takeover.officialBypassedTooltip')}>
          <Tag color="gold">{t('gateway.takeover.officialBypassedTag')}</Tag>
        </Tooltip>
      )}
      {showRuntimeApplied && (
        <AppliedTag>{t('geminicli.provider.applied')}</AppliedTag>
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
    </>
  );

  /**
   * The second line.
   *
   * The official card has no endpoint and no key (it authenticates through the
   * CLI's own login), and what a reader of *that* card needs is what switching
   * accounts does — the same choice Codex made. A custom card states the
   * endpoint, the model, the masked key and notes.
   */
  const metaEntries = React.useMemo<ProviderCardMetaEntry[]>(() => {
    if (isOfficialProvider) {
      const entries: ProviderCardMetaEntry[] = [{
        kind: 'text',
        value: <OfficialAccountHint>{t('geminicli.provider.officialAccountHint')}</OfficialAccountHint>,
      }];
      // The official card has no endpoint and no key, but it does have notes —
      // the form shows the notes field in both modes, and the bespoke card
      // rendered them on this line for every provider. The switching hint
      // replaces the connection facts, not the user's own text.
      if (provider.notes) {
        entries.push({ kind: 'text', value: provider.notes });
      }
      return entries;
    }
    const entries: ProviderCardMetaEntry[] = [];
    if (baseUrl) {
      entries.push({ kind: 'code', value: baseUrl });
    }
    if (modelName) {
      entries.push({ kind: 'tag', value: modelName, color: 'blue' });
    }
    if (maskedApiKey) {
      entries.push({ kind: 'text', value: `API Key: ${maskedApiKey}` });
    }
    if (provider.notes) {
      entries.push({ kind: 'text', value: provider.notes });
    }
    return entries;
  }, [baseUrl, isOfficialProvider, maskedApiKey, modelName, provider.notes, t]);

  const gatewayActions = (
    <>
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
        <Tooltip title={restoreDirectUnavailableTitle}>
          <Button type="link" size="small" disabled>
            {t('gateway.proxy.restoreDirectButton')}
          </Button>
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
              {t('geminicli.provider.apply')}
            </Button>
          </span>
        </Tooltip>
      )}
    </>
  );

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.id,
      name: isOfficialProvider ? t('common.officialAccount.headingTitle') : provider.name,
      baseUrl,
    },
    providerState: {
      isDisabled: provider.isDisabled,
      onToggleDisabled: isLocalProvider ? undefined : handleToggleDisabled,
      draggable: !selectable && !dragDisabled,
      sortableId: provider.id,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
      dimmed: provider.isDisabled,
      accent: isGatewayPrimary
        ? 'gatewayPrimary'
        : showRuntimeApplied
          ? 'applied'
          : undefined,
    },
    actions: {
      onEdit: () => onEdit(provider),
      onCopy: () => onCopy(provider),
      onShare: () => onShare(provider),
      onDelete: isLocalProvider ? undefined : () => onDelete(provider),
      enabledStateLabel: provider.isDisabled
        ? t('geminicli.configDisabled')
        : t('geminicli.configEnabled'),
      gatewayActions,
      primaryAction: showApplyAction
        ? {
            label: t('geminicli.provider.apply'),
            icon: <CheckOutlined />,
            onClick: () => onSelect(provider),
            disabled: provider.isDisabled,
          }
        : undefined,
    },
    namePrefix: isOfficialProvider ? <OfficialAccountHeadingIcon /> : undefined,
    nameTags: stateTags,
    metaEntries,
    // The name row doubles as the heading of the account block: `🔗 官方账号 (n)`.
    // `nameTags` carries the count (`OfficialAccountCount`), `namePrefix` the glyph.
    footer: renderOfficialAccounts('footer'),
  };

  if (viewMode !== 'card') {
    return <ClaudeStyleCard {...props} />;
  }

  /**
   * The cell's second line states one thing. For an ordinary provider that is
   * its model; for the official card it is the account the CLI is currently
   * using — applied account → the login on disk (`isVirtual`) → "not signed
   * in". The line is reserved unconditionally by the shared card, so leaving it
   * blank would read as a broken cell (the same rule Codex, Kimi and ZCode
   * follow).
   */
  const currentLoginRow = showOfficialRuntimeState
    ? officialAccountRows.find((row) => row.isApplied) ?? officialAccountRows.find((row) => row.isVirtual)
    : undefined;
  const cardSubtitle = isOfficialProvider
    ? [{
        label: t('common.officialAccount.currentLogin'),
        value: currentLoginRow?.label ?? t('common.officialAccount.currentLoginNone'),
      }]
    : modelName
      ? [{ label: t('geminicli.provider.defaultModelLabel'), value: modelName }]
      : undefined;

  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('geminicli.provider.baseUrl'),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      {
        key: 'apiKey',
        label: t('geminicli.provider.apiKey'),
        value: env.GEMINI_API_KEY || env.GOOGLE_API_KEY,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
        elideMiddle: true,
      },
      {
        key: 'model',
        label: t('geminicli.provider.model'),
        value: modelName,
        emptyText: t('common.provider.notConfigured'),
      },
    ],
  }];

  const detailOfficialAccounts = renderOfficialAccounts('detail');
  if (detailOfficialAccounts) {
    detailSections.push({ key: 'officialAccounts', content: detailOfficialAccounts });
  }
  if (provider.notes) {
    detailSections.push({
      key: 'notes',
      title: t('common.provider.notes'),
      fields: [{ key: 'notes', label: t('common.provider.notes'), value: provider.notes }],
    });
  }

  return (
    <ProviderCompactCard
      provider={props.provider}
      subtitle={cardSubtitle}
      // One fact and the action set (gateway / apply): the same shape Codex's
      // cell uses, so the action row rides the second line instead of opening
      // a mostly empty footer bar.
      actionPlacement="subtitle"
      providerState={props.providerState}
      actions={props.actions}
      namePrefix={props.namePrefix}
      nameTags={stateTags}
      detail={{
        title: (
          <ProviderNameLink
            name={isOfficialProvider ? t('common.officialAccount.headingTitle') : provider.name}
            baseUrl={baseUrl}
          />
        ),
        statusTags: stateTags,
        sections: detailSections,
        primaryActions: (
          <>
            {gatewayActions}
            {renderProviderPrimaryAction(props.actions?.primaryAction)}
          </>
        ),
        secondaryActions: (
          <Space size={4} wrap>
            <Button size="small" type="text" onClick={() => onEdit(provider)}>
              {t('common.edit')}
            </Button>
            <Button size="small" type="text" onClick={() => onCopy(provider)}>
              {t('common.copy')}
            </Button>
            <Button size="small" type="text" icon={<Share2 size={14} aria-hidden="true" />} onClick={() => onShare(provider)}>
              {t('common.share')}
            </Button>
            {!isLocalProvider && (
              <Button size="small" type="text" danger onClick={() => onDelete(provider)}>
                {t('common.delete')}
              </Button>
            )}
          </Space>
        ),
      }}
    />
  );
};

export default GeminiCliProviderCard;
