import React from 'react';
import { Modal, Radio, Space, Typography, Alert } from 'antd';
import { useTranslation } from 'react-i18next';
import type { ImportConflictInfo, ImportConflictAction } from '@/types/grok';

const { Text } = Typography;

interface ImportConflictDialogProps {
  open: boolean;
  conflictInfo: ImportConflictInfo | null;
  onResolve: (action: ImportConflictAction) => void;
  onCancel: () => void;
}

const ImportConflictDialog: React.FC<ImportConflictDialogProps> = ({
  open,
  conflictInfo,
  onResolve,
  onCancel,
}) => {
  const { t } = useTranslation();
  const [selectedAction, setSelectedAction] = React.useState<ImportConflictAction>('duplicate');

  const handleOk = () => {
    onResolve(selectedAction);
  };

  if (!conflictInfo) return null;

  const createdDate = conflictInfo.existingProvider.createdAt
    ? new Date(conflictInfo.existingProvider.createdAt).toLocaleString()
    : t('common.notSet');

  // Parse settingsConfig JSON string
  const existingConfig = React.useMemo(() => {
    try {
      return JSON.parse(conflictInfo.existingProvider.settingsConfig);
    } catch (error) {
      console.error('Failed to parse settingsConfig:', error);
      return {};
    }
  }, [conflictInfo.existingProvider.settingsConfig]);

  return (
    <Modal
      title={t('grok.conflict.title')}
      open={open}
      onOk={handleOk}
      onCancel={() => {
        setSelectedAction('duplicate');
        onCancel();
      }}
      okText={t('common.confirm')}
      cancelText={t('common.cancel')}
      width={500}
    >
      <Space direction="vertical" size={16} style={{ width: '100%' }}>
        <Alert
          message={t('grok.conflict.message', { name: conflictInfo.newProviderName })}
          type="warning"
          showIcon
        />

        <div>
          <Text strong>{t('grok.conflict.existingConfig')}</Text>
          <div style={{ marginTop: 8, marginLeft: 16 }}>
            <div>{t('grok.provider.name')}: {conflictInfo.existingProvider.name}</div>
            <div>
              API Key: {existingConfig.auth?.API_KEY ? '••••••••' : '-'}
            </div>
            <div>{t('grok.conflict.createdAt')}: {createdDate}</div>
          </div>
        </div>

        <div>
          <Text strong>{t('grok.conflict.chooseAction')}</Text>
          <Radio.Group
            value={selectedAction}
            onChange={(e) => setSelectedAction(e.target.value)}
            style={{ marginTop: 8, width: '100%' }}
          >
            <Space direction="vertical" size={8}>
              <Radio value="overwrite">{t('grok.conflict.overwrite')}</Radio>
              <Radio value="duplicate">{t('grok.conflict.duplicate')}</Radio>
            </Space>
          </Radio.Group>
        </div>
      </Space>
    </Modal>
  );
};

export default ImportConflictDialog;
