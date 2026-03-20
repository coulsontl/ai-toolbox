import React from 'react';
import { Modal, Alert, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { getCodexCommonConfig, saveCodexCommonConfig, saveCodexLocalConfig } from '@/services/codexApi';
import TomlEditor from '@/components/common/TomlEditor';
import { parse as parseToml } from 'smol-toml';

interface CodexCommonConfigModalProps {
  open: boolean;
  onCancel: () => void;
  onSuccess: () => void;
  isLocalProvider?: boolean;
}

const CodexCommonConfigModal: React.FC<CodexCommonConfigModalProps> = ({
  open,
  onCancel,
  onSuccess,
  isLocalProvider = false,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [configValue, setConfigValue] = React.useState<string>('');
  const [isTomlValid, setIsTomlValid] = React.useState(true);

  const loadConfig = React.useCallback(async () => {
    try {
      const config = await getCodexCommonConfig();
      if (config?.config) {
        setConfigValue(config.config);
      } else {
        setConfigValue('');
      }
    } catch (error) {
      console.error('Failed to load common config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  }, [t]);

  // Load existing config
  React.useEffect(() => {
    if (open) {
      loadConfig();
    }
  }, [loadConfig, open]);

  const handleSave = async () => {
    // 验证 TOML 格式
    if (!isTomlValid) {
      message.error(t('codex.provider.configTomlInvalid'));
      return;
    }

    setLoading(true);
    try {
      if (isLocalProvider) {
        await saveCodexLocalConfig({ commonConfig: configValue });
      } else {
        await saveCodexCommonConfig(configValue);
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

  const handleEditorChange = (value: string) => {
    setConfigValue(value);

    // 验证 TOML 有效性
    try {
      if (value.trim()) {
        parseToml(value);
      }
      setIsTomlValid(true);
    } catch {
      setIsTomlValid(false);
    }
  };

  return (
    <Modal
      title={t('codex.commonConfig.title')}
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
          message={t('codex.localConfigHint')}
          type="warning"
          showIcon
          style={{ marginBottom: 16 }}
        />
      )}
      <TomlEditor
        value={configValue}
        onChange={handleEditorChange}
        height={400}
      />

      <div style={{ marginTop: 12 }}>
        <Alert
          message={t('codex.commonConfig.description')}
          type="info"
          showIcon
        />
      </div>
    </Modal>
  );
};

export default CodexCommonConfigModal;
