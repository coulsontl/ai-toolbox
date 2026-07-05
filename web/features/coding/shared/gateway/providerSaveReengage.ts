export type GatewayReengageMode = 'single' | 'failover' | null | undefined;

interface SaveProviderWithGatewayReengageOptions<TResult, TStatus> {
  gatewayMode: GatewayReengageMode;
  saveProvider: () => Promise<TResult>;
  restoreDirect: () => Promise<TStatus>;
  engageSingle: () => Promise<TStatus>;
  engageFailover: () => Promise<TStatus>;
  onGatewayStatusChange?: (status: TStatus) => void;
}

export const isGatewayReengageMode = (
  gatewayMode: GatewayReengageMode,
): gatewayMode is 'single' | 'failover' =>
  gatewayMode === 'single' || gatewayMode === 'failover';

export const saveProviderWithGatewayReengage = async <TResult, TStatus>({
  gatewayMode,
  saveProvider,
  restoreDirect,
  engageSingle,
  engageFailover,
  onGatewayStatusChange,
}: SaveProviderWithGatewayReengageOptions<TResult, TStatus>): Promise<TResult> => {
  if (!isGatewayReengageMode(gatewayMode)) {
    return saveProvider();
  }

  const directStatus = await restoreDirect();
  onGatewayStatusChange?.(directStatus);

  const result = await saveProvider();

  let nextStatus = await engageSingle();
  if (gatewayMode === 'failover') {
    nextStatus = await engageFailover();
  }
  onGatewayStatusChange?.(nextStatus);

  return result;
};
