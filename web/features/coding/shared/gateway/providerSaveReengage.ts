import type { GatewayAggregateNamingMode } from '@/services';
import {
  canReplaySubagentExposureSelection,
  isGatewayReengageMode,
  type GatewayReengageMode,
} from './gatewayAggregateConfig';
import {
  notifyGatewayAggregateConfigChanged,
  runGatewayAggregateMutation,
} from './gatewayAggregateMutation';

// Re-exported for the existing `shared/gateway` surface: the mode guard and its
// type live in `gatewayAggregateConfig` so the re-engage flow can depend on the
// aggregate helpers without creating an import cycle.
export { isGatewayReengageMode };
export type { GatewayReengageMode };

/** Aggregate routing config that must be replayed when re-engaging. */
export interface GatewayAggregateReengageConfig {
  providerIds: string[];
  separator: string;
  aliases?: Record<string, string>;
  naming?: GatewayAggregateNamingMode;
  /**
   * Whether the takeover may fail a request over to another selected site.
   *
   * `undefined` keeps the backend default, which refuses cross-site failover;
   * an engaged takeover must still replay its stored value so a provider save
   * cannot flip the safety switch.
   */
  crossSiteFailover?: boolean;
  /**
   * Bare upstream model names promoted into Codex's visible catalog. Other
   * names remain addressable as hidden aliases unless an identical site slug
   * already exists.
   *
   * `undefined` keeps the backend default, which publishes every bare model;
   * and because an empty array means the same thing, a narrowed set has to be
   * replayed explicitly or a provider save would silently widen the promoted
   * picker entries.
   */
  subagentExposedModels?: string[];
  /** Bare model names declared by the proposed post-save catalogs of selected sites. */
  proposedDeclaredBareModels?: string[];
  /**
   * Codex `[agents]` defaults the takeover owns.
   *
   * The re-engage round trip restores direct first, which drops these keys, so
   * they must be replayed here or a provider save would silently unset the
   * user's subagent default. `undefined` means the takeover owns no such key
   * and the user's own `[agents]` settings must be left alone.
   */
  subagentModel?: string;
  subagentReasoningEffort?: string;
}

export interface GatewayReengageSnapshot {
  gatewayMode: GatewayReengageMode;
  aggregateConfig?: GatewayAggregateReengageConfig | null;
}

interface SaveProviderWithGatewayReengageOptions<TResult, TStatus> {
  gatewayMode: GatewayReengageMode;
  saveProvider: () => Promise<TResult>;
  restoreDirect: () => Promise<TStatus>;
  engageSingle: () => Promise<TStatus>;
  engageFailover: () => Promise<TStatus>;
  /** Required when `gatewayMode` is `aggregate`; ignored otherwise. */
  engageAggregate?: (
    config: GatewayAggregateReengageConfig,
  ) => Promise<TStatus>;
  /** Aggregate selection to replay. Required when `gatewayMode` is `aggregate`. */
  aggregateConfig?: GatewayAggregateReengageConfig | null;
  /**
   * Reads the canonical takeover after this operation reaches the shared lane.
   * This prevents a provider save that waited behind a newer aggregate edit
   * from replaying an obsolete captured manifest.
   */
  resolveCurrentGatewayReengage?: () => Promise<GatewayReengageSnapshot>;
  onGatewayStatusChange?: (status: TStatus) => void;
}

export const saveProviderWithGatewayReengage = async <TResult, TStatus>({
  gatewayMode,
  saveProvider,
  restoreDirect,
  engageSingle,
  engageFailover,
  engageAggregate,
  aggregateConfig,
  resolveCurrentGatewayReengage,
  onGatewayStatusChange,
}: SaveProviderWithGatewayReengageOptions<TResult, TStatus>): Promise<TResult> => {
  if (!isGatewayReengageMode(gatewayMode)) {
    return saveProvider();
  }

  const saveAndReengage = async (): Promise<TResult> => {
    let effectiveGatewayMode: GatewayReengageMode = gatewayMode;
    let effectiveAggregateConfig = aggregateConfig;

    // Resolve immediately before the restore/save sequence, not when the form
    // opened. The lane may have waited behind a newer aggregate edit.
    if (gatewayMode === 'aggregate' && resolveCurrentGatewayReengage) {
      const current = await resolveCurrentGatewayReengage();
      effectiveGatewayMode = current.gatewayMode;
      effectiveAggregateConfig = current.aggregateConfig ?? null;
    }

    // Fail closed before touching the current provider config. Re-engaging an
    // aggregate takeover with no canonical site selection would drop the
    // cross-site model catalog and leave the UI/backend out of sync.
    if (effectiveGatewayMode === 'aggregate' && (!engageAggregate || !effectiveAggregateConfig)) {
      throw new Error('Aggregate gateway re-engage requires engageAggregate and aggregateConfig');
    }
    // Fail closed before `restoreDirect` below. The backend refuses selections
    // over the hint limit and bare names no longer declared by the proposed
    // provider catalogs. Replaying either can fail after restoreDirect drops the
    // takeover, so validate the exact post-save model universe first.
    if (
      effectiveGatewayMode === 'aggregate'
      && effectiveAggregateConfig
      && !canReplaySubagentExposureSelection(
        effectiveAggregateConfig.subagentExposedModels,
        effectiveAggregateConfig.proposedDeclaredBareModels,
      )
    ) {
      throw new Error(
        'Aggregate gateway re-engage cannot replay an exposure selection over the hint limit or containing undeclared models',
      );
    }
    if (!isGatewayReengageMode(effectiveGatewayMode)) {
      return saveProvider();
    }

    const directStatus = await restoreDirect();
    onGatewayStatusChange?.(directStatus);

    const result = await saveProvider();

    if (effectiveGatewayMode === 'aggregate') {
      // The check above keeps this invocation safe after awaited callbacks too.
      if (!engageAggregate || !effectiveAggregateConfig) {
        throw new Error('Aggregate gateway re-engage requires engageAggregate and aggregateConfig');
      }
      const { proposedDeclaredBareModels: _proposedDeclaredBareModels, ...replayConfig } =
        effectiveAggregateConfig;
      const aggregateStatus = await engageAggregate(replayConfig);
      onGatewayStatusChange?.(aggregateStatus);
      notifyGatewayAggregateConfigChanged();
      return result;
    }

    let nextStatus = await engageSingle();
    if (effectiveGatewayMode === 'failover') {
      nextStatus = await engageFailover();
    }
    onGatewayStatusChange?.(nextStatus);

    return result;
  };

  // Aggregate manifest/catalog writes must not interleave with aggregate editor
  // writes. Single/failover retain their existing behavior and do not enter
  // this lane because they do not rewrite the aggregate catalog.
  return gatewayMode === 'aggregate'
    ? runGatewayAggregateMutation(saveAndReengage)
    : saveAndReengage();
};
