import React from 'react';
import { App, Input, Switch, Tooltip } from 'antd';
import {
  Activity,
  Copy,
  Cpu,
  Download,
  Loader2,
  RefreshCw,
  RotateCcw,
  Save,
  ShieldCheck,
  Trash2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import { copyTextToClipboard } from '@/services/clipboardApi';
import {
  clearPluginRuntimeLogs,
  ensurePluginBun,
  getPluginBunStatus,
  getPluginHostStatus,
  getPluginMiddlewareStates,
  getPluginRuntimeLogs,
  getPluginSettings,
  probePluginBunPath,
  restartPluginHost,
  savePluginSettings,
  type BunHostStatus,
  type BunPathProbe,
  type BunRuntimeStatus,
  type MiddlewareStateView,
  type PluginRuntimeLogEntry,
  type PluginSettings,
} from '@/services';
import {
  formatMicroseconds,
  formatPluginError,
  isChinaMirrorRegistry,
  redactPluginLog,
} from '../utils/pluginFormatters';
import styles from './PluginRuntimeView.module.less';

type LogFilter = 'all' | 'info' | 'warn' | 'error';

interface PluginRuntimeViewProps {
  refreshKey: number;
  pluginSettings: PluginSettings | null;
  chinaMirrorBusy: boolean;
  onToggleChinaMirror: (enabled: boolean) => void;
  onSettingsSaved: (settings: PluginSettings) => void;
  onBunStatusChange: (status: BunRuntimeStatus) => void;
}

const DEFAULT_PLUGIN_SETTINGS: PluginSettings = {
  bunManualPath: '',
  npmRegistry: 'https://registry.npmjs.org',
  followAppProxy: true,
  providerAutoCheckin: {},
};

const redactLogEntries = (entries: PluginRuntimeLogEntry[]): PluginRuntimeLogEntry[] =>
  entries.map((entry) => ({ ...entry, message: redactPluginLog(entry.message) }));

const PluginRuntimeView: React.FC<PluginRuntimeViewProps> = ({
  refreshKey,
  pluginSettings,
  chinaMirrorBusy,
  onToggleChinaMirror,
  onSettingsSaved,
  onBunStatusChange,
}) => {
  const { t, i18n } = useTranslation();
  const { message, modal } = App.useApp();
  const { isActive } = useKeepAlive();

  const [bunStatus, setBunStatus] = React.useState<BunRuntimeStatus | null>(null);
  const [hostStatus, setHostStatus] = React.useState<BunHostStatus | null>(null);
  const [settings, setSettings] = React.useState<PluginSettings | null>(null);
  const [draft, setDraft] = React.useState<PluginSettings>(DEFAULT_PLUGIN_SETTINGS);
  const [middlewareStates, setMiddlewareStates] = React.useState<MiddlewareStateView[]>([]);
  const [logs, setLogs] = React.useState<PluginRuntimeLogEntry[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [ensureBusy, setEnsureBusy] = React.useState(false);
  const [probeBusy, setProbeBusy] = React.useState(false);
  const [probeResult, setProbeResult] = React.useState<BunPathProbe | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [hostBusy, setHostBusy] = React.useState(false);
  const [logsBusy, setLogsBusy] = React.useState(false);
  const [logFilter, setLogFilter] = React.useState<LogFilter>('all');

  const loadAll = React.useCallback(async () => {
    setLoading(true);
    const [bunResult, hostResult, settingsResult, middlewareResult, logsResult] =
      await Promise.allSettled([
        getPluginBunStatus(),
        getPluginHostStatus(),
        getPluginSettings(),
        getPluginMiddlewareStates(),
        getPluginRuntimeLogs(300),
      ]);
    let firstError: string | null = null;

    if (bunResult.status === 'fulfilled') {
      setBunStatus(bunResult.value);
      onBunStatusChange(bunResult.value);
    } else {
      firstError = formatPluginError(bunResult.reason);
    }
    if (hostResult.status === 'fulfilled') {
      setHostStatus(hostResult.value);
    } else if (!firstError) {
      firstError = formatPluginError(hostResult.reason);
    }
    if (settingsResult.status === 'fulfilled') {
      setSettings(settingsResult.value);
      setDraft(settingsResult.value);
    } else if (!firstError) {
      firstError = formatPluginError(settingsResult.reason);
    }
    if (middlewareResult.status === 'fulfilled') {
      setMiddlewareStates(middlewareResult.value);
    }
    if (logsResult.status === 'fulfilled') {
      setLogs(redactLogEntries(logsResult.value));
    }
    setError(firstError);
    setLoading(false);
  }, [onBunStatusChange]);

  React.useEffect(() => {
    void loadAll();
  }, [loadAll, refreshKey]);

  // The page-level mirror switch writes settings immediately; keep this
  // page's draft registry in step even while the form has unrelated edits.
  React.useEffect(() => {
    if (!pluginSettings) {
      return;
    }
    setDraft((current) =>
      current.npmRegistry === pluginSettings.npmRegistry
        ? current
        : { ...current, npmRegistry: pluginSettings.npmRegistry });
    setSettings((current) =>
      current && current.npmRegistry !== pluginSettings.npmRegistry
        ? { ...current, npmRegistry: pluginSettings.npmRegistry }
        : current);
  }, [pluginSettings]);

  React.useEffect(() => {
    if (!isActive) {
      return undefined;
    }
    let disposed = false;
    const tick = async () => {
      if (document.visibilityState === 'hidden') return;
      const [hostResult, bunResult, logsResult] = await Promise.allSettled([
        getPluginHostStatus(),
        getPluginBunStatus(),
        getPluginRuntimeLogs(300),
      ]);
      if (disposed) return;
      if (hostResult.status === 'fulfilled') setHostStatus(hostResult.value);
      if (bunResult.status === 'fulfilled') {
        setBunStatus(bunResult.value);
        onBunStatusChange(bunResult.value);
      }
      if (logsResult.status === 'fulfilled') setLogs(redactLogEntries(logsResult.value));
    };
    const timer = window.setInterval(() => void tick(), 5000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [isActive, onBunStatusChange]);

  const settingsDirty = Boolean(
    settings
    && (
      draft.bunManualPath !== settings.bunManualPath
      || draft.npmRegistry !== settings.npmRegistry
      || draft.followAppProxy !== settings.followAppProxy
    ),
  );

  const updateDraft = (patch: Partial<PluginSettings>) => {
    setDraft((current) => ({ ...current, ...patch }));
  };

  const handleBunPathChange = (value: string) => {
    updateDraft({ bunManualPath: value });
    // The probe verdict belongs to the previous path; never show it for new input.
    setProbeResult(null);
  };

  const handleEnsureBun = async () => {
    setEnsureBusy(true);
    try {
      const nextStatus = await ensurePluginBun();
      setBunStatus(nextStatus);
      onBunStatusChange(nextStatus);
      if (nextStatus.available) {
        message.success(
          t('plugins.runtime.ensureSuccess', {
            version: nextStatus.version ?? nextStatus.expectedVersion,
          }),
        );
      } else {
        message.error(
          t('plugins.runtime.ensureFailed', {
            error: nextStatus.lastError ?? t('common.unknown'),
          }),
        );
      }
    } catch (ensureError) {
      message.error(
        t('plugins.runtime.ensureFailed', { error: formatPluginError(ensureError) }),
      );
    } finally {
      setEnsureBusy(false);
    }
  };

  const handleRecheckBun = async () => {
    try {
      const nextStatus = await getPluginBunStatus();
      setBunStatus(nextStatus);
      onBunStatusChange(nextStatus);
    } catch (checkError) {
      message.error(formatPluginError(checkError));
    }
  };

  const handleProbePath = async () => {
    const path = draft.bunManualPath.trim();
    if (!path) {
      setProbeResult({ ok: false, version: null, error: t('plugins.runtime.pathRequired') });
      return;
    }
    setProbeBusy(true);
    setProbeResult(null);
    try {
      const probe = await probePluginBunPath(path);
      setProbeResult(probe);
    } catch (probeError) {
      setProbeResult({ ok: false, version: null, error: formatPluginError(probeError) });
    } finally {
      setProbeBusy(false);
    }
  };

  const handleSaveSettings = async () => {
    setSaving(true);
    try {
      const saved = await savePluginSettings(draft);
      setSettings(saved);
      setDraft(saved);
      onSettingsSaved(saved);
      message.success(t('plugins.runtime.settingsSaved'));
      const nextBunStatus = await getPluginBunStatus();
      setBunStatus(nextBunStatus);
      onBunStatusChange(nextBunStatus);
    } catch (saveError) {
      message.error(
        t('plugins.runtime.settingsSaveFailed', {
          error: formatPluginError(saveError),
        }),
      );
    } finally {
      setSaving(false);
    }
  };

  const handleRestartHost = async () => {
    setHostBusy(true);
    try {
      const nextStatus = await restartPluginHost();
      setHostStatus(nextStatus);
      message.success(t('plugins.runtime.hostRestartSuccess'));
    } catch (restartError) {
      message.error(
        t('plugins.runtime.hostRestartFailed', {
          error: formatPluginError(restartError),
        }),
      );
    } finally {
      setHostBusy(false);
    }
  };

  const handleRefreshLogs = async () => {
    setLogsBusy(true);
    try {
      setLogs(redactLogEntries(await getPluginRuntimeLogs(300)));
    } catch (logsError) {
      message.error(
        t('plugins.runtime.logsLoadFailed', { error: formatPluginError(logsError) }),
      );
    } finally {
      setLogsBusy(false);
    }
  };

  const handleClearLogs = () => {
    modal.confirm({
      title: t('plugins.runtime.logsClearConfirmTitle'),
      content: t('plugins.runtime.logsClearConfirmContent'),
      okText: t('common.clear'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await clearPluginRuntimeLogs();
          setLogs([]);
          message.success(t('plugins.runtime.logsCleared'));
        } catch (clearError) {
          message.error(formatPluginError(clearError));
        }
      },
    });
  };

  const filteredLogs = React.useMemo(() => {
    if (logFilter === 'all') return logs;
    if (logFilter === 'info') {
      return logs.filter((entry) => entry.level === 'info' || entry.level === 'debug');
    }
    return logs.filter((entry) => entry.level === logFilter);
  }, [logFilter, logs]);

  const handleCopyLogs = async () => {
    const text = filteredLogs
      .map((entry) => `[${formatLogTime(entry.at)}] [${entry.level}] [${entry.source}] ${entry.message}`)
      .join('\n');
    try {
      await copyTextToClipboard(text);
      message.success(t('plugins.runtime.logsCopied'));
    } catch (copyError) {
      message.error(
        t('plugins.runtime.logsCopyFailed', { error: formatPluginError(copyError) }),
      );
    }
  };

  function formatLogTime(value: string) {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return '';
    return date.toLocaleTimeString(i18n.language, { hour12: false });
  }

  const bunSourceLabel = (source: string | null | undefined) => {
    if (source === 'manual') return t('plugins.runtime.bunSourceManual');
    if (source === 'managed') return t('plugins.runtime.bunSourceManaged');
    if (source === 'system') return t('plugins.runtime.bunSourceSystem');
    return t('plugins.runtime.bunSourceNone');
  };

  const logLevelClass = (level: string) => {
    if (level === 'error') return styles.logLevelError;
    if (level === 'warn') return styles.logLevelWarn;
    return styles.logLevelInfo;
  };

  const logFilterLabels: Record<LogFilter, string> = {
    all: t('plugins.runtime.logsFilter.all'),
    info: t('plugins.runtime.logsFilter.info'),
    warn: t('plugins.runtime.logsFilter.warn'),
    error: t('plugins.runtime.logsFilter.error'),
  };

  const eventsLabel = (state: MiddlewareStateView) => {
    if (state.events == null) {
      return t('plugins.runtime.eventsAll');
    }
    return t('plugins.runtime.eventsCount', { count: state.events.length });
  };

  if (loading && !bunStatus && !hostStatus) {
    return (
      <div className={styles.loading}>
        <Loader2 size={16} className={styles.spin} aria-hidden="true" />
        <span>{t('common.loading')}</span>
      </div>
    );
  }

  return (
    <div className={styles.view}>
      {error ? (
        <div className={styles.errorNotice} role="alert">{error}</div>
      ) : null}

      <div className={styles.runtimeCard}>
        <div className={styles.runtimeTop}>
          <span className={styles.runtimeIcon}>
            <Cpu size={18} aria-hidden="true" />
          </span>
          <div className={styles.runtimeInfo}>
            <span className={styles.runtimeTitle}>{t('plugins.runtime.bunTitle')}</span>
            <span
              className={styles.runtimePath}
              title={bunStatus?.path || bunStatus?.managedPath || undefined}
            >
              {bunStatus?.path || bunStatus?.managedPath || t('plugins.runtime.bunMissing')}
            </span>
            <div className={styles.runtimeMeta}>
              <span>
                {t('plugins.runtime.bunSource')}: {bunSourceLabel(bunStatus?.source)}
              </span>
              {bunStatus?.version ? (
                <span>{t('plugins.runtime.bunVersion')}: {bunStatus.version}</span>
              ) : null}
              <span>
                {t('plugins.runtime.bunExpected')}: {bunStatus?.expectedVersion ?? '—'}
              </span>
              {bunStatus?.downloading ? (
                <span className={styles.runtimeWarning}>
                  {t('plugins.runtime.bunDownloading')}
                </span>
              ) : null}
            </div>
          </div>
          <div className={styles.runtimeActions}>
            <button
              type="button"
              className={styles.actionButton}
              disabled={loading}
              onClick={() => void loadAll()}
            >
              <RefreshCw size={14} aria-hidden="true" />
              <span>{t('plugins.runtime.refreshAll')}</span>
            </button>
            <button
              type="button"
              className={styles.actionButton}
              onClick={() => void handleRecheckBun()}
            >
              <ShieldCheck size={14} aria-hidden="true" />
              <span>{t('plugins.runtime.recheck')}</span>
            </button>
            {bunStatus?.available ? null : (
              <button
                type="button"
                className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                disabled={ensureBusy || bunStatus?.downloading}
                onClick={() => void handleEnsureBun()}
              >
                {ensureBusy || bunStatus?.downloading ? (
                  <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                ) : (
                  <Download size={14} aria-hidden="true" />
                )}
                <span>{t('plugins.runtime.download')}</span>
              </button>
            )}
          </div>
        </div>

        {bunStatus?.lastError ? (
          <div className={styles.inlineError}>{bunStatus.lastError}</div>
        ) : null}

        <div className={styles.divider} />

        <div className={styles.hostRow}>
          <span
            className={`${styles.statusDot} ${hostStatus?.running ? styles.statusOk : styles.statusMuted}`}
            aria-hidden="true"
          />
          <span className={styles.hostName}>{t('plugins.runtime.hostTitle')}</span>
          <span>{hostStatus?.running ? t('plugins.runtime.hostRunning') : t('plugins.runtime.hostStopped')}</span>
          {hostStatus?.running && hostStatus.pid != null ? (
            <span className={styles.hostMeta}>
              {t('plugins.runtime.hostPid', { pid: hostStatus.pid })}
            </span>
          ) : null}
          {hostStatus ? (
            <>
              <span className={styles.hostMeta}>
                {t('plugins.runtime.hostPending', { count: hostStatus.pendingRequests })}
              </span>
              <span className={styles.hostMeta}>
                {t('plugins.runtime.hostCrashes', { count: hostStatus.crashes })}
              </span>
            </>
          ) : null}
          {hostStatus?.circuitOpen ? (
            <span className={styles.runtimeError}>{t('plugins.runtime.hostCircuit')}</span>
          ) : null}
          {hostStatus && hostStatus.retryInMillis > 0 ? (
            <span className={styles.runtimeWarning}>
              {t('plugins.runtime.hostRetry', { ms: hostStatus.retryInMillis })}
            </span>
          ) : null}
          <button
            type="button"
            className={styles.hostRestartButton}
            disabled={hostBusy}
            onClick={() => void handleRestartHost()}
          >
            {hostBusy ? (
              <Loader2 size={14} className={styles.spin} aria-hidden="true" />
            ) : (
              <RotateCcw size={14} aria-hidden="true" />
            )}
            <span>{t('plugins.runtime.hostRestart')}</span>
          </button>
        </div>

        {hostStatus?.lastError ? (
          <div className={styles.inlineError}>{hostStatus.lastError}</div>
        ) : null}
      </div>

      <div className={styles.settingsCard}>
        <div className={styles.settingsHeader}>
          <h3>{t('plugins.runtime.settingsTitle')}</h3>
          <span className={styles.settingsHint}>{t('plugins.runtime.settingsHint')}</span>
        </div>

        <div className={styles.settingRows}>
          <div className={styles.settingRow}>
            <div className={styles.settingInfo}>
              <span className={styles.settingTitle}>{t('plugins.runtime.chinaMirror')}</span>
              <span className={styles.settingDesc}>
                {t('plugins.runtime.chinaMirrorHint')}
              </span>
            </div>
            <div className={styles.settingControl}>
              <Switch
                size="small"
                checked={isChinaMirrorRegistry(draft.npmRegistry)}
                loading={chinaMirrorBusy}
                disabled={chinaMirrorBusy || !pluginSettings}
                aria-label={t('plugins.runtime.chinaMirror')}
                onChange={onToggleChinaMirror}
              />
            </div>
          </div>

          <div className={styles.settingRow}>
            <div className={styles.settingInfoNarrow}>
              <span className={styles.settingTitle}>{t('plugins.runtime.npmRegistry')}</span>
              <span className={styles.settingDesc}>
                {t('plugins.runtime.npmRegistryHint')}
              </span>
            </div>
            <div className={styles.settingField}>
              <Input
                value={draft.npmRegistry}
                onChange={(event) => updateDraft({ npmRegistry: event.target.value })}
                placeholder="https://registry.npmjs.org"
                className={styles.settingInput}
              />
            </div>
          </div>

          <div className={styles.settingRow}>
            <div className={styles.settingInfoNarrow}>
              <span className={styles.settingTitle}>{t('plugins.runtime.bunManualPath')}</span>
              <span className={styles.settingDesc}>
                {t('plugins.runtime.bunManualHint')}
              </span>
            </div>
            <div className={styles.settingField}>
              <div className={styles.settingFieldRow}>
                <Input
                  value={draft.bunManualPath}
                  onChange={(event) => handleBunPathChange(event.target.value)}
                  placeholder={t('plugins.runtime.bunManualPlaceholder')}
                  className={styles.settingInput}
                />
                <button
                  type="button"
                  className={styles.actionButton}
                  disabled={probeBusy}
                  onClick={() => void handleProbePath()}
                >
                  {probeBusy ? (
                    <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                  ) : (
                    <ShieldCheck size={14} aria-hidden="true" />
                  )}
                  <span>{t('plugins.runtime.probe')}</span>
                </button>
              </div>
              {probeResult ? (
                <div
                  className={`${styles.probeResult} ${probeResult.ok ? styles.probeOk : styles.probeError}`}
                >
                  {probeResult.ok
                    ? t('plugins.runtime.probeOk', { version: probeResult.version ?? '' })
                    : t('plugins.runtime.probeFailed', {
                      error: probeResult.error ?? t('common.unknown'),
                    })}
                </div>
              ) : null}
            </div>
          </div>

          <div className={styles.settingRow}>
            <div className={styles.settingInfo}>
              <span className={styles.settingTitle}>
                {t('plugins.runtime.followAppProxy')}
              </span>
              <span className={styles.settingDesc}>
                {t('plugins.runtime.followAppProxyHint')}
              </span>
            </div>
            <div className={styles.settingControl}>
              <Switch
                size="small"
                checked={draft.followAppProxy}
                onChange={(checked) => updateDraft({ followAppProxy: checked })}
              />
            </div>
          </div>
        </div>

        <div className={styles.formActions}>
          <button
            type="button"
            className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
            disabled={!settingsDirty || saving}
            onClick={() => void handleSaveSettings()}
          >
            {saving ? (
              <Loader2 size={14} className={styles.spin} aria-hidden="true" />
            ) : (
              <Save size={14} aria-hidden="true" />
            )}
            <span>{t('plugins.runtime.saveSettings')}</span>
          </button>
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.sectionHeader}>
          <h3>{t('plugins.runtime.middlewareTitle')}</h3>
          <span className={styles.sectionHint}>
            {t('plugins.runtime.middlewareCount', { count: middlewareStates.length })}
          </span>
        </div>

        {middlewareStates.length === 0 ? (
          <div className={styles.middlewareEmptyCard}>
            {t('plugins.runtime.middlewareEmpty')}
          </div>
        ) : (
          <div className={styles.tableCard}>
            <table className={styles.statsTable}>
              <thead>
                <tr>
                  <th>{t('plugins.runtime.columns.name')}</th>
                  <th>{t('plugins.runtime.columns.hooks')}</th>
                  <th className={styles.numeric}>{t('plugins.runtime.columns.calls')}</th>
                  <th className={styles.numeric}>{t('plugins.runtime.columns.avg')}</th>
                  <th className={styles.numeric}>{t('plugins.runtime.columns.failures')}</th>
                  <th className={styles.numeric}>{t('plugins.runtime.columns.dropped')}</th>
                  <th>{t('plugins.runtime.columns.lastError')}</th>
                </tr>
              </thead>
              <tbody>
                {middlewareStates.map((state) => {
                  const lastError = state.lastError || state.loadError || '';
                  return (
                    <tr key={state.pluginId}>
                      <td>
                        <div className={styles.identity}>
                          <span className={styles.identityName} title={state.name || state.spec}>
                            {state.name || state.spec}
                          </span>
                          <span className={styles.identityMeta}>
                            {state.spec}
                            {' · '}
                            {eventsLabel(state)}
                          </span>
                        </div>
                      </td>
                      <td>
                        {state.hooks.length > 0 ? (
                          <div className={styles.hookList}>
                            {state.hooks.map((hook) => (
                              <span key={hook} className={styles.hook}>{hook}</span>
                            ))}
                          </div>
                        ) : (
                          <span className={styles.muted}>{t('plugins.runtime.hooksEmpty')}</span>
                        )}
                      </td>
                      <td className={styles.numeric}>{state.calls}</td>
                      <td className={styles.numeric}>{formatMicroseconds(state.avgMicros)}</td>
                      <td className={styles.numeric}>{state.failures}</td>
                      <td className={styles.numeric}>{state.usageDropped}</td>
                      <td>
                        <span className={styles.errorCell} title={lastError}>
                          {lastError || '—'}
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionHeader}>
          <h3>{t('plugins.runtime.logsTitle')}</h3>
          <span className={styles.sectionHint}>
            {t('plugins.runtime.logsCount', { count: filteredLogs.length })}
          </span>
          <span className={styles.sectionHint}>
            {t('plugins.runtime.logsRedactedHint')}
          </span>
        </div>

        <div className={styles.logsCard}>
          <div className={styles.logsToolbar}>
            <div className={styles.segmented}>
              {(['all', 'info', 'warn', 'error'] as LogFilter[]).map((key) => (
                <button
                  key={key}
                  type="button"
                  className={`${styles.segmentButton} ${logFilter === key ? styles.segmentButtonActive : ''}`}
                  onClick={() => setLogFilter(key)}
                >
                  {logFilterLabels[key]}
                </button>
              ))}
            </div>
            <div className={styles.logsActions}>
              <Tooltip title={t('plugins.runtime.logsRefresh')}>
                <button
                  type="button"
                  className={styles.iconButton}
                  aria-label={t('plugins.runtime.logsRefresh')}
                  disabled={logsBusy}
                  onClick={() => void handleRefreshLogs()}
                >
                  <RefreshCw
                    size={14}
                    className={logsBusy ? styles.spin : undefined}
                    aria-hidden="true"
                  />
                </button>
              </Tooltip>
              <Tooltip title={t('plugins.runtime.logsCopy')}>
                <button
                  type="button"
                  className={styles.iconButton}
                  aria-label={t('plugins.runtime.logsCopy')}
                  disabled={filteredLogs.length === 0}
                  onClick={() => void handleCopyLogs()}
                >
                  <Copy size={14} aria-hidden="true" />
                </button>
              </Tooltip>
              <Tooltip title={t('plugins.runtime.logsClear')}>
                <button
                  type="button"
                  className={`${styles.iconButton} ${styles.iconButtonDanger}`}
                  aria-label={t('plugins.runtime.logsClear')}
                  disabled={logs.length === 0}
                  onClick={handleClearLogs}
                >
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </Tooltip>
            </div>
          </div>

          <div className={styles.logsList}>
            {filteredLogs.length === 0 ? (
              <div className={styles.logsEmpty}>{t('plugins.runtime.logsEmpty')}</div>
            ) : (
              filteredLogs.map((entry) => (
                <div key={entry.seq} className={styles.logRow}>
                  <span className={styles.logTime}>{formatLogTime(entry.at)}</span>
                  <span className={`${styles.logLevel} ${logLevelClass(entry.level)}`}>
                    {entry.level}
                  </span>
                  <span className={styles.logSource} title={entry.source}>
                    {entry.source}
                  </span>
                  <span className={styles.logMessage}>{entry.message}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>

      <div className={styles.runtimeFooter}>
        <Activity size={13} aria-hidden="true" />
        <span>{t('plugins.runtime.autoRefreshHint')}</span>
      </div>
    </div>
  );
};

export default PluginRuntimeView;
