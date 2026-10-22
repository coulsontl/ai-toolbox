import React from 'react';
import { message } from 'antd';
import {
  BarChart2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  ApiOutlined,
  CheckOutlined,
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  ShareAltOutlined,
} from '@ant-design/icons';
import { Button, Space, Tag, Tooltip, Typography } from 'antd';
import { KimiProvider, KIMI_LOCAL_PROVIDER_ID } from '@/types/kimi';
import type { KimiCatalogModel, KimiOfficialAccount } from '@/types/kimi';
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
  parseKimiSettingsConfig,
} from '../utils/settingsConfig';
import { kimiCatalogRowKey } from '../utils/kimiCatalogModels';
import AppliedTag from '@/components/common/AppliedTag';
import ProxyTag from '@/components/common/ProxyTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import type { ModelDisplayData } from '@/components/common/ProviderCard/types';
import {
  canApplyProviderWithGatewayProxy,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  isGatewayAggregateMode,
  isGatewayFailoverMode,
  isGatewayProxyMode,
  openAiApiFormatFromBaseUrl,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import {
  CodexStyleCard,
  InlineConnectivityButton,
  type ProviderCardMetaEntry,
  type ProviderCardModels,
  type ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
import { renderProviderPrimaryAction } from '@/features/coding/shared/providerCardVariants/providerCardActions';
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
  OfficialAccountRowView,
} from '@/features/coding/shared/officialAccounts';

const { Text } = Typography;

/**
 * Name an account row after when it was captured.
 *
 * Kimi's OAuth grants no identity — the CLI's own token record holds only tokens
 * and an expiry, and there is no userinfo endpoint — so a row cannot be labelled
 * with an email the way Codex's rows are. The capture time is the only stable
 * fact about it, and it is enough to tell two rows apart in the list.
 */
function formatAccountTimestamp(value: string): string {
  if (!value) {
    return '';
  }
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return parsed.toLocaleString();
}

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
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
  /** Open the single-model editor for this provider (add or edit). */
  onEditModel?: (provider: KimiProvider, model?: KimiCatalogModel) => void;
  /** Duplicate a catalog row under a free key. */
  onCopyModel?: (provider: KimiProvider, model: KimiCatalogModel) => void;
  /** Point `defaultModelKey` at this row. */
  onSetPrimaryModel?: (provider: KimiProvider, model: KimiCatalogModel) => void;
  onDeleteModel?: (provider: KimiProvider, model: KimiCatalogModel) => void;
  onDeleteModels?: (provider: KimiProvider, models: KimiCatalogModel[]) => void;
  /** Open the fetch-models modal for this provider. */
  onFetchModels?: (provider: KimiProvider) => void;
  /** Whether the provider has enough config for an upstream model fetch. */
  canFetchModels?: boolean;
  /**
   * Hides the drag handle when reordering is not available (non-`custom` sort
   * mode, or an active search) — a handle that cannot move anything is worse
   * than no handle.
   */
  dragDisabled?: boolean;
  /**
   * Accounts of the official channel, shown inside this card. Only the official
   * row renders them — the channel is where a login belongs, and an official
   * account cannot be attached to a relay.
   */
  officialAccounts?: KimiOfficialAccount[];
  onOfficialAccountLogin?: () => void;
  onOfficialAccountApply?: (account: KimiOfficialAccount) => void;
  onOfficialAccountDelete?: (account: KimiOfficialAccount) => void;
  onOfficialAccountSaveLocal?: (account: KimiOfficialAccount) => void;
  loginPending?: boolean;
  applyingOfficialAccountId?: string | null;
  savingOfficialAccountId?: string | null;
  /** `'card'` renders the compact grid cell plus the detail sidebar. */
  viewMode?: ProviderViewMode;
}

