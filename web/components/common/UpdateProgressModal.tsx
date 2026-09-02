import React from 'react';
import { Modal, Progress, Typography } from 'antd';
import i18n from '@/i18n';
import { formatFileSize, formatSpeed } from './updateProgressFormat';

const { Text } = Typography;

export { formatFileSize, formatSpeed };

export interface UpdateProgressModalProps {
  /** Whether the modal is visible. */
  open: boolean;
  /** Download progress percentage (0–100). */
  progress: number;
  /** Current update status: 'started' | 'downloading' | 'installing'. */
  status: string;
  /** Current download speed in bytes per second. */
  speed: number;
  /** Bytes downloaded so far. */
  downloaded: number;
  /** Total bytes to download (0 while unknown). */
  total: number;
}

/**
 * Update download + install progress modal.
 *
 * Pure presentational component shared by the settings page update flow
 * (`GeneralSettingsPage`), the global `AppInitializer`, and the startup
 * recovery screen (`RecoveryApp`). Listens to nothing itself — the caller
 * wires `update-download-progress` event state into the props.
 *
 * Visuals intentionally preserve the established progress bar styling.
 */
const UpdateProgressModal: React.FC<UpdateProgressModalProps> = ({
  open,
  progress,
  status,
  speed,
  downloaded,
  total,
}) => {
  return (
    <Modal title={i18n.t('settings.about.downloadingUpdate')} open={open} closable={false} footer={null}>
      <div style={{ padding: '20px 0' }}>
        <Progress
          percent={progress}
          status="active"
          strokeColor={{
            '0%': '#108ee9',
            '100%': '#87d068',
          }}
        />
        <div style={{ marginTop: 16 }}>
          {status === 'downloading' && (
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <Text type="secondary" style={{ fontSize: 14 }}>
                {formatFileSize(downloaded)} / {formatFileSize(total)}
              </Text>
              <Text style={{ color: '#1890ff', fontSize: 14, fontWeight: 500 }}>
                {formatSpeed(speed)}
              </Text>
            </div>
          )}
          {status === 'installing' && (
            <Text type="secondary" style={{ fontSize: 14 }}>
              {i18n.t('settings.about.installingUpdate')}
            </Text>
          )}
          {status === 'started' && (
            <Text type="secondary" style={{ fontSize: 14 }}>
              {i18n.t('settings.about.downloadingUpdate')}
            </Text>
          )}
        </div>
      </div>
    </Modal>
  );
};

export default UpdateProgressModal;
