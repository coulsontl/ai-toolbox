/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  patchCodexMemoriesModels,
  readCodexMemoriesModels,
} from '../../../../../features/coding/codex/utils/codexMemoriesModels.ts';

test('reads both native memory models and ignores unrelated keys', () => {
  const configToml = [
    'model = "main"',
    '',
    '[memories]',
    'extract_model = "gpt-5-mini"',
    'consolidation_model = "gpt-5-mini"',
    'generate_memories = true',
    '',
    '[model_providers.custom]',
    'name = "OpenAI"',
  ].join('\n');

  assert.deepEqual(readCodexMemoriesModels(configToml), {
    extractModel: 'gpt-5-mini',
    consolidationModel: 'gpt-5-mini',
  });
});

test('missing [memories] table reads as empty', () => {
  assert.deepEqual(readCodexMemoriesModels('model = "main"\n'), {});
  assert.deepEqual(readCodexMemoriesModels(''), {});
});

test('quoted, spaced and commented [memories] headers are recognized', () => {
  for (const header of ['["memories"]', "['memories']", '[ memories ]', '[memories] # native']) {
    const configToml = [header, 'extract_model = "old"', '[other]', 'value = 1'].join('\n');
    assert.deepEqual(
      readCodexMemoriesModels(configToml),
      { extractModel: 'old', consolidationModel: undefined },
      header,
    );
    const result = patchCodexMemoriesModels(configToml, { extractModel: 'new' });
    assert.equal(result.changed, true, header);
    assert.equal(
      result.nextToml,
      [header, 'extract_model = "new"', '[other]', 'value = 1'].join('\n'),
      header,
    );
  }
});

test('quoted top-level memories keys are still rejected as unsupported shapes', () => {
  assert.equal(
    readCodexMemoriesModels('"memories".extract_model = "x"\n').unsupportedShape,
    'dotted_keys',
  );
  assert.equal(
    readCodexMemoriesModels("'memories' = { extract_model = \"x\" }\n").unsupportedShape,
    'inline_table',
  );
});

test('inline table and dotted keys disable structured editing', () => {
  assert.equal(
    readCodexMemoriesModels('memories = { extract_model = "x" }\n').unsupportedShape,
    'inline_table',
  );
  assert.equal(
    readCodexMemoriesModels('memories.extract_model = "x"\n').unsupportedShape,
    'dotted_keys',
  );
});

test('invalid TOML and multiline values are reported as unsupported', () => {
  assert.equal(readCodexMemoriesModels('[memories\n').unsupportedShape, 'invalid_toml');
  assert.equal(
    readCodexMemoriesModels('[memories]\nextract_model = """\nx\n"""\n').unsupportedShape,
    'multiline_value',
  );
});

test('patch replaces only the value and preserves comments, layout and CRLF', () => {
  const configToml = [
    'model = "main"',
    '',
    '[memories]',
    'extract_model = "old" # keep this comment',
    'generate_memories = true',
  ].join('\r\n');

  const result = patchCodexMemoriesModels(configToml, {
    extractModel: 'new-model',
    consolidationModel: '',
  });

  assert.equal(result.changed, true);
  assert.equal(
    result.nextToml,
    [
      'model = "main"',
      '',
      '[memories]',
      'extract_model = "new-model" # keep this comment',
      'generate_memories = true',
    ].join('\r\n'),
  );
});

test('patch preserves quoted keys and single-quoted string style', () => {
  const configToml = [
    '[memories]',
    '"extract_model" = \'old\'',
    "'consolidation_model' = 'old-two'",
  ].join('\n');

  const result = patchCodexMemoriesModels(configToml, {
    extractModel: 'new',
    consolidationModel: 'new-two',
  });

  assert.equal(
    result.nextToml,
    [
      '[memories]',
      '"extract_model" = \'new\'',
      "'consolidation_model' = 'new-two'",
    ].join('\n'),
  );
});

test('patch inserts a missing key with the existing table indentation', () => {
  const configToml = [
    '[memories]',
    '  extract_model = "old"',
    '',
    '[other]',
    'value = 1',
  ].join('\n');

  const result = patchCodexMemoriesModels(configToml, {
    extractModel: 'old',
    consolidationModel: 'new-model',
  });

  assert.equal(
    result.nextToml,
    [
      '[memories]',
      '  extract_model = "old"',
      '  consolidation_model = "new-model"',
      '',
      '[other]',
      'value = 1',
    ].join('\n'),
  );
});

test('patch appends a [memories] table when none exists', () => {
  const result = patchCodexMemoriesModels('model = "main"', {
    extractModel: 'mini',
  });

  assert.equal(result.nextToml, 'model = "main"\n\n[memories]\nextract_model = "mini"\n');
});

test('patch round-trips backslashes, quotes and non-ASCII text', () => {
  const value = 'C:\\models\\"quoted" 中文';
  const result = patchCodexMemoriesModels('[memories]\nextract_model = "old"\n', {
    extractModel: value,
  });

  assert.deepEqual(readCodexMemoriesModels(result.nextToml), {
    extractModel: value,
    consolidationModel: undefined,
  });
});

test('clearing a value removes only that key and keeps the table', () => {
  const configToml = [
    '[memories]',
    'extract_model = "old"',
    'consolidation_model = "also-old"',
  ].join('\n');

  const result = patchCodexMemoriesModels(configToml, {
    extractModel: '',
    consolidationModel: 'also-old',
  });

  assert.equal(
    result.nextToml,
    ['[memories]', 'consolidation_model = "also-old"'].join('\n'),
  );
});

test('unsupported shapes are left untouched', () => {
  const configToml = 'memories = { extract_model = "x" }\n';
  const result = patchCodexMemoriesModels(configToml, { extractModel: 'y' });

  assert.equal(result.changed, false);
  assert.equal(result.nextToml, configToml);
  assert.equal(result.unsupportedShape, 'inline_table');
});
