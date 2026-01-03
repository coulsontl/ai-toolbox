import React from 'react';
import { Modal, List, Empty, Spin, message } from 'antd';
import { FileZipOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { listWebDAVBackups } from '@/services';

interface WebDAVRestoreModalProps {
  open: boolean;
  onClose: () => void;
  onSelect: (filename: string) => void;
  url: string;
  username: string;
  password: string;
  remotePath: string;
}

const WebDAVRestoreModal: React.FC<WebDAVRestoreModalProps> = ({
  open,
  onClose,
  onSelect,
  url,
  username,
  password,
  remotePath,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [backups, setBackups] = React.useState<string[]>([]);

  React.useEffect(() => {
    if (open) {
      loadBackups();
    }
  }, [open]);

  const loadBackups = async () => {
    if (!url) {
      message.warning(t('settings.backupSettings.noWebDAVConfigured'));
      return;
    }

    setLoading(true);
    try {
      const files = await listWebDAVBackups(url, username, password, remotePath);
      setBackups(files);
    } catch (error) {
      console.error('Failed to list backups:', error);
      message.error(t('settings.backupSettings.listBackupsFailed'));
    } finally {
      setLoading(false);
    }
  };

  const handleSelect = (filename: string) => {
    onSelect(filename);
    onClose();
  };

  // Extract date from filename for display
  const formatBackupName = (filename: string) => {
    // ai-toolbox-backup-20260101-120000.zip -> 2026-01-01 12:00:00
    const match = filename.match(/ai-toolbox-backup-(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})\.zip/);
    if (match) {
      const [, year, month, day, hour, min, sec] = match;
      return `${year}-${month}-${day} ${hour}:${min}:${sec}`;
    }
    return filename;
  };

  return (
    <Modal
      title={t('settings.backupSettings.selectBackupFile')}
      open={open}
      onCancel={onClose}
      footer={null}
      width={480}
    >
      {loading ? (
        <div style={{ display: 'flex', justifyContent: 'center', padding: 40 }}>
          <Spin />
        </div>
      ) : backups.length === 0 ? (
        <Empty description={t('settings.backupSettings.noBackupsFound')} />
      ) : (
        <List
          dataSource={backups}
          renderItem={(item) => (
            <List.Item
              style={{ cursor: 'pointer' }}
              onClick={() => handleSelect(item)}
            >
              <List.Item.Meta
                avatar={<FileZipOutlined style={{ fontSize: 24, color: '#1890ff' }} />}
                title={formatBackupName(item)}
                description={item}
              />
            </List.Item>
          )}
        />
      )}
    </Modal>
  );
};

export default WebDAVRestoreModal;
