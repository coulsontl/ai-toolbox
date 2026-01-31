// MCP Server types

export interface McpServer {
  id: string;
  name: string;
  server_type: 'stdio' | 'http' | 'sse';
  server_config: StdioConfig | HttpConfig;
  enabled_tools: string[];
  sync_details: McpSyncDetail[];
  description: string | null;
  tags: string[];
  sort_index: number;
  created_at: number;
  updated_at: number;
}

export interface StdioConfig {
  command: string;
  args: string[];
  env?: Record<string, string>;
}

export interface HttpConfig {
  url: string;
  headers?: Record<string, string>;
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
}

export interface UpdateMcpServerInput {
  name?: string;
  server_type?: 'stdio' | 'http' | 'sse';
  server_config?: StdioConfig | HttpConfig;
  enabled_tools?: string[];
  description?: string;
  tags?: string[];
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

export interface McpDiscoveredServer {
  name: string;
  tool_key: string;
  tool_name: string;
  server_type: string;
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
}
