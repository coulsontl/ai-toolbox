import React from 'react';
import { ProLayout } from '@ant-design/pro-components';
import { Tabs } from 'antd';
import { useNavigate, useLocation, Outlet } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { CodeOutlined, SettingOutlined } from '@ant-design/icons';
import { platform } from '@tauri-apps/plugin-os';
import { MODULES } from '@/constants';
import { useAppStore } from '@/stores';
import { WSLStatusIndicator } from '@/features/settings/components/WSLStatusIndicator';
import { WSLSyncModal } from '@/features/settings/components/WSLSyncModal';
import { useWSLSync } from '@/features/settings/hooks/useWSLSync';
import styles from './styles.module.less';

import OpencodeIcon from '@/assets/opencode.svg';
import ClaudeIcon from '@/assets/claude.svg';
import ChatgptIcon from '@/assets/chatgpt.svg';

const TAB_ICONS: Record<string, string> = {
  opencode: OpencodeIcon,
  claudecode: ClaudeIcon,
  codex: ChatgptIcon,
};

const MainLayout: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { setCurrentModule, setCurrentSubTab } = useAppStore();
  const { config, status } = useWSLSync();

  // Check if current platform is Windows (only show WSL on Windows)
  const isWindows = React.useMemo(() => platform() === 'windows', []);

  // WSL modal state
  const [wslModalOpen, setWslModalOpen] = React.useState(false);

  // Listen for WSL settings open event
  React.useEffect(() => {
    const handleOpenWSLSettings = () => setWslModalOpen(true);
    window.addEventListener('open-wsl-settings', handleOpenWSLSettings);
    return () => {
      window.removeEventListener('open-wsl-settings', handleOpenWSLSettings);
    };
  }, []);

  const isSettingsPage = location.pathname.startsWith('/settings');

  // Get coding module's subTabs
  const codingModule = MODULES.find((m) => m.key === 'coding');
  const subTabs = codingModule?.subTabs || [];

  // Current active tab key
  const currentTabKey = React.useMemo(() => {
    for (const tab of subTabs) {
      if (location.pathname.startsWith(tab.path)) {
        return tab.key;
      }
    }
    return subTabs[0]?.key || '';
  }, [location.pathname, subTabs]);


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
    <>
      <ProLayout
        layout="top"
        fixedHeader
        menuRender={false}
        contentStyle={{ padding: 0 }}
        // Left logo area
        headerTitleRender={() => (
          <div className={styles.logoArea}>
            <CodeOutlined className={styles.logoIcon} />
            <div className={styles.divider} />
          </div>
        )}
        // Center tabs area
        headerContentRender={() => (
          <div className={`${styles.tabsWrapper} ${isSettingsPage ? styles.noActiveTab : ''}`}>
            <Tabs
              activeKey={currentTabKey}
              onChange={handleTabChange}
              onTabClick={handleTabClick}
              items={subTabs.map((tab) => ({
                key: tab.key,
                label: (
                  <span className={styles.tabLabel}>
                    {TAB_ICONS[tab.key] && (
                      <img src={TAB_ICONS[tab.key]} className={styles.tabIcon} alt="" />
                    )}
                    <span>{t(tab.labelKey)}</span>
                  </span>
                ),
              }))}
            />
          </div>
        )}
        // Right actions area
        actionsRender={() => {
          const actions: React.ReactNode[] = [];

          // WSL status indicator (Windows only)
          if (isWindows && config && status) {
            actions.push(
              <WSLStatusIndicator
                key="wsl"
                enabled={config.enabled}
                status={
                  status.lastSyncStatus === 'success'
                    ? 'success'
                    : status.lastSyncStatus === 'error'
                      ? 'error'
                      : 'idle'
                }
                wslAvailable={status.wslAvailable}
                onClick={() => window.dispatchEvent(new CustomEvent('open-wsl-settings'))}
              />
            );
          }

          // Settings button with icon and text
          actions.push(
            <div
              key="settings"
              className={`${styles.settingsBtn} ${isSettingsPage ? styles.active : ''}`}
              onClick={() => navigate('/settings')}
            >
              <SettingOutlined className={styles.settingsIcon} />
              <span className={styles.settingsText}>{t('modules.settings')}</span>
            </div>
          );

          return actions;
        }}
      >
        <div className={styles.contentArea}>
          <Outlet />
        </div>
      </ProLayout>

      {/* WSL Sync Modal - only render on Windows */}
      {isWindows && <WSLSyncModal open={wslModalOpen} onClose={() => setWslModalOpen(false)} />}
    </>
  );
};

export default MainLayout;
