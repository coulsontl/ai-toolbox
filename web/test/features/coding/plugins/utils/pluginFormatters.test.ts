/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildPluginQuotaRows,
  getPluginKindCategory,
  getPluginPageTitle,
  isChinaMirrorRegistry,
  redactPluginLog,
} from '../../../../../features/coding/plugins/utils/pluginFormatters.ts';
import type {
  PluginUsage,
  PluginUsageWindow,
} from '../../../../../services/pluginsApi.ts';

test('isChinaMirrorRegistry matches npmmirror and ignores trailing slashes', () => {
  assert.equal(isChinaMirrorRegistry('https://registry.npmmirror.com'), true);
  assert.equal(isChinaMirrorRegistry('https://registry.npmmirror.com/'), true);
  assert.equal(isChinaMirrorRegistry(' https://registry.npmmirror.com// '), true);
  assert.equal(isChinaMirrorRegistry('https://mirror.example/npmmirror.com/proxy'), true);
  assert.equal(isChinaMirrorRegistry('https://registry.npmjs.org'), false);
  assert.equal(isChinaMirrorRegistry('https://npm.company.internal'), false);
  assert.equal(isChinaMirrorRegistry(''), false);
  assert.equal(isChinaMirrorRegistry(null), false);
});

test('redactPluginLog masks headers, assignments and known key shapes', () => {
  assert.equal(
    redactPluginLog('Authorization: Bearer sk-abcdef1234567890'),
    'Authorization: ***',
  );
  assert.equal(
    redactPluginLog('GET https://api.test/v1?api_key=abcdef123456&model=x'),
    'GET https://api.test/v1?api_key=***&model=x',
  );
  assert.equal(
    redactPluginLog('{"apiKey": "sk-test1234567890", "ok": true}'),
    '{"apiKey": "***", "ok": true}',
  );
  assert.equal(
    redactPluginLog('client_secret=supersecretvalue'),
    'client_secret=***',
  );
  assert.equal(
    redactPluginLog('Cookie: session=abcdef123456; theme=dark'),
    'Cookie: ***',
  );
});

test('redactPluginLog leaves ordinary diagnostics readable', () => {
  assert.equal(
    redactPluginLog('request failed code=404 encode=abcdefgh'),
    'request failed code=404 encode=abcdefgh',
  );
  assert.equal(
    redactPluginLog('token refresh scheduled in 5 minutes'),
    'token refresh scheduled in 5 minutes',
  );
});

const windowOf = (patch: Partial<PluginUsageWindow>): PluginUsageWindow => ({
  name: '',
  used: null,
  resetsAt: '',
  resetSecs: 0,
  display: '',
  amount: 0,
  limit: 0,
  unit: '',
  span: 0,
  model: '',
  models: [],
  notModels: [],
  family: '',
  pool: '',
  aside: false,
  runsOutAt: '',
  runsOut: 0,
  credits: false,
  balance: null,
  ...patch,
});

test('buildPluginQuotaRows normalizes percent, amount and balance windows', () => {
  const rows = buildPluginQuotaRows(
    {
      windows: [
        windowOf({ name: '5h', used: 42 }),
        windowOf({ name: 'weekly', amount: 30, limit: 100, unit: 'credits' }),
        windowOf({ name: 'wallet', balance: 12.5, unit: 'USD', display: 'Credits left' }),
      ],
    } as unknown as PluginUsage,
    'en-US',
  );

  assert.equal(rows[0].percentText, '42%');
  assert.equal(rows[0].level, 'normal');
  assert.equal(rows[1].amountText, '30 / 100 credits');
  assert.equal(rows[1].percent, 30);
  assert.equal(rows[2].balanceText, '12.5 USD');
  assert.equal(rows[2].displayText, 'Credits left');
});

test('buildPluginQuotaRows marks low remaining allowance as warning or error', () => {
  const rows = buildPluginQuotaRows(
    {
      windows: [
        windowOf({ name: 'tight', used: 75 }),
        windowOf({ name: 'spent', used: 96 }),
      ],
    } as unknown as PluginUsage,
    'en-US',
  );

  assert.equal(rows[0].level, 'warning');
  assert.equal(rows[1].level, 'error');
});

test('getPluginKindCategory treats an empty market kind as provider', () => {
  assert.equal(getPluginKindCategory(''), 'provider');
  assert.equal(getPluginKindCategory('middleware'), 'middleware');
  assert.equal(getPluginKindCategory('both'), 'both');
  assert.equal(getPluginKindCategory('page'), 'page');
});

test('getPluginPageTitle uses the language, its family, then the plain title', () => {
  const titles = { 'zh-CN': '控制台', zh: '控制台简', en: 'Console' };
  assert.equal(getPluginPageTitle({ title: 'Fallback', titles }, 'zh-CN'), '控制台');
  assert.equal(getPluginPageTitle({ title: 'Fallback', titles }, 'zh-TW'), '控制台简');
  assert.equal(getPluginPageTitle({ title: 'Fallback', titles: {} }, 'fr-FR'), 'Fallback');
});
