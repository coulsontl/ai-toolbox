/**
 * FetchModelsModal Types
 */

/** API type for fetching models */
export type ApiType = 'native' | 'openai_compat';

/** Config value syntax the backend resolves for this request.
 * Pi, OMP and OmO Native resolve `apiKey` / header values in their own runtime,
 * so the modal must not treat `apiKey` as a URL credential or as a plain
 * literal. `omo` uses Pi's syntax but OmO's runtime location. */
export type ConfigValueMode = 'pi' | 'omp' | 'omo';

/** Fetched model info from API */
export interface FetchedModel {
  id: string;
  /** Display label (`display_name` / `displayName` / `name` upstream). The
   * backend falls back to `id` when the provider sends no label. */
  name?: string;
  ownedBy?: string;
  created?: number;
}

/** Response from fetch models API */
export interface FetchModelsResponse {
  models: FetchedModel[];
  total: number;
}

/** Result returned when applying fetched models */
export interface FetchModelsApplyResult {
  selectedModels: FetchedModel[];
  removedModelIds: string[];
  /** All fetched model ids in display order (grouped by owner), including
   * unselected ones, so consumers can normalize their list/mapping ordering
   * to match what the modal showed. */
  orderedModelIds: string[];
}

/** Props for FetchModelsModal component */
export interface FetchModelsModalProps {
  open: boolean;
  providerId: string;
  providerName: string;
  baseUrl: string;
  apiKey?: string;
  headers?: Record<string, string>;
  sdkType?: string;
  /** Opt in to the provider tool's own config value syntax (Pi `models.json` / OMP `models.yml`). */
  configValueMode?: ConfigValueMode;
  existingModelIds: string[];
  /** Owner groups (ownedBy values) pinned to the front of the sorted list,
   * in order. Optional; defaults to plain alphabetical owner grouping. */
  priorityOwnedBy?: string[];
  onCancel: () => void;
  onSuccess: (result: FetchModelsApplyResult) => void;
}
