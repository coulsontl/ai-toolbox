/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildPiThinkingLevelMapFromPreset,
  isPiThinkingLevelMapEntrySupported,
  normalizePiThinkingLevelKey,
  PI_THINKING_LEVEL_OPTIONS,
} from '../../utils/piModelMetadata.ts';

test('normalizePiThinkingLevelKey maps none to off', () => {
  assert.equal(normalizePiThinkingLevelKey('none'), 'off');
  assert.equal(normalizePiThinkingLevelKey('medium'), 'medium');
  assert.equal(normalizePiThinkingLevelKey('max'), 'max');
  assert.equal(normalizePiThinkingLevelKey('unknown'), undefined);
});

test('buildPiThinkingLevelMapFromPreset fills omitted levels when preset has variants', () => {
  const thinkingLevelMap = buildPiThinkingLevelMapFromPreset({
    none: { reasoningEffort: 'none' },
    medium: { thinkingConfig: { thinkingLevel: 'medium' } },
    high: { disabled: true },
  });

  assert.deepEqual(thinkingLevelMap, {
    off: 'none',
    minimal: null,
    low: null,
    medium: 'medium',
    high: null,
    xhigh: null,
    max: null,
  });
});

test('buildPiThinkingLevelMapFromPreset preserves max reasoning effort', () => {
  assert.deepEqual(buildPiThinkingLevelMapFromPreset({
    none: { reasoningEffort: 'none' },
    xhigh: { reasoningEffort: 'xhigh' },
    max: { reasoningEffort: 'max' },
  }), {
    off: 'none',
    minimal: null,
    low: null,
    medium: null,
    high: null,
    xhigh: 'xhigh',
    max: 'max',
  });
});

test('buildPiThinkingLevelMapFromPreset reads Claude effort variants', () => {
  assert.deepEqual(buildPiThinkingLevelMapFromPreset({
    high: {
      thinking: {
        display: 'summarized',
        type: 'adaptive',
      },
      effort: 'high',
    },
    low: {
      thinking: {
        display: 'summarized',
        type: 'adaptive',
      },
      effort: 'low',
    },
    max: {
      thinking: {
        display: 'summarized',
        type: 'adaptive',
      },
      effort: 'max',
    },
    medium: {
      thinking: {
        display: 'summarized',
        type: 'adaptive',
      },
      effort: 'medium',
    },
    xhigh: {
      thinking: {
        display: 'summarized',
        type: 'adaptive',
      },
      effort: 'xhigh',
    },
  }), {
    off: null,
    minimal: null,
    low: 'low',
    medium: 'medium',
    high: 'high',
    xhigh: 'xhigh',
    max: 'max',
  });
});

test('buildPiThinkingLevelMapFromPreset returns empty map when variants are empty', () => {
  assert.deepEqual(buildPiThinkingLevelMapFromPreset({}), {});
  assert.deepEqual(buildPiThinkingLevelMapFromPreset(undefined), {});
});

test('default Pi thinking level options only include standard identity-mapped levels', () => {
  assert.deepEqual(PI_THINKING_LEVEL_OPTIONS.map(({ value }) => value), [
    'off',
    'minimal',
    'low',
    'medium',
    'high',
  ]);
});

test('extended Pi thinking levels require an explicit non-null mapping', () => {
  const thinkingLevelMap = {
    minimal: null,
    high: 'high',
    xhigh: 'max',
  };

  assert.equal(isPiThinkingLevelMapEntrySupported('off', thinkingLevelMap), true);
  assert.equal(isPiThinkingLevelMapEntrySupported('minimal', thinkingLevelMap), false);
  assert.equal(isPiThinkingLevelMapEntrySupported('high', thinkingLevelMap), true);
  assert.equal(isPiThinkingLevelMapEntrySupported('xhigh', thinkingLevelMap), true);
  assert.equal(isPiThinkingLevelMapEntrySupported('max', thinkingLevelMap), false);
  assert.equal(isPiThinkingLevelMapEntrySupported('max', { max: 'max' }), true);
});
