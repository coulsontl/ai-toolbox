import { invoke } from '@tauri-apps/api/core';

export type GatewayCliKey = 'claude' | 'codex' | 'gemini' | 'opencode';

export interface AppProxyConfig {
  streaming_first_byte_timeout_secs?: number | null;
  streaming_idle_timeout_secs?: number | null;
  non_streaming_timeout_secs?: number | null;
  per_provider_retry_count?: number | null;
  max_retry_count?: number | null;
}

export interface ProxyGatewaySettings {
  enabled_on_startup: boolean;
  listen_host: string;
  listen_port: number;
  port_auto_select: boolean;
  enabled_cli_keys: GatewayCliKey[];
  request_log_enabled: boolean;
  request_log_level: string;
  metrics_enabled: boolean;
  store_request_body: boolean;
  store_headers: boolean;
  store_response_body: boolean;
  thinking_rectifier_enabled: boolean;
  thinking_budget_rectifier_enabled: boolean;
  cache_injection_enabled: boolean;
  streaming_first_byte_timeout_secs: number;
  streaming_idle_timeout_secs: number;
  non_streaming_timeout_secs: number;
  log_retention_days: number;
  log_max_dir_size_mb: number;
  log_max_body_size_kb: number;
  per_provider_retry_count: number;
  max_retry_count: number;
  app_configs: Partial<Record<GatewayCliKey, AppProxyConfig>>;
  model_failure_score_threshold: number;
  model_failure_window_seconds: number;
  model_base_cooldown_seconds: number;
  model_max_cooldown_seconds: number;
  half_open_success_required: number;
}

export interface ProxyGatewayStatus {
  running: boolean;
  base_url: string | null;
  listen_host: string;
  listen_port: number | null;
  active_connections: number;
  last_error: string | null;
}

export interface GatewayFailoverEvent {
  cli_key: GatewayCliKey;
  from_provider_id: string;
  from_provider_name: string | null;
  to_provider_id: string;
  to_provider_name: string | null;
}

export interface ProxyGatewayPortCheckInput {
  listen_host: string;
  listen_port: number;
}

export interface ProxyGatewayPortCheckResult {
  available: boolean;
  listen_host: string;
  listen_port: number;
}

export interface ProxyGatewayHealthCheckResult {
  ok: boolean;
  status_code: number | null;
  error: string | null;
}

export type GatewayCliTakeoverState =
  | 'direct'
  | 'takeover_applied'
  | 'gateway_stopped'
  | 'outdated_origin'
  | 'drifted'
  | 'no_proxy_provider'
  | 'restore_unavailable'
  | 'unsupported'
  | 'error';

export type GatewayCliStatusDot = 'gray' | 'green' | 'orange' | 'red';

export interface GatewayManagedTarget {
  kind: string;
  path: string;
  existed: boolean;
}

export interface GatewayCliTakeoverStatus {
  cli_key: GatewayCliKey;
  state: GatewayCliTakeoverState;
  dot: GatewayCliStatusDot;
  can_takeover: boolean;
  can_restore_direct: boolean;
  gateway_origin: string | null;
  runtime_root: string | null;
  managed_targets: GatewayManagedTarget[];
  message: string | null;
}

export interface ProxyGatewayStopPreflight {
  allowed: boolean;
  blocking_cli_takeovers: GatewayCliTakeoverStatus[];
  message: string | null;
}

export interface ProxyGatewayRequestLogListInput {
  limit?: number | null;
}

export interface GatewayRequestLogFilters {
  cli_key?: GatewayCliKey | null;
  provider_name?: string | null;
  model?: string | null;
  status_code?: number | null;
  start_date?: number | null;
  end_date?: number | null;
}

export interface GatewayPaginatedRequestLogs {
  data: GatewayRequestLogItem[];
  total: number;
  page: number;
  page_size: number;
}

export interface GatewayRequestLogItem {
  trace_id: string;
  cli_key: GatewayCliKey;
  provider_id: string;
  provider_name: string | null;
  requested_model: string | null;
  upstream_model_id: string;
  status_code: number;
  success: boolean;
  error_message: string | null;
  created_at: string;
  duration_ms: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  total_tokens: number;
  total_cost_usd: string;
  is_streaming: boolean;
  first_token_ms: number | null;
  detail_file?: string | null;
  detail_offset?: number | null;
}

export interface GatewayUsageSummary {
  total_requests: number;
  total_cost_usd: string;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_creation_tokens: number;
  success_rate: number;
  total_tokens: number;
}

