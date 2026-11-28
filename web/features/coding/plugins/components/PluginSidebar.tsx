import React from 'react';
import { Tooltip } from 'antd';
import {
  Loader2,
  PackageCheck,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  RefreshCw,
  Store,
  Terminal,
  Users,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { BunRuntimeStatus, RegisteredPluginPage } from '@/services';
import { getPluginPageTitle } from '../utils/pluginFormatters';
import { PLUGIN_TABS, type PluginsPageTab } from '../utils/pluginNavigation';
import PluginPageGlyph from './PluginPageGlyph';
import styles from './PluginSidebar.module.less';

const TAB_ICONS: Record<PluginsPageTab, React.ReactNode> = {
  installed: <PackageCheck size={15} aria-hidden="true" />,
  accounts: <Users size={15} aria-hidden="true" />,
  market: <Store size={15} aria-hidden="true" />,
  runtime: <Terminal size={15} aria-hidden="true" />,
};

interface PluginSidebarProps {
  collapsed: boolean;
  overlay?: boolean;
  onToggleCollapsed: () => void;
  activeTab: PluginsPageTab | null;
  activePageKey: string | null;
  counts: Record<PluginsPageTab, number>;
  pages: RegisteredPluginPage[];
  onSelectTab: (tab: PluginsPageTab) => void;
  onSelectPage: (page: RegisteredPluginPage) => void;
  onInstall: () => void;
  onRefresh: () => void;
  refreshing: boolean;
  bunStatus: BunRuntimeStatus | null;
  bunBusy: boolean;
  onBunClick: () => void;
}

const PluginSidebar: React.FC<PluginSidebarProps> = ({
  collapsed,
  overlay = false,
  onToggleCollapsed,
  activeTab,
  activePageKey,
  counts,
  pages,
  onSelectTab,
  onSelectPage,
  onInstall,
  onRefresh,
  refreshing,
  bunStatus,
  bunBusy,
  onBunClick,
}) => {
  const { t, i18n } = useTranslation();
  const bunAvailable = Boolean(bunStatus?.available);
  const bunPreparing = Boolean(bunStatus?.downloading);

  const bunTooltip = bunAvailable
    ? t('plugins.page.bun.readyTooltip', {
      version: bunStatus?.version ?? bunStatus?.expectedVersion ?? '',
    })
    : bunPreparing
      ? t('plugins.page.bun.downloadingTooltip')
      : t('plugins.page.bun.missingTooltip');

  return (
    <aside
      className={[
        styles.sidebar,
        collapsed ? styles.sidebarCollapsed : '',
        overlay ? styles.sidebarOverlay : '',
      ].filter(Boolean).join(' ')}
    >
      <div className={styles.header}>
        {!collapsed ? (
          <span className={styles.brandTitle}>{t('plugins.page.title')}</span>
        ) : null}
        <Tooltip
          title={collapsed ? t('plugins.sidebar.expand') : t('plugins.sidebar.collapse')}
          placement="right"
        >
          <button
            type="button"
            className={styles.iconButton}
            aria-label={collapsed ? t('plugins.sidebar.expand') : t('plugins.sidebar.collapse')}
            onClick={onToggleCollapsed}
          >
            {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          </button>
        </Tooltip>
      </div>

      <Tooltip title={collapsed ? t('plugins.page.actions.install') : undefined} placement="right">
        <button
          type="button"
          className={`${styles.installButton} ${collapsed ? styles.installButtonCollapsed : ''}`}
          aria-label={t('plugins.page.actions.install')}
          onClick={onInstall}
        >
          <Plus size={15} aria-hidden="true" />
          {!collapsed ? <span>{t('plugins.page.actions.install')}</span> : null}
        </button>
      </Tooltip>

      <nav className={styles.nav} aria-label={t('plugins.page.title')}>
        <div className={styles.group}>
          {!collapsed ? (
            <div className={styles.groupLabel}>{t('plugins.sidebar.groups.manage')}</div>
          ) : null}
          {PLUGIN_TABS.map((tab) => {
            const label = t(tab.labelKey);
            const count = counts[tab.key];
            const selected = activeTab === tab.key;
            return (
              <Tooltip
                key={tab.key}
                title={collapsed ? t('plugins.sidebar.itemTooltip', { label, count }) : undefined}
                placement="right"
              >
                <button
                  type="button"
                  className={`${styles.navItem} ${selected ? styles.navItemActive : ''}`}
                  aria-label={t('plugins.sidebar.itemTooltip', { label, count })}
                  aria-current={selected ? 'page' : undefined}
                  onClick={() => onSelectTab(tab.key)}
                >
                  <span className={styles.navIcon}>{TAB_ICONS[tab.key]}</span>
                  {!collapsed ? (
                    <>
                      <span className={styles.navLabel}>{label}</span>
                      <span className={styles.navCount}>{count}</span>
                    </>
                  ) : null}
                </button>
              </Tooltip>
            );
          })}
        </div>

        {pages.length > 0 ? (
          <div className={styles.group}>
            {!collapsed ? (
              <div className={styles.groupLabel}>{t('plugins.sidebar.groups.pages')}</div>
            ) : (
              <div className={styles.groupDivider} aria-hidden="true" />
            )}
            {pages.map((page) => {
              const title = getPluginPageTitle(page, i18n.language);
              const pageKey = `${page.pluginId}:${page.pageId}`;
              const selected = activePageKey === pageKey;
              return (
                <Tooltip
                  key={pageKey}
                  title={collapsed ? title : undefined}
                  placement="right"
                >
                  <button
                    type="button"
                    className={`${styles.navItem} ${selected ? styles.navItemActive : ''}`}
                    aria-label={title}
                    aria-current={selected ? 'page' : undefined}
                    onClick={() => onSelectPage(page)}
                  >
                    <span className={styles.navIcon}>
                      <PluginPageGlyph
                        iconDataUrl={page.iconDataUrl}
                        pluginIcon={page.pluginIcon}
                        pluginName={page.pluginName || page.packageName}
                        title={title}
                        size={collapsed ? 22 : 24}
                      />
                    </span>
                    {!collapsed ? <span className={styles.navLabel}>{title}</span> : null}
                  </button>
                </Tooltip>
              );
            })}
          </div>
        ) : null}
      </nav>

      <div className={styles.footer}>
        <Tooltip title={collapsed ? bunTooltip : undefined} placement="right">
          <button
            type="button"
            className={`${styles.bunButton} ${collapsed ? styles.bunButtonCollapsed : ''}`}
            aria-label={bunTooltip}
            onClick={onBunClick}
            disabled={bunBusy || bunPreparing || !bunStatus}
          >
            {bunBusy || bunPreparing ? (
              <Loader2 size={13} className={styles.spin} aria-hidden="true" />
            ) : (
              <span
                className={`${styles.bunDot} ${bunAvailable ? styles.bunDotReady : styles.bunDotMissing}`}
                aria-hidden="true"
              />
            )}
            {!collapsed ? (
              <span className={styles.bunText}>
                {bunBusy || bunPreparing
                  ? t('plugins.page.bun.downloading')
                  : bunAvailable
                    ? t('plugins.page.bun.ready', {
                      version: bunStatus?.version ?? bunStatus?.expectedVersion ?? '',
                    })
                    : bunStatus
                      ? t('plugins.page.bun.missing')
                      : t('plugins.page.bun.checking')}
              </span>
            ) : null}
          </button>
        </Tooltip>
        <Tooltip title={t('plugins.page.actions.refresh')} placement="right">
          <button
            type="button"
            className={styles.iconButton}
            aria-label={t('plugins.page.actions.refresh')}
            onClick={onRefresh}
            disabled={refreshing}
          >
            <RefreshCw
              size={15}
              className={refreshing ? styles.spin : undefined}
              aria-hidden="true"
            />
          </button>
        </Tooltip>
      </div>
    </aside>
  );
};

export default PluginSidebar;
