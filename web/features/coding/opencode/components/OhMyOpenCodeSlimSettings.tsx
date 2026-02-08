import React from 'react';
import { Button, Typography, Collapse, Empty, Spin, Space, message, Modal, Alert, Tag } from 'antd';
import { PlusOutlined, LinkOutlined, WarningOutlined, AppstoreOutlined, ThunderboltOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  DndContext,
  closestCenter,
  PointerSensor,
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
import type { OhMyOpenCodeSlimConfig, OhMyOpenCodeSlimGlobalConfig, OhMyOpenCodeSlimGlobalConfigInput } from '@/types/ohMyOpenCodeSlim';
import OhMyOpenCodeSlimConfigCard from './OhMyOpenCodeSlimConfigCard';
import OhMyOpenCodeSlimConfigModal, { OhMyOpenCodeSlimConfigFormValues } from './OhMyOpenCodeSlimConfigModal';
import OhMyOpenCodeSlimGlobalConfigModal from './OhMyOpenCodeSlimGlobalConfigModal';
import {
  listOhMyOpenCodeSlimConfigs,
  createOhMyOpenCodeSlimConfig,
  updateOhMyOpenCodeSlimConfig,
  deleteOhMyOpenCodeSlimConfig,
  applyOhMyOpenCodeSlimConfig,
  toggleOhMyOpenCodeSlimConfigDisabled,
  reorderOhMyOpenCodeSlimConfigs,
  getOhMyOpenCodeSlimGlobalConfig,
  saveOhMyOpenCodeSlimGlobalConfig,
  saveOhMyOpenCodeSlimLocalConfig,
} from '@/services/ohMyOpenCodeSlimApi';
import { openExternalUrl } from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { useRefreshStore } from '@/stores';

const { Text, Link } = Typography;

interface OhMyOpenCodeSlimSettingsProps {
  modelOptions: { label: string; value: string }[];
  /** Map of model ID to its variant keys */
  modelVariantsMap?: Record<string, string[]>;
  disabled?: boolean;
  onConfigApplied?: (config: OhMyOpenCodeSlimConfig) => void;
  onConfigUpdated?: () => void;
}

const OhMyOpenCodeSlimSettings: React.FC<OhMyOpenCodeSlimSettingsProps> = ({
  modelOptions,
  modelVariantsMap = {},
  disabled = false,
  onConfigApplied,
  onConfigUpdated,
}) => {
  const { t } = useTranslation();
  const { omosConfigRefreshKey, incrementOmosConfigRefresh } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [configs, setConfigs] = React.useState<OhMyOpenCodeSlimConfig[]>([]);
  const [modalOpen, setModalOpen] = React.useState(false);
  const [globalModalOpen, setGlobalModalOpen] = React.useState(false);
  const [editingConfig, setEditingConfig] = React.useState<OhMyOpenCodeSlimConfig | null>(null);
  const [globalConfig, setGlobalConfig] = React.useState<OhMyOpenCodeSlimGlobalConfig | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);

  // 配置拖拽传感器
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8, // 防止点击误触
      },
    })
  );

  // Load configs on mount and when refresh key changes
  React.useEffect(() => {
    loadConfigs();
  }, [omosConfigRefreshKey]);

  const loadConfigs = async () => {
    setLoading(true);
    try {
      const data = await listOhMyOpenCodeSlimConfigs();
      setConfigs(data);
    } catch (error) {
      console.error('Failed to load configs:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const handleAddConfig = () => {
    setEditingConfig(null);
    setIsCopyMode(false);
    setModalOpen(true);
  };

  const handleEditConfig = (config: OhMyOpenCodeSlimConfig) => {
    // 深拷贝 config，避免后续 loadConfigs 影响 editingConfig
    setEditingConfig(JSON.parse(JSON.stringify(config)));
    setIsCopyMode(false);
    setModalOpen(true);
  };

  const handleCopyConfig = (config: OhMyOpenCodeSlimConfig) => {
    // 深拷贝 config，避免后续 loadConfigs 影响 editingConfig
    setEditingConfig(JSON.parse(JSON.stringify(config)));
    setIsCopyMode(true);
    setModalOpen(true);
  };

  const handleDeleteConfig = (config: OhMyOpenCodeSlimConfig) => {
    Modal.confirm({
      title: t('common.confirm'),
      content: t('opencode.ohMyOpenCode.confirmDelete', { name: config.name }),
      onOk: async () => {
        try {
          await deleteOhMyOpenCodeSlimConfig(config.id);
          message.success(t('common.success'));
          loadConfigs();
          // 触发其他组件（如 ConfigSelector）刷新
          incrementOmosConfigRefresh();
          // Refresh tray menu after deleting config
          await refreshTrayMenu();
          if (onConfigUpdated) {
            onConfigUpdated();
          }
        } catch {
          message.error(t('common.error'));
        }
      },
    });
  };

  const handleApplyConfig = async (config: OhMyOpenCodeSlimConfig) => {
    try {
      await applyOhMyOpenCodeSlimConfig(config.id);
      message.success(t('opencode.ohMyOpenCode.applySuccess'));
      loadConfigs();
      // 触发其他组件（如 ConfigSelector）刷新
      incrementOmosConfigRefresh();
      // Refresh tray menu after applying config
      await refreshTrayMenu();
      if (onConfigApplied) {
        onConfigApplied(config);
      }
    } catch {
      message.error(t('common.error'));
    }
  };

  const handleToggleDisabled = async (config: OhMyOpenCodeSlimConfig, isDisabled: boolean) => {
    try {
      await toggleOhMyOpenCodeSlimConfigDisabled(config.id, isDisabled);
      message.success(isDisabled ? t('opencode.ohMyOpenCode.configDisabled') : t('opencode.ohMyOpenCode.configEnabled'));
      loadConfigs();
      incrementOmosConfigRefresh();
      await refreshTrayMenu();
    } catch (error) {
      console.error('Failed to toggle config disabled status:', error);
      message.error(t('common.error'));
    }
  };

  // 拖拽结束处理
  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;

    if (!over || active.id === over.id) {
      return;
    }

    const oldIndex = configs.findIndex((c) => c.id === active.id);
    const newIndex = configs.findIndex((c) => c.id === over.id);

    if (oldIndex === -1 || newIndex === -1) {
      return;
    }

    // 乐观更新
    const oldConfigs = [...configs];
    const newConfigs = arrayMove(configs, oldIndex, newIndex);
    setConfigs(newConfigs);

    try {
      await reorderOhMyOpenCodeSlimConfigs(newConfigs.map((c) => c.id));
      await refreshTrayMenu();
    } catch (error) {
      // 失败回滚
      console.error('Failed to reorder configs:', error);
      setConfigs(oldConfigs);
      message.error(t('common.error'));
    }
  };

  const handleModalSuccess = async (values: OhMyOpenCodeSlimConfigFormValues) => {
    try {
      // Check if this is a __local__ config (temporary config from local file)
      const isLocalConfig = editingConfig?.id === '__local__';

      if (isLocalConfig) {
        // Save local config to database using saveOhMyOpenCodeSlimLocalConfig
        await saveOhMyOpenCodeSlimLocalConfig({
          config: {
            name: values.name,
            agents: values.agents,
            otherFields: values.otherFields,
          },
        });
      } else {
        // id 只在编辑时传递，创建时不传递，让后端生成
        const apiInput = {
          id: editingConfig && !isCopyMode ? values.id : undefined,
          name: values.name,
          isApplied: editingConfig?.isApplied, // 保留原有的 isApplied 状态
          agents: values.agents,
          otherFields: values.otherFields,
        };

        if (editingConfig && !isCopyMode) {
          // Update existing config
          await updateOhMyOpenCodeSlimConfig(apiInput);
        } else {
          // Create new config (both copy mode and new config mode)
          // id is undefined, backend will generate it automatically
          await createOhMyOpenCodeSlimConfig(apiInput);
        }
      }
      message.success(t('common.success'));
      setModalOpen(false);
      loadConfigs();
      // 触发其他组件（如 ConfigSelector）刷新
      incrementOmosConfigRefresh();
      // Refresh tray menu after creating/updating config
      await refreshTrayMenu();
      if (onConfigUpdated) {
        onConfigUpdated();
      }
    } catch (error) {
      console.error('Failed to save config:', error);
      message.error(t('common.error'));
    }
  };

  const handleOpenGlobalConfig = async () => {
    try {
      const data = await getOhMyOpenCodeSlimGlobalConfig();
      setGlobalConfig(data);
      setGlobalModalOpen(true);
    } catch (error) {
      console.error('Failed to load global config:', error);
      message.error(t('common.error'));
    }
  };

  const handleSaveGlobalConfig = async (values: OhMyOpenCodeSlimGlobalConfigInput) => {
    try {
      // Check if this is a __local__ config (temporary config from local file)
      const isLocalConfig = globalConfig?.id === '__local__';

      if (isLocalConfig) {
        // Save local config to database using saveOhMyOpenCodeSlimLocalConfig
        await saveOhMyOpenCodeSlimLocalConfig({
          globalConfig: values,
        });
      } else {
        await saveOhMyOpenCodeSlimGlobalConfig(values);
      }
      message.success(t('common.success'));
      setGlobalModalOpen(false);
      // Reload configs to get the new config from database
      if (isLocalConfig) {
        loadConfigs();
        incrementOmosConfigRefresh();
        await refreshTrayMenu();
      }
    } catch (error) {
      console.error('Failed to save global config:', error);
      message.error(t('common.error'));
    }
  };

  const appliedConfig = configs.find((c) => c.isApplied);

  const content = (
    <Spin spinning={loading}>
      {disabled && (
        <Alert
          type="warning"
          showIcon
          message={t('opencode.ohMyOpenCodeSlim.pluginRequiredDesc')}
          style={{ marginBottom: 16 }}
        />
      )}
      {configs.length === 0 ? (
        <Empty
          description={t('opencode.ohMyOpenCodeSlim.emptyText')}
          style={{ margin: '24px 0' }}
        />
      ) : (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          modifiers={[restrictToVerticalAxis]}
          onDragEnd={handleDragEnd}
        >
          <SortableContext
            items={configs.map((c) => c.id)}
            strategy={verticalListSortingStrategy}
          >
            <div>
              {configs.map((config) => (
                <OhMyOpenCodeSlimConfigCard
                  key={config.id}
                  config={config}
                  isSelected={config.isApplied}
                  disabled={disabled}
                  onEdit={handleEditConfig}
                  onCopy={handleCopyConfig}
                  onDelete={handleDeleteConfig}
                  onApply={handleApplyConfig}
                  onToggleDisabled={handleToggleDisabled}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      )}
    </Spin>
  );

  return (
    <>
      <Collapse
        style={{ marginBottom: 16, opacity: disabled ? 0.6 : 1 }}
        defaultActiveKey={disabled ? [] : ['oh-my-opencode-slim']}
        items={[
          {
            key: 'oh-my-opencode-slim',
            label: (
              <Space>
                <Text strong><ThunderboltOutlined style={{ marginRight: 8 }} />{t('opencode.ohMyOpenCodeSlim.title')}</Text>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openExternalUrl('https://github.com/alvinunreal/oh-my-opencode-slim');
                  }}
                >
                  <LinkOutlined /> {t('opencode.ohMyOpenCode.docs')}
                </Link>
                {disabled && (
                  <Tag color="warning" icon={<WarningOutlined />}>
                    {t('opencode.ohMyOpenCodeSlim.pluginRequired')}
                  </Tag>
                )}
                {!disabled && appliedConfig && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {t('opencode.ohMyOpenCode.current')}: {appliedConfig.name}
                  </Text>
                )}
              </Space>
            ),
            extra: (
              <Space>
                <Button
                  type="text"
                  size="small"
                  style={{ fontSize: 12 }}
                  icon={<AppstoreOutlined />}
                  disabled={disabled}
                  onClick={(e) => {
                    e.stopPropagation();
                    handleOpenGlobalConfig();
                  }}
                >
                  {t('opencode.ohMyOpenCode.globalConfig')}
                </Button>
                <Button
                  type="link"
                  size="small"
                  style={{ fontSize: 12 }}
                  icon={<PlusOutlined />}
                  disabled={disabled}
                  onClick={(e) => {
                    e.stopPropagation();
                    handleAddConfig();
                  }}
                >
                  {t('opencode.ohMyOpenCode.addConfig')}
                </Button>
              </Space>
            ),
            children: content,
          },
        ]}
      />

      <OhMyOpenCodeSlimConfigModal
        open={modalOpen}
        isEdit={!isCopyMode && !!editingConfig}
        modelOptions={modelOptions}
        modelVariantsMap={modelVariantsMap}
        initialValues={
          editingConfig
            ? {
                ...editingConfig,
                // 复制模式下移除 id，避免意外使用原配置的 id
                id: isCopyMode ? undefined : editingConfig.id,
                name: isCopyMode ? `${editingConfig.name}_copy` : editingConfig.name,
              }
            : undefined
        }
        onCancel={() => {
          setModalOpen(false);
          setEditingConfig(null);
          setIsCopyMode(false);
        }}
        onSuccess={handleModalSuccess}
      />

      <OhMyOpenCodeSlimGlobalConfigModal
        open={globalModalOpen}
        isLocal={globalConfig?.id === '__local__'}
        initialConfig={globalConfig || undefined}
        onCancel={() => {
          setGlobalModalOpen(false);
          setGlobalConfig(null);
        }}
        onSuccess={handleSaveGlobalConfig}
      />
    </>
  );
};

export default OhMyOpenCodeSlimSettings;
