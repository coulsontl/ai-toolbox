import React, { useState, useCallback } from 'react';
import { Modal, message } from 'antd';
import {
  ChevronsDown,
  ChevronsUp,
  ExternalLink,
  FileText,
  GripVertical,
  Import,
  LayoutGrid,
  ListTree,
  MinusCircle,
  MoreHorizontal,
  Plus,
  PlusCircle,
  SlidersHorizontal,
  Tags,
  Trash2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { arrayMove } from '@dnd-kit/sortable';
import type { DragEndEvent } from '@dnd-kit/core';
import {
  ManagementButton,
  ManagementIconButton,
  ManagementMenu,
  ManagementSearchInput,
  ManagementSegmented,
  MANAGEMENT_GRID_COLUMN_OPTIONS,
  type ManagementGridColumnSetting,
  type ManagementMenuItem,
} from '@/features/coding/shared/management';
import { useMcp } from '../hooks/useMcp';
import { useMcpActions } from '../hooks/useMcpActions';
import { useMcpTools } from '../hooks/useMcpTools';
import { useMcpStore } from '../stores/mcpStore';
import { McpList } from '../components/McpList';
import { McpGroupedList } from '../components/McpGroupedList';
import { AddMcpModal } from '../components/modals/AddMcpModal';
import { McpSettingsModal } from '../components/modals/McpSettingsModal';
import { ImportMcpModal } from '../components/modals/ImportMcpModal';
import { ImportJsonModal } from '../components/modals/ImportJsonModal';
import { McpMetadataModal } from '../components/modals/McpMetadataModal';
import * as mcpApi from '../services/mcpApi';
import {
  buildMcpGroups,
  filterMcpServersBySearch,
  getMcpGroupToolKeys,
  getMcpGroupOptions,
  getMcpServerIdsMissingTool,
  getMcpServerIdsWithTool,
  isMcpGroupToolsAligned,
  isMcpUngroupedCustomGroup,
  normalizeMcpMetadataText,
} from '../utils/mcpGrouping';
import type { McpGroup, McpServer, CreateMcpServerInput, UpdateMcpServerInput } from '../types';
import styles from './McpPage.module.less';

const AUTO_EXPAND_MCP_THRESHOLD = 20;

function getMcpConfigSummary(server: McpServer): string {
  if (server.server_type === 'stdio') {
    const config = server.server_config as { command?: string };
    return config.command || 'stdio';
  }

  const config = server.server_config as { url?: string };
  return config.url || 'http';
}

const McpPage: React.FC = () => {
  const { t } = useTranslation();
  const { servers, loading, refresh } = useMcp();
  const { tools } = useMcpTools();
  const { setServers, isSettingsModalOpen, setSettingsModalOpen, isImportModalOpen, setImportModalOpen, isImportJsonModalOpen, setImportJsonModalOpen, loadScanResult } = useMcpStore();
  const {
    createServer,
    editServer,
    deleteServer,
    toggleTool,
    reorderServers,
    syncAll,
  } = useMcpActions();

  const [isAddModalOpen, setAddModalOpen] = useState(false);
  const [editingServer, setEditingServer] = useState<McpServer | null>(null);
  const [actionLoading, setActionLoading] = useState(false);
  const [reorderMode, setReorderMode] = useState(false);
  const [searchText, setSearchText] = useState('');
  const [viewMode, setViewMode] = useState<'flat' | 'grouped'>('flat');
  const [groupActiveKeys, setGroupActiveKeys] = useState<string[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = useState(false);
  const [metadataServer, setMetadataServer] = useState<McpServer | null>(null);
  const [batchGroupModalOpen, setBatchGroupModalOpen] = useState(false);
  const [batchGroupValue, setBatchGroupValue] = useState('');
  const [groupToolMode, setGroupToolMode] = useState(false);
  const [gridColumnSetting, setGridColumnSetting] = useState<ManagementGridColumnSetting>('auto');
  const deferredSearchText = React.useDeferredValue(searchText);
  const previousViewModeRef = React.useRef<'flat' | 'grouped'>('flat');
  const previousAutoExpandRef = React.useRef(false);

  const filteredServers = React.useMemo(() => {
    return filterMcpServersBySearch(servers, deferredSearchText, getMcpConfigSummary);
  }, [deferredSearchText, servers]);

  const isSearchActive = !!searchText.trim();
  const isFlatReorderEnabled = viewMode === 'flat' && reorderMode && !isSearchActive;
  const canUseGroupToolMode = viewMode === 'grouped' && !isSearchActive;
  const gridColumns = gridColumnSetting === 'auto' ? undefined : gridColumnSetting;
  const groupOptions = React.useMemo(() => getMcpGroupOptions(servers), [servers]);
  const groupedServers = React.useMemo<McpGroup[]>(() => {
    if (viewMode !== 'grouped') return [];

    return buildMcpGroups(filteredServers, {
      groupUngrouped: t('mcp.groupUngrouped'),
    });
  }, [filteredServers, t, viewMode]);

  React.useEffect(() => {
    if (viewMode !== 'flat' || isSearchActive) {
      setReorderMode(false);
    }
  }, [isSearchActive, viewMode]);

  React.useEffect(() => {
    if (!canUseGroupToolMode) {
      setGroupToolMode(false);
    }
  }, [canUseGroupToolMode]);

  const shouldAutoExpandGroups =
    filteredServers.length > 0 && filteredServers.length < AUTO_EXPAND_MCP_THRESHOLD;

  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      previousViewModeRef.current = viewMode;
      previousAutoExpandRef.current = false;
      return;
    }

    const enteredGroupedView = previousViewModeRef.current !== 'grouped';
    const autoExpandChanged = previousAutoExpandRef.current !== shouldAutoExpandGroups;
    previousViewModeRef.current = viewMode;
    previousAutoExpandRef.current = shouldAutoExpandGroups;
    if (!enteredGroupedView && !autoExpandChanged) {
      return;
    }

    if (shouldAutoExpandGroups) {
      setGroupActiveKeys(groupedServers.map((group) => group.key));
      return;
    }

    setGroupActiveKeys([]);
  }, [groupedServers, shouldAutoExpandGroups, viewMode]);

  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      return;
    }

    const validGroupKeys = new Set(groupedServers.map((group) => group.key));
    setGroupActiveKeys((previousKeys) => {
      const nextKeys = previousKeys.filter((key) => validGroupKeys.has(key));
      return nextKeys.length === previousKeys.length ? previousKeys : nextKeys;
    });
  }, [groupedServers, viewMode]);

  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      setSelectionMode(false);
      setSelectedIds(new Set());
      return;
    }

    setSelectedIds((previousSelectedIds) => {
      const visibleServerIds = new Set(filteredServers.map((server) => server.id));
      const nextSelectedIds = new Set([...previousSelectedIds].filter((id) => visibleServerIds.has(id)));
      return nextSelectedIds.size === previousSelectedIds.size ? previousSelectedIds : nextSelectedIds;
    });
  }, [filteredServers, viewMode]);

  const groupToolTargetGroups = React.useMemo(
    () => groupedServers.filter((group) => !isMcpUngroupedCustomGroup(group)),
    [groupedServers],
  );

  const groupsNeedingToolNormalization = React.useMemo(
    () => groupToolTargetGroups.filter((group) => !isMcpGroupToolsAligned(group)),
    [groupToolTargetGroups],
  );

  const getToolLabel = React.useCallback((toolKey: string) => {
    return tools.find((tool) => tool.key === toolKey)?.display_name ?? toolKey;
  }, [tools]);

  const selectedArray = React.useMemo(() => [...selectedIds], [selectedIds]);
  const selectedServers = React.useMemo(
    () => servers.filter((server) => selectedIds.has(server.id)),
    [selectedIds, servers],
  );
  const hasSelection = selectedArray.length > 0;
  const installedTools = React.useMemo(() => tools.filter((tool) => tool.installed), [tools]);

  const handleToggleSelectionMode = React.useCallback(() => {
    if (selectionMode) {
      setSelectedIds(new Set());
    }
    setSelectionMode((previousSelectionMode) => !previousSelectionMode);
  }, [selectionMode]);

  const handleSelectChange = React.useCallback((serverId: string, checked: boolean) => {
    setSelectedIds((previousSelectedIds) => {
      const nextSelectedIds = new Set(previousSelectedIds);
      if (checked) {
        nextSelectedIds.add(serverId);
      } else {
        nextSelectedIds.delete(serverId);
      }
      return nextSelectedIds;
    });
  }, []);

  const handleSelectAllGroup = React.useCallback((group: McpGroup, checked: boolean) => {
    setSelectedIds((previousSelectedIds) => {
      const nextSelectedIds = new Set(previousSelectedIds);
      for (const server of group.servers) {
        if (checked) {
          nextSelectedIds.add(server.id);
        } else {
          nextSelectedIds.delete(server.id);
        }
      }
      return nextSelectedIds;
    });
  }, []);

  const applyMcpToolState = React.useCallback(async (
    serverIds: string[],
    toolKey: string,
    enabled: boolean,
    quiet = false,
  ) => {
    if (serverIds.length === 0) {
      return true;
    }

    setActionLoading(true);
    try {
      for (const serverId of serverIds) {
        await mcpApi.toggleMcpTool(serverId, toolKey);
      }
      await refresh();
      if (!quiet) {
        message.success(t(
          enabled ? 'mcp.groupTools.addSuccess' : 'mcp.groupTools.removeSuccess',
          { count: serverIds.length, tool: getToolLabel(toolKey) },
        ));
      }
      return true;
    } catch (error) {
      message.error(t('mcp.toggleToolFailed') + ': ' + String(error));
      await refresh();
      return false;
    } finally {
      setActionLoading(false);
    }
  }, [getToolLabel, refresh, t]);

  const handleBatchAddTool = React.useCallback(async (toolKey: string) => {
    const missingServerIds = selectedServers
      .filter((server) => !server.enabled_tools.includes(toolKey))
      .map((server) => server.id);
    await applyMcpToolState(missingServerIds, toolKey, true);
  }, [applyMcpToolState, selectedServers]);

  const handleBatchRemoveTool = React.useCallback(async (toolKey: string) => {
    const enabledServerIds = selectedServers
      .filter((server) => server.enabled_tools.includes(toolKey))
      .map((server) => server.id);
    await applyMcpToolState(enabledServerIds, toolKey, false);
  }, [applyMcpToolState, selectedServers]);

  const batchAddToolItems = React.useMemo<ManagementMenuItem[]>(
    () => installedTools.map((tool) => ({
      key: `add-${tool.key}`,
      label: tool.display_name,
      onSelect: () => handleBatchAddTool(tool.key),
    })),
    [handleBatchAddTool, installedTools],
  );

  const batchRemoveToolItems = React.useMemo<ManagementMenuItem[]>(
    () => installedTools.map((tool) => ({
      key: `remove-${tool.key}`,
      label: tool.display_name,
      onSelect: () => handleBatchRemoveTool(tool.key),
    })),
    [handleBatchRemoveTool, installedTools],
  );

  const handleConfirmBatchGroup = React.useCallback(async () => {
    if (selectedServers.length === 0) {
      return;
    }

    setActionLoading(true);
    try {
      const nextGroup = normalizeMcpMetadataText(batchGroupValue);
      for (const server of selectedServers) {
        await mcpApi.updateMcpMetadata(
          server.id,
          nextGroup,
          normalizeMcpMetadataText(server.user_note),
        );
      }
      await refresh();
      message.success(t('mcp.batch.setGroupSuccess', { count: selectedServers.length }));
      setBatchGroupModalOpen(false);
      setBatchGroupValue('');
      setSelectedIds(new Set());
    } catch (error) {
      message.error(t('mcp.serverUpdateFailed') + ': ' + String(error));
      await refresh();
    } finally {
      setActionLoading(false);
    }
  }, [batchGroupValue, refresh, selectedServers, t]);

  const handleBatchDelete = React.useCallback(() => {
    if (selectedArray.length === 0) {
      return;
    }

    Modal.confirm({
      title: t('mcp.batch.deleteConfirmTitle'),
      content: t('mcp.batch.deleteConfirmMessage', { count: selectedArray.length }),
      okText: t('common.delete'),
      okType: 'danger',
      cancelText: t('common.cancel'),
      onOk: async () => {
        setActionLoading(true);
        try {
          for (const serverId of selectedArray) {
            await mcpApi.deleteMcpServer(serverId);
          }
          await refresh();
          message.success(t('mcp.batch.deleteSuccess', { count: selectedArray.length }));
          setSelectedIds(new Set());
        } catch (error) {
          message.error(t('mcp.serverDeleteFailed') + ': ' + String(error));
          await refresh();
        } finally {
          setActionLoading(false);
        }
      },
    });
  }, [refresh, selectedArray, t]);

  const normalizeMcpGroupTools = React.useCallback(async () => {
    let updatedCount = 0;
    for (const group of groupToolTargetGroups) {
      for (const toolKey of getMcpGroupToolKeys(group)) {
        const missingServerIds = getMcpServerIdsMissingTool(group, toolKey);
        if (missingServerIds.length === 0) {
          continue;
        }

        const saved = await applyMcpToolState(missingServerIds, toolKey, true, true);
        if (!saved) {
          return false;
        }
        updatedCount += missingServerIds.length;
      }
    }

    if (updatedCount > 0) {
      message.success(t('mcp.groupTools.normalizedSuccess', { count: updatedCount }));
    }
    return true;
  }, [applyMcpToolState, groupToolTargetGroups, t]);

  const handleToggleGroupToolMode = React.useCallback((nextEnabled: boolean) => {
    if (!nextEnabled) {
      setGroupToolMode(false);
      return;
    }

    if (!canUseGroupToolMode) {
      return;
    }

    if (groupsNeedingToolNormalization.length === 0) {
      setGroupToolMode(true);
      return;
    }

    Modal.confirm({
      title: t('mcp.groupTools.confirmTitle'),
      content: t('mcp.groupTools.confirmContent', {
        count: groupsNeedingToolNormalization.length,
      }),
      okText: t('mcp.groupTools.confirmOk'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        const normalized = await normalizeMcpGroupTools();
        if (normalized) {
          setGroupToolMode(true);
        }
      },
    });
  }, [canUseGroupToolMode, groupsNeedingToolNormalization.length, normalizeMcpGroupTools, t]);

  const handleAddGroupTool = React.useCallback(async (group: McpGroup, toolKey: string) => {
    if (isMcpUngroupedCustomGroup(group)) {
      return;
    }

    const missingServerIds = getMcpServerIdsMissingTool(group, toolKey);
    await applyMcpToolState(missingServerIds, toolKey, true);
  }, [applyMcpToolState]);

  const handleRemoveGroupTool = React.useCallback(async (group: McpGroup, toolKey: string) => {
    if (isMcpUngroupedCustomGroup(group)) {
      return;
    }

    const enabledServerIds = getMcpServerIdsWithTool(group, toolKey);
    await applyMcpToolState(enabledServerIds, toolKey, false);
  }, [applyMcpToolState]);

  const groupControlItems = React.useMemo<ManagementMenuItem[]>(() => [
    {
      key: 'mode-section',
      type: 'section',
      label: t('mcp.groupControls.modeSection'),
    },
    {
      key: 'selection-mode',
      label: selectionMode ? t('mcp.batch.exitSelectionMode') : t('mcp.batch.selectionMode'),
      tooltip: t('mcp.groupControls.selectionModeTip'),
      kind: 'checkbox',
      active: selectionMode,
      onSelect: handleToggleSelectionMode,
    },
    {
      key: 'group-tools',
      label: t('mcp.groupTools.mode'),
      tooltip: isSearchActive
        ? t('mcp.groupTools.disabledWhileSearching')
        : t('mcp.groupControls.groupToolsTip'),
      kind: 'checkbox',
      active: groupToolMode,
      disabled: loading || actionLoading || isSearchActive,
      onSelect: () => handleToggleGroupToolMode(!groupToolMode),
    },
  ], [
    actionLoading,
    groupToolMode,
    handleToggleGroupToolMode,
    handleToggleSelectionMode,
    isSearchActive,
    loading,
    selectionMode,
    t,
  ]);

  const handleAddServer = async (input: CreateMcpServerInput) => {
    setActionLoading(true);
    try {
      await createServer(input);
      setAddModalOpen(false);
    } finally {
      setActionLoading(false);
    }
  };

  const handleUpdateServer = async (serverId: string, input: UpdateMcpServerInput) => {
    setActionLoading(true);
    try {
      await editServer(serverId, input);
      setEditingServer(null);
      setAddModalOpen(false);
    } finally {
      setActionLoading(false);
    }
  };

  const handleEdit = (server: McpServer) => {
    setEditingServer(server);
    setAddModalOpen(true);
  };

  const handleCloseModal = () => {
    setAddModalOpen(false);
    setEditingServer(null);
  };

  const handleDelete = (serverId: string) => {
    const serverToDelete = servers.find((s) => s.id === serverId);
    Modal.confirm({
      title: t('mcp.deleteConfirm'),
      content: t('mcp.deleteConfirmContent', { name: serverToDelete?.name }),
      okText: t('common.delete'),
      okType: 'danger',
      cancelText: t('common.cancel'),
      onOk: async () => {
        setActionLoading(true);
        try {
          await deleteServer(serverId);
        } finally {
          setActionLoading(false);
        }
      },
    });
  };

  const handleToggleTool = async (serverId: string, toolKey: string) => {
    setActionLoading(true);
    try {
      await toggleTool(serverId, toolKey);
    } finally {
      setActionLoading(false);
    }
  };

  const handleDragEnd = useCallback(
    async (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;

      const oldIndex = servers.findIndex((s) => s.id === active.id);
      const newIndex = servers.findIndex((s) => s.id === over.id);

      if (oldIndex !== -1 && newIndex !== -1) {
        const newServers = arrayMove(servers, oldIndex, newIndex);
        setServers(newServers);
        const ids = newServers.map((s) => s.id);
        await reorderServers(ids);
      }
    },
    [servers, setServers, reorderServers]
  );

  return (
    <div className={styles.mcpPage}>
      <div className={styles.pageHeader}>
        <div className={styles.titleBlock}>
          <div className={styles.titleRow}>
            <h1 className={styles.title}>{t('mcp.title')}</h1>
            <button
              type="button"
              className={styles.docsLink}
              onClick={() => openUrl('https://code.claude.com/docs/en/mcp#installing-mcp-servers')}
            >
              <ExternalLink size={13} aria-hidden="true" />
              {t('mcp.viewDocs')}
            </button>
          </div>
          <p className={styles.pageHint}>{t('mcp.pageHint')}</p>
        </div>
        <ManagementButton
          variant="ghost"
          icon={<MoreHorizontal size={16} aria-hidden="true" />}
          className={styles.moreMenuTrigger}
          onClick={() => setSettingsModalOpen(true)}
        >
          {t('mcp.settings')}
        </ManagementButton>
      </div>

      <div className={styles.toolbar}>
        <div className={styles.toolbarPrimary}>
          <ManagementSearchInput
            placeholder={t('mcp.searchPlaceholder')}
            clearLabel={t('common.clearSearch')}
            value={searchText}
            onChange={setSearchText}
            className={styles.toolbarSearch}
          />
          <span className={styles.resultCount}>
            {filteredServers.length}/{servers.length}
          </span>
          <ManagementButton
            variant="subtle"
            controlSize="compact"
            icon={<Import size={14} aria-hidden="true" />}
            onClick={() => setImportModalOpen(true)}
          >
            {t('mcp.importExisting')}
          </ManagementButton>
          <ManagementButton
            variant="subtle"
            controlSize="compact"
            icon={<FileText size={14} aria-hidden="true" />}
            onClick={() => setImportJsonModalOpen(true)}
          >
            {t('mcp.importJson.button')}
          </ManagementButton>
          <ManagementButton
            variant="primary"
            controlSize="compact"
            icon={<Plus size={14} aria-hidden="true" />}
            onClick={() => setAddModalOpen(true)}
          >
            {t('mcp.addServer')}
          </ManagementButton>
        </div>
        <div className={styles.toolbarActions}>
          {viewMode === 'flat' && (
            <ManagementButton
              variant={reorderMode ? 'primary' : 'ghost'}
              controlSize="compact"
              icon={<GripVertical size={14} aria-hidden="true" />}
              title={
                isSearchActive
                  ? t('mcp.reorderDisabledWhileSearching')
                  : t('mcp.reorderHint')
              }
              className={styles.reorderButton}
              onClick={() => setReorderMode((prev) => !prev)}
              disabled={loading || actionLoading || isSearchActive}
            >
              {t('mcp.reorder')}
            </ManagementButton>
          )}
          {viewMode === 'grouped' && (
            <ManagementMenu
              items={groupControlItems}
              title={t('mcp.groupControls.title')}
              controlSize="compact"
            >
              <SlidersHorizontal size={14} aria-hidden="true" />
            </ManagementMenu>
          )}
          {viewMode === 'grouped' && selectionMode && (
            <>
              <ManagementMenu
                items={batchAddToolItems}
                disabled={!hasSelection || loading || actionLoading}
                title={hasSelection ? t('mcp.batch.addTool') : t('mcp.batch.noneSelected')}
                controlSize="compact"
              >
                <PlusCircle size={14} aria-hidden="true" />
              </ManagementMenu>
              <ManagementMenu
                items={batchRemoveToolItems}
                disabled={!hasSelection || loading || actionLoading}
                title={hasSelection ? t('mcp.batch.removeTool') : t('mcp.batch.noneSelected')}
                controlSize="compact"
              >
                <MinusCircle size={14} aria-hidden="true" />
              </ManagementMenu>
              <ManagementIconButton
                icon={<Tags size={14} aria-hidden="true" />}
                title={hasSelection ? t('mcp.batch.setGroup') : t('mcp.batch.noneSelected')}
                disabled={!hasSelection || loading || actionLoading}
                onClick={() => {
                  setBatchGroupValue('');
                  setBatchGroupModalOpen(true);
                }}
                controlSize="compact"
              />
              <ManagementIconButton
                icon={<Trash2 size={14} aria-hidden="true" />}
                title={hasSelection ? t('mcp.batch.delete') : t('mcp.batch.noneSelected')}
                disabled={!hasSelection || loading || actionLoading}
                onClick={handleBatchDelete}
                danger
                controlSize="compact"
              />
              <span className={styles.batchDivider} />
            </>
          )}
          {viewMode === 'grouped' && (
            <>
              <ManagementIconButton
                icon={<ChevronsDown size={14} aria-hidden="true" />}
                title={t('mcp.expandAll')}
                onClick={() => setGroupActiveKeys(groupedServers.map((g) => g.key))}
                controlSize="compact"
              />
              <ManagementIconButton
                icon={<ChevronsUp size={14} aria-hidden="true" />}
                title={t('mcp.collapseAll')}
                onClick={() => setGroupActiveKeys([])}
                controlSize="compact"
              />
            </>
          )}
          <ManagementSegmented<'flat' | 'grouped'>
            value={viewMode}
            ariaLabel={t('mcp.groupedViewTip')}
            onChange={setViewMode}
            options={[
              { value: 'flat', icon: <LayoutGrid size={13} aria-hidden="true" />, label: t('mcp.viewFlat') },
              { value: 'grouped', icon: <ListTree size={13} aria-hidden="true" />, label: t('mcp.viewGrouped') },
            ]}
          />
        </div>
      </div>

      <div className={styles.content}>
        {viewMode === 'flat' ? (
          <McpList
            servers={filteredServers}
            tools={tools}
            loading={loading || actionLoading}
            columns={gridColumns}
            dragDisabled={!isFlatReorderEnabled}
            onEdit={handleEdit}
            onEditMetadata={setMetadataServer}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onDragEnd={handleDragEnd}
          />
        ) : (
          <McpGroupedList
            groups={groupedServers}
            tools={tools}
            loading={loading || actionLoading}
            columns={gridColumns}
            activeKeys={groupActiveKeys}
            onActiveKeysChange={setGroupActiveKeys}
            selectionMode={selectionMode}
            selectedIds={selectedIds}
            onSelectChange={handleSelectChange}
            onSelectAllGroup={handleSelectAllGroup}
            onEdit={handleEdit}
            onEditMetadata={setMetadataServer}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            groupToolMode={groupToolMode}
            onAddGroupTool={handleAddGroupTool}
            onRemoveGroupTool={handleRemoveGroupTool}
          />
        )}
      </div>

      {isAddModalOpen && (
        <AddMcpModal
          open={isAddModalOpen}
          tools={tools}
          servers={servers}
          editingServer={editingServer}
          onClose={handleCloseModal}
          onSubmit={handleAddServer}
          onUpdate={handleUpdateServer}
          onSyncAll={syncAll}
        />
      )}

      {isSettingsModalOpen && (
        <McpSettingsModal
          open={isSettingsModalOpen}
          cardColumnSetting={gridColumnSetting}
          cardColumnOptions={MANAGEMENT_GRID_COLUMN_OPTIONS}
          onCardColumnSettingChange={setGridColumnSetting}
          onClose={() => setSettingsModalOpen(false)}
        />
      )}

      {isImportModalOpen && (
        <ImportMcpModal
          open={isImportModalOpen}
          onClose={() => setImportModalOpen(false)}
          onSuccess={() => {
            setImportModalOpen(false);
            loadScanResult();
          }}
        />
      )}

      {isImportJsonModalOpen && (
        <ImportJsonModal
          open={isImportJsonModalOpen}
          servers={servers}
          onClose={() => setImportJsonModalOpen(false)}
          onSuccess={() => {
            setImportJsonModalOpen(false);
            loadScanResult();
          }}
          onSyncAll={syncAll}
        />
      )}

      <Modal
        open={batchGroupModalOpen}
        title={t('mcp.batch.setGroupTitle')}
        onCancel={() => setBatchGroupModalOpen(false)}
        onOk={handleConfirmBatchGroup}
        okText={t('common.save')}
        cancelText={t('common.cancel')}
        confirmLoading={actionLoading}
        okButtonProps={{ disabled: selectedArray.length === 0 }}
      >
        <div className={styles.batchGroupEditor}>
          <input
            className={styles.batchGroupInput}
            value={batchGroupValue}
            list="mcp-batch-group-options"
            placeholder={t('mcp.metadata.groupPlaceholder')}
            onChange={(event) => setBatchGroupValue(event.target.value)}
          />
          <datalist id="mcp-batch-group-options">
            {groupOptions.map((group) => (
              <option key={group} value={group} />
            ))}
          </datalist>
          <p className={styles.batchGroupHint}>
            {t('mcp.batch.setGroupHint')}
          </p>
        </div>
      </Modal>

      <McpMetadataModal
        open={!!metadataServer}
        server={metadataServer}
        groupOptions={groupOptions}
        onClose={() => setMetadataServer(null)}
        onSuccess={() => {
          setMetadataServer(null);
          refresh();
        }}
      />
    </div>
  );
};

export default McpPage;
