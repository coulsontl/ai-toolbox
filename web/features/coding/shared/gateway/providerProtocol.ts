import type { GatewayCliTakeoverStatus } from '@/services';

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

  const match = config.match(/^\s*(?:wire_api|api_format)\s*=\s*["']([^"']+)["']/m);
  return match?.[1] ?? null;
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
