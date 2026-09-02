/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import { formatFileSize, formatSpeed } from '../../../components/common/updateProgressFormat.ts';

test('formatFileSize formats byte counts into human-readable sizes', () => {
  assert.equal(formatFileSize(0), '0 B');
  assert.equal(formatFileSize(512), '512 B');
  assert.equal(formatFileSize(1024), '1 KB');
  assert.equal(formatFileSize(1536), '1.5 KB');
  assert.equal(formatFileSize(1048576), '1 MB');
  assert.equal(formatFileSize(1073741824), '1 GB');
});

test('formatSpeed formats byte-per-second rates into human-readable speeds', () => {
  assert.equal(formatSpeed(0), '0 B/s');
  assert.equal(formatSpeed(1024), '1 KB/s');
  assert.equal(formatSpeed(1048576), '1 MB/s');
  // Sizes array tops out at GB/s; ensure large values still render.
  assert.ok(formatSpeed(1073741824).includes('GB/s'));
});
