import React from 'react';
import { Typography, Button, Select, Space, message, Modal, Table, Switch, Progress, Input, Row, Col, Card, Divider } from 'antd';
import {
  EditOutlined,
  CloudUploadOutlined,
  CloudDownloadOutlined,
  GithubOutlined,
  SyncOutlined,
  GlobalOutlined,
  DesktopOutlined,
  InfoCircleOutlined,
  ApiOutlined,
  CloudSyncOutlined,
  AppstoreOutlined,
  CloudServerOutlined,
  BulbOutlined,
  EyeOutlined,
  HolderOutlined,
  DragOutlined
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  SortableContext,
  horizontalListSortingStrategy,
  useSortable,
  arrayMove,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { restrictToHorizontalAxis } from '@dnd-kit/modifiers';
import { useAppStore, useSettingsStore } from '@/stores';
import { useThemeStore, type ThemeMode } from '@/stores/themeStore';
import { languages, type Language } from '@/i18n';
import i18n from '@/i18n';
import { BackupSettingsModal, WebDAVRestoreModal } from '../components';
import { platform } from '@tauri-apps/plugin-os';
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
import styles from './GeneralSettingsPage.module.less';

const { Text } = Typography;

interface SortableCodingChipProps {
  id: string;
  label: string;
  checked: boolean;
  onToggle: (checked: boolean) => void;
  reorderMode: boolean;
}

const SortableCodingChip: React.FC<SortableCodingChipProps> = ({ id, label, checked, onToggle, reorderMode }) => {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id, disabled: !reorderMode });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    zIndex: isDragging ? 10 : undefined,
    opacity: isDragging ? 0.85 : 1,
  };

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={`${styles.tabPill} ${checked ? styles.tabPillActive : styles.tabPillInactive}`}
      data-dragging={isDragging || undefined}
      onClick={() => onToggle(!checked)}
    >
      {reorderMode && (
        <span className={styles.dragHandle} {...attributes} {...listeners} onClick={(e) => e.stopPropagation()}>
          <HolderOutlined />
        </span>
      )}
      <span>{label}</span>
    </div>
  );
};

