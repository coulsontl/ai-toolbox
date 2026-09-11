import assert from 'node:assert/strict';
import test from 'node:test';
import {
  extractProviderShareFields,
  extractProviderShareVariants,
  hasUnresolvedShareCredential,
  PROVIDER_SHARE_TOOLS,
  type ProviderShareApp,
  type ShareableProvider,
} from '../../../../features/shared/deepLink/providerTransfer.ts';
import { buildProviderShareUrl } from '../../../../features/shared/deepLink/providerShareUrl.ts';

function source(settings: unknown, extra: Partial<ShareableProvider> = {}): ShareableProvider {
  return { id: 'relay', name: 'Relay', category: 'custom', settingsConfig: JSON.stringify(settings), ...extra };
}

test('every supported tool can produce a portable connection URL', () => {
  const sources: Record<ProviderShareApp, ShareableProvider> = {
    claude: source({ env: { ANTHROPIC_API_KEY: 'test-key', ANTHROPIC_BASE_URL: 'https://relay.test', ANTHROPIC_MODEL: 'model/a' } }),
    claudedesktop: source({ env: { ANTHROPIC_AUTH_TOKEN: 'test-key', ANTHROPIC_BASE_URL: 'https://relay.test', ANTHROPIC_MODEL: 'model/a' } }),
    codex: source({ auth: { OPENAI_API_KEY: 'test-key' }, config: 'model_provider="relay"\nmodel="model/a"\n[model_providers.relay]\nbase_url="https://relay.test"\nwire_api="responses"' }),
    grok: source({ auth: { API_KEY: 'test-key' }, defaultModelKey: 'main', modelCatalog: { models: [{ key: 'main', model: 'model/a', baseUrl: 'https://relay.test', apiBackend: 'chat_completions' }] } }),
    kimi: source({ auth: { API_KEY: 'test-key' }, defaultModelKey: 'main', providerConfigs: { relay: { type: 'openai_legacy', base_url: 'https://relay.test' } }, modelCatalog: { models: [{ key: 'main', model: 'model/a', provider: 'relay' }] } }),
    gemini: source({ env: { GEMINI_API_KEY: 'test-key', GOOGLE_GEMINI_BASE_URL: 'https://relay.test', GEMINI_MODEL: 'model/a' } }),
    opencode: source({ npm: '@ai-sdk/openai-compatible', options: { apiKey: 'test-key', baseURL: 'https://relay.test' }, models: { 'model/a': {} } }),
    openclaw: source({ api: 'openai-completions', apiKey: 'test-key', baseUrl: 'https://relay.test', models: [{ id: 'model/a' }] }),
    pi: source({ api: 'openai-completions', baseUrl: 'https://relay.test', models: [{ id: 'model/a' }] }, { credential: { type: 'api_key', key: 'test-key' } }),
    omp: source({ api: 'openai-responses', apiKey: 'test-key', baseUrl: 'https://relay.test', models: [{ id: 'model/a' }] }),
    hermes: source({ api_mode: 'openai', api_key: 'test-key', base_url: 'https://relay.test', models: { 'model/a': { context_length: 64000 } } }),
    dsh: source({ api: 'anthropic-messages', baseURL: 'https://relay.test', models: [{ id: 'model/a' }] }, { credential: 'test-key' }),
  };
  assert.equal(PROVIDER_SHARE_TOOLS.length, 12);
  for (const { app } of PROVIDER_SHARE_TOOLS) {
    const fields = extractProviderShareFields(app, sources[app]);
    const url = new URL(buildProviderShareUrl({ ...fields, app: 'codex', name: 'Relay' }));
    assert.equal(url.searchParams.get('sourceApp'), app, app);
    assert.equal(url.searchParams.get('app'), 'codex', app);
    assert.equal(url.searchParams.get('baseUrl'), 'https://relay.test', app);
    assert.equal(url.searchParams.get('apiKey'), 'test-key', app);
    assert.ok(url.searchParams.get('apiFormat'), app);
    assert.equal(url.searchParams.get('model'), 'model/a', app);
    assert.equal(url.searchParams.has('config'), false, app);
    assert.equal(url.searchParams.has('extra'), false, app);
  }
});

