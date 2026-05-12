/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildModelVariantsMap,
  type UnifiedModelOption,
} from '../../../../../services/opencodeApi.ts';

test('buildModelVariantsMap maps experimental virtual models to base variants', () => {
  const unifiedModels: UnifiedModelOption[] = [
    {
      id: 'opencode/gpt-5.5',
      displayName: 'OpenCode Zen / GPT-5.5',
      providerId: 'opencode',
      modelId: 'gpt-5.5',
      isFree: false,
    },
    {
      id: 'opencode/gpt-5.5-fast',
      displayName: 'OpenCode Zen / GPT-5.5 Fast',
      providerId: 'opencode',
      modelId: 'gpt-5.5-fast',
      isFree: false,
      baseModelId: 'gpt-5.5',
      experimentalMode: 'fast',
    },
    {
      id: 'opencode/gpt-5.5-preview-mode',
      displayName: 'OpenCode Zen / GPT-5.5 Preview-mode',
      providerId: 'opencode',
      modelId: 'gpt-5.5-preview-mode',
      isFree: false,
      baseModelId: 'gpt-5.5',
      experimentalMode: 'preview-mode',
    },
    {
      id: 'opencode/gpt-5.5-pro',
      displayName: 'OpenCode Zen / GPT-5.5 Pro',
      providerId: 'opencode',
      modelId: 'gpt-5.5-pro',
      isFree: false,
    },
  ];

  const variantsMap = buildModelVariantsMap(null, unifiedModels, {
    '@ai-sdk/openai': [
      {
        id: 'gpt-5.5',
        variants: {
          medium: {},
          high: {},
        },
      },
    ],
  });

  assert.deepEqual(variantsMap['opencode/gpt-5.5'], ['medium', 'high']);
  assert.deepEqual(variantsMap['opencode/gpt-5.5-fast'], ['medium', 'high']);
  assert.deepEqual(variantsMap['opencode/gpt-5.5-preview-mode'], ['medium', 'high']);
  assert.equal(variantsMap['opencode/gpt-5.5-pro'], undefined);
});

test('buildModelVariantsMap lets experimental models inherit custom config variants', () => {
  const unifiedModels: UnifiedModelOption[] = [
    {
      id: 'openai/openai/gpt-5.5-fast',
      displayName: 'OpenAI Compatible / GPT-5.5 Fast',
      providerId: 'openai',
      modelId: 'openai/gpt-5.5-fast',
      isFree: false,
      baseModelId: 'openai/gpt-5.5',
      experimentalMode: 'fast',
    },
  ];

  const variantsMap = buildModelVariantsMap(
    {
      provider: {
        openai: {
          models: {
            'openai/gpt-5.5': {
              variants: {
                customHigh: {},
              },
            },
          },
        },
      },
    },
    unifiedModels,
    undefined,
  );

  assert.deepEqual(variantsMap['openai/openai/gpt-5.5-fast'], ['customHigh']);
});

test('buildModelVariantsMap keeps experimental variants when base model is filtered out', () => {
  const unifiedModels: UnifiedModelOption[] = [
    {
      id: 'opencode/gpt-5.5-fast',
      displayName: 'OpenCode Zen / GPT-5.5 Fast',
      providerId: 'opencode',
      modelId: 'gpt-5.5-fast',
      isFree: false,
      baseModelId: 'gpt-5.5',
      experimentalMode: 'fast',
    },
  ];

  const variantsMap = buildModelVariantsMap(null, unifiedModels, {
    '@ai-sdk/openai': [
      {
        id: 'gpt-5.5',
        variants: {
          medium: {},
          high: {},
        },
      },
    ],
  });

  assert.deepEqual(variantsMap['opencode/gpt-5.5-fast'], ['medium', 'high']);
});

test('buildModelVariantsMap lets slash-scoped experimental models inherit preset variants', () => {
  const unifiedModels: UnifiedModelOption[] = [
    {
      id: 'zenmux/openai/gpt-5.5-fast',
      displayName: 'ZenMux / GPT-5.5 Fast',
      providerId: 'zenmux',
      modelId: 'openai/gpt-5.5-fast',
      isFree: false,
      baseModelId: 'openai/gpt-5.5',
      experimentalMode: 'fast',
    },
  ];

  const variantsMap = buildModelVariantsMap(null, unifiedModels, {
    '@ai-sdk/openai-compatible': [
      {
        id: 'gpt-5.5',
        variants: {
          none: {},
          high: {},
        },
      },
    ],
  });

  assert.deepEqual(variantsMap['zenmux/openai/gpt-5.5-fast'], ['none', 'high']);
});

test('buildModelVariantsMap does not infer experimental models from suffix alone', () => {
  const unifiedModels: UnifiedModelOption[] = [
    {
      id: 'opencode/grok-code-fast-1',
      displayName: 'OpenCode Zen / grok-code-fast-1',
      providerId: 'opencode',
      modelId: 'grok-code-fast-1',
      isFree: false,
    },
  ];

  const variantsMap = buildModelVariantsMap(null, unifiedModels, {
    '@ai-sdk/openai': [
      {
        id: 'grok-code-fast',
        variants: {
          high: {},
        },
      },
    ],
  });

  assert.equal(variantsMap['opencode/grok-code-fast-1'], undefined);
});
