/**
 * SSH Sync Settings Modal
 *
 * Modal for configuring SSH sync settings with connection management
 */

import React, { useState, useEffect } from 'react';
import { Modal, Switch, Select, Button, List, Space, Typography, Alert, Spin, Tag, Modal as AntdModal, Tabs, Tooltip, Progress, theme } from 'antd';
import { CheckCircleOutlined, CloseCircleOutlined, ReloadOutlined, DeleteOutlined, EditOutlined, PlusOutlined, ClearOutlined, ApiOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSSHSync } from '@/features/settings/hooks/useSSHSync';
import { SSHConnectionModal } from './SSHConnectionModal';
import { SSHFileMappingModal } from './SSHFileMappingModal';
import {
  sshDeleteFileMapping,
  sshResetFileMappings,
  sshTestConnection,
  sshCreateConnection,
  sshUpdateConnection,
  sshDeleteConnection,
  sshSetActiveConnection,
} from '@/services/sshSyncApi';
import type { SSHConnection, SSHFileMapping, SSHConnectionResult } from '@/types/sshsync';

const { Text } = Typography;

// Module display names
const MODULE_NAMES: Record<string, string> = {
  opencode: 'OpenCode',
  claude: 'Claude Code',
  codex: 'Codex',
};

// Module tag colors
const MODULE_COLORS: Record<string, string> = {
  opencode: 'blue',
  claude: 'purple',
  codex: 'orange',
};

interface SSHSyncModalProps {
  open: boolean;
  onClose: () => void;
}

