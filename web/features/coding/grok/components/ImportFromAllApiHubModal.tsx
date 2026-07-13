import React from 'react';
import { useTranslation } from 'react-i18next';
import type { ExternalProviderDisplayItem } from '@/components/common/ImportExternalProvidersModal/types';
import ImportFromAllApiHubModalBase from '@/features/coding/shared/allApiHub/ImportFromAllApiHubModal';
import type { AllApiHubProviderModelsState } from '@/features/coding/shared/allApiHubModelsCache';
import {
  listGrokAllApiHubProviders,
  resolveGrokAllApiHubProviders,
} from '@/services/grokApi';
import type { OpenCodeAllApiHubProvider } from '@/services/opencodeApi';
import type { OpenCodeProvider } from '@/types/opencode';

interface Props {
  open: boolean;
  existingProviderIds: string[];
  onCancel: () => void;
  onImport: (providers: OpenCodeAllApiHubProvider[]) => void;
}

const SUPPORTED_PROVIDER_TYPES = ['@ai-sdk/openai', '@ai-sdk/openai-compatible'];

const ImportFromAllApiHubModal: React.FC<Props> = ({
  open,
  existingProviderIds,
  onCancel,
  onImport,
}) => {
  const { t } = useTranslation();

  const texts = React.useMemo(
    () => ({
      title: t('common.allApiHub.importFromAllApiHub'),
      noProvidersText: t('common.allApiHub.noAllApiHubProviders'),
      cancelText: t('common.cancel'),
      importButtonText: t('common.allApiHub.importSelected'),
      selectAllText: t('common.allApiHub.selectAll'),
      deselectAllText: t('common.allApiHub.deselectAll'),
      existingTagText: t('common.allApiHub.alreadyExists'),
      noApiKeyTagText: t('common.allApiHub.apiKeyMissing'),
      disabledTagText: t('common.allApiHub.disabled'),
      balanceLabelText: t('common.allApiHub.balance'),
      modelsLabelText: t('common.allApiHub.models'),
      loadingModelsText: t('common.allApiHub.loadingModels'),
      emptyModelsText: t('common.allApiHub.emptyModels'),
      modelsErrorText: t('common.allApiHub.modelsLoadFailed'),
      unsupportedModelsText: t('common.allApiHub.unsupportedModels'),
      expandModelsText: t('common.allApiHub.expandModels'),
      collapseModelsText: t('common.allApiHub.collapseModels'),
      profileLabel: t('common.allApiHub.sourceProfile'),
      siteTypeLabel: t('common.allApiHub.siteType'),
      loadingTokenText: t('common.allApiHub.loadingApiKey'),
      tokenResolvedText: t('common.allApiHub.apiKeyReady'),
      retryResolveText: t('common.allApiHub.retryResolve'),
      searchPlaceholder: t('common.allApiHub.searchPlaceholder'),
      confirmTitle: t('common.allApiHub.importAllApiHubProtocolTitle'),
      confirmOkText: t('common.allApiHub.importAllApiHubReviewConfirm'),
    }),
    [t]
  );

  const mapProviderToItem = React.useCallback(
    (
      provider: OpenCodeAllApiHubProvider,
      modelState?: AllApiHubProviderModelsState
    ): ExternalProviderDisplayItem<OpenCodeProvider> => ({
      providerId: provider.providerId,
      name: provider.name,
      baseUrl: provider.baseUrl || undefined,
      accountLabel: provider.accountLabel,
      siteName: provider.siteName || undefined,
      siteType: provider.siteType || undefined,
      sourceProfileName: provider.sourceProfileName,
      sourceExtensionId: provider.sourceExtensionId,
      requiresBrowserOpen: provider.requiresBrowserOpen,
      isDisabled: provider.isDisabled,
      hasApiKey: provider.hasApiKey,
      apiKeyPreview: provider.apiKeyPreview,
      balanceUsd: provider.balanceUsd,
      balanceCny: provider.balanceCny,
      models: modelState?.models || [],
      modelsStatus: modelState?.status || 'idle',
      modelsError: modelState?.error,
      config: provider.providerConfig,
      secondaryLabel: provider.npm,
    }),
    []
  );

  const getConfirmSections = React.useCallback(
    (providers: OpenCodeAllApiHubProvider[]) =>
      [
        providers.filter((provider) => provider.npm === '@ai-sdk/openai-compatible').length > 0
          ? {
              description: t('common.allApiHub.importAllApiHubProtocolDesc'),
              providerNames: providers
                .filter((provider) => provider.npm === '@ai-sdk/openai-compatible')
                .map((provider) => provider.name),
            }
          : null,
        providers.filter((provider) => !provider.hasApiKey).length > 0
          ? {
              description: t('common.allApiHub.importAllApiHubMissingApiKeyDesc'),
              providerNames: providers
                .filter((provider) => !provider.hasApiKey)
                .map((provider) => provider.name),
            }
          : null,
      ].filter((section): section is { description: string; providerNames: string[] } => !!section),
    [t]
  );

  return (
    <ImportFromAllApiHubModalBase
      open={open}
      providerTypes={SUPPORTED_PROVIDER_TYPES}
      existingProviderIds={existingProviderIds}
      listProviders={listGrokAllApiHubProviders}
      resolveProviders={resolveGrokAllApiHubProviders}
      onCancel={onCancel}
      onImport={onImport}
      texts={texts}
      getProviderId={(provider) => provider.providerId}
      getProviderType={(provider) => provider.npm}
      mapProviderToItem={mapProviderToItem}
      getConfirmSections={getConfirmSections}
    />
  );
};

export default ImportFromAllApiHubModal;
