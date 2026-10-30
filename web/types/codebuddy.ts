// CodeBuddy Code CLI runtime configuration types.
//
// Same `models.json` contract as WorkBuddy desktop (both come from one engine),
// plus the CLI's own `CODEBUDDY.md` prompt presets and `settings.json`.
// The channel/model shapes live in `workbuddyFamily.ts`.
// All view fields are camelCase (serde `rename_all`).

import type { WorkbuddyFamilyProviderView } from './workbuddyFamily';

export type { WorkbuddyFamilyModelInput as CodebuddyModelInput } from './workbuddyFamily';
export type {
  WorkbuddyFamilyModelView as CodebuddyModelView,
  WorkbuddyFamilyProviderView as CodebuddyProviderView,
} from './workbuddyFamily';

export interface CodebuddyPathInfo {
  path: string;
  source: 'custom' | 'env' | 'shell' | 'default';
}

export interface CodebuddySettingsConfig {
  configDir?: string;
  updatedAt?: string;
}

export interface CodebuddySettingsConfigInput {
  configDir?: string | null;
  clearConfigDir?: boolean;
}

export interface CodebuddyRuntimeConfig {
  providers: WorkbuddyFamilyProviderView[];
  /** Absent or empty both mean "show every model, built-ins included". */
  availableModels?: string[];
  exists: boolean;
  path: string;
}

export interface CodebuddyPromptConfigInput {
  id?: string | null;
  name: string;
  content: string;
}

export interface CodebuddyPromptConfig {
  id: string;
  name: string;
  content: string;
  isApplied: boolean;
  sortIndex?: number | null;
  createdAt?: string | null;
  updatedAt?: string | null;
}
