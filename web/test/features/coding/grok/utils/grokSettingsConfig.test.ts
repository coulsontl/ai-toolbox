/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import { normalizeGrokCatalogModels } from '../../../../../features/coding/grok/utils/grokCatalogModels.ts';
import {
  applyGrokEndpointSettingsConfig,
  buildGrokSettingsConfig,
  CUSTOM_GROK_MODEL_KEY,
} from '../../../../../features/coding/grok/utils/grokSettingsConfig.ts';
import {
  extractGrokSettingsModel,
  extractGrokSettingsReasoningEffort,
} from '../../../../../utils/grokConfigUtils.ts';

test('Grok catalog normalization preserves the complete model payload', () => {
  const normalizedModels = normalizeGrokCatalogModels([{
      key: 'grok-complete',
      model: 'upstream-grok',
      displayName: 'Grok Complete',
      description: 'Complete field fixture',
      baseUrl: 'https://model.example.com/v1',
      apiBackend: 'responses',
      apiKey: null,
      envKey: 'XAI_API_KEY',
      contextWindow: 131072,
      maxCompletionTokens: 16384,
      temperature: 0,
      topP: 0.9,
      supportsBackendSearch: false,
      supportsReasoningEffort: true,
      reasoningEffort: 'high',
      streamToolCalls: false,
      maxRetries: 0,
      inferenceIdleTimeoutSecs: 120,
      extraHeaders: {},
      extraConfig: {},
      supportsImage: false,
      vision: true,
      attachment: false,
      modalities: {
        input: ['text', 'image'],
        output: ['text'],
      },
    }]);

  assert.deepEqual(normalizedModels[0], {
    key: 'grok-complete',
    model: 'upstream-grok',
    displayName: 'Grok Complete',
    description: 'Complete field fixture',
    baseUrl: 'https://model.example.com/v1',
    apiBackend: 'responses',
    apiKey: null,
    envKey: 'XAI_API_KEY',
    contextWindow: 131072,
    maxCompletionTokens: 16384,
    temperature: 0,
    topP: 0.9,
    supportsBackendSearch: false,
    supportsReasoningEffort: true,
    reasoningEffort: 'high',
    streamToolCalls: false,
    maxRetries: 0,
    inferenceIdleTimeoutSecs: 120,
    extraHeaders: {},
    extraConfig: {},
    supportsImage: false,
    vision: true,
    attachment: false,
    modalities: {
      input: ['text', 'image'],
      output: ['text'],
    },
  });
});

test('buildGrokSettingsConfig overwrites stale model apiBackend with form apiFormat', () => {
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://chat.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_chat',
    config: '',
    catalogModels: [{
      key: 'custom',
      model: 'grok-4.5',
      displayName: 'custom',
      // Stale value left from a previous responses channel / import.
      apiBackend: 'responses',
    }],
    auth: {},
  }));

  // Single-slot legacy key "custom" soft-migrates to the upstream model id.
  assert.equal(settingsConfig.defaultModelKey, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].key, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].model, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].apiBackend, 'chat_completions');
});

test('buildGrokSettingsConfig overwrites stale catalog baseUrl with form baseUrl', () => {
  // Regression for issue #256: editing Base URL on an already-saved provider left
  // modelCatalog.models[].baseUrl (and live [model.<key>].base_url) unchanged.
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://grok2api.test.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_responses',
    config: '',
    catalogModels: [{
      key: 'custom',
      model: 'grok-4.5',
      displayName: 'grok-4.5',
      baseUrl: 'https://grok2api.ldsxcom.ccwu.cc/v1',
      apiBackend: 'responses',
    }, {
      key: 'grok-fast',
      model: 'grok-4-fast',
      displayName: 'fast',
      baseUrl: 'https://old-per-model.example.com/v1',
      apiBackend: 'responses',
    }],
    auth: {},
  }));

  // Multi-model catalogs keep free keys (including legacy "custom" slot) untouched.
  assert.equal(settingsConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.modelCatalog.models[0].key, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.modelCatalog.models[0].baseUrl, 'https://grok2api.test.com/v1');
  assert.equal(settingsConfig.modelCatalog.models[1].baseUrl, 'https://grok2api.test.com/v1');
  assert.equal(settingsConfig.modelCatalog.models[0].apiBackend, 'responses');
  // Channel-level SoT, so the URL survives model-list mutations (issue #391).
  assert.equal(settingsConfig.baseUrl, 'https://grok2api.test.com/v1');
});

