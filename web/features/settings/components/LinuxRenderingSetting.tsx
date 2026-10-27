import React from 'react';
import { Button, Select, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import {
  getLinuxWebviewRendering,
  setLinuxWebviewRendering,
  restartApp,
  type LinuxRenderingInfo,
  type LinuxRenderingMode,
} from '@/services/settingsApi';
import styles from './LinuxRenderingSetting.module.less';

export default function LinuxRenderingSetting() {
  const { t } = useTranslation();
  const [info, setInfo] = React.useState<LinuxRenderingInfo | null>(null);
  const [busy, setBusy] = React.useState(true);
  const [error, setError] = React.useState('');
  const pending = React.useRef(false);
  const mounted = React.useRef(true);

  const run = React.useCallback(async (operation: () => Promise<LinuxRenderingInfo | void>) => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError('');
    try {
      const result = await operation();
      if (mounted.current && result) setInfo(result);
    } catch (reason) {
      if (mounted.current) setError(String(reason));
    } finally {
      pending.current = false;
      if (mounted.current) setBusy(false);
    }
  }, []);

  React.useEffect(() => {
    mounted.current = true;
    void run(getLinuxWebviewRendering);
    return () => { mounted.current = false; };
  }, [run]);

  const modeLabel = (mode: LinuxRenderingMode) => t(`settings.rendering.${mode}`);
  return (
    <div className={styles.row}>
      <div className={styles.content}>
        <Typography.Text>{t('settings.rendering.title')}</Typography.Text>
        <Typography.Text type="secondary" className={styles.hint}>
          {t('settings.rendering.description')}
        </Typography.Text>
        {info?.environment_override && (
          <Typography.Text type="secondary" className={styles.hint}>
            {t('settings.rendering.environmentOverride')}
          </Typography.Text>
        )}
        {info?.active_mode === 'performance' && (info.workaround_level ?? 0) > 0 && !info.environment_override && (
          <Typography.Text type="secondary" className={styles.hint}>
            {t('settings.rendering.automaticFallback')}
          </Typography.Text>
        )}
        {info?.restart_required && (
          <Typography.Text type="secondary" className={styles.hint} role="status">
            {t('settings.rendering.restartRequired', { mode: modeLabel(info.active_mode) })}
          </Typography.Text>
        )}
        {error && (
          <Typography.Text type="danger" className={styles.hint} role="alert">
            {t('settings.rendering.failed', { error })}
          </Typography.Text>
        )}
      </div>
      <div className={styles.controls}>
        <Select<LinuxRenderingMode>
          className={styles.select}
          aria-label={t('settings.rendering.title')}
          value={info?.mode}
          loading={busy}
          disabled={busy || !info || info.environment_override}
          options={(['compatible', 'performance'] as const).map(mode => ({ value: mode, label: modeLabel(mode) }))}
          onChange={mode => void run(() => setLinuxWebviewRendering(mode))}
        />
        {info?.restart_required && (
          <Button disabled={busy} onClick={() => void run(restartApp)}>{t('common.restart')}</Button>
        )}
        {!info && error && (
          <Button disabled={busy} onClick={() => void run(getLinuxWebviewRendering)}>{t('common.retry')}</Button>
        )}
      </div>
    </div>
  );
}
