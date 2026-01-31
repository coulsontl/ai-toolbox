import React from 'react';
import { Modal, Button, Space, message } from 'antd';
import { PlusOutlined, UserOutlined, ImportOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { arrayMove } from '@dnd-kit/sortable';
import type { DragEndEvent } from '@dnd-kit/core';
import { useSkillsStore } from '../stores/skillsStore';
import { useSkills } from '../hooks/useSkills';
import { SkillsList } from './SkillsList';
import { AddSkillModal } from './modals/AddSkillModal';
import { ImportModal } from './modals/ImportModal';
import { SkillsSettingsModal } from './modals/SkillsSettingsModal';
import { DeleteConfirmModal } from './modals/DeleteConfirmModal';
import { NewToolsModal } from './modals/NewToolsModal';
import * as api from '../services/skillsApi';
import styles from './SkillsModal.module.less';

interface SkillsModalProps {
  open?: boolean;
  onClose?: () => void;
}

export const SkillsModal: React.FC<SkillsModalProps> = ({ open, onClose }) => {
  const { t } = useTranslation();
  const {
    isModalOpen,
    setModalOpen,
    isAddModalOpen,
    setAddModalOpen,
    isImportModalOpen,
    setImportModalOpen,
    isSettingsModalOpen,
    setSettingsModalOpen,
    isNewToolsModalOpen,
    onboardingPlan,
    loading,
  } = useSkillsStore();

  // Use props if provided, otherwise use store state
  const isOpen = open !== undefined ? open : isModalOpen;
  const handleClose = () => {
    if (onClose) {
      onClose();
    } else {
      setModalOpen(false);
    }
  };

  const {
    skills,
    getAllTools,
    formatRelative,
    getGithubInfo,
    getSkillSourceLabel,
    updateSkill,
    deleteSkill,
    refresh,
    setSkills,
  } = useSkills();

  const [deleteSkillId, setDeleteSkillId] = React.useState<string | null>(null);
  const [actionLoading, setActionLoading] = React.useState(false);

  const allTools = getAllTools();
  const skillToDelete = deleteSkillId
    ? skills.find((s) => s.id === deleteSkillId)
    : null;

  const discoveredCount = onboardingPlan?.total_skills_found || 0;

  const handleToggleTool = async (skill: typeof skills[0], toolId: string) => {
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
    } catch (error) {
      const errMsg = String(error);
      if (errMsg.includes('TARGET_EXISTS|')) {
        const match = errMsg.match(/TARGET_EXISTS\|(.+)/);
        const targetPath = match ? match[1] : '';
        const toolLabel = allTools.find((t) => t.id === toolId)?.label || toolId;
        Modal.confirm({
          title: t('skills.targetExists.title'),
          content: t('skills.targetExists.message', { skill: skill.name, tool: toolLabel, path: targetPath }),
          okText: t('skills.overwrite.confirm'),
          okType: 'danger',
          cancelText: t('common.cancel'),
          onOk: async () => {
            try {
              await api.syncSkillToTool(skill.central_path, skill.id, toolId, skill.name, true);
              await refresh();
            } catch (retryError) {
              message.error(String(retryError));
            }
          },
        });
      } else {
        message.error(errMsg);
      }
    } finally {
      setActionLoading(false);
    }
  };

  const handleUpdate = async (skill: typeof skills[0]) => {
    setActionLoading(true);
    try {
      await updateSkill(skill);
    } catch (error) {
      console.error('Failed to update skill:', error);
    } finally {
      setActionLoading(false);
    }
  };

  const handleDelete = (skillId: string) => {
    setDeleteSkillId(skillId);
  };

  const confirmDelete = async () => {
    if (!deleteSkillId) return;
    setActionLoading(true);
    try {
      await deleteSkill(deleteSkillId);
      setDeleteSkillId(null);
    } catch (error) {
      console.error('Failed to delete skill:', error);
    } finally {
      setActionLoading(false);
    }
  };

  const handleDragEnd = async (event: DragEndEvent) => {
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
    } catch (error) {
      // Rollback on error
      console.error('Failed to reorder skills:', error);
      setSkills(oldSkills);
      message.error(t('common.error'));
    }
  };

  return (
    <>
      <Modal
        title={t('skills.title')}
        open={isOpen}
        onCancel={handleClose}
        footer={null}
        width={900}
        className={styles.skillsModal}
        destroyOnClose
      >
        <div className={styles.header}>
          <Space>
            <Button
              type="primary"
              icon={<PlusOutlined />}
              onClick={() => setAddModalOpen(true)}
            >
              {t('skills.newSkill')}
            </Button>
            {discoveredCount > 0 && (
              <Button icon={<ImportOutlined />} onClick={() => setImportModalOpen(true)}>
                {t('skills.reviewImport')} ({discoveredCount})
              </Button>
            )}
          </Space>
          <Button
            icon={<UserOutlined />}
            onClick={() => setSettingsModalOpen(true)}
          >
            {t('skills.settings')}
          </Button>
        </div>

        <div className={styles.content}>
          <SkillsList
            skills={skills}
            allTools={allTools}
            loading={loading || actionLoading}
            getGithubInfo={getGithubInfo}
            getSkillSourceLabel={getSkillSourceLabel}
            formatRelative={formatRelative}
            onUpdate={handleUpdate}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onDragEnd={handleDragEnd}
          />
        </div>
      </Modal>

      <AddSkillModal
        open={isAddModalOpen}
        onClose={() => setAddModalOpen(false)}
        allTools={allTools}
        onSuccess={() => {
          setAddModalOpen(false);
          refresh();
        }}
      />

      <ImportModal
        open={isImportModalOpen}
        onClose={() => setImportModalOpen(false)}
        onSuccess={() => {
          setImportModalOpen(false);
          refresh();
        }}
      />

      <SkillsSettingsModal
        open={isSettingsModalOpen}
        onClose={() => setSettingsModalOpen(false)}
      />

      <DeleteConfirmModal
        open={!!deleteSkillId}
        skillName={skillToDelete?.name || ''}
        onClose={() => setDeleteSkillId(null)}
        onConfirm={confirmDelete}
        loading={actionLoading}
      />

      <NewToolsModal
        open={isNewToolsModalOpen}
      />
    </>
  );
};
