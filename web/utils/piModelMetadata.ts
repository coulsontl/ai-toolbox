import type { OpenCodeModelVariant } from '@/types/opencode';

export const PI_INPUT_TYPES = new Set(['text', 'image']);
export const PI_STANDARD_THINKING_LEVEL_KEYS = ['off', 'minimal', 'low', 'medium', 'high'] as const;
export const PI_EXTENDED_THINKING_LEVEL_KEYS = ['xhigh', 'max'] as const;
export const PI_THINKING_LEVEL_KEYS = [
  ...PI_STANDARD_THINKING_LEVEL_KEYS,
  ...PI_EXTENDED_THINKING_LEVEL_KEYS,
] as const;
export const PI_THINKING_LEVELS = new Set<string>(PI_THINKING_LEVEL_KEYS);
export const PI_THINKING_LEVEL_OPTIONS = PI_STANDARD_THINKING_LEVEL_KEYS.map((value) => ({
  value,
  label: value,
}));
const PI_EXTENDED_THINKING_LEVELS = new Set<string>(PI_EXTENDED_THINKING_LEVEL_KEYS);

const asRecord = (value: unknown): Record<string, unknown> => (
  value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
);

export const normalizePiThinkingLevelKey = (key: string): string | undefined => {
  if (key === 'none') {
    return 'off';
  }
  return PI_THINKING_LEVELS.has(key) ? key : undefined;
};

export const isPiThinkingLevelMapEntrySupported = (
  levelKey: string,
  thinkingLevelMap: Record<string, unknown>,
): boolean => {
  const mappedValue = thinkingLevelMap[levelKey];
  if (mappedValue === null) {
    return false;
  }
  return !PI_EXTENDED_THINKING_LEVELS.has(levelKey) || mappedValue !== undefined;
};

export const getPresetThinkingLevelValue = (
  variant: OpenCodeModelVariant,
): string | null | undefined => {
  if (variant.disabled === true) {
    return null;
  }
  if (typeof variant.reasoningEffort === 'string') {
    return variant.reasoningEffort === 'none' ? 'none' : variant.reasoningEffort;
  }
  // Claude / Anthropic OpenCode presets use top-level `effort` (not reasoningEffort).
  if (typeof variant.effort === 'string') {
    return variant.effort === 'none' ? 'none' : variant.effort;
  }
  const thinkingConfig = asRecord(variant.thinkingConfig);
  if (typeof thinkingConfig.thinkingLevel === 'string') {
    return thinkingConfig.thinkingLevel;
  }
  if (typeof variant.thinkingLevel === 'string') {
    return variant.thinkingLevel;
  }
  return undefined;
};

export const buildPiThinkingLevelMapFromPreset = (
  variants: Record<string, OpenCodeModelVariant> | undefined,
): Record<string, string | null> => {
  if (!variants || Object.keys(variants).length === 0) {
    return {};
  }
  const result: Record<string, string | null> = {};
  Object.entries(variants).forEach(([variantKey, variant]) => {
    const levelKey = normalizePiThinkingLevelKey(variantKey);
    if (!levelKey) {
      return;
    }
    const levelValue = getPresetThinkingLevelValue(variant);
    if (levelValue !== undefined) {
      result[levelKey] = levelValue;
    }
  });
  if (Object.keys(result).length > 0) {
    PI_THINKING_LEVEL_KEYS.forEach((levelKey) => {
      if (!(levelKey in result)) {
        result[levelKey] = null;
      }
    });
  }
  return result;
};
