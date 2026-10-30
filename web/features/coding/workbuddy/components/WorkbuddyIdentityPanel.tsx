import React from 'react';
import { Alert, App, Button, Input, Space, Spin, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import {
  getWorkbuddyIdentityState,
  saveWorkbuddyIdentityFile,
} from '@/services/workbuddyApi';
import type { WorkbuddyIdentityState } from '@/types/workbuddy';
import styles from './WorkbuddyIdentityPanel.module.less';

const { Text } = Typography;

interface Props {
  /** Called after a successful save so the page can refresh its own state. */
  onSaved?: () => Promise<void> | void;
  /**
   * The config root's path; any change reloads the panel.
   *
   * The panel's data is scoped to the signed-in account of *that* root, so a
   * root switch invalidates everything here. Without this the panel kept
   * showing the previous root's files while saves failed against the new one
   * ("No WorkBuddy account is signed in") — a path the user had no way to
   * understand from the screen.
   */
  rootPath?: string;
}

/**
 * The four per-account identity documents.
 *
 * These are live files the product itself reads and rewrites, and each one is
 * a different document (`IDENTITY.md` = who the assistant is, `USER.md` = who
 * the user is, …). They are therefore edited in place, not modelled as
 * switchable prompt presets — there is no "apply" here, only save.
 *
 * The directory is scoped to the signed-in account (`user-<uid>-<scope>`), so
 * without a login the whole section is unavailable and says why instead of
 * showing paths that do not exist.
 */
const WorkbuddyIdentityPanel: React.FC<Props> = ({ onSaved, rootPath }) => {
  const { t } = useTranslation();
  const { message } = App.useApp();

  const [state, setState] = React.useState<WorkbuddyIdentityState | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [drafts, setDrafts] = React.useState<Record<string, string>>({});
  const [savingFile, setSavingFile] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    setLoading(true);
    try {
      const next = await getWorkbuddyIdentityState();
      setState(next);
      setDrafts(
        Object.fromEntries(next.files.map((file) => [file.fileName, file.content])),
      );
    } catch (error) {
      console.error('Failed to load WorkBuddy identity state:', error);
      message.error(t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [message, t]);

  React.useEffect(() => {
    void load();
  }, [load, rootPath]);

  const handleSave = async (fileName: string) => {
    setSavingFile(fileName);
    try {
      const next = await saveWorkbuddyIdentityFile({
        fileName,
        content: drafts[fileName] ?? '',
      });
      setState(next);
      message.success(t('workbuddy.identity.saved', { name: fileName }));
      await onSaved?.();
    } catch (error) {
      console.error('Failed to save WorkBuddy identity file:', error);
      message.error(String(error));
    } finally {
      setSavingFile(null);
    }
  };

  if (loading && !state) {
    return <Spin />;
  }
  if (!state) {
    return null;
  }
  if (state.unavailableReason) {
    return (
      <div className={styles.panel}>
        <div className={styles.panelHeader}>{t('workbuddy.identity.title')}</div>
        <Alert type="info" showIcon message={state.unavailableReason} />
      </div>
    );
  }

  return (
    <div className={styles.panel}>
      <div className={styles.panelHeader}>{t('workbuddy.identity.title')}</div>
      <Space direction="vertical" style={{ width: '100%' }} size={16}>
        <Space direction="vertical" size={2}>
          <Text type="secondary">{t('workbuddy.identity.hint')}</Text>
          <Text type="secondary" copyable={{ text: state.identityDir }}>
            {t('workbuddy.identity.dir')}: {state.identityDir}
          </Text>
        </Space>
        {state.files.map((file) => (
          <Space key={file.fileName} direction="vertical" style={{ width: '100%' }} size={4}>
            <Space>
              <Text strong>{file.fileName}</Text>
              {!file.exists && <Text type="secondary">{t('workbuddy.identity.missing')}</Text>}
            </Space>
            <Input.TextArea
              value={drafts[file.fileName] ?? ''}
              autoSize={{ minRows: 4, maxRows: 16 }}
              onChange={(event) => {
                const value = event.target.value;
                setDrafts((previous) => ({ ...previous, [file.fileName]: value }));
              }}
            />
            <Button
              size="small"
              loading={savingFile === file.fileName}
              onClick={() => {
                void handleSave(file.fileName);
              }}
            >
              {t('workbuddy.identity.save')}
            </Button>
          </Space>
        ))}
      </Space>
    </div>
  );
};

export default WorkbuddyIdentityPanel;
