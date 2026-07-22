import assert from 'node:assert/strict';
import test from 'node:test';

import type { GrokProvider } from '../../../../../types/grok.ts';
import { saveGrokProviderCatalogWithGatewayReengage } from '../../../../../features/coding/grok/utils/grokProviderCatalogSave.ts';

const createProvider = (isApplied: boolean): GrokProvider => ({
  id: 'grok-provider',
  name: 'Grok Provider',
  category: 'custom',
  settingsConfig: JSON.stringify({
    defaultModelKey: 'old-model',
    modelCatalog: {
      models: [{ key: 'old-model', model: 'old-model' }],
    },
  }),
  isApplied,
  createdAt: '2026-07-22T00:00:00.000Z',
  updatedAt: '2026-07-22T00:00:00.000Z',
});

test('applied Grok catalog save restores direct before updating and reengages failover', async () => {
  const calls: string[] = [];
  let savedProvider: GrokProvider | undefined;

  const result = await saveGrokProviderCatalogWithGatewayReengage({
    provider: createProvider(true),
    settingsConfig: JSON.stringify({
      defaultModelKey: 'new-model',
      modelCatalog: {
        models: [{ key: 'new-model', model: 'upstream-model', displayName: 'New Model' }],
      },
    }),
    gatewayMode: 'failover',
    updateProvider: async (provider) => {
      calls.push('save');
      savedProvider = provider;
      return provider;
    },
    restoreDirect: async () => {
      calls.push('restore');
      return 'direct';
    },
    engageSingle: async () => {
      calls.push('single');
      return 'single';
    },
    engageFailover: async () => {
      calls.push('failover');
      return 'failover';
    },
    onGatewayStatusChange: (status) => {
      calls.push(`status:${status}`);
    },
  });

  assert.equal(result, savedProvider);
  assert.deepEqual(calls, [
    'restore',
    'status:direct',
    'save',
    'single',
    'failover',
    'status:failover',
  ]);
  assert.deepEqual(JSON.parse(savedProvider?.settingsConfig || '{}'), {
    defaultModelKey: 'new-model',
    modelCatalog: {
      models: [{ key: 'new-model', model: 'upstream-model', displayName: 'New Model' }],
    },
  });
});

test('unapplied Grok catalog save does not interrupt an active gateway takeover', async () => {
  const calls: string[] = [];

  await saveGrokProviderCatalogWithGatewayReengage({
    provider: createProvider(false),
    settingsConfig: JSON.stringify({
      defaultModelKey: 'new-model',
      modelCatalog: { models: [{ key: 'new-model', model: 'new-model' }] },
    }),
    gatewayMode: 'single',
    updateProvider: async (provider) => {
      calls.push('save');
      return provider;
    },
    restoreDirect: async () => {
      calls.push('restore');
      return 'direct';
    },
    engageSingle: async () => {
      calls.push('single');
      return 'single';
    },
    engageFailover: async () => {
      calls.push('failover');
      return 'failover';
    },
  });

  assert.deepEqual(calls, ['save']);
});

test('applied Grok catalog save writes directly when gateway mode is inactive', async () => {
  const calls: string[] = [];

  await saveGrokProviderCatalogWithGatewayReengage({
    provider: createProvider(true),
    settingsConfig: JSON.stringify({
      defaultModelKey: 'new-model',
      modelCatalog: { models: [{ key: 'new-model', model: 'new-model' }] },
    }),
    gatewayMode: null,
    updateProvider: async (provider) => {
      calls.push('save');
      return provider;
    },
    restoreDirect: async () => {
      calls.push('restore');
      return 'direct';
    },
    engageSingle: async () => {
      calls.push('single');
      return 'single';
    },
    engageFailover: async () => {
      calls.push('failover');
      return 'failover';
    },
  });

  assert.deepEqual(calls, ['save']);
});
