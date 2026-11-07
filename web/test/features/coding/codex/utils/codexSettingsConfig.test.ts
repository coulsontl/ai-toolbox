/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildCodexSettingsConfig,
  normalizeCodexAuxiliaryModels,
} from '../../../../../features/coding/codex/utils/codexSettingsConfig.ts';
import { resolveCodexDefaultReasoningEffort } from '../../../../../features/coding/codex/utils/codexCatalogModels.ts';
import {
  extractCodexModel,
  extractCodexReasoningEffort,
} from '../../../../../utils/codexConfigUtils.ts';

const CUSTOM_CONFIG = [
  'model_provider = "custom"',
  'model_reasoning_effort = "high"',
  '',
  '[model_providers.custom]',
  'name = "OpenAI"',
  'wire_api = "responses"',
].join('\n');

function buildSettings(options: { reasoningEffort?: string }) {
  const raw = buildCodexSettingsConfig({
    category: 'custom',
    apiKey: 'sk-test',
    baseUrl: 'https://example.com/v1',
    model: 'glm-5.3',
    reasoningEffort: options.reasoningEffort,
    config: CUSTOM_CONFIG,
    catalogModels: [
      { model: 'glm-5.3', displayName: 'GLM 5.3', defaultReasoningLevel: 'max' },
    ],
    auth: {},
  });
  return JSON.parse(raw) as { config: string };
}

test('main model reasoning effort is projected into config.toml with the model', () => {
  const settings = buildSettings({ reasoningEffort: 'max' });

  assert.equal(extractCodexModel(settings.config), 'glm-5.3');
  assert.equal(extractCodexReasoningEffort(settings.config), 'max');
});

test('missing main-model reasoning effort keeps the stored config.toml value', () => {
  const settings = buildSettings({});

  assert.equal(extractCodexModel(settings.config), 'glm-5.3');
  assert.equal(extractCodexReasoningEffort(settings.config), 'high');
});

test('main model without a declared level falls back to xhigh in config.toml', () => {
  const catalogModels = [{ model: 'glm-5.3', displayName: 'GLM 5.3' }];
  const raw = buildCodexSettingsConfig({
    category: 'custom',
    apiKey: 'sk-test',
    baseUrl: 'https://example.com/v1',
    model: 'glm-5.3',
    reasoningEffort: resolveCodexDefaultReasoningEffort(catalogModels, 'glm-5.3'),
    config: CUSTOM_CONFIG,
    catalogModels,
    auth: {},
  });
  const settings = JSON.parse(raw) as { config: string };

  assert.equal(extractCodexModel(settings.config), 'glm-5.3');
  assert.equal(extractCodexReasoningEffort(settings.config), 'xhigh');
});

test('auxiliary routes keep only the two valid shapes, with auto represented by absence', () => {
  assert.deepEqual(
    normalizeCodexAuxiliaryModels({
      title: { mode: 'main' },
      summary: { mode: 'model', model: '  gpt-5-mini  ' },
      description: { mode: 'model', model: '   ' },
      prediction: { mode: 'auto' },
      memory: 'main',
      unknown: { mode: 'main' },
    }),
    {
      title: { mode: 'main' },
      summary: { mode: 'model', model: 'gpt-5-mini' },
    },
  );
  assert.equal(normalizeCodexAuxiliaryModels({}), undefined);
  assert.equal(normalizeCodexAuxiliaryModels(null), undefined);
});

test('auxiliary routes are stored without leftover model fields and dropped when empty', () => {
  const raw = buildCodexSettingsConfig({
    category: 'custom',
    apiKey: 'sk-test',
    baseUrl: 'https://example.com/v1',
    model: 'glm-5.3',
    config: CUSTOM_CONFIG,
    catalogModels: [],
    auxiliaryModels: {
      title: { mode: 'main' },
      description: { mode: 'model', model: ' glm-5.3 ' },
    },
    auth: {},
  });
  const settings = JSON.parse(raw) as {
    auxiliaryModels?: Record<string, unknown>;
  };
  assert.deepEqual(settings.auxiliaryModels, {
    title: { mode: 'main' },
    description: { mode: 'model', model: 'glm-5.3' },
  });

  const empty = JSON.parse(buildCodexSettingsConfig({
    category: 'custom',
    apiKey: 'sk-test',
    baseUrl: 'https://example.com/v1',
    model: 'glm-5.3',
    config: CUSTOM_CONFIG,
    catalogModels: [],
    auxiliaryModels: {},
    auth: {},
  })) as { auxiliaryModels?: unknown };
  assert.equal(empty.auxiliaryModels, undefined);
});

test('auxiliary routes drop stale model fields and are never stored for official providers', () => {
  assert.deepEqual(
    normalizeCodexAuxiliaryModels({
      title: { mode: 'main', model: 'stale-model' },
      summary: { mode: 'model', model: 'gpt-5-mini', extra: true },
    }),
    {
      title: { mode: 'main' },
      summary: { mode: 'model', model: 'gpt-5-mini' },
    },
  );

  const official = JSON.parse(buildCodexSettingsConfig({
    category: 'official',
    apiKey: '',
    baseUrl: '',
    model: 'gpt-5.5',
    config: '',
    catalogModels: [],
    auxiliaryModels: { title: { mode: 'main' } },
    auth: {},
  })) as { auxiliaryModels?: unknown };
  assert.equal(official.auxiliaryModels, undefined);
});
