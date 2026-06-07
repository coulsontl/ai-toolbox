import type { SessionMessage } from '../../types';

import { getBlockText, getMessageBlocks } from './messageBlocks';

export type SessionSearchScope = 'content' | 'toolId';

export function getMessageSearchText(message: SessionMessage, scope: SessionSearchScope = 'content'): string {
  if (scope === 'toolId') {
    return getMessageBlocks(message)
      .map((block) => block.toolId)
      .filter(Boolean)
      .join('\n');
  }

  return [
    message.role,
    message.content,
    message.model,
    message.messageType,
    ...getMessageBlocks(message).map(getBlockText),
  ]
    .filter(Boolean)
    .join('\n');
}

export function countQueryMatches(text: string, query: string): number {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) {
    return 0;
  }

  let count = 0;
  let cursor = 0;
  const normalizedText = text.toLowerCase();
  while (cursor < normalizedText.length) {
    const index = normalizedText.indexOf(normalizedQuery, cursor);
    if (index < 0) {
      break;
    }
    count += 1;
    cursor = index + Math.max(normalizedQuery.length, 1);
  }
  return count;
}

export function messageMatchesQuery(message: SessionMessage, query: string, scope: SessionSearchScope = 'content'): boolean {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) {
    return true;
  }
  return getMessageSearchText(message, scope).toLowerCase().includes(normalizedQuery);
}
