import React from 'react';
import { createPortal } from 'react-dom';
import {
  Modal,
  message,
} from 'antd';
import {
  ChevronsDown,
  ChevronsUp,
  ChevronRight,
  ExternalLink,
  GripVertical,
  Import,
  FileJson,
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
import { SkillGroupsModal } from '../components/modals/SkillGroupsModal';
import { SkillInventoryModal } from '../components/modals/SkillInventoryModal';
import * as api from '../services/skillsApi';
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
import { GROUP_TOOL_BATCH_OPTIONS } from '../utils/batchToolOptions';
import type { ManagedSkill, SkillEnabledFilter, SkillGroup, SkillViewMode } from '../types';
import styles from './SkillsPage.module.less';

const AUTO_EXPAND_SKILL_THRESHOLD = 20;

interface ToolbarOptionsPopoverProps {
  title: string;
  active?: boolean;
  activeTitle?: string;
  children: React.ReactNode | ((controls: { close: () => void }) => React.ReactNode);
}

interface ToolbarActionItemProps {
  icon: React.ReactNode;
  title: string;
  description: string;
  onClick: () => void;
}

const POPOVER_VIEWPORT_MARGIN = 12;
const POPOVER_MAX_WIDTH = 420;

const ToolbarOptionsPopover: React.FC<ToolbarOptionsPopoverProps> = ({ title, active, activeTitle, children }) => {
  const triggerRef = React.useRef<HTMLSpanElement | null>(null);
  const popoverRef = React.useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = React.useState(false);
  const [position, setPosition] = React.useState({ top: 0, left: 0 });

  const closePopover = React.useCallback(() => setOpen(false), []);

  const updatePosition = React.useCallback(() => {
    const triggerElement = triggerRef.current;
    if (!triggerElement) {
      return;
    }

    const rect = triggerElement.getBoundingClientRect();
    const popoverWidth = popoverRef.current?.offsetWidth
      ?? Math.min(POPOVER_MAX_WIDTH, window.innerWidth - POPOVER_VIEWPORT_MARGIN * 2);
    const nextLeft = Math.min(
      Math.max(POPOVER_VIEWPORT_MARGIN, rect.right - popoverWidth),
      Math.max(POPOVER_VIEWPORT_MARGIN, window.innerWidth - popoverWidth - POPOVER_VIEWPORT_MARGIN),
    );

    const popoverHeight = popoverRef.current?.offsetHeight
      ?? Math.min(window.innerHeight - POPOVER_VIEWPORT_MARGIN * 2, window.innerHeight);
    const nextTop = Math.min(
      Math.max(POPOVER_VIEWPORT_MARGIN, rect.bottom + 6),
      Math.max(POPOVER_VIEWPORT_MARGIN, window.innerHeight - popoverHeight - POPOVER_VIEWPORT_MARGIN),
    );

    setPosition({
      top: nextTop,
      left: nextLeft,
    });
  }, []);

  React.useLayoutEffect(() => {
    if (open) {
      updatePosition();
    }
  }, [open, updatePosition]);

  React.useEffect(() => {
    if (!open) {
      return undefined;
    }

    updatePosition();

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target) || popoverRef.current?.contains(target)) {
        return;
      }
      closePopover();
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        closePopover();
      }
    };

    window.addEventListener('pointerdown', handlePointerDown, true);
    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('resize', updatePosition);
    window.addEventListener('scroll', updatePosition, true);

    return () => {
      window.removeEventListener('pointerdown', handlePointerDown, true);
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('resize', updatePosition);
      window.removeEventListener('scroll', updatePosition, true);
    };
  }, [closePopover, open, updatePosition]);

  return (
    <span ref={triggerRef} className={styles.toolbarOptionsHost}>
      <ManagementIconButton
        icon={<SlidersHorizontal size={14} aria-hidden="true" />}
        title={activeTitle ?? title}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={activeTitle ?? title}
        className={active ? styles.toolbarOptionsTriggerActive : undefined}
        onClick={() => {
          updatePosition();
          setOpen((previousOpen) => !previousOpen);
        }}
        controlSize="compact"
      />
      {open && createPortal(
        <div
          ref={popoverRef}
          className={styles.toolbarOptionsPopover}
          role="dialog"
          aria-label={title}
          style={{ top: position.top, left: position.left }}
        >
          {typeof children === 'function' ? children({ close: closePopover }) : children}
        </div>,
        document.body,
      )}
    </span>
  );
};

