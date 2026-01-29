import React from 'react';
import { Modal, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';

const { Text } = Typography;

export interface JsonPreviewModalProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  data: unknown;
}

const JsonPreviewModal: React.FC<JsonPreviewModalProps> = ({
  open,
  onClose,
  title,
  data,
}) => {
  const { t } = useTranslation();

  return (
    <Modal
      title={
        <span>
          {title || t('common.previewConfig')}{' '}
          <Text type="secondary" style={{ fontSize: 12, fontWeight: 'normal' }}>
            ({t('common.readOnly')})
          </Text>
        </span>
      }
      open={open}
      onCancel={onClose}
      footer={null}
      width={1000}
      centered
      styles={{
        body: {
          padding: '16px 24px',
        },
      }}
    >
      <JsonEditor
        value={data}
        readOnly={true}
        mode="text"
        height="calc(80vh - 120px)"
        resizable={false}
        showMainMenuBar={false}
        showStatusBar={false}
      />
    </Modal>
  );
};

export default JsonPreviewModal;
