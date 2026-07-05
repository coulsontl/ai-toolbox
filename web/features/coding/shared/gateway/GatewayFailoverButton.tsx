import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { AlertTriangle, CheckCircle2, Loader2, Network, RotateCcw, ShieldCheck, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  disengageProxyGatewayFailover,
  engageProxyGatewayFailover,
  getProxyGatewayCliStatus,
  restoreProxyGatewayCliDirect,
  type GatewayCliKey,
  type GatewayCliTakeoverStatus,
} from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import styles from './GatewayFailoverButton.module.less';

type SupportedGatewayCliKey = Extract<GatewayCliKey, 'claude' | 'codex' | 'gemini'>;
type ActionKind = 'load' | 'enableFailover' | 'disableFailover' | 'restore';
type NoticeKind = 'success' | 'error' | 'info';

interface GatewayFailoverButtonProps {
  cliKey: SupportedGatewayCliKey;
  status?: GatewayCliTakeoverStatus | null;
  primaryProviderNeedsGatewayProxy?: boolean;
  onStatusChange?: (status: GatewayCliTakeoverStatus) => void;
}

interface NoticeState {
  kind: NoticeKind;
  text: string;
}

const joinClassNames = (...classNames: Array<string | false | null | undefined>) =>
  classNames.filter(Boolean).join(' ');

const formatGatewayError = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

const isGatewayProxyActive = (status: GatewayCliTakeoverStatus | null) =>
  status?.mode === 'single' || status?.mode === 'failover';

