/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';
import type { TFunction } from 'i18next';

import {
  buildOfficialAccountResetLine,
  formatOfficialAccountCredits,
  formatQuotaResetAbsoluteText,
  formatQuotaResetRemainingText,
  formatQuotaResetText,
} from '../../../../../features/coding/codex/utils/codexQuotaDisplay.ts';
import type { CodexOfficialAccount } from '../../../../../types/codex.ts';

const NOW = 1_700_000_000;
const HOUR = 3600;
const DAY = 24 * HOUR;

const translate = ((key: string, options?: Record<string, unknown>) => {
  switch (key) {
    case 'codex.provider.officialAccountResetLinePrefix':
      return '重置：';
    case 'codex.provider.officialAccountResetDone':
      return '已重置';
    case 'codex.provider.officialAccountWeeklyLimitLabel':
      return '周限额';
    case 'codex.provider.officialAccountMonthlyLimitLabel':
      return '月限额';
    case 'codex.provider.officialAccountResetCredits':
      return `重置卡 ${String(options?.count)} 张`;
    case 'codex.provider.officialAccountCreditsUnlimited':
      return '不限量';
    case 'codex.provider.officialAccountCreditsNone':
      return '无可用积分';
    case 'codex.provider.officialAccountCreditsBalanceUnavailable':
      return `${String(options?.balance)} 积分（无可用积分）`;
    case 'codex.provider.officialAccountCreditsAvailable':
      return '可用（余额未知）';
    case 'codex.provider.officialAccountCreditsBalance':
      return `${String(options?.balance)} 积分`;
    default:
      return key;
  }
}) as unknown as TFunction;

