/**
 * Claude Code Configuration Types
 *
 * Type definitions for Claude Code configuration management.
 */

export type ClaudeProviderCategory = 'official' | 'third_party' | 'custom';

/**
 * Claude Code Provider settings configuration
 * Maps to the settings.json env section
 */
export interface ClaudeSettingsConfig {
  env?: {
    ANTHROPIC_API_KEY?: string;
    ANTHROPIC_AUTH_TOKEN?: string;
    ANTHROPIC_BASE_URL?: string;
  };
  // Model configurations
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
}

/**
 * Claude Code Provider stored in database
 */
export interface ClaudeCodeProvider {
  id: string;
  name: string;
  category: ClaudeProviderCategory;
  settingsConfig: string; // JSON string of ClaudeSettingsConfig
  // Source info if imported from settings
  sourceProviderId?: string;
  // Metadata
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  sortIndex?: number;
  isCurrent?: boolean;
  isApplied?: boolean;
  createdAt?: string;
  updatedAt?: string;
}

/**
 * Common configuration that applies to all providers
 * Stored as a single record in database
 */
export interface ClaudeCommonConfig {
  id: string; // Always "common"
  config: string; // JSON string like '{ "statusLine": {...}, "skipWebFetchPreflight": true }'
  updatedAt?: number;
}

/**
 * Claude Code settings.json file structure
 */
export interface ClaudeSettings {
  env?: {
    ANTHROPIC_API_KEY?: string;
    ANTHROPIC_AUTH_TOKEN?: string;
    ANTHROPIC_BASE_URL?: string;
    ANTHROPIC_MODEL?: string;
    ANTHROPIC_DEFAULT_HAIKU_MODEL?: string;
    ANTHROPIC_DEFAULT_SONNET_MODEL?: string;
    ANTHROPIC_DEFAULT_OPUS_MODEL?: string;
  };
  // Common config fields (merged from ClaudeCommonConfig)
  [key: string]: unknown;
}

/**
 * Form values for creating/editing a provider
 */
export interface ClaudeProviderFormValues {
  name: string;
  category: ClaudeProviderCategory;
  baseUrl?: string;
  apiKey?: string;
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
  notes?: string;
  // For import from settings
  sourceProviderId?: string;
}

/**
 * Import conflict action
 */
export type ImportConflictAction = 'overwrite' | 'duplicate' | 'cancel';

/**
 * Import conflict info
 */
export interface ImportConflictInfo {
  existingProvider: ClaudeCodeProvider;
  newProviderName: string;
  sourceProviderId: string;
}
