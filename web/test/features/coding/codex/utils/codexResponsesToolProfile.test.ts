/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  codexResponsesToolProfileApplies,
  codexResponsesToolProfileOverride,
  defaultCodexResponsesToolProfile,
  readCodexResponsesToolProfile,
  resolveCodexResponsesToolProfile,
  withCodexResponsesToolProfile,
} from '../../../../../features/coding/codex/utils/codexResponsesToolProfile.ts';

test('defaultCodexResponsesToolProfile follows the preset and defaults to full', () => {
  assert.equal(defaultCodexResponsesToolProfile(undefined), 'full');
  assert.equal(defaultCodexResponsesToolProfile({}), 'full');
  assert.equal(
    defaultCodexResponsesToolProfile({ codexResponsesToolProfile: 'safe' }),
    'safe',
  );
  assert.equal(
    defaultCodexResponsesToolProfile({ codexResponsesToolProfile: 'full' }),
    'full',
  );
  // Same spellings the Rust resolver accepts.
  assert.equal(
    defaultCodexResponsesToolProfile({ codexResponsesToolProfile: 'native_safe' }),
    'safe',
  );
  assert.equal(
    defaultCodexResponsesToolProfile({ codex_responses_tool_profile: 'native' }),
    'full',
  );
});

test('resolveCodexResponsesToolProfile lets the explicit override win', () => {
  assert.equal(resolveCodexResponsesToolProfile(undefined, undefined), 'full');
  assert.equal(
    resolveCodexResponsesToolProfile(
      { codexResponsesToolProfile: 'safe' },
      undefined,
    ),
    'safe',
  );
  assert.equal(
    resolveCodexResponsesToolProfile(
      { codexResponsesToolProfile: 'full' },
      { codexResponsesToolProfile: 'safe' },
    ),
    'full',
  );
  assert.equal(readCodexResponsesToolProfile(undefined), undefined);
  assert.equal(readCodexResponsesToolProfile({}), undefined);
  assert.equal(
    readCodexResponsesToolProfile({ codexResponsesToolProfile: 'safe' }),
    'safe',
  );
  assert.equal(
    readCodexResponsesToolProfile({
      codex_responses_tool_profile: 'native-safe',
    }),
    'safe',
  );
});

test('withCodexResponsesToolProfile writes and clears only this key', () => {
  assert.deepEqual(
    withCodexResponsesToolProfile({ providerType: 'custom' }, 'safe'),
    { providerType: 'custom', codexResponsesToolProfile: 'safe' },
  );
  assert.deepEqual(
    withCodexResponsesToolProfile(
      { providerType: 'custom', codexResponsesToolProfile: 'safe' },
      undefined,
    ),
    { providerType: 'custom' },
  );
  assert.equal(withCodexResponsesToolProfile(undefined, undefined), undefined);
});

test('codexResponsesToolProfileOverride stores only a deviation from the preset', () => {
  // Preset defaults to safe: leaving it on is not an override.
  assert.equal(
    codexResponsesToolProfileOverride(true, { codexResponsesToolProfile: 'safe' }),
    undefined,
  );
  assert.equal(
    codexResponsesToolProfileOverride(false, { codexResponsesToolProfile: 'safe' }),
    'full',
  );
  // Custom/undeclared endpoints default to full.
  assert.equal(codexResponsesToolProfileOverride(false, undefined), undefined);
  assert.equal(codexResponsesToolProfileOverride(true, undefined), 'safe');
});

test('codexResponsesToolProfileApplies only matches direct native Responses providers', () => {
  assert.equal(codexResponsesToolProfileApplies(false, 'openai_responses'), true);
  assert.equal(codexResponsesToolProfileApplies(false, 'openai_chat'), false);
  assert.equal(
    codexResponsesToolProfileApplies(false, 'anthropic_messages'),
    false,
  );
  assert.equal(codexResponsesToolProfileApplies(true, 'openai_responses'), false);
});