const GeneralSettingsPage: React.FC = () => {
  const { t } = useTranslation();
  const { language, setLanguage } = useAppStore();
  const { mode: themeMode, setMode: setThemeMode } = useThemeStore();
  const {
    backupType,
    localBackupPath,
    webdav,
    lastBackupTime,
    setLastBackupTime,
    launchOnStartup,
    minimizeToTrayOnClose,
    startMinimized,
    setLaunchOnStartup,
    setMinimizeToTrayOnClose,
    setStartMinimized,
    proxyUrl,
    setProxyUrl,
    autoBackupEnabled,
    autoBackupIntervalDays,
    lastAutoBackupTime,
    autoCheckUpdate,
    setAutoCheckUpdate,
    visibleTabs,
    setVisibleTabs,
  } = useSettingsStore();

  const isWindows = React.useMemo(() => platform() === 'windows', []);

  const [backupModalOpen, setBackupModalOpen] = React.useState(false);
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
    if (autoCheckUpdate) {
      handleCheckUpdate(true);
    }
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
      // toast 由 AppInitializer (providers.tsx) 全局处理，此处仅更新进度状态
    });

    return () => {
      unlisten.then((fn) => fn()).catch(console.error);
    };
  }, [t]);

  // Listen for auto-backup completion to refresh lastAutoBackupTime
  React.useEffect(() => {
    const unlistenCompleted = listen<string>('auto-backup-completed', (event) => {
      useSettingsStore.getState().setLastAutoBackupTime(event.payload);
    });

    const unlistenFailed = listen<string>('auto-backup-failed', (event) => {
      message.error(`${t('settings.autoBackup.autoBackupFailed')}: ${event.payload}`);
    });

    return () => {
      unlistenCompleted.then((fn) => fn()).catch(console.error);
      unlistenFailed.then((fn) => fn()).catch(console.error);
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
          webdav.remotePath,
          webdav.hostLabel
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

      // Parse error if it's JSON
      let errorMessage = t('settings.backupSettings.backupFailed');
      try {
        const errorObj = JSON.parse(String(error));
        if (errorObj.suggestion) {
          errorMessage = `${t('settings.backupSettings.backupFailed')}: ${t(errorObj.suggestion)}`;
        }
      } catch {
        errorMessage = `${t('settings.backupSettings.backupFailed')}: ${String(error)}`;
      }

      message.error(errorMessage);
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

          // Parse error if it's JSON
          let errorMessage = t('settings.backupSettings.restoreFailed');
          try {
            const errorObj = JSON.parse(String(error));
            if (errorObj.suggestion) {
              errorMessage = `${t('settings.backupSettings.restoreFailed')}: ${t(errorObj.suggestion)}`;
            }
          } catch {
            errorMessage = `${t('settings.backupSettings.restoreFailed')}: ${String(error)}`;
          }

          message.error(errorMessage);
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
      message.error(t('settings.openDataDirFailed'));
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

  const CardTitle = ({ icon, title }: { icon: React.ReactNode, title: string }) => (
    <div className={styles.cardTitle}>
      {icon}
      <span>{title}</span>
    </div>
  );

  const SectionTitle = ({ icon, title, extra }: { icon: React.ReactNode, title: string, extra?: React.ReactNode }) => (
    <div className={styles.sectionTitle}>
      <div className={styles.titleLeft}>
        {icon}
        <span>{title}</span>
      </div>
      {extra}
    </div>
  );

  const CODING_TABS = ['opencode', 'claudecode', 'codex', 'openclaw'] as const;
  const OTHER_TABS = ['ssh', ...(isWindows ? ['wsl'] : [])] as string[];

  const [reorderMode, setReorderMode] = React.useState(false);

  // Full coding tab order: visible ones keep their order from visibleTabs,
  // invisible ones stay in their original relative position among coding tabs.
  const [codingTabOrder, setCodingTabOrder] = React.useState<string[]>(() => {
    const fromVisible = visibleTabs.filter((k) => (CODING_TABS as readonly string[]).includes(k));
    const missing = CODING_TABS.filter((k) => !fromVisible.includes(k));
    // Interleave missing tabs back into their default positions
    const result: string[] = [...fromVisible];
    for (const key of missing) {
      const defaultIdx = CODING_TABS.indexOf(key as typeof CODING_TABS[number]);
      // Insert at the position closest to its default index
      let insertAt = result.length;
      for (let i = 0; i < result.length; i++) {
        if (CODING_TABS.indexOf(result[i] as typeof CODING_TABS[number]) > defaultIdx) {
          insertAt = i;
          break;
        }
      }
      result.splice(insertAt, 0, key);
    }
    return result;
  });

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } })
  );

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const oldIndex = codingTabOrder.indexOf(active.id as string);
    const newIndex = codingTabOrder.indexOf(over.id as string);
    const newOrder = arrayMove(codingTabOrder, oldIndex, newIndex);
    setCodingTabOrder(newOrder);
    // Rebuild visibleTabs: visible coding tabs in new order + other tabs
    const visibleCoding = newOrder.filter((k) => visibleTabs.includes(k));
    const visibleOther = OTHER_TABS.filter((k) => visibleTabs.includes(k));
    setVisibleTabs([...visibleCoding, ...visibleOther]);
  };

  const handleCodingTabToggle = (key: string, checked: boolean) => {
    if (checked) {
      // Add: coding tabs in codingTabOrder that are now visible + other tabs
      const visibleCoding = codingTabOrder.filter((k) => visibleTabs.includes(k) || k === key);
      const visibleOther = OTHER_TABS.filter((k) => visibleTabs.includes(k));
      setVisibleTabs([...visibleCoding, ...visibleOther]);
    } else {
      setVisibleTabs(visibleTabs.filter((k) => k !== key));
    }
  };

  const handleOtherTabToggle = (key: string, checked: boolean) => {
    if (checked) {
      setVisibleTabs([...visibleTabs, key]);
    } else {
      setVisibleTabs(visibleTabs.filter((k) => k !== key));
    }
  };

  return (
    <div className={styles.container}>
      <Row gutter={[16, 16]}>
        {/* Left Column: General Settings */}
        <Col xs={24} lg={12}>
          <Card
            title={<CardTitle icon={<AppstoreOutlined style={{ color: '#1890ff' }} />} title={t('settings.cards.general')} />}
            className={styles.card}
          >
            {/* Language Settings */}
            <SectionTitle icon={<GlobalOutlined style={{ color: '#1890ff' }} />} title={t('settings.cards.language')} />
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
              <Text>{t('settings.currentLanguage')}:</Text>
              <Select
                value={language}
                onChange={handleLanguageChange}
                options={languages.map((lang) => ({
                  value: lang.value,
                  label: lang.label,
                }))}
                style={{ width: 160 }}
              />
            </div>

            <Divider />

            {/* Theme Settings */}
            <SectionTitle icon={<BulbOutlined style={{ color: '#faad14' }} />} title={t('settings.cards.theme')} />
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
              <Text>{t('settings.currentTheme')}:</Text>
              <Select
                value={themeMode}
                onChange={(value: ThemeMode) => setThemeMode(value)}
                options={[
                  { value: 'light', label: t('theme.light') },
                  { value: 'dark', label: t('theme.dark') },
                  { value: 'system', label: t('theme.system') },
                ]}
                style={{ width: 160 }}
              />
            </div>

            <Divider />

            {/* Window Settings */}
            <SectionTitle icon={<DesktopOutlined style={{ color: '#13c2c2' }} />} title={t('settings.cards.window')} />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginBottom: 16 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <Text>{t('settings.window.launchOnStartup')}</Text>
                <Switch
                  checked={launchOnStartup}
                  onChange={(checked) => {
                    setLaunchOnStartup(checked);
                    // Disable start minimized when launch on startup is disabled
                    if (!checked && startMinimized) {
                      setStartMinimized(false);
                    }
                  }}
                />
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <Text>{t('settings.window.startMinimized')}</Text>
                <Switch
                  checked={startMinimized}
                  disabled={!launchOnStartup}
                  onChange={setStartMinimized}
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

            <Divider />

            {/* About */}
            <SectionTitle icon={<InfoCircleOutlined style={{ color: '#722ed1' }} />} title={t('settings.cards.about')} />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <Text>{t('settings.about.version')}:</Text>
                <Text strong>{appVersion || '-'}</Text>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <Text>{t('settings.about.autoCheckUpdate')}</Text>
                <Switch
                  checked={autoCheckUpdate}
                  onChange={setAutoCheckUpdate}
                />
              </div>
              <Space wrap>
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
          </Card>
        </Col>

        {/* Right Column: Tab Visibility + Network & Backup */}
        <Col xs={24} lg={12}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
          {/* Tab Visibility Card */}
          <Card
            title={<CardTitle icon={<EyeOutlined style={{ color: '#722ed1' }} />} title={t('settings.cards.tabVisibility')} />}
            className={styles.card}
            extra={
              <Button
                type={reorderMode ? 'primary' : 'text'}
                size="small"
                icon={<DragOutlined />}
                onClick={() => setReorderMode(!reorderMode)}
              >
                {t('settings.tabVisibility.sort')}
              </Button>
            }
          >
            <Text type="secondary" style={{ fontSize: 12, display: 'block', marginBottom: 12 }}>
              {t('settings.tabVisibility.hint')}
            </Text>
            <div className={styles.tabVisibilityRows}>
              <div className={styles.tabVisibilityRow}>
                <Text type="secondary" className={styles.tabVisibilityRowLabel}>
                  {t('settings.tabVisibility.codingTabs')}
                </Text>
                <DndContext
                  sensors={sensors}
                  collisionDetection={closestCenter}
                  modifiers={[restrictToHorizontalAxis]}
                  onDragEnd={handleDragEnd}
                >
                  <SortableContext items={codingTabOrder} strategy={horizontalListSortingStrategy}>
                    <div className={styles.sortableChipList}>
                      {codingTabOrder.map((key) => (
                        <SortableCodingChip
                          key={key}
                          id={key}
                          label={t(`subModules.${key}`)}
                          checked={visibleTabs.includes(key)}
                          onToggle={(checked) => handleCodingTabToggle(key, checked)}
                          reorderMode={reorderMode}
                        />
                      ))}
                    </div>
                  </SortableContext>
                </DndContext>
              </div>
              <div className={styles.tabVisibilityRow}>
                <Text type="secondary" className={styles.tabVisibilityRowLabel}>
                  {t('settings.tabVisibility.otherModules')}
                </Text>
                <div className={styles.otherTabList}>
                  {OTHER_TABS.map((key) => (
                    <div
                      key={key}
                      className={`${styles.tabPill} ${visibleTabs.includes(key) ? styles.tabPillActive : styles.tabPillInactive}`}
                      onClick={() => handleOtherTabToggle(key, !visibleTabs.includes(key))}
                    >
                      <span>{t(`subModules.${key}`)}</span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </Card>

          <Card
            title={<CardTitle icon={<CloudServerOutlined style={{ color: '#52c41a' }} />} title={t('settings.cards.networkBackup')} />}
            className={styles.card}
          >
            {/* Proxy Settings */}
            <SectionTitle icon={<ApiOutlined style={{ color: '#fa8c16' }} />} title={t('settings.cards.proxy')} />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginBottom: 16 }}>
              <div style={{ display: 'flex', gap: 8 }}>
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

            <Divider />

            {/* Backup Settings */}
            <SectionTitle 
              icon={<CloudSyncOutlined style={{ color: '#52c41a' }} />} 
              title={t('settings.cards.backup')} 
              extra={
                <Button
                  type="text"
                  icon={<EditOutlined />}
                  size="small"
                  onClick={() => setBackupModalOpen(true)}
                >
                  {t('common.edit')}
                </Button>
              }
            />
            <Table
              columns={backupColumns}
              dataSource={backupData}
              pagination={false}
              size="small"
              bordered
              style={{ marginBottom: 16 }}
            />
            <Space wrap>
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
            {autoBackupEnabled && (
              <div style={{ marginTop: 12 }}>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('settings.autoBackup.statusEnabled')} | {t('settings.autoBackup.statusInterval', { days: autoBackupIntervalDays })} | {t('settings.autoBackup.lastTime')}: {lastAutoBackupTime ? formatBackupTime(lastAutoBackupTime) : t('settings.autoBackup.neverBackedUp')}
                </Text>
              </div>
            )}
          </Card>
          </div>
        </Col>
      </Row>

      {/* Modals */}
      <BackupSettingsModal open={backupModalOpen} onClose={() => setBackupModalOpen(false)} />
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
                <Text type="secondary" style={{ fontSize: 14 }}>
                  {formatFileSize(updateDownloaded)} / {formatFileSize(updateTotal)}
                </Text>
                <Text style={{ color: '#1890ff', fontSize: 14, fontWeight: 500 }}>
                  {formatSpeed(updateSpeed)}
                </Text>
              </div>
            )}
            {updateStatus === 'installing' && (
              <Text type="secondary" style={{ fontSize: 14 }}>
                {t('settings.about.installingUpdate')}
              </Text>
            )}
            {updateStatus === 'started' && (
              <Text type="secondary" style={{ fontSize: 14 }}>
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
