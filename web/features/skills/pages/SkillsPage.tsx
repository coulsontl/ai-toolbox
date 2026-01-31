import React from 'react';
import { Typography, Button, Space, Modal, message } from 'antd';
import { PlusOutlined, UserOutlined, ImportOutlined, LinkOutlined } from '@ant-design/icons';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { arrayMove } from '@dnd-kit/sortable';
import type { DragEndEvent } from '@dnd-kit/core';
import { useSkillsStore } from '../stores/skillsStore';
import { useSkills } from '../hooks/useSkills';
import { SkillsList } from '../components/SkillsList';
import { AddSkillModal } from '../components/modals/AddSkillModal';
import { ImportModal } from '../components/modals/ImportModal';
import { SkillsSettingsModal } from '../components/modals/SkillsSettingsModal';
import { DeleteConfirmModal } from '../components/modals/DeleteConfirmModal';
import { NewToolsModal } from '../components/modals/NewToolsModal';
import { formatGitError, isGitError } from '../utils/gitErrorParser';
import * as api from '../services/skillsApi';
import styles from './SkillsPage.module.less';

const { Title, Link } = Typography;

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
    onboardingPlan,
    loading,
  } = useSkillsStore();

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

  // Initialize data on mount
  React.useEffect(() => {
    refresh();
  }, []);

  const allTools = getAllTools();
  const skillToDelete = deleteSkillId
    ? skills.find((s) => s.id === deleteSkillId)
    : null;

  const discoveredCount = onboardingPlan?.total_skills_found || 0;

  const showGitError = (errMsg: string) => {
    // Handle TOOL_NOT_INSTALLED|toolKey|skillsPath error
    if (errMsg.startsWith('TOOL_NOT_INSTALLED|')) {
      const parts = errMsg.split('|');
      const toolKey = parts[1] || '';
      const skillsPath = parts[2] || '';
      const tool = allTools.find((t) => t.id === toolKey);
      const toolName = tool?.label || toolKey;
      Modal.error({
        title: t('common.error'),
        content: (
          <div>
            <p>{t('skills.errors.toolNotInstalled', { tool: toolName })}</p>
            <p style={{ fontSize: 12, color: 'var(--color-text-tertiary)' }}>
              {t('skills.errors.checkSkillsPath', { path: skillsPath })}
            </p>
          </div>
        ),
      });
      return;
    }

    if (isGitError(errMsg)) {
      Modal.error({
        title: t('common.error'),
        content: (
          <div style={{ whiteSpace: 'pre-wrap', maxHeight: '400px', overflow: 'auto' }}>
            {formatGitError(errMsg, t)}
          </div>
        ),
        width: 600,
      });
    } else {
      message.error(errMsg);
    }
  };

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
        showGitError(errMsg);
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
      showGitError(String(error));
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
      showGitError(String(error));
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
    <div className={styles.skillsPage}>
      <div className={styles.pageHeader}>
        <div>
          <Title level={4} style={{ margin: 0, display: 'inline-block', marginRight: 8 }}>
            {t('skills.title')}
          </Title>
          <Link
            type="secondary"
            style={{ fontSize: 12 }}
            onClick={(e) => {
              e.stopPropagation();
              openUrl('https://code.claude.com/docs/en/skills');
            }}
          >
            <LinkOutlined /> {t('skills.viewDocs')}
          </Link>
        </div>
        <Button
          type="text"
          icon={<UserOutlined />}
          onClick={() => setSettingsModalOpen(true)}
        >
          {t('skills.settings')}
        </Button>
      </div>

      <div className={styles.toolbar}>
        <Space size="small">
          <Button
            type="text"
            icon={<ImportOutlined />}
            onClick={() => setImportModalOpen(true)}
            style={{ color: 'var(--color-text-tertiary)' }}
          >
            {t('skills.importExisting')} ({discoveredCount})
          </Button>
          <Button
            type="link"
            icon={<PlusOutlined />}
            onClick={() => setAddModalOpen(true)}
          >
            {t('skills.addSkill')}
          </Button>
        </Space>
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
    </div>
  );
};

export default SkillsPage;
