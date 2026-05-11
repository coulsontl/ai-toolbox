import React from 'react';
import {
  Modal,
  message,
} from 'antd';
import {
  ChevronsDown,
  ChevronsUp,
  ExternalLink,
  GripVertical,
  Import,
  LayoutGrid,
  ListTree,
  MinusCircle,
  MoreHorizontal,
  Plus,
  PlusCircle,
  RefreshCw,
  SlidersHorizontal,
  Tags,
  Trash2,
} from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
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
import { useSkillsStore } from '../stores/skillsStore';
import { useSkills } from '../hooks/useSkills';
import { useSkillActions } from '../hooks/useSkillActions';
import { SkillsList } from '../components/SkillsList';
import { SkillsGroupedList } from '../components/SkillsGroupedList';
import { AddSkillModal } from '../components/modals/AddSkillModal';
import { ImportModal } from '../components/modals/ImportModal';
import { SkillsSettingsModal } from '../components/modals/SkillsSettingsModal';
import { DeleteConfirmModal } from '../components/modals/DeleteConfirmModal';
import { NewToolsModal } from '../components/modals/NewToolsModal';
import { SkillMetadataModal } from '../components/modals/SkillMetadataModal';
import {
  buildSkillGroups,
  filterSkillsBySearch,
  getSkillGroupOptions,
  getSkillGroupToolIds,
  getSkillIdsMissingTool,
  getSkillIdsWithTool,
  isSkillGroupToolsAligned,
  isSkillUngroupedCustomGroup,
  normalizeSkillMetadataText,
  type SkillGroupingMode,
} from '../utils/skillGrouping';
import type { ManagedSkill, SkillGroup } from '../types';
import styles from './SkillsPage.module.less';

const AUTO_EXPAND_SKILL_THRESHOLD = 20;

