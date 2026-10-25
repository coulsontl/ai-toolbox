import test from 'node:test';
import assert from 'node:assert/strict';
import type { CodexOfficialAccount } from '../../../../../types/codex.ts';
import {
  formatCodexOfficialAccountError,
  isCodexReauthRequiredError,
  matchesOfficialAccountRefresh,
  markOfficialAccountRefreshFailed,
  reconcileOfficialAccountUsage,
} from '../../../../../features/coding/codex/utils/codexAccountRefresh.ts';

const account: CodexOfficialAccount = {
  id: '__local__', providerId: 'official-a', accountId: 'login-a',
  name: 'Local', email: 'a@example.invalid', kind: 'local',
  isApplied: false, isVirtual: true, createdAt: '', updatedAt: '',
  credits: { hasCredits: true, balance: 42 }, limitWeeklyText: '75%',
  lastLimitsFetchedAt: '2026-10-24T12:00:00Z',
};

test('a refresh only updates the same account, provider and runtime login', () => {
  assert.equal(matchesOfficialAccountRefresh({ ...account }, account), true);
  for (const update of [
    { id: 'saved-account' }, { providerId: 'official-b' },
    { accountId: 'login-b' }, { email: 'b@example.invalid' },
  ]) {
    assert.equal(matchesOfficialAccountRefresh({ ...account, ...update }, account), false);
  }
  const unidentified = { ...account, accountId: undefined, email: undefined };
  assert.equal(matchesOfficialAccountRefresh(unidentified, unidentified), false);
  assert.equal(matchesOfficialAccountRefresh(
    { ...unidentified, refreshTokenPreview: 'synthetic-a' },
    { ...unidentified, refreshTokenPreview: 'synthetic-b' },
  ), false);
});

test('a failed refresh clears credits without inventing zero or advancing freshness', () => {
  const failed = markOfficialAccountRefreshFailed(account, 'synthetic usage failure');
  assert.equal(failed.credits, undefined);
  assert.equal(failed.lastError, 'synthetic usage failure');
  assert.equal(failed.lastLimitsFetchedAt, account.lastLimitsFetchedAt);
  assert.equal(failed.limitWeeklyText, '75%');
  assert.equal(account.credits?.balance, 42);
});

test('a failure marks the newest row instead of restoring the click-time snapshot', () => {
  // The list read that completed mid-flight is newer than the click; a failed
  // usage read says nothing about it and must not roll it back.
  const fresherRow = {
    ...account,
    limitWeeklyText: '90%',
    lastLimitsFetchedAt: '2026-10-24T13:00:00Z',
  };
  const failed = markOfficialAccountRefreshFailed(fresherRow, 'boom');
  assert.equal(failed.limitWeeklyText, '90%');
  assert.equal(failed.lastLimitsFetchedAt, '2026-10-24T13:00:00Z');
  assert.equal(failed.credits, undefined);
  assert.equal(failed.lastError, 'boom');
});

test('a cleared balance stays cleared until a success replaces it', () => {
  // The page drops the cached success on failure. Without that, a virtual row
  // (which the backend never persists) would get the stale balance re-applied
  // by the next list reload, resurrecting a value the failure invalidated.
  const marked = markOfficialAccountRefreshFailed(account, 'boom');
  // Rendering after the failure: the row carries no balance.
  assert.equal(marked.credits, undefined);
  // Once the cache entry is gone there is nothing left to reconcile.
  const reconciled = reconcileOfficialAccountUsage(marked, undefined, 0);
  assert.equal(reconciled.credits, undefined);
  assert.equal(reconciled.limitWeeklyText, '75%');
  // A later success is the only thing that brings a balance back.
  const recovered = { account: { ...account, credits: { hasCredits: true, balance: 7 } }, revision: 2 };
  assert.equal(reconcileOfficialAccountUsage(marked, recovered, 1).credits?.balance, 7);
});

test('virtual usage survives same-login reloads, but never crosses runtime identities', () => {
  const listed = { ...account, credits: undefined, name: 'Current metadata' };
  const result = { account, revision: 1 };
  const merged = reconcileOfficialAccountUsage(listed, result, 2);
  assert.equal(merged.credits?.balance, 42);
  assert.equal(merged.name, 'Current metadata');
  const switched = { ...listed, accountId: 'login-b' };
  assert.equal(reconcileOfficialAccountUsage(switched, result, 0), switched);
});

test('saved-account results only override list reads started before completion', () => {
  const saved = { ...account, id: 'saved-a', isVirtual: false };
  const listed = { ...saved, credits: { hasCredits: true, balance: 1 } };
  const result = { account: saved, revision: 2 };
  assert.equal(reconcileOfficialAccountUsage(listed, result, 1).credits?.balance, 42);
  assert.equal(reconcileOfficialAccountUsage(listed, result, 2), listed);
  const failed = { account: markOfficialAccountRefreshFailed(saved, 'failure'), revision: 3 };
  const reconciled = reconcileOfficialAccountUsage(listed, failed, 2);
  assert.equal(reconciled.credits, undefined);
  assert.equal(reconciled.lastError, 'failure');
  const current = { ...listed, isApplied: true, name: 'Renamed', accessTokenPreview: 'new-token' };
  const lateFailure = reconcileOfficialAccountUsage(current, failed, -1);
  assert.equal(lateFailure.isApplied, true);
  assert.equal(lateFailure.name, 'Renamed');
  assert.equal(lateFailure.accessTokenPreview, 'new-token');
});

test('only the reauth marker gets localized copy; other errors stay actionable as-is', () => {
  const t = ((key: string) => `translated:${key}`) as unknown as Parameters<
    typeof formatCodexOfficialAccountError
  >[1];
  const reauth = 'codex_reauth_required: Codex official account authorization expired (x). Sign in again.';
  assert.equal(isCodexReauthRequiredError(reauth), true);
  assert.equal(
    formatCodexOfficialAccountError(reauth, t),
    'translated:codex.provider.officialAccountReauthRequired',
  );
  for (const message of [
    'Codex usage request failed: HTTP status 429 Too Many Requests',
    'Codex usage request failed: HTTP status 503 Service Unavailable',
    'Failed to fetch usage: request timed out',
    'Failed to parse usage response: expected value',
  ]) {
    assert.equal(isCodexReauthRequiredError(message), false);
    assert.equal(formatCodexOfficialAccountError(message, t), message);
  }
  assert.equal(isCodexReauthRequiredError(undefined), false);
});
