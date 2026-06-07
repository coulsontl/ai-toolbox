import type { SessionMessage } from '../../types';

export type SessionDisplayRow =
  | { type: 'date'; id: string; label: string }
  | { type: 'message'; id: string; message: SessionMessage; index: number };

export function flattenMessagesWithDateDividers(items: Array<{ message: SessionMessage; index: number }>): SessionDisplayRow[] {
  const rows: SessionDisplayRow[] = [];
  let previousDateKey = '';

  items.forEach(({ message, index }) => {
    const dateKey = getDateKey(message.ts);
    if (dateKey && dateKey !== previousDateKey) {
      rows.push({
        type: 'date',
        id: `date-${dateKey}`,
        label: formatDateLabel(message.ts),
      });
      previousDateKey = dateKey;
    }

    rows.push({
      type: 'message',
      id: message.id || `message-${index}`,
      message,
      index,
    });
  });

  return rows;
}

function getDateKey(timestamp?: number): string {
  if (!timestamp) {
    return '';
  }
  const date = new Date(timestamp);
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

function formatDateLabel(timestamp?: number): string {
  if (!timestamp) {
    return '';
  }
  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    weekday: 'long',
  }).format(new Date(timestamp));
}
