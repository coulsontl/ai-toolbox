import type { TFunction } from 'i18next';

import type {
  SessionMessage,
  SessionMeta,
  SessionSourceMode,
  SessionSourceOption,
  SessionTimeRange,
  SessionTocItem,
} from './types';

// Wire values must mirror the backend `SessionTimeRange::parse` presets
// (all / today / 7d / 30d / older_30d) one to one.
export const SESSION_TIME_RANGE_OPTIONS: ReadonlyArray<{
  value: SessionTimeRange;
  labelKey: string;
}> = [
  { value: 'all', labelKey: 'sessionManager.timeRange.all' },
  { value: 'today', labelKey: 'sessionManager.timeRange.today' },
  { value: '7d', labelKey: 'sessionManager.timeRange.last7Days' },
  { value: '30d', labelKey: 'sessionManager.timeRange.last30Days' },
  { value: 'older_30d', labelKey: 'sessionManager.timeRange.older30Days' },
];

export function advanceVisibleContextId(
  currentVisibleContextId: number,
  wasActive: boolean,
  isActive: boolean,
): number {
  return wasActive && !isActive ? currentVisibleContextId + 1 : currentVisibleContextId;
}

export function shouldShowVisibleFeedback(
  isPageActive: boolean,
  requestVisibleContextId: number | undefined,
  currentVisibleContextId: number,
): boolean {
  if (!isPageActive) {
    return false;
  }

  if (requestVisibleContextId === undefined) {
    return true;
  }

  return requestVisibleContextId === currentVisibleContextId;
}

export function resolveEffectiveSessionSourceMode(
  sourceMode: SessionSourceMode,
  availableSources: SessionSourceOption[],
): SessionSourceMode {
  const hasLocalSource = availableSources.some((item) => item.source === 'local');
  const hasWslSource = availableSources.some((item) => item.source === 'wsl');
  return hasLocalSource && hasWslSource ? sourceMode : 'all';
}

export function formatSessionTitle(session: SessionMeta): string {
  if (session.title?.trim()) {
    return session.title.trim();
  }

  if (session.projectDir?.trim()) {
    const normalized = session.projectDir.replace(/[\\/]+$/, '');
    const basename = normalized.split(/[\\/]/).pop();
    if (basename?.trim()) {
      return basename.trim();
    }
  }

  return shortSessionId(session.sessionId);
}

export function shortSessionId(sessionId: string): string {
  if (sessionId.length <= 12) {
    return sessionId;
  }
  return `${sessionId.slice(0, 8)}...${sessionId.slice(-4)}`;
}

export function formatTimestamp(timestamp?: number): string {
  if (!timestamp) {
    return '';
  }

  return new Date(timestamp).toLocaleString();
}

export function formatRelativeTime(timestamp: number | undefined, t: TFunction): string {
  if (!timestamp) {
    return t('common.notSet');
  }

  const date = new Date(timestamp);
  const diffMs = Date.now() - timestamp;
  const diffMinutes = Math.floor(diffMs / 60_000);
  if (diffMinutes < 1) {
    return t('sessionManager.justNow');
  }
  if (diffMinutes < 60) {
    return t('sessionManager.minutesAgo', { count: diffMinutes });
  }

  const diffHours = Math.floor(diffMinutes / 60);
  if (diffHours < 24) {
    return t('sessionManager.hoursAgo', { count: diffHours });
  }

  const diffDays = Math.floor(diffHours / 24);
  if (diffDays > 7) {
    return date.toLocaleString();
  }
  return t('sessionManager.daysAgo', { count: diffDays });
}

export function getRoleLabel(role: string, t: TFunction): string {
  const normalizedRole = role.toLowerCase();
  if (normalizedRole === 'user') {
    return t('sessionManager.roles.user');
  }
  if (normalizedRole === 'assistant') {
    return t('sessionManager.roles.assistant');
  }
  if (normalizedRole === 'tool') {
    return t('sessionManager.roles.tool');
  }
  if (normalizedRole === 'system') {
    return t('sessionManager.roles.system');
  }
  return role;
}

export function getToolLabel(tool: SessionMeta['providerId'], t: TFunction): string {
  switch (tool) {
    case 'claudecode':
      return t('subModules.claudecode');
    case 'geminicli':
      return t('subModules.geminicli');
    case 'openclaw':
      return t('subModules.openclaw');
    case 'opencode':
      return t('subModules.opencode');
    case 'pi':
      return t('subModules.pi');
    case 'dsh':
      return t('subModules.dsh');
    case 'workbuddy':
      return t('subModules.workbuddy');
    case 'codebuddy':
      return t('subModules.codebuddy');
    case 'codex':
    default:
      return t('subModules.codex');
  }
}

export function buildSessionTocItems(messages: SessionMessage[]): SessionTocItem[] {
  return messages
    .map((message, index) => ({ message, index }))
    .filter(({ message }) => message.role.toLowerCase() === 'user')
    .map(({ message, index }) => ({
      index,
      preview: createPreview(message.content),
      ts: message.ts,
    }));
}

export function createPreview(content: string, maxLength = 80): string {
  const collapsed = content.replace(/\s+/g, ' ').trim();
  if (collapsed.length <= maxLength) {
    return collapsed;
  }
  return `${collapsed.slice(0, maxLength)}...`;
}

export function shouldCollapseMessage(role: string, content: string): boolean {
  const normalizedRole = role.toLowerCase();
  const lineCount = content.split('\n').length;

  if (
    normalizedRole === 'assistant'
    || normalizedRole === 'tool'
    || normalizedRole === 'developer'
  ) {
    return lineCount > 2;
  }

  return lineCount > 20 || content.length > 1500;
}

export function usesCompactMessageCollapse(role: string): boolean {
  const normalizedRole = role.toLowerCase();
  return (
    normalizedRole === 'assistant'
    || normalizedRole === 'tool'
    || normalizedRole === 'developer'
  );
}
