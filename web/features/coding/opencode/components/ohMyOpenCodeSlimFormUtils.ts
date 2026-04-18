import type { OhMyOpenCodeSlimAgent, OhMyOpenCodeSlimAgents } from '@/types/ohMyOpenCodeSlim';

export interface BuildSlimAgentsInput {
  builtInAgentKeys: string[];
  customAgents: string[];
  formValues: Record<string, unknown>;
  initialAgents?: OhMyOpenCodeSlimAgents;
}

export function buildSlimAgentsFromFormValues({
  builtInAgentKeys,
  customAgents,
  formValues,
  initialAgents,
}: BuildSlimAgentsInput): OhMyOpenCodeSlimAgents {
  const allAgentKeys = [...builtInAgentKeys, ...customAgents];
  const agents: OhMyOpenCodeSlimAgents = {};

  allAgentKeys.forEach((agentType) => {
    const modelFieldName = `agent_${agentType}_model`;
    const variantFieldName = `agent_${agentType}_variant`;
    const fallbackFieldName = `agent_${agentType}_fallback_models`;
    const modelValue = formValues[modelFieldName];
    const variantValue = formValues[variantFieldName];
    const fallbackValue = formValues[fallbackFieldName];
    const existingAgent =
      initialAgents?.[agentType] && typeof initialAgents[agentType] === 'object'
        ? (initialAgents[agentType] as OhMyOpenCodeSlimAgent)
        : undefined;

    const {
      model: _existingModel,
      variant: _existingVariant,
      fallback_models: _existingFallbackModels,
      ...existingUnmanagedFields
    } =
      existingAgent || {};

    let normalizedFallbackValue: string[] | undefined;
    if (typeof fallbackValue === 'string') {
      const trimmedValue = fallbackValue.trim();
      normalizedFallbackValue = trimmedValue ? [trimmedValue] : undefined;
    } else if (Array.isArray(fallbackValue)) {
      const items = fallbackValue
        .filter((item): item is string => typeof item === 'string')
        .map((item) => item.trim())
        .filter((item) => item !== '');
      normalizedFallbackValue = items.length > 0 ? items : undefined;
    }

    if (
      modelValue ||
      variantValue ||
      normalizedFallbackValue?.length ||
      Object.keys(existingUnmanagedFields).length > 0
    ) {
      agents[agentType] = {
        ...existingUnmanagedFields,
        ...(modelValue ? { model: modelValue } : {}),
        ...(variantValue ? { variant: variantValue } : {}),
        ...(normalizedFallbackValue ? { fallback_models: normalizedFallbackValue } : {}),
      };
    }
  });

  return agents;
}
