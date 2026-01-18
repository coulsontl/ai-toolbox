import React from 'react';
import { Typography, Button, Select, Divider, Space, message, Modal, Table, Switch, Progress, Input } from 'antd';
import { EditOutlined, CloudUploadOutlined, CloudDownloadOutlined, GithubOutlined, SyncOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useAppStore, useSettingsStore } from '@/stores';
import { languages, type Language } from '@/i18n';
import i18n from '@/i18n';
import { BackupSettingsModal /* S3SettingsModal */, WebDAVRestoreModal } from '../components';
import {
  backupDatabase,
  restoreDatabase,
  selectBackupFile,
  backupToWebDAV,
  restoreFromWebDAV,
  openAppDataDir,
  getAppVersion,
  checkForUpdates,
  openGitHubPage,
  openExternalUrl,
  installUpdate,
  testProxyConnection,
  type UpdateInfo,
  GITHUB_REPO,
} from '@/services';
import { restartApp } from '@/services/settingsApi';
import { listen } from '@tauri-apps/api/event';

const { Title, Text } = Typography;

const GeneralSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const { language, setLanguage } = useAppStore();
  const {
    backupType,
    localBackupPath,
    webdav,
    // s3,
    lastBackupTime,
    setLastBackupTime,
    launchOnStartup,
    minimizeToTrayOnClose,
    setLaunchOnStartup,
    setMinimizeToTrayOnClose,
    proxyUrl,
    setProxyUrl,
  } = useSettingsStore();

  const [backupModalOpen, setBackupModalOpen] = React.useState(false);
  // const [s3ModalOpen, setS3ModalOpen] = React.useState(false);
  const [webdavRestoreModalOpen, setWebdavRestoreModalOpen] = React.useState(false);
  const [backupLoading, setBackupLoading] = React.useState(false);
  const [restoreLoading, setRestoreLoading] = React.useState(false);

  // Proxy settings states
  const [proxyInput, setProxyInput] = React.useState(proxyUrl);
  const [proxyTesting, setProxyTesting] = React.useState(false);

  // Version and update states
  const [appVersion, setAppVersion] = React.useState<string>('');
  const [checkingUpdate, setCheckingUpdate] = React.useState(false);
  const [updateInfo, setUpdateInfo] = React.useState<UpdateInfo | null>(null);
  const [updateProgress, setUpdateProgress] = React.useState<number>(0);
  const [updateStatus, setUpdateStatus] = React.useState<string>('');
  const [updateSpeed, setUpdateSpeed] = React.useState<number>(0);
  const [updateDownloaded, setUpdateDownloaded] = React.useState<number>(0);
  const [updateTotal, setUpdateTotal] = React.useState<number>(0);
  const [updateModalOpen, setUpdateModalOpen] = React.useState(false);

  // Load app version on mount
  React.useEffect(() => {
    getAppVersion().then(setAppVersion).catch(console.error);
  }, []);

  // Auto check for updates on mount
  React.useEffect(() => {
    handleCheckUpdate(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Listen for update download progress
  React.useEffect(() => {
    const unlisten = listen<{
      status: string;
      progress: number;
      downloaded: number;
      total: number;
      speed: number;
    }>('update-download-progress', (event) => {
      const { status, progress, downloaded, total, speed } = event.payload;
      setUpdateStatus(status);
      setUpdateProgress(progress);
      setUpdateSpeed(speed);
      setUpdateDownloaded(downloaded);
      setUpdateTotal(total);

      if (status === 'installing') {
        message.success(t('settings.about.downloadingComplete'));
      }
    });

    return () => {
      unlisten.then((fn) => fn()).catch(console.error);
    };
  }, [t]);

  // Sync proxyInput with proxyUrl from store
  React.useEffect(() => {
    setProxyInput(proxyUrl);
  }, [proxyUrl]);

  const handleCheckUpdate = async (silent = false) => {
    setCheckingUpdate(true);
    setUpdateInfo(null);
    try {
      const info = await checkForUpdates();
      setUpdateInfo(info);
      if (!silent) {
        if (info.hasUpdate) {
          message.info(t('settings.about.updateAvailable', { version: info.latestVersion }));
        } else {
          message.success(t('settings.about.latestVersion'));
        }
      }
    } catch (error) {
      console.error('Check update failed:', error);
      if (!silent) {
        message.error(t('settings.about.checkFailed'));
      }
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleOpenGitHub = async () => {
    try {
      await openGitHubPage();
    } catch (error) {
      console.error('Failed to open GitHub:', error);
    }
  };

  const handleGoToDownload = async () => {
    // 如果有 signature 和 url，尝试自动更新
    if (updateInfo?.signature && updateInfo?.url) {
      // 打开更新进度模态框
      setUpdateModalOpen(true);
      setUpdateProgress(0);
      setUpdateStatus('started');
      setUpdateSpeed(0);
      setUpdateDownloaded(0);
      setUpdateTotal(0);

      try {
        await installUpdate();
        setUpdateModalOpen(false);
        // 更新安装成功后需要手动重启
        Modal.success({
          title: t('settings.about.updateComplete'),
          content: t('settings.about.updateCompleteRestart'),
          okText: t('common.restart'),
          onOk: () => {
            restartApp();
          },
        });
      } catch (error) {
        console.error('Failed to install update:', error);
        setUpdateModalOpen(false);

        // 下载失败，提示去 GitHub Actions 下载
        const githubActionsUrl = `https://github.com/${GITHUB_REPO}/actions`;
        Modal.error({
          title: t('settings.about.updateFailed'),
          content: (
            <div>
              <p>{t('settings.about.updateFailedMessage')}</p>
              <p style={{ marginTop: 8 }}>
                <Typography.Link onClick={() => openExternalUrl(githubActionsUrl)}>
                  {t('settings.about.goToGitHubActions')}
                </Typography.Link>
              </p>
            </div>
          ),
          okText: t('common.close'),
        });
      }
    } else if (updateInfo?.releaseUrl) {
      // 没有签名信息，打开外部下载链接
      try {
        await openExternalUrl(updateInfo.releaseUrl);
      } catch (error) {
        console.error('Failed to open release page:', error);
      }
    }
  };

  const handleLanguageChange = (value: Language) => {
    setLanguage(value);
    i18n.changeLanguage(value);
  };

  // const maskSecret = (value: string) => {
  //   if (!value) return t('common.notSet');
  //   if (value.length <= 4) return '****';
  //   return value.slice(0, 4) + '****';
  // };

  const formatBackupTime = (isoTime: string | null) => {
    if (!isoTime) return t('common.notSet');
    try {
      return new Date(isoTime).toLocaleString();
    } catch {
      return t('common.notSet');
    }
  };

  // 格式化文件大小
  const formatFileSize = (bytes: number) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  };

  // 格式化下载速度
  const formatSpeed = (bytesPerSecond: number) => {
    if (bytesPerSecond === 0) return '0 B/s';
    const k = 1024;
    const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
    const i = Math.floor(Math.log(bytesPerSecond) / Math.log(k));
    return parseFloat((bytesPerSecond / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  };

  // 格式化剩余时间

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
              // 恢复成功后弹出重启对话框
              Modal.info({
                title: t('settings.backupSettings.restoreSuccess'),
                content: t('settings.backupSettings.restoreSuccessReload'),
                okText: t('common.restart'),
                onOk: () => {
                  restartApp();
                },
              });
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
          // 恢复成功后弹出重启对话框
          Modal.info({
            title: t('settings.backupSettings.restoreSuccess'),
            content: t('settings.backupSettings.restoreSuccessReload'),
            okText: t('common.restart'),
            onOk: () => {
              restartApp();
            },
          });
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

  // Save proxy URL when input loses focus
  const handleProxySave = async () => {
    if (proxyInput !== proxyUrl) {
      try {
        await setProxyUrl(proxyInput);
        message.success(t('common.success'));
      } catch (error) {
        console.error('Failed to save proxy:', error);
        message.error(t('common.error'));
      }
    }
  };

  // Test proxy connection
  const handleProxyTest = async () => {
    if (!proxyInput) {
      message.warning(t('settings.proxy.urlRequired'));
      return;
    }

    setProxyTesting(true);
    try {
      await testProxyConnection(proxyInput);
      message.success(t('settings.proxy.testSuccess'));
    } catch (error) {
      console.error('Proxy test failed:', error);
      message.error(t('settings.proxy.testFailed') + ': ' + String(error));
    } finally {
      setProxyTesting(false);
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
  // const s3Columns = [
  //   { title: t('settings.s3.bucket'), dataIndex: 'bucket', key: 'bucket' },
  //   { title: t('settings.s3.region'), dataIndex: 'region', key: 'region' },
  //   { title: t('settings.s3.accessKey'), dataIndex: 'accessKey', key: 'accessKey' },
  //   { title: t('settings.s3.prefix'), dataIndex: 'prefix', key: 'prefix' },
  // ];

  // const s3Data = [
  //   {
  //     key: '1',
  //     bucket: s3.bucket || t('common.notSet'),
  //     region: s3.region || t('common.notSet'),
  //     accessKey: maskSecret(s3.accessKey),
  //     prefix: s3.prefix || t('common.notSet'),
  //   },
  // ];

  return (
    <div className="settings-container">
      {/* Language Settings */}
      <div className="settings-card">
        <Title level={5} className="settings-card-title-only">
          {t('settings.language')}
        </Title>
        <div className="settings-card-content">
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
      </div>

      <Divider />

      {/* Window Settings */}
      <div className="settings-card">
        <Title level={5} className="settings-card-title-only">
          {t('settings.window.title')}
        </Title>
        <div className="settings-card-content">
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 }}>
            <Text>{t('settings.window.launchOnStartup')}</Text>
            <Switch
              checked={launchOnStartup}
              onChange={setLaunchOnStartup}
            />
          </div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <Text>{t('settings.window.minimizeToTrayOnClose')}</Text>
            <Switch
              checked={minimizeToTrayOnClose}
              onChange={setMinimizeToTrayOnClose}
            />
          </div>
        </div>
      </div>

      <Divider />

      {/* Proxy Settings */}
      <div className="settings-card">
        <Title level={5} className="settings-card-title-only">
          {t('settings.proxy.title')}
        </Title>
        <div className="settings-card-content">
          <div style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
            <Input
              value={proxyInput}
              onChange={(e) => setProxyInput(e.target.value)}
              onBlur={handleProxySave}
              onPressEnter={handleProxySave}
              placeholder={t('settings.proxy.urlPlaceholder')}
              style={{ flex: 1 }}
            />
            <Button
              onClick={handleProxyTest}
              loading={proxyTesting}
            >
              {proxyTesting ? t('settings.proxy.testing') : t('settings.proxy.testConnection')}
            </Button>
          </div>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t('settings.proxy.hint')}
          </Text>
        </div>
      </div>

      <Divider />

      {/* Backup Settings */}
      <div className="settings-card">
        <div className="settings-card-header">
          <Title level={5} className="settings-card-title" style={{ margin: 0 }}>
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
        <div className="settings-card-content">
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
            <Typography.Link onClick={handleOpenDataDir} style={{ fontSize: 14 }}>
              {t('settings.backupSettings.openDataDir')}
            </Typography.Link>
          </Space>
        </div>
      </div>

      <Divider />

      {/* S3 Settings */}
      {/* <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 }}>
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

      <Divider /> */}

      {/* About */}
      <div className="settings-card">
        <Title level={5} className="settings-card-title-only">
          {t('settings.about.title')}
        </Title>
        <div className="settings-card-content">
          <Text style={{ marginRight: 12 }}>{t('settings.about.version')}:</Text>
          <Text strong>{appVersion || '-'}</Text>
        </div>
        <div className="settings-card-content" style={{ paddingTop: 0 }}>
          <Space>
            <Button
              icon={<SyncOutlined spin={checkingUpdate} />}
              onClick={() => handleCheckUpdate()}
              loading={checkingUpdate}
            >
              {checkingUpdate ? t('settings.about.checking') : t('settings.about.checkUpdate')}
            </Button>
            {updateInfo?.hasUpdate && (
              <Button type="primary" onClick={handleGoToDownload}>
                {t('settings.about.goToDownload')} (v{updateInfo.latestVersion})
              </Button>
            )}
            <Button icon={<GithubOutlined />} onClick={handleOpenGitHub}>
              {t('settings.about.github')}
            </Button>
          </Space>
        </div>
      </div>

      {/* Modals */}
      <BackupSettingsModal open={backupModalOpen} onClose={() => setBackupModalOpen(false)} />
      {/* <S3SettingsModal open={s3ModalOpen} onClose={() => setS3ModalOpen(false)} /> */}
      <WebDAVRestoreModal
        open={webdavRestoreModalOpen}
        onClose={() => setWebdavRestoreModalOpen(false)}
        onSelect={handleWebDAVRestoreSelect}
        url={webdav.url}
        username={webdav.username}
        password={webdav.password}
        remotePath={webdav.remotePath}
      />

      {/* Update Progress Modal */}
      <Modal
        title={t('settings.about.downloadingUpdate')}
        open={updateModalOpen}
        closable={false}
        footer={null}
        centered
      >
        <div style={{ padding: '20px 0' }}>
          <Progress
            percent={updateProgress}
            status={updateStatus === 'installing' ? 'active' : 'active'}
            strokeColor={{
              '0%': '#108ee9',
              '100%': '#87d068',
            }}
          />
          <div style={{ marginTop: 16 }}>
            {updateStatus === 'downloading' && (
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <Text style={{ color: '#666', fontSize: 14 }}>
                  {formatFileSize(updateDownloaded)} / {formatFileSize(updateTotal)}
                </Text>
                <Text style={{ color: '#1890ff', fontSize: 14, fontWeight: 500 }}>
                  {formatSpeed(updateSpeed)}
                </Text>
              </div>
            )}
            {updateStatus === 'installing' && (
              <Text style={{ color: '#666', fontSize: 14 }}>
                {t('settings.about.installingUpdate')}
              </Text>
            )}
            {updateStatus === 'started' && (
              <Text style={{ color: '#666', fontSize: 14 }}>
                {t('settings.about.downloadingUpdate')}
              </Text>
            )}
          </div>
        </div>
      </Modal>
    </div>
  );
};

export default GeneralSettingsPage;
