import React from 'react';
import { Modal, Alert, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { getClaudeCommonConfig, saveClaudeCommonConfig, saveClaudeLocalConfig } from '@/services/claudeCodeApi';
import JsonEditor from '@/components/common/JsonEditor';

interface CommonConfigModalProps {
  open: boolean;
  onCancel: () => void;
  onSuccess: () => void;
  isLocalProvider?: boolean;
}

const CommonConfigModal: React.FC<CommonConfigModalProps> = ({
  open,
  onCancel,
  onSuccess,
  isLocalProvider = false,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [configValue, setConfigValue] = React.useState<unknown>({});

  // Use ref for validation state to avoid re-renders during editing
  const isValidRef = React.useRef(true);

  // 加载现有配置
  React.useEffect(() => {
    if (open) {
      loadConfig();
    }
  }, [open]);

  const loadConfig = async () => {
    try {
      const config = await getClaudeCommonConfig();
      if (config?.config) {
        try {
          const configObj = JSON.parse(config.config);
          setConfigValue(configObj);
          isValidRef.current = true;
        } catch (error) {
          console.error('Failed to parse config JSON:', error);
          setConfigValue(config.config);
          isValidRef.current = false;
        }
      } else {
        // 空配置时设置为空字符串，让 JSON 编辑器显示 placeholder
        setConfigValue("");
        isValidRef.current = true;
      }
    } catch (error) {
      console.error('Failed to load common config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const handleSave = async () => {
    if (!isValidRef.current) {
      message.error(t('claudecode.commonConfig.invalidJson'));
      return;
    }

    setLoading(true);
    try {
      const configString = JSON.stringify(configValue, null, 2);
      if (isLocalProvider) {
        await saveClaudeLocalConfig({ commonConfig: configString });
      } else {
        await saveClaudeCommonConfig(configString);
      }
      message.success(t('common.success'));
      onSuccess();
      onCancel();
    } catch (error) {
      console.error('Failed to save common config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const handleEditorChange = (value: unknown, valid: boolean) => {
    setConfigValue(value);
    isValidRef.current = valid;
  };

  return (
    <Modal
      title={t('claudecode.commonConfig.title')}
      open={open}
      onCancel={onCancel}
      onOk={handleSave}
      confirmLoading={loading}
      width={800}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
    >
      {isLocalProvider && (
        <Alert
          message={t('claudecode.localConfigHint')}
          type="warning"
          showIcon
          style={{ marginBottom: 16 }}
        />
      )}
      <JsonEditor
        value={configValue}
        onChange={handleEditorChange}
        mode="text"
        height={400}
        minHeight={200}
        maxHeight={600}
        resizable
        placeholder={`{
    "skipWebFetchPreflight": true
}`}
      />

      <div style={{ marginTop: 12 }}>
        <Alert
          message={t('claudecode.commonConfig.combinedHint')}
          type="info"
          showIcon
        />
      </div>
    </Modal>
  );
};

export default CommonConfigModal;
