/**
 * SSH Sync Settings Modal
 *
 * Modal for configuring SSH sync settings with connection management
 */

import React, { useCallback, useEffect, useState } from 'react';
import { Modal, Switch, Select, Button, List, Space, Typography, Alert, Spin, Tag, Modal as AntdModal, Tabs, Tooltip, Progress, theme } from 'antd';
import { CheckCircleOutlined, CloseCircleOutlined, ReloadOutlined, DeleteOutlined, EditOutlined, PlusOutlined, ClearOutlined, ApiOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSSHSync } from '@/features/settings/hooks/useSSHSync';
import {
  isBuiltInDefaultMappingName,
  translateDefaultMappingName,
  translateSyncMessage,
} from '@/features/settings/utils/syncMessageTranslator';
import {
  sshDeleteFileMapping,
  sshResetFileMappings,
  sshTestConnection,
  sshCreateConnection,
  sshUpdateConnection,
  sshDeleteConnection,
  sshSetActiveConnection,
} from '@/services/sshSyncApi';
import { useSettingsStore } from '@/stores';
import { DEFAULT_SSH_DIRECTORY_EXCLUDES } from '@/types/sshsync';
import type { SSHConnection, SSHFileMapping, SSHConnectionResult } from '@/types/sshsync';
import type { WslDirectModuleStatus } from '@/types/wslsync';
import { SSHConnectionModal } from './SSHConnectionModal';
import { SSHFileMappingModal } from './SSHFileMappingModal';

const { Text } = Typography;

// Module display names
const MODULE_NAMES: Record<string, string> = {
  opencode: 'OpenCode',
  claude: 'Claude',
  codex: 'Codex',
  openclaw: 'OpenClaw',
  geminicli: 'Gemini',
};

// Module tag colors
const MODULE_COLORS: Record<string, string> = {
  opencode: 'blue',
  claude: 'purple',
  codex: 'orange',
  openclaw: 'green',
  geminicli: 'cyan',
};

const AUTH_METHOD_TAG_COLORS: Record<SSHConnection['authMethod'], string> = {
  key: 'blue',
  password: 'green',
  none: 'default',
};

const CURRENT_FILE_TAIL_MAX_LENGTH = 72;

const truncateTailPath = (path: string, maxLength = CURRENT_FILE_TAIL_MAX_LENGTH) => {
  if (path.length <= maxLength) {
    return path;
  }

  const tailLength = Math.max(1, maxLength - 3);
  return `...${path.slice(-tailLength)}`;
};

// Map sync module keys to visibleTabs keys
const MODULE_TO_TAB: Record<string, string> = {
  opencode: 'opencode',
  claude: 'claudecode',
  codex: 'codex',
  openclaw: 'openclaw',
  geminicli: 'geminicli',
};

const ALL_MODULE_KEYS = ['opencode', 'claude', 'codex', 'geminicli', 'openclaw'];

interface SSHSyncModalProps {
  open: boolean;
  onClose: () => void;
}

