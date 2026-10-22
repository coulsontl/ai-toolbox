import React from 'react';
import { Button, Space, Tag, Tooltip, Typography, message } from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
  KeyOutlined,
  LinkOutlined,
} from '@ant-design/icons';
import { BarChart2, Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
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
  isGatewayAggregateMode,
  isGatewayFailoverMode,
  isGatewayProxyMode,
  openAiApiFormatFromBaseUrl,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import CodexStyleCard from '@/features/coding/shared/providerCardVariants/CodexStyleCard';
import {
  renderProviderPrimaryAction,
} from '@/features/coding/shared/providerCardVariants/providerCardActions';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
import type {
  ProviderCardMetaEntry,
  ProviderCardModels,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import {
  ProviderCompactCard,
  type ProviderDetailSection,
} from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import type { ModelDisplayData, ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import {
  GROK_LOCAL_PROVIDER_ID,
  isGrokLocalProviderId,
  shouldShowGrokOfficialAccounts,
} from '../utils/localProvider';
import {
  getGrokProviderCatalogModels,
  getGrokProviderDefaultModelKey,
} from '../utils/grokProviderModels';
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

interface GrokProviderCardProps {
  provider: GrokProvider;
  isApplied: boolean;
  /** Which layout the page is in; `'card'` renders the compact grid cell. */
  viewMode?: ProviderViewMode;
  onEdit: (provider: GrokProvider) => void;
  onDelete: (provider: GrokProvider) => void;
  onCopy: (provider: GrokProvider) => void;
  onShare?: (provider: GrokProvider) => void;
  onTest: (provider: GrokProvider) => void;
  onSelect: (provider: GrokProvider) => void;
  onToggleDisabled: (provider: GrokProvider, isDisabled: boolean) => void;
  /** Hides the drag handle when reordering is unavailable. */
  dragDisabled?: boolean;
  onAddModel?: (provider: GrokProvider) => void;
  onEditModel?: (provider: GrokProvider, modelKey: string) => void;
  onDeleteModel?: (provider: GrokProvider, modelKey: string) => void;
  onSetDefaultModel?: (provider: GrokProvider, modelKey: string) => void;
  onFetchModels?: (provider: GrokProvider) => void;
  /** Enter/exit batch-delete selection mode for this provider's model list. */
  onToggleBatchDeleteMode?: (provider: GrokProvider) => void;
  /** Confirm delete of currently selected models (caller shows confirm dialog). */
  onBatchDeleteModels?: (provider: GrokProvider) => void;
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (provider: GrokProvider, modelId: string, selected: boolean) => void;
  officialAccounts?: GrokOfficialAccount[];
  onOfficialAccountLogin?: (provider: GrokProvider) => void;
  onOfficialLocalAccountSave?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountApply?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountDelete?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountRefresh?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountRefreshToken?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  onOfficialAccountViewDetails?: (provider: GrokProvider, account: GrokOfficialAccount) => void;
  refreshingOfficialAccountId?: string | null;
  refreshingOfficialTokenAccountId?: string | null;
  savingOfficialAccountId?: string | null;
  connectivityStatus?: ProviderConnectivityStatusItem;
  gatewayTakeoverActive?: boolean;
  gatewayStatus?: GatewayCliTakeoverStatus | null;
  onGatewayStatusChange?: (status: GatewayCliTakeoverStatus) => void | Promise<void>;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (checked: boolean) => void;
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
}

/**
 * A Grok provider, rendered in the Codex style.
 *
 * Style choice follows SOP §4.2.1: `settingsConfig.modelCatalog` is a real
 * model catalog, and exactly one provider is applied — so the card carries a
 * model section and an 应用 action, unlike the OpenCode style.
 *
 * The gateway decisions, the catalog mapping and the official-account handlers
 * stay here; the layout and the account/row chrome come from the shared layer.
 */
const GrokProviderCard: React.FC<GrokProviderCardProps> = ({
  provider,
  isApplied,
  viewMode = 'list',
  onEdit,
  onDelete,
  onCopy,
  onShare,
  onTest,
  onSelect,
  onToggleDisabled,
  dragDisabled = false,
  onAddModel,
  onEditModel,
  onDeleteModel,
  onSetDefaultModel,
  onFetchModels,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  modelSelectionMode = false,
  selectedModelIds = [],
  onToggleModelSelection,
  officialAccounts = [],
  onOfficialAccountLogin,
  onOfficialLocalAccountSave,
  onOfficialAccountApply,
  onOfficialAccountDelete,
  onOfficialAccountRefresh,
  onOfficialAccountRefreshToken,
  onOfficialAccountViewDetails,
  refreshingOfficialAccountId,
  refreshingOfficialTokenAccountId,
  savingOfficialAccountId,
  connectivityStatus,
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
  const restoreDirectUnavailableTitle = t(
    'gateway.proxy.restoreDirectUnavailableHintProtocol',
    { cli: t('settings.gateway.cli.grok') },
  );
  const gatewayCanApplyProxy = canApplyProviderWithGatewayProxy(gatewayStatus);
  const gatewayMode = gatewayStatus?.mode ?? null;
  const gatewayFailoverActive = isGatewayFailoverMode(gatewayMode);
  const gatewayAggregateActive = isGatewayAggregateMode(gatewayMode);
  const gatewayProxyActive = isGatewayProxyMode(gatewayMode);
  const priorityEntry = gatewayFailoverActive
    ? gatewayStatus?.provider_priorities.find((entry) => entry.provider_id === provider.id)
    : undefined;
  const isGatewayPrimary = priorityEntry?.label === 'P0';
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
    // Aggregate has no single primary to switch; its site list is edited in the
    // gateway settings aggregate block, so hide the P0-style switch action.
    !gatewayAggregateActive &&
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

  /** The shared switch hands over the new enabled state; invert it once, here. */
  const handleToggleDisabled = (enabled: boolean) => {
    if (showRuntimeApplied && !enabled) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !enabled);
  };

  const shouldShowOfficialAccounts = shouldShowGrokOfficialAccounts(
    provider,
    officialAccounts.length,
  );
  /**
   * An official channel with no saved login has nothing to apply: applying
   * it would rewrite `[model.*]` / auth state for an account that does not
   * exist. The button stays visible (a missing button reads as a missing
   * feature) and explains itself on hover — the shared `disabledReason`.
   */
  const officialApplyBlocked = isOfficialProvider && officialAccounts.length === 0;

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

  const officialAccountRows = React.useMemo<OfficialAccountRowView[]>(
    () =>
      officialAccounts.map((account) => ({
        id: account.id,
        label: formatOfficialAccountLabel(account),
        kindTag: account.id === GROK_LOCAL_PROVIDER_ID
          ? t('grok.provider.officialAccountLocalTag')
          : t('grok.provider.officialAccountOauthTag'),
        metaLines: [
          account.planType,
          account.limitWeeklyText
            ? `${t('grok.provider.officialAccountWeeklyLimitLabel')}: ${account.limitWeeklyText}`
            : undefined,
          account.limitMonthlyText
            ? `${t('grok.provider.officialAccountMonthlyLimitLabel')}: ${account.limitMonthlyText}`
            : undefined,
        ].filter((line): line is string => Boolean(line)),
        isApplied: showOfficialRuntimeState && account.isApplied,
        // Grok never synthesizes a virtual (disk-only) login row; the local
        // bridge account is a stored record like any other.
        isVirtual: false,
        lastError: account.lastError
          ? t('grok.provider.officialAccountLastError', { message: account.lastError })
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
   * The account block. `fold` is the list card's folding block; the detail
   * sidebar draws the same section uncollapsed (`ProviderDetailPanel` is opened
   * to read those rows, so a fold there would only be a header in the way).
   */
  const renderOfficialAccounts = (fallback: 'footer' | 'detail') => {
    if (!shouldShowOfficialAccounts) {
      return undefined;
    }
    return (
      <OfficialAccountsSection
        variant="embedded"
        listTitle={t('common.officialAccount.listTitle')}
        emptyText={t('grok.provider.officialAccountsEmpty')}
        accounts={officialAccountRows}
        {...(fallback === 'footer'
          ? {
              collapsed: accountsCollapsed,
              onToggleCollapsed: () => setAccountsCollapsed((current) => !current),
            }
          : {})}
        pending={officialAccountPending}
        applyHint={t('grok.officialAccount.applyHint')}
        loginAction={
          isOfficialProvider ? (
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
        // Grok's sixth row action — re-mint the OAuth token — has no named slot
        // in the shared section (it is the only CLI that needs one), so it rides
        // the free-form slot. The button keeps its own spinner.
        renderExtraActions={onOfficialAccountRefreshToken
          ? (row) => {
              const account = findOfficialAccount(row);
              if (!account || account.id === GROK_LOCAL_PROVIDER_ID) {
                return null;
              }
              return (
                <Button
                  type="text"
                  size="small"
                  icon={<KeyOutlined />}
                  loading={refreshingOfficialTokenAccountId === account.id}
                  style={{ fontSize: 12, height: 'auto', paddingInline: 4 }}
                  onClick={() => onOfficialAccountRefreshToken(provider, account)}
                >
                  {t('grok.provider.officialAccountRefreshToken')}
                </Button>
              );
            }
          : undefined}
      />
    );
  };

  const stateTags = (
    <>
      {isLocalProvider && (
        <Text type="secondary" style={{ fontSize: 11 }}>
          ({t('grok.localConfigHint')})
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
        <AppliedTag>{t('grok.provider.applied')}</AppliedTag>
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
   * The second line. The official card states what switching accounts does
   * instead of an endpoint/key/model it does not have (same choice as Codex and
   * Kimi); a custom card states endpoint, model, protocol, masked key and notes.
   */
  const metaEntries = React.useMemo<ProviderCardMetaEntry[]>(() => {
    if (isOfficialProvider) {
      const entries: ProviderCardMetaEntry[] = [{
        kind: 'text',
        value: <OfficialAccountHint>{t('grok.provider.officialAccountHint')}</OfficialAccountHint>,
        // A node value cannot double as its own hover text; the one-line row
        // may compress this sentence, so the hover carries the full wording.
        title: t('grok.provider.officialAccountHint'),
      }];
      // Notes are editable in both modes and the bespoke card always showed
      // them on this line; the switching hint only replaces the connection
      // facts (endpoint/model/protocol/key), not the user's own text.
      if (provider.notes) {
        entries.push({ kind: 'text', value: provider.notes });
      }
      return entries;
    }
    const entries: ProviderCardMetaEntry[] = [];
    if (baseUrl) {
      entries.push({ kind: 'code', value: baseUrl });
    }
    if (displayModelName) {
      entries.push({ kind: 'tag', value: displayModelName, color: 'blue' });
    }
    if (apiFormatLabel) {
      entries.push({ kind: 'tag', value: apiFormatLabel, color: 'purple' });
    }
    if (maskedApiKey) {
      entries.push({ kind: 'text', value: `API Key: ${maskedApiKey}` });
    }
    if (provider.notes) {
      entries.push({ kind: 'text', value: provider.notes });
    }
    return entries;
  }, [apiFormatLabel, baseUrl, displayModelName, isOfficialProvider, maskedApiKey, provider.notes, t]);

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
    </>
  );

  const modelSection: ProviderCardModels | undefined = showModelList
    ? {
        models: modelListItems,
        // The catalog is edited, not reordered: `defaultModelKey` points at a
        // row, so row order carries no meaning in `settingsConfig`.
        modelsDraggable: false,
        modelSelectionMode,
        selectedModelIds,
        onToggleModelSelection: onToggleModelSelection
          ? (modelId, isSelected) => onToggleModelSelection(provider, modelId, isSelected)
          : undefined,
        onToggleBatchDeleteMode: onToggleBatchDeleteMode
          ? () => onToggleBatchDeleteMode(provider)
          : undefined,
        onBatchDeleteModels: onBatchDeleteModels
          ? () => onBatchDeleteModels(provider)
          : undefined,
        onTestModels: () => onTest(provider),
        testModelsDisabled: !canRunConnectivityTest,
        testModelsDisabledTooltip: isOfficialProvider
          ? t('grok.provider.officialConnectivityHint')
          : t('common.modelMissing'),
        onFetchModels: onFetchModels ? () => onFetchModels(provider) : undefined,
        onAddModel: onAddModel ? () => onAddModel(provider) : undefined,
        onEditModel: onEditModel ? (modelId) => onEditModel(provider, modelId) : undefined,
        onDeleteModel: onDeleteModel ? (modelId) => onDeleteModel(provider, modelId) : undefined,
        onSetPrimaryModel: onSetDefaultModel ? (modelId) => onSetDefaultModel(provider, modelId) : undefined,
      }
    : undefined;

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
      connectivityStatus,
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
      onShare: onShare ? () => onShare(provider) : undefined,
      onDelete: isLocalProvider ? undefined : () => onDelete(provider),
      enabledStateLabel: provider.isDisabled
        ? t('grok.configDisabled')
        : t('grok.configEnabled'),
      gatewayActions,
      primaryAction: showDirectApplyAction
        ? {
            label: t('grok.provider.apply'),
            icon: <CheckOutlined />,
            onClick: () => onSelect(provider),
            disabled: provider.isDisabled || officialApplyBlocked,
            disabledReason: officialApplyBlocked
              ? t('grok.provider.officialApplyNoAccount')
              : undefined,
          }
        : undefined,
    },
    namePrefix: isOfficialProvider ? <OfficialAccountHeadingIcon /> : undefined,
    nameTags: stateTags,
    metaEntries,
    // No `inlineActions`: this CLI's bespoke card reached connectivity only
    // through the model list's toolbar, and that toolbar is still there. Adding
    // the shared inline button would put a second 「模型测试」 on the same card,
    // which is the duplication the card rules keep out (Codex and Kimi carry
    // both because their toolbars sit inside a collapsed section the header
    // does not repeat; Grok's toolbar is the only entry the user knows).
    footer: renderOfficialAccounts('footer'),
    modelSection,
  };

  if (viewMode !== 'card') {
    return <CodexStyleCard {...props} />;
  }

  /**
   * The cell's second line states one thing: the model the CLI starts with, or
   * — for the official card — the account it is currently using, in all three
   * states (applied → the login on disk → "not signed in"). The line is always
   * reserved by the shared card, so a blank one reads as a broken cell.
   */
  const currentLoginRow = showOfficialRuntimeState
    ? officialAccountRows.find((row) => row.isApplied) ?? officialAccountRows.find((row) => row.isVirtual)
    : undefined;
  const cardSubtitle = isOfficialProvider
    ? [{
        label: t('common.officialAccount.currentLogin'),
        value: currentLoginRow?.label ?? t('common.officialAccount.currentLoginNone'),
      }]
    : displayModelName
      ? [{ label: t('common.model.currentPrimary'), value: displayModelName }]
      : undefined;

  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('grok.provider.baseUrl'),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      {
        key: 'apiFormat',
        label: t('grok.provider.apiFormat'),
        value: apiFormatLabel ?? undefined,
      },
      {
        key: 'apiKey',
        label: t('grok.provider.apiKey'),
        value: apiKey,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
        elideMiddle: true,
      },
    ],
  }];

  if (modelSection) {
    detailSections.push({
      key: 'catalog',
      content: (
        <ProviderModelSection
          providerId={provider.id}
          modelSection={modelSection}
          appearance="detail"
        />
      ),
    });
  }
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
      // One fact plus the pair of primary actions, so the action row rides the
      // second line (Codex's grid cell shape) instead of a footer bar.
      actionPlacement="subtitle"
      providerState={props.providerState}
      actions={props.actions}
      namePrefix={props.namePrefix}
      nameTags={stateTags}
      extraMenuItems={[{
        key: 'connectivity',
        label: t('opencode.connectivity.button'),
        icon: <ApiOutlined />,
        disabled: !canRunConnectivityTest,
        onClick: () => onTest(provider),
      }]}
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
            <Button
              size="small"
              type="text"
              icon={<ApiOutlined />}
              onClick={() => onTest(provider)}
              disabled={!canRunConnectivityTest}
              title={isOfficialProvider ? t('grok.provider.officialConnectivityHint') : undefined}
            >
              {t('opencode.connectivity.button')}
            </Button>
            <Button size="small" type="text" onClick={() => onEdit(provider)}>
              {t('common.edit')}
            </Button>
            <Button size="small" type="text" onClick={() => onCopy(provider)}>
              {t('common.copy')}
            </Button>
            {onShare && (
              <Button size="small" type="text" icon={<Share2 size={14} aria-hidden="true" />} onClick={() => onShare(provider)}>
                {t('common.share')}
              </Button>
            )}
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

export default GrokProviderCard;
