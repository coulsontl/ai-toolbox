/**
 * SSH Sync Types
 */

import type { WslDirectModuleStatus } from './wslsync';

/**
 * SSH connection preset
 */
export type SSHAuthMethod = 'key' | 'password' | 'none';

export interface SSHConnection {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: SSHAuthMethod;
  password: string;
  privateKeyPath: string;
  privateKeyContent: string;
  passphrase: string;
  sortOrder: number;
}

/**
 * SSH file mapping (global, shared across all connections)
 */
export interface SSHFileMapping {
  id: string;
  name: string;
  module: string; // "opencode" | "claude" | "codex" | "openclaw" | "geminicli"
  localPath: string;
  remotePath: string;
  enabled: boolean;
  isPattern: boolean;
  isDirectory: boolean;
}

/**
 * SSH sync configuration
 */
export interface SSHSyncConfig {
  enabled: boolean;
  activeConnectionId: string;
  fileMappings: SSHFileMapping[];
  connections: SSHConnection[];
  lastSyncTime?: string;
  lastSyncStatus: string; // "success" | "error" | "never"
  lastSyncError?: string;
  moduleStatuses: WslDirectModuleStatus[];
}

/**
 * SSH connection test result
 */
export interface SSHConnectionResult {
  connected: boolean;
  error?: string;
  serverInfo?: string;
}

/**
 * SSH status result
 */
export interface SSHStatusResult {
  sshAvailable: boolean;
  activeConnectionName?: string;
  lastSyncTime?: string;
  lastSyncStatus: string;
  lastSyncError?: string;
}

/**
 * Result of a sync operation (reuse from WSL)
 */
export interface SyncResult {
  success: boolean;
  syncedFiles: string[];
  skippedFiles: string[];
  errors: string[];
}

/**
 * Sync progress event payload (reuse from WSL)
 */
export interface SyncProgress {
  phase: string;
  currentItem: string;
  current: number;
  total: number;
  message: string;
}