export interface GatewayUsageSummaryByCli {
  cli_key: GatewayCliKey;
  summary: GatewayUsageSummary;
}

export interface GatewayUsageTrendPoint {
  date: string;
  request_count: number;
  total_cost_usd: string;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
}

export interface GatewayProviderStats {
  cli_key: GatewayCliKey;
  provider_id: string;
  provider_name: string | null;
  request_count: number;
  total_tokens: number;
  total_cost_usd: string;
  success_rate: number;
  avg_latency_ms: number;
}

export interface GatewayModelStats {
  cli_key: GatewayCliKey;
  model: string;
  request_count: number;
  total_tokens: number;
  total_cost_usd: string;
  avg_latency_ms: number;
}

export interface GatewayRequestLogSummary {
  trace_id: string;
  started_at: string;
  ended_at: string;
  cli_key: GatewayCliKey | null;
  route_name: string;
  method: string;
  path: string;
  provider_id: string | null;
  provider_name: string | null;
  requested_model: string | null;
  upstream_model_id: string | null;
  upstream_url: string | null;
  status_code: number | null;
  success: boolean;
  error_category: string | null;
  error_message: string | null;
  duration_ms: number;
  attempt_count: number;
  total_attempt_count: number;
  failover: boolean;
  input_tokens: number | null;
  output_tokens: number | null;
  cache_read_tokens: number | null;
  cache_creation_tokens: number | null;
  total_tokens: number | null;
  request_body_bytes: number;
  response_body_bytes: number;
  is_streaming: boolean;
  first_token_ms: number | null;
}

export interface GatewayProviderAttempt {
  provider_id: string | null;
  provider_name: string | null;
  upstream_model_id: string | null;
  status_code: number | null;
  success: boolean;
  error_category: string | null;
  error_message: string | null;
  attempt_count: number;
  total_attempt_count: number;
}

export interface GatewayRequestLogDetail extends GatewayRequestLogSummary {
  request_headers: Record<string, string> | null;
  request_body: string | null;
  upstream_request_body: string | null;
  response_headers: Record<string, string> | null;
  response_body: string | null;
  provider_attempts: GatewayProviderAttempt[];
}

export type ModelHealthStateKind = 'healthy' | 'degraded' | 'cooling_down' | 'probing';
export type GatewayModelHealthScope = 'model' | 'provider';

export interface GatewayModelHealthItem {
  scope: GatewayModelHealthScope;
  cli_key: GatewayCliKey;
  provider_id: string;
  provider_name: string | null;
  upstream_model_id: string | null;
  state: ModelHealthStateKind;
  failure_score: number;
  consecutive_open_count: number;
  half_open_success_count: number;
  next_retry_at: string | null;
  last_failure_at: string | null;
  last_error_category: string | null;
}

export type GatewaySessionImportCli = 'all' | 'claude' | 'codex' | 'gemini';

export interface GatewaySessionUsageImportInput {
  cli_key: GatewaySessionImportCli;
}

export interface GatewaySessionUsageImportResult {
  scanned_files: number;
  parsed_records: number;
  inserted_records: number;
  skipped_records: number;
}

export const getProxyGatewaySettings = async (): Promise<ProxyGatewaySettings> => {
  return invoke<ProxyGatewaySettings>('proxy_gateway_get_settings');
};

export const updateProxyGatewaySettings = async (
  settings: ProxyGatewaySettings
): Promise<ProxyGatewaySettings> => {
  return invoke<ProxyGatewaySettings>('proxy_gateway_update_settings', { settings });
};

export const startProxyGateway = async (
  settings?: ProxyGatewaySettings
): Promise<ProxyGatewayStatus> => {
  return invoke<ProxyGatewayStatus>('proxy_gateway_start', { settings: settings ?? null });
};

export const stopProxyGateway = async (): Promise<ProxyGatewayStatus> => {
  return invoke<ProxyGatewayStatus>('proxy_gateway_stop');
};

export const getProxyGatewayStatus = async (): Promise<ProxyGatewayStatus> => {
  return invoke<ProxyGatewayStatus>('proxy_gateway_status');
};

export const checkProxyGatewayHealth = async (): Promise<ProxyGatewayHealthCheckResult> => {
  return invoke<ProxyGatewayHealthCheckResult>('proxy_gateway_health_check');
};

