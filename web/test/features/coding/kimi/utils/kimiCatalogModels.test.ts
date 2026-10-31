import test from 'node:test';
import assert from 'node:assert/strict';

import type { KimiCatalogModel } from '../../../../../types/kimi.ts';
import {
  KIMI_CAPABILITY_VALUES,
  KIMI_SUPPORTED_CAPABILITIES,
  KIMI_SUPPORTED_EFFORTS,
  buildKimiCatalogModelsFromPresets,
  importModelsIntoKimiCatalog,
  kimiCatalogRowKey,
  normalizeKimiCapabilities,
  normalizeKimiStringArray,
  removeKimiCatalogModels,
  upsertKimiCatalogModel,
} from '../../../../../features/coding/kimi/utils/kimiCatalogModels.ts';

function model(overrides: Partial<KimiCatalogModel> = {}): KimiCatalogModel {
  return {
    key: 'moonshotai/kimi-k3',
    model: 'kimi-k3',
    provider: 'moonshotai',
    ...overrides,
  };
}

test('kimiCatalogRowKey uses the alias key as row identity', () => {
  assert.equal(kimiCatalogRowKey(model()), 'moonshotai/kimi-k3');
  assert.equal(kimiCatalogRowKey(model({ key: '  padded/key  ' })), 'padded/key');
});

test('upsertKimiCatalogModel appends when no previous row is given', () => {
  const existing = [model()];
  const added = model({ key: 'moonshotai/kimi-k2.6', model: 'kimi-k2.6' });

  const result = upsertKimiCatalogModel(existing, added);

  assert.equal(result.length, 2);
  assert.equal(result[1].key, 'moonshotai/kimi-k2.6');
});

test('upsertKimiCatalogModel replaces the row behind previousRowKey in place', () => {
  const existing = [
    model(),
    model({ key: 'moonshotai/kimi-k2.6', model: 'kimi-k2.6' }),
    model({ key: 'moonshotai/kimi-k2.5', model: 'kimi-k2.5' }),
  ];
  const edited = model({ key: 'moonshotai/kimi-k3', model: 'kimi-k3', displayName: 'K3' });

  const result = upsertKimiCatalogModel(existing, edited, 'moonshotai/kimi-k3');

  assert.equal(result.length, 3);
  // Replaced in place, not appended.
  assert.equal(result[0].displayName, 'K3');
  assert.equal(result[1].key, 'moonshotai/kimi-k2.6');
});

test('upsertKimiCatalogModel appends when the previous row disappeared', () => {
  const existing = [model({ key: 'other/key', model: 'key' })];
  const edited = model();

  const result = upsertKimiCatalogModel(existing, edited, 'moonshotai/kimi-k3');

  assert.equal(result.length, 2);
  assert.equal(result[1].key, 'moonshotai/kimi-k3');
});

test('removeKimiCatalogModels drops every row whose key matches', () => {
  const existing = [
    model(),
    model({ key: 'moonshotai/kimi-k2.6', model: 'kimi-k2.6' }),
    model({ key: 'moonshotai/kimi-k2.5', model: 'kimi-k2.5' }),
  ];

  const result = removeKimiCatalogModels(existing, [
    'moonshotai/kimi-k3',
    'moonshotai/kimi-k2.5',
  ]);

  assert.deepEqual(result.map((item) => item.key), ['moonshotai/kimi-k2.6']);
});

test('normalizeKimiCapabilities keeps only the vocabulary the CLI understands', () => {
  const result = normalizeKimiCapabilities([
    'image_in',
    'bogus_capability',
    'thinking',
    '  tool_use  ',
  ]);

  assert.deepEqual(result, ['image_in', 'thinking', 'tool_use']);
});

test('normalizeKimiCapabilities returns undefined for empty or non-array input', () => {
  assert.equal(normalizeKimiCapabilities([]), undefined);
  assert.equal(normalizeKimiCapabilities(['nope']), undefined);
  assert.equal(normalizeKimiCapabilities(undefined), undefined);
  assert.equal(normalizeKimiCapabilities('image_in'), undefined);
});

test('normalizeKimiStringArray trims and drops blanks without a vocabulary filter', () => {
  assert.deepEqual(normalizeKimiStringArray([' low ', '', '  ', 'custom']), ['low', 'custom']);
  assert.equal(normalizeKimiStringArray([]), undefined);
  assert.equal(normalizeKimiStringArray(undefined), undefined);
});

test('capability and effort vocabularies cover the CLI enums', () => {
  // Guards against silently dropping a capability the CLI accepts.
  assert.deepEqual([...KIMI_CAPABILITY_VALUES], [
    'image_in',
    'video_in',
    'audio_in',
    'thinking',
    'always_thinking',
    'tool_use',
  ]);
  // The dialog renders this same list; a caption pointing at a key that does
  // not exist would render the literal key name.
  assert.deepEqual(
    [...KIMI_SUPPORTED_CAPABILITIES].map((item) => item.value),
    [...KIMI_CAPABILITY_VALUES],
  );
  assert.deepEqual([...KIMI_SUPPORTED_EFFORTS], ['low', 'medium', 'high', 'max']);
});

