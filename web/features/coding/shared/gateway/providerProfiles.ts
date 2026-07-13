import { normalizeGatewayApiFormat, type GatewayApiFormat } from './providerProtocol';

export type GatewayProviderToolKey = 'claude' | 'codex' | 'grok' | 'gemini';

export interface GatewayProviderProfileReference {
  tool?: GatewayProviderToolKey;
  profileId: string;
  endpointId: string;
}

export interface GatewayProviderModelDefaults {
  primary?: string;
  haiku?: string;
  sonnet?: string;
  opus?: string;
  fable?: string;
}

export interface GatewayProviderEndpointProfile {
  id: string;
  label: string;
  apiFormat: GatewayApiFormat | 'anthropic';
  baseUrl: string;
  apiKeyField?: string;
  model?: string;
  reasoningField?: 'reasoning_content' | 'content' | 'reasoning' | 'none' | 'all' | string;
  defaultMaxTokens?: number;
  imageInputPolicy?: 'auto' | 'preserve' | 'strip' | 'text_only' | string;
  textOnlyModels?: string[];
  imageCapableModels?: string[];
  allowTextOnlyModelHeuristic?: boolean;
  models?: GatewayProviderModelDefaults;
  configProviderId?: string;
  modelCatalog?: {
    models?: Array<{
      model: string;
      displayName?: string;
      contextWindow?: number;
      supportsImage?: boolean;
      vision?: boolean;
      attachment?: boolean;
      modalities?: {
        input?: string[];
        output?: string[];
      };
    }>;
  };
  codexChatReasoning?: Record<string, unknown>;
}

export interface GatewayProviderToolProfile {
  defaultEndpointId: string;
  endpoints: GatewayProviderEndpointProfile[];
}

export interface GatewayProviderProfile {
  id: string;
  providerType: string;
  apiKeyField?: string;
  reasoningField?: 'reasoning_content' | 'content' | 'reasoning' | 'none' | 'all' | string;
  defaultMaxTokens?: number;
  label: string;
  category?: string;
  aliases?: string[];
  tools: Partial<Record<GatewayProviderToolKey, GatewayProviderToolProfile>>;
  compat?: Record<string, string[]>;
}

export interface GatewayProviderProfileCatalog {
  schemaVersion: number;
  updatedAt?: string;
  profiles: GatewayProviderProfile[];
}

type GatewayProviderProfileListener = () => void;

export const GATEWAY_PROVIDER_PROFILES_REMOTE_URL =
  'https://raw.githubusercontent.com/coulsontl/ai-toolbox/main/tauri/resources/gateway_provider_profiles.json';

export const GATEWAY_PROVIDER_PROFILE_CATALOG: GatewayProviderProfileCatalog = {
  schemaVersion: 1,
  profiles: [],
};

let gatewayProviderProfilesVersion = 0;
let gatewayProviderProfilesInitialized = false;
const gatewayProviderProfileListeners = new Set<GatewayProviderProfileListener>();

export const getGatewayProviderProfilesVersion = () => gatewayProviderProfilesVersion;
export const areGatewayProviderProfilesInitialized = () => gatewayProviderProfilesInitialized;
export const markGatewayProviderProfilesInitialized = () => {
  if (gatewayProviderProfilesInitialized) return;
  gatewayProviderProfilesInitialized = true;
  notifyGatewayProviderProfilesUpdated();
};

export const subscribeGatewayProviderProfiles = (
  listener: GatewayProviderProfileListener,
): (() => void) => {
  gatewayProviderProfileListeners.add(listener);
  return () => {
    gatewayProviderProfileListeners.delete(listener);
  };
};

const notifyGatewayProviderProfilesUpdated = () => {
  gatewayProviderProfilesVersion += 1;
  gatewayProviderProfileListeners.forEach((listener) => listener());
};

export const updateGatewayProviderProfiles = (catalog: GatewayProviderProfileCatalog) => {
  if (!catalog || catalog.schemaVersion !== 1 || !Array.isArray(catalog.profiles) || catalog.profiles.length === 0) {
    return;
  }
  GATEWAY_PROVIDER_PROFILE_CATALOG.schemaVersion = catalog.schemaVersion;
  GATEWAY_PROVIDER_PROFILE_CATALOG.updatedAt = catalog.updatedAt;
  GATEWAY_PROVIDER_PROFILE_CATALOG.profiles = catalog.profiles;
  gatewayProviderProfilesInitialized = true;
  notifyGatewayProviderProfilesUpdated();
};

