/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  extractGrokBaseUrl,
  extractGrokModel,
  isGrokPrivacyProtectionEnabled,
  setGrokBaseUrl,
  setGrokModel,
  setGrokPrivacyProtection,
} from '../../../../../utils/grokConfigUtils.ts';

test('Grok model helpers use official models and model tables', () => {
  const withModel = setGrokModel('[ui]\nsimple_mode = true', 'custom-model');
  const withBaseUrl = setGrokBaseUrl(withModel, 'https://example.com/v1');

  assert.equal(extractGrokModel(withBaseUrl), 'custom-model');
  assert.equal(extractGrokBaseUrl(withBaseUrl), 'https://example.com/v1');
  assert.match(withBaseUrl, /\[models\]\ndefault = "custom-model"/);
  assert.match(withBaseUrl, /\[model\.custom-model\]\nbase_url = "https:\/\/example\.com\/v1"/);
  assert.doesNotMatch(withBaseUrl, /model_provider|model_providers|\[chat\]/);
});

test('Grok privacy protection adds all official privacy fields', () => {
  const nextConfig = setGrokPrivacyProtection('[features]\nfeedback = true\n', true);

  assert.equal(isGrokPrivacyProtectionEnabled(nextConfig), true);
  assert.match(nextConfig, /\[features\][\s\S]*telemetry = false/);
  assert.match(nextConfig, /\[features\][\s\S]*codebase_indexing = false/);
  assert.match(nextConfig, /\[telemetry\]\ntrace_upload = false/);
  assert.match(nextConfig, /\[harness\]\ndisable_codebase_upload = true/);
  assert.match(nextConfig, /feedback = true/);
});

test('Grok privacy protection removes only shortcut-owned matching values', () => {
  const config = [
    '[features]',
    'feedback = true',
    'telemetry = false',
    'codebase_indexing = false',
    '',
    '[telemetry]',
    'trace_upload = false',
    'custom = true',
    '',
    '[harness]',
    'disable_codebase_upload = true',
    'sandbox = "workspace"',
  ].join('\n');

  const nextConfig = setGrokPrivacyProtection(config, false);

  assert.equal(isGrokPrivacyProtectionEnabled(nextConfig), false);
  assert.match(nextConfig, /\[features\]\nfeedback = true/);
  assert.match(nextConfig, /\[telemetry\]\ncustom = true/);
  assert.match(nextConfig, /\[harness\]\nsandbox = "workspace"/);
  assert.doesNotMatch(nextConfig, /codebase_indexing|trace_upload|disable_codebase_upload/);
});
