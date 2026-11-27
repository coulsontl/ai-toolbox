import React from 'react';
import { App, Modal } from 'antd';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';
import { setPluginOptions, type InstalledPluginRecord } from '@/services';
import { formatPluginError } from '../utils/pluginFormatters';
import styles from './PluginModals.module.less';

interface PluginOptionsModalProps {
  open: boolean;
  plugin: InstalledPluginRecord | null;
  onClose: () => void;
  onSaved: (record: InstalledPluginRecord) => void;
}

const PluginOptionsModal: React.FC<PluginOptionsModalProps> = ({
  open,
  plugin,
  onClose,
  onSaved,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [value, setValue] = React.useState<unknown>({});
  const [valid, setValid] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setValue(plugin?.options ?? {});
    setValid(true);
    setBusy(false);
    setError(null);
  }, [open, plugin]);

  const handleSave = async () => {
    if (!plugin) return;
    if (!valid) {
      setError(t('plugins.optionsModal.invalidJson'));
      return;
    }
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      setError(t('plugins.optionsModal.objectRequired'));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const record = await setPluginOptions(plugin.id, value);
      message.success(t('plugins.optionsModal.saveSuccess'));
      onSaved(record);
    } catch (saveError) {
      setError(
        t('plugins.optionsModal.saveFailed', { error: formatPluginError(saveError) }),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open={open}
      title={t('plugins.optionsModal.title', {
        name: plugin?.displayName || plugin?.packageName || '',
      })}
      width={720}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      confirmLoading={busy}
      okButtonProps={{ disabled: busy }}
      mask={{ closable: !busy }}
      onOk={() => void handleSave()}
      onCancel={onClose}
    >
      <div className={styles.content}>
        <div className={styles.sectionCard}>
          <p className={styles.sectionHint}>{t('plugins.optionsModal.hint')}</p>
          <JsonEditor
            key={plugin?.id ?? 'empty'}
            value={value}
            onChange={(nextValue, isValid) => {
              setValue(nextValue);
              setValid(isValid);
            }}
            height={340}
            minHeight={220}
            maxHeight={520}
          />
          {error ? <div className={styles.formError}>{error}</div> : null}
        </div>
      </div>
    </Modal>
  );
};

export default PluginOptionsModal;
