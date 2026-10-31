import React from 'react';
import type { ClaudeCodeProvider, ClaudeSettingsConfig } from '@/types/claudecode';
import type { CodexProvider, CodexSettingsConfig } from '@/types/codex';
import type { GrokProvider, GrokSettingsConfig } from '@/types/grok';
import type { KimiProvider } from '@/types/kimi';
import type { OpenCodeProvider } from '@/types/opencode';
import type { OpenCodeDiagnosticsConfig } from '@/services/opencodeApi';
import { extractCodexBaseUrl, extractCodexModel, extractCodexReasoningEffort } from '@/utils/codexConfigUtils';
import { buildCodexConnectivityModelIds } from '@/features/coding/codex/utils/codexCatalogModels';
import {
  extractGrokSettingsBaseUrl,
  extractGrokSettingsModel,
} from '@/utils/grokConfigUtils';
import { getClaudeConfiguredModelIds } from '@/features/coding/claudecode/utils/claudeModelConfig';
import {
  KIMI_OFFICIAL_API_BASE_URL,
  parseKimiSettingsConfig,
} from '@/features/coding/kimi/utils/settingsConfig';
import ConnectivityTestModal from '@/features/coding/opencode/components/ConnectivityTestModal';
import type { ProviderModelConnections } from '@/features/coding/shared/providerConnectivity/modelConnection';
import type { GatewayCliKey } from '@/services/proxyGatewayApi';
import type { ConfigValueMode } from '@/components/common/FetchModelsModal/types';
import type { ZcodeProvider } from '@/types/zcode';
import { parseZcodeProviderSettings } from '@/features/coding/zcode/utils/zcodeSettingsConfig';
import { zcodeSdkName } from '@/features/coding/zcode/utils/zcodeFavoriteProvider';
import type { OmoNativeProvider } from '@/types/omoNative';
import {
  asRecord,
  getOmoNativeModelEntries,
  getStringField,
  omoNativeApiToNpm,
} from '@/features/coding/omo_native/utils/omoNativeProviders';

const DEFAULT_CLAUDE_BASE_URL = 'https://api.anthropic.com/v1';
const DEFAULT_CODEX_BASE_URL = 'https://api.openai.com/v1';
const DEFAULT_GROK_BASE_URL = 'https://api.x.ai/v1';

export interface ProviderConnectivityInfo {
  providerId: string;
  providerName: string;
  providerConfig: OpenCodeProvider;
  modelIds: string[];
  reasoningEffort?: string;
  apiFormat?: 'openai-codex-responses';
  configValueMode?: ConfigValueMode;
  /** Models whose own api/baseUrl overrides the provider connection (OMP). */
  modelConnections?: ProviderModelConnections;
}