test('Claude roles, authentication and all mapped Desktop models survive sharing', () => {
  const fields = extractProviderShareFields('claude', source({ env: {
    ANTHROPIC_API_KEY: 'api-key', ANTHROPIC_DEFAULT_SONNET_MODEL: 'sonnet-v1[1M]', ANTHROPIC_DEFAULT_OPUS_MODEL: 'opus-v1',
  } }));
  assert.equal(fields.apiKeyField, 'x-api-key');
  assert.equal(fields.model, 'sonnet-v1');
  assert.deepEqual(fields.modelRoles, { sonnet: 'sonnet-v1', opus: 'opus-v1' });
  assert.deepEqual(fields.models?.map((model) => model.id), ['sonnet-v1', 'opus-v1']);

  const desktop = extractProviderShareFields('claudedesktop', source({}, { meta: {
    claudeDesktopModelRoutes: {
      'claude-sonnet-share-1': { model: 'gpt/one', labelOverride: 'One' },
      'claude-sonnet-share-2': { model: 'gpt/two', tierAlias: 'sonnet' },
      'claude-opus-5': { model: 'opus/upstream', tierAlias: 'opus' },
    },
  } }));
  assert.deepEqual(desktop.models?.map((model) => model.id), ['gpt/one', 'gpt/two', 'opus/upstream']);
  assert.equal(desktop.modelRoles?.sonnet, 'gpt/two');
});

test('Codex catalog order does not invent a default and chat.model wins when present', () => {
  const settings = {
    config: 'model_provider="custom"\n[model_providers.custom]\nbase_url="https://relay.test/v1"\nwire_api="responses"',
    modelCatalog: { models: [{ model: 'second', contextWindow: 128000 }, { model: 'first' }] },
  };
  const noDefault = extractProviderShareFields('codex', source(settings));
  assert.equal(noDefault.model, undefined);
  assert.deepEqual(noDefault.models?.map((model) => model.id), ['second', 'first']);
  const explicit = extractProviderShareFields('codex', source({ ...settings, config: `${settings.config}\n[chat]\nmodel="first"` }));
  assert.equal(explicit.model, 'first');
});

test('OpenCode aliases resolve to upstream IDs without losing embedded slashes', () => {
  const fields = extractProviderShareFields('opencode', source({
    npm: '@ai-sdk/anthropic', options: { authToken: 'bearer-key', baseURL: 'https://relay.test/v1', headers: { 'X-Project': 'a+b' } },
    models: { quick: { id: 'vendor/model/a', name: 'Quick', limit: { context: 64000, output: 12000 } } },
  }, { defaultModel: 'quick' }));
  assert.equal(fields.model, 'vendor/model/a');
  assert.deepEqual(fields.models?.map((model) => model.id), ['vendor/model/a']);
  assert.equal(fields.models?.[0].maxTokens, 12000);
  assert.equal(fields.apiKeyField, 'bearer');
  assert.equal(fields.apiKey, 'bearer-key');
  assert.deepEqual(fields.headers, [{ op: 'set', name: 'X-Project', value: 'a+b' }]);
});

test('non-Claude model identifiers retain literal bracket suffixes', () => {
  const fields = extractProviderShareFields('opencode', source({
    npm: '@ai-sdk/openai', models: { custom: { id: 'vendor/model[1m]' } },
  }, { defaultModel: 'custom' }));
  assert.equal(fields.model, 'vendor/model[1m]');
  assert.equal(fields.models?.[0].id, 'vendor/model[1m]');
});

test('Kimi capabilities and per-provider API keys are read from their native fields', () => {
  const fields = extractProviderShareFields('kimi', source({
    providerConfigs: { main: { type: 'openai_responses', api_key: 'native-key', base_url: 'https://relay.test/v1' } },
    modelCatalog: { models: [{ key: 'one', provider: 'main', model: 'm1', maxContextSize: 64000, capabilities: ['image_in', 'video_in', 'thinking'] }] },
  }));
  assert.equal(fields.apiKey, 'native-key');
  assert.equal(fields.apiFormat, 'openai_responses');
  assert.deepEqual(fields.models?.[0].input, ['text', 'image', 'video']);
  assert.equal(fields.models?.[0].reasoning, true);
});

