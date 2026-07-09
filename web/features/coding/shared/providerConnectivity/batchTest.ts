import type { ProviderConnectivityStatusItem } from '@/components/common/ProviderCard/types';
import { testProviderModelConnectivity } from '@/services/opencodeApi';
import { testGatewayProviderModelConnectivity } from '@/services/proxyGatewayApi';
export {
  buildProviderConnectivityBatchTarget,
  type ProviderConnectivityBatchTarget,
  type ProviderConnectivityInfo,
} from './batchTestTarget';
import type { ProviderConnectivityBatchTarget } from './batchTestTarget';

export const CONNECTIVITY_BATCH_CONCURRENCY = 5;

export async function probeProviderConnectivity(
  target: ProviderConnectivityBatchTarget,
): Promise<ProviderConnectivityStatusItem> {
  if (!target.request && !target.gatewayRequest) {
    return {
      status: 'error',
      errorMessage: target.errorMessage,
    };
  }

  try {
    const response = target.gatewayRequest
      ? await testGatewayProviderModelConnectivity(target.gatewayRequest)
      : await testProviderModelConnectivity(target.request!);
    const result = response.results[0];

    if (!result) {
      return {
        status: 'error',
        errorMessage: target.errorMessage || 'No test result returned',
      };
    }

    if (result.status === 'success') {
      return {
        status: 'success',
        modelId: result.modelId,
        totalMs: result.totalMs,
      };
    }

    return {
      status: 'error',
      modelId: result.modelId,
      totalMs: result.totalMs,
      errorMessage: result.errorMessage || `Connectivity test failed: ${result.status}`,
    };
  } catch (error) {
    return {
      status: 'error',
      errorMessage: error instanceof Error ? error.message : String(error),
    };
  }
}

export async function runProviderConnectivityBatch(
  targets: ProviderConnectivityBatchTarget[],
  onUpdate: (providerId: string, status: ProviderConnectivityStatusItem) => void,
  concurrency: number = CONNECTIVITY_BATCH_CONCURRENCY,
): Promise<void> {
  let currentIndex = 0;

  const worker = async () => {
    while (currentIndex < targets.length) {
      const target = targets[currentIndex];
      currentIndex += 1;

      const status = await probeProviderConnectivity(target);
      onUpdate(target.providerId, status);
    }
  };

  const workerCount = Math.min(Math.max(concurrency, 1), targets.length);
  await Promise.all(Array.from({ length: workerCount }, () => worker()));
}
