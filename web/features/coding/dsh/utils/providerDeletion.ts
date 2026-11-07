import type { DshRuntimeProviderView } from '@/types/dsh';

/** Derive the same env-style ref for provider forms, backups, and deletion. */
export function credentialRefFromProviderKey(providerKey: string): string {
  const normalized = providerKey.trim().toUpperCase().replace(/[^A-Z0-9_]/g, '_');
  return normalized.endsWith('_API_KEY') ? normalized : `${normalized}_API_KEY`;
}

/**
 * Whether the user layer carries anything for this route.
 *
 * A route id that happens to be in the adapter catalog is not a fact about
 * ownership: a catalog route with an `llm-pi-ai.providers.<route>` entry (or a
 * stored credential) is user configuration and can be removed. Only a route
 * with no user-layer record at all has nothing to delete.
 */
export function hasDshProviderOwnedData(provider: DshRuntimeProviderView): boolean {
  // `provider.provider` is `undefined` when the route only exists as the
  // `agent-default-model` pointer. A stored key can carry a null value (a
  // hand-edited YAML entry), and that still counts as a user-layer record.
  return provider.credentialExists || provider.provider !== undefined;
}

export function canDeleteDshProvider(provider: DshRuntimeProviderView): boolean {
  return !provider.isDefault && hasDshProviderOwnedData(provider);
}

export function buildDshProviderDeletionPlan(
  providers: readonly DshRuntimeProviderView[],
  selectedIds: readonly string[],
) {
  const providersToDelete = providers.filter(
    (provider) => selectedIds.includes(provider.providerKey) && canDeleteDshProvider(provider),
  );
  const deletedProviderIds = new Set(providersToDelete.map((provider) => provider.providerKey));
  const credentialRef = (provider: DshRuntimeProviderView) => (
    provider.apiKeyEnv || credentialRefFromProviderKey(provider.providerKey)
  );
  const retainedCredentialRefs = new Set(
    providers.filter((provider) => !deletedProviderIds.has(provider.providerKey)).map(credentialRef),
  );
  const scheduledCredentialRefs = new Set<string>();

  return providersToDelete.map((provider) => {
    const refName = credentialRef(provider);
    const deleteCredential = provider.credentialExists
      && !retainedCredentialRefs.has(refName)
      && !scheduledCredentialRefs.has(refName);
    if (deleteCredential) scheduledCredentialRefs.add(refName);
    return {
      provider,
      credentialRef: refName,
      // Capture every backup before deletion changes a shared credential ref.
      credential: provider.credentialExists && provider.apiKey
        ? { refName, value: provider.apiKey }
        : undefined,
      deleteCredential,
    };
  });
}
