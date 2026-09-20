import React from 'react';
import { Tooltip } from 'antd';
import { Globe } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { MINI_BROWSER_PAGE_PATH } from '../utils/miniBrowserNavigation';
import styles from './MiniBrowserButton.module.less';

/**
 * Toolbar entry for the embedded browser.
 *
 * The browser windows themselves are native webviews created by the backend
 * (see `tauri/src/mini_browser.rs`) and stay in their own top-level windows.
 * This entry only navigates the main window to the standalone workbench page
 * (`pages/MiniBrowserPage.tsx`: saved sites, accounts and the open-window
 * strip). It used to open a Modal, which could not hold the management surface
 * as comfortably as a route.
 */
export const MiniBrowserButton: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const isActive = location.pathname.startsWith(MINI_BROWSER_PAGE_PATH);

  return (
    <Tooltip title={t('miniBrowser.tooltip')}>
      <div
        className={`${styles.button} ${isActive ? styles.active : ''}`}
        role="button"
        tabIndex={0}
        onClick={() => navigate(MINI_BROWSER_PAGE_PATH)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            navigate(MINI_BROWSER_PAGE_PATH);
          }
        }}
      >
        <Globe className={styles.icon} size={14} />
        <span className={styles.text}>{t('miniBrowser.button')}</span>
      </div>
    </Tooltip>
  );
};

export default MiniBrowserButton;
