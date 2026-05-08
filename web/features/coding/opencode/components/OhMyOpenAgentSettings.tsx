import React from 'react';
import { Button, Typography, Collapse, Empty, Spin, Space, message, Modal, Alert, Tag } from 'antd';
import { PlusOutlined, AppstoreOutlined, LinkOutlined, WarningOutlined, ThunderboltOutlined, SyncOutlined, CheckCircleOutlined } from '@ant-design/icons';
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
import type {
  OhMyOpenAgentConfig,
  OhMyOpenAgentGlobalConfig,
  OhMyOpenAgentLegacyUpgradeStatus,
} from '@/types/ohMyOpenAgent';
import OhMyOpenAgentConfigCard from './OhMyOpenAgentConfigCard';
import OhMyOpenAgentConfigModal, { type OhMyOpenAgentConfigFormValues } from './OhMyOpenAgentConfigModal';
import OhMyOpenAgentGlobalConfigModal from './OhMyOpenAgentGlobalConfigModal';
import {
  listOhMyOpenAgentConfigs,
  createOhMyOpenAgentConfig,
  updateOhMyOpenAgentConfig,
  deleteOhMyOpenAgentConfig,
  applyOhMyOpenAgentConfig,
  getOhMyOpenAgentGlobalConfig,
  saveOhMyOpenAgentGlobalConfig,
  saveOhMyOpenAgentLocalConfig,
  toggleOhMyOpenAgentConfigDisabled,
  reorderOhMyOpenAgentConfigs,
  getOhMyOpenAgentUpgradeStatus,
  upgradeOhMyOpenAgentLegacySetup,
  getOhMyOpenAgentConfigPathInfo,
  clearOhMyOpenAgentAppliedConfig,
} from '@/services/ohMyOpenAgentApi';
import { openExternalUrl } from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { useRefreshStore } from '@/stores';

const { Text, Link } = Typography;

interface OhMyOpenAgentSettingsProps {
  modelOptions: Array<
    | { label: string; value: string; disabled?: boolean }
    | { label: string; options: { label: string; value: string; disabled?: boolean }[] }
  >;
  /** Map of model ID to its variant keys */
  modelVariantsMap?: Record<string, string[]>;
  disabled?: boolean;
  allowClearAppliedConfig?: boolean;
  onConfigApplied?: (config: OhMyOpenAgentConfig) => void;
  onConfigUpdated?: () => void; // 新增：配置更新/创建/删除后的回调
  onLegacyUpgraded?: () => void;
}

