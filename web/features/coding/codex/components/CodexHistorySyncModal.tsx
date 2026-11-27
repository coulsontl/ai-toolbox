import React from 'react';
import { Alert, Button, Checkbox, Descriptions, Modal, Segmented, Space, Spin, Table, Tag, Typography, message } from 'antd';
import type { ColumnsType } from 'antd/es/table';
import { DatabaseOutlined, FolderOpenOutlined, ReloadOutlined, SyncOutlined, UndoOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import type {
  CodexHistoryProviderGroup,
  CodexHistoryProviderKind,
  CodexHistorySourceMode,
  CodexHistorySyncStatus,
} from '@/types/codex';
import {
  backupCodexHistory,
  getCodexHistorySyncStatus,
  restoreLatestCodexHistoryBackup,
  syncCodexHistory,
} from '@/services/codexApi';
import {
  countCodexHistorySelectedGroups,
  defaultCodexHistorySyncSelection,
  emptyCodexHistorySyncSelection,
  hasCrossUpstreamCodexHistorySelection,
  isCodexHistoryGroupSelectable,
  isCodexHistorySyncSelectionEmpty,
  setCodexHistoryMissingProviderSelected,
  setCodexHistoryProviderSelected,
  type CodexHistorySyncSelection,
} from '../utils/codexHistorySyncSelection';

const { Text, Paragraph } = Typography;

interface CodexHistorySyncModalProps {
  open: boolean;
  sourceMode: CodexHistorySourceMode;
  onCancel: () => void;
  onChanged?: () => void;
}

const resolveInitialHistorySourceMode = (sourceMode: CodexHistorySourceMode): CodexHistorySourceMode => (
  sourceMode === 'wsl' ? 'wsl' : 'all'
);

const isCodexHistoryDatabaseLockedError = (errorMessage: string): boolean => {
  const normalized = errorMessage.toLowerCase();
  return (
    normalized.includes('state database is locked')
    || normalized.includes('database is locked')
    || normalized.includes('database table is locked')
    || normalized.includes('database is busy')
    || normalized.includes('codex is writing history')
    || normalized.includes('codex is writing local history')
  );
};

const providerKindTagColor: Record<CodexHistoryProviderKind, string> = {
  current: 'green',
  ai_toolbox_legacy: 'orange',
  official: 'blue',
  missing: 'default',
  other: 'default',
};

const providerKindLabelKey = (kind: CodexHistoryProviderKind): string => {
  switch (kind) {
    case 'current':
      return 'codex.historySync.providerKindCurrent';
    case 'ai_toolbox_legacy':
      return 'codex.historySync.providerKindAiToolboxLegacy';
    case 'official':
      return 'codex.historySync.providerKindOfficial';
    case 'missing':
      return 'codex.historySync.providerKindMissing';
    default:
      return 'codex.historySync.providerKindOther';
  }
};

const providerGroupRowKey = (group: CodexHistoryProviderGroup): string => (
  group.kind === 'missing' ? '__missing_provider__' : (group.providerId ?? group.kind)
);

const CodexHistorySyncModal: React.FC<CodexHistorySyncModalProps> = ({
  open,
  sourceMode,
  onCancel,
  onChanged,
}) => {
  const { t } = useTranslation();
  const [status, setStatus] = React.useState<CodexHistorySyncStatus | null>(null);
  const [selection, setSelection] = React.useState<CodexHistorySyncSelection>(emptyCodexHistorySyncSelection);
  const [loading, setLoading] = React.useState(false);
  const [actionLoading, setActionLoading] = React.useState<null | 'sync' | 'backup' | 'restore' | 'open'>(null);
  const [selectedSourceMode, setSelectedSourceMode] = React.useState<CodexHistorySourceMode>(() => resolveInitialHistorySourceMode(sourceMode));

  const formatHistorySyncError = React.useCallback((error: unknown) => {
    const errorMessage = error instanceof Error ? error.message : String(error);
    if (isCodexHistoryDatabaseLockedError(errorMessage)) {
      return t('codex.historySync.databaseLocked');
    }
    return errorMessage || t('common.error');
  }, [t]);

  const loadStatus = React.useCallback(async (
    showSuccess = false,
    sourceModeToLoad: CodexHistorySourceMode,
  ) => {
    try {
      setLoading(true);
      const result = await getCodexHistorySyncStatus(sourceModeToLoad);
      setStatus(result);
      setSelection(defaultCodexHistorySyncSelection(result.providerBreakdown));
      if (showSuccess) {
        message.success(t('codex.historySync.statusSuccess'));
      }
    } catch (error) {
      message.error(formatHistorySyncError(error));
    } finally {
      setLoading(false);
    }
  }, [formatHistorySyncError, t]);

  React.useEffect(() => {
    if (open) {
      const nextSourceMode = resolveInitialHistorySourceMode(sourceMode);
      setSelectedSourceMode(nextSourceMode);
      setStatus(null);
      setSelection(emptyCodexHistorySyncSelection());
      void loadStatus(false, nextSourceMode);
    }
  }, [loadStatus, open, sourceMode]);

  const sourceLabel = React.useCallback((sourceStatus: CodexHistorySyncStatus | null) => {
    if (sourceStatus?.runtimeSource === 'wsl') {
      return sourceStatus.runtimeDistro
        ? t('codex.historySync.sourceWslWithDistro', { distro: sourceStatus.runtimeDistro })
        : t('codex.historySync.sourceWsl');
    }
    return t('codex.historySync.sourceLocal');
  }, [t]);

  const availableSources = status?.availableSources ?? [];
  const wslSource = availableSources.find((item) => item.source === 'wsl');
  const showSourceSwitcher = availableSources.some((item) => item.source === 'local') && Boolean(wslSource);
  const activeSourceValue = status?.runtimeSource === 'wsl' ? 'wsl' : 'local';
  const sourceOptions = React.useMemo(() => [
    { label: t('codex.historySync.sourceLocal'), value: 'local' },
    {
      label: wslSource?.distro
        ? t('codex.historySync.sourceWslWithDistro', { distro: wslSource.distro })
        : t('codex.historySync.sourceWsl'),
      value: 'wsl',
    },
  ], [t, wslSource?.distro]);

  const providerGroups = status?.providerBreakdown ?? [];
  const crossUpstreamSelected = hasCrossUpstreamCodexHistorySelection(selection, providerGroups);
  const selectionEmpty = isCodexHistorySyncSelectionEmpty(selection);
  const selectedGroupCount = countCodexHistorySelectedGroups(selection);

  const handleSourceChange = React.useCallback((value: string | number) => {
    const nextSourceMode = value as CodexHistorySourceMode;
    setSelectedSourceMode(nextSourceMode);
    setStatus(null);
    setSelection(emptyCodexHistorySyncSelection());
    void loadStatus(false, nextSourceMode);
  }, [loadStatus]);

  const openBackupDir = async () => {
    if (!status) {
      return;
    }
    try {
      setActionLoading('open');
      try {
        await revealItemInDir(status.backupDir);
      } catch {
        await invoke('open_folder', { path: status.backupDir });
      }
      message.success(t('codex.historySync.openBackupDirSuccess'));
    } catch (error) {
      message.error(formatHistorySyncError(error));
    } finally {
      setActionLoading(null);
    }
  };

  const createBackup = async () => {
    try {
      setActionLoading('backup');
      const result = await backupCodexHistory(selectedSourceMode);
      message.success(t('codex.historySync.backupSuccess', { path: result.backupPath }));
      await loadStatus(false, selectedSourceMode);
    } catch (error) {
      message.error(formatHistorySyncError(error));
    } finally {
      setActionLoading(null);
    }
  };

  const performSync = async () => {
    if (!status) {
      return;
    }
    if (selectionEmpty) {
      message.info(t('codex.historySync.noSelection'));
      return;
    }
    const selectedProviderLabels = [
      ...selection.sourceProviders,
      ...(selection.includeMissingProvider ? [t('codex.historySync.providerKindMissing')] : []),
    ];
    const syncFilter = {
      sourceProviders: selection.sourceProviders,
      includeMissingProvider: selection.includeMissingProvider,
    };
    Modal.confirm({
      title: crossUpstreamSelected
        ? t('codex.historySync.crossUpstreamConfirmTitle')
        : t('codex.historySync.syncConfirmTitle'),
      content: (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <span>
            {t('codex.historySync.syncConfirmContent', {
              source: sourceLabel(status),
              count: selectedGroupCount,
              providers: selectedProviderLabels.join(', '),
              provider: status.currentProvider,
            })}
          </span>
          {crossUpstreamSelected ? (
            <Alert type="warning" showIcon description={t('codex.historySync.crossUpstreamWarning')} />
          ) : null}
        </div>
      ),
      okText: t('codex.historySync.sync'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        try {
          setActionLoading('sync');
          const result = await syncCodexHistory(selectedSourceMode, syncFilter);
          if (result.partialSuccess) {
            message.warning(t('codex.historySync.syncPartialSuccess', {
              threads: result.updatedThreadRows,
              files: result.updatedSessionFiles,
              failed: result.failedSessionFiles,
              error: result.firstSessionFileError || '',
            }));
          } else {
            message.success(t('codex.historySync.syncSuccess', {
              threads: result.updatedThreadRows,
              files: result.updatedSessionFiles,
              index: result.rewrittenIndexEntries,
            }));
          }
          message.info(t('codex.historySync.duration', {
            ms: result.durationMs,
            wait: result.lockWaitMs,
            attempts: result.attempts,
          }));
          setStatus(result.status);
          setSelection(defaultCodexHistorySyncSelection(result.status.providerBreakdown));
          onChanged?.();
        } catch (error) {
          message.error(formatHistorySyncError(error));
        } finally {
          setActionLoading(null);
        }
      },
    });
  };

  const restoreLatest = async () => {
    if (!status?.latestBackupPath) {
      message.info(t('codex.historySync.noBackup'));
      return;
    }
    Modal.confirm({
      title: t('codex.historySync.restoreConfirmTitle'),
      content: t('codex.historySync.restoreConfirmContent'),
      okText: t('codex.historySync.restoreLatest'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel'),
      onOk: async () => {
        try {
          setActionLoading('restore');
          const result = await restoreLatestCodexHistoryBackup(selectedSourceMode);
          message.success(t('codex.historySync.restoreSuccess', { path: result.safetyBackupPath }));
          message.info(t('codex.historySync.duration', {
            ms: result.durationMs,
            wait: result.lockWaitMs,
            attempts: result.attempts,
          }));
          setStatus(result.status);
          setSelection(defaultCodexHistorySyncSelection(result.status.providerBreakdown));
          onChanged?.();
        } catch (error) {
          message.error(formatHistorySyncError(error));
        } finally {
          setActionLoading(null);
        }
      },
    });
  };

  const providerColumns = React.useMemo<ColumnsType<CodexHistoryProviderGroup>>(() => [
    {
      title: '',
      key: 'selection',
      width: 44,
      render: (_value, group) => {
        const selectable = isCodexHistoryGroupSelectable(group);
        if (group.kind === 'missing') {
          return (
            <Checkbox
              checked={selection.includeMissingProvider}
              disabled={!selectable}
              onChange={(event) => setSelection((current) => (
                setCodexHistoryMissingProviderSelected(current, event.target.checked)
              ))}
            />
          );
        }
        const providerId = group.providerId ?? '';
        return (
          <Checkbox
            checked={Boolean(providerId) && selection.sourceProviders.includes(providerId)}
            disabled={!selectable || !providerId}
            onChange={(event) => setSelection((current) => (
              setCodexHistoryProviderSelected(current, providerId, event.target.checked)
            ))}
          />
        );
      },
    },
    {
      title: t('codex.historySync.providerColumn'),
      key: 'providerId',
      render: (_value, group) => (
        group.providerId
          ? <Text code>{group.providerId}</Text>
          : <Text type="secondary">{t('codex.historySync.providerNotRecorded')}</Text>
      ),
    },
    {
      title: t('codex.historySync.kindColumn'),
      key: 'kind',
      width: 150,
      render: (_value, group) => (
        <Tag color={providerKindTagColor[group.kind]}>
          {t(providerKindLabelKey(group.kind))}
        </Tag>
      ),
    },
    {
      title: t('codex.historySync.threadsColumn'),
      dataIndex: 'threadCount',
      key: 'threadCount',
      width: 90,
      align: 'right',
    },
    {
      title: t('codex.historySync.sessionFilesColumn'),
      dataIndex: 'sessionFileCount',
      key: 'sessionFileCount',
      width: 120,
      align: 'right',
    },
  ], [selection, t]);

  const renderStatus = () => {
    if (!status) {
      return null;
    }
    return (
      <div style={{ width: '100%', display: 'flex', flexDirection: 'column', gap: 16 }}>
        <Alert type="info" showIcon description={t('codex.historySync.description')} />
        {showSourceSwitcher ? (
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
            <Text type="secondary">{t('codex.historySync.source')}</Text>
            <Segmented
              size="small"
              value={activeSourceValue}
              options={sourceOptions}
              disabled={loading || Boolean(actionLoading)}
              onChange={handleSourceChange}
            />
          </div>
        ) : null}
        <Descriptions size="small" bordered column={1} title={t('codex.historySync.currentRuntime')}>
          <Descriptions.Item label={t('codex.historySync.source')}>
            <Tag color={status.runtimeSource === 'wsl' ? 'blue' : 'default'}>{sourceLabel(status)}</Tag>
          </Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.codexHome')}>
            <Text code copyable>{status.codexHome}</Text>
          </Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.currentProvider')}>
            <Text code>{status.currentProvider}</Text>
          </Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.currentModel')}>
            <Text code>{status.currentModel || t('codex.historySync.notDetected')}</Text>
          </Descriptions.Item>
        </Descriptions>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <Text strong>{t('codex.historySync.providerBreakdown')}</Text>
          <Table<CodexHistoryProviderGroup>
            size="small"
            columns={providerColumns}
            dataSource={providerGroups}
            rowKey={providerGroupRowKey}
            pagination={false}
            locale={{ emptyText: t('codex.historySync.providerBreakdownEmpty') }}
          />
          <Text type="secondary" style={{ fontSize: 10 }}>{t('codex.historySync.selectionHint')}</Text>
          {crossUpstreamSelected ? (
            <Alert type="warning" showIcon description={t('codex.historySync.crossUpstreamWarning')} />
          ) : null}
        </div>
        <Descriptions size="small" bordered column={2} title={t('codex.historySync.historyState')}>
          <Descriptions.Item label={t('codex.historySync.totalThreads')}>{status.totalThreads}</Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.providerMismatchThreads')}>{status.providerMismatchThreads}</Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.sessionFileCount')}>{status.sessionFileCount}</Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.sessionMetaMismatchCount')}>{status.sessionMetaMismatchCount}</Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.indexedThreads')}>{status.indexedThreads}</Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.missingSessionIndexEntries')}>{status.missingSessionIndexEntries}</Descriptions.Item>
          <Descriptions.Item label={t('codex.historySync.backupCount')}>{status.backupCount}</Descriptions.Item>
        </Descriptions>
        {status.latestBackupPath ? (
          <Paragraph style={{ marginBottom: 0 }}>
            <Text type="secondary">{t('codex.historySync.latestBackupPath')}: </Text>
            <Text code copyable>{status.latestBackupPath}</Text>
          </Paragraph>
        ) : null}
      </div>
    );
  };

  return (
    <Modal
      title={<Space><DatabaseOutlined />{t('codex.historySync.title')}</Space>}
      open={open}
      onCancel={onCancel}
      width={840}
      footer={[
        <Button key="close" onClick={onCancel}>{t('common.close')}</Button>,
        <Button key="refresh" icon={<ReloadOutlined />} loading={loading} onClick={() => void loadStatus(true, selectedSourceMode)}>
          {t('codex.historySync.refresh')}
        </Button>,
        <Button key="backup" icon={<DatabaseOutlined />} loading={actionLoading === 'backup'} onClick={() => void createBackup()}>
          {t('codex.historySync.backup')}
        </Button>,
        <Button key="open" icon={<FolderOpenOutlined />} loading={actionLoading === 'open'} onClick={() => void openBackupDir()} disabled={!status}>
          {t('codex.historySync.openBackupDir')}
        </Button>,
        <Button key="restore" danger icon={<UndoOutlined />} loading={actionLoading === 'restore'} onClick={() => void restoreLatest()} disabled={!status?.latestBackupPath}>
          {t('codex.historySync.restoreLatest')}
        </Button>,
        <Button
          key="sync"
          type="primary"
          icon={<SyncOutlined />}
          loading={actionLoading === 'sync'}
          onClick={() => void performSync()}
          disabled={!status || selectionEmpty}
        >
          {t('codex.historySync.sync')}
        </Button>,
      ]}
    >
      <Spin spinning={loading && !status}>{renderStatus()}</Spin>
    </Modal>
  );
};

export default CodexHistorySyncModal;
