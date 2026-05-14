import type { OpenCodeDiagnosticsConfig, OpenCodeFavoriteProvider } from '@/services/opencodeApi';
import type { OpenCodeProvider } from '@/types/opencode';

export type FavoriteProviderSource = 'opencode' | 'claudecode' | 'codex' | 'openclaw';

export interface ClaudeFavoriteProviderPayload {
  name: string;
  category: string;
  settingsConfig: string;
  extraSettingsConfig?: string;
  notes?: string;
}

export interface CodexFavoriteProviderPayload {
  name: string;
  category: string;
  settingsConfig: string;
  notes?: string;
}

export interface OpenClawFavoriteProviderPayload {
  providerId: string;
  config: Record<string, unknown>;
}

const SOURCE_PREFIX_SEPARATOR = ':';
const STORAGE_KEY_PREFIX: Record<FavoriteProviderSource, string> = {
  opencode: 'opencode',
  claudecode: 'claudecode',
  codex: 'codex',
  openclaw: 'openclaw',
};
const SOURCE_PAYLOAD_KEY = '__aiToolboxSourcePayload';
const OPENCODE_STORAGE_PREFIX = `${STORAGE_KEY_PREFIX.opencode}${SOURCE_PREFIX_SEPARATOR}`;

function getStoragePrefix(source: FavoriteProviderSource): string {
  return `${STORAGE_KEY_PREFIX[source]}${SOURCE_PREFIX_SEPARATOR}`;
}

function startsWithKnownStoragePrefix(providerId: string): boolean {
  return Object.values(STORAGE_KEY_PREFIX).some((prefix) =>
    providerId.startsWith(`${prefix}${SOURCE_PREFIX_SEPARATOR}`),
  );
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function normalizeForStableSignature(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => normalizeForStableSignature(item));
  }

  if (isPlainObject(value)) {
    return Object.keys(value)
      .sort()
      .reduce<Record<string, unknown>>((result, key) => {
        const normalizedValue = normalizeForStableSignature(value[key]);
        if (normalizedValue !== undefined) {
          result[key] = normalizedValue;
        }
        return result;
      }, {});
  }

  if (typeof value === 'function' || typeof value === 'symbol' || value === undefined) {
    return undefined;
  }

  return value;
}

function stableSerialize(value: unknown): string {
  return JSON.stringify(normalizeForStableSignature(value)) ?? 'null';
}

export function buildFavoriteProviderStorageKey(
  source: FavoriteProviderSource,
  providerId: string,
): string {
  return `${getStoragePrefix(source)}${providerId}`;
}

export function extractFavoriteProviderRawId(
  source: FavoriteProviderSource,
  storageProviderId: string,
): string {
  const prefix = getStoragePrefix(source);
  if (storageProviderId.startsWith(prefix)) {
    return storageProviderId.slice(prefix.length);
  }

  if (source === 'opencode') {
    return storageProviderId;
  }

  return storageProviderId;
}

export function needsFavoriteProviderMigration(
  source: FavoriteProviderSource,
  providerId: string,
): boolean {
  return source === 'opencode' && !providerId.startsWith(OPENCODE_STORAGE_PREFIX) && !startsWithKnownStoragePrefix(providerId);
}

export function isFavoriteProviderForSource(
  source: FavoriteProviderSource,
  favoriteProvider: OpenCodeFavoriteProvider,
): boolean {
  const providerId = favoriteProvider.providerId;

  if (source === 'opencode') {
    return providerId.startsWith(getStoragePrefix('opencode')) || !startsWithKnownStoragePrefix(providerId);
  }

  return providerId.startsWith(getStoragePrefix(source));
}

export function buildFavoriteProviderOptions(
  provider: OpenCodeProvider,
  payload: unknown,
): OpenCodeProvider {
  return {
    ...provider,
    options: {
      ...(provider.options || {}),
      [SOURCE_PAYLOAD_KEY]: payload,
    },
  };
}

export function getFavoriteProviderPayload<T>(
  favoriteProvider: OpenCodeFavoriteProvider,
): T | null {
  const payload = favoriteProvider.providerConfig.options?.[SOURCE_PAYLOAD_KEY];
  return payload && typeof payload === 'object' ? (payload as T) : null;
}

export function mergeDiagnosticsIntoFavoriteProviders(
  previousProviders: OpenCodeFavoriteProvider[],
  nextProvider: OpenCodeFavoriteProvider,
  source: FavoriteProviderSource,
): OpenCodeFavoriteProvider[] {
  if (!isFavoriteProviderForSource(source, nextProvider)) {
    return previousProviders;
  }

  const targetStorageKey = nextProvider.providerId;
  const existingIndex = previousProviders.findIndex(
    (provider) => provider.providerId === targetStorageKey,
  );

  if (existingIndex >= 0) {
    const nextProviders = [...previousProviders];
    nextProviders[existingIndex] = nextProvider;
    return nextProviders;
  }

  return [...previousProviders, nextProvider];
}

