/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildOtherSettingsPatch,
  parseOtherConfigPayload,
} from '../../../../../features/coding/omo_native/utils/omoNativeOtherSettings.ts';

/**
 * 守的是「其他配置 → settings.json」页签的保存语义：只提交用户实际改动的键。
 * 整份覆盖会把页面打开期间引擎写入的 `tipsHistory` 等键回退。
 */

test('other settings patch contains only changed, added and removed keys', () => {
  const baseline = {
    theme: 'dark',
    lookAt: { models: ['gemini'] },
    tipsHistory: { tip: 1 },
  };
  const next = {
    theme: 'light',
    lookAt: { models: ['gemini'] },
    lookAtEnabled: true,
  };

  assert.deepEqual(buildOtherSettingsPatch(baseline, next), {
    set: {
      theme: 'light',
      lookAtEnabled: true,
    },
    remove: ['tipsHistory'],
  });
});

test('other settings patch is empty when the editor content is unchanged', () => {
  const baseline = { theme: 'dark', lookAt: { models: ['gemini'] } };

  assert.deepEqual(buildOtherSettingsPatch(baseline, { ...baseline }), {
    set: {},
    remove: [],
  });
});

test('non-object JSON5 is rejected instead of becoming an empty payload', () => {
  for (const parsed of [[], null, 123, 'text', true]) {
    assert.deepEqual(parseOtherConfigPayload(JSON.stringify(parsed), parsed, true), {
      ok: false,
      reason: 'notObject',
    });
  }

  // 空文本是显式清空，按 `{}` 保存。
  assert.deepEqual(parseOtherConfigPayload('  ', null, true), {
    ok: true,
    payload: {},
  });

  // 语法错误交给调用方跳过。
  assert.deepEqual(parseOtherConfigPayload('{ broken', null, false), {
    ok: false,
    reason: 'invalid',
  });

  // 普通对象原样返回。
  assert.deepEqual(
    parseOtherConfigPayload('{"lookAt":{"models":["gemini"]}}', { lookAt: { models: ['gemini'] } }, true),
    { ok: true, payload: { lookAt: { models: ['gemini'] } } },
  );
});

test('other settings patch leaves keys added on disk while the page was open untouched', () => {
  // 前端快照里没有 `tipsHistory`：它不在 set/remove 里，后端不会动它。
  const baseline = { theme: 'dark' };
  const next = { theme: 'light' };

  const patch = buildOtherSettingsPatch(baseline, next);
  // 先断言键集合，再 deepEqual：`assert.deepEqual` 会把 `remove` 收窄成 `never[]`，
  // 之后的 `.includes('tipsHistory')` 会被 TS 判为类型错误。
  assert.equal(patch.remove.includes('tipsHistory'), false);
  assert.equal('tipsHistory' in patch.set, false);
  assert.deepEqual(patch, { set: { theme: 'light' }, remove: [] });
});
