import React from 'react';
import { Modal, Form, Input, Radio, Space, Button, InputNumber, Switch, Divider, message, type RadioChangeEvent } from 'antd';
import { FolderOpenOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettingsStore, type WebDAVConfigFE } from '@/stores';
import { testWebDAVConnection } from '@/services';

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
  const {
    backupType,
    localBackupPath,
    webdav,
    backupImageAssetsEnabled,
    setBackupSettings,
    autoBackupEnabled,
    autoBackupIntervalDays,
    autoBackupMaxKeep,
    setAutoBackupSettings,
  } = useSettingsStore();

  const [currentBackupType, setCurrentBackupType] = React.useState<'local' | 'webdav'>(backupType);
  const [currentLocalPath, setCurrentLocalPath] = React.useState(localBackupPath);
  const [testingConnection, setTestingConnection] = React.useState(false);
  const [currentBackupImageAssetsEnabled, setCurrentBackupImageAssetsEnabled] =
    React.useState(backupImageAssetsEnabled);
  const [currentAutoBackupEnabled, setCurrentAutoBackupEnabled] = React.useState(autoBackupEnabled);
  const [currentIntervalDays, setCurrentIntervalDays] = React.useState(autoBackupIntervalDays);
  const [currentMaxKeep, setCurrentMaxKeep] = React.useState(autoBackupMaxKeep);

  React.useEffect(() => {
    if (isOpen) {
      setCurrentBackupType(backupType);
      setCurrentLocalPath(localBackupPath);
      setCurrentBackupImageAssetsEnabled(backupImageAssetsEnabled);
      setCurrentAutoBackupEnabled(autoBackupEnabled);
      setCurrentIntervalDays(autoBackupIntervalDays);
      setCurrentMaxKeep(autoBackupMaxKeep);
      form.setFieldsValue({
        backupType,
        webdav,
      });
    }
  }, [isOpen, backupType, localBackupPath, webdav, backupImageAssetsEnabled, autoBackupEnabled, autoBackupIntervalDays, autoBackupMaxKeep, form]);

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
      await setBackupSettings({
        backupType: currentBackupType,
        localBackupPath: currentLocalPath,
        webdav: values.webdav as Partial<WebDAVConfigFE>,
        backupImageAssetsEnabled: currentBackupImageAssetsEnabled,
      });
      await setAutoBackupSettings({
        enabled: currentAutoBackupEnabled,
        intervalDays: currentIntervalDays,
        maxKeep: currentMaxKeep,
      });
      onClose();
    } catch {
      // Validation failed
    }
  };

  const handleBackupTypeChange = (e: RadioChangeEvent) => {
    setCurrentBackupType(e.target.value as 'local' | 'webdav');
  };

  const handleTestConnection = async () => {
    try {
      const values = await form.validateFields(['webdav']);
      const webdavConfig = values.webdav as Partial<WebDAVConfigFE>;

      if (!webdavConfig.url) {
        message.warning(t('settings.webdav.errors.checkUrl'));
        return;
      }

      setTestingConnection(true);
      await testWebDAVConnection(
        webdavConfig.url,
        webdavConfig.username || '',
        webdavConfig.password || '',
        webdavConfig.remotePath || ''
      );
      message.success(t('settings.webdav.testSuccess'));
    } catch (error) {
      console.error('WebDAV connection test failed:', error);

      // Parse error if it's JSON
      let errorMessage = String(error);
      try {
        const errorObj = JSON.parse(String(error));
        if (errorObj.suggestion) {
          errorMessage = t(errorObj.suggestion);
        }
      } catch {
        // Not JSON, use as is
      }

      message.error(`${t('settings.webdav.testFailed')}: ${errorMessage}`);
    } finally {
      setTestingConnection(false);
    }
  };

  return (
    <Modal
      title={t('settings.backupSettings.title')}
      open={isOpen}
      onOk={handleSave}
      onCancel={onClose}
      width={640}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      <Form form={form} layout="horizontal" size="small" labelCol={{ span: 6 }} wrapperCol={{ span: 18 }}>
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
              <Button icon={<FolderOpenOutlined />} onClick={handleSelectFolder} style={{ fontSize: 14 }}>
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
            <Form.Item label={t('settings.webdav.hostLabel')} name={['webdav', 'hostLabel']}>
              <Input placeholder={t('settings.webdav.hostLabelPlaceholder')} />
            </Form.Item>
            <Form.Item wrapperCol={{ offset: 6, span: 18 }}>
              <Button
                onClick={handleTestConnection}
                loading={testingConnection}
              >
                {testingConnection ? t('settings.webdav.testing') : t('settings.webdav.testConnection')}
              </Button>
            </Form.Item>
          </>
        )}

        <Form.Item label={t('settings.backupSettings.imageAssets')}>
          <Switch
            checked={currentBackupImageAssetsEnabled}
            onChange={setCurrentBackupImageAssetsEnabled}
          />
        </Form.Item>

        <Divider />

        <Form.Item label={t('settings.autoBackup.title')}>
          <Switch
            checked={currentAutoBackupEnabled}
            onChange={setCurrentAutoBackupEnabled}
          />
        </Form.Item>
        {currentAutoBackupEnabled && (
          <>
            <Form.Item label={t('settings.autoBackup.interval')}>
              <InputNumber
                value={currentIntervalDays}
                onChange={(v) => setCurrentIntervalDays(v && v >= 1 ? Math.floor(v) : 1)}
                min={1}
                precision={0}
                style={{ width: 60 }}
                addonAfter={t('settings.autoBackup.days')}
              />
            </Form.Item>
            <Form.Item label={t('settings.autoBackup.maxKeep')}>
              <InputNumber
                value={currentMaxKeep}
                onChange={(v) => setCurrentMaxKeep(v != null && v >= 0 ? Math.floor(v) : 0)}
                min={0}
                precision={0}
                style={{ width: 60 }}
                addonAfter={t('settings.autoBackup.count')}
              />
              {currentMaxKeep === 0 && (
                <div style={{ marginTop: 4 }}>
                  <span style={{ fontSize: 12, color: '#faad14' }}>{t('settings.autoBackup.unlimitedHint')}</span>
                </div>
              )}
            </Form.Item>
          </>
        )}
      </Form>
    </Modal>
  );
};

export default BackupSettingsModal;
