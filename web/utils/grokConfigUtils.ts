import { parse as parseToml, stringify as stringifyToml } from 'smol-toml';

type TomlObject = Record<string, unknown>;

interface GrokSettingsLike {
  config?: string;
  defaultModelKey?: string;
  defaultReasoningEffort?: string;
  modelCatalog?: {
    models?: Array<{
      key?: string;
      model?: string;
      baseUrl?: string;
      apiBackend?: string;
      reasoningEffort?: string;
    }>;
  };
}

function isTomlObject(value: unknown): value is TomlObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseGrokToml(configText: string | undefined | null): TomlObject {
  const normalizedText = normalizeQuotes(typeof configText === 'string' ? configText : '');
  return normalizedText.trim() ? parseToml(normalizedText) as TomlObject : {};
}

function writeGrokToml(config: TomlObject): string {
  return stringifyToml(config).trim();
}

function getDefaultModelKey(config: TomlObject): string | undefined {
  const models = isTomlObject(config.models) ? config.models : undefined;
  const defaultModel = models?.default;
  return typeof defaultModel === 'string' && defaultModel.trim()
    ? defaultModel.trim()
    : undefined;
}

function getSelectedModelConfig(config: TomlObject): TomlObject | undefined {
  const modelRoot = isTomlObject(config.model) ? config.model : undefined;
  if (!modelRoot) return undefined;

  const defaultModelKey = getDefaultModelKey(config);
  if (defaultModelKey && isTomlObject(modelRoot[defaultModelKey])) {
    return modelRoot[defaultModelKey] as TomlObject;
  }

  return Object.values(modelRoot).find(isTomlObject);
}

export function normalizeQuotes(text: string): string {
  if (!text) return text;
  return text
    .replace(/[“”＂]/g, '"')
    .replace(/[‘’＇]/g, "'");
}

