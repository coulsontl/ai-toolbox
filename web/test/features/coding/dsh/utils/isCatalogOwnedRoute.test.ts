import assert from 'node:assert/strict';
import test from 'node:test';

import { isCatalogOwnedRoute } from '../../../../../features/coding/dsh/utils/providerConfig.ts';
import type { DshRuntimeProviderView } from '../../../../../types/dsh.ts';

/**
 * Whether a route takes its name and protocol from the adapter catalog decides
 * two separate things: which fields the provider modal shows, and which fields
 * the save path writes. Both ask this function.
 *
 * The copy case is the regression this file exists for: copying a built-in
 * route used to keep the source's `isBuiltin`, so the modal hid 显示名称 /
 * API 类型 and the save deleted both — the copied route ended up with neither.
 */

const builtinRoute = {
  providerKey: 'deepseek-official',
  displayName: 'DeepSeek Official',
  credentialExists: false,
  modelIds: [],
  modelSource: 'builtin',
  isBuiltin: true,
  isDefault: false,
  warnings: [],
} as unknown as DshRuntimeProviderView;

const builtinBySourceOnly = {
  ...builtinRoute,
  providerKey: 'adapter-default',
  modelSource: 'builtin',
  isBuiltin: false,
} as unknown as DshRuntimeProviderView;

const customRoute = {
  providerKey: 'deepseek-custom',
  displayName: 'DeepSeek Custom',
  credentialExists: false,
  modelIds: ['deepseek-v4'],
  modelSource: 'explicit',
  isBuiltin: false,
  isDefault: false,
  warnings: [],
} as unknown as DshRuntimeProviderView;

test('a catalog-backed route is catalog-owned', () => {
  assert.equal(isCatalogOwnedRoute(builtinRoute, false), true);
});

test('a route named after the adapter default counts as catalog-backed', () => {
  assert.equal(isCatalogOwnedRoute(builtinBySourceOnly, false), true);
});

test('a custom route owns its name and protocol', () => {
  assert.equal(isCatalogOwnedRoute(customRoute, false), false);
});

test('copying a catalog-backed route produces a route that owns both fields', () => {
  assert.equal(isCatalogOwnedRoute(builtinRoute, true), false);
});

test('a brand-new route has nothing to inherit', () => {
  assert.equal(isCatalogOwnedRoute(undefined, false), false);
});
