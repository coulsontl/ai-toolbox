import type { PluginKind, PluginProviderRow, PluginUsage, PluginUsageWindow } from '@/services/pluginsApi';

/** Turn an unknown rejection value into one readable line. */
export const formatPluginError = (error: unknown): string => {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === 'string') {
    return error;
  }
  if (error && typeof error === 'object') {
    try {
      return JSON.stringify(error);
    } catch {
      return String(error);
    }
  }
  return String(error ?? '');
};

/**
 * Pick one localized value from a plugin-provided map. Plugins commonly use
 * `zh` / `en`, while the app translates with `zh-CN` / `en-US`, so try the
 * exact language first and then the family fallbacks.
 */
export const pickLocalizedText = (
  values: Record<string, string> | null | undefined,
  fallback: string,
  language: string,
): string => {
  if (!values) {
    return fallback;
  }
  const candidates = language.toLowerCase().startsWith('zh')
    ? [language, 'zh-CN', 'zh', 'en']
    : [language, 'en-US', 'en', 'zh'];
  for (const candidate of candidates) {
    const value = values[candidate];
    if (typeof value === 'string' && value.trim()) {
      return value;
    }
  }
  return Object.values(values).find((value) => typeof value === 'string' && value.trim()) ?? fallback;
};

export const getPluginSummary = (
  summary: Record<string, string> | null | undefined,
  language: string,
): string => pickLocalizedText(summary, '', language);

/**
 * A page title by the reader's language, then by its language family, then
 * the plain title the package declares — never an unrelated language.
 */
export const getPluginPageTitle = (
  page: { title: string; titles?: Record<string, string> | null },
  language: string,
): string => {
  const titles = page.titles ?? {};
  return titles[language] || titles[language.split('-')[0]] || page.title;
};

