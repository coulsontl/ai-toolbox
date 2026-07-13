import type { GatewayCliTakeoverStatus } from '@/services';
import { parse as parseToml } from 'smol-toml';

export type GatewayApiFormat =
  | 'anthropic_messages'
  | 'openai_responses'
  | 'openai_chat'
  | 'gemini_native';

const normalizeFormatKey = (value: string) =>
  value.trim().toLowerCase().replace(/[/-]/g, '_');

export const normalizeGatewayApiFormat = (
  value?: string | null,
): GatewayApiFormat | null => {
  if (!value) {
    return null;
  }

  switch (normalizeFormatKey(value)) {
    case 'anthropic':
    case 'anthropic_messages':
    case 'claude':
    case 'claude_messages':
      return 'anthropic_messages';
    case 'openai_responses':
    case 'responses':
    case 'response':
      return 'openai_responses';
    case 'openai_chat':
    case 'chat_completions':
    case 'chat':
      return 'openai_chat';
    case 'gemini_native':
    case 'gemini':
      return 'gemini_native';
    default:
      return null;
  }
};

export const firstGatewayApiFormat = (
  ...values: Array<string | null | undefined>
): GatewayApiFormat | null => {
  for (const value of values) {
    const normalized = normalizeGatewayApiFormat(value);
    if (normalized) {
      return normalized;
    }
  }
  return null;
};

export const providerNeedsGatewayProxy = (
  targetFormat: string | null | undefined,
  nativeFormat: string,
) => {
  const normalizedTargetFormat = normalizeGatewayApiFormat(targetFormat);
  const normalizedNativeFormat = normalizeGatewayApiFormat(nativeFormat);
  return Boolean(
    normalizedTargetFormat &&
    normalizedNativeFormat &&
    normalizedTargetFormat !== normalizedNativeFormat,
  );
};

export const isGatewayConfigFlagEnabled = (value: unknown) => {
  if (typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'number') {
    return value !== 0;
  }
  if (typeof value === 'string') {
    const normalizedValue = value.trim().toLowerCase();
    return normalizedValue === 'true' ||
      normalizedValue === '1' ||
      normalizedValue === 'yes' ||
      normalizedValue === 'on';
  }
  return false;
};

export const canApplyProviderWithGatewayProxy = (
  status?: GatewayCliTakeoverStatus | null,
) => Boolean(status?.can_takeover);

export const codexWireApiFormatFromConfig = (config?: string | null) => {
  if (!config) {
    return null;
  }

  try {
    const parsed = parseToml(config) as Record<string, unknown>;
    const selectedProvider = typeof parsed.model_provider === 'string'
      ? parsed.model_provider.trim()
      : '';
    const modelProviders = parsed.model_providers && typeof parsed.model_providers === 'object'
      && !Array.isArray(parsed.model_providers)
      ? parsed.model_providers as Record<string, unknown>
      : undefined;
    const provider = selectedProvider && modelProviders?.[selectedProvider]
      && typeof modelProviders[selectedProvider] === 'object'
      && !Array.isArray(modelProviders[selectedProvider])
      ? modelProviders[selectedProvider] as Record<string, unknown>
      : undefined;
    const selectedValue = provider?.wire_api ?? provider?.api_format;
    if (typeof selectedValue === 'string' && selectedValue.trim()) {
      return selectedValue.trim();
    }
    const rootValue = parsed.wire_api ?? parsed.api_format;
    if (typeof rootValue === 'string' && rootValue.trim()) {
      return rootValue.trim();
    }
    for (const value of Object.values(modelProviders ?? {})) {
      if (!value || typeof value !== 'object' || Array.isArray(value)) continue;
      const providerValue = (value as Record<string, unknown>).wire_api
        ?? (value as Record<string, unknown>).api_format;
      if (typeof providerValue === 'string' && providerValue.trim()) {
        return providerValue.trim();
      }
    }
    return null;
  } catch {
    const match = config.match(/^\s*(?:wire_api|api_format)\s*=\s*["']([^"']+)["']/m);
    return match?.[1] ?? null;
  }
};

export const grokWireApiFormatFromConfig = (config?: string | null) => {
  if (!config) {
    return null;
  }

  try {
    const parsed = parseToml(config) as Record<string, unknown>;
    const models = parsed.models && typeof parsed.models === 'object' && !Array.isArray(parsed.models)
      ? parsed.models as Record<string, unknown>
      : undefined;
    const defaultModelKey = typeof models?.default === 'string' ? models.default.trim() : '';
    const modelTables = parsed.model && typeof parsed.model === 'object' && !Array.isArray(parsed.model)
      ? parsed.model as Record<string, unknown>
      : undefined;
    const selectedModel = defaultModelKey && modelTables?.[defaultModelKey]
      && typeof modelTables[defaultModelKey] === 'object' && !Array.isArray(modelTables[defaultModelKey])
      ? modelTables[defaultModelKey] as Record<string, unknown>
      : undefined;
    const selectedValue = selectedModel?.api_backend ?? selectedModel?.api_format;
    if (typeof selectedValue === 'string' && selectedValue.trim()) {
      return selectedValue.trim();
    }
    return null;
  } catch {
    const match = config.match(/^\s*(?:api_backend|api_format)\s*=\s*["']([^"']+)["']/m);
    return match?.[1] ?? null;
  }
};

export const openAiApiFormatFromBaseUrl = (baseUrl?: string | null) => {
  const normalizedBaseUrl = baseUrl?.trim().toLowerCase();
  if (!normalizedBaseUrl) {
    return null;
  }
  return normalizedBaseUrl.endsWith('/chat/completions') ||
    normalizedBaseUrl.includes('/chat/completions?')
    ? 'openai_chat'
    : null;
};
