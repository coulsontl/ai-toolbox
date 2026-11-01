import React from 'react';
import { Empty, Spin } from 'antd';
import { AlertCircle, Plug } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import {
  listRelayChannels,
  listRelayCheckinRecords,
  type RelayChannel,
  type RelayCheckinRecord,
} from '@/services';
import RelayPluginsPanel from '../components/RelayPluginsPanel';
import {
  RELAY_TABS,
  getRelayPathForTab,
  resolveRelayTabFromPath,
} from '../utils/relayNavigation';
import styles from './RelayPage.module.less';

const RelayPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { isActive } = useKeepAlive();
  const activeTab = resolveRelayTabFromPath(location.pathname);

  const [channels, setChannels] = React.useState<RelayChannel[]>([]);
  const [records, setRecords] = React.useState<RelayCheckinRecord[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setError(null);
      const [loadedChannels, loadedRecords] = await Promise.all([
        listRelayChannels(),
        listRelayCheckinRecords(50),
      ]);
      setChannels(loadedChannels);
      setRecords(loadedRecords);
    } catch (loadError) {
      setError(String(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    // The page stays mounted after the first visit; reloading on every
    // tab switch would refetch from a page nobody is looking at.
    if (!isActive) {
      return;
    }
    void refresh();
  }, [isActive, refresh]);

  const renderChannels = () => {
    if (channels.length === 0) {
      return (
        <Empty
          description={t('relay.page.channels.empty')}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      );
    }
    return (
      <div className={styles.cardGrid}>
        {channels.map((channel) => (
          <div key={channel.id} className={styles.card}>
            <div className={styles.cardTitle}>{channel.name || channel.id}</div>
            <div className={styles.cardMeta}>
              {channel.kind === 'plugin'
                ? t('relay.page.channels.kindPlugin')
                : t('relay.page.channels.kindHttp')}
              {channel.target_protocol ? ` · ${channel.target_protocol}` : ''}
            </div>
            <div className={styles.cardMeta}>
              {channel.enabled
                ? t('relay.page.enabled')
                : t('relay.page.disabled')}
            </div>
          </div>
        ))}
      </div>
    );
  };

  const renderCheckin = () => {
    if (records.length === 0) {
      return (
        <Empty
          description={t('relay.page.checkin.empty')}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      );
    }
    return (
      <table className={styles.table}>
        <thead>
          <tr>
            <th>{t('relay.page.checkin.columns.time')}</th>
            <th>{t('relay.page.checkin.columns.provider')}</th>
            <th>{t('relay.page.checkin.columns.account')}</th>
            <th>{t('relay.page.checkin.columns.outcome')}</th>
            <th>{t('relay.page.checkin.columns.message')}</th>
          </tr>
        </thead>
        <tbody>
          {records.map((record) => (
            <tr key={record.id}>
              <td>{new Date(record.ts).toLocaleString()}</td>
              <td>{record.provider || '—'}</td>
              <td>{record.account_name || record.account_id || '—'}</td>
              <td>{record.outcome || '—'}</td>
              <td className={styles.ellipsis} title={record.message}>
                {record.message || '—'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    );
  };

  const renderTabContent = () => {
    if (loading) {
      return (
        <div className={styles.center}>
          <Spin />
        </div>
      );
    }
    if (error) {
      return (
        <div className={styles.notice}>
          <AlertCircle size={14} />
          <span>{error}</span>
        </div>
      );
    }
    switch (activeTab) {
      case 'plugins':
        // The plugin panel owns its own data: it drives the Bun download, the
        // host process and sign-ins, none of which the page shell knows about.
        return <RelayPluginsPanel />;
      case 'checkin':
        return renderCheckin();
      case 'settings':
        return (
          <div className={styles.helper}>{t('relay.page.settings.placeholder')}</div>
        );
      default:
        return renderChannels();
    }
  };

  return (
    <div className={styles.relayPage}>
      <div className={styles.header}>
        <div className={styles.titleBlock}>
          <div className={styles.titleIcon}>
            <Plug size={20} />
          </div>
          <div>
            <div className={styles.title}>{t('relay.page.title')}</div>
            <div className={styles.subtitle}>{t('relay.page.subtitle')}</div>
          </div>
        </div>
        <div className={styles.tabList} role="tablist" aria-label={t('relay.page.title')}>
          {RELAY_TABS.map((tab) => (
            <button
              key={tab.key}
              type="button"
              role="tab"
              aria-selected={activeTab === tab.key}
              className={`${styles.tabButton} ${
                activeTab === tab.key ? styles.tabButtonActive : ''
              }`}
              onClick={() => navigate(getRelayPathForTab(tab.key))}
            >
              {t(tab.labelKey)}
            </button>
          ))}
        </div>
      </div>
      <div className={styles.content}>{renderTabContent()}</div>
    </div>
  );
};

export default RelayPage;
