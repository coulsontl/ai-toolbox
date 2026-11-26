import React from 'react';
import { Modal, Table, Radio, Button, Space, Typography, message, Alert, Input, Tooltip, Checkbox, Tag } from 'antd';
import { CloudDownloadOutlined, ReloadOutlined, SearchOutlined, UndoOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import type { FetchModelsModalProps, FetchedModel, ApiType, FetchModelsResponse } from './types';
import { createFetchedModelsComparator } from './sort';
import { buildModelsUrl, getDefaultModelsApiType, resolveModelsUrlApiKey } from './request';
import styles from './index.module.less';

const { Text } = Typography;

const FetchModelsModal: React.FC<FetchModelsModalProps> = ({
  open,
  providerId,
  providerName,
  baseUrl,
  apiKey,
  headers,
  sdkType,
  configValueMode,
  existingModelIds,
  priorityOwnedBy,
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  // Default to native if supported, otherwise openai_compat
  const [apiType, setApiType] = React.useState<ApiType>(() => getDefaultModelsApiType(sdkType));
  const [models, setModels] = React.useState<FetchedModel[]>([]);
  const [selectedRowKeys, setSelectedRowKeys] = React.useState<string[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [fetched, setFetched] = React.useState(false);
  const [customUrl, setCustomUrl] = React.useState('');
  const [searchText, setSearchText] = React.useState('');
  const [removeMissingModels, setRemoveMissingModels] = React.useState(false);
  const fetchRequestIdRef = React.useRef(0);

  // Only show Native option for Google and Anthropic SDKs
  const supportsNative = sdkType === '@ai-sdk/google' || sdkType === '@ai-sdk/anthropic';

  // Sort models grouped by owner so models read vendor-by-vendor instead of
  // raw API order; consumers can pin specific owners (e.g. Codex pins openai)
  const sortComparator = React.useMemo(
    () => createFetchedModelsComparator(priorityOwnedBy),
    [priorityOwnedBy],
  );

  // Filter models based on search text
  const filteredModels = React.useMemo(() => {
    const lowerSearch = searchText.toLowerCase();
    const list = searchText
      ? models.filter(m =>
          m.id.toLowerCase().includes(lowerSearch) ||
          (m.name && m.name.toLowerCase().includes(lowerSearch)) ||
          (m.ownedBy && m.ownedBy.toLowerCase().includes(lowerSearch))
        )
      : models;
    return [...list].sort(sortComparator);
  }, [models, searchText, sortComparator]);

  // Calculate the default URL based on baseUrl, apiType, and sdkType
  const calculatedUrl = React.useMemo(
    () => buildModelsUrl(baseUrl, apiType, sdkType, resolveModelsUrlApiKey(apiKey, configValueMode)),
    [baseUrl, apiType, sdkType, apiKey, configValueMode],
  );

  // The component stays mounted while providers and their SDKs can change.
  React.useEffect(() => {
    if (open) setApiType(getDefaultModelsApiType(sdkType));
  }, [open, providerId, sdkType]);

  // Reset state when modal opens
  React.useEffect(() => {
    fetchRequestIdRef.current += 1;
    setLoading(false);
    if (open) {
      setModels([]);
      setSelectedRowKeys([]);
      setError(null);
      setFetched(false);
      setSearchText('');
      setRemoveMissingModels(false);
      // Reset custom URL to calculated default
      setCustomUrl(calculatedUrl);
    }
    return () => { fetchRequestIdRef.current += 1; };
  }, [open, providerId, sdkType, apiType, apiKey, calculatedUrl]);

  // Fetch models from provider API
  const handleFetch = async () => {
    const requestId = ++fetchRequestIdRef.current;
    setLoading(true);
    setError(null);

    try {
      const response = await invoke<FetchModelsResponse>('fetch_provider_models', {
        request: {
          providerId,
          baseUrl,
          apiKey,
          headers,
          apiType,
          sdkType,
          configValueMode,
          customUrl, // Use custom URL instead of calculated one
        },
      });

      if (requestId !== fetchRequestIdRef.current) return;

      setModels(response.models);
      setFetched(true);

      // Don't auto-select, let user choose manually
      setSelectedRowKeys([]);
      setRemoveMissingModels(false);

      if (response.models.length === 0) {
        message.info(t('opencode.fetchModels.noModelsFound'));
      }
    } catch (err) {
      if (requestId !== fetchRequestIdRef.current) return;
      const errorMsg = err instanceof Error ? err.message : String(err);
      setError(errorMsg);
      message.error(t('opencode.fetchModels.fetchFailed'));
    } finally {
      if (requestId === fetchRequestIdRef.current) setLoading(false);
    }
  };

  // Confirm and add selected models
  const handleConfirm = () => {
    // Select from the full sorted list, not the search-filtered view, so rows
    // hidden by an active search filter are not silently dropped; the sorted
    // order keeps the applied order in line with what was displayed.
    const sortedModels = [...models].sort(sortComparator);
    const selectedModels = sortedModels.filter((m) => selectedRowKeys.includes(m.id));
    const fetchedModelIds = new Set(models.map((model) => model.id));
    const removedModelIds = removeMissingModels
      ? existingModelIds.filter((modelId) => !fetchedModelIds.has(modelId))
      : [];
    const orderedModelIds = sortedModels.map((model) => model.id);
    onSuccess({ selectedModels, removedModelIds, orderedModelIds });
  };

  // Table columns
  const columns = [
    {
      title: t('opencode.fetchModels.modelId'),
      dataIndex: 'id',
      key: 'id',
      render: (id: string) => {
        const isExisting = existingModelIds.includes(id);
        return (
          <div className={styles.modelIdCell}>
            <Text className={styles.modelIdText}>{id}</Text>
            {isExisting && (
              <Tag bordered={false} className={styles.existingTag}>
                {t('opencode.fetchModels.alreadyExists')}
              </Tag>
            )}
          </div>
        );
      },
    },
    {
      title: t('common.model.name'),
      dataIndex: 'name',
      key: 'name',
      width: 220,
      ellipsis: true,
      render: (name: string | undefined, record: FetchedModel) => {
        // The backend falls back to the model id when the provider sends no
        // display name, so an unchanged value must not read as a second id.
        const displayName = name?.trim();
        if (!displayName || displayName === record.id.trim()) {
          return <Text className={styles.ownedByText}>-</Text>;
        }
        return <Text className={styles.modelNameText}>{displayName}</Text>;
      },
    },
    {
      title: t('opencode.fetchModels.ownedBy'),
      dataIndex: 'ownedBy',
      key: 'ownedBy',
      width: 150,
      render: (ownedBy: string | undefined) => <Text className={styles.ownedByText}>{ownedBy || '-'}</Text>,
    },
  ];

  const rowSelection = {
    preserveSelectedRowKeys: true,
    selectedRowKeys,
    onChange: (keys: React.Key[]) => setSelectedRowKeys(keys as string[]),
    getCheckboxProps: (record: FetchedModel) => ({
      disabled: existingModelIds.includes(record.id),
      name: record.id,
    }),
  };

  const missingModelCount = React.useMemo(() => {
    if (!fetched) return 0;
    const fetchedModelIds = new Set(models.map((model) => model.id));
    return existingModelIds.filter((modelId) => !fetchedModelIds.has(modelId)).length;
  }, [existingModelIds, fetched, models]);

  const canConfirm = selectedRowKeys.length > 0 || (removeMissingModels && missingModelCount > 0);

  return (
    <Modal
      className={styles.modal}
      title={
        <Space>
          <CloudDownloadOutlined />
          {t('opencode.fetchModels.title', { provider: providerName })}
        </Space>
      }
      open={open}
      onCancel={onCancel}
      width={820}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button
          key="confirm"
          type="primary"
          disabled={loading || !canConfirm}
          onClick={handleConfirm}
        >
          {t('opencode.fetchModels.applyChanges', {
            addCount: selectedRowKeys.length,
            removeCount: removeMissingModels ? missingModelCount : 0,
          })}
        </Button>,
      ]}
    >
      <div className={styles.content}>
        <section className={`${styles.sectionCard} ${styles.sourceCard}`}>
          <div className={styles.sectionHeader}>
            <div className={styles.sectionTitle}>{t('opencode.fetchModels.sourceSection')}</div>
            <Text className={styles.sectionHint}>{t('opencode.fetchModels.sourceSectionHint')}</Text>
          </div>

          <div className={styles.fieldRow}>
            <Text strong className={styles.fieldLabel}>
              {t('opencode.fetchModels.apiType')}
            </Text>
            <Radio.Group
              value={apiType}
              onChange={(e) => setApiType(e.target.value)}
              className={styles.apiTypeGroup}
            >
              <Radio value="openai_compat">
                {t('opencode.fetchModels.openaiCompat')}
                <Text type="secondary" style={{ marginLeft: 8, fontSize: 12 }}>
                  (/models)
                </Text>
              </Radio>
              {supportsNative && (
                <Radio value="native">
                  {t('opencode.fetchModels.native')}
                  <Text type="secondary" style={{ marginLeft: 8, fontSize: 12 }}>
                    ({t('opencode.fetchModels.nativeHint')})
                  </Text>
                </Radio>
              )}
            </Radio.Group>
          </div>

          <div className={styles.fieldRow}>
            <Text strong className={styles.fieldLabel}>
              {t('opencode.fetchModels.apiUrl')}
            </Text>
            <Input
              className={styles.urlInput}
              value={customUrl}
              onChange={(e) => setCustomUrl(e.target.value)}
              placeholder="https://api.example.com/v1/models"
              addonAfter={
                <Tooltip title={t('opencode.fetchModels.resetToDefault')}>
                  <Button
                    type="text"
                    size="small"
                    icon={<UndoOutlined />}
                    onClick={() => setCustomUrl(calculatedUrl)}
                    style={{ fontSize: 12 }}
                  />
                </Tooltip>
              }
            />
          </div>
        </section>

        <section className={`${styles.sectionCard} ${styles.resultCard}`}>
          <div className={styles.sectionHeader}>
            <div className={styles.sectionTitle}>{t('opencode.fetchModels.resultSection')}</div>
            <Text className={styles.sectionHint}>{t('opencode.fetchModels.resultSectionHint')}</Text>
          </div>

          <div className={styles.toolbar}>
            <Button
              type="primary"
              icon={fetched ? <ReloadOutlined /> : <CloudDownloadOutlined />}
              loading={loading}
              onClick={handleFetch}
            >
              {fetched ? t('opencode.fetchModels.refresh') : t('opencode.fetchModels.fetch')}
            </Button>
            <Input
              className={styles.searchInput}
              prefix={<SearchOutlined />}
              placeholder={t('opencode.fetchModels.searchPlaceholder')}
              value={searchText}
              onChange={(e) => setSearchText(e.target.value)}
              allowClear
            />
            {fetched && (
              <Text className={styles.statsText}>
                {t('opencode.fetchModels.statsLine', {
                  returned: models.length,
                  selected: selectedRowKeys.length,
                })}
              </Text>
            )}
          </div>

          {fetched && (
            <div className={styles.cleanupRow}>
              {missingModelCount > 0 ? (
                <Checkbox
                  checked={removeMissingModels}
                  onChange={(event) => setRemoveMissingModels(event.target.checked)}
                >
                  {t('opencode.fetchModels.removeMissing', { count: missingModelCount })}
                </Checkbox>
              ) : (
                <Text type="secondary" className={styles.statsMuted}>
                  {t('opencode.fetchModels.removeMissingNone')}
                </Text>
              )}
            </div>
          )}

          {error && (
            <Alert
              type="error"
              message={t('opencode.fetchModels.fetchFailed')}
              description={error}
              showIcon
              closable
              onClose={() => setError(null)}
            />
          )}

          {fetched && (
            <div className={styles.tableWrap}>
              <Table
                rowKey="id"
                columns={columns}
                dataSource={filteredModels}
                rowSelection={rowSelection}
                pagination={false}
                size="small"
                locale={{
                  emptyText: searchText ? t('opencode.fetchModels.noSearchResults') : t('opencode.fetchModels.noModelsFound'),
                }}
              />
            </div>
          )}
        </section>
      </div>
    </Modal>
  );
};

export default FetchModelsModal;
