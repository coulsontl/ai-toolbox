import React from 'react';
import { Button, Typography, Collapse, Empty, Spin, Space, message, Modal, Alert, Tag } from 'antd';
import { PlusOutlined, SettingOutlined, LinkOutlined, WarningOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { OhMyOpenCodeConfig, OhMyOpenCodeGlobalConfig } from '@/types/ohMyOpenCode';
import OhMyOpenCodeConfigCard from './OhMyOpenCodeConfigCard';
import OhMyOpenCodeConfigModal, { OhMyOpenCodeConfigFormValues } from './OhMyOpenCodeConfigModal';
import OhMyOpenCodeGlobalConfigModal from './OhMyOpenCodeGlobalConfigModal';
import {
  listOhMyOpenCodeConfigs,
  createOhMyOpenCodeConfig,
  updateOhMyOpenCodeConfig,
  deleteOhMyOpenCodeConfig,
  applyOhMyOpenCodeConfig,
  getOhMyOpenCodeGlobalConfig,
  saveOhMyOpenCodeGlobalConfig,
} from '@/services/ohMyOpenCodeApi';
import { openExternalUrl } from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { useRefreshStore } from '@/stores';

const { Text, Link } = Typography;

interface OhMyOpenCodeSettingsProps {
  modelOptions: { label: string; value: string }[];
  disabled?: boolean;
  onConfigApplied?: (config: OhMyOpenCodeConfig) => void;
  onConfigUpdated?: () => void; // 新增：配置更新/创建/删除后的回调
}

const OhMyOpenCodeSettings: React.FC<OhMyOpenCodeSettingsProps> = ({
  modelOptions,
  disabled = false,
  onConfigApplied,
  onConfigUpdated,
}) => {
  const { t } = useTranslation();
  const { omoConfigRefreshKey } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [configs, setConfigs] = React.useState<OhMyOpenCodeConfig[]>([]);
  const [modalOpen, setModalOpen] = React.useState(false);
  const [globalModalOpen, setGlobalModalOpen] = React.useState(false);
  const [editingConfig, setEditingConfig] = React.useState<OhMyOpenCodeConfig | null>(null);
  const [globalConfig, setGlobalConfig] = React.useState<OhMyOpenCodeGlobalConfig | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);

  // Load configs on mount and when refresh key changes
  React.useEffect(() => {
    loadConfigs();
  }, [omoConfigRefreshKey]);

  const loadConfigs = async () => {
    setLoading(true);
    try {
      const data = await listOhMyOpenCodeConfigs();
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

  const handleEditConfig = (config: OhMyOpenCodeConfig) => {
    // 深拷贝 config，避免后续 loadConfigs 影响 editingConfig
    setEditingConfig(JSON.parse(JSON.stringify(config)));
    setIsCopyMode(false);
    setModalOpen(true);
  };

  const handleCopyConfig = (config: OhMyOpenCodeConfig) => {
    // 深拷贝 config，避免后续 loadConfigs 影响 editingConfig
    setEditingConfig(JSON.parse(JSON.stringify(config)));
    setIsCopyMode(true);
    setModalOpen(true);
  };

  const handleDeleteConfig = (config: OhMyOpenCodeConfig) => {
    Modal.confirm({
      title: t('common.confirm'),
      content: t('opencode.ohMyOpenCode.confirmDelete', { name: config.name }),
      onOk: async () => {
        try {
          await deleteOhMyOpenCodeConfig(config.id);
          message.success(t('common.success'));
          loadConfigs();
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

  const handleApplyConfig = async (config: OhMyOpenCodeConfig) => {
    try {
      await applyOhMyOpenCodeConfig(config.id);
      message.success(t('opencode.ohMyOpenCode.applySuccess'));
      loadConfigs();
      // Refresh tray menu after applying config
      await refreshTrayMenu();
      if (onConfigApplied) {
        onConfigApplied(config);
      }
    } catch {
      message.error(t('common.error'));
    }
  };

  const handleModalSuccess = async (values: OhMyOpenCodeConfigFormValues) => {
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

      // id 只在编辑时传递，创建时不传递，让后端生成
      const apiInput = {
        id: editingConfig && !isCopyMode ? values.id : undefined,
        name: values.name,
        isApplied: editingConfig?.isApplied, // 保留原有的 isApplied 状态
        agents: Object.keys(agentsForApi).length > 0 ? agentsForApi : null,
        otherFields: values.otherFields,
      };

      if (editingConfig && !isCopyMode) {
        // Update existing config
        await updateOhMyOpenCodeConfig(apiInput);
      } else {
        // Create new config (both copy mode and new config mode)
        // id is undefined, backend will generate it automatically
        await createOhMyOpenCodeConfig(apiInput);
      }
      message.success(t('common.success'));
      setModalOpen(false);
      loadConfigs();
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
      const data = await getOhMyOpenCodeGlobalConfig();
      setGlobalConfig(data);
      setGlobalModalOpen(true);
    } catch (error) {
      console.error('Failed to load global config:', error);
      message.error(t('common.error'));
    }
  };

  const handleSaveGlobalConfig = async (values: {
    schema: string;
    sisyphusAgent: Record<string, unknown> | null;
    disabledAgents: string[];
    disabledMcps: string[];
    disabledHooks: string[];
    lsp?: Record<string, unknown> | null;
    experimental?: Record<string, unknown> | null;
    otherFields?: Record<string, unknown>;
  }) => {
    try {
      await saveOhMyOpenCodeGlobalConfig(values);
      message.success(t('common.success'));
      setGlobalModalOpen(false);
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
      {configs.length === 0 ? (
        <Empty
          description={t('opencode.ohMyOpenCode.emptyText')}
          style={{ margin: '24px 0' }}
        />
      ) : (
        <div>
          {configs.map((config) => (
            <OhMyOpenCodeConfigCard
              key={config.id}
              config={config}
              isSelected={config.isApplied}
              disabled={disabled}
              onEdit={handleEditConfig}
              onCopy={handleCopyConfig}
              onDelete={handleDeleteConfig}
              onApply={handleApplyConfig}
            />
          ))}
        </div>
      )}
    </Spin>
  );

  return (
    <>
      <Collapse
        style={{ marginBottom: 16, opacity: disabled ? 0.6 : 1 }}
        defaultActiveKey={disabled ? [] : ['oh-my-opencode']}
        items={[
          {
            key: 'oh-my-opencode',
            label: (
              <Space>
                <Text strong>{t('opencode.ohMyOpenCode.title')}</Text>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openExternalUrl('https://github.com/code-yeongyu/oh-my-opencode/blob/dev/README.zh-cn.md');
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
                  size="small"
                  style={{ fontSize: 12 }}
                  icon={<SettingOutlined />}
                  disabled={disabled}
                  onClick={(e) => {
                    e.stopPropagation();
                    handleOpenGlobalConfig();
                  }}
                >
                  {t('opencode.ohMyOpenCode.globalConfig')}
                </Button>
                <Button
                  type="primary"
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

      <OhMyOpenCodeConfigModal
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
        onCancel={() => {
          setModalOpen(false);
          setEditingConfig(null);
          setIsCopyMode(false);
        }}
        onSuccess={handleModalSuccess}
      />

      <OhMyOpenCodeGlobalConfigModal
        open={globalModalOpen}
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

export default OhMyOpenCodeSettings;
