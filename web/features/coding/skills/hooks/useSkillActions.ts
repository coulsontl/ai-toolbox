import React from 'react';
import { message } from 'antd';
import { arrayMove } from '@dnd-kit/sortable';
import type { DragEndEvent } from '@dnd-kit/core';
import { useTranslation } from 'react-i18next';
import * as api from '../services/skillsApi';
import { useSkills } from './useSkills';
import type { ManagedSkill, ToolOption } from '../types';
import { showGitError, confirmTargetOverwrite } from '../utils/errorHandlers';
import { refreshTrayMenu } from '@/services/appApi';

export interface UseSkillActionsOptions {
  allTools: ToolOption[];
}

export interface UseSkillActionsResult {
  actionLoading: boolean;
  updatingSkillIds: string[];
  deleteSkillId: string | null;
  setDeleteSkillId: (id: string | null) => void;
  skillToDelete: ManagedSkill | undefined;
  batchDeleteIds: string[];
  setBatchDeleteIds: (ids: string[]) => void;
  handleToggleTool: (skill: ManagedSkill, toolId: string) => Promise<void>;
  handleUpdate: (skill: ManagedSkill) => Promise<void>;
  handleDelete: (skillId: string) => void;
  confirmDelete: () => Promise<void>;
  handleDragEnd: (event: DragEndEvent) => Promise<void>;
  handleBatchRefresh: (skillIds: string[]) => Promise<void>;
  handleBatchDelete: (skillIds: string[]) => void;
  confirmBatchDelete: () => Promise<void>;
  handleBatchAddTool: (skillIds: string[], toolId: string) => Promise<void>;
  handleBatchRemoveTool: (skillIds: string[], toolId: string) => Promise<void>;
  handleBatchSetGroup: (skillIds: string[], userGroup: string | null) => Promise<boolean>;
}

