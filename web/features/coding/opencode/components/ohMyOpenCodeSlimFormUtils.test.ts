import test from 'node:test';
import assert from 'node:assert/strict';

import { buildSlimAgentsFromFormValues } from './ohMyOpenCodeSlimFormUtils.ts';

test('buildSlimAgentsFromFormValues preserves unmanaged agent fields while updating managed ones', () => {
  const result = buildSlimAgentsFromFormValues({
    builtInAgentKeys: ['orchestrator'],
    customAgents: ['reviewer'],
    formValues: {
      agent_orchestrator_model: 'gpt-5.4',
      agent_orchestrator_variant: 'fast',
      agent_reviewer_model: 'gpt-5.4-mini',
    },
    initialAgents: {
      orchestrator: {
        model: 'old-model',
        variant: 'old-variant',
        skills: ['plan', 'delegate'],
        temperature: 0.2,
      },
      reviewer: {
        skills: ['lint'],
      },
    },
  });

  assert.deepEqual(result, {
    orchestrator: {
      skills: ['plan', 'delegate'],
      temperature: 0.2,
      model: 'gpt-5.4',
      variant: 'fast',
    },
    reviewer: {
      skills: ['lint'],
      model: 'gpt-5.4-mini',
    },
  });
});

test('buildSlimAgentsFromFormValues omits agent when managed and unmanaged fields are both empty', () => {
  const result = buildSlimAgentsFromFormValues({
    builtInAgentKeys: ['orchestrator'],
    customAgents: [],
    formValues: {},
    initialAgents: {
      orchestrator: {
        model: 'old-model',
        variant: 'old-variant',
      },
    },
  });

  assert.deepEqual(result, {});
});

test('buildSlimAgentsFromFormValues writes normalized fallback_models for managed agent fields', () => {
  const result = buildSlimAgentsFromFormValues({
    builtInAgentKeys: ['oracle'],
    customAgents: [],
    formValues: {
      agent_oracle_model: 'gpt-5.4',
      agent_oracle_fallback_models: [' gpt-5.4-mini ', '', 'gpt-4.1'],
    },
    initialAgents: {
      oracle: {
        model: 'old-oracle',
        fallback_models: ['legacy-model'],
        temperature: 0.3,
      },
    },
  });

  assert.deepEqual(result, {
    oracle: {
      temperature: 0.3,
      model: 'gpt-5.4',
      fallback_models: ['gpt-5.4-mini', 'gpt-4.1'],
    },
  });
});

test('buildSlimAgentsFromFormValues removes legacy fallback_models when user clears managed fallback field', () => {
  const result = buildSlimAgentsFromFormValues({
    builtInAgentKeys: ['oracle'],
    customAgents: [],
    formValues: {
      agent_oracle_model: 'gpt-5.4',
      agent_oracle_fallback_models: [],
    },
    initialAgents: {
      oracle: {
        model: 'old-oracle',
        fallback_models: ['legacy-model'],
        skills: ['plan'],
      },
    },
  });

  assert.deepEqual(result, {
    oracle: {
      skills: ['plan'],
      model: 'gpt-5.4',
    },
  });
});

test('buildSlimAgentsFromFormValues normalizes string fallback_models for custom agents', () => {
  const result = buildSlimAgentsFromFormValues({
    builtInAgentKeys: [],
    customAgents: ['reviewer'],
    formValues: {
      agent_reviewer_model: 'gpt-5.4-mini',
      agent_reviewer_fallback_models: ' gpt-4.1-mini ',
    },
    initialAgents: {
      reviewer: {
        tools: ['lint'],
      },
    },
  });

  assert.deepEqual(result, {
    reviewer: {
      tools: ['lint'],
      model: 'gpt-5.4-mini',
      fallback_models: ['gpt-4.1-mini'],
    },
  });
});