test('official Grok providers never store a channel Base URL', () => {
  const officialSettings = JSON.parse(buildGrokSettingsConfig({
    category: 'official',
    apiKey: '',
    baseUrl: 'https://stale.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_chat',
    config: '',
    catalogModels: [],
    auth: {},
  }));

  assert.equal(officialSettings.baseUrl, undefined);
  assert.equal(officialSettings.modelCatalog, undefined);
  assert.equal(officialSettings.defaultModelKey, 'grok-4.5');
});

test('applyGrokEndpointSettingsConfig keeps the channel Base URL in sync with the endpoint', () => {
  // Form value (built by buildGrokSettingsConfig) wins over the endpoint default.
  const edited = JSON.parse(applyGrokEndpointSettingsConfig({
    settingsConfig: JSON.stringify({
      auth: { API_KEY: 'secret' },
      config: '',
      baseUrl: 'https://user-edited.example.com/v1',
      defaultModelKey: 'custom',
      modelCatalog: { models: [{ key: 'custom', model: 'grok-4.5' }] },
    }),
    apiFormat: 'openai_chat',
    endpointBaseUrl: 'https://endpoint.test/v1',
    endpointCatalogModels: [],
  }));

  assert.equal(edited.baseUrl, 'https://user-edited.example.com/v1');
  assert.equal(edited.modelCatalog.models[0].baseUrl, 'https://user-edited.example.com/v1');

  // No form value: the endpoint default becomes the channel value, so later model
  // list edits cannot lose it.
  const seeded = JSON.parse(applyGrokEndpointSettingsConfig({
    settingsConfig: JSON.stringify({ auth: {}, config: '' }),
    apiFormat: 'openai_chat',
    endpointBaseUrl: 'https://endpoint.test/v1',
    endpointCatalogModels: [{ key: 'endpoint-model', model: 'endpoint-model' }],
  }));

  assert.equal(seeded.baseUrl, 'https://endpoint.test/v1');
  assert.equal(seeded.modelCatalog.models[0].baseUrl, 'https://endpoint.test/v1');
});

test('buildGrokSettingsConfig leaves catalog baseUrl when form baseUrl is empty', () => {
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: '',
    model: 'grok-4.5',
    apiFormat: 'openai_chat',
    config: '',
    catalogModels: [{
      key: 'custom',
      model: 'grok-4.5',
      baseUrl: 'https://keep.example.com/v1',
    }],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultModelKey, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].key, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].baseUrl, 'https://keep.example.com/v1');
});

test('buildGrokSettingsConfig keeps multi-model keys and displayName under model-list ownership', () => {
  // Channel form projects baseUrl/apiBackend only. Free multi-model keys and displayName
  // stay under model-list ownership (not rewritten by the channel "model name" field).
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_responses',
    config: '',
    catalogModels: [{
      key: 'old-upstream',
      model: 'old-upstream',
      displayName: 'My Menu Label',
    }],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultModelKey, 'old-upstream');
  assert.equal(settingsConfig.modelCatalog.models[0].key, 'old-upstream');
  assert.equal(settingsConfig.modelCatalog.models[0].model, 'old-upstream');
  assert.equal(settingsConfig.modelCatalog.models[0].displayName, 'My Menu Label');
  assert.equal(settingsConfig.modelCatalog.models[0].apiBackend, 'responses');
  assert.equal(extractGrokSettingsModel(settingsConfig), 'old-upstream');
});

test('buildGrokSettingsConfig preserves free multi-model catalog keys', () => {
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'upstream-model',
    apiFormat: 'openai_chat',
    config: '',
    catalogModels: [{
      key: 'local-model-key',
      model: 'upstream-model',
      displayName: 'Keep Display',
    }],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultModelKey, 'local-model-key');
  assert.equal(settingsConfig.modelCatalog.models[0].key, 'local-model-key');
  assert.equal(settingsConfig.modelCatalog.models[0].model, 'upstream-model');
  assert.equal(settingsConfig.modelCatalog.models[0].displayName, 'Keep Display');
});