export const CUSTOM_PROVIDER_PROFILE_ID = '__custom__';
export const CUSTOM_PROVIDER_ENDPOINT_KEY = `${CUSTOM_PROVIDER_PROFILE_ID}:`;

export interface GatewayProviderEndpointSelection {
  providerProfileId: string;
  providerEndpointId?: string;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const normalizeGatewayProviderTool = (value?: string | null): GatewayProviderToolKey | undefined => {
  const normalized = value?.trim().toLowerCase();
  if (normalized === 'claude' || normalized === 'codex' || normalized === 'grok' || normalized === 'gemini') {
    return normalized;
  }
  return undefined;
};

const nonEmptyString = (value: unknown) =>
  typeof value === 'string' && value.trim() ? value.trim() : undefined;

export const toGatewayProviderEndpointKey = (
  profileId: string,
  endpointId?: string | null,
) => `${profileId}:${endpointId || ''}`;

export const parseGatewayProviderEndpointKey = (value?: string | null) => {
  if (!value || value === CUSTOM_PROVIDER_ENDPOINT_KEY) {
    return {
      providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
      providerEndpointId: undefined,
    };
  }
  const separatorIndex = value.indexOf(':');
  if (separatorIndex < 0) {
    return {
      providerProfileId: value,
      providerEndpointId: undefined,
    };
  }
  return {
    providerProfileId: value.slice(0, separatorIndex),
    providerEndpointId: value.slice(separatorIndex + 1) || undefined,
  };
};

export const getGatewayProviderProfilesForTool = (tool: GatewayProviderToolKey) =>
  GATEWAY_PROVIDER_PROFILE_CATALOG.profiles.filter((profile) => profile.tools?.[tool]);

export const findGatewayProviderProfile = (profileId?: string | null) =>
  GATEWAY_PROVIDER_PROFILE_CATALOG.profiles.find((profile) => profile.id === profileId);

export const findGatewayProviderToolProfile = (
  profileId: string | null | undefined,
  tool: GatewayProviderToolKey,
) => findGatewayProviderProfile(profileId)?.tools?.[tool];

export const findGatewayProviderEndpoint = (
  profileId: string | null | undefined,
  tool: GatewayProviderToolKey,
  endpointId?: string | null,
) => {
  const toolProfile = findGatewayProviderToolProfile(profileId, tool);
  if (!toolProfile) {
    return undefined;
  }
  const selectedEndpointId = endpointId || toolProfile.defaultEndpointId;
  return toolProfile.endpoints.find((endpoint) => endpoint.id === selectedEndpointId)
    ?? toolProfile.endpoints.find((endpoint) => endpoint.id === toolProfile.defaultEndpointId)
    ?? toolProfile.endpoints[0];
};

export const toGatewayProviderProfileReference = (
  tool: GatewayProviderToolKey,
  profileId: string,
  endpointId: string,
): GatewayProviderProfileReference => ({
  tool,
  profileId,
  endpointId,
});

export const getGatewayProviderProfileReferenceFromMeta = (
  meta?: unknown,
): GatewayProviderProfileReference | undefined => {
  if (!isRecord(meta)) {
    return undefined;
  }

  const rawReference = meta.gatewayProfile ?? meta.gateway_profile;
  if (!isRecord(rawReference)) {
    return undefined;
  }

  const profileId = nonEmptyString(rawReference.profileId ?? rawReference.profile_id);
  const endpointId = nonEmptyString(rawReference.endpointId ?? rawReference.endpoint_id);
  if (!profileId || !endpointId) {
    return undefined;
  }

  return {
    tool: normalizeGatewayProviderTool(nonEmptyString(rawReference.tool)),
    profileId,
    endpointId,
  };
};

export const findGatewayProviderEndpointByReference = (
  reference: GatewayProviderProfileReference | undefined,
  fallbackTool?: GatewayProviderToolKey,
) => {
  if (!reference) {
    return undefined;
  }

  const tool = reference.tool ?? fallbackTool;
  if (!tool) {
    return undefined;
  }

  const profile = findGatewayProviderProfile(reference.profileId);
  const toolProfile = profile?.tools?.[tool];
  const endpoint = toolProfile?.endpoints.find((item) => item.id === reference.endpointId);
  if (!profile || !endpoint) {
    return undefined;
  }

  return {
    tool,
    profile,
    endpoint,
  };
};

export const getGatewayProviderApiFormatFromMeta = (
  meta: unknown,
  tool: GatewayProviderToolKey,
) => {
  const resolved = findGatewayProviderEndpointByReference(
    getGatewayProviderProfileReferenceFromMeta(meta),
    tool,
  );
  return resolved ? normalizeGatewayApiFormat(resolved.endpoint.apiFormat) : undefined;
};

const customGatewayProviderEndpointSelection = (): GatewayProviderEndpointSelection => ({
  providerProfileId: CUSTOM_PROVIDER_PROFILE_ID,
  providerEndpointId: undefined,
});

export const inferUniqueGatewayProviderEndpointSelection = (params: {
  tool: GatewayProviderToolKey;
  providerType?: string | null;
  apiFormat?: string | null;
}): GatewayProviderEndpointSelection => {
  const normalizedProviderType = params.providerType?.trim().toLowerCase();
  const normalizedApiFormat = normalizeGatewayApiFormat(params.apiFormat);

  if (!normalizedProviderType || !normalizedApiFormat) {
    return customGatewayProviderEndpointSelection();
  }

  const matches = getGatewayProviderProfilesForTool(params.tool)
    .filter((profile) => profile.providerType.toLowerCase() === normalizedProviderType)
    .flatMap((profile) => {
      const toolProfile = profile.tools[params.tool];
      return (toolProfile?.endpoints || []).map((endpoint) => ({ profile, endpoint }));
    })
    .filter(({ endpoint }) => normalizeGatewayApiFormat(endpoint.apiFormat) === normalizedApiFormat);

  if (matches.length !== 1) {
    return customGatewayProviderEndpointSelection();
  }

  return {
    providerProfileId: matches[0].profile.id,
    providerEndpointId: matches[0].endpoint.id,
  };
};

export const inferGatewayProviderEndpointSelection = (params: {
  tool: GatewayProviderToolKey;
  meta?: unknown;
  providerType?: string | null;
  apiFormat?: string | null;
}): GatewayProviderEndpointSelection => {
  const resolvedReference = findGatewayProviderEndpointByReference(
    getGatewayProviderProfileReferenceFromMeta(params.meta),
    params.tool,
  );
  if (resolvedReference) {
    return {
      providerProfileId: resolvedReference.profile.id,
      providerEndpointId: resolvedReference.endpoint.id,
    };
  }

  if (params.providerType || params.apiFormat) {
    return inferUniqueGatewayProviderEndpointSelection({
      tool: params.tool,
      providerType: params.providerType,
      apiFormat: params.apiFormat,
    });
  }

  return customGatewayProviderEndpointSelection();
};

export const mergeGatewayProfileReferenceIntoMeta = <T extends object>(
  meta: T | undefined,
  reference: GatewayProviderProfileReference | undefined,
  apiFormat?: string,
): T | undefined => {
  const nextMeta = { ...(meta || {}) } as Record<string, unknown>;
  delete nextMeta.gatewayProfile;
  delete nextMeta.gateway_profile;

  if (reference) {
    delete nextMeta.providerType;
    delete nextMeta.provider_type;
    delete nextMeta.apiKeyField;
    delete nextMeta.api_key_field;
    delete nextMeta.reasoningField;
    delete nextMeta.reasoning_field;
    delete nextMeta.defaultMaxTokens;
    delete nextMeta.default_max_tokens;
    delete nextMeta.imageInputPolicy;
    delete nextMeta.image_input_policy;
    delete nextMeta.textOnlyModels;
    delete nextMeta.text_only_models;
    delete nextMeta.imageCapableModels;
    delete nextMeta.image_capable_models;
    delete nextMeta.allowTextOnlyModelHeuristic;
    delete nextMeta.allow_text_only_model_heuristic;
    delete nextMeta.codexChatReasoning;
    delete nextMeta.codex_chat_reasoning;
    delete nextMeta.apiFormat;
    delete nextMeta.api_format;
    nextMeta.gatewayProfile = reference;
  } else if (apiFormat) {
    delete nextMeta.apiFormat;
    delete nextMeta.api_format;
    nextMeta.apiFormat = apiFormat;
  }

  return Object.values(nextMeta).some((value) => value !== undefined && value !== null && value !== '')
    ? nextMeta as T
    : undefined;
};
