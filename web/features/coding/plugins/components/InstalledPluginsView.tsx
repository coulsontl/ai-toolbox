import React from 'react';
import { App, Checkbox, Dropdown, Input, Modal, Spin, Switch, Tooltip } from 'antd';
import {
  ArrowDownAZ,
  ArrowUpAZ,
  CircleAlert,
  Combine,
  Copy,
  Download,
  Layers,
  Loader2,
  PackageCheck,
  PanelsTopLeft,
  PlugZap,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Search,
  Settings2,
  SquareArrowOutUpRight,
  Store,
  Trash2,
  Users,
  Wrench,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  listPluginTools,
  setPluginEnabled,
  syncPluginToolsMcpServer,
  uninstallPlugin,
  updatePlugins,
  type BunRuntimeStatus,
  type InstalledPluginRecord,
  type MiddlewareStateView,
  type PluginAccountSummary,
  type PluginKind,
  type PluginSettings,
  type PluginToolDto,
  type PluginToolsSummaryDto,
  type PluginUpdateItem,
  type PluginUpdateSummary,
} from '@/services';
import { copyTextToClipboard } from '@/services/clipboardApi';
import {
  formatMicroseconds,
  formatPluginError,
  getPluginKindCategory,
  getPluginPageTitle,
  getPluginSummary,
  isChinaMirrorRegistry,
  isMiddlewarePluginKind,
  isPagePluginKind,
  isProviderPluginKind,
} from '../utils/pluginFormatters';
import PluginIcon from './PluginIcon';
import styles from './InstalledPluginsView.module.less';

type PluginFilter = 'all' | 'provider' | 'middleware' | 'page';
type SortDirection = 'asc' | 'desc';

interface InstalledPluginsViewProps {
  plugins: InstalledPluginRecord[];
  accounts: PluginAccountSummary[];
  middlewareStates: MiddlewareStateView[];
  bunStatus: BunRuntimeStatus | null;
  pluginSettings: PluginSettings | null;
  chinaMirrorBusy: boolean;
  /** Latest known available update per plugin id, shared with the runtime view. */
  updateItems: Record<string, PluginUpdateItem>;
  checkingUpdates: boolean;
  loading: boolean;
  refreshing?: boolean;
  onRefresh?: () => void;
  onCheckUpdates: () => Promise<PluginUpdateSummary>;
  onUpdateSummary: (summary: PluginUpdateSummary) => void;
  onDataChanged: () => Promise<void>;
  onToggleChinaMirror: (enabled: boolean) => void;
  onOpenInstall: () => void;
  onBrowseMarket: () => void;
  onOpenRuntime: () => void;
  onManageAccounts: (providerId?: string) => void;
  onEditOptions: (plugin: InstalledPluginRecord) => void;
  onOpenPage: (pluginId: string, pageId?: string) => void;
}