test('applyGrokEndpointSettingsConfig preserves edited mappings and form-owned fields', () => {
  const builtSettingsConfig = buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://edited.example.com/v1',
    model: 'edited-upstream-model',
    apiFormat: 'openai_responses',
    supportsBackendSearch: false,
    config: '[ui]\nsimple_mode = true',
    catalogModels: [{
      key: 'custom',
      model: 'edited-upstream-model',
      displayName: 'Edited Grok',
      contextWindow: 262144,
      baseUrl: 'https://old.example.com/v1',
      apiBackend: 'chat_completions',
      supportsBackendSearch: true,
    }],
    auth: {},
  });

  const settingsConfig = JSON.parse(applyGrokEndpointSettingsConfig({
    settingsConfig: builtSettingsConfig,
    apiFormat: 'openai_responses',
    endpointBaseUrl: 'https://endpoint-default.example.com/v1',
    endpointModel: 'grok-4.5',
    endpointCatalogModels: [{
      model: 'grok-4.5',
      displayName: 'Endpoint Default',
      contextWindow: 131072,
    }, {
      model: 'grok-fast',
      displayName: 'Endpoint Fast',
    }],
  }));

  // Legacy fixed key "custom" soft-migrates to the upstream model id on save.
  assert.equal(settingsConfig.defaultModelKey, 'edited-upstream-model');
  assert.equal(settingsConfig.config, '[ui]\nsimple_mode = true');
  assert.equal(settingsConfig.modelCatalog.models.length, 1);
  assert.deepEqual(settingsConfig.modelCatalog.models[0], {
    key: 'edited-upstream-model',
    model: 'edited-upstream-model',
    displayName: 'Edited Grok',
    baseUrl: 'https://edited.example.com/v1',
    apiBackend: 'responses',
    contextWindow: 262144,
    supportsBackendSearch: false,
  });
});

test('buildGrokSettingsConfig keeps official default model independent from stale catalog data', () => {
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'official',
    apiKey: '',
    baseUrl: '',
    model: 'grok-4.5',
    apiFormat: 'openai_chat',
    config: '',
    catalogModels: [{
      key: 'stale-custom-key',
      model: 'grok-4.5',
    }],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultModelKey, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog, undefined);
  assert.equal(settingsConfig.defaultReasoningEffort, undefined);
});

test('buildGrokSettingsConfig stores official defaultReasoningEffort', () => {
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'official',
    apiKey: '',
    baseUrl: '',
    model: 'grok-build',
    reasoningEffort: 'high',
    config: '',
    catalogModels: [],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultModelKey, 'grok-build');
  assert.equal(settingsConfig.defaultReasoningEffort, 'high');
  assert.equal(settingsConfig.modelCatalog, undefined);
});

test('buildGrokSettingsConfig permanently clears legacy official reasoning effort', () => {
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'official',
    apiKey: '',
    baseUrl: '',
    model: 'grok-build',
    reasoningEffort: undefined,
    config: [
      '[models]',
      'default = "grok-build"',
      'default_reasoning_effort = "high"',
      '',
      '[ui]',
      'simple_mode = true',
    ].join('\n'),
    catalogModels: [],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultReasoningEffort, undefined);
  assert.doesNotMatch(settingsConfig.config, /default_reasoning_effort/);
  assert.match(settingsConfig.config, /\[ui\]\nsimple_mode = true/);
  assert.equal(extractGrokSettingsReasoningEffort(settingsConfig), undefined);
});

test('buildGrokSettingsConfig stamps channel reasoningEffort only on default model without menu', () => {
  const enabledConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'claude-opus-4-6',
    apiFormat: 'anthropic_messages',
    reasoningEffort: 'medium',
    config: '',
    catalogModels: [
      { key: 'custom', model: 'claude-opus-4-6', reasoningEffort: 'low' },
      { key: 'extra', model: 'claude-sonnet' },
    ],
    auth: {},
  }));
  assert.equal(enabledConfig.defaultReasoningEffort, undefined);
  assert.equal(enabledConfig.modelCatalog.models[0].reasoningEffort, 'medium');
  assert.equal(enabledConfig.modelCatalog.models[0].supportsReasoningEffort, true);
  // Non-default models keep their own effort ownership.
  assert.equal(enabledConfig.modelCatalog.models[1].reasoningEffort, undefined);
  assert.equal(enabledConfig.modelCatalog.models[1].supportsReasoningEffort, undefined);

  const withMenu = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'claude-opus-4-6',
    apiFormat: 'anthropic_messages',
    reasoningEffort: 'medium',
    config: '',
    catalogModels: [{
      key: 'custom',
      model: 'claude-opus-4-6',
      reasoningEfforts: ['low', 'high'],
      reasoningEffort: 'high',
    }],
    auth: {},
  }));
  // Menu is model-list SoT; channel effort only applies when it is in the menu.
  assert.equal(withMenu.modelCatalog.models[0].reasoningEffort, 'high');
  assert.deepEqual(withMenu.modelCatalog.models[0].reasoningEfforts, ['low', 'high']);

  const preservedConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'claude-opus-4-6',
    apiFormat: 'anthropic_messages',
    config: '',
    catalogModels: [{
      key: 'custom',
      model: 'claude-opus-4-6',
      supportsReasoningEffort: true,
      reasoningEffort: 'high',
    }],
    auth: {},
  }));
  // Clearing channel effort no longer strips per-model effort.
  assert.equal(preservedConfig.modelCatalog.models[0].reasoningEffort, 'high');
  assert.equal(preservedConfig.modelCatalog.models[0].supportsReasoningEffort, true);
});

