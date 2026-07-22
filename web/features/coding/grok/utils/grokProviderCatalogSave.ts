import {
  saveProviderWithGatewayReengage,
  type GatewayReengageMode,
} from '../../shared/gateway/providerSaveReengage';
import type { GrokProvider } from '../../../../types/grok';

interface SaveGrokProviderCatalogOptions<TStatus> {
  provider: GrokProvider;
  settingsConfig: string;
  gatewayMode: GatewayReengageMode;
  updateProvider: (provider: GrokProvider) => Promise<GrokProvider>;
  restoreDirect: () => Promise<TStatus>;
  engageSingle: () => Promise<TStatus>;
  engageFailover: () => Promise<TStatus>;
  onGatewayStatusChange?: (status: TStatus) => void;
}

export async function saveGrokProviderCatalogWithGatewayReengage<TStatus>({
  provider,
  settingsConfig,
  gatewayMode,
  updateProvider,
  restoreDirect,
  engageSingle,
  engageFailover,
  onGatewayStatusChange,
}: SaveGrokProviderCatalogOptions<TStatus>): Promise<GrokProvider> {
  const shouldReengageGateway = provider.isApplied
    && (gatewayMode === 'single' || gatewayMode === 'failover');

  return saveProviderWithGatewayReengage({
    gatewayMode: shouldReengageGateway ? gatewayMode : null,
    restoreDirect,
    engageSingle,
    engageFailover,
    onGatewayStatusChange,
    saveProvider: () => updateProvider({
      ...provider,
      settingsConfig,
    }),
  });
}