export function dedupeFavoriteProvidersByPayload(
  favoriteProviders: OpenCodeFavoriteProvider[],
  currentStorageKeys: Set<string>,
): {
  keptProviders: OpenCodeFavoriteProvider[];
  duplicateIds: string[];
} {
  const providerBySignature = new Map<string, OpenCodeFavoriteProvider>();
  const duplicateIds: string[] = [];

  for (const favoriteProvider of favoriteProviders) {
    const payload = getFavoriteProviderPayload<Record<string, unknown>>(favoriteProvider);
    const signature = payload ? stableSerialize(payload) : favoriteProvider.providerId;
    const existingProvider = providerBySignature.get(signature);

    if (!existingProvider) {
      providerBySignature.set(signature, favoriteProvider);
      continue;
    }

    const existingIsCurrent = currentStorageKeys.has(existingProvider.providerId);
    const nextIsCurrent = currentStorageKeys.has(favoriteProvider.providerId);
    const shouldReplaceExisting =
      (!existingIsCurrent && nextIsCurrent) ||
      (existingIsCurrent === nextIsCurrent &&
        favoriteProvider.updatedAt > existingProvider.updatedAt);

    if (shouldReplaceExisting) {
      duplicateIds.push(existingProvider.providerId);
      providerBySignature.set(signature, favoriteProvider);
    } else {
      duplicateIds.push(favoriteProvider.providerId);
    }
  }

  return {
    keptProviders: Array.from(providerBySignature.values()),
    duplicateIds,
  };
}

export function dedupeOpenCodeFavoriteProviders(
  favoriteProviders: OpenCodeFavoriteProvider[],
  currentStorageKeys: Set<string>,
): {
  keptProviders: OpenCodeFavoriteProvider[];
  duplicateIds: string[];
} {
  const providerByRawId = new Map<string, OpenCodeFavoriteProvider>();
  const duplicateIds: string[] = [];

  for (const favoriteProvider of favoriteProviders) {
    const rawId = extractFavoriteProviderRawId('opencode', favoriteProvider.providerId);
    const existingProvider = providerByRawId.get(rawId);

    if (!existingProvider) {
      providerByRawId.set(rawId, favoriteProvider);
      continue;
    }

    const existingStorageKey = buildFavoriteProviderStorageKey('opencode', rawId);
    const existingIsCurrent = existingProvider.providerId === existingStorageKey;
    const nextIsCurrent = favoriteProvider.providerId === existingStorageKey;
    const existingMatchesConfig = currentStorageKeys.has(existingProvider.providerId);
    const nextMatchesConfig = currentStorageKeys.has(favoriteProvider.providerId);
    const shouldReplaceExisting =
      (!existingIsCurrent && nextIsCurrent) ||
      (existingIsCurrent === nextIsCurrent &&
        !existingMatchesConfig &&
        nextMatchesConfig) ||
      (existingIsCurrent === nextIsCurrent &&
        existingMatchesConfig === nextMatchesConfig &&
        favoriteProvider.updatedAt > existingProvider.updatedAt);

    if (shouldReplaceExisting) {
      duplicateIds.push(existingProvider.providerId);
      providerByRawId.set(rawId, favoriteProvider);
    } else {
      duplicateIds.push(favoriteProvider.providerId);
    }
  }

  return {
    keptProviders: Array.from(providerByRawId.values()),
    duplicateIds,
  };
}

export function findDiagnosticsForProvider(
  favoriteProviders: OpenCodeFavoriteProvider[],
  source: FavoriteProviderSource,
  providerId: string,
): OpenCodeDiagnosticsConfig | undefined {
  const storageKey = buildFavoriteProviderStorageKey(source, providerId);
  const matchedProviders = favoriteProviders.filter((provider) => {
    if (provider.providerId === storageKey) {
      return true;
    }

    return source === 'opencode' && extractFavoriteProviderRawId('opencode', provider.providerId) === providerId;
  });

  if (matchedProviders.length === 0) {
    return undefined;
  }

  matchedProviders.sort((left, right) => {
    const leftIsCurrent = left.providerId === storageKey;
    const rightIsCurrent = right.providerId === storageKey;
    if (leftIsCurrent !== rightIsCurrent) {
      return leftIsCurrent ? -1 : 1;
    }

    return right.updatedAt.localeCompare(left.updatedAt);
  });

  return matchedProviders[0]?.diagnostics;
}

export function findDefaultTestModelIdForProvider(
  favoriteProviders: OpenCodeFavoriteProvider[],
  source: FavoriteProviderSource,
  providerId: string,
): string | undefined {
  return findDiagnosticsForProvider(favoriteProviders, source, providerId)?.defaultTestModelId;
}
