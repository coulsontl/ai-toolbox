import React from 'react';
import { Tabs } from 'antd';
import { useNavigate, useLocation } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { CodeOutlined, SettingOutlined } from '@ant-design/icons';
import { platform } from '@tauri-apps/plugin-os';
import { MODULES } from '@/constants';
import { useAppStore, useSettingsStore } from '@/stores';
import { useThemeStore } from '@/stores/themeStore';
import { WSLStatusIndicator } from '@/features/settings/components/WSLStatusIndicator';
import { WSLSyncModal } from '@/features/settings/components/WSLSyncModal';
import { useWSLSync } from '@/features/settings/hooks/useWSLSync';
import { SSHStatusIndicator } from '@/features/settings/components/SSHStatusIndicator';
import { SSHSyncModal } from '@/features/settings/components/SSHSyncModal';
import { useSSHSync } from '@/features/settings/hooks/useSSHSync';
import { SkillsButton } from '@/features/coding/skills';
import { McpButton } from '@/features/coding/mcp';
import { ImageButton } from '@/features/coding/image';
import { GatewayButton } from '@/features/coding/gateway';
import KeepAliveOutlet from '@/components/layout/KeepAliveOutlet';
import { PAGE_ROUTES } from '@/app/routeConfig';
import { getRouteChrome, matchRouteEntry, shouldShowRouteAppHeader } from '@/app/routeMatching';
import styles from './styles.module.less';

import OpencodeIcon from '@/assets/opencode.svg';
import ClaudeIcon from '@/assets/claude.svg';
import ChatgptIcon from '@/assets/chatgpt.svg';
import PiIcon from '@/assets/pi.svg';
import { Gemini, Grok, OpenClaw as OpenClawIcon } from '@lobehub/icons';

const TAB_ICONS: Record<string, string> = {
  opencode: OpencodeIcon,
  claudecode: ClaudeIcon,
  codex: ChatgptIcon,
  pi: PiIcon,
};

// macOS Overlay 模式需要为交通灯按钮预留空间，Windows/Linux 使用原生标题栏
const DRAG_BAR_HEIGHT = platform() === 'windows' || platform() === 'linux' ? 0 : 28; // px
const HEADER_HEIGHT = 56; // px
const CONTENT_TOP_OFFSET = DRAG_BAR_HEIGHT + HEADER_HEIGHT;

