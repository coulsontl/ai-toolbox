import React from 'react';
import { App, Button, Modal, Space, Tag, Typography } from 'antd';
import { SafetyCertificateOutlined, WarningOutlined } from '@ant-design/icons';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import {
  CONTROL_API_APPROVAL_EXPIRED_EVENT,
  CONTROL_API_APPROVAL_REQUEST_EVENT,
  listPendingControlApiApprovals,
  resolveControlApiApproval,
  type ControlApiApprovalInfo,
} from '@/services';
import styles from './ControlApiApprovalModal.module.less';

const { Text, Paragraph } = Typography;

interface ExpiredPayload {
  requestId: string;
  reason?: string;
}

/**
 * Global approval gate for the local Agent Control API.
 *
 * Mounted once under the app providers. It queues L1/L2 write requests while
 * the backend request is parked on a oneshot channel, so resolving here
 * unblocks the existing HTTP call instead of polling.
 */
export const ControlApiApprovalModal: React.FC = () => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [queue, setQueue] = React.useState<ControlApiApprovalInfo[]>([]);
  const [secondsLeft, setSecondsLeft] = React.useState(0);
  const [submitting, setSubmitting] = React.useState(false);

  const current = queue[0] ?? null;

  const enqueue = React.useCallback((info: ControlApiApprovalInfo) => {
    setQueue((previous) => {
      if (previous.some((item) => item.requestId === info.requestId)) {
        return previous;
      }
      return [...previous, info];
    });
  }, []);

  const remove = React.useCallback((requestId: string) => {
    setQueue((previous) => previous.filter((item) => item.requestId !== requestId));
  }, []);

  React.useEffect(() => {
    let disposed = false;
    listPendingControlApiApprovals()
      .then((pending) => {
        if (!disposed) {
          pending.forEach(enqueue);
        }
      })
      .catch((error) => {
        // Older builds without the command are harmless; the events still work.
        console.debug('Control API pending approvals unavailable:', error);
      });
    return () => {
      disposed = true;
    };
  }, [enqueue]);

  React.useEffect(() => {
    let unlistenRequest: (() => void) | undefined;
    let unlistenExpired: (() => void) | undefined;
    listen<ControlApiApprovalInfo>(CONTROL_API_APPROVAL_REQUEST_EVENT, (event) => {
      enqueue(event.payload);
    })
      .then((dispose) => {
        unlistenRequest = dispose;
      })
      .catch((error) => console.error('Failed to listen for control API approvals:', error));
    listen<ExpiredPayload>(CONTROL_API_APPROVAL_EXPIRED_EVENT, (event) => {
      remove(event.payload.requestId);
    })
      .then((dispose) => {
        unlistenExpired = dispose;
      })
      .catch((error) => console.error('Failed to listen for control API expiries:', error));
    return () => {
      unlistenRequest?.();
      unlistenExpired?.();
    };
  }, [enqueue, remove]);

  React.useEffect(() => {
    if (!current) {
      setSecondsLeft(0);
      return;
    }
    let remaining = current.timeoutSecs;
    setSecondsLeft(remaining);
    const timer = window.setInterval(() => {
      remaining -= 1;
      setSecondsLeft(Math.max(0, remaining));
      if (remaining <= 0) {
        // The backend answers 408 on its own; removing the card keeps the UI
        // from showing a request that can no longer be approved.
        window.clearInterval(timer);
        remove(current.requestId);
      }
    }, 1000);
    return () => window.clearInterval(timer);
  }, [current, remove]);

  const decide = React.useCallback(
    async (approved: boolean) => {
      if (!current) return;
      setSubmitting(true);
      const requestId = current.requestId;
      try {
        const result = await resolveControlApiApproval(requestId, approved);
        if (!result.resolved) {
          message.warning(t('controlApiApproval.expired'));
        }
      } catch (error) {
        console.error('Failed to resolve control API approval:', error);
        message.error(t('controlApiApproval.resolveFailed', { error: String(error) }));
      } finally {
        remove(requestId);
        setSubmitting(false);
      }
    },
    [current, message, remove, t],
  );

  const risk = current?.risk ?? 'l1';
  const isHighRisk = risk === 'l2';

  return (
    <Modal
      open={current !== null}
      closable={false}
      mask={{ closable: false }}
      keyboard={false}
      centered
      width={520}
      title={
        <Space size={8}>
          {isHighRisk ? (
            <WarningOutlined style={{ color: 'var(--ant-color-error)' }} />
          ) : (
            <SafetyCertificateOutlined style={{ color: 'var(--ant-color-primary)' }} />
          )}
          <span>{t('controlApiApproval.title')}</span>
          <Tag color={isHighRisk ? 'red' : 'blue'}>
            {isHighRisk
              ? t('controlApiApproval.riskHigh')
              : t('controlApiApproval.riskStandard')}
          </Tag>
        </Space>
      }
      footer={
        current ? (
          <div className={styles.footer}>
            <Text type="secondary" className={styles.countdown}>
              {t('controlApiApproval.countdown', { seconds: secondsLeft })}
            </Text>
            <Space size={8}>
              <Button onClick={() => void decide(false)} loading={submitting}>
                {t('controlApiApproval.deny')}
              </Button>
              <Button type="primary" onClick={() => void decide(true)} loading={submitting}>
                {t('controlApiApproval.allowOnce')}
              </Button>
            </Space>
          </div>
        ) : null
      }
    >
      {current && (
        <div className={styles.content}>
          <Paragraph className={styles.summary}>{current.summary}</Paragraph>
          <div className={styles.metaRow}>
            <Text type="secondary">{t('controlApiApproval.action')}</Text>
            <Text code>{current.action}</Text>
          </div>
          {current.target && (
            <div className={styles.metaRow}>
              <Text type="secondary">{t('controlApiApproval.target')}</Text>
              <Text>{current.target}</Text>
            </div>
          )}
          <Text type="secondary" className={styles.note}>
            {isHighRisk
              ? t('controlApiApproval.highRiskNote')
              : t('controlApiApproval.standardNote')}
          </Text>
          {queue.length > 1 && (
            <Text type="secondary" className={styles.note}>
              {t('controlApiApproval.queued', { count: queue.length - 1 })}
            </Text>
          )}
        </div>
      )}
    </Modal>
  );
};

export default ControlApiApprovalModal;
