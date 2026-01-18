import React from 'react';
import { Layout, Tabs } from 'antd';
import { useNavigate, useLocation, Outlet } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { platform } from '@tauri-apps/plugin-os';
import { MODULES, SETTINGS_MODULE } from '@/constants';
import { useAppStore } from '@/stores';
import { WSLStatusIndicator } from '@/features/settings/components/WSLStatusIndicator';
import { WSLSyncModal } from '@/features/settings/components/WSLSyncModal';
import { useWSLSync } from '@/features/settings/hooks/useWSLSync';
import styles from './styles.module.less';

const { Sider, Content } = Layout;

const MainLayout: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { currentModule, setCurrentModule, setCurrentSubTab, currentSubTab } = useAppStore();
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

  // Determine active module based on current URL path
  const activeModule = React.useMemo(() => {
    if (isSettingsPage) return 'settings';
    if (location.pathname.startsWith('/preview')) return currentModule;
    for (const module of MODULES) {
      if (location.pathname.startsWith(module.path)) {
        return module.key;
      }
    }
    return currentModule;
  }, [location.pathname, isSettingsPage, currentModule]);

  const currentModuleConfig = MODULES.find((m) => m.key === activeModule);
  const subTabs = currentModuleConfig?.subTabs || [];

  const currentSubTabKey = React.useMemo(() => {
    if (location.pathname.startsWith('/preview')) return currentSubTab;
    const path = location.pathname;
    for (const tab of subTabs) {
      if (path.startsWith(tab.path)) {
        return tab.key;
      }
    }
    return subTabs[0]?.key || '';
  }, [location.pathname, subTabs, currentSubTab]);

  const handleModuleClick = (moduleKey: string) => {
    if (moduleKey === 'settings') {
      navigate('/settings');
      return;
    }

    const module = MODULES.find((m) => m.key === moduleKey);
    if (module) {
      setCurrentModule(moduleKey);
      const firstSubTab = module.subTabs[0];
      if (firstSubTab) {
        setCurrentSubTab(firstSubTab.key);
        navigate(firstSubTab.path);
      } else {
        navigate(module.path);
      }
    }
  };

  const handleSubTabChange = (key: string) => {
    const tab = subTabs.find((t) => t.key === key);
    if (tab) {
      setCurrentSubTab(key);
      navigate(tab.path);
    }
  };

  return (
    <Layout className={styles.layout}>
      <Sider width={80} className={styles.sidebar}>
        <div className={styles.sidebarTop}>
          {MODULES.map((module) => (
            <div
              key={module.key}
              className={`${styles.moduleItem} ${activeModule === module.key ? styles.active : ''}`}
              onClick={() => handleModuleClick(module.key)}
            >
              <span className={styles.moduleIcon}>{module.icon}</span>
              <span className={styles.moduleLabel}>{t(module.labelKey)}</span>
            </div>
          ))}
        </div>
        <div className={styles.sidebarBottom}>
          <div
            className={`${styles.settingsBtn} ${activeModule === 'settings' ? styles.active : ''}`}
            onClick={() => handleModuleClick('settings')}
          >
            <span className={styles.moduleIcon}>{SETTINGS_MODULE.icon}</span>
            <span className={styles.moduleLabel}>{t(SETTINGS_MODULE.labelKey)}</span>
          </div>
        </div>
      </Sider>
      <Layout className={styles.mainContent}>
        {!isSettingsPage && !location.pathname.startsWith('/preview') && subTabs.length > 0 && (
          <div className={styles.subTabsHeader}>
            <Tabs
              activeKey={currentSubTabKey}
              onChange={handleSubTabChange}
              items={subTabs.map((tab) => ({
                key: tab.key,
                label: t(tab.labelKey),
              }))}
            />
            {isWindows && config && status && (
              <WSLStatusIndicator
                enabled={config.enabled}
                status={status.lastSyncStatus === 'success' ? 'success' : status.lastSyncStatus === 'error' ? 'error' : 'idle'}
                wslAvailable={status.wslAvailable}
                onClick={() => {
                  window.dispatchEvent(new CustomEvent('open-wsl-settings'));
                }}
              />
            )}
          </div>
        )}
        <Content className={styles.contentArea}>
          <Outlet />
        </Content>
      </Layout>

      {/* WSL Sync Modal - only render on Windows */}
      {isWindows && (
        <WSLSyncModal open={wslModalOpen} onClose={() => setWslModalOpen(false)} />
      )}
    </Layout>
  );
};

export default MainLayout;
