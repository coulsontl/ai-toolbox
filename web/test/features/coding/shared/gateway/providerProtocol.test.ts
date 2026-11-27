import assert from 'node:assert/strict';
import test from 'node:test';

import {
  extractPluginProviderIdFromUrl,
  getPluginProviderRef,
  grokProviderNeedsGatewayProxy,
  isPluginBackedProvider,
  providerNeedsGatewayProxy,
} from '../../../../../features/coding/shared/gateway/providerProtocol.ts';

test('getPluginProviderRef reads the camelCase reference the backend writes', () => {
  const meta = {
    pluginProvider: {
      pluginSpec: '@magpie/plugin-antigravity',
      providerId: 'antigravity',
      accountKey: 'antigravity#work',
      accountStrategy: 'single',
    },
  };

  assert.deepEqual(getPluginProviderRef(meta), {
    pluginSpec: '@magpie/plugin-antigravity',
    providerId: 'antigravity',
    accountKey: 'antigravity#work',
    accountStrategy: 'single',
  });
});

test('getPluginProviderRef accepts snake_case and defaults the strategy', () => {
  const meta = {
    plugin_provider: {
      plugin_spec: 'plugin-zai',
      provider_id: 'zai',
    },
  };

  assert.deepEqual(getPluginProviderRef(meta), {
    pluginSpec: 'plugin-zai',
    providerId: 'zai',
    accountKey: null,
    accountStrategy: 'auto_rotate',
  });
});

test('getPluginProviderRef rejects a reference without both identities', () => {
  assert.equal(getPluginProviderRef(null), null);
  assert.equal(getPluginProviderRef({}), null);
  assert.equal(getPluginProviderRef({ pluginProvider: 'plugin-zai' }), null);
  assert.equal(
    getPluginProviderRef({ pluginProvider: { providerId: 'zai' } }),
    null,
  );
  assert.equal(
    getPluginProviderRef({ pluginProvider: { pluginSpec: 'plugin-zai' } }),
    null,
  );
});

test('isPluginBackedProvider is true only for a complete plugin reference', () => {
  assert.equal(isPluginBackedProvider({}), false);
  assert.equal(isPluginBackedProvider({ apiFormat: 'openai_chat' }), false);
  assert.equal(
    isPluginBackedProvider({
      pluginProvider: { pluginSpec: 'plugin-zai', providerId: 'zai' },
    }),
    true,
  );
});

test('extractPluginProviderIdFromUrl recognizes direct plugin endpoints', () => {
  assert.equal(
    extractPluginProviderIdFromUrl('http://127.0.0.1:38271/plugin/zai/v1'),
    'zai',
  );
  assert.equal(
    extractPluginProviderIdFromUrl('http://localhost:38271/plugin/zai'),
    'zai',
  );
  assert.equal(
    extractPluginProviderIdFromUrl('http://127.0.0.1:38271/plugin/zai%2Dplus/v1/chat/completions'),
    'zai-plus',
  );
  assert.equal(
    extractPluginProviderIdFromUrl('https://api.example.com/v1/plugin/zai'),
    null,
  );
  assert.equal(extractPluginProviderIdFromUrl('https://api.example.com/v1'), null);
  assert.equal(extractPluginProviderIdFromUrl('not a url'), null);
  assert.equal(extractPluginProviderIdFromUrl(''), null);
  assert.equal(extractPluginProviderIdFromUrl(null), null);
});

test('a plugin-backed provider always requires the gateway', () => {
  const pluginMeta = {
    pluginProvider: { pluginSpec: 'plugin-zai', providerId: 'zai' },
  };

  // The protocol matches natively, but the transport still lives in the plugin
  // bridge, so a direct config would point at an unreachable endpoint.
  assert.equal(
    providerNeedsGatewayProxy('openai_chat', 'openai_chat', { meta: pluginMeta }),
    true,
  );
  assert.equal(
    providerNeedsGatewayProxy('openai_chat', 'openai_chat', {
      baseUrl: 'http://127.0.0.1:38271/plugin/zai/v1',
    }),
    true,
  );
  assert.equal(
    providerNeedsGatewayProxy('openai_chat', 'openai_chat', {
      meta: { apiFormat: 'openai_chat' },
      baseUrl: 'https://api.example.com/v1',
    }),
    false,
  );
  assert.equal(
    providerNeedsGatewayProxy('openai_chat', 'anthropic_messages', null),
    true,
  );
});

test('grok still gates Gemini Native and now gates plugin routes', () => {
  assert.equal(grokProviderNeedsGatewayProxy('gemini_native'), true);
  assert.equal(grokProviderNeedsGatewayProxy('openai_chat'), false);
  assert.equal(
    grokProviderNeedsGatewayProxy('openai_chat', {
      baseUrl: 'http://127.0.0.1:38271/plugin/grok/v1',
    }),
    true,
  );
});
