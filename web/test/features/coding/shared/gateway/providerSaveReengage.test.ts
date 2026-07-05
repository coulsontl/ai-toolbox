import assert from 'node:assert/strict';
import test from 'node:test';

import { saveProviderWithGatewayReengage } from '../../../../../features/coding/shared/gateway/providerSaveReengage.ts';

test('save provider reengage helper saves directly when gateway mode is inactive', async () => {
  const calls: string[] = [];

  const result = await saveProviderWithGatewayReengage({
    gatewayMode: null,
    saveProvider: async () => {
      calls.push('save');
      return 'saved';
    },
    restoreDirect: async () => {
      calls.push('restore');
      return 'direct';
    },
    engageSingle: async () => {
      calls.push('single');
      return 'single';
    },
    engageFailover: async () => {
      calls.push('failover');
      return 'failover';
    },
    onGatewayStatusChange: (status) => {
      calls.push(`status:${status}`);
    },
  });

  assert.equal(result, 'saved');
  assert.deepEqual(calls, ['save']);
});

test('save provider reengage helper restores direct before saving and reengages single mode', async () => {
  const calls: string[] = [];

  const result = await saveProviderWithGatewayReengage({
    gatewayMode: 'single',
    saveProvider: async () => {
      calls.push('save');
      return 'saved';
    },
    restoreDirect: async () => {
      calls.push('restore');
      return 'direct';
    },
    engageSingle: async () => {
      calls.push('single');
      return 'single';
    },
    engageFailover: async () => {
      calls.push('failover');
      return 'failover';
    },
    onGatewayStatusChange: (status) => {
      calls.push(`status:${status}`);
    },
  });

  assert.equal(result, 'saved');
  assert.deepEqual(calls, ['restore', 'status:direct', 'save', 'single', 'status:single']);
});

test('save provider reengage helper restores direct before saving and reengages failover mode', async () => {
  const calls: string[] = [];

  const result = await saveProviderWithGatewayReengage({
    gatewayMode: 'failover',
    saveProvider: async () => {
      calls.push('save');
      return 'saved';
    },
    restoreDirect: async () => {
      calls.push('restore');
      return 'direct';
    },
    engageSingle: async () => {
      calls.push('single');
      return 'single';
    },
    engageFailover: async () => {
      calls.push('failover');
      return 'failover';
    },
    onGatewayStatusChange: (status) => {
      calls.push(`status:${status}`);
    },
  });

  assert.equal(result, 'saved');
  assert.deepEqual(calls, [
    'restore',
    'status:direct',
    'save',
    'single',
    'failover',
    'status:failover',
  ]);
});

test('save provider reengage helper does not reengage when save fails after restore', async () => {
  const calls: string[] = [];

  await assert.rejects(
    saveProviderWithGatewayReengage({
      gatewayMode: 'single',
      saveProvider: async () => {
        calls.push('save');
        throw new Error('save failed');
      },
      restoreDirect: async () => {
        calls.push('restore');
        return 'direct';
      },
      engageSingle: async () => {
        calls.push('single');
        return 'single';
      },
      engageFailover: async () => {
        calls.push('failover');
        return 'failover';
      },
      onGatewayStatusChange: (status) => {
        calls.push(`status:${status}`);
      },
    }),
    /save failed/,
  );

  assert.deepEqual(calls, ['restore', 'status:direct', 'save']);
});
