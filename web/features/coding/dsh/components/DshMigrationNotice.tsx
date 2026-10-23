import React from 'react';
import { Alert } from 'antd';
import { CloseOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';

/**
 * Which migration notice the user already closed, stored as the profile patch
 * path it announced. Keyed by path rather than a plain boolean so pinning a
 * different migrated profile shows the notice again.
 */
const MIGRATION_NOTICE_DISMISSED_KEY = 'dsh.migrationNoticeDismissedPath';

const readDismissedMigrationPath = (): string | undefined => {
  try {
    return localStorage.getItem(MIGRATION_NOTICE_DISMISSED_KEY) ?? undefined;
  } catch {
    // Storage unavailable (private mode, disabled): the notice simply returns
    // on the next visit, which is the pre-dismissal behaviour.
    return undefined;
  }
};

interface DshMigrationNoticeProps {
  /** The patch path dsh imported `settings.yaml` into. */
  patchPath: string;
  /** Where the original `settings.yaml` was renamed to. */
  importedPath: string;
}

/**
 * One-line notice that `settings.yaml` has been imported into the profile layer.
 *
 * Lives in its own component because the page cannot be mounted in a test
 * fixture (30+ Tauri commands) and the browser check needs to measure the exact
 * markup it ships. Copying the props into a fixture instead would let the two
 * drift, and the thing this guards — "is there actually a visible ×" — is
 * decided entirely by the props.
 *
 * Two antd 6 details are load-bearing:
 * - `closable` only counts as closable when the object carries `closeIcon` or
 *   `closeText`, or is a boolean. A bare `{ onClose }` renders **no × at all**
 *   while typing perfectly — that is the bug this component was extracted from.
 * - the text goes in `title` (not `message` + `description`): a two-line alert
 *   banner was reported as too large for a one-time, informational notice.
 */
const DshMigrationNotice: React.FC<DshMigrationNoticeProps> = ({ patchPath, importedPath }) => {
  const { t } = useTranslation();
  const [dismissedPath, setDismissedPath] = React.useState<string | undefined>(readDismissedMigrationPath);

  if (patchPath === dismissedPath) {
    return null;
  }

  const dismiss = () => {
    setDismissedPath(patchPath);
    try {
      localStorage.setItem(MIGRATION_NOTICE_DISMISSED_KEY, patchPath);
    } catch {
      // Storage unavailable: dismissed for this visit only.
    }
  };

  return (
    <Alert
      type="info"
      // 12px, not 16: the notice is a note about the header it sits under, and
      // the first section spaces itself 16px from the header block as a whole.
      style={{ marginBottom: 12, padding: '4px 10px' }}
      closable={{ closeIcon: <CloseOutlined />, onClose: dismiss }}
      title={(
        <span data-dsh-migration-notice style={{ fontSize: 12 }}>
          {t('dsh.profile.migrationNotice', {
            defaultValue:
              '配置已迁移到 profile 层：settings.yaml 已导入 {{patchPath}} 并改名为 {{importedPath}}，旧文件不再生效。',
            patchPath,
            importedPath,
          })}
        </span>
      )}
    />
  );
};

export default DshMigrationNotice;