const GatewayFailoverButton: React.FC<GatewayFailoverButtonProps> = ({
  cliKey,
  status: externalStatus,
  primaryProviderNeedsGatewayProxy = false,
  onStatusChange,
}) => {
  const { t } = useTranslation();
  const [status, setStatus] = React.useState<GatewayCliTakeoverStatus | null>(null);
  const [busyAction, setBusyAction] = React.useState<ActionKind | null>('load');
  const [open, setOpen] = React.useState(false);
  const [notice, setNotice] = React.useState<NoticeState | null>(null);

  React.useEffect(() => {
    setStatus(externalStatus ?? null);
  }, [externalStatus]);

  const refreshStatus = React.useCallback(async () => {
    const nextStatus = await getProxyGatewayCliStatus(cliKey);
    setStatus(nextStatus);
    onStatusChange?.(nextStatus);
    return nextStatus;
  }, [cliKey, onStatusChange]);

  const refreshTrayAfterGatewayChange = React.useCallback(() => {
    void refreshTrayMenu().catch((error) => {
      console.error('Failed to refresh tray menu after gateway change:', error);
    });
  }, []);

  React.useEffect(() => {
    let disposed = false;

    const loadStatus = async () => {
      setBusyAction('load');
      try {
        const nextStatus = await getProxyGatewayCliStatus(cliKey);
        if (disposed) {
          return;
        }
        setStatus(nextStatus);
        onStatusChange?.(nextStatus);
      } catch (error) {
        if (!disposed) {
          setNotice({
            kind: 'error',
            text: t('gateway.takeover.notice.loadFailed', { error: formatGatewayError(error) }),
          });
        }
      } finally {
        if (!disposed) {
          setBusyAction(null);
        }
      }
    };

    void loadStatus();

    return () => {
      disposed = true;
    };
  }, [cliKey, onStatusChange, t]);

  React.useEffect(() => {
    let disposed = false;
    const unlistenPromise = listen<boolean>('gateway-running-changed', () => {
      if (!disposed) {
        void refreshStatus().catch(() => undefined);
      }
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [refreshStatus]);

  const visible = isGatewayProxyActive(status);
  const failoverActive = status?.mode === 'failover';
  const canRestoreDirect = Boolean(status?.can_restore_direct);
  const dot = failoverActive ? (status?.dot ?? 'gray') : 'gray';
  const statusMessage = status?.message ?? t('gateway.takeover.buttonTooltip');
  const actionLabel = failoverActive
    ? t('gateway.failover.disengageButton')
    : t('gateway.failover.button');

  const handleOpen = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setNotice(null);
    setOpen(true);
  };

  const handleClose = (event?: React.MouseEvent<HTMLButtonElement>) => {
    event?.preventDefault();
    event?.stopPropagation();
    setOpen(false);
  };

  const handleToggleFailover = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    const nextBusyAction: ActionKind = failoverActive ? 'disableFailover' : 'enableFailover';
    setBusyAction(nextBusyAction);
    setNotice(null);
    try {
      const nextStatus = failoverActive
        ? await disengageProxyGatewayFailover(cliKey)
        : await engageProxyGatewayFailover(cliKey);
      setStatus(nextStatus);
      onStatusChange?.(nextStatus);
      refreshTrayAfterGatewayChange();
      setNotice({
        kind: 'success',
        text: failoverActive
          ? t('gateway.failover.notice.disabled')
          : t('gateway.failover.notice.enabled'),
      });
      setOpen(false);
    } catch (error) {
      setNotice({
        kind: 'error',
        text: failoverActive
          ? t('gateway.failover.notice.disableFailed', { error: formatGatewayError(error) })
          : t('gateway.failover.notice.enableFailed', { error: formatGatewayError(error) }),
      });
      await refreshStatus().catch(() => undefined);
    } finally {
      setBusyAction(null);
    }
  };

  const handleRestoreDirect = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    if (primaryProviderNeedsGatewayProxy) {
      setNotice({
        kind: 'info',
        text: t('gateway.proxy.restoreDirectProtocolConversionUnavailableHint'),
      });
      return;
    }
    setBusyAction('restore');
    setNotice(null);
    try {
      const nextStatus = await restoreProxyGatewayCliDirect(cliKey);
      setStatus(nextStatus);
      onStatusChange?.(nextStatus);
      refreshTrayAfterGatewayChange();
      setNotice({
        kind: 'success',
        text: t('gateway.proxy.notice.restored'),
      });
      setOpen(false);
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('gateway.proxy.notice.restoreFailed', { error: formatGatewayError(error) }),
      });
      await refreshStatus().catch(() => undefined);
    } finally {
      setBusyAction(null);
    }
  };

  if (!visible) {
    return null;
  }

  return (
    <span className={styles.shell} onClick={(event) => event.stopPropagation()}>
      <button
        type="button"
        className={joinClassNames(styles.button, failoverActive && styles.buttonActive)}
        title={statusMessage}
        onClick={handleOpen}
      >
        <span className={joinClassNames(styles.dot, styles[`dot_${dot}`])} aria-hidden="true" />
        <span>{actionLabel}</span>
      </button>

      {open ? (
        <div className={styles.overlay} role="presentation" onClick={() => setOpen(false)}>
          <div
            className={styles.dialog}
            role="dialog"
            aria-modal="true"
            aria-labelledby={`gateway-failover-title-${cliKey}`}
            onClick={(event) => event.stopPropagation()}
          >
            <div className={styles.dialogHeader}>
              <div className={styles.dialogTitleBlock}>
                <span className={styles.dialogIcon}>
                  <Network size={16} aria-hidden="true" />
                </span>
                <div>
                  <h3 id={`gateway-failover-title-${cliKey}`}>
                    {t('gateway.failover.confirmTitle', {
                      cli: t(`settings.gateway.cli.${cliKey}`),
                    })}
                  </h3>
                  <p>{statusMessage}</p>
                </div>
              </div>
              <button
                type="button"
                className={styles.iconButton}
                aria-label={t('common.close')}
                onClick={handleClose}
              >
                <X size={15} aria-hidden="true" />
              </button>
            </div>

            <div className={styles.dialogBody}>
              {failoverActive && (
                <div className={styles.stateRow}>
                  <span className={joinClassNames(styles.dot, styles[`dot_${dot}`])} aria-hidden="true" />
                  <span>{t(`gateway.takeover.state.${status?.state ?? 'direct'}`)}</span>
                  {status?.mode ? (
                    <span className={styles.modeLabel}>
                      {t(`gateway.failover.mode.${status.mode}`)}
                    </span>
                  ) : null}
                </div>
              )}

              <div className={styles.effectList}>
                <div>
                  <CheckCircle2 size={14} aria-hidden="true" />
                  <span>{t('gateway.failover.effects.singleProxy')}</span>
                </div>
                <div>
                  <ShieldCheck size={14} aria-hidden="true" />
                  <span>{t('gateway.failover.effects.p0Pinned')}</span>
                </div>
                <div>
                  <CheckCircle2 size={14} aria-hidden="true" />
                  <span>{t('gateway.failover.effects.providerOrder')}</span>
                </div>
                <div>
                  <AlertTriangle size={14} aria-hidden="true" />
                  <span>{t('gateway.failover.effects.applyDisabled')}</span>
                </div>
              </div>

              {failoverActive && status?.provider_priorities.length ? (
                <div className={styles.priorityList}>
                  <span className={styles.targetTitle}>{t('gateway.failover.priorities')}</span>
                  <div>
                    {status.provider_priorities.map((entry) => (
                      <code key={entry.provider_id}>{entry.label}</code>
                    ))}
                  </div>
                </div>
              ) : null}

              {status?.managed_targets.length ? (
                <div className={styles.targetList}>
                  <span className={styles.targetTitle}>{t('gateway.takeover.targets')}</span>
                  {status.managed_targets.map((target) => (
                    <code key={`${target.kind}:${target.path}`}>{target.path}</code>
                  ))}
                </div>
              ) : null}

              {notice ? (
                <div className={joinClassNames(styles.notice, styles[`notice_${notice.kind}`])} role="status">
                  {notice.text}
                </div>
              ) : canRestoreDirect && primaryProviderNeedsGatewayProxy ? (
                <div className={joinClassNames(styles.notice, styles.notice_info)} role="status">
                  {t('gateway.proxy.restoreDirectProtocolConversionUnavailableHint')}
                </div>
              ) : null}
            </div>

            <div className={styles.dialogFooter}>
              <button type="button" className={styles.secondaryButton} onClick={handleClose}>
                {t('common.cancel')}
              </button>
              {canRestoreDirect ? (
                <button
                  type="button"
                  className={styles.secondaryButton}
                  disabled={busyAction !== null || primaryProviderNeedsGatewayProxy}
                  title={
                    primaryProviderNeedsGatewayProxy
                      ? t('gateway.proxy.restoreDirectProtocolConversionUnavailableHint')
                      : t('gateway.proxy.restoreDirectHint')
                  }
                  onClick={handleRestoreDirect}
                >
                  {busyAction === 'restore' ? (
                    <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                  ) : (
                    <RotateCcw size={14} aria-hidden="true" />
                  )}
                  <span>{t('gateway.proxy.restoreDirectButton')}</span>
                </button>
              ) : null}
              <button
                type="button"
                className={styles.primaryButton}
                disabled={busyAction !== null}
                onClick={handleToggleFailover}
              >
                {busyAction === 'enableFailover' || busyAction === 'disableFailover' ? (
                  <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                ) : (
                  <Network size={14} aria-hidden="true" />
                )}
                <span>{actionLabel}</span>
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </span>
  );
};

export default GatewayFailoverButton;
