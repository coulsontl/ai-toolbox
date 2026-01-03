import React from 'react';
import { Layout, Tabs } from 'antd';
import { useNavigate, useLocation, Outlet } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { MODULES, SETTINGS_MODULE } from '@/constants';
import { useAppStore } from '@/stores';
import styles from './styles.module.less';

const { Sider, Content } = Layout;

const MainLayout: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { currentModule, setCurrentModule, setCurrentSubTab } = useAppStore();

  const isSettingsPage = location.pathname.startsWith('/settings');
  const activeModule = isSettingsPage ? 'settings' : currentModule;

  const currentModuleConfig = MODULES.find((m) => m.key === currentModule);
  const subTabs = currentModuleConfig?.subTabs || [];

  const currentSubTabKey = React.useMemo(() => {
    const path = location.pathname;
    for (const tab of subTabs) {
      if (path.startsWith(tab.path)) {
        return tab.key;
      }
    }
    return subTabs[0]?.key || '';
  }, [location.pathname, subTabs]);

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
        {!isSettingsPage && subTabs.length > 0 && (
          <div className={styles.subTabsHeader}>
            <Tabs
              activeKey={currentSubTabKey}
              onChange={handleSubTabChange}
              items={subTabs.map((tab) => ({
                key: tab.key,
                label: t(tab.labelKey),
              }))}
            />
          </div>
        )}
        <Content className={styles.contentArea}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
};

export default MainLayout;
