/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  importModelsIntoCatalog,
  normalizeCodexCatalogModels,
} from '../../../../../features/coding/codex/utils/codexCatalogModels.ts';

const resolveContextWindow = (modelId: string): number | undefined => {
  if (modelId === 'gpt-6-astra') {
    return 1_050_000;
  }
  if (modelId === 'glm-5.3-flash') {
    return 1_000_000;
  }
  return undefined;
};

test('importModelsIntoCatalog reorders existing rows to the fetched display order and appends additions', () => {
  const rows = importModelsIntoCatalog(
    [{ model: 'qwen3.8-flash', contextWindow: 128000 }, { model: 'gpt-6-astra' }],
    [{ id: 'glm-5.3-flash' }],
    [],
    ['gpt-6-astra', 'glm-5.3-flash', 'qwen3.8-flash'],
    resolveContextWindow,
  );

  assert.deepEqual(rows, [
    { model: 'gpt-6-astra' },
    { model: 'glm-5.3-flash', contextWindow: 1000000 },
    { model: 'qwen3.8-flash', contextWindow: 128000 },
  ]);
});

test('importModelsIntoCatalog keeps rows unknown to the fetch at the end in previous order', () => {
  const rows = importModelsIntoCatalog(
    [{ model: 'custom-model' }, { model: 'qwen3.8-flash', contextWindow: 128000 }, { model: 'pinned-model' }],
    [],
    [],
    ['qwen3.8-flash'],
    resolveContextWindow,
  );

  assert.deepEqual(rows, [
    { model: 'qwen3.8-flash', contextWindow: 128000 },
    { model: 'custom-model' },
    { model: 'pinned-model' },
  ]);
});

test('importModelsIntoCatalog drops removed ids and can re-add them fresh in one call', () => {
  const rows = importModelsIntoCatalog(
    [{ model: 'met/ds4f', contextWindow: 4096 }],
    [{ id: 'met/ds4f' }],
    ['met/ds4f'],
    ['met/ds4f'],
    resolveContextWindow,
  );

  assert.deepEqual(rows, [{ model: 'met/ds4f' }]);
});

test('importModelsIntoCatalog keeps duplicate model ids with different display names intact', () => {
  const luna = { model: 'terra', displayName: 'luna' };
  const terra = { model: 'terra', displayName: 'terra' };
  const rows = importModelsIntoCatalog(
    [luna, terra],
    [],
    [],
    ['terra'],
    resolveContextWindow,
  );

  assert.deepEqual(rows, [luna, terra]);
});

test('importModelsIntoCatalog only adds selected ids even when the fetch returns more', () => {
  const rows = importModelsIntoCatalog(
    [],
    [],
    [],
    ['gpt-6-astra', 'glm-5.3-flash'],
    resolveContextWindow,
  );

  assert.deepEqual(rows, []);
});

test('importModelsIntoCatalog skips blank ids and dedupes ordered ids', () => {
  const rows = importModelsIntoCatalog(
    [],
    [{ id: 'glm-5.3-flash' }],
    [],
    ['', '  ', 'glm-5.3-flash', 'glm-5.3-flash'],
    resolveContextWindow,
  );

  assert.deepEqual(rows, [{ model: 'glm-5.3-flash', contextWindow: 1000000 }]);
});

test('importModelsIntoCatalog output stays stable through catalog normalization', () => {
  const rows = importModelsIntoCatalog(
    [],
    [{ id: ' gpt-6-astra ' }],
    [],
    ['gpt-6-astra', 'unknown-model'],
    resolveContextWindow,
  );

  assert.deepEqual(normalizeCodexCatalogModels(rows.filter((row) => row.model === 'gpt-6-astra')), [
    { model: 'gpt-6-astra', contextWindow: 1050000 },
  ]);
});
