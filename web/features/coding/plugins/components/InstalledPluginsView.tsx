import React from 'react';
import { App, Checkbox, Dropdown, Input, Modal, Spin, Switch } from 'antd';
import {
  PackageCheck,
  RefreshCw,
  Search,
  Settings2,
  SquareArrowOutUpRight,
  Store,
  Trash2,
  Users,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  setPluginEnabled,
  uninstallPlugin,
  type InstalledPluginRecord,
  type MiddlewareStateView,
  type PluginAccountSummary,
} from '@/services';
import {
  formatMicroseconds,
  formatPluginError,
  getPluginKindCategory,
  getPluginPageTitle,
  getPluginSummary,
  isMiddlewarePluginKind,
  isPagePluginKind,
  isProviderPluginKind,
} from '../utils/pluginFormatters';
import PluginIcon from './PluginIcon';
import styles from './InstalledPluginsView.module.less';

type PluginFilter = 'all' | 'provider' | 'middleware' | 'page';

interface InstalledPluginsViewProps {
  plugins: InstalledPluginRecord[];
  accounts: PluginAccountSummary[];
  middlewareStates: MiddlewareStateView[];
  loading: boolean;
  refreshing?: boolean;
  onRefresh?: () => void;
  onDataChanged: () => Promise<void>;
  onOpenInstall: () => void;
  onBrowseMarket: () => void;
  onManageAccounts: (providerId?: string) => void;
  onEditOptions: (plugin: InstalledPluginRecord) => void;
  onOpenPage: (pluginId: string, pageId?: string) => void;
}

