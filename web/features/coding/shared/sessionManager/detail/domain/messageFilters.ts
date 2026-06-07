import type { SessionMessage } from '../../types';

import { getMessageBlocks, isToolBlock } from './messageBlocks';
import { hasSessionCommandTags } from './commandTags';
import { messageMatchesQuery, type SessionSearchScope } from './messageSearch';
import { pairToolBlocks } from './toolPairing';

export type SessionRoleFilterKey = 'user' | 'assistant';
export type SessionContentFilterKey = 'text' | 'thinking' | 'tool_call' | 'command';
export type SessionRoleFilter = Record<SessionRoleFilterKey, boolean>;
export type SessionContentFilter = Record<SessionContentFilterKey, boolean>;

export const DEFAULT_SESSION_ROLE_FILTER: SessionRoleFilter = {
  user: true,
  assistant: true,
};

export const DEFAULT_SESSION_CONTENT_FILTER: SessionContentFilter = {
  text: true,
  thinking: true,
  tool_call: true,
  command: true,
};

export interface SessionDetailFilters {
  query: string;
  roleFilter: SessionRoleFilter;
  contentFilter: SessionContentFilter;
  searchScope: SessionSearchScope;
}

export function filterSessionMessages(messages: SessionMessage[], filters: SessionDetailFilters): Array<{ message: SessionMessage; index: number }> {
  return messages
    .map((message, index) => ({ message, index }))
    .filter(({ message }) => {
      return matchesRoleFilter(message, filters.roleFilter)
        && matchesContentFilter(message, filters.contentFilter)
        && messageMatchesQuery(message, filters.query, filters.searchScope);
    });
}

export function matchesRoleFilter(message: SessionMessage, roleFilter: SessionRoleFilter): boolean {
  const role = message.role.toLowerCase();
  if (role === 'user') {
    return roleFilter.user;
  }
  if (role === 'assistant') {
    return roleFilter.assistant;
  }

  return true;
}

export function matchesContentFilter(message: SessionMessage, contentFilter: SessionContentFilter): boolean {
  if (isContentFilterFullyVisible(contentFilter)) {
    return true;
  }

  return getVisibleMessageBlocks(message, contentFilter).length > 0;
}

export function getVisibleMessageBlocks(message: SessionMessage, contentFilter: SessionContentFilter) {
  const blocks = pairToolBlocks(getMessageBlocks(message));
  if (isContentFilterFullyVisible(contentFilter)) {
    return blocks;
  }

  return blocks.filter((block) => matchesBlockContentFilter(block, contentFilter));
}

function matchesBlockContentFilter(block: ReturnType<typeof getMessageBlocks>[number], contentFilter: SessionContentFilter): boolean {
  if (hasSessionCommandTags(block.text || '')) {
    return contentFilter.command;
  }

  if (isToolBlock(block)) {
    return contentFilter.tool_call;
  }

  if (block.kind === 'thinking' || block.kind === 'redacted_thinking') {
    return contentFilter.thinking;
  }

  if (block.kind === 'text' || block.kind === 'system') {
    return contentFilter.text;
  }

  return true;
}

function isContentFilterFullyVisible(contentFilter: SessionContentFilter): boolean {
  return contentFilter.text
    && contentFilter.thinking
    && contentFilter.tool_call
    && contentFilter.command;
}
