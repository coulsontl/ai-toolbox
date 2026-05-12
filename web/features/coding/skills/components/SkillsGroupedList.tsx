import React from 'react';
import { ChevronDown, Plus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  ManagementCheckbox,
  ManagementEmpty,
  ManagementMenu,
  VirtualGrid,
  type ManagementMenuItem,
} from '@/features/coding/shared/management';
import { SkillCard } from './SkillCard';
import type { SkillGroup, ToolOption, ManagedSkill } from '../types';
import { getSkillGroupToolIds, isSkillUngroupedCustomGroup } from '../utils/skillGrouping';
import styles from './SkillsGroupedList.module.less';

interface SkillsGroupedListProps {
  groups: SkillGroup[];
  allTools: ToolOption[];
  loading: boolean;
  updatingSkillIds: string[];
  columns?: number;
  activeKeys: string[];
  onActiveKeysChange: (keys: string[]) => void;
  selectionMode: boolean;
  selectedIds: Set<string>;
  onSelectChange: (skillId: string, checked: boolean) => void;
  onSelectAllGroup: (group: SkillGroup, checked: boolean) => void;
  getGithubInfo: (url: string | null | undefined) => { label: string; href: string } | null;
  getSkillSourceLabel: (skill: ManagedSkill) => string;
  formatRelative: (ms: number | null | undefined) => string;
  onUpdate: (skill: ManagedSkill) => void;
  onDelete: (skillId: string) => void;
  onToggleTool: (skill: ManagedSkill, toolId: string) => void;
  onEditMetadata: (skill: ManagedSkill) => void;
  onSetManagementEnabled: (skill: ManagedSkill, enabled: boolean) => void;
  groupToolMode?: boolean;
  onAddGroupTool?: (group: SkillGroup, toolId: string) => void;
  onRemoveGroupTool?: (group: SkillGroup, toolId: string) => void;
}

export const SkillsGroupedList: React.FC<SkillsGroupedListProps> = ({
  groups,
  allTools,
  loading,
  updatingSkillIds,
  columns,
  activeKeys,
  onActiveKeysChange,
  selectionMode,
  selectedIds,
  onSelectChange,
  onSelectAllGroup,
  getGithubInfo,
  getSkillSourceLabel,
  formatRelative,
  onUpdate,
  onDelete,
  onToggleTool,
  onEditMetadata,
  onSetManagementEnabled,
  groupToolMode = false,
  onAddGroupTool,
  onRemoveGroupTool,
}) => {
  const { t } = useTranslation();
  const activeKeySet = React.useMemo(() => new Set(activeKeys), [activeKeys]);

  if (groups.length === 0) {
    return (
      <div className={styles.empty}>
        <ManagementEmpty description={t('skills.skillsEmpty')} />
      </div>
    );
  }

  const isGroupAllSelected = (group: SkillGroup) =>
    group.skills.length > 0 && group.skills.every((s) => selectedIds.has(s.id));

  const isGroupPartialSelected = (group: SkillGroup) =>
    group.skills.some((s) => selectedIds.has(s.id)) && !isGroupAllSelected(group);

  const renderGroupTools = (group: SkillGroup) => {
    const activeToolIds = new Set(getSkillGroupToolIds(group));
    const activeTools = allTools.filter((tool) => activeToolIds.has(tool.id));
    const availableTools = allTools.filter((tool) => tool.installed && !activeToolIds.has(tool.id));
    const availableToolItems: ManagementMenuItem[] = availableTools.map((tool) => ({
      key: tool.id,
      label: tool.label,
      onSelect: () => onAddGroupTool?.(group, tool.id),
    }));

    return (
      <div className={styles.groupTools}>
        {activeTools.map((tool) => (
          <button
            key={tool.id}
            title={t('skills.groupTools.removeTool', { tool: tool.label })}
            type="button"
            className={styles.groupToolPill}
            disabled={loading}
            onClick={() => onRemoveGroupTool?.(group, tool.id)}
          >
            <span className={styles.statusBadge} />
            {tool.label}
          </button>
        ))}
        {availableTools.length > 0 && (
          <ManagementMenu
            items={availableToolItems}
            disabled={loading}
            title={t('common.add')}
            triggerClassName={styles.groupToolAdd}
            controlSize="compact"
          >
            <Plus size={13} aria-hidden="true" />
          </ManagementMenu>
        )}
      </div>
    );
  };

  const handleToggleGroup = (groupKey: string) => {
    const nextKeys = activeKeySet.has(groupKey)
      ? activeKeys.filter((key) => key !== groupKey)
      : [...activeKeys, groupKey];
    onActiveKeysChange(nextKeys);
  };

  return (
    <div className={styles.groupedList}>
      {groups.map((group) => {
        const groupToolsEnabled = groupToolMode && !isSkillUngroupedCustomGroup(group);
        const isOpen = activeKeySet.has(group.key);

        return (
          <section key={group.key} className={styles.groupSection}>
            <div className={styles.groupHeader}>
              <div className={styles.groupTitle}>
                {selectionMode && (
                  <ManagementCheckbox
                    checked={isGroupAllSelected(group)}
                    indeterminate={isGroupPartialSelected(group)}
                    ariaLabel={`${t('skills.batch.selectAll')} ${group.label}`}
                    onChange={(checked) => onSelectAllGroup(group, checked)}
                  />
                )}
                <button
                  type="button"
                  className={styles.groupToggle}
                  aria-expanded={isOpen}
                  onClick={() => handleToggleGroup(group.key)}
                >
                  <ChevronDown
                    size={15}
                    className={`${styles.groupChevron}${isOpen ? ` ${styles.groupChevronOpen}` : ''}`}
                    aria-hidden="true"
                  />
                  <span className={styles.groupLabel}>{group.label}</span>
                  <span className={styles.groupCount}>
                    {t('skills.skillCount', { count: group.skills.length })}
                  </span>
                </button>
                {group.note && <span className={styles.groupNote} title={group.note}>{group.note}</span>}
              </div>
              {groupToolsEnabled && renderGroupTools(group)}
            </div>
            {isOpen && (
              <div className={styles.groupBody}>
                <VirtualGrid
                  items={group.skills}
                  getKey={(skill) => skill.id}
                  columns={columns}
                  defaultRowHeight={84}
                  virtualize={group.skills.length > 24}
                  renderItem={(skill) => (
                    <SkillCard
                      skill={skill}
                      allTools={allTools}
                      loading={loading}
                      isUpdating={updatingSkillIds.includes(skill.id)}
                      dragDisabled
                      selectable={selectionMode}
                      selected={selectedIds.has(skill.id)}
                      toolsReadOnly={groupToolsEnabled}
                      onSelectChange={onSelectChange}
                      getGithubInfo={getGithubInfo}
                      getSkillSourceLabel={getSkillSourceLabel}
                      formatRelative={formatRelative}
                      onUpdate={onUpdate}
                      onDelete={onDelete}
                      onToggleTool={onToggleTool}
                      onEditMetadata={onEditMetadata}
                      onSetManagementEnabled={onSetManagementEnabled}
                    />
                  )}
                />
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
};
