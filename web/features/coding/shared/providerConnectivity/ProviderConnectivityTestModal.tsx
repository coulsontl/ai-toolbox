import React from 'react';
import type { ClaudeCodeProvider, ClaudeSettingsConfig } from '@/types/claudecode';
import type { CodexProvider, CodexSettingsConfig } from '@/types/codex';
import type { GrokProvider, GrokSettingsConfig } from '@/types/grok';
import type { OpenCodeProvider } from '@/types/opencode';
import type { OpenCodeDiagnosticsConfig } from '@/services/opencodeApi';
import { extractCodexBaseUrl, extractCodexModel, extractCodexReasoningEffort } from '@/utils/codexConfigUtils';
import {
  extractGrokSettingsBaseUrl,
  extractGrokSettingsModel,
} from '@/utils/grokConfigUtils';
import { getClaudeConfiguredModelIds } from '@/features/coding/claudecode/utils/claudeModelConfig';
import ConnectivityTestModal from '@/features/coding/opencode/components/ConnectivityTestModal';
import type { GatewayCliKey } from '@/services/proxyGatewayApi';

const DEFAULT_CLAUDE_BASE_URL = 'https://api.anthropic.com/v1';
const DEFAULT_CODEX_BASE_URL = 'https://api.openai.com/v1';
const DEFAULT_GROK_BASE_URL = 'https://api.x.ai/v1';

export interface ProviderConnectivityInfo {
  providerId: string;
  providerName: string;
  providerConfig: OpenCodeProvider;
  modelIds: string[];
  reasoningEffort?: string;
}

interface ProviderConnectivityTestModalProps {
  open: boolean;
  connectivityInfo: ProviderConnectivityInfo | null;
  onCancel: () => void;
  diagnostics?: OpenCodeDiagnosticsConfig;
  onSaveDiagnostics?: (diagnostics: OpenCodeDiagnosticsConfig) => Promise<void>;
  gatewayCliKey?: Extract<GatewayCliKey, 'claude' | 'codex' | 'grok' | 'gemini'>;
  useGateway?: boolean;
}

function parseJsonConfig<T>(rawConfig: string, fallbackValue: T): T {
  try {
    return JSON.parse(rawConfig) as T;
  } catch (error) {
    console.error('Failed to parse provider settings config:', error);
    return fallbackValue;
  }
}

function normalizeClaudeBaseUrl(baseUrl?: string): string {
  const trimmedBaseUrl = baseUrl?.trim();
  if (!trimmedBaseUrl) {
    return DEFAULT_CLAUDE_BASE_URL;
  }

  const normalizedBaseUrl = trimmedBaseUrl.replace(/\/+$/, '');
  if (/\/v\d+(?:beta\d*)?$/i.test(normalizedBaseUrl)) {
    return normalizedBaseUrl;
  }

  return `${normalizedBaseUrl}/v1`;
}

function buildProviderModels(modelIds: string[]): OpenCodeProvider['models'] {
  return Object.fromEntries(modelIds.map((modelId) => [modelId, {}]));
}

export function buildClaudeProviderConnectivityInfo(
  provider: ClaudeCodeProvider
): ProviderConnectivityInfo {
  const settingsConfig = parseJsonConfig<ClaudeSettingsConfig>(provider.settingsConfig, {});
  const apiKey =
    settingsConfig.env?.ANTHROPIC_AUTH_TOKEN?.trim() ||
    settingsConfig.env?.ANTHROPIC_API_KEY?.trim();
  const uniqueModelIds = getClaudeConfiguredModelIds(settingsConfig, {
    stripOneMMarker: true,
  });

  return {
    providerId: provider.id,
    providerName: provider.name,
    providerConfig: {
      npm: '@ai-sdk/anthropic',
      name: provider.name,
      options: {
        baseURL: normalizeClaudeBaseUrl(settingsConfig.env?.ANTHROPIC_BASE_URL),
        ...(apiKey ? { apiKey } : {}),
      },
      models: buildProviderModels(uniqueModelIds),
    },
    modelIds: uniqueModelIds,
  };
}

export function buildCodexProviderConnectivityInfo(provider: CodexProvider): ProviderConnectivityInfo {
  const settingsConfig = parseJsonConfig<CodexSettingsConfig>(provider.settingsConfig, {});
  const modelId = extractCodexModel(settingsConfig.config)?.trim();
  const reasoningEffort = extractCodexReasoningEffort(settingsConfig.config)?.trim();
  const apiKey = settingsConfig.auth?.OPENAI_API_KEY?.trim();
  const baseUrl = extractCodexBaseUrl(settingsConfig.config)?.trim() || DEFAULT_CODEX_BASE_URL;
  const modelIds = modelId ? [modelId] : [];

  return {
    providerId: provider.id,
    providerName: provider.name,
    providerConfig: {
      npm: '@ai-sdk/openai',
      name: provider.name,
      options: {
        baseURL: baseUrl,
        ...(apiKey ? { apiKey } : {}),
        ...(reasoningEffort ? { reasoningEffort } : {}),
      },
      models: buildProviderModels(modelIds),
    },
    modelIds,
    ...(reasoningEffort ? { reasoningEffort } : {}),
  };
}

export function buildGrokProviderConnectivityInfo(provider: GrokProvider): ProviderConnectivityInfo {
  const settingsConfig = parseJsonConfig<GrokSettingsConfig>(provider.settingsConfig, {});
  const modelId = extractGrokSettingsModel(settingsConfig)?.trim();
  const catalogModelIds = settingsConfig.modelCatalog?.models
    .map((model) => model.model.trim())
    .filter(Boolean) ?? [];
  const modelIds = [...new Set([...(modelId ? [modelId] : []), ...catalogModelIds])];
  const apiKey = settingsConfig.auth?.API_KEY?.trim();
  const baseUrl = extractGrokSettingsBaseUrl(settingsConfig)?.trim() || DEFAULT_GROK_BASE_URL;

  return {
    providerId: provider.id,
    providerName: provider.name,
    providerConfig: {
      npm: '@ai-sdk/openai',
      name: provider.name,
      options: {
        baseURL: baseUrl,
        ...(apiKey ? { apiKey } : {}),
      },
      models: buildProviderModels(modelIds),
    },
    modelIds,
  };
}

const ProviderConnectivityTestModal: React.FC<ProviderConnectivityTestModalProps> = ({
  open,
  connectivityInfo,
  onCancel,
  diagnostics,
  onSaveDiagnostics,
  gatewayCliKey,
  useGateway,
}) => {
  if (!connectivityInfo) {
    return null;
  }

  return (
    <ConnectivityTestModal
      open={open}
      onCancel={onCancel}
      providerId={connectivityInfo.providerId}
      providerName={connectivityInfo.providerName}
      providerConfig={connectivityInfo.providerConfig}
      modelIds={connectivityInfo.modelIds}
      diagnostics={diagnostics}
      onSaveDiagnostics={onSaveDiagnostics || (async () => {})}
      gatewayRequest={useGateway && gatewayCliKey
        ? { cliKey: gatewayCliKey, providerId: connectivityInfo.providerId }
        : undefined}
    />
  );
};

export default ProviderConnectivityTestModal;
