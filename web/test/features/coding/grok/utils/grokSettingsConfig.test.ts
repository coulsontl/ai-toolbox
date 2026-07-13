/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import { normalizeGrokCatalogModels } from '../../../../../features/coding/grok/utils/grokCatalogModels.ts';

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
