import React from 'react';
import { Switch } from 'antd';
import { platform } from '@tauri-apps/plugin-os';
import {
  AlertCircle,
  ArrowRightLeft,
  CircleHelp,
  FileText,
  Gauge,
  Loader2,
  Network,
  Terminal,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  checkProxyGatewayPortAvailable,
  getProxyGatewayCliStatuses,
  getProxyGatewaySettings,
  getProxyGatewayStatus,
  updateProxyGatewaySettings,
  type GatewayCliTakeoverStatus,
  type GatewayCliKey,
  type AppProxyConfig,
  type ProxyGatewaySettings,
  type ProxyGatewayStatus,
} from '@/services';
import styles from './GatewaySettingsPanel.module.less';

type BusyAction = 'load' | 'autosave';
type NoticeKind = 'success' | 'error' | 'info';
type SupportedGatewayCliKey = Extract<GatewayCliKey, 'claude' | 'codex' | 'gemini'>;

interface NoticeState {
  kind: NoticeKind;
  text: string;
}

interface CliOption {
  key: SupportedGatewayCliKey;
  labelKey: string;
}

const CLI_OPTIONS: CliOption[] = [
  {
    key: 'claude',
    labelKey: 'settings.gateway.cli.claude',
  },
  {
    key: 'codex',
    labelKey: 'settings.gateway.cli.codex',
  },
  {
    key: 'gemini',
    labelKey: 'settings.gateway.cli.gemini',
  },
];

const joinClassNames = (...classNames: Array<string | false | null | undefined>) =>
  classNames.filter(Boolean).join(' ');

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

const toInteger = (value: string, fallback: number, minimum = 0) => {
  const nextValue = Number(value);
  if (!Number.isFinite(nextValue)) {
    return fallback;
  }
  return Math.max(minimum, Math.trunc(nextValue));
};

const formatGatewayError = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

const deriveRequestLogLevel = (settings: ProxyGatewaySettings) => {
  if (!settings.request_log_enabled) {
    return 'off';
  }
  if (settings.store_request_body && settings.store_headers && settings.store_response_body) {
    return 'full';
  }
  if (settings.store_request_body || settings.store_response_body) {
    return 'body';
  }
  if (settings.store_headers) {
    return 'headers';
  }
  return 'summary';
};

const isCliTakeoverActive = (status: GatewayCliTakeoverStatus | undefined) =>
  Boolean(status?.can_restore_direct);

type NumericAppProxyConfigKey =
  | 'streaming_first_byte_timeout_secs'
  | 'streaming_idle_timeout_secs'
  | 'non_streaming_timeout_secs'
  | 'per_provider_retry_count'
  | 'max_retry_count'
  | 'retry_interval_secs';

const appProxyConfigKeys: Array<keyof AppProxyConfig> = [
  'streaming_first_byte_timeout_secs',
  'streaming_idle_timeout_secs',
  'non_streaming_timeout_secs',
  'per_provider_retry_count',
  'max_retry_count',
  'retry_interval_secs',
  'cost_multiplier',
  'pricing_model_source',
];

interface SwitchControlProps {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}

const SwitchControl: React.FC<SwitchControlProps> = ({ checked, disabled, label, onChange }) => (
  <div className={styles.switchControl}>
    <Switch size="small" checked={checked} disabled={disabled} onChange={onChange} />
    <span className={styles.switchLabel}>{label}</span>
  </div>
);

interface FieldRowProps {
  label: string;
  description?: string;
  help?: string;
  wide?: boolean;
  children: React.ReactNode;
}

const FieldRow: React.FC<FieldRowProps> = ({ label, description, help, wide, children }) => (
  <div className={joinClassNames(styles.fieldRow, wide && styles.fieldRowWide)}>
    <div className={styles.fieldMeta}>
      <span className={styles.fieldLabelRow}>
        <span className={styles.fieldLabel}>{label}</span>
        {help ? (
          <span className={styles.fieldHelpButton} tabIndex={0} aria-label={help}>
            <CircleHelp size={12} aria-hidden="true" />
            <span className={styles.fieldHelpBubble} role="tooltip">
              {help}
            </span>
          </span>
        ) : null}
      </span>
      {description ? <span className={styles.fieldDescription}>{description}</span> : null}
    </div>
    <div className={styles.fieldControl}>{children}</div>
  </div>
);

