/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

// Execute the actual fetch handler and picker JSX with synthetic dependencies.
// This is a bounded integration contract test, not browser/keyboard acceptance.
const source = readFileSync(new URL(
  '../../../../../features/coding/codex/components/CodexProviderFormModal.tsx', import.meta.url,
), 'utf8');
const start = source.indexOf('  const handleFetchModels = async () => {');
const end = source.indexOf('\n  // 根据 baseUrl', start);
assert.ok(start >= 0 && end > start);
function compile(code: string) {
  return ts.transpileModule(code, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
}
const handler = compile(`${source.slice(start, end)}\nhandleFetchModels;`);
const models = Array.from({ length: 8 }, (_, i) => ({ id: `model-${i}`, name: `Model ${i}` }));

function harness(official = true) {
  const state = {
    models: [] as typeof models, loading: false, search: 'old search', status: 'idle',
    sequence: { current: 0 }, notifications: [] as string[],
  };
  const requests: Array<{ resolve: (value: unknown) => void; reject: (error: Error) => void }> = [];
  const request = () => new Promise((resolve, reject) => requests.push({ resolve, reject }));
  const notify = (value: string) => state.notifications.push(value);
  const fetch = runInNewContext(handler, {
    isOfficialMode: official,
    modelFetchSequence: state.sequence,
    setFetchedModels: (value: typeof models) => { state.models = value; },
    setLoadingModels: (value: boolean) => { state.loading = value; },
    setModelPickerSearch: (value: string) => { state.search = value; },
    setModelFetchStatus: (value: string) => { state.status = value; },
    fetchCodexOfficialModels: request,
    invoke: request,
    CODEX_OFFICIAL_FALLBACK_MODELS: models,
    form: { getFieldValue: (key: string) => key === 'baseUrl' ? 'https://synthetic.invalid' : '' },
    modelsSdkType: '@ai-sdk/openai',
    getDefaultModelsApiType: () => 'openai',
    t: (key: string) => key,
    console: { error() {} },
    message: { success: notify, info: notify, warning: notify, error: notify },
  }) as () => Promise<void>;
  return { state, requests, fetch };
}

test('official fetch exposes all eight models without writing any form field', async () => {
  const h = harness();
  const pending = h.fetch();
  assert.equal(h.state.loading, true);
  assert.equal(h.state.search, '');
  h.requests[0].resolve({ models, source: 'remote' });
  await pending;
  assert.equal(h.state.models.length, 8);
  assert.equal(h.state.loading, false);
  assert.deepEqual(h.state.notifications, ['codex.fetchModels.officialUpdated']);
});

test('newer fetch wins and old finally cannot stop its loading indicator', async () => {
  const h = harness();
  const old = h.fetch();
  const latest = h.fetch();
  h.requests[0].resolve({ models: [models[0]], source: 'remote' });
  await old;
  assert.equal(h.state.loading, true);
  assert.equal(h.state.models.length, 0);
  assert.equal(h.state.notifications.length, 0);
  h.requests[1].resolve({ models, source: 'remote' });
  await latest;
  assert.equal(h.state.models.length, 8);
  assert.equal(h.state.loading, false);
});

test('cancel/session invalidation suppresses both late success and fallback error', async () => {
  for (const fail of [false, true]) {
    const h = harness();
    const pending = h.fetch();
    h.state.sequence.current += 1;
    if (fail) h.requests[0].reject(new Error('synthetic failure'));
    else h.requests[0].resolve({ models, source: 'remote' });
    await pending;
    assert.equal(h.state.models.length, 0);
    assert.equal(h.state.notifications.length, 0);
  }
  assert.match(source, /onCancel=\{\(\) => \{\s*modelFetchSequence.current \+= 1;/);
  assert.match(source, /return \(\) => \{ modelFetchSequence.current \+= 1; \};/);
  assert.match(source, /\[open, provider, mode, isCopy, isOfficialMode, codexBaseUrl, codexApiKey, modelsSdkType\]/);
});

test('session effect clears picker state and invalidates requests on setup and cleanup', () => {
  const effectStart = source.indexOf('  React.useEffect(() => {\n    modelFetchSequence.current += 1;');
  const effectEnd = source.indexOf('\n\n  const selectedProviderProfileId', effectStart);
  assert.ok(effectStart >= 0 && effectEnd > effectStart);
  const state = { sequence: { current: 4 }, models, loading: true, search: 'old', status: 'error' };
  let cleanup: (() => void) | undefined;
  runInNewContext(compile(source.slice(effectStart, effectEnd)), {
    React: { useEffect: (effect: () => () => void) => { cleanup = effect(); } },
    modelFetchSequence: state.sequence,
    setFetchedModels: (value: typeof models) => { state.models = value; },
    setLoadingModels: (value: boolean) => { state.loading = value; },
    setModelPickerSearch: (value: string) => { state.search = value; },
    setModelFetchStatus: (value: string) => { state.status = value; },
    open: true, provider: {}, mode: 'manual', isCopy: false, isOfficialMode: true,
    codexBaseUrl: '', codexApiKey: '', modelsSdkType: '@ai-sdk/openai',
  });
  assert.equal(state.sequence.current, 5);
  assert.equal(state.models.length, 0);
  assert.equal(state.loading, false);
  assert.equal(state.search, '');
  assert.equal(state.status, 'idle');
  cleanup!();
  assert.equal(state.sequence.current, 6);
});

test('official empty response or failure retains the bundled fallback', async () => {
  for (const fail of [false, true]) {
    const h = harness();
    const pending = h.fetch();
    if (fail) h.requests[0].reject(new Error('synthetic failure'));
    else h.requests[0].resolve({ models: [], source: 'bundled' });
    await pending;
    assert.equal(h.state.models.length, 8);
    assert.equal(h.state.loading, false);
    assert.deepEqual(h.state.notifications, ['codex.fetchModels.officialBundled']);
  }
});

test('custom empty/error states are explicit and failed refresh retains previous results', async () => {
  const h = harness(false);
  const empty = h.fetch();
  h.requests[0].resolve({ models: [] });
  await empty;
  assert.equal(h.state.status, 'empty');
  h.state.models = models;
  const failed = h.fetch();
  h.requests[1].reject(new Error('synthetic failure'));
  await failed;
  assert.equal(h.state.status, 'error');
  assert.equal(h.state.models, models);
  assert.equal(h.state.loading, false);
});

test('picker is independent of current input, searchable, labeled, and updates the TOML hook only on selection', () => {
  const pickerStart = source.indexOf('            <Select\n              className={styles.modelPicker}');
  const pickerEnd = source.indexOf('\n            />', pickerStart);
  assert.ok(pickerStart >= 0 && pickerEnd > pickerStart);
  let currentModel = 'gpt-6.1-sol';
  let hookModel = currentModel;
  let search = '';
  const picker = runInNewContext(compile(`(${source.slice(pickerStart, pickerEnd + 15)});`), {
    React: { createElement: (_type: unknown, props: unknown) => props },
    Select: 'Select', styles: { modelPicker: 'modelPicker' },
    fetchedModels: models, loadingModels: false, modelPickerSearch: '',
    t: (key: string) => key,
    form: { setFieldValue: (key: string, value: string) => { assert.equal(key, 'model'); currentModel = value; } },
    handleModelChange: (value: string) => { hookModel = value; },
    setModelPickerSearch: (value: string) => { search = value; },
  });
  assert.equal(currentModel, 'gpt-6.1-sol');
  assert.equal(picker.value, undefined);
  assert.equal(picker.options.length, 8);
  assert.equal(picker.options[7].label, 'Model 7 (model-7)');
  assert.equal(picker.options[7].title, 'Model 7 (model-7)');
  assert.equal(picker.showSearch, true);
  assert.equal(picker.optionFilterProp, 'label');
  assert.equal(picker['aria-label'], 'codex.fetchModels.choose');
  assert.equal(picker.notFoundContent, 'codex.fetchModels.noMatches');
  picker.onSearch('missing');
  assert.equal(search, 'missing');
  picker.onOpenChange(false);
  assert.equal(search, '');
  assert.equal(currentModel, 'gpt-6.1-sol');
  picker.onChange('model-7');
  assert.equal(currentModel, 'model-7');
  assert.equal(hookModel, 'model-7');
});
