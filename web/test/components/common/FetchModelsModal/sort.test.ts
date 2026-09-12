/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  compareFetchedModels,
  createFetchedModelsComparator,
} from '../../../../components/common/FetchModelsModal/sort.ts';

const ids = (models: Array<{ id: string; ownedBy?: string }>) => models.map((model) => model.id);

const sample = [
  { id: 'metapi/GLM-5.3-Flash', ownedBy: 'lilililwan' },
  { id: 'GLM-5.3-Flash', ownedBy: 'lilililwan' },
  { id: 'qwen3.8-flash', ownedBy: 'cun' },
  { id: 'gpt-5.6-sol', ownedBy: 'openai' },
  { id: 'gpt-5.6-terra', ownedBy: 'openai' },
];

test('default comparator groups owners alphabetically without priority', () => {
  const sorted = [...sample].sort(compareFetchedModels);

  assert.deepEqual(ids(sorted), [
    'qwen3.8-flash',
    'GLM-5.3-Flash',
    'metapi/GLM-5.3-Flash',
    'gpt-5.6-sol',
    'gpt-5.6-terra',
  ]);
});

test('comparator pins priority owners first, then other groups alphabetically, ownerless last', () => {
  const sorted = [
    { id: 'b-model' },
    ...sample,
    { id: 'a-model' },
  ].sort(createFetchedModelsComparator(['openai']));

  // Ownerless models share the last bucket and fall back to id ordering.
  assert.deepEqual(ids(sorted), [
    'gpt-5.6-sol',
    'gpt-5.6-terra',
    'qwen3.8-flash',
    'GLM-5.3-Flash',
    'metapi/GLM-5.3-Flash',
    'a-model',
    'b-model',
  ]);
});

test('comparator respects multiple priority owners in the given order', () => {
  const sorted = [...sample].sort(createFetchedModelsComparator(['openai', 'lilililwan']));

  assert.deepEqual(ids(sorted), [
    'gpt-5.6-sol',
    'gpt-5.6-terra',
    'GLM-5.3-Flash',
    'metapi/GLM-5.3-Flash',
    'qwen3.8-flash',
  ]);
});

test('comparator sorts ids case-insensitively with natural numeric order', () => {
  const sorted = [
    { id: 'gpt-5.10', ownedBy: 'openai' },
    { id: 'deepseek-v4.1', ownedBy: 'openai' },
    { id: 'DeepSeek-V4-Pro', ownedBy: 'openai' },
    { id: 'gpt-5.6', ownedBy: 'openai' },
  ].sort(compareFetchedModels);

  // Case-insensitive; numeric runs compare as numbers (5.6 < 5.10). ICU
  // collation breaks the "V4-Pro" vs "V4.1" tie by punctuation weight.
  assert.deepEqual(ids(sorted), ['DeepSeek-V4-Pro', 'deepseek-v4.1', 'gpt-5.6', 'gpt-5.10']);
});