function buildAccount(overrides: Partial<CodexOfficialAccount> = {}): CodexOfficialAccount {
  return {
    id: 'account-1',
    providerId: 'provider-1',
    name: 'account',
    kind: 'oauth',
    isApplied: false,
    isVirtual: false,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

test('formatQuotaResetRemainingText keeps windows compact and drops minutes once days remain', () => {
  assert.equal(formatQuotaResetRemainingText(NOW + 3 * DAY + 5 * HOUR, { nowSeconds: NOW }), '3d 5h');
  assert.equal(formatQuotaResetRemainingText(NOW + 2 * HOUR + 13 * 60, { nowSeconds: NOW }), '2h 13m');
  assert.equal(formatQuotaResetRemainingText(NOW + 12 * 60, { nowSeconds: NOW }), '12m');
  assert.equal(formatQuotaResetRemainingText(NOW + 45, { nowSeconds: NOW }), '<1m');
});

test('formatQuotaResetRemainingText reports expired and unknown windows', () => {
  assert.equal(formatQuotaResetRemainingText(NOW - 10, { nowSeconds: NOW, expiredLabel: '已重置' }), '已重置');
  assert.equal(formatQuotaResetRemainingText(NOW - 10, { nowSeconds: NOW }), '');
  assert.equal(formatQuotaResetRemainingText(undefined, { nowSeconds: NOW }), '');
  assert.equal(formatQuotaResetRemainingText(null, { nowSeconds: NOW }), '');
});

test('formatQuotaResetAbsoluteText renders a zero padded local timestamp', () => {
  const localTimestamp = new Date(2026, 8, 18, 20, 30, 0).getTime() / 1000;
  assert.equal(formatQuotaResetAbsoluteText(localTimestamp), '09/18 20:30');
  assert.equal(formatQuotaResetAbsoluteText(undefined), '');
});

test('formatQuotaResetText combines the remaining duration with the absolute time', () => {
  const resetAt = NOW + 2 * HOUR + 13 * 60;
  assert.equal(
    formatQuotaResetText(resetAt, { nowSeconds: NOW }),
    `2h 13m (${formatQuotaResetAbsoluteText(resetAt)})`,
  );
  assert.equal(
    formatQuotaResetText(NOW - 10, { nowSeconds: NOW, expiredLabel: '已重置' }),
    '已重置',
  );
  assert.equal(formatQuotaResetText(undefined, { nowSeconds: NOW }), '');
});

test('buildOfficialAccountResetLine joins every known window and the reset credit count', () => {
  const shortResetAt = NOW + 2 * HOUR + 13 * 60;
  const weeklyResetAt = NOW + 3 * DAY + 5 * HOUR;
  const account = buildAccount({
    limitShortLabel: '5h',
    limit5hResetAt: shortResetAt,
    limitWeeklyResetAt: weeklyResetAt,
    resetCreditsAvailable: 2,
  });

  assert.equal(
    buildOfficialAccountResetLine(account, translate, NOW),
    `重置：5h 2h 13m (${formatQuotaResetAbsoluteText(shortResetAt)})`
      + ` · 周限额 3d 5h (${formatQuotaResetAbsoluteText(weeklyResetAt)})`
      + ' · 重置卡 2 张',
  );
});

test('buildOfficialAccountResetLine keeps zero reset credits and hides unknown accounts', () => {
  assert.equal(
    buildOfficialAccountResetLine(buildAccount({ resetCreditsAvailable: 0 }), translate, NOW),
    '重置：重置卡 0 张',
  );
  assert.equal(buildOfficialAccountResetLine(buildAccount(), translate, NOW), null);
});

test('buildOfficialAccountResetLine marks expired windows with the translated label', () => {
  const account = buildAccount({ limitWeeklyResetAt: NOW - 10 });

  assert.equal(
    buildOfficialAccountResetLine(account, translate, NOW),
    '重置：周限额 已重置',
  );
});

test('formatOfficialAccountCredits keeps credits separate from weekly allowance and reset cards', () => {
  const account = buildAccount({
    limitWeeklyText: '75%',
    resetCreditsAvailable: 2,
    credits: { hasCredits: true, unlimited: false, balance: 62500.25 },
  });
  assert.equal(formatOfficialAccountCredits(account, translate), '62500.25 积分');
  assert.equal(account.limitWeeklyText, '75%');
  assert.equal(buildOfficialAccountResetLine(account, translate, NOW), '重置：重置卡 2 张');
});

test('formatOfficialAccountCredits distinguishes absent, zero, unavailable and unlimited', () => {
  assert.equal(formatOfficialAccountCredits(buildAccount(), translate), null);
  assert.equal(formatOfficialAccountCredits(buildAccount({ credits: {} }), translate), null);
  assert.equal(formatOfficialAccountCredits(buildAccount({
    credits: { hasCredits: true, balance: 0 },
  }), translate), '0 积分');
  assert.equal(formatOfficialAccountCredits(buildAccount({
    credits: { hasCredits: false, balance: 123 },
  }), translate), '123 积分（无可用积分）');
  assert.equal(formatOfficialAccountCredits(buildAccount({
    credits: { hasCredits: false, unlimited: true, balance: 0 },
  }), translate), '不限量');
});

test('formatOfficialAccountCredits does not invent a balance for invalid or stale data', () => {
  for (const balance of [undefined, NaN, Infinity]) {
    assert.equal(formatOfficialAccountCredits(buildAccount({
      credits: { hasCredits: true, balance },
    }), translate), '可用（余额未知）');
  }
  assert.equal(formatOfficialAccountCredits(buildAccount({ credits: { balance: 12 } }), translate), '12 积分');
  assert.equal(formatOfficialAccountCredits(buildAccount({
    credits: { hasCredits: false, balance: -0.25 },
  }), translate), '-0.25 积分（无可用积分）');
  assert.equal(formatOfficialAccountCredits(buildAccount({
    credits: { hasCredits: false },
  }), translate), '无可用积分');
  assert.equal(formatOfficialAccountCredits(buildAccount({
    credits: { hasCredits: true, balance: -0.25 },
  }), translate), '-0.25 积分');
  assert.equal(formatOfficialAccountCredits(buildAccount({
    credits: { hasCredits: true, balance: 12 }, lastError: 'usage request failed',
  }), translate), null);
});
