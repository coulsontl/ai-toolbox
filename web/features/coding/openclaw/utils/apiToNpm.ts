import type { OpenClawProviderConfig } from '@/types/openclaw';
import type { OpenCodeProvider } from '@/types/opencode';

const OPENAI_COMPATIBLE_NPM = '@ai-sdk/openai-compatible';
const OPENAI_RESPONSES_NPM = '@ai-sdk/openai';
const ANTHROPIC_NPM = '@ai-sdk/anthropic';
const GOOGLE_NPM = '@ai-sdk/google';

/**
 * Map an OpenClaw `api` protocol (plus an optional baseUrl hint) to the AI SDK
 * package name the shared card shows and the connectivity/fetch modals speak.
 *
 * The baseUrl fallback covers old imports that never carried `api` at all. It
 * is a hint, not a rule: a provider with a known protocol never consults it.
 */
export const openClawApiToNpm = (api?: string, baseUrl?: string): string => {
  if (api === 'openai-completions') return OPENAI_COMPATIBLE_NPM;
  if (api === 'openai-responses') return OPENAI_RESPONSES_NPM;
  if (api === 'anthropic-messages') return ANTHROPIC_NPM;
  if (api === 'google-generative-ai') return GOOGLE_NPM;

  const url = (baseUrl || '').toLowerCase();
  if (url.includes('anthropic')) return ANTHROPIC_NPM;
  if (url.includes('generativelanguage.googleapis.com') || url.includes('google')) return GOOGLE_NPM;

  return OPENAI_COMPATIBLE_NPM;
};

/** One OpenClaw provider, in the shape the connectivity test modal consumes. */
export const toOpenCodeProvider = (config: OpenClawProviderConfig): OpenCodeProvider => ({
  npm: openClawApiToNpm(config.api, config.baseUrl),
  options: {
    baseURL: config.baseUrl || '',
    apiKey: config.apiKey,
  },
  models: Object.fromEntries(
    (config.models || []).map((model) => [model.id, { name: model.name || model.id }]),
  ),
});
