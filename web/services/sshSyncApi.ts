/**
 * SSH Sync API Service
 *
 * Handles all SSH sync configuration related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  SSHConnection,
  SSHConnectionResult,
  SSHFileMapping,
  SSHStatusResult,
  SSHSyncConfig,
  SyncResult,
} from '@/types/sshsync';

/**
 * Get SSH sync configuration (config + connections + file mappings)
 */
export const sshGetConfig = async (): Promise<SSHSyncConfig> => {
  return await invoke<SSHSyncConfig>('ssh_get_config');
};

/**
 * Save SSH sync configuration
 */
export const sshSaveConfig = async (config: SSHSyncConfig): Promise<void> => {
  await invoke('ssh_save_config', { config });
};

/**
 * Patch the Claude root permission fix preference without saving the whole
 * SSH sync config.
 */
export const sshSetClaudeRootPermissionFix = async (enabled: boolean): Promise<void> => {
  await invoke('ssh_set_claude_root_permission_fix', { enabled });
};

/**
 * List all SSH connection presets
 */
export const sshListConnections = async (): Promise<SSHConnection[]> => {
  return await invoke<SSHConnection[]>('ssh_list_connections');
};

/**
 * Create a new SSH connection preset
 */
export const sshCreateConnection = async (connection: SSHConnection): Promise<void> => {
  await invoke('ssh_create_connection', { connection });
};

/**
 * Update an existing SSH connection preset
 */
export const sshUpdateConnection = async (connection: SSHConnection): Promise<void> => {
  await invoke('ssh_update_connection', { connection });
};

/**
 * Delete an SSH connection preset
 */
export const sshDeleteConnection = async (id: string): Promise<void> => {
  await invoke('ssh_delete_connection', { id });
};

/**
 * Set active SSH connection
 */
export const sshSetActiveConnection = async (connectionId: string): Promise<void> => {
  await invoke('ssh_set_active_connection', { connectionId });
};

/**
 * Test an SSH connection
 */
export const sshTestConnection = async (connection: SSHConnection): Promise<SSHConnectionResult> => {
  return await invoke<SSHConnectionResult>('ssh_test_connection', { connection });
};

/**
 * Add a new SSH file mapping
 */
export const sshAddFileMapping = async (mapping: SSHFileMapping): Promise<void> => {
  await invoke('ssh_add_file_mapping', { mapping });
};

/**
 * Update an existing SSH file mapping
 */
export const sshUpdateFileMapping = async (mapping: SSHFileMapping): Promise<void> => {
  await invoke('ssh_update_file_mapping', { mapping });
};

/**
 * Delete an SSH file mapping
 */
export const sshDeleteFileMapping = async (id: string): Promise<void> => {
  await invoke('ssh_delete_file_mapping', { id });
};

/**
 * Reset all SSH file mappings
 */
export const sshResetFileMappings = async (): Promise<void> => {
  await invoke('ssh_reset_file_mappings');
};

/**
 * Sync files to SSH remote
 */
export const sshSync = async (module?: string, skipModules?: string[]): Promise<SyncResult> => {
  return await invoke<SyncResult>('ssh_sync', { module, skipModules });
};

/**
 * Get SSH sync status
 */
export const sshGetStatus = async (): Promise<SSHStatusResult> => {
  return await invoke<SSHStatusResult>('ssh_get_status');
};

/**
 * Test if a local path exists
 */
export const sshTestLocalPath = async (localPath: string): Promise<boolean> => {
  return await invoke<boolean>('ssh_test_local_path', { localPath });
};

/**
 * Get default file mappings
 */
export const sshGetDefaultMappings = async (): Promise<SSHFileMapping[]> => {
  return await invoke<SSHFileMapping[]>('ssh_get_default_mappings');
};
