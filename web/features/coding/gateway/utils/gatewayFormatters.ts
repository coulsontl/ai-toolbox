import type { GatewayCliKey, GatewayModelStats, GatewayRequestLogFilters, GatewayUsageTool, ProxyGatewaySettings, ProxyGatewayStatus } from '@/services';

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
  if (!Number.isFinite(durationMs) || durationMs < 0) {
    return '-';
  }
  if (durationMs < 1000) {
    return `${durationMs}ms`;
  }
  return `${(durationMs / 1000).toFixed(1)}s`;
};

/**
 * Duration cell for an imported local record. The value is an estimate the
 * parser derived from transcript timestamps, so an absent one must stay `-`
 * rather than read as a measured zero.
 */
export const sessionDurationText = (durationMs?: number | null) =>
  durationMs != null && Number.isFinite(durationMs) && durationMs > 0
    ? formatDuration(durationMs)
    : '-';

export const formatDurationPair = (durationMs: number, firstTokenMs?: number | null) => {
  if (firstTokenMs == null || !Number.isFinite(firstTokenMs) || firstTokenMs < 0
    || !Number.isFinite(durationMs) || firstTokenMs > durationMs) {
    return formatDuration(durationMs);
  }
  return `${(firstTokenMs / 1000).toFixed(1)}s/${(durationMs / 1000).toFixed(1)}s`;
};

interface GatewayTpsInput extends GatewayRequestDisplayInput {
  output_tokens?: number | null;
  duration_ms: number;
  first_token_ms?: number | null;
  is_streaming: boolean;
}

export const formatTps = (record: GatewayTpsInput): string | null => {
  const { output_tokens: outputTokens, duration_ms: durationMs, first_token_ms: firstTokenMs } = record;
  if (!isGatewayRequestUsageApplicable(record) || outputTokens == null
    || !Number.isFinite(outputTokens) || outputTokens <= 0
    || !Number.isFinite(durationMs) || durationMs <= 0) {
    return null;
  }
  // Local session rows carry only an estimated end-to-end duration and no
  // first-token mark, so their rate includes the wait before the first chunk.
  let generationMs = durationMs;
  if (record.data_source !== 'session' && record.is_streaming && firstTokenMs != null) {
    if (!Number.isFinite(firstTokenMs) || firstTokenMs < 0 || firstTokenMs >= durationMs) {
      return null;
    }
    generationMs -= firstTokenMs;
  }
  const tokensPerSecond = Number((outputTokens * 1000 / generationMs).toFixed(1));
  return `${tokensPerSecond} tok/s`;
};

/** Model text plus its request qualifiers, e.g. `gpt-5.6-sol (high · Fast)`. */
export const formatModelQualifiers = (
  modelText: string,
  qualifiers: Array<string | null | undefined>,
) => {
  const parts = qualifiers
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));
  return parts.length ? `${modelText} (${parts.join(' · ')})` : modelText;
};

export const formatModelWithEffort = (modelText: string, effort?: string | null) =>
  formatModelQualifiers(modelText, [effort]);

/**
 * Display label for a billed service tier. Only the canonical ids the gateway
 * records are recognized, so an unexpected value renders nothing instead of a
 * misleading badge.
 */
export const serviceTierLabel = (serviceTier?: string | null) => {
  const tier = serviceTier?.trim().toLowerCase();
  if (tier === 'fast') {
    return 'Fast';
  }
  if (tier === 'ultrafast') {
    return 'Ultrafast';
  }
  return null;
};

/**
 * i18n key for the compaction badge on a Codex remote compaction v2 turn.
 *
 * All three v2 values describe which side wrote the `compaction` item — the wire
 * looks identical either way, so this badge is the only place a user can tell
 * them apart without reading the upstream response.
 *
 * `local` is a different thing: Codex asked for a plain summary on the ordinary
 * `/responses` path and installs it itself, so no `compaction` item exists at
 * all. It is labelled anyway because the request list is the only place to see
 * that the turn was a compaction.
 */
export const compactionModeKey = (compactionMode?: string | null): string | null => {
  const mode = compactionMode?.trim().toLowerCase();
  if (mode === 'native') {
    return 'gateway.page.requests.compaction.native';
  }
  if (mode === 'fallback') {
    return 'gateway.page.requests.compaction.fallback';
  }
  if (mode === 'simulated') {
    return 'gateway.page.requests.compaction.simulated';
  }
  if (mode === 'local') {
    return 'gateway.page.requests.compaction.local';
  }
  return null;
};

/**
 * i18n key for one native/hosted tool that the upstream actually invoked.
 *
 * Returns literal keys so `i18n:check` can see every one of them. An unknown
 * kind renders nothing rather than the raw machine key: this badge is a
 * summary, and the detail dialog still lists the kind verbatim for debugging.
 */
