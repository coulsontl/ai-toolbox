import assert from 'node:assert/strict';
import test from 'node:test';

import type { CodexProvider } from '../../../../../types/codex.ts';
import {
  saveCodexProviderCatalogWithGatewayReengage,
  withProposedCodexAggregateModels,
} from '../../../../../features/coding/codex/utils/codexProviderCatalogSave.ts';

const createProvider = (isApplied: boolean): CodexProvider => ({
  id: 'codex-provider',
  name: 'Codex Provider',
  category: 'custom',
  settingsConfig: JSON.stringify({
    auth: { OPENAI_API_KEY: 'sk-test' },
    config: 'model = "old-model"',
    modelCatalog: { models: [{ model: 'old-model' }] },
  }),
  isApplied,
  createdAt: '2026-07-22T00:00:00.000Z',
  updatedAt: '2026-07-22T00:00:00.000Z',
});

const nextSettingsConfig = JSON.stringify({
  auth: { OPENAI_API_KEY: 'sk-test' },
  config: 'model = "new-model"',
  modelCatalog: { models: [{ model: 'new-model', displayName: 'New Model' }] },
});

test('applied Codex catalog save restores direct before updating and reengages failover', async () => {
  const calls: string[] = [];
  let savedProvider: CodexProvider | undefined;

  const result = await saveCodexProviderCatalogWithGatewayReengage({
    provider: createProvider(true),
    providers: [createProvider(true)],
    settingsConfig: nextSettingsConfig,
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
  assert.deepEqual(calls, ['restore', 'status:direct', 'save', 'single', 'failover', 'status:failover']);
  assert.equal(savedProvider?.settingsConfig, nextSettingsConfig);
});

test('applied Codex catalog save replays an aggregate takeover with its config', async () => {
  const calls: string[] = [];
  const provider = createProvider(true);
  const otherProvider: CodexProvider = { ...provider, id: 'site-b' };
  const aggregateConfig = { providerIds: [provider.id, otherProvider.id], separator: '-' };

  await saveCodexProviderCatalogWithGatewayReengage({
    provider,
    providers: [provider, otherProvider],
    settingsConfig: nextSettingsConfig,
    gatewayMode: 'aggregate',
    aggregateConfig,
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
    engageAggregate: async (config) => {
      calls.push(`aggregate:${config.providerIds.join(',')}`);
      return 'aggregate';
    },
    onGatewayStatusChange: (status) => {
      calls.push(`status:${status}`);
    },
  });

  assert.deepEqual(calls, [
    'restore',
    'status:direct',
    'save',
    'aggregate:codex-provider,site-b',
    'status:aggregate',
  ]);
});

test('unapplied Codex catalog save does not interrupt an active gateway takeover', async () => {
  const calls: string[] = [];

  await saveCodexProviderCatalogWithGatewayReengage({
    provider: createProvider(false),
    providers: [createProvider(false)],
    settingsConfig: nextSettingsConfig,
    gatewayMode: 'aggregate',
    aggregateConfig: { providerIds: ['a'], separator: '-' },
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
    engageAggregate: async () => {
      calls.push('aggregate');
      return 'aggregate';
    },
  });

  assert.deepEqual(calls, ['save']);
});

test('applied Codex catalog save writes directly when gateway mode is inactive', async () => {
  const calls: string[] = [];

  await saveCodexProviderCatalogWithGatewayReengage({
    provider: createProvider(true),
    providers: [createProvider(true)],
    settingsConfig: nextSettingsConfig,
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

test('proposed aggregate model set replaces edited provider and preserves other selected site models', () => {
  const editedProvider = createProvider(true);
  const otherProvider: CodexProvider = {
    ...editedProvider,
    id: 'site-b',
    settingsConfig: JSON.stringify({
      config: 'model = "site-b-default"',
      modelCatalog: { models: [{ model: 'site-b-mapped' }] },
    }),
  };
  const proposedSettingsConfig = JSON.stringify({
    config: 'model = "renamed-model"',
    modelCatalog: { models: [{ model: 'renamed-model' }] },
  });

  const config = withProposedCodexAggregateModels(
    {
      providerIds: [editedProvider.id, otherProvider.id],
      separator: '.',
      subagentExposedModels: ['old-model'],
    },
    [editedProvider, otherProvider],
    { id: editedProvider.id, settingsConfig: proposedSettingsConfig },
  );

  assert.deepEqual(config?.proposedDeclaredBareModels, [
    'renamed-model',
    'site-b-mapped',
    'site-b-default',
  ]);
});

test('proposed aggregate model set does not declare a chat-section-only default', () => {
  const provider = createProvider(true);
  const config = withProposedCodexAggregateModels(
    {
      providerIds: [provider.id],
      separator: '.',
      subagentExposedModels: [],
    },
    [provider],
    {
      id: provider.id,
      settingsConfig: JSON.stringify({
        config: '[chat]\nmodel = "chat-only-model"',
      }),
    },
  );

  assert.deepEqual(config?.proposedDeclaredBareModels, []);
});

test('proposed aggregate model set accepts the backend snake_case auto-review override', () => {
  const provider = createProvider(true);
  const config = withProposedCodexAggregateModels(
    {
      providerIds: [provider.id],
      separator: '.',
      subagentExposedModels: [],
    },
    [provider],
    {
      id: provider.id,
      settingsConfig: JSON.stringify({
        config: '',
        auto_review_model_override: ' snake-review-model ',
      }),
    },
  );

  assert.deepEqual(config?.proposedDeclaredBareModels, ['snake-review-model']);
});

test('proposed aggregate model set ignores malformed and blank catalog model rows', () => {
  const provider = createProvider(true);
  const config = withProposedCodexAggregateModels(
    {
      providerIds: [provider.id],
      separator: '.',
      subagentExposedModels: [],
    },
    [provider],
    {
      id: provider.id,
      settingsConfig: JSON.stringify({
        config: 'model = "default-model"',
        modelCatalog: {
          models: [
            null,
            { model: 42 },
            { model: '' },
            { model: ' mapped-model ' },
          ],
        },
      }),
    },
  );

  assert.deepEqual(config?.proposedDeclaredBareModels, ['mapped-model', 'default-model']);
});

test('catalog save fails closed for an unresolved selected aggregate site before restore or save', async () => {
  const provider = createProvider(true);
  const calls: string[] = [];

  await assert.rejects(
    saveCodexProviderCatalogWithGatewayReengage({
      provider,
      providers: [provider],
      settingsConfig: nextSettingsConfig,
      gatewayMode: 'aggregate',
      aggregateConfig: {
        providerIds: [provider.id, 'missing-site'],
        separator: '.',
        subagentExposedModels: [],
      },
      updateProvider: async (nextProvider) => {
        calls.push('save');
        return nextProvider;
      },
      restoreDirect: async () => {
        calls.push('restore');
        return 'direct';
      },
      engageSingle: async () => 'single',
      engageFailover: async () => 'failover',
      engageAggregate: async () => 'aggregate',
    }),
    /cannot resolve selected Codex provider 'missing-site'/,
  );

  assert.deepEqual(calls, []);
});

test('catalog save blocks a removed selection before restore and accepts a current model', async () => {
  const provider = createProvider(true);
  const calls: string[] = [];
  const selectedConfig = {
    providerIds: [provider.id],
    separator: '.',
    subagentExposedModels: ['old-model'],
  };
  const proposedSettingsConfig = JSON.stringify({
    config: 'model = "new-model"',
    modelCatalog: { models: [{ model: 'new-model' }] },
  });

  await assert.rejects(
    saveCodexProviderCatalogWithGatewayReengage({
      provider,
      providers: [provider],
      settingsConfig: proposedSettingsConfig,
      gatewayMode: 'aggregate',
      aggregateConfig: withProposedCodexAggregateModels(
        selectedConfig,
        [provider],
        { id: provider.id, settingsConfig: proposedSettingsConfig },
      ),
      updateProvider: async (nextProvider) => {
        calls.push('save');
        return nextProvider;
      },
      restoreDirect: async () => {
        calls.push('restore');
        return 'direct';
      },
      engageSingle: async () => 'single',
      engageFailover: async () => 'failover',
      engageAggregate: async () => 'aggregate',
    }),
    /containing undeclared models/,
  );
  assert.deepEqual(calls, []);

  const validCalls: string[] = [];
  await saveCodexProviderCatalogWithGatewayReengage({
    provider,
    providers: [provider],
    settingsConfig: proposedSettingsConfig,
    gatewayMode: 'aggregate',
    aggregateConfig: withProposedCodexAggregateModels(
      { ...selectedConfig, subagentExposedModels: ['new-model'] },
      [provider],
      { id: provider.id, settingsConfig: proposedSettingsConfig },
    ),
    updateProvider: async (nextProvider) => {
      validCalls.push('save-valid');
      return nextProvider;
    },
    restoreDirect: async () => {
      validCalls.push('restore-valid');
      return 'direct';
    },
    engageSingle: async () => 'single',
    engageFailover: async () => 'failover',
    engageAggregate: async () => {
      validCalls.push('aggregate-valid');
      return 'aggregate';
    },
  });
  assert.deepEqual(validCalls, ['restore-valid', 'save-valid', 'aggregate-valid']);
});
