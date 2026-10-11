import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

import { getOmpApiBaseUrlUpdate } from '../../../../../features/coding/oh_my_pi/utils/ompApiOptions.ts';

// Execute the actual page handlers, replacing only React state and the IO boundary.
function createFormHarness() {
  const file = new URL('../../../../../features/coding/oh_my_pi/pages/OhMyPiPage.tsx', import.meta.url);
  const source = ts.createSourceFile(file.pathname, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const names = new Set(['asRecord', 'getStringField', 'createDefaultProviderConfig', 'setOptionalStringField', 'isRecordEmpty', 'hasProviderConfigContent', 'openProviderModal', 'handleProviderApiChange', 'handleProviderBaseUrlChange', 'handleSaveProviderModal']);
  const declarations: string[] = [];
  function visit(node: ts.Node) {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && names.has(node.name.text)) {
      declarations.push(`const ${node.name.text} = ${node.initializer!.getText(source)};`);
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.equal(declarations.length, names.size);
  const fields: Record<string, unknown> = {};
  let saved: { providerKey: string; provider: Record<string, unknown> } | undefined;
  const noop = () => {};
  const context: Record<string, any> = {
    getOmpApiBaseUrlUpdate, automaticProviderBaseUrlRef: { current: undefined },
    providerModalForm: {
      setFieldsValue: (values: Record<string, unknown>) => Object.assign(fields, values),
      getFieldValue: (key: string) => fields[key],
      setFieldValue: (key: string, value: unknown) => { fields[key] = value; },
      validateFields: async () => ({ ...fields }),
    },
    setProviderAdvancedExpanded: noop, setSaving: noop, setRuntimeConfig: noop, setOtherSettings: noop,
    saveOmpModelsProvider: async (value: typeof saved) => { saved = JSON.parse(JSON.stringify(value)); return { otherSettings: {} }; },
    upsertOmpFavoriteProvider: async () => {}, refreshTrayMenu: async () => {},
    message: { success: noop, error: (error: unknown) => { throw new Error(String(error)); } }, t: (key: string) => key, console,
  };
  for (const state of ['providerModal', 'providerConfigJson', 'providerHeadersJson', 'providerCompatJson', 'providerModelOverridesJson', 'providerConfigJsonValid', 'providerHeadersJsonValid', 'providerCompatJsonValid', 'providerModelOverridesJsonValid']) {
    context[`set${state[0].toUpperCase()}${state.slice(1)}`] = (value: unknown) => { context[state] = value; };
  }
  const compiled = ts.transpileModule(`${declarations.join('\n')}\n({ openProviderModal, handleProviderApiChange, handleProviderBaseUrlChange, handleSaveProviderModal });`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const handlers = vm.runInNewContext(compiled, context);
  return { fields, handlers, saved: () => saved! };
}

test('switching API in a new provider saves the matching automatic endpoint and edit preserves it', async () => {
  const form = createFormHarness();
  form.handlers.openProviderModal();
  form.fields.providerKey = 'example';
  for (const api of ['openai-responses', 'anthropic-messages']) {
    form.fields.api = api;
    form.handlers.handleProviderApiChange(api);
  }
  await form.handlers.handleSaveProviderModal();
  assert.equal(form.saved().provider.api, 'anthropic-messages');
  assert.equal(form.saved().provider.baseUrl, 'https://api.anthropic.com');
  form.handlers.openProviderModal({ providerKey: 'example', modelsProvider: form.saved().provider, sources: ['models_yml'] });
  form.fields.api = 'openai-responses';
  form.handlers.handleProviderApiChange(form.fields.api);
  assert.equal(form.fields.baseUrl, 'https://api.anthropic.com');
});

test('manual endpoints and explicit clearing survive later API changes', async () => {
  for (const baseUrl of ['https://custom.example/v1', '']) {
    const form = createFormHarness();
    form.handlers.openProviderModal();
    form.fields.providerKey = 'example';
    form.handlers.handleProviderApiChange('openai-responses');
    form.fields.baseUrl = baseUrl;
    form.handlers.handleProviderBaseUrlChange();
    form.fields.api = 'anthropic-messages';
    form.handlers.handleProviderApiChange(form.fields.api);
    await form.handlers.handleSaveProviderModal();
    assert.equal(form.saved().provider.baseUrl, baseUrl || undefined);
  }
});

test('copy does not autofill and a new dialog resets autofill ownership', () => {
  const form = createFormHarness();
  form.handlers.openProviderModal({ providerKey: 'existing', modelsProvider: { api: 'openai-responses' } }, { copy: true });
  form.handlers.handleProviderApiChange('anthropic-messages');
  assert.ok(!form.fields.baseUrl);
  form.handlers.openProviderModal();
  form.handlers.handleProviderApiChange('anthropic-messages');
  assert.equal(form.fields.baseUrl, 'https://api.anthropic.com');
  form.handlers.handleProviderApiChange('azure-openai-responses');
  assert.equal(form.fields.baseUrl, '');
});

test('subscription cannot be added through the API-key provider form', async () => {
  const form = createFormHarness();
  form.handlers.openProviderModal();
  form.fields.providerKey = 'openai-codex';
  await assert.rejects(form.handlers.handleSaveProviderModal(), /ohMyPi.subscription.useLoginGuidance/);
  assert.equal(form.saved(), undefined);
});

test('existing Codex YAML override remains editable without converting OAuth to an API key', async () => {
  const form = createFormHarness();
  const provider = { api: 'openai-codex-responses', models: [{ id: 'gpt-5-codex' }] };
  form.handlers.openProviderModal({ providerKey: 'openai-codex', modelsProvider: provider, sources: ['models_yml'] });
  await form.handlers.handleSaveProviderModal();
  assert.deepEqual(form.saved().provider.models, provider.models);
  assert.equal(form.saved().provider.apiKey, undefined);
});
