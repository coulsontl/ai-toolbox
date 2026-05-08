import React from 'react';
import { Modal, Switch } from 'antd';
import { useTranslation } from 'react-i18next';

interface SidebarSettingsModalProps {
  open: boolean;
  onClose: () => void;
  sidebarVisible: boolean;
  onSidebarVisibleChange: (visible: boolean) => void | Promise<void>;
  width?: number;
  children?: React.ReactNode;
}

const SidebarSettingsModal: React.FC<SidebarSettingsModalProps> = ({
  open,
  onClose,
  sidebarVisible,
  onSidebarVisibleChange,
  width = 680,
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
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 16 }}>
        <div style={{ width: 180, paddingTop: 4, color: 'var(--color-text-primary)', fontWeight: 500 }}>
          {t('common.showSidebar')}
        </div>
        <div style={{ flex: 1 }}>
          <Switch checked={sidebarVisible} onChange={onSidebarVisibleChange} />
        </div>
      </div>
      {children && (
        <div style={{ marginTop: 18, display: 'flex', flexDirection: 'column', gap: 16 }}>
          {children}
        </div>
      )}
    </Modal>
  );
};

export default SidebarSettingsModal;
