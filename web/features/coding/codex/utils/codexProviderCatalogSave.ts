import { parse as parseToml } from 'smol-toml';
import {
  saveProviderWithGatewayReengage,
  type GatewayAggregateReengageConfig,
  type GatewayReengageMode,
} from '../../shared/gateway/providerSaveReengage';
import type { CodexProvider } from '../../../../types/codex';
import {
  normalizeSubagentExposedModels,
} from '../../shared/gateway/gatewayAggregateConfig';
import {
  normalizeCodexAutoReviewModelOverride,
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
 * The aggregate catalog uses TOML's top-level `model`, not `[chat].model`.
 * Parse TOML instead of using extractCodexModel, which intentionally supports
 * the legacy chat-section form for the provider editor.
 */
function extractCodexTopLevelModelForAggregate(configText: unknown): string | undefined {
  if (typeof configText !== 'string' || !configText.trim()) {
    return undefined;
  }

  try {
    const config = parseToml(configText) as Record<string, unknown>;
    const model = config.model;
    return typeof model === 'string' && model.trim() ? model.trim() : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Match the backend's top-level alias precedence while retaining support for
 * legacy per-model override values handled by resolveCodexAutoReviewModelOverride.
 */
function resolveCodexAggregateAutoReviewModelOverride(
  settings: ReturnType<typeof parseCodexSettingsConfig>,
): string | undefined {
  const settingsObject = settings as typeof settings & {
    auto_review_model_override?: unknown;
  };
  const topLevelValue = Object.prototype.hasOwnProperty.call(settings, 'autoReviewModelOverride')
    ? settings.autoReviewModelOverride
    : settingsObject.auto_review_model_override;
  const topLevel = normalizeCodexAutoReviewModelOverride(topLevelValue);
  if (topLevel) {
    return topLevel;
  }

  return resolveCodexAutoReviewModelOverride(settings);
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
    const selectedProvider = providerById.get(providerId);
    if (providerId !== proposedProvider.id && !selectedProvider) {
      throw new Error(
        `Aggregate gateway preflight cannot resolve selected Codex provider '${providerId}'`,
      );
    }
    const settingsConfig = providerId === proposedProvider.id
      ? proposedProvider.settingsConfig
      : selectedProvider!.settingsConfig;

    const settings = parseCodexSettingsConfig(settingsConfig);
    const rawMappedModels: unknown = settings.modelCatalog?.models;
    const mappedModels = Array.isArray(rawMappedModels)
      ? rawMappedModels.flatMap((item: unknown) => {
        if (!item || typeof item !== 'object' || Array.isArray(item)) {
          return [];
        }
        const model = (item as Record<string, unknown>).model;
        return typeof model === 'string' ? [model] : [];
      })
      : [];
    const defaultModel = extractCodexTopLevelModelForAggregate(settings.config)
      ?? resolveCodexAggregateAutoReviewModelOverride(settings);
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
