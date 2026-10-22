/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import { buildFetchedHermesModel } from '../../../../../features/coding/hermes/utils/hermesFetchedModels.ts';
import type { PresetModel } from '../../../../../constants/presetModels.ts';

test('buildFetchedHermesModel keeps upstream id casing and name', () => {
  const record = buildFetchedHermesModel({ id: 'Claude-Sonnet-5', name: 'Sonnet 5' });
  assert.equal(record.id, 'Claude-Sonnet-5');
  assert.equal(record.name, 'Sonnet 5');
});

test('buildFetchedHermesModel fills preset context/max_tokens/reasoning/name', () => {
  const preset: PresetModel = {
    id: 'claude-sonnet-5',
    name: 'Claude Sonnet 5',
    contextLimit: 200000,
    outputLimit: 64000,
    reasoning: true,
    modalities: { input: ['text', 'image'], output: ['text'] },
  };
  const record = buildFetchedHermesModel({ id: 'claude-sonnet-5', name: 'Sonnet' }, preset);
  assert.equal(record.context_length, 200000);
  assert.equal(record.max_tokens, 64000);
  assert.equal(record.reasoning, true);
  // The preset's readable name wins over whatever the upstream echoed back —
  // most endpoints answer with the id, which is not a display name.
  assert.equal(record.name, 'Claude Sonnet 5');
});

test('buildFetchedHermesModel falls back to the fetched name when the preset has none', () => {
  const record = buildFetchedHermesModel({ id: 'x', name: 'Upstream X' }, { id: 'x', name: '' } as PresetModel);
  assert.equal(record.name, 'Upstream X');
});

test('buildFetchedHermesModel omits limits when preset lacks them', () => {
  const fallback = buildFetchedHermesModel({ id: 'x' }, { id: 'x', name: 'X' } as PresetModel);
  assert.equal('context_length' in fallback, false);
  assert.equal('max_tokens' in fallback, false);
  assert.equal('reasoning' in fallback, false);
  assert.equal(fallback.id, 'x');
});