import React from 'react';
import { Button, Space, message, Modal } from 'antd';
import { CheckOutlined, DownloadOutlined, FileSearchOutlined, FolderOpenOutlined, RobotOutlined } from '@ant-design/icons';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import * as api from '../../services/skillsApi';
import type { SkillInventoryPreview } from '../../types';
import styles from './SkillInventoryModal.module.less';

interface SkillInventoryModalProps {
  open: boolean;
  onClose: () => void;
  onSuccess: () => void;
}

export const SkillInventoryModal: React.FC<SkillInventoryModalProps> = ({ open, onClose, onSuccess }) => {
  const { t } = useTranslation();
  const [exportPath, setExportPath] = React.useState('');
  const [importPath, setImportPath] = React.useState('');
  const [preview, setPreview] = React.useState<SkillInventoryPreview | null>(null);
  const [loading, setLoading] = React.useState(false);

  React.useEffect(() => {
    if (!open) {
      setExportPath('');
      setImportPath('');
      setPreview(null);
    }
  }, [open]);

  const handleExportFile = async () => {
    setLoading(true);
    try {
      const path = await api.exportSkillInventoryFile();
      setExportPath(path);
      message.success(t('skills.inventory.exportSuccess', { path }));
      return path;
    } catch (error) {
      message.error(String(error));
      return null;
    } finally {
      setLoading(false);
    }
  };

  const handleCopyPrompt = async () => {
    setLoading(true);
    try {
      let currentExportPath = exportPath;
      if (!currentExportPath.trim()) {
        currentExportPath = await api.exportSkillInventoryFile();
        setExportPath(currentExportPath);
      }
      const prompt = t('skills.inventory.agentPromptText', { path: currentExportPath });
      await navigator.clipboard.writeText(prompt);
      message.success(t('skills.inventory.copyPromptSuccess'));
    } catch (error) {
      message.error(String(error));
    } finally {
      setLoading(false);
    }
  };

  const handleSelectImportFile = async () => {
    try {
      const selected = await openDialog({
        title: t('skills.inventory.importDialogTitle'),
        multiple: false,
        directory: false,
        filters: [
          {
            name: 'JSON',
            extensions: ['json'],
          },
        ],
      });
      if (typeof selected !== 'string') {
        return;
      }
      setImportPath(selected);
      setPreview(null);
    } catch (error) {
      message.error(String(error));
    }
  };

  const handlePreview = async () => {
    if (!importPath.trim()) return;
    setLoading(true);
    try {
      const result = await api.previewSkillInventoryImportFile(importPath);
      setPreview(result);
      if (!result.valid) {
        message.error(t('skills.inventory.previewInvalid'));
      }
    } catch (error) {
      message.error(String(error));
    } finally {
      setLoading(false);
    }
  };

  const handleApply = () => {
    if (!preview?.valid || !importPath.trim()) return;
    Modal.confirm({
      title: t('skills.inventory.applyTitle'),
      content: t('skills.inventory.applyContent', { count: preview.default_disable_count }),
      okText: t('skills.inventory.apply'),
      cancelText: t('common.cancel'),
      onOk: async () => {
        setLoading(true);
        try {
          const result = await api.applySkillInventoryImportFile(importPath);
          if (!result.valid) {
            setPreview(result);
            return;
          }
          message.success(t('skills.inventory.applySuccess'));
          onSuccess();
          onClose();
        } catch (error) {
          message.error(String(error));
        } finally {
          setLoading(false);
        }
      },
    });
  };

  const footer = (
    <div className={styles.footer}>
      <Space>
        <Button icon={<DownloadOutlined />} onClick={handleExportFile} loading={loading}>
          {t('skills.inventory.exportFile')}
        </Button>
        <Button icon={<RobotOutlined />} onClick={handleCopyPrompt} loading={loading}>
          {t('skills.inventory.copyAgentPrompt')}
        </Button>
      </Space>
      <Space>
        <Button onClick={onClose}>{t('common.cancel')}</Button>
        {!preview ? (
          <Button
            type="primary"
            icon={<FileSearchOutlined />}
            onClick={handlePreview}
            loading={loading}
            disabled={!importPath.trim()}
          >
            {t('skills.inventory.preview')}
          </Button>
        ) : (
          <Button
            type="primary"
            icon={<CheckOutlined />}
            onClick={handleApply}
            loading={loading}
            disabled={!preview.valid}
          >
            {t('skills.inventory.apply')}
          </Button>
        )}
      </Space>
    </div>
  );

  return (
    <Modal
      open={open}
      title={t('skills.inventory.title')}
      onCancel={onClose}
      width={780}
      footer={footer}
      destroyOnHidden
      className={styles.modal}
    >
      <div className={styles.content}>
        <section className={styles.sectionCard}>
          <div className={styles.sectionHeader}>
            <div>
              <strong>{t('skills.inventory.exportTitle')}</strong>
              <p>{t('skills.inventory.exportHint')}</p>
            </div>
          </div>
          <div className={styles.pathRow}>
            <span>{t('skills.inventory.exportPath')}</span>
            <code>{exportPath || t('skills.inventory.defaultExportPath')}</code>
          </div>
        </section>
        <section className={styles.sectionCard}>
          <div className={styles.sectionHeader}>
            <div>
              <strong>{t('skills.inventory.importTitle')}</strong>
              <p>{t('skills.inventory.importHint')}</p>
            </div>
            <Button icon={<FolderOpenOutlined />} onClick={handleSelectImportFile} disabled={loading}>
              {t('skills.inventory.selectFile')}
            </Button>
          </div>
          <div className={styles.pathRow}>
            <span>{t('skills.inventory.importPath')}</span>
            <code>{importPath || t('skills.inventory.noImportFile')}</code>
          </div>
        </section>
        {preview && (
          <section className={styles.previewCard}>
            <div className={styles.previewRow}>
              {t('skills.inventory.previewGroups', { count: preview.group_count })}
            </div>
            <div className={styles.previewRow}>
              {t('skills.inventory.previewMatched', { count: preview.matched_skill_count })}
            </div>
            <div className={styles.previewRow}>
              {t('skills.inventory.previewChanged', { count: preview.content_changed_count })}
            </div>
            <div className={`${styles.previewRow} ${preview.default_disable_count > 0 ? styles.previewRowWarning : ''}`}>
              {t('skills.inventory.previewDisable', { count: preview.default_disable_count })}
            </div>
            {preview.errors.length > 0 && (
              <div className={styles.previewErrors}>
                {preview.errors.map((err, i) => <div key={i}>{err}</div>)}
              </div>
            )}
          </section>
        )}
      </div>
    </Modal>
  );
};
