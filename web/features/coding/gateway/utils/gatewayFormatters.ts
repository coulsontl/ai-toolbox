import type { ProxyGatewaySettings, ProxyGatewayStatus } from '@/services';

export const joinClassNames = (...classNames: Array<string | false | null | undefined>) =>
  classNames.filter(Boolean).join(' ');

export const formatGatewayError = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export const deriveRequestLogLevel = (settings: ProxyGatewaySettings | null) => {
  if (!settings?.request_log_enabled) {
    return 'off';
  }
  if (settings.store_request_body && settings.store_headers && settings.store_response_body) {
    return 'full';
  }
  if (settings.store_request_body || settings.store_response_body) {
    return 'body';
  }
  if (settings.store_headers) {
    return 'headers';
  }
  return 'summary';
};

export const buildGatewayOrigin = (status: ProxyGatewayStatus | null) => {
  if (!status) {
    return '-';
  }
  if (status.base_url) {
    return status.base_url;
  }
  return status.listen_port ? `http://${status.listen_host}:${status.listen_port}` : '-';
};

export const formatDuration = (durationMs: number) => {
  if (durationMs < 1000) {
    return `${durationMs}ms`;
  }
  return `${(durationMs / 1000).toFixed(durationMs < 10_000 ? 1 : 0)}s`;
};