/**
 * A Kimi Code provider, rendered in the Codex style.
 *
 * The layout lives in the shared variant; this file only maps Kimi's storage
 * shape onto it, so the card cannot drift from the other CLIs that share the
 * style.
 *
 * Three Kimi-specific facts drive the mapping:
 *
 * - The endpoint, the active model and the catalog all live inside the
 *   `settings_config` JSON blob (`auth` / `providerConfigs` / `modelCatalog`),
 *   not on the row, so they are parsed out here.
 * - The model catalog is this CLI's own list, so the card carries a model
 *   section — but the official and `__local__` rows have no persisted catalog
 *   and must not show one.
 * - Official channels authenticate through OAuth, so there is no static API key
 *   to probe: the connectivity action is disabled for them.
 */
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
  selectDisabledReason,
  onEditModel,
  onCopyModel,
  onSetPrimaryModel,
  onDeleteModel,
  onDeleteModels,
  onFetchModels,
  canFetchModels = false,
  dragDisabled = false,
  officialAccounts = [],
  onOfficialAccountLogin,
  onOfficialAccountApply,
  onOfficialAccountDelete,
  onOfficialAccountSaveLocal,
  loginPending = false,
  applyingOfficialAccountId = null,
  savingOfficialAccountId = null,
  viewMode = 'list',
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);
  const [modelsSelectionMode, setModelsSelectionMode] = React.useState(false);
  const [selectedModelKeys, setSelectedModelKeys] = React.useState<string[]>([]);

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
  /**
   * The stored key, verbatim.
   *
   * The list card never prints it; the detail sidebar does, and it prints the
   * whole value — a truncated credential is one you cannot check against the
   * console it came from. It is elided in the middle on screen and copyable, the
   * same contract the Codex sidebar's key row has.
   */
  const parsedApiKey = React.useMemo(
    () => parseKimiSettingsConfig(provider.settingsConfig).apiKey,
    [provider.settingsConfig],
  );

  // Catalog rows shown in the model section. `__local__` is a read-only bridge
  // to the on-disk config, so it has no editable catalog of its own.
  const catalogModels = React.useMemo<KimiCatalogModel[]>(
    () => (isLocalProvider ? [] : parseKimiSettingsConfig(provider.settingsConfig).catalogModels),
    [isLocalProvider, provider.settingsConfig],
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
  const gatewayFailoverActive = isGatewayFailoverMode(gatewayMode);
  const gatewayAggregateActive = isGatewayAggregateMode(gatewayMode);
  const gatewayProxyActive = isGatewayProxyMode(gatewayMode);
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
  const showGatewayLockedApply =
    gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;
  const applyWithProxyDisabled = provider.isDisabled || !gatewayCanApplyProxy;

  const refreshTrayAfterGatewayChange = () => {
    void refreshTrayMenu().catch(() => {});
  };

  const handleEngageGatewayProxy = async () => {
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

  const handleApplyWithGatewayProxy = async () => {
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

  const handleRestoreDirect = async () => {
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

  const handleSwitchGatewayProvider = async () => {
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

  const handleToggleDisabled = (enabled: boolean) => {
    if (showRuntimeApplied && !enabled) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    void onToggleDisabled(provider, !enabled);
  };

  const handleToggleModelsSelectionMode = () => {
    setModelsSelectionMode((current) => !current);
    setSelectedModelKeys([]);
  };

  const handleToggleModelSelected = (rowKey: string, checked: boolean) => {
    setSelectedModelKeys((current) =>
      checked ? [...current, rowKey] : current.filter((key) => key !== rowKey),
    );
  };

  const handleBatchDeleteModels = () => {
    if (selectedModelKeys.length === 0) return;
    const selected = catalogModels.filter((model) =>
      selectedModelKeys.includes(kimiCatalogRowKey(model)),
    );
    if (selected.length > 0) {
      onDeleteModels?.(provider, selected);
    }
    setSelectedModelKeys([]);
    setModelsSelectionMode(false);
  };

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
            loading={engagingGatewayProxy}
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
              {t('kimi.provider.apply')}
            </Button>
          </span>
        </Tooltip>
      )}
    </>
  );

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
          ({t('kimi.localConfigHint')})
        </Text>
      )}
      {isOfficialProvider && gatewayTakeoverActive && (
        <Tooltip title={t('gateway.takeover.officialBypassedTooltip')}>
          <Tag color="gold">{t('gateway.takeover.officialBypassedTag')}</Tag>
        </Tooltip>
      )}
      {showRuntimeApplied && (
        <AppliedTag>{t('kimi.provider.applied')}</AppliedTag>
      )}
      {showProxyTag && <ProxyTag>{t('gateway.proxy.proxyTag')}</ProxyTag>}
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
    </>
  );

  // The bespoke card showed the active model and the endpoint unlabelled, each
  // behind a small icon. `metaEntries` has no icon slot, so the entries carry
  // the same two facts as `code` (monospaced, so the model id and the URL read
  // as values rather than prose) without inventing labels the original did not
  // have.
  const metaEntries: ProviderCardMetaEntry[] = [];
  if (isOfficialProvider) {
    // The official card's second line is the account block's explanation: an
    // official channel has no endpoint and no key of its own (it authenticates
    // through the CLI's OAuth login), and the one thing a reader of *this* card
    // needs is what switching accounts does.
    metaEntries.push({
      kind: 'text',
      value: <OfficialAccountHint>{t('kimi.officialAccount.hint')}</OfficialAccountHint>,
    });
  } else {
    if (isLocalProvider) {
      metaEntries.push({ kind: 'text', value: `(${t('kimi.localConfigHint')})` });
    } else {
      if (modelName) {
        metaEntries.push({ kind: 'code', value: modelName });
      }
      if (baseUrl) {
        metaEntries.push({ kind: 'code', value: baseUrl });
      }
    }
    if (provider.notes) {
      metaEntries.push({ kind: 'text', value: provider.notes });
    }
  }

  const showModelList = !isLocalProvider && !isOfficialProvider;

  /**
   * The catalog row's identity is its **key** (the `[models."<key>"]` table
   * name, which is also what `defaultModelKey` points at), but the shared row
   * prints `id` as the parenthesised second fact. So the key travels as the row
   * key and the upstream model id is what `id` carries — the same split the
   * Codex card makes.
   */
  const modelRows: ModelDisplayData[] = catalogModels.map((model) => ({
    id: model.model,
    name: model.displayName || model.key,
    contextLimit: model.maxContextSize,
    isPrimary: Boolean(modelName) && modelName === model.key,
  }));

  const rowKeyByDisplay = new Map<ModelDisplayData, string>(
    modelRows.map((row, index) => [row, kimiCatalogRowKey(catalogModels[index])]),
  );

  /**
   * The official channel's accounts, as the shared section renders them.
   *
   * The label is the platform nickname (`/me`), falling back to the email and
   * then to when the login was captured — a row the platform could not identify
   * still has to be distinguishable from its neighbours.
   */
  const officialAccountRows = React.useMemo<OfficialAccountRowView[]>(
    () =>
      officialAccounts.map((account) => ({
        id: account.id,
        label:
          account.nickname
          || account.email
          || t('kimi.officialAccount.rowLabel', {
            time: formatAccountTimestamp(account.createdAt),
          }),
        kindTag: account.isVirtual
          ? t('kimi.officialAccount.currentTag')
          : t('kimi.officialAccount.savedTag'),
        // The plan name (`Free`, `Vivace`, …) is what the platform reports for
        // the account; it is the one piece of quota context Kimi can show
        // without a billing call.
        metaLines: account.planType ? [account.planType] : undefined,
        isApplied: account.isApplied,
        isVirtual: Boolean(account.isVirtual),
        lastError: account.lastError ?? undefined,
      })),
    [officialAccounts, t],
  );

  /**
   * The login the official channel is running on.
   *
   * The applied row wins: it is the login the channel actually runs on. When the
   * channel holds no applied account, the CLI's own credential file can still
   * hold a login the app never saved — that is the `当前` row the block draws
   * (`isVirtual`), and stating it is reading that file, not claiming a runtime.
   * When neither exists the channel has no login at all, and the cell says so
   * rather than leaving its second line blank, which reads as a broken card
   * (user-reported 2026-10-14, on a channel that was simply idle).
   *
   * Read off the mapped rows rather than off the raw records, so the value the
   * cell states is the very label the account's own row carries — the grid cell
   * speaks for that row, and one account cannot be named two ways. Kimi's rows
   * carry no gateway gate (unlike Codex's): a gateway takeover leaves the channel's
   * login in place and says so with the 绕过 tag beside it, so the cell keeps
   * naming the account it names.
   */
  const currentLoginRow =
    officialAccountRows.find((row) => row.isApplied)
    ?? officialAccountRows.find((row) => row.isVirtual);

  const officialAccountById = (id: string) =>
    officialAccounts.find((account) => account.id === id);

  const officialAccountPending =
    savingOfficialAccountId !== null
      ? { accountId: savingOfficialAccountId, action: 'save' as const }
      : applyingOfficialAccountId !== null
        ? { accountId: applyingOfficialAccountId, action: 'apply' as const }
        : null;

  /**
   * The account list collapses, like Codex's and ZCode's.
   *
   * It starts **open** here: this card's accounts have always been on screen,
   * and the heading above them is what a reader lands on. Codex starts closed
   * only because its card already carries a model section. The ability, not the
   * starting state, is what the three cards share.
   */
  const [accountsCollapsed, setAccountsCollapsed] = React.useState(false);

  const renderOfficialAccounts = ({ fold }: { fold: boolean }) =>
    isOfficialProvider && onOfficialAccountLogin ? (
      <OfficialAccountsSection
        variant="embedded"
        listTitle={t('common.officialAccount.listTitle')}
        // The explanation is no longer rendered here: it explains the *card*, so
        // it sits on the card's second line. The sign-in entry stays, at the end
        // of the list's title line — the row it adds to.
        loginAction={
          <Button
            type="link"
            size="small"
            icon={<ApiOutlined />}
            onClick={onOfficialAccountLogin}
            loading={loginPending}
            style={{ paddingInline: 0, height: 'auto', fontSize: 12 }}
          >
            {t('kimi.officialAccount.login')}
          </Button>
        }
        applyHint={t('kimi.officialAccount.applyHint')}
        emptyText={t('kimi.officialAccount.empty')}
        accounts={officialAccountRows}
        // `collapsed` / `onToggleCollapsed` are what the shared section reads to
        // decide whether to draw a caret, so withholding them is how the detail
        // sidebar gets the block open. The list card keeps its fold (default
        // open for Kimi — its accounts have always been on screen there).
        {...(fold
          ? {
              collapsed: accountsCollapsed,
              onToggleCollapsed: () => setAccountsCollapsed((current) => !current),
            }
          : {})}
        pending={officialAccountPending}
        actionsDisabled={loginPending}
        onSaveLocal={(row) => {
          const account = officialAccountById(row.id);
          if (account) {
            onOfficialAccountSaveLocal?.(account);
          }
        }}
        onApply={(row) => {
          const account = officialAccountById(row.id);
          if (account) {
            onOfficialAccountApply?.(account);
          }
        }}
        onDelete={(row) => {
          const account = officialAccountById(row.id);
          if (account) {
            onOfficialAccountDelete?.(account);
          }
        }}
      />
    ) : undefined;

  /**
   * The catalog, as data.
   *
   * Hoisted out of the props literal because the card view renders it in a
   * *different place* — the detail sidebar — not because it states something
   * different. Building it once is what keeps the two layouts offering the same
   * catalog with the same actions.
   */
  const modelSection: ProviderCardModels | undefined = showModelList
    ? {
        models: modelRows,
        // Identity-based on purpose: rebuilding the key from `display.id` or
        // `display.name` cannot recover the catalog alias, and the alias is
        // what every callback (`edit` / `copy` / `delete` / `set primary`)
        // resolves the model by.
        rowKeyOf: (model) => rowKeyByDisplay.get(model) ?? model.id,
        modelsDraggable: false,
        modelSelectionMode: modelsSelectionMode,
        selectedModelIds: selectedModelKeys,
        onToggleModelSelection: handleToggleModelSelected,
        onToggleBatchDeleteMode: onDeleteModels ? handleToggleModelsSelectionMode : undefined,
        onBatchDeleteModels: onDeleteModels ? handleBatchDeleteModels : undefined,
        onTestModels: onTest ? () => onTest(provider) : undefined,
        testModelsDisabled: isOfficialProvider || provider.isDisabled,
        testModelsDisabledTooltip: isOfficialProvider
          ? t('kimi.provider.officialConnectivityHint')
          : t('common.modelMissing'),
        onFetchModels: onFetchModels ? () => onFetchModels(provider) : undefined,
        fetchDisabled: !canFetchModels,
        fetchDisabledTooltip: t('opencode.provider.completeUrlAndKey'),
        onAddModel: onEditModel ? () => onEditModel(provider) : undefined,
        onEditModel: onEditModel
          ? (rowKey) => {
              const model = catalogModels.find((item) => kimiCatalogRowKey(item) === rowKey);
              if (model) {
                onEditModel(provider, model);
              }
            }
          : undefined,
        onCopyModel: onCopyModel
          ? (rowKey) => {
              const model = catalogModels.find((item) => kimiCatalogRowKey(item) === rowKey);
              if (model) {
                onCopyModel(provider, model);
              }
            }
          : undefined,
        onSetPrimaryModel: onSetPrimaryModel
          ? (rowKey) => {
              const model = catalogModels.find((item) => kimiCatalogRowKey(item) === rowKey);
              if (model) {
                onSetPrimaryModel(provider, model);
              }
            }
          : undefined,
        onDeleteModel: onDeleteModel
          ? (rowKey) => {
              const model = catalogModels.find((item) => kimiCatalogRowKey(item) === rowKey);
              if (model) {
                onDeleteModel(provider, model);
              }
            }
          : undefined,
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
      // The style card hands this the switch's **new** enabled state, which is
      // what `handleToggleDisabled` takes — it owns the applied-provider guard.
      onToggleDisabled: isLocalProvider ? undefined : handleToggleDisabled,
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
      onCopy: onCopy ? () => onCopy(provider) : undefined,
      onShare: onShare ? () => onShare(provider) : undefined,
      // `__local__` is not a managed preset: it has no delete path.
      onDelete: isLocalProvider ? undefined : () => onDelete(provider),
      enabledStateLabel: provider.isDisabled
        ? t('common.disabled')
        : t('common.enabled'),
      gatewayActions,
      primaryAction: showDirectApplyAction
        ? {
            label: t('kimi.provider.apply'),
            icon: <CheckOutlined />,
            onClick: () => void onApply(provider),
            disabled: provider.isDisabled,
          }
        : undefined,
    },
    namePrefix,
    nameTags,
    metaEntries,
    // The account list belongs to the official channel, so it rides inside the
    // official card rather than in a card of its own. The slot spans the card's
    // full width, which is what keeps its rows aligned with the model section.
    footer: renderOfficialAccounts({ fold: true }),
    // The connectivity probe moved out of the "more" menu: the shared menu is a
    // fixed contract (enable → edit → copy → share → ─── → delete) with no room
    // for a tool-specific item, so it lands on the meta line — where the Codex
    // style already puts it — and on the model-section toolbar below.
    //
    // Except on the official card, whose meta line carries the account block's
    // explanation: a probe sentence and an explanation sentence on one line read
    // as one thought. The probe was disabled there anyway — an official channel
    // authenticates through its OAuth login, not a static key — so only the
    // greyed-out word goes away, not an action.
    inlineActions: isOfficialProvider ? undefined : (
      <>
        <Text type="secondary" style={{ fontSize: 11 }}>|</Text>
        <InlineConnectivityButton
          onClick={() => onTest?.(provider)}
          disabled={provider.isDisabled}
        />
      </>
    ),
    modelSection,
  };

  /**
   * The detail sidebar's content, in the order a reader asks about a provider:
   * where it points, what else it can serve (the catalog), and — for the
   * official channel — the accounts it authenticates with.
   *
   * The default model is not repeated as a field: the catalog below marks its
   * row 当前主模型, and a second statement of one fact is how the two drift.
   */
  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('kimi.providerForm.baseUrl'),
        value: baseUrl ?? '',
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      // No `emptyText`: the official channel has no static key of its own (it
      // authenticates through the CLI's OAuth login), so the row drops rather
      // than rendering a placeholder for something it never had.
      ...(isOfficialProvider ? [] : [{
        key: 'apiKey',
        label: t('kimi.providerForm.apiKey'),
        value: parsedApiKey,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
        elideMiddle: true,
      }]),
    ],
  }];

  // The catalog brings its own header, toolbar and empty state, so this section
  // carries no title of its own. It is conditional because the official and
  // `__local__` rows have none (`showModelList`), and an empty section would
  // still cost its share of the gap between the sections around it.
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

  if (viewMode !== 'card') {
    return <CodexStyleCard {...props} />;
  }

  return (
    <ProviderCompactCard
      provider={{ id: provider.id, name: props.provider.name, baseUrl }}
      // 🔗 官方账号 (n): the count travels in `nameTags` (shared with the list
      // card), the glyph in front of the name here.
      namePrefix={namePrefix}
      // The one model the cell states — the catalog's default. The list card
      // states it too (as a tag on its second line); the cell has no meta line
      // to put it on, so it carries a label instead. `__local__` rows have no
      // model of their own, so they get no line.
      //
      // The official channel states the login it is running on instead: the
      // accounts are what that row is about, and the value is the same label its
      // own row carries inside the block (one account, one way of naming it).
      // It always states one: a channel that holds no login says 未登录 rather
      // than leaving the line blank.
      subtitle={isOfficialProvider
        ? [{
            label: t('common.officialAccount.currentLogin'),
            value: currentLoginRow
              ? currentLoginRow.label
              : t('common.officialAccount.currentLoginNone'),
          }]
        : (!isLocalProvider && modelName
          ? [{ label: t('kimi.providerForm.defaultModelKey'), value: modelName }]
          : undefined)}
      // One model and one action (应用 / 网关), so the action rides the model
      // line rather than a footer bar under it — the same call the Codex cell
      // makes.
      actionPlacement="subtitle"
      providerState={props.providerState}
      actions={props.actions}
      nameTags={nameTags}
      // The list card runs the connectivity probe inline on its meta line; the
      // cell has no meta line, so the diagnostic moves into the menu rather
      // than disappearing. The official channel has no probe anywhere (it
      // authenticates through OAuth, not a static key), so it gets no item.
      extraMenuItems={!isOfficialProvider && onTest ? [{
        key: 'connectivity',
        label: t('opencode.connectivity.button'),
        icon: <ApiOutlined />,
        disabled: provider.isDisabled,
        onClick: () => onTest(provider),
      }] : undefined}
      detail={{
        title: <ProviderNameLink name={props.provider.name} baseUrl={baseUrl} />,
        statusTags: nameTags,
        sections: detailSections,
        primaryActions: (
          <>
            {gatewayActions}
            {renderProviderPrimaryAction(props.actions?.primaryAction)}
          </>
        ),
        secondaryActions: (
          <Space size={4} wrap>
            {onTest && !isOfficialProvider && (
              <Button
                size="small"
                type="text"
                icon={<ApiOutlined />}
                onClick={() => onTest(provider)}
                disabled={provider.isDisabled}
              >
                {t('opencode.connectivity.button')}
              </Button>
            )}
            <Button size="small" type="text" icon={<EditOutlined />} onClick={() => onEdit(provider)}>
              {t('common.edit')}
            </Button>
            {onCopy && (
              <Button size="small" type="text" icon={<CopyOutlined />} onClick={() => onCopy(provider)}>
                {t('common.copy')}
              </Button>
            )}
            {onShare && (
              <Button size="small" type="text" icon={<ShareAltOutlined />} onClick={() => onShare(provider)}>
                {t('common.share')}
              </Button>
            )}
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
};

export default KimiProviderCard;
