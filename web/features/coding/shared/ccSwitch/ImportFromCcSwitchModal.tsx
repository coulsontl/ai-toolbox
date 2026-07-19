import React from 'react';
import { Modal, Checkbox, Button, Empty, Spin, Typography, Tag, Input, message } from 'antd';
import { ApiOutlined, CloudServerOutlined, AppstoreOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  listCcSwitchProviders,
  type CcSwitchProviderCandidate,
} from '@/services/ccSwitchApi';
import styles from './ImportFromCcSwitchModal.module.less';

const { Text } = Typography;

export interface ImportFromCcSwitchModalProps {
  open: boolean;
  appType: string;
  existingProviderIds: string[];
  onClose: () => void;
  onImport: (providers: CcSwitchProviderCandidate[]) => void | Promise<void>;
}

const ImportFromCcSwitchModal: React.FC<ImportFromCcSwitchModalProps> = ({
  open,
  appType,
  existingProviderIds,
  onClose,
  onImport,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [importing, setImporting] = React.useState(false);
  const [providers, setProviders] = React.useState<CcSwitchProviderCandidate[]>([]);
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(new Set());
  const [searchText, setSearchText] = React.useState('');
  const [emptyMessage, setEmptyMessage] = React.useState<string | undefined>();

  const loadProviders = React.useCallback(async () => {
    setLoading(true);
    setEmptyMessage(undefined);
    try {
      const discovery = await listCcSwitchProviders(appType);
      setProviders(discovery.providers || []);
      setSelectedIds(new Set());
      if (!discovery.found) {
        setEmptyMessage(t('common.ccSwitch.dbNotFound'));
      } else if (discovery.message === 'cc_switch_db_open_failed') {
        setEmptyMessage(t('common.ccSwitch.dbOpenFailed'));
      } else if ((discovery.providers || []).length === 0) {
        setEmptyMessage(t('common.ccSwitch.noProviders'));
      }
    } catch (error) {
      console.error('Failed to list CC Switch providers:', error);
      message.error(t('common.error'));
      setProviders([]);
      setEmptyMessage(t('common.ccSwitch.dbOpenFailed'));
    } finally {
      setLoading(false);
    }
  }, [appType, t]);

  React.useEffect(() => {
    if (open) {
      setSearchText('');
      void loadProviders();
    }
  }, [open, loadProviders]);

  const existingSet = React.useMemo(
    () => new Set(existingProviderIds.filter(Boolean)),
    [existingProviderIds],
  );

  const filteredProviders = React.useMemo(() => {
    const keyword = searchText.trim().toLowerCase();
    if (!keyword) {
      return providers;
    }
    return providers.filter((provider) => {
      const haystack = [
        provider.name,
        provider.baseUrlPreview,
        provider.modelPreview,
        provider.rawId,
        provider.providerId,
      ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      return haystack.includes(keyword);
    });
  }, [providers, searchText]);

  const sortedProviders = React.useMemo(() => {
    return [...filteredProviders].sort((a, b) => {
      const aExisting = existingSet.has(a.providerId);
      const bExisting = existingSet.has(b.providerId);
      if (aExisting === bExisting) {
        return 0;
      }
      return aExisting ? 1 : -1;
    });
  }, [filteredProviders, existingSet]);

  const importableProviders = sortedProviders.filter(
    (provider) => !existingSet.has(provider.providerId),
  );

  const isAllSelected =
    importableProviders.length > 0 &&
    importableProviders.every((provider) => selectedIds.has(provider.providerId));

  const handleToggle = (providerId: string, selected: boolean) => {
    setSelectedIds((previous) => {
      const next = new Set(previous);
      if (selected) {
        next.add(providerId);
      } else {
        next.delete(providerId);
      }
      return next;
    });
  };

  const handleSelectAll = () => {
    setSelectedIds(new Set(importableProviders.map((provider) => provider.providerId)));
  };

  const handleDeselectAll = () => {
    setSelectedIds(new Set());
  };

  const importableSelectedCount = Array.from(selectedIds).filter(
    (id) => !existingSet.has(id),
  ).length;

  const handleImport = async () => {
    const selectedProviders = providers.filter(
      (provider) =>
        selectedIds.has(provider.providerId) && !existingSet.has(provider.providerId),
    );
    if (selectedProviders.length === 0) {
      return;
    }
    setImporting(true);
    try {
      await onImport(selectedProviders);
    } finally {
      setImporting(false);
    }
  };

  return (
    <Modal
      title={t('common.ccSwitch.modalTitle')}
      open={open}
      onCancel={onClose}
      width={800}
      className={styles.modal}
      footer={[
        <Button key="cancel" onClick={onClose}>
          {t('common.cancel')}
        </Button>,
        <Button
          key="import"
          type="primary"
          onClick={() => void handleImport()}
          disabled={importableSelectedCount === 0}
          loading={importing}
        >
          {t('common.ccSwitch.importSelected')} ({importableSelectedCount})
        </Button>,
      ]}
    >
      <Spin spinning={loading}>
        {providers.length === 0 && !loading ? (
          <Empty description={emptyMessage || t('common.ccSwitch.noProviders')} />
        ) : (
          <div>
            <div className={styles.toolbar}>
              <Checkbox
                checked={isAllSelected}
                indeterminate={selectedIds.size > 0 && !isAllSelected}
                onChange={(event) =>
                  event.target.checked ? handleSelectAll() : handleDeselectAll()
                }
              >
                {isAllSelected
                  ? t('common.ccSwitch.deselectAll')
                  : t('common.ccSwitch.selectAll')}
              </Checkbox>
              <div className={styles.toolbarRight}>
                <Text className={styles.summary}>
                  {filteredProviders.length} / {providers.length}
                </Text>
                <Input
                  allowClear
                  size="small"
                  className={styles.searchInput}
                  placeholder={t('common.ccSwitch.searchPlaceholder')}
                  value={searchText}
                  onChange={(event) => setSearchText(event.target.value)}
                />
              </div>
            </div>
            <div className={styles.container}>
              {sortedProviders.map((provider) => {
                const isExisting = existingSet.has(provider.providerId);
                const isSelected = selectedIds.has(provider.providerId);
                const isDisabled = isExisting;

                return (
                  <div
                    key={provider.providerId}
                    className={`${styles.card} ${isExisting ? styles.existing : ''} ${
                      isSelected && !isDisabled ? styles.selected : ''
                    }`}
                    onClick={() => {
                      if (!isDisabled) {
                        handleToggle(provider.providerId, !isSelected);
                      }
                    }}
                  >
                    <div className={styles.cardHeader}>
                      <Checkbox
                        checked={isSelected}
                        disabled={isDisabled}
                        onClick={(event) => event.stopPropagation()}
                        onChange={(event) =>
                          handleToggle(provider.providerId, event.target.checked)
                        }
                      />
                      <div className={styles.titleArea}>
                        <Text strong className={styles.title}>
                          {provider.name}
                        </Text>
                        {isExisting && (
                          <Tag className={styles.tag}>{t('common.ccSwitch.existingTag')}</Tag>
                        )}
                        {provider.isLocalEndpoint && (
                          <Tag
                            className={styles.tag}
                            title={t('common.ccSwitch.localEndpointHint')}
                          >
                            {t('common.ccSwitch.localEndpointTag')}
                          </Tag>
                        )}
                        {!provider.hasApiKey && (
                          <Tag className={styles.tag}>{t('common.ccSwitch.noApiKeyTag')}</Tag>
                        )}
                      </div>
                    </div>
                    <div className={styles.cardBody}>
                      <div className={styles.infoRow}>
                        {provider.baseUrlPreview && (
                          <div className={styles.infoItem}>
                            <CloudServerOutlined className={styles.icon} />
                            <Text
                              className={styles.infoText}
                              ellipsis
                              title={provider.baseUrlPreview}
                            >
                              {provider.baseUrlPreview}
                            </Text>
                          </div>
                        )}
                        {provider.modelPreview && (
                          <div className={styles.infoItem}>
                            <AppstoreOutlined className={styles.icon} />
                            <Text className={styles.infoText} ellipsis title={provider.modelPreview}>
                              {provider.modelPreview}
                            </Text>
                          </div>
                        )}
                        {!provider.baseUrlPreview && !provider.modelPreview && (
                          <div className={styles.infoItem}>
                            <ApiOutlined className={styles.icon} />
                            <Text className={styles.infoText} type="secondary">
                              {provider.normalizedCategory}
                            </Text>
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </Spin>
    </Modal>
  );
};

export default ImportFromCcSwitchModal;
