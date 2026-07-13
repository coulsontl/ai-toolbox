export type SessionTool =
  | 'codex'
  | 'grok'
  | 'claudecode'
  | 'geminicli'
  | 'openclaw'
  | 'opencode'
  | 'pi';

export type SessionSourceMode = 'all' | 'local' | 'wsl';
export type SessionListLoadMode = 'auto' | 'cache-first' | 'full' | 'refresh';
export type SessionListCacheState = 'none' | 'quick' | 'stale' | 'fresh';
export type SessionExportFormat = 'ai_toolbox' | 'grok_markdown' | 'grok_native';

export interface SessionMeta {
  providerId: SessionTool;
  sessionId: string;
  title?: string;
  summary?: string;
  projectDir?: string | null;
  createdAt?: number;
  lastActiveAt?: number;
  sourcePath: string;
  resumeCommand?: string | null;
  runtimeSource?: 'local' | 'wsl';
  runtimeDistro?: string | null;
}

export interface SessionMessage {
  role: string;
  content: string;
  ts?: number;
  id?: string;
  parentId?: string;
  messageType?: string;
  blocks?: SessionMessageBlock[];
  model?: string;
  usage?: SessionMessageUsage;
  durationMs?: number;
  costUsd?: number;
  isSidechain?: boolean;
  metadata?: unknown;
}

export interface SessionMessageUsage {
  inputTokens?: number;
  outputTokens?: number;
  cacheCreationInputTokens?: number;
  cacheReadInputTokens?: number;
}

export interface SessionMessageBlock {
  kind: string;
  text?: string;
  title?: string;
  variant?: string;
  language?: string;
  toolId?: string;
  toolName?: string;
  normalizedToolName?: string;
  status?: string;
  isError?: boolean;
  input?: unknown;
  output?: unknown;
  metadata?: unknown;
}

export interface SessionListPage {
  items: SessionMeta[];
  page: number;
  pageSize: number;
  total: number;
  hasMore: boolean;
  partial?: boolean;
  cacheState?: SessionListCacheState;
  metaComplete?: boolean;
  messageSearchComplete?: boolean;
  availablePaths?: string[];
  availableSources?: SessionSourceOption[];
}

export interface SessionDetail {
  meta: SessionMeta;
  messages: SessionMessage[];
}

export interface SessionSubagentMeta {
  id: string;
  sourcePath: string;
  title: string;
  summary?: string;
  subagentType?: string;
  messageCount: number;
  firstMessageTime?: number;
  lastMessageTime?: number;
}

export interface DeleteSessionFailure {
  sourcePath: string;
  error: string;
}

export interface DeleteToolSessionsResult {
  deletedCount: number;
  failedItems: DeleteSessionFailure[];
}

export interface ExportSessionItem {
  sourcePath: string;
  exportPath: string;
}

export interface ExportSessionFailure {
  sourcePath: string;
  error: string;
}

export interface ExportToolSessionsResult {
  exportedCount: number;
  exportedItems: ExportSessionItem[];
  failedItems: ExportSessionFailure[];
}

export interface SessionTocItem {
  index: number;
  preview: string;
  ts?: number;
}

export interface SessionPathOption {
  label: string;
  value: string;
}

export interface SessionSourceOption {
  source: 'local' | 'wsl';
  distro?: string | null;
}