test('buildKimiCatalogModelsFromPresets derives keys, sizes and capabilities', () => {
  const rows = buildKimiCatalogModelsFromPresets(
    [{
      id: 'kimi-k3',
      displayName: 'Kimi K3',
      maxContextSize: 1048576,
      maxInputSize: 900000,
      maxOutputSize: 262144,
      reasoning: true,
      toolCall: true,
      inputModalities: ['text', 'image', 'video'],
      outputModalities: ['text'],
    }],
    'moonshotai',
    [],
    262144,
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].key, 'moonshotai/kimi-k3');
  assert.equal(rows[0].model, 'kimi-k3');
  assert.equal(rows[0].provider, 'moonshotai');
  assert.equal(rows[0].displayName, 'Kimi K3');
  assert.equal(rows[0].maxContextSize, 1048576);
  assert.equal(rows[0].maxInputSize, 900000);
  assert.equal(rows[0].maxOutputSize, 262144);
  // text has no capability counterpart; image/video map to *_in, the catalog's
  // tool_call flag adds tool_use, and reasoning adds thinking.
  assert.deepEqual(rows[0].capabilities, ['image_in', 'video_in', 'tool_use', 'thinking']);
});

test('buildKimiCatalogModelsFromPresets does not infer tool_use from the output modality', () => {
  // 969 catalog models declare `tool_call: false`; the output modality being
  // text must not promote them, or the CLI fails when the upstream rejects tools.
  const rows = buildKimiCatalogModelsFromPresets(
    [{
      id: 'no-tools',
      reasoning: false,
      toolCall: false,
      inputModalities: ['text'],
      outputModalities: ['text'],
    }],
    'custom',
    [],
    262144,
  );

  assert.equal(rows[0].capabilities, undefined);
});

test('buildKimiCatalogModelsFromPresets falls back to the CLI context requirement', () => {
  const rows = buildKimiCatalogModelsFromPresets(
    [{
      id: 'k3',
      reasoning: false,
      toolCall: false,
      inputModalities: [],
      outputModalities: [],
    }],
    'custom',
    [],
    262144,
  );

  assert.equal(rows[0].maxContextSize, 262144);
  assert.equal(rows[0].displayName, undefined);
  assert.equal(rows[0].capabilities, undefined);
});

test('buildKimiCatalogModelsFromPresets skips models already in the catalog', () => {
  const existing = [model({ key: 'moonshotai/kimi-k3', model: 'kimi-k3' })];
  const rows = buildKimiCatalogModelsFromPresets(
    [
      { id: 'kimi-k3', reasoning: false, toolCall: false, inputModalities: [], outputModalities: [] },
      { id: 'kimi-k2.6', reasoning: false, toolCall: false, inputModalities: [], outputModalities: [] },
    ],
    'moonshotai',
    existing,
    262144,
  );

  // The known upstream id is skipped, the new one is added.
  assert.deepEqual(rows.map((row) => row.model), ['kimi-k2.6']);
});

test('importModelsIntoKimiCatalog follows the fetched display order', () => {
  const current = [model({ key: 'pinned/custom', model: 'custom', provider: 'pinned' })];
  const result = importModelsIntoKimiCatalog(
    current,
    [{ id: 'b' }, { id: 'a' }],
    [],
    ['a', 'b'],
    'relay',
    262144,
  );

  // Fetched rows first in display order, then the untouched custom row.
  assert.deepEqual(result.map((row) => row.key), ['relay/a', 'relay/b', 'pinned/custom']);
});

test('importModelsIntoKimiCatalog drops only explicitly removed models', () => {
  const current = [
    model({ key: 'relay/a', model: 'a', provider: 'relay' }),
    model({ key: 'relay/b', model: 'b', provider: 'relay' }),
  ];
  const result = importModelsIntoKimiCatalog(current, [], ['b'], [], 'relay', 262144);

  assert.deepEqual(result.map((row) => row.model), ['a']);
});

test('importModelsIntoKimiCatalog preserves an existing row instead of duplicating it', () => {
  const current = [
    model({
      key: 'relay/a',
      model: 'a',
      provider: 'relay',
      displayName: 'My Name',
      maxContextSize: 999,
    }),
  ];
  const result = importModelsIntoKimiCatalog(current, [{ id: 'a' }], [], ['a'], 'relay', 262144);

  assert.equal(result.length, 1);
  // User customizations survive the re-import.
  assert.equal(result[0].displayName, 'My Name');
  assert.equal(result[0].maxContextSize, 999);
});

test('importModelsIntoKimiCatalog gives new rows the CLI-required context size', () => {
  const result = importModelsIntoKimiCatalog([], [{ id: 'x', name: 'X' }], [], ['x'], 'relay', 262144);

  assert.equal(result[0].maxContextSize, 262144);
  assert.equal(result[0].displayName, 'X');
});

test('importModelsIntoKimiCatalog does not resurrect a second alias for a selected upstream', () => {
  // Two rows already map the same upstream id. The ordered loop keeps the
  // first; the trailing reference-equality check used to push the second one
  // back, leaving the catalog with two rows for one selected upstream.
  const current = [
    model({ key: 'relay/a', model: 'a', provider: 'relay' }),
    model({ key: 'pinned/a', model: 'a', provider: 'pinned' }),
  ];
  const result = importModelsIntoKimiCatalog(current, [{ id: 'a' }], [], ['a'], 'relay', 262144);

  assert.deepEqual(result.map((row) => row.key), ['relay/a']);
});

test('importModelsIntoKimiCatalog leaves duplicate aliases alone when their upstream is not selected', () => {
  // The fetch selected a different model, so the duplicated upstream is none
  // of its business — both aliases survive untouched.
  const current = [
    model({ key: 'relay/a', model: 'a', provider: 'relay' }),
    model({ key: 'pinned/a', model: 'a', provider: 'pinned' }),
  ];
  const result = importModelsIntoKimiCatalog(current, [{ id: 'b' }], [], ['b'], 'relay', 262144);

  assert.deepEqual(result.map((row) => row.key), ['relay/b', 'relay/a', 'pinned/a']);
});