const SECRET_HEADER = /\b((?:authorization|set-cookie|cookie)["']?\s*[:=]\s*["']?)([^\s"'][^\r\n"']*)/gi;
const SECRET_ASSIGNMENT = /(\b(?:api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|session[_-]?token|client[_-]?secret|private[_-]?key|token|secret|password|passwd|code|key)["']?\s*[:=]\s*["']?)([^\s"',;&]{6,})(["']?)/gi;
const BEARER_TOKEN = /\b(Bearer\s+)[A-Za-z0-9._~+/-]+=*/gi;
const KNOWN_KEY_SHAPE = /\b(?:sk-[A-Za-z0-9_-]{8,}|gh[pousr]_[A-Za-z0-9]{8,}|github_pat_[A-Za-z0-9_]{8,}|xox[baprs]-[A-Za-z0-9-]{8,}|AIza[A-Za-z0-9_-]{10,})\b/g;

/**
 * Mask credentials in host log lines before they reach the DOM or clipboard.
 * Plugin logs may echo request headers, URLs and auth payloads verbatim.
 */
export const redactPluginLog = (text: string): string =>
  text
    .replace(SECRET_HEADER, (_match, prefix: string) => `${prefix}***`)
    .replace(SECRET_ASSIGNMENT, (_match, prefix: string, _value: string, suffix: string) => `${prefix}***${suffix}`)
    .replace(BEARER_TOKEN, (_match, prefix: string) => `${prefix}***`)
    .replace(KNOWN_KEY_SHAPE, '***');

export const NPM_REGISTRY_OFFICIAL = 'https://registry.npmjs.org';
export const NPM_REGISTRY_CHINA = 'https://registry.npmmirror.com';

/** Whether a registry URL points at the China npmmirror mirror. */
export const isChinaMirrorRegistry = (registry: string | null | undefined): boolean => {
  const normalized = (registry ?? '').trim().replace(/\/+$/, '');
  return normalized === NPM_REGISTRY_CHINA || normalized.includes('npmmirror.com');
};

export const formatCompactNumber = (value: number | null | undefined): string => {
  if (value == null || !Number.isFinite(value)) {
    return '';
  }
  if (Math.abs(value) >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (Math.abs(value) >= 1_000) {
    return `${(value / 1_000).toFixed(1)}k`;
  }
  return String(value);
};

export const formatMicroseconds = (micros: number): string => {
  if (!Number.isFinite(micros) || micros <= 0) {
    return '0 µs';
  }
  if (micros >= 1000) {
    const milliseconds = micros / 1000;
    return `${milliseconds >= 10 ? milliseconds.toFixed(0) : milliseconds.toFixed(1)} ms`;
  }
  return `${Math.round(micros)} µs`;
};

export const formatContextWindow = (tokens: number): string => {
  if (!Number.isFinite(tokens) || tokens <= 0) {
    return '—';
  }
  if (tokens >= 1_000_000) {
    return `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 === 0 ? 0 : 1)}M`;
  }
  if (tokens >= 1_000) {
    return `${Math.round(tokens / 1_000)}K`;
  }
  return String(tokens);
};

export const formatDateTime = (value: string | null | undefined, language: string): string => {
  if (!value) {
    return '';
  }
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return '';
  }
  return new Date(timestamp).toLocaleString(language, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
};

const RELATIVE_UNITS: Array<{ unit: Intl.RelativeTimeFormatUnit; seconds: number }> = [
  { unit: 'day', seconds: 86_400 },
  { unit: 'hour', seconds: 3_600 },
  { unit: 'minute', seconds: 60 },
];

/** Relative time for timestamps such as “last synced”. */
export const formatRelativeTime = (value: string | null | undefined, language: string): string => {
  if (!value) {
    return '';
  }
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return '';
  }
  const deltaSeconds = (timestamp - Date.now()) / 1000;
  const formatter = new Intl.RelativeTimeFormat(language, { numeric: 'auto' });
  for (const { unit, seconds } of RELATIVE_UNITS) {
    if (Math.abs(deltaSeconds) >= seconds) {
      return formatter.format(Math.round(deltaSeconds / seconds), unit);
    }
  }
  return formatter.format(Math.round(deltaSeconds), 'second');
};

export type CountdownUnit = 'day' | 'hour' | 'minute';

export interface CountdownParts {
  value: number;
  unit: CountdownUnit;
}

/** Whole-unit countdown; returns null for missing or unreadable timestamps. */
export const getCountdownParts = (
  value: string | null | undefined,
  now = Date.now(),
): CountdownParts | null => {
  if (!value) {
    return null;
  }
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return null;
  }
  const secondsLeft = Math.max(0, Math.round((timestamp - now) / 1000));
  if (secondsLeft >= 86_400) {
    return { value: Math.round(secondsLeft / 86_400), unit: 'day' };
  }
  if (secondsLeft >= 3_600) {
    return { value: Math.round(secondsLeft / 3_600), unit: 'hour' };
  }
  return { value: Math.max(1, Math.round(secondsLeft / 60)), unit: 'minute' };
};

export type PluginQuotaLevel = 'normal' | 'warning' | 'error';

/** One rendered allowance row, normalized from the plugin's usage hook. */
export interface PluginQuotaRow {
  key: string;
  name: string;
  model: string;
  percent: number | null;
  percentText: string;
  amountText: string;
  balanceText: string;
  displayText: string;
  resetAt: string;
  runsOutAt: string;
  credits: boolean;
  aside: boolean;
  level: PluginQuotaLevel;
}

const formatAmount = (value: number, language: string): string =>
  new Intl.NumberFormat(language, { maximumFractionDigits: 2 }).format(value);

const getQuotaLevel = (percent: number | null): PluginQuotaLevel => {
  if (percent == null) {
    return 'normal';
  }
  if (percent >= 90) {
    return 'error';
  }
  if (percent >= 70) {
    return 'warning';
  }
  return 'normal';
};

const getWindowPercent = (window: PluginUsageWindow): number | null => {
  if (typeof window.used === 'number' && Number.isFinite(window.used)) {
    return Math.max(0, window.used);
  }
  if (window.limit > 0) {
    return Math.max(0, (window.amount / window.limit) * 100);
  }
  return null;
};

/** Normalize quota windows into display-ready rows (bars, money, text). */
export const buildPluginQuotaRows = (
  usage: PluginUsage | null | undefined,
  language: string,
  now = Date.now(),
): PluginQuotaRow[] => {
  if (!usage?.windows?.length) {
    return [];
  }
  return usage.windows.map((window, index) => {
    const percent = getWindowPercent(window);
    const amountText = window.limit > 0
      ? `${formatAmount(window.amount, language)} / ${formatAmount(window.limit, language)} ${window.unit}`.trim()
      : window.amount > 0
        ? `${formatAmount(window.amount, language)} ${window.unit}`.trim()
        : '';
    const derivedResetAt = !window.resetsAt && window.resetSecs > 0
      ? new Date(now + window.resetSecs * 1000).toISOString()
      : '';
    return {
      key: `${index}-${window.name || window.family || window.pool || window.model || 'window'}`,
      name: window.name || window.family || window.pool || window.model,
      model: window.model,
      percent,
      percentText: percent == null ? '' : `${Math.round(percent)}%`,
      amountText,
      balanceText: window.balance == null
        ? ''
        : `${formatAmount(window.balance, language)} ${window.unit}`.trim(),
      displayText: window.display,
      resetAt: window.resetsAt || derivedResetAt,
      runsOutAt: window.runsOutAt,
      credits: window.credits,
      aside: window.aside,
      level: getQuotaLevel(percent),
    };
  });
};

export type PluginHealth = 'healthy' | 'warning' | 'idle';

export const getProviderHealth = (provider: PluginProviderRow): PluginHealth => {
  if (provider.listError) {
    return 'warning';
  }
  if (!provider.signedIn) {
    return 'idle';
  }
  return provider.fellBack ? 'warning' : 'healthy';
};

/** Empty market `kind` means Provider, mirroring the Rust mapping. */
export const getPluginKindCategory = (kind: string | null | undefined): PluginKind => {
  const normalized = (kind ?? '').trim().toLowerCase();
  if (normalized === 'middleware') {
    return 'middleware';
  }
  if (normalized === 'both') {
    return 'both';
  }
  if (normalized === 'page') {
    return 'page';
  }
  if (normalized === 'unknown') {
    return 'unknown';
  }
  return 'provider';
};

export const isProviderPluginKind = (kind: PluginKind): boolean =>
  kind === 'provider' || kind === 'both';

export const isMiddlewarePluginKind = (kind: PluginKind): boolean =>
  kind === 'middleware' || kind === 'both';

export const isPagePluginKind = (kind: PluginKind): boolean => kind === 'page';

/** Human-readable protocol name from an AI SDK package (`@ai-sdk/anthropic`). */
export const getProtocolLabel = (npm: string): string => {
  const cleaned = npm
    .replace(/^@ai-sdk\//, '')
    .replace(/^@/, '')
    .split('/')[0]
    .replace(/-/g, ' ')
    .trim();
  if (!cleaned) {
    return '—';
  }
  return cleaned.charAt(0).toUpperCase() + cleaned.slice(1);
};

/** Up to two deterministic initials for the fallback plugin tile. */
export const getPluginInitials = (name: string): string => {
  const cleaned = name.replace(/^@/, '').trim();
  if (!cleaned) {
    return '?';
  }
  const words = cleaned.split(/[\s\-_/]+/).filter(Boolean);
  if (words.length >= 2) {
    return `${Array.from(words[0])[0] ?? ''}${Array.from(words[1])[0] ?? ''}`.toUpperCase();
  }
  return Array.from(cleaned).slice(0, 2).join('').toUpperCase();
};

const ACCENT_COUNT = 4;

/** Stable accent slot (0..3) so one plugin keeps its tile colour in every list. */
export const getPluginAccentIndex = (seed: string): number => {
  let hash = 0;
  for (const character of seed) {
    hash = (hash * 31 + (character.codePointAt(0) ?? 0)) >>> 0;
  }
  return hash % ACCENT_COUNT;
};