const InstalledPluginsView: React.FC<InstalledPluginsViewProps> = ({
  plugins,
  accounts,
  middlewareStates,
  loading,
  refreshing = false,
  onRefresh,
  onDataChanged,
  onOpenInstall,
  onBrowseMarket,
  onManageAccounts,
  onEditOptions,
  onOpenPage,
}) => {
  const { t, i18n } = useTranslation();
  const { message } = App.useApp();
  const [filter, setFilter] = React.useState<PluginFilter>('all');
  const [search, setSearch] = React.useState('');
  const [busyId, setBusyId] = React.useState<string | null>(null);
  const [uninstallTarget, setUninstallTarget] = React.useState<InstalledPluginRecord | null>(null);
  const [purgeData, setPurgeData] = React.useState(false);
  const [uninstallBusy, setUninstallBusy] = React.useState(false);

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
    return plugins.filter((plugin) => {
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
  }, [filter, plugins, search]);

  const getAccountCount = (plugin: InstalledPluginRecord) => {
    const providerIds = new Set(plugin.providers);
    return accounts.filter((account) => providerIds.has(account.provider)).length;
  };

  const getMiddlewareState = (plugin: InstalledPluginRecord) =>
    middlewareStates.find((state) => state.pluginId === plugin.id);

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

  const kindLabel = (plugin: InstalledPluginRecord) => {
    const category = getPluginKindCategory(plugin.kind);
    if (category === 'middleware') {
      return { text: t('plugins.kind.middleware'), className: styles.pillMiddleware };
    }
    if (category === 'both') {
      return { text: t('plugins.kind.both'), className: styles.pillBoth };
    }
    if (category === 'page') {
      return { text: t('plugins.kind.page'), className: styles.pillPage };
    }
    if (category === 'unknown') {
      return { text: t('plugins.kind.unknown'), className: styles.pillMuted };
    }
    return { text: t('plugins.kind.provider'), className: styles.pillProvider };
  };

  const sourceLabel = (sourceType: string) => {
    if (sourceType === 'github') return t('plugins.source.github');
    if (sourceType === 'local') return t('plugins.source.local');
    return t('plugins.source.npm');
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
      <div className={styles.filterBar}>
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
        <Input
          allowClear
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          prefix={<Search size={14} aria-hidden="true" />}
          placeholder={t('plugins.installed.searchPlaceholder')}
          className={styles.searchInput}
        />
        <span className={styles.totalLabel}>
          {t('plugins.installed.count', { count: filteredPlugins.length })}
        </span>
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
        <div className={styles.grid}>
          {filteredPlugins.map((plugin) => {
            const category = getPluginKindCategory(plugin.kind);
            const kind = kindLabel(plugin);
            const middlewareState = getMiddlewareState(plugin);
            const canManageAccounts = isProviderPluginKind(category) && plugin.hasProviderEntry;
            const accountCount = getAccountCount(plugin);
            const summary = getPluginSummary(plugin.summary, i18n.language);
            const busy = busyId === plugin.id;
            const openablePages = plugin.enabled ? plugin.pages ?? [] : [];

            return (
              <article key={plugin.id} className={styles.card}>
                <header className={styles.cardHeader}>
                  <PluginIcon icon={plugin.icon} name={plugin.displayName || plugin.packageName} />
                  <div className={styles.cardHeading}>
                    <div className={styles.nameRow}>
                      <span
                        className={styles.name}
                        title={plugin.displayName || plugin.packageName}
                      >
                        {plugin.displayName || plugin.packageName}
                      </span>
                      <span className={styles.version}>v{plugin.version || '—'}</span>
                    </div>
                    <div className={styles.metaRow}>
                      <span className={`${styles.pill} ${kind.className}`}>{kind.text}</span>
                      <span className={`${styles.pill} ${styles.pillMuted}`}>
                        {sourceLabel(plugin.sourceType)}
                      </span>
                      {plugin.marketId ? (
                        <span className={`${styles.pill} ${styles.pillMuted}`}>
                          {t('plugins.installed.marketTag')}
                        </span>
                      ) : null}
                    </div>
                  </div>
                  <Switch
                    size="small"
                    checked={plugin.enabled}
                    loading={busy}
                    disabled={busy}
                    aria-label={t('plugins.installed.toggleLabel', {
                      name: plugin.displayName || plugin.packageName,
                    })}
                    onChange={(checked) => void handleToggle(plugin, checked)}
                  />
                </header>

                <p className={styles.summary}>
                  {summary || plugin.packageName || plugin.spec}
                </p>

                <div className={styles.metrics}>
                  {isProviderPluginKind(category) ? (
                    <>
                      <span className={styles.metric}>
                        <span className={styles.metricLabel}>
                          {t('plugins.installed.metrics.channels')}
                        </span>
                        {plugin.providers.length}
                      </span>
                      <span className={styles.metric}>
                        <span className={styles.metricLabel}>
                          {t('plugins.installed.metrics.accounts')}
                        </span>
                        {accountCount}
                      </span>
                    </>
                  ) : null}
                  {isMiddlewarePluginKind(category) && middlewareState ? (
                    <>
                      <span className={styles.metric}>
                        <span className={styles.metricLabel}>
                          {t('plugins.installed.metrics.calls')}
                        </span>
                        {middlewareState.calls}
                      </span>
                      <span className={styles.metric}>
                        <span className={styles.metricLabel}>
                          {t('plugins.installed.metrics.avg')}
                        </span>
                        {formatMicroseconds(middlewareState.avgMicros)}
                      </span>
                      <span className={styles.metric}>
                        <span className={styles.metricLabel}>
                          {t('plugins.installed.metrics.failures')}
                        </span>
                        {middlewareState.failures}
                      </span>
                    </>
                  ) : null}
                  {isMiddlewarePluginKind(category) && !middlewareState ? (
                    <span className={styles.metricMuted}>
                      {t('plugins.installed.metrics.notLoaded')}
                    </span>
                  ) : null}
                  {plugin.pages && plugin.pages.length > 0 ? (
                    <span className={styles.metric}>
                      <span className={styles.metricLabel}>
                        {t('plugins.installed.metrics.pages')}
                      </span>
                      {plugin.pages.length}
                    </span>
                  ) : null}
                </div>

                {plugin.lastError ? (
                  <details className={styles.errorDetails}>
                    <summary>{t('plugins.installed.viewError')}</summary>
                    <p>{plugin.lastError}</p>
                  </details>
                ) : null}

                <footer className={styles.cardFooter}>
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
                  <span className={styles.footerSpacer} />
                  <button
                    type="button"
                    className={`${styles.textButton} ${styles.textButtonDanger}`}
                    disabled={busy}
                    onClick={() => openUninstallConfirm(plugin)}
                  >
                    <Trash2 size={14} aria-hidden="true" />
                    <span>{t('plugins.installed.actions.uninstall')}</span>
                  </button>
                </footer>
              </article>
            );
          })}
        </div>
      )}

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
