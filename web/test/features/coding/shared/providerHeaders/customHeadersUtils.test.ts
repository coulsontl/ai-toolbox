import assert from 'node:assert/strict';
import test from 'node:test';

import {
  emptyHeaderEntry,
  getCustomHeadersFromMeta,
  getEffectiveCustomHeaders,
  getEffectiveCustomHeadersFromMeta,
  mergeCustomHeadersIntoMeta,
} from '../../../../../features/coding/shared/providerHeaders/customHeadersUtils.ts';

test('getCustomHeadersFromMeta reports enabled when a non-empty array is present', () => {
  assert.deepEqual(
    getCustomHeadersFromMeta({
      customHeaders: [{ op: 'set', name: 'User-Agent', value: 'claude-cli/2.1.161', from: '', to: '' }],
    }),
    {
      enabled: true,
      headers: [{ op: 'set', name: 'User-Agent', value: 'claude-cli/2.1.161', from: '', to: '' }],
    },
  );
});
test('getCustomHeadersFromMeta surfaces legacy customUserAgent as a set row', () => {
  assert.deepEqual(
    getCustomHeadersFromMeta({ customUserAgent: 'claude-cli/2.1.161' }),
    {
      enabled: true,
      headers: [{ op: 'set', name: 'User-Agent', value: 'claude-cli/2.1.161', from: '', to: '' }],
    },
  );
});

