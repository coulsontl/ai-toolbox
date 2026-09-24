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
  resolveGrokChannelApiBackend,
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

test('a model added after the list was emptied inherits the channel API format', () => {
  // Without the channel fallback the rebuilt entry has no api_backend, so the live
  // [model.<key>] makes the CLI fall back to chat_completions while the card still
  // shows the channel format the user configured.
  const emptiedProvider: GrokProvider = {
    ...createSavedProvider(),
    meta: { apiFormat: 'anthropic_messages' },
    settingsConfig: deleteModels(createSavedProvider(), ['grok-4.5', 'grok-fast']),
  };

  assert.equal(resolveGrokChannelApiBackend(emptiedProvider), 'messages');

  const nextModel = fromGrokModelFormValues(
    { key: 'grok-4.6', model: 'grok-4.6', displayName: 'grok-4.6' },
    undefined,
    {
      baseUrl: extractGrokSettingsBaseUrl(JSON.parse(emptiedProvider.settingsConfig)),
      apiBackend: resolveGrokChannelApiBackend(emptiedProvider),
    },
  );
  assert.equal(nextModel.apiBackend, 'messages');
});

test('the channel API format for new entries prefers the catalog over meta', () => {
  // Non-empty catalogs keep their existing behavior: the selected entry wins.
  const provider: GrokProvider = {
    ...createSavedProvider(),
    meta: { apiFormat: 'anthropic_messages' },
  };
  assert.equal(resolveGrokChannelApiBackend(provider), 'chat_completions');

  // Nothing known at all still lands on the Grok custom-channel default.
  const bareProvider: GrokProvider = {
    ...createSavedProvider(),
    settingsConfig: JSON.stringify({ auth: {}, config: '' }),
  };
  assert.equal(resolveGrokChannelApiBackend(bareProvider), 'chat_completions');
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

test('mixed per-model URLs are only promoted once the record states one address', () => {
  // Third-party records can address different hosts per model. The app never collapses
  // those onto each other: a mutation that runs while two distinct addresses exist
  // promotes nothing and keeps every entry's own URL. Once a mutation runs against a
  // record that states a single address, that address is promoted and from then on the
  // stored channel value always agrees with the surviving catalog.
  const mixedProvider = (): GrokProvider => ({
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
  });

  // One mutation while both addresses exist: nothing is promoted.
  const afterFirstDelete = JSON.parse(deleteModels(mixedProvider(), ['relay-a']));
  assert.equal(afterFirstDelete.baseUrl, undefined);
  assert.equal(afterFirstDelete.modelCatalog.models[0].baseUrl, 'https://b.test/v1');
  assert.equal(extractGrokSettingsBaseUrl(afterFirstDelete), 'https://b.test/v1');

  // A batch delete that empties such a record promotes nothing: the record never stated
  // one address, so there is no channel value to invent (same as before the fix).
  const emptiedInOneStep = JSON.parse(deleteModels(mixedProvider(), ['relay-a', 'relay-b']));
  assert.deepEqual(emptiedInOneStep.modelCatalog.models, []);
  assert.equal(emptiedInOneStep.baseUrl, undefined);

  // Chained deletes narrow the record to one address, which is then kept.
  const narrowed: GrokProvider = {
    ...mixedProvider(),
    settingsConfig: deleteModels(mixedProvider(), ['relay-a']),
  };
  const afterChainedDelete = JSON.parse(deleteModels(narrowed, ['relay-b']));
  assert.deepEqual(afterChainedDelete.modelCatalog.models, []);
  assert.equal(afterChainedDelete.baseUrl, 'https://b.test/v1');
});

test('a malformed channel Base URL value does not throw', () => {
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
