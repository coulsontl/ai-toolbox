import {
  Bot,
  BookOpen,
  CircleHelp,
  Edit3,
  FileCode2,
  FilePlus2,
  FileSearch,
  FileText,
  Globe,
  ListChecks,
  ListTodo,
  Search,
  Server,
  Terminal,
  type LucideIcon,
} from 'lucide-react';

export type SessionToolVariant =
  | 'terminal'
  | 'code'
  | 'file'
  | 'search'
  | 'task'
  | 'web'
  | 'mcp'
  | 'document'
  | 'system'
  | 'thinking'
  | 'success'
  | 'warning'
  | 'error'
  | 'neutral';

export function getNormalizedToolName(toolName: string | undefined): string {
  const normalized = (toolName ?? '')
    .trim()
    .replace(/^functions\./i, '')
    .toLowerCase()
    .replace(/[-\s.]+/g, '_');

  if (!normalized) {
    return 'unknown';
  }

  if (
    normalized.startsWith('mcp__')
    || normalized.startsWith('mcp_')
    || normalized.includes('__mcp__')
    || normalized.includes('server_tool')
  ) {
    return 'mcp';
  }

  if (
    ['bash', 'shell', 'terminal', 'command', 'execute', 'execute_command', 'run_command'].includes(normalized)
    || normalized.includes('execute_command')
    || normalized.includes('terminal')
  ) {
    return 'bash';
  }

  if (['read', 'read_file', 'file_read', 'view_file', 'open_file'].includes(normalized)) {
    return 'read';
  }
  if (['write', 'write_file', 'create_file', 'file_write'].includes(normalized)) {
    return 'write';
  }
  if (['multiedit', 'multi_edit', 'batch_edit', 'multi_file_edit'].includes(normalized)) {
    return 'multi_edit';
  }
  if (['edit', 'edit_file', 'file_edit', 'replace_in_file'].includes(normalized)) {
    return 'edit';
  }
  if (['apply_patch', 'applypatch', 'patch', 'file_patch'].includes(normalized)) {
    return 'apply_patch';
  }
  if (['notebookedit', 'notebook_edit', 'edit_notebook'].includes(normalized)) {
    return 'notebook_edit';
  }
  if (['grep', 'rg', 'search_text', 'text_search'].includes(normalized)) {
    return 'grep';
  }
  if (['glob', 'file_glob', 'find_files', 'folder_search'].includes(normalized)) {
    return 'glob';
  }
  if (['webfetch', 'web_fetch', 'fetch_url', 'browser_fetch'].includes(normalized)) {
    return 'web_fetch';
  }
  if (['websearch', 'web_search', 'search_web', 'browser_search'].includes(normalized)) {
    return 'web_search';
  }
  if (['todowrite', 'todo_write', 'todo', 'write_todos'].includes(normalized)) {
    return 'todo_write';
  }
  if (['update_plan', 'updateplan', 'plan'].includes(normalized)) {
    return 'update_plan';
  }
  if (['task', 'subagent_task', 'task_create', 'task_update', 'task_output'].includes(normalized)) {
    return 'task';
  }
  if (['agent', 'subagent', 'delegate', 'create_agent'].includes(normalized)) {
    return 'agent';
  }

  return 'unknown';
}

export function inferNormalizedToolNameFromInput(input: unknown): string {
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    return 'unknown';
  }

  const record = input as Record<string, unknown>;
  if (typeof record.command === 'string' || typeof record.cmd === 'string') {
    return 'bash';
  }
  if (typeof record.pattern === 'string') {
    return 'grep';
  }
  if (typeof record.file_path === 'string' || typeof record.filePath === 'string' || typeof record.path === 'string') {
    return 'read';
  }
  if (typeof record.query === 'string') {
    return 'web_search';
  }
  return 'unknown';
}

export function getToolVariant(toolName: string | undefined, normalizedToolName?: string): SessionToolVariant {
  const normalized = normalizedToolName || getNormalizedToolName(toolName);
  switch (normalized) {
    case 'bash':
      return 'terminal';
    case 'read':
    case 'write':
    case 'edit':
    case 'multi_edit':
    case 'apply_patch':
    case 'notebook_edit':
      return 'code';
    case 'grep':
      return 'search';
    case 'glob':
      return 'file';
    case 'web_fetch':
    case 'web_search':
      return 'web';
    case 'todo_write':
    case 'update_plan':
    case 'task':
    case 'agent':
      return 'task';
    case 'mcp':
      return 'mcp';
    default:
      return toolName?.toLowerCase().includes('mcp') ? 'mcp' : 'neutral';
  }
}

export function getToolIcon(normalizedToolName: string | undefined, toolName?: string): LucideIcon {
  const normalized = normalizedToolName || getNormalizedToolName(toolName);
  switch (normalized) {
    case 'bash':
      return Terminal;
    case 'read':
      return FileText;
    case 'write':
      return FilePlus2;
    case 'edit':
    case 'multi_edit':
      return Edit3;
    case 'apply_patch':
      return FileCode2;
    case 'notebook_edit':
      return BookOpen;
    case 'grep':
      return FileSearch;
    case 'glob':
      return Search;
    case 'web_fetch':
      return Globe;
    case 'web_search':
      return Search;
    case 'todo_write':
      return ListTodo;
    case 'update_plan':
      return ListChecks;
    case 'task':
    case 'agent':
      return Bot;
    case 'mcp':
      return Server;
    default:
      return CircleHelp;
  }
}

export function getToolDisplayName(normalizedToolName: string | undefined, toolName?: string): string {
  const explicitToolName = toolName?.trim();
  if (explicitToolName && explicitToolName.toLowerCase() !== 'unknown') {
    return explicitToolName;
  }

  switch (normalizedToolName) {
    case 'bash':
      return 'Bash';
    case 'read':
      return 'Read';
    case 'write':
      return 'Write';
    case 'edit':
      return 'Edit';
    case 'multi_edit':
      return 'MultiEdit';
    case 'apply_patch':
      return 'ApplyPatch';
    case 'notebook_edit':
      return 'NotebookEdit';
    case 'grep':
      return 'Grep';
    case 'glob':
      return 'Glob';
    case 'web_fetch':
      return 'WebFetch';
    case 'web_search':
      return 'WebSearch';
    case 'todo_write':
      return 'TodoWrite';
    case 'update_plan':
      return 'UpdatePlan';
    case 'task':
      return 'Task';
    case 'agent':
      return 'Agent';
    case 'mcp':
      return 'MCP Tool';
    default:
      return 'Tool';
  }
}