test('buildGrokSettingsConfig projects anthropic and responses form formats', () => {
  const responsesConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_responses',
    config: '',
    catalogModels: [{ key: 'custom', model: 'grok-4.5', apiBackend: 'chat_completions' }],
    auth: {},
  }));
  assert.equal(responsesConfig.modelCatalog.models[0].apiBackend, 'responses');

  const anthropicConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'claude-sonnet',
    apiFormat: 'anthropic_messages',
    config: '',
    catalogModels: [{ key: 'custom', model: 'claude-sonnet', apiBackend: 'responses' }],
    auth: {},
  }));
  assert.equal(anthropicConfig.modelCatalog.models[0].apiBackend, 'messages');
  // Single-slot legacy "custom" key soft-migrates to the upstream model id.
  assert.equal(anthropicConfig.defaultModelKey, 'claude-sonnet');
  assert.equal(anthropicConfig.modelCatalog.models[0].key, 'claude-sonnet');
});

test('buildGrokSettingsConfig forces supportsBackendSearch across catalog models', () => {
  const enabledConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://cpa.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_responses',
    supportsBackendSearch: true,
    config: '',
    catalogModels: [
      { key: 'custom', model: 'grok-4.5', supportsBackendSearch: false },
      { key: 'cpa-fast', model: 'grok-4-fast' },
    ],
    auth: {},
  }));
  assert.equal(enabledConfig.modelCatalog.models[0].supportsBackendSearch, true);
  assert.equal(enabledConfig.modelCatalog.models[1].supportsBackendSearch, true);

  const emptyCatalogConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://cpa.example.com/v1',
    model: 'cpa-grok45',
    apiFormat: 'openai_responses',
    supportsBackendSearch: true,
    config: '',
    catalogModels: [],
    auth: {},
  }));
  // Empty catalog bootstraps from the form upstream model id as both key and model.
  assert.equal(emptyCatalogConfig.defaultModelKey, 'cpa-grok45');
  assert.equal(emptyCatalogConfig.modelCatalog.models[0].key, 'cpa-grok45');
  assert.equal(emptyCatalogConfig.modelCatalog.models[0].model, 'cpa-grok45');
  assert.equal(emptyCatalogConfig.modelCatalog.models[0].supportsBackendSearch, true);

  const disabledConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://cpa.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_responses',
    supportsBackendSearch: false,
    config: '',
    catalogModels: [{ key: 'custom', model: 'grok-4.5', supportsBackendSearch: true }],
    auth: {},
  }));
  assert.equal(disabledConfig.modelCatalog.models[0].supportsBackendSearch, false);
});

test('extractGrokSettingsModel returns upstream model not local custom key', () => {
  assert.equal(extractGrokSettingsModel({
    defaultModelKey: 'custom',
    modelCatalog: {
      // GrokSettingsLike only types key/model/baseUrl/apiBackend/reasoningEffort.
      models: [{ key: 'custom', model: 'grok-4.5' }],
    },
  }), 'grok-4.5');

  assert.equal(extractGrokSettingsModel({
    defaultModelKey: 'grok-4.5',
  }), 'grok-4.5');
});

test('buildGrokSettingsConfig soft-migrates legacy custom key catalogs on save', () => {
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_chat',
    config: '',
    catalogModels: [{
      key: 'custom',
      model: 'grok-4.5',
      displayName: 'Grok 4.5',
      reasoningEffort: 'high',
    }],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultModelKey, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].key, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].model, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].displayName, 'Grok 4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].reasoningEffort, 'high');
});
