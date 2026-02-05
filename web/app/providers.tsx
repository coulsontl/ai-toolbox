import React from 'react';
import { ConfigProvider, Spin, App, theme as antdTheme, Button, Modal, Progress, Typography, Space } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import enUS from 'antd/locale/en_US';
import { emit, listen } from '@tauri-apps/api/event';
import { useAppStore, useSettingsStore } from '@/stores';
import { useThemeStore } from '@/stores/themeStore';
import { checkForUpdates, openExternalUrl, setWindowBackgroundColor, installUpdate, GITHUB_REPO, type UpdateInfo } from '@/services';
import { restartApp } from '@/services/settingsApi';
import i18n from '@/i18n';

interface ProvidersProps {
  children: React.ReactNode;
}

const antdLocales = {
  'zh-CN': zhCN,
  'en-US': enUS,
};

/**
 * Inner component that uses App.useApp() to get theme-aware notification
 */
const { Text } = Typography;

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

const AppInitializer: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { notification, message } = App.useApp();
  const hasCheckedUpdate = React.useRef(false);

  // Update progress states
  const [updateModalOpen, setUpdateModalOpen] = React.useState(false);
  const [updateProgress, setUpdateProgress] = React.useState<number>(0);
  const [updateStatus, setUpdateStatus] = React.useState<string>('');
  const [updateSpeed, setUpdateSpeed] = React.useState<number>(0);
  const [updateDownloaded, setUpdateDownloaded] = React.useState<number>(0);
  const [updateTotal, setUpdateTotal] = React.useState<number>(0);

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
        message.success(i18n.t('settings.about.downloadingComplete'));
      }
    });

    return () => {
      unlisten.then((fn) => fn()).catch(console.error);
    };
  }, [message]);

  const handleInstallUpdate = async (info: UpdateInfo) => {
    notification.destroy();

    if (info.signature && info.url) {
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
        Modal.success({
          title: i18n.t('settings.about.updateComplete'),
          content: i18n.t('settings.about.updateCompleteRestart'),
          okText: i18n.t('common.restart'),
          onOk: () => {
            restartApp();
          },
        });
      } catch (error) {
        console.error('Failed to install update:', error);
        setUpdateModalOpen(false);

        const githubActionsUrl = `https://github.com/${GITHUB_REPO}/actions`;
        Modal.error({
          title: i18n.t('settings.about.updateFailed'),
          content: (
            <div>
              <p>{i18n.t('settings.about.updateFailedMessage')}</p>
              <p style={{ marginTop: 8 }}>
                <Typography.Link onClick={() => openExternalUrl(githubActionsUrl)}>
                  {i18n.t('settings.about.goToGitHubActions')}
                </Typography.Link>
              </p>
            </div>
          ),
          okText: i18n.t('common.close'),
        });
      }
    } else if (info.releaseUrl) {
      try {
        await openExternalUrl(info.releaseUrl);
      } catch (error) {
        console.error('Failed to open release page:', error);
      }
    }
  };

  // Check for updates on app startup (at most once per hour)
  React.useEffect(() => {
    if (hasCheckedUpdate.current) return;
    hasCheckedUpdate.current = true;

    const LAST_CHECK_KEY = 'lastUpdateCheckTime';
    const now = Date.now();
    const lastCheck = Number(localStorage.getItem(LAST_CHECK_KEY) || '0');
    // Skip rate limit in dev mode
    if (!import.meta.env.DEV && now - lastCheck < 3600000) return;

    const checkUpdate = async () => {
      try {
        const info = await checkForUpdates();
        localStorage.setItem(LAST_CHECK_KEY, String(now));
        if (info.hasUpdate) {
          notification.info({
            message: i18n.t('settings.about.newVersion'),
            description: i18n.t('settings.about.updateAvailable', { version: info.latestVersion }),
            btn: (
              <Space>
                <Button
                  size="small"
                  onClick={() => {
                    openExternalUrl(info.releaseUrl);
                    notification.destroy();
                  }}
                >
                  {i18n.t('settings.about.viewReleaseNotes')}
                </Button>
                <Button
                  type="primary"
                  size="small"
                  onClick={() => handleInstallUpdate(info)}
                >
                  {i18n.t('settings.about.goToDownload')}
                </Button>
              </Space>
            ),
            duration: 10,
          });
        }
      } catch (error) {
        console.error('Auto check update failed:', error);
      }
    };

    checkUpdate();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [notification]);

  // Listen for config changes from tray menu
  React.useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      try {
        unlisten = await listen<string>('config-changed', async (event) => {
          const configType = event.payload;
          if (configType === 'tray') {
            window.location.reload();
          }
        });
      } catch (error) {
        console.error('Failed to setup config change listener:', error);
      }
    };

    setupListener();

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  return (
    <>
      {children}
      {/* Update Progress Modal */}
      <Modal
        title={i18n.t('settings.about.downloadingUpdate')}
        open={updateModalOpen}
        closable={false}
        footer={null}
        centered
      >
        <div style={{ padding: '20px 0' }}>
          <Progress
            percent={updateProgress}
            status="active"
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
                {i18n.t('settings.about.installingUpdate')}
              </Text>
            )}
            {updateStatus === 'started' && (
              <Text type="secondary" style={{ fontSize: 14 }}>
                {i18n.t('settings.about.downloadingUpdate')}
              </Text>
            )}
          </div>
        </div>
      </Modal>
    </>
  );
};

