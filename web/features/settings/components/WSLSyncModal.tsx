/**
 * WSL Sync Settings Modal
 *
 * Modal for configuring WSL sync settings
 */

import React, { useState, useEffect, useCallback } from 'react';
import { Modal, Form, Switch, Select, Button, List, Space, Typography, Alert, Spin, Tag, Modal as AntdModal, Tabs, Tooltip, Progress } from 'antd';
import { CheckCircleOutlined, CloseCircleOutlined, ReloadOutlined, DeleteOutlined, EditOutlined, PlusOutlined, ClearOutlined, CodeOutlined, FolderOpenOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useWSLSync } from '@/features/settings/hooks/useWSLSync';
import { useSettingsStore } from '@/stores';
import {
  isBuiltInDefaultMappingName,
  translateDefaultMappingName,
  translateSyncMessage,
} from '@/features/settings/utils/syncMessageTranslator';
import { FileMappingModal } from './FileMappingModal';
import { wslDeleteFileMapping, wslResetFileMappings, wslOpenTerminal, wslOpenFolder, wslGetDistroState } from '@/services/wslSyncApi';
import type { FileMapping } from '@/types/wslsync';

const { Text } = Typography;

// Module display names
const MODULE_NAMES: Record<string, string> = {
  opencode: 'OpenCode',
  claude: 'Claude Code',
  codex: 'Codex',
  openclaw: 'OpenClaw',
};

// Module tag colors
const MODULE_COLORS: Record<string, string> = {
  opencode: 'blue',
  claude: 'purple',
  codex: 'orange',
  openclaw: 'green',
};

// Map sync module keys to visibleTabs keys
const MODULE_TO_TAB: Record<string, string> = {
  opencode: 'opencode',
  claude: 'claudecode',
  codex: 'codex',
  openclaw: 'openclaw',
};

const ALL_MODULE_KEYS = ['opencode', 'claude', 'codex', 'openclaw'];

interface WSLSyncModalProps {
  open: boolean;
  onClose: () => void;
}

