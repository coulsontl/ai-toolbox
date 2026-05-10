import React from 'react';
import { Collapse, Empty, Checkbox } from 'antd';
import { useTranslation } from 'react-i18next';
import { SkillCard } from './SkillCard';
import type { SkillGroup, ToolOption, ManagedSkill } from '../types';
import styles from './SkillsGroupedList.module.less';

interface SkillsGroupedListProps {
  groups: SkillGroup[];
  allTools: ToolOption[];
  loading: boolean;
  updatingSkillIds: string[];
  activeKeys: string[];
  onActiveKeysChange: (keys: string[]) => void;
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
}

export const SkillsGroupedList: React.FC<SkillsGroupedListProps> = ({
  groups,
  allTools,
  loading,
  updatingSkillIds,
  activeKeys,
  onActiveKeysChange,
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
}) => {
  const { t } = useTranslation();

  if (groups.length === 0) {
    return (
      <div className={styles.empty}>
        <Empty description={t('skills.skillsEmpty')} />
      </div>
    );
  }

  const isGroupAllSelected = (group: SkillGroup) =>
    group.skills.length > 0 && group.skills.every((s) => selectedIds.has(s.id));

  const isGroupPartialSelected = (group: SkillGroup) =>
    group.skills.some((s) => selectedIds.has(s.id)) && !isGroupAllSelected(group);

  const items = groups.map((group) => ({
    key: group.key,
    label: (
      <div className={styles.groupHeader}>
        <Checkbox
          checked={isGroupAllSelected(group)}
          indeterminate={isGroupPartialSelected(group)}
          onChange={(e) => {
            e.stopPropagation();
            onSelectAllGroup(group, e.target.checked);
          }}
          onClick={(e) => e.stopPropagation()}
        />
        <span className={styles.groupLabel}>
          {group.label}
          <span className={styles.groupCount}>
            ({t('skills.skillCount', { count: group.skills.length })})
          </span>
        </span>
      </div>
    ),
    children: (
      <div className={styles.groupGrid}>
        {group.skills.map((skill) => (
          <SkillCard
            key={skill.id}
            skill={skill}
            allTools={allTools}
            loading={loading}
            isUpdating={updatingSkillIds.includes(skill.id)}
            dragDisabled
            selectable
            selected={selectedIds.has(skill.id)}
            onSelectChange={onSelectChange}
            getGithubInfo={getGithubInfo}
            getSkillSourceLabel={getSkillSourceLabel}
            formatRelative={formatRelative}
            onUpdate={onUpdate}
            onDelete={onDelete}
            onToggleTool={onToggleTool}
            onEditMetadata={onEditMetadata}
          />
        ))}
      </div>
    ),
  }));

  return (
    <div className={styles.groupedList}>
      <Collapse
        activeKey={activeKeys}
        onChange={(keys) => onActiveKeysChange(keys as string[])}
        items={items}
      />
    </div>
  );
};
