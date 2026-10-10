import type { OmpRuntimeProviderView } from '../../../../types/ohMyPi.ts';

export const OMP_CODEX_PROVIDER_KEY = 'openai-codex';

/** Native OAuth has its own entry; never infer usable API-key credentials from it. */
export const isOmpCodexSubscriptionProvider = (provider: OmpRuntimeProviderView): boolean => (
  provider.providerKey === OMP_CODEX_PROVIDER_KEY
);
