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

  assert.equal(settingsConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.modelCatalog.models[0].key, CUSTOM_GROK_MODEL_KEY);
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

  assert.equal(settingsConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.modelCatalog.models[0].baseUrl, 'https://grok2api.test.com/v1');
  assert.equal(settingsConfig.modelCatalog.models[1].baseUrl, 'https://grok2api.test.com/v1');
  assert.equal(settingsConfig.modelCatalog.models[0].apiBackend, 'responses');
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

  assert.equal(settingsConfig.modelCatalog.models[0].baseUrl, 'https://keep.example.com/v1');
});

test('buildGrokSettingsConfig form model name updates upstream model but not displayName', () => {
  // Form "model name" is upstream model ID. Local key is fixed "custom".
  // Mapping displayName is owned by the mapping UI and must not be rewritten.
  const settingsConfig = JSON.parse(buildGrokSettingsConfig({
    category: 'custom',
    apiKey: 'secret',
    baseUrl: 'https://api.example.com/v1',
    model: 'grok-4.5',
    apiFormat: 'openai_responses',
    config: '',
    catalogModels: [{
      key: 'custom',
      model: 'old-upstream',
      displayName: 'My Menu Label',
    }],
    auth: {},
  }));

  assert.equal(settingsConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.modelCatalog.models[0].key, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.modelCatalog.models[0].model, 'grok-4.5');
  assert.equal(settingsConfig.modelCatalog.models[0].displayName, 'My Menu Label');
  assert.equal(extractGrokSettingsModel(settingsConfig), 'grok-4.5');
});

test('buildGrokSettingsConfig migrates legacy non-custom keys to fixed custom key', () => {
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

  assert.equal(settingsConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.modelCatalog.models[0].key, CUSTOM_GROK_MODEL_KEY);
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

  assert.equal(settingsConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
  assert.equal(settingsConfig.config, '[ui]\nsimple_mode = true');
  assert.equal(settingsConfig.modelCatalog.models.length, 1);
  assert.deepEqual(settingsConfig.modelCatalog.models[0], {
    key: CUSTOM_GROK_MODEL_KEY,
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

test('buildGrokSettingsConfig projects channel reasoningEffort onto custom catalog', () => {
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
  assert.equal(enabledConfig.modelCatalog.models[1].reasoningEffort, 'medium');
  assert.equal(enabledConfig.modelCatalog.models[1].supportsReasoningEffort, true);

  const clearedConfig = JSON.parse(buildGrokSettingsConfig({
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
  assert.equal(clearedConfig.modelCatalog.models[0].reasoningEffort, undefined);
  assert.equal(clearedConfig.modelCatalog.models[0].supportsReasoningEffort, undefined);
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
  assert.equal(anthropicConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
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
  assert.equal(emptyCatalogConfig.defaultModelKey, CUSTOM_GROK_MODEL_KEY);
  assert.equal(emptyCatalogConfig.modelCatalog.models[0].key, CUSTOM_GROK_MODEL_KEY);
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
