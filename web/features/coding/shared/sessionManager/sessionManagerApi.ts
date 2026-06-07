import { invoke } from '@tauri-apps/api/core';

import type {
  DeleteToolSessionsResult,
  SessionDetail,
  SessionListPage,
  SessionSubagentMeta,
  SessionTool,
} from './types';

interface ListToolSessionsInput {
  tool: SessionTool;
  query?: string;
  pathFilter?: string;
  page?: number;
  pageSize?: number;
  forceRefresh?: boolean;
}

export const listToolSessions = async ({
  tool,
  query,
  pathFilter,
  page = 1,
  pageSize = 10,
  forceRefresh = false,
}: ListToolSessionsInput): Promise<SessionListPage> => {
  return await invoke<SessionListPage>('list_tool_sessions', {
    tool,
    query,
    pathFilter,
    page,
    pageSize,
    forceRefresh,
  });
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
): Promise<void> => {
  await invoke('export_tool_session', {
    tool,
    sourcePath,
    exportPath,
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
