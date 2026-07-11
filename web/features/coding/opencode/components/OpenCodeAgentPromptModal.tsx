import React from 'react';
import { Button, Modal } from 'antd';
import { useTranslation } from 'react-i18next';

import MarkdownEditor from '@/components/common/MarkdownEditor';

import styles from './OpenCodeAgentPromptModal.module.less';

interface OpenCodeAgentPromptModalProps {
  open: boolean;
  agentName: string;
  initialValue?: string;
  onCancel: () => void;
  onSave: (prompt: string | undefined) => Promise<void> | void;
}

const OpenCodeAgentPromptModal: React.FC<OpenCodeAgentPromptModalProps> = ({
  open,
  agentName,
  initialValue,
  onCancel,
  onSave,
}) => {
  const { t } = useTranslation();
  const [draft, setDraft] = React.useState('');
  const [saving, setSaving] = React.useState(false);

  React.useEffect(() => {
    if (open) {
      setDraft(initialValue ?? '');
    }
  }, [initialValue, open]);

  const handleSave = async () => {
    setSaving(true);
    try {
      await onSave(draft.trim() ? draft : undefined);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      title={t('opencode.agentSettings.promptEditorTitle', { name: agentName })}
      open={open}
      width={920}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="save" type="primary" loading={saving} onClick={() => void handleSave()}>
          {t('opencode.agentSettings.savePrompt')}
        </Button>,
      ]}
    >
      <div className={styles.content}>
        <div className={styles.hint}>
          {t('opencode.agentSettings.promptEditorHint', { name: agentName })}
        </div>
        <MarkdownEditor
          value={draft}
          onChange={(value) => setDraft(value)}
          height={320}
          minHeight={220}
          maxHeight={520}
          resizable
          placeholder={t('opencode.agentSettings.promptPlaceholder')}
        />
      </div>
    </Modal>
  );
};

export default OpenCodeAgentPromptModal;
