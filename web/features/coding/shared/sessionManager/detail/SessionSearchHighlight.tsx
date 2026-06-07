import React from 'react';

import styles from './SessionDetailWorkbench.module.less';

interface SessionSearchHighlightProps {
  text: string;
  query: string;
}

const SessionSearchHighlight: React.FC<SessionSearchHighlightProps> = ({ text, query }) => {
  const normalizedQuery = query.trim();
  if (!normalizedQuery) {
    return <>{text}</>;
  }

  const lowerText = text.toLowerCase();
  const lowerQuery = normalizedQuery.toLowerCase();
  const parts: React.ReactNode[] = [];
  let cursor = 0;

  while (cursor < text.length) {
    const index = lowerText.indexOf(lowerQuery, cursor);
    if (index < 0) {
      parts.push(text.slice(cursor));
      break;
    }

    if (index > cursor) {
      parts.push(text.slice(cursor, index));
    }

    parts.push(
      <mark key={`${index}-${lowerQuery}`} className={styles.searchMark}>
        {text.slice(index, index + normalizedQuery.length)}
      </mark>,
    );
    cursor = index + normalizedQuery.length;
  }

  return <>{parts}</>;
};

export default SessionSearchHighlight;