export const nativeToolBadgeKey = (kind?: string | null): string | null => {
  switch (kind?.trim().toLowerCase()) {
    case 'web_search':
      return 'gateway.page.requests.nativeTools.webSearch';
    case 'image_generation':
      return 'gateway.page.requests.nativeTools.imageGeneration';
    case 'code_interpreter':
      return 'gateway.page.requests.nativeTools.codeInterpreter';
    case 'file_search':
      return 'gateway.page.requests.nativeTools.fileSearch';
    case 'mcp':
      return 'gateway.page.requests.nativeTools.mcp';
    case 'computer_use':
      return 'gateway.page.requests.nativeTools.computerUse';
    case 'local_shell':
      return 'gateway.page.requests.nativeTools.localShell';
    default:
      return null;
  }
};

/** How many tool badges a row shows before collapsing the rest into `+N`. */
export const NATIVE_TOOL_BADGE_LIMIT = 2;

/**
 * Tool badges for a request summary row.
 *
 * Only kinds the backend observed running become badges, so a declaration that
 * was merely forwarded never shows up here. Unknown kinds are dropped; the
 * overflow is counted into `hiddenCount`. `labels` carries the same tools in
 * the order they are shown (recognized kinds only) so the tooltip can name the
 * collapsed ones without exposing wire-level ids.
 */
export const nativeToolBadges = (
  kinds: readonly string[] | null | undefined,
): { keys: string[]; labels: string[]; hiddenCount: number; allKinds: string[] } => {
  const keys: string[] = [];
  const labels: string[] = [];
  const allKinds: string[] = [];
  for (const kind of kinds ?? []) {
    const badgeKey = nativeToolBadgeKey(kind);
    if (!badgeKey) {
      continue;
    }
    allKinds.push(kind.trim());
    labels.push(badgeKey);
    if (keys.length < NATIVE_TOOL_BADGE_LIMIT) {
      keys.push(badgeKey);
    }
  }
  return { keys, labels, hiddenCount: allKinds.length - keys.length, allKinds };
};

/**
 * Transport badge for a request summary row, or null when there is nothing
 * useful to say.
 *
 * A handshake row is the Gateway declining an upgrade, not a WebSocket turn:
 * showing `WS` on it would claim the request used a transport it never got. The
 * 426 case gets its own `回退 HTTP` badge instead, and every other handshake
 * outcome stays silent because the status column already reports it.
 */
export const gatewayTransportBadgeKey = (record: {
  transport?: string;
  request_kind?: string;
  data_source?: string | null;
  status_code?: number | null;
}): string | null => {
  if (record.data_source === 'session') {
    return null;
  }
  if (record.request_kind === 'websocket_handshake') {
    return record.status_code === 426 ? 'gateway.page.requests.websocket.fallback' : null;
  }
  return record.transport === 'websocket' ? 'gateway.page.requests.websocket.badge' : null;
};

export const formatCacheHitRate = (rate?: number | null) =>
  rate != null && Number.isFinite(rate) ? `${(rate * 100).toFixed(1)}%` : '-';

export const calculateCacheHitRate = (inputTokens: number, cacheReadTokens: number, cacheCreationTokens: number) => {
  const totalInputTokens = inputTokens + cacheReadTokens + cacheCreationTokens;
  return totalInputTokens > 0 ? cacheReadTokens / totalInputTokens : null;
};

/**
 * Tools the gateway only reads usage for, never proxies. Their request rate is
 * reported as `null` (`-`) instead of 0, which would read as "nothing running".
 */
const USAGE_ONLY_TOOLS: readonly GatewayUsageTool[] = [
  'pi', 'oh_my_pi', 'omo_native', 'dsh', 'hermes', 'openclaw', 'kimi_cli', 'zcode',
  'mimocode', 'prime_agent', 'devin', 'musecode', 'cmdcode', 'qwencode',
  'workbuddy_ai', 'minimax',
  'qoder_ide', 'qoder_ide_cn', 'qodercli', 'qodercli_cn',
];

/**
 * Usage spelling → takeover spelling, for the two tools that have both.
 *
 * A request rate is counted per *takeover* key (`GatewayCliKey::as_str()`:
 * `workbuddy_desktop` / `codebuddy_cli`), while the filters and the usage list
 * use the *usage* spelling (`workbuddy` / `codebuddy`). Without this the
 * lookup missed and the rate could only be shown as `-`.
 */
const TAKEOVER_SPELLING_BY_USAGE_TOOL: Partial<Record<GatewayUsageTool, GatewayCliKey>> = {
  workbuddy: 'workbuddy_desktop' as GatewayCliKey,
  codebuddy: 'codebuddy_cli' as GatewayCliKey,
};

