import React from 'react';
import { Tooltip } from 'antd';
import { Plug } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { DEFAULT_RELAY_PATH } from '../utils/relayNavigation';
import styles from './RelayButton.module.less';

const RelayButton: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const isActive = location.pathname.startsWith('/2api');

  return (
    <Tooltip title={t('relay.tooltip')}>
      <div
        className={`${styles.relayButton} ${isActive ? styles.active : ''}`}
        onClick={() => navigate(DEFAULT_RELAY_PATH)}
      >
        <Plug className={styles.icon} size={14} />
        <span className={styles.text}>{t('relay.navLabel')}</span>
      </div>
    </Tooltip>
  );
};

export default RelayButton;