test('getCustomHeadersFromMeta surfaces legacy snake custom_user_agent', () => {
  const state = getCustomHeadersFromMeta({ custom_user_agent: 'Kilo-Code/1.0' } as never);
  assert.equal(state.enabled, true);
  assert.deepEqual(state.headers, [
    { op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' },
  ]);
});

test('getCustomHeadersFromMeta reads snake custom_headers array', () => {
  const state = getCustomHeadersFromMeta({
    custom_headers: [{ op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' }],
  } as never);
  assert.equal(state.enabled, true);
  assert.deepEqual(state.headers, [
    { op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' },
  ]);
});

test('getCustomHeadersFromMeta reports disabled and seeds a blank row when unset', () => {
  const fromUndefined = getCustomHeadersFromMeta(undefined);
  assert.equal(fromUndefined.enabled, false);
  assert.equal(fromUndefined.headers.length, 1);
  assert.deepEqual(fromUndefined.headers[0], emptyHeaderEntry());

  const fromEmpty = getCustomHeadersFromMeta({ customHeaders: [] });
  assert.equal(fromEmpty.enabled, false);
  assert.equal(fromEmpty.headers.length, 1);
});

test('getCustomHeadersFromMeta normalizes unknown ops back to set', () => {
  const { headers } = getCustomHeadersFromMeta({
    customHeaders: [{ op: 'bogus', name: 'X-Foo', value: 'bar', from: '', to: '' } as never],
  });
  assert.equal(headers[0].op, 'set');
});

test('mergeCustomHeadersIntoMeta writes trimmed set rows when enabled', () => {
  assert.deepEqual(
    mergeCustomHeadersIntoMeta(undefined, {
      enabled: true,
      headers: [{ op: 'set', name: '  User-Agent  ', value: '  claude-cli/1.0  ', from: '', to: '' }],
    }),
    { customHeaders: [{ op: 'set', name: 'User-Agent', value: 'claude-cli/1.0', from: '', to: '' }] },
  );
});

test('mergeCustomHeadersIntoMeta removes legacy customUserAgent', () => {
  assert.deepEqual(
    mergeCustomHeadersIntoMeta(
      { providerType: 'anthropic', customUserAgent: 'claude-cli/2.1.161' } as never,
      { enabled: false, headers: [] },
    ),
    { providerType: 'anthropic' },
  );
});

test('mergeCustomHeadersIntoMeta removes legacy snake custom_user_agent', () => {
  assert.equal(
    mergeCustomHeadersIntoMeta(
      { custom_user_agent: 'Kilo-Code/1.0' } as never,
      { enabled: false, headers: [] },
    ),
    undefined,
  );
});

test('mergeCustomHeadersIntoMeta replaces legacy customUserAgent with customHeaders when enabled', () => {
  assert.deepEqual(
    mergeCustomHeadersIntoMeta(
      { providerType: 'anthropic', customUserAgent: 'claude-cli/2.1.161' } as never,
      { enabled: true, headers: [{ op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' }] },
    ),
    { providerType: 'anthropic', customHeaders: [{ op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' }] },
  );
});

test('mergeCustomHeadersIntoMeta clears the key when disabled', () => {
  assert.deepEqual(
    mergeCustomHeadersIntoMeta(
      { providerType: 'anthropic', customHeaders: [{ op: 'set', name: 'X', value: '1', from: '', to: '' }] } as never,
      { enabled: false, headers: [] },
    ),
    { providerType: 'anthropic' },
  );
});

test('mergeCustomHeadersIntoMeta drops meaningless rows (set without value)', () => {
  assert.equal(
    mergeCustomHeadersIntoMeta(
      { customHeaders: [{ op: 'set', name: 'X', value: '', from: '', to: '' }] },
      { enabled: true, headers: [{ op: 'set', name: 'X', value: '', from: '', to: '' }] },
    ),
    undefined,
  );
});

test('mergeCustomHeadersIntoMeta keeps rename rows with from/to', () => {
  assert.deepEqual(
    mergeCustomHeadersIntoMeta(undefined, {
      enabled: true,
      headers: [{ op: 'rename', name: '', value: '', from: 'X-Old', to: 'X-New' }],
    }),
    { customHeaders: [{ op: 'rename', name: '', value: '', from: 'X-Old', to: 'X-New' }] },
  );
});

test('mergeCustomHeadersIntoMeta preserves unrelated meta fields', () => {
  assert.deepEqual(
    mergeCustomHeadersIntoMeta(
      { providerType: 'openrouter', costMultiplier: '0.5' } as never,
      {
        enabled: true,
        headers: [{ op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' }],
      },
    ),
    {
      providerType: 'openrouter',
      costMultiplier: '0.5',
      customHeaders: [{ op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' }],
    },
  );
});

test('getEffectiveCustomHeaders returns an empty result when disabled or empty', () => {
  assert.equal(getEffectiveCustomHeaders(undefined), undefined);
  assert.equal(getEffectiveCustomHeaders({ enabled: false, headers: [
    { op: 'set', name: 'User-Agent', value: 'Kilo-Code/1.0', from: '', to: '' },
  ] }), undefined);
  assert.equal(getEffectiveCustomHeaders({ enabled: true, headers: [emptyHeaderEntry()] }), undefined);
});

test('getEffectiveCustomHeaders flattens trimmed set rows into a request-header map', () => {
  assert.deepEqual(
    getEffectiveCustomHeaders({
      enabled: true,
      headers: [
        { op: 'set', name: ' User-Agent ', value: ' claude-cli/2.1.237 ', from: '', to: '' },
        { op: 'set', name: 'x-api-key', value: 'sk-test', from: '', to: '' },
      ],
    }),
    {
      'User-Agent': 'claude-cli/2.1.237',
      'x-api-key': 'sk-test',
    },
  );
});

test('getEffectiveCustomHeaders keeps the last set row per header name case-insensitively', () => {
  assert.deepEqual(
    getEffectiveCustomHeaders({
      enabled: true,
      headers: [
        { op: 'set', name: 'User-Agent', value: 'first-agent', from: '', to: '' },
        { op: 'set', name: 'user-agent', value: 'second-agent', from: '', to: '' },
      ],
    }),
    { 'user-agent': 'second-agent' },
  );
});

test('getEffectiveCustomHeaders ignores non-set ops and invalid rows', () => {
  assert.deepEqual(
    getEffectiveCustomHeaders({
      enabled: true,
      headers: [
        { op: 'delete', name: 'Authorization', value: '', from: '', to: '' },
        { op: 'rename', name: '', value: '', from: 'X-Old', to: 'X-New' },
        { op: 'copy', name: '', value: '', from: 'X-Source', to: 'X-Target' },
        { op: 'set', name: 'Bad Header', value: 'value', from: '', to: '' },
        { op: 'set', name: 'X-Empty', value: '   ', from: '', to: '' },
        { op: 'set', name: 'X-Good', value: 'value', from: '', to: '' },
      ],
    }),
    { 'X-Good': 'value' },
  );
});

test('getEffectiveCustomHeadersFromMeta reads saved rows and the legacy customUserAgent', () => {
  assert.deepEqual(
    getEffectiveCustomHeadersFromMeta({
      customHeaders: [{ op: 'set', name: 'User-Agent', value: 'claude-cli/2.1.237', from: '', to: '' }],
    }),
    { 'User-Agent': 'claude-cli/2.1.237' },
  );
  assert.deepEqual(
    getEffectiveCustomHeadersFromMeta({ customUserAgent: 'Kilo-Code/1.0' }),
    { 'User-Agent': 'Kilo-Code/1.0' },
  );
  assert.equal(getEffectiveCustomHeadersFromMeta(undefined), undefined);
});
