/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import type { GrokProvider } from '../../../../../types/grok.ts';
import {
  buildGrokProviderSettingsWithModels,
  fromGrokModelFormValues,
  getGrokProviderCatalogModels,
  getGrokProviderDefaultModelKey,
  removeGrokCatalogModel,
  upsertGrokCatalogModel,
} from '../../../../../features/coding/grok/utils/grokProviderModels.ts';
import { buildGrokSettingsConfig } from '../../../../../features/coding/grok/utils/grokSettingsConfig.ts';
import { extractGrokSettingsBaseUrl } from '../../../../../utils/grokConfigUtils.ts';

const CHANNEL_BASE_URL = 'https://relay.example.com/v1';

/** Provider as saved by the provider form: channel field + matching entries. */
const createSavedProvider = (): GrokProvider => ({
  id: 'grok-provider',
  name: 'Grok Relay',
  category: 'custom',
  settingsConfig: buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: CHANNEL_BASE_URL,
    model: 'grok-4.5',
    apiFormat: 'openai_chat',
    reasoningEffort: 'high',
    defaultModelKey: 'grok-4.5',
    config: '',
    catalogModels: [{
      key: 'grok-4.5',
      model: 'grok-4.5',
      displayName: 'grok-4.5',
      baseUrl: CHANNEL_BASE_URL,
      apiBackend: 'chat_completions',
    }, {
      key: 'grok-fast',
      model: 'grok-4-fast',
      displayName: 'fast',
      baseUrl: CHANNEL_BASE_URL,
      apiBackend: 'chat_completions',
    }],
    auth: {},
  }),
  isApplied: false,
  createdAt: '2026-09-23T00:00:00.000Z',
  updatedAt: '2026-09-23T00:00:00.000Z',
});

/** Mirrors the card model-list delete handlers in GrokPage. */
const deleteModels = (provider: GrokProvider, modelKeys: string[]): string => {
  let models = getGrokProviderCatalogModels(provider);
  modelKeys.forEach((modelKey) => {
    models = removeGrokCatalogModel(models, modelKey);
  });
  const currentDefault = getGrokProviderDefaultModelKey(provider);
  const nextDefault = currentDefault && modelKeys.includes(currentDefault)
    ? (models[0]?.key || models[0]?.model)
    : currentDefault;
  return buildGrokProviderSettingsWithModels(provider, models, nextDefault);
};

test('deleting one model keeps the channel Base URL', () => {
  const provider = createSavedProvider();
  const settings = JSON.parse(deleteModels(provider, ['grok-fast']));

  assert.equal(settings.baseUrl, CHANNEL_BASE_URL);
  assert.equal(extractGrokSettingsBaseUrl(settings), CHANNEL_BASE_URL);
  assert.deepEqual(settings.modelCatalog.models.map((model: { key: string }) => model.key), ['grok-4.5']);
});

test('deleting the last model keeps the channel Base URL', () => {
  // Regression for issue #391: the Base URL used to live only in the catalog, so
  // removing the last entry silently dropped the whole channel address.
  const provider = createSavedProvider();
  const settings = JSON.parse(deleteModels(provider, ['grok-4.5', 'grok-fast']));

  assert.deepEqual(settings.modelCatalog.models, []);
  assert.equal(settings.defaultModelKey, undefined);
  assert.equal(settings.baseUrl, CHANNEL_BASE_URL);
  assert.equal(extractGrokSettingsBaseUrl(settings), CHANNEL_BASE_URL);
  // The API key must survive an emptied list as well.
  assert.equal(settings.auth.API_KEY, 'secret');
});

