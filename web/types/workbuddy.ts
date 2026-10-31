// WorkBuddy desktop runtime configuration types.
//
// The backend manages `<config root>/models.json`, a flat list of model entries
// with no provider concept. The page shows "channels", so the backend groups
// entries by `(normalized url, apiKey)` and returns them as `providers`.
//
// The channel/model shapes are shared with CodeBuddy Code — same engine, same
// file — and live in `workbuddyFamily.ts`. All view fields are camelCase
// (serde `rename_all`).

import type { WorkbuddyFamilyProviderView } from './workbuddyFamily';

export type { WorkbuddyFamilyModelInput as WorkbuddyModelInput } from './workbuddyFamily';
export type {
  WorkbuddyFamilyModelView as WorkbuddyModelView,
  WorkbuddyFamilyProviderView as WorkbuddyProviderView,
} from './workbuddyFamily';

export interface WorkbuddyPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

export interface WorkbuddySettingsConfig {
  configDir?: string;
  updatedAt?: string;
}

export interface WorkbuddySettingsConfigInput {
  configDir?: string | null;
  clearConfigDir?: boolean;
}

export interface WorkbuddyRuntimeConfig {
  providers: WorkbuddyFamilyProviderView[];
  /**
   * The file's own `availableModels`. Absent or empty both mean "show every
   * model, built-ins included" — never render an empty list as "no models".
   */
  availableModels?: string[];
  exists: boolean;
  path: string;
}