const MainLayout: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { setCurrentModule, setCurrentSubTab } = useAppStore();
  const { visibleTabs } = useSettingsStore();
  const { resolvedTheme } = useThemeStore();
  const { config, status, loadError } = useWSLSync();
  const { config: sshConfig, status: sshStatus } = useSSHSync();
  const mainRef = React.useRef<HTMLElement | null>(null);

  // Check if current platform is Windows (only show WSL on Windows)
  const isWindows = React.useMemo(() => platform() === 'windows', []);

  // WSL modal state
  const [wslModalOpen, setWslModalOpen] = React.useState(false);
  // SSH modal state
  const [sshModalOpen, setSSHModalOpen] = React.useState(false);

  // Listen for WSL settings open event
  React.useEffect(() => {
    const handleOpenWSLSettings = () => setWslModalOpen(true);
    const handleOpenSSHSettings = () => setSSHModalOpen(true);
    window.addEventListener('open-wsl-settings', handleOpenWSLSettings);
    window.addEventListener('open-ssh-settings', handleOpenSSHSettings);
    return () => {
      window.removeEventListener('open-wsl-settings', handleOpenWSLSettings);
      window.removeEventListener('open-ssh-settings', handleOpenSSHSettings);
    };
  }, []);

  const isSettingsPage = location.pathname.startsWith('/settings');
  const isSkillsPage = location.pathname.startsWith('/skills');
  const isMcpPage = location.pathname.startsWith('/mcp');
  const isGatewayPage = location.pathname.startsWith('/gateway');
  const isImagePage = location.pathname.startsWith('/images');
  const currentRoute = React.useMemo(
    () => matchRouteEntry(PAGE_ROUTES, location.pathname),
    [location.pathname],
  );
  const routeChrome = React.useMemo(() => getRouteChrome(currentRoute), [currentRoute]);
  const showAppHeader = shouldShowRouteAppHeader(routeChrome);
  const contentTopOffset = showAppHeader ? CONTENT_TOP_OFFSET : DRAG_BAR_HEIGHT;
  const isGatewayVisible = visibleTabs.includes('gateway');
  const isImageVisible = visibleTabs.includes('image');
  const isStandalonePage = isSettingsPage || isSkillsPage || isMcpPage || isGatewayPage || isImagePage;
  const isNonTabPage = isStandalonePage || routeChrome.mode !== 'default';
  const showCodingTabs = showAppHeader && routeChrome.mode === 'default';

  // Get coding module's subTabs, filtered and ordered by visibility settings
  const codingModule = MODULES.find((m) => m.key === 'coding');
  const subTabs = React.useMemo(
    () => visibleTabs
      .map((key) => (codingModule?.subTabs || []).find((tab) => tab.key === key))
      .filter((tab): tab is NonNullable<typeof tab> => tab != null),
    [codingModule?.subTabs, visibleTabs]
  );

  // Current active tab key
  const currentTabKey = React.useMemo(() => {
    for (const tab of subTabs) {
      if (location.pathname.startsWith(tab.path)) {
        return tab.key;
      }
    }
    return subTabs[0]?.key || '';
  }, [location.pathname, subTabs]);

  // Redirect to first visible tab when current path is a hidden coding tab
  React.useEffect(() => {
    if (subTabs.length === 0) {
      return;
    }

    if (routeChrome.mode === 'secondary') {
      if (routeChrome.ownerTabKey && !visibleTabs.includes(routeChrome.ownerTabKey)) {
        navigate(routeChrome.parentPath || subTabs[0].path, { replace: true });
      }
      return;
    }

    if (isStandalonePage) return;
    const isOnVisibleTab = subTabs.some((tab) => location.pathname.startsWith(tab.path));
    if (!isOnVisibleTab) {
      navigate(subTabs[0].path, { replace: true });
    }
  }, [isStandalonePage, location.pathname, navigate, routeChrome, subTabs, visibleTabs]);

  React.useEffect(() => {
    if (!isImagePage || isImageVisible) {
      return;
    }

    navigate(subTabs[0]?.path ?? '/settings', { replace: true });
  }, [isImagePage, isImageVisible, navigate, subTabs]);

  React.useEffect(() => {
    if (!isGatewayPage || isGatewayVisible) {
      return;
    }

    navigate(subTabs[0]?.path ?? '/settings', { replace: true });
  }, [isGatewayPage, isGatewayVisible, navigate, subTabs]);

  const handleTabChange = (key: string) => {
    const tab = subTabs.find((t) => t.key === key);
    if (tab) {
      setCurrentModule('coding');
      setCurrentSubTab(key);
      navigate(tab.path);
    }
  };

  const handleTabClick = (key: string) => {
    const tab = subTabs.find((t) => t.key === key);
    if (tab) {
      setCurrentModule('coding');
      setCurrentSubTab(key);
      navigate(tab.path);
    }
  };

  return (
    <div
      className={styles.layout}
      style={{ ['--content-top-offset' as any]: `${contentTopOffset}px` }}
    >
      {/* 全局拖拽区域（顶部 28px on macOS），避免上边框无法拖动 */}
      {DRAG_BAR_HEIGHT > 0 && (
        <div
          className={styles.dragBar}
          data-tauri-drag-region
          style={{ height: DRAG_BAR_HEIGHT }}
        >
          <img
            src="/tray-icon.png"
            alt="AI Toolbox"
            className={styles.dragBarIcon}
            data-tauri-drag-region
          />
        </div>
      )}

      {showAppHeader ? (
        <header
          className={styles.header}
          data-tauri-drag-region
          style={{ top: 0, height: CONTENT_TOP_OFFSET, paddingTop: DRAG_BAR_HEIGHT }}
        >
          <div className={styles.headerContent} data-tauri-drag-region>
            {/* Left - Logo area */}
            <div className={styles.logoArea} style={{ WebkitAppRegion: 'no-drag' } as any}>
              <CodeOutlined className={styles.logoIcon} />
              <div className={styles.divider} />
            </div>

            {showCodingTabs ? (
              <div className={styles.tabsArea} style={{ WebkitAppRegion: 'no-drag' } as any}>
                <div className={`${styles.tabsWrapper} ${isNonTabPage ? styles.noActiveTab : ''}`}>
                  <Tabs
                    activeKey={currentTabKey}
                    onChange={handleTabChange}
                    onTabClick={handleTabClick}
                    indicator={{
                      size: (origin) => origin - 14,
                      align: 'center',
                    }}
                    items={subTabs.map((tab) => ({
                      key: tab.key,
                      label: (
                        <span className={styles.tabLabel}>
                          {tab.key === 'openclaw' ? (
                            resolvedTheme === 'dark' ? (
                              <OpenClawIcon size={16} className={styles.tabIconColor} />
                            ) : (
                              <OpenClawIcon.Color size={16} className={styles.tabIconColor} />
                            )
                          ) : tab.key === 'grok' ? (
                            <Grok size={16} className={styles.tabIconColor} />
                          ) : tab.key === 'geminicli' ? (
                            <Gemini.Color size={16} className={styles.tabIconColor} />
                          ) : TAB_ICONS[tab.key] ? (
                            <img src={TAB_ICONS[tab.key]} className={styles.tabIcon} alt="" />
                          ) : null}
                          <span>{t(tab.labelKey)}</span>
                        </span>
                      ),
                    }))}
                  />
                </div>
              </div>
            ) : null}

            {/* Right - Actions */}
            <div className={styles.actionsArea} style={{ WebkitAppRegion: 'no-drag' } as any}>
              {/* SSH status indicator (all platforms) */}
              {visibleTabs.includes('ssh') && sshConfig && sshStatus && (
                <>
                  <SSHStatusIndicator
                    enabled={sshConfig.enabled}
                    status={
                      sshStatus.lastSyncStatus === 'success'
                        ? 'success'
                        : sshStatus.lastSyncStatus === 'error'
                          ? 'error'
                          : 'idle'
                    }
                    onClick={() => window.dispatchEvent(new CustomEvent('open-ssh-settings'))}
                  />
                  <div className={styles.actionsDivider} />
                </>
              )}

              {/* WSL status indicator (Windows only) */}
              {visibleTabs.includes('wsl') && isWindows && (
                <>
                  <WSLStatusIndicator
                    enabled={config?.enabled ?? false}
                    status={
                      loadError || status?.lastSyncStatus === 'error'
                        ? 'error'
                        : status?.lastSyncStatus === 'success'
                        ? 'success'
                        : 'idle'
                    }
                    wslAvailable={status?.wslAvailable ?? false}
                    onClick={() => window.dispatchEvent(new CustomEvent('open-wsl-settings'))}
                  />
                  <div className={styles.actionsDivider} />
                </>
              )}

              {/* Skills button */}
              <SkillsButton />
              <div className={styles.actionsDivider} />

              {/* MCP button */}
              <McpButton />
              <div className={styles.actionsDivider} />

              {isGatewayVisible && (
                <>
                  <GatewayButton />
                  <div className={styles.actionsDivider} />
                </>
              )}

              {isImageVisible && (
                <>
                  <ImageButton />
                  <div className={styles.actionsDivider} />
                </>
              )}

              {/* Settings button */}
              <div
                className={`${styles.settingsBtn} ${isSettingsPage ? styles.active : ''}`}
                onClick={() => navigate('/settings')}
              >
                <SettingOutlined className={styles.settingsIcon} />
                <span className={styles.settingsText}>{t('modules.settings')}</span>
              </div>
            </div>
          </div>
        </header>
      ) : null}

      {/* Main content */}
      <main ref={mainRef} className={styles.main}>
        <div className={[
          styles.contentArea,
          routeChrome.contentPadding === 'compact' ? styles.contentAreaCompact : '',
          routeChrome.contentPadding === 'none' ? styles.contentAreaNone : '',
        ].filter(Boolean).join(' ')}
        >
          <KeepAliveOutlet routes={PAGE_ROUTES} max={12} scrollContainerRef={mainRef} />
        </div>
      </main>

      {/* WSL Sync Modal - only render on Windows */}
      {isWindows && <WSLSyncModal open={wslModalOpen} onClose={() => setWslModalOpen(false)} />}

      {/* SSH Sync Modal - all platforms */}
      <SSHSyncModal open={sshModalOpen} onClose={() => setSSHModalOpen(false)} />
    </div>
  );
};

export default MainLayout;
