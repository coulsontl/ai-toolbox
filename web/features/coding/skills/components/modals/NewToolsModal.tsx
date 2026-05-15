import React from 'react';
import { Modal, Button, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { useSkillsStore } from '../../stores/skillsStore';
import * as api from '../../services/skillsApi';
import styles from './NewToolsModal.module.less';

interface NewToolsModalProps {
  open: boolean;
}

export const NewToolsModal: React.FC<NewToolsModalProps> = ({ open }) => {
  const { t } = useTranslation();
  const { setNewToolsModalOpen, toolStatus, skills, loadSkills } = useSkillsStore();
  const [loading, setLoading] = React.useState(false);

  const newlyInstalled = toolStatus?.newly_installed || [];

  // Get tool labels
  const toolLabels = React.useMemo(() => {
    const tools = toolStatus?.tools || [];
    return newlyInstalled
      .map((key) => {
        const tool = tools.find((t) => t.key === key);
        return tool?.label || key;
      })
      .join(', ');
  }, [toolStatus, newlyInstalled]);

  const handleSyncAll = async () => {
    setLoading(true);
    try {
      for (const skill of skills.filter((skill) => skill.management_enabled)) {
        for (const toolKey of newlyInstalled) {
          // Check if already synced
          if (skill.targets.some((t) => t.tool === toolKey)) {
            continue;
          }
          try {
            await api.syncSkillToTool(
              skill.central_path,
              skill.id,
              toolKey,
              skill.name
            );
          } catch (error) {
            console.warn(`Failed to sync ${skill.name} to ${toolKey}:`, error);
          }
        }
      }
      message.success(t('skills.status.syncCompleted'));
      await loadSkills();
    } catch (error) {
      message.error(String(error));
    } finally {
      setLoading(false);
      setNewToolsModalOpen(false);
    }
  };

  const handleLater = () => {
    setNewToolsModalOpen(false);
  };

  return (
    <Modal
      title={t('skills.newToolsTitle')}
      open={open}
      onCancel={handleLater}
      footer={null}
      width={450}
    >
      <p className={styles.body}>
        {t('skills.newToolsBody', { tools: toolLabels })}
      </p>

      <div className={styles.footer}>
        <Button onClick={handleLater}>{t('skills.later')}</Button>
        <Button type="primary" onClick={handleSyncAll} loading={loading}>
          {t('skills.syncAll')}
        </Button>
      </div>
    </Modal>
  );
};
