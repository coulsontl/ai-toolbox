/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  openClawApiToNpm,
  toOpenCodeProvider,
} from '../../../../../features/coding/openclaw/utils/apiToNpm.ts';
import type { OpenClawProviderConfig } from '../../../../../types/openclaw.ts';

test('a known protocol wins over the endpoint hint', () => {
  assert.equal(
    openClawApiToNpm('openai-completions', 'https://api.anthropic.com'),
    '@ai-sdk/openai-compatible',
  );
  assert.equal(openClawApiToNpm('openai-responses'), '@ai-sdk/openai');
  assert.equal(openClawApiToNpm('anthropic-messages'), '@ai-sdk/anthropic');
  assert.equal(openClawApiToNpm('google-generative-ai'), '@ai-sdk/google');
});

test('a provider without a protocol falls back to its endpoint', () => {
  assert.equal(openClawApiToNpm(undefined, 'https://api.anthropic.com'), '@ai-sdk/anthropic');
  assert.equal(
    openClawApiToNpm(undefined, 'https://generativelanguage.googleapis.com/v1beta'),
    '@ai-sdk/google',
  );
  assert.equal(openClawApiToNpm(undefined, 'https://relay.example.test'), '@ai-sdk/openai-compatible');
  assert.equal(openClawApiToNpm(undefined, undefined), '@ai-sdk/openai-compatible');
});

test('toOpenCodeProvider maps the endpoint, key and catalog', () => {
  const config: OpenClawProviderConfig = {
    api: 'openai-completions',
    baseUrl: 'https://relay.example.test',
    apiKey: 'sk-test',
    models: [
      { id: 'model-a', name: 'Model A' },
      { id: 'model-b' },
    ],
  };

  assert.deepEqual(toOpenCodeProvider(config), {
    npm: '@ai-sdk/openai-compatible',
    options: { baseURL: 'https://relay.example.test', apiKey: 'sk-test' },
    models: {
      'model-a': { name: 'Model A' },
      'model-b': { name: 'model-b' },
    },
  });
});
