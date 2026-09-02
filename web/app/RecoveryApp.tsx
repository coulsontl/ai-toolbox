import React from 'react';
import { ConfigProvider, App, Spin, Modal, Typography, Button, Space } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import enUS from 'antd/locale/en_US';
import { theme as antdTheme } from 'antd';
import { listen } from '@tauri-apps/api/event';
import i18n from '@/i18n';
import {
  checkForUpdates,
  installUpdate,
  openExternalUrl,
  exitApp,
  LATEST_RELEASE_URL,
} from '@/services';
import { restartApp } from '@/services/settingsApi';
import UpdateProgressModal from '@/components/common/UpdateProgressModal';

const { Title, Paragraph } = Typography;

const antdLocales = {
  'zh-CN': zhCN,
  'en-US': enUS,
};

/** Detect a best-effort UI language without the database (recovery mode). */
function detectLanguage(): 'zh-CN' | 'en-US' {
  if (typeof navigator !== 'undefined' && navigator.language?.toLowerCase().startsWith('zh')) {
    return 'zh-CN';
  }
  return 'en-US';
}

/** Detect system theme without the database (recovery mode). */
function detectSystemTheme(): 'light' | 'dark' {
  if (typeof window !== 'undefined' && window.matchMedia) {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return 'light';
}

/**
 * Minimal recovery shell rendered when the SQLite schema is too new to open.
 *
 * Mirrors the normal `Providers` ConfigProvider setup (locale + theme
 * algorithm + `data-theme`) but skips every database-dependent init effect.
 * The only thing it does is run the auto-updater and offer a manual-download
 * fallback. No DB access happens anywhere in this tree.
 */
const RecoveryApp: React.FC<{ errorMessage: string }> = ({ errorMessage }) => {
  const language = React.useMemo(detectLanguage, []);
  const [resolvedTheme, setResolvedTheme] = React.useState(detectSystemTheme);

  React.useEffect(() => {
    document.documentElement.setAttribute('data-theme', resolvedTheme);
  }, [resolvedTheme]);

  React.useEffect(() => {
    if (!window.matchMedia) return;
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleChange = (e: MediaQueryListEvent) => setResolvedTheme(e.matches ? 'dark' : 'light');
    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, []);

  const antdLocale = antdLocales[language];
  const antdThemeConfig = React.useMemo(
    () => ({
      algorithm: resolvedTheme === 'dark' ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm,
      token: { colorPrimary: '#1890ff' },
    }),
    [resolvedTheme],
  );

  return (
    <ConfigProvider locale={antdLocale} theme={antdThemeConfig}>
      <App>
        <RecoveryUpdateScreen errorMessage={errorMessage} />
      </App>
    </ConfigProvider>
  );
};

/** Auto-upgrade screen: check → download → install → restart. */
const RecoveryUpdateScreen: React.FC<{ errorMessage: string }> = ({ errorMessage }) => {
  const { message: messageApi } = App.useApp();
  const [modalOpen, setModalOpen] = React.useState(false);
  const [progress, setProgress] = React.useState(0);
  const [status, setStatus] = React.useState('started');
  const [speed, setSpeed] = React.useState(0);
  const [downloaded, setDownloaded] = React.useState(0);
  const [total, setTotal] = React.useState(0);

  // Listen for download progress (emitted by the backend install_update flow).
  React.useEffect(() => {
    const unlisten = listen<{
      status: string;
      progress: number;
      downloaded: number;
      total: number;
      speed: number;
    }>('update-download-progress', (event) => {
      const { status: s, progress: p, downloaded: d, total: t, speed: sp } = event.payload;
      setStatus(s);
      setProgress(p);
      setSpeed(sp);
      setDownloaded(d);
      setTotal(t);
      if (s === 'installing') {
        messageApi.success(i18n.t('settings.about.downloadingComplete'));
      }
    });
    return () => {
      unlisten.then((fn) => fn()).catch(console.error);
    };
  }, [messageApi]);

  // Recovery screen phases:
  // - checking: querying GitHub for the latest release
  // - ready: an update is available, awaiting the user's click to start
  // - noUpdate: already on the latest version, can only download manually
  // - checkFailed: update check errored, can retry or download manually
  // - downloading: install in progress (progress modal shown)
  // - installFailed: install errored, can retry the install or download manually
  const [phase, setPhase] = React.useState<
    'checking' | 'ready' | 'noUpdate' | 'checkFailed' | 'downloading' | 'installFailed'
  >('checking');
  const [latestVersion, setLatestVersion] = React.useState<string>('');

  const runCheck = React.useCallback(async () => {
    setPhase('checking');
    try {
      const info = await checkForUpdates();
      if (info.hasUpdate && info.signature && info.url) {
        setLatestVersion(info.latestVersion);
        setPhase('ready');
      } else {
        setPhase('noUpdate');
      }
    } catch (error) {
      console.error('Recovery update check failed:', error);
      setPhase('checkFailed');
    }
  }, []);

  // Check for updates on mount — do NOT auto-start the install. The user must
  // explicitly click "Upgrade now" so we never download/replace the app
  // without consent.
  React.useEffect(() => {
    runCheck();
  }, [runCheck]);

  const startInstall = React.useCallback(async () => {
    setModalOpen(true);
    setProgress(0);
    setStatus('started');
    setSpeed(0);
    setDownloaded(0);
    setTotal(0);
    setPhase('downloading');
    try {
      await installUpdate();
      setModalOpen(false);
      Modal.success({
        title: i18n.t('settings.about.updateComplete'),
        content: i18n.t('settings.about.updateCompleteRestart'),
        okText: i18n.t('common.restart'),
        onOk: () => restartApp(),
      });
    } catch (error) {
      console.error('Recovery auto-update failed:', error);
      setModalOpen(false);
      setPhase('installFailed');
    }
  }, []);

  return (
    <div
      style={{
        minHeight: '100vh',
        display: 'flex',
        flexDirection: 'column',
        justifyContent: 'center',
        alignItems: 'center',
        padding: 24,
        gap: 16,
      }}
    >
      <Title level={3}>{i18n.t('recovery.title')}</Title>
      <Paragraph
        type="secondary"
        style={{ maxWidth: 560, whiteSpace: 'pre-wrap', textAlign: 'center' }}
      >
        {errorMessage}
      </Paragraph>

      {phase === 'checking' && (
        <Space>
          <Spin />
          <span style={{ color: 'var(--color-text-secondary)' }}>
            {i18n.t('recovery.checkingLatest')}
          </span>
        </Space>
      )}

      {phase === 'ready' && (
        <Paragraph style={{ textAlign: 'center', marginBottom: 0 }}>
          {i18n.t('recovery.newVersionAvailable', { version: latestVersion })}
        </Paragraph>
      )}

      {phase === 'noUpdate' && (
        <Paragraph type="secondary" style={{ textAlign: 'center', marginBottom: 0 }}>
          {i18n.t('recovery.alreadyLatest')}
        </Paragraph>
      )}

      {phase === 'checkFailed' && (
        <Paragraph type="warning" style={{ textAlign: 'center', marginBottom: 0 }}>
          {i18n.t('recovery.checkFailed')}
        </Paragraph>
      )}

      {phase === 'installFailed' && (
        <Paragraph type="danger" style={{ textAlign: 'center', marginBottom: 0 }}>
          {i18n.t('recovery.updateFailedMessage')}
        </Paragraph>
      )}

      <Space>
        {(phase === 'ready' || phase === 'installFailed') && (
          <Button type="primary" loading={modalOpen} onClick={startInstall}>
            {i18n.t('recovery.upgradeNow')}
          </Button>
        )}
        {phase === 'checkFailed' && (
          <Button onClick={runCheck}>{i18n.t('recovery.retry')}</Button>
        )}
        <Button onClick={() => openExternalUrl(LATEST_RELEASE_URL)}>
          {i18n.t('recovery.openDownloadPage')}
        </Button>
        <Button danger onClick={() => exitApp()}>
          {i18n.t('recovery.exit')}
        </Button>
      </Space>

      <UpdateProgressModal
        open={modalOpen}
        progress={progress}
        status={status}
        speed={speed}
        downloaded={downloaded}
        total={total}
      />
    </div>
  );
};

export default RecoveryApp;
