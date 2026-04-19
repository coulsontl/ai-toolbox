/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  applyBatchReplaceModel,
  collectBatchReplaceSourceUsage,
} from '../../../../../features/coding/opencode/components/batchReplaceModelUtils.ts';

test('collectBatchReplaceSourceUsage includes fallback models in source candidates', () => {
  const result = collectBatchReplaceSourceUsage({
    values: {
      agent_oracle_model: 'openai/gpt-5.4',
      agent_oracle_variant: 'high',
      agent_oracle_fallback_models: ['qwen3.5', 'gpt-4.1'],
    },
    modelFieldNames: ['agent_oracle_model'],
    getVariantFieldName: (modelFieldName) => modelFieldName.replace('_model', '_variant'),
    getFallbackFieldName: (modelFieldName) => modelFieldName.replace('_model', '_fallback_models'),
  });

  assert.deepEqual(Array.from(result.usedModels).sort(), [
    'gpt-4.1',
    'openai/gpt-5.4',
    'qwen3.5',
  ]);
  assert.deepEqual(Array.from(result.variantsByModel.get('openai/gpt-5.4') ?? []), ['high']);
  assert.equal(result.variantsByModel.has('qwen3.5'), false);
});

test('applyBatchReplaceModel replaces fallback-only matches when no source variant is selected', () => {
  const result = applyBatchReplaceModel({
    values: {
      agent_oracle_model: 'openai/gpt-5.4',
      agent_oracle_variant: 'high',
      agent_oracle_fallback_models: ['qwen3.5', 'gpt-4.1'],
      agent_reviewer_model: 'anthropic/claude-sonnet-4.5',
      agent_reviewer_fallback_models: ['qwen3.5'],
    },
    modelFieldNames: ['agent_oracle_model', 'agent_reviewer_model'],
    fromModel: 'qwen3.5',
    toModel: 'qwen3.7',
    targetVariants: [],
    getVariantFieldName: (modelFieldName) => modelFieldName.replace('_model', '_variant'),
    getFallbackFieldName: (modelFieldName) => modelFieldName.replace('_model', '_fallback_models'),
  });

  assert.deepEqual(result, {
    updateValues: {
      agent_oracle_fallback_models: ['qwen3.7', 'gpt-4.1'],
      agent_reviewer_fallback_models: ['qwen3.7'],
    },
    replacedCount: 2,
    clearedVariantCount: 0,
  });
});

test('applyBatchReplaceModel does not replace fallback models when source variant filtering is enabled', () => {
  const result = applyBatchReplaceModel({
    values: {
      agent_oracle_model: 'openai/gpt-5.4',
      agent_oracle_variant: 'high',
      agent_oracle_fallback_models: ['openai/gpt-5.4'],
    },
    modelFieldNames: ['agent_oracle_model'],
    fromModel: 'openai/gpt-5.4',
    toModel: 'openai/gpt-5.5',
    fromVariant: 'high',
    targetVariants: ['high'],
    getVariantFieldName: (modelFieldName) => modelFieldName.replace('_model', '_variant'),
    getFallbackFieldName: (modelFieldName) => modelFieldName.replace('_model', '_fallback_models'),
  });

  assert.deepEqual(result, {
    updateValues: {
      agent_oracle_model: 'openai/gpt-5.5',
    },
    replacedCount: 1,
    clearedVariantCount: 0,
  });
});

test('applyBatchReplaceModel supports openagent field naming for category fallback models', () => {
  const result = applyBatchReplaceModel({
    values: {
      agent_architect: 'openai/gpt-5.4',
      agent_architect_fallback_models: ['qwen3.5'],
      category_coding: 'anthropic/claude-sonnet-4.5',
      category_coding_fallback_models: ['qwen3.5', 'gpt-4.1'],
    },
    modelFieldNames: ['agent_architect', 'category_coding'],
    fromModel: 'qwen3.5',
    toModel: 'qwen3.7',
    targetVariants: [],
    getVariantFieldName: (modelFieldName) => `${modelFieldName}_variant`,
    getFallbackFieldName: (modelFieldName) => `${modelFieldName}_fallback_models`,
  });

  assert.deepEqual(result, {
    updateValues: {
      agent_architect_fallback_models: ['qwen3.7'],
      category_coding_fallback_models: ['qwen3.7', 'gpt-4.1'],
    },
    replacedCount: 2,
    clearedVariantCount: 0,
  });
});
