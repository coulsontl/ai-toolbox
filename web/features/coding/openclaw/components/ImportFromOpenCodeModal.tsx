import React from 'react';
import { Modal, Checkbox, Button, Empty, Spin, Typography, Tag, message } from 'antd';
import { CloudServerOutlined, AppstoreOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { readOpenCodeConfig } from '@/services/opencodeApi';
import type { OpenCodeProvider } from '@/types/opencode';
import type { OpenClawProviderConfig, OpenClawModel } from '@/types/openclaw';
import styles from './ImportFromOpenCodeModal.module.less';

const { Text } = Typography;

export interface ImportedProvider {
  providerId: string;
  config: OpenClawProviderConfig;
}

interface Props {
  open: boolean;
  existingProviderIds: string[];
  onCancel: () => void;
  onImport: (providers: ImportedProvider[]) => void;
}

/** Map OpenCode npm package to OpenClaw api protocol */
function npmToApi(npm?: string): string | undefined {
  switch (npm) {
    case '@ai-sdk/anthropic':
      return 'anthropic-messages';
    case '@ai-sdk/google':
      return 'google-generative-ai';
    case '@ai-sdk/openai':
      return 'openai-responses';
    case '@ai-sdk/openai-compatible':
      return 'openai-completions';
    default:
      return undefined;
  }
}

/** Convert an OpenCode provider to OpenClaw format */
function convertProvider(
  providerId: string,
  oc: OpenCodeProvider,
): ImportedProvider {
  const models: OpenClawModel[] = [];
  if (oc.models) {
    for (const [modelId, modelCfg] of Object.entries(oc.models)) {
      models.push({
        id: modelId,
        name: modelCfg.name || undefined,
        contextWindow: modelCfg.limit?.context,
        maxTokens: modelCfg.limit?.output,
        reasoning: modelCfg.reasoning || undefined,
      });
    }
  }

  const config: OpenClawProviderConfig = { models };
  if (oc.options?.baseURL) config.baseUrl = oc.options.baseURL;
  if (oc.options?.apiKey) config.apiKey = oc.options.apiKey;
  const api = npmToApi(oc.npm);
  if (api) config.api = api;

  return { providerId, config };
}

const ImportFromOpenCodeModal: React.FC<Props> = ({
  open,
  existingProviderIds,
  onCancel,
  onImport,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [entries, setEntries] = React.useState<
    { providerId: string; provider: OpenCodeProvider }[]
  >([]);
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(new Set());

  // Load OpenCode providers when modal opens
  const loadProviders = React.useCallback(async () => {
    setLoading(true);
    try {
      const cfg = await readOpenCodeConfig();
      if (!cfg || !cfg.provider) {
        setEntries([]);
        return;
      }
      const list = Object.entries(cfg.provider).map(([id, p]) => ({
        providerId: id,
        provider: p,
      }));
      setEntries(list);
      setSelectedIds(new Set());
    } catch (error) {
      console.error('Failed to load OpenCode config:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    if (open) {
      loadProviders();
    }
  }, [open, loadProviders]);

  const handleToggle = (id: string, checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (checked) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const importable = entries.filter(
    (e) => !existingProviderIds.includes(e.providerId),
  );
  const isAllSelected =
    importable.length > 0 && importable.every((e) => selectedIds.has(e.providerId));

  const handleSelectAll = () => {
    setSelectedIds(new Set(importable.map((e) => e.providerId)));
  };
  const handleDeselectAll = () => {
    setSelectedIds(new Set());
  };

  const importableCount = Array.from(selectedIds).filter(
    (id) => !existingProviderIds.includes(id),
  ).length;

  const handleImport = () => {
    const selected = entries.filter((e) => selectedIds.has(e.providerId));
    if (selected.length === 0) return;
    const converted = selected.map((e) =>
      convertProvider(e.providerId, e.provider),
    );
    onImport(converted);
  };

  // Sort: importable first, then existing
  const sorted = React.useMemo(
    () =>
      [...entries].sort((a, b) => {
        const ae = existingProviderIds.includes(a.providerId);
        const be = existingProviderIds.includes(b.providerId);
        if (ae === be) return 0;
        return ae ? 1 : -1;
      }),
    [entries, existingProviderIds],
  );

  return (
    <Modal
      title={t('openclaw.providers.importFromOpenCode')}
      open={open}
      onCancel={onCancel}
      width={640}
      className={styles.modal}
      destroyOnClose
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button
          key="import"
          type="primary"
          onClick={handleImport}
          disabled={importableCount === 0}
        >
          {t('openclaw.providers.importSelected')} ({importableCount})
        </Button>,
      ]}
    >
      <Spin spinning={loading}>
        {entries.length === 0 && !loading ? (
          <Empty description={t('openclaw.providers.importNoConfig')} />
        ) : (
          <div>
            <div className={styles.toolbar}>
              <Checkbox
                checked={isAllSelected}
                indeterminate={selectedIds.size > 0 && !isAllSelected}
                onChange={(e) =>
                  e.target.checked ? handleSelectAll() : handleDeselectAll()
                }
              >
                {t('openclaw.providers.selectAll')}
              </Checkbox>
            </div>
            <div className={styles.container}>
              {sorted.map((entry) => {
                const isExisting = existingProviderIds.includes(
                  entry.providerId,
                );
                const isSelected = selectedIds.has(entry.providerId);
                const modelIds = Object.keys(entry.provider.models || {});

                return (
                  <div
                    key={entry.providerId}
                    className={`${styles.card} ${isExisting ? styles.existing : ''} ${isSelected && !isExisting ? styles.selected : ''}`}
                    onClick={() =>
                      !isExisting &&
                      handleToggle(entry.providerId, !isSelected)
                    }
                  >
                    <div className={styles.cardHeader}>
                      <Checkbox
                        checked={isSelected}
                        disabled={isExisting}
                        onClick={(e) => e.stopPropagation()}
                        onChange={(e) =>
                          handleToggle(entry.providerId, e.target.checked)
                        }
                      />
                      <div className={styles.titleArea}>
                        <Text strong className={styles.title}>
                          {entry.provider.name || entry.providerId}
                        </Text>
                        {isExisting && (
                          <Tag className={styles.existsTag}>
                            {t('openclaw.providers.alreadyExists')}
                          </Tag>
                        )}
                      </div>
                    </div>

                    <div className={styles.cardBody}>
                      <div className={styles.infoRow}>
                        {entry.provider.options?.baseURL && (
                          <div className={styles.infoItem}>
                            <CloudServerOutlined className={styles.icon} />
                            <Text
                              className={styles.infoText}
                              ellipsis
                              title={entry.provider.options.baseURL}
                            >
                              {entry.provider.options.baseURL}
                            </Text>
                          </div>
                        )}
                      </div>
                      {modelIds.length > 0 && (
                        <div className={styles.modelsRow}>
                          <AppstoreOutlined className={styles.icon} />
                          <div className={styles.modelTags}>
                            {modelIds.map((mid) => (
                              <Tag key={mid} className={styles.modelTag}>
                                {mid}
                              </Tag>
                            ))}
                          </div>
                        </div>
                      )}
                      {modelIds.length === 0 && (
                        <div className={styles.modelsRow}>
                          <AppstoreOutlined className={styles.icon} />
                          <Text type="secondary" className={styles.noModels}>
                            0
                          </Text>
                        </div>
                      )}
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

export default ImportFromOpenCodeModal;
