import React from 'react';
import { Modal, Button, Space, Dropdown, MenuProps } from 'antd';
import {
  FileTextOutlined,
  PlusOutlined,
  TagsOutlined,
  MoreOutlined,
  SettingOutlined,
  CloudDownloadOutlined,
} from '@ant-design/icons';
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
import { SkillGroupsModal } from './modals/SkillGroupsModal';
import { SkillInventoryModal } from './modals/SkillInventoryModal';
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
    groups,
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
    refresh,
  } = useSkills();

  const allTools = getAllTools();
  const [metadataSkill, setMetadataSkill] = React.useState<ManagedSkill | null>(null);
  const [groupsModalOpen, setGroupsModalOpen] = React.useState(false);
  const [inventoryModalOpen, setInventoryModalOpen] = React.useState(false);
  const groupOptions = React.useMemo(() => getSkillGroupOptions(groups), [groups]);

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
    handleSetManagementEnabled,
  } = useSkillActions({ allTools });

  const moreMenuItems: MenuProps['items'] = [
    {
      key: 'settings',
      icon: <SettingOutlined />,
      label: t('skills.settings'),
      onClick: () => setSettingsModalOpen(true),
    },
  ];

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
            <Button icon={<CloudDownloadOutlined />} onClick={() => setImportModalOpen(true)}>
              {t('skills.importExisting')}
            </Button>
            <Button icon={<TagsOutlined />} onClick={() => setGroupsModalOpen(true)}>
              {t('skills.groups.manage')}
            </Button>
            <Button icon={<FileTextOutlined />} onClick={() => setInventoryModalOpen(true)}>
              {t('skills.inventory.button')}
            </Button>
          </Space>
          <Dropdown menu={{ items: moreMenuItems }} trigger={['click']}>
            <Button icon={<MoreOutlined />} />
          </Dropdown>
        </div>

        <div className={styles.content}>
          <SkillsList
            skills={skills}
            allTools={allTools}
            loading={loading || actionLoading}
            updatingSkillIds={updatingSkillIds}
            getGithubInfo={getGithubInfo}
            formatRelative={formatRelative}
            onUpdate={handleUpdate}
            onDelete={handleDelete}
            onToggleTool={handleToggleTool}
            onEditMetadata={setMetadataSkill}
            onSetManagementEnabled={handleSetManagementEnabled}
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
    </>
  );
};
