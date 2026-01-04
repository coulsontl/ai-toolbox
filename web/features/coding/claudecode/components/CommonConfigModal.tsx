import React from 'react';
import { Modal, Alert, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { getClaudeCommonConfig, saveClaudeCommonConfig } from '@/services/claudeCodeApi';
import JsonEditor from '@/components/common/JsonEditor';

interface CommonConfigModalProps {
  open: boolean;
  onCancel: () => void;
  onSuccess: () => void;
}

const CommonConfigModal: React.FC<CommonConfigModalProps> = ({
  open,
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [configValue, setConfigValue] = React.useState<unknown>({});
  const [isValid, setIsValid] = React.useState(true);

  // 加载现有配置
  React.useEffect(() => {
    if (open) {
      loadConfig();
    }
  }, [open]);

  const loadConfig = async () => {
    try {
      const config = await getClaudeCommonConfig();
      if (config && config.config) {
        try {
          const configObj = JSON.parse(config.config);
          setConfigValue(configObj);
          setIsValid(true);
        } catch (error) {
          console.error('Failed to parse config JSON:', error);
          setConfigValue(config.config);
          setIsValid(false);
        }
      } else {
        setConfigValue({});
        setIsValid(true);
      }
    } catch (error) {
      console.error('Failed to load common config:', error);
      message.error(t('common.error'));
    }
  };

  const handleSave = async () => {
    if (!isValid) {
      message.error(t('claudecode.commonConfig.invalidJson'));
      return;
    }

    setLoading(true);
    try {
      const configString = JSON.stringify(configValue, null, 2);
      await saveClaudeCommonConfig(configString);
      message.success(t('common.success'));
      onSuccess();
      onCancel();
    } catch (error) {
      console.error('Failed to save common config:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const handleEditorChange = (value: unknown, valid: boolean) => {
    setConfigValue(value);
    setIsValid(valid);
  };

  return (
    <Modal
      title={t('claudecode.commonConfig.title')}
      open={open}
      onCancel={onCancel}
      onOk={handleSave}
      confirmLoading={loading}
      width={700}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      okButtonProps={{ disabled: !isValid }}
    >
      <div style={{ marginBottom: 16 }}>
        <Alert
          message={t('claudecode.commonConfig.description')}
          type="info"
          showIcon
          style={{ marginBottom: 12 }}
        />
      </div>

      <JsonEditor
        value={configValue}
        onChange={handleEditorChange}
        mode="text"
        height={400}
        minHeight={200}
        maxHeight={600}
        resizable
      />

      <div style={{ marginTop: 12 }}>
        <Alert
          message={t('claudecode.commonConfig.hint')}
          type="info"
          showIcon
          closable
        />
      </div>
    </Modal>
  );
};

export default CommonConfigModal;
