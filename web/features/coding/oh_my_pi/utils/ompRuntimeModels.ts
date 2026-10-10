import type { OmpRuntimeProviderView } from '../../../../types/ohMyPi.ts';
import { getProviderModelRecords } from '../../../../utils/ompModelMetadata.ts';

/** Read-only catalog view. YAML entries win; never use this list as a save payload. */
export const getOmpRuntimeModelRecords = (provider: OmpRuntimeProviderView) => {
  const configured = getProviderModelRecords(provider.modelsProvider);
  const configuredIds = new Set(configured.map(({ id }) => id));
  return [
    ...configured,
    ...getProviderModelRecords({ models: provider.runtimeModels }).filter(({ id }) => !configuredIds.has(id)),
  ];
};

export const getOmpRuntimeModelIds = (provider: OmpRuntimeProviderView): string[] => (
  Array.from(new Set([
    ...getOmpRuntimeModelRecords(provider).map(({ id }) => id),
    ...(provider.modelIds ?? []),
  ]))
);