export function extractGrokBaseUrl(configText: string | undefined | null): string | undefined {
  try {
    const config = parseGrokToml(configText);
    const selectedModel = getSelectedModelConfig(config);
    if (typeof selectedModel?.base_url === 'string' && selectedModel.base_url.trim()) {
      return selectedModel.base_url.trim();
    }
    return typeof config.base_url === 'string' && config.base_url.trim()
      ? config.base_url.trim()
      : undefined;
  } catch {
    return normalizeQuotes(typeof configText === 'string' ? configText : '')
      .match(/^\s*base_url\s*=\s*(['"])([^'"]+)\1/m)?.[2];
  }
}

export function setGrokBaseUrl(configText: string, baseUrl: string): string {
  const normalizedBaseUrl = baseUrl.trim().replace(/\s+/g, '');
  if (!normalizedBaseUrl) return configText;

  const config = parseGrokToml(configText);
  const defaultModelKey = getDefaultModelKey(config) || 'custom';
  const models = isTomlObject(config.models) ? { ...config.models } : {};
  models.default = defaultModelKey;
  config.models = models;

  const modelRoot = isTomlObject(config.model) ? { ...config.model } : {};
  const modelConfig = isTomlObject(modelRoot[defaultModelKey])
    ? { ...(modelRoot[defaultModelKey] as TomlObject) }
    : {};
  modelConfig.base_url = normalizedBaseUrl;
  modelRoot[defaultModelKey] = modelConfig;
  config.model = modelRoot;
  return writeGrokToml(config);
}

export function removeGrokBaseUrl(configText: string): string {
  const config = parseGrokToml(configText);
  delete config.base_url;
  const modelRoot = isTomlObject(config.model) ? { ...config.model } : undefined;
  if (modelRoot) {
    for (const [modelKey, rawModelConfig] of Object.entries(modelRoot)) {
      if (!isTomlObject(rawModelConfig)) continue;
      const modelConfig = { ...rawModelConfig };
      delete modelConfig.base_url;
      modelRoot[modelKey] = modelConfig;
    }
    config.model = modelRoot;
  }
  return writeGrokToml(config);
}

export function extractGrokModel(configText: string | undefined | null): string | undefined {
  try {
    const config = parseGrokToml(configText);
    return getDefaultModelKey(config)
      || (typeof config.model === 'string' && config.model.trim() ? config.model.trim() : undefined);
  } catch {
    const normalizedText = normalizeQuotes(typeof configText === 'string' ? configText : '');
    return normalizedText.match(/^\s*default\s*=\s*(['"])([^'"]+)\1/m)?.[2]
      || normalizedText.match(/^\s*model\s*=\s*(['"])([^'"]+)\1/m)?.[2];
  }
}

export function extractGrokReasoningEffort(
  configText: string | undefined | null,
): string | undefined {
  try {
    const config = parseGrokToml(configText);
    const models = isTomlObject(config.models) ? config.models : {};
    if (typeof models.default_reasoning_effort === 'string') {
      return models.default_reasoning_effort;
    }
    const selectedModel = getSelectedModelConfig(config);
    return typeof selectedModel?.reasoning_effort === 'string'
      ? selectedModel.reasoning_effort
      : undefined;
  } catch {
    return undefined;
  }
}

function getSelectedGrokCatalogModel(settings: GrokSettingsLike) {
  const catalogModels = settings.modelCatalog?.models || [];
  if (catalogModels.length === 0) {
    return undefined;
  }
  // Prefer the local catalog key pointed to by defaultModelKey (custom providers use
  // fixed key "custom"). Fall back to a match on upstream model id, then first entry.
  const defaultModelKey = settings.defaultModelKey?.trim();
  if (defaultModelKey) {
    const byKey = catalogModels.find((model) => model.key?.trim() === defaultModelKey);
    if (byKey) {
      return byKey;
    }
    const byUpstreamModel = catalogModels.find((model) => model.model?.trim() === defaultModelKey);
    if (byUpstreamModel) {
      return byUpstreamModel;
    }
  }
  return catalogModels[0];
}

/**
 * Upstream model ID for the selected default slot.
 * Custom providers store local key in defaultModelKey (usually "custom") and the
 * real request model on modelCatalog.models[].model — never surface the key as the
 * form "model name".
 */
export function extractGrokSettingsModel(settings: GrokSettingsLike): string | undefined {
  const selectedModel = getSelectedGrokCatalogModel(settings);
  const upstreamModel = selectedModel?.model?.trim();
  if (upstreamModel) {
    return upstreamModel;
  }
  // Official providers only store defaultModelKey as the model id (no catalog).
  const defaultModelKey = settings.defaultModelKey?.trim();
  if (defaultModelKey && defaultModelKey !== 'custom') {
    return defaultModelKey;
  }
  return extractGrokModel(settings.config);
}

export function extractGrokSettingsBaseUrl(settings: GrokSettingsLike): string | undefined {
  const selectedModel = getSelectedGrokCatalogModel(settings);
  return selectedModel?.baseUrl?.trim() || extractGrokBaseUrl(settings.config);
}

export function extractGrokSettingsApiBackend(settings: GrokSettingsLike): string | undefined {
  const selectedModel = getSelectedGrokCatalogModel(settings);
  return selectedModel?.apiBackend?.trim();
}

export function extractGrokSettingsReasoningEffort(settings: GrokSettingsLike): string | undefined {
  const defaultReasoningEffort = settings.defaultReasoningEffort?.trim();
  if (defaultReasoningEffort) {
    return defaultReasoningEffort;
  }
  const selectedModel = getSelectedGrokCatalogModel(settings);
  return selectedModel?.reasoningEffort?.trim() || extractGrokReasoningEffort(settings.config);
}

export function setGrokModel(configText: string, model: string): string {
  const normalizedModel = model.trim();
  if (!normalizedModel) return configText;
  const config = parseGrokToml(configText);
  const models = isTomlObject(config.models) ? { ...config.models } : {};
  models.default = normalizedModel;
  config.models = models;
  delete config.model_provider;
  delete config.model_providers;
  if (typeof config.model === 'string') delete config.model;
  return writeGrokToml(config);
}

export function removeGrokModel(configText: string): string {
  const config = parseGrokToml(configText);
  const models = isTomlObject(config.models) ? { ...config.models } : undefined;
  if (models) {
    delete models.default;
    if (Object.keys(models).length > 0) config.models = models;
    else delete config.models;
  }
  if (typeof config.model === 'string') delete config.model;
  return writeGrokToml(config);
}

export function removeGrokField(configText: string, fieldName: string): string {
  const config = parseGrokToml(configText);
  delete config[fieldName];
  return writeGrokToml(config);
}

export function normalizeGrokConfigForOfficialMode(configText: string): string {
  const config = parseGrokToml(configText);
  delete config.base_url;
  delete config.model_provider;
  delete config.model_providers;
  delete config.model;
  const models = isTomlObject(config.models) ? { ...config.models } : undefined;
  if (models) {
    delete models.default;
    delete models.default_reasoning_effort;
    if (Object.keys(models).length > 0) config.models = models;
    else delete config.models;
  }
  return writeGrokToml(config);
}

export function ensureGrokCustomProviderConfig(configText: string): string {
  const config = parseGrokToml(configText);
  delete config.model_provider;
  delete config.model_providers;
  return writeGrokToml(config);
}

export function isGrokPrivacyProtectionEnabled(
  configText: string | undefined | null,
): boolean {
  try {
    const config = parseGrokToml(configText);
    const features = isTomlObject(config.features) ? config.features : {};
    const telemetry = isTomlObject(config.telemetry) ? config.telemetry : {};
    const harness = isTomlObject(config.harness) ? config.harness : {};
    return features.telemetry === false
      && features.codebase_indexing === false
      && telemetry.trace_upload === false
      && harness.disable_codebase_upload === true;
  } catch {
    return false;
  }
}

export function setGrokPrivacyProtection(configText: string, enabled: boolean): string {
  const config = parseGrokToml(configText);
  const updateSection = (sectionName: 'features' | 'telemetry' | 'harness', values: Record<string, boolean>) => {
    const section = isTomlObject(config[sectionName]) ? { ...config[sectionName] } : {};
    for (const [key, value] of Object.entries(values)) {
      if (enabled) section[key] = value;
      else if (section[key] === value) delete section[key];
    }
    if (Object.keys(section).length > 0) config[sectionName] = section;
    else delete config[sectionName];
  };

  updateSection('features', { telemetry: false, codebase_indexing: false });
  updateSection('telemetry', { trace_upload: false });
  updateSection('harness', { disable_codebase_upload: true });
  return writeGrokToml(config);
}

export function getGrokIgnoredCommonConfigKeys(
  configText: string | undefined | null,
): string[] {
  try {
    const config = parseGrokToml(configText);
    const ignoredKeys: string[] = [];
    for (const protectedSection of ['model', 'mcp_servers', 'plugins', 'marketplace']) {
      if (config[protectedSection] !== undefined) ignoredKeys.push(`[${protectedSection}]`);
    }
    const models = isTomlObject(config.models) ? config.models : {};
    if (Object.prototype.hasOwnProperty.call(models, 'default')) {
      ignoredKeys.push('[models].default');
    }
    return ignoredKeys;
  } catch {
    return [];
  }
}
