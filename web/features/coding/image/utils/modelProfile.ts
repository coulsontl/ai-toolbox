import type { ImageProviderKind } from '../services/imageApi';
import {
  DEFAULT_IMAGE_PARAMETER_VISIBILITY,
  getImageProviderProfile,
} from './providerProfile';
import type { ImageParameterVisibility } from './providerProfile';

export type { ImageParameterVisibility } from './providerProfile';

export type ImageModelProfile = 'default' | 'gemini_banana' | 'minimax_h3';

export interface ImageHistoryJobParams {
  size?: string;
  quality?: string;
  output_format?: string;
  output_compression?: number | null;
  moderation?: string;
}

const GEMINI_BANANA_PARAMETER_VISIBILITY: ImageParameterVisibility = {
  size: true,
  quality: true,
  outputFormat: true,
  moderation: false,
  outputCompression: false,
};

const GEMINI_BANANA_MODEL_MARKERS = ['nano-banana'];

const MINIMAX_H3_PARAMETER_VISIBILITY: ImageParameterVisibility = {
  size: true,
  quality: false,
  outputFormat: true,
  moderation: false,
  outputCompression: true,
};

const MINIMAX_H3_MODEL_MARKERS = ['minimax-h3-image'];

const normalizeModelValue = (value?: string | null): string =>
  value?.trim().toLowerCase() ?? '';

const matchesModelMarkers = (value: string | null | undefined, markers: string[]): boolean => {
  const normalizedValue = normalizeModelValue(value);
  if (!normalizedValue) {
    return false;
  }

  return markers.some((marker) => normalizedValue.includes(marker));
};

export const resolveImageModelProfile = (
  modelId: string,
  modelName?: string | null
): ImageModelProfile => {
  if (
    matchesModelMarkers(modelId, GEMINI_BANANA_MODEL_MARKERS) ||
    matchesModelMarkers(modelName, GEMINI_BANANA_MODEL_MARKERS)
  ) {
    return 'gemini_banana';
  }
  if (
    matchesModelMarkers(modelId, MINIMAX_H3_MODEL_MARKERS) ||
    matchesModelMarkers(modelName, MINIMAX_H3_MODEL_MARKERS)
  ) {
    return 'minimax_h3';
  }

  return 'default';
};

export const getImageParameterVisibility = (
  providerKind: ImageProviderKind,
  modelId: string,
  modelName?: string | null
): ImageParameterVisibility => {
  const providerProfile = getImageProviderProfile(providerKind);
  if (!providerProfile.usesModelProfileVisibility) {
    return providerProfile.parameterVisibility ?? DEFAULT_IMAGE_PARAMETER_VISIBILITY;
  }

  const modelProfile = resolveImageModelProfile(modelId, modelName);
  if (modelProfile === 'gemini_banana') {
    return GEMINI_BANANA_PARAMETER_VISIBILITY;
  }
  if (modelProfile === 'minimax_h3') {
    return MINIMAX_H3_PARAMETER_VISIBILITY;
  }

  return providerProfile.parameterVisibility ?? DEFAULT_IMAGE_PARAMETER_VISIBILITY;
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
  providerKind: ImageProviderKind,
  modelId: string,
  modelName?: string | null
): ImageHistoryJobParams => {
  const parameterVisibility = getImageParameterVisibility(providerKind, modelId, modelName);

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
