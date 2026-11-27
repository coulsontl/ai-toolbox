/**
 * Agent Control API service.
 *
 * These commands are reachable only from the desktop webview. The HTTP
 * bearer token is deliberately absent from every payload here: the frontend
 * only needs status, settings and approval decisions.
 */

import { invoke } from '@tauri-apps/api/core';

export type ControlApiApprovalMode =
  | 'always_ask'
  | 'ask_high_risk'
  | 'auto_approve'
  | 'read_only';

/** Stored settings, serialized with snake_case field names by the backend. */
export interface ControlApiSettings {
  enabled: boolean;
  approval_mode: ControlApiApprovalMode;
  auto_approve_local_plugin_reload: boolean;
  approval_timeout_secs: number;
}

export type ControlApiRisk = 'l0' | 'l1' | 'l2';

export interface ControlApiApprovalInfo {
  requestId: string;
  action: string;
  summary: string;
  risk: ControlApiRisk;
  target: string | null;
  timeoutSecs: number;
  createdAt: string;
}

export interface ControlApiStatus {
  app: string;
  version: string;
  pid: number;
  enabled: boolean;
  running: boolean;
  url: string | null;
  discoveryFile: string;
  dataDir: string;
  approvalMode: ControlApiApprovalMode;
  approvalTimeoutSecs: number;
  autoApproveLocalPluginReload: boolean;
  pendingApprovals?: ControlApiApprovalInfo[];
}

export interface ControlApiCliStatus {
  installed: boolean;
  installPath: string | null;
  binarySource: string | null;
  installDir: string | null;
  onPath: boolean;
  version: string;
}

export const CONTROL_API_APPROVAL_REQUEST_EVENT = 'control-api-approval-request';
export const CONTROL_API_APPROVAL_EXPIRED_EVENT = 'control-api-approval-expired';

export function getControlApiSettings(): Promise<ControlApiSettings> {
  return invoke<ControlApiSettings>('control_api_get_settings');
}

export function saveControlApiSettings(input: ControlApiSettings): Promise<ControlApiStatus> {
  return invoke<ControlApiStatus>('control_api_save_settings', { input });
}

export function getControlApiStatus(): Promise<ControlApiStatus> {
  return invoke<ControlApiStatus>('control_api_status');
}

export function resolveControlApiApproval(
  requestId: string,
  approved: boolean,
): Promise<{ resolved: boolean }> {
  return invoke<{ resolved: boolean }>('control_api_resolve_approval', {
    requestId,
    approved,
  });
}

export function listPendingControlApiApprovals(): Promise<ControlApiApprovalInfo[]> {
  return invoke<ControlApiApprovalInfo[]>('control_api_list_pending_approvals');
}

export function getControlApiCliStatus(): Promise<ControlApiCliStatus> {
  return invoke<ControlApiCliStatus>('control_api_cli_status');
}

export function installControlApiCli(): Promise<ControlApiCliStatus> {
  return invoke<ControlApiCliStatus>('control_api_install_cli');
}

export function uninstallControlApiCli(): Promise<ControlApiCliStatus> {
  return invoke<ControlApiCliStatus>('control_api_uninstall_cli');
}
