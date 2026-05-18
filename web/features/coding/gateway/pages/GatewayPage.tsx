import React from 'react';
import {
  Activity,
  BarChart3,
  FileText,
  Loader2,
  Network,
  Power,
  RefreshCw,
  Settings,
  Square,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import GatewaySettingsPanel from '@/features/settings/pages/GatewaySettingsPanel';
import {
  checkProxyGatewayHealth,
  getProxyGatewaySettings,
  getProxyGatewayStatus,
  preflightStopProxyGateway,
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

type GatewayAction = 'load' | 'start' | 'stop' | 'health' | 'refresh';
type GatewayNoticeKind = 'success' | 'error';

const cloneGatewaySettings = (settings: ProxyGatewaySettings): ProxyGatewaySettings => ({
  ...settings,
  enabled_cli_keys: [...settings.enabled_cli_keys],
});

const GatewayPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
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

  React.useEffect(() => {
    if (location.pathname === '/gateway') {
      navigate(DEFAULT_GATEWAY_PATH, { replace: true });
    }
  }, [location.pathname, navigate]);

  React.useEffect(() => {
    let disposed = false;

    const loadGatewayState = async () => {
      setBusyAction('load');
      try {
        const nextStatus = await getProxyGatewayStatus();
        if (!disposed) {
          setStatus(nextStatus);
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
  }, [t]);

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

  const handleSettingsDraftChange = React.useCallback((settings: ProxyGatewaySettings | null) => {
    settingsDraftRef.current = settings ? cloneGatewaySettings(settings) : null;
  }, []);

  const handleStart = async () => {
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
      setStatus(nextStatus);
      bumpTabRefreshKey(activeTab);
      setNotice({ kind: 'success', text: t('settings.gateway.notice.started') });
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('settings.gateway.notice.startFailed', { error: formatGatewayError(error) }),
      });
      try {
        setStatus(await getProxyGatewayStatus());
      } catch {
        // Best effort refresh only.
      }
    } finally {
      setBusyAction(null);
    }
  };

  const handleStop = async () => {
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
      setStatus(nextStatus);
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

  const handleRefresh = async () => {
    const refreshTab = activeTab;
    setBusyAction('refresh');
    try {
      setStatus(await getProxyGatewayStatus());
      bumpTabRefreshKey(refreshTab);
      setNotice({ kind: 'success', text: t('settings.gateway.notice.refreshed') });
    } catch (error) {
      bumpTabRefreshKey(refreshTab);
      setNotice({
        kind: 'error',
        text: t('settings.gateway.notice.loadFailed', { error: formatGatewayError(error) }),
      });
    } finally {
      setBusyAction(null);
    }
  };

  return (
    <div className={styles.gatewayPage}>
      <div className={styles.header}>
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
          <div className={styles.actionBar}>
            {status?.running ? (
              <button
                type="button"
                className={styles.actionButton}
                disabled={Boolean(busyAction)}
                aria-label={t('settings.gateway.actions.stop')}
                title={t('settings.gateway.actions.stop')}
                onClick={() => void handleStop()}
              >
                {busyAction === 'stop' ? (
                  <Loader2 size={15} className={styles.spin} aria-hidden="true" />
                ) : (
                  <Square size={14} aria-hidden="true" />
                )}
              </button>
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
            </button>
            <button
              type="button"
              className={styles.actionButton}
              disabled={Boolean(busyAction)}
              aria-label={t('common.refresh')}
              title={t('common.refresh')}
              onClick={() => void handleRefresh()}
            >
              {busyAction === 'refresh' || busyAction === 'load' ? (
                <Loader2 size={15} className={styles.spin} aria-hidden="true" />
              ) : (
                <RefreshCw size={15} aria-hidden="true" />
              )}
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

      {activeTab === 'statistics' ? <GatewayStatisticsView refreshKey={tabRefreshKeys.statistics} /> : null}
      {activeTab === 'requests' ? <GatewayRequestsView refreshKey={tabRefreshKeys.requests} /> : null}
      {activeTab === 'settings' ? (
        <GatewaySettingsPanel
          key={`settings-${tabRefreshKeys.settings}`}
          showTitleBlock={false}
          onStatusChange={setStatus}
          onDraftSettingsChange={handleSettingsDraftChange}
        />
      ) : null}
    </div>
  );
};

export default GatewayPage;
