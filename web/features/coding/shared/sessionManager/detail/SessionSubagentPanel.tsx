import React from 'react';
import { Bot, ChevronDown, ChevronRight } from 'lucide-react';
import type { TFunction } from 'i18next';

import type { SessionSubagentMeta } from '../types';
import styles from './SessionDetailWorkbench.module.less';

interface SessionSubagentPanelProps {
  subagents: SessionSubagentMeta[];
  t: TFunction;
  onSelect?: (subagent: SessionSubagentMeta) => void;
}

const SessionSubagentPanel: React.FC<SessionSubagentPanelProps> = ({ subagents, t, onSelect }) => {
  const [open, setOpen] = React.useState(true);

  if (subagents.length === 0) {
    return null;
  }

  const ChevronIcon = open ? ChevronDown : ChevronRight;

  return (
    <section className={styles.subagentPanel}>
      <button
        type="button"
        className={styles.subagentPanelHeader}
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        <ChevronIcon size={14} aria-hidden="true" />
        <Bot size={16} aria-hidden="true" />
        <span>{t('sessionManager.subagentSessions')}</span>
        <span className={styles.subagentCount}>{subagents.length}</span>
      </button>
      {open ? (
        <div className={styles.subagentList}>
          {subagents.map((subagent) => (
            <button
              key={subagent.sourcePath}
              type="button"
              className={styles.subagentItem}
              title={subagent.summary || subagent.title}
              onClick={() => onSelect?.(subagent)}
            >
              <Bot size={14} aria-hidden="true" />
              <span className={styles.subagentTitle}>{subagent.title}</span>
              {subagent.subagentType ? <span className={styles.subagentType}>{subagent.subagentType}</span> : null}
              <span className={styles.subagentMessages}>
                {t('sessionManager.subagentMessageCount', { count: subagent.messageCount })}
              </span>
            </button>
          ))}
        </div>
      ) : null}
    </section>
  );
};

export default SessionSubagentPanel;
