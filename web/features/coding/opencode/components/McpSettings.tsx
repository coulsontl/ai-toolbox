import React from 'react';
import { Button, Space, Empty, Typography, Collapse, List, Modal, Form, Input, Popconfirm } from 'antd';
import { PlusOutlined, EditOutlined, DeleteOutlined, CopyOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import type { McpServerConfig } from '@/types/opencode';

const { Text } = Typography;

interface McpSettingsProps {
  mcp: Record<string, McpServerConfig>;
  onChange: (mcp: Record<string, McpServerConfig>) => void;
  defaultCollapsed?: boolean;
}

const McpSettings: React.FC<McpSettingsProps> = ({ mcp, onChange, defaultCollapsed = true }) => {
  const { t } = useTranslation();
  const [collapsed, setCollapsed] = React.useState(defaultCollapsed);
  const [modalOpen, setModalOpen] = React.useState(false);
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [formId, setFormId] = React.useState('');
  const [formConfig, setFormConfig] = React.useState<McpServerConfig>({
    type: 'local',
    command: ['npx', '-y', 'your-mcp-package'],
    enabled: true,
  });
  const [jsonValid, setJsonValid] = React.useState(true);
  const [idError, setIdError] = React.useState<string | null>(null);

  const mcpEntries = Object.entries(mcp);

  const handleAdd = () => {
    setEditingId(null);
    setFormId('');
    setFormConfig(undefined as unknown as McpServerConfig);
    setJsonValid(true);
    setIdError(null);
    setModalOpen(true);
    setCollapsed(false);
  };

  const handleEdit = (id: string) => {
    const config = mcp[id];
    if (!config) return;

    setEditingId(id);
    setFormId(id);
    setFormConfig(config);
    setJsonValid(true);
    setIdError(null);
    setModalOpen(true);
  };

  const handleDelete = (id: string) => {
    const newMcp = { ...mcp };
    delete newMcp[id];
    onChange(newMcp);
  };

  const handleCopy = (id: string) => {
    const config = mcp[id];
    if (!config) return;

    setEditingId(null);
    setFormId(`${id}_copy`);
    setFormConfig({ ...config });
    setJsonValid(true);
    setIdError(null);
    setModalOpen(true);
    setCollapsed(false);
  };

  const handleModalOk = () => {
    // Validate ID
    const trimmedId = formId.trim();
    if (!trimmedId) {
      setIdError(t('opencode.mcp.idRequired'));
      return;
    }

    // Check for duplicate ID (only when adding new or changing ID)
    if (trimmedId !== editingId && mcp[trimmedId]) {
      setIdError(t('opencode.mcp.idExists'));
      return;
    }

    if (!jsonValid || !formConfig) {
      return;
    }

    const newMcp = { ...mcp };

    // If editing and ID changed, remove old entry
    if (editingId && editingId !== trimmedId) {
      delete newMcp[editingId];
    }

    newMcp[trimmedId] = formConfig;
    onChange(newMcp);
    setModalOpen(false);
  };

  const handleJsonChange = (value: unknown, isValid: boolean) => {
    setJsonValid(isValid);
    if (isValid && typeof value === 'object' && value !== null) {
      setFormConfig(value as McpServerConfig);
    }
  };

  const handleIdChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFormId(e.target.value);
    setIdError(null);
  };

  const renderListItem = (id: string, config: McpServerConfig) => {
    // Generate a summary for display
    let summary = '';
    if (config.type === 'local' && config.command) {
      summary = config.command.join(' ');
    } else if (config.type === 'remote' && config.url) {
      summary = config.url;
    }

    const typeLabel = config.type === 'local' ? t('opencode.mcp.typeLocal') : t('opencode.mcp.typeRemote');
    const disabledLabel = config.enabled === false ? ` [${t('opencode.mcp.disabled')}]` : '';

    return (
      <List.Item
        actions={[
          <Button
            key="edit"
            type="text"
            size="small"
            icon={<EditOutlined />}
            onClick={() => handleEdit(id)}
          />,
          <Button
            key="copy"
            type="text"
            size="small"
            icon={<CopyOutlined />}
            onClick={() => handleCopy(id)}
          />,
          <Popconfirm
            key="delete"
            title={t('opencode.mcp.confirmDelete', { name: id })}
            onConfirm={() => handleDelete(id)}
            okText={t('common.confirm')}
            cancelText={t('common.cancel')}
          >
            <Button type="text" size="small" danger icon={<DeleteOutlined />} />
          </Popconfirm>,
        ]}
      >
        <Space style={{ width: '100%' }}>
          <Text strong>{id}</Text>
          <Text type="secondary" style={{ fontSize: 12 }}>
            ({typeLabel}{disabledLabel})
          </Text>
          <Text code style={{ fontSize: 12 }}>
            {summary}
          </Text>
        </Space>
      </List.Item>
    );
  };

  const content = mcpEntries.length === 0 ? (
    <Empty
      image={Empty.PRESENTED_IMAGE_SIMPLE}
      description={t('opencode.mcp.emptyText')}
    />
  ) : (
    <List
      size="small"
      dataSource={mcpEntries}
      renderItem={([id, config]) => renderListItem(id, config)}
    />
  );

  return (
    <>
      <Collapse
        style={{ marginBottom: 16 }}
        activeKey={collapsed ? [] : ['mcp']}
        onChange={(keys) => setCollapsed(!keys.includes('mcp'))}
        items={[
          {
            key: 'mcp',
            label: <Text strong>{t('opencode.mcp.title')}</Text>,
            extra: (
              <Button
                type="primary"
                size="small"
                style={{ fontSize: 12 }}
                icon={<PlusOutlined />}
                onClick={(e) => {
                  e.stopPropagation();
                  handleAdd();
                }}
              >
                {t('opencode.mcp.addServer')}
              </Button>
            ),
            children: content,
          },
        ]}
      />

      <Modal
        title={editingId ? t('opencode.mcp.editServer') : t('opencode.mcp.addServer')}
        open={modalOpen}
        onOk={handleModalOk}
        onCancel={() => setModalOpen(false)}
        okText={t('common.save')}
        cancelText={t('common.cancel')}
        okButtonProps={{ disabled: !jsonValid || !formId.trim() || !formConfig }}
        width={600}
        destroyOnHidden
      >
        <Form layout="vertical">
          <Form.Item
            label={t('opencode.mcp.id')}
            required
            validateStatus={idError ? 'error' : undefined}
            help={idError}
          >
            <Input
              value={formId}
              onChange={handleIdChange}
              placeholder={t('opencode.mcp.idPlaceholder')}
              disabled={!!editingId}
            />
          </Form.Item>

          <Form.Item
            label={t('opencode.mcp.jsonConfig')}
            required
            validateStatus={!jsonValid ? 'error' : undefined}
            help={!jsonValid ? t('opencode.mcp.invalidJson') : undefined}
          >
            <JsonEditor
              value={formConfig}
              onChange={handleJsonChange}
              height={300}
              minHeight={200}
              maxHeight={500}
              resizable
              mode="text"
              placeholder={`{
    "type": "local",
    "command": ["npx", "-y", "my-mcp-command"],
    "enabled": true,
    "environment": {
        "MY_ENV_VAR": "my_env_var_value"
    }
}`}
            />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
};

export default McpSettings;
