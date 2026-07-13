import type { SessionTool } from './types';

export const SESSION_DETAIL_ROUTE_SUFFIX = '/sessions/detail';
export const SESSION_MANAGER_REFRESH_EVENT = 'session-manager-refresh';

export interface SessionDetailRouteParams {
  sourcePath: string;
  subagentSourcePath?: string;
}

export interface SessionManagerRefreshEventDetail {
  tool: SessionTool;
}

const SESSION_TOOL_BASE_PATH: Record<SessionTool, string> = {
  claudecode: '/coding/claudecode',
  codex: '/coding/codex',
  grok: '/coding/grok',
  geminicli: '/coding/geminicli',
  openclaw: '/coding/openclaw',
  opencode: '/coding/opencode',
  pi: '/coding/pi',
};

export const getSessionToolBasePath = (tool: SessionTool) => SESSION_TOOL_BASE_PATH[tool];

export const getSessionDetailRoutePath = (tool: SessionTool) => (
  `${getSessionToolBasePath(tool)}${SESSION_DETAIL_ROUTE_SUFFIX}`
);

export const buildSessionDetailPath = (
  tool: SessionTool,
  sourcePath: string,
  subagentSourcePath?: string,
) => {
  const params = new URLSearchParams();
  params.set('sourcePath', sourcePath);
  if (subagentSourcePath) {
    params.set('subagentSourcePath', subagentSourcePath);
  }
  return `${getSessionDetailRoutePath(tool)}?${params.toString()}`;
};

export const parseSessionDetailSearchParams = (
  searchParams: URLSearchParams,
): SessionDetailRouteParams | null => {
  const sourcePath = searchParams.get('sourcePath');
  if (!sourcePath) {
    return null;
  }

  const subagentSourcePath = searchParams.get('subagentSourcePath');
  return subagentSourcePath ? { sourcePath, subagentSourcePath } : { sourcePath };
};

export const dispatchSessionManagerRefresh = (tool: SessionTool) => {
  window.dispatchEvent(new CustomEvent<SessionManagerRefreshEventDetail>(
    SESSION_MANAGER_REFRESH_EVENT,
    { detail: { tool } },
  ));
};