export const checkProxyGatewayPortAvailable = async (
  input: ProxyGatewayPortCheckInput
): Promise<ProxyGatewayPortCheckResult> => {
  return invoke<ProxyGatewayPortCheckResult>('proxy_gateway_check_port_available', { input });
};

export const getProxyGatewayCliStatuses = async (): Promise<GatewayCliTakeoverStatus[]> => {
  return invoke<GatewayCliTakeoverStatus[]>('proxy_gateway_cli_statuses');
};

export const getProxyGatewayCliStatus = async (
  cliKey: GatewayCliKey
): Promise<GatewayCliTakeoverStatus> => {
  return invoke<GatewayCliTakeoverStatus>('proxy_gateway_cli_status', { cliKey });
};

export const takeoverProxyGatewayCli = async (
  cliKey: GatewayCliKey
): Promise<GatewayCliTakeoverStatus> => {
  return invoke<GatewayCliTakeoverStatus>('proxy_gateway_takeover_cli', { cliKey });
};

export const restoreProxyGatewayCliDirect = async (
  cliKey: GatewayCliKey
): Promise<GatewayCliTakeoverStatus> => {
  return invoke<GatewayCliTakeoverStatus>('proxy_gateway_restore_cli_direct', { cliKey });
};

export const preflightStopProxyGateway = async (): Promise<ProxyGatewayStopPreflight> => {
  return invoke<ProxyGatewayStopPreflight>('proxy_gateway_stop_preflight');
};

export const listProxyGatewayRequestLogs = async (
  filters: GatewayRequestLogFilters = {},
  page = 0,
  pageSize = 20,
  input: ProxyGatewayRequestLogListInput | null = null
): Promise<GatewayPaginatedRequestLogs> => {
  return invoke<GatewayPaginatedRequestLogs>('proxy_gateway_request_logs', {
    filters,
    page,
    pageSize,
    input,
  });
};

export const getProxyGatewayRequestLogDetail = async (
  traceId: string
): Promise<GatewayRequestLogDetail | null> => {
  return invoke<GatewayRequestLogDetail | null>('proxy_gateway_request_log_detail', { traceId });
};

export const getProxyGatewayUsageSummary = async (
  startDate?: number,
  endDate?: number,
  cliKey?: GatewayCliKey
): Promise<GatewayUsageSummary> => {
  return invoke<GatewayUsageSummary>('proxy_gateway_usage_summary', {
    startDate: startDate ?? null,
    endDate: endDate ?? null,
    cliKey: cliKey ?? null,
  });
};

export const getProxyGatewayUsageSummaryByCli = async (
  startDate?: number,
  endDate?: number
): Promise<GatewayUsageSummaryByCli[]> => {
  return invoke<GatewayUsageSummaryByCli[]>('proxy_gateway_usage_summary_by_cli', {
    startDate: startDate ?? null,
    endDate: endDate ?? null,
  });
};

export const getProxyGatewayUsageTrends = async (
  startDate?: number,
  endDate?: number,
  cliKey?: GatewayCliKey
): Promise<GatewayUsageTrendPoint[]> => {
  return invoke<GatewayUsageTrendPoint[]>('proxy_gateway_usage_trends', {
    startDate: startDate ?? null,
    endDate: endDate ?? null,
    cliKey: cliKey ?? null,
  });
};

export const getProxyGatewayProviderStats = async (
  startDate?: number,
  endDate?: number,
  cliKey?: GatewayCliKey
): Promise<GatewayProviderStats[]> => {
  return invoke<GatewayProviderStats[]>('proxy_gateway_provider_stats', {
    startDate: startDate ?? null,
    endDate: endDate ?? null,
    cliKey: cliKey ?? null,
  });
};

export const getProxyGatewayModelStats = async (
  startDate?: number,
  endDate?: number,
  cliKey?: GatewayCliKey
): Promise<GatewayModelStats[]> => {
  return invoke<GatewayModelStats[]>('proxy_gateway_model_stats', {
    startDate: startDate ?? null,
    endDate: endDate ?? null,
    cliKey: cliKey ?? null,
  });
};

export const importProxyGatewaySessionUsage = async (
  input: GatewaySessionUsageImportInput
): Promise<GatewaySessionUsageImportResult> => {
  return invoke<GatewaySessionUsageImportResult>('proxy_gateway_import_session_usage', { input });
};

export const listProxyGatewayModelHealthEntries = async (): Promise<GatewayModelHealthItem[]> => {
  return invoke<GatewayModelHealthItem[]>('proxy_gateway_model_health_entries');
};
