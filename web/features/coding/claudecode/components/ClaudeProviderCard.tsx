import React from 'react';
import { Button, Tag, Tooltip, Typography, message } from 'antd';
import { ApiOutlined, CheckOutlined, CodeOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { BarChart2 } from 'lucide-react';
import type { ClaudeCodeProvider } from '@/types/claudecode';
import {
  engageProxyGatewaySingle,
  restoreProxyGatewayCliDirect,
  switchProxyGatewayPrimaryProvider,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { launchClaudeProviderCli } from '@/services/claudeCodeApi';
import { refreshTrayMenu } from '@/services/appApi';
import { open as openDirectoryDialog } from '@tauri-apps/plugin-dialog';
import AppliedTag from '@/components/common/AppliedTag';
import ProxyTag from '@/components/common/ProxyTag';
import {
  canApplyProviderWithGatewayProxy,
  firstGatewayApiFormat,
  getGatewayProviderApiFormatFromMeta,
  getGatewayProviderProfilesVersion,
  isGatewayAggregateMode,
  isGatewayFailoverMode,
  isGatewayProxyMode,
  providerNeedsGatewayProxy,
  subscribeGatewayProviderProfiles,
} from '@/features/coding/shared/gateway';
import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import ClaudeStyleCard from '@/features/coding/shared/providerCardVariants/ClaudeStyleCard';
import type {
  ProviderCardMetaEntry,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import { useSettingsStore } from '@/stores/settingsStore';
import {
  getClaudeConfiguredModelIds,
  getClaudeProviderModelConfig,
  parseClaudeSettingsConfig,
} from '../utils/claudeModelConfig';
import { LOCAL_CONFIG_ID } from '../../shared/localConfig';

const { Text } = Typography;

interface ClaudeProviderCardProps {
  provider: ClaudeCodeProvider;
  isApplied: boolean;
  onEdit: (provider: ClaudeCodeProvider) => void;
  onDelete: (provider: ClaudeCodeProvider) => void;
  onCopy: (provider: ClaudeCodeProvider) => void;
  onShare: (provider: ClaudeCodeProvider) => void;
  onTest: (provider: ClaudeCodeProvider) => void;
  onSelect: (provider: ClaudeCodeProvider) => void;
  onToggleDisabled: (provider: ClaudeCodeProvider, isDisabled: boolean) => void;
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
 * A Claude Code provider, rendered in the Claude style.
 *
 * The layout lives in the shared variant; this file only maps Claude Code's
 * storage shape onto it, so the card cannot drift from the other CLIs that
 * share the style.
 *
 * Three Claude Code-specific facts drive the mapping:
 *
 * - The endpoint and the model bindings live inside the `settings_config` JSON
 *   blob, not on the row, so they are parsed out here.
 * - A provider is either the active one or not (single-active-provider CLI), so
 *   the header carries an 应用 action — unlike the OpenCode style, where the
 *   choice lives on the model row.
 * - The role bindings (default / Haiku / Sonnet / Opus / Fable / Reasoning) are
 *   the card's second line, because this CLI has no model catalog to list.
 */
const ClaudeProviderCard: React.FC<ClaudeProviderCardProps> = ({
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
  dragDisabled = false,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const claudeCliLaunchFullAccess = useSettingsStore((state) => state.claudeCliLaunchFullAccess);
  const preferredTerminal = useSettingsStore((state) => state.preferredTerminal);
  const [engagingGatewayProxy, setEngagingGatewayProxy] = React.useState(false);
  const [restoringDirect, setRestoringDirect] = React.useState(false);
  const [switchingGatewayProvider, setSwitchingGatewayProvider] = React.useState(false);
  const [launchingCli, setLaunchingCli] = React.useState(false);

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
  const isLocalProvider = provider.id === LOCAL_CONFIG_ID;
  // `__local__` is a local-file bridge, not a managed applied preset.
  const showRuntimeApplied = isApplied && !isLocalProvider;
  const settingsConfigApiFormat = settingsConfig as {
    apiFormat?: unknown;
    api_format?: unknown;
  };
  // The protocol check reads the gateway provider profile store, which updates
  // independently of `provider`, so it has to be a dependency — not just a
  // re-render trigger.
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
  // Official providers and the `__local__` bridge never route through the
  // gateway; everything else is the exact check the Claude Code page and the
  // aggregate settings panel run, kept in one place.
  const needsGatewayProxy =
    !isOfficialProvider &&
    !isLocalProvider &&
    providerNeedsGatewayProxy(providerApiFormat, 'anthropic');
  const restoreDirectUnavailableTitle = t(
    'gateway.proxy.restoreDirectUnavailableHintProtocol',
    { cli: t('settings.gateway.cli.claude') },
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
  const requiresExplicitBaseUrl = !isOfficialProvider;
  const canRunConnectivityTest =
    !isOfficialProvider &&
    Boolean(configuredApiKey) &&
    configuredModelIds.length > 0 &&
    (!requiresExplicitBaseUrl || Boolean(configuredBaseUrl));

  const handleToggleDisabled = (checked: boolean) => {
    if (showRuntimeApplied && !checked) {
      message.warning(t('common.disableAppliedConfigWarning'));
      return;
    }
    onToggleDisabled(provider, !checked);  // Switch 的 checked 表示"启用"，所以取反
  };

  const handleLaunchCli = async () => {
    if (provider.isDisabled) {
      return;
    }
    // Let the user pick a project folder to launch the CLI in. Cancelling the
    // picker aborts the launch (KISS: no persistence of the last choice).
    const selectedDir = await openDirectoryDialog({ directory: true, multiple: false });
    if (!selectedDir) {
      return;
    }
    const cwd = typeof selectedDir === 'string' ? selectedDir : selectedDir[0];
    setLaunchingCli(true);
    try {
      await launchClaudeProviderCli(provider.id, {
        fullAccess: claudeCliLaunchFullAccess,
        cwd,
        terminal: preferredTerminal,
      });
      message.success(t('claudecode.provider.launchCliSuccess'));
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      message.error(t('claudecode.provider.launchCliFailed', { error: errorMessage }));
    } finally {
      setLaunchingCli(false);
    }
  };

  const refreshTrayAfterGatewayChange = () => {
    void refreshTrayMenu().catch((error) => {
      console.error('Failed to refresh tray menu after gateway change:', error);
    });
  };

  const handleEngageGatewayProxy = async () => {
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

  const handleApplyWithGatewayProxy = async () => {
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

  const handleRestoreDirect = async () => {
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

  const handleSwitchGatewayProvider = async () => {
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

  /**
   * The second line: the model bindings this CLI writes into `settings.json`.
   *
   * Each binding is a label plus a monospaced model id. The label travels
   * inside its entry rather than as a preceding entry — see
   * `ProviderCardMetaEntry.label` for why the two are not interchangeable.
   */
  const metaEntries = React.useMemo<ProviderCardMetaEntry[]>(() => {
    const entries: ProviderCardMetaEntry[] = [];
    const push = (label: string, model?: string) => {
      if (!model) {
        return;
      }
      entries.push({ kind: 'code', label: `${label}:`, value: model });
    };

    push(t('claudecode.model.defaultLabel'), modelConfig.fallbackModel);
    push('Haiku', modelConfig.roles.haiku.model);
    push('Sonnet', modelConfig.roles.sonnet.model);
    push('Opus', modelConfig.roles.opus.model);
    push('Fable', modelConfig.roles.fable.model);
    push(t('claudecode.model.reasoningLabel'), modelConfig.legacyReasoningModel);

    if (!entries.length && provider.notes) {
      entries.push({ kind: 'text', value: provider.notes });
    }
    return entries;
  }, [modelConfig, provider.notes, t]);

  const hasConfiguredModels =
    Boolean(modelConfig.fallbackModel) ||
    Boolean(
      modelConfig.roles.haiku.model ||
        modelConfig.roles.sonnet.model ||
        modelConfig.roles.opus.model ||
        modelConfig.roles.fable.model ||
        modelConfig.legacyReasoningModel,
    );

  const showProxyTag = showRuntimeApplied && gatewayProxyActive;

  const nameTags = (
    <>
      {isLocalProvider && (
        <Text type="secondary" style={{ fontSize: 11 }}>
          ({t('claudecode.localConfigHint')})
        </Text>
      )}
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
    </>
  );

  const inlineActions = (
    <>
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
      <Button
        type="text"
        size="small"
        icon={<CodeOutlined />}
        onClick={() => {
          void handleLaunchCli();
        }}
        loading={launchingCli}
        disabled={provider.isDisabled}
        title={t('claudecode.provider.launchCliHint')}
        style={{ fontSize: 12, padding: '0 4px', height: 'auto', flexShrink: 0 }}
      >
        {t('claudecode.provider.launchCli')}
      </Button>
    </>
  );

  const showApplyAction = !gatewayProxyActive && !isApplied && !isLocalProvider;
  const showApplyWithProxyAction = showApplyAction && needsGatewayProxy;
  const showDirectApplyAction = showApplyAction && !needsGatewayProxy;
  const showGatewaySwitchAction = canSwitchGatewayProvider;
  const showGatewayLockedApply = gatewayProxyActive && !isApplied && !canSwitchGatewayProvider;
  const applyWithProxyDisabled = provider.isDisabled || !gatewayCanApplyProxy;

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
              {t('claudecode.provider.apply')}
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
      baseUrl: configuredBaseUrl,
    },
    providerState: {
      isDisabled: provider.isDisabled,
      // `__local__` is the current file's bridge record, not a managed preset:
      // there is nothing to switch on or off.
      //
      // `handleToggleDisabled` is passed through unchanged: the style card hands
      // it the switch's **new** enabled state, which is exactly what it takes.
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
      // `__local__` is not a managed preset: it has no delete path.
      onDelete: isLocalProvider ? undefined : () => onDelete(provider),
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
    nameTags,
    metaEntries,
    inlineActions,
    // The notes line only shows on its own when no bindings are configured;
    // otherwise it would repeat inside the binding line.
    footer: provider.notes && hasConfiguredModels ? (
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <Text type="secondary" style={{ fontSize: 12 }}>
          {provider.notes}
        </Text>
      </div>
    ) : undefined,
  };

  return <ClaudeStyleCard {...props} />;
};

export default ClaudeProviderCard;
