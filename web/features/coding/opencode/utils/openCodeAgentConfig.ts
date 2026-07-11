import type {
  OpenCodeAgentConfig,
  OpenCodeConfig,
} from '@/types/opencode';

export const OPEN_CODE_BUILT_IN_PRIMARY_AGENTS = ['build', 'plan'] as const;
export const OPEN_CODE_BUILT_IN_SUBAGENTS = ['general', 'explore', 'scout'] as const;
export const OPEN_CODE_INTERNAL_AGENTS = ['title', 'summary', 'compaction'] as const;

const builtInAgentNames = new Set<string>([
  ...OPEN_CODE_BUILT_IN_PRIMARY_AGENTS,
  ...OPEN_CODE_BUILT_IN_SUBAGENTS,
  ...OPEN_CODE_INTERNAL_AGENTS,
]);

export const isOpenCodeBuiltInAgentName = (agentName: string): boolean => (
  builtInAgentNames.has(agentName)
);

const defaultAgentModes: Record<string, OpenCodeAgentConfig['mode']> = {
  build: 'primary',
  plan: 'primary',
  general: 'subagent',
  explore: 'subagent',
  scout: 'subagent',
  title: 'primary',
  summary: 'primary',
  compaction: 'primary',
};

const hiddenByDefault = new Set<string>(OPEN_CODE_INTERNAL_AGENTS);

export const isOpenCodeJsonObject = (
  value: unknown,
): value is Record<string, unknown> => (
  typeof value === 'object' && value !== null && !Array.isArray(value)
);

export const getOpenCodeAgentConfigs = (
  config: OpenCodeConfig | null | undefined,
): Record<string, OpenCodeAgentConfig> => {
  if (!isOpenCodeJsonObject(config?.agent)) {
    return {};
  }

  return config.agent as Record<string, OpenCodeAgentConfig>;
};

export const getOpenCodeCustomAgentNames = (
  config: OpenCodeConfig | null | undefined,
): string[] => Object.keys(getOpenCodeAgentConfigs(config))
  .filter((agentName) => !builtInAgentNames.has(agentName))
  .sort((left, right) => left.localeCompare(right));

export const getOpenCodeAgentMode = (
  agentName: string,
  agentConfig?: OpenCodeAgentConfig,
): NonNullable<OpenCodeAgentConfig['mode']> => (
  agentConfig?.mode ?? defaultAgentModes[agentName] ?? 'all'
);

export const isOpenCodeAgentHidden = (
  agentName: string,
  agentConfig?: OpenCodeAgentConfig,
): boolean => agentConfig?.hidden ?? hiddenByDefault.has(agentName);

export const getOpenCodeDefaultAgentCandidates = (
  config: OpenCodeConfig | null | undefined,
): string[] => {
  const agents = getOpenCodeAgentConfigs(config);
  const candidateNames = new Set<string>([
    ...OPEN_CODE_BUILT_IN_PRIMARY_AGENTS,
    ...Object.keys(agents),
  ]);

  return Array.from(candidateNames)
    .filter((agentName) => {
      const agentConfig = agents[agentName];
      if (agentConfig?.disable) return false;
      if (isOpenCodeAgentHidden(agentName, agentConfig)) return false;
      return getOpenCodeAgentMode(agentName, agentConfig) !== 'subagent';
    })
    .sort((left, right) => {
      if (left === 'build') return -1;
      if (right === 'build') return 1;
      if (left === 'plan') return -1;
      if (right === 'plan') return 1;
      return left.localeCompare(right);
    });
};

export const clearInvalidOpenCodeDefaultAgent = (
  config: OpenCodeConfig,
): OpenCodeConfig => {
  if (!config.default_agent) return config;

  const agentConfig = getOpenCodeAgentConfigs(config)[config.default_agent];
  const invalid = agentConfig?.disable
    || isOpenCodeAgentHidden(config.default_agent, agentConfig)
    || getOpenCodeAgentMode(config.default_agent, agentConfig) === 'subagent';

  if (!invalid) return config;
  return {
    ...config,
    default_agent: undefined,
  };
};

const updateAgentRecord = (
  config: OpenCodeConfig,
  agentName: string,
  updater: (agentConfig: OpenCodeAgentConfig) => OpenCodeAgentConfig | undefined,
): OpenCodeConfig => {
  const currentAgents = getOpenCodeAgentConfigs(config);
  const nextAgents = { ...currentAgents };
  const nextAgentConfig = updater({ ...(currentAgents[agentName] ?? {}) });

  if (nextAgentConfig === undefined) {
    delete nextAgents[agentName];
  } else {
    nextAgents[agentName] = nextAgentConfig;
  }

  return {
    ...config,
    agent: Object.keys(nextAgents).length > 0 ? nextAgents : undefined,
  };
};

