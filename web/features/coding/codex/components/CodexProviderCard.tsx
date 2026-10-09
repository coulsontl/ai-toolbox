import React from 'react';
import {
  Button,
  Tag,
  Typography,
  Tooltip,
  message,
} from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
  LinkOutlined,
  SafetyOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { BarChart2 } from 'lucide-react';
import type {
  CodexCatalogModel,
  CodexOfficialAccount,
  CodexProvider,
  CodexSettingsConfig,
} from '@/types/codex';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { extractCodexBaseUrl, extractCodexModel, extractCodexReasoningEffort } from '@/utils/codexConfigUtils';
import AppliedTag from '@/components/common/AppliedTag';
import ProxyTag from '@/components/common/ProxyTag';
import type { ModelDisplayData } from '@/components/common/ProviderCard/types';
import {
  canApplyProviderWithGatewayProxy,
  getGatewayProviderProfilesVersion,
  isGatewayAggregateMode,
  isGatewayFailoverMode,
  isGatewayProxyMode,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import CodexStyleCard, {
  InlineConnectivityButton,
} from '@/features/coding/shared/providerCardVariants/CodexStyleCard';
import { OfficialAccountsSection } from '@/features/coding/shared/officialAccounts';
import type {
  OfficialAccountPendingAction,
  OfficialAccountRowView,
} from '@/features/coding/shared/officialAccounts';
import type {
  ProviderCardMetaEntry,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import {
  CODEX_LOCAL_PROVIDER_ID,
  isCodexLocalProviderId,
  shouldShowCodexOfficialAccounts,
} from '../utils/localProvider';
import { codexProviderNeedsGatewayProxy } from '../utils/codexGatewayProxyNeed';
import { buildOfficialAccountResetLine } from '../utils/codexQuotaDisplay';
import { codexCatalogRowKey } from '../utils/codexCatalogModels';

const { Text } = Typography;

interface CodexProviderCardProps {
  provider: CodexProvider;
  isApplied: boolean;
  onEdit: (provider: CodexProvider) => void;
  onDelete: (provider: CodexProvider) => void;
  onCopy: (provider: CodexProvider) => void;
  onShare: (provider: CodexProvider) => void;
  onTest: (provider: CodexProvider) => void;
  onSelect: (provider: CodexProvider) => void;
  onToggleDisabled: (provider: CodexProvider, isDisabled: boolean) => void;

  /** Model catalog actions. Custom providers only; official and local bridges
   *  have no persisted catalog. */
  onAddModel?: (provider: CodexProvider) => void;
  onEditModel?: (provider: CodexProvider, modelRowKey: string) => void;
  onCopyModel?: (provider: CodexProvider, modelRowKey: string) => void;
  onDeleteModel?: (provider: CodexProvider, modelRowKey: string) => void;
  onSetPrimaryModel?: (provider: CodexProvider, modelRowKey: string) => void;
  onSetAutoReviewModel?: (provider: CodexProvider, modelRowKey: string) => void;
  onClearAutoReviewModel?: (provider: CodexProvider) => void;
  onFetchModels?: (provider: CodexProvider) => void;
  onReorderModels?: (provider: CodexProvider, orderedModelRowKeys: string[]) => void;
  /** Enter/exit batch-delete selection mode for this provider's model list. */
  onToggleBatchDeleteMode?: (provider: CodexProvider) => void;
  /** Confirm delete of currently selected models (caller shows the confirm dialog). */
  onBatchDeleteModels?: (provider: CodexProvider) => void;
  modelSelectionMode?: boolean;
  selectedModelRowKeys?: string[];
  onToggleModelSelection?: (provider: CodexProvider, modelRowKey: string, selected: boolean) => void;

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
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (checked: boolean) => void;
  /**
   * Hides the drag handle when reordering is not available (non-`custom` sort
   * mode, or an active search) — a handle that cannot move anything is worse
   * than no handle.
   *
   * The page also empties the DndContext sensors in those states, so this is
   * what keeps the two in agreement: without it the grip renders, shows a
   * `grab` cursor, and refuses to move.
   */
  dragDisabled?: boolean;
}

/**
 * A Codex provider, rendered in the Codex style.
 *
 * The layout lives in the shared variant; this file only maps Codex's storage
 * shape onto it, so the card cannot drift from the other CLIs that share the
 * style.
 *
 * Three Codex-specific facts drive the mapping:
 *
 * - The endpoint, the active model and the catalog all live inside the
 *   `settings_config` JSON blob (`config.toml` text + `modelCatalog`), not on
 *   the row, so they are parsed out here.
 * - The model catalog is this CLI's own list, so the card carries a model
 *   section — but the official and `__local__` rows have no persisted catalog
 *   and must not show one.
 * - Official accounts are a Codex concept with their own row actions; they go
 *   through the style's `footer`, which renders below the meta line and above
 *   the model section.
 */
const CodexProviderCard: React.FC<CodexProviderCardProps> = ({
  provider,
  isApplied,
  onEdit,
  onDelete,
  onCopy,
  onShare,
  onTest,
  onSelect,
  onToggleDisabled,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  onSetPrimaryModel,
  onSetAutoReviewModel,
  onClearAutoReviewModel,
  onFetchModels,
  onReorderModels,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  modelSelectionMode = false,
  selectedModelRowKeys = [],
  onToggleModelSelection,
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
  selectable = false,
  selected = false,
  onSelectChange,
  dragDisabled = false,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(true);
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);
  const [switchingGatewayProvider, setSwitchingGatewayProvider] = React.useState(false);

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
  const showModelList = !isOfficialProvider && !isLocalProvider;

  // Catalog rows keep their stored shape; the list only needs display data plus
  // the raw row for row-scoped actions.
  const catalogModels = React.useMemo<CodexCatalogModel[]>(() => {
    const rawModels = settingsConfig.modelCatalog?.models;
    if (!Array.isArray(rawModels)) {
      return [];
    }
    return rawModels.filter((item) => Boolean(item?.model?.trim()));
  }, [settingsConfig.modelCatalog?.models]);
  const codexAutoReviewModelOverride = React.useMemo(
    () => (typeof settingsConfig.autoReviewModelOverride === 'string'
      ? settingsConfig.autoReviewModelOverride.trim()
      : ''),
    [settingsConfig.autoReviewModelOverride],
  );
  const modelRows = React.useMemo(() => {
    const defaultModelId = modelName?.trim() ?? '';
    return catalogModels.map((item) => {
      const upstreamModelId = item.model.trim();
      const rawContextWindow = typeof item.contextWindow === 'number'
        ? item.contextWindow
        : Number.parseInt(String(item.contextWindow ?? '').replace(/[^\d]/g, ''), 10);
      return {
        item,
        // Row identity may differ from the displayed model id: one upstream
        // model can appear several times under different menu names.
        rowKey: codexCatalogRowKey(item),
        display: {
          id: upstreamModelId,
          name: item.displayName?.trim() || upstreamModelId,
          contextLimit: Number.isFinite(rawContextWindow) && rawContextWindow > 0
            ? rawContextWindow
            : undefined,
          isPrimary: Boolean(defaultModelId) && upstreamModelId === defaultModelId,
        } satisfies ModelDisplayData,
      };
    });
  }, [catalogModels, modelName]);

  /**
   * Maps each display object back to the catalog row key it was built from.
   * Identity-based on purpose: rebuilding the key from `display.id`/`display.name`
   * cannot recover `displayName` (it falls back to the upstream id), and the
   * upstream id may itself equal the display name.
   *
   * Only `rowKeyOf` needs this lookup — it receives the display object and
   * nothing else. Handlers that already have a row key (`onEditModel`,
   * `renderModelExtraActions`) get it passed in directly. The `?? model.id`
   * fallback at the call site is unreachable for rows built from `modelRows`;
   * it only keeps the signature total.
   */
  const rowKeyByDisplay = React.useMemo(() => {
    const map = new Map<ModelDisplayData, string>();
    for (const row of modelRows) {
      map.set(row.display, row.rowKey);
    }
    return map;
  }, [modelRows]);

  const canFetchModels = !isOfficialProvider
    && Boolean(apiKey?.trim())
    && Boolean(baseUrl?.trim());
  // `__local__` is a local-file bridge, not a managed applied preset.
  const showRuntimeApplied = isApplied && !isLocalProvider;
  // The protocol check reads the gateway provider profile store, which updates
  // independently of `provider`, so it has to be a dependency — not just a
  // re-render trigger.
  const gatewayProviderProfilesVersion = React.useSyncExternalStore(
    subscribeGatewayProviderProfiles,
    getGatewayProviderProfilesVersion,
    getGatewayProviderProfilesVersion,
  );
  // Official providers and the `__local__` bridge never route through the
  // gateway; everything else is the exact check the Codex page and the aggregate
  // settings panel run, kept in one place.
  const needsGatewayProxy = React.useMemo(
    () => !isOfficialProvider && !isLocalProvider && codexProviderNeedsGatewayProxy(provider),
    [gatewayProviderProfilesVersion, isLocalProvider, isOfficialProvider, provider],
  );
  const restoreDirectUnavailableTitle = t(
    'gateway.proxy.restoreDirectUnavailableHintProtocol',
    { cli: t('settings.gateway.cli.codex') },
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
  // The catalog can hold models even when config.toml declares no model, so the
  // test is enabled whenever there is something to send.
  const canRunConnectivityTest =
    !isOfficialProvider &&
    Boolean(apiKey?.trim()) &&
    (Boolean(modelName?.trim()) || catalogModels.length > 0) &&
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

  const handleToggleDisabled = (checked: boolean) => {
    if (showRuntimeApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !checked);
  };

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

  const handleEngageGatewayProxy = async () => {
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

  const handleApplyWithGatewayProxy = async () => {
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

  const handleRestoreDirect = async () => {
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

  const handleSwitchGatewayProvider = async () => {
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

  /**
   * This CLI's accounts, resolved to what the shared section renders. A mapping
   * rather than JSX: the section knows nothing about `CodexOfficialAccount`, and
   * the quota wording is Codex's business.
   */
  const officialAccountRows = React.useMemo<OfficialAccountRowView[]>(() => {
    // Accounts only ever surface on the official row; a custom provider that
    // somehow carries one keeps the "no accounts" notice instead of a list.
    if (!isOfficialProvider) {
      return [];
    }
    return officialAccounts.map((account) => {
      const metaLines: string[] = [];
      if (account.planType) {
        metaLines.push(account.planType);
      }
      if (!account.lastError) {
        if (account.limit5hText) {
          metaLines.push(
            `${t('codex.provider.officialAccountShortWindowLimitLabel', {
              label: account.limitShortLabel || '5h',
            })}: ${account.limit5hText}`,
          );
        }
        if (account.limitWeeklyText) {
          metaLines.push(
            `${t('codex.provider.officialAccountWeeklyLimitLabel')}: ${account.limitWeeklyText}`,
          );
        }
        if (account.limitMonthlyText) {
          metaLines.push(
            `${t('codex.provider.officialAccountMonthlyLimitLabel')}: ${account.limitMonthlyText}`,
          );
        }
        const resetLine = buildOfficialAccountResetLine(account, t);
        if (resetLine) {
          metaLines.push(resetLine);
        }
      }
      return {
        id: account.id,
        label: formatOfficialAccountLabel(account),
        kindTag: account.isVirtual
          ? t('codex.provider.officialAccountLocalTag')
          : t('codex.provider.officialAccountOauthTag'),
        metaLines,
        // Gateway takeover owns the runtime state; while it is on, neither the
        // "default" mark nor the switch action belongs on screen.
        isApplied: showOfficialRuntimeState && account.isApplied,
        isVirtual: account.isVirtual,
        lastError: account.lastError
          ? t('codex.provider.officialAccountLastError', { message: account.lastError })
          : undefined,
      };
    });
  }, [officialAccounts, isOfficialProvider, showOfficialRuntimeState, t]);

  const officialAccountById = (id: string) =>
    officialAccounts.find((account) => account.id === id);

  /** Row actions address view models; the handlers take this CLI's records. */
  const withOfficialAccount =
    (handler?: (provider: CodexProvider, account: CodexOfficialAccount) => void) =>
    (row: OfficialAccountRowView) => {
      const account = officialAccountById(row.id);
      if (account) {
        handler?.(provider, account);
      }
    };

  const officialAccountPending: OfficialAccountPendingAction | null =
    refreshingOfficialAccountId
      ? { accountId: refreshingOfficialAccountId, action: 'refresh' }
      : savingOfficialAccountId
        ? { accountId: savingOfficialAccountId, action: 'save' }
        : null;

  const renderOfficialAccounts = () => {
    if (!shouldShowCodexOfficialAccounts(provider, officialAccounts.length)) {
      return null;
    }

    return (
      <OfficialAccountsSection
        variant="embedded"
        title={t('codex.provider.officialAccountsTitle')}
        emptyText={t('codex.provider.officialAccountsEmpty')}
        accounts={officialAccountRows}
        collapsed={accountsCollapsed}
        onToggleCollapsed={() => setAccountsCollapsed((current) => !current)}
        pending={officialAccountPending}
        loginAction={
          isOfficialProvider ? (
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
          )
        }
        onRefresh={onOfficialAccountRefresh ? withOfficialAccount(onOfficialAccountRefresh) : undefined}
        onViewDetails={
          onOfficialAccountViewDetails ? withOfficialAccount(onOfficialAccountViewDetails) : undefined
        }
        onSaveLocal={onOfficialLocalAccountSave ? withOfficialAccount(onOfficialLocalAccountSave) : undefined}
        // Hiding the switch while the CLI is on gateway takeover is Codex's
        // rule, and the section only shows it when the handler is supplied.
        onApply={
          showOfficialRuntimeState && onOfficialAccountApply
            ? withOfficialAccount(onOfficialAccountApply)
            : undefined
        }
        onDelete={onOfficialAccountDelete ? withOfficialAccount(onOfficialAccountDelete) : undefined}
      />
    );
  };

  /**
   * The second line: endpoint, active model, masked key and notes, each
   * optional, in whatever order the CLI supplies. This is what the Codex style
   * means by "free-form" — the OpenCode style fixes that line to id/SDK/endpoint.
   */
  const metaEntries = React.useMemo<ProviderCardMetaEntry[]>(() => {
    const entries: ProviderCardMetaEntry[] = [];
    if (baseUrl) {
      entries.push({ kind: 'code', value: baseUrl });
    }
    if (displayModelName) {
      entries.push({ kind: 'tag', value: displayModelName, color: 'blue' });
    }
    if (maskedApiKey) {
      entries.push({ kind: 'text', value: `API Key: ${maskedApiKey}` });
    }
    if (provider.notes) {
      entries.push({ kind: 'text', value: provider.notes });
    }
    return entries;
  }, [baseUrl, displayModelName, maskedApiKey, provider.notes]);

  const nameTags = (
    <>
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
    </>
  );

  const gatewayActions = (
    <>
      {canShowGatewayProxyButton && (
        <Tooltip title={t('gateway.proxy.singleHint')}>
          <Button
            type="link"
            size="small"
            icon={<ApiOutlined />}
            onClick={() => void handleEngageGatewayProxy()}
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
            onClick={() => void handleRestoreDirect()}
            loading={restoringDirect}
          >
            {t('gateway.proxy.restoreDirectButton')}
          </Button>
        </Tooltip>
      )}
      {canShowRestoreDirectUnavailable && (
        <Tooltip title={restoreDirectUnavailableTitle}>
          <Button
            type="link"
            size="small"
            disabled
          >
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
              onClick={() => void handleApplyWithGatewayProxy()}
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
            onClick={() => void handleSwitchGatewayProvider()}
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
    </>
  );

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.id,
      name: provider.name,
      baseUrl,
    },
    providerState: {
      isDisabled: provider.isDisabled,
      // Passed through unchanged: the style card hands it the switch's **new**
      // enabled state, which is exactly what it takes.
      onToggleDisabled: isLocalProvider ? undefined : handleToggleDisabled,
      // Card-level drag handle. The bespoke card registered `useSortable` under
      // `provider.id` and showed the handle whenever batch selection was off;
      // the page empties the DndContext sensors in the same states that set
      // `dragDisabled`, so gating on it keeps the grip and the sensors in
      // agreement — otherwise the grip renders, shows a `grab` cursor, and
      // cannot move anything.
      draggable: !selectable && !dragDisabled,
      sortableId: provider.id,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
      dimmed: provider.isDisabled,
      // A provider can be both applied and the failover P0; CardShell gives the
      // gateway role precedence.
      accent: isGatewayPrimary ? 'gatewayPrimary' : showRuntimeApplied ? 'applied' : undefined,
    },
    actions: {
      onEdit: () => onEdit(provider),
      onCopy: () => onCopy(provider),
      onShare: () => onShare(provider),
      // `__local__` is the local-file bridge: it has no delete path.
      onDelete: isLocalProvider ? undefined : () => onDelete(provider),
      enabledStateLabel: provider.isDisabled
        ? t('codex.configDisabled')
        : t('codex.configEnabled'),
      gatewayActions,
      primaryAction: showDirectApplyAction
        ? {
            label: t('codex.provider.apply'),
            icon: <CheckOutlined />,
            onClick: () => onSelect(provider),
            disabled: provider.isDisabled,
          }
        : undefined,
    },
    nameTags,
    metaEntries,
    inlineActions: (
      <>
        <Text type="secondary" style={{ fontSize: 11 }}>|</Text>
        <InlineConnectivityButton
          onClick={() => onTest(provider)}
          disabled={!canRunConnectivityTest}
          tooltip={isOfficialProvider ? t('codex.provider.officialConnectivityHint') : undefined}
        />
      </>
    ),
    footer: renderOfficialAccounts() ?? undefined,
    modelSection: showModelList
      ? {
          models: modelRows.map((row) => row.display),
          rowKeyOf: (model) => rowKeyByDisplay.get(model) ?? model.id,
          // Indent only: the transparent Collapse layers come from
          // `transparentRows`, which ModelListSection applies itself.
          bodyStyle: { paddingLeft: 18, background: 'transparent' },
          modelsDraggable: !modelSelectionMode,
          onReorderModels: onReorderModels
            ? (orderedRowKeys) => onReorderModels(provider, orderedRowKeys)
            : undefined,
          modelSelectionMode,
          selectedModelIds: selectedModelRowKeys,
          onToggleModelSelection: onToggleModelSelection
            ? (rowKey, isSelected) => onToggleModelSelection(provider, rowKey, isSelected)
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
            ? t('codex.provider.officialConnectivityHint')
            : t('common.modelMissing'),
          onFetchModels: onFetchModels ? () => onFetchModels(provider) : undefined,
          fetchDisabled: !canFetchModels,
          fetchDisabledTooltip: t('opencode.provider.completeUrlAndKey'),
          onAddModel: onAddModel ? () => onAddModel(provider) : undefined,
          onEditModel: onEditModel
            ? (rowKey) => onEditModel(provider, rowKey)
            : undefined,
          onCopyModel: onCopyModel
            ? (rowKey) => onCopyModel(provider, rowKey)
            : undefined,
          onDeleteModel: onDeleteModel
            ? (rowKey) => onDeleteModel(provider, rowKey)
            : undefined,
          onSetPrimaryModel: onSetPrimaryModel
            ? (rowKey) => onSetPrimaryModel(provider, rowKey)
            : undefined,
          // The per-row auto-review action and the line above the rows are
          // Codex-only; both go through the model section's own slots.
          renderModelExtraActions: onSetAutoReviewModel
            ? (model, rowKey) => {
                const isAutoReviewRow =
                  Boolean(codexAutoReviewModelOverride) &&
                  model.id.trim() === codexAutoReviewModelOverride;
                return (
                  <Button
                    size="small"
                    type="text"
                    icon={<SafetyOutlined />}
                    disabled={isAutoReviewRow}
                    onClick={() => onSetAutoReviewModel(provider, rowKey)}
                  >
                    {isAutoReviewRow
                      ? t('codex.model.alreadyAutoReview')
                      : t('codex.model.autoReview')}
                  </Button>
                );
              }
            : undefined,
          aboveList: codexAutoReviewModelOverride ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
              <Text type="secondary" style={{ fontSize: 10 }}>
                {t('codex.model.autoReviewCurrent')}: {codexAutoReviewModelOverride}
              </Text>
              {onClearAutoReviewModel && (
                <Button
                  type="link"
                  size="small"
                  style={{ height: 'auto', padding: 0, fontSize: 10 }}
                  onClick={() => onClearAutoReviewModel(provider)}
                >
                  {t('codex.model.clearAutoReview')}
                </Button>
              )}
            </div>
          ) : undefined,
        }
      : undefined,
  };

  return <CodexStyleCard {...props} />;
};

export default CodexProviderCard;
