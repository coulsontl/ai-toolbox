import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildOmpProviderConfigFromForm,
  ompProviderKeySaveError,
  resolveOmpAutomaticBaseUrlChange,
  resolveOmpProviderFormInitialState,
} from '../../../../../features/coding/oh_my_pi/utils/ompProviderForm.ts';
import type { OmpRuntimeProviderView } from '../../../../../types/ohMyPi.ts';

/**
 * These used to scrape `OhMyPiPage.tsx` for the inline handlers. The provider
 * editor now lives in `OmpProviderFormModal` and the decisions behind it in
 * `utils/ompProviderForm.ts`, so the same behaviour is asserted directly on the
 * pure functions — a source-scrape would have to be rewritten for every layout
 * change and would still assert nothing about the values it never sets.
 */

const emptyJsonBlocks = { headers: {}, compat: {}, modelOverrides: {} };

test('a new provider starts with the form owning the endpoint', () => {
  const state = resolveOmpProviderFormInitialState();
  assert.equal(state.automaticBaseUrl, '');
  assert.equal(state.values.providerKey, '');
  assert.equal(state.values.api, 'openai-completions');
});

test('switching API in a new provider saves the matching automatic endpoint', () => {
  let automaticBaseUrl = resolveOmpProviderFormInitialState().automaticBaseUrl;
  let baseUrl: string | undefined;

  const change = resolveOmpAutomaticBaseUrlChange('anthropic-messages', baseUrl, automaticBaseUrl);
  automaticBaseUrl = change.automaticBaseUrl;
  baseUrl = change.baseUrl;

  const provider = buildOmpProviderConfigFromForm(
    { api: 'openai-completions', baseUrl: '', models: [] },
    { providerKey: 'example', api: 'anthropic-messages', baseUrl },
    emptyJsonBlocks,
  );
  assert.equal(provider.api, 'anthropic-messages');
  assert.equal(provider.baseUrl, 'https://api.anthropic.com');
});

test('an edit keeps the stored endpoint when the protocol changes', () => {
  const provider = {
    providerKey: 'example',
    displayName: 'Example',
    sources: ['models_yml'],
    categories: [],
    credentialKind: 'api_key',
    modelsProvider: { api: 'anthropic-messages', baseUrl: 'https://api.anthropic.com' },
    runtimeFiles: [],
    isBuiltin: false,
    isOverride: false,
    isDefault: false,
  } as unknown as OmpRuntimeProviderView;

  const state = resolveOmpProviderFormInitialState(provider);
  assert.equal(state.automaticBaseUrl, undefined);
  const change = resolveOmpAutomaticBaseUrlChange(
    'openai-responses',
    state.values.baseUrl,
    state.automaticBaseUrl,
  );
  assert.equal(change.baseUrl, undefined);
});

test('manual endpoints and explicit clearing survive later API changes', () => {
  for (const baseUrl of ['https://custom.example/v1', '']) {
    const change = resolveOmpAutomaticBaseUrlChange('anthropic-messages', baseUrl, undefined);
    // `undefined` means "the user owns it": no replacement at all.
    assert.equal(change.baseUrl, undefined);
    assert.equal(change.automaticBaseUrl, undefined);
  }
});

test('copy does not inherit the source endpoint ownership', () => {
  const provider = {
    providerKey: 'existing',
    displayName: 'Existing',
    sources: ['models_yml'],
    categories: [],
    credentialKind: 'api_key',
    modelsProvider: { api: 'openai-responses' },
    runtimeFiles: [],
    isBuiltin: false,
    isOverride: false,
    isDefault: false,
  } as unknown as OmpRuntimeProviderView;

  const state = resolveOmpProviderFormInitialState(provider, true);
  assert.equal(state.values.providerKey, 'existing_copy');
  // A copy keeps the source endpoint if it has one but must not rewrite it.
  assert.equal(state.automaticBaseUrl, undefined);
});

test('a fresh dialog resets autofill ownership', () => {
  const state = resolveOmpProviderFormInitialState();
  assert.equal(state.automaticBaseUrl, '');
  const change = resolveOmpAutomaticBaseUrlChange('anthropic-messages', '', state.automaticBaseUrl);
  assert.equal(change.baseUrl, 'https://api.anthropic.com');
});

test('subscription cannot be added through the API-key provider form', () => {
  assert.equal(ompProviderKeySaveError('openai-codex'), 'subscriptionUseLoginGuidance');
  assert.equal(ompProviderKeySaveError('example'), null);
});

test('existing Codex YAML override remains editable without converting OAuth to an API key', () => {
  const provider = {
    providerKey: 'openai-codex',
    displayName: 'OpenAI Codex',
    sources: ['models_yml'],
    categories: [],
    credentialKind: 'oauth',
    modelsProvider: { api: 'openai-codex-responses', models: [{ id: 'gpt-5-codex' }] },
    runtimeFiles: [],
    isBuiltin: true,
    isOverride: true,
    isDefault: false,
  } as unknown as OmpRuntimeProviderView;

  assert.equal(ompProviderKeySaveError('openai-codex', provider), null);
  const state = resolveOmpProviderFormInitialState(provider);
  const saved = buildOmpProviderConfigFromForm(
    provider.modelsProvider ?? {},
    state.values,
    state.jsonBlocks,
  );
  assert.deepEqual(saved.models, provider.modelsProvider?.models);
  assert.equal(saved.apiKey, undefined);
});
