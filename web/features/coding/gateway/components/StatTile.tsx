import React from 'react';
import { joinClassNames } from '../utils/gatewayFormatters';
import styles from './StatTile.module.less';

interface StatTileProps {
  icon: React.ReactNode;
  label: string;
  value: string;
  tone?: 'default' | 'success' | 'error' | 'muted';
  meta?: string;
}

const StatTile: React.FC<StatTileProps> = ({ icon, label, value, tone = 'default', meta }) => (
  <section className={styles.statTile}>
    <div className={styles.statIcon}>{icon}</div>
    <div className={styles.statBody}>
      <div className={styles.statMain}>
        <span className={styles.statLabel}>{label}</span>
        <span className={joinClassNames(styles.statValue, styles[`statValue_${tone}`])}>{value}</span>
      </div>
      {meta ? <span className={styles.statMeta}>{meta}</span> : null}
    </div>
  </section>
);

export default StatTile;
