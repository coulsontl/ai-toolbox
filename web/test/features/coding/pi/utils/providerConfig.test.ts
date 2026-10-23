/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  maskApiKeyValue,
  maskProviderApiKey,
  resolvePiProviderApiKey,
} from '../../../../../features/coding/pi/utils/providerConfig.ts';

test('an auth.json api_key credential wins over models.json apiKey (issue #435)', () => {
  const apiKey = resolvePiProviderApiKey({
    credential: { type: 'api_key', key: 'sk-from-auth' },
    modelsProvider: { apiKey: 'sk-from-models' },
  });

  assert.equal(apiKey, 'sk-from-auth');
});

test('a provider configured only in auth.json still yields a key', () => {
  const apiKey = resolvePiProviderApiKey({
    credential: { type: 'api_key', key: '  sk-auth-only  ' },
    modelsProvider: {},
  });

  assert.equal(apiKey, 'sk-auth-only');
});

test('models.json apiKey is the fallback when no credential is stored', () => {
  const apiKey = resolvePiProviderApiKey({
    modelsProvider: { apiKey: 'sk-from-models' },
  });

  assert.equal(apiKey, 'sk-from-models');
});

test('OAuth credentials are not sent as an API key', () => {
  const apiKey = resolvePiProviderApiKey({
    credential: { type: 'oauth', refresh: 'r', access: 'a' },
    modelsProvider: {},
  });

  assert.equal(apiKey, '');
});

test('config value templates pass through unresolved so the backend can resolve them', () => {
  const apiKey = resolvePiProviderApiKey({
    credential: { type: 'api_key', key: '$MY_PI_KEY' },
    modelsProvider: { apiKey: '!op read op://vault/key' },
  });

  assert.equal(apiKey, '$MY_PI_KEY');
});

test('a blank credential key falls through to models.json instead of sending nothing', () => {
  const apiKey = resolvePiProviderApiKey({
    credential: { type: 'api_key', key: '   ' },
    modelsProvider: { apiKey: 'sk-from-models' },
  });

  assert.equal(apiKey, 'sk-from-models');
});

test('a non-string apiKey is ignored', () => {
  const apiKey = resolvePiProviderApiKey({
    modelsProvider: { apiKey: 42 },
  });

  assert.equal(apiKey, '');
});

test('a typeless credential is not treated as an API key', () => {
  // Pi's auth-storage rejects such an entry outright, and the backend's
  // credential_kind classifies it as OAuth — so it must not be sent.
  const apiKey = resolvePiProviderApiKey({
    credential: { key: 'sk-typeless' },
    modelsProvider: {},
  });

  assert.equal(apiKey, '');
});

test('the sidebar mask follows the same resolution as the requests', () => {
  assert.equal(
    maskProviderApiKey({
      credential: { type: 'api_key', key: 'sk-auth-1234567890' },
      modelsProvider: { apiKey: 'sk-models-1234567890' },
    }),
    'sk-a...7890',
  );
  assert.equal(
    maskProviderApiKey({ modelsProvider: { apiKey: 'sk-models-1234567890' } }),
    'sk-m...7890',
  );
});

test('the sidebar shows a provider configured only in models.json', () => {
  // The mirror image of issue #435: showing only the auth.json credential made
  // a models.json-only provider read as "not configured".
  const masked = maskProviderApiKey({
    credential: { type: 'oauth', access: 'a', refresh: 'r', expires: 1 },
    modelsProvider: { apiKey: 'sk-models-1234567890' },
  });

  assert.notEqual(masked, '');
  assert.equal(masked, 'sk-m...7890');
});

test('config-value templates and short keys are masked the way the sidebar expects', () => {
  assert.equal(maskApiKeyValue('$MY_PI_KEY'), '$MY_PI_KEY');
  assert.equal(maskApiKeyValue('!op read op://vault/key'), '!op read op://vault/key');
  assert.equal(maskApiKeyValue('  sk-auth-1234567890  '), 'sk-a...7890');
  assert.equal(maskApiKeyValue('short'), '********');
  assert.equal(maskApiKeyValue('   '), '');
});
