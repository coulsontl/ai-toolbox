import React from 'react';
import { Tooltip } from 'antd';
import { Blocks } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { DEFAULT_PLUGINS_PATH } from '../utils/pluginNavigation';
import styles from './PluginsButton.module.less';

const PluginsButton: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const isActive = location.pathname.startsWith('/plugins');

  return (
    <Tooltip title={t('plugins.tooltip')}>
      <div
        className={`${styles.pluginsButton} ${isActive ? styles.active : ''}`}
        onClick={() => navigate(DEFAULT_PLUGINS_PATH)}
      >
        <Blocks className={styles.icon} size={14} />
        <span className={styles.text}>{t('plugins.navLabel')}</span>
      </div>
    </Tooltip>
  );
};

export default PluginsButton;
