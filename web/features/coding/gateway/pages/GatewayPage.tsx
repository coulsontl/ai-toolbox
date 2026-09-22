import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { Dropdown } from 'antd';
import type { MenuProps } from 'antd';
import {
  Activity,
  BarChart3,
  ChevronDown,
  FileText,
  Loader2,
  Network,
  Power,
  RotateCcw,
  Settings,
  Square,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import GatewaySettingsPanel from '@/features/settings/pages/GatewaySettingsPanel';
import {
  checkProxyGatewayHealth,
  getProxyGatewaySettings,
  getProxyGatewayStatus,
  importProxyGatewaySessionUsage,
  preflightStopProxyGateway,
  restartProxyGateway,
  startProxyGateway,
  stopProxyGateway,
  updateProxyGatewaySettings,
  type ProxyGatewaySettings,
  type ProxyGatewayStatus,
} from '@/services';
import GatewayRequestsView from '../components/GatewayRequestsView';
import GatewayStatisticsView from '../components/GatewayStatisticsView';
import { formatGatewayError, joinClassNames } from '../utils/gatewayFormatters';
import {
  DEFAULT_GATEWAY_PATH,
  GATEWAY_TABS,
  getGatewayPathForTab,
  resolveGatewayTabFromPath,
  type GatewayPageTab,
} from '../utils/gatewayNavigation';
import styles from './GatewayPage.module.less';

type GatewayAction = 'load' | 'start' | 'stop' | 'restart' | 'health';
type GatewayNoticeKind = 'success' | 'error';

const cloneGatewaySettings = (settings: ProxyGatewaySettings): ProxyGatewaySettings => ({
  ...settings,
  enabled_cli_keys: [...settings.enabled_cli_keys],
  app_configs: Object.fromEntries(
    Object.entries(settings.app_configs ?? {}).map(([cliKey, config]) => [
      cliKey,
      { ...config },
    ]),
  ),
});

const GatewayPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { isActive } = useKeepAlive();
  const activeTab = resolveGatewayTabFromPath(location.pathname);
  const [status, setStatus] = React.useState<ProxyGatewayStatus | null>(null);
  const [busyAction, setBusyAction] = React.useState<GatewayAction | null>('load');
  const [notice, setNotice] = React.useState<{ kind: GatewayNoticeKind; text: string } | null>(null);
  const [tabRefreshKeys, setTabRefreshKeys] = React.useState<Record<GatewayPageTab, number>>({
    statistics: 0,
    requests: 0,
    settings: 0,
  });
  const settingsDraftRef = React.useRef<ProxyGatewaySettings | null>(null);
  const usageRefreshTimerRef = React.useRef<number | null>(null);
  const statusRevisionRef = React.useRef(0);
  const [statusRefreshKey, setStatusRefreshKey] = React.useState(0);
  const [documentVisible, setDocumentVisible] = React.useState(() => document.visibilityState !== 'hidden');

  // The statistics/requests filter bars stick right below this header, so the
  // offset they need is the header's measured height rather than a constant:
  // it grows when the controls wrap onto a second row or the subtitle wraps.
  const headerRef = React.useRef<HTMLDivElement | null>(null);
  const [headerHeight, setHeaderHeight] = React.useState(64);

  React.useLayoutEffect(() => {
    const node = headerRef.current;
    if (!node) return;
    const syncHeaderHeight = () => {
      // Round down: a bar tucked a hairline under the header is invisible,
      // while the opposite rounding opens a gap the list scrolls through.
      const height = Math.floor(node.getBoundingClientRect().height);
      // KeepAlive hides visited pages with `display: none`, which measures 0
      // — hold the last real height instead of pinning the bars at the top.
      if (height > 0) setHeaderHeight(height);
    };
    syncHeaderHeight();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(syncHeaderHeight);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const handleStatusChange = React.useCallback((nextStatus: ProxyGatewayStatus) => {
    // An authoritative command result must not be overwritten by an older poll.
    statusRevisionRef.current += 1;
    setStatus(nextStatus);
  }, []);

  React.useEffect(() => {
    if (location.pathname === '/gateway') {
      navigate(DEFAULT_GATEWAY_PATH, { replace: true });
    }
  }, [location.pathname, navigate]);

  React.useEffect(() => {
    let disposed = false;

    const loadGatewayState = async () => {
      setBusyAction('load');
      const revision = statusRevisionRef.current;
      try {
        const nextStatus = await getProxyGatewayStatus();
        if (!disposed && revision === statusRevisionRef.current) {
          handleStatusChange(nextStatus);
        }
      } catch (error) {
        if (!disposed) {
          setNotice({
            kind: 'error',
            text: t('settings.gateway.notice.loadFailed', { error: formatGatewayError(error) }),
          });
        }
      } finally {
        if (!disposed) {
          setBusyAction(null);
        }
      }
    };

    void loadGatewayState();

    return () => {
      disposed = true;
    };
  }, [handleStatusChange, t]);

  React.useEffect(() => {
    const onVisibilityChange = () => {
      statusRevisionRef.current += 1;
      setDocumentVisible(document.visibilityState !== 'hidden');
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => document.removeEventListener('visibilitychange', onVisibilityChange);
  }, []);

  React.useEffect(() => {
    if (!isActive || !documentVisible || busyAction) {
      return undefined;
    }
    let disposed = false;
    let refreshing = false;
    const isPageVisible = () => document.visibilityState !== 'hidden';
    const refreshStatus = async () => {
      if (disposed || refreshing || !isPageVisible()) {
        return;
      }
      refreshing = true;
      const revision = statusRevisionRef.current;
      try {
        const nextStatus = await getProxyGatewayStatus();
        if (!disposed && revision === statusRevisionRef.current && isPageVisible()) {
          setStatus(nextStatus);
        }
      } catch {
        // Keep the last known status on a transient refresh failure.
      } finally {
        refreshing = false;
      }
    };
    const timer = status?.running
      ? window.setInterval(() => void refreshStatus(), 5000)
      : null;
    void refreshStatus();
    return () => {
      disposed = true;
      if (timer !== null) {
        window.clearInterval(timer);
      }
    };
  }, [busyAction, documentVisible, isActive, status?.running, statusRefreshKey]);

  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen('gateway-running-changed', () => {
      if (!disposed) {
        statusRevisionRef.current += 1;
        setStatusRefreshKey((current) => current + 1);
      }
    }).then((dispose) => {
      if (disposed) {
        dispose();
      } else {
        unlisten = dispose;
      }
    }).catch(() => {
      // Status still refreshes when the page becomes visible.
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  React.useEffect(() => {
    if (notice?.kind !== 'success') {
      return undefined;
    }

    const noticeTimer = window.setTimeout(() => {
      setNotice((currentNotice) =>
        currentNotice?.kind === notice.kind && currentNotice.text === notice.text
          ? null
          : currentNotice,
      );
    }, 2400);

    return () => {
      window.clearTimeout(noticeTimer);
    };
  }, [notice]);

  const handleTabChange = (tabKey: GatewayPageTab) => {
    navigate(getGatewayPathForTab(tabKey));
  };

  const bumpTabRefreshKey = React.useCallback((tabKey: GatewayPageTab) => {
    setTabRefreshKeys((currentKeys) => ({
      ...currentKeys,
      [tabKey]: currentKeys[tabKey] + 1,
    }));
  }, []);

  const bumpUsageRefreshKeys = React.useCallback(() => {
    setTabRefreshKeys((currentKeys) => ({
      ...currentKeys,
      statistics: currentKeys.statistics + 1,
      requests: currentKeys.requests + 1,
    }));
  }, []);

  const scheduleUsageRefresh = React.useCallback(() => {
    if (usageRefreshTimerRef.current !== null) {
      return;
    }

    usageRefreshTimerRef.current = window.setTimeout(() => {
      usageRefreshTimerRef.current = null;
      bumpUsageRefreshKeys();
    }, 300);
  }, [bumpUsageRefreshKeys]);

  React.useEffect(() => {
    if (!isActive || !documentVisible) {
      return undefined;
    }
    let disposed = false;
    void importProxyGatewaySessionUsage({ cli_key: 'all' }).then(() => {
      if (!disposed) {
        scheduleUsageRefresh();
      }
    }).catch((error) => {
      // The background scheduler will retry; manual sync exposes the error.
      console.warn('Failed to refresh local session usage', error);
    });
    return () => { disposed = true; };
  }, [documentVisible, isActive, scheduleUsageRefresh]);

  React.useEffect(() => () => {
    if (usageRefreshTimerRef.current !== null) {
      window.clearTimeout(usageRefreshTimerRef.current);
      usageRefreshTimerRef.current = null;
    }
  }, []);

  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen('gateway-failover', () => {
      setStatus((currentStatus) => currentStatus ? { ...currentStatus } : currentStatus);
      bumpTabRefreshKey(activeTab);
    }).then((dispose) => {
      if (disposed) {
        dispose();
        return;
      }
      unlisten = dispose;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [activeTab, bumpTabRefreshKey]);

  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen('usage-log-recorded', scheduleUsageRefresh).then((dispose) => {
      if (disposed) {
        dispose();
        return;
      }
      unlisten = dispose;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [scheduleUsageRefresh]);

  const handleSettingsDraftChange = React.useCallback((settings: ProxyGatewaySettings | null) => {
    settingsDraftRef.current = settings ? cloneGatewaySettings(settings) : null;
  }, []);

  const handleStart = async () => {
    statusRevisionRef.current += 1;
    setBusyAction('start');
    try {
      const settings = settingsDraftRef.current
        ? cloneGatewaySettings(settingsDraftRef.current)
        : await getProxyGatewaySettings();
      const nextSettings = await updateProxyGatewaySettings({
        ...settings,
        enabled_on_startup: false,
      });
      const nextStatus = await startProxyGateway(nextSettings);
      handleStatusChange(nextStatus);
      bumpTabRefreshKey(activeTab);
      setNotice({ kind: 'success', text: t('settings.gateway.notice.started') });
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('settings.gateway.notice.startFailed', { error: formatGatewayError(error) }),
      });
      try {
        handleStatusChange(await getProxyGatewayStatus());
      } catch {
        // Best effort refresh only.
      }
    } finally {
      setBusyAction(null);
    }
  };

  const handleStop = async () => {
    statusRevisionRef.current += 1;
    setBusyAction('stop');
    try {
      const preflight = await preflightStopProxyGateway();
      if (!preflight.allowed) {
        const blockingNames = preflight.blocking_cli_takeovers
          .map((cliStatus) => t(`settings.gateway.cli.${cliStatus.cli_key}`))
          .join(', ');
        setNotice({
          kind: 'error',
          text: t('settings.gateway.notice.stopBlockedByCli', { cli: blockingNames || '-' }),
        });
        return;
      }
      const nextStatus = await stopProxyGateway();
      handleStatusChange(nextStatus);
      bumpTabRefreshKey(activeTab);
      setNotice({ kind: 'success', text: t('settings.gateway.notice.stopped') });
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('settings.gateway.notice.stopFailed', { error: formatGatewayError(error) }),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const handleRestart = async () => {
    statusRevisionRef.current += 1;
    setBusyAction('restart');
    try {
      const nextStatus = await restartProxyGateway();
      handleStatusChange(nextStatus);
      bumpTabRefreshKey(activeTab);
      setNotice({ kind: 'success', text: t('settings.gateway.notice.restarted') });
    } catch (error) {
      const errorText = formatGatewayError(error);
      let nextStatus: ProxyGatewayStatus | null = null;
      try {
        nextStatus = await getProxyGatewayStatus();
        handleStatusChange(nextStatus);
      } catch {
        // Best effort refresh only.
      }
      // Backend already returns a full "now stopped" message after mid-restart
      // failure; avoid wrapping it again with restartFailedStopped.
      setNotice({
        kind: 'error',
        text: nextStatus && !nextStatus.running
          ? errorText
          : t('settings.gateway.notice.restartFailed', { error: errorText }),
      });
    } finally {
      setBusyAction(null);
    }
  };

  const powerMenuItems = React.useMemo<MenuProps['items']>(
    () => [
      {
        key: 'restart',
        icon: <RotateCcw size={14} aria-hidden="true" />,
        label: t('settings.gateway.actions.restart'),
        disabled: Boolean(busyAction),
      },
      {
        key: 'stop',
        icon: <Square size={14} aria-hidden="true" />,
        label: t('settings.gateway.actions.stop'),
        disabled: Boolean(busyAction),
        danger: true,
      },
    ],
    [busyAction, t],
  );

  const handlePowerMenuClick: MenuProps['onClick'] = ({ key }) => {
    if (key === 'restart') {
      void handleRestart();
      return;
    }
    if (key === 'stop') {
      void handleStop();
    }
  };

  const handleHealthCheck = async () => {
    setBusyAction('health');
    try {
      const nextHealth = await checkProxyGatewayHealth();
      if (activeTab === 'statistics') {
        bumpTabRefreshKey('statistics');
      }
      setNotice({
        kind: nextHealth.ok ? 'success' : 'error',
        text: nextHealth.ok
          ? t('settings.gateway.notice.healthOk', { statusCode: nextHealth.status_code ?? '-' })
          : t('settings.gateway.notice.healthFailed', { error: nextHealth.error ?? '-' }),
      });
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('settings.gateway.notice.healthFailed', { error: formatGatewayError(error) }),
      });
    } finally {
      setBusyAction(null);
    }
  };

  return (
    <div
      className={styles.gatewayPage}
      style={{ ['--gateway-header-height' as any]: `${headerHeight}px` }}
    >
      <div className={styles.header} ref={headerRef}>
        <div className={styles.titleBlock}>
          <span className={styles.titleIcon}>
            <Network size={18} aria-hidden="true" />
          </span>
          <div>
            <h1>{t('gateway.page.title')}</h1>
            <p>{t('gateway.page.subtitle')}</p>
          </div>
        </div>
        <div className={styles.headerControls}>
          <div className={styles.statusPill} title={status?.base_url ?? status?.listen_host ?? ''}>
            <span
              className={joinClassNames(styles.statusDot, status?.running && styles.statusDotRunning)}
              aria-hidden="true"
            />
            <span>{status?.running ? t('settings.gateway.status.running') : t('settings.gateway.status.stopped')}</span>
            <strong>{t('settings.gateway.status.activeConnections', { count: status?.active_connections ?? 0 })}</strong>
          </div>
          <div className={styles.actionBar}>
            {status?.running ? (
              <Dropdown
                trigger={['click']}
                disabled={Boolean(busyAction)}
                menu={{
                  items: powerMenuItems,
                  onClick: handlePowerMenuClick,
                }}
              >
                <button
                  type="button"
                  className={styles.actionButton}
                  disabled={Boolean(busyAction)}
                  aria-label={t('settings.gateway.actions.power')}
                  title={t('settings.gateway.actions.power')}
                >
                  {busyAction === 'restart' || busyAction === 'stop' ? (
                    <Loader2 size={15} className={styles.spin} aria-hidden="true" />
                  ) : (
                    <Power size={15} aria-hidden="true" />
                  )}
                  <span>{t('settings.gateway.actions.power')}</span>
                  <ChevronDown size={14} aria-hidden="true" />
                </button>
              </Dropdown>
            ) : (
              <button
                type="button"
                className={joinClassNames(styles.actionButton, styles.actionButtonPrimary)}
                disabled={Boolean(busyAction)}
                aria-label={t('settings.gateway.actions.start')}
                title={t('settings.gateway.actions.start')}
                onClick={() => void handleStart()}
              >
                {busyAction === 'start' ? (
                  <Loader2 size={15} className={styles.spin} aria-hidden="true" />
                ) : (
                  <Power size={15} aria-hidden="true" />
                )}
                <span>{t('settings.gateway.actions.start')}</span>
              </button>
            )}
            <button
              type="button"
              className={styles.actionButton}
              disabled={Boolean(busyAction)}
              aria-label={t('settings.gateway.actions.health')}
              title={t('settings.gateway.actions.health')}
              onClick={() => void handleHealthCheck()}
            >
              {busyAction === 'health' ? (
                <Loader2 size={15} className={styles.spin} aria-hidden="true" />
              ) : (
                <Activity size={15} aria-hidden="true" />
              )}
              <span>{t('settings.gateway.actions.health')}</span>
            </button>
          </div>
          <span className={styles.toolbarDivider} aria-hidden="true" />
          <div className={styles.tabList} role="tablist" aria-label={t('gateway.page.title')}>
            {GATEWAY_TABS.map((tab) => (
              <button
                key={tab.key}
                type="button"
                role="tab"
                aria-selected={activeTab === tab.key}
                className={joinClassNames(styles.tabButton, activeTab === tab.key && styles.tabButtonActive)}
                onClick={() => handleTabChange(tab.key)}
              >
                {tab.key === 'statistics' ? <BarChart3 size={14} aria-hidden="true" /> : null}
                {tab.key === 'requests' ? <FileText size={14} aria-hidden="true" /> : null}
                {tab.key === 'settings' ? <Settings size={14} aria-hidden="true" /> : null}
                <span>{t(tab.labelKey)}</span>
              </button>
            ))}
          </div>
        </div>
      </div>

      {notice ? (
        <div className={joinClassNames(styles.notice, styles[`notice_${notice.kind}`])} role="status" aria-live="polite">
          {notice.text}
        </div>
      ) : null}
      {status?.last_error ? (
        <div className={joinClassNames(styles.notice, styles.notice_error)} role="alert">
          {status.last_error}
        </div>
      ) : null}

      {activeTab === 'statistics' ? (
        <GatewayStatisticsView
          refreshKey={tabRefreshKeys.statistics}
          gatewayStatus={status}
        />
      ) : null}
      {activeTab === 'requests' ? <GatewayRequestsView refreshKey={tabRefreshKeys.requests} /> : null}
      {activeTab === 'settings' ? (
        <GatewaySettingsPanel
          key={`settings-${tabRefreshKeys.settings}`}
          showTitleBlock={false}
          onStatusChange={handleStatusChange}
          onDraftSettingsChange={handleSettingsDraftChange}
        />
      ) : null}
    </div>
  );
};

export default GatewayPage;
