import React from 'react';
import { Alert, Button, Input, Modal, Select, Spin, Table, message } from 'antd';
import type { ColumnsType } from 'antd/es/table';
import { Pencil, Plus, RefreshCw, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  deleteModelPricing,
  fetchRemoteModelPricing,
  getGatewayPricingConfig,
  getModelPricingList,
  saveGatewayPricingConfig,
  type GatewayPricingConfig,
  type ModelPricing,
} from '@/services';
import ModelPricingEditModal from './ModelPricingEditModal';
import styles from './ModelPricingModal.module.less';

interface ModelPricingModalProps {
  open: boolean;
  onClose: () => void;
}

type PricingCliKey = 'claude' | 'codex' | 'grok' | 'gemini';
type PricingConfigState = Record<PricingCliKey, GatewayPricingConfig>;

const pricingCliKeys: readonly PricingCliKey[] = ['claude', 'codex', 'grok', 'gemini'];
const costPattern = /^\d+(?:\.\d+)?$/;

const createDefaultPricingConfigs = (): PricingConfigState => ({
  claude: { cost_multiplier: '1.0', pricing_model_source: 'upstream' },
  codex: { cost_multiplier: '1.0', pricing_model_source: 'upstream' },
  grok: { cost_multiplier: '1.0', pricing_model_source: 'upstream' },
  gemini: { cost_multiplier: '1.0', pricing_model_source: 'upstream' },
});

const createEmptyPricing = (): ModelPricing => ({
  model_id: '',
  display_name: '',
  input_cost_per_million: '0',
  output_cost_per_million: '0',
  cache_read_cost_per_million: '0',
  cache_creation_cost_per_million: '0',
});

const isConfigDirty = (
  currentConfigs: PricingConfigState,
  originalConfigs: PricingConfigState | null,
) => {
  if (!originalConfigs) {
    return false;
  }
  return pricingCliKeys.some((cliKey) => {
    const currentConfig = currentConfigs[cliKey];
    const originalConfig = originalConfigs[cliKey];
    return (
      currentConfig.cost_multiplier !== originalConfig.cost_multiplier ||
      currentConfig.pricing_model_source !== originalConfig.pricing_model_source
    );
  });
};

const isNonNegativeDecimalString = (value: string) => costPattern.test(value.trim());

const formatCost = (value: string) => `$${value}`;