interface ProviderConnectivityTestModalProps {
  open: boolean;
  connectivityInfo: ProviderConnectivityInfo | null;
  onCancel: () => void;
  diagnostics?: OpenCodeDiagnosticsConfig;
  onSaveDiagnostics?: (diagnostics: OpenCodeDiagnosticsConfig) => Promise<void>;
  /** Model IDs that may be removed after a failed connectivity test. */
  removableModelIds?: string[];
  /** Remove selected failed models from the provider catalog/config. */
  onRemoveModels?: (modelIds: string[]) => Promise<void>;
  gatewayCliKey?: Extract<
    GatewayCliKey,
    | 'claude'
    | 'codex'
    | 'grok'
    | 'kimi'
    | 'gemini'
    | 'claude_desktop'
    // The WorkBuddy family is both a usage tool and a takeover target; the
    // probe speaks the takeover spelling.
    | 'workbuddy_desktop'
    | 'codebuddy_cli'
  >;
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
  // The catalog is the provider's real model list, so the test covers every
  // catalog row in addition to the config.toml default.
  const modelIds = buildCodexConnectivityModelIds(modelId, settingsConfig.modelCatalog?.models);

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

/**
 * OmO Native 的连通性测试入参。
 *
 * ⚠️ `configValueMode` 必须是 `'omo'`，**不是** `'omp'`：两者同属 `oh-my-*`
 * 家族但配置值语法不同——OmO 支持 `$ENV_VAR` 插值 / `!command` / `$$` 转义
 * （与 Pi 同引擎），OMP 只认 `!command` 或环境变量名精确匹配。传错会把
 * `$MY_KEY` 当字面量发给上游（2026-10-07 实测 401）。
 *
 * 密钥走 `provider.apiKey`（列表接口回填的明文，`auth.json` 优先），不从
 * `config.apiKey` 取——保存时那条已经被迁移进 `auth.json` 了。
 */
export function buildOmoNativeProviderConnectivityInfo(
  provider: OmoNativeProvider,
): ProviderConnectivityInfo {
  const modelIds = getOmoNativeModelEntries(provider.config).map((entry) => entry.id);
  const headers = asRecord(provider.config.headers);
  const apiKey = provider.apiKey?.trim();

  return {
    providerId: provider.key,
    providerName: getStringField(provider.config, 'name') || provider.key,
    providerConfig: {
      npm: omoNativeApiToNpm(getStringField(provider.config, 'api')),
      name: getStringField(provider.config, 'name') || provider.key,
      options: {
        baseURL: getStringField(provider.config, 'baseUrl'),
        ...(apiKey ? { apiKey } : {}),
        ...(Object.keys(headers).length > 0
          ? { headers: headers as Record<string, string> }
          : {}),
      },
      models: buildProviderModels(modelIds),
    },
    modelIds,
    configValueMode: 'omo',
  };
}

/**
 * ZCode 的连通性测试入参。
 *
 * ZCode 的 baseUrl / apiKey / 模型目录全在 `settingsConfig` 的 JSON 里
 * （与 Claude / Codex / Grok 同形），所以这里解一次就够。
 */
export function buildZcodeProviderConnectivityInfo(
  provider: ZcodeProvider,
): ProviderConnectivityInfo {
  const settings = parseZcodeProviderSettings(provider.settingsConfig);
  const modelIds = (settings?.models ?? []).map((model) => model.modelId);
  const apiKey = settings?.config?.access?.apiKey?.trim();
  const baseUrl = settings?.config?.api?.baseUrl?.trim() ?? '';

  return {
    providerId: provider.id,
    providerName: provider.name,
    providerConfig: {
      npm: zcodeSdkName(settings?.config?.api?.type),
      name: provider.name,
      options: {
        baseURL: baseUrl,
        ...(apiKey ? { apiKey } : {}),
        ...(settings?.config?.api?.headers ? { headers: settings.config.api.headers } : {}),
      },
      models: buildProviderModels(modelIds),
    },
    modelIds,
  };
}

export function buildGrokProviderConnectivityInfo(provider: GrokProvider): ProviderConnectivityInfo {
  const settingsConfig = parseJsonConfig<GrokSettingsConfig>(provider.settingsConfig, {});
  const catalogModels = settingsConfig.modelCatalog?.models || [];
  const defaultModelKey = settingsConfig.defaultModelKey?.trim() || extractGrokSettingsModel(settingsConfig)?.trim();
  // Grok stores local catalog keys (e.g. "custom") separately from upstream model IDs.
  // Connectivity tests must only send upstream model IDs, never the local key.
  const selectedCatalogModel = catalogModels.find(
    (model) => model.key?.trim() === defaultModelKey || model.model?.trim() === defaultModelKey,
  ) || catalogModels[0];
  const selectedUpstreamModelId = selectedCatalogModel?.model?.trim()
    || (selectedCatalogModel ? undefined : defaultModelKey);
  const catalogUpstreamModelIds = catalogModels
    .map((model) => model.model?.trim())
    .filter((modelId): modelId is string => Boolean(modelId));
  const modelIds = [...new Set([
    ...(selectedUpstreamModelId ? [selectedUpstreamModelId] : []),
    ...catalogUpstreamModelIds,
  ])];
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

// Kimi catalog keys are provider-aliased (e.g. "kimi-code/kimi-for-coding");
// the upstream model id is the alias tail.
function stripKimiModelAlias(key: string): string {
  return key.includes('/') ? key.slice(key.lastIndexOf('/') + 1) : key;
}

export function buildKimiProviderConnectivityInfo(provider: KimiProvider): ProviderConnectivityInfo {
  const settingsConfig = parseKimiSettingsConfig(provider.settingsConfig);
  const defaultModelKey = settingsConfig.defaultModelKey.trim();
  const catalogModels = settingsConfig.catalogModels;
  const selectedCatalogModel = catalogModels.find(
    (model) => model.key?.trim() === defaultModelKey || model.model?.trim() === defaultModelKey,
  ) || catalogModels[0];
  const selectedUpstreamModelId = selectedCatalogModel?.model?.trim()
    || (selectedCatalogModel ? undefined : (defaultModelKey ? stripKimiModelAlias(defaultModelKey) : ''));
  const catalogUpstreamModelIds = catalogModels
    .map((model) => model.model?.trim())
    .filter((modelId): modelId is string => Boolean(modelId));
  const modelIds = [...new Set([
    ...(selectedUpstreamModelId ? [selectedUpstreamModelId] : []),
    ...catalogUpstreamModelIds,
  ])];
  const apiKey = settingsConfig.apiKey.trim();
  const baseUrl = settingsConfig.baseUrl.trim() || KIMI_OFFICIAL_API_BASE_URL;

  return {
    providerId: provider.id,
    providerName: provider.name,
    providerConfig: {
      // Kimi relays are OpenAI-compatible endpoints.
      npm: '@ai-sdk/openai-compatible',
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
  removableModelIds,
  onRemoveModels,
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
      apiFormat={connectivityInfo.apiFormat}
      configValueMode={connectivityInfo.configValueMode}
      modelConnections={connectivityInfo.modelConnections}
      modelIds={connectivityInfo.modelIds}
      removableModelIds={removableModelIds ?? connectivityInfo.modelIds}
      diagnostics={diagnostics}
      onSaveDiagnostics={onSaveDiagnostics || (async () => {})}
      onRemoveModels={onRemoveModels}
      gatewayRequest={useGateway && gatewayCliKey
        ? { cliKey: gatewayCliKey, providerId: connectivityInfo.providerId }
        : undefined}
    />
  );
};

export default ProviderConnectivityTestModal;
