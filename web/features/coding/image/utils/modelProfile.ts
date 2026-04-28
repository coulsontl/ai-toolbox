export type ImageModelProfile = 'default' | 'gemini_banana';

export interface ImageHistoryJobParams {
  size?: string;
  quality?: string;
  output_format?: string;
  output_compression?: number | null;
  moderation?: string;
}

export interface ImageParameterVisibility {
  size: boolean;
  quality: boolean;
  outputFormat: boolean;
  moderation: boolean;
  outputCompression: boolean;
}

const DEFAULT_IMAGE_PARAMETER_VISIBILITY: ImageParameterVisibility = {
  size: true,
  quality: true,
  outputFormat: true,
  moderation: true,
  outputCompression: true,
};

const GEMINI_BANANA_PARAMETER_VISIBILITY: ImageParameterVisibility = {
  size: true,
  quality: true,
  outputFormat: true,
  moderation: false,
  outputCompression: false,
};

const GEMINI_BANANA_MODEL_MARKERS = ['nano-banana'];

const normalizeModelValue = (value?: string | null): string =>
  value?.trim().toLowerCase() ?? '';

const isGeminiBananaValue = (value?: string | null): boolean => {
  const normalizedValue = normalizeModelValue(value);
  if (!normalizedValue) {
    return false;
  }

  return GEMINI_BANANA_MODEL_MARKERS.some((marker) => normalizedValue.includes(marker));
};

export const resolveImageModelProfile = (
  modelId: string,
  modelName?: string | null
): ImageModelProfile => {
  if (isGeminiBananaValue(modelId) || isGeminiBananaValue(modelName)) {
    return 'gemini_banana';
  }

  return 'default';
};

export const getImageParameterVisibility = (
  modelId: string,
  modelName?: string | null
): ImageParameterVisibility => {
  return resolveImageModelProfile(modelId, modelName) === 'gemini_banana'
    ? GEMINI_BANANA_PARAMETER_VISIBILITY
    : DEFAULT_IMAGE_PARAMETER_VISIBILITY;
};

export const parseHistoryJobParams = (
  rawValue: string
): ImageHistoryJobParams | null => {
  const trimmedValue = rawValue.trim();
  if (!trimmedValue) {
    return null;
  }

  try {
    return JSON.parse(trimmedValue) as ImageHistoryJobParams;
  } catch {
    return null;
  }
};

export const filterHistoryJobParamsByModel = (
  jobParams: ImageHistoryJobParams,
  modelId: string,
  modelName?: string | null
): ImageHistoryJobParams => {
  const parameterVisibility = getImageParameterVisibility(modelId, modelName);

  return {
    ...(parameterVisibility.size && jobParams.size
      ? { size: jobParams.size }
      : {}),
    ...(parameterVisibility.quality && jobParams.quality
      ? { quality: jobParams.quality }
      : {}),
    ...(parameterVisibility.outputFormat && jobParams.output_format
      ? { output_format: jobParams.output_format }
      : {}),
    ...(parameterVisibility.moderation && jobParams.moderation
      ? { moderation: jobParams.moderation }
      : {}),
    ...(parameterVisibility.outputCompression &&
    typeof jobParams.output_compression !== 'undefined'
      ? { output_compression: jobParams.output_compression }
      : {}),
  };
};