const ToolbarActionItem: React.FC<ToolbarActionItemProps> = ({ icon, title, description, onClick }) => (
  <button type="button" className={styles.toolbarActionItem} onClick={onClick}>
    <span className={styles.toolbarActionIcon} aria-hidden="true">{icon}</span>
    <span className={styles.toolbarActionContent}>
      <span className={styles.toolbarActionTitle}>{title}</span>
      <span className={styles.toolbarActionDescription}>{description}</span>
    </span>
    <ChevronRight size={15} className={styles.toolbarActionArrow} aria-hidden="true" />
  </button>
);

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
    groups,
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
  const [viewMode, setViewMode] = React.useState<SkillViewMode>('flat');
  const [groupMode, setGroupMode] = React.useState<SkillGroupingMode>('custom');
  const [groupActiveKeys, setGroupActiveKeys] = React.useState<string[]>([]);
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = React.useState(false);
  const [reorderMode, setReorderMode] = React.useState(false);
  const [metadataSkill, setMetadataSkill] = React.useState<ManagedSkill | null>(null);
  const [batchGroupModalOpen, setBatchGroupModalOpen] = React.useState(false);
  const [batchGroupValue, setBatchGroupValue] = React.useState('');
  const [groupsModalOpen, setGroupsModalOpen] = React.useState(false);
  const [inventoryModalOpen, setInventoryModalOpen] = React.useState(false);
  const [enabledFilter, setEnabledFilter] = React.useState<SkillEnabledFilter>('all');
  const [groupToolMode, setGroupToolMode] = React.useState(false);
  const [gridColumnSetting, setGridColumnSetting] = React.useState<ManagementGridColumnSetting>('auto');
  const deferredSearchText = React.useDeferredValue(searchText);
  const previousViewModeRef = React.useRef<SkillViewMode>('flat');
  const previousAutoExpandRef = React.useRef(false);
  const hasUserSelectedViewModeRef = React.useRef(false);

  // Initialize data on mount
  React.useEffect(() => {
    let cancelled = false;
    api.getDefaultViewMode()
      .then((mode) => {
        if (!cancelled && !hasUserSelectedViewModeRef.current) {
          setViewMode(mode);
        }
      })
      .catch(console.error);
    refresh();

    return () => {
      cancelled = true;
    };
  }, []);

  const handleViewModeChange = React.useCallback((mode: SkillViewMode) => {
    hasUserSelectedViewModeRef.current = true;
    setViewMode(mode);
  }, []);

  const handleDefaultViewModeApply = React.useCallback((mode: SkillViewMode) => {
    hasUserSelectedViewModeRef.current = true;
    setViewMode(mode);
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
    handleSetManagementEnabled,
  } = useSkillActions({ allTools });

  // Filter skills by search text
  const filteredSkills = React.useMemo(() => {
    const byStatus = skills.filter((skill) => {
      if (enabledFilter === 'enabled') return skill.management_enabled;
      if (enabledFilter === 'disabled') return !skill.management_enabled;
      return true;
    });
    return filterSkillsBySearch(byStatus, deferredSearchText);
  }, [skills, deferredSearchText, enabledFilter]);

  const isSearchActive = !!searchText.trim();
  const isFlatReorderEnabled = viewMode === 'flat' && reorderMode && !isSearchActive;
  const canUseGroupToolMode = viewMode === 'grouped' && groupMode === 'custom' && !isSearchActive;
  const groupOptions = React.useMemo(() => getSkillGroupOptions(groups), [groups]);

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
    const normalizedGroupName = normalizeSkillMetadataText(batchGroupValue);
    const groupId = normalizedGroupName
      ? groupOptions.find((group) => group.name === normalizedGroupName)?.id
        ?? await api.saveSkillGroup(normalizedGroupName, null, groupOptions.length)
      : null;
    const saved = await handleBatchSetGroup(
      selectedArray,
      groupId,
    );
    if (!saved) {
      return;
    }
    setBatchGroupModalOpen(false);
    setBatchGroupValue('');
    setSelectedIds(new Set());
  }, [batchGroupValue, groupOptions, handleBatchSetGroup, selectedArray]);

  const handleSetSkillEnabled = React.useCallback((skill: ManagedSkill, enabled: boolean) => {
    if (!enabled) {
      Modal.confirm({
        title: t('skills.disableConfirmTitle'),
        content: t('skills.disableConfirmContent', { name: skill.name, count: skill.enabled_tools.length }),
        okText: t('skills.disableSkill'),
        cancelText: t('common.cancel'),
        onOk: () => handleSetManagementEnabled(skill, false),
      });
      return;
    }

    const restoreTools = skill.disabled_previous_tools.filter((toolId) => allTools.some((tool) => tool.id === toolId));
    Modal.confirm({
      title: t('skills.enableConfirmTitle'),
      content: restoreTools.length > 0
        ? t('skills.enableConfirmContent', { count: restoreTools.length })
        : t('skills.enableConfirmEmpty'),
      okText: t('skills.enableSkill'),
      cancelText: t('common.cancel'),
      onOk: () => handleSetManagementEnabled(skill, true, restoreTools),
    });
  }, [allTools, handleSetManagementEnabled, t]);

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
      groups,
    );
  }, [filteredSkills, viewMode, groupMode, getGithubInfo, groups, t]);

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

        const saved = await handleBatchAddTool(
          missingSkillIds,
          toolId,
          GROUP_TOOL_BATCH_OPTIONS,
        );
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
    await handleBatchAddTool(missingSkillIds, toolId, GROUP_TOOL_BATCH_OPTIONS);
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

  const groupToolControlTitle = groupMode !== 'custom'
    ? t('skills.groupControls.groupToolsCustomOnlyTip')
    : isSearchActive
      ? t('skills.groupTools.disabledWhileSearching')
      : t('skills.groupControls.groupToolsTip');

  const toolbarOptionStates = React.useMemo(() => {
    const states: string[] = [];
    if (enabledFilter !== 'all') {
      states.push(t(`skills.enabledFilter.${enabledFilter}`));
    }
    if (viewMode === 'flat' && reorderMode) {
      states.push(t('skills.reorder'));
    }
    if (groupMode !== 'custom') {
      states.push(t('skills.groupBySource'));
    }
    if (viewMode === 'grouped' && selectionMode) {
      states.push(t('skills.batch.bulkManage'));
    }
    if (viewMode === 'grouped' && groupToolMode) {
      states.push(t('skills.groupControls.groupTools'));
    }
    return states;
  }, [enabledFilter, groupMode, groupToolMode, reorderMode, selectionMode, t, viewMode]);

  const toolbarOptionsActive = toolbarOptionStates.length > 0;
  const toolbarOptionsTitle = toolbarOptionsActive
    ? t('skills.toolbar.optionsActive', { states: toolbarOptionStates.join(' / ') })
    : t('skills.toolbar.options');

  const flatReorderDisabledHint = viewMode === 'flat' && isSearchActive
    ? t('skills.reorderDisabledWhileSearching')
    : null;
  const groupToolsDisabledHint = viewMode === 'grouped' && (groupMode !== 'custom' || isSearchActive)
    ? groupToolControlTitle
    : null;

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
          <ToolbarOptionsPopover
            title={t('skills.toolbar.options')}
            active={toolbarOptionsActive}
            activeTitle={toolbarOptionsTitle}
          >
            {({ close }) => (
              <>
                <section className={styles.toolbarOptionsSection} aria-label={t('skills.toolbar.viewControls')}>
                  <div className={styles.toolbarOptionsSectionTitle}>{t('skills.toolbar.viewFilters')}</div>
                  {toolbarOptionsActive && (
                    <div className={styles.toolbarActiveSummary} title={toolbarOptionStates.join(' / ')}>
                      <span className={styles.toolbarActiveDot} aria-hidden="true" />
                      <span className={styles.toolbarActiveText}>
                        {t('skills.toolbar.activeSummary', { states: toolbarOptionStates.join(' / ') })}
                      </span>
                    </div>
                  )}
                  <div className={styles.toolbarOptionRow}>
                    <span className={styles.toolbarOptionLabel}>{t('skills.enabledFilter.label')}</span>
                    <ManagementSegmented<SkillEnabledFilter>
                      value={enabledFilter}
                      ariaLabel={t('skills.enabledFilter.label')}
                      onChange={setEnabledFilter}
                      options={[
                        { value: 'all', label: t('skills.enabledFilter.all') },
                        { value: 'enabled', label: t('skills.enabledFilter.enabled') },
                        { value: 'disabled', label: t('skills.enabledFilter.disabled') },
                      ]}
                    />
                  </div>
                  {viewMode === 'flat' && (
                    <div className={styles.toolbarOptionRow}>
                      <span className={styles.toolbarOptionLabel}>{t('skills.toolbar.arrange')}</span>
                      <ManagementSegmented<'browse' | 'reorder'>
                        value={reorderMode ? 'reorder' : 'browse'}
                        ariaLabel={t('skills.reorder')}
                        title={isSearchActive ? t('skills.reorderDisabledWhileSearching') : t('skills.reorderHint')}
                        disabled={isSearchActive}
                        onChange={(nextMode) => setReorderMode(nextMode === 'reorder')}
                        options={[
                          { value: 'browse', label: t('skills.groupControls.browseMode') },
                          {
                            value: 'reorder',
                            icon: <GripVertical size={13} aria-hidden="true" />,
                            label: t('skills.reorder'),
                            title: isSearchActive ? t('skills.reorderDisabledWhileSearching') : t('skills.reorderHint'),
                          },
                        ]}
                      />
                      {flatReorderDisabledHint && (
                        <div className={styles.toolbarOptionHint}>{flatReorderDisabledHint}</div>
                      )}
                    </div>
                  )}
                  {viewMode === 'grouped' && (
                    <>
                      <div className={styles.toolbarOptionRow}>
                        <span className={styles.toolbarOptionLabel}>{t('skills.groupControls.groupingLabel')}</span>
                        <ManagementSegmented<SkillGroupingMode>
                          value={groupMode}
                          ariaLabel={t('skills.groupControls.groupingLabel')}
                          onChange={setGroupMode}
                          options={[
                            {
                              value: 'custom',
                              label: t('skills.groupByCustom'),
                              title: t('skills.groupControls.customGroupingTip'),
                            },
                            {
                              value: 'source',
                              label: t('skills.groupBySource'),
                              title: t('skills.groupControls.sourceGroupingTip'),
                            },
                          ]}
                        />
                      </div>
                      <div className={styles.toolbarOptionRow}>
                        <span className={styles.toolbarOptionLabel}>{t('skills.groupControls.selectionModeLabel')}</span>
                        <ManagementSegmented<'browse' | 'select'>
                          value={selectionMode ? 'select' : 'browse'}
                          ariaLabel={t('skills.groupControls.selectionModeLabel')}
                          onChange={(nextMode) => {
                            if ((nextMode === 'select') !== selectionMode) {
                              handleToggleSelectionMode();
                            }
                          }}
                          options={[
                            { value: 'browse', label: t('skills.groupControls.browseMode') },
                            {
                              value: 'select',
                              label: t('skills.batch.bulkManage'),
                              title: t('skills.groupControls.selectionModeTip'),
                            },
                          ]}
                        />
                        <div className={styles.toolbarOptionInfo}>{t('skills.groupControls.selectionModeTip')}</div>
                      </div>
                      <div className={styles.toolbarOptionRow}>
                        <span className={styles.toolbarOptionLabel}>{t('skills.groupControls.groupToolsLabel')}</span>
                        <ManagementSegmented<'card' | 'group'>
                          value={groupToolMode ? 'group' : 'card'}
                          ariaLabel={t('skills.groupControls.groupToolsLabel')}
                          title={groupToolControlTitle}
                          disabled={groupMode !== 'custom' || loading || actionLoading || isSearchActive}
                          onChange={(nextMode) => handleToggleGroupToolMode(nextMode === 'group')}
                          options={[
                            { value: 'card', label: t('skills.groupControls.cardTools') },
                            {
                              value: 'group',
                              label: t('skills.groupControls.groupTools'),
                              title: groupToolControlTitle,
                            },
                          ]}
                        />
                        <div className={styles.toolbarOptionInfo}>{t('skills.groupControls.groupToolsTip')}</div>
                        {groupToolsDisabledHint && (
                          <div className={styles.toolbarOptionHint}>{groupToolsDisabledHint}</div>
                        )}
                      </div>
                    </>
                  )}
                </section>
                <section className={styles.toolbarOptionsSection} aria-label={t('skills.toolbar.managementControls')}>
                  <div className={styles.toolbarOptionsSectionTitle}>{t('skills.toolbar.management')}</div>
                  <div className={styles.toolbarActionList}>
                    <ToolbarActionItem
                      icon={<Tags size={14} aria-hidden="true" />}
                      title={t('skills.toolbar.groupManagement')}
                      description={t('skills.toolbar.groupManagementDescription')}
                      onClick={() => {
                        close();
                        setGroupsModalOpen(true);
                      }}
                    />
                    <ToolbarActionItem
                      icon={<FileJson size={14} aria-hidden="true" />}
                      title={t('skills.toolbar.inventory')}
                      description={t('skills.toolbar.inventoryDescription')}
                      onClick={() => {
                        close();
                        setInventoryModalOpen(true);
                      }}
                    />
                  </div>
                </section>
              </>
            )}
          </ToolbarOptionsPopover>
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
          <ManagementSegmented<SkillViewMode>
            value={viewMode}
            ariaLabel={t('skills.groupedViewTip')}
            className={styles.viewModeSegmented}
            onChange={handleViewModeChange}
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
            onSetManagementEnabled={handleSetSkillEnabled}
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
            onSetManagementEnabled={handleSetSkillEnabled}
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
          onDefaultViewModeApply={handleDefaultViewModeApply}
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
              <option key={group.id} value={group.name} />
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

      <SkillGroupsModal
        open={groupsModalOpen}
        groups={groups}
        onClose={() => setGroupsModalOpen(false)}
        onSuccess={refresh}
      />

      <SkillInventoryModal
        open={inventoryModalOpen}
        onClose={() => setInventoryModalOpen(false)}
        onSuccess={refresh}
      />

      <NewToolsModal
        open={isNewToolsModalOpen}
      />
    </div>
  );
};

export default SkillsPage;
