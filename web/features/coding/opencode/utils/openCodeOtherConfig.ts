import type { OpenCodeConfig } from '@/types/opencode';

const managedConfigFields = new Set([
  '$schema',
  'provider',
  'model',
  'small_model',
  'default_agent',
  'agent',
  'plugin',
  'mcp',
]);

const isJsonObject = (value: unknown): value is Record<string, unknown> => (
  typeof value === 'object' && value !== null && !Array.isArray(value)
);

export const extractOpenCodeOtherConfigFields = (
  config: OpenCodeConfig | null | undefined,
): Record<string, unknown> | undefined => {
  if (!config) {
    return undefined;
  }

  const otherFields: Record<string, unknown> = {};
  Object.keys(config).forEach((key) => {
    if (!managedConfigFields.has(key)) {
      otherFields[key] = config[key];
    }
  });

  return Object.keys(otherFields).length > 0 ? otherFields : undefined;
};

export const mergeOpenCodeOtherConfigFields = (
  config: OpenCodeConfig,
  value: unknown,
): OpenCodeConfig => {
  const otherFields = isJsonObject(value) ? value : {};

  return {
    $schema: config.$schema,
    provider: config.provider,
    model: config.model,
    small_model: config.small_model,
    default_agent: config.default_agent,
    agent: config.agent,
    plugin: config.plugin,
    mcp: config.mcp,
    ...otherFields,
  };
};
