import assert from 'node:assert/strict';
import test from 'node:test';
import type { OmpRuntimeConfig, OmpRuntimeProviderView } from '../../../../../types/ohMyPi.ts';
import { getOmpRuntimeModelIds, getOmpRuntimeModelRecords, mergeOmpRuntimeCatalog } from '../../../../../features/coding/oh_my_pi/utils/ompRuntimeModels.ts';
import { getOmpThinkingOptionsForModel } from '../../../../../features/coding/oh_my_pi/utils/ompAgentsUtils.ts';

const codex: OmpRuntimeProviderView = {
  providerKey: 'openai-codex', displayName: 'OpenAI Codex', sources: ['official_builtin'],
  categories: ['subscription'], credentialKind: 'oauth', runtimeFiles: ['agent.db'],
  isBuiltin: true, isOverride: false, isDefault: false, oauthStatus: 'stored',
  runtimeModels: [{ id: 'gpt-5-codex', name: 'GPT-5 Codex', reasoning: true, thinking: { mode: 'effort', efforts: ['low', 'high', 'xhigh'] } }],
};


test('runtime and YAML models merge for selection without mutating the YAML payload', () => {
  const yaml = { models: [{ id: 'gpt-5-codex', name: 'Custom name', reasoning: false }, { id: 'custom-model' }] };
  const before = JSON.stringify(yaml);
  const provider = { ...codex, modelsProvider: yaml, runtimeModels: [...codex.runtimeModels!, { id: 'native-only' }], modelIds: ['role-only-model'] };
  assert.deepEqual(getOmpRuntimeModelRecords(provider).map(({ id }) => id), ['gpt-5-codex', 'custom-model', 'native-only']);
  assert.equal(getOmpRuntimeModelRecords(provider)[0].model.name, 'Custom name');
  assert.deepEqual(getOmpRuntimeModelIds(provider), ['gpt-5-codex', 'custom-model', 'native-only', 'role-only-model']);
  assert.equal(JSON.stringify(yaml), before);
  assert.equal(getOmpThinkingOptionsForModel('openai-codex/gpt-5-codex', [provider]).supported, false);
});

test('native models work for agent roles with their real thinking options, even without YAML', () => {
  assert.deepEqual(getOmpRuntimeModelIds(codex), ['gpt-5-codex']);
  assert.deepEqual(getOmpThinkingOptionsForModel('openai-codex/gpt-5-codex', [codex]).options?.map(({ value }) => value), ['off', 'low', 'high', 'xhigh', 'auto']);
  assert.deepEqual(getOmpRuntimeModelRecords({ ...codex, runtimeModels: [] }), []);
});


const localConfig = (rootPath = '/runtime/a', modelId = 'native-only'): OmpRuntimeConfig => ({
  rootPathInfo: { path: rootPath, source: 'custom' },
  configPath: `${rootPath}/config.yml`, modelsPath: `${rootPath}/models.yml`,
  mcpPath: `${rootPath}/mcp.json`, promptPath: `${rootPath}/AGENTS.md`,
  settings: { localSetting: 'before-save' }, models: { providers: {} }, otherSettings: {},
  modelSettings: { providerKey: 'openai-codex', modelId }, builtinProviders: [],
  providers: [{ ...codex, runtimeModels: [], modelIds: ['configured'],
    modelsProvider: { models: [{ id: 'configured', name: 'YAML name' }] },
    isDefault: true, warnings: ['missing_model'] }],
});

