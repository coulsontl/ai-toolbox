import React from 'react';
import { App, Spin } from 'antd';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import {
  checkPluginUpdates,
  closePluginPage,
  ensurePluginBun,
  getPluginBunStatus,
  getPluginMiddlewareStates,
  getPluginSettings,
  listInstalledPlugins,
  listPluginAccounts,
  listPluginMarkets,
  listPluginPages,
  openPluginPage,
  savePluginSettings,
  type BunRuntimeStatus,
  type InstalledPluginRecord,
  type MarketSourceRecord,
  type MiddlewareStateView,
  type PluginAccountSummary,
  type PluginPageSession,
  type PluginSettings,
  type PluginUpdateItem,
  type PluginUpdateSummary,
  type RegisteredPluginPage,
} from '@/services';
import InstalledPluginsView from '../components/InstalledPluginsView';
import PluginAccountsView from '../components/PluginAccountsView';
import PluginInstallModal from '../components/PluginInstallModal';
import PluginMarketView from '../components/PluginMarketView';
import PluginOptionsModal from '../components/PluginOptionsModal';
import PluginPageFrame from '../components/PluginPageFrame';
import PluginRuntimeView from '../components/PluginRuntimeView';
import PluginSidebar from '../components/PluginSidebar';
import {
  formatPluginError,
  NPM_REGISTRY_CHINA,
  NPM_REGISTRY_OFFICIAL,
} from '../utils/pluginFormatters';
import { usePluginBridgeTheme } from '../utils/pluginPageTheme';
import {
  DEFAULT_PLUGINS_PATH,
  getPluginsAccountsPath,
  getPluginsCustomPagePath,
  getPluginsPathForTab,
  isPluginsPath,
  rememberLastPluginsPath,
  resolvePluginsRoute,
  type PluginsPageTab,
} from '../utils/pluginNavigation';
import styles from './PluginsPage.module.less';

const SIDEBAR_COLLAPSED_KEY = 'ai-toolbox:plugins-sidebar-collapsed';
const NARROW_BREAKPOINT = 860;
const PAGE_RECYCLE_MS = 10 * 60 * 1000;

interface MountedPageEntry {
  key: string;
  page: RegisteredPluginPage;
  session: PluginPageSession | null;
  loading: boolean;
  error: string | null;
}

