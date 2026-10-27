import React from 'react';
import {
  Button,
  Space,
  Tag,
  Typography,
  Tooltip,
  message,
} from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  LinkOutlined,
  SafetyOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { BarChart2, Share2 } from 'lucide-react';
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
import { useCodexDaemonNotice } from '../hooks/useCodexDaemonNotice';
import ProxyTag from '@/components/common/ProxyTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
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
import type {
  ProviderCardMetaEntry,
  ProviderCardModels,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import { renderProviderPrimaryAction } from '@/features/coding/shared/providerCardVariants/providerCardActions';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
// Through the module's barrel, the way the sibling shared modules are consumed
// (`shared/providerList`, `shared/officialAccounts`).
import {
  ProviderCompactCard,
  type ProviderDetailSection,
} from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import {
  CODEX_LOCAL_PROVIDER_ID,
  isCodexLocalProviderId,
  shouldShowCodexOfficialAccounts,
} from '../utils/localProvider';
import { codexProviderNeedsGatewayProxy, resolveCodexProviderApiFormat } from '../utils/codexGatewayProxyNeed';
import { buildOfficialAccountResetLine, formatOfficialAccountCredits } from '../utils/codexQuotaDisplay';
import { formatCodexOfficialAccountError } from '../utils/codexAccountRefresh';
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
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
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
  /**
   * `'card'` renders the compact grid card, and moves everything the list card
   * shows below its header — the model catalog and the official accounts — into
   * the detail sidebar the cell opens.
   *
   * Both branches consume the same computed `providerState` / `actions` /
   * `nameTags` / `modelSection`, so the apply / gateway / catalog behaviour
   * cannot differ between the two layouts.
   */
  viewMode?: ProviderViewMode;
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
  selectDisabledReason,
  dragDisabled = false,
  viewMode = 'list',
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { notifyIfDaemonHoldsStaleConfig } = useCodexDaemonNotice();
  /**
   * The account block's fold, for the list card. The detail sidebar renders the
   * same block open, so nothing reads this on that path — and one piece of state
   * is enough because the two hosts are never on screen together (the card view
   * draws compact cards, which have no footer to hang the block on).
   */
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
  const providerApiFormat = React.useMemo(
    () => resolveCodexProviderApiFormat(provider),
    [gatewayProviderProfilesVersion, provider],
  );
  /**
   * The protocol as a readable name, for the detail sidebar.
   *
   * A per-CLI key map rather than a shared one keyed by template string: the
   * four suffixes are identical in every CLI's namespace, but a dynamic
   * `t(\`\${prefix}.apiFormatX\`)` is invisible to `i18n:check`, which is how
   * the CLI-prefixed locale schemes failed here before. Same shape as
   * `ClaudeProviderCard`'s and `GrokProviderCard`'s maps.
   */
  const apiFormatLabel = React.useMemo(() => {
    if (!providerApiFormat || isOfficialProvider) {
      return null;
    }
    if (providerApiFormat === 'openai_responses') {
      return t('codex.provider.apiFormatOpenAIResponses');
    }
    if (providerApiFormat === 'openai_chat') {
      return t('codex.provider.apiFormatOpenAIChat');
    }
    if (providerApiFormat === 'anthropic_messages') {
      return t('codex.provider.apiFormatAnthropicMessages');
    }
    return t('codex.provider.apiFormatGeminiNative');
  }, [isOfficialProvider, providerApiFormat, t]);
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
      // Takeover rewrites config.toml/auth.json; a running Codex daemon keeps
      // the direct-connect settings until it restarts (issue #443).
      await notifyIfDaemonHoldsStaleConfig();
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
      await notifyIfDaemonHoldsStaleConfig();
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
      // Restoring direct also rewrites config.toml/auth.json back to the
      // provider's own settings; the daemon needs the same restart (issue #443).
      await notifyIfDaemonHoldsStaleConfig();
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
      await notifyIfDaemonHoldsStaleConfig();
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
        const credits = formatOfficialAccountCredits(account, t);
        if (credits) {
          metaLines.push(`${t('codex.provider.officialAccountCreditsLabel')}: ${credits}`);
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
          ? t('codex.provider.officialAccountLastError', {
            message: formatCodexOfficialAccountError(account.lastError, t),
          })
          : undefined,
      };
    });
  }, [officialAccounts, isOfficialProvider, showOfficialRuntimeState, t]);

  /**
   * The login the official channel is running on — what its cell states instead
   * of a primary model.
   *
   * The applied row wins: it is the login the channel actually runs on. When the
   * channel holds no applied account, the CLI's own `auth.json` can still hold a
   * login the app never saved — and that is precisely the `本地` row the block
   * already draws (`isVirtual`), which the app's own wording calls what is in use.
   * Stating it is reading the file, not claiming a runtime: it is the same fact
   * the block's row states one click away. When neither row exists the channel
   * has no login at all, and the cell says so — a silently empty second line
   * reads as a broken card (user-reported 2026-10-14, on two channels that were
   * simply idle).
   *
   * Read off the mapped rows rather than off the raw records, so the value is
   * the very label the account's own row carries: the cell speaks for that row,
   * and one account cannot be named two ways. The whole line is behind
   * `showOfficialRuntimeState`, the gate this CLI already puts on the applied
   * mark: while the gateway owns the runtime, `auth.json` holds *its* credentials
   * rather than this channel's login, so neither row is the channel's current
   * login and the cell says 未登录 like any other idle channel.
   */
  const currentLoginRow = showOfficialRuntimeState
    ? (officialAccountRows.find((row) => row.isApplied)
      ?? officialAccountRows.find((row) => row.isVirtual))
    : undefined;

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

  /** `fold` is what the list card has and the detail sidebar does not. */
  const renderOfficialAccounts = ({ fold }: { fold: boolean }) => {
    if (!shouldShowCodexOfficialAccounts(provider, officialAccounts.length)) {
      return null;
    }

    return (
      <OfficialAccountsSection
        variant="embedded"
        listTitle={t('common.officialAccount.listTitle')}
        // The explanation is not passed here: it explains the *card*, so it sits
        // on the card's second line, and this list keeps the one line it needs —
        // its title, its count, and the sign-in entry it adds rows to. What is
        // left of a non-official card's section is its notice, which stays on
        // the title line, beside the list it is about.
        emptyText={t('codex.provider.officialAccountsEmpty')}
        accounts={officialAccountRows}
        // `collapsed` / `onToggleCollapsed` are what the shared section reads to
        // decide whether to draw a caret, so withholding them is how the detail
        // sidebar gets the block open (user-directed, 2026-10-12). The list card
        // keeps its fold: the card has neighbours to line up with, and the rows
        // are one click away.
        {...(fold
          ? {
              collapsed: accountsCollapsed,
              onToggleCollapsed: () => setAccountsCollapsed((current) => !current),
            }
          : {})}
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
   *
   * The official card is the exception: it has no endpoint and no key (it
   * authenticates through `auth.json`), its active model is already the marked
   * row of the model list below, and the one thing a reader of *this* card
   * needs is what switching accounts does. So the line carries that instead.
   */
  const metaEntries = React.useMemo<ProviderCardMetaEntry[]>(() => {
    if (isOfficialProvider) {
      return [{
        kind: 'text',
        value: <OfficialAccountHint>{t('codex.provider.officialAccountHint')}</OfficialAccountHint>,
      }];
    }
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
  }, [baseUrl, displayModelName, isOfficialProvider, maskedApiKey, provider.notes, t]);

  /**
   * The official card's name row doubles as the heading of its account block:
   * `🔗 官方账号 (2)`. The glyph comes in through the style card's `namePrefix`,
   * the count through `OfficialAccountCount` — the same pair the section itself
   * draws for a standalone host (ZCode), so the three headings cannot drift
   * apart.
   *
   * The count reads the account records rather than the rows memo: the name row
   * is built above that memo, and the records are what both the heading and the
   * list below are counting.
   */
  const namePrefix = isOfficialProvider ? <OfficialAccountHeadingIcon /> : undefined;

  const nameTags = (
    <>
      {isOfficialProvider && <OfficialAccountCount count={officialAccounts.length} />}
      {isLocalProvider && (
        <Text type="secondary" style={{ fontSize: 11 }}>
          ({t('codex.localConfigHint')})
        </Text>
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

  /**
   * The model catalog, as data.
   *
   * Extracted from the props literal because the card view renders it in a
   * *different place* — the detail sidebar — not because it states something
   * different. Building it before the branch is what keeps the two layouts
   * offering the same catalog with the same actions.
   *
   * The official and `__local__` rows have no persisted catalog (`showModelList`
   * is false for them), and a provider whose catalog is empty still gets the
   * section: an empty list with an 添加模型 button is the only way to start one.
   */
  const modelSectionProps: ProviderCardModels | undefined = showModelList
    ? {
        models: modelRows.map((row) => row.display),
        rowKeyOf: (model) => rowKeyByDisplay.get(model) ?? model.id,
        // Nothing but the transparency, which the section's own layers take
        // care of (`transparentRows`). The indent that used to be here belongs
        // to the fold rather than to this data: the list card's rows clear its
        // caret, the detail sidebar's draw no caret and so are not indented.
        // The two hosts render this same object, so the difference cannot live
        // in it (user-directed, 2026-10-13).
        bodyStyle: { background: 'transparent' },
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
    : undefined;

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.id,
      // The official card is named for what it holds, not for the row's own
      // label: every CLI's official channel heads its account block the same
      // way, which is what makes the three cards one style.
      name: isOfficialProvider
        ? t('common.officialAccount.headingTitle')
        : provider.name,
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
      selectDisabledReason,
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
    namePrefix,
    nameTags,
    metaEntries,
    // The probe rides the meta line, except on the official card, whose meta
    // line is the account block's explanation — the two would read as one
    // sentence. It was disabled there anyway.
    inlineActions: isOfficialProvider ? undefined : (
      <>
        <Text type="secondary" style={{ fontSize: 11 }}>|</Text>
        <InlineConnectivityButton
          onClick={() => onTest(provider)}
          disabled={!canRunConnectivityTest}
        />
      </>
    ),
    footer: renderOfficialAccounts({ fold: true }) ?? undefined,
    modelSection: modelSectionProps,
  };

  /**
   * The detail sidebar's content, in the order a reader asks about a provider:
   * where it points, which model runs, what else it can serve (the catalog), who
   * is logged in (official accounts), and the note the user left.
   *
   * Built here rather than inside the panel because the panel is shared with
   * Claude — it knows how to lay a section out, not which facts Codex states.
   */
  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('codex.provider.baseUrl'),
        value: baseUrl ?? '',
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      // No `emptyText`: an official subscription has no protocol of its own, so
      // the row drops instead of rendering a placeholder.
      { key: 'apiFormat', label: t('codex.provider.apiFormat'), value: apiFormatLabel ?? undefined },
      {
        key: 'apiKey',
        label: t('codex.provider.apiKey'),
        value: apiKey ?? '',
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
        elideMiddle: true,
      },
    ],
  }, {
    key: 'models',
    // Not `common.model.title`: that reads 模型列表, which is what the catalog
    // section *below* this one is headed — two identical headings in one
    // sidebar, one of them a single binding. Claude's panel calls its
    // equivalent 模型映射 for the same reason.
    title: t('codex.model.mappingTitle'),
    fields: [
      {
        key: 'primaryModel',
        label: t('codex.model.primaryLabel'),
        value: displayModelName ?? '',
        emptyText: t('common.provider.notConfigured'),
      },
      // Only when one is set: an unset auto-review model is the default, not a
      // missing value worth a row.
      {
        key: 'autoReviewModel',
        label: t('codex.provider.autoReviewModel'),
        value: codexAutoReviewModelOverride || undefined,
      },
    ],
  }];

  // The catalog brings its own header, toolbar and empty state, so this section
  // carries no title of its own. It is conditional because the official and
  // `__local__` rows have none (`showModelList`), and an empty section would
  // still cost its share of the gap between the sections around it.
  if (modelSectionProps) {
    detailSections.push({
      key: 'catalog',
      content: (
        <ProviderModelSection
          providerId={provider.id}
          modelSection={modelSectionProps}
          appearance="detail"
        />
      ),
    });
  }

  const officialAccountsSection = renderOfficialAccounts({ fold: false });
  if (officialAccountsSection) {
    detailSections.push({ key: 'officialAccounts', content: officialAccountsSection });
  }

  if (provider.notes) {
    detailSections.push({
      key: 'notes',
      title: t('common.provider.notes'),
      content: <Text type="secondary" style={{ fontSize: 12 }}>{provider.notes}</Text>,
    });
  }

  if (viewMode === 'card') {
    return (
      <ProviderCompactCard
        provider={{
          id: provider.id,
          // The official card is named for what it holds, not for the row's
          // own label — the same heading the list card draws.
          name: props.provider.name,
          baseUrl: baseUrl ?? undefined,
        }}
        // 🔗 官方账号 (n): the count travels in `nameTags` (built above, and
        // shared with the list card), the glyph in front of the name here.
        namePrefix={namePrefix}
        // One model, not the whole catalog: the cell answers "which model does
        // this provider run", and the catalog is a manager, not a label.
        //
        // The official channel answers a different question — which login it
        // is running on — because that is what the row is about; the accounts
        // are what its card opens. It always answers it: a channel that holds
        // no login says 「未登录」 rather than leaving the line blank.
        subtitle={isOfficialProvider
          ? [{
              label: t('common.officialAccount.currentLogin'),
              value: currentLoginRow
                ? currentLoginRow.label
                : t('common.officialAccount.currentLoginNone'),
            }]
          : (displayModelName
              ? [{ label: t('codex.model.primaryLabel'), value: displayModelName }]
              : undefined)}
        // The cell states one fact and one action, so the action rides that
        // line instead of a footer bar under it — a whole bar for a single
        // button read as mostly empty (user-directed, 2026-10-12).
        actionPlacement="subtitle"
        providerState={props.providerState}
        actions={props.actions}
        nameTags={nameTags}
        // The list card renders 连通性测试 inline on its meta line; the grid
        // cell has room for the two primary actions only, so the diagnostic
        // moves into the menu rather than disappearing.
        extraMenuItems={[{
          key: 'connectivity',
          label: t('opencode.connectivity.button'),
          icon: <ApiOutlined />,
          disabled: !canRunConnectivityTest,
          onClick: () => onTest(provider),
        }]}
        detail={{
        title: <ProviderNameLink name={props.provider.name} baseUrl={baseUrl} />,
        statusTags: nameTags,
        sections: detailSections,
        // The panel renders the same computed actions the grid cell does,
        // through the same renderers — so "the sidebar offers what the card
        // offers" is a fact about the code, not a thing to re-check.
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
              title={isOfficialProvider ? t('codex.provider.officialConnectivityHint') : undefined}
            >
              {t('opencode.connectivity.button')}
            </Button>
            <Button size="small" type="text" icon={<EditOutlined />} onClick={() => onEdit(provider)}>
              {t('common.edit')}
            </Button>
            <Button size="small" type="text" icon={<CopyOutlined />} onClick={() => onCopy(provider)}>
              {t('common.copy')}
            </Button>
            <Button size="small" type="text" icon={<Share2 size={14} aria-hidden="true" />} onClick={() => onShare(provider)}>
              {t('common.share')}
            </Button>
            {!isLocalProvider && (
              <Button size="small" type="text" danger icon={<DeleteOutlined />} onClick={() => onDelete(provider)}>
                {t('common.delete')}
              </Button>
            )}
          </Space>
        ),
      }}
    />
  );
  }

  return <CodexStyleCard {...props} />;
};

export default CodexProviderCard;
