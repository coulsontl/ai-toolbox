import React from 'react';
import { Alert, App, Collapse, Spin, Typography } from 'antd';
import { FileTextOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  getWorkbuddyIdentityState,
  saveWorkbuddyIdentityFile,
} from '@/services/workbuddyApi';
import type { WorkbuddyIdentityState } from '@/types/workbuddy';
import type { GlobalPromptConfig } from '@/types/globalPrompt';
import GlobalPromptConfigCard from '@/features/coding/shared/prompt/GlobalPromptConfigCard';
import GlobalPromptConfigModal, {
  type GlobalPromptConfigFormValues,
} from '@/features/coding/shared/prompt/GlobalPromptConfigModal';
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

/** The fixed document set, in the order the app seeds them. */
const IDENTITY_ORDER = ['IDENTITY.md', 'USER.md', 'SOUL.md', 'BOOTSTRAP.md'];

/**
 * The four per-account identity documents, drawn as the global-prompt list.
 *
 * They reuse that section's card, dialog and collapse — same chrome, same
 * preview, same editor — because to a reader they *are* prompts. What differs
 * is what the set allows, and each difference is structural rather than a
 * preference:
 *
 * - **No add**: the app reads exactly these four files. A fifth row would be a
 *   name the product never opens.
 * - **No delete**: deleting the file is not deleting a preset, it is blanking a
 *   document the assistant runs on. Saving an empty body is the reversible way
 *   to empty one.
 * - **No apply / disable badge**: there is no "current" preset here — all four
 *   are live at once, which is exactly why this is not a preset list.
 * - **No reorder**: the file *is* the record; order has nowhere to be written.
 *
 * Editing is the one operation, and it writes straight to disk.
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
  const [editing, setEditing] = React.useState<GlobalPromptConfig | null>(null);

  const load = React.useCallback(async () => {
    setLoading(true);
    try {
      setState(await getWorkbuddyIdentityState());
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

  const handleSave = async (values: GlobalPromptConfigFormValues) => {
    if (!editing) {
      return;
    }
    try {
      setState(await saveWorkbuddyIdentityFile({
        fileName: editing.id,
        content: values.content,
      }));
      setEditing(null);
      message.success(t('workbuddy.identity.saved', { name: editing.name }));
      await onSaved?.();
    } catch (error) {
      console.error('Failed to save WorkBuddy identity file:', error);
      message.error(String(error));
    }
  };

  const content = (
    <Spin spinning={loading}>
      <div className={styles.hintBlock}>
        <div>{t('workbuddy.identity.hint')}</div>
        <Text type="secondary" copyable={{ text: state?.identityDir }}>
          {t('workbuddy.identity.dir')}: {state?.identityDir}
        </Text>
      </div>
      <div>
        {(state?.files ?? [])
          // The backend already returns the four files; the sort just pins the
          // order the app itself uses, so a backend reordering cannot reshuffle
          // what the user reads as a fixed set.
          .slice()
          .sort((left, right) =>
            IDENTITY_ORDER.indexOf(left.fileName) - IDENTITY_ORDER.indexOf(right.fileName))
          .map((file) => (
            <GlobalPromptConfigCard
              key={file.fileName}
              config={{
                id: file.fileName,
                name: file.fileName,
                content: file.content,
                isApplied: false,
              }}
              hint={!file.exists
                ? (
                  <Text type="secondary" className={styles.cardHint}>
                    ({t('workbuddy.identity.missing')})
                  </Text>
                )
                : undefined}
              reorderable={false}
              onEdit={setEditing}
            />
          ))}
      </div>
    </Spin>
  );

  return (
    <>
      <Collapse
        style={{ marginBottom: 16 }}
        items={[{
          key: 'workbuddy-identity',
          label: (
            <Text strong>
              <FileTextOutlined style={{ marginRight: 8 }} />
              {t('workbuddy.identity.title')}
            </Text>
          ),
          children: state?.unavailableReason
            ? <Alert type="info" showIcon message={state.unavailableReason} />
            : content,
        }]}
      />

      <GlobalPromptConfigModal
        open={Boolean(editing)}
        // The name is the file on disk; there is nothing to rename it to.
        nameReadOnly
        toolName="WorkBuddy"
        promptFileName={editing?.id ?? ''}
        initialValues={editing ?? undefined}
        onCancel={() => setEditing(null)}
        onSuccess={handleSave}
      />
    </>
  );
};

export default WorkbuddyIdentityPanel;
