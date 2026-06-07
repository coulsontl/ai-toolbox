import React from 'react';

import { valueToSearchText } from './domain/messageBlocks';
import SessionAnsiText from './SessionAnsiText';
import SessionSearchHighlight from './SessionSearchHighlight';
import styles from './SessionDetailWorkbench.module.less';

interface SessionToolResultBlockProps {
  output: unknown;
  query: string;
  status?: string;
}

const SessionToolResultBlock: React.FC<SessionToolResultBlockProps> = ({ output, query, status }) => {
  if (output === undefined || output === null || valueToSearchText(output).trim() === '') {
    const normalizedStatus = status?.toLowerCase();
    return (
      <div className={styles.resultMuted}>
        {normalizedStatus === 'pending' || normalizedStatus === 'running' ? 'Pending' : 'No output'}
      </div>
    );
  }

  if (typeof output === 'string') {
    return <PreBlock text={output} query={query} />;
  }

  if (Array.isArray(output)) {
    return <PreBlock text={valueToSearchText(output)} query={query} />;
  }

  if (isRecord(output)) {
    const stdout = getStringField(output, ['stdout', 'output']);
    const stderr = getStringField(output, ['stderr', 'error']);
    const returnCode = output.returnCode ?? output.return_code ?? output.exitCode ?? output.exit_code;

    if (stdout || stderr || returnCode !== undefined) {
      return (
        <div className={styles.resultStack}>
          {returnCode !== undefined ? (
            <div className={styles.metaRow}>
              <span>Exit code</span>
              <code>{String(returnCode)}</code>
            </div>
          ) : null}
          {stdout ? <PreBlock title="stdout" text={stdout} query={query} /> : null}
          {stderr ? <PreBlock title="stderr" text={stderr} query={query} tone="error" /> : null}
        </div>
      );
    }
  }

  return <PreBlock text={safeJson(output)} query={query} />;
};

interface PreBlockProps {
  title?: string;
  text: string;
  query: string;
  tone?: 'default' | 'error';
}

const PreBlock: React.FC<PreBlockProps> = ({ title, text, query, tone = 'default' }) => (
  <div className={styles.preBlockShell}>
    {title ? <div className={styles.preBlockTitle}>{title}</div> : null}
    <pre className={`${styles.preBlock}${tone === 'error' ? ` ${styles.preBlockError}` : ''}`}>
      {query.trim() ? <SessionSearchHighlight text={text} query={query} /> : <SessionAnsiText text={text} />}
    </pre>
  </div>
);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function getStringField(record: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim()) {
      return value;
    }
  }
  return '';
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export default SessionToolResultBlock;
