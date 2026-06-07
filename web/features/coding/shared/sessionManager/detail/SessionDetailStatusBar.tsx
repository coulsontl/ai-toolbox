import React from 'react';
import type { TFunction } from 'i18next';

import type { SessionDetail } from '../types';
import { formatTimestamp } from '../utils';
import styles from './SessionDetailWorkbench.module.less';

interface SessionDetailStatusBarProps {
  detail: SessionDetail;
  visibleCount: number;
  totalCount: number;
  t: TFunction;
}

const SessionDetailStatusBar: React.FC<SessionDetailStatusBarProps> = ({
  detail,
  visibleCount,
  totalCount,
  t,
}) => (
  <footer className={styles.statusBar}>
    <span>{t('sessionManager.visibleMessages')}: {visibleCount}</span>
    <span>{t('sessionManager.totalMessages')}: {totalCount}</span>
    {detail.meta.lastActiveAt ? <span>{t('sessionManager.lastActiveAt')}: {formatTimestamp(detail.meta.lastActiveAt)}</span> : null}
    <span className={styles.statusSourcePath}>{detail.meta.sourcePath}</span>
  </footer>
);

export default SessionDetailStatusBar;
