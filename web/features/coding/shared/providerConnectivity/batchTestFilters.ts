interface DisableAwareCustomProvider {
  category?: string;
  isDisabled?: boolean;
}

export function getEnabledCustomProviderBatchCandidates<T extends DisableAwareCustomProvider>(
  providers: T[],
): T[] {
  return providers.filter((provider) => provider.category !== 'official' && !provider.isDisabled);
}

export function getEnabledProviderBatchEntries<T>(
  providerEntries: Array<[string, T]>,
  disabledProviderIds: ReadonlySet<string>,
): Array<[string, T]> {
  return providerEntries.filter(([providerId]) => !disabledProviderIds.has(providerId));
}
