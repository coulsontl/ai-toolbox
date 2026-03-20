import React from 'react';
import { Button, Collapse, Empty, Modal, Space, Spin, Typography, message } from 'antd';
import { FileTextOutlined, PlusOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  DndContext,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  SortableContext,
  arrayMove,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import type {
  GlobalPromptConfig,
  GlobalPromptConfigInput,
} from '@/types/globalPrompt';
import type { GlobalPromptApi } from '@/services/globalPromptApi';
import GlobalPromptConfigCard from './GlobalPromptConfigCard';
import GlobalPromptConfigModal, { type GlobalPromptConfigFormValues } from './GlobalPromptConfigModal';
import styles from './GlobalPromptSettings.module.less';

const { Text } = Typography;

interface GlobalPromptSettingsProps {
  translationKeyPrefix: string;
  service: GlobalPromptApi;
  collapseKey: string;
  refreshKey?: number;
  onUpdated?: () => Promise<void> | void;
  /**
   * Whether the collapse panel should be opened on mount.
   * Used by pages that provide "jump to section" sidebars.
   */
  defaultExpanded?: boolean;
}

const GlobalPromptSettings: React.FC<GlobalPromptSettingsProps> = ({
  translationKeyPrefix,
  service,
  collapseKey,
  refreshKey = 0,
  onUpdated,
  defaultExpanded = false,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [configs, setConfigs] = React.useState<GlobalPromptConfig[]>([]);
  const [configModalOpen, setConfigModalOpen] = React.useState(false);
  const [editingConfig, setEditingConfig] = React.useState<GlobalPromptConfig | null>(null);

  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8,
      },
    })
  );

  const loadConfigs = React.useCallback(async () => {
    setLoading(true);
    try {
      const configList = await service.listConfigs();
      setConfigs(configList);
    } catch (error) {
      console.error('Failed to load global prompt configs:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [service, t]);

  React.useEffect(() => {
    // Reference refreshKey to make the dependency explicit for the linter.
    void refreshKey;
    loadConfigs();
  }, [loadConfigs, refreshKey]);

  const notifyUpdated = async () => {
    await onUpdated?.();
  };

  const handleAddConfig = () => {
    setEditingConfig(null);
    setConfigModalOpen(true);
  };

  const handleEditConfig = (config: GlobalPromptConfig) => {
    setEditingConfig({ ...config });
    setConfigModalOpen(true);
  };

  const handleDeleteConfig = (config: GlobalPromptConfig) => {
    Modal.confirm({
      title: t('common.confirm'),
      content: t(`${translationKeyPrefix}.confirmDelete`, { name: config.name }),
      onOk: async () => {
        try {
          await service.deleteConfig(config.id);
          message.success(t('common.success'));
          await loadConfigs();
          await notifyUpdated();
        } catch (error) {
          console.error('Failed to delete global prompt config:', error);
          message.error(t('common.error'));
        }
      },
    });
  };

  const handleApplyConfig = async (config: GlobalPromptConfig) => {
    try {
      await service.applyConfig(config.id);
      message.success(t(`${translationKeyPrefix}.applySuccess`));
      await loadConfigs();
      await notifyUpdated();
    } catch (error) {
      console.error('Failed to apply global prompt config:', error);
      message.error(t('common.error'));
    }
  };

  const handleConfigSuccess = async (values: GlobalPromptConfigFormValues) => {
    const payload: GlobalPromptConfigInput = {
      id: editingConfig?.id !== '__local__' ? editingConfig?.id : undefined,
      name: values.name,
      content: values.content,
    };

    try {
      if (editingConfig?.id === '__local__') {
        await service.saveLocalConfig(payload);
      } else if (editingConfig?.id) {
        await service.updateConfig(payload);
      } else {
        await service.createConfig(payload);
      }

      message.success(t('common.success'));
      setConfigModalOpen(false);
      setEditingConfig(null);
      await loadConfigs();
      await notifyUpdated();
    } catch (error) {
      console.error('Failed to save global prompt config:', error);
      message.error(t('common.error'));
    }
  };

  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) {
      return;
    }

    if (configs.some((config) => config.id === '__local__')) {
      return;
    }

    const oldIndex = configs.findIndex((config) => config.id === active.id);
    const newIndex = configs.findIndex((config) => config.id === over.id);

    if (oldIndex === -1 || newIndex === -1) {
      return;
    }

    const oldConfigs = [...configs];
    const newConfigs = arrayMove(configs, oldIndex, newIndex);
    setConfigs(newConfigs);

    try {
      await service.reorderConfigs(newConfigs.map((config) => config.id));
      await notifyUpdated();
    } catch (error) {
      console.error('Failed to reorder global prompt configs:', error);
      setConfigs(oldConfigs);
      message.error(t('common.error'));
    }
  };

  const content = (
    <Spin spinning={loading}>
      <div className={styles.hintBlock}>
        <div>{t(`${translationKeyPrefix}.sectionHint`)}</div>
        <div>{t(`${translationKeyPrefix}.sectionWarning`)}</div>
      </div>

      {configs.length === 0 ? (
        <Empty description={t(`${translationKeyPrefix}.emptyText`)} style={{ margin: '24px 0' }} />
      ) : (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          modifiers={[restrictToVerticalAxis]}
          onDragEnd={handleDragEnd}
        >
          <SortableContext items={configs.map((config) => config.id)} strategy={verticalListSortingStrategy}>
            <div>
              {configs.map((config) => (
                <GlobalPromptConfigCard
                  key={config.id}
                  config={config}
                  translationKeyPrefix={translationKeyPrefix}
                  onEdit={handleEditConfig}
                  onDelete={handleDeleteConfig}
                  onApply={handleApplyConfig}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      )}
    </Spin>
  );

  const appliedConfig = configs.find((config) => config.isApplied);

  return (
    <>
      <Collapse
        style={{ marginBottom: 16 }}
        defaultActiveKey={defaultExpanded ? [collapseKey] : []}
        items={[
          {
            key: collapseKey,
            label: (
              <Space>
                <Text strong>
                  <FileTextOutlined style={{ marginRight: 8 }} />
                  {t(`${translationKeyPrefix}.title`)}
                </Text>
                {appliedConfig && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {t(`${translationKeyPrefix}.current`)}: {appliedConfig.name}
                  </Text>
                )}
              </Space>
            ),
            extra: (
              <Button
                type="link"
                size="small"
                style={{ fontSize: 12 }}
                icon={<PlusOutlined />}
                onClick={(event) => {
                  event.stopPropagation();
                  handleAddConfig();
                }}
              >
                {t(`${translationKeyPrefix}.addConfig`)}
              </Button>
            ),
            children: content,
          },
        ]}
      />

      <GlobalPromptConfigModal
        open={configModalOpen}
        translationKeyPrefix={translationKeyPrefix}
        initialValues={editingConfig || undefined}
        onCancel={() => {
          setConfigModalOpen(false);
          setEditingConfig(null);
        }}
        onSuccess={handleConfigSuccess}
      />
    </>
  );
};

export default GlobalPromptSettings;