export const SSHSyncModal: React.FC<SSHSyncModalProps> = ({ open, onClose }) => {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { config, status, loading, syncing, syncWarning, syncProgress, saveConfig, sync, dismissSyncWarning } = useSSHSync();

  const [enabled, setEnabled] = useState(false);
  const [activeConnectionId, setActiveConnectionId] = useState('');
  const [connectionModalOpen, setConnectionModalOpen] = useState(false);
  const [editingConnection, setEditingConnection] = useState<SSHConnection | null>(null);
  const [editingMapping, setEditingMapping] = useState<SSHFileMapping | null>(null);
  const [mappingModalOpen, setMappingModalOpen] = useState(false);
  const [activeModuleTab, setActiveModuleTab] = useState<string>('all');
  const [testResult, setTestResult] = useState<SSHConnectionResult | null>(null);
  const [testing, setTesting] = useState(false);

  // Initialize state when config loads
  useEffect(() => {
    if (config) {
      setEnabled(config.enabled);
      setActiveConnectionId(config.activeConnectionId);
    }
  }, [config]);

  // Handle enabled switch change
  const handleEnabledChange = async (checked: boolean) => {
    if (!config) return;
    setEnabled(checked);
    try {
      await saveConfig({
        ...config,
        enabled: checked,
      });
    } catch (error) {
      console.error('Failed to save enabled state:', error);
    }
  };

  // Handle active connection change
  const handleActiveConnectionChange = async (value: string) => {
    setActiveConnectionId(value);
    setTestResult(null);
    try {
      await sshSetActiveConnection(value);
    } catch (error) {
      console.error('Failed to set active connection:', error);
    }
  };

  // Test connection
  const handleTestConnection = async () => {
    const conn = config?.connections.find(c => c.id === activeConnectionId);
    if (!conn) return;

    setTesting(true);
    setTestResult(null);
    try {
      const result = await sshTestConnection(conn);
      setTestResult(result);
    } catch (error) {
      setTestResult({
        connected: false,
        error: String(error),
      });
    } finally {
      setTesting(false);
    }
  };

  // Connection management
  const handleNewConnection = () => {
    setEditingConnection({
      id: '',
      name: '',
      host: '',
      port: 22,
      username: '',
      authMethod: 'key',
      password: '',
      privateKeyPath: '',
      privateKeyContent: '',
      passphrase: '',
      sortOrder: (config?.connections.length || 0),
    });
    setConnectionModalOpen(true);
  };

  const handleEditConnection = () => {
    const conn = config?.connections.find(c => c.id === activeConnectionId);
    if (conn) {
      setEditingConnection(conn);
      setConnectionModalOpen(true);
    }
  };

  const handleDeleteCurrentConnection = () => {
    const conn = config?.connections.find(c => c.id === activeConnectionId);
    if (!conn) return;

    AntdModal.confirm({
      title: t('settings.ssh.deleteConnectionConfirm'),
      content: t('settings.ssh.deleteConnectionConfirmMessage', { name: conn.name }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await sshDeleteConnection(conn.id);
          setActiveConnectionId('');
          setTestResult(null);
        } catch (error) {
          console.error('Failed to delete connection:', error);
        }
      },
    });
  };

  const handleSaveConnection = async (connection: SSHConnection) => {
    try {
      if (editingConnection?.id) {
        await sshUpdateConnection(connection);
      } else {
        await sshCreateConnection(connection);
        // Auto-select new connection
        setActiveConnectionId(connection.id);
        await sshSetActiveConnection(connection.id);
      }
      setTestResult(null);
    } catch (error) {
      console.error('Failed to save connection:', error);
    }
  };

  // File mapping management
  const handleEditMapping = (mapping: SSHFileMapping) => {
    setEditingMapping(mapping);
    setMappingModalOpen(true);
  };

  const handleAddMapping = (module: string) => {
    const newMapping: SSHFileMapping = {
      id: '',
      name: '',
      module,
      localPath: '',
      remotePath: '',
      enabled: true,
      isPattern: false,
      isDirectory: false,
    };
    setEditingMapping(newMapping);
    setMappingModalOpen(true);
  };

  const handleDeleteMapping = (mapping: SSHFileMapping) => {
    AntdModal.confirm({
      title: t('settings.ssh.deleteMappingConfirm'),
      content: t('settings.ssh.deleteMappingConfirmMessage', { name: mapping.name }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await sshDeleteFileMapping(mapping.id);
        } catch (error) {
          console.error('Failed to delete mapping:', error);
        }
      },
    });
  };

  const handleResetMappings = () => {
    AntdModal.confirm({
      title: t('settings.ssh.resetMappingsTitle'),
      content: t('settings.ssh.resetMappingsContent'),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await sshResetFileMappings();
        } catch (error) {
          console.error('Failed to reset mappings:', error);
        }
      },
    });
  };

  const handleSyncNow = async () => {
    try {
      await sync();
    } catch (error) {
      console.error('Failed to sync:', error);
    }
  };

  const formatSyncTime = (time?: string) => {
    if (!time) return t('settings.ssh.never');
    return new Date(time).toLocaleString();
  };

  const getStatusIcon = () => {
    if (!status) return null;
    if (status.lastSyncStatus === 'success') {
      return <CheckCircleOutlined style={{ color: token.colorSuccess }} />;
    }
    if (status.lastSyncStatus === 'error') {
      return <CloseCircleOutlined style={{ color: token.colorError }} />;
    }
    return null;
  };

  // Get active connection info
  const activeConnection = config?.connections.find(c => c.id === activeConnectionId);

  // Render mapping list
  const renderMappingList = (mappings: SSHFileMapping[], moduleFilter: string) => {
    const filteredMappings = moduleFilter === 'all'
      ? mappings
      : mappings.filter(m => m.module === moduleFilter);

    return (
      <>
        <div style={{ marginBottom: 12, display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <Text type="secondary">
            {moduleFilter === 'all' ? t('settings.ssh.allMappings') : MODULE_NAMES[moduleFilter]} - {filteredMappings.length} {t('settings.ssh.mappings')}
          </Text>
          {moduleFilter !== 'all' && (
            <Button
              type="dashed"
              size="small"
              icon={<PlusOutlined />}
              onClick={() => handleAddMapping(moduleFilter)}
              disabled={!enabled}
            >
              {t('settings.ssh.addMapping')}
            </Button>
          )}
        </div>
        <List
          size="small"
          dataSource={filteredMappings}
          renderItem={(item: SSHFileMapping) => (
            <List.Item
              actions={[
                <Button
                  type="link"
                  icon={<EditOutlined />}
                  onClick={() => handleEditMapping(item)}
                  disabled={!enabled}
                >
                  {t('common.edit')}
                </Button>,
                <Button
                  type="link"
                  danger
                  icon={<DeleteOutlined />}
                  onClick={() => handleDeleteMapping(item)}
                  disabled={!enabled}
                >
                  {t('common.delete')}
                </Button>,
              ]}
            >
              <List.Item.Meta
                title={
                  <Space>
                    <Text>{item.name}</Text>
                    <Tag color={MODULE_COLORS[item.module] || 'default'}>{MODULE_NAMES[item.module] || item.module}</Tag>
                    {!item.enabled && <Tag>{t('settings.ssh.disabled')}</Tag>}
                  </Space>
                }
                description={
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {item.localPath} → {item.remotePath}
                  </Text>
                }
              />
            </List.Item>
          )}
          locale={{ emptyText: t('settings.ssh.noMappings') }}
        />
      </>
    );
  };

  return (
    <>
      <Modal
        title={t('settings.ssh.title')}
        open={open}
        onCancel={onClose}
        width={700}
        footer={null}
      >
        <Spin spinning={loading}>
          {/* A. Enable switch */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
            <Text>{t('settings.ssh.enableSync')}</Text>
            <Switch
              checked={enabled}
              onChange={handleEnabledChange}
            />
          </div>

          {/* B. Connection management */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
            <Space style={{ flex: 1 }}>
              <Text>{t('settings.ssh.activeConnection')}:</Text>
              <Select
                value={activeConnectionId || undefined}
                onChange={handleActiveConnectionChange}
                disabled={!enabled || !config?.connections.length}
                style={{ width: 200 }}
                placeholder={t('settings.ssh.selectConnection')}
              >
                {config?.connections.map((c) => (
                  <Select.Option key={c.id} value={c.id}>
                    {c.name}
                  </Select.Option>
                ))}
              </Select>
            </Space>
            <Space>
              <Tooltip title={t('settings.ssh.newConnection')}>
                <Button
                  icon={<PlusOutlined />}
                  onClick={handleNewConnection}
                  disabled={!enabled}
                  size="small"
                />
              </Tooltip>
              <Tooltip title={t('settings.ssh.editConnection')}>
                <Button
                  icon={<EditOutlined />}
                  onClick={handleEditConnection}
                  disabled={!enabled || !activeConnectionId}
                  size="small"
                />
              </Tooltip>
              <Tooltip title={t('settings.ssh.deleteConnection')}>
                <Button
                  icon={<DeleteOutlined />}
                  onClick={handleDeleteCurrentConnection}
                  disabled={!enabled || !activeConnectionId}
                  size="small"
                  danger
                />
              </Tooltip>
              <Tooltip title={t('settings.ssh.testConnection')}>
                <Button
                  icon={<ApiOutlined />}
                  onClick={handleTestConnection}
                  disabled={!enabled || !activeConnectionId}
                  loading={testing}
                  size="small"
                />
              </Tooltip>
            </Space>
          </div>

          {/* C. Connection status */}
          {activeConnection && (
            <div style={{ marginBottom: 16, padding: 8, background: 'var(--color-bg-elevated)', borderRadius: 4 }}>
              <Space wrap>
                <Text type="secondary">{activeConnection.host}:{activeConnection.port}</Text>
                <Tag>{activeConnection.username}@</Tag>
                <Tag color={activeConnection.authMethod === 'key' ? 'blue' : 'green'}>
                  {activeConnection.authMethod === 'key' ? t('settings.ssh.authKey') : t('settings.ssh.authPassword')}
                </Tag>
                {testResult && (
                  testResult.connected
                    ? <Tag color="success">{t('settings.ssh.connected')}</Tag>
                    : <Tag color="error">{t('settings.ssh.connectionFailed')}</Tag>
                )}
              </Space>
              {testResult?.serverInfo && (
                <div style={{ marginTop: 4 }}>
                  <Text type="secondary" style={{ fontSize: 12 }}>{testResult.serverInfo}</Text>
                </div>
              )}
              {testResult?.error && (
                <div style={{ marginTop: 4 }}>
                  <Text type="danger" style={{ fontSize: 12 }}>{testResult.error}</Text>
                </div>
              )}
            </div>
          )}

          {/* D. File Mappings with Tabs */}
          <div style={{ marginTop: 8 }}>
            <Tabs
              activeKey={activeModuleTab}
              onChange={setActiveModuleTab}
              tabBarExtraContent={
                (config?.fileMappings?.length ?? 0) > 0 ? (
                  <Button
                    type="link"
                    size="small"
                    danger
                    icon={<ClearOutlined />}
                    onClick={handleResetMappings}
                    disabled={!enabled}
                  >
                    {t('common.reset')}
                  </Button>
                ) : null
              }
              items={[
                {
                  key: 'all',
                  label: `${t('settings.ssh.allMappings')} (${config?.fileMappings?.length || 0})`,
                  children: renderMappingList(config?.fileMappings || [], 'all'),
                },
                {
                  key: 'opencode',
                  label: (
                    <Space>
                      <span>OpenCode</span>
                      <Tag color={MODULE_COLORS.opencode} style={{ marginRight: 0 }}>
                        {config?.fileMappings?.filter(m => m.module === 'opencode').length || 0}
                      </Tag>
                    </Space>
                  ),
                  children: renderMappingList(config?.fileMappings || [], 'opencode'),
                },
                {
                  key: 'claude',
                  label: (
                    <Space>
                      <span>Claude Code</span>
                      <Tag color={MODULE_COLORS.claude} style={{ marginRight: 0 }}>
                        {config?.fileMappings?.filter(m => m.module === 'claude').length || 0}
                      </Tag>
                    </Space>
                  ),
                  children: renderMappingList(config?.fileMappings || [], 'claude'),
                },
                {
                  key: 'codex',
                  label: (
                    <Space>
                      <span>Codex</span>
                      <Tag color={MODULE_COLORS.codex} style={{ marginRight: 0 }}>
                        {config?.fileMappings?.filter(m => m.module === 'codex').length || 0}
                      </Tag>
                    </Space>
                  ),
                  children: renderMappingList(config?.fileMappings || [], 'codex'),
                },
              ]}
            />
          </div>

          {/* E. Sync Status */}
          <div style={{ marginTop: 24, padding: 12, background: 'var(--color-bg-elevated)', borderRadius: 4 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <Space>
                <Text>{t('settings.ssh.lastSyncTime')}:</Text>
                <Text>{formatSyncTime(status?.lastSyncTime)}</Text>
                {getStatusIcon()}
              </Space>
              <Button
                type="primary"
                icon={<ReloadOutlined />}
                onClick={handleSyncNow}
                disabled={!enabled || syncing || !activeConnectionId}
                loading={syncing}
              >
                {t('settings.ssh.syncNow')}
              </Button>
            </div>
            {syncing && syncProgress && (
              <div style={{ marginTop: 12 }}>
                <div style={{ marginBottom: 4 }}>
                  <Text type="secondary">{syncProgress.message}</Text>
                </div>
                <Progress
                  percent={syncProgress.total > 0 ? Math.round((syncProgress.current / syncProgress.total) * 100) : 0}
                  size="small"
                  status="active"
                />
              </div>
            )}
            {status?.lastSyncError && (
              <Alert
                type="error"
                message={status.lastSyncError}
                showIcon
                style={{ marginTop: 12 }}
              />
            )}
            {syncWarning && (
              <Alert
                type="warning"
                message={syncWarning}
                showIcon
                closable
                onClose={dismissSyncWarning}
                style={{ marginTop: 12 }}
              />
            )}
          </div>
        </Spin>
      </Modal>

      {/* Connection Modal */}
      <SSHConnectionModal
        open={connectionModalOpen}
        onClose={() => {
          setConnectionModalOpen(false);
          setEditingConnection(null);
        }}
        onSave={handleSaveConnection}
        connection={editingConnection}
      />

      {/* File Mapping Modal */}
      <SSHFileMappingModal
        open={mappingModalOpen}
        onClose={() => {
          setMappingModalOpen(false);
          setEditingMapping(null);
        }}
        mapping={editingMapping}
      />
    </>
  );
};
