import type { TFunction } from 'i18next';
import type { CodexOfficialAccount } from '@/types/codex';

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR;

export interface QuotaResetTextOptions {
  /** Unix seconds used as "now"; injectable so tests stay deterministic. */
  nowSeconds?: number;
  /** Text used once the reset time is already in the past. */
  expiredLabel?: string;
}

/** Local absolute reset time formatted as `MM/DD HH:mm`. */
export function formatQuotaResetAbsoluteText(resetAt?: number | null): string {
  if (!resetAt) {
    return '';
  }

  const date = new Date(resetAt * 1000);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/**
 * Compact remaining duration such as `3d 5h`, `2h 13m`, `12m` or `<1m`.
 * Returns the expired label once the window has reset and `''` when unknown.
 */
export function formatQuotaResetRemainingText(
  resetAt?: number | null,
  options: QuotaResetTextOptions = {},
): string {
  if (!resetAt) {
    return '';
  }

  const nowSeconds = options.nowSeconds ?? Math.floor(Date.now() / 1000);
  const diffSeconds = resetAt - nowSeconds;
  if (diffSeconds <= 0) {
    return options.expiredLabel ?? '';
  }
  if (diffSeconds < SECONDS_PER_MINUTE) {
    return '<1m';
  }

  const days = Math.floor(diffSeconds / SECONDS_PER_DAY);
  const hours = Math.floor((diffSeconds % SECONDS_PER_DAY) / SECONDS_PER_HOUR);
  const minutes = Math.floor((diffSeconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE);

  const parts: string[] = [];
  if (days > 0) {
    parts.push(`${days}d`);
  }
  if (hours > 0) {
    parts.push(`${hours}h`);
  }
  if (minutes > 0 && days === 0) {
    parts.push(`${minutes}m`);
  }
  return parts.join(' ');
}

/** Remaining duration plus absolute time, e.g. `3d 5h (09/21 10:00)`. */
export function formatQuotaResetText(
  resetAt?: number | null,
  options: QuotaResetTextOptions = {},
): string {
  const remaining = formatQuotaResetRemainingText(resetAt, options);
  if (!remaining) {
    return '';
  }

  const absolute = formatQuotaResetAbsoluteText(resetAt);
  if (!absolute || remaining === options.expiredLabel) {
    return remaining;
  }
  return `${remaining} (${absolute})`;
}

/**
 * Builds the compact 10px reset summary line shown under the official account limits.
 * Returns `null` when neither reset time nor reset credit count is known.
 */
export function buildOfficialAccountResetLine(
  account: CodexOfficialAccount,
  t: TFunction,
  nowSeconds?: number,
): string | null {
  const expiredLabel = t('codex.provider.officialAccountResetDone');
  const segments: string[] = [];

  const appendWindow = (label: string, resetAt?: number | null) => {
    const text = formatQuotaResetText(resetAt, { expiredLabel, nowSeconds });
    if (text) {
      segments.push(`${label} ${text}`);
    }
  };

  appendWindow(account.limitShortLabel || '5h', account.limit5hResetAt);
  appendWindow(t('codex.provider.officialAccountWeeklyLimitLabel'), account.limitWeeklyResetAt);
  appendWindow(t('codex.provider.officialAccountMonthlyLimitLabel'), account.limitMonthlyResetAt);

  if (typeof account.resetCreditsAvailable === 'number' && Number.isFinite(account.resetCreditsAvailable)) {
    segments.push(
      t('codex.provider.officialAccountResetCredits', { count: account.resetCreditsAvailable }),
    );
  }

  if (segments.length === 0) {
    return null;
  }
  return `${t('codex.provider.officialAccountResetLinePrefix')}${segments.join(' · ')}`;
}

/**
 * A credit balance is a count, not currency: no thousands grouping, at most two
 * decimals, and no floating-point artifacts (`0.1 + 0.2`) or exponent notation
 * on the label. `en-US` is deliberate — the digits must not depend on the host
 * locale.
 */
function formatCreditsBalance(balance: number): string {
  const normalized = balance === 0 ? 0 : balance; // -0 reads as "0", not "-0"
  return normalized.toLocaleString('en-US', {
    useGrouping: false,
    maximumFractionDigits: 2,
  });
}

/** Additional credits do not alter the subscription/reset-card presentation. */
export function formatOfficialAccountCredits(
  account: CodexOfficialAccount,
  t: TFunction,
): string | null {
  const credits = account.credits;
  if (account.lastError || !credits) {
    return null;
  }
  if (credits.unlimited === true) {
    return t('codex.provider.officialAccountCreditsUnlimited');
  }
  if (typeof credits.balance === 'number' && Number.isFinite(credits.balance)) {
    // Availability is not the balance: retain even negative/contradictory values.
    const key = credits.hasCredits === false
      ? 'codex.provider.officialAccountCreditsBalanceUnavailable'
      : 'codex.provider.officialAccountCreditsBalance';
    return t(key, { balance: formatCreditsBalance(credits.balance) });
  }
  if (credits.hasCredits === false) {
    return t('codex.provider.officialAccountCreditsNone');
  }
  if (credits.hasCredits !== true) {
    return null;
  }
  return t('codex.provider.officialAccountCreditsAvailable');
}
