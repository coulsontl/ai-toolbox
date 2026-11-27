import type { KimiProvider } from '@/types/kimi';
import {
  saveProviderWithGatewayReengage,
  type GatewayReengageMode,
} from '@/features/coding/shared/gateway';
import { shouldReengageKimiGatewayOnSave } from './providerSaveFlow';

interface SaveKimiProviderCatalogOptions<TStatus> {
  provider: KimiProvider;
  settingsConfig: string;
  /** Active gateway mode captured before the save; null when not taken over. */
  gatewayMode: GatewayReengageMode;
  updateProvider: (provider: KimiProvider) => Promise<KimiProvider>;
  restoreDirect: () => Promise<TStatus>;
  engageSingle: () => Promise<TStatus>;
  engageFailover: () => Promise<TStatus>;
  onGatewayStatusChange?: (status: TStatus) => void;
}

/**
 * Persist a Kimi provider with catalog edits, replaying an active gateway
 * takeover around the write.
 *
 * Kimi has no aggregate mode (that is Codex-only), so only single and failover
 * takeovers are replayed — passing an aggregate mode here would make the shared
 * helper reject the replay for a missing `engageAggregate`.
 */
export async function saveKimiProviderCatalogWithGatewayReengage<TStatus>({
  provider,
  settingsConfig,
  gatewayMode,
  updateProvider,
  restoreDirect,
  engageSingle,
  engageFailover,
  onGatewayStatusChange,
}: SaveKimiProviderCatalogOptions<TStatus>): Promise<KimiProvider> {
  // The predicate is owned by providerSaveFlow so the catalog path and the
  // provider-form path cannot drift apart.
  const shouldReengageGateway = shouldReengageKimiGatewayOnSave(provider, gatewayMode);

  return saveProviderWithGatewayReengage({
    gatewayMode: shouldReengageGateway ? gatewayMode : null,
    gatewayCliKey: 'kimi',
    restoreDirect,
    engageSingle,
    engageFailover,
    onGatewayStatusChange,
    saveProvider: () => updateProvider({ ...provider, settingsConfig }),
  });
}