export function useSkillActions({ allTools }: UseSkillActionsOptions): UseSkillActionsResult {
  const { t } = useTranslation();
  const { skills, refresh, updateSkill, deleteSkill, setSkills } = useSkills();

  const [deleteSkillId, setDeleteSkillId] = React.useState<string | null>(null);
  const [batchDeleteIds, setBatchDeleteIds] = React.useState<string[]>([]);
  const [actionLoading, setActionLoading] = React.useState(false);
  const [updatingSkillIds, setUpdatingSkillIds] = React.useState<string[]>([]);

  const skillToDelete = deleteSkillId
    ? skills.find((s) => s.id === deleteSkillId)
    : undefined;

  const handleToggleTool = React.useCallback(async (skill: ManagedSkill, toolId: string) => {
    const target = skill.targets.find((t) => t.tool === toolId);
    const synced = Boolean(target);

    setActionLoading(true);
    try {
      if (synced) {
        await api.unsyncSkillFromTool(skill.id, toolId);
      } else {
        await api.syncSkillToTool(skill.central_path, skill.id, toolId, skill.name);
      }
      await refresh();
      await refreshTrayMenu();
    } catch (error) {
      const errMsg = String(error);
      if (errMsg.includes('TARGET_EXISTS|')) {
        const match = errMsg.match(/TARGET_EXISTS\|(.+)/);
        const targetPath = match ? match[1] : '';
        const toolLabel = allTools.find((t) => t.id === toolId)?.label || toolId;
        const shouldOverwrite = await confirmTargetOverwrite(skill.name, toolLabel, targetPath, t);
        if (shouldOverwrite) {
          try {
            await api.syncSkillToTool(skill.central_path, skill.id, toolId, skill.name, true);
            await refresh();
            await refreshTrayMenu();
          } catch (retryError) {
            message.error(String(retryError));
          }
        }
      } else {
        showGitError(errMsg, t, allTools);
      }
    } finally {
      setActionLoading(false);
    }
  }, [allTools, t, refresh]);

  const handleUpdate = React.useCallback(async (skill: ManagedSkill) => {
    if (updatingSkillIds.includes(skill.id)) {
      return;
    }

    setUpdatingSkillIds((prev) => [...prev, skill.id]);
    try {
      await updateSkill(skill);
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setUpdatingSkillIds((prev) => prev.filter((id) => id !== skill.id));
    }
  }, [allTools, t, updateSkill, updatingSkillIds]);

  const handleDelete = React.useCallback((skillId: string) => {
    setDeleteSkillId(skillId);
  }, []);

  const confirmDelete = React.useCallback(async () => {
    if (!deleteSkillId) return;
    setActionLoading(true);
    try {
      await deleteSkill(deleteSkillId);
      setDeleteSkillId(null);
      await refreshTrayMenu();
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setActionLoading(false);
    }
  }, [deleteSkillId, deleteSkill, t, allTools]);

  const handleDragEnd = React.useCallback(async (event: DragEndEvent) => {
    const { active, over } = event;

    if (!over || active.id === over.id) {
      return;
    }

    const oldIndex = skills.findIndex((s) => s.id === active.id);
    const newIndex = skills.findIndex((s) => s.id === over.id);

    if (oldIndex === -1 || newIndex === -1) {
      return;
    }

    // Optimistic update
    const oldSkills = [...skills];
    const newSkills = arrayMove(skills, oldIndex, newIndex);
    setSkills(newSkills);

    try {
      await api.reorderSkills(newSkills.map((s) => s.id));
      await refreshTrayMenu();
    } catch (error) {
      // Rollback on error
      console.error('Failed to reorder skills:', error);
      setSkills(oldSkills);
      message.error(t('common.error'));
    }
  }, [skills, setSkills, t]);

  // Batch refresh
  const handleBatchRefresh = React.useCallback(async (skillIds: string[]) => {
    setActionLoading(true);
    try {
      for (const id of skillIds) {
        await api.updateManagedSkill(id);
      }
      await refresh();
      message.success(t('skills.batch.refreshSuccess', { count: skillIds.length }));
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setActionLoading(false);
    }
  }, [refresh, t, allTools]);

  // Batch delete - trigger confirmation
  const handleBatchDelete = React.useCallback((skillIds: string[]) => {
    setBatchDeleteIds(skillIds);
  }, []);

  // Batch delete - confirm
  const confirmBatchDelete = React.useCallback(async () => {
    if (batchDeleteIds.length === 0) return;
    setActionLoading(true);
    try {
      for (const id of batchDeleteIds) {
        await api.deleteManagedSkill(id);
      }
      await refresh();
      await refreshTrayMenu();
      message.success(t('skills.batch.deleteSuccess', { count: batchDeleteIds.length }));
      setBatchDeleteIds([]);
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setActionLoading(false);
    }
  }, [batchDeleteIds, refresh, t, allTools]);

  // Batch add tool sync
  const handleBatchAddTool = React.useCallback(async (skillIds: string[], toolId: string) => {
    setActionLoading(true);
    let successCount = 0;
    try {
      for (const id of skillIds) {
        const skill = skills.find((s) => s.id === id);
        if (!skill) continue;
        const alreadySynced = skill.targets.some((t) => t.tool === toolId);
        if (alreadySynced) continue;
        await api.syncSkillToTool(skill.central_path, skill.id, toolId, skill.name);
        successCount++;
      }
      await refresh();
      await refreshTrayMenu();
      const toolLabel = allTools.find((t) => t.id === toolId)?.label || toolId;
      if (successCount > 0) {
        message.success(t('skills.batch.addToolSuccess', { count: successCount, tool: toolLabel }));
      }
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setActionLoading(false);
    }
  }, [skills, refresh, t, allTools]);

  // Batch remove tool sync
  const handleBatchRemoveTool = React.useCallback(async (skillIds: string[], toolId: string) => {
    setActionLoading(true);
    let successCount = 0;
    try {
      for (const id of skillIds) {
        const skill = skills.find((s) => s.id === id);
        if (!skill) continue;
        const isSynced = skill.targets.some((t) => t.tool === toolId);
        if (!isSynced) continue;
        await api.unsyncSkillFromTool(id, toolId);
        successCount++;
      }
      await refresh();
      await refreshTrayMenu();
      const toolLabel = allTools.find((t) => t.id === toolId)?.label || toolId;
      if (successCount > 0) {
        message.success(t('skills.batch.removeToolSuccess', { count: successCount, tool: toolLabel }));
      }
    } catch (error) {
      showGitError(String(error), t, allTools);
    } finally {
      setActionLoading(false);
    }
  }, [skills, refresh, t, allTools]);

  const handleBatchSetGroup = React.useCallback(async (
    skillIds: string[],
    userGroup: string | null,
  ) => {
    if (skillIds.length === 0) {
      return false;
    }

    setActionLoading(true);
    try {
      await api.batchUpdateSkillGroup(skillIds, userGroup);
      await refresh();
      message.success(t('skills.batch.setGroupSuccess', { count: skillIds.length }));
      return true;
    } catch (error) {
      message.error(String(error));
      return false;
    } finally {
      setActionLoading(false);
    }
  }, [refresh, t]);

  return {
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
  };
}
