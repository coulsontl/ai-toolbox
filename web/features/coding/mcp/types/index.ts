// MCP Server types

export interface McpServer {
  id: string;
  name: string;
  server_type: 'stdio' | 'http' | 'sse';
  server_config: StdioConfig | HttpConfig;
  enabled_tools: string[];
  sync_details: McpSyncDetail[];
  description: string | null;
  user_group: string | null;
  user_note: string | null;
  tags: string[];
  timeout: number | null;
  sort_index: number;
  /** AI Toolbox management state; false = UX-disabled and unsynced from all tools. */
  management_enabled: boolean;
  /** Tool bindings recorded before disable; used to default the re-enable restore check. */
  disabled_previous_tools: string[];
  created_at: number;
  updated_at: number;
}

export interface StdioConfig {
  command: string;
  args: string[];
  env?: Record<string, string>;
  /** Codex / Grok: startup handshake timeout in seconds */
  startup_timeout_sec?: number;
  /** Codex / Grok: per-tool call timeout in seconds */
  tool_timeout_sec?: number;
  /** OmO: connect + initialize handshake timeout in milliseconds */
  connectTimeoutMs?: number;
  /** OmO: per-request / tool-call timeout in milliseconds */
  requestTimeoutMs?: number;
  /** Tool-specific keys preserved from an imported config (OmO's `lifecycle`, ...). */
  [key: string]: unknown;
}

export interface HttpConfig {
  url: string;
  headers?: Record<string, string>;
  /** Codex / Grok: startup handshake timeout in seconds */
  startup_timeout_sec?: number;
  /** Codex / Grok: per-tool call timeout in seconds */
  tool_timeout_sec?: number;
  /** OmO: connect + initialize handshake timeout in milliseconds */
  connectTimeoutMs?: number;
  /** OmO: per-request / tool-call timeout in milliseconds */
  requestTimeoutMs?: number;
  /** Tool-specific keys preserved from an imported config (OmO's `lifecycle`, ...). */
  [key: string]: unknown;
}

export interface McpSyncDetail {
  tool: string;
  status: 'ok' | 'error' | 'pending';
  synced_at: number | null;
  error_message: string | null;
}

export interface CreateMcpServerInput {
  name: string;
  server_type: 'stdio' | 'http' | 'sse';
  server_config: StdioConfig | HttpConfig;
  enabled_tools?: string[];
  description?: string;
  tags?: string[];
  timeout?: number;
}

export interface UpdateMcpServerInput {
  name?: string;
  server_type?: 'stdio' | 'http' | 'sse';
  server_config?: StdioConfig | HttpConfig;
  enabled_tools?: string[];
  description?: string;
  tags?: string[];
  timeout?: number;
}

export interface McpSyncResult {
  tool: string;
  success: boolean;
  error_message: string | null;
}

export interface McpImportResult {
  servers_imported: number;
  servers_skipped: number;
  servers_duplicated: string[];  // Names of servers created with suffix due to config differences
  errors: string[];
}

/** Preview of a group-inventory JSON import (server grouping only). */
export interface McpGroupInventoryPreview {
  valid: boolean;
  group_count: number;
  matched_server_count: number;
  changed_count: number;
  errors: string[];
}

/** Managed group entity for the group management modal (mirrors SkillGroupRecord). */
export interface McpGroupRecord {
  id: string;
  name: string;
  note: string | null;
  sort_index: number;
  created_at: number;
  updated_at: number;
}

export interface McpDiscoveredServer {
  name: string;
  tool_key: string;
  tool_name: string;
  server_type: string;
  server_config: StdioConfig | HttpConfig;
  /** Tool keys marked enabled at the source; only set for the CC Switch source. */
  source_enabled_tools?: string[];
}

export interface McpScanResult {
  total_tools_scanned: number;
  total_servers_found: number;
  servers: McpDiscoveredServer[];
}

export interface McpTool {
  key: string;
  display_name: string;
  is_custom: boolean;
  installed: boolean;
  relative_skills_dir: string | null;
  skills_path: string | null;
  supports_skills: boolean;
  mcp_config_path: string | null;
  mcp_config_format: string | null;
  mcp_field: string | null;
  supports_mcp: boolean;
  /** Custom tool brand image URL (http/https), mirrors RuntimeToolDto.icon_url. */
  icon_url?: string | null;
}

export interface McpGroup {
  key: string;
  label: string;
  servers: McpServer[];
}

export interface McpPackageVersionResolveRequest {
  manager: 'npx' | 'uv';
  package_name: string;
}

export interface McpPackageVersionResolveResult {
  manager: 'npx' | 'uv';
  package_name: string;
  version: string | null;
  error_message: string | null;
}