const ModelPricingModal: React.FC<ModelPricingModalProps> = ({ open, onClose }) => {
  const { t } = useTranslation();
  const [pricingList, setPricingList] = React.useState<ModelPricing[]>([]);
  const [pricingLoading, setPricingLoading] = React.useState(false);
  const [configLoading, setConfigLoading] = React.useState(false);
  const [savingConfig, setSavingConfig] = React.useState(false);
  const [syncingOfficialPricing, setSyncingOfficialPricing] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [pricingConfigs, setPricingConfigs] = React.useState<PricingConfigState>(
    createDefaultPricingConfigs,
  );
  const [originalPricingConfigs, setOriginalPricingConfigs] =
    React.useState<PricingConfigState | null>(null);
  const [editingPricing, setEditingPricing] = React.useState<ModelPricing | null>(null);
  const [isAddingPricing, setIsAddingPricing] = React.useState(false);

  const loadPricingList = React.useCallback(async () => {
    setPricingLoading(true);
    try {
      const nextPricingList = await getModelPricingList();
      setPricingList(nextPricingList);
    } catch (loadError) {
      setError(
        t('gateway.page.pricing.loadModelFailed', {
          error: loadError instanceof Error ? loadError.message : String(loadError),
        }),
      );
    } finally {
      setPricingLoading(false);
    }
  }, [t]);

  const loadPricingConfigs = React.useCallback(async () => {
    setConfigLoading(true);
    try {
      const loadedConfigs = await Promise.all(
        pricingCliKeys.map(async (cliKey) => {
          const config = await getGatewayPricingConfig(cliKey);
          return [cliKey, config] as const;
        }),
      );
      const nextConfigs = createDefaultPricingConfigs();
      for (const [cliKey, config] of loadedConfigs) {
        nextConfigs[cliKey] = {
          cost_multiplier: config.cost_multiplier,
          pricing_model_source: config.pricing_model_source,
        };
      }
      setPricingConfigs(nextConfigs);
      setOriginalPricingConfigs(nextConfigs);
    } catch (loadError) {
      setError(
        t('gateway.page.pricing.loadConfigFailed', {
          error: loadError instanceof Error ? loadError.message : String(loadError),
        }),
      );
    } finally {
      setConfigLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setError(null);
    void loadPricingConfigs();
    void loadPricingList();
  }, [loadPricingConfigs, loadPricingList, open]);

  const dirty = isConfigDirty(pricingConfigs, originalPricingConfigs);

  const updatePricingConfig = React.useCallback(
    (cliKey: PricingCliKey, patch: Partial<GatewayPricingConfig>) => {
      setPricingConfigs((currentConfigs) => ({
        ...currentConfigs,
        [cliKey]: {
          ...currentConfigs[cliKey],
          ...patch,
        },
      }));
    },
    [],
  );

  const handleSaveConfigs = React.useCallback(async () => {
    for (const cliKey of pricingCliKeys) {
      if (!isNonNegativeDecimalString(pricingConfigs[cliKey].cost_multiplier)) {
        message.error(
          t('gateway.page.pricing.invalidMultiplierForCli', {
            cli: t(`settings.gateway.cli.${cliKey}`),
          }),
        );
        return;
      }
    }

    setSavingConfig(true);
    try {
      const savedConfigs = createDefaultPricingConfigs();
      for (const cliKey of pricingCliKeys) {
        savedConfigs[cliKey] = await saveGatewayPricingConfig(cliKey, {
          cost_multiplier: pricingConfigs[cliKey].cost_multiplier.trim(),
          pricing_model_source: pricingConfigs[cliKey].pricing_model_source,
        });
      }
      setPricingConfigs(savedConfigs);
      setOriginalPricingConfigs(savedConfigs);
      message.success(t('gateway.page.pricing.configSaved'));
    } catch (saveError) {
      message.error(
        t('gateway.page.pricing.saveConfigFailed', {
          error: saveError instanceof Error ? saveError.message : String(saveError),
        }),
      );
    } finally {
      setSavingConfig(false);
    }
  }, [pricingConfigs, t]);

  const handleAddPricing = React.useCallback(() => {
    setIsAddingPricing(true);
    setEditingPricing(createEmptyPricing());
  }, []);

  const handleSyncOfficialPricing = React.useCallback(async () => {
    setSyncingOfficialPricing(true);
    try {
      const result = await fetchRemoteModelPricing();
      message.success(
        t('gateway.page.pricing.officialSyncSucceeded', {
          count: result.inserted_count,
        }),
      );
      await loadPricingList();
    } catch (syncError) {
      message.error(
        t('gateway.page.pricing.officialSyncFailed', {
          error: syncError instanceof Error ? syncError.message : String(syncError),
        }),
      );
    } finally {
      setSyncingOfficialPricing(false);
    }
  }, [loadPricingList, t]);

  const handleEditPricing = React.useCallback((pricing: ModelPricing) => {
    setIsAddingPricing(false);
    setEditingPricing(pricing);
  }, []);

  const handleDeletePricing = React.useCallback(
    (pricing: ModelPricing) => {
      Modal.confirm({
        title: t('gateway.page.pricing.deleteConfirmTitle'),
        content: t('gateway.page.pricing.deleteConfirmDesc', { modelId: pricing.model_id }),
        okText: t('common.delete'),
        cancelText: t('common.cancel'),
        okButtonProps: { danger: true },
        onOk: async () => {
          await deleteModelPricing(pricing.model_id);
          message.success(t('gateway.page.pricing.pricingDeleted'));
          await loadPricingList();
        },
      });
    },
    [loadPricingList, t],
  );

  const handlePricingSaved = React.useCallback(() => {
    void loadPricingList();
  }, [loadPricingList]);

  const pricingColumns: ColumnsType<ModelPricing> = [
    {
      title: t('gateway.page.pricing.modelId'),
      dataIndex: 'model_id',
      width: 220,
      render: (value: string) => <span className={styles.monoCell}>{value}</span>,
    },
    {
      title: t('gateway.page.pricing.displayName'),
      dataIndex: 'display_name',
      width: 180,
    },
    {
      title: t('gateway.page.pricing.tableInput'),
      dataIndex: 'input_cost_per_million',
      width: 76,
      align: 'right',
      render: formatCost,
    },
    {
      title: t('gateway.page.pricing.tableOutput'),
      dataIndex: 'output_cost_per_million',
      width: 76,
      align: 'right',
      render: formatCost,
    },
    {
      title: t('gateway.page.pricing.tableCacheRead'),
      dataIndex: 'cache_read_cost_per_million',
      width: 96,
      align: 'right',
      render: formatCost,
    },
    {
      title: t('gateway.page.pricing.tableCacheCreation'),
      dataIndex: 'cache_creation_cost_per_million',
      width: 96,
      align: 'right',
      render: formatCost,
    },
    {
      title: t('gateway.page.pricing.actions'),
      key: 'actions',
      width: 88,
      align: 'right',
      render: (_, record) => (
        <div className={styles.rowActions}>
          <Button
            type="text"
            size="small"
            icon={<Pencil size={14} />}
            aria-label={t('common.edit')}
            title={t('common.edit')}
            onClick={() => handleEditPricing(record)}
          />
          <Button
            type="text"
            size="small"
            danger
            icon={<Trash2 size={14} />}
            aria-label={t('common.delete')}
            title={t('common.delete')}
            onClick={() => handleDeletePricing(record)}
          />
        </div>
      ),
    },
  ];

  return (
    <Modal
      open={open}
      title={t('gateway.page.pricing.title')}
      width={980}
      className={styles.modal}
      footer={null}
      onCancel={onClose}
    >
      <div className={styles.content}>
        {error ? <Alert type="error" showIcon message={error} /> : null}

        <section className={styles.sectionCard}>
          <div className={styles.sectionHeader}>
            <h3>{t('gateway.page.pricing.defaultsTitle')}</h3>
            <Button
              type="primary"
              size="small"
              loading={savingConfig}
              disabled={configLoading || savingConfig || !dirty}
              onClick={() => void handleSaveConfigs()}
            >
              {t('common.save')}
            </Button>
          </div>

          {configLoading ? (
            <div className={styles.loadingBlock}>
              <Spin size="small" />
            </div>
          ) : (
            <div className={styles.defaultsTableWrap}>
              <table className={styles.defaultsTable}>
                <thead>
                  <tr>
                    <th>{t('gateway.page.pricing.cli')}</th>
                    <th>{t('gateway.page.pricing.costMultiplier')}</th>
                    <th>{t('gateway.page.pricing.pricingModelSource')}</th>
                  </tr>
                </thead>
                <tbody>
                  {pricingCliKeys.map((cliKey) => (
                    <tr key={cliKey}>
                      <td>{t(`settings.gateway.cli.${cliKey}`)}</td>
                      <td>
                        <Input
                          size="small"
                          inputMode="decimal"
                          className={styles.multiplierInput}
                          value={pricingConfigs[cliKey].cost_multiplier}
                          onChange={(event) =>
                            updatePricingConfig(cliKey, {
                              cost_multiplier: event.target.value,
                            })
                          }
                        />
                      </td>
                      <td>
                        <Select
                          size="small"
                          className={styles.sourceSelect}
                          value={pricingConfigs[cliKey].pricing_model_source}
                          options={[
                            {
                              value: 'upstream',
                              label: t('gateway.page.pricing.sourceUpstream'),
                            },
                            {
                              value: 'requested',
                              label: t('gateway.page.pricing.sourceRequested'),
                            },
                          ]}
                          onChange={(value) =>
                            updatePricingConfig(cliKey, {
                              pricing_model_source: value,
                            })
                          }
                        />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>

        <section className={styles.listSection}>
          <div className={styles.sectionHeader}>
            <h3>
              {t('gateway.page.pricing.modelPricingDesc')}{' '}
              <span>{t('gateway.page.pricing.perMillion')}</span>
            </h3>
            <div className={styles.headerActions}>
              <Button
                size="small"
                icon={<RefreshCw size={14} />}
                loading={syncingOfficialPricing}
                disabled={syncingOfficialPricing}
                onClick={() => void handleSyncOfficialPricing()}
              >
                {t('gateway.page.pricing.syncOfficial')}
              </Button>
              <Button
                type="primary"
                size="small"
                icon={<Plus size={14} />}
                onClick={handleAddPricing}
              >
                {t('common.add')}
              </Button>
            </div>
          </div>
          <Table
            rowKey="model_id"
            size="small"
            columns={pricingColumns}
            dataSource={pricingList}
            loading={pricingLoading}
            pagination={{ pageSize: 10, size: 'small' }}
            scroll={{ x: 840 }}
          />
        </section>
      </div>

      <ModelPricingEditModal
        open={Boolean(editingPricing)}
        pricing={editingPricing}
        isNew={isAddingPricing}
        onClose={() => {
          setEditingPricing(null);
          setIsAddingPricing(false);
        }}
        onSaved={handlePricingSaved}
      />
    </Modal>
  );
};

export default ModelPricingModal;
