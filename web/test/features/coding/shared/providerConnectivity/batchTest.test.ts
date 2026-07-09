import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildProviderConnectivityBatchTarget,
  type ProviderConnectivityInfo,
} from '../../../../../features/coding/shared/providerConnectivity/batchTestTarget.ts';
import {
  getEnabledCustomProviderBatchCandidates,
  getEnabledProviderBatchEntries,
} from '../../../../../features/coding/shared/providerConnectivity/batchTestFilters.ts';

const errorMessages = {
  missingBaseUrl: 'Missing base URL',
  missingApiKey: 'Missing API key',
  missingModel: 'Missing model',
};

function createConnectivityInfo(overrides: Partial<ProviderConnectivityInfo> = {}): ProviderConnectivityInfo {
  return {
    providerId: 'provider-1',
    providerName: 'Provider 1',
    providerConfig: {
      npm: '@ai-sdk/openai-compatible',
      options: {
        baseURL: 'https://api.example.com/v1',
        apiKey: 'provider-key',
      },
    },
    modelIds: ['model-a', 'model-b'],
    ...overrides,
  };
}

test('getEnabledCustomProviderBatchCandidates skips official and disabled providers', () => {
  const providers = [
    { id: 'custom-enabled', category: 'custom', isDisabled: false },
    { id: 'custom-disabled', category: 'custom', isDisabled: true },
    { id: 'official-enabled', category: 'official', isDisabled: false },
    { id: 'official-disabled', category: 'official', isDisabled: true },
    { id: 'legacy-enabled' },
  ];

  assert.deepEqual(
    getEnabledCustomProviderBatchCandidates(providers).map((provider) => provider.id),
    ['custom-enabled', 'legacy-enabled'],
  );
});

test('getEnabledProviderBatchEntries skips entries listed in disabled provider ids', () => {
  const providerEntries: Array<[string, { name: string }]> = [
    ['enabled-a', { name: 'Enabled A' }],
    ['disabled', { name: 'Disabled' }],
    ['enabled-b', { name: 'Enabled B' }],
  ];

  assert.deepEqual(
    getEnabledProviderBatchEntries(providerEntries, new Set(['disabled'])).map(([providerId]) => providerId),
    ['enabled-a', 'enabled-b'],
  );
});

test('buildProviderConnectivityBatchTarget builds direct provider request by default', () => {
  const target = buildProviderConnectivityBatchTarget(createConnectivityInfo(), {
    preferredModelId: 'model-b',
    prompt: 'ping',
    timeoutSecs: 12,
    requireBaseUrl: true,
    requireApiKey: true,
    errorMessages,
  });

  assert.equal(target.providerId, 'provider-1');
  assert.equal(target.gatewayRequest, undefined);
  assert.deepEqual(target.request, {
    npm: '@ai-sdk/openai-compatible',
    providerId: 'provider-1',
    baseUrl: 'https://api.example.com/v1',
    apiKey: 'provider-key',
    prompt: 'ping',
    stream: true,
    modelIds: ['model-b'],
    timeoutSecs: 12,
  });
});

test('buildProviderConnectivityBatchTarget builds gateway request for converted providers', () => {
  const target = buildProviderConnectivityBatchTarget(createConnectivityInfo(), {
    preferredModelId: 'model-b',
    prompt: 'ping',
    timeoutSecs: 12,
    requireBaseUrl: false,
    requireApiKey: false,
    gatewayCliKey: 'claude',
    useGateway: true,
    errorMessages,
  });

  assert.equal(target.providerId, 'provider-1');
  assert.equal(target.request, undefined);
  assert.deepEqual(target.gatewayRequest, {
    cliKey: 'claude',
    providerId: 'provider-1',
    prompt: 'ping',
    stream: true,
    modelIds: ['model-b'],
    timeoutSecs: 12,
  });
});
