export type SessionTool =
  | 'codex'
  | 'grok'
  | 'kimi'
  | 'claudecode'
  | 'claudedesktop'
  | 'geminicli'
  | 'antigravity'
  | 'zcode'
  | 'openclaw'
  | 'opencode'
  | 'pi'
  | 'oh_my_pi'
  | 'omo_native'
  | 'hermes'
  | 'dsh'
  | 'workbuddy'
  | 'codebuddy';

export type SessionSourceMode = 'all' | 'local' | 'wsl';
export type SessionTimeRange = 'all' | 'today' | '7d' | '30d' | 'older_30d';
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

/**
 * Which auxiliary Codex cleanup the delete should also perform.
 *
 * Only Codex sessions have this: Codex creates a scratch workspace and a
 * `[projects]` trust entry for every project-less chat, and reclaims neither
 * when the chat is deleted.
 */
export interface CodexCleanupOptions {
  removeWorkspace: boolean;
  removeTrustEntry: boolean;
}

export interface ScratchWorkspaceInfo {
  path: string;
  exists: boolean;
  isEmpty: boolean;
  fileCount: number;
  totalBytes: number;
  /** Contains a git repository: removal is refused by the backend. */
  hasGit: boolean;
  truncated: boolean;
}

export interface CodexCleanupSkip {
  target: string;
  reason: string;
}

export interface CodexCleanupFailure {
  target: string;
  error: string;
}

/**
 * The auxiliary cleanup outcome, reported separately from the deletion.
 *
 * A cleanup failure is never a deletion failure: the session is gone even when
 * its workspace or trust entry could not be removed.
 */
export interface CodexCleanupSummary {
  removedWorkspaces: string[];
  removedTrustKeys: string[];
  removedDateDirs: string[];
  skipped: CodexCleanupSkip[];
  failures: CodexCleanupFailure[];
}

export interface CodexCleanupPreviewItem {
  sourcePath: string;
  projectDir: string;
  workspace?: ScratchWorkspaceInfo | null;
  trustKeys: string[];
  configPath: string;
  runtimeSource: 'local' | 'wsl';
  runtimeDistro?: string | null;
}

export interface CodexCleanupPreview {
  items: CodexCleanupPreviewItem[];
}

export interface DeleteToolSessionResult {
  sourcePath: string;
  cleanup?: CodexCleanupSummary;
}

export interface DeleteToolSessionsResult {
  deletedCount: number;
  failedItems: DeleteSessionFailure[];
  cleanup?: CodexCleanupSummary;
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
