import type { OmpRuntimeConfig, OmpRuntimeProviderView } from '../../../../types/ohMyPi.ts';
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

export interface OmpRuntimeCatalog {
  rootPath: string;
  models: Record<string, unknown>[];
  error: string | null;
}

/** Derive the catalog view from the latest local snapshot; never modify save payloads. */
export const mergeOmpRuntimeCatalog = (
  config: OmpRuntimeConfig,
  catalog: OmpRuntimeCatalog | null,
): OmpRuntimeConfig => {
  const currentCatalog = catalog?.rootPath === config.rootPathInfo.path ? catalog : null;
  return {
    ...config,
    providers: config.providers.map((provider) => {
      if (provider.providerKey !== 'openai-codex') return provider;
      const runtimeModels = currentCatalog?.models ?? provider.runtimeModels ?? [];
      const modelIds = getOmpRuntimeModelRecords({ ...provider, runtimeModels }).map(({ id }) => id);
      const warnings: NonNullable<OmpRuntimeProviderView['warnings']> =
        (provider.warnings ?? []).filter((warning) => warning !== 'missing_model');
      const defaultModel = config.modelSettings.modelId;
      if (config.modelSettings.providerKey === provider.providerKey
        && defaultModel?.trim() && modelIds.length > 0 && !modelIds.includes(defaultModel)) {
        warnings.push('missing_model');
      }
      return {
        ...provider,
        runtimeModels,
        runtimeCatalogError: currentCatalog ? currentCatalog.error : provider.runtimeCatalogError ?? null,
        modelIds,
        warnings,
      };
    }),
  };
};
