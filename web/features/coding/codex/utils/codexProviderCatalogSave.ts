import {
  saveProviderWithGatewayReengage,
  type GatewayAggregateReengageConfig,
  type GatewayReengageMode,
} from '../../shared/gateway/providerSaveReengage';
import type { CodexProvider } from '../../../../types/codex';
import { extractCodexModel } from '../../../../utils/codexConfigUtils';
import {
  normalizeSubagentExposedModels,
} from '../../shared/gateway/gatewayAggregateConfig';
import {
  parseCodexSettingsConfig,
  resolveCodexAutoReviewModelOverride,
} from './codexSettingsConfig';

interface SaveCodexProviderCatalogOptions<TStatus> {
  provider: CodexProvider;
  providers: readonly CodexProvider[];
  settingsConfig: string;
  gatewayMode: GatewayReengageMode;
  /** Required when `gatewayMode` is `aggregate`; ignored otherwise. */
  aggregateConfig?: GatewayAggregateReengageConfig | null;
  updateProvider: (provider: CodexProvider) => Promise<CodexProvider>;
  restoreDirect: () => Promise<TStatus>;
  engageSingle: () => Promise<TStatus>;
  engageFailover: () => Promise<TStatus>;
  engageAggregate?: (config: GatewayAggregateReengageConfig) => Promise<TStatus>;
  onGatewayStatusChange?: (status: TStatus) => void;
}

/**
 * Build the declared bare-model universe that an aggregate re-engage would see
 * after replacing one provider with its proposed settings. This mirrors the
 * Codex catalog inputs: mapped models plus the config default, falling back to
 * the auto-review model only when the config has no default.
 */
export function withProposedCodexAggregateModels(
  aggregateConfig: GatewayAggregateReengageConfig | null | undefined,
  providers: readonly CodexProvider[],
  proposedProvider: Pick<CodexProvider, 'id' | 'settingsConfig'>,
): GatewayAggregateReengageConfig | null {
  if (!aggregateConfig) {
    return null;
  }

  const providerById = new Map(providers.map((provider) => [provider.id, provider]));
  const declaredModels = aggregateConfig.providerIds.flatMap((providerId) => {
    const settingsConfig = providerId === proposedProvider.id
      ? proposedProvider.settingsConfig
      : providerById.get(providerId)?.settingsConfig;
    if (settingsConfig === undefined) {
      return [];
    }

    const settings = parseCodexSettingsConfig(settingsConfig);
    const mappedModels = settings.modelCatalog?.models?.map((item) => item.model) ?? [];
    const defaultModel = extractCodexModel(settings.config)
      ?? resolveCodexAutoReviewModelOverride(settings);
    return [...mappedModels, defaultModel ?? ''];
  });

  return {
    ...aggregateConfig,
    proposedDeclaredBareModels: normalizeSubagentExposedModels(declaredModels),
  };
}

/**
 * Persist a Codex provider with catalog edits and replay any active gateway
 * takeover. Codex has a four-state gateway (direct -> single -> failover ->
 * aggregate), so unlike Grok this must also replay an aggregate takeover or the
 * cross-site catalog would be dropped by the restore-direct round trip.
 */
export async function saveCodexProviderCatalogWithGatewayReengage<TStatus>({
  provider,
  providers,
  settingsConfig,
  gatewayMode,
  aggregateConfig,
  updateProvider,
  restoreDirect,
  engageSingle,
  engageFailover,
  engageAggregate,
  onGatewayStatusChange,
}: SaveCodexProviderCatalogOptions<TStatus>): Promise<CodexProvider> {
  const shouldReengageGateway = provider.isApplied
    && (gatewayMode === 'single' || gatewayMode === 'failover' || gatewayMode === 'aggregate');
  const proposedAggregateConfig = shouldReengageGateway && gatewayMode === 'aggregate'
    ? withProposedCodexAggregateModels(aggregateConfig, providers, {
      id: provider.id,
      settingsConfig,
    })
    : null;

  return saveProviderWithGatewayReengage({
    gatewayMode: shouldReengageGateway ? gatewayMode : null,
    aggregateConfig: proposedAggregateConfig,
    restoreDirect,
    engageSingle,
    engageFailover,
    engageAggregate,
    onGatewayStatusChange,
    saveProvider: () => updateProvider({
      ...provider,
      settingsConfig,
    }),
  });
}
