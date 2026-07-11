/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  OPEN_CODE_BUILT_IN_SUBAGENTS,
  clearInvalidOpenCodeDefaultAgent,
  getConfiguredOpenCodeAgentModelIds,
  getOpenCodeDefaultAgentCandidates,
  isOpenCodeBuiltInAgentName,
  removeOpenCodeAgentOverride,
  sanitizeOpenCodeAgentModelReferences,
  setOpenCodeAgentAdvancedConfig,
  setOpenCodeAgentModel,
  setOpenCodeAgentPrompt,
  setOpenCodeAgentVariant,
  validateOpenCodeAgentConfig,
} from '../../../../../features/coding/opencode/utils/openCodeAgentConfig.ts';

test('built-in subagents stay aligned with current OpenCode agents', () => {
  assert.deepEqual(OPEN_CODE_BUILT_IN_SUBAGENTS, ['general', 'explore', 'scout']);
});

test('all built-in agent names are reserved for overrides', () => {
  for (const agentName of [
    'build', 'plan', 'general', 'explore', 'scout', 'title', 'summary', 'compaction',
  ]) {
    assert.equal(isOpenCodeBuiltInAgentName(agentName), true);
  }
  assert.equal(isOpenCodeBuiltInAgentName('code-reviewer'), false);
});
import type { OpenCodeConfig } from '../../../../../types/opencode.ts';

const createConfig = (): OpenCodeConfig => ({
  provider: {},
  agent: {
    explore: {
      model: 'anthropic/claude-sonnet-5',
      variant: 'high',
      permission: { edit: 'deny' },
      futureField: { enabled: true },
    },
    'spec-verify': {
      mode: 'subagent',
      model: 'zenmux/openai/gpt-5.5',
      variant: 'medium',
    },
  },
});

test('setOpenCodeAgentModel preserves unknown fields and clears incompatible variant', () => {
  const result = setOpenCodeAgentModel(
    createConfig(),
    'explore',
    'anthropic/claude-opus-4-6',
    ['low', 'medium'],
  );

  assert.deepEqual(result.agent?.explore, {
    model: 'anthropic/claude-opus-4-6',
    permission: { edit: 'deny' },
    futureField: { enabled: true },
  });
});

test('setOpenCodeAgentModel keeps compatible variant and accepts model IDs containing slashes', () => {
  const result = setOpenCodeAgentModel(
    createConfig(),
    'spec-verify',
    'zenmux/openai/gpt-5.5',
    ['low', 'medium', 'high'],
  );

  assert.equal(result.agent?.['spec-verify']?.model, 'zenmux/openai/gpt-5.5');
  assert.equal(result.agent?.['spec-verify']?.variant, 'medium');
});

test('clearing an agent model also clears variant but preserves permissions', () => {
  const result = setOpenCodeAgentModel(createConfig(), 'explore', undefined);

  assert.deepEqual(result.agent?.explore, {
    permission: { edit: 'deny' },
    futureField: { enabled: true },
  });
});

test('clearing the only built-in agent overrides restores the OpenCode default', () => {
  const result = setOpenCodeAgentModel({
    provider: {},
    agent: {
      build: {
        model: 'anthropic/claude-sonnet-5',
        variant: 'high',
      },
    },
  }, 'build', undefined);

  assert.equal(result.agent, undefined);
});

test('setOpenCodeAgentVariant only writes variant for an explicit agent model', () => {
  const config: OpenCodeConfig = { provider: {}, agent: { general: {} } };
  const withoutModel = setOpenCodeAgentVariant(config, 'general', 'high');
  assert.equal(withoutModel.agent?.general?.variant, undefined);

  const withModel = setOpenCodeAgentVariant(createConfig(), 'explore', 'xhigh');
  assert.equal(withModel.agent?.explore?.variant, 'xhigh');
});

test('setOpenCodeAgentPrompt preserves other fields and clears empty built-in overrides', () => {
  const withPrompt = setOpenCodeAgentPrompt(createConfig(), 'explore', '# Explore\n\nRead only.');
  assert.equal(withPrompt.agent?.explore?.prompt, '# Explore\n\nRead only.');
  assert.equal(withPrompt.agent?.explore?.model, 'anthropic/claude-sonnet-5');
  assert.deepEqual(withPrompt.agent?.explore?.permission, { edit: 'deny' });

  const cleared = setOpenCodeAgentPrompt({
    provider: {},
    agent: { build: { prompt: 'Custom build instructions' } },
  }, 'build', '   ');
  assert.equal(cleared.agent, undefined);
});

