import type { GrokProvider } from '@/types/grok';

export const GROK_LOCAL_PROVIDER_ID = '__local__';

export function isGrokLocalProviderId(providerId: string | null | undefined): boolean {
  return providerId === GROK_LOCAL_PROVIDER_ID;
}

export function shouldLoadGrokOfficialAccounts(provider: Pick<GrokProvider, 'id'>): boolean {
  return !isGrokLocalProviderId(provider.id);
}

export function shouldShowGrokOfficialAccounts(
  provider: Pick<GrokProvider, 'id' | 'category'>,
  officialAccountCount: number,
): boolean {
  return shouldLoadGrokOfficialAccounts(provider) && (
    provider.category === 'official' || officialAccountCount > 0
  );
}
