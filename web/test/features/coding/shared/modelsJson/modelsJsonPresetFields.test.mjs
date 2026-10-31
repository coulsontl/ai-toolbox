import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const mappingPath = path.resolve(
  here,
  '../../../../../features/coding/shared/modelsJson/modelsJsonPresetFields.ts',
);

const source = () => readFile(mappingPath, 'utf8');

/**
 * `models.json` accepts fewer facts than a models.dev preset carries, and the
 * mapping between them is where a silent wrong value hides: the entry still
 * saves, the row still renders, and only the model behaves differently.
 *
 * SOP §4.2.4: 「本 CLI 没有对应字段的，留空/继承，不要硬塞」. SOP §4.2.5: the
 * fetch-apply path must carry preset parameters over, or 「应用」 hands the user
 * a bare id (the ZCode #31 regression).
 */
test('the preset mapping writes no field models.json does not have', async () => {
  const text = await source();
  const body = text.slice(
    text.indexOf('export const presetToModelsJsonFields'),
    text.indexOf('/**', text.indexOf('export const presetToModelsJsonFields')),
  );

  // The preset carries cost / options / variants; models.json has none of them,
  // and writing them would produce keys the products never read.
  for (const absent of ['cost', 'options', 'variants', 'modalities:']) {
    assert.ok(
      !new RegExp(`\\b${absent}\\b`).test(body),
      `presetToModelsJsonFields must not write \`${absent}\``,
    );
  }
});

test('temperature is not copied: the preset means a boolean, the file stores a number', async () => {
  const text = await source();
  const start = text.indexOf('export const presetToModelsJsonFields');
  const body = text.slice(start, text.indexOf('/**', start));

  // `preset.temperature` is "the model accepts a temperature parameter";
  // models.json's `temperature` is the value to send. Copying one into the
  // other writes `true` where a number belongs.
  assert.ok(
    !/temperature\s*:/.test(body),
    'temperature must stay unmapped — the two fields mean different things',
  );
});

test('only the image modality sets supportsImages', async () => {
  const text = await source();
  const start = text.indexOf('export const presetToModelsJsonFields');
  const body = text.slice(start, text.indexOf('/**', start));

  // models.json has one image flag and no pdf / audio / video flags. Folding
  // any non-image modality into `supportsImages` would mark a pdf-only model as
  // image-capable, which the product then advertises to the model picker.
  assert.match(body, /modalities\?\.input\?\.includes\('image'\)/);
  for (const other of ['pdf', 'audio', 'video']) {
    assert.ok(!body.includes(`'${other}'`), `\`${other}\` must not set supportsImages`);
  }
});

test('the capability flags stay absent when the preset does not state them', async () => {
  const text = await source();
  const start = text.indexOf('export const presetToModelsJsonFields');
  const body = text.slice(start, text.indexOf('/**', start));

  // A preset that omits `tool_call` must leave the form's value alone rather
  // than asserting `false`: models.json defaults `supportsToolCall` to true, and
  // "the catalogue did not say" is not "the model cannot call tools".
  assert.match(body, /preset\.tool_call === undefined \? \{\}/);
  assert.match(body, /preset\.reasoning === undefined \? \{\}/);
});

test('the fetched-model builder falls back to the caller defaults, not to false', async () => {
  const text = await source();
  const start = text.indexOf('export const buildFetchedModelsJsonModel');

  assert.ok(start > 0, 'buildFetchedModelsJsonModel must exist');
  const body = text.slice(start);

  assert.match(body, /matchedPreset\?\.tool_call \?\? existing\.supportsToolCall/);
  assert.match(body, /matchedPreset\?\.reasoning \?\? existing\.supportsReasoning/);
  // An id and a name must always be written — the entry cannot exist without
  // them, so there is no "inherit" for these two.
  assert.match(body, /id: fetched\.id/);
  assert.match(body, /name: matchedPreset\?\.name \|\| fetched\.name \|\| fetched\.id/);
});
