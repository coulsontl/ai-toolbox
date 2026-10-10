import React from 'react';
import { Alert, App, Button, Modal, Tag, Typography } from 'antd';
import { Copy, RefreshCw, Terminal } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { copyTextToClipboard } from '@/services/clipboardApi';
import type { OmpCodexSubscription } from '@/types/ohMyPi';
import styles from './OmpCodexSubscriptionSection.module.less';

interface Props {
  subscription: OmpCodexSubscription;
  disabled?: boolean;
  onRefresh: () => Promise<void>;
}

const STATUS_KEYS = {
  configured: 'ohMyPi.codexSubscription.configured',
  not_configured: 'ohMyPi.codexSubscription.notConfigured',
  unknown: 'ohMyPi.codexSubscription.unknown',
} as const;

const OmpCodexSubscriptionSection: React.FC<Props> = ({ subscription, disabled, onRefresh }) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [guideOpen, setGuideOpen] = React.useState(false);
  const [refreshing, setRefreshing] = React.useState(false);
  const refreshInFlightRef = React.useRef(false);

  const handleRefresh = async () => {
    // The ref also protects two clicks before React commits the loading state.
    if (refreshInFlightRef.current || disabled) return;
    refreshInFlightRef.current = true;
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      refreshInFlightRef.current = false;
      setRefreshing(false);
    }
  };

  const handleCopy = async (command: string) => {
    if (!command) return;
    try {
      await copyTextToClipboard(command);
      message.success(t('common.copied'));
    } catch {
      message.error(t('common.error'));
    }
  };

  const status = (
    <Tag color={subscription.status === 'unknown' ? 'warning' : 'default'}>
      {t(STATUS_KEYS[subscription.status])}
    </Tag>
  );
  const overrideWarning = subscription.hasApiKeyOverride ? (
    <Alert type="warning" showIcon title={t('ohMyPi.codexSubscription.apiKeyOverride')} />
  ) : null;

  return (
    <section className={styles.section} aria-label={t('ohMyPi.codexSubscription.title')}>
      <div className={styles.header}>
        <div className={styles.heading}>
          <Typography.Text strong>{t('ohMyPi.codexSubscription.title')}</Typography.Text>
          {status}
        </div>
        <div className={styles.actions}>
          <Button icon={<Terminal size={14} />} onClick={() => setGuideOpen(true)}>
            {t('ohMyPi.codexSubscription.loginGuide')}
          </Button>
          <Button icon={<RefreshCw size={14} />} loading={refreshing} disabled={disabled} onClick={handleRefresh}>
            {t('common.refresh')}
          </Button>
        </div>
      </div>
      <Typography.Paragraph className={styles.hint}>
        {t('ohMyPi.codexSubscription.description')}
      </Typography.Paragraph>
      {overrideWarning}
      <Modal
        open={guideOpen}
        title={t('ohMyPi.codexSubscription.loginGuide')}
        onCancel={() => setGuideOpen(false)}
        width={680}
        footer={[
          <Button key="close" onClick={() => setGuideOpen(false)}>{t('common.close')}</Button>,
          <Button key="refresh" type="primary" loading={refreshing} disabled={disabled} onClick={handleRefresh}>
            {t('ohMyPi.codexSubscription.refreshAfterLogin')}
          </Button>,
        ]}
      >
        <div className={styles.guide}>
          <div className={styles.heading}>{status}</div>
          <Typography.Paragraph className={styles.hint}>
            {t('ohMyPi.codexSubscription.localStatusHint')}
          </Typography.Paragraph>
          {overrideWarning}
          <Typography.Text>
            {subscription.wslDistro
              ? t('ohMyPi.codexSubscription.wslShell', { distro: subscription.wslDistro })
              : subscription.shell === 'powershell'
                ? t('ohMyPi.codexSubscription.powershell')
                : t('ohMyPi.codexSubscription.posixShell')}
          </Typography.Text>
          {[
            { label: t('ohMyPi.codexSubscription.loginStep'), command: subscription.loginCommand },
            { label: t('ohMyPi.codexSubscription.modelsStep'), command: subscription.modelsCommand },
          ].map(({ label, command }) => (
            <div className={styles.commandGroup} key={label}>
              <Typography.Text strong>{label}</Typography.Text>
              <div className={styles.commandRow}>
                <code className={styles.command}>{command || t('ohMyPi.codexSubscription.commandUnavailable')}</code>
                <Button
                  icon={<Copy size={14} />}
                  disabled={!command}
                  onClick={() => void handleCopy(command)}
                  aria-label={t('ohMyPi.codexSubscription.copyCommand', { step: label })}
                >
                  {t('common.copy')}
                </Button>
              </div>
            </div>
          ))}
          <Typography.Paragraph className={styles.hint}>
            {t('ohMyPi.codexSubscription.finishHint')}
          </Typography.Paragraph>
        </div>
      </Modal>
    </section>
  );
};

export default OmpCodexSubscriptionSection;
