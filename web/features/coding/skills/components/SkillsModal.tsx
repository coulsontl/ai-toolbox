import React from 'react';
import { Modal, Button, Space } from 'antd';
import { PlusOutlined, UserOutlined, ImportOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSkillsStore } from '../stores/skillsStore';
import { useSkills } from '../hooks/useSkills';
import { useSkillActions } from '../hooks/useSkillActions';
import { SkillsList } from './SkillsList';
import { AddSkillModal } from './modals/AddSkillModal';
import { ImportModal } from './modals/ImportModal';
import { SkillsSettingsModal } from './modals/SkillsSettingsModal';
import { DeleteConfirmModal } from './modals/DeleteConfirmModal';
import { NewToolsModal } from './modals/NewToolsModal';
import { SkillMetadataModal } from './modals/SkillMetadataModal';
import { getSkillGroupOptions } from '../utils/skillGrouping';
import type { ManagedSkill } from '../types';
import { refreshTrayMenu } from '@/services/appApi';
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
    refresh,
  } = useSkills();

  const allTools = getAllTools();
  const [metadataSkill, setMetadataSkill] = React.useState<ManagedSkill | null>(null);
  const groupOptions = React.useMemo(() => getSkillGroupOptions(skills), [skills]);

  const {
    actionLoading,
    updatingSkillIds,
    deleteSkillId,
    setDeleteSkillId,
    skillToDelete,
    handleToggleTool,
    handleUpdate,
    handleDelete,
    confirmDelete,
    handleDragEnd,
  } = useSkillActions({ allTools });

  return (
    <>
      <Modal
        title={t('skills.title')}
        open={isOpen}
        onCancel={handleClose}
        footer={null}
        width={900}
        className={styles.skillsModal}
        destroyOnHidden
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
            <Button icon={<ImportOutlined />} onClick={() => setImportModalOpen(true)}>
              {t('skills.reviewImport')}
            </Button>
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
            updatingSkillIds={updatingSkillIds}
            getGithubInfo={getGithubInfo}
            getSkillSourceLabel={getSkillSourceLabel}
            formatRelative={formatRelative}
            onUpdate={handleUpdate}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onEditMetadata={setMetadataSkill}
            onDragEnd={handleDragEnd}
          />
        </div>
      </Modal>

      {isAddModalOpen && (
        <AddSkillModal
          open={isAddModalOpen}
          onClose={() => setAddModalOpen(false)}
          allTools={allTools}
          onSuccess={async () => {
            setAddModalOpen(false);
            await refresh();
            await refreshTrayMenu();
          }}
        />
      )}

      {isImportModalOpen && (
        <ImportModal
          open={isImportModalOpen}
          onClose={() => setImportModalOpen(false)}
          onSuccess={async () => {
            setImportModalOpen(false);
            await refresh();
            await refreshTrayMenu();
          }}
        />
      )}

      {isSettingsModalOpen && (
        <SkillsSettingsModal
          open={isSettingsModalOpen}
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

      <NewToolsModal
        open={isNewToolsModalOpen}
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
    </>
  );
};
