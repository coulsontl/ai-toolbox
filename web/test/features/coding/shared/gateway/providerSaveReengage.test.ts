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

test('save provider reengage helper replays the aggregate selection, not single mode', async () => {
  const calls: string[] = [];

  const result = await saveProviderWithGatewayReengage({
    gatewayMode: 'aggregate',
    aggregateConfig: { providerIds: ['site-b', 'site-a'], separator: '.' },
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
    engageAggregate: async (config) => {
      calls.push(`aggregate:${config.providerIds.join(',')}:${config.separator}`);
      return 'aggregate';
    },
    onGatewayStatusChange: (status) => {
      calls.push(`status:${status}`);
    },
  });

  assert.equal(result, 'saved');
  // Aggregate must not fall through to single/failover: doing so would drop the
  // cross-site model list the user configured.
  assert.deepEqual(calls, [
    'restore',
    'status:direct',
    'save',
    'aggregate:site-b,site-a:.',
    'status:aggregate',
  ]);
});

test('save provider reengage helper refuses aggregate without a selection', async () => {
  const calls: string[] = [];

  await assert.rejects(
    saveProviderWithGatewayReengage({
      gatewayMode: 'aggregate',
      aggregateConfig: null,
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
      engageAggregate: async () => {
        calls.push('aggregate');
        return 'aggregate';
      },
      onGatewayStatusChange: (status) => {
        calls.push(`status:${status}`);
      },
    }),
    /Aggregate gateway re-engage requires/,
  );

  // Fail closed before touching the current provider config: an aggregate
  // re-engage without its canonical selection would otherwise drop routing.
  assert.deepEqual(calls, []);
});

test('save provider reengage helper still reengages aggregate when the save fails', async () => {
  const calls: string[] = [];

  await assert.rejects(
    saveProviderWithGatewayReengage({
      gatewayMode: 'aggregate',
      aggregateConfig: { providerIds: ['site-a'], separator: '::' },
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
      engageAggregate: async () => {
        calls.push('aggregate');
        return 'aggregate';
      },
      onGatewayStatusChange: (status) => {
        calls.push(`status:${status}`);
      },
    }),
    /save failed/,
  );

  assert.deepEqual(calls, ['restore', 'status:direct', 'save']);
});

test('save provider reengage helper refuses to restart a takeover it could never replay', async () => {
  const calls: string[] = [];
  const tooManyModels = ['m0', 'm1', 'm2', 'm3', 'm4', 'm5'];

  await assert.rejects(
    saveProviderWithGatewayReengage({
      gatewayMode: 'aggregate',
      aggregateConfig: {
        providerIds: ['site-a'],
        separator: '.',
        subagentExposedModels: tooManyModels,
      },
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
      engageAggregate: async () => {
        calls.push('aggregate');
        return 'aggregate';
      },
      onGatewayStatusChange: (status) => {
        calls.push(`status:${status}`);
      },
    }),
    /exposure selection larger than the subagent hint limit/,
  );

  // The whole point: `restoreDirect` already dropped the takeover, so the
  // unreplayable selection has to be caught before anything is touched.
  assert.deepEqual(calls, []);
});

test('save provider reengage helper still restarts an at-limit exposure selection', async () => {
  const calls: string[] = [];

  const result = await saveProviderWithGatewayReengage({
    gatewayMode: 'aggregate',
    aggregateConfig: {
      providerIds: ['site-a'],
      separator: '.',
      subagentExposedModels: ['m0', 'm1', 'm2', 'm3', 'm4'],
    },
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
    engageAggregate: async (config) => {
      calls.push(`aggregate:${(config.subagentExposedModels ?? []).length}`);
      return 'aggregate';
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
    'aggregate:5',
    'status:aggregate',
  ]);
});
