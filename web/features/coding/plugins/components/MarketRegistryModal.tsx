import React from 'react';
import { App, Input, Modal, Switch, Tooltip } from 'antd';
import { Loader2, Pencil, RefreshCw, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  addPluginMarket,
  refreshPluginMarkets,
  removePluginMarket,
  updatePluginMarket,
  type MarketSourceRecord,
} from '@/services';
import {
  formatPluginError,
  formatRelativeTime,
} from '../utils/pluginFormatters';
import styles from './PluginModals.module.less';

interface MarketRegistryModalProps {
  open: boolean;
  markets: MarketSourceRecord[];
  onClose: () => void;
  onChanged: () => Promise<void>;
}

const MarketRegistryModal: React.FC<MarketRegistryModalProps> = ({
  open,
  markets,
  onClose,
  onChanged,
}) => {
  const { t, i18n } = useTranslation();
  const { message, modal } = App.useApp();
  const [name, setName] = React.useState('');
  const [url, setUrl] = React.useState('');
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [busyId, setBusyId] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setName('');
    setUrl('');
    setEditingId(null);
    setBusyId(null);
    setError(null);
  }, [open]);

  const resetForm = () => {
    setName('');
    setUrl('');
    setEditingId(null);
    setError(null);
  };

  const handleEdit = (market: MarketSourceRecord) => {
    setEditingId(market.id);
    setName(market.name);
    setUrl(market.url);
    setError(null);
  };

  const handleSubmit = async () => {
    if (!url.trim()) {
      setError(t('plugins.marketRegistry.urlRequired'));
      return;
    }
    setBusyId(editingId ?? 'new');
    setError(null);
    try {
      if (editingId) {
        await updatePluginMarket(editingId, {
          name: name.trim() || null,
          url: url.trim(),
        });
        message.success(t('plugins.marketRegistry.saveSuccess'));
      } else {
        await addPluginMarket(name.trim(), url.trim());
        message.success(t('plugins.marketRegistry.addSuccess'));
      }
      resetForm();
      await onChanged();
    } catch (submitError) {
      setError(
        t('plugins.marketRegistry.submitFailed', {
          error: formatPluginError(submitError),
        }),
      );
    } finally {
      setBusyId(null);
    }
  };

  const handleToggle = async (market: MarketSourceRecord, enabled: boolean) => {
    setBusyId(market.id);
    setError(null);
    try {
      await updatePluginMarket(market.id, { enabled });
      await onChanged();
    } catch (toggleError) {
      setError(
        t('plugins.marketRegistry.submitFailed', {
          error: formatPluginError(toggleError),
        }),
      );
    } finally {
      setBusyId(null);
    }
  };

  const handleRefreshAll = async () => {
    setBusyId('refresh');
    setError(null);
    try {
      await refreshPluginMarkets();
      message.success(t('plugins.marketRegistry.refreshSuccess'));
      await onChanged();
    } catch (refreshError) {
      setError(
        t('plugins.marketRegistry.submitFailed', {
          error: formatPluginError(refreshError),
        }),
      );
    } finally {
      setBusyId(null);
    }
  };

  const handleRemove = (market: MarketSourceRecord) => {
    modal.confirm({
      title: t('plugins.marketRegistry.removeTitle'),
      content: t('plugins.marketRegistry.removeContent', { name: market.name }),
      okText: t('common.delete'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await removePluginMarket(market.id);
          message.success(t('plugins.marketRegistry.removeSuccess'));
          await onChanged();
        } catch (removeError) {
          message.error(
            t('plugins.marketRegistry.submitFailed', {
              error: formatPluginError(removeError),
            }),
          );
        }
      },
    });
  };

  return (
    <Modal
      open={open}
      title={t('plugins.marketRegistry.title')}
      width={760}
      footer={null}
      mask={{ closable: busyId === null }}
      onCancel={onClose}
    >
      <div className={styles.content}>
        <div className={styles.sectionCard}>
          <h3 className={styles.sectionTitle}>
            {editingId
              ? t('plugins.marketRegistry.editTitle')
              : t('plugins.marketRegistry.addSectionTitle')}
          </h3>
          <p className={styles.sectionHint}>
            {t('plugins.marketRegistry.addSectionHint')}
          </p>

          <div className={styles.formFieldRow}>
            <span className={styles.formLabel}>
              {t('plugins.marketRegistry.name')}
            </span>
            <div className={styles.formControl}>
              <Input
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder={t('plugins.marketRegistry.namePlaceholder')}
                disabled={busyId !== null}
                className={styles.formControlInput}
              />
            </div>
          </div>
          <div className={styles.formFieldRow}>
            <span className={styles.formLabel}>
              {t('plugins.marketRegistry.url')}
            </span>
            <div className={styles.formControl}>
              <Input
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder={t('plugins.marketRegistry.urlPlaceholder')}
                disabled={busyId !== null}
                onPressEnter={() => void handleSubmit()}
                className={styles.formControlInput}
              />
            </div>
          </div>

          {error ? <div className={styles.formError}>{error}</div> : null}

          <div className={styles.footerActions}>
            {editingId ? (
              <button
                type="button"
                className={styles.actionButton}
                disabled={busyId !== null}
                onClick={resetForm}
              >
                {t('plugins.marketRegistry.cancelEdit')}
              </button>
            ) : null}
            <button
              type="button"
              className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
              disabled={busyId !== null}
              onClick={() => void handleSubmit()}
            >
              {busyId === (editingId ?? 'new') ? (
                <Loader2 size={14} className={styles.spin} aria-hidden="true" />
              ) : null}
              <span>
                {editingId
                  ? t('plugins.marketRegistry.save')
                  : t('plugins.marketRegistry.register')}
              </span>
            </button>
          </div>
        </div>

        <div className={styles.sectionCard}>
          <h3 className={styles.sectionTitle}>
            {t('plugins.marketRegistry.listSectionTitle')}
          </h3>
          <p className={styles.sectionHint}>
            {t('plugins.marketRegistry.manageHint')}
          </p>

          {markets.length === 0 ? (
            <div className={styles.emptyText}>{t('plugins.marketRegistry.empty')}</div>
          ) : (
            markets.map((market) => {
              return (
                <div key={market.id} className={styles.marketRow}>
                  <Switch
                    size="small"
                    checked={market.enabled}
                    loading={busyId === market.id}
                    disabled={busyId !== null}
                    aria-label={t('plugins.marketRegistry.enabledToggle')}
                    onChange={(checked) => void handleToggle(market, checked)}
                  />
                  <div className={styles.marketInfo}>
                    <div className={styles.marketNameRow}>
                      <span className={styles.marketName}>{market.name}</span>
                      {market.builtin ? (
                        <span className={styles.builtinTag}>
                          {t('plugins.marketRegistry.builtinTag')}
                        </span>
                      ) : null}
                    </div>
                    <span className={styles.marketUrl}>{market.url}</span>
                    <span className={styles.marketMeta}>
                      <span>
                        {t('plugins.marketRegistry.pluginCount', {
                          count: market.pluginCount,
                        })}
                      </span>
                      <span>
                        {market.lastSyncedAt
                          ? t('plugins.marketRegistry.lastSynced', {
                            time: formatRelativeTime(market.lastSyncedAt, i18n.language),
                          })
                          : t('plugins.marketRegistry.neverSynced')}
                      </span>
                    </span>
                    {market.lastError ? (
                      <span className={styles.marketError}>
                        {t('plugins.marketRegistry.errorPrefix', {
                          error: market.lastError,
                        })}
                      </span>
                    ) : null}
                  </div>
                  <div className={styles.marketActions}>
                    <Tooltip title={t('plugins.marketRegistry.edit')}>
                      <button
                        type="button"
                        className={styles.iconButton}
                        aria-label={t('plugins.marketRegistry.edit')}
                        disabled={busyId !== null}
                        onClick={() => handleEdit(market)}
                      >
                        <Pencil size={14} aria-hidden="true" />
                      </button>
                    </Tooltip>
                    <Tooltip
                      title={
                        market.builtin
                          ? t('plugins.marketRegistry.builtinDeleteHint')
                          : t('plugins.marketRegistry.delete')
                      }
                    >
                      <span className={styles.tooltipHost}>
                        <button
                          type="button"
                          className={styles.iconButton}
                          aria-label={t('plugins.marketRegistry.delete')}
                          disabled={market.builtin || busyId !== null}
                          onClick={() => handleRemove(market)}
                        >
                          <Trash2 size={14} aria-hidden="true" />
                        </button>
                      </span>
                    </Tooltip>
                  </div>
                </div>
              );
            })
          )}

          <div className={styles.footerActions}>
            <button
              type="button"
              className={styles.actionButton}
              disabled={busyId !== null}
              onClick={() => void handleRefreshAll()}
            >
              <RefreshCw
                size={14}
                className={busyId === 'refresh' ? styles.spin : undefined}
                aria-hidden="true"
              />
              <span>{t('plugins.marketRegistry.refreshAll')}</span>
            </button>
            <button type="button" className={styles.actionButton} onClick={onClose}>
              {t('common.close')}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
};

export default MarketRegistryModal;
