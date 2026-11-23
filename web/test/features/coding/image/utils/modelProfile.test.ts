import assert from 'node:assert/strict';
import test from 'node:test';

import {
  filterHistoryJobParamsByModel,
  getImageParameterVisibility,
  parseHistoryJobParams,
  resolveImageModelProfile,
} from '../../../../../features/coding/image/utils/modelProfile.ts';
import { getImageProviderProfile } from '../../../../../features/coding/image/utils/providerProfile.ts';
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

test('resolveImageModelProfile detects MiniMax H3 model ids and names', () => {
  assert.equal(resolveImageModelProfile('MiniMax-H3-Image'), 'minimax_h3');
  assert.equal(resolveImageModelProfile('minimax-h3-image-highspeed'), 'minimax_h3');
  assert.equal(
    resolveImageModelProfile('custom-image-model', 'MiniMax-H3-Image-Highspeed'),
    'minimax_h3'
  );
  assert.equal(resolveImageModelProfile('gpt-image-1'), 'default');
});

test('getImageParameterVisibility hides unsupported quality and moderation for MiniMax H3', () => {
  assert.deepEqual(
    getImageParameterVisibility('openai_compatible', 'MiniMax-H3-Image-Highspeed'),
    {
      size: true,
      quality: false,
      outputFormat: true,
      moderation: false,
      outputCompression: true,
    }
  );
});

test('getImageParameterVisibility hides openai-specific fields for banana models', () => {
  assert.deepEqual(getImageParameterVisibility('openai_compatible', 'google/nano-banana'), {
    size: true,
    quality: true,
    outputFormat: true,
    moderation: false,
    outputCompression: false,
  });

  assert.deepEqual(getImageParameterVisibility('openai_compatible', 'gpt-image-1'), {
    size: true,
    quality: true,
    outputFormat: true,
    moderation: true,
    outputCompression: true,
  });
});

test('getImageParameterVisibility hides openai-only fields for gemini native channels', () => {
  assert.deepEqual(getImageParameterVisibility('gemini', 'gemini-2.5-flash-image'), {
    size: true,
    quality: false,
    outputFormat: false,
    moderation: false,
    outputCompression: false,
  });
});

test('getImageParameterVisibility hides unsupported moderation for openai_responses channels', () => {
  assert.deepEqual(getImageParameterVisibility('openai_responses', 'gpt-image-1'), {
    size: true,
    quality: true,
    outputFormat: true,
    moderation: false,
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
    filterHistoryJobParamsByModel(params, 'openai_compatible', 'google/nano-banana'),
    {
      size: '1024x1024',
      quality: 'high',
      output_format: 'png',
    }
  );

  assert.deepEqual(
    filterHistoryJobParamsByModel(params, 'openai_compatible', 'gpt-image-1'),
    params
  );
});

test('filterHistoryJobParamsByModel removes hidden fields for MiniMax H3 history', () => {
  const params = {
    size: '1024x1024',
    quality: 'high',
    output_format: 'webp',
    output_compression: 80,
    moderation: 'auto',
  };

  assert.deepEqual(
    filterHistoryJobParamsByModel(params, 'openai_compatible', 'MiniMax-H3-Image'),
    {
      size: '1024x1024',
      output_format: 'webp',
      output_compression: 80,
    }
  );
});

test('banana submission params can omit hidden moderation field', () => {
  const visibility = getImageParameterVisibility('openai_compatible', 'google/nano-banana');
  const params: ImageTaskParams = {
    size: '1024x1024',
    quality: 'high',
    output_format: 'png',
    output_compression: visibility.outputCompression ? 80 : null,
    moderation: visibility.moderation ? 'low' : null,
  };

  assert.equal(params.moderation, null);
});

test('gemini native history params only keep size', () => {
  const params = {
    size: '1536x1024',
    quality: 'high',
    output_format: 'png',
    output_compression: 80,
    moderation: 'auto',
  };

  assert.deepEqual(
    filterHistoryJobParamsByModel(params, 'gemini', 'gemini-2.5-flash-image'),
    {
      size: '1536x1024',
    }
  );
});

test('openai responses history params omit moderation', () => {
  const params = {
    size: '1536x1024',
    quality: 'high',
    output_format: 'png',
    output_compression: 80,
    moderation: 'auto',
  };

  assert.deepEqual(
    filterHistoryJobParamsByModel(params, 'openai_responses', 'gpt-image-1'),
    {
      size: '1536x1024',
      quality: 'high',
      output_format: 'png',
      output_compression: 80,
    }
  );
});

test('provider profiles centralize path capability and default base url', () => {
  assert.equal(getImageProviderProfile('openai_compatible').supportsCustomPaths, true);
  assert.equal(getImageProviderProfile('openai_responses').supportsCustomPaths, false);
  assert.equal(getImageProviderProfile('gemini').supportsCustomPaths, false);
  assert.equal(
    getImageProviderProfile('gemini').defaultBaseUrl,
    'https://generativelanguage.googleapis.com/v1beta'
  );
});
