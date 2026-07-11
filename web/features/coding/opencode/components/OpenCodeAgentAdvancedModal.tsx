import React from 'react';
import { Alert, Button, Modal } from 'antd';
import { useTranslation } from 'react-i18next';

import JsonEditor from '@/components/common/JsonEditor';
import type { OpenCodeAgentConfig } from '@/types/opencode';

import {
  isOpenCodeJsonObject,
  validateOpenCodeAgentConfig,
} from '../utils/openCodeAgentConfig';
import styles from './OpenCodeAgentAdvancedModal.module.less';

interface OpenCodeAgentAdvancedModalProps {
  open: boolean;
  agentName: string;
  initialConfig?: OpenCodeAgentConfig;
  requireDescription?: boolean;
  onCancel: () => void;
  onSave: (agentConfig: OpenCodeAgentConfig) => Promise<void> | void;
}

const OpenCodeAgentAdvancedModal: React.FC<OpenCodeAgentAdvancedModalProps> = ({
  open,
  agentName,
  initialConfig,
  requireDescription = false,
  onCancel,
  onSave,
}) => {
  const { t } = useTranslation();
  const [value, setValue] = React.useState<unknown>({});
  const [jsonValid, setJsonValid] = React.useState(true);
  const [validationKey, setValidationKey] = React.useState<string>();
  const [saving, setSaving] = React.useState(false);

  React.useEffect(() => {
    if (!open) return;
    setValue(initialConfig ?? {});
    setJsonValid(true);
    setValidationKey(undefined);
  }, [open, initialConfig]);

  const handleSave = async () => {
    if (!jsonValid) {
      setValidationKey('json');
      return;
    }

    const nextValidationKey = validateOpenCodeAgentConfig(value, { requireDescription });
    if (nextValidationKey) {
      setValidationKey(nextValidationKey);
      return;
    }
    if (!isOpenCodeJsonObject(value)) {
      setValidationKey('object');
      return;
    }

    setSaving(true);
    try {
      await onSave(value as OpenCodeAgentConfig);
      onCancel();
    } catch {
      // Parent save handler already reports the error and the modal stays open.
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      title={t('opencode.agentSettings.advancedTitle', { name: agentName })}
      open={open}
      width={760}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="save" type="primary" loading={saving} onClick={handleSave}>
          {t('common.save')}
        </Button>,
      ]}
    >
      <div className={styles.content}>
        <div className={styles.hint}>
          {t('opencode.agentSettings.advancedHint')}
        </div>
        {validationKey ? (
          <Alert
            className={styles.error}
            type="error"
            showIcon
            message={t(`opencode.agentSettings.validation.${validationKey}`)}
          />
        ) : null}
        <JsonEditor
          value={value}
          onChange={(nextValue, isValid) => {
            setValue(nextValue ?? {});
            setJsonValid(isValid);
            if (isValid) setValidationKey(undefined);
          }}
          height={420}
          minHeight={260}
          maxHeight={620}
          resizable
          mode="text"
          placeholder={`{
  "model": "anthropic/claude-sonnet-5",
  "variant": "high",
  "permission": {
    "edit": "deny"
  }
}`}
        />
      </div>
    </Modal>
  );
};

export default OpenCodeAgentAdvancedModal;