const getPageKey = (page: Pick<RegisteredPluginPage, 'pluginId' | 'pageId'>) =>
  `${page.pluginId}:${page.pageId}`;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const PluginsPage: React.FC = () => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const navigate = useNavigate();
  const location = useLocation();
  const { isActive: keepAliveActive } = useKeepAlive();
  const route = resolvePluginsRoute(location.pathname);
  const activeTab = route.kind === 'tab' ? route.tab : null;
  const routeKind = route.kind;
  const routePluginId = route.kind === 'page' ? route.pluginId : null;
  const routePageId = route.kind === 'page' ? route.pageId : null;
  const isPageRoute = route.kind === 'page';
  const bridgeTheme = usePluginBridgeTheme();

  const [installedPlugins, setInstalledPlugins] = React.useState<InstalledPluginRecord[]>([]);
  const [accounts, setAccounts] = React.useState<PluginAccountSummary[]>([]);
  const [markets, setMarkets] = React.useState<MarketSourceRecord[]>([]);
  const [middlewareStates, setMiddlewareStates] = React.useState<MiddlewareStateView[]>([]);
  const [bunStatus, setBunStatus] = React.useState<BunRuntimeStatus | null>(null);
  const [pluginSettings, setPluginSettings] = React.useState<PluginSettings | null>(null);
  const [chinaMirrorBusy, setChinaMirrorBusy] = React.useState(false);
  const [marketCount, setMarketCount] = React.useState<number | null>(null);
  const [registeredPages, setRegisteredPages] = React.useState<RegisteredPluginPage[]>([]);
  const [pagesLoaded, setPagesLoaded] = React.useState(false);
  const [pagesError, setPagesError] = React.useState<string | null>(null);
  const [mountedPages, setMountedPages] = React.useState<MountedPageEntry[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [refreshing, setRefreshing] = React.useState(false);
  const [installedRefreshing, setInstalledRefreshing] = React.useState(false);
  // Shared by the runtime and installed views so both show the same versions.
  const [updateItems, setUpdateItems] = React.useState<Record<string, PluginUpdateItem>>({});
  const [checkingUpdates, setCheckingUpdates] = React.useState(false);
  const [bunBusy, setBunBusy] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [refreshKey, setRefreshKey] = React.useState(0);
  const [installOpen, setInstallOpen] = React.useState(false);
  const [installPrefill, setInstallPrefill] = React.useState('');
  const [optionsTarget, setOptionsTarget] = React.useState<InstalledPluginRecord | null>(null);
  const [visitedTabs, setVisitedTabs] = React.useState<Set<PluginsPageTab>>(
    () => new Set(activeTab ? [activeTab] : []),
  );

  const [sidebarCollapsed, setSidebarCollapsed] = React.useState(
    () => window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === '1',
  );
  const [narrow, setNarrow] = React.useState(
    () => window.matchMedia(`(max-width: ${NARROW_BREAKPOINT}px)`).matches,
  );
  const [narrowOpen, setNarrowOpen] = React.useState(false);
  const effectiveCollapsed = narrow ? !narrowOpen : sidebarCollapsed;
  const sidebarOverlay = narrow && narrowOpen;

  const mountedKeysRef = React.useRef(new Set<string>());
  const recycleTimersRef = React.useRef(new Map<string, number>());

  React.useEffect(() => {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, sidebarCollapsed ? '1' : '0');
  }, [sidebarCollapsed]);

  React.useEffect(() => {
    const media = window.matchMedia(`(max-width: ${NARROW_BREAKPOINT}px)`);
    const syncNarrow = () => {
      setNarrow(media.matches);
      if (!media.matches) {
        setNarrowOpen(false);
      }
    };
    syncNarrow();
    media.addEventListener('change', syncNarrow);
    return () => media.removeEventListener('change', syncNarrow);
  }, []);

  const loadSharedData = React.useCallback(async (): Promise<boolean> => {
    const results = await Promise.allSettled([
      listInstalledPlugins(),
      listPluginAccounts(),
      listPluginMarkets(),
      getPluginMiddlewareStates(),
      getPluginBunStatus(),
      getPluginSettings(),
      listPluginPages(),
    ]);
    const [
      installedResult,
      accountsResult,
      marketsResult,
      middlewareResult,
      bunResult,
      settingsResult,
      pagesResult,
    ] = results;

    if (installedResult.status === 'fulfilled') {
      setInstalledPlugins(installedResult.value);
    }
    if (accountsResult.status === 'fulfilled') {
      setAccounts(accountsResult.value);
    }
    if (marketsResult.status === 'fulfilled') {
      setMarkets(marketsResult.value);
    }
    if (middlewareResult.status === 'fulfilled') {
      setMiddlewareStates(middlewareResult.value);
    }
    if (bunResult.status === 'fulfilled') {
      setBunStatus(bunResult.value);
    }
    if (settingsResult.status === 'fulfilled') {
      setPluginSettings(settingsResult.value);
    }
    if (pagesResult.status === 'fulfilled') {
      setRegisteredPages(pagesResult.value);
      setPagesError(null);
    } else {
      // Keep the last known pages so a transient failure doesn't tear down
      // frames the reader still has open.
      setPagesError(formatPluginError(pagesResult.reason));
    }

    const firstFailure = results.find(
      (result): result is PromiseRejectedResult => result.status === 'rejected',
    );
    setLoadError(firstFailure ? formatPluginError(firstFailure.reason) : null);
    setPagesLoaded(true);
    return !firstFailure;
  }, []);

  // Child views only need completion semantics when they reload shared data
  // after their own mutations, so hide the success flag they would ignore.
  const reloadSharedData = React.useCallback(async (): Promise<void> => {
    await loadSharedData();
  }, [loadSharedData]);

  React.useEffect(() => {
    let disposed = false;
    void loadSharedData().finally(() => {
      if (!disposed) {
        setLoading(false);
      }
    });
    return () => {
      disposed = true;
    };
  }, [loadSharedData]);

  React.useEffect(() => {
    if (location.pathname === '/plugins') {
      navigate(DEFAULT_PLUGINS_PATH, { replace: true });
      return;
    }
    if (isPluginsPath(location.pathname)) {
      rememberLastPluginsPath(`${location.pathname}${location.search}`);
    }
  }, [location.pathname, location.search, navigate]);

  React.useEffect(() => {
    if (!activeTab) {
      return;
    }
    setVisitedTabs((current) => {
      if (current.has(activeTab)) {
        return current;
      }
      const next = new Set(current);
      next.add(activeTab);
      return next;
    });
  }, [activeTab]);

  const routePage = React.useMemo(() => {
    if (routeKind !== 'page' || !routePluginId) {
      return null;
    }
    const candidates = registeredPages.filter((page) => page.pluginId === routePluginId);
    if (routePageId) {
      return candidates.find((page) => page.pageId === routePageId) ?? null;
    }
    return candidates[0] ?? null;
  }, [registeredPages, routeKind, routePageId, routePluginId]);

  const activePageKey = routePage ? getPageKey(routePage) : null;

  React.useEffect(() => {
    if (
      routeKind === 'page'
      && pagesLoaded
      && !pagesError
      && !routePage
    ) {
      navigate(DEFAULT_PLUGINS_PATH, { replace: true });
    }
  }, [navigate, pagesError, pagesLoaded, routeKind, routePage]);

  const clearRecycleTimer = React.useCallback((key: string) => {
    const timer = recycleTimersRef.current.get(key);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      recycleTimersRef.current.delete(key);
    }
  }, []);

  const openPageSession = React.useCallback(async (page: RegisteredPluginPage) => {
    const key = getPageKey(page);
    setMountedPages((current) => current.map((entry) => (
      entry.key === key ? { ...entry, loading: true, error: null } : entry
    )));
    try {
      const session = await openPluginPage(page.pluginId, page.pageId);
      setMountedPages((current) => current.map((entry) => (
        entry.key === key ? { ...entry, session, loading: false, error: null } : entry
      )));
    } catch (error) {
      setMountedPages((current) => current.map((entry) => (
        entry.key === key
          ? { ...entry, session: null, loading: false, error: formatPluginError(error) }
          : entry
      )));
    }
  }, []);

  // Mount the page the route names, and refresh its metadata when the
  // installed record changes. A page recycled after being hidden remounts
  // here because the reader is back on its route.
  React.useEffect(() => {
    if (!routePage) {
      return;
    }
    const key = getPageKey(routePage);
    if (!mountedKeysRef.current.has(key)) {
      mountedKeysRef.current.add(key);
      setMountedPages((current) => [
        ...current,
        { key, page: routePage, session: null, loading: true, error: null },
      ]);
      void openPageSession(routePage);
      return;
    }
    setMountedPages((current) => {
      let changed = false;
      const next = current.map((entry) => {
        if (entry.key !== key || entry.page === routePage) {
          return entry;
        }
        changed = true;
        return { ...entry, page: routePage };
      });
      return changed ? next : current;
    });
  }, [mountedPages, openPageSession, routePage]);

  // A disabled or uninstalled plugin disappears from `plugins_list_pages`;
  // its frames must go with it.
  React.useEffect(() => {
    if (!pagesLoaded || pagesError) {
      return;
    }
    const knownKeys = new Set(registeredPages.map((page) => getPageKey(page)));
    const staleEntries = mountedPages.filter((entry) => !knownKeys.has(entry.key));
    if (staleEntries.length === 0) {
      return;
    }
    for (const entry of staleEntries) {
      mountedKeysRef.current.delete(entry.key);
      clearRecycleTimer(entry.key);
      void closePluginPage(entry.page.pluginId, entry.page.pageId).catch(() => undefined);
    }
    setMountedPages((current) => current.filter((entry) => knownKeys.has(entry.key)));
  }, [clearRecycleTimer, mountedPages, pagesError, pagesLoaded, registeredPages]);

  // Hidden pages keep running only while their package asks for it; the rest
  // are let go after ten minutes. Leaving `/plugins` for another main module
  // also counts as hiding, even though the page route itself did not change.
  const visiblePageKey = isPageRoute && keepAliveActive ? activePageKey : null;

  React.useEffect(() => {
    for (const entry of mountedPages) {
      if (entry.key === visiblePageKey || entry.page.keepAlive) {
        clearRecycleTimer(entry.key);
        continue;
      }
      if (recycleTimersRef.current.has(entry.key)) {
        continue;
      }
      const timer = window.setTimeout(() => {
        recycleTimersRef.current.delete(entry.key);
        mountedKeysRef.current.delete(entry.key);
        setMountedPages((current) => current.filter((item) => item.key !== entry.key));
        void closePluginPage(entry.page.pluginId, entry.page.pageId).catch(() => undefined);
      }, PAGE_RECYCLE_MS);
      recycleTimersRef.current.set(entry.key, timer);
    }
  }, [clearRecycleTimer, mountedPages, visiblePageKey]);

  React.useEffect(() => () => {
    for (const timer of recycleTimersRef.current.values()) {
      window.clearTimeout(timer);
    }
    recycleTimersRef.current.clear();
  }, []);

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      const succeeded = await loadSharedData();
      setRefreshKey((current) => current + 1);
      if (succeeded) {
        message.success(t('plugins.page.notice.refreshed'));
      }
    } finally {
      setRefreshing(false);
    }
  };

  // One switch shared by the installed list, market and runtime settings; it
  // only rewrites `npmRegistry`, so the other settings are carried through.
  const handleToggleChinaMirror = async (enabled: boolean) => {
    if (!pluginSettings || chinaMirrorBusy) {
      return;
    }
    const previous = pluginSettings;
    const next: PluginSettings = {
      ...pluginSettings,
      npmRegistry: enabled ? NPM_REGISTRY_CHINA : NPM_REGISTRY_OFFICIAL,
    };
    setPluginSettings(next);
    setChinaMirrorBusy(true);
    try {
      const saved = await savePluginSettings(next);
      setPluginSettings(saved);
      if (enabled) {
        message.success(t('plugins.page.notice.mirrorOn'));
      } else {
        message.success(t('plugins.page.notice.mirrorOff'));
      }
    } catch (error) {
      setPluginSettings(previous);
      message.error(
        t('plugins.page.notice.mirrorFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setChinaMirrorBusy(false);
    }
  };

  const handleRefreshInstalled = async () => {
    setInstalledRefreshing(true);
    try {
      const succeeded = await loadSharedData();
      if (succeeded) {
        message.success(t('plugins.page.notice.refreshed'));
      }
    } finally {
      setInstalledRefreshing(false);
    }
  };

  // A full desktop check records the timestamp on the backend; mirror the
  // returned value so the runtime view updates without a settings reload.
  const handleCheckUpdates = async (): Promise<PluginUpdateSummary> => {
    setCheckingUpdates(true);
    try {
      const summary = await checkPluginUpdates();
      setUpdateItems(
        Object.fromEntries(
          summary.items
            .filter((item) => item.status === 'available')
            .map((item) => [item.pluginId, item]),
        ),
      );
      setPluginSettings((current) =>
        current ? { ...current, lastAutoUpdateCheckAt: summary.checkedAt } : current,
      );
      return summary;
    } finally {
      setCheckingUpdates(false);
    }
  };

  // Failed items keep their badge so the reader can retry them.
  const handleUpdateSummary = (summary: PluginUpdateSummary) => {
    const settledIds = new Set(
      summary.items
        .filter((item) => item.status !== 'failed' && item.status !== 'available')
        .map((item) => item.pluginId),
    );
    setUpdateItems((current) => {
      const next: Record<string, PluginUpdateItem> = {};
      for (const [pluginId, item] of Object.entries(current)) {
        if (!settledIds.has(pluginId)) {
          next[pluginId] = item;
        }
      }
      return next;
    });
  };

  const refreshAfterPluginUpdate = React.useCallback(async () => {
    const results = await Promise.allSettled([
      listInstalledPlugins(),
      getPluginSettings(),
      listPluginPages(),
    ]);
    const [installedResult, settingsResult, pagesResult] = results;
    if (installedResult.status === 'fulfilled') {
      setInstalledPlugins(installedResult.value);
    }
    if (settingsResult.status === 'fulfilled') {
      setPluginSettings(settingsResult.value);
    }
    if (pagesResult.status === 'fulfilled') {
      setRegisteredPages(pagesResult.value);
      setPagesError(null);
    }
  }, []);

  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{ updatedIds?: string[] }>('plugins-updated', (event) => {
      const updatedIds = event.payload?.updatedIds ?? [];
      if (updatedIds.length > 0) {
        setUpdateItems((current) => {
          const next = { ...current };
          for (const pluginId of updatedIds) {
            delete next[pluginId];
          }
          return next;
        });
      }
      void refreshAfterPluginUpdate();
    }).then((stop) => {
      if (disposed) {
        stop();
        return;
      }
      unlisten = stop;
    }).catch(() => {
      // Updates still refresh through the manual buttons when the event bridge
      // is unavailable (for example while the page runs in a browser).
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshAfterPluginUpdate]);

  const handleBunClick = async () => {
    if (bunStatus?.available || bunStatus?.downloading) {
      navigate(getPluginsPathForTab('runtime'));
      return;
    }
    setBunBusy(true);
    try {
      const nextStatus = await ensurePluginBun();
      setBunStatus(nextStatus);
      if (nextStatus.available) {
        message.success(
          t('plugins.page.bun.initSuccess', {
            version: nextStatus.version ?? nextStatus.expectedVersion,
          }),
        );
      } else {
        message.error(
          t('plugins.page.bun.initFailed', {
            error: nextStatus.lastError ?? t('common.unknown'),
          }),
        );
      }
    } catch (error) {
      message.error(
        t('plugins.page.bun.initFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setBunBusy(false);
    }
  };

  const openInstallModal = (spec = '') => {
    setInstallPrefill(spec);
    setInstallOpen(true);
  };

  const handleInstalled = async (record: InstalledPluginRecord) => {
    setInstallOpen(false);
    setInstallPrefill('');
    await loadSharedData();
    setRefreshKey((current) => current + 1);
    navigate(getPluginsPathForTab('installed'));
    message.success(
      t('plugins.installModal.success', {
        name: record.displayName || record.packageName || record.spec,
      }),
    );
  };

  const handleOptionsSaved = (record: InstalledPluginRecord) => {
    setInstalledPlugins((current) =>
      current.map((plugin) => (plugin.id === record.id ? record : plugin)),
    );
    setOptionsTarget(null);
  };

  const handlePageOptionsChanged = (
    pluginId: string,
    nextOptions: Record<string, unknown>,
  ) => {
    setInstalledPlugins((current) =>
      current.map((plugin) => (
        plugin.id === pluginId ? { ...plugin, options: nextOptions } : plugin
      )),
    );
  };

  const pluginRecordById = React.useMemo(
    () => new Map(installedPlugins.map((plugin) => [plugin.id, plugin])),
    [installedPlugins],
  );

  const handleOpenPageOptions = (pluginId: string) => {
    const record = pluginRecordById.get(pluginId);
    if (record) {
      setOptionsTarget(record);
    }
  };

  const optionsForPage = (entry: MountedPageEntry): Record<string, unknown> | null => {
    const record = pluginRecordById.get(entry.page.pluginId);
    const rawOptions = record?.options ?? entry.session?.options ?? null;
    return isRecord(rawOptions) ? rawOptions : null;
  };

  const tabCounts: Record<PluginsPageTab, number> = {
    installed: installedPlugins.length,
    // The badge matches the per-channel capsules: signed-in account count.
    accounts: accounts.length,
    market: marketCount ?? markets.reduce((sum, market) => sum + market.pluginCount, 0),
    runtime: middlewareStates.length,
  };

  const handleSelectTab = (tab: PluginsPageTab) => {
    setNarrowOpen(false);
    navigate(getPluginsPathForTab(tab));
  };

  const handleSelectPage = (page: RegisteredPluginPage) => {
    setNarrowOpen(false);
    navigate(getPluginsCustomPagePath(page.pluginId, page.pageId));
  };

  return (
    <div
      className={styles.pluginsPage}
      style={{ ['--plugins-header-height' as string]: '0px' }}
    >
      <div className={styles.workspace}>
        <PluginSidebar
          collapsed={effectiveCollapsed}
          overlay={sidebarOverlay}
          onToggleCollapsed={() => {
            if (narrow) {
              setNarrowOpen((current) => !current);
              return;
            }
            setSidebarCollapsed((current) => !current);
          }}
          activeTab={activeTab}
          activePageKey={activePageKey}
          counts={tabCounts}
          pages={registeredPages}
          onSelectTab={handleSelectTab}
          onSelectPage={handleSelectPage}
          onInstall={() => openInstallModal()}
          onRefresh={() => void handleRefresh()}
          refreshing={refreshing}
          bunStatus={bunStatus}
          bunBusy={bunBusy}
          onBunClick={() => void handleBunClick()}
        />
        {sidebarOverlay ? (
          <button
            type="button"
            className={styles.scrim}
            aria-label={t('plugins.sidebar.closeMenu')}
            onClick={() => setNarrowOpen(false)}
          />
        ) : null}

        <div
          className={`${styles.stage} ${styles.stagePage}`}
          style={{ display: isPageRoute ? undefined : 'none' }}
        >
          <div className={styles.pageStage}>
            {pagesError ? (
              <div className={`${styles.notice} ${styles.noticeError}`} role="alert">
                {t('plugins.page.notice.loadFailed', { error: pagesError })}
              </div>
            ) : null}
            {!pagesError && mountedPages.length === 0 ? (
              <div className={styles.stagePlaceholder}>
                <Spin size="small" />
                <span>{t('plugins.pageHost.loading')}</span>
              </div>
            ) : null}
            {mountedPages.length > 0 ? (
              <div className={styles.pageFrames}>
                {mountedPages.map((entry) => {
                  const pageVisible =
                    isPageRoute && keepAliveActive && entry.key === activePageKey;
                  return (
                    <PluginPageFrame
                      key={entry.key}
                      page={entry.page}
                      session={entry.session}
                      sessionLoading={entry.loading}
                      sessionError={entry.error}
                      visible={pageVisible}
                      theme={bridgeTheme}
                      options={optionsForPage(entry)}
                      onRetrySession={() => void openPageSession(entry.page)}
                      onOptionsChanged={handlePageOptionsChanged}
                      onOpenOptions={handleOpenPageOptions}
                      onOpenInstall={(packageName) => openInstallModal(packageName)}
                    />
                  );
                })}
              </div>
            ) : null}
          </div>
        </div>

        <div
          className={`${styles.stage} ${styles.stageManage}`}
          style={{ display: isPageRoute ? 'none' : undefined }}
        >
          {loadError ? (
            <div className={`${styles.notice} ${styles.noticeError}`} role="alert">
              {t('plugins.page.notice.loadFailed', { error: loadError })}
            </div>
          ) : null}

          {visitedTabs.has('installed') ? (
            <div style={{ display: activeTab === 'installed' ? undefined : 'none' }}>
              <InstalledPluginsView
                plugins={installedPlugins}
                accounts={accounts}
                middlewareStates={middlewareStates}
                bunStatus={bunStatus}
                pluginSettings={pluginSettings}
                chinaMirrorBusy={chinaMirrorBusy}
                updateItems={updateItems}
                checkingUpdates={checkingUpdates}
                loading={loading}
                refreshing={installedRefreshing}
                onRefresh={() => void handleRefreshInstalled()}
                onCheckUpdates={handleCheckUpdates}
                onUpdateSummary={handleUpdateSummary}
                onDataChanged={reloadSharedData}
                onToggleChinaMirror={(enabled) => void handleToggleChinaMirror(enabled)}
                onOpenInstall={() => openInstallModal()}
                onBrowseMarket={() => handleSelectTab('market')}
                onOpenRuntime={() => handleSelectTab('runtime')}
                onManageAccounts={(providerId) => navigate(getPluginsAccountsPath(providerId))}
                onEditOptions={setOptionsTarget}
                onOpenPage={(pluginId, pageId) => navigate(getPluginsCustomPagePath(pluginId, pageId))}
              />
            </div>
          ) : null}
          {visitedTabs.has('accounts') ? (
            <div style={{ display: activeTab === 'accounts' ? undefined : 'none' }}>
              <PluginAccountsView
                active={keepAliveActive && !isPageRoute && activeTab === 'accounts'}
                refreshKey={refreshKey}
                installedPlugins={installedPlugins}
                onDataChanged={reloadSharedData}
                onBrowseMarket={() => handleSelectTab('market')}
                onOpenRuntime={() => handleSelectTab('runtime')}
              />
            </div>
          ) : null}
          {visitedTabs.has('market') ? (
            <div style={{ display: activeTab === 'market' ? undefined : 'none' }}>
              <PluginMarketView
                refreshKey={refreshKey}
                pluginSettings={pluginSettings}
                chinaMirrorBusy={chinaMirrorBusy}
                onToggleChinaMirror={(enabled) => void handleToggleChinaMirror(enabled)}
                onDataChanged={reloadSharedData}
                onListingCountChange={setMarketCount}
              />
            </div>
          ) : null}
          {visitedTabs.has('runtime') ? (
            <div style={{ display: activeTab === 'runtime' ? undefined : 'none' }}>
              <PluginRuntimeView
                active={keepAliveActive && !isPageRoute && activeTab === 'runtime'}
                refreshKey={refreshKey}
                pluginSettings={pluginSettings}
                chinaMirrorBusy={chinaMirrorBusy}
                checkingUpdates={checkingUpdates}
                onToggleChinaMirror={(enabled) => void handleToggleChinaMirror(enabled)}
                onCheckUpdates={handleCheckUpdates}
                onSettingsSaved={setPluginSettings}
                onBunStatusChange={setBunStatus}
              />
            </div>
          ) : null}
        </div>
      </div>

      <PluginInstallModal
        open={installOpen}
        initialSpec={installPrefill}
        onClose={() => {
          setInstallOpen(false);
          setInstallPrefill('');
        }}
        onInstalled={(record) => void handleInstalled(record)}
      />
      <PluginOptionsModal
        open={Boolean(optionsTarget)}
        plugin={optionsTarget}
        onClose={() => setOptionsTarget(null)}
        onSaved={handleOptionsSaved}
      />
    </div>
  );
};

export default PluginsPage;
