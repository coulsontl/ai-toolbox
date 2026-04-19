/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

// Copy this file to a real `.test.ts` path under `web/test/`
// and update the relative import to mirror the source tree.
import { exampleFunction } from '../../../../features/featureName/exampleFunction.ts';

test('exampleFunction returns the expected value', () => {
  const result = exampleFunction('input');

  assert.equal(result, 'expected-output');
});
