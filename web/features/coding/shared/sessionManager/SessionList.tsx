import React from 'react';
import { ClockCircleOutlined, CopyOutlined, DeleteOutlined, FolderOpenOutlined } from '@ant-design/icons';
import { Button, Checkbox, Tag } from 'antd';
import { useTranslation } from 'react-i18next';

import { VirtualGrid } from '@/features/coding/shared/management';

import type { SessionMeta } from './types';
import { formatRelativeTime, formatSessionTitle, shortSessionId } from './utils';
import styles from './SessionManagerPanel.module.less';

interface SessionActions {
  onOpenDetail: (session: SessionMeta) => void;
  onToggleSelection: (session: SessionMeta) => void;
  onCopyResume: (command: string) => Promise<void>;
  onDelete: (session: SessionMeta) => void;
}

interface SessionCardProps extends SessionActions {
  session: SessionMeta;
  selected: boolean;
  selectionMode: boolean;
  showRuntimeSourceTag: boolean;
}

const SessionCard = React.memo(function SessionCard({
  session,
  selected,
  selectionMode,
  showRuntimeSourceTag,
  onOpenDetail,
  onToggleSelection,
  onCopyResume,
  onDelete,
}: SessionCardProps) {
  const { t } = useTranslation();
  const displayTime = session.lastActiveAt || session.createdAt;
  const runtimeSourceLabel = session.runtimeSource === 'wsl'
    ? session.runtimeDistro
      ? t('sessionManager.sourceMode.wslWithDistro', { distro: session.runtimeDistro })
      : t('sessionManager.sourceMode.wsl')
    : t('sessionManager.sourceMode.local');

  return (
    <div
      className={`${styles.sessionCard}${selected ? ` ${styles.sessionCardSelected}` : ''}`}
      onClick={() => selectionMode ? onToggleSelection(session) : onOpenDetail(session)}
    >
      <div className={styles.sessionHeader}>
        {selectionMode ? (
          <Checkbox
            className={styles.sessionCheckbox}
            aria-label={formatSessionTitle(session)}
            checked={selected}
            onChange={() => onToggleSelection(session)}
            onClick={(event) => event.stopPropagation()}
          />
        ) : null}
        <div className={styles.sessionHeaderMain}>
          <div className={styles.sessionTitleRow}>
            <button type="button" className={styles.sessionTitle} aria-pressed={selectionMode ? selected : undefined}>
              {formatSessionTitle(session)}
            </button>
          </div>
          <div className={styles.sessionMetaRow}>
            <span><ClockCircleOutlined style={{ marginRight: 4 }} />{formatRelativeTime(displayTime, t)}</span>
            {showRuntimeSourceTag && session.runtimeSource ? (
              <Tag
                bordered={false}
                className={session.runtimeSource === 'wsl' ? styles.runtimeSourceTagWsl : styles.runtimeSourceTagLocal}
              >
                {runtimeSourceLabel}
              </Tag>
            ) : null}
            <span>{shortSessionId(session.sessionId)}</span>
            {session.projectDir ? (
              <span className={styles.sessionProjectDir}><FolderOpenOutlined style={{ marginRight: 4 }} />{session.projectDir}</span>
            ) : null}
          </div>
        </div>
        <div className={styles.sessionActions} onClick={(event) => event.stopPropagation()}>
          <Button
            type="link"
            size="small"
            className={styles.actionButton}
            icon={<CopyOutlined />}
            disabled={!session.resumeCommand}
            onClick={() => {
              if (session.resumeCommand) void onCopyResume(session.resumeCommand);
            }}
          >
            {t('sessionManager.copyResume')}
          </Button>
          <Button
            type="link"
            size="small"
            danger
            className={styles.actionButton}
            icon={<DeleteOutlined />}
            disabled={selectionMode}
            onClick={() => onDelete(session)}
          >
            {t('common.delete')}
          </Button>
        </div>
      </div>
    </div>
  );
});

interface SessionListProps extends SessionActions {
  items: SessionMeta[];
  selectedSourcePaths: ReadonlySet<string>;
  selectionMode: boolean;
  showRuntimeSourceTag: boolean;
}

const getSessionKey = (session: SessionMeta) => `${session.providerId}-${session.sessionId}-${session.sourcePath}`;

const SessionList = React.memo(function SessionList({
  items,
  selectedSourcePaths,
  selectionMode,
  showRuntimeSourceTag,
  onOpenDetail,
  onToggleSelection,
  onCopyResume,
  onDelete,
}: SessionListProps) {
  const renderSession = React.useCallback((session: SessionMeta) => (
    <SessionCard
      session={session}
      selected={selectedSourcePaths.has(session.sourcePath)}
      selectionMode={selectionMode}
      showRuntimeSourceTag={showRuntimeSourceTag}
      onOpenDetail={onOpenDetail}
      onToggleSelection={onToggleSelection}
      onCopyResume={onCopyResume}
      onDelete={onDelete}
    />
  ), [selectedSourcePaths, selectionMode, showRuntimeSourceTag, onOpenDetail, onToggleSelection, onCopyResume, onDelete]);

  return (
    <VirtualGrid
      items={items}
      getKey={getSessionKey}
      renderItem={renderSession}
      columns={1}
      rowGap={12}
      defaultRowHeight={80}
    />
  );
});

export default SessionList;
