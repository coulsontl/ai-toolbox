import React from 'react';
import { Tooltip } from 'antd';
import { Blocks } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { getLastPluginsPath } from '../utils/pluginNavigation';
import styles from './PluginsButton.module.less';

const PluginsButton: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const isActive = location.pathname.startsWith('/plugins');

  const handleClick = () => {
    // Stay on the current plugins sub-route when already inside `/plugins`,
    // and restore the last visited sub-route when returning from another tab.
    if (isActive) {
      return;
    }
    navigate(getLastPluginsPath());
  };

  return (
    <Tooltip title={t('plugins.tooltip')}>
      <div
        className={`${styles.pluginsButton} ${isActive ? styles.active : ''}`}
        onClick={handleClick}
      >
        <Blocks className={styles.icon} size={14} />
        <span className={styles.text}>{t('plugins.navLabel')}</span>
      </div>
    </Tooltip>
  );
};

export default PluginsButton;
