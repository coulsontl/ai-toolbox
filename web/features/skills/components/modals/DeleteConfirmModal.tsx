import React from 'react';
import { Modal, Button } from 'antd';
import { ExclamationCircleOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import styles from './DeleteConfirmModal.module.less';

interface DeleteConfirmModalProps {
  open: boolean;
  skillName: string;
  onClose: () => void;
  onConfirm: () => void;
  loading?: boolean;
}

export const DeleteConfirmModal: React.FC<DeleteConfirmModalProps> = ({
  open,
  skillName,
  onClose,
  onConfirm,
  loading,
}) => {
  const { t } = useTranslation();

  return (
    <Modal
      title={
        <span className={styles.title}>
          <ExclamationCircleOutlined className={styles.icon} />
          {t('skills.delete.title')}
        </span>
      }
      open={open}
      onCancel={onClose}
      footer={null}
      width={400}
    >
      <div className={styles.content}>
        <p>{t('skills.delete.message', { name: skillName })}</p>
        <ul className={styles.warnings}>
          <li>{t('skills.delete.warningRemoveFromTools')}</li>
          <li>{t('skills.delete.warningDeleteFromRepo')}</li>
        </ul>
      </div>

      <div className={styles.footer}>
        <Button onClick={onClose}>{t('common.cancel')}</Button>
        <Button type="primary" danger onClick={onConfirm} loading={loading}>
          {t('skills.delete.confirmButton')}
        </Button>
      </div>
    </Modal>
  );
};
