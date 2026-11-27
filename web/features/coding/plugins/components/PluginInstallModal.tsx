import React from 'react';
import { Input, Modal } from 'antd';
import { FileCode2, FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { installPlugin, type InstalledPluginRecord } from '@/services';
import { formatPluginError } from '../utils/pluginFormatters';
import styles from './PluginModals.module.less';

type InstallSourceType = 'npm' | 'github' | 'local';

interface PluginInstallModalProps {
  open: boolean;
  /** A package a plugin page asked to install; the reader still confirms. */
  initialSpec?: string;
  onClose: () => void;
  onInstalled: (record: InstalledPluginRecord) => void;
}

const PluginInstallModal: React.FC<PluginInstallModalProps> = ({
  open,
  initialSpec = '',
  onClose,
  onInstalled,
}) => {
  const { t } = useTranslation();
  const [sourceType, setSourceType] = React.useState<InstallSourceType>('npm');
  const [spec, setSpec] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setSourceType('npm');
    setSpec(initialSpec);
    setBusy(false);
    setError(null);
  }, [initialSpec, open]);

  const sourceLabels: Record<InstallSourceType, string> = {
    npm: t('plugins.installModal.sourceNpm'),
    github: t('plugins.installModal.sourceGithub'),
    local: t('plugins.installModal.sourceLocal'),
  };

  const placeholder = sourceType === 'npm'
    ? t('plugins.installModal.npmPlaceholder')
    : sourceType === 'github'
      ? t('plugins.installModal.githubPlaceholder')
      : t('plugins.installModal.localPlaceholder');

  const normalizeSpec = (): string => {
    const value = spec.trim();
    if (sourceType === 'github' && value && !value.startsWith('github:')) {
      return `github:${value}`;
    }
    return value;
  };

  const choosePath = async (directory: boolean) => {
    const selected = await openDialog({
      multiple: false,
      directory,
      filters: directory
        ? undefined
        : [{ name: 'Middleware', extensions: ['js', 'mjs', 'cjs'] }],
    });
    if (typeof selected === 'string') {
      setSpec(selected);
      setError(null);
    }
  };

  const handleSubmit = async () => {
    if (!spec.trim()) {
      setError(t('plugins.installModal.specRequired'));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const record = await installPlugin(normalizeSpec());
      onInstalled(record);
    } catch (installError) {
      setError(
        t('plugins.installModal.failed', { error: formatPluginError(installError) }),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open={open}
      title={t('plugins.installModal.title')}
      width={600}
      okText={t('plugins.installModal.install')}
      cancelText={t('common.cancel')}
      confirmLoading={busy}
      okButtonProps={{ disabled: busy }}
      mask={{ closable: !busy }}
      onOk={() => void handleSubmit()}
      onCancel={onClose}
    >
      <div className={styles.content}>
        <div className={styles.sectionCard}>
          <div className={styles.formFieldRow}>
            <span className={styles.formLabel}>
              {t('plugins.installModal.sourceType')}
            </span>
            <div className={styles.formControl}>
              <div className={styles.segmented}>
                {(['npm', 'github', 'local'] as InstallSourceType[]).map((type) => (
                  <button
                    key={type}
                    type="button"
                    className={`${styles.segmentButton} ${sourceType === type ? styles.segmentButtonActive : ''}`}
                    disabled={busy}
                    onClick={() => {
                      setSourceType(type);
                      setSpec('');
                      setError(null);
                    }}
                  >
                    {sourceLabels[type]}
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div className={styles.formFieldRow}>
            <span className={styles.formLabel}>
              {t('plugins.installModal.spec')}
            </span>
            <div className={styles.formControl}>
              <Input
                value={spec}
                onChange={(event) => setSpec(event.target.value)}
                placeholder={placeholder}
                disabled={busy}
                onPressEnter={() => void handleSubmit()}
                className={styles.formControlInput}
              />
              {sourceType === 'local' ? (
                <>
                  <button
                    type="button"
                    className={styles.actionButton}
                    disabled={busy}
                    onClick={() => void choosePath(true)}
                  >
                    <FolderOpen size={14} aria-hidden="true" />
                    <span>{t('plugins.installModal.browseFolder')}</span>
                  </button>
                  <button
                    type="button"
                    className={styles.actionButton}
                    disabled={busy}
                    onClick={() => void choosePath(false)}
                  >
                    <FileCode2 size={14} aria-hidden="true" />
                    <span>{t('plugins.installModal.browseFile')}</span>
                  </button>
                </>
              ) : null}
            </div>
          </div>

          <p className={styles.sectionHint}>{t('plugins.installModal.hint')}</p>
          {error ? <div className={styles.formError}>{error}</div> : null}
        </div>
      </div>
    </Modal>
  );
};

export default PluginInstallModal;
