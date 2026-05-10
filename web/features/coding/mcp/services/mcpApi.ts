import { invoke } from '@tauri-apps/api/core';
import type {
  McpServer,
  CreateMcpServerInput,
  UpdateMcpServerInput,
  McpSyncResult,
  McpImportResult,
  McpTool,
  McpScanResult,
} from '../types';

// Server CRUD
export const listMcpServers = async (): Promise<McpServer[]> => {
  return invoke<McpServer[]>('mcp_list_servers');
};

export const createMcpServer = async (input: CreateMcpServerInput): Promise<McpServer> => {
  return invoke<McpServer>('mcp_create_server', { input });
};

export const updateMcpServer = async (serverId: string, input: UpdateMcpServerInput): Promise<McpServer> => {
  return invoke<McpServer>('mcp_update_server', { serverId, input });
};

export const deleteMcpServer = async (serverId: string): Promise<void> => {
  return invoke('mcp_delete_server', { serverId });
};

export const toggleMcpTool = async (serverId: string, toolKey: string): Promise<boolean> => {
  return invoke<boolean>('mcp_toggle_tool', { serverId, toolKey });
};

export const reorderMcpServers = async (ids: string[]): Promise<void> => {
  return invoke('mcp_reorder_servers', { ids });
};

export const updateMcpMetadata = async (
  serverId: string,
  userGroup: string | null,
  userNote: string | null,
): Promise<void> => {
  return invoke('mcp_update_metadata', { serverId, userGroup, userNote });
};

// Sync operations
export const syncMcpToTool = async (toolKey: string): Promise<McpSyncResult[]> => {
  return invoke<McpSyncResult[]>('mcp_sync_to_tool', { toolKey });
};

export const syncMcpAll = async (): Promise<McpSyncResult[]> => {
  return invoke<McpSyncResult[]>('mcp_sync_all');
};

export const importMcpFromTool = async (toolKey: string, enabledTools?: string[]): Promise<McpImportResult> => {
  return invoke<McpImportResult>('mcp_import_from_tool', { toolKey, enabledTools });
};

// Tools API
export const getMcpTools = async (): Promise<McpTool[]> => {
  return invoke<McpTool[]>('mcp_get_tools');
};

// Scan for existing MCP servers in tool configs
export const scanMcpServers = async (): Promise<McpScanResult> => {
  return invoke<McpScanResult>('mcp_scan_servers');
};

// Preferences
export const getMcpShowInTray = async (): Promise<boolean> => {
  return invoke<boolean>('mcp_get_show_in_tray');
};

export const setMcpShowInTray = async (enabled: boolean): Promise<void> => {
  return invoke('mcp_set_show_in_tray', { enabled });
};

export const getMcpPreferredTools = async (): Promise<string[]> => {
  return invoke<string[]>('mcp_get_preferred_tools');
};

export const setMcpPreferredTools = async (tools: string[]): Promise<void> => {
  return invoke('mcp_set_preferred_tools', { tools });
};

export const getMcpSyncDisabledToOpencode = async (): Promise<boolean> => {
  return invoke<boolean>('mcp_get_sync_disabled_to_opencode');
};

export const setMcpSyncDisabledToOpencode = async (enabled: boolean): Promise<void> => {
  return invoke('mcp_set_sync_disabled_to_opencode', { enabled });
};

// Custom Tool Management
export interface AddMcpCustomToolInput {
  key: string;
  displayName: string;
  relativeDetectDir?: string;
  mcpConfigPath: string;
  mcpConfigFormat: 'json' | 'toml';
  mcpField: string;
}

export const addMcpCustomTool = async (input: AddMcpCustomToolInput): Promise<void> => {
  return invoke('mcp_add_custom_tool', { ...input });
};

export const removeMcpCustomTool = async (key: string): Promise<void> => {
  return invoke('mcp_remove_custom_tool', { key });
};

// Favorite MCP API
export interface FavoriteMcp {
  id: string;
  name: string;
  server_type: 'stdio' | 'http' | 'sse';
  server_config: Record<string, unknown>;
  description?: string;
  tags: string[];
  is_preset: boolean;
  created_at: number;
  updated_at: number;
}

export interface FavoriteMcpInput {
  name: string;
  server_type: 'stdio' | 'http' | 'sse';
  server_config: Record<string, unknown>;
  description?: string;
  tags?: string[];
}

export const listMcpFavorites = async (): Promise<FavoriteMcp[]> => {
  return invoke<FavoriteMcp[]>('mcp_list_favorites');
};

export const upsertMcpFavorite = async (input: FavoriteMcpInput): Promise<FavoriteMcp> => {
  return invoke<FavoriteMcp>('mcp_upsert_favorite', { input });
};

export const deleteMcpFavorite = async (favoriteId: string): Promise<void> => {
  return invoke('mcp_delete_favorite', { favoriteId });
};

export const initMcpDefaultFavorites = async (): Promise<number> => {
  return invoke<number>('mcp_init_default_favorites');
};