test('clearing a custom agent prompt preserves the custom agent', () => {
  const result = setOpenCodeAgentPrompt({
    provider: {},
    agent: {
      reviewer: {
        description: 'Reviews code',
        mode: 'subagent',
        prompt: 'Review carefully',
      },
    },
  }, 'reviewer', undefined);

  assert.deepEqual(result.agent?.reviewer, {
    description: 'Reviews code',
    mode: 'subagent',
  });
});

test('sanitizeOpenCodeAgentModelReferences removes only model and variant', () => {
  const result = sanitizeOpenCodeAgentModelReferences(
    createConfig(),
    new Set(['anthropic/claude-sonnet-5', 'zenmux/openai/gpt-5.5']),
  );

  assert.deepEqual(result.agent?.explore, {
    permission: { edit: 'deny' },
    futureField: { enabled: true },
  });
  assert.deepEqual(result.agent?.['spec-verify'], { mode: 'subagent' });
});

test('custom agents can be added, replaced, and removed without changing other agents', () => {
  const added = setOpenCodeAgentAdvancedConfig(createConfig(), 'code-reviewer', {
    description: 'Review code',
    mode: 'subagent',
  });
  assert.equal(added.agent?.['code-reviewer']?.description, 'Review code');
  assert.ok(added.agent?.explore);

  const removed = removeOpenCodeAgentOverride(added, 'code-reviewer');
  assert.equal(removed.agent?.['code-reviewer'], undefined);
  assert.ok(removed.agent?.explore);
});

test('saving an empty built-in advanced config restores defaults while custom agents remain valid', () => {
  const builtInResult = setOpenCodeAgentAdvancedConfig(createConfig(), 'explore', {});
  assert.equal(builtInResult.agent?.explore, undefined);

  const customResult = setOpenCodeAgentAdvancedConfig({ provider: {} }, 'empty-helper', {});
  assert.deepEqual(customResult.agent?.['empty-helper'], {});
});

test('default agent candidates exclude subagents, hidden agents, and disabled agents', () => {
  const result = getOpenCodeDefaultAgentCandidates({
    provider: {},
    agent: {
      deploy: { mode: 'primary' },
      review: { mode: 'all' },
      helper: { mode: 'subagent' },
      hidden: { mode: 'primary', hidden: true },
      disabled: { mode: 'primary', disable: true },
    },
  });

  assert.deepEqual(result, ['build', 'plan', 'deploy', 'review']);
});

test('clearInvalidOpenCodeDefaultAgent removes invalid defaults after advanced edits', () => {
  for (const invalidConfig of [
    { mode: 'subagent' as const },
    { mode: 'primary' as const, hidden: true },
    { mode: 'primary' as const, disable: true },
  ]) {
    const result = clearInvalidOpenCodeDefaultAgent({
      provider: {},
      default_agent: 'reviewer',
      agent: { reviewer: invalidConfig },
    });
    assert.equal(result.default_agent, undefined);
  }

  const valid = clearInvalidOpenCodeDefaultAgent({
    provider: {},
    default_agent: 'reviewer',
    agent: { reviewer: { mode: 'primary' } },
  });
  assert.equal(valid.default_agent, 'reviewer');
});

test('configured agent model IDs include every explicit model override', () => {
  assert.deepEqual(
    getConfiguredOpenCodeAgentModelIds(createConfig()),
    ['anthropic/claude-sonnet-5', 'zenmux/openai/gpt-5.5'],
  );
});

test('validateOpenCodeAgentConfig accepts unknown fields and checks managed field shapes', () => {
  assert.equal(validateOpenCodeAgentConfig({ future: true }), undefined);
  assert.equal(validateOpenCodeAgentConfig([]), 'object');
  assert.equal(validateOpenCodeAgentConfig({ model: 123 }), 'model');
  assert.equal(validateOpenCodeAgentConfig({ variant: false }), 'variant');
  assert.equal(validateOpenCodeAgentConfig({ prompt: { content: 'invalid' } }), 'prompt');
  assert.equal(validateOpenCodeAgentConfig({}, { requireDescription: true }), 'description');
  assert.equal(validateOpenCodeAgentConfig({ description: '   ' }, { requireDescription: true }), 'description');
  assert.equal(validateOpenCodeAgentConfig({ description: 'Reviews code' }, { requireDescription: true }), undefined);
  assert.equal(validateOpenCodeAgentConfig({ mode: 'worker' }), 'mode');
  assert.equal(validateOpenCodeAgentConfig({ steps: 0 }), 'steps');
});