test('Grok catalogs with different connections are shared as separate model groups', () => {
  const variants = extractProviderShareVariants('grok', source({
    auth: { API_KEY: 'shared-key' }, defaultModelKey: 'b', modelCatalog: { models: [
      { key: 'a', model: 'm-a', apiBackend: 'chat_completions', baseUrl: 'https://a.test/v1' },
      { key: 'b', model: 'm-b', apiBackend: 'responses', baseUrl: 'https://b.test/v1', apiKey: 'b-key' },
      { key: 'c', model: 'm-c', apiBackend: 'chat_completions', baseUrl: 'https://a.test/v1' },
    ] },
  }));
  assert.equal(variants.length, 2);
  assert.equal(variants[0].id, 'b');
  assert.equal(variants[0].fields.apiKey, 'b-key');
  assert.equal(variants[0].fields.apiFormat, 'openai_responses');
  assert.deepEqual(variants[0].fields.models?.map((model) => model.id), ['m-b']);
  assert.deepEqual(variants[1].fields.models?.map((model) => model.id), ['m-a', 'm-c']);
});

test('Kimi groups models by the selected provider configuration', () => {
  const variants = extractProviderShareVariants('kimi', source({
    providerConfigs: { a: { type: 'anthropic', api_key: 'a-key', base_url: 'https://a.test' }, b: { type: 'gemini', api_key: 'b-key', base_url: 'https://b.test' } },
    defaultModelKey: 'a', modelCatalog: { models: [
      { key: 'a', provider: 'a', model: 'm-a' }, { key: 'b', provider: 'b', model: 'm-b' },
    ] },
  }));
  assert.equal(variants.length, 2);
  assert.deepEqual(variants.map((variant) => variant.fields.apiKey), ['a-key', 'b-key']);
  assert.deepEqual(variants.map((variant) => variant.fields.apiFormat), ['anthropic_messages', 'gemini_native']);
});

test('native model-level URL and SDK overrides form coherent share connections', () => {
  const variants = extractProviderShareVariants('opencode', source({
    npm: '@ai-sdk/openai-compatible', options: { baseURL: 'https://a.test/v1', apiKey: 'key' },
    models: { chat: { id: 'vendor/chat' }, vision: { id: 'vendor/vision', provider: { npm: '@ai-sdk/google', api: 'https://b.test/v1beta' } } },
  }, { defaultModel: 'vision' }));
  assert.equal(variants.length, 2);
  assert.equal(variants[0].fields.baseUrl, 'https://b.test/v1beta');
  assert.equal(variants[0].fields.apiFormat, 'gemini_native');
  assert.equal(variants[0].fields.model, 'vendor/vision');
  assert.equal(variants[1].fields.baseUrl, 'https://a.test/v1');
  assert.deepEqual(variants[1].fields.models?.map((model) => model.id), ['vendor/chat']);

  const piVariants = extractProviderShareVariants('pi', source({
    api: 'openai-completions', apiKey: 'key', baseUrl: 'https://a.test/v1',
    models: [{ id: 'chat' }, { id: 'messages', api: 'anthropic-messages', baseUrl: 'https://b.test' }],
  }));
  assert.equal(piVariants.length, 2);
  assert.equal(piVariants[1].fields.apiFormat, 'anthropic_messages');
  assert.equal(piVariants[1].fields.baseUrl, 'https://b.test');
});

test('cross-protocol auth follows the actual upstream instead of the env variable name', () => {
  const claude = extractProviderShareFields('claude', source({ env: { ANTHROPIC_API_KEY: 'key' } }, { meta: { apiFormat: 'openai_chat' } }));
  assert.equal(claude.apiKeyField, 'bearer');
  const gemini = extractProviderShareFields('gemini', source({ env: { GEMINI_API_KEY: 'key' } }, { meta: { apiFormat: 'anthropic_messages' } }));
  assert.equal(gemini.apiKeyField, 'x-api-key');
  const profile = extractProviderShareFields('claude', source({ env: { ANTHROPIC_API_KEY: 'key' } }, {
    meta: { gatewayProfile: { tool: 'claude', profileId: 'relay', endpointId: 'chat' }, apiFormat: 'anthropic_messages' },
    connectionDefaults: { apiFormat: 'openai_chat' },
  }));
  assert.equal(profile.apiFormat, 'openai_chat');
  assert.equal(profile.apiKeyField, 'bearer');
});