test('local config stays available while catalog is absent and late catalog uses the latest saved selection', () => {
  const initial = localConfig();
  const pendingView = mergeOmpRuntimeCatalog(initial, null);
  assert.equal(pendingView.settings, initial.settings);
  assert.deepEqual(pendingView.providers[0].modelIds, ['configured']);
  assert.deepEqual(pendingView.providers[0].warnings, ['missing_model']);
  const cached = { ...initial, providers: [{ ...initial.providers[0], runtimeModels: [{ id: 'native-only' }] }] };
  const cachedView = mergeOmpRuntimeCatalog(cached, null);
  assert.deepEqual(cachedView.providers[0].modelIds, ['configured', 'native-only']);
  assert.deepEqual(cachedView.providers[0].warnings, []);

  const saved = { ...initial, settings: { localSetting: 'after-save' },
    modelSettings: { ...initial.modelSettings, modelId: 'configured', thinkingLevel: 'high' } };
  const catalog = { rootPath: initial.rootPathInfo.path, error: null,
    models: [...codex.runtimeModels!, { id: 'configured', name: 'Native name' }, { id: 'native-only' }] };
  const view = mergeOmpRuntimeCatalog(saved, catalog);
  assert.equal(view.settings, saved.settings);
  assert.equal(view.modelSettings, saved.modelSettings);
  assert.equal(view.settings.localSetting, 'after-save');
  assert.deepEqual(view.modelSettings, { providerKey: 'openai-codex', modelId: 'configured', thinkingLevel: 'high' });
  assert.deepEqual(view.providers[0].warnings, []);
  assert.deepEqual(view.providers[0].modelIds, ['configured', 'gpt-5-codex', 'native-only']);
  assert.equal(getOmpRuntimeModelRecords(view.providers[0])[0].model.name, 'YAML name');
  assert.equal(getOmpThinkingOptionsForModel('openai-codex/gpt-5-codex', view.providers).supported, true);

  const switchedDefault = { ...saved, modelSettings: { providerKey: 'openai-codex', modelId: 'native-only' } };
  const afterAnotherSave = mergeOmpRuntimeCatalog(switchedDefault, catalog);
  assert.equal(afterAnotherSave.providers[0].runtimeModels, catalog.models);
  assert.deepEqual(afterAnotherSave.providers[0].warnings, []);
  assert.equal(afterAnotherSave.modelSettings.modelId, 'native-only');
  assert.deepEqual(saved.providers[0].runtimeModels, []);
  assert.deepEqual(saved.providers[0].modelsProvider?.models, [{ id: 'configured', name: 'YAML name' }]);
  assert.equal(saved.providers[0].runtimeCatalogError, undefined);
});

test('catalog scope and errors never replace local data or carry native IDs across roots', () => {
  const initial = localConfig();
  const nextRoot = localConfig('/runtime/b');
  const oldCatalog = { rootPath: initial.rootPathInfo.path, models: [{ id: 'native-only' }], error: null };
  const view = mergeOmpRuntimeCatalog(nextRoot, oldCatalog);
  assert.equal(view.rootPathInfo, nextRoot.rootPathInfo);
  assert.deepEqual(view.providers[0].runtimeModels, []);
  assert.deepEqual(view.providers[0].modelIds, ['configured']);
  assert.equal(view.providers[0].runtimeCatalogError, null);
  assert.deepEqual(view.providers[0].warnings, ['missing_model']);

  const failure = mergeOmpRuntimeCatalog(nextRoot, { rootPath: '/runtime/b', models: [], error: 'Selected root changed' });
  assert.equal(failure.settings, nextRoot.settings);
  assert.equal(failure.models, nextRoot.models);
  assert.equal(failure.providers[0].runtimeCatalogError, 'Selected root changed');
  assert.deepEqual(failure.providers[0].modelIds, ['configured']);
  assert.deepEqual(nextRoot.providers[0].runtimeModels, []);
});

test('Codex missing-model warnings follow the current local default, preserving unrelated warnings', () => {
  const config = localConfig();
  config.providers[0].warnings = ['missing_provider', 'missing_model'];
  config.providers[0].modelIds = ['stale-native'];
  const otherProvider = { ...codex, providerKey: 'custom', runtimeModels: [], modelIds: ['custom-model'] };
  config.providers.push(otherProvider);
  const catalog = { rootPath: config.rootPathInfo.path, models: [{ id: 'native-only' }], error: null };
  assert.deepEqual(mergeOmpRuntimeCatalog(config, catalog).providers[0].warnings, ['missing_provider']);
  const missing = mergeOmpRuntimeCatalog({ ...config, modelSettings: { providerKey: 'openai-codex', modelId: 'stale-native' } }, catalog);
  assert.deepEqual(missing.providers[0].warnings, ['missing_provider', 'missing_model']);
  assert.deepEqual(missing.providers[0].modelIds, ['configured', 'native-only']);
  assert.equal(missing.providers[1], otherProvider);
  const otherDefault = mergeOmpRuntimeCatalog({ ...config, modelSettings: { providerKey: 'custom', modelId: 'custom-model' } }, catalog);
  assert.deepEqual(otherDefault.providers[0].warnings, ['missing_provider']);
});
