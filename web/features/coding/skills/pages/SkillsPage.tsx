import React from 'react';
import {
  Drawer,
  Modal,
  message,
  Popover,
  Progress,
} from 'antd';
import {
  Check,
  ChevronsDown,
  ChevronsUp,
  ChevronRight,
  ExternalLink,
  FolderInput,
  Folders,
  GripVertical,
  Import,
  FileJson,
  LayoutGrid,
  ListTree,
  MinusCircle,
  MoreHorizontal,
  Plus,
  PlusCircle,
  Power,
  PowerOff,
  Radar,
  RefreshCw,
  Search,
  SlidersHorizontal,
  Tag,
  Trash2,
} from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import {
  ManagementButton,
  ManagementCheckbox,
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
import { SkillDetailPanel } from '../components/SkillDetailPanel';
import { ToolIcon } from '@/features/coding/shared/toolIcon/ToolIcon';
import { AddSkillModal } from '../components/modals/AddSkillModal';
import { BatchTagDialog } from '../components/modals/BatchTagDialog';
import { ImportModal } from '../components/modals/ImportModal';
import { SkillsSettingsModal } from '../components/modals/SkillsSettingsModal';
import { SkillsAutoSyncModal } from '../components/modals/SkillsAutoSyncModal';
import { DeleteConfirmModal } from '../components/modals/DeleteConfirmModal';
import { NewToolsModal } from '../components/modals/NewToolsModal';
import { SkillMetadataModal } from '../components/modals/SkillMetadataModal';
import { SkillGroupsModal } from '../components/modals/SkillGroupsModal';
import { SkillInventoryModal } from '../components/modals/SkillInventoryModal';
import * as api from '../services/skillsApi';
import {
  buildSkillGroups,
  findSkillGroupOptionByName,
  filterSkillsBySearch,
  getSkillGroupOptions,
  getSkillGroupToolIds,
  getSkillIdsMissingTool,
  getSkillIdsWithTool,
  isSkillGroupToolsAligned,
  isSkillUngroupedCustomGroup,
  normalizeSkillMetadataText,
  resolveSkillGroupingMode,
  type SkillGroupingMode,
  type SkillGroupModePreference,
} from '../utils/skillGrouping';
import { GROUP_TOOL_BATCH_OPTIONS } from '../utils/batchToolOptions';
import {
  collectAllTags,
  matchesTagFilters,
  normalizeTagList,
  pruneStaleTagFilters,
  UNTAGGED_FILTER,
} from '../utils/skillTags';
import type { ManagedSkill, SkillEnabledFilter, SkillsUpdateProgress, SkillGroup, SkillViewMode } from '../types';
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

// Options popover built on antd Popover (click trigger, viewport-aware
// placement) so positioning, dismissal and layering stay consistent with the
// rest of the app; only the inner layout comes from module styles.
const ToolbarOptionsPopover: React.FC<ToolbarOptionsPopoverProps> = ({ title, active, activeTitle, children }) => {
  const [open, setOpen] = React.useState(false);
  const close = React.useCallback(() => setOpen(false), []);

  return (
    <Popover
      trigger="click"
      placement="bottomRight"
      arrow={false}
      open={open}
      onOpenChange={setOpen}
      content={(
        <div className={styles.toolbarOptionsPopover} role="dialog" aria-label={title}>
          {typeof children === 'function' ? children({ close }) : children}
        </div>
      )}
    >
      <span className={styles.toolbarOptionsHost}>
        <ManagementIconButton
          icon={<SlidersHorizontal size={14} aria-hidden="true" />}
          title={activeTitle ?? title}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label={activeTitle ?? title}
          className={active ? styles.toolbarOptionsTriggerActive : undefined}
          controlSize="compact"
        />
      </span>
    </Popover>
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

interface TagFilterOption {
  value: string;
  label: string;
  count: number;
}

// axonhub-style faceted tag filter: a dashed outline trigger (plus icon +
// title + selected badges) opening a searchable option list with per-tag
// counts and a clear action. Selection stays multi-select AND; the untagged
// sentinel stays mutually exclusive with concrete tags (handleToggleTagFilter).
const TagFilterDropdown: React.FC<{
  options: TagFilterOption[];
  selected: string[];
  onToggle: (value: string) => void;
  onClear: () => void;
}> = ({ options, selected, onToggle, onClear }) => {
  const { t } = useTranslation();
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState('');

  const selectedSet = React.useMemo(() => new Set(selected), [selected]);
  const selectedBadges = React.useMemo(
    () => options.filter((option) => selectedSet.has(option.value)),
    [options, selectedSet],
  );
  const visibleOptions = React.useMemo(() => {
    const keyword = query.trim().toLowerCase();
    if (!keyword) return options;
    return options.filter((option) => option.label.toLowerCase().includes(keyword));
  }, [options, query]);

  const handleOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (!nextOpen) {
      setQuery('');
    }
  };

  return (
    <Popover
      trigger="click"
      placement="bottomLeft"
      arrow={false}
      open={open}
      onOpenChange={handleOpenChange}
      content={(
        <div className={styles.tagFilterPopover}>
          <div className={styles.tagFilterSearch}>
            <Search size={13} aria-hidden="true" />
            <input
              value={query}
              placeholder={t('skills.tags.searchPlaceholder')}
              aria-label={t('skills.tags.searchPlaceholder')}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
          <div className={styles.tagFilterList} role="listbox" aria-multiselectable aria-label={t('skills.tags.filterLabel')}>
            {visibleOptions.map((option) => {
              const isSelected = selectedSet.has(option.value);
              return (
                <button
                  key={option.value}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  className={styles.tagFilterOption}
                  onClick={() => onToggle(option.value)}
                >
                  <span
                    className={`${styles.tagFilterCheckbox}${isSelected ? ` ${styles.tagFilterCheckboxChecked}` : ''}`}
                    aria-hidden="true"
                  >
                    {isSelected && <Check size={12} />}
                  </span>
                  <span className={styles.tagFilterOptionLabel}>{option.label}</span>
                  <span className={styles.tagFilterCount}>{option.count}</span>
                </button>
              );
            })}
            {visibleOptions.length === 0 && (
              <div className={styles.tagFilterEmpty}>{t('skills.tags.noMatch')}</div>
            )}
            {selected.length > 0 && (
              <>
                {visibleOptions.length > 0 && <div className={styles.tagFilterSeparator} />}
                <button type="button" className={styles.tagFilterClear} onClick={onClear}>
                  {t('skills.tags.clearFilter')}
                </button>
              </>
            )}
          </div>
        </div>
      )}
    >
      <button
        type="button"
        className={styles.tagFilterTrigger}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={t('skills.tags.filterLabel')}
      >
        <PlusCircle size={14} aria-hidden="true" />
        <span>{t('skills.tags.filterLabel')}</span>
        {selectedBadges.length > 0 && (
          <>
            <span className={styles.tagFilterDivider} aria-hidden="true" />
            {selectedBadges.length > 2
              ? (
                <span className={styles.tagFilterBadge}>
                  {t('skills.tags.selectedCount', { count: selectedBadges.length })}
                </span>
              )
              : selectedBadges.map((badge) => (
                <span key={badge.value} className={styles.tagFilterBadge}>{badge.label}</span>
              ))}
          </>
        )}
      </button>
    </Popover>
  );
};

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
    getRepoInfo,
    refresh,
  } = useSkills();

  const [searchText, setSearchText] = React.useState('');
  const [viewMode, setViewMode] = React.useState<SkillViewMode>('flat');
  const [groupModePreference, setGroupModePreference] =
    React.useState<SkillGroupModePreference | null>(null);
  const groupMode = resolveSkillGroupingMode(groupModePreference, groups.length);
  const [groupActiveKeys, setGroupActiveKeys] = React.useState<string[]>([]);
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = React.useState(false);
  const [reorderMode, setReorderMode] = React.useState(false);
  const [metadataSkill, setMetadataSkill] = React.useState<ManagedSkill | null>(null);
  const [detailSkillId, setDetailSkillId] = React.useState<string | null>(null);
  const [batchGroupModalOpen, setBatchGroupModalOpen] = React.useState(false);
  const [batchGroupValue, setBatchGroupValue] = React.useState('');
  const [groupsModalOpen, setGroupsModalOpen] = React.useState(false);
  const [inventoryModalOpen, setInventoryModalOpen] = React.useState(false);
  const [autoSyncModalOpen, setAutoSyncModalOpen] = React.useState(false);
  const [autoSyncEnabled, setAutoSyncEnabled] = React.useState(false);

  // The header button must show the live on/off state, so read the config here
  // (the dialog reads it again on open; both read the same backend settings).
  const loadAutoSyncState = React.useCallback(async () => {
    try {
      const config = await api.getAutoSync();
      setAutoSyncEnabled(config.enabled);
    } catch (error) {
      console.error('Failed to load auto sync state:', error);
    }
  }, []);

  React.useEffect(() => {
    void loadAutoSyncState();
  }, [loadAutoSyncState]);
  const [enabledFilter, setEnabledFilter] = React.useState<SkillEnabledFilter>('all');
  const [tagFilter, setTagFilter] = React.useState<string[]>([]);
  const [batchTagModalOpen, setBatchTagModalOpen] = React.useState(false);
  const [batchTagApplying, setBatchTagApplying] = React.useState(false);
  const [groupToolMode, setGroupToolMode] = React.useState(false);
  const [updatingAll, setUpdatingAll] = React.useState(false);
  const [updateAllProgress, setUpdateAllProgress] = React.useState<SkillsUpdateProgress | null>(null);
  const [gridColumnSetting, setGridColumnSetting] = React.useState<ManagementGridColumnSetting>('auto');
  const [preferredToolsForAddMore, setPreferredToolsForAddMore] = React.useState<string[]>([]);
  const [limitAddMoreToPreferredTools, setLimitAddMoreToPreferredTools] = React.useState(false);
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

  const loadSkillToolMenuPreferences = React.useCallback(async () => {
    try {
      const [savedPreferredTools, savedLimitAddMoreToPreferredTools] = await Promise.all([
        api.getPreferredTools(),
        api.getLimitAddMoreToPreferredTools(),
      ]);
      setPreferredToolsForAddMore(savedPreferredTools ?? []);
      setLimitAddMoreToPreferredTools(savedLimitAddMoreToPreferredTools);
    } catch (error) {
      console.error('Failed to load Skill tool menu preferences:', error);
    }
  }, []);

  React.useEffect(() => {
    void loadSkillToolMenuPreferences();
  }, [loadSkillToolMenuPreferences]);

  const handleToolMenuPreferencesChange = React.useCallback((preferences: {
    preferredTools: string[];
    limitAddMoreToPreferredTools: boolean;
  }) => {
    setPreferredToolsForAddMore(preferences.preferredTools);
    setLimitAddMoreToPreferredTools(preferences.limitAddMoreToPreferredTools);
  }, []);

  const handleViewModeChange = React.useCallback((mode: SkillViewMode) => {
    hasUserSelectedViewModeRef.current = true;
    setViewMode(mode);
  }, []);

  const handleDefaultViewModeApply = React.useCallback((mode: SkillViewMode) => {
    hasUserSelectedViewModeRef.current = true;
    setViewMode(mode);
  }, []);

  const handleUpdateAll = React.useCallback(() => {
    Modal.confirm({
      title: t('skills.updateAll.confirmTitle'),
      content: t('skills.updateAll.confirmContent'),
      okText: t('skills.updateAll.run'),
      cancelText: t('common.cancel'),
      // Fire-and-forget: return nothing (not a Promise) so the confirm dialog
      // closes immediately and yields to the progress modal. Awaiting here would
      // keep the confirm open (with a loading button) on top of the progress modal.
      onOk: () => {
        setUpdatingAll(true);
        setUpdateAllProgress(null);
        let unlisten: UnlistenFn | null = null;
        (async () => {
          try {
            unlisten = await listen<SkillsUpdateProgress>('skills-update-progress', (event) => {
              setUpdateAllProgress(event.payload);
            });
            const result = await api.updateAllSkills();
            await refresh();
            if (result.errors.length === 0) {
              message.success(t('skills.updateAll.success', { count: result.updated.length }));
            } else {
              message.warning(
                t('skills.updateAll.partial', {
                  updated: result.updated.length,
                  failed: result.errors.length,
                }),
              );
            }
          } catch (error) {
            message.error(String(error));
          } finally {
            unlisten?.();
            setUpdatingAll(false);
            setUpdateAllProgress(null);
          }
        })();
      },
    });
  }, [t, refresh]);

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
    handleBatchSetManagementEnabled,
    handleSetManagementEnabled,
  } = useSkillActions({ allTools });

  // Collect distinct tags across all managed skills (source for tag chips/datalists)
  const allTags = React.useMemo(() => collectAllTags(skills), [skills]);
  const hasUntaggedSkill = React.useMemo(
    () => skills.some((skill) => normalizeTagList(skill.tags ?? []).length === 0),
    [skills],
  );
  // Whether any tag chip (or the untagged chip) can be offered in the filter row
  const hasTagFilters = allTags.length > 0 || hasUntaggedSkill;

  // Options for the tag filter dropdown: untagged sentinel first (when
  // applicable), then every distinct tag, each with its skill count.
  const tagFilterOptions = React.useMemo<TagFilterOption[]>(() => {
    const options: TagFilterOption[] = [];
    if (hasUntaggedSkill) {
      const untaggedCount = skills.filter(
        (skill) => normalizeTagList(skill.tags ?? []).length === 0,
      ).length;
      options.push({ value: UNTAGGED_FILTER, label: t('skills.tags.untagged'), count: untaggedCount });
    }
    const tagCounts = new Map<string, number>();
    for (const skill of skills) {
      for (const tagItem of normalizeTagList(skill.tags ?? [])) {
        tagCounts.set(tagItem, (tagCounts.get(tagItem) ?? 0) + 1);
      }
    }
    for (const tagItem of allTags) {
      options.push({ value: tagItem, label: tagItem, count: tagCounts.get(tagItem) ?? 0 });
    }
    return options;
  }, [allTags, hasUntaggedSkill, skills, t]);

  // Filter skills by status, tag filter, then search text
  const filteredSkills = React.useMemo(() => {
    const byStatus = skills.filter((skill) => {
      if (enabledFilter === 'enabled') return skill.management_enabled;
      if (enabledFilter === 'disabled') return !skill.management_enabled;
      return true;
    });
    const byTags = byStatus.filter((skill) => matchesTagFilters(skill, tagFilter));
    return filterSkillsBySearch(byTags, deferredSearchText);
  }, [skills, deferredSearchText, enabledFilter, tagFilter]);

  // Drop stale tag filters when their tags disappear (e.g. skills removed)
  React.useEffect(() => {
    setTagFilter((prev) => pruneStaleTagFilters(prev, allTags, hasUntaggedSkill));
  }, [allTags, hasUntaggedSkill]);

  // Load the persisted grouped-view preference once. Until it arrives (and when
  // it is still "auto"), resolveSkillGroupingMode keeps the historical source
  // grouping for users who have no custom groups.
  React.useEffect(() => {
    let cancelled = false;
    api
      .getSkillGroupMode()
      .then((preference) => {
        if (!cancelled) {
          setGroupModePreference((current) => current ?? preference);
        }
      })
      .catch((error) => {
        console.error('Failed to load skill group mode:', error);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleGroupModeChange = React.useCallback((nextMode: SkillGroupingMode) => {
    setGroupModePreference(nextMode);
    api.setSkillGroupMode(nextMode).catch((error) => {
      console.error('Failed to save skill group mode:', error);
    });
  }, []);

  const isSearchActive = !!searchText.trim();
  const isFlatReorderEnabled = viewMode === 'flat' && reorderMode && !isSearchActive;
  const canUseGroupToolMode = viewMode === 'grouped' && groupMode === 'custom' && !isSearchActive;
  const groupOptions = React.useMemo(() => getSkillGroupOptions(groups), [groups]);

  React.useEffect(() => {
    if (viewMode !== 'flat' || isSearchActive) {
      setReorderMode(false);
    }
  }, [viewMode, isSearchActive]);

  // In flat view, reorder and selection are mutually exclusive: enabling one
  // turns the other off. SkillCard renders a checkbox in place of the drag
  // handle when selectable, so the two modes cannot coexist on one card.
  React.useEffect(() => {
    if (viewMode !== 'flat') return;
    if (selectionMode && reorderMode) {
      setReorderMode(false);
    }
  }, [viewMode, selectionMode, reorderMode]);

  React.useEffect(() => {
    if (!canUseGroupToolMode) {
      setGroupToolMode(false);
    }
  }, [canUseGroupToolMode]);

  // Keep selection scoped to the visible skills in either view. Stale ids
  // (removed skills, or skills hidden by filter/search) are pruned so the
  // batch toolbar never operates on invisible cards.
  React.useEffect(() => {
    setSelectedIds((prev) => {
      const allSkillIds = new Set(filteredSkills.map((s) => s.id));
      const next = new Set([...prev].filter((id) => allSkillIds.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [filteredSkills]);

  const handleToggleSelectionMode = React.useCallback(() => {
    if (selectionMode) {
      setSelectedIds(new Set());
    }
    setSelectionMode((previousSelectionMode) => !previousSelectionMode);
  }, [selectionMode]);

  // The "untagged" sentinel contradicts any concrete tag under AND semantics,
  // so selecting one side drops the other instead of yielding an empty list.
  const handleToggleTagFilter = React.useCallback((tag: string) => {
    setTagFilter((prev) => {
      if (prev.includes(tag)) {
        return prev.filter((existing) => existing !== tag);
      }
      const next = tag === UNTAGGED_FILTER
        ? prev.filter((existing) => existing === UNTAGGED_FILTER)
        : prev.filter((existing) => existing !== UNTAGGED_FILTER);
      return [...next, tag];
    });
  }, []);

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

  // Flat view has no group headers to host a per-group select-all, so the
  // batch toolbar exposes a global checkbox that selects every visible
  // (filtered) skill at once. Grouped view benefits from it too as a
  // cross-group shortcut alongside the per-group checkboxes.
  const handleSelectAllFiltered = React.useCallback((checked: boolean) => {
    setSelectedIds((prev) => {
      if (!checked) {
        return prev.size === 0 ? prev : new Set();
      }
      const next = new Set(prev);
      for (const skill of filteredSkills) {
        next.add(skill.id);
      }
      return next;
    });
  }, [filteredSkills]);

  const selectedArray = React.useMemo(() => [...selectedIds], [selectedIds]);
  const hasSelection = selectedArray.length > 0;
  const installedTools = React.useMemo(() => allTools.filter((tool) => tool.installed), [allTools]);
  const effectivePreferredToolsForAddMore = React.useMemo(() => {
    if (preferredToolsForAddMore.length > 0) {
      return preferredToolsForAddMore;
    }
    return allTools.filter((tool) => tool.installed).map((tool) => tool.id);
  }, [preferredToolsForAddMore, allTools]);
  const allToolIds = React.useMemo(() => new Set(allTools.map((tool) => tool.id)), [allTools]);
  const skillById = React.useMemo(
    () => new Map(skills.map((skill) => [skill.id, skill])),
    [skills],
  );
  const detailSkill = detailSkillId ? skillById.get(detailSkillId) ?? null : null;
  const handleOpenDetail = React.useCallback((skill: ManagedSkill) => {
    setDetailSkillId(skill.id);
  }, []);
  const handleCloseDetail = React.useCallback(() => {
    setDetailSkillId(null);
  }, []);
  const selectedSkills = React.useMemo(
    () => selectedArray
      .map((skillId) => skillById.get(skillId))
      .filter((skill): skill is ManagedSkill => Boolean(skill)),
    [selectedArray, skillById],
  );
  const selectedEnabledSkills = React.useMemo(
    () => selectedSkills.filter((skill) => skill.management_enabled),
    [selectedSkills],
  );
  const selectedDisabledSkills = React.useMemo(
    () => selectedSkills.filter((skill) => !skill.management_enabled),
    [selectedSkills],
  );

  const handleUpdateSkillsTags = React.useCallback(async (skillId: string, nextTags: string[]) => {
    const skill = skillById.get(skillId);
    if (!skill) {
      return;
    }
    try {
      await api.updateSkillMetadata(skill.id, skill.group_id, skill.user_note, nextTags);
      await refresh();
    } catch (error) {
      message.error(String(error));
    }
  }, [refresh, skillById]);

  const handleApplyBatchTags = React.useCallback(async (nextTags: string[]) => {
    if (selectedEnabledSkills.length === 0) {
      return;
    }
    setBatchTagApplying(true);
    try {
      for (const skill of selectedEnabledSkills) {
        const merged = normalizeTagList([...skill.tags, ...nextTags]);
        await api.updateSkillMetadata(skill.id, skill.group_id, skill.user_note, merged);
      }
      await refresh();
      setBatchTagModalOpen(false);
      setSelectedIds(new Set());
      message.success(t('skills.tags.applySuccess', { count: selectedEnabledSkills.length }));
    } catch (error) {
      message.error(String(error));
    } finally {
      setBatchTagApplying(false);
    }
  }, [refresh, selectedEnabledSkills, t]);

  const selectedEnabledToolCount = React.useMemo(
    () => selectedEnabledSkills.reduce((total, skill) => total + skill.enabled_tools.length, 0),
    [selectedEnabledSkills],
  );
  const gridColumns = gridColumnSetting === 'auto' ? undefined : gridColumnSetting;
  const batchAddToolItems = React.useMemo<ManagementMenuItem[]>(
    () => installedTools.map((tool) => ({
      key: `add-${tool.id}`,
      label: tool.label,
      icon: <ToolIcon toolKey={tool.id} label={tool.label} size={14} iconUrl={tool.iconUrl ?? undefined} />,
      onSelect: () => handleBatchAddTool(selectedArray, tool.id),
    })),
    [handleBatchAddTool, installedTools, selectedArray],
  );
  const batchRemoveToolItems = React.useMemo<ManagementMenuItem[]>(
    () => installedTools.map((tool) => ({
      key: `remove-${tool.id}`,
      label: tool.label,
      icon: <ToolIcon toolKey={tool.id} label={tool.label} size={14} iconUrl={tool.iconUrl ?? undefined} />,
      onSelect: () => handleBatchRemoveTool(selectedArray, tool.id),
    })),
    [handleBatchRemoveTool, installedTools, selectedArray],
  );
  const getRestorableToolIds = React.useCallback((skill: ManagedSkill) => {
    return skill.disabled_previous_tools.filter((toolId) => allToolIds.has(toolId));
  }, [allToolIds]);

  const buildRestoreToolsBySkillId = React.useCallback((targetSkills: ManagedSkill[]) => {
    const restoreToolsBySkillId: Record<string, string[]> = {};
    for (const skill of targetSkills) {
      const restorableToolIds = getRestorableToolIds(skill);
      if (restorableToolIds.length > 0) {
        restoreToolsBySkillId[skill.id] = restorableToolIds;
      }
    }
    return restoreToolsBySkillId;
  }, [getRestorableToolIds]);

  const selectedRestoreToolCount = React.useMemo(
    () => selectedDisabledSkills.reduce((total, skill) => total + getRestorableToolIds(skill).length, 0),
    [getRestorableToolIds, selectedDisabledSkills],
  );

  const handleBatchEnableSelected = React.useCallback(() => {
    if (selectedDisabledSkills.length === 0) {
      return;
    }

    Modal.confirm({
      title: t('skills.batch.enableConfirmTitle'),
      content: selectedRestoreToolCount > 0
        ? t('skills.batch.enableConfirmContent', {
          count: selectedDisabledSkills.length,
          tools: selectedRestoreToolCount,
        })
        : t('skills.batch.enableConfirmEmpty', { count: selectedDisabledSkills.length }),
      okText: t('skills.batch.enable'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        const saved = await handleBatchSetManagementEnabled(
          selectedDisabledSkills.map((skill) => skill.id),
          true,
          buildRestoreToolsBySkillId(selectedDisabledSkills),
        );
        if (saved) {
          setSelectedIds(new Set());
        }
      },
    });
  }, [
    buildRestoreToolsBySkillId,
    handleBatchSetManagementEnabled,
    selectedDisabledSkills,
    selectedRestoreToolCount,
    t,
  ]);

  const handleBatchDisableSelected = React.useCallback(() => {
    if (selectedEnabledSkills.length === 0) {
      return;
    }

    Modal.confirm({
      title: t('skills.batch.disableConfirmTitle'),
      content: t('skills.batch.disableConfirmContent', {
        count: selectedEnabledSkills.length,
        tools: selectedEnabledToolCount,
      }),
      okText: t('skills.batch.disable'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        const saved = await handleBatchSetManagementEnabled(
          selectedEnabledSkills.map((skill) => skill.id),
          false,
        );
        if (saved) {
          setSelectedIds(new Set());
        }
      },
    });
  }, [
    handleBatchSetManagementEnabled,
    selectedEnabledSkills,
    selectedEnabledToolCount,
    t,
  ]);

  const batchManagementStateItems = React.useMemo<ManagementMenuItem[]>(() => [
    {
      key: 'enable-selected',
      icon: <Power size={14} />,
      label: t('skills.batch.enable'),
      disabled: selectedDisabledSkills.length === 0,
      onSelect: handleBatchEnableSelected,
    },
    {
      key: 'disable-selected',
      danger: true,
      icon: <PowerOff size={14} />,
      label: t('skills.batch.disable'),
      disabled: selectedEnabledSkills.length === 0,
      onSelect: handleBatchDisableSelected,
    },
  ], [
    handleBatchDisableSelected,
    handleBatchEnableSelected,
    selectedDisabledSkills.length,
    selectedEnabledSkills.length,
    t,
  ]);

  const handleConfirmBatchGroup = React.useCallback(async () => {
    const normalizedGroupName = normalizeSkillMetadataText(batchGroupValue);
    const groupId = normalizedGroupName
      ? findSkillGroupOptionByName(groupOptions, normalizedGroupName)?.id
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
        groupCentral: t('skills.groupCentral'),
      },
      getRepoInfo,
      groups,
    );
  }, [filteredSkills, viewMode, groupMode, getRepoInfo, groups, t]);

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
    if (selectionMode) {
      states.push(t('skills.batch.bulkManage'));
    }
    if (tagFilter.length > 0) {
      states.push(t('skills.tags.activeFilter'));
    }
    if (viewMode === 'grouped' && groupToolMode) {
      states.push(t('skills.groupControls.groupTools'));
    }
    return states;
  }, [enabledFilter, groupMode, groupToolMode, reorderMode, selectionMode, tagFilter, t, viewMode]);

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
        <div className={styles.headerActions}>
          <ManagementButton
            variant="subtle"
            icon={<Radar size={14} aria-hidden="true" />}
            className={styles.autoSyncTrigger}
            onClick={() => setAutoSyncModalOpen(true)}
            title={t('skills.autoSync.title')}
          >
            <span className={styles.autoSyncTriggerLabel}>
              {t('skills.autoSync.entry')}
              <span
                className={`${styles.autoSyncDot}${autoSyncEnabled ? ` ${styles.autoSyncDotOn}` : ''}`}
                aria-hidden="true"
              />
            </span>
          </ManagementButton>
          <ManagementButton
            variant="ghost"
            icon={<MoreHorizontal size={16} aria-hidden="true" />}
            className={styles.moreMenuTrigger}
            onClick={() => setSettingsModalOpen(true)}
          >
            {t('skills.settings')}
          </ManagementButton>
        </div>
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
            variant="subtle"
            controlSize="compact"
            icon={<RefreshCw size={14} aria-hidden="true" />}
            disabled={updatingAll || actionLoading}
            onClick={handleUpdateAll}
          >
            {t('skills.updateAll.label')}
          </ManagementButton>
          <ManagementButton
            variant="primary"
            controlSize="compact"
            icon={<Plus size={14} aria-hidden="true" />}
            onClick={() => setAddModalOpen(true)}
          >
            {t('skills.addSkill')}
          </ManagementButton>
          {hasTagFilters && (
            <TagFilterDropdown
              options={tagFilterOptions}
              selected={tagFilter}
              onToggle={handleToggleTagFilter}
              onClear={() => setTagFilter([])}
            />
          )}
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
                    <>
                      <div className={styles.toolbarOptionRow}>
                        <span className={styles.toolbarOptionLabel}>{t('skills.toolbar.arrange')}</span>
                        <ManagementSegmented<'browse' | 'reorder'>
                          value={reorderMode ? 'reorder' : 'browse'}
                          ariaLabel={t('skills.reorder')}
                          title={isSearchActive ? t('skills.reorderDisabledWhileSearching') : t('skills.reorderHint')}
                          disabled={isSearchActive}
                          onChange={(nextMode) => {
                            const nextReorder = nextMode === 'reorder';
                            setReorderMode(nextReorder);
                            // Reorder and selection are mutually exclusive in
                            // flat view: the card renders either a drag handle
                            // or a selection checkbox in the same status slot.
                            if (nextReorder && selectionMode) {
                              setSelectionMode(false);
                              setSelectedIds(new Set());
                            }
                          }}
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
                      <div className={styles.toolbarOptionRow}>
                        <span className={styles.toolbarOptionLabel}>{t('skills.groupControls.selectionModeLabel')}</span>
                        <ManagementSegmented<'browse' | 'select'>
                          value={selectionMode ? 'select' : 'browse'}
                          ariaLabel={t('skills.groupControls.selectionModeLabel')}
                          onChange={(nextMode) => {
                            const nextSelect = nextMode === 'select';
                            if (nextSelect === selectionMode) return;
                            setSelectionMode(nextSelect);
                            if (nextSelect) {
                              setReorderMode(false);
                            } else {
                              setSelectedIds(new Set());
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
                    </>
                  )}
                  {viewMode === 'grouped' && (
                    <>
                      <div className={styles.toolbarOptionRow}>
                        <span className={styles.toolbarOptionLabel}>{t('skills.groupControls.groupingLabel')}</span>
                        <ManagementSegmented<SkillGroupingMode>
                          value={groupMode}
                          ariaLabel={t('skills.groupControls.groupingLabel')}
                          onChange={handleGroupModeChange}
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
                      icon={<Folders size={14} aria-hidden="true" />}
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
          {selectionMode && (
            <>
              <ManagementCheckbox
                checked={hasSelection && selectedArray.length === filteredSkills.length}
                indeterminate={hasSelection && selectedArray.length !== filteredSkills.length}
                ariaLabel={t('skills.batch.selectAll')}
                onChange={handleSelectAllFiltered}
                disabled={filteredSkills.length === 0 || loading || actionLoading}
              />
              <ManagementIconButton
                icon={<RefreshCw size={14} aria-hidden="true" />}
                title={hasSelection ? t('skills.batch.refresh') : t('skills.batch.noneSelected')}
                disabled={!hasSelection || loading || actionLoading}
                onClick={() => handleBatchRefresh(selectedArray)}
                controlSize="compact"
              />
              <ManagementMenu
                items={batchManagementStateItems}
                disabled={!hasSelection || loading || actionLoading}
                title={hasSelection ? t('skills.batch.managementState') : t('skills.batch.noneSelected')}
                controlSize="compact"
              >
                <Power size={14} aria-hidden="true" />
              </ManagementMenu>
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
                icon={<FolderInput size={14} aria-hidden="true" />}
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
              <ManagementIconButton
                icon={<Tag size={14} aria-hidden="true" />}
                title={hasSelection ? t('skills.tags.batchAdd') : t('skills.batch.noneSelected')}
                disabled={!hasSelection || loading || actionLoading}
                onClick={() => setBatchTagModalOpen(true)}
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
            dragDisabled={!isFlatReorderEnabled || selectionMode}
            selectionMode={selectionMode}
            selectedIds={selectedIds}
            onSelectChange={handleSelectChange}
            preferredToolKeysForAddMore={effectivePreferredToolsForAddMore}
            limitAddMoreToPreferredTools={limitAddMoreToPreferredTools}
            onOpenDetail={handleOpenDetail}
            getRepoInfo={getRepoInfo}
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
            preferredToolKeysForAddMore={effectivePreferredToolsForAddMore}
            limitAddMoreToPreferredTools={limitAddMoreToPreferredTools}
            activeKeys={groupActiveKeys}
            onActiveKeysChange={setGroupActiveKeys}
            selectionMode={selectionMode}
            selectedIds={selectedIds}
            onSelectChange={handleSelectChange}
            onSelectAllGroup={handleSelectAllGroup}
            onOpenDetail={handleOpenDetail}
            getRepoInfo={getRepoInfo}
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

      <Drawer
        placement="right"
        width="min(60vw, 760px)"
        open={!!detailSkill}
        onClose={handleCloseDetail}
        destroyOnHidden
        closable={false}
        styles={{
          body: {
            padding: 0,
            overflow: 'hidden',
            display: 'flex',
            flexDirection: 'column',
          },
        }}
      >
        {detailSkill && (
          <SkillDetailPanel
            skill={detailSkill}
            allTools={allTools}
            loading={loading || actionLoading}
            updatingSkillIds={updatingSkillIds}
            getRepoInfo={getRepoInfo}
            formatRelative={formatRelative}
            onClose={handleCloseDetail}
            onToggleTool={handleToggleTool}
            onUpdate={handleUpdate}
            onDelete={(skillId) => {
              setDetailSkillId(null);
              handleDelete(skillId);
            }}
            onSetManagementEnabled={handleSetSkillEnabled}
            allTags={allTags}
            onUpdateTags={handleUpdateSkillsTags}
            onEditMetadata={setMetadataSkill}
          />
        )}
      </Drawer>

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
          onToolMenuPreferencesChange={handleToolMenuPreferencesChange}
          onClose={() => setSettingsModalOpen(false)}
        />
      )}

      <SkillsAutoSyncModal
        open={autoSyncModalOpen}
        onClose={() => setAutoSyncModalOpen(false)}
        onConfigSaved={loadAutoSyncState}
      />

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

      <BatchTagDialog
        open={batchTagModalOpen}
        skillCount={selectedEnabledSkills.length}
        skippedCount={selectedDisabledSkills.length}
        allTags={allTags}
        applying={batchTagApplying}
        onCancel={() => setBatchTagModalOpen(false)}
        onApply={handleApplyBatchTags}
      />

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

      <Modal
        open={updatingAll}
        title={t('skills.updateAll.progressTitle')}
        closable={false}
        footer={null}
        width={420}
      >
        <Progress
          percent={
            updateAllProgress && updateAllProgress.total > 0
              ? Math.min(100, Math.round((updateAllProgress.current / updateAllProgress.total) * 100))
              : 0
          }
          status="active"
        />
        <div style={{ marginTop: 12 }}>
          {updateAllProgress && updateAllProgress.current > 0
            ? t('skills.updateAll.updating', {
                name: updateAllProgress.name,
                current: updateAllProgress.current,
                total: updateAllProgress.total,
              })
            : t('skills.updateAll.pending', { total: updateAllProgress?.total ?? 0 })}
        </div>
      </Modal>
    </div>
  );
};

export default SkillsPage;