export const SSHSyncModal: React.FC<SSHSyncModalProps> = ({ open, onClose }) => {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { config, status, loading, syncing, syncWarning, syncProgress, saveConfig, sync, dismissSyncWarning } = useSSHSync();
  const { visibleTabs } = useSettingsStore();

  // Filter module keys by visibleTabs
  const visibleModuleKeys = ALL_MODULE_KEYS.filter((k) => visibleTabs.includes(MODULE_TO_TAB[k]));

  const [enabled, setEnabled] = useState(false);
  const [activeConnectionId, setActiveConnectionId] = useState('');
  const [connectionModalOpen, setConnectionModalOpen] = useState(false);
  const [editingConnection, setEditingConnection] = useState<SSHConnection | null>(null);
  const [editingMapping, setEditingMapping] = useState<SSHFileMapping | null>(null);
  const [mappingModalOpen, setMappingModalOpen] = useState(false);
  const [activeModuleTab, setActiveModuleTab] = useState<string>(visibleModuleKeys[0] || 'all');
  const [testResult, setTestResult] = useState<SSHConnectionResult | null>(null);
  const [testing, setTesting] = useState(false);

  const moduleStatusMap = React.useMemo(() => {
    return new Map((config?.moduleStatuses || []).map((item) => [item.module, item] as const));
  }, [config?.moduleStatuses]);

  const getMappingDisplayName = (mapping: SSHFileMapping) => {
    if (isBuiltInDefaultMappingName(mapping.id, mapping.name)) {
      return translateDefaultMappingName(mapping.id, t);
    }
    return mapping.name;
  };

  const getProgressDisplayName = (currentItem: string) => {
    const mapping = config?.fileMappings?.find((item) => item.name === currentItem);
    return mapping ? getMappingDisplayName(mapping) : currentItem;
  };

  const getAuthMethodLabel = (authMethod: SSHConnection['authMethod']) => {
    if (authMethod === 'key') {
      return t('settings.ssh.authKey');
    }
    if (authMethod === 'password') {
      return t('settings.ssh.authPassword');
    }
    return t('settings.ssh.authNone');
  };

  const getModuleStatus = useCallback((module: string): WslDirectModuleStatus | undefined => {
    return moduleStatusMap.get(module);
  }, [moduleStatusMap]);

  const getDirectoryExcludes = (mapping: SSHFileMapping): string[] => {
    if (!mapping.isDirectory) {
      return [];
    }
    return mapping.directoryExcludes ?? [...DEFAULT_SSH_DIRECTORY_EXCLUDES];
  };

  const getDisplayLocalPath = useCallback((mapping: SSHFileMapping) => {
    const status = getModuleStatus(mapping.module);
    if (!status?.isWslDirect || !status.linuxPath) {
      return mapping.localPath;
    }

    const formatWslDisplayPath = (linuxPath: string) => {
      if (!status.distro) {
        return linuxPath;
      }

      const normalizedLinuxPath = linuxPath.replace(/\\/g, '/').replace(/^\/+/, '');
      if (!normalizedLinuxPath) {
        return `\\\\wsl.localhost\\${status.distro}`;
      }

      return `\\\\wsl.localhost\\${status.distro}\\${normalizedLinuxPath.replace(/\//g, '\\')}`;
    };

    const linuxRootPath = status.linuxPath.replace(/\\/g, '/').replace(/\/+$/, '');
    const linuxUserRoot = status.linuxUserRoot?.replace(/\\/g, '/').replace(/\/+$/, '');
    const normalizedLocalPath = mapping.localPath.replace(/\\/g, '/');
    const mappingId = mapping.id;

    if (mappingId === 'opencode-main' || mappingId === 'openclaw-config') {
      return formatWslDisplayPath(status.linuxPath);
    }

    if (mappingId === 'opencode-oh-my' || mappingId === 'opencode-oh-my-slim') {
      const fileName = normalizedLocalPath.split('/').pop();
      return formatWslDisplayPath(fileName ? `${linuxRootPath}/${fileName}` : status.linuxPath);
    }

    if (mappingId === 'opencode-prompt') {
      const parentPath = linuxRootPath.includes('/')
        ? linuxRootPath.slice(0, linuxRootPath.lastIndexOf('/')) || '/'
        : '/';
      return formatWslDisplayPath(`${parentPath.replace(/\/+$/, '') || ''}/AGENTS.md`);
    }

    if (mappingId === 'opencode-plugins') {
      const parentPath = linuxRootPath.includes('/')
        ? linuxRootPath.slice(0, linuxRootPath.lastIndexOf('/')) || '/'
        : '/';
      return formatWslDisplayPath(`${parentPath.replace(/\/+$/, '') || ''}/*.mjs`);
    }

    if (mappingId === 'opencode-auth' && linuxUserRoot) {
      return formatWslDisplayPath(`${linuxUserRoot}/.local/share/opencode/auth.json`);
    }

    if (
      mappingId === 'claude-settings' ||
      mappingId === 'claude-config' ||
      mappingId === 'claude-prompt' ||
      mappingId === 'codex-auth' ||
      mappingId === 'codex-config' ||
      mappingId === 'codex-prompt' ||
      mappingId === 'geminicli-env' ||
      mappingId === 'geminicli-settings' ||
      mappingId === 'geminicli-prompt' ||
      mappingId === 'geminicli-oauth'
    ) {
      const fileName = normalizedLocalPath.split('/').pop();
      return formatWslDisplayPath(fileName ? `${linuxRootPath}/${fileName}` : status.linuxPath);
    }

    return mapping.localPath;
  }, [getModuleStatus]);

  const getProgressMessage = () => {
    if (!syncProgress) {
      return '';
    }

    if (syncProgress.phase === 'files') {
      if (syncProgress.current === 0) {
        return t('settings.ssh.progress.preparingFiles', { total: syncProgress.total });
      }

      return t('settings.ssh.progress.filesWithName', {
        current: syncProgress.current,
        total: syncProgress.total,
        name: getProgressDisplayName(syncProgress.currentItem),
      });
    }

    if (syncProgress.phase === 'skills') {
      if (syncProgress.current === 0) {
        return t('settings.ssh.progress.preparingSkills', { total: syncProgress.total });
      }

      return t('settings.ssh.progress.skillsWithName', {
        current: syncProgress.current,
        total: syncProgress.total,
        name: syncProgress.currentItem,
      });
    }

    return translateSyncMessage(syncProgress.message, 'ssh', t);
  };

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
  const handleTestConnection = useCallback(async (connId?: string) => {
    const targetId = connId || activeConnectionId;
    const conn = config?.connections.find(c => c.id === targetId);
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
  }, [activeConnectionId, config?.connections]);

  // Auto test connection when modal opens or active connection changes
  useEffect(() => {
    if (open && enabled && activeConnectionId && config?.connections.length) {
      handleTestConnection(activeConnectionId);
    }
  }, [activeConnectionId, config?.connections.length, enabled, handleTestConnection, open]);

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

  const handleEditConnectionById = (connId: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    const conn = config?.connections.find(c => c.id === connId);
    if (conn) {
      setEditingConnection(conn);
      setConnectionModalOpen(true);
    }
  };

  const handleDeleteConnectionById = (connId: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    const conn = config?.connections.find(c => c.id === connId);
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
          if (activeConnectionId === conn.id) {
            setActiveConnectionId('');
          }
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
      directoryExcludes: [...DEFAULT_SSH_DIRECTORY_EXCLUDES],
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
                <Tooltip key="edit" title={t('common.edit')}>
                  <Button
                    type="text"
                    size="small"
                    icon={<EditOutlined />}
                    onClick={() => handleEditMapping(item)}
                    disabled={!enabled}
                  />
                </Tooltip>,
                <Tooltip key="delete" title={t('common.delete')}>
                  <Button
                    type="text"
                    size="small"
                    danger
                    icon={<DeleteOutlined />}
                    onClick={() => handleDeleteMapping(item)}
                    disabled={!enabled}
                  />
                </Tooltip>,
              ]}
            >
              <List.Item.Meta
                title={
                  <Space>
                     <Text>{getMappingDisplayName(item)}</Text>
                    <Tag color={MODULE_COLORS[item.module] || 'default'}>{MODULE_NAMES[item.module] || item.module}</Tag>
                    {!item.enabled && <Tag>{t('settings.ssh.disabled')}</Tag>}
                  </Space>
                }
                description={
                  <Space orientation="vertical" size={2}>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {getDisplayLocalPath(item)} → {item.remotePath}
                    </Text>
                    {item.isDirectory && (
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                        <Text type="secondary" style={{ fontSize: 12, flexShrink: 0 }}>
                          {t('settings.ssh.directoryExcludes')}:
                        </Text>
                        {getDirectoryExcludes(item).length > 0 ? (
                          <Space size={[4, 2]} wrap>
                            {getDirectoryExcludes(item).map((exclude) => (
                              <Tag
                                key={exclude}
                                color="default"
                                style={{ marginInlineEnd: 0, fontSize: 11, lineHeight: '18px' }}
                              >
                                {exclude}
                              </Tag>
                            ))}
                          </Space>
                        ) : (
                          <Text type="secondary" style={{ fontSize: 12 }}>
                            {t('settings.ssh.noDirectoryExcludes')}
                          </Text>
                        )}
                      </div>
                    )}
                  </Space>
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
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 4 }}>
            <Text>{t('settings.ssh.enableSync')}</Text>
            <Switch
              checked={enabled}
              onChange={handleEnabledChange}
            />
          </div>
          <Text type="secondary" style={{ fontSize: 12, marginBottom: 16, display: 'block' }}>
            {t('settings.ssh.enableSyncHint')}
          </Text>

          {/* B. Connection management */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
            <Space style={{ flex: 1 }}>
              <Text>{t('settings.ssh.activeConnection')}:</Text>
              <Select
                value={activeConnectionId || undefined}
                onChange={handleActiveConnectionChange}
                disabled={!enabled || !config?.connections.length}
                style={{ width: 300 }}
                placeholder={t('settings.ssh.selectConnection')}
                optionLabelProp="label"
              >
                {config?.connections.map((c) => (
                  <Select.Option key={c.id} value={c.id} label={c.name}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                      <span>{c.name}</span>
                      <Space size={2} onClick={e => e.stopPropagation()}>
                        <Button
                          type="text"
                          icon={<EditOutlined />}
                          size="small"
                          onClick={(e) => handleEditConnectionById(c.id, e)}
                          style={{ padding: '0 4px' }}
                        />
                        <Button
                          type="text"
                          icon={<DeleteOutlined />}
                          size="small"
                          danger
                          onClick={(e) => handleDeleteConnectionById(c.id, e)}
                          style={{ padding: '0 4px' }}
                        />
                      </Space>
                    </div>
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
              <Tooltip title={t('settings.ssh.testConnection')}>
                <Button
                  icon={<ApiOutlined />}
                  onClick={() => handleTestConnection()}
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
                <Tag color={AUTH_METHOD_TAG_COLORS[activeConnection.authMethod] || 'default'}>
                  {getAuthMethodLabel(activeConnection.authMethod)}
                </Tag>
                {testing && <Spin size="small" />}
                {!testing && testResult && (
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
                  <Text type="danger" style={{ fontSize: 12 }}>{translateSyncMessage(testResult.error, 'ssh', t)}</Text>
                </div>
              )}
            </div>
          )}

          {/* D. File Mappings with Tabs */}
          <div style={{ marginTop: 8 }}>
            <Tabs
              size="small"
              tabBarGutter={12}
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
                  label: `${t('settings.ssh.allMappings')} (${config?.fileMappings?.filter(m => visibleModuleKeys.includes(m.module)).length || 0})`,
                  children: renderMappingList(config?.fileMappings?.filter(m => visibleModuleKeys.includes(m.module)) || [], 'all'),
                },
                ...visibleModuleKeys.map((moduleKey) => ({
                  key: moduleKey,
                  label: (
                    <Space>
                      <span>{MODULE_NAMES[moduleKey]}</span>
                      <Tag color={MODULE_COLORS[moduleKey]} style={{ marginRight: 0 }}>
                        {config?.fileMappings?.filter(m => m.module === moduleKey).length || 0}
                      </Tag>
                    </Space>
                  ),
                  children: renderMappingList(config?.fileMappings || [], moduleKey),
                })),
              ]}
            />
          </div>

          <div style={{ fontSize: 12, color: 'var(--color-text-tertiary)', borderLeft: '2px solid var(--color-border)', paddingLeft: 8, marginTop: 8 }}>
            {t('settings.ssh.autoSyncHint')}
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
                  <Text type="secondary">{getProgressMessage()}</Text>
                </div>
                <Progress
                  percent={syncProgress.total > 0 ? Math.round((syncProgress.current / syncProgress.total) * 100) : 0}
                  size="small"
                  status="active"
                />
                {syncProgress.currentFile && (
                  <div style={{ marginTop: 4, minWidth: 0, maxWidth: '100%' }}>
                    <Tooltip title={syncProgress.currentFile} placement="topLeft">
                      <Text
                        type="secondary"
                        style={{
                          display: 'block',
                          maxWidth: '100%',
                          fontSize: 12,
                          whiteSpace: 'nowrap',
                          overflow: 'hidden',
                          textOverflow: 'clip',
                        }}
                      >
                        {t('settings.ssh.progress.currentFile', {
                          file: truncateTailPath(syncProgress.currentFile),
                        })}
                      </Text>
                    </Tooltip>
                  </div>
                )}
              </div>
            )}
            {status?.lastSyncError && (
              <Alert
                type="error"
                title={translateSyncMessage(status.lastSyncError, 'ssh', t)}
                showIcon
                style={{ marginTop: 12 }}
              />
            )}
            {syncWarning && (
              <Alert
                type="warning"
                title={translateSyncMessage(syncWarning, 'ssh', t)}
                showIcon
                closable={{ onClose: dismissSyncWarning }}
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
