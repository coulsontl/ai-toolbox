import type { TFunction } from 'i18next';
import type { CodexOfficialAccount } from '@/types/codex';

/**
 * Backend marker for "this account cannot recover on its own; authorize again".
 * The backend keeps a stable prefix in `lastError` so a reloaded list can be
 * localized here instead of showing the raw English fallback.
 */
export const CODEX_REAUTH_REQUIRED_MARKER = 'codex_reauth_required:';

export function isCodexReauthRequiredError(message: string | undefined): boolean {
  return Boolean(message?.includes(CODEX_REAUTH_REQUIRED_MARKER));
}

/**
 * Copy for a stored or freshly returned `lastError`.
 *
 * Reauth is the only class that gets its own wording: the raw backend text says
 * "sign in again" in English and carries internal detail the user cannot act on.
 * Every other error is already actionable as-is (status code, timeout, parse).
 */
export function formatCodexOfficialAccountError(message: string, t: TFunction): string {
  return isCodexReauthRequiredError(message)
    ? t('codex.provider.officialAccountReauthRequired')
    : message;
}

/** The local row reuses __local__ across providers and different runtime logins. */
export function matchesOfficialAccountRefresh(
  current: CodexOfficialAccount,
  requested: CodexOfficialAccount,
): boolean {
  return current.id === requested.id
    && current.providerId === requested.providerId
    && current.isVirtual === requested.isVirtual
    && current.accountId === requested.accountId
    && current.email === requested.email
    && (!current.isVirtual || Boolean(current.accountId || current.email)
      || (Boolean(current.refreshTokenPreview || current.accessTokenPreview)
        && current.refreshTokenPreview === requested.refreshTokenPreview
        && current.accessTokenPreview === requested.accessTokenPreview));
}

/**
 * A failed read is not new usage. It only hides the balance and states why:
 * quota windows and the last successful fetch time stay, and the result is
 * never cached — a later account list must win over a stale failure.
 */
export function markOfficialAccountRefreshFailed(
  account: CodexOfficialAccount,
  error: string,
): CodexOfficialAccount {
  return { ...account, credits: undefined, lastError: error };
}

export interface OfficialAccountUsageResult {
  account: CodexOfficialAccount;
  revision: number;
}

/** A list can return a snapshot taken before an in-flight usage read completed. */
export function reconcileOfficialAccountUsage(
  listed: CodexOfficialAccount,
  result: OfficialAccountUsageResult | undefined,
  listStartedAtRevision: number,
): CodexOfficialAccount {
  // Virtual usage is never persisted by the backend, so a list can only ever
  // return the row without it; keep the revision gate for stored accounts,
  // whose rows do carry usage and must win whenever the read is newer.
  if (!result || !matchesOfficialAccountRefresh(listed, result.account)
    || (!listed.isVirtual && result.revision <= listStartedAtRevision)) {
    return listed;
  }
  // Virtual usage is not stored by the backend. Preserve it across same-login
  // reloads, but never replace metadata returned by the current account list.
  const usage = result.account;
  return {
    ...listed,
    limitShortLabel: usage.limitShortLabel,
    limit5hText: usage.limit5hText,
    limitWeeklyText: usage.limitWeeklyText,
    limitMonthlyText: usage.limitMonthlyText,
    limit5hResetAt: usage.limit5hResetAt,
    limitWeeklyResetAt: usage.limitWeeklyResetAt,
    limitMonthlyResetAt: usage.limitMonthlyResetAt,
    resetCreditsAvailable: usage.resetCreditsAvailable,
    credits: usage.credits,
    lastLimitsFetchedAt: usage.lastLimitsFetchedAt,
    lastError: usage.lastError,
  };
}
