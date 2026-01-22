import React from 'react';
import { Button, Typography, Collapse, Empty, Spin, Space, message, Modal, Alert, Tag } from 'antd';
import { PlusOutlined, LinkOutlined, WarningOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { OhMyOpenCodeSlimConfig } from '@/types/ohMyOpenCodeSlim';
import OhMyOpenCodeSlimConfigCard from './OhMyOpenCodeSlimConfigCard';
import OhMyOpenCodeSlimConfigModal, { OhMyOpenCodeSlimConfigFormValues } from './OhMyOpenCodeSlimConfigModal';
import {
  listOhMyOpenCodeSlimConfigs,
  createOhMyOpenCodeSlimConfig,
  updateOhMyOpenCodeSlimConfig,
  deleteOhMyOpenCodeSlimConfig,
  applyOhMyOpenCodeSlimConfig,
} from '@/services/ohMyOpenCodeSlimApi';
import { openExternalUrl } from '@/services';
import { refreshTrayMenu } from '@/services/appApi';
import { useRefreshStore } from '@/stores';

const { Text, Link } = Typography;

interface OhMyOpenCodeSlimSettingsProps {
  disabled?: boolean;
  onConfigApplied?: (config: OhMyOpenCodeSlimConfig) => void;
  onConfigUpdated?: () => void;
}

const OhMyOpenCodeSlimSettings: React.FC<OhMyOpenCodeSlimSettingsProps> = ({
  disabled = false,
  onConfigApplied,
  onConfigUpdated,
}) => {
  const { t } = useTranslation();
  const { omoConfigRefreshKey } = useRefreshStore();
  const [loading, setLoading] = React.useState(false);
  const [configs, setConfigs] = React.useState<OhMyOpenCodeSlimConfig[]>([]);
  const [modalOpen, setModalOpen] = React.useState(false);
  const [editingConfig, setEditingConfig] = React.useState<OhMyOpenCodeSlimConfig | null>(null);
  const [isCopyMode, setIsCopyMode] = React.useState(false);

  // Load configs on mount and when refresh key changes
  React.useEffect(() => {
    loadConfigs();
  }, [omoConfigRefreshKey]);

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
      // Refresh tray menu after applying config
      await refreshTrayMenu();
      if (onConfigApplied) {
        onConfigApplied(config);
      }
    } catch {
      message.error(t('common.error'));
    }
  };

  const handleModalSuccess = async (values: OhMyOpenCodeSlimConfigFormValues) => {
    try {
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
        defaultActiveKey={disabled ? [] : ['oh-my-opencode-slim']}
        items={[
          {
            key: 'oh-my-opencode-slim',
            label: (
              <Space>
                <Text strong>{t('opencode.ohMyOpenCodeSlim.title')}</Text>
                <Link
                  type="secondary"
                  style={{ fontSize: 12 }}
                  onClick={(e) => {
                    e.stopPropagation();
                    openExternalUrl('https://github.com/code-yeongyu/oh-my-opencode-slim');
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
            ),
            children: content,
          },
        ]}
      />

      <OhMyOpenCodeSlimConfigModal
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
        onCancel={() => {
          setModalOpen(false);
          setEditingConfig(null);
          setIsCopyMode(false);
        }}
        onSuccess={handleModalSuccess}
      />
    </>
  );
};

export default OhMyOpenCodeSlimSettings;
