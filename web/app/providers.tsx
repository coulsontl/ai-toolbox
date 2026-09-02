import React from 'react';
import { ConfigProvider, Spin, App, theme as antdTheme, Button, Modal, Typography, Space } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import enUS from 'antd/locale/en_US';
import { emit, listen } from '@tauri-apps/api/event';
import { TRAY_CONFIG_REFRESH_EVENT } from '@/constants/configEvents';
import UpdateProgressModal from '@/components/common/UpdateProgressModal';
import DeepLinkImportDialog from '@/features/shared/deepLink/DeepLinkImportDialog';
import { useDeepLinkImport } from '@/features/shared/deepLink/useDeepLinkImport';
import type { DeepLinkErrorPayload } from '@/services/deeplinkApi';
import { useAppStore, useSettingsStore } from '@/stores';
import { useThemeStore } from '@/stores/themeStore';
import {
  checkForUpdates,
  openExternalUrl,
  setWindowBackgroundColor,
  installUpdate,
  loadCachedPresetModels,
  fetchRemotePresetModels,
  loadCachedGatewayProviderProfiles,
  fetchRemoteGatewayProviderProfiles,
  fetchRemoteModelPricing,
  GITHUB_REPO,
  type UpdateInfo,
} from '@/services';
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
/**
 * Globally-mounted deep-link import dialog: listens for `deep-link-import` /
 * `deep-link-error` events from the backend and shows a confirmation modal
 * (with a masked API key) before persisting via `import_from_deeplink_unified`.
 * The hook marks the frontend listener ready and drains a cold-start pending
 * request after the listeners are attached.
 */
const DeepLinkImportMount: React.FC = () => {
  const { message } = App.useApp();

  const onError = React.useCallback(
    (error: DeepLinkErrorPayload) => {
      message.error(`${i18n.t('common.deepLink.parseError')}: ${error.error}`);
    },
    [message],
  );

  const { request, dismiss } = useDeepLinkImport(onError);

  return <DeepLinkImportDialog request={request} onDismiss={dismiss} />;
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

  // Keep a global fallback for tray-driven config changes so inactive pages and
  // subpanels that do not maintain their own listeners still resync to disk state.
  React.useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      try {
        unlisten = await listen<string>('config-changed', async (event) => {
          if (event.payload === 'tray') {
            const refreshEvent = new CustomEvent(TRAY_CONFIG_REFRESH_EVENT, {
              cancelable: true,
            });
            window.dispatchEvent(refreshEvent);

            if (!refreshEvent.defaultPrevented) {
              window.location.reload();
            }
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
      {/* Deep-link (`aitoolbox://`) provider import confirmation */}
      <DeepLinkImportMount />
      {/* Update Progress Modal */}
      <UpdateProgressModal
        open={updateModalOpen}
        progress={updateProgress}
        status={updateStatus}
        speed={updateSpeed}
        downloaded={updateDownloaded}
        total={updateTotal}
      />
    </>
  );
};

export const Providers: React.FC<ProvidersProps> = ({ children }) => {
  const { language, isInitialized: appInitialized, initApp } = useAppStore();
  const { isInitialized: settingsInitialized, initSettings } = useSettingsStore();
  const { mode, resolvedTheme, isInitialized: themeInitialized, initTheme, updateResolvedTheme } = useThemeStore();

  const isLoading = !appInitialized || !settingsInitialized || !themeInitialized;
  const antdLocale = antdLocales[language];
  const modalConfig = React.useMemo(() => ({
    centered: true,
  }), []);
  const antdThemeConfig = React.useMemo(() => ({
    algorithm: resolvedTheme === 'dark' ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm,
    token: {
      colorPrimary: '#1890ff',
    },
  }), [resolvedTheme]);

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
      // Load preset models: local cache first (fast), then remote (background)
      await loadCachedPresetModels();
      await loadCachedGatewayProviderProfiles();
      fetchRemotePresetModels();
      fetchRemoteGatewayProviderProfiles();
      fetchRemoteModelPricing().catch(() => {});
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

  React.useEffect(() => {
    ConfigProvider.config({
      holderRender: (modalChildren) => (
        <ConfigProvider
          locale={antdLocale}
          modal={modalConfig}
          theme={antdThemeConfig}
        >
          {modalChildren}
        </ConfigProvider>
      ),
    });

    return () => {
      ConfigProvider.config({ holderRender: undefined });
    };
  }, [antdLocale, antdThemeConfig, modalConfig]);

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
      locale={antdLocale}
      modal={modalConfig}
      theme={antdThemeConfig}
    >
      <App>
        <AppInitializer>
          {children}
        </AppInitializer>
      </App>
    </ConfigProvider>
  );
};
