import assert from 'node:assert/strict';
import test from 'node:test';
import type { DshRuntimeProviderView } from '../../../../../types/dsh.ts';
import {
  buildDshProviderDeletionPlan,
  canDeleteDshProvider,
  credentialRefFromProviderKey,
} from '../../../../../features/coding/dsh/utils/providerDeletion.ts';
import { backupProvidersBeforeDelete } from '../../../../../features/coding/shared/providerList/providerBatchOperations.ts';

const makeProvider = (providerKey: string, overrides: Partial<DshRuntimeProviderView> = {}): DshRuntimeProviderView => ({
  providerKey, displayName: providerKey,
  apiKeyEnv: 'SHARED_KEY', apiKey: 'test-key', credentialExists: true,
  provider: { apiKeyEnv: 'SHARED_KEY', models: [{ id: 'model-a' }] },
  modelIds: ['model-a'], modelSource: 'explicit',
  isBuiltin: false, isDefault: false, warnings: [],
  ...overrides,
});

test('deletion excludes defaults and entries without owned data', () => {
  assert.equal(canDeleteDshProvider(makeProvider('custom')), true);
  // A catalog route is not automatically undeletable: what matters is whether
  // the user layer carries a provider entry or a credential for it.
  assert.equal(canDeleteDshProvider(makeProvider('catalog', {
    isBuiltin: true, modelSource: 'builtin' as const,
  })), true);
  for (const overrides of [
    { isDefault: true },
    { provider: undefined, credentialExists: false },
  ]) {
    assert.equal(canDeleteDshProvider(makeProvider('protected', overrides)), false);
  }
});

test('a catalog route with no user-layer record has nothing to delete', () => {
  assert.equal(canDeleteDshProvider(makeProvider('deepseek', {
    isBuiltin: true,
    modelSource: 'builtin' as const,
    provider: undefined,
    credentialExists: false,
  })), false);
});

test('a catalog-backed route with user data joins batch deletion', () => {
  const plan = buildDshProviderDeletionPlan([
    makeProvider('catalog', { isBuiltin: true, modelSource: 'builtin' as const }),
  ], ['catalog']);
  assert.deepEqual(plan.map((target) => target.provider.providerKey), ['catalog']);
});

test('shared credentials survive backup-delete-reimport for either selected provider', async () => {
  const providers = [makeProvider('first'), makeProvider('second')];
  const plan = buildDshProviderDeletionPlan(providers, ['first', 'second']);
  const credentials = new Map([['SHARED_KEY', 'test-key']]);
  const favorites = new Map<string, { refName: string; value: string }>();

  await backupProvidersBeforeDelete(plan, async (target) => {
    assert.equal(credentials.get('SHARED_KEY'), 'test-key');
    favorites.set(target.provider.providerKey, structuredClone(target.credential!));
  }, (target) => target.provider.providerKey);
  for (const target of plan) {
    if (target.deleteCredential) credentials.delete(target.credentialRef);
  }

  assert.deepEqual(plan.map((target) => target.deleteCredential), [true, false]);
  assert.equal(credentials.size, 0);
  const restored = favorites.get('second')!;
  credentials.set(restored.refName, restored.value);
  assert.equal(credentials.get('SHARED_KEY'), 'test-key');
});

test('an unselected provider keeps its shared credential', () => {
  const plan = buildDshProviderDeletionPlan([makeProvider('selected'), makeProvider('kept')], ['selected']);
  assert.equal(plan.length, 1);
  assert.equal(plan[0].deleteCredential, false);
});

test('selecting a protected default cannot remove it or its shared credential', () => {
  const plan = buildDshProviderDeletionPlan([
    makeProvider('selected'), makeProvider('default', { isDefault: true }),
  ], ['selected', 'default']);
  assert.deepEqual(plan.map((target) => target.provider.providerKey), ['selected']);
  assert.equal(plan[0].deleteCredential, false);
});

test('the deletion plan captures credentials before subsequent runtime views change', () => {
  const providers = [makeProvider('first'), makeProvider('second')];
  const plan = buildDshProviderDeletionPlan(providers, ['first', 'second']);
  providers[1].credentialExists = false;
  providers[1].apiKey = undefined;
  assert.deepEqual(plan[1].credential, { refName: 'SHARED_KEY', value: 'test-key' });
  assert.equal(credentialRefFromProviderKey('my-relay'), 'MY_RELAY_API_KEY');
  assert.equal(credentialRefFromProviderKey('existing_api_key'), 'EXISTING_API_KEY');
});
