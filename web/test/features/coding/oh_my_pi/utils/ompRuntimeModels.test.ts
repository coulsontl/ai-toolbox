import assert from 'node:assert/strict';
import test from 'node:test';
import type { OmpRuntimeProviderView } from '../../../../../types/ohMyPi.ts';
import { getOmpRuntimeModelIds, getOmpRuntimeModelRecords } from '../../../../../features/coding/oh_my_pi/utils/ompRuntimeModels.ts';
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

