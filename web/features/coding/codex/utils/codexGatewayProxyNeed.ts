import {
  codexWireApiFormatFromConfig,
  firstGatewayApiFormat,
  openAiApiFormatFromBaseUrl,
  providerNeedsGatewayProxy,
  type GatewayApiFormat,
  type GatewayProxyReason,
} from '../../shared/gateway/providerProtocol';
import { getGatewayProviderApiFormatFromMeta } from '../../shared/gateway/providerProfiles';
import type { CodexProvider } from '../../../../types/codex';
import { extractCodexBaseUrl } from '../../../../utils/codexConfigUtils';
import { parseCodexSettingsConfig } from './codexSettingsConfig';

/**
 * Same question for a bare API format, which is all the provider form has while
 * the user is still editing. Keeps the form hint and the stored-provider check
 * on one predicate instead of two copies of the comparison.
 */
export const codexApiFormatNeedsGatewayProxy = (
  apiFormat?: string | null,
  pluginSource?: { meta?: unknown; baseUrl?: string | null } | null,
): boolean => providerNeedsGatewayProxy(apiFormat, 'openai_responses', pluginSource);

/**
 * Does one Codex provider have to keep going through the gateway?
 *
 * Codex speaks the Responses wire only, so a provider stored with a different
 * upstream protocol can never be applied in direct mode. Both the provider card
 * and the aggregate settings panel need this answer, and they must agree: the
 * card disables "restore direct" for a primary that needs the gateway, so the
 * settings panel has to refuse the same action instead of quietly restoring a
 * config that cannot work.
 */
export const codexProviderNeedsGatewayProxy = (provider: CodexProvider): boolean => {
  const settingsConfig = parseCodexSettingsConfig(provider.settingsConfig);
  return codexApiFormatNeedsGatewayProxy(
    resolveCodexProviderApiFormat(provider),
    {
      meta: provider.meta,
      baseUrl: extractCodexBaseUrl(settingsConfig.config),
    },
  );
};

/**
 * The wire protocol a stored Codex provider speaks.
 *
 * Exported on its own because a second question now reads it: the provider
 * detail sidebar states the protocol as a fact, the same way Claude's does. The
 * gateway-proxy check above and that row must read the *same* precedence chain —
 * a second copy of it is how the card ends up saying "OpenAI Responses" about a
 * provider the gateway refuses to route natively.
 */
export const resolveCodexProviderApiFormat = (
  provider: CodexProvider,
): GatewayApiFormat | null => {
  const parsedSettingsConfig = parseCodexSettingsConfig(provider.settingsConfig);
  // `apiFormat` / `api_format` are tolerated legacy keys, not part of the type.
  const settingsConfig = parsedSettingsConfig as typeof parsedSettingsConfig & {
    apiFormat?: unknown;
    api_format?: unknown;
  };
  const baseUrl = extractCodexBaseUrl(settingsConfig.config);
  return firstGatewayApiFormat(
    getGatewayProviderApiFormatFromMeta(provider.meta, 'codex'),
    provider.meta?.apiFormat,
    typeof settingsConfig.apiFormat === 'string' ? settingsConfig.apiFormat : undefined,
    typeof settingsConfig.api_format === 'string' ? settingsConfig.api_format : undefined,
    codexWireApiFormatFromConfig(settingsConfig.config),
    openAiApiFormatFromBaseUrl(baseUrl),
  );
};

/**
 * Same question for the provider the aggregate manifest names as primary.
 *
 * Aggregate mode sets `primary_provider_id` to the first selected site, so this
 * is "does the first site keep the takeover alive?". Mirrors the pages'
 * `primaryGatewayProviderNeedsProxy`, including the official/local exclusions
 * that never route through the gateway.
 */
export const primaryCodexProviderNeedsGatewayProxy = (
  providers: readonly CodexProvider[],
  primaryProviderId: string | null | undefined,
  isLocalProviderId: (providerId: string) => boolean,
): { needsProxy: boolean; reason: GatewayProxyReason } => {
  const primaryProvider = providers.find((provider) => provider.id === primaryProviderId);
  if (
    !primaryProvider ||
    primaryProvider.category === 'official' ||
    isLocalProviderId(primaryProvider.id)
  ) {
    return { needsProxy: false, reason: null };
  }
  const needsProxy = codexProviderNeedsGatewayProxy(primaryProvider);
  return { needsProxy, reason: needsProxy ? 'protocol' : null };
};