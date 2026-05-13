import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  SortableContext,
  sortableKeyboardCoordinates,
  rectSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToWindowEdges } from '@dnd-kit/modifiers';
import { ManagementEmpty, VirtualGrid } from '@/features/coding/shared/management';
import { SkillCard } from './SkillCard';
import type { ManagedSkill, ToolOption } from '../types';
import styles from './SkillsList.module.less';

interface SkillsListProps {
  skills: ManagedSkill[];
  allTools: ToolOption[];
  loading: boolean;
  updatingSkillIds: string[];
  columns?: number;
  dragDisabled?: boolean;
  getGithubInfo: (url: string | null | undefined) => { label: string; href: string } | null;
  getSkillSourceLabel: (skill: ManagedSkill) => string;
  formatRelative: (ms: number | null | undefined) => string;
  onUpdate: (skill: ManagedSkill) => void;
  onDelete: (skillId: string) => void;
  onToggleTool: (skill: ManagedSkill, toolId: string) => void;
  onEditMetadata: (skill: ManagedSkill) => void;
  onSetManagementEnabled: (skill: ManagedSkill, enabled: boolean) => void;
  onDragEnd: (event: DragEndEvent) => void;
}

export const SkillsList: React.FC<SkillsListProps> = ({
  skills,
  allTools,
  loading,
  updatingSkillIds,
  columns,
  dragDisabled,
  getGithubInfo,
  getSkillSourceLabel,
  formatRelative,
  onUpdate,
  onDelete,
  onToggleTool,
  onEditMetadata,
  onSetManagementEnabled,
  onDragEnd,
}) => {
  const { t } = useTranslation();

  // Configure drag sensors
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8, // Prevent accidental drags
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  if (skills.length === 0) {
    return (
      <div className={styles.empty}>
        <ManagementEmpty description={t('skills.skillsEmpty')} />
      </div>
    );
  }

  const cardList = (
    <div
      className={[
        styles.list,
        columns === undefined ? styles.listAuto : styles.listFixed,
      ].filter(Boolean).join(' ')}
      style={columns === undefined ? undefined : ({
        '--management-grid-columns': `repeat(${columns}, minmax(0, 1fr))`,
      } as React.CSSProperties)}
    >
      {skills.map((skill) => (
        <SkillCard
          key={skill.id}
          skill={skill}
          allTools={allTools}
          loading={loading}
          isUpdating={updatingSkillIds.includes(skill.id)}
          dragDisabled={dragDisabled}
          getGithubInfo={getGithubInfo}
          getSkillSourceLabel={getSkillSourceLabel}
          formatRelative={formatRelative}
          onUpdate={onUpdate}
          onDelete={onDelete}
          onToggleTool={onToggleTool}
          onEditMetadata={onEditMetadata}
          onSetManagementEnabled={onSetManagementEnabled}
        />
      ))}
    </div>
  );

  if (dragDisabled) {
    return (
      <VirtualGrid
        items={skills}
        getKey={(skill) => skill.id}
        columns={columns}
        minColumnWidth={360}
        maxColumns={3}
        rowGap={14}
        defaultRowHeight={176}
        renderItem={(skill) => (
          <SkillCard
            skill={skill}
            allTools={allTools}
            loading={loading}
            isUpdating={updatingSkillIds.includes(skill.id)}
            dragDisabled
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
    );
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      modifiers={[restrictToWindowEdges]}
      onDragEnd={onDragEnd}
    >
      <SortableContext
        items={skills.map((s) => s.id)}
        strategy={rectSortingStrategy}
      >
        {cardList}
      </SortableContext>
    </DndContext>
  );
};
