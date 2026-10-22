import React from 'react';
import { Button, Space, Tag, Tooltip, Typography, message } from 'antd';
import {
  ApiOutlined,
  CheckOutlined,
} from '@ant-design/icons';
import { BarChart2, Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import type { ClaudeDesktopProvider } from '@/types/claudedesktop';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import AppliedTag from '@/components/common/AppliedTag';
import ProxyTag from '@/components/common/ProxyTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import {
  canApplyProviderWithGatewayProxy,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  hasClaudeDesktopRoutingModels,
  isGatewayAggregateMode,
  isGatewayFailoverMode,
  isGatewayProxyMode,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import {
  InlineConnectivityButton,
} from '@/features/coding/shared/providerCardVariants/CodexStyleCard';
import ClaudeStyleCard from '@/features/coding/shared/providerCardVariants/ClaudeStyleCard';
import {
  OfficialAccountHeadingIcon,
  OfficialAccountHint,
} from '@/features/coding/shared/officialAccounts';
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
  /**
   * Whether this row is the fixed official channel (the first
   * `category = "official"` row, which the backend seeds and protects).
   *
   * Not the same as `category === "official"`: legacy duplicates and copies of
   * the official row keep that category but are ordinary deletable records.
   */
  isOfficialChannel?: boolean;
  /** Which layout the page is in; `'card'` renders the compact grid cell. */
  viewMode?: ProviderViewMode;
  onEdit: (provider: ClaudeDesktopProvider) => void;
  onDelete: (provider: ClaudeDesktopProvider) => void;
  onCopy: (provider: ClaudeDesktopProvider) => void;
  onShare?: (provider: ClaudeDesktopProvider) => void;
  onTest: (provider: ClaudeDesktopProvider) => void;
  onSelect: (provider: ClaudeDesktopProvider) => void;
  onToggleDisabled: (provider: ClaudeDesktopProvider, isDisabled: boolean) => void;
  /**
   * Hides the drag handle when reordering is unavailable (non-`custom` sort mode
   * or an active search). Passed on from the page's own gate.
   */
  dragDisabled?: boolean;
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
 * A Claude Desktop provider, rendered in the Claude style.
 *
 * Style choice follows SOP §4.2.1: the CLI owns no model catalog (its four role
 * bindings live in `meta.claudeDesktopModelRoutes`, with an env fallback for
 * rows imported from Claude Code), and there is exactly one applied provider.
 *
 * Everything the card shows is derived here and handed to the shared variant as
 * `ProviderCardVariantProps`. The gateway handlers, the route derivation and the
 * "an applied provider cannot be disabled" guard stay in this file — they are
 * this CLI's semantics, not the card's layout.
 */
const ClaudeDesktopProviderCard: React.FC<ClaudeDesktopProviderCardProps> = ({
  provider,
  isApplied,
  // Defaults to the category check so a caller that renders a single card
  // (fixtures, tests) still gets the official treatment.
  isOfficialChannel: isOfficialChannelProp,
  viewMode = 'list',
  onEdit,
  onDelete,
  onCopy,
  onShare,
  onTest,
  onSelect,
  onToggleDisabled,
  dragDisabled = false,
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
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);
  const [switchingGatewayProvider, setSwitchingGatewayProvider] = React.useState(false);

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
  /**
   * A channel that applies as "restore the 1P login": the form's official mode
   * writes `{"env":{}}`. Mirrors the backend's `official_restore_settings` — an
   * imported row that merely carries the category but still has its own base
   * URL / token goes through the direct apply path, so it must keep stating
   * its connection facts instead of the official explanation.
   */
  const isOfficialRestoreChannel =
    isOfficialProvider && !configuredBaseUrl && !configuredApiKey;
  /** The fixed official channel; every other official-category row is ordinary. */
  const isOfficialChannel = isOfficialChannelProp ?? isOfficialProvider;
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
      hasClaudeDesktopRoutingModels(effectiveRoutes));
  const gatewayCanApplyProxy = canApplyProviderWithGatewayProxy(gatewayStatus);
  const gatewayMode = gatewayStatus?.mode ?? null;
  const gatewayFailoverActive = isGatewayFailoverMode(gatewayMode);
  const gatewayAggregateActive = isGatewayAggregateMode(gatewayMode);
  const gatewayProxyActive = isGatewayProxyMode(gatewayMode);
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
    // Aggregate has no single primary to switch; its site list is edited in the
    // gateway settings aggregate block, so hide the P0-style switch action.
    !gatewayAggregateActive &&
    !isApplied &&
    !provider.isDisabled &&
    !isOfficialProvider;
  const canRunConnectivityTest =
    !isOfficialProvider &&
    Boolean(configuredApiKey) &&
    configuredModelIds.length > 0 &&
    Boolean(configuredBaseUrl);

  /**
   * The shared switch hands over the **new enabled state**, so the inversion
   * happens exactly once, here — the record stores `isDisabled`.
   */
  const handleToggleDisabled = (enabled: boolean) => {
    if (showRuntimeApplied && !enabled) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !enabled);
  };

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

  const showApplyAction = !gatewayProxyActive && !isApplied;
  const showApplyWithProxyAction = showApplyAction && needsGatewayProxy;
  const showDirectApplyAction = showApplyAction && !needsGatewayProxy;
  const showGatewaySwitchAction = canSwitchGatewayProvider;
  const showGatewayLockedApply = gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;
  const applyWithProxyDisabled = provider.isDisabled || !gatewayCanApplyProxy;
  const showProxyTag = showRuntimeApplied && gatewayProxyActive;
  const hasConfiguredModels = Boolean(effectiveRoutes && Object.keys(effectiveRoutes).length > 0);

  /** The role bindings, in the fixed order the form edits them. */
  const roleBindings = React.useMemo(
    () =>
      CLAUDE_DESKTOP_ROLE_ROUTE_ORDER.flatMap((role) => {
        const route = effectiveRoutes?.[CLAUDE_DESKTOP_ROLE_ROUTE_IDS[role]];
        if (!route?.model) {
          return [];
        }
        return [{
          role,
          label: role.charAt(0).toUpperCase() + role.slice(1),
          model: route.labelOverride || route.model,
          tierAlias: route.tierAlias,
        }];
      }),
    [effectiveRoutes],
  );

  const stateTags = (
    <>
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
    </>
  );

  /**
   * The second line: one `Role: model` binding per configured route, with the
   * tier alias as a tag beside its own route.
   *
   * The endpoint is the first entry, the same place Claude Code puts it — on
   * this card it used to sit on the name row, which the shared style has no
   * slot for and which read as a second title.
   */
  const metaEntries = React.useMemo<ProviderCardMetaEntry[]>(() => {
    const entries: ProviderCardMetaEntry[] = [];
    // An official-restore channel has no endpoint/key/bindings to state: its
    // line is the explanation of what applying it does (the same substitution
    // Codex, Kimi, Gemini and Grok make). Claude Desktop has no account list of
    // its own — the 1P login belongs to the app — so the sentence names that
    // explicitly instead of implying a list this module cannot fill.
    if (isOfficialRestoreChannel) {
      entries.push({
        kind: 'text',
        value: <OfficialAccountHint>{t('claudedesktop.provider.officialAccountHint')}</OfficialAccountHint>,
        // A node value cannot double as its own hover text; the row never
        // wraps, so the sentence stays readable through this.
        title: t('claudedesktop.provider.officialAccountHint'),
      });
      if (provider.notes) {
        entries.push({ kind: 'text', value: provider.notes });
      }
      return entries;
    }
    if (configuredBaseUrl) {
      entries.push({ kind: 'code', value: configuredBaseUrl });
    }
    for (const binding of roleBindings) {
      entries.push({ kind: 'code', label: `${binding.label}:`, value: binding.model });
      if (binding.tierAlias) {
        entries.push({
          kind: 'tag',
          label: `${t('claudecode.model.tierAliasLabel')}:`,
          value: binding.tierAlias,
        });
      }
    }
    // With no role bindings the notes ride this line (as they did on the
    // bespoke card); with bindings they move to the footer so the line stays
    // about the bindings. Same split Claude Code uses.
    if (!hasConfiguredModels && provider.notes) {
      entries.push({ kind: 'text', value: provider.notes });
    }
    return entries;
  }, [configuredBaseUrl, hasConfiguredModels, isOfficialRestoreChannel, provider.notes, roleBindings, t]);

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
    </>
  );

  /**
   * The official channel is a fixed list member, named by the app rather than
   * by the user: `🔗 官方账号`, like every other CLI's official card. Its row
   * keeps whatever name the user gave it (the form still edits it), but the
   * heading states what it *is* — a "官方订阅" tag beside a user-chosen name was
   * the same fact said twice (SOP §4.2.9).
   */
  const cardTitle = isOfficialProvider
    ? t('common.officialAccount.headingTitle')
    : provider.name;

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.id,
      name: cardTitle,
      baseUrl: configuredBaseUrl,
    },
    providerState: {
      isDisabled: provider.isDisabled,
      onToggleDisabled: handleToggleDisabled,
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
      // The fixed official channel cannot be deleted: the backend rejects it,
      // and every listing seeds it back. Omitting the action keeps the entry
      // point out of all three surfaces (list menu, grid menu, detail sidebar)
      // rather than showing a delete that can only fail. A copy of it is an
      // ordinary row and keeps its delete.
      onDelete: isOfficialChannel ? undefined : () => onDelete(provider),
      enabledStateLabel: provider.isDisabled
        ? t('claudecode.configDisabled')
        : t('claudecode.configEnabled'),
      gatewayActions,
      primaryAction: showDirectApplyAction
        ? {
            label: t('claudecode.provider.apply'),
            icon: <CheckOutlined />,
            onClick: () => onSelect(provider),
            disabled: provider.isDisabled,
          }
        : undefined,
    },
    namePrefix: isOfficialProvider ? <OfficialAccountHeadingIcon /> : undefined,
    nameTags: stateTags,
    metaEntries,
    // No connectivity action on an official-restore card: the 1P login is not
    // a static endpoint this app can probe, so the button could only ever be
    // disabled. Codex and Kimi omit it there for the same reason — a greyed
    // 「模型测试」 with a reason on hover still reads as a broken feature, and
    // on an otherwise empty line it was the only thing left (2026-10-21).
    inlineActions: isOfficialRestoreChannel ? undefined : (
      <>
        <Text type="secondary" style={{ fontSize: 12 }}>|</Text>
        <InlineConnectivityButton
          onClick={() => onTest(provider)}
          disabled={!canRunConnectivityTest}
        />
      </>
    ),
    footer: provider.notes && hasConfiguredModels ? (
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <Text type="secondary" style={{ fontSize: 12 }}>
          {provider.notes}
        </Text>
      </div>
    ) : undefined,
  };

  if (viewMode !== 'card') {
    return <ClaudeStyleCard {...props} />;
  }

  // An official-restore channel has no endpoint, key or bindings: the sidebar
  // states the one fact it does have instead of three 「未配置」 rows.
  const detailSections: ProviderDetailSection[] = isOfficialRestoreChannel
    ? [{
      key: 'officialLogin',
      title: t('common.provider.connectionTitle'),
      fields: [{
        key: 'officialLogin',
        label: t('claudedesktop.provider.officialLoginLabel'),
        value: t('claudedesktop.provider.officialLoginManaged'),
      }],
    }]
    : [
      {
        key: 'connection',
        title: t('common.provider.connectionTitle'),
        fields: [
          {
            key: 'baseUrl',
            label: t('claudecode.provider.baseUrl'),
            value: configuredBaseUrl,
            emptyText: t('common.provider.notConfigured'),
            copyable: true,
          },
          {
            key: 'apiKey',
            label: t('claudecode.provider.apiKey'),
            value: configuredApiKey,
            emptyText: t('common.provider.notConfigured'),
            copyable: true,
            elideMiddle: true,
          },
        ],
      },
      {
        key: 'roles',
        title: t('claudecode.model.mappingTitle'),
        fields: roleBindings.map((binding) => ({
          key: binding.role,
          label: binding.label,
          value: binding.tierAlias
            ? `${binding.model} · ${t('claudecode.model.tierAliasLabel')}: ${binding.tierAlias}`
            : binding.model,
          emptyText: t('common.provider.notConfigured'),
        })),
      },
    ];

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
      // The cell states one thing, always. A custom provider states its role
      // bindings (one line, truncated); an official-restore channel states
      // that its sign-in belongs to Claude Desktop — there is no account list
      // this module could read, and a blank second line reads as a broken cell
      // (the rule the other official cells follow).
      subtitle={isOfficialRestoreChannel
        ? [{
          label: t('claudedesktop.provider.officialLoginLabel'),
          value: t('claudedesktop.provider.officialLoginManaged'),
        }]
        : roleBindings.map((binding) => ({
          label: binding.label,
          value: binding.model,
        }))}
      providerState={props.providerState}
      actions={props.actions}
      namePrefix={props.namePrefix}
      nameTags={stateTags}
      // Same rule as `inlineActions`: an official-restore card has no
      // probeable endpoint, so its menu does not offer the action at all.
      extraMenuItems={isOfficialRestoreChannel ? undefined : [{
        key: 'connectivity',
        label: t('opencode.connectivity.button'),
        icon: <ApiOutlined />,
        disabled: !canRunConnectivityTest,
        onClick: () => onTest(provider),
      }]}
      detail={{
        title: <ProviderNameLink name={cardTitle} baseUrl={configuredBaseUrl} />,
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
            {!isOfficialRestoreChannel && (
              <Button
                size="small"
                type="text"
                icon={<ApiOutlined />}
                onClick={() => onTest(provider)}
                disabled={!canRunConnectivityTest}
              >
                {t('opencode.connectivity.button')}
              </Button>
            )}
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
            {/* No delete for the fixed official channel — the same
                `actions.onDelete` omission the list and grid menus render from. */}
            {!isOfficialChannel && (
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

export default ClaudeDesktopProviderCard;
