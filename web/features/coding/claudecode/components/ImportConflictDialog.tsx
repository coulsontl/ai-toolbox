import React from 'react';
import { Modal, Radio, Space, Typography, Alert } from 'antd';
import { useTranslation } from 'react-i18next';
import type { ImportConflictInfo, ImportConflictAction } from '@/types/claudecode';

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

  // 解析 settingsConfig JSON 字符串
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
      title={t('claudecode.conflict.title')}
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
          message={t('claudecode.conflict.message', { name: conflictInfo.newProviderName })}
          type="warning"
          showIcon
        />

        <div>
          <Text strong>{t('claudecode.conflict.existingConfig')}</Text>
          <div style={{ marginTop: 8, marginLeft: 16 }}>
            <div>• {t('claudecode.provider.name')}: {conflictInfo.existingProvider.name}</div>
            <div>
              • {t('claudecode.provider.baseUrl')}:{' '}
              {existingConfig.env?.ANTHROPIC_BASE_URL || '-'}
            </div>
            <div>• {t('claudecode.conflict.createdAt')}: {createdDate}</div>
          </div>
        </div>

        <div>
          <Text strong>{t('claudecode.conflict.chooseAction')}</Text>
          <Radio.Group
            value={selectedAction}
            onChange={(e) => setSelectedAction(e.target.value)}
            style={{ marginTop: 8, width: '100%' }}
          >
            <Space direction="vertical" size={8}>
              <Radio value="overwrite">{t('claudecode.conflict.overwrite')}</Radio>
              <Radio value="duplicate">{t('claudecode.conflict.duplicate')}</Radio>
            </Space>
          </Radio.Group>
        </div>
      </Space>
    </Modal>
  );
};

export default ImportConflictDialog;