export const WSLSyncModal: React.FC<WSLSyncModalProps> = ({ open, onClose }) => {
  const { t } = useTranslation();
  const { config, status, loading, syncing, syncWarning, syncProgress, saveConfig, sync, detect, checkDistro, dismissSyncWarning } = useWSLSync();
  const { visibleTabs } = useSettingsStore();

  // Filter module keys by visibleTabs
  const visibleModuleKeys = ALL_MODULE_KEYS.filter((k) => visibleTabs.includes(MODULE_TO_TAB[k]));

  const [form] = Form.useForm();
  const [enabled, setEnabled] = useState(false);
  const [distro, setDistro] = useState('Ubuntu');
  const [distros, setDistros] = useState<string[]>([]);
  const [distroStatus, setDistroStatus] = useState<'checking' | 'available' | 'unavailable'>('checking');
  const [distroState, setDistroState] = useState<'Running' | 'Stopped' | 'Unknown'>('Unknown');
  const [editingMapping, setEditingMapping] = useState<FileMapping | null>(null);
  const [mappingModalOpen, setMappingModalOpen] = useState(false);
  const [activeModuleTab, setActiveModuleTab] = useState<string>(visibleModuleKeys[0] || 'all');

  const getMappingDisplayName = useCallback((mapping: FileMapping) => {
    if (isBuiltInDefaultMappingName(mapping.id, mapping.name)) {
      return translateDefaultMappingName(mapping.id, t);
    }
    return mapping.name;
  }, [t]);

  const getProgressDisplayName = useCallback((currentItem: string) => {
    const mapping = config?.fileMappings?.find((item) => item.name === currentItem);
    return mapping ? getMappingDisplayName(mapping) : currentItem;
  }, [config?.fileMappings, getMappingDisplayName]);

  const getProgressMessage = useCallback(() => {
    if (!syncProgress) {
      return '';
    }

    if (syncProgress.phase === 'files') {
      if (syncProgress.current === 0) {
        return t('settings.wsl.progress.preparingFiles', { total: syncProgress.total });
      }

      return t('settings.wsl.progress.filesWithName', {
        current: syncProgress.current,
        total: syncProgress.total,
        name: getProgressDisplayName(syncProgress.currentItem),
      });
    }

    if (syncProgress.phase === 'skills') {
      if (syncProgress.current === 0) {
        return t('settings.wsl.progress.preparingSkills', { total: syncProgress.total });
      }

      return t('settings.wsl.progress.skillsWithName', {
        current: syncProgress.current,
        total: syncProgress.total,
        name: syncProgress.currentItem,
      });
    }

    return translateSyncMessage(syncProgress.message, 'wsl', t);
  }, [getProgressDisplayName, syncProgress, t]);

  // Initialize form when config loads
  useEffect(() => {
    if (config) {
      setEnabled(config.enabled);
      setDistro(config.distro);
      form.setFieldsValue({
        enabled: config.enabled,
        distro: config.distro,
      });
    }
  }, [config, form]);

  // Detect WSL when modal opens
  const detectWSL = useCallback(async () => {
    try {
      const result = await detect();
      setDistros(result.distros);
      if (result.distros.length > 0 && !result.distros.includes(distro)) {
        setDistro(result.distros[0]);
      }
    } catch (error) {
      console.error('Failed to detect WSL:', error);
    }
  }, [detect, distro]);

  useEffect(() => {
    if (open) {
      detectWSL();
    }
  }, [open, detectWSL]);

  // Check distro availability
  const checkDistroAvailability = useCallback(async () => {
    setDistroStatus('checking');
    try {
      const result = await checkDistro(distro);
      setDistroStatus(result.available ? 'available' : 'unavailable');
      // Also get running state
      if (result.available) {
        const state = await wslGetDistroState(distro);
        setDistroState(state as 'Running' | 'Stopped' | 'Unknown');
      } else {
        setDistroState('Unknown');
      }
    } catch (error) {
      setDistroStatus('unavailable');
      setDistroState('Unknown');
    }
  }, [checkDistro, distro]);

  useEffect(() => {
    if (open && distro) {
      checkDistroAvailability();
    }
  }, [open, distro, checkDistroAvailability]);

  // Handle enabled switch change - save immediately
  const handleEnabledChange = async (checked: boolean) => {
    if (!config) return;
    setEnabled(checked);
    try {
      await saveConfig({
        ...config,
        enabled: checked,
        distro,
      });
    } catch (error) {
      console.error('Failed to save enabled state:', error);
    }
  };

  // Handle distro change - save immediately
  const handleDistroChange = async (value: string) => {
    if (!config) return;
    setDistro(value);
    try {
      await saveConfig({
        ...config,
        enabled,
        distro: value,
      });
      // Check if new distro is available
      setDistroStatus('checking');
      const result = await checkDistro(value);
      setDistroStatus(result.available ? 'available' : 'unavailable');
      // Also get running state
      if (result.available) {
        const state = await wslGetDistroState(value);
        setDistroState(state as 'Running' | 'Stopped' | 'Unknown');
      } else {
        setDistroState('Unknown');
      }
    } catch (error) {
      console.error('Failed to save distro:', error);
    }
  };

  const handleSyncNow = async () => {
    try {
      await sync();
    } catch (error) {
      console.error('Failed to sync:', error);
    }
  };

  const handleEditMapping = (mapping: FileMapping) => {
    setEditingMapping(mapping);
    setMappingModalOpen(true);
  };

  const handleAddMapping = (module: string) => {
    // Create a new mapping with the selected module
    const newMapping: FileMapping = {
      id: '',
      name: '',
      module,
      windowsPath: '',
      wslPath: '',
      enabled: true,
      isPattern: false,
      isDirectory: false,
    };
    setEditingMapping(newMapping);
    setMappingModalOpen(true);
  };

  const handleMappingModalClose = () => {
    setMappingModalOpen(false);
    setEditingMapping(null);
  };

  const handleDeleteMapping = (mapping: FileMapping) => {
    AntdModal.confirm({
      title: t('settings.wsl.deleteMappingConfirm'),
      content: t('settings.wsl.deleteMappingConfirmMessage', { name: mapping.name }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await wslDeleteFileMapping(mapping.id);
        } catch (error) {
          console.error('Failed to delete mapping:', error);
        }
      },
    });
  };

  const handleResetMappings = () => {
    AntdModal.confirm({
      title: t('settings.wsl.resetMappingsTitle'),
      content: t('settings.wsl.resetMappingsContent'),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await wslResetFileMappings();
        } catch (error) {
          console.error('Failed to reset mappings:', error);
        }
      },
    });
  };

  const formatSyncTime = (time?: string) => {
    if (!time) return t('settings.wsl.never');
    return new Date(time).toLocaleString();
  };

  const handleOpenTerminal = async () => {
    try {
      await wslOpenTerminal(distro);
    } catch (error) {
      console.error('Failed to open WSL terminal:', error);
    }
  };

  const handleOpenFolder = async () => {
    try {
      await wslOpenFolder(distro);
    } catch (error) {
      console.error('Failed to open WSL folder:', error);
    }
  };

  const getStatusIcon = () => {
    if (!status) return null;

    if (status.lastSyncStatus === 'success') {
      return <CheckCircleOutlined style={{ color: '#52c41a' }} />;
    }
    if (status.lastSyncStatus === 'error') {
      return <CloseCircleOutlined style={{ color: '#ff4d4f' }} />;
    }
    return null;
  };

  // Render mapping list for a specific module or all
  const renderMappingList = (mappings: FileMapping[], moduleFilter: string) => {
    const filteredMappings = moduleFilter === 'all'
      ? mappings
      : mappings.filter(m => m.module === moduleFilter);

    return (
      <>
        <div style={{ marginBottom: 12, display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <Text type="secondary">
            {moduleFilter === 'all' ? t('settings.wsl.allMappings') : MODULE_NAMES[moduleFilter]} - {filteredMappings.length} {t('settings.wsl.mappings')}
          </Text>
          {moduleFilter !== 'all' && (
            <Button
              type="dashed"
              size="small"
              icon={<PlusOutlined />}
              onClick={() => handleAddMapping(moduleFilter)}
              disabled={!enabled}
            >
              {t('settings.wsl.addMapping')}
            </Button>
          )}
        </div>
        <List
          size="small"
          dataSource={filteredMappings}
          renderItem={(item: FileMapping) => (
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
                    {!item.enabled && <Tag>{t('settings.wsl.disabled')}</Tag>}
                  </Space>
                }
                description={
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {item.windowsPath} → {item.wslPath}
                  </Text>
                }
              />
            </List.Item>
          )}
          locale={{ emptyText: t('settings.wsl.noMappings') }}
        />
      </>
    );
  };

  return (
    <>
      <Modal
        title={t('settings.wsl.title')}
        open={open}
        onCancel={onClose}
        width={700}
        footer={null}
      >
        <Spin spinning={loading}>
          <Form form={form} layout="horizontal">
            {/* Enable WSL Sync - left-right layout */}
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 4 }}>
              <Text>{t('settings.wsl.enableSync')}</Text>
              <Switch
                checked={enabled}
                onChange={handleEnabledChange}
              />
            </div>
            <Text type="secondary" style={{ fontSize: 12, marginBottom: 16, display: 'block' }}>
              {t('settings.wsl.enableSyncSubtitle')}
            </Text>

            {/* WSL Distro - left-right layout */}
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
              <Text>{t('settings.wsl.distro')}</Text>
              <Select
                value={distro}
                onChange={handleDistroChange}
                disabled={!enabled || distros.length === 0}
                style={{ width: 200 }}
              >
                {distros.map((d) => (
                  <Select.Option key={d} value={d}>
                    {d}
                  </Select.Option>
                ))}
              </Select>
            </div>

            {/* Connection Status - left-right layout */}
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
              <Space>
                <Text>{t('settings.wsl.connectionStatus')}:</Text>
                {distroStatus === 'checking' && <Spin size="small" />}
                {distroStatus === 'available' && (
                  <Tooltip title={t('settings.wsl.connectionStatusTooltip')}>
                    <Tag color="success">{t('settings.wsl.connected')}</Tag>
                  </Tooltip>
                )}
                {distroStatus === 'unavailable' && (
                  <Tag color="error">{t('settings.wsl.disconnected')}</Tag>
                )}
                {distroStatus === 'available' && (
                  <Tooltip title={t('settings.wsl.runningStateTooltip')}>
                    <Tag color={distroState === 'Running' ? 'processing' : 'default'}>
                      {distroState === 'Running' ? t('settings.wsl.running') : t('settings.wsl.stopped')}
                    </Tag>
                  </Tooltip>
                )}
              </Space>
              <Space>
                <Button
                  icon={<CodeOutlined />}
                  onClick={handleOpenTerminal}
                  disabled={!enabled || distroStatus !== 'available'}
                  size="small"
                  title={t('settings.wsl.openTerminal')}
                >
                  {t('settings.wsl.terminal')}
                </Button>
                <Button
                  icon={<FolderOpenOutlined />}
                  onClick={handleOpenFolder}
                  disabled={!enabled || distroStatus !== 'available'}
                  size="small"
                  title={t('settings.wsl.openFolder')}
                >
                  {t('settings.wsl.fileManager')}
                </Button>
                <Button
                  icon={<ReloadOutlined />}
                  onClick={checkDistroAvailability}
                  disabled={!enabled}
                  size="small"
                >
                  {t('settings.wsl.detectWSL')}
                </Button>
              </Space>
            </div>

            {/* File Mappings with Tabs */}
            <div style={{ marginTop: 24 }}>
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
                    label: `${t('settings.wsl.allMappings')} (${config?.fileMappings?.filter(m => visibleModuleKeys.includes(m.module)).length || 0})`,
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
              {t('settings.wsl.autoSyncHint')}
            </div>

            {/* Sync Status - left-right layout */}
            <div style={{ marginTop: 24, padding: 12, background: 'var(--color-bg-elevated)', borderRadius: 4 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <Space>
                  <Text>{t('settings.wsl.lastSyncTime')}:</Text>
                  <Text>{formatSyncTime(status?.lastSyncTime)}</Text>
                  {getStatusIcon()}
                </Space>
                <Button
                  type="primary"
                  icon={<ReloadOutlined />}
                  onClick={handleSyncNow}
                  disabled={syncing}
                  loading={syncing}
                >
                  {t('settings.wsl.syncNow')}
                </Button>
              </div>
              {/* Sync Progress */}
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
                </div>
              )}
              {status?.lastSyncError && (
                <Alert
                  type="error"
                  message={translateSyncMessage(status.lastSyncError, 'wsl', t)}
                  showIcon
                  style={{ marginTop: 12 }}
                />
              )}
              {syncWarning && (
                <Alert
                  type="warning"
                  message={translateSyncMessage(syncWarning, 'wsl', t)}
                  showIcon
                  closable
                  onClose={dismissSyncWarning}
                  style={{ marginTop: 12 }}
                />
              )}
            </div>
          </Form>
        </Spin>
      </Modal>

      <FileMappingModal
        open={mappingModalOpen}
        onClose={handleMappingModalClose}
        mapping={editingMapping}
      />
    </>
  );
};