test('a model added after the list was emptied inherits the channel Base URL', () => {
  const provider = createSavedProvider();
  const emptied: GrokProvider = {
    ...provider,
    settingsConfig: deleteModels(provider, ['grok-4.5', 'grok-fast']),
  };

  const channelBaseUrl = extractGrokSettingsBaseUrl(JSON.parse(emptied.settingsConfig));
  const nextModel = fromGrokModelFormValues(
    { key: 'grok-4.6', model: 'grok-4.6', displayName: 'grok-4.6' },
    undefined,
    { baseUrl: channelBaseUrl },
  );
  const settings = JSON.parse(buildGrokProviderSettingsWithModels(
    emptied,
    upsertGrokCatalogModel([], nextModel),
    nextModel.key,
  ));

  assert.equal(settings.baseUrl, CHANNEL_BASE_URL);
  assert.equal(settings.defaultModelKey, 'grok-4.6');
  assert.equal(settings.modelCatalog.models[0].baseUrl, CHANNEL_BASE_URL);
});

test('records saved before the channel field existed promote it on the first catalog edit', () => {
  // Legacy record: the URL only lives inside the entries.
  const legacyProvider: GrokProvider = {
    ...createSavedProvider(),
    settingsConfig: JSON.stringify({
      auth: { API_KEY: 'secret' },
      config: '',
      defaultModelKey: 'grok-4.5',
      modelCatalog: {
        models: [
          { key: 'grok-4.5', model: 'grok-4.5', baseUrl: CHANNEL_BASE_URL, apiBackend: 'chat_completions' },
          { key: 'grok-fast', model: 'grok-4-fast' },
        ],
      },
    }),
  };

  // Deleting the entry that carried the URL must still keep the channel address.
  const settings = JSON.parse(deleteModels(legacyProvider, ['grok-4.5']));

  assert.equal(settings.baseUrl, CHANNEL_BASE_URL);
  assert.equal(extractGrokSettingsBaseUrl(settings), CHANNEL_BASE_URL);
  // The surviving entry keeps its own (absent) per-model URL untouched.
  assert.equal(settings.modelCatalog.models[0].baseUrl, undefined);
});

test('setting the default model does not touch the channel Base URL', () => {
  const provider = createSavedProvider();
  const settings = JSON.parse(buildGrokProviderSettingsWithModels(
    provider,
    getGrokProviderCatalogModels(provider),
    'grok-fast',
  ));

  assert.equal(settings.baseUrl, CHANNEL_BASE_URL);
  assert.equal(settings.defaultModelKey, 'grok-fast');
});

test('mixed per-model URLs are not frozen into the channel field', () => {
  // Third-party records can address different hosts per model. Deleting one model
  // must not collapse the others onto the channel field.
  const mixedProvider: GrokProvider = {
    ...createSavedProvider(),
    settingsConfig: JSON.stringify({
      auth: { API_KEY: 'secret' },
      config: '',
      defaultModelKey: 'relay-a',
      modelCatalog: {
        models: [
          { key: 'relay-a', model: 'm-a', baseUrl: 'https://a.test/v1' },
          { key: 'relay-b', model: 'm-b', baseUrl: 'https://b.test/v1' },
        ],
      },
    }),
  };

  const settings = JSON.parse(deleteModels(mixedProvider, ['relay-a']));

  assert.equal(settings.baseUrl, undefined);
  // The surviving model keeps its own address and is still what the UI reads.
  assert.equal(settings.modelCatalog.models[0].baseUrl, 'https://b.test/v1');
  assert.equal(extractGrokSettingsBaseUrl(settings), 'https://b.test/v1');
});

test('a malformed channel Base URL never throws in a render path', () => {
  assert.equal(
    extractGrokSettingsBaseUrl(JSON.parse('{"baseUrl":123,"modelCatalog":{"models":[]}}')),
    undefined,
  );
});

test('official providers keep no channel Base URL through catalog edits', () => {
  const officialProvider: GrokProvider = {
    ...createSavedProvider(),
    category: 'official',
    // A leaked custom channel field must not survive an official rewrite either.
    settingsConfig: JSON.stringify({
      auth: {},
      config: '',
      defaultModelKey: 'grok-4.5',
      baseUrl: 'https://leaked.example.com/v1',
    }),
  };

  const settings = JSON.parse(buildGrokProviderSettingsWithModels(
    officialProvider,
    getGrokProviderCatalogModels(officialProvider),
    'grok-4.5',
  ));

  assert.equal(settings.baseUrl, undefined);
  assert.equal(settings.modelCatalog, undefined);
});