export const formatDateTime = (value: string | null | undefined) => {
  if (!value) {
    return '-';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
};

export const formatInteger = (value: number | null | undefined) => {
  if (value == null) {
    return '-';
  }
  return value.toLocaleString();
};

export const formatCompactInteger = (value: number | null | undefined) => {
  if (value == null) {
    return '-';
  }
  return new Intl.NumberFormat(undefined, {
    notation: 'compact',
    maximumFractionDigits: 1,
  }).format(value);
};

export const formatUsd = (value: string | number | null | undefined, digits = 6) => {
  const parsed = typeof value === 'number' ? value : Number.parseFloat(value ?? '0');
  if (!Number.isFinite(parsed)) {
    return '$0';
  }
  return `$${parsed.toFixed(digits)}`;
};

export type GatewayRequestDisplayKind =
  | 'model'
  | 'modelList'
  | 'contextCompact'
  | 'connectionProbe'
  | 'genericRequest'
  | 'unknown';

export interface GatewayRequestDisplayInput {
  method?: string | null;
  path?: string | null;
  requested_model?: string | null;
  upstream_model_id?: string | null;
}

export interface GatewayRequestDisplay {
  kind: GatewayRequestDisplayKind;
  titleKey: string | null;
  requestLine: string;
  modelText: string;
  modelApplicable: boolean;
}

const REQUEST_EXPORT_FILE_NAME_FALLBACK = 'gateway-request';
const placeholderModelValues = new Set(['', 'unknown', 'null', 'none']);

const isPlaceholderModel = (value: string | null | undefined) =>
  placeholderModelValues.has(value?.trim().toLowerCase() ?? '');

export const formatModelRoute = (
  requestedModel: string | null,
  upstreamModelId: string | null,
  fallback: string,
) => {
  const requested = requestedModel?.trim() ?? '';
  const upstream = upstreamModelId?.trim() ?? '';
  const hasRequested = !isPlaceholderModel(requested);
  const hasUpstream = !isPlaceholderModel(upstream);
  const displayModel = hasRequested ? requested : hasUpstream ? upstream : fallback;
  if (hasRequested && hasUpstream && upstream !== requested) {
    return `${requested} -> ${upstream}`;
  }
  return displayModel;
};

const splitRequestPath = (path: string | null | undefined) => {
  const trimmed = path?.trim() ?? '';
  const [pathOnly] = trimmed.split('?');
  return (pathOnly || trimmed).toLowerCase();
};

const compactMethod = (method: string | null | undefined) => method?.trim().toUpperCase() ?? '';

const normalizedPathSegments = (normalizedPath: string) =>
  normalizedPath.split('/').filter(Boolean);

const pathEndsWithSegments = (normalizedPath: string, suffix: string[]) => {
  const segments = normalizedPathSegments(normalizedPath);
  if (segments.length < suffix.length) {
    return false;
  }
  return suffix.every((segment, index) => segments[segments.length - suffix.length + index] === segment);
};

const isModelListPath = (normalizedPath: string) => {
  if (pathEndsWithSegments(normalizedPath, ['models'])) {
    return true;
  }
  return /\/models:listmodels$/.test(normalizedPath);
};

const isConnectionProbePath = (normalizedPath: string) =>
  normalizedPath === '/anthropic' ||
  normalizedPath === '/openai/v1' ||
  normalizedPath === '/grok/v1' ||
  normalizedPath === '/gemini/v1' ||
  normalizedPath === '/gemini/v1beta' ||
  normalizedPath === '/gemini/v1alpha';

const isContextCompactPath = (normalizedPath: string) =>
  pathEndsWithSegments(normalizedPath, ['responses', 'compact']);

export const requestLineText = (
  value: Pick<GatewayRequestDisplayInput, 'method' | 'path'>,
  fallback: string,
) => {
  const method = compactMethod(value.method);
  const path = value.path?.trim() ?? '';
  if (method && path) {
    return `${method} ${path}`;
  }
  if (path) {
    return path;
  }
  if (method) {
    return method;
  }
  return fallback;
};

export const gatewayRequestDisplayKind = (
  value: GatewayRequestDisplayInput,
): GatewayRequestDisplayKind => {
  const method = compactMethod(value.method);
  const normalizedPath = splitRequestPath(value.path);

  if (method === 'POST' && isContextCompactPath(normalizedPath)) {
    return 'contextCompact';
  }

  if (method === 'GET' || method === 'HEAD') {
    if (isModelListPath(normalizedPath)) {
      return 'modelList';
    }
    if (isConnectionProbePath(normalizedPath)) {
      return 'connectionProbe';
    }
  }

  if (!isPlaceholderModel(value.requested_model) || !isPlaceholderModel(value.upstream_model_id)) {
    return 'model';
  }
  if (method || normalizedPath) {
    return 'genericRequest';
  }
  return 'unknown';
};

export const requestDisplayTitleKey = (kind: GatewayRequestDisplayKind) => {
  switch (kind) {
    case 'modelList':
      return 'gateway.page.requests.requestTypes.modelList';
    case 'contextCompact':
      return 'gateway.page.requests.requestTypes.contextCompact';
    case 'connectionProbe':
      return 'gateway.page.requests.requestTypes.connectionProbe';
    case 'genericRequest':
      return 'gateway.page.requests.requestTypes.genericRequest';
    case 'unknown':
      return 'gateway.page.requests.requestTypes.unknown';
    case 'model':
    default:
      return null;
  }
};

export const isGatewayRequestUsageApplicable = (
  value: GatewayRequestDisplayInput | GatewayRequestDisplayKind,
) => {
  const kind = typeof value === 'string' ? value : gatewayRequestDisplayKind(value);
  return kind === 'model' || kind === 'contextCompact';
};

export const deriveGatewayRequestDisplay = (
  value: GatewayRequestDisplayInput,
): GatewayRequestDisplay => {
  const kind = gatewayRequestDisplayKind(value);
  const modelText = formatModelRoute(value.requested_model ?? null, value.upstream_model_id ?? null, '-');
  if (kind === 'model') {
    return {
      kind,
      titleKey: null,
      requestLine: '',
      modelText,
      modelApplicable: true,
    };
  }

  return {
    kind,
    titleKey: requestDisplayTitleKey(kind),
    requestLine: requestLineText(value, ''),
    modelText,
    modelApplicable: false,
  };
};

export const sanitizeGatewayFileNamePart = (
  value: string | null | undefined,
  fallback = REQUEST_EXPORT_FILE_NAME_FALLBACK,
) => {
  const normalized = value
    ?.trim()
    .replace(/[\\/:*?"<>|\s]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '');
  return normalized || fallback;
};

export const requestExportPrefix = (
  value: GatewayRequestDisplayInput,
  fallback = REQUEST_EXPORT_FILE_NAME_FALLBACK,
) => {
  const kind = gatewayRequestDisplayKind(value);
  if (kind === 'model') {
    return sanitizeGatewayFileNamePart(
      formatModelRoute(value.requested_model ?? null, value.upstream_model_id ?? null, ''),
      fallback,
    );
  }
  switch (kind) {
    case 'modelList':
      return 'models-list';
    case 'contextCompact':
      return 'compact';
    case 'connectionProbe':
      return 'probe';
    case 'genericRequest':
      return 'gateway-request';
    case 'unknown':
    default:
      return fallback;
  }
};

interface AttemptCountsInput {
  attempt_count: number;
  total_attempt_count?: number | null;
}

export const normalizeAttemptCounts = (value: AttemptCountsInput) => {
  const current = Math.max(value.attempt_count || 0, 1);
  return {
    current,
    total: Math.max(value.total_attempt_count || 0, current),
  };
};

export const shouldShowBodyComparison = (
  comparisonBody: string | null | undefined,
  primaryBody: string | null | undefined,
) => comparisonBody != null && comparisonBody !== primaryBody;

export const successRateText = (successCount: number, totalCount: number) => {
  if (totalCount <= 0) {
    return '-';
  }
  return `${Math.round((successCount / totalCount) * 100)}%`;
};

export const stringifyDetailValue = (value: unknown) => {
  if (value == null) {
    return '';
  }
  if (typeof value === 'string') {
    return value;
  }
  return JSON.stringify(value, null, 2);
};

export type GatewayUsageRangePreset = 'today' | '1d' | '7d' | '14d' | '30d' | 'custom';

interface GatewayDateLike {
  toDate: () => Date;
}

export interface GatewayUsageRangeSelection {
  preset: GatewayUsageRangePreset;
  customRange?: [GatewayDateLike | null, GatewayDateLike | null] | null;
}

export interface ResolvedGatewayUsageRange {
  startDate: number;
  endDate: number;
}

const DAY_SECONDS = 24 * 60 * 60;
const DAY_MS = DAY_SECONDS * 1000;

const startOfLocalDay = (timeMs: number) => {
  const date = new Date(timeMs);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
};

export const resolveGatewayUsageRange = (
  selection: GatewayUsageRangeSelection,
  nowMs = Date.now(),
): ResolvedGatewayUsageRange => {
  const endDate = Math.floor(nowMs / 1000);
  if (selection.preset === 'custom') {
    const [start, end] = selection.customRange ?? [];
    return {
      startDate: start ? Math.floor(start.toDate().getTime() / 1000) : endDate - DAY_SECONDS,
      endDate: end ? Math.floor(end.toDate().getTime() / 1000) : endDate,
    };
  }
  if (selection.preset === 'today') {
    return {
      startDate: Math.floor(startOfLocalDay(nowMs) / 1000),
      endDate,
    };
  }
  if (selection.preset === '1d') {
    return {
      startDate: endDate - DAY_SECONDS,
      endDate,
    };
  }
  const dayCount = selection.preset === '7d' ? 7 : selection.preset === '14d' ? 14 : 30;
  return {
    startDate: Math.floor(startOfLocalDay(nowMs - (dayCount - 1) * DAY_MS) / 1000),
    endDate,
  };
};
