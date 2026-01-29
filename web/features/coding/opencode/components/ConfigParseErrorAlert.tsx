import React from 'react';
import { Alert, Button, Space, Typography, Modal } from 'antd';
import { DeleteOutlined, FolderOpenOutlined, ExclamationCircleOutlined, ReloadOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
import { backupOpenCodeConfig } from '@/services/opencodeApi';

const { Text, Paragraph } = Typography;

interface ConfigParseErrorAlertProps {
  path: string;
  error: string;
  contentPreview?: string;
  onBackedUp: () => void;
}

const ConfigParseErrorAlert: React.FC<ConfigParseErrorAlertProps> = ({
  path,
  error,
  contentPreview,
  onBackedUp,
}) => {
  const { t } = useTranslation();
  const [backingUp, setBackingUp] = React.useState(false);

  const handleOpenFolder = async () => {
    try {
      // Try to reveal the file in explorer
      await revealItemInDir(path);
    } catch {
      // If file doesn't exist, fallback to opening parent directory
      try {
        const parentDir = path.replace(/[\\/][^\\/]+$/, '');
        await invoke('open_folder', { path: parentDir });
      } catch (err) {
        console.error('Failed to open folder:', err);
      }
    }
  };

  const handleBackup = () => {
    Modal.confirm({
      title: t('opencode.configParseError.confirmBackupTitle'),
      icon: <ExclamationCircleOutlined />,
      content: t('opencode.configParseError.confirmBackupContent'),
      okText: t('opencode.configParseError.backupAndRecreate'),
      okType: 'danger',
      cancelText: t('common.cancel'),
      onOk: async () => {
        setBackingUp(true);
        try {
          await backupOpenCodeConfig();
          onBackedUp();
        } catch (err) {
          console.error('Failed to backup config:', err);
        } finally {
          setBackingUp(false);
        }
      },
    });
  };

  return (
    <Alert
      type="error"
      showIcon
      message={t('opencode.configParseError.title')}
      description={
        <Space direction="vertical" style={{ width: '100%' }}>
          <Paragraph style={{ marginBottom: 8 }}>
            {t('opencode.configParseError.description')}
          </Paragraph>

          <div>
            <Text type="secondary">{t('opencode.configParseError.filePath')}: </Text>
            <Text code>{path}</Text>
          </div>

          <div>
            <Text type="secondary">{t('opencode.configParseError.errorDetail')}: </Text>
            <Text type="danger">{error}</Text>
          </div>

          {contentPreview && (
            <div style={{ marginTop: 8 }}>
              <Text type="secondary">{t('opencode.configParseError.contentPreview')}:</Text>
              <pre
                style={{
                  background: 'var(--color-bg-elevated)',
                  padding: 8,
                  borderRadius: 4,
                  maxHeight: 150,
                  overflow: 'auto',
                  fontSize: 12,
                  marginTop: 4,
                }}
              >
                {contentPreview}
              </pre>
            </div>
          )}

          <Space style={{ marginTop: 12 }}>
            <Button icon={<FolderOpenOutlined />} onClick={handleOpenFolder}>
              {t('opencode.configParseError.openFolder')}
            </Button>
            <Button
              danger
              icon={<DeleteOutlined />}
              onClick={handleBackup}
              loading={backingUp}
            >
              {t('opencode.configParseError.backupAndRecreate')}
            </Button>
            <Button
              type="primary"
              icon={<ReloadOutlined />}
              onClick={onBackedUp}
            >
              {t('opencode.configParseError.reload')}
            </Button>
          </Space>
        </Space>
      }
      style={{ marginBottom: 16 }}
    />
  );
};

export default ConfigParseErrorAlert;
