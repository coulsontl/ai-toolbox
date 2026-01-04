import React from 'react';
import { Typography, Button, Select, Divider, Space, message, Modal, Table } from 'antd';
import { EditOutlined, CloudUploadOutlined, CloudDownloadOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useAppStore, useSettingsStore } from '@/stores';
import { languages, type Language } from '@/i18n';
import i18n from '@/i18n';
import { BackupSettingsModal, S3SettingsModal, WebDAVRestoreModal } from '../components';
import {
  backupDatabase,
  restoreDatabase,
  selectBackupFile,
  backupToWebDAV,
  restoreFromWebDAV,
  openAppDataDir,
} from '@/services';

const { Title, Text } = Typography;

const GeneralSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const { language, setLanguage } = useAppStore();
  const { backupType, localBackupPath, webdav, s3, lastBackupTime, setLastBackupTime } =
    useSettingsStore();

  const [backupModalOpen, setBackupModalOpen] = React.useState(false);
  const [s3ModalOpen, setS3ModalOpen] = React.useState(false);
  const [webdavRestoreModalOpen, setWebdavRestoreModalOpen] = React.useState(false);
  const [backupLoading, setBackupLoading] = React.useState(false);
  const [restoreLoading, setRestoreLoading] = React.useState(false);

  const handleLanguageChange = (value: Language) => {
    setLanguage(value);
    i18n.changeLanguage(value);
  };

  const maskSecret = (value: string) => {
    if (!value) return t('common.notSet');
    if (value.length <= 4) return '****';
    return value.slice(0, 4) + '****';
  };

  const formatBackupTime = (isoTime: string | null) => {
    if (!isoTime) return t('common.notSet');
    try {
      return new Date(isoTime).toLocaleString();
    } catch {
      return t('common.notSet');
    }
  };

  const handleBackup = async () => {
    setBackupLoading(true);
    try {
      if (backupType === 'webdav') {
        // WebDAV backup
        if (!webdav.url) {
          message.warning(t('settings.backupSettings.noWebDAVConfigured'));
          return;
        }
        const uploadUrl = await backupToWebDAV(
          webdav.url,
          webdav.username,
          webdav.password,
          webdav.remotePath
        );
        const now = new Date().toISOString();
        await setLastBackupTime(now);
        message.success(t('settings.backupSettings.backupSuccess'));
        console.log('Backup uploaded to:', uploadUrl);
      } else {
        // Local backup
        if (!localBackupPath) {
          message.warning(t('settings.backupSettings.noPathConfigured'));
          return;
        }
        const filePath = await backupDatabase(localBackupPath);
        const now = new Date().toISOString();
        await setLastBackupTime(now);
        message.success(t('settings.backupSettings.backupSuccess'));
        console.log('Backup saved to:', filePath);
      }
    } catch (error) {
      console.error('Backup failed:', error);
      message.error(t('settings.backupSettings.backupFailed'));
    } finally {
      setBackupLoading(false);
    }
  };

  const handleRestore = async () => {
    if (backupType === 'webdav') {
      // Show WebDAV file selection modal
      if (!webdav.url) {
        message.warning(t('settings.backupSettings.noWebDAVConfigured'));
        return;
      }
      setWebdavRestoreModalOpen(true);
    } else {
      // Local file selection
      setRestoreLoading(true);
      try {
        const zipFilePath = await selectBackupFile();
        if (!zipFilePath) {
          setRestoreLoading(false);
          return;
        }

        Modal.confirm({
          title: t('settings.backupSettings.confirmRestore'),
          content: t('settings.backupSettings.confirmRestoreDesc'),
          okText: t('common.confirm'),
          cancelText: t('common.cancel'),
          onOk: async () => {
            try {
              await restoreDatabase(zipFilePath);
              message.success(t('settings.backupSettings.restoreSuccess'));
              setTimeout(() => {
                window.location.reload();
              }, 1000);
            } catch (error) {
              console.error('Restore failed:', error);
              message.error(t('settings.backupSettings.restoreFailed'));
            }
          },
        });
      } catch (error) {
        console.error('Restore failed:', error);
        message.error(t('settings.backupSettings.restoreFailed'));
      } finally {
        setRestoreLoading(false);
      }
    }
  };

  const handleWebDAVRestoreSelect = async (filename: string) => {
    Modal.confirm({
      title: t('settings.backupSettings.confirmRestore'),
      content: t('settings.backupSettings.confirmRestoreDesc'),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        setRestoreLoading(true);
        try {
          await restoreFromWebDAV(
            webdav.url,
            webdav.username,
            webdav.password,
            webdav.remotePath,
            filename
          );
          message.success(t('settings.backupSettings.restoreSuccess'));
          setTimeout(() => {
            window.location.reload();
          }, 1000);
        } catch (error) {
          console.error('Restore failed:', error);
          message.error(t('settings.backupSettings.restoreFailed'));
        } finally {
          setRestoreLoading(false);
        }
      },
    });
  };

  const handleOpenDataDir = async () => {
    try {
      await openAppDataDir();
    } catch (error) {
      console.error('Failed to open data directory:', error);
      message.error('打开数据目录失败');
    }
  };

  // Backup settings table data
  const backupColumns = [
    { title: t('settings.backupSettings.storageType'), dataIndex: 'storageType', key: 'storageType' },
    { title: backupType === 'local' ? t('settings.backupSettings.localPath') : t('settings.webdav.url'), dataIndex: 'path', key: 'path' },
    ...(backupType === 'webdav' ? [{ title: t('settings.webdav.username'), dataIndex: 'username', key: 'username' }] : []),
    { title: t('settings.lastBackup'), dataIndex: 'lastBackup', key: 'lastBackup' },
  ];

  const backupData = [
    {
      key: '1',
      storageType: backupType === 'local' ? t('settings.backupSettings.local') : t('settings.backupSettings.webdav'),
      path: backupType === 'local' ? (localBackupPath || t('common.notSet')) : (webdav.url || t('common.notSet')),
      username: webdav.username || t('common.notSet'),
      lastBackup: formatBackupTime(lastBackupTime),
    },
  ];

  // S3 settings table data
  const s3Columns = [
    { title: t('settings.s3.bucket'), dataIndex: 'bucket', key: 'bucket' },
    { title: t('settings.s3.region'), dataIndex: 'region', key: 'region' },
    { title: t('settings.s3.accessKey'), dataIndex: 'accessKey', key: 'accessKey' },
    { title: t('settings.s3.prefix'), dataIndex: 'prefix', key: 'prefix' },
  ];

  const s3Data = [
    {
      key: '1',
      bucket: s3.bucket || t('common.notSet'),
      region: s3.region || t('common.notSet'),
      accessKey: maskSecret(s3.accessKey),
      prefix: s3.prefix || t('common.notSet'),
    },
  ];

  return (
    <div>
      {/* Language Settings */}
      <Title level={5} style={{ marginBottom: 12 }}>
        {t('settings.language')}
      </Title>
      <div style={{ marginBottom: 16 }}>
        <Text style={{ marginRight: 12 }}>{t('settings.currentLanguage')}:</Text>
        <Select
          value={language}
          onChange={handleLanguageChange}
          options={languages.map((lang) => ({
            value: lang.value,
            label: lang.label,
          }))}
          style={{ width: 160 }}
          size="small"
        />
      </div>

      <Divider />

      {/* Backup Settings */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 }}>
        <Title level={5} style={{ margin: 0 }}>
          {t('settings.backupSettings.title')}
        </Title>
        <Button
          type="text"
          icon={<EditOutlined />}
          size="small"
          onClick={() => setBackupModalOpen(true)}
        >
          {t('common.edit')}
        </Button>
      </div>
      <Table
        columns={backupColumns}
        dataSource={backupData}
        pagination={false}
        size="small"
        bordered
        style={{ marginBottom: 16 }}
      />
      <Space>
        <Button
          type="primary"
          icon={<CloudUploadOutlined />}
          onClick={handleBackup}
          loading={backupLoading}
        >
          {t('settings.backupSettings.backupNow')}
        </Button>
        <Button icon={<CloudDownloadOutlined />} onClick={handleRestore} loading={restoreLoading}>
          {t('settings.backupSettings.restoreBackup')}
        </Button>
        <Typography.Link onClick={handleOpenDataDir}>
          {t('settings.backupSettings.openDataDir')}
        </Typography.Link>
      </Space>

      <Divider />

      {/* S3 Settings */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 }}>
        <Title level={5} style={{ margin: 0 }}>
          {t('settings.s3.title')}
        </Title>
        <Button
          type="text"
          icon={<EditOutlined />}
          size="small"
          onClick={() => setS3ModalOpen(true)}
        >
          {t('common.edit')}
        </Button>
      </div>
      <Table
        columns={s3Columns}
        dataSource={s3Data}
        pagination={false}
        size="small"
        bordered
      />

      {/* Modals */}
      <BackupSettingsModal open={backupModalOpen} onClose={() => setBackupModalOpen(false)} />
      <S3SettingsModal open={s3ModalOpen} onClose={() => setS3ModalOpen(false)} />
      <WebDAVRestoreModal
        open={webdavRestoreModalOpen}
        onClose={() => setWebdavRestoreModalOpen(false)}
        onSelect={handleWebDAVRestoreSelect}
        url={webdav.url}
        username={webdav.username}
        password={webdav.password}
        remotePath={webdav.remotePath}
      />
    </div>
  );
};

export default GeneralSettingsPage;