const OhMyOpenAgentSettings: React.FC<OhMyOpenAgentSettingsProps> = ({
  modelOptions,
  modelVariantsMap = {},
  disabled = false,
  allowClearAppliedConfig = false,
  onConfigApplied,
  onConfigUpdated,
  onLegacyUpgraded,
}) => {
  const { t } = useTranslation();
  const { omoConfigRefreshKey, incrementOmoConfigRefresh } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [configs, setConfigs] = React.useState<OhMyOpenAgentConfig[]>([]);
  const [modalOpen, setModalOpen] = React.useState(false);
  const [globalModalOpen, setGlobalModalOpen] = React.useState(false);
  const [editingConfig, setEditingConfig] = React.useState<OhMyOpenAgentConfig | null>(null);
  const [globalConfig, setGlobalConfig] = React.useState<OhMyOpenAgentGlobalConfig | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);
  const [upgradeStatus, setUpgradeStatus] = React.useState<OhMyOpenAgentLegacyUpgradeStatus | null>(null);
  const [upgradeLoading, setUpgradeLoading] = React.useState(false);

  // 配置拖拽传感器
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8, // 防止点击误触
      },
    })
  );

  // Load configs on mount and when refresh key changes
  const loadConfigs = React.useCallback(async () => {
    setLoading(true);
    try {
      const data = await listOhMyOpenAgentConfigs();
      setConfigs(data);
    } catch (error) {
      console.error('Failed to load configs:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  const loadUpgradeStatus = React.useCallback(async () => {
    try {
      const status = await getOhMyOpenAgentUpgradeStatus();
      setUpgradeStatus(status);
    } catch (error) {
      console.error('Failed to load Oh My OpenAgent upgrade status:', error);
      setUpgradeStatus(null);
    }
  }, []);

  React.useEffect(() => {
    // Biome exhaustive-deps: explicitly reference the refresh key
    // so the dependency array is considered necessary.
    void omoConfigRefreshKey;
    void loadConfigs();
    void loadUpgradeStatus();
  }, [omoConfigRefreshKey, loadConfigs, loadUpgradeStatus]);

  const handleAddConfig = () => {
    setEditingConfig(null);
    setIsCopyMode(false);
    setModalOpen(true);
  };

  const handleEditConfig = (config: OhMyOpenAgentConfig) => {
    // 深拷贝 config，避免后续 loadConfigs 影响 editingConfig
    setEditingConfig(JSON.parse(JSON.stringify(config)));
    setIsCopyMode(false);
    setModalOpen(true);
  };

  const handleCopyConfig = (config: OhMyOpenAgentConfig) => {
    // 深拷贝 config，避免后续 loadConfigs 影响 editingConfig
    setEditingConfig(JSON.parse(JSON.stringify(config)));
    setIsCopyMode(true);
    setModalOpen(true);
  };

  const handleDeleteConfig = (config: OhMyOpenAgentConfig) => {
    Modal.confirm({
      title: t('common.confirm'),
      content: t('opencode.ohMyOpenCode.confirmDelete', { name: config.name }),
      onOk: async () => {
        try {
          await deleteOhMyOpenAgentConfig(config.id);
          message.success(t('common.success'));
          loadConfigs();
          // 触发其他组件（如 ConfigSelector）刷新
          incrementOmoConfigRefresh();
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

  const handleApplyConfig = async (config: OhMyOpenAgentConfig) => {
    try {
      await applyOhMyOpenAgentConfig(config.id);
      message.success(t('opencode.ohMyOpenCode.applySuccess'));
      loadConfigs();
      // 触发其他组件（如 ConfigSelector）刷新
      incrementOmoConfigRefresh();
      // Refresh tray menu after applying config
      await refreshTrayMenu();
      if (onConfigApplied) {
        onConfigApplied(config);
      }
    } catch {
      message.error(t('common.error'));
    }
  };

  const handleClearAppliedConfig = async (config: OhMyOpenAgentConfig) => {
    let configPath = '';
    try {
      const pathInfo = await getOhMyOpenAgentConfigPathInfo();
      configPath = pathInfo.path;
    } catch (error) {
      console.error('Failed to get Oh My OpenAgent config path:', error);
    }

    Modal.confirm({
      title: t('opencode.ohMyOpenCode.clearAppliedConfirmTitle'),
      content: t('opencode.ohMyOpenCode.clearAppliedConfirmContent', {
        name: config.name,
        path: configPath || t('common.unknown'),
      }),
      okText: t('opencode.ohMyOpenCode.clearAppliedConfirmOk'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await clearOhMyOpenAgentAppliedConfig(config.id);
          message.success(t('opencode.ohMyOpenCode.clearAppliedSuccess'));
          loadConfigs();
          incrementOmoConfigRefresh();
          await refreshTrayMenu();
          if (onConfigUpdated) {
            onConfigUpdated();
          }
        } catch (error) {
          console.error('Failed to clear Oh My OpenAgent applied config:', error);
          message.error(t('opencode.ohMyOpenCode.clearAppliedError'));
        }
      },
    });
  };

  const handleToggleDisabled = async (config: OhMyOpenAgentConfig, isDisabled: boolean) => {
    try {
      await toggleOhMyOpenAgentConfigDisabled(config.id, isDisabled);
      message.success(isDisabled ? t('opencode.ohMyOpenCode.configDisabled') : t('opencode.ohMyOpenCode.configEnabled'));
      loadConfigs();
      incrementOmoConfigRefresh();
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
      await reorderOhMyOpenAgentConfigs(newConfigs.map((c) => c.id));
      await refreshTrayMenu();
    } catch (error) {
      // 失败回滚
      console.error('Failed to reorder configs:', error);
      setConfigs(oldConfigs);
      message.error(t('common.error'));
    }
  };

  const handleModalSuccess = async (values: OhMyOpenAgentConfigFormValues) => {
    try {
      // Convert agents to the expected API format (filter out undefined values)
      const agentsForApi: Record<string, Record<string, unknown>> = {};
      if (values.agents) {
        Object.entries(values.agents).forEach(([key, value]) => {
          if (value !== undefined) {
            agentsForApi[key] = value as Record<string, unknown>;
          }
        });
      }

      // Convert categories to the expected API format (filter out undefined values)
      const categoriesForApi: Record<string, Record<string, unknown>> = {};
      if (values.categories) {
        Object.entries(values.categories).forEach(([key, value]) => {
          if (value !== undefined) {
            categoriesForApi[key] = value as Record<string, unknown>;
          }
        });
      }

      // Check if this is a __local__ config (temporary config from local file)
      const isLocalConfig = editingConfig?.id === '__local__';

      if (isLocalConfig) {
        // Save local config to database using saveOhMyOpenAgentLocalConfig
        await saveOhMyOpenAgentLocalConfig({
          config: {
            name: values.name,
            agents: Object.keys(agentsForApi).length > 0 ? agentsForApi : null,
            categories: Object.keys(categoriesForApi).length > 0 ? categoriesForApi : null,
            otherFields: values.otherFields,
          },
        });
      } else {
        // id 只在编辑时传递，创建时不传递，让后端生成
        const apiInput = {
          id: editingConfig && !isCopyMode ? values.id : undefined,
          name: values.name,
          isApplied: editingConfig?.isApplied, // 保留原有的 isApplied 状态
          agents: Object.keys(agentsForApi).length > 0 ? agentsForApi : null,
          categories: Object.keys(categoriesForApi).length > 0 ? categoriesForApi : null,
          otherFields: values.otherFields,
        };

        if (editingConfig && !isCopyMode) {
          // Update existing config
          await updateOhMyOpenAgentConfig(apiInput);
        } else {
          // Create new config (both copy mode and new config mode)
          // id is undefined, backend will generate it automatically
          await createOhMyOpenAgentConfig(apiInput);
        }
      }
      message.success(t('common.success'));
      setModalOpen(false);
      loadConfigs();
      // 触发其他组件（如 ConfigSelector）刷新
      incrementOmoConfigRefresh();
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
      const data = await getOhMyOpenAgentGlobalConfig();
      setGlobalConfig(data);
      setGlobalModalOpen(true);
    } catch (error) {
      console.error('Failed to load global config:', error);
      message.error(t('common.error'));
    }
  };

  const handleUpgradeLegacySetup = async () => {
    setUpgradeLoading(true);
    try {
      const result = await upgradeOhMyOpenAgentLegacySetup();
      if (result.changed) {
        message.success(t('opencode.ohMyOpenCode.upgradeSuccess'));
      } else {
        message.success(t('opencode.ohMyOpenCode.upgradeAlreadyCurrent'));
      }
      await Promise.all([
        loadConfigs(),
        loadUpgradeStatus(),
        refreshTrayMenu(),
      ]);
      incrementOmoConfigRefresh();
      if (onConfigUpdated) {
        onConfigUpdated();
      }
      if (onLegacyUpgraded) {
        onLegacyUpgraded();
      }
    } catch (error) {
      console.error('Failed to upgrade legacy Oh My OpenAgent setup:', error);
      message.error(t('opencode.ohMyOpenCode.upgradeFailed'));
    } finally {
      setUpgradeLoading(false);
    }
  };

  const upgradeTags = React.useMemo(() => {
    if (!upgradeStatus?.needsUpgrade) {
      return [];
    }

    const tags: string[] = [];
    if (upgradeStatus.hasLegacyPlugin) {
      tags.push(t('opencode.ohMyOpenCode.upgradeTagPlugin'));
    }
    if (upgradeStatus.hasLegacyLocalConfig || upgradeStatus.hasLegacyCustomConfigPath) {
      tags.push(t('opencode.ohMyOpenCode.upgradeTagLocalFile'));
    }
    if (upgradeStatus.hasLegacyWslMapping) {
      tags.push(t('opencode.ohMyOpenCode.upgradeTagWsl'));
    }
    if (upgradeStatus.hasLegacySshMapping) {
      tags.push(t('opencode.ohMyOpenCode.upgradeTagSsh'));
    }
    return tags;
  }, [t, upgradeStatus]);

  const handleSaveGlobalConfig = async (values: {
    schema: string;
    sisyphusAgent: Record<string, unknown> | null;
    disabledAgents: string[];
    disabledMcps: string[];
    disabledHooks: string[];
    disabledSkills?: string[];
    lsp?: Record<string, unknown> | null;
    experimental?: Record<string, unknown> | null;
    backgroundTask?: Record<string, unknown> | null;
    browserAutomationEngine?: Record<string, unknown> | null;
    claudeCode?: Record<string, unknown> | null;
    otherFields?: Record<string, unknown>;
  }) => {
    try {
      // Check if this is a __local__ config (temporary config from local file)
      const isLocalConfig = globalConfig?.id === '__local__';

      if (isLocalConfig) {
        // Save local config to database using saveOhMyOpenAgentLocalConfig
        await saveOhMyOpenAgentLocalConfig({
          globalConfig: values,
        });
      } else {
        await saveOhMyOpenAgentGlobalConfig(values);
      }
      message.success(t('common.success'));
      setGlobalModalOpen(false);
      // Reload configs to get the new config from database
      if (isLocalConfig) {
        loadConfigs();
        incrementOmoConfigRefresh();
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
          message={t('opencode.ohMyOpenCode.pluginRequiredDesc')}
          style={{ marginBottom: 16 }}
        />
      )}
      {!disabled && (
        <div style={{ fontSize: 12, color: 'rgba(0,0,0,0.45)', borderLeft: '2px solid rgba(0,0,0,0.12)', paddingLeft: 8, marginBottom: 12 }}>
          <div>{t('opencode.ohMyOpenCode.sectionHint')}</div>
          <div>{t('opencode.ohMyOpenCode.sectionWarning')}</div>
        </div>
      )}
      {upgradeStatus?.needsUpgrade && (
        <Alert
          type="info"
          showIcon
          icon={<SyncOutlined />}
          style={{ marginBottom: 16, borderRadius: 10 }}
          message={t('opencode.ohMyOpenCode.upgradeBannerTitle')}
          description={(
            <div style={{ marginTop: 8 }}>
              <div style={{ marginBottom: 12 }}>
                {t('opencode.ohMyOpenCode.upgradeBannerDesc')}
              </div>
              <Space wrap size={[8, 8]} style={{ marginBottom: 12 }}>
                {upgradeTags.map((label) => (
                  <Tag key={label} color="processing" icon={<CheckCircleOutlined />}>
                    {label}
                  </Tag>
                ))}
              </Space>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t('opencode.ohMyOpenCode.upgradeBannerHint')}
                </Text>
                <Button
                  type="primary"
                  icon={<SyncOutlined />}
                  loading={upgradeLoading}
                  onClick={handleUpgradeLegacySetup}
                >
                  {t('opencode.ohMyOpenCode.upgradeAction')}
                </Button>
              </div>
            </div>
          )}
        />
      )}
      {configs.length === 0 ? (
        <Empty
          description={t('opencode.ohMyOpenCode.emptyText')}
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
                <OhMyOpenAgentConfigCard
                  key={config.id}
                  config={config}
                  isSelected={config.isApplied}
                  disabled={disabled}
                  onEdit={handleEditConfig}
                  onCopy={handleCopyConfig}
                  onDelete={handleDeleteConfig}
                  onApply={handleApplyConfig}
                  onToggleDisabled={handleToggleDisabled}
                  allowClearAppliedConfig={allowClearAppliedConfig}
                  onClearAppliedConfig={handleClearAppliedConfig}
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
        defaultActiveKey={disabled ? [] : ['oh-my-openagent']}
        items={[
          {
            key: 'oh-my-openagent',
            label: (
              <Space>
                <Text strong><ThunderboltOutlined style={{ marginRight: 8 }} />{t('opencode.ohMyOpenCode.title')}</Text>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openExternalUrl('https://github.com/code-yeongyu/oh-my-openagent/blob/dev/README.zh-cn.md');
                  }}
                >
                  <LinkOutlined /> {t('opencode.ohMyOpenCode.docs')}
                </Link>
                {disabled && (
                  <Tag color="warning" icon={<WarningOutlined />}>
                    {t('opencode.ohMyOpenCode.pluginRequired')}
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

      <OhMyOpenAgentConfigModal
        open={modalOpen}
        isEdit={!isCopyMode && !!editingConfig}
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
        modelOptions={modelOptions}
        modelVariantsMap={modelVariantsMap}
        onCancel={() => {
          setModalOpen(false);
          setEditingConfig(null);
          setIsCopyMode(false);
        }}
        onSuccess={handleModalSuccess}
      />

      <OhMyOpenAgentGlobalConfigModal
        open={globalModalOpen}
        isLocal={globalConfig?.id === '__local__'}
        initialValues={globalConfig || undefined}
        onCancel={() => {
          setGlobalModalOpen(false);
          setGlobalConfig(null);
        }}
        onSuccess={handleSaveGlobalConfig}
      />
    </>
  );
};

export default OhMyOpenAgentSettings;