const InstalledPluginsView: React.FC<InstalledPluginsViewProps> = ({
  plugins,
  accounts,
  middlewareStates,
  bunStatus,
  pluginSettings,
  chinaMirrorBusy,
  updateItems,
  checkingUpdates,
  loading,
  refreshing = false,
  onRefresh,
  onCheckUpdates,
  onUpdateSummary,
  onDataChanged,
  onToggleChinaMirror,
  onOpenInstall,
  onBrowseMarket,
  onOpenRuntime,
  onManageAccounts,
  onEditOptions,
  onOpenPage,
}) => {
  const { t, i18n } = useTranslation();
  const { message } = App.useApp();
  const [filter, setFilter] = React.useState<PluginFilter>('all');
  const [search, setSearch] = React.useState('');
  const [sortDirection, setSortDirection] = React.useState<SortDirection>('asc');
  const [busyId, setBusyId] = React.useState<string | null>(null);
  const [updatingAll, setUpdatingAll] = React.useState(false);
  const [updatingIds, setUpdatingIds] = React.useState<Set<string>>(() => new Set());
  const availableUpdateItems = React.useMemo(
    () => Object.values(updateItems),
    [updateItems],
  );
  const [uninstallTarget, setUninstallTarget] = React.useState<InstalledPluginRecord | null>(null);
  const [purgeData, setPurgeData] = React.useState(false);
  const [uninstallBusy, setUninstallBusy] = React.useState(false);
  const [toolsSummary, setToolsSummary] = React.useState<PluginToolsSummaryDto | null>(null);
  const [syncingTools, setSyncingTools] = React.useState(false);
  const chinaMirror = isChinaMirrorRegistry(pluginSettings?.npmRegistry);

  // Tools are optional decoration for this page: a failing backend command
  // (older build, host not ready) hides the strip instead of blocking the list.
  const loadTools = React.useCallback(async () => {
    try {
      setToolsSummary(await listPluginTools());
    } catch {
      setToolsSummary(null);
    }
  }, []);

  // Every installed-list refresh — first load, manual refresh, enable/disable,
  // uninstall, update, `plugins-updated` — hands down a new `plugins` array, so
  // it doubles as the silent trigger for the tools summary.
  React.useEffect(() => {
    if (loading) {
      return;
    }
    void loadTools();
  }, [loadTools, loading, plugins]);

  const toolsByPlugin = React.useMemo(() => {
    const grouped = new Map<string, PluginToolDto[]>();
    for (const tool of toolsSummary?.tools ?? []) {
      for (const key of [tool.pluginId, tool.pluginSpec]) {
        if (!key) {
          continue;
        }
        const existing = grouped.get(key);
        if (existing) {
          existing.push(tool);
        } else {
          grouped.set(key, [tool]);
        }
      }
    }
    return grouped;
  }, [toolsSummary]);

  const counts = React.useMemo(() => {
    const next = { all: plugins.length, provider: 0, middleware: 0, page: 0 };
    for (const plugin of plugins) {
      const category = getPluginKindCategory(plugin.kind);
      if (isProviderPluginKind(category)) next.provider += 1;
      if (isMiddlewarePluginKind(category)) next.middleware += 1;
      if (isPagePluginKind(category)) next.page += 1;
    }
    return next;
  }, [plugins]);

  const filteredPlugins = React.useMemo(() => {
    const needle = search.trim().toLowerCase();
    const matched = plugins.filter((plugin) => {
      const category = getPluginKindCategory(plugin.kind);
      if (filter === 'provider' && !isProviderPluginKind(category)) {
        return false;
      }
      if (filter === 'middleware' && !isMiddlewarePluginKind(category)) {
        return false;
      }
      if (filter === 'page' && !isPagePluginKind(category)) {
        return false;
      }
      if (!needle) {
        return true;
      }
      const haystack = [
        plugin.displayName,
        plugin.packageName,
        plugin.spec,
        ...Object.values(plugin.summary ?? {}),
      ].join(' ').toLowerCase();
      return haystack.includes(needle);
    });
    return [...matched].sort((left, right) => {
      const leftName = left.displayName || left.packageName || left.spec;
      const rightName = right.displayName || right.packageName || right.spec;
      const order = leftName.localeCompare(rightName, i18n.language, { sensitivity: 'base' });
      return sortDirection === 'asc' ? order : -order;
    });
  }, [filter, i18n.language, plugins, search, sortDirection]);

  const getMiddlewareState = (plugin: InstalledPluginRecord) =>
    middlewareStates.find((state) => state.pluginId === plugin.id);

  const getPluginTools = (plugin: InstalledPluginRecord): PluginToolDto[] =>
    toolsByPlugin.get(plugin.id) ?? toolsByPlugin.get(plugin.spec) ?? [];

  const handleCopyToolsEndpoint = async () => {
    if (!toolsSummary) {
      return;
    }
    try {
      await copyTextToClipboard(toolsSummary.endpointUrl);
      message.success(t('plugins.installed.tools.copySuccess'));
    } catch (error) {
      message.error(
        t('plugins.installed.tools.copyFailed', { error: formatPluginError(error) }),
      );
    }
  };

  const handleSyncTools = async () => {
    setSyncingTools(true);
    try {
      await syncPluginToolsMcpServer();
      message.success(t('plugins.installed.tools.syncSuccess'));
      await loadTools();
    } catch (error) {
      message.error(
        t('plugins.installed.tools.syncFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setSyncingTools(false);
    }
  };

  // Tooltip content portals to document.body, outside the --pl-* scope, so it
  // leans on the tooltip's inherited chrome rather than plugin tokens.
  const renderToolsTooltip = (tools: PluginToolDto[]) => {
    const visibleTools = tools.slice(0, 8);
    return (
      <div className={styles.toolsTooltip}>
        <ul className={styles.toolsTooltipList}>
          {visibleTools.map((tool, index) => (
            <li key={`${tool.mcpName}-${index}`} className={styles.toolsTooltipItem}>
              <code className={styles.toolsTooltipName}>{tool.mcpName}</code>
              {tool.description ? (
                <span className={styles.toolsTooltipDesc}>{tool.description}</span>
              ) : null}
            </li>
          ))}
        </ul>
        {tools.length > visibleTools.length ? (
          <span className={styles.toolsTooltipMore}>
            {t('plugins.installed.tools.moreTools', {
              count: tools.length - visibleTools.length,
            })}
          </span>
        ) : null}
      </div>
    );
  };

  const handleToggle = async (plugin: InstalledPluginRecord, enabled: boolean) => {
    setBusyId(plugin.id);
    try {
      await setPluginEnabled(plugin.id, enabled);
      await onDataChanged();
      message.success(
        t('plugins.installed.toggleSuccess', {
          name: plugin.displayName || plugin.packageName,
        }),
      );
    } catch (error) {
      message.error(
        t('plugins.installed.actionFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setBusyId(null);
    }
  };

  const reportCheckSummary = (summary: PluginUpdateSummary) => {
    const firstFailure = summary.items.find((item) => item.status === 'failed');
    if (summary.availableCount > 0 && summary.failedCount > 0) {
      message.warning(
        t('plugins.installed.notice.checkUpdatesPartial', {
          available: summary.availableCount,
          failed: summary.failedCount,
        }),
      );
      return;
    }
    if (summary.availableCount > 0) {
      message.info(
        t('plugins.installed.notice.updatesAvailable', { count: summary.availableCount }),
      );
      return;
    }
    if (summary.failedCount > 0) {
      message.error(
        t('plugins.installed.notice.checkUpdatesFailed', {
          error: firstFailure?.message || t('common.unknown'),
        }),
      );
      return;
    }
    message.success(t('plugins.installed.notice.upToDate'));
  };

  const reportUpdateSummary = (summary: PluginUpdateSummary, all: boolean) => {
    const firstFailure = summary.items.find((item) => item.status === 'failed');
    if (summary.updatedCount > 0 && summary.failedCount > 0) {
      message.warning(
        t('plugins.installed.notice.updatePartial', {
          updated: summary.updatedCount,
          failed: summary.failedCount,
        }),
      );
      return;
    }
    if (summary.updatedCount > 0) {
      message.success(
        t(
          all
            ? 'plugins.installed.notice.updateAllSuccess'
            : 'plugins.installed.notice.updateSuccess',
          { count: summary.updatedCount },
        ),
      );
      return;
    }
    if (summary.failedCount > 0) {
      message.error(
        t('plugins.installed.notice.updateFailed', {
          error: firstFailure?.message || t('common.unknown'),
        }),
      );
      return;
    }
    message.success(t('plugins.installed.notice.upToDate'));
  };

  const handleCheckUpdates = async () => {
    try {
      const summary = await onCheckUpdates();
      reportCheckSummary(summary);
    } catch (error) {
      message.error(
        t('plugins.installed.notice.checkUpdatesFailed', {
          error: formatPluginError(error),
        }),
      );
    }
  };

  const handleUpdate = async (plugin?: InstalledPluginRecord) => {
    const all = !plugin;
    if (all) {
      setUpdatingAll(true);
    } else {
      setUpdatingIds((current) => new Set(current).add(plugin.id));
    }
    try {
      const summary = await updatePlugins(plugin?.id);
      onUpdateSummary(summary);
      await onDataChanged();
      reportUpdateSummary(summary, all);
    } catch (error) {
      message.error(
        t('plugins.installed.notice.updateFailed', {
          error: formatPluginError(error),
        }),
      );
    } finally {
      if (all) {
        setUpdatingAll(false);
      } else {
        setUpdatingIds((current) => {
          const next = new Set(current);
          next.delete(plugin.id);
          return next;
        });
      }
    }
  };

  const openUninstallConfirm = (plugin: InstalledPluginRecord) => {
    setUninstallTarget(plugin);
    setPurgeData(false);
  };

  const closeUninstallConfirm = () => {
    if (uninstallBusy) {
      return;
    }
    setUninstallTarget(null);
    setPurgeData(false);
  };

  const handleUninstall = async () => {
    if (!uninstallTarget) {
      return;
    }
    const target = uninstallTarget;
    setUninstallBusy(true);
    try {
      await uninstallPlugin(target.id, { purgeData });
      await onDataChanged();
      message.success(
        t('plugins.installed.uninstallSuccess', {
          name: target.displayName || target.packageName,
        }),
      );
      setUninstallTarget(null);
      setPurgeData(false);
    } catch (error) {
      message.error(
        t('plugins.installed.actionFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setUninstallBusy(false);
    }
  };

  const kindLabel = (category: PluginKind) => {
    if (category === 'middleware') {
      return t('plugins.kind.middleware');
    }
    if (category === 'both') {
      return t('plugins.kind.both');
    }
    if (category === 'page') {
      return t('plugins.kind.page');
    }
    if (category === 'unknown') {
      return t('plugins.kind.unknown');
    }
    return t('plugins.kind.provider');
  };

  const kindIcon = (category: PluginKind) => {
    if (category === 'middleware') {
      return <Layers size={12} aria-hidden="true" />;
    }
    if (category === 'both') {
      return <Combine size={12} aria-hidden="true" />;
    }
    if (category === 'page') {
      return <PanelsTopLeft size={12} aria-hidden="true" />;
    }
    if (category === 'unknown') {
      return <PackageCheck size={12} aria-hidden="true" />;
    }
    return <PlugZap size={12} aria-hidden="true" />;
  };

  const sourceLabel = (sourceType: string) => {
    if (sourceType === 'github') return t('plugins.source.github');
    if (sourceType === 'local') return t('plugins.source.local');
    return t('plugins.source.npm');
  };

  // Literal t() calls per known capability: the i18n scanner only sees static
  // keys, so a template-built key would be reported as missing.
  const unsupportedCapabilityLabel = (feature: string) => {
    if (feature === 'hooks') return t('plugins.installed.capabilities.hooks');
    if (feature === 'panels') return t('plugins.installed.capabilities.panels');
    if (feature === 'actions') return t('plugins.installed.capabilities.actions');
    if (feature === 'replaces') return t('plugins.installed.capabilities.replaces');
    if (feature === 'agent') return t('plugins.installed.capabilities.agent');
    if (feature === 'privacyModels') {
      return t('plugins.installed.capabilities.privacyModels');
    }
    if (feature === 'search') return t('plugins.installed.capabilities.search');
    return feature;
  };

  const filterLabels: Record<PluginFilter, string> = {
    all: t('plugins.installed.filter.all'),
    provider: t('plugins.installed.filter.provider'),
    middleware: t('plugins.installed.filter.middleware'),
    page: t('plugins.installed.filter.page'),
  };

  if (loading && plugins.length === 0) {
    return (
      <div className={styles.loading}>
        <Spin size="small" />
        <span>{t('common.loading')}</span>
      </div>
    );
  }

  if (plugins.length === 0) {
    return (
      <div className={styles.empty}>
        <span className={styles.emptyIcon}>
          <PackageCheck size={20} aria-hidden="true" />
        </span>
        <div className={styles.emptyTitle}>{t('plugins.installed.emptyTitle')}</div>
        <p className={styles.emptyDesc}>{t('plugins.installed.emptyDesc')}</p>
        <div className={styles.emptyActions}>
          <button
            type="button"
            className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
            onClick={onBrowseMarket}
          >
            <Store size={15} aria-hidden="true" />
            <span>{t('plugins.installed.browseMarket')}</span>
          </button>
          <button type="button" className={styles.actionButton} onClick={onOpenInstall}>
            <PackageCheck size={15} aria-hidden="true" />
            <span>{t('plugins.installed.manualInstall')}</span>
          </button>
          {onRefresh ? (
            <button
              type="button"
              className={styles.actionButton}
              disabled={refreshing}
              onClick={onRefresh}
            >
              <RefreshCw
                size={14}
                className={refreshing ? styles.spin : undefined}
                aria-hidden="true"
              />
              <span>{t('common.refresh')}</span>
            </button>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <div className={styles.segmented}>
          {(['all', 'provider', 'middleware', 'page'] as PluginFilter[]).map((key) => (
            <button
              key={key}
              type="button"
              className={`${styles.segmentButton} ${filter === key ? styles.segmentButtonActive : ''}`}
              onClick={() => setFilter(key)}
            >
              <span>{filterLabels[key]}</span>
              <span className={styles.segmentCount}>{counts[key]}</span>
            </button>
          ))}
        </div>
        <div className={styles.toolbarActions}>
          <label className={styles.mirrorToggle}>
            <span>{t('plugins.installed.chinaMirror')}</span>
            <Switch
              size="small"
              checked={chinaMirror}
              loading={chinaMirrorBusy}
              disabled={chinaMirrorBusy || !pluginSettings}
              aria-label={t('plugins.installed.chinaMirror')}
              onChange={onToggleChinaMirror}
            />
          </label>
          <Input
            allowClear
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            prefix={<Search size={14} aria-hidden="true" />}
            placeholder={t('plugins.installed.searchPlaceholder')}
            className={styles.searchInput}
          />
          <Tooltip
            title={
              sortDirection === 'asc'
                ? t('plugins.installed.sortDesc')
                : t('plugins.installed.sortAsc')
            }
          >
            <button
              type="button"
              className={styles.iconButton}
              aria-label={
                sortDirection === 'asc'
                  ? t('plugins.installed.sortDesc')
                  : t('plugins.installed.sortAsc')
              }
              onClick={() =>
                setSortDirection((current) => (current === 'asc' ? 'desc' : 'asc'))}
            >
              {sortDirection === 'asc' ? (
                <ArrowDownAZ size={15} aria-hidden="true" />
              ) : (
                <ArrowUpAZ size={15} aria-hidden="true" />
              )}
            </button>
          </Tooltip>
          <Tooltip title={t('plugins.installed.refresh')}>
            <button
              type="button"
              className={styles.iconButton}
              aria-label={t('plugins.installed.refresh')}
              disabled={refreshing}
              onClick={onRefresh}
            >
              <RefreshCw
                size={15}
                className={refreshing ? styles.spin : undefined}
                aria-hidden="true"
              />
            </button>
          </Tooltip>
          <button
            type="button"
            className={styles.actionButton}
            disabled={checkingUpdates || refreshing || updatingAll}
            onClick={() => void handleCheckUpdates()}
          >
            <RefreshCw
              size={14}
              className={checkingUpdates ? styles.spin : undefined}
              aria-hidden="true"
            />
            <span>{t('plugins.installed.checkUpdates')}</span>
          </button>
          {availableUpdateItems.length > 1 ? (
            <button
              type="button"
              className={styles.actionButton}
              disabled={checkingUpdates || updatingAll || updatingIds.size > 0}
              onClick={() => void handleUpdate()}
            >
              {updatingAll ? (
                <Loader2 size={14} className={styles.spin} aria-hidden="true" />
              ) : (
                <Download size={14} aria-hidden="true" />
              )}
              <span>
                {t('plugins.installed.updateAll', { count: availableUpdateItems.length })}
              </span>
            </button>
          ) : null}
          <button
            type="button"
            className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
            onClick={onOpenInstall}
          >
            <Plus size={14} aria-hidden="true" />
            <span>{t('plugins.installed.install')}</span>
          </button>
        </div>
      </div>

      {toolsSummary && toolsSummary.tools.length > 0 ? (
        <div className={styles.toolsBar}>
          <div className={styles.toolsBarMain}>
            <span className={styles.toolsBarTitle}>
              <Wrench size={14} aria-hidden="true" />
              <span>
                {t('plugins.installed.tools.title', { count: toolsSummary.tools.length })}
              </span>
            </span>
            {toolsSummary.mcpServerId ? (
              <span className={styles.toolsSyncedTag}>
                {t('plugins.installed.tools.synced')}
              </span>
            ) : null}
            <code className={styles.toolsEndpoint} title={toolsSummary.endpointUrl}>
              {toolsSummary.endpointUrl}
            </code>
            {toolsSummary.gatewayRunning ? null : (
              <span className={styles.toolsGatewayHint}>
                <CircleAlert size={11} aria-hidden="true" />
                <span>{t('plugins.installed.tools.gatewayStopped')}</span>
              </span>
            )}
          </div>
          <div className={styles.toolsBarActions}>
            <button
              type="button"
              className={styles.actionButton}
              onClick={() => void handleCopyToolsEndpoint()}
            >
              <Copy size={14} aria-hidden="true" />
              <span>{t('plugins.installed.tools.copyEndpoint')}</span>
            </button>
            <button
              type="button"
              className={styles.actionButton}
              disabled={syncingTools}
              onClick={() => void handleSyncTools()}
            >
              <RefreshCw
                size={14}
                className={syncingTools ? styles.spin : undefined}
                aria-hidden="true"
              />
              <span>
                {toolsSummary.mcpServerId
                  ? t('plugins.installed.tools.resync')
                  : t('plugins.installed.tools.sync')}
              </span>
            </button>
          </div>
        </div>
      ) : null}

      {filteredPlugins.length === 0 ? (
        <div className={styles.empty}>
          <span className={styles.emptyIcon}>
            <Search size={20} aria-hidden="true" />
          </span>
          <div className={styles.emptyTitle}>{t('plugins.installed.filteredEmpty')}</div>
          <p className={styles.emptyDesc}>{t('plugins.installed.filteredEmptyDesc')}</p>
          <div className={styles.emptyActions}>
            <button
              type="button"
              className={styles.actionButton}
              onClick={() => {
                setFilter('all');
                setSearch('');
              }}
            >
              {t('plugins.installed.clearFilters')}
            </button>
          </div>
        </div>
      ) : (
        <div className={styles.list}>
          {filteredPlugins.map((plugin) => {
            const category = getPluginKindCategory(plugin.kind);
            const middlewareState = getMiddlewareState(plugin);
            const canManageAccounts = isProviderPluginKind(category) && plugin.hasProviderEntry;
            const busy = busyId === plugin.id;
            const updateItem = updateItems[plugin.id];
            const updating = updatingIds.has(plugin.id);
            const openablePages = plugin.enabled ? plugin.pages ?? [] : [];
            const providerStatuses = plugin.providers.map((provider) => ({
              provider,
              signedIn: accounts.some((account) => account.provider === provider),
            }));
            const unsupportedFeatures = plugin.contributions?.unsupportedFeatures ?? [];
            const pluginTools = getPluginTools(plugin);
            const showSourceTag =
              plugin.sourceType === 'github' || plugin.sourceType === 'local';
            const providerList = plugin.providers.join(
              i18n.language.startsWith('zh') ? '、' : ', ',
            );
            // Middleware and page kinds always render a status item; provider
            // kinds only do when the package declares channels. Everything else
            // falls back to the package summary so the second line is never an
            // empty div.
            const hasKindStatus =
              isMiddlewarePluginKind(category) || isPagePluginKind(category);
            const summaryFallback = providerStatuses.length > 0 || hasKindStatus
              ? ''
              : getPluginSummary(plugin.summary, i18n.language);
            const hasStatusRow =
              pluginTools.length > 0
              || providerStatuses.length > 0
              || hasKindStatus
              || Boolean(summaryFallback);
            const rowCompact = !hasStatusRow && !plugin.lastError;

            return (
              <article
                key={plugin.id}
                className={`${styles.row} ${plugin.enabled ? '' : styles.rowDisabled} ${
                  rowCompact ? styles.rowCompact : ''
                }`}
              >
                <span className={styles.rowIcon}>
                  <PluginIcon icon={plugin.icon} name={plugin.displayName || plugin.packageName} />
                </span>
                <div className={styles.rowMain}>
                  <div className={styles.rowHeading}>
                    <span
                      className={`${styles.rowName} ${plugin.enabled ? '' : styles.rowNameDisabled}`}
                      title={plugin.displayName || plugin.packageName}
                    >
                      {plugin.displayName || plugin.packageName}
                    </span>
                    <span className={styles.rowVersion}>v{plugin.version || '—'}</span>
                    {unsupportedFeatures.length > 0 ? (
                      <Tooltip
                        title={(
                          <div className={styles.unsupportedTooltip}>
                            <span>{t('plugins.installed.unsupportedTooltip')}</span>
                            <ul className={styles.unsupportedFeatureList}>
                              {unsupportedFeatures.map((feature, index) => (
                                <li key={`${feature}-${index}`}>
                                  {unsupportedCapabilityLabel(feature)}
                                </li>
                              ))}
                            </ul>
                          </div>
                        )}
                      >
                        <span className={styles.unsupportedTag} tabIndex={0}>
                          <CircleAlert size={11} aria-hidden="true" />
                          <span>{t('plugins.installed.unsupportedTag')}</span>
                        </span>
                      </Tooltip>
                    ) : null}
                    {updateItem ? (
                      <span className={styles.updateBadge}>
                        {t('plugins.installed.updateAvailable', {
                          version: updateItem.latestVersion,
                        })}
                      </span>
                    ) : null}
                    {showSourceTag ? (
                      <span className={styles.rowSourceTag}>
                        {sourceLabel(plugin.sourceType)}
                      </span>
                    ) : null}
                    {plugin.enabled ? null : (
                      <span className={styles.rowDisabledTag}>
                        {t('plugins.installed.disabledTag')}
                      </span>
                    )}
                    <span className={styles.rowKind}>
                      {kindIcon(category)}
                      <span>{kindLabel(category)}</span>
                    </span>
                    <div className={styles.rowActions}>
                      {updateItem ? (
                        <button
                          type="button"
                          className={`${styles.textButton} ${styles.textButtonAccent}`}
                          disabled={updating || updatingAll}
                          onClick={() => void handleUpdate(plugin)}
                        >
                          {updating ? (
                            <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                          ) : (
                            <Download size={14} aria-hidden="true" />
                          )}
                          <span>{t('plugins.installed.actions.update')}</span>
                        </button>
                      ) : null}
                      {canManageAccounts ? (
                        <button
                          type="button"
                          className={styles.textButton}
                          onClick={() => onManageAccounts(
                            plugin.providers.length === 1 ? plugin.providers[0] : undefined,
                          )}
                        >
                          <Users size={14} aria-hidden="true" />
                          <span>{t('plugins.installed.actions.manageAccounts')}</span>
                        </button>
                      ) : null}
                      {openablePages.length === 1 ? (
                        <button
                          type="button"
                          className={styles.textButton}
                          onClick={() => onOpenPage(plugin.id, openablePages[0].id)}
                        >
                          <SquareArrowOutUpRight size={14} aria-hidden="true" />
                          <span>{t('plugins.installed.actions.openPage')}</span>
                        </button>
                      ) : openablePages.length > 1 ? (
                        <Dropdown
                          trigger={['click']}
                          menu={{
                            items: openablePages.map((page) => ({
                              key: page.id,
                              label: getPluginPageTitle(page, i18n.language),
                            })),
                            onClick: ({ key }) => onOpenPage(plugin.id, key),
                          }}
                        >
                          <button type="button" className={styles.textButton}>
                            <SquareArrowOutUpRight size={14} aria-hidden="true" />
                            <span>{t('plugins.installed.actions.openPage')}</span>
                          </button>
                        </Dropdown>
                      ) : null}
                      <button
                        type="button"
                        className={styles.textButton}
                        onClick={() => onEditOptions(plugin)}
                      >
                        <Settings2 size={14} aria-hidden="true" />
                        <span>{t('plugins.installed.actions.options')}</span>
                      </button>
                      <button
                        type="button"
                        className={styles.textButton}
                        disabled={busy}
                        onClick={() => void handleToggle(plugin, !plugin.enabled)}
                      >
                        {busy ? (
                          <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                        ) : plugin.enabled ? (
                          <PowerOff size={14} aria-hidden="true" />
                        ) : (
                          <Power size={14} aria-hidden="true" />
                        )}
                        <span>
                          {plugin.enabled
                            ? t('plugins.installed.actions.disable')
                            : t('plugins.installed.actions.enable')}
                        </span>
                      </button>
                      <button
                        type="button"
                        className={`${styles.textButton} ${styles.textButtonDanger}`}
                        disabled={busy}
                        onClick={() => openUninstallConfirm(plugin)}
                      >
                        <Trash2 size={14} aria-hidden="true" />
                        <span>{t('plugins.installed.actions.uninstall')}</span>
                      </button>
                    </div>
                  </div>
                  {hasStatusRow ? (
                    <div className={styles.rowStatus}>
                      {pluginTools.length > 0 ? (
                        <Tooltip title={renderToolsTooltip(pluginTools)}>
                          <span className={styles.toolCountTag} tabIndex={0}>
                            {t('plugins.installed.tools.pluginCount', {
                              count: pluginTools.length,
                            })}
                          </span>
                        </Tooltip>
                      ) : null}
                      {providerStatuses.length > 0 ? (
                        <>
                          <span className={styles.rowCarries}>
                            <span className={styles.rowCarriesDot} aria-hidden="true" />
                            {t('plugins.installed.providersCarried', { providers: providerList })}
                          </span>
                          {/* Provider statuses form one flex item so the whole cluster
                              wraps to a new line instead of splitting mid-list. */}
                          <span className={styles.statusGroup}>
                            {providerStatuses.map((status) => (
                              <span key={status.provider} className={styles.statusItem}>
                                <span
                                  className={`${styles.statusDot} ${
                                    status.signedIn ? styles.statusDotOn : ''
                                  }`}
                                  aria-hidden="true"
                                />
                                <span className={styles.statusProvider}>{status.provider}</span>
                                <span className={styles.statusText}>
                                  {status.signedIn
                                    ? t('plugins.installed.providerSignedIn')
                                    : t('plugins.installed.providerSignedOut')}
                                </span>
                              </span>
                            ))}
                          </span>
                        </>
                      ) : null}
                      {isMiddlewarePluginKind(category) ? (
                        middlewareState ? (
                          <span className={styles.statusItem}>
                            {t('plugins.installed.middlewareSummary', {
                              hooks: middlewareState.hooks.length > 0
                                ? middlewareState.hooks.join(' · ')
                                : t('plugins.runtime.hooksEmpty'),
                              calls: middlewareState.calls,
                              avg: formatMicroseconds(middlewareState.avgMicros),
                            })}
                          </span>
                        ) : (
                          <span className={styles.statusItem}>
                            {t('plugins.installed.metrics.notLoaded')}
                          </span>
                        )
                      ) : null}
                      {isPagePluginKind(category) ? (
                        <span className={styles.statusItem}>
                          {t('plugins.installed.pageSummary', {
                            pages: plugin.pages?.length ?? 0,
                            permissions: plugin.permissions?.length ?? 0,
                          })}
                        </span>
                      ) : null}
                      {summaryFallback ? (
                        <span className={styles.statusSummary} title={summaryFallback}>
                          {summaryFallback}
                        </span>
                      ) : null}
                    </div>
                  ) : null}
                  {plugin.lastError ? (
                    <details className={styles.errorDetails}>
                      <summary>{t('plugins.installed.viewError')}</summary>
                      <p>{plugin.lastError}</p>
                    </details>
                  ) : null}
                </div>
              </article>
            );
          })}
        </div>
      )}

      <div className={styles.statusBar}>
        <span className={styles.statusBarText}>
          {bunStatus?.available
            ? t('plugins.installed.bunReady', {
              version: bunStatus.version ?? bunStatus.expectedVersion,
            })
            : t('plugins.installed.bunMissing')}
        </span>
        <span className={styles.statusBarActions}>
          <button type="button" className={styles.statusBarButton} onClick={onOpenRuntime}>
            {t('plugins.installed.viewRuntime')}
          </button>
          <button
            type="button"
            className={styles.statusBarButton}
            disabled={refreshing}
            onClick={onRefresh}
          >
            {t('plugins.installed.refresh')}
          </button>
        </span>
      </div>

      <Modal
        open={Boolean(uninstallTarget)}
        title={t('plugins.installed.confirmUninstallTitle')}
        width={480}
        okText={t('plugins.installed.actions.uninstall')}
        cancelText={t('common.cancel')}
        confirmLoading={uninstallBusy}
        okButtonProps={{ danger: true, disabled: uninstallBusy }}
        mask={{ closable: !uninstallBusy }}
        onOk={() => void handleUninstall()}
        onCancel={closeUninstallConfirm}
      >
        <div className={styles.uninstallBody}>
          <p className={styles.uninstallText}>
            {t('plugins.installed.confirmUninstallContent', {
              name: uninstallTarget?.displayName
                || uninstallTarget?.packageName
                || uninstallTarget?.spec
                || '',
            })}
          </p>
          <Checkbox
            checked={purgeData}
            disabled={uninstallBusy}
            onChange={(event) => setPurgeData(event.target.checked)}
          >
            <span className={styles.purgeLabel}>
              {t('plugins.installed.purgeData')}
            </span>
          </Checkbox>
        </div>
      </Modal>
    </div>
  );
};

export default InstalledPluginsView;
