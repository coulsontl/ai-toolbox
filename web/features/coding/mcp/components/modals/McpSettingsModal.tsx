import React from 'react';
import { Modal, Button, Checkbox, Tag, message, Form, Input, Space, Tooltip, Switch, Radio } from 'antd';
import { DeleteOutlined, PlusOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { McpTool } from '../../types';
import * as mcpApi from '../../services/mcpApi';
import { useMcpStore } from '../../stores/mcpStore';
import { refreshTrayMenu } from '@/services/appApi';
import styles from './McpSettingsModal.module.less';

interface McpSettingsModalProps {
  open: boolean;
  onClose: () => void;
}

interface CustomMcpTool {
  key: string;
  display_name: string;
  mcp_config_path: string | null;
  mcp_config_format: string | null;
  mcp_field: string | null;
}

export const McpSettingsModal: React.FC<McpSettingsModalProps> = ({
  open: isOpen,
  onClose,
}) => {
  const { t } = useTranslation();
  const { fetchTools } = useMcpStore();
  const [form] = Form.useForm();
  const [loading, setLoading] = React.useState(false);
  const [allTools, setAllTools] = React.useState<McpTool[]>([]);
  const [preferredTools, setPreferredTools] = React.useState<string[]>([]);
  const [customTools, setCustomTools] = React.useState<CustomMcpTool[]>([]);
  const [showAddCustomModal, setShowAddCustomModal] = React.useState(false);
  const [addingTool, setAddingTool] = React.useState(false);
  const [showInTray, setShowInTray] = React.useState(false);

  // Load settings on open
  React.useEffect(() => {
    if (isOpen) {
      loadData();
    }
  }, [isOpen]);

  const loadData = async () => {
    try {
      const [tools, trayEnabled, savedPreferredTools] = await Promise.all([
        mcpApi.getMcpTools(),
        mcpApi.getMcpShowInTray(),
        mcpApi.getMcpPreferredTools(),
      ]);

      // Sort: installed tools first
      const sorted = [...tools].sort((a, b) => {
        if (a.installed === b.installed) return 0;
        return a.installed ? -1 : 1;
      });
      setAllTools(sorted);
      setShowInTray(trayEnabled);

      // Extract custom tools
      const custom = tools.filter((t) => t.is_custom && t.supports_mcp);
      setCustomTools(
        custom.map((t) => ({
          key: t.key,
          display_name: t.display_name,
          mcp_config_path: t.mcp_config_path,
          mcp_config_format: t.mcp_config_format,
          mcp_field: t.mcp_field,
        }))
      );

      // Use saved preferred tools if available, otherwise default to installed tools
      if (savedPreferredTools.length > 0) {
        setPreferredTools(savedPreferredTools);
      } else {
        setPreferredTools(tools.filter((t) => t.installed && t.supports_mcp).map((t) => t.key));
      }
    } catch (error) {
      console.error('Failed to load settings:', error);
    }
  };

  const handleToolToggle = (toolKey: string, checked: boolean) => {
    setPreferredTools((prev) =>
      checked ? [...prev, toolKey] : prev.filter((k) => k !== toolKey)
    );
  };

  const handleShowInTrayChange = async (checked: boolean) => {
    setShowInTray(checked);
    try {
      await mcpApi.setMcpShowInTray(checked);
      await refreshTrayMenu();
    } catch (error) {
      message.error(String(error));
      setShowInTray(!checked); // Revert on error
    }
  };

  // Sort tools: installed built-in > custom tools > not installed built-in
  const sortedTools = React.useMemo(() => {
    const customKeys = new Set(customTools.map((c) => c.key));
    const installedBuiltin = allTools.filter((t) => t.installed && !customKeys.has(t.key) && t.supports_mcp);
    const customToolItems = allTools.filter((t) => customKeys.has(t.key) && t.supports_mcp);
    const notInstalledBuiltin = allTools.filter((t) => !t.installed && !customKeys.has(t.key) && t.supports_mcp);
    return [...installedBuiltin, ...customToolItems, ...notInstalledBuiltin];
  }, [allTools, customTools]);

  const handleSave = async () => {
    setLoading(true);
    try {
      // Save preferred tools
      await mcpApi.setMcpPreferredTools(preferredTools);
      await fetchTools(); // Refresh global store
      message.success(t('common.success'));
      onClose();
    } catch (error) {
      message.error(String(error));
    } finally {
      setLoading(false);
    }
  };

  const handleAddCustomTool = async (values: {
    key: string;
    displayName: string;
    mcpConfigPath: string;
    mcpConfigFormat: 'json' | 'toml';
    mcpField: string;
  }) => {
    setAddingTool(true);
    try {
      await mcpApi.addMcpCustomTool({
        key: values.key,
        displayName: values.displayName,
        mcpConfigPath: values.mcpConfigPath,
        mcpConfigFormat: values.mcpConfigFormat,
        mcpField: values.mcpField,
      });
      message.success(t('common.success'));
      form.resetFields();
      setShowAddCustomModal(false);
      await loadData();
      await fetchTools();
    } catch (error) {
      message.error(String(error));
    } finally {
      setAddingTool(false);
    }
  };

  const handleRemoveCustomTool = async (key: string) => {
    try {
      await mcpApi.removeMcpCustomTool(key);
      message.success(t('common.success'));
      await loadData();
      await fetchTools();
    } catch (error) {
      message.error(String(error));
    }
  };

  return (
    <Modal
      title={t('mcp.settings')}
      open={isOpen}
      onCancel={onClose}
      footer={null}
      width={700}
      destroyOnClose
    >
      <div className={styles.section}>
        <div className={styles.labelArea}>
          <label className={styles.label}>{t('mcp.showInTray')}</label>
        </div>
        <div className={styles.inputArea}>
          <Switch checked={showInTray} onChange={handleShowInTrayChange} />
          <p className={styles.hint}>{t('mcp.showInTrayHint')}</p>
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.labelArea}>
          <label className={styles.label}>{t('mcp.preferredTools')}</label>
        </div>
        <div className={styles.inputArea}>
          <div className={styles.toolList}>
            {sortedTools.map((tool) => {
              const isCustomTool = customTools.some((c) => c.key === tool.key);
              const isDisabled = !tool.installed && !isCustomTool;
              return (
                <div key={tool.key} className={styles.toolItem}>
                  <Tooltip title={tool.mcp_config_path || ''}>
                    <Checkbox
                      checked={preferredTools.includes(tool.key)}
                      onChange={(e) => handleToolToggle(tool.key, e.target.checked)}
                      disabled={isDisabled}
                    >
                      {tool.display_name}
                    </Checkbox>
                  </Tooltip>
                  {isDisabled && <Tag color="default">{t('mcp.notInstalled')}</Tag>}
                  {isCustomTool && (
                    <Button
                      type="text"
                      size="small"
                      icon={<DeleteOutlined />}
                      danger
                      onClick={() => handleRemoveCustomTool(tool.key)}
                    />
                  )}
                </div>
              );
            })}
            <Button
              type="dashed"
              size="small"
              icon={<PlusOutlined />}
              onClick={() => setShowAddCustomModal(true)}
            >
              {t('mcp.customToolSettings.add')}
            </Button>
          </div>
          <p className={styles.hint}>{t('mcp.preferredToolsHint')}</p>
        </div>
      </div>

      <div className={styles.footer}>
        <Button onClick={onClose}>{t('common.cancel')}</Button>
        <Button type="primary" onClick={handleSave} loading={loading}>
          {t('common.save')}
        </Button>
      </div>

      <Modal
        title={t('mcp.customToolSettings.addTitle')}
        open={showAddCustomModal}
        onCancel={() => setShowAddCustomModal(false)}
        footer={null}
        destroyOnClose
      >
        <Form form={form} layout="vertical" onFinish={handleAddCustomTool}>
          <Form.Item
            name="key"
            label={t('mcp.customToolSettings.key')}
            rules={[
              { required: true, message: t('mcp.customToolSettings.keyRequired') },
              { pattern: /^[a-z][a-z0-9_]*$/, message: t('mcp.customToolSettings.keyHint') },
            ]}
          >
            <Input placeholder="my_tool" />
          </Form.Item>
          <Form.Item
            name="displayName"
            label={t('mcp.customToolSettings.displayName')}
            rules={[{ required: true, message: t('mcp.customToolSettings.displayNameRequired') }]}
          >
            <Input placeholder="My Tool" />
          </Form.Item>
          <Form.Item
            name="mcpConfigPath"
            label={t('mcp.customToolSettings.configPath')}
            rules={[{ required: true, message: t('mcp.customToolSettings.configPathRequired') }]}
          >
            <Input placeholder="~/.mytool/mcp.json" />
          </Form.Item>
          <Form.Item
            name="mcpConfigFormat"
            label={t('mcp.customToolSettings.configFormat')}
            rules={[{ required: true }]}
            initialValue="json"
          >
            <Radio.Group
              options={[
                { label: 'JSON', value: 'json' },
                { label: 'TOML', value: 'toml' },
              ]}
            />
          </Form.Item>
          <Form.Item
            name="mcpField"
            label={t('mcp.customToolSettings.configField')}
            rules={[{ required: true, message: t('mcp.customToolSettings.configFieldRequired') }]}
          >
            <Input placeholder="mcpServers" />
          </Form.Item>
          <div style={{ textAlign: 'right' }}>
            <Space>
              <Button onClick={() => setShowAddCustomModal(false)}>{t('common.cancel')}</Button>
              <Button type="primary" htmlType="submit" loading={addingTool}>
                {t('common.add')}
              </Button>
            </Space>
          </div>
        </Form>
      </Modal>
    </Modal>
  );
};