export const setOpenCodeAgentModel = (
  config: OpenCodeConfig,
  agentName: string,
  model: string | undefined,
  availableVariants: string[] = [],
): OpenCodeConfig => updateAgentRecord(config, agentName, (agentConfig) => {
  if (!model) {
    const { model: _model, variant: _variant, ...remainingConfig } = agentConfig;
    if (Object.keys(remainingConfig).length === 0 && builtInAgentNames.has(agentName)) {
      return undefined;
    }
    return remainingConfig;
  }

  const currentVariant = agentConfig.variant;
  const keepVariant = currentVariant
    && availableVariants.includes(currentVariant);

  if (!keepVariant) {
    const { variant: _variant, ...remainingConfig } = agentConfig;
    return {
      ...remainingConfig,
      model,
    };
  }

  return {
    ...agentConfig,
    model,
    variant: currentVariant,
  };
});

export const setOpenCodeAgentVariant = (
  config: OpenCodeConfig,
  agentName: string,
  variant: string | undefined,
): OpenCodeConfig => updateAgentRecord(config, agentName, (agentConfig) => {
  if (!agentConfig.model || !variant) {
    const { variant: _variant, ...remainingConfig } = agentConfig;
    return remainingConfig;
  }

  return {
    ...agentConfig,
    variant,
  };
});

export const setOpenCodeAgentPrompt = (
  config: OpenCodeConfig,
  agentName: string,
  prompt: string | undefined,
): OpenCodeConfig => updateAgentRecord(config, agentName, (agentConfig) => {
  if (!prompt?.trim()) {
    const { prompt: _prompt, ...remainingConfig } = agentConfig;
    if (Object.keys(remainingConfig).length === 0 && builtInAgentNames.has(agentName)) {
      return undefined;
    }
    return remainingConfig;
  }

  return {
    ...agentConfig,
    prompt,
  };
});

export const setOpenCodeAgentAdvancedConfig = (
  config: OpenCodeConfig,
  agentName: string,
  agentConfig: OpenCodeAgentConfig,
): OpenCodeConfig => updateAgentRecord(config, agentName, () => {
  if (Object.keys(agentConfig).length === 0 && builtInAgentNames.has(agentName)) {
    return undefined;
  }
  return { ...agentConfig };
});

export const removeOpenCodeAgentOverride = (
  config: OpenCodeConfig,
  agentName: string,
): OpenCodeConfig => updateAgentRecord(config, agentName, () => undefined);

export const getConfiguredOpenCodeAgentModelIds = (
  config: OpenCodeConfig | null | undefined,
): string[] => Object.values(getOpenCodeAgentConfigs(config))
  .map((agentConfig) => agentConfig.model)
  .filter((model): model is string => typeof model === 'string' && model.length > 0);

export const sanitizeOpenCodeAgentModelReferences = (
  config: OpenCodeConfig,
  removedModelIds: Set<string>,
): OpenCodeConfig => {
  const currentAgents = getOpenCodeAgentConfigs(config);
  let changed = false;
  const nextAgents: Record<string, OpenCodeAgentConfig> = {};

  for (const [agentName, agentConfig] of Object.entries(currentAgents)) {
    if (!agentConfig.model || !removedModelIds.has(agentConfig.model)) {
      nextAgents[agentName] = agentConfig;
      continue;
    }

    const { model: _model, variant: _variant, ...remainingConfig } = agentConfig;
    changed = true;

    if (Object.keys(remainingConfig).length > 0 || !builtInAgentNames.has(agentName)) {
      nextAgents[agentName] = remainingConfig;
    }
  }

  if (!changed) return config;

  return {
    ...config,
    agent: Object.keys(nextAgents).length > 0 ? nextAgents : undefined,
  };
};

export const validateOpenCodeAgentConfig = (
  value: unknown,
  options: { requireDescription?: boolean } = {},
): string | undefined => {
  if (!isOpenCodeJsonObject(value)) {
    return 'object';
  }

  if (value.model !== undefined && typeof value.model !== 'string') {
    return 'model';
  }
  if (value.variant !== undefined && typeof value.variant !== 'string') {
    return 'variant';
  }
  if (value.prompt !== undefined && typeof value.prompt !== 'string') {
    return 'prompt';
  }
  if (
    options.requireDescription
    && (typeof value.description !== 'string' || !value.description.trim())
  ) {
    return 'description';
  }
  if (
    value.mode !== undefined
    && value.mode !== 'primary'
    && value.mode !== 'subagent'
    && value.mode !== 'all'
  ) {
    return 'mode';
  }
  if (
    value.steps !== undefined
    && (!Number.isInteger(value.steps) || Number(value.steps) <= 0)
  ) {
    return 'steps';
  }

  return undefined;
};
