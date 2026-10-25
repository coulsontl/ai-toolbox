import type { CodexOfficialAccount } from '@/types/codex';

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
