import assert from 'node:assert/strict';
import test from 'node:test';

import {
  filterHistoryJobParamsByModel,
  getImageParameterVisibility,
  parseHistoryJobParams,
  resolveImageModelProfile,
} from '../../../../../features/coding/image/utils/modelProfile.ts';
import type { ImageTaskParams } from '../../../../../features/coding/image/services/imageApi.ts';

test('resolveImageModelProfile detects nano-banana model ids and names', () => {
  assert.equal(resolveImageModelProfile('google/nano-banana'), 'gemini_banana');
  assert.equal(resolveImageModelProfile('nano-banana-pro'), 'gemini_banana');
  assert.equal(
    resolveImageModelProfile('custom-image-model', 'Nano-Banana Pro'),
    'gemini_banana'
  );
  assert.equal(resolveImageModelProfile('gpt-image-1'), 'default');
});

test('getImageParameterVisibility hides openai-specific fields for banana models', () => {
  assert.deepEqual(getImageParameterVisibility('google/nano-banana'), {
    size: true,
    quality: true,
    outputFormat: true,
    moderation: false,
    outputCompression: false,
  });

  assert.deepEqual(getImageParameterVisibility('gpt-image-1'), {
    size: true,
    quality: true,
    outputFormat: true,
    moderation: true,
    outputCompression: true,
  });
});

test('parseHistoryJobParams returns null for empty or invalid payloads', () => {
  assert.equal(parseHistoryJobParams('   '), null);
  assert.equal(parseHistoryJobParams('{invalid json}'), null);
});

test('filterHistoryJobParamsByModel removes hidden fields for banana model history', () => {
  const params = {
    size: '1024x1024',
    quality: 'high',
    output_format: 'png',
    output_compression: 80,
    moderation: 'auto',
  };

  assert.deepEqual(
    filterHistoryJobParamsByModel(params, 'google/nano-banana'),
    {
      size: '1024x1024',
      quality: 'high',
      output_format: 'png',
    }
  );

  assert.deepEqual(
    filterHistoryJobParamsByModel(params, 'gpt-image-1'),
    params
  );
});

test('banana submission params can omit hidden moderation field', () => {
  const visibility = getImageParameterVisibility('google/nano-banana');
  const params: ImageTaskParams = {
    size: '1024x1024',
    quality: 'high',
    output_format: 'png',
    output_compression: visibility.outputCompression ? 80 : null,
    moderation: visibility.moderation ? 'low' : null,
  };

  assert.equal(params.moderation, null);
});