const SkillsPage: React.FC = () => {
  const { t } = useTranslation();
  const {
    isAddModalOpen,
    setAddModalOpen,
    isImportModalOpen,
    setImportModalOpen,
    isSettingsModalOpen,
    setSettingsModalOpen,
    isNewToolsModalOpen,
    loading,
  } = useSkillsStore();

  const {
    skills,
    getAllTools,
    formatRelative,
    getGithubInfo,
    getSkillSourceLabel,
    refresh,
  } = useSkills();

  const [searchText, setSearchText] = React.useState('');
  const [viewMode, setViewMode] = React.useState<'flat' | 'grouped'>('flat');
  const [groupMode, setGroupMode] = React.useState<SkillGroupingMode>('custom');
  const [groupActiveKeys, setGroupActiveKeys] = React.useState<string[]>([]);
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = React.useState(false);
  const [reorderMode, setReorderMode] = React.useState(false);
  const [metadataSkill, setMetadataSkill] = React.useState<ManagedSkill | null>(null);
  const [batchGroupModalOpen, setBatchGroupModalOpen] = React.useState(false);
  const [batchGroupValue, setBatchGroupValue] = React.useState('');
  const [groupToolMode, setGroupToolMode] = React.useState(false);
  const [gridColumnSetting, setGridColumnSetting] = React.useState<ManagementGridColumnSetting>('auto');
  const deferredSearchText = React.useDeferredValue(searchText);
  const previousViewModeRef = React.useRef<'flat' | 'grouped'>('flat');
  const previousAutoExpandRef = React.useRef(false);

  // Initialize data on mount
  React.useEffect(() => {
    refresh();
  }, []);

  const allTools = getAllTools();

  const {
    actionLoading,
    updatingSkillIds,
    deleteSkillId,
    setDeleteSkillId,
    skillToDelete,
    batchDeleteIds,
    setBatchDeleteIds,
    handleToggleTool,
    handleUpdate,
    handleDelete,
    confirmDelete,
    handleDragEnd,
    handleBatchRefresh,
    handleBatchDelete,
    confirmBatchDelete,
    handleBatchAddTool,
    handleBatchRemoveTool,
    handleBatchSetGroup,
  } = useSkillActions({ allTools });

  // Filter skills by search text
  const filteredSkills = React.useMemo(() => {
    return filterSkillsBySearch(skills, deferredSearchText);
  }, [skills, deferredSearchText]);

  const isSearchActive = !!searchText.trim();
  const isFlatReorderEnabled = viewMode === 'flat' && reorderMode && !isSearchActive;
  const canUseGroupToolMode = viewMode === 'grouped' && groupMode === 'custom' && !isSearchActive;
  const groupOptions = React.useMemo(() => getSkillGroupOptions(skills), [skills]);

  React.useEffect(() => {
    if (viewMode !== 'flat' || isSearchActive) {
      setReorderMode(false);
    }
  }, [viewMode, isSearchActive]);

  React.useEffect(() => {
    if (!canUseGroupToolMode) {
      setGroupToolMode(false);
    }
  }, [canUseGroupToolMode]);

  // Keep selection scoped to the visible grouped list.
  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      setSelectionMode(false);
      setSelectedIds(new Set());
    } else {
      setSelectedIds((prev) => {
        const allSkillIds = new Set(filteredSkills.map((s) => s.id));
        const next = new Set([...prev].filter((id) => allSkillIds.has(id)));
        return next.size === prev.size ? prev : next;
      });
    }
  }, [viewMode, filteredSkills]);

  const handleToggleSelectionMode = React.useCallback(() => {
    if (selectionMode) {
      setSelectedIds(new Set());
    }
    setSelectionMode((previousSelectionMode) => !previousSelectionMode);
  }, [selectionMode]);

  const handleSelectChange = React.useCallback((skillId: string, checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (checked) {
        next.add(skillId);
      } else {
        next.delete(skillId);
      }
      return next;
    });
  }, []);

  const handleSelectAllGroup = React.useCallback((group: SkillGroup, checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const skill of group.skills) {
        if (checked) {
          next.add(skill.id);
        } else {
          next.delete(skill.id);
        }
      }
      return next;
    });
  }, []);

  const selectedArray = React.useMemo(() => [...selectedIds], [selectedIds]);
  const hasSelection = selectedArray.length > 0;
  const installedTools = React.useMemo(() => allTools.filter((tool) => tool.installed), [allTools]);
  const gridColumns = gridColumnSetting === 'auto' ? undefined : gridColumnSetting;
  const batchAddToolItems = React.useMemo<ManagementMenuItem[]>(
    () => installedTools.map((tool) => ({
      key: `add-${tool.id}`,
      label: tool.label,
      onSelect: () => handleBatchAddTool(selectedArray, tool.id),
    })),
    [handleBatchAddTool, installedTools, selectedArray],
  );
  const batchRemoveToolItems = React.useMemo<ManagementMenuItem[]>(
    () => installedTools.map((tool) => ({
      key: `remove-${tool.id}`,
      label: tool.label,
      onSelect: () => handleBatchRemoveTool(selectedArray, tool.id),
    })),
    [handleBatchRemoveTool, installedTools, selectedArray],
  );

  const handleConfirmBatchGroup = React.useCallback(async () => {
    const saved = await handleBatchSetGroup(
      selectedArray,
      normalizeSkillMetadataText(batchGroupValue),
    );
    if (!saved) {
      return;
    }
    setBatchGroupModalOpen(false);
    setBatchGroupValue('');
    setSelectedIds(new Set());
  }, [batchGroupValue, handleBatchSetGroup, selectedArray]);

  const groupedSkills = React.useMemo<SkillGroup[]>(() => {
    if (viewMode !== 'grouped') return [];

    return buildSkillGroups(
      filteredSkills,
      groupMode,
      {
        groupLocal: t('skills.groupLocal'),
        groupImport: t('skills.groupImport'),
        groupUngrouped: t('skills.groupUngrouped'),
      },
      getGithubInfo,
    );
  }, [filteredSkills, viewMode, groupMode, getGithubInfo, t]);

  const groupToolTargetGroups = React.useMemo(
    () => groupedSkills.filter((group) => !isSkillUngroupedCustomGroup(group)),
    [groupedSkills],
  );

  const groupsNeedingToolNormalization = React.useMemo(
    () => groupToolTargetGroups.filter((group) => !isSkillGroupToolsAligned(group)),
    [groupToolTargetGroups],
  );

  const normalizeSkillGroupTools = React.useCallback(async () => {
    let updatedCount = 0;
    for (const group of groupToolTargetGroups) {
      for (const toolId of getSkillGroupToolIds(group)) {
        const missingSkillIds = getSkillIdsMissingTool(group, toolId);
        if (missingSkillIds.length === 0) {
          continue;
        }

        const saved = await handleBatchAddTool(missingSkillIds, toolId, { quiet: true });
        if (!saved) {
          return false;
        }
        updatedCount += missingSkillIds.length;
      }
    }

    if (updatedCount > 0) {
      message.success(t('skills.groupTools.normalizedSuccess', { count: updatedCount }));
    }
    return true;
  }, [groupToolTargetGroups, handleBatchAddTool, t]);

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
      title: t('skills.groupTools.confirmTitle'),
      content: t('skills.groupTools.confirmContent', {
        count: groupsNeedingToolNormalization.length,
      }),
      okText: t('skills.groupTools.confirmOk'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        const normalized = await normalizeSkillGroupTools();
        if (normalized) {
          setGroupToolMode(true);
        }
      },
    });
  }, [canUseGroupToolMode, groupsNeedingToolNormalization.length, normalizeSkillGroupTools, t]);

  const handleAddGroupTool = React.useCallback(async (group: SkillGroup, toolId: string) => {
    if (isSkillUngroupedCustomGroup(group)) {
      return;
    }

    const missingSkillIds = getSkillIdsMissingTool(group, toolId);
    if (missingSkillIds.length === 0) {
      return;
    }
    await handleBatchAddTool(missingSkillIds, toolId);
  }, [handleBatchAddTool]);

  const handleRemoveGroupTool = React.useCallback(async (group: SkillGroup, toolId: string) => {
    if (isSkillUngroupedCustomGroup(group)) {
      return;
    }

    const syncedSkillIds = getSkillIdsWithTool(group, toolId);
    if (syncedSkillIds.length === 0) {
      return;
    }
    await handleBatchRemoveTool(syncedSkillIds, toolId);
  }, [handleBatchRemoveTool]);

  const groupControlItems = React.useMemo<ManagementMenuItem[]>(() => [
    {
      key: 'mode-section',
      type: 'section',
      label: t('skills.groupControls.modeSection'),
    },
    {
      key: 'selection-mode',
      label: selectionMode ? t('skills.batch.exitSelectionMode') : t('skills.batch.selectionMode'),
      tooltip: t('skills.groupControls.selectionModeTip'),
      kind: 'checkbox',
      active: selectionMode,
      onSelect: handleToggleSelectionMode,
    },
    {
      key: 'group-tools',
      label: t('skills.groupTools.mode'),
      tooltip: groupMode !== 'custom'
        ? t('skills.groupControls.groupToolsCustomOnlyTip')
        : isSearchActive
          ? t('skills.groupTools.disabledWhileSearching')
          : t('skills.groupControls.groupToolsTip'),
      kind: 'checkbox',
      active: groupToolMode,
      disabled: groupMode !== 'custom' || loading || actionLoading || isSearchActive,
      onSelect: () => handleToggleGroupToolMode(!groupToolMode),
    },
    {
      key: 'group-section',
      type: 'section',
      label: t('skills.groupControls.groupSection'),
    },
    {
      key: 'group-custom',
      label: t('skills.groupByCustom'),
      tooltip: t('skills.groupControls.customGroupingTip'),
      kind: 'radio',
      active: groupMode === 'custom',
      onSelect: () => setGroupMode('custom'),
    },
    {
      key: 'group-source',
      label: t('skills.groupBySource'),
      tooltip: t('skills.groupControls.sourceGroupingTip'),
      kind: 'radio',
      active: groupMode === 'source',
      onSelect: () => setGroupMode('source'),
    },
  ], [
    actionLoading,
    groupMode,
    groupToolMode,
    handleToggleGroupToolMode,
    handleToggleSelectionMode,
    isSearchActive,
    loading,
    selectionMode,
    t,
  ]);

  const shouldAutoExpandGroups =
    filteredSkills.length > 0 && filteredSkills.length < AUTO_EXPAND_SKILL_THRESHOLD;

  // Entering grouped view or crossing the auto-expand threshold applies the default strategy once.
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
      setGroupActiveKeys(groupedSkills.map((group) => group.key));
      return;
    }

    setGroupActiveKeys([]);
  }, [groupedSkills, shouldAutoExpandGroups, viewMode]);

  // Refreshes should only prune removed groups, not overwrite user-expanded state.
  React.useEffect(() => {
    if (viewMode !== 'grouped') {
      return;
    }

    const validGroupKeys = new Set(groupedSkills.map((group) => group.key));
    setGroupActiveKeys((previousKeys) => {
      const nextKeys = previousKeys.filter((key) => validGroupKeys.has(key));
      return nextKeys.length === previousKeys.length ? previousKeys : nextKeys;
    });
  }, [groupedSkills, viewMode]);

  return (
    <div className={styles.skillsPage}>
      <div className={styles.pageHeader}>
        <div className={styles.titleBlock}>
          <div className={styles.titleRow}>
            <h1 className={styles.title}>{t('skills.title')}</h1>
            <button
              type="button"
              className={styles.docsLink}
              onClick={() => openUrl('https://code.claude.com/docs/en/skills')}
            >
              <ExternalLink size={13} aria-hidden="true" />
              {t('skills.viewDocs')}
            </button>
          </div>
          <p className={styles.pageHint}>{t('skills.pageHint')}</p>
        </div>
        <ManagementButton
          variant="ghost"
          icon={<MoreHorizontal size={16} aria-hidden="true" />}
          className={styles.moreMenuTrigger}
          onClick={() => setSettingsModalOpen(true)}
        >
          {t('skills.settings')}
        </ManagementButton>
      </div>

      <div className={styles.toolbar}>
        <div className={styles.toolbarPrimary}>
          <ManagementSearchInput
            placeholder={t('skills.searchPlaceholder')}
            clearLabel={t('common.clearSearch')}
            value={searchText}
            onChange={setSearchText}
            className={styles.toolbarSearch}
          />
          <span className={styles.resultCount}>
            {filteredSkills.length}/{skills.length}
          </span>
          <ManagementButton
            variant="subtle"
            controlSize="compact"
            icon={<Import size={14} aria-hidden="true" />}
            onClick={() => setImportModalOpen(true)}
          >
            {t('skills.importExisting')}
          </ManagementButton>
          <ManagementButton
            variant="primary"
            controlSize="compact"
            icon={<Plus size={14} aria-hidden="true" />}
            onClick={() => setAddModalOpen(true)}
          >
            {t('skills.addSkill')}
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
                  ? t('skills.reorderDisabledWhileSearching')
                  : t('skills.reorderHint')
              }
              className={styles.reorderButton}
              onClick={() => setReorderMode((prev) => !prev)}
              disabled={isSearchActive}
            >
              {t('skills.reorder')}
            </ManagementButton>
          )}
          {viewMode === 'grouped' && (
            <ManagementMenu
              items={groupControlItems}
              title={t('skills.groupControls.title')}
              controlSize="compact"
            >
              <SlidersHorizontal size={14} aria-hidden="true" />
            </ManagementMenu>
          )}
          {viewMode === 'grouped' && selectionMode && (
            <>
              <ManagementIconButton
                icon={<RefreshCw size={14} aria-hidden="true" />}
                title={hasSelection ? t('skills.batch.refresh') : t('skills.batch.noneSelected')}
                disabled={!hasSelection || loading || actionLoading}
                onClick={() => handleBatchRefresh(selectedArray)}
                controlSize="compact"
              />
              <ManagementMenu
                items={batchAddToolItems}
                disabled={!hasSelection || loading || actionLoading}
                title={hasSelection ? t('skills.batch.addTool') : t('skills.batch.noneSelected')}
                controlSize="compact"
              >
                <PlusCircle size={14} aria-hidden="true" />
              </ManagementMenu>
              <ManagementMenu
                items={batchRemoveToolItems}
                disabled={!hasSelection || loading || actionLoading}
                title={hasSelection ? t('skills.batch.removeTool') : t('skills.batch.noneSelected')}
                controlSize="compact"
              >
                <MinusCircle size={14} aria-hidden="true" />
              </ManagementMenu>
              <ManagementIconButton
                icon={<Tags size={14} aria-hidden="true" />}
                title={hasSelection ? t('skills.batch.setGroup') : t('skills.batch.noneSelected')}
                disabled={!hasSelection || loading || actionLoading}
                onClick={() => {
                  setBatchGroupValue('');
                  setBatchGroupModalOpen(true);
                }}
                controlSize="compact"
              />
              <ManagementIconButton
                icon={<Trash2 size={14} aria-hidden="true" />}
                title={hasSelection ? t('skills.batch.delete') : t('skills.batch.noneSelected')}
                disabled={!hasSelection || loading || actionLoading}
                onClick={() => handleBatchDelete(selectedArray)}
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
                title={t('skills.expandAll')}
                onClick={() => setGroupActiveKeys(groupedSkills.map((g) => g.key))}
                controlSize="compact"
              />
              <ManagementIconButton
                icon={<ChevronsUp size={14} aria-hidden="true" />}
                title={t('skills.collapseAll')}
                onClick={() => setGroupActiveKeys([])}
                controlSize="compact"
              />
            </>
          )}
          <ManagementSegmented<'flat' | 'grouped'>
            value={viewMode}
            ariaLabel={t('skills.groupedViewTip')}
            onChange={setViewMode}
            options={[
              { value: 'flat', icon: <LayoutGrid size={13} aria-hidden="true" />, label: t('skills.viewFlat') },
              { value: 'grouped', icon: <ListTree size={13} aria-hidden="true" />, label: t('skills.viewGrouped') },
            ]}
          />
        </div>
      </div>

      <div className={styles.content}>
        {viewMode === 'flat' ? (
          <SkillsList
            skills={filteredSkills}
            allTools={allTools}
            loading={loading || actionLoading}
            updatingSkillIds={updatingSkillIds}
            columns={gridColumns}
            dragDisabled={!isFlatReorderEnabled}
            getGithubInfo={getGithubInfo}
            getSkillSourceLabel={getSkillSourceLabel}
            formatRelative={formatRelative}
            onUpdate={handleUpdate}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onEditMetadata={setMetadataSkill}
            onDragEnd={handleDragEnd}
          />
        ) : (
          <SkillsGroupedList
            groups={groupedSkills}
            allTools={allTools}
            loading={loading || actionLoading}
            updatingSkillIds={updatingSkillIds}
            columns={gridColumns}
            activeKeys={groupActiveKeys}
            onActiveKeysChange={setGroupActiveKeys}
            selectionMode={selectionMode}
            selectedIds={selectedIds}
            onSelectChange={handleSelectChange}
            onSelectAllGroup={handleSelectAllGroup}
            getGithubInfo={getGithubInfo}
            getSkillSourceLabel={getSkillSourceLabel}
            formatRelative={formatRelative}
            onUpdate={handleUpdate}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onEditMetadata={setMetadataSkill}
            groupToolMode={groupToolMode}
            onAddGroupTool={handleAddGroupTool}
            onRemoveGroupTool={handleRemoveGroupTool}
          />
        )}
      </div>

      {isAddModalOpen && (
        <AddSkillModal
          open={isAddModalOpen}
          onClose={() => setAddModalOpen(false)}
          allTools={allTools}
          onSuccess={() => {
            setAddModalOpen(false);
            refresh();
          }}
        />
      )}

      {isImportModalOpen && (
        <ImportModal
          open={isImportModalOpen}
          onClose={() => setImportModalOpen(false)}
          onSuccess={() => {
            setImportModalOpen(false);
            refresh();
          }}
        />
      )}

      {isSettingsModalOpen && (
        <SkillsSettingsModal
          open={isSettingsModalOpen}
          cardColumnSetting={gridColumnSetting}
          cardColumnOptions={MANAGEMENT_GRID_COLUMN_OPTIONS}
          onCardColumnSettingChange={setGridColumnSetting}
          onClose={() => setSettingsModalOpen(false)}
        />
      )}

      <DeleteConfirmModal
        open={!!deleteSkillId}
        skillName={skillToDelete?.name || ''}
        onClose={() => setDeleteSkillId(null)}
        onConfirm={confirmDelete}
        loading={actionLoading}
      />

      <Modal
        open={batchDeleteIds.length > 0}
        title={t('skills.batch.deleteConfirmTitle')}
        onCancel={() => setBatchDeleteIds([])}
        onOk={confirmBatchDelete}
        okButtonProps={{ danger: true, loading: actionLoading }}
        okText={t('skills.batch.delete')}
      >
        {t('skills.batch.deleteConfirmMessage', { count: batchDeleteIds.length })}
      </Modal>

      <Modal
        open={batchGroupModalOpen}
        title={t('skills.batch.setGroupTitle')}
        onCancel={() => setBatchGroupModalOpen(false)}
        onOk={handleConfirmBatchGroup}
        okButtonProps={{ loading: actionLoading }}
        okText={t('common.save')}
        cancelText={t('common.cancel')}
      >
        <div className={styles.batchGroupEditor}>
          <input
            className={styles.batchGroupInput}
            value={batchGroupValue}
            list="skills-batch-group-options"
            placeholder={t('skills.metadata.groupPlaceholder')}
            onChange={(event) => setBatchGroupValue(event.target.value)}
          />
          <datalist id="skills-batch-group-options">
            {groupOptions.map((group) => (
              <option key={group} value={group} />
            ))}
          </datalist>
          <p className={styles.batchGroupHint}>
            {t('skills.batch.setGroupHint')}
          </p>
        </div>
      </Modal>

      <SkillMetadataModal
        open={!!metadataSkill}
        skill={metadataSkill}
        groupOptions={groupOptions}
        onClose={() => setMetadataSkill(null)}
        onSuccess={() => {
          setMetadataSkill(null);
          refresh();
        }}
      />

      <NewToolsModal
        open={isNewToolsModalOpen}
      />
    </div>
  );
};

export default SkillsPage;
