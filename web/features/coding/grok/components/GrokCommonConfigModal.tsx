import React from 'react';
import { Modal, Alert, Button, Checkbox, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { parse as parseToml } from 'smol-toml';
import {
  extractGrokCommonConfigFromCurrentFile,
  getGrokCommonConfig,
  saveGrokCommonConfig,
  saveGrokLocalConfig,
} from '@/services/grokApi';
import TomlEditor from '@/components/common/TomlEditor';
import {
  getGrokIgnoredCommonConfigKeys,
  isGrokPrivacyProtectionEnabled,
  setGrokPrivacyProtection,
} from '@/utils/grokConfigUtils';
import styles from './GrokCommonConfigModal.module.less';

interface GrokCommonConfigModalProps {
  open: boolean;
  onCancel: () => void;
  onSuccess: () => void;
  isLocalProvider?: boolean;
  gatewaySaveLocked?: boolean;
}

function isTomlTextValid(value: string): boolean {
  try {
    if (value.trim()) {
      parseToml(value);
    }
    return true;
  } catch {
    return false;
  }
}

const GrokCommonConfigModal: React.FC<GrokCommonConfigModalProps> = ({
  open,
  onCancel,
  onSuccess,
  isLocalProvider = false,
  gatewaySaveLocked = false,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = React.useState(false);
  const [configValue, setConfigValue] = React.useState<string>('');
  const [rootDir, setRootDir] = React.useState<string | null>(null);
  const [isTomlValid, setIsTomlValid] = React.useState(true);

  const loadConfig = React.useCallback(async () => {
    setLoading(true);
    try {
      const config = await getGrokCommonConfig();
      if (config?.config) {
        setConfigValue(config.config);
        setIsTomlValid(isTomlTextValid(config.config));
        setRootDir(config.rootDir ?? null);
      } else {
        setConfigValue('');
        setIsTomlValid(true);
        setRootDir(null);
      }
    } catch (error) {
      console.error('Failed to load common config:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  // Load existing config
  React.useEffect(() => {
    if (open) {
      loadConfig();
    }
  }, [loadConfig, open]);

  const updateConfigValue = React.useCallback((value: string) => {
    setConfigValue(value);
    setIsTomlValid(isTomlTextValid(value));
  }, []);

  const handleSave = async () => {
    if (gatewaySaveLocked) {
      message.warning(t('gateway.proxy.commonConfigSaveLockedTooltip'));
      return;
    }

    // 验证 TOML 格式
    if (!isTomlValid) {
      message.error(t('grok.provider.configTomlInvalid'));
      return;
    }

    setLoading(true);
    try {
      if (isLocalProvider) {
        await saveGrokLocalConfig({ commonConfig: configValue, rootDir });
      } else {
        await saveGrokCommonConfig({ config: configValue, rootDir });
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
    updateConfigValue(value);
  };

  const privacyProtectionEnabled = React.useMemo(
    () => isTomlValid && isGrokPrivacyProtectionEnabled(configValue),
    [configValue, isTomlValid],
  );

  const ignoredCommonConfigKeys = React.useMemo(
    () => (isTomlValid ? getGrokIgnoredCommonConfigKeys(configValue) : []),
    [configValue, isTomlValid],
  );

  const handlePrivacyProtectionToggle = (checked: boolean) => {
    try {
      updateConfigValue(setGrokPrivacyProtection(configValue, checked));
    } catch (error) {
      console.error('Failed to toggle Grok privacy protection:', error);
      message.error(t('grok.provider.configTomlInvalid'));
    }
  };

  const handleExtractFromCurrentConfig = async () => {
    setLoading(true);
    try {
      const extractedConfig = await extractGrokCommonConfigFromCurrentFile();
      const nextConfig = extractedConfig.config || '';
      setConfigValue(nextConfig);
      setRootDir(extractedConfig.rootDir ?? null);
      setIsTomlValid(isTomlTextValid(nextConfig));
      message.success(t('grok.commonConfig.extractSuccess'));
    } catch (error) {
      console.error('Failed to extract common config from current Grok file:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  return (
    <Modal
      title={t('grok.commonConfig.title')}
      open={open}
      onCancel={onCancel}
      onOk={handleSave}
      confirmLoading={loading}
      width={800}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      footer={[
        <Button
          key="extract"
          onClick={handleExtractFromCurrentConfig}
          loading={loading}
        >
          {t('grok.commonConfig.extractFromCurrent')}
        </Button>,
        <Button key="cancel" onClick={onCancel} disabled={loading}>
          {t('common.cancel')}
        </Button>,
        <Button key="save" type="primary" onClick={handleSave} loading={loading}>
          {t('common.save')}
        </Button>,
      ]}
    >
      <div className={styles.content}>
        {isLocalProvider && (
          <Alert
            message={t('grok.localConfigHint')}
            type="warning"
            showIcon
          />
        )}
        <div className={styles.editorSection}>
          <div className={styles.quickOptions}>
            <Checkbox
              checked={privacyProtectionEnabled}
              disabled={!isTomlValid}
              title={t('grok.commonConfig.privacyProtectionHint')}
              onChange={(event) => handlePrivacyProtectionToggle(event.target.checked)}
            >
              {t('grok.commonConfig.enablePrivacyProtection')}
            </Checkbox>
          </div>
          <TomlEditor
            value={configValue}
            onChange={handleEditorChange}
            height={400}
          />
          {ignoredCommonConfigKeys.length > 0 && (
            <Alert
              message={t('grok.commonConfig.protectedWarningTitle')}
              description={t('grok.commonConfig.protectedWarningDescription', {
                keys: ignoredCommonConfigKeys.join(', '),
              })}
              type="warning"
              showIcon
            />
          )}
        </div>

        <Alert
          message={t('grok.commonConfig.description')}
          type="info"
          showIcon
        />
      </div>
    </Modal>
  );
};

export default GrokCommonConfigModal;
