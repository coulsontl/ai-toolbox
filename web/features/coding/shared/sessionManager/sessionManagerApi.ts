import { invoke } from '@tauri-apps/api/core';

import type {
  DeleteToolSessionsResult,
  ExportToolSessionsResult,
  SessionDetail,
  SessionListPage,
  SessionListLoadMode,
  SessionSourceMode,
  SessionSubagentMeta,
  SessionTool,
  SessionExportFormat,
} from './types';

interface ListToolSessionsInput {
  tool: SessionTool;
  query?: string;
  pathFilter?: string;
  page?: number;
  pageSize?: number;
  forceRefresh?: boolean;
  sourceMode?: SessionSourceMode;
  loadMode?: SessionListLoadMode;
}

const pendingListToolSessions = new Map<string, Promise<SessionListPage>>();

const buildListToolSessionsKey = (input: Required<ListToolSessionsInput>) => JSON.stringify(input);

export const listToolSessions = async ({
  tool,
  query,
  pathFilter,
  page = 1,
  pageSize = 10,
  forceRefresh = false,
  sourceMode = 'all',
  loadMode = 'auto',
}: ListToolSessionsInput): Promise<SessionListPage> => {
  const input = {
    tool,
    query: query ?? '',
    pathFilter: pathFilter ?? '',
    page,
    pageSize,
    forceRefresh,
    sourceMode,
    loadMode,
  };
  const pendingKey = buildListToolSessionsKey(input);
  const pendingRequest = pendingListToolSessions.get(pendingKey);
  if (pendingRequest) {
    return await pendingRequest;
  }

  const request = invoke<SessionListPage>('list_tool_sessions', {
    tool: input.tool,
    query: input.query || undefined,
    pathFilter: input.pathFilter || undefined,
    page: input.page,
    pageSize: input.pageSize,
    forceRefresh: input.forceRefresh,
    sourceMode: input.sourceMode,
    loadMode: input.loadMode,
  }).finally(() => {
    pendingListToolSessions.delete(pendingKey);
  });
  pendingListToolSessions.set(pendingKey, request);
  return await request;
};

export const listToolSessionPaths = async (
  tool: SessionTool,
  limit = 200,
  forceRefresh = false,
): Promise<string[]> => {
  return await invoke<string[]>('list_tool_session_paths', {
    tool,
    limit,
    forceRefresh,
  });
};

export const getToolSessionDetail = async (
  tool: SessionTool,
  sourcePath: string,
): Promise<SessionDetail> => {
  return await invoke<SessionDetail>('get_tool_session_detail', {
    tool,
    sourcePath,
  });
};

export const listToolSessionSubagents = async (
  tool: SessionTool,
  sourcePath: string,
): Promise<SessionSubagentMeta[]> => {
  return await invoke<SessionSubagentMeta[]>('list_tool_session_subagents', {
    tool,
    sourcePath,
  });
};

export const getToolSubagentSessionDetail = async (
  tool: SessionTool,
  parentSourcePath: string,
  subagentSourcePath: string,
): Promise<SessionDetail> => {
  return await invoke<SessionDetail>('get_tool_subagent_session_detail', {
    tool,
    parentSourcePath,
    subagentSourcePath,
  });
};

export const deleteToolSession = async (
  tool: SessionTool,
  sourcePath: string,
): Promise<void> => {
  await invoke('delete_tool_session', {
    tool,
    sourcePath,
  });
};

export const deleteToolSessions = async (
  tool: SessionTool,
  sourcePaths: string[],
): Promise<DeleteToolSessionsResult> => {
  return await invoke<DeleteToolSessionsResult>('delete_tool_sessions', {
    tool,
    sourcePaths,
  });
};

export const exportToolSession = async (
  tool: SessionTool,
  sourcePath: string,
  exportPath: string,
  exportFormat: SessionExportFormat = 'ai_toolbox',
): Promise<void> => {
  await invoke('export_tool_session', {
    tool,
    sourcePath,
    exportPath,
    exportFormat,
  });
};

export const exportToolSessions = async (
  tool: SessionTool,
  sourcePaths: string[],
  exportDir: string,
  exportFormat: SessionExportFormat = 'ai_toolbox',
): Promise<ExportToolSessionsResult> => {
  return await invoke<ExportToolSessionsResult>('export_tool_sessions', {
    tool,
    sourcePaths,
    exportDir,
    exportFormat,
  });
};

export const importToolSession = async (
  tool: SessionTool,
  importPath: string,
): Promise<void> => {
  await invoke('import_tool_session', {
    tool,
    importPath,
  });
};

export const renameToolSession = async (
  tool: SessionTool,
  sourcePath: string,
  title: string,
): Promise<void> => {
  await invoke('rename_tool_session', {
    tool,
    sourcePath,
    title,
  });
};
