import type {
  GrokApiFormat,
  GrokCatalogModel,
  GrokProviderCategory,
  GrokSettingsConfig,
} from '../../../../types/grok';
import { normalizeGrokConfigForOfficialMode } from '../../../../utils/grokConfigUtils';
import { isJsonObject } from '../../../../utils/json';
import { normalizeGrokCatalogModels } from './grokCatalogModels';

export interface BuildGrokSettingsConfigInput {
  category: GrokProviderCategory;
  apiKey: string;
  baseUrl: string;
  model: string;
  apiFormat?: GrokApiFormat;
  config: string;
  catalogModels: GrokCatalogModel[];
  auth: Record<string, unknown>;
}

export function parseGrokSettingsConfig(rawConfig: string | undefined): GrokSettingsConfig {
  if (!rawConfig?.trim()) return {};

  try {
    const parsedConfig = JSON.parse(rawConfig) as unknown;
    return isJsonObject(parsedConfig) ? parsedConfig as GrokSettingsConfig : {};
  } catch (error) {
    console.error('Failed to parse Grok settings config:', error);
    return {};
  }
}

export function buildGrokSettingsConfig({
  category,
  apiKey,
  baseUrl,
  model,
  apiFormat,
  config,
  catalogModels,
  auth,
}: BuildGrokSettingsConfigInput): string {
  const finalConfig = category === 'official'
    ? normalizeGrokConfigForOfficialMode(config)
    : config.trim();
  const normalizedApiKey = apiKey.trim();
  const normalizedModel = model.trim();
  const normalizedBaseUrl = baseUrl.trim();
  const apiBackend = apiFormat === 'openai_responses'
    ? 'responses'
    : apiFormat === 'anthropic_messages'
      ? 'messages'
      : 'chat_completions';
  let normalizedCatalogModels = normalizeGrokCatalogModels(catalogModels);

  if (category === 'custom') {
    normalizedCatalogModels = normalizedCatalogModels.map((catalogModel) => ({
      ...catalogModel,
      key: catalogModel.key?.trim() || catalogModel.model,
      ...(catalogModel.baseUrl?.trim() || !normalizedBaseUrl
        ? {}
        : { baseUrl: normalizedBaseUrl }),
      ...(catalogModel.apiBackend?.trim() ? {} : { apiBackend }),
    }));

    const selectedModelExists = normalizedCatalogModels.some(
      (catalogModel) => catalogModel.key === normalizedModel || catalogModel.model === normalizedModel,
    );
    if (normalizedModel && !selectedModelExists) {
      normalizedCatalogModels.push({
        key: normalizedModel,
        model: normalizedModel,
        displayName: normalizedModel,
        ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
        apiBackend,
      });
    }
  }

  const finalAuth = { ...auth };
  if (category === 'custom' && normalizedApiKey) {
    finalAuth.API_KEY = normalizedApiKey;
  } else {
    delete finalAuth.API_KEY;
  }

  const settingsConfig: GrokSettingsConfig = {
    auth: finalAuth,
    config: finalConfig.trim(),
    ...(normalizedModel ? { defaultModelKey: normalizedModel } : {}),
  };
  if (category === 'custom' && normalizedCatalogModels.length > 0) {
    settingsConfig.modelCatalog = {
      models: normalizedCatalogModels,
    };
  }

  return JSON.stringify(settingsConfig);
}
