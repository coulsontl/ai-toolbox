import React from 'react';
import { Empty } from 'antd';
import type { TFunction } from 'i18next';

import type { SessionMessage } from '../types';
import type { SessionContentFilter } from './domain/messageFilters';
import type { SessionDisplayRow } from './domain/messageFlatten';
import SessionMessageCard from './SessionMessageCard';
import styles from './SessionDetailWorkbench.module.less';

interface SessionMessageViewerProps {
  rows: SessionDisplayRow[];
  activeMessageIndex: number | null;
  query: string;
  contentFilter: SessionContentFilter;
  assistantLabel: string;
  t: TFunction;
  viewerRef: React.RefObject<HTMLDivElement | null>;
  onCopyText: (text: string, successText: string) => void | Promise<void>;
  setMessageRef: (index: number, node: HTMLElement | null) => void;
}

const SessionMessageViewer: React.FC<SessionMessageViewerProps> = ({
  rows,
  activeMessageIndex,
  query,
  contentFilter,
  assistantLabel,
  t,
  viewerRef,
  onCopyText,
  setMessageRef,
}) => {
  if (rows.length === 0) {
    return (
      <div className={styles.viewerEmpty}>
        <Empty description={t('sessionManager.noMessagesMatched')} />
      </div>
    );
  }

  return (
    <div ref={viewerRef} className={styles.messageViewer}>
      {rows.map((row) => {
        if (row.type === 'date') {
          return (
            <div key={row.id} className={styles.dateDivider}>
              <span>{row.label}</span>
            </div>
          );
        }

        return (
          <SessionMessageCard
            key={row.id}
            message={row.message as SessionMessage}
            index={row.index}
            active={activeMessageIndex === row.index}
            query={query}
            contentFilter={contentFilter}
            assistantLabel={assistantLabel}
            t={t}
            onCopyText={onCopyText}
            setMessageRef={setMessageRef}
          />
        );
      })}
    </div>
  );
};

export default SessionMessageViewer;
