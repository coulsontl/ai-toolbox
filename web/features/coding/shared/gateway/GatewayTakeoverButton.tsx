import React from 'react';
import { AlertTriangle, CheckCircle2, Loader2, Network, RotateCcw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getProxyGatewayCliStatus,
  restoreProxyGatewayCliDirect,
  takeoverProxyGatewayCli,
  type GatewayCliKey,
  type GatewayCliTakeoverStatus,
} from '@/services';
import styles from './GatewayTakeoverButton.module.less';

type SupportedGatewayCliKey = Extract<GatewayCliKey, 'claude' | 'codex' | 'gemini'>;
type ActionKind = 'load' | 'takeover' | 'restore';
type NoticeKind = 'success' | 'error' | 'info';

interface GatewayTakeoverButtonProps {
  cliKey: SupportedGatewayCliKey;
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

const isGatewayTakeoverActive = (status: GatewayCliTakeoverStatus | null) =>
  Boolean(status?.can_restore_direct);

const GatewayTakeoverButton: React.FC<GatewayTakeoverButtonProps> = ({ cliKey, onStatusChange }) => {
  const { t } = useTranslation();
  const [status, setStatus] = React.useState<GatewayCliTakeoverStatus | null>(null);
  const [busyAction, setBusyAction] = React.useState<ActionKind | null>('load');
  const [open, setOpen] = React.useState(false);
  const [notice, setNotice] = React.useState<NoticeState | null>(null);

  const refreshStatus = React.useCallback(async () => {
    const nextStatus = await getProxyGatewayCliStatus(cliKey);
    setStatus(nextStatus);
    onStatusChange?.(nextStatus);
    return nextStatus;
  }, [cliKey, onStatusChange]);

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

  const visible = Boolean(status && (status.can_takeover || status.can_restore_direct || status.state !== 'direct'));
  const takeoverActive = isGatewayTakeoverActive(status);
  const dot = status?.dot ?? 'gray';
  const statusMessage =
    status?.state === 'no_proxy_provider'
      ? t('gateway.takeover.noProxyProvider')
      : status?.message ?? t('gateway.takeover.buttonTooltip');

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

  const handleTakeover = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setBusyAction('takeover');
    setNotice(null);
    try {
      const nextStatus = await takeoverProxyGatewayCli(cliKey);
      setStatus(nextStatus);
      onStatusChange?.(nextStatus);
      setNotice({ kind: 'success', text: t('gateway.takeover.notice.takeoverApplied') });
      setOpen(false);
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('gateway.takeover.notice.takeoverFailed', { error: formatGatewayError(error) }),
      });
      await refreshStatus().catch(() => undefined);
    } finally {
      setBusyAction(null);
    }
  };

  const handleRestore = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setBusyAction('restore');
    setNotice(null);
    try {
      const nextStatus = await restoreProxyGatewayCliDirect(cliKey);
      setStatus(nextStatus);
      onStatusChange?.(nextStatus);
      setNotice({ kind: 'success', text: t('gateway.takeover.notice.restored') });
      setOpen(false);
    } catch (error) {
      setNotice({
        kind: 'error',
        text: t('gateway.takeover.notice.restoreFailed', { error: formatGatewayError(error) }),
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
        className={joinClassNames(styles.button, takeoverActive && styles.buttonActive)}
        title={statusMessage}
        onClick={handleOpen}
      >
        <span className={joinClassNames(styles.dot, styles[`dot_${dot}`])} aria-hidden="true" />
        <Network size={13} aria-hidden="true" />
        <span>{t('gateway.takeover.button')}</span>
      </button>

      {open ? (
        <div className={styles.overlay} role="presentation" onClick={() => setOpen(false)}>
          <div
            className={styles.dialog}
            role="dialog"
            aria-modal="true"
            aria-labelledby={`gateway-takeover-title-${cliKey}`}
            onClick={(event) => event.stopPropagation()}
          >
            <div className={styles.dialogHeader}>
              <div className={styles.dialogTitleBlock}>
                <span className={styles.dialogIcon}>
                  <Network size={16} aria-hidden="true" />
                </span>
                <div>
                  <h3 id={`gateway-takeover-title-${cliKey}`}>
                    {t('gateway.takeover.confirmTitle', {
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
              <div className={styles.stateRow}>
                <span className={joinClassNames(styles.dot, styles[`dot_${dot}`])} aria-hidden="true" />
                <span>{t(`gateway.takeover.state.${status?.state ?? 'direct'}`)}</span>
              </div>

              <div className={styles.effectList}>
                <div>
                  <CheckCircle2 size={14} aria-hidden="true" />
                  <span>{t('gateway.takeover.effects.routeToGateway')}</span>
                </div>
                <div>
                  <CheckCircle2 size={14} aria-hidden="true" />
                  <span>{t('gateway.takeover.effects.backupManifest')}</span>
                </div>
                <div>
                  <CheckCircle2 size={14} aria-hidden="true" />
                  <span>{t('gateway.takeover.effects.providerOrder')}</span>
                </div>
                <div>
                  <AlertTriangle size={14} aria-hidden="true" />
                  <span>{t('gateway.takeover.effects.hideApply')}</span>
                </div>
              </div>

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
              ) : null}
            </div>

            <div className={styles.dialogFooter}>
              {status?.can_restore_direct ? (
                <button
                  type="button"
                  className={styles.secondaryButton}
                  disabled={busyAction === 'restore'}
                  onClick={handleRestore}
                >
                  {busyAction === 'restore' ? (
                    <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                  ) : (
                    <RotateCcw size={14} aria-hidden="true" />
                  )}
                  <span>{t('gateway.takeover.actions.restoreDirect')}</span>
                </button>
              ) : null}
              <button type="button" className={styles.secondaryButton} onClick={handleClose}>
                {t('common.cancel')}
              </button>
              <button
                type="button"
                className={styles.primaryButton}
                disabled={!status?.can_takeover || busyAction === 'takeover'}
                onClick={handleTakeover}
              >
                {busyAction === 'takeover' ? (
                  <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                ) : (
                  <Network size={14} aria-hidden="true" />
                )}
                <span>
                  {takeoverActive
                    ? t('gateway.takeover.actions.retakeover')
                    : t('gateway.takeover.actions.takeover')}
                </span>
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </span>
  );
};

export default GatewayTakeoverButton;
