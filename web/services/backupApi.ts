/**
 * Backup API Service
 *
 * Handles all backup-related communication with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

/**
 * Backup database to a local zip file
 * @param backupPath - The directory to save the backup file
 * @returns The full path of the created backup file
 */
export const backupDatabase = async (backupPath: string): Promise<string> => {
  if (!backupPath) {
    throw new Error('Backup path is not configured');
  }

  const result = await invoke<string>('backup_database', { backupPath });
  return result;
};

/**
 * Restore database from a local zip file
 * @param zipFilePath - The path to the backup zip file
 */
export const restoreDatabase = async (zipFilePath: string): Promise<void> => {
  await invoke('restore_database', { zipFilePath });
};

/**
 * Get the database directory path
 */
export const getDatabasePath = async (): Promise<string> => {
  const result = await invoke<string>('get_database_path');
  return result;
};

/**
 * Open file dialog to select a backup file for restore
 * @returns The selected file path, or null if cancelled
 */
export const selectBackupFile = async (): Promise<string | null> => {
  const selected = await open({
    multiple: false,
    filters: [
      {
        name: 'Backup Files',
        extensions: ['zip'],
      },
    ],
    title: 'Select Backup File',
  });

  return selected as string | null;
};

/**
 * Backup database to WebDAV server
 */
export const backupToWebDAV = async (
  url: string,
  username: string,
  password: string,
  remotePath: string
): Promise<string> => {
  const result = await invoke<string>('backup_to_webdav', {
    url,
    username,
    password,
    remotePath,
  });
  return result;
};

/**
 * List backup files from WebDAV server
 */
export const listWebDAVBackups = async (
  url: string,
  username: string,
  password: string,
  remotePath: string
): Promise<string[]> => {
  const result = await invoke<string[]>('list_webdav_backups', {
    url,
    username,
    password,
    remotePath,
  });
  return result;
};

/**
 * Restore database from WebDAV server
 */
export const restoreFromWebDAV = async (
  url: string,
  username: string,
  password: string,
  remotePath: string,
  filename: string
): Promise<void> => {
  await invoke('restore_from_webdav', {
    url,
    username,
    password,
    remotePath,
    filename,
  });
};
