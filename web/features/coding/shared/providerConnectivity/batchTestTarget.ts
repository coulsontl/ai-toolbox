export interface ProviderConnectivityInfo {
  providerId: string;
  providerName: string;
  providerConfig: {
    npm?: string;
    options?: {
      baseURL?: string;
      apiKey?: string;
    };
  };
  modelIds: string[];
  reasoningEffort?: string;
}

export interface ProviderConnectivityBatchTarget {
  providerId: string;
  request?: {
    npm: string;
    providerId: string;
    baseUrl: string;
    apiKey?: string;
    reasoningEffort?: string;
    prompt: string;
    stream: boolean;
    modelIds: string[];
    timeoutSecs: number;
  };
  gatewayRequest?: {
    cliKey: 'claude' | 'codex' | 'grok' | 'gemini';
    providerId: string;
    prompt: string;
    stream: boolean;
    modelIds: string[];
    timeoutSecs: number;
  };
  errorMessage?: string;
}

interface ProviderConnectivityBatchTargetOptions {
  requireBaseUrl?: boolean;
  requireApiKey?: boolean;
  preferredModelId?: string;
  prompt?: string;
  timeoutSecs?: number;
  gatewayCliKey?: 'claude' | 'codex' | 'grok' | 'gemini';
  useGateway?: boolean;
  errorMessages: {
    missingBaseUrl: string;
    missingApiKey: string;
    missingModel: string;
  };
}

const DEFAULT_CONNECTIVITY_PROMPT = 'say hi!';

export function buildProviderConnectivityBatchTarget(
  info: ProviderConnectivityInfo,
  options: ProviderConnectivityBatchTargetOptions,
): ProviderConnectivityBatchTarget {
  const providerOptions = info.providerConfig.options || {};
  const npm = info.providerConfig.npm || '@ai-sdk/openai-compatible';
  const baseUrl = providerOptions.baseURL?.trim() || '';
  const apiKey = providerOptions.apiKey?.trim();
  const modelId = options.preferredModelId && info.modelIds.includes(options.preferredModelId)
    ? options.preferredModelId
    : info.modelIds[0];

  if (options.requireBaseUrl && !baseUrl) {
    return {
      providerId: info.providerId,
      errorMessage: options.errorMessages.missingBaseUrl,
    };
  }

  if (options.requireApiKey && !apiKey) {
    return {
      providerId: info.providerId,
      errorMessage: options.errorMessages.missingApiKey,
    };
  }

  if (!modelId) {
    return {
      providerId: info.providerId,
      errorMessage: options.errorMessages.missingModel,
    };
  }

  return {
    providerId: info.providerId,
    ...(options.useGateway && options.gatewayCliKey
      ? {
          gatewayRequest: {
            cliKey: options.gatewayCliKey,
            providerId: info.providerId,
            prompt: options.prompt || DEFAULT_CONNECTIVITY_PROMPT,
            stream: true,
            modelIds: [modelId],
            timeoutSecs: options.timeoutSecs ?? 30,
          },
        }
      : {
          request: {
            npm,
            providerId: info.providerId,
            baseUrl,
            ...(apiKey ? { apiKey } : {}),
            ...(info.reasoningEffort ? { reasoningEffort: info.reasoningEffort } : {}),
            prompt: options.prompt || DEFAULT_CONNECTIVITY_PROMPT,
            stream: true,
            modelIds: [modelId],
            timeoutSecs: options.timeoutSecs ?? 30,
          },
        }),
  };
}