interface SectionProps {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}

const Section: React.FC<SectionProps> = ({ icon, title, children }) => (
  <section className={styles.section}>
    <div className={styles.sectionHeader}>
      <span className={styles.sectionIcon}>{icon}</span>
      <h3>{title}</h3>
    </div>
    <div className={styles.sectionBody}>{children}</div>
  </section>
);

interface GatewaySettingsPanelProps {
  showTitleBlock?: boolean;
  onStatusChange?: (status: ProxyGatewayStatus) => void;
  onDraftSettingsChange?: (settings: ProxyGatewaySettings | null) => void;
}

const GatewaySettingsPanel: React.FC<GatewaySettingsPanelProps> = ({
  showTitleBlock = true,
  onStatusChange,
  onDraftSettingsChange,
}) => {
  const { t } = useTranslation();
  const isWindows = React.useMemo(() => platform() === 'windows', []);
  const [savedSettings, setSavedSettings] = React.useState<ProxyGatewaySettings | null>(null);
  const [draftSettings, setDraftSettings] = React.useState<ProxyGatewaySettings | null>(null);
  const [status, setStatus] = React.useState<ProxyGatewayStatus | null>(null);
  const [cliStatuses, setCliStatuses] = React.useState<GatewayCliTakeoverStatus[]>([]);
  const [busyAction, setBusyAction] = React.useState<BusyAction | null>('load');
  const [checkingPort, setCheckingPort] = React.useState(false);
  const [notice, setNotice] = React.useState<NoticeState | null>(null);
  const saveTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const saveSequenceRef = React.useRef(0);
  const pendingSaveRef = React.useRef(false);

  const updateDraftSetting = React.useCallback(
    <K extends keyof ProxyGatewaySettings>(key: K, value: ProxyGatewaySettings[K]) => {
      setDraftSettings((previousSettings) =>
        previousSettings ? { ...previousSettings, [key]: value } : previousSettings,
      );
    },
    [],
  );

  const updateDraftAndSave = React.useCallback(
    <K extends keyof ProxyGatewaySettings>(key: K, value: ProxyGatewaySettings[K]) => {
      pendingSaveRef.current = true;
      setDraftSettings((previousSettings) =>
        previousSettings ? { ...previousSettings, [key]: value } : previousSettings,
      );
    },
    [],
  );

  React.useEffect(() => {
    let disposed = false;

    const loadGateway = async () => {
      setBusyAction('load');
      try {
        const [nextSettings, nextStatus, nextCliStatuses] = await Promise.all([
          getProxyGatewaySettings(),
          getProxyGatewayStatus(),
          getProxyGatewayCliStatuses(),
        ]);
        if (disposed) {
          return;
        }
        setSavedSettings(nextSettings);
        setDraftSettings(cloneGatewaySettings(nextSettings));
        setStatus(nextStatus);
        onStatusChange?.(nextStatus);
        setCliStatuses(nextCliStatuses);
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

    void loadGateway();

    return () => {
      disposed = true;
    };
  }, [onStatusChange, t]);

  React.useEffect(() => () => {
    if (saveTimerRef.current) {
      clearTimeout(saveTimerRef.current);
    }
    saveSequenceRef.current += 1;
  }, []);

  React.useEffect(() => {
    onDraftSettingsChange?.(draftSettings ? cloneGatewaySettings(draftSettings) : null);
  }, [draftSettings, onDraftSettingsChange]);

  React.useEffect(
    () => () => {
      onDraftSettingsChange?.(null);
    },
    [onDraftSettingsChange],
  );

  const cliStatusByKey = React.useMemo(() => {
    const entries = cliStatuses.map((cliStatus) => [cliStatus.cli_key, cliStatus] as const);
    return Object.fromEntries(entries) as Partial<Record<SupportedGatewayCliKey, GatewayCliTakeoverStatus>>;
  }, [cliStatuses]);

  const triggerSave = React.useCallback(() => {
    if (!draftSettings || !savedSettings) {
      return;
    }
    if (JSON.stringify(draftSettings) === JSON.stringify(savedSettings)) {
      return;
    }

    if (saveTimerRef.current) {
      clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }

    const sequence = saveSequenceRef.current + 1;
    saveSequenceRef.current = sequence;
    setBusyAction('autosave');
    saveTimerRef.current = setTimeout(() => {
      saveTimerRef.current = null;
      void (async () => {
        try {
          const nextSettings = await updateProxyGatewaySettings({
            ...draftSettings,
            enabled_on_startup: status?.running ? true : draftSettings.enabled_on_startup,
          });
          if (saveSequenceRef.current !== sequence) {
            return;
          }
          setSavedSettings(nextSettings);
          setDraftSettings(cloneGatewaySettings(nextSettings));
          setNotice({ kind: 'success', text: t('settings.gateway.notice.autoSaved') });
        } catch (error) {
          if (saveSequenceRef.current !== sequence) {
            return;
          }
          setNotice({
            kind: 'error',
            text: t('settings.gateway.notice.saveFailed', { error: formatGatewayError(error) }),
          });
        } finally {
          if (saveSequenceRef.current === sequence) {
            setBusyAction(null);
          }
        }
      })();
    }, 200);
  }, [draftSettings, savedSettings, status?.running, t]);

  React.useEffect(() => {
    if (!pendingSaveRef.current) {
      return;
    }
    pendingSaveRef.current = false;
    triggerSave();
  }, [draftSettings, triggerSave]);

  const handleContentBlur = React.useCallback(
    (event: React.FocusEvent<HTMLDivElement>) => {
      if (event.target instanceof HTMLInputElement) {
        triggerSave();
      }
    },
    [triggerSave],
  );

  const handleCheckPort = async () => {
    if (!draftSettings) {
      return;
    }
    setCheckingPort(true);
    try {
      const result = await checkProxyGatewayPortAvailable({
        listen_host: draftSettings.listen_host,
        listen_port: draftSettings.listen_port,
      });
      setNotice({
        kind: result.available ? 'success' : 'error',
        text: result.available
          ? t('settings.gateway.notice.portAvailable', { port: result.listen_port })
          : t('settings.gateway.notice.portOccupied', { port: result.listen_port }),
      });
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('settings.gateway.notice.portCheckFailed', { error: formatGatewayError(error) }),
      });
    } finally {
      setCheckingPort(false);
    }
  };

  const handleLogPartToggle = (
    key: 'store_request_body' | 'store_headers' | 'store_response_body',
    checked: boolean,
  ) => {
    if (!draftSettings) {
      return;
    }
    pendingSaveRef.current = true;
    const nextSettings = { ...draftSettings, [key]: checked };
    nextSettings.request_log_level = deriveRequestLogLevel(nextSettings);
    setDraftSettings(nextSettings);
  };

  const handleRequestLogEnabledToggle = (checked: boolean) => {
    if (!draftSettings) {
      return;
    }
    pendingSaveRef.current = true;
    const nextSettings = { ...draftSettings, request_log_enabled: checked };
    nextSettings.request_log_level = deriveRequestLogLevel(nextSettings);
    setDraftSettings(nextSettings);
  };

  const updateAppProxyConfig = (
    cliKey: SupportedGatewayCliKey,
    key: NumericAppProxyConfigKey,
    rawValue: string,
    minimum = 0,
  ) => {
    if (!draftSettings) {
      return;
    }
    const trimmedValue = rawValue.trim();
    const currentValue = draftSettings.app_configs?.[cliKey]?.[key];
    const nextValue = trimmedValue === ''
      ? null
      : toInteger(trimmedValue, typeof currentValue === 'number' ? currentValue : 0, minimum);
    const nextConfig = {
      ...(draftSettings.app_configs?.[cliKey] ?? {}),
      [key]: nextValue,
    };
    const emptyConfig = appProxyConfigKeys.every((configKey) => nextConfig[configKey] == null);
    setDraftSettings({
      ...draftSettings,
      app_configs: {
        ...(draftSettings.app_configs ?? {}),
        [cliKey]: emptyConfig ? undefined : nextConfig,
      },
    });
  };

  if (!draftSettings) {
    return (
      <div className={styles.loadingState}>
        <Loader2 size={18} className={styles.spin} aria-hidden="true" />
        <span>{t('settings.gateway.loading')}</span>
      </div>
    );
  }

  return (
    <div className={styles.panel}>
      {showTitleBlock || busyAction === 'autosave' ? (
        <div className={joinClassNames(styles.topBar, !showTitleBlock && styles.topBarActionsOnly)}>
          {showTitleBlock ? (
            <div className={styles.titleBlock}>
              <span className={styles.titleIcon}>
                <Network size={18} aria-hidden="true" />
              </span>
              <div>
                <h2>{t('settings.gateway.title')}</h2>
                <p>{t('settings.gateway.subtitle')}</p>
              </div>
            </div>
          ) : null}

          <div className={styles.actionBar}>
            {busyAction === 'autosave' ? (
              <span className={styles.autoSaveText}>
                <Loader2 size={12} className={styles.spin} aria-hidden="true" />
                {t('settings.gateway.notice.autoSaving')}
              </span>
            ) : null}
          </div>
        </div>
      ) : null}

      {status?.last_error ? (
        <div className={styles.inlineAlert} role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{status.last_error}</span>
        </div>
      ) : null}

      {notice ? (
        <div className={joinClassNames(styles.notice, styles[`notice_${notice.kind}`])} role="status" aria-live="polite">
          {notice.text}
        </div>
      ) : null}

      <div className={styles.contentGrid} onBlur={handleContentBlur}>
        <div className={styles.contentColumn}>
          <Section icon={<Network size={15} aria-hidden="true" />} title={t('settings.gateway.sections.listen')}>
            <div className={styles.fieldStack}>
              <FieldRow label={t('settings.gateway.fields.host')} description={t('settings.gateway.hints.host')}>
                <input
                  className={styles.textInput}
                  value={draftSettings.listen_host}
                  onChange={(event) => updateDraftSetting('listen_host', event.currentTarget.value)}
                />
              </FieldRow>
              <FieldRow label={t('settings.gateway.fields.port')} description={t('settings.gateway.hints.port')}>
                <div className={styles.inlineControlGroup}>
                  <input
                    className={styles.numberInput}
                    type="number"
                    min={1024}
                    value={draftSettings.listen_port}
                    onChange={(event) =>
                      updateDraftSetting(
                        'listen_port',
                        toInteger(event.currentTarget.value, draftSettings.listen_port, 0),
                      )
                    }
                  />
                  <button
                    type="button"
                    className={styles.textButton}
                    disabled={checkingPort}
                    onClick={() => void handleCheckPort()}
                  >
                    {t('common.check')}
                  </button>
                </div>
              </FieldRow>
              <FieldRow label={t('settings.gateway.fields.autoSelectPort')} wide>
                <SwitchControl
                  checked={draftSettings.port_auto_select}
                  label={draftSettings.port_auto_select ? t('common.enabled') : t('common.disabled')}
                  onChange={(checked) => updateDraftAndSave('port_auto_select', checked)}
                />
              </FieldRow>
              {isWindows && (
                <FieldRow label={t('settings.gateway.fields.wslHost')} description={t('settings.gateway.hints.wslHost')}>
                  <input
                    className={styles.textInput}
                    value={draftSettings.wsl_host}
                    placeholder="192.168.x.x"
                    onChange={(event) => updateDraftSetting('wsl_host', event.currentTarget.value)}
                  />
                </FieldRow>
              )}
            </div>
          </Section>

          <Section icon={<ArrowRightLeft size={15} aria-hidden="true" />} title={t('settings.gateway.sections.resilience')}>
            <div className={styles.fieldStack}>
              <div className={styles.subGroup}>
                <div className={styles.subGroupLabel}>{t('settings.gateway.subGroups.rectifier')}</div>
                <FieldRow
                  label={t('settings.gateway.fields.thinkingRectifier')}
                  description={t('settings.gateway.hints.thinkingRectifier')}
                  wide
                >
                  <SwitchControl
                    checked={draftSettings.thinking_rectifier_enabled}
                    label={draftSettings.thinking_rectifier_enabled ? t('common.enabled') : t('common.disabled')}
                    onChange={(checked) => updateDraftAndSave('thinking_rectifier_enabled', checked)}
                  />
                </FieldRow>
                <FieldRow
                  label={t('settings.gateway.fields.responsesEncryptedContentRectifier')}
                  description={t('settings.gateway.hints.responsesEncryptedContentRectifier')}
                  wide
                >
                  <SwitchControl
                    checked={draftSettings.responses_encrypted_content_rectifier_enabled}
                    label={draftSettings.responses_encrypted_content_rectifier_enabled ? t('common.enabled') : t('common.disabled')}
                    onChange={(checked) => updateDraftAndSave('responses_encrypted_content_rectifier_enabled', checked)}
                  />
                </FieldRow>
                <FieldRow
                  label={t('settings.gateway.fields.thinkingBudgetRectifier')}
                  description={t('settings.gateway.hints.thinkingBudgetRectifier')}
                  wide
                >
                  <SwitchControl
                    checked={draftSettings.thinking_budget_rectifier_enabled}
                    label={draftSettings.thinking_budget_rectifier_enabled ? t('common.enabled') : t('common.disabled')}
                    onChange={(checked) => updateDraftAndSave('thinking_budget_rectifier_enabled', checked)}
                  />
                </FieldRow>
                <FieldRow
                  label={t('settings.gateway.fields.lossyRejection')}
                  description={t('settings.gateway.hints.lossyRejection')}
                  wide
                >
                  <SwitchControl
                    checked={draftSettings.lossy_rejection_enabled}
                    label={draftSettings.lossy_rejection_enabled ? t('common.enabled') : t('common.disabled')}
                    onChange={(checked) => updateDraftAndSave('lossy_rejection_enabled', checked)}
                  />
                </FieldRow>
                <FieldRow
                  label={t('settings.gateway.fields.cacheInjection')}
                  description={t('settings.gateway.hints.cacheInjection')}
                  wide
                >
                  <SwitchControl
                    checked={draftSettings.cache_injection_enabled}
                    label={draftSettings.cache_injection_enabled ? t('common.enabled') : t('common.disabled')}
                    onChange={(checked) => updateDraftAndSave('cache_injection_enabled', checked)}
                  />
                </FieldRow>
              </div>

              <div className={styles.subGroup}>
                <div className={styles.subGroupLabel}>{t('settings.gateway.subGroups.timeout')}</div>
                <div className={styles.fieldPairGrid}>
                  <FieldRow
                    label={t('settings.gateway.fields.firstByteTimeout')}
                    help={t('settings.gateway.fieldHelp.firstByteTimeout')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={1}
                      value={draftSettings.streaming_first_byte_timeout_secs}
                      onChange={(event) =>
                        updateDraftSetting(
                          'streaming_first_byte_timeout_secs',
                          toInteger(event.currentTarget.value, draftSettings.streaming_first_byte_timeout_secs, 1),
                        )
                      }
                    />
                  </FieldRow>
                  <FieldRow
                    label={t('settings.gateway.fields.idleTimeout')}
                    help={t('settings.gateway.fieldHelp.idleTimeout')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={1}
                      value={draftSettings.streaming_idle_timeout_secs}
                      onChange={(event) =>
                        updateDraftSetting(
                          'streaming_idle_timeout_secs',
                          toInteger(event.currentTarget.value, draftSettings.streaming_idle_timeout_secs, 1),
                        )
                      }
                    />
                  </FieldRow>
                </div>
                <div className={styles.fieldPairGrid}>
                  <FieldRow
                    label={t('settings.gateway.fields.nonStreamingTimeout')}
                    help={t('settings.gateway.fieldHelp.nonStreamingTimeout')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={1}
                      value={draftSettings.non_streaming_timeout_secs}
                      onChange={(event) =>
                        updateDraftSetting(
                          'non_streaming_timeout_secs',
                          toInteger(event.currentTarget.value, draftSettings.non_streaming_timeout_secs, 1),
                        )
                      }
                    />
                  </FieldRow>
                </div>
              </div>

              <div className={styles.subGroup}>
                <div className={styles.subGroupLabel}>{t('settings.gateway.subGroups.retry')}</div>
                <div className={styles.fieldPairGrid}>
                  <FieldRow
                    label={t('settings.gateway.fields.perProviderRetry')}
                    help={t('settings.gateway.fieldHelp.perProviderRetry')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={0}
                      value={draftSettings.per_provider_retry_count}
                      onChange={(event) =>
                        updateDraftSetting(
                          'per_provider_retry_count',
                          Math.min(
                            toInteger(event.currentTarget.value, draftSettings.per_provider_retry_count, 0),
                            draftSettings.max_retry_count,
                          ),
                        )
                      }
                    />
                  </FieldRow>
                  <FieldRow
                    label={t('settings.gateway.fields.maxRetry')}
                    help={t('settings.gateway.fieldHelp.maxRetry')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={0}
                      value={draftSettings.max_retry_count}
                      onChange={(event) => {
                        const maxRetryCount = toInteger(event.currentTarget.value, draftSettings.max_retry_count, 0);
                        setDraftSettings((previousSettings) =>
                          previousSettings
                            ? {
                                ...previousSettings,
                                max_retry_count: maxRetryCount,
                                per_provider_retry_count: Math.min(
                                  previousSettings.per_provider_retry_count,
                                  maxRetryCount,
                                ),
                              }
                            : previousSettings,
                        );
                      }}
                    />
                  </FieldRow>
                  <FieldRow
                    label={t('settings.gateway.fields.retryInterval')}
                    help={t('settings.gateway.fieldHelp.retryInterval')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={0}
                      value={draftSettings.retry_interval_secs}
                      onChange={(event) =>
                        updateDraftSetting(
                          'retry_interval_secs',
                          toInteger(event.currentTarget.value, draftSettings.retry_interval_secs, 0),
                        )
                      }
                    />
                  </FieldRow>
                </div>
              </div>

              <div className={styles.subGroup}>
                <div className={styles.subGroupLabel}>
                  {t('settings.gateway.subGroups.health')}
                  <span
                    style={{
                      marginLeft: 8,
                      fontSize: 10,
                      fontWeight: 400,
                      color: 'var(--color-text-tertiary)',
                    }}
                  >
                    {t('settings.gateway.subGroups.healthHint')}
                  </span>
                </div>
                <div className={styles.fieldPairGrid}>
                  <FieldRow
                    label={t('settings.gateway.fields.failureThreshold')}
                    help={t('settings.gateway.fieldHelp.failureThreshold')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={1}
                      value={draftSettings.model_failure_score_threshold}
                      onChange={(event) =>
                        updateDraftSetting(
                          'model_failure_score_threshold',
                          toInteger(event.currentTarget.value, draftSettings.model_failure_score_threshold, 1),
                        )
                      }
                    />
                  </FieldRow>
                  <FieldRow
                    label={t('settings.gateway.fields.failureWindow')}
                    help={t('settings.gateway.fieldHelp.failureWindow')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={30}
                      value={draftSettings.model_failure_window_seconds}
                      onChange={(event) =>
                        updateDraftSetting(
                          'model_failure_window_seconds',
                          toInteger(event.currentTarget.value, draftSettings.model_failure_window_seconds, 30),
                        )
                      }
                    />
                  </FieldRow>
                </div>
                <div className={styles.fieldPairGrid}>
                  <FieldRow
                    label={t('settings.gateway.fields.baseCooldown')}
                    help={t('settings.gateway.fieldHelp.baseCooldown')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={30}
                      value={draftSettings.model_base_cooldown_seconds}
                      onChange={(event) =>
                        updateDraftSetting(
                          'model_base_cooldown_seconds',
                          toInteger(event.currentTarget.value, draftSettings.model_base_cooldown_seconds, 30),
                        )
                      }
                    />
                  </FieldRow>
                  <FieldRow
                    label={t('settings.gateway.fields.maxCooldown')}
                    help={t('settings.gateway.fieldHelp.maxCooldown')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={60}
                      value={draftSettings.model_max_cooldown_seconds}
                      onChange={(event) =>
                        updateDraftSetting(
                          'model_max_cooldown_seconds',
                          toInteger(event.currentTarget.value, draftSettings.model_max_cooldown_seconds, 60),
                        )
                      }
                    />
                  </FieldRow>
                </div>
                <div className={styles.fieldPairGrid}>
                  <FieldRow
                    label={t('settings.gateway.fields.probeSuccess')}
                    help={t('settings.gateway.fieldHelp.probeSuccess')}
                  >
                    <input
                      className={styles.numberInput}
                      type="number"
                      min={1}
                      value={draftSettings.half_open_success_required}
                      onChange={(event) =>
                        updateDraftSetting(
                          'half_open_success_required',
                          toInteger(event.currentTarget.value, draftSettings.half_open_success_required, 1),
                        )
                      }
                    />
                  </FieldRow>
                </div>
              </div>
            </div>
          </Section>
        </div>

        <div className={styles.contentColumn}>
          <Section icon={<Terminal size={15} aria-hidden="true" />} title={t('settings.gateway.sections.cli')}>
            <div className={styles.perCliGrid}>
              {CLI_OPTIONS.map((option) => {
                const cliConfig = draftSettings.app_configs?.[option.key] ?? {};
                const cliStatus = cliStatusByKey[option.key];
                const active = isCliTakeoverActive(cliStatus);
                const dot = cliStatus?.dot ?? 'gray';
                return (
                  <div key={option.key} className={styles.perCliBlock}>
                    <div className={styles.perCliTitle} title={cliStatus?.message ?? t('settings.gateway.cliStatus.direct')}>
                      <span>{t(option.labelKey)}</span>
                      <span
                        className={joinClassNames(
                          styles.cliTag,
                          styles[`cliTag_${dot}`],
                          active && styles.cliTagActive,
                        )}
                      >
                        {t(`settings.gateway.cliStatus.${cliStatus?.state ?? 'direct'}`)}
                      </span>
                    </div>
                    <div className={styles.perCliFields}>
                      <label>
                        <span>{t('settings.gateway.perCli.firstByte')}</span>
                        <input
                          className={styles.numberInput}
                          type="number"
                          min={1}
                          placeholder={String(draftSettings.streaming_first_byte_timeout_secs)}
                          value={cliConfig.streaming_first_byte_timeout_secs ?? ''}
                          onChange={(event) =>
                            updateAppProxyConfig(option.key, 'streaming_first_byte_timeout_secs', event.currentTarget.value, 1)
                          }
                        />
                      </label>
                      <label>
                        <span>{t('settings.gateway.perCli.idle')}</span>
                        <input
                          className={styles.numberInput}
                          type="number"
                          min={1}
                          placeholder={String(draftSettings.streaming_idle_timeout_secs)}
                          value={cliConfig.streaming_idle_timeout_secs ?? ''}
                          onChange={(event) =>
                            updateAppProxyConfig(option.key, 'streaming_idle_timeout_secs', event.currentTarget.value, 1)
                          }
                        />
                      </label>
                      <label>
                        <span>{t('settings.gateway.perCli.nonStreaming')}</span>
                        <input
                          className={styles.numberInput}
                          type="number"
                          min={1}
                          placeholder={String(draftSettings.non_streaming_timeout_secs)}
                          value={cliConfig.non_streaming_timeout_secs ?? ''}
                          onChange={(event) =>
                            updateAppProxyConfig(option.key, 'non_streaming_timeout_secs', event.currentTarget.value, 1)
                          }
                        />
                      </label>
                      <label>
                        <span>{t('settings.gateway.perCli.providerRetry')}</span>
                        <input
                          className={styles.numberInput}
                          type="number"
                          min={0}
                          placeholder={String(draftSettings.per_provider_retry_count)}
                          value={cliConfig.per_provider_retry_count ?? ''}
                          onChange={(event) =>
                            updateAppProxyConfig(option.key, 'per_provider_retry_count', event.currentTarget.value, 0)
                          }
                        />
                      </label>
                      <label>
                        <span>{t('settings.gateway.perCli.maxRetry')}</span>
                        <input
                          className={styles.numberInput}
                          type="number"
                          min={0}
                          placeholder={String(draftSettings.max_retry_count)}
                          value={cliConfig.max_retry_count ?? ''}
                          onChange={(event) =>
                            updateAppProxyConfig(option.key, 'max_retry_count', event.currentTarget.value, 0)
                          }
                        />
                      </label>
                      <label>
                        <span>{t('settings.gateway.perCli.retryInterval')}</span>
                        <input
                          className={styles.numberInput}
                          type="number"
                          min={0}
                          placeholder={String(draftSettings.retry_interval_secs)}
                          value={cliConfig.retry_interval_secs ?? ''}
                          onChange={(event) =>
                            updateAppProxyConfig(option.key, 'retry_interval_secs', event.currentTarget.value, 0)
                          }
                        />
                      </label>
                    </div>
                  </div>
                );
              })}
            </div>
          </Section>

          <Section icon={<FileText size={15} aria-hidden="true" />} title={t('settings.gateway.sections.logs')}>
            <div className={styles.fieldStack}>
              <FieldRow label={t('settings.gateway.fields.requestLog')} wide>
                <SwitchControl
                  checked={draftSettings.request_log_enabled}
                  label={draftSettings.request_log_enabled ? t('common.enabled') : t('common.disabled')}
                  onChange={handleRequestLogEnabledToggle}
                />
              </FieldRow>
              <FieldRow label={t('settings.gateway.fields.metrics')} wide>
                <SwitchControl
                  checked={draftSettings.metrics_enabled}
                  label={draftSettings.metrics_enabled ? t('common.enabled') : t('common.disabled')}
                  onChange={(checked) => updateDraftAndSave('metrics_enabled', checked)}
                />
              </FieldRow>
              <div className={styles.logParts} aria-label={t('settings.gateway.fields.detailStorage')}>
                <label className={styles.checkItem}>
                  <input
                    type="checkbox"
                    checked={draftSettings.store_headers}
                    disabled={!draftSettings.request_log_enabled}
                    onChange={(event) => handleLogPartToggle('store_headers', event.currentTarget.checked)}
                  />
                  <span>{t('settings.gateway.logParts.headers')}</span>
                </label>
                <label className={styles.checkItem}>
                  <input
                    type="checkbox"
                    checked={draftSettings.store_request_body}
                    disabled={!draftSettings.request_log_enabled}
                    onChange={(event) => handleLogPartToggle('store_request_body', event.currentTarget.checked)}
                  />
                  <span>{t('settings.gateway.logParts.requestBody')}</span>
                </label>
                <label className={styles.checkItem}>
                  <input
                    type="checkbox"
                    checked={draftSettings.store_response_body}
                    disabled={!draftSettings.request_log_enabled}
                    onChange={(event) => handleLogPartToggle('store_response_body', event.currentTarget.checked)}
                  />
                  <span>{t('settings.gateway.logParts.response')}</span>
                </label>
              </div>
              <div className={styles.fieldPairGrid}>
                <FieldRow label={t('settings.gateway.fields.retentionDays')}>
                  <input
                    className={styles.numberInput}
                    type="number"
                    min={1}
                    value={draftSettings.log_retention_days}
                    onChange={(event) =>
                      updateDraftSetting(
                        'log_retention_days',
                        toInteger(event.currentTarget.value, draftSettings.log_retention_days, 1),
                      )
                    }
                  />
                </FieldRow>
                <FieldRow label={t('settings.gateway.fields.maxDirSize')}>
                  <input
                    className={styles.numberInput}
                    type="number"
                    min={1}
                    value={draftSettings.log_max_dir_size_mb}
                    onChange={(event) =>
                      updateDraftSetting(
                        'log_max_dir_size_mb',
                        toInteger(event.currentTarget.value, draftSettings.log_max_dir_size_mb, 1),
                      )
                    }
                  />
                </FieldRow>
              </div>
              <div className={styles.fieldPairGrid}>
                <FieldRow label={t('settings.gateway.fields.maxBodySize')}>
                  <input
                    className={styles.numberInput}
                    type="number"
                    min={1}
                    value={draftSettings.log_max_body_size_kb}
                    onChange={(event) =>
                      updateDraftSetting(
                        'log_max_body_size_kb',
                        toInteger(event.currentTarget.value, draftSettings.log_max_body_size_kb, 1),
                      )
                    }
                  />
                </FieldRow>
              </div>
            </div>
          </Section>
        </div>
      </div>

      <div className={styles.footerMetrics}>
        <div>
          <Gauge size={14} aria-hidden="true" />
          <span>{t('settings.gateway.metrics.logLevel', { level: deriveRequestLogLevel(draftSettings) })}</span>
        </div>
        <div>
          <FileText size={14} aria-hidden="true" />
          <span>{t('settings.gateway.metrics.logStorage')}</span>
        </div>
      </div>
    </div>
  );
};

export default GatewaySettingsPanel;