export const Providers: React.FC<ProvidersProps> = ({ children }) => {
  const { language, isInitialized: appInitialized, initApp } = useAppStore();
  const { isInitialized: settingsInitialized, initSettings } = useSettingsStore();
  const { mode, resolvedTheme, isInitialized: themeInitialized, initTheme, updateResolvedTheme } = useThemeStore();

  const isLoading = !appInitialized || !settingsInitialized || !themeInitialized;

  React.useEffect(() => {
    let cancelled = false;

    const sendReady = () => {
      emit('frontend-ready').catch(() => {});
    };

    // Emit twice to avoid missing the backend listener during early startup.
    sendReady();
    const timer = window.setTimeout(() => {
      if (!cancelled) {
        sendReady();
      }
    }, 1000);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, []);

  // Initialize app, settings and theme on mount
  React.useEffect(() => {
    const init = async () => {
      await initApp();
      await initSettings();
      await initTheme();
    };
    init();
  }, [initApp, initSettings, initTheme]);

  // Listen for system theme changes
  React.useEffect(() => {
    if (!themeInitialized) return;

    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');

    const handleChange = (e: MediaQueryListEvent) => {
      if (mode === 'system') {
        updateResolvedTheme(e.matches ? 'dark' : 'light');
      }
    };

    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, [mode, themeInitialized, updateResolvedTheme]);

  // Apply data-theme attribute to document
  React.useEffect(() => {
    if (themeInitialized) {
      document.documentElement.setAttribute('data-theme', resolvedTheme);
    }
  }, [resolvedTheme, themeInitialized]);

  // Set window background color for macOS titlebar
  React.useEffect(() => {
    if (themeInitialized) {
      // Light theme: #ffffff, Dark theme: #1f1f1f
      const bgColor = resolvedTheme === 'dark' ? { r: 31, g: 31, b: 31 } : { r: 255, g: 255, b: 255 };
      setWindowBackgroundColor(bgColor.r, bgColor.g, bgColor.b).catch(console.error);
    }
  }, [resolvedTheme, themeInitialized]);

  // Sync i18n language when app language changes
  React.useEffect(() => {
    if (appInitialized && i18n.language !== language) {
      i18n.changeLanguage(language);
    }
  }, [language, appInitialized]);

  if (isLoading) {
    return (
      <div
        style={{
          display: 'flex',
          justifyContent: 'center',
          alignItems: 'center',
          height: '100vh',
          width: '100vw',
        }}
      >
        <Spin size="large" />
      </div>
    );
  }

  return (
    <ConfigProvider
      locale={antdLocales[language]}
      theme={{
        algorithm: resolvedTheme === 'dark' ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm,
        token: {
          colorPrimary: '#1890ff',
        },
      }}
    >
      <App>
        <AppInitializer>
          {children}
        </AppInitializer>
      </App>
    </ConfigProvider>
  );
};