test('OAuth credentials and unresolved environment references are never exported as keys', () => {
  const oauth = source({ api: 'openai-responses' }, { credential: { type: 'oauth', access: 'oauth-access', refresh: 'oauth-refresh' } });
  assert.equal(extractProviderShareFields('pi', oauth).apiKey, undefined);
  for (const key of ['{env:API_KEY}', '${API_KEY}', '!read-secret', 'env:API_KEY']) {
    const provider = source({ npm: '@ai-sdk/openai', options: { apiKey: key } });
    assert.equal(extractProviderShareFields('opencode', provider).apiKey, undefined);
    assert.equal(hasUnresolvedShareCredential(provider), true);
    assert.ok(!buildProviderShareUrl({ ...extractProviderShareFields('opencode', provider), app: 'codex', name: 'Ref' }).includes('apiKey='));
  }
});

test('adopted Codex and Kimi OAuth configs still require a portable API key', () => {
  const codex = source({ auth: { tokens: { access_token: 'test-access', refresh_token: 'test-refresh' } } });
  assert.equal(hasUnresolvedShareCredential(codex), true);
  assert.equal(extractProviderShareFields('codex', codex).apiKey, undefined);
  const kimiOauth = source({ providerConfigs: { main: { type: 'kimi', oauth: { storage: 'file', key: 'local-account' } } } });
  assert.equal(hasUnresolvedShareCredential(kimiOauth), true);
  assert.equal(extractProviderShareFields('kimi', kimiOauth).apiKey, undefined);
  const kimiEnv = source({ providerConfigs: { main: { type: 'openai_legacy', api_key: 'env:LOCAL_KEY' } } });
  assert.equal(hasUnresolvedShareCredential(kimiEnv), true);
  assert.equal(extractProviderShareFields('kimi', kimiEnv).apiKey, undefined);
});

test('runtime built-in defaults fill missing fields and never replace explicit user settings', () => {
  const defaults = { apiFormat: 'openai_responses' as const, baseUrl: 'https://official.test/v1', apiKey: 'official-key', models: [{ id: 'official-model' }] };
  const implicit = extractProviderShareFields('opencode', source({}, { connectionDefaults: defaults }));
  assert.equal(implicit.baseUrl, defaults.baseUrl);
  assert.equal(implicit.apiKey, defaults.apiKey);
  assert.equal(implicit.model, 'official-model');
  const explicit = extractProviderShareFields('opencode', source({
    npm: '@ai-sdk/anthropic', options: { baseURL: 'https://custom.test/v1', apiKey: 'custom-key' }, models: { custom: {} },
  }, { connectionDefaults: defaults }));
  assert.equal(explicit.apiFormat, 'anthropic_messages');
  assert.equal(explicit.baseUrl, 'https://custom.test/v1');
  assert.equal(explicit.apiKey, 'custom-key');
  assert.deepEqual(explicit.models?.map((model) => model.id), ['custom']);
});

test('profile references, custom header rules and special characters round-trip through URLs', () => {
  const fields = extractProviderShareFields('claude', source({ env: { ANTHROPIC_AUTH_TOKEN: 'key +&=?/中文', ANTHROPIC_MODEL: 'vendor/a+b' } }, { meta: {
    gatewayProfile: { tool: 'claude', profileId: 'relay', endpointId: 'messages' },
    customHeaders: [{ op: 'copy', from: 'X-One', to: 'X-Two' }],
  } }));
  const params = new URL(buildProviderShareUrl({ ...fields, app: 'codex', name: '测试 & Team' })).searchParams;
  assert.equal(params.get('name'), '测试 & Team');
  assert.equal(params.get('apiKey'), 'key +&=?/中文');
  assert.equal(params.get('model'), 'vendor/a+b');
  assert.deepEqual(JSON.parse(params.get('gatewayProfile')!), fields.gatewayProfile);
  assert.deepEqual(JSON.parse(params.get('headers')!), fields.headers);
  assert.equal(params.has('providerType'), false);
});
