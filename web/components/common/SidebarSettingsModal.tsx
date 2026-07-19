import React from 'react';
import { Modal, Switch } from 'antd';
import { useTranslation } from 'react-i18next';
import styles from './SidebarSettingsModal.module.less';

export interface SettingsToggleRowProps {
  title: string;
  hint?: string;
  checked: boolean;
  loading?: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void | Promise<void>;
}

/** Flat toggle row for "More Options" modals: title + optional 10px hint on the left, Switch on the right. */
export const SettingsToggleRow: React.FC<SettingsToggleRowProps> = ({
  title,
  hint,
  checked,
  loading = false,
  disabled = false,
  onChange,
}) => (
  <div className={styles.row}>
    <div className={styles.text}>
      <div className={styles.title}>{title}</div>
      {hint ? <p className={styles.hint}>{hint}</p> : null}
    </div>
    <div className={styles.control}>
      <Switch
        checked={checked}
        loading={loading}
        disabled={disabled}
        onChange={onChange}
      />
    </div>
  </div>
);

interface SidebarSettingsModalProps {
  open: boolean;
  onClose: () => void;
  sidebarVisible: boolean;
  onSidebarVisibleChange: (visible: boolean) => void | Promise<void>;
  width?: number;
  /** Additional flat rows (typically SettingsToggleRow). Rendered after the sidebar switch. */
  children?: React.ReactNode;
}

const SidebarSettingsModal: React.FC<SidebarSettingsModalProps> = ({
  open,
  onClose,
  sidebarVisible,
  onSidebarVisibleChange,
  width = 480,
  children,
}) => {
  const { t } = useTranslation();

  return (
    <Modal
      title={t('common.moreOptions')}
      open={open}
      onCancel={onClose}
      footer={null}
      width={width}
      destroyOnHidden
    >
      <div className={styles.list}>
        <SettingsToggleRow
          title={t('common.showSidebar')}
          checked={sidebarVisible}
          onChange={onSidebarVisibleChange}
        />
        {children}
      </div>
    </Modal>
  );
};

export default SidebarSettingsModal;
