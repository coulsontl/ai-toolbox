import type { GrokCatalogModel } from '../../../../types/grok';

function normalizeStringArray(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }

  const items = value
    .map((item) => (typeof item === 'string' ? item.trim() : ''))
    .filter((item) => item.length > 0);

  return items.length > 0 ? items : undefined;
}

export function normalizeGrokCatalogModalities(value: unknown): GrokCatalogModel['modalities'] | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return undefined;
  }

  const modalities = value as { input?: unknown; output?: unknown };
  const input = normalizeStringArray(modalities.input);
  const output = normalizeStringArray(modalities.output);

  if (!input && !output) {
    return undefined;
  }

  return {
    ...(input ? { input } : {}),
    ...(output ? { output } : {}),
  };
}

export function normalizeGrokCatalogModels(models: GrokCatalogModel[]): GrokCatalogModel[] {
  const seenKeys = new Set<string>();
  const normalizedModels: GrokCatalogModel[] = [];

  for (const item of models) {
    const model = item.model.trim();
    const key = item.key?.trim() || model;
    if (!model || !key || seenKeys.has(key)) {
      continue;
    }
    seenKeys.add(key);

    const displayName = item.displayName?.trim();
    const rawContextWindow = String(item.contextWindow ?? '').replace(/[^\d]/g, '');
    const contextWindow = rawContextWindow ? Number.parseInt(rawContextWindow, 10) : undefined;
    const modalities = normalizeGrokCatalogModalities(item.modalities);

    normalizedModels.push({
      key,
      model,
      ...(displayName ? { displayName } : {}),
      ...(item.description?.trim() ? { description: item.description.trim() } : {}),
      ...(item.baseUrl?.trim() ? { baseUrl: item.baseUrl.trim() } : {}),
      ...(item.apiBackend?.trim() ? { apiBackend: item.apiBackend.trim() } : {}),
      ...(item.apiKey === null || typeof item.apiKey === 'string' ? { apiKey: item.apiKey } : {}),
      ...(item.envKey?.trim() ? { envKey: item.envKey.trim() } : {}),
      ...(contextWindow && contextWindow > 0 ? { contextWindow } : {}),
      ...(typeof item.maxCompletionTokens === 'number' ? { maxCompletionTokens: item.maxCompletionTokens } : {}),
      ...(typeof item.temperature === 'number' ? { temperature: item.temperature } : {}),
      ...(typeof item.topP === 'number' ? { topP: item.topP } : {}),
      ...(typeof item.supportsBackendSearch === 'boolean' ? { supportsBackendSearch: item.supportsBackendSearch } : {}),
      ...(typeof item.supportsReasoningEffort === 'boolean' ? { supportsReasoningEffort: item.supportsReasoningEffort } : {}),
      ...(item.reasoningEffort?.trim() ? { reasoningEffort: item.reasoningEffort.trim() } : {}),
      ...(typeof item.streamToolCalls === 'boolean' ? { streamToolCalls: item.streamToolCalls } : {}),
      ...(typeof item.maxRetries === 'number' ? { maxRetries: item.maxRetries } : {}),
      ...(typeof item.inferenceIdleTimeoutSecs === 'number' ? { inferenceIdleTimeoutSecs: item.inferenceIdleTimeoutSecs } : {}),
      ...(item.extraHeaders ? { extraHeaders: { ...item.extraHeaders } } : {}),
      ...(item.extraConfig ? { extraConfig: { ...item.extraConfig } } : {}),
      ...(typeof item.supportsImage === 'boolean' ? { supportsImage: item.supportsImage } : {}),
      ...(typeof item.vision === 'boolean' ? { vision: item.vision } : {}),
      ...(typeof item.attachment === 'boolean' ? { attachment: item.attachment } : {}),
      ...(modalities ? { modalities } : {}),
    });
  }

  return normalizedModels;
}
