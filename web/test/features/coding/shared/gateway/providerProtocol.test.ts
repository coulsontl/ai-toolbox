import assert from 'node:assert/strict';
import test from 'node:test';

import { isGatewayConfigFlagEnabled } from '../../../../../features/coding/shared/gateway/providerProtocol.ts';

test('gateway config flag parser matches backend truthy compatibility values', () => {
  for (const value of [true, 1, 'true', '1', 'yes', 'on', ' YES ']) {
    assert.equal(isGatewayConfigFlagEnabled(value), true);
  }

  for (const value of [false, 0, 'false', '0', 'no', 'off', '', null, undefined]) {
    assert.equal(isGatewayConfigFlagEnabled(value), false);
  }
});