export const getGatewayRequestsPerMinute = (
  status: Pick<ProxyGatewayStatus, 'requests_per_minute' | 'requests_per_minute_by_cli'> | null | undefined,
  cliKey?: GatewayUsageTool,
): number | null => {
  if (!status) {
    return null;
  }
  if (!cliKey) {
    return status.requests_per_minute;
  }
  // Usage-only tools are never routed by the gateway, so they have no
  // request rate at all; `null` renders as `-`, while 0 would claim silence.
  if (USAGE_ONLY_TOOLS.includes(cliKey)) {
    return null;
  }
  const rateKey = TAKEOVER_SPELLING_BY_USAGE_TOOL[cliKey] ?? (cliKey as GatewayCliKey);
  return status.requests_per_minute_by_cli == null
    ? null
    : status.requests_per_minute_by_cli[rateKey] ?? 0;
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

export const formatCompactInteger = (value: number | null | undefined, locale?: string) => {
  if (value == null) {
    return '-';
  }
  return new Intl.NumberFormat(locale, {
    notation: 'compact',
    maximumFractionDigits: 1,
  }).format(value);
};

export const formatUsd = (value: string | number | null | undefined, digits = 2) => {
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
  transport?: 'http' | 'websocket';
  request_kind?: 'request' | 'websocket_handshake' | 'websocket_warmup';
  total_tokens?: number | null;
  data_source?: string | null;
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

/**
 * Model ids in range that the pricing table cannot price at all, deduped across
 * CLIs and sorted. Placeholder rows (`unknown`) are not models and are excluded,
 * same as the models table. Priced-but-zero-cost rows are NOT reported: the
 * backend flag answers "does a price row resolve for this id", never "is the
 * displayed amount zero".
 */
export const unpricedModelIds = (
  modelStats: Array<Pick<GatewayModelStats, 'model' | 'has_pricing'>>,
) =>
  Array.from(
    new Set(
      modelStats
        .filter((row) => row.has_pricing === false && !isPlaceholderModel(row.model))
        .map((row) => row.model),
    ),
  ).sort();

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

  if (value.request_kind === 'websocket_handshake') {
    return 'connectionProbe';
  }

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
  if (typeof value !== 'string' && value.request_kind === 'websocket_handshake') {
    return false;
  }
  if (typeof value !== 'string' && value.request_kind === 'websocket_warmup') {
    return (value.total_tokens ?? 0) > 0;
  }
  if (typeof value !== 'string' && value.data_source === 'session') {
    return true;
  }
  const kind = typeof value === 'string' ? value : gatewayRequestDisplayKind(value);
  return kind === 'model' || kind === 'contextCompact';
};

export const gatewayWebSocketStatusKey = (record: {
  transport?: string;
  request_kind?: string;
  stream_outcome?: string | null;
  status_code?: number | null;
  success: boolean;
}): string | null => {
  if (record.transport !== 'websocket') return null;
  if (record.request_kind === 'websocket_handshake') {
    return record.status_code === 426 ? 'gateway.page.requests.websocket.fallback' : null;
  }
  const outcome = record.stream_outcome;
  if (outcome && ['completed', 'failed', 'incomplete', 'canceled'].includes(outcome)) {
    return `gateway.page.requests.websocket.${outcome}`;
  }
  return record.success ? 'gateway.page.requests.websocket.completed' : 'gateway.page.requests.websocket.failed';
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

/**
 * A project row is named by its last path segment (`D:\repo\ai-toolbox` ->
 * `ai-toolbox`), the same way the reference collector names projects. Both
 * separators are accepted because Windows transcripts use `\` and WSL ones `/`.
 * `null` means "no project recorded" and lets the caller label it.
 */
export const gatewayProjectName = (projectPath: string): string | null => {
  const trimmed = projectPath.trim().replace(/[\\/]+$/, '');
  if (!trimmed) {
    return null;
  }
  const segments = trimmed.split(/[\\/]/);
  return segments[segments.length - 1] || trimmed;
};

export const GATEWAY_USAGE_RANGE_PRESETS = [
  'today',
  'yesterday',
  'thisWeek',
  'lastWeek',
  'thisMonth',
  'lastMonth',
  'thisYear',
  'custom',
] as const;

export type GatewayUsageRangePreset = typeof GATEWAY_USAGE_RANGE_PRESETS[number];

export const isGatewayUsageRangePreset = (value: unknown): value is GatewayUsageRangePreset =>
  typeof value === 'string'
  && (GATEWAY_USAGE_RANGE_PRESETS as readonly string[]).includes(value);

/** Wire shape of `gateway_usage_range` in the persisted app settings. */
export interface StoredGatewayUsageRange {
  preset: string;
  start_date?: number | null;
  end_date?: number | null;
}

/**
 * Convert the page's range selection into the persisted shape. Relative
 * presets carry no absolute bounds: the backend re-resolves them from the
 * current clock, so "today" never freezes at the moment it was saved.
 */
export const gatewayUsageRangeToStored = (
  selection: GatewayUsageRangeSelection,
): StoredGatewayUsageRange => {
  if (selection.preset !== 'custom') {
    return { preset: selection.preset, start_date: null, end_date: null };
  }
  const [start, end] = selection.customRange ?? [];
  return {
    preset: 'custom',
    start_date: start ? Math.floor(start.toDate().getTime() / 1000) : null,
    end_date: end ? Math.floor(end.toDate().getTime() / 1000) : null,
  };
};

/** Stable key used to skip redundant saves when nothing actually changed. */
export const gatewayUsageRangeKey = (selection: GatewayUsageRangeSelection): string => {
  const stored = gatewayUsageRangeToStored(selection);
  return `${stored.preset}:${stored.start_date ?? ''}:${stored.end_date ?? ''}`;
};

interface GatewayDateLike {
  toDate: () => Date;
}

export interface GatewayUsageRangeSelection {
  preset: GatewayUsageRangePreset;
  customRange?: [GatewayDateLike | null, GatewayDateLike | null] | null;
}

export interface GatewayRequestRangeSelection extends Omit<GatewayUsageRangeSelection, 'preset'> {
  preset: GatewayUsageRangePreset | 'all';
}

export interface ResolvedGatewayUsageRange {
  startDate: number;
  endDate: number;
}

const DAY_SECONDS = 24 * 60 * 60;

const startOfLocalDay = (timeMs: number) => {
  const date = new Date(timeMs);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
};

/** Monday 00:00 local time of the week containing `timeMs`. */
const startOfLocalWeek = (timeMs: number) => {
  const date = new Date(startOfLocalDay(timeMs));
  const daysSinceMonday = (date.getDay() + 6) % 7;
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() - daysSinceMonday).getTime();
};

const startOfLocalMonth = (timeMs: number) => {
  const date = new Date(timeMs);
  return new Date(date.getFullYear(), date.getMonth(), 1).getTime();
};

const startOfLocalYear = (timeMs: number) => {
  const date = new Date(timeMs);
  return new Date(date.getFullYear(), 0, 1).getTime();
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
  if (selection.preset === 'yesterday') {
    return {
      startDate: Math.floor(startOfLocalDay(nowMs) / 1000) - DAY_SECONDS,
      endDate: Math.floor(startOfLocalDay(nowMs) / 1000) - 1,
    };
  }
  if (selection.preset === 'thisWeek') {
    return {
      startDate: Math.floor(startOfLocalWeek(nowMs) / 1000),
      endDate,
    };
  }
  if (selection.preset === 'lastWeek') {
    const thisWeekStart = startOfLocalWeek(nowMs);
    return {
      startDate: Math.floor(startOfLocalWeek(thisWeekStart - 1) / 1000),
      endDate: Math.floor(thisWeekStart / 1000) - 1,
    };
  }
  if (selection.preset === 'thisMonth') {
    return {
      startDate: Math.floor(startOfLocalMonth(nowMs) / 1000),
      endDate,
    };
  }
  if (selection.preset === 'lastMonth') {
    const thisMonthStart = startOfLocalMonth(nowMs);
    return {
      startDate: Math.floor(startOfLocalMonth(thisMonthStart - 1) / 1000),
      endDate: Math.floor(thisMonthStart / 1000) - 1,
    };
  }
  if (selection.preset === 'thisYear') {
    return {
      startDate: Math.floor(startOfLocalYear(nowMs) / 1000),
      endDate,
    };
  }
  // `GATEWAY_USAGE_RANGE_PRESETS` has no remaining rolling preset; the fallback
  // only keeps an unknown persisted value from producing an unbounded query.
  return {
    startDate: Math.floor(startOfLocalDay(nowMs) / 1000),
    endDate,
  };
};

export const resolveGatewayRequestRange = (
  selection: GatewayRequestRangeSelection,
  nowMs = Date.now(),
): Pick<GatewayRequestLogFilters, 'start_date' | 'end_date'> => {
  if (selection.preset === 'all') {
    return { start_date: null, end_date: null };
  }
  if (selection.preset === 'custom') {
    const [start, end] = selection.customRange ?? [];
    return {
      start_date: start ? Math.floor(start.toDate().getTime() / 1000) : null,
      end_date: end ? Math.floor(end.toDate().getTime() / 1000) : null,
    };
  }
  const range = resolveGatewayUsageRange({ preset: selection.preset }, nowMs);
  return { start_date: range.startDate, end_date: range.endDate };
};
