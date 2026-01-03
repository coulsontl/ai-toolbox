import React from 'react';
import { Modal, Form, Input, Radio, Space, Button, type RadioChangeEvent } from 'antd';
import { FolderOpenOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettingsStore, type WebDAVConfigFE } from '@/stores';

interface BackupSettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const BackupSettingsModal: React.FC<BackupSettingsModalProps> = ({
  open: isOpen,
  onClose,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const { backupType, localBackupPath, webdav, setBackupSettings } = useSettingsStore();

  const [currentBackupType, setCurrentBackupType] = React.useState<'local' | 'webdav'>(backupType);
  const [currentLocalPath, setCurrentLocalPath] = React.useState(localBackupPath);

  React.useEffect(() => {
    if (isOpen) {
      setCurrentBackupType(backupType);
      setCurrentLocalPath(localBackupPath);
      form.setFieldsValue({
        backupType,
        webdav,
      });
    }
  }, [isOpen, backupType, localBackupPath, webdav, form]);

  const handleSelectFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t('settings.backupSettings.selectFolder'),
      });
      if (selected) {
        setCurrentLocalPath(selected as string);
      }
    } catch {
      // User cancelled
    }
  };

  const handleSave = async () => {
    try {
      const values = await form.validateFields();
      setBackupSettings({
        backupType: currentBackupType,
        localBackupPath: currentLocalPath,
        webdav: values.webdav as Partial<WebDAVConfigFE>,
      });
      onClose();
    } catch {
      // Validation failed
    }
  };

  const handleBackupTypeChange = (e: RadioChangeEvent) => {
    setCurrentBackupType(e.target.value as 'local' | 'webdav');
  };

  return (
    <Modal
      title={t('settings.backupSettings.title')}
      open={isOpen}
      onOk={handleSave}
      onCancel={onClose}
      width={480}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form form={form} layout="vertical" size="small">
        <Form.Item label={t('settings.backupSettings.storageType')}>
          <Radio.Group value={currentBackupType} onChange={handleBackupTypeChange}>
            <Radio value="local">{t('settings.backupSettings.local')}</Radio>
            <Radio value="webdav">{t('settings.backupSettings.webdav')}</Radio>
          </Radio.Group>
        </Form.Item>

        {currentBackupType === 'local' && (
          <Form.Item label={t('settings.backupSettings.localPath')}>
            <Space.Compact style={{ width: '100%' }}>
              <Input
                value={currentLocalPath}
                readOnly
                placeholder={t('settings.backupSettings.selectFolder')}
                style={{ flex: 1 }}
              />
              <Button icon={<FolderOpenOutlined />} onClick={handleSelectFolder}>
                {t('common.browse')}
              </Button>
            </Space.Compact>
          </Form.Item>
        )}

        {currentBackupType === 'webdav' && (
          <>
            <Form.Item label={t('settings.webdav.url')} name={['webdav', 'url']}>
              <Input placeholder="https://dav.example.com" />
            </Form.Item>
            <Form.Item label={t('settings.webdav.username')} name={['webdav', 'username']}>
              <Input />
            </Form.Item>
            <Form.Item label={t('settings.webdav.password')} name={['webdav', 'password']}>
              <Input.Password visibilityToggle />
            </Form.Item>
            <Form.Item label={t('settings.webdav.remotePath')} name={['webdav', 'remotePath']}>
              <Input placeholder="/backup" />
            </Form.Item>
          </>
        )}
      </Form>
    </Modal>
  );
};

export default BackupSettingsModal;
